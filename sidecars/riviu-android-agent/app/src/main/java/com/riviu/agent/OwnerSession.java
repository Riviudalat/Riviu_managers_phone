package com.riviu.agent;

import java.util.UUID;

/** Independent owner identity, never derived from a public status response or token digest. */
final class OwnerSession {
    enum Claim { NEW, RESUME, CONFLICT }
    static boolean bindingMatches(String nonce, String owner, String suppliedNonce, String suppliedOwner) {
        return nonce.equals(suppliedNonce) && owner.equals(suppliedOwner);
    }
    private String owner;
    private String token;
    private String instance;
    private String generation;
    private boolean releasing;
    private int recoveries;
    synchronized boolean recover() {
        if (releasing || recoveries >= 3) return false;
        recoveries++;
        return true;
    }
    synchronized Claim claim(String requestedOwner, String requestedToken, boolean legacyOccupied) {
        if (legacyOccupied || releasing) return Claim.CONFLICT;
        if (owner != null) {
            return owner.equals(requestedOwner) && RequestAuth.tokenMatches(token, requestedToken)
                    ? Claim.RESUME : Claim.CONFLICT;
        }
        owner = requestedOwner;
        token = requestedToken;
        recoveries = 0;
        instance = UUID.randomUUID().toString();
        generation = UUID.randomUUID().toString();
        return Claim.NEW;
    }
    synchronized boolean release(String requestedOwner, String requestedToken, String requestedInstance) {
        if (owner == null || releasing || !owner.equals(requestedOwner)
                || !instance.equals(requestedInstance) || !RequestAuth.tokenMatches(token, requestedToken)) return false;
        releasing = true;
        return true;
    }
    synchronized boolean finishRelease(boolean settled) {
        if (!releasing || !settled) return false;
        owner = null; token = null; instance = null; generation = null; releasing = false;
        return true;
    }
    synchronized String owner() { return owner; }
    synchronized String instance() { return instance; }
    synchronized String generation() { return generation; }
    synchronized boolean releasing() { return releasing; }
}
