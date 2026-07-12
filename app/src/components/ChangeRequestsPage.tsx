import { useMemo, useState } from "react";

import type {
  ChangeRequest,
  ChangeRequestKind,
  ChangeRequestStatus,
  CreateChangeRequestInput,
  Priority,
  Project,
  Task,
  UpdateChangeRequestInput,
} from "../types";
import { EmptyState } from "./EmptyState";
import { Icon } from "./Icon";

type FilterId =
  | "active"
  | "draft"
  | "approved"
  | "claimed"
  | "failed"
  | "closed"
  | "all";

type EditorValue = CreateChangeRequestInput & {
  changeRequestId?: string;
  expectedRevision?: number;
  expectedAttemptCount?: number;
};

interface ChangeRequestsPageProps {
  requests: ChangeRequest[];
  projects: Project[];
  tasks: Task[];
  onCreate(input: CreateChangeRequestInput): Promise<boolean>;
  onUpdate(input: UpdateChangeRequestInput): Promise<boolean>;
  onApprove(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ): Promise<boolean>;
  onReturnToDraft(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
  ): Promise<boolean>;
  onCancel(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ): Promise<boolean>;
  onAbandon(
    changeRequestId: string,
    expectedRevision: number,
    expectedAttemptCount: number,
    reason: string,
  ): Promise<boolean>;
}

const kindLabels: Record<ChangeRequestKind, string> = {
  bug: "버그",
  ui: "UI",
  feature: "기능",
  other: "기타",
};

const statusLabels: Record<ChangeRequestStatus, string> = {
  draft: "초안",
  approved: "승인됨",
  claimed: "처리 중",
  completed: "완료",
  failed: "실패",
  cancelled: "취소됨",
};

const priorityLabels: Record<Priority, string> = {
  none: "없음",
  low: "낮음",
  medium: "보통",
  high: "높음",
};

const writingGuide: Record<
  ChangeRequestKind,
  { heading: string; description: string; prompts: Array<{ label: string; text: string }> }
> = {
  bug: {
    heading: "버그 요청은 재현 가능한 사실을 중심으로 적으세요",
    description: "실제 동작과 기대 동작을 분리하면 Codex가 변경 범위와 회귀 테스트를 정확히 잡을 수 있습니다.",
    prompts: [
      { label: "현재 상황", text: "문제가 발생한 화면, 실제 현상, 발생 빈도와 업무 영향을 적습니다." },
      { label: "확인·재현 절차", text: "1. 화면 열기  2. 실행한 동작  3. 문제가 나타나는 지점을 순서대로 적습니다." },
      { label: "원하는 결과", text: "정상 동작과 함께 ‘문제가 재발하지 않음’ 같은 완료 기준을 적습니다." },
    ],
  },
  ui: {
    heading: "UI 개선은 사용 흐름과 완료 모습을 함께 적으세요",
    description: "색이나 위치만 지정하기보다 어떤 정보를 더 빨리 찾고 어떤 행동이 쉬워져야 하는지 설명하는 편이 좋습니다.",
    prompts: [
      { label: "현재 상황", text: "현재 화면에서 헷갈리거나 시간이 걸리는 지점과 주로 사용하는 상황을 적습니다." },
      { label: "원하는 결과", text: "개선 후 화면 구성, 사용자 동작과 유지해야 할 기존 기능을 적습니다." },
      { label: "완료 기준", text: "데스크톱·모바일 배치, 키보드 접근성, 다크·라이트 테마 등 확인 조건을 적습니다." },
    ],
  },
  feature: {
    heading: "기능 요청은 사용자 시나리오와 범위를 명확히 적으세요",
    description: "구현 기술보다 누가 언제 무엇을 할 수 있어야 하는지와 저장되어야 할 결과를 설명하세요.",
    prompts: [
      { label: "현재 상황", text: "현재 작업 방식, 우회 방법과 불편하거나 반복되는 단계를 적습니다." },
      { label: "원하는 결과", text: "새 기능을 사용하는 순서와 최종적으로 저장·표시되어야 할 내용을 적습니다." },
      { label: "완료 기준·제외 범위", text: "성공 조건과 이번 요청에서 변경하지 않을 기능을 함께 적습니다." },
    ],
  },
  other: {
    heading: "기타 요청도 현재와 목표를 분리해 적으세요",
    description: "배경 설명 뒤에 원하는 결과와 확인 가능한 완료 기준을 짧게 덧붙이면 충분합니다.",
    prompts: [
      { label: "현재 상황", text: "요청이 필요한 배경과 현재 방식을 적습니다." },
      { label: "원하는 결과", text: "완료 후 달라져야 할 동작이나 문서를 적습니다." },
      { label: "완료 기준", text: "직접 확인할 수 있는 조건과 유지해야 할 기존 동작을 적습니다." },
    ],
  },
};

const blankEditor = (): EditorValue => ({
  kind: "feature",
  projectId: null,
  taskId: null,
  title: "",
  description: "",
  desiredOutcome: "",
  reproductionSteps: "",
  priority: "medium",
});

const editorFrom = (request: ChangeRequest): EditorValue => ({
  changeRequestId: request.id,
  expectedRevision: request.revision,
  expectedAttemptCount: request.attemptCount,
  kind: request.kind,
  projectId: request.projectId,
  taskId: request.taskId,
  title: request.title,
  description: request.description,
  desiredOutcome: request.desiredOutcome,
  reproductionSteps: request.reproductionSteps,
  priority: request.priority,
});

const formatTimestamp = (value: string | null): string => {
  if (!value) return "기록 없음";
  return new Intl.DateTimeFormat("ko-KR", {
    timeZone: "Asia/Seoul",
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
};

function ChangeRequestEditor({
  value,
  projects,
  tasks,
  saving,
  onCancel,
  onChange,
  onSubmit,
}: {
  value: EditorValue;
  projects: Project[];
  tasks: Task[];
  saving: boolean;
  onCancel(): void;
  onChange(next: EditorValue): void;
  onSubmit(): void;
}) {
  const linkedTasks = value.projectId
    ? tasks.filter((task) => task.projectId === value.projectId && !task.deletedAt)
    : tasks.filter((task) => !task.deletedAt);
  const isEditing = Boolean(value.changeRequestId);
  const guide = writingGuide[value.kind];

  return (
    <section className="change-editor panel" aria-labelledby="change-editor-title">
      <header className="change-editor__header">
        <span className="editor-card__icon"><Icon name="spark" size={18} /></span>
        <div>
          <span className="eyebrow">{isEditing ? "DRAFT EDIT" : "NEW DRAFT"}</span>
          <h2 id="change-editor-title">
            {isEditing ? "개선 요청 초안 편집" : "새 개선 요청 초안"}
          </h2>
          <p>저장만으로 처리되지 않습니다. 내용을 확인한 뒤 별도로 승인해야 합니다.</p>
        </div>
        <button
          aria-label="개선 요청 편집 닫기"
          className="icon-button"
          onClick={onCancel}
          type="button"
        >
          <Icon name="close" />
        </button>
      </header>

      <form
        className="change-editor__form"
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
      >
        <details className="change-writing-guide">
          <summary>
            <span><Icon name="note" size={16} /> 요청 작성 가이드</span>
            <small>가이드를 열어도 입력 내용은 바뀌지 않습니다.</small>
          </summary>
          <div className="change-writing-guide__content">
            <header>
              <span>{kindLabels[value.kind]} GUIDE</span>
              <h3>{guide.heading}</h3>
              <p>{guide.description}</p>
            </header>
            <div className="change-writing-guide__grid">
              {guide.prompts.map((prompt, index) => (
                <article key={prompt.label}>
                  <em>{index + 1}</em>
                  <div><strong>{prompt.label}</strong><p>{prompt.text}</p></div>
                </article>
              ))}
            </div>
            <aside>
              <strong>좋은 완료 기준 예시</strong>
              <p>“저장 직후 목록에 한 번만 표시되고, 기존 데이터와 다른 화면 동작은 그대로 유지된다.”</p>
            </aside>
          </div>
        </details>

        <div className="form-grid change-editor__basics">
          <label className="field">
            <span>유형 <em>필수</em></span>
            <select
              aria-label="개선 요청 유형"
              disabled={saving}
              onChange={(event) =>
                onChange({ ...value, kind: event.target.value as ChangeRequestKind })
              }
              value={value.kind}
            >
              {Object.entries(kindLabels).map(([kind, label]) => (
                <option key={kind} value={kind}>{label}</option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>우선순위</span>
            <select
              aria-label="개선 요청 우선순위"
              disabled={saving}
              onChange={(event) =>
                onChange({ ...value, priority: event.target.value as Priority })
              }
              value={value.priority}
            >
              {Object.entries(priorityLabels).map(([priority, label]) => (
                <option key={priority} value={priority}>{label}</option>
              ))}
            </select>
          </label>
          <label className="field field--full">
            <span>제목 <em>필수</em></span>
            <input
              aria-label="개선 요청 제목"
              disabled={saving}
              maxLength={160}
              onChange={(event) => onChange({ ...value, title: event.target.value })}
              placeholder="무엇을 개선하면 좋을지 한 문장으로 적으세요"
              required
              value={value.title}
            />
          </label>
          <label className="field">
            <span>연결 프로젝트 <small>선택</small></span>
            <select
              aria-label="개선 요청 프로젝트"
              disabled={saving}
              onChange={(event) =>
                onChange({
                  ...value,
                  projectId: event.target.value || null,
                  taskId: null,
                })
              }
              value={value.projectId ?? ""}
            >
              <option value="">연결하지 않음</option>
              {projects.filter((project) => !project.archived).map((project) => (
                <option key={project.id} value={project.id}>{project.name}</option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>연결 Task <small>선택</small></span>
            <select
              aria-label="개선 요청 Task"
              disabled={saving}
              onChange={(event) =>
                onChange({ ...value, taskId: event.target.value || null })
              }
              value={value.taskId ?? ""}
            >
              <option value="">연결하지 않음</option>
              {linkedTasks.map((task) => (
                <option key={task.id} value={task.id}>{task.title}</option>
              ))}
            </select>
          </label>
        </div>

        <label className="field">
          <span>현재 상황 <em>필수</em></span>
          <textarea
            aria-label="개선 요청 설명"
            disabled={saving}
            onChange={(event) => onChange({ ...value, description: event.target.value })}
            placeholder="현재 동작과 불편한 점을 구체적으로 적으세요."
            required
            rows={4}
            value={value.description}
          />
        </label>
        <label className="field">
          <span>원하는 결과 <em>필수</em></span>
          <textarea
            aria-label="개선 요청 원하는 결과"
            disabled={saving}
            onChange={(event) => onChange({ ...value, desiredOutcome: event.target.value })}
            placeholder="완료되었을 때 어떤 모습이어야 하는지 적으세요."
            required
            rows={3}
            value={value.desiredOutcome}
          />
        </label>
        <label className="field">
          <span>확인·재현 절차 <small>선택</small></span>
          <textarea
            aria-label="개선 요청 재현 절차"
            disabled={saving}
            onChange={(event) =>
              onChange({ ...value, reproductionSteps: event.target.value })
            }
            placeholder="버그라면 문제를 확인하는 순서를 적으세요."
            rows={3}
            value={value.reproductionSteps}
          />
        </label>

        <footer className="change-editor__actions">
          <p><Icon name="shield" size={14} /> 초안 저장 후 승인 전까지 Codex가 가져갈 수 없습니다.</p>
          <div>
            <button className="secondary-button" disabled={saving} onClick={onCancel} type="button">
              닫기
            </button>
            <button className="primary-button" disabled={saving} type="submit">
              <Icon name="check" size={15} />
              {saving ? "저장 중…" : "초안 저장"}
            </button>
          </div>
        </footer>
      </form>
    </section>
  );
}

export function ChangeRequestsPage({
  requests,
  projects,
  tasks,
  onCreate,
  onUpdate,
  onApprove,
  onReturnToDraft,
  onCancel,
  onAbandon,
}: ChangeRequestsPageProps) {
  const [filter, setFilter] = useState<FilterId>("active");
  const [editor, setEditor] = useState<EditorValue | null>(null);
  const [savingEditor, setSavingEditor] = useState(false);
  const [workingId, setWorkingId] = useState<string | null>(null);
  const [reasonAction, setReasonAction] = useState<{
    request: ChangeRequest;
    kind: "cancel" | "abandon";
  } | null>(null);
  const [reason, setReason] = useState("");

  const counts = useMemo(() => {
    const count = (status: ChangeRequestStatus) =>
      requests.filter((request) => request.status === status).length;
    return {
      draft: count("draft"),
      approved: count("approved"),
      claimed: count("claimed"),
      failed: count("failed"),
      closed: count("completed") + count("cancelled"),
      active: requests.filter((request) =>
        ["draft", "approved", "claimed", "failed"].includes(request.status),
      ).length,
      all: requests.length,
    };
  }, [requests]);

  const filtered = useMemo(() => {
    if (filter === "all") return requests;
    if (filter === "active") {
      return requests.filter((request) =>
        ["draft", "approved", "claimed", "failed"].includes(request.status),
      );
    }
    if (filter === "closed") {
      return requests.filter((request) =>
        request.status === "completed" || request.status === "cancelled",
      );
    }
    return requests.filter((request) => request.status === filter);
  }, [filter, requests]);

  const saveEditor = async () => {
    if (!editor) return;
    setSavingEditor(true);
    const success = editor.changeRequestId
      && editor.expectedRevision !== undefined
      && editor.expectedAttemptCount !== undefined
      ? await onUpdate({
          changeRequestId: editor.changeRequestId,
          expectedRevision: editor.expectedRevision,
          expectedAttemptCount: editor.expectedAttemptCount,
          kind: editor.kind,
          projectId: editor.projectId,
          taskId: editor.taskId,
          title: editor.title,
          description: editor.description,
          desiredOutcome: editor.desiredOutcome,
          reproductionSteps: editor.reproductionSteps,
          priority: editor.priority,
        })
      : await onCreate({
          kind: editor.kind,
          projectId: editor.projectId,
          taskId: editor.taskId,
          title: editor.title,
          description: editor.description,
          desiredOutcome: editor.desiredOutcome,
          reproductionSteps: editor.reproductionSteps,
          priority: editor.priority,
        });
    setSavingEditor(false);
    if (success) setEditor(null);
  };

  const run = async (request: ChangeRequest, action: () => Promise<boolean>) => {
    setWorkingId(request.id);
    const success = await action();
    setWorkingId(null);
    return success;
  };

  const editFailed = async (request: ChangeRequest) => {
    const success = await run(request, () =>
      onReturnToDraft(request.id, request.revision, request.attemptCount),
    );
    if (success) {
      setEditor({ ...editorFrom(request), expectedRevision: request.revision + 1 });
    }
  };

  const submitReason = async () => {
    if (!reasonAction || !reason.trim()) return;
    const { request, kind } = reasonAction;
    const success = await run(request, () =>
      kind === "cancel"
        ? onCancel(
            request.id,
            request.revision,
            request.attemptCount,
            reason.trim(),
          )
        : onAbandon(
            request.id,
            request.revision,
            request.attemptCount,
            reason.trim(),
          ),
    );
    if (success) {
      setReasonAction(null);
      setReason("");
    }
  };

  const filterItems: Array<{ id: FilterId; label: string; count: number }> = [
    { id: "active", label: "진행 항목", count: counts.active },
    { id: "draft", label: "초안", count: counts.draft },
    { id: "approved", label: "승인됨", count: counts.approved },
    { id: "claimed", label: "처리 중", count: counts.claimed },
    { id: "failed", label: "실패", count: counts.failed },
    { id: "closed", label: "완료 · 취소", count: counts.closed },
    { id: "all", label: "전체", count: counts.all },
  ];

  return (
    <div className="page-stack change-requests-page">
      <header className="page-header change-requests-header">
        <div>
          <span className="eyebrow">MANUAL APPROVAL QUEUE</span>
          <h1>개선 요청함</h1>
          <p>아이디어를 초안으로 다듬고, 검토가 끝난 요청만 명시적으로 승인합니다.</p>
        </div>
        <button
          className="primary-button primary-button--large"
          onClick={() => setEditor(blankEditor())}
          type="button"
        >
          <Icon name="plus" size={16} /> 새 요청
        </button>
      </header>

      <section className="change-overview" aria-label="개선 요청 상태 요약">
        <article>
          <span className="change-overview__icon change-overview__icon--draft"><Icon name="note" /></span>
          <div><small>승인 대기</small><strong>{counts.draft}</strong><p>검토 가능한 초안</p></div>
        </article>
        <article>
          <span className="change-overview__icon change-overview__icon--approved"><Icon name="check" /></span>
          <div><small>처리 대기</small><strong>{counts.approved}</strong><p>사용자가 승인함</p></div>
        </article>
        <article>
          <span className="change-overview__icon change-overview__icon--claimed"><Icon name="spark" /></span>
          <div><small>처리 중</small><strong>{counts.claimed}</strong><p>CLI가 원자적으로 claim</p></div>
        </article>
        <article>
          <span className="change-overview__icon change-overview__icon--failed"><Icon name="flag" /></span>
          <div><small>확인 필요</small><strong>{counts.failed}</strong><p>실패 후 재검토</p></div>
        </article>
      </section>

      {editor && (
        <ChangeRequestEditor
          onCancel={() => setEditor(null)}
          onChange={setEditor}
          onSubmit={() => void saveEditor()}
          projects={projects}
          saving={savingEditor}
          tasks={tasks}
          value={editor}
        />
      )}

      <section className="panel change-queue" aria-labelledby="change-queue-title">
        <header className="panel__header panel__header--compact change-queue__header">
          <div>
            <h2 id="change-queue-title">요청 목록</h2>
            <p>상태 전이는 기록되며 완료·취소된 요청은 읽기 전용입니다.</p>
          </div>
          <span className="count-pill">{filtered.length}건</span>
        </header>
        <div className="filter-tabs change-filter-tabs" role="group" aria-label="개선 요청 필터">
          {filterItems.map((item) => (
            <button
              aria-pressed={filter === item.id}
              key={item.id}
              onClick={() => setFilter(item.id)}
              type="button"
            >
              {item.label}<span>{item.count}</span>
            </button>
          ))}
        </div>

        {filtered.length === 0 ? (
          <EmptyState
            description="이 상태에 해당하는 개선 요청이 없습니다."
            icon="spark"
            title="표시할 요청이 없습니다"
          />
        ) : (
          <div className="change-request-list">
            {filtered.map((request) => {
              const project = projects.find((item) => item.id === request.projectId);
              const task = tasks.find((item) => item.id === request.taskId);
              const terminal =
                request.status === "completed" || request.status === "cancelled";
              const busy = workingId === request.id;
              const showingReason = reasonAction?.request.id === request.id;

              return (
                <article
                  className={"change-request-card change-request-card--" + request.status}
                  key={request.id}
                >
                  <header className="change-request-card__header">
                    <div className="change-request-card__badges">
                      <span className={"change-kind change-kind--" + request.kind}>
                        {kindLabels[request.kind]}
                      </span>
                      <span className={"change-status change-status--" + request.status}>
                        {statusLabels[request.status]}
                      </span>
                      {request.priority !== "none" && (
                        <span className={"change-priority change-priority--" + request.priority}>
                          {priorityLabels[request.priority]} 우선순위
                        </span>
                      )}
                    </div>
                    <div className="change-request-card__title">
                      <h3>{request.title}</h3>
                      <span>rev. {request.revision}</span>
                    </div>
                    <div className="change-request-card__links">
                      {project && <span><Icon name="projects" size={12} /> {project.name}</span>}
                      {task && <span><Icon name="check" size={12} /> {task.title}</span>}
                      {!project && !task && <span>연결 항목 없음</span>}
                    </div>
                  </header>

                  <div className="change-request-card__body">
                    <section>
                      <h4>현재 상황</h4>
                      <p>{request.description || "설명 없음"}</p>
                    </section>
                    <section>
                      <h4>원하는 결과</h4>
                      <p>{request.desiredOutcome || "기록 없음"}</p>
                    </section>
                    <section className="change-request-card__reproduction">
                      <h4>확인·재현 절차</h4>
                      <p>{request.reproductionSteps || "기록 없음"}</p>
                    </section>
                  </div>

                  {request.status === "approved" && (
                    <aside className="change-guidance">
                      <span><Icon name="spark" size={18} /></span>
                      <div>
                        <strong>승인된 요청이 처리 대기 중입니다</strong>
                        <p><code>TM 승인 요청 처리해줘. app/docs/prompts/change-request-processing.md를 따라줘.</code></p>
                        <small>승인만으로 자동 수정이나 예약 작업이 시작되지는 않습니다.</small>
                      </div>
                    </aside>
                  )}

                  {request.status === "claimed" && (
                    <aside className="change-result change-result--claimed">
                      <span><Icon name="clock" size={18} /></span>
                      <div>
                        <strong>{request.claimedBy ?? "Codex"}가 처리 중입니다</strong>
                        <p>claim {formatTimestamp(request.claimedAt)} · 시도 {request.attemptCount}회</p>
                      </div>
                    </aside>
                  )}

                  {request.status === "completed" && (
                    <aside className="change-result change-result--completed">
                      <span><Icon name="check" size={18} /></span>
                      <div>
                        <strong>처리 결과</strong>
                        <p>{request.resultSummary ?? "완료 결과가 기록되지 않았습니다."}</p>
                        <small>
                          {request.patchRef ? "패치 " + request.patchRef + " · " : ""}
                          {formatTimestamp(request.completedAt)}
                        </small>
                      </div>
                    </aside>
                  )}

                  {request.status === "failed" && (
                    <aside className="change-result change-result--failed">
                      <span><Icon name="flag" size={18} /></span>
                      <div>
                        <strong>처리 실패 이유</strong>
                        <p>{request.failureReason ?? "실패 이유가 기록되지 않았습니다."}</p>
                        <small>시도 {request.attemptCount}회 · {formatTimestamp(request.failedAt)}</small>
                      </div>
                    </aside>
                  )}

                  {request.status === "cancelled" && (
                    <aside className="change-result change-result--cancelled">
                      <span><Icon name="close" size={18} /></span>
                      <div>
                        <strong>취소 이유</strong>
                        <p>{request.cancellationReason ?? "취소 이유가 기록되지 않았습니다."}</p>
                        <small>{formatTimestamp(request.cancelledAt)}</small>
                      </div>
                    </aside>
                  )}

                  <footer className="change-request-card__footer">
                    <div className="change-request-card__meta">
                      <span>작성 {formatTimestamp(request.createdAt)}</span>
                      <span>수정 {formatTimestamp(request.updatedAt)}</span>
                      {request.approvedRevision !== null && (
                        <span>승인 revision {request.approvedRevision}</span>
                      )}
                    </div>
                    <div className="change-request-card__actions">
                      {request.status === "draft" && (
                        <>
                          <button
                            className="secondary-button secondary-button--small"
                            disabled={busy}
                            onClick={() => setEditor(editorFrom(request))}
                            type="button"
                          >
                            초안 편집
                          </button>
                          <button
                            className="primary-button"
                            disabled={busy}
                            onClick={() => void run(request, () =>
                              onApprove(request.id, request.revision, request.attemptCount),
                            )}
                            type="button"
                          >
                            <Icon name="check" size={14} /> 승인
                          </button>
                        </>
                      )}
                      {request.status === "approved" && (
                        <button
                          className="secondary-button secondary-button--small"
                          disabled={busy}
                          onClick={() => void run(request, () =>
                            onReturnToDraft(request.id, request.revision, request.attemptCount),
                          )}
                          type="button"
                        >
                          승인 취소
                        </button>
                      )}
                      {request.status === "failed" && (
                        <>
                          <button
                            className="secondary-button secondary-button--small"
                            disabled={busy}
                            onClick={() => void editFailed(request)}
                            type="button"
                          >
                            수정 후 재승인
                          </button>
                          <button
                            className="primary-button"
                            disabled={busy}
                            onClick={() => void run(request, () =>
                              onApprove(request.id, request.revision, request.attemptCount),
                            )}
                            type="button"
                          >
                            변경 없이 다시 승인
                          </button>
                        </>
                      )}
                      {request.status === "claimed" && (
                        <button
                          className="secondary-button secondary-button--small danger-text"
                          disabled={busy}
                          onClick={() => {
                            setReason("");
                            setReasonAction({ request, kind: "abandon" });
                          }}
                          type="button"
                        >
                          처리 유실 기록
                        </button>
                      )}
                      {!terminal && request.status !== "claimed" && (
                        <button
                          className="text-button danger-text"
                          disabled={busy}
                          onClick={() => {
                            setReason("");
                            setReasonAction({ request, kind: "cancel" });
                          }}
                          type="button"
                        >
                          요청 취소
                        </button>
                      )}
                      {terminal && <span className="read-only-pill"><Icon name="shield" size={12} /> 읽기 전용</span>}
                    </div>
                  </footer>

                  {showingReason && (
                    <form
                      className="change-reason-form"
                      onSubmit={(event) => {
                        event.preventDefault();
                        void submitReason();
                      }}
                    >
                      <div>
                        <strong>
                          {reasonAction.kind === "abandon"
                            ? "claim 유실을 실패로 기록"
                            : "개선 요청 취소"}
                        </strong>
                        <p>
                          {reasonAction.kind === "abandon"
                            ? "실행이 종료되었고 더 이상 완료·실패 명령을 받을 수 없을 때만 사용하세요."
                            : "취소된 요청은 다시 열 수 없으며 기록만 읽을 수 있습니다."}
                        </p>
                      </div>
                      <label className="field">
                        <span>{reasonAction.kind === "abandon" ? "유실 이유" : "취소 이유"} <em>필수</em></span>
                        <textarea
                          aria-label={reasonAction.kind === "abandon" ? "처리 유실 이유" : "개선 요청 취소 이유"}
                          autoFocus
                          disabled={busy}
                          onChange={(event) => setReason(event.target.value)}
                          required
                          rows={2}
                          value={reason}
                        />
                      </label>
                      <div>
                        <button
                          className="secondary-button secondary-button--small"
                          disabled={busy}
                          onClick={() => {
                            setReasonAction(null);
                            setReason("");
                          }}
                          type="button"
                        >
                          돌아가기
                        </button>
                        <button className="danger-button" disabled={busy || !reason.trim()} type="submit">
                          {reasonAction.kind === "abandon" ? "실패로 전환" : "요청 취소 확정"}
                        </button>
                      </div>
                    </form>
                  )}
                </article>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
