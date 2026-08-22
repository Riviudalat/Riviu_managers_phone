package com.riviu.agent;

import android.app.WallpaperManager;
import android.content.Context;
import org.json.JSONObject;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;

/**
 * Set the device wallpaper from a file the desktop already pushed (feature A3, xiaowei
 * "set number as wallpaper" for visual identification of a phone in the grid). The PC renders
 * a PNG of the phone's number and pushes it; this sets it as the wallpaper.
 */
final class WallpaperSet {
    private WallpaperSet() {}

    /** The only directory the host ever pushes a wallpaper into. */
    private static final String ALLOWED_DIR = "/data/local/tmp/";
    /** ...and the only name prefix within it. */
    private static final String ALLOWED_PREFIX = "riviu-";

    /**
     * Set the wallpaper from a file the host pushed.
     *
     * <p>The path is checked against a fixed allowlist rather than trusted. It arrives straight
     * off the wire, and before this it went to {@code new FileInputStream} unexamined — which
     * handed any app on the phone three things at once: read an image it has no permission for
     * (the helper holds {@code READ_EXTERNAL_STORAGE}, auto-granted by {@code install -g}) and
     * recover it through {@code WallpaperManager}; probe the filesystem, because the error text
     * came back verbatim and "no such file" reads differently from "permission denied"; and hang
     * the helper indefinitely by naming a FIFO or {@code /dev/urandom}.
     *
     * <p>Canonicalised before the check, so {@code /data/local/tmp/../../sdcard/x} cannot walk
     * out of the allowed directory.
     */
    static JSONObject setFromFile(Context context, String path) throws Exception {
        if (path == null || path.isEmpty()) {
            return Protocol.error("bad_request", "path required");
        }
        File file = new File(path);
        String resolved = file.getCanonicalPath();
        if (!resolved.startsWith(ALLOWED_DIR)
                || !new File(resolved).getName().startsWith(ALLOWED_PREFIX)) {
            // Says nothing about whether the path exists: the old verbatim error was itself the
            // filesystem oracle.
            return Protocol.error("bad_request", "path is not an allowed wallpaper staging file");
        }
        if (!file.isFile()) {
            return Protocol.error("bad_request", "path is not a regular file");
        }
        WallpaperManager wm = WallpaperManager.getInstance(context);
        try (InputStream in = new FileInputStream(file)) {
            wm.setStream(in);
        }
        return Protocol.ok();
    }
}
