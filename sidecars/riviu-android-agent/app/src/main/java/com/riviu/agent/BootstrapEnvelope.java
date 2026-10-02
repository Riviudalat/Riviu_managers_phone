package com.riviu.agent;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import android.net.LocalSocket;
import java.util.concurrent.TimeUnit;

/** Binary framing only; v2 ownership semantics are deliberately not activated. */
final class BootstrapEnvelope {
    static final int MAX_BYTES = 4096;
    static byte[] read(InputStream stream) throws IOException {
        DataInputStream input = new DataInputStream(stream);
        int size = input.readInt();
        if (size <= 0 || size > MAX_BYTES) throw new IOException("bootstrap frame rejected");
        byte[] payload = new byte[size];
        input.readFully(payload);
        return payload;
    }
    interface DeadlineClock { long now(); }
    interface Timeout { void set(int millis) throws IOException; }
    static byte[] readBoundedDeadline(LocalSocket socket, long deadline) throws IOException {
        return readDeadline(socket.getInputStream(), deadline, System::nanoTime, socket::setSoTimeout);
    }
    static byte[] readDeadline(InputStream input, long deadline, DeadlineClock clock, Timeout timeout) throws IOException {
        byte[] prefix = new byte[4];
        readDeadlineExact(input, prefix, deadline, clock, timeout);
        int size = ((prefix[0] & 255) << 24) | ((prefix[1] & 255) << 16)
                | ((prefix[2] & 255) << 8) | (prefix[3] & 255);
        if (size <= 0 || size > MAX_BYTES) throw new IOException("bootstrap frame rejected");
        byte[] payload = new byte[size];
        try { readDeadlineExact(input, payload, deadline, clock, timeout); return payload; }
        catch (IOException e) { java.util.Arrays.fill(payload, (byte) 0); throw e; }
    }
    private static void readDeadlineExact(InputStream input, byte[] target, long deadline,
            DeadlineClock clock, Timeout timeout) throws IOException {
        int offset = 0;
        while (offset < target.length) {
            long remaining = deadline - clock.now();
            if (remaining <= 0) throw new IOException("bootstrap deadline");
            long millis = Math.max(1, TimeUnit.NANOSECONDS.toMillis(remaining));
            timeout.set((int) Math.min(Integer.MAX_VALUE, millis));
            int count = input.read(target, offset, target.length - offset);
            if (clock.now() - deadline >= 0) throw new IOException("bootstrap deadline");
            if (count <= 0) throw new IOException("bootstrap frame incomplete");
            offset += count;
        }
    }
    static void write(OutputStream stream, byte[] payload) throws IOException {
        if (payload.length <= 0 || payload.length > MAX_BYTES) throw new IOException("bootstrap frame rejected");
        DataOutputStream output = new DataOutputStream(stream);
        output.writeInt(payload.length);
        output.write(payload);
        output.flush();
    }
    static boolean serverUidAllowed(int actual, int checkedPackageUid) {
        return checkedPackageUid >= 10000 && checkedPackageUid < 100000 && actual == checkedPackageUid;
    }
    static boolean callerUidAllowed(int actual, int ownUid, boolean debuggable) {
        // Canary transport uses Android run-as to execute the bridge as this debug package UID.
        // Shell/root/other apps are not bootstrap callers, even if privileged elsewhere.
        return debuggable && ownUid >= 10000 && ownUid < 100000 && actual == ownUid;
    }
}
