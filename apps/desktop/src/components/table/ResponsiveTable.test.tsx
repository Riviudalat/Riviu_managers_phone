import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ResponsiveTable } from "./ResponsiveTable";
import { readTableView } from "./tableView";

const rows = [{ id: "2", name: "Máy 2", state: "Sẵn sàng" }, { id: "10", name: "Máy 10", state: "Đang bận" }];
const columns = [
  { id: "name", label: "Thiết bị", render: (row: typeof rows[number]) => row.name, sortValue: (row: typeof rows[number]) => row.name },
  { id: "state", label: "Trạng thái", render: (row: typeof rows[number]) => row.state },
];
function table(onOpen = vi.fn(), viewKey = "test") {
  return <ResponsiveTable label="Máy" viewKey={viewKey} rows={rows} columns={columns} keyForRow={(row) => row.id} onRowOpen={onOpen} searchText={(row) => row.name} />;
}
afterEach(() => { cleanup(); localStorage.clear(); });
describe("table view preferences", () => {
  it("filters without dispatching a row action or storing the query", async () => {
    const open = vi.fn(); render(table(open));
    await userEvent.type(screen.getByRole("searchbox"), "Máy 10");
    expect(screen.queryByText("Máy 2")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("1/2");
    expect(open).not.toHaveBeenCalled();
    expect(localStorage.length).toBe(0);
  });
  it("sorts naturally and persists column visibility without hiding identity", async () => {
    render(table());
    await userEvent.click(screen.getByRole("button", { name: "Thiết bị" }));
    await userEvent.click(screen.getByRole("button", { name: "Thiết bị" }));
    expect(within(screen.getByRole("table")).getAllByRole("row")[1]).toHaveTextContent("Máy 10");
    await userEvent.click(screen.getByLabelText("Chọn cột: Máy"));
    expect(screen.getByRole("checkbox", { name: "Thiết bị" })).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: "Trạng thái" }));
    cleanup(); render(table());
    expect(screen.queryByRole("columnheader", { name: "Trạng thái" })).toBeNull();
    expect(screen.getByRole("columnheader", { name: "Thiết bị" })).toHaveAttribute("aria-sort", "descending");
  });
  it("ignores corrupted stored preferences", () => {
    localStorage.setItem("riviu.table.test.v1", '{"hidden":17,"sort":{"column":7}}');
    expect(readTableView("test")).toEqual({ hidden: [], sort: null });
  });
  it("closes column disclosure with Escape and restores keyboard focus", async () => {
    const parentKeyDown = vi.fn();
    render(<div onKeyDown={parentKeyDown}>{table()}</div>);
    const trigger = screen.getByLabelText("Chọn cột: Máy");
    await userEvent.click(trigger);
    screen.getByRole("checkbox", { name: "Trạng thái" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(trigger.parentElement).not.toHaveAttribute("open");
    expect(trigger).toHaveFocus();
    expect(parentKeyDown).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    expect(parentKeyDown).toHaveBeenCalledOnce();
  });
  it("loads the new view scope and clears transient search when its key changes", async () => {
    const alpha = { hidden: ["state"], sort: { column: "name", direction: "asc" } };
    localStorage.setItem("riviu.table.alpha.v1", JSON.stringify(alpha));
    localStorage.setItem("riviu.table.beta.v1", JSON.stringify({ hidden: [], sort: { column: "name", direction: "desc" } }));
    const { rerender } = render(table(vi.fn(), "alpha"));
    await userEvent.type(screen.getByRole("searchbox"), "Máy 2");
    expect(screen.queryByRole("columnheader", { name: "Trạng thái" })).toBeNull();

    rerender(table(vi.fn(), "beta"));

    expect(screen.getByRole("searchbox")).toHaveValue("");
    expect(screen.getByRole("columnheader", { name: "Trạng thái" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Thiết bị" })).toHaveAttribute("aria-sort", "descending");
    expect(within(screen.getByRole("table")).getAllByRole("row")[1]).toHaveTextContent("Máy 10");
    await userEvent.click(screen.getByRole("button", { name: "Thiết bị" }));
    expect(readTableView("beta")).toEqual({ hidden: [], sort: { column: "name", direction: "asc" } });
    expect(readTableView("alpha")).toEqual(alpha);
  });
  it("respects an explicitly suppressed empty state during loading or failure", () => {
    const { rerender } = render(<ResponsiveTable label="Máy" viewKey="test" rows={[]} columns={columns} keyForRow={(row) => row.id} empty={null} />);
    expect(screen.queryByText("Chưa có dữ liệu")).toBeNull();
    rerender(<ResponsiveTable label="Máy" viewKey="test" rows={[]} columns={columns} keyForRow={(row) => row.id} />);
    expect(screen.getByText("Chưa có dữ liệu")).toBeInTheDocument();
  });
});
