import {
  searchStockCatalog,
  stockCatalogCount,
} from "./stock-catalog";

describe("stock catalog", () => {
  it("시장 선택을 유지하며 회사명과 종목코드로 검색한다", () => {
    expect(searchStockCatalog("KRX", "삼성전자")[0]).toEqual({
      market: "KRX",
      ticker: "005930",
      name: "삼성전자",
    });
    expect(searchStockCatalog("NASDAQ", "AAPL")[0]).toEqual({
      market: "NASDAQ",
      ticker: "AAPL",
      name: "Apple Inc.",
    });
    expect(searchStockCatalog("NASDAQ", "삼성전자")).toEqual([]);
  });

  it("전체 카탈로그를 화면에 쏟지 않고 최대 8개 후보만 반환한다", () => {
    expect(stockCatalogCount("KRX")).toBeGreaterThan(2_000);
    expect(stockCatalogCount("NASDAQ")).toBeGreaterThan(4_000);
    expect(searchStockCatalog("NASDAQ", "a")).toHaveLength(8);
  });
});
