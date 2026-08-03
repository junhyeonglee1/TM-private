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

export const UNCATEGORIZED_PROJECT_SYSTEM_KEY = "uncategorized" as const;

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
  expenseOccurrences: RecurringExpenseOccurrence[];
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
  systemKey: typeof UNCATEGORIZED_PROJECT_SYSTEM_KEY | null;
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

export type ExpenseCategory =
  | "food"
  | "delivery"
  | "cafe"
  | "groceries"
  | "housing_utilities"
  | "transportation"
  | "ott_subscriptions"
  | "shopping"
  | "health"
  | "leisure"
  | "education"
  | "travel"
  | "insurance_finance_tax"
  | "gifts_dues"
  | "refund_income"
  | "transfer_settlement"
  | "other"
  | "unconfirmed";

export type ExpenseEventKind =
  | "purchase"
  | "refund"
  | "settlement_received"
  | "settlement_sent"
  | "fee"
  | "internal_transfer"
  | "card_payment"
  | "wallet_topup"
  | "external_transfer"
  | "unknown_p2p"
  | "manual_recurring";

export type EditableExpenseEventKind = Exclude<
  ExpenseEventKind,
  "external_transfer" | "unknown_p2p" | "manual_recurring"
>;

export type ExpenseReportStatus = "confirmed" | "provisional" | "incomplete";
export type ExpenseSourceKind = "card" | "account" | "wallet";
export type ExpenseAdapter =
  | "kb_card_usage_v1"
  | "kb_account_history_v1"
  | "kakaopay_money_v1";

export interface ExpenseSourceStatus {
  id: string;
  adapter: ExpenseAdapter;
  sourceKind: ExpenseSourceKind;
  requiredForCompleteReport: boolean;
  isActive: boolean;
  coverageStart: string | null;
  coverageEnd: string | null;
  version: number;
}

export interface UpdateExpenseSourceStatusInput {
  requiredForCompleteReport: boolean;
  isActive: boolean;
  expectedVersion: number;
}

export interface ExpenseCurrencyTotals {
  currency: string;
  netPersonalSpendMinor: number;
  grossPurchaseMinor: number;
  refundsMinor: number;
  settlementReceivedMinor: number;
  settlementSentMinor: number;
  feesMinor: number;
  unconfirmedOutflowMinor: number;
  recurringExpectedMinor: number;
  recurringPaidMinor: number;
  recurringRemainingMinor: number;
}

export interface ExpenseCategoryTotal {
  category: ExpenseCategory;
  currency: string;
  amountMinor: number;
}

export interface ExpenseDailyTotal {
  date: string;
  currency: string;
  amountMinor: number;
}

export interface ExpenseMonthSummary {
  month: string;
  monthStart: string;
  monthEnd: string;
  status: ExpenseReportStatus;
  currencies: ExpenseCurrencyTotals[];
  categories: ExpenseCategoryTotal[];
  daily: ExpenseDailyTotal[];
  recurringCandidates: number;
  completeness: {
    activeSourceCount: number;
    coveredSourceCount: number;
    rejectedRowCount: number;
    pendingReviewCount: number;
  };
}

export type ExpenseClassificationSource = "deterministic" | "user_rule" | "manual" | "ai";

export interface ExpenseClassificationRunResult {
  runId: string | null;
  targetMonth: string;
  status: "applied" | "no_candidates" | "stale";
  candidateGroupCount: number;
  affectedTransactionCount: number;
  autoConfirmedCount: number;
  provisionalCount: number;
  manualReviewCount: number;
  privacySkippedCount: number;
  versionConflictCount: number;
  cached: boolean;
  costMicrousd: number;
  attemptNumber: number | null;
  monthlyLimitMicrousd: number;
  remainingMicrousd: number;
  completedAt: string;
}

export interface ExpenseTransaction {
  id: string;
  kind: ExpenseEventKind;
  category: ExpenseCategory;
  status: "confirmed" | "unconfirmed" | "excluded";
  amountMinor: number;
  currency: string;
  occurredAt: string;
  postedDate: string;
  sourceKind: ExpenseSourceKind | null;
  merchant: string | null;
  counterparty: string | null;
  memo: string | null;
  paymentMethodFingerprint: string | null;
  exclusionReason: string | null;
  duplicateOfEventId: string | null;
  personalAmountMinor: number | null;
  relatedEventId: string | null;
  isProvisional: boolean;
  pendingReviewId: string | null;
  classificationSource: ExpenseClassificationSource;
  classificationConfidence: number | null;
  version: number;
}

export interface ExpenseTransactionPage {
  items: ExpenseTransaction[];
  nextCursor: string | null;
}

export interface ListExpenseTransactionsInput {
  month: string;
  cursor?: string;
  limit?: number;
}

export interface ExpenseReview {
  id: string;
  reason: "unknown_p2p" | "ambiguous_mirror" | "recurring_match_candidate" | "recurring_registration_candidate" | "category_confirmation" | "import_rejected";
  status: "pending" | "resolved";
  transaction: ExpenseTransaction;
  recurringExpenseId: string | null;
  suggestedKind: ExpenseEventKind | null;
  suggestedCategory: ExpenseCategory | null;
  suggestedDuplicateOfEventId: string | null;
  suggestionSource: ExpenseClassificationSource | null;
  suggestionConfidence: number | null;
  createdAt: string;
  resolvedAt: string | null;
  version: number;
}

export interface ExpenseReviewPage {
  items: ExpenseReview[];
  nextCursor: string | null;
}

export interface ListExpenseReviewsInput {
  month?: string;
  status?: "pending" | "resolved";
  scope?: "all" | "required" | "category_confirmation";
  cursor?: string;
  limit?: number;
}

export interface ResolveExpenseReviewInput {
  kind: EditableExpenseEventKind;
  category: ExpenseCategory;
  expectedVersion: number;
  duplicateOfEventId: string | null;
  relatedEventId?: string | null;
  personalAmountMinor?: number | null;
  createRule: boolean;
}

export interface OverrideExpenseTransactionInput extends ResolveExpenseReviewInput {
  clearPersonalAmount: boolean;
  clearRelatedEvent: boolean;
}

export type RecurringAmountKind = "fixed" | "estimate" | "limit";
export type RecurringDueRule = "specific_day" | "first_day" | "last_day";
export type RecurringExpenseStatus = "active" | "paused" | "ended";
export type RecurringOccurrenceStatus = "scheduled" | "due_today" | "due_soon" | "overdue" | "paid" | "matched";

export interface RecurringExpenseItem {
  id: string;
  name: string;
  category: ExpenseCategory;
  vendor: string | null;
  amountMinor: number;
  currency: string;
  paymentMethodFingerprint: string | null;
  startDate: string;
  endDate: string | null;
  memo: string | null;
  reminderDays: number;
  amountKind: RecurringAmountKind;
  intervalMonths: 1 | 2 | 3 | 6 | 12;
  dueRule: RecurringDueRule;
  dueDay: number | null;
  status: RecurringExpenseStatus;
  autoMatchEnabled: boolean;
  createdAt: string;
  updatedAt: string;
  version: number;
}

export interface RecurringExpenseOccurrence {
  occurrenceKey: string;
  recurringExpenseId: string;
  itemVersion: number;
  name: string;
  category: ExpenseCategory;
  vendor: string | null;
  expectedAmountMinor: number;
  actualAmountMinor: number | null;
  currency: string;
  dueDate: string;
  status: RecurringOccurrenceStatus;
  amountChanged: boolean;
  reminderDays: number;
  actualEventId: string | null;
  version: number;
}

export interface CreateRecurringExpenseInput {
  name: string;
  category: ExpenseCategory;
  vendor: string | null;
  amountMinor: number;
  currency: string;
  paymentMethodFingerprint: string | null;
  startDate: string;
  endDate: string | null;
  memo: string | null;
  reminderDays: number;
  amountKind: RecurringAmountKind;
  intervalMonths: 1 | 2 | 3 | 6 | 12;
  dueRule: RecurringDueRule;
  dueDay: number | null;
  status: RecurringExpenseStatus;
}

export interface UpdateRecurringExpenseInput extends CreateRecurringExpenseInput {
  expectedVersion: number;
  effectiveFromMonth: string;
  autoMatchEnabled: boolean;
}

export interface ExpenseImportPreviewRow {
  rowNumber: number;
  occurredAt: string;
  amountMinor: number;
  currency: string;
  displayName: string;
  kind: string;
  needsReview: boolean;
  excluded: boolean;
}

export interface ExpenseImportPreview {
  status: "ready" | "password_required";
  sessionId: string | null;
  adapter: ExpenseAdapter | null;
  sourceLabel: string | null;
  periodStart: string | null;
  periodEnd: string | null;
  passwordRequired: boolean;
  counts: {
    parsed: number;
    new: number;
    duplicate: number;
    settlementCandidate: number;
    excluded: number;
    unconfirmed: number;
    rejected: number;
  };
  rows: ExpenseImportPreviewRow[];
}

export interface PreviewExpenseImportInput {
  path: string;
  password?: string;
}

export interface ExpenseImportCommitResult {
  batchId: string;
  sourceId: string;
  rowCount: number;
  newCount: number;
  duplicateCount: number;
  rejectedCount: number;
  excludedCount: number;
  reviewCount: number;
  idempotentReplay: boolean;
}

export interface ExpenseReportResult {
  reportId: string;
  month: string;
  title: string;
  summary: string;
  observations: Array<{ factIds: string[]; text: string }>;
  alerts: Array<{ factIds: string[]; text: string }>;
  nextMonthChecks: string[];
  facts: Array<{
    factId: string;
    metric: string;
    currency: string;
    amountMinor: number;
  }>;
  helpful: boolean | null;
}
