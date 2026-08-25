package com.riviu.agent;

import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.drawable.BitmapDrawable;
import android.graphics.drawable.Drawable;
import android.util.Base64;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.util.List;

/**
 * App names and icons, which is the one thing about an installed-app list that adb cannot
 * answer.
 *
 * `cmd package list packages` gives package names only; the label lives in the APK's resource
 * table as a resource id and needs the device locale to resolve, and no farm phone here has
 * `aapt`. Measured on the Redmi: 257 of 273 records from `cmd package query-activities`
 * carried `nonLocalizedLabel=null`, and pulling APKs to read them is absurd at the sizes
 * involved (one `base.apk` was 261 MB). On the device, in this process, it is one call:
 * {@link PackageManager#getApplicationLabel} and {@link PackageManager#getApplicationIcon}.
 *
 * The desktop still gets its *list* from adb — that reads both partitions and includes apps
 * with no launcher activity — and uses this only to put a name and a picture on the rows it
 * already has. So a phone without this helper shows package names, exactly as before, rather
 * than showing nothing.
 */
final class AppList {
    /**
     * Icon edge in pixels. 48 is a grid row's icon at any sane density, and the size is what
     * keeps the whole response sendable: a launcher icon rendered at its native 192 px is
     * ~20 KB of PNG, so 160 user apps would be 3 MB of base64 in one JSON body. At 48 px it
     * is ~1–2 KB each.
     */
    private static final int DEFAULT_ICON_PX = 48;
    private static final int MAX_ICON_PX = 192;
    /**
     * Ceiling on the icon bytes one response will carry, before base64. Reaching it stops
     * *icons* and never rows — a list that silently loses apps because they sorted late would
     * be worse than a list with some rows unillustrated, and `iconsTruncated` says it happened.
     */
    private static final int ICON_BUDGET_BYTES = 3 * 1024 * 1024;

    private AppList() {}

    /**
     * @param packages when non-empty, only these package names are described. The desktop
     *                 passes the list adb gave it, so this method never decides *which* apps
     *                 exist — only what they are called and what they look like.
     */
    static JSONObject describe(Context context, JSONArray packages, boolean withIcons, int iconPx)
            throws Exception {
        PackageManager pm = context.getPackageManager();
        int size = iconPx <= 0 ? DEFAULT_ICON_PX : Math.min(iconPx, MAX_ICON_PX);
        JSONArray apps = new JSONArray();
        int iconBytes = 0;
        int truncated = 0;

        for (String name : requested(context, pm, packages)) {
            ApplicationInfo info;
            try {
                info = pm.getApplicationInfo(name, 0);
            } catch (PackageManager.NameNotFoundException absent) {
                // Uninstalled between the desktop's listing and now. Skipped rather than
                // reported as an error: the row simply keeps its package name.
                continue;
            }
            JSONObject app = new JSONObject()
                    .put("package", name)
                    .put("label", String.valueOf(pm.getApplicationLabel(info)))
                    .put("system", (info.flags & ApplicationInfo.FLAG_SYSTEM) != 0);
            if (withIcons) {
                if (iconBytes < ICON_BUDGET_BYTES) {
                    byte[] png = iconPng(pm, info, size);
                    if (png != null) {
                        iconBytes += png.length;
                        app.put("icon", Base64.encodeToString(png, Base64.NO_WRAP));
                    }
                } else {
                    truncated++;
                }
            }
            apps.put(app);
        }
        return Protocol.ok()
                .put("apps", apps)
                .put("iconPx", size)
                .put("iconsTruncated", truncated);
    }

    /** The packages asked for, or every launchable one when the caller named none. */
    private static java.util.List<String> requested(
            Context context, PackageManager pm, JSONArray packages) {
        java.util.List<String> names = new java.util.ArrayList<>();
        if (packages != null && packages.length() > 0) {
            for (int i = 0; i < packages.length(); i++) {
                String name = packages.optString(i, "");
                if (!name.isEmpty()) {
                    names.add(name);
                }
            }
            return names;
        }
        // No list given: describe what the launcher would show. Deliberately narrower than
        // adb's listing — this branch exists for a caller with no list of its own, and
        // "everything installed" includes hundreds of packages with no icon and no name a
        // person would recognise.
        android.content.Intent main = new android.content.Intent(android.content.Intent.ACTION_MAIN)
                .addCategory(android.content.Intent.CATEGORY_LAUNCHER);
        List<android.content.pm.ResolveInfo> resolved = pm.queryIntentActivities(main, 0);
        for (android.content.pm.ResolveInfo row : resolved) {
            if (row.activityInfo != null && row.activityInfo.packageName != null) {
                names.add(row.activityInfo.packageName);
            }
        }
        return names;
    }

    /**
     * The app's icon as a PNG of exactly {@code size}×{@code size}.
     *
     * Drawn through a Canvas rather than cast to a BitmapDrawable: an adaptive icon (every
     * app on Android 8+) is a layered drawable with no bitmap to take, and a cast would throw
     * on the majority of a modern phone's apps. The fast path is kept for the ones that are
     * plain bitmaps.
     */
    private static byte[] iconPng(PackageManager pm, ApplicationInfo info, int size) {
        try {
            Drawable drawable = pm.getApplicationIcon(info);
            Bitmap bitmap;
            if (drawable instanceof BitmapDrawable
                    && ((BitmapDrawable) drawable).getBitmap() != null) {
                bitmap = Bitmap.createScaledBitmap(
                        ((BitmapDrawable) drawable).getBitmap(), size, size, true);
            } else {
                bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888);
                Canvas canvas = new Canvas(bitmap);
                drawable.setBounds(0, 0, size, size);
                drawable.draw(canvas);
            }
            ByteArrayOutputStream png = new ByteArrayOutputStream();
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, png);
            return png.toByteArray();
        } catch (Throwable failed) {
            // One app's icon is not worth failing the list for, and there is no useful
            // fallback picture to invent: the row goes out without one.
            return null;
        }
    }
}
