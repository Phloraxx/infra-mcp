use super::*;

pub(crate) const BROWSER_SESSION_COOKIE: &str = "infra_session";

pub(crate) const BROWSER_CSRF_HEADER: &str = "x-infra-csrf";

pub(crate) const BROWSER_SESSION_TTL_MS: u64 = 12 * 60 * 60 * 1000;

#[derive(Deserialize)]
pub(crate) struct BrowserLoginInput {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ChangePasswordInput {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct BrowserSessionView {
    authenticated: bool,
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at_unix_ms: Option<u64>,
}

pub(crate) fn browser_cookie(headers: &HeaderMap) -> Option<&str> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == BROWSER_SESSION_COOKIE).then_some(value)
    })
}

pub(crate) type HmacSha256 = Hmac<Sha256>;

pub(crate) fn signed_session_message(expires_at_unix_ms: u64, nonce: &str) -> String {
    format!("infra-browser-session-v1|{expires_at_unix_ms}|{nonce}")
}

pub(crate) fn browser_session_signature(
    secret: &[u8],
    expires_at_unix_ms: u64,
    nonce: &str,
) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts arbitrary key length");
    mac.update(signed_session_message(expires_at_unix_ms, nonce).as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

pub(crate) fn create_browser_session_cookie(secret: &[u8]) -> Result<(String, u64), &'static str> {
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| "OS random source unavailable")?;
    let nonce = URL_SAFE_NO_PAD.encode(nonce);
    let expires = now_ms().saturating_add(BROWSER_SESSION_TTL_MS);
    let signature = browser_session_signature(secret, expires, &nonce);
    Ok((format!("v1.{expires}.{nonce}.{signature}"), expires))
}

pub(crate) fn browser_session_expiry(state: &AppState, headers: &HeaderMap) -> Option<u64> {
    let secret = browser_session_secret(state)?;
    let cookie = browser_cookie(headers)?;
    let mut fields = cookie.split('.');
    if fields.next()? != "v1" {
        return None;
    }
    let expires = fields.next()?.parse::<u64>().ok()?;
    let nonce = fields.next()?;
    let signature = fields.next()?;
    if fields.next().is_some() || nonce.len() > 128 || expires <= now_ms() {
        return None;
    }
    let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(signed_session_message(expires, nonce).as_bytes());
    mac.verify_slice(&signature).ok()?;
    Some(expires)
}

pub(crate) async fn require_api_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if bearer_authorized(&state, request.headers()) {
        return next.run(request).await;
    }
    if browser_session_expiry(&state, request.headers()).is_none() {
        return (StatusCode::UNAUTHORIZED, "dashboard authorization required").into_response();
    }
    let safe_method = matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let csrf_ok = request
        .headers()
        .get(BROWSER_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if safe_method || csrf_ok {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "CSRF header required").into_response()
    }
}

pub(crate) fn secure_cookie_requested(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("https"))
}

pub(crate) fn session_cookie(value: &str, max_age_seconds: u64, secure: bool) -> String {
    format!(
        "{BROWSER_SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}{}",
        if secure { "; Secure" } else { "" }
    )
}

pub(crate) async fn browser_session_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let open = state.admin_token.is_none() && state.owner_auth.is_none();
    let bearer = bearer_authorized(&state, &headers);
    let expires = browser_session_expiry(&state, &headers);
    Json(serde_json::json!({
        "authenticated":open || bearer || expires.is_some(),
        "mode":if open { "open_loopback" } else if expires.is_some() { "browser_session" } else if bearer { "bearer" } else { "none" },
        "expires_at_unix_ms":expires,
        "login_method":if state.owner_auth.is_some() { "password" } else { "token" }
    }))
}

pub(crate) fn browser_session_secret(state: &AppState) -> Option<String> {
    state
        .owner_auth
        .as_ref()
        .map(|owner| owner.session_secret())
        .or_else(|| state.admin_token.as_deref().map(str::to_owned))
}

pub(crate) async fn browser_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BrowserLoginInput>,
) -> Response {
    if let Some(owner) = &state.owner_auth {
        if headers
            .get(BROWSER_CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            != Some("1")
        {
            return (StatusCode::FORBIDDEN, "CSRF header required").into_response();
        }
        if !owner.admit_attempt() {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                "Too many sign-in attempts. Try again shortly.",
            )
                .into_response();
        }
        let Ok(permit) = owner.verification_slots.clone().try_acquire_owned() else {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Sign-in is busy. Try again shortly.",
            )
                .into_response();
        };
        let owner = owner.clone();
        let username = input.username.unwrap_or_default();
        let password = input.password.unwrap_or_default();
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            owner.verify(&username, &password)
        })
        .await
        .unwrap_or(false);
        if !valid {
            return (StatusCode::UNAUTHORIZED, "Incorrect username or password.").into_response();
        }
    } else if let Some(expected) = &state.admin_token {
        if !constant_time_eq(
            input.token.unwrap_or_default().as_bytes(),
            expected.as_bytes(),
        ) {
            return (StatusCode::UNAUTHORIZED, "invalid admin credential").into_response();
        }
    } else {
        return Json(BrowserSessionView {
            authenticated: true,
            mode: "open_loopback",
            expires_at_unix_ms: None,
        })
        .into_response();
    }
    issue_browser_session(&state, &headers)
}

pub(crate) fn issue_browser_session(state: &AppState, headers: &HeaderMap) -> Response {
    let Some(secret) = browser_session_secret(state) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let (session, expires) = match create_browser_session_cookie(secret.as_bytes()) {
        Ok(value) => value,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let mut response = Json(BrowserSessionView {
        authenticated: true,
        mode: "browser_session",
        expires_at_unix_ms: Some(expires),
    })
    .into_response();
    let cookie = session_cookie(
        &session,
        BROWSER_SESSION_TTL_MS / 1000,
        secure_cookie_requested(headers),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

pub(crate) async fn change_browser_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ChangePasswordInput>,
) -> Response {
    let Some(owner) = &state.owner_auth else {
        return (StatusCode::BAD_REQUEST, "Password login is not configured.").into_response();
    };
    if input.current_password.len() > 1024 || input.new_password.len() > 1024 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    if !owner.admit_attempt() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts. Try again shortly.",
        )
            .into_response();
    }
    let Ok(permit) = owner.verification_slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let owner = owner.clone();
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.change_password(&input.current_password, &input.new_password)
    })
    .await
    {
        Ok(Ok(())) => {
            state.emit("password_changed", None);
            issue_browser_session(&state, &headers)
        }
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn browser_logout(headers: HeaderMap) -> Response {
    if headers
        .get(BROWSER_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        != Some("1")
    {
        return (StatusCode::FORBIDDEN, "CSRF header required").into_response();
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    let cookie = session_cookie("", 0, secure_cookie_requested(&headers));
    if let Ok(value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}
