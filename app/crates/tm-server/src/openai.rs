use std::{env, fmt, net::IpAddr, sync::Arc, time::Duration};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tm_core::{AiBudgetPolicy, AiBudgetStatus};
use url::Url;

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1/";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-5.6";
pub const DEFAULT_OPENAI_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_OPENAI_MONTHLY_WARNING_MICROUSD: u64 = 10_000_000;
pub const DEFAULT_OPENAI_MONTHLY_HARD_LIMIT_MICROUSD: u64 = 20_000_000;
pub const PROBE_MAXIMUM_COST_MICROUSD: u64 = 10_000;

const PROBE_EXPECTED_TEXT: &str = "TM_OPENAI_OK";
const PROBE_INSTRUCTIONS: &str =
    "This is a connectivity check. Reply with exactly TM_OPENAI_OK and nothing else.";
const PROBE_INPUT: &str = "Verify the TM server OpenAI connection.";

#[derive(Clone, PartialEq, Eq)]
pub struct OpenAiConfig {
    api_key: Option<Arc<str>>,
    model: String,
    base_url: Url,
    timeout: Duration,
    budget_policy: AiBudgetPolicy,
}

impl fmt::Debug for OpenAiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiConfig")
            .field("configured", &self.configured())
            .field("model", &self.model)
            .field("base_url", &self.base_url.as_str())
            .field("timeout", &self.timeout)
            .field("budget_policy", &self.budget_policy)
            .finish()
    }
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: DEFAULT_OPENAI_MODEL.to_owned(),
            base_url: Url::parse(DEFAULT_OPENAI_BASE_URL)
                .expect("the built-in OpenAI base URL must be valid"),
            timeout: Duration::from_secs(DEFAULT_OPENAI_TIMEOUT_SECS),
            budget_policy: AiBudgetPolicy {
                warning_limit_microusd: DEFAULT_OPENAI_MONTHLY_WARNING_MICROUSD,
                hard_limit_microusd: DEFAULT_OPENAI_MONTHLY_HARD_LIMIT_MICROUSD,
            },
        }
    }
}

impl OpenAiConfig {
    pub fn from_env() -> Result<Self, String> {
        let api_key = optional_env("OPENAI_API_KEY")?
            .map(|value| validate_api_key(&value))
            .transpose()?
            .map(Arc::<str>::from);
        let model =
            optional_env("TM_OPENAI_MODEL")?.unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_owned());
        validate_model(&model)?;

        let base_url = optional_env("TM_OPENAI_BASE_URL")?
            .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_owned());
        let base_url = validate_base_url(&base_url)?;

        let timeout_secs = optional_env("TM_OPENAI_TIMEOUT_SECS")?
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|error| format!("TM_OPENAI_TIMEOUT_SECS is invalid: {error}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_OPENAI_TIMEOUT_SECS);
        if !(1..=120).contains(&timeout_secs) {
            return Err("TM_OPENAI_TIMEOUT_SECS must be between 1 and 120".to_owned());
        }

        let warning_limit_microusd = optional_env("TM_OPENAI_MONTHLY_WARNING_USD")?
            .map(|value| parse_usd_microusd("TM_OPENAI_MONTHLY_WARNING_USD", &value))
            .transpose()?
            .unwrap_or(DEFAULT_OPENAI_MONTHLY_WARNING_MICROUSD);
        let hard_limit_microusd = optional_env("TM_OPENAI_MONTHLY_HARD_LIMIT_USD")?
            .map(|value| parse_usd_microusd("TM_OPENAI_MONTHLY_HARD_LIMIT_USD", &value))
            .transpose()?
            .unwrap_or(DEFAULT_OPENAI_MONTHLY_HARD_LIMIT_MICROUSD);
        if warning_limit_microusd == 0
            || hard_limit_microusd == 0
            || warning_limit_microusd > hard_limit_microusd
        {
            return Err(
                "TM OpenAI monthly warning must be positive and no greater than the hard limit"
                    .to_owned(),
            );
        }

        Ok(Self {
            api_key,
            model,
            base_url,
            timeout: Duration::from_secs(timeout_secs),
            budget_policy: AiBudgetPolicy {
                warning_limit_microusd,
                hard_limit_microusd,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        api_key: Option<&str>,
        model: &str,
        base_url: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        let api_key = api_key
            .map(validate_api_key)
            .transpose()?
            .map(Arc::<str>::from);
        validate_model(model)?;

        Ok(Self {
            api_key,
            model: model.to_owned(),
            base_url: validate_base_url(base_url)?,
            timeout,
            budget_policy: AiBudgetPolicy {
                warning_limit_microusd: DEFAULT_OPENAI_MONTHLY_WARNING_MICROUSD,
                hard_limit_microusd: DEFAULT_OPENAI_MONTHLY_HARD_LIMIT_MICROUSD,
            },
        })
    }

    #[must_use]
    pub fn configured(&self) -> bool {
        self.api_key.is_some()
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    #[must_use]
    pub const fn budget_policy(&self) -> AiBudgetPolicy {
        self.budget_policy
    }

    #[must_use]
    pub fn estimate_cost_microusd(&self, usage: &ProbeUsage) -> Option<u64> {
        if !self.model.starts_with("gpt-5.6") {
            return None;
        }
        let uncached_input = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
        let numerator = u128::from(uncached_input)
            .saturating_mul(5_000_000)
            .saturating_add(u128::from(usage.cached_input_tokens).saturating_mul(500_000))
            .saturating_add(u128::from(usage.output_tokens).saturating_mul(30_000_000));
        let rounded_up = numerator.saturating_add(999_999) / 1_000_000;
        u64::try_from(rounded_up).ok()
    }

    fn responses_url(&self) -> Result<Url, OpenAiError> {
        self.base_url
            .join("responses")
            .map_err(|_| OpenAiError::InvalidConfiguration)
    }
}

#[derive(Clone)]
pub struct OpenAiClient {
    config: OpenAiConfig,
    http: reqwest::Client,
}

impl OpenAiClient {
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            config: OpenAiConfig::default(),
            http: reqwest::Client::new(),
        }
    }

    pub fn new(config: OpenAiConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("tm-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("failed to build OpenAI HTTP client: {error}"))?;
        Ok(Self { config, http })
    }

    #[must_use]
    pub fn config(&self) -> &OpenAiConfig {
        &self.config
    }

    pub async fn probe(&self) -> Result<OpenAiProbeResult, OpenAiError> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or(OpenAiError::NotConfigured)?;
        let response = self
            .http
            .post(self.config.responses_url()?)
            .bearer_auth(api_key)
            .json(&ProbeRequest {
                model: &self.config.model,
                instructions: PROBE_INSTRUCTIONS,
                input: PROBE_INPUT,
                max_output_tokens: 32,
                store: false,
                reasoning: ProbeReasoning { effort: "none" },
            })
            .send()
            .await
            .map_err(|_| OpenAiError::Transport)?;

        let status = response.status();
        let upstream_request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        if !status.is_success() {
            return Err(classify_upstream_error(status, upstream_request_id));
        }

        let response = response
            .json::<ProbeResponse>()
            .await
            .map_err(|_| OpenAiError::InvalidResponse)?;
        let output_text = response
            .output
            .iter()
            .flat_map(|item| &item.content)
            .filter(|content| content.kind == "output_text")
            .filter_map(|content| content.text.as_deref())
            .collect::<String>();

        if output_text.trim().is_empty() {
            return Err(OpenAiError::InvalidResponse);
        }

        Ok(OpenAiProbeResult {
            status: response.status,
            provider: "openai",
            response_id: response.id,
            upstream_request_id,
            model: response.model,
            output_text,
            expected_text: PROBE_EXPECTED_TEXT,
            matched_expected_text: response_matches_probe(&response.output),
            usage: response.usage.map(Into::into),
            stored: false,
            estimated_cost_microusd: None,
            budget: None,
        })
    }
}

#[derive(Debug)]
pub enum OpenAiError {
    NotConfigured,
    InvalidConfiguration,
    Transport,
    Authentication { upstream_request_id: Option<String> },
    RateLimited { upstream_request_id: Option<String> },
    RequestRejected { upstream_request_id: Option<String> },
    UpstreamUnavailable { upstream_request_id: Option<String> },
    InvalidResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiProbeResult {
    pub status: String,
    pub provider: &'static str,
    pub response_id: String,
    pub upstream_request_id: Option<String>,
    pub model: String,
    pub output_text: String,
    pub expected_text: &'static str,
    pub matched_expected_text: bool,
    pub usage: Option<ProbeUsage>,
    pub stored: bool,
    pub estimated_cost_microusd: Option<u64>,
    pub budget: Option<AiBudgetStatus>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Serialize)]
struct ProbeRequest<'a> {
    model: &'a str,
    instructions: &'static str,
    input: &'static str,
    max_output_tokens: u32,
    store: bool,
    reasoning: ProbeReasoning,
}

#[derive(Serialize)]
struct ProbeReasoning {
    effort: &'static str,
}

#[derive(Deserialize)]
struct ProbeResponse {
    id: String,
    status: String,
    model: String,
    #[serde(default)]
    output: Vec<ProbeOutputItem>,
    usage: Option<UpstreamUsage>,
}

#[derive(Deserialize)]
struct ProbeOutputItem {
    #[serde(default)]
    content: Vec<ProbeOutputContent>,
}

#[derive(Deserialize)]
struct ProbeOutputContent {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct UpstreamUsage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    #[serde(default)]
    input_tokens_details: Option<InputTokenDetails>,
}

#[derive(Deserialize)]
struct InputTokenDetails {
    #[serde(default)]
    cached_tokens: u64,
}

impl From<UpstreamUsage> for ProbeUsage {
    fn from(value: UpstreamUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            cached_input_tokens: value
                .input_tokens_details
                .map_or(0, |details| details.cached_tokens),
            output_tokens: value.output_tokens,
            total_tokens: value.total_tokens,
        }
    }
}

fn parse_usd_microusd(name: &str, value: &str) -> Result<u64, String> {
    let value = value.trim();
    let (whole, fractional) = value.split_once('.').map_or((value, ""), |parts| parts);
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.len() > 6
    {
        return Err(format!(
            "{name} must be a positive USD decimal with at most 6 digits after the decimal point"
        ));
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|error| format!("{name} is invalid: {error}"))?;
    let fractional = if fractional.is_empty() {
        0
    } else {
        format!("{fractional:0<6}")
            .parse::<u64>()
            .map_err(|error| format!("{name} is invalid: {error}"))?
    };
    whole
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(fractional))
        .ok_or_else(|| format!("{name} is too large"))
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must contain valid Unicode")),
    }
}

fn validate_api_key(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("OPENAI_API_KEY cannot be empty".to_owned());
    }
    if value.len() > 2048 || value.contains(['\r', '\n']) {
        return Err("OPENAI_API_KEY has an invalid format".to_owned());
    }
    Ok(value.to_owned())
}

fn validate_model(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err("TM_OPENAI_MODEL has an invalid format".to_owned());
    }
    Ok(())
}

fn validate_base_url(value: &str) -> Result<Url, String> {
    let mut url =
        Url::parse(value).map_err(|error| format!("TM_OPENAI_BASE_URL is invalid: {error}"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "TM_OPENAI_BASE_URL cannot contain credentials, query parameters, or a fragment"
                .to_owned(),
        );
    }

    let loopback_http = url.scheme() == "http" && is_loopback_host(&url);
    if url.scheme() != "https" && !loopback_http {
        return Err(
            "TM_OPENAI_BASE_URL must use HTTPS; HTTP is allowed only for loopback tests".to_owned(),
        );
    }
    if url.host_str().is_none() {
        return Err("TM_OPENAI_BASE_URL must include a host".to_owned());
    }
    if url.scheme() == "https"
        && !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"))
    {
        return Err(
            "TM_OPENAI_BASE_URL must use api.openai.com; loopback HTTP is allowed only for tests"
                .to_owned(),
        );
    }

    let normalized_path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&normalized_path);
    Ok(url)
}

fn is_loopback_host(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

fn classify_upstream_error(status: StatusCode, upstream_request_id: Option<String>) -> OpenAiError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => OpenAiError::Authentication {
            upstream_request_id,
        },
        StatusCode::TOO_MANY_REQUESTS => OpenAiError::RateLimited {
            upstream_request_id,
        },
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY => {
            OpenAiError::RequestRejected {
                upstream_request_id,
            }
        }
        _ => OpenAiError::UpstreamUnavailable {
            upstream_request_id,
        },
    }
}

fn response_matches_probe(output: &[ProbeOutputItem]) -> bool {
    output
        .iter()
        .flat_map(|item| &item.content)
        .filter(|content| content.kind == "output_text")
        .filter_map(|content| content.text.as_deref())
        .collect::<String>()
        .trim()
        == PROBE_EXPECTED_TEXT
}
