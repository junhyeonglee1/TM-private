export type TaskStatus =
  | "inbox"
  | "todo"
  | "in_progress"
  | "blocked"
  | "done"
  | "cancelled";

export type Priority = "none" | "low" | "medium" | "high";
export type DayEntryStatus = "planned" | "done" | "deferred" | "skipped";
export type NoteType = "concept" | "howto" | "decision" | "reference" | "daily";
export type ChangeRequestKind = "bug" | "ui" | "feature" | "other";
export type ChangeRequestStatus =
  | "draft"
  | "approved"
  | "claimed"
  | "completed"
  | "failed"
  | "cancelled";
export type CalendarEventKind = "personal" | "payment";
export type CalendarRecurrence =
  | "none"
  | "monthly_day"
  | "monthly_first_day"
  | "monthly_last_day";

export interface CalendarEvent {
  id: string;
  title: string;
  description: string;
  kind: CalendarEventKind;
  startDate: string;
  eventTime: string | null;
  recurrence: CalendarRecurrence;
  dayOfMonth: number | null;
  endsOn: string | null;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
  version: number;
}

export interface CalendarOccurrence {
  occurrenceKey: string;
  eventId: string;
  title: string;
  description: string;
  kind: CalendarEventKind;
  date: string;
  eventTime: string | null;
  recurrence: CalendarRecurrence;
}

export interface CalendarMonth {
  month: string;
  monthStart: string;
  monthEnd: string;
  events: CalendarEvent[];
  occurrences: CalendarOccurrence[];
}

export interface CreateCalendarEventInput {
  title: string;
  description: string;
  kind: CalendarEventKind;
  startDate: string;
  eventTime: string | null;
  recurrence: CalendarRecurrence;
  dayOfMonth: number | null;
  endsOn: string | null;
}

export interface UpdateCalendarEventInput extends CreateCalendarEventInput {
  expectedVersion: number;
}

export type StockMarket = "KRX" | "NASDAQ" | "NYSE" | "AMEX";

export interface StockWatchlistItem {
  symbol: string;
  market: StockMarket;
  ticker: string;
  displayName: string;
  createdAt: string;
  updatedAt: string;
}

export interface UpsertStockWatchlistItemInput {
  market: StockMarket;
  ticker: string;
  displayName: string;
}

export type StockScreenHorizon = 5 | 21;
export type StockScreenDirection = "up" | "down";
export type StockScreenFilterBand = "ten_to_twenty" | "twenty_plus";
export type StockScreenResultBand =
  | "down_20_plus"
  | "down_10_to_20"
  | "up_10_to_20"
  | "up_20_plus";
export type StockScreenAiStatus = "started" | "succeeded" | "failed" | "ai_uncertain";
export type StockScreenAttemptStatus =
  | "started"
  | "succeeded"
  | "no_candidates"
  | "upstream_unavailable"
  | "coverage_failed"
  | "failed";

export interface StockScreenCoverage {
  currentCovered: number;
  total: number;
  currentPct: number;
  baseline5Covered: number;
  baseline5Pct: number;
  baseline21Covered: number;
  baseline21Pct: number;
}

export interface StockScreenUniverse {
  name: string;
  sourceUrl: string;
  revision: string;
  asOfDate: string;
  memberCount: number;
  ageDays: number;
  attributionText: string;
}

export interface StockScreenCounts {
  up5TenToTwenty: number;
  up5TwentyPlus: number;
  down5TenToTwenty: number;
  down5TwentyPlus: number;
  up21TenToTwenty: number;
  up21TwentyPlus: number;
  down21TenToTwenty: number;
  down21TwentyPlus: number;
}

export interface StockScreenResult {
  runId: string;
  ticker: string;
  displayName: string;
  sector: string | null;
  horizon: StockScreenHorizon;
  direction: StockScreenDirection;
  band: StockScreenResultBand;
  currentDate: string;
  currentCloseMicrousd: number;
  baselineDate: string;
  baselineCloseMicrousd: number;
  returnMicros: number;
  returnPct: number;
  universeSha256: string;
  marketDataSha256: string;
}

export interface StockScreenAiSummary {
  id: string;
  screenRunId: string;
  status: StockScreenAiStatus;
  promptVersion: string;
  model: string;
  responseId: string | null;
  upstreamRequestId: string | null;
  requestStartedAt: string | null;
  result: unknown;
  inputTokens: number | null;
  cachedInputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  estimatedCostMicrousd: number;
  failureCode: string | null;
  createdAt: string;
  completedAt: string | null;
}

export interface StockScreenSummary {
  runId: string;
  marketDate: string;
  completedAt: string;
  coverage: StockScreenCoverage;
  universe: StockScreenUniverse;
  counts: StockScreenCounts;
  top3: StockScreenResult[];
  ai: StockScreenAiSummary | null;
}

export interface StockScreenAttempt {
  runId: string;
  status: StockScreenAttemptStatus;
  marketDate: string;
  startedAt: string;
  completedAt: string | null;
  failureCode: string | null;
  coverage: StockScreenCoverage;
}

export interface LatestStockScreen {
  latestSuccess: StockScreenSummary | null;
  latestAttempt: StockScreenAttempt | null;
  aiBudget: {
    budgetMonth: string;
    operation: string;
    hardLimitMicrousd: number;
    committedMicrousd: number;
    remainingMicrousd: number;
    hardStopReached: boolean;
  };
  stale: boolean;
  stalenessReason: string | null;
}

export interface ListStockScreenResultsInput {
  runId: string;
  horizon: StockScreenHorizon;
  direction: StockScreenDirection;
  band?: StockScreenFilterBand;
  cursor?: string;
  limit?: number;
}

export interface StockScreenResultPage {
  items: StockScreenResult[];
  nextCursor: string | null;
  total: number;
}

export interface Project {
  id: string;
  name: string;
  description: string;
  color: string;
  openTaskCount: number;
  completedTaskCount: number;
  archived: boolean;
}

export interface ChecklistItem {
  id: string;
  label: string;
  checked: boolean;
}

export interface TaskEvent {
  id: string;
  taskId: string;
  eventType: string;
  summary: string;
  before: Record<string, unknown> | null;
  after: Record<string, unknown> | null;
  createdAt: string;
}

export interface WorkLog {
  id: string;
  taskIds: string[];
  sessionId: string | null;
  projectId: string | null;
  title: string;
  content: string;
  outcome: string | null;
  blockers: string | null;
  createdAt: string;
  promotedNoteId: string | null;
}

export interface Task {
  id: string;
  title: string;
  description: string;
  status: TaskStatus;
  priority: Priority;
  dueDate: string | null;
  projectId: string | null;
  projectName: string | null;
  tags: string[];
  checklist: ChecklistItem[];
  events: TaskEvent[];
  workLogs: WorkLog[];
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface TaskDayEntry {
  id: string;
  taskId: string;
  date: string;
  status: DayEntryStatus;
  task: Task;
  resolvedAt: string | null;
}

export interface WorkSession {
  id: string;
  projectId: string | null;
  projectName: string | null;
  taskIds: string[];
  taskTitles: string[];
  goal: string;
  startedAt: string;
  endedAt: string | null;
  result: string | null;
  blockers: string | null;
  nextAction: string | null;
}

export interface NoteLink {
  taskIds: string[];
  sessionIds: string[];
  workLogIds: string[];
  filePaths: string[];
  urls: string[];
}

export interface Note {
  id: string;
  type: NoteType;
  title: string;
  content: string;
  links: NoteLink;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface HistoryDay {
  date: string;
  entries: TaskDayEntry[];
  workLogCount: number;
  sessionMinutes: number;
}

export interface TrashItem {
  id: string;
  entityType: "task" | "note" | "project" | "work_log" | "session";
  title: string;
  deletedAt: string;
}

export interface BackupInfo {
  id: string;
  fileName: string;
  createdAt: string;
  sizeBytes: number;
  trigger: "startup" | "shutdown" | "daily" | "manual" | "pre_migration" | "pre_restore";
}

export interface ChangeRequest {
  id: string;
  kind: ChangeRequestKind;
  projectId: string | null;
  taskId: string | null;
  title: string;
  description: string;
  desiredOutcome: string;
  reproductionSteps: string;
  priority: Priority;
  status: ChangeRequestStatus;
  revision: number;
  approvedRevision: number | null;
  attemptCount: number;
  requestedBy: string;
  updatedBy: string;
  createdAt: string;
  updatedAt: string;
  approvedAt: string | null;
  approvedBy: string | null;
  claimedAt: string | null;
  claimedBy: string | null;
  completedAt: string | null;
  completedBy: string | null;
  failedAt: string | null;
  failedBy: string | null;
  cancelledAt: string | null;
  cancelledBy: string | null;
  resultSummary: string | null;
  patchRef: string | null;
  failureReason: string | null;
  cancellationReason: string | null;
}

export interface TodaySnapshot {
  yesterdayIncomplete: TaskDayEntry[];
  planned: TaskDayEntry[];
  inProgress: Task[];
  completed: TaskDayEntry[];
}

export interface AppSnapshot {
  generatedAt: string;
  today: string;
  projects: Project[];
  tasks: Task[];
  todayView: TodaySnapshot;
  history: HistoryDay[];
  activeSession: WorkSession | null;
  recentSessions: WorkSession[];
  workLogs: WorkLog[];
  notes: Note[];
  changeRequests: ChangeRequest[];
  trash: TrashItem[];
  backups: BackupInfo[];
  databasePath: string;
  lastBackupAt: string | null;
}

export interface SearchResult {
  id: string;
  entityType: "task" | "session" | "work_log" | "note";
  title: string;
  excerpt: string;
  meta: string;
  taskId?: string;
}

export interface CreateTaskInput {
  title: string;
  description?: string;
  projectId?: string | null;
  status?: TaskStatus;
  priority?: Priority;
  dueDate?: string | null;
  tags?: string[];
}

export interface UpdateTaskInput {
  taskId: string;
  title: string;
  description: string;
  status: TaskStatus;
  priority: Priority;
  dueDate: string | null;
  projectId: string | null;
  tags: string[];
  checklist: ChecklistItem[];
}

export interface StartSessionInput {
  projectId: string | null;
  taskIds: string[];
  goal: string;
}

export interface FinishSessionInput {
  sessionId: string;
  result: string;
  blockers: string;
  nextAction: string;
  createNextTask: boolean;
}

export interface CreateWorkLogInput {
  title: string;
  content: string;
  outcome: string;
  blockers: string;
  projectId: string | null;
  taskIds: string[];
  sessionId: string | null;
}

export interface CreateNoteInput {
  type: NoteType;
  title: string;
  content: string;
  links: NoteLink;
}

export interface CreateChangeRequestInput {
  kind: ChangeRequestKind;
  projectId: string | null;
  taskId: string | null;
  title: string;
  description: string;
  desiredOutcome: string;
  reproductionSteps: string;
  priority: Priority;
}

export interface UpdateChangeRequestInput extends CreateChangeRequestInput {
  changeRequestId: string;
  expectedRevision: number;
  expectedAttemptCount: number;
}

export interface ExportResult {
  jsonPath: string;
  markdownPath: string;
  exportedAt: string;
}
