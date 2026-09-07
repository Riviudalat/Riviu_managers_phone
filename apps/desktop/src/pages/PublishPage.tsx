import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { RefreshCw } from "lucide-react";
import { PublishWizard } from "../components/publish/PublishWizard";
import { reconcileAssignments } from "../components/publish/publishAssignments";

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
  publishSheetGetConfig,
  publishSheetSaveConfig,
} from "../api";
import { publishProfileConfig } from "../automationProfileConfig";
import {
  AutomationProfileControl,
  type AutomationProfileHandle,
} from "../components/AutomationProfileControl";
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
  SummaryRail,
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
  PublishSheetConfig,
  OperationRunSummary,
  AutomationDefinitionRecord,
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
  verifying: "Đang xác nhận",
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
  "failedBeforeDispatch",
];

function campaignTone(state: PublishCampaignRecord["state"]): StatusTone {
  if (state === "succeeded") return "warning";
  if (state === "uncertain" || state === "missed") return "warning";
  if (state === "failedBeforeDispatch" || state === "cancelled") return "error";
  return state === "queued" || state === "scheduled" ? "neutral" : "info";
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
  if (
    [
      "scheduled",
      "preparing",
      "transferring",
      "posting",
      "verifying",
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
    if (state === "kept") return { label: "đã giữ nội dung trên máy", raw };
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
}: PublishPageProps) {
  const [workspaceTab, setWorkspaceTab] = useState<"setup" | "monitor">(
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
  const [runAt, setRunAt] = useState(restoredForm?.runAt ?? "");
  const [soundPolicyOverride, setSoundPolicyOverride] =
    useState<PublishSoundPolicy | null>(restoredForm?.soundPolicyOverride ?? null);
  const [sheetEnabled, setSheetEnabled] = useState(restoredForm?.sheetEnabled ?? false);
  const [deleteAfterPublish, setDeleteAfterPublish] = useState(restoredForm?.deleteAfterPublish ?? false);
  const profileRef = useRef<AutomationProfileHandle>(null);
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
  const [sheetConfig, setSheetConfig] = useState<PublishSheetConfig | null>(
    null,
  );
  const [sheetLoadState, setSheetLoadState] = useState<
    "loading" | "ready" | "error"
  >("loading");
  const [sheetLoadError, setSheetLoadError] = useState<string | null>(null);
  const [sheetUrlDraft, setSheetUrlDraft] = useState("");
  const [sheetTokenDraft, setSheetTokenDraft] = useState("");
  const [sheetBusy, setSheetBusy] = useState(false);
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
        runAt: runAt || null,
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
    runAt: runAt || null,
    captionOverrides: currentCaptionOverrides,
    soundPolicy: currentSoundPolicy,
  };
  const inputKey = JSON.stringify(preflightRequest);
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
      runAt,
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
      runAt,
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
  const applyingProfile = useRef<TargetRef | null>(null);
  const profileNeedsGroupMapping = useRef(false);
  const dirty = draftKey !== JSON.stringify(baseline);
  useWorkspaceDraft({
    id: "publish",
    label: "Đăng bài",
    dirty,
    snapshotKey: draftKey,
    autoSave: () => writeFormDraft("publish", draftSnapshot),
    onAutoSaveError: error => setNotice({ tone: "error", text: `Chưa tự lưu được thiết lập: ${describeError(error)}` }),
    save: async () => {
      if (runAt) {
        setNotice({
          tone: "warning",
          text: "Lịch hẹn chưa được tạo. Xác nhận lịch hoặc xóa thời gian hẹn trước khi lưu hồ sơ.",
        });
        return false;
      }
      return (await profileRef.current?.save()) ?? false;
    },
    discard: () => {
      invalidateScan();
      latestSourceRoot.current = baseline.sourceRoot;
      setSourceRoot(baseline.sourceRoot);
      setBundleIds(baseline.bundleIds);
      setAssignments(baseline.assignments);
      setDeleteAfterPublish(baseline.deleteAfterPublish);
      setCaptionDrafts(baseline.captionDrafts);
      setRunAt(baseline.runAt);
      setSoundPolicyOverride(baseline.soundPolicyOverride);
      setSheetEnabled(baseline.sheetEnabled);
      setManifest(baselineManifest);
      onTargetRefChange?.(baseline.targetRef);
      setPreflightSnapshot(null);
    },
  });
  const mappingReady =
    selectedBundles.length > 0 &&
    selectedBundles.length === targets.length &&
    new Set(targets).size === targets.length &&
    targets.every((udid) => eligibleTargets.includes(udid));
  const captionsReady = Object.values(currentCaptionOverrides).every(
    (caption) => caption.length > 0,
  );
  const profileReady = mappingReady && captionsReady;
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
    if (
      applyingProfile.current &&
      JSON.stringify(applyingProfile.current) !== JSON.stringify(targetRef)
    )
      return;
    const mapResolvedGroup = profileNeedsGroupMapping.current;
    setAssignments((current) => {
      if (mapResolvedGroup) {
        return Object.fromEntries(
          bundleIds.map((id, index) => [id, eligibleTargets[index] ?? ""]),
        );
      }
      const next = reconcileAssignments(bundleIds, current, eligibleTargets);
      return JSON.stringify(current) === JSON.stringify(next) ? current : next;
    });
    profileNeedsGroupMapping.current = false;
  }, [eligibleTargets, bundleIds, targetRef, restoringForm, restoredForm, devices.length]);

  useEffect(() => {
    if (
      applyingProfile.current &&
      JSON.stringify(applyingProfile.current) === JSON.stringify(targetRef) &&
      mappingReady
    ) {
      applyingProfile.current = null;
      setBaseline(draftSnapshot);
      setBaselineManifest(manifest);
    }
  }, [draftSnapshot, mappingReady, manifest, targetRef]);

  const applyProfile = async (record: AutomationDefinitionRecord) => {
    invalidateScan();
    const ticket = scanTicket.current;
    const applyingKey = latestDraftKey.current;
    const config = record.revision.config;
    if (
      !config ||
      typeof config !== "object" ||
      Array.isArray(config) ||
      config.schemaVersion !== 1 ||
      typeof config.sourceRoot !== "string" ||
      !Array.isArray(config.bundleIds) ||
      !config.bundleIds.every((id): id is string => typeof id === "string")
    ) {
      throw new Error("Hồ sơ Đăng bài không đúng định dạng.");
    }
    const next = await publishScanFolder(config.sourceRoot);
    if (!mounted.current) return;
    if (ticket !== scanTicket.current)
      throw new Error("Thiết lập vừa thay đổi. Chọn lại hồ sơ để áp dụng.");
    if (latestDraftKey.current !== applyingKey)
      throw new Error("Thiết lập vừa thay đổi. Chọn lại hồ sơ để áp dụng.");
    if (
      !config.bundleIds.every((id) =>
        next.bundles.some((bundle) => bundle.id === id),
      )
    ) {
      throw new Error(
        "Nội dung hồ sơ đã thay đổi hoặc bị thiếu. Chọn lại thư mục trước khi đăng.",
      );
    }
    const captions = config.captionOverrides;
    if (
      !captions ||
      typeof captions !== "object" ||
      Array.isArray(captions) ||
      !Object.values(captions).every((caption) => typeof caption === "string")
    ) {
      throw new Error("Hồ sơ Đăng bài thiếu chú thích hợp lệ.");
    }
    const policy = config.soundPolicy;
    if (
      (config.sheetEnabled !== undefined &&
        typeof config.sheetEnabled !== "boolean") ||
      (config.deleteAfterPublish !== undefined &&
        typeof config.deleteAfterPublish !== "boolean")
    ) {
      throw new Error("Hồ sơ Đăng bài có lựa chọn Sheet sai định dạng.");
    }
    if (
      !policy ||
      typeof policy !== "object" ||
      Array.isArray(policy) ||
      (policy.kind !== "default" &&
        !(
          policy.kind === "trendingAny" &&
          typeof policy.poolSize === "number" &&
          typeof policy.seed === "number"
        ))
    ) {
      throw new Error("Hồ sơ Đăng bài thiếu lựa chọn nhạc hợp lệ.");
    }
    latestSourceRoot.current = config.sourceRoot;
    setSourceRoot(config.sourceRoot);
    setManifest(next);
    setBundleIds(config.bundleIds);
    setCaptionDrafts(captions as Record<string, string>);
    setRunAt("");
    setSoundPolicyOverride(policy as unknown as PublishSoundPolicy);
    setSheetEnabled(config.sheetEnabled !== false);
    const profileTargets =
      record.revision.targetRef.type === "explicit"
        ? record.revision.targetRef.udids
        : eligibleTargets;
    setAssignments(
      Object.fromEntries(
        next.bundles
          .filter((b) => (config.bundleIds as string[]).includes(b.id))
          .map((b, i) => [b.id, profileTargets[i] ?? ""]),
      ),
    );
    setDeleteAfterPublish(
      config.deleteAfterPublish === undefined
        ? true
        : config.deleteAfterPublish === true,
    );
    applyingProfile.current = record.revision.targetRef;
    profileNeedsGroupMapping.current =
      record.revision.targetRef.type !== "explicit";
    onTargetRefChange?.(record.revision.targetRef);
    setPreflightSnapshot(null);
    setPreflightState("idle");
    setPreflightError(null);
    setWorkspaceTab("setup");
  };

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
        if (ticket !== reloadTicket.current) return;
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
        if (ticket !== reloadTicket.current) return;
        setCampaignLoadError(describeError(error));
        setCampaignLoadState("error");
      });
  };

  useEffect(() => {
    void reload();
    let unlisten: UnlistenFn | undefined;
    let live = true;
    listenRiviuEvents((event) => {
      if (event.type === "publishUpdated") void reload();
    })
      .then((off) => {
        if (live) unlisten = off;
        else off();
      })
      .catch(() => undefined);
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);

  const sheetLoadTicket = useRef(0);
  const reloadSheetConfig = useCallback(async () => {
    const ticket = ++sheetLoadTicket.current;
    setSheetLoadState("loading");
    setSheetLoadError(null);
    try {
      const config = await publishSheetGetConfig();
      if (ticket !== sheetLoadTicket.current) return;
      setSheetConfig(config);
      setSheetUrlDraft(config.webhookUrl);
      setSheetLoadState("ready");
    } catch (error) {
      if (ticket !== sheetLoadTicket.current) return;
      setSheetConfig(null);
      setSheetLoadError(describeError(error));
      setSheetLoadState("error");
    }
  }, []);

  useEffect(() => {
    void reloadSheetConfig();
    return () => {
      sheetLoadTicket.current += 1;
    };
  }, [reloadSheetConfig]);

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
    if (!profileReady) {
      setPreflightState("error");
      setPreflightError(
        "Chọn đủ nội dung, máy đích và chú thích trước khi kiểm tra.",
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
    if (!currentPreflight?.canExecute) return;
    const approvedDraftKey = latestDraftKey.current;
    const confirmed = await requestConfirm({
      title: runAt ? "Xác nhận lập lịch đăng bài?" : "Xác nhận đăng công khai?",
      message: runAt
        ? `${selectedBundles.length} bài sẽ chạy trên ${targets.length} máy vào lịch đã chọn.`
        : `${selectedBundles.length} bài sẽ được đăng công khai trên ${targets.length} máy Nhạc sẽ được chọn sau khi mở TikTok và xác nhận lại trước Đăng.`,
      confirmLabel: runAt ? "Lập lịch" : "Đăng bài",
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
        runAt || null,
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
      setWorkspaceTab("monitor");
      if (!runAt) {
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
            result.status === "complete"
              ? sheetEnabled
                ? "Đã đăng, lấy liên kết và ghi Sheet."
                : "Đã đăng và lấy liên kết. Không ghi Sheet."
              : result.status === "uncertain"
                ? "Có máy chưa xác định được kết quả sau thao tác Đăng. Quy trình đã dừng."
                : "Bài đã xử lý nhưng còn bước cần hoàn tất. Mở chi tiết để xem phạm vi retry.",
        });
      } else {
        setNotice({
          tone: "success",
          text: `Đã lập lịch ${selectedBundles.length} bài cho ${targets.length} máy.`,
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
      if (snapshot.retryScope === "none") {
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
          result.status === "complete"
            ? snapshotSheetEnabled(snapshot)
              ? "Đã hoàn tất đăng bài và ghi Sheet."
              : "Đã đăng và lấy liên kết. Không ghi Sheet."
            : result.status === "uncertain"
              ? "Kết quả sau thao tác Đăng chưa chắc chắn; app không tự đăng lại."
              : "Quy trình còn bước chưa hoàn tất. Xem chi tiết để xử lý tiếp.",
      });
    } catch (error) {
      setNotice({ tone: "error", text: describeError(error) });
    } finally {
      setBusy(false);
    }
  };

  const toggleCampaignDetail = async (campaign: PublishCampaignRecord) => {
    if (details[campaign.id]) {
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
    setDetailErrors((current) => {
      const next = { ...current };
      delete next[campaign.id];
      return next;
    });
    setDetailLoading((current) => ({ ...current, [campaign.id]: true }));
    try {
      const snapshot = await publishReconcile(campaign.id);
      const [detail, operation] = await Promise.all([
        publishGet(campaign.id),
        operationGetRun(`publish:${campaign.id}`),
      ]);
      if (detail) {
        if (operation)
          setOperations((current) => ({
            ...current,
            [campaign.id]: operation.summary,
          }));
        setExecutionSnapshots((current) => ({
          ...current,
          [campaign.id]: snapshot,
        }));
        setDetails((current) => ({ ...current, [campaign.id]: detail }));
      } else
        setDetailErrors((current) => ({
          ...current,
          [campaign.id]: "Chiến dịch không còn trong dữ liệu.",
        }));
    } catch (error) {
      setDetailErrors((current) => ({
        ...current,
        [campaign.id]: describeError(error),
      }));
    } finally {
      setDetailLoading((current) => ({ ...current, [campaign.id]: false }));
    }
  };

  return (
    <main className="panel publish-page">
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

      {
        <PublishWizard
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
          preflightError={preflightError}
          sound={currentSoundPolicy}
          sheet={sheetEnabled}
          cleanup={deleteAfterPublish}
          runAt={runAt}
          onSource={(path) => {
            editSourceRoot(path);
            setManifest(null);
            setBundleIds([]);
            setAssignments({});
            setCaptionDrafts({});
            invalidatePreflight();
          }}
          onScan={scan}
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
          onRunAt={(value) => {
            setRunAt(value);
            invalidatePreflight();
          }}
          onPreflight={runPreflight}
          onExecute={executeNewCampaign}
          onHistory={() => setWorkspaceTab("monitor")}
          settings={
            <PublishAside
              deleteAfterPublish={deleteAfterPublish}
              manifest={manifest}
              selectedCount={selectedBundles.length}
              targetCount={targets.length}
              currentPreflight={currentPreflight}
              preflightState={preflightState}
              sheetConfig={sheetConfig}
              sheetEnabled={sheetEnabled}
              setSheetEnabled={setSheetEnabled}
              sheetLoadState={sheetLoadState}
              sheetLoadError={sheetLoadError}
              sheetUrlDraft={sheetUrlDraft}
              sheetTokenDraft={sheetTokenDraft}
              sheetBusy={sheetBusy}
              sourceRoot={sourceRoot}
              orderedBundleIds={orderedBundleIds}
              captionOverrides={currentCaptionOverrides}
              soundPolicy={currentSoundPolicy}
              targetRef={effectiveTargetRef}
              profileReady={profileReady && !runAt}
              pendingSchedule={Boolean(runAt)}
              busy={busy}
              setSheetUrlDraft={setSheetUrlDraft}
              setSheetTokenDraft={setSheetTokenDraft}
              setSheetConfig={setSheetConfig}
              reloadSheetConfig={reloadSheetConfig}
              setSheetBusy={setSheetBusy}
              setNotice={setNotice}
              profileRef={profileRef}
              dirty={dirty}
              applyProfile={applyProfile}
              profileSaved={() => {
                setBaseline(draftSnapshot);
                setBaselineManifest(manifest);
              }}
            />
          }
        />
      }

      <section
        id="publish-panel-monitor"
        className="publish-workspace-section"
        aria-label="Theo dõi"
        hidden={workspaceTab !== "monitor"}
      >
        <button
          type="button"
          className="ghost"
          onClick={() => setWorkspaceTab("setup")}
        >
          ← Về thiết lập
        </button>
        {sourceLoading && (
          <LoadingState label="Đang mở chiến dịch được chọn…" />
        )}
        {sourceError && <StatusNotice tone="error">{sourceError}</StatusNotice>}
        <div className="publish-monitor-head">
          <div>
            <h2>Tiến độ chiến dịch</h2>
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
        {(sourceId
          ? sourceCampaign !== null
          : campaignLoadState === "ready" && campaigns.length > 0) && (
          <CampaignMonitor
            campaigns={
              sourceCampaign
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

type NoticeSetter = (notice: { tone: NoticeTone; text: string } | null) => void;

function PublishAside({
  deleteAfterPublish,
  pendingSchedule,
  profileRef,
  dirty,
  applyProfile,
  profileSaved,
  manifest,
  selectedCount,
  targetCount,
  currentPreflight,
  preflightState,
  sheetConfig,
  sheetEnabled,
  setSheetEnabled,
  sheetLoadState,
  sheetLoadError,
  sheetUrlDraft,
  sheetTokenDraft,
  sheetBusy,
  sourceRoot,
  orderedBundleIds,
  captionOverrides,
  soundPolicy,
  targetRef,
  profileReady,
  busy,
  setSheetUrlDraft,
  setSheetTokenDraft,
  setSheetConfig,
  reloadSheetConfig,
  setSheetBusy,
  setNotice,
}: {
  deleteAfterPublish: boolean;
  pendingSchedule: boolean;
  profileRef: React.Ref<AutomationProfileHandle>;
  dirty: boolean;
  applyProfile: (record: AutomationDefinitionRecord) => Promise<void>;
  profileSaved: () => void;
  manifest: PublishFolderManifest | null;
  selectedCount: number;
  targetCount: number;
  currentPreflight: PublishPreflightReport | null;
  preflightState: AsyncState;
  sheetConfig: PublishSheetConfig | null;
  sheetEnabled: boolean;
  setSheetEnabled: (value: boolean) => void;
  sheetLoadState: "loading" | "ready" | "error";
  sheetLoadError: string | null;
  sheetUrlDraft: string;
  sheetTokenDraft: string;
  sheetBusy: boolean;
  sourceRoot: string;
  orderedBundleIds: string[];
  captionOverrides: Record<string, string>;
  soundPolicy: PublishPreflightRequest["soundPolicy"];
  targetRef: TargetRef;
  profileReady: boolean;
  busy: boolean;
  setSheetUrlDraft: (value: string) => void;
  setSheetTokenDraft: (value: string) => void;
  setSheetConfig: (value: PublishSheetConfig) => void;
  reloadSheetConfig: () => Promise<void>;
  setSheetBusy: (value: boolean) => void;
  setNotice: NoticeSetter;
}) {
  const canExecute = currentPreflight?.canExecute === true;
  const saveSheet = async () => {
    if (sheetBusy || sheetLoadState !== "ready") return false;
    setSheetBusy(true);
    try {
      const saved = await publishSheetSaveConfig(
        sheetUrlDraft,
        sheetTokenDraft === "" ? undefined : sheetTokenDraft,
      );
      setSheetConfig(saved);
      setSheetUrlDraft(saved.webhookUrl);
      setSheetTokenDraft("");
      setNotice({ tone: "success", text: "Đã lưu cấu hình Sheet." });
      return true;
    } catch (error) {
      setNotice({ tone: "error", text: describeError(error) });
      return false;
    } finally {
      setSheetBusy(false);
    }
  };
  useWorkspaceDraft({
    id: "publish-sheet",
    label: "Cấu hình Sheet",
    autoSave: saveSheet,
    dirty:
      sheetLoadState === "ready" &&
      (sheetUrlDraft !== sheetConfig?.webhookUrl || sheetTokenDraft !== ""),
    snapshotKey: JSON.stringify([sheetUrlDraft, sheetTokenDraft]),
    save: saveSheet,
    discard: () => {
      setSheetUrlDraft(sheetConfig?.webhookUrl ?? "");
      setSheetTokenDraft("");
    },
  });
  return (
    <div className="publish-workspace-aside">
      <SummaryRail title="Tóm tắt lượt chạy">
        <dl className="publish-summary-list">
          <div>
            <dt>Nguồn</dt>
            <dd>
              {manifest ? `${manifest.bundles.length} gói hợp lệ` : "Chưa quét"}
            </dd>
          </div>
          <div>
            <dt>Đã chọn</dt>
            <dd>{selectedCount} bài</dd>
          </div>
          <div>
            <dt>Máy đích</dt>
            <dd>{targetCount} máy</dd>
          </div>
          <div>
            <dt>Âm thanh</dt>
            <dd>
              {soundPolicy.kind === "default"
                ? "Âm thanh mặc định"
                : `Ngẫu nhiên trong tối đa ${soundPolicy.poolSize} đề xuất`}
            </dd>
          </div>
        </dl>
        <div className="publish-summary-status">
          <StatusChip
            tone={
              canExecute
                ? "success"
                : preflightState === "error"
                  ? "error"
                  : "neutral"
            }
          >
            {canExecute ? "Preflight đạt" : "Chưa có preflight hợp lệ"}
          </StatusChip>
          {!sheetEnabled ? (
            <StatusChip tone="neutral">Không ghi Sheet</StatusChip>
          ) : sheetLoadState === "error" ? (
            <StatusChip tone="error">Không đọc được Sheet</StatusChip>
          ) : sheetConfig ? (
            <StatusChip
              tone={
                sheetConfig.webhookUrl && sheetConfig.hasToken
                  ? "success"
                  : "warning"
              }
            >
              {sheetConfig.webhookUrl && sheetConfig.hasToken
                ? "Sheet sẵn sàng"
                : "Sheet chờ cấu hình"}
            </StatusChip>
          ) : null}
        </div>
        {currentPreflight && (
          <details className="publish-technical-details">
            <summary>Chi tiết lần kiểm tra</summary>
            <code>{currentPreflight.inputDigest}</code>
          </details>
        )}
        <AutomationProfileControl
          ref={profileRef}
          dirty={dirty}
          draftId="publish"
          onApply={applyProfile}
          onSaved={profileSaved}
          kind="publish"
          target={targetRef}
          config={publishProfileConfig(
            sourceRoot.trim(),
            orderedBundleIds,
            captionOverrides,
            soundPolicy,
            true,
            sheetEnabled,
            deleteAfterPublish,
          )}
          defaultName="Đăng bài theo thư mục"
          disabled={!profileReady || busy}
          disabledReason={
            pendingSchedule
              ? "Lịch hẹn chưa được tạo. Xác nhận lịch hoặc xóa thời gian hẹn trước khi lưu hồ sơ."
              : "Chọn đủ nội dung, máy đích và chú thích trước khi lưu hồ sơ."
          }
          confirmSave={() =>
            requestConfirm({
              title: "Cho phép hồ sơ đăng công khai?",
              message:
                "Mỗi lần chạy hồ sơ, app có thể chuyển nội dung, chọn nhạc và đăng công khai trên các máy đích.",
              confirmLabel: "Cho phép và lưu",
              cancelLabel: "Hủy",
              danger: true,
            })
          }
        />
      </SummaryRail>
      <label className="publish-sheet-toggle">
        <input
          type="checkbox"
          checked={sheetEnabled}
          disabled={busy}
          onChange={(event) => setSheetEnabled(event.target.checked)}
        />
        <span>Ghi kết quả lên Sheet</span>
      </label>
      {sheetEnabled && (
        <details className="publish-sheet-panel">
          <summary>Cấu hình Sheet</summary>
          {sheetLoadState === "loading" && (
            <LoadingState label="Đang đọc cấu hình Sheet…" />
          )}
          {sheetLoadState === "error" && (
            <StatusNotice
              tone="error"
              action={
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void reloadSheetConfig()}
                >
                  Thử lại
                </button>
              }
            >
              {sheetLoadError ?? "Không đọc được cấu hình Sheet."}
            </StatusNotice>
          )}
          {sheetConfig &&
            (!sheetConfig.webhookUrl || !sheetConfig.hasToken) && (
              <StatusNotice tone="warning">
                Sheet chưa cấu hình. Link đã xác nhận sẽ nằm trong hàng chờ.
              </StatusNotice>
            )}
          <label>
            <span>Webhook URL</span>
            <input
              type="url"
              aria-label="Webhook URL"
              value={sheetUrlDraft}
              onChange={(event) => setSheetUrlDraft(event.target.value)}
              placeholder="https://script.google.com/.../exec"
            />
          </label>
          <label>
            <span>Webhook token</span>
            <input
              type="password"
              aria-label="Webhook token"
              value={sheetTokenDraft}
              onChange={(event) => setSheetTokenDraft(event.target.value)}
              placeholder={
                sheetConfig?.hasToken ? "Để trống để giữ token" : "Nhập token"
              }
            />
          </label>
          <div className="publish-sheet-actions">
            <button
              type="button"
              className="primary"
              disabled={sheetBusy || sheetLoadState !== "ready"}
              onClick={() => void saveSheet()}
            >
              Lưu cấu hình
            </button>
            {sheetConfig?.hasToken && (
              <button
                type="button"
                className="ghost"
                disabled={sheetBusy || sheetLoadState !== "ready"}
                onClick={async () => {
                  setSheetBusy(true);
                  try {
                    const saved = await publishSheetSaveConfig(
                      sheetUrlDraft,
                      "",
                    );
                    setSheetConfig(saved);
                    setNotice({ tone: "success", text: "Đã xoá token." });
                  } catch (error) {
                    setNotice({ tone: "error", text: describeError(error) });
                  } finally {
                    setSheetBusy(false);
                  }
                }}
              >
                Xoá token
              </button>
            )}
          </div>
        </details>
      )}
    </div>
  );
}

function CampaignMonitor({
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
  return (
    <div className="publish-campaigns">
      <ResponsiveTable
        label="Chiến dịch đăng bài"
        rows={campaigns}
        keyForRow={(campaign) => campaign.id}
        columns={[
          {
            id: "campaign",
            label: "Chiến dịch",
            render: (campaign) => (
              <span className="publish-campaign-name">
                <strong>Chiến dịch {campaigns.indexOf(campaign) + 1}</strong>
                <small>{new Date(campaign.createdAt).toLocaleString()}</small>
              </span>
            ),
          },
          {
            id: "scope",
            label: "Phạm vi",
            render: (campaign) => `${campaign.assignments.length} bài`,
          },
          {
            id: "state",
            label: "Trạng thái",
            render: (campaign) => {
              const view = campaignView(
                campaign,
                operations[campaign.id],
                executionSnapshots[campaign.id],
              );
              return (
                <StatusChip tone={view.tone}>
                  {view.label ?? "Trạng thái chưa nhận diện"}
                </StatusChip>
              );
            },
          },
          {
            id: "actions",
            label: "Thao tác",
            render: (campaign) => (
              <div className="publish-row-actions">
                {campaignView(
                  campaign,
                  operations[campaign.id],
                  executionSnapshots[campaign.id],
                ).retryScope !== "none" && (
                  <button
                    type="button"
                    className="primary"
                    disabled={busy}
                    onClick={() => void retryCampaign(campaign)}
                  >
                    {retryActionLabel(
                      campaignView(
                        campaign,
                        operations[campaign.id],
                        executionSnapshots[campaign.id],
                      ).retryScope,
                      snapshotSheetEnabled(executionSnapshots[campaign.id]),
                    )}
                  </button>
                )}
                {CANCELLABLE_STATES.includes(campaign.state) && (
                  <button
                    type="button"
                    className="ghost"
                    disabled={busy}
                    onClick={() => void cancel(campaign)}
                  >
                    Huỷ
                  </button>
                )}
                <button
                  type="button"
                  className="ghost"
                  disabled={detailLoading[campaign.id] === true}
                  onClick={() => void toggleDetail(campaign)}
                >
                  {details[campaign.id]
                    ? "Ẩn chi tiết máy"
                    : !operations[campaign.id] && campaign.state === "succeeded"
                      ? "Đối chiếu kết quả"
                      : "Chi tiết máy"}
                </button>
              </div>
            ),
          },
        ]}
      />
      {campaigns.map((campaign) => (
        <CampaignDetail
          key={campaign.id}
          detail={details[campaign.id]}
          error={detailErrors[campaign.id]}
          loading={detailLoading[campaign.id] === true}
          snapshot={executionSnapshots[campaign.id]}
          devices={devices}
          metas={metas}
          retry={() => void toggleDetail(campaign)}
        />
      ))}
    </div>
  );
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
                  snapshot.status === "complete"
                    ? "success"
                    : snapshot.status === "uncertain"
                      ? "warning"
                      : "info"
                }
              >
                {snapshot.status === "complete"
                  ? "Đã hoàn tất"
                  : snapshot.status === "uncertain"
                    ? "Kết quả chưa chắc chắn"
                    : "Còn bước cần hoàn tất"}
              </StatusChip>
              <span>{retryScopeLabel(snapshot.retryScope)}</span>
              <span>
                {!snapshotSheetEnabled(snapshot)
                  ? "Không ghi Sheet"
                  : snapshot.status === "complete"
                    ? "Sheet đã xác nhận"
                    : snapshot.retryScope === "sheetOnly"
                      ? "Sheet chưa hoàn tất"
                      : "Sheet chưa xác nhận hoàn tất"}
              </span>
            </div>
          )}
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
                render: (assignment: PublishAssignmentRecord) =>
                  PUBLISH_STATE_LABELS[assignment.state] ??
                  "Trạng thái chưa nhận diện",
              },
              {
                id: "link",
                label: "Bài đã đăng",
                render: (assignment: PublishAssignmentRecord) => {
                  const evidence = postEvidence(assignment.evidenceJson);
                  return evidence.url ? (
                    <a href={evidence.url} target="_blank" rel="noreferrer">
                      Mở bài đã xác nhận
                    </a>
                  ) : (
                    <span>Chưa có liên kết xác nhận</span>
                  );
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
