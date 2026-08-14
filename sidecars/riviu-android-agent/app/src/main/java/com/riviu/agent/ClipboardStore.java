package com.riviu.agent;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.os.Handler;
import android.os.Looper;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Clipboard access for this UID. On Android 10+ a read only succeeds when this
 * app is the current IME (or the focused app). The desktop driver enables this
 * IME for the duration of one request and then restores the previous IME.
 *
 * ClipboardManager must run on a thread that has a Looper. The HTTP accept
 * loop does not — hopping to the main looper is load-bearing, not style.
 */
final class ClipboardStore {
    private ClipboardStore() {}

    static void setText(final Context context, final String text) {
        runOnMain(new Runnable() {
            @Override
            public void run() {
                manager(context).setPrimaryClip(
                        ClipData.newPlainText("riviu", text == null ? "" : text));
            }
        });
    }

    static String getText(final Context context) {
        final AtomicReference<String> out = new AtomicReference<String>("");
        runOnMain(new Runnable() {
            @Override
            public void run() {
                ClipData clip = manager(context).getPrimaryClip();
                if (clip == null || clip.getItemCount() == 0) {
                    return;
                }
                CharSequence text = clip.getItemAt(0).coerceToText(context);
                out.set(text == null ? "" : text.toString());
            }
        });
        return out.get();
    }

    private static ClipboardManager manager(Context context) {
        ClipboardManager clipboard =
                (ClipboardManager) context.getSystemService(Context.CLIPBOARD_SERVICE);
        if (clipboard == null) {
            throw new IllegalStateException("ClipboardManager is missing");
        }
        return clipboard;
    }

    private static void runOnMain(final Runnable action) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            action.run();
            return;
        }
        final CountDownLatch done = new CountDownLatch(1);
        final AtomicReference<RuntimeException> error = new AtomicReference<RuntimeException>();
        new Handler(Looper.getMainLooper()).post(new Runnable() {
            @Override
            public void run() {
                try {
                    action.run();
                } catch (RuntimeException e) {
                    error.set(e);
                } finally {
                    done.countDown();
                }
            }
        });
        try {
            if (!done.await(5, TimeUnit.SECONDS)) {
                throw new IllegalStateException("main-thread clipboard timed out");
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IllegalStateException("main-thread clipboard interrupted", e);
        }
        if (error.get() != null) {
            throw error.get();
        }
    }
}
