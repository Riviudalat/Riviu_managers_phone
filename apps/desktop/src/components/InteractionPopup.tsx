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
  const inScope = useMemo(
    () => devices.filter((device) => (selected.length ? selected.includes(device.udid) : true)),
    [devices, selected],
  );
  // Android is a first-class actor now: it drives the comment drawer through the
  // accessibility hierarchy instead of by pixel matching, so nothing is filtered out
  // here any more. What replaces the filter is a *grouping*, because the two readers
  // cannot be mixed inside one nested thread — see `mixedThread` below.
  const actorChoices = inScope;
  const pixelActors = useMemo(
    () => inScope.filter((device) => device.platform === "ios"),
    [inScope],
  );
  const hierarchyActors = useMemo(
    () => inScope.filter((device) => device.platform === "android"),
    [inScope],
  );
  const [tab, setTab] = useState<"setup" | "monitor">("setup");
  const [rawLinks, setRawLinks] = useState("");
  const [lines, setLines] = useState<TikTokLinkLine[]>([]);
  const [actors, setActors] = useState<string[]>([]);
  const [messageCount, setMessageCount] = useState(2);
  const [maxWords, setMaxWords] = useState(12);
  const [mode, setMode] = useState<ThreadMode>("threaded");
  const [instruction, setInstruction] = useState("tự nhiên, ngắn, nói như người vừa xem xong");
  // "ai" | "manual" — which writes the comments. Kept as a mode rather than inferred from
  // whether the box has text, so switching back to AI does not mean deleting what was pasted.
  const [textSource, setTextSource] = useState<"ai" | "manual">("ai");
  const [manualText, setManualText] = useState("");
  const [likeTarget, setLikeTarget] = useState(false);
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
  /**
   * One comment per non-empty line, in the order they were pasted.
   *
   * Blank lines are dropped rather than sent: the backend refuses an empty comment, and a
   * trailing newline in a pasted block is not the operator asking for one.
   */
  const manualComments = useMemo(
    () =>
      textSource === "manual"
        ? manualText
            .split(/\r?\n/)
            .map((line) => line.trim())
            .filter((line) => line.length > 0)
        : [],
    [manualText, textSource],
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
      manualComments,
      likeTarget,
    }),
    [actors, instruction, likeTarget, manualComments, maxWords, messageCount, mode, validTargets],
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
    // Pre-select from ONE group, never across both: a default selection that is already
    // invalid for Threaded would make the operator undo the app's own choice before they
    // could start. The larger group wins so the default covers as much of the fleet as
    // one thread can.
    const group = hierarchyActors.length > pixelActors.length ? hierarchyActors : pixelActors;
    setActors(group.slice(0, 6).map((device) => device.udid));
  }, [hierarchyActors, pixelActors, reloadCampaigns]);

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

  // A nested thread is a linear chain in which each message is sent from a *different*
  // actor, so message N has to find message N-1's comment on screen. A hierarchy actor
  // stores an author label read out of a node's `text`; a pixel actor then has to re-find
  // that row by OCR and match the label, and the two do not have to agree — a badge, a
  // truncation, a rendered-versus-attribute difference. Standalone has no parent to find,
  // so mixing is fine there.
  const mixedThread =
    mode === "threaded" &&
    actors.some((udid) => pixelActors.some((device) => device.udid === udid)) &&
    actors.some((udid) => hierarchyActors.some((device) => device.udid === udid));
  const mixedThreadReason =
    "Chuỗi lồng nhau không chạy trộn iPhone với Android: hai bên đọc nhãn tác giả theo hai " +
    "cách nên mắt xích có thể đứt giữa chừng. Chọn toàn iPhone, toàn Android, hoặc chuyển " +
    "sang Standalone.";

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
    if (mixedThread) {
      // The server refuses this too (`require_parent_locator` -> `MixedPlatformThread`),
      // which is the real gate. This only saves a round trip and states the reason where
      // the operator is already looking.
      setError(mixedThreadReason);
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
                ? "Tạo hội thoại lồng nhau. Actor Android tìm lại bình luận cha trong hierarchy; actor iPhone cần OCR đọc được tiếng Việt, hiện chỉ có trên macOS. Không trộn hai loại trong một chuỗi."
                : "Mỗi acc để một bình luận riêng, không lồng nhau. Không cần OCR, chạy được trên mọi máy, và trộn iPhone với Android cũng được."}
            </p>
            <label className="check">
              <input
                type="checkbox"
                checked={likeTarget}
                onChange={(e) => setLikeTarget(e.target.checked)}
              />
              Thả tim bài
            </label>
            <p className="hint">
              Mỗi actor thả tim bài trước khi bình luận, xác nhận bằng nhãn nút tim đổi trạng
              thái. Máy Android làm được; actor iPhone sẽ bị từ chối vì chưa đo toạ độ nút tim
              trên trang bài — và một lần thả tim thất bại không làm mất bình luận.
            </p>
            <label>
              Nội dung bình luận
              <select
                value={textSource}
                onChange={(e) => setTextSource(e.target.value as "ai" | "manual")}
              >
                <option value="ai">AI viết — đọc nội dung bài rồi tự viết</option>
                <option value="manual">Thủ công — dán sẵn danh sách bình luận</option>
              </select>
            </label>
            {textSource === "ai" ? (
              <label>
                Giọng điệu / hướng dẫn
                <input value={instruction} onChange={(e) => setInstruction(e.target.value)} />
              </label>
            ) : (
              <>
                <label>
                  Danh sách bình luận — mỗi dòng một câu
                  <textarea
                    rows={6}
                    value={manualText}
                    placeholder={["đẹp quá", "chỗ này ở đâu vậy ạ", "lưu lại đi ăn thử"].join(
                      "\n",
                    )}
                    onChange={(e) => setManualText(e.target.value)}
                  />
                </label>
                <p className="hint">
                  {manualComments.length} câu · cần ít nhất {messageCount} câu cho {messageCount}{" "}
                  message. Chia lần lượt theo từng link nên mười link không mở đầu bằng cùng một
                  câu, và cùng một chiến dịch chạy lại sẽ gửi đúng chữ đó.
                </p>
              </>
            )}
            <fieldset className="interaction-actors">
              <legend>Actor tham gia</legend>
              {actorChoices.length === 0 && <span className="hint">Chưa có thiết bị</span>}
              {/* Grouped by *how each device reads the screen*, not by brand: that is the
                  property the thread rule depends on, and naming it here is what makes the
                  refusal below make sense instead of looking arbitrary. */}
              {[
                { label: "iPhone (nhận dạng ảnh)", group: pixelActors },
                { label: "Android (hierarchy)", group: hierarchyActors },
              ]
                .filter((section) => section.group.length > 0)
                .map((section) => (
                  <div key={section.label} className="interaction-actor-group">
                    <span className="hint">{section.label}</span>
                    {section.group.map((device) => (
                      <label key={device.udid}>
                        <input
                          type="checkbox"
                          checked={actors.includes(device.udid)}
                          onChange={() => setActors((prev) => (prev.includes(device.udid) ? prev.filter((id) => id !== device.udid) : [...prev, device.udid]))}
                        />
                        <span>{device.name || device.udid.slice(0, 8)}</span>
                      </label>
                    ))}
                  </div>
                ))}
              {mixedThread && <p className="error">{mixedThreadReason}</p>}
            </fieldset>
            <button
              type="button"
              className="primary"
              disabled={busy || mixedThread}
              title={mixedThread ? mixedThreadReason : undefined}
              onClick={() => void run()}
            >
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
