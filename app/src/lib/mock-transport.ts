import type { CommandTransport, TaskReportResult } from "./api";
import { UNCATEGORIZED_PROJECT_SYSTEM_KEY } from "../types";
import type {
  AppSnapshot,
  BackupInfo,
  CalendarEvent,
  CalendarMonth,
  ChangeRequest,
  CreateChangeRequestInput,
  CreateCalendarEventInput,
  CreateNoteInput,
  CreateTaskInput,
  CreateWorkLogInput,
  DayEntryStatus,
  ExpenseEventKind,
  ExpenseImportCommitResult,
  ExpenseImportPreview,
  ExpenseMonthSummary,
  ExpenseSourceStatus,
  ExpenseReportResult,
  ExpenseReview,
  ExpenseReviewPage,
  ExpenseTransaction,
  ExpenseTransactionPage,
  OverrideExpenseTransactionInput,
  ExportResult,
  FinishSessionInput,
  LatestStockScreen,
  ListExpenseReviewsInput,
  ListExpenseTransactionsInput,
  ListStockScreenResultsInput,
  Note,
  NoteType,
  PreviewExpenseImportInput,
  Project,
  RecurringExpenseItem,
  RecurringExpenseOccurrence,
  ResolveExpenseReviewInput,
  SearchResult,
  StartSessionInput,
  StockWatchlistItem,
  StockScreenResult,
  StockScreenResultPage,
  Task,
  TaskDayEntry,
  TrashItem,
  UpdateTaskInput,
  UpdateCalendarEventInput,
  UpdateChangeRequestInput,
  UpdateRecurringExpenseInput,
  UpdateExpenseSourceStatusInput,
  UpsertStockWatchlistItemInput,
  CreateRecurringExpenseInput,
  WorkLog,
  WorkSession,
} from "../types";

const SEOUL_TIME_ZONE = "Asia/Seoul";

const seoulDate = (value = new Date()): string =>
  new Intl.DateTimeFormat("en-CA", {
    timeZone: SEOUL_TIME_ZONE,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(value);

const shiftDate = (date: string, days: number): string => {
  const base = new Date(`${date}T00:00:00+09:00`);
  base.setUTCDate(base.getUTCDate() + days);
  return seoulDate(base);
};

const now = (): string => new Date().toISOString();
let sequence = 100;
const id = (prefix: string): string => `${prefix}-019mock-${++sequence}`;

const emptyLinks = () => ({
  taskIds: [],
  sessionIds: [],
  workLogIds: [],
  filePaths: [],
  urls: [],
});

const buildSample = (): AppSnapshot => {
  const today = seoulDate();
  const yesterday = shiftDate(today, -1);
  const twoDaysAgo = shiftDate(today, -2);
  const timestamp = now();

  const projects: Project[] = [
    {
      id: "project-tm",
      name: "TM 데스크톱",
      description: "로컬 퍼스트 개인 업무 관리 앱",
      color: "#5b6cf9",
      systemKey: null,
      openTaskCount: 4,
      completedTaskCount: 8,
      archived: false,
    },
    {
      id: "project-writing",
      name: "기록 정리",
      description: "업무 지식과 회고를 정리합니다",
      color: "#d77849",
      systemKey: null,
      openTaskCount: 2,
      completedTaskCount: 3,
      archived: false,
    },
    {
      id: "project-personal",
      name: "개인 루틴",
      description: "꾸준히 유지할 생활 루틴",
      color: "#2f9b75",
      systemKey: null,
      openTaskCount: 2,
      completedTaskCount: 12,
      archived: false,
    },
    {
      id: "project-uncategorized",
      name: "기타",
      description: "프로젝트를 선택하지 않은 Task",
      color: "#7386ff",
      systemKey: UNCATEGORIZED_PROJECT_SYSTEM_KEY,
      openTaskCount: 0,
      completedTaskCount: 0,
      archived: false,
    },
  ];

  const baseTask = (
    taskId: string,
    title: string,
    status: Task["status"],
    projectId: string | null,
    projectName: string | null,
    priority: Task["priority"],
    dueDate: string | null,
    tags: string[],
  ): Task => ({
    id: taskId,
    title,
    description: "",
    status,
    priority,
    dueDate,
    projectId,
    projectName,
    tags,
    checklist: [],
    events: [
      {
        id: id("event"),
        taskId,
        eventType: "created",
        summary: "Task를 만들었습니다",
        before: null,
        after: { status },
        createdAt: timestamp,
      },
    ],
    workLogs: [],
    createdAt: timestamp,
    updatedAt: timestamp,
    deletedAt: null,
  });

  const tasks: Task[] = [
    {
      ...baseTask(
        "task-schema",
        "데이터 모델 불변 조건 검토",
        "in_progress",
        "project-tm",
        "TM 데스크톱",
        "high",
        today,
        ["설계", "SQLite"],
      ),
      description:
        "TaskDayEntry의 확정 이력과 TaskEvent before/after가 변경되지 않는지 검토합니다.",
      checklist: [
        { id: "check-1", label: "Task 상태 전이 검토", checked: true },
        { id: "check-2", label: "이월 시나리오 작성", checked: false },
        { id: "check-3", label: "복원 테스트 추가", checked: false },
      ],
      events: [
        {
          id: "event-schema-2",
          taskId: "task-schema",
          eventType: "status_changed",
          summary: "상태를 진행 중으로 변경했습니다",
          before: { status: "todo" },
          after: { status: "in_progress" },
          createdAt: timestamp,
        },
      ],
    },
    baseTask(
      "task-ui",
      "오늘 화면 정보 밀도 다듬기",
      "todo",
      "project-tm",
      "TM 데스크톱",
      "medium",
      today,
      ["UI"],
    ),
    baseTask(
      "task-export",
      "JSON · Markdown 내보내기 검증",
      "todo",
      "project-tm",
      "TM 데스크톱",
      "high",
      shiftDate(today, 1),
      ["데이터 안전"],
    ),
    baseTask(
      "task-blocked",
      "서명 인증서 발급 방식 확인",
      "blocked",
      "project-tm",
      "TM 데스크톱",
      "medium",
      shiftDate(today, 3),
      ["배포"],
    ),
    baseTask(
      "task-notes",
      "회의 결정 사항을 Note로 정리",
      "inbox",
      "project-writing",
      "기록 정리",
      "none",
      null,
      ["정리"],
    ),
    baseTask(
      "task-routine",
      "저녁 회고 10분",
      "todo",
      "project-personal",
      "개인 루틴",
      "low",
      today,
      ["회고"],
    ),
    baseTask(
      "task-done",
      "백업 정책 초안 작성",
      "done",
      "project-tm",
      "TM 데스크톱",
      "medium",
      yesterday,
      ["백업"],
    ),
    baseTask(
      "task-yesterday",
      "참고자료 링크 분류",
      "todo",
      "project-writing",
      "기록 정리",
      "low",
      yesterday,
      ["정리"],
    ),
  ];

  const dayEntry = (
    entryId: string,
    taskId: string,
    date: string,
    status: DayEntryStatus,
  ): TaskDayEntry => ({
    id: entryId,
    taskId,
    date,
    status,
    task: tasks.find((task) => task.id === taskId) as Task,
    resolvedAt: status === "planned" ? null : timestamp,
  });

  const todayView = {
    yesterdayIncomplete: [dayEntry("day-yesterday", "task-yesterday", yesterday, "planned")],
    planned: [
      dayEntry("day-ui", "task-ui", today, "planned"),
      dayEntry("day-routine", "task-routine", today, "planned"),
    ],
    inProgress: [tasks[0]],
    completed: [dayEntry("day-done", "task-done", today, "done")],
  };

  const workLogs: WorkLog[] = [
    {
      id: "log-1",
      taskIds: ["task-schema"],
      sessionId: "session-1",
      projectId: "project-tm",
      title: "이력 불변성 검토",
      content: "확정된 날짜 기록을 수정하는 경로를 모두 차단하고 테스트 항목을 정리했습니다.",
      outcome: "상태 전이 규칙 확정",
      blockers: null,
      createdAt: timestamp,
      promotedNoteId: null,
    },
    {
      id: "log-2",
      taskIds: ["task-export"],
      sessionId: null,
      projectId: "project-tm",
      title: "내보내기 범위 확인",
      content: "연결 테이블과 삭제된 항목을 포함하는지 체크리스트를 만들었습니다.",
      outcome: "검증 목록 작성",
      blockers: "대용량 첨부파일 처리 방식 확인 필요",
      createdAt: timestamp,
      promotedNoteId: "note-export",
    },
  ];
  tasks[0].workLogs = [workLogs[0]];
  tasks[2].workLogs = [workLogs[1]];

  const recentSessions: WorkSession[] = [
    {
      id: "session-1",
      projectId: "project-tm",
      projectName: "TM 데스크톱",
      taskIds: ["task-schema"],
      taskTitles: ["데이터 모델 불변 조건 검토"],
      goal: "데이터 이력 규칙 확정",
      startedAt: new Date(Date.now() - 7_200_000).toISOString(),
      endedAt: new Date(Date.now() - 4_500_000).toISOString(),
      result: "핵심 상태 전이 규칙을 확정했습니다.",
      blockers: null,
      nextAction: "통합 테스트 추가",
    },
  ];

  const notes: Note[] = [
    {
      id: "note-wal",
      type: "concept",
      title: "SQLite WAL과 busy timeout",
      content: "앱과 CLI가 동시에 접근할 때 읽기와 쓰기 대기 전략을 일관되게 적용합니다.",
      links: { ...emptyLinks(), taskIds: ["task-schema"] },
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
    },
    {
      id: "note-export",
      type: "decision",
      title: "내보내기는 단일 read transaction 사용",
      content: "JSON과 Markdown이 같은 시점의 데이터를 담도록 하나의 읽기 트랜잭션에서 생성합니다.",
      links: { ...emptyLinks(), taskIds: ["task-export"], workLogIds: ["log-2"] },
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
    },
    {
      id: "note-install",
      type: "howto",
      title: "NSIS 데이터 보존 확인 절차",
      content: "설치, 재설치, 제거 전후로 외부 data 폴더의 해시를 비교합니다.",
      links: emptyLinks(),
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
    },
  ];

  const backups: BackupInfo[] = [
    {
      id: "backup-1",
      fileName: `tm-${today.replaceAll("-", "")}-startup.sqlite3`,
      createdAt: timestamp,
      sizeBytes: 1_824_768,
      trigger: "startup",
    },
  ];

  const trash: TrashItem[] = [
    {
      id: "trash-note-1",
      entityType: "note",
      title: "임시 설치 메모",
      deletedAt: timestamp,
    },
  ];

  const changeRequest = (
    overrides: Partial<ChangeRequest> & Pick<ChangeRequest, "id" | "title" | "status">,
  ): ChangeRequest => {
    const base: ChangeRequest = {
    id: overrides.id,
    kind: "feature",
    projectId: "project-tm",
    taskId: null,
    title: overrides.title,
    description: "TM을 사용하면서 발견한 개선 지점을 정리했습니다.",
    desiredOutcome: "기존 데이터와 작업 흐름을 유지하면서 더 분명하게 사용할 수 있습니다.",
    reproductionSteps: "TM을 열고 관련 화면에서 현재 동작을 확인합니다.",
    priority: "medium",
    status: overrides.status,
    revision: 1,
    approvedRevision: null,
    attemptCount: 0,
    requestedBy: "desktop-user",
    updatedBy: "desktop-user",
    createdAt: timestamp,
    updatedAt: timestamp,
    approvedAt: null,
    approvedBy: null,
    claimedAt: null,
    claimedBy: null,
    completedAt: null,
    completedBy: null,
    failedAt: null,
    failedBy: null,
    cancelledAt: null,
    cancelledBy: null,
    resultSummary: null,
    patchRef: null,
    failureReason: null,
      cancellationReason: null,
    };
    return { ...base, ...overrides };
  };

  const changeRequests: ChangeRequest[] = [
    changeRequest({
      id: "change-draft",
      kind: "ui",
      title: "오늘 화면의 구역 구분을 더 선명하게",
      status: "draft",
      taskId: "task-ui",
      priority: "low",
    }),
    changeRequest({
      id: "change-approved",
      kind: "feature",
      title: "검색 결과 키보드 이동 지원",
      status: "approved",
      approvedRevision: 1,
      approvedAt: timestamp,
      approvedBy: "desktop-user",
    }),
    changeRequest({
      id: "change-claimed",
      kind: "bug",
      title: "세션 종료 요약이 작은 화면에서 잘리는 문제",
      status: "claimed",
      approvedRevision: 1,
      approvedAt: timestamp,
      approvedBy: "desktop-user",
      claimedAt: timestamp,
      claimedBy: "codex-local",
      attemptCount: 1,
      priority: "high",
    }),
    changeRequest({
      id: "change-completed",
      kind: "ui",
      title: "백업 상태 배지 문구 개선",
      status: "completed",
      approvedRevision: 1,
      approvedAt: timestamp,
      approvedBy: "desktop-user",
      claimedAt: timestamp,
      claimedBy: "codex-local",
      completedAt: timestamp,
      completedBy: "codex-local",
      resultSummary: "백업 시각과 트리거를 한눈에 확인하도록 배지를 정리했습니다.",
      patchRef: "0002",
      attemptCount: 1,
    }),
    changeRequest({
      id: "change-failed",
      kind: "other",
      title: "히스토리 필터 응답 속도 개선",
      status: "failed",
      approvedRevision: 1,
      approvedAt: timestamp,
      approvedBy: "desktop-user",
      claimedAt: timestamp,
      claimedBy: "codex-local",
      failedAt: timestamp,
      failedBy: "codex-local",
      failureReason: "재현 조건이 부족해 안전하게 변경 범위를 확정하지 못했습니다.",
      attemptCount: 1,
    }),
    changeRequest({
      id: "change-cancelled",
      kind: "feature",
      title: "사용하지 않는 보기 모드 추가",
      status: "cancelled",
      cancelledAt: timestamp,
      cancelledBy: "desktop-user",
      cancellationReason: "현재 작업 방식에는 필요하지 않아 요청을 종료했습니다.",
    }),
  ];

  return {
    generatedAt: timestamp,
    today,
    projects,
    tasks,
    todayView,
    history: [
      {
        date: today,
        entries: [...todayView.planned, ...todayView.completed],
        workLogCount: 2,
        sessionMinutes: 45,
      },
      {
        date: yesterday,
        entries: [todayView.yesterdayIncomplete[0]],
        workLogCount: 1,
        sessionMinutes: 80,
      },
      { date: twoDaysAgo, entries: [], workLogCount: 3, sessionMinutes: 120 },
    ],
    activeSession: null,
    recentSessions,
    workLogs,
    notes,
    changeRequests,
    trash,
    backups,
    databasePath: "C:\\Users\\tkfk0\\Desktop\\codex\\TM\\data\\tm.sqlite3",
    lastBackupAt: timestamp,
  };
};

const textValue = (value: unknown): string => (typeof value === "string" ? value : "");

const sampleCalendarEvents = (today: string): CalendarEvent[] => {
  const timestamp = now();
  return [
    {
      id: "calendar-rent",
      title: "월세 납부",
      description: "매월 마지막 날 확인",
      kind: "payment",
      startDate: `${today.slice(0, 7)}-01`,
      eventTime: null,
      recurrence: "monthly_last_day",
      dayOfMonth: null,
      endsOn: null,
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
      version: 1,
    },
    {
      id: "calendar-insurance",
      title: "보험료 납부",
      description: "자동이체 잔액 확인",
      kind: "payment",
      startDate: `${today.slice(0, 7)}-01`,
      eventTime: "09:00:00",
      recurrence: "monthly_day",
      dayOfMonth: 10,
      endsOn: null,
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
      version: 1,
    },
  ];
};

const sampleExpenses = (today: string): {
  sources: ExpenseSourceStatus[];
  transactions: ExpenseTransaction[];
  reviews: ExpenseReview[];
  recurring: RecurringExpenseItem[];
} => {
  const month = today.slice(0, 7);
  const occurredAt = (day: number, hour = 12) =>
    `${month}-${String(day).padStart(2, "0")}T${String(hour).padStart(2, "0")}:00:00+09:00`;
  const sources: ExpenseSourceStatus[] = [
    { id: "expense-source-card", adapter: "kb_card_usage_v1", sourceKind: "card", requiredForCompleteReport: true, isActive: true, coverageStart: `${month}-01`, coverageEnd: today, version: 1 },
    { id: "expense-source-account", adapter: "kb_account_history_v1", sourceKind: "account", requiredForCompleteReport: true, isActive: true, coverageStart: `${month}-01`, coverageEnd: today, version: 1 },
    { id: "expense-source-wallet", adapter: "kakaopay_money_v1", sourceKind: "wallet", requiredForCompleteReport: false, isActive: true, coverageStart: `${month}-01`, coverageEnd: today, version: 1 },
  ];
  const transactions: ExpenseTransaction[] = [
    {
      id: "expense-lunch",
      kind: "purchase",
      category: "food",
      status: "confirmed",
      amountMinor: 12_000,
      currency: "KRW",
      occurredAt: occurredAt(2),
      postedDate: `${month}-02`,
      sourceKind: "card",
      merchant: "동네식당",
      counterparty: null,
      memo: null,
      paymentMethodFingerprint: "kb-card-sample",
      exclusionReason: null,
      duplicateOfEventId: null,
      personalAmountMinor: null,
      relatedEventId: null,
      isProvisional: false,
      pendingReviewId: null,
      version: 1,
    },
    {
      id: "expense-coffee",
      kind: "purchase",
      category: "cafe",
      status: "confirmed",
      amountMinor: 5_500,
      currency: "KRW",
      occurredAt: occurredAt(3, 9),
      postedDate: `${month}-03`,
      sourceKind: "wallet",
      merchant: "작은카페",
      counterparty: null,
      memo: null,
      paymentMethodFingerprint: "kakaopay-sample",
      exclusionReason: null,
      duplicateOfEventId: null,
      personalAmountMinor: null,
      relatedEventId: null,
      isProvisional: false,
      pendingReviewId: null,
      version: 1,
    },
    {
      id: "expense-refund",
      kind: "refund",
      category: "refund_income",
      status: "confirmed",
      amountMinor: 3_000,
      currency: "KRW",
      occurredAt: occurredAt(4),
      postedDate: `${month}-04`,
      sourceKind: "card",
      merchant: "온라인상점",
      counterparty: null,
      memo: "부분 환불",
      paymentMethodFingerprint: "kb-card-sample",
      exclusionReason: null,
      duplicateOfEventId: null,
      personalAmountMinor: null,
      relatedEventId: null,
      isProvisional: false,
      pendingReviewId: null,
      version: 1,
    },
    {
      id: "expense-p2p",
      kind: "unknown_p2p",
      category: "unconfirmed",
      status: "unconfirmed",
      amountMinor: 20_000,
      currency: "KRW",
      occurredAt: occurredAt(5, 18),
      postedDate: `${month}-05`,
      sourceKind: "wallet",
      merchant: null,
      counterparty: "카카오페이 송금 상대",
      memo: null,
      paymentMethodFingerprint: "kakaopay-sample",
      exclusionReason: null,
      duplicateOfEventId: null,
      personalAmountMinor: null,
      relatedEventId: null,
      isProvisional: true,
      pendingReviewId: "review-p2p",
      version: 1,
    },
    {
      id: "expense-openai",
      kind: "purchase",
      category: "ott_subscriptions",
      status: "confirmed",
      amountMinor: 2_000,
      currency: "USD",
      occurredAt: occurredAt(6),
      postedDate: `${month}-06`,
      sourceKind: "card",
      merchant: "OpenAI",
      counterparty: null,
      memo: null,
      paymentMethodFingerprint: "kb-card-sample",
      exclusionReason: null,
      duplicateOfEventId: null,
      personalAmountMinor: null,
      relatedEventId: null,
      isProvisional: false,
      pendingReviewId: null,
      version: 1,
    },
    {
      id: "expense-card-payment",
      kind: "card_payment",
      category: "transfer_settlement",
      status: "excluded",
      amountMinor: 12_000,
      currency: "KRW",
      occurredAt: occurredAt(7),
      postedDate: `${month}-07`,
      sourceKind: "account",
      merchant: "KB국민카드",
      counterparty: null,
      memo: "카드대금 자동이체",
      paymentMethodFingerprint: "kb-account-sample",
      exclusionReason: "카드대금으로 자동 제외",
      duplicateOfEventId: null,
      personalAmountMinor: null,
      relatedEventId: null,
      isProvisional: false,
      pendingReviewId: null,
      version: 1,
    },
  ];
  const reviews: ExpenseReview[] = [
    {
      id: "review-p2p",
      reason: "unknown_p2p",
      status: "pending",
      transaction: transactions[3],
      recurringExpenseId: null,
      suggestedKind: "settlement_sent",
      suggestedCategory: "transfer_settlement",
      suggestedDuplicateOfEventId: null,
      createdAt: now(),
      resolvedAt: null,
      version: 1,
    },
  ];
  const recurring: RecurringExpenseItem[] = [
    {
      id: "recurring-rent",
      name: "월세",
      category: "housing_utilities",
      vendor: "임대인",
      amountMinor: 650_000,
      currency: "KRW",
      paymentMethodFingerprint: "kb-account-sample",
      startDate: `${month}-01`,
      endDate: null,
      memo: "매월 이체",
      reminderDays: 7,
      amountKind: "fixed",
      intervalMonths: 1,
      dueRule: "last_day",
      dueDay: null,
      status: "active",
      autoMatchEnabled: true,
      createdAt: now(),
      updatedAt: now(),
      version: 1,
    },
    {
      id: "recurring-insurance",
      name: "보험료",
      category: "insurance_finance_tax",
      vendor: "보험사",
      amountMinor: 89_000,
      currency: "KRW",
      paymentMethodFingerprint: "kb-card-sample",
      startDate: `${month}-01`,
      endDate: null,
      memo: null,
      reminderDays: 3,
      amountKind: "estimate",
      intervalMonths: 1,
      dueRule: "specific_day",
      dueDay: Number(today.slice(8, 10)),
      status: "active",
      autoMatchEnabled: false,
      createdAt: now(),
      updatedAt: now(),
      version: 1,
    },
    {
      id: "recurring-cloud",
      name: "Railway",
      category: "ott_subscriptions",
      vendor: "Railway",
      amountMinor: 30_000,
      currency: "USD",
      paymentMethodFingerprint: "kb-card-sample",
      startDate: `${month}-01`,
      endDate: null,
      memo: "월 최대 한도",
      reminderDays: 7,
      amountKind: "limit",
      intervalMonths: 1,
      dueRule: "specific_day",
      dueDay: Math.min(28, Number(today.slice(8, 10)) + 3),
      status: "active",
      autoMatchEnabled: false,
      createdAt: now(),
      updatedAt: now(),
      version: 1,
    },
  ];
  return { sources, transactions, reviews, recurring };
};

const sampleStockScreen = (today: string): {
  latest: LatestStockScreen;
  results: StockScreenResult[];
} => {
  const marketDate = shiftDate(today, -1);
  const completedAt = `${marketDate}T21:30:00Z`;
  const base = (
    ticker: string,
    displayName: string,
    exchange: "NASDAQ" | "NYSE" | "AMEX",
    horizon: StockScreenResult["horizon"],
    direction: StockScreenResult["direction"],
    band: StockScreenResult["band"],
    returnPct: number,
  ): StockScreenResult => ({
    runId: "stock-screen-sample",
    ticker,
    displayName,
    sector: exchange === "NASDAQ" ? "Information Technology" : "Industrials",
    horizon,
    direction,
    band,
    currentDate: marketDate,
    currentCloseMicrousd: Math.round(100_000_000 * (1 + returnPct / 100)),
    baselineDate: shiftDate(marketDate, horizon === 5 ? -7 : -30),
    baselineCloseMicrousd: 100_000_000,
    returnMicros: Math.round(returnPct * 1_000_000),
    returnPct,
    universeSha256: "a".repeat(64),
    marketDataSha256: "b".repeat(64),
  });
  const results: StockScreenResult[] = [
    base("NVDA", "NVIDIA Corporation", "NASDAQ", 5, "up", "up_10_to_20", 14.3),
    base("AMD", "Advanced Micro Devices, Inc.", "NASDAQ", 5, "up", "up_10_to_20", 11.2),
    base("META", "Meta Platforms, Inc.", "NASDAQ", 5, "up", "up_10_to_20", 10.08),
    base("TSLA", "Tesla, Inc.", "NASDAQ", 5, "down", "down_20_plus", -22.4),
    base("F", "Ford Motor Company", "NYSE", 21, "down", "down_10_to_20", -13.8),
    base("PLTR", "Palantir Technologies Inc.", "NASDAQ", 21, "up", "up_20_plus", 25.3),
  ];
  const coverage = {
    currentCovered: 503,
    total: 503,
    currentPct: 100,
    baseline5Covered: 501,
    baseline5Pct: 99.6,
    baseline21Covered: 500,
    baseline21Pct: 99.4,
  };
  return {
    latest: {
      latestSuccess: {
        runId: "stock-screen-sample",
        marketDate,
        completedAt,
        coverage,
        universe: {
          name: "S&P 500 구성종목",
          sourceUrl: "https://github.com/datasets/s-and-p-500-companies",
          revision: "sample-revision",
          asOfDate: marketDate,
          memberCount: 503,
          ageDays: 1,
          attributionText: "S&P 500 구성종목 공개 데이터 · DataHub/Wikipedia",
        },
        counts: {
          up5TenToTwenty: 3,
          up5TwentyPlus: 0,
          down5TenToTwenty: 0,
          down5TwentyPlus: 1,
          up21TenToTwenty: 0,
          up21TwentyPlus: 1,
          down21TenToTwenty: 1,
          down21TwentyPlus: 0,
        },
        top3: results.slice(0, 3),
        ai: {
          id: "stock-ai-sample",
          screenRunId: "stock-screen-sample",
          status: "succeeded",
          promptVersion: "stock-daily-v1",
          model: "gpt-5.4-nano-2026-03-17",
          responseId: "resp_stock_sample",
          upstreamRequestId: null,
          requestStartedAt: completedAt,
          result: {
            headline: "단기 상승 종목이 하락 종목보다 많았습니다.",
            bullets: [
              "5거래일 10~20% 상승 구간에 3개 종목이 있습니다.",
              "수치는 분할 조정 종가로 계산했으며 투자 추천이 아닙니다.",
            ],
            notableSymbols: ["NVDA", "AMD"],
          },
          inputTokens: 900,
          cachedInputTokens: 0,
          outputTokens: 110,
          totalTokens: 1_010,
          estimatedCostMicrousd: 2_800,
          failureCode: null,
          createdAt: completedAt,
          completedAt,
        },
      },
      latestAttempt: {
        runId: "stock-screen-sample",
        status: "succeeded",
        marketDate,
        startedAt: `${marketDate}T21:28:00Z`,
        completedAt,
        failureCode: null,
        coverage,
      },
      aiBudget: {
        budgetMonth: today.slice(0, 7),
        operation: "stock_daily_report",
        hardLimitMicrousd: 2_000_000,
        committedMicrousd: 2_800,
        remainingMicrousd: 1_997_200,
        hardStopReached: false,
      },
      stale: false,
      stalenessReason: null,
    },
    results,
  };
};

export class MemoryTransport implements CommandTransport {
  private snapshot = buildSample();
  private taskReport: TaskReportResult | null = null;
  private calendarEvents = sampleCalendarEvents(this.snapshot.today);
  private expenses = sampleExpenses(this.snapshot.today);
  private recurringOccurrenceOverrides = new Map<string, RecurringExpenseOccurrence>();
  private expenseImportPreviews = new Map<string, ExpenseImportPreview>();
  private expenseReport: ExpenseReportResult | null = null;
  private stockWatchlist: StockWatchlistItem[] = [];
  private stockScreen = sampleStockScreen(this.snapshot.today);

  async invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    const result = this.handle(command, args);
    return structuredClone(result) as T;
  }

  private handle(command: string, args: Record<string, unknown>): unknown {
    switch (command) {
      case "get_app_snapshot":
        return this.snapshot;
      case "get_cost_status":
        return {
          api: {
            budgetMonth: this.snapshot.today.slice(0, 7),
            usedMicrousd: 11_209,
            hardLimitMicrousd: 20_000_000,
          },
          cloud: {
            available: true,
            usedMicrousd: 86_783,
            hardLimitMicrousd: 30_000_000,
            billingPeriodStart: "2026-07-13T12:53:10Z",
            billingPeriodEnd: "2026-08-13T12:53:10Z",
            refreshedAt: now(),
            stale: false,
          },
        };
      case "get_calendar_month":
        return this.getCalendarMonth(textValue(args.month));
      case "create_calendar_event":
        return this.createCalendarEvent(args.input as CreateCalendarEventInput);
      case "update_calendar_event":
        return this.updateCalendarEvent(
          textValue(args.eventId),
          args.input as UpdateCalendarEventInput,
        );
      case "delete_calendar_event":
        return this.deleteCalendarEvent(textValue(args.eventId), Number(args.expectedVersion));
      case "get_expense_summary":
        return this.getExpenseSummary(textValue(args.month));
      case "list_expense_sources":
        return this.expenses.sources;
      case "update_expense_source":
        return this.updateExpenseSource(
          textValue(args.sourceId),
          args.input as UpdateExpenseSourceStatusInput,
        );
      case "list_expense_transactions":
        return this.listExpenseTransactions(args.input as ListExpenseTransactionsInput);
      case "override_expense_transaction":
        return this.overrideExpenseTransaction(
          textValue(args.eventId),
          args.input as OverrideExpenseTransactionInput,
        );
      case "list_expense_reviews":
        return this.listExpenseReviews(args.input as ListExpenseReviewsInput);
      case "resolve_expense_review":
        return this.resolveExpenseReview(
          textValue(args.reviewId),
          args.input as ResolveExpenseReviewInput,
        );
      case "list_recurring_expenses":
        return this.expenses.recurring;
      case "list_recurring_expense_occurrences":
        return this.listRecurringExpenseOccurrences(textValue(args.month));
      case "create_recurring_expense":
        return this.createRecurringExpense(args.input as CreateRecurringExpenseInput);
      case "update_recurring_expense":
        return this.updateRecurringExpense(
          textValue(args.recurringExpenseId),
          args.input as UpdateRecurringExpenseInput,
        );
      case "delete_recurring_expense":
        return this.deleteRecurringExpense(
          textValue(args.recurringExpenseId),
          Number(args.expectedVersion),
        );
      case "confirm_recurring_expense_paid":
        return this.confirmRecurringExpensePaid(
          textValue(args.occurrenceKey),
          args.amountMinor === null || args.amountMinor === undefined
            ? null
            : Number(args.amountMinor),
          Number(args.expectedVersion),
        );
      case "match_recurring_expense_occurrence":
        return this.matchRecurringExpenseOccurrence(
          textValue(args.occurrenceKey),
          textValue(args.eventId),
          Boolean(args.enableFutureAutoMatch),
          Number(args.expectedVersion),
        );
      case "preview_expense_import":
        return this.previewExpenseImport(args.input as PreviewExpenseImportInput);
      case "commit_expense_import":
        return this.commitExpenseImport(textValue(args.sessionId));
      case "latest_expense_report":
        return this.expenseReport?.month === textValue(args.month) ? this.expenseReport : null;
      case "generate_expense_report":
        return this.generateExpenseReport(textValue(args.month));
      case "rate_expense_report":
        if (!this.expenseReport || this.expenseReport.reportId !== args.reportId) {
          throw new Error("지출 리포트를 찾지 못했습니다.");
        }
        this.expenseReport.helpful = Boolean(args.helpful);
        return this.expenseReport;
      case "get_stock_watchlist":
        return this.stockWatchlist;
      case "upsert_stock_watchlist_item":
        return this.upsertStockWatchlistItem(args.input as UpsertStockWatchlistItemInput);
      case "delete_stock_watchlist_item":
        return this.deleteStockWatchlistItem(textValue(args.symbol));
      case "get_latest_stock_screen":
        return this.stockScreen.latest;
      case "list_stock_screen_results":
        return this.listStockScreenResults(args as unknown as ListStockScreenResultsInput);
      case "create_project":
        return this.createProject(textValue(args.name), textValue(args.description));
      case "create_task":
        return this.createTask(args.input as CreateTaskInput);
      case "update_task":
        return this.updateTask(args.input as UpdateTaskInput);
      case "plan_task":
        return this.planTask(textValue(args.taskId), textValue(args.date));
      case "resolve_day_entry":
        return this.resolveDayEntry(
          textValue(args.entryId),
          args.status as Exclude<DayEntryStatus, "planned">,
        );
      case "start_session":
        return this.startSession(args.input as StartSessionInput);
      case "finish_session":
        return this.finishSession(args.input as FinishSessionInput);
      case "create_work_log":
        return this.createWorkLog(args.input as CreateWorkLogInput);
      case "create_note":
        return this.createNote(args.input as CreateNoteInput);
      case "create_change_request":
        return this.createChangeRequest(args.input as CreateChangeRequestInput);
      case "update_change_request":
        return this.updateChangeRequest(args.input as UpdateChangeRequestInput);
      case "approve_change_request":
        return this.approveChangeRequest(
          textValue(args.changeRequestId),
          Number(args.expectedRevision),
          Number(args.expectedAttemptCount),
        );
      case "return_change_request_to_draft":
        return this.returnChangeRequestToDraft(
          textValue(args.changeRequestId),
          Number(args.expectedRevision),
          Number(args.expectedAttemptCount),
        );
      case "cancel_change_request":
        return this.cancelChangeRequest(
          textValue(args.changeRequestId),
          Number(args.expectedRevision),
          Number(args.expectedAttemptCount),
          textValue(args.reason),
        );
      case "abandon_change_request":
        return this.abandonChangeRequest(
          textValue(args.changeRequestId),
          Number(args.expectedRevision),
          Number(args.expectedAttemptCount),
          textValue(args.reason),
        );
      case "promote_work_log":
        return this.promoteWorkLog(
          textValue(args.workLogId),
          args.noteType as NoteType,
          textValue(args.title),
        );
      case "search":
        return this.search(textValue(args.query));
      case "move_to_trash":
        return this.moveToTrash(textValue(args.itemId), textValue(args.entityType));
      case "restore_trash_item":
        this.snapshot.trash = this.snapshot.trash.filter((item) => item.id !== args.itemId);
        return null;
      case "create_backup":
        return this.createBackup();
      case "restore_backup":
        return null;
      case "export_all":
        return {
          jsonPath: `C:\\Users\\tkfk0\\Desktop\\codex\\TM\\exports\\tm-${this.snapshot.today}.json`,
          markdownPath: `C:\\Users\\tkfk0\\Desktop\\codex\\TM\\exports\\tm-${this.snapshot.today}.md`,
          exportedAt: now(),
        } satisfies ExportResult;
      case "latest_task_report":
        return this.taskReport;
      case "generate_task_report":
        return this.generateTaskReport();
      case "rate_task_report":
        if (!this.taskReport || this.taskReport.runId !== args.reportId) {
          throw new Error("Task 리포트를 찾지 못했습니다.");
        }
        this.taskReport.helpful = Boolean(args.helpful);
        return this.taskReport;
      default:
        throw new Error(`지원하지 않는 mock command: ${command}`);
    }
  }

  private generateTaskReport(): TaskReportResult {
    const candidates = this.snapshot.tasks
      .filter((task) => !task.deletedAt && !["done", "cancelled"].includes(task.status))
      .slice(0, 3);
    const startDate = this.snapshot.today;
    const end = new Date(`${startDate}T00:00:00Z`);
    end.setUTCDate(end.getUTCDate() + 6);
    const endDate = end.toISOString().slice(0, 10);
    const months = [...new Set([startDate.slice(0, 7), endDate.slice(0, 7)])];
    const scheduleHighlights = months
      .flatMap((month) => this.getCalendarMonth(month).occurrences)
      .filter((item) => item.date >= startDate && item.date <= endDate)
      .sort((left, right) => left.date.localeCompare(right.date) || (left.eventTime || "").localeCompare(right.eventTime || ""))
      .slice(0, 5)
      .map((item) => ({
        occurrenceKey: item.occurrenceKey,
        eventId: item.eventId,
        title: item.title,
        kind: item.kind,
        date: item.date,
        eventTime: item.eventTime,
        reason: item.date === startDate ? "오늘 확인할 일정입니다." : "앞으로 7일 안에 예정된 일정입니다.",
        alert: item.kind === "payment" ? "납부 여부를 확인하세요." : "",
      }));
    const hasBriefingItems = candidates.length > 0 || scheduleHighlights.length > 0;
    this.taskReport = {
      runId: id("report"),
      reportDate: this.snapshot.today,
      status: hasBriefingItems ? "succeeded" : "no_tasks",
      report: {
        headline: hasBriefingItems ? "오늘의 Task와 일정을 확인하세요" : "오늘 처리할 Task와 일정이 없습니다",
        summary: hasBriefingItems ? "마감일과 진행 상태, 앞으로 7일의 일정을 함께 확인했습니다." : "새 Task나 일정을 추가하면 우선순위와 알림을 제안합니다.",
        priorities: candidates.map((task, index) => ({
          taskId: task.id,
          rank: index + 1,
          reason: index === 0 ? "현재 상태와 우선순위를 함께 고려했습니다." : "다음으로 이어서 처리하기 좋습니다.",
          nextAction: `${task.title}의 첫 단계를 10분 동안 시작하기`,
          alert: task.status === "blocked" ? "막힘 원인을 먼저 확인하세요." : "",
        })),
        scheduleHighlights,
        alerts: [],
      },
      candidateCount: candidates.length + scheduleHighlights.length,
      model: "mock-step17",
      promptVersion: "calendar-task-report-v1",
      usage: hasBriefingItems ? { inputTokens: 320, cachedInputTokens: 0, outputTokens: 180, totalTokens: 500 } : null,
      estimatedCostMicrousd: hasBriefingItems ? 3500 : 0,
      latencyMs: hasBriefingItems ? 420 : 0,
      helpful: null,
      readOnly: true,
      limits: { dailyCalls: 4, maximumCostMicrousd: 50_000, maximumOutputTokens: 800 },
    };
    return this.taskReport;
  }

  private getExpenseSummary(month: string): ExpenseMonthSummary {
    if (!/^\d{4}-\d{2}$/.test(month)) throw new Error("월 형식이 올바르지 않습니다.");
    const transactions = this.expenses.transactions.filter((item) =>
      item.occurredAt.startsWith(`${month}-`));
    const currencies = [...new Set([
      ...transactions.map((item) => item.currency),
      ...this.expenses.recurring.map((item) => item.currency),
    ])].sort();
    const recurring = this.listRecurringExpenseOccurrences(month);
    const totals = currencies.map((currency) => {
      const items = transactions.filter((item) => item.currency === currency && item.status === "confirmed");
      const sum = (kind: ExpenseEventKind) => items
        .filter((item) => item.kind === kind)
        .reduce((total, item) => total + item.amountMinor, 0);
      const expected = recurring
        .filter((item) => item.currency === currency)
        .reduce((total, item) => total + item.expectedAmountMinor, 0);
      const paid = recurring
        .filter((item) => item.currency === currency && ["matched", "paid"].includes(item.status))
        .reduce((total, item) => total + (item.actualAmountMinor ?? item.expectedAmountMinor), 0);
      const grossPurchaseMinor = sum("purchase");
      const refundsMinor = sum("refund");
      const settlementReceivedMinor = sum("settlement_received");
      const settlementSentMinor = sum("settlement_sent");
      const feesMinor = sum("fee");
      return {
        currency,
        netPersonalSpendMinor: grossPurchaseMinor - refundsMinor
          - settlementReceivedMinor + settlementSentMinor + feesMinor,
        grossPurchaseMinor,
        refundsMinor,
        settlementReceivedMinor,
        settlementSentMinor,
        feesMinor,
        unconfirmedOutflowMinor: transactions
          .filter((item) => item.currency === currency && item.kind === "unknown_p2p")
          .reduce((total, item) => total + item.amountMinor, 0),
        recurringExpectedMinor: expected,
        recurringPaidMinor: paid,
        recurringRemainingMinor: Math.max(0, expected - paid),
      };
    });
    const categories = [...new Set(transactions.map((item) => item.category))]
      .flatMap((category) => currencies.map((currency) => ({
        category,
        currency,
        amountMinor: transactions
          .filter((item) => item.category === category && item.currency === currency && item.status === "confirmed")
          .reduce((total, item) => total + (item.kind === "refund" ? -item.amountMinor : item.amountMinor), 0),
      })))
      .filter((item) => item.amountMinor !== 0);
    const daily = transactions
      .filter((item) => item.status === "confirmed")
      .map((item) => ({
        date: item.occurredAt.slice(0, 10),
        currency: item.currency,
        amountMinor: item.kind === "refund" ? -item.amountMinor : item.amountMinor,
      }));
    const reviews = this.expenses.reviews.filter((item) => item.transaction.occurredAt.startsWith(`${month}-`));
    const [year, monthNumber] = month.split("-").map(Number);
    const monthEnd = `${month}-${String(new Date(Date.UTC(year, monthNumber, 0)).getUTCDate()).padStart(2, "0")}`;
    return {
      month,
      monthStart: `${month}-01`,
      monthEnd,
      status: reviews.length > 0 ? "provisional" : "confirmed",
      currencies: totals,
      categories,
      daily,
      recurringCandidates: 1,
      completeness: {
        activeSourceCount: this.expenses.sources.filter((source) => source.isActive && source.requiredForCompleteReport).length,
        coveredSourceCount: this.expenses.sources.filter((source) => source.isActive && source.requiredForCompleteReport && source.coverageStart && source.coverageEnd).length,
        rejectedRowCount: 0,
        pendingReviewCount: reviews.length,
      },
    };
  }

  private updateExpenseSource(
    sourceId: string,
    input: UpdateExpenseSourceStatusInput,
  ): ExpenseSourceStatus {
    const source = this.expenses.sources.find((item) => item.id === sourceId);
    if (!source) throw new Error("지출 출처를 찾지 못했습니다.");
    if (source.version !== input.expectedVersion) throw new Error("지출 출처가 먼저 변경되었습니다.");
    source.requiredForCompleteReport = input.requiredForCompleteReport;
    source.isActive = input.isActive;
    source.version += 1;
    return source;
  }

  private listExpenseTransactions(input: ListExpenseTransactionsInput): ExpenseTransactionPage {
    const offset = Number(input.cursor ?? 0);
    const limit = Math.min(100, Math.max(1, input.limit ?? 50));
    const filtered = this.expenses.transactions
      .filter((item) => item.occurredAt.startsWith(`${input.month}-`))
      .sort((left, right) => right.occurredAt.localeCompare(left.occurredAt));
    return {
      items: filtered.slice(offset, offset + limit),
      nextCursor: offset + limit < filtered.length ? String(offset + limit) : null,
    };
  }

  private overrideExpenseTransaction(
    eventId: string,
    input: OverrideExpenseTransactionInput,
  ): ExpenseTransaction {
    const transaction = this.expenses.transactions.find((item) => item.id === eventId);
    if (!transaction) throw new Error("거래를 찾지 못했습니다.");
    if (transaction.kind === "manual_recurring") {
      throw new Error("수동 납부 거래는 정기지출 발생 건에서 변경해 주세요.");
    }
    if (transaction.version !== input.expectedVersion) throw new Error("거래가 먼저 변경되었습니다.");
    transaction.kind = input.kind;
    transaction.category = input.category;
    transaction.duplicateOfEventId = input.duplicateOfEventId;
    if (input.personalAmountMinor !== null && input.personalAmountMinor !== undefined) {
      transaction.personalAmountMinor = input.personalAmountMinor;
      transaction.relatedEventId = null;
    } else if (input.clearPersonalAmount) {
      transaction.personalAmountMinor = null;
    }
    if (input.relatedEventId !== null && input.relatedEventId !== undefined) {
      transaction.relatedEventId = input.relatedEventId;
      transaction.personalAmountMinor = null;
    } else if (input.clearRelatedEvent) {
      transaction.relatedEventId = null;
    }
    const excludedKinds: ExpenseEventKind[] = ["internal_transfer", "card_payment", "wallet_topup"];
    transaction.status = input.duplicateOfEventId || excludedKinds.includes(input.kind)
      ? "excluded"
      : "confirmed";
    transaction.exclusionReason = input.duplicateOfEventId
      ? "사용자가 중복 거래로 분류"
      : excludedKinds.includes(input.kind) ? "사용자가 비지출 이동으로 분류" : null;
    transaction.isProvisional = false;
    transaction.version += 1;
    return transaction;
  }

  private listExpenseReviews(input: ListExpenseReviewsInput): ExpenseReviewPage {
    const offset = Number(input.cursor ?? 0);
    const limit = Math.min(100, Math.max(1, input.limit ?? 50));
    const filtered = this.expenses.reviews
      .filter((item) => !input.month || item.transaction.occurredAt.startsWith(`${input.month}-`))
      .filter((item) => !input.status || item.status === input.status)
      .sort((left, right) => right.transaction.occurredAt.localeCompare(left.transaction.occurredAt));
    return {
      items: filtered.slice(offset, offset + limit),
      nextCursor: offset + limit < filtered.length ? String(offset + limit) : null,
    };
  }

  private resolveExpenseReview(reviewId: string, input: ResolveExpenseReviewInput): null {
    const review = this.expenses.reviews.find((item) => item.id === reviewId);
    if (!review) throw new Error("검토 항목을 찾지 못했습니다.");
    if (review.version !== input.expectedVersion) throw new Error("검토 항목이 먼저 변경되었습니다.");
    const transaction = this.expenses.transactions.find((item) => item.id === review.transaction.id);
    if (!transaction) throw new Error("연결된 거래를 찾지 못했습니다.");
    transaction.kind = input.kind;
    transaction.category = input.category;
    transaction.duplicateOfEventId = input.duplicateOfEventId;
    transaction.status = input.duplicateOfEventId
      ? "excluded"
      : ["internal_transfer", "card_payment", "wallet_topup"].includes(input.kind)
      ? "excluded"
      : "confirmed";
    this.expenses.reviews = this.expenses.reviews.filter((item) => item.id !== reviewId);
    return null;
  }

  private listRecurringExpenseOccurrences(month: string): RecurringExpenseOccurrence[] {
    if (!/^\d{4}-\d{2}$/.test(month)) throw new Error("월 형식이 올바르지 않습니다.");
    const [year, monthNumber] = month.split("-").map(Number);
    const targetIndex = year * 12 + monthNumber - 1;
    const lastDay = new Date(Date.UTC(year, monthNumber, 0)).getUTCDate();
    return this.expenses.recurring.flatMap((item) => {
      if (item.status !== "active") return [];
      const [startYear, startMonth] = item.startDate.slice(0, 7).split("-").map(Number);
      const startIndex = startYear * 12 + startMonth - 1;
      if (targetIndex < startIndex || (targetIndex - startIndex) % item.intervalMonths !== 0) return [];
      const day = item.dueRule === "first_day"
        ? 1
        : item.dueRule === "last_day"
          ? lastDay
          : Math.min(item.dueDay ?? 1, lastDay);
      const dueDate = `${month}-${String(day).padStart(2, "0")}`;
      if (dueDate < item.startDate || (item.endDate && dueDate > item.endDate)) return [];
      const occurrenceKey = `${item.id}:${dueDate}`;
      const overridden = this.recurringOccurrenceOverrides.get(occurrenceKey);
      if (overridden) return [overridden];
      return [{
        occurrenceKey,
        recurringExpenseId: item.id,
        itemVersion: item.version,
        name: item.name,
        category: item.category,
        vendor: item.vendor,
        expectedAmountMinor: item.amountMinor,
        actualAmountMinor: null,
        currency: item.currency,
        dueDate,
        status: dueDate < this.snapshot.today
          ? "overdue"
          : dueDate === this.snapshot.today
            ? "due_today"
            : dueDate <= shiftDate(this.snapshot.today, 7) ? "due_soon" : "scheduled",
        reminderDays: item.reminderDays,
        amountChanged: false,
        actualEventId: null,
        version: 1,
      } satisfies RecurringExpenseOccurrence];
    }).sort((left, right) => left.dueDate.localeCompare(right.dueDate)
      || left.name.localeCompare(right.name, "ko-KR"));
  }

  private createRecurringExpense(input: CreateRecurringExpenseInput): RecurringExpenseItem {
    this.validateRecurringExpense(input);
    const timestamp = now();
    const item: RecurringExpenseItem = {
      id: id("recurring"),
      ...input,
      autoMatchEnabled: false,
      createdAt: timestamp,
      updatedAt: timestamp,
      version: 1,
    };
    this.expenses.recurring.push(item);
    return item;
  }

  private updateRecurringExpense(
    recurringExpenseId: string,
    input: UpdateRecurringExpenseInput,
  ): RecurringExpenseItem {
    const item = this.expenses.recurring.find((entry) => entry.id === recurringExpenseId);
    if (!item) throw new Error("정기지출을 찾지 못했습니다.");
    if (item.version !== input.expectedVersion) throw new Error("정기지출이 먼저 변경되었습니다.");
    if (input.autoMatchEnabled && !item.autoMatchEnabled) {
      throw new Error("자동 연결은 첫 실제 거래 연결에서만 켤 수 있습니다.");
    }
    this.validateRecurringExpense(input);
    const { expectedVersion, effectiveFromMonth, ...patch } = input;
    void expectedVersion;
    void effectiveFromMonth;
    Object.assign(item, patch, { version: item.version + 1 });
    return item;
  }

  private deleteRecurringExpense(recurringExpenseId: string, expectedVersion: number): null {
    const item = this.expenses.recurring.find((entry) => entry.id === recurringExpenseId);
    if (!item) throw new Error("정기지출을 찾지 못했습니다.");
    if (item.version !== expectedVersion) throw new Error("정기지출이 먼저 변경되었습니다.");
    this.expenses.recurring = this.expenses.recurring.filter((entry) => entry.id !== item.id);
    return null;
  }

  private confirmRecurringExpensePaid(
    occurrenceKey: string,
    amountMinor: number | null,
    expectedVersion: number,
  ): RecurringExpenseOccurrence {
    const occurrence = this.findRecurringOccurrence(occurrenceKey);
    if (occurrence.version !== expectedVersion) throw new Error("정기지출 납부 상태가 먼저 변경되었습니다.");
    const updated = {
      ...occurrence,
      actualAmountMinor: amountMinor ?? occurrence.expectedAmountMinor,
      status: "paid" as const,
      amountChanged: amountMinor !== null && amountMinor !== occurrence.expectedAmountMinor,
      version: occurrence.version + 1,
    };
    this.recurringOccurrenceOverrides.set(occurrenceKey, updated);
    return updated;
  }

  private matchRecurringExpenseOccurrence(
    occurrenceKey: string,
    eventId: string,
    enableFutureAutoMatch: boolean,
    expectedVersion: number,
  ): RecurringExpenseOccurrence {
    const occurrence = this.findRecurringOccurrence(occurrenceKey);
    if (occurrence.version !== expectedVersion) throw new Error("정기지출 납부 상태가 먼저 변경되었습니다.");
    const transaction = this.expenses.transactions.find((item) => item.id === eventId);
    if (!transaction) throw new Error("연결할 거래를 찾지 못했습니다.");
    const updated = {
      ...occurrence,
      actualAmountMinor: transaction.amountMinor,
      status: "matched" as const,
      amountChanged: transaction.amountMinor !== occurrence.expectedAmountMinor,
      actualEventId: eventId,
      version: occurrence.version + 1,
    };
    this.recurringOccurrenceOverrides.set(occurrenceKey, updated);
    if (enableFutureAutoMatch) {
      const recurring = this.expenses.recurring.find((item) => item.id === occurrence.recurringExpenseId);
      if (recurring) {
        recurring.autoMatchEnabled = true;
        recurring.paymentMethodFingerprint ??= transaction.paymentMethodFingerprint;
      }
    }
    return updated;
  }

  private findRecurringOccurrence(occurrenceKey: string): RecurringExpenseOccurrence {
    const month = occurrenceKey.slice(-10, -3);
    const occurrence = this.listRecurringExpenseOccurrences(month)
      .find((item) => item.occurrenceKey === occurrenceKey);
    if (!occurrence) throw new Error("정기지출 발생 건을 찾지 못했습니다.");
    return occurrence;
  }

  private validateRecurringExpense(input: CreateRecurringExpenseInput): void {
    if (!input.name.trim()) throw new Error("정기지출 이름을 입력하세요.");
    if (input.name.trim().length > 120 || (input.vendor?.trim().length ?? 0) > 200 || (input.memo?.trim().length ?? 0) > 500) {
      throw new Error("이름 120자, 업체 200자, 메모 500자 이내로 입력해 주세요.");
    }
    if (!Number.isSafeInteger(input.amountMinor) || input.amountMinor < 0) {
      throw new Error("금액을 올바르게 입력하세요.");
    }
    if (input.dueRule === "specific_day" && (!input.dueDay || input.dueDay > 31)) {
      throw new Error("결제일은 1일부터 31일까지 입력하세요.");
    }
  }

  private previewExpenseImport(input: PreviewExpenseImportInput): ExpenseImportPreview {
    if (!input.path.trim()) throw new Error("가져올 파일을 선택하세요.");
    const emptyCounts = {
      parsed: 0,
      new: 0,
      duplicate: 0,
      settlementCandidate: 0,
      excluded: 0,
      unconfirmed: 0,
      rejected: 0,
    };
    if (/kakao/i.test(input.path) && !input.password) {
      return {
        status: "password_required",
        sessionId: null,
        adapter: "kakaopay_money_v1",
        sourceLabel: "카카오페이머니",
        periodStart: null,
        periodEnd: null,
        passwordRequired: true,
        counts: emptyCounts,
        rows: [],
      };
    }
    const sessionId = id("expense-import");
    const preview: ExpenseImportPreview = {
      status: "ready",
      sessionId,
      adapter: /kakao/i.test(input.path) ? "kakaopay_money_v1" : "kb_card_usage_v1",
      sourceLabel: /kakao/i.test(input.path) ? "카카오페이머니" : "KB 신용카드",
      periodStart: this.snapshot.today,
      periodEnd: this.snapshot.today,
      passwordRequired: false,
      counts: {
        parsed: 3,
        new: 2,
        duplicate: 1,
        settlementCandidate: 1,
        excluded: 0,
        unconfirmed: 1,
        rejected: 0,
      },
      rows: [
        { rowNumber: 2, occurredAt: `${this.snapshot.today}T12:00:00+09:00`, amountMinor: 18_000, currency: "KRW", displayName: "합성 식당", kind: "purchase", needsReview: false, excluded: false },
        { rowNumber: 3, occurredAt: `${this.snapshot.today}T14:00:00+09:00`, amountMinor: 20_000, currency: "KRW", displayName: "합성 송금", kind: "unknown_p2p", needsReview: true, excluded: false },
      ],
    };
    this.expenseImportPreviews.set(sessionId, preview);
    return preview;
  }

  private commitExpenseImport(sessionId: string): ExpenseImportCommitResult {
    const preview = this.expenseImportPreviews.get(sessionId);
    if (!preview) throw new Error("가져오기 미리보기가 만료되었거나 이미 사용되었습니다.");
    this.expenseImportPreviews.delete(sessionId);
    return {
      batchId: id("expense-batch"),
      sourceId: id("expense-source"),
      rowCount: preview.counts.parsed,
      newCount: preview.counts.new,
      duplicateCount: preview.counts.duplicate,
      rejectedCount: preview.counts.rejected,
      excludedCount: preview.counts.excluded,
      reviewCount: preview.counts.unconfirmed + preview.counts.settlementCandidate,
      idempotentReplay: false,
    };
  }

  private generateExpenseReport(month: string): ExpenseReportResult {
    const summary = this.getExpenseSummary(month);
    const krw = summary.currencies.find((item) => item.currency === "KRW");
    this.expenseReport = {
      reportId: id("expense-report"),
      month,
      title: `${Number(month.slice(5, 7))}월 지출 해설`,
      summary: "확정 집계만 사용해 지출 흐름과 다음 달 확인 항목을 정리했습니다.",
      observations: [{ factIds: ["currency:KRW:net_personal_spend"], text: `원화 순 개인지출 집계가 준비되었습니다 (${krw ? "확정값" : "자료 없음"}).` }],
      alerts: summary.completeness.pendingReviewCount > 0
        ? [{ factIds: ["currency:KRW:unconfirmed_outflow"], text: "확인하지 않은 송금이 있어 리포트가 잠정 상태입니다." }]
        : [],
      nextMonthChecks: ["정기지출 실제 납부액과 예상액 차이를 확인하세요."],
      facts: [
        {
          factId: "currency:KRW:net_personal_spend",
          metric: "net_personal_spend",
          currency: "KRW",
          amountMinor: krw?.netPersonalSpendMinor ?? 0,
        },
        {
          factId: "currency:KRW:unconfirmed_outflow",
          metric: "unconfirmed_outflow",
          currency: "KRW",
          amountMinor: krw?.unconfirmedOutflowMinor ?? 0,
        },
      ],
      helpful: null,
    };
    return this.expenseReport;
  }

  private getCalendarMonth(month: string): CalendarMonth {
    if (!/^\d{4}-\d{2}$/.test(month)) throw new Error("월 형식이 올바르지 않습니다.");
    const [year, monthNumber] = month.split("-").map(Number);
    if (monthNumber < 1 || monthNumber > 12) throw new Error("월 형식이 올바르지 않습니다.");
    const lastDay = new Date(Date.UTC(year, monthNumber, 0)).getUTCDate();
    const monthStart = `${month}-01`;
    const monthEnd = `${month}-${String(lastDay).padStart(2, "0")}`;
    const occurrences = this.calendarEvents.flatMap((event) => {
      if (event.deletedAt) return [];
      let day: number | null = null;
      if (event.recurrence === "none") {
        if (!event.startDate.startsWith(`${month}-`)) return [];
        day = Number(event.startDate.slice(8, 10));
      } else if (event.recurrence === "monthly_first_day") day = 1;
      else if (event.recurrence === "monthly_last_day") day = lastDay;
      else if (event.dayOfMonth && event.dayOfMonth <= lastDay) day = event.dayOfMonth;
      if (day === null) return [];
      const date = `${month}-${String(day).padStart(2, "0")}`;
      if (date < event.startDate || (event.endsOn && date > event.endsOn)) return [];
      return [{
        occurrenceKey: `${event.id}:${date}`,
        eventId: event.id,
        title: event.title,
        description: event.description,
        kind: event.kind,
        date,
        eventTime: event.eventTime,
        recurrence: event.recurrence,
      }];
    }).sort((left, right) => left.date.localeCompare(right.date)
      || (left.eventTime ?? "").localeCompare(right.eventTime ?? "")
      || left.title.localeCompare(right.title, "ko-KR"));
    return {
      month,
      monthStart,
      monthEnd,
      events: this.calendarEvents.filter((event) => !event.deletedAt),
      occurrences,
      expenseOccurrences: this.listRecurringExpenseOccurrences(month),
    };
  }

  private createCalendarEvent(input: CreateCalendarEventInput): CalendarEvent {
    this.validateCalendarInput(input);
    const timestamp = now();
    const event: CalendarEvent = {
      id: id("calendar"),
      ...input,
      eventTime: input.eventTime ? `${input.eventTime.slice(0, 5)}:00` : null,
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
      version: 1,
    };
    this.calendarEvents.push(event);
    return event;
  }

  private updateCalendarEvent(
    eventId: string,
    input: UpdateCalendarEventInput,
  ): CalendarEvent {
    const event = this.calendarEvents.find((item) => item.id === eventId && !item.deletedAt);
    if (!event) throw new Error("일정을 찾지 못했습니다.");
    if (event.version !== input.expectedVersion) throw new Error("일정이 먼저 변경되었습니다.");
    this.validateCalendarInput(input);
    Object.assign(event, input, {
      eventTime: input.eventTime ? `${input.eventTime.slice(0, 5)}:00` : null,
      updatedAt: now(),
      version: event.version + 1,
    });
    return event;
  }

  private deleteCalendarEvent(eventId: string, expectedVersion: number): null {
    const event = this.calendarEvents.find((item) => item.id === eventId && !item.deletedAt);
    if (!event) throw new Error("일정을 찾지 못했습니다.");
    if (event.version !== expectedVersion) throw new Error("일정이 먼저 변경되었습니다.");
    event.deletedAt = now();
    event.updatedAt = event.deletedAt;
    event.version += 1;
    return null;
  }

  private validateCalendarInput(input: CreateCalendarEventInput): void {
    if (!input.title.trim()) throw new Error("일정 제목을 입력하세요.");
    if (input.recurrence === "monthly_day" && (!input.dayOfMonth || input.dayOfMonth > 31)) {
      throw new Error("매월 반복 날짜는 1일부터 31일까지 입력하세요.");
    }
    if (input.recurrence !== "monthly_day" && input.dayOfMonth !== null) {
      throw new Error("특정일 반복에서만 날짜를 지정할 수 있습니다.");
    }
    if (input.recurrence === "none" && input.endsOn !== null) {
      throw new Error("한 번 일정에는 반복 종료일을 지정할 수 없습니다.");
    }
  }

  private upsertStockWatchlistItem(
    input: UpsertStockWatchlistItemInput,
  ): StockWatchlistItem {
    const ticker = input.ticker.trim().toUpperCase();
    const displayName = input.displayName.trim();
    const validTicker = input.market === "KRX"
      ? /^\d{6}$/.test(ticker)
      : /^[A-Z0-9.-]{1,10}$/.test(ticker);
    if (!validTicker) throw new Error("종목 코드 형식이 올바르지 않습니다.");
    if (!displayName || Array.from(displayName).length > 80) {
      throw new Error("표시 이름 형식이 올바르지 않습니다.");
    }
    const symbol = `${input.market}:${ticker}`;
    const existing = this.stockWatchlist.find((item) => item.symbol === symbol);
    if (!existing && this.stockWatchlist.length >= 50) {
      throw new Error("관심 종목은 최대 50개까지 저장할 수 있습니다.");
    }
    if (existing) {
      existing.displayName = displayName;
      existing.updatedAt = now();
      return existing;
    }
    const timestamp = now();
    const item: StockWatchlistItem = {
      symbol,
      market: input.market,
      ticker,
      displayName,
      createdAt: timestamp,
      updatedAt: timestamp,
    };
    this.stockWatchlist.push(item);
    return item;
  }

  private deleteStockWatchlistItem(symbol: string): null {
    this.stockWatchlist = this.stockWatchlist.filter((item) => item.symbol !== symbol);
    return null;
  }

  private listStockScreenResults(
    input: ListStockScreenResultsInput,
  ): StockScreenResultPage {
    const limit = Math.min(Math.max(Number(input.limit ?? 50), 1), 50);
    const offset = input.cursor ? Number.parseInt(input.cursor, 10) : 0;
    const resultBand = input.band
      ? `${input.direction}_${input.band === "ten_to_twenty" ? "10_to_20" : "20_plus"}`
      : null;
    const matching = this.stockScreen.results.filter((item) =>
      item.horizon === Number(input.horizon)
      && item.direction === input.direction
      && (!resultBand || item.band === resultBand));
    const items = matching.slice(offset, offset + limit);
    return {
      items,
      nextCursor: offset + items.length < matching.length
        ? String(offset + items.length)
        : null,
      total: matching.length,
    };
  }

  private createProject(name: string, description: string): string {
    const trimmed = name.trim();
    if (!trimmed) throw new Error("프로젝트 이름을 입력하세요.");
    const projectId = id("project");
    this.snapshot.projects.push({
      id: projectId,
      name: trimmed,
      description: description.trim(),
      color: "#7386ff",
      systemKey: null,
      openTaskCount: 0,
      completedTaskCount: 0,
      archived: false,
    });
    return projectId;
  }

  private moveToTrash(itemId: string, entityType: string): null {
    const deletedAt = now();
    if (entityType === "task") {
      const task = this.snapshot.tasks.find((item) => item.id === itemId);
      if (!task) throw new Error("Task를 찾지 못했습니다.");
      task.deletedAt = deletedAt;
      this.snapshot.trash.unshift({ id: task.id, entityType: "task", title: task.title, deletedAt });
    } else if (entityType === "note") {
      const note = this.snapshot.notes.find((item) => item.id === itemId);
      if (!note) throw new Error("Note를 찾지 못했습니다.");
      this.snapshot.notes = this.snapshot.notes.filter((item) => item.id !== itemId);
      this.snapshot.trash.unshift({ id: note.id, entityType: "note", title: note.title, deletedAt });
    } else if (entityType === "work_log") {
      const workLog = this.snapshot.workLogs.find((item) => item.id === itemId);
      if (!workLog) throw new Error("WorkLog를 찾지 못했습니다.");
      this.snapshot.workLogs = this.snapshot.workLogs.filter((item) => item.id !== itemId);
      this.snapshot.trash.unshift({ id: workLog.id, entityType: "work_log", title: workLog.title, deletedAt });
    }
    return null;
  }

  private createTask(input: CreateTaskInput): null {
    const project = this.taskProject(input.projectId);
    const timestamp = now();
    const status = input.status ?? "inbox";
    if (!["inbox", "todo", "in_progress", "blocked", "done", "cancelled"].includes(status)) {
      throw new Error(`알 수 없는 Task 상태입니다: ${status}`);
    }
    const task: Task = {
      id: id("task"),
      title: input.title.trim(),
      description: input.description ?? "",
      status,
      priority: input.priority ?? "none",
      dueDate: input.dueDate ?? null,
      projectId: project.id,
      projectName: project.name,
      tags: input.tags ?? [],
      checklist: [],
      workLogs: [],
      events: [
        {
          id: id("event"),
          taskId: "pending",
          eventType: "created",
          summary: "Task를 만들었습니다",
          before: null,
          after: { title: input.title, status },
          createdAt: timestamp,
        },
      ],
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
    };
    task.events[0].taskId = task.id;
    this.snapshot.tasks.unshift(task);
    return null;
  }

  private updateTask(input: UpdateTaskInput): null {
    const task = this.snapshot.tasks.find((item) => item.id === input.taskId);
    if (!task) throw new Error("Task를 찾을 수 없습니다.");
    const before = {
      title: task.title,
      description: task.description,
      status: task.status,
      priority: task.priority,
      dueDate: task.dueDate,
      projectId: task.projectId,
      tags: task.tags,
      checklist: task.checklist,
    };
    const project = this.taskProject(input.projectId, task.projectId);
    Object.assign(task, input, {
      projectId: project.id,
      projectName: project.name,
      updatedAt: now(),
    });
    task.events.unshift({
      id: id("event"),
      taskId: task.id,
      eventType: "updated",
      summary: "Task 정보를 변경했습니다",
      before,
      after: { ...input },
      createdAt: now(),
    });
    return null;
  }

  private taskProject(
    projectId: string | null | undefined,
    currentProjectId: string | null = null,
  ): Project {
    const normalizedProjectId = projectId ?? this.snapshot.projects.find(
      (project) => project.systemKey === UNCATEGORIZED_PROJECT_SYSTEM_KEY,
    )?.id;
    const project = this.snapshot.projects.find(
      (candidate) => candidate.id === normalizedProjectId,
    );
    if (!project) {
      throw new Error("프로젝트를 찾지 못했습니다.");
    }
    if (project.archived && project.id !== currentProjectId) {
      throw new Error("보관된 프로젝트에는 Task를 배정할 수 없습니다.");
    }
    return project;
  }

  private planTask(taskId: string, date: string): null {
    if (
      this.snapshot.todayView.planned.some(
        (entry) => entry.taskId === taskId && entry.date === date,
      )
    ) {
      return null;
    }
    const task = this.snapshot.tasks.find((item) => item.id === taskId);
    if (!task) throw new Error("Task를 찾을 수 없습니다.");
    const entry: TaskDayEntry = {
      id: id("day"),
      taskId,
      date,
      status: "planned",
      task,
      resolvedAt: null,
    };
    this.snapshot.todayView.planned.push(entry);
    return null;
  }

  private resolveDayEntry(
    entryId: string,
    status: Exclude<DayEntryStatus, "planned">,
  ): null {
    const sections = [
      this.snapshot.todayView.yesterdayIncomplete,
      this.snapshot.todayView.planned,
      this.snapshot.todayView.completed,
    ];
    const entry = sections.flat().find((item) => item.id === entryId);
    if (!entry || entry.status !== "planned") {
      throw new Error("확정된 오늘 기록은 다시 변경할 수 없습니다.");
    }
    entry.status = status;
    entry.resolvedAt = now();
    if (status === "done") {
      entry.task.status = "done";
      this.snapshot.todayView.planned = this.snapshot.todayView.planned.filter(
        (item) => item.id !== entryId,
      );
      this.snapshot.todayView.completed.push(entry);
    }
    if (status === "deferred" && entry.date !== this.snapshot.today) {
      this.planTask(entry.taskId, this.snapshot.today);
      this.snapshot.todayView.yesterdayIncomplete =
        this.snapshot.todayView.yesterdayIncomplete.filter((item) => item.id !== entryId);
    }
    if (status === "skipped") {
      this.snapshot.todayView.planned = this.snapshot.todayView.planned.filter(
        (item) => item.id !== entryId,
      );
    }
    return null;
  }

  private startSession(input: StartSessionInput): null {
    if (this.snapshot.activeSession) throw new Error("이미 실행 중인 세션이 있습니다.");
    const project = this.snapshot.projects.find((item) => item.id === input.projectId);
    this.snapshot.activeSession = {
      id: id("session"),
      projectId: input.projectId,
      projectName: project?.name ?? null,
      taskIds: input.taskIds,
      taskTitles: this.snapshot.tasks
        .filter((task) => input.taskIds.includes(task.id))
        .map((task) => task.title),
      goal: input.goal,
      startedAt: now(),
      endedAt: null,
      result: null,
      blockers: null,
      nextAction: null,
    };
    return null;
  }

  private finishSession(input: FinishSessionInput): null {
    const session = this.snapshot.activeSession;
    if (!session || session.id !== input.sessionId) throw new Error("실행 중인 세션이 없습니다.");
    session.endedAt = now();
    session.result = input.result;
    session.blockers = input.blockers || null;
    session.nextAction = input.nextAction || null;
    const log: WorkLog = {
      id: id("log"),
      taskIds: session.taskIds,
      sessionId: session.id,
      projectId: session.projectId,
      title: session.goal,
      content: input.result,
      outcome: input.result,
      blockers: input.blockers || null,
      createdAt: now(),
      promotedNoteId: null,
    };
    this.snapshot.workLogs.unshift(log);
    if (input.createNextTask && input.nextAction.trim()) {
      this.createTask({
        title: input.nextAction,
        projectId: session.projectId,
        priority: "medium",
      });
    }
    this.snapshot.recentSessions.unshift(session);
    this.snapshot.activeSession = null;
    return null;
  }

  private createWorkLog(input: CreateWorkLogInput): null {
    this.snapshot.workLogs.unshift({
      id: id("log"),
      ...input,
      outcome: input.outcome || null,
      blockers: input.blockers || null,
      createdAt: now(),
      promotedNoteId: null,
    });
    return null;
  }

  private createNote(input: CreateNoteInput): null {
    const timestamp = now();
    this.snapshot.notes.unshift({
      id: id("note"),
      ...input,
      createdAt: timestamp,
      updatedAt: timestamp,
      deletedAt: null,
    });
    return null;
  }

  private createChangeRequest(input: CreateChangeRequestInput): null {
    this.validateChangeRequestContent(input);
    const timestamp = now();
    this.snapshot.changeRequests.unshift({
      id: id("change"),
      ...input,
      status: "draft",
      revision: 1,
      approvedRevision: null,
      attemptCount: 0,
      requestedBy: "desktop-user",
      updatedBy: "desktop-user",
      createdAt: timestamp,
      updatedAt: timestamp,
      approvedAt: null,
      approvedBy: null,
      claimedAt: null,
      claimedBy: null,
      completedAt: null,
      completedBy: null,
      failedAt: null,
      failedBy: null,
      cancelledAt: null,
      cancelledBy: null,
      resultSummary: null,
      patchRef: null,
      failureReason: null,
      cancellationReason: null,
    });
    return null;
  }

  private updateChangeRequest(input: UpdateChangeRequestInput): null {
    const request = this.changeRequestForExpectedState(
      input.changeRequestId,
      input.expectedRevision,
      input.expectedAttemptCount,
    );
    if (request.status !== "draft") {
      throw new Error("초안 상태의 개선 요청만 편집할 수 있습니다.");
    }
    this.validateChangeRequestContent(input);
    Object.assign(request, {
      kind: input.kind,
      projectId: input.projectId,
      taskId: input.taskId,
      title: input.title.trim(),
      description: input.description.trim(),
      desiredOutcome: input.desiredOutcome.trim(),
      reproductionSteps: input.reproductionSteps.trim(),
      priority: input.priority,
      status: "draft",
      revision: request.revision + 1,
      approvedRevision: null,
      updatedBy: "desktop-user",
      updatedAt: now(),
      failedAt: null,
      failedBy: null,
      failureReason: null,
    });
    return null;
  }

  private approveChangeRequest(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ): null {
    const request = this.changeRequestForExpectedState(
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
    );
    if (request.status !== "draft" && request.status !== "failed") {
      throw new Error("초안 또는 실패한 개선 요청만 승인할 수 있습니다.");
    }
    request.status = "approved";
    request.approvedRevision = request.revision;
    request.approvedAt = now();
    request.approvedBy = "desktop-user";
    request.claimedAt = null;
    request.claimedBy = null;
    request.failedAt = null;
    request.failedBy = null;
    request.failureReason = null;
    request.updatedBy = "desktop-user";
    request.updatedAt = now();
    return null;
  }

  private returnChangeRequestToDraft(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ): null {
    const request = this.changeRequestForExpectedState(
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
    );
    if (request.status !== "approved" && request.status !== "failed") {
      throw new Error("승인 또는 실패 상태의 개선 요청만 초안으로 돌릴 수 있습니다.");
    }
    request.status = "draft";
    request.revision += 1;
    request.approvedRevision = null;
    request.approvedAt = null;
    request.approvedBy = null;
    request.claimedAt = null;
    request.claimedBy = null;
    request.failedAt = null;
    request.failedBy = null;
    request.failureReason = null;
    request.updatedBy = "desktop-user";
    request.updatedAt = now();
    return null;
  }

  private cancelChangeRequest(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ): null {
    const request = this.changeRequestForExpectedState(
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
    );
    if (!["draft", "approved", "failed"].includes(request.status)) {
      throw new Error("이 상태의 개선 요청은 취소할 수 없습니다.");
    }
    if (!reason.trim()) throw new Error("취소 이유를 입력하세요.");
    request.status = "cancelled";
    request.approvedRevision = null;
    request.approvedAt = null;
    request.approvedBy = null;
    request.claimedAt = null;
    request.claimedBy = null;
    request.failedAt = null;
    request.failedBy = null;
    request.failureReason = null;
    request.cancelledAt = now();
    request.cancelledBy = "desktop-user";
    request.cancellationReason = reason.trim();
    request.updatedBy = "desktop-user";
    request.updatedAt = now();
    return null;
  }

  private abandonChangeRequest(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ): null {
    const request = this.snapshot.changeRequests.find((item) => item.id === changeRequestId);
    if (!request) throw new Error("개선 요청을 찾을 수 없습니다.");
    if (
      request.revision !== expectedRevision
      || request.attemptCount !== expectedAttemptCount
    ) {
      throw new Error("claim 이후 요청이 변경되었습니다. 최신 상태를 확인한 뒤 다시 시도하세요.");
    }
    if (request.status !== "claimed") {
      throw new Error("처리 중인 개선 요청만 유실로 전환할 수 있습니다.");
    }
    if (!reason.trim()) throw new Error("처리 유실 이유를 입력하세요.");
    request.status = "failed";
    request.failedAt = now();
    request.failedBy = "desktop-user";
    request.failureReason = reason.trim();
    request.updatedBy = "desktop-user";
    request.updatedAt = now();
    return null;
  }

  private changeRequestForExpectedState(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ): ChangeRequest {
    const request = this.snapshot.changeRequests.find((item) => item.id === changeRequestId);
    if (!request) throw new Error("개선 요청을 찾을 수 없습니다.");
    if (
      request.revision !== expectedRevision
      || request.attemptCount !== expectedAttemptCount
    ) {
      throw new Error("다른 변경이 먼저 저장되었습니다. 최신 내용을 확인한 뒤 다시 시도하세요.");
    }
    return request;
  }

  private validateChangeRequestContent(input: CreateChangeRequestInput): void {
    if (!input.title.trim()) throw new Error("개선 요청 제목을 입력하세요.");
    if (!input.description.trim()) throw new Error("현재 상황을 설명하세요.");
    if (!input.desiredOutcome.trim()) throw new Error("원하는 결과를 입력하세요.");
  }

  private promoteWorkLog(workLogId: string, type: NoteType, title: string): null {
    const log = this.snapshot.workLogs.find((item) => item.id === workLogId);
    if (!log) throw new Error("WorkLog를 찾을 수 없습니다.");
    this.createNote({
      type,
      title,
      content: log.content,
      links: {
        ...emptyLinks(),
        taskIds: log.taskIds,
        sessionIds: log.sessionId ? [log.sessionId] : [],
        workLogIds: [log.id],
      },
    });
    log.promotedNoteId = this.snapshot.notes[0].id;
    return null;
  }

  private search(query: string): SearchResult[] {
    const normalized = query.trim().toLocaleLowerCase("ko-KR");
    if (!normalized) return [];
    const contains = (...values: Array<string | null>): boolean =>
      values.some((value) => value?.toLocaleLowerCase("ko-KR").includes(normalized));
    const taskResults: SearchResult[] = this.snapshot.tasks
      .filter((task) => contains(task.title, task.description, task.tags.join(" ")))
      .map((task) => ({
        id: task.id,
        entityType: "task",
        title: task.title,
        excerpt: task.description || task.tags.join(" · "),
        meta: task.projectName ?? "Inbox",
        taskId: task.id,
      }));
    const sessionResults: SearchResult[] = this.snapshot.recentSessions
      .filter((session) => contains(session.goal, session.result, session.blockers))
      .map((session) => ({
        id: session.id,
        entityType: "session",
        title: session.goal,
        excerpt: session.result ?? "",
        meta: session.projectName ?? "프로젝트 없음",
      }));
    const logResults: SearchResult[] = this.snapshot.workLogs
      .filter((log) => contains(log.title, log.content, log.outcome, log.blockers))
      .map((log) => ({
        id: log.id,
        entityType: "work_log",
        title: log.title,
        excerpt: log.content,
        meta: "WorkLog",
        taskId: log.taskIds[0],
      }));
    const noteResults: SearchResult[] = this.snapshot.notes
      .filter((note) => contains(note.title, note.content))
      .map((note) => ({
        id: note.id,
        entityType: "note",
        title: note.title,
        excerpt: note.content,
        meta: note.type,
      }));
    return [...taskResults, ...sessionResults, ...logResults, ...noteResults];
  }

  private createBackup(): null {
    const createdAt = now();
    this.snapshot.backups.unshift({
      id: id("backup"),
      fileName: `tm-${createdAt.replaceAll(/[:.-]/g, "").slice(0, 15)}-manual.sqlite3`,
      createdAt,
      sizeBytes: 1_835_008,
      trigger: "manual",
    });
    this.snapshot.lastBackupAt = createdAt;
    return null;
  }
}

export const createMemoryTransport = (): MemoryTransport => new MemoryTransport();
