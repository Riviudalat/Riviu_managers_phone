package com.riviu.agent;

import android.app.Activity;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.view.View;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

/** Launcher information only: opening this activity never starts services or changes the IME. */
public final class MainActivity extends Activity {
    private static final int INK = 0xFF172033;
    private static final int SECONDARY = 0xFF475569;
    private static final int ACCENT = 0xFFC2410C;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private TextView statusTitle;
    private TextView statusDetail;
    private AgentService.LocalStatus displayedStatus;
    private final Runnable refresh = new Runnable() {
        @Override public void run() {
            updateStatus();
            handler.postDelayed(this, 1000);
        }
    };

    @Override public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setPadding(dp(24), dp(32), dp(24), dp(32));
        scroll.addView(content, new ScrollView.LayoutParams(-1, -2));

        LinearLayout brand = new LinearLayout(this);
        brand.setGravity(Gravity.CENTER_VERTICAL);
        ImageView logo = new ImageView(this);
        logo.setImageResource(R.drawable.riviu_logo);
        logo.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        brand.addView(logo, new LinearLayout.LayoutParams(dp(56), dp(56)));
        TextView name = text(R.string.app_name, 26, INK, true);
        LinearLayout.LayoutParams nameLayout = new LinearLayout.LayoutParams(0, -2, 1);
        nameLayout.setMarginStart(dp(16));
        brand.addView(name, nameLayout);
        content.addView(brand);
        addText(content, R.string.helper_purpose, 16, SECONDARY, false, 24);

        LinearLayout status = new LinearLayout(this);
        status.setOrientation(LinearLayout.VERTICAL);
        status.setPadding(dp(20), dp(20), dp(20), dp(20));
        GradientDrawable surface = new GradientDrawable();
        surface.setColor(0xFFFFFFFF);
        surface.setCornerRadius(dp(12));
        surface.setStroke(dp(1), 0xFFE1E5EB);
        status.setBackground(surface);
        LinearLayout.LayoutParams statusLayout = new LinearLayout.LayoutParams(-1, -2);
        statusLayout.topMargin = dp(28);
        content.addView(status, statusLayout);
        addText(status, R.string.status_heading, 13, SECONDARY, true, 0);
        statusTitle = addText(status, R.string.status_stopped, 20, INK, true, 12);
        statusTitle.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        statusDetail = addText(status, R.string.status_stopped_detail, 15, SECONDARY, false, 8);
        addText(content, R.string.status_scope, 13, SECONDARY, false, 12);

        addText(content, R.string.connection_heading, 17, INK, true, 32);
        addText(content, R.string.connection_description, 15, SECONDARY, false, 10);
        addText(content, R.string.helper_functions, 15, SECONDARY, false, 16);
        TextView version = text(R.string.app_name, 13, SECONDARY, false);
        version.setText(getString(R.string.helper_version, Protocol.AGENT_VERSION));
        LinearLayout.LayoutParams versionLayout = new LinearLayout.LayoutParams(-1, -2);
        versionLayout.topMargin = dp(32);
        content.addView(version, versionLayout);
        setContentView(scroll);
    }

    @Override protected void onResume() {
        super.onResume();
        handler.post(refresh);
    }

    @Override protected void onPause() {
        handler.removeCallbacks(refresh);
        super.onPause();
    }

    private void updateStatus() {
        AgentService.LocalStatus next = AgentService.localStatus();
        if (next == displayedStatus) return;
        displayedStatus = next;
        int title = R.string.status_stopped;
        int detail = R.string.status_stopped_detail;
        int color = SECONDARY;
        switch (next) {
            case RUNNING:
                title = R.string.status_running;
                detail = R.string.status_running_detail;
                color = 0xFF166534;
                break;
            case WAITING:
                title = R.string.status_waiting;
                detail = R.string.status_waiting_detail;
                color = ACCENT;
                break;
            case ERROR:
                title = R.string.status_error;
                detail = R.string.status_error_detail;
                color = 0xFFB91C1C;
                break;
            default:
                break;
        }
        statusTitle.setText(title);
        statusTitle.setTextColor(color);
        statusDetail.setText(detail);
    }

    private TextView addText(LinearLayout parent, int resource, int size, int color, boolean bold, int top) {
        TextView view = text(resource, size, color, bold);
        LinearLayout.LayoutParams layout = new LinearLayout.LayoutParams(-1, -2);
        layout.topMargin = dp(top);
        parent.addView(view, layout);
        return view;
    }

    private TextView text(int resource, int size, int color, boolean bold) {
        TextView view = new TextView(this);
        view.setText(resource);
        view.setTextSize(size);
        view.setTextColor(color);
        view.setTypeface(Typeface.create("sans-serif", bold ? Typeface.BOLD : Typeface.NORMAL));
        view.setLineSpacing(dp(3), 1f);
        return view;
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }
}
