import { useEffect, useState } from "react";
import {
  publishCancel,
  publishCreateCampaign,
  publishList,
  publishPrepare,
  publishPost,
  publishScanFolder,
  publishTransfer,
} from "../api";
import { SelectionStrip } from "../components/SelectionStrip";
import { targetsOf } from "../selectionTargets";
import { EmptyState } from "../components/States";
import { IconRocket } from "../components/Icons";
import { pickDirectory } from "../pickFile";
import type { PublishBundle, PublishCampaignRecord, PublishFolderManifest } from "../types";
import { describeError } from "../describeError";
import type { SelProps } from "./pageProps";

/** Publish campaigns: scan a folder, transfer, post, and watch the result. */
export function PublishPage({ devices, selected, onSelectUdids }: SelProps) {
  const [sourceRoot, setSourceRoot] = useState("");
  const [manifest, setManifest] = useState<PublishFolderManifest | null>(null);
  const [bundleIds, setBundleIds] = useState<string[]>([]);
  const [runAt, setRunAt] = useState("");
  const [campaigns, setCampaigns] = useState<PublishCampaignRecord[]>([]);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const targets = targetsOf(selected, devices);

  const reload = () => publishList().then(setCampaigns).catch((e) => setMsg(describeError(e)));
  useEffect(() => {
    reload();
  }, []);

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

  const scan = async (path: string) => {
    setBusy(true);
    setMsg(null);
    try {
      const next = await publishScanFolder(path);
      setSourceRoot(path);
      setManifest(next);
      setBundleIds(next.bundles.slice(0, targets.length).map((bundle) => bundle.id));
    } catch (e) {
      setManifest(null);
      setBundleIds([]);
      setMsg(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Đăng carousel</h2>
        <button type="button" className="ghost" onClick={reload} disabled={busy}>
          Làm mới
        </button>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />
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
            <strong>{manifest.bundles.length} bundle · {manifest.ignoredPartnerFiles} partner bỏ qua</strong>
            <span className="hint">{manifest.ignoredHiddenFiles} file ẩn bỏ qua</span>
          </div>
          <div className="job-list" style={{ marginTop: 8, maxHeight: 330, overflow: "auto" }}>
            {manifest.bundles.map((bundle: PublishBundle) => {
              const checked = bundleIds.includes(bundle.id);
              return (
                <label key={bundle.id} className="job-card" style={{ cursor: "pointer" }}>
                  <div className="row" style={{ alignItems: "flex-start" }}>
                    <input
                      type="checkbox"
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
                      <span className="pill">{bundle.images.length} ảnh</span>
                      <p className="hint" style={{ whiteSpace: "pre-wrap" }}>
                        {bundle.caption.slice(0, 180) || "(caption rỗng)"}
                      </p>
                    </div>
                  </div>
                </label>
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
        <h3>Mapping tuần tự</h3>
        <div className="job-list">
          {selectedBundles.map((bundle, index) => (
            <article key={bundle.id} className="job-card">
              <strong>{index + 1}. {bundle.name}</strong>
              <span className="hint mono">→ {targets[index] ?? "Chưa có máy"}</span>
            </article>
          ))}
          {!selectedBundles.length && <p className="hint">Chọn bundle để tạo mapping.</p>}
        </div>
      </section>
      <label style={{ marginTop: 12 }}>
        Lịch chạy một lần (để trống = chạy ngay)
        <input type="datetime-local" value={runAt} onChange={(e) => setRunAt(e.target.value)} />
      </label>
      <p className="hint">Public · âm thanh mặc định · xoá asset sau khi có bằng chứng đăng thành công.</p>
      {androidTargets.length > 0 && (
        <p className="hint">
          {androidTargets.length} máy Android trong danh sách. Composer Android điều khiển
          theo nhãn đã đo, nên máy nào chạy bản TikTok chưa đo sẽ bị từ chối kèm tên —
          trước khi ảnh rời máy tính. Đo bằng <code>composer_scout</code>.
        </p>
      )}
      <button
        type="button"
        className="primary"
        disabled={!mappingReady || busy}
        onClick={async () => {
          setBusy(true);
          setMsg(null);
          try {
            const campaign = await publishCreateCampaign(
              sourceRoot.trim(),
              orderedBundleIds,
              targets,
              runAt || null,
            );
            if (!runAt) await publishPrepare(campaign.id);
            await reload();
            setMsg(
              runAt
                ? `Đã lập lịch ${bundleIds.length} bundle cho ${targets.length} máy.`
                : `Đã chuẩn bị ${bundleIds.length} bundle. Bấm Post để đăng native trên TikTok.`,
            );
          } catch (e) {
            setMsg(describeError(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        {runAt ? "Lập lịch" : "Tạo & chuẩn bị"} ({bundleIds.length} → {targets.length})
      </button>
      {msg && <p className="error" style={{ whiteSpace: "pre-wrap" }}>{msg}</p>}
      <div className="job-list" style={{ marginTop: 12 }}>
        {campaigns.map((campaign) => (
          <article key={campaign.id} className="job-card">
            <div>
              <strong>{campaign.id.slice(0, 8)}</strong>
              <span className={`pill ${campaign.state}`}>{campaign.state}</span>
            </div>
            <p className="hint">
              {campaign.assignments.length} mapping · {new Date(campaign.createdAt).toLocaleString()}
              {campaign.runAt ? ` · ${campaign.runAt}` : ""}
            </p>
            <div className="row">
              {campaign.state === "ready" && (
                <button
                  type="button"
                  className="primary"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishTransfer(campaign.id);
                      await reload();
                      setMsg("Đã import ảnh vào Photos. Bấm Post để mở composer TikTok.");
                    } catch (e) {
                      setMsg(describeError(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Transfer media
                </button>
              )}
              {campaign.state === "imported" && (
                <button
                  type="button"
                  className="primary"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishPost(campaign.id);
                      await reload();
                      setMsg("Đã đăng và xác nhận frame TikTok; ảnh tạm đã được dọn.");
                    } catch (e) {
                      setMsg(describeError(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Post
                </button>
              )}
              {(campaign.state === "queued" || campaign.state === "scheduled") && (
                <button
                  type="button"
                  className="ghost"
                  disabled={busy || campaign.state === "scheduled"}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishPrepare(campaign.id);
                      await reload();
                    } catch (e) {
                      setMsg(describeError(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Prepare
                </button>
              )}
              {!['succeeded', 'cancelled', 'uncertain'].includes(campaign.state) && (
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
                      setMsg(describeError(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Huỷ
                </button>
              )}
            </div>
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
      </div>
    </div>
  );
}
