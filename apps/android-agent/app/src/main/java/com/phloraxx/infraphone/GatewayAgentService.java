package com.phloraxx.infraphone;

import android.app.*;
import android.content.*;
import android.content.pm.ServiceInfo;
import android.os.*;
import android.os.storage.StorageManager;
import android.os.StatFs;
import org.json.*;
import okhttp3.*;
import okio.ByteString;
import java.io.File;
import java.net.URI;
import java.util.UUID;
import java.util.concurrent.TimeUnit;

/** Outbound signed Infra v1 agent. Never opens an inbound listening port. */
public class GatewayAgentService extends Service {
    public static final String ACTION_CONNECT="com.phloraxx.infraphone.CONNECT";
    public static final String ACTION_STOP="com.phloraxx.infraphone.DISCONNECT";
    private static volatile boolean active=false;
    private static volatile String state="Disconnected";
    private static volatile String lastError="";
    private final Handler handler=new Handler(Looper.getMainLooper());
    private OkHttpClient client;
    private WebSocket socket;
    private int attempts=0;
    private String instance="";
    private String boot="";
    private JSONObject hello;
    private volatile boolean authenticated=false;
    private DeviceIdentity identity;
    private Runnable reconnectTask;
    private final java.util.concurrent.ThreadPoolExecutor work = new java.util.concurrent.ThreadPoolExecutor(
        2,2,0,TimeUnit.MILLISECONDS,new java.util.concurrent.ArrayBlockingQueue<>(8));
    public static boolean isActive(){return active;}
    public static String connectionState(){return state;}
    public static String error(){return lastError;}

    @Override public IBinder onBind(Intent intent){return null;}
    @Override public int onStartCommand(Intent intent,int flags,int startId){
        if(intent!=null && ACTION_STOP.equals(intent.getAction())){stopSelf();return START_NOT_STICKY;}
        if(active)return START_NOT_STICKY;
        identity=new DeviceIdentity(this);
        if(!identity.enrolled()){state="Not enrolled";stopSelf();return START_NOT_STICKY;}
        createChannel();
        PendingIntent stop=PendingIntent.getService(this,88,
            new Intent(this,GatewayAgentService.class).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder n=Build.VERSION.SDK_INT>=26?
            new Notification.Builder(this,"infra_gateway"):new Notification.Builder(this);
        n.setContentTitle("Infra Phone connected session")
            .setContentText("Signed Infra gateway agent — stop any time")
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setOngoing(true).addAction(new Notification.Action.Builder(
                android.R.drawable.ic_menu_close_clear_cancel,"Disconnect",stop).build());
        if(Build.VERSION.SDK_INT>=34)startForeground(8770,n.build(),ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        else startForeground(8770,n.build());
        active=true;lastError="";attempts=0;
        client=new OkHttpClient.Builder()
            .connectTimeout(10,TimeUnit.SECONDS)
            .readTimeout(0,TimeUnit.MILLISECONDS)
            .pingInterval(15,TimeUnit.SECONDS).build();
        connect();
        return START_NOT_STICKY;
    }
    private void createChannel(){
        if(Build.VERSION.SDK_INT>=26){
            NotificationChannel ch=new NotificationChannel("infra_gateway","Infra gateway agent",NotificationManager.IMPORTANCE_LOW);
            ((NotificationManager)getSystemService(NOTIFICATION_SERVICE)).createNotificationChannel(ch);
        }
    }
    private void connect(){
        if(!active)return;
        state="Connecting";authenticated=false;
        instance=UUID.randomUUID().toString();
        boot=identity.bootId();
        try {
            String base=GatewayEnrollment.normalize(identity.gateway());
            URI endpoint=new URI(base.replaceFirst("^http","ws")+"/agent");
            Request req=new Request.Builder().url(endpoint.toString()).build();
            socket=client.newWebSocket(req,new WebSocketListener(){
                @Override public void onOpen(WebSocket ws, Response response){
                    try {
                        JSONObject b=new JSONObject().put("protocol",1).put("device_id",identity.deviceId())
                            .put("agent_instance_id",instance);
                        send(ws,"auth_start",b);
                        state="Authenticating";
                    }catch(Exception e){fail(ws,e);}
                }
                @Override public void onMessage(WebSocket ws,String text){
                    try {
                        if(text.length()>1024*1024)throw new IllegalArgumentException("Control frame exceeds Android limit");
                        JSONObject message=new JSONObject(text);
                        String type=message.getString("type");
                        if("ping".equals(type)){
                            ws.send(new JSONObject().put("type","pong").toString());
                            if(authenticated)send(ws,"reconcile",new JSONObject().put("items",new JSONArray()));
                            return;
                        }
                        JSONObject body=message.optJSONObject("body");
                        if(body==null)throw new IllegalArgumentException("Missing message body");
                        if("auth_challenge".equals(type)){
                            JSONObject h=new JSONObject();
                            h.put("protocol",1).put("device_id",identity.deviceId())
                             .put("device_name",identity.name()).put("agent_version","0.2.0")
                             .put("platform","android").put("arch",Build.SUPPORTED_ABIS[0])
                             .put("boot_id",boot).put("agent_instance_id",instance)
                             .put("capabilities",new JSONArray().put("status").put("system").put("files"));
                            h.put("metadata",new JSONObject().put("android_api",String.valueOf(Build.VERSION.SDK_INT)));
                            hello=h;
                            String signature=identity.sign(body.getString("nonce"),body.getString("gateway_epoch"),
                                    1,identity.deviceId(),instance,boot);
                            send(ws,"signed_hello",new JSONObject().put("hello",h).put("signature",signature));
                        }else if("hello_ack".equals(type)){
                            if(body.getInt("protocol")!=1)throw new IllegalArgumentException("Wrong protocol");
                            authenticated=true;attempts=0;state="Connected";lastError="";
                            send(ws,"reconcile",new JSONObject().put("items",new JSONArray()));
                        }else if("request".equals(type)){
                            if(!authenticated)throw new IllegalStateException("Not authenticated");
                            dispatchRequest(ws,body);
                        }
                    }catch(Exception e){fail(ws,e);}
                }
                @Override public void onMessage(WebSocket ws,ByteString bytes){ws.close(1003,"Binary frames not supported");}
                @Override public void onClosing(WebSocket ws,int code,String reason){ws.close(code,null);}
                @Override public void onClosed(WebSocket ws,int code,String reason){retry("Closed ("+code+")");}
                @Override public void onFailure(WebSocket ws,Throwable err,Response response){
                    retry("Transport unavailable: "+err.getClass().getSimpleName());
                }
            });
        }catch(Exception e){retry(e.getClass().getSimpleName()+": "+e.getMessage());}
    }
    private void dispatchRequest(WebSocket ws,JSONObject request){
        try {
            work.execute(()->{
                try {
                    String id=request.getString("id");
                    JSONObject result;
                    try{result=reply(request).getJSONObject("result");}
                    catch(Exception e){
                        result=new JSONObject().put("status","failed")
                            .put("error","Android operation failed: "+e.getClass().getSimpleName()+": "+e.getMessage());
                    }
                    send(ws,"response",new JSONObject().put("id",id).put("result",result));
                }catch(Exception ignored){/* malformed requests never reveal local stack or secrets */}
            });
        }catch(java.util.concurrent.RejectedExecutionException busy){
            try{
                send(ws,"response",new JSONObject().put("id",request.getString("id"))
                    .put("result",new JSONObject().put("status","failed").put("error","Android agent busy")));
            }catch(Exception ignored){}
        }
    }
    private JSONObject reply(JSONObject request)throws Exception {
        String id=request.getString("id");
        JSONObject op=request.getJSONObject("operation");
        String name=op.getString("op");
        JSONObject outcome;
        if("status".equals(name)){
            outcome=new JSONObject().put("status","ok")
                .put("body",new JSONObject().put("protocol",1).put("pid",android.os.Process.myPid()));
        }else if("system".equals(name)){
            JSONObject systemAction=op.optJSONObject("action");
            if(systemAction==null)throw new IllegalArgumentException("Missing system action object");
            String action=systemAction.optString("action");
            if("summary".equals(action)){
                StatFs stat=new StatFs(getFilesDir().getAbsolutePath());
                JSONObject details=new JSONObject();
                details.put("hostname",Build.MODEL);
                details.put("os_name","Android");
                details.put("os_version",Build.VERSION.RELEASE);
                details.put("kernel_version",System.getProperty("os.version","unknown"));
                details.put("architecture",Build.SUPPORTED_ABIS[0]);
                details.put("uptime_seconds",SystemClock.elapsedRealtime()/1000);
                details.put("cpu_count",Runtime.getRuntime().availableProcessors());
                details.put("cpu_brand",Build.HARDWARE);
                details.put("total_memory",androidTotalMemory());
                details.put("used_memory",androidUsedMemory());
                details.put("total_swap",0).put("used_swap",0);
                details.put("load_one",0).put("load_five",0).put("load_fifteen",0);
                details.put("android_api",Build.VERSION.SDK_INT);
                details.put("workspace_available",stat.getAvailableBytes());
                outcome=new JSONObject().put("status","ok").put("body",details);
            }else outcome=unsupported("system."+action);
        }else if("files".equals(name)){
            JSONObject fileAction=op.optJSONObject("action");
            if(fileAction==null)throw new IllegalArgumentException("Missing files action object");
            JSONObject body=new AndroidFilesOps(this).handle(fileAction);
            outcome=body==null?unsupported("files."+fileAction.optString("action")):
                new JSONObject().put("status","ok").put("body",body);
        }else outcome=unsupported(name);
        return new JSONObject().put("id",id).put("result",outcome);
    }
    private long androidTotalMemory(){
        android.app.ActivityManager am=(android.app.ActivityManager)getSystemService(ACTIVITY_SERVICE);
        android.app.ActivityManager.MemoryInfo m=new android.app.ActivityManager.MemoryInfo();
        am.getMemoryInfo(m);return m.totalMem;
    }
    private long androidUsedMemory(){
        android.app.ActivityManager am=(android.app.ActivityManager)getSystemService(ACTIVITY_SERVICE);
        android.app.ActivityManager.MemoryInfo m=new android.app.ActivityManager.MemoryInfo();
        am.getMemoryInfo(m);return m.totalMem-m.availMem;
    }
    private JSONObject unsupported(String name)throws Exception {
        return new JSONObject().put("status","unsupported").put("operation",name);
    }
    private void send(WebSocket ws,String type,JSONObject body)throws Exception {
        if(!ws.send(new JSONObject().put("type",type).put("body",body).toString()))
            throw new IllegalStateException("WebSocket send failed");
    }
    private void fail(WebSocket ws,Exception e){
        lastError=e.getClass().getSimpleName()+": "+e.getMessage();
        ws.close(1008,"Protocol failure");
    }
    private synchronized void retry(String message){
        if(!active)return;
        authenticated=false;state="Reconnecting";
        lastError=message;attempts=Math.min(attempts+1,7);
        if(reconnectTask!=null)handler.removeCallbacks(reconnectTask);
        int max=Math.min(60000,1000*(1<<attempts));
        long delay=max/2 + new java.security.SecureRandom().nextInt(Math.max(1,max/2));
        reconnectTask=this::connect;handler.postDelayed(reconnectTask,delay);
    }
    @Override public void onDestroy(){
        active=false;state="Disconnected";authenticated=false;
        if(reconnectTask!=null)handler.removeCallbacks(reconnectTask);
        if(socket!=null){socket.cancel();socket=null;}
        work.shutdownNow();
        if(client!=null){client.dispatcher().executorService().shutdown();client.connectionPool().evictAll();client=null;}
        super.onDestroy();
    }
}
