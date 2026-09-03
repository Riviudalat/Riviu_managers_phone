import type {
  JsonValue,
  NurtureSettings,
  PublishSoundPolicy,
  ThreadCampaignRequest,
} from "./types";

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

export function interactionProfileConfig(request: ThreadCampaignRequest): JsonValue {
  const { requestId: _requestId, actorUdids: _actorUdids, ...profileRequest } = request;
  return cloneJson({ schemaVersion: 1, request: profileRequest });
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
