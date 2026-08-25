import type { GroupInputReport } from "./types";

/// What to tell the operator after an action aimed at several phones at once.
///
/// The backend has always returned which phones it reached and which it skipped; the
/// frontend declared the call `invoke<void>` and threw the answer away. So an action aimed at
/// twenty phones that reached **none** of them resolved successfully and toasted "done" —
/// and the operator had no way to know, short of watching twenty tiles.

export type GroupInputOutcome =
  /// Every phone took it. Say nothing special.
  | { kind: "ok" }
  /// Some took it, some did not.
  | { kind: "partial"; title: string; detail: string }
  /// None took it. This is a failure however the promise resolved.
  | { kind: "none"; title: string; detail: string };

/// Names the phones, not just the count.
///
/// "3 máy bị bỏ qua" is not actionable; "máy 4, 11, 19 — đang bị nurture giữ" is, because the
/// operator can go and stop that. `currentOwner` is the single most useful field on the wire
/// and it was being discarded with the rest.
function describe(report: GroupInputReport): string {
  const byReason = new Map<string, string[]>();
  for (const skip of report.skipped) {
    const reason =
      skip.code === "DeviceBusy"
        ? `đang bị ${skip.currentOwner ?? "việc khác"} giữ`
        : skip.message || "lỗi khi thực hiện";
    const list = byReason.get(reason) ?? [];
    list.push(skip.udid.slice(-6));
    byReason.set(reason, list);
  }
  return [...byReason.entries()]
    .map(([reason, udids]) => `${udids.join(", ")} — ${reason}`)
    .join("\n");
}

export function groupInputOutcome(report: GroupInputReport): GroupInputOutcome {
  if (!report.skipped.length) return { kind: "ok" };

  const reached = report.completedUdids.length;
  if (reached === 0) {
    return {
      kind: "none",
      title: `Không máy nào nhận được thao tác (${report.skipped.length})`,
      detail: describe(report),
    };
  }
  return {
    kind: "partial",
    title: `${reached} máy nhận, ${report.skipped.length} máy bỏ qua`,
    detail: describe(report),
  };
}
