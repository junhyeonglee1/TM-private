import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { App } from "../App";
import { createApi } from "../lib/api";
import type { CommandTransport } from "../lib/api";
import { createMemoryTransport } from "../lib/mock-transport";
import type { ExpenseReview, ExpenseTransaction, ResolveExpenseReviewInput } from "../types";

const renderApp = () => {
  const api = createApi(createMemoryTransport());
  return { api, ...render(<App api={api} />) };
};

const openExpenses = async (user: ReturnType<typeof userEvent.setup>) => {
  await screen.findByRole("heading", { name: "오늘", level: 1 });
  await user.click(screen.getAllByRole("button", { name: "지출" })[0]);
  return screen.findByRole("heading", { name: "지출", level: 1 });
};

const transportWithReview = (
  base: CommandTransport,
  review: ExpenseReview,
  transactions: ExpenseTransaction[],
  intercept?: (
    command: string,
    args: Record<string, unknown>,
  ) => Promise<{ handled: true; value: unknown } | undefined> | { handled: true; value: unknown } | undefined,
): CommandTransport => ({
  async invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    const override = await intercept?.(command, args);
    if (override?.handled) return override.value as T;
    if (command === "list_expense_reviews") return { items: [review], nextCursor: null } as T;
    if (command === "list_expense_transactions") return { items: transactions, nextCursor: null } as T;
    return base.invoke<T>(command, args);
  },
});

describe("지출·정기지출 UI", () => {
  it("Today에서 날짜만으로 오늘·7일·기한 경과를 분류한다", async () => {
    renderApp();

    const section = (await screen.findByRole("heading", { name: "정기지출 확인" })).closest("section");
    expect(section).not.toBeNull();
    expect(within(section as HTMLElement).getByText("오늘 납부")).toBeInTheDocument();
    expect(within(section as HTMLElement).getByText("7일 이내")).toBeInTheDocument();
    expect(within(section as HTMLElement).getByText("기한 경과")).toBeInTheDocument();
    expect(within(section as HTMLElement).getByText("보험료")).toBeInTheDocument();
    expect(within(section as HTMLElement).getByText("Railway")).toBeInTheDocument();
  });

  it("월간 순 개인지출을 통화별로 분리하고 미확인을 표시한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await openExpenses(user);

    expect(await screen.findByLabelText("KRW 지출 요약")).toHaveTextContent("순 개인지출");
    expect(screen.getByLabelText("USD 지출 요약")).toHaveTextContent("US$");
    expect(screen.getByText("잠정")).toBeInTheDocument();
    expect(screen.getByText(/미확인 1건/)).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "KRW 일별 순지출 막대 차트" })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "USD 일별 순지출 막대 차트" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "가져오기" })).not.toBeInTheDocument();
  });

  it("AI 해설은 명시적으로 생성하고 도움됨 피드백을 저장한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await openExpenses(user);

    await user.click(await screen.findByRole("button", { name: "AI 해설 생성" }));
    const reportResult = (await screen.findByRole("heading", { name: /월 지출 해설/ }))
      .closest(".expense-report-result");
    expect(reportResult).not.toBeNull();
    expect(within(reportResult as HTMLElement).getByText("순 개인지출 ₩14,500")).toBeInTheDocument();
    expect(within(reportResult as HTMLElement).queryByText("currency:KRW:net_personal_spend")).not.toBeInTheDocument();
    const helpful = screen.getByRole("button", { name: "도움됨" });
    expect(helpful).toHaveAttribute("aria-pressed", "false");
    await user.click(helpful);
    await waitFor(() => expect(helpful).toHaveAttribute("aria-pressed", "true"));
    const month = (await api.getSnapshot()).today.slice(0, 7);
    expect((await api.latestExpenseReport(month))?.helpful).toBe(true);
  });

  it("미확인 P2P를 사용자 결정 후 확정 큐에서 제거한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "확인 필요" }));

    expect(await screen.findByText("카카오페이 송금 상대")).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("처리"), "settlement_sent");
    await user.selectOptions(screen.getByLabelText("카테고리"), "transfer_settlement");
    await user.click(screen.getByRole("button", { name: "결정 저장" }));

    expect(await screen.findByText("확인할 거래가 없습니다")).toBeInTheDocument();
    expect((await api.listExpenseReviews({ limit: 10 })).items).toHaveLength(0);
  });

  it("중복 후보의 서버 제안을 선택해 실제 결정값으로 전송한다", async () => {
    const user = userEvent.setup();
    const base = createMemoryTransport();
    const baseApi = createApi(base);
    const month = (await baseApi.getSnapshot()).today.slice(0, 7);
    const source = (await baseApi.listExpenseTransactions({ month, limit: 100 })).items[0];
    const original = { ...source, id: "expense-original" };
    const duplicate = {
      ...source,
      id: "expense-duplicate",
      status: "unconfirmed" as const,
      pendingReviewId: "review-duplicate",
    };
    const review: ExpenseReview = {
      id: "review-duplicate",
      reason: "ambiguous_mirror",
      status: "pending",
      transaction: duplicate,
      recurringExpenseId: null,
      suggestedKind: "purchase",
      suggestedCategory: "food",
      suggestedDuplicateOfEventId: original.id,
      createdAt: duplicate.occurredAt,
      resolvedAt: null,
      version: 3,
    };
    let resolved: ResolveExpenseReviewInput | null = null;
    const transport = transportWithReview(base, review, [duplicate, original], (command, args) => {
      if (command !== "resolve_expense_review") return undefined;
      resolved = args.input as ResolveExpenseReviewInput;
      return { handled: true, value: null };
    });
    render(<App api={createApi(transport)} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "확인 필요" }));

    const duplicateSelect = await screen.findByRole("combobox", { name: /중복 대상/ });
    expect(duplicateSelect).toHaveValue(original.id);
    await user.click(screen.getByRole("button", { name: "결정 저장" }));

    await waitFor(() => expect(resolved?.duplicateOfEventId).toBe(original.id));
  });

  it("100건 이후 검토도 cursor로 이어 불러온다", async () => {
    const user = userEvent.setup();
    const base = createMemoryTransport();
    const baseApi = createApi(base);
    const month = (await baseApi.getSnapshot()).today.slice(0, 7);
    const source = (await baseApi.listExpenseTransactions({ month, limit: 100 })).items[0];
    const makeReview = (id: string, merchant: string): ExpenseReview => {
      const transaction: ExpenseTransaction = {
        ...source,
        id: `expense-${id}`,
        merchant,
        status: "unconfirmed",
        pendingReviewId: id,
      };
      return {
        id,
        reason: "category_confirmation",
        status: "pending",
        transaction,
        recurringExpenseId: null,
        suggestedKind: "purchase",
        suggestedCategory: "food",
        suggestedDuplicateOfEventId: null,
        createdAt: transaction.occurredAt,
        resolvedAt: null,
        version: 1,
      };
    };
    const first = makeReview("review-first-page", "첫 페이지 거래");
    const second = makeReview("review-second-page", "다음 페이지 거래");
    const requestedCursors: Array<string | undefined> = [];
    const requestedStatuses: Array<string | undefined> = [];
    const transport: CommandTransport = {
      async invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
        if (command === "list_expense_reviews") {
          const input = args.input as { cursor?: string; status?: string };
          const cursor = input.cursor;
          requestedCursors.push(cursor);
          requestedStatuses.push(input.status);
          return (cursor === "reviews-next"
            ? { items: [second], nextCursor: null }
            : { items: [first], nextCursor: "reviews-next" }) as T;
        }
        if (command === "list_expense_transactions") {
          return { items: [first.transaction, second.transaction], nextCursor: null } as T;
        }
        return base.invoke<T>(command, args);
      },
    };
    render(<App api={createApi(transport)} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "확인 필요" }));

    expect(await screen.findByText("첫 페이지 거래")).toBeInTheDocument();
    expect(screen.queryByText("다음 페이지 거래")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "검토 더 보기" }));

    expect(await screen.findByText("다음 페이지 거래")).toBeInTheDocument();
    expect(screen.getByText("첫 페이지 거래")).toBeInTheDocument();
    expect(requestedCursors).toEqual([undefined, "reviews-next"]);
    expect(requestedStatuses).toEqual(["pending", "pending"]);
    expect(screen.queryByRole("button", { name: "검토 더 보기" })).not.toBeInTheDocument();
  });

  it("정기지출 연결 후보는 발생 건 버전과 별도 자동 연결 동의를 사용한다", async () => {
    const user = userEvent.setup();
    const base = createMemoryTransport();
    const baseApi = createApi(base);
    const month = (await baseApi.getSnapshot()).today.slice(0, 7);
    const transactions = (await baseApi.listExpenseTransactions({ month, limit: 100 })).items;
    const transaction = { ...transactions[0], pendingReviewId: "review-recurring-match" };
    const occurrence = (await baseApi.listRecurringExpenseOccurrences(month))
      .find((item) => item.recurringExpenseId === "recurring-insurance");
    expect(occurrence).toBeDefined();
    const review: ExpenseReview = {
      id: "review-recurring-match",
      reason: "recurring_match_candidate",
      status: "pending",
      transaction,
      recurringExpenseId: "recurring-insurance",
      suggestedKind: "purchase",
      suggestedCategory: "insurance_finance_tax",
      suggestedDuplicateOfEventId: null,
      createdAt: transaction.occurredAt,
      resolvedAt: null,
      version: 1,
    };
    let matchArgs: Record<string, unknown> | null = null;
    const transport = transportWithReview(base, review, transactions, (command, args) => {
      if (command === "match_recurring_expense_occurrence") matchArgs = args;
      return undefined;
    });
    render(<App api={createApi(transport)} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "확인 필요" }));

    const autoMatch = await screen.findByRole("checkbox", { name: /앞으로 고신뢰 거래도 자동 연결/ });
    expect(autoMatch).not.toBeChecked();
    await user.click(autoMatch);
    await user.click(screen.getByRole("button", { name: "정기지출에 연결" }));

    await waitFor(() => expect(matchArgs).not.toBeNull());
    expect(matchArgs).toMatchObject({
      eventId: transaction.id,
      enableFutureAutoMatch: true,
      expectedVersion: occurrence?.version,
    });
  });

  it("반복 거래 후보 등록의 검토 완료 실패를 중복 생성 없이 재시도한다", async () => {
    const user = userEvent.setup();
    const base = createMemoryTransport();
    const baseApi = createApi(base);
    const month = (await baseApi.getSnapshot()).today.slice(0, 7);
    const transactions = (await baseApi.listExpenseTransactions({ month, limit: 100 })).items;
    const transaction = { ...transactions.find((item) => item.id === "expense-openai")!, pendingReviewId: "review-recurring-register" };
    const review: ExpenseReview = {
      id: "review-recurring-register",
      reason: "recurring_registration_candidate",
      status: "pending",
      transaction,
      recurringExpenseId: null,
      suggestedKind: "purchase",
      suggestedCategory: "ott_subscriptions",
      suggestedDuplicateOfEventId: null,
      createdAt: transaction.occurredAt,
      resolvedAt: null,
      version: 2,
    };
    let createCount = 0;
    let resolveCount = 0;
    const transport = transportWithReview(base, review, transactions, (command) => {
      if (command === "create_recurring_expense") createCount += 1;
      if (command !== "resolve_expense_review") return undefined;
      resolveCount += 1;
      if (resolveCount === 1) throw new Error("합성 검토 완료 실패");
      return { handled: true, value: null };
    });
    render(<App api={createApi(transport)} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "확인 필요" }));
    await user.click(await screen.findByRole("button", { name: "정기지출로 등록" }));

    expect(screen.getByLabelText("이름")).toHaveValue("OpenAI");
    expect(screen.queryByLabelText(/fingerprint/i)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "등록하고 검토 완료" }));
    const retry = await screen.findByRole("button", { name: "검토 완료 다시 시도" });
    expect(createCount).toBe(1);
    expect(resolveCount).toBe(1);

    await user.click(retry);
    await waitFor(() => expect(resolveCount).toBe(2));
    expect(createCount).toBe(1);
  });

  it("정기지출을 등록하고 납부 완료를 실제 발생 건으로 기록한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "정기지출" }));

    await user.click(await screen.findByRole("button", { name: /정기지출 추가/ }));
    expect(screen.getByLabelText("예상 금액 (minor unit)")).toHaveAttribute("min", "1");
    await user.type(screen.getByLabelText("이름"), "합성 구독");
    await user.clear(screen.getByLabelText("예상 금액 (minor unit)"));
    await user.type(screen.getByLabelText("예상 금액 (minor unit)"), "9900");
    await user.click(screen.getByRole("button", { name: "저장" }));

    expect(await screen.findByRole("status")).toHaveTextContent("정기지출을 등록했습니다");
    expect((await screen.findAllByText("합성 구독")).length).toBeGreaterThan(0);
    expect((await api.listRecurringExpenses()).some((item) => item.name === "합성 구독")).toBe(true);

    const insurance = (await screen.findAllByText("보험료"))
      .map((element) => element.closest("article"))
      .find((element): element is HTMLElement => element instanceof HTMLElement);
    expect(insurance).not.toBeNull();
    await user.click(within(insurance as HTMLElement).getByRole("button", { name: "납부 완료" }));
    await waitFor(async () => {
      const month = (await api.getSnapshot()).today.slice(0, 7);
      expect((await api.listRecurringExpenseOccurrences(month)).find((item) => item.name === "보험료")?.status).toBe("paid");
    });
  });

  it("캘린더 정기지출 발생 건은 일정 편집 대신 지출 상세로 이동한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getAllByRole("button", { name: "캘린더" })[0]);

    const openButton = await screen.findByRole("button", { name: "보험료 정기지출 열기" });
    await user.click(openButton);
    expect(await screen.findByRole("heading", { name: "지출", level: 1 })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /보험료/ })).toHaveClass("recurring-item--selected");
  });

  it("일반 메뉴로 나갔다가 지출에 재진입하면 캘린더의 정기지출 선택을 지운다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getAllByRole("button", { name: "캘린더" })[0]);
    await user.click(await screen.findByRole("button", { name: "보험료 정기지출 열기" }));
    expect(screen.getByRole("button", { name: /보험료/ })).toHaveClass("recurring-item--selected");

    await user.click(screen.getAllByRole("button", { name: "오늘" })[0]);
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getAllByRole("button", { name: "지출" })[0]);
    expect(await screen.findByRole("heading", { name: "AI 지출 해설" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /보험료/ })).not.toBeInTheDocument();
  });

  it("정기지출 최초 거래 연결은 월 거래 선택과 별도 자동 연결 동의를 요구한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "정기지출" }));

    const insurance = (await screen.findAllByText("보험료"))
      .map((element) => element.closest("article"))
      .find((element): element is HTMLElement => element instanceof HTMLElement);
    expect(insurance).toBeDefined();
    await user.click(within(insurance as HTMLElement).getByRole("button", { name: "거래 연결" }));
    const selector = within(insurance as HTMLElement).getByRole("combobox");
    const autoMatch = within(insurance as HTMLElement).getByRole("checkbox", { name: /이후 고신뢰 거래도 자동 연결/ });
    expect(autoMatch).not.toBeChecked();
    await user.selectOptions(selector, "expense-lunch");
    await user.click(within(insurance as HTMLElement).getByRole("button", { name: "거래 연결 확인" }));

    const month = (await api.getSnapshot()).today.slice(0, 7);
    await waitFor(async () => {
      expect((await api.listRecurringExpenseOccurrences(month)).find((item) => item.name === "보험료")?.status).toBe("matched");
    });
    expect((await api.listRecurringExpenses()).find((item) => item.name === "보험료")?.autoMatchEnabled).toBe(false);
  });

  it("활성 출처와 확정 리포트 필수 여부를 version CAS로 저장한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await openExpenses(user);

    const required = await screen.findByRole("checkbox", { name: "카카오페이머니 확정 리포트 필수" });
    expect(required).not.toBeChecked();
    const row = required.closest("article");
    expect(row).not.toBeNull();
    await user.click(required);
    await user.click(within(row as HTMLElement).getByRole("button", { name: "설정 저장" }));

    await waitFor(async () => {
      expect((await api.listExpenseSources()).find((source) => source.adapter === "kakaopay_money_v1"))
        .toMatchObject({ requiredForCompleteReport: true, version: 2 });
    });
  });

  it("거래 재분류는 기존 분할 금액을 보존하고 명시 해제만 반영한다", async () => {
    const user = userEvent.setup();
    const transport = createMemoryTransport();
    const expenses = Reflect.get(transport, "expenses") as { transactions: ExpenseTransaction[] };
    const lunch = expenses.transactions.find((item) => item.id === "expense-lunch");
    if (!lunch) throw new Error("mock 지출 거래 fixture가 없습니다.");
    lunch.personalAmountMinor = 6_000;
    const api = createApi(transport);
    render(<App api={api} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "거래" }));

    let card = (await screen.findByText("동네식당")).closest("article");
    expect(card).not.toBeNull();
    await user.click(within(card as HTMLElement).getByRole("button", { name: "재분류·제외 해제" }));
    const kind = within(card as HTMLElement).getByLabelText("expense-lunch 새 처리");
    expect(within(kind).queryByRole("option", { name: "외부 송금" })).not.toBeInTheDocument();
    expect(within(kind).queryByRole("option", { name: "미확인 송금" })).not.toBeInTheDocument();
    expect(within(kind).queryByRole("option", { name: "수동 정기지출" })).not.toBeInTheDocument();
    expect(within(card as HTMLElement).getByLabelText(/^내 부담액 \(minor unit·선택\)/))
      .toHaveValue(6_000);
    await user.selectOptions(within(card as HTMLElement).getByLabelText("expense-lunch 새 카테고리"), "shopping");
    await user.click(within(card as HTMLElement).getByRole("button", { name: "재분류 저장" }));
    await waitFor(() => expect(expenses.transactions.find((item) => item.id === "expense-lunch"))
      .toMatchObject({ category: "shopping", personalAmountMinor: 6_000, version: 2 }));

    card = (await screen.findByText("동네식당")).closest("article");
    expect(card).not.toBeNull();
    await user.click(within(card as HTMLElement).getByRole("button", { name: "재분류·제외 해제" }));
    await user.click(within(card as HTMLElement).getByRole("checkbox", { name: /기존 내 부담액.*명시적으로 해제/ }));
    await user.click(within(card as HTMLElement).getByRole("button", { name: "재분류 저장" }));
    await waitFor(() => expect(expenses.transactions.find((item) => item.id === "expense-lunch"))
      .toMatchObject({ personalAmountMinor: null, version: 3 }));
  });

  it("일반 검토는 서버 의미를 우회하는 처리와 업체 없는 자동 규칙을 노출하지 않는다", async () => {
    const user = userEvent.setup();
    renderApp();
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "확인 필요" }));

    const kind = await screen.findByLabelText("처리");
    expect(within(kind).queryByRole("option", { name: "외부 송금" })).not.toBeInTheDocument();
    expect(within(kind).queryByRole("option", { name: "미확인 송금" })).not.toBeInTheDocument();
    expect(within(kind).queryByRole("option", { name: "수동 정기지출" })).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: "앞으로 같은 업체에 적용" })).not.toBeInTheDocument();
    expect(screen.getByText("업체 정보가 없는 송금·이체는 자동 분류 규칙을 만들 수 없습니다.")).toBeInTheDocument();
  });

  it("월말 구매는 다음 달 정산 검토의 안전한 연결 후보로 표시한다", async () => {
    const user = userEvent.setup();
    const base = createMemoryTransport();
    const baseApi = createApi(base);
    const month = (await baseApi.getSnapshot()).today.slice(0, 7);
    const [year, monthNumber] = month.split("-").map(Number);
    const previousLastDate = new Date(Date.UTC(year, monthNumber - 1, 0));
    const previousLast = `${previousLastDate.getUTCFullYear()}-${String(previousLastDate.getUTCMonth() + 1).padStart(2, "0")}-${String(previousLastDate.getUTCDate()).padStart(2, "0")}`;
    const previousMonth = previousLast.slice(0, 7);
    const template = (await baseApi.listExpenseTransactions({ month, limit: 100 })).items[0];
    const purchase: ExpenseTransaction = {
      ...template,
      id: "expense-previous-purchase",
      kind: "purchase",
      category: "food",
      status: "confirmed",
      amountMinor: 12_000,
      occurredAt: `${previousLast}T23:30:00+09:00`,
      postedDate: previousLast,
      merchant: "월말 식사",
      counterparty: null,
      isProvisional: false,
      pendingReviewId: null,
    };
    const provisional = {
      ...purchase,
      id: "expense-provisional-purchase",
      merchant: "미확정 구매",
      isProvisional: true,
    };
    const settlement: ExpenseTransaction = {
      ...template,
      id: "expense-current-settlement",
      kind: "unknown_p2p",
      category: "unconfirmed",
      status: "unconfirmed",
      amountMinor: 6_000,
      occurredAt: `${month}-01T09:00:00+09:00`,
      postedDate: `${month}-01`,
      merchant: null,
      counterparty: "친구에게 송금",
      isProvisional: true,
      pendingReviewId: "review-cross-month-settlement",
    };
    const review: ExpenseReview = {
      id: "review-cross-month-settlement",
      reason: "unknown_p2p",
      status: "pending",
      transaction: settlement,
      recurringExpenseId: null,
      suggestedKind: "settlement_sent",
      suggestedCategory: "transfer_settlement",
      suggestedDuplicateOfEventId: null,
      createdAt: settlement.occurredAt,
      resolvedAt: null,
      version: 1,
    };
    const transport = transportWithReview(base, review, [settlement], (command, args) => {
      if (command !== "list_expense_transactions") return undefined;
      const requestedMonth = (args.input as { month: string }).month;
      const items = requestedMonth === previousMonth
        ? [purchase, provisional]
        : requestedMonth === month ? [settlement] : [];
      return { handled: true, value: { items, nextCursor: null } };
    });
    render(<App api={createApi(transport)} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "확인 필요" }));

    const form = (await screen.findByText("친구에게 송금")).closest("form");
    expect(form).not.toBeNull();
    const related = within(form as HTMLElement).getByLabelText(/^정산·연결 대상 \(선택\)/);
    const personal = within(form as HTMLElement).getByLabelText(/^내 부담액 \(minor unit·선택\)/);
    expect(related).toBeEnabled();
    expect(within(related).getByRole("option", { name: /월말 식사/ })).toHaveValue(purchase.id);
    expect(within(related).queryByRole("option", { name: /미확정 구매/ })).not.toBeInTheDocument();
    expect(personal).toBeDisabled();

    await user.selectOptions(within(form as HTMLElement).getByLabelText("처리"), "purchase");
    expect(related).toBeDisabled();
    expect(personal).toBeEnabled();
  });

  it("기존 자동 연결 동의는 정기지출 편집에서 즉시 철회할 수 있다", async () => {
    const user = userEvent.setup();
    const base = createMemoryTransport();
    const baseApi = createApi(base);
    const today = (await baseApi.getSnapshot()).today;
    let updateInput: Record<string, unknown> | null = null;
    const transport: CommandTransport = {
      async invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
        if (command === "update_recurring_expense") updateInput = args.input as Record<string, unknown>;
        return base.invoke<T>(command, args);
      },
    };
    const api = createApi(transport);
    render(<App api={api} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "다음 달" }));
    await user.click(screen.getByRole("button", { name: "정기지출" }));
    await user.click(await screen.findByRole("button", { name: /월세/ }));

    const consent = screen.getByRole("checkbox", { name: /이후 고신뢰 거래 자동 연결/ });
    expect(consent).toBeChecked();
    expect(consent).toBeEnabled();
    await user.click(consent);
    await user.click(screen.getByRole("button", { name: "저장" }));

    await waitFor(async () => {
      expect((await api.listRecurringExpenses()).find((item) => item.name === "월세")?.autoMatchEnabled).toBe(false);
    });
    expect(updateInput).toMatchObject({
      autoMatchEnabled: false,
      effectiveFromMonth: `${today.slice(0, 7)}-01`,
    });
  });

  it("과거 월 납부 확인은 발생일을 기본 납부일로 보내 현재 월 지출을 왜곡하지 않는다", async () => {
    const user = userEvent.setup();
    const base = createMemoryTransport();
    const baseApi = createApi(base);
    const today = (await baseApi.getSnapshot()).today;
    const [year, month] = today.slice(0, 7).split("-").map(Number);
    const previousDate = new Date(Date.UTC(year, month - 2, 1));
    const previousMonth = `${previousDate.getUTCFullYear()}-${String(previousDate.getUTCMonth() + 1).padStart(2, "0")}`;
    const expenses = Reflect.get(base, "expenses") as { recurring: Array<{ id: string; startDate: string }> };
    const insurance = expenses.recurring.find((item) => item.id === "recurring-insurance");
    if (!insurance) throw new Error("mock 정기지출 fixture가 없습니다.");
    insurance.startDate = `${previousMonth}-01`;
    const occurrence = (await baseApi.listRecurringExpenseOccurrences(previousMonth))
      .find((item) => item.recurringExpenseId === insurance.id);
    expect(occurrence).toBeDefined();
    let paidDate: string | null = null;
    const transport: CommandTransport = {
      async invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
        if (command === "confirm_recurring_expense_paid") paidDate = String(args.paidDate);
        return base.invoke<T>(command, args);
      },
    };
    render(<App api={createApi(transport)} />);
    await openExpenses(user);
    await user.click(screen.getByRole("button", { name: "이전 달" }));
    await user.click(screen.getByRole("button", { name: "정기지출" }));

    const card = (await screen.findAllByText("보험료"))
      .map((element) => element.closest("article"))
      .find((element): element is HTMLElement => element instanceof HTMLElement);
    expect(card).toBeDefined();
    await user.click(within(card as HTMLElement).getByRole("button", { name: "납부 완료" }));
    await waitFor(() => expect(paidDate).toBe(occurrence?.dueDate));
  });
});
