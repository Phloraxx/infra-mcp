use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OAuthAuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthAuthorizeForm {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    scope: String,
    resource: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthTokenForm {
    grant_type: String,
    client_id: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    resource: Option<String>,
}

pub(crate) fn oauth_origin(state: &AppState) -> Option<String> {
    state.owner_auth.as_ref().map(|owner| owner.public_origin())
}

pub(crate) fn oauth_resource(state: &AppState) -> Option<String> {
    oauth_origin(state).map(|origin| format!("{origin}/mcp"))
}

pub(crate) fn oauth_signing_secret(state: &AppState) -> Option<String> {
    state
        .admin_token
        .as_deref()
        .map(str::to_owned)
        .or_else(|| browser_session_secret(state))
}

pub(crate) fn oauth_json_error(status: StatusCode, code: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": code,
            "error_description": description,
        })),
    )
        .into_response()
}

pub(crate) async fn oauth_protected_resource_metadata(State(state): State<AppState>) -> Response {
    let Some(origin) = oauth_origin(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth is not configured").into_response();
    };
    let resource = format!("{origin}/mcp");
    Json(serde_json::json!({
        "resource": resource,
        "authorization_servers": [origin.clone()],
        "scopes_supported": [oauth::CONTROL_SCOPE, oauth::OFFLINE_SCOPE],
        "resource_documentation": origin,
    }))
    .into_response()
}

pub(crate) async fn oauth_authorization_server_metadata(State(state): State<AppState>) -> Response {
    let Some(origin) = oauth_origin(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth is not configured").into_response();
    };
    Json(serde_json::json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/oauth/authorize"),
        "token_endpoint": format!("{origin}/oauth/token"),
        "registration_endpoint": format!("{origin}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [oauth::CONTROL_SCOPE, oauth::OFFLINE_SCOPE],
        "authorization_response_iss_parameter_supported": true,
    }))
    .into_response()
}

pub(crate) async fn oauth_register(
    State(state): State<AppState>,
    Json(input): Json<oauth::RegistrationRequest>,
) -> Response {
    let Some(secret) = oauth_signing_secret(&state) else {
        return oauth_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "OAuth is not configured",
        );
    };
    match oauth::register_client(secret.as_bytes(), input) {
        Ok(registration) => (StatusCode::CREATED, Json(registration)).into_response(),
        Err(error) => oauth_json_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", error),
    }
}

pub(crate) fn validate_oauth_authorize_request(
    state: &AppState,
    query: &OAuthAuthorizeQuery,
) -> Result<(String, String), &'static str> {
    if query.response_type != "code" {
        return Err("response_type must be code");
    }
    if query.code_challenge_method != "S256" || query.code_challenge.len() != 43 {
        return Err("PKCE S256 is required");
    }
    let secret = oauth_signing_secret(state).ok_or("OAuth is not configured")?;
    let registration = oauth::validate_client(secret.as_bytes(), &query.client_id)
        .ok_or("unknown or expired client_id")?;
    if !registration
        .redirect_uris
        .iter()
        .any(|uri| uri == &query.redirect_uri)
    {
        return Err("redirect_uri is not registered for this client");
    }
    let resource = query
        .resource
        .clone()
        .unwrap_or_else(|| oauth_resource(state).unwrap_or_default());
    if Some(resource.as_str()) != oauth_resource(state).as_deref() {
        return Err("resource does not match this MCP server");
    }
    let scope = oauth::normalize_scope(query.scope.as_deref())?;
    Ok((scope, resource))
}

pub(crate) fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn oauth_authorize_page(
    query: &OAuthAuthorizeQuery,
    scope: &str,
    resource: &str,
    origin: &str,
    signed_in: bool,
) -> Response {
    let hidden = |name: &str, value: &str| {
        format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            html_escape(name),
            html_escape(value)
        )
    };
    let mut fields = String::new();
    fields.push_str(&hidden("response_type", &query.response_type));
    fields.push_str(&hidden("client_id", &query.client_id));
    fields.push_str(&hidden("redirect_uri", &query.redirect_uri));
    fields.push_str(&hidden("code_challenge", &query.code_challenge));
    fields.push_str(&hidden(
        "code_challenge_method",
        &query.code_challenge_method,
    ));
    fields.push_str(&hidden("scope", scope));
    fields.push_str(&hidden("resource", resource));
    if let Some(state) = &query.state {
        fields.push_str(&hidden("state", state));
    }
    let credentials = if signed_in {
        "<p class=\"signed\">Signed in to Infra. Continue to authorize ChatGPT.</p>".to_string()
    } else {
        "<label>Username<input name=\"username\" autocomplete=\"username\" required></label><label>Password<input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>".to_string()
    };
    let body = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Authorize ChatGPT · Infra</title>
<style>body{{margin:0;background:#090a0c;color:#f5f6f7;font:15px/1.45 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;display:grid;place-items:center;min-height:100vh}}main{{width:min(430px,calc(100vw - 40px));background:#111318;border:1px solid #272a31;border-radius:20px;padding:28px;box-shadow:0 24px 80px #0008}}.mark{{width:44px;height:44px;border-radius:12px;background:#f4f4f4 url('/assets/logo.png') center/34px 34px no-repeat;margin-bottom:18px}}h1{{font-size:24px;margin:0 0 8px}}p{{color:#a9adb7;margin:0 0 20px}}.scope{{padding:13px 14px;background:#0c0e12;border:1px solid #24272d;border-radius:12px;margin:18px 0;color:#dfe2e7}}label{{display:block;color:#b8bcc5;margin:14px 0 6px}}input{{display:block;box-sizing:border-box;width:100%;margin-top:7px;background:#0b0d10;color:white;border:1px solid #30343b;border-radius:10px;padding:12px 13px;font:inherit;outline:none}}input:focus{{border-color:#747b89}}button{{width:100%;margin-top:18px;padding:13px 16px;border:0;border-radius:11px;background:white;color:#090a0c;font-weight:700;font:inherit;cursor:pointer}}small{{display:block;margin-top:16px;color:#777d89}}.signed{{padding:12px 13px;border-radius:10px;background:#101a14;color:#bfe5c9;border:1px solid #24452d}}</style></head>
<body><main><div class="mark"></div><h1>Authorize ChatGPT</h1><p>ChatGPT wants permission to control your Infra-connected devices through MCP.</p><div class="scope"><strong>Requested access</strong><br>{scope}</div><form method="post" action="{origin}/oauth/authorize">{fields}{credentials}<button type="submit">Authorize ChatGPT</button></form><small>Only authorize this connection if you initiated it from ChatGPT.</small></main></body></html>"#,
        scope = html_escape(scope),
        fields = fields,
        credentials = credentials,
        origin = html_escape(origin)
    );
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let csp = String::from(
        "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src https://static.cloudflareinsights.com; connect-src https://cloudflareinsights.com; form-action https: http://127.0.0.1:* http://localhost:* http://[::1]:*; base-uri 'none'; frame-ancestors 'none'",
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&csp) {
        response
            .headers_mut()
            .insert(header::CONTENT_SECURITY_POLICY, value);
    }
    response
}

pub(crate) async fn oauth_authorize_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OAuthAuthorizeQuery>,
) -> Response {
    let (scope, resource) = match validate_oauth_authorize_request(&state, &query) {
        Ok(value) => value,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let signed_in = browser_session_expiry(&state, &headers).is_some();
    let Some(origin) = oauth_origin(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth is not configured").into_response();
    };
    oauth_authorize_page(&query, &scope, &resource, &origin, signed_in)
}

pub(crate) async fn oauth_authorize_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(input): Form<OAuthAuthorizeForm>,
) -> Response {
    let query = OAuthAuthorizeQuery {
        response_type: input.response_type,
        client_id: input.client_id,
        redirect_uri: input.redirect_uri,
        code_challenge: input.code_challenge,
        code_challenge_method: input.code_challenge_method,
        scope: Some(input.scope),
        state: input.state,
        resource: Some(input.resource),
    };
    let (scope, resource) = match validate_oauth_authorize_request(&state, &query) {
        Ok(value) => value,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };

    if browser_session_expiry(&state, &headers).is_none() {
        let Some(owner) = &state.owner_auth else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Owner login is not configured",
            )
                .into_response();
        };
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
    }

    let code = match state
        .oauth
        .issue_code(oauth::AuthorizationCode {
            client_id: query.client_id.clone(),
            redirect_uri: query.redirect_uri.clone(),
            code_challenge: query.code_challenge.clone(),
            scope,
            resource,
            expires_at: oauth::code_expiry(),
        })
        .await
    {
        Ok(code) => code,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let Some(issuer) = oauth_origin(&state) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(mut redirect) = url::Url::parse(&query.redirect_uri) else {
        return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
    };
    {
        let mut pairs = redirect.query_pairs_mut();
        pairs.append_pair("code", &code);
        if let Some(state_value) = &query.state {
            pairs.append_pair("state", state_value);
        }
        pairs.append_pair("iss", &issuer);
    }
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, redirect.to_string())],
    )
        .into_response()
}

pub(crate) async fn oauth_token(
    State(state): State<AppState>,
    Form(input): Form<OAuthTokenForm>,
) -> Response {
    let Some(secret) = oauth_signing_secret(&state) else {
        return oauth_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "OAuth is not configured",
        );
    };
    if oauth::validate_client(secret.as_bytes(), &input.client_id).is_none() {
        return oauth_json_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "Unknown or expired client_id",
        );
    }
    let Some(expected_resource) = oauth_resource(&state) else {
        return oauth_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "OAuth is not configured",
        );
    };
    if let Some(resource) = &input.resource
        && resource != &expected_resource
    {
        return oauth_json_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "Resource does not match this MCP server",
        );
    }

    match input.grant_type.as_str() {
        "authorization_code" => {
            let Some(code_value) = input.code.as_deref() else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code is required",
                );
            };
            let Some(code) = state.oauth.take_code(code_value).await else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "Authorization code is invalid or expired",
                );
            };
            if code.client_id != input.client_id
                || input.redirect_uri.as_deref() != Some(code.redirect_uri.as_str())
                || code.resource != expected_resource
            {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "Authorization code binding mismatch",
                );
            }
            let Some(verifier) = input.code_verifier.as_deref() else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code_verifier is required",
                );
            };
            if !oauth::verify_pkce(verifier, &code.code_challenge) {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "PKCE verification failed",
                );
            }
            match oauth::issue_tokens(
                secret.as_bytes(),
                &input.client_id,
                &code.scope,
                &expected_resource,
            ) {
                Ok(tokens) => Json(tokens).into_response(),
                Err(error) => {
                    oauth_json_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", error)
                }
            }
        }
        "refresh_token" => {
            let Some(refresh_token) = input.refresh_token.as_deref() else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required",
                );
            };
            match oauth::refresh_tokens(
                secret.as_bytes(),
                refresh_token,
                &input.client_id,
                &expected_resource,
            ) {
                Ok(tokens) => Json(tokens).into_response(),
                Err(error) => oauth_json_error(StatusCode::BAD_REQUEST, "invalid_grant", error),
            }
        }
        _ => oauth_json_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "Only authorization_code and refresh_token are supported",
        ),
    }
}
