import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import {
  DetailDrawer,
  FormSection,
  PageHeader,
  ResponsiveTable,
  StatusChip,
  SummaryRail,
  WorkflowStepper,
  WorkspaceTabs,
} from "./WorkspacePrimitives";

describe("production workspace primitives", () => {
  it("renders one page heading with real metadata and actions", () => {
    render(
      <PageHeader
        title="Đăng bài"
        description="Một chiến dịch đang theo dõi"
        density="compact"
        meta={<StatusChip tone="warning">Partial</StatusChip>}
        actions={<button type="button">Làm mới</button>}
      />,
    );

    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("heading", { name: "Đăng bài" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "Đăng bài" }).closest("header")).toHaveClass(
      "is-compact",
    );
    expect(screen.getByText("Partial")).toHaveClass("is-warning");
    expect(screen.getByRole("button", { name: "Làm mới" })).toBeVisible();
  });

  it("uses the horizontal keyboard pattern for workspace tabs", async () => {
    function Fixture() {
      const [active, setActive] = useState("setup");
      return (
        <WorkspaceTabs
          label="Chế độ Nuôi TikTok"
          value={active}
          onChange={setActive}
          tabs={[
            { id: "setup", label: "Thiết lập" },
            { id: "blocked", label: "Đang khóa", disabled: true },
            { id: "monitor", label: "Theo dõi" },
          ]}
        />
      );
    }
    render(<Fixture />);

    const setup = screen.getByRole("tab", { name: "Thiết lập" });
    const monitor = screen.getByRole("tab", { name: "Theo dõi" });
    setup.focus();
    fireEvent.keyDown(setup, { key: "ArrowRight" });

    await waitFor(() => expect(monitor).toHaveAttribute("aria-selected", "true"));
    expect(monitor).toHaveFocus();
    fireEvent.keyDown(monitor, { key: "Home" });
    await waitFor(() => expect(setup).toHaveFocus());
  });

  it("announces the current workflow step and preserves explicit outcomes", () => {
    render(
      <WorkflowStepper
        current="check"
        steps={[
          { id: "scope", label: "Phạm vi" },
          { id: "check", label: "Kiểm tra" },
          { id: "run", label: "Theo dõi", state: "warning" },
        ]}
      />,
    );

    expect(screen.getByText("Kiểm tra").closest("li")).toHaveAttribute("aria-current", "step");
    expect(screen.getByText("Theo dõi").closest("li")).toHaveClass("is-warning");
  });

  it("opens a table row through Enter without hiding the table label", async () => {
    const onOpen = vi.fn();
    render(
      <ResponsiveTable
        label="Tác vụ gần đây"
        columns={[{ id: "name", label: "Tên", render: (row: { name: string }) => row.name }]}
        rows={[{ name: "Nuôi Máy 2" }]}
        keyForRow={(row) => row.name}
        labelForRow={(row) => `Mở ${row.name}`}
        onRowOpen={onOpen}
      />,
    );

    expect(screen.getByRole("table", { name: "Tác vụ gần đây" })).toBeVisible();
    const row = screen.getByRole("row", { name: "Mở Nuôi Máy 2" });
    row.focus();
    await userEvent.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("does not turn a row action into a second row-open action", async () => {
    const onOpen = vi.fn();
    const onAction = vi.fn();
    render(
      <ResponsiveTable
        label="Thiết bị"
        columns={[
          { id: "name", label: "Máy", render: (row: { name: string }) => row.name },
          {
            id: "action",
            label: "Thao tác",
            render: () => (
              <button type="button" onClick={onAction}>
                Chi tiết
              </button>
            ),
          },
        ]}
        rows={[{ name: "Máy 2" }]}
        keyForRow={(row) => row.name}
        onRowOpen={onOpen}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Chi tiết" }));
    expect(onAction).toHaveBeenCalledOnce();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("closes the detail drawer with Escape and restores focus", async () => {
    const onClose = vi.fn();
    const { rerender } = render(<button type="button">Mở chi tiết</button>);
    const trigger = screen.getByRole("button", { name: "Mở chi tiết" });
    trigger.focus();
    rerender(
      <>
        <button type="button">Mở chi tiết</button>
        <DetailDrawer open title="Máy 2" onClose={onClose}>
          <button type="button">Thao tác</button>
        </DetailDrawer>
      </>,
    );

    expect(screen.getByRole("button", { name: "Đóng" })).toHaveFocus();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
    rerender(<button type="button">Mở chi tiết</button>);
    await waitFor(() => expect(screen.getByRole("button", { name: "Mở chi tiết" })).toHaveFocus());
  });

  it("keeps the operator's focus when a parent replaces the close callback", () => {
    const { rerender } = render(
      <DetailDrawer open title="Máy 2" onClose={() => undefined}>
        <button type="button">Thao tác</button>
      </DetailDrawer>,
    );
    const action = screen.getByRole("button", { name: "Thao tác" });
    action.focus();

    rerender(
      <DetailDrawer open title="Máy 2" onClose={() => undefined}>
        <button type="button">Thao tác</button>
      </DetailDrawer>,
    );

    expect(action).toHaveFocus();
  });

  it("labels summary and form regions without inventing values", () => {
    render(
      <>
        <SummaryRail title="Kết quả thật">Chưa có phiên chạy</SummaryRail>
        <FormSection title="Hành động" description="Chọn trước khi kiểm tra">
          <label>
            <input type="checkbox" /> Lưu
          </label>
        </FormSection>
      </>,
    );

    expect(screen.getByRole("complementary", { name: "Kết quả thật" })).toHaveTextContent(
      "Chưa có phiên chạy",
    );
    expect(screen.getByRole("region", { name: "Hành động" })).toBeVisible();
  });
});
