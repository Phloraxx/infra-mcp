package com.phloraxx.infraphone;

import android.app.*;
import android.content.*;
import android.content.pm.ServiceInfo;
import android.net.Uri;
import android.os.*;
import android.provider.DocumentsContract;
import android.database.Cursor;
import android.graphics.Color;
import android.content.Intent;
import java.io.*;
import java.net.*;
import java.nio.file.*;
import java.security.*;
import java.util.*;
import org.json.*;
import fi.iki.elonen.NanoHTTPD;

/** Owner-enabled, short-lived development and diagnostic server. No root access. */
public class AgentService extends Service {
    private static volatile String key, bindings = "";
    private final List<AgentServer> servers = new ArrayList<>();
    private static final int PORT = 8765;

    public static boolean isRunning(){return key!=null;}
    public static String getToken(){return key;}
    public static String getBindings(){return bindings;}

    @Override public IBinder onBind(Intent intent){return null;}
    @Override public int onStartCommand(Intent i,int flags,int startId){
        if(i!=null && "STOP".equals(i.getAction())){
            stopSelf();return START_NOT_STICKY;
        }
        if(key!=null)return START_NOT_STICKY;
        try {
            createNotificationChannel();
            Notification.Builder n=(Build.VERSION.SDK_INT>=26)
                ?new Notification.Builder(this,"infra_phone")
                :new Notification.Builder(this);
            Intent stop=new Intent(this,AgentService.class).setAction("STOP");
            PendingIntent action=PendingIntent.getService(this,100,stop,PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
            n.setContentTitle("Infra Phone is running")
              .setContentText("Local/Tailscale development API — tap Stop to disconnect")
              .setSmallIcon(android.R.drawable.stat_sys_upload)
              .setOngoing(true)
              .addAction(new Notification.Action.Builder(
                   android.R.drawable.ic_menu_close_clear_cancel,"Stop",action).build());
            if(Build.VERSION.SDK_INT>=34)startForeground(8765,n.build(),ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            else startForeground(8765,n.build());
            File web=new File(getFilesDir(),"workspace/site");
            if(!web.exists())web.mkdirs();
            File home=new File(web,"index.html");
            if(!home.exists()){
                try(FileOutputStream f=new FileOutputStream(home)){
                    f.write(("<!doctype html><html><meta name='viewport' content='width=device-width,initial-scale=1'>" +
                        "<title>Infra Phone</title><body style='background:#0e1729;color:#e8efff;font-family:sans-serif;padding:10vw'>"+
                        "<h1>Infra Phone</h1><p>Your phone can host files privately over Tailscale or localhost.</p>"+
                        "<p>Upload a new site/index.html to replace this page.</p></body></html>").getBytes("UTF-8"));
                }
            }
            byte[] random=new byte[32];new SecureRandom().nextBytes(random);
            StringBuilder sb=new StringBuilder();
            for(byte b:random)sb.append(String.format(Locale.ROOT,"%02x",b&255));
            key=sb.toString();
            AgentServer local=new AgentServer(this,"127.0.0.1");
            local.start(5000,false);
            servers.add(local);
            StringBuilder names=new StringBuilder("127.0.0.1:"+PORT);
            for(InetAddress address:tailscaleAddresses()){
                try {
                    AgentServer vpn=new AgentServer(this,address.getHostAddress());
                    vpn.start(5000,false);servers.add(vpn);
                    names.append(", ").append(address.getHostAddress()).append(":").append(PORT);
                }catch(Exception ignored){}
            }
            bindings=names.toString();
        }catch(Exception e){key=null;bindings="";closeAll();stopSelf();}
        return START_NOT_STICKY;
    }
    private List<InetAddress> tailscaleAddresses(){
        List<InetAddress> result=new ArrayList<>();
        try {
            Enumeration<NetworkInterface> interfaces=NetworkInterface.getNetworkInterfaces();
            while(interfaces.hasMoreElements()){
                NetworkInterface ni=interfaces.nextElement();
                if(!ni.isUp() || !ni.getName().startsWith("tun"))continue;
                Enumeration<InetAddress> addresses=ni.getInetAddresses();
                while(addresses.hasMoreElements()){
                    InetAddress a=addresses.nextElement();
                    if(a instanceof Inet4Address && isTailscaleIp(a.getAddress()))result.add(a);
                }
            }
        }catch(Exception ignored){}
        return result;
    }
    private boolean isTailscaleIp(byte[] ip){
        return (ip[0]&255)==100 && ((ip[1]&255)>=64) && ((ip[1]&255)<=127);
    }
    private void createNotificationChannel(){
        if(Build.VERSION.SDK_INT>=26){
            NotificationChannel c=new NotificationChannel("infra_phone","Infra Phone session",NotificationManager.IMPORTANCE_LOW);
            ((NotificationManager)getSystemService(NOTIFICATION_SERVICE)).createNotificationChannel(c);
        }
    }
    private void closeAll(){for(AgentServer s:servers)try{s.stop();}catch(Exception ignored){}servers.clear();}
    @Override public void onDestroy(){closeAll();key=null;bindings="";super.onDestroy();}

    private static class AgentServer extends NanoHTTPD {
        private final AgentService parent;
        AgentServer(AgentService parent,String host){super(host,PORT);this.parent=parent;}
        private Response response(Response.Status status,String mime,String text){
            return NanoHTTPD.newFixedLengthResponse(status,mime,text);
        }
        private Response json(int code,String data){
            Response.Status status=code==200?Response.Status.OK:
                code==401?Response.Status.UNAUTHORIZED:
                code==403?Response.Status.FORBIDDEN:
                code==404?Response.Status.NOT_FOUND:
                code==405?Response.Status.METHOD_NOT_ALLOWED:
                Response.Status.BAD_REQUEST;
            return response(status,"application/json; charset=utf-8",data);
        }
        private Response err(int code,String msg){
            JSONObject o=new JSONObject();try{o.put("error",msg);}catch(Exception ignored){}
            return json(code,o.toString());
        }
        private boolean authed(IHTTPSession s){
            String auth=s.getHeaders().get("authorization");
            String current=key;
            if(current==null || auth==null || !auth.startsWith("Bearer "))return false;
            return MessageDigest.isEqual(current.getBytes(java.nio.charset.StandardCharsets.US_ASCII),
                auth.substring(7).getBytes(java.nio.charset.StandardCharsets.US_ASCII));
        }
        @Override public Response serve(IHTTPSession s){
            try{
                String uri=s.getUri();
                // The site is intentionally reachable only on loopback or Tailscale.
                if(uri.equals("/site")||uri.startsWith("/site/")){
                    if(s.getMethod()!=Method.GET)return err(405,"GET only");
                    String suffix=uri.equals("/site")? "site/index.html" :
                        (uri.equals("/site/")?"site/index.html":"site/"+uri.substring(6));
                    return sendFile(pathWithinWorkspace(suffix));
                }
                if(!uri.startsWith("/api/"))return err(404,"Not found");
                if(!authed(s))return err(401,"Session key required");
                Map<String,String> p=s.getParms();
                Method method=s.getMethod();
                if(uri.equals("/api/status")&&method==Method.GET){
                    JSONObject o=new JSONObject();
                    o.put("device",Build.MANUFACTURER+" "+Build.MODEL);
                    o.put("android",Build.VERSION.RELEASE);
                    o.put("sdk",Build.VERSION.SDK_INT);
                    o.put("batteryPercent",batteryLevel());
                    o.put("freeWorkspaceBytes",parent.getFilesDir().getUsableSpace());
                    o.put("workspace","app-private workspace");
                    o.put("sharedFolderSelected",sharedTree()!=null);
                    o.put("interfaces",bindings);
                    o.put("session","manual, not persistent after stop or reboot");
                    return json(200,o.toString());
                }
                if(uri.equals("/api/files")&&method==Method.GET){
                    File folder=pathWithinWorkspace(p.getOrDefault("path",""));
                    if(!folder.isDirectory())return err(404,"Directory not found");
                    JSONArray items=new JSONArray();
                    File[] children=folder.listFiles();
                    if(children!=null){
                        Arrays.sort(children,Comparator.comparing(File::getName));
                        for(int z=0;z<Math.min(children.length,500);z++){
                            File f=children[z];
                            items.put(new JSONObject().put("name",f.getName()).put("directory",f.isDirectory()).put("bytes",f.length()));
                        }
                    }
                    return json(200,new JSONObject().put("entries",items).toString());
                }
                if(uri.equals("/api/file")&&method==Method.GET){
                    return sendFile(pathWithinWorkspace(p.getOrDefault("path","")));
                }
                if(uri.equals("/api/file")&&method==Method.PUT){
                    String name=p.get("path");
                    if(name==null||name.isEmpty())return err(400,"Missing path");
                    File dest=pathWithinWorkspace(name);
                    if(dest.isDirectory())return err(403,"Not a file");
                    long declared=Long.parseLong(s.getHeaders().getOrDefault("content-length","0"));
                    if(declared<0||declared>10*1024*1024)return err(400,"Maximum upload 10 MiB");
                    Map<String,String> uploads=new HashMap<>();
                    s.parseBody(uploads);
                    String temp=uploads.get("content");
                    if(temp==null)return err(400,"Missing upload body");
                    File f=new File(temp);
                    if(f.length()>10*1024*1024)return err(400,"Maximum upload 10 MiB");
                    dest.getParentFile().mkdirs();
                    try(InputStream in=new FileInputStream(f);OutputStream out=new FileOutputStream(dest)){
                        byte[] buf=new byte[8192];int n;while((n=in.read(buf))!=-1)out.write(buf,0,n);
                    }
                    return json(200,"{\"saved\":true}");
                }
                if(uri.equals("/api/shared")&&method==Method.GET){
                    Uri tree=sharedTree();
                    if(tree==null)return err(403,"Choose a shared folder in the app first");
                    String parentId=DocumentsContract.getTreeDocumentId(tree);
                    Uri childrenUri=DocumentsContract.buildChildDocumentsUriUsingTree(tree,parentId);
                    JSONArray items=new JSONArray();
                    try(Cursor c=parent.getContentResolver().query(childrenUri,
                        new String[]{DocumentsContract.Document.COLUMN_DISPLAY_NAME,DocumentsContract.Document.COLUMN_MIME_TYPE,
                         DocumentsContract.Document.COLUMN_SIZE},null,null,null)){
                        if(c!=null)while(c.moveToNext() && items.length()<500){
                            items.put(new JSONObject().put("name",c.getString(0))
                                .put("mime",c.getString(1)).put("bytes",c.isNull(2)?JSONObject.NULL:c.getLong(2)));
                        }
                    }
                    return json(200,new JSONObject().put("entries",items).toString());
                }
                if(uri.equals("/api/shared/file")&&method==Method.GET){
                    Uri tree=sharedTree();
                    if(tree==null)return err(403,"Choose a shared folder first");
                    String name=p.get("name");
                    if(name==null||name.length()==0||name.length()>256)return err(400,"Missing filename");
                    String parentId=DocumentsContract.getTreeDocumentId(tree);
                    Uri list=DocumentsContract.buildChildDocumentsUriUsingTree(tree,parentId);
                    String selectedId=null;long declaredSize=-1;
                    try(Cursor c=parent.getContentResolver().query(list,
                        new String[]{DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                          DocumentsContract.Document.COLUMN_DISPLAY_NAME,DocumentsContract.Document.COLUMN_MIME_TYPE,
                          DocumentsContract.Document.COLUMN_SIZE},null,null,null)){
                        if(c!=null)while(c.moveToNext()){
                            if(name.equals(c.getString(1)) && !DocumentsContract.Document.MIME_TYPE_DIR.equals(c.getString(2))){
                                selectedId=c.getString(0);
                                declaredSize=c.isNull(3)?-1:c.getLong(3);break;
                            }
                        }
                    }
                    if(selectedId==null)return err(404,"File not found in selected folder");
                    if(declaredSize>10*1024*1024)return err(400,"Maximum download 10 MiB");
                    Uri doc=DocumentsContract.buildDocumentUriUsingTree(tree,selectedId);
                    ByteArrayOutputStream bytes=new ByteArrayOutputStream();
                    try(InputStream in=parent.getContentResolver().openInputStream(doc)){
                        if(in==null)return err(404,"Unable to read selected document");
                        byte[] buf=new byte[8192];int count,total=0;
                        while((count=in.read(buf))!=-1){
                            total+=count;if(total>10*1024*1024)return err(400,"Maximum download 10 MiB");
                            bytes.write(buf,0,count);
                        }
                    }
                    byte[] payload=bytes.toByteArray();
                    return NanoHTTPD.newFixedLengthResponse(Response.Status.OK,"application/octet-stream",
                        new ByteArrayInputStream(payload),payload.length);
                }
                return err(404,"Unknown endpoint");
            }catch(SecurityException e){return err(403,"Permission denied");
            }catch(Exception e){return err(400,"Request failed: "+e.getClass().getSimpleName());}
        }
        private Uri sharedTree(){
            String u=parent.getSharedPreferences("owner",MODE_PRIVATE).getString("folder",null);
            return u==null?null:Uri.parse(u);
        }
        private int batteryLevel(){
            Intent battery=parent.registerReceiver(null,new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            if(battery==null)return -1;
            int level=battery.getIntExtra(BatteryManager.EXTRA_LEVEL,-1);
            int scale=battery.getIntExtra(BatteryManager.EXTRA_SCALE,-1);
            return scale>0?level*100/scale:-1;
        }
        private File pathWithinWorkspace(String relative) throws IOException {
            File base=new File(parent.getFilesDir(),"workspace").getCanonicalFile();
            File file=new File(base,relative).getCanonicalFile();
            if(!file.equals(base)&&!file.getPath().startsWith(base.getPath()+File.separator))
                throw new SecurityException("Path outside workspace");
            return file;
        }
        private Response sendFile(File file)throws IOException {
            if(!file.exists()||!file.isFile())return err(404,"File not found");
            String n=file.getName().toLowerCase(Locale.ROOT),mime="application/octet-stream";
            if(n.endsWith(".html"))mime="text/html; charset=utf-8";
            else if(n.endsWith(".css"))mime="text/css; charset=utf-8";
            else if(n.endsWith(".js"))mime="text/javascript; charset=utf-8";
            else if(n.endsWith(".json"))mime="application/json; charset=utf-8";
            else if(n.endsWith(".txt")||n.endsWith(".md"))mime="text/plain; charset=utf-8";
            else if(n.endsWith(".png"))mime="image/png";
            else if(n.endsWith(".jpg")||n.endsWith(".jpeg"))mime="image/jpeg";
            else if(n.endsWith(".svg"))mime="image/svg+xml";
            return NanoHTTPD.newFixedLengthResponse(Response.Status.OK,mime,new FileInputStream(file),file.length());
        }
    }
}
