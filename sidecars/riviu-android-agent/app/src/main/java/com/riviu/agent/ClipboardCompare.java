package com.riviu.agent;

/** One dispatched conditional restore job, not a cross-process Android clipboard CAS. */
final class ClipboardCompare {
    interface Access { String read(); void write(String text); }
    static final class Result {
        final boolean available;
        final boolean written;
        final boolean changed;
        final boolean verified;
        Result(boolean available, boolean written, boolean changed, boolean verified) {
            this.available = available; this.written = written; this.changed = changed; this.verified = verified;
        }
    }
    static Result apply(Access clipboard, String expected, String replacement) {
        String current = clipboard.read();
        if (current == null) return new Result(false, false, false, false);
        if (!current.equals(expected)) return new Result(true, false, true, false);
        clipboard.write(replacement); // Exactly one attempt; lost write ACK is never replayed.
        String after = clipboard.read();
        return new Result(after != null, true, false, replacement.equals(after));
    }
}
