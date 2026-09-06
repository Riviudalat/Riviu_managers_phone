import { describe, expect, it } from "vitest";
import {
  assignmentStateVi,
  campaignStateVi,
  interactionErrorVi,
  linkErrorVi,
  stateTone,
} from "./interactionErrors";

describe("interactionErrorVi", () => {
  it.each([
    "target_post_unavailable",
    "like_action_stopped: target re-proof before Like failed: target_exact_open: exact post proof failed: target_post_unavailable: TikTok reported 'This video is unavailable'",
  ])("names TikTok's unavailable response without inferring deletion or an account block: %s", (raw) => {
    const result = interactionErrorVi(raw);
    expect(result.title).toBe("TikTok báo bài không khả dụng trên máy này");
    expect(result.detail).toContain("chưa xác định bài đã bị xóa hay tài khoản bị chặn");
    expect(result.detail).toContain("đúng tài khoản trên máy đó");
    if (raw.includes("This video")) expect(result.detail).toContain("TikTok reported 'This video is unavailable'");
    expect(result.raw).toBe(raw);
  });

  it.each([
    "target_package_unreadable",
    "target_exact_open: target_package_unreadable: cmd: Can't find service: package",
  ])("distinguishes unreadable Android package services from an uninstalled app: %s", (raw) => {
    const result = interactionErrorVi(raw);
    expect(result.title).toBe("Chưa đọc được ứng dụng TikTok trên máy");
    expect(result.detail).toContain("dịch vụ hệ thống Android");
    expect(result.detail).toContain("không có nghĩa TikTok chưa được cài");
    if (raw.includes("Can't find service")) expect(result.detail).toContain("Can't find service: package");
    expect(result.raw).toBe(raw);
  });

  it("keeps expected and observed links in the exact-target mismatch diagnostic", () => {
    const expected = "https://www.tiktok.com/@fixture.expected/photo/111";
    const observed = "https://www.tiktok.com/@fixture.other/video/222";
    const raw = `like_action_stopped: target re-proof before Like failed: target_exact_open: exact post proof failed: target_link_proof: copied link belongs to a different post; expected=${expected}; observed=${observed}`;
    const result = interactionErrorVi(raw);
    expect(result.title).toBe("Chưa xác minh được đúng bài bằng liên kết");
    expect(result.title).not.toContain("https://");
    expect(result.detail).toContain(`expected=${expected}; observed=${observed}`);
    expect(result.detail).toContain("copied link belongs to a different post");
    expect(result.raw).toBe(raw);
  });

  it.each([
    "foreground: chưa đọc được ứng dụng đang mở",
    "Comments: measured control missing or unreadable",
    "author: label missing or unreadable",
    "caption: measured identity missing, empty, ambiguous or unreadable",
  ])("retains the exact-open measurement failure in details: %s", (cause) => {
    const raw = `target_exact_open: no readable post in the target app within 14 seconds; last check: ${cause}`;
    const result = interactionErrorVi(raw);
    expect(result.title).toBe("Chưa mở và xác minh được đúng bài");
    expect(result.detail).toContain(cause);
    expect(result.raw).toBe(raw);
  });

  it("renders actual campaign totals instead of claiming the failure is unknown", () => {
    const raw = "xong 11, lỗi 9, còn dở 0";
    expect(interactionErrorVi(raw)).toEqual({ title: "11 lượt hoàn tất, 9 lượt lỗi, 0 lượt chưa xong", raw });
    expect(interactionErrorVi("Like không an toàn để tiếp tục assignment").title).toContain("bước Tim");
  });
  it("preserves the precise proof cause when the assignment stops before Like", () => {
    expect(interactionErrorVi("like_action_stopped: target re-proof before Like failed: target_open_no_baseline: tác giả chưa đọc được").title).toBe("Không đọc được bài đang mở trước đó");
    expect(interactionErrorVi("target_exact_open: link mismatch").title).toBe("Chưa mở và xác minh được đúng bài");
    expect(interactionErrorVi("skipped_after_assignment_stopped: Like đã dừng").title).toBe("Chưa thực hiện vì lượt trên máy đã dừng");
  });
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

  it("finds the code inside an anyhow chain, not only at the head", () => {
    // `interaction_commands.rs` writes the campaign-level reason as `format!("{error:#}")`, and
    // an anyhow chain renders outermost-first — so the head is context and the code is one
    // segment in. Splitting only at the head printed the same failure two ways in one panel:
    // the assignment row read "AI không viết được bình luận" and the campaign row above it
    // printed the whole chain untranslated.
    const view = interactionErrorVi(
      "AI chuẩn bị assignment 0: ai_comment_unavailable: ordinal 0 — comment_context_rejected: context=0 overall=0",
    );
    expect(view.title).toBe("AI không viết được bình luận");
    expect(view.detail).toContain("AI chuẩn bị assignment 0");
    expect(view.detail).toContain("comment_context_rejected");
  });

  it("translates the planner's own refusals, which arrive in English", () => {
    // `plan_threads` runs `validate()` and the desktop wraps its `Display` impl as
    // `CommandError::code("InteractionFailed", …)`. `describeError` keeps named codes, so this
    // is exactly the string that used to be pushed into the reasons list verbatim — in a panel
    // whose whole premise is that raw codes never reach the operator.
    expect(
      interactionErrorVi(
        "InteractionFailed: message count must cover every selected actor",
      ).title,
    ).toBe("Số bình luận phải đủ cho cụm lớn nhất");
    expect(
      interactionErrorVi(
        "InteractionFailed: manual mode needs at least as many comments as there are messages",
      ).title,
    ).toBe("Danh sách bình luận ít hơn số bình luận cần gửi");
  });

  it("does not read an ordinal it cannot trust", () => {
    // `message_ordinal` is a `u8` capped at 63, so three digits is already generous. An
    // unbounded `\d+` through `Number` makes the `+ 1` a silent no-op past 2^53 and renders
    // "bình luận thứ 1e+21" past 1e21.
    const view = interactionErrorVi(
      "parent_identity_not_confirmed_at_ordinal_99999999999999999999",
    );
    expect(view.title).toBe("Lỗi tương tác chưa xác định");
    // …while a real one still reads as a one-based position.
    expect(
      interactionErrorVi("parent_identity_not_confirmed_at_ordinal_2").title,
    ).toContain("thứ 3");
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

  it("gives an unrecognised reason a Vietnamese headline and retains the raw diagnostic", () => {
    const raw = "AI API key chưa được cấu hình cho Interaction";
    expect(interactionErrorVi(raw)).toEqual({
      title: "Lỗi tương tác chưa xác định",
      raw,
    });
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
