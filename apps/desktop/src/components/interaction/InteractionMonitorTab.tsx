import { useCallback, useEffect, useRef, useState } from "react";
import {
  interactionCancel,
  interactionGet,
  interactionList,
  interactionListArtifacts,
  interactionReadArtifact,
  interactionRetry,
  listenRiviuEvents,
  type InteractionArtifactRecord,
} from "../../api";
import { describeError } from "../../describeError";
import { campaignStateVi, interactionErrorVi, stateTone } from "../../interactionErrors";
import { timeAgoVi } from "../../timeAgo";
import { ProgressBar } from "../ProgressBar";
import { Banner, EmptyState } from "../States";
import { InteractionCampaignDetailView } from "./InteractionCampaignDetail";
import type {
  DeviceInfo,
  InteractionCampaignDetail,
  InteractionCampaignSummary,
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
  const [detail, setDetail] = useState<InteractionCampaignDetail | null>(null);
  const [artifacts, setArtifacts] = useState<InteractionArtifactRecord[]>([]);
  const [shot, setShot] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reloadCampaigns = useCallback(async () => {
    try {
      setCampaigns(await interactionList());
    } catch (e) {
      setError(describeError(e));
    }
  }, []);

  const loadDetail = useCallback(async (campaignId: string) => {
    try {
      setDetail(await interactionGet(campaignId));
      // Saved frames are what makes a campaign result checkable rather than just asserted; a
      // campaign that has none still opens.
      setArtifacts(await interactionListArtifacts(campaignId).catch(() => []));
    } catch (e) {
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

  /// Which campaign is on screen, for the event handler to read.
  ///
  /// A ref rather than a dependency: keying the subscription on the open id made it tear down
  /// and re-subscribe on every navigation, and `listen` is a promise — an unmount before it
  /// resolved left the listener attached with nothing to unsubscribe it.
  const openRef = useRef<string | null>(openCampaignId);
  openRef.current = openCampaignId;

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

  if (detail) {
    return (
      <div className="interaction-body">
        <InteractionCampaignDetailView
          detail={detail}
          artifacts={artifacts}
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
      <div className="interaction-campaign-list">
        {campaigns.map((campaign) => {
          const total = campaign.messageCount * campaign.targetCount;
          const settled = campaign.succeededMessages + campaign.failedMessages;
          // The brief is what makes one row tell itself apart from the next. Before it, the
          // only name a row had was fourteen characters of a UUID.
          const title = campaign.brief?.firstAuthor
            ? `@${campaign.brief.firstAuthor}${
                campaign.targetCount > 1 ? ` +${campaign.targetCount - 1} link` : ""
              }`
            : `${campaign.targetCount} link`;
          return (
            <button
              type="button"
              key={campaign.id}
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
                  {campaign.succeededMessages}/{total} bình luận
                  {campaign.failedMessages > 0 && ` · ${campaign.failedMessages} lỗi`}
                  {campaign.updatedAt && ` · ${timeAgoVi(campaign.updatedAt)}`}
                </small>
                <ProgressBar
                  fraction={total > 0 ? settled / total : null}
                  failedFraction={total > 0 ? campaign.failedMessages / total : 0}
                  tone={
                    campaign.state === "running"
                      ? "run"
                      : stateTone(campaign.state) === "ok"
                        ? "done"
                        : "failed"
                  }
                  label={`Tiến trình ${title}`}
                />
                {campaign.errorCode && (
                  <small className="interaction-error" title={campaign.errorCode}>
                    {interactionErrorVi(campaign.errorCode).title}
                  </small>
                )}
              </span>
            </button>
          );
        })}
        {!campaigns.length && (
          <EmptyState compact title="Chưa có chiến dịch nào" />
        )}
      </div>
    </div>
  );
}
