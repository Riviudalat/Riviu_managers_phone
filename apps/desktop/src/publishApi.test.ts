import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { publishCreateCampaign, publishPreflight, publishReconcile } from "./api";
import type { PublishPreflightRequest } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("Publish API client", () => {
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
        },
      ],
      ["publish_reconcile", { campaignId: "campaign-a" }],
    ]);
  });
});
