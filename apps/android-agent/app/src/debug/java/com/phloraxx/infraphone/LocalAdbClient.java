package com.phloraxx.infraphone;

import android.os.Build;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;

import java.math.BigInteger;
import java.security.KeyPairGenerator;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.cert.Certificate;
import java.util.Date;
import java.util.concurrent.TimeUnit;

import javax.security.auth.x500.X500Principal;

import io.github.muntashirakon.adb.AbsAdbConnectionManager;

/**
 * Experimental localhost-only ADB client for the debug APK.
 * Never use the signed Infra device key as an ADB host key.
 * This class has no path into the MCP request dispatcher.
 */
final class LocalAdbClient extends AbsAdbConnectionManager {
    private static final String KEY_ALIAS = "infra-local-adb-debug-v1";
    private final PrivateKey privateKey;
    private final Certificate certificate;

    LocalAdbClient() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (!store.containsAlias(KEY_ALIAS)) {
            KeyPairGenerator generator = KeyPairGenerator.getInstance(
                KeyProperties.KEY_ALGORITHM_RSA, "AndroidKeyStore");
            long now = System.currentTimeMillis();
            KeyGenParameterSpec spec = new KeyGenParameterSpec.Builder(
                KEY_ALIAS, KeyProperties.PURPOSE_SIGN | KeyProperties.PURPOSE_VERIFY)
                .setKeySize(2048)
                .setDigests(KeyProperties.DIGEST_SHA1, KeyProperties.DIGEST_SHA256,
                    KeyProperties.DIGEST_SHA512)
                .setSignaturePaddings(KeyProperties.SIGNATURE_PADDING_RSA_PKCS1,
                    KeyProperties.SIGNATURE_PADDING_RSA_PSS)
                .setCertificateSubject(new X500Principal("CN=Infra Phone local ADB"))
                .setCertificateSerialNumber(BigInteger.valueOf(now))
                .setCertificateNotBefore(new Date(now - 60000L))
                .setCertificateNotAfter(new Date(now + 10L * 365 * 24 * 60 * 60 * 1000))
                .build();
            generator.initialize(spec);
            generator.generateKeyPair();
            store.load(null);
        }
        privateKey = (PrivateKey) store.getKey(KEY_ALIAS, null);
        certificate = store.getCertificate(KEY_ALIAS);
        if (privateKey == null || certificate == null)
            throw new IllegalStateException("Android Keystore ADB identity unavailable");
        setHostAddress("127.0.0.1");
        setApi(Build.VERSION.SDK_INT);
        setTimeout(6, TimeUnit.SECONDS);
        setThrowOnUnauthorised(true);
    }

    @Override protected PrivateKey getPrivateKey() { return privateKey; }
    @Override protected Certificate getCertificate() { return certificate; }
    @Override protected String getDeviceName() { return "Infra Phone (local lab)"; }

    boolean pairLocally(int port, String code) throws Exception {
        checkPort(port);
        if (code == null || !code.matches("[0-9]{6}"))
            throw new IllegalArgumentException("Enter the six-digit code from Android Wireless debugging");
        return pair("127.0.0.1", port, code);
    }

    void connectLocally(int port) throws Exception {
        checkPort(port);
        if (!isConnected()) connect("127.0.0.1", port);
        if (!isConnected()) throw new IllegalStateException("ADB connection unavailable");
    }

    private static void checkPort(int port) {
        if (port < 1024 || port > 65535 || port == 5555)
            throw new IllegalArgumentException(
                "Enter the current Wireless debugging pairing/connection port, not legacy TCP 5555");
    }
}
