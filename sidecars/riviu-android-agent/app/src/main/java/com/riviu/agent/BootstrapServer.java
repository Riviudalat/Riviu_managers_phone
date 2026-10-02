package com.riviu.agent;

import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.Handler;
import android.system.Os;
import android.system.OsConstants;
import android.system.StructPollfd;
import org.json.JSONObject;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/** One-shot bounded abstract socket. No HTTP path can claim/release ownership. */
final class BootstrapServer {
    interface Reply { void send(JSONObject value); }
    interface HandlerAction { void handle(JSONObject payload, Reply reply); }
    private final LocalServerSocket listener;
    private volatile LocalSocket accepted;
    private volatile boolean closed;
    BootstrapServer(String name, String nonce, String owner, boolean debuggable,
            Handler main, HandlerAction action, Runnable onClosed) throws IOException {
        listener = new LocalServerSocket(name); // Collision must fail; never connect/take over another listener.
        final long absoluteDeadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(4500);
        new Thread(() -> {
            // Only close accepted IO at deadline; listener acceptance is bounded by poll.
            try { Thread.sleep(4500); } catch (InterruptedException ignored) { return; }
            closed = true;
            LocalSocket socket = accepted;
            if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        }, "riviu-bootstrap-io-deadline").start();
        new Thread(() -> {
            byte[] bytes = null;
            try {
                StructPollfd poll = new StructPollfd();
                poll.fd = listener.getFileDescriptor();
                poll.events = (short) OsConstants.POLLIN;
                if (!AcceptDeadline.await(timeout -> {
                    poll.revents = 0;
                    int result = Os.poll(new StructPollfd[] { poll }, timeout);
                    if ((poll.revents & (OsConstants.POLLERR | OsConstants.POLLHUP | OsConstants.POLLNVAL)) != 0) {
                        throw new IOException("listener unavailable");
                    }
                    return result > 0 && (poll.revents & OsConstants.POLLIN) != 0;
                }, System::nanoTime, () -> closed,
                        Math.max(0, TimeUnit.NANOSECONDS.toMillis(absoluteDeadline - System.nanoTime())))) return;
                LocalSocket socket = listener.accept(); // Sole accepter; only after readiness and deadline check.
                accepted = socket;
                if (closed) return;
                long remaining = absoluteDeadline - System.nanoTime();
                if (remaining <= 0) return;
                socket.setSoTimeout((int) Math.max(1, Math.min(3500, TimeUnit.NANOSECONDS.toMillis(remaining))));
                if (!BootstrapBridge.acceptedCaller(socket, debuggable)) throw new IOException("caller rejected");
                bytes = BootstrapEnvelope.readBoundedDeadline(socket, absoluteDeadline);
                JSONObject payload = new JSONObject(new String(bytes, StandardCharsets.UTF_8));
                if (!OwnerSession.bindingMatches(nonce, owner, payload.optString("nonce", ""), payload.optString("ownerId", ""))) {
                    throw new IOException("binding rejected");
                }
                String token = payload.optString("token", "");
                if (!token.matches("[A-Za-z0-9_-]{32,256}")) throw new IOException("credential rejected");
                CountDownLatch done = new CountDownLatch(1);
                AtomicReference<JSONObject> response = new AtomicReference<JSONObject>();
                if (!AcceptDeadline.mayDispatch(System::nanoTime, () -> closed, absoluteDeadline)) return;
                if (!main.post(() -> {
                    if (!AcceptDeadline.mayDispatch(System::nanoTime, () -> closed, absoluteDeadline)) {
                        done.countDown(); return;
                    }
                    action.handle(payload, value -> { response.set(value); done.countDown(); });
                })) throw new IOException("dispatch rejected");
                long replyRemaining = absoluteDeadline - System.nanoTime();
                if (replyRemaining <= 0 || !done.await(Math.min(TimeUnit.MILLISECONDS.toNanos(3500), replyRemaining),
                        TimeUnit.NANOSECONDS) || response.get() == null) throw new IOException("settlement unknown");
                BootstrapEnvelope.write(socket.getOutputStream(), response.get().toString().getBytes(StandardCharsets.UTF_8));
            } catch (Exception ignored) {
                // Never serialize payload/exception messages containing credentials.
            } finally {
                if (bytes != null) Arrays.fill(bytes, (byte) 0);
                close();
                main.post(onClosed);
            }
        }, "riviu-bootstrap-server").start();
    }
    void close() {
        closed = true;
        LocalSocket socket = accepted;
        if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        try { listener.close(); } catch (IOException ignored) {}
    }
}
