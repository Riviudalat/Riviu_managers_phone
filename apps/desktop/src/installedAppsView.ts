import type { InstalledApp } from "./types";

/** What the fetch has produced so far. The component owns the transition, this owns the meaning. */
export type InstalledAppsLoad =
  | { state: "loading" }
  | { state: "ready"; apps: InstalledApp[] }
  | { state: "failed"; reason: string };

export interface InstalledAppsView {
  /** Rows to render, already filtered and ordered as the phone reported them. */
  rows: InstalledApp[];
  userCount: number;
  systemCount: number;
  /**
   * Why there is nothing to show, or null when the list itself is the answer.
   *
   * The distinction this exists for: a refusal and an empty phone are different facts.
   * A backend that cannot enumerate must not be rendered as a phone with nothing
   * installed, so a failure produces `kind: "refused"` carrying the backend's own
   * sentence, and a genuinely empty result produces `kind: "empty"`.
   */
  notice: { kind: "loading" | "refused" | "empty" | "filtered"; text: string } | null;
}

const NAMES_ARE_PACKAGES =
  "Android không trả tên hiển thị qua adb, nên đây là tên gói.";

/** The one line explaining why rows read as package names rather than app names. */
export function installedAppsFootnote(): string {
  return NAMES_ARE_PACKAGES;
}

/**
 * Turn a fetch outcome plus the operator's two controls into what to draw.
 *
 * Pure on purpose, following `updateView` and `agentStatus`: the backend owns the
 * evidence, this owns the wording, and the component owns neither. It also means the
 * refusal-versus-empty distinction — the whole reason the backend refuses instead of
 * returning `[]` — is covered by tests that need no promises and no rendering.
 */
export function installedAppsView(
  load: InstalledAppsLoad,
  showSystem: boolean,
  filter: string,
): InstalledAppsView {
  if (load.state === "loading") {
    return {
      rows: [],
      userCount: 0,
      systemCount: 0,
      notice: { kind: "loading", text: "Đang đọc danh sách ứng dụng…" },
    };
  }
  if (load.state === "failed") {
    return {
      rows: [],
      userCount: 0,
      systemCount: 0,
      notice: {
        kind: "refused",
        text: `Không đọc được danh sách ứng dụng: ${load.reason}`,
      },
    };
  }

  const userCount = load.apps.filter((app) => app.kind === "user").length;
  const systemCount = load.apps.filter((app) => app.kind === "system").length;
  const needle = filter.trim().toLowerCase();
  const rows = load.apps
    .filter((app) => showSystem || app.kind === "user")
    .filter(
      (app) =>
        !needle ||
        app.bundleId.toLowerCase().includes(needle) ||
        (app.label ?? "").toLowerCase().includes(needle),
    );

  if (rows.length > 0) {
    return { rows, userCount, systemCount, notice: null };
  }
  // Two different emptinesses, and conflating them would tell the operator their phone
  // is bare when they have simply typed a filter that matches nothing.
  return {
    rows,
    userCount,
    systemCount,
    notice: needle
      ? { kind: "filtered", text: "Không có ứng dụng nào khớp." }
      : { kind: "empty", text: "Máy này không báo ứng dụng nào." },
  };
}
