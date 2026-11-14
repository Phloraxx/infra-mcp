package com.phloraxx.infraphone;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.os.Build;
import android.provider.Settings;

/**
 * Debug-build-only owner control over Android 11+ standard Wireless debugging.
 * Does NOT alter legacy ADB TCP mode, USB debugging, or developer options.
 * Nothing invokes this from Infra's remote command dispatcher.
 */
final class WirelessDebuggingControl {
    private static final String WIFI_ADB_SETTING = "adb_wifi_enabled";
    private static final String DEVELOPER_SETTING = "development_settings_enabled";

    private WirelessDebuggingControl() {}

    static boolean hasPermission(Context context) {
        return context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS)
            == PackageManager.PERMISSION_GRANTED;
    }

    static boolean isEnabled(Context context) {
        return Settings.Global.getInt(context.getContentResolver(), WIFI_ADB_SETTING, 0) == 1;
    }

    static String describe(Context context) {
        if (Build.VERSION.SDK_INT < 30) return "Wireless debugging requires Android 11+.";
        boolean dev = Settings.Global.getInt(context.getContentResolver(), DEVELOPER_SETTING, 0) == 1;
        return "Wireless debugging: " + (isEnabled(context) ? "on" : "off")
            + "  |  Permission: " + (hasPermission(context) ? "granted" : "needs one-time ADB grant")
            + (dev ? "" : "  |  Enable Developer options first");
    }

    static void setEnabled(Context context, boolean enable) {
        if (Build.VERSION.SDK_INT < 30)
            throw new IllegalStateException("Wireless debugging requires Android 11+.");
        if (!hasPermission(context))
            throw new SecurityException("First grant WRITE_SECURE_SETTINGS from a trusted ADB client");
        if (enable && Settings.Global.getInt(context.getContentResolver(), DEVELOPER_SETTING, 0) != 1)
            throw new IllegalStateException("Enable Android Developer options first");
        if (isEnabled(context) == enable) return;
        if (!Settings.Global.putInt(context.getContentResolver(), WIFI_ADB_SETTING, enable ? 1 : 0))
            throw new IllegalStateException("Android refused the Wireless debugging setting change");
        if (isEnabled(context) != enable)
            throw new IllegalStateException("Android did not retain the requested debugging state");
    }

    static String grantCommand(Context context) {
        return "adb shell pm grant " + context.getPackageName()
            + " android.permission.WRITE_SECURE_SETTINGS";
    }
}
