package com.phloraxx.infraphone;

import android.content.Context;
import android.os.SystemClock;
import android.provider.Settings;

/** Foreground-only one-hour lease; never survives reboot or explicit revoke. */
public final class ScreenConsent {
    private static final long CONSENT_DURATION_MS=60L*60L*1000L;
    private static int bootCount(Context c) {
        try{return Settings.Global.getInt(c.getContentResolver(),Settings.Global.BOOT_COUNT,-1);}
        catch(Exception e){return -1;}
    }
    public static void grant(Context c) {
        c.getSharedPreferences("dev-control",Context.MODE_PRIVATE).edit()
            .putLong("start_elapsed",SystemClock.elapsedRealtime())
            .putLong("until_elapsed",SystemClock.elapsedRealtime()+CONSENT_DURATION_MS)
            .putLong("until_wall",System.currentTimeMillis()+CONSENT_DURATION_MS)
            .putInt("boot_count",bootCount(c)).apply();
    }
    public static void revoke(Context c) {
        c.getSharedPreferences("dev-control",Context.MODE_PRIVATE).edit().clear().apply();
    }
    public static boolean enabled(Context c){
        android.content.SharedPreferences s=c.getSharedPreferences("dev-control",Context.MODE_PRIVATE);
        long start=s.getLong("start_elapsed",-1);
        long until=s.getLong("until_elapsed",0);
        long wall=s.getLong("until_wall",0);
        long now=SystemClock.elapsedRealtime();
        return GatewayAgentService.isActive() && start>=0 && now>=start && now<until
            && System.currentTimeMillis()<wall && s.getInt("boot_count",-9)==bootCount(c);
    }
}
