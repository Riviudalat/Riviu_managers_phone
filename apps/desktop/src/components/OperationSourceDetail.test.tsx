import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OperationSourceDetail } from "./OperationSourceDetail";
import type { OperationRunDetail } from "../types";
const read = vi.hoisted(() => vi.fn());
vi.mock("../api", () => ({ operationGetRun: read }));
const source = { operationId: "nurture:old", sourceId: "old", kind: "nurture" as const };
const detail = { summary: { id: source.operationId, sourceId: source.sourceId, kind: source.kind, title: "Phiên cũ", state: "succeeded" }, items: [] } as unknown as OperationRunDetail;
beforeEach(() => read.mockReset());
describe("OperationSourceDetail", () => {
  it("loads the exact durable run rather than the latest session", async () => {
    read.mockResolvedValue(detail);
    render(<OperationSourceDetail source={source} />);
    expect(await screen.findByText("Phiên cũ")).toBeVisible();
    expect(read).toHaveBeenCalledWith("nurture:old");
  });
  it.each([null, { ...detail, summary: { ...detail.summary, id: "nurture:other" } }, { ...detail, summary: { ...detail.summary, kind: "flow" } }])("reports missing or mismatched source without substituting another run", async (result) => {
    read.mockResolvedValue(result);
    render(<OperationSourceDetail source={source} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Tác vụ được chọn không còn trong nguồn dữ liệu.");
    expect(screen.queryByText("Phiên cũ")).toBeNull();
  });
  it("ignores a source response that arrives after navigation to another run", async () => {
    let resolve!: (value: OperationRunDetail) => void;
    read.mockReturnValueOnce(new Promise((done) => { resolve = done; })).mockResolvedValueOnce({ ...detail, summary: { ...detail.summary, id: "nurture:new", sourceId: "new", title: "Phiên mới" } });
    const view = render(<OperationSourceDetail source={source} />);
    await waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    view.rerender(<OperationSourceDetail source={{ ...source, operationId: "nurture:new", sourceId: "new" }} />);
    expect(await screen.findByText("Phiên mới")).toBeVisible();
    await act(async () => resolve(detail));
    expect(screen.queryByText("Phiên cũ")).toBeNull();
  });
});
