import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { Project, Task, UpdateTaskInput } from "../types";
import { TaskDetail } from "./TaskDetail";

const projects: Project[] = [
  {
    id: "project-tm",
    name: "TM",
    description: "",
    color: "#5b6cf9",
    systemKey: null,
    openTaskCount: 1,
    completedTaskCount: 0,
    archived: false,
  },
  {
    id: "project-uncategorized",
    name: "기타",
    description: "프로젝트를 선택하지 않은 Task",
    color: "#7386ff",
    systemKey: "uncategorized",
    openTaskCount: 0,
    completedTaskCount: 0,
    archived: false,
  },
];

const legacyTask: Task = {
  id: "task-legacy",
  title: "국민은행 부가서비스 해지",
  description: "",
  status: "done",
  priority: "none",
  dueDate: null,
  projectId: null,
  projectName: null,
  tags: [],
  checklist: [],
  events: [],
  workLogs: [],
  createdAt: "2026-07-01T00:00:00Z",
  updatedAt: "2026-07-01T00:00:00Z",
  deletedAt: null,
};

describe("Task 상세 프로젝트 선택", () => {
  it("기존 null 프로젝트를 시스템 기타로 표시하고 저장한다", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn<(input: UpdateTaskInput) => Promise<void>>().mockResolvedValue();
    render(
      <TaskDetail
        onClose={vi.fn()}
        onSave={onSave}
        onTrash={vi.fn().mockResolvedValue(undefined)}
        projects={projects}
        saving={false}
        task={legacyTask}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: /국민은행 부가서비스 해지/ });
    const projectSelect = within(dialog).getByLabelText("프로젝트") as HTMLSelectElement;
    expect(projectSelect).toHaveValue("project-uncategorized");
    expect(projectSelect.selectedOptions[0]).toHaveTextContent("기타");
    expect(within(dialog).queryByRole("option", { name: "프로젝트 없음" })).not.toBeInTheDocument();

    fireEvent.change(projectSelect, { target: { value: "" } });
    expect(projectSelect).toHaveValue("project-uncategorized");
    await user.click(within(dialog).getByRole("button", { name: "변경 저장" }));

    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
      taskId: legacyTask.id,
      projectId: "project-uncategorized",
    }));
  });
});
