use std::{env, fmt, time::Duration};

use axum::{
    Extension, Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tm_core::{
    EncryptedMailAccount, EncryptedMailItem, EncryptedMailReport, MailClassification,
    MailOAuthState, MailProvider, MailSummary, StoreMailAccountInput, TmCore,
};
use url::Url;
use zeroize::Zeroizing;

use super::{
    ApiEnvelope, ApiError, AppState, AuthenticatedSession, AuthenticatedSubject, RequestId,
    expense_api::{ExpectedVersion, mutation_preconditions},
    mail_crypto::{MailCrypto, MailCryptoError},
};

pub const MAIL_ENABLED_ENV: &str = "TM_MAIL_ENABLED";
pub const GMAIL_ENABLED_ENV: &str = "TM_GMAIL_ENABLED";
pub const NAVER_ENABLED_ENV: &str = "TM_NAVER_MAIL_ENABLED";
pub const MAIL_AI_ENABLED_ENV: &str = "TM_MAIL_AI_ENABLED";
pub const MAIL_REPORTS_ENABLED_ENV: &str = "TM_MAIL_REPORTS_ENABLED";
const GOOGLE_CLIENT_ID_ENV: &str = "TM_GMAIL_CLIENT_ID";
const GOOGLE_CLIENT_SECRET_ENV: &str = "TM_GMAIL_CLIENT_SECRET";
const GOOGLE_REDIRECT_URI_ENV: &str = "TM_GMAIL_OAUTH_REDIRECT_URI";
const GOOGLE_PUBSUB_TOPIC_ENV: &str = "TM_GMAIL_PUBSUB_TOPIC";
const GOOGLE_PUBSUB_AUDIENCE_ENV: &str = "TM_GMAIL_PUBSUB_AUDIENCE";
const GOOGLE_PUBSUB_SERVICE_ACCOUNT_ENV: &str = "TM_GMAIL_PUBSUB_SERVICE_ACCOUNT";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_TOKENINFO_URL: &str = "https://oauth2.googleapis.com/tokeninfo";
const GOOGLE_GMAIL_PROFILE_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";
const GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const MAX_BODY_BYTES: usize = 64 * 1024;
#[derive(Clone, Default)]
pub struct MailConfig {
    enabled: bool,
    gmail_enabled: bool,
    naver_enabled: bool,
    ai_enabled: bool,
    reports_enabled: bool,
    google: Option<GoogleConfig>,
}

#[derive(Clone)]
struct GoogleConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: Url,
    pubsub_topic: String,
    pubsub_audience: String,
    pubsub_service_account: String,
}

impl fmt::Debug for MailConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MailConfig")
            .field("enabled", &self.enabled)
            .field("gmail_enabled", &self.gmail_enabled)
            .field("naver_enabled", &self.naver_enabled)
            .field("ai_enabled", &self.ai_enabled)
            .field("reports_enabled", &self.reports_enabled)
            .field("google_configured", &self.google.is_some())
            .finish()
    }
}

impl MailConfig {
    pub fn from_env(cloud: bool, global_ai_enabled: bool) -> Result<Self, String> {
        if !cloud {
            return Ok(Self::default());
        }
        let enabled = env_bool(MAIL_ENABLED_ENV, false)?;
        let gmail_enabled = env_bool(GMAIL_ENABLED_ENV, false)?;
        let naver_enabled = env_bool(NAVER_ENABLED_ENV, false)?;
        let ai_enabled = env_bool(MAIL_AI_ENABLED_ENV, false)?;
        let reports_enabled = env_bool(MAIL_REPORTS_ENABLED_ENV, false)?;
        if (gmail_enabled || naver_enabled || ai_enabled || reports_enabled) && !enabled {
            return Err(format!(
                "{MAIL_ENABLED_ENV}=true is required before enabling a mail subfeature"
            ));
        }
        if ai_enabled && !global_ai_enabled {
            return Err(format!(
                "{MAIL_AI_ENABLED_ENV}=true requires TM_AI_ENABLED=true"
            ));
        }
        let google = if gmail_enabled {
            let redirect_uri = Url::parse(&required_env(GOOGLE_REDIRECT_URI_ENV)?)
                .map_err(|_| format!("{GOOGLE_REDIRECT_URI_ENV} is invalid"))?;
            if redirect_uri.scheme() != "https" || redirect_uri.host_str().is_none() {
                return Err(format!(
                    "{GOOGLE_REDIRECT_URI_ENV} must be an absolute HTTPS URL"
                ));
            }
            let audience = required_env(GOOGLE_PUBSUB_AUDIENCE_ENV)?;
            let audience_url = Url::parse(&audience)
                .map_err(|_| format!("{GOOGLE_PUBSUB_AUDIENCE_ENV} is invalid"))?;
            if audience_url.scheme() != "https" || audience_url.host_str().is_none() {
                return Err(format!(
                    "{GOOGLE_PUBSUB_AUDIENCE_ENV} must be an absolute HTTPS URL"
                ));
            }
            Some(GoogleConfig {
                client_id: required_env(GOOGLE_CLIENT_ID_ENV)?,
                client_secret: required_env(GOOGLE_CLIENT_SECRET_ENV)?,
                redirect_uri,
                pubsub_topic: required_env(GOOGLE_PUBSUB_TOPIC_ENV)?,
                pubsub_audience: audience,
                pubsub_service_account: required_env(GOOGLE_PUBSUB_SERVICE_ACCOUNT_ENV)?,
            })
        } else {
            None
        };
        Ok(Self {
            enabled,
            gmail_enabled,
            naver_enabled,
            ai_enabled,
            reports_enabled,
            google,
        })
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }
    pub const fn gmail_enabled(&self) -> bool {
        self.gmail_enabled
    }
    pub const fn naver_enabled(&self) -> bool {
        self.naver_enabled
    }
    pub const fn ai_enabled(&self) -> bool {
        self.ai_enabled
    }
    pub const fn reports_enabled(&self) -> bool {
        self.reports_enabled
    }
    pub fn google_topic(&self) -> Option<&str> {
        self.google
            .as_ref()
            .map(|value| value.pubsub_topic.as_str())
    }

    pub(crate) fn google_client_id(&self) -> Option<&str> {
        self.google.as_ref().map(|value| value.client_id.as_str())
    }

    pub(crate) fn google_client_secret(&self) -> Option<&str> {
        self.google
            .as_ref()
            .map(|value| value.client_secret.as_str())
    }
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/mail/accounts", get(accounts))
        .route("/api/v1/mail/summary", get(summary))
        .route("/api/v1/mail/items", get(items))
        .route("/api/v1/mail/reports", get(reports))
        .route(
            "/api/v1/mail/accounts/gmail/oauth/start",
            post(gmail_oauth_start).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route(
            "/api/v1/mail/oauth/google/callback",
            get(gmail_oauth_callback),
        )
        .route(
            "/api/v1/mail/accounts/naver",
            post(connect_naver).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route(
            "/api/v1/mail/accounts/{id}",
            axum::routing::delete(disconnect_account),
        )
        .route(
            "/api/v1/mail/items/{id}/acknowledge",
            post(acknowledge).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/v1/mail/items/{id}/feedback",
            post(feedback).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/v1/mail/webhooks/google",
            post(google_pubsub).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
}

pub(super) fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/mail/oauth/google/callback" | "/api/v1/mail/webhooks/google"
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MailAccountDto {
    id: String,
    provider: MailProvider,
    email: String,
    display_name: Option<String>,
    status: String,
    last_success_at: Option<String>,
    next_expected_at: Option<String>,
    watch_expires_at: Option<String>,
    last_error_code: Option<String>,
    consecutive_failures: u32,
    created_at: String,
    updated_at: String,
    version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MailItemDto {
    id: String,
    account_id: String,
    provider: MailProvider,
    sender: String,
    sender_domain: Option<String>,
    subject: String,
    summary: Option<String>,
    action: Option<String>,
    deadline: Option<NaiveDate>,
    received_at: String,
    classification: MailClassification,
    importance_score: u8,
    confidence: u8,
    decision_source: String,
    decision_reason: String,
    sensitive_kind: Option<String>,
    acknowledged_at: Option<String>,
    original_url: String,
    version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MailItemPageDto {
    items: Vec<MailItemDto>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MailReportDto {
    id: String,
    report_date: NaiveDate,
    slot: String,
    summary: String,
    important_count: u32,
    review_count: u32,
    generated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MailSummaryDto {
    date: NaiveDate,
    unacknowledged_important: u32,
    review_count: u32,
    latest_report: Option<MailReportDto>,
    presentation_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NaverConnectBody {
    email: String,
    display_name: Option<String>,
    app_password: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeedbackBody {
    important: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MailItemsQuery {
    status: Option<MailClassification>,
    #[serde(rename = "accountId")]
    account_id: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DateQuery {
    date: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: String,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailProfile {
    email_address: String,
    history_id: String,
}

#[derive(Debug, Deserialize)]
struct PubSubEnvelope {
    message: PubSubMessage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PubSubMessage {
    data: String,
    message_id: String,
    publish_time: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailPushData {
    email_address: String,
    history_id: String,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenInfo {
    aud: String,
    iss: String,
    email: String,
    email_verified: String,
}

async fn accounts(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ApiEnvelope<Vec<MailAccountDto>>>, ApiError> {
    ensure_mail_enabled(&state, &request_id)?;
    let core = state.core.clone();
    let encrypted = blocking(&request_id, move || core.list_mail_accounts()).await?;
    let crypto = mail_crypto(&state, &request_id)?;
    let data = encrypted
        .into_iter()
        .map(|item| decrypt_account(item, crypto, &request_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

async fn summary(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<DateQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<MailSummaryDto>>, ApiError> {
    ensure_mail_enabled(&state, &request_id)?;
    let date = parse_date(query, &request_id)?;
    let core = state.core.clone();
    let value = blocking(&request_id, move || core.mail_summary(date)).await?;
    let data = decrypt_summary(
        value,
        mail_crypto(&state, &request_id)?,
        state.mail.reports_enabled(),
        &request_id,
    )?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

async fn items(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<MailItemsQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<MailItemPageDto>>, ApiError> {
    ensure_mail_enabled(&state, &request_id)?;
    let query = query
        .map_err(|_| input_error("MAIL_QUERY_INVALID", "mail query is invalid", &request_id))?
        .0;
    if query
        .cursor
        .as_deref()
        .is_some_and(|value| value.len() > 128)
        || query
            .account_id
            .as_deref()
            .is_some_and(|value| value.len() > 128)
    {
        return Err(input_error(
            "MAIL_QUERY_INVALID",
            "mail cursor or account ID is invalid",
            &request_id,
        ));
    }
    let core = state.core.clone();
    let encrypted = blocking(&request_id, move || {
        core.list_mail_items(
            query.status,
            query.account_id.as_deref(),
            query.cursor.as_deref(),
            query.limit.unwrap_or(50),
        )
    })
    .await?;
    let crypto = mail_crypto(&state, &request_id)?;
    let data = MailItemPageDto {
        items: encrypted
            .items
            .into_iter()
            .map(|item| decrypt_item(item, crypto, &request_id))
            .collect::<Result<Vec<_>, _>>()?,
        next_cursor: encrypted.next_cursor,
    };
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

async fn reports(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<DateQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<Vec<MailReportDto>>>, ApiError> {
    ensure_mail_enabled(&state, &request_id)?;
    let date = parse_date(query, &request_id)?;
    let core = state.core.clone();
    let encrypted = blocking(&request_id, move || core.list_mail_reports(date)).await?;
    let crypto = mail_crypto(&state, &request_id)?;
    let data = encrypted
        .into_iter()
        .map(|report| decrypt_report(report, crypto, &request_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

async fn gmail_oauth_start(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_admin(&session, &request_id)?;
    ensure_gmail_enabled(&state, &request_id)?;
    let preconditions = mutation_preconditions(
        &headers,
        "mail-gmail-oauth-start",
        ExpectedVersion::Absent,
        &request_id,
    )?;
    let request_sha = sha256_hex(b"gmail-oauth-start-v1");
    let core = state.core.clone();
    let replay_key = preconditions.idempotency_key.clone();
    let receipt_aad = format!("mail-receipt:{replay_key}");
    let existing = blocking(&request_id, move || {
        core.get_mail_mutation_receipt(&replay_key, "gmail_oauth_start", &request_sha)
    })
    .await?;
    if let Some(receipt) = existing {
        let encrypted_url = receipt
            .get("authorizationUrlCiphertext")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                unavailable(
                    "MAIL_RECEIPT_INVALID",
                    "mail mutation receipt is invalid",
                    &request_id,
                )
            })?;
        let authorization_url = mail_crypto(&state, &request_id)?
            .decrypt(encrypted_url, receipt_aad.as_bytes())
            .map_err(|error| crypto_error(error, &request_id))?;
        let data = json!({ "authorizationUrl": authorization_url, "expiresInSeconds": 600 });
        return Ok((
            StatusCode::OK,
            Json(ApiEnvelope {
                request_id: request_id.0,
                data,
            }),
        )
            .into_response());
    }
    let google = state.mail.google.as_ref().ok_or_else(|| {
        unavailable(
            "GMAIL_NOT_CONFIGURED",
            "Gmail OAuth is not configured",
            &request_id,
        )
    })?;
    let state_token = random_token(32, &request_id)?;
    let verifier = random_token(48, &request_id)?;
    let state_sha = sha256_hex(state_token.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let crypto = mail_crypto(&state, &request_id)?;
    let verifier_ciphertext = crypto
        .encrypt("oauth_verifier", state_sha.as_bytes(), &verifier)
        .map_err(|error| crypto_error(error, &request_id))?;
    let return_ciphertext = crypto
        .encrypt("oauth_return", state_sha.as_bytes(), "/mail")
        .map_err(|error| crypto_error(error, &request_id))?;
    let oauth_state = MailOAuthState {
        state_sha256: state_sha.clone(),
        pkce_verifier_ciphertext: verifier_ciphertext,
        return_uri_ciphertext: return_ciphertext,
        expires_at: (Utc::now() + ChronoDuration::minutes(10)).to_rfc3339(),
    };
    let core = state.core.clone();
    blocking(&request_id, move || {
        core.create_mail_oauth_state(oauth_state)
    })
    .await?;
    let mut url = Url::parse(GOOGLE_AUTH_URL).map_err(|_| {
        unavailable(
            "GMAIL_OAUTH_CONFIGURATION_INVALID",
            "Gmail OAuth is unavailable",
            &request_id,
        )
    })?;
    url.query_pairs_mut()
        .append_pair("client_id", &google.client_id)
        .append_pair("redirect_uri", google.redirect_uri.as_str())
        .append_pair("response_type", "code")
        .append_pair("scope", GMAIL_READONLY_SCOPE)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state_token)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("include_granted_scopes", "false");
    let data = json!({ "authorizationUrl": url.as_str(), "expiresInSeconds": 600 });
    let core = state.core.clone();
    let receipt_key = preconditions.idempotency_key;
    let receipt_data = json!({
        "authorizationUrlCiphertext": crypto
            .encrypt("receipt", receipt_aad.as_bytes(), url.as_str())
            .map_err(|error| crypto_error(error, &request_id))?,
    });
    blocking(&request_id, move || {
        core.store_mail_mutation_receipt(
            &receipt_key,
            "gmail_oauth_start",
            &sha256_hex(b"gmail-oauth-start-v1"),
            &receipt_data,
        )
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiEnvelope {
            request_id: request_id.0,
            data,
        }),
    )
        .into_response())
}

async fn gmail_oauth_callback(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<OAuthCallbackQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<Value>>, ApiError> {
    ensure_gmail_enabled(&state, &request_id)?;
    let query = query
        .map_err(|_| {
            input_error(
                "GMAIL_OAUTH_CALLBACK_INVALID",
                "Gmail OAuth callback is invalid",
                &request_id,
            )
        })?
        .0;
    if query.error.is_some() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "GMAIL_OAUTH_DENIED",
            message: "Gmail access was not approved".to_owned(),
            request_id: request_id.0,
        });
    }
    let code = query
        .code
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .ok_or_else(|| {
            input_error(
                "GMAIL_OAUTH_CODE_MISSING",
                "Gmail OAuth code is missing",
                &request_id,
            )
        })?;
    if query.state.len() > 512 {
        return Err(input_error(
            "GMAIL_OAUTH_STATE_INVALID",
            "Gmail OAuth state is invalid",
            &request_id,
        ));
    }
    let state_sha = sha256_hex(query.state.as_bytes());
    let core = state.core.clone();
    let state_record = blocking(&request_id, move || {
        core.consume_mail_oauth_state(&state_sha, Utc::now())
    })
    .await?;
    let verifier = mail_crypto(&state, &request_id)?
        .decrypt(
            &state_record.pkce_verifier_ciphertext,
            state_record.state_sha256.as_bytes(),
        )
        .map_err(|error| crypto_error(error, &request_id))?;
    let google = state.mail.google.as_ref().ok_or_else(|| {
        unavailable(
            "GMAIL_NOT_CONFIGURED",
            "Gmail OAuth is not configured",
            &request_id,
        )
    })?;
    let http = provider_client(&request_id)?;
    let token_response = http
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", google.client_id.as_str()),
            ("client_secret", google.client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", google.redirect_uri.as_str()),
        ])
        .send()
        .await
        .map_err(|_| {
            unavailable(
                "GMAIL_OAUTH_UNAVAILABLE",
                "Google OAuth could not be reached",
                &request_id,
            )
        })?;
    if !token_response.status().is_success() {
        return Err(unavailable(
            "GMAIL_OAUTH_EXCHANGE_FAILED",
            "Google rejected the OAuth exchange",
            &request_id,
        ));
    }
    let token: GoogleTokenResponse = token_response.json().await.map_err(|_| {
        unavailable(
            "GMAIL_OAUTH_RESPONSE_INVALID",
            "Google OAuth returned an invalid response",
            &request_id,
        )
    })?;
    if !has_exact_readonly_scope(token.scope.as_deref()) {
        return Err(unavailable(
            "GMAIL_SCOPE_MISMATCH",
            "Google did not grant the exact Gmail read-only scope",
            &request_id,
        ));
    }
    let access_token = Zeroizing::new(token.access_token);
    let refresh_token = Zeroizing::new(token.refresh_token.ok_or_else(|| {
        unavailable(
            "GMAIL_REFRESH_TOKEN_MISSING",
            "Google did not return an offline refresh token",
            &request_id,
        )
    })?);
    let profile_response = http
        .get(GOOGLE_GMAIL_PROFILE_URL)
        .bearer_auth(&*access_token)
        .send()
        .await
        .map_err(|_| {
            unavailable(
                "GMAIL_PROFILE_UNAVAILABLE",
                "Gmail profile could not be read",
                &request_id,
            )
        })?;
    if !profile_response.status().is_success() {
        return Err(unavailable(
            "GMAIL_PROFILE_REJECTED",
            "Gmail profile access was rejected",
            &request_id,
        ));
    }
    let profile: GmailProfile = profile_response.json().await.map_err(|_| {
        unavailable(
            "GMAIL_PROFILE_INVALID",
            "Gmail profile was invalid",
            &request_id,
        )
    })?;
    let email = normalize_email(&profile.email_address, &request_id)?;
    let crypto = mail_crypto(&state, &request_id)?;
    let blind = crypto.blind_index("account_email", &email);
    let account_aad = format!("account:{blind}");
    let input = StoreMailAccountInput {
        provider: MailProvider::Gmail,
        email_ciphertext: crypto
            .encrypt("email", account_aad.as_bytes(), &email)
            .map_err(|error| crypto_error(error, &request_id))?,
        email_blind_index: blind.clone(),
        display_name_ciphertext: None,
        credential_kind: "oauth_refresh_token".to_owned(),
        secret_ciphertext: crypto
            .encrypt("credential", account_aad.as_bytes(), &refresh_token)
            .map_err(|error| crypto_error(error, &request_id))?,
        scope: token.scope,
    };
    let core = state.core.clone();
    let account = blocking(&request_id, move || core.store_mail_account(input)).await?;
    let cursor = crypto
        .encrypt(
            "sync_cursor",
            format!("mail-sync:{}:cursor", account.id).as_bytes(),
            &profile.history_id,
        )
        .map_err(|error| crypto_error(error, &request_id))?;
    let core = state.core.clone();
    let account_id = account.id.clone();
    blocking(&request_id, move || {
        core.complete_mail_sync(
            &account_id,
            MailProvider::Gmail,
            "reconcile",
            Some(&cursor),
            None,
            None,
            &(Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
            0,
        )
    })
    .await?;
    let data = json!({ "connected": true, "provider": "gmail" });
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

async fn connect_naver(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    payload: Result<Json<NaverConnectBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_admin(&session, &request_id)?;
    ensure_naver_enabled(&state, &request_id)?;
    let preconditions = mutation_preconditions(
        &headers,
        "mail-naver-connect",
        ExpectedVersion::Absent,
        &request_id,
    )?;
    let body = payload
        .map_err(|_| {
            input_error(
                "NAVER_ACCOUNT_INVALID",
                "Naver account input is invalid",
                &request_id,
            )
        })?
        .0;
    let email = normalize_email(&body.email, &request_id)?;
    if body.app_password.len() < 8
        || body.app_password.len() > 256
        || body
            .display_name
            .as_deref()
            .is_some_and(|value| value.chars().count() > 120)
    {
        return Err(input_error(
            "NAVER_ACCOUNT_INVALID",
            "Naver account input is invalid",
            &request_id,
        ));
    }
    let app_password = Zeroizing::new(body.app_password);
    let crypto = mail_crypto(&state, &request_id)?;
    let body_sha = sha256_hex(
        format!(
            "{email}\0{}\0{}",
            body.display_name.as_deref().unwrap_or(""),
            crypto.blind_index("naver_credential", &app_password)
        )
        .as_bytes(),
    );
    let core = state.core.clone();
    let replay_key = preconditions.idempotency_key.clone();
    let operation = "naver_connect";
    if let Some(receipt) = blocking(&request_id, move || {
        core.get_mail_mutation_receipt(&replay_key, operation, &body_sha)
    })
    .await?
    {
        let account_id = receipt_reference(&receipt, "accountId", &request_id)?;
        let core = state.core.clone();
        let account = blocking(&request_id, move || core.mail_account(&account_id)).await?;
        let data = serde_json::to_value(decrypt_account(
            account,
            mail_crypto(&state, &request_id)?,
            &request_id,
        )?)
        .map_err(|_| {
            unavailable(
                "MAIL_RESPONSE_INVALID",
                "mail response could not be encoded",
                &request_id,
            )
        })?;
        return Ok((
            StatusCode::OK,
            Json(ApiEnvelope {
                request_id: request_id.0,
                data,
            }),
        )
            .into_response());
    }
    super::mail_worker::verify_naver_credentials(&email, &app_password)
        .await
        .map_err(|code| ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "NAVER_IMAP_VERIFICATION_FAILED",
            message: format!("Naver IMAP credential verification failed with {code}"),
            request_id: request_id.0.clone(),
        })?;
    let blind = crypto.blind_index("account_email", &email);
    let aad = format!("account:{blind}");
    let input = StoreMailAccountInput {
        provider: MailProvider::Naver,
        email_ciphertext: crypto
            .encrypt("email", aad.as_bytes(), &email)
            .map_err(|error| crypto_error(error, &request_id))?,
        email_blind_index: blind,
        display_name_ciphertext: body
            .display_name
            .as_deref()
            .map(|value| crypto.encrypt("display_name", aad.as_bytes(), value))
            .transpose()
            .map_err(|error| crypto_error(error, &request_id))?,
        credential_kind: "app_password".to_owned(),
        secret_ciphertext: crypto
            .encrypt("credential", aad.as_bytes(), &app_password)
            .map_err(|error| crypto_error(error, &request_id))?,
        scope: None,
    };
    let core = state.core.clone();
    let account = blocking(&request_id, move || core.store_mail_account(input)).await?;
    let account_id = account.id.clone();
    let data =
        serde_json::to_value(decrypt_account(account, crypto, &request_id)?).map_err(|_| {
            unavailable(
                "MAIL_RESPONSE_INVALID",
                "mail response could not be encoded",
                &request_id,
            )
        })?;
    let core = state.core.clone();
    let receipt_key = preconditions.idempotency_key;
    let receipt_data = json!({ "accountId": account_id });
    blocking(&request_id, move || {
        core.store_mail_mutation_receipt(&receipt_key, operation, &body_sha, &receipt_data)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiEnvelope {
            request_id: request_id.0,
            data,
        }),
    )
        .into_response())
}

async fn disconnect_account(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_admin(&session, &request_id)?;
    ensure_mail_enabled(&state, &request_id)?;
    let preconditions = mutation_preconditions(
        &headers,
        "mail-account-disconnect",
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let version = preconditions.expected_version.ok_or_else(|| {
        input_error(
            "MAIL_VERSION_REQUIRED",
            "mail account version is required",
            &request_id,
        )
    })?;
    let request_sha = sha256_hex(format!("{account_id}:{version}").as_bytes());
    let core = state.core.clone();
    let replay_key = preconditions.idempotency_key.clone();
    if let Some(receipt) = blocking(&request_id, move || {
        core.get_mail_mutation_receipt(&replay_key, "account_disconnect", &request_sha)
    })
    .await?
    {
        let replay_account_id = receipt_reference(&receipt, "accountId", &request_id)?;
        let core = state.core.clone();
        let account = blocking(&request_id, move || core.mail_account(&replay_account_id)).await?;
        let data = serde_json::to_value(decrypt_account(
            account,
            mail_crypto(&state, &request_id)?,
            &request_id,
        )?)
        .map_err(|_| {
            unavailable(
                "MAIL_RESPONSE_INVALID",
                "mail response could not be encoded",
                &request_id,
            )
        })?;
        return Ok((
            StatusCode::OK,
            Json(ApiEnvelope {
                request_id: request_id.0,
                data,
            }),
        )
            .into_response());
    }
    let core = state.core.clone();
    let receipt_key = preconditions.idempotency_key.clone();
    let mutation_sha = request_sha.clone();
    let account = blocking(&request_id, move || {
        core.disable_mail_account_with_receipt(&account_id, version, &receipt_key, &mutation_sha)
    })
    .await?;
    let data = serde_json::to_value(decrypt_account(
        account,
        mail_crypto(&state, &request_id)?,
        &request_id,
    )?)
    .map_err(|_| {
        unavailable(
            "MAIL_RESPONSE_INVALID",
            "mail response could not be encoded",
            &request_id,
        )
    })?;
    Ok((
        StatusCode::OK,
        Json(ApiEnvelope {
            request_id: request_id.0,
            data,
        }),
    )
        .into_response())
}

async fn acknowledge(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    Path(item_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_mail_enabled(&state, &request_id)?;
    let preconditions = mutation_preconditions(
        &headers,
        "mail-item-acknowledge",
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let version = preconditions.expected_version.ok_or_else(|| {
        input_error(
            "MAIL_VERSION_REQUIRED",
            "mail item version is required",
            &request_id,
        )
    })?;
    let fingerprint = format!("{item_id}:{version}");
    let request_sha = sha256_hex(fingerprint.as_bytes());
    let core = state.core.clone();
    let replay_key = preconditions.idempotency_key.clone();
    if let Some(receipt) = blocking(&request_id, move || {
        core.get_mail_mutation_receipt(&replay_key, "mail_acknowledge", &request_sha)
    })
    .await?
    {
        let replay_item_id = receipt_reference(&receipt, "itemId", &request_id)?;
        let core = state.core.clone();
        let item = blocking(&request_id, move || core.mail_item(&replay_item_id)).await?;
        let data = serde_json::to_value(decrypt_item(
            item,
            mail_crypto(&state, &request_id)?,
            &request_id,
        )?)
        .map_err(|_| {
            unavailable(
                "MAIL_RESPONSE_INVALID",
                "mail response could not be encoded",
                &request_id,
            )
        })?;
        return Ok((
            StatusCode::OK,
            Json(ApiEnvelope {
                request_id: request_id.0,
                data,
            }),
        )
            .into_response());
    }
    let core = state.core.clone();
    let receipt_key = preconditions.idempotency_key.clone();
    let mutation_sha = request_sha.clone();
    let item = blocking(&request_id, move || {
        core.acknowledge_mail_item_with_receipt(&item_id, version, &receipt_key, &mutation_sha)
    })
    .await?;
    let dto = decrypt_item(item, mail_crypto(&state, &request_id)?, &request_id)?;
    let _ = session;
    Ok((
        StatusCode::OK,
        Json(ApiEnvelope {
            request_id: request_id.0,
            data: dto,
        }),
    )
        .into_response())
}

async fn feedback(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    Path(item_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<FeedbackBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    ensure_mail_enabled(&state, &request_id)?;
    let body = payload
        .map_err(|_| {
            input_error(
                "MAIL_FEEDBACK_INVALID",
                "mail feedback is invalid",
                &request_id,
            )
        })?
        .0;
    let preconditions = mutation_preconditions(
        &headers,
        "mail-item-feedback",
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let version = preconditions.expected_version.ok_or_else(|| {
        input_error(
            "MAIL_VERSION_REQUIRED",
            "mail item version is required",
            &request_id,
        )
    })?;
    let fingerprint = format!("{item_id}:{version}:{}", body.important);
    let request_sha = sha256_hex(fingerprint.as_bytes());
    let core = state.core.clone();
    let replay_key = preconditions.idempotency_key.clone();
    if let Some(receipt) = blocking(&request_id, move || {
        core.get_mail_mutation_receipt(&replay_key, "mail_feedback", &request_sha)
    })
    .await?
    {
        let replay_item_id = receipt_reference(&receipt, "itemId", &request_id)?;
        let core = state.core.clone();
        let item = blocking(&request_id, move || core.mail_item(&replay_item_id)).await?;
        let data = serde_json::to_value(decrypt_item(
            item,
            mail_crypto(&state, &request_id)?,
            &request_id,
        )?)
        .map_err(|_| {
            unavailable(
                "MAIL_RESPONSE_INVALID",
                "mail response could not be encoded",
                &request_id,
            )
        })?;
        return Ok((
            StatusCode::OK,
            Json(ApiEnvelope {
                request_id: request_id.0,
                data,
            }),
        )
            .into_response());
    }
    let actor = session.audit_actor();
    let core = state.core.clone();
    let receipt_key = preconditions.idempotency_key.clone();
    let mutation_sha = request_sha.clone();
    let item = blocking(&request_id, move || {
        core.record_mail_feedback_with_receipt(
            &item_id,
            body.important,
            version,
            &actor,
            &receipt_key,
            &mutation_sha,
        )
    })
    .await?;
    let dto = decrypt_item(item, mail_crypto(&state, &request_id)?, &request_id)?;
    Ok((
        StatusCode::OK,
        Json(ApiEnvelope {
            request_id: request_id.0,
            data: dto,
        }),
    )
        .into_response())
}

async fn google_pubsub(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    payload: Result<Json<PubSubEnvelope>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    ensure_gmail_enabled(&state, &request_id)?;
    let google = state.mail.google.as_ref().ok_or_else(|| {
        unavailable(
            "GMAIL_NOT_CONFIGURED",
            "Gmail push is not configured",
            &request_id,
        )
    })?;
    validate_google_oidc(&headers, google, &request_id).await?;
    let envelope = payload
        .map_err(|_| {
            input_error(
                "GMAIL_PUSH_INVALID",
                "Gmail push payload is invalid",
                &request_id,
            )
        })?
        .0;
    if envelope.message.message_id.is_empty()
        || envelope.message.message_id.len() > 512
        || envelope.message.data.len() > 16 * 1024
    {
        return Err(input_error(
            "GMAIL_PUSH_INVALID",
            "Gmail push payload is invalid",
            &request_id,
        ));
    }
    let decoded = STANDARD.decode(&envelope.message.data).map_err(|_| {
        input_error(
            "GMAIL_PUSH_INVALID",
            "Gmail push payload is invalid",
            &request_id,
        )
    })?;
    let push: GmailPushData = serde_json::from_slice(&decoded).map_err(|_| {
        input_error(
            "GMAIL_PUSH_INVALID",
            "Gmail push data is invalid",
            &request_id,
        )
    })?;
    let email = normalize_email(&push.email_address, &request_id)?;
    if push.history_id.is_empty()
        || push.history_id.len() > 80
        || !push.history_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(input_error(
            "GMAIL_PUSH_INVALID",
            "Gmail history cursor is invalid",
            &request_id,
        ));
    }
    let message_sha = sha256_hex(envelope.message.message_id.as_bytes());
    let account_sha = sha256_hex(email.as_bytes());
    let core = state.core.clone();
    let inserted = blocking(&request_id, move || {
        core.record_mail_webhook(
            &message_sha,
            Some(&account_sha),
            envelope.message.publish_time.as_deref(),
        )
    })
    .await?;
    if inserted {
        let core = state.core.clone();
        blocking(&request_id, move || {
            core.wake_mail_scheduler_job("mail.gmail_reconcile", Utc::now())
        })
        .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_google_oidc(
    headers: &HeaderMap,
    google: &GoogleConfig,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() <= 8192)
        .ok_or_else(|| ApiError {
            status: StatusCode::UNAUTHORIZED,
            code: "GMAIL_PUSH_OIDC_REQUIRED",
            message: "Google OIDC authentication is required".to_owned(),
            request_id: request_id.0.clone(),
        })?;
    let http = provider_client(request_id)?;
    let response = http
        .get(GOOGLE_TOKENINFO_URL)
        .query(&[("id_token", token)])
        .send()
        .await
        .map_err(|_| {
            unavailable(
                "GMAIL_PUSH_OIDC_UNAVAILABLE",
                "Google OIDC verification is unavailable",
                request_id,
            )
        })?;
    if !response.status().is_success() {
        return Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            code: "GMAIL_PUSH_OIDC_REJECTED",
            message: "Google OIDC token was rejected".to_owned(),
            request_id: request_id.0.clone(),
        });
    }
    let claims: GoogleTokenInfo = response.json().await.map_err(|_| {
        unavailable(
            "GMAIL_PUSH_OIDC_INVALID",
            "Google OIDC verification returned invalid data",
            request_id,
        )
    })?;
    if !google_claims_match(&claims, google) {
        return Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            code: "GMAIL_PUSH_OIDC_MISMATCH",
            message: "Google OIDC claims did not match the configured push identity".to_owned(),
            request_id: request_id.0.clone(),
        });
    }
    Ok(())
}

fn has_exact_readonly_scope(scope: Option<&str>) -> bool {
    scope.map(str::split_whitespace).is_some_and(|mut scopes| {
        scopes.next() == Some(GMAIL_READONLY_SCOPE) && scopes.next().is_none()
    })
}

fn google_claims_match(claims: &GoogleTokenInfo, google: &GoogleConfig) -> bool {
    claims.aud == google.pubsub_audience
        && matches!(
            claims.iss.as_str(),
            "accounts.google.com" | "https://accounts.google.com"
        )
        && claims.email == google.pubsub_service_account
        && claims.email_verified == "true"
}

fn decrypt_account(
    item: EncryptedMailAccount,
    crypto: &MailCrypto,
    request_id: &RequestId,
) -> Result<MailAccountDto, ApiError> {
    let aad = format!("account:{}", item.email_blind_index);
    Ok(MailAccountDto {
        id: item.id,
        provider: item.provider,
        email: crypto
            .decrypt(&item.email_ciphertext, aad.as_bytes())
            .map_err(|error| crypto_error(error, request_id))?,
        display_name: item
            .display_name_ciphertext
            .as_deref()
            .map(|value| crypto.decrypt(value, aad.as_bytes()))
            .transpose()
            .map_err(|error| crypto_error(error, request_id))?,
        status: item.status,
        last_success_at: item.last_success_at,
        next_expected_at: item.next_expected_at,
        watch_expires_at: item.watch_expires_at,
        last_error_code: item.last_error_code,
        consecutive_failures: item.consecutive_failures,
        created_at: item.created_at,
        updated_at: item.updated_at,
        version: item.version,
    })
}

fn decrypt_item(
    item: EncryptedMailItem,
    crypto: &MailCrypto,
    request_id: &RequestId,
) -> Result<MailItemDto, ApiError> {
    let message_id = crypto
        .decrypt(&item.provider_message_ciphertext, b"mail-item:provider-id")
        .map_err(|error| crypto_error(error, request_id))?;
    let original_url = match item.provider {
        MailProvider::Gmail => format!("https://mail.google.com/mail/u/0/#all/{message_id}"),
        MailProvider::Naver => "https://mail.naver.com/v2/folders/0/all".to_owned(),
    };
    Ok(MailItemDto {
        id: item.id,
        account_id: item.account_id,
        provider: item.provider,
        sender: crypto
            .decrypt(&item.sender_ciphertext, b"mail-item:sender")
            .map_err(|error| crypto_error(error, request_id))?,
        sender_domain: item
            .sender_domain_ciphertext
            .as_deref()
            .map(|value| crypto.decrypt(value, b"mail-item:sender-domain"))
            .transpose()
            .map_err(|error| crypto_error(error, request_id))?,
        subject: crypto
            .decrypt(&item.subject_ciphertext, b"mail-item:subject")
            .map_err(|error| crypto_error(error, request_id))?,
        summary: item
            .summary_ciphertext
            .as_deref()
            .map(|value| crypto.decrypt(value, b"mail-item:summary"))
            .transpose()
            .map_err(|error| crypto_error(error, request_id))?,
        action: item
            .action_ciphertext
            .as_deref()
            .map(|value| crypto.decrypt(value, b"mail-item:action"))
            .transpose()
            .map_err(|error| crypto_error(error, request_id))?,
        deadline: item.deadline,
        received_at: item.received_at,
        classification: item.classification,
        importance_score: item.importance_score,
        confidence: item.confidence,
        decision_source: item.decision_source,
        decision_reason: item.decision_reason,
        sensitive_kind: item.sensitive_kind,
        acknowledged_at: item.acknowledged_at,
        original_url,
        version: item.received_version,
    })
}

fn decrypt_report(
    report: EncryptedMailReport,
    crypto: &MailCrypto,
    request_id: &RequestId,
) -> Result<MailReportDto, ApiError> {
    Ok(MailReportDto {
        id: report.id,
        report_date: report.report_date,
        slot: report.slot,
        summary: crypto
            .decrypt(&report.summary_ciphertext, b"mail-report:summary")
            .map_err(|error| crypto_error(error, request_id))?,
        important_count: report.important_count,
        review_count: report.review_count,
        generated_at: report.generated_at,
    })
}

fn decrypt_summary(
    summary: MailSummary,
    crypto: &MailCrypto,
    presentation_enabled: bool,
    request_id: &RequestId,
) -> Result<MailSummaryDto, ApiError> {
    Ok(MailSummaryDto {
        date: summary.date,
        unacknowledged_important: summary.unacknowledged_important,
        review_count: summary.review_count,
        latest_report: summary
            .latest_report
            .map(|value| decrypt_report(value, crypto, request_id))
            .transpose()?,
        presentation_enabled,
    })
}

fn receipt_reference(
    receipt: &Value,
    field: &str,
    request_id: &RequestId,
) -> Result<String, ApiError> {
    receipt
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 80)
        .map(str::to_owned)
        .ok_or_else(|| {
            unavailable(
                "MAIL_RECEIPT_INVALID",
                "mail mutation receipt is invalid",
                request_id,
            )
        })
}

fn parse_date(
    query: Result<Query<DateQuery>, QueryRejection>,
    request_id: &RequestId,
) -> Result<NaiveDate, ApiError> {
    let value = query
        .map_err(|_| input_error("MAIL_DATE_INVALID", "mail date is invalid", request_id))?
        .0
        .date;
    NaiveDate::parse_from_str(&value, "%Y-%m-%d")
        .map_err(|_| input_error("MAIL_DATE_INVALID", "mail date is invalid", request_id))
}

fn normalize_email(value: &str, request_id: &RequestId) -> Result<String, ApiError> {
    let value = value.trim().to_lowercase();
    if value.len() > 254 || !value.contains('@') || value.chars().any(char::is_whitespace) {
        return Err(input_error(
            "MAIL_EMAIL_INVALID",
            "mail address is invalid",
            request_id,
        ));
    }
    Ok(value)
}

fn random_token(bytes: usize, request_id: &RequestId) -> Result<String, ApiError> {
    let mut value = vec![0_u8; bytes];
    getrandom::fill(&mut value).map_err(|_| {
        unavailable(
            "MAIL_RANDOMNESS_UNAVAILABLE",
            "secure mail setup could not start",
            request_id,
        )
    })?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}

fn sha256_hex(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn provider_client(request_id: &RequestId) -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("tm-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| {
            unavailable(
                "MAIL_HTTP_UNAVAILABLE",
                "mail provider client could not be initialized",
                request_id,
            )
        })
}

fn mail_crypto<'a>(
    state: &'a AppState,
    request_id: &RequestId,
) -> Result<&'a MailCrypto, ApiError> {
    state
        .mail_crypto
        .as_ref()
        .map_err(|error| crypto_error(*error, request_id))
}

fn crypto_error(error: MailCryptoError, request_id: &RequestId) -> ApiError {
    tracing::error!(error_kind = ?error, request_id = %request_id.0, "mail data protection failed");
    unavailable(
        "MAIL_CRYPTO_UNAVAILABLE",
        "mail data protection is unavailable",
        request_id,
    )
}

fn ensure_mail_enabled(state: &AppState, request_id: &RequestId) -> Result<(), ApiError> {
    if state.mail.enabled() {
        Ok(())
    } else {
        Err(unavailable(
            "MAIL_DISABLED",
            "mail monitoring is not enabled",
            request_id,
        ))
    }
}

fn ensure_gmail_enabled(state: &AppState, request_id: &RequestId) -> Result<(), ApiError> {
    ensure_mail_enabled(state, request_id)?;
    if state.mail.gmail_enabled() {
        Ok(())
    } else {
        Err(unavailable(
            "GMAIL_DISABLED",
            "Gmail monitoring is not enabled",
            request_id,
        ))
    }
}

fn ensure_naver_enabled(state: &AppState, request_id: &RequestId) -> Result<(), ApiError> {
    ensure_mail_enabled(state, request_id)?;
    if state.mail.naver_enabled() {
        Ok(())
    } else {
        Err(unavailable(
            "NAVER_MAIL_DISABLED",
            "Naver mail monitoring is not enabled",
            request_id,
        ))
    }
}

fn require_admin(session: &AuthenticatedSession, request_id: &RequestId) -> Result<(), ApiError> {
    if matches!(session.subject, AuthenticatedSubject::PrimaryAdmin) {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "MAIL_PRIMARY_ADMIN_REQUIRED",
            message: "the primary administrator credential is required".to_owned(),
            request_id: request_id.0.clone(),
        })
    }
}

fn input_error(code: &'static str, message: &'static str, request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        code,
        message: message.to_owned(),
        request_id: request_id.0.clone(),
    }
}

fn unavailable(code: &'static str, message: &'static str, request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code,
        message: message.to_owned(),
        request_id: request_id.0.clone(),
    }
}

async fn blocking<T: Send + 'static>(
    request_id: &RequestId,
    operation: impl FnOnce() -> tm_core::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| {
            unavailable(
                "MAIL_WORKER_FAILED",
                "mail storage worker failed",
                request_id,
            )
        })?
        .map_err(|error| {
            let (status, code) = match error {
                tm_core::Error::Conflict(_) => (StatusCode::CONFLICT, "MAIL_CONFLICT"),
                tm_core::Error::InvalidInput(_) => (StatusCode::BAD_REQUEST, "MAIL_INPUT_INVALID"),
                tm_core::Error::NotFound { .. } => (StatusCode::NOT_FOUND, "MAIL_NOT_FOUND"),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "MAIL_STORAGE_FAILED"),
            };
            ApiError {
                status,
                code,
                message: "mail data operation failed".to_owned(),
                request_id: request_id.0.clone(),
            }
        })
}

fn required_env(name: &str) -> Result<String, String> {
    let value = env::var(name).map_err(|_| format!("{name} is required"))?;
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(format!("{name} is invalid"));
    }
    Ok(value)
}

fn env_bool(name: &str, default: bool) -> Result<bool, String> {
    match env::var(name) {
        Ok(value) if value == "true" => Ok(true),
        Ok(value) if value == "false" => Ok(false),
        Ok(_) => Err(format!("{name} must be true or false")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(_) => Err(format!("{name} is invalid")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google_config() -> GoogleConfig {
        GoogleConfig {
            client_id: "client".to_owned(),
            client_secret: "secret".to_owned(),
            redirect_uri: Url::parse("https://tm.example/api/v1/mail/oauth/google/callback")
                .expect("test redirect URI is valid"),
            pubsub_topic: "projects/tm/topics/gmail".to_owned(),
            pubsub_audience: "https://tm.example/api/v1/mail/webhooks/google".to_owned(),
            pubsub_service_account: "gmail-push@tm.example".to_owned(),
        }
    }

    fn claims() -> GoogleTokenInfo {
        GoogleTokenInfo {
            aud: "https://tm.example/api/v1/mail/webhooks/google".to_owned(),
            iss: "https://accounts.google.com".to_owned(),
            email: "gmail-push@tm.example".to_owned(),
            email_verified: "true".to_owned(),
        }
    }

    #[test]
    fn gmail_oauth_accepts_only_the_exact_readonly_scope() {
        assert!(has_exact_readonly_scope(Some(GMAIL_READONLY_SCOPE)));
        assert!(!has_exact_readonly_scope(None));
        assert!(!has_exact_readonly_scope(Some("https://mail.google.com/")));
        assert!(!has_exact_readonly_scope(Some(&format!(
            "{GMAIL_READONLY_SCOPE} https://www.googleapis.com/auth/gmail.modify"
        ))));
    }

    #[test]
    fn pubsub_oidc_claims_are_bound_to_audience_and_service_account() {
        let config = google_config();
        assert!(google_claims_match(&claims(), &config));

        let mut wrong_audience = claims();
        wrong_audience.aud = "https://attacker.example".to_owned();
        assert!(!google_claims_match(&wrong_audience, &config));

        let mut wrong_identity = claims();
        wrong_identity.email = "other@tm.example".to_owned();
        assert!(!google_claims_match(&wrong_identity, &config));

        let mut unverified = claims();
        unverified.email_verified = "false".to_owned();
        assert!(!google_claims_match(&unverified, &config));
    }
}
