import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MaterialPage } from "./MaterialPage";
import type { DeviceInfo, MaterialItem, MaterialPushBatchResult } from "../types";

const listMaterials = vi.hoisted(() => vi.fn());
const listGroups = vi.hoisted(() => vi.fn());
const flashError = vi.hoisted(() => vi.fn());
const pushMaterialBatch = vi.hoisted(() => vi.fn());

vi.mock("../api", () => ({
  addMaterial: vi.fn(async () => undefined),
  deleteMaterial: vi.fn(async () => undefined),
  listGroups,
  listMaterials,
  pushMaterialBatch,
}));

vi.mock("../pickFile", () => ({ pickMaterial: vi.fn(async () => null) }));
vi.mock("../farmToast", () => ({ flash: vi.fn(), flashError }));
vi.mock("../confirmStore", () => ({ requestConfirm: vi.fn(async () => true) }));

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
  pushMaterialBatch.mockReset();
});

function renderPage() {
  return render(<MaterialPage devices={[]} selected={[]} onSelectUdids={() => undefined} />);
}

describe("MaterialPage list states", () => {
  it("does not call the list empty while it is still loading", async () => {
    listMaterials.mockResolvedValue([]);

    renderPage();

    expect(screen.getByText("Đang tải kho nội dung…")).toBeInTheDocument();
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

  it("sends the whole selected target and renders isolated device results", async () => {
    listMaterials.mockResolvedValue([material]);
    const devices = [
      { udid: "phone-1", name: "Galaxy A", model: "A", platform: "android" },
      { udid: "phone-2", name: "Galaxy B", model: "B", platform: "android" },
    ] as DeviceInfo[];
    const result: MaterialPushBatchResult = {
      batchId: "batch-1",
      materialId: material.id,
      target: {
        targetRef: { type: "explicit", udids: ["phone-1", "phone-2"] },
        included: [
          { udid: "phone-1", alias: "", number: null },
          { udid: "phone-2", alias: "", number: null },
        ],
        excluded: [],
        rosterSha256: "a".repeat(64),
      },
      results: [
        { udid: "phone-1", status: "succeeded", evidence: "sha256=ok" },
        { udid: "phone-2", status: "failed", error: "device busy" },
      ],
    };
    pushMaterialBatch.mockResolvedValueOnce(result).mockResolvedValueOnce({
      ...result,
      batchId: "batch-2",
      target: {
        ...result.target,
        targetRef: { type: "explicit", udids: ["phone-2"] },
        included: [{ udid: "phone-2", alias: "Ca chiều", number: 2 }],
        excluded: [],
        rosterSha256: "c".repeat(64),
      },
      results: [{ udid: "phone-2", status: "succeeded", evidence: "sha256=ok" }],
    });
    const user = userEvent.setup();
    render(
      <MaterialPage
        devices={devices}
        selected={["phone-1", "phone-2"]}
        onSelectUdids={() => undefined}
      />,
    );

    await user.click(await screen.findByRole("button", { name: "Chuyển tới 2 máy" }));
    expect(pushMaterialBatch).toHaveBeenNthCalledWith(1, {
      materialId: material.id,
      target: { type: "explicit", udids: ["phone-1", "phone-2"] },
    });
    expect(await screen.findByText("Máy 1 · Galaxy A")).toBeVisible();
    expect(screen.getByText("Máy 2 · Galaxy B")).toBeVisible();
    expect(screen.getByText("Thất bại")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Thử lại 1 máy lỗi" }));
    expect(pushMaterialBatch).toHaveBeenNthCalledWith(2, {
      materialId: material.id,
      target: { type: "explicit", udids: ["phone-2"] },
    });
    expect(await screen.findByText("Ca chiều")).toBeVisible();
    expect(screen.queryByText("Máy 1 · Galaxy A")).toBeNull();
    expect(screen.queryByRole("button", { name: "Thử lại 1 máy lỗi" })).toBeNull();
  });

  it("resolves an empty selection as the whole fleet instead of one device", async () => {
    listMaterials.mockResolvedValue([material]);
    const devices = [
      { udid: "phone-1", name: "Galaxy A", model: "A", platform: "android" },
      { udid: "phone-2", name: "Galaxy B", model: "B", platform: "android" },
    ] as DeviceInfo[];
    pushMaterialBatch.mockResolvedValue({
      batchId: "batch-all",
      materialId: material.id,
      target: {
        targetRef: { type: "all" },
        included: [
          { udid: "phone-1", alias: "", number: null },
          { udid: "phone-2", alias: "", number: null },
        ],
        excluded: [],
        rosterSha256: "b".repeat(64),
      },
      results: [
        { udid: "phone-1", status: "succeeded" },
        { udid: "phone-2", status: "succeeded" },
      ],
    } satisfies MaterialPushBatchResult);

    render(
      <MaterialPage devices={devices} selected={[]} onSelectUdids={() => undefined} />,
    );

    await userEvent.click(await screen.findByRole("button", { name: "Chuyển tới 2 máy" }));
    expect(pushMaterialBatch).toHaveBeenCalledWith({
      materialId: material.id,
      target: { type: "all" },
    });
  });
});
