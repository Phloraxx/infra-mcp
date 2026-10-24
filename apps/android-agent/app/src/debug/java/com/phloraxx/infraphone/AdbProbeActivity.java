package com.phloraxx.infraphone;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.graphics.Color;
import android.os.Bundle;
import android.provider.Settings;
import android.text.InputType;
import android.view.Gravity;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

import io.github.muntashirakon.adb.AdbStream;

/**
 * Local-only, owner-operated read-only pairing experiment.
 * No Android shell output or pairing keys ever go through Infra MCP.
 */
public final class AdbProbeActivity extends Activity {
    private final ExecutorService worker=Executors.newSingleThreadExecutor();
    private final AtomicBoolean busy=new AtomicBoolean();
    private volatile LocalAdbClient client;
    private TextView status;
    private TextView wirelessStatus;
    private EditText pairingPort, pairingCode, connectionPort;

    @Override public void onCreate(Bundle state){
        super.onCreate(state);
        if(!"debug".equals(BuildConfig.BUILD_TYPE) || !getPackageName().endsWith(".dev")){
            finish();return;
        }
        ScrollView scroll=new ScrollView(this);
        scroll.setBackgroundColor(Color.rgb(12,18,32));
        LinearLayout content=new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setPadding(32,28,32,36);
        scroll.addView(content);
        title(content,"Local ADB experiment",25);
        title(content,"Debug build only. No remote shell, background ADB, or automatic elevation. " +
            "Android Wireless debugging must be enabled and paired on this phone.",14);
        status=title(content,"Not paired. Normal Infra connectivity is unaffected.",14);
        wirelessStatus=title(content,WirelessDebuggingControl.describe(this),14);
        title(content,"One-time setup: grant permission from an authorized ADB computer. " +
            "These buttons control Android's standard Wireless debugging only, " +
            "not legacy TCP port 5555.",13);
        button(content,"Copy one-time grant command",()->{
            ClipboardManager clipboard=(ClipboardManager)getSystemService(Context.CLIPBOARD_SERVICE);
            clipboard.setPrimaryClip(ClipData.newPlainText("Infra Phone ADB setup",
                WirelessDebuggingControl.grantCommand(this)));
            status.setText("Copied. Run this command on your trusted ADB computer, then refresh.");
        });
        button(content,"Enable Wireless debugging",()->toggleWirelessDebugging(true));
        button(content,"Disable Wireless debugging",()->
            new AlertDialog.Builder(this)
                .setTitle("Disable Wireless debugging?")
                .setMessage("This can disconnect active ADB sessions and other tools. " +
                    "It does not close legacy TCP port 5555.")
                .setNegativeButton("Cancel",null)
                .setPositiveButton("Disable",(dialog,which)->toggleWirelessDebugging(false))
                .show());
        button(content,"Refresh Wireless debugging status",this::refreshWirelessStatus);
        button(content,"Open Android Developer options",()->
            startActivity(new Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)));
        pairingPort=field(content,"Pairing port (shown with six-digit code)",false);
        pairingCode=field(content,"Six-digit pairing code",true);
        button(content,"Pair locally",()->{
            final int port=parsePort(pairingPort);
            final String secret=pairingCode.getText().toString().trim();
            pairingCode.setText("");
            if(!secret.matches("[0-9]{6}")){
                status.setText("Enter the current six-digit code from Wireless debugging.");return;
            }
            doWork(()->{
                if(!getClient().pairLocally(port,secret))
                    throw new IllegalStateException("Pairing not completed");
                return "Local ADB pairing completed. Now enter the connection port.";
            });
        });
        connectionPort=field(content,"Wireless debugging connection port (not the pairing port)",false);
        button(content,"Connect and verify read-only shell identity",()->{
            final int port=parsePort(connectionPort);
            doWork(()->{
                LocalAdbClient adb=getClient();
                adb.connectLocally(port);
                String result=readIdentity(adb);
                if(!result.contains("uid=2000(shell)"))
                    throw new IllegalStateException("Connected endpoint did not return Android shell UID");
                return "Verified on-device ADB shell UID 2000. No MCP commands enabled.";
            });
        });
        button(content,"Disconnect ADB client",()->{
            LocalAdbClient old=client;
            client=null;
            if(old!=null)new Thread(()->{
                try{old.close();}catch(Exception ignored){}
            },"infra-adb-close").start();
            status.setText("ADB client disconnected. Android pairing can remain until revoked in Developer options.");
        });
        title(content,"Use Android's Wireless debugging pairing and connection ports; " +
            "legacy network TCP port 5555 is deliberately refused. " +
            "If this fails, regular Infra and SenseGuide features still work.",13);
        setContentView(scroll);
    }

    @Override protected void onResume() {
        super.onResume();
        refreshWirelessStatus();
    }

    private void refreshWirelessStatus() {
        if(wirelessStatus!=null)
            wirelessStatus.setText(WirelessDebuggingControl.describe(this));
    }

    private void toggleWirelessDebugging(boolean enabled) {
        try {
            WirelessDebuggingControl.setEnabled(this,enabled);
            status.setText("Wireless debugging setting is now "+
                (enabled?"on. Android may still require a trusted Wi-Fi network.":
                         "off. Legacy TCP ADB (if active) is unaffected by this setting."));
        }catch(Exception e) {
            status.setText("Wireless debugging change denied: "+e.getMessage());
        }finally{refreshWirelessStatus();}
    }

    private synchronized LocalAdbClient getClient()throws Exception {
        if(client==null)client=new LocalAdbClient();
        return client;
    }

    private static String readIdentity(LocalAdbClient adb)throws Exception {
        try(AdbStream stream=adb.openStream("shell:id");
            InputStream in=stream.openInputStream()) {
            ByteArrayOutputStream out=new ByteArrayOutputStream();
            byte[] b=new byte[512];
            int n;
            while(out.size()<2048) {
                try {
                    n=in.read(b);
                } catch(IOException e) {
                    // This ADB library throws on remote CLSE rather than returning EOF.
                    // Accept the close only after receiving some command output.
                    if(out.size()==0 || !"Stream closed.".equals(e.getMessage()))throw e;
                    break;
                }
                if(n<0)break;
                out.write(b,0,Math.min(n,2048-out.size()));
                // "id" emits a single newline-terminated line; do not wait for a
                // second read after it closes its ADB stream.
                if(new String(out.toByteArray(),StandardCharsets.UTF_8).contains("\n"))
                    break;
            }
            return new String(out.toByteArray(),StandardCharsets.UTF_8);
        }
    }

    private interface Operation { String run()throws Exception; }

    private void doWork(Operation task) {
        if(!busy.compareAndSet(false,true)){
            status.setText("An ADB operation is already running.");return;
        }
        status.setText("Working locally…");
        worker.execute(()->{
            String result;
            try{result=task.run();}
            catch(Exception e){result="ADB unavailable: "+e.getClass().getSimpleName()+
                ". Check Wireless debugging, the correct port, and pairing approval.";}
            final String message=result;
            busy.set(false);
            runOnUiThread(()->{if(!isFinishing())status.setText(message);});
        });
    }

    private static int parsePort(EditText field) {
        try {
            int port=Integer.parseInt(field.getText().toString().trim());
            if(port>=1024 && port<=65535 && port!=5555)return port;
        }catch(NumberFormatException ignored){}
        throw new IllegalArgumentException("Use the port currently shown in Android Wireless debugging (not 5555)");
    }

    private TextView title(LinearLayout parent,String s,int size){
        TextView v=new TextView(this);
        v.setText(s);v.setTextSize(size);v.setTextColor(Color.WHITE);
        v.setPadding(0,10,0,16);parent.addView(v);return v;
    }
    private EditText field(LinearLayout parent,String hint,boolean hidden){
        EditText e=new EditText(this);e.setHint(hint);
        e.setTextColor(Color.WHITE);e.setHintTextColor(0xff9eacc7);
        e.setInputType(InputType.TYPE_CLASS_NUMBER|
            (hidden?InputType.TYPE_NUMBER_VARIATION_PASSWORD:0));
        e.setSingleLine(true);parent.addView(e);return e;
    }
    private void button(LinearLayout parent,String label,Runnable action){
        Button b=new Button(this);b.setText(label);b.setAllCaps(false);
        parent.addView(b);b.setOnClickListener(v->{
            try{action.run();}catch(Exception e){status.setText(e.getMessage());}
        });
    }

    @Override protected void onDestroy(){
        worker.shutdownNow();
        LocalAdbClient old=client;client=null;
        if(old!=null)new Thread(()->{
            try{old.close();}catch(Exception ignored){}
        },"infra-adb-close").start();
        super.onDestroy();
    }
}
