import { describe, expect, it } from "vitest";
import {
  HELPER_IME_ID,
  parseCurrentInputMethod,
  parseInputMethods,
} from "./imeList";

/// Copied verbatim from `adb -s <serial> shell ime list -s` on SM-G955F, Android 9.
const REAL_OUTPUT = `com.android.xwkeyboard/.XwIME
com.genfarmer.uiautomator/.AdbKeyboard
com.sec.android.inputmethod/.SamsungKeypad
`;

describe("keyboards on the phone", () => {
  it("reads the measured output of ime list -s", () => {
    expect(parseInputMethods(REAL_OUTPUT)).toEqual([
      { id: "com.android.xwkeyboard/.XwIME", label: "XwIME" },
      { id: "com.genfarmer.uiautomator/.AdbKeyboard", label: "AdbKeyboard" },
      { id: "com.sec.android.inputmethod/.SamsungKeypad", label: "SamsungKeypad" },
    ]);
  });

  it("never offers the Riviu helper keyboard", () => {
    // It is switched in for one clipboard call and switched straight back out. Leaving it
    // as the phone's keyboard is the thing this project rules out, and a picker that lists
    // it can do that with a single click.
    const methods = parseInputMethods(
      `${HELPER_IME_ID}\ncom.sec.android.inputmethod/.SamsungKeypad\n`,
    );
    expect(methods.map((method) => method.id)).toEqual([
      "com.sec.android.inputmethod/.SamsungKeypad",
    ]);
  });

  it("drops anything that is not shaped like an id, so it cannot reach a shell", () => {
    // The chosen value is interpolated into `ime set <id>` on a real shell and the
    // validator for that lives in Rust. Everything here is a line `ime list` could
    // plausibly emit on some phone, or a line an attacker would want it to.
    const methods = parseInputMethods(
      [
        "Error: no input methods",
        "com.evil/.X; reboot",
        "com.evil/.X && rm -rf /",
        "$(reboot)",
        "com.evil/.X`reboot`",
        "no-slash-at-all",
        "too/many/slashes",
        "/.LeadingSlash",
        "com.ok/.Fine",
      ].join("\n"),
    );
    expect(methods.map((method) => method.id)).toEqual(["com.ok/.Fine"]);
  });

  it("keeps the list stable when a phone repeats an id", () => {
    const methods = parseInputMethods("com.ok/.Fine\ncom.ok/.Fine\n");
    expect(methods).toHaveLength(1);
  });

  it("reads the current keyboard, and refuses a value it does not recognise", () => {
    expect(parseCurrentInputMethod("com.genfarmer.uiautomator/.AdbKeyboard\n")).toBe(
      "com.genfarmer.uiautomator/.AdbKeyboard",
    );
    // `settings get` prints this literal when nothing is set.
    expect(parseCurrentInputMethod("null\n")).toBeNull();
    expect(parseCurrentInputMethod("")).toBeNull();
  });
});
