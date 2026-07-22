import { useEffect, useState } from "react";

import type {
  CreateTaskInput,
  DayEntryStatus,
  HistoryDay,
  Project,
  Task,
  TodaySnapshot,
} from "../types";
import type { TaskReportResult } from "../lib/api";
import { EmptyState } from "./EmptyState";
import { Icon } from "./Icon";
import { TaskCard } from "./TaskCard";

export function InboxPage() {
  return (
    <div className="page-stack inbox-hold-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">PLANNING HOLD</span>
          <h1>Inbox</h1>
          <p>역할을 다시 정하기 전까지 기능을 비워 둡니다.</p>
        </div>
      </header>
      <section className="panel inbox-hold" aria-labelledby="inbox-hold-heading">
        <span className="inbox-hold__icon"><Icon name="inbox" size={24} /></span>
        <div>
          <span className="section-kicker">간결한 Task 흐름</span>
          <h2 id="inbox-hold-heading">Inbox는 잠시 비워 두었습니다</h2>
          <p>즉흥 수집과 분류 기능은 제거했습니다. 새 Task는 프로젝트 화면에서 소속을 먼저 정한 뒤 추가하세요.</p>
          <small>Inbox의 새로운 역할은 사용 흐름을 더 살펴본 뒤 다시 기획합니다.</small>
        </div>
      </section>
    </div>
  );
}

interface TodayPageProps {
  today: string;
  view: TodaySnapshot;
  tasks: Task[];
  report: TaskReportResult | null;
  reportLoading: boolean;
  onOpen: (task: Task) => void;
  onResolve: (entryId: string, status: Exclude<DayEntryStatus, "planned">) => void;
  onGenerateReport: () => Promise<void>;
  onRateReport: (helpful: boolean) => Promise<void>;
}

const friendlyDate = (date: string): string =>
  new Intl.DateTimeFormat("ko-KR", {
    month: "long",
    day: "numeric",
    weekday: "long",
    timeZone: "Asia/Seoul",
  }).format(new Date(`${date}T12:00:00+09:00`));

export function TodayPage({
  today,
  view,
  tasks,
  report,
  reportLoading,
  onOpen,
  onResolve,
  onGenerateReport,
  onRateReport,
}: TodayPageProps) {
  const total = view.planned.length + view.inProgress.length + view.completed.length;
  const progress = total ? Math.round((view.completed.length / total) * 100) : 0;
  return (
    <div className="page-stack">
      <header className="page-header page-header--today">
        <div>
          <span className="eyebrow">{friendlyDate(today)}</span>
          <h1>오늘</h1>
          <p>자동 이월 없이, 오늘 집중할 일을 직접 선택합니다.</p>
        </div>
        <div className="today-progress" aria-label={`오늘 완료율 ${progress}%`}>
          <div className="today-progress__ring" style={{ "--progress": `${progress * 3.6}deg` } as React.CSSProperties}>
            <span>{progress}%</span>
          </div>
          <div><strong>{view.completed.length}</strong><span> / {total} 완료</span></div>
        </div>
      </header>

      <section className="panel task-report" aria-labelledby="task-report-heading" aria-busy={reportLoading}>
        <div className="panel__header task-report__header">
          <div>
            <span className="section-kicker section-kicker--accent"><Icon name="spark" size={14} /> AI 비서 · 읽기 전용</span>
            <h2 id="task-report-heading">오늘의 Task 우선순위</h2>
            <p>제목·상태·우선순위·마감일만 선별해 최대 3개의 다음 행동을 제안합니다.</p>
          </div>
          <button
            className="primary-button"
            disabled={reportLoading}
            onClick={() => { void onGenerateReport(); }}
            type="button"
          >
            {reportLoading ? "분석 중…" : report?.reportDate === today ? "다시 분석" : "AI 리포트 만들기"}
          </button>
        </div>

        {!report && !reportLoading && (
          <div className="task-report__empty">
            <strong>아직 생성한 리포트가 없습니다.</strong>
            <span>수동 호출만 사용하며 하루 최대 4회, 1회 비용 상한은 $0.05입니다.</span>
          </div>
        )}

        {report && (
          <div className="task-report__body">
            {report.reportDate !== today && <span className="task-report__stale">이전 리포트 · {report.reportDate}</span>}
            <div className="task-report__summary">
              <h3>{report.report.headline}</h3>
              <p>{report.report.summary}</p>
            </div>
            {report.report.priorities.length > 0 && (
              <ol className="task-report__priorities">
                {report.report.priorities.map((priority) => {
                  const task = tasks.find((item) => item.id === priority.taskId);
                  return (
                    <li key={priority.taskId}>
                      <span className="task-report__rank">{priority.rank}</span>
                      <div>
                        <button disabled={!task} onClick={() => task && onOpen(task)} type="button">
                          {task?.title ?? "현재 목록에서 찾을 수 없는 Task"}
                        </button>
                        <p>{priority.reason}</p>
                        <strong>다음 행동 · {priority.nextAction}</strong>
                        {priority.alert && <small>{priority.alert}</small>}
                      </div>
                    </li>
                  );
                })}
              </ol>
            )}
            {report.report.alerts.length > 0 && (
              <ul className="task-report__alerts">
                {report.report.alerts.map((alert) => <li key={alert}>{alert}</li>)}
              </ul>
            )}
            <footer className="task-report__footer">
              <span>
                후보 {report.candidateCount}개 · {report.usage ? `${report.usage.totalTokens.toLocaleString("ko-KR")} tokens` : "AI 호출 없음"}
                {` · $${(report.estimatedCostMicrousd / 1_000_000).toFixed(4)}`}
                {report.latencyMs !== null ? ` · ${(report.latencyMs / 1000).toFixed(1)}초` : ""}
              </span>
              <div aria-label="리포트 품질 평가">
                {report.helpful === null ? (
                  <>
                    <button onClick={() => { void onRateReport(true); }} type="button">도움 됨</button>
                    <button onClick={() => { void onRateReport(false); }} type="button">도움 안 됨</button>
                  </>
                ) : <strong>{report.helpful ? "도움 됨으로 평가함" : "도움 안 됨으로 평가함"}</strong>}
              </div>
            </footer>
          </div>
        )}
      </section>

      {view.yesterdayIncomplete.length > 0 && (
        <section className="panel panel--attention" aria-labelledby="yesterday-heading">
          <div className="panel__header">
            <div>
              <span className="section-kicker section-kicker--attention"><Icon name="history" size={14} /> 선택 필요</span>
              <h2 id="yesterday-heading">어제 미완료</h2>
              <p>이월하면 어제 기록은 확정되고 오늘 계획이 새로 만들어집니다.</p>
            </div>
            <span className="count-pill count-pill--attention">{view.yesterdayIncomplete.length}</span>
          </div>
          <div className="task-list">
            {view.yesterdayIncomplete.map((entry) => (
              <TaskCard allowDefer dayEntryId={entry.id} key={entry.id} onOpen={onOpen} onResolve={onResolve} task={entry.task} />
            ))}
          </div>
        </section>
      )}

      <div className="today-grid">
        <section className="panel" aria-labelledby="planned-heading">
          <div className="panel__header panel__header--compact">
            <div>
              <span className="section-kicker"><Icon name="today" size={14} /> 계획</span>
              <h2 id="planned-heading">오늘 계획</h2>
            </div>
            <span className="count-pill">{view.planned.length}</span>
          </div>
          <div className="task-list">
            {view.planned.map((entry) => (
              <TaskCard compact dayEntryId={entry.id} key={entry.id} onOpen={onOpen} onResolve={onResolve} task={entry.task} />
            ))}
            {view.planned.length === 0 && <EmptyState icon="today" title="오늘 계획이 없습니다" description="프로젝트에서 오늘 할 일을 선택하세요." />}
          </div>
        </section>

        <section className="panel panel--accent" aria-labelledby="progress-heading">
          <div className="panel__header panel__header--compact">
            <div>
              <span className="section-kicker section-kicker--accent"><Icon name="play" size={14} /> 집중</span>
              <h2 id="progress-heading">진행 중</h2>
            </div>
            <span className="count-pill count-pill--accent">{view.inProgress.length}</span>
          </div>
          <div className="task-list">
            {view.inProgress.map((task) => <TaskCard compact key={task.id} onOpen={onOpen} task={task} />)}
            {view.inProgress.length === 0 && <EmptyState icon="play" title="진행 중인 Task가 없습니다" description="하나를 골라 집중 세션을 시작해 보세요." />}
          </div>
        </section>
      </div>

      <section className="panel panel--completed" aria-labelledby="completed-heading">
        <div className="panel__header panel__header--compact">
          <div>
            <span className="section-kicker section-kicker--success"><Icon name="check" size={14} /> 성과</span>
            <h2 id="completed-heading">오늘 완료</h2>
          </div>
          <span className="count-pill count-pill--success">{view.completed.length}</span>
        </div>
        <div className="task-list task-list--completed">
          {view.completed.map((entry) => <TaskCard compact key={entry.id} onOpen={onOpen} task={entry.task} />)}
          {view.completed.length === 0 && <EmptyState icon="check" title="아직 완료 기록이 없습니다" description="작은 일부터 하나씩 마쳐 보세요." />}
        </div>
      </section>
    </div>
  );
}

interface ProjectsPageProps {
  projects: Project[];
  tasks: Task[];
  onOpen: (task: Task) => void;
  onPlan: (task: Task) => void;
  onCreateProject: (name: string) => Promise<string>;
  onCreateTask: (input: CreateTaskInput) => Promise<void>;
}

interface ProjectQuickAddProps {
  project: Project;
  onCreate: (input: CreateTaskInput) => Promise<void>;
}

function ProjectQuickAdd({ project, onCreate }: ProjectQuickAddProps) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [priority, setPriority] = useState<Task["priority"]>("none");
  const [dueDate, setDueDate] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!title.trim()) return;
    setSubmitting(true);
    try {
      await onCreate({
        title: title.trim(),
        description: description.trim(),
        projectId: project.id,
        status: "todo",
        priority,
        dueDate: dueDate || null,
      });
      setTitle("");
      setDescription("");
      setPriority("none");
      setDueDate("");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="project-quick-add" onSubmit={submit}>
      <div className="project-quick-add__context" title={`소속 프로젝트: ${project.name}`}>
        <Icon name="projects" size={14} />
        <span><small>소속</small><strong>{project.name}</strong></span>
      </div>
      <div className="project-quick-add__title">
        <span><Icon name="plus" size={17} /></span>
        <input
          aria-label={`${project.name} Task 제목`}
          autoComplete="off"
          placeholder={`${project.name}에 할 일 추가`}
          value={title}
          onChange={(event) => setTitle(event.target.value)}
        />
      </div>
      <input
        aria-label={`${project.name} Task 설명`}
        className="project-quick-add__description"
        autoComplete="off"
        placeholder="설명 (선택)"
        value={description}
        onChange={(event) => setDescription(event.target.value)}
      />
      <select aria-label={`${project.name} Task 우선순위`} value={priority} onChange={(event) => setPriority(event.target.value as Task["priority"])}>
        <option value="none">우선순위 없음</option>
        <option value="low">낮음</option>
        <option value="medium">보통</option>
        <option value="high">높음</option>
      </select>
      <label className="project-quick-add__date">
        <span>마감일 · 선택</span>
        <input aria-label={`${project.name} Task 마감일`} type="date" value={dueDate} onChange={(event) => setDueDate(event.target.value)} />
      </label>
      <button className="primary-button" disabled={submitting || !title.trim()} type="submit">
        {submitting ? "추가 중…" : "Task 추가"}
      </button>
    </form>
  );
}

const openTaskGroups: Array<{ status: Task["status"]; label: string; description: string }> = [
  { status: "in_progress", label: "진행 중", description: "지금 집중하고 있는 Task" },
  { status: "todo", label: "할 일", description: "프로젝트에서 실행할 준비가 된 Task" },
  { status: "blocked", label: "막힘", description: "해결할 장애물이 있는 Task" },
];

const closedTaskGroups: Array<{ status: Task["status"]; label: string; description: string }> = [
  { status: "done", label: "완료", description: "완료한 Task" },
  { status: "cancelled", label: "취소", description: "더 진행하지 않기로 한 Task" },
];

export function ProjectsPage({
  projects,
  tasks,
  onOpen,
  onPlan,
  onCreateProject,
  onCreateTask,
}: ProjectsPageProps) {
  const activeProjects = projects.filter((project) => !project.archived);
  const [selectedProjectId, setSelectedProjectId] = useState(activeProjects[0]?.id ?? "");
  const [projectName, setProjectName] = useState("");
  const [creatingProject, setCreatingProject] = useState(false);
  const [taskFilter, setTaskFilter] = useState<"open" | "closed">("open");
  const selected = activeProjects.find((project) => project.id === selectedProjectId);
  const projectTasks = tasks.filter((task) => task.projectId === selectedProjectId && !task.deletedAt);
  const groups = taskFilter === "open" ? openTaskGroups : closedTaskGroups;
  const visibleTaskCount = projectTasks.filter((task) => groups.some((group) => group.status === task.status)).length;

  useEffect(() => {
    if (!activeProjects.some((project) => project.id === selectedProjectId)) {
      setSelectedProjectId(activeProjects[0]?.id ?? "");
    }
  }, [activeProjects, selectedProjectId]);

  const createProject = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = projectName.trim();
    if (!name) return;
    setCreatingProject(true);
    try {
      const projectId = await onCreateProject(name);
      if (projectId) {
        setProjectName("");
        setSelectedProjectId(projectId);
        setTaskFilter("open");
      }
    } finally {
      setCreatingProject(false);
    }
  };

  return (
    <div className="page-stack">
      <header className="page-header">
        <div>
          <span className="eyebrow">영역별 보기</span>
          <h1>프로젝트</h1>
          <p>관련 Task와 진행 상태를 한곳에서 확인합니다.</p>
        </div>
        <form className="inline-add" onSubmit={(event) => void createProject(event)}>
          <input aria-label="새 프로젝트 이름" onChange={(event) => setProjectName(event.target.value)} placeholder="새 프로젝트" value={projectName} />
          <button className="primary-button" disabled={creatingProject || !projectName.trim()} type="submit"><Icon name="plus" size={15} /> {creatingProject ? "추가 중…" : "추가"}</button>
        </form>
      </header>
      <div className="project-workspace">
        <aside className="panel project-master" aria-label="프로젝트 목록">
          <div className="project-master__header">
            <h2>프로젝트 목록</h2>
            <span className="count-pill">{activeProjects.length}</span>
          </div>
          <div className="project-list" role="list">
            {activeProjects.map((project) => {
              const total = project.openTaskCount + project.completedTaskCount;
              const percent = total ? Math.round((project.completedTaskCount / total) * 100) : 0;
              return (
                <button
                  aria-pressed={selectedProjectId === project.id}
                  className="project-list-item"
                  key={project.id}
                  onClick={() => { setSelectedProjectId(project.id); setTaskFilter("open"); }}
                  role="listitem"
                  style={{ "--project-color": project.color } as React.CSSProperties}
                  type="button"
                >
                  <span className="project-list-item__icon"><Icon name="projects" size={17} /></span>
                  <span className="project-list-item__content">
                    <strong>{project.name}</strong>
                    <small>{project.openTaskCount}개 열림 · {percent}% 완료</small>
                    <span><i style={{ width: `${percent}%` }} /></span>
                  </span>
                  <Icon className="project-list-item__chevron" name="chevron" size={15} />
                </button>
              );
            })}
            {activeProjects.length === 0 && <EmptyState icon="projects" title="프로젝트가 없습니다" description="위에서 첫 프로젝트를 만들어 보세요." />}
          </div>
        </aside>

        {selected ? (
          <section className="panel project-detail" aria-labelledby="project-task-heading">
            <div className="project-detail__header">
              <div>
                <span className="section-kicker" style={{ color: selected.color }}><Icon name="projects" size={14} /> 선택한 프로젝트</span>
                <h2 id="project-task-heading">{selected.name}</h2>
                <p>{selected.description || "설명 없이 만든 프로젝트입니다."}</p>
              </div>
              <span className="count-pill">{visibleTaskCount}</span>
            </div>

            <ProjectQuickAdd key={selected.id} onCreate={onCreateTask} project={selected} />

            <div className="project-detail__toolbar">
              <div className="segmented-control" aria-label="프로젝트 Task 필터">
                <button aria-pressed={taskFilter === "open"} onClick={() => setTaskFilter("open")} type="button">열린 Task</button>
                <button aria-pressed={taskFilter === "closed"} onClick={() => setTaskFilter("closed")} type="button">완료 · 취소</button>
              </div>
              <span>{visibleTaskCount}개 표시</span>
            </div>

            <div className="project-task-groups">
              {groups.map((group) => {
                const groupedTasks = projectTasks.filter((task) => task.status === group.status);
                return (
                  <section className="project-task-group" key={group.status} aria-labelledby={`project-group-${group.status}`}>
                    <header>
                      <div>
                        <h3 id={`project-group-${group.status}`}>{group.label}</h3>
                        <p>{group.description}</p>
                      </div>
                      <span>{groupedTasks.length}</span>
                    </header>
                    {groupedTasks.length > 0 && (
                      <div className={`task-list ${group.status === "done" ? "task-list--completed" : ""}`}>
                        {groupedTasks.map((task) => <TaskCard key={task.id} onOpen={onOpen} onPlan={onPlan} task={task} />)}
                      </div>
                    )}
                  </section>
                );
              })}
              {visibleTaskCount === 0 && (
                <EmptyState
                  icon={taskFilter === "open" ? "check" : "history"}
                  title={taskFilter === "open" ? "열린 Task가 없습니다" : "완료하거나 취소한 Task가 없습니다"}
                  description={taskFilter === "open" ? "위 입력창에서 이 프로젝트의 첫 Task를 추가하세요." : "Task를 마치면 이곳에서 다시 볼 수 있습니다."}
                />
              )}
            </div>
          </section>
        ) : (
          <section className="panel project-detail project-detail--empty">
            <EmptyState icon="projects" title="프로젝트를 선택하세요" description="프로젝트를 만들거나 목록에서 선택하면 Task 작업공간이 열립니다." />
          </section>
        )}
      </div>
    </div>
  );
}

interface HistoryPageProps {
  history: HistoryDay[];
  onOpen: (task: Task) => void;
}

const dayStatusLabel: Record<DayEntryStatus, string> = {
  planned: "계획",
  done: "완료",
  deferred: "이월",
  skipped: "건너뜀",
};

export function HistoryPage({ history, onOpen }: HistoryPageProps) {
  return (
    <div className="page-stack">
      <header className="page-header">
        <div>
          <span className="eyebrow">변하지 않는 기록</span>
          <h1>히스토리</h1>
          <p>날짜별 계획과 확정 결과, 세션과 기록을 돌아봅니다.</p>
        </div>
      </header>
      <div className="history-list">
        {history.map((day, index) => (
          <section className="history-day" key={day.date} aria-labelledby={`history-${day.date}`}>
            <div className="history-day__date">
              <span>{new Intl.DateTimeFormat("ko-KR", { month: "short", timeZone: "Asia/Seoul" }).format(new Date(`${day.date}T12:00:00+09:00`))}</span>
              <strong>{day.date.slice(-2)}</strong>
              {index === 0 && <em>오늘</em>}
            </div>
            <div className="history-day__content">
              <div className="history-day__heading">
                <h2 id={`history-${day.date}`}>{friendlyDate(day.date)}</h2>
                <div className="history-day__summary">
                  <span><Icon name="timer" size={14} /> {day.sessionMinutes}분</span>
                  <span><Icon name="worklog" size={14} /> WorkLog {day.workLogCount}</span>
                </div>
              </div>
              <div className="history-entries">
                {day.entries.map((entry) => (
                  <button className="history-entry" key={entry.id} onClick={() => onOpen(entry.task)} type="button">
                    <span className={`history-entry__status history-entry__status--${entry.status}`}>
                      {entry.status === "done" ? <Icon name="check" size={14} /> : <Icon name="clock" size={14} />}
                      {dayStatusLabel[entry.status]}
                    </span>
                    <span>{entry.task.title}</span>
                    <Icon name="chevron" size={14} />
                  </button>
                ))}
                {day.entries.length === 0 && <p className="muted-copy">TaskDayEntry 기록은 없고 활동 기록만 있습니다.</p>}
              </div>
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
