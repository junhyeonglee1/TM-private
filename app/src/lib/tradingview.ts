export const tradingViewUrl = (symbol: string): string =>
  `https://www.tradingview.com/symbols/${symbol.replace(":", "-")}/`;

export const buildTradingViewDocument = (
  symbol: string,
  watchlist: string[],
): string => {
  const configuration = JSON.stringify({
    allow_symbol_change: true,
    autosize: true,
    calendar: false,
    details: false,
    hide_legend: false,
    hide_side_toolbar: false,
    hide_top_toolbar: false,
    hide_volume: false,
    hotlist: false,
    interval: "D",
    locale: "kr",
    save_image: false,
    style: "1",
    symbol,
    theme: "dark",
    timezone: "Asia/Seoul",
    watchlist,
    withdateranges: true,
  }).replaceAll("<", "\\u003c");

  return `<!doctype html>
<html lang="ko">
<head>
  <meta charset="utf-8">
  <meta name="referrer" content="no-referrer">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src https://s3.tradingview.com; style-src 'unsafe-inline'; frame-src https://s.tradingview.com https://www.tradingview-widget.com https://www.tradingview.com; base-uri 'none'; form-action 'none'">
  <style>html,body,.tradingview-widget-container,.tradingview-widget-container__widget{width:100%;height:100%;margin:0;background:#10141d;overflow:hidden}.tradingview-widget-copyright{height:24px;padding:4px 8px;box-sizing:border-box;font:11px system-ui;color:#929cb3}.tradingview-widget-copyright a{color:#aebcff;text-decoration:none}.tradingview-widget-container__widget{height:calc(100% - 24px)}</style>
</head>
<body>
  <div class="tradingview-widget-container">
    <div class="tradingview-widget-container__widget"></div>
    <div class="tradingview-widget-copyright"><a href="${tradingViewUrl(symbol)}" rel="noopener nofollow noreferrer" target="_blank">시장 차트</a> by TradingView</div>
    <script src="https://s3.tradingview.com/external-embedding/embed-widget-advanced-chart.js" async>${configuration}</script>
  </div>
</body>
</html>`;
};
