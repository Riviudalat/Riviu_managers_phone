import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  getDeviceMeta,
  interactionParseLinks,
  interactionPreviewThread,
  interactionResolveLinks,
  interactionStartThread,
  saveDeviceMeta,
  interactionMeasurePost,
} from "../api";
import { describeError } from "../describeError";
import { orderDevicesByNumber, tileName, tileNumber } from "../deviceNaming";
import { parseMentions, resolveMentionActors, unionActors } from "../interactionMentions";
import {
  buildRequest,
  DEFAULT_DRAFT,
  draftWarnings,
  groupPlanByCohort,
  largestCohortOf,
  validateDraft,
  type InteractionDraft,
} from "../interactionPlan";
import { interactionProfileConfig } from "../automationProfileConfig";
import type {
  DeviceInfo,
  DeviceMeta,
  InteractionPostReading,
  PostTargets,
  TargetRef,
  ThreadPreview,
  TikTokLinkLine,
} from "../types";
import { IconChat, IconClose } from "./Icons";
import { InteractionMonitorTab } from "./interaction/InteractionMonitorTab";
import { InteractionSetupTab } from "./interaction/InteractionSetupTab";
import { AutomationProfileControl } from "./AutomationProfileControl";
import { CommandBar, StatusChip, SummaryRail, WorkflowStepper } from "./WorkspacePrimitives";

type Props = {
  devices: DeviceInfo[];
  selected: string[];
  /** Already resolved from All/Group/Explicit; an empty array remains an empty scope. */
  targetUdids?: string[];
  /** Semantic scope stored with an automation profile; groups resolve again at execution. */
  targetRef?: TargetRef;
  /**
   * The operator's own records for each phone — the name and the number they assigned.
   *
   * Passed in rather than fetched here so the panel labels phones exactly as the wall does:
   * one source for `tileName`/`tileNumber`, and a rename shows up in both at once.
   */
  metas: Map<string, DeviceMeta>;
  onClose?: () => void;
  surface?: "popup" | "page";
};

function newRequestId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `interaction-${Date.now()}`;
}

/**
 * The interaction panel: set a campaign up, then watch it.
 *
 * This file is the shell — the float, the two tabs, and the state a tab switch has to
 * survive. Everything with a shape of its own moved into `interaction/`, the way the nurture
 * panel was split: a 965-line component with eighteen `useState` had nowhere left to put a
 * rule, and the rules were the problem.
 */
/**
 * Keep at least this much of the card reachable while dragging, in pixels.
 *
 * The drag had no bounds at all: pulled far enough up and left, the card — including its close
 * button — left the viewport, and the only way back was the sidebar toggle, which unmounts the
 * popup and discards the whole draft. A strip this wide always leaves the header, and so the
 * drag handle itself, under the cursor.
 */
const DRAG_KEEP = 64;

export function InteractionPopup({
  devices,
  selected,
  targetUdids,
  targetRef,
  metas,
  onClose,
  surface = "popup",
}: Props) {
  const inScope = useMemo(
    () => devices.filter((device) =>
      targetUdids !== undefined
        ? targetUdids.includes(device.udid)
        : selected.length
          ? selected.includes(device.udid)
          : true,
    ),
    [devices, selected, targetUdids],
  );
  // **The number the operator wrote on the phone, not this list's index.**
  //
  // The comment here used to claim this was "the same 1-based number the grid stamps", and it
  // was not: it counted positions in `devices`, while the grid stamps
  // `tileNumber(position, meta)` over `orderDevicesByNumber(...)`. So the moment anyone used
  // Change Number — the whole point of which is that a number survives the list moving — the
  // picker and the wall disagreed, and "máy số 7" meant two different phones depending on
  // which one you were looking at. On a fleet of twenty identical SM-G950Fs that is the only
  // handle an operator has.
  const deviceNumber = useMemo(() => {
    const map = new Map<string, number>();
    orderDevicesByNumber(devices, metas).forEach((device, index) =>
      map.set(device.udid, tileNumber(index + 1, metas.get(device.udid))),
    );
    return map;
  }, [devices, metas]);
  /// What the operator calls each phone, or what the phone reports if they never renamed it.
  ///
  /// Same `tileName` the grid uses, so renaming a phone renames it here too.
  const deviceLabel = useMemo(() => {
    const map = new Map<string, string>();
    devices.forEach((device) => map.set(device.udid, tileName(device, metas.get(device.udid))));
    return map;
  }, [devices, metas]);
  // Android is a first-class actor: it drives the comment drawer through the accessibility
  // hierarchy instead of by pixel matching. The split is by *how each device reads the
  // screen*, because that is the property the thread rule depends on.
  const pixelActors = useMemo(
    () => inScope.filter((device) => device.platform === "ios"),
    [inScope],
  );
  const hierarchyActors = useMemo(
    () => inScope.filter((device) => device.platform === "android"),
    [inScope],
  );

  const [tab, setTab] = useState<"setup" | "monitor">("setup");
  const [draft, setDraft] = useState<InteractionDraft>(DEFAULT_DRAFT);
  /// Set one draft field, from a value or from the value it currently has.
  ///
  /// The updater form matters for the actor list. `patch("actors", draft.actors.filter(...))`
  /// looks functional because `patch` wraps it in `setDraft(previous => …)`, but the array
  /// handed in was computed from the *rendered* prop — so a departed-phone sweep landing between
  /// that render and the click would be undone by the toggle, resurrecting a phone that is no
  /// longer in the fleet.
  const patch = useCallback(
    <K extends keyof InteractionDraft>(
      key: K,
      value: InteractionDraft[K] | ((previous: InteractionDraft[K]) => InteractionDraft[K]),
    ) =>
      setDraft((previous) => ({
        ...previous,
        [key]:
          typeof value === "function"
            ? (value as (from: InteractionDraft[K]) => InteractionDraft[K])(previous[key])
            : value,
      })),
    [],
  );
  const [lines, setLines] = useState<TikTokLinkLine[]>([]);
  const [preview, setPreview] = useState<ThreadPreview | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [handles, setHandles] = useState<Record<string, string>>({});
  const [openCampaignId, setOpenCampaignId] = useState<string | null>(null);
  // Scoped, not one shared string. A link that would not parse and a dispatch that was
  // refused used to overwrite each other, so the message on screen was whichever failed last.
  const [linkBusy, setLinkBusy] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [runBusy, setRunBusy] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const pageSurface = surface === "page";

  /// One id per attempt, not one per keystroke.
  ///
  /// It used to be a `crypto.randomUUID()` inside the `useMemo` that built the request, so it
  /// changed on every character typed into any field. The backend treats it as the campaign's
  /// identity, and `request_id` is `UNIQUE`.
  ///
  /// Lazily, too: `useRef(newRequestId())` evaluates its argument on **every** render and
  /// discards the result — during a header drag that is one `crypto.randomUUID()` per
  /// `pointermove`.
  const requestIdRef = useRef<string>("");
  if (!requestIdRef.current) requestIdRef.current = newRequestId();

  /// Whether the Nâng cao disclosure is open — see `InteractionSetupTab`'s prop for why it
  /// lives up here rather than in the tab that draws it.
  const [advancedOpen, setAdvancedOpen] = useState(false);

  /// The numbers the operator wants the post to reach, and the last reading of it.
  ///
  /// Up here for the same reason as the disclosure: a look at Theo dõi must not throw away a
  /// reading that cost a phone two to four minutes.
  const [wanted, setWanted] = useState<PostTargets>({
    views: null,
    likes: null,
    comments: null,
  });
  const [readViews, setReadViews] = useState(false);
  const [reading, setReading] = useState<InteractionPostReading | null>(null);
  const [measureBusy, setMeasureBusy] = useState(false);
  const [measureError, setMeasureError] = useState<string | null>(null);

  const [pos, setPos] = useState({ x: 0, y: 0 });
  const drag = useRef<{ ox: number; oy: number; sx: number; sy: number } | null>(null);

  const validTargets = useMemo(
    () => lines.flatMap((line) => (line.target ? [line.target] : [])),
    [lines],
  );
  const badLineCount = useMemo(
    () => lines.filter((line) => !line.target).length,
    [lines],
  );

  const inScopeKey = useMemo(() => inScope.map((device) => device.udid).join(","), [inScope]);
  // @handles for the in-scope phones. The fleet poll rebuilds `inScope` every few seconds, so
  // keying the load on the udid list keeps it from refetching — and clobbering an unsaved
  // edit — on every poll. A locally-edited handle in `prev` wins over a reload.
  useEffect(() => {
    let alive = true;
    const udids = inScopeKey ? inScopeKey.split(",") : [];
    void Promise.all(
      udids.map((udid) =>
        getDeviceMeta(udid)
          .then((meta) => [udid, meta.handle ?? ""] as const)
          .catch(() => [udid, ""] as const),
      ),
    ).then((pairs) => {
      if (alive) setHandles((prev) => ({ ...Object.fromEntries(pairs), ...prev }));
    });
    return () => {
      alive = false;
    };
  }, [inScopeKey]);

  const persistHandle = useCallback(async (udid: string, value: string) => {
    const handle = value.trim().replace(/^@+/, "");
    setHandles((prev) => ({ ...prev, [udid]: handle }));
    try {
      // Round-trip the full meta so notes/tags/group/proxy are preserved, not wiped.
      const meta = await getDeviceMeta(udid);
      await saveDeviceMeta({ ...meta, handle });
    } catch {
      // Non-fatal: the tag still resolves from local state for this session.
    }
  }, []);

  const mentions = useMemo(
    () => (draft.actions.comment ? parseMentions(draft.mentionText) : []),
    [draft.actions.comment, draft.mentionText],
  );
  /// The phones a tag names, by matching each tag to a phone's @handle. These join the actor
  /// set so the tagged account comments on the post itself.
  const mentionActors = useMemo(() => {
    const udids = inScopeKey ? inScopeKey.split(",") : [];
    return resolveMentionActors(
      mentions,
      udids.map((udid) => ({ udid, handle: handles[udid] ?? "" })),
    );
  }, [mentions, inScopeKey, handles]);
  const effectiveActors = useMemo(
    () => unionActors(draft.actors, mentionActors),
    [draft.actors, mentionActors],
  );

  /// Seed the default **once**, and never again.
  ///
  /// This used to depend on the actor lists, which are memos over `devices` — a fresh array
  /// every three seconds from the fleet poll — so it threw away whatever the operator had just
  /// chosen. Selecting actors was a race against the next tick, and nobody wins that.
  ///
  /// Pre-selects from ONE group, never across both: a default already invalid for a thread
  /// would make the operator undo the app's own choice before they could start.
  const seededActors = useRef(false);
  useEffect(() => {
    if (seededActors.current) return;
    if (!hierarchyActors.length && !pixelActors.length) return;
    seededActors.current = true;
    const group = hierarchyActors.length > pixelActors.length ? hierarchyActors : pixelActors;
    setDraft((previous) => ({
      ...previous,
      actors: group.map((device) => device.udid),
    }));
  }, [hierarchyActors, pixelActors]);

  /// A phone that has left the fleet drops out of the selection — **and comes back selected.**
  ///
  /// The two effects were asymmetric: this one removed, and the seeding one above is guarded by
  /// `seededActors` so it never restored. Clicking a single different phone on the wall narrows
  /// `selected`, and so `inScope`, and so stripped the actor list — then re-selecting the three
  /// phones on the wall did *not* bring them back, and the operator had to re-tick every tile by
  /// hand. A momentary fleet-poll blip that returned a short device list did the same.
  ///
  /// So departure is remembered rather than acted on once. Deliberately unticking a phone that
  /// is present records nothing, because that phone never left.
  ///
  /// Returning early when nothing moved is what keeps this from being the old bug in a new
  /// shape: a new array on every poll re-renders forever.
  const departed = useRef<string[]>([]);
  useEffect(() => {
    const here = (udid: string) => inScope.some((device) => device.udid === udid);
    const gone = draft.actors.filter((udid) => !here(udid));
    const back = departed.current.filter(here);
    if (!gone.length && !back.length) return;
    departed.current = [
      ...new Set([...departed.current.filter((udid) => !here(udid)), ...gone]),
    ];
    setDraft((previous) => ({
      ...previous,
      actors: [...previous.actors.filter(here), ...back],
    }));
  }, [inScope, draft.actors]);

  /// Cancels the debounced parse below.
  ///
  /// `interaction_resolve_links` is `async` while `interaction_parse_links` is not, so pressing
  /// Gỡ link rút gọn with a parse timer still pending let the parse land *after* the resolve and
  /// silently throw the resolved links away — the operator pressed the button, watched the list
  /// change, and then watched it change back.
  const cancelParse = useRef<() => void>(() => undefined);
  const linkRequestGeneration = useRef(0);

  // Debounced: this used to fire one IPC round trip per keystroke.
  useEffect(() => {
    const generation = ++linkRequestGeneration.current;
    setLinkBusy(false);
    const raw = draft.rawLinks;
    if (!raw.trim()) {
      setLines([]);
      // Cleared here too. Paste something the parser rejects, then select-all and delete: the
      // list emptied and the red banner stayed on screen for the rest of the session, because
      // this branch returned before touching it.
      setLinkError(null);
      return;
    }
    let live = true;
    const timer = setTimeout(() => {
      void interactionParseLinks(raw)
        .then((next) => {
          if (!live || linkRequestGeneration.current !== generation) return;
          setLines(next);
          setLinkError(null);
        })
        .catch((e) => {
          if (!live || linkRequestGeneration.current !== generation) return;
          setLinkError(describeError(e));
        });
    }, 300);
    cancelParse.current = () => {
      live = false;
      clearTimeout(timer);
    };
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [draft.rawLinks]);

  /// What the plan on screen was computed for.
  ///
  /// Everything the partition depends on, and nothing else — a changed instruction or word
  /// limit does not move the cohorts, so it must not disable the run button either.
  const previewKey = useMemo(
    () =>
      JSON.stringify([validTargets.map((target) => target.targetKey), effectiveActors]),
    [validTargets, effectiveActors],
  );
  const [previewFor, setPreviewFor] = useState<string | null>(null);
  const previewGeneration = useRef(0);
  const previewStale =
    draft.actions.comment &&
    validTargets.length > 0 &&
    effectiveActors.length >= 2 &&
    previewFor !== previewKey;

  const cohorts = useMemo(() => groupPlanByCohort(preview?.plan), [preview]);
  const largestCohort = largestCohortOf(cohorts, effectiveActors.length);

  /// Ask the real planner what this draft would do.
  ///
  /// Debounced and best-effort: a refusal becomes a validation reason rather than an error
  /// banner, because `plan_threads` runs the same `validate()` the dispatch will and its
  /// complaint is about the form, not about the request having failed.
  useEffect(() => {
    const generation = ++previewGeneration.current;
    if (!draft.actions.comment) {
      setPreview(null);
      setPlanError(null);
      setPreviewFor(previewKey);
      return;
    }
    if (validTargets.length === 0 || effectiveActors.length < 2) {
      setPreview(null);
      setPlanError(null);
      return;
    }
    const key = previewKey;
    let live = true;
    const timer = setTimeout(() => {
      void interactionPreviewThread(
        buildRequest(draft, {
          requestId: requestIdRef.current,
          targets: validTargets,
          actorUdids: effectiveActors,
          mentions,
          // The fleet count, not the derived one: this is the request that *discovers* the
          // cohort size, so it cannot be built from it. Safe because the planner only requires
          // `messageCount >= largest cohort`, and the fleet is never smaller than a cohort.
          largestCohort: effectiveActors.length,
          purpose: "preview",
        }),
      )
        .then((next) => {
          if (!live || previewGeneration.current !== generation) return;
          setPreview(next);
          setPreviewFor(key);
          setPlanError(null);
        })
        .catch((e) => {
          if (!live || previewGeneration.current !== generation) return;
          setPreview(null);
          setPreviewFor(key);
          setPlanError(describeError(e));
        });
    }, 350);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [draft, validTargets, effectiveActors, mentions, previewKey]);

  const mixedThread =
    effectiveActors.some((udid) => pixelActors.some((device) => device.udid === udid)) &&
    effectiveActors.some((udid) => hierarchyActors.some((device) => device.udid === udid));

  const validationContext = useMemo(
    () => ({
      targets: validTargets,
      actorUdids: effectiveActors,
      largestCohort,
      badLineCount,
      mixedThread,
      planError,
      previewStale,
    }),
    [
      validTargets,
      effectiveActors,
      largestCohort,
      badLineCount,
      mixedThread,
      planError,
      previewStale,
    ],
  );
  const issues = useMemo(
    () => validateDraft(draft, validationContext),
    [draft, validationContext],
  );
  // Advice rather than refusals: these never disable the run button.
  const warnings = useMemo(
    () => draftWarnings(draft, validationContext),
    [draft, validationContext],
  );
  const profileConfig = useMemo(
    () =>
      interactionProfileConfig(
        buildRequest(draft, {
          requestId: requestIdRef.current,
          targets: validTargets,
          actorUdids: effectiveActors,
          mentions,
          largestCohort,
        }),
      ),
    [draft, effectiveActors, largestCohort, mentions, validTargets],
  );

  const resolveShortLinks = useCallback(async () => {
    if (!draft.rawLinks.trim()) return;
    // Drop any parse still in flight for the same text; see `cancelParse`.
    cancelParse.current();
    const generation = ++linkRequestGeneration.current;
    setLinkBusy(true);
    try {
      const next = await interactionResolveLinks(draft.rawLinks);
      if (linkRequestGeneration.current !== generation) return;
      setLines(next);
      setLinkError(null);
    } catch (e) {
      if (linkRequestGeneration.current !== generation) return;
      setLinkError(describeError(e));
    } finally {
      if (linkRequestGeneration.current === generation) setLinkBusy(false);
    }
  }, [draft.rawLinks]);

  /// Read the first link on the first selected phone.
  ///
  /// The first of each rather than all of them: a reading is a property of the **post**, so one
  /// phone answers it, and taking more than one lease to learn the same number would be a cost
  /// with nothing behind it. `effectiveActors.length` goes in as the fleet size, which is what
  /// bounds a like target.
  const measure = useCallback(async () => {
    const target = validTargets[0];
    const udid = effectiveActors[0];
    if (!target || !udid) return;
    setMeasureBusy(true);
    setMeasureError(null);
    try {
      setReading(
        await interactionMeasurePost(
          udid,
          target,
          wanted,
          effectiveActors.length,
          readViews,
        ),
      );
    } catch (e) {
      // The old reading is dropped rather than left on screen next to a fresh error: it
      // describes a post as it was before something went wrong, and which of the two the
      // operator would believe is not a question worth creating.
      setReading(null);
      setMeasureError(describeError(e));
    } finally {
      setMeasureBusy(false);
    }
  }, [validTargets, effectiveActors, wanted, readViews]);

  const run = useCallback(async () => {
    if (issues.length) return;
    setRunBusy(true);
    setRunError(null);
    try {
      const result = await interactionStartThread(
        buildRequest(draft, {
          requestId: requestIdRef.current,
          targets: validTargets,
          actorUdids: effectiveActors,
          mentions,
          largestCohort,
        }),
      );
      // The started campaign was thrown away here, so the operator landed on a list of UUID
      // fragments and had to find their own run. Open it instead.
      requestIdRef.current = newRequestId();
      setOpenCampaignId(result.campaign.id);
      setTab("monitor");
    } catch (e) {
      setRunError(describeError(e));
    } finally {
      setRunBusy(false);
    }
  }, [draft, effectiveActors, issues.length, largestCohort, mentions, validTargets]);

  const onTitleDown = (event: React.PointerEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).closest("button")) return;
    drag.current = { ox: event.clientX, oy: event.clientY, sx: pos.x, sy: pos.y };
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onTitleMove = (event: React.PointerEvent<HTMLElement>) => {
    if (!drag.current) return;
    const card = event.currentTarget.parentElement?.getBoundingClientRect();
    const clamp = (value: number, min: number, max: number) =>
      Math.min(Math.max(value, min), max);
    const wanted = {
      x: drag.current.sx + (event.clientX - drag.current.ox),
      y: drag.current.sy + (event.clientY - drag.current.oy),
    };
    setPos({
      x: card
        ? clamp(
            wanted.x,
            wanted.x - (card.right - DRAG_KEEP),
            wanted.x + (window.innerWidth - DRAG_KEEP - card.left),
          )
        : wanted.x,
      y: card
        ? clamp(
            wanted.y,
            wanted.y - (card.bottom - DRAG_KEEP),
            wanted.y + (window.innerHeight - DRAG_KEEP - card.top),
          )
        : wanted.y,
    });
  };
  const onTitleUp = () => {
    drag.current = null;
  };

  const onTabKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    let next: "setup" | "monitor" | null = null;
    if (event.key === "ArrowRight" || event.key === "End") next = "monitor";
    if (event.key === "ArrowLeft" || event.key === "Home") next = "setup";
    if (!next) return;
    event.preventDefault();
    document.getElementById(`interaction-tab-${next}`)?.focus();
    setTab(next);
  };

  return (
    <div
      className={pageSurface ? "interaction-workspace" : "interaction-float-layer"}
      role={pageSurface ? "region" : undefined}
      aria-label={pageSurface ? "Không gian Tương tác" : undefined}
    >
      <section
        className={pageSurface ? "interaction-workspace-inner" : "interaction-float"}
        aria-label={pageSurface ? undefined : "Tương tác comment"}
        style={pageSurface ? undefined : { transform: `translate(${pos.x}px, ${pos.y}px)` }}
      >
        {!pageSurface && (
          <header
            className="interaction-title"
            onPointerDown={onTitleDown}
            onPointerMove={onTitleMove}
            onPointerUp={onTitleUp}
            onPointerCancel={onTitleUp}
            onLostPointerCapture={onTitleUp}
          >
            <IconChat size={15} />
            <strong>Tương tác</strong>
            <span className="hint">{inScope.length} thiết bị</span>
            <div className="grow" />
            <button type="button" className="close" title="Đóng" onClick={onClose}>
              <IconClose size={14} />
            </button>
          </header>
        )}
        {pageSurface && (
          <WorkflowStepper
            label="Quy trình Tương tác"
            current={
              tab === "monitor"
                ? "monitor"
                : !effectiveActors.length || !validTargets.length
                  ? "scope"
                  : issues.length
                    ? "actions"
                    : "review"
            }
            steps={[
              { id: "scope", label: "Phạm vi" },
              { id: "actions", label: "Hành động" },
              { id: "review", label: "Kiểm tra" },
              { id: "monitor", label: "Theo dõi" },
            ]}
          />
        )}
        <div className="interaction-tabs" role="tablist" aria-label="Chế độ Tương tác">
          <button
            type="button"
            role="tab"
            id="interaction-tab-setup"
            aria-controls="interaction-panel-setup"
            aria-selected={tab === "setup"}
            tabIndex={tab === "setup" ? 0 : -1}
            onClick={() => setTab("setup")}
            onKeyDown={onTabKeyDown}
          >
            Thiết lập
          </button>
          <button
            type="button"
            role="tab"
            id="interaction-tab-monitor"
            aria-controls="interaction-panel-monitor"
            aria-selected={tab === "monitor"}
            tabIndex={tab === "monitor" ? 0 : -1}
            onClick={() => setTab("monitor")}
            onKeyDown={onTabKeyDown}
          >
            Theo dõi
          </button>
        </div>
        {tab === "setup" && (
          <CommandBar
            title={issues.length ? `${issues.length} mục cần xử lý` : `${validTargets.length} bài sẵn sàng`}
            detail={issues.length ? "Sửa các mục được đánh dấu trong phần thiết lập." : `Thực hiện trên ${effectiveActors.length} máy theo thứ tự đã chọn.`}
            tone={issues.length ? "warning" : "success"}
            actions={(
              <button
                type="button"
                className="primary"
                disabled={runBusy || issues.length > 0}
                onClick={() => void run()}
              >
                {runBusy ? "Đang bắt đầu…" : pageSurface ? "Bắt đầu tương tác" : "Chạy ngay"}
              </button>
            )}
          />
        )}
        <div
          className="interaction-float-body"
          role="tabpanel"
          id="interaction-panel-setup"
          aria-labelledby="interaction-tab-setup"
          hidden={tab !== "setup"}
        >
          {tab === "setup" && (
            <div className={pageSurface ? "interaction-setup-grid" : undefined}>
              <div className="interaction-setup-main">
                {pageSurface && targetRef && (
                  <AutomationProfileControl
                    kind="interaction"
                    target={targetRef}
                    config={profileConfig}
                    defaultName="Hồ sơ Tương tác"
                    disabled={issues.length > 0}
                    disabledReason={issues[0]?.message}
                  />
                )}
                <InteractionSetupTab
                threshold={{
                  wanted,
                  setWanted,
                  readViews,
                  setReadViews,
                  reading,
                  busy: measureBusy,
                  error: measureError,
                  onMeasure: () => void measure(),
                  canMeasure: validTargets.length > 0 && effectiveActors.length > 0,
                }}
                advancedOpen={advancedOpen}
                setAdvancedOpen={setAdvancedOpen}
                draft={draft}
                patch={patch}
                lines={lines}
                preview={preview}
                issues={issues}
                warnings={warnings}
                devices={devices}
                deviceNumber={deviceNumber}
                deviceLabel={deviceLabel}
                pixelActors={pixelActors}
                hierarchyActors={hierarchyActors}
                largestCohort={largestCohort}
                handles={handles}
                onHandleChange={(udid, value) =>
                  setHandles((prev) => ({ ...prev, [udid]: value }))
                }
                onHandleBlur={(udid, value) => void persistHandle(udid, value)}
                mentions={mentions}
                mentionActorCount={mentionActors.length}
                linkBusy={linkBusy}
                linkError={linkError}
                runError={runError}
                onResolveShortLinks={() => void resolveShortLinks()}
                />
              </div>
              {pageSurface && (
                <SummaryRail
                  title="Kiểm tra chiến dịch"
                  actions={(
                    <StatusChip tone={issues.length ? "warning" : "success"}>
                      {issues.length ? `${issues.length} mục cần xử lý` : "Sẵn sàng"}
                    </StatusChip>
                  )}
                >
                  <dl className="interaction-review-list">
                    <div><dt>Link hợp lệ</dt><dd>{validTargets.length}</dd></div>
                    <div><dt>Thiết bị chạy</dt><dd>{effectiveActors.length}</dd></div>
                    <div><dt>Hành động</dt><dd>{[
                      draft.actions.like && "Tim",
                      draft.actions.save && "Lưu",
                      draft.actions.comment && "Bình luận",
                    ].filter(Boolean).join(" → ")}</dd></div>
                    {draft.actions.comment && (
                      <div><dt>Bình luận/link</dt><dd>{draft.messageCount ?? largestCohort}</dd></div>
                    )}
                  </dl>
                  {warnings.length > 0 && (
                    <StatusChip tone="warning">{warnings.length} cảnh báo</StatusChip>
                  )}
                  {issues.length > 0 && (
                    <ul className="interaction-review-issues" aria-label="Mục cần xử lý">
                      {issues.slice(0, 3).map((issue) => (
                        <li key={`${issue.field}:${issue.message}`}>{issue.message}</li>
                      ))}
                    </ul>
                  )}
                </SummaryRail>
              )}
            </div>
          )}
        </div>
        <div
          className="interaction-float-body"
          role="tabpanel"
          id="interaction-panel-monitor"
          aria-labelledby="interaction-tab-monitor"
          hidden={tab !== "monitor"}
        >
          {tab === "monitor" && (
            <InteractionMonitorTab
              devices={devices}
              deviceNumber={deviceNumber}
              handles={handles}
              openCampaignId={openCampaignId}
              onOpenCampaign={setOpenCampaignId}
              masterDetail={pageSurface}
            />
          )}
        </div>
      </section>
    </div>
  );
}
