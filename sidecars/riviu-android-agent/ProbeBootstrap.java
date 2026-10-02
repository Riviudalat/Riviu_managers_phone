package com.riviu.agent;

import android.net.LocalSocket;
import android.net.LocalSocketAddress;
import android.system.ErrnoException;
import android.system.OsConstants;
import org.json.JSONObject;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/** Standalone read-only diagnostic DEX, never included in the helper APK. */
public final class ProbeBootstrap {
    private static final int MAX_BYTES = 4096;
    private static volatile String stage = "arguments";
    private static byte[] read(java.io.InputStream stream) throws IOException {
        DataInputStream input = new DataInputStream(stream);
        int size = input.readInt();
        if (size <= 0 || size > MAX_BYTES) throw new IOException();
        byte[] value = new byte[size]; input.readFully(value); return value;
    }
    private static void write(byte[] value) throws IOException {
        DataOutputStream output = new DataOutputStream(System.out);
        output.writeInt(value.length); output.write(value); output.flush();
    }
    private static void stage(String value) { stage = value; System.err.println("probe_stage=" + value); }
    private static LocalSocket connect(String name, AtomicReference<LocalSocket> active)
            throws IOException, InterruptedException {
        long deadline = System.nanoTime() + java.util.concurrent.TimeUnit.MILLISECONDS.toNanos(1500);
        while (true) {
            LocalSocket socket = new LocalSocket(); active.set(socket);
            try {
                socket.connect(new LocalSocketAddress(name, LocalSocketAddress.Namespace.ABSTRACT));
                return socket;
            } catch (IOException failure) {
                try { socket.close(); } catch (IOException ignored) {}
                if (System.nanoTime() - deadline >= 0) throw failure;
                Thread.sleep(50);
            }
        }
    }
    private static String causeKinds(Throwable failure) {
        StringBuilder kinds = new StringBuilder();
        Throwable current = failure;
        for (int depth = 0; current != null && depth < 8; depth++, current = current.getCause()) {
            if (depth != 0) kinds.append(',');
            if (current instanceof ErrnoException) kinds.append("ErrnoException");
            else if (current instanceof java.net.SocketTimeoutException) kinds.append("SocketTimeoutException");
            else if (current instanceof java.net.ConnectException) kinds.append("ConnectException");
            else if (current instanceof java.io.EOFException) kinds.append("EOFException");
            else if (current instanceof IOException) kinds.append("IOException");
            else if (current instanceof SecurityException) kinds.append("SecurityException");
            else kinds.append("Other");
        }
        return kinds.toString();
    }
    private static String errno(Throwable failure) {
        Throwable current = failure;
        for (int depth = 0; current != null && depth < 8; depth++, current = current.getCause()) {
            if (current instanceof ErrnoException) {
                int code = ((ErrnoException) current).errno;
                if (code == OsConstants.EACCES) return "EACCES";
                if (code == OsConstants.EPERM) return "EPERM";
                if (code == OsConstants.ENOENT) return "ENOENT";
                if (code == OsConstants.ECONNREFUSED) return "ECONNREFUSED";
                if (code == OsConstants.ECONNRESET) return "ECONNRESET";
                if (code == OsConstants.EPIPE) return "EPIPE";
                if (code == OsConstants.ETIMEDOUT) return "ETIMEDOUT";
                return "OTHER";
            }
        }
        // Android LocalSocket sometimes discards ErrnoException and exposes only IOException
        // text. Match fixed kernel phrases into fixed codes; never emit the message itself.
        current = failure;
        for (int depth = 0; current != null && depth < 8; depth++, current = current.getCause()) {
            if (current instanceof IOException) {
                String message = current.getMessage();
                if (message == null || message.length() > 512) continue;
                String lower = message.toLowerCase(java.util.Locale.ROOT);
                if (lower.contains("permission denied")) return "EACCES";
                if (lower.contains("operation not permitted")) return "EPERM";
                if (lower.contains("no such file or directory")) return "ENOENT";
                if (lower.contains("connection refused")) return "ECONNREFUSED";
                if (lower.contains("connection reset")) return "ECONNRESET";
                if (lower.contains("broken pipe")) return "EPIPE";
                if (lower.contains("timed out")) return "ETIMEDOUT";
            }
        }
        return "UNKNOWN";
    }
    public static void main(String[] args) {
        AtomicBoolean finished = new AtomicBoolean();
        AtomicReference<LocalSocket> active = new AtomicReference<LocalSocket>();
        Thread deadline = new Thread(() -> {
            try { Thread.sleep(7000); } catch (InterruptedException e) { return; }
            if (!finished.get()) {
                System.err.println("probe_timeout_stage=" + stage);
                LocalSocket socket = active.get();
                if (socket != null) try { socket.close(); } catch (IOException ignored) {}
                Runtime.getRuntime().halt(2);
            }
        }, "probe-deadline");
        deadline.setDaemon(true); deadline.start();
        byte[] payload = null, response = null;
        try {
            if (args.length == 1 && "--stdin-only".equals(args[0])) {
                stage("stdin_frame"); payload = read(System.in);
                byte[] hash = MessageDigest.getInstance("SHA-256").digest(payload);
                StringBuilder hex = new StringBuilder();
                for (byte b : hash) hex.append(String.format(java.util.Locale.ROOT, "%02x", b & 255));
                // Only nonsecret diagnostic payloads may use stdin-only: caller owns data scope.
                JSONObject result = new JSONObject().put("ok", true).put("length", payload.length).put("sha256", hex.toString());
                stage("stdout_frame"); write(result.toString().getBytes(StandardCharsets.UTF_8));
            } else {
                if (args.length != 2 || !args[0].matches("[A-Za-z0-9_.-]{16,100}")) throw new IOException();
                int expected = Integer.parseInt(args[1]);
                if (expected < 10000 || expected >= 100000) throw new IOException();
                stage("connect");
                LocalSocket socket = connect(args[0], active);
                socket.setSoTimeout(4000);
                stage("peer_uid");
                int observed = socket.getPeerCredentials().getUid();
                System.err.println("probe_expected_uid=" + expected + " probe_observed_uid=" + observed);
                if (observed != expected) throw new IOException();
                stage("stdin_frame"); payload = read(System.in);
                stage("request_validate");
                JSONObject request = new JSONObject(new String(payload, StandardCharsets.UTF_8));
                // This probe cannot send claim/release, token credentials or arbitrary fields.
                if (request.length() != 4 || !"inspect_invalidaction".equals(request.optString("action"))
                        || !"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".equals(request.optString("token"))
                        || !request.optString("nonce").matches("[A-Za-z0-9_-]{16,128}")
                        || !request.optString("ownerId").matches("[A-Za-z0-9_-]{16,128}")) throw new IOException();
                stage("request_send");
                DataOutputStream out = new DataOutputStream(socket.getOutputStream());
                out.writeInt(payload.length); out.write(payload); out.flush();
                stage("server_reply_frame"); response = read(socket.getInputStream());
                stage("server_reply_validate");
                JSONObject reply = new JSONObject(new String(response, StandardCharsets.UTF_8));
                if (reply.optBoolean("ok", true) || !"invalid_action".equals(reply.optString("error"))
                        || !"invalid_action".equals(reply.optString("state"))
                        || !request.optString("nonce").equals(reply.optString("nonce"))
                        || !request.optString("ownerId").equals(reply.optString("ownerId"))) throw new IOException();
                System.err.println("probe_binding_verified=true");
                // All echoed identities are nonsecret; no payload/error content is printed to stderr.
                stage("stdout_frame"); write(response);
            }
            stage("done");
        } catch (Throwable failure) {
            System.err.println("probe_failed_stage=" + stage + " probe_errno=" + errno(failure)
                    + " probe_causes=" + causeKinds(failure));
        }
        finally {
            finished.set(true); deadline.interrupt();
            if (payload != null) Arrays.fill(payload, (byte) 0);
            if (response != null) Arrays.fill(response, (byte) 0);
            LocalSocket socket = active.get();
            if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        }
    }
}
