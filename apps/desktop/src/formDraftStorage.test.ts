import { expect, it } from "vitest";
import { readFormDraft, readTargetDraft, restoreFormShape, writeFormDraft } from "./formDraftStorage";
import { readPublishForm } from "./components/publish/publishDraftStorage";

it("restores known editor fields, discarding incompatible fields", () => {
  const defaultDraft = { text: "", actors: [] as string[], actions: { save: false, comment: true } };
  writeFormDraft("test", { text: "caption", actors: ["A"], actions: { save: true }, approval: true });
  expect(readFormDraft("test", value => restoreFormShape(value, defaultDraft))).toEqual({
    text: "caption", actors: ["A"], actions: { save: true, comment: true },
  });
  expect(restoreFormShape({ actors: [5] }, { actors: [] })).toEqual({ actors: [] });
});
it("validates the target union without turning a group into all devices", () => {
  writeFormDraft("scope", { type: "group", groupId: "g" });
  expect(readTargetDraft("scope", { type: "all" })).toEqual({ type: "group", groupId: "g" });
});
it("restores publish input without ever restoring approval or media evidence", () => {
  const draft = { sourceRoot: "C:/media", bundleIds: ["b"], assignments: { b: "phone" },
    captionDrafts: { b: "Caption" }, runAt: "", soundPolicyOverride: null,
    sheetEnabled: false, deleteAfterPublish: true };
  writeFormDraft("publish", { ...draft, canExecute: true, inputDigest: "old", manifest: { bundles: [] } });
  expect(readPublishForm()).toEqual(draft);
  localStorage.setItem("riviu.form-draft.v1.publish", "broken-json");
  expect(readPublishForm()).toBeNull();
});
