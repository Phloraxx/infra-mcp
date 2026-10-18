package com.phloraxx.infraphone;

import android.content.Context;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.*;
import java.nio.charset.*;
import java.nio.file.*;
import java.nio.file.attribute.BasicFileAttributes;
import java.security.MessageDigest;
import java.util.*;

/** Implements a strict subset of existing Infra files RPC. All paths stay in app-private workspace. */
public final class AndroidFilesOps {
    private final ScopedWorkspace workspace;
    private static final int MAX_INLINE=1024*1024;
    private static final int MAX_LIST=500;
    public AndroidFilesOps(Context ctx)throws Exception{
        workspace=new ScopedWorkspace(new File(ctx.getFilesDir(),"workspace"));
    }
    public JSONObject handle(JSONObject op)throws Exception {
        String action=op.optString("action","");
        switch(action){
            case "list":return list(op.getString("path"));
            case "stat":return stat(op.getString("path"));
            case "read":return read(op);
            case "hash":return hash(op.getString("path"));
            case "mkdir":return mkdir(op.getString("path"));
            case "write":return write(op);
            case "exact_edit":return exactEdit(op);
            default:return null; // caller converts to explicit unsupported, never untrusted shell
        }
    }
    private File resolve(String path)throws IOException{return workspace.resolve(path);}
    private JSONObject list(String path)throws Exception{
        File dir=resolve(path);
        if(!dir.isDirectory())throw new IOException("Directory not found");
        File[] entries=dir.listFiles();
        if(entries==null)throw new IOException("Cannot list directory");
        Arrays.sort(entries,Comparator.comparing(File::getName,String.CASE_INSENSITIVE_ORDER));
        JSONArray items=new JSONArray();
        for(int i=0;i<Math.min(entries.length,MAX_LIST);i++){
            File child=resolve(path.isEmpty()?entries[i].getName():path+"/"+entries[i].getName());
            boolean directory=child.isDirectory();
            JSONObject item=new JSONObject().put("name",child.getName())
                .put("path",path.isEmpty()?child.getName():path+"/"+child.getName())
                .put("kind",directory?"directory":"file")
                .put("size",directory?JSONObject.NULL:child.length())
                .put("modified_unix_ms",child.lastModified());
            items.put(item);
        }
        return new JSONObject().put("path",path).put("entries",items).put("truncated",entries.length>MAX_LIST);
    }
    private JSONObject stat(String path)throws Exception{
        File f=resolve(path);
        if(!f.exists())throw new IOException("File not found");
        BasicFileAttributes a=Files.readAttributes(f.toPath(),BasicFileAttributes.class);
        return new JSONObject().put("path",path)
            .put("kind",a.isDirectory()?"directory":(a.isRegularFile()?"file":"other"))
            .put("size",a.size()).put("readonly",!f.canWrite())
            .put("modified_unix_ms",a.lastModifiedTime().toMillis())
            .put("created_unix_ms",a.creationTime().toMillis());
    }
    private JSONObject read(JSONObject request)throws Exception{
        String path=request.getString("path");
        File f=resolve(path);
        if(!f.isFile())throw new IOException("File not found");
        long size=f.length(), offset=request.optLong("offset",0);
        if(offset<0)throw new IOException("Negative offset");
        long start=Math.min(size,offset);
        int length=request.has("length")&&!request.isNull("length")?
            request.getInt("length"):65536;
        length=Math.min(MAX_INLINE,Math.max(1,length));
        int available=(int)Math.min((long)length,size-start);
        byte[] bytes=new byte[available];
        try(RandomAccessFile in=new RandomAccessFile(f,"r")){
            in.seek(start);in.readFully(bytes);
        }
        for(byte b:bytes)if(b==0)throw new IOException("Binary file: use transfer instead");
        String value=StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(java.nio.ByteBuffer.wrap(bytes)).toString();
        return new JSONObject().put("path",path).put("content",value).put("offset",start)
            .put("next_offset",start+bytes.length).put("size",size)
            .put("has_more",start+bytes.length<size);
    }
    private JSONObject hash(String path)throws Exception{
        File f=resolve(path);
        if(!f.isFile())throw new IOException("File not found");
        MessageDigest digest=MessageDigest.getInstance("SHA-256");
        try(InputStream in=new FileInputStream(f)){
            byte[] buf=new byte[8192];int n;
            while((n=in.read(buf))!=-1)digest.update(buf,0,n);
        }
        StringBuilder hex=new StringBuilder();
        for(byte b:digest.digest())hex.append(String.format(Locale.ROOT,"%02x",b&255));
        return new JSONObject().put("path",path).put("algorithm","sha256")
            .put("sha256",hex.toString()).put("size",f.length());
    }
    private JSONObject mkdir(String path)throws Exception{
        File dir=resolve(path);
        if(!dir.isDirectory() && !dir.mkdirs())throw new IOException("Cannot create directory");
        return new JSONObject().put("ok",true).put("path",path);
    }
    private JSONObject write(JSONObject op)throws Exception {
        String path=op.getString("path"),content=op.getString("content");
        byte[] bytes=content.getBytes(StandardCharsets.UTF_8);
        if(bytes.length>MAX_INLINE)throw new IOException("Maximum write 1 MiB");
        File file=resolve(path);
        if(file.equals(workspace.root())||file.isDirectory())throw new IOException("Path is a directory");
        File parent=file.getParentFile();
        boolean createParents=op.optBoolean("create_parents",false);
        if(createParents&&!parent.isDirectory()&&!parent.mkdirs())throw new IOException("Cannot create parent directory");
        if(!parent.isDirectory())throw new IOException("Parent directory not found");
        atomicWrite(file,bytes);
        return new JSONObject().put("ok",true).put("path",path).put("bytes",bytes.length);
    }
    private JSONObject exactEdit(JSONObject op)throws Exception{
        String path=op.getString("path"),old=op.getString("old"),replacement=op.getString("new");
        int expected=op.optInt("expected_replacements",1);
        if(old.isEmpty()||expected<1||expected>10000)throw new IOException("Invalid replacement constraint");
        File file=resolve(path);
        if(!file.isFile()||file.length()>MAX_INLINE)throw new IOException("File missing or exceeds 1 MiB");
        byte[] bytes=Files.readAllBytes(file.toPath());
        for(byte b:bytes)if(b==0)throw new IOException("Binary file");
        String before=StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                .decode(java.nio.ByteBuffer.wrap(bytes)).toString();
        int found=0,at=0;
        while((at=before.indexOf(old,at))>=0){found++;at+=old.length();}
        if(found!=expected)throw new IOException("Expected "+expected+" replacements, found "+found);
        String after=before.replace(old,replacement);
        byte[] result=after.getBytes(StandardCharsets.UTF_8);
        if(result.length>MAX_INLINE)throw new IOException("Edited file exceeds 1 MiB");
        atomicWrite(file,result);
        return new JSONObject().put("ok",true).put("path",path)
            .put("replacements",found).put("bytes",result.length);
    }
    private void atomicWrite(File dst,byte[] bytes)throws Exception {
        Path dir=dst.getParentFile().toPath();
        Path tmp=Files.createTempFile(dir,".infra-write-",".tmp");
        boolean completed=false;
        try {
            Files.write(tmp,bytes,StandardOpenOption.TRUNCATE_EXISTING);
            try {
                Files.move(tmp,dst.toPath(),StandardCopyOption.ATOMIC_MOVE,StandardCopyOption.REPLACE_EXISTING);
            }catch(AtomicMoveNotSupportedException e){
                Files.move(tmp,dst.toPath(),StandardCopyOption.REPLACE_EXISTING);
            }
            completed=true;
        }finally{if(!completed)Files.deleteIfExists(tmp);}
    }
}
