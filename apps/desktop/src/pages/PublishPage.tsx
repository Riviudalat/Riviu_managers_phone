import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { RefreshCw, Search, ListChecks, ArrowUpRight } from "lucide-react";
import { AutomationTabs, type AutomationMode } from "../components/AutomationTabs";
import { PublishQuickSetup as PublishWizard } from "../components/publish/PublishQuickSetup";
import { PublishSchedulePlanner } from "../components/publish/PublishSchedulePlanner";
import { PublishSheetConnection } from "../components/publish/PublishSheetConnection";
import "../styles/publish-workspace.css";
import { PublishScheduleRetime } from "../components/publish/PublishScheduleRetime";
import { reconcileAssignments } from "../components/publish/publishAssignments";
import { publishSelectionStatus } from "../components/publish/publishSelectionStatus";
import { usePublishDeviceGuards } from "../components/publish/usePublishDeviceGuards";
import { deviceGuardBlock, UNKNOWN_PUBLISH_GUARD } from "../components/publish/publishDeviceGuardState";

import {
  listenRiviuEvents,
  operationGetRun,
  operationListRuns,
  publishCancel,
  publishCheckLinks,
  publishRecoveryCapabilities,
  publishResumeVerification,
  operationStop,
  publishCreateCampaign,
  publishExecute,
  publishGet,
  publishGetLimits,
  publishList,
  publishPreflight,
  operationPrepareDevices,
  publishReconcile,
  publishRetryAssignment,
  publishScanFolder,
  publishSetLimits,
  type PublishLimits,
} from "../api";
import { useWorkspaceDraft } from "../workspaceDraft";
import { writeFormDraft } from "../formDraftStorage";
import { readPublishForm } from "../components/publish/publishDraftStorage";
import { IconRocket } from "../components/Icons";
import {
  EmptyState,
  LoadingState,
  StatusNotice,
  type NoticeTone,
} from "../components/States";
import {
  ResponsiveTable,
  StatusChip,
  type StatusTone,
} from "../components/WorkspacePrimitives";
import { requestConfirm } from "../confirmStore";
import { describeError } from "../describeError";
import { orderDevicesByNumber, tileName, tileNumber } from "../deviceNaming";
import {
  publishScanErrorView,
  type PublishScanErrorView,
} from "../publishScanErrors";
import { targetsOf } from "../selectionTargets";
import type { OperationSourceRef } from "../operationSource";
import type {
  PublishAssignmentRecord,
  PublishRecoveryCapability,
  PublishCampaignDetail,
  PublishCampaignRecord,
  PublishFolderManifest,
  PublishExecutionSnapshot,
  PublishPreflightReport,
  PublishPreflightRequest,
  OperationRunSummary,
  PublishSoundPolicy,
  TargetRef,
} from "../types";
import type { SelProps } from "./pageProps";

const PUBLISH_STATE_LABELS: Record<PublishCampaignRecord["state"], string> = {
  queued: "Đang chờ",
  scheduled: "Đã lên lịch",
  preparing: "Đang kiểm tra",
  ready: "Sẵn sàng",
  transferring: "Đang chuyển nội dung",
  imported: "Đã nhập nội dung",
  posting: "Đang đăng",
  verifying: "Đã bấm Đăng · chờ xác minh",
  succeeded: "Đã đăng",
  failedBeforeDispatch: "Dừng trước khi đăng",
  uncertain: "Chưa chắc chắn",
  cancelled: "Đã huỷ",
  missed: "Lỡ lịch",
};

const RETRYABLE_STATES: PublishCampaignRecord["state"][] = [
  "queued",
  "ready",
  "imported",
  "failedBeforeDispatch",
];
const CANCELLABLE_STATES: PublishCampaignRecord["state"][] = [
  "queued",
  "scheduled",
  "preparing",
  "ready",
  "transferring",
  "imported",
  "failedBeforeDispatch",
];

function campaignTone(state: PublishCampaignRecord["state"]): StatusTone {
  if (state === "succeeded") return "warning";
  if (state === "uncertain" || state === "missed") return "warning";
  if (state === "failedBeforeDispatch" || state === "cancelled") return "error";
  return state === "queued" || state === "scheduled" ? "neutral" : "info";
}

function needsPublicationReview(value: Pick<PublishCampaignRecord, "state" | "errorCode">): boolean {
  return value.state === "uncertain" && value.errorCode === "post_verification_needs_review";
}

function publicationReviewReason(evidenceJson?: string | null): string | null {
  try {
    const status = JSON.parse(evidenceJson ?? "null")?.verificationStatus;
    if (status?.state !== "needsReview" || typeof status.reason !== "string") return null;
    if (/tự kiểm tra đã dừng/i.test(status.reason)) return status.reason;
    return `${status.reason} · Tự kiểm tra đã dừng · chọn Kiểm tra liên kết`;
  } catch {
    return null;
  }
}

function verificationDetail(evidenceJson?: string | null): string | null {
  try {
    const value = JSON.parse(evidenceJson ?? "null");
    const status = value?.verificationStatus;
    if (status?.state === "verified" || value?.post?.publicationVerified || value?.publicationVerified) return null;
    const reason = status?.reason ?? value?.post?.linkCaptureReason ?? value?.linkCaptureReason;
    if (typeof reason !== "string") return null;
    const format = (value: unknown) => typeof value === "string" && Number.isFinite(Date.parse(value))
      ? new Date(value).toLocaleTimeString("vi-VN") : null;
    const checked = format(status?.checkedAt), next = format(status?.nextCheckAt);
    const stopped = status?.state === "needsReview"
      && !/tự kiểm tra đã dừng/i.test(reason)
      ? "Tự kiểm tra đã dừng · chọn Kiểm tra liên kết"
      : null;
    const budget = status?.checkIntervalSeconds === 300
      ? "Tự kiểm tra mỗi 5 phút đến khi có link"
      : typeof status?.reviewAfterMinutes === "number"
      ? `Ngân sách tự kiểm: ${status.reviewAfterMinutes} phút`
      : null;
    return [
      reason,
      stopped,
      checked && `Kiểm tra gần nhất: ${checked}`,
      next && status?.state === "pending" && `Kiểm tra tiếp: ${next}`,
      status?.state === "pending" && budget,
    ].filter(Boolean).join(" · ");
  } catch { return null; }
}

function campaignView(
  campaign: PublishCampaignRecord,
  operation?: OperationRunSummary,
  snapshot?: PublishExecutionSnapshot,
  detail?: PublishCampaignDetail,
): {
  label: string;
  tone: StatusTone;
  retryScope: PublishExecutionSnapshot["retryScope"];
} {
  if (campaign.state === "verifying" || needsPublicationReview(campaign)) {
    const currentSnapshot = snapshot && (!operation?.updatedAt || snapshot.updatedAt >= operation.updatedAt) ? snapshot : undefined;
    const proposedScope = currentSnapshot?.retryScope ?? operation?.retryScope;
    return {
      label: needsPublicationReview(campaign) ? "Cần kiểm tra bài đăng" : PUBLISH_STATE_LABELS.verifying,
      tone: needsPublicationReview(campaign) ? "warning" : "info",
      retryScope: proposedScope === "linkAndSheet" ? "linkAndSheet" : "none",
    };
  }
  if (
    [
      "scheduled",
      "preparing",
      "transferring",
      "posting",
      "uncertain",
      "cancelled",
      "missed",
    ].includes(campaign.state)
  ) {
    return {
      label: PUBLISH_STATE_LABELS[campaign.state],
      tone: campaignTone(campaign.state),
      retryScope: "none",
    };
  }
  if (campaign.state === "succeeded" && detail?.assignments.some(assignment => assignment.sheetDelivery?.state === "superseded")) {
    return { label: "Đã đăng · đợt báo cáo đã đóng", tone: "warning", retryScope: "none" };
  }
  const newestSnapshot =
    snapshot &&
    (!operation?.updatedAt || snapshot.updatedAt >= operation.updatedAt)
      ? snapshot
      : undefined;
  const state = newestSnapshot
    ? newestSnapshot.status === "complete"
      ? "succeeded"
      : newestSnapshot.status
    : operation?.state;
  const proposedScope =
    newestSnapshot?.retryScope ??
    operation?.retryScope ??
    (RETRYABLE_STATES.includes(campaign.state) ? "fullPipeline" : "none");
  const retryScope =
    campaign.state === "succeeded" && proposedScope === "fullPipeline"
      ? "none"
      : proposedScope;
  if (state === "succeeded")
    return { label: "Hoàn tất", tone: "success", retryScope: "none" };
  if (state === "uncertain")
    return { label: "Chưa chắc chắn", tone: "warning", retryScope: "none" };
  if (state === "partial")
    return { label: "Hoàn tất một phần", tone: "warning", retryScope };
  return {
    label:
      campaign.state === "succeeded"
        ? "Đã đăng · chờ đối chiếu"
        : PUBLISH_STATE_LABELS[campaign.state],
    tone: campaignTone(campaign.state),
    retryScope,
  };
}

function snapshotSheetEnabled(snapshot?: PublishExecutionSnapshot): boolean {
  const report = snapshot?.reportJson;
  return !(
    report &&
    typeof report === "object" &&
    !Array.isArray(report) &&
    report.sheetEnabled === false
  );
}

function recoveryReason(reason: string | null | undefined): string | null {
  if (!reason) return null;
  const labels: Record<string, string> = {
    activePipeline: "Chờ các máy còn lại kết thúc lượt chạy",
    stopInProgress: "Tác vụ đang dừng và nhả máy; hoàn tất Dừng trước khi tiếp tục xác minh",
    notRetryableBeforePost: "Bài không đủ điều kiện thử lại trước Đăng",
    notSubmitted: "Bài chưa có bằng chứng đã gửi; không thể tiếp tục xác minh",
    submissionIdentityMissing: "Thiếu tài khoản hoặc thời điểm của lần Đăng; cần kiểm tra thủ công",
    operatorStopped: "Người dùng đã dừng kiểm tra; cần xác nhận tiếp tục xác minh",
    explicitReview: "Bài cần kiểm tra thủ công; không tự gỡ trạng thái cần xử lý",
    alreadyPending: "Bài đang được xác minh định kỳ; không khởi động lại lượt kiểm tra",
    alreadyVerified: "Bài đã được xác minh; không đăng lại",
    revisionChanged: "Trạng thái bài đã thay đổi; tải lại chi tiết trước khi tiếp tục",
    confirmationRequired: "Cần xác nhận phạm vi chỉ kiểm tra bài đã gửi",
    assignmentMissing: "Không tìm thấy bài trong lượt đăng",
    noCandidate: "Không có bài đủ điều kiện kiểm tra liên kết",
    verificationNeedsReview: "Bài cần kiểm tra thủ công; tự kiểm tra đã dừng",
    deviceBusy: "Máy đang bận; chờ nhả máy để kiểm tra",
  };
  return labels[reason] ?? reason;
}

function canRetryAssignment(assignment: PublishAssignmentRecord, campaign: PublishCampaignRecord): boolean {
  return assignment.state === "failedBeforeDispatch"
    && assignment.effectIntent == null
    && !["cancelled", "missed"].includes(campaign.state)
    && !["queued", "running"].includes(assignment.dispatch?.state ?? "");
}

function dispatchDetail(assignment: PublishAssignmentRecord): string | null {
  const dispatch = assignment.dispatch;
  const reason = dispatch?.reason ?? assignment.errorCode;
  const reasons: Record<string, string> = {
    device_busy: "Máy đang bận",
    account_busy: "Tài khoản đang được sử dụng",
    schedule_capacity_deadline: "Chưa được cấp lượt trong cửa sổ 30 giây; không tự đăng bù",
    app_opened_after_deadline: "Ứng dụng mở sau giờ hẹn; không tự đăng bù",
    app_closing: "Ứng dụng đang đóng",
    campaign_cancelled: "Chiến dịch đã hủy",
  };
  if (assignment.state === "missed") return reasons[reason ?? ""] ?? "Chưa bắt đầu đúng cửa sổ giờ hẹn; không tự đăng bù";
  if (!dispatch || !["queued", "running", "paused"].includes(dispatch.state)) return null;
  const time = new Date(dispatch.queuedAtMs);
  return [
    `${dispatch.state === "running" ? "Đang" : "Chờ"} ${dispatch.phase === "transfer" ? "chuyển media" : "thao tác TikTok"}`,
    Number.isFinite(time.getTime()) && `Vào hàng: ${time.toLocaleString("vi-VN")}`,
    dispatch.owner && `Đang giữ lượt: ${dispatch.owner}`,
    dispatch.state !== "running" && (reasons[reason ?? ""] ?? "Chờ lượt điều phối"),
  ].filter(Boolean).join(" · ");
}

const PUBLISH_LIMIT_FIELDS: { key: keyof PublishLimits; label: string }[] = [
  { key: "transfer", label: "Lượt chuyển media" },
  { key: "compose", label: "Phiên thao tác TikTok" },
  { key: "verify", label: "Lượt xác minh liên kết" },
  { key: "deviceTotal", label: "Tổng lượt điều khiển thiết bị" },
];

function PublishHostLimits({ onSaved }: { onSaved: () => void }) {
  const [limits, setLimits] = useState<PublishLimits | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const flight = useRef(false);
  const valid = limits !== null && Object.values(limits).every(value => Number.isInteger(value) && value >= 1 && value <= 64);
  const read = async () => {
    if (flight.current) return;
    flight.current = true; setBusy(true); setError(null); setMessage(null);
    try { setLimits(await publishGetLimits()); }
    catch (e) { setError(describeError(e)); }
    finally { flight.current = false; setBusy(false); }
  };
  const save = async () => {
    if (flight.current || !valid || !limits) return;
    flight.current = true; setBusy(true); setError(null); setMessage(null);
    try {
      await publishSetLimits(limits);
      setLimits(await publishGetLimits());
      onSaved();
      setMessage("Đã lưu giới hạn cho toàn ứng dụng trên máy tính này.");
    } catch (e) { setError(describeError(e)); }
    finally { flight.current = false; setBusy(false); }
  };
  return <details className="publish-workspace-section" style={{ flex: "0 0 auto", maxHeight: "45%", padding: "6px 12px" }} onToggle={event => {
    if (event.currentTarget.open && !limits && !flight.current) void read();
  }}>
    <summary>Giới hạn chạy đồng thời</summary>
    <p>Đăng ngay và hẹn giờ dùng chung các giới hạn này. Sheet giữ tối đa 2 yêu cầu, gồm tối đa 1 yêu cầu báo tiến độ.</p>
    {busy && <p role="status">Đang đọc hoặc lưu giới hạn…</p>}
    {error && <p role="alert">{error}</p>}
    {limits && <fieldset disabled={busy}><legend>Giới hạn của máy tính này</legend>
      {PUBLISH_LIMIT_FIELDS.map(({ key, label }) => <label key={key}>{label}
        <input type="number" min={1} max={64} step={1} value={Number.isNaN(limits[key]) ? "" : limits[key]} onChange={event => {
          setLimits({ ...limits, [key]: event.target.valueAsNumber }); setMessage(null);
        }} />
      </label>)}
      {!valid && <p role="alert">Mỗi giới hạn phải là số nguyên từ 1 đến 64.</p>}
      <button type="button" disabled={!valid} onClick={() => void save()}>Lưu giới hạn</button>
    </fieldset>}
    {error && <button type="button" disabled={busy} onClick={() => void read()}>Đọc lại giới hạn</button>}
    {message && <p role="status">{message}</p>}
    <p>Giảm giới hạn chỉ áp dụng khi cấp lượt mới. Mở lại Riviu sau khi đổi lượt xác minh để cập nhật số phiên xác minh.</p>
  </details>;
}

function pendingPublicationMessage(detail: PublishCampaignDetail, sheetEnabled: boolean): string | null {
  const review = detail.assignments.filter(needsPublicationReview).length;
  if (review) return `${review} bài cần kiểm tra trên điện thoại. Chưa có đủ bằng chứng xác nhận; tác vụ tự kiểm tra đã dừng. Mở TikTok để xem bài đăng hoặc bản nháp, rồi chọn Kiểm tra liên kết. App giữ nội dung và không tự đăng lại.`;
  const pending = detail.assignments.filter((assignment) => assignment.state === "verifying").length;
  if (!pending) return null;
  return `${pending} bài đã bấm Đăng, đang chờ TikTok hoàn tất và xác minh liên kết. Riviu tự kiểm tra khi máy rảnh; giữ app và điện thoại kết nối.${sheetEnabled ? " Sheet chờ liên kết đã xác minh." : " Không ghi Sheet."}`;
}

function retryActionLabel(
  scope: PublishExecutionSnapshot["retryScope"],
  sheetEnabled = true,
): string {
  if (scope === "sheetOnly") return "Ghi lại Sheet";
  if (scope === "linkAndSheet")
    return sheetEnabled ? "Lấy link và ghi Sheet" : "Lấy lại liên kết";
  return "Chạy lại từ đầu";
}

function cleanupEvidence(
  evidenceJson?: string | null,
): { label: string; raw: string } | null {
  if (!evidenceJson) return null;
  try {
    const evidence = JSON.parse(evidenceJson) as unknown;
    if (!evidence || typeof evidence !== "object" || !("cleanup" in evidence))
      return null;
    const cleanup = (evidence as { cleanup?: unknown }).cleanup;
    if (!cleanup || typeof cleanup !== "object") return null;
    const state =
      "state" in cleanup
        ? String((cleanup as { state?: unknown }).state ?? "")
        : "";
    const message =
      "message" in cleanup
        ? String((cleanup as { message?: unknown }).message ?? "").trim()
        : "";
    const raw = JSON.stringify(cleanup);
    if (state === "cleaned") return { label: "ảnh tạm đã dọn", raw };
    if (state === "kept") {
      const appCleanup = "appCleanup" in cleanup ? cleanup.appCleanup : null;
      const leftRunning = appCleanup && typeof appCleanup === "object"
        && "state" in appCleanup && appCleanup.state === "leftRunning";
      return { label: leftRunning ? "đã giữ nội dung và để TikTok tiếp tục xử lý" : "đã giữ nội dung trên máy", raw };
    }
    if (state === "not_cleaned") {
      return {
        label: `chưa dọn được ảnh tạm${message ? `: ${message}` : ""}`,
        raw,
      };
    }
    return { label: "trạng thái dọn ảnh chưa nhận diện", raw };
  } catch {
    return null;
  }
}

function postEvidence(evidenceJson?: string | null): {
  url: string | null;
  sound: {
    title: string;
    artist: string;
    section: string;
    index: number;
    digest: string;
    confirmed: boolean;
  } | null;
} {
  const empty = { url: null, sound: null };
  try {
    const evidence = JSON.parse(evidenceJson ?? "null") as unknown;
    if (!evidence || typeof evidence !== "object" || Array.isArray(evidence))
      return empty;
    const root = evidence as Record<string, unknown>;
    const post =
      root.post && typeof root.post === "object" && !Array.isArray(root.post)
        ? (root.post as Record<string, unknown>)
        : root;
    let url: string | null = null;
    if (typeof post.postUrl === "string") {
      const parsed = new URL(post.postUrl);
      if (
        parsed.protocol === "https:" &&
        ["www.tiktok.com", "tiktok.com"].includes(parsed.hostname) &&
        /^\/@[^/]+\/(?:video|photo)\/\d+\/?$/.test(parsed.pathname)
      )
        url = parsed.href;
    }
    const rawSound = post.soundSelection ?? root.soundSelection;
    const sound =
      rawSound && typeof rawSound === "object" && !Array.isArray(rawSound)
        ? (rawSound as Record<string, unknown>)
        : null;
    return {
      url,
      sound:
        sound &&
        typeof sound.title === "string" &&
        typeof sound.artist === "string" &&
        typeof sound.section === "string" &&
        typeof sound.index === "number" &&
        typeof sound.candidatesDigest === "string"
          ? {
              title: sound.title,
              artist: sound.artist,
              section: sound.section,
              index: sound.index,
              digest: sound.candidatesDigest,
              confirmed: sound.confirmed === true,
            }
          : null,
    };
  } catch {
    return empty;
  }
}

function stableSoundSeed(value: string): number {
  let seed = 0x811c9dc5;
  for (const char of value) {
    seed ^= char.charCodeAt(0);
    seed = Math.imul(seed, 0x01000193);
  }
  return seed >>> 0;
}

function deviceDisplayName(
  devices: SelProps["devices"],
  metas: Map<string, import("../types").DeviceMeta>,
  udid: string,
): string {
  const ordered = orderDevicesByNumber(devices, metas);
  const index = ordered.findIndex((device) => device.udid === udid);
  const device = ordered[index];
  const meta = metas.get(udid);
  return device
    ? `Máy ${tileNumber(index + 1, meta)} · ${tileName(device, meta)}`
    : "Máy chưa kết nối";
}

function sameOrderedTargets(
  left: readonly string[],
  right: readonly string[],
): boolean {
  return (
    left.length === right.length &&
    left.every((udid, index) => udid === right[index])
  );
}

type PublishPageProps = SelProps & {
  scopeControl?: ReactNode;
  targetUdids?: string[];
  targetRef?: TargetRef;
  onTargetRefChange?: (target: TargetRef) => void;
  metas?: Map<string, import("../types").DeviceMeta>;
  operationSource?: OperationSourceRef;
};
type AsyncState = "idle" | "loading" | "ready" | "error";

export function PublishPage({
  devices,
  selected,
  targetUdids,
  targetRef = { type: "all" },
  onTargetRefChange,
  metas = new Map(),
  operationSource,
  scopeControl,
}: PublishPageProps) {
  const [workspaceTab, setWorkspaceTab] = useState<AutomationMode>(
    "setup",
  );
  const { guards: deviceGuards, failed: guardsFailed, refresh: refreshDeviceGuards } = usePublishDeviceGuards(devices.map(device => device.udid));
  const [restoredForm] = useState(readPublishForm);
  const [restoringForm, setRestoringForm] = useState(Boolean(restoredForm?.sourceRoot));
  const [sourceRoot, setSourceRoot] = useState(restoredForm?.sourceRoot ?? "");
  const [manifest, setManifest] = useState<PublishFolderManifest | null>(null);
  const [bundleIds, setBundleIds] = useState<string[]>(restoredForm?.bundleIds ?? []);
  const [assignments, setAssignments] = useState<Record<string, string>>(restoredForm?.assignments ?? {});
  const [captionDrafts, setCaptionDrafts] = useState<Record<string, string>>(
    restoredForm?.captionDrafts ?? {},
  );
  // Setup always posts immediately; daily schedules own a separate draft.
  const [sheetConnectionReady, setSheetConnectionReady] = useState(false);
  const [sheetConnectionRevision, setSheetConnectionRevision] = useState(0);
  const updateSheetConnection = useCallback((ready: boolean) => {
    setSheetConnectionReady(ready);
    setSheetConnectionRevision(revision => revision + 1);
  }, []);
  const [createdCampaignId, setCreatedCampaignId] = useState<string>();
  const [soundPolicyOverride, setSoundPolicyOverride] =
    useState<PublishSoundPolicy | null>(restoredForm?.soundPolicyOverride ?? null);
  // New campaigns always bind Sheet delivery and remove their imported device media
  // only after the public post is verified. Historical campaign flags remain untouched.
  const sheetEnabled = true;
  const deleteAfterPublish = true;
  const [campaigns, setCampaigns] = useState<PublishCampaignRecord[]>([]);
  const [campaignLoadState, setCampaignLoadState] = useState<
    "loading" | "ready" | "error"
  >("loading");
  const [campaignLoadError, setCampaignLoadError] = useState<string | null>(
    null,
  );
  const [operationBusy, setBusy] = useState(false);
  const publishInFlight = useRef(false);
  const [limitsRevision, setLimitsRevision] = useState(0);
  const [scanning, setScanning] = useState(false);
  const busy = operationBusy || scanning;
  const [scanError, setScanError] = useState<PublishScanErrorView | null>(null);
  const scanTicket = useRef(0);
  const latestSourceRoot = useRef(sourceRoot);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      scanTicket.current += 1;
    };
  }, []);
  const invalidateScan = () => {
    setRestoringForm(false);
    scanTicket.current += 1;
    setScanning(false);
    setScanError(null);
  };
  const editSourceRoot = (value: string) => {
    invalidateScan();
    latestSourceRoot.current = value;
    setSourceRoot(value);
  };
  const [notice, setNotice] = useState<{
    tone: NoticeTone;
    text: string;
  } | null>(null);
  const [details, setDetails] = useState<Record<string, PublishCampaignDetail>>(
    {},
  );
  const [recoveryCapabilities, setRecoveryCapabilities] = useState<Record<string, PublishRecoveryCapability[]>>({});
  const [recoveryErrors, setRecoveryErrors] = useState<Record<string, string>>({});
  const [detailErrors, setDetailErrors] = useState<Record<string, string>>({});
  const [detailLoading, setDetailLoading] = useState<Record<string, boolean>>(
    {},
  );
  const [executionSnapshots, setExecutionSnapshots] = useState<
    Record<string, PublishExecutionSnapshot>
  >({});
  const [operations, setOperations] = useState<
    Record<string, OperationRunSummary>
  >({});
  const [operationError, setOperationError] = useState<string | null>(null);
  const openDetailIds = useRef(new Set<string>());
  const detailTickets = useRef(new Map<string, number>());
  useEffect(() => {
    Object.keys(details).forEach((id) => openDetailIds.current.add(id));
  }, [details]);
  const loadCampaignDetail = useCallback(async (campaignId: string, reconcile: boolean) => {
    const ticket = (detailTickets.current.get(campaignId) ?? 0) + 1;
    detailTickets.current.set(campaignId, ticket);
    const isCurrent = () => mounted.current && openDetailIds.current.has(campaignId)
      && detailTickets.current.get(campaignId) === ticket;
    setDetailErrors((current) => {
      const next = { ...current };
      delete next[campaignId];
      return next;
    });
    setDetailLoading((current) => ({ ...current, [campaignId]: true }));
    try {
      // Reconciliation announces another event. Automatic refresh must remain read-only
      // instead of calling reconcile again and creating a feedback loop.
      const snapshot = reconcile ? await publishReconcile(campaignId) : null;
      if (!isCurrent()) return;
      const [detail, projection, recovery] = await Promise.all([
        publishGet(campaignId),
        operationGetRun(`publish:${campaignId}`)
          .then((operation) => ({ operation, error: null as string | null }))
          .catch((error) => ({ operation: null, error: describeError(error) })),
        publishRecoveryCapabilities(campaignId)
          .then(capabilities => ({ capabilities, error: "" }))
          .catch(error => ({ capabilities: [] as PublishRecoveryCapability[], error: describeError(error) })),
      ]);
      if (!isCurrent()) return;
      if (!detail || detail.campaign.id !== campaignId) throw new Error("Chiến dịch không còn trong dữ liệu hoặc kết quả trả về không khớp.");
      if (projection.operation) setOperations((current) => ({ ...current, [campaignId]: projection.operation!.summary }));
      setOperationError(projection.error);
      setRecoveryCapabilities(current => ({ ...current, [campaignId]: recovery.capabilities }));
      setRecoveryErrors(current => ({ ...current, [campaignId]: recovery.error }));
      if (snapshot) setExecutionSnapshots((current) => ({ ...current, [campaignId]: snapshot }));
      setDetails((current) => ({ ...current, [campaignId]: detail }));
      setCampaigns(current => current.some(campaign => campaign.id === campaignId) ? current : [detail.campaign, ...current]);
      setSourceCampaign((current) => current?.id === campaignId ? detail.campaign : current);
    } catch (error) {
      if (isCurrent()) {
        setDetailErrors((current) => ({ ...current, [campaignId]: describeError(error) }));
        setRecoveryCapabilities(current => ({ ...current, [campaignId]: [] }));
        setRecoveryErrors(current => ({ ...current, [campaignId]: describeError(error) }));
      }
    } finally {
      if (isCurrent()) setDetailLoading((current) => ({ ...current, [campaignId]: false }));
    }
  }, []);
  const [preflightState, setPreflightState] = useState<AsyncState>("idle");
  const [preflightStage, setPreflightStage] = useState<"preparing" | "checking" | null>(null);
  const [preflightError, setPreflightError] = useState<string | null>(null);
  const [preflightSnapshot, setPreflightSnapshot] = useState<{
    inputKey: string;
    report: PublishPreflightReport;
  } | null>(null);
  const [sourceCampaign, setSourceCampaign] =
    useState<PublishCampaignRecord | null>(null);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [sourceLoading, setSourceLoading] = useState(false);
  const sourceId =
    operationSource?.kind === "publish" ? operationSource.sourceId : undefined;
  useEffect(() => {
    setCreatedCampaignId(undefined);
    if (!sourceId) {
      setSourceCampaign(null);
      setSourceError(null);
      return;
    }
    let active = true;
    setWorkspaceTab("monitor");
    setSourceCampaign(null);
    setSourceError(null);
    setSourceLoading(true);
    void Promise.all([publishGet(sourceId), publishRecoveryCapabilities(sourceId)
      .then(capabilities => ({ capabilities, error: "" }))
      .catch(error => ({ capabilities: [] as PublishRecoveryCapability[], error: describeError(error) }))])
      .then(([detail, recovery]) => {
        if (!active) return;
        setRecoveryCapabilities(current => ({ ...current, [sourceId]: recovery.capabilities }));
        setRecoveryErrors(current => ({ ...current, [sourceId]: recovery.error }));
        if (!detail || detail.campaign.id !== sourceId)
          throw new Error(
            "Chiến dịch được chọn không còn trong nguồn dữ liệu.",
          );
        setSourceCampaign(detail.campaign);
        setDetails((current) => ({ ...current, [sourceId]: detail }));
      })
      .catch((error) => {
        if (active) setSourceError(describeError(error));
      })
      .finally(() => {
        if (active) setSourceLoading(false);
      });
    return () => {
      active = false;
    };
  }, [sourceId]);

  const eligibleTargets = useMemo(
    () => targetUdids ?? targetsOf(selected, devices),
    [targetUdids, selected, devices],
  );
  const selectedBundles =
    manifest?.bundles.filter((bundle) => bundleIds.includes(bundle.id)) ?? [];
  const assignedUdids = selectedBundles.map(
    (bundle) => assignments[bundle.id] ?? "",
  );
  const targets = assignedUdids;
  const effectiveTargetRef: TargetRef = sameOrderedTargets(
    targets,
    eligibleTargets,
  )
    ? targetRef
    : { type: "explicit", udids: targets };
  const orderedBundleIds = selectedBundles.map((bundle) => bundle.id);
  const currentCaptionOverrides = Object.fromEntries(
    selectedBundles.map((bundle) => [
      bundle.id,
      (captionDrafts[bundle.id] ?? bundle.caption).trim(),
    ]),
  );
  const currentSoundPolicy = soundPolicyOverride ?? {
    kind: "trendingAny" as const,
    poolSize: 5,
    seed: stableSoundSeed(
      JSON.stringify({
        sourceRoot: sourceRoot.trim(),
        bundleIds: orderedBundleIds,
        targets,
        runAt: null,
        captions: orderedBundleIds.map((id) => currentCaptionOverrides[id]),
      }),
    ),
  };
  const preflightRequest: PublishPreflightRequest = {
    deleteAfterPublish,
    sheetEnabled,
    sourceRoot: sourceRoot.trim(),
    bundleIds: orderedBundleIds,
    udids: targets,
    targetRef: effectiveTargetRef,
    runAt: null,
    captionOverrides: currentCaptionOverrides,
    soundPolicy: currentSoundPolicy,
  };
  const sheetBlocked = sheetEnabled && !sheetConnectionReady;
  const sheetBlockingReason = sheetBlocked ? "Kiểm tra và xác minh link Sheet trong tab Thiết lập trước khi ghi kết quả." : undefined;
  const pendingBlockingReason = targets.filter(Boolean).map(udid => deviceGuardBlock(deviceGuards, udid)).find(Boolean);
  const publishBlockingReason = pendingBlockingReason ?? sheetBlockingReason;
  const inputKey = JSON.stringify({ request: preflightRequest, sheetBlocked, sheetConnectionRevision: sheetEnabled ? sheetConnectionRevision : 0 });
  const latestInputKey = useRef(inputKey);
  latestInputKey.current = inputKey;
  const preflightTicket = useRef(0);
  useEffect(() => {
    preflightTicket.current += 1;
    setPreflightState("idle");
    setPreflightStage(null);
    setPreflightError(null);
    setPreflightSnapshot(null);
  }, [inputKey]);
  const draftSnapshot = useMemo(
    () => ({
      sourceRoot,
      bundleIds,
      assignments,
      captionDrafts,
      runAt: "",
      targetRef,
      soundPolicyOverride,
      sheetEnabled,
      deleteAfterPublish,
    }),
    [
      sourceRoot,
      bundleIds,
      assignments,
      captionDrafts,
      targetRef,
      soundPolicyOverride,
      sheetEnabled,
      deleteAfterPublish,
    ],
  );
  const draftKey = JSON.stringify(draftSnapshot);
  const latestDraftKey = useRef(draftKey);
  latestDraftKey.current = draftKey;
  const [baseline, setBaseline] = useState(draftSnapshot);
  const [baselineManifest, setBaselineManifest] = useState(manifest);
  const dirty = draftKey !== JSON.stringify(baseline);
  useWorkspaceDraft({
    id: "publish",
    label: "Đăng bài",
    dirty,
    snapshotKey: draftKey,
    autoSave: () => writeFormDraft("publish", draftSnapshot),
    onAutoSaveError: error => setNotice({ tone: "error", text: `Chưa tự lưu được thiết lập: ${describeError(error)}` }),
    save: () => {
      try {
        writeFormDraft("publish", draftSnapshot);
        setBaseline(draftSnapshot);
        setBaselineManifest(manifest);
        return Promise.resolve(true);
      } catch (error) {
        setNotice({ tone: "error", text: describeError(error) });
        return Promise.resolve(false);
      }
    },
    discard: () => {
      invalidateScan();
      latestSourceRoot.current = baseline.sourceRoot;
      setSourceRoot(baseline.sourceRoot);
      setBundleIds(baseline.bundleIds);
      setAssignments(baseline.assignments);
      setCaptionDrafts(baseline.captionDrafts);
      setSoundPolicyOverride(baseline.soundPolicyOverride);
      setManifest(baselineManifest);
      onTargetRefChange?.(baseline.targetRef);
      setPreflightSnapshot(null);
    },
  });
  const selectionStatus = publishSelectionStatus({ selectedIds: bundleIds, bundles: manifest?.bundles ?? [],
    assignments, captions: currentCaptionOverrides, eligible: eligibleTargets,
    ready: devices.filter(device => device.status === "ready" || device.status === "busy" || device.status === "connected").map(device => device.udid), blockingReason: sheetBlockingReason });
  const selectionReady = selectionStatus.ready;
  const currentPreflight =
    preflightSnapshot?.inputKey === inputKey ? preflightSnapshot.report : null;
  useEffect(() => {
    if (!restoredForm?.sourceRoot) return;
    const ticket = ++scanTicket.current;
    void publishScanFolder(restoredForm.sourceRoot).then(next => {
      if (!mounted.current || ticket !== scanTicket.current) return;
      setManifest(next);
      setBundleIds(restoredForm.bundleIds.filter(id => next.bundles.some(b => b.id === id)));
    }).catch(error => {
      if (mounted.current && ticket === scanTicket.current) setScanError(publishScanErrorView(error));
    }).finally(() => { if (mounted.current && ticket === scanTicket.current) setRestoringForm(false); });
  }, [restoredForm]);

  const invalidatePreflight = () => {
    preflightTicket.current += 1;
    setSoundPolicyOverride(null);
    setPreflightSnapshot(null);
    setPreflightState("idle");
    setPreflightStage(null);
    setPreflightError(null);
  };

  useEffect(() => {
    if (restoringForm || (restoredForm && devices.length === 0)) return;
    setAssignments((current) => {
      // Roster updates validate availability without erasing the operator's pairs.
      const next = reconcileAssignments(bundleIds, current, Object.values(current));
      return JSON.stringify(current) === JSON.stringify(next) ? current : next;
    });
  }, [eligibleTargets, bundleIds, restoringForm, restoredForm, devices.length]);

  const reloadTicket = useRef(0);
  const reload = () => {
    const ticket = ++reloadTicket.current;
    setCampaignLoadState((current) =>
      current === "ready" ? current : "loading",
    );
    setCampaignLoadError(null);
    return Promise.all([
      publishList(),
      operationListRuns(200)
        .then((runs) => ({ runs, error: null as string | null }))
        .catch((error) => ({ runs: [], error: describeError(error) })),
    ])
      .then(([next, projection]) => {
        if (!mounted.current || ticket !== reloadTicket.current) return;
        setCampaigns(next);
        setOperations(
          Object.fromEntries(
            projection.runs
              .filter((run) => run.kind === "publish")
              .map((run) => [run.sourceId, run]),
          ),
        );
        setOperationError(projection.error);
        setCampaignLoadState("ready");
      })
      .catch((error) => {
        if (!mounted.current || ticket !== reloadTicket.current) return;
        setCampaignLoadError(describeError(error));
        setCampaignLoadState("error");
      });
  };

  useEffect(() => {
    void reload();
    let unlisten: UnlistenFn | undefined;
    let live = true;
    listenRiviuEvents((event) => {
      if (!live || event.type !== "publishUpdated") return;
      void reload();
      void refreshDeviceGuards();
      if (openDetailIds.current.has(event.campaignId)) {
        // A previous retry projection is not evidence for a new campaign revision.
        setExecutionSnapshots((current) => {
          const next = { ...current };
          delete next[event.campaignId];
          return next;
        });
        void loadCampaignDetail(event.campaignId, false);
      }
    })
      .then((off) => {
        if (live) unlisten = off;
        else off();
      })
      .catch(() => undefined);
    return () => {
      live = false;
      reloadTicket.current += 1;
      unlisten?.();
    };
  }, [loadCampaignDetail, refreshDeviceGuards]);

  // Assignment transfer/progress commits can precede the next campaign event.
  // Refresh only the visible monitor; never reconcile or dispatch from this poll.
  const monitorRefresh = useRef<() => Promise<void>>(async () => {});
  monitorRefresh.current = async () => {
    if (!campaigns.some(c => ["queued", "transferring", "imported", "posting", "verifying"].includes(c.state))) return;
    await reload();
    await Promise.all([...openDetailIds.current].filter(id => !detailLoading[id])
      .map(id => loadCampaignDetail(id, false)));
  };
  useEffect(() => {
    if (workspaceTab !== "monitor") return;
    let active = true, inFlight = false;
    const timer = window.setInterval(() => {
      if (!active || inFlight) return;
      inFlight = true;
      void monitorRefresh.current().finally(() => { inFlight = false; });
    }, 5000);
    return () => { active = false; window.clearInterval(timer); };
  }, [workspaceTab]);

  const scan = async (path: string) => {
    setRestoringForm(false);
    if (!mounted.current) return;
    const root = path.trim();
    const ticket = ++scanTicket.current;
    const isCurrent = () =>
      mounted.current &&
      ticket === scanTicket.current &&
      latestSourceRoot.current === root;
    latestSourceRoot.current = root;
    setSourceRoot(root);
    setManifest(null);
    setBundleIds([]);
    setCaptionDrafts({});
    setScanError(null);
    setScanning(true);
    setNotice(null);
    invalidatePreflight();
    try {
      const next = await publishScanFolder(root);
      if (!isCurrent()) return;
      setManifest(next);
      setBundleIds([]);
      setAssignments({});
      setCaptionDrafts(
        Object.fromEntries(
          next.bundles.map((bundle) => [bundle.id, bundle.caption]),
        ),
      );
    } catch (error) {
      if (!isCurrent()) return;
      setManifest(null);
      setBundleIds([]);
      setCaptionDrafts({});
      setScanError(publishScanErrorView(error));
    } finally {
      if (isCurrent()) setScanning(false);
    }
  };

  const runPreflight = async () => {
    if (!selectionReady) {
      setPreflightState("error");
      setPreflightError(
        selectionStatus.reason,
      );
      return;
    }
    setPreflightState("loading");
    setPreflightStage("preparing");
    setPreflightError(null);
    const ticket = ++preflightTicket.current;
    const requestKey = inputKey;
    try {
      const request = preflightRequest;
      await operationPrepareDevices(targets);
      await refreshDeviceGuards();
      if (!mounted.current || ticket !== preflightTicket.current || latestInputKey.current !== requestKey) return;
      setPreflightStage("checking");
      const report = await publishPreflight(request);
      if (
        !mounted.current ||
        ticket !== preflightTicket.current ||
        latestInputKey.current !== requestKey
      )
        return;
      setPreflightSnapshot({ inputKey: requestKey, report });
      setPreflightState("ready");
      setPreflightStage(null);
    } catch (error) {
      if (
        !mounted.current ||
        ticket !== preflightTicket.current ||
        latestInputKey.current !== requestKey
      )
        return;
      setPreflightSnapshot(null);
      setPreflightError(describeError(error));
      setPreflightState("error");
      setPreflightStage(null);
    }
  };

  const executeNewCampaign = async () => {
    if (publishInFlight.current || operationBusy || !currentPreflight?.canExecute || sheetBlocked) return;
    publishInFlight.current = true;
    setBusy(true);
    setNotice(null);
    try {
      const approvedDraftKey = latestDraftKey.current;
      const confirmed = await requestConfirm({
        title: "Xác nhận đăng công khai?",
        message: `${selectedBundles.length} bài sẽ được đăng công khai trên ${targets.length} máy. Nhạc sẽ được chọn sau khi mở TikTok và xác nhận lại trước Đăng.`,
        confirmLabel: "Đăng bài",
        cancelLabel: "Huỷ",
        danger: true,
      });
      if (!confirmed) return;
      if (
        latestDraftKey.current !== approvedDraftKey ||
        latestInputKey.current !== inputKey
      ) {
        setNotice({
          tone: "warning",
          text: "Thiết lập đã đổi trong lúc xác nhận. Kiểm tra lại trước khi đăng.",
        });
        return;
      }
        // Persist before the IPC call; an ACK can be lost after campaign creation.
      // Only an acknowledged create retires this identity, so restart and retry
      // return the same campaign rather than preparing a second publication.
      const createKey = JSON.stringify(preflightRequest);
      const storageKey = "riviu.publish.pending-create.v1";
      let pending: { key: string; requestId: string } | null = null;
      const stored = localStorage.getItem(storageKey);
      if (stored) {
        try { pending = JSON.parse(stored); } catch { /* Replace malformed local draft metadata. */ }
      }
      const requestId = pending?.key === createKey && typeof pending.requestId === "string"
        ? pending.requestId : crypto.randomUUID();
      localStorage.setItem(storageKey, JSON.stringify({ key: createKey, requestId }));
      const campaign = await publishCreateCampaign(
        sourceRoot.trim(),
        orderedBundleIds,
        targets,
        null,
        currentCaptionOverrides,
        currentSoundPolicy,
        effectiveTargetRef,
        true,
        currentPreflight.inputDigest,
        sheetEnabled,
        deleteAfterPublish,
        requestId,
      );
      localStorage.removeItem(storageKey);
      setBaseline(draftSnapshot);
      setBaselineManifest(manifest);
      setCreatedCampaignId(campaign.id);
      setWorkspaceTab("monitor");
      {
        const result = await publishExecute(campaign.id, true);
        setDetails((current) => ({ ...current, [campaign.id]: result.detail }));
        setNotice({
          tone:
            result.status === "complete"
              ? "success"
              : result.status === "uncertain"
                ? "warning"
                : "info",
          text:
            pendingPublicationMessage(result.detail, sheetEnabled) ?? (result.status === "complete"
              ? sheetEnabled
                ? "Đã đăng, lấy liên kết và ghi Sheet."
                : "Đã đăng và lấy liên kết. Không ghi Sheet."
              : result.status === "uncertain"
                ? "Có máy chưa xác định được kết quả sau thao tác Đăng. Quy trình đã dừng."
                : "Bài đã xử lý nhưng còn bước cần hoàn tất. Mở chi tiết để xem bước còn thiếu."),
        });
      }
      await reload();
    } catch (error) {
      setNotice({ tone: "error", text: describeError(error) });
    } finally {
      publishInFlight.current = false;
      setBusy(false);
    }
  };

  const retryCampaign = async (campaign: PublishCampaignRecord) => {
    if (publishInFlight.current || operationBusy) return;
    publishInFlight.current = true;
    try {
      setBusy(true);
      setNotice(null);
      let snapshot: PublishExecutionSnapshot;
      try {
        snapshot = await publishReconcile(campaign.id);
        setExecutionSnapshots((current) => ({
          ...current,
          [campaign.id]: snapshot,
        }));
        if (snapshot.retryScope === "none" || ((campaign.state === "verifying" || needsPublicationReview(campaign)) && snapshot.retryScope !== "linkAndSheet")) {
          setNotice({
            tone: "warning",
            text: "Trạng thái đã được đối chiếu và không có bước nào được phép tự chạy lại.",
          });
          return;
        }
      } catch (error) {
        setNotice({
          tone: "error",
          text: `Không đối chiếu được chiến dịch: ${describeError(error)}`,
        });
        return;
      } finally {
        setBusy(false);
      }
      if (snapshot.retryScope === "linkAndSheet") {
        try {
          const capabilities = await publishRecoveryCapabilities(campaign.id);
          setRecoveryCapabilities(current => ({ ...current, [campaign.id]: capabilities }));
          if (!capabilities.some(item => item.checkLink.allowed)) {
            setNotice({ tone: "warning", text: recoveryReason(capabilities.find(item => item.checkLink.reason)?.checkLink.reason)
              ?? "Chưa xác nhận được quyền kiểm tra liên kết; tải lại chi tiết." });
            return;
          }
        } catch (error) {
          setRecoveryCapabilities(current => ({ ...current, [campaign.id]: [] }));
          setNotice({ tone: "warning", text: `Chưa đọc được quyền kiểm tra liên kết: ${describeError(error)}` });
          return;
        }
      }
      const confirmed = await requestConfirm({
        title: snapshot.retryScope === "linkAndSheet" ? "Kiểm tra liên kết bài đã gửi?" : snapshot.retryScope === "sheetOnly" ? "Ghi lại kết quả lên Sheet?" : "Xác nhận tiếp tục đăng bài?",
        message: `${retryScopeLabel(snapshot.retryScope, snapshotSheetEnabled(snapshot))}. Trạng thái chưa chắc chắn không được tự đăng lại.`,
        confirmLabel: "Tiếp tục",
      });
      if (!confirmed) return;
      setBusy(true);
      setNotice(null);
      try {
        if (snapshot.retryScope === "linkAndSheet") {
          const checked = await publishCheckLinks(campaign.id);
          await reload();
          await loadCampaignDetail(campaign.id, false);
          void refreshDeviceGuards();
          const verified = checked.outcomes.filter(outcome => outcome.verified).length;
          const pending = checked.outcomes.filter(outcome => outcome.status === "pending").length;
          const errors = checked.outcomes.flatMap(outcome => outcome.error ? [recoveryReason(outcome.error) ?? outcome.error] : []);
          const skipped = checked.outcomes.filter(outcome => !outcome.verified && outcome.status !== "pending");
          const explanations = {
            busy: "Máy đang bận; chưa kiểm tra được liên kết",
            stopped: "Lượt xác minh đã dừng; cần tiếp tục xác minh rõ ràng",
            stale: "Kết quả đã thay đổi; tải lại chi tiết",
            noCandidate: "Không có bài đủ điều kiện kiểm tra",
            ineligible: "Bài chưa đủ điều kiện kiểm tra liên kết",
          };
          const skippedText = [...new Set(skipped.map(outcome => explanations[outcome.status as keyof typeof explanations] ?? "Chưa xác nhận kết quả kiểm tra"))].join(". ");
          setNotice({ tone: errors.length || skipped.length || !checked.outcomes.length ? "warning" : "info",
            text: !checked.outcomes.length ? (recoveryReason(checked.reason) || "Không có bài đủ điều kiện kiểm tra liên kết; xem trạng thái và lý do của từng máy.")
              : [`${verified} bài đã xác minh liên kết.`, pending ? `${pending} bài đã bấm Đăng, chưa lấy được liên kết xác minh; Riviu tự kiểm tra khi máy rảnh, không đăng lại.${snapshotSheetEnabled(snapshot) ? " Sheet chờ liên kết đã xác minh." : ""}` : "",
                skippedText, ...errors].filter(Boolean).join(" ") });
          return;
        }
        const result = await publishExecute(campaign.id, true);
        setDetails((current) => ({ ...current, [campaign.id]: result.detail }));
        await reload();
        setNotice({
          tone:
            result.status === "complete"
              ? "success"
              : result.status === "uncertain"
                ? "warning"
                : "info",
          text:
            pendingPublicationMessage(result.detail, snapshotSheetEnabled(snapshot)) ?? (result.status === "complete"
              ? snapshotSheetEnabled(snapshot)
                ? "Đã hoàn tất đăng bài và ghi Sheet."
                : "Đã đăng và lấy liên kết. Không ghi Sheet."
              : result.status === "uncertain"
                ? "Kết quả sau thao tác Đăng chưa chắc chắn; app không tự đăng lại."
                : "Quy trình còn bước chưa hoàn tất. Xem chi tiết để xử lý tiếp."),
        });
      } catch (error) {
        setNotice({ tone: "error", text: describeError(error) });
      } finally {
        setBusy(false);
      }
    } finally { publishInFlight.current = false; }
  };

  const retryAssignment = async (assignment: PublishAssignmentRecord, campaign: PublishCampaignRecord) => {
    if (publishInFlight.current || operationBusy || !canRetryAssignment(assignment, campaign)
      || !recoveryCapabilities[campaign.id]?.find(item => item.assignmentId === assignment.id)?.retryBeforePost.allowed) return;
    publishInFlight.current = true;
    setBusy(true);
    setNotice(null);
    try {
      const machine = deviceDisplayName(devices, metas, assignment.udid);
      const confirmed = await requestConfirm({
        title: `Thử lại bài của ${machine}?`,
        message: `Bài của ${machine} đã dừng trước khi bấm Đăng. Chỉ bài này được kiểm tra và xếp lại vào hàng chờ; các bài khác tiếp tục trạng thái hiện tại. Nhạc tự chọn chưa xác minh sẽ được chọn lại theo cấu hình của bài.`,
        confirmLabel: "Thử lại máy này",
        cancelLabel: "Huỷ",
        danger: true,
      });
      if (!confirmed) return;
      const capability=recoveryCapabilities[campaign.id]?.find(item=>item.assignmentId===assignment.id);
      if (!capability) throw Error("Tải lại quyền thử lại của bài này.");
      await publishRetryAssignment(assignment.id, true, capability.revision, crypto.randomUUID());
      await reload();
      await loadCampaignDetail(campaign.id, true);
      setNotice({ tone: "info", text: `Đã nhận yêu cầu thử lại bài của ${machine}. Theo dõi trạng thái từng máy để xem kết quả.` });
    } catch (error) {
      setNotice({ tone: "error", text: describeError(error) });
    } finally {
      publishInFlight.current = false;
      setBusy(false);
    }
  };

  const resumeVerification = async (assignment: PublishAssignmentRecord, capability: PublishRecoveryCapability) => {
    if (publishInFlight.current || operationBusy || !capability.resumeVerification.allowed) return;
    publishInFlight.current = true;
    setBusy(true);
    try {
      const confirmed = await requestConfirm({
        title: "Tiếp tục xác minh bài đã gửi?",
        message: "Chỉ kiểm tra bài cũ trên máy này và thử lại mỗi 5 phút khi chưa có link. Không đăng lại, không tiếp tục các bài chưa gửi. Quyền đăng bài mới chỉ mở khi có đủ bằng chứng máy đã an toàn.",
        confirmLabel: "Tiếp tục xác minh", cancelLabel: "Huỷ",
      });
      if (!confirmed) return;
      const result = await publishResumeVerification(assignment.id, true, capability.revision);
      await reload();
      await loadCampaignDetail(assignment.campaignId, false);
      void refreshDeviceGuards();
      const refused = result.state === "stale" || result.state === "ineligible";
      setNotice({ tone: refused ? "warning" : "info", text: recoveryReason(result.reason) || (refused
        ? "Trạng thái bài đã thay đổi hoặc chưa đủ điều kiện xác minh; không khởi động lại Post."
        : result.state === "alreadyVerified" ? "Bài đã được xác minh; không thực hiện lại Post."
          : "Đã nhận yêu cầu xác minh bài đã gửi. Khi chưa có link, hệ thống thử lại mỗi 5 phút; không đăng lại.") });
    } catch (error) { setNotice({ tone: "error", text: describeError(error) }); }
    finally { publishInFlight.current = false; setBusy(false); }
  };

  const stopVerification = async (campaign: PublishCampaignRecord) => {
    if (publishInFlight.current || operationBusy) return;
    publishInFlight.current = true;
    setBusy(true);
    try {
      const confirmed = await requestConfirm({ title: "Dừng kiểm tra lại?",
        message: "Dừng các lượt xác minh được tiếp tục trong chiến dịch này. Bài đã gửi và liên kết đã có được giữ nguyên; không đăng lại.",
        confirmLabel: "Dừng kiểm tra", cancelLabel: "Huỷ" });
      if (!confirmed) return;
      const result = await operationStop(`publish:${campaign.id}`);
      await reload();
      await loadCampaignDetail(campaign.id, false);
      void refreshDeviceGuards();
      setNotice({ tone: result.state === "failed" || result.state === "needsAttention" ? "warning" : "info",
        text: result.state === "closed" ? "Đã dừng kiểm tra lại; giữ nguyên bài đã gửi." : result.state === "stopping"
          ? "Đã nhận yêu cầu dừng kiểm tra; đang chờ nhả máy." : "Chưa hoàn tất dừng kiểm tra. Xem trạng thái tác vụ, không gửi lại Post." });
    } catch (error) { setNotice({ tone: "error", text: describeError(error) }); }
    finally { publishInFlight.current = false; setBusy(false); }
  };

  const toggleCampaignDetail = async (campaign: PublishCampaignRecord) => {
    if ((details[campaign.id] || detailLoading[campaign.id]) && !detailErrors[campaign.id]) {
      openDetailIds.current.delete(campaign.id);
      detailTickets.current.set(campaign.id, (detailTickets.current.get(campaign.id) ?? 0) + 1);
      setDetailLoading((current) => ({ ...current, [campaign.id]: false }));
      setDetails((current) => {
        const next = { ...current };
        delete next[campaign.id];
        return next;
      });
      setExecutionSnapshots((current) => {
        const next = { ...current };
        delete next[campaign.id];
        return next;
      });
      return;
    }
    openDetailIds.current.add(campaign.id);
    await loadCampaignDetail(campaign.id, true);
  };
  const openPendingPublication = (campaignId: string) => {
    setCreatedCampaignId(campaignId); setWorkspaceTab("monitor");
    openDetailIds.current.add(campaignId);
    void loadCampaignDetail(campaignId, true);
  };

  return (
    <main className="panel publish-page">
      <AutomationTabs id="publish" label="Chế độ Đăng bài" value={workspaceTab} onChange={setWorkspaceTab} />
      <PublishHostLimits onSaved={() => setLimitsRevision(revision => revision + 1)} />
      {guardsFailed && <div className="publish-global-notice"><StatusNotice tone="warning">{UNKNOWN_PUBLISH_GUARD}. <button type="button" onClick={() => void refreshDeviceGuards()}>Kiểm tra lại trạng thái bài</button></StatusNotice></div>}
      {notice && (
        <div className="publish-global-notice">
          <StatusNotice tone={notice.tone}>{notice.text}</StatusNotice>
        </div>
      )}
      {scanError && (
        <div className="publish-global-notice">
          <StatusNotice tone="error">
            <strong>{scanError.title}</strong>
            {scanError.detail && <p>{scanError.detail}</p>}
            {scanError.raw !== scanError.title && (
              <details>
                <summary>Chi tiết lỗi quét</summary>
                <code>{scanError.raw}</code>
              </details>
            )}
          </StatusNotice>
        </div>
      )}

      <div className="publish-tab-panel" role="tabpanel" id="publish-panel-schedule" aria-labelledby="publish-tab-schedule" hidden={workspaceTab !== "schedule"}>
        <PublishSchedulePlanner key={sourceRoot}
          active={workspaceTab === "schedule"} sourceReady={!scanning && !restoringForm}
          limitsRevision={limitsRevision}
          selectedIds={bundleIds} assignments={assignments} eligible={eligibleTargets}
          deviceGuards={deviceGuards} onPendingPublication={openPendingPublication}
          sourceRoot={sourceRoot} bundles={manifest?.bundles ?? []} devices={devices} metas={metas}
          captions={captionDrafts} sound={currentSoundPolicy}
          blockingReason={sheetBlockingReason}
          onSource={()=>setWorkspaceTab("setup")} onCreated={()=>{void reload();}}
          onSheetSetup={() => { setWorkspaceTab("setup"); requestAnimationFrame(() => {
            const toggle = document.querySelector<HTMLButtonElement>(".pq-settings-toggle");
            if (toggle?.getClientRects().length && toggle.getAttribute("aria-expanded") === "false") toggle.click();
            requestAnimationFrame(() => {
              const target = document.querySelector<HTMLElement>("[data-google-sheet-focus]") ?? document.getElementById("publish-sheet-link");
              target?.scrollIntoView({ block: "nearest" }); target?.focus({ preventScroll: true });
            });
          }); }}
          onHistory={()=>setWorkspaceTab("monitor")}
        />
      </div>
      <div className="publish-tab-panel" role="tabpanel" id="publish-panel-setup" aria-labelledby="publish-tab-setup" hidden={workspaceTab !== "setup"}>
        <PublishWizard
          scopeControl={scopeControl}
          active={workspaceTab === "setup"}
          sourceRoot={sourceRoot}
          manifest={manifest}
          selectedIds={bundleIds}
          assignments={assignments}
          captions={captionDrafts}
          devices={devices}
          metas={metas}
          eligible={eligibleTargets}
          deviceGuards={deviceGuards} onPendingPublication={openPendingPublication}
          busy={operationBusy}
          scanning={scanning || restoringForm}
          preflightLoading={preflightState === "loading"}
          preflightStage={preflightStage}
          preflight={currentPreflight}
          preflightError={publishBlockingReason ?? preflightError}
          blockingReason={sheetBlockingReason}
          sound={currentSoundPolicy}
          runAt=""
          onSource={(path) => {
            editSourceRoot(path);
            setManifest(null);
            setBundleIds([]);
            setAssignments({});
            setCaptionDrafts({});
            invalidatePreflight();
          }}
          onScan={scan}
          onAssignmentChange={(ids, next) => {
            setBundleIds(ids);
            setAssignments(next);
            invalidatePreflight();
          }}
          onSelect={(ids) => {
            setBundleIds(ids);
            invalidatePreflight();
          }}
          onAssign={(next) => {
            setAssignments(next);
            invalidatePreflight();
          }}
          onCaption={(id, value) => {
            setCaptionDrafts((current) => ({ ...current, [id]: value }));
            invalidatePreflight();
          }}
          onRunAt={() => {}}
          onPreflight={runPreflight}
          onExecute={executeNewCampaign}
          onHistory={() => setWorkspaceTab("monitor")}
          settings={<PublishSheetConnection onReadyChange={updateSheetConnection} />}
        />
      </div>

      <section
        role="tabpanel"
        aria-labelledby="publish-tab-monitor"
        id="publish-panel-monitor"
        className="publish-workspace-section"
        aria-label="Theo dõi"
        hidden={workspaceTab !== "monitor"}
      >
        {sourceLoading && (
          <LoadingState label="Đang mở chiến dịch được chọn…" />
        )}
        {sourceError && <StatusNotice tone="error">{sourceError}</StatusNotice>}
        <div className="publish-monitor-head">
          <div>
            <h2>Tiến độ chiến dịch</h2>
            <p>Xem bài đã đăng, liên kết và những máy cần xử lý.</p>
          </div>
          <button
            type="button"
            className="ghost"
            onClick={() => void reload()}
            disabled={busy}
          >
            <RefreshCw size={16} aria-hidden="true" /> Làm mới
          </button>
        </div>
        {campaignLoadState === "loading" && (
          <LoadingState label="Đang tải chiến dịch…" />
        )}
        {campaignLoadState === "error" && (
          <StatusNotice
            tone="error"
            action={
              <button
                type="button"
                className="ghost"
                onClick={() => void reload()}
              >
                Thử lại
              </button>
            }
          >
            {campaignLoadError ?? "Không tải được chiến dịch."}
          </StatusNotice>
        )}
        {operationError && (
          <StatusNotice
            tone="warning"
            action={
              <button
                type="button"
                className="ghost"
                onClick={() => void reload()}
              >
                Thử đối chiếu lại
              </button>
            }
          >
            Chưa đối chiếu được kết quả link và Sheet. {operationError}
          </StatusNotice>
        )}
        {campaignLoadState === "ready" && campaigns.length === 0 && (
          <EmptyState
            compact
            icon={<IconRocket size={17} />}
            title="Chưa có chiến dịch"
            hint="Tạo chiến dịch ở thẻ Thiết lập để bắt đầu đăng bài."
          />
        )}
        {(sourceId && !createdCampaignId
          ? sourceCampaign !== null
          : campaignLoadState === "ready" && campaigns.length > 0) && (
          <CampaignMonitor
          onScheduleChanged={() => { void reload(); }}
            key={createdCampaignId ?? sourceId ?? "monitor"}
            initialSelectedId={createdCampaignId ?? sourceId}
            campaigns={
              sourceCampaign && !createdCampaignId
                ? [
                    campaigns.find(
                      (campaign) => campaign.id === sourceCampaign.id,
                    ) ?? sourceCampaign,
                  ]
                : campaigns
            }
            busy={busy}
            details={details}
            detailErrors={detailErrors}
            detailLoading={detailLoading}
            executionSnapshots={executionSnapshots}
            operations={operations}
            devices={devices}
            metas={metas}
            retryCampaign={retryCampaign}
            retryAssignment={retryAssignment}
            recoveryCapabilities={recoveryCapabilities}
            recoveryErrors={recoveryErrors}
            resumeVerification={resumeVerification}
            stopVerification={stopVerification}
            toggleDetail={toggleCampaignDetail}
            cancel={async (campaign) => {
              setBusy(true);
              try {
                await publishCancel(campaign.id);
                await reload();
              } catch (error) {
                setNotice({ tone: "error", text: describeError(error) });
              } finally {
                setBusy(false);
              }
            }}
          />
        )}
      </section>
    </main>
  );
}

function CampaignMonitor({
  initialSelectedId,
  onScheduleChanged,
  campaigns,
  busy,
  details,
  detailErrors,
  detailLoading,
  executionSnapshots,
  operations,
  devices,
  metas,
  retryCampaign,
  retryAssignment,
  recoveryCapabilities,
  recoveryErrors,
  resumeVerification,
  stopVerification,
  toggleDetail,
  cancel,
}: {
  initialSelectedId?: string;
  onScheduleChanged: () => void;
  campaigns: PublishCampaignRecord[];
  busy: boolean;
  details: Record<string, PublishCampaignDetail>;
  detailErrors: Record<string, string>;
  detailLoading: Record<string, boolean>;
  executionSnapshots: Record<string, PublishExecutionSnapshot>;
  operations: Record<string, OperationRunSummary>;
  devices: SelProps["devices"];
  metas: Map<string, import("../types").DeviceMeta>;
  retryCampaign: (campaign: PublishCampaignRecord) => Promise<void>;
  retryAssignment: (assignment: PublishAssignmentRecord, campaign: PublishCampaignRecord) => Promise<void>;
  recoveryCapabilities: Record<string, PublishRecoveryCapability[]>;
  recoveryErrors: Record<string, string>;
  resumeVerification: (assignment: PublishAssignmentRecord, capability: PublishRecoveryCapability) => Promise<void>;
  stopVerification: (campaign: PublishCampaignRecord) => Promise<void>;
  toggleDetail: (campaign: PublishCampaignRecord) => Promise<void>;
  cancel: (campaign: PublishCampaignRecord) => Promise<void>;
}) {
  const [filter, setFilter] = useState<"all" | "scheduled" | "active" | "attention" | "done">("all");
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null | undefined>(initialSelectedId);
  const detailRef = useRef<HTMLDivElement>(null);
  useEffect(() => { if (initialSelectedId) setSelectedId(initialSelectedId); }, [initialSelectedId]);
  const bucket = (campaign: PublishCampaignRecord) => {
    if (campaign.state === "scheduled") return "scheduled";
    const view = campaignView(campaign, operations[campaign.id], executionSnapshots[campaign.id], details[campaign.id]);
    if (view.label === "Hoàn tất") return "done";
    if (view.tone === "error" || view.tone === "warning" || campaign.state === "uncertain") return "attention";
    return "active";
  };
  const filtered = campaigns.filter(campaign => (filter === "all" || bucket(campaign) === filter)
    && `${campaign.sourceRoot} ${campaigns.indexOf(campaign) + 1} ${new Date(campaign.createdAt).toLocaleString("vi-VN")} ${campaign.runAt ? new Date(campaign.runAt).toLocaleString("vi-VN") : ""}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const selected = selectedId === undefined
    ? filtered.find(campaign => details[campaign.id])
    : filtered.find(campaign => campaign.id === selectedId);
  const selectedView = selected ? campaignView(selected, operations[selected.id], executionSnapshots[selected.id], details[selected.id]) : null;
  const selectedRecovery = selected ? recoveryCapabilities[selected.id] : undefined;
  const checkingLinks = selected && selectedView && (selected.state === "verifying" || needsPublicationReview(selected) || selectedView.retryScope === "linkAndSheet");
  const linkAllowed = selectedRecovery?.some(item => item.checkLink.allowed) === true;
  const linkBlockedReason = recoveryReason(selectedRecovery?.find(item => item.checkLink.reason)?.checkLink.reason)
    ?? "Chưa xác nhận được quyền kiểm tra liên kết; tải lại chi tiết.";
  const name = (campaign: PublishCampaignRecord) => campaign.sourceRoot.split(/[\\/]/).filter(Boolean).at(-1) ?? "Nguồn bài đăng";
  const choose = (campaign: PublishCampaignRecord) => {
    if (selected && selected.id !== campaign.id && (details[selected.id] || detailLoading[selected.id])) void toggleDetail(selected);
    setSelectedId(campaign.id);
    if (!details[campaign.id] && !detailLoading[campaign.id]) void toggleDetail(campaign);
    if (window.matchMedia?.("(max-width: 900px)").matches) detailRef.current?.scrollIntoView({ block: "start" });
  };
  return <div className="publish-campaigns">
    <div className="publish-monitor-toolbar">
      <div className="publish-monitor-filters" role="group" aria-label="Lọc chiến dịch">
        {([["all", "Tất cả"], ["scheduled", "Đã hẹn"], ["active", "Đang chạy"], ["attention", "Cần xử lý"], ["done", "Hoàn tất"]] as const).map(([value, label]) =>
          <button key={value} type="button" aria-pressed={filter === value} onClick={() => setFilter(value)}>{label}<span>{campaigns.filter(c => value === "all" || bucket(c) === value).length}</span></button>)}
      </div>
      <label className="publish-monitor-search"><Search size={15} aria-hidden="true"/><input aria-label="Tìm chiến dịch" placeholder="Tìm nguồn hoặc ngày đăng" value={query} onChange={e => setQuery(e.target.value)}/></label>
    </div>
    <div className="publish-monitor-layout">
      <div className="publish-run-list" role="list" aria-label="Chiến dịch đăng bài">
        {filtered.map(campaign => {
          const view = campaignView(campaign, operations[campaign.id], executionSnapshots[campaign.id], details[campaign.id]);
          return <div role="listitem" key={campaign.id} className={`publish-run-card ${selected?.id === campaign.id ? "is-selected" : ""}`}>
            <button type="button" className="publish-run-title" aria-current={selected?.id === campaign.id ? "true" : undefined} onClick={() => choose(campaign)}>
              <span><strong>Chiến dịch {campaigns.indexOf(campaign) + 1}</strong><small>{campaign.assignments.length} bài · {new Date(campaign.createdAt).toLocaleString("vi-VN")}</small></span>
              <StatusChip tone={view.tone}>{view.label}</StatusChip>
            </button>
            <div className="publish-run-meta"><span title={campaign.sourceRoot}>{name(campaign)}{campaign.runAt ? ` · ${new Date(campaign.runAt).toLocaleString("vi-VN", {day:"2-digit",month:"2-digit",year:"numeric",hour:"2-digit",minute:"2-digit"})}` : ""}</span>
              <button type="button" className="ghost" disabled={detailLoading[campaign.id]} onClick={() => choose(campaign)}>Chi tiết máy <ArrowUpRight size={13} aria-hidden="true"/></button></div>
          </div>;
        })}
        {!filtered.length && <div className="publish-monitor-placeholder"><Search size={24}/><strong>Không có chiến dịch phù hợp</strong><span>Đổi bộ lọc hoặc từ khóa để xem các lượt khác.</span></div>}
      </div>
      <div className="publish-monitor-detail" ref={detailRef}>
        {selected && selectedView ? <>
          <div className="publish-monitor-detail-head"><div><h3>Chiến dịch {campaigns.indexOf(selected) + 1}</h3><p>{selected.assignments.length} bài · {name(selected)}</p></div>
            <div className="publish-row-actions">
              {selectedView.retryScope !== "none" && <button type="button" className="primary" disabled={busy || detailLoading[selected.id] || (!!checkingLinks && !linkAllowed)} title={checkingLinks && !linkAllowed ? linkBlockedReason : undefined} onClick={() => void retryCampaign(selected)}>{selected.state === "verifying" || needsPublicationReview(selected) ? "Kiểm tra liên kết" : retryActionLabel(selectedView.retryScope, snapshotSheetEnabled(executionSnapshots[selected.id]))}</button>}
              {recoveryCapabilities[selected.id]?.some(capability => capability.verificationResumed) && <button type="button" className="ghost" disabled={busy} onClick={() => void stopVerification(selected)}>Dừng kiểm tra lại</button>}
              <PublishScheduleRetime key={selected.id} campaign={selected} onSaved={onScheduleChanged}/>
              {CANCELLABLE_STATES.includes(selected.state) && <button type="button" className="ghost" onClick={() => void cancel(selected)}>Huỷ</button>}
              <button type="button" className="ghost" onClick={() => { setSelectedId(null); if (details[selected.id] || detailLoading[selected.id]) void toggleDetail(selected); }}>Ẩn chi tiết máy</button>
            </div>
          </div>
          {checkingLinks && !linkAllowed && <p role="status">{linkBlockedReason}</p>}
          <CampaignDetail detail={details[selected.id]} error={detailErrors[selected.id]} loading={detailLoading[selected.id] === true}
            snapshot={executionSnapshots[selected.id]} devices={devices} metas={metas} retry={() => void toggleDetail(selected)}
            busy={busy} retryAssignment={assignment => void retryAssignment(assignment, selected)}
            recoveryCapabilities={recoveryCapabilities[selected.id]} recoveryError={recoveryErrors[selected.id]}
            resumeVerification={(assignment, capability) => void resumeVerification(assignment, capability)}/>
        </> : <div className="publish-monitor-placeholder"><ListChecks size={30}/><strong>Chọn một chiến dịch để theo dõi</strong><span>Kết quả từng máy, link và ghi chú sẽ hiện tại đây.</span></div>}
      </div>
    </div>
  </div>;
}

function isComposing(assignment: PublishAssignmentRecord): boolean {
  if (assignment.state !== "imported") return false;
  try { return JSON.parse(assignment.evidenceJson ?? "null")?.pipelinePhase === "composing"; }
  catch { return false; }
}

function CampaignDetail({
  detail,
  error,
  loading,
  snapshot,
  devices,
  metas,
  retry,
  busy,
  retryAssignment,
  recoveryCapabilities,
  recoveryError,
  resumeVerification,
}: {
  detail?: PublishCampaignDetail;
  error?: string;
  loading: boolean;
  snapshot?: PublishExecutionSnapshot;
  devices: SelProps["devices"];
  metas: Map<string, import("../types").DeviceMeta>;
  retry: () => void;
  busy: boolean;
  retryAssignment: (assignment: PublishAssignmentRecord) => void;
  recoveryCapabilities?: PublishRecoveryCapability[];
  recoveryError?: string;
  resumeVerification: (assignment: PublishAssignmentRecord, capability: PublishRecoveryCapability) => void;
}) {
  if (!detail && !error && !loading) return null;
  // Current assignment evidence outranks a completed snapshot from before the
  // latest read. Submission alone must never claim Post or Sheet completion.
  const pendingPost = detail?.campaign.state === "verifying"
    || detail?.assignments.some((assignment) => assignment.state === "verifying") === true;
  const reviewPost = detail?.assignments.some(needsPublicationReview) === true;
  const supersededSheet = detail?.assignments.some(assignment => assignment.sheetDelivery?.state === "superseded") === true;
  return (
    <section
      className="publish-campaign-detail"
      aria-label="Chi tiết chiến dịch đang chọn"
    >
      {loading && (
        <LoadingState label="Đang đối chiếu trạng thái chiến dịch…" />
      )}
      {error && (
        <StatusNotice
          tone="error"
          action={
            <button type="button" className="ghost" onClick={retry}>
              Thử lại
            </button>
          }
        >
          {error}
        </StatusNotice>
      )}
      {detail && (
        <>
          {recoveryError && <StatusNotice tone="warning">Chưa đọc được quyền phục hồi: {recoveryError}. Các thao tác phục hồi tạm khóa; tải lại chi tiết để kiểm tra.</StatusNotice>}
          {snapshot && (
            <div className="publish-reconcile-summary">
              <StatusChip
                tone={
                  reviewPost || supersededSheet ? "warning" : pendingPost ? "info" : snapshot.status === "complete"
                    ? "success"
                    : snapshot.status === "uncertain"
                      ? "warning"
                      : "info"
                }
              >
                {reviewPost ? "Cần kiểm tra bài đăng" : pendingPost ? "Đang chờ xác minh bài đăng" : supersededSheet ? "Đợt báo cáo đã đóng" : snapshot.status === "complete"
                  ? "Đã hoàn tất"
                  : snapshot.status === "uncertain"
                    ? "Kết quả chưa chắc chắn"
                    : "Còn bước cần hoàn tất"}
              </StatusChip>
              <span>{reviewPost ? "Đã dừng kiểm tra tự động; mở TikTok kiểm tra bài đăng hoặc bản nháp" : pendingPost ? "Bài đang chờ xác minh sẽ không được đăng lại" : retryScopeLabel(snapshot.retryScope, snapshotSheetEnabled(snapshot))}</span>
              <span>
                {supersededSheet ? "Nghĩa vụ ghi Sheet cũ đã đóng; kết quả giữ trong app"
                  : !snapshotSheetEnabled(snapshot)
                  ? "Không ghi Sheet"
                  : reviewPost || pendingPost ? "Sheet chờ liên kết đã xác minh" : snapshot.status === "complete"
                    ? "Sheet đã xác nhận"
                    : snapshot.retryScope === "sheetOnly"
                      ? "Sheet chưa hoàn tất"
                      : "Sheet chưa xác nhận hoàn tất"}
              </span>
            </div>
          )}
          <div className="publish-reconcile-summary" role="status" aria-label="Tiến độ từng máy">
            <span>{detail.assignments.filter(a => ["queued", "scheduled", "ready", "preparing"].includes(a.state)).length} máy chờ tải</span>
            <span>{detail.assignments.filter(a => a.state === "transferring").length} máy đang tải</span>
            <span>{detail.assignments.filter(a => a.state === "imported" && !isComposing(a)).length} máy đã tải, chờ đăng</span>
            <span>{detail.assignments.filter(isComposing).length} máy đang chuẩn bị bài trên TikTok</span>
            <span>{detail.assignments.filter(a => a.state === "posting").length} máy đang gửi bài</span>
            <span>{detail.assignments.filter(a => a.state === "verifying").length} máy chờ liên kết</span>
            <span>{detail.assignments.filter(a => ["uncertain", "failedBeforeDispatch", "missed"].includes(a.state)).length} máy cần xử lý</span>
          </div>
          <ResponsiveTable
            label="Kết quả theo máy"
            rows={detail.assignments}
            keyForRow={(assignment) => assignment.id}
            columns={[
              {
                id: "device",
                label: "Máy",
                render: (assignment: PublishAssignmentRecord) => {
                  const raw = assignmentRaw(assignment);
                  return (
                    <span>
                      {deviceDisplayName(devices, metas, assignment.udid)}
                      <details
                        className="publish-technical-details"
                        aria-label="Chi tiết kỹ thuật máy"
                      >
                        <summary>Chi tiết</summary>
                        <code>{raw}</code>
                      </details>
                    </span>
                  );
                },
              },
              {
                id: "state",
                label: "Kết quả",
                render: (assignment: PublishAssignmentRecord) => {
                  const capability = recoveryCapabilities?.find(item => item.assignmentId === assignment.id);
                  const retryable = canRetryAssignment(assignment, detail.campaign);
                  const retryReason = recoveryReason(capability?.retryBeforePost.reason) ?? "Chưa xác nhận được quyền thử lại; tải lại chi tiết.";
                  return <span>
                    {capability?.recovery?.state==="waitingDevice"&&<p>Mất kết nối · chờ đúng máy kết nối lại, còn {Math.max(0,Math.ceil(((capability.recovery.reconnectDeadline??0)-Date.now())/1000))} giây</p>}
                    {capability?.recovery?.state==="retryWaiting"&&<p>Thử lại {capability.recovery.retriesUsed}/{capability.recovery.maxRetries} · {capability.recovery.step}</p>}
                    {needsPublicationReview(assignment) ? <>Cần kiểm tra bài đăng<p>{publicationReviewReason(assignment.evidenceJson) ?? "Chưa có đủ bằng chứng xác nhận bài; kiểm tra TikTok trước khi tiếp tục."}</p></>
                      : <>{isComposing(assignment) ? "Đang chuẩn bị bài trên TikTok" : PUBLISH_STATE_LABELS[assignment.state] ?? "Trạng thái chưa nhận diện"}{dispatchDetail(assignment) && <p>{dispatchDetail(assignment)}</p>}{verificationDetail(assignment.evidenceJson) && <p>{verificationDetail(assignment.evidenceJson)}</p>}</>}
                    <PrePostFailureEvidence assignment={assignment}/>
                    {retryable && <><button type="button" disabled={busy || loading || !capability?.retryBeforePost.allowed}
                      aria-label={`Thử lại trước khi Đăng · ${deviceDisplayName(devices, metas, assignment.udid)}`}
                      onClick={() => retryAssignment(assignment)}>Thử lại máy này</button>
                      {!capability?.retryBeforePost.allowed && <p>{retryReason}</p>}</>}
                    {capability?.resumeVerification.allowed && <button type="button" disabled={busy || loading}
                      aria-label={`Tiếp tục xác minh bài đã gửi · ${deviceDisplayName(devices, metas, assignment.udid)}`}
                      onClick={() => resumeVerification(assignment, capability)}>Tiếp tục xác minh bài đã gửi</button>}
                    {capability && !capability.resumeVerification.allowed && capability.resumeVerification.reason && assignment.effectIntent && !capability.verificationResumed
                      && assignment.state !== "succeeded" && <p>{recoveryReason(capability.resumeVerification.reason)}</p>}
                    {capability?.verificationResumed && <p role="status">Đã tiếp tục xác minh bài cũ; không mở lại quyền Đăng.</p>}
                  </span>;
                },
              },
              {
                id: "link",
                label: "Bài đã đăng",
                render: (assignment: PublishAssignmentRecord) => {
                  const evidence = postEvidence(assignment.evidenceJson);
                  return evidence.url ? (
                    <a href={evidence.url} target="_blank" rel="noreferrer" aria-label="Mở bài đã xác nhận">
                      Đã xác minh link · Mở bài
                    </a>
                  ) : (
                    <span>{needsPublicationReview(assignment) ? "Chưa có liên kết; không tự đăng lại" : assignment.state === "verifying" ? "Đã gửi bài; chờ TikTok hoàn tất và xác minh liên kết" : "Chưa có liên kết xác nhận"}</span>
                  );
                },
              },
              {
                id: "delivery",
                label: "Ghi Sheet",
                render: (assignment: PublishAssignmentRecord) => {
                  const delivery = assignment.sheetDelivery;
                  if (!delivery) return snapshot && !snapshotSheetEnabled(snapshot)
                    ? "Không ghi Sheet" : "Chờ liên kết đã xác minh";
                  if (delivery.state === "superseded") return "Đợt báo cáo đã đóng · giữ lịch sử trong app";
                  const next = delivery.nextAttemptAtMs == null ? null
                    : new Date(delivery.nextAttemptAtMs).toLocaleTimeString("vi-VN");
                  return <span>
                    {delivery.state === "sent" ? "Sheet đã xác nhận"
                      : delivery.nextAttemptAtMs == null ? "Cần xử lý ghi Sheet" : "Đang chờ ghi Sheet"}
                    {delivery.state !== "sent" && delivery.lastError && <p>{delivery.lastError}</p>}
                    {delivery.attempts > 0 && <p>Đã thử {delivery.attempts} lần · Gần nhất: {new Date(delivery.updatedAt).toLocaleTimeString("vi-VN")}</p>}
                    {delivery.state !== "sent" && next && <p>Thử tiếp: {next}</p>}
                    {delivery.state !== "sent" && delivery.nextAttemptAtMs == null && <p>Sửa nguyên nhân rồi chọn Ghi lại Sheet</p>}
                  </span>;
                },
              },
              {
                id: "sound",
                label: "Nhạc đã chọn",
                render: (assignment: PublishAssignmentRecord) => {
                  const sound = postEvidence(assignment.evidenceJson).sound;
                  return sound ? (
                    <span>
                      {sound.title} · {sound.artist}
                      <details className="publish-technical-details">
                        <summary>
                          {sound.confirmed
                            ? "Đã xác nhận nhạc"
                            : "Chưa xác nhận nhạc"}
                        </summary>
                        <dl>
                          <div>
                            <dt>Khu vực</dt>
                            <dd>
                              {sound.section === "trending"
                                ? "Thịnh hành"
                                : sound.section === "recommended"
                                  ? "Đề xuất"
                                  : "Chưa nhận diện"}
                            </dd>
                          </div>
                          <div>
                            <dt>Vị trí</dt>
                            <dd>{sound.index + 1}</dd>
                          </div>
                          <div>
                            <dt>Dấu xác nhận danh sách</dt>
                            <dd>
                              <code>{sound.digest}</code>
                            </dd>
                          </div>
                        </dl>
                      </details>
                    </span>
                  ) : (
                    "Chưa có bằng chứng nhạc"
                  );
                },
              },
              {
                id: "cleanup",
                label: "Dọn nội dung tạm",
                render: (assignment: PublishAssignmentRecord) => {
                  const cleanup = cleanupEvidence(assignment.evidenceJson);
                  return cleanup ? (
                    <span>
                      {cleanup.label}
                      <details
                        className="publish-technical-details"
                        aria-label="Chi tiết dọn nội dung"
                      >
                        <summary>Chi tiết</summary>
                        <code>{cleanup.raw}</code>
                      </details>
                    </span>
                  ) : (
                    "Chưa có bằng chứng"
                  );
                },
              },
            ]}
          />
        </>
      )}
    </section>
  );
}

function retryScopeLabel(
  scope: PublishExecutionSnapshot["retryScope"],
  sheetEnabled = true,
): string {
  switch (scope) {
    case "fullPipeline":
      return "Có thể chạy lại từ đầu";
    case "linkAndSheet":
      return sheetEnabled
        ? "Chỉ tiếp tục lấy liên kết và ghi Sheet"
        : "Chỉ tiếp tục lấy liên kết";
    case "sheetOnly":
      return "Chỉ tiếp tục ghi Sheet";
    case "none":
      return "Không có bước được phép tự chạy lại";
  }
}

function PrePostFailureEvidence({ assignment }: { assignment: PublishAssignmentRecord }) {
  if (assignment.state !== "failedBeforeDispatch") return null;
  let value: Record<string, unknown>;
  try {
    const parsed: unknown = JSON.parse(assignment.evidenceJson ?? "null");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    value = parsed as Record<string, unknown>;
  } catch { return null; }
  const message = typeof value.message === "string" ? value.message.slice(0, 2000) : null;
  const raw = value.selectionDiagnostic;
  const diagnostic = raw && typeof raw === "object" && !Array.isArray(raw) ? raw as Record<string, unknown> : null;
  if (!message && !diagnostic) return null;
  const count = (key: string) => typeof diagnostic?.[key] === "number" && Number.isSafeInteger(diagnostic[key]) && Number(diagnostic[key]) >= 0 ? Number(diagnostic[key]) : null;
  const expected = count("expectedCount"), selected = count("lastVerifiedCount"), scrolls = count("scrollCount");
  const reason = typeof diagnostic?.reasonCode === "string" ? diagnostic.reasonCode.slice(0, 120) : null;
  const stage = typeof diagnostic?.stage === "string" ? diagnostic.stage.slice(0, 80) : null;
  return <details className="publish-technical-details">
    <summary>Bằng chứng lỗi trước Đăng</summary>
    {message && <p>{message}</p>}
    {selected !== null && expected !== null && <p>Đã xác nhận {selected}/{expected} ảnh{scrolls !== null ? ` · ${scrolls} lần cuộn` : ""}</p>}
    {reason && <p><code>{stage ? `${stage} · ` : ""}{reason}</code></p>}
    {diagnostic?.artifactWriteFailed === true && <p>Không lưu đủ artifact; không coi lượt chọn ảnh là thành công.</p>}
  </details>;
}

function assignmentRaw(assignment: PublishAssignmentRecord): string {
  return [
    `UDID: ${assignment.udid}`,
    `state: ${assignment.state}`,
    assignment.publicationId ? `publicationId: ${assignment.publicationId}` : null,
    assignment.attemptId ? `attemptId: ${assignment.attemptId}` : null,
    assignment.errorCode ? `error: ${assignment.errorCode}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}
