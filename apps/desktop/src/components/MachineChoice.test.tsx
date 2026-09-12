import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MachineChoice } from "./MachineChoice";

afterEach(cleanup);

describe("machine selection status", () => {
  it("shows readiness as visible text while preserving the checkbox name", () => {
    const change = vi.fn();
    render(<MachineChoice number={4} name="Pixel" status="ready" checked={false} label="Chọn Máy 4" onChange={change} />);
    expect(screen.getByText("Sẵn sàng")).toBeVisible();
    fireEvent.click(screen.getByRole("checkbox", { name: "Chọn Máy 4" }));
    expect(change).toHaveBeenCalledWith(true);
  });

  it("shows the unavailable reason and allows removing an already selected machine", () => {
    const change = vi.fn();
    render(<MachineChoice number={4} name="Pixel" status="error" reason="Kết nối thiết bị đã ngắt" checked label="Chọn Máy 4" onChange={change} />);
    expect(screen.getByText("Cần kiểm tra")).toBeVisible();
    expect(screen.getByText("Kết nối thiết bị đã ngắt")).toBeVisible();
    fireEvent.click(screen.getByRole("checkbox", { name: "Chọn Máy 4" }));
    expect(change).toHaveBeenCalledWith(false);
  });

  it("keeps account actions separate from selection on an unavailable machine", () => {
    const change = vi.fn();
    const account = vi.fn();
    render(<MachineChoice number={4} name="Pixel" status="disconnected" checked={false} disabled label="Chọn Máy 4" onChange={change} detail={<button onClick={account}>Gán tài khoản</button>} />);
    expect(screen.getByText("Mất kết nối")).toBeVisible();
    expect(screen.getByRole("checkbox")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Gán tài khoản" }));
    expect(account).toHaveBeenCalledOnce();
    expect(change).not.toHaveBeenCalled();
  });
});
