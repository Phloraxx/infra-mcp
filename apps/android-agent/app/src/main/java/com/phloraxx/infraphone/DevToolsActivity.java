package com.phloraxx.infraphone;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipData;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.os.Build;
import android.provider.Settings;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Owner-driven app launch, staged APK install, and optional screen permission controls. */
public final class DevToolsActivity extends Activity {
    private LinearLayout layout;
    private TextView status;
    @Override public void onCreate(Bundle state){
        super.onCreate(state);
        layout=new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(28,36,28,30);layout.setBackgroundColor(Color.rgb(15,22,39));
        title("SenseGuide development controls",23);
        title("Every app installation and launch is initiated here on the phone. Remote commands only request approval.",14);
        status=title("",14);
        button("Refresh status",this::refresh);
        button("Launch SenseGuide Diagnostics",()->approve("Launch diagnostics?",()->{
            Intent i=getPackageManager().getLaunchIntentForPackage(SenseGuideDevelopment.DEBUG);
            if(i==null)throw new IllegalStateException("SenseGuide Diagnostics isn't installed");
            startActivity(i);
        }));
        button("Review staged SenseGuide APK",()->{
            try{
                String info=SenseGuideDevelopment.stagedApkPackage(this);
                approve("Install "+info+"? Android will separately ask for installation approval.",this::install);
            }catch(Exception e){status.setText(e.getMessage());}
        });
        button("Review staged Infra Phone update",this::reviewInfraUpdate);
        button("Accessibility settings (optional screen control)",()->{
            startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS));
        });
        button("Enable 1-hour SenseGuide-only screen control",()->{
            new AlertDialog.Builder(this)
                .setTitle("Enable restricted remote screen controls?")
                .setMessage("Only while SenseGuide is foreground. Remote screenshots/taps can reveal camera views. Requires explicitly enabled Android Accessibility service. Expires after 1 hour; disable from this screen anytime.")
                .setNegativeButton("Cancel",null)
                .setPositiveButton("Enable",(d,w)->{ScreenConsent.grant(this);refresh();})
                .show();
        });
        button("Disable remote screen control",()->{
            ScreenConsent.revoke(this);
            refresh();
        });
        setContentView(layout);refresh();
        if(InfraSelfUpdate.ACTION_REVIEW.equals(getIntent().getAction()))
            layout.post(this::reviewInfraUpdate);
    }
    private TextView title(String t,int size) {
        TextView text=new TextView(this);text.setText(t);text.setTextColor(Color.WHITE);
        text.setTextSize(size);text.setPadding(0,12,0,18);layout.addView(text);return text;
    }
    private void button(String name,Runnable click) {
        Button b=new Button(this);b.setText(name);
        layout.addView(b);b.setOnClickListener(v->{try{click.run();}catch(Exception e){status.setText(e.getMessage());}});
    }
    private void approve(String message,Runnable task){
        new AlertDialog.Builder(this).setTitle("Owner approval").setMessage(message)
            .setNegativeButton("Cancel",null).setPositiveButton("Continue",(d,w)->{
                try{task.run();}catch(Exception e){status.setText(e.getMessage());}
            }).show();
    }
    private void refresh(){
        try {
            status.setText(SenseGuideDevelopment.status(this).toString(2)
                +"\nScreen control: "+(ScreenConsent.enabled(this)?"1-hour consent active":"disabled"));
        }catch(Exception e){status.setText(e.getMessage());}
    }
    private void reviewInfraUpdate() {
        try {
            String info=InfraSelfUpdate.description(this);
            approve("Install this Infra Phone update? "+info+". Android will ask you to confirm the installation.",
                this::installInfraUpdate);
        }catch(Exception e){status.setText("Update unavailable: "+e.getMessage());}
    }
    private void installInfraUpdate() {
        try {
            if(Build.VERSION.SDK_INT>=26&&!getPackageManager().canRequestPackageInstalls()){
                status.setText("Allow installs from Infra Phone, then return and tap Review staged Infra Phone update.");
                startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:"+getPackageName())));
                return;
            }
            startActivity(InfraSelfUpdate.installIntent(this));
        }catch(Exception e){status.setText("Update failed: "+e.getMessage());}
    }
    private void install() {
        try{
            SenseGuideDevelopment.stagedApkPackage(this);
            if(Build.VERSION.SDK_INT>=26&&!getPackageManager().canRequestPackageInstalls()){
                status.setText("Allow APK installation for Infra Phone, then return and tap Review.");
                startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:"+getPackageName())));
                return;
            }
            Uri uri=new Uri.Builder().scheme("content").authority(getPackageName()+".stagedapk")
                .appendPath("senseguide-debug.apk").build();
            Intent intent=new Intent(Intent.ACTION_VIEW);
            intent.setDataAndType(uri,"application/vnd.android.package-archive");
            intent.setClipData(ClipData.newRawUri("SenseGuide Diagnostics APK",uri));
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(intent);
        }catch(Exception e){status.setText(e.getMessage());}
    }
}
