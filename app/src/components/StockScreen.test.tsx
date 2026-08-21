import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { StockScreenPanel } from "./StockScreen";
import type {
  LatestStockScreen,
  ListStockScreenResultsInput,
  StockScreenAiStatus,
  StockScreenResult,
  StockScreenResultPage,
} from "../types";

const result = (
  ticker: string,
  displayName: string,
  direction: "up" | "down",
  returnPct: number,
): StockScreenResult => ({
  runId: "run-1",
  ticker,
  displayName,
  sector: null,
  horizon: 5,
  direction,
  band: direction === "up" ? "up_10_to_20" : "down_10_to_20",
  currentDate: "2026-07-24",
  currentCloseMicrousd: 112_000_000,
  baselineDate: "2026-07-17",
  baselineCloseMicrousd: 100_000_000,
  returnMicros: returnPct * 1_000_000,
  returnPct,
  universeSha256: "a".repeat(64),
  marketDataSha256: "b".repeat(64),
});

const latest = (overrides: Partial<LatestStockScreen> = {}): LatestStockScreen => ({
  latestSuccess: {
    runId: "run-1",
    marketDate: "2026-07-24",
    completedAt: "2026-07-25T01:30:00Z",
    coverage: {
      currentCovered: 503,
      total: 503,
      currentPct: 100,
      baseline5Covered: 503,
      baseline5Pct: 100,
      baseline21Covered: 503,
      baseline21Pct: 100,
    },
    universe: {
      name: "S&P 500 구성종목",
      sourceUrl: "https://example.com/universe",
      revision: "rev-1",
      asOfDate: "2026-07-24",
      memberCount: 503,
      ageDays: 1,
      attributionText: "DataHub/Wikipedia 공개 스냅샷",
    },
    counts: {
      up5TenToTwenty: 1,
      up5TwentyPlus: 0,
      down5TenToTwenty: 1,
      down5TwentyPlus: 0,
      up21TenToTwenty: 0,
      up21TwentyPlus: 0,
      down21TenToTwenty: 0,
      down21TwentyPlus: 0,
    },
    top3: [],
    ai: null,
  },
  latestAttempt: {
    runId: "run-1",
    status: "succeeded",
    marketDate: "2026-07-24",
    startedAt: "2026-07-25T01:29:00Z",
    completedAt: "2026-07-25T01:30:00Z",
    failureCode: null,
    coverage: {
      currentCovered: 503,
      total: 503,
      currentPct: 100,
      baseline5Covered: 503,
      baseline5Pct: 100,
      baseline21Covered: 503,
      baseline21Pct: 100,
    },
  },
  aiBudget: {
    budgetMonth: "2026-07",
    operation: "stock_daily_report",
    hardLimitMicrousd: 2_000_000,
    committedMicrousd: 12_500,
    remainingMicrousd: 1_987_500,
    hardStopReached: false,
  },
  stale: false,
  stalenessReason: null,
  ...overrides,
});

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => { resolve = next; });
  return { promise, resolve };
};

const emptyPage = (): StockScreenResultPage => ({ items: [], nextCursor: null, total: 0 });

const withAi = (
  status: StockScreenAiStatus,
  failureCode: string | null,
): LatestStockScreen => {
  const value = latest();
  if (!value.latestSuccess) return value;
  value.latestSuccess.ai = {
    id: "ai-1",
    screenRunId: "run-1",
    status,
    promptVersion: "stock-v1",
    model: "gpt-5.4-nano-2026-03-17",
    responseId: null,
    upstreamRequestId: null,
    requestStartedAt: "2026-07-25T01:30:00Z",
    result: null,
    inputTokens: null,
    cachedInputTokens: null,
    outputTokens: null,
    totalTokens: null,
    estimatedCostMicrousd: 0,
    failureCode,
    createdAt: "2026-07-25T01:30:00Z",
    completedAt: "2026-07-25T01:30:01Z",
  };
  return value;
};

describe("StockScreenPanel", () => {
  it("첫 실행 pending 사유를 기계 코드 대신 안내 문구로 표시한다", async () => {
    render(
      <StockScreenPanel
        error={null}
        loading={false}
        onList={async () => emptyPage()}
        onRefresh={async () => undefined}
        onSelectSymbol={() => undefined}
        screen={latest({
          latestSuccess: null,
          latestAttempt: null,
          stale: true,
          stalenessReason: "first_run_pending",
        })}
      />,
    );
    expect(await screen.findByText("첫 시장 데이터 수집과 검증을 기다리고 있습니다.")).toBeInTheDocument();
    expect(screen.queryByText("first_run_pending")).not.toBeInTheDocument();
  });

  it("새 필터 응답 뒤에 도착한 이전 응답을 무시한다", async () => {
    const user = userEvent.setup();
    const down = deferred<StockScreenResultPage>();
    const up = deferred<StockScreenResultPage>();
    const onList = vi.fn((input: ListStockScreenResultsInput) =>
      input.direction === "up" ? up.promise : down.promise);

    render(
      <StockScreenPanel
        error={null}
        loading={false}
        onList={onList}
        onRefresh={async () => undefined}
        onSelectSymbol={() => undefined}
        screen={latest()}
      />,
    );
    await user.selectOptions(screen.getByLabelText("방향"), "up");
    up.resolve({ items: [result("NVDA", "NVIDIA", "up", 14.3)], nextCursor: null, total: 1 });
    expect(await screen.findByRole("button", { name: /NVIDIA \+14\.30%/ })).toBeInTheDocument();

    down.resolve({ items: [result("F", "Ford", "down", -12)], nextCursor: null, total: 1 });
    await waitFor(() => expect(screen.queryByText("Ford")).not.toBeInTheDocument());
    expect(screen.getByText(/5거래일 상승 10% 이상 20% 미만 결과 1개 표시/)).toBeInTheDocument();
  });

  it("집계가 있어도 빈 페이지면 0건 상태를 명시한다", async () => {
    render(
      <StockScreenPanel
        error={null}
        loading={false}
        onList={async () => emptyPage()}
        onRefresh={async () => undefined}
        onSelectSymbol={() => undefined}
        screen={latest()}
      />,
    );
    expect(await screen.findByText("조건 집계는 있지만 서버가 표시할 종목을 반환하지 않았습니다.")).toBeInTheDocument();
  });

  it("opaque cursor로 다음 페이지를 이어 붙인다", async () => {
    const user = userEvent.setup();
    const onList = vi.fn()
      .mockResolvedValueOnce({
        items: [result("F", "Ford", "down", -12)],
        nextCursor: "opaque-2",
        total: 2,
      })
      .mockResolvedValueOnce({
        items: [result("GM", "General Motors", "down", -11)],
        nextCursor: null,
        total: 2,
      });
    render(
      <StockScreenPanel
        error={null}
        loading={false}
        onList={onList}
        onRefresh={async () => undefined}
        onSelectSymbol={() => undefined}
        screen={latest()}
      />,
    );
    await screen.findByText("Ford");
    await user.click(screen.getByRole("button", { name: "다음 50개" }));
    expect(await screen.findByText("General Motors")).toBeInTheDocument();
    expect(onList).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: "opaque-2", limit: 50 }));
  });

  it("새로고침 오류·stale 사유·AI 예산 중단을 구분한다", async () => {
    const budgetStopped = latest({
      aiBudget: {
        ...latest().aiBudget,
        hardStopReached: true,
        committedMicrousd: 2_000_000,
        remainingMicrousd: 0,
      },
    });
    const view = render(
      <StockScreenPanel
        error="network unavailable"
        loading={false}
        onList={async () => emptyPage()}
        onRefresh={async () => undefined}
        onSelectSymbol={() => undefined}
        screen={budgetStopped}
      />,
    );
    expect(await screen.findByText("연결 오류 · 저장된 결과")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("마지막으로 불러온 결과");
    expect(screen.getByText("AI 월 예산 도달 · 숫자 결과만 표시")).toBeInTheDocument();
    expect(screen.getByText(/이번 달 \$2\.0000 \/ \$2/)).toBeInTheDocument();
    expect(screen.getByText("DataHub/Wikipedia 공개 스냅샷")).toBeInTheDocument();

    view.rerender(
      <StockScreenPanel
        error={null}
        loading={false}
        onList={async () => emptyPage()}
        onRefresh={async () => undefined}
        onSelectSymbol={() => undefined}
        screen={latest({ stale: true, stalenessReason: "latest_attempt_coverage_failed" })}
      />,
    );
    expect(await screen.findByText("필수 종목의 98%를 확인하지 못해 이전 성공 결과를 표시합니다.")).toBeInTheDocument();
  });

  it.each([
    ["failed", null, "AI 요약 실패 · 숫자 결과는 정상"],
    ["ai_uncertain", null, "AI 호출 결과 불명확 · 자동 재시도 안 함"],
    ["failed", "disabled", "AI 요약 꺼짐 · 숫자 결과만 표시"],
  ] as const)("AI 상태 %s/%s를 사용자 상태로 표시한다", async (status, failureCode, expected) => {
    render(
      <StockScreenPanel
        error={null}
        loading={false}
        onList={async () => emptyPage()}
        onRefresh={async () => undefined}
        onSelectSymbol={() => undefined}
        screen={withAi(status, failureCode)}
      />,
    );
    expect(await screen.findByText(expected)).toBeInTheDocument();
  });
});
