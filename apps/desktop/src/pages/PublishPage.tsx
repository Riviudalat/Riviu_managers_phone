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

import {
  listenRiviuEvents,
  operationGetRun,
  operationListRuns,
  publishCancel,
  publishCreateCampaign,
  publishExecute,
  publishGet,
  publishList,
  publishPreflight,
  publishReconcile,
  publishScanFolder,
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
    const budget = typeof status?.reviewAfterMinutes === "number"
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
  const [sheetEnabled, setSheetEnabled] = useState(restoredForm?.sheetEnabled ?? false);
  const [deleteAfterPublish, setDeleteAfterPublish] = useState(restoredForm?.deleteAfterPublish ?? false);
  const [campaigns, setCampaigns] = useState<PublishCampaignRecord[]>([]);
  const [campaignLoadState, setCampaignLoadState] = useState<
    "loading" | "ready" | "error"
  >("loading");
  const [campaignLoadError, setCampaignLoadError] = useState<string | null>(
    null,
  );
  const [operationBusy, setBusy] = useState(false);
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
      const [detail, projection] = await Promise.all([
        publishGet(campaignId),
        operationGetRun(`publish:${campaignId}`)
          .then((operation) => ({ operation, error: null as string | null }))
          .catch((error) => ({ operation: null, error: describeError(error) })),
      ]);
      if (!isCurrent()) return;
      if (!detail || detail.campaign.id !== campaignId) throw new Error("Chiến dịch không còn trong dữ liệu hoặc kết quả trả về không khớp.");
      if (projection.operation) setOperations((current) => ({ ...current, [campaignId]: projection.operation!.summary }));
      setOperationError(projection.error);
      if (snapshot) setExecutionSnapshots((current) => ({ ...current, [campaignId]: snapshot }));
      setDetails((current) => ({ ...current, [campaignId]: detail }));
      setSourceCampaign((current) => current?.id === campaignId ? detail.campaign : current);
    } catch (error) {
      if (isCurrent()) setDetailErrors((current) => ({ ...current, [campaignId]: describeError(error) }));
    } finally {
      if (isCurrent()) setDetailLoading((current) => ({ ...current, [campaignId]: false }));
    }
  }, []);
  const [preflightState, setPreflightState] = useState<AsyncState>("idle");
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
    void publishGet(sourceId)
      .then((detail) => {
        if (!active) return;
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
  const inputKey = JSON.stringify({ request: preflightRequest, sheetBlocked, sheetConnectionRevision: sheetEnabled ? sheetConnectionRevision : 0 });
  const latestInputKey = useRef(inputKey);
  latestInputKey.current = inputKey;
  const preflightTicket = useRef(0);
  useEffect(() => {
    preflightTicket.current += 1;
    setPreflightState("idle");
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
      setDeleteAfterPublish(baseline.deleteAfterPublish);
      setCaptionDrafts(baseline.captionDrafts);
      setSoundPolicyOverride(baseline.soundPolicyOverride);
      setSheetEnabled(baseline.sheetEnabled);
      setManifest(baselineManifest);
      onTargetRefChange?.(baseline.targetRef);
      setPreflightSnapshot(null);
    },
  });
  const selectionStatus = publishSelectionStatus({ selectedIds: bundleIds, bundles: manifest?.bundles ?? [],
    assignments, captions: currentCaptionOverrides, eligible: eligibleTargets,
    ready: devices.filter(device => device.status === "ready").map(device => device.udid), blockingReason: sheetBlockingReason });
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
  }, [loadCampaignDetail]);

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
    setPreflightError(null);
    const ticket = ++preflightTicket.current;
    const requestKey = inputKey;
    try {
      const request = preflightRequest;
      const report = await publishPreflight(request);
      if (
        !mounted.current ||
        ticket !== preflightTicket.current ||
        latestInputKey.current !== requestKey
      )
        return;
      setPreflightSnapshot({ inputKey: requestKey, report });
      setPreflightState("ready");
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
    }
  };

  const executeNewCampaign = async () => {
    if (!currentPreflight?.canExecute || sheetBlocked) return;
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
    setBusy(true);
    setNotice(null);
    try {
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
      );
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
      setBusy(false);
    }
  };

  const retryCampaign = async (campaign: PublishCampaignRecord) => {
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
    const confirmed = await requestConfirm({
      title: "Xác nhận tiếp tục đăng bài?",
      message: `${retryScopeLabel(snapshot.retryScope, snapshotSheetEnabled(snapshot))}. Trạng thái chưa chắc chắn không được tự đăng lại.`,
      confirmLabel: "Tiếp tục",
    });
    if (!confirmed) return;
    setBusy(true);
    setNotice(null);
    try {
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

  return (
    <main className="panel publish-page">
      <AutomationTabs id="publish" label="Chế độ Đăng bài" value={workspaceTab} onChange={setWorkspaceTab} />
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
          selectedIds={bundleIds} assignments={assignments} eligible={eligibleTargets}
          sourceRoot={sourceRoot} bundles={manifest?.bundles ?? []} devices={devices} metas={metas}
          captions={captionDrafts} sound={currentSoundPolicy} sheet={sheetEnabled} cleanup={deleteAfterPublish}
          blockingReason={sheetBlockingReason}
          onSource={()=>setWorkspaceTab("setup")} onCreated={()=>{void reload();}}
          onSheetSetup={() => { setWorkspaceTab("setup"); requestAnimationFrame(() => document.getElementById("publish-sheet-link")?.focus()); }}
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
          busy={operationBusy}
          scanning={scanning || restoringForm}
          preflightLoading={preflightState === "loading"}
          preflight={currentPreflight}
          preflightError={sheetBlockingReason ?? preflightError}
          blockingReason={sheetBlockingReason}
          sound={currentSoundPolicy}
          sheet={sheetEnabled}
          cleanup={deleteAfterPublish}
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
          onSheet={(value) => {
            setSheetEnabled(value);
            invalidatePreflight();
          }}
          onCleanup={(value) => {
            setDeleteAfterPublish(value);
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
  toggleDetail: (campaign: PublishCampaignRecord) => Promise<void>;
  cancel: (campaign: PublishCampaignRecord) => Promise<void>;
}) {
  const [filter, setFilter] = useState<"all" | "scheduled" | "active" | "attention" | "done">("all");
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null | undefined>(initialSelectedId);
  const bucket = (campaign: PublishCampaignRecord) => {
    if (campaign.state === "scheduled") return "scheduled";
    const view = campaignView(campaign, operations[campaign.id], executionSnapshots[campaign.id]);
    if (view.label === "Hoàn tất") return "done";
    if (view.tone === "error" || view.tone === "warning" || campaign.state === "uncertain") return "attention";
    return "active";
  };
  const filtered = campaigns.filter(campaign => (filter === "all" || bucket(campaign) === filter)
    && `${campaign.sourceRoot} ${campaigns.indexOf(campaign) + 1} ${new Date(campaign.createdAt).toLocaleString("vi-VN")} ${campaign.runAt ? new Date(campaign.runAt).toLocaleString("vi-VN") : ""}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const selected = selectedId === undefined
    ? filtered.find(campaign => details[campaign.id])
    : filtered.find(campaign => campaign.id === selectedId);
  const selectedView = selected ? campaignView(selected, operations[selected.id], executionSnapshots[selected.id]) : null;
  const name = (campaign: PublishCampaignRecord) => campaign.sourceRoot.split(/[\\/]/).filter(Boolean).at(-1) ?? "Nguồn bài đăng";
  const choose = (campaign: PublishCampaignRecord) => {
    if (selected && selected.id !== campaign.id && (details[selected.id] || detailLoading[selected.id])) void toggleDetail(selected);
    setSelectedId(campaign.id);
    if (!details[campaign.id] && !detailLoading[campaign.id]) void toggleDetail(campaign);
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
          const view = campaignView(campaign, operations[campaign.id], executionSnapshots[campaign.id]);
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
      <div className="publish-monitor-detail">
        {selected && selectedView ? <>
          <div className="publish-monitor-detail-head"><div><h3>Chiến dịch {campaigns.indexOf(selected) + 1}</h3><p>{selected.assignments.length} bài · {name(selected)}</p></div>
            <div className="publish-row-actions">
              {selectedView.retryScope !== "none" && <button type="button" className="primary" disabled={busy} onClick={() => void retryCampaign(selected)}>{selected.state === "verifying" || needsPublicationReview(selected) ? "Kiểm tra liên kết" : retryActionLabel(selectedView.retryScope, snapshotSheetEnabled(executionSnapshots[selected.id]))}</button>}
              <PublishScheduleRetime key={selected.id} campaign={selected} onSaved={onScheduleChanged}/>
              {CANCELLABLE_STATES.includes(selected.state) && <button type="button" className="ghost" onClick={() => void cancel(selected)}>Huỷ</button>}
              <button type="button" className="ghost" onClick={() => { setSelectedId(null); if (details[selected.id] || detailLoading[selected.id]) void toggleDetail(selected); }}>Ẩn chi tiết máy</button>
            </div>
          </div>
          <CampaignDetail detail={details[selected.id]} error={detailErrors[selected.id]} loading={detailLoading[selected.id] === true}
            snapshot={executionSnapshots[selected.id]} devices={devices} metas={metas} retry={() => void toggleDetail(selected)}/>
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
}: {
  detail?: PublishCampaignDetail;
  error?: string;
  loading: boolean;
  snapshot?: PublishExecutionSnapshot;
  devices: SelProps["devices"];
  metas: Map<string, import("../types").DeviceMeta>;
  retry: () => void;
}) {
  if (!detail && !error && !loading) return null;
  // Current assignment evidence outranks a completed snapshot from before the
  // latest read. Submission alone must never claim Post or Sheet completion.
  const pendingPost = detail?.campaign.state === "verifying"
    || detail?.assignments.some((assignment) => assignment.state === "verifying") === true;
  const reviewPost = detail?.assignments.some(needsPublicationReview) === true;
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
          {snapshot && (
            <div className="publish-reconcile-summary">
              <StatusChip
                tone={
                  reviewPost ? "warning" : pendingPost ? "info" : snapshot.status === "complete"
                    ? "success"
                    : snapshot.status === "uncertain"
                      ? "warning"
                      : "info"
                }
              >
                {reviewPost ? "Cần kiểm tra bài đăng" : pendingPost ? "Đang chờ xác minh bài đăng" : snapshot.status === "complete"
                  ? "Đã hoàn tất"
                  : snapshot.status === "uncertain"
                    ? "Kết quả chưa chắc chắn"
                    : "Còn bước cần hoàn tất"}
              </StatusChip>
              <span>{reviewPost ? "Đã dừng kiểm tra tự động; mở TikTok kiểm tra bài đăng hoặc bản nháp" : pendingPost ? "Bài đang chờ xác minh sẽ không được đăng lại" : retryScopeLabel(snapshot.retryScope, snapshotSheetEnabled(snapshot))}</span>
              <span>
                {!snapshotSheetEnabled(snapshot)
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
            <span>{detail.assignments.filter(a => ["uncertain", "failedBeforeDispatch"].includes(a.state)).length} máy cần xử lý</span>
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
                render: (assignment: PublishAssignmentRecord) => needsPublicationReview(assignment)
                  ? <span>Cần kiểm tra bài đăng<p>{publicationReviewReason(assignment.evidenceJson) ?? "Chưa có đủ bằng chứng xác nhận bài; kiểm tra TikTok trước khi tiếp tục."}</p></span>
                  : <span>{isComposing(assignment) ? "Đang chuẩn bị bài trên TikTok" : PUBLISH_STATE_LABELS[assignment.state] ?? "Trạng thái chưa nhận diện"}{verificationDetail(assignment.evidenceJson) && <p>{verificationDetail(assignment.evidenceJson)}</p>}</span>,
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

function assignmentRaw(assignment: PublishAssignmentRecord): string {
  return [
    `UDID: ${assignment.udid}`,
    `state: ${assignment.state}`,
    assignment.errorCode ? `error: ${assignment.errorCode}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}
