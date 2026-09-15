import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { publishCreateCampaign, publishPreflight, publishReconcile, publishRetryAssignment } from "./api";
import type { PublishPreflightRequest } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("Publish API client", () => {
  it("retries exactly one assignment through its typed command", async () => {
    await publishRetryAssignment("failed-assignment", true);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("publish_retry_assignment", { assignmentId: "failed-assignment", confirmed: true });
  });
  it("pins the preflight digest and restart reconciliation wire contract", async () => {
    const request: PublishPreflightRequest = {
      sourceRoot: "C:/Nội dung đăng",
      bundleIds: ["bundle-a"],
      udids: ["PHONE-A"],
      targetRef: { type: "group", groupId: "morning" },
      runAt: null,
      captionOverrides: { "bundle-a": "Chú thích đã duyệt" },
      soundPolicy: { kind: "trendingAny", poolSize: 5, seed: 42 },
    };

    await publishPreflight(request);
    await publishCreateCampaign(
      request.sourceRoot,
      request.bundleIds,
      request.udids,
      null,
      request.captionOverrides,
      request.soundPolicy,
      request.targetRef!,
      true,
      "approved-digest",
    );
    await publishReconcile("campaign-a");

    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["publish_preflight", { request }],
      [
        "publish_create_campaign",
        {
          sourceRoot: request.sourceRoot,
          bundleIds: request.bundleIds,
          udids: request.udids,
          runAt: null,
          captionOverrides: request.captionOverrides,
          soundPolicy: request.soundPolicy,
          targetRef: request.targetRef,
          confirmed: true,
          approvedInputDigest: "approved-digest",
          sheetEnabled: true,
          deleteAfterPublish: true,
          requestId: null,
        },
      ],
      ["publish_reconcile", { campaignId: "campaign-a" }],
    ]);
  });

  it("forwards a stable creation request ID unchanged across retries", async () => {
    const requestId = "ec19ec69-0606-4987-95e8-06eb2d21caa8";
    for (let attempt = 0; attempt < 2; attempt += 1) {
      await publishCreateCampaign("C:/Nội dung đăng", ["bundle-a"], ["PHONE-A"], null,
        { "bundle-a": "Chú thích đã duyệt" }, { kind: "default" },
        { type: "explicit", udids: ["PHONE-A"] }, true, "approved-digest", false, false, requestId);
    }
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(vi.mocked(invoke).mock.calls[0]).toEqual(vi.mocked(invoke).mock.calls[1]);
    expect(invoke).toHaveBeenLastCalledWith("publish_create_campaign", expect.objectContaining({ requestId }));
  });
});
