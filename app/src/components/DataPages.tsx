import { useState } from "react";

import type { BackupInfo, ExportResult, TrashItem } from "../types";
import { EmptyState } from "./EmptyState";
import { Icon } from "./Icon";

const formatDateTime = (value: string): string =>
  new Intl.DateTimeFormat("ko-KR", {
    timeZone: "Asia/Seoul",
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));

const entityLabels: Record<TrashItem["entityType"], string> = {
  task: "Task",
  note: "Note",
  project: "프로젝트",
  work_log: "WorkLog",
  session: "작업 세션",
};

interface TrashPageProps {
  items: TrashItem[];
  onRestore: (itemId: string) => Promise<void>;
}

export function TrashPage({ items, onRestore }: TrashPageProps) {
  const [restoringId, setRestoringId] = useState<string | null>(null);
  return (
    <div className="page-stack">
      <header className="page-header"><div><span className="eyebrow">안전한 삭제</span><h1>휴지통</h1><p>삭제한 항목을 확인하고 원래 연결과 함께 복원합니다.</p></div><span className="count-pill">{items.length}개</span></header>
      <section className="panel" aria-labelledby="trash-heading">
        <div className="panel__header"><div><h2 id="trash-heading">삭제된 항목</h2><p>영구 삭제는 데이터 보존 정책에 따라 별도 처리됩니다.</p></div></div>
        <div className="trash-list">
          {items.map((item) => (
            <article key={item.id}><span className="trash-list__icon"><Icon name="trash" /></span><div><span>{entityLabels[item.entityType]}</span><h3>{item.title}</h3><time dateTime={item.deletedAt}>{formatDateTime(item.deletedAt)} 삭제</time></div><button className="secondary-button" disabled={restoringId === item.id} onClick={async () => { setRestoringId(item.id); try { await onRestore(item.id); } finally { setRestoringId(null); } }} type="button"><Icon name="restore" size={15} /> {restoringId === item.id ? "복원 중…" : "복원"}</button></article>
          ))}
          {items.length === 0 && <EmptyState icon="trash" title="휴지통이 비었습니다" description="삭제한 Task와 Note가 여기에 표시됩니다." />}
        </div>
      </section>
    </div>
  );
}

const triggerLabels: Record<BackupInfo["trigger"], string> = {
  startup: "앱 시작",
  shutdown: "앱 종료",
  daily: "일일 자동",
  manual: "수동",
  pre_migration: "Migration 전",
  pre_restore: "복원 전",
};

interface DataPageProps {
  backups: BackupInfo[];
  databasePath: string;
  lastBackupAt: string | null;
  onBackup: () => Promise<void>;
  onRestore: (backupId: string) => Promise<void>;
  onExport: () => Promise<ExportResult>;
}

export function DataPage({ backups, databasePath, lastBackupAt, onBackup, onRestore, onExport }: DataPageProps) {
  const [working, setWorking] = useState<"backup" | "restore" | "export" | null>(null);
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const runBackup = async () => { setWorking("backup"); try { await onBackup(); } finally { setWorking(null); } };
  const restore = async (backup: BackupInfo) => {
    if (!window.confirm(`${backup.fileName}으로 복원할까요?\n현재 DB는 복원 전에 자동으로 안전 백업됩니다.`)) return;
    setWorking("restore"); try { await onRestore(backup.id); } finally { setWorking(null); }
  };
  const exportAll = async () => { setWorking("export"); try { setExportResult(await onExport()); } finally { setWorking(null); } };
  return (
    <div className="page-stack">
      <header className="page-header"><div><span className="eyebrow">로컬 데이터 안전</span><h1>백업 · 내보내기</h1><p>데이터베이스를 보호하고 전체 기록을 이동 가능한 형식으로 보관합니다.</p></div><span className="safety-pill"><Icon name="shield" size={15} /> 외부 데이터 보존</span></header>
      <div className="data-overview">
        <article className="data-stat data-stat--primary"><span><Icon name="database" /></span><div><small>데이터베이스</small><strong>정상</strong><code title={databasePath}>{databasePath}</code></div></article>
        <article className="data-stat"><span><Icon name="shield" /></span><div><small>최근 백업</small><strong>{lastBackupAt ? formatDateTime(lastBackupAt) : "없음"}</strong><p>최근 DB 백업 {backups.length}개 보관</p></div></article>
        <article className="data-stat"><span><Icon name="clock" /></span><div><small>자동 정책</small><strong>보호 활성화</strong><p>시작 · 종료 · 서울 날짜 하루 1회</p></div></article>
      </div>
      <div className="data-action-grid">
        <article className="data-action-card"><span className="data-action-card__icon"><Icon name="database" size={23} /></span><div><h2>지금 DB 백업</h2><p>최신 상태를 새 타임스탬프 파일로 안전하게 저장합니다.</p></div><button className="primary-button" disabled={working !== null} onClick={() => void runBackup()} type="button"><Icon name="plus" size={15} /> {working === "backup" ? "백업 중…" : "새 백업 만들기"}</button></article>
        <article className="data-action-card"><span className="data-action-card__icon data-action-card__icon--warm"><Icon name="download" size={23} /></span><div><h2>전체 내보내기</h2><p>하나의 read transaction에서 JSON과 Markdown을 함께 생성합니다.</p></div><button className="secondary-button" disabled={working !== null} onClick={() => void exportAll()} type="button"><Icon name="download" size={15} /> {working === "export" ? "내보내는 중…" : "JSON · Markdown"}</button></article>
      </div>
      {exportResult && <aside className="export-success" role="status"><span><Icon name="check" /></span><div><strong>전체 내보내기를 완료했습니다.</strong><code>{exportResult.jsonPath}</code><code>{exportResult.markdownPath}</code></div><button aria-label="알림 닫기" className="icon-button" onClick={() => setExportResult(null)} type="button"><Icon name="close" size={15} /></button></aside>}
      <section className="panel" aria-labelledby="backup-list-heading">
        <div className="panel__header"><div><h2 id="backup-list-heading">DB 백업 기록</h2><p>최근 30개를 유지하며 복원 전 현재 DB를 다시 백업합니다.</p></div><span className="count-pill">{backups.length} / 30</span></div>
        <div className="backup-list">
          {backups.map((backup, index) => (
            <article key={backup.id}><span className="backup-list__icon"><Icon name="database" size={18} /></span><div><div><strong>{backup.fileName}</strong>{index === 0 && <em>최신</em>}</div><span>{triggerLabels[backup.trigger]} · {formatDateTime(backup.createdAt)} · {(backup.sizeBytes / 1_048_576).toFixed(1)} MB</span></div><button className="secondary-button secondary-button--small" disabled={working !== null} onClick={() => void restore(backup)} type="button"><Icon name="restore" size={14} /> 복원</button></article>
          ))}
          {backups.length === 0 && <EmptyState icon="database" title="백업 파일이 없습니다" description="새 백업을 만들어 현재 데이터를 보호하세요." />}
        </div>
      </section>
      <aside className="safety-note"><Icon name="shield" size={19} /><div><strong>설치 프로그램과 앱은 이 데이터 경로를 소유하지 않습니다.</strong><p>업데이트·재설치·제거 과정에서 <code>TM\data</code>를 삭제하거나 덮어쓰지 않도록 분리되어 있습니다.</p></div></aside>
    </div>
  );
}
