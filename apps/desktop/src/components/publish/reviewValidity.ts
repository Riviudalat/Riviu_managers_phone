import type { DeviceInfo, DeviceMeta, PublishNeedsRelease, PublishPreflightReport } from "../../types";

/**
 * A public-post review is valid for one version of its inputs (PLAN §3).
 *
 * The backend binds the approval to `inputDigest` (devices and roster, per-phone account,
 * content files by hash, caption, sound policy, Sheet target, TikTok build) and refuses a
 * Start whose digest no longer matches. These helpers let the review dialog stop *before*
 * sending and say what changed, instead of letting the operator learn it from a refusal.
 * They never substitute a phone or an account and never shrink the scope: a change only
 * ever means "review again".
 */

const account = (handle: string | undefined) => handle?.trim().replace(/^@+/, "").toLowerCase() ?? "";
const shown = (handle: string) => handle ? `@${handle}` : "chưa gắn";

/** The account each reviewed phone carried when the review was taken. */
export function reviewedAccounts(
  report: PublishPreflightReport,
  metas: ReadonlyMap<string, DeviceMeta>,
): Record<string, string> {
  return Object.fromEntries(report.assignments.map((row) => [row.udid, account(metas.get(row.udid)?.handle)]));
}

/** Changes the app can already see while the review is open: a phone left, went busy, or its account changed. */
export function liveReviewChanges(
  report: PublishPreflightReport,
  accounts: Readonly<Record<string, string>>,
  devices: readonly Pick<DeviceInfo, "udid" | "status">[],
  metas: ReadonlyMap<string, DeviceMeta>,
  label: (udid: string) => string,
): string[] {
  const released = new Set((report.needsRelease ?? []).map((row) => row.udid));
  const changes: string[] = [];
  for (const row of report.assignments) {
    const device = devices.find((candidate) => candidate.udid === row.udid);
    if (!device || device.status === "disconnected") changes.push(`${label(row.udid)} mất kết nối`);
    // A phone the review already said it would release is part of what was approved.
    else if (device.status === "busy" && !released.has(row.udid)) changes.push(`${label(row.udid)} vừa bận việc khác`);
    const before = accounts[row.udid];
    const now = account(metas.get(row.udid)?.handle);
    if (before !== undefined && before !== now) changes.push(`${label(row.udid)}: tài khoản ${shown(before)} → ${shown(now)}`);
  }
  return changes;
}

/** Differences between the review the operator saw and a fresh check of the same request. */
export function reviewChanges(
  before: PublishPreflightReport,
  after: PublishPreflightReport,
  label: (udid: string) => string,
  bundleName: (bundleId: string) => string,
): string[] {
  const changes: string[] = [];
  const next = new Map(after.assignments.map((row) => [row.udid, row]));
  for (const row of before.assignments) {
    const fresh = next.get(row.udid);
    if (!fresh) {
      changes.push(`${label(row.udid)} không còn trong lượt đăng`);
      continue;
    }
    if (fresh.bundleId !== row.bundleId) changes.push(`${label(row.udid)}: bài ${bundleName(row.bundleId)} → ${bundleName(fresh.bundleId)}`);
    if (fresh.packageName !== row.packageName || fresh.version !== row.version) {
      changes.push(`${label(row.udid)}: TikTok ${row.version ?? "chưa rõ"} → ${fresh.version ?? "chưa rõ"}`);
    }
    if (fresh.storage !== row.storage) changes.push(`${label(row.udid)}: dung lượng ${fresh.storage === "pass" ? "đã đủ" : "không còn đủ"}`);
  }
  const reviewed = new Set(before.assignments.map((row) => row.udid));
  for (const row of after.assignments) if (!reviewed.has(row.udid)) changes.push(`${label(row.udid)} mới vào lượt đăng`);
  // Holders are compared by their exact envelope (phone, owner kind, job or lease generation),
  // never by phone alone: a different job on the same phone voids the confirmation. Direct
  // control and the idle sweep are momentary and not part of what was approved.
  const transient = (row: PublishNeedsRelease) => row.owner === "ManualControl" || row.owner === "IdleSweep";
  const job = (row: PublishNeedsRelease) => row.title ? `${row.owner} · ${row.title}` : row.owner;
  const reviewedHolds = (before.needsRelease ?? []).filter((row) => !transient(row));
  const freshHolds = (after.needsRelease ?? []).filter((row) => !transient(row));
  const reviewedEnvelopes = new Set(reviewedHolds.map((row) => row.envelope));
  const freshEnvelopes = new Set(freshHolds.map((row) => row.envelope));
  for (const row of freshHolds) {
    if (reviewedEnvelopes.has(row.envelope)) continue;
    const old = reviewedHolds.filter((reviewedRow) => reviewedRow.udid === row.udid && !freshEnvelopes.has(reviewedRow.envelope));
    changes.push(old.length
      ? `${label(row.udid)}: việc đang giữ máy đổi từ ${old.map(job).join(", ")} sang ${job(row)}`
      : `${label(row.udid)} vừa bận: ${job(row)}; xác nhận sẽ phải dừng việc này`);
  }
  for (const row of reviewedHolds) {
    if (!freshEnvelopes.has(row.envelope) && !freshHolds.some((fresh) => fresh.udid === row.udid)) {
      changes.push(`${label(row.udid)} không còn việc cần dừng`);
    }
  }
  const sheet = (report: PublishPreflightReport) =>
    report.sheetDelivery ? `${report.sheetDelivery.spreadsheetId}#${report.sheetDelivery.sheetGid}` : "";
  if (sheet(before) !== sheet(after) || before.sheetEnabled !== after.sheetEnabled) changes.push("Đích ghi Google Sheet đã đổi");
  if (before.targetSnapshot.rosterSha256 !== after.targetSnapshot.rosterSha256) changes.push("Danh sách máy đang kết nối đã đổi");
  if (!after.canExecute) {
    const issues = after.issues.map((issue) => issue.message).slice(0, 3).join("; ");
    changes.push(`Kiểm tra mới chưa đạt${issues ? `: ${issues}` : ""}`);
  }
  if (!changes.length && before.inputDigest !== after.inputDigest) {
    changes.push("Nội dung nguồn, caption, tài khoản hoặc cấu hình đăng đã đổi sau khi xem lại");
  }
  return changes;
}
