package com.riviu.agent;

import android.content.res.Configuration;
import android.os.LocaleList;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import org.json.JSONObject;

/**
 * Put the phone's system language on English, the only UI language the fleet drives
 * (owner decision 2026-10-12, "Fix luôn phần đổi ngôn ngữ"). Riviu reads the locale from
 * {@code am get-config}; TikTok 45.7.3 follows it unless its own app language was set.
 *
 * <p>Android has no public API for this. Settings itself calls the hidden
 * {@code IActivityManager.updatePersistentConfiguration}, which demands
 * {@code CHANGE_CONFIGURATION}: a development permission the desktop grants with
 * {@code pm grant} before the call. The locale is not taken off the wire: only {@code en-US}
 * is accepted, so this endpoint cannot be used to put a phone on any other language.
 * English moves to the front; the other languages the operator added stay after it.
 */
final class SystemLocale {
    private SystemLocale() {}

    static final String ENGLISH = "en-US";

    static JSONObject set(String languageTag) throws Exception {
        if (!ENGLISH.equals(languageTag)) {
            return Protocol.error("bad_request", "only en-US is accepted");
        }
        Object activityManager = activityManager();
        Method getConfiguration = activityManager.getClass().getMethod("getConfiguration");
        Configuration config = (Configuration) getConfiguration.invoke(activityManager);
        if (config == null) {
            return Protocol.error("locale_unreadable", "system configuration unavailable");
        }
        Locale english = Locale.forLanguageTag(ENGLISH);
        List<Locale> ordered = new ArrayList<>();
        ordered.add(english);
        LocaleList current = config.getLocales();
        for (int i = 0; i < current.size(); i++) {
            Locale locale = current.get(i);
            if (!english.getLanguage().equals(locale.getLanguage())) {
                ordered.add(locale);
            }
        }
        config.setLocales(new LocaleList(ordered.toArray(new Locale[0])));
        // Marks the locale as the user's choice, as Settings does, so it is persisted.
        try {
            Configuration.class.getField("userSetLocale").setBoolean(config, true);
        } catch (ReflectiveOperationException ignored) {
            // Absent on some builds; updatePersistentConfiguration persists regardless.
        }
        try {
            activityManager.getClass()
                    .getMethod("updatePersistentConfiguration", Configuration.class)
                    .invoke(activityManager, config);
        } catch (ReflectiveOperationException | SecurityException primary) {
            // The same call through Settings' own helper, for builds that hide the binder
            // method from apps but keep LocalePicker reachable.
            try {
                Class.forName("com.android.internal.app.LocalePicker")
                        .getMethod("updateLocales", LocaleList.class)
                        .invoke(null, config.getLocales());
            } catch (ReflectiveOperationException | SecurityException fallback) {
                return Protocol.error("locale_refused", describe(primary) + "; " + describe(fallback));
            }
        }
        Configuration applied = (Configuration) getConfiguration.invoke(activityManager);
        String now = applied == null || applied.getLocales().isEmpty()
                ? ""
                : applied.getLocales().get(0).toLanguageTag();
        return Protocol.ok().put("locale", now);
    }

    /** The exception class only (a SecurityException names the missing permission). */
    private static String describe(Throwable error) {
        Throwable cause = error.getCause() != null ? error.getCause() : error;
        String message = cause.getMessage();
        return cause.getClass().getSimpleName() + (message == null ? "" : ": " + message);
    }

    /** {@code ActivityManager.getService()} on API 26+, the only range this APK installs on. */
    private static Object activityManager() throws Exception {
        Method getService = Class.forName("android.app.ActivityManager").getMethod("getService");
        Object service = getService.invoke(null);
        if (service == null) {
            throw new IllegalStateException("activity manager unavailable");
        }
        return service;
    }
}
