use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct CreateEnrollmentInput {
    #[serde(default)]
    ttl_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateEnrollmentOutput {
    enrollment_id: String,
    code: String,
    expires_at_unix_ms: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnrollInput {
    code: String,
    request_id: String,
    device_name: String,
    public_key: String,
    platform: String,
    arch: String,
    agent_version: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct EnrollOutput {
    device_id: String,
    device_name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PairRequestInput {
    request_id: String,
    device_name: String,
    public_key: String,
    platform: String,
    arch: String,
    agent_version: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PairStatusInput {
    request_id: String,
    public_key: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApprovePairInput {
    verification_code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RenameDeviceInput {
    name: Option<String>,
}

pub(crate) async fn create_enrollment(
    State(state): State<AppState>,
    Json(input): Json<CreateEnrollmentInput>,
) -> Response {
    let ttl_ms = input.ttl_seconds.unwrap_or(300).clamp(30, 900) * 1000;
    match state.store.create_enrollment(ttl_ms) {
        Ok(created) => {
            let _ = state.store.record_activity(
                None,
                "enrollment.create",
                "ok",
                Some(&created.enrollment_id),
            );
            state.emit("enrollment_created", None);
            Json(CreateEnrollmentOutput {
                enrollment_id: created.enrollment_id,
                code: created.code,
                expires_at_unix_ms: created.expires_at_unix_ms,
            })
            .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn enroll(
    State(state): State<AppState>,
    Json(input): Json<EnrollInput>,
) -> Response {
    if Uuid::parse_str(&input.request_id).is_err()
        || input.device_name.trim().is_empty()
        || input.device_name.len() > 120
    {
        return (StatusCode::BAD_REQUEST, "invalid enrollment request").into_response();
    }
    let key = match URL_SAFE_NO_PAD
        .decode(&input.public_key)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
    {
        Some(k) => k,
        None => return (StatusCode::BAD_REQUEST, "invalid public key").into_response(),
    };
    match state.store.consume_enrollment(
        &input.code,
        &input.request_id,
        input.device_name.trim(),
        &key,
        &input.platform,
        &input.arch,
        &input.agent_version,
    ) {
        Ok(result) => {
            let device_id = result.device_id.clone();
            let _ = state.store.record_activity(
                Some(&device_id),
                "enrollment.consume",
                "ok",
                Some(&input.request_id),
            );
            state.emit("device_enrolled", Some(device_id));
            Json(EnrollOutput {
                device_id: result.device_id,
                device_name: result.device_name,
            })
            .into_response()
        }
        Err(StoreError::EnrollmentInvalid) => {
            (StatusCode::UNAUTHORIZED, "invalid enrollment code").into_response()
        }
        Err(StoreError::EnrollmentExpired) => {
            (StatusCode::GONE, "enrollment code expired").into_response()
        }
        Err(StoreError::EnrollmentConsumed) => {
            (StatusCode::CONFLICT, "enrollment code already used").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// Proof of possession for an untrusted candidate key. Approval grants trust.
pub(crate) fn verify_pair_signature(
    key: &str,
    signature: &str,
    message: &[u8],
) -> Option<[u8; 32]> {
    let bytes = URL_SAFE_NO_PAD.decode(key).ok()?;
    let key_bytes: [u8; 32] = bytes.try_into().ok()?;
    let verifying = VerifyingKey::from_bytes(&key_bytes).ok()?;
    let sig: [u8; 64] = URL_SAFE_NO_PAD.decode(signature).ok()?.try_into().ok()?;
    verifying
        .verify_strict(message, &Signature::from_bytes(&sig))
        .ok()?;
    Some(key_bytes)
}

pub(crate) fn pair_field_valid(field: &str, maximum: usize) -> bool {
    !field.trim().is_empty() && field.len() <= maximum && !field.contains(['\r', '\n', '\0'])
}

pub(crate) fn pair_request_message(input: &PairRequestInput) -> String {
    format!(
        "infra-pair-request-v1\n{}\n{}\n{}\n{}\n{}\n{}",
        input.request_id,
        input.public_key,
        input.device_name,
        input.platform,
        input.arch,
        input.agent_version,
    )
}

pub(crate) async fn request_pair(
    State(state): State<AppState>,
    Json(input): Json<PairRequestInput>,
) -> Response {
    if Uuid::parse_str(&input.request_id).is_err()
        || !pair_field_valid(&input.device_name, 120)
        || !pair_field_valid(&input.platform, 32)
        || !pair_field_valid(&input.arch, 32)
        || !pair_field_valid(&input.agent_version, 64)
    {
        return (StatusCode::BAD_REQUEST, "invalid pairing request").into_response();
    }
    let Some(key) = verify_pair_signature(
        &input.public_key,
        &input.signature,
        pair_request_message(&input).as_bytes(),
    ) else {
        return (StatusCode::UNAUTHORIZED, "invalid pairing key proof").into_response();
    };
    match state.store.request_pair(
        &input.request_id,
        &key,
        &input.device_name,
        &input.platform,
        &input.arch,
        &input.agent_version,
    ) {
        Ok(record) => {
            state.emit("pair_requested", None);
            Json(record).into_response()
        }
        Err(StoreError::PairCapacity) => (
            StatusCode::TOO_MANY_REQUESTS,
            "pairing is busy; try again later",
        )
            .into_response(),
        Err(StoreError::PairConflict) => {
            (StatusCode::CONFLICT, "pairing identity conflict").into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn pair_status(
    State(state): State<AppState>,
    Json(input): Json<PairStatusInput>,
) -> Response {
    if Uuid::parse_str(&input.request_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let message = format!("infra-pair-status-v1\n{}", input.request_id);
    let Some(key) = verify_pair_signature(&input.public_key, &input.signature, message.as_bytes())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match state.store.pair_status(&input.request_id, &key) {
        Ok(record) => Json(record).into_response(),
        Err(StoreError::PairInvalid) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn list_pending_pairs(State(state): State<AppState>) -> Response {
    match state.store.pending_pairs() {
        Ok(records) => Json(records).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn owner_pair_status(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Response {
    if Uuid::parse_str(&request_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut response = match state.store.owner_pair_status(&request_id) {
        Ok(record) => Json(record).into_response(),
        Err(StoreError::PairInvalid) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

pub(crate) async fn approve_pair(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Json(input): Json<ApprovePairInput>,
) -> Response {
    if Uuid::parse_str(&request_id).is_err()
        || input.verification_code.len() != 6
        || !input.verification_code.bytes().all(|b| b.is_ascii_digit())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match state
        .store
        .approve_pair(&request_id, &input.verification_code)
    {
        Ok(record) => {
            let _ = state.store.record_activity(
                record.device_id.as_deref(),
                "pair.approve",
                "ok",
                Some(&request_id),
            );
            state.emit("pair_approved", record.device_id.clone());
            Json(record).into_response()
        }
        Err(StoreError::PairInvalid) => StatusCode::NOT_FOUND.into_response(),
        Err(StoreError::PairMismatch) => StatusCode::FORBIDDEN.into_response(),
        Err(StoreError::PairExpired) => StatusCode::GONE.into_response(),
        Err(StoreError::PairConflict) => StatusCode::CONFLICT.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn reject_pair(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Response {
    if Uuid::parse_str(&request_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match state.store.reject_pair(&request_id) {
        Ok(true) => {
            state.emit("pair_rejected", None);
            Json(serde_json::json!({"ok":true})).into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn rename_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(input): Json<RenameDeviceInput>,
) -> Response {
    match state
        .store
        .set_device_alias(&device_id, input.name.as_deref())
    {
        Ok(true) => {
            state.emit("device_renamed", Some(device_id.clone()));
            Json(serde_json::json!({"ok":true,"device_id":device_id})).into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(StoreError::PairInvalid) => {
            (StatusCode::BAD_REQUEST, "invalid display name").into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn revoke_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Response {
    match state.store.revoke(&device_id) {
        Ok(true) => {
            state.devices.write().await.remove(&device_id);
            let _ = state
                .store
                .record_activity(Some(&device_id), "device.revoke", "ok", None);
            state.emit("device_revoked", Some(device_id.clone()));
            Json(serde_json::json!({"ok":true,"device_id":device_id})).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "device not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
