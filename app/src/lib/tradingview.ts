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
  const configuration = JSON.stringify({
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
    locale: "kr",
    save_image: false,
    style: "1",
    support_host: "https://www.tradingview.com",
    symbol: safeSymbol,
    theme: "dark",
    timezone: "Asia/Seoul",
    watchlist: safeWatchlist,
    width: "100%",
    withdateranges: true,
  }).replaceAll("<", "\\u003c");
  const externalUrl = tradingViewUrl(safeSymbol);
  const document = `<!doctype html>
<html lang="ko">
<head>
  <meta charset="utf-8">
  <meta name="referrer" content="no-referrer">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-tm-stock-widget-v1' https://s3.tradingview.com; style-src 'unsafe-inline'; frame-src https://s.tradingview.com https://www.tradingview-widget.com https://www.tradingview.com; base-uri 'none'; form-action 'none'">
  <style>
    :root{color-scheme:dark;font-family:Inter,Pretendard,system-ui,sans-serif}
    *{box-sizing:border-box}html,body,.widget-shell,.tradingview-widget-container{width:100%;height:100%;margin:0}
    body{min-height:460px;overflow:hidden;background:#10141d;color:#d9e0f2}
    .widget-shell{position:relative;display:grid;grid-template-rows:minmax(0,1fr) 26px}
    .tradingview-widget-container{min-height:0}.tradingview-widget-container__widget{width:100%;height:100%;min-height:434px}
    .widget-status{position:absolute;inset:0 0 26px;z-index:2;align-content:center;display:grid;justify-items:center;gap:8px;padding:24px;background:#10141d;color:#929cb3;text-align:center}
    .widget-status strong{color:#d9e0f2}.widget-status p{max-width:440px;margin:0;font-size:12px;line-height:1.5}.widget-status a{color:#aebcff}.widget-status.hidden{display:none}
    .tradingview-widget-copyright{height:26px;padding:5px 9px;background:#10141d;color:#929cb3;font-size:11px}
    .tradingview-widget-copyright a{color:#aebcff;text-decoration:none}
  </style>
</head>
<body>
  <main class="widget-shell">
    <div class="tradingview-widget-container"><div id="tradingview-widget" class="tradingview-widget-container__widget"></div></div>
    <div id="widget-status" class="widget-status" role="status"><strong>시장 차트를 불러오는 중입니다.</strong><p>TradingView 연결 상태에 따라 몇 초 정도 걸릴 수 있습니다.</p></div>
    <div class="tradingview-widget-copyright"><a href="${externalUrl}" rel="noopener nofollow noreferrer" target="_blank">시장 차트</a> by TradingView</div>
  </main>
  <script nonce="tm-stock-widget-v1">
    (() => {
      const container = document.querySelector(".tradingview-widget-container");
      const status = document.getElementById("widget-status");
      const fail = () => {
        status.innerHTML = '<strong>차트를 표시하지 못했습니다.</strong><p>인터넷 연결 또는 TradingView 위젯 제한을 확인하세요. <a href="${externalUrl}" rel="noopener nofollow noreferrer" target="_blank">TradingView에서 확인</a></p>';
        status.classList.remove("hidden");
      };
      const observer = new MutationObserver(() => {
        const frame = container.querySelector("iframe");
        if (!frame) return;
        frame.addEventListener("load", () => status.classList.add("hidden"), { once: true });
        setTimeout(() => status.classList.add("hidden"), 1500);
        observer.disconnect();
      });
      observer.observe(container, { childList: true, subtree: true });
      const script = document.createElement("script");
      script.async = true;
      script.src = "https://s3.tradingview.com/external-embedding/embed-widget-advanced-chart.js";
      script.textContent = ${JSON.stringify(configuration)};
      script.addEventListener("error", fail, { once: true });
      container.append(script);
      setTimeout(() => { if (!container.querySelector("iframe")) fail(); }, 12000);
    })();
  </script>
</body>
</html>`;
  return `data:text/html;charset=utf-8,${encodeURIComponent(document)}`;
};
