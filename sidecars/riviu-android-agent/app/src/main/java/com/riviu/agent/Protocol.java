package com.riviu.agent;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * Wire contract for the helper HTTP server. The Rust client in
 * {@code crates/android-driver/src/riviu_agent.rs} parses the same JSON.
 * Changing a field name here without changing that file is a protocol break.
 */
final class Protocol {
    static final String AGENT_VERSION = "0.1.0";
    static final int PROTOCOL_VERSION = 1;
    static final int PORT = 17980;
    static final int MAX_BODY_BYTES = 64 * 1024;

    private Protocol() {}

    static JSONObject status() throws Exception {
        JSONArray features = new JSONArray();
        features.put("clipboard");
        features.put("pushMedia");
        return new JSONObject()
                .put("ok", true)
                .put("agentVersion", AGENT_VERSION)
                .put("protocolVersion", PROTOCOL_VERSION)
                .put("features", features);
    }

    static JSONObject ok() throws Exception {
        return new JSONObject().put("ok", true);
    }

    static JSONObject error(String code, String message) throws Exception {
        return new JSONObject()
                .put("ok", false)
                .put("error", code)
                .put("message", message);
    }
}
