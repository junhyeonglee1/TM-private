use std::{fmt, str::FromStr};

use rusqlite::{Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{Error, Result, TmCore, database::now_utc, error::invalid};

pub const STOCK_WATCHLIST_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum StockMarket {
    Krx,
    Nasdaq,
    Nyse,
    Amex,
}

impl StockMarket {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Krx => "KRX",
            Self::Nasdaq => "NASDAQ",
            Self::Nyse => "NYSE",
            Self::Amex => "AMEX",
        }
    }
}

impl fmt::Display for StockMarket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for StockMarket {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "KRX" => Ok(Self::Krx),
            "NASDAQ" => Ok(Self::Nasdaq),
            "NYSE" => Ok(Self::Nyse),
            "AMEX" => Ok(Self::Amex),
            other => Err(invalid(format!("unsupported stock market: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StockWatchlistItem {
    pub symbol: String,
    pub market: StockMarket,
    pub ticker: String,
    pub display_name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpsertStockWatchlistItemInput {
    pub market: StockMarket,
    pub ticker: String,
    pub display_name: String,
}

impl TmCore {
    pub fn stock_watchlist(&self) -> Result<Vec<StockWatchlistItem>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT symbol, market, ticker, display_name, created_at, updated_at
             FROM stock_watchlist_items
             ORDER BY created_at, symbol",
        )?;
        let rows = statement.query_map([], map_watchlist_item)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn upsert_stock_watchlist_item(
        &self,
        input: UpsertStockWatchlistItemInput,
    ) -> Result<StockWatchlistItem> {
        let market = input.market;
        let ticker = normalize_ticker(market, &input.ticker)?;
        let display_name = normalize_display_name(&input.display_name)?;
        let symbol = format!("{market}:{ticker}");
        let now = now_utc();

        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let exists = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM stock_watchlist_items WHERE symbol = ?1
                    )",
                    [&symbol],
                    |row| row.get::<_, bool>(0),
                )?;
                if !exists {
                    let count: i64 = transaction.query_row(
                        "SELECT COUNT(*) FROM stock_watchlist_items",
                        [],
                        |row| row.get(0),
                    )?;
                    if count >= STOCK_WATCHLIST_LIMIT as i64 {
                        return Err(invalid(format!(
                            "stock watchlist cannot exceed {STOCK_WATCHLIST_LIMIT} items"
                        )));
                    }
                }

                transaction.execute(
                    "INSERT INTO stock_watchlist_items(
                        symbol, market, ticker, display_name, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                     ON CONFLICT(symbol) DO UPDATE SET
                        display_name = excluded.display_name,
                        updated_at = excluded.updated_at",
                    params![symbol, market.as_str(), ticker, display_name, now],
                )?;
                query_watchlist_item(transaction, &symbol)
            })
    }

    pub fn delete_stock_watchlist_item(&self, symbol: &str) -> Result<()> {
        let normalized = normalize_symbol(symbol)?;
        self.database.connect()?.execute(
            "DELETE FROM stock_watchlist_items WHERE symbol = ?1",
            [normalized],
        )?;
        Ok(())
    }
}

fn normalize_symbol(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_uppercase();
    let (market, ticker) = value
        .split_once(':')
        .ok_or_else(|| invalid("stock symbol must use MARKET:TICKER"))?;
    if ticker.contains(':') {
        return Err(invalid("stock symbol must contain exactly one separator"));
    }
    let market = StockMarket::from_str(market)?;
    Ok(format!("{market}:{}", normalize_ticker(market, ticker)?))
}

fn normalize_ticker(market: StockMarket, value: &str) -> Result<String> {
    let ticker = value.trim().to_ascii_uppercase();
    let valid = match market {
        StockMarket::Krx => ticker.len() == 6 && ticker.bytes().all(|byte| byte.is_ascii_digit()),
        StockMarket::Nasdaq | StockMarket::Nyse | StockMarket::Amex => {
            !ticker.is_empty()
                && ticker.len() <= 10
                && ticker
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        }
    };
    if !valid {
        return Err(invalid(match market {
            StockMarket::Krx => "KRX ticker must contain exactly six digits",
            _ => "US ticker must contain 1-10 letters, digits, dots, or hyphens",
        }));
    }
    Ok(ticker)
}

fn normalize_display_name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid("stock display name cannot be empty"));
    }
    if value.chars().count() > 80 {
        return Err(invalid("stock display name cannot exceed 80 characters"));
    }
    Ok(value.to_owned())
}

fn query_watchlist_item(
    connection: &rusqlite::Connection,
    symbol: &str,
) -> Result<StockWatchlistItem> {
    connection
        .query_row(
            "SELECT symbol, market, ticker, display_name, created_at, updated_at
             FROM stock_watchlist_items
             WHERE symbol = ?1",
            [symbol],
            map_watchlist_item,
        )
        .map_err(Into::into)
}

fn map_watchlist_item(row: &Row<'_>) -> rusqlite::Result<StockWatchlistItem> {
    let market = row.get::<_, String>(1)?;
    Ok(StockWatchlistItem {
        symbol: row.get(0)?,
        market: StockMarket::from_str(&market).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        ticker: row.get(2)?,
        display_name: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}
