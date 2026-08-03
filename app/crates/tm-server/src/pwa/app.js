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
  expenseMonth: null,
  expenseSummary: null,
  expenseReport: null,
  expenseReportRetryNeeded: false,
  expenseSources: [],
  expenseTransactions: [],
  recurringMatchTransactions: [],
  expenseTransactionsNextCursor: null,
  expenseTransactionsTotal: 0,
  expenseReviews: [],
  expenseReviewsNextCursor: null,
  expenseCategoryReviewsNextCursor: null,
  showExpensePurchaseCategories: false,
  recurringExpenses: [],
  recurringExpenseOccurrences: [],
  editingRecurringExpense: null,
  recurringRegistrationReview: null,
  recurringRegistrationCreated: null,
  selectedRecurringExpenseId: null,
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
const pendingExpenseMutationKeys = new Map();
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
  void loadExpenseDueBrief();
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
  if (tab === "expenses") void loadExpenses();
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

const expenseCategoryLabels = {
  food: "식비",
  delivery: "배달",
  cafe: "카페",
  groceries: "장보기",
  housing_utilities: "주거/공과금",
  transportation: "교통",
  ott_subscriptions: "OTT/구독",
  shopping: "쇼핑",
  health: "건강",
  leisure: "여가",
  education: "교육",
  travel: "여행",
  insurance_finance_tax: "보험/금융/세금",
  gifts_dues: "선물/회비",
  refund_income: "환불/수입",
  transfer_settlement: "이체/정산",
  other: "기타",
  unconfirmed: "미확인"
};

const expenseKindLabels = {
  purchase: "구매",
  refund: "환불",
  card_payment: "카드대금",
  wallet_topup: "지갑충전",
  internal_transfer: "내 계좌이동",
  settlement_received: "정산받음",
  settlement_sent: "정산보냄",
  fee: "수수료",
  external_transfer: "외부 송금",
  unknown_p2p: "미확인 송금",
  manual_recurring: "수동 정기지출"
};

const editableExpenseKindLabels = Object.fromEntries(
  Object.entries(expenseKindLabels).filter(([value]) =>
    !["external_transfer", "unknown_p2p", "manual_recurring"].includes(value))
);

const expenseSourceLabels = {
  kb_card_usage_v1: "KB 신용카드",
  kb_account_history_v1: "KB 계좌·체크카드",
  kakaopay_money_v1: "카카오페이머니"
};

const expenseReviewReasonLabels = {
  unknown_p2p: "송금 목적 확인",
  ambiguous_mirror: "중복·이체 여부 확인",
  recurring_match_candidate: "정기지출 거래 연결 후보",
  recurring_registration_candidate: "정기지출 등록 후보",
  category_confirmation: "구매 카테고리 확인",
  import_rejected: "가져오기 거부 행 확인"
};

const recurringOccurrenceLabels = {
  scheduled: "예정",
  due_today: "오늘 납부",
  due_soon: "7일 이내",
  overdue: "기한 경과",
  paid: "납부 완료",
  matched: "거래 연결"
};

function formatExpenseMoney(amountMinor, currency) {
  const zeroDecimal = currency === "KRW" || currency === "JPY";
  const amount = zeroDecimal ? Number(amountMinor) : Number(amountMinor) / 100;
  try {
    return new Intl.NumberFormat("ko-KR", {
      style: "currency",
      currency,
      maximumFractionDigits: zeroDecimal ? 0 : 2
    }).format(amount);
  } catch {
    return `${amount.toLocaleString("ko-KR")} ${currency}`;
  }
}

const expenseFactMetricLabels = {
  net_personal_spend: "순 개인지출",
  gross_purchase: "총구매",
  refunds: "환불",
  settlement_received: "정산받음",
  settlement_sent: "정산보냄",
  fees: "수수료",
  unconfirmed_outflow: "미확인 외부 유출",
  recurring_expected: "정기지출 예정",
  recurring_paid: "정기지출 납부",
  recurring_remaining: "정기지출 잔여"
};

function expenseFactMetricLabel(metric) {
  const delta = metric.startsWith("delta.");
  const normalized = delta ? metric.slice("delta.".length) : metric;
  const label = normalized.startsWith("category.")
    ? (expenseCategoryLabels[normalized.slice("category.".length)] || "카테고리")
    : (expenseFactMetricLabels[normalized] || "집계값");
  return delta ? `전월 대비 ${label}` : label;
}

function expenseFactCitations(report, factIds) {
  const facts = new Map((report.facts || []).map((fact) => [fact.factId, fact]));
  return factIds.flatMap((factId) => {
    const fact = facts.get(factId);
    return fact
      ? [`${expenseFactMetricLabel(fact.metric)} ${formatExpenseMoney(fact.amountMinor, fact.currency)}`]
      : [];
  });
}

function expenseMonthLabel(month) {
  const [year, monthNumber] = month.split("-").map(Number);
  return `${year}년 ${monthNumber}월`;
}

function shiftSeoulDate(date, days) {
  const [year, month, day] = date.split("-").map(Number);
  const value = new Date(Date.UTC(year, month - 1, day));
  value.setUTCDate(value.getUTCDate() + days);
  return value.toISOString().slice(0, 10);
}

async function loadExpenseDueBrief() {
  const root = byId("expense-due-brief-content");
  const today = todaySeoul();
  const month = today.slice(0, 7);
  try {
    const months = Array.from({ length: 14 }, (_, index) => shiftMonth(month, index - 12));
    const groups = await Promise.allSettled(months.map((value) => api(expenseQuery("/api/v1/expenses/recurring/occurrences", { month: value }))));
    const all = groups.flatMap((result) => result.status === "fulfilled" ? result.value : []);
    const partialHistory = groups.some((result) => result.status === "rejected");
    const cutoff = shiftSeoulDate(today, 7);
    const unpaid = all.filter((item) => !["paid", "matched"].includes(item.status));
    const buckets = [
      ["오늘 납부", unpaid.filter((item) => item.dueDate === today)],
      ["7일 이내", unpaid.filter((item) => item.dueDate > today && item.dueDate <= cutoff)],
      ["기한 경과", unpaid.filter((item) => item.dueDate < today)],
      ["금액 변동", all.filter((item) => item.amountChanged && item.dueDate.startsWith(`${month}-`))]
    ];
    clear(root);
    root.append(text("p", `지난 12개월 미납과 다음 달까지 확인합니다.${partialHistory ? " 일부 월은 불러오지 못했습니다." : ""}`, "muted"));
    buckets.forEach(([label, items]) => {
      const card = text("button", "", "expense-due-card"); card.type = "button";
      card.append(text("span", label), text("strong", String(items.length)));
      if (items[0]) card.append(text("small", `${items[0].name} · ${items[0].dueDate}`));
      card.addEventListener("click", () => {
        state.selectedRecurringExpenseId = items[0]?.recurringExpenseId || null;
        selectTab("expenses");
      });
      root.append(card);
    });
  } catch (error) {
    clear(root);
    root.append(text("p", `납부 일정을 불러오지 못했습니다: ${error.message}`, "empty"));
  }
}

function expenseQuery(path, params) {
  const query = new URLSearchParams();
  Object.entries(params).forEach(([key, value]) => {
    if (value !== null && value !== undefined && value !== "") query.set(key, String(value));
  });
  return `${path}?${query.toString()}`;
}

async function loadExpenses(month = state.expenseMonth || todaySeoul().slice(0, 7)) {
  if (state.expenseMonth !== month) state.expenseReportRetryNeeded = false;
  state.expenseMonth = month;
  byId("expense-month-label").textContent = "불러오는 중…";
  loadingList(byId("expense-transactions-list"));
  loadingList(byId("expense-reviews-list"));
  loadingList(byId("expense-recurring-list"));
  loadingList(byId("expense-occurrences-list"));
  state.expenseReviewsNextCursor = null;
  state.expenseCategoryReviewsNextCursor = null;
  show("expense-reviews-more", false);
  try {
    const transactionMonths = [shiftMonth(month, -1), month, shiftMonth(month, 1)];
    const [summary, sources, transactionPage, recurringTransactionPages, reviewPage, categoryReviewPage, recurring, occurrences, report] = await Promise.all([
      api(expenseQuery("/api/v1/expenses/summary", { month })),
      api("/api/v1/expenses/sources"),
      api(expenseQuery("/api/v1/expenses/transactions", { month, limit: 100 })),
      Promise.all(transactionMonths.map((transactionMonth) =>
        api(expenseQuery("/api/v1/expenses/transactions", { month: transactionMonth, limit: 100 })))),
      api(expenseQuery("/api/v1/expenses/reviews", { month, status: "pending", scope: "required", limit: 100 })),
      api(expenseQuery("/api/v1/expenses/reviews", { month, status: "pending", scope: "category_confirmation", limit: 100 })),
      api("/api/v1/expenses/recurring"),
      api(expenseQuery("/api/v1/expenses/recurring/occurrences", { month })),
      api(expenseQuery("/api/v1/expenses/reports/latest", { month })).catch(() => null)
    ]);
    state.expenseSummary = summary;
    state.expenseSources = sources;
    state.expenseReport = report;
    state.expenseTransactions = transactionPage.items;
    state.recurringMatchTransactions = [...new Map(recurringTransactionPages
      .flatMap((page) => page.items)
      .map((transaction) => [transaction.id, transaction])).values()];
    state.expenseTransactionsNextCursor = transactionPage.nextCursor;
    state.expenseReviews = [...reviewPage.items, ...categoryReviewPage.items];
    state.expenseReviewsNextCursor = reviewPage.nextCursor;
    state.expenseCategoryReviewsNextCursor = categoryReviewPage.nextCursor;
    state.recurringExpenses = recurring;
    state.recurringExpenseOccurrences = occurrences;
    renderExpenses();
    if (state.selectedRecurringExpenseId) {
      const selected = state.recurringExpenses.find((item) => item.id === state.selectedRecurringExpenseId);
      state.selectedRecurringExpenseId = null;
      selectExpenseView("recurring");
      if (selected) openRecurringExpenseForm(selected);
    }
  } catch (error) {
    byId("expense-month-label").textContent = expenseMonthLabel(month);
    clear(byId("expense-summary"));
    byId("expense-summary").append(text("p", `지출을 불러오지 못했습니다: ${error.message}`, "empty"));
    ["expense-transactions-list", "expense-reviews-list", "expense-recurring-list", "expense-occurrences-list"]
      .forEach((id) => listError(byId(id), error));
  }
}

function renderExpenses() {
  byId("expense-month-label").textContent = expenseMonthLabel(state.expenseMonth);
  renderExpenseSummary();
  renderExpenseTransactions();
  renderExpenseReviews();
  renderRecurringExpenses();
}

function renderExpenseSummary() {
  const root = byId("expense-summary");
  clear(root);
  const summary = state.expenseSummary;
  if (!summary) return root.append(text("p", "월간 요약이 없습니다.", "empty"));
  const heading = text("div", "", "expense-summary-heading");
  const title = text("div");
  title.append(text("h2", "월간 요약"));
  title.append(text("span", `${summary.status === "confirmed" ? "확정" : summary.status === "provisional" ? "잠정" : "불완전"} · 출처 ${summary.completeness.coveredSourceCount}/${summary.completeness.activeSourceCount}`));
  heading.append(title, text("strong", `미확인 ${summary.completeness.pendingReviewCount}건`, `expense-report-status ${summary.status}`));
  root.append(heading);
  const totals = text("div", "", "expense-currency-list");
  summary.currencies.forEach((currency) => {
    const card = text("article", "", "expense-currency-card");
    card.append(text("span", currency.currency));
    card.append(text("strong", `순 개인지출 ${formatExpenseMoney(currency.netPersonalSpendMinor, currency.currency)}`));
    const metrics = text("div", "", "expense-mini-metrics");
    metrics.append(
      text("span", `총구매 ${formatExpenseMoney(currency.grossPurchaseMinor, currency.currency)}`),
      text("span", `환불 ${formatExpenseMoney(currency.refundsMinor, currency.currency)}`),
      text("span", `고정비 잔여 ${formatExpenseMoney(currency.recurringRemainingMinor, currency.currency)}`),
      text("span", `미확인 유출 ${formatExpenseMoney(currency.unconfirmedOutflowMinor, currency.currency)}`)
    );
    card.append(metrics);
    totals.append(card);
  });
  if (!summary.currencies.length) totals.append(text("p", "이 달에 집계된 지출이 없습니다.", "empty"));
  root.append(totals);
  const dailyList = text("div", "", "expense-daily-list");
  const dailyCurrencies = [...new Set([
    ...summary.currencies.map((item) => item.currency),
    ...summary.daily.map((item) => item.currency)
  ])].sort();
  dailyCurrencies.forEach((currency) => {
    const series = text("article", "", "expense-daily-series");
    const seriesHeading = text("div", "", "expense-daily-heading");
    seriesHeading.append(text("strong", `${currency} 일별 순지출`), text("small", "환율 합산 없음"));
    series.append(seriesHeading);
    const byDate = new Map(summary.daily.filter((item) => item.currency === currency).map((item) => [item.date, item.amountMinor]));
    const daysInMonth = Number(summary.monthEnd.slice(8, 10));
    const points = Array.from({ length: daysInMonth }, (_, index) => {
      const day = index + 1;
      const date = `${summary.month}-${String(day).padStart(2, "0")}`;
      return { day, date, amountMinor: byDate.get(date) || 0 };
    });
    const maximum = Math.max(1, ...points.map((point) => Math.abs(point.amountMinor)));
    const chart = text("div", "", "expense-daily-chart");
    chart.setAttribute("role", "img");
    chart.setAttribute("aria-label", `${currency} 일별 순지출 막대 차트`);
    points.forEach((point) => {
      const bar = text("span", "", point.amountMinor < 0 ? "expense-daily-bar negative" : "expense-daily-bar");
      bar.setAttribute("aria-label", `${point.date} ${formatExpenseMoney(point.amountMinor, currency)}`);
      bar.title = `${point.date} · ${formatExpenseMoney(point.amountMinor, currency)}`;
      const fill = text("i");
      fill.style.height = `${point.amountMinor === 0 ? 1 : Math.max(5, Math.abs(point.amountMinor) / maximum * 100)}%`;
      bar.append(fill, text("small", point.day === 1 || point.day === daysInMonth || point.day % 5 === 0 ? point.day : ""));
      chart.append(bar);
    });
    series.append(chart);
    dailyList.append(series);
  });
  root.append(dailyList);
  const categories = text("div", "", "expense-category-chips");
  summary.categories.slice(0, 8).forEach((item) => categories.append(text("span", `${expenseCategoryLabels[item.category] || item.category} ${formatExpenseMoney(item.amountMinor, item.currency)}`)));
  root.append(categories);
  if (summary.recurringCandidates) root.append(text("p", `반복 거래 ${summary.recurringCandidates}건을 정기지출 후보로 검토할 수 있습니다.`, "expense-candidate-note"));
  renderExpenseSources(root);
  renderMobileExpenseReport(root);
}

function renderExpenseSources(root) {
  const panel = text("section", "", "expense-sources-card");
  panel.append(text("h3", "데이터 출처"), text("p", "활성 출처와 확정 리포트에 꼭 필요한 출처를 별도로 관리합니다.", "muted"));
  const list = text("div", "", "expense-source-list");
  state.expenseSources.forEach((source) => {
    const form = text("form", "", "expense-source-item");
    const name = text("div");
    name.append(
      text("strong", expenseSourceLabels[source.adapter] || source.adapter),
      text("small", source.coverageStart && source.coverageEnd ? `${source.coverageStart}–${source.coverageEnd}` : "기간 정보 없음")
    );
    const active = document.createElement("input"); active.type = "checkbox"; active.checked = source.isActive;
    const activeLabel = text("label", "", "expense-checkbox"); activeLabel.append(active, document.createTextNode(" 활성"));
    const required = document.createElement("input"); required.type = "checkbox"; required.checked = source.requiredForCompleteReport;
    const requiredLabel = text("label", "", "expense-checkbox"); requiredLabel.append(required, document.createTextNode(" 확정 리포트 필수"));
    const save = text("button", "설정 저장", "secondary"); save.type = "submit";
    form.append(name, activeLabel, requiredLabel, save);
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const body = { requiredForCompleteReport: required.checked, isActive: active.checked };
      setBusy(save, true, "설정 저장");
      try {
        const updated = await expenseMutation(`/api/v1/expenses/sources/${encodeURIComponent(source.id)}`, {
          method: "PATCH",
          operation: "expense-source-update",
          version: source.version,
          resource: source.id,
          body
        });
        state.expenseSources = state.expenseSources.map((item) => item.id === updated.id ? updated : item);
        toast("지출 출처 설정을 변경했습니다.");
        await loadExpenses();
      } catch (error) {
        toast(`${error.message} 최신 출처 상태를 다시 불러와 주세요.`);
      } finally {
        setBusy(save, false, "설정 저장");
      }
    });
    list.append(form);
  });
  if (!state.expenseSources.length) list.append(text("p", "가져온 지출 출처가 없습니다.", "empty"));
  panel.append(list);
  root.append(panel);
}

function renderMobileExpenseReport(root) {
  const report = state.expenseReport;
  const panel = text("section", "", "expense-ai-card");
  const heading = text("div", "", "expense-ai-heading");
  const title = text("div");
  title.append(text("strong", "AI 지출 해설"), text("small", "수동 호출만 · 업체·상대방·개별 거래 미전송"));
  const generate = text("button", state.expenseReportRetryNeeded ? "같은 요청 다시 확인" : report ? "다시 생성" : "AI 해설 생성", "primary"); generate.type = "button";
  generate.addEventListener("click", () => void generateMobileExpenseReport(generate, false));
  heading.append(title, generate);
  if (state.expenseReportRetryNeeded) {
    const newRequest = text("button", "새 AI 요청으로 다시 시도 (새 횟수·최대 $0.05가 발생할 수 있음)", "secondary expense-ai-new-request");
    newRequest.type = "button";
    newRequest.addEventListener("click", () => void generateMobileExpenseReport(newRequest, true));
    heading.append(newRequest);
  }
  panel.append(heading, text("p", "요청당 최대 $0.05 예약 · 서울 기준 월 8회 · 지출 AI 월 $1 hard stop", "expense-ai-limit"));
  if (report) {
    const result = text("div", "", "expense-ai-result");
    result.append(text("h3", report.title), text("p", report.summary));
    [...report.observations, ...report.alerts].forEach((item) => {
      const observation = text("article");
      const citations = expenseFactCitations(report, item.factIds);
      observation.append(text("small", citations.length ? citations.join(" · ") : "확정 집계 근거"), text("strong", item.text));
      result.append(observation);
    });
    if (report.nextMonthChecks.length) {
      result.append(text("h4", "다음 달 확인"));
      const list = document.createElement("ul");
      report.nextMonthChecks.forEach((item) => list.append(text("li", item)));
      result.append(list);
    }
    const feedback = text("div", "", "expense-ai-feedback");
    feedback.append(text("span", report.helpful === null ? "이 해설이 도움됐나요?" : "평가가 저장되었습니다."));
    [[true, "도움됨"], [false, "도움 안 됨"]].forEach(([helpful, label]) => {
      const button = text("button", label, "secondary"); button.type = "button";
      button.setAttribute("aria-pressed", String(report.helpful === helpful));
      button.addEventListener("click", () => void rateMobileExpenseReport(Boolean(helpful), button));
      feedback.append(button);
    });
    result.append(feedback);
    panel.append(result);
  } else {
    panel.append(text("p", "기본 집계는 AI 호출 없이 항상 표시됩니다.", "empty"));
  }
  root.append(panel);
}

async function generateMobileExpenseReport(button, newRequest) {
  const restingLabel = newRequest
    ? "새 AI 요청으로 다시 시도 (새 횟수·최대 $0.05가 발생할 수 있음)"
    : state.expenseReportRetryNeeded ? "같은 요청 다시 확인" : "AI 해설 생성";
  setBusy(button, true, restingLabel);
  if (newRequest) discardExpenseMutation("expense-report-generate", state.expenseMonth);
  try {
    const body = { month: state.expenseMonth };
    state.expenseReport = await expenseMutation("/api/v1/expenses/reports", {
      operation: "expense-report-generate",
      resource: state.expenseMonth,
      body,
      aiConfirmation: "expense-report"
    });
    state.expenseReportRetryNeeded = false;
    renderExpenseSummary();
    toast("집계 데이터로 AI 지출 해설을 만들었습니다.");
  } catch (error) {
    state.expenseReportRetryNeeded = true;
    renderExpenseSummary();
    toast(`${error.message} 기본 집계는 계속 사용할 수 있으며 같은 요청 다시 확인은 새 횟수를 사용하지 않습니다.`);
  }
}

async function rateMobileExpenseReport(helpful, button) {
  const report = state.expenseReport;
  if (!report) return;
  setBusy(button, true, helpful ? "도움됨" : "도움 안 됨");
  try {
    const body = { helpful };
    state.expenseReport = await expenseMutation(`/api/v1/expenses/reports/${encodeURIComponent(report.reportId)}/feedback`, {
      operation: "expense-report-feedback",
      resource: report.reportId,
      body
    });
    renderExpenseSummary();
    toast("AI 해설 평가를 저장했습니다.");
  } catch (error) {
    toast(error.message);
    setBusy(button, false, helpful ? "도움됨" : "도움 안 됨");
  }
}

async function loadMoreExpenseTransactions(button) {
  if (!state.expenseTransactionsNextCursor) return;
  setBusy(button, true, "거래 더 보기");
  try {
    const page = await api(expenseQuery("/api/v1/expenses/transactions", {
      month: state.expenseMonth,
      cursor: state.expenseTransactionsNextCursor,
      limit: 50
    }));
    state.expenseTransactions.push(...page.items);
    state.expenseTransactionsNextCursor = page.nextCursor;
    renderExpenseTransactions();
  } catch (error) {
    toast(error.message);
  } finally {
    setBusy(button, false, "거래 더 보기");
  }
}

function renderExpenseTransactions() {
  const list = byId("expense-transactions-list");
  clear(list);
  byId("expense-transactions-count").textContent = `${state.expenseTransactions.length}건`;
  state.expenseTransactions.forEach((item) => {
    const card = text("article", "", `item-card expense-transaction ${item.status}`);
    const heading = text("div", "", "expense-card-heading");
    const title = text("div");
    title.append(text("strong", item.merchant || item.counterparty || "표시 이름 없음"));
    title.append(text("small", `${item.postedDate} · ${expenseKindLabels[item.kind] || item.kind}`));
    heading.append(title, text("strong", formatExpenseMoney(item.amountMinor, item.currency)));
    card.append(heading);
    card.append(text("p", `${expenseCategoryLabels[item.category] || item.category} · ${item.sourceKind || "수동"} · ${item.status}`));
    if (item.exclusionReason) card.append(text("small", item.exclusionReason, "expense-exclusion"));
    if (item.kind === "manual_recurring") {
      card.append(text("small", "수동 납부 기록은 정기지출 발생 건에서 관리합니다."));
    } else {
      renderExpenseTransactionOverride(item, card);
    }
    list.append(card);
  });
  if (!state.expenseTransactions.length) list.append(text("p", "이 달에 표시할 거래가 없습니다.", "empty"));
  show("expense-transactions-more", Boolean(state.expenseTransactionsNextCursor));
}

function renderExpenseTransactionOverride(item, card) {
  const toggle = text("button", "재분류·제외 해제", "secondary"); toggle.type = "button";
  const form = text("form", "", "expense-transaction-override hidden");
  const kind = expenseSelect(editableExpenseKindLabels, editableExpenseKindLabels[item.kind] ? item.kind : "purchase");
  const category = expenseSelect(expenseCategoryLabels, item.category);
  const kindLabel = text("label", "새 처리"); kindLabel.append(kind);
  const categoryLabel = text("label", "새 카테고리"); categoryLabel.append(category);
  const duplicate = document.createElement("select");
  const notDuplicate = text("option", "중복 아님"); notDuplicate.value = ""; duplicate.append(notDuplicate);
  const related = document.createElement("select");
  const notRelated = text("option", "연결하지 않음"); notRelated.value = ""; related.append(notRelated);
  const relatedCandidates = state.recurringMatchTransactions.filter((candidate) =>
    candidate.id !== item.id
      && candidate.currency === item.currency
      && candidate.status === "confirmed"
      && !candidate.isProvisional
      && ["purchase", "refund"].includes(candidate.kind));
  const duplicateCandidates = state.recurringMatchTransactions.filter((candidate) =>
    candidate.id !== item.id
      && candidate.currency === item.currency
      && candidate.amountMinor === item.amountMinor
      && candidate.status === "confirmed"
      && !candidate.isProvisional
      && !candidate.duplicateOfEventId);
  relatedCandidates.forEach((candidate) => {
    const label = `${candidate.postedDate} · ${candidate.merchant || candidate.counterparty || "표시 이름 없음"} · ${formatExpenseMoney(candidate.amountMinor, candidate.currency)}`;
    const relatedOption = text("option", label); relatedOption.value = candidate.id; related.append(relatedOption);
  });
  duplicateCandidates.forEach((candidate) => {
    const label = `${candidate.postedDate} · ${candidate.merchant || candidate.counterparty || "표시 이름 없음"} · ${formatExpenseMoney(candidate.amountMinor, candidate.currency)}`;
    const duplicateOption = text("option", label); duplicateOption.value = candidate.id; duplicate.append(duplicateOption);
  });
  if (item.duplicateOfEventId && !duplicateCandidates.some((candidate) => candidate.id === item.duplicateOfEventId)) {
    const currentDuplicate = text("option", "현재 중복 대상"); currentDuplicate.value = item.duplicateOfEventId; duplicate.append(currentDuplicate);
  }
  if (item.relatedEventId && !relatedCandidates.some((candidate) => candidate.id === item.relatedEventId)) {
    const currentRelated = text("option", "현재 연결 거래"); currentRelated.value = item.relatedEventId; related.append(currentRelated);
  }
  duplicate.value = item.duplicateOfEventId || "";
  related.value = item.relatedEventId || "";
  const duplicateLabel = text("label", "중복 대상 (선택)"); duplicateLabel.append(duplicate);
  const relatedLabel = text("label", "정산·연결 대상 (선택)"); relatedLabel.append(related);
  const personal = document.createElement("input");
  personal.type = "number"; personal.min = "1"; personal.max = String(Math.abs(item.amountMinor)); personal.step = "1"; personal.placeholder = "전체 금액";
  personal.value = item.personalAmountMinor === null ? "" : String(item.personalAmountMinor);
  const personalLabel = text("label", "내 부담액 (minor unit·선택)"); personalLabel.append(personal);
  const clearPersonal = document.createElement("input"); clearPersonal.type = "checkbox";
  const clearRelated = document.createElement("input"); clearRelated.type = "checkbox";
  const clearPersonalLabel = item.personalAmountMinor === null ? null : text("label", "", "expense-checkbox");
  if (clearPersonalLabel) clearPersonalLabel.append(clearPersonal, document.createTextNode(` 기존 내 부담액 ${formatExpenseMoney(item.personalAmountMinor, item.currency)} 명시적으로 해제`));
  const clearRelatedLabel = item.relatedEventId ? text("label", "", "expense-checkbox") : null;
  if (clearRelatedLabel) clearRelatedLabel.append(clearRelated, document.createTextNode(" 기존 정산 연결 명시적으로 해제"));
  const syncAlternatives = () => {
    const settlementKind = ["settlement_received", "settlement_sent"].includes(kind.value);
    related.disabled = !settlementKind || Boolean(duplicate.value) || clearRelated.checked;
    personal.disabled = kind.value !== "purchase" || Boolean(duplicate.value) || clearPersonal.checked;
  };
  kind.addEventListener("change", () => {
    if (!["settlement_received", "settlement_sent"].includes(kind.value)) related.value = item.relatedEventId || "";
    if (kind.value !== "purchase") personal.value = item.personalAmountMinor === null ? "" : String(item.personalAmountMinor);
    syncAlternatives();
  });
  duplicate.addEventListener("change", () => {
    if (duplicate.value) {
      related.value = ""; personal.value = "";
      clearRelated.checked = Boolean(item.relatedEventId);
      clearPersonal.checked = item.personalAmountMinor !== null;
    }
    syncAlternatives();
  });
  related.addEventListener("change", () => {
    clearRelated.checked = false;
    if (related.value) {
      duplicate.value = ""; personal.value = "";
      clearPersonal.checked = item.personalAmountMinor !== null;
    }
    syncAlternatives();
  });
  personal.addEventListener("input", () => {
    clearPersonal.checked = false;
    if (personal.value) {
      duplicate.value = ""; related.value = "";
      clearRelated.checked = Boolean(item.relatedEventId);
    }
    syncAlternatives();
  });
  clearPersonal.addEventListener("change", () => {
    personal.value = clearPersonal.checked ? "" : item.personalAmountMinor === null ? "" : String(item.personalAmountMinor);
    syncAlternatives();
  });
  clearRelated.addEventListener("change", () => {
    related.value = clearRelated.checked ? "" : item.relatedEventId || "";
    syncAlternatives();
  });
  syncAlternatives();
  const createRule = document.createElement("input"); createRule.type = "checkbox";
  const canCreateRule = Boolean(item.merchant && item.paymentMethodFingerprint);
  const rule = canCreateRule
    ? text("label", "", "expense-checkbox")
    : text("small", "업체와 결제수단을 모두 확인할 수 있는 거래만 자동 분류 규칙을 만들 수 있습니다.");
  if (canCreateRule) rule.append(createRule, document.createTextNode(" 앞으로 같은 업체에 적용"));
  const actions = text("div", "", "expense-form-actions");
  const cancel = text("button", "재분류 취소", "secondary"); cancel.type = "button";
  const save = text("button", "재분류 저장", "primary"); save.type = "submit";
  actions.append(cancel, save);
  form.append(
    text("p", item.status === "excluded"
      ? "구매·환불·정산으로 다시 분류하면 자동 제외를 해제할 수 있습니다."
      : "제외 사유를 포함해 분류 결정을 다시 저장할 수 있습니다."),
    kindLabel,
    categoryLabel,
    duplicateLabel,
    relatedLabel,
    personalLabel,
    ...(clearPersonalLabel ? [clearPersonalLabel] : []),
    ...(clearRelatedLabel ? [clearRelatedLabel] : []),
    rule,
    actions
  );
  toggle.addEventListener("click", () => {
    form.classList.remove("hidden");
    toggle.classList.add("hidden");
  });
  cancel.addEventListener("click", () => {
    discardExpenseMutation("expense-transaction-override", item.id);
    form.classList.add("hidden");
    toggle.classList.remove("hidden");
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const body = {
      kind: kind.value,
      category: category.value,
      duplicateOfEventId: duplicate.value || null,
      relatedEventId: clearRelated.checked || related.value === (item.relatedEventId || "") ? null : related.value || null,
      personalAmountMinor: clearPersonal.checked || personal.value === (item.personalAmountMinor === null ? "" : String(item.personalAmountMinor)) ? null : personal.value === "" ? null : Number(personal.value),
      clearPersonalAmount: clearPersonal.checked,
      clearRelatedEvent: clearRelated.checked,
      createRule: canCreateRule ? createRule.checked : false
    };
    setBusy(save, true, "재분류 저장");
    try {
      await expenseMutation(`/api/v1/expenses/transactions/${encodeURIComponent(item.id)}`, {
        method: "PATCH",
        operation: "expense-transaction-override",
        version: item.version,
        resource: item.id,
        body
      });
      toast(item.status === "excluded" ? "자동 제외를 해제하거나 새 분류로 저장했습니다." : "거래 분류를 변경했습니다.");
      await loadExpenses();
    } catch (error) {
      toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
    } finally {
      setBusy(save, false, "재분류 저장");
    }
  });
  card.append(toggle, form);
}

function expenseSelect(options, selected) {
  const select = document.createElement("select");
  Object.entries(options).forEach(([value, label]) => {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    option.selected = value === selected;
    select.append(option);
  });
  return select;
}

function renderExpenseReviews() {
  const list = byId("expense-reviews-list");
  clear(list);
  const purchaseCategoryCount = state.expenseReviews
    .filter((review) => review.reason === "category_confirmation").length;
  const visibleReviews = state.showExpensePurchaseCategories
    ? state.expenseReviews
    : state.expenseReviews.filter((review) => review.reason !== "category_confirmation");
  byId("expense-reviews-count").textContent = `${visibleReviews.length}건`;
  const categoryToggle = byId("expense-review-categories-toggle");
  categoryToggle.textContent = state.showExpensePurchaseCategories
    ? "선택 분류 숨기기"
    : `선택 분류 ${purchaseCategoryCount}${state.expenseCategoryReviewsNextCursor ? "+" : ""}건 보기`;
  categoryToggle.classList.toggle("hidden", purchaseCategoryCount === 0);
  visibleReviews.forEach((review) => {
    const transaction = review.transaction;
    const heading = text("div", "", "expense-card-heading");
    const title = text("div");
    title.append(text("strong", transaction.merchant || transaction.counterparty || "표시 이름 없음"));
    title.append(text("small", `${transaction.postedDate} · ${expenseReviewReasonLabels[review.reason] || review.reason}`));
    heading.append(title, text("strong", formatExpenseMoney(transaction.amountMinor, transaction.currency)));

    if (review.reason === "recurring_match_candidate") {
      const card = text("article", "", "item-card expense-review-card expense-review-card--candidate");
      const occurrence = review.recurringExpenseId
        ? state.recurringExpenseOccurrences.find((item) => item.recurringExpenseId === review.recurringExpenseId)
        : null;
      const matched = occurrence && ["paid", "matched"].includes(occurrence.status);
      card.append(heading);
      card.append(text(
        "p",
        occurrence
          ? `${occurrence.name}의 ${occurrence.dueDate} 발생 건과 조건이 일치합니다.`
          : "연결할 정기지출 발생 건을 다시 불러와 주세요."
      ));
      const autoMatch = document.createElement("input");
      autoMatch.type = "checkbox";
      autoMatch.checked = false;
      autoMatch.disabled = !occurrence || Boolean(matched);
      const autoMatchLabel = text("label", "", "expense-checkbox");
      autoMatchLabel.append(autoMatch, document.createTextNode(" 앞으로 고신뢰 거래도 자동 연결"));
      card.append(autoMatchLabel, text("small", "처음 연결을 확인하면 업체·결제수단을 안전하게 학습하며, 자동 연결은 별도로 동의한 경우에만 켭니다.", "expense-payment-learning"));
      const actions = text("div", "", "expense-form-actions");
      const reject = text("button", "이번에는 연결하지 않음", "secondary");
      reject.type = "button";
      reject.addEventListener("click", () => void resolveRecurringCandidateReview(review, reject, "이번에는 연결하지 않음"));
      const connect = text("button", "정기지출에 연결", "primary");
      connect.type = "button";
      connect.disabled = !occurrence || Boolean(matched);
      connect.addEventListener("click", () => {
        if (occurrence) void matchRecurringOccurrence(occurrence, transaction.id, autoMatch.checked, connect, "정기지출에 연결");
      });
      actions.append(reject, connect);
      card.append(actions);
      list.append(card);
      return;
    }

    if (review.reason === "recurring_registration_candidate") {
      const card = text("article", "", "item-card expense-review-card expense-review-card--candidate");
      card.append(
        heading,
        text("p", "반복 간격과 금액이 안정적인 거래입니다. 거래 정보로 정기지출 입력을 채운 뒤 내용을 확인하세요.")
      );
      const actions = text("div", "", "expense-form-actions");
      const reject = text("button", "등록하지 않음", "secondary");
      reject.type = "button";
      reject.addEventListener("click", () => void resolveRecurringCandidateReview(review, reject, "등록하지 않음"));
      const register = text("button", "정기지출로 등록", "primary");
      register.type = "button";
      register.addEventListener("click", () => {
        selectExpenseView("recurring");
        openRecurringExpenseForm(null, review);
      });
      actions.append(reject, register);
      card.append(actions);
      list.append(card);
      return;
    }

    const form = text("form", "", "item-card expense-review-card");
    form.append(heading);
    const kind = expenseSelect(
      editableExpenseKindLabels,
      editableExpenseKindLabels[review.suggestedKind] ? review.suggestedKind : "purchase"
    );
    const category = expenseSelect(expenseCategoryLabels, review.suggestedCategory || "other");
    const kindLabel = text("label", "처리"); kindLabel.append(kind);
    const categoryLabel = text("label", "카테고리"); categoryLabel.append(category);
    let duplicateEvent = null;
    let duplicateLabel = null;
    if (review.reason === "ambiguous_mirror") {
      duplicateEvent = document.createElement("select");
      const notDuplicate = text("option", "중복 아님");
      notDuplicate.value = "";
      duplicateEvent.append(notDuplicate);
      const candidates = state.recurringMatchTransactions.filter((item) =>
        item.id !== transaction.id
        && item.currency === transaction.currency
        && item.amountMinor === transaction.amountMinor
        && item.status === "confirmed"
        && !item.isProvisional
        && !item.duplicateOfEventId
      );
      const suggestedMissing = review.suggestedDuplicateOfEventId
        && !candidates.some((item) => item.id === review.suggestedDuplicateOfEventId);
      if (suggestedMissing) {
        const suggested = text("option", "제안된 동일 거래");
        suggested.value = review.suggestedDuplicateOfEventId;
        duplicateEvent.append(suggested);
      }
      candidates.forEach((item) => {
        const option = text("option", `${item.postedDate} · ${item.merchant || item.counterparty || "표시 이름 없음"} · ${formatExpenseMoney(item.amountMinor, item.currency)}`);
        option.value = item.id;
        duplicateEvent.append(option);
      });
      duplicateEvent.value = review.suggestedDuplicateOfEventId || "";
      duplicateLabel = text("label", "중복 대상");
      duplicateLabel.append(duplicateEvent, text("small", "서버 제안을 수락하거나 같은 금액·통화의 다른 거래를 선택합니다."));
    }
    const relatedEvent = document.createElement("select");
    const noRelatedEvent = text("option", "연결하지 않음"); noRelatedEvent.value = ""; relatedEvent.append(noRelatedEvent);
    state.recurringMatchTransactions
      .filter((item) => item.id !== transaction.id
        && item.currency === transaction.currency
        && item.status === "confirmed"
        && !item.isProvisional
        && ["purchase", "refund"].includes(item.kind))
      .forEach((item) => {
        const option = text("option", `${item.postedDate} · ${item.merchant || item.counterparty || "표시 이름 없음"} · ${formatExpenseMoney(item.amountMinor, item.currency)}`);
        option.value = item.id;
        relatedEvent.append(option);
      });
    const relatedLabel = text("label", "정산·연결 대상 (선택)"); relatedLabel.append(relatedEvent);
    const personalAmount = document.createElement("input");
    personalAmount.type = "number"; personalAmount.min = "1"; personalAmount.max = String(Math.abs(transaction.amountMinor)); personalAmount.step = "1"; personalAmount.placeholder = "전체 금액";
    const syncAlternativeDecision = () => {
      const duplicated = Boolean(duplicateEvent?.value);
      relatedEvent.disabled = duplicated || !["settlement_received", "settlement_sent"].includes(kind.value);
      personalAmount.disabled = duplicated || kind.value !== "purchase";
    };
    kind.addEventListener("change", () => {
      if (!["settlement_received", "settlement_sent"].includes(kind.value)) relatedEvent.value = "";
      if (kind.value !== "purchase") personalAmount.value = "";
      syncAlternativeDecision();
    });
    duplicateEvent?.addEventListener("change", () => {
      if (duplicateEvent?.value) {
        relatedEvent.value = "";
        personalAmount.value = "";
      }
      syncAlternativeDecision();
    });
    relatedEvent.addEventListener("change", () => {
      if (relatedEvent.value) {
        personalAmount.value = "";
        if (duplicateEvent) duplicateEvent.value = "";
      }
      syncAlternativeDecision();
    });
    personalAmount.addEventListener("input", () => {
      if (personalAmount.value) {
        relatedEvent.value = "";
        if (duplicateEvent) duplicateEvent.value = "";
      }
      syncAlternativeDecision();
    });
    syncAlternativeDecision();
    relatedLabel.append(text("small", "정산 처리에는 앞뒤 달의 확정 구매·환불만 표시합니다."));
    const personalAmountLabel = text("label", "내 부담액 (minor unit·선택)"); personalAmountLabel.append(personalAmount, text("small", "구매 처리에서만 지정하며 중복·연결 대상과 함께 저장할 수 없습니다."));
    const createRule = document.createElement("input"); createRule.type = "checkbox";
    const canCreateRule = Boolean(transaction.merchant && transaction.paymentMethodFingerprint);
    createRule.checked = review.reason === "category_confirmation" && canCreateRule;
    const ruleLabel = canCreateRule
      ? text("label", "", "expense-checkbox")
      : text("small", "업체와 결제수단을 모두 확인할 수 있는 거래만 자동 분류 규칙을 만들 수 있습니다.");
    if (canCreateRule) ruleLabel.append(
      createRule,
      document.createTextNode(review.reason === "category_confirmation"
        ? " 이 거래와 안전하게 일치하는 같은 업체·결제수단에 적용"
        : " 앞으로 같은 업체에 적용")
    );
    const save = text("button", "결정 저장", "primary"); save.type = "submit";
    form.append(kindLabel, categoryLabel);
    if (duplicateLabel) form.append(duplicateLabel);
    form.append(relatedLabel, personalAmountLabel, ruleLabel, save);
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      setBusy(save, true, "결정 저장");
      try {
        const body = {
          kind: kind.value,
          category: category.value,
          duplicateOfEventId: duplicateEvent?.value || null,
          relatedEventId: relatedEvent.value || null,
          personalAmountMinor: personalAmount.value === "" ? null : Number(personalAmount.value),
          createRule: canCreateRule ? createRule.checked : false
        };
        await expenseMutation(`/api/v1/expenses/reviews/${encodeURIComponent(review.id)}/resolve`, {
          operation: "expense-review-resolve",
          version: review.version,
          resource: review.id,
          body
        });
        toast(body.createRule
          ? "이 거래와 서버에서 안전하게 일치한 거래 및 향후 규칙에 적용했습니다."
          : "거래 검토 결정을 저장했습니다.");
        await loadExpenses();
      } catch (error) {
        toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
      } finally {
        setBusy(save, false, "결정 저장");
      }
    });
    list.append(form);
  });
  if (!visibleReviews.length) list.append(text(
    "p",
    purchaseCategoryCount > 0
      ? "필수 확인 거래가 없습니다. 구매 분류는 선택 사항이며 합계에는 이미 반영되었습니다."
      : "확인할 거래가 없습니다.",
    "empty"
  ));
  show("expense-reviews-more", Boolean(state.expenseReviewsNextCursor));
  show("expense-category-reviews-more", Boolean(
    state.showExpensePurchaseCategories && state.expenseCategoryReviewsNextCursor
  ));
}

async function loadMoreExpenseReviews(button) {
  if (!state.expenseReviewsNextCursor) return;
  setBusy(button, true, "검토 더 보기");
  try {
    const page = await api(expenseQuery("/api/v1/expenses/reviews", {
      month: state.expenseMonth,
      status: "pending",
      scope: "required",
      cursor: state.expenseReviewsNextCursor,
      limit: 100
    }));
    const merged = new Map(state.expenseReviews.map((review) => [review.id, review]));
    page.items.forEach((review) => merged.set(review.id, review));
    state.expenseReviews = [...merged.values()];
    state.expenseReviewsNextCursor = page.nextCursor;
    renderExpenseReviews();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "검토 더 보기");
  }
}

async function loadMoreExpenseCategoryReviews(button) {
  if (!state.expenseCategoryReviewsNextCursor) return;
  setBusy(button, true, "선택 분류 더 보기");
  try {
    const page = await api(expenseQuery("/api/v1/expenses/reviews", {
      month: state.expenseMonth,
      status: "pending",
      scope: "category_confirmation",
      cursor: state.expenseCategoryReviewsNextCursor,
      limit: 100
    }));
    const merged = new Map(state.expenseReviews.map((review) => [review.id, review]));
    page.items.forEach((review) => merged.set(review.id, review));
    state.expenseReviews = [...merged.values()];
    state.expenseCategoryReviewsNextCursor = page.nextCursor;
    renderExpenseReviews();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "선택 분류 더 보기");
  }
}

async function resolveRecurringCandidateReview(review, button, label) {
  setBusy(button, true, label);
  try {
    const body = {
      kind: "purchase",
      category: review.suggestedCategory || (review.transaction.category === "unconfirmed" ? "other" : review.transaction.category),
      duplicateOfEventId: null,
      relatedEventId: null,
      personalAmountMinor: null,
      createRule: false
    };
    await expenseMutation(`/api/v1/expenses/reviews/${encodeURIComponent(review.id)}/resolve`, {
      operation: "expense-review-resolve",
      version: review.version,
      resource: review.id,
      body
    });
    toast("거래 검토 결정을 저장했습니다.");
    await loadExpenses();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, label);
  }
}

function renderRecurringExpenses() {
  const occurrences = byId("expense-occurrences-list");
  clear(occurrences);
  state.recurringExpenseOccurrences.forEach((occurrence) => {
    const card = text("article", "", `item-card expense-occurrence ${occurrence.status}`);
    const heading = text("div", "", "expense-card-heading");
    const title = text("div");
    title.append(text("strong", occurrence.name));
    title.append(text("small", `${occurrence.dueDate} · ${recurringOccurrenceLabels[occurrence.status] || occurrence.status}${occurrence.amountChanged ? " · 금액 변동" : ""}`));
    heading.append(title, text("strong", formatExpenseMoney(occurrence.actualAmountMinor ?? occurrence.expectedAmountMinor, occurrence.currency)));
    card.append(heading);
    if (!["paid", "matched"].includes(occurrence.status)) {
      const actions = text("div", "", "expense-occurrence-actions");
      const paid = text("button", "납부 완료", "secondary"); paid.type = "button";
      paid.addEventListener("click", () => void confirmRecurringPaid(occurrence, paid));
      const connect = text("button", "거래 연결", "secondary"); connect.type = "button";
      const matchForm = text("form", "", "expense-match-form hidden");
      const selectLabel = text("label", "이 달의 실제 거래");
      const transaction = document.createElement("select");
      transaction.required = true;
      const placeholder = text("option", "연결할 거래를 선택하세요");
      placeholder.value = "";
      transaction.append(placeholder);
      const candidates = state.recurringMatchTransactions
        .filter((item) => item.currency === occurrence.currency
          && item.status !== "excluded"
          && !item.isProvisional
          && ["purchase", "external_transfer", "unknown_p2p"].includes(item.kind)
          && Math.abs(Date.parse(`${item.postedDate}T00:00:00Z`) - Date.parse(`${occurrence.dueDate}T00:00:00Z`)) <= 5 * 86_400_000)
        .sort((left, right) => Number(Boolean(right.pendingReviewId)) - Number(Boolean(left.pendingReviewId))
          || left.postedDate.localeCompare(right.postedDate)
          || left.id.localeCompare(right.id));
      candidates.forEach((item) => {
        const option = text("option", `${item.postedDate} · ${item.merchant || item.counterparty || "표시 이름 없음"} · ${formatExpenseMoney(item.amountMinor, item.currency)}${item.pendingReviewId ? " · 확인 필요" : ""}`);
        option.value = item.id;
        transaction.append(option);
      });
      selectLabel.append(transaction, text("small", "같은 통화의 제외되지 않은 거래만 표시합니다."));
      const autoMatch = document.createElement("input"); autoMatch.type = "checkbox"; autoMatch.checked = false;
      const autoMatchLabel = text("label", "", "expense-checkbox");
      autoMatchLabel.append(autoMatch, document.createTextNode(" 이 연결을 확인했고 이후 고신뢰 거래도 자동 연결"));
      const submit = text("button", "거래 연결 확인", "primary"); submit.type = "submit";
      submit.disabled = true;
      transaction.addEventListener("change", () => { submit.disabled = !transaction.value; });
      matchForm.append(selectLabel, autoMatchLabel, text("small", "처음 동의한 연결에서 업체·결제수단을 안전하게 학습합니다.", "expense-payment-learning"), submit);
      matchForm.addEventListener("submit", async (event) => {
        event.preventDefault();
        await matchRecurringOccurrence(occurrence, transaction.value, autoMatch.checked, submit);
      });
      connect.addEventListener("click", () => matchForm.classList.toggle("hidden"));
      actions.append(paid, connect);
      card.append(actions, matchForm);
    }
    occurrences.append(card);
  });
  if (!state.recurringExpenseOccurrences.length) occurrences.append(text("p", "이 달에 예정된 정기지출이 없습니다.", "empty"));

  const items = byId("expense-recurring-list");
  clear(items);
  state.recurringExpenses.forEach((item) => {
    const button = text("button", "", "item-card expense-recurring-item");
    button.type = "button";
    const body = text("span");
    body.append(text("strong", item.name));
    body.append(text("small", `${expenseCategoryLabels[item.category] || item.category} · ${item.intervalMonths === 1 ? "매월" : `${item.intervalMonths}개월마다`} · ${item.status}`));
    button.append(body, text("strong", formatExpenseMoney(item.amountMinor, item.currency)));
    button.addEventListener("click", () => openRecurringExpenseForm(item));
    items.append(button);
  });
  if (!state.recurringExpenses.length) items.append(text("p", "등록한 정기지출이 없습니다.", "empty"));
}

async function confirmRecurringPaid(occurrence, button) {
  setBusy(button, true, "납부 완료");
  try {
    const today = todaySeoul();
    const body = { amountMinor: null, paidDate: occurrence.dueDate < today ? occurrence.dueDate : today };
    await expenseMutation(`/api/v1/expenses/recurring/occurrences/${encodeURIComponent(occurrence.occurrenceKey)}/confirm-paid`, {
      operation: "recurring-expense-confirm-paid",
      version: occurrence.version,
      resource: occurrence.occurrenceKey,
      body
    });
    toast("납부 완료를 기록했습니다.");
    await loadExpenses();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "납부 완료");
  }
}

async function matchRecurringOccurrence(occurrence, eventId, enableFutureAutoMatch, button, buttonLabel = "거래 연결 확인") {
  if (!eventId) return;
  setBusy(button, true, buttonLabel);
  try {
    const body = { eventId, enableFutureAutoMatch };
    await expenseMutation(`/api/v1/expenses/recurring/occurrences/${encodeURIComponent(occurrence.occurrenceKey)}/match`, {
      operation: "recurring-expense-match",
      version: occurrence.version,
      resource: occurrence.occurrenceKey,
      body
    });
    toast(enableFutureAutoMatch
      ? "거래를 연결하고 업체·결제수단의 이후 자동 연결을 켰습니다."
      : "실제 거래를 정기지출에 연결했습니다.");
    await loadExpenses();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, buttonLabel);
  }
}

function selectExpenseView(view) {
  document.querySelectorAll("[data-expense-view]").forEach((button) => {
    if (button.dataset.expenseView === view) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  });
  document.querySelectorAll(".expense-view").forEach((panel) => panel.classList.add("hidden"));
  byId(`expense-view-${view}`).classList.remove("hidden");
}

function populateExpenseCategories() {
  const select = byId("expense-recurring-category");
  if (select.options.length) return;
  Object.entries(expenseCategoryLabels).filter(([value]) => value !== "unconfirmed").forEach(([value, label]) => {
    const option = document.createElement("option"); option.value = value; option.textContent = label; select.append(option);
  });
}

function recurringCategoryFromReview(review) {
  const suggested = review?.suggestedCategory;
  if (suggested && suggested !== "unconfirmed") return suggested;
  const current = review?.transaction?.category;
  return current && current !== "unconfirmed" ? current : "other";
}

function openRecurringExpenseForm(item = null, review = null) {
  populateExpenseCategories();
  state.editingRecurringExpense = item;
  state.recurringRegistrationReview = review;
  state.recurringRegistrationCreated = null;
  const transaction = review?.transaction || null;
  const displayName = transaction?.merchant || transaction?.counterparty || "정기지출";
  byId("expense-recurring-form-title").textContent = review ? "반복 거래 후보 등록" : item ? "정기지출 편집" : "정기지출 추가";
  byId("expense-recurring-name").value = item?.name || (review ? displayName : "");
  byId("expense-recurring-category").value = item?.category || (review ? recurringCategoryFromReview(review) : "ott_subscriptions");
  byId("expense-recurring-merchant").value = item?.vendor || transaction?.merchant || "";
  byId("expense-recurring-amount").value = item?.amountMinor ?? transaction?.amountMinor ?? "";
  byId("expense-recurring-currency").value = item?.currency || transaction?.currency || "KRW";
  byId("expense-recurring-amount-type").value = item?.amountKind || "fixed";
  byId("expense-recurring-interval").value = item?.intervalMonths || 1;
  byId("expense-recurring-payment-method").value = item?.paymentMethodFingerprint || transaction?.paymentMethodFingerprint || "";
  byId("expense-recurring-due-rule").value = item?.dueRule || "specific_day";
  byId("expense-recurring-due-day").value = item?.dueDay || Number((transaction?.postedDate || todaySeoul()).slice(8, 10));
  byId("expense-recurring-start").value = item?.startDate || transaction?.postedDate || todaySeoul();
  byId("expense-recurring-end").value = item?.endDate || "";
  byId("expense-recurring-reminder").value = item?.reminderDays ?? 7;
  byId("expense-recurring-status").value = item?.status || "active";
  byId("expense-recurring-note").value = item?.memo || (review ? "반복 거래 후보에서 등록" : "");
  byId("expense-recurring-auto-match").checked = Boolean(item?.autoMatchEnabled);
  byId("expense-recurring-auto-match").disabled = !item?.autoMatchEnabled;
  byId("expense-recurring-payment-method-note").textContent = byId("expense-recurring-payment-method").value
    ? "가져온 거래의 결제수단을 내부 식별값으로 연결합니다. 원문 카드·계좌번호는 표시하거나 저장하지 않습니다."
    : "첫 실제 거래 연결을 확인하면 업체·결제수단을 안전하게 학습합니다. 원문 카드·계좌번호는 표시하거나 저장하지 않습니다.";
  show("expense-recurring-registration-note", Boolean(review));
  byId("expense-recurring-registration-note").textContent = review
    ? "거래의 업체·금액·통화·결제일·결제수단을 내부에서 채웠습니다. 내용을 확인한 뒤 저장하면 후보 검토도 함께 완료됩니다."
    : "";
  show("expense-recurring-delete", Boolean(item));
  show("expense-recurring-form", true);
  updateRecurringSaveLabel();
  updateRecurringDueFields();
  byId("expense-recurring-form").scrollIntoView({ behavior: "smooth", block: "start" });
}

function closeRecurringExpenseForm() {
  const item = state.editingRecurringExpense;
  const review = state.recurringRegistrationReview;
  if (item) {
    discardExpenseMutation("recurring-expense-update", item.id);
    discardExpenseMutation("recurring-expense-delete", item.id);
  } else {
    discardExpenseMutation("recurring-expense-create", review ? `registration:${review.id}` : "recurring-form");
  }
  if (review) discardExpenseMutation("expense-review-resolve", review.id);
  state.editingRecurringExpense = null;
  state.recurringRegistrationReview = null;
  state.recurringRegistrationCreated = null;
  show("expense-recurring-form", false);
}

function updateRecurringSaveLabel() {
  const button = byId("expense-recurring-save");
  button.textContent = state.recurringRegistrationReview
    ? state.recurringRegistrationCreated ? "검토 완료 다시 시도" : "등록하고 검토 완료"
    : "저장";
}

function updateRecurringDueFields() {
  const specific = byId("expense-recurring-due-rule").value === "specific_day";
  show("expense-recurring-due-day-field", specific);
  byId("expense-recurring-due-day").required = specific;
}

function recurringExpenseBody(includeEffectiveMonth) {
  const paymentMethodFingerprint = byId("expense-recurring-payment-method").value.trim();
  const name = byId("expense-recurring-name").value.trim();
  const vendor = byId("expense-recurring-merchant").value.trim();
  const memo = byId("expense-recurring-note").value.trim();
  if (name.length > 120 || vendor.length > 200 || memo.length > 500) {
    throw new Error("이름 120자, 업체 200자, 메모 500자 이내로 입력해 주세요.");
  }
  const body = {
    name,
    category: byId("expense-recurring-category").value,
    vendor: vendor || null,
    amountMinor: Number(byId("expense-recurring-amount").value),
    currency: byId("expense-recurring-currency").value.trim().toUpperCase(),
    paymentMethodFingerprint: paymentMethodFingerprint || null,
    startDate: byId("expense-recurring-start").value,
    endDate: byId("expense-recurring-end").value || null,
    memo: memo || null,
    reminderDays: Number(byId("expense-recurring-reminder").value),
    amountKind: byId("expense-recurring-amount-type").value,
    intervalMonths: Number(byId("expense-recurring-interval").value),
    dueRule: byId("expense-recurring-due-rule").value,
    dueDay: byId("expense-recurring-due-rule").value === "specific_day" ? Number(byId("expense-recurring-due-day").value) : null,
    status: byId("expense-recurring-status").value
  };
  if (includeEffectiveMonth) {
    const currentMonth = todaySeoul().slice(0, 7);
    body.effectiveFromMonth = `${currentMonth}-01`;
    body.autoMatchEnabled = byId("expense-recurring-auto-match").checked;
  }
  return body;
}

function sameRecurringExpenseInput(item, input) {
  return item.name === input.name
    && item.category === input.category
    && item.vendor === input.vendor
    && item.amountMinor === input.amountMinor
    && item.currency === input.currency
    && item.paymentMethodFingerprint === input.paymentMethodFingerprint
    && item.startDate === input.startDate
    && item.endDate === input.endDate
    && item.memo === input.memo
    && item.reminderDays === input.reminderDays
    && item.amountKind === input.amountKind
    && item.intervalMonths === input.intervalMonths
    && item.dueRule === input.dueRule
    && item.dueDay === input.dueDay
    && item.status === input.status;
}

document.querySelectorAll("[data-expense-view]").forEach((button) => {
  button.addEventListener("click", () => selectExpenseView(button.dataset.expenseView));
});
byId("expense-due-open").addEventListener("click", () => selectTab("expenses"));
byId("expense-refresh").addEventListener("click", () => void loadExpenses());
byId("expense-prev").addEventListener("click", () => void loadExpenses(shiftMonth(state.expenseMonth || todaySeoul().slice(0, 7), -1)));
byId("expense-next").addEventListener("click", () => void loadExpenses(shiftMonth(state.expenseMonth || todaySeoul().slice(0, 7), 1)));
byId("expense-transactions-more").addEventListener("click", (event) => void loadMoreExpenseTransactions(event.currentTarget));
byId("expense-reviews-more").addEventListener("click", (event) => void loadMoreExpenseReviews(event.currentTarget));
byId("expense-category-reviews-more").addEventListener("click", (event) => void loadMoreExpenseCategoryReviews(event.currentTarget));
byId("expense-review-categories-toggle").addEventListener("click", () => {
  state.showExpensePurchaseCategories = !state.showExpensePurchaseCategories;
  renderExpenseReviews();
});
byId("expense-recurring-add").addEventListener("click", () => openRecurringExpenseForm());
byId("expense-recurring-form-close").addEventListener("click", closeRecurringExpenseForm);
byId("expense-recurring-due-rule").addEventListener("change", updateRecurringDueFields);

byId("expense-recurring-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const item = state.editingRecurringExpense;
  const registrationReview = state.recurringRegistrationReview;
  const button = byId("expense-recurring-save");
  const restingLabel = registrationReview
    ? state.recurringRegistrationCreated ? "검토 완료 다시 시도" : "등록하고 검토 완료"
    : "저장";
  setBusy(button, true, restingLabel);
  try {
    const body = recurringExpenseBody(Boolean(item));
    if (registrationReview && !item) {
      const latest = await api("/api/v1/expenses/recurring");
      let registered = state.recurringRegistrationCreated
        || latest.find((candidate) => sameRecurringExpenseInput(candidate, body))
        || null;
      if (!registered) {
        registered = await expenseMutation("/api/v1/expenses/recurring", {
          operation: "recurring-expense-create",
          resource: `registration:${registrationReview.id}`,
          body
        });
      }
      state.recurringRegistrationCreated = registered;
      updateRecurringSaveLabel();
      try {
        const resolution = {
          kind: "purchase",
          category: body.category,
          duplicateOfEventId: null,
          relatedEventId: null,
          personalAmountMinor: null,
          createRule: false
        };
        await expenseMutation(`/api/v1/expenses/reviews/${encodeURIComponent(registrationReview.id)}/resolve`, {
          operation: "expense-review-resolve",
          version: registrationReview.version,
          resource: registrationReview.id,
          body: resolution
        });
      } catch (error) {
        byId("expense-recurring-registration-note").textContent = "정기지출 항목은 등록되었습니다. 후보 검토 완료만 다시 시도하면 중복 항목을 만들지 않습니다.";
        toast(`${error.message} 등록한 항목은 유지했으며 검토 완료만 다시 시도할 수 있습니다.`);
        await loadExpenses();
        if (!state.expenseReviews.some((review) => review.id === registrationReview.id)) {
          closeRecurringExpenseForm();
          toast("정기지출을 등록하고 후보 검토를 완료했습니다.");
        }
        return;
      }
      closeRecurringExpenseForm();
      toast("정기지출을 등록하고 후보 검토를 완료했습니다.");
      await loadExpenses();
      return;
    }
    await expenseMutation(item ? `/api/v1/expenses/recurring/${encodeURIComponent(item.id)}` : "/api/v1/expenses/recurring", {
      method: item ? "PATCH" : "POST",
      operation: item ? "recurring-expense-update" : "recurring-expense-create",
      version: item?.version ?? null,
      resource: item?.id ?? "recurring-form",
      body
    });
    closeRecurringExpenseForm();
    toast(item ? "정기지출을 변경했습니다." : "정기지출을 등록했습니다.");
    await loadExpenses();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    const label = state.recurringRegistrationReview
      ? state.recurringRegistrationCreated ? "검토 완료 다시 시도" : "등록하고 검토 완료"
      : "저장";
    setBusy(button, false, label);
  }
});

byId("expense-recurring-delete").addEventListener("click", async (event) => {
  const item = state.editingRecurringExpense;
  if (!item || !window.confirm(`‘${item.name}’ 정기지출을 삭제할까요?`)) return;
  const button = event.currentTarget;
  setBusy(button, true, "삭제");
  try {
    await expenseMutation(`/api/v1/expenses/recurring/${encodeURIComponent(item.id)}`, {
      method: "DELETE",
      operation: "recurring-expense-delete",
      version: item.version,
      resource: item.id
    });
    closeRecurringExpenseForm();
    toast("정기지출을 삭제했습니다.");
    await loadExpenses();
  } catch (error) {
    toast(`${error.message} 자동으로 다시 시도하지 않았습니다.`);
  } finally {
    setBusy(button, false, "삭제");
  }
});

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
  const calendarOccurrences = state.calendar.occurrences.map((occurrence) => ({ ...occurrence, isExpense: false }));
  const expenseOccurrences = (state.calendar.expenseOccurrences || []).map((occurrence) => ({
    ...occurrence,
    date: occurrence.dueDate,
    title: occurrence.name,
    kind: "expense",
    isExpense: true
  }));
  [...calendarOccurrences, ...expenseOccurrences].forEach((occurrence) => {
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
  const occurrences = [
    ...state.calendar.occurrences.map((occurrence) => ({ ...occurrence, isExpense: false })),
    ...(state.calendar.expenseOccurrences || []).map((occurrence) => ({
      ...occurrence,
      date: occurrence.dueDate,
      title: occurrence.name,
      kind: "expense",
      isExpense: true
    }))
  ].filter((occurrence) => occurrence.date === date);
  if (!occurrences.length) {
    list.append(text("p", "등록된 일정이 없습니다.", "empty"));
    return;
  }
  occurrences.forEach((occurrence) => {
    const button = text("button", "", `calendar-agenda-item ${occurrence.kind}`);
    button.type = "button";
    const body = text("span", "");
    body.append(text("strong", occurrence.title));
    const source = occurrence.isExpense ? null : events.get(occurrence.eventId);
    const time = occurrence.eventTime ? occurrence.eventTime.slice(0, 5) : "하루 종일";
    body.append(text("small", occurrence.isExpense
      ? `정기지출 · ${recurringOccurrenceLabels[occurrence.status] || occurrence.status}${occurrence.amountChanged ? " · 금액 변동" : ""}`
      : `${time} · ${recurrenceText(source || occurrence)}`));
    button.append(body);
    if (source) button.addEventListener("click", () => openCalendarForm(source));
    if (occurrence.isExpense) button.addEventListener("click", () => {
      state.selectedRecurringExpenseId = occurrence.recurringExpenseId;
      selectTab("expenses");
    });
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

function mutationHeaders(operation, version = null, idempotencyKey = crypto.randomUUID()) {
  const headers = {
    "idempotency-key": idempotencyKey,
    "x-tm-confirm-mutation": operation
  };
  if (version === null) headers["if-none-match"] = "*";
  else headers["if-match"] = `"${version}"`;
  return headers;
}

function expenseMutationFingerprint(value) {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(expenseMutationFingerprint).join(",")}]`;
  return `{${Object.keys(value).sort().map((key) =>
    `${JSON.stringify(key)}:${expenseMutationFingerprint(value[key])}`).join(",")}}`;
}

function expenseMutationAction(operation, resource) {
  return `${operation}:${resource}`;
}

function discardExpenseMutation(operation, resource) {
  pendingExpenseMutationKeys.delete(expenseMutationAction(operation, resource));
}

async function expenseMutation(path, {
  method = "POST",
  operation,
  version = null,
  resource,
  body,
  aiConfirmation = null
}) {
  const action = expenseMutationAction(operation, resource);
  const fingerprint = expenseMutationFingerprint(body ?? null);
  let pending = pendingExpenseMutationKeys.get(action);
  if (!pending || pending.fingerprint !== fingerprint) {
    pending = { fingerprint, key: crypto.randomUUID() };
    pendingExpenseMutationKeys.set(action, pending);
  }
  const headers = mutationHeaders(operation, version, pending.key);
  if (aiConfirmation) headers["x-tm-confirm-ai-call"] = aiConfirmation;
  const result = await api(path, { method, headers, body });
  if (pendingExpenseMutationKeys.get(action)?.key === pending.key) {
    pendingExpenseMutationKeys.delete(action);
  }
  return result;
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
