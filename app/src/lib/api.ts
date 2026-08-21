import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import type {
  AppSnapshot,
  CalendarEvent,
  CalendarMonth,
  CreateCalendarEventInput,
  CreateChangeRequestInput,
  CreateNoteInput,
  CreateTaskInput,
  CreateWorkLogInput,
  DayEntryStatus,
  ExpenseImportCommitResult,
  ExpenseImportPreview,
  ExpenseClassificationRunResult,
  ExpenseMonthSummary,
  ExpenseSourceStatus,
  ExpenseReportResult,
  ExpenseReviewPage,
  ExpenseTransaction,
  ExpenseTransactionPage,
  ExportResult,
  FinishSessionInput,
  LatestStockScreen,
  ListExpenseReviewsInput,
  ListExpenseTransactionsInput,
  ListStockScreenResultsInput,
  NoteType,
  PreviewExpenseImportInput,
  OverrideExpenseTransactionInput,
  RecurringExpenseItem,
  RecurringExpenseOccurrence,
  ResolveExpenseReviewInput,
  SearchResult,
  StartSessionInput,
  StockWatchlistItem,
  StockScreenResultPage,
  UpdateTaskInput,
  UpdateChangeRequestInput,
  UpdateCalendarEventInput,
  UpdateRecurringExpenseInput,
  UpdateExpenseSourceStatusInput,
  UpsertStockWatchlistItemInput,
  CreateRecurringExpenseInput,
  MailAccount,
  MailClassification,
  MailItem,
  MailItemPage,
  MailReport,
  MailSummary,
} from "../types";
import { createMemoryTransport } from "./mock-transport";

export interface CommandTransport {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
}

export interface TaskReportPriority {
  taskId: string;
  rank: number;
  reason: string;
  nextAction: string;
  alert: string;
}

export interface TaskReportScheduleHighlight {
  occurrenceKey: string;
  eventId: string;
  title: string;
  kind: "personal" | "payment";
  date: string;
  eventTime: string | null;
  reason: string;
  alert: string;
}

export interface TaskReportResult {
  runId: string;
  reportDate: string;
  status: "succeeded" | "no_tasks";
  report: {
    headline: string;
    summary: string;
    priorities: TaskReportPriority[];
    scheduleHighlights: TaskReportScheduleHighlight[];
    alerts: string[];
  };
  candidateCount: number;
  model: string;
  promptVersion: string;
  usage: null | {
    inputTokens: number;
    cachedInputTokens: number;
    outputTokens: number;
    totalTokens: number;
  };
  estimatedCostMicrousd: number;
  latencyMs: number | null;
  helpful: boolean | null;
  readOnly: true;
  limits: {
    dailyCalls: number;
    maximumCostMicrousd: number;
    maximumOutputTokens: number;
  };
}

export interface CostStatus {
  api: {
    budgetMonth: string;
    usedMicrousd: number;
    hardLimitMicrousd: number;
  };
  cloud: {
    available: boolean;
    usedMicrousd: number | null;
    hardLimitMicrousd: number;
    billingPeriodStart: string | null;
    billingPeriodEnd: string | null;
    refreshedAt: string | null;
    stale: boolean;
  };
}

export interface TmApi {
  readonly canImportExpenses: boolean;
  readonly canManageMailAccounts: boolean;
  getSnapshot(): Promise<AppSnapshot>;
  getCostStatus(): Promise<CostStatus>;
  getCalendarMonth(month: string): Promise<CalendarMonth>;
  createCalendarEvent(input: CreateCalendarEventInput): Promise<CalendarEvent>;
  updateCalendarEvent(eventId: string, input: UpdateCalendarEventInput): Promise<CalendarEvent>;
  deleteCalendarEvent(eventId: string, expectedVersion: number): Promise<void>;
  getStockWatchlist(): Promise<StockWatchlistItem[]>;
  upsertStockWatchlistItem(input: UpsertStockWatchlistItemInput): Promise<StockWatchlistItem>;
  deleteStockWatchlistItem(symbol: string): Promise<void>;
  getLatestStockScreen(): Promise<LatestStockScreen>;
  listStockScreenResults(input: ListStockScreenResultsInput): Promise<StockScreenResultPage>;
  createProject(name: string, description?: string): Promise<string>;
  createTask(input: CreateTaskInput): Promise<void>;
  updateTask(input: UpdateTaskInput): Promise<void>;
  planTask(taskId: string, date: string): Promise<void>;
  resolveDayEntry(entryId: string, status: Exclude<DayEntryStatus, "planned">): Promise<void>;
  startSession(input: StartSessionInput): Promise<void>;
  finishSession(input: FinishSessionInput): Promise<void>;
  createWorkLog(input: CreateWorkLogInput): Promise<void>;
  createNote(input: CreateNoteInput): Promise<void>;
  createChangeRequest(input: CreateChangeRequestInput): Promise<void>;
  updateChangeRequest(input: UpdateChangeRequestInput): Promise<void>;
  approveChangeRequest(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ): Promise<void>;
  returnChangeRequestToDraft(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ): Promise<void>;
  cancelChangeRequest(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ): Promise<void>;
  abandonChangeRequest(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ): Promise<void>;
  promoteWorkLog(workLogId: string, type: NoteType, title: string): Promise<void>;
  search(query: string): Promise<SearchResult[]>;
  moveToTrash(itemId: string, entityType: "task" | "note" | "project" | "work_log" | "session"): Promise<void>;
  restoreTrashItem(itemId: string): Promise<void>;
  createBackup(): Promise<void>;
  restoreBackup(backupId: string): Promise<void>;
  exportAll(): Promise<ExportResult>;
  generateTaskReport(): Promise<TaskReportResult>;
  latestTaskReport(): Promise<TaskReportResult | null>;
  rateTaskReport(reportId: string, helpful: boolean): Promise<TaskReportResult>;
  getExpenseSummary(month: string): Promise<ExpenseMonthSummary>;
  listExpenseSources(): Promise<ExpenseSourceStatus[]>;
  updateExpenseSource(
    sourceId: string,
    input: UpdateExpenseSourceStatusInput,
  ): Promise<ExpenseSourceStatus>;
  listExpenseTransactions(input: ListExpenseTransactionsInput): Promise<ExpenseTransactionPage>;
  overrideExpenseTransaction(
    eventId: string,
    input: OverrideExpenseTransactionInput,
  ): Promise<ExpenseTransaction>;
  listExpenseReviews(input: ListExpenseReviewsInput): Promise<ExpenseReviewPage>;
  resolveExpenseReview(reviewId: string, input: ResolveExpenseReviewInput): Promise<void>;
  classifyExpenseTransactions(month: string): Promise<ExpenseClassificationRunResult>;
  listRecurringExpenses(): Promise<RecurringExpenseItem[]>;
  listRecurringExpenseOccurrences(month: string): Promise<RecurringExpenseOccurrence[]>;
  createRecurringExpense(input: CreateRecurringExpenseInput): Promise<RecurringExpenseItem>;
  updateRecurringExpense(
    recurringExpenseId: string,
    input: UpdateRecurringExpenseInput,
  ): Promise<RecurringExpenseItem>;
  deleteRecurringExpense(recurringExpenseId: string, expectedVersion: number): Promise<void>;
  confirmRecurringExpensePaid(
    occurrenceKey: string,
    amountMinor: number | null,
    paidDate: string | null,
    expectedVersion: number,
  ): Promise<RecurringExpenseOccurrence>;
  matchRecurringExpenseOccurrence(
    occurrenceKey: string,
    eventId: string,
    enableFutureAutoMatch: boolean,
    expectedVersion: number,
  ): Promise<RecurringExpenseOccurrence>;
  previewExpenseImport(input: PreviewExpenseImportInput): Promise<ExpenseImportPreview>;
  commitExpenseImport(sessionId: string): Promise<ExpenseImportCommitResult>;
  generateExpenseReport(month: string): Promise<ExpenseReportResult>;
  latestExpenseReport(month: string): Promise<ExpenseReportResult | null>;
  rateExpenseReport(reportId: string, helpful: boolean): Promise<ExpenseReportResult>;
  discardExpenseMutation(command: ExpenseMutationCommandName, resourceId?: string): void;
  getMailAccounts(): Promise<MailAccount[]>;
  getMailSummary(date: string): Promise<MailSummary>;
  listMailItems(input: {
    status?: MailClassification;
    accountId?: string;
    cursor?: string;
    limit?: number;
  }): Promise<MailItemPage>;
  getMailReports(date: string): Promise<MailReport[]>;
  startGmailOAuth(): Promise<{ authorizationUrl: string; expiresInSeconds: number }>;
  connectNaverMail(input: { email: string; displayName?: string; appPassword: string }): Promise<MailAccount>;
  disconnectMailAccount(accountId: string, expectedVersion: number): Promise<MailAccount>;
  acknowledgeMailItem(itemId: string, expectedVersion: number): Promise<MailItem>;
  feedbackMailItem(itemId: string, important: boolean, expectedVersion: number): Promise<MailItem>;
}

export type ExpenseMutationCommandName =
  | "update_expense_source"
  | "override_expense_transaction"
  | "resolve_expense_review"
  | "classify_expense_transactions"
  | "create_recurring_expense"
  | "update_recurring_expense"
  | "delete_recurring_expense"
  | "confirm_recurring_expense_paid"
  | "match_recurring_expense_occurrence"
  | "generate_expense_report"
  | "rate_expense_report";

type MailMutationCommandName =
  | "start_gmail_oauth"
  | "connect_naver_mail"
  | "disconnect_mail_account"
  | "acknowledge_mail_item"
  | "feedback_mail_item";

class TauriTransport implements CommandTransport {
  private readonly mode = tauriInvoke<"local" | "cloud">("data_mode");

  async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    if (command === "get_cost_status") {
      return tauriInvoke<T>("get_cost_status");
    }
    if (["preview_expense_import", "commit_expense_import"].includes(command)) {
      return tauriInvoke<T>(command, args);
    }
    if ((await this.mode) === "cloud") {
      if (["generate_task_report", "latest_task_report", "rate_task_report"].includes(command)) {
        return tauriInvoke<T>("invoke_cloud_assistant_feature", {
          command,
          args: args ?? {},
        });
      }
      if (command.includes("expense")) {
        return tauriInvoke<T>("invoke_cloud_expense_feature", {
          command,
          args: args ?? {},
        });
      }
      if (command.includes("mail") || command === "start_gmail_oauth") {
        return tauriInvoke<T>("invoke_cloud_mail_feature", {
          command,
          args: args ?? {},
        });
      }
      return tauriInvoke<T>("invoke_cloud_command", { command, args: args ?? {} });
    }
    return tauriInvoke<T>(command, args);
  }
}

const run = async (
  transport: CommandTransport,
  command: string,
  args?: Record<string, unknown>,
): Promise<void> => {
  await transport.invoke<unknown>(command, args);
};

const expenseClassificationTerminalCodes = new Set([
  "EXPENSE_CLASSIFICATION_BUDGET_EXHAUSTED",
  "EXPENSE_CLASSIFICATION_BUDGET_PERSISTENCE_FAILED",
  "EXPENSE_CLASSIFICATION_COST_INVALID",
  "EXPENSE_CLASSIFICATION_IDEMPOTENCY_CONFLICT",
  "EXPENSE_CLASSIFICATION_IDEMPOTENCY_TERMINAL",
  "EXPENSE_CLASSIFICATION_LEASE_EXPIRED",
  "EXPENSE_CLASSIFICATION_OPENAI_AUTHENTICATION_FAILED",
  "EXPENSE_CLASSIFICATION_OPENAI_RATE_LIMITED",
  "EXPENSE_CLASSIFICATION_OPENAI_REQUEST_REJECTED",
  "EXPENSE_CLASSIFICATION_OPENAI_RESPONSE_INVALID",
  "EXPENSE_CLASSIFICATION_OPENAI_UNAVAILABLE",
  "EXPENSE_CLASSIFICATION_PREVIOUS_RUN_RECOVERED",
  "EXPENSE_CLASSIFICATION_PREVIOUSLY_FAILED",
  "EXPENSE_CLASSIFICATION_STAGE_FAILED",
  "EXPENSE_CLASSIFICATION_TIMEOUT",
]);

export const expenseClassificationTerminalErrorCode = (error: unknown): string | null => {
  const directCode = error && typeof error === "object" && "code" in error
    ? Reflect.get(error, "code")
    : null;
  if (typeof directCode === "string" && expenseClassificationTerminalCodes.has(directCode)) {
    return directCode;
  }
  const message = error instanceof Error ? error.message : String(error ?? "");
  const markedCode = message.match(/\[TM_ERROR_CODE:([A-Z_]+)\]/)?.[1] ?? null;
  return markedCode && expenseClassificationTerminalCodes.has(markedCode) ? markedCode : null;
};

export const isExpenseClassificationTerminalError = (error: unknown): boolean =>
  expenseClassificationTerminalErrorCode(error) !== null;

export const createApi = (
  transport: CommandTransport,
  options: { canImportExpenses?: boolean } = {},
): TmApi => {
  const pendingExpenseMutations = new Map<string, { fingerprint: string; key: string }>();
  const expenseMutationScope = (command: ExpenseMutationCommandName, resourceId = "") =>
    `${command}:${resourceId}`;
  const discardExpenseMutation = (command: ExpenseMutationCommandName, resourceId = "") => {
    pendingExpenseMutations.delete(expenseMutationScope(command, resourceId));
  };
  const invokeExpenseMutation = async <T>(
    command: ExpenseMutationCommandName,
    args: Record<string, unknown>,
    resourceId = "",
  ): Promise<T> => {
    const scope = expenseMutationScope(command, resourceId);
    const fingerprint = JSON.stringify(args);
    let pending = pendingExpenseMutations.get(scope);
    if (!pending || pending.fingerprint !== fingerprint) {
      pending = { fingerprint, key: `desktop-expense:${crypto.randomUUID()}` };
      pendingExpenseMutations.set(scope, pending);
    }
    let result: T;
    try {
      result = await transport.invoke<T>(command, {
        ...args,
        idempotencyKey: pending.key,
      });
    } catch (error) {
      if (
        command === "classify_expense_transactions"
        && isExpenseClassificationTerminalError(error)
        && pendingExpenseMutations.get(scope)?.key === pending.key
      ) {
        pendingExpenseMutations.delete(scope);
      }
      throw error;
    }
    if (pendingExpenseMutations.get(scope)?.key === pending.key) {
      pendingExpenseMutations.delete(scope);
    }
    return result;
  };
  const pendingMailMutations = new Map<string, { fingerprint: string; key: string }>();
  const mailMutationFingerprint = async (value: unknown): Promise<string> => {
    const bytes = new TextEncoder().encode(JSON.stringify(value));
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
  };
  const invokeMailMutation = async <T>(
    command: MailMutationCommandName,
    args: Record<string, unknown>,
    resourceId = "",
  ): Promise<T> => {
    const scope = `${command}:${await mailMutationFingerprint(resourceId)}`;
    const fingerprint = await mailMutationFingerprint(args);
    let pending = pendingMailMutations.get(scope);
    if (!pending || pending.fingerprint !== fingerprint) {
      pending = { fingerprint, key: `desktop-mail:${crypto.randomUUID()}` };
      pendingMailMutations.set(scope, pending);
    }
    const result = await transport.invoke<T>(command, {
      ...args,
      idempotencyKey: pending.key,
    });
    if (pendingMailMutations.get(scope)?.key === pending.key) {
      pendingMailMutations.delete(scope);
    }
    return result;
  };

  return {
  canImportExpenses: options.canImportExpenses ?? false,
  canManageMailAccounts: options.canImportExpenses ?? false,
  getSnapshot: () => transport.invoke<AppSnapshot>("get_app_snapshot"),
  getCostStatus: () => transport.invoke<CostStatus>("get_cost_status"),
  getCalendarMonth: (month) => transport.invoke<CalendarMonth>("get_calendar_month", { month }),
  createCalendarEvent: (input) => transport.invoke<CalendarEvent>("create_calendar_event", { input }),
  updateCalendarEvent: (eventId, input) => transport.invoke<CalendarEvent>("update_calendar_event", { eventId, input }),
  deleteCalendarEvent: (eventId, expectedVersion) => run(transport, "delete_calendar_event", { eventId, expectedVersion }),
  getStockWatchlist: () => transport.invoke<StockWatchlistItem[]>("get_stock_watchlist"),
  upsertStockWatchlistItem: (input) =>
    transport.invoke<StockWatchlistItem>("upsert_stock_watchlist_item", { input }),
  deleteStockWatchlistItem: (symbol) =>
    run(transport, "delete_stock_watchlist_item", { symbol }),
  getLatestStockScreen: () =>
    transport.invoke<LatestStockScreen>("get_latest_stock_screen"),
  listStockScreenResults: (input) =>
    transport.invoke<StockScreenResultPage>("list_stock_screen_results", { ...input }),
  createProject: (name, description = "") =>
    transport.invoke<string>("create_project", { name, description }),
  createTask: (input) => run(transport, "create_task", { input }),
  updateTask: (input) => run(transport, "update_task", { input }),
  planTask: (taskId, date) => run(transport, "plan_task", { taskId, date }),
  resolveDayEntry: (entryId, status) =>
    run(transport, "resolve_day_entry", { entryId, status }),
  startSession: (input) => run(transport, "start_session", { input }),
  finishSession: (input) => run(transport, "finish_session", { input }),
  createWorkLog: (input) => run(transport, "create_work_log", { input }),
  createNote: (input) => run(transport, "create_note", { input }),
  createChangeRequest: (input) => run(transport, "create_change_request", { input }),
  updateChangeRequest: (input) => run(transport, "update_change_request", { input }),
  approveChangeRequest: (changeRequestId, expectedRevision, expectedAttemptCount) =>
    run(transport, "approve_change_request", {
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
    }),
  returnChangeRequestToDraft: (changeRequestId, expectedRevision, expectedAttemptCount) =>
    run(transport, "return_change_request_to_draft", {
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
    }),
  cancelChangeRequest: (changeRequestId, expectedRevision, expectedAttemptCount, reason) =>
    run(transport, "cancel_change_request", {
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
      reason,
    }),
  abandonChangeRequest: (changeRequestId, expectedRevision, expectedAttemptCount, reason) =>
    run(transport, "abandon_change_request", {
      changeRequestId,
      expectedRevision,
      expectedAttemptCount,
      reason,
    }),
  promoteWorkLog: (workLogId, type, title) =>
    run(transport, "promote_work_log", { workLogId, noteType: type, title }),
  search: (query) => transport.invoke<SearchResult[]>("search", { query }),
  moveToTrash: (itemId, entityType) =>
    run(transport, "move_to_trash", { itemId, entityType }),
  restoreTrashItem: (itemId) => run(transport, "restore_trash_item", { itemId }),
  createBackup: () => run(transport, "create_backup"),
  restoreBackup: (backupId) => run(transport, "restore_backup", { backupId }),
  exportAll: () => transport.invoke<ExportResult>("export_all"),
  generateTaskReport: () => transport.invoke<TaskReportResult>("generate_task_report"),
  latestTaskReport: () => transport.invoke<TaskReportResult | null>("latest_task_report"),
  rateTaskReport: (reportId, helpful) =>
    transport.invoke<TaskReportResult>("rate_task_report", { reportId, helpful }),
  getExpenseSummary: (month) =>
    transport.invoke<ExpenseMonthSummary>("get_expense_summary", { month }),
  listExpenseSources: () =>
    transport.invoke<ExpenseSourceStatus[]>("list_expense_sources"),
  updateExpenseSource: (sourceId, input) =>
    invokeExpenseMutation<ExpenseSourceStatus>(
      "update_expense_source",
      { sourceId, input },
      sourceId,
    ),
  listExpenseTransactions: (input) =>
    transport.invoke<ExpenseTransactionPage>("list_expense_transactions", { input }),
  overrideExpenseTransaction: (eventId, input) =>
    invokeExpenseMutation<ExpenseTransaction>(
      "override_expense_transaction",
      { eventId, input },
      eventId,
    ),
  listExpenseReviews: (input) =>
    transport.invoke<ExpenseReviewPage>("list_expense_reviews", { input }),
  resolveExpenseReview: (reviewId, input) =>
    invokeExpenseMutation<void>("resolve_expense_review", { reviewId, input }, reviewId),
  classifyExpenseTransactions: (month) =>
    invokeExpenseMutation<ExpenseClassificationRunResult>(
      "classify_expense_transactions",
      { month },
      month,
    ),
  listRecurringExpenses: () =>
    transport.invoke<RecurringExpenseItem[]>("list_recurring_expenses"),
  listRecurringExpenseOccurrences: (month) =>
    transport.invoke<RecurringExpenseOccurrence[]>("list_recurring_expense_occurrences", { month }),
  createRecurringExpense: (input) =>
    invokeExpenseMutation<RecurringExpenseItem>("create_recurring_expense", { input }),
  updateRecurringExpense: (recurringExpenseId, input) =>
    invokeExpenseMutation<RecurringExpenseItem>(
      "update_recurring_expense",
      { recurringExpenseId, input },
      recurringExpenseId,
    ),
  deleteRecurringExpense: (recurringExpenseId, expectedVersion) =>
    invokeExpenseMutation<void>(
      "delete_recurring_expense",
      { recurringExpenseId, expectedVersion },
      recurringExpenseId,
    ),
  confirmRecurringExpensePaid: (occurrenceKey, amountMinor, paidDate, expectedVersion) =>
    invokeExpenseMutation<RecurringExpenseOccurrence>(
      "confirm_recurring_expense_paid",
      { occurrenceKey, amountMinor, paidDate, expectedVersion },
      occurrenceKey,
    ),
  matchRecurringExpenseOccurrence: (occurrenceKey, eventId, enableFutureAutoMatch, expectedVersion) =>
    invokeExpenseMutation<RecurringExpenseOccurrence>(
      "match_recurring_expense_occurrence",
      { occurrenceKey, eventId, enableFutureAutoMatch, expectedVersion },
      occurrenceKey,
    ),
  previewExpenseImport: (input) =>
    transport.invoke<ExpenseImportPreview>("preview_expense_import", { input }),
  commitExpenseImport: (sessionId) =>
    transport.invoke<ExpenseImportCommitResult>("commit_expense_import", { sessionId }),
  generateExpenseReport: (month) =>
    invokeExpenseMutation<ExpenseReportResult>("generate_expense_report", { month }, month),
  latestExpenseReport: (month) =>
    transport.invoke<ExpenseReportResult | null>("latest_expense_report", { month }),
  rateExpenseReport: (reportId, helpful) =>
    invokeExpenseMutation<ExpenseReportResult>(
      "rate_expense_report",
      { reportId, helpful },
      reportId,
    ),
  discardExpenseMutation,
  getMailAccounts: () => transport.invoke<MailAccount[]>("get_mail_accounts"),
  getMailSummary: (date) => transport.invoke<MailSummary>("get_mail_summary", { date }),
  listMailItems: (input) => transport.invoke<MailItemPage>("list_mail_items", { ...input }),
  getMailReports: (date) => transport.invoke<MailReport[]>("get_mail_reports", { date }),
  startGmailOAuth: () => invokeMailMutation<{ authorizationUrl: string; expiresInSeconds: number }>("start_gmail_oauth", {}),
  connectNaverMail: (input) => invokeMailMutation<MailAccount>("connect_naver_mail", { ...input }, input.email),
  disconnectMailAccount: (accountId, expectedVersion) =>
    invokeMailMutation<MailAccount>("disconnect_mail_account", { accountId, expectedVersion }, accountId),
  acknowledgeMailItem: (itemId, expectedVersion) =>
    invokeMailMutation<MailItem>("acknowledge_mail_item", { itemId, expectedVersion }, itemId),
  feedbackMailItem: (itemId, important, expectedVersion) =>
    invokeMailMutation<MailItem>("feedback_mail_item", { itemId, important, expectedVersion }, itemId),
  };
};

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export const createDefaultApi = (): TmApi => {
  const isTauri = typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
  const transport = isTauri ? new TauriTransport() : createMemoryTransport();
  return createApi(transport, {
    canImportExpenses: isTauri && /Windows/i.test(navigator.userAgent),
  });
};
