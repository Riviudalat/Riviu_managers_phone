package com.riviu.agent;

import android.app.WallpaperManager;
import android.content.Context;
import org.json.JSONObject;
import java.io.FileInputStream;
import java.io.InputStream;

/**
 * Set the device wallpaper from a file the desktop already pushed (feature A3, xiaowei
 * "set number as wallpaper" for visual identification of a phone in the grid). The PC renders
 * a PNG of the phone's number and pushes it; this sets it as the wallpaper.
 */
final class WallpaperSet {
    private WallpaperSet() {}

    static JSONObject setFromFile(Context context, String path) throws Exception {
        if (path == null || path.isEmpty()) {
            return Protocol.error("bad_request", "path required");
        }
        WallpaperManager wm = WallpaperManager.getInstance(context);
        try (InputStream in = new FileInputStream(path)) {
            wm.setStream(in);
        }
        return Protocol.ok();
    }
}
