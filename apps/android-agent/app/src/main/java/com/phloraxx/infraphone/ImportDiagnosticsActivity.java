package com.phloraxx.infraphone;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.Locale;

/** User-approved one-time import of diagnostic JSONL from the SenseGuide debug provider. */
public final class ImportDiagnosticsActivity extends Activity {
    public static final String ACTION_IMPORT="io.github.phloraxx.infraphone.IMPORT_DIAGNOSTICS";
    private static final int LIMIT_BYTES=65536;
    private Uri reportUri;
    private LinearLayout view;
    private TextView message;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        Intent intent=getIntent();
        reportUri=intent.getData();
        view=new LinearLayout(this);
        view.setOrientation(LinearLayout.VERTICAL);
        int pad=(int)(22*getResources().getDisplayMetrics().density);
        view.setPadding(pad,pad,pad,pad);
        view.setBackgroundColor(Color.rgb(16,24,40));
        TextView title=label("SenseGuide diagnostic import",23);
        view.addView(title);
        message=label("Approve a one-time import of the metadata-only lab report into Infra Phone's private workspace. You can inspect it remotely through signed Infra-MCP. No ongoing access is granted.",15);
        view.addView(message);
        Button importButton=new Button(this);
        importButton.setText("Import report");
        importButton.setOnClickListener(v->{
            importButton.setEnabled(false);
            try {
                String path=importReport();
                message.setText("Imported. Infra-MCP file path:\n"+path+"\n\nNo background sharing is enabled.");
            }catch(Exception e){
                message.setText("Import refused: "+e.getClass().getSimpleName()+": "+e.getMessage());
            }
        });
        view.addView(importButton);
        Button dismiss=new Button(this);
        dismiss.setText("Close");
        dismiss.setOnClickListener(v->finish());
        view.addView(dismiss);
        setContentView(view);
        if(intent==null || !ACTION_IMPORT.equals(intent.getAction())
                || (intent.getFlags()&Intent.FLAG_GRANT_READ_URI_PERMISSION)==0
                || !validSource(reportUri)) {
            importButton.setEnabled(false);
            message.setText("No authorized SenseGuide diagnostic report was provided.");
        }
    }

    private TextView label(String text,int size){
        TextView t=new TextView(this);t.setText(text);t.setTextSize(size);
        t.setTextColor(Color.WHITE);t.setPadding(0,12,0,24);
        return t;
    }
    static boolean validSource(Uri uri){
        if(uri==null || !"content".equals(uri.getScheme()) || uri.getPathSegments().size()!=1
            || !"report".equals(uri.getLastPathSegment()))return false;
        String authority=uri.getAuthority();
        return "io.github.phloraxx.senseguide.lab.bridge.debugreport".equals(authority)
            || "io.github.phloraxx.senseguide.lab.debugreport".equals(authority);
    }
    private String importReport()throws Exception{
        if(!validSource(reportUri))throw new SecurityException("Untrusted report provider");
        ByteArrayOutputStream data=new ByteArrayOutputStream();
        try(InputStream in=getContentResolver().openInputStream(reportUri)){
            if(in==null)throw new FileNotFoundException("Provider did not return report");
            byte[] buffer=new byte[4096];int n;
            while((n=in.read(buffer))!=-1){
                if(data.size()+n>LIMIT_BYTES)throw new IOException("Report exceeds 64 KiB");
                data.write(buffer,0,n);
            }
        }
        byte[] bytes=data.toByteArray();
        String prefix="{\"schema\":\"senseguide.debug.v1\"";
        if(bytes.length<26||!new String(bytes,0,Math.min(bytes.length,120),StandardCharsets.UTF_8)
                .startsWith(prefix))
            throw new IOException("Unknown diagnostics schema");
        File folder=new File(getFilesDir(),"workspace/senseguide/reports");
        if(!folder.isDirectory() && !folder.mkdirs())throw new IOException("Cannot create private report folder");
        String filename="diagnostics-"+System.currentTimeMillis()+".jsonl";
        Path tmp=Files.createTempFile(folder.toPath(),".incoming-",".tmp");
        boolean saved=false;
        try {
            Files.write(tmp,bytes,StandardOpenOption.TRUNCATE_EXISTING);
            Path dest=new File(folder,filename).toPath();
            try { Files.move(tmp,dest,StandardCopyOption.ATOMIC_MOVE); }
            catch(AtomicMoveNotSupportedException e) {
                Files.move(tmp,dest,StandardCopyOption.REPLACE_EXISTING);
            }
            saved=true;
        }finally{if(!saved)Files.deleteIfExists(tmp);}
        return "senseguide/reports/"+filename;
    }
}
