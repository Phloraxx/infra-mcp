package com.phloraxx.infraphone;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.GestureDescription;
import android.graphics.Bitmap;
import android.graphics.Path;
import android.os.Handler;
import android.os.Looper;
import android.view.Display;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;
import android.hardware.HardwareBuffer;
import android.util.Base64;
import org.json.*;
import java.io.ByteArrayOutputStream;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/**
 * Explicit Android Settings grant + foreground SenseGuide window + locally timed one-hour lease.
 * Never interacts with installer/settings/other apps and never reads clipboard or types arbitrary text.
 */
public final class SenseGuideAccessibility extends AccessibilityService {
    private static volatile SenseGuideAccessibility instance;
    private final Handler ui=new Handler(Looper.getMainLooper());

    @Override public void onAccessibilityEvent(AccessibilityEvent e){}
    @Override public void onInterrupt(){}
    @Override protected void onServiceConnected(){super.onServiceConnected();instance=this;}
    @Override public void onDestroy(){if(instance==this)instance=null;super.onDestroy();}
    private boolean authorized() {
        if(!ScreenConsent.enabled(this))return false;
        AccessibilityNodeInfo n=getRootInActiveWindow();
        if(n==null || n.getPackageName()==null)return false;
        String pkg=n.getPackageName().toString();
        return SenseGuideDevelopment.LAB.equals(pkg) || SenseGuideDevelopment.DEBUG.equals(pkg);
    }
    public static JSONObject perform(android.content.Context ctx, JSONObject op)throws Exception {
        String kind=op.optString("action","");
        if("permissions".equals(kind))return new JSONObject()
            .put("screen_control_consent",ScreenConsent.enabled(ctx))
            .put("accessibility_enabled",instance!=null)
            .put("foreground_senseguide",instance!=null && instance.authorized())
            .put("requires_owner_activation",true);
        SenseGuideAccessibility svc=instance;
        if(svc==null||!svc.authorized())throw new SecurityException(
            "Enable Accessibility manually, open SenseGuide, and grant 1-hour consent in Infra Phone");
        if("screenshot".equals(kind))return svc.screenshot(op);
        if("click".equals(kind))return svc.click(op);
        if("key".equals(kind) && "back".equalsIgnoreCase(op.optString("key"))
            && op.optJSONArray("modifiers")==null)return svc.back();
        throw new UnsupportedOperationException("Only screenshot, click and key=back are allowed within SenseGuide");
    }
    private JSONObject back()throws Exception {
        if(!authorized())throw new SecurityException("SenseGuide not foreground");
        boolean ok=performGlobalAction(GLOBAL_ACTION_BACK);
        return new JSONObject().put("ok",ok);
    }
    private JSONObject click(JSONObject op)throws Exception {
        if(!authorized())throw new SecurityException("SenseGuide not foreground");
        String button=op.optString("button","left");
        if(!("left".equals(button)||"".equals(button)))throw new SecurityException("Only touch tap supported");
        int x=op.getInt("x"),y=op.getInt("y");
        android.util.DisplayMetrics metrics=new android.util.DisplayMetrics();
        getSystemService(android.view.WindowManager.class).getDefaultDisplay().getRealMetrics(metrics);
        if(x<0||y<0||x>=metrics.widthPixels||y>=metrics.heightPixels)
            throw new IllegalArgumentException("Tap outside screen");
        CountDownLatch done=new CountDownLatch(1);
        boolean[] ok={false};
        Path p=new Path();p.moveTo(x,y);
        GestureDescription.StrokeDescription s=new GestureDescription.StrokeDescription(p,0,75);
        GestureDescription g=new GestureDescription.Builder().addStroke(s).build();
        ui.post(()->{
            if(!authorized()){done.countDown();return;}
            dispatchGesture(g,new GestureResultCallback(){
                @Override public void onCompleted(GestureDescription gesture){ok[0]=true;done.countDown();}
                @Override public void onCancelled(GestureDescription gesture){done.countDown();}
            },ui);
        });
        if(!done.await(4,TimeUnit.SECONDS))throw new IllegalStateException("Gesture timeout");
        return new JSONObject().put("ok",ok[0]);
    }
    private JSONObject screenshot(JSONObject op)throws Exception {
        if(android.os.Build.VERSION.SDK_INT<30)throw new UnsupportedOperationException("Android 11 required");
        if(!authorized())throw new SecurityException("SenseGuide not foreground");
        if(op.has("display") && op.optInt("display",0)!=0)
            throw new IllegalArgumentException("Only primary display allowed");
        CountDownLatch done=new CountDownLatch(1);
        final byte[][] bytes={null};
        final int[] size=new int[2];
        ui.post(()->{
            if(!authorized()){done.countDown();return;}
            takeScreenshot(Display.DEFAULT_DISPLAY,getMainExecutor(),new TakeScreenshotCallback(){
                @Override public void onSuccess(ScreenshotResult result){
                    try {
                        HardwareBuffer buffer=result.getHardwareBuffer();
                        Bitmap wrapped=Bitmap.wrapHardwareBuffer(buffer,result.getColorSpace());
                        if(wrapped==null)throw new IllegalStateException("Screenshot bitmap unavailable");
                        int w=wrapped.getWidth(),h=wrapped.getHeight();
                        // Downscale to reduce disclosure/bandwidth and stay below MCP control limit.
                        double scale=Math.min(1.0,960.0/Math.max(w,h));
                        Bitmap smaller=Bitmap.createScaledBitmap(wrapped,
                            Math.max(1,(int)(w*scale)),Math.max(1,(int)(h*scale)),true);
                        ByteArrayOutputStream out=new ByteArrayOutputStream();
                        smaller.compress(Bitmap.CompressFormat.JPEG,65,out);
                        if(smaller!=wrapped)smaller.recycle();
                        buffer.close();
                        bytes[0]=out.toByteArray();
                        size[0]=(int)(w*scale);size[1]=(int)(h*scale);
                    }catch(Exception ignored){}finally{done.countDown();}
                }
                @Override public void onFailure(int errorCode){done.countDown();}
            });
        });
        if(!done.await(6,TimeUnit.SECONDS)||bytes[0]==null)throw new IllegalStateException("Screenshot unavailable");
        if(bytes[0].length>1024*1024)throw new IllegalStateException("Screenshot exceeds cap");
        return new JSONObject().put("mime_type","image/jpeg").put("width",size[0])
            .put("height",size[1]).put("image_base64",Base64.encodeToString(bytes[0],Base64.NO_WRAP));
    }
}
