import { useEffect, useId, useState } from "react";

import type { ChecklistItem, Project, Task, UpdateTaskInput } from "../types";
import { Icon } from "./Icon";

interface TaskDetailProps {
  task: Task;
  projects: Project[];
  saving: boolean;
  onClose: () => void;
  onSave: (input: UpdateTaskInput) => Promise<void>;
  onTrash: () => Promise<void>;
}

const statusOptions: Array<{ value: Task["status"]; label: string }> = [
  { value: "inbox", label: "Inbox" },
  { value: "todo", label: "할 일" },
  { value: "in_progress", label: "진행 중" },
  { value: "blocked", label: "막힘" },
  { value: "done", label: "완료" },
  { value: "cancelled", label: "취소" },
];

const priorityOptions: Array<{ value: Task["priority"]; label: string }> = [
  { value: "none", label: "없음" },
  { value: "low", label: "낮음" },
  { value: "medium", label: "보통" },
  { value: "high", label: "높음" },
];

const formatDateTime = (value: string): string =>
  new Intl.DateTimeFormat("ko-KR", {
    timeZone: "Asia/Seoul",
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));

export function TaskDetail({ task, projects, saving, onClose, onSave, onTrash }: TaskDetailProps) {
  const titleId = useId();
  const [title, setTitle] = useState(task.title);
  const [description, setDescription] = useState(task.description);
  const [status, setStatus] = useState(task.status);
  const [priority, setPriority] = useState(task.priority);
  const [dueDate, setDueDate] = useState(task.dueDate ?? "");
  const [projectId, setProjectId] = useState(task.projectId ?? "");
  const [tags, setTags] = useState(task.tags.join(", "));
  const [checklist, setChecklist] = useState(task.checklist);
  const [newChecklistItem, setNewChecklistItem] = useState("");
  const [activityTab, setActivityTab] = useState<"events" | "logs">("events");

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  const addChecklistItem = () => {
    const label = newChecklistItem.trim();
    if (!label) return;
    setChecklist((items) => [
      ...items,
      { id: `draft-${Date.now()}-${items.length}`, label, checked: false },
    ]);
    setNewChecklistItem("");
  };

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    await onSave({
      taskId: task.id,
      title: title.trim(),
      description: description.trim(),
      status,
      priority,
      dueDate: dueDate || null,
      projectId: projectId || null,
      tags: tags
        .split(",")
        .map((tag) => tag.trim().replace(/^#/, ""))
        .filter(Boolean),
      checklist: checklist.map((item) => ({
        ...item,
        id: item.id.startsWith("draft-") ? "" : item.id,
      })),
    });
  };

  const toggleChecklist = (target: ChecklistItem) => {
    setChecklist((items) =>
      items.map((item) =>
        item.id === target.id ? { ...item, checked: !item.checked } : item,
      ),
    );
  };

  return (
    <div className="drawer-backdrop" onMouseDown={(event) => {
      if (event.currentTarget === event.target) onClose();
    }}>
      <aside aria-labelledby={titleId} aria-modal="true" className="task-drawer" role="dialog">
        <header className="task-drawer__header">
          <div>
            <span className="eyebrow">Task 상세</span>
            <h2 id={titleId}>{task.title}</h2>
          </div>
          <button aria-label="상세 닫기" className="icon-button" onClick={onClose} type="button">
            <Icon name="close" />
          </button>
        </header>

        <form className="task-detail-form" onSubmit={submit}>
          <label className="field field--full">
            <span>제목</span>
            <input autoFocus required value={title} onChange={(event) => setTitle(event.target.value)} />
          </label>

          <label className="field field--full">
            <span>설명</span>
            <textarea
              placeholder="완료 조건, 맥락, 참고 사항을 적어 두세요."
              rows={4}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          </label>

          <div className="form-grid">
            <label className="field">
              <span>상태</span>
              <select value={status} onChange={(event) => setStatus(event.target.value as Task["status"])}>
                {statusOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
              </select>
            </label>
            <label className="field">
              <span>우선순위</span>
              <select value={priority} onChange={(event) => setPriority(event.target.value as Task["priority"])}>
                {priorityOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
              </select>
            </label>
            <label className="field">
              <span>마감일</span>
              <input type="date" value={dueDate} onChange={(event) => setDueDate(event.target.value)} />
            </label>
            <label className="field">
              <span>프로젝트</span>
              <select value={projectId} onChange={(event) => setProjectId(event.target.value)}>
                <option value="">프로젝트 없음</option>
                {projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}
              </select>
            </label>
          </div>

          <label className="field field--full">
            <span>태그 <small>쉼표로 구분</small></span>
            <div className="input-with-icon">
              <Icon name="tag" size={16} />
              <input placeholder="설계, SQLite" value={tags} onChange={(event) => setTags(event.target.value)} />
            </div>
          </label>

          <section className="detail-section" aria-labelledby="checklist-heading">
            <div className="detail-section__heading">
              <h3 id="checklist-heading">체크리스트</h3>
              <span>{checklist.filter((item) => item.checked).length}/{checklist.length}</span>
            </div>
            <div className="checklist">
              {checklist.map((item) => (
                <div className="checklist__item" key={item.id}>
                  <label>
                    <input checked={item.checked} onChange={() => toggleChecklist(item)} type="checkbox" />
                    <span>{item.label}</span>
                  </label>
                  <button
                    aria-label={`${item.label} 삭제`}
                    className="icon-button icon-button--small"
                    onClick={() => setChecklist((items) => items.filter((candidate) => candidate.id !== item.id))}
                    type="button"
                  ><Icon name="close" size={14} /></button>
                </div>
              ))}
              {checklist.length === 0 && <p className="muted-copy">아직 체크리스트가 없습니다.</p>}
            </div>
            <div className="inline-add">
              <input
                aria-label="새 체크리스트 항목"
                placeholder="항목 추가"
                value={newChecklistItem}
                onChange={(event) => setNewChecklistItem(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    addChecklistItem();
                  }
                }}
              />
              <button className="secondary-button" onClick={addChecklistItem} type="button">
                <Icon name="plus" size={15} /> 추가
              </button>
            </div>
          </section>

          <section className="detail-section" aria-labelledby="activity-heading">
            <div className="detail-section__heading detail-section__heading--tabs">
              <h3 id="activity-heading">활동</h3>
              <div className="segmented-control" aria-label="활동 유형">
                <button
                  aria-pressed={activityTab === "events"}
                  onClick={() => setActivityTab("events")}
                  type="button"
                >이벤트 {task.events.length}</button>
                <button
                  aria-pressed={activityTab === "logs"}
                  onClick={() => setActivityTab("logs")}
                  type="button"
                >WorkLog {task.workLogs.length}</button>
              </div>
            </div>

            {activityTab === "events" ? (
              <ol className="timeline">
                {task.events.map((event) => (
                  <li key={event.id}>
                    <span className="timeline__dot" />
                    <div>
                      <strong>{event.summary}</strong>
                      <time dateTime={event.createdAt}>{formatDateTime(event.createdAt)}</time>
                      {(event.before || event.after) && (
                        <details>
                          <summary>before / after 보기</summary>
                          <div className="event-diff">
                            <code>{JSON.stringify(event.before, null, 2)}</code>
                            <Icon name="chevron" size={14} />
                            <code>{JSON.stringify(event.after, null, 2)}</code>
                          </div>
                        </details>
                      )}
                    </div>
                  </li>
                ))}
              </ol>
            ) : (
              <div className="activity-list">
                {task.workLogs.map((log) => (
                  <article className="activity-card" key={log.id}>
                    <strong>{log.title}</strong>
                    <p>{log.content}</p>
                    <time dateTime={log.createdAt}>{formatDateTime(log.createdAt)}</time>
                  </article>
                ))}
                {task.workLogs.length === 0 && <p className="muted-copy">연결된 WorkLog가 없습니다.</p>}
              </div>
            )}
          </section>

          <footer className="task-drawer__footer">
            <span className="save-hint"><Icon name="shield" size={15} /> 변경은 수정 불가능한 이벤트로 기록됩니다.</span>
            <button className="danger-button" disabled={saving} onClick={() => void onTrash()} type="button">
              <Icon name="trash" size={15} /> 휴지통으로 이동
            </button>
            <button className="primary-button" disabled={saving || !title.trim()} type="submit">
              {saving ? "저장 중…" : "변경 저장"}
            </button>
          </footer>
        </form>
      </aside>
    </div>
  );
}
