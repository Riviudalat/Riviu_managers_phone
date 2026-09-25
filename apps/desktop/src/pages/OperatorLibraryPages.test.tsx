import { StrictMode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MyAppsPage } from "./MyAppsPage";
import { SavedTasksPage } from "./SavedTasksPage";
import { OperatorRecordsPage } from "./OperatorRecordsPage";
import { OperatorSchedulesPage } from "./OperatorSchedulesPage";
import type { AppWorkflowSummary, AppWorkflowV1 } from "../appWorkflow";
import type { OperatorRecord } from "../operatorRecords";
import type { AutomationDefinition, AutomationSchedule, DeviceInfo } from "../types";

const { invoke, confirm } = vi.hoisted(() => ({ invoke: vi.fn(), confirm: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../confirmStore", () => ({ requestConfirm: confirm }));
// Editor có bộ test riêng; ở đây chỉ quan sát tài liệu đã qua parser/validate.
vi.mock("../components/AppWorkflowEditor", () => ({
  AppWorkflowEditor: ({ initial }: { initial: AppWorkflowV1 }) => (
    <section aria-label="Tài liệu nhập"><output>{JSON.stringify(initial)}</output></section>
  ),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const date = "2026-09-17T08:00:00Z";
const workflow: AppWorkflowSummary = {
  id: "app-1", name: "Quy trình kiểm thử", kind: "nurture", latestRevision: 7, updatedAt: date,
};
const account: OperatorRecord = {
  id: "account-1", kind: "account", name: "Tài khoản kiểm thử", revision: 4,
  data: { username: "test-account", platform: "tiktok", group: "Nhóm thử", deviceIds: [], credentialRef: "", notes: "" },
  archived: false, createdAt: date, updatedAt: date,
};
const task: OperatorRecord = {
  ...account, id: "task-1", kind: "savedTask", name: "Tác vụ kiểm thử",
  data: { appId: "app-1", appRevision: 3, target: { type: "explicit", udids: ["phone-test"] }, inputs: {} },
};
const taskDevice: DeviceInfo = {
  udid: "phone-test", name: "Máy thử", model: "Fixture", platform: "android",
  osVersion: "15", connection: "mock", status: "ready", wdaReady: false,
};
const profile: AutomationDefinition = {
  id: "profile-1", name: "Hồ sơ kiểm thử", kind: "nurture", latestRevision: 8,
  archived: false, createdAt: date, updatedAt: date,
};
const schedule: AutomationSchedule = {
  id: "schedule-1", revision: 5, name: "Lịch kiểm thử", definitionId: profile.id,
  definitionRevision: 2, enabled: true,
  schedule: { schemaVersion: 1, kind: "interval", everyMinutes: 60 },
  nextDueAt: date, lastErrorCode: "Uncertain", createdAt: date, updatedAt: date,
};
const readonlyCommands = ["app_workflow_list", "operator_list", "automation_list", "automation_schedule_list", "flow_connector_info"];
function defaultRead(command: string) {
  if (command === "app_workflow_list") return [workflow];
  if (command === "automation_list") return [profile];
  if (command === "flow_connector_info") return { credentialNames: [], root: "C:/fixture/flow-data", sheetConfigured: false };
  if (readonlyCommands.includes(command)) return [];
  throw new Error(`IPC ngoài phạm vi đọc: ${command}`);
}
function listRead(command: string, read: () => Promise<unknown>) {
  invoke.mockImplementation(async (name: string) => name === command ? read() : defaultRead(name));
}
beforeEach(() => {
  invoke.mockReset().mockImplementation(async (command: string) => defaultRead(command));
  confirm.mockReset().mockResolvedValue(false);
});
afterEach(() => vi.restoreAllMocks());

const pages = [
  {
    name: "My Apps", render: () => <MyAppsPage devices={[]} onOpenApp={() => undefined} />,
    command: "app_workflow_list", refresh: "Làm mới ứng dụng",
    loading: "Đang tải quy trình đã lưu…", refreshing: "Đang làm mới quy trình đã lưu…",
    empty: "Chưa có quy trình đã lưu", zero: /^3 ứng dụng$/, row: workflow,
  },
  {
    name: "Tác vụ đã lưu", render: () => <SavedTasksPage devices={[]} />,
    command: "operator_list", refresh: "Làm mới tác vụ đã lưu",
    loading: "Đang tải tác vụ đã lưu…", refreshing: "Đang làm mới tác vụ đã lưu…",
    empty: "Chưa có tác vụ đã lưu", zero: /^0 tác vụ đã lưu$/, row: task,
  },
  {
    name: "Bản ghi", render: () => <OperatorRecordsPage kind="account" devices={[]} />,
    command: "operator_list", refresh: "Làm mới bản ghi",
    loading: "Đang tải bản ghi…", refreshing: "Đang làm mới bản ghi…",
    empty: "Chưa có bản ghi", zero: /^0 bản ghi$/, row: account,
  },
  {
    name: "Lịch chạy", render: () => <OperatorSchedulesPage />,
    command: "automation_schedule_list", refresh: "Làm mới lịch chạy",
    loading: "Đang tải lịch chạy…", refreshing: "Đang làm mới lịch chạy…",
    empty: "Chưa có lịch", zero: /^0 lịch chạy$/, row: schedule,
  },
];

describe.each(pages)("$name — trạng thái đọc", (page) => {
  it("không báo rỗng hoặc số 0 khi lần đọc đầu đang chờ", async () => {
    const pending = deferred<unknown[]>();
    listRead(page.command, () => pending.promise);
    render(page.render());
    expect(screen.getByRole("status")).toHaveTextContent(page.loading);
    expect(screen.queryByText(page.empty)).not.toBeInTheDocument();
    expect(screen.queryByText(page.zero)).not.toBeInTheDocument();
    await act(async () => pending.resolve([]));
    expect(screen.getByText(page.empty)).toBeInTheDocument();
  });

  it("lỗi lần đọc đầu có thử lại, không giả danh sách rỗng", async () => {
    const first = deferred<unknown[]>(), retry = deferred<unknown[]>();
    const read = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(retry.promise);
    listRead(page.command, read);
    render(page.render());
    await act(async () => first.reject(new Error("Không đọc được thư viện")));
    expect(screen.getByRole("alert")).toHaveTextContent("Không đọc được thư viện");
    expect(screen.queryByText(page.empty)).not.toBeInTheDocument();
    expect(screen.queryByText(page.zero)).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    expect(screen.getByRole("status")).toHaveTextContent(page.loading);
    await act(async () => retry.resolve([page.row]));
    expect(screen.getByText(page.row.name)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("refresh giữ hàng cũ, báo lỗi cạnh bảng và phục hồi qua thử lại", async () => {
    const refresh = deferred<unknown[]>(), retry = deferred<unknown[]>();
    const read = vi.fn().mockResolvedValueOnce([page.row])
      .mockReturnValueOnce(refresh.promise).mockReturnValueOnce(retry.promise);
    listRead(page.command, read);
    render(page.render());
    expect(await screen.findByText(page.row.name)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: page.refresh }));
    expect(screen.getByText(page.row.name)).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(page.refreshing);
    await act(async () => refresh.reject(new Error("Lần làm mới thất bại")));
    expect(screen.getByText(page.row.name)).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Lần làm mới thất bại");
    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    await act(async () => retry.resolve([]));
    expect(screen.queryByText(page.row.name)).not.toBeInTheDocument();
    expect(screen.getByText(page.empty)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(invoke.mock.calls.every(([command]) => readonlyCommands.includes(command))).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("response cũ không ghi đè danh sách mới khi effect chạy lại", async () => {
    const older = deferred<unknown[]>(), newer = deferred<unknown[]>();
    const read = vi.fn().mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    listRead(page.command, read);
    render(<StrictMode>{page.render()}</StrictMode>);
    if (page.command === "automation_schedule_list") {
      expect(read).toHaveBeenCalledTimes(1);
      await act(async () => older.resolve([page.row]));
      expect(screen.getByText(page.row.name)).toBeInTheDocument();
      return;
    }
    expect(read).toHaveBeenCalledTimes(2);
    await act(async () => newer.resolve([page.row]));
    expect(screen.getByText(page.row.name)).toBeInTheDocument();
    await act(async () => older.resolve([{ ...page.row, id: "stale", name: "Hàng đã lỗi thời" }]));
    expect(screen.getByText(page.row.name)).toBeInTheDocument();
    expect(screen.queryByText("Hàng đã lỗi thời")).not.toBeInTheDocument();
  });

  it("reject và finally cũ không xoá loading hay gắn lỗi cho lượt đọc mới", async () => {
    const older = deferred<unknown[]>(), newer = deferred<unknown[]>();
    const read = vi.fn().mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    listRead(page.command, read);
    render(<StrictMode>{page.render()}</StrictMode>);
    await act(async () => older.reject(new Error("Lỗi của lượt cũ")));
    if (page.command === "automation_schedule_list") {
      expect(read).toHaveBeenCalledTimes(1);
      expect(screen.getByRole("alert")).toHaveTextContent("Lỗi của lượt cũ");
      await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));
      await act(async () => newer.resolve([page.row]));
      expect(screen.getByText(page.row.name)).toBeInTheDocument();
      return;
    }
    expect(screen.getByRole("status")).toHaveTextContent(page.loading);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.queryByText(page.empty)).not.toBeInTheDocument();
    await act(async () => newer.resolve([page.row]));
    expect(screen.getByText(page.row.name)).toBeInTheDocument();
  });
});

it("My Apps giữ ba ứng dụng cục bộ khi đọc quy trình đang chờ hoặc thất bại", async () => {
  const pending = deferred<unknown[]>(), open = vi.fn();
  listRead("app_workflow_list", () => pending.promise);
  render(<MyAppsPage devices={[]} onOpenApp={open} />);
  for (const name of ["Nuôi TikTok", "Tương tác", "Đăng bài"]) {
    expect(screen.getByRole("button", { name })).toBeEnabled();
  }
  expect(screen.queryByText(/^3 ứng dụng$/)).not.toBeInTheDocument();
  await act(async () => pending.reject(new Error("Mất kết nối thư viện")));
  await userEvent.click(screen.getByRole("button", { name: "Nuôi TikTok" }));
  expect(open).toHaveBeenCalledWith("nurture");
  expect(screen.getByRole("alert")).toHaveTextContent("Mất kết nối thư viện");
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["app_workflow_list"]);
});

it.each([
  { name: "My Apps", render: pages[0].render, search: "Tìm ứng dụng", empty: "Chưa có quy trình đã lưu", command: "app_workflow_list", row: workflow },
  { name: "Bản ghi", render: pages[2].render, search: "Tìm bản ghi", empty: "Chưa có bản ghi", command: "operator_list", row: account },
])("$name phân biệt không khớp bộ lọc với thư viện rỗng", async (page) => {
  listRead(page.command, async () => [page.row]);
  render(page.render());
  await screen.findByText(page.row.name);
  await userEvent.type(screen.getByRole("searchbox", { name: page.search }), "không-khớp");
  expect(screen.getByText(/Không có .* khớp/)).toBeInTheDocument();
  expect(screen.queryByText(page.empty)).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Xóa tìm kiếm" }));
  expect(screen.getByText(page.row.name)).toBeInTheDocument();
});

it.each([
  { name: "My Apps", render: pages[0].render, button: "Nhập ứng dụng từ JSON" },
  { name: "Bản ghi", render: pages[2].render, button: "Nhập bản ghi từ JSON" },
])("$name cho phép dùng Tab, Enter và Space để mở input JSON", async (page) => {
  const user = userEvent.setup();
  const { container } = render(page.render());
  const button = screen.getByRole("button", { name: page.button });
  const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
  const click = vi.spyOn(input, "click").mockImplementation(() => undefined);
  for (let i = 0; i < 12 && document.activeElement !== button; i++) await user.tab();
  expect(button).toHaveFocus();
  await user.keyboard("{Enter}");
  await user.keyboard(" ");
  expect(click).toHaveBeenCalledTimes(2);
  expect(input.accept).toBe(".json");
  expect(invoke.mock.calls.every(([command]) => readonlyCommands.includes(command))).toBe(true);
});

it("đổi kind không hiển thị bản ghi hoặc credential của kind cũ, kể cả response muộn", async () => {
  const oldRead = deferred<OperatorRecord[]>(), networkRead = deferred<OperatorRecord[]>();
  const oldSecrets = deferred<unknown>(), networkSecrets = deferred<unknown>();
  let accountReads = 0, secretReads = 0;
  invoke.mockImplementation(async (command: string, args?: { kind?: string }) => {
    if (command === "operator_list") {
      if (args?.kind === "network") return networkRead.promise;
      return accountReads++ === 0 ? [account] : oldRead.promise;
    }
    if (command === "flow_connector_info") return secretReads++ === 0 ? oldSecrets.promise : networkSecrets.promise;
    return defaultRead(command);
  });
  const { rerender } = render(<OperatorRecordsPage kind="account" devices={[]} />);
  await screen.findByText(account.name);
  await userEvent.click(screen.getByRole("button", { name: "Làm mới bản ghi" }));
  rerender(<OperatorRecordsPage kind="network" devices={[]} />);
  expect(screen.queryByText(account.name)).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Đang tải bản ghi…");
  await act(async () => oldRead.reject(new Error("Lỗi tài khoản cũ")));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Đang tải bản ghi…");
  await act(async () => {
    networkRead.resolve([{ ...account, id: "network-1", kind: "network", name: "Kết nối mới", data: { host: "127.0.0.1", protocol: "http", port: 8080, deviceIds: [], credentialRef: "" } }]);
    networkSecrets.resolve({ credentialNames: ["Khóa mới"], root: "C:/fixture/flow-data", sheetConfigured: false });
  });
  await userEvent.click(screen.getByRole("button", { name: "Thêm kết nối" }));
  expect(screen.getByRole("option", { name: "Khóa mới" })).toBeInTheDocument();
  await act(async () => oldSecrets.resolve({ credentialNames: ["Khóa cũ"], root: "C:/fixture/flow-data", sheetConfigured: false }));
  expect(screen.queryByRole("option", { name: "Khóa cũ" })).not.toBeInTheDocument();
  expect(screen.getByText("Kết nối mới")).toBeInTheDocument();
  expect(invoke.mock.calls.every(([command]) => readonlyCommands.includes(command))).toBe(true);
});

it("refresh bản ghi giữ nguyên nháp và không tự đọc tài khoản hay áp dụng mạng", async () => {
  listRead("operator_list", async () => [account]);
  render(<OperatorRecordsPage kind="account" devices={[]} />);
  await screen.findByText(account.name);
  await userEvent.click(screen.getByRole("button", { name: "Chỉnh sửa" }));
  await userEvent.type(screen.getByRole("textbox", { name: "Tên" }), " sửa nháp");
  await userEvent.click(screen.getByRole("button", { name: "Làm mới bản ghi" }));
  expect(screen.getByRole("textbox", { name: "Tên" })).toHaveValue("Tài khoản kiểm thử sửa nháp");
  expect(invoke.mock.calls.every(([command]) => readonlyCommands.includes(command))).toBe(true);
});

it("editor tài khoản giữ xác nhận bỏ nháp và lưu bằng revision cũ", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "operator_list") return [account];
    if (command === "operator_save") return { ...account, revision: 5 };
    return defaultRead(command);
  });
  render(<OperatorRecordsPage kind="account" devices={[]} />);
  await screen.findByText(account.name);
  await userEvent.click(screen.getByRole("button", { name: "Chỉnh sửa" }));
  await userEvent.type(screen.getByRole("textbox", { name: "Tên" }), " mới");
  await userEvent.click(screen.getByRole("button", { name: "Đóng bản ghi" }));
  expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ title: "Bỏ thay đổi?" }));
  expect(screen.getByRole("textbox", { name: "Tên" })).toHaveValue("Tài khoản kiểm thử mới");
  await userEvent.click(screen.getByRole("button", { name: "Lưu bản ghi" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("operator_save", expect.objectContaining({
    input: expect.objectContaining({ id: account.id, expectedRevision: account.revision, name: "Tài khoản kiểm thử mới" }),
  })));
});

it("editor tác vụ giữ revision đã ghim khi lưu", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "operator_list") return [task];
    if (command === "operator_save") return { ...task, revision: 5 };
    return defaultRead(command);
  });
  render(<SavedTasksPage devices={[taskDevice]} />);
  await screen.findByText(task.name);
  await userEvent.click(screen.getByRole("button", { name: "Chỉnh sửa" }));
  await userEvent.click(screen.getByRole("button", { name: "Lưu tác vụ" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("operator_save", expect.objectContaining({
    input: expect.objectContaining({ id: task.id, expectedRevision: task.revision, name: task.name }),
  })));
});

it("nhập bản ghi vẫn từ chối sai kind và gửi đúng metadata khi JSON hợp lệ", async () => {
  invoke.mockImplementation(async (command: string) => command === "operator_import" ? [account] : defaultRead(command));
  const user = userEvent.setup({ applyAccept: false });
  const { container } = render(<OperatorRecordsPage kind="account" devices={[]} />);
  const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
  const file = (body: unknown) => {
    const value = new File([JSON.stringify(body)], "records.json", { type: "application/json" });
    Object.defineProperty(value, "text", { value: async () => JSON.stringify(body) });
    return value;
  };
  await user.upload(input, file({ schemaVersion: 1, kind: "network", records: [] }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Tệp không đúng loại dữ liệu");
  expect(invoke.mock.calls.some(([command]) => command === "operator_import")).toBe(false);
  await user.upload(input, file({ schemaVersion: 1, kind: "account", records: [{ name: account.name, data: account.data }] }));
  expect(await screen.findByText("Đã nhập 1 bản ghi")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith("operator_import", {
    inputs: [{ id: expect.any(String), kind: "account", expectedRevision: null, name: account.name, data: account.data }],
  });
  expect(input.value).toBe("");
});

it("nhập ứng dụng vẫn validate tài liệu trước khi mở bản sao chưa lưu", async () => {
  const doc: AppWorkflowV1 = {
    schemaVersion: 1, id: "original", revision: 9, name: "Ứng dụng nhập", kind: "nurture",
    entryNodeId: "start", nodes: [{ id: "start", action: "start", position: { x: 1, y: 2 }, config: {} }],
    edges: [], viewport: { x: 0, y: 0, zoom: 1 }, profileConfig: {},
  };
  const validation = deferred<void>();
  invoke.mockImplementation(async (command: string) => command === "app_workflow_validate" ? validation.promise : defaultRead(command));
  const { container } = render(pages[0].render());
  const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
  const file = new File([JSON.stringify(doc)], "app.json", { type: "application/json" });
  Object.defineProperty(file, "text", { value: async () => JSON.stringify(doc) });
  await userEvent.upload(input, file);
  expect(invoke).toHaveBeenCalledWith("app_workflow_validate", { document: doc });
  expect(screen.queryByRole("region", { name: "Tài liệu nhập" })).not.toBeInTheDocument();
  await act(async () => validation.resolve());
  const editor = await screen.findByRole("region", { name: "Tài liệu nhập" });
  expect(JSON.parse(editor.textContent!)).toEqual({ ...doc, id: expect.any(String), revision: 0 });
  expect(JSON.parse(editor.textContent!).id).not.toBe(doc.id);
  expect(invoke.mock.calls.some(([command]) => ["app_workflow_save", "app_workflow_run"].includes(command))).toBe(false);
});

it("tác vụ vẫn xác nhận trước Chạy/Lập lịch/Lưu trữ và dùng revision đã ghim", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "operator_list") return [task];
    if (command === "app_workflow_run") return { run: { id: "run-test" } };
    if (["app_workflow_schedule", "operator_archive"].includes(command)) return undefined;
    return defaultRead(command);
  });
  render(pages[1].render());
  await screen.findByText(task.name);
  const run = screen.getByRole("button", { name: "Chạy" });
  const makeSchedule = screen.getByRole("button", { name: "Lập lịch mỗi giờ" });
  const archive = screen.getByRole("button", { name: "Lưu trữ tác vụ" });
  for (const control of [run, makeSchedule, archive]) await userEvent.click(control);
  expect(confirm).toHaveBeenCalledTimes(3);
  expect(invoke.mock.calls.every(([command]) => readonlyCommands.includes(command))).toBe(true);
  confirm.mockResolvedValue(true);
  await userEvent.click(run);
  expect(invoke).toHaveBeenCalledWith("app_workflow_run", { id: "app-1", revision: 3, target: { type: "explicit", udids: ["phone-test"] } });
  await userEvent.click(makeSchedule);
  expect(invoke).toHaveBeenCalledWith("app_workflow_schedule", {
    id: "app-1", revision: 3, target: { type: "explicit", udids: ["phone-test"] }, name: task.name, everyMinutes: 60,
  });
  await userEvent.click(archive);
  expect(invoke).toHaveBeenCalledWith("operator_archive", { id: "task-1", expectedRevision: 4 });
});

it.each([
  { name: "Lưu trữ ứng dụng", render: pages[0].render, command: "app_workflow_archive", list: "app_workflow_list", row: workflow },
  { name: `Lưu trữ ${account.name}`, render: pages[2].render, command: "operator_archive", list: "operator_list", row: account },
  { name: "Tắt lịch", render: pages[3].render, command: "automation_schedule_update", list: "automation_schedule_list", row: schedule },
])("$name xóa lỗi thao tác cũ khi lần thử lại thành công", async (operation) => {
  let attempts = 0;
  confirm.mockResolvedValue(true);
  invoke.mockImplementation(async (command: string) => {
    if (command === operation.list) return [operation.row];
    if (command === operation.command) {
      if (attempts++ === 0) throw new Error("Thao tác chưa thành công");
      return undefined;
    }
    return defaultRead(command);
  });
  render(operation.render());
  await screen.findByText(operation.row.name);
  await userEvent.click(screen.getByRole("button", { name: operation.name }));
  expect(screen.getByRole("alert")).toHaveTextContent("Thao tác chưa thành công");
  await userEvent.click(screen.getByRole("button", { name: operation.name }));
  await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
});

it("bật/tắt lịch giữ revision, chu kỳ và kết quả gần nhất thay vì nâng thành thành công", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "automation_schedule_list") return [schedule];
    if (command === "automation_schedule_update") return { ...schedule, enabled: false };
    return defaultRead(command);
  });
  render(pages[3].render());
  await screen.findByText(schedule.name);
  expect(screen.getByText("Uncertain")).toBeInTheDocument();
  expect(screen.getByText(new Date(date).toLocaleString("vi-VN"))).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Tắt lịch" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("automation_schedule_update", {
    scheduleId: "schedule-1", expectedRevision: 5, name: schedule.name, definitionId: "profile-1",
    definitionRevision: 2, enabled: false, schedule: { schemaVersion: 1, kind: "interval", everyMinutes: 60 },
  }));
  expect(confirm).not.toHaveBeenCalled();
});

it("lịch đặt bảng và cấu hình trong hai vùng riêng, chọn hàng mở đúng hồ sơ", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "automation_schedule_list") return [schedule];
    return defaultRead(command);
  });
  render(<OperatorSchedulesPage />);
  const list = await screen.findByRole("region", { name: "Danh sách lịch" });
  const detail = screen.getByRole("region", { name: "Cấu hình lịch" });
  expect(list).toContainElement(screen.getByText(schedule.name));
  expect(detail).toContainElement(screen.getByRole("combobox", { name: "Cấu hình ứng dụng" }));
  await userEvent.click(screen.getByRole("button", { name: "Chỉnh sửa" }));
  expect(screen.getByRole("combobox", { name: "Cấu hình ứng dụng" })).toHaveValue(profile.id);
  expect(detail).toContainElement(await screen.findByRole("region", { name: "Lịch tự động" }));
});
