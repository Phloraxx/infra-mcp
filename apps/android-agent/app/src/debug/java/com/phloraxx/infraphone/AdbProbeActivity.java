package com.phloraxx.infraphone;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.provider.Settings;
import android.text.InputType;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.Switch;
import android.widget.TextView;
import android.widget.Toast;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;
import io.github.muntashirakon.adb.AdbStream;

/** Owner-operated, local-only ADB probe. Not connected to the signed MCP dispatcher. */
public final class AdbProbeActivity extends Activity {
    private final ExecutorService worker=Executors.newSingleThreadExecutor();
    private final AtomicBoolean busy=new AtomicBoolean();
    private volatile LocalAdbClient client;
    private LinearLayout root;
    private TextView status,connectionAction,wirelessLabel;
    private Switch wirelessSwitch;
    private EditText pairingPort,pairingCode,connectionPort;
    private boolean expanded,changingSwitch;

    @Override public void onCreate(Bundle state){
        super.onCreate(state);
        if(!"debug".equals(BuildConfig.BUILD_TYPE)||!getPackageName().endsWith(".dev")){
            finish();return;
        }
        render();
    }
    @Override protected void onResume(){
        super.onResume();
        refreshWireless();
    }

    private void render(){
        root=UiKit.screen(this);
        UiKit.header(root,"Android access",R.drawable.infra_ic_back,this::finish);
        UiKit.addText(root,"More control.\nOnly when you need it.",26,UiKit.INK,true);
        UiKit.gap(root,12);
        UiKit.addText(root,"Local ADB is experimental. It never grants MCP remote shell access.",
            14,UiKit.MUTED,false);
        UiKit.gap(root,30);
        UiKit.section(root,"LOCAL CONNECTION");
        LinearLayout card=UiKit.card(root);
        status=UiKit.addText(card,isConnected()?"● Connected as Android shell":"○ Not connected",
            17,isConnected()?UiKit.GOOD:UiKit.INK,true);
        UiKit.gap(card,10);
        UiKit.addText(card,"Use the existing on-device ADB listener at 127.0.0.1:5555.",
            13,UiKit.MUTED,false);
        UiKit.gap(card,22);
        connectionAction=UiKit.button(card,isConnected()?"Disconnect":"Connect locally",
            true,()->{
                if(isConnected())disconnect();
                else doWork(()->{
                    LocalAdbClient adb=getClient();
                    adb.connectLegacyLoopback();
                    return verify(adb);
                });
            });
        UiKit.gap(card,13);
        UiKit.addText(card,"Port 5555 is not encrypted and may be visible to other devices on your network. "+
            "Infra won't turn it on automatically.",12,UiKit.SUBTLE,false);

        UiKit.gap(root,18);
        UiKit.section(root,"ALTERNATIVE");
        UiKit.row(root,R.drawable.infra_ic_lock,
            "Secure wireless pairing",expanded?"Hide setup":"Use Android's paired debugging mode",
            ()->{expanded=!expanded;render();});
        if(expanded)showWirelessSetup();
        UiKit.gap(root,24);
        UiKit.note(root,"Access stays local to this experiment until you explicitly approve an advanced session.");
    }

    private void showWirelessSetup(){
        LinearLayout box=UiKit.card(root);
        wirelessLabel=UiKit.addText(box,"Wireless debugging",15,UiKit.INK,true);
        UiKit.gap(box,9);
        wirelessSwitch=new Switch(this);
        wirelessSwitch.setText("Allow wireless debugging");
        wirelessSwitch.setTextColor(UiKit.INK);wirelessSwitch.setTextSize(14);
        wirelessSwitch.setThumbTintList(android.content.res.ColorStateList.valueOf(UiKit.ACCENT));
        box.addView(wirelessSwitch);
        wirelessSwitch.setOnCheckedChangeListener((button,enabled)->{
            if(changingSwitch)return;
            if(!enabled){
                new AlertDialog.Builder(this)
                    .setTitle("Turn off wireless debugging?")
                    .setMessage("Active secure ADB connections may disconnect. Port 5555 is separate.")
                    .setNegativeButton("Cancel",(dialog,which)->refreshWireless())
                    .setPositiveButton("Turn off",(dialog,which)->setWireless(false))
                    .show();
            }else setWireless(true);
        });
        UiKit.gap(box,8);
        if(!WirelessDebuggingControl.hasPermission(this)){
            UiKit.addText(box,"One-time setup required from an already authorized ADB computer.",
                12,UiKit.MUTED,false);
            UiKit.gap(box,10);
            UiKit.quietAction(box,"Copy ADB permission command",()->{
                ((ClipboardManager)getSystemService(Context.CLIPBOARD_SERVICE))
                    .setPrimaryClip(ClipData.newPlainText("Infra Phone ADB permission",
                        WirelessDebuggingControl.grantCommand(this)));
                Toast.makeText(this,"Setup command copied",Toast.LENGTH_SHORT).show();
            });
        }
        UiKit.quietAction(box,"Open Android Developer options",()->
            startActivity(new Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)));
        UiKit.gap(box,14);
        UiKit.addText(box,"Pair once",16,UiKit.INK,true);
        UiKit.gap(box,8);
        UiKit.addText(box,
            "In Wireless debugging, choose Pair device with pairing code. Enter that port and code here.",
            13,UiKit.MUTED,false);
        UiKit.gap(box,13);
        pairingPort=UiKit.input(box,"Pairing port","");
        pairingPort.setInputType(InputType.TYPE_CLASS_NUMBER);
        pairingCode=UiKit.input(box,"Six-digit code","");
        pairingCode.setInputType(InputType.TYPE_CLASS_NUMBER|InputType.TYPE_NUMBER_VARIATION_PASSWORD);
        UiKit.button(box,"Pair this phone",false,()->{
            Integer port=parsePort(pairingPort);
            String secret=pairingCode.getText().toString().trim();
            pairingCode.setText("");
            if(port==null || !secret.matches("[0-9]{6}")){
                status.setText("Enter a valid pairing port and six-digit code.");return;
            }
            doWork(()->getClient().pairLocally(port,secret)
                ?"Paired. Enter the connection port below.":"Pairing wasn't completed.");
        });
        UiKit.gap(box,17);
        UiKit.addText(box,"Connect securely",16,UiKit.INK,true);
        UiKit.gap(box,8);
        connectionPort=UiKit.input(box,"Connection port","");
        connectionPort.setInputType(InputType.TYPE_CLASS_NUMBER);
        UiKit.button(box,"Verify secure ADB",false,()->{
            Integer port=parsePort(connectionPort);
            if(port==null){status.setText("Enter the separate Wireless debugging connection port.");return;}
            doWork(()->{
                LocalAdbClient prior=client;
                if(prior!=null && prior.isConnected()){
                    prior.close();
                    client=null;
                }
                LocalAdbClient adb=getClient();
                adb.connectLocally(port);
                return verify(adb);
            });
        });
        UiKit.gap(box,7);
        refreshWireless();
    }

    private static Integer parsePort(EditText input){
        try{
            int n=Integer.parseInt(input.getText().toString().trim());
            if(n>=1024&&n<=65535&&n!=5555)return n;
        }catch(NumberFormatException ignored){}
        return null;
    }

    private void setWireless(boolean enabled){
        try{
            WirelessDebuggingControl.setEnabled(this,enabled);
            status.setText(enabled?"Wireless debugging enabled. Pair through Android settings.":
                                   "Standard wireless debugging disabled.");
        }catch(Exception e){
            status.setText("Wireless debugging unavailable. Check permission and Wi-Fi.");
        }finally{refreshWireless();}
    }

    private void refreshWireless(){
        if(wirelessSwitch!=null){
            changingSwitch=true;
            wirelessSwitch.setChecked(WirelessDebuggingControl.isEnabled(this));
            changingSwitch=false;
        }
        if(wirelessLabel!=null)
            wirelessLabel.setText("Wireless debugging  •  "+
                (WirelessDebuggingControl.isEnabled(this)?"On":"Off"));
    }

    private boolean isConnected(){return client!=null && client.isConnected();}
    private synchronized LocalAdbClient getClient() throws Exception {
        if(client==null)client=new LocalAdbClient();
        return client;
    }
    private void disconnect(){
        if(busy.get())return;
        final LocalAdbClient old=client;
        client=null;
        status.setText("Disconnected");
        connectionAction.setText("Connect locally");
        if(old!=null)worker.execute(()->{
            try{old.close();}catch(Exception ignored){}
        });
    }
    private static String verify(LocalAdbClient adb) throws Exception {
        String id=readIdentity(adb);
        if(!id.contains("uid=2000(shell)"))
            throw new SecurityException("Not an authorized Android shell session");
        return "● Connected as Android shell";
    }
    private static String readIdentity(LocalAdbClient adb)throws Exception {
        try(AdbStream stream=adb.openStream("shell:id");
            InputStream in=stream.openInputStream()){
            ByteArrayOutputStream out=new ByteArrayOutputStream();
            byte[] b=new byte[512];
            int n;
            while(out.size()<2048){
                try {n=in.read(b);}
                catch(IOException e){
                    if(out.size()==0||!"Stream closed.".equals(e.getMessage()))throw e;
                    break;
                }
                if(n<0)break;
                out.write(b,0,Math.min(n,2048-out.size()));
                if(new String(out.toByteArray(),StandardCharsets.UTF_8).contains("\n"))break;
            }
            return new String(out.toByteArray(),StandardCharsets.UTF_8);
        }
    }
    private interface Task {String run()throws Exception;}
    private void doWork(Task task){
        if(!busy.compareAndSet(false,true))return;
        status.setText("Connecting…");
        worker.execute(()->{
            String text;
            try{text=task.run();}
            catch(Exception e){text="Connection unavailable. Verify authorization and debugging settings.";}
            final String outcome=text;
            busy.set(false);
            runOnUiThread(()->{
                if(isFinishing()||isDestroyed())return;
                status.setText(outcome);
                connectionAction.setText(isConnected()?"Disconnect":"Connect locally");
            });
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
