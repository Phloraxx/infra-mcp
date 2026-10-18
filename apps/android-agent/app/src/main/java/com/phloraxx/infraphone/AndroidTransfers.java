package com.phloraxx.infraphone;

import android.content.Context;
import android.util.Base64;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Infra v1 transfer protocol scoped to private workspace.
 * Bounded 128 KiB chunks, 256 MiB files, idempotent retries and SHA-256 verification.
 * Transfer handles survive reconnects while the foreground service process is alive.
 */
public final class AndroidTransfers {
    private static final int CHUNK=128*1024;
    private static final long MAX_SIZE=256L*1024*1024;
    private static final int MAX_TRANSFERS=8;
    private final ScopedWorkspace workspace;
    private final Map<String,Entry> active=new ConcurrentHashMap<>();
    private static final class Entry {
        String id,path,direction,status,hash; File dest,staging;
        long size,progress;
        boolean overwrite;
        JSONObject asJson()throws Exception {
            return new JSONObject().put("id",id).put("direction",direction)
                .put("path",path).put("size",size)
                .put("sha256",hash==null?JSONObject.NULL:hash)
                .put("status",status).put("bytes_transferred",progress);
        }
    }
    public AndroidTransfers(Context ctx)throws Exception {
        workspace=new ScopedWorkspace(new File(ctx.getFilesDir(),"workspace"));
    }
    public synchronized JSONObject handle(JSONObject op)throws Exception {
        String action=op.optString("action","");
        switch(action){
            case "upload_begin":return beginUpload(op);
            case "upload_chunk":return uploadChunk(op);
            case "upload_commit":return commit(op);
            case "download_begin":return beginDownload(op);
            case "download_chunk":return downloadChunk(op);
            case "status":return new JSONObject().put("transfer",lookup(op).asJson());
            case "list":{
                JSONArray a=new JSONArray();
                for(Entry e:active.values())a.put(e.asJson());
                return new JSONObject().put("transfers",a);
            }
            case "cancel":{
                Entry e=lookup(op);
                active.remove(e.id);
                if(e.staging!=null)e.staging.delete();
                e.status="cancelled";
                return new JSONObject().put("transfer",e.asJson());
            }
            default:return null;
        }
    }
    private Entry lookup(JSONObject req)throws Exception {
        String id=req.getString("transfer_id");
        Entry e=active.get(id);
        if(e==null)throw new IOException("Unknown/expired transfer");
        return e;
    }
    private Entry create(String path,String direction)throws Exception {
        if(active.size()>=MAX_TRANSFERS)throw new IOException("Too many active transfers");
        if(path==null || path.isEmpty())throw new IOException("File path required");
        Entry e=new Entry();
        e.id=UUID.randomUUID().toString();e.path=path;e.direction=direction;
        e.dest=workspace.resolve(path);
        if(e.dest.equals(workspace.root()) || e.dest.isDirectory())
            throw new IOException("Expected regular workspace file");
        return e;
    }
    private JSONObject beginUpload(JSONObject req)throws Exception {
        String path=req.getString("path");
        long size=req.getLong("size");
        if(size<0||size>MAX_SIZE)throw new IOException("Upload exceeds size cap");
        String hash=req.isNull("sha256")?null:req.optString("sha256",null);
        if(hash!=null && !hash.matches("(?i)[0-9a-f]{64}"))throw new IOException("Invalid SHA-256");
        Entry e=create(path,"upload");
        e.size=size;e.hash=hash==null?null:hash.toLowerCase(Locale.ROOT);
        e.overwrite=req.optBoolean("overwrite",false);
        if(e.dest.exists() && !e.overwrite)throw new IOException("File exists, overwrite denied");
        File parent=e.dest.getParentFile();
        if(!parent.isDirectory() && (!req.optBoolean("create_parents",false)||!parent.mkdirs()))
            throw new IOException("Missing parent directory");
        e.staging=File.createTempFile(".infra-upload-",".part",parent);
        e.status="uploading";
        active.put(e.id,e);
        return new JSONObject().put("transfer",e.asJson()).put("chunk_size",CHUNK);
    }
    private JSONObject uploadChunk(JSONObject req)throws Exception {
        Entry e=lookup(req);
        if(!"upload".equals(e.direction)||!"uploading".equals(e.status))
            throw new IOException("Not an active upload");
        String encoded=req.getString("data_base64");
        if(encoded.length()>CHUNK*2)throw new IOException("Encoded chunk too large");
        byte[] bytes=Base64.decode(encoded,Base64.DEFAULT);
        if(bytes.length<1||bytes.length>CHUNK)throw new IOException("Invalid chunk size");
        long offset=req.getLong("offset");
        if(offset<0||offset>e.progress||offset+bytes.length>e.size)
            throw new IOException("Upload offset/size invalid");
        try(RandomAccessFile f=new RandomAccessFile(e.staging,"rw")){
            if(offset<e.progress){
                if(offset+bytes.length>e.progress)throw new IOException("Partial overlap");
                byte[] existing=new byte[bytes.length];
                f.seek(offset);f.readFully(existing);
                if(!Arrays.equals(existing,bytes))throw new IOException("Retry bytes differ");
            }else{
                f.seek(offset);f.write(bytes);
                e.progress+=bytes.length;
            }
        }
        return new JSONObject().put("transfer",e.asJson()).put("next_offset",e.progress)
            .put("has_more",e.progress<e.size);
    }
    private JSONObject commit(JSONObject req)throws Exception {
        Entry e=lookup(req);
        if(!"upload".equals(e.direction)||!"uploading".equals(e.status)||e.progress!=e.size)
            throw new IOException("Incomplete or inactive upload");
        String digest=sha256(e.staging);
        if(e.hash!=null && !e.hash.equals(digest))throw new IOException("SHA-256 mismatch");
        if(e.dest.exists() && !e.overwrite)throw new IOException("Destination exists");
        // Move stays inside one app-private filesystem, so an atomic rename is expected.
        try {java.nio.file.Files.move(e.staging.toPath(),e.dest.toPath(),
             java.nio.file.StandardCopyOption.ATOMIC_MOVE,
             java.nio.file.StandardCopyOption.REPLACE_EXISTING);}
        catch(java.nio.file.AtomicMoveNotSupportedException exc){
            java.nio.file.Files.move(e.staging.toPath(),e.dest.toPath(),
                java.nio.file.StandardCopyOption.REPLACE_EXISTING);
        }
        e.status="completed";e.staging=null;e.hash=digest;
        return new JSONObject().put("transfer",e.asJson()).put("sha256",digest);
    }
    private JSONObject beginDownload(JSONObject req)throws Exception {
        Entry e=create(req.getString("path"),"download");
        if(!e.dest.isFile())throw new FileNotFoundException("File missing");
        e.size=e.dest.length();
        if(e.size>MAX_SIZE)throw new IOException("File exceeds transfer cap");
        e.hash=sha256(e.dest);e.status="ready";
        active.put(e.id,e);
        return new JSONObject().put("transfer",e.asJson()).put("chunk_size",CHUNK);
    }
    private JSONObject downloadChunk(JSONObject req)throws Exception {
        Entry e=lookup(req);
        if(!"download".equals(e.direction))throw new IOException("Not a download");
        if(e.dest.length()!=e.size)throw new IOException("Source changed size");
        long offset=req.getLong("offset");
        if(offset<0)throw new IOException("Negative offset");
        long start=Math.min(offset,e.size);
        int len=Math.min(CHUNK,Math.max(1,req.optInt("length",CHUNK)));
        byte[] b=new byte[(int)Math.min(len,e.size-start)];
        try(RandomAccessFile f=new RandomAccessFile(e.dest,"r")){
            f.seek(start);f.readFully(b);
        }
        long next=start+b.length;
        e.progress=Math.max(e.progress,next);
        if(next==e.size){
            if(!e.hash.equals(sha256(e.dest)))throw new IOException("Source changed hash");
            e.status="completed";
        }
        return new JSONObject().put("transfer",e.asJson())
            .put("data_base64",Base64.encodeToString(b,Base64.NO_WRAP))
            .put("next_offset",next).put("has_more",next<e.size);
    }
    private static String sha256(File file)throws Exception{
        MessageDigest d=MessageDigest.getInstance("SHA-256");
        try(InputStream in=new BufferedInputStream(new FileInputStream(file))){
            byte[] b=new byte[16384];int n;
            while((n=in.read(b))!=-1)d.update(b,0,n);
        }
        StringBuilder out=new StringBuilder();
        for(byte b:d.digest())out.append(String.format(Locale.ROOT,"%02x",b&255));
        return out.toString();
    }
}
