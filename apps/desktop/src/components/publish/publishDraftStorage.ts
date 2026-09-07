import { readFormDraft } from "../../formDraftStorage";
import type { PublishSoundPolicy } from "../../types";

export interface PublishFormDraft {
  sourceRoot: string;
  bundleIds: string[];
  assignments: Record<string, string>;
  captionDrafts: Record<string, string>;
  runAt: string;
  soundPolicyOverride: PublishSoundPolicy | null;
  sheetEnabled: boolean;
  deleteAfterPublish: boolean;
}
const strings = (value: unknown): value is Record<string, string> => !!value && typeof value === "object"
  && !Array.isArray(value) && Object.values(value).every(v => typeof v === "string");
export function readPublishForm(): PublishFormDraft | null {
  return readFormDraft("publish", value => {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw Error("Invalid publish draft");
    const d = value as Record<string, unknown>;
    if (typeof d.sourceRoot !== "string" || typeof d.runAt !== "string"
      || !Array.isArray(d.bundleIds) || !d.bundleIds.every(id => typeof id === "string")
      || !strings(d.assignments) || !strings(d.captionDrafts)
      || typeof d.sheetEnabled !== "boolean" || typeof d.deleteAfterPublish !== "boolean") throw Error("Invalid publish fields");
    const policy = d.soundPolicyOverride as PublishSoundPolicy | null;
    if (policy !== null && (!policy || (policy.kind !== "default" && !(policy.kind === "trendingAny"
      && Number.isInteger(policy.seed) && policy.poolSize >= 1 && policy.poolSize <= 5)))) throw Error("Invalid sound policy");
    return { sourceRoot: d.sourceRoot, runAt: d.runAt, bundleIds: d.bundleIds,
      assignments: d.assignments, captionDrafts: d.captionDrafts, sheetEnabled: d.sheetEnabled,
      deleteAfterPublish: d.deleteAfterPublish, soundPolicyOverride: policy };
  });
}
