import { useCallback, useEffect, useMemo, useState } from "react";

import type { TmApi } from "../lib/api";
import type { MailAccount, MailClassification, MailItem, MailReport, MailSummary } from "../types";
import { EmptyState } from "./EmptyState";
import { Icon } from "./Icon";

type MailTab = "important" | "review" | "reports" | "rules" | "status";

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : "메일 정보를 불러오지 못했습니다.";

const receivedAt = (value: string): string =>
  new Intl.DateTimeFormat("ko-KR", {
    timeZone: "Asia/Seoul",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));

const decisionLabel = (value: string): string => ({
  otp: "인증 요청",
  security_alert: "계정 보안 경고",
  payment_failure: "결제 실패",
  refund: "환불 확인",
  reservation_change: "예약 변경·취소",
  deadline: "기한 확인",
  action_required: "행동 필요",
  account_alert: "계정 알림",
  payment_or_refund: "결제·환불 확인",
  reservation_or_deadline: "예약·기한 확인",
  uncertain: "사용자 확인 필요",
}[value] ?? "중요도 판정");

interface MailPageProps {
  api: TmApi;
  today: string;
  onNotify: (message: string, type?: "success" | "error") => void;
}

export function MailPage({ api, today, onNotify }: MailPageProps) {
  const [tab, setTab] = useState<MailTab>("important");
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [items, setItems] = useState<MailItem[]>([]);
  const [reports, setReports] = useState<MailReport[]>([]);
  const [summary, setSummary] = useState<MailSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [naverFormOpen, setNaverFormOpen] = useState(false);
  const [naverEmail, setNaverEmail] = useState("");
  const [naverName, setNaverName] = useState("");
  const [naverPassword, setNaverPassword] = useState("");
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [nextAccounts, nextSummary, important, review, nextReports] = await Promise.all([
        api.getMailAccounts(),
        api.getMailSummary(today),
        api.listMailItems({ status: "important", limit: 100 }),
        api.listMailItems({ status: "review", limit: 100 }),
        api.getMailReports(today),
      ]);
      setAccounts(nextAccounts);
      setSummary(nextSummary);
      setItems([...important.items, ...review.items]);
      setReports(nextReports);
      setUnavailable(null);
    } catch (error) {
      setUnavailable(errorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [api, today]);

  useEffect(() => { void load(); }, [load]);

  const visibleItems = useMemo(() => {
    const classification: MailClassification = tab === "review" ? "review" : "important";
    return items
      .filter((item) => item.classification === classification)
      .sort((left, right) => right.receivedAt.localeCompare(left.receivedAt));
  }, [items, tab]);

  const replaceItem = (next: MailItem) => {
    setItems((current) => current.map((item) => item.id === next.id ? next : item));
  };

  const acknowledge = async (item: MailItem) => {
    try {
      replaceItem(await api.acknowledgeMailItem(item.id, item.version));
      onNotify("TM 안에서 메일 확인을 완료했습니다.");
    } catch (error) {
      onNotify(errorMessage(error), "error");
    }
  };

  const feedback = async (item: MailItem, important: boolean) => {
    try {
      replaceItem(await api.feedbackMailItem(item.id, important, item.version));
      onNotify("중요도 피드백을 반영했습니다.");
    } catch (error) {
      onNotify(errorMessage(error), "error");
    }
  };

  const startGmail = async () => {
    setSaving(true);
    try {
      const result = await api.startGmailOAuth();
      window.open(result.authorizationUrl, "_blank", "noopener,noreferrer");
      onNotify("브라우저에서 Gmail 읽기 전용 연결을 완료한 뒤 이 화면을 새로고침하세요.");
    } catch (error) {
      onNotify(errorMessage(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const connectNaver = async (event: React.FormEvent) => {
    event.preventDefault();
    setSaving(true);
    try {
      await api.connectNaverMail({
        email: naverEmail,
        displayName: naverName || undefined,
        appPassword: naverPassword,
      });
      setNaverPassword("");
      setNaverFormOpen(false);
      await load();
      onNotify("네이버 메일 읽기 전용 연결 정보를 저장했습니다.");
    } catch (error) {
      onNotify(errorMessage(error), "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="page-stack mail-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">READ ONLY · PRIVATE</span>
          <h1>메일</h1>
          <p>중요한 메일만 모아 보고, 원문은 공급자 웹메일에서 직접 확인합니다.</p>
        </div>
        <button className="secondary-button" disabled={loading} onClick={() => void load()} type="button">
          새로고침
        </button>
      </header>

      {unavailable && (
        <section className="panel mail-gate" role="status">
          <span><Icon name="shield" size={22} /></span>
          <div>
            <h2>메일 모니터링 준비 중</h2>
            <p>schema 17 구조는 설치되었지만 운영 기능 스위치나 보안키 설정이 아직 완료되지 않았습니다.</p>
            <small>{unavailable}</small>
          </div>
        </section>
      )}

      {!unavailable && (
        <>
          <section className="mail-metrics" aria-label="메일 요약">
            <div><span>미확인 중요</span><strong>{summary?.unacknowledgedImportant ?? 0}</strong></div>
            <div><span>확인 필요</span><strong>{summary?.reviewCount ?? 0}</strong></div>
            <div><span>연결 계정</span><strong>{accounts.filter((account) => account.status === "connected").length}</strong></div>
          </section>

          <nav className="segmented-tabs" aria-label="메일 보기">
            {([
              ["important", "중요 메일"],
              ["review", "확인 필요"],
              ["reports", "요약"],
              ["rules", "규칙"],
              ["status", "연동 상태"],
            ] as const).map(([id, label]) => (
              <button aria-current={tab === id ? "page" : undefined} key={id} onClick={() => setTab(id)} type="button">{label}</button>
            ))}
          </nav>

          {(tab === "important" || tab === "review") && (
            <section className="panel mail-list" aria-busy={loading}>
              {visibleItems.map((item) => (
                <article className={`mail-row ${item.acknowledgedAt ? "mail-row--done" : ""}`} key={item.id}>
                  <div className="mail-row__score" aria-label={`중요도 ${item.importanceScore}점`}>{item.importanceScore}</div>
                  <div className="mail-row__content">
                    <div className="mail-row__meta"><strong>{item.sender}</strong><span>{receivedAt(item.receivedAt)}</span></div>
                    <h2>{item.subject}</h2>
                    {item.summary && <p>{item.summary}</p>}
                    {item.action && <p className="mail-row__action"><strong>필요한 행동</strong> {item.action}</p>}
                    <small>{decisionLabel(item.decisionReason)}{item.deadline ? ` · 기한 ${item.deadline}` : ""}</small>
                  </div>
                  <div className="mail-row__actions">
                    <a className="secondary-button" href={item.originalUrl} rel="noreferrer" target="_blank">원문 열기</a>
                    {!item.acknowledgedAt && <button onClick={() => void acknowledge(item)} type="button">확인 완료</button>}
                    <button onClick={() => void feedback(item, true)} type="button">중요</button>
                    <button onClick={() => void feedback(item, false)} type="button">중요하지 않음</button>
                  </div>
                </article>
              ))}
              {!loading && visibleItems.length === 0 && <EmptyState icon="inbox" title={tab === "important" ? "미확인 중요 메일이 없습니다" : "직접 확인할 메일이 없습니다"} description="규칙과 피드백을 반영해 필요한 항목만 이곳에 표시합니다." />}
            </section>
          )}

          {tab === "reports" && (
            <section className="panel mail-reports">
              <div className="panel__header"><div><span className="section-kicker">08:00 · 19:00</span><h2>오늘의 요약</h2></div></div>
              {reports.map((report) => <article key={report.id}><strong>{report.slot === "morning" ? "아침" : "저녁"} 요약</strong><p>{report.summary}</p><small>중요 {report.importantCount} · 확인 필요 {report.reviewCount}</small></article>)}
              {reports.length === 0 && <EmptyState icon="note" title="아직 생성된 요약이 없습니다" description="연동을 활성화하면 서울 시간 08:00과 19:00에 무료 집계 요약을 만듭니다." />}
            </section>
          )}

          {tab === "rules" && (
            <section className="panel mail-rules">
              <div className="panel__header"><div><span className="section-kicker">RULES FIRST</span><h2>기본 중요 규칙</h2><p>AI를 쓰기 전에 명확한 신호를 무료로 판정합니다.</p></div></div>
              <ul>
                <li><strong>항상 긴급</strong><span>계정 보안 경고, 결제 실패, 예약 취소·변경</span></li>
                <li><strong>행동 필요</strong><span>환불, 제출·납부 마감, 계정 보안 알림</span></li>
                <li><strong>기본 제외</strong><span>Spam, Trash, 광고·뉴스레터, 중복 알림</span></li>
                <li><strong>민감 정보</strong><span>OTP·인증번호·비밀번호 재설정은 AI에 보내지 않음</span></li>
              </ul>
            </section>
          )}

          {tab === "status" && (
            <div className="mail-status-grid">
              <section className="panel">
                <div className="panel__header"><div><span className="section-kicker">ACCOUNTS</span><h2>연동 계정</h2></div></div>
                <div className="mail-account-list">
                  {accounts.map((account) => (
                    <article key={account.id}>
                      <span className={`status-dot status-dot--${account.status === "connected" ? "ok" : "warning"}`} />
                      <div><strong>{account.displayName || account.email}</strong><small>{account.provider === "gmail" ? "Gmail" : "네이버"} · {account.status === "connected" ? "정상" : "재인증 필요"}</small></div>
                      {api.canManageMailAccounts && account.status !== "disabled" && <button onClick={() => void api.disconnectMailAccount(account.id, account.version).then(load).catch((error) => onNotify(errorMessage(error), "error"))} type="button">연동 해제</button>}
                    </article>
                  ))}
                  {accounts.length === 0 && <p className="muted-copy">연동된 메일 계정이 없습니다.</p>}
                </div>
              </section>

              <section className="panel">
                <div className="panel__header"><div><span className="section-kicker">WINDOWS ADMIN</span><h2>계정 연결</h2><p>모바일에서는 연결 정보를 입력할 수 없습니다.</p></div></div>
                {api.canManageMailAccounts ? (
                  <div className="mail-connect-actions">
                    <button className="primary-button" disabled={saving} onClick={() => void startGmail()} type="button">Gmail 읽기 전용 연결</button>
                    <button className="secondary-button" disabled={saving} onClick={() => setNaverFormOpen((value) => !value)} type="button">네이버 앱 비밀번호 연결</button>
                    {naverFormOpen && (
                      <form className="mail-connect-form" onSubmit={(event) => void connectNaver(event)}>
                        <label>네이버 메일 주소<input autoComplete="username" onChange={(event) => setNaverEmail(event.target.value)} required type="email" value={naverEmail} /></label>
                        <label>표시 이름<input maxLength={120} onChange={(event) => setNaverName(event.target.value)} value={naverName} /></label>
                        <label>애플리케이션 비밀번호<input autoComplete="new-password" onChange={(event) => setNaverPassword(event.target.value)} required type="password" value={naverPassword} /></label>
                        <button className="primary-button" disabled={saving} type="submit">안전하게 저장</button>
                      </form>
                    )}
                  </div>
                ) : <p className="muted-copy">계정 연결과 해제는 Windows 관리자 앱에서만 가능합니다.</p>}
              </section>
            </div>
          )}
        </>
      )}
    </div>
  );
}

export function TodayMailCards({ api, onOpenMail, today }: { api: TmApi; onOpenMail: () => void; today: string }) {
  const [summary, setSummary] = useState<MailSummary | null>(null);
  useEffect(() => {
    let active = true;
    void api.getMailSummary(today).then((value) => { if (active) setSummary(value); }).catch(() => undefined);
    return () => { active = false; };
  }, [api, today]);
  if (!summary?.presentationEnabled || (summary.unacknowledgedImportant === 0 && summary.reviewCount === 0 && !summary.latestReport)) return null;
  return (
    <button className="today-mail-card" onClick={onOpenMail} type="button">
      <span><Icon name="mail" size={19} /></span>
      <div><strong>중요 메일 {summary.unacknowledgedImportant}개</strong><small>확인 필요 {summary.reviewCount}개{summary.latestReport ? ` · ${summary.latestReport.slot === "morning" ? "아침" : "저녁"} 요약 있음` : ""}</small></div>
      <Icon name="chevron" size={15} />
    </button>
  );
}
