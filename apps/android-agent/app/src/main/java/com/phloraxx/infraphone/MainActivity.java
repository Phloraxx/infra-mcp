package com.phloraxx.infraphone;

import android.Manifest;
import android.app.Activity;
import android.content.*;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.view.Gravity;
import android.view.View;
import android.widget.*;

public class MainActivity extends Activity {
    private static final int PICK_FOLDER = 21;
    private final int bg = Color.rgb(12,18,32), panel = Color.rgb(22,32,52), fg = Color.WHITE;
    private TextView status, token, hint, folder, gatewayStatus;
    private EditText gatewayInput, pairingInput;
    private LinearLayout root;
    private final android.os.Handler uiHandler=new android.os.Handler(android.os.Looper.getMainLooper());
    private final Runnable statusPoll=new Runnable(){public void run(){if(root!=null)refresh();uiHandler.postDelayed(this,2500);}};

    @Override public void onCreate(Bundle b) {
        super.onCreate(b);
        getWindow().setStatusBarColor(bg);
        getWindow().setNavigationBarColor(bg);
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(bg);
        root = new LinearLayout(this);
        root.setPadding(dp(22),dp(28),dp(22),dp(30));
        root.setOrientation(LinearLayout.VERTICAL);
        scroll.addView(root);
        setContentView(scroll);

        label("INFRA / ANDROID", 13, 0xff85A9FF, true);
        label("Your phone.\nYour infrastructure.", 30, fg, true);
        label("A local, permission-based development companion. No root or wireless debugging needed for normal operation.", 14, 0xffA8B9D4, false);
        space(20);

        LinearLayout state = card();
        status = labelIn(state, "", 19, fg, true);
        hint = labelIn(state, "", 13, 0xffA8B9D4, false);
        token = labelIn(state, "", 12, 0xffA7E6D7, false);
        button(state,"Copy session key",0xff293C5E,()-> {
            String t=AgentService.getToken();
            if(t==null) return;
            ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE))
                .setPrimaryClip(ClipData.newPlainText("Infra Phone key",t));
            Toast.makeText(this,"Session key copied",Toast.LENGTH_SHORT).show();
        });
        space(14);

        LinearLayout gateway=card();
        labelIn(gateway,"Direct Infra-MCP connection (v0.3)",18,fg,true);
        labelIn(gateway,"Enroll this Android phone as a separate signed device. For production, enter the trusted HTTPS gateway address.",13,0xffA8B9D4,false);
        gatewayInput=new EditText(this);
        gatewayInput.setSingleLine(true);gatewayInput.setTextColor(fg);gatewayInput.setHintTextColor(0xff8A9DBB);
        gatewayInput.setText(new DeviceIdentity(this).gateway());
        gatewayInput.setHint("https://your-infra-gateway");
        gatewayInput.setTextSize(14);gateway.addView(gatewayInput);
        pairingInput=new EditText(this);pairingInput.setTextColor(fg);pairingInput.setSingleLine(true);
        pairingInput.setHintTextColor(0xff8A9DBB);pairingInput.setHint("One-time pairing code");
        pairingInput.setTextSize(14);gateway.addView(pairingInput);
        gatewayStatus=labelIn(gateway,"",13,0xffA7E6D7,false);
        button(gateway,"Enroll phone",0xff3365E8,()-> {
            String url=gatewayInput.getText().toString(),code=pairingInput.getText().toString();
            gatewayStatus.setText("Enrolling…");
            new Thread(()-> {
                try {
                    GatewayEnrollment.pair(getApplicationContext(),url,code);
                    runOnUiThread(()->{pairingInput.setText("");refresh();});
                }catch(Exception e){runOnUiThread(()->gatewayStatus.setText("Enrollment failed: "+e.getMessage()));}
            },"infra-enroll").start();
        });
        button(gateway,"Connect signed agent",0xff3365E8,()-> {
            if(!new DeviceIdentity(this).enrolled()){
                gatewayStatus.setText("Enroll with a one-time code first.");return;
            }
            if(Build.VERSION.SDK_INT>=33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)
                requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},31);
            Intent svc=new Intent(this,GatewayAgentService.class).setAction(GatewayAgentService.ACTION_CONNECT);
            if(Build.VERSION.SDK_INT>=26)startForegroundService(svc);else startService(svc);
            root.postDelayed(this::refresh,900);
        });
        button(gateway,"Disconnect signed agent",0xff663845,()->{
            stopService(new Intent(this,GatewayAgentService.class));
            root.postDelayed(this::refresh,300);
        });
        button(gateway,"Unpair (local identity)",0xff663845,()->{
            if(GatewayAgentService.isActive()){
                gatewayStatus.setText("Disconnect first, then unpair.");return;
            }
            new android.app.AlertDialog.Builder(this)
                .setTitle("Unpair this device locally?")
                .setMessage("You must also revoke the device in the Infra gateway dashboard. This clears the local signing identity.")
                .setNegativeButton("Cancel",null)
                .setPositiveButton("Unpair",(d,w)->{
                    getSharedPreferences("owner",MODE_PRIVATE).edit().remove("reconnect_boot").apply();
                    getSharedPreferences("dev-control",MODE_PRIVATE).edit().remove("until").apply();
                    new DeviceIdentity(this).clear();refresh();})
                .show();
        });
        space(14);

        LinearLayout advanced=card();
        labelIn(advanced,"Developer controls and updates",18,fg,true);
        labelIn(advanced,"Signed transfers, owner-approved Infra Phone updates, SenseGuide APK review and optional SenseGuide-only Accessibility controls. Installations are never silent.",13,0xffA8B9D4,false);
        button(advanced,"Open developer controls",0xff3365E8,()->
            startActivity(new Intent(this,DevToolsActivity.class)));
        if(BuildConfig.DEBUG)button(advanced,"Local ADB pairing lab (experimental)",0xff293C5E,()->
            startActivity(new Intent().setClassName(getPackageName(),getPackageName()+".AdbProbeActivity")));
        CheckBox reboot=new CheckBox(this);
        reboot.setText("Reconnect signed gateway after phone reboot (opt-in)");
        reboot.setTextColor(fg);
        reboot.setChecked(getSharedPreferences("owner",MODE_PRIVATE).getBoolean("reconnect_boot",false));
        reboot.setOnCheckedChangeListener((checkbox,enabled)->
            getSharedPreferences("owner",MODE_PRIVATE).edit().putBoolean("reconnect_boot",enabled).apply());
        advanced.addView(reboot);
        space(14);

        LinearLayout controls = card();
        labelIn(controls, "Remote agent", 18, fg, true);
        labelIn(controls, "Start only when needed. API access is limited to localhost and your Tailscale VPN IP.",13,0xffA8B9D4,false);
        button(controls, "Start agent", 0xff3365E8, () -> {
            if (Build.VERSION.SDK_INT>=33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)
                requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},30);
            Intent i=new Intent(this,AgentService.class);
            i.setAction("START");
            if(Build.VERSION.SDK_INT>=26) startForegroundService(i);
            else startService(i);
            root.postDelayed(this::refresh,550);
        });
        button(controls, "Stop agent", 0xff663845, () -> {
            Intent i=new Intent(this,AgentService.class);
            i.setAction("STOP");startService(i);
            root.postDelayed(this::refresh,300);
        });
        space(14);

        LinearLayout access = card();
        labelIn(access,"Permission-based data access",18,fg,true);
        labelIn(access,"Choose a folder to share. All other app data remains protected by Android.",13,0xffA8B9D4,false);
        folder=labelIn(access,"",12,0xffA7E6D7,false);
        button(access, "Choose shared folder",0xff293C5E,()->{
            Intent in=new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            in.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION|Intent.FLAG_GRANT_WRITE_URI_PERMISSION|
                Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            startActivityForResult(in,PICK_FOLDER);
        });
        space(14);

        LinearLayout dev=card();
        labelIn(dev,"Development + hosting",18,fg,true);
        labelIn(dev,"Upload HTML, CSS, JS, and other files into the app workspace using the authenticated API. Files in site/ are served at /site/. This is not a root terminal.",13,0xffA8B9D4,false);
        labelIn(dev,"API port 8765  •  /api/status  •  /api/files  •  /api/apps",12,0xff85A9FF,false);
        labelIn(dev,"ADB USB test: adb forward tcp:8765 tcp:8765",12,0xff85A9FF,false);
        button(dev,"Refresh status",0xff293C5E,this::refresh);
        space(15);
        label("Only use this agent on your own devices. Keep the session key private. Stop the agent when finished.",12,0xff8397B3,false);
        refresh();
    }
    @Override protected void onResume(){super.onResume();if(root!=null){uiHandler.removeCallbacks(statusPoll);uiHandler.post(statusPoll);}}
    @Override protected void onPause(){uiHandler.removeCallbacks(statusPoll);super.onPause();}
    @Override protected void onActivityResult(int request,int result,Intent data){
        super.onActivityResult(request,result,data);
        if(request==PICK_FOLDER && result==RESULT_OK && data!=null && data.getData()!=null){
            Uri uri=data.getData();
            int flags=data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION|Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            try {getContentResolver().takePersistableUriPermission(uri,flags);}
            catch(Exception e){Toast.makeText(this,"Could not persist folder permission",Toast.LENGTH_LONG).show();return;}
            getSharedPreferences("owner",MODE_PRIVATE).edit().putString("folder",uri.toString()).apply();
            refresh();
        }
    }
    private void refresh(){
        boolean active=AgentService.isRunning();
        status.setText(active?"●  Agent running":"○  Agent stopped");
        status.setTextColor(active?0xff83E8BB:0xffE4B9C4);
        hint.setText(active? "Endpoints: " + AgentService.getBindings() : "No remote API is listening.");
        String key=AgentService.getToken();
        token.setText(key==null?"":"Session key (private):\n"+key);
        String uri=getSharedPreferences("owner",MODE_PRIVATE).getString("folder",null);
        folder.setText(uri==null?"No shared folder selected":"Shared folder permission granted");
        if(gatewayStatus!=null){
          DeviceIdentity ident=new DeviceIdentity(this);
          gatewayStatus.setText((ident.enrolled()?"Enrolled: "+ident.name():"Not enrolled")
              +"\nAgent: "+GatewayAgentService.connectionState()
              +(GatewayAgentService.error().isEmpty()?"":"\n"+GatewayAgentService.error()));
        }
    }
    private LinearLayout card(){
        LinearLayout l=new LinearLayout(this);
        l.setOrientation(LinearLayout.VERTICAL);
        l.setPadding(dp(17),dp(16),dp(17),dp(16));
        GradientDrawable d=new GradientDrawable();
        d.setColor(panel);d.setCornerRadius(dp(18));
        d.setStroke(dp(1),0xff334966);
        l.setBackground(d);
        root.addView(l,new LinearLayout.LayoutParams(-1,-2));
        return l;
    }
    private TextView label(String text,int size,int color,boolean bold){
        TextView t=new TextView(this);
        t.setText(text); t.setTextSize(size);t.setTextColor(color);
        if(bold)t.setTypeface(Typeface.DEFAULT,Typeface.BOLD);
        t.setPadding(0,dp(3),0,dp(7));
        root.addView(t);return t;
    }
    private TextView labelIn(LinearLayout parent,String text,int size,int color,boolean bold){
        TextView t=new TextView(this);
        t.setText(text);t.setTextSize(size);t.setTextColor(color);
        if(bold)t.setTypeface(Typeface.DEFAULT,Typeface.BOLD);
        t.setPadding(0,dp(3),0,dp(9)); parent.addView(t);return t;
    }
    private void button(LinearLayout parent,String title,int color,Runnable cb){
        TextView b=new TextView(this);
        b.setText(title);b.setTextSize(14);b.setTextColor(Color.WHITE);
        b.setTypeface(Typeface.DEFAULT,Typeface.BOLD);b.setGravity(Gravity.CENTER);
        b.setPadding(dp(12),dp(13),dp(12),dp(13));
        GradientDrawable d=new GradientDrawable();d.setColor(color);d.setCornerRadius(dp(12));b.setBackground(d);
        LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(9);
        parent.addView(b,lp);b.setOnClickListener(v->cb.run());
    }
    private void space(int h){View v=new View(this);root.addView(v,new LinearLayout.LayoutParams(1,dp(h)));}
    private int dp(int d){return (int)(d*getResources().getDisplayMetrics().density+0.5f);}
}
