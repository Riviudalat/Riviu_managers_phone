package com.riviu.agent;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.os.Handler;
import android.os.Build;
import android.os.Looper;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.concurrent.TimeUnit;

/** Main-looper jobs retain settlement, rather than letting an expired queued write run later. */
final class ClipboardStore {
    static final class Value {
        final boolean available;
        final String text;
        final boolean written;
        final ClipboardCompare.Result comparison;
        boolean plainTextBaseline;
        String baselineId;
        String baselineKind;
        private Value(boolean available, String text, boolean written, ClipboardCompare.Result comparison) {
            this.available = available;
            this.text = text;
            this.written = written;
            this.comparison = comparison;
        }
        static Value read(String text) { return new Value(text != null, text, false, null); }
        static Value written() { return new Value(false, null, true, null); }
        static Value compared(ClipboardCompare.Result result) { return new Value(result.available, null, result.written, result); }
    }
    private final Context context;
    private final MainThreadJobs jobs;
    private final ClipboardSnapshots<ClipData> snapshots = new ClipboardSnapshots<ClipData>(32);
    private final ClipboardSnapshots.Access<ClipData> snapshotAccess = new ClipboardSnapshots.Access<ClipData>() {
        @Override public ClipData read() { return manager().getPrimaryClip(); }
        @Override public boolean canClear() { return Build.VERSION.SDK_INT >= 28; }
        @Override public boolean plain(ClipData clip) { return plainClip(clip); }
        @Override public String text(ClipData clip) { return clip.getItemAt(0).getText().toString(); }
        @Override public void write(ClipData original) {
            if (original != null) manager().setPrimaryClip(original);
            else {
                if (!canClear()) throw new IllegalStateException("native clipboard clear unavailable");
                manager().clearPrimaryClip();
            }
        }
        @Override public boolean equal(ClipData first, ClipData second) {
            return plainClip(first) && plainClip(second)
                    && first.getItemAt(0).getText().toString().equals(second.getItemAt(0).getText().toString())
                    && java.util.Objects.equals(first.getDescription().getLabel() == null ? null : first.getDescription().getLabel().toString(),
                            second.getDescription().getLabel() == null ? null : second.getDescription().getLabel().toString());
        }
    };
    ClipboardStore(Context context) {
        this.context = context;
        final Handler handler = new Handler(Looper.getMainLooper());
        jobs = new MainThreadJobs(new MainThreadJobs.Scheduler() {
            @Override public boolean post(Runnable task) { return handler.post(task); }
            @Override public void remove(Runnable task) { handler.removeCallbacks(task); }
        }, System::nanoTime, 128);
    }
    MainThreadJobs.Job<Value> submit(String id, boolean set, String text) {
        String fingerprint = fingerprint(set ? "set" : "get", null, text);
        return jobs.submit(id, fingerprint, () -> {
            ClipboardManager manager = manager();
            if (set) {
                manager.setPrimaryClip(ClipData.newPlainText("riviu", text));
                return Value.written();
            }
            ClipData clip = manager.getPrimaryClip();
            Value result = Value.read(plainClip(clip) ? clip.getItemAt(0).getText().toString() : null);
            result.plainTextBaseline = plainClip(clip);
            result.baselineKind = snapshots.kind(clip, snapshotAccess);
            return result;
        }, 5, TimeUnit.SECONDS);
    }
    MainThreadJobs.Job<Value> compareSet(String id, String expectedText, String text) {
        return jobs.submit(id, fingerprint("compareSet", expectedText, text), () ->
            Value.compared(ClipboardCompare.apply(new ClipboardCompare.Access() {
                @Override public String read() {
                    ClipData clip = manager().getPrimaryClip();
                    if (clip == null || clip.getItemCount() == 0) return null;
                    CharSequence value = clip.getItemAt(0).coerceToText(context);
                    return value == null ? null : value.toString();
                }
                @Override public void write(String replacement) {
                    manager().setPrimaryClip(ClipData.newPlainText("riviu", replacement));
                }
            }, expectedText, text)), 5, TimeUnit.SECONDS);
    }
    MainThreadJobs.Job<Value> snapshot(String id) {
        return jobs.submit(id, fingerprint("snapshot", null, null), () -> {
            ClipData original = manager().getPrimaryClip();
            Value value = Value.read(original == null ? null : plainClip(original) ? original.getItemAt(0).getText().toString() : null);
            value.plainTextBaseline = plainClip(original);
            // Set only after a successful native read; read exceptions remain failed jobs.
            value.baselineKind = snapshots.kind(original, snapshotAccess);
            value.baselineId = snapshots.capture(original, snapshotAccess);
            return value;
        }, 5, TimeUnit.SECONDS);
    }
    MainThreadJobs.Job<Value> restoreSnapshot(String id, String baselineId, String expectedText) {
        if (!snapshots.known(baselineId)) throw new IllegalArgumentException("snapshot_unknown");
        return jobs.submit(id, fingerprint("restoreSnapshot", baselineId, expectedText),
                () -> Value.compared(snapshots.restore(baselineId, expectedText, snapshotAccess)), 5, TimeUnit.SECONDS);
    }
    static boolean plainClip(ClipData clip) {
        if (clip == null || clip.getItemCount() != 1 || clip.getDescription() == null
                || clip.getDescription().getMimeTypeCount() != 1
                || !"text/plain".equals(clip.getDescription().getMimeType(0))) return false;
        ClipData.Item item = clip.getItemAt(0);
        CharSequence text = item.getText();
        return text instanceof String && !(text instanceof android.text.Spanned)
                && item.getUri() == null && item.getIntent() == null && item.getHtmlText() == null;
    }
    MainThreadJobs.Job<?> find(String id) { return jobs.find(id); }
    int unsettled() { return jobs.unsettled(); }
    void closeAdmission() { jobs.closeAdmission(); }
    private ClipboardManager manager() {
        ClipboardManager manager = (ClipboardManager) context.getSystemService(Context.CLIPBOARD_SERVICE);
        if (manager == null) throw new IllegalStateException("ClipboardManager unavailable");
        return manager;
    }
    static String fingerprint(String action, String expected, String text) {
        try {
            String a = expected == null ? "" : expected;
            String replacement = text == null ? "" : text;
            // Length framing makes embedded separators unambiguous and binds BOTH values.
            byte[] digest = MessageDigest.getInstance("SHA-256").digest(
                    (action.length() + ":" + action + a.length() + ":" + a + replacement.length() + ":" + replacement)
                            .getBytes(StandardCharsets.UTF_8));
            StringBuilder hex = new StringBuilder();
            for (byte b : digest) hex.append(String.format(java.util.Locale.ROOT, "%02x", b & 255));
            return hex.toString();
        } catch (java.security.NoSuchAlgorithmException e) {
            throw new IllegalStateException("SHA-256 unavailable");
        }
    }
}
