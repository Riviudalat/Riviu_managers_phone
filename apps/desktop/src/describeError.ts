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
  if (typeof cause === "string") return cause;
  if (cause instanceof Error) return cause.message;
  if (typeof cause === "object") {
    const record = cause as Record<string, unknown>;
    const message = record.message ?? record.error ?? record.detail;
    if (typeof message === "string" && message.length > 0) {
      const code = typeof record.code === "string" ? `${record.code}: ` : "";
      return `${code}${message}`;
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
