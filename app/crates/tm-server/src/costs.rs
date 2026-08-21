use std::{
    env, fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use uuid::Uuid;

pub const RAILWAY_API_TOKEN_ENV: &str = "TM_RAILWAY_API_TOKEN";
pub const RAILWAY_WORKSPACE_ID_ENV: &str = "TM_RAILWAY_WORKSPACE_ID";
pub const RAILWAY_HARD_LIMIT_USD_ENV: &str = "TM_RAILWAY_HARD_LIMIT_USD";
pub const DEFAULT_RAILWAY_HARD_LIMIT_MICROUSD: u64 = 30_000_000;

const RAILWAY_GRAPHQL_ENDPOINT: &str = "https://backboard.railway.com/graphql/v2";
const SUCCESS_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const FAILURE_CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const WORKSPACE_USAGE_QUERY: &str = r#"
query WorkspaceUsageContext($workspaceId: String!) {
  workspace(workspaceId: $workspaceId) {
    customer {
      currentUsage
      billingPeriod {
        start
        end
      }
      usageLimit {
        hardLimit
      }
    }
  }
}
"#;

#[derive(Clone, PartialEq, Eq)]
pub struct RailwayUsageConfig {
    token: Option<String>,
    workspace_id: Option<String>,
    fallback_hard_limit_microusd: u64,
}

impl fmt::Debug for RailwayUsageConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RailwayUsageConfig")
            .field("configured", &self.configured())
            .field("workspace_id", &self.workspace_id)
            .field(
                "fallback_hard_limit_microusd",
                &self.fallback_hard_limit_microusd,
            )
            .finish()
    }
}

impl RailwayUsageConfig {
    pub fn from_env() -> Result<Self, String> {
        let token = optional_env(RAILWAY_API_TOKEN_ENV)?;
        let workspace_id = optional_env(RAILWAY_WORKSPACE_ID_ENV)?;
        if let Some(value) = token.as_deref()
            && !valid_token(value)
        {
            return Err(format!("{RAILWAY_API_TOKEN_ENV} has an invalid format"));
        }
        if let Some(value) = workspace_id.as_deref()
            && Uuid::parse_str(value).is_err()
        {
            return Err(format!("{RAILWAY_WORKSPACE_ID_ENV} must be a UUID"));
        }
        let fallback_hard_limit_microusd = optional_env(RAILWAY_HARD_LIMIT_USD_ENV)?
            .map(|value| parse_usd_microusd(RAILWAY_HARD_LIMIT_USD_ENV, &value))
            .transpose()?
            .unwrap_or(DEFAULT_RAILWAY_HARD_LIMIT_MICROUSD);
        if fallback_hard_limit_microusd == 0 {
            return Err(format!(
                "{RAILWAY_HARD_LIMIT_USD_ENV} must be greater than zero"
            ));
        }
        let (token, workspace_id) = match (token, workspace_id) {
            (Some(token), Some(workspace_id)) => (Some(token), Some(workspace_id)),
            _ => (None, None),
        };
        Ok(Self {
            token,
            workspace_id,
            fallback_hard_limit_microusd,
        })
    }

    pub const fn disabled() -> Self {
        Self {
            token: None,
            workspace_id: None,
            fallback_hard_limit_microusd: DEFAULT_RAILWAY_HARD_LIMIT_MICROUSD,
        }
    }

    const fn configured(&self) -> bool {
        self.token.is_some() && self.workspace_id.is_some()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCostMeter {
    pub available: bool,
    pub used_microusd: Option<u64>,
    pub hard_limit_microusd: u64,
    pub billing_period_start: Option<String>,
    pub billing_period_end: Option<String>,
    pub refreshed_at: Option<String>,
    pub stale: bool,
}

impl CloudCostMeter {
    fn unavailable(hard_limit_microusd: u64) -> Self {
        Self {
            available: false,
            used_microusd: None,
            hard_limit_microusd,
            billing_period_start: None,
            billing_period_end: None,
            refreshed_at: None,
            stale: false,
        }
    }
}

#[derive(Clone)]
pub struct RailwayUsageClient {
    inner: Option<Arc<RailwayUsageInner>>,
    fallback_hard_limit_microusd: u64,
}

struct RailwayUsageInner {
    http: reqwest::Client,
    token: String,
    workspace_id: String,
    fallback_hard_limit_microusd: u64,
    cache: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    last_success: Option<CachedMeter>,
    last_failure_at: Option<Instant>,
}

#[derive(Clone)]
struct CachedMeter {
    fetched_at: Instant,
    meter: CloudCostMeter,
}

impl fmt::Debug for RailwayUsageClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RailwayUsageClient")
            .field("configured", &self.inner.is_some())
            .field(
                "fallback_hard_limit_microusd",
                &self.fallback_hard_limit_microusd,
            )
            .finish()
    }
}

impl RailwayUsageClient {
    pub fn new(config: RailwayUsageConfig) -> Result<Self, String> {
        let fallback_hard_limit_microusd = config.fallback_hard_limit_microusd;
        let (Some(token), Some(workspace_id)) = (config.token, config.workspace_id) else {
            return Ok(Self::disabled_with_limit(fallback_hard_limit_microusd));
        };
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("tm-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "Railway usage HTTPS client could not be created".to_owned())?;
        Ok(Self {
            inner: Some(Arc::new(RailwayUsageInner {
                http,
                token,
                workspace_id,
                fallback_hard_limit_microusd,
                cache: Mutex::new(CacheState::default()),
            })),
            fallback_hard_limit_microusd,
        })
    }

    pub fn disabled() -> Self {
        Self::disabled_with_limit(DEFAULT_RAILWAY_HARD_LIMIT_MICROUSD)
    }

    fn disabled_with_limit(fallback_hard_limit_microusd: u64) -> Self {
        Self {
            inner: None,
            fallback_hard_limit_microusd,
        }
    }

    pub async fn status(&self) -> CloudCostMeter {
        let Some(inner) = &self.inner else {
            return CloudCostMeter::unavailable(self.fallback_hard_limit_microusd);
        };
        let mut cache = inner.cache.lock().await;
        if let Some(cached) = &cache.last_success
            && cached.fetched_at.elapsed() < SUCCESS_CACHE_TTL
        {
            return cached.meter.clone();
        }
        if cache
            .last_failure_at
            .is_some_and(|failed_at| failed_at.elapsed() < FAILURE_CACHE_TTL)
        {
            return stale_or_unavailable(&cache, inner.fallback_hard_limit_microusd);
        }

        match fetch_workspace_usage(inner).await {
            Ok(meter) => {
                cache.last_failure_at = None;
                cache.last_success = Some(CachedMeter {
                    fetched_at: Instant::now(),
                    meter: meter.clone(),
                });
                meter
            }
            Err(error) => {
                cache.last_failure_at = Some(Instant::now());
                tracing::warn!(event = "railway_usage_refresh_failed", %error);
                stale_or_unavailable(&cache, inner.fallback_hard_limit_microusd)
            }
        }
    }
}

fn stale_or_unavailable(cache: &CacheState, fallback_hard_limit_microusd: u64) -> CloudCostMeter {
    cache.last_success.as_ref().map_or_else(
        || CloudCostMeter::unavailable(fallback_hard_limit_microusd),
        |cached| {
            let mut meter = cached.meter.clone();
            meter.stale = true;
            meter
        },
    )
}

async fn fetch_workspace_usage(inner: &RailwayUsageInner) -> Result<CloudCostMeter, String> {
    let response = inner
        .http
        .post(RAILWAY_GRAPHQL_ENDPOINT)
        .bearer_auth(&inner.token)
        .json(&json!({
            "query": WORKSPACE_USAGE_QUERY,
            "variables": { "workspaceId": inner.workspace_id }
        }))
        .send()
        .await
        .map_err(|_| "Railway usage request failed".to_owned())?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("Railway usage response exceeded the safety limit".to_owned());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Railway usage response could not be read".to_owned())?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("Railway usage response exceeded the safety limit".to_owned());
    }
    if status != StatusCode::OK {
        return Err(format!(
            "Railway usage request returned HTTP {}",
            status.as_u16()
        ));
    }
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Railway usage response was not valid JSON".to_owned())?;
    parse_workspace_usage(&payload, inner.fallback_hard_limit_microusd)
}

fn parse_workspace_usage(
    payload: &Value,
    fallback_hard_limit_microusd: u64,
) -> Result<CloudCostMeter, String> {
    if payload
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err("Railway usage API returned an error".to_owned());
    }
    let customer = payload
        .pointer("/data/workspace/customer")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            "Railway usage response did not contain workspace billing data".to_owned()
        })?;
    let used_microusd = money_value_to_microusd(
        customer
            .get("currentUsage")
            .ok_or_else(|| "Railway usage response did not contain current usage".to_owned())?,
    )?;
    let hard_limit_microusd = customer
        .pointer("/usageLimit/hardLimit")
        .filter(|value| !value.is_null())
        .map(money_value_to_microusd)
        .transpose()?
        .filter(|value| *value > 0)
        .unwrap_or(fallback_hard_limit_microusd);
    let billing_period_start = timestamp_value(customer.pointer("/billingPeriod/start"))?;
    let billing_period_end = timestamp_value(customer.pointer("/billingPeriod/end"))?;
    Ok(CloudCostMeter {
        available: true,
        used_microusd: Some(used_microusd),
        hard_limit_microusd,
        billing_period_start,
        billing_period_end,
        refreshed_at: Some(Utc::now().to_rfc3339()),
        stale: false,
    })
}

fn timestamp_value(value: Option<&Value>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = value
        .as_str()
        .ok_or_else(|| "Railway billing period timestamp was invalid".to_owned())?;
    DateTime::parse_from_rfc3339(raw)
        .map(|timestamp| Some(timestamp.to_rfc3339()))
        .map_err(|_| "Railway billing period timestamp was invalid".to_owned())
}

fn money_value_to_microusd(value: &Value) -> Result<u64, String> {
    let dollars = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|raw| raw.parse::<f64>().ok()))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| "Railway cost value was invalid".to_owned())?;
    let microusd = (dollars * 1_000_000.0).round();
    if microusd > u64::MAX as f64 {
        return Err("Railway cost value exceeded the supported range".to_owned());
    }
    Ok(microusd as u64)
}

fn parse_usd_microusd(name: &str, value: &str) -> Result<u64, String> {
    let dollars = value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| format!("{name} must be a non-negative USD amount"))?;
    let microusd = (dollars * 1_000_000.0).round();
    if microusd > u64::MAX as f64 {
        return Err(format!("{name} exceeds the supported range"));
    }
    Ok(microusd as u64)
}

fn valid_token(value: &str) -> bool {
    (20..=512).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\"' | b'\'' | b'`'))
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} cannot be empty")),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid Unicode")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DEFAULT_RAILWAY_HARD_LIMIT_MICROUSD, parse_workspace_usage, valid_token};

    #[test]
    fn workspace_usage_parses_money_and_period_without_rounding_drift() {
        let payload = json!({
            "data": {
                "workspace": {
                    "customer": {
                        "currentUsage": 0.0867828608789582,
                        "billingPeriod": {
                            "start": "2026-07-13T12:53:10.000Z",
                            "end": "2026-08-13T12:53:10.000Z"
                        },
                        "usageLimit": { "hardLimit": 30.0 }
                    }
                }
            }
        });

        let meter = parse_workspace_usage(&payload, DEFAULT_RAILWAY_HARD_LIMIT_MICROUSD)
            .expect("parse Railway usage");
        assert_eq!(meter.used_microusd, Some(86_783));
        assert_eq!(meter.hard_limit_microusd, 30_000_000);
        assert_eq!(
            meter.billing_period_start.as_deref(),
            Some("2026-07-13T12:53:10+00:00")
        );
        assert!(meter.available);
        assert!(!meter.stale);
    }

    #[test]
    fn workspace_usage_falls_back_when_no_provider_limit_is_present() {
        let payload = json!({
            "data": {
                "workspace": {
                    "customer": {
                        "currentUsage": "1.25",
                        "billingPeriod": null,
                        "usageLimit": null
                    }
                }
            }
        });

        let meter =
            parse_workspace_usage(&payload, 12_000_000).expect("parse Railway usage without limit");
        assert_eq!(meter.used_microusd, Some(1_250_000));
        assert_eq!(meter.hard_limit_microusd, 12_000_000);
        assert_eq!(meter.billing_period_start, None);
    }

    #[test]
    fn railway_token_shape_rejects_whitespace_and_quoted_values() {
        assert!(valid_token(&"a".repeat(32)));
        assert!(!valid_token("short"));
        assert!(!valid_token(&format!(
            "{} {}",
            "a".repeat(20),
            "b".repeat(20)
        )));
        assert!(!valid_token(&format!("{}\"", "a".repeat(20))));
    }
}
