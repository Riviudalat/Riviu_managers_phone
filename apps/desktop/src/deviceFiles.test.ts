import { describe, expect, it } from "vitest";
import {
  deviceCrumbs,
  formatDeviceSize,
  isBrowsableEntry,
  joinDevicePath,
  parentDevicePath,
  sortDeviceEntries,
} from "./deviceFiles";
import type { DeviceFileEntry } from "./types";

function entry(over: Partial<DeviceFileEntry> & { name: string }): DeviceFileEntry {
  return { kind: "file", size: 0, modified: null, linkTarget: null, ...over };
}

describe("parentDevicePath", () => {
  it("walks up one level", () => {
    expect(parentDevicePath("/sdcard/Download/CV.pdf")).toBe("/sdcard/Download");
    expect(parentDevicePath("/sdcard/Download")).toBe("/sdcard");
    expect(parentDevicePath("/sdcard")).toBe("/");
  });

  it("stops at the root instead of walking above it", () => {
    expect(parentDevicePath("/")).toBeNull();
    expect(parentDevicePath("")).toBeNull();
  });

  it("ignores a trailing slash, which the phone's own paths carry", () => {
    expect(parentDevicePath("/sdcard/Download/")).toBe("/sdcard");
  });
});

describe("joinDevicePath", () => {
  it("never produces a double slash, at the root least of all", () => {
    expect(joinDevicePath("/", "sdcard")).toBe("/sdcard");
    expect(joinDevicePath("/sdcard", "Download")).toBe("/sdcard/Download");
    expect(joinDevicePath("/sdcard/", "/Download")).toBe("/sdcard/Download");
  });

  it("keeps the spaces real filenames on this fleet have", () => {
    expect(joinDevicePath("/sdcard/Download", "CV prototype.pdf")).toBe(
      "/sdcard/Download/CV prototype.pdf",
    );
  });
});

describe("deviceCrumbs", () => {
  it("gives every ancestor a path to jump to", () => {
    expect(deviceCrumbs("/sdcard/Download")).toEqual([
      { label: "/", path: "/" },
      { label: "sdcard", path: "/sdcard" },
      { label: "Download", path: "/sdcard/Download" },
    ]);
  });

  it("at the root there is one crumb and it is the root", () => {
    expect(deviceCrumbs("/")).toEqual([{ label: "/", path: "/" }]);
  });
});

describe("sortDeviceEntries", () => {
  it("puts folders first and then sorts by name, accents folded", () => {
    const sorted = sortDeviceEntries([
      entry({ name: "zalo.apk" }),
      entry({ name: "Ảnh", kind: "directory" }),
      entry({ name: "apk", kind: "directory" }),
      entry({ name: "CV.pdf" }),
    ]);
    expect(sorted.map((row) => row.name)).toEqual(["Ảnh", "apk", "CV.pdf", "zalo.apk"]);
  });

  it("does not reorder the array it was given", () => {
    const rows = [entry({ name: "b" }), entry({ name: "a" })];
    sortDeviceEntries(rows);
    expect(rows.map((row) => row.name)).toEqual(["b", "a"]);
  });
});

describe("formatDeviceSize", () => {
  it("scales and keeps one decimal only where it reads", () => {
    expect(formatDeviceSize(entry({ name: "a", size: 108 }))).toBe("108 B");
    // The measured size of `CV prototype.pdf` on this fleet: past 10 the decimal is dropped.
    expect(formatDeviceSize(entry({ name: "a", size: 138_078 }))).toBe("135 KB");
    expect(formatDeviceSize(entry({ name: "a", size: 1_500_000 }))).toBe("1.4 MB");
    expect(formatDeviceSize(entry({ name: "a", size: 45_868_000 }))).toBe("44 MB");
  });

  /**
   * The number `ls` prints for a directory is its inode size — 3452 for every folder on
   * this fleet's sdcard, whether it holds nothing or a thousand files. Showing it would be
   * showing a number that answers no question.
   */
  it("says nothing about the size of a folder", () => {
    expect(formatDeviceSize(entry({ name: "DCIM", kind: "directory", size: 3452 }))).toBe("");
  });
});

describe("isBrowsableEntry", () => {
  it("follows a symlink, because /sdcard itself is one on every phone here", () => {
    expect(isBrowsableEntry(entry({ name: "sdcard", kind: "symlink" }))).toBe(true);
    expect(isBrowsableEntry(entry({ name: "DCIM", kind: "directory" }))).toBe(true);
    expect(isBrowsableEntry(entry({ name: "CV.pdf" }))).toBe(false);
    expect(isBrowsableEntry(entry({ name: "socket", kind: "other" }))).toBe(false);
  });
});
