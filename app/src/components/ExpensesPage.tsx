import { useCallback, useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import type { TmApi } from "../lib/api";
import { safeErrorText } from "../lib/error-text";
import type {
  CreateRecurringExpenseInput,
  EditableExpenseEventKind,
  ExpenseCategory,
  ExpenseEventKind,
  ExpenseImportPreview,
  ExpenseMonthSummary,
  ExpenseReportResult,
  ExpenseReview,
  ExpenseSourceStatus,
  ExpenseTransaction,
  RecurringAmountKind,
  RecurringDueRule,
  RecurringExpenseItem,
  RecurringExpenseOccurrence,
  RecurringExpenseStatus,
} from "../types";
import { EmptyState } from "./EmptyState";
import { Icon } from "./Icon";

type ExpenseTab = "summary" | "transactions" | "reviews" | "recurring" | "import";

interface ExpensesPageProps {
  api: TmApi;
  today: string;
  initialRecurringExpenseId?: string | null;
  onNotify: (message: string, type?: "success" | "error") => void;
}

const categoryLabels: Record<ExpenseCategory, string> = {
  food: "식비",
  delivery: "배달",
  cafe: "카페",
  groceries: "장보기",
  housing_utilities: "주거/공과금",
  transportation: "교통",
  ott_subscriptions: "OTT/구독",
  shopping: "쇼핑",
  health: "건강",
  leisure: "여가",
  education: "교육",
  travel: "여행",
  insurance_finance_tax: "보험/금융/세금",
  gifts_dues: "선물/회비",
  refund_income: "환불/수입",
  transfer_settlement: "이체/정산",
  other: "기타",
  unconfirmed: "미확인",
};

const eventKindLabels: Record<ExpenseEventKind, string> = {
  purchase: "구매",
  refund: "환불",
  settlement_received: "정산받음",
  settlement_sent: "정산보냄",
  fee: "수수료",
  internal_transfer: "내 계좌이동",
  card_payment: "카드대금",
  wallet_topup: "지갑충전",
  external_transfer: "외부 송금",
  unknown_p2p: "미확인 송금",
  manual_recurring: "수동 정기지출",
};

const editableEventKindEntries = Object.entries(eventKindLabels).filter(
  ([value]) => !["external_transfer", "unknown_p2p", "manual_recurring"].includes(value),
) as Array<[EditableExpenseEventKind, string]>;

const isEditableEventKind = (value: ExpenseEventKind | null): value is EditableExpenseEventKind =>
  value !== null && editableEventKindEntries.some(([candidate]) => candidate === value);

const expenseSourceLabels: Record<ExpenseSourceStatus["adapter"], string> = {
  kb_card_usage_v1: "KB 신용카드",
  kb_account_history_v1: "KB 계좌·체크카드",
  kakaopay_money_v1: "카카오페이머니",
};

const statusLabels = {
  confirmed: "확정",
  provisional: "잠정",
  incomplete: "불완전",
} as const;

const occurrenceStatusLabels = {
  scheduled: "예정",
  due_today: "오늘 납부",
  due_soon: "7일 이내",
  overdue: "기한 경과",
  paid: "납부 완료",
  matched: "거래 연결",
} as const;

const reviewReasonLabels: Record<ExpenseReview["reason"], string> = {
  unknown_p2p: "송금 목적 확인",
  ambiguous_mirror: "중복·이체 여부 확인",
  recurring_match_candidate: "정기지출 거래 연결 후보",
  recurring_registration_candidate: "정기지출 등록 후보",
  category_confirmation: "구매 카테고리 확인",
  import_rejected: "가져오기 거부 행 확인",
};

const formatMoney = (amountMinor: number, currency: string): string => {
  const zeroDecimal = new Set(["KRW", "JPY"]);
  const amount = zeroDecimal.has(currency) ? amountMinor : amountMinor / 100;
  try {
    return new Intl.NumberFormat("ko-KR", {
      style: "currency",
      currency,
      maximumFractionDigits: zeroDecimal.has(currency) ? 0 : 2,
    }).format(amount);
  } catch {
    return `${amount.toLocaleString("ko-KR")} ${currency}`;
  }
};

const factMetricLabels: Record<string, string> = {
  net_personal_spend: "순 개인지출",
  gross_purchase: "총구매",
  refunds: "환불",
  settlement_received: "정산받음",
  settlement_sent: "정산보냄",
  fees: "수수료",
  unconfirmed_outflow: "미확인 외부 유출",
  recurring_expected: "정기지출 예정",
  recurring_paid: "정기지출 납부",
  recurring_remaining: "정기지출 잔여",
};

const factMetricLabel = (metric: string): string => {
  const isDelta = metric.startsWith("delta.");
  const normalized = isDelta ? metric.slice("delta.".length) : metric;
  const label = normalized.startsWith("category.")
    ? categoryLabels[normalized.slice("category.".length) as ExpenseCategory] ?? "카테고리"
    : factMetricLabels[normalized] ?? "집계값";
  return isDelta ? `전월 대비 ${label}` : label;
};

const citedFactLabels = (report: ExpenseReportResult, factIds: string[]): string[] => {
  const facts = new Map(report.facts.map((fact) => [fact.factId, fact]));
  return factIds.flatMap((factId) => {
    const fact = facts.get(factId);
    return fact
      ? [`${factMetricLabel(fact.metric)} ${formatMoney(fact.amountMinor, fact.currency)}`]
      : [];
  });
};

const shiftMonth = (month: string, delta: number): string => {
  const [year, monthNumber] = month.split("-").map(Number);
  const date = new Date(Date.UTC(year, monthNumber - 1 + delta, 1));
  return `${date.getUTCFullYear()}-${String(date.getUTCMonth() + 1).padStart(2, "0")}`;
};

const shiftCalendarDate = (value: string, days: number): string => {
  const [year, month, day] = value.split("-").map(Number);
  const date = new Date(Date.UTC(year, month - 1, day));
  date.setUTCDate(date.getUTCDate() + days);
  return date.toISOString().slice(0, 10);
};

const monthTitle = (month: string): string => {
  const [year, monthNumber] = month.split("-").map(Number);
  return `${year}년 ${monthNumber}월`;
};

const basename = (path: string): string => path.split(/[\\/]/).at(-1) ?? "선택한 파일";

const errorText = (error: unknown): string =>
  safeErrorText(error, "요청을 처리하지 못했습니다.");

export function ExpensesPage({
  api,
  today,
  initialRecurringExpenseId = null,
  onNotify,
}: ExpensesPageProps) {
  const [month, setMonth] = useState(today.slice(0, 7));
  const [tab, setTab] = useState<ExpenseTab>(initialRecurringExpenseId ? "recurring" : "summary");
  const [registrationReview, setRegistrationReview] = useState<ExpenseReview | null>(null);
  const tabs: Array<{ id: ExpenseTab; label: string }> = [
    { id: "summary", label: "요약" },
    { id: "transactions", label: "거래" },
    { id: "reviews", label: "확인 필요" },
    { id: "recurring", label: "정기지출" },
    ...(api.canImportExpenses ? [{ id: "import" as const, label: "가져오기" }] : []),
  ];

  return (
    <div className="page-stack expenses-page">
      <header className="page-header expenses-page__header">
        <div>
          <span className="eyebrow">개인 금융 원장 · 원본 파일은 이 PC에만</span>
          <h1>지출</h1>
          <p>중복 결제와 계좌 이동을 제외한 순 개인지출을 통화별로 확인합니다.</p>
        </div>
        <div className="expense-month-picker" aria-label="지출 조회 월">
          <button aria-label="이전 달" className="icon-button" onClick={() => setMonth(shiftMonth(month, -1))} type="button"><Icon name="arrow" /></button>
          <strong>{monthTitle(month)}</strong>
          <button aria-label="다음 달" className="icon-button expense-month-picker__next" onClick={() => setMonth(shiftMonth(month, 1))} type="button"><Icon name="arrow" /></button>
        </div>
      </header>

      <nav className="expense-tabs" aria-label="지출 메뉴">
        {tabs.map((item) => (
          <button aria-current={tab === item.id ? "page" : undefined} key={item.id} onClick={() => setTab(item.id)} type="button">{item.label}</button>
        ))}
      </nav>

      {tab === "summary" && <ExpenseSummary api={api} month={month} onNotify={onNotify} />}
      {tab === "transactions" && <ExpenseTransactions api={api} month={month} onNotify={onNotify} />}
      {tab === "reviews" && (
        <ExpenseReviews
          api={api}
          month={month}
          onNotify={onNotify}
          onRegisterRecurring={(review) => {
            setRegistrationReview(review);
            setTab("recurring");
          }}
        />
      )}
      {tab === "recurring" && (
        <RecurringExpenses
          api={api}
          initialRecurringExpenseId={initialRecurringExpenseId}
          month={month}
          onNotify={onNotify}
          onRegistrationHandled={() => setRegistrationReview(null)}
          registrationReview={registrationReview}
          today={today}
        />
      )}
      {tab === "import" && api.canImportExpenses && <ExpenseImport api={api} onNotify={onNotify} />}
    </div>
  );
}

function ExpenseSummary({
  api,
  month,
  onNotify,
}: Pick<ExpensesPageProps, "api" | "onNotify"> & { month: string }) {
  const [summary, setSummary] = useState<ExpenseMonthSummary | null>(null);
  const [report, setReport] = useState<ExpenseReportResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [reportLoading, setReportLoading] = useState(false);
  const [ratingLoading, setRatingLoading] = useState(false);
  const [reportRetryNeeded, setReportRetryNeeded] = useState(false);
  const [sourceRevision, setSourceRevision] = useState(0);

  useEffect(() => setReportRetryNeeded(false), [month]);

  useEffect(() => {
    let active = true;
    setLoading(true);
    Promise.all([api.getExpenseSummary(month), api.latestExpenseReport(month)])
      .then(([nextSummary, nextReport]) => {
        if (!active) return;
        setSummary(nextSummary);
        setReport(nextReport);
      })
      .catch((error) => onNotify(errorText(error), "error"))
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [api, month, onNotify, sourceRevision]);

  const generateReport = async (newRequest = false) => {
    setReportLoading(true);
    if (newRequest) api.discardExpenseMutation("generate_expense_report", month);
    try {
      setReport(await api.generateExpenseReport(month));
      setReportRetryNeeded(false);
      onNotify("집계 데이터로 AI 지출 해설을 만들었습니다.");
    } catch (error) {
      setReportRetryNeeded(true);
      onNotify(errorText(error), "error");
    } finally {
      setReportLoading(false);
    }
  };

  const rateReport = async (helpful: boolean) => {
    if (!report) return;
    setRatingLoading(true);
    try {
      setReport(await api.rateExpenseReport(report.reportId, helpful));
      onNotify(helpful ? "도움됨으로 저장했습니다." : "도움 안 됨으로 저장했습니다.");
    } catch (error) {
      onNotify(errorText(error), "error");
    } finally {
      setRatingLoading(false);
    }
  };

  if (loading) return <section className="panel expense-loading" aria-live="polite">월간 지출을 계산하는 중입니다.</section>;
  if (!summary) return <EmptyState icon="chart" title="지출 요약을 불러오지 못했습니다" description="잠시 후 다시 시도해 주세요." />;
  const maxCategory = Math.max(1, ...summary.categories.map((item) => Math.abs(item.amountMinor)));

  return (
    <div className="expense-section-stack">
      <section className="panel expense-status-panel">
        <div>
          <span className={`expense-status expense-status--${summary.status}`}>{statusLabels[summary.status]}</span>
          <strong>출처 {summary.completeness.coveredSourceCount}/{summary.completeness.activeSourceCount}</strong>
          <span>미확인 {summary.completeness.pendingReviewCount}건 · 거부 행 {summary.completeness.rejectedRowCount}건</span>
        </div>
        <p>원화와 외화는 환산하지 않고 각각 표시합니다.</p>
      </section>

      <ExpenseSources
        api={api}
        onChanged={() => setSourceRevision((current) => current + 1)}
        onNotify={onNotify}
      />

      {summary.currencies.map((total) => (
        <section className="panel expense-currency" key={total.currency} aria-label={`${total.currency} 지출 요약`}>
          <div className="panel__header">
            <div><span className="eyebrow">{total.currency}</span><h2>순 개인지출 {formatMoney(total.netPersonalSpendMinor, total.currency)}</h2></div>
            {total.unconfirmedOutflowMinor > 0 && <span className="count-pill count-pill--attention">미확인 {formatMoney(total.unconfirmedOutflowMinor, total.currency)}</span>}
          </div>
          <div className="expense-metrics">
            <article><span>총구매</span><strong>{formatMoney(total.grossPurchaseMinor, total.currency)}</strong></article>
            <article><span>환불</span><strong>-{formatMoney(total.refundsMinor, total.currency)}</strong></article>
            <article><span>정산받음 / 보냄</span><strong>{formatMoney(total.settlementReceivedMinor, total.currency)} / {formatMoney(total.settlementSentMinor, total.currency)}</strong></article>
            <article><span>고정비 예정 / 납부 / 잔여</span><strong>{formatMoney(total.recurringExpectedMinor, total.currency)} / {formatMoney(total.recurringPaidMinor, total.currency)} / {formatMoney(total.recurringRemainingMinor, total.currency)}</strong></article>
          </div>
        </section>
      ))}

      <ExpenseDailyTrend summary={summary} />

      <div className="expense-summary-grid">
        <section className="panel expense-breakdown">
          <div className="panel__header"><div><span className="eyebrow">확정 거래</span><h2>카테고리</h2></div></div>
          <div className="expense-bars">
            {summary.categories.map((item) => (
              <div key={`${item.category}-${item.currency}`}>
                <span>{categoryLabels[item.category]} · {item.currency}</span>
                <i><b style={{ width: `${Math.max(3, Math.abs(item.amountMinor) / maxCategory * 100)}%` }} /></i>
                <strong>{formatMoney(item.amountMinor, item.currency)}</strong>
              </div>
            ))}
            {summary.categories.length === 0 && <p className="calendar-empty">확정된 지출이 없습니다.</p>}
          </div>
        </section>
        <section className="panel expense-breakdown">
          <div className="panel__header"><div><span className="eyebrow">무료 알고리즘</span><h2>반복 지출 후보</h2></div></div>
          <div className="expense-candidate-list">
            {summary.recurringCandidates > 0
              ? <article><div><strong>{summary.recurringCandidates}개 후보</strong><span>3회 이상 반복되고 간격과 금액이 안정적인 거래</span></div><span>확인 필요 탭에서 검토</span></article>
              : <p className="calendar-empty">새 정기지출 후보가 없습니다.</p>}
          </div>
        </section>
      </div>

      <section className="panel expense-ai-report" aria-busy={reportLoading}>
        <div className="panel__header">
          <div><span className="section-kicker section-kicker--accent"><Icon name="spark" size={14} /> 수동 호출</span><h2>AI 지출 해설</h2><p>업체·상대방·개별 거래 없이 확정 집계와 fact ID만 전송합니다.</p></div>
          <div className="expense-ai-actions"><button className="primary-button" disabled={reportLoading} onClick={() => void generateReport(false)} type="button">{reportLoading ? "생성 중…" : reportRetryNeeded ? "같은 요청 다시 확인" : report ? "다시 생성" : "AI 해설 생성"}</button>{reportRetryNeeded && <button className="secondary-button secondary-button--small" disabled={reportLoading} onClick={() => void generateReport(true)} type="button">새 AI 요청으로 다시 시도 (새 횟수·최대 $0.05가 발생할 수 있음)</button>}</div>
        </div>
        {report ? (
          <div className="expense-report-result">
            <h3>{report.title}</h3><p>{report.summary}</p>
            {[...report.observations, ...report.alerts].map((item, index) => {
              const citations = citedFactLabels(report, item.factIds);
              return <article key={`${item.text}-${index}`}><span>{citations.length ? citations.join(" · ") : "확정 집계 근거"}</span><strong>{item.text}</strong></article>;
            })}
            <h4>다음 달 확인</h4><ul>{report.nextMonthChecks.map((item) => <li key={item}>{item}</li>)}</ul>
            <div className="expense-report-feedback" aria-label="AI 해설 평가">
              <span>{report.helpful === null ? "이 해설이 도움됐나요?" : "평가가 저장되었습니다."}</span>
              <button aria-pressed={report.helpful === true} className="secondary-button secondary-button--small" disabled={ratingLoading} onClick={() => void rateReport(true)} type="button">도움됨</button>
              <button aria-pressed={report.helpful === false} className="secondary-button secondary-button--small" disabled={ratingLoading} onClick={() => void rateReport(false)} type="button">도움 안 됨</button>
            </div>
          </div>
        ) : <p className="expense-ai-empty">기본 요약은 AI 호출 없이 항상 사용할 수 있습니다.</p>}
      </section>
    </div>
  );
}

function ExpenseSources({
  api,
  onChanged,
  onNotify,
}: Pick<ExpensesPageProps, "api" | "onNotify"> & { onChanged: () => void }) {
  const [sources, setSources] = useState<ExpenseSourceStatus[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let active = true;
    setLoading(true);
    api.listExpenseSources()
      .then((items) => { if (active) setSources(items); })
      .catch((error) => onNotify(errorText(error), "error"))
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [api, onNotify]);

  const update = async (
    source: ExpenseSourceStatus,
    requiredForCompleteReport: boolean,
    isActive: boolean,
  ) => {
    try {
      const updated = await api.updateExpenseSource(source.id, {
        requiredForCompleteReport,
        isActive,
        expectedVersion: source.version,
      });
      setSources((items) => items.map((item) => item.id === updated.id ? updated : item));
      onNotify("지출 출처 설정을 변경했습니다.");
      onChanged();
    } catch (error) {
      onNotify(`${errorText(error)} 최신 출처 상태를 다시 불러와 주세요.`, "error");
    }
  };

  return (
    <section className="panel expense-sources" aria-busy={loading}>
      <div className="panel__header"><div><span className="eyebrow">리포트 완성도</span><h2>데이터 출처</h2><p>활성 출처와 확정 리포트에 꼭 필요한 출처를 별도로 관리합니다.</p></div></div>
      <div className="expense-source-list">
        {sources.map((source) => (
          <ExpenseSourceRow key={source.id} onSave={update} source={source} />
        ))}
        {!loading && sources.length === 0 && <p className="calendar-empty">가져온 지출 출처가 없습니다.</p>}
      </div>
    </section>
  );
}

function ExpenseSourceRow({
  source,
  onSave,
}: {
  source: ExpenseSourceStatus;
  onSave: (
    source: ExpenseSourceStatus,
    requiredForCompleteReport: boolean,
    isActive: boolean,
  ) => Promise<void>;
}) {
  const [required, setRequired] = useState(source.requiredForCompleteReport);
  const [active, setActive] = useState(source.isActive);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    setRequired(source.requiredForCompleteReport);
    setActive(source.isActive);
  }, [source]);
  const label = expenseSourceLabels[source.adapter];
  const changed = required !== source.requiredForCompleteReport || active !== source.isActive;
  return (
    <article>
      <div><strong>{label}</strong><span>{source.coverageStart && source.coverageEnd ? `${source.coverageStart}–${source.coverageEnd}` : "기간 정보 없음"}</span></div>
      <label className="check-field"><input aria-label={`${label} 활성 출처`} checked={active} onChange={(event) => setActive(event.target.checked)} type="checkbox" /> 활성</label>
      <label className="check-field"><input aria-label={`${label} 확정 리포트 필수`} checked={required} onChange={(event) => setRequired(event.target.checked)} type="checkbox" /> 확정 리포트 필수</label>
      <button className="secondary-button secondary-button--small" disabled={!changed || saving} onClick={() => {
        setSaving(true);
        void onSave(source, required, active).finally(() => setSaving(false));
      }} type="button">{saving ? "저장 중…" : "설정 저장"}</button>
    </article>
  );
}

function ExpenseDailyTrend({ summary }: { summary: ExpenseMonthSummary }) {
  const daysInMonth = Number(summary.monthEnd.slice(8, 10));
  const currencies = Array.from(new Set([
    ...summary.currencies.map((item) => item.currency),
    ...summary.daily.map((item) => item.currency),
  ])).sort();

  return (
    <section className="panel expense-daily-trend">
      <div className="panel__header"><div><span className="eyebrow">확정 순지출 · 통화별</span><h2>일별 추이</h2><p>환율로 합산하지 않고 각 통화를 독립적으로 표시합니다.</p></div></div>
      <div className="expense-daily-series-list">
        {currencies.map((currency) => {
          const byDate = new Map(summary.daily
            .filter((item) => item.currency === currency)
            .map((item) => [item.date, item.amountMinor]));
          const points = Array.from({ length: daysInMonth }, (_, index) => {
            const day = index + 1;
            const date = `${summary.month}-${String(day).padStart(2, "0")}`;
            return { date, day, amountMinor: byDate.get(date) ?? 0 };
          });
          const maximum = Math.max(1, ...points.map((point) => Math.abs(point.amountMinor)));
          return (
            <article key={currency}>
              <div><strong>{currency}</strong><span>{points.length}일</span></div>
              <div className="expense-daily-chart" role="img" aria-label={`${currency} 일별 순지출 막대 차트`}>
                {points.map((point) => (
                  <span
                    aria-label={`${point.date} ${formatMoney(point.amountMinor, currency)}`}
                    className={point.amountMinor < 0 ? "expense-daily-bar expense-daily-bar--negative" : "expense-daily-bar"}
                    key={point.date}
                    title={`${point.date} · ${formatMoney(point.amountMinor, currency)}`}
                  >
                    <i style={{ height: `${point.amountMinor === 0 ? 1 : Math.max(5, Math.abs(point.amountMinor) / maximum * 100)}%` }} />
                    <small>{point.day === 1 || point.day === daysInMonth || point.day % 5 === 0 ? point.day : ""}</small>
                  </span>
                ))}
              </div>
            </article>
          );
        })}
        {currencies.length === 0 && <p className="calendar-empty">일별로 표시할 확정 지출이 없습니다.</p>}
      </div>
    </section>
  );
}

function ExpenseTransactions({ api, month, onNotify }: Pick<ExpensesPageProps, "api" | "onNotify"> & { month: string }) {
  const [items, setItems] = useState<ExpenseTransaction[]>([]);
  const [candidateTransactions, setCandidateTransactions] = useState<ExpenseTransaction[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [category, setCategory] = useState<ExpenseCategory | "">("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async (append = false) => {
    setLoading(true);
    try {
      const pages = append
        ? [await api.listExpenseTransactions({ month, cursor: nextCursor ?? undefined, limit: 100 })]
        : await Promise.all([shiftMonth(month, -1), month, shiftMonth(month, 1)].map((candidateMonth) =>
          api.listExpenseTransactions({ month: candidateMonth, limit: 100 })));
      const page = append ? pages[0] : pages[1];
      setItems((current) => append ? [...current, ...page.items] : page.items);
      setCandidateTransactions((current) => [...new Map((append ? [...current, ...page.items] : pages.flatMap((candidatePage) => candidatePage.items))
        .map((transaction) => [transaction.id, transaction])).values()]);
      setNextCursor(page.nextCursor);
    } catch (error) {
      onNotify(errorText(error), "error");
    } finally {
      setLoading(false);
    }
  }, [api, month, nextCursor, onNotify]);

  useEffect(() => { void load(false); }, [api, month]);
  const visibleItems = category ? items.filter((item) => item.category === category) : items;

  const override = async (
    item: ExpenseTransaction,
    input: {
      kind: EditableExpenseEventKind;
      category: ExpenseCategory;
      duplicateOfEventId: string | null;
      relatedEventId: string | null;
      personalAmountMinor: number | null;
      clearPersonalAmount: boolean;
      clearRelatedEvent: boolean;
      createRule: boolean;
    },
  ) => {
    try {
      await api.overrideExpenseTransaction(item.id, { ...input, expectedVersion: item.version });
      setEditingId(null);
      onNotify(item.status === "excluded" ? "자동 제외를 해제하거나 새 분류로 저장했습니다." : "거래 분류를 변경했습니다.");
      await load(false);
    } catch (error) {
      onNotify(`${errorText(error)} 자동으로 다시 시도하지 않았습니다.`, "error");
    }
  };

  return (
    <section className="panel expense-list-panel">
      <div className="panel__header">
        <div><span className="eyebrow">확정·제외·미확인</span><h2>거래</h2></div>
        <label className="expense-filter"><span>카테고리</span><select onChange={(event) => setCategory(event.target.value as ExpenseCategory | "")} value={category}><option value="">전체</option>{Object.entries(categoryLabels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
      </div>
      <div className="expense-transaction-list" aria-busy={loading}>
        {visibleItems.map((item) => (
          <article className={item.status !== "confirmed" ? "expense-transaction--unconfirmed" : ""} key={item.id}>
            <div className="expense-transaction__summary"><div><span>{item.occurredAt.slice(0, 10)} · {eventKindLabels[item.kind]}</span><strong>{item.merchant ?? item.counterparty ?? "표시 이름 없음"}</strong><small>{categoryLabels[item.category]} · {item.sourceKind ?? "수동"} · {item.status}</small></div><strong>{formatMoney(item.amountMinor, item.currency)}</strong></div>
            {item.kind === "manual_recurring"
              ? <small>수동 납부 기록은 정기지출 발생 건에서 관리합니다.</small>
              : <button className="secondary-button secondary-button--small" onClick={() => {
                if (editingId === item.id) {
                  api.discardExpenseMutation("override_expense_transaction", item.id);
                  setEditingId(null);
                } else setEditingId(item.id);
              }} type="button">{editingId === item.id ? "재분류 취소" : "재분류·제외 해제"}</button>}
            {editingId === item.id && item.kind !== "manual_recurring" && (
              <ExpenseTransactionOverrideForm
                item={item}
                onSave={override}
                transactions={candidateTransactions}
              />
            )}
          </article>
        ))}
        {!loading && visibleItems.length === 0 && <EmptyState icon="chart" title="표시할 거래가 없습니다" description="가져온 거래나 선택한 필터를 확인해 주세요." />}
      </div>
      {nextCursor && <button className="secondary-button expense-more" disabled={loading} onClick={() => void load(true)} type="button">거래 더 보기</button>}
    </section>
  );
}

function ExpenseTransactionOverrideForm({
  item,
  onSave,
  transactions,
}: {
  item: ExpenseTransaction;
  onSave: (
    item: ExpenseTransaction,
    input: {
      kind: EditableExpenseEventKind;
      category: ExpenseCategory;
      duplicateOfEventId: string | null;
      relatedEventId: string | null;
      personalAmountMinor: number | null;
      clearPersonalAmount: boolean;
      clearRelatedEvent: boolean;
      createRule: boolean;
    },
  ) => Promise<void>;
  transactions: ExpenseTransaction[];
}) {
  const [kind, setKind] = useState<EditableExpenseEventKind>(
    isEditableEventKind(item.kind) ? item.kind : "purchase",
  );
  const [category, setCategory] = useState(item.category);
  const [duplicateOfEventId, setDuplicateOfEventId] = useState(item.duplicateOfEventId ?? "");
  const [relatedEventId, setRelatedEventId] = useState(item.relatedEventId ?? "");
  const [personalAmountMinor, setPersonalAmountMinor] = useState(
    item.personalAmountMinor === null ? "" : String(item.personalAmountMinor),
  );
  const [clearPersonalAmount, setClearPersonalAmount] = useState(false);
  const [clearRelatedEvent, setClearRelatedEvent] = useState(false);
  const [createRule, setCreateRule] = useState(false);
  const [saving, setSaving] = useState(false);
  const relatedCandidates = transactions.filter((candidate) =>
    candidate.id !== item.id
    && candidate.currency === item.currency
    && candidate.status === "confirmed"
    && !candidate.isProvisional
    && ["purchase", "refund"].includes(candidate.kind));
  const duplicateCandidates = transactions.filter((candidate) =>
    candidate.id !== item.id
    && candidate.currency === item.currency
    && candidate.amountMinor === item.amountMinor
    && candidate.status === "confirmed"
    && !candidate.isProvisional
    && !candidate.duplicateOfEventId);
  const settlementKind = kind === "settlement_received" || kind === "settlement_sent";

  return (
    <form className="expense-transaction-override" onSubmit={(event) => {
      event.preventDefault();
      setSaving(true);
      void onSave(item, {
        kind,
        category,
        duplicateOfEventId: duplicateOfEventId || null,
        relatedEventId: clearRelatedEvent || relatedEventId === (item.relatedEventId ?? "") ? null : relatedEventId || null,
        personalAmountMinor: clearPersonalAmount || personalAmountMinor === (item.personalAmountMinor === null ? "" : String(item.personalAmountMinor)) ? null : personalAmountMinor === "" ? null : Number(personalAmountMinor),
        clearPersonalAmount,
        clearRelatedEvent,
        createRule,
      }).finally(() => setSaving(false));
    }}>
      <p>{item.status === "excluded" ? "구매·환불·정산으로 다시 분류하면 자동 제외를 해제할 수 있습니다." : "제외 사유를 포함해 분류 결정을 다시 저장할 수 있습니다."}</p>
      <label><span>새 처리</span><select aria-label={`${item.id} 새 처리`} onChange={(event) => {
        const nextKind = event.target.value as EditableExpenseEventKind;
        setKind(nextKind);
        if (nextKind !== "settlement_received" && nextKind !== "settlement_sent") setRelatedEventId(item.relatedEventId ?? "");
        if (nextKind !== "purchase") setPersonalAmountMinor(item.personalAmountMinor === null ? "" : String(item.personalAmountMinor));
      }} value={kind}>{editableEventKindEntries.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
      <label><span>새 카테고리</span><select aria-label={`${item.id} 새 카테고리`} onChange={(event) => setCategory(event.target.value as ExpenseCategory)} value={category}>{Object.entries(categoryLabels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
      <label><span>중복 대상 (선택)</span><select onChange={(event) => { setDuplicateOfEventId(event.target.value); if (event.target.value) { setRelatedEventId(""); setPersonalAmountMinor(""); setClearRelatedEvent(Boolean(item.relatedEventId)); setClearPersonalAmount(item.personalAmountMinor !== null); } }} value={duplicateOfEventId}><option value="">중복 아님</option>{duplicateCandidates.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.postedDate} · {candidate.merchant ?? candidate.counterparty ?? "표시 이름 없음"} · {formatMoney(candidate.amountMinor, candidate.currency)}</option>)}</select></label>
      <label><span>정산·연결 대상 (선택)</span><select disabled={!settlementKind || Boolean(duplicateOfEventId) || clearRelatedEvent} onChange={(event) => { setRelatedEventId(event.target.value); setClearRelatedEvent(false); if (event.target.value) { setDuplicateOfEventId(""); setPersonalAmountMinor(""); setClearPersonalAmount(item.personalAmountMinor !== null); } }} value={relatedEventId}>{item.relatedEventId && !relatedCandidates.some((candidate) => candidate.id === item.relatedEventId) && <option value={item.relatedEventId}>현재 연결 거래</option>}<option value="">연결하지 않음</option>{relatedCandidates.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.postedDate} · {candidate.merchant ?? candidate.counterparty ?? "표시 이름 없음"} · {formatMoney(candidate.amountMinor, candidate.currency)}</option>)}</select><small>{settlementKind ? "앞뒤 달의 확정 구매·환불만 표시합니다." : "정산받음·정산보냄으로 처리할 때 연결할 수 있습니다."}</small></label>
      {item.relatedEventId && <label className="check-field"><input checked={clearRelatedEvent} onChange={(event) => { setClearRelatedEvent(event.target.checked); if (event.target.checked) setRelatedEventId(""); else setRelatedEventId(item.relatedEventId ?? ""); }} type="checkbox" /> 기존 정산 연결 명시적으로 해제</label>}
      <label><span>내 부담액 (minor unit·선택)</span><input disabled={kind !== "purchase" || Boolean(duplicateOfEventId) || clearPersonalAmount} max={Math.abs(item.amountMinor)} min="1" onChange={(event) => { setPersonalAmountMinor(event.target.value); setClearPersonalAmount(false); if (event.target.value) { setDuplicateOfEventId(""); setRelatedEventId(""); setClearRelatedEvent(Boolean(item.relatedEventId)); } }} placeholder="전체 금액" step="1" type="number" value={personalAmountMinor} /><small>{kind === "purchase" ? "구매액 중 내 부담액만 따로 지정합니다." : "구매로 처리할 때만 지정할 수 있습니다."}</small></label>
      {item.personalAmountMinor !== null && <label className="check-field"><input checked={clearPersonalAmount} onChange={(event) => { setClearPersonalAmount(event.target.checked); if (event.target.checked) setPersonalAmountMinor(""); else setPersonalAmountMinor(String(item.personalAmountMinor)); }} type="checkbox" /> 기존 내 부담액 {formatMoney(item.personalAmountMinor, item.currency)} 명시적으로 해제</label>}
      {item.merchant && item.paymentMethodFingerprint
        ? <label className="check-field"><input checked={createRule} onChange={(event) => setCreateRule(event.target.checked)} type="checkbox" /> 앞으로 같은 업체에 적용</label>
        : <small>업체와 결제수단을 모두 확인할 수 있는 거래만 자동 분류 규칙을 만들 수 있습니다.</small>}
      <button className="primary-button" disabled={saving} type="submit">{saving ? "저장 중…" : "재분류 저장"}</button>
    </form>
  );
}

function ExpenseReviews({ api, month, onNotify, onRegisterRecurring }: Pick<ExpensesPageProps, "api" | "onNotify"> & {
  month: string;
  onRegisterRecurring: (review: ExpenseReview) => void;
}) {
  const [reviews, setReviews] = useState<ExpenseReview[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [categoryNextCursor, setCategoryNextCursor] = useState<string | null>(null);
  const [transactions, setTransactions] = useState<ExpenseTransaction[]>([]);
  const [occurrences, setOccurrences] = useState<RecurringExpenseOccurrence[]>([]);
  const [loading, setLoading] = useState(true);
  const [showPurchaseCategories, setShowPurchaseCategories] = useState(false);
  const purchaseCategoryCount = reviews.filter((review) => review.reason === "category_confirmation").length;
  const visibleReviews = showPurchaseCategories
    ? reviews
    : reviews.filter((review) => review.reason !== "category_confirmation");

  const load = useCallback(async (cursor?: string) => {
    setLoading(true);
    if (!cursor) {
      setNextCursor(null);
      setCategoryNextCursor(null);
    }
    try {
      const transactionMonths = [shiftMonth(month, -1), month, shiftMonth(month, 1)];
      const [reviewPage, categoryPage, transactionPages, nextOccurrences] = await Promise.all([
        api.listExpenseReviews({ month, status: "pending", scope: "required", cursor, limit: 100 }),
        cursor
          ? Promise.resolve(null)
          : api.listExpenseReviews({ month, status: "pending", scope: "category_confirmation", limit: 100 }),
        Promise.all(transactionMonths.map((transactionMonth) =>
          api.listExpenseTransactions({ month: transactionMonth, limit: 100 }))),
        api.listRecurringExpenseOccurrences(month),
      ]);
      setReviews((current) => {
        if (!cursor) return [...reviewPage.items, ...(categoryPage?.items ?? [])];
        const merged = new Map(current.map((review) => [review.id, review]));
        reviewPage.items.forEach((review) => merged.set(review.id, review));
        return [...merged.values()];
      });
      setNextCursor(reviewPage.nextCursor);
      if (categoryPage) setCategoryNextCursor(categoryPage.nextCursor);
      setTransactions([...new Map(transactionPages
        .flatMap((page) => page.items)
        .map((transaction) => [transaction.id, transaction])).values()]);
      setOccurrences(nextOccurrences);
    } catch (error) {
      onNotify(errorText(error), "error");
    } finally {
      setLoading(false);
    }
  }, [api, month, onNotify]);
  useEffect(() => { void load(); }, [load]);

  const loadMoreCategories = async () => {
    if (!categoryNextCursor) return;
    setLoading(true);
    try {
      const page = await api.listExpenseReviews({
        month,
        status: "pending",
        scope: "category_confirmation",
        cursor: categoryNextCursor,
        limit: 100,
      });
      setReviews((current) => {
        const merged = new Map(current.map((review) => [review.id, review]));
        page.items.forEach((review) => merged.set(review.id, review));
        return [...merged.values()];
      });
      setCategoryNextCursor(page.nextCursor);
    } catch (error) {
      onNotify(errorText(error), "error");
    } finally {
      setLoading(false);
    }
  };

  const resolve = async (
    review: ExpenseReview,
    decision: EditableExpenseEventKind,
    category: ExpenseCategory,
    rememberRule: boolean,
    relatedEventId: string | null,
    personalAmountMinor: number | null,
    duplicateOfEventId: string | null,
  ) => {
    try {
      await api.resolveExpenseReview(review.id, {
        kind: decision,
        category,
        duplicateOfEventId,
        relatedEventId,
        personalAmountMinor,
        createRule: rememberRule,
        expectedVersion: review.version,
      });
      onNotify("거래 검토 결정을 저장했습니다.");
      await load();
    } catch (error) {
      onNotify(errorText(error), "error");
    }
  };

  const matchRecurring = async (
    review: ExpenseReview,
    occurrence: RecurringExpenseOccurrence,
    enableFutureAutoMatch: boolean,
  ) => {
    try {
      await api.matchRecurringExpenseOccurrence(
        occurrence.occurrenceKey,
        review.transaction.id,
        enableFutureAutoMatch,
        occurrence.version,
      );
      onNotify(enableFutureAutoMatch
        ? "거래를 연결하고 업체·결제수단의 이후 자동 연결을 켰습니다."
        : "거래를 정기지출에 연결했습니다.");
      await load();
    } catch (error) {
      onNotify(errorText(error), "error");
    }
  };

  return (
    <section className="panel expense-list-panel">
      <div className="panel__header"><div><span className="eyebrow">필수 확인 우선</span><h2>확인 필요</h2><p>구매 카테고리는 검토하지 않아도 현재 분류로 합계에 반영됩니다. 불명확한 개인 간 송금만 결정 전까지 확정 지출에서 제외됩니다.</p></div><span className="count-pill count-pill--attention">{visibleReviews.length}</span></div>
      {purchaseCategoryCount > 0 && <button className="secondary-button expense-more" onClick={() => setShowPurchaseCategories((current) => !current)} type="button">{showPurchaseCategories ? "선택 분류 숨기기" : `선택 분류 ${purchaseCategoryCount}${categoryNextCursor ? "+" : ""}건 보기`}</button>}
      <div className="expense-review-list" aria-busy={loading}>
        {visibleReviews.map((review) => (
          <ExpenseReviewCard
            key={review.id}
            occurrences={occurrences}
            onMatchRecurring={matchRecurring}
            onRegisterRecurring={onRegisterRecurring}
            onResolve={resolve}
            review={review}
            transactions={transactions}
          />
        ))}
        {!loading && visibleReviews.length === 0 && <EmptyState icon="check" title="필수 확인 거래가 없습니다" description={purchaseCategoryCount > 0 ? "구매 분류는 선택 사항이며 합계에는 이미 반영되었습니다." : "현재 월의 거래 결정이 모두 완료되었습니다."} />}
      </div>
      {nextCursor && <button className="secondary-button expense-more" disabled={loading} onClick={() => void load(nextCursor)} type="button">검토 더 보기</button>}
      {showPurchaseCategories && categoryNextCursor && <button className="secondary-button expense-more" disabled={loading} onClick={() => void loadMoreCategories()} type="button">선택 분류 더 보기</button>}
    </section>
  );
}

function ExpenseReviewCard({ review, onResolve, transactions, occurrences, onMatchRecurring, onRegisterRecurring }: {
  review: ExpenseReview;
  onResolve: (
    review: ExpenseReview,
    decision: EditableExpenseEventKind,
    category: ExpenseCategory,
    rememberRule: boolean,
    relatedEventId: string | null,
    personalAmountMinor: number | null,
    duplicateOfEventId: string | null,
  ) => Promise<void>;
  transactions: ExpenseTransaction[];
  occurrences: RecurringExpenseOccurrence[];
  onMatchRecurring: (
    review: ExpenseReview,
    occurrence: RecurringExpenseOccurrence,
    enableFutureAutoMatch: boolean,
  ) => Promise<void>;
  onRegisterRecurring: (review: ExpenseReview) => void;
}) {
  const [decision, setDecision] = useState<EditableExpenseEventKind>(
    isEditableEventKind(review.suggestedKind) ? review.suggestedKind : "purchase",
  );
  const [category, setCategory] = useState<ExpenseCategory>(review.suggestedCategory ?? (decision.startsWith("settlement") ? "transfer_settlement" : "other"));
  const [remember, setRemember] = useState(
    review.reason === "category_confirmation"
      && Boolean(review.transaction.merchant)
      && Boolean(review.transaction.paymentMethodFingerprint),
  );
  const [relatedEventId, setRelatedEventId] = useState("");
  const [personalAmountMinor, setPersonalAmountMinor] = useState("");
  const [duplicateEventId, setDuplicateEventId] = useState(review.suggestedDuplicateOfEventId ?? "");
  const [enableFutureAutoMatch, setEnableFutureAutoMatch] = useState(false);
  const [saving, setSaving] = useState(false);
  const relatedCandidates = transactions.filter((transaction) =>
    transaction.id !== review.transaction.id
    && transaction.currency === review.transaction.currency
    && transaction.status === "confirmed"
    && !transaction.isProvisional
    && ["purchase", "refund"].includes(transaction.kind));
  const duplicateCandidates = transactions.filter((transaction) =>
    transaction.id !== review.transaction.id
    && transaction.currency === review.transaction.currency
    && transaction.amountMinor === review.transaction.amountMinor
    && transaction.status === "confirmed"
    && !transaction.isProvisional
    && !transaction.duplicateOfEventId);
  const settlementDecision = decision === "settlement_received" || decision === "settlement_sent";
  const suggestedDuplicateMissing = Boolean(
    review.suggestedDuplicateOfEventId
    && !duplicateCandidates.some((transaction) => transaction.id === review.suggestedDuplicateOfEventId),
  );
  const recurringOccurrence = review.recurringExpenseId
    ? occurrences.find((occurrence) => occurrence.recurringExpenseId === review.recurringExpenseId)
    : null;
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setSaving(true);
    try {
      await onResolve(
        review,
        decision,
        category,
        remember,
        relatedEventId || null,
        personalAmountMinor === "" ? null : Number(personalAmountMinor),
        duplicateEventId || null,
      );
    } finally { setSaving(false); }
  };
  const candidateHeader = <div><span>{review.transaction.occurredAt.slice(0, 10)}</span><h3>{review.transaction.merchant ?? review.transaction.counterparty ?? "표시 이름 없음"}</h3><p>{reviewReasonLabels[review.reason]}</p><strong>{formatMoney(review.transaction.amountMinor, review.transaction.currency)}</strong></div>;

  if (review.reason === "recurring_match_candidate") {
    const candidateResolved = ["paid", "matched"].includes(recurringOccurrence?.status ?? "");
    return (
      <article className="expense-review-card expense-review-card--candidate">
        {candidateHeader}
        {recurringOccurrence ? <p><strong>{recurringOccurrence.name}</strong>의 {recurringOccurrence.dueDate} 발생 건과 조건이 일치합니다.</p> : <p>연결할 정기지출 발생 건을 다시 불러와 주세요.</p>}
        <label className="check-field"><input checked={enableFutureAutoMatch} disabled={!recurringOccurrence || candidateResolved} onChange={(event) => setEnableFutureAutoMatch(event.target.checked)} type="checkbox" /> 앞으로 고신뢰 거래도 자동 연결</label>
        <small>처음 연결을 확인하면 업체·결제수단을 안전하게 학습하며, 자동 연결은 별도로 동의한 경우에만 켭니다.</small>
        <div className="recurring-form__actions">
          <button className="secondary-button" disabled={saving} onClick={() => void onResolve(review, "purchase", review.suggestedCategory ?? review.transaction.category, false, null, null, null)} type="button">이번에는 연결하지 않음</button>
          <button className="primary-button" disabled={!recurringOccurrence || candidateResolved || saving} onClick={() => {
            if (!recurringOccurrence) return;
            setSaving(true);
            void onMatchRecurring(review, recurringOccurrence, enableFutureAutoMatch).finally(() => setSaving(false));
          }} type="button">{saving ? "연결 중…" : "정기지출에 연결"}</button>
        </div>
      </article>
    );
  }

  if (review.reason === "recurring_registration_candidate") {
    return (
      <article className="expense-review-card expense-review-card--candidate">
        {candidateHeader}
        <p>반복 간격과 금액이 안정적인 거래입니다. 거래 정보로 정기지출 입력을 채운 뒤 내용을 확인하세요.</p>
        <div className="recurring-form__actions">
          <button className="secondary-button" disabled={saving} onClick={() => void onResolve(review, "purchase", review.suggestedCategory ?? review.transaction.category, false, null, null, null)} type="button">등록하지 않음</button>
          <button className="primary-button" onClick={() => onRegisterRecurring(review)} type="button">정기지출로 등록</button>
        </div>
      </article>
    );
  }

  return (
    <form className="expense-review-card" onSubmit={submit}>
      {candidateHeader}
      <label><span>처리</span><select onChange={(event) => {
        const nextDecision = event.target.value as EditableExpenseEventKind;
        setDecision(nextDecision);
        if (nextDecision !== "settlement_received" && nextDecision !== "settlement_sent") setRelatedEventId("");
        if (nextDecision !== "purchase") setPersonalAmountMinor("");
      }} value={decision}>{editableEventKindEntries.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
      <label><span>카테고리</span><select onChange={(event) => setCategory(event.target.value as ExpenseCategory)} value={category}>{Object.entries(categoryLabels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
      {review.reason === "ambiguous_mirror" && <label className="expense-review-related"><span>중복 대상</span><select onChange={(event) => { setDuplicateEventId(event.target.value); if (event.target.value) { setRelatedEventId(""); setPersonalAmountMinor(""); } }} value={duplicateEventId}><option value="">중복 아님</option>{suggestedDuplicateMissing && <option value={review.suggestedDuplicateOfEventId ?? ""}>제안된 동일 거래</option>}{duplicateCandidates.map((transaction) => <option key={transaction.id} value={transaction.id}>{transaction.postedDate} · {transaction.merchant ?? transaction.counterparty ?? "표시 이름 없음"} · {formatMoney(transaction.amountMinor, transaction.currency)}</option>)}</select><small>서버 제안을 수락하거나 같은 금액·통화의 다른 거래를 선택합니다.</small></label>}
      <label className="expense-review-related"><span>정산·연결 대상 (선택)</span><select disabled={!settlementDecision || Boolean(duplicateEventId)} onChange={(event) => { setRelatedEventId(event.target.value); if (event.target.value) { setPersonalAmountMinor(""); setDuplicateEventId(""); } }} value={relatedEventId}><option value="">연결하지 않음</option>{relatedCandidates.map((transaction) => <option key={transaction.id} value={transaction.id}>{transaction.postedDate} · {transaction.merchant ?? transaction.counterparty ?? "표시 이름 없음"} · {formatMoney(transaction.amountMinor, transaction.currency)}</option>)}</select><small>{settlementDecision ? "앞뒤 달의 확정 구매·환불만 표시합니다." : "정산받음·정산보냄으로 처리할 때 연결할 수 있습니다."}</small></label>
      <label><span>내 부담액 (minor unit·선택)</span><input disabled={decision !== "purchase" || Boolean(duplicateEventId)} max={Math.abs(review.transaction.amountMinor)} min="1" onChange={(event) => { setPersonalAmountMinor(event.target.value); if (event.target.value) { setRelatedEventId(""); setDuplicateEventId(""); } }} placeholder="전체 금액" step="1" type="number" value={personalAmountMinor} /><small>{decision === "purchase" ? "중복·연결 대상·내 부담액 중 하나만 선택합니다." : "구매로 처리할 때만 지정할 수 있습니다."}</small></label>
      {review.transaction.merchant && review.transaction.paymentMethodFingerprint
        ? <label className="check-field"><input checked={remember} onChange={(event) => setRemember(event.target.checked)} type="checkbox" /> {review.reason === "category_confirmation" ? "이 거래와 안전하게 일치하는 같은 업체·결제수단에 적용" : "앞으로 같은 업체에 적용"}</label>
        : <small>업체와 결제수단을 모두 확인할 수 있는 거래만 자동 분류 규칙을 만들 수 있습니다.</small>}
      <button className="primary-button" disabled={saving} type="submit">{saving ? "저장 중…" : "결정 저장"}</button>
    </form>
  );
}

interface RecurringFormState {
  name: string;
  category: ExpenseCategory;
  vendor: string;
  amount: string;
  currency: string;
  paymentMethodFingerprint: string;
  startDate: string;
  endDate: string;
  memo: string;
  reminderDays: string;
  amountKind: RecurringAmountKind;
  intervalMonths: "1" | "2" | "3" | "6" | "12";
  dueRule: RecurringDueRule;
  dueDay: string;
  status: RecurringExpenseStatus;
  autoMatchEnabled: boolean;
}

const recurringForm = (today: string, item?: RecurringExpenseItem | null): RecurringFormState => ({
  name: item?.name ?? "",
  category: item?.category ?? "ott_subscriptions",
  vendor: item?.vendor ?? "",
  amount: item ? String(item.amountMinor) : "",
  currency: item?.currency ?? "KRW",
  paymentMethodFingerprint: item?.paymentMethodFingerprint ?? "",
  startDate: item?.startDate ?? today,
  endDate: item?.endDate ?? "",
  memo: item?.memo ?? "",
  reminderDays: String(item?.reminderDays ?? 7),
  amountKind: item?.amountKind ?? "fixed",
  intervalMonths: String(item?.intervalMonths ?? 1) as RecurringFormState["intervalMonths"],
  dueRule: item?.dueRule ?? "specific_day",
  dueDay: String(item?.dueDay ?? Number(today.slice(8, 10))),
  status: item?.status ?? "active",
  autoMatchEnabled: item?.autoMatchEnabled ?? false,
});

const recurringFormFromReview = (review: ExpenseReview): RecurringFormState => {
  const transaction = review.transaction;
  const displayName = transaction.merchant ?? transaction.counterparty ?? "정기지출";
  return {
    name: displayName,
    category: review.suggestedCategory === "unconfirmed"
      ? "other"
      : review.suggestedCategory ?? (transaction.category === "unconfirmed" ? "other" : transaction.category),
    vendor: transaction.merchant ?? "",
    amount: String(transaction.amountMinor),
    currency: transaction.currency,
    paymentMethodFingerprint: transaction.paymentMethodFingerprint ?? "",
    startDate: transaction.postedDate,
    endDate: "",
    memo: "반복 거래 후보에서 등록",
    reminderDays: "7",
    amountKind: "fixed",
    intervalMonths: "1",
    dueRule: "specific_day",
    dueDay: String(Number(transaction.postedDate.slice(8, 10))),
    status: "active",
    autoMatchEnabled: false,
  };
};

const sameRecurringRegistration = (
  item: RecurringExpenseItem,
  review: ExpenseReview,
): boolean => {
  const transaction = review.transaction;
  return item.currency === transaction.currency
    && item.amountMinor === transaction.amountMinor
    && item.vendor === (transaction.merchant ?? null)
    && item.paymentMethodFingerprint === transaction.paymentMethodFingerprint;
};

function RecurringExpenses({
  api,
  month,
  today,
  initialRecurringExpenseId,
  onNotify,
  registrationReview,
  onRegistrationHandled,
}: Pick<ExpensesPageProps, "api" | "today" | "initialRecurringExpenseId" | "onNotify"> & {
  month: string;
  registrationReview: ExpenseReview | null;
  onRegistrationHandled: () => void;
}) {
  const [items, setItems] = useState<RecurringExpenseItem[]>([]);
  const [occurrences, setOccurrences] = useState<RecurringExpenseOccurrence[]>([]);
  const [transactions, setTransactions] = useState<ExpenseTransaction[]>([]);
  const [editing, setEditing] = useState<RecurringExpenseItem | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [form, setForm] = useState<RecurringFormState>(() => recurringForm(today));
  const [registrationCreated, setRegistrationCreated] = useState<RecurringExpenseItem | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    try {
      const transactionMonths = [shiftMonth(month, -1), month, shiftMonth(month, 1)];
      const [nextItems, nextOccurrences, transactionPages] = await Promise.all([
        api.listRecurringExpenses(), api.listRecurringExpenseOccurrences(month),
        Promise.all(transactionMonths.map((transactionMonth) =>
          api.listExpenseTransactions({ month: transactionMonth, limit: 100 }))),
      ]);
      setItems(nextItems);
      setOccurrences(nextOccurrences);
      setTransactions([...new Map(transactionPages
        .flatMap((page) => page.items)
        .map((transaction) => [transaction.id, transaction])).values()]);
    } catch (error) { onNotify(errorText(error), "error"); }
  }, [api, month, onNotify]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (!initialRecurringExpenseId || items.length === 0) return;
    const target = items.find((item) => item.id === initialRecurringExpenseId);
    const element = target ? document.getElementById(`recurring-${target.id}`) : null;
    if (element && typeof element.scrollIntoView === "function") element.scrollIntoView({ block: "center" });
  }, [initialRecurringExpenseId, items]);
  useEffect(() => {
    if (!registrationReview) return;
    setEditing(null);
    setRegistrationCreated(null);
    setForm(recurringFormFromReview(registrationReview));
    setFormOpen(true);
  }, [registrationReview]);

  const openForm = (item: RecurringExpenseItem | null = null) => {
    onRegistrationHandled();
    setRegistrationCreated(null);
    setEditing(item);
    setForm(recurringForm(today, item));
    setFormOpen(true);
  };
  const closeForm = () => {
    if (editing) api.discardExpenseMutation("update_recurring_expense", editing.id);
    else api.discardExpenseMutation("create_recurring_expense");
    if (registrationReview) {
      api.discardExpenseMutation("resolve_expense_review", registrationReview.id);
    }
    setFormOpen(false);
    setEditing(null);
    setRegistrationCreated(null);
    onRegistrationHandled();
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const name = form.name.trim();
    const vendor = form.vendor.trim();
    const memo = form.memo.trim();
    if (name.length > 120 || vendor.length > 200 || memo.length > 500) {
      onNotify("이름 120자, 업체 200자, 메모 500자 이내로 입력해 주세요.", "error");
      return;
    }
    const input: CreateRecurringExpenseInput = {
      name, category: form.category, vendor: vendor || null,
      amountMinor: Number(form.amount), currency: form.currency.toUpperCase(), paymentMethodFingerprint: form.paymentMethodFingerprint.trim() || null,
      startDate: form.startDate, endDate: form.endDate || null, memo: memo || null,
      reminderDays: Number(form.reminderDays), amountKind: form.amountKind,
      intervalMonths: Number(form.intervalMonths) as 1 | 2 | 3 | 6 | 12,
      dueRule: form.dueRule, dueDay: form.dueRule === "specific_day" ? Number(form.dueDay) : null,
      status: form.status,
    };
    setSaving(true);
    try {
      if (registrationReview && !editing) {
        const latestItems = await api.listRecurringExpenses();
        let registered = registrationCreated
          ?? latestItems.find((item) => sameRecurringRegistration(item, registrationReview))
          ?? null;
        if (!registered) {
          registered = await api.createRecurringExpense(input);
        }
        setRegistrationCreated(registered);
        try {
          await api.resolveExpenseReview(registrationReview.id, {
            kind: "purchase",
            category: input.category,
            duplicateOfEventId: null,
            relatedEventId: null,
            personalAmountMinor: null,
            createRule: false,
            expectedVersion: registrationReview.version,
          });
        } catch (error) {
          await load();
          try {
            const pending = await api.listExpenseReviews({ month, status: "pending", limit: 100 });
            if (!pending.items.some((review) => review.id === registrationReview.id)) {
              onNotify("정기지출을 등록하고 반복 거래 후보 검토를 완료했습니다.");
              closeForm();
              return;
            }
          } catch { /* 원래 오류와 재시도 가능한 상태를 보존합니다. */ }
          onNotify(`정기지출 항목은 중복 없이 보존했습니다. 검토 완료만 다시 시도해 주세요: ${errorText(error)}`, "error");
          return;
        }
        onNotify("정기지출을 등록하고 반복 거래 후보 검토를 완료했습니다.");
        closeForm();
        await load();
      } else if (editing) {
        const currentMonth = today.slice(0, 7);
        await api.updateRecurringExpense(editing.id, {
          ...input,
          expectedVersion: editing.version,
          effectiveFromMonth: `${currentMonth}-01`,
          autoMatchEnabled: form.autoMatchEnabled,
        });
        onNotify("정기지출 설정을 변경했습니다.");
        closeForm();
        await load();
      } else {
        await api.createRecurringExpense(input);
        onNotify("정기지출을 등록했습니다.");
        closeForm();
        await load();
      }
    } catch (error) { onNotify(errorText(error), "error"); } finally { setSaving(false); }
  };
  const remove = async (item: RecurringExpenseItem) => {
    try { await api.deleteRecurringExpense(item.id, item.version); onNotify("정기지출을 삭제했습니다."); closeForm(); await load(); }
    catch (error) { onNotify(errorText(error), "error"); }
  };
  const confirmPaid = async (occurrence: RecurringExpenseOccurrence) => {
    const paidDate = occurrence.dueDate < today ? occurrence.dueDate : today;
    try { await api.confirmRecurringExpensePaid(occurrence.occurrenceKey, null, paidDate, occurrence.version); onNotify("납부 완료를 기록했습니다."); await load(); }
    catch (error) { onNotify(errorText(error), "error"); }
  };

  return (
    <div className="expense-section-stack">
      <section className="panel recurring-occurrences">
        <div className="panel__header"><div><span className="eyebrow">{monthTitle(month)}</span><h2>납부 현황</h2><p>예정 금액은 실제 거래가 연결되거나 납부 완료한 뒤에만 지출로 계산됩니다.</p></div><button className="primary-button" onClick={() => openForm()} type="button"><Icon name="plus" size={15} /> 정기지출 추가</button></div>
        <div className="recurring-occurrence-list">
          {occurrences.map((item) => <RecurringOccurrenceCard api={api} item={item} key={item.occurrenceKey} onConfirmPaid={confirmPaid} onNotify={onNotify} onRefresh={load} transactions={transactions} />)}
          {occurrences.length === 0 && <p className="calendar-empty">이 달에 예정된 정기지출이 없습니다.</p>}
        </div>
      </section>
      <section className="panel recurring-items">
        <div className="panel__header"><div><span className="eyebrow">설정</span><h2>정기지출 항목</h2></div><span className="count-pill">{items.length}</span></div>
        <div className="recurring-item-list">
          {items.map((item) => <button className={initialRecurringExpenseId === item.id ? "recurring-item--selected" : ""} id={`recurring-${item.id}`} key={item.id} onClick={() => openForm(item)} type="button"><span><strong>{item.name}</strong><small>{categoryLabels[item.category]} · {item.intervalMonths === 1 ? "매월" : `${item.intervalMonths}개월마다`} · {item.status}</small></span><span>{formatMoney(item.amountMinor, item.currency)}<Icon name="chevron" size={14} /></span></button>)}
        </div>
      </section>
      {formOpen && <section className="panel recurring-form-panel"><div className="panel__header"><div><span className="eyebrow">{registrationReview ? "반복 거래 후보" : editing ? "설정 변경" : "새 항목"}</span><h2>{editing?.name ?? "정기지출 등록"}</h2></div><button aria-label="정기지출 입력 닫기" className="icon-button" onClick={closeForm} type="button"><Icon name="close" /></button></div>{registrationReview && <p className="expense-registration-note">거래의 업체·금액·통화·결제일·결제수단을 내부에서 채웠습니다. 내용을 확인한 뒤 저장하면 후보 검토도 함께 완료됩니다.{registrationCreated ? " 항목 등록은 이미 끝났으며 검토 완료만 다시 시도합니다." : ""}</p>}<form className="recurring-form" onSubmit={submit}>
        <label className="field"><span>이름</span><input maxLength={120} onChange={(event) => setForm({ ...form, name: event.target.value })} required value={form.name} /></label>
        <label className="field"><span>카테고리</span><select onChange={(event) => setForm({ ...form, category: event.target.value as ExpenseCategory })} value={form.category}>{Object.entries(categoryLabels).filter(([value]) => value !== "unconfirmed").map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
        <label className="field"><span>업체 (선택)</span><input maxLength={200} onChange={(event) => setForm({ ...form, vendor: event.target.value })} value={form.vendor} /></label>
        <label className="field"><span>예상 금액 (minor unit)</span><input min="1" onChange={(event) => setForm({ ...form, amount: event.target.value })} required step="1" type="number" value={form.amount} /></label>
        <label className="field"><span>통화</span><input maxLength={3} minLength={3} onChange={(event) => setForm({ ...form, currency: event.target.value })} required value={form.currency} /></label>
        <label className="field"><span>금액 유형</span><select onChange={(event) => setForm({ ...form, amountKind: event.target.value as RecurringAmountKind })} value={form.amountKind}><option value="fixed">고정</option><option value="estimate">예상</option><option value="limit">최대 한도</option></select></label>
        <div className="field expense-payment-learning"><span>결제수단 연결</span><small>{form.paymentMethodFingerprint ? "가져온 거래의 결제수단을 내부 식별값으로 연결합니다." : "첫 실제 거래 연결을 확인하면 업체·결제수단을 안전하게 학습합니다."} 원문 카드·계좌번호는 표시하거나 저장하지 않습니다.</small></div>
        <label className="field"><span>주기</span><select onChange={(event) => setForm({ ...form, intervalMonths: event.target.value as RecurringFormState["intervalMonths"] })} value={form.intervalMonths}>{[1, 2, 3, 6, 12].map((value) => <option key={value} value={value}>{value === 1 ? "매월" : `${value}개월마다`}</option>)}</select></label>
        <label className="field"><span>결제일</span><select onChange={(event) => setForm({ ...form, dueRule: event.target.value as RecurringDueRule })} value={form.dueRule}><option value="specific_day">특정일</option><option value="first_day">초일</option><option value="last_day">말일</option></select></label>
        {form.dueRule === "specific_day" && <label className="field"><span>매월 날짜</span><input max="31" min="1" onChange={(event) => setForm({ ...form, dueDay: event.target.value })} required type="number" value={form.dueDay} /><small>없는 날짜는 그 달 말일로 보정합니다.</small></label>}
        <label className="field"><span>시작일</span><input onChange={(event) => setForm({ ...form, startDate: event.target.value })} required type="date" value={form.startDate} /></label>
        <label className="field"><span>종료일 (선택)</span><input min={form.startDate} onChange={(event) => setForm({ ...form, endDate: event.target.value })} type="date" value={form.endDate} /></label>
        <label className="field"><span>사전 알림일</span><input max="31" min="0" onChange={(event) => setForm({ ...form, reminderDays: event.target.value })} required type="number" value={form.reminderDays} /></label>
        <label className="field"><span>상태</span><select onChange={(event) => setForm({ ...form, status: event.target.value as RecurringExpenseStatus })} value={form.status}><option value="active">활성</option><option value="paused">일시정지</option><option value="ended">종료</option></select></label>
        {editing && <label className="check-field recurring-auto-match-consent"><input checked={form.autoMatchEnabled} disabled={!editing.autoMatchEnabled} onChange={(event) => setForm({ ...form, autoMatchEnabled: event.target.checked })} type="checkbox" /> 이후 고신뢰 거래 자동 연결<small>{editing.autoMatchEnabled ? "체크를 해제해 기존 자동 연결 동의를 철회할 수 있습니다." : "자동 연결은 첫 실제 거래를 직접 연결하며 동의한 경우에만 켤 수 있습니다."}</small></label>}
        <label className="field recurring-form__note"><span>메모 (선택)</span><textarea maxLength={500} onChange={(event) => setForm({ ...form, memo: event.target.value })} rows={3} value={form.memo} /></label>
        <div className="recurring-form__actions">{editing && <button className="danger-button" onClick={() => void remove(editing)} type="button">삭제</button>}<button className="secondary-button" onClick={closeForm} type="button">취소</button><button className="primary-button" disabled={saving} type="submit">{saving ? "저장 중…" : registrationReview ? registrationCreated ? "검토 완료 다시 시도" : "등록하고 검토 완료" : "저장"}</button></div>
      </form></section>}
    </div>
  );
}

function RecurringOccurrenceCard({
  api,
  item,
  onConfirmPaid,
  onNotify,
  onRefresh,
  transactions,
}: {
  api: TmApi;
  item: RecurringExpenseOccurrence;
  onConfirmPaid: (item: RecurringExpenseOccurrence) => Promise<void>;
  onNotify: ExpensesPageProps["onNotify"];
  onRefresh: () => Promise<void>;
  transactions: ExpenseTransaction[];
}) {
  const [matchOpen, setMatchOpen] = useState(false);
  const [eventId, setEventId] = useState("");
  const [enableFutureAutoMatch, setEnableFutureAutoMatch] = useState(false);
  const [matching, setMatching] = useState(false);
  const matched = ["paid", "matched"].includes(item.status);
  const candidates = transactions
    .filter((transaction) => transaction.currency === item.currency
      && transaction.status !== "excluded"
      && !transaction.isProvisional
      && ["purchase", "external_transfer", "unknown_p2p"].includes(transaction.kind)
      && Math.abs(Date.parse(`${transaction.postedDate}T00:00:00Z`) - Date.parse(`${item.dueDate}T00:00:00Z`)) <= 5 * 86_400_000)
    .sort((left, right) => Number(Boolean(right.pendingReviewId)) - Number(Boolean(left.pendingReviewId))
      || left.postedDate.localeCompare(right.postedDate)
      || left.id.localeCompare(right.id));
  const match = async (event: FormEvent) => {
    event.preventDefault();
    setMatching(true);
    try {
      await api.matchRecurringExpenseOccurrence(
        item.occurrenceKey,
        eventId.trim(),
        enableFutureAutoMatch,
        item.version,
      );
      onNotify("실제 거래를 정기지출에 연결했습니다.");
      await onRefresh();
    } catch (error) {
      onNotify(errorText(error), "error");
    } finally {
      setMatching(false);
    }
  };
  return (
    <article>
      <div><span className={`recurring-status recurring-status--${item.status}`}>{occurrenceStatusLabels[item.status]}</span><strong>{item.name}</strong><small>{item.dueDate}{item.amountChanged ? " · 금액 변동" : ""}</small></div>
      <div className="recurring-occurrence-actions"><strong>{formatMoney(item.actualAmountMinor ?? item.expectedAmountMinor, item.currency)}</strong>{!matched && <><button className="secondary-button secondary-button--small" onClick={() => void onConfirmPaid(item)} type="button">납부 완료</button><button className="secondary-button secondary-button--small" onClick={() => setMatchOpen((open) => !open)} type="button">거래 연결</button></>}</div>
      {matchOpen && !matched && <form className="recurring-match-form" onSubmit={match}>
        <label className="field"><span>이 달의 실제 거래</span><select onChange={(event) => setEventId(event.target.value)} required value={eventId}><option value="">연결할 거래를 선택하세요</option>{candidates.map((transaction) => <option key={transaction.id} value={transaction.id}>{transaction.postedDate} · {transaction.merchant ?? transaction.counterparty ?? "표시 이름 없음"} · {formatMoney(transaction.amountMinor, transaction.currency)}{transaction.pendingReviewId ? " · 확인 필요" : ""}</option>)}</select><small>같은 통화의 제외되지 않은 거래만 표시합니다. 최초 연결은 직접 확인해야 합니다.</small></label>
        <label className="check-field"><input checked={enableFutureAutoMatch} onChange={(event) => setEnableFutureAutoMatch(event.target.checked)} type="checkbox" /> 이 연결을 확인했고 이후 고신뢰 거래도 자동 연결</label>
        <small>처음 동의한 연결에서 업체·결제수단을 안전하게 학습합니다.</small>
        <button className="primary-button" disabled={matching || !eventId} type="submit">{matching ? "연결 중…" : "거래 연결 확인"}</button>
      </form>}
    </article>
  );
}

function ExpenseImport({ api, onNotify }: Pick<ExpensesPageProps, "api" | "onNotify">) {
  const [filePath, setFilePath] = useState("");
  const [password, setPassword] = useState("");
  const [preview, setPreview] = useState<ExpenseImportPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const selectFile = async () => {
    const selected = await open({ multiple: false, directory: false, filters: [{ name: "금융 거래내역", extensions: ["xls", "xlsx"] }] });
    if (typeof selected === "string") { setFilePath(selected); setPreview(null); setPassword(""); }
  };
  const runPreview = async (event: FormEvent) => {
    event.preventDefault(); setBusy(true);
    try {
      const next = await api.previewExpenseImport({ path: filePath, password: password || undefined });
      setPreview(next);
      if (next.status === "ready") setPassword("");
      else onNotify("암호가 필요합니다. 입력한 뒤 다시 미리보세요.", "error");
    } catch (error) {
      const message = errorText(error);
      onNotify(message.startsWith("TM_EXPENSE_PASSWORD_RETRY:") ? "암호가 맞지 않습니다. 다시 입력하세요." : message, "error");
    } finally { setBusy(false); }
  };
  const commit = async () => {
    if (!preview?.sessionId) return;
    setBusy(true);
    try {
      const result = await api.commitExpenseImport(preview.sessionId);
      onNotify(`새 거래 ${result.newCount}건을 가져왔습니다.`);
      setPreview(null); setFilePath(""); setPassword("");
    } catch (error) { onNotify(errorText(error), "error"); } finally { setBusy(false); }
  };
  return <div className="expense-section-stack"><section className="panel expense-import"><div className="panel__header"><div><span className="eyebrow">Windows 전용 · 로컬 해석</span><h2>거래내역 가져오기</h2><p>KB 신용카드·KB 계좌/체크카드·카카오페이머니 형식을 자동 판별합니다. 원본 파일과 암호는 Railway로 보내거나 저장하지 않습니다.</p></div></div><form onSubmit={runPreview}>
    <div className="expense-file-picker"><button className="secondary-button" onClick={() => void selectFile()} type="button">XLS/XLSX 선택</button><span>{filePath ? basename(filePath) : "선택한 파일 없음"}</span></div>
    {(preview?.passwordRequired || password) && <label className="field"><span>파일 암호</span><input autoComplete="off" onChange={(event) => setPassword(event.target.value)} type="password" value={password} /><small>암호는 명령행·로그·데이터베이스에 남기지 않습니다.</small></label>}
    <button className="primary-button" disabled={busy || !filePath} type="submit">{busy ? "확인 중…" : "미리보기"}</button>
  </form></section>
  {preview?.status === "ready" && <section className="panel expense-import-preview"><div className="panel__header"><div><span className="eyebrow">10분 · 1회 사용</span><h2>가져오기 미리보기</h2><p>{preview.sourceLabel} · {preview.periodStart}–{preview.periodEnd}</p></div><button className="primary-button" disabled={busy} onClick={() => void commit()} type="button">{busy ? "저장 중…" : "가져오기 확정"}</button></div><div className="expense-import-counts"><span>새 거래 <strong>{preview.counts.new}</strong></span><span>중복 <strong>{preview.counts.duplicate}</strong></span><span>정산 후보 <strong>{preview.counts.settlementCandidate}</strong></span><span>제외 <strong>{preview.counts.excluded}</strong></span><span>미확인 <strong>{preview.counts.unconfirmed}</strong></span><span>거부 <strong>{preview.counts.rejected}</strong></span></div><div className="expense-import-rows">{preview.rows.map((row) => <article key={row.rowNumber}><span>{row.rowNumber}행 · {row.occurredAt.slice(0, 10)}</span><strong>{row.displayName}</strong><span>{formatMoney(row.amountMinor, row.currency)} · {row.excluded ? "제외" : row.needsReview ? "확인 필요" : row.kind}</span></article>)}</div></section>}
  </div>;
}

export function TodayExpenseDueCards({ api, today, onOpenExpenses }: { api: TmApi; today: string; onOpenExpenses: (recurringExpenseId?: string) => void }) {
  const [occurrences, setOccurrences] = useState<RecurringExpenseOccurrence[]>([]);
  const [partialHistory, setPartialHistory] = useState(false);
  useEffect(() => {
    let active = true;
    const month = today.slice(0, 7);
    const months = Array.from({ length: 14 }, (_, index) => shiftMonth(month, index - 12));
    Promise.allSettled(months.map((item) => api.listRecurringExpenseOccurrences(item)))
      .then((groups) => {
        if (!active) return;
        setOccurrences(groups.flatMap((result) => result.status === "fulfilled" ? result.value : []));
        setPartialHistory(groups.some((result) => result.status === "rejected"));
      })
      .catch(() => { if (active) setOccurrences([]); });
    return () => { active = false; };
  }, [api, today]);
  const cutoff = useMemo(() => {
    return shiftCalendarDate(today, 7);
  }, [today]);
  const unpaid = occurrences.filter((item) => !["paid", "matched"].includes(item.status));
  const buckets = [
    { id: "today", label: "오늘 납부", items: unpaid.filter((item) => item.dueDate === today) },
    { id: "soon", label: "7일 이내", items: unpaid.filter((item) => item.dueDate > today && item.dueDate <= cutoff) },
    { id: "overdue", label: "기한 경과", items: unpaid.filter((item) => item.dueDate < today) },
    { id: "changed", label: "금액 변동", items: occurrences.filter((item) => item.amountChanged && item.dueDate.startsWith(`${today.slice(0, 7)}-`)) },
  ];
  if (!buckets.some((bucket) => bucket.items.length > 0)) return null;
  return <section className="panel today-expenses" aria-labelledby="today-expenses-heading"><div className="panel__header"><div><span className="section-kicker section-kicker--attention"><Icon name="calendar" size={14} /> 무료 일정 계산</span><h2 id="today-expenses-heading">정기지출 확인</h2><p>AI 호출이나 푸시 없이 결제일과 납부 상태로만 표시합니다. 지난 12개월 미납과 다음 달까지 확인합니다.{partialHistory ? " 일부 월은 불러오지 못했습니다." : ""}</p></div><button className="secondary-button secondary-button--small" onClick={() => onOpenExpenses()} type="button">전체 보기</button></div><div className="today-expense-grid">{buckets.map((bucket) => <article className={`today-expense-card today-expense-card--${bucket.id}`} key={bucket.id}><span>{bucket.label}</span><strong>{bucket.items.length}</strong>{bucket.items.slice(0, 2).map((item) => <button key={item.occurrenceKey} onClick={() => onOpenExpenses(item.recurringExpenseId)} type="button">{item.name}<small>{item.dueDate} · {formatMoney(item.actualAmountMinor ?? item.expectedAmountMinor, item.currency)}</small></button>)}</article>)}</div></section>;
}
