//! Deterministic CI key, never a publisher/release trust root.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
fn main() {
    let dir = std::path::PathBuf::from(std::env::args_os().nth(1).expect("fixture directory"));
    let mut assets = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let name = entry.file_name().into_string().unwrap();
            if name.starts_with("infra-") && !name.ends_with(".json") && !name.ends_with(".sig") {
                assets.insert(
                    name,
                    hex::encode(Sha256::digest(std::fs::read(entry.path()).unwrap())),
                );
            }
        }
    }
    let version = std::env::args()
        .nth(2)
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    let manifest =
        serde_json::to_vec(&serde_json::json!({"version":version,"assets":assets})).unwrap();
    let key = SigningKey::from_bytes(&[31; 32]);
    std::fs::write(dir.join("infra-release.json"), &manifest).unwrap();
    std::fs::write(
        dir.join("infra-release.json.sig"),
        STANDARD.encode(key.sign(&manifest).to_bytes()),
    )
    .unwrap();
    println!("{}", STANDARD.encode(key.verifying_key().to_bytes()));
}
