package com.riviu.agent;

import android.content.Context;
import android.util.Log;

import org.json.JSONObject;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;

/**
 * Loopback HTTP/1.1 for the helper protocol. Binds 127.0.0.1 only — the host
 * reaches it through {@code adb forward}, the same shape as minicap.
 */
final class HttpServer {
    private static final String TAG = "RiviuHelper";

    private final Context context;
    private final int port;
    private volatile boolean running;
    private ServerSocket server;
    private Thread thread;

    HttpServer(Context context, int port) {
        this.context = context.getApplicationContext();
        this.port = port;
    }

    synchronized void start() throws IOException {
        if (running) {
            return;
        }
        server = new ServerSocket(port, 8, InetAddress.getByName("127.0.0.1"));
        server.setSoTimeout(1000);
        running = true;
        thread = new Thread(this::acceptLoop, "riviu-helper-http");
        thread.start();
    }

    synchronized void stop() {
        running = false;
        if (server != null) {
            try {
                server.close();
            } catch (IOException ignored) {
            }
            server = null;
        }
        if (thread != null) {
            thread.interrupt();
            thread = null;
        }
    }

    private void acceptLoop() {
        while (running) {
            try {
                Socket socket = server.accept();
                try {
                    handle(socket);
                } finally {
                    try {
                        socket.close();
                    } catch (IOException ignored) {
                    }
                }
            } catch (SocketTimeoutException ignored) {
            } catch (IOException error) {
                if (running) {
                    Log.w(TAG, "accept failed", error);
                }
            }
        }
    }

    private void handle(Socket socket) throws IOException {
        socket.setSoTimeout(5000);
        InputStream raw = new BufferedInputStream(socket.getInputStream());
        OutputStream out = new BufferedOutputStream(socket.getOutputStream());
        String requestLine = readLine(raw);
        if (requestLine == null || requestLine.isEmpty()) {
            return;
        }
        String[] parts = requestLine.split(" ");
        if (parts.length < 2) {
            writeQuiet(out, 400, "bad_request", "request line is not HTTP");
            return;
        }
        String method = parts[0];
        String path = parts[1];
        int contentLength = 0;
        while (true) {
            String header = readLine(raw);
            if (header == null || header.isEmpty()) {
                break;
            }
            int colon = header.indexOf(':');
            if (colon < 0) {
                continue;
            }
            String name = header.substring(0, colon).trim();
            String value = header.substring(colon + 1).trim();
            if (name.equalsIgnoreCase("Content-Length")) {
                try {
                    contentLength = Integer.parseInt(value);
                } catch (NumberFormatException ignored) {
                    contentLength = -1;
                }
            }
        }
        if (contentLength < 0 || contentLength > Protocol.MAX_BODY_BYTES) {
            writeQuiet(out, 413, "too_large", "body exceeds " + Protocol.MAX_BODY_BYTES);
            return;
        }
        byte[] body = readExact(raw, contentLength);
        try {
            JSONObject response = route(method, path, body);
            write(out, response.optBoolean("ok", false) ? 200 : 400, response);
        } catch (IllegalArgumentException error) {
            try {
                write(out, 400, Protocol.error("invalid_argument", error.getMessage()));
            } catch (Exception ignored) {
            }
        } catch (Exception error) {
            Log.w(TAG, "route failed", error);
            try {
                write(out, 500, Protocol.error("internal", String.valueOf(error.getMessage())));
            } catch (Exception ignored) {
            }
        }
    }

    private JSONObject route(String method, String path, byte[] body) throws Exception {
        if ("GET".equals(method) && "/status".equals(path)) {
            return Protocol.status();
        }
        JSONObject json = body.length == 0 ? new JSONObject() : new JSONObject(new String(body, StandardCharsets.UTF_8));
        if ("POST".equals(method) && "/v1/clipboard/set".equals(path)) {
            ClipboardStore.setText(context, json.optString("text", ""));
            return Protocol.ok();
        }
        if ("POST".equals(method) && "/v1/clipboard/get".equals(path)) {
            return Protocol.ok().put("text", ClipboardStore.getText(context));
        }
        if ("POST".equals(method) && "/v1/media/import".equals(path)) {
            return MediaStoreImport.importFile(
                    context,
                    json.optString("relativePath", ""),
                    json.optString("displayName", ""));
        }
        if ("POST".equals(method) && "/v1/media/delete".equals(path)) {
            return MediaStoreImport.deleteById(context, json.optString("id", ""));
        }
        if ("POST".equals(method) && "/v1/wallpaper/set".equals(path)) {
            return WallpaperSet.setFromFile(context, json.optString("path", ""));
        }
        if ("POST".equals(method) && "/v1/location/set".equals(path)) {
            return MockLocation.set(context, json.optDouble("lat", 0), json.optDouble("lng", 0));
        }
        if ("POST".equals(method) && "/v1/location/stop".equals(path)) {
            return MockLocation.stop(context);
        }
        if ("POST".equals(method) && "/v1/apps/describe".equals(path)) {
            return AppList.describe(
                    context,
                    json.optJSONArray("packages"),
                    json.optBoolean("icons", true),
                    json.optInt("iconPx", 0));
        }
        return Protocol.error("not_found", method + " " + path);
    }

    private static void write(OutputStream out, int status, JSONObject json) throws Exception {
        byte[] payload = json.toString().getBytes(StandardCharsets.UTF_8);
        String reason = status == 200 ? "OK" : "Error";
        String header = "HTTP/1.1 " + status + " " + reason + "\r\n"
                + "Content-Type: application/json; charset=utf-8\r\n"
                + "Content-Length: " + payload.length + "\r\n"
                + "Connection: close\r\n\r\n";
        out.write(header.getBytes(StandardCharsets.US_ASCII));
        out.write(payload);
        out.flush();
    }

    private static void writeQuiet(OutputStream out, int status, String code, String message) {
        try {
            write(out, status, Protocol.error(code, message));
        } catch (Exception ignored) {
        }
    }

    private static String readLine(InputStream in) throws IOException {
        ByteArrayOutputStream line = new ByteArrayOutputStream();
        int previous = -1;
        while (true) {
            int next = in.read();
            if (next < 0) {
                if (line.size() == 0) {
                    return null;
                }
                break;
            }
            if (next == '\n') {
                break;
            }
            if (previous == '\r' && next != '\n') {
                line.write(previous);
            }
            if (next != '\r') {
                line.write(next);
            }
            previous = next;
            if (line.size() > 8192) {
                throw new IOException("header line exceeds 8192 bytes");
            }
        }
        return new String(line.toByteArray(), StandardCharsets.US_ASCII);
    }

    private static byte[] readExact(InputStream in, int length) throws IOException {
        byte[] body = new byte[length];
        int offset = 0;
        while (offset < length) {
            int read = in.read(body, offset, length - offset);
            if (read < 0) {
                throw new IOException("body ended after " + offset + " of " + length);
            }
            offset += read;
        }
        return body;
    }
}
