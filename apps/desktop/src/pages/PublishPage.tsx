import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  listenRiviuEvents,
  publishAutoAssign,
  publishCancel,
  publishCreateCampaign,
  publishExecute,
  publishGet,
  publishList,
  publishReadiness,
  publishScanFolder,
  publishSheetGetConfig,
  publishSheetSaveConfig,
} from "../api";
import { targetsOf } from "../selectionTargets";
import { publishProfileConfig } from "../automationProfileConfig";
import { AutomationProfileControl } from "../components/AutomationProfileControl";
import { EmptyState, LoadingState, StatusNotice, type NoticeTone } from "../components/States";
import { IconRocket } from "../components/Icons";
import { requestConfirm } from "../confirmStore";
import { pickDirectory } from "../pickFile";
import type {
  DevicePublishReadiness,
  PublishBundle,
  PublishCampaignDetail,
  PublishCampaignRecord,
  PublishFolderManifest,
  PublishReadinessInfo,
  PublishSheetConfig,
  TargetRef,
} from "../types";
import { describeError } from "../describeError";
import { orderDevicesByNumber, tileName, tileNumber } from "../deviceNaming";
import type { SelProps } from "./pageProps";

/**
 * One readiness answer as pill text. The `default` arm is deliberate wire-defence: this
 * union mirrors a Rust enum, and a variant this page has not heard of must render as its
 * raw JSON rather than as nothing — an empty chip would read as "fine".
 *
 * **`hierarchyReady` is not a promise about this phone.** The backend answers it from the
 * shortest gap across every catalogued (language, version) set for the package, without
 * reading the build the phone is actually running — while Post refuses unless that exact
 * pair is catalogued. So a phone whose TikTok updated itself keeps a green chip and is
 * refused at the first tap. Until the command reads the phone's own version and locale
 * (see the note on the refresh button), the wording says what was really checked.
 */
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

function readinessView(info: PublishReadinessInfo): { label: string; raw?: string } {
  switch (info.kind) {
    case "hierarchyReady":
      return { label: "bản đo có đủ nhãn (chưa đối chiếu build máy)" };
    case "pixelGrid":
      return { label: "đường pixel" };
    case "hierarchyMissing":
      return {
        label: `thiếu ${info.labels.map((label) => LOCATOR_LABELS[label] ?? "một điều khiển chưa nhận diện").join(", ")}`,
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
    const message = "message" in cleanup
      ? String((cleanup as { message?: unknown }).message ?? "").trim()
      : "";
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

const PUBLISH_STATE_LABELS: Record<PublishCampaignRecord["state"], string> = {
  queued: "Đang chờ",
  scheduled: "Đã lên lịch",
  preparing: "Đang kiểm tra",
  ready: "Sẵn sàng",
  transferring: "Đang chuyển nội dung",
  imported: "Đã nhập nội dung",
  posting: "Đang đăng",
  verifying: "Đang xác nhận",
  succeeded: "Hoàn tất",
  failedBeforeDispatch: "Dừng trước khi đăng",
  uncertain: "Chưa chắc chắn",
  cancelled: "Đã huỷ",
  missed: "Lỡ lịch",
};

function stableSoundSeed(approvedInput: string): number {
  let value = 0x811c9dc5;
  for (const char of approvedInput) {
    value ^= char.charCodeAt(0);
    value = Math.imul(value, 0x01000193);
  }
  return value >>> 0;
}

function mediaSummary(bundle: PublishBundle): string {
  if (bundle.mediaKind === "video" && bundle.video) {
    const seconds = Math.round(bundle.video.durationMs / 1000);
    const minutes = Math.floor(seconds / 60);
    const rest = String(seconds % 60).padStart(2, "0");
    return `Video · ${minutes}:${rest} · ${(bundle.video.byteLen / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${bundle.images.length} ảnh`;
}

function deviceLabel(
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

type PublishPageProps = SelProps & {
  targetUdids?: string[];
  targetRef?: TargetRef;
  metas?: Map<string, import("../types").DeviceMeta>;
};

/** Publish campaigns: scan a folder, transfer, post, and watch the result. */
export function PublishPage(props: PublishPageProps) {
  const {
    devices,
    selected,
    targetUdids,
    targetRef = { type: "all" },
    metas = new Map(),
  } = props;
  const [workspaceTab, setWorkspaceTab] = useState<"setup" | "monitor">("setup");
  const [sourceRoot, setSourceRoot] = useState("");
  const [manifest, setManifest] = useState<PublishFolderManifest | null>(null);
  const [bundleIds, setBundleIds] = useState<string[]>([]);
  const [captionDrafts, setCaptionDrafts] = useState<Record<string, string>>({});
  const [runAt, setRunAt] = useState("");
  const [campaigns, setCampaigns] = useState<PublishCampaignRecord[]>([]);
  const [campaignLoadState, setCampaignLoadState] = useState<"loading" | "ready" | "error">(
    "loading",
  );
  const [campaignLoadError, setCampaignLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ tone: NoticeTone; text: string } | null>(null);
  const [readiness, setReadiness] = useState<DevicePublishReadiness[]>([]);
  const [readinessNote, setReadinessNote] = useState<string | null>(null);
  // `null` means "not answered yet" — the unconfigured badge must not flash while the
  // config is still loading, so it renders only from a real answer.
  const [sheetConfig, setSheetConfig] = useState<PublishSheetConfig | null>(null);
  const [sheetUrlDraft, setSheetUrlDraft] = useState("");
  const [sheetTokenDraft, setSheetTokenDraft] = useState("");
  const [sheetBusy, setSheetBusy] = useState(false);
  /** Per-campaign detail, fetched on demand — `publishList` carries plans, not states. */
  const [details, setDetails] = useState<Record<string, PublishCampaignDetail>>({});
  const targets = targetUdids ?? targetsOf(selected, devices);

  // **Sequenced, because a run emits several events close together.** Each reload takes a
  // ticket and only the newest ticket may write: without that, reload A (started while a
  // campaign was `posting`) can resolve *after* reload B (started once it was `succeeded`) and
  // put the older state back on screen, where it stays until the operator navigates away.
  const reloadTicket = useRef(0);
  const reload = () => {
    const ticket = ++reloadTicket.current;
    setCampaignLoadState((current) => (current === "ready" ? current : "loading"));
    setCampaignLoadError(null);
    return publishList()
      .then((next) => {
        if (ticket === reloadTicket.current) {
          setCampaigns(next);
          setCampaignLoadState("ready");
        }
      })
      .catch((e) => {
        if (ticket === reloadTicket.current) {
          setCampaignLoadError(describeError(e));
          setCampaignLoadState("error");
        }
      });
  };
  useEffect(() => {
    reload();
    // **Follow a run while it runs.** Publish emitted no event at all before, so a campaign
    // that took twenty minutes across five phones left this page frozen at the moment the
    // button was pressed — the only way to see progress was to navigate away and back.
    //
    // The payload carries an id and a revision and this re-reads the list rather than
    // trusting it: a broadcast that lost a race would otherwise render a state the database
    // has already moved past.
    let unlisten: UnlistenFn | undefined;
    let live = true;
    listenRiviuEvents((event) => {
      if (event.type === "publishUpdated") reload();
    })
      .then((off) => {
        // StrictMode double-invokes effects, so the cleanup of the first run can arrive
        // before this resolves; without the flag that listener is never detached.
        if (live) unlisten = off;
        else off();
      })
      .catch(() => undefined);
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let live = true;
    publishSheetGetConfig()
      .then((config) => {
        if (!live) return;
        setSheetConfig(config);
        setSheetUrlDraft(config.webhookUrl);
      })
      .catch(() => {
        // A page that cannot read the config still publishes fine; the sweeper is the
        // one that cares, and it says so in its own log.
        if (live) setSheetConfig(null);
      });
    return () => {
      live = false;
    };
  }, []);

  // **Keyed on the udid set, not on the `devices` array.** The roster identity churns on
  // every 3-second scan, and `readiness_of` resolves each phone's TikTok package over adb —
  // refetching that per scan would put twenty shell round-trips on a timer for an answer
  // that only changes when a phone (or its TikTok) comes or goes.
  const androidKey = devices
    .filter((device) => device.platform === "android")
    .map((device) => device.udid)
    .sort()
    .join(",");
  // Bumped by the refresh button. A phone whose TikTok updates in place keeps the same
  // udid, so the key above cannot notice it — and that is exactly the change readiness is
  // keyed on a build for. Without a way to re-ask, the only cure was to unplug the phone.
  const [readinessNonce, setReadinessNonce] = useState(0);
  useEffect(() => {
    if (androidKey === "") {
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
        // The rows that are on screen described the fleet at the last successful answer;
        // leaving them up beside an error is the same "stale answer shown as current"
        // shape the chips' own wording is being fixed for.
        setReadiness([]);
        setReadinessNote(describeError(error));
      });
    return () => {
      live = false;
    };
  }, [androidKey, readinessNonce]);

  const selectedBundles =
    manifest?.bundles.filter((bundle) => bundleIds.includes(bundle.id)) ?? [];
  // **The order that is shown is the order that is sent.**
  //
  // `bundleIds` is checkbox history: the handler appends on tick, so unticking bo2 and
  // reconsidering it leaves [bo1, bo3, bo2] while `selectedBundles` — which the preview
  // below iterates — is still scanned-folder order. Sending `bundleIds` therefore paired
  // each phone with a different bundle than the operator had just read, and the pairing is
  // positional the whole way down (`validate_publish_mapping` zips `bundle_ids[i]` with
  // `udids[i]`), so nothing downstream could notice. Every phone is a different live
  // TikTok account: the cost is one account posting another's photographs under another's
  // caption, with no error, no discrepancy in the evidence, and no delete path to undo it.
  const orderedBundleIds = selectedBundles.map((bundle) => bundle.id);
  // **Android is no longer refused here, and the reason is that this is the wrong place to
  // ask.** The old gate refused every Android target outright, correctly, because there was
  // no composer for them. There is one now — driven by measured labels — so the real question
  // is per *build*: has this phone's TikTok had the publish controls read off it? That needs
  // the device's package, language and app version, which only the backend can read, so the
  // backend refuses by name and this page reports what it said.
  //
  // Nothing is silently dropped either way: the bundle -> device mapping is positional
  // (`targets[index]` below), so removing a target would re-index the rest and post the wrong
  // caption to the wrong account.
  const androidTargets = targets.filter(
    (udid) => devices.find((device) => device.udid === udid)?.platform === "android",
  );
  const mappingReady =
    selectedBundles.length > 0 && selectedBundles.length === targets.length;
  const currentCaptionOverrides = Object.fromEntries(
    selectedBundles.map((bundle) => [
      bundle.id,
      (captionDrafts[bundle.id] ?? bundle.caption).trim(),
    ]),
  );
  const currentSoundPolicy = {
    kind: "trendingAny" as const,
    poolSize: 5,
    seed: stableSoundSeed(JSON.stringify({
      sourceRoot: sourceRoot.trim(),
      bundleIds: orderedBundleIds,
      targets,
      runAt: runAt || null,
      captions: orderedBundleIds.map((id) => currentCaptionOverrides[id]),
    })),
  };
  const profileReady =
    mappingReady && Object.values(currentCaptionOverrides).every((caption) => caption.length > 0);

  const scan = async (path: string) => {
    setBusy(true);
    setNotice(null);
    try {
      const next = await publishScanFolder(path);
      setSourceRoot(path);
      setManifest(next);
      setBundleIds(next.bundles.slice(0, targets.length).map((bundle) => bundle.id));
      setCaptionDrafts(Object.fromEntries(next.bundles.map((bundle) => [bundle.id, bundle.caption])));
    } catch (e) {
      setManifest(null);
      setBundleIds([]);
      setNotice({ tone: "error", text: describeError(e) });
    } finally {
      setBusy(false);
    }
  };

  const activateWorkspaceTab = (next: "setup" | "monitor") => {
    setWorkspaceTab(next);
    document.getElementById(`publish-tab-${next}`)?.focus();
  };
  const onWorkspaceTabKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    let next: "setup" | "monitor" | null = null;
    if (event.key === "ArrowRight" || event.key === "End") next = "monitor";
    if (event.key === "ArrowLeft" || event.key === "Home") next = "setup";
    if (!next) return;
    event.preventDefault();
    activateWorkspaceTab(next);
  };

  return (
    <div className="panel">
      <header className="panel-header">
        <div>
          <h2>Đăng bài</h2>
          <p className="hint">Chuẩn bị, đăng và ghi nhận Sheet cho gói ảnh hoặc video.</p>
        </div>
        <button type="button" className="ghost" onClick={reload} disabled={busy}>
          Làm mới
        </button>
      </header>
      <div className="automation-tabs" role="tablist" aria-label="Không gian Đăng bài">
        <button
          id="publish-tab-setup"
          type="button"
          role="tab"
          aria-selected={workspaceTab === "setup"}
          aria-controls="publish-panel-setup"
          tabIndex={workspaceTab === "setup" ? 0 : -1}
          onClick={() => setWorkspaceTab("setup")}
          onKeyDown={onWorkspaceTabKeyDown}
        >
          Thiết lập
        </button>
        <button
          id="publish-tab-monitor"
          type="button"
          role="tab"
          aria-selected={workspaceTab === "monitor"}
          aria-controls="publish-panel-monitor"
          tabIndex={workspaceTab === "monitor" ? 0 : -1}
          onClick={() => setWorkspaceTab("monitor")}
          onKeyDown={onWorkspaceTabKeyDown}
        >
          Theo dõi
        </button>
      </div>
      <section
        id="publish-panel-setup"
        className="publish-workspace-section"
        role="tabpanel"
        aria-labelledby="publish-tab-setup"
        hidden={workspaceTab !== "setup"}
      >
      <div className="row" style={{ marginTop: 8 }}>
        <input
          style={{ flex: 1 }}
          value={sourceRoot}
          onChange={(e) => setSourceRoot(e.target.value)}
          placeholder="Thư mục chứa bo1, bo2…"
        />
        <button
          type="button"
          className="ghost"
          disabled={busy}
          onClick={async () => {
            const path = await pickDirectory();
            if (path) await scan(path);
          }}
        >
          Chọn thư mục
        </button>
        <button
          type="button"
          className="primary"
          disabled={!sourceRoot.trim() || busy}
          onClick={() => scan(sourceRoot.trim())}
        >
          Quét
        </button>
      </div>
      {manifest && (
        <>
          <div className="row" style={{ marginTop: 10, justifyContent: "space-between" }}>
            <strong>{manifest.bundles.length} gói nội dung</strong>
            <details className="publish-scan-details">
              <summary>Chi tiết quét</summary>
              <span className="hint">
                {manifest.ignoredPartnerFiles} tệp đối tác và {manifest.ignoredHiddenFiles} tệp ẩn được bỏ qua
              </span>
            </details>
          </div>
          <div className="job-list" style={{ marginTop: 8, maxHeight: 330, overflow: "auto" }}>
            {manifest.bundles.map((bundle: PublishBundle) => {
              const checked = bundleIds.includes(bundle.id);
              return (
                <article key={bundle.id} className="job-card publish-bundle-card">
                  <div className="row" style={{ alignItems: "flex-start" }}>
                    <input
                      type="checkbox"
                      aria-label={`Chọn ${bundle.name}`}
                      checked={checked}
                      onChange={(e) => {
                        setBundleIds((current) =>
                          e.target.checked
                            ? [...current, bundle.id]
                            : current.filter((id) => id !== bundle.id),
                        );
                      }}
                    />
                    <div style={{ flex: 1 }}>
                      <strong>{bundle.name}</strong>
                      <span className="pill">{mediaSummary(bundle)}</span>
                      <label className="publish-caption-field">
                      <span>Chú thích</span>
                      <textarea
                          aria-label={`Chú thích cho ${bundle.name}`}
                          value={captionDrafts[bundle.id] ?? bundle.caption}
                          onChange={(event) => {
                            const caption = event.target.value;
                            setCaptionDrafts((current) => ({ ...current, [bundle.id]: caption }));
                          }}
                          rows={3}
                        />
                      </label>
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
          {manifest.notices.length > 0 && (
            <p className="hint" style={{ whiteSpace: "pre-wrap" }}>
              {manifest.notices.map((notice) => notice.message).join("\n")}
            </p>
          )}
        </>
      )}
      <section style={{ marginTop: 12 }}>
        <h3>Ghép bài với máy</h3>
        <div className="job-list">
          {selectedBundles.map((bundle, index) => (
            <article key={bundle.id} className="job-card">
              <strong>{index + 1}. {bundle.name}</strong>
              <span className="hint">→ {targets[index] ? deviceLabel(devices, metas, targets[index]) : "Chưa có máy"}</span>
            </article>
          ))}
          {!selectedBundles.length && <p className="hint">Chọn bài để ghép với máy.</p>}
        </div>
        {/*
          Ticking twenty-one checkboxes against twenty phones is a pairing done by hand every
          run, and the pairing is positional all the way down — a slip there posts one
          account's photographs under another's caption, with no delete. This asks the
          database which bundles have not gone out yet and fills the boxes in that order.
        */}
        <button
          type="button"
          disabled={busy || !manifest || targets.length === 0}
          onClick={async () => {
            if (!manifest) return;
            setBusy(true);
            setNotice(null);
            try {
              const deal = await publishAutoAssign(sourceRoot.trim(), targets, targets.length);
              setBundleIds(deal.plan.map((row) => row.bundleId));
              setNotice({
                tone: "success",
                text: `Đã chia ${deal.plan.length} bài chưa đăng cho ${targets.length} máy.`,
              });
            } catch (e) {
              setNotice({ tone: "error", text: describeError(e) });
            } finally {
              setBusy(false);
            }
          }}
        >
          Chia tự động ({targets.length} máy)
        </button>
      </section>
      <label style={{ marginTop: 12 }}>
        Lịch chạy một lần (để trống = chạy ngay)
        <input type="datetime-local" value={runAt} onChange={(e) => setRunAt(e.target.value)} />
      </label>
      <details className="publish-operation-details">
        <summary>Quy trình thực hiện</summary>
        <p className="hint">
          App kiểm tra nội dung, chuyển sang máy, chọn một âm thanh đang được TikTok đề xuất,
          đăng công khai, lấy liên kết, ghi Sheet rồi dọn nội dung tạm. Nếu trạng thái sau nút
          Đăng không chắc chắn, app dừng để tránh đăng trùng.
        </p>
      </details>
      {androidTargets.length > 0 && (
        <p className="hint">
          {androidTargets.length} máy Android. Máy chạy bản TikTok chưa được hỗ trợ sẽ bị từ
          chối trước khi chuyển nội dung, kèm tên máy.
        </p>
      )}
      {readinessNote && (
        <p className="hint" role="alert">
          Không đọc được trạng thái sẵn sàng: {readinessNote}
        </p>
      )}
      {readiness.length > 0 && (
        <div className="row" style={{ flexWrap: "wrap", gap: 6 }}>
          {/* The preflight's own answer, shown before the refusal instead of inside it. */}
          {readiness.map(({ udid, readiness: info }) => {
            const view = readinessView(info);
            return (
              <span
                key={udid}
                className="pill"
                title={view.raw ? `${udid} · ${view.raw}` : udid}
              >
                {deviceLabel(devices, metas, udid)}:{" "}
                {view.label}
              </span>
            );
          })}
          {/*
            A phone that updates TikTok in place keeps its udid, so nothing re-asks on its
            own. This is the operator's way to say "I just changed that phone".
          */}
          <button
            type="button"
            className="ghost"
            onClick={() => setReadinessNonce((nonce) => nonce + 1)}
          >
            Hỏi lại
          </button>
        </div>
      )}
      {sheetConfig && (!sheetConfig.webhookUrl || !sheetConfig.hasToken) && (
        <p className="hint" role="status">
          Sheet chưa cấu hình — bài đăng xong sẽ giữ link trong hàng chờ (`pending`) cho tới
          khi điền webhook + token bên dưới.
        </p>
      )}
      <details style={{ marginTop: 8 }}>
        <summary>Cấu hình Sheet (Apps Script webhook)</summary>
        <div className="row" style={{ flexWrap: "wrap", gap: 6, marginTop: 6 }}>
          <input
            type="text"
            placeholder="https://script.google.com/…/exec"
            value={sheetUrlDraft}
            onChange={(event) => setSheetUrlDraft(event.target.value)}
            style={{ minWidth: 320 }}
            aria-label="Webhook URL"
          />
          <input
            type="password"
            placeholder={
              sheetConfig?.hasToken ? "để trống = giữ token hiện tại" : "token của script"
            }
            value={sheetTokenDraft}
            onChange={(event) => setSheetTokenDraft(event.target.value)}
            aria-label="Webhook token"
          />
          <button
            type="button"
            className="ghost"
            disabled={sheetBusy}
            onClick={async () => {
              setSheetBusy(true);
              try {
                // An untouched token field means "keep what is stored" — the operator can
                // fix a URL without holding the credential; clearing is its own button.
                const saved = await publishSheetSaveConfig(
                  sheetUrlDraft,
                  sheetTokenDraft === "" ? undefined : sheetTokenDraft,
                );
                setSheetConfig(saved);
                setSheetUrlDraft(saved.webhookUrl);
                setSheetTokenDraft("");
            setNotice({ tone: "success", text: "Đã lưu cấu hình Sheet." });
          } catch (e) {
            setNotice({ tone: "error", text: describeError(e) });
              } finally {
                setSheetBusy(false);
              }
            }}
          >
            Lưu
          </button>
          {sheetConfig?.hasToken && (
            <button
              type="button"
              className="ghost"
              disabled={sheetBusy}
              onClick={async () => {
                setSheetBusy(true);
                try {
                  const saved = await publishSheetSaveConfig(sheetUrlDraft, "");
                  setSheetConfig(saved);
                  setNotice({ tone: "success", text: "Đã xoá token." });
                } catch (e) {
                  setNotice({ tone: "error", text: describeError(e) });
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
      <button
        type="button"
        className="primary"
        disabled={!mappingReady || busy}
        onClick={async () => {
          if (Object.values(currentCaptionOverrides).some((caption) => caption.length === 0)) {
            setNotice({ tone: "error", text: "Mỗi bài phải có chú thích trước khi chạy." });
            return;
          }
          const confirmed = await requestConfirm({
            title: runAt ? "Xác nhận lập lịch đăng bài?" : "Xác nhận đăng công khai?",
            message: runAt
              ? `${selectedBundles.length} bài sẽ chạy trên ${targets.length} máy vào lịch đã chọn.`
              : `${selectedBundles.length} bài sẽ được đăng công khai trên ${targets.length} máy. App sẽ chọn âm thanh từ danh sách TikTok đang hiển thị.`,
            confirmLabel: runAt ? "Lập lịch" : "Đăng bài",
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
              true,
            );
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
                    ? "Đã đăng, lấy liên kết và ghi Sheet."
                    : result.status === "uncertain"
                      ? "Có máy chưa xác định được kết quả sau thao tác Đăng. App đã dừng để tránh đăng trùng."
                      : "Quy trình đã dừng ở bước chưa hoàn tất. Mở chi tiết để xử lý phần còn lại.",
              });
            } else {
              setNotice({
                tone: "success",
                text: `Đã lập lịch ${selectedBundles.length} bài cho ${targets.length} máy.`,
              });
            }
            await reload();
          } catch (e) {
            setNotice({ tone: "error", text: describeError(e) });
          } finally {
            setBusy(false);
          }
        }}
      >
        {runAt ? "Xác nhận và lập lịch" : "Xác nhận và đăng"} ({bundleIds.length} → {targets.length})
      </button>
      <AutomationProfileControl
        kind="publish"
        target={targetRef}
        config={publishProfileConfig(
          sourceRoot.trim(),
          orderedBundleIds,
          currentCaptionOverrides,
          currentSoundPolicy,
          true,
        )}
        defaultName="Đăng bài theo thư mục"
        disabled={!profileReady || busy}
        disabledReason="Chọn đủ nội dung, máy đích và chú thích trước khi lưu hồ sơ."
        confirmSave={() => requestConfirm({
          title: "Cho phép hồ sơ đăng công khai?",
          message:
            "Mỗi lần hồ sơ này được chạy, app có thể chuyển nội dung, chọn nhạc và bấm Đăng công khai trên các máy đích. Kiểm tra an toàn trước/sau nút Đăng vẫn được giữ để tránh đăng trùng.",
          confirmLabel: "Cho phép và lưu",
          cancelLabel: "Hủy",
          danger: true,
        })}
      />
      </section>
      {notice && <StatusNotice tone={notice.tone}>{notice.text}</StatusNotice>}
      <section
        id="publish-panel-monitor"
        className="publish-workspace-section"
        role="tabpanel"
        aria-labelledby="publish-tab-monitor"
        hidden={workspaceTab !== "monitor"}
      >
      {campaignLoadState === "loading" && <LoadingState label="Đang tải chiến dịch…" />}
      {campaignLoadState === "error" && (
        <StatusNotice
          tone="error"
          action={(
            <button type="button" className="ghost" onClick={() => void reload()}>
              Thử lại
            </button>
          )}
        >
          {campaignLoadError ?? "Không tải được chiến dịch."}
        </StatusNotice>
      )}
      {campaignLoadState === "ready" && <div className="job-list" style={{ marginTop: 12 }}>
        {campaigns.map((campaign, campaignIndex) => (
          <article key={campaign.id} className="job-card">
            <div className="row publish-campaign-title">
              <strong>Chiến dịch {campaignIndex + 1}</strong>
              <span className={`pill ${campaign.state}`}>
                {PUBLISH_STATE_LABELS[campaign.state] ?? "Trạng thái chưa nhận diện"}
              </span>
            </div>
            <p className="hint">
              {campaign.assignments.length} bài · {new Date(campaign.createdAt).toLocaleString()}
              {campaign.runAt ? ` · ${campaign.runAt}` : ""}
            </p>
            <div className="row">
              {(["queued", "ready", "imported", "failedBeforeDispatch"] as const).includes(
                campaign.state as "queued" | "ready" | "imported" | "failedBeforeDispatch",
              ) && (
                <button
                  type="button"
                  className="primary"
                  disabled={busy}
                  onClick={async () => {
                    const confirmed = await requestConfirm({
                      title: "Xác nhận tiếp tục đăng bài?",
                      message: "App sẽ chỉ tiếp tục từ ranh giới hiệu ứng đã được lưu. Trạng thái chưa chắc chắn sẽ không tự đăng lại.",
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
                    } catch (e) {
                      setNotice({ tone: "error", text: describeError(e) });
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  {campaign.state === "failedBeforeDispatch" ? "Chạy lại từ đầu" : "Tiếp tục quy trình"}
                </button>
              )}
              {(["queued", "scheduled", "preparing", "ready", "failedBeforeDispatch"] as const).includes(
                campaign.state as "queued" | "scheduled" | "preparing" | "ready" | "failedBeforeDispatch",
              ) && (
                <button
                  type="button"
                  className="ghost"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishCancel(campaign.id);
                      await reload();
                    } catch (e) {
                      setNotice({ tone: "error", text: describeError(e) });
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Huỷ
                </button>
              )}
              <button
                type="button"
                className="ghost"
                onClick={async () => {
                  if (details[campaign.id]) {
                    setDetails((current) => {
                      const next = { ...current };
                      delete next[campaign.id];
                      return next;
                    });
                    return;
                  }
                  try {
                    const detail = await publishGet(campaign.id);
                    if (detail) setDetails((current) => ({ ...current, [campaign.id]: detail }));
                    else setNotice({ tone: "warning", text: "Chiến dịch không còn trong dữ liệu." });
                  } catch (e) {
                    setNotice({ tone: "error", text: describeError(e) });
                  }
                }}
              >
                {details[campaign.id] ? "Ẩn chi tiết máy" : "Chi tiết máy"}
              </button>
            </div>
            {details[campaign.id] && (
              <ul className="hint" style={{ marginTop: 8 }}>
                {/*
                  `publishList` carries assignment PLANS (bundle↔udid), so per-phone state
                  and errorCode were invisible on this page — a campaign could sit
                  `failedBeforeDispatch` with the one refusing phone unnameable except by
                  reading the backend log. This is the read the retry buttons act on.
                */}
                {details[campaign.id].assignments.map((assignment) => {
                  const cleanup = cleanupEvidence(assignment.evidenceJson);
                  const rawDetail = [
                    `UDID: ${assignment.udid}`,
                    `state: ${assignment.state}`,
                    assignment.errorCode ? `error: ${assignment.errorCode}` : null,
                  ]
                    .filter(Boolean)
                    .join(" · ");
                  return (
                    <li key={assignment.id} title={rawDetail}>
                      {deviceLabel(devices, metas, assignment.udid)}
                      {" — "}
                      {PUBLISH_STATE_LABELS[assignment.state] ?? "Trạng thái chưa nhận diện"}
                      {cleanup && <span title={cleanup.raw}> · {cleanup.label}</span>}
                    </li>
                  );
                })}
              </ul>
            )}
          </article>
        ))}
        {!campaigns.length && (
          <EmptyState
            compact
            icon={<IconRocket size={15} />}
            title="Chưa có chiến dịch"
            hint="Tạo chiến dịch ở khung bên trên để bắt đầu đăng bài."
          />
        )}
      </div>}
      </section>
    </div>
  );
}
