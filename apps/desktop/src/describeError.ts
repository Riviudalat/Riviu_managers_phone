/**
 * Codes that say nothing the message does not already say.
 *
 * `CommandError::operation` stamps `OperationFailed` on anything without a more specific
 * cause, which is most errors — it is the absence of a code, spelled as one. Printing it
 * would put "OperationFailed: " in front of every sentence the operator reads. Named codes
 * like `DeviceBusy` do earn their place, because they are the difference between "try again"
 * and "something is wrong".
 */
const GENERIC_CODES = new Set(["OperationFailed"]);

function readableMessage(message: string): string {
  if (message.includes("device_reconnect_timeout")) return "Máy mất kết nối quá 2 phút. Kết nối lại đúng điện thoại rồi bấm Thử lại.";
  if (/adb(?:\.exe)?.*device[^\n]*not found/i.test(message) || /device offline|no devices\/emulators found/i.test(message)) {
    const serial=message.match(/device ['"]([^'"]+)['"]/i)?.[1];
    return `Không tìm thấy ${serial ? `máy ${serial}` : "thiết bị"} trong ADB. Kiểm tra kết nối USB; tác vụ đăng sẽ chờ đúng máy kết nối lại tối đa 2 phút.`;
  }
  if (message.includes("typesafe_http_401")) {
    return "Khóa TypeSafe không được chấp nhận (401). Kiểm tra khóa của tài khoản TypeSafe, bấm Lưu khóa TypeSafe rồi kiểm tra lại. Lượt bình luận chưa được gửi.";
  }
  return message;
}

/**
 * One line of text for anything that can be thrown or rejected.
 *
 * Written because `String(error)` is wrong for the single most common failure in this app: a
 * Tauri command rejects with a plain object, `{ code, message }`, and `String` on that yields
 * **`[object Object]`**. That is what the operator read instead of "Permission denied" when a
 * folder was refused, and it is silent — nothing throws, the message just says nothing.
 *
 * It lives in its own module, apart from the toast store that first needed it, so a pure
 * module (`liveDrag`, `flow/validation`) can normalise an error without importing a React
 * store to do it.
 */
export function describeError(cause: unknown): string {
  if (cause === null || cause === undefined) return "Lỗi không rõ nguyên nhân";
  if (typeof cause === "string") return readableMessage(cause);
  if (cause instanceof Error) return readableMessage(cause.message);
  if (typeof cause === "object") {
    const record = cause as Record<string, unknown>;
    if (record.code === "DeviceAppSelectionRequired") {
      const device = typeof record.udid === "string" ? ` ${record.udid}` : "";
      return `Máy${device} có nhiều ứng dụng TikTok. Mở Chi tiết thiết bị, chọn ứng dụng cần dùng rồi kiểm tra lại.`;
    }
    const message = record.message ?? record.error ?? record.detail;
    if (typeof message === "string" && message.length > 0) {
      const named = typeof record.code === "string" && !GENERIC_CODES.has(record.code);
      return readableMessage(named ? `${record.code as string}: ${message}` : message);
    }
    // No field worth naming. JSON at least carries what came back, where `String` would
    // throw away the whole payload and print `[object Object]`.
    try {
      return JSON.stringify(cause);
    } catch {
      return String(cause);
    }
  }
  return String(cause);
}
