package com.riviu.agent;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

/**
 * Keeps the loopback HTTP server alive. Not a keyboard and not a launcher
 * activity — the desktop starts this with {@code am start-foreground-service}.
 */
public final class AgentService extends Service {
    private static final String TAG = "RiviuHelper";
    private static final String CHANNEL = "riviu-helper";
    private static final int NOTICE_ID = 17980;

    private HttpServer server;

    @Override
    public void onCreate() {
        super.onCreate();
        ensureChannel();
        Notification notification = new Notification.Builder(this, CHANNEL)
                .setContentTitle(getString(R.string.notification_title))
                .setContentText(getString(R.string.notification_text))
                .setSmallIcon(android.R.drawable.ic_menu_manage)
                .setOngoing(true)
                .build();
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(
                    NOTICE_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        } else {
            startForeground(NOTICE_ID, notification);
        }
        server = new HttpServer(this, Protocol.PORT);
        try {
            server.start();
        } catch (Exception error) {
            Log.e(TAG, "HTTP bind failed", error);
            stopSelf();
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        if (server != null) {
            server.stop();
            server = null;
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
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
