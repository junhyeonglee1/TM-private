import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type FormEvent,
} from "react";

import { Icon } from "./Icon";
import { StockSearchCombobox } from "./StockSearchCombobox";
import { StockScreenPanel } from "./StockScreen";
import { stockWidgetUrl, tradingViewUrl } from "../lib/tradingview";
import { findUsStockCatalogItem, type StockCatalogItem } from "../lib/stock-catalog";
import type {
  LatestStockScreen,
  ListStockScreenResultsInput,
  StockMarket,
  StockScreenResultPage,
  StockWatchlistItem,
  UpsertStockWatchlistItemInput,
} from "../types";

const DEFAULT_SYMBOL = "NASDAQ:AAPL";
const WATCHLIST_LIMIT = 50;
const MARKETS: Array<{ value: StockMarket; label: string }> = [
  { value: "KRX", label: "한국 KRX" },
  { value: "NASDAQ", label: "미국 NASDAQ" },
  { value: "NYSE", label: "미국 NYSE" },
  { value: "AMEX", label: "미국 AMEX" },
];

interface StockPageProps {
  onDelete: (symbol: string) => Promise<void>;
  onLoad: () => Promise<StockWatchlistItem[]>;
  onNotify: (message: string, type?: "success" | "error") => void;
  onUpsert: (input: UpsertStockWatchlistItemInput) => Promise<StockWatchlistItem>;
  onListScreenResults: (input: ListStockScreenResultsInput) => Promise<StockScreenResultPage>;
  onRefreshScreen: () => Promise<void>;
  screen: LatestStockScreen | null;
  screenError: string | null;
  screenLoading: boolean;
}

export function StockPage({
  onDelete,
  onLoad,
  onNotify,
  onUpsert,
  onListScreenResults,
  onRefreshScreen,
  screen,
  screenError,
  screenLoading,
}: StockPageProps) {
  const [items, setItems] = useState<StockWatchlistItem[]>([]);
  const [selectedSymbol, setSelectedSymbol] = useState(DEFAULT_SYMBOL);
  const [market, setMarket] = useState<StockMarket>("KRX");
  const [candidate, setCandidate] = useState<StockCatalogItem | null>(null);
  const [searchResetKey, setSearchResetKey] = useState(0);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [online, setOnline] = useState(() => navigator.onLine);
  const [loadedChartUrl, setLoadedChartUrl] = useState<string | null>(null);
  const [failedChartUrl, setFailedChartUrl] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await onLoad();
      setItems(next);
      setSelectedSymbol((current) =>
        current === DEFAULT_SYMBOL || next.some((item) => item.symbol === current)
          ? current
          : (next[0]?.symbol ?? DEFAULT_SYMBOL));
    } catch {
      onNotify("관심 종목을 불러오지 못했습니다.", "error");
    } finally {
      setLoading(false);
    }
  }, [onLoad, onNotify]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const update = () => setOnline(navigator.onLine);
    window.addEventListener("online", update);
    window.addEventListener("offline", update);
    return () => {
      window.removeEventListener("online", update);
      window.removeEventListener("offline", update);
    };
  }, []);

  const watchlistSymbols = useMemo(
    () => items.map((item) => item.symbol),
    [items],
  );
  const chartUrl = useMemo(
    () => stockWidgetUrl(selectedSymbol, watchlistSymbols),
    [selectedSymbol, watchlistSymbols],
  );
  const chartLoadState = loadedChartUrl === chartUrl
    ? "ready"
    : failedChartUrl === chartUrl
      ? "failed"
      : "loading";

  useEffect(() => {
    if (!online || loadedChartUrl === chartUrl) return;
    const timeout = window.setTimeout(() => {
      setFailedChartUrl(chartUrl);
    }, 15_000);
    return () => window.clearTimeout(timeout);
  }, [chartUrl, loadedChartUrl, online]);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!candidate || candidate.market !== market) {
      onNotify("검색 결과에서 저장할 종목을 먼저 선택하세요.", "error");
      return;
    }
    if (
      items.length >= WATCHLIST_LIMIT
      && !items.some((item) => item.symbol === `${candidate.market}:${candidate.ticker}`)
    ) {
      onNotify(`관심 종목은 최대 ${WATCHLIST_LIMIT}개까지 저장할 수 있습니다.`, "error");
      return;
    }
    setSaving(true);
    try {
      const saved = await onUpsert({
        market: candidate.market,
        ticker: candidate.ticker,
        displayName: candidate.name,
      });
      setItems((current) => {
        const next = current.filter((item) => item.symbol !== saved.symbol);
        next.push(saved);
        return next.sort((left, right) =>
          left.createdAt.localeCompare(right.createdAt) || left.symbol.localeCompare(right.symbol));
      });
      setSelectedSymbol(saved.symbol);
      setCandidate(null);
      setSearchResetKey((current) => current + 1);
      onNotify(`${saved.displayName} 관심 종목을 저장했습니다.`);
    } catch (error) {
      onNotify(error instanceof Error ? error.message : "관심 종목을 저장하지 못했습니다.", "error");
    } finally {
      setSaving(false);
    }
  };

  const remove = async (item: StockWatchlistItem) => {
    try {
      await onDelete(item.symbol);
      const replacement = items.find((candidate) => candidate.symbol !== item.symbol)?.symbol
        ?? DEFAULT_SYMBOL;
      setItems((current) => current.filter((candidate) => candidate.symbol !== item.symbol));
      if (selectedSymbol === item.symbol) setSelectedSymbol(replacement);
      onNotify(`${item.displayName} 관심 종목을 제거했습니다.`);
    } catch (error) {
      onNotify(error instanceof Error ? error.message : "관심 종목을 제거하지 못했습니다.", "error");
    }
  };

  return (
    <section className="stock-page">
      <header className="page-header stock-page__header">
        <div>
          <span className="eyebrow">조회 전용 · 거래소별 지연 가능</span>
          <h1>주식 차트</h1>
          <p>TradingView가 제공하는 한국·미국 시세를 봅니다. 주문과 계좌 연결은 없습니다.</p>
        </div>
        <a
          className="secondary-button"
          href={tradingViewUrl(selectedSymbol)}
          rel="noopener noreferrer"
          target="_blank"
        >
          TradingView에서 열기 <Icon name="arrow" size={15} />
        </a>
      </header>

      <StockScreenPanel
        error={screenError}
        loading={screenLoading}
        onList={onListScreenResults}
        onRefresh={onRefreshScreen}
        onSelectSymbol={(ticker) => {
          const catalogItem = findUsStockCatalogItem(ticker);
          if (!catalogItem) {
            onNotify(`${ticker}의 거래소를 확인하지 못해 차트를 자동 선택하지 않았습니다.`, "error");
            return;
          }
          setSelectedSymbol(`${catalogItem.market}:${catalogItem.ticker}`);
        }}
        screen={screen}
      />

      <div className="stock-layout">
        <aside className="stock-watchlist" aria-label="관심 종목">
          <div className="stock-watchlist__heading">
            <div><span>관심 종목</span><strong>{items.length}/{WATCHLIST_LIMIT}</strong></div>
            <button className="icon-button icon-button--small" aria-label="관심 종목 새로고침" onClick={() => void load()} type="button">
              <Icon name="restore" size={15} />
            </button>
          </div>
          {loading ? (
            <p className="empty-state">불러오는 중…</p>
          ) : items.length === 0 ? (
            <p className="empty-state">아직 저장한 종목이 없습니다.</p>
          ) : (
            <ul className="stock-watchlist__items">
              {items.map((item) => (
                <li key={item.symbol} className={selectedSymbol === item.symbol ? "selected" : ""}>
                  <button onClick={() => setSelectedSymbol(item.symbol)} type="button">
                    <strong>{item.displayName}</strong>
                    <span>{item.symbol}</span>
                  </button>
                  <button aria-label={`${item.displayName} 관심 종목 삭제`} className="stock-watchlist__delete" onClick={() => void remove(item)} type="button">
                    <Icon name="close" size={13} />
                  </button>
                </li>
              ))}
            </ul>
          )}

          <form className="stock-form" onSubmit={(event) => void submit(event)}>
            <h2>관심 종목 추가</h2>
            <label htmlFor="stock-market">시장</label>
            <select
              id="stock-market"
              onChange={(event) => {
                setMarket(event.target.value as StockMarket);
                setCandidate(null);
                setSearchResetKey((current) => current + 1);
              }}
              value={market}
            >
              {MARKETS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
            </select>
            <StockSearchCombobox
              key={`${market}:${searchResetKey}`}
              market={market}
              onChange={setCandidate}
              selected={candidate}
            />
            <button className="primary-button" disabled={saving || !candidate} type="submit">
              <Icon name="plus" size={15} /> {saving ? "저장 중…" : "관심 종목 저장"}
            </button>
          </form>
        </aside>

        <section className="stock-chart-card" aria-labelledby="stock-chart-title">
          <div className="stock-chart-card__heading">
            <div><span className="eyebrow">외부 데이터 · OpenAI 비용 없음</span><h2 id="stock-chart-title">{selectedSymbol}</h2></div>
            <span className={`stock-network ${online ? "online" : "offline"}`}>{online ? "온라인" : "오프라인"}</span>
          </div>
          {!online ? (
            <div className="stock-chart-placeholder" role="status">
              <Icon name="chart" size={28} />
              <strong>차트를 보려면 인터넷 연결이 필요합니다.</strong>
              <p>관심 종목 목록은 연결 후 다시 동기화됩니다.</p>
            </div>
          ) : (
            <div className="stock-chart-frame-shell">
              {chartLoadState !== "ready" && (
                <div className="stock-chart-loading" role="status">
                  <Icon name="chart" size={28} />
                  {chartLoadState === "failed" ? (
                    <>
                      <strong>차트를 표시하지 못했습니다.</strong>
                      <p>
                        TradingView 연결을 확인한 뒤{" "}
                        <a href={tradingViewUrl(selectedSymbol)} rel="noopener noreferrer" target="_blank">
                          외부 차트에서 확인
                        </a>
                        하세요.
                      </p>
                    </>
                  ) : (
                    <>
                      <strong>시장 차트를 불러오는 중입니다.</strong>
                      <p>TradingView 연결 상태에 따라 몇 초 정도 걸릴 수 있습니다.</p>
                    </>
                  )}
                </div>
              )}
              <iframe
                className="stock-chart-frame"
                data-testid="tradingview-frame"
                key={`${selectedSymbol}:${watchlistSymbols.join(",")}`}
                onLoad={() => {
                  setLoadedChartUrl(chartUrl);
                  setFailedChartUrl(null);
                }}
                referrerPolicy="no-referrer"
                sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox"
                src={chartUrl}
                title={`${selectedSymbol} TradingView 조회 전용 차트`}
              />
            </div>
          )}
          <p className="stock-disclaimer">
            시세는 거래소 정책에 따라 지연되거나 일부 종목이 위젯에서 제한될 수 있습니다. 투자 추천이나 주문 기능을 제공하지 않습니다.
          </p>
          <p className="stock-fallback">
            차트가 비어 있거나 KRX 종목 표시가 제한되면 우회 수집하지 않습니다.{" "}
            <a href={tradingViewUrl(selectedSymbol)} rel="noopener noreferrer" target="_blank">
              TradingView 외부 차트에서 확인
            </a>
          </p>
        </section>
      </div>
    </section>
  );
}
