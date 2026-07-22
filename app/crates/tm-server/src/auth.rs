use std::{
    env, fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::http::{HeaderMap, header::AUTHORIZATION};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const AUTH_TOKEN_HASH_ENV: &str = "TM_AUTH_TOKEN_SHA256";
pub const AUTH_TOKEN_EXPIRY_ENV: &str = "TM_AUTH_TOKEN_EXPIRES_AT";
pub const TOKEN_PREFIX: &str = "tm_pat_v1_";
pub(crate) const DEVICE_TOKEN_PREFIX: &str = "tm_dev_v1_";

const TOKEN_SECRET_LENGTH: usize = 43;
const FAILED_ATTEMPT_LIMIT: u32 = 20;
const AUTHENTICATED_REQUEST_LIMIT: u32 = 120;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq, Eq)]
pub struct AuthConfig {
    expected_hash: [u8; 32],
    expires_at: DateTime<Utc>,
}

impl AuthConfig {
    pub fn from_env() -> Result<Self, String> {
        let expected_hash = required_env(AUTH_TOKEN_HASH_ENV)?;
        let expires_at = required_env(AUTH_TOKEN_EXPIRY_ENV)?;
        Self::from_values(&expected_hash, &expires_at)
    }

    pub fn from_values(expected_hash: &str, expires_at: &str) -> Result<Self, String> {
        let expected_hash = decode_sha256_hex(expected_hash)?;
        let expires_at = DateTime::parse_from_rfc3339(expires_at)
            .map_err(|error| format!("{AUTH_TOKEN_EXPIRY_ENV} must be RFC 3339: {error}"))?
            .with_timezone(&Utc);
        if expires_at <= Utc::now() {
            return Err(format!("{AUTH_TOKEN_EXPIRY_ENV} must be in the future"));
        }

        Ok(Self {
            expected_hash,
            expires_at,
        })
    }

    #[must_use]
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    #[cfg(test)]
    pub(crate) fn for_test(token: &str, expires_at: DateTime<Utc>) -> Self {
        Self {
            expected_hash: hash_token(token),
            expires_at,
        }
    }
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthConfig")
            .field("token_hash", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthDecision {
    Authenticated { expires_at: DateTime<Utc> },
    AuthenticationRequired,
    TokenExpired,
    FailedAttemptRateLimited,
    RequestRateLimited,
}

#[derive(Clone)]
pub(crate) struct TokenAuthenticator {
    config: AuthConfig,
    failed_attempts: Arc<Mutex<FixedWindow>>,
    authenticated_requests: Arc<Mutex<FixedWindow>>,
    failed_attempt_limit: u32,
    authenticated_request_limit: u32,
    window: Duration,
}

impl TokenAuthenticator {
    pub(crate) fn new(config: AuthConfig) -> Self {
        Self::with_limits(
            config,
            FAILED_ATTEMPT_LIMIT,
            AUTHENTICATED_REQUEST_LIMIT,
            RATE_LIMIT_WINDOW,
        )
    }

    fn with_limits(
        config: AuthConfig,
        failed_attempt_limit: u32,
        authenticated_request_limit: u32,
        window: Duration,
    ) -> Self {
        Self {
            config,
            failed_attempts: Arc::new(Mutex::new(FixedWindow::new())),
            authenticated_requests: Arc::new(Mutex::new(FixedWindow::new())),
            failed_attempt_limit,
            authenticated_request_limit,
            window,
        }
    }

    pub(crate) fn authorize(&self, headers: &HeaderMap) -> AuthDecision {
        if Utc::now() >= self.config.expires_at {
            return AuthDecision::TokenExpired;
        }

        let authenticated = bearer_token(headers)
            .filter(|token| valid_token_shape(token))
            .is_some_and(|token| {
                constant_time_equal(&hash_token(token), &self.config.expected_hash)
            });

        if !authenticated {
            return if self.take_failed_attempt() {
                AuthDecision::AuthenticationRequired
            } else {
                AuthDecision::FailedAttemptRateLimited
            };
        }

        if !self.take_authenticated_request() {
            return AuthDecision::RequestRateLimited;
        }

        AuthDecision::Authenticated {
            expires_at: self.config.expires_at,
        }
    }

    pub(crate) fn accept_authenticated_request(&self) -> bool {
        self.take_authenticated_request()
    }

    pub(crate) fn reject_failed_attempt(&self) -> AuthDecision {
        if self.take_failed_attempt() {
            AuthDecision::AuthenticationRequired
        } else {
            AuthDecision::FailedAttemptRateLimited
        }
    }

    fn take_failed_attempt(&self) -> bool {
        take_window_slot(
            &self.failed_attempts,
            self.failed_attempt_limit,
            self.window,
        )
    }

    fn take_authenticated_request(&self) -> bool {
        take_window_slot(
            &self.authenticated_requests,
            self.authenticated_request_limit,
            self.window,
        )
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    if headers.get_all(AUTHORIZATION).iter().count() != 1 {
        return None;
    }
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.contains(' ') {
        return None;
    }
    Some(token)
}

fn valid_token_shape(token: &str) -> bool {
    let Some(secret) = token.strip_prefix(TOKEN_PREFIX) else {
        return false;
    };
    secret.len() == TOKEN_SECRET_LENGTH
        && secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn hash_token(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

pub(crate) fn sha256_hex(value: &str) -> String {
    let digest = hash_token(value);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub(crate) fn generate_secret(prefix: &str) -> String {
    let material = format!(
        "{}:{}:{}:{}",
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    format!("{prefix}{}", sha256_hex(&material))
}

pub(crate) fn generate_pairing_code() -> String {
    let material = generate_secret("pairing_");
    let digest = hash_token(&material);
    let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{number:06}")
}

pub(crate) fn valid_device_token(value: &str) -> bool {
    value
        .strip_prefix(DEVICE_TOKEN_PREFIX)
        .is_some_and(|secret| {
            secret.len() == 64 && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn decode_sha256_hex(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{AUTH_TOKEN_HASH_ENV} must be exactly 64 hexadecimal characters"
        ));
    }

    let mut decoded = [0_u8; 32];
    for (index, output) in decoded.iter_mut().enumerate() {
        let offset = index * 2;
        *output = u8::from_str_radix(&value[offset..offset + 2], 16).map_err(|_| {
            format!("{AUTH_TOKEN_HASH_ENV} must contain valid hexadecimal characters")
        })?;
    }
    Ok(decoded)
}

fn required_env(name: &str) -> Result<String, String> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Err(format!("{name} must be set")),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must contain valid Unicode")),
    }
}

#[derive(Debug)]
struct FixedWindow {
    started_at: Instant,
    count: u32,
}

impl FixedWindow {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            count: 0,
        }
    }

    fn take(&mut self, limit: u32, window: Duration) -> bool {
        if self.started_at.elapsed() >= window {
            self.started_at = Instant::now();
            self.count = 0;
        }
        if self.count >= limit {
            return false;
        }
        self.count += 1;
        true
    }
}

fn take_window_slot(window: &Mutex<FixedWindow>, limit: u32, duration: Duration) -> bool {
    let Ok(mut window) = window.lock() else {
        return false;
    };
    window.take(limit, duration)
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use chrono::{Duration as ChronoDuration, Utc};

    use super::{AuthConfig, AuthDecision, TOKEN_PREFIX, TokenAuthenticator};

    const TEST_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn token() -> String {
        format!("{TOKEN_PREFIX}{TEST_SECRET}")
    }

    fn headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("valid authorization"),
        );
        headers
    }

    #[test]
    fn matching_token_is_authenticated_without_exposing_the_hash() {
        let token = token();
        let config = AuthConfig::for_test(&token, Utc::now() + ChronoDuration::minutes(5));
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(TEST_SECRET));

        let authenticator = TokenAuthenticator::new(config);
        assert!(matches!(
            authenticator.authorize(&headers(&token)),
            AuthDecision::Authenticated { .. }
        ));
        assert_eq!(
            authenticator.authorize(&headers(&format!("{TOKEN_PREFIX}{:A<43}", "B"))),
            AuthDecision::AuthenticationRequired
        );
    }

    #[test]
    fn configuration_rejects_malformed_hashes_and_past_expiry() {
        let future = (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339();
        assert!(AuthConfig::from_values("not-a-sha256-hash", &future).is_err());

        let hash = "00".repeat(32);
        let past = (Utc::now() - ChronoDuration::minutes(5)).to_rfc3339();
        assert!(AuthConfig::from_values(&hash, &past).is_err());
    }

    #[test]
    fn invalid_attempts_and_authenticated_requests_have_separate_limits() {
        let token = token();
        let config = AuthConfig::for_test(&token, Utc::now() + ChronoDuration::minutes(5));
        let authenticator =
            TokenAuthenticator::with_limits(config, 1, 1, std::time::Duration::from_secs(60));

        assert_eq!(
            authenticator.authorize(&HeaderMap::new()),
            AuthDecision::AuthenticationRequired
        );
        assert_eq!(
            authenticator.authorize(&HeaderMap::new()),
            AuthDecision::FailedAttemptRateLimited
        );
        assert!(matches!(
            authenticator.authorize(&headers(&token)),
            AuthDecision::Authenticated { .. }
        ));
        assert_eq!(
            authenticator.authorize(&headers(&token)),
            AuthDecision::RequestRateLimited
        );
    }

    #[test]
    fn expired_token_is_rejected_before_header_processing() {
        let token = token();
        let config = AuthConfig {
            expected_hash: super::hash_token(&token),
            expires_at: Utc::now() - ChronoDuration::seconds(1),
        };
        let authenticator = TokenAuthenticator::new(config);

        assert_eq!(
            authenticator.authorize(&headers(&token)),
            AuthDecision::TokenExpired
        );
    }
}
