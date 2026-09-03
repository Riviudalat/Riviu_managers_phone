import { useCallback, useEffect, useRef, useState } from "react";
import {
  interactionCancel,
  interactionGet,
  interactionList,
  interactionListArtifacts,
  interactionListTargetNotes,
  interactionReadArtifact,
  interactionRetry,
  listenRiviuEvents,
  type InteractionArtifactRecord,
} from "../../api";
import { describeError } from "../../describeError";
import { campaignStateVi, interactionErrorVi, stateTone } from "../../interactionErrors";
import { timeAgoVi } from "../../timeAgo";
import { useTickWhile } from "../../useTickWhile";
import { ProgressBar } from "../ProgressBar";
import { Banner, EmptyState, LoadingState } from "../States";
import { InteractionCampaignDetailView } from "./InteractionCampaignDetail";
import type {
  DeviceInfo,
  InteractionCampaignDetail,
  InteractionCampaignSummary,
  InteractionTargetNote,
} from "../../types";

/**
 * What is running and what already ran.
 *
 * Two things about the shape. Opening a campaign **replaces** the list rather than appearing
 * under it: a fourteen-phone run is fourteen rows plus a heading, and in a 460px card the
 * list and the detail were fighting over the same screen. And the live subscription lives
 * here rather than in the popup, because this tab is the only thing that reacts to it — the
 * old effect re-subscribed to the global event stream every time a campaign was opened, and
 * could leak the listener when it unmounted before `listen` resolved.
 */
/// How long relative wording can still change, so how long the clock is worth ticking.
///
/// `timeAgoVi` switches to an absolute time past an hour, and everything before that — "vừa
/// xong", "3 phút trước" — goes stale on its own. One hour is therefore the whole window in
/// which a re-render buys anything.
const RELATIVE_WORDING_WINDOW_MS = 60 * 60 * 1000;

export function InteractionMonitorTab({
  devices,
  deviceNumber,
  handles,
  openCampaignId,
  onOpenCampaign,
}: {
  devices: DeviceInfo[];
  deviceNumber: Map<string, number>;
  handles: Record<string, string>;
  openCampaignId: string | null;
  onOpenCampaign: (id: string | null) => void;
}) {
  const [campaigns, setCampaigns] = useState<InteractionCampaignSummary[]>([]);
  const [campaignLoadState, setCampaignLoadState] = useState<"loading" | "ready" | "error">(
    "loading",
  );
  const [campaignLoadError, setCampaignLoadError] = useState<string | null>(null);
  const [detail, setDetail] = useState<InteractionCampaignDetail | null>(null);
  const [artifacts, setArtifacts] = useState<InteractionArtifactRecord[]>([]);
  const [notes, setNotes] = useState<InteractionTargetNote[]>([]);
  const [shot, setShot] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reloadCampaigns = useCallback(async () => {
    setCampaignLoadState((current) => (current === "ready" ? current : "loading"));
    setCampaignLoadError(null);
    try {
      setCampaigns(await interactionList());
      setCampaignLoadState("ready");
      // Cleared on success. Only `guard()` ever reset this, so one transient failure pinned a
      // red banner over a panel that had been working again for an hour.
      setError(null);
    } catch (e) {
      setCampaignLoadError(describeError(e));
      setCampaignLoadState("error");
    }
  }, []);

  /// Which campaign is on screen, for the event handler and the staleness guard to read.
  ///
  /// A ref rather than a dependency: keying the subscription on the open id made it tear down
  /// and re-subscribe on every navigation, and `listen` is a promise — an unmount before it
  /// resolved left the listener attached with nothing to unsubscribe it. Written in an effect
  /// rather than during render, because a render React throws away must not leave a mutation
  /// behind.
  const openRef = useRef<string | null>(openCampaignId);
  useEffect(() => {
    openRef.current = openCampaignId;
  }, [openCampaignId]);

  const loadDetail = useCallback(async (campaignId: string) => {
    try {
      const loaded = await interactionGet(campaignId);
      // Saved frames are what makes a campaign result checkable rather than just asserted; a
      // campaign that has none still opens.
      const frames = await interactionListArtifacts(campaignId).catch(() => []);
      // Same treatment as the frames: a campaign whose targets were never looked up still
      // opens, and the panel says "chưa tra" rather than showing nothing at all.
      const targetNotes = await interactionListTargetNotes(campaignId).catch(() => []);
      // **Dropped if the operator has moved on.** Two clicks — a slow campaign then a fast one
      // — used to settle out of order and leave B open while A was on screen, and then Dừng
      // cancelled A. Cancelling the wrong live campaign is not recoverable, and the same hole
      // was open on the event path, where every `interactionUpdated` fired an unsequenced load.
      if (openRef.current !== campaignId) return;
      setDetail(loaded);
      setArtifacts(frames);
      setNotes(targetNotes);
      setError(null);
    } catch (e) {
      if (openRef.current !== campaignId) return;
      setError(describeError(e));
    }
  }, []);

  useEffect(() => {
    void reloadCampaigns();
  }, [reloadCampaigns]);

  useEffect(() => {
    if (!openCampaignId) {
      setDetail(null);
      setArtifacts([]);
      setShot(null);
      return;
    }
    void loadDetail(openCampaignId);
  }, [openCampaignId, loadDetail]);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void listenRiviuEvents((event) => {
      if (event.type !== "interactionUpdated") return;
      void reloadCampaigns();
      if (event.campaignId && event.campaignId === openRef.current) {
        // Artifacts too. They were fetched once when the campaign was opened and never again,
        // so an evidence frame saved mid-run only appeared after closing and reopening it.
        void loadDetail(event.campaignId);
      }
    }).then((fn) => {
      if (!alive) {
        fn();
        return;
      }
      unlisten = fn;
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [reloadCampaigns, loadDetail]);

  /// Keep the relative times honest, without spinning for ever.
  ///
  /// `timeAgoVi` is evaluated at render and this list only re-renders when an
  /// `interactionUpdated` arrives — so on an idle fleet a run that finished an hour ago still
  /// read "vừa xong". `useTickWhile` was written for exactly this and `NurturePopup` uses it;
  /// this panel had duplicated the problem instead of the solution.
  ///
  /// Ticking while anything is running **or** while the newest row is still inside the hour
  /// that relative wording can change in. After that the wording is stable, so the timer
  /// stops rather than re-rendering a finished panel once a second for ever.
  const newest = campaigns.reduce(
    (max, campaign) =>
      Math.max(max, campaign.updatedAt ? Date.parse(campaign.updatedAt) || 0 : 0),
    0,
  );
  const ticking =
    campaigns.some(
      (campaign) => campaign.state === "running" || campaign.state === "queued",
    ) ||
    (newest > 0 && Date.now() - newest < RELATIVE_WORDING_WINDOW_MS);
  useTickWhile(ticking);

  const guard = useCallback(async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  }, []);

  // **Driven by the open id, not by `detail`.** With `detail` in charge, a load that
  // resolved after Back re-showed the panel, and the second Back was a no-op on an already
  // null id — so the effect never ran again and the detail view could not be left at all.
  if (openCampaignId && (!detail || detail.summary.id !== openCampaignId)) {
    return (
      <div className="interaction-body">
        <button
          type="button"
          className="interaction-back"
          onClick={() => onOpenCampaign(null)}
        >
          ← Chiến dịch gần đây
        </button>
        {error && <Banner tone="error">{error}</Banner>}
        <EmptyState compact title="Đang mở chiến dịch…" />
      </div>
    );
  }

  if (openCampaignId && detail) {
    return (
      <div className="interaction-body">
        <InteractionCampaignDetailView
          detail={detail}
          artifacts={artifacts}
          notes={notes}
          devices={devices}
          deviceNumber={deviceNumber}
          handles={handles}
          busy={busy}
          error={error}
          onBack={() => onOpenCampaign(null)}
          // Awaited, with a busy state and a caught failure. It used to be fire-and-forget, so
          // a cancel the backend refused looked exactly like one it accepted.
          onCancel={() =>
            void guard(async () => {
              await interactionCancel(detail.summary.id);
              await loadDetail(detail.summary.id);
              await reloadCampaigns();
            })
          }
          onRetry={(assignmentIds) =>
            void guard(async () => {
              await interactionRetry(detail.summary.id, assignmentIds);
              await loadDetail(detail.summary.id);
              await reloadCampaigns();
            })
          }
          onShowShot={(artifactId) =>
            void guard(async () => {
              const payload = await interactionReadArtifact(artifactId);
              setShot(`data:${payload.mimeType};base64,${payload.base64}`);
            })
          }
          shot={shot}
          onDismissShot={() => setShot(null)}
        />
      </div>
    );
  }

  return (
    <div className="interaction-body">
      <div className="interaction-monitor-head">
        <strong>Chiến dịch gần đây</strong>
        <button type="button" className="ghost" onClick={() => void reloadCampaigns()}>
          Làm mới
        </button>
      </div>
      {error && <Banner tone="error">{error}</Banner>}
      {campaignLoadState === "loading" && <LoadingState label="Đang tải chiến dịch…" />}
      {campaignLoadState === "error" && (
        <Banner
          tone="error"
          action={(
            <button type="button" className="ghost" onClick={() => void reloadCampaigns()}>
              Thử lại
            </button>
          )}
        >
          {campaignLoadError ?? "Không tải được chiến dịch."}
        </Banner>
      )}
      {campaignLoadState === "ready" && <div className="interaction-campaign-list">
        {campaigns.map((campaign) => {
          const total = campaign.messageCount * campaign.targetCount;
          const settled = campaign.succeededMessages + campaign.failedMessages;
          const actionCounters = campaign.actionCounters;
          const hasActionCounters = Boolean(actionCounters?.planned);
          const actionSettled = actionCounters
            ? actionCounters.confirmed + actionCounters.noOp + actionCounters.uncertain
            : 0;
          // The brief is what makes one row tell itself apart from the next. Before it, the
          // only name a row had was fourteen characters of a UUID.
          const title = campaign.brief?.firstAuthor
            ? `@${campaign.brief.firstAuthor}${
                campaign.targetCount > 1 ? ` +${campaign.targetCount - 1} link` : ""
              }`
            : `${campaign.targetCount} link`;
          return (
            // The row is the box; the button is the clickable part of it and the bar sits
            // under it as a sibling. `role="progressbar"` is a `<div>`, and `<button>`'s
            // content model is phrasing content — so the bar was invalid there, and worse, its
            // `aria-label` was folded into the button's name-from-content, giving one
            // ninety-character name per row.
            <div className="interaction-campaign-row" key={campaign.id}>
              <button
                type="button"
                className="interaction-campaign"
                onClick={() => onOpenCampaign(campaign.id)}
              >
                <span className="grow">
                  <span className="interaction-campaign-head">
                    <strong>{title}</strong>
                    <span className={`chip ${stateTone(campaign.state)}`}>
                      {campaignStateVi(campaign.state)}
                    </span>
                  </span>
                  <small>
                    {hasActionCounters
                      ? `${actionSettled}/${actionCounters!.planned} hành động · ${actionCounters!.confirmed} xác nhận · ${actionCounters!.noOp} không cần làm${actionCounters!.uncertain > 0 ? ` · ${actionCounters!.uncertain} chưa chắc` : ""}`
                      : `${campaign.succeededMessages}/${total} bình luận`}
                    {campaign.failedMessages > 0 && ` · ${campaign.failedMessages} lỗi`}
                    {campaign.updatedAt && ` · ${timeAgoVi(campaign.updatedAt)}`}
                  </small>
                  {campaign.errorCode && (
                    <small className="interaction-error" title={campaign.errorCode}>
                      {interactionErrorVi(campaign.errorCode).title}
                    </small>
                  )}
                </span>
              </button>
              <ProgressBar
                fraction={
                  hasActionCounters
                    ? actionSettled / actionCounters!.planned
                    : total > 0
                      ? settled / total
                      : null
                }
                failedFraction={
                  hasActionCounters ? 0 : total > 0 ? campaign.failedMessages / total : 0
                }
                tone={
                  campaign.state === "running"
                    ? "run"
                    : stateTone(campaign.state) === "ok"
                      ? "done"
                      : "failed"
                }
                label={`Tiến trình ${title}`}
              />
            </div>
          );
        })}
        {!campaigns.length && (
          <EmptyState compact title="Chưa có chiến dịch nào" />
        )}
      </div>}
    </div>
  );
}
