CREATE TABLE stock_watchlist_items (
    symbol TEXT PRIMARY KEY,
    market TEXT NOT NULL CHECK (market IN ('KRX', 'NASDAQ', 'NYSE', 'AMEX')),
    ticker TEXT NOT NULL CHECK (
        (market = 'KRX'
            AND length(ticker) = 6
            AND ticker GLOB '[0-9][0-9][0-9][0-9][0-9][0-9]')
        OR
        (market <> 'KRX'
            AND length(ticker) BETWEEN 1 AND 10
            AND ticker NOT GLOB '*[^A-Z0-9.-]*')
    ),
    display_name TEXT NOT NULL CHECK (length(trim(display_name)) BETWEEN 1 AND 80),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (symbol = market || ':' || ticker)
) STRICT;

CREATE INDEX idx_stock_watchlist_created
    ON stock_watchlist_items(created_at, symbol);
