import { readFileSync } from "node:fs";
import { join } from "node:path";
import process from "node:process";

const pwaSource = (name: string) =>
  readFileSync(join(process.cwd(), "crates", "tm-server", "src", "pwa", name), "utf8")
    .replaceAll("\r\n", "\n");

describe("모바일 PWA 지출", () => {
  const app = pwaSource("app.js");
  const shell = pwaSource("index.html");

  it("파일 가져오기 없이 조회·검토·정기지출 UI만 노출한다", () => {
    expect(shell).toContain('data-tab="expenses"');
    expect(shell).toContain('id="expense-summary"');
    expect(shell).toContain('id="expense-recurring-amount" type="number" min="1"');
    expect(shell).not.toContain('id="expense-import"');
    expect(app).not.toContain('"/api/v1/expenses/imports"');
  });

  it("AI 해설은 명시 호출·동일 요청 재확인·새 과금 요청을 구분한다", () => {
    expect(app).toContain('generate.addEventListener("click", () => void generateMobileExpenseReport(generate, false))');
    expect(app).toContain('newRequest.addEventListener("click", () => void generateMobileExpenseReport(newRequest, true))');
    expect(app).toContain('discardExpenseMutation("expense-report-generate", state.expenseMonth)');
    expect(app).toContain("같은 요청 다시 확인은 새 횟수를 사용하지 않습니다.");
    expect(app).toContain('aiConfirmation: "expense-report"');
    expect(app).toContain("수동 호출만 · 업체·상대방·개별 거래 미전송");
    expect(app).toContain("요청당 최대 $0.05 예약 · 서울 기준 월 8회 · 지출 AI 월 $1 hard stop");
    expect(app).toContain("expenseFactCitations(report, item.factIds)");
    expect(app).toContain("formatExpenseMoney(fact.amountMinor, fact.currency)");
    expect(app).not.toContain('item.factIds.join(" · ")');
  });

  it("출처 CAS와 거래 override를 전용 mutation으로 저장하고 안전한 처리만 허용한다", () => {
    expect(app).toContain('api("/api/v1/expenses/sources")');
    expect(app).toContain('method: "PATCH",\n          operation: "expense-source-update",\n          version: source.version');
    expect(app).toContain('/api/v1/expenses/transactions/${encodeURIComponent(item.id)}');
    expect(app).toContain('operation: "expense-transaction-override"');
    expect(app).toContain("clearPersonalAmount: clearPersonal.checked");
    expect(app).toContain("clearRelatedEvent: clearRelated.checked");
    expect(app).toContain('!["external_transfer", "unknown_p2p", "manual_recurring"].includes(value)');
    expect(app).toContain("createRule: item.merchant ? createRule.checked : false");
    expect(app).toContain("createRule: transaction.merchant ? createRule.checked : false");
    expect(app).toContain('status: "pending"');
  });

  it("지출 mutation은 실패 재시도에 같은 키를 쓰고 성공 또는 명시 취소 뒤 교체한다", async () => {
    const helperSource = app.match(/function mutationHeaders[\s\S]*?(?=function activeProject)/)?.[0];
    expect(helperSource).toBeTruthy();

    const calls: Array<{ options: { headers: Record<string, string> } }> = [];
    let fail = true;
    let sequence = 0;
    const api = async (_path: string, options: { headers: Record<string, string> }) => {
      calls.push({ options });
      if (fail) throw new Error("response lost");
      return { ok: true };
    };
    const crypto = { randomUUID: () => `key-${++sequence}` };
    const helpers = Function(
      "api",
      "crypto",
      `"use strict"; const pendingExpenseMutationKeys = new Map(); ${helperSource}; return { expenseMutation, discardExpenseMutation };`,
    )(api, crypto) as {
      expenseMutation: (path: string, input: Record<string, unknown>) => Promise<unknown>;
      discardExpenseMutation: (operation: string, resource: string) => void;
    };
    const request = { operation: "expense-test", resource: "event-1", body: { b: 2, a: 1 } };

    await expect(helpers.expenseMutation("/expense", request)).rejects.toThrow("response lost");
    const firstKey = calls[0].options.headers["idempotency-key"];
    fail = false;
    await helpers.expenseMutation("/expense", { ...request, body: { a: 1, b: 2 } });
    expect(calls[1].options.headers["idempotency-key"]).toBe(firstKey);

    await helpers.expenseMutation("/expense", request);
    expect(calls[2].options.headers["idempotency-key"]).not.toBe(firstKey);

    fail = true;
    await expect(helpers.expenseMutation("/expense", request)).rejects.toThrow("response lost");
    const retryKey = calls[3].options.headers["idempotency-key"];
    helpers.discardExpenseMutation("expense-test", "event-1");
    fail = false;
    await helpers.expenseMutation("/expense", request);
    expect(calls[4].options.headers["idempotency-key"]).not.toBe(retryKey);
  });

  it("정기지출 최초 연결과 이후 자동 연결 동의를 별도 값으로 전송한다", () => {
    expect(app).toContain("const body = { eventId, enableFutureAutoMatch }");
    expect(app).toContain("autoMatch.checked = false");
    expect(app).toContain('operation: "recurring-expense-match"');
    expect(app).toContain("version: occurrence.version");
    expect(app).toContain("matchRecurringOccurrence(occurrence, transaction.id, autoMatch.checked, connect");
    expect(app).toContain("처음 동의한 연결에서 업체·결제수단을 안전하게 학습합니다.");
    expect(app).toContain('byId("expense-recurring-auto-match").disabled = !item?.autoMatchEnabled');
    expect(app).toContain('body.autoMatchEnabled = byId("expense-recurring-auto-match").checked');
    expect(app).toContain('body.effectiveFromMonth = `${currentMonth}-01`');
    expect(app).not.toContain("state.expenseMonth < currentMonth ? currentMonth : state.expenseMonth");
    expect(app).toContain("paidDate: occurrence.dueDate < today ? occurrence.dueDate : today");
  });

  it("정기지출 연결 후보는 앞뒤 달을 조회해 날짜·종류·확정 상태로 제한한다", () => {
    expect(app).toContain("const transactionMonths = [shiftMonth(month, -1), month, shiftMonth(month, 1)]");
    expect(app).toContain("Promise.all(transactionMonths.map");
    expect(app).toContain("state.recurringMatchTransactions = [...new Map(recurringTransactionPages");
    expect(app).toContain("state.recurringMatchTransactions\n        .filter");
    expect(app).toContain('item.status !== "excluded"');
    expect(app).toContain("!item.isProvisional");
    expect(app).toContain('["purchase", "external_transfer", "unknown_p2p"].includes(item.kind)');
    expect(app).toContain("<= 5 * 86_400_000");
  });

  it("일반 검토·재분류도 앞뒤 달의 서버 허용 후보와 필드만 활성화한다", () => {
    expect(app).toContain("const relatedCandidates = state.recurringMatchTransactions.filter");
    expect(app).toContain("const duplicateCandidates = state.recurringMatchTransactions.filter");
    expect(app).toContain("const candidates = state.recurringMatchTransactions.filter");
    expect(app).toContain('candidate.status === "confirmed"');
    expect(app).toContain('item.status === "confirmed"');
    expect(app).toContain('["purchase", "refund"].includes(candidate.kind)');
    expect(app).toContain('["purchase", "refund"].includes(item.kind)');
    expect(app).toContain('related.disabled = !settlementKind || Boolean(duplicate.value) || clearRelated.checked');
    expect(app).toContain('personal.disabled = kind.value !== "purchase" || Boolean(duplicate.value) || clearPersonal.checked');
    expect(app).toContain('relatedEvent.disabled = duplicated || !["settlement_received", "settlement_sent"].includes(kind.value)');
    expect(app).toContain('personalAmount.disabled = duplicated || kind.value !== "purchase"');
  });

  it("모든 지출 쓰기는 안정적인 전용 helper와 정확한 확인 헤더를 사용한다", () => {
    expect(app).toContain("const pendingExpenseMutationKeys = new Map()");
    expect(app).toContain("function expenseMutationFingerprint(value)");
    expect(app).toContain('headers["x-tm-confirm-ai-call"] = aiConfirmation');
    expect(app).not.toMatch(/mutationHeaders\("(?:expense|recurring-expense)/);
    [
      "expense-source-update",
      "expense-transaction-override",
      "expense-review-resolve",
      "expense-report-generate",
      "expense-report-feedback",
      "recurring-expense-delete",
      "recurring-expense-confirm-paid",
      "recurring-expense-match",
    ].forEach((operation) => expect(app).toContain(`operation: "${operation}"`));
    expect(app).toContain('operation: item ? "recurring-expense-update" : "recurring-expense-create"');
  });

  it("중복 제안과 정기지출 등록 후보를 안전한 전용 흐름으로 처리한다", () => {
    expect(app).toContain("review.suggestedDuplicateOfEventId");
    expect(app).toContain("duplicateOfEventId: duplicateEvent?.value || null");
    expect(app).toContain("openRecurringExpenseForm(null, review)");
    expect(app).toContain("sameRecurringExpenseInput(candidate, body)");
    expect(app).toContain("정기지출 항목은 등록되었습니다. 후보 검토 완료만 다시 시도하면 중복 항목을 만들지 않습니다.");
    expect(shell).toContain('id="expense-recurring-payment-method" type="hidden"');
    expect(shell).not.toContain("결제수단 fingerprint");
  });

  it("검토 큐를 pending 상태와 cursor로 끝까지 이어 불러온다", () => {
    expect(shell).toContain('id="expense-reviews-more"');
    expect(app).toContain("expenseReviewsNextCursor: null");
    expect(app).toContain('status: "pending"');
    expect(app).toContain("cursor: state.expenseReviewsNextCursor");
    expect(app).toContain("const merged = new Map(state.expenseReviews.map");
    expect(app).toContain('byId("expense-reviews-more").addEventListener');
  });

  it("Today는 지난 12개월 미납·다음 달과 현재 월 금액 변동만 조회한다", () => {
    expect(app).toContain("Array.from({ length: 14 }, (_, index) => shiftMonth(month, index - 12))");
    expect(app).toContain("Promise.allSettled(months.map");
    expect(app).toContain('item.amountChanged && item.dueDate.startsWith(`${month}-`)');
    expect(app).toContain("지난 12개월 미납과 다음 달까지 확인합니다.");
  });

  it("서울 달력 날짜의 7일 경계를 시간대 변환 없이 정확히 계산한다", () => {
    const source = app.match(/function shiftSeoulDate\(date, days\) \{[\s\S]*?^\}/m)?.[0];
    expect(source).toBeTruthy();
    const shiftSeoulDate = Function(`${source}; return shiftSeoulDate;`)() as (date: string, days: number) => string;
    expect(shiftSeoulDate("2026-08-01", 7)).toBe("2026-08-08");
    expect(shiftSeoulDate("2026-01-27", 7)).toBe("2026-02-03");
  });
});
