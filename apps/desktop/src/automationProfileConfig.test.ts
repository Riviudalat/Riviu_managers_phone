import { describe, expect, it } from "vitest";

import {
  interactionProfileConfig,
  nurtureProfileConfig,
  publishProfileConfig,
} from "./automationProfileConfig";
import type { NurtureSettings, ThreadCampaignRequest } from "./types";

const nurtureSettings: NurtureSettings = {
  baseUrl: "https://provider.example/v1",
  model: "model",
  apiKey: "must-not-be-stored",
  hasApiKey: true,
  bundleId: "com.zhiliaoapp.musically",
  numVideos: 3,
  numRounds: 1,
  likeProb: 10,
  commentProb: 10,
  saveProb: 25,
  followProb: 5,
  frenzyProb: 20,
  watchMin: 2,
  watchMax: 4,
  persona: "viewer",
  fatigue: true,
  timeOfDay: true,
  pauseSwipe: true,
  nightStart: 0,
  nightEnd: 0,
  recoverDelayMin: 1,
  recoverDelayMax: 2,
  staggerDelayMin: 1,
  staggerDelayMax: 2,
  commentLang: "vi",
  aiDirections: "brief",
  maxCommentWords: 12,
  scheduleEnabled: false,
  scheduleEveryMinutes: 60,
  scheduleDurationMinutes: 10,
  scheduleUdids: ["old-device"],
  saveEnabled: true,
};

describe("automation profile config v1", () => {
  it("strips nurture credentials while preserving independent Save settings", () => {
    const config = nurtureProfileConfig(nurtureSettings, 12);
    expect(config).toMatchObject({
      schemaVersion: 1,
      durationMinutes: 12,
      settings: { saveEnabled: true, saveProb: 25 },
    });
    expect(JSON.stringify(config)).not.toContain("must-not-be-stored");
    expect(config).not.toHaveProperty("settings.apiKey");
    expect(config).not.toHaveProperty("settings.hasApiKey");
  });

  it("removes interaction attempt identity and device actors", () => {
    const request: ThreadCampaignRequest = {
      requestId: "attempt-id",
      targets: [{
        originalUrl: "https://www.tiktok.com/@riviu/video/1",
        normalizedUrl: "https://www.tiktok.com/@riviu/video/1",
        targetKey: "video:1",
        contentId: "1",
        author: "riviu",
        kind: "video",
      }],
      actorUdids: ["phone-a"],
      messageCount: 1,
      instruction: "",
      maxWords: 12,
      mode: "standalone",
      actions: { like: true, comment: false, save: true },
    };
    const config = interactionProfileConfig(request);
    expect(config).toMatchObject({
      schemaVersion: 1,
      request: { actions: { like: true, comment: false, save: true } },
    });
    expect(config).not.toHaveProperty("request.requestId");
    expect(config).not.toHaveProperty("request.actorUdids");
  });

  it("pins Publish input and seeded TikTok sound policy without run identity", () => {
    expect(
      publishProfileConfig(
        "C:/Nội dung",
        ["bo-1"],
        { "bo-1": "Caption" },
        { kind: "trendingAny", poolSize: 5, seed: 42 },
        true,
      ),
    ).toEqual({
      schemaVersion: 1,
      sourceRoot: "C:/Nội dung",
      bundleIds: ["bo-1"],
      captionOverrides: { "bo-1": "Caption" },
      soundPolicy: { kind: "trendingAny", poolSize: 5, seed: 42 },
      executionConfirmed: true,
    });
  });
});
