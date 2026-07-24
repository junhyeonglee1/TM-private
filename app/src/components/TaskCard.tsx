import { useEffect, useId, useState } from "react";

import type { DayEntryStatus, Task } from "../types";
import { Icon } from "./Icon";

interface TaskCardProps {
  task: Task;
  compact?: boolean;
  allowDefer?: boolean;
  dayEntryId?: string;
  onOpen: (task: Task) => void;
  onComplete?: (task: Task) => void | Promise<void>;
  onPlan?: (task: Task) => void;
  onResolve?: (entryId: string, status: Exclude<DayEntryStatus, "planned">) => void;
}

const statusLabels: Record<Task["status"], string> = {
  inbox: "Inbox",
  todo: "할 일",
  in_progress: "진행 중",
  blocked: "막힘",
  done: "완료",
  cancelled: "취소",
};

const priorityLabels: Record<Task["priority"], string> = {
  none: "",
  low: "낮음",
  medium: "보통",
  high: "높음",
};

export function TaskCard({
  task,
  compact = false,
  allowDefer = false,
  dayEntryId,
  onOpen,
  onComplete,
  onPlan,
  onResolve,
}: TaskCardProps) {
  const checked = task.checklist.filter((item) => item.checked).length;
  const completionTitleId = useId();
  const completionDescriptionId = useId();
  const [confirmingCompletion, setConfirmingCompletion] = useState(false);
  const [completing, setCompleting] = useState(false);
  const canComplete = task.status !== "done"
    && task.status !== "cancelled"
    && Boolean(onComplete || (dayEntryId && onResolve));

  useEffect(() => {
    if (!confirmingCompletion) return undefined;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !completing) setConfirmingCompletion(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [completing, confirmingCompletion]);

  const confirmCompletion = async () => {
    setCompleting(true);
    try {
      if (dayEntryId && onResolve) {
        onResolve(dayEntryId, "done");
      } else if (onComplete) {
        await onComplete(task);
      }
      setConfirmingCompletion(false);
    } finally {
      setCompleting(false);
    }
  };

  return (
    <article className={`task-card ${compact ? "task-card--compact" : ""}`}>
      <button className="task-card__body" onClick={() => onOpen(task)} type="button">
        <span className={`status-dot status-dot--${task.status}`} aria-hidden="true" />
        <span className="task-card__content">
          <span className="task-card__title">{task.title}</span>
          <span className="task-card__meta">
            {task.projectName && <span>{task.projectName}</span>}
            {task.dueDate && (
              <span className={task.priority === "high" ? "meta-danger" : ""}>
                <Icon name="calendar" size={13} /> {task.dueDate.slice(5).replace("-", ".")}
              </span>
            )}
            {task.priority !== "none" && (
              <span className={`priority priority--${task.priority}`}>
                <Icon name="flag" size={13} /> {priorityLabels[task.priority]}
              </span>
            )}
            {task.checklist.length > 0 && (
              <span>
                <Icon name="check" size={13} /> {checked}/{task.checklist.length}
              </span>
            )}
          </span>
          {!compact && task.tags.length > 0 && (
            <span className="tag-row">
              {task.tags.map((tag) => (
                <span className="tag" key={tag}>#{tag}</span>
              ))}
            </span>
          )}
        </span>
        <span className={`status-label status-label--${task.status}`}>{statusLabels[task.status]}</span>
        <Icon className="task-card__chevron" name="chevron" size={16} />
      </button>

      {(onPlan || canComplete || (dayEntryId && onResolve)) && (
        <div className="task-card__actions" aria-label={`${task.title} 작업`}>
          {onPlan && (
            <button className="text-button" onClick={() => onPlan(task)} type="button">
              오늘 계획
            </button>
          )}
          {dayEntryId && onResolve && (
            <>
              <button
                className="icon-button icon-button--success"
                aria-label={`${task.title} 완료`}
                onClick={() => setConfirmingCompletion(true)}
                title="완료"
                type="button"
              >
                <Icon name="check" size={16} />
              </button>
              {allowDefer && (
                <button
                  className="text-button"
                  onClick={() => onResolve(dayEntryId, "deferred")}
                  type="button"
                >
                  이월
                </button>
              )}
              <button
                className="icon-button"
                aria-label={`${task.title} 건너뛰기`}
                onClick={() => onResolve(dayEntryId, "skipped")}
                title="건너뛰기"
                type="button"
              >
                <Icon name="close" size={15} />
              </button>
            </>
          )}
          {!dayEntryId && canComplete && (
            <button
              className="icon-button icon-button--success"
              aria-label={`${task.title} 완료`}
              onClick={() => setConfirmingCompletion(true)}
              title="완료"
              type="button"
            >
              <Icon name="check" size={16} />
            </button>
          )}
        </div>
      )}

      {confirmingCompletion && (
        <div
          className="completion-confirm-backdrop"
          onMouseDown={(event) => {
            if (event.currentTarget === event.target && !completing) {
              setConfirmingCompletion(false);
            }
          }}
        >
          <section
            aria-describedby={completionDescriptionId}
            aria-labelledby={completionTitleId}
            aria-modal="true"
            className="completion-confirm-dialog"
            role="dialog"
          >
            <span className="completion-confirm-dialog__icon" aria-hidden="true">
              <Icon name="check" size={22} />
            </span>
            <div>
              <span className="eyebrow">상태 변경 확인</span>
              <h2 id={completionTitleId}>Task를 완료할까요?</h2>
              <p id={completionDescriptionId}>
                <strong>{task.title}</strong>을 완료 상태로 변경합니다.
              </p>
            </div>
            <footer>
              <button
                className="secondary-button"
                disabled={completing}
                onClick={() => setConfirmingCompletion(false)}
                type="button"
              >
                취소
              </button>
              <button
                autoFocus
                className="primary-button"
                disabled={completing}
                onClick={() => { void confirmCompletion(); }}
                type="button"
              >
                {completing ? "완료 처리 중…" : "완료 처리"}
              </button>
            </footer>
          </section>
        </div>
      )}
    </article>
  );
}
