import { describe, expect, it } from "vitest";

import { describeError } from "./describeError";

/**
 * The bug this module exists for, pinned.
 *
 * Every Tauri command in this app rejects with a plain `{ code, message }` object, and
 * `String` on one of those yields `[object Object]`. That is what an operator read when a
 * folder was refused — not "Permission denied" — and nothing threw, so nothing pointed at it.
 * The same call was open-coded in six files at various depths of correctness before it moved
 * here.
 */
describe("describeError", () => {
  it("does not promise publish reconnect for an offline preview", () => {
    const message = describeError({
      code: "OperationFailed",
      message: "adb -s ce11171beb408a1501 get-state failed: error: device offline",
    });
    expect(message).toContain("ADB thấy máy ce11171beb408a1501 nhưng đang offline");
    expect(message).not.toContain("tác vụ đăng");
    expect(message).not.toContain("2 phút");
  });
  it("explains a TypeSafe rejected key without suggesting a device retry", () => {
    for (const error of ["typesafe_http_401", new Error("typesafe_http_401"), { code: "OperationFailed", message: "typesafe_http_401" }]) {
      expect(describeError(error)).toContain("Khóa TypeSafe không được chấp nhận");
      expect(describeError(error)).toContain("Lưu khóa TypeSafe");
    }
  });
  it("reads a Tauri rejection, which is the case String() gets wrong", () => {
    expect(describeError({ code: "DeviceBusy", message: "máy đang bận" })).toBe(
      "DeviceBusy: máy đang bận",
    );
    // No code, just a reason.
    expect(describeError({ message: "Permission denied" })).toBe("Permission denied");
    // The two other field names the Rust side has used.
    expect(describeError({ error: "không kết nối được" })).toBe("không kết nối được");
    expect(describeError({ detail: "adb offline" })).toBe("adb offline");
  });

  it("turns an ambiguous TikTok package into an actionable per-device instruction", () => {
    expect(describeError({
      code: "DeviceAppSelectionRequired",
      udid: "dual-phone",
      message: "resolveTikTokPackage failed",
    })).toBe("Máy dual-phone có nhiều ứng dụng TikTok. Mở Chi tiết thiết bị, chọn ứng dụng cần dùng rồi kiểm tra lại.");
  });

  it("never answers [object Object], even for a shape it does not know", () => {
    // The whole point: an unrecognised payload still carries what came back. `String` here
    // would throw the payload away and print the string that started all this.
    const answer = describeError({ udid: "10969614", exit: 1 });
    expect(answer).not.toContain("[object Object]");
    expect(answer).toBe('{"udid":"10969614","exit":1}');
  });

  it("survives a payload JSON cannot serialise", () => {
    const cyclic: Record<string, unknown> = { message: 7 };
    cyclic.self = cyclic;
    // Falls through to String rather than throwing inside an error handler — a throw here
    // would replace the operator's error message with a different one.
    expect(() => describeError(cyclic)).not.toThrow();
  });

  it("passes Error and string throwables straight through", () => {
    expect(describeError(new Error("boom"))).toBe("boom");
    expect(describeError("plain")).toBe("plain");
  });

  it("has something to say when there is nothing at all", () => {
    // `throw undefined` is legal JavaScript and a rejected promise can carry nothing.
    expect(describeError(null)).toBe("Lỗi không rõ nguyên nhân");
    expect(describeError(undefined)).toBe("Lỗi không rõ nguyên nhân");
  });

  it("does not mistake an empty message for a message", () => {
    // An empty string would render a blank error box: technically a message, useless as one.
    expect(describeError({ code: "Io", message: "" })).toBe('{"code":"Io","message":""}');
  });
});

describe("codes the operator should not have to read", () => {
  it("drops OperationFailed, which is the absence of a code spelled as one", () => {
    // Every command that used to return a plain String now returns
    // `CommandError::operation(...)`, so without this the whole app would grow a
    // "OperationFailed: " prefix on messages that read fine on their own.
    expect(
      describeError({ code: "OperationFailed", message: "không mở được thư mục" }),
    ).toBe("không mở được thư mục");
  });

  it("keeps a code that tells the operator something the message does not", () => {
    expect(describeError({ code: "DeviceBusy", message: "máy đang bận" })).toBe(
      "DeviceBusy: máy đang bận",
    );
  });
});
