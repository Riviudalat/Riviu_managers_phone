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

/** The `adb_unreachable` latch reason in android-driver: a connection fault, not an owner to recover. */
const ADB_UNREACHABLE_MESSAGE = "Mất kết nối ADB với máy (offline hoặc không thấy máy); tự kiểm tra lại khi máy kết nối lại, chưa nhận phiên thay thế.";

/** `HELPER_SETUP_FAILURE` in android-driver: the roster's latched automatic-setup failure. */
const HELPER_SETUP_LATCH = "Riviu Helper: runtime qualification failed; recovery required";

/**
 * adbd shell-identity refusals, whichever boundary reports them: the driver's typed marker,
 * the prepare command's code, the latched roster category, or the already-described text.
 * The texts match `HelperShellIdentityRefused::operator_message` so describing twice is stable.
 */
const SHELL_IDENTITY_REFUSALS: { category: string; pattern: RegExp; message: string }[] = [
  {
    category: "shell_root",
    pattern: /HelperShellRootRefused:|^HelperMaintenanceShellRoot\b|^adb trên máy đang chạy quyền root \(uid 0\)/,
    message: "adb trên máy đang chạy quyền root (uid 0), thường do GenFarmer hoặc ROM bật; helper không chạy với root. Riviu tự tắt quyền root của adb một lần cho mỗi lần khởi động máy; nếu vẫn thấy lỗi này: tắt GenFarmer trên PC này rồi khởi động lại máy.",
  },
  {
    category: "shell_uid",
    pattern: /HelperShellUidRefused:|^HelperMaintenanceShellUid\b|^adb shell của máy không chạy bằng tài khoản shell/,
    message: "adb shell của máy không chạy bằng tài khoản shell (uid 2000); helper từ chối. Tắt rồi bật lại Gỡ lỗi USB trên máy, rồi chọn Khôi phục helper lại.",
  },
  {
    category: "android_user",
    pattern: /HelperAndroidUserRefused:|^HelperMaintenanceAndroidUser\b|^Máy đang ở người dùng Android khác người dùng chính/,
    message: "Máy đang ở người dùng Android khác người dùng chính (user 0) — helper chỉ chạy cho người dùng chính. Chuyển về người dùng chính trên máy, rồi Khôi phục helper lại.",
  },
];

/** Recognize retained helper ownership at the shared IPC error boundary. */
export function helperRecoveryMessage(cause: unknown): string | null {
  const message = typeof cause === "string" ? cause
    : cause instanceof Error ? cause.message
      : cause && typeof cause === "object"
        ? [Reflect.get(cause, "code"), Reflect.get(cause, "message")].filter(value => typeof value === "string").join(": ")
        : "";
  // Categories are read only from the roster latch: an unproved execution names one too
  // and keeps its own explanation.
  const latched = message.includes(HELPER_SETUP_LATCH);
  const category = latched ? /; category=([a-z_]+);/.exec(message)?.[1] : undefined;
  const identity = SHELL_IDENTITY_REFUSALS.find(refusal => refusal.category === category || refusal.pattern.test(message));
  if (identity) return identity.message;
  if (category === "adb_unreachable") return ADB_UNREACHABLE_MESSAGE;
  return /HelperRecoveryRequired|owner_conflict|helper_recovery_required/.test(message)
    || latched
    || message === HELPER_RECOVERY_MESSAGE ? HELPER_RECOVERY_MESSAGE : null;
}

/**
 * A cancelled call has no cause worth naming. TanStack Query rejects a cancelled read with
 * `CancelledError` as its whole message, and an aborted web API rejects with a DOMException
 * named `AbortError` and an English sentence; both used to reach the operator verbatim, as in
 * "Chưa xác nhận gán số máy: CancelledError".
 */
function isCancellation(cause: object): boolean {
  const name = Reflect.get(cause, "name");
  return name === "AbortError" || name === "CancelledError"
    || (cause instanceof Error && cause.message === "CancelledError");
}

const CANCELLED_MESSAGE = "Lượt này đã bị hủy trước khi có kết quả (thường do một lượt mới hơn thay thế).";

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
  if (typeof cause === "object" && isCancellation(cause)) return CANCELLED_MESSAGE;
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

/** What an error says about itself beyond its sentence: the typed half of an Activity entry. */
export interface ErrorFacts {
  /** A named code. Generic codes are left out so unrelated failures never group together. */
  code?: string;
  udid?: string;
  runId?: string;
}

/** Read the typed fields a `CommandError` payload carries; prose is never parsed for them. */
export function errorFacts(cause: unknown): ErrorFacts {
  const record = cause && typeof cause === "object" && !(cause instanceof Error)
    ? cause as Record<string, unknown> : null;
  const text = (key: string) => {
    const value = record?.[key];
    return typeof value === "string" && value.length > 0 ? value : undefined;
  };
  const named = text("code");
  const code = named && !GENERIC_CODES.has(named) ? named
    : helperRecoveryMessage(cause) ? "HelperRecoveryRequired" : undefined;
  return { code, udid: text("udid"), runId: text("runId") };
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
