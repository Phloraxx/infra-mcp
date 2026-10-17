use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const ACCESS_TTL_SECS: u64 = 60 * 60;
const REFRESH_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const CODE_TTL_SECS: u64 = 10 * 60;
const CLIENT_TTL_SECS: u64 = 180 * 24 * 60 * 60;
pub const CONTROL_SCOPE: &str = "infra:control";
pub const OFFLINE_SCOPE: &str = "offline_access";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRegistration {
    pub redirect_uris: Vec<String>,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
pub struct RegistrationRequest {
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    pub grant_types: Vec<String>,
    #[serde(default)]
    pub response_types: Vec<String>,
    #[serde(default)]
    pub client_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegistrationResponse {
    pub client_id: String,
    pub client_id_issued_at: u64,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: &'static str,
    pub grant_types: Vec<&'static str>,
    pub response_types: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthorizationCode {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub scope: String,
    pub resource: String,
    pub expires_at: u64,
}

#[derive(Clone, Default)]
pub struct OAuthState {
    codes: Arc<Mutex<HashMap<String, AuthorizationCode>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TokenClaims {
    kind: String,
    exp: u64,
    client_id: String,
    scope: String,
    resource: String,
    nonce: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: String,
    pub resource: String,
}

impl OAuthState {
    pub async fn issue_code(&self, code: AuthorizationCode) -> Result<String, &'static str> {
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|_| "random source unavailable")?;
        let value = URL_SAFE_NO_PAD.encode(raw);
        let now = now_secs();
        let mut codes = self.codes.lock().await;
        codes.retain(|_, entry| entry.expires_at > now);
        codes.insert(value.clone(), code);
        Ok(value)
    }

    pub async fn take_code(&self, code: &str) -> Option<AuthorizationCode> {
        let now = now_secs();
        let mut codes = self.codes.lock().await;
        codes.retain(|_, entry| entry.expires_at > now);
        codes.remove(code).filter(|entry| entry.expires_at > now)
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn code_expiry() -> u64 {
    now_secs().saturating_add(CODE_TTL_SECS)
}

pub fn register_client(
    secret: &[u8],
    input: RegistrationRequest,
) -> Result<RegistrationResponse, &'static str> {
    if input.redirect_uris.is_empty() || input.redirect_uris.len() > 8 {
        return Err("redirect_uris must contain 1-8 entries");
    }
    for uri in &input.redirect_uris {
        validate_redirect_uri(uri)?;
    }
    if input
        .token_endpoint_auth_method
        .as_deref()
        .is_some_and(|method| method != "none")
    {
        return Err("only token_endpoint_auth_method=none is supported");
    }
    if !input.grant_types.is_empty()
        && input
            .grant_types
            .iter()
            .any(|grant| grant != "authorization_code" && grant != "refresh_token")
    {
        return Err("unsupported grant type");
    }
    if !input.response_types.is_empty()
        && input
            .response_types
            .iter()
            .any(|response| response != "code")
    {
        return Err("unsupported response type");
    }
    let issued_at = now_secs();
    let registration = ClientRegistration {
        redirect_uris: input.redirect_uris.clone(),
        issued_at,
        expires_at: issued_at.saturating_add(CLIENT_TTL_SECS),
    };
    let payload = serde_json::to_vec(&registration).map_err(|_| "client encoding failed")?;
    let client_id = signed_value(secret, "dcr1", &payload)?;
    Ok(RegistrationResponse {
        client_id,
        client_id_issued_at: issued_at,
        redirect_uris: input.redirect_uris,
        token_endpoint_auth_method: "none",
        grant_types: vec!["authorization_code", "refresh_token"],
        response_types: vec!["code"],
        client_name: input.client_name,
    })
}

pub fn validate_client(secret: &[u8], client_id: &str) -> Option<ClientRegistration> {
    let payload = verify_signed_value(secret, "dcr1", client_id)?;
    let registration: ClientRegistration = serde_json::from_slice(&payload).ok()?;
    (registration.expires_at > now_secs()).then_some(registration)
}

pub fn verify_pkce(code_verifier: &str, expected_challenge: &str) -> bool {
    if !(43..=128).contains(&code_verifier.len()) {
        return false;
    }
    let digest = Sha256::digest(code_verifier.as_bytes());
    let actual = URL_SAFE_NO_PAD.encode(digest);
    constant_time_eq(actual.as_bytes(), expected_challenge.as_bytes())
}

pub fn issue_tokens(
    secret: &[u8],
    client_id: &str,
    scope: &str,
    resource: &str,
) -> Result<TokenResponse, &'static str> {
    let now = now_secs();
    let access_claims = TokenClaims {
        kind: "access".into(),
        exp: now.saturating_add(ACCESS_TTL_SECS),
        client_id: client_id.to_owned(),
        scope: scope.to_owned(),
        resource: resource.to_owned(),
        nonce: random_nonce()?,
    };
    let refresh_claims = TokenClaims {
        kind: "refresh".into(),
        exp: now.saturating_add(REFRESH_TTL_SECS),
        client_id: client_id.to_owned(),
        scope: scope.to_owned(),
        resource: resource.to_owned(),
        nonce: random_nonce()?,
    };
    Ok(TokenResponse {
        access_token: encode_claims(secret, "iat1", &access_claims)?,
        token_type: "Bearer",
        expires_in: ACCESS_TTL_SECS,
        refresh_token: encode_claims(secret, "irt1", &refresh_claims)?,
        scope: scope.to_owned(),
        resource: resource.to_owned(),
    })
}

pub fn validate_access_token(secret: &[u8], token: &str, expected_resource: &str) -> bool {
    let Some(claims) = decode_claims(secret, "iat1", token) else {
        return false;
    };
    claims.kind == "access"
        && claims.exp > now_secs()
        && claims.resource == expected_resource
        && scope_contains(&claims.scope, CONTROL_SCOPE)
}

pub fn refresh_tokens(
    secret: &[u8],
    refresh_token: &str,
    client_id: &str,
    expected_resource: &str,
) -> Result<TokenResponse, &'static str> {
    let claims = decode_claims(secret, "irt1", refresh_token).ok_or("invalid refresh token")?;
    if claims.kind != "refresh"
        || claims.exp <= now_secs()
        || claims.client_id != client_id
        || claims.resource != expected_resource
        || !scope_contains(&claims.scope, CONTROL_SCOPE)
    {
        return Err("invalid refresh token");
    }
    issue_tokens(secret, client_id, &claims.scope, expected_resource)
}

pub fn normalize_scope(scope: Option<&str>) -> Result<String, &'static str> {
    let requested = scope.unwrap_or(CONTROL_SCOPE);
    let mut out = Vec::new();
    for item in requested.split_ascii_whitespace() {
        match item {
            CONTROL_SCOPE | OFFLINE_SCOPE => {
                if !out.contains(&item) {
                    out.push(item);
                }
            }
            _ => return Err("unsupported scope"),
        }
    }
    if !out.contains(&CONTROL_SCOPE) {
        out.insert(0, CONTROL_SCOPE);
    }
    Ok(out.join(" "))
}

pub fn scope_contains(scope: &str, needle: &str) -> bool {
    scope.split_ascii_whitespace().any(|item| item == needle)
}

fn validate_redirect_uri(uri: &str) -> Result<(), &'static str> {
    let parsed = url::Url::parse(uri).map_err(|_| "invalid redirect_uri")?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() || parsed.fragment().is_some() {
        return Err("redirect_uri must be an HTTPS URL without a fragment");
    }
    Ok(())
}

fn random_nonce() -> Result<String, &'static str> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| "random source unavailable")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn encode_claims(
    secret: &[u8],
    prefix: &str,
    claims: &TokenClaims,
) -> Result<String, &'static str> {
    let payload = serde_json::to_vec(claims).map_err(|_| "token encoding failed")?;
    signed_value(secret, prefix, &payload)
}

fn decode_claims(secret: &[u8], prefix: &str, value: &str) -> Option<TokenClaims> {
    let payload = verify_signed_value(secret, prefix, value)?;
    serde_json::from_slice(&payload).ok()
}

fn signed_value(secret: &[u8], prefix: &str, payload: &[u8]) -> Result<String, &'static str> {
    let body = URL_SAFE_NO_PAD.encode(payload);
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| "invalid signing key")?;
    mac.update(prefix.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{prefix}.{body}.{signature}"))
}

fn verify_signed_value(secret: &[u8], expected_prefix: &str, value: &str) -> Option<Vec<u8>> {
    let mut parts = value.split('.');
    let prefix = parts.next()?;
    let body = parts.next()?;
    let signature = parts.next()?;
    if parts.next().is_some() || prefix != expected_prefix {
        return None;
    }
    let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(prefix.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    mac.verify_slice(&signature).ok()?;
    URL_SAFE_NO_PAD.decode(body).ok()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_registration_and_tokens_round_trip() {
        let secret = b"this-is-a-test-secret-which-is-long-enough";
        let registration = register_client(
            secret,
            RegistrationRequest {
                redirect_uris: vec!["https://chatgpt.com/oauth/test/callback".into()],
                token_endpoint_auth_method: Some("none".into()),
                grant_types: vec!["authorization_code".into(), "refresh_token".into()],
                response_types: vec!["code".into()],
                client_name: Some("ChatGPT".into()),
            },
        )
        .unwrap();
        let decoded = validate_client(secret, &registration.client_id).unwrap();
        assert_eq!(decoded.redirect_uris, registration.redirect_uris);
        let tokens = issue_tokens(
            secret,
            &registration.client_id,
            "infra:control offline_access",
            "https://infra.example.com/mcp",
        )
        .unwrap();
        assert!(validate_access_token(
            secret,
            &tokens.access_token,
            "https://infra.example.com/mcp"
        ));
        assert!(
            refresh_tokens(
                secret,
                &tokens.refresh_token,
                &registration.client_id,
                "https://infra.example.com/mcp"
            )
            .is_ok()
        );
    }

    #[test]
    fn pkce_s256_verifies() {
        let verifier = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~abc";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert!(verify_pkce(verifier, &challenge));
        assert!(!verify_pkce(verifier, "wrong"));
    }
}
