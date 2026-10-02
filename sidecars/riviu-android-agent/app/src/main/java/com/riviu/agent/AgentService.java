package com.riviu.agent;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.os.Handler;
import android.os.Looper;
import org.json.JSONObject;
import java.util.concurrent.TimeUnit;
import android.util.Log;

/**
 * Keeps the loopback HTTP server alive. The desktop starts this with
 * {@code am start-foreground-service}; the launcher only observes local status.
 */
public final class AgentService extends Service {
    private static final String TAG = "RiviuHelper";
    private static final String CHANNEL = "riviu-helper";
    private static final int NOTICE_ID = 17980;
    enum LocalStatus { STOPPED, WAITING, RUNNING, ERROR }
    private static volatile AgentService activeService;
    private static volatile LocalStatus localStatus = LocalStatus.STOPPED;

    /** Intent extra the desktop passes the shared token in. */
    public static final String EXTRA_TOKEN = "token";

    private HttpServer server;
    private String activeToken;
    private final ServiceStartPolicy starts = new ServiceStartPolicy();
    private final OwnerSession ownership = new OwnerSession();
    private final Handler main = new Handler(Looper.getMainLooper());
    private BootstrapServer bootstrap;
    private boolean destroyed;

    static LocalStatus localStatus() {
        AgentService active = activeService;
        if (localStatus == LocalStatus.RUNNING
                && (active == null || active.server == null || !active.server.isRunning())) {
            return LocalStatus.ERROR;
        }
        return localStatus;
    }

    @Override
    public void onCreate() {
        super.onCreate();
        activeService = this;
        localStatus = LocalStatus.WAITING;
        ensureChannel();
        Notification notification = notification(false);
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(
                    NOTICE_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        } else {
            startForeground(NOTICE_ID, notification);
        }
        // The server is deliberately NOT started here. It needs the shared token, and the
        // token arrives on the Intent, which `onCreate` does not see — so binding the port
        // before `onStartCommand` would mean binding it unauthenticated.
    }

    /**
     * Bind after a legacy token is supplied; never treat a different token as takeover proof.
     *
     * <p>No token, no server. That is the safe direction and it is the point: a helper that any
     * app could start with a bare {@code am start-foreground-service} used to come up serving
     * every endpoint to the whole phone. Now such a start produces a foreground service that
     * listens to nothing.
     *
     * <p>{@code START_STICKY} can hand back a null Intent after the process is killed, which
     * leaves the token behind. That case fails closed too; the desktop re-issues the start on
     * its next {@code ensure}.
     */
    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && intent.hasExtra("bootstrapSocket")) {
            startBootstrap(intent);
            return START_STICKY;
        }
        if (ownership.owner() != null) return START_STICKY; // Legacy argv can never rekey a secure owner.
        String token = intent == null ? null : intent.getStringExtra(EXTRA_TOKEN);
        ServiceStartPolicy.Decision decision = starts.decide(token, activeToken, server != null,
                server != null && server.isRunning(), server != null && server.hasOutstandingWork());
        if (decision == ServiceStartPolicy.Decision.WAIT) {
            // Warm tokenless start preserves the existing session; cold sticky restart stays waiting.
            if (activeToken == null) localStatus = LocalStatus.WAITING;
            return START_STICKY;
        }
        if (decision == ServiceStartPolicy.Decision.KEEP) return START_STICKY;
        if (decision == ServiceStartPolicy.Decision.REFUSE_OWNER
                || decision == ServiceStartPolicy.Decision.REFUSE_BUSY
                || decision == ServiceStartPolicy.Decision.REFUSE_RECOVERY) {
            Log.w(TAG, "start refused: existing session requires reconciliation");
            return START_STICKY;
        }
        if (server != null) {
            server.stop(); // Closes admission synchronously; effects drain on their own threads.
            if (server.hasOutstandingWork()) return START_STICKY;
            server = null;
        }
        HttpServer fresh = new HttpServer(this, Protocol.PORT, token);
        try {
            fresh.start();
            server = fresh;
            activeToken = token;
            localStatus = LocalStatus.RUNNING;
            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) manager.notify(NOTICE_ID, notification(true));
        } catch (Exception error) {
            localStatus = LocalStatus.ERROR;
            fresh.stop();
            Log.e(TAG, "HTTP bind failed; session requires reconciliation");
        }
        return START_STICKY;
    }

    private void startBootstrap(Intent intent) {
        String socket = intent.getStringExtra("bootstrapSocket");
        String nonce = intent.getStringExtra("bootstrapNonce");
        String owner = intent.getStringExtra("bootstrapOwnerId");
        try {
            RequestAuth.requireId(nonce);
            RequestAuth.requireId(owner);
            if (socket == null || !socket.matches("[A-Za-z0-9_.-]{16,100}")) return;
            // Do not cancel another pending bootstrap or swap its nonce.
            if (bootstrap != null) return;
            final BootstrapServer[] ownListener = new BootstrapServer[1];
            boolean debuggable = (getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0;
            if (!debuggable) return; // run-as transport is a debug-canary contract, never a release fallback.
            ownListener[0] = new BootstrapServer(socket, nonce, owner, debuggable, main, (payload, reply) -> {
                handleBootstrap(payload, response -> {
                    // Clear only this one-shot admission BEFORE making its ACK visible. A release
                    // start immediately after claim ACK cannot hit a stale bootstrap field.
                    if (bootstrap == ownListener[0]) bootstrap = null;
                    reply.send(response);
                });
            }, () -> { if (bootstrap == ownListener[0]) bootstrap = null; });
            bootstrap = ownListener[0];
        } catch (Exception ignored) { Log.w(TAG, "bootstrap listener unavailable"); }
    }

    private JSONObject bootstrapResponse(JSONObject request, boolean ok, String state) {
        try {
            JSONObject response = new JSONObject().put("ok", ok).put("nonce", request.optString("nonce"))
                    .put("ownerId", request.optString("ownerId")).put("state", state);
            if (ownership.instance() != null) response.put("serviceInstance", ownership.instance())
                    .put("ownerGeneration", ownership.generation());
            if (!ok) response.put("error", state);
            return response;
        } catch (Exception e) { return new JSONObject(); }
    }

    private void handleBootstrap(JSONObject request, BootstrapServer.Reply reply) {
        if (destroyed) { reply.send(bootstrapResponse(request, false, "service_destroyed")); return; }
        String owner = request.optString("ownerId");
        String token = request.optString("token");
        String action = request.optString("action");
        if ("claim".equals(action)) {
            OwnerSession.Claim claim = ownership.claim(owner, token, activeToken != null && ownership.owner() == null);
            if (claim == OwnerSession.Claim.CONFLICT) {
                reply.send(bootstrapResponse(request, false, "owner_conflict")); return;
            }
            if (claim == OwnerSession.Claim.RESUME && (server == null || !server.isRunning())
                    && !ownership.recover()) {
                reply.send(bootstrapResponse(request, false, "reconciliation_required")); return;
            }
            if (server != null && !server.isRunning()) {
                if (server.hasOutstandingWork()) {
                    reply.send(bootstrapResponse(request, false, "reconciliation_required")); return;
                }
                server.stop();
                server = null;
            }
            if (server == null) {
                HttpServer fresh = new HttpServer(this, Protocol.PORT, token, ownership.instance(), owner, ownership.generation());
                try { fresh.start(); server = fresh; activeToken = token; }
                catch (Exception e) {
                    fresh.stop(); localStatus = LocalStatus.ERROR;
                    reply.send(bootstrapResponse(request, false, "bind_failed")); return;
                }
            }
            localStatus = LocalStatus.RUNNING;
            reply.send(bootstrapResponse(request, true, "ready"));
        } else if ("release".equals(action)) {
            if (!ownership.release(owner, token, request.optString("serviceInstance", ""))) {
                reply.send(bootstrapResponse(request, false, "release_refused")); return;
            }
            final HttpServer releasing = server;
            if (releasing != null) releasing.stop();
            final JSONObject settledReply = bootstrapResponse(request, true, "released");
            new Thread(() -> {
                boolean settled = false;
                try { settled = releasing == null || releasing.awaitSettlement(2500, TimeUnit.MILLISECONDS); }
                catch (InterruptedException e) { Thread.currentThread().interrupt(); }
                final boolean safe = settled;
                main.post(() -> {
                    if (destroyed || !ownership.finishRelease(safe)) {
                        localStatus = LocalStatus.ERROR;
                        reply.send(bootstrapResponse(request, false, "reconciliation_required"));
                        return;
                    }
                    server = null; activeToken = null; localStatus = LocalStatus.WAITING;
                    reply.send(settledReply);
                });
            }, "riviu-owned-release").start();
        } else reply.send(bootstrapResponse(request, false, "invalid_action"));
    }

    @Override
    public void onDestroy() {
        destroyed = true;
        if (bootstrap != null) bootstrap.close();
        if (server != null) {
            server.stop();
            server = null;
        }
        activeService = null;
        if (localStatus != LocalStatus.ERROR) localStatus = LocalStatus.STOPPED;
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private Notification notification(boolean running) {
        Intent open = new Intent(this, MainActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent content = PendingIntent.getActivity(this, 0, open,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this, CHANNEL)
                .setContentTitle(getString(R.string.notification_title))
                .setContentText(getString(running ? R.string.notification_text : R.string.notification_waiting))
                .setSmallIcon(R.drawable.ic_notification)
                .setColor(0xFFC2410C)
                .setContentIntent(content)
                .setOngoing(true)
                .build();
    }

    private void ensureChannel() {
        if (Build.VERSION.SDK_INT < 26) {
            return;
        }
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager == null || manager.getNotificationChannel(CHANNEL) != null) {
            return;
        }
        NotificationChannel channel = new NotificationChannel(
                CHANNEL,
                getString(R.string.notification_title),
                NotificationManager.IMPORTANCE_LOW);
        channel.setDescription(getString(R.string.notification_text));
        manager.createNotificationChannel(channel);
    }
}
