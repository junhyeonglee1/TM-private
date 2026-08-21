export const tradingViewUrl = (symbol: string): string =>
  `https://www.tradingview.com/symbols/${symbol.replace(":", "-")}/`;

const SYMBOL_PATTERN = /^(?:KRX:\d{6}|(?:NASDAQ|NYSE|AMEX):[A-Z0-9.-]{1,10})$/;
const DEFAULT_SYMBOL = "NASDAQ:AAPL";

export const stockWidgetUrl = (
  symbol: string,
  watchlist: string[],
): string => {
  const safeSymbol = SYMBOL_PATTERN.test(symbol) ? symbol : DEFAULT_SYMBOL;
  const safeWatchlist = watchlist.filter((item) => SYMBOL_PATTERN.test(item)).slice(0, 50);
  const configuration = {
    allow_symbol_change: true,
    autosize: true,
    calendar: false,
    details: false,
    height: "100%",
    hide_legend: false,
    hide_side_toolbar: false,
    hide_top_toolbar: false,
    hide_volume: false,
    hotlist: false,
    interval: "D",
    "page-uri": "__NHTTP__",
    save_image: false,
    style: "1",
    support_host: "https://www.tradingview.com",
    symbol: safeSymbol,
    theme: "dark",
    timezone: "Asia/Seoul",
    watchlist: safeWatchlist,
    width: "100%",
    withdateranges: true,
  };
  return `https://www.tradingview-widget.com/embed-widget/advanced-chart/?locale=kr#${encodeURIComponent(JSON.stringify(configuration))}`;
};
