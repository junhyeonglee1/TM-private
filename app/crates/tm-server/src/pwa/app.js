"use strict";

const pairingKey = "tm.mobile.pairing.v1";
const state = { device: null, activeTab: "assistant", taskReport: null, taskReportTasks: new Map() };
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
  if (tab === "tasks") void loadTasks();
  if (tab === "notes") void loadNotes();
  if (tab === "approvals") void loadApprovals();
  if (tab === "device" && state.device) renderDevice(state.device);
}

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
