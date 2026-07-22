import { useCallback, useEffect, useMemo, useState } from "react";

import { DataPage, TrashPage } from "./components/DataPages";
import { ChangeRequestsPage } from "./components/ChangeRequestsPage";
import { Icon, type IconName } from "./components/Icon";
import { NotesPage, SearchPage, WorkLogsPage } from "./components/KnowledgePages";
import { SessionPage } from "./components/SessionPage";
import { DeviceManagementPage } from "./components/DeviceManagementPage";
import { TaskDetail } from "./components/TaskDetail";
import { HistoryPage, InboxPage, ProjectsPage, TodayPage } from "./components/TaskPages";
import {
  createDefaultApi,
  type CostStatus,
  type TaskReportResult,
  type TmApi,
} from "./lib/api";
import type {
  AppSnapshot,
  CreateChangeRequestInput,
  CreateNoteInput,
  CreateTaskInput,
  CreateWorkLogInput,
  DayEntryStatus,
  FinishSessionInput,
  NoteType,
  StartSessionInput,
  Task,
  UpdateChangeRequestInput,
  UpdateTaskInput,
} from "./types";

type PageId =
  | "inbox"
  | "today"
  | "projects"
  | "history"
  | "sessions"
  | "worklogs"
  | "notes"
  | "search"
  | "change-requests"
  | "trash"
  | "devices"
  | "data";

interface AppProps {
  api?: TmApi;
}

interface NavigationItem {
  id: PageId;
  label: string;
  icon: IconName;
  count?: number;
}

const defaultApi = createDefaultApi();
const DEFAULT_API_HARD_LIMIT_MICROUSD = 20_000_000;
const DEFAULT_CLOUD_HARD_LIMIT_MICROUSD = 30_000_000;
const COST_REFRESH_INTERVAL_MS = 5 * 60 * 1000;

const formatUsd = (microusd: number, alwaysCents = true): string => {
  const dollars = microusd / 1_000_000;
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: alwaysCents ? 2 : Number.isInteger(dollars) ? 0 : 2,
    maximumFractionDigits: 2,
  }).format(dollars);
};

const costClassName = (usedMicrousd: number | null, hardLimitMicrousd: number): string => {
  if (usedMicrousd === null || hardLimitMicrousd <= 0) return "cost-pill cost-pill--unavailable";
  const ratio = usedMicrousd / hardLimitMicrousd;
  if (ratio >= 1) return "cost-pill cost-pill--danger";
  if (ratio >= 0.8) return "cost-pill cost-pill--warning";
  return "cost-pill";
};

const errorMessage = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "요청을 처리하지 못했습니다.";
};

export function App({ api = defaultApi }: AppProps) {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [page, setPage] = useState<PageId>("today");
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [savingTask, setSavingTask] = useState(false);
  const [fatalError, setFatalError] = useState<string | null>(null);
  const [toast, setToast] = useState<{ type: "success" | "error"; message: string } | null>(null);
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [taskReport, setTaskReport] = useState<TaskReportResult | null>(null);
  const [taskReportLoading, setTaskReportLoading] = useState(false);
  const [costStatus, setCostStatus] = useState<CostStatus | null>(null);

  const loadSnapshot = useCallback(async () => {
    try {
      const next = await api.getSnapshot();
      setSnapshot(next);
      setFatalError(null);
    } catch (error) {
      setFatalError(errorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  const loadCostStatus = useCallback(async () => {
    try {
      setCostStatus(await api.getCostStatus());
    } catch {
      // Cost status is supplementary and must never block the main workspace.
    }
  }, [api]);

  useEffect(() => {
    void loadCostStatus();
    const timer = window.setInterval(() => void loadCostStatus(), COST_REFRESH_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [loadCostStatus]);

  useEffect(() => {
    let active = true;
    void api.latestTaskReport().then((report) => {
      if (active) setTaskReport(report);
    }).catch(() => undefined);
    return () => { active = false; };
  }, [api]);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === "k") {
        event.preventDefault();
        setPage("search");
      }
    };
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, []);

  useEffect(() => {
    if (!toast) return undefined;
    const timer = window.setTimeout(() => setToast(null), 3600);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const notify = (message: string, type: "success" | "error" = "success") =>
    setToast({ message, type });

  const mutate = useCallback(
    async (action: () => Promise<void>, successMessage: string) => {
      try {
        await action();
        await loadSnapshot();
        notify(successMessage);
        return true;
      } catch (error) {
        notify(errorMessage(error), "error");
        return false;
      }
    },
    [loadSnapshot],
  );

  const mutateVoid = useCallback(
    async (action: () => Promise<void>, successMessage: string): Promise<void> => {
      await mutate(action, successMessage);
    },
    [mutate],
  );

  const createTask = (input: CreateTaskInput) => mutateVoid(
    () => api.createTask(input),
    "프로젝트에 할 일을 추가했습니다.",
  );
  const createProject = async (name: string): Promise<string> => {
    try {
      const projectId = await api.createProject(name);
      await loadSnapshot();
      notify("프로젝트를 추가했습니다.");
      return projectId;
    } catch (error) {
      notify(errorMessage(error), "error");
      return "";
    }
  };
  const updateTask = async (input: UpdateTaskInput) => {
    setSavingTask(true);
    try {
      if (await mutate(() => api.updateTask(input), "Task 변경과 이벤트를 저장했습니다.")) {
        setSelectedTaskId(null);
      }
    } finally {
      setSavingTask(false);
    }
  };
  const planTask = (task: Task) => snapshot
    ? mutate(() => api.planTask(task.id, snapshot.today), "오늘 계획에 추가했습니다.")
    : Promise.resolve();
  const resolveDayEntry = (entryId: string, status: Exclude<DayEntryStatus, "planned">) => {
    const messages = { done: "완료 기록을 확정했습니다.", deferred: "어제 기록을 이월하고 오늘 계획을 만들었습니다.", skipped: "건너뛰기 기록을 확정했습니다." };
    void mutate(() => api.resolveDayEntry(entryId, status), messages[status]).catch(() => undefined);
  };
  const generateTaskReport = async () => {
    setTaskReportLoading(true);
    try {
      const report = await api.generateTaskReport();
      setTaskReport(report);
      void loadCostStatus();
      notify("오늘의 Task AI 리포트를 만들었습니다.");
    } catch (error) {
      notify(errorMessage(error), "error");
    } finally {
      setTaskReportLoading(false);
    }
  };
  const rateTaskReport = async (helpful: boolean) => {
    if (!taskReport) return;
    try {
      const report = await api.rateTaskReport(taskReport.runId, helpful);
      setTaskReport(report);
      notify("리포트 평가를 기록했습니다.");
    } catch (error) {
      notify(errorMessage(error), "error");
    }
  };
  const startSession = (input: StartSessionInput) => mutateVoid(() => api.startSession(input), "집중 세션을 시작했습니다.");
  const finishSession = (input: FinishSessionInput) => mutateVoid(() => api.finishSession(input), "세션과 WorkLog를 원자적으로 저장했습니다.");
  const createWorkLog = (input: CreateWorkLogInput) => mutateVoid(() => api.createWorkLog(input), "WorkLog를 저장했습니다.");
  const createNote = (input: CreateNoteInput) => mutateVoid(() => api.createNote(input), "Note를 저장했습니다.");
  const createChangeRequest = (input: CreateChangeRequestInput) =>
    mutate(() => api.createChangeRequest(input), "개선 요청 초안을 저장했습니다.");
  const updateChangeRequest = (input: UpdateChangeRequestInput) =>
    mutate(() => api.updateChangeRequest(input), "개선 요청 초안을 새 revision으로 저장했습니다.");
  const approveChangeRequest = (
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ) =>
    mutate(
      () => api.approveChangeRequest(changeRequestId, expectedRevision, expectedAttemptCount),
      "현재 revision의 개선 요청을 승인했습니다.",
    );
  const returnChangeRequestToDraft = (
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ) =>
    mutate(
      () => api.returnChangeRequestToDraft(
        changeRequestId,
        expectedRevision,
        expectedAttemptCount,
      ),
      "개선 요청을 초안으로 돌렸습니다.",
    );
  const cancelChangeRequest = (
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ) => mutate(
    () => api.cancelChangeRequest(
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
      reason,
    ),
    "개선 요청을 취소하고 이유를 기록했습니다.",
  );
  const abandonChangeRequest = (
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ) =>
    mutate(
      () => api.abandonChangeRequest(
        changeRequestId,
        expectedRevision,
        expectedAttemptCount,
        reason,
      ),
      "유실된 claim을 실패로 전환하고 이유를 기록했습니다.",
    );
  const promoteWorkLog = (workLogId: string, type: NoteType, title: string) => mutateVoid(() => api.promoteWorkLog(workLogId, type, title), "원본 WorkLog를 유지하고 Note로 승격했습니다.");
  const moveRecordToTrash = (itemId: string, entityType: "note" | "work_log", label: string) =>
    mutateVoid(() => api.moveToTrash(itemId, entityType), `${label}를 휴지통으로 이동했습니다.`);

  const selectedTask = useMemo(
    () => snapshot?.tasks.find((task) => task.id === selectedTaskId) ?? null,
    [selectedTaskId, snapshot],
  );

  const navigate = (target: PageId) => {
    setPage(target);
    setMobileNavOpen(false);
    document.querySelector<HTMLElement>("#main-content")?.focus({ preventScroll: true });
  };

  if (loading) {
    return (
      <div className="boot-screen" aria-live="polite">
        <span className="brand-mark brand-mark--large">TM</span>
        <div className="boot-screen__spinner" />
        <strong>로컬 데이터를 준비하고 있습니다</strong>
        <p>시작 백업과 데이터베이스 상태를 확인합니다.</p>
      </div>
    );
  }

  if (!snapshot || fatalError) {
    return (
      <div className="error-screen" role="alert">
        <span><Icon name="database" size={28} /></span>
        <h1>데이터를 열지 못했습니다</h1>
        <p>{fatalError ?? "알 수 없는 오류가 발생했습니다."}</p>
        <button className="primary-button" onClick={() => { setLoading(true); void loadSnapshot(); }} type="button">다시 시도</button>
      </div>
    );
  }

  const pendingApprovalCount = snapshot.changeRequests.filter(
    (request) => request.status === "draft",
  ).length;
  const navigation: Array<{ title: string; items: NavigationItem[] }> = [
    {
      title: "Task",
      items: [
        { id: "inbox", label: "Inbox", icon: "inbox" },
        { id: "today", label: "오늘", icon: "today", count: snapshot.todayView.planned.length + snapshot.todayView.inProgress.length },
        { id: "projects", label: "프로젝트", icon: "projects" },
        { id: "history", label: "히스토리", icon: "history" },
      ],
    },
    {
      title: "기록",
      items: [
        { id: "sessions", label: "작업 세션", icon: "timer" },
        { id: "worklogs", label: "WorkLog", icon: "worklog" },
        { id: "notes", label: "Note", icon: "note" },
      ],
    },
    {
      title: "도구",
      items: [
        { id: "search", label: "통합 검색", icon: "search" },
        { id: "change-requests", label: "개선 요청함", icon: "spark", count: pendingApprovalCount || undefined },
        { id: "trash", label: "휴지통", icon: "trash", count: snapshot.trash.length || undefined },
        { id: "devices", label: "기기 관리", icon: "shield" },
        { id: "data", label: "백업 · 내보내기", icon: "database" },
      ],
    },
  ];

  const renderPage = () => {
    switch (page) {
      case "inbox":
        return <InboxPage />;
      case "today":
        return <TodayPage onGenerateReport={generateTaskReport} onOpen={(task) => setSelectedTaskId(task.id)} onRateReport={rateTaskReport} onResolve={resolveDayEntry} report={taskReport} reportLoading={taskReportLoading} tasks={snapshot.tasks} today={snapshot.today} view={snapshot.todayView} />;
      case "projects":
        return <ProjectsPage onCreateProject={createProject} onCreateTask={createTask} onOpen={(task) => setSelectedTaskId(task.id)} onPlan={(task) => void planTask(task)} projects={snapshot.projects} tasks={snapshot.tasks} />;
      case "history":
        return <HistoryPage history={snapshot.history} onOpen={(task) => setSelectedTaskId(task.id)} />;
      case "sessions":
        return <SessionPage activeSession={snapshot.activeSession} onFinish={finishSession} onOpenTask={(task) => setSelectedTaskId(task.id)} onStart={startSession} projects={snapshot.projects} recentSessions={snapshot.recentSessions} tasks={snapshot.tasks} />;
      case "worklogs":
        return <WorkLogsPage logs={snapshot.workLogs} onCreate={createWorkLog} onOpenTask={(task) => setSelectedTaskId(task.id)} onPromote={promoteWorkLog} onTrash={(workLogId) => moveRecordToTrash(workLogId, "work_log", "WorkLog")} projects={snapshot.projects} tasks={snapshot.tasks} />;
      case "notes":
        return <NotesPage notes={snapshot.notes} onCreate={createNote} onOpenTask={(task) => setSelectedTaskId(task.id)} onTrash={(noteId) => moveRecordToTrash(noteId, "note", "Note")} sessions={snapshot.recentSessions} tasks={snapshot.tasks} />;
      case "search":
        return <SearchPage onOpenTask={(taskId) => setSelectedTaskId(taskId)} onSearch={(query) => api.search(query)} />;
      case "change-requests":
        return <ChangeRequestsPage onAbandon={abandonChangeRequest} onApprove={approveChangeRequest} onCancel={cancelChangeRequest} onCreate={createChangeRequest} onReturnToDraft={returnChangeRequestToDraft} onUpdate={updateChangeRequest} projects={snapshot.projects} requests={snapshot.changeRequests} tasks={snapshot.tasks} />;
      case "trash":
        return <TrashPage items={snapshot.trash} onRestore={(itemId) => mutateVoid(() => api.restoreTrashItem(itemId), "항목과 연결을 복원했습니다.")} />;
      case "devices":
        return <DeviceManagementPage />;
      case "data":
        return <DataPage backups={snapshot.backups} databasePath={snapshot.databasePath} lastBackupAt={snapshot.lastBackupAt} onBackup={() => mutateVoid(() => api.createBackup(), "최신 상태를 새 백업 파일로 저장했습니다.")} onExport={() => api.exportAll()} onRestore={(backupId) => mutateVoid(() => api.restoreBackup(backupId), "백업 복원을 완료했습니다.")} />;
    }
  };

  const pageLabel = navigation.flatMap((section) => section.items).find((item) => item.id === page)?.label ?? "TM";

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">본문으로 건너뛰기</a>
      <aside className={`sidebar ${mobileNavOpen ? "sidebar--open" : ""}`} aria-label="주 메뉴">
        <header className="sidebar__brand">
          <span className="brand-mark">TM</span>
          <div><strong>Task Manager</strong><span>개인 업무 공간</span></div>
          <button aria-label="메뉴 닫기" className="icon-button sidebar__close" onClick={() => setMobileNavOpen(false)} type="button"><Icon name="close" /></button>
        </header>

        <nav className="sidebar__nav">
          {navigation.map((section) => (
            <section key={section.title}>
              <h2>{section.title}</h2>
              {section.items.map((item) => (
                <button aria-current={page === item.id ? "page" : undefined} key={item.id} onClick={() => navigate(item.id)} type="button">
                  <Icon name={item.icon} /><span>{item.label}</span>{item.count !== undefined && <em>{item.count}</em>}
                </button>
              ))}
            </section>
          ))}
        </nav>

        {snapshot.activeSession && (
          <button className="sidebar-session" onClick={() => navigate("sessions")} type="button">
            <span className="sidebar-session__icon"><Icon name="timer" size={17} /></span>
            <span><small>집중 세션 진행 중</small><strong>{snapshot.activeSession.goal}</strong></span>
            <Icon name="chevron" size={14} />
          </button>
        )}

        <footer className="sidebar__footer">
          <span className="db-status"><i /> 로컬 DB 정상</span>
          <span>{snapshot.lastBackupAt ? `백업 ${new Intl.DateTimeFormat("ko-KR", { timeZone: "Asia/Seoul", hour: "2-digit", minute: "2-digit" }).format(new Date(snapshot.lastBackupAt))}` : "백업 없음"}</span>
        </footer>
      </aside>

      {mobileNavOpen && <button aria-label="메뉴 닫기" className="mobile-scrim" onClick={() => setMobileNavOpen(false)} type="button" />}

      <div className="workspace">
        <header className="topbar">
          <button aria-label="메뉴 열기" className="icon-button topbar__menu" onClick={() => setMobileNavOpen(true)} type="button"><Icon name="menu" /></button>
          <strong className="topbar__title">{pageLabel}</strong>
          <button className="command-search" onClick={() => navigate("search")} type="button"><Icon name="search" size={16} /><span>전체 기록 검색</span><kbd>Ctrl K</kbd></button>
          <div className="topbar__costs" aria-label="현재 비용 현황">
            <span
              className={costClassName(
                costStatus?.api.usedMicrousd ?? null,
                costStatus?.api.hardLimitMicrousd ?? DEFAULT_API_HARD_LIMIT_MICROUSD,
              )}
              title={costStatus ? `OpenAI API ${costStatus.api.budgetMonth}` : "OpenAI API 비용 확인 중"}
            >
              API {costStatus ? formatUsd(costStatus.api.usedMicrousd) : "—"} / {formatUsd(costStatus?.api.hardLimitMicrousd ?? DEFAULT_API_HARD_LIMIT_MICROUSD, false)}
            </span>
            <span
              className={costClassName(
                costStatus?.cloud.usedMicrousd ?? null,
                costStatus?.cloud.hardLimitMicrousd ?? DEFAULT_CLOUD_HARD_LIMIT_MICROUSD,
              )}
              title={costStatus?.cloud.billingPeriodStart && costStatus.cloud.billingPeriodEnd
                ? `Railway ${new Date(costStatus.cloud.billingPeriodStart).toLocaleDateString("ko-KR")}–${new Date(costStatus.cloud.billingPeriodEnd).toLocaleDateString("ko-KR")}${costStatus.cloud.stale ? " · 마지막 확인값" : ""}`
                : "Railway 비용 확인 중"}
            >
              Cloud {costStatus?.cloud.usedMicrousd !== null && costStatus?.cloud.usedMicrousd !== undefined
                ? formatUsd(costStatus.cloud.usedMicrousd)
                : "—"} / {formatUsd(costStatus?.cloud.hardLimitMicrousd ?? DEFAULT_CLOUD_HARD_LIMIT_MICROUSD, false)}
            </span>
          </div>
          <div className="topbar__date"><span>{new Intl.DateTimeFormat("ko-KR", { timeZone: "Asia/Seoul", month: "long", day: "numeric", weekday: "short" }).format(new Date(`${snapshot.today}T12:00:00+09:00`))}</span><i /></div>
        </header>

        <main className="main-content" id="main-content" tabIndex={-1}>{renderPage()}</main>
      </div>

      <nav className="bottom-nav" aria-label="빠른 메뉴">
        {navigation[0].items.slice(0, 3).map((item) => <button aria-current={page === item.id ? "page" : undefined} key={item.id} onClick={() => navigate(item.id)} type="button"><Icon name={item.icon} size={19} /><span>{item.label}</span></button>)}
        <button aria-current={page === "sessions" ? "page" : undefined} onClick={() => navigate("sessions")} type="button"><Icon name="timer" size={19} /><span>세션</span></button>
        <button aria-label="전체 메뉴" onClick={() => setMobileNavOpen(true)} type="button"><Icon name="menu" size={19} /><span>더보기</span></button>
      </nav>

      {selectedTask && <TaskDetail key={selectedTask.id} onClose={() => setSelectedTaskId(null)} onSave={updateTask} onTrash={async () => {
        if (await mutate(() => api.moveToTrash(selectedTask.id, "task"), "Task를 휴지통으로 이동했습니다.")) {
          setSelectedTaskId(null);
        }
      }} projects={snapshot.projects} saving={savingTask} task={selectedTask} />}

      {toast && (
        <div className={`toast toast--${toast.type}`} role={toast.type === "error" ? "alert" : "status"}>
          <span><Icon name={toast.type === "error" ? "flag" : "check"} size={17} /></span>
          <p>{toast.message}</p>
          <button aria-label="알림 닫기" className="icon-button icon-button--small" onClick={() => setToast(null)} type="button"><Icon name="close" size={14} /></button>
        </div>
      )}
    </div>
  );
}
