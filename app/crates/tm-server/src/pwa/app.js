"use strict";

const pairingKey = "tm.mobile.pairing.v1";
const state = {
  device: null,
  activeTab: "assistant",
  taskReport: null,
  taskReportTasks: new Map(),
  projects: [],
  projectsNextOffset: null,
  projectsTotal: 0,
  projectLoadGeneration: 0,
  tasks: [],
  tasksNextOffset: null,
  tasksTotal: 0,
  taskLoadGeneration: 0,
  taskFilters: {
    projectId: "",
    status: ""
  },
  editingTask: null,
  calendar: null,
  calendarMonth: null,
  selectedCalendarDate: null,
  editingCalendarEvent: null,
  stockWatchlist: [],
  stockCatalog: null,
  selectedStockCandidate: null,
  stockSearchActiveIndex: 0,
  selectedStockSymbol: "NASDAQ:AAPL",
  stockScreen: null,
  stockScreenError: null,
  stockScreenResults: [],
  stockScreenNextCursor: null,
  stockScreenResultTotal: 0,
  stockScreenLoadGeneration: 0,
  stockScreenResultGeneration: 0
};
const costRefreshIntervalMs = 5 * 60 * 1000;
const defaultApiHardLimitMicrousd = 20_000_000;
const defaultCloudHardLimitMicrousd = 30_000_000;
let costRefreshTimer = null;
let stockChartLoadTimer = null;

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
  void loadStockScreen();
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
    void loadStockScreen();
  }
  if (tab === "tasks") void loadTaskWorkspace();
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
  const configuration = {
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
    "page-uri": "__NHTTP__",
    save_image: false,
    style: "1",
    support_host: "https://www.tradingview.com",
    symbol: safeSymbol,
    theme: "dark",
    timezone: "Asia/Seoul",
    watchlist: safeWatchlist,
    width: "100%",
    withdateranges: true
  };
  return `https://www.tradingview-widget.com/embed-widget/advanced-chart/?locale=kr#${encodeURIComponent(JSON.stringify(configuration))}`;
}

function formatStockReturn(value) {
  const number = Number(value);
  return `${number > 0 ? "+" : ""}${number.toFixed(2)}%`;
}

function formatStockPrice(microusd) {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2
  }).format(Number(microusd) / 1_000_000);
}

function stockAttemptMessage(screen) {
  const status = screen?.latestAttempt?.status;
  if (status === "started") return "최신 미국 시장 데이터를 확인하고 있습니다.";
  if (status === "no_candidates") return "10% 이상 움직인 종목이 없어 AI를 호출하지 않았습니다.";
  if (status === "upstream_unavailable") return "시장 데이터 제공처에 연결하지 못했습니다. 이전 성공 결과를 유지합니다.";
  if (status === "coverage_failed") return "필수 종목의 98%를 확인하지 못해 이번 결과를 발행하지 않았습니다.";
  if (status === "failed") return "이번 분석을 완료하지 못했습니다.";
  return null;
}

function stockStalenessMessage(reason) {
  if (reason === "first_run_pending") return "첫 시장 데이터 수집과 검증을 기다리고 있습니다.";
  if (reason === "latest_attempt_started") return "최신 미국 시장 데이터를 확인하는 동안 이전 성공 결과를 표시합니다.";
  if (reason === "latest_attempt_succeeded") return "최신 분석은 끝났지만 기대 시장일 확인이 완료되지 않았습니다.";
  if (reason === "latest_attempt_no_candidates") return "최신 시장일에는 10% 이상 움직인 종목이 없어 AI를 호출하지 않았습니다.";
  if (reason === "latest_attempt_coverage_failed") return "필수 종목의 98%를 확인하지 못해 이전 성공 결과를 표시합니다.";
  if (reason === "latest_attempt_upstream_unavailable") return "시장 데이터 제공처에 연결하지 못해 이전 성공 결과를 표시합니다.";
  if (reason === "latest_attempt_failed") return "최신 분석을 완료하지 못해 이전 성공 결과를 표시합니다.";
  if (reason === "latest_success_before_expected_market_date") return "예상된 최신 미국 시장일 결과가 없어 이전 성공 결과를 표시합니다.";
  return "최신 확정 결과가 없어 이전 성공 결과를 표시합니다.";
}

function stockAiResult(ai) {
  const result = ai?.result;
  if (!result || typeof result !== "object" || Array.isArray(result)) return null;
  return {
    headline: typeof result.headline === "string" ? result.headline : null,
    bullets: Array.isArray(result.bullets)
      ? result.bullets.filter((item) => typeof item === "string")
      : []
  };
}

function stockFilterCount(summary) {
  if (!summary) return 0;
  const horizon = byId("stock-screen-horizon").value;
  const direction = byId("stock-screen-direction").value;
  const band = byId("stock-screen-band").value === "ten_to_twenty" ? "TenToTwenty" : "TwentyPlus";
  return Number(summary.counts[`${direction}${horizon}${band}`] || 0);
}

function stockAiStatus(ai, budget) {
  if (budget?.hardStopReached) return "AI 월 예산 도달 · 숫자 결과만 표시";
  if (!ai) return "AI 호출 없음";
  const failureCode = String(ai.failureCode || "").toLocaleLowerCase("en-US");
  if (failureCode.includes("budget_blocked")) return "AI 월 예산 도달 · 숫자 결과만 표시";
  if (failureCode.includes("disabled")) return "AI 요약 꺼짐 · 숫자 결과만 표시";
  if (ai.status === "started") return "AI 요약 준비 중";
  if (ai.status === "succeeded") return "AI 요약 완료";
  if (ai.status === "failed") return "AI 요약 실패 · 수치 결과는 정상";
  if (ai.status === "ai_uncertain") return "AI 결과 불명확 · 자동 재시도 안 함";
  return "AI 상태 확인 필요";
}

function renderStockScreenBrief() {
  const root = byId("stock-screen-brief");
  const content = byId("stock-screen-brief-content");
  if (!root || !content) return;
  root.setAttribute("aria-busy", "false");
  clear(content);
  const summary = state.stockScreen?.latestSuccess;
  if (!summary) {
    const message = state.stockScreenError
      || (state.stockScreen?.stalenessReason
        ? stockStalenessMessage(state.stockScreen.stalenessReason)
        : null)
      || stockAttemptMessage(state.stockScreen)
      || "아직 발행된 분석 결과가 없습니다.";
    content.append(text("p", message, state.stockScreenError ? "empty stock-screen-error" : "empty"));
    return;
  }
  const meta = text("div", "", "stock-screen-brief-meta");
  meta.append(
    text("strong", `${summary.marketDate} 미국 시장 확정 종가`),
    text("small", `${summary.coverage.currentCovered}/${summary.coverage.total} 종목${state.stockScreenError ? " · 연결 오류 시 저장된 결과" : state.stockScreen.stale ? " · 이전 성공 결과" : ""}`)
  );
  content.append(meta);
  if (state.stockScreenError) {
    const refreshError = text("p", `서버 새로고침에 실패해 마지막으로 불러온 결과를 표시합니다. ${state.stockScreenError}`, "stock-screen-notice stock-screen-error");
    refreshError.setAttribute("role", "alert");
    content.append(refreshError);
  }
  const notice = !state.stockScreenError && state.stockScreen.stale
    ? stockStalenessMessage(state.stockScreen.stalenessReason)
    : !state.stockScreenError ? stockAttemptMessage(state.stockScreen) : null;
  if (notice) content.append(text("p", notice, "stock-screen-notice"));
  const list = document.createElement("ol");
  list.className = "stock-screen-brief-list";
  summary.top3.slice(0, 3).forEach((item) => {
    const row = document.createElement("li");
    row.append(
      text("span", item.direction === "up" ? "↑ 상승" : "↓ 하락"),
      text("strong", item.displayName),
      text("em", formatStockReturn(item.returnPct))
    );
    list.append(row);
  });
  if (list.childElementCount) content.append(list);
  else content.append(text("p", "10% 이상 움직인 종목이 없습니다.", "empty"));
  content.append(text("small", "가격 변화만 계산하며 투자 추천이나 예측이 아닙니다.", "stock-screen-disclaimer"));
}

function renderStockScreenResults(loading = false) {
  const container = byId("stock-screen-results");
  const more = byId("stock-screen-more");
  if (!container || !more) return;
  clear(container);
  const horizon = byId("stock-screen-horizon").value;
  const direction = byId("stock-screen-direction").value === "up" ? "상승" : "하락";
  const band = byId("stock-screen-band").value === "ten_to_twenty"
    ? "10% 이상 20% 미만"
    : "20% 이상";
  container.append(text(
    "p",
    `${horizon}거래일 ${direction} ${band} 결과 ${state.stockScreenResults.length.toLocaleString("ko-KR")}개 표시`,
    "sr-only"
  ));
  more.classList.toggle("hidden", !state.stockScreenNextCursor);
  more.disabled = loading;
  if (loading && !state.stockScreenResults.length) {
    container.append(text("p", "종목 목록을 불러오는 중입니다.", "empty"));
    return;
  }
  if (!state.stockScreenResults.length) {
    const expected = stockFilterCount(state.stockScreen?.latestSuccess);
    container.append(text(
      "p",
      expected > 0
        ? "조건 집계는 있지만 서버가 표시할 종목을 반환하지 않았습니다."
        : "선택한 조건에 해당하는 종목이 없습니다.",
      "empty"
    ));
    return;
  }
  const list = document.createElement("ol");
  state.stockScreenResults.forEach((item) => {
    const row = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute("aria-label", `${item.displayName} ${formatStockReturn(item.returnPct)}, 차트에서 보기`);
    const movement = text("span", "", `stock-screen-move ${item.direction}`);
    movement.append(
      text("strong", item.direction === "up" ? "↑ 상승" : "↓ 하락"),
      text("em", formatStockReturn(item.returnPct))
    );
    const identity = text("span", "", "stock-screen-identity");
    identity.append(text("strong", item.displayName), text("small", `${item.ticker}${item.sector ? ` · ${item.sector}` : ""} · ${item.horizon}거래일`));
    const price = text("span", "", "stock-screen-prices");
    price.append(
      text("strong", formatStockPrice(item.currentCloseMicrousd)),
      text("small", `${item.baselineDate} ${formatStockPrice(item.baselineCloseMicrousd)}`)
    );
    button.append(movement, identity, price, text("span", "›", "stock-screen-chevron"));
    button.addEventListener("click", () => {
      const catalogItem = state.stockCatalog?.items.find((candidate) =>
        candidate.market !== "KRX" && candidate.ticker === item.ticker);
      if (!catalogItem) {
        toast(`${item.ticker}의 거래소를 확인하지 못해 차트를 자동 선택하지 않았습니다.`);
        return;
      }
      state.selectedStockSymbol = `${catalogItem.market}:${catalogItem.ticker}`;
      renderStockWatchlist();
      renderStockChart();
      byId("stock-chart-container").scrollIntoView({ behavior: "smooth", block: "start" });
    });
    row.append(button);
    list.append(row);
  });
  container.append(list);
}

function renderStockScreen() {
  const panel = byId("stock-screen-panel");
  const status = byId("stock-screen-status");
  if (!panel || !status) return;
  panel.setAttribute("aria-busy", "false");
  clear(status);
  const summary = state.stockScreen?.latestSuccess;
  if (!summary) {
    const message = state.stockScreenError
      || (state.stockScreen?.stalenessReason
        ? stockStalenessMessage(state.stockScreen.stalenessReason)
        : null)
      || stockAttemptMessage(state.stockScreen)
      || "첫 결과를 기다리고 있습니다.";
    status.append(text("p", message, state.stockScreenError ? "empty stock-screen-error" : "empty"));
    renderStockScreenResults(false);
    return;
  }
  const meta = text("div", "", "stock-screen-meta");
  const badge = text(
    "span",
    state.stockScreenError ? "연결 오류 · 저장된 결과" : state.stockScreen.stale ? "이전 성공 결과" : "최신 확정 결과",
    (state.stockScreenError || state.stockScreen.stale) ? "stock-screen-badge warning" : "stock-screen-badge"
  );
  const market = text("div");
  market.append(
    badge,
    text("strong", `${summary.marketDate} 미국 시장`),
    text("small", `현재 ${Number(summary.coverage.currentPct).toFixed(1)}% · 5일 ${Number(summary.coverage.baseline5Pct).toFixed(1)}% · 21일 ${Number(summary.coverage.baseline21Pct).toFixed(1)}%`)
  );
  const universe = text("div");
  const source = document.createElement("a");
  source.href = summary.universe.sourceUrl;
  source.rel = "noopener noreferrer";
  source.target = "_blank";
  source.textContent = `출처·기준일 ${summary.universe.asOfDate}`;
  universe.append(
    text("span", summary.universe.name),
    text("strong", `${Number(summary.universe.memberCount).toLocaleString("ko-KR")}개 종목`),
    source,
    text("small", summary.universe.attributionText || "구성종목 출처는 위 링크에서 확인")
  );
  meta.append(market, universe);
  status.append(meta);
  if (state.stockScreenError) {
    const refreshError = text("p", `서버 새로고침에 실패해 마지막으로 불러온 결과를 표시합니다. ${state.stockScreenError}`, "stock-screen-notice stock-screen-error");
    refreshError.setAttribute("role", "alert");
    status.append(refreshError);
  }
  const notice = !state.stockScreenError && state.stockScreen.stale
    ? stockStalenessMessage(state.stockScreen.stalenessReason)
    : !state.stockScreenError ? stockAttemptMessage(state.stockScreen) : null;
  if (notice) status.append(text("p", notice, "stock-screen-notice"));
  const ai = text("div", "", "stock-screen-ai");
  const result = stockAiResult(summary.ai);
  const aiText = text("div");
  aiText.append(text("span", stockAiStatus(summary.ai, state.stockScreen.aiBudget)));
  if (result?.headline) aiText.append(text("strong", result.headline));
  ai.append(
    aiText,
    text(
      "small",
      `${summary.ai?.model || "AI 호출 없음"} · 이번 요약 $${(Number(summary.ai?.estimatedCostMicrousd || 0) / 1_000_000).toFixed(4)} · 이번 달 $${(Number(state.stockScreen.aiBudget?.committedMicrousd || 0) / 1_000_000).toFixed(4)} / $${(Number(state.stockScreen.aiBudget?.hardLimitMicrousd || 2_000_000) / 1_000_000).toFixed(0)}`
    )
  );
  if (result?.bullets.length) {
    const bullets = document.createElement("ul");
    result.bullets.forEach((bullet) => bullets.append(text("li", bullet)));
    ai.append(bullets);
  }
  status.append(ai);
  const count = stockFilterCount(summary);
  const countNode = text("p", `${count.toLocaleString("ko-KR")}개`, "stock-screen-count");
  status.append(countNode);
  renderStockScreenResults(false);
}

async function loadStockScreenResults(append = false) {
  const summary = state.stockScreen?.latestSuccess;
  if (!summary) return renderStockScreenResults(false);
  const runId = summary.runId;
  const generation = ++state.stockScreenResultGeneration;
  const cursor = append ? state.stockScreenNextCursor : undefined;
  if (!append) {
    state.stockScreenResults = [];
    state.stockScreenNextCursor = null;
    state.stockScreenResultTotal = 0;
  }
  renderStockScreenResults(true);
  try {
    const page = await stockCommand("list_stock_screen_results", {
      runId,
      horizon: Number(byId("stock-screen-horizon").value),
      direction: byId("stock-screen-direction").value,
      band: byId("stock-screen-band").value,
      cursor,
      limit: 50
    });
    if (
      generation !== state.stockScreenResultGeneration
      || state.stockScreen?.latestSuccess?.runId !== runId
    ) return;
    state.stockScreenResults = append
      ? state.stockScreenResults.concat(page.items)
      : page.items;
    state.stockScreenNextCursor = page.nextCursor;
    state.stockScreenResultTotal = Number(page.total || page.items.length);
    renderStockScreenResults(false);
  } catch (error) {
    if (generation !== state.stockScreenResultGeneration) return;
    if (!append) state.stockScreenResults = [];
    renderStockScreenResults(false);
    const message = text("p", `등락 종목을 불러오지 못했습니다. ${error.message}`, "empty stock-screen-error");
    byId("stock-screen-results").prepend(message);
  }
}

async function loadStockScreen() {
  const generation = ++state.stockScreenLoadGeneration;
  const previousRunId = state.stockScreen?.latestSuccess?.runId || null;
  byId("stock-screen-brief")?.setAttribute("aria-busy", "true");
  byId("stock-screen-panel")?.setAttribute("aria-busy", "true");
  try {
    const next = await stockCommand("get_latest_stock_screen");
    if (generation !== state.stockScreenLoadGeneration) return;
    state.stockScreen = next;
    state.stockScreenError = null;
  } catch (error) {
    if (generation !== state.stockScreenLoadGeneration) return;
    state.stockScreenError = `최근 시장 결과를 확인하지 못했습니다. ${error.message}`;
  }
  renderStockScreenBrief();
  renderStockScreen();
  const nextRunId = state.stockScreen?.latestSuccess?.runId || null;
  if (nextRunId && (nextRunId !== previousRunId || !state.stockScreenResults.length)) {
    await loadStockScreenResults(false);
  }
}

function renderStockChart() {
  const container = byId("stock-chart-container");
  if (!container) return;
  if (stockChartLoadTimer !== null) {
    clearTimeout(stockChartLoadTimer);
    stockChartLoadTimer = null;
  }
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
  frame.setAttribute(
    "sandbox",
    "allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox"
  );
  frame.src = stockWidgetUrl(
    state.selectedStockSymbol,
    state.stockWatchlist.map((item) => item.symbol)
  );
  const loading = text("div", "", "stock-chart-loading");
  loading.setAttribute("role", "status");
  loading.append(text("strong", "시장 차트를 불러오는 중입니다."));
  loading.append(text("p", "TradingView 연결 상태에 따라 몇 초 정도 걸릴 수 있습니다."));
  frame.addEventListener("load", () => {
    if (stockChartLoadTimer !== null) {
      clearTimeout(stockChartLoadTimer);
      stockChartLoadTimer = null;
    }
    loading.remove();
  }, { once: true });
  container.append(frame, loading);
  stockChartLoadTimer = setTimeout(() => {
    if (!loading.isConnected) return;
    clear(loading);
    loading.append(text("strong", "차트를 표시하지 못했습니다."));
    const message = text("p", "TradingView 연결을 확인한 뒤 ");
    const link = document.createElement("a");
    link.href = stockTradingViewUrl(state.selectedStockSymbol);
    link.rel = "noopener noreferrer";
    link.target = "_blank";
    link.textContent = "외부 차트에서 확인";
    message.append(link, document.createTextNode("하세요."));
    loading.append(message);
    stockChartLoadTimer = null;
  }, 15_000);
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

byId("stock-screen-open").addEventListener("click", () => selectTab("stocks"));
byId("stock-screen-refresh").addEventListener("click", async (event) => {
  const button = event.currentTarget;
  setBusy(button, true, "새로고침");
  try {
    await loadStockScreen();
  } finally {
    setBusy(button, false, "새로고침");
  }
});
["stock-screen-horizon", "stock-screen-direction", "stock-screen-band"].forEach((id) => {
  byId(id).addEventListener("change", () => {
    renderStockScreen();
    void loadStockScreenResults(false);
  });
});
byId("stock-screen-more").addEventListener("click", async (event) => {
  const button = event.currentTarget;
  setBusy(button, true, "다음 50개");
  try {
    await loadStockScreenResults(true);
  } finally {
    setBusy(button, false, "다음 50개");
  }
});

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

const taskStatusLabels = {
  inbox: "수신함",
  todo: "할 일",
  in_progress: "진행 중",
  blocked: "막힘",
  done: "완료",
  cancelled: "취소"
};

const taskStatusTransitions = {
  inbox: ["todo", "in_progress", "cancelled"],
  todo: ["inbox", "in_progress", "blocked", "done", "cancelled"],
  in_progress: ["todo", "blocked", "done", "cancelled"],
  blocked: ["todo", "in_progress", "cancelled"],
  done: ["todo", "in_progress"],
  cancelled: ["inbox", "todo"]
};

function mutationHeaders(operation, version = null) {
  const headers = {
    "idempotency-key": crypto.randomUUID(),
    "x-tm-confirm-mutation": operation
  };
  if (version === null) headers["if-none-match"] = "*";
  else headers["if-match"] = `"${version}"`;
  return headers;
}

function activeProject(projectId) {
  return state.projects.find((project) => project.id === projectId);
}

function uncategorizedProject() {
  return state.projects.find((project) => project.systemKey === "uncategorized") || null;
}

function projectLabel(projectId) {
  if (!projectId) return "기타";
  return activeProject(projectId)?.name || "현재 목록 밖의 프로젝트";
}

function taskStatusLabel(status) {
  return taskStatusLabels[status] || status;
}

function populateProjectSelect(select, blankLabel, selectedValue) {
  clear(select);
  const blank = text("option", blankLabel);
  blank.value = "";
  select.append(blank);
  state.projects.forEach((project) => {
    const option = text("option", project.name);
    option.value = project.id;
    select.append(option);
  });
  if (selectedValue && !state.projects.some((project) => project.id === selectedValue)) {
    const unknown = text("option", `현재 프로젝트 · ${selectedValue.slice(0, 8)}`);
    unknown.value = selectedValue;
    select.append(unknown);
  }
  select.value = selectedValue || "";
}

function refreshProjectSelects() {
  populateProjectSelect(
    byId("task-filter-project"),
    "전체 프로젝트",
    state.taskFilters.projectId
  );
  populateProjectSelect(
    byId("task-project"),
    "기타에 자동 배정",
    state.editingTask
      ? (state.editingTask.projectId || "")
      : (byId("task-project").value || uncategorizedProject()?.id || "")
  );
}

async function loadTaskWorkspace() {
  await loadProjects(false);
  await loadTasks(false);
}

async function loadProjects(append = false) {
  const list = byId("projects-list");
  const more = byId("projects-more");
  const offset = append ? state.projectsNextOffset : 0;
  if (append && offset === null) return;
  const generation = append ? state.projectLoadGeneration : ++state.projectLoadGeneration;
  if (!append) loadingList(list);
  more.disabled = true;
  try {
    const params = new URLSearchParams({
      archived: "false",
      limit: "50",
      offset: String(offset),
      sort: "name"
    });
    const data = await api(`/api/v1/projects?${params}`);
    if (generation !== state.projectLoadGeneration) return;
    state.projects = append ? [...state.projects, ...data.items] : data.items;
    state.projectsNextOffset = data.page.nextOffset;
    state.projectsTotal = data.page.total;
    renderProjects();
    refreshProjectSelects();
    renderTasks();
  } catch (error) {
    if (generation !== state.projectLoadGeneration) return;
    if (append) toast(`프로젝트를 더 불러오지 못했습니다. ${error.message}`);
    else listError(list, error);
  } finally {
    if (generation === state.projectLoadGeneration) more.disabled = false;
  }
}

function renderProjects() {
  const list = byId("projects-list");
  clear(list);
  byId("projects-count").textContent = state.projectsTotal > state.projects.length
    ? `${state.projects.length}/${state.projectsTotal}개`
    : `${state.projectsTotal}개`;
  show("projects-more", state.projectsNextOffset !== null);
  if (!state.projects.length) {
    list.append(text("p", "등록된 프로젝트가 없습니다.", "empty"));
    return;
  }
  state.projects.forEach((project) => {
    const card = text("article", "", "project-card");
    const color = text("span", "", "project-color");
    if (/^#[0-9a-f]{6}$/i.test(project.color || "")) {
      color.style.setProperty("--project-color", project.color);
    }
    const details = text("div");
    details.append(text("strong", project.name));
    if (project.description) details.append(text("small", project.description));
    const open = text("button", "Task 보기");
    open.type = "button";
    open.addEventListener("click", () => {
      state.taskFilters.projectId = project.id;
      byId("task-filter-project").value = project.id;
      void loadTasks(false);
    });
    card.append(color, details, open);
    list.append(card);
  });
}

function taskQuery(offset) {
  const params = new URLSearchParams({
    limit: "30",
    offset: String(offset),
    sort: "updated_desc"
  });
  if (state.taskFilters.projectId) params.set("projectId", state.taskFilters.projectId);
  if (state.taskFilters.status) params.set("status", state.taskFilters.status);
  return params;
}

async function loadTasks(append = false) {
  const list = byId("tasks-list");
  const more = byId("tasks-more");
  const offset = append ? state.tasksNextOffset : 0;
  if (append && offset === null) return;
  const generation = append ? state.taskLoadGeneration : ++state.taskLoadGeneration;
  if (!append) loadingList(list);
  more.disabled = true;
  try {
    const data = await api(`/api/v1/tasks?${taskQuery(offset)}`);
    if (generation !== state.taskLoadGeneration) return;
    state.tasks = append ? [...state.tasks, ...data.items] : data.items;
    state.tasksNextOffset = data.page.nextOffset;
    state.tasksTotal = data.page.total;
    renderTasks();
  } catch (error) {
    if (generation !== state.taskLoadGeneration) return;
    if (append) toast(`Task를 더 불러오지 못했습니다. ${error.message}`);
    else listError(list, error);
  } finally {
    if (generation === state.taskLoadGeneration) more.disabled = false;
  }
}

function renderTasks() {
  const list = byId("tasks-list");
  clear(list);
  byId("tasks-count").textContent = state.tasksTotal > state.tasks.length
    ? `${state.tasks.length}/${state.tasksTotal}개`
    : `${state.tasksTotal}개`;
  show("tasks-more", state.tasksNextOffset !== null);
  if (!state.tasks.length) {
    list.append(text("p", "조건에 맞는 Task가 없습니다.", "empty"));
    return;
  }
  state.tasks.forEach((task) => list.append(taskCard(task)));
}

function taskCard(task) {
  const card = text("article", "", "item-card task-card");
  const heading = text("div", "", "task-card-heading");
  const open = text("button");
  open.type = "button";
  open.setAttribute("aria-label", `${task.title} 상세 편집`);
  open.append(text("h2", task.title));
  open.append(text("span", projectLabel(task.projectId), "task-project-name"));
  open.addEventListener("click", () => openTaskForm(task));
  const status = text("span", taskStatusLabel(task.status), `task-status ${task.status}`);
  heading.append(open, status);
  card.append(heading);
  if (task.description) card.append(text("p", task.description, "task-card-description"));
  const meta = text("div", "", "item-meta");
  meta.append(text("span", `우선순위 ${task.priority}`));
  if (task.dueDate) meta.append(text("span", `기한 ${task.dueDate}`));
  if (task.completedAt) {
    meta.append(text("span", `완료 ${new Date(task.completedAt).toLocaleDateString("ko-KR")}`));
  }
  card.append(meta);
  const actions = text("div", "", "task-card-actions");
  const edit = text("button", "상세 편집", "task-edit");
  edit.type = "button";
  edit.addEventListener("click", () => openTaskForm(task));
  actions.append(edit);
  if (task.status === "todo" || task.status === "in_progress") {
    const complete = text("button", "빠른 완료", "task-complete");
    complete.type = "button";
    complete.addEventListener("click", () => void completeTask(task, complete));
    actions.append(complete);
  } else {
    actions.classList.add("single");
  }
  card.append(actions);
  return card;
}

function setTaskStatusOptions(currentStatus = null) {
  const select = byId("task-status");
  const statuses = currentStatus
    ? [currentStatus, ...(taskStatusTransitions[currentStatus] || [])]
    : ["inbox", "todo", "in_progress", "blocked"];
  clear(select);
  [...new Set(statuses)].forEach((status) => {
    const option = text("option", taskStatusLabel(status));
    option.value = status;
    select.append(option);
  });
  select.value = currentStatus || "todo";
}

function openTaskForm(task = null, projectId = "") {
  state.editingTask = task ? { ...task } : null;
  const selectedProjectId = task
    ? (task.projectId || "")
    : (projectId || state.taskFilters.projectId || uncategorizedProject()?.id || "");
  show("project-form", false);
  show("task-form", true);
  byId("task-form-title").textContent = task ? "Task 상세 편집" : "Task 추가";
  byId("task-save").textContent = task ? "변경 저장" : "Task 저장";
  byId("task-title").value = task?.title || "";
  byId("task-description").value = task?.description || "";
  populateProjectSelect(
    byId("task-project"),
    "기타에 자동 배정",
    selectedProjectId
  );
  setTaskStatusOptions(task?.status || null);
  if (!task) byId("task-status").value = "todo";
  byId("task-priority").value = String(task?.priority ?? 0);
  byId("task-due-date").value = task?.dueDate || "";
  byId("task-form").scrollIntoView({ behavior: "smooth", block: "start" });
  byId("task-title").focus({ preventScroll: true });
}

function closeTaskForm() {
  state.editingTask = null;
  byId("task-form").reset();
  show("task-form", false);
}

function taskPatch(editing) {
  const patch = {};
  const title = byId("task-title").value.trim();
  const description = byId("task-description").value.trim();
  const projectId = byId("task-project").value || null;
  const status = byId("task-status").value;
  const priority = Number(byId("task-priority").value);
  const dueDate = byId("task-due-date").value || null;
  if (title !== editing.title) patch.title = title;
  if (description !== editing.description) patch.description = description;
  if (projectId !== (editing.projectId || null)) {
    if (projectId) patch.projectId = projectId;
    else patch.clearProject = true;
  }
  if (status !== editing.status) patch.status = status;
  if (priority !== editing.priority) patch.priority = priority;
  if (dueDate !== (editing.dueDate || null)) {
    if (dueDate) patch.dueDate = dueDate;
    else patch.clearDueDate = true;
  }
  return patch;
}

async function completeTask(task, button) {
  if (!window.confirm(`‘${task.title}’ Task를 완료할까요?`)) return;
  setBusy(button, true, "빠른 완료");
  try {
    await api(`/api/v1/tasks/${encodeURIComponent(task.id)}`, {
      method: "PATCH",
      headers: mutationHeaders("task.update", task.version),
      body: { status: "done" }
    });
    toast(`‘${task.title}’ Task를 완료했습니다.`);
    await loadTasks(false);
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "빠른 완료");
  }
}

byId("project-add").addEventListener("click", () => {
  closeTaskForm();
  show("project-form", true);
  byId("project-form").scrollIntoView({ behavior: "smooth", block: "start" });
  byId("project-name").focus({ preventScroll: true });
});

byId("project-form-close").addEventListener("click", () => {
  byId("project-form").reset();
  show("project-form", false);
});

byId("project-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = byId("project-save");
  const body = {
    name: byId("project-name").value.trim(),
    description: byId("project-description").value.trim(),
    color: byId("project-color").value
  };
  setBusy(button, true, "프로젝트 저장");
  try {
    const result = await api("/api/v1/projects", {
      method: "POST",
      headers: mutationHeaders("project.create"),
      body
    });
    const projectId = result.item?.id || result.resourceId || "";
    byId("project-form").reset();
    show("project-form", false);
    await loadProjects(false);
    if (projectId) {
      state.taskFilters.projectId = projectId;
      refreshProjectSelects();
      await loadTasks(false);
    }
    toast(`‘${body.name}’ 프로젝트를 추가했습니다.`);
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "프로젝트 저장");
  }
});

byId("task-add").addEventListener("click", () => openTaskForm());
byId("task-form-close").addEventListener("click", closeTaskForm);

byId("task-project").addEventListener("change", () => {
  if (state.editingTask) return;
  const status = byId("task-status");
  if (status.value === "inbox" || status.value === "todo") {
    status.value = "todo";
  }
});

byId("task-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const editing = state.editingTask;
  const button = byId("task-save");
  const title = byId("task-title").value.trim();
  const projectId = byId("task-project").value || null;
  const status = byId("task-status").value;
  setBusy(button, true, editing ? "변경 저장" : "Task 저장");
  try {
    if (editing) {
      const patch = taskPatch(editing);
      if (!Object.keys(patch).length) {
        toast("변경된 내용이 없습니다.");
        return;
      }
      if (
        patch.status
        && !window.confirm(
          `‘${editing.title}’ 상태를 ${taskStatusLabel(editing.status)} → ${taskStatusLabel(patch.status)}(으)로 변경하고 저장할까요?`
        )
      ) {
        return;
      }
      await api(`/api/v1/tasks/${encodeURIComponent(editing.id)}`, {
        method: "PATCH",
        headers: mutationHeaders("task.update", editing.version),
        body: patch
      });
      toast(`‘${title}’ Task 변경을 저장했습니다.`);
    } else {
      await api("/api/v1/tasks", {
        method: "POST",
        headers: mutationHeaders("task.create"),
        body: {
          projectId,
          title,
          description: byId("task-description").value.trim(),
          status,
          priority: Number(byId("task-priority").value),
          dueDate: byId("task-due-date").value || null
        }
      });
      toast(`‘${title}’ Task를 추가했습니다.`);
    }
    closeTaskForm();
    await loadTasks(false);
  } catch (error) {
    const suffix = error.status === 409
      ? " 다른 기기에서 변경되었을 수 있습니다. 새로고침 후 다시 확인하세요."
      : " 자동으로 다시 시도하지 않았습니다.";
    toast(`${error.message}${suffix}`);
  } finally {
    setBusy(button, false, editing ? "변경 저장" : "Task 저장");
  }
});

byId("task-filters").addEventListener("submit", (event) => event.preventDefault());
byId("task-filter-project").addEventListener("change", (event) => {
  state.taskFilters.projectId = event.currentTarget.value;
  void loadTasks(false);
});
byId("task-filter-status").addEventListener("change", (event) => {
  state.taskFilters.status = event.currentTarget.value;
  void loadTasks(false);
});
byId("projects-more").addEventListener("click", () => void loadProjects(true));
byId("tasks-more").addEventListener("click", () => void loadTasks(true));

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
    if (button.dataset.refresh === "tasks") void loadTaskWorkspace();
    if (button.dataset.refresh === "notes") void loadNotes();
    if (button.dataset.refresh === "approvals") void loadApprovals();
  });
});

window.addEventListener("online", updateNetwork);
window.addEventListener("offline", updateNetwork);
void boot();
