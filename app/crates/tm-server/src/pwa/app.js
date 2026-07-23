"use strict";

const pairingKey = "tm.mobile.pairing.v1";
const state = {
  device: null,
  activeTab: "assistant",
  taskReport: null,
  taskReportTasks: new Map(),
  calendar: null,
  calendarMonth: null,
  selectedCalendarDate: null,
  editingCalendarEvent: null,
  stockWatchlist: [],
  stockCatalog: null,
  selectedStockCandidate: null,
  stockSearchActiveIndex: 0,
  selectedStockSymbol: "NASDAQ:AAPL"
};
const costRefreshIntervalMs = 5 * 60 * 1000;
const defaultApiHardLimitMicrousd = 20_000_000;
const defaultCloudHardLimitMicrousd = 30_000_000;
let costRefreshTimer = null;

const byId = (id) => document.getElementById(id);
const show = (id, visible = true) => byId(id).classList.toggle("hidden", !visible);
const clear = (element) => { while (element.firstChild) element.removeChild(element.firstChild); };
const text = (tag, value, className) => {
  const node = document.createElement(tag);
  node.textContent = value ?? "";
  if (className) node.className = className;
  return node;
};

function cookie(name) {
  const prefix = `${name}=`;
  const matches = document.cookie.split(";").map((value) => value.trim()).filter((value) => value.startsWith(prefix));
  return matches.length === 1 ? matches[0].slice(prefix.length) : null;
}

async function api(path, options = {}) {
  const method = options.method || "GET";
  const headers = new Headers(options.headers || {});
  const unsafe = method !== "GET" && method !== "HEAD";
  if (options.body !== undefined) headers.set("content-type", "application/json");
  if (unsafe) {
    const csrf = cookie("__Host-tm_csrf");
    if (csrf) headers.set("x-tm-csrf", csrf);
  }
  const request = () => fetch(path, {
    method,
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error"
  });

  let response;
  try {
    response = await request();
  } catch (error) {
    if (method !== "GET") throw error;
    response = await request();
  }
  let payload = null;
  try { payload = await response.json(); } catch { /* JSON error handled below */ }
  if (!response.ok) {
    const error = new Error(payload?.error?.message || `요청 실패 (${response.status})`);
    error.status = response.status;
    error.code = payload?.error?.code || "REQUEST_FAILED";
    throw error;
  }
  if (!payload || !("data" in payload)) throw new Error("서버 응답 형식이 올바르지 않습니다.");
  return payload.data;
}

async function calendarCommand(command, args = {}, mutating = false) {
  const headers = mutating ? { "x-tm-confirm-desktop-command": command } : {};
  return api(`/api/v1/desktop/commands/${command}`, {
    method: "POST",
    headers,
    body: { args }
  });
}

async function stockCommand(command, args = {}, mutating = false) {
  const headers = mutating ? { "x-tm-confirm-desktop-command": command } : {};
  return api(`/api/v1/desktop/commands/${command}`, {
    method: "POST",
    headers,
    body: { args }
  });
}

function todaySeoul() {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Seoul", year: "numeric", month: "2-digit", day: "2-digit"
  }).format(new Date());
}

function shiftMonth(month, amount) {
  const [year, monthNumber] = month.split("-").map(Number);
  const shifted = new Date(Date.UTC(year, monthNumber - 1 + amount, 1));
  return `${shifted.getUTCFullYear()}-${String(shifted.getUTCMonth() + 1).padStart(2, "0")}`;
}

function recurrenceText(event) {
  if (event.recurrence === "monthly_day") return `매월 ${event.dayOfMonth}일`;
  if (event.recurrence === "monthly_first_day") return "매월 초일";
  if (event.recurrence === "monthly_last_day") return "매월 말일";
  return "한 번";
}

function toast(message) {
  const node = byId("toast");
  node.textContent = message;
  node.classList.remove("hidden");
  window.clearTimeout(toast.timer);
  toast.timer = window.setTimeout(() => node.classList.add("hidden"), 3600);
}

function setBusy(button, busy, label) {
  button.disabled = busy;
  if (label) button.textContent = busy ? "처리 중…" : label;
}

function updateNetwork() {
  const node = byId("network-state");
  const online = navigator.onLine;
  node.textContent = online ? "온라인" : "오프라인 · 화면만 사용 가능";
  node.classList.toggle("online", online);
  node.classList.toggle("offline", !online);
  if (state.activeTab === "stocks") renderStockChart();
}

function formatUsd(microusd, alwaysCents = true) {
  const dollars = Number(microusd) / 1_000_000;
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: alwaysCents ? 2 : Number.isInteger(dollars) ? 0 : 2,
    maximumFractionDigits: 2
  }).format(dollars);
}

function renderCostPill(node, label, usedMicrousd, hardLimitMicrousd) {
  const used = usedMicrousd === null || usedMicrousd === undefined ? null : Number(usedMicrousd);
  const limit = Number(hardLimitMicrousd);
  node.textContent = `${label} ${used === null ? "—" : formatUsd(used)} / ${formatUsd(limit, false)}`;
  const ratio = used === null || limit <= 0 ? null : used / limit;
  node.classList.toggle("unavailable", ratio === null);
  node.classList.toggle("warning", ratio !== null && ratio >= .8 && ratio < 1);
  node.classList.toggle("danger", ratio !== null && ratio >= 1);
}

async function loadCostStatus() {
  if (!state.device) return;
  try {
    const costs = await api("/api/v1/costs/status");
    renderCostPill(byId("cost-api"), "API", costs.api.usedMicrousd, costs.api.hardLimitMicrousd);
    renderCostPill(byId("cost-cloud"), "Cloud", costs.cloud.usedMicrousd, costs.cloud.hardLimitMicrousd);
    byId("cost-api").title = `OpenAI API ${costs.api.budgetMonth}`;
    byId("cost-cloud").title = costs.cloud.billingPeriodStart && costs.cloud.billingPeriodEnd
      ? `Railway ${new Date(costs.cloud.billingPeriodStart).toLocaleDateString("ko-KR")}–${new Date(costs.cloud.billingPeriodEnd).toLocaleDateString("ko-KR")}${costs.cloud.stale ? " · 마지막 확인값" : ""}`
      : "Railway 비용을 확인할 수 없습니다";
  } catch {
    renderCostPill(byId("cost-api"), "API", null, defaultApiHardLimitMicrousd);
    renderCostPill(byId("cost-cloud"), "Cloud", null, defaultCloudHardLimitMicrousd);
  }
}

function startCostRefresh() {
  if (costRefreshTimer !== null) window.clearInterval(costRefreshTimer);
  void loadCostStatus();
  costRefreshTimer = window.setInterval(() => void loadCostStatus(), costRefreshIntervalMs);
}

function stopCostRefresh() {
  if (costRefreshTimer !== null) window.clearInterval(costRefreshTimer);
  costRefreshTimer = null;
}

function savedPairing() {
  try {
    const value = JSON.parse(sessionStorage.getItem(pairingKey) || "null");
    return value && value.id && value.pollingSecret && value.code ? value : null;
  } catch { return null; }
}

function renderPairing(pairing) {
  show("pairing-code-card", Boolean(pairing));
  byId("pairing-form").classList.toggle("hidden", Boolean(pairing));
  if (!pairing) return;
  byId("pairing-code").textContent = pairing.code;
  const expiry = new Date(pairing.expiresAt);
  byId("pairing-expiry").textContent = `유효 시간: ${expiry.toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" })}까지`;
}

function showPairing(message = "") {
  state.device = null;
  stopCostRefresh();
  show("cost-status", false);
  show("loading-view", false);
  show("app-view", false);
  show("pairing-view", true);
  renderPairing(savedPairing());
  byId("pairing-message").textContent = message;
}

function showApp(device) {
  state.device = device;
  show("loading-view", false);
  show("pairing-view", false);
  show("app-view", true);
  show("cost-status", true);
  renderDevice(device);
  startCostRefresh();
  void loadTaskReport();
}

async function boot() {
  updateNetwork();
  try {
    const device = await api("/api/v1/device/self");
    showApp(device);
  } catch (error) {
    if (error.status === 401 || error.status === 403) showPairing();
    else showPairing(`서버 연결 확인이 필요합니다: ${error.message}`);
  }
  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.register("/mobile/sw.js", { scope: "/mobile/" }).catch(() => undefined);
  }
}

byId("pairing-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = event.currentTarget.querySelector("button[type=submit]");
  setBusy(button, true, "6자리 코드 만들기");
  byId("pairing-message").textContent = "";
  try {
    const data = await api("/api/v1/device-pairings", {
      method: "POST",
      body: { deviceLabel: byId("device-label").value }
    });
    const pairing = {
      id: data.pairing.id,
      code: data.code,
      pollingSecret: data.pollingSecret,
      expiresAt: data.pairing.expiresAt
    };
    sessionStorage.setItem(pairingKey, JSON.stringify(pairing));
    renderPairing(pairing);
  } catch (error) {
    byId("pairing-message").textContent = error.message;
  } finally {
    setBusy(button, false, "6자리 코드 만들기");
  }
});

byId("complete-pairing").addEventListener("click", async (event) => {
  const pairing = savedPairing();
  if (!pairing) return renderPairing(null);
  const button = event.currentTarget;
  setBusy(button, true, "Windows에서 승인했어요");
  byId("pairing-message").textContent = "";
  try {
    const device = await api(`/api/v1/device-pairings/${encodeURIComponent(pairing.id)}/complete`, {
      method: "POST",
      body: { pollingSecret: pairing.pollingSecret }
    });
    sessionStorage.removeItem(pairingKey);
    showApp(device);
    toast("이 기기가 안전하게 등록되었습니다.");
  } catch (error) {
    byId("pairing-message").textContent = error.code === "DEVICE_CONFLICT"
      ? "아직 Windows TM 승인이 확인되지 않았습니다. 승인 후 다시 눌러 주세요."
      : error.message;
  } finally {
    setBusy(button, false, "Windows에서 승인했어요");
  }
});

byId("cancel-pairing").addEventListener("click", () => {
  sessionStorage.removeItem(pairingKey);
  renderPairing(null);
  byId("pairing-message").textContent = "기존 코드는 만료될 때까지 사용할 수 없으며 새 코드만 사용하세요.";
});

document.querySelectorAll(".tabs button").forEach((button) => {
  button.addEventListener("click", () => selectTab(button.dataset.tab));
});

function selectTab(tab) {
  state.activeTab = tab;
  document.querySelectorAll(".tabs button").forEach((button) => {
    if (button.dataset.tab === tab) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  });
  document.querySelectorAll(".tab-panel").forEach((panel) => panel.classList.add("hidden"));
  byId(`tab-${tab}`).classList.remove("hidden");
  if (tab === "calendar") void loadCalendar();
  if (tab === "stocks") {
    void loadStockCatalog();
    void loadStockWatchlist();
  }
  if (tab === "tasks") void loadTasks();
  if (tab === "notes") void loadNotes();
  if (tab === "approvals") void loadApprovals();
  if (tab === "device" && state.device) renderDevice(state.device);
}

function stockTradingViewUrl(symbol) {
  return `https://www.tradingview.com/symbols/${symbol.replace(":", "-")}/`;
}

function stockWidgetUrl(symbol, watchlist) {
  const pattern = /^(?:KRX:\d{6}|(?:NASDAQ|NYSE|AMEX):[A-Z0-9.-]{1,10})$/;
  const safeSymbol = pattern.test(symbol) ? symbol : "NASDAQ:AAPL";
  const safeWatchlist = watchlist.filter((item) => pattern.test(item)).slice(0, 50);
  const configuration = JSON.stringify({
    allow_symbol_change: true,
    autosize: true,
    calendar: false,
    details: false,
    height: "100%",
    hide_legend: false,
    hide_side_toolbar: true,
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
    withdateranges: true
  }).replaceAll("<", "\\u003c");
  const externalUrl = stockTradingViewUrl(safeSymbol);
  const widgetDocument = `<!doctype html>
<html lang="ko">
<head>
  <meta charset="utf-8">
  <meta name="referrer" content="no-referrer">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-tm-stock-widget-v1' https://s3.tradingview.com; style-src 'unsafe-inline'; frame-src https://s.tradingview.com https://www.tradingview-widget.com https://www.tradingview.com; base-uri 'none'; form-action 'none'">
  <style>
    :root{color-scheme:dark;font-family:system-ui,sans-serif}
    *{box-sizing:border-box}html,body,.widget-shell,.tradingview-widget-container{width:100%;height:100%;margin:0}
    body{min-height:420px;overflow:hidden;background:#10141d;color:#d9e0f2}
    .widget-shell{position:relative;display:grid;grid-template-rows:minmax(0,1fr) 26px}
    .tradingview-widget-container{min-height:0}.tradingview-widget-container__widget{width:100%;height:100%;min-height:394px}
    .widget-status{position:absolute;inset:0 0 26px;z-index:2;align-content:center;display:grid;justify-items:center;gap:8px;padding:24px;background:#10141d;color:#929cb3;text-align:center}
    .widget-status strong{color:#d9e0f2}.widget-status p{margin:0;font-size:12px;line-height:1.5}.widget-status a{color:#aebcff}.widget-status.hidden{display:none}
    .tradingview-widget-copyright{height:26px;padding:5px 9px;background:#10141d;color:#929cb3;font-size:11px}
    .tradingview-widget-copyright a{color:#aebcff;text-decoration:none}
  </style>
</head>
<body>
  <main class="widget-shell">
    <div class="tradingview-widget-container">
      <div class="tradingview-widget-container__widget"></div>
      <script
        id="tradingview-embed-script"
        type="text/javascript"
        src="https://s3.tradingview.com/external-embedding/embed-widget-advanced-chart.js"
        async
      >${configuration}</script>
    </div>
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
      let observer;
      const revealWhenReady = () => {
        const frame = container.querySelector("iframe");
        if (!frame) return false;
        frame.addEventListener("load", () => status.classList.add("hidden"), { once: true });
        setTimeout(() => status.classList.add("hidden"), 1500);
        observer?.disconnect();
        return true;
      };
      observer = new MutationObserver(revealWhenReady);
      if (!revealWhenReady()) observer.observe(container, { childList: true, subtree: true });
      window.addEventListener("error", (event) => {
        if (event.target?.id === "tradingview-embed-script") fail();
      }, true);
      setTimeout(() => { if (!container.querySelector("iframe")) fail(); }, 12000);
    })();
  </script>
</body>
</html>`;
  return `data:text/html;charset=utf-8,${encodeURIComponent(widgetDocument)}`;
}

function renderStockChart() {
  const container = byId("stock-chart-container");
  if (!container) return;
  clear(container);
  const online = navigator.onLine;
  const network = byId("stock-network");
  network.textContent = online ? "온라인" : "오프라인";
  network.classList.toggle("online", online);
  network.classList.toggle("offline", !online);
  byId("stock-chart-symbol").textContent = state.selectedStockSymbol;
  byId("stock-external-link").href = stockTradingViewUrl(state.selectedStockSymbol);
  byId("stock-fallback-link").href = stockTradingViewUrl(state.selectedStockSymbol);
  if (!online) {
    const placeholder = text("div", "", "stock-chart-placeholder");
    placeholder.append(text("strong", "차트를 보려면 인터넷 연결이 필요합니다."));
    placeholder.append(text("p", "관심 종목은 연결 후 다시 동기화됩니다."));
    container.append(placeholder);
    return;
  }
  const frame = document.createElement("iframe");
  frame.className = "stock-chart-frame";
  frame.title = `${state.selectedStockSymbol} TradingView 조회 전용 차트`;
  frame.referrerPolicy = "no-referrer";
  frame.setAttribute("sandbox", "allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox");
  frame.src = stockWidgetUrl(
    state.selectedStockSymbol,
    state.stockWatchlist.map((item) => item.symbol)
  );
  container.append(frame);
}

function normalizeStockSearch(value) {
  return value.normalize("NFKC").trim().toLocaleLowerCase("ko-KR");
}

function stockSearchResults() {
  const query = normalizeStockSearch(byId("stock-company-search").value);
  if (!query || !state.stockCatalog) return [];
  const market = byId("stock-market").value;
  const score = (item) => {
    const name = normalizeStockSearch(item.name);
    const ticker = normalizeStockSearch(item.ticker);
    if (name === query || ticker === query) return 0;
    if (name.startsWith(query)) return 1;
    if (ticker.startsWith(query)) return 2;
    if (name.split(/[\s,.(\)/-]+/u).some((word) => word.startsWith(query))) return 3;
    return 4;
  };
  return state.stockCatalog.items
    .filter((item) => {
      if (item.market !== market) return false;
      const name = normalizeStockSearch(item.name);
      const ticker = normalizeStockSearch(item.ticker);
      return name.includes(query) || ticker.includes(query);
    })
    .sort((left, right) =>
      score(left) - score(right)
      || left.name.localeCompare(right.name, "ko")
      || left.ticker.localeCompare(right.ticker))
    .slice(0, 8);
}

function selectStockCandidate(item) {
  state.selectedStockCandidate = item;
  state.stockSearchActiveIndex = 0;
  byId("stock-company-search").value = item.name;
  byId("stock-company-search").setAttribute("aria-expanded", "false");
  show("stock-search-results", false);
  const selection = byId("stock-selection");
  clear(selection);
  selection.append(text("strong", item.name), text("span", `${item.market}:${item.ticker}`));
  show("stock-selection", true);
  byId("stock-save").disabled = false;
}

function clearStockCandidate() {
  state.selectedStockCandidate = null;
  show("stock-selection", false);
  byId("stock-save").disabled = true;
}

function renderStockSearch() {
  const input = byId("stock-company-search");
  const list = byId("stock-search-results");
  const results = stockSearchResults();
  clear(list);
  if (!input.value.trim()) {
    input.setAttribute("aria-expanded", "false");
    show("stock-search-results", false);
    return;
  }
  if (!results.length) {
    list.append(text("p", "일치하는 상장 종목이 없습니다."));
  } else {
    if (state.stockSearchActiveIndex >= results.length) state.stockSearchActiveIndex = 0;
    results.forEach((item, index) => {
      const option = document.createElement("button");
      option.type = "button";
      option.id = `stock-search-option-${index}`;
      option.setAttribute("role", "option");
      option.setAttribute("aria-selected", String(index === state.stockSearchActiveIndex));
      if (index === state.stockSearchActiveIndex) option.className = "active";
      option.append(text("strong", item.name), text("span", `${item.market}:${item.ticker}`));
      option.addEventListener("pointerdown", (event) => event.preventDefault());
      option.addEventListener("click", () => selectStockCandidate(item));
      list.append(option);
    });
    input.setAttribute("aria-activedescendant", `stock-search-option-${state.stockSearchActiveIndex}`);
  }
  input.setAttribute("aria-expanded", "true");
  show("stock-search-results", true);
}

async function loadStockCatalog() {
  if (state.stockCatalog) {
    renderStockCatalogSummary();
    return;
  }
  try {
    const response = await fetch("/mobile/stock-catalog.json", {
      cache: "force-cache",
      credentials: "same-origin"
    });
    if (!response.ok) throw new Error(`목록 요청 실패 (${response.status})`);
    const catalog = await response.json();
    if (!catalog || !Array.isArray(catalog.items) || !catalog.counts) {
      throw new Error("상장 종목 목록 형식이 올바르지 않습니다.");
    }
    state.stockCatalog = catalog;
    renderStockCatalogSummary();
    renderStockSearch();
  } catch (error) {
    byId("stock-catalog-summary").textContent = `상장 종목 목록을 불러오지 못했습니다. ${error.message}`;
  }
}

function renderStockCatalogSummary() {
  if (!state.stockCatalog) return;
  const market = byId("stock-market").value;
  const count = Number(state.stockCatalog.counts[market] || 0).toLocaleString("ko-KR");
  byId("stock-catalog-summary").textContent = `${market} 상장 종목 ${count}개에서 검색`;
}

function renderStockWatchlist() {
  const list = byId("stock-watchlist");
  clear(list);
  byId("stock-watchlist-count").textContent = `${state.stockWatchlist.length}/50`;
  if (!state.stockWatchlist.length) {
    list.append(text("p", "아직 저장한 종목이 없습니다.", "empty"));
  } else {
    state.stockWatchlist.forEach((item) => {
      const row = document.createElement("div");
      row.className = `stock-watchlist-item${state.selectedStockSymbol === item.symbol ? " selected" : ""}`;
      const select = document.createElement("button");
      select.type = "button";
      select.append(text("strong", item.displayName));
      select.append(text("small", item.symbol));
      select.addEventListener("click", () => {
        state.selectedStockSymbol = item.symbol;
        renderStockWatchlist();
        renderStockChart();
      });
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "stock-remove";
      remove.setAttribute("aria-label", `${item.displayName} 관심 종목 삭제`);
      remove.textContent = "×";
      remove.addEventListener("click", async () => {
        try {
          await stockCommand("delete_stock_watchlist_item", { symbol: item.symbol }, true);
          state.stockWatchlist = state.stockWatchlist.filter((candidate) => candidate.symbol !== item.symbol);
          if (state.selectedStockSymbol === item.symbol) {
            state.selectedStockSymbol = state.stockWatchlist[0]?.symbol || "NASDAQ:AAPL";
          }
          renderStockWatchlist();
          renderStockChart();
          toast(`${item.displayName} 관심 종목을 제거했습니다.`);
        } catch (error) {
          toast(error.message);
        }
      });
      row.append(select, remove);
      list.append(row);
    });
  }
}

async function loadStockWatchlist() {
  try {
    state.stockWatchlist = await stockCommand("get_stock_watchlist");
    if (
      state.selectedStockSymbol !== "NASDAQ:AAPL"
      && !state.stockWatchlist.some((item) => item.symbol === state.selectedStockSymbol)
    ) {
      state.selectedStockSymbol = state.stockWatchlist[0]?.symbol || "NASDAQ:AAPL";
    }
    renderStockWatchlist();
    renderStockChart();
  } catch (error) {
    toast(`관심 종목을 불러오지 못했습니다. ${error.message}`);
  }
}

byId("stock-market").addEventListener("change", (event) => {
  const krx = event.currentTarget.value === "KRX";
  const input = byId("stock-company-search");
  input.placeholder = krx ? "예: 삼성전자" : "예: Apple 또는 AAPL";
  input.value = "";
  clearStockCandidate();
  renderStockCatalogSummary();
  renderStockSearch();
});

byId("stock-company-search").addEventListener("input", () => {
  clearStockCandidate();
  state.stockSearchActiveIndex = 0;
  renderStockSearch();
});

byId("stock-company-search").addEventListener("focus", renderStockSearch);
byId("stock-company-search").addEventListener("blur", () => {
  window.setTimeout(() => {
    byId("stock-company-search").setAttribute("aria-expanded", "false");
    show("stock-search-results", false);
  }, 0);
});

byId("stock-company-search").addEventListener("keydown", (event) => {
  const results = stockSearchResults();
  if (event.key === "Escape") {
    byId("stock-company-search").setAttribute("aria-expanded", "false");
    show("stock-search-results", false);
    return;
  }
  if (!results.length) return;
  if (event.key === "ArrowDown") {
    event.preventDefault();
    state.stockSearchActiveIndex = (state.stockSearchActiveIndex + 1) % results.length;
    renderStockSearch();
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    state.stockSearchActiveIndex = (state.stockSearchActiveIndex - 1 + results.length) % results.length;
    renderStockSearch();
  } else if (event.key === "Enter" && byId("stock-company-search").getAttribute("aria-expanded") === "true") {
    event.preventDefault();
    selectStockCandidate(results[state.stockSearchActiveIndex] || results[0]);
  }
});

byId("stock-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const candidate = state.selectedStockCandidate;
  if (!candidate || candidate.market !== byId("stock-market").value) {
    return toast("검색 결과에서 저장할 종목을 먼저 선택하세요.");
  }
  const symbol = `${candidate.market}:${candidate.ticker}`;
  if (state.stockWatchlist.length >= 50 && !state.stockWatchlist.some((item) => item.symbol === symbol)) {
    return toast("관심 종목은 최대 50개까지 저장할 수 있습니다.");
  }
  const button = byId("stock-save");
  setBusy(button, true, "관심 종목 저장");
  try {
    const saved = await stockCommand("upsert_stock_watchlist_item", {
      input: {
        market: candidate.market,
        ticker: candidate.ticker,
        displayName: candidate.name
      }
    }, true);
    state.stockWatchlist = state.stockWatchlist.filter((item) => item.symbol !== saved.symbol);
    state.stockWatchlist.push(saved);
    state.stockWatchlist.sort((left, right) => left.createdAt.localeCompare(right.createdAt) || left.symbol.localeCompare(right.symbol));
    state.selectedStockSymbol = saved.symbol;
    byId("stock-company-search").value = "";
    clearStockCandidate();
    renderStockSearch();
    renderStockWatchlist();
    renderStockChart();
    toast(`${saved.displayName} 관심 종목을 저장했습니다.`);
  } catch (error) {
    toast(error.message);
  } finally {
    setBusy(button, false, "관심 종목 저장");
    button.disabled = !state.selectedStockCandidate;
  }
});

byId("assistant-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = byId("assistant-message");
  const message = input.value.trim();
  if (!message) return;
  const button = event.currentTarget.querySelector("button[type=submit]");
  appendAssistant(message, "user");
  input.value = "";
  setBusy(button, true, "AI 비서에게 전송");
  try {
    const result = await api("/api/v1/assistant/query", {
      method: "POST",
      headers: { "x-tm-confirm-ai-call": "assistant" },
      body: { message }
    });
    appendAssistant(result.answer, "tm");
    void loadCostStatus();
    if (result.proposedActions?.length) toast("승인이 필요한 AI 제안이 생겼습니다.");
  } catch (error) {
    appendAssistant(`요청을 완료하지 못했습니다. ${error.message}\n자동으로 다시 보내지 않았습니다.`, "tm");
  } finally {
    setBusy(button, false, "AI 비서에게 전송");
  }
});

byId("task-report-generate").addEventListener("click", async (event) => {
  const button = event.currentTarget;
  setBusy(button, true, "AI 리포트 만들기");
  try {
    const report = await api("/api/v1/assistant/task-report", {
      method: "POST",
      headers: { "x-tm-confirm-ai-call": "task-report" }
    });
    state.taskReport = report;
    await loadTaskReportTasks();
    renderTaskReport(report);
    void loadCostStatus();
    toast("오늘의 Task AI 리포트를 만들었습니다.");
  } catch (error) {
    renderTaskReportError(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "AI 리포트 만들기");
  }
});

async function loadTaskReport() {
  try {
    state.taskReport = await api("/api/v1/assistant/task-reports/latest");
    await loadTaskReportTasks();
    renderTaskReport(state.taskReport);
  } catch (error) {
    if (error.code !== "TASK_REPORT_DISABLED") renderTaskReportError(error.message);
  }
}

async function loadTaskReportTasks() {
  const data = await api("/api/v1/tasks?limit=100&offset=0&sort=updated_desc");
  state.taskReportTasks = new Map(data.items.map((task) => [task.id, task]));
}

function renderTaskReport(report) {
  const root = byId("task-report-result");
  clear(root);
  if (!report) {
    root.append(text("p", "아직 생성한 리포트가 없습니다.", "empty"));
    return;
  }
  const summary = text("div", "", "task-report-summary");
  summary.append(text("h3", report.report.headline), text("p", report.report.summary));
  root.append(summary);
  if (report.report.priorities.length) {
    const list = text("ol", "", "task-report-priorities");
    report.report.priorities.forEach((priority) => {
      const item = text("li", "");
      item.append(text("span", String(priority.rank), "task-report-rank"));
      const body = text("div", "");
      const task = state.taskReportTasks.get(priority.taskId);
      body.append(text("h3", task?.title || "현재 목록에서 찾을 수 없는 Task"));
      body.append(text("p", priority.reason));
      body.append(text("strong", `다음 행동 · ${priority.nextAction}`));
      if (priority.alert) body.append(text("small", priority.alert));
      item.append(body);
      list.append(item);
    });
    root.append(list);
  }
  const scheduleHighlights = report.report.scheduleHighlights || [];
  if (scheduleHighlights.length) {
    const section = text("section", "", "brief-schedule");
    section.append(text("h3", "가까운 일정"));
    const list = text("ul", "");
    scheduleHighlights.forEach((schedule) => {
      const item = text("li", "");
      item.append(text("span", schedule.kind === "payment" ? "납부" : "일정", `brief-schedule-kind ${schedule.kind}`));
      const body = text("div", "");
      body.append(text("strong", schedule.title));
      body.append(text("small", `${schedule.date}${schedule.eventTime ? ` · ${schedule.eventTime.slice(0, 5)}` : ""}`));
      body.append(text("p", schedule.reason));
      if (schedule.alert) body.append(text("em", schedule.alert));
      item.append(body);
      list.append(item);
    });
    section.append(list);
    root.append(section);
  }
  if (report.report.alerts.length) {
    const alerts = text("ul", "", "task-report-alerts");
    report.report.alerts.forEach((alert) => alerts.append(text("li", alert)));
    root.append(alerts);
  }
  const meta = text("p", "", "task-report-meta");
  const tokenText = report.usage ? `${Number(report.usage.totalTokens).toLocaleString("ko-KR")} tokens` : "AI 호출 없음";
  const cost = (Number(report.estimatedCostMicrousd) / 1_000_000).toFixed(4);
  meta.textContent = `후보 ${report.candidateCount}개 · ${tokenText} · $${cost}${report.latencyMs === null ? "" : ` · ${(report.latencyMs / 1000).toFixed(1)}초`}`;
  root.append(meta);
  const feedback = text("div", "", "task-report-feedback");
  if (report.helpful === null) {
    const helpful = text("button", "도움 됨", "secondary");
    helpful.type = "button";
    helpful.addEventListener("click", () => void rateTaskReport(true, helpful));
    const unhelpful = text("button", "도움 안 됨", "secondary");
    unhelpful.type = "button";
    unhelpful.addEventListener("click", () => void rateTaskReport(false, unhelpful));
    feedback.append(helpful, unhelpful);
  } else {
    feedback.append(text("strong", report.helpful ? "도움 됨으로 평가함" : "도움 안 됨으로 평가함"));
  }
  root.append(feedback);
}

async function rateTaskReport(helpful, button) {
  if (!state.taskReport) return;
  setBusy(button, true, helpful ? "도움 됨" : "도움 안 됨");
  try {
    state.taskReport = await api(`/api/v1/assistant/task-reports/${encodeURIComponent(state.taskReport.runId)}/feedback`, {
      method: "POST",
      body: { helpful }
    });
    renderTaskReport(state.taskReport);
    toast("리포트 평가를 기록했습니다.");
  } catch (error) {
    toast(error.message);
    setBusy(button, false, helpful ? "도움 됨" : "도움 안 됨");
  }
}

function renderTaskReportError(message) {
  const root = byId("task-report-result");
  clear(root);
  root.append(text("p", `리포트를 불러오지 못했습니다: ${message}`, "empty"));
}

function appendAssistant(message, role) {
  const log = byId("assistant-log");
  log.append(text("article", message, `assistant-message assistant-message--${role}`));
  log.scrollTop = log.scrollHeight;
}

async function loadCalendar(month = state.calendarMonth || todaySeoul().slice(0, 7)) {
  state.calendarMonth = month;
  if (!state.selectedCalendarDate || !state.selectedCalendarDate.startsWith(`${month}-`)) {
    state.selectedCalendarDate = month === todaySeoul().slice(0, 7) ? todaySeoul() : `${month}-01`;
  }
  byId("calendar-month-label").textContent = "불러오는 중…";
  try {
    state.calendar = await calendarCommand("get_calendar_month", { month });
    renderCalendar();
  } catch (error) {
    byId("calendar-month-label").textContent = month;
    listError(byId("calendar-agenda-list"), error);
  }
}

function renderCalendar() {
  if (!state.calendar) return;
  const month = state.calendar.month;
  const [year, monthNumber] = month.split("-").map(Number);
  const lastDay = new Date(Date.UTC(year, monthNumber, 0)).getUTCDate();
  const leading = (new Date(Date.UTC(year, monthNumber - 1, 1)).getUTCDay() + 6) % 7;
  const totalCells = Math.ceil((leading + lastDay) / 7) * 7;
  const occurrencesByDate = new Map();
  state.calendar.occurrences.forEach((occurrence) => {
    const values = occurrencesByDate.get(occurrence.date) || [];
    values.push(occurrence);
    occurrencesByDate.set(occurrence.date, values);
  });

  byId("calendar-month-label").textContent = `${year}년 ${monthNumber}월`;
  const grid = byId("calendar-grid");
  clear(grid);
  for (let index = 0; index < totalCells; index += 1) {
    if (index < leading || index >= leading + lastDay) {
      grid.append(text("span", "", "calendar-day empty-day"));
      continue;
    }
    const day = index - leading + 1;
    const date = `${month}-${String(day).padStart(2, "0")}`;
    const occurrences = occurrencesByDate.get(date) || [];
    const button = text("button", "", `calendar-day${date === todaySeoul() ? " today" : ""}`);
    button.type = "button";
    button.setAttribute("aria-label", `${day}일${occurrences.length ? ` 일정 ${occurrences.length}개` : ""}`);
    button.setAttribute("aria-pressed", String(date === state.selectedCalendarDate));
    button.append(text("strong", String(day)));
    const dots = text("span", "", "calendar-dots");
    occurrences.slice(0, 5).forEach((occurrence) => dots.append(text("i", "", `calendar-dot ${occurrence.kind}`)));
    button.append(dots);
    button.addEventListener("click", () => {
      state.selectedCalendarDate = date;
      renderCalendar();
    });
    grid.append(button);
  }
  renderCalendarAgenda();
  renderRecurringCalendarEvents();
}

function renderCalendarAgenda() {
  const date = state.selectedCalendarDate;
  const [year, month, day] = date.split("-").map(Number);
  byId("calendar-selected-label").textContent = `${year}년 ${month}월 ${day}일`;
  const list = byId("calendar-agenda-list");
  clear(list);
  const events = new Map(state.calendar.events.map((event) => [event.id, event]));
  const occurrences = state.calendar.occurrences.filter((occurrence) => occurrence.date === date);
  if (!occurrences.length) {
    list.append(text("p", "등록된 일정이 없습니다.", "empty"));
    return;
  }
  occurrences.forEach((occurrence) => {
    const button = text("button", "", `calendar-agenda-item ${occurrence.kind}`);
    button.type = "button";
    const body = text("span", "");
    body.append(text("strong", occurrence.title));
    const source = events.get(occurrence.eventId);
    const time = occurrence.eventTime ? occurrence.eventTime.slice(0, 5) : "하루 종일";
    body.append(text("small", `${time} · ${recurrenceText(source || occurrence)}`));
    button.append(body);
    if (source) button.addEventListener("click", () => openCalendarForm(source));
    list.append(button);
  });
}

function renderRecurringCalendarEvents() {
  const list = byId("calendar-recurring-list");
  clear(list);
  const recurring = state.calendar.events.filter((event) => event.recurrence !== "none");
  if (!recurring.length) {
    list.append(text("p", "등록된 반복 일정이 없습니다.", "empty"));
    return;
  }
  recurring.forEach((event) => {
    const button = text("button", "", "calendar-rule");
    button.type = "button";
    button.append(text("strong", event.title));
    button.append(text("small", `${recurrenceText(event)} · ${event.startDate}부터${event.endsOn ? ` ${event.endsOn}까지` : " 계속"}`));
    button.addEventListener("click", () => openCalendarForm(event));
    list.append(button);
  });
}

function updateCalendarFormFields() {
  const recurring = byId("calendar-recurrence").value;
  show("calendar-day-field", recurring === "monthly_day");
  show("calendar-end-field", recurring !== "none");
  byId("calendar-day-of-month").required = recurring === "monthly_day";
  byId("calendar-ends-on").min = byId("calendar-start-date").value;
}

function openCalendarForm(calendarEvent = null, date = state.selectedCalendarDate || todaySeoul()) {
  state.editingCalendarEvent = calendarEvent;
  const startDate = calendarEvent?.startDate || date;
  byId("calendar-form-title").textContent = calendarEvent ? "일정 편집" : "일정 추가";
  byId("calendar-title").value = calendarEvent?.title || "";
  byId("calendar-kind").value = calendarEvent?.kind || "personal";
  byId("calendar-start-date").value = startDate;
  byId("calendar-time").value = calendarEvent?.eventTime?.slice(0, 5) || "";
  byId("calendar-recurrence").value = calendarEvent?.recurrence || "none";
  byId("calendar-day-of-month").value = calendarEvent?.dayOfMonth || Number(startDate.slice(8, 10));
  byId("calendar-ends-on").value = calendarEvent?.endsOn || "";
  byId("calendar-description").value = calendarEvent?.description || "";
  byId("calendar-save").textContent = calendarEvent ? "변경 저장" : "일정 추가";
  show("calendar-delete", Boolean(calendarEvent));
  show("calendar-form", true);
  updateCalendarFormFields();
  byId("calendar-title").focus();
  byId("calendar-form").scrollIntoView({ behavior: "smooth", block: "start" });
}

function closeCalendarForm() {
  state.editingCalendarEvent = null;
  show("calendar-form", false);
}

byId("calendar-add").addEventListener("click", () => openCalendarForm());
byId("calendar-add-selected").addEventListener("click", () => openCalendarForm());
byId("calendar-form-close").addEventListener("click", closeCalendarForm);
byId("calendar-prev").addEventListener("click", () => void loadCalendar(shiftMonth(state.calendarMonth || todaySeoul().slice(0, 7), -1)));
byId("calendar-next").addEventListener("click", () => void loadCalendar(shiftMonth(state.calendarMonth || todaySeoul().slice(0, 7), 1)));
byId("calendar-today").addEventListener("click", () => {
  state.selectedCalendarDate = todaySeoul();
  void loadCalendar(todaySeoul().slice(0, 7));
});
byId("calendar-recurrence").addEventListener("change", updateCalendarFormFields);
byId("calendar-start-date").addEventListener("change", (event) => {
  const value = event.currentTarget.value;
  if (value) byId("calendar-day-of-month").value = Number(value.slice(8, 10));
  updateCalendarFormFields();
});

byId("calendar-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const recurrence = byId("calendar-recurrence").value;
  const input = {
    title: byId("calendar-title").value.trim(),
    description: byId("calendar-description").value.trim(),
    kind: byId("calendar-kind").value,
    startDate: byId("calendar-start-date").value,
    eventTime: byId("calendar-time").value ? `${byId("calendar-time").value}:00` : null,
    recurrence,
    dayOfMonth: recurrence === "monthly_day" ? Number(byId("calendar-day-of-month").value) : null,
    endsOn: recurrence === "none" || !byId("calendar-ends-on").value ? null : byId("calendar-ends-on").value
  };
  const editing = state.editingCalendarEvent;
  const button = byId("calendar-save");
  setBusy(button, true, editing ? "변경 저장" : "일정 추가");
  try {
    if (editing) {
      await calendarCommand("update_calendar_event", {
        eventId: editing.id,
        input: { ...input, expectedVersion: editing.version }
      }, true);
    } else {
      await calendarCommand("create_calendar_event", { input }, true);
    }
    const targetMonth = input.startDate.slice(0, 7);
    state.selectedCalendarDate = input.startDate;
    closeCalendarForm();
    await loadCalendar(targetMonth);
    toast(editing ? "일정을 변경했습니다." : "일정을 추가했습니다.");
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, editing ? "변경 저장" : "일정 추가");
  }
});

byId("calendar-delete").addEventListener("click", async (event) => {
  const editing = state.editingCalendarEvent;
  if (!editing || !window.confirm(`‘${editing.title}’ 일정을 삭제할까요?`)) return;
  const button = event.currentTarget;
  setBusy(button, true, "일정 삭제");
  try {
    await calendarCommand("delete_calendar_event", {
      eventId: editing.id, expectedVersion: editing.version
    }, true);
    closeCalendarForm();
    await loadCalendar();
    toast("일정을 삭제했습니다.");
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "일정 삭제");
  }
});

async function loadTasks() {
  const list = byId("tasks-list");
  loadingList(list);
  try {
    const data = await api("/api/v1/tasks?limit=30&offset=0&sort=updated_desc");
    clear(list);
    if (!data.items.length) return list.append(text("p", "등록된 할 일이 없습니다.", "empty"));
    data.items.forEach((task) => {
      const card = text("article", "", "item-card");
      card.append(text("h2", task.title));
      if (task.description) card.append(text("p", task.description));
      const meta = text("div", "", "item-meta");
      meta.append(text("span", task.status));
      meta.append(text("span", `우선순위 ${task.priority}`));
      if (task.dueDate) meta.append(text("span", `기한 ${task.dueDate}`));
      card.append(meta);
      list.append(card);
    });
  } catch (error) { listError(list, error); }
}

async function loadNotes() {
  const list = byId("notes-list");
  loadingList(list);
  try {
    const data = await api("/api/v1/notes?limit=30&offset=0&sort=updated_desc");
    clear(list);
    if (!data.items.length) return list.append(text("p", "등록된 노트가 없습니다.", "empty"));
    data.items.forEach((note) => {
      const card = text("article", "", "item-card");
      card.append(text("h2", note.title));
      card.append(text("p", note.body || note.content || ""));
      card.append(text("div", note.noteType || note.type || "note", "item-meta"));
      list.append(card);
    });
  } catch (error) { listError(list, error); }
}

async function loadApprovals() {
  const list = byId("approvals-list");
  loadingList(list);
  try {
    const data = await api("/api/v1/assistant/actions");
    const pending = data.items.filter((action) => action.status === "pending");
    clear(list);
    if (!pending.length) return list.append(text("p", "대기 중인 승인이 없습니다.", "empty"));
    pending.forEach((action) => list.append(actionCard(action)));
  } catch (error) { listError(list, error); }
}

function actionCard(action) {
  const card = text("article", "", "item-card");
  card.append(text("h2", action.operation));
  card.append(text("p", JSON.stringify(action.preview, null, 2)));
  card.append(text("div", `만료 ${new Date(action.expiresAt).toLocaleString("ko-KR")}`, "item-meta"));
  const row = text("div", "", "action-row");
  const approve = text("button", "승인하고 실행", "approve");
  approve.type = "button";
  approve.addEventListener("click", () => void decideAction(action, true, approve));
  const reject = text("button", "거절", "reject");
  reject.type = "button";
  reject.addEventListener("click", () => void decideAction(action, false, reject));
  row.append(approve, reject);
  card.append(row);
  return card;
}

async function decideAction(action, approve, button) {
  const label = approve ? "승인하고 실행" : "거절";
  if (!window.confirm(approve ? "표시된 내용을 지금 실행할까요?" : "이 제안을 거절할까요?")) return;
  setBusy(button, true, label);
  try {
    const suffix = approve ? "approve" : "reject";
    const headers = { "x-tm-confirm-action": approve ? action.operation : "reject" };
    if (approve) headers["idempotency-key"] = crypto.randomUUID();
    const body = {
      expectedRevision: action.revision,
      payloadSha256: action.payloadSha256
    };
    if (!approve) body.reason = "mobile user rejected";
    await api(`/api/v1/assistant/actions/${encodeURIComponent(action.id)}/${suffix}`, {
      method: "POST", headers, body
    });
    toast(approve ? "승인한 작업을 완료했습니다." : "제안을 거절했습니다.");
    await loadApprovals();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, label);
  }
}

function loadingList(list) { clear(list); list.append(text("p", "온라인 데이터를 불러오는 중…", "empty")); }
function listError(list, error) { clear(list); list.append(text("p", `불러오지 못했습니다: ${error.message}`, "empty")); }

function renderDevice(device) {
  const details = byId("device-details");
  clear(details);
  const values = [
    ["이름", device.label],
    ["등록", new Date(device.createdAt).toLocaleString("ko-KR")],
    ["최근 사용", new Date(device.lastSeenAt).toLocaleString("ko-KR")],
    ["만료", new Date(device.expiresAt).toLocaleString("ko-KR")]
  ];
  values.forEach(([key, value]) => details.append(text("dt", key), text("dd", value)));
}

byId("logout").addEventListener("click", async (event) => {
  if (!window.confirm("이 브라우저의 TM 연결을 즉시 해제할까요?")) return;
  const button = event.currentTarget;
  setBusy(button, true, "이 기기 연결 해제");
  try {
    await api("/api/v1/device/logout", { method: "POST", body: {} });
    showPairing("이 기기 연결을 해제했습니다.");
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "이 기기 연결 해제");
  }
});

document.querySelectorAll("[data-refresh]").forEach((button) => {
  button.addEventListener("click", () => {
    if (button.dataset.refresh === "tasks") void loadTasks();
    if (button.dataset.refresh === "notes") void loadNotes();
    if (button.dataset.refresh === "approvals") void loadApprovals();
  });
});

window.addEventListener("online", updateNetwork);
window.addEventListener("offline", updateNetwork);
void boot();
