import { useState } from "react";

import type {
  CreateNoteInput,
  CreateWorkLogInput,
  Note,
  NoteType,
  Project,
  SearchResult,
  Task,
  WorkLog,
  WorkSession,
} from "../types";
import { EmptyState } from "./EmptyState";
import { Icon } from "./Icon";

const formatDateTime = (value: string): string =>
  new Intl.DateTimeFormat("ko-KR", {
    timeZone: "Asia/Seoul",
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));

const noteTypeLabels: Record<NoteType, string> = {
  concept: "개념",
  howto: "설치·방법",
  decision: "결정",
  reference: "참고자료",
  daily: "일일 기록",
};

interface WorkLogsPageProps {
  logs: WorkLog[];
  projects: Project[];
  tasks: Task[];
  onCreate: (input: CreateWorkLogInput) => Promise<void>;
  onPromote: (workLogId: string, type: NoteType, title: string) => Promise<void>;
  onTrash: (workLogId: string) => Promise<void>;
  onOpenTask: (task: Task) => void;
}

export function WorkLogsPage({ logs, projects, tasks, onCreate, onPromote, onTrash, onOpenTask }: WorkLogsPageProps) {
  const [showForm, setShowForm] = useState(false);
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [outcome, setOutcome] = useState("");
  const [blockers, setBlockers] = useState("");
  const [projectId, setProjectId] = useState("");
  const [taskIds, setTaskIds] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!title.trim() || !content.trim()) return;
    setSubmitting(true);
    try {
      await onCreate({
        title: title.trim(),
        content: content.trim(),
        outcome: outcome.trim(),
        blockers: blockers.trim(),
        projectId: projectId || null,
        taskIds,
        sessionId: null,
      });
      setTitle("");
      setContent("");
      setOutcome("");
      setBlockers("");
      setTaskIds([]);
      setShowForm(false);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="page-stack">
      <header className="page-header">
        <div><span className="eyebrow">시도와 결과</span><h1>WorkLog</h1><p>진행 과정과 시행착오를 Task 상태와 별도로 기록합니다.</p></div>
        <button className="primary-button" onClick={() => setShowForm((value) => !value)} type="button">
          <Icon name={showForm ? "close" : "plus"} size={15} /> {showForm ? "닫기" : "기록 작성"}
        </button>
      </header>

      {showForm && (
        <form className="editor-card" onSubmit={submit}>
          <div className="editor-card__heading"><span className="editor-card__icon"><Icon name="worklog" /></span><div><h2>새 WorkLog</h2><p>한 일보다 알아낸 것과 막힌 점을 구체적으로 남겨 보세요.</p></div></div>
          <div className="form-grid">
            <label className="field field--full"><span>제목 <em>필수</em></span><input required value={title} onChange={(event) => setTitle(event.target.value)} placeholder="예: 복원 테스트에서 발견한 잠금 조건" /></label>
            <label className="field field--full"><span>진행·시도 <em>필수</em></span><textarea required rows={4} value={content} onChange={(event) => setContent(event.target.value)} placeholder="무엇을 시도했고 어떤 과정을 거쳤나요?" /></label>
            <label className="field"><span>결과</span><textarea rows={3} value={outcome} onChange={(event) => setOutcome(event.target.value)} /></label>
            <label className="field"><span>막힌 점</span><textarea rows={3} value={blockers} onChange={(event) => setBlockers(event.target.value)} /></label>
            <label className="field field--full"><span>프로젝트</span><select value={projectId} onChange={(event) => { setProjectId(event.target.value); setTaskIds([]); }}><option value="">프로젝트 없음</option>{projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}</select></label>
          </div>
          <fieldset className="chip-picker"><legend>관련 Task <small>여러 개 선택 가능</small></legend><div>{tasks.filter((task) => !projectId || task.projectId === projectId).map((task) => <label key={task.id}><input checked={taskIds.includes(task.id)} onChange={() => setTaskIds((ids) => ids.includes(task.id) ? ids.filter((id) => id !== task.id) : [...ids, task.id])} type="checkbox" /><span>{task.title}</span></label>)}</div></fieldset>
          <div className="editor-card__actions"><button className="secondary-button" onClick={() => setShowForm(false)} type="button">취소</button><button className="primary-button" disabled={submitting || !title.trim() || !content.trim()} type="submit">{submitting ? "저장 중…" : "WorkLog 저장"}</button></div>
        </form>
      )}

      <div className="log-list">
        {logs.map((log) => (
          <article className="log-card" key={log.id}>
            <header>
              <span className="log-card__icon"><Icon name="worklog" size={17} /></span>
              <div><h2>{log.title}</h2><time dateTime={log.createdAt}>{formatDateTime(log.createdAt)}</time></div>
              <div className="card-actions">
                {log.promotedNoteId ? <span className="promoted-pill"><Icon name="note" size={13} /> Note로 승격됨</span> : (
                  <button className="secondary-button secondary-button--small" onClick={() => void onPromote(log.id, "concept", log.title)} type="button"><Icon name="arrow" size={13} /> Note로 승격</button>
                )}
                <button aria-label={`${log.title} 휴지통으로 이동`} className="secondary-button secondary-button--small danger-text" onClick={() => void onTrash(log.id)} type="button"><Icon name="trash" size={13} /> 휴지통</button>
              </div>
            </header>
            <p className="log-card__content">{log.content}</p>
            {(log.outcome || log.blockers) && <div className="log-card__result">{log.outcome && <div><span><Icon name="check" size={14} /> 결과</span><p>{log.outcome}</p></div>}{log.blockers && <div className="log-card__blocker"><span><Icon name="flag" size={14} /> 막힌 점</span><p>{log.blockers}</p></div>}</div>}
            {log.taskIds.length > 0 && <footer><Icon name="link" size={14} /><span>연결</span>{log.taskIds.map((taskId) => { const task = tasks.find((item) => item.id === taskId); return task ? <button key={taskId} onClick={() => onOpenTask(task)} type="button">{task.title}</button> : null; })}</footer>}
          </article>
        ))}
        {logs.length === 0 && <EmptyState icon="worklog" title="WorkLog가 없습니다" description="진행 과정과 시도한 내용을 첫 기록으로 남겨 보세요." />}
      </div>
    </div>
  );
}

interface NotesPageProps {
  notes: Note[];
  tasks: Task[];
  sessions: WorkSession[];
  onCreate: (input: CreateNoteInput) => Promise<void>;
  onTrash: (noteId: string) => Promise<void>;
  onOpenTask: (task: Task) => void;
}

export function NotesPage({ notes, tasks, sessions, onCreate, onTrash, onOpenTask }: NotesPageProps) {
  const [filter, setFilter] = useState<NoteType | "all">("all");
  const [showForm, setShowForm] = useState(false);
  const [type, setType] = useState<NoteType>("concept");
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [taskIds, setTaskIds] = useState<string[]>([]);
  const [sessionIds, setSessionIds] = useState<string[]>([]);
  const [urls, setUrls] = useState("");
  const [filePaths, setFilePaths] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const visibleNotes = notes.filter((note) => filter === "all" || note.type === filter);

  const toggle = (value: string, selected: string[], setter: React.Dispatch<React.SetStateAction<string[]>>) => {
    setter(selected.includes(value) ? selected.filter((item) => item !== value) : [...selected, value]);
  };

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!title.trim() || !content.trim()) return;
    setSubmitting(true);
    try {
      await onCreate({
        type,
        title: title.trim(),
        content: content.trim(),
        links: {
          taskIds,
          sessionIds,
          workLogIds: [],
          urls: urls.split("\n").map((value) => value.trim()).filter(Boolean),
          filePaths: filePaths.split("\n").map((value) => value.trim()).filter(Boolean),
        },
      });
      setTitle(""); setContent(""); setTaskIds([]); setSessionIds([]); setUrls(""); setFilePaths(""); setShowForm(false);
    } finally { setSubmitting(false); }
  };

  return (
    <div className="page-stack">
      <header className="page-header">
        <div><span className="eyebrow">재사용 가능한 지식</span><h1>Note</h1><p>개념, 방법, 결정, 참고자료, 일일 기록을 서로 연결합니다.</p></div>
        <button className="primary-button" onClick={() => setShowForm((value) => !value)} type="button"><Icon name={showForm ? "close" : "plus"} size={15} /> {showForm ? "닫기" : "Note 작성"}</button>
      </header>
      <div className="filter-tabs" aria-label="Note 유형 필터">
        <button aria-pressed={filter === "all"} onClick={() => setFilter("all")} type="button">전체 <span>{notes.length}</span></button>
        {(Object.keys(noteTypeLabels) as NoteType[]).map((noteType) => <button aria-pressed={filter === noteType} key={noteType} onClick={() => setFilter(noteType)} type="button">{noteTypeLabels[noteType]} <span>{notes.filter((note) => note.type === noteType).length}</span></button>)}
      </div>
      {showForm && (
        <form className="editor-card" onSubmit={submit}>
          <div className="editor-card__heading"><span className="editor-card__icon"><Icon name="note" /></span><div><h2>새 Note</h2><p>Task와 세션을 여러 개 연결해 지식을 맥락 속에 보관합니다.</p></div></div>
          <div className="form-grid">
            <label className="field"><span>유형</span><select value={type} onChange={(event) => setType(event.target.value as NoteType)}>{(Object.keys(noteTypeLabels) as NoteType[]).map((noteType) => <option key={noteType} value={noteType}>{noteTypeLabels[noteType]}</option>)}</select></label>
            <label className="field"><span>제목 <em>필수</em></span><input required value={title} onChange={(event) => setTitle(event.target.value)} /></label>
            <label className="field field--full"><span>내용 <em>필수</em></span><textarea required rows={6} value={content} onChange={(event) => setContent(event.target.value)} /></label>
          </div>
          <div className="note-link-grid">
            <fieldset className="chip-picker"><legend>관련 Task</legend><div>{tasks.filter((task) => !task.deletedAt).map((task) => <label key={task.id}><input checked={taskIds.includes(task.id)} onChange={() => toggle(task.id, taskIds, setTaskIds)} type="checkbox" /><span>{task.title}</span></label>)}</div></fieldset>
            <fieldset className="chip-picker"><legend>관련 세션</legend><div>{sessions.map((session) => <label key={session.id}><input checked={sessionIds.includes(session.id)} onChange={() => toggle(session.id, sessionIds, setSessionIds)} type="checkbox" /><span>{session.goal}</span></label>)}</div></fieldset>
            <label className="field"><span>URL <small>줄마다 하나</small></span><textarea rows={3} placeholder="https://…" value={urls} onChange={(event) => setUrls(event.target.value)} /></label>
            <label className="field"><span>파일 경로 <small>줄마다 하나</small></span><textarea rows={3} placeholder="C:\\…" value={filePaths} onChange={(event) => setFilePaths(event.target.value)} /></label>
          </div>
          <div className="editor-card__actions"><button className="secondary-button" onClick={() => setShowForm(false)} type="button">취소</button><button className="primary-button" disabled={submitting || !title.trim() || !content.trim()} type="submit">{submitting ? "저장 중…" : "Note 저장"}</button></div>
        </form>
      )}
      <div className="note-grid">
        {visibleNotes.map((note) => (
          <article className={`note-card note-card--${note.type}`} key={note.id}>
            <header><span className="note-type"><Icon name={note.type === "reference" ? "link" : note.type === "daily" ? "calendar" : "note"} size={14} /> {noteTypeLabels[note.type]}</span><button aria-label={`${note.title} 휴지통으로 이동`} className="secondary-button secondary-button--small danger-text" onClick={() => void onTrash(note.id)} type="button"><Icon name="trash" size={13} /> 휴지통</button></header>
            <h2>{note.title}</h2><p>{note.content}</p>
            <footer><time dateTime={note.updatedAt}>{formatDateTime(note.updatedAt)}</time><span className="note-links"><Icon name="link" size={13} /> {note.links.taskIds.length + note.links.sessionIds.length + note.links.workLogIds.length + note.links.filePaths.length + note.links.urls.length}</span></footer>
            {note.links.taskIds.length > 0 && <div className="note-card__linked">{note.links.taskIds.map((taskId) => { const task = tasks.find((item) => item.id === taskId); return task ? <button key={taskId} onClick={() => onOpenTask(task)} type="button">{task.title}</button> : null; })}</div>}
          </article>
        ))}
        {visibleNotes.length === 0 && <EmptyState icon="note" title="해당 유형의 Note가 없습니다" description="다시 참고할 지식을 Note로 정리해 보세요." />}
      </div>
    </div>
  );
}

interface SearchPageProps {
  onSearch: (query: string) => Promise<SearchResult[]>;
  onOpenTask: (taskId: string) => void;
}

const resultTypeLabels: Record<SearchResult["entityType"], string> = { task: "Task", session: "세션", work_log: "WorkLog", note: "Note" };

export function SearchPage({ onSearch, onOpenTask }: SearchPageProps) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [searched, setSearched] = useState(false);
  const [loading, setLoading] = useState(false);
  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault(); if (!query.trim()) return; setLoading(true);
    try { setResults(await onSearch(query.trim())); setSearched(true); } finally { setLoading(false); }
  };
  return (
    <div className="page-stack search-page">
      <header className="page-header"><div><span className="eyebrow">SQLite FTS 통합 검색</span><h1>검색</h1><p>Task, 세션, WorkLog, Note를 한 번에 찾습니다.</p></div></header>
      <form className="search-hero" onSubmit={submit}><Icon name="search" size={22} /><input aria-label="통합 검색어" autoFocus placeholder="제목, 설명, 기록 속 단어를 검색하세요" value={query} onChange={(event) => setQuery(event.target.value)} /><kbd>Enter</kbd><button className="primary-button" disabled={loading || !query.trim()} type="submit">{loading ? "검색 중…" : "검색"}</button></form>
      {searched && <div className="search-summary"><span>“<strong>{query}</strong>” 검색 결과</span><strong>{results.length}개</strong></div>}
      <div className="search-results">
        {results.map((result) => {
          const content = (
            <>
              <span className={`search-result__icon search-result__icon--${result.entityType}`}><Icon name={result.entityType === "task" ? "check" : result.entityType === "session" ? "timer" : result.entityType === "work_log" ? "worklog" : "note"} /></span>
              <span className="search-result__body"><span><em>{resultTypeLabels[result.entityType]}</em><small>{result.meta}</small></span><strong>{result.title}</strong><p>{result.excerpt}</p></span>
            </>
          );
          return result.entityType === "task" && result.taskId ? (
            <button className="search-result" key={`${result.entityType}-${result.id}`} onClick={() => onOpenTask(result.taskId as string)} type="button">
              {content}<Icon name="chevron" size={16} />
            </button>
          ) : (
            <article className="search-result search-result--static" key={`${result.entityType}-${result.id}`}>
              {content}<span className="search-result__static-label">보기 전용</span>
            </article>
          );
        })}
        {searched && results.length === 0 && <EmptyState icon="search" title="검색 결과가 없습니다" description="다른 표현이나 더 짧은 검색어를 사용해 보세요." />}
        {!searched && <div className="search-suggestion"><span><Icon name="spark" size={22} /></span><div><strong>모든 기록에서 찾습니다</strong><p>예: “백업”, “SQLite 잠금”, “다음 행동”</p></div></div>}
      </div>
    </div>
  );
}
