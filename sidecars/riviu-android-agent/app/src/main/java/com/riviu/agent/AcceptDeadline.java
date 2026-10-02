package com.riviu.agent;

import java.util.concurrent.TimeUnit;

/** Pure admission deadline seam: poll, recheck deadline, and only then enter ready accept. */
final class AcceptDeadline {
    interface Poller { boolean ready(int timeoutMillis) throws Exception; }
    interface Clock { long now(); }
    interface Closed { boolean get(); }
    static boolean mayDispatch(Clock clock, Closed closed, long deadline) {
        return !closed.get() && clock.now() - deadline < 0;
    }
    static boolean await(Poller poller, Clock clock, Closed closed, long durationMillis) throws Exception {
        long deadline = clock.now() + TimeUnit.MILLISECONDS.toNanos(durationMillis);
        while (!closed.get()) {
            long remaining = deadline - clock.now();
            if (remaining <= 0) return false;
            int wait = (int) Math.min(100, Math.max(1, TimeUnit.NANOSECONDS.toMillis(remaining)));
            boolean ready = poller.ready(wait);
            // Late readiness never dispatches work after the deadline or admission close.
            if (closed.get() || clock.now() - deadline >= 0) return false;
            if (ready) return true;
        }
        return false;
    }
}
