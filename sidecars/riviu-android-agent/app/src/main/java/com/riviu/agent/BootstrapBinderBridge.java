package com.riviu.agent;

import android.content.ComponentName;
import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.IBinder;
import android.os.Looper;
import android.os.Parcel;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.concurrent.TimeUnit;
import org.json.JSONObject;

/** One shell app_process, one externally acquired guarded Binder, stdin-only credentials. */
final class BootstrapBinderBridge {
    static void main(String[] args) {
        // The watchdog includes reflection, package checks, bind, stdin and synchronous transact.
        Thread watchdog = new Thread(() -> {
            try { Thread.sleep(5000); } catch (InterruptedException ignored) { return; }
            Runtime.getRuntime().halt(2);
        }, "riviu-binder-deadline");
        watchdog.setDaemon(true); watchdog.start();
        try {
            if (android.os.Process.myUid() != 2000 || args.length != 7) throw new IOException();
            int checkedUid = Integer.parseInt(args[1]);
            if (checkedUid < 10000 || checkedUid >= 100000 || !args[2].matches("[a-fA-F0-9]{64}")) throw new IOException();
            RequestAuth.requireId(args[3]); RequestAuth.requireId(args[4]);
            for (int i = 5; i <= 6; i++) if (!"-".equals(args[i])) RequestAuth.requireId(args[i]);
            if ("-".equals(args[5]) != "-".equals(args[6])) throw new IOException();
            if (Looper.myLooper() == null) Looper.prepareMainLooper();
            // app_process has no Application. Use Android's systemMain context with shell attribution;
            // no impersonation of the helper's UID, no run-as and no privilege escalation.
            Class<?> activityThread = Class.forName("android.app.ActivityThread");
            Object thread = activityThread.getMethod("systemMain").invoke(null);
            Context system = (Context) activityThread.getMethod("getSystemContext").invoke(thread);
            Context shell = system.createPackageContext("com.android.shell", 0);
            provePackage(shell, checkedUid, args[2]);
            new Thread(() -> exchange(shell, checkedUid, args), "riviu-binder-exchange").start();
            Looper.loop();
        } catch (Throwable ignored) {
            System.err.println("bootstrap_failed"); System.exit(2);
        }
    }

    private static void provePackage(Context context, int checkedUid, String certificate) throws Exception {
        PackageManager pm = context.getPackageManager();
        ComponentName exact = new ComponentName(BootstrapBinderPolicy.PACKAGE, BootstrapBinderPolicy.SERVICE);
        ServiceInfo info = pm.getServiceInfo(exact, 0);
        PackageInfo pkg = pm.getPackageInfo(BootstrapBinderPolicy.PACKAGE, PackageManager.GET_SIGNATURES);
        if (!info.enabled || !info.applicationInfo.enabled || !info.exported
                || !BootstrapBinderPolicy.PACKAGE.equals(info.packageName)
                || !BootstrapBinderPolicy.SERVICE.equals(info.name)
                || !"android.permission.DUMP".equals(info.permission)
                || !BootstrapEnvelope.serverUidAllowed(info.applicationInfo.uid, checkedUid)
                || !BootstrapBinderPolicy.PACKAGE.equals(pkg.packageName)
                || pkg.applicationInfo.uid != checkedUid || pkg.signatures == null || pkg.signatures.length != 1
                || context.checkSelfPermission("android.permission.DUMP") != PackageManager.PERMISSION_GRANTED) {
            throw new IOException();
        }
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(pkg.signatures[0].toByteArray());
        StringBuilder hex = new StringBuilder();
        for (byte value : digest) hex.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
        if (!BootstrapBinderPolicy.identityAllowed(info.packageName, info.name, info.permission,
                info.applicationInfo.uid, checkedUid, hex.toString(), certificate,
                info.enabled && info.applicationInfo.enabled, info.exported)) throw new IOException();
    }

    private static void metadata(Parcel data, String[] args) {
        data.writeInterfaceToken(BootstrapBinderPolicy.DESCRIPTOR);
        data.writeString(args[3]); data.writeString(args[4]);
        data.writeString(args[5]); data.writeString(args[6]);
    }
    private static void exchange(Context context, int checkedUid, String[] args) {
        BootstrapProviderCarrier handle = null;
        boolean ok = false;
        byte[] payload = null, response = null;
        try {
            handle = BootstrapProviderCarrier.acquire(context, checkedUid, args[2]);
            IBinder service = handle.binder();
            // Probe occurs BEFORE stdin reading and proves the pinned service's live UID and echo.
            Parcel data = Parcel.obtain(), reply = Parcel.obtain();
            try {
                metadata(data, args);
                if (!service.transact(BootstrapBinderPolicy.PROBE, data, reply, 0)) throw new IOException();
                reply.readException();
                if (!BootstrapEnvelope.serverUidAllowed(reply.readInt(), checkedUid)
                        || !args[3].equals(reply.readString()) || !args[4].equals(reply.readString())) throw new IOException();
                String instance = reply.readString(), generation = reply.readString();
                if ((!"-".equals(args[5]) && !args[5].equals(instance))
                        || (!"-".equals(args[6]) && !args[6].equals(generation)) || reply.dataAvail() != 0) throw new IOException();
                // Recheck package proof after acquisition to catch replacements before credential read.
                provePackage(context, checkedUid, args[2]);
            } finally { data.recycle(); reply.recycle(); }
            payload = BootstrapEnvelope.read(System.in);
            JSONObject request = new JSONObject(new String(payload, StandardCharsets.UTF_8));
            if (!OwnerSession.bindingMatches(args[3], args[4], request.optString("nonce"), request.optString("ownerId"))
                    || !args[5].equals(request.optString("serviceInstance", "-"))
                    || !args[6].equals(request.optString("ownerGeneration", "-"))) throw new IOException();
            data = Parcel.obtain(); reply = Parcel.obtain();
            try {
                metadata(data, args); data.writeByteArray(payload);
                // Exactly one credential transaction. Lost ACK must reconcile OwnerSession, never retry here.
                if (!service.transact(BootstrapBinderPolicy.REQUEST, data, reply, 0)) throw new IOException();
                reply.readException();
                if (reply.dataAvail() <= 0 || reply.dataAvail() > BootstrapEnvelope.MAX_BYTES + 8) throw new IOException();
                response = reply.createByteArray();
                if (response == null || reply.dataAvail() != 0) throw new IOException();
                JSONObject result = new JSONObject(new String(response, StandardCharsets.UTF_8));
                if (!OwnerSession.bindingMatches(args[3], args[4], result.optString("nonce"), result.optString("ownerId"))) throw new IOException();
                BootstrapEnvelope.write(System.out, response);
                ok = true;
            } finally { data.recycle(); reply.recycle(); }
        } catch (Throwable ignored) {
            System.err.println("bootstrap_failed");
        } finally {
            if (payload != null) Arrays.fill(payload, (byte) 0);
            if (response != null) Arrays.fill(response, (byte) 0);
            if (handle != null) try { handle.close(); } catch (Exception ignored) {
                System.err.println("bootstrap_failed"); ok = false;
            }
            System.exit(ok ? 0 : 2);
        }
    }
}
