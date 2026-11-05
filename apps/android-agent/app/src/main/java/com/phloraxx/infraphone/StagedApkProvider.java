package com.phloraxx.infraphone;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import java.io.File;
import java.io.FileNotFoundException;

/** Grants a single, user-selected staged APK to Android's installer. */
public final class StagedApkProvider extends ContentProvider {
    @Override public boolean onCreate(){return true;}
    @Override public String getType(Uri uri){return "application/vnd.android.package-archive";}
    @Override public ParcelFileDescriptor openFile(Uri uri,String mode)throws FileNotFoundException {
        if(!"r".equals(mode)||uri==null||uri.getPathSegments().size()!=1
            ||!"senseguide-debug.apk".equals(uri.getLastPathSegment())
            ||!(getContext().getPackageName()+".stagedapk").equals(uri.getAuthority()))
            throw new FileNotFoundException("Not a staged SenseGuide APK");
        File f=new File(getContext().getFilesDir(),"workspace/apks/senseguide-debug.apk");
        if(!f.isFile()||f.length()<1024||f.length()>256L*1024*1024)
            throw new FileNotFoundException("Missing or invalid staged APK");
        return ParcelFileDescriptor.open(f,ParcelFileDescriptor.MODE_READ_ONLY);
    }
    @Override public Cursor query(Uri uri,String[] cols,String where,String[] args,String order){return null;}
    @Override public Uri insert(Uri uri,ContentValues cv){throw new UnsupportedOperationException();}
    @Override public int delete(Uri uri,String sel,String[] args){throw new UnsupportedOperationException();}
    @Override public int update(Uri uri,ContentValues cv,String sel,String[] args){throw new UnsupportedOperationException();}
}
