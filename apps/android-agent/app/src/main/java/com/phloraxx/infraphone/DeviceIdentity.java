package com.phloraxx.infraphone;

import android.content.Context;
import android.content.SharedPreferences;
import android.os.SystemClock;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters;
import org.bouncycastle.crypto.signers.Ed25519Signer;
import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.security.SecureRandom;
import java.util.UUID;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** Per-installation identity; only a wrapped seed is stored outside AndroidKeyStore. */
public final class DeviceIdentity {
    private static final String ALIAS="infraphone-v1-wrap";
    private static final String PREF="signed_device";
    private static final SecureRandom RNG=new SecureRandom();
    private final Context app;
    private final SharedPreferences pref;
    public DeviceIdentity(Context context) {
        app=context.getApplicationContext();
        pref=app.getSharedPreferences(PREF,Context.MODE_PRIVATE);
    }
    public boolean enrolled(){return !pref.getString("device_id","").isEmpty();}
    public String deviceId(){return pref.getString("device_id","");}
    public String name(){return pref.getString("device_name","Android phone");}
    public String gateway(){return pref.getString("gateway","");}
    public String pendingRequest(){return pref.getString("request_id","");}
    public void configureGateway(String url){pref.edit().putString("gateway",url).apply();}
    public synchronized String ensurePending() throws Exception {
        if(pref.getString("seed","").isEmpty()){
            byte[] seed=new byte[32];RNG.nextBytes(seed);
            Cipher c=Cipher.getInstance("AES/GCM/NoPadding");
            c.init(Cipher.ENCRYPT_MODE,wrappingKey());
            byte[] encrypted=c.doFinal(seed);
            java.util.Arrays.fill(seed,(byte)0);
            pref.edit().putString("seed",b64(encrypted))
                .putString("iv",b64(c.getIV()))
                .putString("request_id",UUID.randomUUID().toString()).commit();
        }
        return pref.getString("request_id","");
    }
    public String publicKey() throws Exception {
        byte[] seed=unwrap();
        try {
            return b64(new Ed25519PrivateKeyParameters(seed,0).generatePublicKey().getEncoded());
        }finally{java.util.Arrays.fill(seed,(byte)0);}
    }
    public void enrolledAs(String deviceId,String deviceName){
        pref.edit().putString("device_id",deviceId)
            .putString("device_name",deviceName).remove("request_id").apply();
    }
    public synchronized void clear(){
        pref.edit().clear().commit();
        try{KeyStore store=KeyStore.getInstance("AndroidKeyStore");store.load(null);store.deleteEntry(ALIAS);}
        catch(Exception ignored){}
    }
    private SecretKey wrappingKey() throws Exception {
        KeyStore store=KeyStore.getInstance("AndroidKeyStore");store.load(null);
        KeyStore.Entry e=store.getEntry(ALIAS,null);
        if(e instanceof KeyStore.SecretKeyEntry)return ((KeyStore.SecretKeyEntry)e).getSecretKey();
        KeyGenerator g=KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore");
        g.init(new KeyGenParameterSpec.Builder(ALIAS,KeyProperties.PURPOSE_ENCRYPT|KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true).build());
        return g.generateKey();
    }
    private byte[] unwrap() throws Exception {
        String encrypted=pref.getString("seed","");
        if(encrypted.isEmpty())throw new IllegalStateException("Device identity not created");
        Cipher c=Cipher.getInstance("AES/GCM/NoPadding");
        c.init(Cipher.DECRYPT_MODE,wrappingKey(),
            new GCMParameterSpec(128,unb64(pref.getString("iv",""))));
        return c.doFinal(unb64(encrypted));
    }
    public String sign(String nonce,String epoch,int protocol,String id,String instance,String boot) throws Exception {
        byte[] bytes=signingMessage(nonce,epoch,protocol,id,instance,boot);
        byte[] seed=unwrap();
        try {
            Ed25519Signer s=new Ed25519Signer();
            s.init(true,new Ed25519PrivateKeyParameters(seed,0));
            s.update(bytes,0,bytes.length);
            return b64(s.generateSignature());
        }finally{java.util.Arrays.fill(seed,(byte)0);}
    }
    public static byte[] signingMessage(String nonce,String epoch,int protocol,String id,String instance,String boot) throws Exception {
        ByteArrayOutputStream out=new ByteArrayOutputStream();
        out.write("infra-v1-device-auth\0".getBytes(StandardCharsets.UTF_8));
        field(out,nonce.getBytes(StandardCharsets.UTF_8));
        field(out,uuidBytes(epoch));
        field(out,ByteBuffer.allocate(2).putShort((short)protocol).array());
        field(out,id.getBytes(StandardCharsets.UTF_8));
        field(out,uuidBytes(instance));
        field(out,boot.getBytes(StandardCharsets.UTF_8));
        return out.toByteArray();
    }
    private static void field(ByteArrayOutputStream out,byte[] data)throws Exception {
        out.write(ByteBuffer.allocate(4).putInt(data.length).array());out.write(data);
    }
    private static byte[] uuidBytes(String text){
        UUID u=UUID.fromString(text);
        return ByteBuffer.allocate(16).putLong(u.getMostSignificantBits()).putLong(u.getLeastSignificantBits()).array();
    }
    public synchronized String bootId(){
        long estimatedBoot=System.currentTimeMillis()-SystemClock.elapsedRealtime();
        long last=pref.getLong("boot_epoch",0L);
        String current=pref.getString("boot_id","");
        if(current.isEmpty()||Math.abs(estimatedBoot-last)>60000){
            current=UUID.randomUUID().toString();
            pref.edit().putLong("boot_epoch",estimatedBoot).putString("boot_id",current).apply();
        }
        return current;
    }
    static String b64(byte[] data){return Base64.encodeToString(data,Base64.URL_SAFE|Base64.NO_WRAP|Base64.NO_PADDING);}
    static byte[] unb64(String text){return Base64.decode(text,Base64.URL_SAFE|Base64.NO_WRAP|Base64.NO_PADDING);}
}
