import { readFileSync } from "node:fs";
import { join } from "node:path";
import process from "node:process";

const pwaSource = (name: string) =>
  readFileSync(join(process.cwd(), "crates", "tm-server", "src", "pwa", name), "utf8")
    .replaceAll("\r\n", "\n");

describe("모바일 PWA Task 프로젝트 기본값", () => {
  const app = pwaSource("app.js");
  const shell = pwaSource("index.html");
  const serviceWorker = pwaSource("sw.js");

  it("systemKey로 기타 프로젝트를 찾고 새 Task의 기본 프로젝트로 사용한다", () => {
    expect(app).toContain('project.systemKey === "uncategorized"');
    expect(app).toContain(
      'projectId || state.taskFilters.projectId || uncategorizedProject()?.id || ""',
    );
    expect(app).toContain('if (!task) byId("task-status").value = "todo"');
  });

  it("기존 프로젝트 미지정 Task 편집에서는 현재 필터를 상속하지 않는다", () => {
    expect(app).toContain('task\n    ? (task.projectId || "")');
    expect(app).not.toContain("task?.projectId || projectId || state.taskFilters.projectId");
    expect(app).toContain('state.editingTask.projectId || ""');
  });

  it("기타 프로젝트 응답이 없을 때 서버 자동 배정 fallback을 유지한다", () => {
    expect(shell).toContain('<option value="">기타에 자동 배정</option>');
    expect(app).toContain('const projectId = byId("task-project").value || null');
    expect(app).toContain("projectId,\n          title,");
    expect(app).toContain('if (!projectId) return "기타"');
  });

  it("변경된 모바일 shell을 즉시 갱신하도록 캐시 버전을 올린다", () => {
    expect(serviceWorker).toContain('const CACHE_NAME = "tm-mobile-shell-v16-simple-ui-v1"');
  });

  it("모바일 첫 화면은 한 줄 Task 목록이고 프로젝트 관리는 접어서 제공한다", () => {
    expect(shell).toContain('<button data-tab="tasks" aria-current="page">할 일</button>');
    expect(shell).toContain('<section id="tab-tasks" class="tab-panel panel">');
    expect(shell).toContain('<details class="project-browser project-browser--collapsed">');
    expect(app).toContain('activeTab: "tasks"');
    expect(app).toContain('void loadTaskWorkspace();');
    expect(app).toContain('const card = text("article", "", "item-card task-card")');
    expect(app).toContain('const open = text("button", "", "task-card-main")');
  });
});
