/**
 * The phone's keyboards, read out of `ime list -s`.
 *
 * Measured format, SM-G955F (Android 9) — one id per line, nothing else:
 *
 * ```text
 * com.android.xwkeyboard/.XwIME
 * com.genfarmer.uiautomator/.AdbKeyboard
 * com.sec.android.inputmethod/.SamsungKeypad
 * ```
 *
 * Two rules this file exists to enforce, both of which are easy to get wrong in a click
 * handler and impossible to get wrong here:
 *
 * 1. **Only ever offer an id the phone itself just printed.** The chosen value is
 *    interpolated into `ime set <id>` on a real shell, and the validator for that
 *    (`validate_ime_id`) lives in Rust with no TypeScript equivalent. Parsing the phone's
 *    own output and choosing from that list means the string never originates here.
 * 2. **Never offer the Riviu helper IME.** `com.riviu.agent/.RiviuIme` is switched in for a
 *    single clipboard call and switched straight back out; leaving it as the phone's
 *    keyboard is GenFarmer's mark and this project rules it out (AGENTS.md §9.52). A picker
 *    that lists it can violate that with one click.
 */

/// The helper's IME id, mirrored from `riviu_agent.rs`.
export const HELPER_IME_ID = "com.riviu.agent/.RiviuIme";

export interface InputMethod {
  id: string;
  /// The class name without its package, which is what an operator recognises:
  /// `SamsungKeypad` rather than `com.sec.android.inputmethod/.SamsungKeypad`.
  label: string;
}

/// An id shaped like `package/.Class` or `package/fully.qualified.Class`.
///
/// Deliberately strict and deliberately duplicated from the Rust validator: anything that
/// does not match is dropped rather than shown, so a line of unexpected output — an error,
/// a warning, a localised "no input methods" — can never become a shell argument.
function isPlausibleImeId(value: string): boolean {
  if (!value || value.length > 255) return false;
  if (/[\s"'`$;&|<>(){}\\]/.test(value)) return false;
  const parts = value.split("/");
  if (parts.length !== 2) return false;
  const [pkg, cls] = parts;
  if (!/^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$/.test(pkg)) return false;
  const className = cls.startsWith(".") ? cls.slice(1) : cls;
  return /^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)*$/.test(className);
}

export function parseInputMethods(stdout: string): InputMethod[] {
  const seen = new Set<string>();
  const methods: InputMethod[] = [];
  for (const raw of stdout.split(/\r?\n/)) {
    const id = raw.trim();
    if (!isPlausibleImeId(id)) continue;
    if (id === HELPER_IME_ID) continue;
    if (seen.has(id)) continue;
    seen.add(id);
    const cls = id.split("/")[1];
    const label = (cls.startsWith(".") ? cls.slice(1) : cls).split(".").pop() ?? id;
    methods.push({ id, label });
  }
  return methods;
}

/// `settings get secure default_input_method` prints the id and nothing else, or `null` when
/// no keyboard is selected. Anything unrecognised answers `null` rather than guessing.
export function parseCurrentInputMethod(stdout: string): string | null {
  const id = stdout.trim();
  return isPlausibleImeId(id) ? id : null;
}
