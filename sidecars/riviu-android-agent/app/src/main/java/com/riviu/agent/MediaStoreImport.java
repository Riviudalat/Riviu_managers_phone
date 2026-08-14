package com.riviu.agent;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.provider.MediaStore;

import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.OutputStream;

/**
 * Insert one staged file into MediaStore and clear {@code is_pending} when the
 * column exists. Cleanup is by {@code _id} only — never a {@code _data LIKE}
 * pattern (a co-resident farm tool already has a file matching {@code riviu}).
 */
final class MediaStoreImport {
    private MediaStoreImport() {}

    static JSONObject importFile(Context context, String relativePath, String displayName)
            throws Exception {
        String name = safeFileName(relativePath);
        File inbox = new File(context.getExternalFilesDir(null), "inbox");
        File source = new File(inbox, name);
        if (!source.isFile()) {
            throw new IllegalArgumentException("staged file is missing: " + name);
        }
        String shown = displayName == null || displayName.isEmpty() ? name : safeFileName(displayName);
        String mime = mimeFor(name);

        ContentResolver resolver = context.getContentResolver();
        ContentValues values = new ContentValues();
        values.put(MediaStore.Images.Media.DISPLAY_NAME, shown);
        values.put(MediaStore.Images.Media.MIME_TYPE, mime);
        String pendingModel = "absent";
        if (Build.VERSION.SDK_INT >= 29) {
            values.put(MediaStore.Images.Media.IS_PENDING, 1);
            values.put(MediaStore.Images.Media.RELATIVE_PATH, "Pictures/Riviu");
            pendingModel = "pending";
        }

        Uri uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values);
        if (uri == null) {
            throw new IllegalStateException("MediaStore insert returned null");
        }
        OutputStream out = resolver.openOutputStream(uri);
        if (out == null) {
            throw new IllegalStateException("MediaStore openOutputStream returned null");
        }
        try {
            FileInputStream in = new FileInputStream(source);
            try {
                byte[] buffer = new byte[16 * 1024];
                int read;
                while ((read = in.read(buffer)) != -1) {
                    out.write(buffer, 0, read);
                }
            } finally {
                in.close();
            }
        } finally {
            out.close();
        }

        if (Build.VERSION.SDK_INT >= 29) {
            ContentValues clear = new ContentValues();
            clear.put(MediaStore.Images.Media.IS_PENDING, 0);
            resolver.update(uri, clear, null, null);
            pendingModel = "cleared";
        }

        String id = uri.getLastPathSegment();
        return Protocol.ok()
                .put("id", id == null ? "" : id)
                .put("pendingModel", pendingModel);
    }

    static JSONObject deleteById(Context context, String id) throws Exception {
        String safe = safeId(id);
        Uri uri = Uri.withAppendedPath(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, safe);
        context.getContentResolver().delete(uri, null, null);
        return Protocol.ok().put("id", safe);
    }

    static boolean rowExists(Context context, String id) {
        String safe;
        try {
            safe = safeId(id);
        } catch (IllegalArgumentException ignored) {
            return false;
        }
        Uri uri = Uri.withAppendedPath(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, safe);
        Cursor cursor = context.getContentResolver().query(uri, new String[]{"_id"}, null, null, null);
        if (cursor == null) {
            return false;
        }
        try {
            return cursor.moveToFirst();
        } finally {
            cursor.close();
        }
    }

    static String safeFileName(String raw) {
        if (raw == null || raw.isEmpty() || raw.length() > 128) {
            throw new IllegalArgumentException("file name must be 1..=128 characters");
        }
        if (raw.contains("/") || raw.contains("\\") || raw.contains("..")) {
            throw new IllegalArgumentException("file name must be a single path segment");
        }
        for (int i = 0; i < raw.length(); i++) {
            char ch = raw.charAt(i);
            if (!(Character.isLetterOrDigit(ch) || ch == '.' || ch == '_' || ch == '-')) {
                throw new IllegalArgumentException("file name carries a character the shell would act on");
            }
        }
        return raw;
    }

    static String safeId(String raw) {
        if (raw == null || raw.isEmpty() || raw.length() > 32) {
            throw new IllegalArgumentException("MediaStore id must be 1..=32 digits");
        }
        for (int i = 0; i < raw.length(); i++) {
            if (!Character.isDigit(raw.charAt(i))) {
                throw new IllegalArgumentException("MediaStore id must be digits");
            }
        }
        return raw;
    }

    private static String mimeFor(String name) {
        String lower = name.toLowerCase();
        if (lower.endsWith(".png")) {
            return "image/png";
        }
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) {
            return "image/jpeg";
        }
        throw new IllegalArgumentException("only PNG/JPG are imported");
    }
}
