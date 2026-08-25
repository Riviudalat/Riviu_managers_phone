/**
 * The interaction form as data, and everything that can be decided about it without a DOM.
 *
 * Split out of the popup for two reasons. The obvious one is that a 965-line component with
 * eighteen `useState` had nowhere to put a rule. The load-bearing one is that every check
 * here used to run **after** the operator pressed Chạy ngay, into a single shared error
 * string — so a form that could not possibly succeed looked identical to one that could, and
 * the reason arrived only once the attempt had already been made.
 */
import { interactionErrorVi } from "./interactionErrors";
import type {
  ResolvedTikTokTarget,
  ThreadCampaignRequest,
  ThreadPlan,
} from "./types";

/**
 * The three shapes an operator actually chooses between.
 *
 * On screen this was two dependent dropdowns — "Kiểu tương tác" (qua lại / riêng lẻ) and
 * "Hình chuỗi" (nối tiếp / toả), the second only meaningful when the first said qua lại. That
 * is one decision wearing two controls, and the combination that does not exist
 * (standalone + a chain shape) was expressible.
 */
export type ThreadKind = "standalone" | "chain" | "star";

export interface InteractionDraft {
  rawLinks: string;
  /**
   * How many comments each link gets, or `null` for "as many as the biggest cohort".
   *
   * Null is the default and it matters: the old default was the literal 2, while the popup
   * pre-selects the whole platform group, so opening it on this fourteen-phone fleet
   * produced a form that was already invalid — `messageCount >= largest cohort` is a backend
   * rule. With pre-submit gating that would have meant a Chạy ngay disabled on first paint
   * for a reason the operator never asked for.
   */
  messageCount: number | null;
  maxWords: number;
  threadKind: ThreadKind;
  textSource: "ai" | "manual";
  instruction: string;
  manualText: string;
  likeTarget: boolean;
  /** Each reply tags the account it answers — the fleet talking to itself. */
  mentionParent: boolean;
  mentionText: string;
  actors: string[];
}

export const DEFAULT_DRAFT: InteractionDraft = {
  rawLinks: "",
  messageCount: null,
  maxWords: 12,
  // Star: the replies do not wait for each other, and one that fails costs only itself. The
  // chain default predated the star shape existing.
  threadKind: "star",
  textSource: "ai",
  instruction: "tự nhiên, ngắn, nói như người vừa xem xong",
  manualText: "",
  likeTarget: false,
  mentionParent: false,
  mentionText: "",
  actors: [],
};

/** One reason Chạy ngay is not available, tied to the field that can fix it. */
export interface DraftIssue {
  field:
    | "links"
    | "actors"
    | "manual"
    | "messageCount"
    | "maxWords"
    | "plan";
  message: string;
  /** A value that would resolve it, when there is exactly one obvious value. */
  fix?: { label: string; messageCount: number };
}

export const MIXED_THREAD_REASON =
  "Chuỗi lồng nhau không chạy trộn iPhone với Android: hai bên đọc nhãn tác giả theo hai " +
  "cách nên mắt xích có thể đứt giữa chừng. Chọn toàn iPhone, toàn Android, hoặc chuyển " +
  "sang Riêng lẻ.";

/**
 * What a `<input type="number">` really hands back, read as a whole number.
 *
 * The box returns its raw text and a `step` violation does not blank `.value`, so `"2.5"`
 * arrives verbatim. `Number("2.5")` then passed every range check in this file and was
 * serialised into a number the Rust side deserialises as an integer: a serde failure at
 * dispatch, in English, with no field to hang it on. Truncated rather than refused, because
 * `2.5` typed into a box counting whole things means 2.
 *
 * An empty box reads as `0`. Only `messageCount` treats empty as "automatic", and it checks
 * for the empty string before calling this.
 */
export function wholeNumber(value: string): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.trunc(parsed) : 0;
}

/** Comments the operator typed: one per non-blank line, in order. */
export function manualCommentsOf(draft: InteractionDraft): string[] {
  if (draft.textSource !== "manual") return [];
  return draft.manualText
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/**
 * How many messages the run will actually ask for.
 *
 * `largestCohort` comes from the backend's own plan when there is one. Falling back to the
 * actor count is right for the un-split case, which is what a fleet with no cohort size is.
 */
export function effectiveMessageCount(
  draft: InteractionDraft,
  largestCohort: number,
): number {
  if (draft.messageCount !== null) return draft.messageCount;
  return Math.max(2, largestCohort);
}

/** `threadKind` as the two fields the backend has always taken. */
export function requestShapeOf(kind: ThreadKind): Pick<ThreadCampaignRequest, "mode" | "shape"> {
  if (kind === "standalone") return { mode: "standalone" };
  return { mode: "threaded", shape: kind };
}

export interface BuildContext {
  requestId: string;
  targets: ResolvedTikTokTarget[];
  /** Selected actors already unioned with the phones the mentions name. */
  actorUdids: string[];
  mentions: string[];
  largestCohort: number;
  /**
   * What the request is for. `"preview"` leaves the manual comment list out.
   *
   * The preview exists to answer one question — how the planner would split these phones into
   * cohorts — and it is the *only* channel that can tell the panel the real cohort size. Sending
   * the comment list with it made that channel depend on a number it was being asked to
   * provide: fourteen phones, cohort size 3 and four pasted comments needs exactly four
   * comments, but with no preview yet the panel guesses the fleet count, the preview is refused
   * for `TooFewManualComments` against fourteen, so no preview arrives, so the guess never
   * improves. The screen then demanded "cần ≥ 14" — a number that was never true — with no way
   * out but typing `4` by hand.
   */
  purpose?: "run" | "preview";
}

export function buildRequest(
  draft: InteractionDraft,
  context: BuildContext,
): ThreadCampaignRequest {
  return {
    requestId: context.requestId,
    targets: context.targets,
    actorUdids: context.actorUdids,
    messageCount: effectiveMessageCount(draft, context.largestCohort),
    instruction: draft.instruction,
    maxWords: draft.maxWords,
    ...requestShapeOf(draft.threadKind),
    // **No cohort size is sent, so the actor list is always one cohort.**
    //
    // It used to be an advanced field, and it quietly outranked the thing above it: load a
    // group of eight with a cohort size of three and the group became `[3,3,2]` — three
    // separate root comments — while the screen still said "Toả: các máy cùng trả lời bình
    // luận gốc". Groups are how a fleet is divided now, and two ways to divide it that
    // disagree is one too many. `undefined` means `partition_actors(actors, None)`, which is
    // a single cohort: the group, whole.
    cohortSize: undefined,
    manualComments: context.purpose === "preview" ? [] : manualCommentsOf(draft),
    likeTarget: draft.likeTarget,
    mentionParent: draft.threadKind === "standalone" ? false : draft.mentionParent,
    mentions: context.mentions,
  };
}

/**
 * What the checks need — which is **not** what a request needs.
 *
 * It used to extend `BuildContext`, so it carried a `requestId` and a `mentions` list that
 * nothing in this file reads. Worse than dead weight: the popup builds this in a `useMemo`
 * whose dependency array cannot list `requestIdRef`, so that copy went stale the moment `run()`
 * rotated the id after a successful dispatch. Harmless only for as long as nobody reads it, and
 * a field that exists to be ignored is a field the next person will read.
 */
export interface ValidateContext {
  targets: ResolvedTikTokTarget[];
  /** Selected actors already unioned with the phones the mentions name. */
  actorUdids: string[];
  /** The biggest cohort the backend's own plan produced, or the fleet size with no plan yet. */
  largestCohort: number;
  /** Lines that were pasted but could not be read as a target. */
  badLineCount: number;
  /**
   * True while the plan on screen was computed for a different draft.
   *
   * `largestCohort` comes from the last preview, and the preview is 350 ms behind the draft. Any
   * gap there is a number the run would use and the planner would refuse: deselect a phone, wait
   * for the preview, reselect it, press Chạy ngay inside the debounce, and the request goes out
   * with thirteen messages for fourteen actors. Blocking for the length of one round trip is the
   * only answer that does not involve guessing.
   */
  previewStale?: boolean;
  /** True when a threaded run would mix the two ways of reading the screen. */
  mixedThread: boolean;
  /** A refusal the backend's own planner gave for this draft, if it gave one. */
  planError?: string | null;
}

/**
 * Everything standing between this draft and a campaign.
 *
 * Returned as a list rather than one string because they are independent: a form can be
 * missing a link *and* have too few comments, and fixing one used to overwrite the message
 * about the other.
 */
export function validateDraft(
  draft: InteractionDraft,
  context: ValidateContext,
): DraftIssue[] {
  const issues: DraftIssue[] = [];

  if (context.targets.length === 0) {
    issues.push({
      field: "links",
      message: context.badLineCount
        ? `Cần ít nhất một link hợp lệ — ${context.badLineCount} dòng đang lỗi`
        : "Cần ít nhất một link video/photo hợp lệ",
    });
  }

  const actors = context.actorUdids.length;
  if (actors < 2 || actors > 64) {
    issues.push({
      field: "actors",
      message: `Chọn từ 2 đến 64 máy làm actor (kể cả acc được tag) — đang chọn ${actors}`,
    });
  }

  if (draft.threadKind !== "standalone" && context.mixedThread) {
    issues.push({ field: "actors", message: MIXED_THREAD_REASON });
  }

  // **The plan on screen has to be the plan for this draft.** `largestCohort` is read out of
  // the last preview, so any moment the two disagree is a moment `messages` is a number the run
  // would send and the planner would refuse. Held for the length of one round trip rather than
  // guessed — which is what the rewrite claimed to do, and did only in the direction where the
  // stale value happened to be too small.
  if (context.previewStale) {
    issues.push({ field: "plan", message: "Đang tính lại kế hoạch cho lựa chọn mới…" });
  }

  const messages = effectiveMessageCount(draft, context.largestCohort);

  // Advertised in a hint since the feature shipped and enforced nowhere, so the run started
  // and the backend refused it — `TooFewManualComments`, after the campaign row existed.
  if (draft.textSource === "manual") {
    const pool = manualCommentsOf(draft).length;
    if (pool < messages) {
      issues.push({
        field: "manual",
        message: `Danh sách bình luận đang có ${pool} câu, cần ≥ ${messages}`,
      });
    }
  }

  if (draft.messageCount !== null) {
    if (draft.messageCount < 2 || draft.messageCount > 64) {
      issues.push({
        field: "messageCount",
        message: "Số bình luận mỗi link phải từ 2 đến 64",
      });
      // Per cohort, not per fleet: fourteen phones in teams of three need three comments a
      // link, not fourteen. Measured against the biggest team, because spreading the
      // remainder makes them uneven by one.
    } else if (draft.messageCount < context.largestCohort) {
      issues.push({
        field: "messageCount",
        message: `Cụm lớn nhất có ${context.largestCohort} máy nên cần ≥ ${context.largestCohort} bình luận mỗi link`,
        fix: {
          label: `Đặt = ${context.largestCohort}`,
          messageCount: context.largestCohort,
        },
      });
    }
  }

  if (draft.maxWords < 4 || draft.maxWords > 20) {
    issues.push({ field: "maxWords", message: "Số từ tối đa mỗi câu phải từ 4 đến 20" });
  }

  // **Translated, and only when nothing above already said it.** The planner runs the same
  // `validate()` this function mirrors, so its refusal is usually the Vietnamese reason already
  // on screen said again in English — the existing test walked straight into a panel showing
  // both "Cụm lớn nhất có 3 máy nên cần ≥ 3 bình luận" and "InteractionFailed: message count
  // must cover every selected actor", and asserted only the first, so it passed while the UI was
  // visibly wrong. A refusal the panel has no field for is still worth showing: that one is a
  // rule this copy does not know about.
  if (context.planError && issues.length === 0) {
    issues.push({ field: "plan", message: interactionErrorVi(context.planError).title });
  }

  return issues;
}

/**
 * How many comments the AI has to invent for one post before it starts repeating itself.
 *
 * Measured 24/08/2026 on `.../@.lt.gi.mang.v/photo/7668947001618320660`: asking for fourteen
 * comments on one photo post had the model's own quality gate refuse two of them for being
 * too generic (`genericity=58`, `genericity=35`) — the fourteenth thing to say about one
 * picture is not as specific as the first. Six was the old schema cap and never showed this,
 * so the warning starts above it.
 */
const AI_COMMENTS_PER_LINK_ADVISORY = 8;

/**
 * Things worth saying before a run that are **not** reasons to refuse it.
 *
 * Kept apart from [`validateDraft`] on purpose: an issue disables Chạy ngay, and a campaign
 * that will probably lose a couple of comments to the quality gate is still a campaign the
 * operator may well want. Blurring the two would either block a legitimate run or bury a real
 * refusal among advice.
 */
export function draftWarnings(draft: InteractionDraft, context: ValidateContext): string[] {
  const warnings: string[] = [];
  const messages = effectiveMessageCount(draft, context.largestCohort);
  if (draft.textSource === "ai" && messages >= AI_COMMENTS_PER_LINK_ADVISORY) {
    warnings.push(
      `${messages} bình luận cho mỗi link là nhiều: AI phải nghĩ ra ${messages} câu khác nhau về cùng một bài, ` +
        "và cổng chất lượng sẽ loại bớt những câu chung chung. Bớt máy, thêm link, " +
        "hoặc dán sẵn danh sách bình luận nếu muốn đủ số.",
    );
  }
  return warnings;
}

/** One cohort of the backend's plan, ready to render. */
export interface CohortView {
  cohort: number;
  /** Actor udids in the order the plan assigns them, deduplicated. */
  actorUdids: string[];
  targetKeys: string[];
}

/**
 * Group a plan by the team that will run it.
 *
 * This replaces a hand-written copy of `partition_actors` — remainder-spreading and all —
 * that lived in the popup purely to draw a preview. Two implementations of one split are two
 * chances to show the operator a plan that is not the plan.
 */
export function groupPlanByCohort(plan: ThreadPlan | null | undefined): CohortView[] {
  if (!plan) return [];
  const byCohort = new Map<number, CohortView>();
  for (const assignment of plan.assignments) {
    const cohort = assignment.cohort ?? 0;
    let view = byCohort.get(cohort);
    if (!view) {
      view = { cohort, actorUdids: [], targetKeys: [] };
      byCohort.set(cohort, view);
    }
    if (!view.actorUdids.includes(assignment.actorUdid)) {
      view.actorUdids.push(assignment.actorUdid);
    }
    if (!view.targetKeys.includes(assignment.targetKey)) {
      view.targetKeys.push(assignment.targetKey);
    }
  }
  return [...byCohort.values()].sort((left, right) => left.cohort - right.cohort);
}

/** The biggest team in a plan — the number `messageCount` has to clear. */
export function largestCohortOf(cohorts: CohortView[], fallback: number): number {
  if (!cohorts.length) return fallback;
  return cohorts.reduce((most, team) => Math.max(most, team.actorUdids.length), 0);
}
