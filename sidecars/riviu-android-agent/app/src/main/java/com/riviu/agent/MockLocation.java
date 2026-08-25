package com.riviu.agent;

import android.content.Context;
import android.location.Location;
import android.location.LocationManager;
import android.os.SystemClock;
import org.json.JSONObject;

/**
 * GPS spoofing via Android's test-provider API (feature B, xiaowei "虚拟定位").
 *
 * Requires this app to be the selected mock-location app, which the desktop grants with
 * {@code adb shell appops set com.riviu.agent android:mock_location allow} (or Developer
 * Options → Select mock location app). With that granted, the app injects a fixed location
 * into the GPS and network providers so foreground apps read the chosen coordinates.
 */
final class MockLocation {
    private MockLocation() {}

    private static final String[] PROVIDERS = {
        LocationManager.GPS_PROVIDER,
        LocationManager.NETWORK_PROVIDER,
    };

    static JSONObject set(Context context, double lat, double lng) throws Exception {
        LocationManager lm = (LocationManager) context.getSystemService(Context.LOCATION_SERVICE);
        if (lm == null) {
            return Protocol.error("no_location_manager", "LocationManager unavailable");
        }
        for (String provider : PROVIDERS) {
            try {
                // addTestProvider throws if this app is not the mock-location app; the caller
                // (desktop) is expected to have granted android:mock_location via appops.
                lm.addTestProvider(
                        provider, false, false, false, false, true, true, true, 1, 1);
            } catch (Exception ignored) {
                // Already added, or not permitted — setTestProviderLocation below reports.
            }
            try {
                lm.setTestProviderEnabled(provider, true);
                Location loc = new Location(provider);
                loc.setLatitude(lat);
                loc.setLongitude(lng);
                loc.setAccuracy(1.0f);
                loc.setAltitude(0);
                loc.setTime(System.currentTimeMillis());
                loc.setElapsedRealtimeNanos(SystemClock.elapsedRealtimeNanos());
                lm.setTestProviderLocation(provider, loc);
            } catch (SecurityException error) {
                return Protocol.error(
                        "mock_not_allowed",
                        "grant with: appops set com.riviu.agent android:mock_location allow");
            } catch (Exception ignored) {
                // A provider this device does not have — the other still applies.
            }
        }
        return Protocol.ok();
    }

    static JSONObject stop(Context context) throws Exception {
        LocationManager lm = (LocationManager) context.getSystemService(Context.LOCATION_SERVICE);
        if (lm == null) {
            return Protocol.ok();
        }
        for (String provider : PROVIDERS) {
            try {
                lm.removeTestProvider(provider);
            } catch (Exception ignored) {
                // Not registered — nothing to remove.
            }
        }
        return Protocol.ok();
    }
}
