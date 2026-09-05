import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLibraryBatch } from "./useLibraryBatch";
const query = vi.hoisted(() => vi.fn());
const read = vi.hoisted(() => vi.fn());
vi.mock("./api", () => ({ operationGetRun: read, operationQueryRuns: query }));
beforeEach(() => { query.mockReset(); read.mockReset(); });
describe("exact library batch history", () => {
  it("reads a historical batch without the 24-hour latest query", async () => {
    read.mockResolvedValue({ summary: { id: "materialTransfer:old", kind: "materialTransfer" }, items: [] });
    const { result } = renderHook(() => useLibraryBatch("materialTransfer", "materialTransfer:old"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(read).toHaveBeenCalledWith("materialTransfer:old");
    expect(query).not.toHaveBeenCalled();
    expect(result.current.detail?.summary.id).toBe("materialTransfer:old");
  });
  it("does not substitute an unrelated kind for a missing batch", async () => {
    read.mockResolvedValue({ summary: { id: "materialTransfer:old", kind: "appInstall" }, items: [] });
    const { result } = renderHook(() => useLibraryBatch("materialTransfer", "materialTransfer:old"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.detail).toBeNull();
    expect(result.current.error).toContain("không còn trong nguồn dữ liệu");
  });
  it("follows a new retry batch without returning to the old historical source", async () => {
    read.mockImplementation(async (id) => ({ summary: { id, kind: "materialTransfer" }, items: [] }));
    const { result, rerender } = renderHook(({ id }) => useLibraryBatch("materialTransfer", id), { initialProps: { id: "materialTransfer:old" } });
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.follow("materialTransfer:retry"));
    await waitFor(() => expect(result.current.detail?.summary.id).toBe("materialTransfer:retry"));
    rerender({ id: "materialTransfer:different" });
    await waitFor(() => expect(result.current.detail?.summary.id).toBe("materialTransfer:different"));
  });
});
