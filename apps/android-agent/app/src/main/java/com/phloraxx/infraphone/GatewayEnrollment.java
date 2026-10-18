package com.phloraxx.infraphone;

import android.content.Context;
import org.json.JSONObject;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;

public final class GatewayEnrollment {
    private GatewayEnrollment(){}
    public static String normalize(String input) throws Exception {
        String url=input.trim();
        URI uri=new URI(url);
        if(uri.getUserInfo()!=null||uri.getFragment()!=null||uri.getQuery()!=null||
            uri.getHost()==null||!"/".equals(uri.getPath())&&!uri.getPath().isEmpty()||
            uri.getPort()>65535)throw new IllegalArgumentException("Enter a gateway URL without path or query");
        boolean secure="https".equalsIgnoreCase(uri.getScheme());
        boolean dev="http".equalsIgnoreCase(uri.getScheme()) && BuildConfig.DEBUG
            && privateAddress(uri.getHost());
        if(!secure&&!dev)throw new IllegalArgumentException("Gateway must use HTTPS (debug-only tailnet HTTP allowed)");
        return uri.toString().replaceAll("/$","");
    }
    private static boolean privateAddress(String host){
        if("localhost".equalsIgnoreCase(host)||"127.0.0.1".equals(host))return true;
        String[] p=host.split("\\.");
        if(p.length!=4)return false;
        try {
            int a=Integer.parseInt(p[0]),b=Integer.parseInt(p[1]);
            for(String v:p)if(Integer.parseInt(v)<0||Integer.parseInt(v)>255)return false;
            return a==100&&b>=64&&b<=127;
        }catch(Exception e){return false;}
    }
    public static void pair(Context app,String rawUrl,String code)throws Exception {
        String base=normalize(rawUrl);
        if(code.trim().isEmpty())throw new IllegalArgumentException("Enter a one-time enrollment code");
        DeviceIdentity identity=new DeviceIdentity(app);
        if(identity.enrolled())throw new IllegalStateException("Already enrolled; unpair first");
        String request=identity.ensurePending();
        JSONObject data=new JSONObject();
        data.put("code",code.trim());
        data.put("request_id",request);
        data.put("device_name",android.os.Build.MANUFACTURER+" "+android.os.Build.MODEL);
        data.put("public_key",identity.publicKey());
        data.put("platform","android");
        data.put("arch",android.os.Build.SUPPORTED_ABIS[0]);
        data.put("agent_version","0.2.0");
        HttpURLConnection c=(HttpURLConnection)new URL(base+"/enroll").openConnection();
        c.setRequestMethod("POST");c.setConnectTimeout(8000);c.setReadTimeout(10000);
        c.setRequestProperty("Content-Type","application/json");
        c.setDoOutput(true);
        try {
            try(OutputStream os=c.getOutputStream()){os.write(data.toString().getBytes(StandardCharsets.UTF_8));}
            int status=c.getResponseCode();
            InputStream in=status>=400?c.getErrorStream():c.getInputStream();
            ByteArrayOutputStream b=new ByteArrayOutputStream();
            if(in!=null)try(InputStream stream=in){byte[] bytes=new byte[2048];int n;while((n=stream.read(bytes))!=-1 && b.size()<16384)b.write(bytes,0,n);}
            if(status!=200)throw new IOException("Gateway enrollment rejected (HTTP "+status+")");
            JSONObject result=new JSONObject(b.toString("UTF-8"));
            String id=result.getString("device_id");
            if(id.isEmpty())throw new IOException("Gateway did not return a device ID");
            identity.configureGateway(base);
            identity.enrolledAs(id,result.optString("device_name","Android phone"));
        }finally{c.disconnect();}
    }
}
