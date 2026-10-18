package com.phloraxx.infraphone;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.util.Log;

/** Opt-in boot resume. Does not bypass OS battery/start restrictions or user Disconnect. */
public final class BootConnectReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context ctx,Intent intent) {
        if(intent==null || !Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction()))return;
        if(!ctx.getSharedPreferences("owner",Context.MODE_PRIVATE).getBoolean("reconnect_boot",false))return;
        if(!new DeviceIdentity(ctx).enrolled())return;
        try {
            Intent service=new Intent(ctx,GatewayAgentService.class)
                .setAction(GatewayAgentService.ACTION_CONNECT);
            if(Build.VERSION.SDK_INT>=26)ctx.startForegroundService(service);
            else ctx.startService(service);
        }catch(RuntimeException err) {
            Log.w("InfraPhone","Android blocked opt-in boot reconnection: "+err.getClass().getSimpleName());
        }
    }
}
