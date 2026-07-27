import { open } from "@tauri-apps/plugin-dialog";

/** Native file picker — returns absolute path or null if cancelled. */
export async function pickFile(opts?: {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
}): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    title: opts?.title ?? "Chọn file",
    filters: opts?.filters,
  });
  if (selected == null) return null;
  if (Array.isArray(selected)) return selected[0] ?? null;
  return selected;
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
