import { describe, expect, it } from "vitest";
import {
  buildRequest,
  DEFAULT_DRAFT,
  draftWarnings,
  effectiveMessageCount,
  groupPlanByCohort,
  largestCohortOf,
  manualCommentsOf,
  requestShapeOf,
  validateDraft,
  type InteractionDraft,
  type BuildContext,
  type ValidateContext,
} from "./interactionPlan";
import type { ResolvedTikTokTarget, ThreadPlan } from "./types";

const target: ResolvedTikTokTarget = {
  originalUrl: "https://www.tiktok.com/@.lt.gi.mang.v/photo/7668947001618320660",
  normalizedUrl: "https://www.tiktok.com/@.lt.gi.mang.v/photo/7668947001618320660",
  targetKey: "content:7668947001618320660",
  contentId: "7668947001618320660",
  author: ".lt.gi.mang.v",
  kind: "photo",
};

function context(over: Partial<ValidateContext> = {}): ValidateContext {
  return {
    targets: [target],
    actorUdids: ["a", "b"],
    largestCohort: 2,
    badLineCount: 0,
    mixedThread: false,
    ...over,
  };
}

/** What a *request* needs, which the checks do not: an id, and the mention list. */
function buildContext(over: Partial<BuildContext> = {}): BuildContext {
  return {
    requestId: "req-1",
    targets: [target],
    actorUdids: ["a", "b"],
    mentions: [],
    largestCohort: 2,
    ...over,
  };
}

function draft(over: Partial<InteractionDraft> = {}): InteractionDraft {
  return { ...DEFAULT_DRAFT, ...over };
}

describe("requestShapeOf", () => {
  it("maps the three visible choices onto the two fields the backend takes", () => {
    // One decision used to wear two dependent dropdowns, which made
    // standalone-with-a-chain-shape expressible even though it means nothing.
    expect(requestShapeOf("standalone")).toEqual({ mode: "standalone" });
    expect(requestShapeOf("chain")).toEqual({ mode: "threaded", shape: "chain" });
    expect(requestShapeOf("star")).toEqual({ mode: "threaded", shape: "star" });
  });

  it("defaults to the shape whose failures cost only themselves", () => {
    expect(DEFAULT_DRAFT.threadKind).toBe("star");
  });
});

describe("effectiveMessageCount", () => {
  it("follows the biggest cohort while it is on auto", () => {
    // The old literal default of 2 against a pre-selected fourteen-phone fleet made the form
    // invalid the moment it opened.
    expect(effectiveMessageCount(draft(), 14)).toBe(14);
    expect(effectiveMessageCount(draft(), 0)).toBe(2);
  });

  it("uses the number the operator typed once they type one", () => {
    expect(effectiveMessageCount(draft({ messageCount: 4 }), 14)).toBe(4);
  });
});

describe("validateDraft", () => {
  it("passes a whole-fleet run — the one the schema used to refuse", () => {
    const fleet = Array.from({ length: 14 }, (_, index) => `udid-${index}`);
    expect(
      validateDraft(draft(), context({ actorUdids: fleet, largestCohort: 14 })),
    ).toEqual([]);
  });

  it("refuses before the run rather than after it", () => {
    const issues = validateDraft(draft(), context({ targets: [], actorUdids: ["a"] }));
    expect(issues.map((issue) => issue.field).sort()).toEqual(["actors", "links"]);
  });

  it("counts the unusable lines so the operator knows a paste half-worked", () => {
    const [issue] = validateDraft(draft(), context({ targets: [], badLineCount: 3 }));
    expect(issue.message).toContain("3 dòng");
  });

  it("enforces the manual pool the hint has always advertised", () => {
    // Advertised in a hint and enforced nowhere, so the campaign row existed before the
    // backend refused it with TooFewManualComments.
    const short = draft({ textSource: "manual", manualText: "đẹp quá" });
    const [issue] = validateDraft(short, context({ largestCohort: 2 }));
    expect(issue.field).toBe("manual");
    expect(issue.message).toContain("1 câu");

    const enough = draft({
      textSource: "manual",
      manualText: "đẹp quá\nchỗ này ở đâu ạ\n\n",
    });
    expect(validateDraft(enough, context())).toEqual([]);
  });

  it("offers the one number that would fix a too-small message count", () => {
    const [issue] = validateDraft(
      draft({ messageCount: 3 }),
      context({ actorUdids: ["a", "b", "c", "d"], largestCohort: 4 }),
    );
    expect(issue.field).toBe("messageCount");
    expect(issue.fix).toEqual({ label: "Đặt = 4", messageCount: 4 });
  });

  it("refuses a mixed-reader thread but allows the same phones standalone", () => {
    const mixed = context({ mixedThread: true });
    expect(validateDraft(draft({ threadKind: "chain" }), mixed)).toHaveLength(1);
    expect(validateDraft(draft({ threadKind: "standalone" }), mixed)).toEqual([]);
  });

  it("surfaces the planner's own refusal instead of swallowing it", () => {
    const [issue] = validateDraft(draft(), context({ planError: "DuplicateActor" }));
    expect(issue.field).toBe("plan");
    expect(issue.message).toBe("DuplicateActor");
  });

  it("keeps zero meaning one cohort", () => {
    expect(validateDraft(draft({ cohortSize: 0 }), context())).toEqual([]);
    expect(validateDraft(draft({ cohortSize: 1 }), context())).toHaveLength(1);
  });
});

describe("draftWarnings", () => {
  it("says when the AI is being asked for more comments than it can keep distinct", () => {
    // Measured 24/08/2026: fourteen comments on one photo post had the model's own quality
    // gate refuse two for being too generic. The operator could not have known that from the
    // form, because the form only spoke in refusals.
    const fleet = Array.from({ length: 14 }, (_, index) => `udid-${index}`);
    const [warning] = draftWarnings(
      draft(),
      context({ actorUdids: fleet, largestCohort: 14 }),
    );
    expect(warning).toContain("14 bình luận cho mỗi link");
  });

  it("says nothing when the operator wrote the comments themselves", () => {
    const fleet = Array.from({ length: 14 }, (_, index) => `udid-${index}`);
    const manual = draft({
      textSource: "manual",
      manualText: Array.from({ length: 14 }, (_, index) => `câu ${index}`).join("\n"),
    });
    expect(draftWarnings(manual, context({ actorUdids: fleet, largestCohort: 14 }))).toEqual(
      [],
    );
  });

  it("never blocks the run — a warned campaign is still a legal one", () => {
    const fleet = Array.from({ length: 14 }, (_, index) => `udid-${index}`);
    const ctx = context({ actorUdids: fleet, largestCohort: 14 });
    expect(draftWarnings(draft(), ctx)).toHaveLength(1);
    expect(validateDraft(draft(), ctx)).toEqual([]);
  });

  it("stays quiet for a cohort-sized run, which is the shape being recommended", () => {
    expect(draftWarnings(draft(), context({ largestCohort: 3 }))).toEqual([]);
  });
});

describe("buildRequest", () => {
  it("sends the shape, the resolved message count, and the trimmed manual pool", () => {
    const request = buildRequest(
      draft({ textSource: "manual", manualText: " một \n\n hai " }),
      buildContext({ largestCohort: 2, mentions: ["ann"] }),
    );
    expect(request).toMatchObject({
      requestId: "req-1",
      mode: "threaded",
      shape: "star",
      messageCount: 2,
      manualComments: ["một", "hai"],
      mentions: ["ann"],
    });
    // Zero is "one cohort", and the backend spells that as absent.
    expect(request.cohortSize).toBeUndefined();
  });
});

describe("groupPlanByCohort", () => {
  const plan: ThreadPlan = {
    requestId: "req-1",
    assignments: [
      { targetKey: "content:1", ordinal: 0, actorUdid: "a", parentOrdinal: null, cohort: 0 },
      { targetKey: "content:1", ordinal: 1, actorUdid: "b", parentOrdinal: 0, cohort: 0 },
      { targetKey: "content:2", ordinal: 0, actorUdid: "c", parentOrdinal: null, cohort: 1 },
    ],
  };

  it("reads the teams off the backend's plan rather than recomputing the split", () => {
    const cohorts = groupPlanByCohort(plan);
    expect(cohorts).toHaveLength(2);
    expect(cohorts[0]).toEqual({
      cohort: 0,
      actorUdids: ["a", "b"],
      targetKeys: ["content:1"],
    });
    expect(cohorts[1].targetKeys).toEqual(["content:2"]);
  });

  it("has nothing to show before a plan arrives", () => {
    expect(groupPlanByCohort(null)).toEqual([]);
  });

  it("measures the biggest team, which is the number message count has to clear", () => {
    expect(largestCohortOf(groupPlanByCohort(plan), 99)).toBe(2);
    expect(largestCohortOf([], 7)).toBe(7);
  });
});

describe("manualCommentsOf", () => {
  it("keeps what was pasted when the AI is writing instead", () => {
    // Switching back to AI must not delete the pool — the operator may switch again.
    expect(manualCommentsOf(draft({ textSource: "ai", manualText: "đẹp quá" }))).toEqual([]);
  });
});
