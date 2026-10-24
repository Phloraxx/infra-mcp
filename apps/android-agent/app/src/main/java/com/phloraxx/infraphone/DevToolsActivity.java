package com.phloraxx.infraphone;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipData;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Restricted SenseGuide approvals and staged updates: minimal until needed. */
public final class DevToolsActivity extends Activity {
    private LinearLayout root;
    private TextView message;
    private boolean reviewedIntent;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        render();
    }

    @Override protected void onResume() {
        super.onResume();
        if(root!=null)render();
        if(!reviewedIntent){
            reviewedIntent=true;
            String action=getIntent().getAction();
            if(action!=null && (InfraSelfUpdate.ACTION_REVIEW.equals(action)
                ||SenseGuideDevelopment.ACTION_LAUNCH.equals(action)
                ||SenseGuideDevelopment.ACTION_INSTALL.equals(action))){
                root.post(()->{
                    if(InfraSelfUpdate.ACTION_REVIEW.equals(action))reviewInfraUpdate();
                    else if(SenseGuideDevelopment.ACTION_INSTALL.equals(action))reviewDiagnosticsUpdate();
                    else launchDiagnostics();
                });
            }
        }
    }

    private void render(){
        root=UiKit.screen(this);
        UiKit.header(root,"SenseGuide",R.drawable.infra_ic_back,this::finish);
        UiKit.addText(root,"Access, only when\nyou approve it.",26,UiKit.INK,true);
        UiKit.gap(root,12);
        UiKit.addText(root,"Screen access is restricted to SenseGuide and never permanent.",
            14,UiKit.MUTED,false);
        UiKit.gap(root,30);

        UiKit.section(root,"SCREEN ACCESS");
        LinearLayout access=UiKit.card(root);
        boolean granted=ScreenConsent.enabled(this);
        UiKit.addText(access,granted?"● Enabled for up to 1 hour":"○ Currently off",
            16,granted?UiKit.GOOD:UiKit.INK,true);
        UiKit.gap(access,9);
        UiKit.addText(access,
            "Requires a foreground SenseGuide session and Android Accessibility approval.",
            13,UiKit.MUTED,false);
        UiKit.gap(access,20);
        UiKit.button(access,granted?"Disable screen access":"Allow screen access for 1 hour",
            !granted,()->{
                if(granted){ScreenConsent.revoke(this);render();}
                else new AlertDialog.Builder(this)
                    .setTitle("Allow screen access for 1 hour?")
                    .setMessage("Only SenseGuide may be controlled while foreground. " +
                        "Screenshots and taps can reveal sensitive information. " +
                        "Disable access here at any time.")
                    .setNegativeButton("Cancel",null)
                    .setPositiveButton("Allow",(d,w)->{ScreenConsent.grant(this);render();})
                    .show();
            });
        UiKit.gap(access,5);
        UiKit.quietAction(access,"Android Accessibility settings",()->
            startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)));

        UiKit.gap(root,19);
        UiKit.section(root,"DIAGNOSTICS");
        UiKit.row(root,R.drawable.infra_ic_screen,
            "Open SenseGuide","Launch Diagnostics on this phone",this::launchDiagnostics);

        UiKit.gap(root,19);
        UiKit.section(root,"UPDATES");
        boolean pending=false;
        try{
            org.json.JSONObject status=InfraSelfUpdate.status(this);
            if(status.optLong("staged_bytes",0)>0){
                pending=true;
                UiKit.row(root,R.drawable.infra_ic_update,
                    "Infra Phone update",status.optBoolean("ready",false)?"Ready to review":"Needs a valid signed APK",
                    this::reviewInfraUpdate);
            }
        }catch(Exception ignored){}
        try{
            org.json.JSONObject s=SenseGuideDevelopment.status(this);
            if(s.optLong("staged_apk_bytes",0)>0){
                pending=true;
                UiKit.row(root,R.drawable.infra_ic_update,
                    "SenseGuide update","Review staged diagnostics APK",this::reviewDiagnosticsUpdate);
            }
        }catch(Exception ignored){}
        if(!pending)UiKit.addText(root,"No updates waiting.",14,UiKit.MUTED,false);
        UiKit.gap(root,20);
        message=UiKit.addText(root,"",13,UiKit.WARN,false);
    }

    private void launchDiagnostics(){
        approve("Open SenseGuide Diagnostics? Only the phone owner can approve this action.",
            ()->{
                Intent launch=getPackageManager().getLaunchIntentForPackage(SenseGuideDevelopment.DEBUG);
                if(launch==null)throw new IllegalStateException("Diagnostics isn't installed.");
                startActivity(launch);
            });
    }

    private void reviewDiagnosticsUpdate(){
        try{
            String staged=SenseGuideDevelopment.stagedApkPackage(this);
            approve("Install "+staged+"? Android will separately ask for approval.",
                this::installDiagnostics);
        }catch(Exception e){showError(e.getMessage());}
    }

    private void reviewInfraUpdate(){
        try{
            String staged=InfraSelfUpdate.description(this);
            approve("Install "+staged+"? Android will separately ask for approval.",
                this::installInfra);
        }catch(Exception e){showError(e.getMessage());}
    }

    private void installInfra(){
        try{
            if(needsInstallerPermission())return;
            startActivity(InfraSelfUpdate.installIntent(this));
        }catch(Exception e){showError(e.getMessage());}
    }

    private void installDiagnostics(){
        try{
            SenseGuideDevelopment.stagedApkPackage(this);
            if(needsInstallerPermission())return;
            Uri uri=new Uri.Builder().scheme("content")
                .authority(getPackageName()+".stagedapk").appendPath("senseguide-debug.apk").build();
            Intent intent=new Intent(Intent.ACTION_VIEW);
            intent.setDataAndType(uri,"application/vnd.android.package-archive");
            intent.setClipData(ClipData.newRawUri("SenseGuide Diagnostics update",uri));
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(intent);
        }catch(Exception e){showError(e.getMessage());}
    }

    private boolean needsInstallerPermission(){
        if(Build.VERSION.SDK_INT>=26&&!getPackageManager().canRequestPackageInstalls()){
            showError("Allow installs from Infra Phone, then return to review.");
            startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:"+getPackageName())));
            return true;
        }
        return false;
    }

    private void approve(String text,Runnable action){
        new AlertDialog.Builder(this).setTitle("Confirm on this phone")
            .setMessage(text).setNegativeButton("Cancel",null)
            .setPositiveButton("Continue",(dialog,which)->{
                try{action.run();}catch(Exception e){showError(e.getMessage());}
            }).show();
    }

    private void showError(String text){
        if(message!=null)message.setText(text==null?"Action unavailable.":text);
    }
}
