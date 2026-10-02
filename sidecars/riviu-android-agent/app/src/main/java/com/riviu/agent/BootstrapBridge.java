package com.riviu.agent;

import android.net.LocalSocket;
import android.net.LocalSocketAddress;

import java.io.IOException;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/** app_process bridge. Only socket name and controller-checked user-0 package UID are arguments. */
public final class BootstrapBridge {
    private BootstrapBridge() {}
    public static void main(String[] args) {
        AtomicReference<LocalSocket> connected = new AtomicReference<LocalSocket>();
        AtomicBoolean finished = new AtomicBoolean();
        Thread deadline = new Thread(() -> {
            try { Thread.sleep(5000); } catch (InterruptedException ignored) { return; }
            if (!finished.get()) {
                LocalSocket socket = connected.get();
                if (socket != null) try { socket.close(); } catch (IOException ignored) {}
                // ADB stdin may block without EOF. Process owns this pipe; bound its lifetime.
                Runtime.getRuntime().halt(2);
            }
        }, "riviu-bootstrap-deadline");
        deadline.setDaemon(true);
        deadline.start();
        byte[] payload = null;
        byte[] reply = null;
        try {
            if (args.length != 2 || !args[0].matches("[A-Za-z0-9_.-]{16,100}")) throw new IOException("arguments rejected");
            int checkedUid = Integer.parseInt(args[1]);
            if (checkedUid < 10000 || checkedUid >= 100000) throw new IOException("UID rejected");
            LocalSocket socket = connectBeforeCredential(args[0], connected);
            socket.setSoTimeout(4000);
            // UID comes from controller's pinned-package/ADB check, NOT arbitrary payload/HTTP.
            if (!BootstrapEnvelope.serverUidAllowed(socket.getPeerCredentials().getUid(), checkedUid)) {
                throw new IOException("peer rejected");
            }
            payload = BootstrapEnvelope.read(System.in);
            BootstrapEnvelope.write(socket.getOutputStream(), payload);
            reply = BootstrapEnvelope.read(socket.getInputStream());
            BootstrapEnvelope.write(System.out, reply);
            finished.set(true);
        } catch (Exception e) {
            System.err.println("bootstrap_failed");
        } finally {
            finished.set(true);
            deadline.interrupt();
            if (payload != null) Arrays.fill(payload, (byte) 0);
            if (reply != null) Arrays.fill(reply, (byte) 0);
            LocalSocket socket = connected.get();
            if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        }
    }
    private static LocalSocket connectBeforeCredential(String name, AtomicReference<LocalSocket> connected)
            throws IOException, InterruptedException {
        long deadline = System.nanoTime() + java.util.concurrent.TimeUnit.MILLISECONDS.toNanos(1500);
        while (true) {
            LocalSocket attempt = new LocalSocket();
            connected.set(attempt);
            try {
                attempt.connect(new LocalSocketAddress(name, LocalSocketAddress.Namespace.ABSTRACT));
                return attempt;
            } catch (IOException unavailable) {
                try { attempt.close(); } catch (IOException ignored) {}
                if (System.nanoTime() - deadline >= 0) throw new IOException("listener unavailable");
                Thread.sleep(50);
            }
        }
        // Once connected, UID rejection or payload/ACK failure is NEVER retried.
    }
    static boolean acceptedCaller(LocalSocket socket, boolean debuggable) throws IOException {
        return BootstrapEnvelope.callerUidAllowed(socket.getPeerCredentials().getUid(),
                android.os.Process.myUid(), debuggable);
    }
}
