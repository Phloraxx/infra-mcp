package com.phloraxx.infraphone;
import org.junit.Test;
import java.security.MessageDigest;
import java.util.Locale;
import static org.junit.Assert.*;

public class AuthProtocolTest {
    @Test public void rustCompatibleSignedPayload() throws Exception {
        byte[] data=DeviceIdentity.signingMessage("signed-nonce",
            "00000000-0000-4000-8000-000000000001",1,
            "android-fixture","00000000-0000-4000-8000-000000000002","boot-fixture");
        assertEquals(118,data.length);
        byte[] digest=MessageDigest.getInstance("SHA-256").digest(data);
        StringBuilder hex=new StringBuilder();
        for(byte b:digest)hex.append(String.format(Locale.ROOT,"%02x",b&255));
        assertEquals("ee24049221d4c59b6d8da0914704309e01b9d205bc5f18599edccbd34a02783a",hex.toString());
    }
}
