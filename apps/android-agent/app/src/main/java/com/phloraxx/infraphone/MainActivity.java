package com.phloraxx.infraphone;

import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.drawable.GradientDrawable;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.view.View;
import android.text.InputType;
import java.net.URI;
import java.util.Locale;

/** One-purpose home: signed connection and clear device state. */
public class MainActivity extends Activity {
    private final Handler handler=new Handler(Looper.getMainLooper());
    private final Runnable poll=new Runnable() {
        @Override public void run(){refreshConnection();handler.postDelayed(this,2500);}
    };
    private LinearLayout root;
    private TextView stateTitle,stateDetail,connectionAction,pairStatus;
    private View stateDot;
    private EditText gatewayInput,pairingInput;
    private boolean pairing;
    private boolean enrolledScreen;

    @Override public void onCreate(Bundle state) {super.onCreate(state);render();}

    private void render(){
        root=UiKit.screen(this);
        stateTitle=null;stateDetail=null;connectionAction=null;
        UiKit.header(root,"infra.",R.drawable.infra_ic_settings,
            ()->startActivity(new Intent(this,SettingsActivity.class)));
        enrolledScreen=new DeviceIdentity(this).enrolled();
        if(enrolledScreen)showHome();
        else showPairing();
    }

    private void showHome(){
        UiKit.eyebrow(root,"THIS PHONE");
        UiKit.gap(root,12);
        String device=Build.MODEL==null?"Android device":Build.MODEL.trim();
        if(device.length()>0)device=device.substring(0,1).toUpperCase(Locale.ROOT)+device.substring(1);
        UiKit.addText(root,device,30,UiKit.INK,true);
        UiKit.gap(root,31);

        LinearLayout connection=UiKit.card(root);
        LinearLayout top=UiKit.horizontal(this);
        stateDot=new View(this);
        GradientDrawable dot=new GradientDrawable();
        dot.setShape(GradientDrawable.OVAL);dot.setColor(UiKit.MUTED);
        stateDot.setBackground(dot);
        LinearLayout.LayoutParams dotParams=new LinearLayout.LayoutParams(UiKit.dp(this,9),UiKit.dp(this,9));
        dotParams.rightMargin=UiKit.dp(this,12);top.addView(stateDot,dotParams);
        stateTitle=UiKit.text(this,"Offline",20,UiKit.INK,true);
        top.addView(stateTitle);connection.addView(top);
        UiKit.gap(connection,10);
        stateDetail=UiKit.addText(connection,"Your phone is not connected to Infra.",14,UiKit.MUTED,false);
        stateDetail.setLineSpacing(UiKit.dp(this,4),1f);
        UiKit.gap(connection,25);
        connectionAction=UiKit.button(connection,"Connect",true,this::toggleConnection);

        UiKit.gap(root,7);
        UiKit.addText(root,"SECURE CONNECTION",11,UiKit.SUBTLE,true).setLetterSpacing(.12f);
        UiKit.gap(root,9);
        UiKit.addText(root,host(new DeviceIdentity(this).gateway()),13,UiKit.MUTED,false);
        UiKit.gap(root,34);
        UiKit.section(root,"CONTROLS");
        if("debug".equals(BuildConfig.BUILD_TYPE)){
            UiKit.row(root,R.drawable.infra_ic_terminal,"Android access",
                "Optional local ADB and debugging",
                ()->startActivity(new Intent().setClassName(
                    this,"com.phloraxx.infraphone.AdbProbeActivity")));
        } else {
            UiKit.row(root,R.drawable.infra_ic_terminal,"Advanced tools",
                "Permissions and device controls",
                ()->startActivity(new Intent(this,SettingsActivity.class)));
        }
        UiKit.gap(root,16);
        UiKit.note(root,"Only your approved devices and actions can use this connection.");
        refreshConnection();
    }

    private void showPairing(){
        UiKit.eyebrow(root,"WELCOME");
        UiKit.gap(root,12);
        UiKit.addText(root,"Connect this phone.",31,UiKit.INK,true);
        UiKit.gap(root,12);
        TextView subtitle=UiKit.addText(root,
            "Pair once with your Infra gateway. Your phone keeps its own secure identity.",
            15,UiKit.MUTED,false);
        subtitle.setLineSpacing(UiKit.dp(this,5),1f);
        UiKit.gap(root,30);
        LinearLayout card=UiKit.card(root);
        UiKit.addText(card,"Gateway address",13,UiKit.MUTED,true);
        UiKit.gap(card,9);
        String original=new DeviceIdentity(this).gateway();
        gatewayInput=UiKit.input(card,"https://infra.example.com",
            original.isEmpty()?"https://infra.drbijoy.in":original);
        UiKit.gap(card,10);
        UiKit.addText(card,"Pairing code",13,UiKit.MUTED,true);
        UiKit.gap(card,9);
        pairingInput=UiKit.input(card,"One-time code","");
        pairingInput.setInputType(InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD);
        pairStatus=UiKit.addText(card,"",13,UiKit.MUTED,false);
        UiKit.gap(card,12);
        UiKit.button(card,"Pair and connect",true,this::pairDevice);
        UiKit.gap(root,18);
        UiKit.note(root,"Pairing requires a valid code from your own Infra gateway.");
    }

    private void pairDevice(){
        if(pairing)return;
        String url=gatewayInput.getText().toString().trim();
        String code=pairingInput.getText().toString().trim();
        if(url.isEmpty()||code.isEmpty()){
            pairStatus.setText("Enter a gateway address and pairing code.");return;
        }
        pairing=true;
        pairStatus.setText("Pairing securely…");pairingInput.setText("");
        new Thread(()->{
            try {
                GatewayEnrollment.pair(getApplicationContext(),url,code);
                runOnUiThread(()->{
                    pairing=false;
                    if(isFinishing()||isDestroyed())return;
                    render();startGateway();
                });
            }catch(Exception e){
                runOnUiThread(()->{
                    pairing=false;
                    if(pairStatus!=null)pairStatus.setText("Could not pair. Check your gateway and code.");
                });
            }
        },"infra-pair").start();
    }

    private void toggleConnection(){
        if(GatewayAgentService.isActive())stopService(new Intent(this,GatewayAgentService.class));
        else startGateway();
        handler.postDelayed(this::refreshConnection,350);
    }

    private void startGateway(){
        if(!new DeviceIdentity(this).enrolled())return;
        try {
            if(Build.VERSION.SDK_INT>=33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)
                requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},31);
            Intent intent=new Intent(this,GatewayAgentService.class).setAction(GatewayAgentService.ACTION_CONNECT);
            if(Build.VERSION.SDK_INT>=26)startForegroundService(intent);else startService(intent);
            handler.postDelayed(this::refreshConnection,800);
        }catch(RuntimeException e){
            if(stateDetail!=null)stateDetail.setText("Android prevented the connection. Check permissions.");
        }
    }

    private void refreshConnection(){
        if(stateTitle==null||!new DeviceIdentity(this).enrolled())return;
        boolean active=GatewayAgentService.isActive();
        String state=GatewayAgentService.connectionState();
        boolean connected=active&&"Connected".equalsIgnoreCase(state);
        stateTitle.setText(connected?"Connected":active?"Connecting…":"Not connected");
        stateDetail.setText(connected?"Ready to receive your approved requests.":
            active?"Establishing a secure connection to Infra.":
                "Your phone is offline to Infra. Connect when you need it.");
        connectionAction.setText(active?"Disconnect":"Connect to Infra");
        GradientDrawable dot=new GradientDrawable();
        dot.setShape(GradientDrawable.OVAL);
        dot.setColor(connected?UiKit.GOOD:active?UiKit.WARN:UiKit.SUBTLE);
        stateDot.setBackground(dot);
    }

    private static String host(String base){
        try {String host=new URI(base).getHost();return host==null?"Infra gateway":host;}
        catch(Exception ignored){return "Infra gateway";}
    }

    @Override protected void onResume(){
        super.onResume();
        if(root!=null && enrolledScreen!=new DeviceIdentity(this).enrolled())render();
        handler.removeCallbacks(poll);handler.post(poll);
    }
    @Override protected void onPause(){handler.removeCallbacks(poll);super.onPause();}
}
