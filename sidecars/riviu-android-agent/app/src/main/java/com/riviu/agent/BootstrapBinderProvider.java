package com.riviu.agent;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Binder;
import android.os.Bundle;
import android.os.IBinder;

/** Credential-free discovery only; never creates a service or changes ownership. */
public final class BootstrapBinderProvider extends ContentProvider {
    @Override public boolean onCreate() { return true; }

    @Override public Bundle call(String method, String arg, Bundle extras) {
        // ContentProvider.Transport.call does not enforce manifest read/write permissions.
        // Authenticate the kernel caller before inspecting any caller-provided metadata.
        if (Binder.getCallingUid() != 2000 || getContext() == null
                || getContext().checkCallingPermission("android.permission.DUMP") != PackageManager.PERMISSION_GRANTED) {
            throw new SecurityException("bootstrap caller rejected");
        }
        if (!"binder".equals(method) || arg != null || extras != null) throw new IllegalArgumentException();
        IBinder binder = AgentService.activeBootstrapBinder();
        if (binder == null) throw new IllegalStateException("bootstrap service unavailable");
        Bundle result = new Bundle();
        result.putBinder("binder", binder);
        return result;
    }

    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        throw new UnsupportedOperationException();
    }
    @Override public String getType(Uri uri) { throw new UnsupportedOperationException(); }
    @Override public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri, String selection, String[] args) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri uri, ContentValues values, String selection, String[] args) {
        throw new UnsupportedOperationException();
    }
}
