/**
 * Text distribution (feature A2, xiaowei "文字分发 / Text Distribution").
 *
 * Split one block of text into pieces and pair each piece to a phone, in the operator's
 * chosen order. Pure and tested here; the backend command `distribute_text` only applies the
 * pairing via `UiSession::type_text` (so it works on both Android and iOS).
 */

export type SplitMode =
  | { kind: "lines" }
  | { kind: "separator"; separator: string }
  | { kind: "regex"; pattern: string };

export interface TextAssignment {
  udid: string;
  text: string;
}

/** Drop pieces that are empty/whitespace-only — a blank line is never a message to send. */
function keepMeaningful(pieces: string[]): string[] {
  return pieces.filter((piece) => piece.trim().length > 0);
}

/**
 * Split raw text into per-phone pieces. `regex` may throw on an invalid pattern — the caller
 * shows that to the operator rather than sending a half-parsed batch.
 */
export function splitText(raw: string, mode: SplitMode): string[] {
  switch (mode.kind) {
    case "lines":
      return keepMeaningful(raw.split(/\r?\n/));
    case "separator":
      // An empty separator would explode into single characters — treat it as "whole block".
      if (mode.separator.length === 0) return keepMeaningful([raw]);
      return keepMeaningful(raw.split(mode.separator));
    case "regex":
      return keepMeaningful(raw.split(new RegExp(mode.pattern)));
  }
}

/**
 * Pair pieces to phones by index, up to the shorter of the two lists. Extra pieces and extra
 * phones are left unpaired — the UI reports the leftover so the mismatch is visible, not
 * silent (xiaowei distributes in ascending phone-number order; `udids` must already be in
 * that order).
 */
export function assign(items: string[], udids: string[]): TextAssignment[] {
  const n = Math.min(items.length, udids.length);
  const out: TextAssignment[] = [];
  for (let i = 0; i < n; i += 1) {
    out.push({ udid: udids[i], text: items[i] });
  }
  return out;
}

/** How many pieces / phones are left unpaired, for the preview's mismatch note. */
export function leftover(
  items: string[],
  udids: string[],
): { extraItems: number; extraDevices: number } {
  return {
    extraItems: Math.max(0, items.length - udids.length),
    extraDevices: Math.max(0, udids.length - items.length),
  };
}
