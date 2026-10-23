package com.phloraxx.infraphone;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.net.Uri;
import org.json.JSONObject;
import java.io.File;
import java.util.Arrays;
import java.util.HashSet;

/** Owner-approved updates of this app only; installation always uses Android's installer. */
public final class InfraSelfUpdate {
    public static final String APK_PATH="apks/infra-phone-update.apk";
    public static final String ACTION_REVIEW="com.phloraxx.infraphone.REVIEW_UPDATE";
    private static final long MAX_BYTES=256L*1024*1024;

    public static File stagedFile(Context ctx) {
        return new File(ctx.getFilesDir(),"workspace/"+APK_PATH);
    }

    public static PackageInfo validate(Context ctx)throws Exception {
        File apk=stagedFile(ctx);
        if(!apk.isFile() || apk.length()<1024 || apk.length()>MAX_BYTES)
            throw new IllegalArgumentException("Transfer an Infra Phone APK to "+APK_PATH+" first");
        PackageManager pm=ctx.getPackageManager();
        PackageInfo candidate=pm.getPackageArchiveInfo(apk.getAbsolutePath(),
            PackageManager.GET_SIGNING_CERTIFICATES);
        if(candidate==null || !ctx.getPackageName().equals(candidate.packageName))
            throw new SecurityException("APK is not for this Infra Phone installation");
        PackageInfo installed=pm.getPackageInfo(ctx.getPackageName(),
            PackageManager.GET_SIGNING_CERTIFICATES);
        if(candidate.signingInfo==null || installed.signingInfo==null)
            throw new SecurityException("APK signing identity unavailable");
        Signature[] incoming=candidate.signingInfo.getApkContentsSigners();
        Signature[] current=installed.signingInfo.getApkContentsSigners();
        if(incoming==null || current==null || incoming.length==0 || current.length==0
            || incoming.length!=current.length
            || !new HashSet<>(Arrays.asList(incoming)).equals(new HashSet<>(Arrays.asList(current))))
            throw new SecurityException("APK signing certificate does not match installed Infra Phone");
        if(candidate.getLongVersionCode()<installed.getLongVersionCode())
            throw new SecurityException("Downgrades are not allowed");
        return candidate;
    }

    public static String description(Context ctx)throws Exception {
        PackageInfo apk=validate(ctx);
        return ctx.getPackageName()+" — "+apk.versionName+
            " (version code "+apk.getLongVersionCode()+")";
    }

    public static JSONObject status(Context ctx)throws Exception {
        File f=stagedFile(ctx);
        JSONObject result=new JSONObject().put("staged_path",APK_PATH)
            .put("staged_bytes",f.isFile()?f.length():0);
        if(f.isFile()){
            try{result.put("ready",true).put("candidate",description(ctx));}
            catch(Exception e){result.put("ready",false).put("reason",e.getMessage());}
        }else result.put("ready",false);
        return result;
    }

    public static Intent installIntent(Context ctx)throws Exception {
        validate(ctx);
        Uri uri=new Uri.Builder().scheme("content")
            .authority(ctx.getPackageName()+".stagedapk").appendPath("infra-phone-update.apk").build();
        Intent intent=new Intent(Intent.ACTION_VIEW);
        intent.setDataAndType(uri,"application/vnd.android.package-archive");
        intent.setClipData(ClipData.newRawUri("Infra Phone update",uri));
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        return intent;
    }

    public static void requestReview(Context ctx)throws Exception {
        PackageInfo candidate=validate(ctx);
        NotificationManager nm=(NotificationManager)ctx.getSystemService(Context.NOTIFICATION_SERVICE);
        if(!nm.areNotificationsEnabled())
            throw new IllegalStateException("Notifications off; open Infra Phone developer controls to review the update");
        nm.createNotificationChannel(new NotificationChannel("infra_updates",
            "Infra Phone update approvals",NotificationManager.IMPORTANCE_HIGH));
        Intent open=new Intent(ctx,DevToolsActivity.class).setAction(ACTION_REVIEW);
        PendingIntent pending=PendingIntent.getActivity(ctx,303,open,
            PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        Notification n=new Notification.Builder(ctx,"infra_updates")
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("Review Infra Phone update")
            .setContentText("Version "+candidate.versionName+" ready. Tap to review and install.")
            .setAutoCancel(true).setContentIntent(pending).build();
        nm.notify(303,n);
    }

    private InfraSelfUpdate(){}
}
