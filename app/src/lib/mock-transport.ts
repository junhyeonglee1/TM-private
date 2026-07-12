import type { CommandTransport } from "./api";
import type {
  AppSnapshot,
  BackupInfo,
  ChangeRequest,
  CreateChangeRequestInput,
  CreateNoteInput,
  CreateTaskInput,
  CreateWorkLogInput,
  DayEntryStatus,
  ExportResult,
  FinishSessionInput,
  Note,
  NoteType,
  Project,
  SearchResult,
  StartSessionInput,
  Task,
  TaskDayEntry,
  TrashItem,
  UpdateTaskInput,
  UpdateChangeRequestInput,
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
      openTaskCount: 4,
      completedTaskCount: 8,
      archived: false,
    },
    {
      id: "project-writing",
      name: "기록 정리",
      description: "업무 지식과 회고를 정리합니다",
      color: "#d77849",
      openTaskCount: 2,
      completedTaskCount: 3,
      archived: false,
    },
    {
      id: "project-personal",
      name: "개인 루틴",
      description: "꾸준히 유지할 생활 루틴",
      color: "#2f9b75",
      openTaskCount: 2,
      completedTaskCount: 12,
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

export class MemoryTransport implements CommandTransport {
  private snapshot = buildSample();

  async invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    const result = this.handle(command, args);
    return structuredClone(result) as T;
  }

  private handle(command: string, args: Record<string, unknown>): unknown {
    switch (command) {
      case "get_app_snapshot":
        return this.snapshot;
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
      default:
        throw new Error(`지원하지 않는 mock command: ${command}`);
    }
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
    const project = this.snapshot.projects.find((item) => item.id === input.projectId);
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
      projectId: project?.id ?? null,
      projectName: project?.name ?? null,
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
    const project = this.snapshot.projects.find((item) => item.id === input.projectId);
    Object.assign(task, input, {
      projectName: project?.name ?? null,
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
