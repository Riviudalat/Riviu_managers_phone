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
     * Bind the port once a token has been supplied, and rebind if the token changed.
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
        String token = intent == null ? null : intent.getStringExtra(EXTRA_TOKEN);
        if (token == null || token.isEmpty()) {
            Log.w(TAG, "start without a token; the HTTP server stays down");
            return START_STICKY;
        }
        if (server != null && token.equals(activeToken)) {
            return START_STICKY;
        }
        if (server != null) {
            server.stop();
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
            Log.e(TAG, "HTTP bind failed", error);
            stopSelf();
        }
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
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
