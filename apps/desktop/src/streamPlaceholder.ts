import type { TileStreamState } from "./types";

/// What to draw over a phone picture that is not showing one, and whether input can be sent.
///
/// One decision, shared by the grid tile and the control overlay, because they were making it
/// differently and both were wrong in their own way. The tile spun a loading mark forever on a
/// stream the codec had already refused; the overlay printed one flat string —
/// "Đang chờ stream…" — for every cause, offered nothing to press, and *silently swallowed
/// every click* while it was up.

export type StreamPlaceholder =
  | { kind: "none" }
  /// The app is bringing the stream up. Nothing for the operator to do but wait.
  | { kind: "loading" }
  /// It is not coming without intervention. `canRetry` is false when retrying provably
  /// cannot help, and then no retry control may be offered — a button that cannot succeed is
  /// worse than no button (AGENTS.md §9.58).
  | { kind: "failed"; reason: string; canRetry: boolean };

export interface StreamPlaceholderInput {
  /// A frame has been painted for this device recently.
  hasView: boolean;
  /// The encoded frame size is known. This — not `hasView` — is what the pointer handlers
  /// need, because they map screen coordinates through it.
  hasGeometry: boolean;
  /// Every codec candidate refused this stream.
  decodeFailed: boolean;
  tileStreamState?: TileStreamState;
  lastError?: string | null;
}

export interface StreamPlaceholderView {
  view: StreamPlaceholder;
  /// True when a gesture cannot be mapped to the phone and must not be attempted. The caller
  /// is expected to *say so* rather than drop the event: a click that does nothing and
  /// reports nothing is the complaint this whole helper exists to answer.
  blocksInput: boolean;
}

export function streamPlaceholder(input: StreamPlaceholderInput): StreamPlaceholderView {
  const blocksInput = !input.hasGeometry;

  if (input.decodeFailed) {
    return {
      // Deliberately not retryable. `decodeUnsupported` means every codec candidate was
      // tried and refused, so the same stream will be refused again; the fix is a different
      // encode or a different machine, neither of which a button here can reach.
      view: {
        kind: "failed",
        reason: "Trình giải mã của máy tính không đọc được luồng này.",
        canRetry: false,
      },
      blocksInput,
    };
  }

  const failure = input.lastError || (input.tileStreamState === "error" ? "Stream lỗi." : null);
  if (failure) {
    return { view: { kind: "failed", reason: failure, canRetry: true }, blocksInput };
  }

  // No frame yet and nothing has reported a failure: the keeper starts a producer for every
  // device it sees, so this is genuinely "on its way" rather than "idle, please press start".
  if (!input.hasView) {
    return { view: { kind: "loading" }, blocksInput };
  }

  return { view: { kind: "none" }, blocksInput };
}
