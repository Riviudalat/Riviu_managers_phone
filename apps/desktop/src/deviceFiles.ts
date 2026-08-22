/**
 * Path and listing arithmetic for the on-device file browser (xiaowei "Preview Mobile
 * Files").
 *
 * Pure, because every bug a file browser can have that actually loses data is in here: a
 * parent that walks above the root, a join that produces `//`, a delete aimed at the
 * directory instead of the file inside it. The backend refuses storage roots as a last line
 * of defence, but the path handed to it is composed here.
 *
 * Device paths are POSIX and always absolute — the Rust side rejects anything else — so this
 * module never sees a Windows path and never has to guess a separator.
 */

import type { DeviceFileEntry } from "./types";

/** Where the browser opens. The phone's own storage, which is what an operator wants. */
export const DEVICE_HOME = "/sdcard";

/** `/sdcard/Download/x` → `/sdcard/Download`. `/` has no parent and answers null. */
export function parentDevicePath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (trimmed === "" || trimmed === "/") return null;
  const cut = trimmed.lastIndexOf("/");
  if (cut <= 0) return "/";
  return trimmed.slice(0, cut);
}

/**
 * Join a directory and one entry name.
 *
 * Collapses the double slash that `"/" + name` produces at the root, which matters more than
 * it looks: `//sdcard` is a legal path that resolves the same on Linux, so the mistake is
 * invisible until it reaches a `rm -rf` log an operator is reading to work out what happened.
 */
export function joinDevicePath(dir: string, name: string): string {
  const base = dir.replace(/\/+$/, "");
  const leaf = name.replace(/^\/+/, "");
  return `${base}/${leaf}`;
}

export interface DeviceCrumb {
  label: string;
  path: string;
}

/** `/sdcard/Download` → root, sdcard, Download — each with the path to jump back to. */
export function deviceCrumbs(path: string): DeviceCrumb[] {
  const crumbs: DeviceCrumb[] = [{ label: "/", path: "/" }];
  let walked = "";
  for (const part of path.split("/")) {
    if (!part) continue;
    walked = `${walked}/${part}`;
    crumbs.push({ label: part, path: walked });
  }
  return crumbs;
}

/**
 * Directories first, then by name — case- and accent-insensitively, so `ảnh` sorts with `a`
 * and not after `z`.
 *
 * The phone's `ls` returns its own order (roughly byte order), which puts every dotfile in a
 * block at the top and `Zalo` before `apk`. That is a listing, not a browser.
 */
export function sortDeviceEntries(entries: DeviceFileEntry[]): DeviceFileEntry[] {
  const rank = (entry: DeviceFileEntry) => (entry.kind === "directory" ? 0 : 1);
  return [...entries].sort((a, b) => {
    const byKind = rank(a) - rank(b);
    if (byKind !== 0) return byKind;
    return a.name.localeCompare(b.name, "vi", { sensitivity: "base", numeric: true });
  });
}

/**
 * A size an operator can read, or an empty string where a size means nothing.
 *
 * Directories return `""` on purpose: the number `ls` prints for one is its inode size —
 * 3452 on this fleet's sdcard, for a folder holding anything from zero files to a thousand —
 * so showing it would be showing a number that answers no question anybody asked.
 */
export function formatDeviceSize(entry: DeviceFileEntry): string {
  if (entry.kind === "directory") return "";
  const bytes = entry.size;
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // One decimal below 10, none above: `1.4 MB` reads, `1.43 MB` and `847.2 MB` do not.
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/**
 * Can the browser step into this row?
 *
 * Symlinks yes, and deliberately without resolving them first: the listing does not say
 * where one points (that would cost a `ls` per row) so the phone is asked instead — a
 * symlink to a file answers with the file, and one that dangles answers with the error it
 * has. `/sdcard` itself is a symlink on every phone here, so refusing them would refuse the
 * one path the browser opens on.
 */
export function isBrowsableEntry(entry: DeviceFileEntry): boolean {
  return entry.kind === "directory" || entry.kind === "symlink";
}
