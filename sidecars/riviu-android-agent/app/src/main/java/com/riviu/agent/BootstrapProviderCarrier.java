package com.riviu.agent;

import android.content.ComponentName;
import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ProviderInfo;
import android.os.Binder;
import android.os.Build;
import android.os.Bundle;
import android.os.IBinder;
import java.io.IOException;
import java.lang.reflect.Method;
import java.security.MessageDigest;

/** Shell's external-provider API does not require an AMS ApplicationThread record. */
final class BootstrapProviderCarrier implements AutoCloseable {
    static final String PACKAGE = "com.riviu.agent";
    static final String PROVIDER = PACKAGE + ".BootstrapBinderProvider";
    static final String AUTHORITY = PACKAGE + ".bootstrap";
    static final String DUMP = "android.permission.DUMP";
    private final Object manager;
    private final Class<?> managerType;
    private final IBinder token = new Binder();
    private boolean acquired;
    private IBinder binder;

    private BootstrapProviderCarrier() throws Exception {
        managerType = Class.forName("android.app.IActivityManager");
        manager = Class.forName("android.app.ActivityManager").getMethod("getService").invoke(null);
    }

    static void proveProvider(Context context, int checkedUid, String certificate) throws Exception {
        if (android.os.Process.myUid() != 2000 || checkedUid < 10000 || checkedUid >= 100000
                || certificate == null || !certificate.matches("[a-fA-F0-9]{64}")
                || context.checkSelfPermission(DUMP) != PackageManager.PERMISSION_GRANTED) throw new IOException();
        PackageManager pm = context.getPackageManager();
        ProviderInfo info = pm.getProviderInfo(new ComponentName(PACKAGE, PROVIDER), 0);
        PackageInfo pkg = pm.getPackageInfo(PACKAGE, PackageManager.GET_SIGNATURES);
        if (info.applicationInfo == null || !info.enabled || !info.applicationInfo.enabled || !info.exported
                || !PACKAGE.equals(info.packageName) || !PROVIDER.equals(info.name)
                || !AUTHORITY.equals(info.authority) || !DUMP.equals(info.readPermission)
                || !DUMP.equals(info.writePermission) || info.applicationInfo.uid != checkedUid
                || !PACKAGE.equals(pkg.packageName) || pkg.applicationInfo == null
                || pkg.applicationInfo.uid != checkedUid || pkg.signatures == null || pkg.signatures.length != 1) {
            throw new IOException();
        }
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(pkg.signatures[0].toByteArray());
        StringBuilder hex = new StringBuilder();
        for (byte value : digest) hex.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
        if (!certificate.equalsIgnoreCase(hex.toString())) throw new IOException();
    }

    static BootstrapProviderCarrier acquire(Context context, int checkedUid, String certificate) throws Exception {
        proveProvider(context, checkedUid, certificate);
        BootstrapProviderCarrier handle = new BootstrapProviderCarrier();
        try {
            Object holder;
            if (Build.VERSION.SDK_INT <= 28) {
                holder = handle.managerType.getMethod("getContentProviderExternal", String.class,
                        int.class, IBinder.class).invoke(handle.manager, AUTHORITY, 0, handle.token);
            } else {
                holder = handle.managerType.getMethod("getContentProviderExternal", String.class,
                        int.class, IBinder.class, String.class).invoke(handle.manager, AUTHORITY, 0,
                                handle.token, "riviu-bootstrap");
            }
            if (holder == null) throw new IOException();
            // Holder creation owns an external reference even if the provider/reply is invalid.
            handle.acquired = true;
            Object provider = Class.forName("android.app.ContentProviderHolder").getField("provider").get(holder);
            if (provider == null) throw new IOException();
            Class<?> type = Class.forName("android.content.IContentProvider");
            Bundle result;
            if (Build.VERSION.SDK_INT <= 28) {
                result = (Bundle) type.getMethod("call", String.class, String.class, String.class, Bundle.class)
                        .invoke(provider, "com.android.shell", "binder", null, null);
            } else if (Build.VERSION.SDK_INT == 29) {
                result = (Bundle) type.getMethod("call", String.class, String.class, String.class,
                        String.class, Bundle.class).invoke(provider, "com.android.shell", AUTHORITY, "binder", null, null);
            } else if (Build.VERSION.SDK_INT == 30) {
                result = (Bundle) type.getMethod("call", String.class, String.class, String.class,
                        String.class, String.class, Bundle.class).invoke(provider, "com.android.shell", null,
                                AUTHORITY, "binder", null, null);
            } else {
                Class<?> attribution = Class.forName("android.content.AttributionSource");
                Object source = Context.class.getMethod("getAttributionSource").invoke(context);
                result = (Bundle) type.getMethod("call", attribution, String.class, String.class,
                        String.class, Bundle.class).invoke(provider, source, AUTHORITY, "binder", null, null);
            }
            if (result == null || result.size() != 1) throw new IOException();
            handle.binder = result.getBinder("binder");
            if (handle.binder == null) throw new IOException();
            proveProvider(context, checkedUid, certificate);
            return handle;
        } catch (Exception error) {
            try { handle.close(); } catch (Exception cleanup) { error.addSuppressed(cleanup); }
            throw error;
        }
    }

    IBinder binder() { return binder; }

    @Override public void close() throws Exception {
        if (!acquired) return;
        // Keep the acquisition marked until AMS confirms release; no duplicate call in this process.
        Method release;
        if (Build.VERSION.SDK_INT <= 28) {
            release = managerType.getMethod("removeContentProviderExternal", String.class, IBinder.class);
            release.invoke(manager, AUTHORITY, token);
        } else {
            release = managerType.getMethod("removeContentProviderExternalAsUser", String.class, IBinder.class, int.class);
            release.invoke(manager, AUTHORITY, token, 0);
        }
        acquired = false;
        binder = null;
    }
}
