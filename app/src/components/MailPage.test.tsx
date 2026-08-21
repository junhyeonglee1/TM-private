import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { TmApi } from "../lib/api";
import type { MailItem } from "../types";
import { MailPage, TodayMailCards } from "./MailPage";

const importantItem: MailItem = {
  id: "mail-important-1",
  accountId: "mail-account-1",
  provider: "gmail",
  sender: "보안팀 <security@example.com>",
  senderDomain: "example.com",
  subject: "새 로그인 보안 경고",
  summary: "민감한 인증·보안 메일입니다. 원문에서 직접 확인하세요.",
  action: "계정 보안 상태를 확인하세요.",
  deadline: null,
  receivedAt: "2026-08-21T00:10:00Z",
  classification: "important",
  importanceScore: 95,
  confidence: 95,
  decisionSource: "rule",
  decisionReason: "security_alert",
  sensitiveKind: "security_alert",
  acknowledgedAt: null,
  originalUrl: "https://mail.google.com/mail/u/0/#all/message-1",
  version: 1,
};

const mailApi = (overrides: Partial<TmApi> = {}): TmApi => ({
  canImportExpenses: false,
  canManageMailAccounts: false,
  getMailAccounts: async () => [{
    id: "mail-account-1",
    provider: "gmail",
    email: "owner@example.com",
    displayName: null,
    status: "connected",
    lastSuccessAt: "2026-08-21T00:10:00Z",
    nextExpectedAt: "2026-08-21T00:15:00Z",
    watchExpiresAt: "2026-08-27T00:00:00Z",
    lastErrorCode: null,
    consecutiveFailures: 0,
    createdAt: "2026-08-21T00:00:00Z",
    updatedAt: "2026-08-21T00:10:00Z",
    version: 1,
  }],
  getMailSummary: async () => ({
    date: "2026-08-21",
    unacknowledgedImportant: 1,
    reviewCount: 0,
    latestReport: null,
    presentationEnabled: true,
  }),
  listMailItems: async (input) => ({
    items: input.status === "important" ? [importantItem] : [],
    nextCursor: null,
  }),
  getMailReports: async () => [],
  acknowledgeMailItem: async () => ({ ...importantItem, acknowledgedAt: "2026-08-21T00:20:00Z", version: 2 }),
  feedbackMailItem: async (_id, important) => ({
    ...importantItem,
    classification: important ? "important" : "not_important",
    decisionSource: "manual",
    version: 2,
  }),
  ...overrides,
} as TmApi);

describe("메일 모니터링 UI", () => {
  it("최소 메타데이터만 표시하고 원문은 공급자 웹메일로 연다", async () => {
    render(<MailPage api={mailApi()} onNotify={() => undefined} today="2026-08-21" />);

    const subject = await screen.findByRole("heading", { name: "새 로그인 보안 경고" });
    const row = subject.closest("article");
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getByText("보안팀 <security@example.com>")).toBeInTheDocument();
    expect(within(row as HTMLElement).getByText(/민감한 인증·보안 메일/)).toBeInTheDocument();
    const original = within(row as HTMLElement).getByRole("link", { name: "원문 열기" });
    expect(original).toHaveAttribute("href", importantItem.originalUrl);
    expect(original).toHaveAttribute("target", "_blank");
    expect(document.querySelector("iframe")).not.toBeInTheDocument();
  });

  it("확인 완료와 중요도 피드백을 명시적 사용자 동작으로만 보낸다", async () => {
    const user = userEvent.setup();
    const acknowledge = vi.fn(mailApi().acknowledgeMailItem);
    const feedback = vi.fn(mailApi().feedbackMailItem);
    const notify = vi.fn();
    render(<MailPage api={mailApi({ acknowledgeMailItem: acknowledge, feedbackMailItem: feedback })} onNotify={notify} today="2026-08-21" />);

    await screen.findByRole("heading", { name: "새 로그인 보안 경고" });
    expect(acknowledge).not.toHaveBeenCalled();
    expect(feedback).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "확인 완료" }));
    expect(acknowledge).toHaveBeenCalledWith("mail-important-1", 1);
    await user.click(screen.getByRole("button", { name: "중요하지 않음" }));
    expect(feedback).toHaveBeenCalledWith("mail-important-1", false, 2);
    expect(notify).toHaveBeenCalledWith("TM 안에서 메일 확인을 완료했습니다.");
  });

  it("운영 gate가 꺼져 있으면 계정 정보 대신 준비 상태를 안전하게 표시한다", async () => {
    const api = mailApi({ getMailAccounts: async () => { throw new Error("MAIL_DISABLED"); } });
    render(<MailPage api={api} onNotify={() => undefined} today="2026-08-21" />);
    expect(await screen.findByRole("heading", { name: "메일 모니터링 준비 중" })).toBeInTheDocument();
    expect(screen.getByText("MAIL_DISABLED")).toBeInTheDocument();
    expect(screen.queryByText("owner@example.com")).not.toBeInTheDocument();
  });

  it("Today에는 확인할 메일이 있을 때만 한 줄 진입 카드를 만든다", async () => {
    const user = userEvent.setup();
    const open = vi.fn();
    const { rerender } = render(<TodayMailCards api={mailApi()} onOpenMail={open} today="2026-08-21" />);
    await user.click(await screen.findByRole("button", { name: /중요 메일 1개/ }));
    expect(open).toHaveBeenCalledTimes(1);

    rerender(<TodayMailCards api={mailApi({ getMailSummary: async () => ({
      date: "2026-08-21",
      unacknowledgedImportant: 0,
      reviewCount: 0,
      latestReport: null,
      presentationEnabled: true,
    }) })} onOpenMail={open} today="2026-08-21" />);
    await waitFor(() => expect(screen.queryByRole("button", { name: /중요 메일/ })).not.toBeInTheDocument());
  });
});
