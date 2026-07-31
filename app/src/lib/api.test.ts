import { createApi, type CommandTransport } from "./api";
import { createMemoryTransport } from "./mock-transport";
import type { AppSnapshot } from "../types";

const archiveMockProject = (
  transport: ReturnType<typeof createMemoryTransport>,
  projectId: string,
) => {
  const snapshot = Reflect.get(transport, "snapshot") as AppSnapshot;
  const project = snapshot.projects.find((candidate) => candidate.id === projectId);
  if (!project) throw new Error("보관할 mock 프로젝트 fixture가 없습니다.");
  project.archived = true;
};

describe("Tauri invoke payload 계약", () => {
  it("mock snapshot과 null·undefined 프로젝트 Task를 시스템 기타 프로젝트로 정규화한다", async () => {
    const api = createApi(createMemoryTransport());
    const initial = await api.getSnapshot();
    const uncategorized = initial.projects.find(
      (project) => project.systemKey === "uncategorized",
    );
    expect(uncategorized).toMatchObject({ name: "기타", archived: false });

    await api.createTask({ title: "분류 전 Task", projectId: null, status: "todo" });
    let snapshot = await api.getSnapshot();
    expect(snapshot.tasks.find((task) => task.title === "분류 전 Task")).toMatchObject({
      projectId: uncategorized?.id,
      projectName: "기타",
    });
    await api.createTask({ title: "프로젝트 생략 Task", status: "todo" });
    snapshot = await api.getSnapshot();
    expect(snapshot.tasks.find((task) => task.title === "프로젝트 생략 Task")).toMatchObject({
      projectId: uncategorized?.id,
      projectName: "기타",
    });

    const existing = snapshot.tasks.find((task) => task.id === "task-schema");
    if (!existing) throw new Error("mock Task fixture가 없습니다.");
    await api.updateTask({
      taskId: existing.id,
      title: existing.title,
      description: existing.description,
      status: existing.status,
      priority: existing.priority,
      dueDate: existing.dueDate,
      projectId: null,
      tags: existing.tags,
      checklist: existing.checklist,
    });
    snapshot = await api.getSnapshot();
    expect(snapshot.tasks.find((task) => task.id === existing.id)).toMatchObject({
      projectId: uncategorized?.id,
      projectName: "기타",
    });
  });

  it("mock Task 생성은 명시된 프로젝트가 없거나 비활성이면 거부한다", async () => {
    const transport = createMemoryTransport();
    archiveMockProject(transport, "project-personal");
    const api = createApi(transport);
    const before = await api.getSnapshot();

    await expect(api.createTask({
      title: "존재하지 않는 프로젝트 Task",
      projectId: "project-missing",
      status: "todo",
    })).rejects.toThrow("프로젝트를 찾지 못했습니다");
    await expect(api.createTask({
      title: "보관 프로젝트 Task",
      projectId: "project-personal",
      status: "todo",
    })).rejects.toThrow("보관된 프로젝트에는 Task를 배정할 수 없습니다");

    expect((await api.getSnapshot()).tasks).toHaveLength(before.tasks.length);
  });

  it("mock Task 변경은 명시된 프로젝트가 없거나 비활성이면 원본을 보존한다", async () => {
    const transport = createMemoryTransport();
    archiveMockProject(transport, "project-personal");
    const api = createApi(transport);
    const existing = (await api.getSnapshot()).tasks.find((task) => task.id === "task-schema");
    if (!existing) throw new Error("mock Task fixture가 없습니다.");
    const update = (projectId: string) => api.updateTask({
      taskId: existing.id,
      title: "저장되면 안 되는 제목",
      description: existing.description,
      status: existing.status,
      priority: existing.priority,
      dueDate: existing.dueDate,
      projectId,
      tags: existing.tags,
      checklist: existing.checklist,
    });

    await expect(update("project-missing")).rejects.toThrow("프로젝트를 찾지 못했습니다");
    await expect(update("project-personal")).rejects.toThrow(
      "보관된 프로젝트에는 Task를 배정할 수 없습니다",
    );

    expect((await api.getSnapshot()).tasks.find((task) => task.id === existing.id)).toEqual(existing);
  });

  it("이미 보관된 프로젝트에 속한 mock Task는 소속을 유지한 편집을 허용한다", async () => {
    const transport = createMemoryTransport();
    archiveMockProject(transport, "project-personal");
    const api = createApi(transport);
    const existing = (await api.getSnapshot()).tasks.find(
      (task) => task.projectId === "project-personal",
    );
    if (!existing) throw new Error("보관 프로젝트의 기존 mock Task fixture가 없습니다.");

    await api.updateTask({
      taskId: existing.id,
      title: "보관 프로젝트 소속 유지 편집",
      description: existing.description,
      status: existing.status,
      priority: existing.priority,
      dueDate: existing.dueDate,
      projectId: existing.projectId,
      tags: existing.tags,
      checklist: existing.checklist,
    });

    expect((await api.getSnapshot()).tasks.find((task) => task.id === existing.id)).toMatchObject({
      title: "보관 프로젝트 소속 유지 편집",
      projectId: "project-personal",
    });
  });

  it("생성된 프로젝트 ID를 반환하고 Task 상태를 그대로 전달한다", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const transport: CommandTransport = {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        calls.push({ command, args });
        return (command === "create_project" ? "project-new" : undefined) as T;
      },
    };
    const api = createApi(transport);

    await api.getCostStatus();
    await expect(api.createProject("새 프로젝트")).resolves.toBe("project-new");
    await api.createTask({ title: "프로젝트 Task", projectId: "project-new", status: "todo" });

    expect(calls).toContainEqual({
      command: "create_task",
      args: { input: { title: "프로젝트 Task", projectId: "project-new", status: "todo" } },
    });
    expect(calls[0]).toEqual({ command: "get_cost_status", args: undefined });
  });

  it("WorkLog 승격 유형을 noteType camelCase 인자로 보낸다", async () => {
    let captured: { command: string; args?: Record<string, unknown> } | undefined;
    const transport: CommandTransport = {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        captured = { command, args };
        return undefined as T;
      },
    };

    await createApi(transport).promoteWorkLog("work-log-1", "decision", "백업 정책 결정");

    expect(captured).toEqual({
      command: "promote_work_log",
      args: {
        workLogId: "work-log-1",
        noteType: "decision",
        title: "백업 정책 결정",
      },
    });
    expect(captured?.args).not.toHaveProperty("type");
  });

  it("개선 요청 command에 ID와 revision을 camelCase로 전달한다", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const transport: CommandTransport = {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        calls.push({ command, args });
        return undefined as T;
      },
    };
    const api = createApi(transport);
    const content = {
      kind: "ui" as const,
      projectId: "project-tm",
      taskId: "task-ui",
      title: "오늘 화면 구분선 개선",
      description: "구역 경계가 흐립니다.",
      desiredOutcome: "구역을 빠르게 구분할 수 있습니다.",
      reproductionSteps: "오늘 화면을 엽니다.",
      priority: "medium" as const,
    };

    await api.createChangeRequest(content);
    await api.updateChangeRequest({
      ...content,
      changeRequestId: "change-1",
      expectedRevision: 3,
      expectedAttemptCount: 1,
    });
    await api.approveChangeRequest("change-1", 4, 1);
    await api.returnChangeRequestToDraft("change-1", 4, 1);
    await api.cancelChangeRequest("change-1", 4, 1, "더 이상 필요하지 않음");
    await api.abandonChangeRequest("change-2", 7, 2, "claim 실행이 유실됨");

    expect(calls).toEqual([
      { command: "create_change_request", args: { input: content } },
      {
        command: "update_change_request",
        args: {
          input: {
            ...content,
            changeRequestId: "change-1",
            expectedRevision: 3,
            expectedAttemptCount: 1,
          },
        },
      },
      {
        command: "approve_change_request",
        args: { changeRequestId: "change-1", expectedRevision: 4, expectedAttemptCount: 1 },
      },
      {
        command: "return_change_request_to_draft",
        args: { changeRequestId: "change-1", expectedRevision: 4, expectedAttemptCount: 1 },
      },
      {
        command: "cancel_change_request",
        args: {
          changeRequestId: "change-1",
          expectedRevision: 4,
          expectedAttemptCount: 1,
          reason: "더 이상 필요하지 않음",
        },
      },
      {
        command: "abandon_change_request",
        args: {
          changeRequestId: "change-2",
          expectedRevision: 7,
          expectedAttemptCount: 2,
          reason: "claim 실행이 유실됨",
        },
      },
    ]);
    for (const call of calls) {
      expect(call.args).not.toHaveProperty("change_request_id");
      expect(call.args).not.toHaveProperty("expected_revision");
      expect(call.args).not.toHaveProperty("expected_attempt_count");
    }
  });

  it("mock은 stale claim attempt의 유실 전환을 거부한다", async () => {
    const api = createApi(createMemoryTransport());
    const draft = (await api.getSnapshot()).changeRequests.find(
      (request) => request.id === "change-draft",
    );
    if (!draft) throw new Error("mock draft fixture가 없습니다.");

    await expect(api.updateChangeRequest({
      changeRequestId: draft.id,
      expectedRevision: draft.revision,
      expectedAttemptCount: 1,
      kind: draft.kind,
      projectId: draft.projectId,
      taskId: draft.taskId,
      title: draft.title,
      description: draft.description,
      desiredOutcome: draft.desiredOutcome,
      reproductionSteps: draft.reproductionSteps,
      priority: draft.priority,
    })).rejects.toThrow("다른 변경이 먼저 저장되었습니다");

    await expect(api.approveChangeRequest("change-draft", 1, 1)).rejects.toThrow(
      "다른 변경이 먼저 저장되었습니다",
    );
    await expect(api.returnChangeRequestToDraft("change-approved", 1, 1)).rejects.toThrow(
      "다른 변경이 먼저 저장되었습니다",
    );
    await expect(
      api.cancelChangeRequest("change-approved", 1, 1, "오래된 화면의 취소"),
    ).rejects.toThrow("다른 변경이 먼저 저장되었습니다");
    await api.returnChangeRequestToDraft("change-approved", 1, 0);
    expect(
      (await api.getSnapshot()).changeRequests.find((request) => request.id === "change-approved"),
    ).toMatchObject({ status: "draft", revision: 2, attemptCount: 0 });

    await expect(
      api.abandonChangeRequest("change-claimed", 1, 0, "오래된 실행에서 보낸 요청"),
    ).rejects.toThrow("claim 이후 요청이 변경되었습니다");
    expect(
      (await api.getSnapshot()).changeRequests.find((request) => request.id === "change-claimed"),
    ).toMatchObject({ status: "claimed", revision: 1, attemptCount: 1 });

    await api.abandonChangeRequest("change-claimed", 1, 1, "현재 claim 실행이 유실됨");
    expect(
      (await api.getSnapshot()).changeRequests.find((request) => request.id === "change-claimed"),
    ).toMatchObject({ status: "failed", failureReason: "현재 claim 실행이 유실됨" });
  });
});
