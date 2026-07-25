use std::{
    collections::{BTreeMap, BTreeSet},
    env, fmt,
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use chrono_tz::America::New_York;
use reqwest::{StatusCode, Url, header::HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::openai::{OpenAiClient, OpenAiError, ProbeUsage};

#[cfg(test)]
use std::net::IpAddr;

pub const STOCK_SCREEN_ENABLED_ENV: &str = "TM_STOCK_SCREEN_ENABLED";
pub const STOCK_AI_ENABLED_ENV: &str = "TM_STOCK_AI_ENABLED";
pub const STOCK_LICENSE_ACK_ENV: &str = "TM_STOCK_LICENSE_ACK";
pub const STOCK_OPENAI_MODEL_ENV: &str = "TM_STOCK_OPENAI_MODEL";
pub const ALPACA_KEY_ID_ENV: &str = "APCA_API_KEY_ID";
pub const ALPACA_SECRET_KEY_ENV: &str = "APCA_API_SECRET_KEY";
pub const DEFAULT_STOCK_OPENAI_MODEL: &str = "gpt-5.4-nano-2026-03-17";
pub const STOCK_AI_MAX_CANDIDATES: usize = 40;
pub const STOCK_DAILY_LOCAL_TIME: &str = "10:30:00";
pub const STOCK_MINIMUM_COVERAGE_BASIS_POINTS: u32 = 9_800;
pub const STOCK_UNIVERSE_MAX_AGE_DAYS: i64 = 14;

const ALPACA_DATA_BASE_URL: &str = "https://data.alpaca.markets/v2/";
const DATAHUB_CONSTITUENTS_URL: &str = "https://raw.githubusercontent.com/datasets/s-and-p-500-companies/refs/heads/main/data/constituents.csv";
const DATAHUB_LICENSE: &str = "PDDL-1.0";
const DATAHUB_ATTRIBUTION: &str =
    "DataHub S&P 500 Companies dataset (source data derived from Wikipedia)";
const PROVIDER_TIMEOUT_SECONDS: u64 = 40;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_BAR_PAGES: usize = 5;
const BAR_LOOKBACK_DAYS: i64 = 70;
const MARKET_DATA_DELAY_MINUTES: i64 = 15;
const STOCK_AI_MAX_OUTPUT_TOKENS: u32 = 200;
const STOCK_AI_SAFETY_IDENTIFIER: &str = "tm-single-user-stock-screen";

#[derive(Clone, PartialEq, Eq)]
pub struct StockConfig {
    screen_enabled: bool,
    ai_enabled: bool,
    license_ack: bool,
    alpaca_key_id: Option<Arc<str>>,
    alpaca_secret_key: Option<Arc<str>>,
    openai_model: String,
    data_base_url: Url,
    universe_url: Url,
}

impl fmt::Debug for StockConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StockConfig")
            .field("screen_enabled", &self.screen_enabled)
            .field("ai_enabled", &self.ai_enabled)
            .field("license_ack", &self.license_ack)
            .field("alpaca_configured", &self.alpaca_configured())
            .field("openai_model", &self.openai_model)
            .field("data_base_url", &self.data_base_url.as_str())
            .field("universe_url", &self.universe_url.as_str())
            .finish()
    }
}

impl Default for StockConfig {
    fn default() -> Self {
        Self {
            screen_enabled: false,
            ai_enabled: false,
            license_ack: false,
            alpaca_key_id: None,
            alpaca_secret_key: None,
            openai_model: DEFAULT_STOCK_OPENAI_MODEL.to_owned(),
            data_base_url: built_in_url(ALPACA_DATA_BASE_URL),
            universe_url: built_in_url(DATAHUB_CONSTITUENTS_URL),
        }
    }
}

impl StockConfig {
    pub fn from_env() -> Result<Self, String> {
        let screen_enabled = boolean_env(STOCK_SCREEN_ENABLED_ENV, false)?;
        let ai_enabled = boolean_env(STOCK_AI_ENABLED_ENV, false)?;
        let license_ack = boolean_env(STOCK_LICENSE_ACK_ENV, false)?;
        let alpaca_key_id = secret_env(ALPACA_KEY_ID_ENV)?.map(Arc::<str>::from);
        let alpaca_secret_key = secret_env(ALPACA_SECRET_KEY_ENV)?.map(Arc::<str>::from);
        let openai_model = optional_env(STOCK_OPENAI_MODEL_ENV)?
            .unwrap_or_else(|| DEFAULT_STOCK_OPENAI_MODEL.to_owned());

        if openai_model != DEFAULT_STOCK_OPENAI_MODEL {
            return Err(format!(
                "{STOCK_OPENAI_MODEL_ENV} must be {DEFAULT_STOCK_OPENAI_MODEL}; unknown pricing is blocked"
            ));
        }
        if alpaca_key_id.is_some() != alpaca_secret_key.is_some() {
            return Err(format!(
                "{ALPACA_KEY_ID_ENV} and {ALPACA_SECRET_KEY_ENV} must be configured together"
            ));
        }
        if screen_enabled && !license_ack {
            return Err(format!(
                "{STOCK_LICENSE_ACK_ENV}=true is required before stock data collection can be enabled"
            ));
        }
        if screen_enabled && alpaca_key_id.is_none() {
            return Err(format!(
                "{ALPACA_KEY_ID_ENV} and {ALPACA_SECRET_KEY_ENV} are required when {STOCK_SCREEN_ENABLED_ENV}=true"
            ));
        }
        if ai_enabled && !screen_enabled {
            return Err(format!(
                "{STOCK_AI_ENABLED_ENV}=true requires {STOCK_SCREEN_ENABLED_ENV}=true"
            ));
        }

        Ok(Self {
            screen_enabled,
            ai_enabled,
            license_ack,
            alpaca_key_id,
            alpaca_secret_key,
            openai_model,
            ..Self::default()
        })
    }

    #[must_use]
    pub const fn screen_enabled(&self) -> bool {
        self.screen_enabled
    }

    #[must_use]
    pub const fn ai_enabled(&self) -> bool {
        self.ai_enabled
    }

    #[must_use]
    pub const fn license_ack(&self) -> bool {
        self.license_ack
    }

    #[must_use]
    pub fn openai_model(&self) -> &str {
        &self.openai_model
    }

    #[must_use]
    pub fn alpaca_configured(&self) -> bool {
        self.alpaca_key_id.is_some() && self.alpaca_secret_key.is_some()
    }

    #[cfg(test)]
    pub(crate) fn for_test(data_base_url: &str, universe_url: &str) -> Self {
        Self {
            screen_enabled: true,
            ai_enabled: false,
            license_ack: true,
            alpaca_key_id: Some(Arc::from("test-key")),
            alpaca_secret_key: Some(Arc::from("test-secret")),
            openai_model: DEFAULT_STOCK_OPENAI_MODEL.to_owned(),
            data_base_url: validate_test_url(data_base_url).expect("test data URL"),
            universe_url: validate_test_url(universe_url).expect("test universe URL"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UniverseMember {
    pub symbol: String,
    pub display_name: String,
    pub exchange: String,
    pub sector: Option<String>,
    pub sub_industry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UniverseDownload {
    pub source_url: String,
    pub revision: String,
    pub sha256: String,
    pub license: String,
    pub attribution: String,
    pub fetched_at: String,
    pub members: Vec<UniverseMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedBar {
    pub symbol: String,
    pub session_date: NaiveDate,
    pub close_microusd: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketDownload {
    pub feed: &'static str,
    pub adjustment: &'static str,
    pub source_sha256: String,
    pub fetched_at: String,
    pub sessions: Vec<NaiveDate>,
    pub bars: Vec<DownloadedBar>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StockAiCandidate {
    pub symbol: String,
    #[serde(skip_serializing)]
    pub display_name: String,
    pub horizon: u8,
    #[serde(skip_serializing)]
    pub return_micros: i64,
    pub return_basis_points: i32,
    #[serde(skip_serializing)]
    pub direction: String,
    #[serde(skip_serializing)]
    pub band: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockAiContent {
    pub headline: String,
    pub bullets: Vec<String>,
    pub notable_symbols: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StockAiSelection {
    notable_symbols: Vec<String>,
}

#[derive(Debug)]
pub struct StockAiExecution {
    pub content: StockAiContent,
    pub response_id: String,
    pub upstream_request_id: Option<String>,
    pub model: String,
    pub usage: Option<ProbeUsage>,
}

#[derive(Debug)]
pub enum StockDataError {
    NotConfigured,
    Authentication,
    RateLimited,
    UpstreamUnavailable,
    ResponseTooLarge,
    InvalidResponse,
    CoverageUnavailable,
}

#[derive(Clone)]
pub struct StockDataClient {
    config: StockConfig,
    http: reqwest::Client,
}

impl StockDataClient {
    pub fn new(config: StockConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(PROVIDER_TIMEOUT_SECONDS))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("tm-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("failed to build stock data HTTP client: {error}"))?;
        Ok(Self { config, http })
    }

    #[must_use]
    pub fn config(&self) -> &StockConfig {
        &self.config
    }

    pub async fn fetch_universe(&self) -> Result<UniverseDownload, StockDataError> {
        self.require_configured()?;
        let universe_response = self
            .http
            .get(self.config.universe_url.clone())
            .send()
            .await
            .map_err(|_| StockDataError::UpstreamUnavailable)?;
        if !universe_response.status().is_success() {
            return Err(
                if universe_response.status() == StatusCode::TOO_MANY_REQUESTS {
                    StockDataError::RateLimited
                } else {
                    StockDataError::UpstreamUnavailable
                },
            );
        }
        let universe_bytes = response_bytes(universe_response).await?;
        let sha256 = format!("{:x}", Sha256::digest(&universe_bytes));
        let rows = parse_datahub_members(&universe_bytes)?;
        if !(450..=550).contains(&rows.len()) {
            return Err(StockDataError::InvalidResponse);
        }

        let mut seen = BTreeSet::new();
        let mut members = Vec::with_capacity(rows.len());
        for row in rows {
            let symbol = normalize_symbol(&row.symbol)?;
            if !seen.insert(symbol.clone()) {
                return Err(StockDataError::InvalidResponse);
            }
            let display_name = clean_display_name(&row.security)?;
            members.push(UniverseMember {
                symbol,
                display_name,
                exchange: "US".to_owned(),
                sector: clean_optional_metadata(row.sector, 160)?,
                sub_industry: clean_optional_metadata(row.sub_industry, 200)?,
            });
        }
        members.sort_by(|left, right| left.symbol.cmp(&right.symbol));

        Ok(UniverseDownload {
            source_url: self.config.universe_url.as_str().to_owned(),
            revision: sha256.clone(),
            sha256,
            license: DATAHUB_LICENSE.to_owned(),
            attribution: DATAHUB_ATTRIBUTION.to_owned(),
            fetched_at: Utc::now().to_rfc3339(),
            members,
        })
    }

    pub async fn fetch_daily_bars(
        &self,
        symbols: &[String],
        as_of: DateTime<Utc>,
    ) -> Result<MarketDownload, StockDataError> {
        self.require_configured()?;
        if symbols.is_empty() || symbols.len() > 550 {
            return Err(StockDataError::InvalidResponse);
        }
        let mut requested = BTreeSet::new();
        for symbol in symbols {
            requested.insert(normalize_symbol(symbol)?);
        }
        requested.insert("SPY".to_owned());
        let symbols_csv = requested.iter().cloned().collect::<Vec<_>>().join(",");
        let end = as_of - ChronoDuration::minutes(MARKET_DATA_DELAY_MINUTES);
        let start = end - ChronoDuration::days(BAR_LOOKBACK_DAYS);
        let mut next_page_token: Option<String> = None;
        let mut bars = BTreeMap::<(String, NaiveDate), i64>::new();
        let mut source_hasher = Sha256::new();
        source_hasher.update(b"tm-alpaca-bars-v1\0");
        source_hasher.update(symbols_csv.as_bytes());
        source_hasher.update(b"\0");
        source_hasher.update(start.to_rfc3339().as_bytes());
        source_hasher.update(b"\0");
        source_hasher.update(end.to_rfc3339().as_bytes());
        source_hasher.update(b"\0timeframe=1Day\0adjustment=split\0feed=sip\0currency=USD\0");

        for page in 0..MAX_BAR_PAGES {
            let mut url = self
                .config
                .data_base_url
                .join("stocks/bars")
                .map_err(|_| StockDataError::InvalidResponse)?;
            {
                let mut query = url.query_pairs_mut();
                query
                    .append_pair("symbols", &symbols_csv)
                    .append_pair("timeframe", "1Day")
                    .append_pair("start", &start.to_rfc3339())
                    .append_pair("end", &end.to_rfc3339())
                    .append_pair("limit", "10000")
                    .append_pair("adjustment", "split")
                    .append_pair("feed", "sip")
                    .append_pair("currency", "USD")
                    .append_pair("sort", "asc");
                if let Some(token) = &next_page_token {
                    query.append_pair("page_token", token);
                }
            }
            let response = self.authorized_get(url).await?;
            let bytes = response_bytes(response).await?;
            source_hasher.update((bytes.len() as u64).to_be_bytes());
            source_hasher.update(&bytes);
            let payload: AlpacaBarsResponse =
                serde_json::from_slice(&bytes).map_err(|_| StockDataError::InvalidResponse)?;
            for (raw_symbol, symbol_bars) in payload.bars {
                let symbol = normalize_symbol(&raw_symbol)?;
                if !requested.contains(&symbol) {
                    return Err(StockDataError::InvalidResponse);
                }
                for bar in symbol_bars {
                    let timestamp = parse_bar_timestamp(&bar.timestamp)?;
                    if timestamp < start || timestamp > end {
                        return Err(StockDataError::InvalidResponse);
                    }
                    let date = timestamp.with_timezone(&New_York).date_naive();
                    let close = decimal_microusd(&bar.close)?;
                    if bars.insert((symbol.clone(), date), close).is_some() {
                        return Err(StockDataError::InvalidResponse);
                    }
                }
            }
            next_page_token = payload.next_page_token.filter(|value| !value.is_empty());
            if next_page_token.is_none() {
                break;
            }
            if page + 1 == MAX_BAR_PAGES {
                return Err(StockDataError::InvalidResponse);
            }
        }

        let sessions = bars
            .keys()
            .filter_map(|(symbol, date)| (symbol == "SPY").then_some(*date))
            .collect::<Vec<_>>();
        if sessions.len() < 22 {
            return Err(StockDataError::CoverageUnavailable);
        }
        let session_set = sessions.iter().copied().collect::<BTreeSet<_>>();
        let bars = bars
            .into_iter()
            .filter(|((symbol, session_date), _)| {
                symbol != "SPY" && session_set.contains(session_date)
            })
            .map(|((symbol, session_date), close_microusd)| DownloadedBar {
                symbol,
                session_date,
                close_microusd,
            })
            .collect();

        Ok(MarketDownload {
            feed: "sip",
            adjustment: "split",
            source_sha256: format!("{:x}", source_hasher.finalize()),
            fetched_at: Utc::now().to_rfc3339(),
            sessions,
            bars,
        })
    }

    async fn authorized_get(&self, url: Url) -> Result<reqwest::Response, StockDataError> {
        let key_id = self
            .config
            .alpaca_key_id
            .as_deref()
            .ok_or(StockDataError::NotConfigured)?;
        let secret = self
            .config
            .alpaca_secret_key
            .as_deref()
            .ok_or(StockDataError::NotConfigured)?;
        let response = self
            .http
            .get(url)
            .header(
                "APCA-API-KEY-ID",
                HeaderValue::from_str(key_id).map_err(|_| StockDataError::NotConfigured)?,
            )
            .header(
                "APCA-API-SECRET-KEY",
                HeaderValue::from_str(secret).map_err(|_| StockDataError::NotConfigured)?,
            )
            .send()
            .await
            .map_err(|_| StockDataError::UpstreamUnavailable)?;
        match response.status() {
            status if status.is_success() => Ok(response),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(StockDataError::Authentication),
            StatusCode::TOO_MANY_REQUESTS => Err(StockDataError::RateLimited),
            _ => Err(StockDataError::UpstreamUnavailable),
        }
    }

    fn require_configured(&self) -> Result<(), StockDataError> {
        if !self.config.screen_enabled
            || !self.config.license_ack
            || !self.config.alpaca_configured()
        {
            return Err(StockDataError::NotConfigured);
        }
        Ok(())
    }
}

pub async fn run_ai_summary(
    openai: OpenAiClient,
    model: &str,
    market_date: NaiveDate,
    candidates: &[StockAiCandidate],
) -> Result<StockAiExecution, StockAiError> {
    if model != DEFAULT_STOCK_OPENAI_MODEL || candidates.len() > STOCK_AI_MAX_CANDIDATES {
        return Err(StockAiError {
            possibly_billed: false,
            upstream_request_id: None,
            kind: StockAiErrorKind::InvalidConfiguration,
        });
    }
    let allowed_symbols = candidates
        .iter()
        .map(|candidate| candidate.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let payload = json!({
        "marketDate": market_date,
        "returnType": "split-adjusted close-to-close price return excluding dividends",
        "candidates": candidates,
    });
    let request = json!({
        "model": model,
        "instructions": "Select at most six notable ticker symbols from the supplied deterministic price-movement candidates. Return only the requested JSON selection. Do not write prose, calculate values, predict, recommend, or explain causes.",
        "input": [{
            "role": "user",
            "content": serde_json::to_string(&payload)
                .map_err(|_| StockAiError {
                    possibly_billed: false,
                    upstream_request_id: None,
                    kind: StockAiErrorKind::InvalidConfiguration,
                })?
        }],
        "max_output_tokens": STOCK_AI_MAX_OUTPUT_TOKENS,
        "store": false,
        "service_tier": "default",
        "reasoning": {"effort": "none"},
        "text": {
            "verbosity": "low",
            "format": {
                "type": "json_schema",
                "name": "tm_stock_daily_digest",
                "strict": true,
                "schema": stock_ai_schema(&allowed_symbols)
            }
        },
        "safety_identifier": STOCK_AI_SAFETY_IDENTIFIER,
    });
    let call = openai.create_response(&request).await.map_err(|error| {
        let upstream_request_id = error.upstream_request_id().map(ToOwned::to_owned);
        StockAiError {
            possibly_billed: matches!(
                &error,
                OpenAiError::Transport
                    | OpenAiError::UpstreamUnavailable { .. }
                    | OpenAiError::InvalidResponse { .. }
            ),
            upstream_request_id,
            kind: StockAiErrorKind::OpenAi(error),
        }
    })?;
    let upstream_request_id = call.upstream_request_id.clone();
    if call.response.status != "completed" {
        return Err(StockAiError {
            possibly_billed: true,
            upstream_request_id,
            kind: StockAiErrorKind::InvalidResponse,
        });
    }
    if call.response.model != model {
        return Err(StockAiError {
            possibly_billed: true,
            upstream_request_id,
            kind: StockAiErrorKind::InvalidResponse,
        });
    }
    let output = output_text(&call.response.output).ok_or_else(|| StockAiError {
        possibly_billed: true,
        upstream_request_id: upstream_request_id.clone(),
        kind: StockAiErrorKind::InvalidResponse,
    })?;
    let selection: StockAiSelection = serde_json::from_str(&output).map_err(|_| StockAiError {
        possibly_billed: true,
        upstream_request_id: upstream_request_id.clone(),
        kind: StockAiErrorKind::InvalidResponse,
    })?;
    validate_ai_selection(&selection, &allowed_symbols).map_err(|_| StockAiError {
        possibly_billed: true,
        upstream_request_id: upstream_request_id.clone(),
        kind: StockAiErrorKind::InvalidResponse,
    })?;
    let content = deterministic_ai_content(market_date, candidates, selection);
    Ok(StockAiExecution {
        content,
        response_id: call.response.id,
        upstream_request_id: call.upstream_request_id,
        model: call.response.model,
        usage: call.response.usage.map(Into::into),
    })
}

#[derive(Debug)]
pub struct StockAiError {
    pub possibly_billed: bool,
    pub upstream_request_id: Option<String>,
    pub kind: StockAiErrorKind,
}

#[derive(Debug)]
pub enum StockAiErrorKind {
    OpenAi(OpenAiError),
    InvalidConfiguration,
    InvalidResponse,
}

struct DataHubMember {
    symbol: String,
    security: String,
    sector: Option<String>,
    sub_industry: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlpacaBarsResponse {
    #[serde(default)]
    bars: BTreeMap<String, Vec<AlpacaBar>>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlpacaBar {
    #[serde(rename = "t")]
    timestamp: String,
    #[serde(rename = "c")]
    close: Value,
}

async fn response_bytes(mut response: reqwest::Response) -> Result<Vec<u8>, StockDataError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(StockDataError::ResponseTooLarge);
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(8192)
            .min(MAX_RESPONSE_BYTES),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| StockDataError::UpstreamUnavailable)?
    {
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(StockDataError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn normalize_symbol(value: &str) -> Result<String, StockDataError> {
    let value = value.trim().to_ascii_uppercase();
    if value.is_empty()
        || value.len() > 10
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(StockDataError::InvalidResponse);
    }
    Ok(value)
}

fn clean_display_name(value: &str) -> Result<String, StockDataError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 160 || value.contains(['\r', '\n', '\0']) {
        return Err(StockDataError::InvalidResponse);
    }
    Ok(value.to_owned())
}

fn clean_optional_metadata(
    value: Option<String>,
    maximum_characters: usize,
) -> Result<Option<String>, StockDataError> {
    match value {
        None => Ok(None),
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            if value.chars().count() > maximum_characters || value.contains(['\r', '\n', '\0']) {
                return Err(StockDataError::InvalidResponse);
            }
            Ok(Some(value.to_owned()))
        }
    }
}

fn parse_bar_timestamp(value: &str) -> Result<DateTime<Utc>, StockDataError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| StockDataError::InvalidResponse)
}

fn parse_datahub_members(bytes: &[u8]) -> Result<Vec<DataHubMember>, StockDataError> {
    let input = std::str::from_utf8(bytes).map_err(|_| StockDataError::InvalidResponse)?;
    let records = parse_csv_records(input)?;
    let header = records.first().ok_or(StockDataError::InvalidResponse)?;
    let symbol_index = header
        .iter()
        .position(|field| field == "Symbol")
        .ok_or(StockDataError::InvalidResponse)?;
    let security_index = header
        .iter()
        .position(|field| field == "Security")
        .ok_or(StockDataError::InvalidResponse)?;
    let sector_index = header.iter().position(|field| field == "GICS Sector");
    let sub_industry_index = header.iter().position(|field| field == "GICS Sub-Industry");
    let mut members = Vec::with_capacity(records.len().saturating_sub(1));
    for record in records.into_iter().skip(1) {
        if record.iter().all(|field| field.is_empty()) {
            continue;
        }
        let symbol = record
            .get(symbol_index)
            .ok_or(StockDataError::InvalidResponse)?
            .to_owned();
        let security = record
            .get(security_index)
            .ok_or(StockDataError::InvalidResponse)?
            .to_owned();
        let sector = sector_index
            .and_then(|index| record.get(index))
            .filter(|value| !value.trim().is_empty())
            .cloned();
        let sub_industry = sub_industry_index
            .and_then(|index| record.get(index))
            .filter(|value| !value.trim().is_empty())
            .cloned();
        members.push(DataHubMember {
            symbol,
            security,
            sector,
            sub_industry,
        });
    }
    Ok(members)
}

fn parse_csv_records(input: &str) -> Result<Vec<Vec<String>>, StockDataError> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut characters = input.chars().peekable();
    let mut quoted = false;
    while let Some(character) = characters.next() {
        if quoted {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    field.push('"');
                    characters.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        match character {
            '"' if field.is_empty() => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            '\r' if characters.peek() == Some(&'\n') => {}
            '\r' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(character),
        }
    }
    if quoted {
        return Err(StockDataError::InvalidResponse);
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

fn decimal_microusd(value: &Value) -> Result<i64, StockDataError> {
    let Value::Number(number) = value else {
        return Err(StockDataError::InvalidResponse);
    };
    parse_decimal_microusd(&number.to_string()).ok_or(StockDataError::InvalidResponse)
}

fn parse_decimal_microusd(value: &str) -> Option<i64> {
    if value.starts_with('-') || value.starts_with('+') {
        return None;
    }
    let (mantissa, exponent) = value
        .find(|character| matches!(character, 'e' | 'E'))
        .map_or((value, 0_i32), |index| {
            let (mantissa, exponent) = value.split_at(index);
            (
                mantissa,
                exponent
                    .get(1..)
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(i32::MIN),
            )
        });
    if exponent == i32::MIN || !(-12..=12).contains(&exponent) {
        return None;
    }
    let (whole, fractional) = mantissa
        .split_once('.')
        .map_or((mantissa, ""), |parts| parts);
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fractional}");
    let digits = digits.parse::<i128>().ok()?;
    let scale = i32::try_from(fractional.len())
        .ok()?
        .saturating_sub(exponent);
    let microusd = if scale <= 6 {
        digits.checked_mul(10_i128.checked_pow((6 - scale) as u32)?)?
    } else {
        let divisor = 10_i128.checked_pow((scale - 6) as u32)?;
        let quotient = digits / divisor;
        let remainder = digits % divisor;
        quotient.checked_add(i128::from(remainder.saturating_mul(2) >= divisor))?
    };
    i64::try_from(microusd).ok().filter(|value| *value > 0)
}

fn output_text(output: &[Value]) -> Option<String> {
    let text = output
        .iter()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<String>();
    (!text.trim().is_empty()).then_some(text)
}

fn validate_ai_selection(
    selection: &StockAiSelection,
    allowed_symbols: &BTreeSet<&str>,
) -> Result<(), ()> {
    if selection.notable_symbols.is_empty()
        || selection.notable_symbols.len() > 6
        || selection
            .notable_symbols
            .iter()
            .any(|symbol| !allowed_symbols.contains(symbol.as_str()))
    {
        return Err(());
    }
    let unique = selection.notable_symbols.iter().collect::<BTreeSet<_>>();
    if unique.len() != selection.notable_symbols.len() {
        return Err(());
    }
    Ok(())
}

fn deterministic_ai_content(
    market_date: NaiveDate,
    candidates: &[StockAiCandidate],
    selection: StockAiSelection,
) -> StockAiContent {
    let bullets = selection
        .notable_symbols
        .iter()
        .filter_map(|symbol| {
            let mut matches = candidates
                .iter()
                .filter(|candidate| candidate.symbol == *symbol)
                .collect::<Vec<_>>();
            matches.sort_by_key(|candidate| candidate.horizon);
            let first = *matches.first()?;
            let movements = matches
                .iter()
                .map(|candidate| {
                    format!(
                        "{}거래일 {}",
                        candidate.horizon,
                        format_basis_points(candidate.return_basis_points)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "{} ({symbol}): {movements} · 분할조정 종가 기준",
                first.display_name
            ))
        })
        .collect();
    StockAiContent {
        headline: format!("{market_date} S&P 500 가격 변동 요약 · 투자 조언 아님"),
        bullets,
        notable_symbols: selection.notable_symbols,
    }
}

fn format_basis_points(value: i32) -> String {
    let sign = if value < 0 { "-" } else { "+" };
    let absolute = value.unsigned_abs();
    format!("{sign}{}.{:02}%", absolute / 100, absolute % 100)
}

fn stock_ai_schema(allowed_symbols: &BTreeSet<&str>) -> Value {
    let allowed_symbols = allowed_symbols.iter().copied().collect::<Vec<_>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "notableSymbols": {
                "type": "array",
                "minItems": 1,
                "maxItems": 6,
                "items": {
                    "type": "string",
                    "enum": allowed_symbols
                }
            }
        },
        "required": ["notableSymbols"]
    })
}

fn boolean_env(name: &str, default: bool) -> Result<bool, String> {
    match optional_env(name)? {
        None => Ok(default),
        Some(value) if value == "true" => Ok(true),
        Some(value) if value == "false" => Ok(false),
        Some(_) => Err(format!("{name} must be true or false")),
    }
}

fn secret_env(name: &str) -> Result<Option<String>, String> {
    optional_env(name)?
        .map(|value| {
            if value.len() > 2048 || value.contains(['\r', '\n', '\0']) {
                Err(format!("{name} has an invalid format"))
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must contain valid Unicode")),
    }
}

fn built_in_url(value: &str) -> Url {
    Url::parse(value).expect("the built-in stock provider URL must be valid")
}

#[cfg(test)]
fn validate_test_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|error| format!("invalid test URL: {error}"))?;
    if url.scheme() != "http"
        || !url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        })
    {
        return Err("test URL must use loopback HTTP".to_owned());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_STOCK_OPENAI_MODEL, MAX_RESPONSE_BYTES, StockAiCandidate, StockAiErrorKind,
        StockAiSelection, StockConfig, StockDataClient, StockDataError, deterministic_ai_content,
        normalize_symbol, parse_csv_records, parse_datahub_members, parse_decimal_microusd,
        response_bytes, run_ai_summary, validate_ai_selection,
    };
    use crate::openai::{OpenAiClient, OpenAiConfig, OpenAiError};
    use axum::{
        Json, Router,
        body::Body,
        extract::{Query, State},
        http::{HeaderMap, StatusCode},
        response::Response,
        routing::{get, post},
    };
    use chrono::{NaiveDate, TimeZone, Utc};
    use serde_json::{Value, json};
    use std::{
        collections::{BTreeSet, HashMap},
        time::Duration,
    };

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test address");
        let _server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve test provider");
        });
        format!("http://{address}")
    }

    fn candidate(
        symbol: &str,
        display_name: &str,
        horizon: u8,
        return_basis_points: i32,
    ) -> StockAiCandidate {
        StockAiCandidate {
            symbol: symbol.to_owned(),
            display_name: display_name.to_owned(),
            horizon,
            return_micros: i64::from(return_basis_points) * 100,
            return_basis_points,
            direction: if return_basis_points < 0 {
                "down".to_owned()
            } else {
                "up".to_owned()
            },
            band: "test-band".to_owned(),
        }
    }

    #[test]
    fn provider_symbol_limit_matches_core_and_schema() {
        assert_eq!(
            normalize_symbol("brk.a-1234").expect("ten-byte symbol"),
            "BRK.A-1234"
        );
        assert!(matches!(
            normalize_symbol("BRK.A-12345"),
            Err(StockDataError::InvalidResponse)
        ));
    }

    fn openai_client(origin: &str) -> OpenAiClient {
        let config = OpenAiConfig::for_test(
            Some("test-key"),
            DEFAULT_STOCK_OPENAI_MODEL,
            &format!("{origin}/v1/"),
            Duration::from_secs(5),
        )
        .expect("test OpenAI configuration");
        OpenAiClient::new(config).expect("test OpenAI client")
    }

    #[test]
    fn decimal_prices_convert_without_floating_point() {
        assert_eq!(parse_decimal_microusd("123.456789"), Some(123_456_789));
        assert_eq!(parse_decimal_microusd("1.2345678"), Some(1_234_568));
        assert_eq!(parse_decimal_microusd("1e2"), Some(100_000_000));
        assert_eq!(parse_decimal_microusd("0"), None);
        assert_eq!(parse_decimal_microusd("-1"), None);
    }

    #[test]
    fn ai_output_cannot_introduce_symbols() {
        let allowed = BTreeSet::from(["AAPL"]);
        let outside_selection = StockAiSelection {
            notable_symbols: vec!["MSFT".to_owned()],
        };
        assert!(validate_ai_selection(&outside_selection, &allowed).is_err());

        let duplicate_selection = StockAiSelection {
            notable_symbols: vec!["AAPL".to_owned(), "AAPL".to_owned()],
        };
        assert!(validate_ai_selection(&duplicate_selection, &allowed).is_err());

        let valid_selection = StockAiSelection {
            notable_symbols: vec!["AAPL".to_owned()],
        };
        assert!(validate_ai_selection(&valid_selection, &allowed).is_ok());
    }

    #[test]
    fn deterministic_ai_content_uses_only_server_values() {
        let candidates = vec![
            candidate("AAPL", "Apple", 5, 1_234),
            candidate("AAPL", "Apple", 21, -2_345),
        ];
        let selection = StockAiSelection {
            notable_symbols: vec!["AAPL".to_owned()],
        };
        let content = deterministic_ai_content(
            NaiveDate::from_ymd_opt(2026, 7, 24).expect("market date"),
            &candidates,
            selection,
        );
        let rendered = format!("{} {}", content.headline, content.bullets.join(" "));

        assert_eq!(content.notable_symbols, ["AAPL"]);
        assert!(rendered.contains("Apple (AAPL)"));
        assert!(rendered.contains("+12.34%"));
        assert!(rendered.contains("-23.45%"));
        assert!(!rendered.contains("MODEL_FREEFORM_SHOULD_NEVER_APPEAR"));
        assert!(
            serde_json::from_value::<StockAiSelection>(json!({
                "notableSymbols": ["AAPL"],
                "headline": "MODEL_FREEFORM_SHOULD_NEVER_APPEAR"
            }))
            .is_err()
        );
    }

    #[test]
    fn debug_output_redacts_provider_secrets() {
        let config = StockConfig::for_test(
            "http://127.0.0.1:9001/v2/",
            "http://127.0.0.1:9003/constituents.csv",
        );
        let debug = format!("{config:?}");
        assert!(!debug.contains("test-key"));
        assert!(!debug.contains("test-secret"));
        assert!(debug.contains("alpaca_configured"));
    }

    #[test]
    fn datahub_csv_supports_quoted_names_and_crlf() {
        let csv = b"Symbol,Security,GICS Sector\r\nBRK.B,\"Berkshire, Hathaway\",Financials\r\n";
        let members = parse_datahub_members(csv).expect("parse DataHub CSV");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].symbol, "BRK.B");
        assert_eq!(members[0].security, "Berkshire, Hathaway");
        assert_eq!(
            parse_csv_records("a,\"b\"\"c\"\n").expect("CSV")[0][1],
            "b\"c"
        );
    }

    #[tokio::test]
    async fn alpaca_bars_use_delayed_sip_split_and_follow_pagination() {
        async fn bars(
            headers: HeaderMap,
            Query(query): Query<HashMap<String, String>>,
        ) -> Result<Json<Value>, StatusCode> {
            if headers
                .get("APCA-API-KEY-ID")
                .and_then(|value| value.to_str().ok())
                != Some("test-key")
                || headers
                    .get("APCA-API-SECRET-KEY")
                    .and_then(|value| value.to_str().ok())
                    != Some("test-secret")
                || query.get("feed").map(String::as_str) != Some("sip")
                || query.get("adjustment").map(String::as_str) != Some("split")
                || query.get("currency").map(String::as_str) != Some("USD")
                || query.get("limit").map(String::as_str) != Some("10000")
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            let second = query.get("page_token").is_some();
            let offset = if second { 11 } else { 0 };
            let dates = (0..11)
                .map(|index| format!("2026-06-{:02}T20:00:00Z", offset + index + 1))
                .collect::<Vec<_>>();
            let bars = |close: u64| {
                dates
                    .iter()
                    .map(|timestamp| json!({"t": timestamp, "c": close}))
                    .collect::<Vec<_>>()
            };
            Ok(Json(json!({
                "bars": {
                    "AAPL": bars(200),
                    "SPY": bars(600)
                },
                "next_page_token": if second { Value::Null } else { json!("page-2") }
            })))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test address");
        let router = Router::new().route("/v2/stocks/bars", get(bars));
        let _server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve test provider");
        });
        let origin = format!("http://{address}");
        let config = StockConfig::for_test(
            &format!("{origin}/v2/"),
            &format!("{origin}/constituents.csv"),
        );
        let client = StockDataClient::new(config).expect("stock client");
        let download = client
            .fetch_daily_bars(
                &["AAPL".to_owned()],
                Utc.with_ymd_and_hms(2026, 7, 25, 2, 0, 0)
                    .single()
                    .expect("as of"),
            )
            .await
            .expect("download bars");
        assert_eq!(download.sessions.len(), 22);
        assert_eq!(download.bars.len(), 22);
        assert_eq!(download.feed, "sip");
        assert_eq!(download.adjustment, "split");
        assert_eq!(download.source_sha256.len(), 64);
    }

    #[tokio::test]
    async fn alpaca_bars_reject_malformed_and_out_of_range_timestamps() {
        async fn bars(State(timestamp): State<String>) -> Json<Value> {
            Json(json!({
                "bars": {
                    "AAPL": [{"t": timestamp, "c": 200}]
                },
                "next_page_token": Value::Null
            }))
        }

        let as_of = Utc
            .with_ymd_and_hms(2026, 7, 25, 2, 0, 0)
            .single()
            .expect("as of");
        for timestamp in [
            "2026-06-15T20:00:00Ztrailing",
            "2026-01-01T20:00:00Z",
            "2026-07-26T20:00:00Z",
        ] {
            let router = Router::new()
                .route("/v2/stocks/bars", get(bars))
                .with_state(timestamp.to_owned());
            let origin = serve(router).await;
            let config = StockConfig::for_test(
                &format!("{origin}/v2/"),
                &format!("{origin}/constituents.csv"),
            );
            let client = StockDataClient::new(config).expect("stock client");
            let result = client.fetch_daily_bars(&["AAPL".to_owned()], as_of).await;

            assert!(
                matches!(result, Err(StockDataError::InvalidResponse)),
                "unexpected result for timestamp {timestamp}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn alpaca_5xx_and_transport_failure_fail_closed() {
        async fn upstream_failure() -> StatusCode {
            StatusCode::INTERNAL_SERVER_ERROR
        }

        let origin = serve(Router::new().route("/v2/stocks/bars", get(upstream_failure))).await;
        let config = StockConfig::for_test(
            &format!("{origin}/v2/"),
            &format!("{origin}/constituents.csv"),
        );
        let client = StockDataClient::new(config).expect("stock client");
        let as_of = Utc
            .with_ymd_and_hms(2026, 7, 25, 20, 0, 0)
            .single()
            .expect("as of");
        assert!(matches!(
            client.fetch_daily_bars(&["AAPL".to_owned()], as_of).await,
            Err(StockDataError::UpstreamUnavailable)
        ));

        let closed_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind closed test port");
        let closed_address = closed_listener.local_addr().expect("closed test address");
        drop(closed_listener);
        let closed_origin = format!("http://{closed_address}");
        let config = StockConfig::for_test(
            &format!("{closed_origin}/v2/"),
            &format!("{closed_origin}/constituents.csv"),
        );
        let client = StockDataClient::new(config).expect("stock client");
        assert!(matches!(
            client.fetch_daily_bars(&["AAPL".to_owned()], as_of).await,
            Err(StockDataError::UpstreamUnavailable)
        ));
    }

    #[tokio::test]
    async fn chunked_response_over_limit_is_rejected_without_content_length() {
        async fn oversized_stream() -> Response {
            let fixed_body = Body::from(vec![b'x'; MAX_RESPONSE_BYTES + 1]);
            Response::builder()
                .status(StatusCode::OK)
                .body(Body::from_stream(fixed_body.into_data_stream()))
                .expect("streaming response")
        }

        let origin = serve(Router::new().route("/large", get(oversized_stream))).await;
        let response = reqwest::Client::new()
            .get(format!("{origin}/large"))
            .send()
            .await
            .expect("download streaming response");
        assert_eq!(response.content_length(), None);
        assert!(matches!(
            response_bytes(response).await,
            Err(StockDataError::ResponseTooLarge)
        ));
    }

    #[tokio::test]
    async fn openai_model_mismatch_is_possibly_billed() {
        async fn mismatched_model() -> Json<Value> {
            Json(json!({
                "id": "resp_model_mismatch",
                "status": "completed",
                "model": "gpt-5.4-nano-unexpected",
                "output": [{
                    "content": [{
                        "type": "output_text",
                        "text": "{\"notableSymbols\":[\"AAPL\"]}"
                    }]
                }]
            }))
        }

        let origin = serve(Router::new().route("/v1/responses", post(mismatched_model))).await;
        let error = run_ai_summary(
            openai_client(&origin),
            DEFAULT_STOCK_OPENAI_MODEL,
            NaiveDate::from_ymd_opt(2026, 7, 24).expect("market date"),
            &[candidate("AAPL", "Apple", 5, 1_234)],
        )
        .await
        .expect_err("model mismatch must fail closed");

        assert!(error.possibly_billed);
        assert!(matches!(error.kind, StockAiErrorKind::InvalidResponse));
    }

    #[tokio::test]
    async fn openai_5xx_is_possibly_billed() {
        async fn upstream_failure() -> Response {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("x-request-id", "req_test_500")
                .body(Body::from("upstream failure"))
                .expect("upstream failure response")
        }

        let origin = serve(Router::new().route("/v1/responses", post(upstream_failure))).await;
        let error = run_ai_summary(
            openai_client(&origin),
            DEFAULT_STOCK_OPENAI_MODEL,
            NaiveDate::from_ymd_opt(2026, 7, 24).expect("market date"),
            &[candidate("AAPL", "Apple", 5, 1_234)],
        )
        .await
        .expect_err("5xx must fail");

        assert!(error.possibly_billed);
        assert_eq!(error.upstream_request_id.as_deref(), Some("req_test_500"));
        assert!(matches!(
            error.kind,
            StockAiErrorKind::OpenAi(OpenAiError::UpstreamUnavailable {
                upstream_request_id: Some(request_id)
            }) if request_id == "req_test_500"
        ));
    }

    #[tokio::test]
    async fn openai_invalid_json_preserves_request_id_and_uses_default_service_tier() {
        async fn malformed(Json(request): Json<Value>) -> Response {
            assert_eq!(
                request.get("service_tier").and_then(Value::as_str),
                Some("default")
            );
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .header("x-request-id", "req_test_invalid_json")
                .body(Body::from("{"))
                .expect("malformed upstream response")
        }

        let origin = serve(Router::new().route("/v1/responses", post(malformed))).await;
        let error = run_ai_summary(
            openai_client(&origin),
            DEFAULT_STOCK_OPENAI_MODEL,
            NaiveDate::from_ymd_opt(2026, 7, 24).expect("market date"),
            &[candidate("AAPL", "Apple", 5, 1_234)],
        )
        .await
        .expect_err("malformed response must fail");

        assert!(error.possibly_billed);
        assert_eq!(
            error.upstream_request_id.as_deref(),
            Some("req_test_invalid_json")
        );
        assert!(matches!(
            error.kind,
            StockAiErrorKind::OpenAi(OpenAiError::InvalidResponse {
                upstream_request_id: Some(request_id)
            }) if request_id == "req_test_invalid_json"
        ));
    }
}
