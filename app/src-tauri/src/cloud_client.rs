use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(windows)]
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
};

use reqwest::{Method, StatusCode, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tm_core::TmHome;

const CONFIG_FILE_NAME: &str = "cloud-client.json";
const CREDENTIAL_RESOURCE: &str = "TM Cloud Production";
const CREDENTIAL_USER: &str = "single-user";
const TOKEN_PREFIX: &str = "tm_pat_v1_";
const TOKEN_SECRET_LENGTH: usize = 43;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const EXPENSE_CLASSIFICATION_PATH: [&str; 2] = ["expenses", "classifications:run"];
const EXPENSE_CLASSIFICATION_CONFIRMATION: &str = "expense-classification-run";
const EXPENSE_CLASSIFICATION_AI_CONFIRMATION: &str = "expense-classification";
const EXPENSE_CLASSIFICATION_TERMINAL_CODES: [&str; 15] = [
    "EXPENSE_CLASSIFICATION_BUDGET_EXHAUSTED",
    "EXPENSE_CLASSIFICATION_BUDGET_PERSISTENCE_FAILED",
    "EXPENSE_CLASSIFICATION_COST_INVALID",
    "EXPENSE_CLASSIFICATION_IDEMPOTENCY_CONFLICT",
    "EXPENSE_CLASSIFICATION_IDEMPOTENCY_TERMINAL",
    "EXPENSE_CLASSIFICATION_LEASE_EXPIRED",
    "EXPENSE_CLASSIFICATION_OPENAI_AUTHENTICATION_FAILED",
    "EXPENSE_CLASSIFICATION_OPENAI_RATE_LIMITED",
    "EXPENSE_CLASSIFICATION_OPENAI_REQUEST_REJECTED",
    "EXPENSE_CLASSIFICATION_OPENAI_RESPONSE_INVALID",
    "EXPENSE_CLASSIFICATION_OPENAI_UNAVAILABLE",
    "EXPENSE_CLASSIFICATION_PREVIOUS_RUN_RECOVERED",
    "EXPENSE_CLASSIFICATION_PREVIOUSLY_FAILED",
    "EXPENSE_CLASSIFICATION_STAGE_FAILED",
    "EXPENSE_CLASSIFICATION_TIMEOUT",
];
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
        request = request.timeout(Duration::from_secs(55));
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

    pub(crate) async fn assistant_feature(&self, command: &str, args: Value) -> CloudResult<Value> {
        if self.mode != DataMode::Cloud {
            return Err("TM AI assistant features require cloud mode".to_owned());
        }
        let (method, path, confirmation, body) = match command {
            "generate_task_report" => (
                Method::POST,
                "/api/v1/assistant/task-report".to_owned(),
                Some("task-report"),
                None,
            ),
            "latest_task_report" => (
                Method::GET,
                "/api/v1/assistant/task-reports/latest".to_owned(),
                None,
                None,
            ),
            "rate_task_report" => {
                let report_id = required_resource_id(&args, "reportId")?;
                let helpful = args
                    .get("helpful")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "TM Task report feedback must be a boolean".to_owned())?;
                (
                    Method::POST,
                    format!("/api/v1/assistant/task-reports/{report_id}/feedback"),
                    None,
                    Some(serde_json::json!({ "helpful": helpful })),
                )
            }
            _ => return Err("TM AI assistant feature command is not allowed".to_owned()),
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
        request = request.timeout(Duration::from_secs(55));
        if let Some(value) = confirmation {
            request = request.header("x-tm-confirm-ai-call", value);
        }
        if let Some(value) = body {
            request = request.json(&value);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "TM cloud server could not be reached over HTTPS".to_owned())?;
        let status = response.status();
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

    pub(crate) async fn expense_feature(&self, command: &str, args: Value) -> CloudResult<Value> {
        if self.mode != DataMode::Cloud {
            return Err("TM expense features require cloud mode".to_owned());
        }

        let idempotency_key = optional_expense_idempotency_key(&args)?;
        let mut query = Vec::<(&'static str, String)>::new();
        let mut path = Vec::<String>::new();
        let mut confirmation = None;
        let mut expected_version = None;
        let mut create_precondition = false;
        let mut body = None;
        let method = match command {
            "get_expense_summary" => {
                path.extend(["expenses".to_owned(), "summary".to_owned()]);
                query.push(("month", required_month(&args, "month")?.to_owned()));
                Method::GET
            }
            "list_expense_sources" => {
                path.extend(["expenses".to_owned(), "sources".to_owned()]);
                Method::GET
            }
            "update_expense_source" => {
                let source_id = required_expense_id(&args, "sourceId")?;
                let input = required_object(&args, "input")?;
                path.extend([
                    "expenses".to_owned(),
                    "sources".to_owned(),
                    source_id.to_owned(),
                ]);
                confirmation = Some("expense-source-update");
                expected_version = Some(required_version(input, "expectedVersion")?);
                body = Some(copy_object_fields(
                    input,
                    &["requiredForCompleteReport", "isActive"],
                )?);
                Method::PATCH
            }
            "list_expense_transactions" => {
                let input = required_object(&args, "input")?;
                path.extend(["expenses".to_owned(), "transactions".to_owned()]);
                query.push(("month", required_month(input, "month")?.to_owned()));
                append_optional_page_query(input, &mut query)?;
                Method::GET
            }
            "override_expense_transaction" => {
                let event_id = required_expense_id(&args, "eventId")?;
                let input = required_object(&args, "input")?;
                path.extend([
                    "expenses".to_owned(),
                    "transactions".to_owned(),
                    event_id.to_owned(),
                ]);
                confirmation = Some("expense-transaction-override");
                expected_version = Some(required_version(input, "expectedVersion")?);
                body = Some(copy_object_fields(
                    input,
                    &[
                        "kind",
                        "category",
                        "duplicateOfEventId",
                        "relatedEventId",
                        "personalAmountMinor",
                        "clearPersonalAmount",
                        "clearRelatedEvent",
                        "createRule",
                    ],
                )?);
                Method::PATCH
            }
            "list_expense_reviews" => {
                let input = required_object(&args, "input")?;
                path.extend(["expenses".to_owned(), "reviews".to_owned()]);
                if let Some(month) = optional_month(input, "month")? {
                    query.push(("month", month.to_owned()));
                }
                if let Some(status) = optional_enum(input, "status", &["pending", "resolved"])? {
                    query.push(("status", status.to_owned()));
                }
                if let Some(scope) = optional_enum(
                    input,
                    "scope",
                    &["all", "required", "category_confirmation"],
                )? {
                    query.push(("scope", scope.to_owned()));
                }
                append_optional_page_query(input, &mut query)?;
                Method::GET
            }
            "resolve_expense_review" => {
                let review_id = required_expense_id(&args, "reviewId")?;
                let input = required_object(&args, "input")?;
                path.extend([
                    "expenses".to_owned(),
                    "reviews".to_owned(),
                    review_id.to_owned(),
                    "resolve".to_owned(),
                ]);
                confirmation = Some("expense-review-resolve");
                expected_version = Some(required_version(input, "expectedVersion")?);
                body = Some(copy_object_fields(
                    input,
                    &[
                        "kind",
                        "category",
                        "duplicateOfEventId",
                        "relatedEventId",
                        "personalAmountMinor",
                        "createRule",
                    ],
                )?);
                Method::POST
            }
            "list_recurring_expenses" => {
                path.extend(["expenses".to_owned(), "recurring".to_owned()]);
                Method::GET
            }
            "list_recurring_expense_occurrences" => {
                path.extend([
                    "expenses".to_owned(),
                    "recurring".to_owned(),
                    "occurrences".to_owned(),
                ]);
                query.push(("month", required_month(&args, "month")?.to_owned()));
                Method::GET
            }
            "create_recurring_expense" => {
                let input = required_object(&args, "input")?;
                path.extend(["expenses".to_owned(), "recurring".to_owned()]);
                confirmation = Some("recurring-expense-create");
                create_precondition = true;
                body = Some(copy_recurring_fields(input, false)?);
                Method::POST
            }
            "update_recurring_expense" => {
                let item_id = required_expense_id(&args, "recurringExpenseId")?;
                let input = required_object(&args, "input")?;
                path.extend([
                    "expenses".to_owned(),
                    "recurring".to_owned(),
                    item_id.to_owned(),
                ]);
                confirmation = Some("recurring-expense-update");
                expected_version = Some(required_version(input, "expectedVersion")?);
                body = Some(copy_recurring_fields(input, true)?);
                Method::PATCH
            }
            "delete_recurring_expense" => {
                let item_id = required_expense_id(&args, "recurringExpenseId")?;
                path.extend([
                    "expenses".to_owned(),
                    "recurring".to_owned(),
                    item_id.to_owned(),
                ]);
                confirmation = Some("recurring-expense-delete");
                expected_version = Some(required_version(&args, "expectedVersion")?);
                Method::DELETE
            }
            "confirm_recurring_expense_paid" => {
                let occurrence_key = required_expense_id(&args, "occurrenceKey")?;
                path.extend([
                    "expenses".to_owned(),
                    "recurring".to_owned(),
                    "occurrences".to_owned(),
                    occurrence_key.to_owned(),
                    "confirm-paid".to_owned(),
                ]);
                confirmation = Some("recurring-expense-confirm-paid");
                expected_version = Some(required_version(&args, "expectedVersion")?);
                body = Some(copy_object_fields(&args, &["amountMinor", "paidDate"])?);
                Method::POST
            }
            "match_recurring_expense_occurrence" => {
                let occurrence_key = required_expense_id(&args, "occurrenceKey")?;
                path.extend([
                    "expenses".to_owned(),
                    "recurring".to_owned(),
                    "occurrences".to_owned(),
                    occurrence_key.to_owned(),
                    "match".to_owned(),
                ]);
                confirmation = Some("recurring-expense-match");
                expected_version = Some(required_version(&args, "expectedVersion")?);
                body = Some(copy_object_fields(
                    &args,
                    &["eventId", "enableFutureAutoMatch"],
                )?);
                Method::POST
            }
            "classify_expense_transactions" => {
                path.extend(EXPENSE_CLASSIFICATION_PATH.map(str::to_owned));
                confirmation = Some(EXPENSE_CLASSIFICATION_CONFIRMATION);
                create_precondition = true;
                body = Some(serde_json::json!({
                    "month": required_month(&args, "month")?,
                }));
                Method::POST
            }
            "generate_expense_report" => {
                path.extend(["expenses".to_owned(), "reports".to_owned()]);
                confirmation = Some("expense-report-generate");
                create_precondition = true;
                body = Some(serde_json::json!({
                    "month": required_month(&args, "month")?,
                }));
                Method::POST
            }
            "latest_expense_report" => {
                path.extend([
                    "expenses".to_owned(),
                    "reports".to_owned(),
                    "latest".to_owned(),
                ]);
                query.push(("month", required_month(&args, "month")?.to_owned()));
                Method::GET
            }
            "rate_expense_report" => {
                let report_id = required_expense_id(&args, "reportId")?;
                let helpful = args
                    .get("helpful")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "TM expense report feedback must be a boolean".to_owned())?;
                path.extend([
                    "expenses".to_owned(),
                    "reports".to_owned(),
                    report_id.to_owned(),
                    "feedback".to_owned(),
                ]);
                confirmation = Some("expense-report-feedback");
                create_precondition = true;
                body = Some(serde_json::json!({ "helpful": helpful }));
                Method::POST
            }
            _ => return Err("TM expense feature command is not allowed".to_owned()),
        };

        self.send_expense_request(
            method,
            &path,
            &query,
            confirmation,
            expense_ai_confirmation(command),
            idempotency_key,
            create_precondition,
            expected_version,
            body,
        )
        .await
    }

    pub(crate) async fn import_expenses(
        &self,
        body: Value,
        preview_session_id: &str,
    ) -> CloudResult<Value> {
        if self.mode != DataMode::Cloud {
            return Err("TM expense imports require cloud mode".to_owned());
        }
        let encoded = serde_json::to_vec(&body)
            .map_err(|_| "TM normalized expense import could not be encoded".to_owned())?;
        if encoded.len() > 8 * 1024 * 1024 {
            return Err("TM normalized expense import exceeded the 8 MiB safety limit".to_owned());
        }
        self.send_expense_request(
            Method::POST,
            &["expenses".to_owned(), "imports".to_owned()],
            &[],
            Some("expense-import"),
            None,
            Some(format!("desktop-expense-import:{preview_session_id}")),
            true,
            None,
            Some(body),
        )
        .await
    }

    pub(crate) async fn preview_expenses(&self, body: Value) -> CloudResult<Value> {
        if self.mode != DataMode::Cloud {
            return Err("TM expense import preview requires cloud mode".to_owned());
        }
        let encoded = serde_json::to_vec(&body)
            .map_err(|_| "TM expense import preview could not be encoded".to_owned())?;
        if encoded.len() > 8 * 1024 * 1024 {
            return Err("TM expense import preview exceeded the 8 MiB safety limit".to_owned());
        }
        self.send_expense_request(
            Method::POST,
            &[
                "expenses".to_owned(),
                "imports".to_owned(),
                "preview".to_owned(),
            ],
            &[],
            Some("expense-import-preview"),
            None,
            None,
            true,
            None,
            Some(body),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_expense_request(
        &self,
        method: Method,
        path_segments: &[String],
        query: &[(&str, String)],
        confirmation: Option<&str>,
        ai_confirmation: Option<&str>,
        idempotency_key: Option<String>,
        create_precondition: bool,
        expected_version: Option<u64>,
        body: Option<Value>,
    ) -> CloudResult<Value> {
        let token = load_token_from_os_store()?;
        if !valid_token(&token) {
            return Err("TM cloud credential has an invalid format".to_owned());
        }
        let mut endpoint = self
            .base_url
            .clone()
            .ok_or_else(|| "TM cloud HTTPS base URL is not configured".to_owned())?;
        {
            let mut segments = endpoint
                .path_segments_mut()
                .map_err(|()| "TM cloud expense endpoint could not be constructed".to_owned())?;
            segments.clear();
            segments.extend(["api", "v1"]);
            for segment in path_segments {
                segments.push(segment);
            }
        }
        if !query.is_empty() {
            let mut pairs = endpoint.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }

        let mut request = self
            .http
            .request(method, endpoint)
            .bearer_auth(&token)
            .timeout(Duration::from_secs(55));
        if let Some(value) = confirmation {
            request = request.header("x-tm-confirm-mutation", value).header(
                "idempotency-key",
                idempotency_key.unwrap_or_else(|| format!("desktop:{}", uuid::Uuid::now_v7())),
            );
        }
        if let Some(value) = ai_confirmation {
            request = request.header("x-tm-confirm-ai-call", value);
        }
        if create_precondition {
            request = request.header(header::IF_NONE_MATCH, "*");
        }
        if let Some(version) = expected_version {
            request = request.header(header::IF_MATCH, format!("\"{version}\""));
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

    pub(crate) async fn cost_status(&self) -> CloudResult<Value> {
        if self.mode != DataMode::Cloud {
            return Err("TM cost status requires cloud mode".to_owned());
        }
        let token = load_token_from_os_store()?;
        if !valid_token(&token) {
            return Err("TM cloud credential has an invalid format".to_owned());
        }
        let mut endpoint = self
            .base_url
            .clone()
            .ok_or_else(|| "TM cloud HTTPS base URL is not configured".to_owned())?;
        endpoint.set_path("/api/v1/costs/status");
        let response = self
            .http
            .get(endpoint)
            .bearer_auth(&token)
            .timeout(Duration::from_secs(15))
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

fn required_object<'a>(args: &'a Value, field: &str) -> CloudResult<&'a Value> {
    args.get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| format!("TM expense field must be an object: {field}"))
}

fn optional_expense_idempotency_key(args: &Value) -> CloudResult<Option<String>> {
    let Some(value) = args.get("idempotencyKey") else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .and_then(|value| value.strip_prefix("desktop-expense:"))
        .and_then(|value| {
            let parsed = uuid::Uuid::parse_str(value).ok()?;
            (parsed.hyphenated().to_string() == value).then_some(value)
        })
        .ok_or_else(|| "TM expense idempotency key is invalid".to_owned())?;
    Ok(Some(format!("desktop-expense:{value}")))
}

fn required_month<'a>(args: &'a Value, field: &str) -> CloudResult<&'a str> {
    let value = args
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| valid_month(value))
        .ok_or_else(|| format!("TM expense month is invalid: {field}"))?;
    Ok(value)
}

fn optional_month<'a>(args: &'a Value, field: &str) -> CloudResult<Option<&'a str>> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if valid_month(value) => Ok(Some(value)),
        _ => Err(format!("TM expense month is invalid: {field}")),
    }
}

fn valid_month(value: &str) -> bool {
    if value.len() != 7 || value.as_bytes().get(4) != Some(&b'-') {
        return false;
    }
    let Some(year) = value[..4].parse::<u16>().ok() else {
        return false;
    };
    let Some(month) = value[5..].parse::<u8>().ok() else {
        return false;
    };
    (2000..=2200).contains(&year) && (1..=12).contains(&month)
}

fn optional_enum<'a>(
    args: &'a Value,
    field: &str,
    allowed: &[&str],
) -> CloudResult<Option<&'a str>> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if allowed.contains(&value.as_str()) => Ok(Some(value)),
        _ => Err(format!("TM expense field is invalid: {field}")),
    }
}

fn append_optional_page_query(
    input: &Value,
    query: &mut Vec<(&'static str, String)>,
) -> CloudResult<()> {
    if let Some(cursor) = input.get("cursor").filter(|value| !value.is_null()) {
        let cursor = cursor
            .as_str()
            .filter(|value| {
                !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
            })
            .ok_or_else(|| "TM expense cursor is invalid".to_owned())?;
        query.push(("cursor", cursor.to_owned()));
    }
    if let Some(limit) = input.get("limit").filter(|value| !value.is_null()) {
        let limit = limit
            .as_u64()
            .filter(|value| (1..=100).contains(value))
            .ok_or_else(|| "TM expense page limit must be between 1 and 100".to_owned())?;
        query.push(("limit", limit.to_string()));
    }
    Ok(())
}

fn required_expense_id<'a>(args: &'a Value, field: &str) -> CloudResult<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 256
                && *value != "."
                && *value != ".."
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
                })
        })
        .ok_or_else(|| format!("TM expense resource identifier is invalid: {field}"))
}

fn required_version(args: &Value, field: &str) -> CloudResult<u64> {
    args.get(field)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("TM expense resource version is invalid: {field}"))
}

fn copy_object_fields(input: &Value, allowed: &[&str]) -> CloudResult<Value> {
    let source = input
        .as_object()
        .ok_or_else(|| "TM expense mutation input must be an object".to_owned())?;
    let mut target = serde_json::Map::new();
    for field in allowed {
        if let Some(value) = source.get(*field) {
            target.insert((*field).to_owned(), value.clone());
        }
    }
    Ok(Value::Object(target))
}

fn copy_recurring_fields(input: &Value, update: bool) -> CloudResult<Value> {
    const CREATE_FIELDS: &[&str] = &[
        "name",
        "category",
        "vendor",
        "amountMinor",
        "currency",
        "paymentMethodFingerprint",
        "startDate",
        "endDate",
        "memo",
        "reminderDays",
        "amountKind",
        "intervalMonths",
        "dueRule",
        "dueDay",
        "status",
    ];
    const UPDATE_FIELDS: &[&str] = &[
        "name",
        "category",
        "vendor",
        "amountMinor",
        "currency",
        "paymentMethodFingerprint",
        "startDate",
        "endDate",
        "memo",
        "reminderDays",
        "amountKind",
        "intervalMonths",
        "dueRule",
        "dueDay",
        "status",
        "effectiveFromMonth",
        "autoMatchEnabled",
    ];
    copy_object_fields(input, if update { UPDATE_FIELDS } else { CREATE_FIELDS })
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

fn expense_ai_confirmation(command: &str) -> Option<&'static str> {
    match command {
        "generate_expense_report" => Some("expense-report"),
        "classify_expense_transactions" => Some(EXPENSE_CLASSIFICATION_AI_CONFIRMATION),
        _ => None,
    }
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
    if let Some(code) = expense_classification_terminal_code(payload) {
        return format!(
            "TM expense classification run ended; press AI automatic classification again to start a new run, which may use one monthly attempt and cost up to $0.01 [TM_ERROR_CODE:{code}]{request_suffix}"
        );
    }
    match status {
        StatusCode::UNAUTHORIZED => {
            format!("TM cloud authentication failed; run secure setup again{request_suffix}")
        }
        StatusCode::TOO_MANY_REQUESTS => {
            if payload.pointer("/error/code").and_then(Value::as_str)
                == Some("TASK_REPORT_DAILY_LIMIT_REACHED")
            {
                format!(
                    "오늘의 Task AI 리포트는 하루 최대 네 번까지 만들 수 있습니다{request_suffix}"
                )
            } else {
                format!("TM cloud request limit was reached; try again shortly{request_suffix}")
            }
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

fn expense_classification_terminal_code(payload: &Value) -> Option<&str> {
    payload
        .pointer("/error/code")
        .and_then(Value::as_str)
        .filter(|code| EXPENSE_CLASSIFICATION_TERMINAL_CODES.contains(code))
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
        .creation_flags(CREATE_NO_WINDOW)
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
    use reqwest::StatusCode;
    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        EXPENSE_CLASSIFICATION_AI_CONFIRMATION, EXPENSE_CLASSIFICATION_CONFIRMATION,
        EXPENSE_CLASSIFICATION_PATH, expense_ai_confirmation, expense_classification_terminal_code,
        materialize_export, optional_expense_idempotency_key, response_error, valid_command_name,
        valid_token, validate_base_url,
    };

    #[cfg(windows)]
    use super::CREATE_NO_WINDOW;

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
    fn expense_idempotency_key_requires_the_exact_desktop_uuid_shape() {
        let canonical = "desktop-expense:019f55e2-7d05-7bf0-b7dd-12230168a843";
        assert_eq!(
            optional_expense_idempotency_key(&json!({ "idempotencyKey": canonical }))
                .expect("canonical expense key"),
            Some(canonical.to_owned())
        );
        assert_eq!(
            optional_expense_idempotency_key(&json!({})).expect("optional key"),
            None
        );
        for invalid in [
            "019f55e2-7d05-7bf0-b7dd-12230168a843",
            "desktop-expense:{019f55e2-7d05-7bf0-b7dd-12230168a843}",
            "desktop-expense:019F55E2-7D05-7BF0-B7DD-12230168A843",
            "desktop-expense:not-a-uuid",
        ] {
            assert!(
                optional_expense_idempotency_key(&json!({ "idempotencyKey": invalid })).is_err(),
                "accepted invalid key: {invalid}"
            );
        }
    }

    #[test]
    fn expense_classification_bridge_uses_the_exact_protected_contract() {
        assert_eq!(
            EXPENSE_CLASSIFICATION_PATH,
            ["expenses", "classifications:run"]
        );
        assert_eq!(
            EXPENSE_CLASSIFICATION_CONFIRMATION,
            "expense-classification-run"
        );
        assert_eq!(
            expense_ai_confirmation("classify_expense_transactions"),
            Some(EXPENSE_CLASSIFICATION_AI_CONFIRMATION)
        );
        assert_eq!(expense_ai_confirmation("list_expense_transactions"), None);
    }

    #[test]
    fn cloud_errors_preserve_only_safe_classification_terminal_codes() {
        let terminal = json!({
            "error": { "code": "EXPENSE_CLASSIFICATION_LEASE_EXPIRED" }
        });
        assert_eq!(
            expense_classification_terminal_code(&terminal),
            Some("EXPENSE_CLASSIFICATION_LEASE_EXPIRED")
        );
        assert!(
            response_error(StatusCode::CONFLICT, &terminal, Some("request-1"))
                .contains("[TM_ERROR_CODE:EXPENSE_CLASSIFICATION_LEASE_EXPIRED]")
        );

        let arbitrary = json!({ "error": { "code": "SECRET_INTERNAL_CODE" } });
        assert_eq!(expense_classification_terminal_code(&arbitrary), None);
        let message = response_error(StatusCode::CONFLICT, &arbitrary, None);
        assert!(!message.contains("SECRET_INTERNAL_CODE"));
        assert!(!message.contains("TM_ERROR_CODE"));
    }

    #[cfg(windows)]
    #[test]
    fn credential_process_uses_the_windows_no_console_flag() {
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
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
