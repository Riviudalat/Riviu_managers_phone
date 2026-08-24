import { describe, expect, it } from "vitest";
import {
  assignmentStateVi,
  campaignStateVi,
  interactionErrorVi,
  linkErrorVi,
  stateTone,
} from "./interactionErrors";

describe("interactionErrorVi", () => {
  it("names a hierarchy refusal and keeps the engine's own sentence as the detail", () => {
    // The engine stores these as `code: câu tiếng Việt`, and that sentence was written next
    // to the measurement it describes — re-translating it here would only make it worse.
    const view = interactionErrorVi(
      "target_open_screen_unchanged: mở link xong màn hình vẫn là bài cũ (Follow Xuân)",
    );
    expect(view.title).toBe("Mở link xong màn hình không đổi");
    expect(view.detail).toBe("mở link xong màn hình vẫn là bài cũ (Follow Xuân)");
    expect(view.raw).toContain("target_open_screen_unchanged");
  });

  it("counts the skipped parent from one, the way the operator reads it", () => {
    // The ordinal is zero-based in the code and one-based in every list on screen.
    expect(interactionErrorVi("parent_identity_not_confirmed_at_ordinal_5").title).toContain(
      "thứ 6",
    );
    expect(interactionErrorVi("parent_identity_not_confirmed_at_ordinal_0").title).toContain(
      "thứ 1",
    );
  });

  it("keeps the AI cause chain readable instead of hiding it", () => {
    const view = interactionErrorVi(
      "ai_comment_unavailable: ordinal 0 — comment_context_rejected: context=0 overall=0",
    );
    expect(view.title).toBe("AI không viết được bình luận");
    expect(view.detail).toBe("ordinal 0 — comment_context_rejected: context=0 overall=0");
  });

  it("still says something useful for a refusal added to the engine later", () => {
    expect(interactionErrorVi("reply_something_new: chưa đo").title).toBe(
      "Không trả lời được",
    );
    expect(interactionErrorVi("target_open_something_new").title).toBe("Không mở được bài");
  });

  it("passes an unrecognised reason through rather than replacing it with a guess", () => {
    // The campaign-level code is a free-text anyhow chain and is often already Vietnamese.
    const raw = "AI API key chưa được cấu hình cho Interaction";
    expect(interactionErrorVi(raw)).toEqual({ title: raw, raw });
  });

  it("always carries the raw code, because that is what a bug report is written from", () => {
    for (const raw of [
      "reply_parent_not_found: không thấy bình luận cha",
      "parent_identity_not_confirmed_at_ordinal_2",
      "điều gì đó hoàn toàn mới",
    ]) {
      expect(interactionErrorVi(raw).raw).toBe(raw);
    }
  });
});

describe("linkErrorVi", () => {
  it("covers every code the parser can produce", () => {
    const codes = [
      "empty",
      "invalidUrl",
      "unsupportedScheme",
      "unsupportedHost",
      "userInfoNotAllowed",
      "customPortNotAllowed",
      "unsupportedTargetKind",
      "unresolvedShortLink",
    ];
    for (const code of codes) {
      const label = linkErrorVi(code);
      expect(label).not.toBe(code);
      expect(label.length).toBeGreaterThan(0);
    }
  });

  it("points a short link at the button that fixes it", () => {
    expect(linkErrorVi("unresolvedShortLink")).toContain("Gỡ link rút gọn");
  });
});

describe("state labels", () => {
  it("tells an unconfirmed send apart from a message that never typed anything", () => {
    // Both used to read "Chưa xác nhận", and they need opposite reactions: one may already
    // be public and is never retried, the other did nothing at all.
    expect(assignmentStateVi("uncertain")).not.toBe(assignmentStateVi("skippedParent"));
    expect(assignmentStateVi("uncertain")).toContain("chưa thấy lên");
  });

  it("gives a failure the tone that makes it stand out", () => {
    expect(stateTone("failed")).toBe("danger");
    expect(stateTone("succeeded")).toBe("ok");
    expect(stateTone("skippedParent")).toBe("warn");
    expect(stateTone("running")).toBe("info");
  });

  it("translates every campaign state", () => {
    for (const state of [
      "queued",
      "running",
      "succeeded",
      "partial",
      "failed",
      "cancelled",
    ]) {
      expect(campaignStateVi(state)).not.toBe(state);
    }
  });
});
