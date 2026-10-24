# Native SPAKE2 library resolves this class by its binary name.
# Keep the JNI boundary; other app and crypto classes may be shrunk by R8.
-keep class io.github.muntashirakon.crypto.spake2.Spake2Context { *; }

# Android keeps Manifest-declared components automatically.
# Do not add broad -keep rules for BouncyCastle: only its Ed25519 primitives
# are needed by Infra's device identity and R8 should discard the rest.
