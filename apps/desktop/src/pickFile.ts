import { open } from "@tauri-apps/plugin-dialog";
import { toastError } from "./toastStore";

/**
 * The native pickers, and they **never throw**.
 *
 * That is the whole point of this wrapper, and it is a bug fix rather than a style: every
 * caller was written as
 *
 * ```ts
 * const path = await pickFile(...);
 * if (!path) return;
 * try { …the device work… } catch (e) { toastError(…) }
 * ```
 *
 * so the pick itself sat *outside* the handler. A dialog that failed to open — plugin
 * refused, permission missing, the OS dialog itself erroring — rejected a promise nobody was
 * awaiting, and the operator saw a row click do **absolutely nothing**. Rows reported as "not
 * working" (Cài APK, Đưa ảnh/video vào máy, Lấy ảnh/video từ máy) all share that shape.
 *
 * A picker that cannot open is always operator-visible and never something a caller can
 * recover from, so the failure is toasted here and `null`/`[]` is returned — the same answer
 * as a cancel, which every caller already handles. Cancelling stays silent.
 */
async function openOrReport<T>(
  run: () => Promise<T>,
  fallback: T,
  what: string,
): Promise<T> {
  try {
    return await run();
  } catch (error) {
    toastError(`Không mở được hộp thoại ${what}`, error);
    return fallback;
  }
}

/** Native file picker — absolute path, or null if cancelled or the dialog could not open. */
export async function pickFile(opts?: {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
}): Promise<string | null> {
  return openOrReport(
    async () => {
      const selected = await open({
        multiple: false,
        directory: false,
        title: opts?.title ?? "Chọn file",
        filters: opts?.filters,
      });
      if (selected == null) return null;
      if (Array.isArray(selected)) return selected[0] ?? null;
      return selected;
    },
    null,
    "chọn tệp",
  );
}

/** Native multi-file picker — absolute paths, empty if cancelled or the dialog failed. */
export async function pickFiles(opts?: {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
}): Promise<string[]> {
  return openOrReport(
    async () => {
      const selected = await open({
        multiple: true,
        directory: false,
        title: opts?.title ?? "Chọn tệp",
        filters: opts?.filters,
      });
      if (selected == null) return [];
      return Array.isArray(selected) ? selected : [selected];
    },
    [],
    "chọn tệp",
  );
}

export async function pickDirectory(title = "Chọn thư mục nội dung"): Promise<string | null> {
  return openOrReport(
    async () => {
      const selected = await open({ multiple: false, directory: true, title });
      if (selected == null) return null;
      if (Array.isArray(selected)) return selected[0] ?? null;
      return selected;
    },
    null,
    "chọn thư mục",
  );
}

export async function pickIpa(): Promise<string | null> {
  return pickFile({
    title: "Chọn IPA",
    filters: [{ name: "IPA", extensions: ["ipa"] }],
  });
}

export async function pickMaterial(): Promise<string | null> {
  return pickFile({
    title: "Chọn material",
    filters: [
      {
        name: "Media",
        extensions: ["jpg", "jpeg", "png", "gif", "webp", "heic", "mp4", "mov", "m4v", "pdf"],
      },
      { name: "All", extensions: ["*"] },
    ],
  });
}
