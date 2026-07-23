import catalogJson from "../../assets/stock-catalog.json?raw";

import type { StockMarket } from "../types";

export interface StockCatalogItem {
  market: StockMarket;
  ticker: string;
  name: string;
}

interface StockCatalog {
  version: number;
  generatedAt: string;
  counts: Record<StockMarket, number> & { total: number };
  items: StockCatalogItem[];
}

interface SearchableStockCatalogItem extends StockCatalogItem {
  normalizedName: string;
  normalizedTicker: string;
  words: string[];
}

const catalog = JSON.parse(catalogJson) as StockCatalog;

const normalize = (value: string): string =>
  value.normalize("NFKC").trim().toLocaleLowerCase("ko-KR");

const searchableByMarket = new Map<StockMarket, SearchableStockCatalogItem[]>();
for (const item of catalog.items) {
  const searchable = {
    ...item,
    normalizedName: normalize(item.name),
    normalizedTicker: normalize(item.ticker),
    words: normalize(item.name).split(/[\s,.()/-]+/u).filter(Boolean),
  };
  const existing = searchableByMarket.get(item.market);
  if (existing) existing.push(searchable);
  else searchableByMarket.set(item.market, [searchable]);
}

const score = (item: SearchableStockCatalogItem, query: string): number => {
  if (item.normalizedName === query || item.normalizedTicker === query) return 0;
  if (item.normalizedName.startsWith(query)) return 1;
  if (item.normalizedTicker.startsWith(query)) return 2;
  if (item.words.some((word) => word.startsWith(query))) return 3;
  return 4;
};

export const searchStockCatalog = (
  market: StockMarket,
  value: string,
  limit = 8,
): StockCatalogItem[] => {
  const query = normalize(value);
  if (!query) return [];
  return (searchableByMarket.get(market) ?? [])
    .filter((item) =>
      item.normalizedName.includes(query) || item.normalizedTicker.includes(query))
    .sort((left, right) =>
      score(left, query) - score(right, query)
      || left.name.localeCompare(right.name, "ko")
      || left.ticker.localeCompare(right.ticker))
    .slice(0, limit)
    .map(({ market: itemMarket, ticker, name }) => ({
      market: itemMarket,
      ticker,
      name,
    }));
};

export const stockCatalogCount = (market: StockMarket): number =>
  catalog.counts[market];

export const stockCatalogGeneratedAt = catalog.generatedAt;
