package com.riviu.agent;

/** Legacy token authenticates requests, not takeover ownership. Pure production decision seam. */
final class ServiceStartPolicy {
    enum Decision { WAIT, KEEP, START, RECOVER, REFUSE_OWNER, REFUSE_BUSY, REFUSE_RECOVERY }
    private int recoveries;
    Decision decide(String token, String activeToken, boolean hasServer,
            boolean listenerLive, boolean busy) {
        if (token == null || token.isEmpty()) return Decision.WAIT;
        if (activeToken == null) return Decision.START;
        if (!token.equals(activeToken)) return Decision.REFUSE_OWNER;
        if (listenerLive) return Decision.KEEP;
        if (busy) return Decision.REFUSE_BUSY;
        if (recoveries >= 3) return Decision.REFUSE_RECOVERY;
        recoveries++;
        return Decision.RECOVER;
    }
}
