import { describe, expect, it } from "vitest";
import { publishScanErrorView } from "./publishScanErrors";

describe("publishScanErrorView", () => {
  it.each([
    ["publish folder has no bundle directories", "Chưa tìm thấy gói bài trong thư mục đã chọn"],
    ["publish folder does not exist: C:/Nội dung mới", "Thư mục nguồn không tồn tại"],
    ["publish path is not a directory: C:/Nội dung/video.mp4", "Đường dẫn nguồn không phải thư mục"],
  ])("translates %s and preserves its raw diagnostic", (message, title) => {
    const view = publishScanErrorView({ code: "OperationFailed", message });
    expect(view.title).toBe(title);
    expect(view.detail).toBeTruthy();
    expect(view.raw).toBe(message);
  });

  it.each([
    ["NoBundles", "Chưa tìm thấy gói bài trong thư mục đã chọn"],
    ["MissingRoot", "Thư mục nguồn không tồn tại"],
    ["RootNotDirectory", "Đường dẫn nguồn không phải thư mục"],
  ])("accepts a named scan code %s", (code, title) => {
    expect(publishScanErrorView({ code, message: "measured diagnostic" })).toEqual(expect.objectContaining({ title, raw: `${code}: measured diagnostic` }));
  });

  it("does not replace unknown diagnostic text with a generic error", () => {
    expect(publishScanErrorView(new Error("cannot read C:/Nội dung: Access is denied"))).toEqual({
      title: "cannot read C:/Nội dung: Access is denied", raw: "cannot read C:/Nội dung: Access is denied",
    });
  });
});
