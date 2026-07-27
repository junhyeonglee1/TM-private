import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { App } from "./App";
import { createApi } from "./lib/api";
import { createMemoryTransport } from "./lib/mock-transport";

const renderApp = () => {
  const transport = createMemoryTransport();
  const api = createApi(transport);
  return { ...render(<App api={api} />), api };
};

describe("TM 데스크톱 UI", () => {
  it("상단에 API와 Cloud 비용만 월 한도와 함께 표시한다", async () => {
    renderApp();

    const status = await screen.findByLabelText("현재 비용 현황");
    expect(within(status).getByText("API $0.01 / $20")).toBeInTheDocument();
    expect(within(status).getByText("Cloud $0.09 / $30")).toBeInTheDocument();
    expect(status).not.toHaveTextContent("tokens");
  });

  it("오늘의 네 개 기록 영역을 snapshot에서 표시한다", async () => {
    renderApp();

    expect(await screen.findByRole("heading", { name: "오늘", level: 1 })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "어제 미완료" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "오늘 계획" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "진행 중" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "오늘 완료" })).toBeInTheDocument();
    expect(screen.getByText("참고자료 링크 분류")).toBeInTheDocument();
    expect(screen.queryByLabelText("빠른 Task 제목")).not.toBeInTheDocument();

    const yesterdaySection = screen.getByRole("heading", { name: "어제 미완료" }).closest("section");
    const plannedSection = screen.getByRole("heading", { name: "오늘 계획" }).closest("section");
    expect(yesterdaySection).not.toBeNull();
    expect(plannedSection).not.toBeNull();
    expect(within(yesterdaySection as HTMLElement).getAllByRole("button", { name: "이월" })).toHaveLength(1);
    expect(within(plannedSection as HTMLElement).queryByRole("button", { name: "이월" })).not.toBeInTheDocument();
  });

  it("오늘 Task를 목록에서 확인 모달을 거쳐 원터치로 완료한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    const progressTask = screen.getByText("데이터 모델 불변 조건 검토").closest("article");
    expect(progressTask).not.toBeNull();
    await user.click(within(progressTask as HTMLElement).getByRole("button", {
      name: "데이터 모델 불변 조건 검토 완료",
    }));

    let dialog = await screen.findByRole("dialog", { name: "Task를 완료할까요?" });
    expect(within(dialog).getByText("데이터 모델 불변 조건 검토")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "취소" }));
    expect(screen.queryByRole("dialog", { name: "Task를 완료할까요?" })).not.toBeInTheDocument();
    expect((await api.getSnapshot()).tasks.find((task) => task.id === "task-schema")?.status)
      .toBe("in_progress");

    await user.click(within(progressTask as HTMLElement).getByRole("button", {
      name: "데이터 모델 불변 조건 검토 완료",
    }));
    dialog = await screen.findByRole("dialog", { name: "Task를 완료할까요?" });
    await user.click(within(dialog).getByRole("button", { name: "완료 처리" }));

    expect(await screen.findByRole("status")).toHaveTextContent("Task를 완료했습니다: 데이터 모델 불변 조건 검토");
    await waitFor(async () => {
      expect((await api.getSnapshot()).tasks.find((task) => task.id === "task-schema")?.status)
        .toBe("done");
    });
  });

  it("오늘 계획 Task 완료는 확인 모달 뒤 날짜 기록까지 함께 확정한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    const plannedTask = screen.getByText("오늘 화면 정보 밀도 다듬기").closest("article");
    expect(plannedTask).not.toBeNull();
    await user.click(within(plannedTask as HTMLElement).getByRole("button", {
      name: "오늘 화면 정보 밀도 다듬기 완료",
    }));
    const dialog = await screen.findByRole("dialog", { name: "Task를 완료할까요?" });
    await user.click(within(dialog).getByRole("button", { name: "완료 처리" }));

    await waitFor(async () => {
      const snapshot = await api.getSnapshot();
      expect(snapshot.todayView.completed.find((entry) => entry.taskId === "task-ui")?.status)
        .toBe("done");
      expect(snapshot.tasks.find((task) => task.id === "task-ui")?.status).toBe("done");
    });
  });

  it("프로젝트의 열린 Task도 목록에서 바로 완료한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getAllByRole("button", { name: "프로젝트" })[0]);

    const task = (await screen.findByText("JSON · Markdown 내보내기 검증")).closest("article");
    expect(task).not.toBeNull();
    await user.click(within(task as HTMLElement).getByRole("button", {
      name: "JSON · Markdown 내보내기 검증 완료",
    }));
    const dialog = await screen.findByRole("dialog", { name: "Task를 완료할까요?" });
    await user.click(within(dialog).getByRole("button", { name: "완료 처리" }));

    await waitFor(async () => {
      expect((await api.getSnapshot()).tasks.find((item) => item.id === "task-export")?.status)
        .toBe("done");
    });
  });

  it("오늘의 Task·일정 AI 리포트를 만들고 품질 평가를 기록한다", async () => {
    const user = userEvent.setup();
    renderApp();

    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getByRole("button", { name: "AI 리포트 만들기" }));

    expect(await screen.findByRole("heading", { name: "오늘의 Task와 일정을 확인하세요" })).toBeInTheDocument();
    expect(screen.getByText(/500 tokens/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "도움 됨" }));
    expect(await screen.findByText("도움 됨으로 평가함")).toBeInTheDocument();
  });

  it("Inbox는 기능 없는 기획 대기 화면으로 유지한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getAllByRole("button", { name: /Inbox/ })[0]);
    expect(await screen.findByRole("heading", { name: "Inbox", level: 1 })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Inbox는 잠시 비워 두었습니다" })).toBeInTheDocument();
    expect(screen.getByText(/즉흥 수집과 분류 기능은 제거했습니다/)).toBeInTheDocument();
    expect(screen.queryByLabelText("빠른 Task 제목")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("빠른 Task 프로젝트")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("빠른 Task 우선순위")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "정리 완료" })).not.toBeInTheDocument();
  });

  it("개인 캘린더에 매월 말일 납부 일정을 추가한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    const snapshot = await api.getSnapshot();

    await user.click(screen.getAllByRole("button", { name: "캘린더" })[0]);
    expect(await screen.findByRole("heading", { name: "캘린더", level: 1 })).toBeInTheDocument();
    expect((await screen.findAllByText("보험료 납부")).length).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "일정 추가" }));

    const formHeading = screen.getByRole("heading", { name: "일정 추가", level: 2 });
    const formPanel = formHeading.closest("section");
    expect(formPanel).not.toBeNull();
    const form = within(formPanel as HTMLElement);
    await user.type(form.getByLabelText("제목"), "관리비 납부");
    await user.selectOptions(form.getByLabelText("종류"), "payment");
    await user.selectOptions(form.getByLabelText("반복"), "monthly_last_day");
    await user.click(form.getByRole("button", { name: "일정 추가" }));

    expect(await screen.findByRole("status")).toHaveTextContent("일정을 추가했습니다");
    expect((await screen.findAllByText("관리비 납부")).length).toBeGreaterThan(0);
    const month = await api.getCalendarMonth(snapshot.today.slice(0, 7));
    const created = month.events.find((event) => event.title === "관리비 납부");
    expect(created).toMatchObject({ kind: "payment", recurrence: "monthly_last_day" });
    expect(month.occurrences.find((occurrence) => occurrence.eventId === created?.id)?.date)
      .toBe(month.monthEnd);
  });

  it("새 프로젝트를 자동 선택하고 compact 입력으로 todo Task를 추가한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getAllByRole("button", { name: "프로젝트" })[0]);

    expect(await screen.findByRole("heading", { name: "프로젝트 목록" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "진행 중" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "할 일" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "막힘" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "정리 대기" })).not.toBeInTheDocument();

    await user.type(screen.getByLabelText("새 프로젝트 이름"), "고객 인터뷰");
    await user.click(screen.getByRole("button", { name: "추가" }));
    expect(await screen.findByRole("heading", { name: "고객 인터뷰", level: 2 })).toBeInTheDocument();

    await user.type(screen.getByLabelText("고객 인터뷰 Task 제목"), "질문지 초안 작성");
    await user.type(screen.getByLabelText("고객 인터뷰 Task 설명"), "핵심 질문과 인터뷰 순서를 정리한다");
    await user.selectOptions(screen.getByLabelText("고객 인터뷰 Task 우선순위"), "high");
    await user.type(screen.getByLabelText("고객 인터뷰 Task 마감일"), "2026-07-15");
    await user.click(screen.getByRole("button", { name: "Task 추가" }));

    expect(await screen.findByText("질문지 초안 작성")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("프로젝트에 할 일을 추가했습니다");
    const snapshot = await api.getSnapshot();
    const project = snapshot.projects.find((item) => item.name === "고객 인터뷰");
    const created = snapshot.tasks.find((task) => task.title === "질문지 초안 작성");
    expect(created).toMatchObject({
      status: "todo",
      projectId: project?.id,
      description: "핵심 질문과 인터뷰 순서를 정리한다",
      priority: "high",
      dueDate: "2026-07-15",
    });
  });

  it("프로젝트에서 완료 필터와 기존 Task 상세 drawer를 연다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getAllByRole("button", { name: "프로젝트" })[0]);

    await user.click(screen.getByRole("button", { name: "완료 · 취소" }));
    expect(await screen.findByText("백업 정책 초안 작성")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "열린 Task" }));
    await user.click(await screen.findByText("데이터 모델 불변 조건 검토"));
    expect(await screen.findByRole("dialog", { name: /데이터 모델 불변 조건 검토/ })).toBeInTheDocument();
  });

  it("Task 상세에서 전체 필드와 불변 이벤트를 확인하고 변경한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getByText("데이터 모델 불변 조건 검토"));
    const dialog = await screen.findByRole("dialog", { name: /데이터 모델 불변 조건 검토/ });
    expect((within(dialog).getByLabelText("설명") as HTMLTextAreaElement).value).toContain("TaskDayEntry");
    expect(within(dialog).getByRole("heading", { name: "체크리스트" })).toBeInTheDocument();
    expect(within(dialog).getByRole("heading", { name: "활동" })).toBeInTheDocument();
    expect(within(dialog).getByText("상태를 진행 중으로 변경했습니다")).toBeInTheDocument();

    await user.selectOptions(within(dialog).getByLabelText("우선순위"), "medium");
    await user.click(within(dialog).getByRole("button", { name: "변경 저장" }));

    expect(await screen.findByRole("status")).toHaveTextContent("Task 변경과 이벤트를 저장했습니다");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("세션 시작과 종료 결과를 하나의 흐름으로 기록한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getAllByRole("button", { name: /작업 세션/ })[0]);
    await user.type(screen.getByLabelText(/목표/), "복원 트랜잭션 검증");
    await user.click(screen.getByRole("button", { name: /집중 세션 시작/ }));

    expect(await screen.findByRole("heading", { name: "복원 트랜잭션 검증" })).toBeInTheDocument();
    await user.type(screen.getByLabelText(/결과/), "모든 연결이 유지되는 것을 확인했습니다");
    await user.type(screen.getByLabelText("다음 행동"), "대용량 DB 케이스 추가");
    await user.click(screen.getByRole("button", { name: /세션 종료 및 기록/ }));

    expect(await screen.findByRole("status")).toHaveTextContent("원자적으로 저장했습니다");
    expect(screen.getByText("복원 트랜잭션 검증")).toBeInTheDocument();
    expect(screen.getByText("모든 연결이 유지되는 것을 확인했습니다")).toBeInTheDocument();
  });

  it("통합 검색으로 Task, WorkLog, Note를 함께 찾는다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getByRole("button", { name: /전체 기록 검색/ }));
    await user.type(screen.getByLabelText("통합 검색어"), "SQLite");
    await user.click(screen.getByRole("button", { name: "검색" }));

    expect(await screen.findByText("SQLite WAL과 busy timeout")).toBeInTheDocument();
    expect(screen.getByText(/검색 결과/)).toBeInTheDocument();
    expect(screen.getByText("SQLite WAL과 busy timeout").closest("article")).toHaveClass("search-result--static");
  });

  it("WorkLog와 Note를 목록에서 휴지통으로 이동한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getByRole("button", { name: "WorkLog" }));
    const logHeading = await screen.findByRole("heading", { name: "이력 불변성 검토" });
    const logCard = logHeading.closest("article");
    expect(logCard).not.toBeNull();
    await user.click(within(logCard as HTMLElement).getByRole("button", { name: "이력 불변성 검토 휴지통으로 이동" }));
    await waitFor(() => expect(screen.queryByRole("heading", { name: "이력 불변성 검토" })).not.toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: "Note" }));
    const noteHeading = await screen.findByRole("heading", { name: "SQLite WAL과 busy timeout" });
    const noteCard = noteHeading.closest("article");
    expect(noteCard).not.toBeNull();
    await user.click(within(noteCard as HTMLElement).getByRole("button", { name: "SQLite WAL과 busy timeout 휴지통으로 이동" }));
    await waitFor(() => expect(screen.queryByRole("heading", { name: "SQLite WAL과 busy timeout" })).not.toBeInTheDocument());
  });

  it("개선 요청 초안을 만들고 현재 revision을 명시적으로 승인한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getByRole("button", { name: /개선 요청함/ }));
    expect(await screen.findByRole("heading", { name: "개선 요청함", level: 1 })).toBeInTheDocument();
    expect(
      screen.getByText("TM 승인 요청 처리해줘. app/docs/prompts/change-request-processing.md를 따라줘."),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "새 요청" }));
    await user.selectOptions(screen.getByLabelText("개선 요청 유형"), "bug");
    await user.selectOptions(screen.getByLabelText("개선 요청 우선순위"), "high");
    await user.type(screen.getByLabelText("개선 요청 제목"), "검색 결과가 두 번 표시되는 문제");
    await user.type(screen.getByLabelText("개선 요청 설명"), "같은 검색 결과가 연속으로 두 번 표시됩니다.");
    await user.type(screen.getByLabelText("개선 요청 원하는 결과"), "각 결과가 한 번만 표시되어야 합니다.");
    await user.type(screen.getByLabelText("개선 요청 재현 절차"), "통합 검색에서 SQLite를 검색합니다.");
    await user.click(screen.getByRole("button", { name: "초안 저장" }));

    const createdHeading = await screen.findByRole("heading", {
      name: "검색 결과가 두 번 표시되는 문제",
    });
    const createdCard = createdHeading.closest("article");
    expect(createdCard).not.toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("개선 요청 초안을 저장했습니다.");

    let snapshot = await api.getSnapshot();
    const created = snapshot.changeRequests.find(
      (request) => request.title === "검색 결과가 두 번 표시되는 문제",
    );
    expect(created).toMatchObject({ status: "draft", revision: 1, approvedRevision: null });

    await user.click(within(createdCard as HTMLElement).getByRole("button", { name: "승인" }));
    await waitFor(async () => {
      snapshot = await api.getSnapshot();
      expect(snapshot.changeRequests.find((request) => request.id === created?.id)).toMatchObject({
        status: "approved",
        revision: 1,
        approvedRevision: 1,
      });
    });
    expect(
      screen.getAllByText("TM 승인 요청 처리해줘. app/docs/prompts/change-request-processing.md를 따라줘.").length,
    ).toBeGreaterThan(1);
  });

  it("개선 요청 작성 가이드는 유형에 맞게 바뀌고 입력 내용을 보존한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getByRole("button", { name: /개선 요청함/ }));
    await user.click(screen.getByRole("button", { name: "새 요청" }));

    const title = screen.getByLabelText("개선 요청 제목");
    await user.type(title, "작성 중인 요청은 그대로 유지");
    const guideSummary = screen.getByText("요청 작성 가이드").closest("summary");
    expect(guideSummary).not.toBeNull();
    await user.click(guideSummary as HTMLElement);

    expect(
      screen.getByRole("heading", { name: "기능 요청은 사용자 시나리오와 범위를 명확히 적으세요" }),
    ).toBeInTheDocument();
    expect(screen.getByText("좋은 완료 기준 예시")).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("개선 요청 유형"), "bug");
    expect(
      screen.getByRole("heading", { name: "버그 요청은 재현 가능한 사실을 중심으로 적으세요" }),
    ).toBeInTheDocument();
    expect(title).toHaveValue("작성 중인 요청은 그대로 유지");
  });

  it("실패 요청은 초안으로 돌린 뒤 편집하고 claimed 유실은 이유를 요구한다", async () => {
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getByRole("button", { name: /개선 요청함/ }));

    const failedHeading = await screen.findByRole("heading", {
      name: "히스토리 필터 응답 속도 개선",
    });
    const failedCard = failedHeading.closest("article");
    expect(failedCard).not.toBeNull();
    await user.click(
      within(failedCard as HTMLElement).getByRole("button", { name: "수정 후 재승인" }),
    );

    expect(await screen.findByRole("heading", { name: "개선 요청 초안 편집" })).toBeInTheDocument();
    const desiredOutcome = screen.getByLabelText("개선 요청 원하는 결과");
    await user.clear(desiredOutcome);
    await user.type(desiredOutcome, "많은 기록에서도 필터 결과가 즉시 표시되어야 합니다.");
    await user.click(screen.getByRole("button", { name: "초안 저장" }));

    let snapshot = await api.getSnapshot();
    expect(snapshot.changeRequests.find((request) => request.id === "change-failed")).toMatchObject({
      status: "draft",
      revision: 3,
      failureReason: null,
    });

    const claimedHeading = screen.getByRole("heading", {
      name: "세션 종료 요약이 작은 화면에서 잘리는 문제",
    });
    const claimedCard = claimedHeading.closest("article");
    expect(claimedCard).not.toBeNull();
    await user.click(
      within(claimedCard as HTMLElement).getByRole("button", { name: "처리 유실 기록" }),
    );
    const abandonReason = within(claimedCard as HTMLElement).getByLabelText("처리 유실 이유");
    await user.type(abandonReason, "Codex 실행이 종료되어 claim key를 더 이상 사용할 수 없습니다.");
    await user.click(
      within(claimedCard as HTMLElement).getByRole("button", { name: "실패로 전환" }),
    );

    snapshot = await api.getSnapshot();
    expect(snapshot.changeRequests.find((request) => request.id === "change-claimed")).toMatchObject({
      status: "failed",
      failureReason: "Codex 실행이 종료되어 claim key를 더 이상 사용할 수 없습니다.",
    });
  });

  it("완료와 취소 개선 요청은 읽기 전용으로 표시한다", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    await user.click(screen.getByRole("button", { name: /개선 요청함/ }));
    await user.click(screen.getByRole("button", { name: /완료 · 취소/ }));

    expect(await screen.findByText("백업 시각과 트리거를 한눈에 확인하도록 배지를 정리했습니다.")).toBeInTheDocument();
    expect(screen.getByText("현재 작업 방식에는 필요하지 않아 요청을 종료했습니다.")).toBeInTheDocument();
    expect(screen.getAllByText("읽기 전용")).toHaveLength(2);
  });

  it("조회 전용 주식 차트를 안전한 iframe으로 열고 관심 종목을 동기화한다", async () => {
    Object.defineProperty(window.navigator, "onLine", { configurable: true, value: true });
    const user = userEvent.setup();
    const { api } = renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });
    expect(await screen.findByRole("heading", { name: "S&P 500 일일 등락" })).toBeInTheDocument();
    expect(await screen.findByText("NVIDIA Corporation")).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "주식" })[0]);
    expect(await screen.findByRole("heading", { name: "주식 차트", level: 1 })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "S&P 500 일일 등락 스캐너" })).toBeInTheDocument();

    const frame = screen.getByTestId("tradingview-frame");
    const sandbox = frame.getAttribute("sandbox") ?? "";
    const source = frame.getAttribute("src") ?? "";
    expect(sandbox).toBe(
      "allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox",
    );
    expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
    expect(source).toMatch(/^https:\/\/www\.tradingview-widget\.com\/embed-widget\/advanced-chart\/\?locale=kr#/);
    const widgetSettings = JSON.parse(decodeURIComponent(new URL(source).hash.slice(1)));
    expect(widgetSettings).toMatchObject({
      symbol: "NASDAQ:AAPL",
      support_host: "https://www.tradingview.com",
      timezone: "Asia/Seoul",
      save_image: false,
      "page-uri": "__NHTTP__",
    });
    expect(source).not.toContain("data:text/html");
    expect(source).not.toContain("__TAURI");

    await user.selectOptions(screen.getByLabelText("방향"), "up");
    const nvidia = await screen.findByRole("button", { name: /NVIDIA Corporation \+14\.30%, 차트에서 보기/ });
    await user.click(nvidia);
    expect(screen.getByRole("heading", { name: "NASDAQ:NVDA", level: 2 })).toBeInTheDocument();

    await user.type(screen.getByRole("combobox", { name: "회사명 또는 종목코드" }), "삼성전자");
    await user.click(await screen.findByRole("option", { name: /삼성전자.*KRX:005930/ }));
    await user.click(screen.getByRole("button", { name: /관심 종목 저장/ }));

    expect(await screen.findByText("삼성전자 관심 종목을 저장했습니다.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "KRX:005930", level: 2 })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "TradingView 외부 차트에서 확인" }))
      .toHaveAttribute("href", "https://www.tradingview.com/symbols/KRX-005930/");
    expect((await api.getStockWatchlist())[0]).toMatchObject({
      symbol: "KRX:005930",
      market: "KRX",
      ticker: "005930",
      displayName: "삼성전자",
    });

    await user.click(screen.getByRole("button", { name: "삼성전자 관심 종목 삭제" }));
    await waitFor(async () => expect(await api.getStockWatchlist()).toHaveLength(0));
    expect(screen.getByRole("heading", { name: "NASDAQ:AAPL", level: 2 })).toBeInTheDocument();

    try {
      Object.defineProperty(window.navigator, "onLine", { configurable: true, value: false });
      act(() => window.dispatchEvent(new Event("offline")));
      expect(await screen.findByText("차트를 보려면 인터넷 연결이 필요합니다.")).toBeInTheDocument();
      expect(screen.queryByTestId("tradingview-frame")).not.toBeInTheDocument();
    } finally {
      Object.defineProperty(window.navigator, "onLine", { configurable: true, value: true });
      act(() => window.dispatchEvent(new Event("online")));
    }
  });

  it("페이지 이동 시 본문 포커스가 화면을 강제로 스크롤하지 않는다", async () => {
    const focus = vi.spyOn(HTMLElement.prototype, "focus");
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "오늘", level: 1 });

    await user.click(screen.getByRole("button", { name: /개선 요청함/ }));

    expect(focus).toHaveBeenLastCalledWith({ preventScroll: true });
    focus.mockRestore();
  });
});
