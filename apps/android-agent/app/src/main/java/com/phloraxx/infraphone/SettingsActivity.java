package com.phloraxx.infraphone;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.widget.CheckBox;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;

/** Rarely needed controls live here, not on the connection home. */
public final class SettingsActivity extends Activity {
    private static final int PICK_FOLDER=21;
    private final Handler handler=new Handler(Looper.getMainLooper());
    private boolean localExpanded;
    private LinearLayout root;

    @Override public void onCreate(Bundle state){super.onCreate(state);render();}
    @Override protected void onResume(){super.onResume();if(root!=null)render();}

    private void render(){
        root=UiKit.screen(this);
        UiKit.header(root,"Settings",R.drawable.infra_ic_back,this::finish);
        UiKit.addText(root,"Everything else,\nwhen you need it.",26,UiKit.INK,true);
        UiKit.gap(root,11);
        UiKit.addText(root,"Your permissions and developer tools stay out of the way.",14,UiKit.MUTED,false);
        UiKit.gap(root,34);

        UiKit.section(root,"TOOLS");
        if("debug".equals(BuildConfig.BUILD_TYPE)){
            UiKit.row(root,R.drawable.infra_ic_terminal,
                "Android access","ADB connection and debugging",
                ()->startActivity(new Intent().setClassName(
                    this,"com.phloraxx.infraphone.AdbProbeActivity")));
        }
        UiKit.row(root,R.drawable.infra_ic_screen,
            "SenseGuide","Screen access, diagnostics and updates",
            ()->startActivity(new Intent(this,DevToolsActivity.class)));
        UiKit.row(root,R.drawable.infra_ic_share,
            "Local API",AgentService.isRunning()?"Running • advanced":"Off • advanced",
            ()->{localExpanded=!localExpanded;render();});

        if(localExpanded)showLocalApi();

        UiKit.gap(root,25);
        UiKit.section(root,"PREFERENCES");
        LinearLayout prefs=UiKit.card(root);
        CheckBox reboot=new CheckBox(this);
        reboot.setText("Reconnect after phone restart");
        reboot.setTextColor(UiKit.INK);reboot.setTextSize(14);
        reboot.setButtonTintList(android.content.res.ColorStateList.valueOf(UiKit.ACCENT));
        reboot.setChecked(getSharedPreferences("owner",MODE_PRIVATE).getBoolean("reconnect_boot",false));
        reboot.setOnCheckedChangeListener((button,enabled)->
            getSharedPreferences("owner",MODE_PRIVATE).edit()
                .putBoolean("reconnect_boot",enabled).apply());
        prefs.addView(reboot);
        UiKit.gap(prefs,6);
        UiKit.addText(prefs,"Requires Android to allow background startup.",12,UiKit.MUTED,false);

        UiKit.gap(root,10);
        UiKit.section(root,"DEVICE");
        UiKit.row(root,R.drawable.infra_ic_delete,
            "Forget this device","Remove the local Infra enrollment",
            this::unpair);
        UiKit.gap(root,23);
        UiKit.note(root,"Infra Phone "+BuildConfig.VERSION_NAME+"  •  Private by design");
    }

    private void showLocalApi(){
        LinearLayout api=UiKit.card(root);
        UiKit.addText(api,"Local development API",18,UiKit.INK,true);
        UiKit.gap(api,8);
        UiKit.addText(api,
            "Separate from your signed Infra connection. Intended only for development on your device or trusted network.",
            13,UiKit.MUTED,false);
        UiKit.gap(api,14);
        boolean running=AgentService.isRunning();
        UiKit.addText(api,running?"● Running":"○ Stopped",14,
            running?UiKit.GOOD:UiKit.MUTED,true);
        UiKit.gap(api,15);
        UiKit.button(api,running?"Stop local API":"Start local API",
            !running,()->{
                try{
                    Intent service=new Intent(this,AgentService.class)
                        .setAction(running?"STOP":"START");
                    if(!running && Build.VERSION.SDK_INT>=26)startForegroundService(service);
                    else startService(service);
                    handler.postDelayed(this::render,550);
                }catch(RuntimeException e){
                    Toast.makeText(this,"Android blocked this action.",Toast.LENGTH_LONG).show();
                }
            });
        if(running){
            UiKit.gap(api,5);
            UiKit.quietAction(api,"Copy local API key",()->{
                String token=AgentService.getToken();
                if(token==null || token.isEmpty()){
                    Toast.makeText(this,"Local API key unavailable",Toast.LENGTH_SHORT).show();return;
                }
                ((ClipboardManager)getSystemService(CLIPBOARD_SERVICE))
                    .setPrimaryClip(ClipData.newPlainText("Infra local API key",token));
                Toast.makeText(this,"Key copied. Keep it private.",Toast.LENGTH_SHORT).show();
            });
        }
        UiKit.gap(api,15);
        String uri=getSharedPreferences("owner",MODE_PRIVATE).getString("folder",null);
        UiKit.addText(api,uri==null?"No shared folder selected":"Folder permission granted",
            13,UiKit.MUTED,false);
        UiKit.gap(api,5);
        UiKit.quietAction(api,"Choose shared folder",()->{
            Intent picker=new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            picker.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION|
                Intent.FLAG_GRANT_WRITE_URI_PERMISSION|Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            startActivityForResult(picker,PICK_FOLDER);
        });
    }

    private void unpair(){
        if(GatewayAgentService.isActive()){
            Toast.makeText(this,"Disconnect Infra first.",Toast.LENGTH_LONG).show();return;
        }
        new AlertDialog.Builder(this)
            .setTitle("Forget this phone?")
            .setMessage("Removes the local signing identity. Also revoke this device in your Infra dashboard.")
            .setNegativeButton("Cancel",null)
            .setPositiveButton("Forget",(dialog,which)->{
                getSharedPreferences("owner",MODE_PRIVATE).edit().remove("reconnect_boot").apply();
                ScreenConsent.revoke(this);
                new DeviceIdentity(this).clear();
                finish();
            }).show();
    }

    @Override protected void onActivityResult(int request,int result,Intent data){
        super.onActivityResult(request,result,data);
        if(request!=PICK_FOLDER||result!=RESULT_OK||data==null||data.getData()==null)return;
        Uri uri=data.getData();
        int flags=data.getFlags()&(Intent.FLAG_GRANT_READ_URI_PERMISSION|
            Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        try{
            getContentResolver().takePersistableUriPermission(uri,flags);
            getSharedPreferences("owner",MODE_PRIVATE).edit()
                .putString("folder",uri.toString()).apply();
            render();
        }catch(SecurityException e){
            Toast.makeText(this,"Folder permission couldn't be saved",Toast.LENGTH_LONG).show();
        }
    }
}
