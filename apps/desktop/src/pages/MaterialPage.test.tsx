import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MaterialPage } from "./MaterialPage";
import type { MaterialItem } from "../types";

const listMaterials = vi.hoisted(() => vi.fn());
const listGroups = vi.hoisted(() => vi.fn());
const flashError = vi.hoisted(() => vi.fn());

vi.mock("../api", () => ({
  addMaterial: vi.fn(async () => undefined),
  deleteMaterial: vi.fn(async () => undefined),
  listGroups,
  listMaterials,
  pushMaterial: vi.fn(async () => "Đã chuyển"),
}));

vi.mock("../pickFile", () => ({ pickMaterial: vi.fn(async () => null) }));
vi.mock("../farmToast", () => ({ flash: vi.fn(), flashError }));

const material: MaterialItem = {
  id: "material-1",
  name: "video-01.mp4",
  path: "C:/media/video-01.mp4",
  kind: "video",
  size: 2048,
  createdAt: "2026-09-01T00:00:00.000Z",
};

beforeEach(() => {
  listMaterials.mockReset();
  listGroups.mockReset();
  listGroups.mockResolvedValue([]);
});

function renderPage() {
  return render(<MaterialPage devices={[]} selected={[]} onSelectUdids={() => undefined} />);
}

describe("MaterialPage list states", () => {
  it("does not call the list empty while it is still loading", async () => {
    listMaterials.mockResolvedValue([]);

    renderPage();

    expect(screen.getByRole("status")).toHaveTextContent("Đang tải kho nội dung");
    expect(screen.queryByText("Chưa có nội dung")).toBeNull();
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    expect(await screen.findByText("Chưa có nội dung")).toBeInTheDocument();
  });

  it("keeps a load failure inline and retries without an error toast state", async () => {
    listMaterials
      .mockRejectedValueOnce(new Error("Không đọc được thư mục media"))
      .mockResolvedValueOnce([material]);

    renderPage();

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Không đọc được thư mục media",
    );
    expect(flashError).not.toHaveBeenCalled();
    expect(screen.queryByText("Chưa có nội dung")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));

    await waitFor(() => expect(listMaterials).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("video-01.mp4")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps the newest material list when StrictMode responses arrive out of order", async () => {
    let resolveFirst!: (value: MaterialItem[]) => void;
    let resolveSecond!: (value: MaterialItem[]) => void;
    listMaterials
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }))
      .mockReturnValueOnce(new Promise((resolve) => { resolveSecond = resolve; }));

    render(<StrictMode><MaterialPage devices={[]} selected={[]} onSelectUdids={() => undefined} /></StrictMode>);
    await waitFor(() => expect(listMaterials).toHaveBeenCalledTimes(2));
    resolveSecond([material]);
    expect(await screen.findByText("video-01.mp4")).toBeInTheDocument();
    resolveFirst([{ ...material, id: "old", name: "old.mp4" }]);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(screen.getByText("video-01.mp4")).toBeInTheDocument();
    expect(screen.queryByText("old.mp4")).toBeNull();
  });
});
