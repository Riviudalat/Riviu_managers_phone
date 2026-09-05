import type {
  JsonValue,
  NurtureSettings,
  PublishSoundPolicy,
  ThreadCampaignRequest,
  TargetRef,
} from "./types";
import { DEFAULT_DRAFT, type InteractionDraft } from "./interactionPlan";

function cloneJson(value: unknown): JsonValue {
  return JSON.parse(JSON.stringify(value)) as JsonValue;
}

export function nurtureProfileConfig(
  settings: NurtureSettings,
  durationMinutes?: number,
): JsonValue {
  const { apiKey: _apiKey, hasApiKey: _hasApiKey, ...publicSettings } = settings;
  return cloneJson({
    schemaVersion: 1,
    settings: publicSettings,
    ...(durationMinutes === undefined ? {} : { durationMinutes }),
  });
}

export function nurtureSettingsFromProfile(config: JsonValue, current: NurtureSettings): NurtureSettings {
  if (!config || typeof config !== "object" || Array.isArray(config) || config.schemaVersion !== 1
    || !config.settings || typeof config.settings !== "object" || Array.isArray(config.settings)) {
    throw new Error("Hồ sơ Nuôi TikTok không đúng định dạng.");
  }
  const values = config.settings;
  const optionalDefaults = {
    saveProb: 0, steadyMood: "", likeEnabled: true, commentEnabled: true, saveEnabled: false,
    followEnabled: true, frenzyEnabled: true, carouselEnabled: true, carouselMaxSlides: 1,
    carouselPortionPercent: 100, humanLimits: false,
  };
  for (const [key, value] of Object.entries(values)) {
    if (key === "apiKey" || key === "hasApiKey") continue;
    if (key === "scheduleWindows") {
      if (!Array.isArray(value) || !value.every((window) => {
        if (!window || typeof window !== "object" || Array.isArray(window)) return false;
        return typeof window.id === "string" && ["startMinute", "endMinute", "everyMinutes", "durationMinutes"]
          .every((field) => typeof window[field] === "number") && Array.isArray(window.udids)
          && window.udids.every((udid) => typeof udid === "string")
          && (window.behaviour == null || (typeof window.behaviour === "object" && !Array.isArray(window.behaviour)
            && Object.entries(window.behaviour).every(([field, entry]) => typeof entry === (field === "saveEnabled" ? "boolean" : "number"))));
      })) {
        throw new Error("Hồ sơ Nuôi TikTok có cửa sổ lịch sai kiểu dữ liệu.");
      }
      continue;
    }
    const expected = current[key as keyof NurtureSettings] ?? optionalDefaults[key as keyof typeof optionalDefaults];
    if (expected !== undefined && (Array.isArray(expected) ? !Array.isArray(value)
      || !value.every((item) => typeof item === "string") : typeof value !== typeof expected)) {
      throw new Error("Hồ sơ Nuôi TikTok có thiết lập sai kiểu dữ liệu.");
    }
  }
  return { ...current, ...values, apiKey: current.apiKey, hasApiKey: current.hasApiKey } as NurtureSettings;
}

export function interactionProfileConfig(request: ThreadCampaignRequest): JsonValue {
  const { requestId: _requestId, actorUdids: _actorUdids, ...profileRequest } = request;
  return cloneJson({ schemaVersion: 1, request: profileRequest });
}

export function interactionProfileTarget(
  target: TargetRef,
  inScope: readonly string[],
  actors: readonly string[],
): TargetRef {
  return inScope.length === actors.length && inScope.every((udid, index) => udid === actors[index])
    ? target
    : { type: "explicit", udids: [...actors] };
}

export function interactionDraftFromProfile(config: JsonValue, actors: string[]): InteractionDraft {
  if (!config || typeof config !== "object" || Array.isArray(config) || config.schemaVersion !== 1
    || !config.request || typeof config.request !== "object" || Array.isArray(config.request)) {
    throw new Error("Hồ sơ Tương tác không đúng định dạng.");
  }
  const request = config.request;
  if (!Array.isArray(request.targets) || !request.targets.every((target) => target && typeof target === "object"
    && !Array.isArray(target) && typeof target.normalizedUrl === "string")) {
    throw new Error("Hồ sơ Tương tác thiếu danh sách bài hợp lệ.");
  }
  const actions = request.actions ?? { like: request.likeTarget === true, comment: true, save: false };
  if (!actions || typeof actions !== "object" || Array.isArray(actions)
    || typeof actions.like !== "boolean" || typeof actions.comment !== "boolean" || typeof actions.save !== "boolean") {
    throw new Error("Hồ sơ Tương tác thiếu lựa chọn hành động.");
  }
  const manual = Array.isArray(request.manualComments) ? request.manualComments.filter((item): item is string => typeof item === "string") : [];
  return {
    ...DEFAULT_DRAFT,
    rawLinks: request.targets.map((target) => (target as { normalizedUrl: string }).normalizedUrl).join("\n"),
    actors,
    actions: { like: actions.like, comment: actions.comment, save: actions.save },
    messageCount: typeof request.messageCount === "number" ? request.messageCount : null,
    maxWords: typeof request.maxWords === "number" ? request.maxWords : DEFAULT_DRAFT.maxWords,
    instruction: typeof request.instruction === "string" ? request.instruction : "",
    threadKind: request.mode === "standalone" ? "standalone" : request.shape === "chain" ? "chain" : "star",
    textSource: manual.length ? "manual" : "ai",
    manualText: manual.join("\n"),
    mentionParent: request.mentionParent === true,
    mentionText: Array.isArray(request.mentions) ? request.mentions.filter((item) => typeof item === "string").join(" ") : "",
  };
}

export function publishProfileConfig(
  sourceRoot: string,
  bundleIds: string[],
  captionOverrides: Record<string, string>,
  soundPolicy: PublishSoundPolicy,
  executionConfirmed: boolean,
): JsonValue {
  return cloneJson({
    schemaVersion: 1,
    sourceRoot,
    bundleIds,
    captionOverrides,
    soundPolicy,
    executionConfirmed,
  });
}
