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
