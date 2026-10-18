package com.phloraxx.infraphone;

import org.junit.Test;
import org.junit.Rule;
import org.junit.rules.TemporaryFolder;
import java.io.*;
import java.nio.file.Files;
import static org.junit.Assert.*;

public class ScopedWorkspaceTest {
    @Rule public TemporaryFolder folder = new TemporaryFolder();
    @Test public void allowsOnlyRelativeWorkspacePaths()throws Exception {
        File dir=folder.newFolder("data");
        ScopedWorkspace w=new ScopedWorkspace(dir);
        assertEquals(dir.getCanonicalFile(),w.resolve(""));
        assertEquals(new File(dir,"site/index.html").getCanonicalFile(),w.resolve("site/index.html"));
        assertThrows(IOException.class,()->w.resolve("../outside.txt"));
        assertThrows(IOException.class,()->w.resolve("/etc/passwd"));
        assertThrows(IOException.class,()->w.resolve("site/../../outside.txt"));
        assertThrows(IOException.class,()->w.resolve("..\\outside.txt"));
        assertThrows(IOException.class,()->w.resolve("abc\0def"));
    }
    @Test public void preventsEscapingViaSymlink()throws Exception {
        File dir=folder.newFolder("workspace");
        File out=folder.newFolder("outside");
        Files.createSymbolicLink(new File(dir,"escape").toPath(),out.toPath());
        ScopedWorkspace w=new ScopedWorkspace(dir);
        assertThrows(IOException.class,()->w.resolve("escape/private.txt"));
    }
}
