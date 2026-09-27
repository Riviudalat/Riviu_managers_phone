import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { DeviceInspector } from "./DeviceInspector";
import {
  inspectorConfirmPostcondition,
  inspectorObserve,
  inspectorRecording,
  inspectorTap,
  inspectorTapGalleryCell,
} from "../inspectorApi";

vi.mock("../inspectorApi", () => ({
  inspectorObserve: vi.fn(),
  inspectorTap: vi.fn(),
  inspectorTapGalleryCell: vi.fn(),
  inspectorRecord: vi.fn(),
  inspectorRecording: vi.fn(),
  inspectorConfirmPostcondition: vi.fn(),
}));
vi.mock("../api", () => ({ flowSaveRevision: vi.fn() }));

const profileSelector = { package: "app.fixture", description: "Profile" };
const snapshot = {
  id: "after",
  udid: "phone-a",
  package: "app.fixture",
  version: "1.0",
  locale: "en",
  width: 1080,
  height: 2220,
  pngBase64: "cG5n",
  treeSha256: "a".repeat(64),
  hierarchyXml: "<hierarchy><node content-desc=\"Profile\"/></hierarchy>",
  elements: [
    { index: 1, parent: null, text: "", description: "Favorites", resourceId: "app:id/hly", className: "android.widget.Button", x: 900, y: 1400, width: 180, height: 160, enabled: true, clickable: false, selector: { package: "app.fixture", description: "Favorites", schemaVersion: 2, actionTarget: { kind: "clickableAncestor" as const, ancestor: { maxDepth: 1, resourceId: "app:id/hlx" } } } },
    { index: 2, parent: null, text: "", description: "Profile", resourceId: "app:id/profile", className: "android.widget.FrameLayout", x: 850, y: 2000, width: 230, height: 120, enabled: true, clickable: true, selector: profileSelector },
  ],
};
const pending = {
  id: "record",
  udid: "phone-a",
  name: "Fixture",
  active: true,
  steps: [{ selector: { package: "app.fixture", description: "Create" }, expected: null, beforeId: "before", afterId: "after", verified: false, error: "Chờ chọn phần tử kết quả" }],
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(inspectorObserve).mockResolvedValue(snapshot);
  vi.mocked(inspectorRecording).mockResolvedValue(pending);
  vi.mocked(inspectorConfirmPostcondition).mockResolvedValue({
    ...pending,
    steps: [{ ...pending.steps[0], expected: profileSelector, verified: true, error: null }],
  });
});

it("shows the hierarchy evidence, parent-backed action and explicit recorded postcondition", async () => {
  render(<DeviceInspector udid="phone-a" onClose={vi.fn()}/>);
  expect(await screen.findByText("Favorites")).toBeInTheDocument();
  expect(screen.getByText("Bấm qua phần tử cha")).toBeInTheDocument();
  expect(screen.getByText("XML giao diện")).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Tìm phần tử"), { target: { value: "Profile" } });
  fireEvent.click(screen.getByRole("treeitem", { name: /Profile/ }));
  fireEvent.click(screen.getByRole("button", { name: "Dùng làm kết quả của bước vừa bấm" }));
  await waitFor(() => expect(inspectorConfirmPostcondition).toHaveBeenCalledWith(
    "phone-a", "after", profileSelector,
  ));
});

it("keeps screenshot, properties and a complete expandable hierarchy in separate panes", async () => {
  vi.mocked(inspectorObserve).mockResolvedValue({
    ...snapshot,
    elements: [
      { index: 0, parent: null, text: "", description: "", resourceId: "", className: "android.widget.FrameLayout", x: 0, y: 0, width: 1080, height: 2220, enabled: true, clickable: false, selector: null },
      { ...snapshot.elements[0], parent: 0 },
      { ...snapshot.elements[1], parent: 0 },
    ],
  });
  render(<DeviceInspector udid="phone-a" onClose={vi.fn()} />);

  const tree = await screen.findByRole("tree", { name: "Cây phần tử" });
  const container = screen.getByRole("treeitem", { name: /android.widget.FrameLayout/ });
  expect(container).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByRole("table", { name: "Thuộc tính phần tử" })).toBeVisible();
  expect(screen.getByRole("img", { name: "Màn hình thiết bị" })).toBeVisible();
  expect(tree).toContainElement(screen.getByRole("treeitem", { name: /Favorites/ }));

  fireEvent.click(screen.getByRole("button", { name: "Thu gọn android.widget.FrameLayout" }));
  expect(container).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByRole("treeitem", { name: /Favorites/ })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Mở rộng android.widget.FrameLayout" }));
  fireEvent.click(screen.getByRole("treeitem", { name: /Favorites/ }));
  expect(screen.getByRole("row", { name: /resource-id app:id\/hly/ })).toBeVisible();
  expect(screen.getByRole("row", { name: /clickable false/ })).toBeVisible();
  expect(inspectorTap).not.toHaveBeenCalled();
});

it("selects the smallest element on the screenshot and copies the exact XML", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  render(<DeviceInspector udid="phone-a" onClose={vi.fn()} />);
  const screenImage = await screen.findByRole("img", { name: "Màn hình thiết bị" });
  vi.spyOn(screenImage, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 800, bottom: 1110, width: 800, height: 1110, toJSON: () => ({}) });
  fireEvent.pointerMove(screenImage, { clientX: 580, clientY: 725 });
  fireEvent.click(screenImage, { clientX: 580, clientY: 725 });
  expect(screen.getByRole("treeitem", { name: /Favorites/ })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("row", { name: /resource-id app:id\/hly/ })).toBeVisible();
  expect(inspectorTap).not.toHaveBeenCalled();

  fireEvent.click(screenImage, { clientX: 40, clientY: 725 });
  expect(screen.getByRole("treeitem", { name: /Favorites/ })).toHaveAttribute("aria-selected", "false");

  fireEvent.click(screen.getByRole("button", { name: "Sao chép XML" }));
  expect(writeText).toHaveBeenCalledWith(snapshot.hierarchyXml);
});

it("exports the same screenshot and hierarchy with the detected app build", async () => {
  const createObjectURL = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:screen");
  const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  let downloadedName = "";
  const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) { downloadedName = this.download; });
  try {
    render(<DeviceInspector udid="phone-a" onClose={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Xuất dữ liệu màn hình" }));
    const blob = createObjectURL.mock.calls[0][0] as Blob;
    const body = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error);
      reader.readAsText(blob);
    });
    const evidence = JSON.parse(body);
    expect(evidence).toMatchObject({
      schemaVersion: 1,
      udid: "phone-a",
      package: "app.fixture",
      version: "1.0",
      locale: "en",
      hierarchyXml: snapshot.hierarchyXml,
      pngBase64: snapshot.pngBase64,
      treeSha256: snapshot.treeSha256,
    });
    expect(click).toHaveBeenCalledOnce();
    expect(downloadedName).toBe("riviu-screen-app.fixture-1.0-phone-a-after.json");
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:screen");
  } finally {
    createObjectURL.mockRestore();
    revokeObjectURL.mockRestore();
    click.mockRestore();
  }
});

it("selects an explicitly chosen gallery cell outside a recording", async () => {
  vi.mocked(inspectorObserve).mockResolvedValue({
    ...snapshot,
    package: "com.instagram.barcelona",
    elements: [
      { ...snapshot.elements[0], index: 10, parent: null, resourceId: "com.instagram.barcelona:id/gallery_picker_grid_item_container", clickable: true, selector: null },
      { ...snapshot.elements[1], index: 11, parent: 10, resourceId: "", clickable: false, selector: null },
    ],
  });
  vi.mocked(inspectorRecording).mockResolvedValue(null);
  vi.mocked(inspectorTapGalleryCell).mockResolvedValue(snapshot);
  render(<DeviceInspector udid="phone-a" onClose={vi.fn()} />);
  fireEvent.click(await screen.findByRole("treeitem", { name: /Profile/ }));
  fireEvent.click(screen.getByRole("button", { name: "Chọn ô ảnh này" }));
  await waitFor(() => expect(inspectorTapGalleryCell).toHaveBeenCalledWith("phone-a", "after", 10));
  expect(inspectorTap).not.toHaveBeenCalled();
});

it("explains why gallery selection is unavailable during recording", async () => {
  vi.mocked(inspectorObserve).mockResolvedValue({
    ...snapshot,
    package: "com.instagram.barcelona",
    elements: [{ ...snapshot.elements[0], resourceId: "com.instagram.barcelona:id/gallery_picker_grid_item_container", clickable: true, selector: null }],
  });
  render(<DeviceInspector udid="phone-a" onClose={vi.fn()} />);
  fireEvent.click(await screen.findByRole("treeitem", { name: /Favorites/ }));
  expect(screen.getByRole("button", { name: "Dừng ghi để chọn ảnh" })).toBeDisabled();
});
