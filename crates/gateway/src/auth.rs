use anyhow::{Context, bail};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    username: String,
    password_hash: String,
    session_secret: String,
    public_origin: String,
}

pub struct OwnerAuth {
    path: PathBuf,
    credentials: RwLock<Credentials>,
    attempts: Mutex<Vec<Instant>>,
    pub verification_slots: std::sync::Arc<tokio::sync::Semaphore>,
}

impl OwnerAuth {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let credentials: Credentials = serde_json::from_slice(&fs::read(&path)?)?;
        if credentials.username.is_empty() || credentials.session_secret.len() < 32 {
            bail!("invalid dashboard authentication configuration");
        }
        let hash = PasswordHash::new(&credentials.password_hash)
            .map_err(|_| anyhow::anyhow!("invalid password hash"))?;
        if hash.algorithm.as_str() != "argon2id" {
            bail!("dashboard password must use Argon2id");
        }
        let origin = url::Url::parse(&credentials.public_origin)?;
        if origin.scheme() != "https"
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.path() != "/"
        {
            bail!("dashboard public_origin must be an HTTPS origin");
        }
        private_permissions(&path)?;
        Ok(Self {
            path,
            credentials: RwLock::new(credentials),
            attempts: Mutex::new(Vec::new()),
            verification_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
        })
    }

    pub fn session_secret(&self) -> String {
        self.credentials
            .read()
            .expect("auth lock")
            .session_secret
            .clone()
    }
    pub fn public_origin(&self) -> String {
        self.credentials
            .read()
            .expect("auth lock")
            .public_origin
            .trim_end_matches('/')
            .to_string()
    }

    pub fn admit_attempt(&self) -> bool {
        let mut attempts = self.attempts.lock().expect("rate-limit lock");
        attempts.retain(|t| t.elapsed() < Duration::from_secs(60));
        if attempts.len() >= 12 {
            return false;
        }
        attempts.push(Instant::now());
        true
    }

    pub fn verify(&self, username: &str, password: &str) -> bool {
        if username.len() > 128 || password.len() > 1024 {
            return false;
        }
        let credentials = self.credentials.read().expect("auth lock").clone();
        let Ok(hash) = PasswordHash::new(&credentials.password_hash) else {
            return false;
        };
        let password_ok = Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok();
        password_ok && super::constant_time_eq(username.as_bytes(), credentials.username.as_bytes())
    }

    pub fn change_password(&self, current: &str, new_password: &str) -> anyhow::Result<()> {
        let previous = self.credentials.read().expect("auth lock").clone();
        let hash = PasswordHash::new(&previous.password_hash)
            .map_err(|_| anyhow::anyhow!("invalid password hash"))?;
        if Argon2::default()
            .verify_password(current.as_bytes(), &hash)
            .is_err()
        {
            bail!("current password is incorrect");
        }
        let next = Credentials {
            password_hash: hash_password(new_password)?,
            session_secret: random_secret()?,
            ..previous.clone()
        };
        let mut credentials = self.credentials.write().expect("auth lock");
        if credentials.password_hash != previous.password_hash {
            bail!("password changed concurrently; sign in again");
        }
        save(&self.path, &next)?;
        *credentials = next;
        Ok(())
    }
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    if !(12..=1024).contains(&password.len()) {
        bail!("password must be 12–1024 bytes");
    }
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| anyhow::anyhow!("random source unavailable"))?;
    let salt =
        SaltString::encode_b64(&salt).map_err(|_| anyhow::anyhow!("salt encoding failed"))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| anyhow::anyhow!("password hashing failed"))?
        .to_string())
}

fn random_secret() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("random source unavailable"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn private_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn save(path: &Path, credentials: &Credentials) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    private_permissions(file.path())?;
    file.write_all(&serde_json::to_vec_pretty(credentials)?)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

// Provisioning deliberately reads the password from stdin, never process argv.
pub fn provision_from_cli() -> anyhow::Result<bool> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("init-auth") {
        return Ok(false);
    }
    if args.len() != 5 {
        bail!("Usage: infra-gateway init-auth AUTH_FILE USERNAME HTTPS_ORIGIN < password-input");
    }
    let path = PathBuf::from(&args[2]);
    if path.exists() {
        bail!("authentication file already exists; use dashboard password change");
    }
    let mut password = String::new();
    std::io::stdin().take(1026).read_to_string(&mut password)?;
    let password = password.trim_end_matches(['\r', '\n']);
    let credentials = Credentials {
        username: args[3].clone(),
        password_hash: hash_password(password)?,
        session_secret: random_secret()?,
        public_origin: args[4].clone(),
    };
    save(&path, &credentials).context("save dashboard credentials")?;
    OwnerAuth::load(path)?;
    println!("Dashboard owner login configured.");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passwords_are_hashed_rotatable_and_rate_limited() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner.json");
        let credentials = Credentials {
            username: "owner".into(),
            password_hash: hash_password("first-password-123").unwrap(),
            session_secret: random_secret().unwrap(),
            public_origin: "https://infra.example.com".into(),
        };
        save(&path, &credentials).unwrap();
        let auth = OwnerAuth::load(path.clone()).unwrap();
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .contains("first-password-123")
        );
        assert!(auth.verify("owner", "first-password-123"));
        assert!(!auth.verify("other", "first-password-123"));
        assert!(!auth.verify("owner", "wrong"));
        let secret = auth.session_secret();
        assert!(auth.change_password("wrong", "new-password-456").is_err());
        auth.change_password("first-password-123", "new-password-456")
            .unwrap();
        assert_ne!(secret, auth.session_secret());
        let reloaded = OwnerAuth::load(path).unwrap();
        assert!(reloaded.verify("owner", "new-password-456"));
        assert!(!reloaded.verify("owner", "first-password-123"));
        for _ in 0..12 {
            assert!(auth.admit_attempt());
        }
        assert!(!auth.admit_attempt());
    }
}
