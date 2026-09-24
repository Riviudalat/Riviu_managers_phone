import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { OperatorRecord } from "../operatorRecords";
import { SavedTasksPage } from "./SavedTasksPage";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const date = "2026-09-17T08:00:00Z";
const tasks: OperatorRecord[] = [
  {
    id: "task-1", kind: "savedTask", name: "Nuôi tài khoản", revision: 1,
    data: { appId: "app-1", appRevision: 2, target: { type: "explicit", udids: [] }, inputs: {} },
    archived: false, createdAt: date, updatedAt: date,
  },
  {
    id: "task-2", kind: "savedTask", name: "Đăng video", revision: 1,
    data: { appId: "app-2", appRevision: 3, target: { type: "explicit", udids: [] }, inputs: {} },
    archived: false, createdAt: date, updatedAt: date,
  },
];

beforeEach(() => {
  invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "operator_list") return tasks;
    if (command === "app_workflow_list") return [
      { id: "app-1", name: "Tương tác", kind: "nurture", latestRevision: 2, updatedAt: date },
      { id: "app-2", name: "Kho nội dung", kind: "publish", latestRevision: 3, updatedAt: date },
    ];
    throw new Error(`Unexpected IPC command: ${command}`);
  });
});

it("lọc tác vụ theo tên hoặc ứng dụng, phân biệt không khớp với thư viện rỗng", async () => {
  const user = userEvent.setup();
  render(<SavedTasksPage devices={[]} />);
  expect(await screen.findByText("Nuôi tài khoản")).toBeInTheDocument();
  const search = screen.getByRole("searchbox", { name: "Tìm tác vụ đã lưu" });

  await user.type(search, "NUÔI");
  expect(screen.getByText("Nuôi tài khoản")).toBeInTheDocument();
  expect(screen.queryByText("Đăng video")).not.toBeInTheDocument();

  await user.clear(search);
  await user.type(search, "kho nội dung");
  expect(screen.queryByText("Nuôi tài khoản")).not.toBeInTheDocument();
  expect(screen.getByText("Đăng video")).toBeInTheDocument();

  await user.clear(search);
  await user.type(search, "không-khớp");
  expect(screen.getByText("Không có tác vụ khớp tìm kiếm")).toBeInTheDocument();
  expect(screen.queryByText("Chưa có tác vụ đã lưu")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Xóa tìm kiếm" }));
  expect(screen.getByText("Nuôi tài khoản")).toBeInTheDocument();
  expect(screen.getByText("Đăng video")).toBeInTheDocument();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["operator_list", "app_workflow_list"]);
});
