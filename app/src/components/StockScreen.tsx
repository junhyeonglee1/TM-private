import { useEffect, useMemo, useRef, useState } from "react";

import { Icon } from "./Icon";
import type {
  LatestStockScreen,
  ListStockScreenResultsInput,
  StockScreenDirection,
  StockScreenFilterBand,
  StockScreenHorizon,
  StockScreenResult,
  StockScreenResultPage,
} from "../types";

interface StockScreenCompactCardProps {
  error: string | null;
  loading: boolean;
  onOpen: () => void;
  screen: LatestStockScreen | null;
}

interface StockScreenPanelProps {
  error: string | null;
  loading: boolean;
  onList: (input: ListStockScreenResultsInput) => Promise<StockScreenResultPage>;
  onRefresh: () => Promise<void>;
  onSelectSymbol: (symbol: string) => void;
  screen: LatestStockScreen | null;
}

const aiStatusLabel = (
  ai: NonNullable<NonNullable<LatestStockScreen["latestSuccess"]>["ai"]> | null,
  hardStopReached: boolean,
): string => {
  if (hardStopReached) return "AI 월 예산 도달 · 숫자 결과만 표시";
  if (!ai) return "AI 호출 없음";
  const failureCode = ai.failureCode?.toLocaleLowerCase("en-US") ?? "";
  if (failureCode.includes("budget_blocked")) return "AI 월 예산 도달 · 숫자 결과만 표시";
  if (failureCode.includes("disabled")) return "AI 요약 꺼짐 · 숫자 결과만 표시";
  if (ai.status === "started") return "AI 요약 준비 중";
  if (ai.status === "succeeded") return "AI 요약 완료";
  if (ai.status === "ai_uncertain") return "AI 호출 결과 불명확 · 자동 재시도 안 함";
  return "AI 요약 실패 · 숫자 결과는 정상";
};

const formatReturn = (value: number): string => {
  return `${value > 0 ? "+" : ""}${value.toFixed(2)}%`;
};

const formatPrice = (microusd: number): string =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(microusd / 1_000_000);

const formatDate = (date: string): string =>
  new Intl.DateTimeFormat("ko-KR", {
    month: "long",
    day: "numeric",
    timeZone: "UTC",
  }).format(new Date(`${date}T12:00:00Z`));

const attemptMessage = (screen: LatestStockScreen | null): string | null => {
  const attempt = screen?.latestAttempt;
  if (!attempt) return null;
  switch (attempt.status) {
    case "started":
      return "최신 미국 시장 데이터를 확인하고 있습니다.";
    case "no_candidates":
      return "10% 이상 움직인 종목이 없어 AI를 호출하지 않았습니다.";
    case "upstream_unavailable":
      return "시장 데이터 제공처에 연결하지 못했습니다. 이전 성공 결과가 있으면 그대로 표시합니다.";
    case "coverage_failed":
      return "필수 종목의 98%를 확인하지 못해 이번 결과를 발행하지 않았습니다.";
    case "failed":
      return "이번 분석을 완료하지 못했습니다. 자동 재시도 여부는 운영 상태에서 확인합니다.";
    case "succeeded":
      return null;
  }
};

const stalenessMessage = (reason: string | null): string => {
  if (reason === "first_run_pending") return "첫 시장 데이터 수집과 검증을 기다리고 있습니다.";
  if (reason === "latest_attempt_started") return "최신 미국 시장 데이터를 확인하는 동안 이전 성공 결과를 표시합니다.";
  if (reason === "latest_attempt_succeeded") return "최신 분석은 끝났지만 기대 시장일 확인이 완료되지 않았습니다.";
  if (reason === "latest_attempt_no_candidates") return "최신 시장일에는 10% 이상 움직인 종목이 없어 AI를 호출하지 않았습니다.";
  if (reason === "latest_attempt_coverage_failed") return "필수 종목의 98%를 확인하지 못해 이전 성공 결과를 표시합니다.";
  if (reason === "latest_attempt_upstream_unavailable") return "시장 데이터 제공처에 연결하지 못해 이전 성공 결과를 표시합니다.";
  if (reason === "latest_attempt_failed") return "최신 분석을 완료하지 못해 이전 성공 결과를 표시합니다.";
  if (reason === "latest_success_before_expected_market_date") return "예상된 최신 미국 시장일 결과가 없어 이전 성공 결과를 표시합니다.";
  return "최신 확정 결과가 없어 이전 성공 결과를 표시합니다.";
};

const filterCount = (
  counts: NonNullable<LatestStockScreen["latestSuccess"]>["counts"],
  horizon: StockScreenHorizon,
  direction: StockScreenDirection,
  band: StockScreenFilterBand,
): number => {
  const key = `${direction}${horizon}${band === "ten_to_twenty" ? "TenToTwenty" : "TwentyPlus"}` as keyof typeof counts;
  return counts[key];
};

function ResultRow({
  item,
  onSelectTicker,
}: {
  item: StockScreenResult;
  onSelectTicker: (ticker: string) => void;
}) {
  const movement = item.direction === "up" ? "상승" : "하락";
  return (
    <li>
      <button
        aria-label={`${item.displayName} ${formatReturn(item.returnPct)}, 차트에서 보기`}
        onClick={() => onSelectTicker(item.ticker)}
        type="button"
      >
        <span className={`stock-move stock-move--${item.direction}`}>
          <strong>{item.direction === "up" ? "↑" : "↓"} {movement}</strong>
          <em>{formatReturn(item.returnPct)}</em>
        </span>
        <span className="stock-result__identity">
          <strong>{item.displayName}</strong>
          <small>{item.ticker}{item.sector ? ` · ${item.sector}` : ""} · {item.horizon}거래일</small>
        </span>
        <span className="stock-result__prices">
          <strong>{formatPrice(item.currentCloseMicrousd)}</strong>
          <small>{item.baselineDate} {formatPrice(item.baselineCloseMicrousd)}</small>
        </span>
        <Icon name="chevron" size={15} />
      </button>
    </li>
  );
}

export function StockScreenCompactCard({
  error,
  loading,
  onOpen,
  screen,
}: StockScreenCompactCardProps) {
  const summary = screen?.latestSuccess ?? null;
  const statusMessage = attemptMessage(screen);
  return (
    <section
      className="panel stock-brief"
      aria-labelledby="stock-brief-heading"
      aria-busy={loading}
    >
      <div className="panel__header stock-brief__header">
        <div>
          <span className="section-kicker section-kicker--accent">
            <Icon name="chart" size={14} /> 조회 전용 · 매일 자동 계산
          </span>
          <h2 id="stock-brief-heading">S&amp;P 500 일일 등락</h2>
        </div>
        <button className="secondary-button" onClick={onOpen} type="button">
          전체 보기 <Icon name="chevron" size={14} />
        </button>
      </div>
      {loading && !summary && <p className="stock-screen-message" role="status">최근 결과를 불러오는 중입니다.</p>}
      {error && !summary && <p className="stock-screen-message stock-screen-message--error" role="alert">{error}</p>}
      {!loading && !error && !summary && (
        <p className="stock-screen-message" role="status">
          {screen?.stalenessReason
            ? stalenessMessage(screen.stalenessReason)
            : statusMessage ?? "아직 발행된 분석 결과가 없습니다."}
        </p>
      )}
      {summary && (
        <div className="stock-brief__body">
          <div className="stock-brief__meta">
            <strong>{formatDate(summary.marketDate)} 미국 시장 확정 종가</strong>
            <span>
              {summary.coverage.currentCovered}/{summary.coverage.total} 종목
              {error ? " · 연결 오류 시 저장된 결과" : screen?.stale ? " · 이전 성공 결과" : ""}
            </span>
          </div>
          {error && (
            <p className="stock-screen-notice stock-screen-notice--error" role="alert">
              서버 새로고침에 실패해 마지막으로 불러온 결과를 표시합니다. {error}
            </p>
          )}
          {!error && (screen?.stale || statusMessage) && (
            <p className="stock-screen-notice" role="status">
              {screen?.stale ? stalenessMessage(screen.stalenessReason) : statusMessage}
            </p>
          )}
          {summary.top3.length > 0 ? (
            <ol className="stock-brief__top">
              {summary.top3.slice(0, 3).map((item) => (
                <li key={`${item.ticker}:${item.horizon}:${item.direction}`}>
                  <span>{item.direction === "up" ? "↑ 상승" : "↓ 하락"}</span>
                  <strong>{item.displayName}</strong>
                  <em>{formatReturn(item.returnPct)}</em>
                </li>
              ))}
            </ol>
          ) : (
            <p className="stock-screen-message">10% 이상 움직인 종목이 없습니다.</p>
          )}
          <small className="stock-brief__disclaimer">가격 변화만 계산하며 투자 추천이나 예측이 아닙니다.</small>
        </div>
      )}
    </section>
  );
}

export function StockScreenPanel({
  error,
  loading,
  onList,
  onRefresh,
  onSelectSymbol: onSelectTicker,
  screen,
}: StockScreenPanelProps) {
  const [horizon, setHorizon] = useState<StockScreenHorizon>(5);
  const [direction, setDirection] = useState<StockScreenDirection>("down");
  const [band, setBand] = useState<StockScreenFilterBand>("ten_to_twenty");
  const [items, setItems] = useState<StockScreenResult[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [listLoading, setListLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const requestGeneration = useRef(0);
  const summary = screen?.latestSuccess ?? null;
  const statusMessage = attemptMessage(screen);
  const aiResult = useMemo(() => {
    const result = summary?.ai?.result;
    if (!result || typeof result !== "object") return null;
    const value = result as Record<string, unknown>;
    return {
      headline: typeof value.headline === "string" ? value.headline : null,
      bullets: Array.isArray(value.bullets)
        ? value.bullets.filter((item): item is string => typeof item === "string")
        : [],
    };
  }, [summary?.ai?.result]);
  const count = useMemo(
    () => summary ? filterCount(summary.counts, horizon, direction, band) : 0,
    [band, direction, horizon, summary],
  );

  const loadResults = async (nextCursor?: string, append = false) => {
    if (!summary) return;
    const generation = ++requestGeneration.current;
    setListLoading(true);
    setListError(null);
    try {
      const page = await onList({
        runId: summary.runId,
        horizon,
        direction,
        band,
        cursor: nextCursor,
        limit: 50,
      });
      if (generation !== requestGeneration.current) return;
      setItems((current) => append ? [...current, ...page.items] : page.items);
      setCursor(page.nextCursor);
    } catch (loadError) {
      if (generation !== requestGeneration.current) return;
      setListError(loadError instanceof Error ? loadError.message : "등락 종목을 불러오지 못했습니다.");
      if (!append) {
        setItems([]);
        setCursor(null);
      }
    } finally {
      if (generation === requestGeneration.current) setListLoading(false);
    }
  };

  useEffect(() => {
    setItems([]);
    setCursor(null);
    if (summary) void loadResults();
    return () => {
      requestGeneration.current += 1;
    };
  }, [summary?.runId, horizon, direction, band]);

  return (
    <section className="stock-screen panel" aria-labelledby="stock-screen-heading" aria-busy={loading || listLoading}>
      <div className="panel__header stock-screen__header">
        <div>
          <span className="section-kicker section-kicker--accent">
            <Icon name="chart" size={14} /> 주가 데이터 API 추가비용 $0 · AI 요약 월 $2 제한
          </span>
          <h2 id="stock-screen-heading">S&amp;P 500 일일 등락 스캐너</h2>
          <p>분할 조정 종가를 정확히 5·21거래일 전과 비교합니다.</p>
        </div>
        <button
          className="secondary-button"
          disabled={loading}
          onClick={() => { void onRefresh(); }}
          type="button"
        >
          <Icon name="restore" size={14} /> 새로고침
        </button>
      </div>

      {loading && !summary && <p className="stock-screen-message" role="status">최근 시장 결과를 불러오는 중입니다.</p>}
      {error && !summary && <p className="stock-screen-message stock-screen-message--error" role="alert">{error}</p>}
      {!loading && !error && !summary && (
        <div className="stock-screen-message" role="status">
          <strong>
            {screen?.stalenessReason
              ? stalenessMessage(screen.stalenessReason)
              : statusMessage ?? "첫 결과를 기다리고 있습니다."}
          </strong>
          <span>시장 데이터·AI 기능은 운영 설정이 승인되기 전까지 비용 없이 꺼진 상태일 수 있습니다.</span>
        </div>
      )}

      {summary && (
        <>
          <div className="stock-screen__status">
            <div>
              <span className={(error || screen?.stale) ? "status-badge status-badge--warning" : "status-badge status-badge--success"}>
                {error ? "연결 오류 · 저장된 결과" : screen?.stale ? "이전 성공 결과" : "최신 확정 결과"}
              </span>
              <strong>{formatDate(summary.marketDate)} 미국 시장</strong>
              <small>
                현재 {summary.coverage.currentPct.toFixed(1)}% · 5일 {summary.coverage.baseline5Pct.toFixed(1)}%
                {" · "}21일 {summary.coverage.baseline21Pct.toFixed(1)}% 수집
              </small>
            </div>
            <div>
              <span>{summary.universe.name}</span>
              <strong>{summary.universe.memberCount.toLocaleString("ko-KR")}개 종목</strong>
              <a href={summary.universe.sourceUrl} rel="noopener noreferrer" target="_blank">
                구성종목 출처·기준일 {summary.universe.asOfDate}
              </a>
              <small>{summary.universe.attributionText}</small>
            </div>
          </div>

          {error && (
            <p className="stock-screen-notice stock-screen-notice--error" role="alert">
              서버 새로고침에 실패해 마지막으로 불러온 결과를 표시합니다. {error}
            </p>
          )}
          {!error && (screen?.stale || statusMessage) && (
            <p className="stock-screen-notice" role="status">
              {screen?.stale ? stalenessMessage(screen.stalenessReason) : statusMessage}
            </p>
          )}

          <div className="stock-screen__ai" aria-label="AI 요약 상태">
            <div>
              <span>{aiStatusLabel(summary.ai, screen?.aiBudget.hardStopReached ?? false)}</span>
              {aiResult?.headline && <strong>{aiResult.headline}</strong>}
            </div>
            <small>
              {summary.ai?.model ?? "AI 호출 없음"}
              {" · 이번 요약 $"}
              {(Number(summary.ai?.estimatedCostMicrousd ?? 0) / 1_000_000).toFixed(4)}
              {" · 이번 달 $"}
              {(Number(screen?.aiBudget.committedMicrousd ?? 0) / 1_000_000).toFixed(4)}
              {" / $"}
              {(Number(screen?.aiBudget.hardLimitMicrousd ?? 2_000_000) / 1_000_000).toFixed(0)}
            </small>
            {aiResult?.bullets.length ? (
              <ul>{aiResult.bullets.map((bullet) => <li key={bullet}>{bullet}</li>)}</ul>
            ) : null}
          </div>

          <fieldset className="stock-screen__filters">
            <legend>등락 조건</legend>
            <label>
              기간
              <select value={horizon} onChange={(event) => setHorizon(Number(event.target.value) as StockScreenHorizon)}>
                <option value={5}>5거래일</option>
                <option value={21}>21거래일</option>
              </select>
            </label>
            <label>
              방향
              <select value={direction} onChange={(event) => setDirection(event.target.value as StockScreenDirection)}>
                <option value="down">하락</option>
                <option value="up">상승</option>
              </select>
            </label>
            <label>
              구간
              <select value={band} onChange={(event) => setBand(event.target.value as StockScreenFilterBand)}>
                <option value="ten_to_twenty">10% 이상 20% 미만</option>
                <option value="twenty_plus">20% 이상</option>
              </select>
            </label>
            <strong aria-live="polite">{count.toLocaleString("ko-KR")}개</strong>
          </fieldset>

          <div className="stock-screen__results" aria-atomic="true" aria-live="polite">
            <p className="sr-only">
              {horizon}거래일 {direction === "up" ? "상승" : "하락"}{" "}
              {band === "ten_to_twenty" ? "10% 이상 20% 미만" : "20% 이상"} 결과
              {" "}{items.length.toLocaleString("ko-KR")}개 표시
            </p>
            {listError && <p className="stock-screen-message stock-screen-message--error" role="alert">{listError}</p>}
            {!listError && !listLoading && (count === 0 || items.length === 0) && (
              <p className="stock-screen-message" role="status">
                {count > 0
                  ? "조건 집계는 있지만 서버가 표시할 종목을 반환하지 않았습니다."
                  : "선택한 조건에 해당하는 종목이 없습니다."}
              </p>
            )}
            {!listError && items.length > 0 && (
              <ol>
                {items.map((item) => (
                  <ResultRow
                    item={item}
                    key={`${item.ticker}:${item.horizon}:${item.direction}:${item.band}`}
                    onSelectTicker={onSelectTicker}
                  />
                ))}
              </ol>
            )}
            {listLoading && items.length === 0 && <p className="stock-screen-message" role="status">종목 목록을 불러오는 중입니다.</p>}
            {cursor && (
              <button
                className="secondary-button stock-screen__more"
                disabled={listLoading}
                onClick={() => { void loadResults(cursor, true); }}
                type="button"
              >
                {listLoading ? "불러오는 중…" : "다음 50개"}
              </button>
            )}
          </div>
          <p className="stock-screen__legal">
            S&amp;P 500 구성종목 공개 스냅샷과 지연 시장 데이터를 사용합니다. S&amp;P 또는 데이터 제공사의
            보증·추천을 의미하지 않으며, 배당을 포함한 총수익률이 아닙니다.
          </p>
        </>
      )}
    </section>
  );
}
