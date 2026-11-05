package com.phloraxx.infraphone;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.os.Build;
import org.json.JSONObject;
import java.io.File;

/** Only commands scoped to the owner's SenseGuide development installation. No OS shell. */
public final class SenseGuideDevelopment {
    public static final String LAB="io.github.phloraxx.senseguide.lab";
    public static final String DEBUG="io.github.phloraxx.senseguide.lab.bridge";
    public static final String APK_PATH="apks/senseguide-debug.apk";
    public static final String ACTION_LAUNCH="launch";
    public static final String ACTION_INSTALL="install";

    public static JSONObject command(Context ctx,String command,String cwd)throws Exception {
        if(cwd!=null&&!cwd.isEmpty())return error("Working directories unsupported");
        command=command.trim();
        switch(command){
            case "help":return success("Supported: help | senseguide status | senseguide request-launch | senseguide request-install | senseguide workspace\nNo Android shell, ADB, root, or unrestricted commands.\n");
            case "senseguide status":return success(status(ctx).toString(2)+"\n");
            case "senseguide workspace":return success("Infra Phone private workspace: /senseguide/reports and /apks; use MCP files and transfer.\n");
            case "senseguide request-launch":
                notifyOwner(ctx,ACTION_LAUNCH);
                return success("A local confirmation notification was posted. Approve opening SenseGuide Diagnostics on the phone.\n");
            case "senseguide request-install":
                notifyOwner(ctx,ACTION_INSTALL);
                return success("Review and confirm the staged SenseGuide Diagnostics APK on the phone. Android installer confirmation is also mandatory.\n");
            default:return error("Command refused. Run 'help' for safe commands.");
        }
    }
    private static JSONObject success(String s)throws Exception{
        return new JSONObject().put("status","ok").put("body",
            new JSONObject().put("stdout",s).put("stderr","").put("exit_code",0).put("truncated",false));
    }
    private static JSONObject error(String s)throws Exception{
        return new JSONObject().put("status","failed").put("error",s);
    }
    public static JSONObject status(Context ctx)throws Exception {
        JSONObject a=new JSONObject();
        a.put("lab",installed(ctx,LAB));
        a.put("diagnostics",installed(ctx,DEBUG));
        File f=new File(ctx.getFilesDir(),"workspace/"+APK_PATH);
        a.put("staged_apk_bytes",f.isFile()?f.length():0);
        return a;
    }
    private static JSONObject installed(Context ctx,String packageName)throws Exception {
        JSONObject j=new JSONObject().put("package",packageName);
        try {
            PackageInfo p=ctx.getPackageManager().getPackageInfo(packageName,0);
            j.put("installed",true).put("version",p.versionName);
        }catch(PackageManager.NameNotFoundException e){j.put("installed",false);}
        return j;
    }
    public static String stagedApkPackage(Context ctx)throws Exception {
        File f=new File(ctx.getFilesDir(),"workspace/"+APK_PATH);
        if(!f.isFile() || f.length()>256L*1024*1024 || f.length()<1024)
            throw new IllegalArgumentException("Stage an APK first via signed transfer to "+APK_PATH);
        PackageInfo p=ctx.getPackageManager().getPackageArchiveInfo(f.getAbsolutePath(),0);
        if(p==null || !DEBUG.equals(p.packageName))
            throw new IllegalArgumentException("Only the separate SenseGuide Diagnostics package is accepted");
        return p.packageName+" ("+p.versionName+")";
    }
    public static void notifyOwner(Context ctx,String action){
        NotificationManager nm=(NotificationManager)ctx.getSystemService(Context.NOTIFICATION_SERVICE);
        nm.createNotificationChannel(new NotificationChannel("infra_dev_actions","SenseGuide development approvals",NotificationManager.IMPORTANCE_HIGH));
        Intent in=new Intent(ctx,DevToolsActivity.class).setAction(action);
        PendingIntent pi=PendingIntent.getActivity(ctx,90+action.hashCode(),in,
            PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT);
        Notification n=new Notification.Builder(ctx,"infra_dev_actions")
            .setSmallIcon(android.R.drawable.stat_sys_warning)
            .setContentTitle("SenseGuide development approval")
            .setContentText(ACTION_INSTALL.equals(action)?"Review staged APK installation":"Review SenseGuide launch")
            .setAutoCancel(true).setContentIntent(pi).build();
        nm.notify(ACTION_INSTALL.equals(action)?301:302,n);
    }
}
