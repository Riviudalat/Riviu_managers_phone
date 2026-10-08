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

const HELPER_RECOVERY_MESSAGE = "Helper còn giữ phiên cũ hoặc đang thuộc phiên khác; chưa thể xác minh điều khiển. Chọn Khôi phục helper trên đúng máy để xem và xác nhận kế hoạch. Không tự giành quyền hoặc gửi lại thao tác.";

/** Recognize retained helper ownership at the shared IPC error boundary. */
export function helperRecoveryMessage(cause: unknown): string | null {
  const message = typeof cause === "string" ? cause
    : cause instanceof Error ? cause.message
      : cause && typeof cause === "object"
        ? [Reflect.get(cause, "code"), Reflect.get(cause, "message")].filter(value => typeof value === "string").join(": ")
        : "";
  return /HelperRecoveryRequired|owner_conflict|helper_recovery_required/.test(message)
    || message === HELPER_RECOVERY_MESSAGE ? HELPER_RECOVERY_MESSAGE : null;
}

function readableMessage(message: string): string {
  const helper = helperRecoveryMessage(message);
  if (helper) return helper;
  if (message.includes("TikTok username is already assigned to another device")) {
    return "Tên người dùng TikTok đã được gán cho thiết bị khác; chưa lưu gán nick. Cần đối chiếu username vừa đọc với mapping đã lưu trên các máy. Không tự chuyển hoặc gán lại tài khoản khi chưa xác minh đúng máy.";
  }
  const readinessTimeout = message.match(/the agent on \S+ did not answer \/status within (\d+(?:\.\d+)?) seconds/);
  if (readinessTimeout) {
    return `Agent chưa phản hồi /status trong ${readinessTimeout[1]} giây; chưa xác định nguyên nhân. Mở Chẩn đoán của đúng thiết bị để kiểm tra kết nối và trạng thái Agent trước khi đọc lại tài khoản. Lỗi này chưa chứng minh tài khoản TikTok có vấn đề.`;
  }
  if (message.startsWith("DeviceControlFailed:")) return `Chưa điều khiển được thiết bị: ${message.slice("DeviceControlFailed:".length).trim()}`;
  if (message.includes("device_reconnect_timeout")) return "Máy mất kết nối quá 2 phút. Kết nối lại đúng điện thoại rồi bấm Thử lại.";
  if (/\bdevice offline\b/i.test(message)) {
    const serial = message.match(/\badb(?:\.exe)?\s+-s\s+([^\s;]+)/i)?.[1];
    return `ADB thấy ${serial ? `máy ${serial}` : "thiết bị"} nhưng đang offline. Kiểm tra nguồn box và cáp USB; đợi ADB báo device rồi thử lại.`;
  }
  if (/adb(?:\.exe)?.*device[^\n]*not found|no devices\/emulators found/i.test(message)) {
    const serial = message.match(/device ['"]([^'"]+)['"]/i)?.[1]
      ?? message.match(/\badb(?:\.exe)?\s+-s\s+([^\s;]+)/i)?.[1];
    return `ADB không thấy ${serial ? `máy ${serial}` : "thiết bị"}. Kiểm tra nguồn box và cáp USB; đợi máy xuất hiện ở trạng thái device rồi thử lại.`;
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
  const helper = helperRecoveryMessage(cause);
  if (helper) return helper;
  if (cause === null || cause === undefined) return "Lỗi không rõ nguyên nhân";
  if (typeof cause === "string") return readableMessage(cause);
  if (cause instanceof Error) return readableMessage(cause.message);
  if (typeof cause === "object") {
    const record = cause as Record<string, unknown>;
    if (record.code === "AccountAssignmentConflict" && record.accountConflict
      && typeof record.accountConflict === "object") {
      const conflict = record.accountConflict as Record<string, unknown>;
      if (typeof conflict.attemptedHandle === "string" && typeof conflict.currentHandle === "string"
        && Array.isArray(conflict.conflictingDevices)) {
        const devices = conflict.conflictingDevices.flatMap((value: unknown) => {
          if (!value || typeof value !== "object") return [];
          const device = value as Record<string, unknown>;
          if (typeof device.udid !== "string") return [];
          const number = typeof device.number === "number" && device.number > 0 ? `Máy ${device.number}` : "";
          const alias = typeof device.alias === "string" ? device.alias.trim() : "";
          const label = [number, alias].filter(Boolean).join(" · ");
          return [label ? `${label} (${device.udid})` : device.udid];
        });
        const saved = conflict.currentHandle.trim().replace(/^@+/, "");
        return `Chưa lưu gán @${conflict.attemptedHandle}: trùng mapping đã lưu trên ${devices.join(", ") || "thiết bị khác"}${conflict.conflictsTruncated === true ? " và các máy khác (danh sách giới hạn 20 máy)" : ""}. `
          + `Mapping của máy đích vẫn là ${saved ? `@${saved}` : "chưa gán"}. `
          + "Đây là mapping đã lưu; chưa xác minh tài khoản đang đăng nhập trên các máy trùng. "
          + "Đối chiếu đúng máy và username trước khi sửa mapping; không tự chuyển hoặc đổi tài khoản.";
      }
    }
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

/** Shared typed ownership boundary, with one narrow legacy-string compatibility parser. */
export function controlFailure(cause: unknown) {
  const detail = describeError(cause);
  const record = cause && typeof cause === "object" ? cause as Record<string, unknown> : null;
  const legacy = /^DeviceBusy:.*? is busy with ([A-Za-z]+);/.exec(detail)?.[1];
  const owner = record?.code === "DeviceBusy" && typeof record.currentOwner === "string"
    ? record.currentOwner.toLowerCase() : legacy?.toLowerCase();
  const helper = helperRecoveryMessage(cause) !== null;
  const canHandoff = !helper && ["script", "nurture", "interaction"].includes(owner ?? "");
  const summary = helper ? "Helper cần khôi phục"
    : owner === "script" ? "Tác vụ tự động đang giữ máy"
    : owner === "nurture" ? "Đang nuôi tài khoản"
    : owner === "interaction" ? "Đang tương tác"
    : owner ? "Máy đang có phiên điều khiển"
    : /\/elements\b/.test(detail) && /timed out|timeout/i.test(detail) ? "Hết thời gian đọc giao diện"
    : "Chưa thể điều khiển";
  return { owner, canHandoff, helper, summary, detail, transientIdleSweep: !helper && owner === "idlesweep" };
}
