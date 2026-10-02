package com.riviu.agent;

import android.content.ComponentName;
import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.IBinder;
import android.os.Looper;
import android.os.Parcel;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.concurrent.TimeUnit;

/** Standalone diagnostic only: never packaged into the helper; never reads stdin. */
public final class ProbeBinder {
    private static final String PACKAGE = "com.riviu.agent";
    private static final String SERVICE = PACKAGE + ".AgentService";
    private static final String ACTION = PACKAGE + ".BOOTSTRAP_BINDER";
    private static final String DESCRIPTOR = PACKAGE + ".BootstrapBinder.v1";
    private static final String DUMP = "android.permission.DUMP";
    private static final int PROBE = 1; // The only transaction this diagnostic can send.
    private static final String DIAGNOSTIC_ID = "binder-provider-v3";
    private static volatile String stage = "arguments";
    private static long deadline;

    private static void stage(String value) {
        stage = value;
        System.err.println("probe_stage=" + value);
        if (System.nanoTime() - deadline >= 0) throw new IllegalStateException();
    }
    private static void require(boolean value) {
        if (!value) throw new IllegalStateException();
    }
    private static String hex(byte[] bytes) {
        StringBuilder result = new StringBuilder();
        for (byte value : bytes) result.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
        return result.toString();
    }
    private static String publicId(SecureRandom random) {
        byte[] bytes = new byte[16]; random.nextBytes(bytes); return hex(bytes);
    }
    private static boolean id(String value) {
        return value != null && ("-".equals(value) || value.matches("[A-Za-z0-9_-]{16,128}"));
    }
    private static void proveFeature() throws Exception {
        stage("installed_feature_identity");
        // This pure status constructor comes from the pinned installed APK on CLASSPATH.
        // No HTTP, service start, credential read, or application lifecycle is invoked.
        java.lang.reflect.Method status = Class.forName(PACKAGE + ".Protocol").getDeclaredMethod("status");
        status.setAccessible(true);
        org.json.JSONObject value = (org.json.JSONObject) status.invoke(null);
        require("0.7.0".equals(value.getString("agentVersion")) && value.getInt("protocolVersion") == 1);
        org.json.JSONArray features = value.getJSONArray("features");
        require(features.length() <= 64);
        boolean present = false;
        for (int i = 0; i < features.length(); i++) {
            if ("secureBootstrapBinderShell".equals(features.getString(i))) present = true;
        }
        require(present);
    }
    private static void failure(Throwable error) {
        // Unwrap reflection without ever printing exception messages or stack traces.
        for (int i = 0; i < 4 && error instanceof java.lang.reflect.InvocationTargetException
                && error.getCause() != null; i++) error = error.getCause();
        String kind = error.getClass().getName();
        if (kind.length() > 160 || !kind.matches("[A-Za-z0-9_.$]+")) kind = "Other";
        String reason = "not_classified";
        if ("bind_service".equals(stage) && error instanceof SecurityException) {
            // AOSP Android 9 ActiveServices and ContextImpl use these fixed prefixes.
            // Inspect in memory only: never emit the message, binder identity, Intent or PID.
            String message = error.getMessage();
            reason = "security_exception_unknown";
            if (message != null && message.length() <= 4096) {
                if (message.startsWith("Unable to find app for caller ")
                        && message.contains(" when binding service ")) {
                    reason = "caller_process_unregistered";
                } else if (message.startsWith("Not allowed to bind to service ")) {
                    reason = "service_access_denied";
                }
            }
        }
        System.err.println("probe_failed_stage=" + stage + " exception_class=" + kind
                + " reason=" + reason + " message=diagnostic_failed_redacted");
    }
    private static void provePackage(Context context, int uid, String certificate, String prefix) throws Exception {
        stage(prefix + "_service_info");
        PackageManager pm = context.getPackageManager();
        ServiceInfo info = pm.getServiceInfo(new ComponentName(PACKAGE, SERVICE), 0);
        require(info.applicationInfo != null && info.enabled && info.applicationInfo.enabled
                && info.exported && PACKAGE.equals(info.packageName) && SERVICE.equals(info.name)
                && DUMP.equals(info.permission) && info.applicationInfo.uid == uid);
        stage(prefix + "_package_info");
        PackageInfo pkg = pm.getPackageInfo(PACKAGE, PackageManager.GET_SIGNATURES);
        require(PACKAGE.equals(pkg.packageName) && pkg.applicationInfo != null
                && pkg.applicationInfo.uid == uid && pkg.signatures != null && pkg.signatures.length == 1
                && pkg.versionCode == 7 && "0.7.0".equals(pkg.versionName));
        stage(prefix + "_dump_permission");
        require(android.os.Process.myUid() == 2000
                && context.checkSelfPermission(DUMP) == PackageManager.PERMISSION_GRANTED);
        stage(prefix + "_certificate");
        require(certificate.equalsIgnoreCase(hex(MessageDigest.getInstance("SHA-256")
                .digest(pkg.signatures[0].toByteArray()))));
        stage(prefix + "_verified");
    }
    public static void main(String[] args) {
        deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(5000);
        Thread watchdog = new Thread(() -> {
            while (true) {
                long remaining = deadline - System.nanoTime();
                if (remaining <= 0) break;
                try { TimeUnit.NANOSECONDS.sleep(remaining); } catch (InterruptedException ignored) {}
            }
            System.err.println("probe_failed_stage=" + stage
                    + " exception_class=DeadlineExceeded message=total_deadline_exceeded");
            Runtime.getRuntime().halt(2);
        }, "probe-binder-deadline");
        watchdog.setDaemon(true); watchdog.start();
        try {
            System.err.println("probe_diagnostic=" + DIAGNOSTIC_ID);
            stage("arguments");
            require(args.length == 4 && args[0].matches("[0-9]{5}") && args[1].matches("[a-fA-F0-9]{64}")
                    && id(args[2]) && id(args[3]) && ("-".equals(args[2]) == "-".equals(args[3])));
            int uid = Integer.parseInt(args[0]); require(uid >= 10000 && uid < 100000);
            stage("caller_uid"); require(android.os.Process.myUid() == 2000);
            stage("looper_prepare"); if (Looper.myLooper() == null) Looper.prepareMainLooper();
            stage("activity_thread_class"); Class<?> type = Class.forName("android.app.ActivityThread");
            stage("system_main"); Object thread = type.getMethod("systemMain").invoke(null);
            stage("system_context"); Context system = (Context) type.getMethod("getSystemContext").invoke(thread);
            stage("shell_context"); Context shell = system.createPackageContext("com.android.shell", 0);
            provePackage(shell, uid, args[1], "prebind");
            proveFeature();
            stage("public_identity"); SecureRandom random = new SecureRandom();
            String nonce = publicId(random), owner = publicId(random);
            new Thread(() -> exchange(shell, uid, args[1], nonce, owner, args[2], args[3]), "probe-binder-exchange").start();
            // Callbacks use the same main Looper as the installed production carrier.
            Looper.loop();
        } catch (Throwable error) { failure(error); System.exit(2); }
    }
    private static void exchange(Context context, int uid, String certificate, String nonce, String owner,
            String expectedInstance, String expectedGeneration) {
        BootstrapProviderCarrier handle = null;
        boolean ok = false;
        try {
            stage("provider_acquire");
            handle = BootstrapProviderCarrier.acquire(context, uid, certificate);
            IBinder binder = handle.binder();
            System.err.println("probe_provider_acquired=true");
            provePackage(context, uid, certificate, "bound");
            Parcel data = Parcel.obtain(), reply = Parcel.obtain();
            String instance, generation;
            try {
                stage("probe_metadata"); data.writeInterfaceToken(DESCRIPTOR);
                data.writeString(nonce); data.writeString(owner);
                data.writeString(expectedInstance); data.writeString(expectedGeneration);
                stage("probe_transact"); boolean accepted = binder.transact(PROBE, data, reply, 0);
                System.err.println("probe_transact_return=" + accepted); require(accepted);
                stage("probe_reply_exception"); reply.readException();
                stage("probe_reply_binding"); require(reply.readInt() == uid);
                require(nonce.equals(reply.readString()) && owner.equals(reply.readString()));
                instance = reply.readString(); generation = reply.readString();
                require(id(instance) && id(generation) && reply.dataAvail() == 0);
                require(("-".equals(expectedInstance) || expectedInstance.equals(instance))
                        && ("-".equals(expectedGeneration) || expectedGeneration.equals(generation)));
            } finally { data.recycle(); reply.recycle(); }
            provePackage(context, uid, certificate, "postprobe");
            stage("result");
            System.out.println("{\"ok\":true,\"transaction\":\"PROBE\",\"uid\":" + uid
                    + ",\"nonce\":\"" + nonce + "\",\"ownerId\":\"" + owner
                    + "\",\"serviceInstance\":\"" + instance + "\",\"generation\":\"" + generation + "\"}");
            ok = true;
        } catch (Throwable error) { failure(error); }
        finally {
            if (handle != null) {
                try { stage("provider_release"); handle.close(); }
                catch (Throwable error) { failure(error); ok = false; }
            }
            if (ok) { try { stage("done"); } catch (Throwable error) { failure(error); ok = false; } }
            System.exit(ok ? 0 : 2);
        }
    }
}
