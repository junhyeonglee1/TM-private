import { useEffect, useState } from "react";

import type { FinishSessionInput, Project, StartSessionInput, Task, WorkSession } from "../types";
import { EmptyState } from "./EmptyState";
import { Icon } from "./Icon";

interface SessionPageProps {
  activeSession: WorkSession | null;
  recentSessions: WorkSession[];
  projects: Project[];
  tasks: Task[];
  onStart: (input: StartSessionInput) => Promise<void>;
  onFinish: (input: FinishSessionInput) => Promise<void>;
  onOpenTask: (task: Task) => void;
}

const dateTime = (value: string): string =>
  new Intl.DateTimeFormat("ko-KR", {
    timeZone: "Asia/Seoul",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));

const duration = (startedAt: string, endedAt: string | null, currentTime = Date.now()): string => {
  const elapsed = Math.max(0, (endedAt ? new Date(endedAt).getTime() : currentTime) - new Date(startedAt).getTime());
  const minutes = Math.floor(elapsed / 60_000);
  const hours = Math.floor(minutes / 60);
  return hours > 0 ? `${hours}시간 ${minutes % 60}분` : `${minutes}분`;
};

export function SessionPage({
  activeSession,
  recentSessions,
  projects,
  tasks,
  onStart,
  onFinish,
  onOpenTask,
}: SessionPageProps) {
  const [projectId, setProjectId] = useState("");
  const [taskIds, setTaskIds] = useState<string[]>([]);
  const [goal, setGoal] = useState("");
  const [result, setResult] = useState("");
  const [blockers, setBlockers] = useState("");
  const [nextAction, setNextAction] = useState("");
  const [createNextTask, setCreateNextTask] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [clock, setClock] = useState(Date.now());

  useEffect(() => {
    if (!activeSession) return undefined;
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [activeSession]);

  const availableTasks = tasks.filter(
    (task) => !task.deletedAt && !["done", "cancelled"].includes(task.status) && (!projectId || task.projectId === projectId),
  );

  const start = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!goal.trim()) return;
    setSubmitting(true);
    try {
      await onStart({ projectId: projectId || null, taskIds, goal: goal.trim() });
      setGoal("");
      setTaskIds([]);
    } finally {
      setSubmitting(false);
    }
  };

  const finish = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!activeSession || !result.trim()) return;
    setSubmitting(true);
    try {
      await onFinish({
        sessionId: activeSession.id,
        result: result.trim(),
        blockers: blockers.trim(),
        nextAction: nextAction.trim(),
        createNextTask: createNextTask && Boolean(nextAction.trim()),
      });
      setResult("");
      setBlockers("");
      setNextAction("");
    } finally {
      setSubmitting(false);
    }
  };

  const toggleTask = (taskId: string) => {
    setTaskIds((selected) =>
      selected.includes(taskId) ? selected.filter((id) => id !== taskId) : [...selected, taskId],
    );
  };

  return (
    <div className="page-stack">
      <header className="page-header">
        <div>
          <span className="eyebrow">의도 있는 집중</span>
          <h1>작업 세션</h1>
          <p>시작할 때 목표를 정하고, 끝날 때 결과와 다음 행동을 남깁니다.</p>
        </div>
        {activeSession && <span className="live-pill"><i /> 세션 진행 중</span>}
      </header>

      {activeSession ? (
        <section className="active-session" aria-labelledby="active-session-heading">
          <div className="active-session__hero">
            <span className="active-session__pulse"><Icon name="timer" size={28} /></span>
            <div>
              <span>집중 시간</span>
              <strong>{duration(activeSession.startedAt, null, clock)}</strong>
            </div>
            <time dateTime={activeSession.startedAt}>{dateTime(activeSession.startedAt)} 시작</time>
          </div>
          <div className="active-session__context">
            <span className="section-kicker"><Icon name="projects" size={14} /> {activeSession.projectName ?? "프로젝트 없음"}</span>
            <h2 id="active-session-heading">{activeSession.goal}</h2>
            {activeSession.taskTitles.length > 0 && (
              <div className="linked-task-list">
                {activeSession.taskIds.map((taskId, index) => {
                  const task = tasks.find((item) => item.id === taskId);
                  return task ? (
                    <button key={taskId} onClick={() => onOpenTask(task)} type="button">
                      <Icon name="check" size={14} /> {activeSession.taskTitles[index]}
                    </button>
                  ) : null;
                })}
              </div>
            )}
          </div>
          <form className="session-finish-form" onSubmit={finish}>
            <div className="session-finish-form__heading">
              <span className="stop-icon"><Icon name="stop" /></span>
              <div><h3>세션 종료 기록</h3><p>종료·WorkLog·후속 Task는 하나의 트랜잭션으로 저장됩니다.</p></div>
            </div>
            <label className="field field--full">
              <span>결과 <em>필수</em></span>
              <textarea required rows={3} placeholder="무엇을 완료하거나 확인했나요?" value={result} onChange={(event) => setResult(event.target.value)} />
            </label>
            <label className="field field--full">
              <span>막힌 점</span>
              <textarea rows={2} placeholder="진행을 막은 조건이나 추가 확인이 필요한 부분" value={blockers} onChange={(event) => setBlockers(event.target.value)} />
            </label>
            <label className="field field--full">
              <span>다음 행동</span>
              <input placeholder="다음에 바로 시작할 수 있는 구체적인 행동" value={nextAction} onChange={(event) => setNextAction(event.target.value)} />
            </label>
            <label className="toggle-row">
              <input checked={createNextTask} disabled={!nextAction.trim()} onChange={(event) => setCreateNextTask(event.target.checked)} type="checkbox" />
              <span><strong>다음 행동을 새 Task로 만들기</strong><small>현재 프로젝트에 Inbox Task로 추가합니다.</small></span>
            </label>
            <button className="danger-button" disabled={submitting || !result.trim()} type="submit">
              <Icon name="stop" size={15} /> {submitting ? "종료 중…" : "세션 종료 및 기록"}
            </button>
          </form>
        </section>
      ) : (
        <section className="session-start-card" aria-labelledby="start-session-heading">
          <div className="session-start-card__intro">
            <span className="session-orbit"><Icon name="play" size={26} /></span>
            <span className="eyebrow">새 집중 블록</span>
            <h2 id="start-session-heading">무엇에 집중할까요?</h2>
            <p>명확한 목표와 관련 Task를 고르면 종료 기록이 자연스럽게 연결됩니다.</p>
          </div>
          <form className="session-start-form" onSubmit={start}>
            <label className="field field--full">
              <span>목표 <em>필수</em></span>
              <input autoFocus required placeholder="예: 백업 복원 시나리오 통합 테스트 완성" value={goal} onChange={(event) => setGoal(event.target.value)} />
            </label>
            <label className="field field--full">
              <span>프로젝트</span>
              <select value={projectId} onChange={(event) => { setProjectId(event.target.value); setTaskIds([]); }}>
                <option value="">프로젝트 없음 · 전체 Task 보기</option>
                {projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}
              </select>
            </label>
            <fieldset className="task-picker">
              <legend>관련 Task <small>여러 개 선택 가능</small></legend>
              <div>
                {availableTasks.map((task) => (
                  <label key={task.id}>
                    <input checked={taskIds.includes(task.id)} onChange={() => toggleTask(task.id)} type="checkbox" />
                    <span className={`status-dot status-dot--${task.status}`} />
                    <span><strong>{task.title}</strong><small>{task.projectName ?? "Inbox"}</small></span>
                  </label>
                ))}
              </div>
            </fieldset>
            <button className="primary-button primary-button--large" disabled={submitting || !goal.trim()} type="submit">
              <Icon name="play" size={16} /> {submitting ? "시작 중…" : "집중 세션 시작"}
            </button>
          </form>
        </section>
      )}

      <section className="panel" aria-labelledby="recent-session-heading">
        <div className="panel__header">
          <div><h2 id="recent-session-heading">최근 세션</h2><p>목표, 결과, 다음 행동을 이어서 확인합니다.</p></div>
        </div>
        <div className="session-history">
          {recentSessions.map((session) => (
            <article key={session.id}>
              <div className="session-history__time">
                <strong>{duration(session.startedAt, session.endedAt)}</strong>
                <time dateTime={session.startedAt}>{dateTime(session.startedAt)}</time>
              </div>
              <div className="session-history__body">
                <span className="section-kicker"><Icon name="projects" size={13} /> {session.projectName ?? "프로젝트 없음"}</span>
                <h3>{session.goal}</h3>
                <p>{session.result}</p>
                {session.blockers && <span className="blocker-note"><Icon name="flag" size={14} /> {session.blockers}</span>}
                {session.nextAction && <span className="next-action"><Icon name="arrow" size={14} /> 다음: {session.nextAction}</span>}
              </div>
            </article>
          ))}
          {recentSessions.length === 0 && <EmptyState icon="timer" title="완료한 세션이 없습니다" description="첫 집중 세션을 시작해 보세요." />}
        </div>
      </section>
    </div>
  );
}
