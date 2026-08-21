import {
  useId,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FocusEvent,
  type KeyboardEvent,
} from "react";

import {
  searchStockCatalog,
  stockCatalogCount,
  type StockCatalogItem,
} from "../lib/stock-catalog";
import type { StockMarket } from "../types";

interface StockSearchComboboxProps {
  market: StockMarket;
  onChange: (item: StockCatalogItem | null) => void;
  selected: StockCatalogItem | null;
}

export function StockSearchCombobox({
  market,
  onChange,
  selected,
}: StockSearchComboboxProps) {
  const listboxId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const results = useMemo(
    () => searchStockCatalog(market, query),
    [market, query],
  );

  const choose = (item: StockCatalogItem) => {
    setQuery(item.name);
    setOpen(false);
    setActiveIndex(0);
    onChange(item);
  };

  const updateQuery = (event: ChangeEvent<HTMLInputElement>) => {
    setQuery(event.target.value);
    setOpen(true);
    setActiveIndex(0);
    onChange(null);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      setOpen(false);
      return;
    }
    if (!results.length) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setOpen(true);
      setActiveIndex((current) => (current + 1) % results.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setOpen(true);
      setActiveIndex((current) => (current - 1 + results.length) % results.length);
    } else if (event.key === "Enter" && open) {
      event.preventDefault();
      choose(results[activeIndex] ?? results[0]);
    }
  };

  const handleBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (!rootRef.current?.contains(event.relatedTarget as Node | null)) {
      setOpen(false);
    }
  };

  return (
    <div
      className="stock-search"
      onBlur={handleBlur}
      onFocus={() => setOpen(true)}
      ref={rootRef}
    >
      <label htmlFor="stock-company-search">회사명 또는 종목코드</label>
      <div className="stock-search__control">
        <input
          aria-activedescendant={
            open && results.length ? `${listboxId}-${activeIndex}` : undefined
          }
          aria-autocomplete="list"
          aria-controls={listboxId}
          aria-expanded={open && Boolean(query.trim())}
          autoComplete="off"
          id="stock-company-search"
          onChange={updateQuery}
          onKeyDown={handleKeyDown}
          placeholder={market === "KRX" ? "예: 삼성전자" : "예: Apple 또는 AAPL"}
          role="combobox"
          value={query}
        />
        {open && query.trim() ? (
          <div
            aria-label={`${market} 종목 검색 결과`}
            className="stock-search__results"
            id={listboxId}
            role="listbox"
          >
            {results.length ? results.map((item, index) => (
              <button
                aria-selected={index === activeIndex}
                className={index === activeIndex ? "active" : ""}
                id={`${listboxId}-${index}`}
                key={`${item.market}:${item.ticker}`}
                onClick={() => choose(item)}
                onMouseDown={(event) => event.preventDefault()}
                role="option"
                type="button"
              >
                <strong>{item.name}</strong>
                <span>{item.market}:{item.ticker}</span>
              </button>
            )) : (
              <p>일치하는 상장 종목이 없습니다.</p>
            )}
          </div>
        ) : null}
      </div>
      <small>{market} 상장 종목 {stockCatalogCount(market).toLocaleString("ko-KR")}개에서 검색</small>
      {selected ? (
        <p className="stock-search__selection">
          <strong>{selected.name}</strong>
          <span>{selected.market}:{selected.ticker}</span>
        </p>
      ) : null}
    </div>
  );
}
