import { useCallback, useEffect, useMemo, useState } from "react";
import {
  interactionCancel,
  interactionGet,
  interactionList,
  interactionListArtifacts,
  interactionReadArtifact,
  interactionParseLinks,
  interactionResolveLinks,
  interactionStartThread,
  listenRiviuEvents,
} from "../api";
import type { InteractionArtifactRecord } from "../api";
import type {
  DeviceInfo,
  ThreadMode,
  InteractionCampaignDetail,
  InteractionCampaignSummary,
  ThreadCampaignRequest,
  TikTokLinkLine,
} from "../types";
import { IconChat, IconClose } from "./Icons";

type Props = {
  devices: DeviceInfo[];
  selected: string[];
  onClose: () => void;
};

function requestId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `interaction-${Date.now()}`;
}

function stateLabel(state: string) {
  const labels: Record<string, string> = {
    queued: "Đang chờ",
    running: "Đang chạy",
    succeeded: "Đã gửi",
    partial: "Một phần",
    failed: "Lỗi",
    cancelled: "Đã hủy",
    ready: "Đã chuẩn bị",
    preparing: "Đang chuẩn bị",
    sending: "Đang gửi",
    uncertain: "Chưa xác nhận",
    skippedParent: "Chưa xác nhận parent",
  };
  return labels[state] ?? state;
}

export function InteractionPopup({ devices, selected, onClose }: Props) {
  const actorChoices = useMemo(
    () => devices.filter((device) => (selected.length ? selected.includes(device.udid) : true)),
    [devices, selected],
  );
  const [tab, setTab] = useState<"setup" | "monitor">("setup");
  const [rawLinks, setRawLinks] = useState("");
  const [lines, setLines] = useState<TikTokLinkLine[]>([]);
  const [actors, setActors] = useState<string[]>([]);
  const [messageCount, setMessageCount] = useState(2);
  const [maxWords, setMaxWords] = useState(12);
  const [mode, setMode] = useState<ThreadMode>("threaded");
  const [instruction, setInstruction] = useState("tự nhiên, ngắn, nói như người vừa xem xong");
  const [campaigns, setCampaigns] = useState<InteractionCampaignSummary[]>([]);
  const [detail, setDetail] = useState<InteractionCampaignDetail | null>(null);
  const [artifacts, setArtifacts] = useState<InteractionArtifactRecord[]>([]);
  const [preview, setPreview] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const validTargets = useMemo(
    () => lines.flatMap((line) => (line.target ? [line.target] : [])),
    [lines],
  );
  const request: ThreadCampaignRequest = useMemo(
    () => ({
      requestId: requestId(),
      targets: validTargets,
      actorUdids: actors,
      messageCount,
      instruction,
      maxWords,
      mode,
    }),
    [actors, instruction, maxWords, messageCount, mode, validTargets],
  );

  const reloadCampaigns = useCallback(async () => {
    try {
      setCampaigns(await interactionList());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void reloadCampaigns();
    const first = actorChoices.slice(0, 6).map((device) => device.udid);
    setActors(first);
  }, [actorChoices, reloadCampaigns]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listenRiviuEvents((payload) => {
      const event = payload as { type?: string; campaignId?: string };
      if (event.type !== "interactionUpdated") return;
      void reloadCampaigns();
      if (event.campaignId && detail?.summary.id === event.campaignId) {
        void interactionGet(event.campaignId).then(setDetail).catch(() => undefined);
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [detail?.summary.id, reloadCampaigns]);

  const parse = async (value: string) => {
    setRawLinks(value);
    try {
      setLines(await interactionParseLinks(value));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const resolveShortLinks = async () => {
    if (!rawLinks.trim()) return;
    setBusy(true);
    try {
      setLines(await interactionResolveLinks(rawLinks));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const run = async () => {
    if (validTargets.length === 0) {
      setError("Cần ít nhất một link video/photo hợp lệ");
      return;
    }
    if (actors.length < 2 || actors.length > 6) {
      setError("Chọn từ 2 đến 6 thiết bị làm actor");
      return;
    }
    if (messageCount < actors.length) {
      setError("Số message phải lớn hơn hoặc bằng số actor");
      return;
    }
    setBusy(true);
    try {
      await interactionStartThread(request);
      setTab("monitor");
      await reloadCampaigns();
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const openDetail = async (campaign: InteractionCampaignSummary) => {
    setBusy(true);
    setPreview(null);
    try {
      setDetail(await interactionGet(campaign.id));
      // Saved frames are what makes a campaign result checkable rather than
      // just asserted; a campaign that has none still opens.
      setArtifacts(await interactionListArtifacts(campaign.id).catch(() => []));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const showShot = async (artifactId: string) => {
    try {
      const payload = await interactionReadArtifact(artifactId);
      setPreview(`data:${payload.mimeType};base64,${payload.base64}`);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="interaction-float-layer" aria-label="Tương tác comment">
      <section className="interaction-float">
        <header className="interaction-title">
          <IconChat size={15} />
          <strong>Tương tác</strong>
          <span className="hint">{actorChoices.length} thiết bị</span>
          <div className="grow" />
          <button type="button" className="close" title="Đóng" onClick={onClose}>
            <IconClose size={14} />
          </button>
        </header>
        <div className="interaction-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={tab === "setup"} onClick={() => setTab("setup")}>
            Setup
          </button>
          <button type="button" role="tab" aria-selected={tab === "monitor"} onClick={() => setTab("monitor")}>
            Monitor
          </button>
        </div>
        {error && <div className="banner error">{error}</div>}
        {tab === "setup" ? (
          <div className="interaction-body">
            <label>
              Link TikTok (mỗi dòng một link)
              <textarea
                value={rawLinks}
                onChange={(event) => void parse(event.target.value)}
                placeholder="https://www.tiktok.com/@creator/video/123"
                rows={5}
              />
            </label>
            <div className="interaction-link-list">
              {lines.map((line) => (
                <div key={line.lineNo} className={line.target ? "ok" : "bad"}>
                  <span>{line.target ? "✓" : "!"}</span>
                  <span>{line.target?.normalizedUrl ?? `${line.original} · ${line.error}`}</span>
                </div>
              ))}
            </div>
            {lines.some((line) => line.error === "unresolvedShortLink") && (
              <button type="button" className="ghost" disabled={busy} onClick={() => void resolveShortLinks()}>
                Resolve link rút gọn
              </button>
            )}
            <div className="interaction-grid">
              <label>
                Số message
                <input type="number" min={2} max={6} value={messageCount} onChange={(e) => setMessageCount(Number(e.target.value))} />
              </label>
              <label>
                Tối đa từ
                <input type="number" min={4} max={20} value={maxWords} onChange={(e) => setMaxWords(Number(e.target.value))} />
              </label>
            </div>
            <label>
              Kiểu tương tác
              <select value={mode} onChange={(e) => setMode(e.target.value as ThreadMode)}>
                <option value="threaded">Qua lại — acc sau trả lời acc trước</option>
                <option value="standalone">Riêng lẻ — mỗi acc một bình luận gốc</option>
              </select>
            </label>
            <p className="hint">
              {mode === "threaded"
                ? "Tạo hội thoại lồng nhau. Cần OCR đọc được tiếng Việt để tìm lại bình luận cha — hiện chỉ có trên macOS."
                : "Mỗi acc để một bình luận riêng, không lồng nhau. Không cần OCR, chạy được trên mọi máy."}
            </p>
            <label>
              Giọng điệu / hướng dẫn
              <input value={instruction} onChange={(e) => setInstruction(e.target.value)} />
            </label>
            <fieldset className="interaction-actors">
              <legend>Actor tham gia</legend>
              {actorChoices.length === 0 && <span className="hint">Chưa có thiết bị</span>}
              {actorChoices.map((device) => (
                <label key={device.udid}>
                  <input
                    type="checkbox"
                    checked={actors.includes(device.udid)}
                    onChange={() => setActors((prev) => (prev.includes(device.udid) ? prev.filter((id) => id !== device.udid) : [...prev, device.udid]))}
                  />
                  <span>{device.name || device.udid.slice(0, 8)}</span>
                </label>
              ))}
            </fieldset>
            <button type="button" className="primary" disabled={busy} onClick={() => void run()}>
              Chạy ngay
            </button>
          </div>
        ) : (
          <div className="interaction-body">
            <div className="interaction-monitor-head">
              <strong>Campaign gần đây</strong>
              <button type="button" className="ghost" onClick={() => void reloadCampaigns()}>Làm mới</button>
            </div>
            <div className="interaction-campaign-list">
              {campaigns.map((campaign) => (
                <button type="button" key={campaign.id} className="interaction-campaign" onClick={() => void openDetail(campaign)}>
                  <span className={`status-dot ${campaign.state}`} />
                  <span className="grow">
                    <strong>{campaign.requestId.slice(0, 14)}</strong>
                    <small>{campaign.targetCount} link · {campaign.succeededMessages}/{campaign.messageCount * campaign.targetCount} message · {stateLabel(campaign.state)}</small>
                  </span>
                </button>
              ))}
              {!campaigns.length && <span className="hint">Chưa có campaign</span>}
            </div>
            {detail && (
              <div className="interaction-detail">
                <div className="interaction-monitor-head">
                  <strong>{stateLabel(detail.summary.state)}</strong>
                  {detail.summary.state === "running" && <button type="button" className="danger" onClick={() => void interactionCancel(detail.summary.id)}>Dừng</button>}
                </div>
                {detail.assignments.map((assignment) => {
                  const shot = artifacts.find(
                    (item) => item.assignmentId === assignment.id && item.relativePath,
                  );
                  return (
                    <div key={assignment.id} className="interaction-assignment">
                      <span>#{assignment.ordinal + 1}</span>
                      <span className="grow"><strong>{assignment.actorUdid.slice(0, 8)}</strong><small>{assignment.preparedText ?? "Chưa chuẩn bị"}</small></span>
                      {shot && (
                        <button type="button" className="ghost" onClick={() => void showShot(shot.id)}>
                          Ảnh
                        </button>
                      )}
                      <span className={`chip ${assignment.state === "succeeded" ? "ok" : assignment.state === "uncertain" ? "warn" : "info"}`}>{stateLabel(assignment.state)}</span>
                    </div>
                  );
                })}
                {preview && (
                  <button type="button" className="interaction-shot" onClick={() => setPreview(null)}>
                    <img src={preview} alt="Ảnh màn hình khay bình luận" />
                  </button>
                )}
              </div>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
