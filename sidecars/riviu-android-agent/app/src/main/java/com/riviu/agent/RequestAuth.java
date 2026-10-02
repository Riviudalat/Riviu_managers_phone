package com.riviu.agent;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;

/** Public identification never authenticates a mutation or proves controller ownership. */
final class RequestAuth {
    static boolean tokenMatches(String configured, String presented) {
        return configured != null && !configured.isEmpty() && presented != null
                && MessageDigest.isEqual(presented.getBytes(StandardCharsets.UTF_8),
                        configured.getBytes(StandardCharsets.UTF_8));
    }
    static boolean allowed(String method, String path, String configured, String presented) {
        return ("GET".equals(method) && "/status".equals(path)) || tokenMatches(configured, presented);
    }
    static void requireId(String id) {
        if (id == null || !id.matches("[A-Za-z0-9_-]{16,128}")) {
            throw new IllegalArgumentException("invalid nonce or requestId");
        }
    }
}
