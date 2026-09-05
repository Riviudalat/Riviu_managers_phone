import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { CalendarClock, FolderOpen, RefreshCw, Rocket, ShieldCheck } from "lucide-react";

import {
  listenRiviuEvents,
  operationGetRun,
  operationListRuns,
  publishAutoAssign,
  publishCancel,
  publishCreateCampaign,
  publishExecute,
  publishGet,
  publishList,
  publishPreflight,
  publishReadiness,
  publishReconcile,
  publishScanFolder,
  publishSheetGetConfig,
  publishSheetSaveConfig,
} from "../api";
import { publishProfileConfig } from "../automationProfileConfig";
import { AutomationProfileControl, type AutomationProfileHandle } from "../components/AutomationProfileControl";
import { useWorkspaceDraft } from "../workspaceDraft";
import { IconRocket } from "../components/Icons";
import { EmptyState, LoadingState, StatusNotice, type NoticeTone } from "../components/States";
import {
  CommandBar,
  FormSection,
  ResponsiveTable,
  StatusChip,
  SummaryRail,
  WorkflowStepper,
  WorkspaceTabs,
  type StatusTone,
} from "../components/WorkspacePrimitives";
import { requestConfirm } from "../confirmStore";
import { describeError } from "../describeError";
import { orderDevicesByNumber, tileName, tileNumber } from "../deviceNaming";
import { pickDirectory } from "../pickFile";
import { targetsOf } from "../selectionTargets";
import type { OperationSourceRef } from "../operationSource";
import type {
  DevicePublishReadiness,
  PublishAssignmentRecord,
  PublishBundle,
  PublishCampaignDetail,
  PublishCampaignRecord,
  PublishFolderManifest,
  PublishExecutionSnapshot,
  PublishPreflightAssignmentReport,
  PublishPreflightReport,
  PublishPreflightRequest,
  PublishReadinessInfo,
  PublishSheetConfig,
  OperationRunSummary,
  AutomationDefinitionRecord,
  PublishSoundPolicy,
  TargetRef,
} from "../types";
import type { SelProps } from "./pageProps";

const LOCATOR_LABELS: Record<string, string> = {
  ComposerOpen: "nút Tạo",
  ComposerShutter: "mốc màn quay",
  PickerAlbumMenu: "bộ chọn album",
  PickerTabPhotos: "thẻ Ảnh",
  PickerMultiSelect: "nút Chọn nhiều",
  PickerNext: "nút Tiếp ở thư viện",
  ComposerNext: "nút Tiếp ở trình chỉnh sửa",
  ComposerCaption: "ô chú thích",
  PostButton: "nút Đăng",
};

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
): { label: string; tone: StatusTone; retryScope: PublishExecutionSnapshot["retryScope"] } {
  if (["scheduled", "preparing", "transferring", "posting", "verifying", "uncertain", "cancelled", "missed"].includes(campaign.state)) {
    return { label: PUBLISH_STATE_LABELS[campaign.state], tone: campaignTone(campaign.state), retryScope: "none" };
  }
  const newestSnapshot = snapshot && (!operation?.updatedAt || snapshot.updatedAt >= operation.updatedAt)
    ? snapshot : undefined;
  const state = newestSnapshot
    ? newestSnapshot.status === "complete" ? "succeeded" : newestSnapshot.status
    : operation?.state;
  const proposedScope = newestSnapshot?.retryScope ?? operation?.retryScope
    ?? (RETRYABLE_STATES.includes(campaign.state) ? "fullPipeline" : "none");
  const retryScope = campaign.state === "succeeded" && proposedScope === "fullPipeline" ? "none" : proposedScope;
  if (state === "succeeded") return { label: "Hoàn tất", tone: "success", retryScope: "none" };
  if (state === "uncertain") return { label: "Chưa chắc chắn", tone: "warning", retryScope: "none" };
  if (state === "partial") return { label: "Hoàn tất một phần", tone: "warning", retryScope };
  return {
    label: campaign.state === "succeeded" ? "Đã đăng · chờ đối chiếu" : PUBLISH_STATE_LABELS[campaign.state],
    tone: campaignTone(campaign.state),
    retryScope,
  };
}

function retryActionLabel(scope: PublishExecutionSnapshot["retryScope"]): string {
  if (scope === "sheetOnly") return "Ghi lại Sheet";
  if (scope === "linkAndSheet") return "Lấy link và ghi Sheet";
  return "Chạy lại từ đầu";
}

function readinessView(info: PublishReadinessInfo): { label: string; raw?: string } {
  switch (info.kind) {
    case "hierarchyReady":
      return { label: "bản đo có đủ nhãn (chưa đối chiếu build máy)" };
    case "pixelGrid":
      return { label: "đường pixel" };
    case "hierarchyMissing":
      return {
        label: `thiếu ${info.labels
          .map((label) => LOCATOR_LABELS[label] ?? "một điều khiển chưa nhận diện")
          .join(", ")}`,
        raw: info.labels.join(", "),
      };
    case "hierarchyUnknownBuild":
      return { label: `build chưa đo (${info.version || "?"})` };
    default:
      return { label: "trạng thái chưa nhận diện", raw: JSON.stringify(info) };
  }
}

function cleanupEvidence(evidenceJson?: string | null): { label: string; raw: string } | null {
  if (!evidenceJson) return null;
  try {
    const evidence = JSON.parse(evidenceJson) as unknown;
    if (!evidence || typeof evidence !== "object" || !("cleanup" in evidence)) return null;
    const cleanup = (evidence as { cleanup?: unknown }).cleanup;
    if (!cleanup || typeof cleanup !== "object") return null;
    const state = "state" in cleanup ? String((cleanup as { state?: unknown }).state ?? "") : "";
    const message =
      "message" in cleanup ? String((cleanup as { message?: unknown }).message ?? "").trim() : "";
    const raw = JSON.stringify(cleanup);
    if (state === "cleaned") return { label: "ảnh tạm đã dọn", raw };
    if (state === "not_cleaned") {
      return { label: `chưa dọn được ảnh tạm${message ? `: ${message}` : ""}`, raw };
    }
    return { label: "trạng thái dọn ảnh chưa nhận diện", raw };
  } catch {
    return null;
  }
}

function postEvidence(evidenceJson?: string | null): {
  url: string | null;
  sound: { title: string; artist: string; section: string; index: number; digest: string; confirmed: boolean } | null;
} {
  const empty = { url: null, sound: null };
  try {
    const evidence = JSON.parse(evidenceJson ?? "null") as unknown;
    if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) return empty;
    const root = evidence as Record<string, unknown>;
    const post = root.post && typeof root.post === "object" && !Array.isArray(root.post)
      ? root.post as Record<string, unknown> : root;
    let url: string | null = null;
    if (typeof post.postUrl === "string") {
      const parsed = new URL(post.postUrl);
      if (parsed.protocol === "https:" && ["www.tiktok.com", "tiktok.com"].includes(parsed.hostname)
        && /^\/@[^/]+\/(?:video|photo)\/\d+\/?$/.test(parsed.pathname)) url = parsed.href;
    }
    const rawSound = post.soundSelection ?? root.soundSelection;
    const sound = rawSound && typeof rawSound === "object" && !Array.isArray(rawSound)
      ? rawSound as Record<string, unknown> : null;
    return {
      url,
      sound: sound && typeof sound.title === "string" && typeof sound.artist === "string"
        && typeof sound.section === "string" && typeof sound.index === "number"
        && typeof sound.candidatesDigest === "string" ? {
          title: sound.title, artist: sound.artist, section: sound.section, index: sound.index,
          digest: sound.candidatesDigest, confirmed: sound.confirmed === true,
        } : null,
    };
  } catch { return empty; }
}

function stableSoundSeed(value: string): number {
  let seed = 0x811c9dc5;
  for (const char of value) {
    seed ^= char.charCodeAt(0);
    seed = Math.imul(seed, 0x01000193);
  }
  return seed >>> 0;
}

function mediaSummary(bundle: PublishBundle): string {
  if (bundle.mediaKind === "video" && bundle.video) {
    const seconds = Math.round(bundle.video.durationMs / 1000);
    return `Video · ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")} · ${(
      bundle.video.byteLen / 1024 / 1024
    ).toFixed(1)} MB`;
  }
  return `${bundle.images.length} ảnh`;
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

function reconcilePublishTargets(
  current: readonly string[],
  eligible: readonly string[],
  wanted: number,
): string[] {
  const eligibleSet = new Set(eligible);
  const next = current.filter(
    (udid, index) => eligibleSet.has(udid) && current.indexOf(udid) === index,
  ).slice(0, wanted);
  for (const udid of eligible) {
    if (next.length >= wanted) break;
    if (!next.includes(udid)) next.push(udid);
  }
  return next;
}

function sameOrderedTargets(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((udid, index) => udid === right[index]);
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
  const [workspaceTab, setWorkspaceTab] = useState<"setup" | "monitor">("setup");
  const [sourceRoot, setSourceRoot] = useState("");
  const [manifest, setManifest] = useState<PublishFolderManifest | null>(null);
  const [bundleIds, setBundleIds] = useState<string[]>([]);
  const [assignedUdids, setAssignedUdids] = useState<string[]>([]);
  const [captionDrafts, setCaptionDrafts] = useState<Record<string, string>>({});
  const [runAt, setRunAt] = useState("");
  const [soundPolicyOverride, setSoundPolicyOverride] = useState<PublishSoundPolicy | null>(null);
  const profileRef = useRef<AutomationProfileHandle>(null);
  const [campaigns, setCampaigns] = useState<PublishCampaignRecord[]>([]);
  const [campaignLoadState, setCampaignLoadState] = useState<"loading" | "ready" | "error">("loading");
  const [campaignLoadError, setCampaignLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ tone: NoticeTone; text: string } | null>(null);
  const [readiness, setReadiness] = useState<DevicePublishReadiness[]>([]);
  const [readinessNote, setReadinessNote] = useState<string | null>(null);
  const [readinessNonce, setReadinessNonce] = useState(0);
  const [sheetConfig, setSheetConfig] = useState<PublishSheetConfig | null>(null);
  const [sheetLoadState, setSheetLoadState] = useState<"loading" | "ready" | "error">("loading");
  const [sheetLoadError, setSheetLoadError] = useState<string | null>(null);
  const [sheetUrlDraft, setSheetUrlDraft] = useState("");
  const [sheetTokenDraft, setSheetTokenDraft] = useState("");
  const [sheetBusy, setSheetBusy] = useState(false);
  const [details, setDetails] = useState<Record<string, PublishCampaignDetail>>({});
  const [detailErrors, setDetailErrors] = useState<Record<string, string>>({});
  const [detailLoading, setDetailLoading] = useState<Record<string, boolean>>({});
  const [executionSnapshots, setExecutionSnapshots] = useState<Record<string, PublishExecutionSnapshot>>({});
  const [operations, setOperations] = useState<Record<string, OperationRunSummary>>({});
  const [operationError, setOperationError] = useState<string | null>(null);
  const [preflightState, setPreflightState] = useState<AsyncState>("idle");
  const [preflightError, setPreflightError] = useState<string | null>(null);
  const [preflightSnapshot, setPreflightSnapshot] = useState<{
    inputKey: string;
    report: PublishPreflightReport;
  } | null>(null);
  const [sourceCampaign, setSourceCampaign] = useState<PublishCampaignRecord | null>(null);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [sourceLoading, setSourceLoading] = useState(false);
  const sourceId = operationSource?.kind === "publish" ? operationSource.sourceId : undefined;
  useEffect(() => {
    if (!sourceId) { setSourceCampaign(null); setSourceError(null); return; }
    let active = true;
    setWorkspaceTab("monitor");
    setSourceCampaign(null);
    setSourceError(null);
    setSourceLoading(true);
    void publishGet(sourceId).then((detail) => {
      if (!active) return;
      if (!detail || detail.campaign.id !== sourceId) throw new Error("Chiến dịch được chọn không còn trong nguồn dữ liệu.");
      setSourceCampaign(detail.campaign);
      setDetails((current) => ({ ...current, [sourceId]: detail }));
    }).catch((error) => { if (active) setSourceError(describeError(error)); })
      .finally(() => { if (active) setSourceLoading(false); });
    return () => { active = false; };
  }, [sourceId]);

  const eligibleTargets = useMemo(
    () => targetUdids ?? targetsOf(selected, devices),
    [targetUdids, selected, devices],
  );
  const selectedBundles = manifest?.bundles.filter((bundle) => bundleIds.includes(bundle.id)) ?? [];
  const targets = assignedUdids;
  const effectiveTargetRef: TargetRef = sameOrderedTargets(targets, eligibleTargets)
    ? targetRef
    : { type: "explicit", udids: targets };
  const orderedBundleIds = selectedBundles.map((bundle) => bundle.id);
  const currentCaptionOverrides = Object.fromEntries(
    selectedBundles.map((bundle) => [bundle.id, (captionDrafts[bundle.id] ?? bundle.caption).trim()]),
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
    sourceRoot: sourceRoot.trim(),
    bundleIds: orderedBundleIds,
    udids: targets,
    targetRef: effectiveTargetRef,
    runAt: runAt || null,
    captionOverrides: currentCaptionOverrides,
    soundPolicy: currentSoundPolicy,
  };
  const inputKey = JSON.stringify(preflightRequest);
  const draftSnapshot = useMemo(() => ({ sourceRoot, bundleIds, assignedUdids, captionDrafts, runAt, targetRef, soundPolicyOverride }),
    [sourceRoot, bundleIds, assignedUdids, captionDrafts, runAt, targetRef, soundPolicyOverride]);
  const draftKey = JSON.stringify(draftSnapshot);
  const latestDraftKey = useRef(draftKey);
  latestDraftKey.current = draftKey;
  const [baseline, setBaseline] = useState(draftSnapshot);
  const [baselineManifest, setBaselineManifest] = useState(manifest);
  const applyingProfile = useRef<TargetRef | null>(null);
  const dirty = draftKey !== JSON.stringify(baseline);
  useWorkspaceDraft({
    id: "publish", label: "Đăng bài", dirty, snapshotKey: draftKey,
    save: async () => {
      if (runAt) {
        setNotice({ tone: "warning", text: "Lịch hẹn chưa được tạo. Xác nhận lịch hoặc xóa thời gian hẹn trước khi lưu hồ sơ." });
        return false;
      }
      return await profileRef.current?.save() ?? false;
    },
    discard: () => {
      setSourceRoot(baseline.sourceRoot);
      setBundleIds(baseline.bundleIds);
      setAssignedUdids(baseline.assignedUdids);
      setCaptionDrafts(baseline.captionDrafts);
      setRunAt(baseline.runAt);
      setSoundPolicyOverride(baseline.soundPolicyOverride);
      setManifest(baselineManifest);
      onTargetRefChange?.(baseline.targetRef);
      setPreflightSnapshot(null);
    },
  });
  const mappingReady = selectedBundles.length > 0
    && selectedBundles.length === targets.length
    && new Set(targets).size === targets.length
    && targets.every((udid) => eligibleTargets.includes(udid));
  const captionsReady = Object.values(currentCaptionOverrides).every((caption) => caption.length > 0);
  const profileReady = mappingReady && captionsReady;
  const currentPreflight = preflightSnapshot?.inputKey === inputKey ? preflightSnapshot.report : null;
  const canExecute = currentPreflight?.canExecute === true;
  const readinessTargets = targets.length ? targets : eligibleTargets;
  const androidTargets = readinessTargets.filter(
    (udid) => devices.find((device) => device.udid === udid)?.platform === "android",
  );

  const invalidatePreflight = () => {
    setSoundPolicyOverride(null);
    setPreflightSnapshot(null);
    setPreflightState("idle");
    setPreflightError(null);
  };

  useEffect(() => {
    setAssignedUdids((current) => {
      const next = reconcilePublishTargets(current, eligibleTargets, selectedBundles.length);
      return sameOrderedTargets(current, next) ? current : next;
    });
  }, [eligibleTargets, selectedBundles.length]);

  useEffect(() => {
    if (applyingProfile.current && JSON.stringify(applyingProfile.current) === JSON.stringify(targetRef)
      && mappingReady) {
      applyingProfile.current = null;
      setBaseline(draftSnapshot);
      setBaselineManifest(manifest);
    }
  }, [draftSnapshot, mappingReady, manifest, targetRef]);

  const applyProfile = async (record: AutomationDefinitionRecord) => {
    const applyingKey = latestDraftKey.current;
    const config = record.revision.config;
    if (!config || typeof config !== "object" || Array.isArray(config) || config.schemaVersion !== 1
      || typeof config.sourceRoot !== "string" || !Array.isArray(config.bundleIds)
      || !config.bundleIds.every((id): id is string => typeof id === "string")) {
      throw new Error("Hồ sơ Đăng bài không đúng định dạng.");
    }
    const next = await publishScanFolder(config.sourceRoot);
    if (latestDraftKey.current !== applyingKey) throw new Error("Thiết lập vừa thay đổi. Chọn lại hồ sơ để áp dụng.");
    if (!config.bundleIds.every((id) => next.bundles.some((bundle) => bundle.id === id))) {
      throw new Error("Nội dung hồ sơ đã thay đổi hoặc bị thiếu. Chọn lại thư mục trước khi đăng.");
    }
    const captions = config.captionOverrides;
    if (!captions || typeof captions !== "object" || Array.isArray(captions)
      || !Object.values(captions).every((caption) => typeof caption === "string")) {
      throw new Error("Hồ sơ Đăng bài thiếu chú thích hợp lệ.");
    }
    const policy = config.soundPolicy;
    if (!policy || typeof policy !== "object" || Array.isArray(policy)
      || (policy.kind !== "default" && !(policy.kind === "trendingAny"
        && typeof policy.poolSize === "number" && typeof policy.seed === "number"))) {
      throw new Error("Hồ sơ Đăng bài thiếu lựa chọn nhạc hợp lệ.");
    }
    setSourceRoot(config.sourceRoot);
    setManifest(next);
    setBundleIds(config.bundleIds);
    setCaptionDrafts(captions as Record<string, string>);
    setRunAt("");
    setSoundPolicyOverride(policy as unknown as PublishSoundPolicy);
    setAssignedUdids(record.revision.targetRef.type === "explicit" ? record.revision.targetRef.udids : []);
    applyingProfile.current = record.revision.targetRef;
    onTargetRefChange?.(record.revision.targetRef);
    setPreflightSnapshot(null);
    setPreflightState("idle");
    setPreflightError(null);
    setWorkspaceTab("setup");
  };

  const reloadTicket = useRef(0);
  const reload = () => {
    const ticket = ++reloadTicket.current;
    setCampaignLoadState((current) => (current === "ready" ? current : "loading"));
    setCampaignLoadError(null);
    return Promise.all([
      publishList(),
      operationListRuns(200).then((runs) => ({ runs, error: null as string | null }))
        .catch((error) => ({ runs: [], error: describeError(error) })),
    ])
      .then(([next, projection]) => {
        if (ticket !== reloadTicket.current) return;
        setCampaigns(next);
        setOperations(Object.fromEntries(projection.runs.filter((run) => run.kind === "publish").map((run) => [run.sourceId, run])));
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

  const androidKey = androidTargets
    .slice()
    .sort()
    .join(",");
  useEffect(() => {
    if (!androidKey) {
      setReadiness([]);
      setReadinessNote(null);
      return;
    }
    let live = true;
    publishReadiness(androidKey.split(","))
      .then((rows) => {
        if (!live) return;
        setReadiness(rows);
        setReadinessNote(null);
      })
      .catch((error) => {
        if (!live) return;
        setReadiness([]);
        setReadinessNote(describeError(error));
      });
    return () => {
      live = false;
    };
  }, [androidKey, readinessNonce]);

  const scan = async (path: string) => {
    setBusy(true);
    setNotice(null);
    invalidatePreflight();
    try {
      const next = await publishScanFolder(path);
      setSourceRoot(path);
      setManifest(next);
      setBundleIds(next.bundles.slice(0, eligibleTargets.length).map((bundle) => bundle.id));
      setCaptionDrafts(Object.fromEntries(next.bundles.map((bundle) => [bundle.id, bundle.caption])));
    } catch (error) {
      setManifest(null);
      setBundleIds([]);
      setCaptionDrafts({});
      setNotice({ tone: "error", text: describeError(error) });
    } finally {
      setBusy(false);
    }
  };

  const runPreflight = async () => {
    if (!profileReady) {
      setPreflightState("error");
      setPreflightError("Chọn đủ nội dung, máy đích và chú thích trước khi kiểm tra.");
      return;
    }
    setPreflightState("loading");
    setPreflightError(null);
    try {
      const request = preflightRequest;
      const requestKey = JSON.stringify(request);
      const report = await publishPreflight(request);
      setPreflightSnapshot({ inputKey: requestKey, report });
      setPreflightState("ready");
    } catch (error) {
      setPreflightSnapshot(null);
      setPreflightError(describeError(error));
      setPreflightState("error");
    }
  };

  const executeNewCampaign = async () => {
    if (!currentPreflight?.canExecute) return;
    const confirmed = await requestConfirm({
      title: runAt ? "Xác nhận lập lịch đăng bài?" : "Xác nhận đăng công khai?",
      message: runAt
        ? `${selectedBundles.length} bài sẽ chạy trên ${targets.length} máy vào lịch đã chọn.`
        : `${selectedBundles.length} bài sẽ được đăng công khai trên ${targets.length} máy với âm thanh đã kiểm tra.`,
      confirmLabel: runAt ? "Lập lịch" : "Đăng bài",
      cancelLabel: "Huỷ",
      danger: true,
    });
    if (!confirmed) return;
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
      );
      setWorkspaceTab("monitor");
      if (!runAt) {
        const result = await publishExecute(campaign.id, true);
        setDetails((current) => ({ ...current, [campaign.id]: result.detail }));
        setNotice({
          tone: result.status === "complete" ? "success" : result.status === "uncertain" ? "warning" : "info",
          text:
            result.status === "complete"
              ? "Đã đăng, lấy liên kết và ghi Sheet."
              : result.status === "uncertain"
                ? "Có máy chưa xác định được kết quả sau thao tác Đăng. Quy trình đã dừng."
                : "Bài đã xử lý nhưng còn bước cần hoàn tất. Mở chi tiết để xem phạm vi retry.",
        });
      } else {
        setNotice({ tone: "success", text: `Đã lập lịch ${selectedBundles.length} bài cho ${targets.length} máy.` });
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
      setExecutionSnapshots((current) => ({ ...current, [campaign.id]: snapshot }));
      if (snapshot.retryScope === "none") {
        setNotice({
          tone: "warning",
          text: "Trạng thái đã được đối chiếu và không có bước nào được phép tự chạy lại.",
        });
        return;
      }
    } catch (error) {
      setNotice({ tone: "error", text: `Không đối chiếu được chiến dịch: ${describeError(error)}` });
      return;
    } finally {
      setBusy(false);
    }
    const confirmed = await requestConfirm({
      title: "Xác nhận tiếp tục đăng bài?",
      message: `${retryScopeLabel(snapshot.retryScope)}. Trạng thái chưa chắc chắn không được tự đăng lại.`,
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
        tone: result.status === "complete" ? "success" : result.status === "uncertain" ? "warning" : "info",
        text:
          result.status === "complete"
            ? "Đã hoàn tất đăng bài và ghi Sheet."
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
        if (operation) setOperations((current) => ({ ...current, [campaign.id]: operation.summary }));
        setExecutionSnapshots((current) => ({ ...current, [campaign.id]: snapshot }));
        setDetails((current) => ({ ...current, [campaign.id]: detail }));
      }
      else setDetailErrors((current) => ({ ...current, [campaign.id]: "Chiến dịch không còn trong dữ liệu." }));
    } catch (error) {
      setDetailErrors((current) => ({ ...current, [campaign.id]: describeError(error) }));
    } finally {
      setDetailLoading((current) => ({ ...current, [campaign.id]: false }));
    }
  };

  const setupStep = !manifest ? "source" : !profileReady ? "mapping" : !canExecute ? "preflight" : "confirm";
  const stepper = (
    <div className="publish-stepper-scroll">
      <WorkflowStepper
        current={setupStep}
        label="Quy trình đăng bài"
        onStepChange={(id) => {
          const section = document.getElementById(`publish-step-${id}`);
          section?.focus();
          section?.scrollIntoView?.({ block: "nearest", behavior: "auto" });
        }}
        steps={[
          { id: "source", label: "Nguồn" },
          { id: "mapping", label: "Ghép bài/máy" },
          { id: "preflight", label: "Preflight" },
          { id: "confirm", label: "Xác nhận công khai" },
        ]}
      />
    </div>
  );

  return (
    <main className="panel publish-page">
      <div className="publish-page-tabs">
        <WorkspaceTabs
          label="Không gian Đăng bài"
          value={workspaceTab}
          onChange={(value) => setWorkspaceTab(value as "setup" | "monitor")}
          tabs={[
            { id: "setup", label: "Thiết lập", panelId: "publish-panel-setup" },
            { id: "monitor", label: "Theo dõi", panelId: "publish-panel-monitor" },
          ]}
        />
      </div>

      {notice && (
        <div className="publish-global-notice">
          <StatusNotice tone={notice.tone}>{notice.text}</StatusNotice>
        </div>
      )}

      <section id="publish-panel-setup" className="publish-workspace-section" role="tabpanel" aria-label="Thiết lập" hidden={workspaceTab !== "setup"}>
        {stepper}
        <CommandBar
          title={canExecute ? `${selectedBundles.length} bài đã sẵn sàng` : "Chưa thể đăng"}
          detail={canExecute ? `Đầu vào đã khóa cho ${targets.length} máy.` : "Hoàn tất nguồn, ghép máy và preflight trước khi xác nhận."}
          tone={canExecute ? "success" : "warning"}
          actions={(
            <button type="button" className="primary publish-submit" disabled={!canExecute || busy} onClick={() => void executeNewCampaign()}>
              <Rocket size={17} aria-hidden="true" />
              {runAt ? "Xác nhận và lập lịch" : "Xác nhận và đăng"} ({selectedBundles.length} → {targets.length})
            </button>
          )}
        />
        <div className="publish-workspace-grid">
          <div className="publish-workspace-main">
            <div id="publish-step-source" className="publish-step-section" tabIndex={-1} role="group" aria-label="Nguồn nội dung">
            <SourceSection
              busy={busy}
              sourceRoot={sourceRoot}
              manifest={manifest}
              bundleIds={bundleIds}
              captionDrafts={captionDrafts}
              maxSelected={eligibleTargets.length}
              setSourceRoot={setSourceRoot}
              setManifest={setManifest}
              setBundleIds={setBundleIds}
              setCaptionDrafts={setCaptionDrafts}
              invalidate={invalidatePreflight}
              scan={scan}
            />
            </div>
            <div id="publish-step-mapping" className="publish-step-section" tabIndex={-1} role="group" aria-label="Ghép bài với máy">
            <MappingSection
              busy={busy}
              manifest={manifest}
              sourceRoot={sourceRoot}
              eligibleTargets={eligibleTargets}
              targets={targets}
              devices={devices}
              metas={metas}
              selectedBundles={selectedBundles}
              mappingReady={mappingReady}
              setBusy={setBusy}
              setNotice={setNotice}
              setBundleIds={setBundleIds}
              setAssignedUdids={setAssignedUdids}
              invalidate={invalidatePreflight}
            />
            </div>
            <div id="publish-step-preflight" className="publish-step-section" tabIndex={-1} role="group" aria-label="Kiểm tra trước khi đăng">
            <PreflightSection
              profileReady={profileReady}
              busy={busy}
              preflightState={preflightState}
              preflightError={preflightError}
              snapshot={preflightSnapshot}
              current={currentPreflight}
              run={runPreflight}
              androidTargets={androidTargets}
              readiness={readiness}
              readinessNote={readinessNote}
              refreshReadiness={() => setReadinessNonce((value) => value + 1)}
              devices={devices}
              metas={metas}
            />
            </div>
            <div id="publish-step-confirm" className="publish-step-section" tabIndex={-1} role="group" aria-label="Xác nhận công khai">
            <FormSection title="Xác nhận công khai" description="Nút chạy chỉ mở cho đúng digest vừa vượt qua preflight.">
              <div className="publish-confirm-grid">
                <label>
                  <span>Lịch chạy một lần</span>
                  <span className="publish-input-with-icon">
                    <CalendarClock size={16} aria-hidden="true" />
                    <input
                      type="datetime-local"
                      value={runAt}
                      onChange={(event) => {
                        setRunAt(event.target.value);
                        invalidatePreflight();
                      }}
                    />
                  </span>
                </label>
              </div>
              {!canExecute && <p className="publish-muted">Hoàn tất preflight của đầu vào hiện tại để mở nút xác nhận.</p>}
            </FormSection>
            </div>
          </div>
          <PublishAside
            manifest={manifest}
            selectedCount={selectedBundles.length}
            targetCount={targets.length}
            currentPreflight={currentPreflight}
            preflightState={preflightState}
            sheetConfig={sheetConfig}
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
            profileSaved={() => { setBaseline(draftSnapshot); setBaselineManifest(manifest); }}
          />
        </div>
      </section>

      <section id="publish-panel-monitor" className="publish-workspace-section" role="tabpanel" aria-label="Theo dõi" hidden={workspaceTab !== "monitor"}>
        {sourceLoading && <LoadingState label="Đang mở chiến dịch được chọn…" />}
        {sourceError && <StatusNotice tone="error">{sourceError}</StatusNotice>}
        <div className="publish-monitor-head">
          <div>
            <h2>Tiến độ chiến dịch</h2>
          </div>
          <button type="button" className="ghost" onClick={() => void reload()} disabled={busy}>
            <RefreshCw size={16} aria-hidden="true" /> Làm mới
          </button>
        </div>
        {campaignLoadState === "loading" && <LoadingState label="Đang tải chiến dịch…" />}
        {campaignLoadState === "error" && (
          <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={() => void reload()}>Thử lại</button>}>
            {campaignLoadError ?? "Không tải được chiến dịch."}
          </StatusNotice>
        )}
        {operationError && (
          <StatusNotice tone="warning" action={<button type="button" className="ghost" onClick={() => void reload()}>Thử đối chiếu lại</button>}>
            Chưa đối chiếu được kết quả link và Sheet. {operationError}
          </StatusNotice>
        )}
        {campaignLoadState === "ready" && campaigns.length === 0 && (
          <EmptyState compact icon={<IconRocket size={17} />} title="Chưa có chiến dịch" hint="Tạo chiến dịch ở thẻ Thiết lập để bắt đầu đăng bài." />
        )}
        {(sourceId ? sourceCampaign !== null : campaignLoadState === "ready" && campaigns.length > 0) && (
          <CampaignMonitor
            campaigns={sourceCampaign ? [campaigns.find((campaign) => campaign.id === sourceCampaign.id) ?? sourceCampaign] : campaigns}
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

function SourceSection({
  busy,
  sourceRoot,
  manifest,
  bundleIds,
  captionDrafts,
  maxSelected,
  setSourceRoot,
  setManifest,
  setBundleIds,
  setCaptionDrafts,
  invalidate,
  scan,
}: {
  busy: boolean;
  sourceRoot: string;
  manifest: PublishFolderManifest | null;
  bundleIds: string[];
  captionDrafts: Record<string, string>;
  maxSelected: number;
  setSourceRoot: (value: string) => void;
  setManifest: (value: PublishFolderManifest | null) => void;
  setBundleIds: Dispatch<SetStateAction<string[]>>;
  setCaptionDrafts: Dispatch<SetStateAction<Record<string, string>>>;
  invalidate: () => void;
  scan: (path: string) => Promise<void>;
}) {
  return (
    <FormSection title="Nguồn nội dung" description="Mỗi thư mục con chứa một video hoặc một bộ ảnh cùng chú thích.">
      <div className="publish-source-row">
        <label className="publish-path-field">
          <span>Thư mục nguồn</span>
          <input
            value={sourceRoot}
            onChange={(event) => {
              const next = event.target.value;
              setSourceRoot(next);
              if (next !== manifest?.sourceRoot) {
                setManifest(null);
                setBundleIds([]);
                setCaptionDrafts({});
              }
              invalidate();
            }}
            placeholder="Chọn thư mục nội dung"
          />
        </label>
        <button
          type="button"
          className="ghost"
          disabled={busy}
          onClick={async () => {
            const path = await pickDirectory();
            if (path) await scan(path);
          }}
        >
          <FolderOpen size={16} aria-hidden="true" /> Chọn thư mục
        </button>
        <button type="button" className="primary" disabled={!sourceRoot.trim() || busy} onClick={() => void scan(sourceRoot.trim())}>
          Quét nguồn
        </button>
      </div>
      {!manifest && !busy && (
        <EmptyState compact icon={<FolderOpen size={17} />} title="Chưa có nội dung" hint="Chọn thư mục để đọc các gói bài thật." />
      )}
      {busy && !manifest && <LoadingState label="Đang quét nội dung…" />}
      {manifest && (
        <div className="publish-bundle-list" aria-label="Gói nội dung">
          <div className="publish-section-summary">
            <strong>{manifest.bundles.length} gói hợp lệ</strong>
            <details>
              <summary>Chi tiết quét</summary>
              <span>{manifest.ignoredPartnerFiles} tệp đối tác và {manifest.ignoredHiddenFiles} tệp ẩn được bỏ qua</span>
            </details>
          </div>
          {manifest.bundles.map((bundle) => {
            const checked = bundleIds.includes(bundle.id);
            return (
              <article key={bundle.id} className={`publish-bundle-row ${checked ? "is-selected" : ""}`}>
                <label className="publish-bundle-choice">
                  <input
                    type="checkbox"
                    aria-label={`Chọn ${bundle.name}`}
                    checked={checked}
                    disabled={!checked && bundleIds.length >= maxSelected}
                    onChange={(event) => {
                      setBundleIds((current) =>
                        event.target.checked ? [...current, bundle.id] : current.filter((id) => id !== bundle.id),
                      );
                      invalidate();
                    }}
                  />
                  <span>
                    <strong>{bundle.name}</strong>
                    <small>{mediaSummary(bundle)}</small>
                  </span>
                </label>
                <label className="publish-caption-field">
                  <span>Chú thích</span>
                  <textarea
                    aria-label={`Chú thích cho ${bundle.name}`}
                    value={captionDrafts[bundle.id] ?? bundle.caption}
                    onChange={(event) => {
                      const caption = event.target.value;
                      setCaptionDrafts((current) => ({ ...current, [bundle.id]: caption }));
                      invalidate();
                    }}
                    rows={2}
                  />
                </label>
              </article>
            );
          })}
          {manifest.notices.length > 0 && (
            <StatusNotice tone={manifest.notices.some((row) => row.severity === "error") ? "error" : "warning"}>
              {manifest.notices.map((row) => row.message).join(" ")}
            </StatusNotice>
          )}
        </div>
      )}
    </FormSection>
  );
}

function MappingSection({
  busy,
  manifest,
  sourceRoot,
  eligibleTargets,
  targets,
  devices,
  metas,
  selectedBundles,
  mappingReady,
  setBusy,
  setNotice,
  setBundleIds,
  setAssignedUdids,
  invalidate,
}: {
  busy: boolean;
  manifest: PublishFolderManifest | null;
  sourceRoot: string;
  eligibleTargets: string[];
  targets: string[];
  devices: SelProps["devices"];
  metas: Map<string, import("../types").DeviceMeta>;
  selectedBundles: PublishBundle[];
  mappingReady: boolean;
  setBusy: (value: boolean) => void;
  setNotice: NoticeSetter;
  setBundleIds: (value: string[]) => void;
  setAssignedUdids: Dispatch<SetStateAction<string[]>>;
  invalidate: () => void;
}) {
  return (
    <FormSection
      title="Ghép bài với máy"
      actions={
        <button
          type="button"
          className="ghost"
          disabled={busy || !manifest || eligibleTargets.length === 0}
          onClick={async () => {
            if (!manifest) return;
            setBusy(true);
            setNotice(null);
            invalidate();
            try {
              const wanted = selectedBundles.length || Math.min(manifest.bundles.length, eligibleTargets.length);
              const result = await publishAutoAssign(sourceRoot.trim(), eligibleTargets, wanted);
              setBundleIds(result.plan.map((row) => row.bundleId));
              setAssignedUdids(result.plan.map((row) => row.udid));
              setNotice({ tone: "success", text: `Đã ghép ${result.plan.length} bài với ${result.plan.length} máy.` });
            } catch (error) {
              setNotice({ tone: "error", text: describeError(error) });
            } finally {
              setBusy(false);
            }
          }}
        >
          Chia tự động
        </button>
      }
    >
      {selectedBundles.length ? (
        <ResponsiveTable
          label="Ghép bài với máy"
          rows={selectedBundles}
          keyForRow={(bundle) => bundle.id}
          columns={[
            { id: "bundle", label: "Bài", render: (bundle) => `${selectedBundles.indexOf(bundle) + 1}. ${bundle.name}` },
            { id: "media", label: "Nội dung", render: mediaSummary },
            {
              id: "device",
              label: "Máy đích",
              render: (bundle) => {
                const index = selectedBundles.indexOf(bundle);
                const currentUdid = targets[index] ?? "";
                const usedElsewhere = new Set(targets.filter((_, targetIndex) => targetIndex !== index));
                return (
                  <select
                    aria-label={`Máy đăng ${bundle.name}`}
                    value={currentUdid}
                    onChange={(event) => {
                      const udid = event.target.value;
                      setAssignedUdids((current) => {
                        const next = reconcilePublishTargets(current, eligibleTargets, selectedBundles.length);
                        next[index] = udid;
                        return next;
                      });
                      invalidate();
                    }}
                  >
                    <option value="" disabled>Chọn máy</option>
                    {eligibleTargets.map((udid) => (
                      <option key={udid} value={udid} disabled={usedElsewhere.has(udid)}>
                        {deviceDisplayName(devices, metas, udid)}
                      </option>
                    ))}
                  </select>
                );
              },
            },
          ]}
        />
      ) : (
        <EmptyState compact title="Chưa có cặp bài-máy" hint="Chọn nội dung và máy đích để tạo ánh xạ." />
      )}
      {!mappingReady && selectedBundles.length > 0 && (
        <StatusNotice tone="warning">
          Cần chọn {selectedBundles.length} máy khác nhau cho {selectedBundles.length} bài. Phạm vi hiện có {eligibleTargets.length} máy.
        </StatusNotice>
      )}
    </FormSection>
  );
}

function PreflightSection({
  profileReady,
  busy,
  preflightState,
  preflightError,
  snapshot,
  current,
  run,
  androidTargets,
  readiness,
  readinessNote,
  refreshReadiness,
  devices,
  metas,
}: {
  profileReady: boolean;
  busy: boolean;
  preflightState: AsyncState;
  preflightError: string | null;
  snapshot: { inputKey: string; report: PublishPreflightReport } | null;
  current: PublishPreflightReport | null;
  run: () => Promise<void>;
  androidTargets: string[];
  readiness: DevicePublishReadiness[];
  readinessNote: string | null;
  refreshReadiness: () => void;
  devices: SelProps["devices"];
  metas: Map<string, import("../types").DeviceMeta>;
}) {
  return (
    <FormSection
      title="Preflight"
      description="Kiểm tra đúng digest nguồn, máy, media, composer và bộ chọn nhạc trước khi mở quyền đăng."
      actions={
        <button type="button" className="primary" disabled={!profileReady || preflightState === "loading" || busy} onClick={() => void run()}>
          <ShieldCheck size={16} aria-hidden="true" /> {current ? "Kiểm tra lại" : "Chạy preflight"}
        </button>
      }
    >
      {androidTargets.length > 0 && (
        <div className="publish-readiness-block">
          <div className="publish-inline-heading">
            <strong>Khả năng tương thích Android</strong>
            <button type="button" className="ghost" onClick={refreshReadiness}>Hỏi lại</button>
          </div>
          {readinessNote && <StatusNotice tone="error">Không đọc được trạng thái sẵn sàng: {readinessNote}</StatusNotice>}
          <div className="publish-readiness-list">
            {readiness.map(({ udid, readiness: info }) => {
              const view = readinessView(info);
              return (
                <span key={udid} className="pill">
                  {deviceDisplayName(devices, metas, udid)}: {view.label}
                </span>
              );
            })}
          </div>
          {readiness.length > 0 && (
            <details className="publish-technical-details">
              <summary>Chi tiết khả năng tương thích</summary>
              <ul>
                {readiness.map(({ udid, readiness: info }) => (
                  <li key={`readiness-detail-${udid}`}>
                    <strong>{deviceDisplayName(devices, metas, udid)}</strong>
                    <code>{udid}</code>
                    <code>{readinessView(info).raw}</code>
                  </li>
                ))}
              </ul>
            </details>
          )}
          <p className="publish-muted">Máy có build chưa được đo sẽ bị từ chối trước khi chuyển nội dung.</p>
        </div>
      )}
      {preflightState === "idle" && !current && !snapshot && (
        <EmptyState compact title="Chưa kiểm tra" hint="Preflight phải đạt trên chính đầu vào hiện tại." />
      )}
      {preflightState === "loading" && <LoadingState label="Đang kiểm tra từng máy…" />}
      {preflightState === "error" && (
        <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={() => void run()}>Thử lại</button>}>
          {preflightError ?? "Không chạy được preflight."}
        </StatusNotice>
      )}
      {snapshot && !current && <StatusNotice tone="warning">Đầu vào đã thay đổi. Kết quả kiểm tra trước không còn hiệu lực.</StatusNotice>}
      {current && (
        <div className="publish-preflight-report">
          <StatusNotice tone={current.canExecute ? "success" : "error"}>
            {current.canExecute
              ? `Đạt trên ${current.targetSnapshot.included.length} máy. Có thể chuyển sang xác nhận công khai.`
              : "Preflight chưa đạt. Không thể tạo chiến dịch từ đầu vào này."}
          </StatusNotice>
          <div className="publish-target-snapshot">
            <span>
              Phạm vi đã khóa: {current.targetSnapshot.included.length} máy
              {current.targetSnapshot.excluded.length > 0
                ? ` · ${current.targetSnapshot.excluded.length} máy bị loại`
                : ""}
            </span>
            <details
              className="publish-technical-details"
              aria-label="Chi tiết kỹ thuật phạm vi"
            >
              <summary>Chi tiết phạm vi</summary>
              <code>Roster SHA-256: {current.targetSnapshot.rosterSha256}</code>
              <ul>
                {current.targetSnapshot.included.map((device) => (
                  <li key={`included-${device.udid}`}>
                    {device.number != null ? `Máy ${device.number}` : device.alias || "Máy chưa đặt số"}
                    {device.alias && device.number != null ? ` · ${device.alias}` : ""}
                    <code>{device.udid}</code>
                  </li>
                ))}
                {current.targetSnapshot.excluded.map(({ device, reason }, index) => (
                  <li key={`excluded-${device.udid}-${index}`}>
                    {device.number != null ? `Máy ${device.number}` : device.alias || "Máy chưa đặt số"}
                    {`: ${reason === "not_in_roster" ? "không còn kết nối" : "bị lặp trong phạm vi"}`}
                    <code>{device.udid} · {reason}</code>
                  </li>
                ))}
              </ul>
            </details>
          </div>
          <PreflightTable rows={current.assignments} devices={devices} metas={metas} />
          {current.issues.length > 0 && (
            <ul className="publish-issue-list">
              {current.issues.map((issue, index) => (
                <li key={`${issue.code}-${index}`}>
                  {issue.message}
                  <details className="publish-technical-details" aria-label="Chi tiết lỗi preflight">
                    <summary>Mã lỗi</summary>
                    <code>{issue.code}</code>
                  </details>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </FormSection>
  );
}

function PublishAside({
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
      const saved = await publishSheetSaveConfig(sheetUrlDraft, sheetTokenDraft === "" ? undefined : sheetTokenDraft);
      setSheetConfig(saved);
      setSheetUrlDraft(saved.webhookUrl);
      setSheetTokenDraft("");
      setNotice({ tone: "success", text: "Đã lưu cấu hình Sheet." });
      return true;
    } catch (error) {
      setNotice({ tone: "error", text: describeError(error) });
      return false;
    } finally { setSheetBusy(false); }
  };
  useWorkspaceDraft({
    id: "publish-sheet", label: "Cấu hình Sheet",
    dirty: sheetLoadState === "ready" && (sheetUrlDraft !== sheetConfig?.webhookUrl || sheetTokenDraft !== ""),
    snapshotKey: JSON.stringify([sheetUrlDraft, sheetTokenDraft]),
    save: saveSheet,
    discard: () => { setSheetUrlDraft(sheetConfig?.webhookUrl ?? ""); setSheetTokenDraft(""); },
  });
  return (
    <div className="publish-workspace-aside">
      <SummaryRail title="Tóm tắt lượt chạy">
        <dl className="publish-summary-list">
          <div><dt>Nguồn</dt><dd>{manifest ? `${manifest.bundles.length} gói hợp lệ` : "Chưa quét"}</dd></div>
          <div><dt>Đã chọn</dt><dd>{selectedCount} bài</dd></div>
          <div><dt>Máy đích</dt><dd>{targetCount} máy</dd></div>
          <div><dt>Âm thanh</dt><dd>{soundPolicy.kind === "default" ? "Âm thanh mặc định" : `Ngẫu nhiên trong tối đa ${soundPolicy.poolSize} đề xuất`}</dd></div>
        </dl>
        <div className="publish-summary-status">
          <StatusChip tone={canExecute ? "success" : preflightState === "error" ? "error" : "neutral"}>
            {canExecute ? "Preflight đạt" : "Chưa có preflight hợp lệ"}
          </StatusChip>
          {sheetLoadState === "error" ? (
            <StatusChip tone="error">Không đọc được Sheet</StatusChip>
          ) : sheetConfig ? (
            <StatusChip tone={sheetConfig.webhookUrl && sheetConfig.hasToken ? "success" : "warning"}>
              {sheetConfig.webhookUrl && sheetConfig.hasToken ? "Sheet sẵn sàng" : "Sheet chờ cấu hình"}
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
          config={publishProfileConfig(sourceRoot.trim(), orderedBundleIds, captionOverrides, soundPolicy, true)}
          defaultName="Đăng bài theo thư mục"
          disabled={!profileReady || busy}
          disabledReason={pendingSchedule ? "Lịch hẹn chưa được tạo. Xác nhận lịch hoặc xóa thời gian hẹn trước khi lưu hồ sơ." : "Chọn đủ nội dung, máy đích và chú thích trước khi lưu hồ sơ."}
          confirmSave={() => requestConfirm({
            title: "Cho phép hồ sơ đăng công khai?",
            message: "Mỗi lần chạy hồ sơ, app có thể chuyển nội dung, chọn nhạc và đăng công khai trên các máy đích.",
            confirmLabel: "Cho phép và lưu",
            cancelLabel: "Hủy",
            danger: true,
          })}
        />
      </SummaryRail>
      <details className="publish-sheet-panel">
        <summary>Cấu hình Sheet</summary>
        {sheetLoadState === "loading" && <LoadingState label="Đang đọc cấu hình Sheet…" />}
        {sheetLoadState === "error" && (
          <StatusNotice
            tone="error"
            action={(
              <button type="button" className="ghost" onClick={() => void reloadSheetConfig()}>
                Thử lại
              </button>
            )}
          >
            {sheetLoadError ?? "Không đọc được cấu hình Sheet."}
          </StatusNotice>
        )}
        {sheetConfig && (!sheetConfig.webhookUrl || !sheetConfig.hasToken) && (
          <StatusNotice tone="warning">Sheet chưa cấu hình. Link đã xác nhận sẽ nằm trong hàng chờ.</StatusNotice>
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
            placeholder={sheetConfig?.hasToken ? "Để trống để giữ token" : "Nhập token"}
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
                  const saved = await publishSheetSaveConfig(sheetUrlDraft, "");
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
    </div>
  );
}

function PreflightTable({
  rows,
  devices,
  metas,
}: {
  rows: PublishPreflightAssignmentReport[];
  devices: SelProps["devices"];
  metas: Map<string, import("../types").DeviceMeta>;
}) {
  const check = (value: "pass" | "fail") => (
    <StatusChip tone={value === "pass" ? "success" : "error"}>{value === "pass" ? "Đạt" : "Không đạt"}</StatusChip>
  );
  return (
    <ResponsiveTable
      label="Kết quả preflight theo máy"
      rows={rows}
      keyForRow={(row) => `${row.ordinal}-${row.bundleId}-${row.udid}`}
      columns={[
        { id: "device", label: "Máy", render: (row) => deviceDisplayName(devices, metas, row.udid) },
        {
          id: "bundle",
          label: "Bài",
          render: (row) => (
            <span>
              Bài {row.ordinal + 1}
              <details className="publish-technical-details" aria-label="Chi tiết kỹ thuật bài">
                <summary>Chi tiết</summary>
                <code>{row.bundleId}</code>
              </details>
            </span>
          ),
        },
        {
          id: "environment",
          label: "TikTok",
          render: (row) => row.packageName && row.version && row.locale
            ? `${row.version} · ${row.locale}`
            : "Không đọc được",
        },
        { id: "media", label: "Nội dung", render: (row) => check(row.media) },
        {
          id: "storage",
          label: "Dung lượng",
          render: (row) => (
            <span>
              {check(row.storage)}
              <details className="publish-technical-details" aria-label="Chi tiết dung lượng">
                <summary>Chi tiết</summary>
                <code>{row.availableBytes ?? 0} / {row.requiredBytes} byte</code>
              </details>
            </span>
          ),
        },
        { id: "composer", label: "Màn đăng", render: (row) => check(row.composer) },
        { id: "sound", label: "Nhạc", render: (row) => check(row.soundPicker) },
      ]}
    />
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
          { id: "scope", label: "Phạm vi", render: (campaign) => `${campaign.assignments.length} bài` },
          {
            id: "state",
            label: "Trạng thái",
            render: (campaign) => {
              const view = campaignView(campaign, operations[campaign.id], executionSnapshots[campaign.id]);
              return <StatusChip tone={view.tone}>{view.label ?? "Trạng thái chưa nhận diện"}</StatusChip>;
            },
          },
          {
            id: "actions",
            label: "Thao tác",
            render: (campaign) => (
              <div className="publish-row-actions">
                {campaignView(campaign, operations[campaign.id], executionSnapshots[campaign.id]).retryScope !== "none" && (
                  <button type="button" className="primary" disabled={busy} onClick={() => void retryCampaign(campaign)}>
                    {retryActionLabel(campaignView(campaign, operations[campaign.id], executionSnapshots[campaign.id]).retryScope)}
                  </button>
                )}
                {CANCELLABLE_STATES.includes(campaign.state) && (
                  <button type="button" className="ghost" disabled={busy} onClick={() => void cancel(campaign)}>Huỷ</button>
                )}
                <button
                  type="button"
                  className="ghost"
                  disabled={detailLoading[campaign.id] === true}
                  onClick={() => void toggleDetail(campaign)}
                >
                  {details[campaign.id] ? "Ẩn chi tiết máy" : !operations[campaign.id] && campaign.state === "succeeded" ? "Đối chiếu kết quả" : "Chi tiết máy"}
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
    <section className="publish-campaign-detail" aria-label="Chi tiết chiến dịch đang chọn">
      {loading && <LoadingState label="Đang đối chiếu trạng thái chiến dịch…" />}
      {error && (
        <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={retry}>Thử lại</button>}>
          {error}
        </StatusNotice>
      )}
      {detail && (
        <>
          {snapshot && (
            <div className="publish-reconcile-summary">
              <StatusChip tone={snapshot.status === "complete" ? "success" : snapshot.status === "uncertain" ? "warning" : "info"}>
                {snapshot.status === "complete" ? "Đã hoàn tất" : snapshot.status === "uncertain" ? "Kết quả chưa chắc chắn" : "Còn bước cần hoàn tất"}
              </StatusChip>
              <span>{retryScopeLabel(snapshot.retryScope)}</span>
              <span>{snapshot.status === "complete" ? "Sheet đã xác nhận" : snapshot.retryScope === "sheetOnly" ? "Sheet chưa hoàn tất" : "Sheet chưa xác nhận hoàn tất"}</span>
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
                    <details className="publish-technical-details" aria-label="Chi tiết kỹ thuật máy">
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
                PUBLISH_STATE_LABELS[assignment.state] ?? "Trạng thái chưa nhận diện",
            },
            {
              id: "link",
              label: "Bài đã đăng",
              render: (assignment: PublishAssignmentRecord) => {
                const evidence = postEvidence(assignment.evidenceJson);
                return evidence.url
                  ? <a href={evidence.url} target="_blank" rel="noreferrer">Mở bài đã xác nhận</a>
                  : <span>Chưa có liên kết xác nhận</span>;
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
                      <summary>{sound.confirmed ? "Đã xác nhận nhạc" : "Chưa xác nhận nhạc"}</summary>
                      <dl>
                        <div><dt>Khu vực</dt><dd>{sound.section === "trending" ? "Thịnh hành" : sound.section === "recommended" ? "Đề xuất" : "Chưa nhận diện"}</dd></div>
                        <div><dt>Vị trí</dt><dd>{sound.index + 1}</dd></div>
                        <div><dt>Dấu xác nhận danh sách</dt><dd><code>{sound.digest}</code></dd></div>
                      </dl>
                    </details>
                  </span>
                ) : "Chưa có bằng chứng nhạc";
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
                    <details className="publish-technical-details" aria-label="Chi tiết dọn nội dung">
                      <summary>Chi tiết</summary>
                      <code>{cleanup.raw}</code>
                    </details>
                  </span>
                ) : "Chưa có bằng chứng";
              },
            },
            ]}
          />
        </>
      )}
    </section>
  );
}

function retryScopeLabel(scope: PublishExecutionSnapshot["retryScope"]): string {
  switch (scope) {
    case "fullPipeline":
      return "Có thể chạy lại từ đầu";
    case "linkAndSheet":
      return "Chỉ tiếp tục lấy liên kết và ghi Sheet";
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
