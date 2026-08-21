use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    str::FromStr,
};

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::America::New_York;
use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::{
    AiOperationBudgetStatus, Error, Result, TmCore,
    database::{new_id, now_utc},
    error::{invalid, not_found},
};

pub const STOCK_SCREEN_MIN_COVERAGE_BPS: u32 = 9_800;
pub const STOCK_SCREEN_MAX_MEMBERS: usize = 1_000;
pub const STOCK_SCREEN_MAX_PAGE_SIZE: u32 = 50;
pub const STOCK_SCREEN_BAR_RETENTION_DAYS: i64 = 120;
pub const STOCK_AI_OPERATION: &str = "stock_daily_report";
pub const STOCK_AI_MONTHLY_HARD_LIMIT_MICROUSD: u64 = 2_000_000;
pub const STOCK_AI_MAXIMUM_COST_MICROUSD: u64 = 10_000;
const RETURN_SCALE: i128 = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockUniverseMemberInput {
    pub ticker: String,
    pub display_name: String,
    pub sector: Option<String>,
    pub sub_industry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreStockUniverseInput {
    pub name: String,
    pub effective_date: NaiveDate,
    pub source_url: String,
    pub source_revision: String,
    pub source_sha256: String,
    pub license_name: String,
    pub license_url: String,
    pub attribution_text: String,
    pub members: Vec<StockUniverseMemberInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StockUniverseSnapshot {
    pub id: String,
    pub name: String,
    pub effective_date: NaiveDate,
    pub source_url: String,
    pub source_revision: String,
    pub source_sha256: String,
    pub license_name: String,
    pub license_url: String,
    pub attribution_text: String,
    pub member_count: u32,
    pub fetched_at: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockDailyBarInput {
    pub ticker: String,
    pub session_date: NaiveDate,
    pub close_microusd: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreStockMarketDataInput {
    pub source: String,
    pub feed: String,
    pub adjustment: String,
    pub source_sha256: String,
    pub sessions: Vec<NaiveDate>,
    pub bars: Vec<StockDailyBarInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoreStockMarketDataResult {
    pub session_count: u32,
    pub bar_count: u32,
    pub earliest_session: NaiveDate,
    pub latest_session: NaiveDate,
    pub source_sha256: String,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockScreenStartInput {
    #[serde(default)]
    pub run_id: Option<String>,
    pub market_date: NaiveDate,
    pub universe_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenRun {
    pub id: String,
    pub market_date: NaiveDate,
    pub universe_snapshot_id: Option<String>,
    pub status: String,
    pub total_members: u32,
    pub current_covered: u32,
    pub baseline_5_covered: u32,
    pub baseline_21_covered: u32,
    pub result_count: u32,
    pub universe_sha256: Option<String>,
    pub market_data_sha256: Option<String>,
    pub failure_code: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StockScreenBand {
    Down20Plus,
    Down10To20,
    Up10To20,
    Up20Plus,
}

impl StockScreenBand {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Down20Plus => "down_20_plus",
            Self::Down10To20 => "down_10_to_20",
            Self::Up10To20 => "up_10_to_20",
            Self::Up20Plus => "up_20_plus",
        }
    }

    #[must_use]
    pub const fn direction(self) -> StockScreenDirection {
        match self {
            Self::Down20Plus | Self::Down10To20 => StockScreenDirection::Down,
            Self::Up10To20 | Self::Up20Plus => StockScreenDirection::Up,
        }
    }

    #[must_use]
    pub const fn filter(self) -> StockScreenBandFilter {
        match self {
            Self::Down20Plus | Self::Up20Plus => StockScreenBandFilter::TwentyPlus,
            Self::Down10To20 | Self::Up10To20 => StockScreenBandFilter::TenToTwenty,
        }
    }
}

impl FromStr for StockScreenBand {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "down_20_plus" => Ok(Self::Down20Plus),
            "down_10_to_20" => Ok(Self::Down10To20),
            "up_10_to_20" => Ok(Self::Up10To20),
            "up_20_plus" => Ok(Self::Up20Plus),
            other => Err(Error::Invariant(format!(
                "unsupported stock screen band: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StockScreenDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockScreenHorizon {
    Five,
    TwentyOne,
}

impl StockScreenHorizon {
    #[must_use]
    pub const fn sessions(self) -> u8 {
        match self {
            Self::Five => 5,
            Self::TwentyOne => 21,
        }
    }
}

impl Serialize for StockScreenHorizon {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(self.sessions())
    }
}

impl<'de> Deserialize<'de> for StockScreenHorizon {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            5 => Ok(Self::Five),
            21 => Ok(Self::TwentyOne),
            value => Err(serde::de::Error::custom(format!(
                "stock screen horizon must be 5 or 21, found {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StockScreenBandFilter {
    TenToTwenty,
    TwentyPlus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListStockScreenResultsInput {
    pub run_id: String,
    pub horizon: StockScreenHorizon,
    pub direction: StockScreenDirection,
    pub band: Option<StockScreenBandFilter>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenListItem {
    pub run_id: String,
    pub ticker: String,
    pub display_name: String,
    pub sector: Option<String>,
    pub horizon: StockScreenHorizon,
    pub direction: StockScreenDirection,
    pub band: StockScreenBand,
    pub current_date: NaiveDate,
    pub current_close_microusd: u64,
    pub baseline_date: NaiveDate,
    pub baseline_close_microusd: u64,
    pub return_micros: i64,
    pub return_pct: f64,
    pub universe_sha256: String,
    pub market_data_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenResultPage {
    pub items: Vec<StockScreenListItem>,
    pub next_cursor: Option<String>,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenCoverage {
    pub current_covered: u32,
    pub total: u32,
    pub current_pct: f64,
    pub baseline_5_covered: u32,
    pub baseline_5_pct: f64,
    pub baseline_21_covered: u32,
    pub baseline_21_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenUniverseSummary {
    pub name: String,
    pub source_url: String,
    pub revision: String,
    pub as_of_date: NaiveDate,
    pub member_count: u32,
    pub age_days: u32,
    pub attribution_text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenBucketCounts {
    pub up_5_ten_to_twenty: u32,
    pub up_5_twenty_plus: u32,
    pub down_5_ten_to_twenty: u32,
    pub down_5_twenty_plus: u32,
    pub up_21_ten_to_twenty: u32,
    pub up_21_twenty_plus: u32,
    pub down_21_ten_to_twenty: u32,
    pub down_21_twenty_plus: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockAiReport {
    pub id: String,
    pub screen_run_id: String,
    pub status: String,
    pub prompt_version: String,
    pub model: String,
    pub response_id: Option<String>,
    pub upstream_request_id: Option<String>,
    pub result: Option<Value>,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub estimated_cost_microusd: u64,
    pub failure_code: Option<String>,
    pub request_started_at: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenSuccess {
    pub run_id: String,
    pub market_date: NaiveDate,
    pub completed_at: String,
    pub coverage: StockScreenCoverage,
    pub universe: StockScreenUniverseSummary,
    pub counts: StockScreenBucketCounts,
    pub top3: Vec<StockScreenListItem>,
    pub ai: Option<StockAiReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockScreenAttemptSummary {
    pub run_id: String,
    pub market_date: NaiveDate,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub failure_code: Option<String>,
    pub coverage: StockScreenCoverage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LatestStockScreen {
    pub latest_success: Option<StockScreenSuccess>,
    pub latest_attempt: Option<StockScreenAttemptSummary>,
    pub stale: bool,
    pub staleness_reason: Option<String>,
    pub ai_budget: AiOperationBudgetStatus,
}

#[derive(Debug, Clone)]
pub struct StockAiReportStart<'a> {
    pub id: &'a str,
    pub screen_run_id: &'a str,
    pub prompt_version: &'a str,
    pub model: &'a str,
}

#[derive(Debug, Clone)]
pub struct StockAiReportCompletion {
    pub status: &'static str,
    pub response_id: Option<String>,
    pub upstream_request_id: Option<String>,
    pub result: Option<Value>,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub estimated_cost_microusd: u64,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone)]
struct StoredScreenResult {
    run_id: String,
    ticker: String,
    display_name: String,
    sector: Option<String>,
    current_date: NaiveDate,
    current_close_microusd: u64,
    baseline_5_date: Option<NaiveDate>,
    baseline_5_close_microusd: Option<u64>,
    return_5_micros: Option<i64>,
    band_5: Option<StockScreenBand>,
    baseline_21_date: Option<NaiveDate>,
    baseline_21_close_microusd: Option<u64>,
    return_21_micros: Option<i64>,
    band_21: Option<StockScreenBand>,
    universe_sha256: String,
    market_data_sha256: String,
}

#[derive(Debug, Clone)]
struct StoredMember {
    ticker: String,
    display_name: String,
    sector: Option<String>,
}

impl TmCore {
    pub fn store_stock_universe(
        &self,
        input: StoreStockUniverseInput,
    ) -> Result<StockUniverseSnapshot> {
        let name = clean_text("stock universe name", &input.name, 80)?;
        let source_url = validate_https_url("stock universe source URL", &input.source_url)?;
        let source_revision = clean_text(
            "stock universe source revision",
            &input.source_revision,
            160,
        )?;
        let source_sha256 = normalize_sha256(&input.source_sha256)?;
        let license_name = clean_text("stock universe license", &input.license_name, 160)?;
        let license_url = validate_https_url("stock universe license URL", &input.license_url)?;
        let attribution_text =
            clean_text("stock universe attribution", &input.attribution_text, 500)?;
        if input.members.is_empty() || input.members.len() > STOCK_SCREEN_MAX_MEMBERS {
            return Err(invalid(format!(
                "stock universe must contain 1-{STOCK_SCREEN_MAX_MEMBERS} members"
            )));
        }
        let mut members = BTreeMap::new();
        for member in input.members {
            let ticker = normalize_us_ticker(&member.ticker)?;
            let normalized = StockUniverseMemberInput {
                ticker: ticker.clone(),
                display_name: clean_text("stock universe display name", &member.display_name, 160)?,
                sector: clean_optional_text("stock universe sector", member.sector, 160)?,
                sub_industry: clean_optional_text(
                    "stock universe sub-industry",
                    member.sub_industry,
                    200,
                )?,
            };
            if members.insert(ticker.clone(), normalized).is_some() {
                return Err(invalid(format!(
                    "stock universe contains duplicate ticker: {ticker}"
                )));
            }
        }
        let member_count = u32::try_from(members.len())
            .map_err(|_| invalid("stock universe member count is too large"))?;
        let now = now_utc();
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some(existing) = transaction
                    .query_row(
                        "SELECT id, name, effective_date, source_url, source_revision,
                                source_sha256, license_name, license_url, attribution_text,
                                member_count, fetched_at, created_at
                         FROM stock_universe_snapshots
                         WHERE source_url = ?1 AND effective_date = ?2 AND source_sha256 = ?3",
                        params![source_url, input.effective_date, source_sha256],
                        map_universe_snapshot,
                    )
                    .optional()?
                {
                    if existing.member_count != member_count {
                        return Err(Error::Invariant(
                            "stock universe snapshot member count does not match".to_owned(),
                        ));
                    }
                    return Ok(existing);
                }
                let id = new_id();
                transaction.execute(
                    "INSERT INTO stock_universe_snapshots(
                        id, name, effective_date, source_url, source_revision,
                        source_sha256, license_name, license_url, attribution_text,
                        member_count, fetched_at, created_at
                     ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11
                     )",
                    params![
                        id,
                        name,
                        input.effective_date,
                        source_url,
                        source_revision,
                        source_sha256,
                        license_name,
                        license_url,
                        attribution_text,
                        member_count,
                        now,
                    ],
                )?;
                for member in members.values() {
                    transaction.execute(
                        "INSERT INTO stock_universe_members(
                            snapshot_id, ticker, display_name, sector, sub_industry, created_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            id,
                            member.ticker,
                            member.display_name,
                            member.sector,
                            member.sub_industry,
                            now,
                        ],
                    )?;
                }
                query_universe_snapshot(transaction, &id)
            })
    }

    pub fn latest_stock_universe(&self) -> Result<Option<StockUniverseSnapshot>> {
        self.database
            .connect()?
            .query_row(
                "SELECT id, name, effective_date, source_url, source_revision,
                        source_sha256, license_name, license_url, attribution_text,
                        member_count, fetched_at, created_at
                 FROM stock_universe_snapshots
                 ORDER BY effective_date DESC, fetched_at DESC, id DESC LIMIT 1",
                [],
                map_universe_snapshot,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_stock_universe_snapshot(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<StockUniverseSnapshot>> {
        let snapshot_id = clean_text("stock universe snapshot ID", snapshot_id, 128)?;
        let connection = self.database.connect()?;
        query_universe_snapshot_optional(&connection, &snapshot_id)
    }

    pub fn stock_universe_members(
        &self,
        snapshot_id: &str,
    ) -> Result<Vec<StockUniverseMemberInput>> {
        let connection = self.database.connect()?;
        query_universe_snapshot(&connection, snapshot_id)?;
        let mut statement = connection.prepare(
            "SELECT ticker, display_name, sector, sub_industry
             FROM stock_universe_members
             WHERE snapshot_id = ?1 ORDER BY ticker",
        )?;
        let rows = statement.query_map([snapshot_id], |row| {
            Ok(StockUniverseMemberInput {
                ticker: row.get(0)?,
                display_name: row.get(1)?,
                sector: row.get(2)?,
                sub_industry: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn store_stock_market_data(
        &self,
        input: StoreStockMarketDataInput,
    ) -> Result<StoreStockMarketDataResult> {
        let source = clean_text("stock market data source", &input.source, 80)?;
        let feed = clean_text("stock market data feed", &input.feed, 40)?;
        if input.adjustment != "split" {
            return Err(invalid("stock market data adjustment must be split"));
        }
        let source_sha256 = normalize_sha256(&input.source_sha256)?;
        if input.sessions.is_empty() || input.sessions.len() > 366 {
            return Err(invalid(
                "stock market data must contain 1-366 provider-confirmed sessions",
            ));
        }
        if input.bars.len() > 50_000 {
            return Err(invalid("stock market data cannot exceed 50000 bars"));
        }
        let session_input_count = input.sessions.len();
        let sessions = input.sessions.into_iter().collect::<BTreeSet<_>>();
        if sessions.is_empty() {
            return Err(invalid("stock market data contains no sessions"));
        }
        if sessions.len() != session_input_count {
            return Err(invalid(
                "stock market data contains duplicate provider-confirmed sessions",
            ));
        }
        let mut bars = BTreeMap::new();
        for bar in input.bars {
            if !sessions.contains(&bar.session_date) {
                return Err(invalid(format!(
                    "stock bar date is not a provider-confirmed session: {}",
                    bar.session_date
                )));
            }
            let ticker = normalize_us_ticker(&bar.ticker)?;
            let close_microusd = i64::try_from(bar.close_microusd)
                .map_err(|_| invalid("stock close price is too large"))?;
            if close_microusd <= 0 {
                return Err(invalid("stock close price must be positive"));
            }
            let key = (bar.session_date, ticker.clone());
            if bars.insert(key.clone(), close_microusd).is_some() {
                return Err(invalid(format!("duplicate stock bar: {} {}", key.1, key.0)));
            }
        }
        let mut session_counts = sessions
            .iter()
            .map(|date| (*date, 0_u32))
            .collect::<BTreeMap<_, _>>();
        for (session_date, _) in bars.keys() {
            *session_counts.entry(*session_date).or_default() += 1;
        }
        let earliest_session = *session_counts
            .keys()
            .next()
            .ok_or_else(|| invalid("stock market data contains no sessions"))?;
        let latest_session = *session_counts
            .keys()
            .next_back()
            .ok_or_else(|| invalid("stock market data contains no sessions"))?;
        let session_count = u32::try_from(session_counts.len())
            .map_err(|_| invalid("stock market session count is too large"))?;
        let bar_count = u32::try_from(bars.len())
            .map_err(|_| invalid("stock market bar count is too large"))?;
        let cache_fetched_at = now_utc();
        let batch_fetched_at =
            self.database
                .transaction(TransactionBehavior::Immediate, |transaction| {
                    let existing_batch = transaction
                        .query_row(
                            "SELECT source, feed, adjustment, session_count, bar_count,
                                earliest_session, latest_session, fetched_at
                         FROM stock_market_data_batches
                         WHERE source_sha256 = ?1",
                            [&source_sha256],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, u32>(3)?,
                                    row.get::<_, u32>(4)?,
                                    row.get::<_, NaiveDate>(5)?,
                                    row.get::<_, NaiveDate>(6)?,
                                    row.get::<_, String>(7)?,
                                ))
                            },
                        )
                        .optional()?;
                    let batch_fetched_at = if let Some((
                        saved_source,
                        saved_feed,
                        saved_adjustment,
                        saved_session_count,
                        saved_bar_count,
                        saved_earliest_session,
                        saved_latest_session,
                        saved_fetched_at,
                    )) = existing_batch
                    {
                        if saved_source != source
                            || saved_feed != feed
                            || saved_adjustment != "split"
                            || saved_session_count != session_count
                            || saved_bar_count != bar_count
                            || saved_earliest_session != earliest_session
                            || saved_latest_session != latest_session
                        {
                            return Err(Error::Conflict(
                            "stock market data hash was already used with different batch metadata"
                                .to_owned(),
                        ));
                        }
                        saved_fetched_at
                    } else {
                        transaction.execute(
                            "INSERT INTO stock_market_data_batches(
                            source_sha256, source, feed, adjustment, session_count,
                            bar_count, earliest_session, latest_session, fetched_at,
                            created_at
                         ) VALUES (?1, ?2, ?3, 'split', ?4, ?5, ?6, ?7, ?8, ?8)",
                            params![
                                source_sha256,
                                source,
                                feed,
                                session_count,
                                bar_count,
                                earliest_session,
                                latest_session,
                                cache_fetched_at,
                            ],
                        )?;
                        cache_fetched_at.clone()
                    };
                    for (session_date, bar_count) in &session_counts {
                        transaction.execute(
                            "INSERT INTO stock_market_sessions(
                            session_date, source, feed, adjustment, source_sha256,
                            bar_count, fetched_at, created_at, updated_at
                         ) VALUES (?1, ?2, ?3, 'split', ?4, ?5, ?6, ?6, ?6)
                         ON CONFLICT(session_date) DO UPDATE SET
                            source = excluded.source,
                            feed = excluded.feed,
                            adjustment = excluded.adjustment,
                            source_sha256 = excluded.source_sha256,
                            bar_count = excluded.bar_count,
                            fetched_at = excluded.fetched_at,
                            updated_at = excluded.updated_at",
                            params![
                                session_date,
                                source,
                                feed,
                                source_sha256,
                                bar_count,
                                cache_fetched_at
                            ],
                        )?;
                        transaction.execute(
                            "DELETE FROM stock_daily_bars WHERE session_date = ?1",
                            [session_date],
                        )?;
                    }
                    for ((session_date, ticker), close_microusd) in &bars {
                        transaction.execute(
                            "INSERT INTO stock_daily_bars(
                            ticker, session_date, close_microusd, source_sha256,
                            fetched_at, created_at, updated_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?5)
                         ON CONFLICT(ticker, session_date) DO UPDATE SET
                            close_microusd = excluded.close_microusd,
                            source_sha256 = excluded.source_sha256,
                            fetched_at = excluded.fetched_at,
                            updated_at = excluded.updated_at",
                            params![
                                ticker,
                                session_date,
                                close_microusd,
                                source_sha256,
                                cache_fetched_at
                            ],
                        )?;
                    }
                    Ok(batch_fetched_at)
                })?;
        Ok(StoreStockMarketDataResult {
            session_count,
            bar_count,
            earliest_session,
            latest_session,
            source_sha256,
            fetched_at: batch_fetched_at,
        })
    }

    pub fn prune_stock_market_data(&self, retain_from: NaiveDate) -> Result<u64> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let changed = transaction.execute(
                    "DELETE FROM stock_daily_bars WHERE session_date < ?1",
                    [retain_from],
                )?;
                transaction.execute(
                    "DELETE FROM stock_market_sessions
                     WHERE session_date < ?1
                       AND NOT EXISTS(
                           SELECT 1 FROM stock_daily_bars
                           WHERE stock_daily_bars.session_date =
                                 stock_market_sessions.session_date
                       )",
                    [retain_from],
                )?;
                u64::try_from(changed)
                    .map_err(|_| Error::Invariant("deleted bar count is negative".to_owned()))
            })
    }

    pub fn begin_stock_screen_run(&self, input: StockScreenStartInput) -> Result<StockScreenRun> {
        let id = input
            .run_id
            .as_deref()
            .map(|value| clean_text("stock screen run ID", value, 128))
            .transpose()?
            .unwrap_or_else(new_id);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some(snapshot_id) = input.universe_snapshot_id.as_deref() {
                    query_universe_snapshot(transaction, snapshot_id)?;
                }
                if let Some(existing) = transaction
                    .query_row(
                        &format!("{SCREEN_RUN_SELECT} WHERE id = ?1"),
                        [&id],
                        map_screen_run,
                    )
                    .optional()?
                {
                    if existing.market_date == input.market_date
                        && input
                            .universe_snapshot_id
                            .as_deref()
                            .is_none_or(|expected| {
                                existing.universe_snapshot_id.as_deref() == Some(expected)
                            })
                    {
                        return Ok(existing);
                    }
                    return Err(Error::Conflict(
                        "stock screen run ID was already used with different input".to_owned(),
                    ));
                }
                let now = now_utc();
                transaction.execute(
                    "INSERT INTO stock_screen_runs(
                        id, market_date, universe_snapshot_id, status, started_at, created_at
                     ) VALUES (?1, ?2, ?3, 'started', ?4, ?4)",
                    params![id, input.market_date, input.universe_snapshot_id, now],
                )?;
                query_screen_run(transaction, &id)
            })
    }

    pub fn get_stock_screen_run(&self, run_id: &str) -> Result<Option<StockScreenRun>> {
        let run_id = clean_text("stock screen run ID", run_id, 128)?;
        let connection = self.database.connect()?;
        query_screen_run_optional(&connection, &run_id)
    }

    pub fn fail_stock_screen_run(
        &self,
        run_id: &str,
        status: &str,
        failure_code: &str,
    ) -> Result<StockScreenRun> {
        if !matches!(status, "upstream_unavailable" | "failed") {
            return Err(invalid("stock screen failure status is invalid"));
        }
        let failure_code = clean_text("stock screen failure code", failure_code, 120)?;
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE stock_screen_runs
             SET status = ?2, failure_code = ?3, completed_at = ?4
             WHERE id = ?1 AND status = 'started'",
            params![run_id, status, failure_code, now_utc()],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "stock screen run {run_id} was not in started state"
            )));
        }
        query_screen_run(&connection, run_id)
    }

    pub fn complete_stock_screen_run(
        &self,
        run_id: &str,
        universe_snapshot_id: &str,
        market_data_sha256: &str,
    ) -> Result<StockScreenRun> {
        let market_data_sha256 = normalize_sha256(market_data_sha256)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let run = query_screen_run(transaction, run_id)?;
                if run.status != "started" {
                    return Err(Error::Conflict(format!(
                        "stock screen run {run_id} was not in started state"
                    )));
                }
                let batch_exists: bool = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM stock_market_data_batches
                        WHERE source_sha256 = ?1
                     )",
                    [&market_data_sha256],
                    |row| row.get(0),
                )?;
                if !batch_exists {
                    return Err(not_found("stock market data batch", &market_data_sha256));
                }
                if let Some(started_snapshot) = run.universe_snapshot_id.as_deref()
                    && started_snapshot != universe_snapshot_id
                {
                    return Err(Error::Conflict(
                        "stock screen universe changed after the run started".to_owned(),
                    ));
                }
                let snapshot = query_universe_snapshot(transaction, universe_snapshot_id)?;
                if snapshot.effective_date > run.market_date {
                    return Err(invalid(
                        "stock universe effective date is later than the market date",
                    ));
                }
                let members = query_universe_members(transaction, universe_snapshot_id)?;
                let total = u32::try_from(members.len())
                    .map_err(|_| invalid("stock universe member count is too large"))?;
                if total != snapshot.member_count {
                    return Err(Error::Invariant(
                        "stock universe snapshot and member count differ".to_owned(),
                    ));
                }
                let sessions =
                    query_screen_sessions(transaction, run.market_date, &market_data_sha256)?;
                if sessions.len() < 22 {
                    finish_coverage_failure(
                        transaction,
                        run_id,
                        universe_snapshot_id,
                        total,
                        0,
                        0,
                        0,
                        &snapshot.source_sha256,
                        &market_data_sha256,
                        "insufficient_market_sessions",
                    )?;
                    return query_screen_run(transaction, run_id);
                }
                let current_date = sessions[0];
                if current_date != run.market_date {
                    finish_coverage_failure(
                        transaction,
                        run_id,
                        universe_snapshot_id,
                        total,
                        0,
                        0,
                        0,
                        &snapshot.source_sha256,
                        &market_data_sha256,
                        "market_date_not_available",
                    )?;
                    return query_screen_run(transaction, run_id);
                }
                let baseline_5_date = sessions[5];
                let baseline_21_date = sessions[21];
                let prices = query_prices(
                    transaction,
                    &[current_date, baseline_5_date, baseline_21_date],
                    &market_data_sha256,
                )?;
                let current_covered = coverage_count(&members, current_date, &prices)?;
                let baseline_5_covered = coverage_count(&members, baseline_5_date, &prices)?;
                let baseline_21_covered = coverage_count(&members, baseline_21_date, &prices)?;
                if !coverage_passes(current_covered, total)
                    || !coverage_passes(baseline_5_covered, total)
                    || !coverage_passes(baseline_21_covered, total)
                {
                    finish_coverage_failure(
                        transaction,
                        run_id,
                        universe_snapshot_id,
                        total,
                        current_covered,
                        baseline_5_covered,
                        baseline_21_covered,
                        &snapshot.source_sha256,
                        &market_data_sha256,
                        "coverage_below_98_percent",
                    )?;
                    return query_screen_run(transaction, run_id);
                }

                let now = now_utc();
                let mut result_count = 0_u32;
                for member in &members {
                    let current = prices.get(&(member.ticker.clone(), current_date)).copied();
                    let baseline_5 = prices
                        .get(&(member.ticker.clone(), baseline_5_date))
                        .copied();
                    let baseline_21 = prices
                        .get(&(member.ticker.clone(), baseline_21_date))
                        .copied();
                    let five = calculate_band(current, baseline_5)?;
                    let twenty_one = calculate_band(current, baseline_21)?;
                    if five.is_none() && twenty_one.is_none() {
                        continue;
                    }
                    let current = current.ok_or_else(|| {
                        Error::Invariant("candidate result is missing its current close".to_owned())
                    })?;
                    let (baseline_5_close, return_5, band_5) = unpack_calculation(baseline_5, five);
                    let (baseline_21_close, return_21, band_21) =
                        unpack_calculation(baseline_21, twenty_one);
                    transaction.execute(
                        "INSERT INTO stock_screen_results(
                            run_id, ticker, display_name, sector, current_date,
                            current_close_microusd, baseline_5_date,
                            baseline_5_close_microusd, return_5_micros, band_5,
                            baseline_21_date, baseline_21_close_microusd,
                            return_21_micros, band_21, universe_sha256,
                            market_data_sha256, created_at
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                            ?11, ?12, ?13, ?14, ?15, ?16, ?17
                         )",
                        params![
                            run_id,
                            member.ticker,
                            member.display_name,
                            member.sector,
                            current_date,
                            current,
                            band_5.map(|_| baseline_5_date),
                            baseline_5_close,
                            return_5,
                            band_5.map(StockScreenBand::as_str),
                            band_21.map(|_| baseline_21_date),
                            baseline_21_close,
                            return_21,
                            band_21.map(StockScreenBand::as_str),
                            snapshot.source_sha256,
                            market_data_sha256,
                            now,
                        ],
                    )?;
                    result_count += 1;
                }
                let status = if result_count == 0 {
                    "no_candidates"
                } else {
                    "succeeded"
                };
                let changed = transaction.execute(
                    "UPDATE stock_screen_runs
                     SET universe_snapshot_id = ?2, status = ?3, total_members = ?4,
                         current_covered = ?5, baseline_5_covered = ?6,
                         baseline_21_covered = ?7, result_count = ?8,
                         universe_sha256 = ?9, market_data_sha256 = ?10,
                         failure_code = NULL, completed_at = ?11
                     WHERE id = ?1 AND status = 'started'",
                    params![
                        run_id,
                        universe_snapshot_id,
                        status,
                        total,
                        current_covered,
                        baseline_5_covered,
                        baseline_21_covered,
                        result_count,
                        snapshot.source_sha256,
                        market_data_sha256,
                        now,
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "stock screen run changed during completion".to_owned(),
                    ));
                }
                query_screen_run(transaction, run_id)
            })
    }

    pub fn get_latest_stock_screen(&self) -> Result<LatestStockScreen> {
        let expected = self.latest_expected_stock_market_date(Utc::now())?;
        self.get_latest_stock_screen_as_of(expected)
    }

    pub fn latest_expected_stock_market_date(&self, as_of: DateTime<Utc>) -> Result<NaiveDate> {
        latest_expected_nyse_date(as_of)
    }

    pub fn latest_stock_market_session(&self) -> Result<Option<NaiveDate>> {
        self.database
            .connect()?
            .query_row(
                "SELECT max(session_date) FROM stock_market_sessions",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn get_latest_stock_screen_as_of(
        &self,
        expected_market_date: NaiveDate,
    ) -> Result<LatestStockScreen> {
        let connection = self.database.connect()?;
        let latest_attempt = connection
            .query_row(
                &format!(
                    "{SCREEN_RUN_SELECT}
                     ORDER BY market_date DESC, created_at DESC, id DESC LIMIT 1"
                ),
                [],
                map_screen_run,
            )
            .optional()?;
        let latest_success_run = connection
            .query_row(
                &format!(
                    "{SCREEN_RUN_SELECT}
                     WHERE status IN ('succeeded', 'no_candidates')
                     ORDER BY market_date DESC, completed_at DESC, id DESC LIMIT 1"
                ),
                [],
                map_screen_run,
            )
            .optional()?;
        let latest_success = latest_success_run
            .as_ref()
            .map(|run| build_success(&connection, run, expected_market_date))
            .transpose()?;
        let attempt_summary = latest_attempt.as_ref().map(attempt_summary);
        let (stale, staleness_reason) = match (&latest_success_run, &latest_attempt) {
            (None, _) => (true, Some("first_run_pending".to_owned())),
            (Some(success), Some(attempt))
                if attempt.id != success.id
                    && attempt.market_date >= success.market_date
                    && !matches!(attempt.status.as_str(), "succeeded" | "no_candidates") =>
            {
                (true, Some(format!("latest_attempt_{}", attempt.status)))
            }
            (Some(success), _) if success.market_date < expected_market_date => (
                true,
                Some("latest_success_before_expected_market_date".to_owned()),
            ),
            (Some(_), _) => (false, None),
        };
        drop(connection);
        let ai_budget = self
            .ai_operation_budget_status(STOCK_AI_OPERATION, STOCK_AI_MONTHLY_HARD_LIMIT_MICROUSD)?;
        Ok(LatestStockScreen {
            latest_success,
            latest_attempt: attempt_summary,
            stale,
            staleness_reason,
            ai_budget,
        })
    }

    pub fn list_stock_screen_results(
        &self,
        input: ListStockScreenResultsInput,
    ) -> Result<StockScreenResultPage> {
        if input.run_id.trim().is_empty() || input.run_id.len() > 128 {
            return Err(invalid("stock screen run ID has an invalid format"));
        }
        let limit = input.limit.unwrap_or(STOCK_SCREEN_MAX_PAGE_SIZE);
        if limit == 0 || limit > STOCK_SCREEN_MAX_PAGE_SIZE {
            return Err(invalid(format!(
                "stock screen result limit must be 1-{STOCK_SCREEN_MAX_PAGE_SIZE}"
            )));
        }
        let offset = input
            .cursor
            .as_deref()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| invalid("stock screen result cursor is invalid"))?;
        let connection = self.database.connect()?;
        query_screen_run(&connection, &input.run_id)?;
        let mut items = query_screen_results(&connection, &input.run_id)?
            .into_iter()
            .filter_map(|result| result.for_horizon(input.horizon))
            .filter(|result| result.direction == input.direction)
            .filter(|result| input.band.is_none_or(|band| result.band.filter() == band))
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .return_micros
                .unsigned_abs()
                .cmp(&left.return_micros.unsigned_abs())
                .then_with(|| left.ticker.cmp(&right.ticker))
        });
        let total = u32::try_from(items.len())
            .map_err(|_| invalid("stock screen result count is too large"))?;
        let page = items
            .into_iter()
            .skip(offset)
            .take(limit as usize)
            .collect::<Vec<_>>();
        let next_offset = offset.saturating_add(page.len());
        let next_cursor = (next_offset < total as usize).then(|| next_offset.to_string());
        Ok(StockScreenResultPage {
            items: page,
            next_cursor,
            total,
        })
    }

    pub fn begin_stock_ai_report(&self, input: StockAiReportStart<'_>) -> Result<StockAiReport> {
        let id = clean_text("stock AI report ID", input.id, 128)?;
        let prompt_version = clean_text("stock AI prompt version", input.prompt_version, 64)?;
        let model = clean_text("stock AI model", input.model, 128)?;
        let screen_run_id = clean_text("stock AI screen run ID", input.screen_run_id, 128)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let screen = query_screen_run(transaction, &screen_run_id)?;
                if screen.status != "succeeded" || screen.result_count == 0 {
                    return Err(Error::Conflict(
                        "stock AI report requires a successful non-empty screen".to_owned(),
                    ));
                }
                if let Some(existing) = query_ai_report_optional(transaction, &id)? {
                    if existing.screen_run_id == screen_run_id
                        && existing.prompt_version == prompt_version
                        && existing.model == model
                    {
                        return Ok(existing);
                    }
                    return Err(Error::Conflict(
                        "stock AI report ID was already used with different input".to_owned(),
                    ));
                }
                transaction.execute(
                    "INSERT INTO stock_ai_reports(
                        id, screen_run_id, status, prompt_version, model,
                        estimated_cost_microusd, created_at
                     ) VALUES (?1, ?2, 'started', ?3, ?4, 0, ?5)",
                    params![id, screen_run_id, prompt_version, model, now_utc()],
                )?;
                query_ai_report(transaction, &id)
            })
    }

    pub fn get_stock_ai_report(&self, id: &str) -> Result<Option<StockAiReport>> {
        let id = clean_text("stock AI report ID", id, 128)?;
        let connection = self.database.connect()?;
        query_ai_report_optional(&connection, &id)
    }

    pub fn mark_stock_ai_report_calling(&self, id: &str) -> Result<StockAiReport> {
        let id = clean_text("stock AI report ID", id, 128)?;
        self.try_mark_stock_ai_report_calling(&id)?;
        let connection = self.database.connect()?;
        query_ai_report(&connection, &id)
    }

    pub fn try_mark_stock_ai_report_calling(&self, id: &str) -> Result<bool> {
        let id = clean_text("stock AI report ID", id, 128)?;
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE stock_ai_reports
             SET request_started_at = ?2
             WHERE id = ?1 AND status = 'started' AND request_started_at IS NULL",
            params![id, now_utc()],
        )?;
        if changed == 1 {
            return Ok(true);
        }
        let report = query_ai_report(&connection, &id)?;
        if report.status != "started" {
            return Err(Error::Conflict(format!(
                "stock AI report {id} cannot enter calling state"
            )));
        }
        if report.request_started_at.is_some() {
            return Ok(false);
        }
        Err(Error::Conflict(
            "stock AI report calling marker changed concurrently".to_owned(),
        ))
    }

    pub fn complete_stock_ai_report(
        &self,
        id: &str,
        completion: &StockAiReportCompletion,
    ) -> Result<StockAiReport> {
        let id = clean_text("stock AI report ID", id, 128)?;
        if !matches!(completion.status, "succeeded" | "failed" | "ai_uncertain") {
            return Err(invalid("stock AI report completion status is invalid"));
        }
        if completion.status == "succeeded" && completion.result.is_none() {
            return Err(invalid("successful stock AI report requires a result"));
        }
        if completion.status != "succeeded" && completion.failure_code.is_none() {
            return Err(invalid(
                "unsuccessful stock AI report requires a failure code",
            ));
        }
        let failure_code = completion
            .failure_code
            .as_deref()
            .map(|value| clean_text("stock AI failure code", value, 120))
            .transpose()?;
        let result_json = completion
            .result
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE stock_ai_reports
             SET status = ?2, response_id = ?3, upstream_request_id = ?4,
                 result_json = ?5, input_tokens = ?6, cached_input_tokens = ?7,
                 output_tokens = ?8, total_tokens = ?9,
                 estimated_cost_microusd = ?10, failure_code = ?11,
                 completed_at = ?12
             WHERE id = ?1 AND status = 'started'",
            params![
                id,
                completion.status,
                completion.response_id,
                completion.upstream_request_id,
                result_json,
                completion.input_tokens,
                completion.cached_input_tokens,
                completion.output_tokens,
                completion.total_tokens,
                completion.estimated_cost_microusd,
                failure_code,
                now_utc(),
            ],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "stock AI report {id} was not in started state"
            )));
        }
        query_ai_report(&connection, &id)
    }
}

impl StoredScreenResult {
    fn for_horizon(&self, horizon: StockScreenHorizon) -> Option<StockScreenListItem> {
        let (baseline_date, baseline_close, return_micros, band) = match horizon {
            StockScreenHorizon::Five => (
                self.baseline_5_date?,
                self.baseline_5_close_microusd?,
                self.return_5_micros?,
                self.band_5?,
            ),
            StockScreenHorizon::TwentyOne => (
                self.baseline_21_date?,
                self.baseline_21_close_microusd?,
                self.return_21_micros?,
                self.band_21?,
            ),
        };
        Some(StockScreenListItem {
            run_id: self.run_id.clone(),
            ticker: self.ticker.clone(),
            display_name: self.display_name.clone(),
            sector: self.sector.clone(),
            horizon,
            direction: band.direction(),
            band,
            current_date: self.current_date,
            current_close_microusd: self.current_close_microusd,
            baseline_date,
            baseline_close_microusd: baseline_close,
            return_micros,
            return_pct: return_micros as f64 / 10_000.0,
            universe_sha256: self.universe_sha256.clone(),
            market_data_sha256: self.market_data_sha256.clone(),
        })
    }
}

fn build_success(
    connection: &rusqlite::Connection,
    run: &StockScreenRun,
    as_of: NaiveDate,
) -> Result<StockScreenSuccess> {
    let snapshot_id = run
        .universe_snapshot_id
        .as_deref()
        .ok_or_else(|| Error::Invariant("successful stock screen has no universe".to_owned()))?;
    let snapshot = query_universe_snapshot(connection, snapshot_id)?;
    let results = query_screen_results(connection, &run.id)?;
    let mut counts = StockScreenBucketCounts::default();
    let mut top = Vec::new();
    for result in &results {
        let mut best = None;
        for horizon in [StockScreenHorizon::Five, StockScreenHorizon::TwentyOne] {
            let Some(item) = result.for_horizon(horizon) else {
                continue;
            };
            increment_count(&mut counts, item.horizon, item.band);
            if best.as_ref().is_none_or(|current: &StockScreenListItem| {
                item.return_micros.unsigned_abs() > current.return_micros.unsigned_abs()
            }) {
                best = Some(item);
            }
        }
        if let Some(best) = best {
            top.push(best);
        }
    }
    top.sort_by_key(|item| {
        (
            Reverse(item.return_micros.unsigned_abs()),
            item.ticker.clone(),
            item.horizon.sessions(),
        )
    });
    top.truncate(3);
    let ai = latest_ai_report_for_run(connection, &run.id)?;
    let age_days = (as_of - snapshot.effective_date).num_days().max(0) as u32;
    Ok(StockScreenSuccess {
        run_id: run.id.clone(),
        market_date: run.market_date,
        completed_at: run
            .completed_at
            .clone()
            .ok_or_else(|| Error::Invariant("successful stock screen is incomplete".to_owned()))?,
        coverage: coverage(run),
        universe: StockScreenUniverseSummary {
            name: snapshot.name,
            source_url: snapshot.source_url,
            revision: snapshot.source_revision,
            as_of_date: snapshot.effective_date,
            member_count: snapshot.member_count,
            age_days,
            attribution_text: snapshot.attribution_text,
        },
        counts,
        top3: top,
        ai,
    })
}

fn increment_count(
    counts: &mut StockScreenBucketCounts,
    horizon: StockScreenHorizon,
    band: StockScreenBand,
) {
    match (horizon, band) {
        (StockScreenHorizon::Five, StockScreenBand::Up10To20) => {
            counts.up_5_ten_to_twenty += 1;
        }
        (StockScreenHorizon::Five, StockScreenBand::Up20Plus) => {
            counts.up_5_twenty_plus += 1;
        }
        (StockScreenHorizon::Five, StockScreenBand::Down10To20) => {
            counts.down_5_ten_to_twenty += 1;
        }
        (StockScreenHorizon::Five, StockScreenBand::Down20Plus) => {
            counts.down_5_twenty_plus += 1;
        }
        (StockScreenHorizon::TwentyOne, StockScreenBand::Up10To20) => {
            counts.up_21_ten_to_twenty += 1;
        }
        (StockScreenHorizon::TwentyOne, StockScreenBand::Up20Plus) => {
            counts.up_21_twenty_plus += 1;
        }
        (StockScreenHorizon::TwentyOne, StockScreenBand::Down10To20) => {
            counts.down_21_ten_to_twenty += 1;
        }
        (StockScreenHorizon::TwentyOne, StockScreenBand::Down20Plus) => {
            counts.down_21_twenty_plus += 1;
        }
    }
}

fn attempt_summary(run: &StockScreenRun) -> StockScreenAttemptSummary {
    StockScreenAttemptSummary {
        run_id: run.id.clone(),
        market_date: run.market_date,
        status: run.status.clone(),
        started_at: run.started_at.clone(),
        completed_at: run.completed_at.clone(),
        failure_code: run.failure_code.clone(),
        coverage: coverage(run),
    }
}

fn coverage(run: &StockScreenRun) -> StockScreenCoverage {
    StockScreenCoverage {
        current_covered: run.current_covered,
        total: run.total_members,
        current_pct: coverage_pct(run.current_covered, run.total_members),
        baseline_5_covered: run.baseline_5_covered,
        baseline_5_pct: coverage_pct(run.baseline_5_covered, run.total_members),
        baseline_21_covered: run.baseline_21_covered,
        baseline_21_pct: coverage_pct(run.baseline_21_covered, run.total_members),
    }
}

fn coverage_pct(covered: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(covered) * 100.0 / f64::from(total)
    }
}

fn coverage_passes(covered: u32, total: u32) -> bool {
    total > 0
        && u64::from(covered) * 10_000
            >= u64::from(total) * u64::from(STOCK_SCREEN_MIN_COVERAGE_BPS)
}

fn calculate_band(
    current: Option<i64>,
    baseline: Option<i64>,
) -> Result<Option<(i64, StockScreenBand)>> {
    let (Some(current), Some(baseline)) = (current, baseline) else {
        return Ok(None);
    };
    let delta = i128::from(current) - i128::from(baseline);
    let magnitude = delta.unsigned_abs();
    let baseline_u = u128::try_from(baseline)
        .map_err(|_| Error::Invariant("stock baseline close is not positive".to_owned()))?;
    let band = if delta >= 0 {
        if magnitude.saturating_mul(5) >= baseline_u {
            Some(StockScreenBand::Up20Plus)
        } else if magnitude.saturating_mul(10) >= baseline_u {
            Some(StockScreenBand::Up10To20)
        } else {
            None
        }
    } else if magnitude.saturating_mul(5) >= baseline_u {
        Some(StockScreenBand::Down20Plus)
    } else if magnitude.saturating_mul(10) >= baseline_u {
        Some(StockScreenBand::Down10To20)
    } else {
        None
    };
    let Some(band) = band else {
        return Ok(None);
    };
    Ok(Some((scaled_return_micros(current, baseline)?, band)))
}

fn scaled_return_micros(current: i64, baseline: i64) -> Result<i64> {
    if current <= 0 || baseline <= 0 {
        return Err(Error::Invariant(
            "stock prices must be positive during return calculation".to_owned(),
        ));
    }
    let numerator = (i128::from(current) - i128::from(baseline)) * RETURN_SCALE;
    let denominator = i128::from(baseline);
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        -((-numerator + denominator / 2) / denominator)
    };
    i64::try_from(rounded)
        .map_err(|_| Error::Invariant("stock return exceeds supported range".to_owned()))
}

fn unpack_calculation(
    baseline: Option<i64>,
    calculated: Option<(i64, StockScreenBand)>,
) -> (Option<i64>, Option<i64>, Option<StockScreenBand>) {
    match calculated {
        Some((return_micros, band)) => (baseline, Some(return_micros), Some(band)),
        None => (None, None, None),
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_coverage_failure(
    connection: &rusqlite::Connection,
    run_id: &str,
    universe_snapshot_id: &str,
    total: u32,
    current: u32,
    baseline_5: u32,
    baseline_21: u32,
    universe_sha256: &str,
    market_data_sha256: &str,
    failure_code: &str,
) -> Result<()> {
    let changed = connection.execute(
        "UPDATE stock_screen_runs
         SET universe_snapshot_id = ?2, status = 'coverage_failed',
             total_members = ?3, current_covered = ?4,
             baseline_5_covered = ?5, baseline_21_covered = ?6,
             result_count = 0, universe_sha256 = ?7,
             market_data_sha256 = ?8, failure_code = ?9, completed_at = ?10
         WHERE id = ?1 AND status = 'started'",
        params![
            run_id,
            universe_snapshot_id,
            total,
            current,
            baseline_5,
            baseline_21,
            universe_sha256,
            market_data_sha256,
            failure_code,
            now_utc(),
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "stock screen run changed during coverage failure".to_owned(),
        ));
    }
    Ok(())
}

fn coverage_count(
    members: &[StoredMember],
    date: NaiveDate,
    prices: &HashMap<(String, NaiveDate), i64>,
) -> Result<u32> {
    u32::try_from(
        members
            .iter()
            .filter(|member| prices.contains_key(&(member.ticker.clone(), date)))
            .count(),
    )
    .map_err(|_| invalid("stock coverage count is too large"))
}

fn query_screen_sessions(
    connection: &rusqlite::Connection,
    market_date: NaiveDate,
    source_sha256: &str,
) -> Result<Vec<NaiveDate>> {
    let mut statement = connection.prepare(
        "SELECT session_date FROM stock_market_sessions
         WHERE session_date <= ?1 AND source_sha256 = ?2
         ORDER BY session_date DESC LIMIT 22",
    )?;
    let rows = statement.query_map(params![market_date, source_sha256], |row| row.get(0))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn query_prices(
    connection: &rusqlite::Connection,
    dates: &[NaiveDate; 3],
    source_sha256: &str,
) -> Result<HashMap<(String, NaiveDate), i64>> {
    let mut statement = connection.prepare(
        "SELECT ticker, session_date, close_microusd
         FROM stock_daily_bars
         WHERE session_date IN (?1, ?2, ?3) AND source_sha256 = ?4",
    )?;
    let rows = statement.query_map(
        params![dates[0], dates[1], dates[2], source_sha256],
        |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, NaiveDate>(1)?),
                row.get::<_, i64>(2)?,
            ))
        },
    )?;
    rows.collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(Into::into)
}

fn query_universe_members(
    connection: &rusqlite::Connection,
    snapshot_id: &str,
) -> Result<Vec<StoredMember>> {
    let mut statement = connection.prepare(
        "SELECT ticker, display_name, sector
         FROM stock_universe_members WHERE snapshot_id = ?1 ORDER BY ticker",
    )?;
    let rows = statement.query_map([snapshot_id], |row| {
        Ok(StoredMember {
            ticker: row.get(0)?,
            display_name: row.get(1)?,
            sector: row.get(2)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn query_screen_results(
    connection: &rusqlite::Connection,
    run_id: &str,
) -> Result<Vec<StoredScreenResult>> {
    let mut statement = connection.prepare(
        "SELECT run_id, ticker, display_name, sector, current_date,
                current_close_microusd, baseline_5_date,
                baseline_5_close_microusd, return_5_micros, band_5,
                baseline_21_date, baseline_21_close_microusd,
                return_21_micros, band_21, universe_sha256, market_data_sha256
         FROM stock_screen_results WHERE run_id = ?1 ORDER BY ticker",
    )?;
    let rows = statement.query_map([run_id], map_screen_result)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn map_screen_result(row: &Row<'_>) -> rusqlite::Result<StoredScreenResult> {
    Ok(StoredScreenResult {
        run_id: row.get(0)?,
        ticker: row.get(1)?,
        display_name: row.get(2)?,
        sector: row.get(3)?,
        current_date: row.get(4)?,
        current_close_microusd: positive_u64(row, 5)?,
        baseline_5_date: row.get(6)?,
        baseline_5_close_microusd: optional_positive_u64(row, 7)?,
        return_5_micros: row.get(8)?,
        band_5: optional_band(row, 9)?,
        baseline_21_date: row.get(10)?,
        baseline_21_close_microusd: optional_positive_u64(row, 11)?,
        return_21_micros: row.get(12)?,
        band_21: optional_band(row, 13)?,
        universe_sha256: row.get(14)?,
        market_data_sha256: row.get(15)?,
    })
}

fn optional_band(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<StockScreenBand>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| {
            StockScreenBand::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

fn positive_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn optional_positive_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

const SCREEN_RUN_SELECT: &str =
    "SELECT id, market_date, universe_snapshot_id, status, total_members,
            current_covered, baseline_5_covered, baseline_21_covered,
            result_count, universe_sha256, market_data_sha256, failure_code,
            started_at, completed_at
     FROM stock_screen_runs";

fn query_screen_run(connection: &rusqlite::Connection, id: &str) -> Result<StockScreenRun> {
    query_screen_run_optional(connection, id)?.ok_or_else(|| not_found("stock screen run", id))
}

fn query_screen_run_optional(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<Option<StockScreenRun>> {
    connection
        .query_row(
            &format!("{SCREEN_RUN_SELECT} WHERE id = ?1"),
            [id],
            map_screen_run,
        )
        .optional()
        .map_err(Into::into)
}

fn map_screen_run(row: &Row<'_>) -> rusqlite::Result<StockScreenRun> {
    Ok(StockScreenRun {
        id: row.get(0)?,
        market_date: row.get(1)?,
        universe_snapshot_id: row.get(2)?,
        status: row.get(3)?,
        total_members: row.get(4)?,
        current_covered: row.get(5)?,
        baseline_5_covered: row.get(6)?,
        baseline_21_covered: row.get(7)?,
        result_count: row.get(8)?,
        universe_sha256: row.get(9)?,
        market_data_sha256: row.get(10)?,
        failure_code: row.get(11)?,
        started_at: row.get(12)?,
        completed_at: row.get(13)?,
    })
}

fn query_universe_snapshot(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<StockUniverseSnapshot> {
    query_universe_snapshot_optional(connection, id)?
        .ok_or_else(|| not_found("stock universe snapshot", id))
}

fn query_universe_snapshot_optional(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<Option<StockUniverseSnapshot>> {
    connection
        .query_row(
            "SELECT id, name, effective_date, source_url, source_revision,
                    source_sha256, license_name, license_url, attribution_text,
                    member_count, fetched_at, created_at
             FROM stock_universe_snapshots WHERE id = ?1",
            [id],
            map_universe_snapshot,
        )
        .optional()
        .map_err(Into::into)
}

fn map_universe_snapshot(row: &Row<'_>) -> rusqlite::Result<StockUniverseSnapshot> {
    Ok(StockUniverseSnapshot {
        id: row.get(0)?,
        name: row.get(1)?,
        effective_date: row.get(2)?,
        source_url: row.get(3)?,
        source_revision: row.get(4)?,
        source_sha256: row.get(5)?,
        license_name: row.get(6)?,
        license_url: row.get(7)?,
        attribution_text: row.get(8)?,
        member_count: row.get(9)?,
        fetched_at: row.get(10)?,
        created_at: row.get(11)?,
    })
}

fn query_ai_report(connection: &rusqlite::Connection, id: &str) -> Result<StockAiReport> {
    query_ai_report_optional(connection, id)?.ok_or_else(|| not_found("stock AI report", id))
}

fn query_ai_report_optional(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<Option<StockAiReport>> {
    connection
        .query_row(
            &format!("{AI_REPORT_SELECT} WHERE id = ?1"),
            [id],
            map_ai_report,
        )
        .optional()
        .map_err(Into::into)
}

fn latest_ai_report_for_run(
    connection: &rusqlite::Connection,
    run_id: &str,
) -> Result<Option<StockAiReport>> {
    connection
        .query_row(
            &format!(
                "{AI_REPORT_SELECT}
                 WHERE screen_run_id = ?1
                 ORDER BY created_at DESC, id DESC LIMIT 1"
            ),
            [run_id],
            map_ai_report,
        )
        .optional()
        .map_err(Into::into)
}

const AI_REPORT_SELECT: &str =
    "SELECT id, screen_run_id, status, prompt_version, model, response_id,
            upstream_request_id, result_json, input_tokens, cached_input_tokens,
            output_tokens, total_tokens, estimated_cost_microusd, failure_code,
            request_started_at, created_at, completed_at
     FROM stock_ai_reports";

fn map_ai_report(row: &Row<'_>) -> rusqlite::Result<StockAiReport> {
    let result_json = row.get::<_, Option<String>>(7)?;
    Ok(StockAiReport {
        id: row.get(0)?,
        screen_run_id: row.get(1)?,
        status: row.get(2)?,
        prompt_version: row.get(3)?,
        model: row.get(4)?,
        response_id: row.get(5)?,
        upstream_request_id: row.get(6)?,
        result: result_json
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        input_tokens: row.get(8)?,
        cached_input_tokens: row.get(9)?,
        output_tokens: row.get(10)?,
        total_tokens: row.get(11)?,
        estimated_cost_microusd: row.get(12)?,
        failure_code: row.get(13)?,
        request_started_at: row.get(14)?,
        created_at: row.get(15)?,
        completed_at: row.get(16)?,
    })
}

fn latest_expected_nyse_date(as_of: DateTime<Utc>) -> Result<NaiveDate> {
    let local = as_of.with_timezone(&New_York);
    let confirmed_close = NaiveTime::from_hms_opt(16, 15, 0)
        .ok_or_else(|| Error::Invariant("invalid NYSE confirmation time".to_owned()))?;
    let mut candidate = local.date_naive();
    if local.time() < confirmed_close || !is_nyse_trading_day(candidate)? {
        candidate = candidate
            .pred_opt()
            .ok_or_else(|| Error::Invariant("NYSE date has no predecessor".to_owned()))?;
    }
    for _ in 0..=14 {
        if is_nyse_trading_day(candidate)? {
            return Ok(candidate);
        }
        candidate = candidate
            .pred_opt()
            .ok_or_else(|| Error::Invariant("NYSE date has no predecessor".to_owned()))?;
    }
    Err(Error::Invariant(
        "unable to resolve latest confirmed NYSE trading date".to_owned(),
    ))
}

fn is_nyse_trading_day(date: NaiveDate) -> Result<bool> {
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return Ok(false);
    }
    let mut holidays = BTreeSet::new();
    for year in (date.year() - 1)..=(date.year() + 1) {
        holidays.insert(observed_new_year(year)?);
        holidays.insert(nth_weekday(year, 1, Weekday::Mon, 3)?);
        holidays.insert(nth_weekday(year, 2, Weekday::Mon, 3)?);
        holidays.insert(easter_sunday(year)? - Duration::days(2));
        holidays.insert(last_weekday(year, 5, Weekday::Mon)?);
        if year >= 2022 {
            holidays.insert(observed_fixed_holiday(year, 6, 19)?);
        }
        holidays.insert(observed_fixed_holiday(year, 7, 4)?);
        holidays.insert(nth_weekday(year, 9, Weekday::Mon, 1)?);
        holidays.insert(nth_weekday(year, 11, Weekday::Thu, 4)?);
        holidays.insert(observed_fixed_holiday(year, 12, 25)?);
    }
    Ok(!holidays.contains(&date))
}

fn observed_new_year(year: i32) -> Result<NaiveDate> {
    let date = NaiveDate::from_ymd_opt(year, 1, 1)
        .ok_or_else(|| Error::Invariant("invalid NYSE New Year's Day".to_owned()))?;
    Ok(match date.weekday() {
        // NYSE does not substitute the preceding Friday when January 1 is a Saturday.
        Weekday::Sat => date,
        Weekday::Sun => date + Duration::days(1),
        _ => date,
    })
}

fn observed_fixed_holiday(year: i32, month: u32, day: u32) -> Result<NaiveDate> {
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| Error::Invariant("invalid fixed NYSE holiday".to_owned()))?;
    Ok(match date.weekday() {
        Weekday::Sat => date - Duration::days(1),
        Weekday::Sun => date + Duration::days(1),
        _ => date,
    })
}

fn nth_weekday(year: i32, month: u32, weekday: Weekday, nth: u32) -> Result<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| Error::Invariant("invalid NYSE holiday month".to_owned()))?;
    let offset = (7 + weekday.num_days_from_monday() as i64
        - first.weekday().num_days_from_monday() as i64)
        % 7;
    first
        .checked_add_signed(Duration::days(
            offset + 7 * i64::from(nth.saturating_sub(1)),
        ))
        .ok_or_else(|| Error::Invariant("NYSE holiday is out of range".to_owned()))
}

fn last_weekday(year: i32, month: u32, weekday: Weekday) -> Result<NaiveDate> {
    let next_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .ok_or_else(|| Error::Invariant("invalid NYSE holiday month".to_owned()))?;
    let mut date = next_month - Duration::days(1);
    while date.weekday() != weekday {
        date -= Duration::days(1);
    }
    Ok(date)
}

fn easter_sunday(year: i32) -> Result<NaiveDate> {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32)
        .ok_or_else(|| Error::Invariant("invalid computed Easter date".to_owned()))
}

fn normalize_us_ticker(value: &str) -> Result<String> {
    let ticker = value.trim().to_ascii_uppercase();
    if ticker.is_empty()
        || ticker.len() > 10
        || !ticker
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(invalid(
            "US ticker must contain 1-10 letters, digits, dots, or hyphens",
        ));
    }
    Ok(ticker)
}

fn clean_text(field: &str, value: &str, maximum: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > maximum || value.contains(['\r', '\n', '\0']) {
        return Err(invalid(format!("{field} has an invalid format")));
    }
    Ok(value.to_owned())
}

fn clean_optional_text(
    field: &str,
    value: Option<String>,
    maximum: usize,
) -> Result<Option<String>> {
    value
        .map(|value| clean_text(field, &value, maximum))
        .transpose()
}

fn validate_https_url(field: &str, value: &str) -> Result<String> {
    let value = clean_text(field, value, 2048)?;
    let parsed = Url::parse(&value).map_err(|_| invalid(format!("{field} is invalid")))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid(format!("{field} must be an HTTPS URL")));
    }
    Ok(value)
}

fn normalize_sha256(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(
            "SHA-256 must contain exactly 64 hexadecimal characters",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn exact_integer_boundaries_are_classified_without_float_error() -> Result<()> {
        assert_eq!(
            calculate_band(Some(110), Some(100))?,
            Some((100_000, StockScreenBand::Up10To20))
        );
        assert_eq!(
            calculate_band(Some(120), Some(100))?,
            Some((200_000, StockScreenBand::Up20Plus))
        );
        assert_eq!(
            calculate_band(Some(90), Some(100))?,
            Some((-100_000, StockScreenBand::Down10To20))
        );
        assert_eq!(
            calculate_band(Some(80), Some(100))?,
            Some((-200_000, StockScreenBand::Down20Plus))
        );
        assert_eq!(calculate_band(Some(109), Some(100))?, None);
        assert_eq!(calculate_band(Some(91), Some(100))?, None);
        Ok(())
    }

    #[test]
    fn coverage_threshold_is_exact() {
        assert!(coverage_passes(980, 1000));
        assert!(!coverage_passes(979, 1000));
        assert!(coverage_passes(493, 503));
        assert!(!coverage_passes(492, 503));
    }

    #[test]
    fn expected_nyse_date_respects_dst_close_weekends_and_observed_holidays() -> Result<()> {
        let summer_before_close = Utc
            .with_ymd_and_hms(2026, 7, 6, 20, 14, 0)
            .single()
            .ok_or_else(|| Error::Invariant("invalid test time".to_owned()))?;
        let summer_after_close = Utc
            .with_ymd_and_hms(2026, 7, 6, 20, 16, 0)
            .single()
            .ok_or_else(|| Error::Invariant("invalid test time".to_owned()))?;
        assert_eq!(
            latest_expected_nyse_date(summer_before_close)?,
            NaiveDate::from_ymd_opt(2026, 7, 2)
                .ok_or_else(|| Error::Invariant("invalid test date".to_owned()))?
        );
        assert_eq!(
            latest_expected_nyse_date(summer_after_close)?,
            NaiveDate::from_ymd_opt(2026, 7, 6)
                .ok_or_else(|| Error::Invariant("invalid test date".to_owned()))?
        );

        let winter_before_close = Utc
            .with_ymd_and_hms(2026, 1, 5, 21, 14, 0)
            .single()
            .ok_or_else(|| Error::Invariant("invalid test time".to_owned()))?;
        let winter_after_close = Utc
            .with_ymd_and_hms(2026, 1, 5, 21, 16, 0)
            .single()
            .ok_or_else(|| Error::Invariant("invalid test time".to_owned()))?;
        assert_eq!(
            latest_expected_nyse_date(winter_before_close)?,
            NaiveDate::from_ymd_opt(2026, 1, 2)
                .ok_or_else(|| Error::Invariant("invalid test date".to_owned()))?
        );
        assert_eq!(
            latest_expected_nyse_date(winter_after_close)?,
            NaiveDate::from_ymd_opt(2026, 1, 5)
                .ok_or_else(|| Error::Invariant("invalid test date".to_owned()))?
        );
        Ok(())
    }

    #[test]
    fn expected_nyse_date_skips_good_friday_and_thanksgiving() -> Result<()> {
        let good_friday = Utc
            .with_ymd_and_hms(2026, 4, 3, 22, 0, 0)
            .single()
            .ok_or_else(|| Error::Invariant("invalid test time".to_owned()))?;
        assert_eq!(
            latest_expected_nyse_date(good_friday)?,
            NaiveDate::from_ymd_opt(2026, 4, 2)
                .ok_or_else(|| Error::Invariant("invalid test date".to_owned()))?
        );
        let thanksgiving = Utc
            .with_ymd_and_hms(2026, 11, 26, 23, 0, 0)
            .single()
            .ok_or_else(|| Error::Invariant("invalid test time".to_owned()))?;
        assert_eq!(
            latest_expected_nyse_date(thanksgiving)?,
            NaiveDate::from_ymd_opt(2026, 11, 25)
                .ok_or_else(|| Error::Invariant("invalid test date".to_owned()))?
        );
        Ok(())
    }

    #[test]
    fn saturday_new_year_does_not_close_the_preceding_friday() -> Result<()> {
        let friday_after_close = Utc
            .with_ymd_and_hms(2027, 12, 31, 21, 16, 0)
            .single()
            .ok_or_else(|| Error::Invariant("invalid test time".to_owned()))?;
        assert_eq!(
            latest_expected_nyse_date(friday_after_close)?,
            NaiveDate::from_ymd_opt(2027, 12, 31)
                .ok_or_else(|| Error::Invariant("invalid test date".to_owned()))?
        );
        Ok(())
    }
}
