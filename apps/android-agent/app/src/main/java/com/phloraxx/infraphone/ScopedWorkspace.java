package com.phloraxx.infraphone;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

/** Restricts signed MCP file operations to the private app workspace; never the Android FS root. */
public final class ScopedWorkspace {
    private final File root;
    public ScopedWorkspace(File root) throws IOException {
        this.root=root.getCanonicalFile();
        if(!this.root.exists() && !this.root.mkdirs())
            throw new IOException("Cannot create workspace");
        if(!this.root.isDirectory())throw new IOException("Workspace is not a directory");
    }
    public File resolve(String path) throws IOException {
        if(path==null)throw new IOException("Missing workspace path");
        // Allow the simple conventional paths callers already use, but never an absolute OS path.
        if(path.startsWith("/") || path.indexOf('\0')>=0 || path.contains("\\"))
            throw new IOException("Only relative workspace paths are permitted");
        File child=new File(root,path.isEmpty()?".":path).getCanonicalFile();
        String prefix=root.getPath()+File.separator;
        if(!child.equals(root) && !child.getPath().startsWith(prefix))
            throw new IOException("Path escapes app workspace");
        return child;
    }
    public File root(){return root;}
}
