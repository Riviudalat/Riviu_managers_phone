import { describeError } from "./describeError";

export interface PublishScanErrorView {
  title: string;
  detail?: string;
  raw: string;
}

export function publishScanErrorView(error: unknown): PublishScanErrorView {
  const raw = describeError(error);
  const code = error && typeof error === "object" && "code" in error ? error.code : null;
  if (code === "NoBundles" || raw === "publish folder has no bundle directories") {
    return {
      title: "Chưa tìm thấy gói bài trong thư mục đã chọn",
      detail: "Chọn thư mục chứa một video hoặc bộ ảnh cùng chú thích, hoặc thư mục cha chứa các gói bài.",
      raw,
    };
  }
  if (code === "MissingRoot" || raw.startsWith("publish folder does not exist: ")) {
    return { title: "Thư mục nguồn không tồn tại", detail: "Kiểm tra lại đường dẫn hoặc chọn thư mục khác.", raw };
  }
  if (code === "RootNotDirectory" || raw.startsWith("publish path is not a directory: ")) {
    return { title: "Đường dẫn nguồn không phải thư mục", detail: "Chọn thư mục chứa nội dung thay vì chọn một tệp.", raw };
  }
  return { title: raw, raw };
}
