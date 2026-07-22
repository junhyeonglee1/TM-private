use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(windows)]
use std::process::{Command, Stdio};

use reqwest::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tm_core::TmHome;

const CONFIG_FILE_NAME: &str = "cloud-client.json";
const CREDENTIAL_RESOURCE: &str = "TM Cloud Production";
const CREDENTIAL_USER: &str = "single-user";
const TOKEN_PREFIX: &str = "tm_pat_v1_";
const TOKEN_SECRET_LENGTH: usize = 43;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

type CloudResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DataMode {
    #[default]
    Local,
    Cloud,
}

impl DataMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Cloud => "cloud",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredConfig {
    #[serde(default)]
    mode: DataMode,
    base_url: Option<String>,
}

#[derive(Clone)]
pub(crate) struct CloudClient {
    http: reqwest::Client,
    base_url: Option<Url>,
    exports_dir: PathBuf,
    mode: DataMode,
}

impl std::fmt::Debug for CloudClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudClient")
            .field("configured", &self.base_url.is_some())
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl CloudClient {
    pub(crate) fn load(home: &TmHome) -> CloudResult<Self> {
        let config_path = config_path(home);
        let stored = if config_path.exists() {
            let bytes = fs::read(&config_path)
                .map_err(|_| "TM cloud configuration could not be read".to_owned())?;
            serde_json::from_slice::<StoredConfig>(&bytes)
                .map_err(|_| "TM cloud configuration is invalid".to_owned())?
        } else {
            StoredConfig {
                mode: DataMode::Local,
                base_url: None,
            }
        };
        let base_url = stored
            .base_url
            .as_deref()
            .map(validate_base_url)
            .transpose()?;
        if stored.mode == DataMode::Cloud && base_url.is_none() {
            return Err("TM cloud mode requires an HTTPS base URL".to_owned());
        }
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .user_agent(concat!("tm-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "TM cloud HTTPS client could not be created".to_owned())?;
        Ok(Self {
            http,
            base_url,
            exports_dir: home.exports_dir(),
            mode: stored.mode,
        })
    }

    pub(crate) const fn mode(&self) -> DataMode {
        self.mode
    }

    pub(crate) async fn invoke(&self, command: &str, args: Value) -> CloudResult<Value> {
        if self.mode != DataMode::Cloud {
            return Err("TM cloud command was requested while local mode is active".to_owned());
        }
        if !valid_command_name(command) {
            return Err("TM desktop command name is invalid".to_owned());
        }
        let token = load_token_from_os_store()?;
        if !valid_token(&token) {
            return Err("TM cloud credential has an invalid format".to_owned());
        }
        let mut endpoint = self
            .base_url
            .clone()
            .ok_or_else(|| "TM cloud HTTPS base URL is not configured".to_owned())?;
        endpoint.set_path(&format!("/api/v1/desktop/commands/{command}"));

        let response = self
            .http
            .post(endpoint)
            .bearer_auth(&token)
            .header("x-tm-confirm-desktop-command", command)
            .json(&serde_json::json!({ "args": args }))
            .send()
            .await
            .map_err(|_| "TM cloud server could not be reached over HTTPS".to_owned())?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err("TM cloud response exceeded the safety limit".to_owned());
        }
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let bytes = response
            .bytes()
            .await
            .map_err(|_| "TM cloud response could not be read".to_owned())?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err("TM cloud response exceeded the safety limit".to_owned());
        }
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "TM cloud response was not valid JSON".to_owned())?;
        if !status.is_success() {
            return Err(response_error(status, &payload, request_id.as_deref()));
        }
        let data = payload
            .get("data")
            .cloned()
            .ok_or_else(|| "TM cloud response did not contain data".to_owned())?;
        if command == "export_all" {
            return materialize_export(&self.exports_dir, data);
        }
        Ok(data)
    }

    pub(crate) async fn device_admin(&self, command: &str, args: Value) -> CloudResult<Value> {
        if self.mode != DataMode::Cloud {
            return Err("TM device administration requires cloud mode".to_owned());
        }
        let (method, path, confirmation, body) = match command {
            "operations_status" => (Method::GET, "/api/v1/ops/status".to_owned(), None, None),
            "list_pairings" => (
                Method::GET,
                "/api/v1/admin/device-pairings".to_owned(),
                None,
                None,
            ),
            "list_devices" => (Method::GET, "/api/v1/admin/devices".to_owned(), None, None),
            "approve_pairing" => {
                let pairing_id = required_resource_id(&args, "pairingId")?;
                let code = args
                    .get("code")
                    .and_then(Value::as_str)
                    .filter(|value| {
                        value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    .ok_or_else(|| "TM device pairing code must be six digits".to_owned())?;
                (
                    Method::POST,
                    format!("/api/v1/admin/device-pairings/{pairing_id}/approve"),
                    Some("approve"),
                    Some(serde_json::json!({ "code": code })),
                )
            }
            "revoke_device" => {
                let device_id = required_resource_id(&args, "deviceId")?;
                (
                    Method::POST,
                    format!("/api/v1/admin/devices/{device_id}/revoke"),
                    Some("revoke"),
                    Some(serde_json::json!({})),
                )
            }
            "revoke_all_devices" => (
                Method::POST,
                "/api/v1/admin/devices/revoke-all".to_owned(),
                Some("revoke-all"),
                Some(serde_json::json!({})),
            ),
            _ => return Err("TM device administration command is not allowed".to_owned()),
        };

        let token = load_token_from_os_store()?;
        if !valid_token(&token) {
            return Err("TM cloud credential has an invalid format".to_owned());
        }
        let mut endpoint = self
            .base_url
            .clone()
            .ok_or_else(|| "TM cloud HTTPS base URL is not configured".to_owned())?;
        endpoint.set_path(&path);
        let mut request = self.http.request(method, endpoint).bearer_auth(&token);
        if let Some(value) = confirmation {
            request = request.header("x-tm-confirm-device-admin", value);
        }
        if let Some(value) = body {
            request = request.json(&value);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "TM cloud server could not be reached over HTTPS".to_owned())?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err("TM cloud response exceeded the safety limit".to_owned());
        }
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let bytes = response
            .bytes()
            .await
            .map_err(|_| "TM cloud response could not be read".to_owned())?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err("TM cloud response exceeded the safety limit".to_owned());
        }
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "TM cloud response was not valid JSON".to_owned())?;
        if !status.is_success() {
            return Err(response_error(status, &payload, request_id.as_deref()));
        }
        payload
            .get("data")
            .cloned()
            .ok_or_else(|| "TM cloud response did not contain data".to_owned())
    }
}

fn required_resource_id<'a>(args: &'a Value, field: &str) -> CloudResult<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        .ok_or_else(|| format!("TM device administration field is invalid: {field}"))
}

pub(crate) fn config_path(home: &TmHome) -> PathBuf {
    home.data_dir().join(CONFIG_FILE_NAME)
}

fn validate_base_url(value: &str) -> CloudResult<Url> {
    let mut url = Url::parse(value).map_err(|_| "TM cloud base URL is invalid".to_owned())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("TM cloud base URL must be a credential-free HTTPS origin".to_owned());
    }
    url.set_path("");
    Ok(url)
}

fn valid_command_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn valid_token(value: &str) -> bool {
    value.strip_prefix(TOKEN_PREFIX).is_some_and(|secret| {
        secret.len() == TOKEN_SECRET_LENGTH
            && secret
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn materialize_export(exports_dir: &Path, data: Value) -> CloudResult<Value> {
    let json_name = export_file_name(&data, "jsonFileName", "json")?;
    let markdown_name = export_file_name(&data, "markdownFileName", "md")?;
    let json_content = export_content(&data, "jsonContent")?;
    let markdown_content = export_content(&data, "markdownContent")?;
    let exported_at = data
        .get("exportedAt")
        .and_then(Value::as_str)
        .ok_or_else(|| "TM cloud export timestamp was missing".to_owned())?;

    fs::create_dir_all(exports_dir)
        .map_err(|_| "TM local export directory could not be created".to_owned())?;
    let json_path = exports_dir.join(json_name);
    let markdown_path = exports_dir.join(markdown_name);
    write_new_export_file(&json_path, json_content.as_bytes())?;
    if let Err(error) = write_new_export_file(&markdown_path, markdown_content.as_bytes()) {
        let _ = fs::remove_file(&json_path);
        return Err(error);
    }

    Ok(serde_json::json!({
        "jsonPath": json_path,
        "markdownPath": markdown_path,
        "exportedAt": exported_at,
    }))
}

fn export_file_name(data: &Value, field: &str, extension: &str) -> CloudResult<String> {
    let value = data
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && !value.starts_with('.')
                && value.ends_with(&format!(".{extension}"))
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .ok_or_else(|| "TM cloud export file name was rejected".to_owned())?;
    Ok(value.to_owned())
}

fn export_content<'a>(data: &'a Value, field: &str) -> CloudResult<&'a str> {
    data.get(field)
        .and_then(Value::as_str)
        .filter(|value| value.len() <= MAX_RESPONSE_BYTES)
        .ok_or_else(|| "TM cloud export content was missing or too large".to_owned())
}

fn write_new_export_file(path: &Path, bytes: &[u8]) -> CloudResult<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "TM local export file already exists or could not be created".to_owned())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "TM local export file could not be saved".to_owned())
}

fn response_error(status: StatusCode, payload: &Value, request_id: Option<&str>) -> String {
    let request_suffix = request_id.map_or_else(String::new, |value| format!(" ({value})"));
    match status {
        StatusCode::UNAUTHORIZED => {
            format!("TM cloud authentication failed; run secure setup again{request_suffix}")
        }
        StatusCode::TOO_MANY_REQUESTS => {
            format!("TM cloud request limit was reached; try again shortly{request_suffix}")
        }
        StatusCode::PRECONDITION_REQUIRED => {
            format!("TM cloud command confirmation was rejected{request_suffix}")
        }
        StatusCode::CONFLICT => {
            format!("TM cloud data changed before the command completed{request_suffix}")
        }
        _ => {
            let code = payload
                .pointer("/error/code")
                .and_then(Value::as_str)
                .filter(|value| {
                    !value.is_empty()
                        && value.len() <= 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
                })
                .unwrap_or("CLOUD_REQUEST_FAILED");
            format!("TM cloud request failed: {code}{request_suffix}")
        }
    }
}

#[cfg(windows)]
fn load_token_from_os_store() -> CloudResult<String> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
$credential = $vault.Retrieve($env:TM_CREDENTIAL_RESOURCE, $env:TM_CREDENTIAL_USER)
$credential.RetrievePassword()
[Console]::Out.Write($credential.Password)
"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCRIPT,
        ])
        .env("TM_CREDENTIAL_RESOURCE", CREDENTIAL_RESOURCE)
        .env("TM_CREDENTIAL_USER", CREDENTIAL_USER)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Windows Credential Locker could not be opened".to_owned())?;
    if !output.status.success() {
        return Err("TM cloud credential was not found in Windows Credential Locker".to_owned());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| "TM cloud credential could not be decoded".to_owned())
}

#[cfg(not(windows))]
fn load_token_from_os_store() -> CloudResult<String> {
    let _ = (CREDENTIAL_RESOURCE, CREDENTIAL_USER);
    Err("TM cloud credential storage is currently supported on Windows only".to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::{materialize_export, valid_command_name, valid_token, validate_base_url};

    #[test]
    fn cloud_url_requires_a_clean_https_origin() {
        assert!(validate_base_url("https://tm.example.test").is_ok());
        assert!(validate_base_url("http://tm.example.test").is_err());
        assert!(validate_base_url("https://user:pass@tm.example.test").is_err());
        assert!(validate_base_url("https://tm.example.test?token=secret").is_err());
    }

    #[test]
    fn token_and_command_validation_reject_untrusted_shapes() {
        assert!(valid_command_name("get_app_snapshot"));
        assert!(!valid_command_name("../../snapshot"));
        assert!(valid_token(&format!("tm_pat_v1_{}", "A".repeat(43))));
        assert!(!valid_token("secret"));
    }

    #[test]
    fn cloud_export_is_materialized_without_overwriting_local_files() {
        let temporary = tempdir().expect("create temporary export directory");
        let payload = json!({
            "jsonFileName": "tm-export-test.json",
            "jsonContent": "{\"ok\":true}",
            "markdownFileName": "tm-export-test.md",
            "markdownContent": "# TM export",
            "exportedAt": "2026-07-21T00:00:00Z"
        });

        let result = materialize_export(temporary.path(), payload.clone())
            .expect("materialize cloud export");
        let json_path = result["jsonPath"].as_str().expect("JSON export path");
        let markdown_path = result["markdownPath"]
            .as_str()
            .expect("Markdown export path");
        assert_eq!(
            std::fs::read_to_string(json_path).expect("read JSON export"),
            "{\"ok\":true}"
        );
        assert_eq!(
            std::fs::read_to_string(markdown_path).expect("read Markdown export"),
            "# TM export"
        );
        assert!(materialize_export(temporary.path(), payload).is_err());
    }
}
