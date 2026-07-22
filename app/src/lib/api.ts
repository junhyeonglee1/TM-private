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
  ExportResult,
  FinishSessionInput,
  NoteType,
  SearchResult,
  StartSessionInput,
  UpdateTaskInput,
  UpdateChangeRequestInput,
  UpdateCalendarEventInput,
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

export interface TaskReportResult {
  runId: string;
  reportDate: string;
  status: "succeeded" | "no_tasks";
  report: {
    headline: string;
    summary: string;
    priorities: TaskReportPriority[];
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
  getSnapshot(): Promise<AppSnapshot>;
  getCostStatus(): Promise<CostStatus>;
  getCalendarMonth(month: string): Promise<CalendarMonth>;
  createCalendarEvent(input: CreateCalendarEventInput): Promise<CalendarEvent>;
  updateCalendarEvent(eventId: string, input: UpdateCalendarEventInput): Promise<CalendarEvent>;
  deleteCalendarEvent(eventId: string, expectedVersion: number): Promise<void>;
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
}

class TauriTransport implements CommandTransport {
  private readonly mode = tauriInvoke<"local" | "cloud">("data_mode");

  async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    if (command === "get_cost_status") {
      return tauriInvoke<T>("get_cost_status");
    }
    if ((await this.mode) === "cloud") {
      if (["generate_task_report", "latest_task_report", "rate_task_report"].includes(command)) {
        return tauriInvoke<T>("invoke_cloud_assistant_feature", {
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

export const createApi = (transport: CommandTransport): TmApi => ({
  getSnapshot: () => transport.invoke<AppSnapshot>("get_app_snapshot"),
  getCostStatus: () => transport.invoke<CostStatus>("get_cost_status"),
  getCalendarMonth: (month) => transport.invoke<CalendarMonth>("get_calendar_month", { month }),
  createCalendarEvent: (input) => transport.invoke<CalendarEvent>("create_calendar_event", { input }),
  updateCalendarEvent: (eventId, input) => transport.invoke<CalendarEvent>("update_calendar_event", { eventId, input }),
  deleteCalendarEvent: (eventId, expectedVersion) => run(transport, "delete_calendar_event", { eventId, expectedVersion }),
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
});

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export const createDefaultApi = (): TmApi => {
  const transport =
    typeof window !== "undefined" && window.__TAURI_INTERNALS__
      ? new TauriTransport()
      : createMemoryTransport();
  return createApi(transport);
};
