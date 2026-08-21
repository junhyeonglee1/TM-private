use std::ops::Range;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AiTokenUsage, Error, Result, TmCore,
    database::{new_id, now_utc},
    error::invalid,
};

pub const MAIL_TRIAGE_OPERATION: &str = "mail_triage";
pub const MAIL_TRIAGE_MONTHLY_HARD_LIMIT_MICROUSD: u64 = 1_000_000;
pub const MAIL_TRIAGE_MAXIMUM_COST_MICROUSD: u64 = 10_000;
pub const MAIL_TRIAGE_MONTHLY_ATTEMPT_LIMIT: u32 = 100;
pub const MAIL_RETENTION_DAYS: i64 = 90;
pub const MAIL_REPORT_RETENTION_DAYS: i64 = 366;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailProvider {
    Gmail,
    Naver,
}

impl MailProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gmail => "gmail",
            Self::Naver => "naver",
        }
    }
}

impl TryFrom<&str> for MailProvider {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "gmail" => Ok(Self::Gmail),
            "naver" => Ok(Self::Naver),
            _ => Err(invalid("unsupported mail provider")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailClassification {
    Important,
    Review,
    NotImportant,
    Excluded,
}

impl MailClassification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Important => "important",
            Self::Review => "review",
            Self::NotImportant => "not_important",
            Self::Excluded => "excluded",
        }
    }
}

impl TryFrom<&str> for MailClassification {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "important" => Ok(Self::Important),
            "review" => Ok(Self::Review),
            "not_important" => Ok(Self::NotImportant),
            "excluded" => Ok(Self::Excluded),
            _ => Err(Error::Invariant(
                "invalid persisted mail classification".to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedMailAccount {
    pub id: String,
    pub provider: MailProvider,
    pub email_ciphertext: String,
    pub email_blind_index: String,
    pub display_name_ciphertext: Option<String>,
    pub status: String,
    pub last_success_at: Option<String>,
    pub next_expected_at: Option<String>,
    pub watch_expires_at: Option<String>,
    pub last_error_code: Option<String>,
    pub consecutive_failures: u32,
    pub created_at: String,
    pub updated_at: String,
    pub version: u64,
}

#[derive(Debug, Clone)]
pub struct StoreMailAccountInput {
    pub provider: MailProvider,
    pub email_ciphertext: String,
    pub email_blind_index: String,
    pub display_name_ciphertext: Option<String>,
    pub credential_kind: String,
    pub secret_ciphertext: String,
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EncryptedMailCredential {
    pub account_id: String,
    pub credential_kind: String,
    pub secret_ciphertext: String,
    pub scope: Option<String>,
    pub token_expires_at: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone)]
pub struct MailSyncAccount {
    pub account: EncryptedMailAccount,
    pub credential: EncryptedMailCredential,
    pub cursor_ciphertext: Option<String>,
    pub uid_validity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MailOAuthState {
    pub state_sha256: String,
    pub pkce_verifier_ciphertext: String,
    pub return_uri_ciphertext: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedMailItem {
    pub id: String,
    pub account_id: String,
    pub provider: MailProvider,
    pub provider_message_ciphertext: String,
    pub received_at: String,
    pub sender_ciphertext: String,
    pub sender_domain_ciphertext: Option<String>,
    pub subject_ciphertext: String,
    pub summary_ciphertext: Option<String>,
    pub action_ciphertext: Option<String>,
    pub deadline: Option<NaiveDate>,
    pub classification: MailClassification,
    pub importance_score: u8,
    pub confidence: u8,
    pub decision_source: String,
    pub decision_reason: String,
    pub sensitive_kind: Option<String>,
    pub acknowledged_at: Option<String>,
    pub received_version: u64,
}

#[derive(Debug, Clone)]
pub struct StoreMailItemInput {
    pub account_id: String,
    pub provider_message_sha256: String,
    pub provider_message_ciphertext: String,
    pub thread_sha256: Option<String>,
    pub received_at: DateTime<Utc>,
    pub sender_ciphertext: String,
    pub sender_domain_ciphertext: Option<String>,
    pub sender_domain_blind_index: Option<String>,
    pub subject_ciphertext: String,
    pub summary_ciphertext: Option<String>,
    pub action_ciphertext: Option<String>,
    pub deadline: Option<NaiveDate>,
    pub classification: MailClassification,
    pub importance_score: u8,
    pub confidence: u8,
    pub decision_source: String,
    pub decision_reason: String,
    pub sensitive_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailItemPage {
    pub items: Vec<EncryptedMailItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedMailReport {
    pub id: String,
    pub report_date: NaiveDate,
    pub slot: String,
    pub summary_ciphertext: String,
    pub important_count: u32,
    pub review_count: u32,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailSummary {
    pub date: NaiveDate,
    pub unacknowledged_important: u32,
    pub review_count: u32,
    pub latest_report: Option<EncryptedMailReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailOpsStatus {
    pub account_count: u32,
    pub connected_count: u32,
    pub reconnect_required_count: u32,
    pub unacknowledged_important_count: u32,
    pub review_count: u32,
    pub latest_success_at: Option<String>,
    pub last_error_code: Option<String>,
    pub queue_depth: u32,
    pub retry_count: u32,
    pub dead_letter_count: u32,
    pub accounts: Vec<MailAccountOpsStatus>,
    pub triage_claimed_count: u32,
    pub latest_triage_status: Option<String>,
    pub latest_triage_failure_code: Option<String>,
    pub latest_triage_completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAccountOpsStatus {
    pub account_id: String,
    pub provider: MailProvider,
    pub status: String,
    pub last_success_at: Option<String>,
    pub next_expected_at: Option<String>,
    pub watch_expires_at: Option<String>,
    pub last_error_code: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone)]
pub struct MailTriageDecision {
    pub item_id: String,
    pub expected_version: u64,
    pub importance_score: u8,
    pub confidence: u8,
    pub classification: MailClassification,
    pub reason_code: String,
    pub action_ciphertext: Option<String>,
}

impl TmCore {
    pub fn mail_crypto_probe(&self) -> Result<Option<String>> {
        self.database
            .connect()?
            .query_row(
                "SELECT probe_ciphertext FROM mail_crypto_metadata
                 WHERE singleton_key = 'mail-data-key-probe' AND key_version = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn initialize_mail_crypto_probe(&self, probe_ciphertext: &str) -> Result<String> {
        if probe_ciphertext.is_empty() || probe_ciphertext.len() > 4096 {
            return Err(invalid("mail crypto probe is invalid"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let existing: Option<String> = transaction
                    .query_row(
                        "SELECT probe_ciphertext FROM mail_crypto_metadata
                         WHERE singleton_key = 'mail-data-key-probe'",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(existing) = existing {
                    return Ok(existing);
                }
                let now = now_utc();
                transaction.execute(
                    "INSERT INTO mail_crypto_metadata(
                        singleton_key, key_version, probe_ciphertext, created_at, updated_at
                     ) VALUES ('mail-data-key-probe', 1, ?1, ?2, ?2)",
                    params![probe_ciphertext, now],
                )?;
                Ok(probe_ciphertext.to_owned())
            })
    }

    pub fn store_mail_account(&self, input: StoreMailAccountInput) -> Result<EncryptedMailAccount> {
        validate_sha256(&input.email_blind_index, "mail email blind index")?;
        if !matches!(
            input.credential_kind.as_str(),
            "oauth_refresh_token" | "app_password"
        ) {
            return Err(invalid("mail credential kind is invalid"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                let existing: Option<String> = transaction.query_row(
                "SELECT id FROM mail_accounts WHERE provider = ?1 AND email_blind_index = ?2",
                params![input.provider.as_str(), input.email_blind_index],
                |row| row.get(0),
            ).optional()?;
                let account_id = existing.unwrap_or_else(new_id);
                transaction.execute(
                    "INSERT INTO mail_accounts(
                    id, provider, email_ciphertext, email_blind_index, display_name_ciphertext,
                    status, created_at, updated_at, disconnected_at, version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'connected', ?6, ?6, NULL, 1)
                 ON CONFLICT(id) DO UPDATE SET
                    email_ciphertext = excluded.email_ciphertext,
                    display_name_ciphertext = excluded.display_name_ciphertext,
                    status = 'connected', disconnected_at = NULL,
                    updated_at = excluded.updated_at, version = mail_accounts.version + 1",
                    params![
                        account_id,
                        input.provider.as_str(),
                        input.email_ciphertext,
                        input.email_blind_index,
                        input.display_name_ciphertext,
                        now,
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO mail_credentials(
                    account_id, credential_kind, secret_ciphertext, scope,
                    token_expires_at, created_at, updated_at, version
                 ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?5, 1)
                 ON CONFLICT(account_id) DO UPDATE SET
                    credential_kind = excluded.credential_kind,
                    secret_ciphertext = excluded.secret_ciphertext,
                    scope = excluded.scope, token_expires_at = NULL,
                    updated_at = excluded.updated_at, version = mail_credentials.version + 1",
                    params![
                        account_id,
                        input.credential_kind,
                        input.secret_ciphertext,
                        input.scope,
                        now,
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO mail_sync_state(account_id, updated_at)
                 VALUES (?1, ?2) ON CONFLICT(account_id) DO NOTHING",
                    params![account_id, now],
                )?;
                query_mail_account(transaction, &account_id)
            })
    }

    pub fn list_mail_accounts(&self) -> Result<Vec<EncryptedMailAccount>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT account.id, account.provider, account.email_ciphertext,
                    account.email_blind_index, account.display_name_ciphertext, account.status,
                    sync.last_success_at, sync.next_expected_at, sync.watch_expires_at,
                    sync.last_error_code, coalesce(sync.consecutive_failures, 0),
                    account.created_at, account.updated_at, account.version
             FROM mail_accounts AS account
             LEFT JOIN mail_sync_state AS sync ON sync.account_id = account.id
             ORDER BY account.provider, account.created_at, account.id",
        )?;
        let rows = statement.query_map([], map_mail_account)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn mail_account(&self, account_id: &str) -> Result<EncryptedMailAccount> {
        let connection = self.database.connect()?;
        query_mail_account(&connection, account_id)
    }

    pub fn mail_account_credential(&self, account_id: &str) -> Result<EncryptedMailCredential> {
        self.database
            .connect()?
            .query_row(
                "SELECT account_id, credential_kind, secret_ciphertext, scope,
                    token_expires_at, version
             FROM mail_credentials WHERE account_id = ?1",
                [account_id],
                |row| {
                    Ok(EncryptedMailCredential {
                        account_id: row.get(0)?,
                        credential_kind: row.get(1)?,
                        secret_ciphertext: row.get(2)?,
                        scope: row.get(3)?,
                        token_expires_at: row.get(4)?,
                        version: row.get(5)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn list_mail_sync_accounts(&self, provider: MailProvider) -> Result<Vec<MailSyncAccount>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT account.id, account.provider, account.email_ciphertext,
                    account.email_blind_index, account.display_name_ciphertext, account.status,
                    sync.last_success_at, sync.next_expected_at, sync.watch_expires_at,
                    sync.last_error_code, sync.consecutive_failures,
                    account.created_at, account.updated_at, account.version,
                    credential.account_id, credential.credential_kind,
                    credential.secret_ciphertext, credential.scope,
                    credential.token_expires_at, credential.version,
                    sync.cursor_ciphertext, sync.uid_validity
             FROM mail_accounts AS account
             JOIN mail_credentials AS credential ON credential.account_id = account.id
             JOIN mail_sync_state AS sync ON sync.account_id = account.id
             WHERE account.provider = ?1 AND account.status = 'connected'
             ORDER BY account.created_at, account.id",
        )?;
        let rows = statement.query_map([provider.as_str()], |row| {
            let persisted_provider: String = row.get(1)?;
            Ok(MailSyncAccount {
                account: EncryptedMailAccount {
                    id: row.get(0)?,
                    provider: MailProvider::try_from(persisted_provider.as_str())
                        .map_err(to_sql_error)?,
                    email_ciphertext: row.get(2)?,
                    email_blind_index: row.get(3)?,
                    display_name_ciphertext: row.get(4)?,
                    status: row.get(5)?,
                    last_success_at: row.get(6)?,
                    next_expected_at: row.get(7)?,
                    watch_expires_at: row.get(8)?,
                    last_error_code: row.get(9)?,
                    consecutive_failures: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    version: row.get(13)?,
                },
                credential: EncryptedMailCredential {
                    account_id: row.get(14)?,
                    credential_kind: row.get(15)?,
                    secret_ciphertext: row.get(16)?,
                    scope: row.get(17)?,
                    token_expires_at: row.get(18)?,
                    version: row.get(19)?,
                },
                cursor_ciphertext: row.get(20)?,
                uid_validity: row.get(21)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_mail_sync(
        &self,
        account_id: &str,
        provider: MailProvider,
        operation: &str,
        cursor_ciphertext: Option<&str>,
        uid_validity: Option<&str>,
        watch_expires_at: Option<&str>,
        next_expected_at: &str,
        detected_count: u32,
    ) -> Result<()> {
        if !matches!(
            operation,
            "watch" | "reconcile" | "poll" | "push" | "retention"
        ) {
            return Err(invalid("mail sync operation is invalid"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                let changed = transaction.execute(
                    "UPDATE mail_sync_state SET
                        cursor_ciphertext = coalesce(?2, cursor_ciphertext),
                        uid_validity = coalesce(?3, uid_validity),
                        watch_expires_at = coalesce(?4, watch_expires_at),
                        last_attempt_at = ?5, last_success_at = ?5, next_expected_at = ?6,
                        last_error_code = NULL, consecutive_failures = 0,
                        updated_at = ?5, version = version + 1
                     WHERE account_id = ?1",
                    params![
                        account_id,
                        cursor_ciphertext,
                        uid_validity,
                        watch_expires_at,
                        now,
                        next_expected_at,
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::NotFound {
                        entity: "mail account sync state",
                        id: account_id.to_owned(),
                    });
                }
                transaction.execute(
                    "INSERT INTO mail_sync_events(
                        id, account_id, provider, operation, status, detected_count,
                        changed_provider_state, error_code, started_at, completed_at
                     ) VALUES (?1, ?2, ?3, ?4, 'succeeded', ?5, 0, NULL, ?6, ?6)",
                    params![
                        new_id(),
                        account_id,
                        provider.as_str(),
                        operation,
                        detected_count,
                        now,
                    ],
                )?;
                Ok(())
            })
    }

    pub fn fail_mail_sync(
        &self,
        account_id: &str,
        provider: MailProvider,
        operation: &str,
        error_code: &str,
        reconnect_required: bool,
    ) -> Result<()> {
        if !matches!(
            operation,
            "watch" | "reconcile" | "poll" | "push" | "retention"
        ) || error_code.is_empty()
            || error_code.len() > 80
        {
            return Err(invalid("mail sync failure metadata is invalid"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                if reconnect_required {
                    transaction.execute(
                        "UPDATE mail_accounts SET status = 'reconnect_required',
                            updated_at = ?2, version = version + 1
                         WHERE id = ?1 AND status = 'connected'",
                        params![account_id, now],
                    )?;
                }
                transaction.execute(
                    "UPDATE mail_sync_state SET last_attempt_at = ?2, last_error_code = ?3,
                        consecutive_failures = consecutive_failures + 1, updated_at = ?2,
                        version = version + 1 WHERE account_id = ?1",
                    params![account_id, now, error_code],
                )?;
                transaction.execute(
                    "INSERT INTO mail_sync_events(
                        id, account_id, provider, operation, status, detected_count,
                        changed_provider_state, error_code, started_at, completed_at
                     ) VALUES (?1, ?2, ?3, ?4, 'failed', 0, 0, ?5, ?6, ?6)",
                    params![
                        new_id(),
                        account_id,
                        provider.as_str(),
                        operation,
                        error_code,
                        now,
                    ],
                )?;
                Ok(())
            })
    }

    pub fn create_mail_oauth_state(&self, state: MailOAuthState) -> Result<()> {
        validate_sha256(&state.state_sha256, "mail OAuth state digest")?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                transaction.execute(
                    "INSERT INTO mail_oauth_states(
                    state_sha256, pkce_verifier_ciphertext, return_uri_ciphertext,
                    expires_at, consumed_at, created_at
                 ) VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                    params![
                        state.state_sha256,
                        state.pkce_verifier_ciphertext,
                        state.return_uri_ciphertext,
                        state.expires_at,
                        now_utc(),
                    ],
                )?;
                Ok(())
            })
    }

    pub fn consume_mail_oauth_state(
        &self,
        state_sha256: &str,
        as_of: DateTime<Utc>,
    ) -> Result<MailOAuthState> {
        validate_sha256(state_sha256, "mail OAuth state digest")?;
        self.database.transaction(TransactionBehavior::Immediate, |transaction| {
            let state = transaction.query_row(
                "SELECT state_sha256, pkce_verifier_ciphertext, return_uri_ciphertext, expires_at
                 FROM mail_oauth_states
                 WHERE state_sha256 = ?1 AND consumed_at IS NULL AND expires_at > ?2",
                params![state_sha256, as_of.to_rfc3339()],
                |row| Ok(MailOAuthState {
                    state_sha256: row.get(0)?,
                    pkce_verifier_ciphertext: row.get(1)?,
                    return_uri_ciphertext: row.get(2)?,
                    expires_at: row.get(3)?,
                }),
            ).optional()?.ok_or_else(|| Error::Conflict("mail OAuth state is invalid, expired, or already used".to_owned()))?;
            let changed = transaction.execute(
                "UPDATE mail_oauth_states SET consumed_at = ?2
                 WHERE state_sha256 = ?1 AND consumed_at IS NULL",
                params![state_sha256, as_of.to_rfc3339()],
            )?;
            if changed != 1 {
                return Err(Error::Conflict("mail OAuth state was consumed concurrently".to_owned()));
            }
            Ok(state)
        })
    }

    pub fn record_mail_webhook(
        &self,
        message_id_sha256: &str,
        account_hint_sha256: Option<&str>,
        published_at: Option<&str>,
    ) -> Result<bool> {
        validate_sha256(message_id_sha256, "mail webhook message digest")?;
        if let Some(value) = account_hint_sha256 {
            validate_sha256(value, "mail webhook account hint digest")?;
        }
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "INSERT INTO mail_webhook_events(
                message_id_sha256, account_hint_sha256, published_at, status, created_at, processed_at
             ) VALUES (?1, ?2, ?3, 'accepted', ?4, NULL)
             ON CONFLICT(message_id_sha256) DO NOTHING",
            params![message_id_sha256, account_hint_sha256, published_at, now_utc()],
        )?;
        Ok(changed == 1)
    }

    pub fn complete_pending_mail_webhooks(&self, completed_at: DateTime<Utc>) -> Result<u64> {
        let changed = self.database.connect()?.execute(
            "UPDATE mail_webhook_events SET status = 'processed', processed_at = ?1
             WHERE status = 'accepted'",
            [completed_at.to_rfc3339()],
        )?;
        Ok(changed as u64)
    }

    pub fn mark_mail_account_reconnect_required(
        &self,
        account_id: &str,
        error_code: &str,
    ) -> Result<()> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                transaction.execute(
                    "UPDATE mail_accounts SET status = 'reconnect_required', updated_at = ?2,
                    version = version + 1 WHERE id = ?1 AND status <> 'disabled'",
                    params![account_id, now],
                )?;
                transaction.execute(
                    "UPDATE mail_sync_state SET last_attempt_at = ?2, last_error_code = ?3,
                    consecutive_failures = consecutive_failures + 1, updated_at = ?2,
                    version = version + 1 WHERE account_id = ?1",
                    params![account_id, now, error_code],
                )?;
                Ok(())
            })
    }

    pub fn disable_mail_account(
        &self,
        account_id: &str,
        expected_version: u64,
    ) -> Result<EncryptedMailAccount> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                let changed = transaction.execute(
                    "UPDATE mail_accounts SET status = 'disabled', disconnected_at = ?2,
                    updated_at = ?2, version = version + 1
                 WHERE id = ?1 AND version = ?3 AND status <> 'disabled'",
                    params![account_id, now, expected_version],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "mail account version changed or account is already disabled".to_owned(),
                    ));
                }
                transaction.execute(
                    "DELETE FROM mail_credentials WHERE account_id = ?1",
                    [account_id],
                )?;
                query_mail_account(transaction, account_id)
            })
    }

    pub fn disable_mail_account_with_receipt(
        &self,
        account_id: &str,
        expected_version: u64,
        idempotency_key: &str,
        request_sha256: &str,
    ) -> Result<EncryptedMailAccount> {
        validate_mail_receipt_input(idempotency_key, request_sha256)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some(saved_id) = mail_receipt_reference_in_transaction(
                    transaction,
                    idempotency_key,
                    "account_disconnect",
                    request_sha256,
                    "accountId",
                )? {
                    return query_mail_account(transaction, &saved_id);
                }
                let now = now_utc();
                let changed = transaction.execute(
                    "UPDATE mail_accounts SET status = 'disabled', disconnected_at = ?2,
                        updated_at = ?2, version = version + 1
                     WHERE id = ?1 AND version = ?3 AND status <> 'disabled'",
                    params![account_id, now, expected_version],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "mail account version changed or account is already disabled".to_owned(),
                    ));
                }
                transaction.execute(
                    "DELETE FROM mail_credentials WHERE account_id = ?1",
                    [account_id],
                )?;
                insert_mail_reference_receipt_in_transaction(
                    transaction,
                    idempotency_key,
                    "account_disconnect",
                    request_sha256,
                    "accountId",
                    account_id,
                )?;
                query_mail_account(transaction, account_id)
            })
    }

    pub fn store_mail_item(&self, input: StoreMailItemInput) -> Result<String> {
        validate_sha256(
            &input.provider_message_sha256,
            "mail provider message digest",
        )?;
        if input.importance_score > 100 || input.confidence > 100 {
            return Err(invalid("mail importance score or confidence is invalid"));
        }
        if !matches!(input.decision_source.as_str(), "rule" | "ai" | "manual") {
            return Err(invalid("mail decision source is invalid"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                let expires_at =
                    (input.received_at + Duration::days(MAIL_RETENTION_DAYS)).to_rfc3339();
                let item_id = new_id();
                transaction.execute(
                    "INSERT INTO mail_items(
                    id, account_id, provider_message_sha256, provider_message_ciphertext,
                    thread_sha256, received_at, sender_ciphertext, sender_domain_ciphertext,
                    sender_domain_blind_index, subject_ciphertext, summary_ciphertext,
                    action_ciphertext, deadline, classification, importance_score, confidence,
                    decision_source, decision_reason, sensitive_kind, acknowledged_at,
                    expires_at, created_at, updated_at, version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18, ?19, NULL, ?20, ?21, ?21, 1)
                 ON CONFLICT(account_id, provider_message_sha256) DO NOTHING",
                    params![
                        item_id,
                        input.account_id,
                        input.provider_message_sha256,
                        input.provider_message_ciphertext,
                        input.thread_sha256,
                        input.received_at.to_rfc3339(),
                        input.sender_ciphertext,
                        input.sender_domain_ciphertext,
                        input.sender_domain_blind_index,
                        input.subject_ciphertext,
                        input.summary_ciphertext,
                        input.action_ciphertext,
                        input.deadline.map(|date| date.to_string()),
                        input.classification.as_str(),
                        input.importance_score,
                        input.confidence,
                        input.decision_source,
                        input.decision_reason,
                        input.sensitive_kind,
                        expires_at,
                        now,
                    ],
                )?;
                transaction.query_row(
                "SELECT id FROM mail_items WHERE account_id = ?1 AND provider_message_sha256 = ?2",
                params![input.account_id, input.provider_message_sha256],
                |row| row.get(0),
            ).map_err(Into::into)
            })
    }

    pub fn list_mail_items(
        &self,
        classification: Option<MailClassification>,
        account_id: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<MailItemPage> {
        if !(1..=100).contains(&limit) {
            return Err(invalid("mail item page limit must be between 1 and 100"));
        }
        let (cursor_received_at, cursor_id) = match cursor {
            None => (None, None),
            Some(value) => {
                let (received_at, id) = value
                    .split_once('|')
                    .ok_or_else(|| invalid("mail item cursor is invalid"))?;
                DateTime::parse_from_rfc3339(received_at)
                    .map_err(|_| invalid("mail item cursor is invalid"))?;
                if id.is_empty() || id.len() > 80 {
                    return Err(invalid("mail item cursor is invalid"));
                }
                (Some(received_at), Some(id))
            }
        };
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT item.id, item.account_id, account.provider,
                    item.provider_message_ciphertext, item.received_at,
                    item.sender_ciphertext, item.sender_domain_ciphertext,
                    item.subject_ciphertext, item.summary_ciphertext, item.action_ciphertext,
                    item.deadline, item.classification, item.importance_score, item.confidence,
                    item.decision_source, item.decision_reason, item.sensitive_kind,
                    item.acknowledged_at, item.version
             FROM mail_items AS item
             JOIN mail_accounts AS account ON account.id = item.account_id
             WHERE (?1 IS NULL OR item.classification = ?1)
               AND (?2 IS NULL OR item.account_id = ?2)
               AND (?3 IS NULL OR item.received_at < ?3
                    OR (item.received_at = ?3 AND item.id < ?4))
             ORDER BY item.received_at DESC, item.id DESC
             LIMIT ?5",
        )?;
        let wanted = classification.map(|value| value.as_str());
        let rows = statement.query_map(
            params![
                wanted,
                account_id,
                cursor_received_at,
                cursor_id,
                i64::from(limit) + 1
            ],
            map_mail_item,
        )?;
        let mut items = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > limit as usize {
            items.pop();
            items
                .last()
                .map(|item| format!("{}|{}", item.received_at, item.id))
        } else {
            None
        };
        Ok(MailItemPage { items, next_cursor })
    }

    pub fn acknowledge_mail_item(
        &self,
        item_id: &str,
        expected_version: u64,
    ) -> Result<EncryptedMailItem> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                let changed = transaction.execute(
                    "UPDATE mail_items SET acknowledged_at = coalesce(acknowledged_at, ?2),
                    updated_at = ?2, version = version + 1
                 WHERE id = ?1 AND version = ?3",
                    params![item_id, now, expected_version],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict("mail item version changed".to_owned()));
                }
                query_mail_item(transaction, item_id)
            })
    }

    pub fn acknowledge_mail_item_with_receipt(
        &self,
        item_id: &str,
        expected_version: u64,
        idempotency_key: &str,
        request_sha256: &str,
    ) -> Result<EncryptedMailItem> {
        validate_mail_receipt_input(idempotency_key, request_sha256)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some(saved_id) = mail_receipt_reference_in_transaction(
                    transaction,
                    idempotency_key,
                    "mail_acknowledge",
                    request_sha256,
                    "itemId",
                )? {
                    return query_mail_item(transaction, &saved_id);
                }
                let now = now_utc();
                let changed = transaction.execute(
                    "UPDATE mail_items SET acknowledged_at = coalesce(acknowledged_at, ?2),
                        updated_at = ?2, version = version + 1
                     WHERE id = ?1 AND version = ?3",
                    params![item_id, now, expected_version],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict("mail item version changed".to_owned()));
                }
                insert_mail_reference_receipt_in_transaction(
                    transaction,
                    idempotency_key,
                    "mail_acknowledge",
                    request_sha256,
                    "itemId",
                    item_id,
                )?;
                query_mail_item(transaction, item_id)
            })
    }

    pub fn mail_item(&self, item_id: &str) -> Result<EncryptedMailItem> {
        let connection = self.database.connect()?;
        query_mail_item(&connection, item_id)
    }

    pub fn record_mail_feedback(
        &self,
        item_id: &str,
        important: bool,
        expected_version: u64,
        actor: &str,
    ) -> Result<EncryptedMailItem> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let now = now_utc();
                let classification = if important {
                    "important"
                } else {
                    "not_important"
                };
                let score = if important { 100 } else { 0 };
                let changed = transaction.execute(
                    "UPDATE mail_items SET classification = ?2, importance_score = ?3,
                    confidence = 100, decision_source = 'manual',
                    decision_reason = 'user_feedback', updated_at = ?4, version = version + 1
                 WHERE id = ?1 AND version = ?5",
                    params![item_id, classification, score, now, expected_version],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict("mail item version changed".to_owned()));
                }
                transaction.execute(
                    "INSERT INTO mail_feedback(id, mail_item_id, important, actor, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![new_id(), item_id, i64::from(important), actor, now],
                )?;
                query_mail_item(transaction, item_id)
            })
    }

    pub fn record_mail_feedback_with_receipt(
        &self,
        item_id: &str,
        important: bool,
        expected_version: u64,
        actor: &str,
        idempotency_key: &str,
        request_sha256: &str,
    ) -> Result<EncryptedMailItem> {
        validate_mail_receipt_input(idempotency_key, request_sha256)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some(saved_id) = mail_receipt_reference_in_transaction(
                    transaction,
                    idempotency_key,
                    "mail_feedback",
                    request_sha256,
                    "itemId",
                )? {
                    return query_mail_item(transaction, &saved_id);
                }
                let now = now_utc();
                let classification = if important {
                    "important"
                } else {
                    "not_important"
                };
                let score = if important { 100 } else { 0 };
                let changed = transaction.execute(
                    "UPDATE mail_items SET classification = ?2, importance_score = ?3,
                        confidence = 100, decision_source = 'manual',
                        decision_reason = 'user_feedback', updated_at = ?4,
                        version = version + 1
                     WHERE id = ?1 AND version = ?5",
                    params![item_id, classification, score, now, expected_version],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict("mail item version changed".to_owned()));
                }
                transaction.execute(
                    "INSERT INTO mail_feedback(id, mail_item_id, important, actor, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![new_id(), item_id, i64::from(important), actor, now],
                )?;
                insert_mail_reference_receipt_in_transaction(
                    transaction,
                    idempotency_key,
                    "mail_feedback",
                    request_sha256,
                    "itemId",
                    item_id,
                )?;
                query_mail_item(transaction, item_id)
            })
    }

    pub fn mail_summary(&self, date: NaiveDate) -> Result<MailSummary> {
        let connection = self.database.connect()?;
        let (important, review): (i64, i64) = connection.query_row(
            "SELECT
                coalesce(sum(CASE WHEN classification = 'important' AND acknowledged_at IS NULL THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN classification = 'review' THEN 1 ELSE 0 END), 0)
             FROM mail_items WHERE substr(received_at, 1, 10) <= ?1",
            [date.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let latest_report = connection
            .query_row(
                "SELECT id, report_date, slot, summary_ciphertext, important_count,
                    review_count, generated_at
             FROM mail_reports WHERE report_date <= ?1
             ORDER BY report_date DESC, CASE slot WHEN 'evening' THEN 1 ELSE 0 END DESC
             LIMIT 1",
                [date.to_string()],
                map_mail_report,
            )
            .optional()?;
        Ok(MailSummary {
            date,
            unacknowledged_important: u32::try_from(important).unwrap_or(u32::MAX),
            review_count: u32::try_from(review).unwrap_or(u32::MAX),
            latest_report,
        })
    }

    pub fn list_mail_reports(&self, date: NaiveDate) -> Result<Vec<EncryptedMailReport>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, report_date, slot, summary_ciphertext, important_count,
                    review_count, generated_at
             FROM mail_reports WHERE report_date = ?1 ORDER BY slot",
        )?;
        let rows = statement.query_map([date.to_string()], map_mail_report)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn store_mail_report(
        &self,
        report_date: NaiveDate,
        slot: &str,
        summary_ciphertext: &str,
    ) -> Result<EncryptedMailReport> {
        if !matches!(slot, "morning" | "evening") || summary_ciphertext.is_empty() {
            return Err(invalid("mail report input is invalid"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let (important_count, review_count): (i64, i64) = transaction.query_row(
                    "SELECT
                        coalesce(sum(CASE WHEN classification = 'important' AND acknowledged_at IS NULL THEN 1 ELSE 0 END), 0),
                        coalesce(sum(CASE WHEN classification = 'review' THEN 1 ELSE 0 END), 0)
                     FROM mail_items WHERE substr(received_at, 1, 10) <= ?1",
                    [report_date.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                let now = now_utc();
                let report_id = new_id();
                let expires_at = (Utc::now() + Duration::days(MAIL_REPORT_RETENTION_DAYS)).to_rfc3339();
                transaction.execute(
                    "INSERT INTO mail_reports(
                        id, report_date, slot, summary_ciphertext, important_count,
                        review_count, generated_at, expires_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(report_date, slot) DO NOTHING",
                    params![
                        report_id,
                        report_date.to_string(),
                        slot,
                        summary_ciphertext,
                        important_count,
                        review_count,
                        now,
                        expires_at,
                    ],
                )?;
                transaction.query_row(
                    "SELECT id, report_date, slot, summary_ciphertext, important_count,
                        review_count, generated_at
                     FROM mail_reports WHERE report_date = ?1 AND slot = ?2",
                    params![report_date.to_string(), slot],
                    map_mail_report,
                ).map_err(Into::into)
            })
    }

    pub fn list_mail_ai_candidates(
        &self,
        ready_before: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<EncryptedMailItem>> {
        if !(1..=10).contains(&limit) {
            return Err(invalid("mail AI candidate limit must be between 1 and 10"));
        }
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT item.id, item.account_id, account.provider,
                    item.provider_message_ciphertext, item.received_at,
                    item.sender_ciphertext, item.sender_domain_ciphertext,
                    item.subject_ciphertext, item.summary_ciphertext, item.action_ciphertext,
                    item.deadline, item.classification, item.importance_score, item.confidence,
                    item.decision_source, item.decision_reason, item.sensitive_kind,
                    item.acknowledged_at, item.version
             FROM mail_items AS item
             JOIN mail_accounts AS account ON account.id = item.account_id
             WHERE item.classification = 'review'
               AND item.decision_source = 'rule'
               AND item.decision_reason = 'ai_candidate'
               AND item.sensitive_kind IS NULL
               AND item.created_at <= ?1
               AND NOT EXISTS(
                    SELECT 1 FROM mail_triage_items AS triage
                    WHERE triage.mail_item_id = item.id
               )
             ORDER BY item.created_at, item.id
             LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![ready_before.to_rfc3339(), i64::from(limit)],
            map_mail_item,
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn claim_mail_triage_batch(
        &self,
        request_id: &str,
        input_sha256: &str,
        prompt_version: &str,
        model: &str,
        items: &[(String, u64)],
        month_window: Range<DateTime<Utc>>,
    ) -> Result<String> {
        let item_count = u32::try_from(items.len()).unwrap_or(u32::MAX);
        validate_sha256(input_sha256, "mail triage input digest")?;
        if request_id.is_empty()
            || request_id.len() > 160
            || prompt_version.is_empty()
            || prompt_version.len() > 64
            || model.is_empty()
            || model.len() > 128
            || !(1..=10).contains(&item_count)
        {
            return Err(invalid("mail triage batch metadata is invalid"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let attempts: i64 = transaction.query_row(
                    "SELECT count(*) FROM mail_triage_batches
                     WHERE created_at >= ?1 AND created_at < ?2",
                    params![
                        month_window.start.to_rfc3339(),
                        month_window.end.to_rfc3339()
                    ],
                    |row| row.get(0),
                )?;
                if attempts >= i64::from(MAIL_TRIAGE_MONTHLY_ATTEMPT_LIMIT) {
                    return Err(Error::AiDailyLimitExceeded {
                        operation: MAIL_TRIAGE_OPERATION.to_owned(),
                        limit: MAIL_TRIAGE_MONTHLY_ATTEMPT_LIMIT,
                    });
                }
                let batch_id = new_id();
                transaction.execute(
                    "INSERT INTO mail_triage_batches(
                        id, request_id, input_sha256, prompt_version, model, status,
                        item_count, cost_microusd, created_at, completed_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 'claimed', ?6, 0, ?7, NULL)",
                    params![
                        batch_id,
                        request_id,
                        input_sha256,
                        prompt_version,
                        model,
                        item_count,
                        now_utc(),
                    ],
                )?;
                for (item_id, expected_version) in items {
                    if item_id.is_empty() || item_id.len() > 80 || *expected_version == 0 {
                        return Err(invalid("mail triage item binding is invalid"));
                    }
                    transaction.execute(
                        "INSERT INTO mail_triage_items(
                            batch_id, mail_item_id, expected_version, importance_score,
                            confidence, classification, reason_code
                         ) VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL)",
                        params![batch_id, item_id, expected_version],
                    )?;
                }
                Ok(batch_id)
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_mail_triage_batch(
        &self,
        batch_id: &str,
        response_id: &str,
        upstream_request_id: Option<&str>,
        usage: AiTokenUsage,
        cost_microusd: u64,
        decisions: &[MailTriageDecision],
    ) -> Result<u32> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let expected_count: i64 = transaction.query_row(
                    "SELECT item_count FROM mail_triage_batches
                     WHERE id = ?1 AND status = 'claimed'",
                    [batch_id],
                    |row| row.get(0),
                )?;
                if usize::try_from(expected_count).ok() != Some(decisions.len()) {
                    return Err(Error::Invariant(
                        "mail triage response count does not match its batch".to_owned(),
                    ));
                }
                let now = now_utc();
                let mut applied = 0_u32;
                for decision in decisions {
                    if decision.importance_score > 100
                        || decision.confidence > 100
                        || decision.reason_code.is_empty()
                        || decision.reason_code.len() > 80
                    {
                        return Err(invalid("mail triage decision is invalid"));
                    }
                    let completed = transaction.execute(
                        "UPDATE mail_triage_items SET importance_score = ?4,
                            confidence = ?5, classification = ?6, reason_code = ?7
                         WHERE batch_id = ?1 AND mail_item_id = ?2 AND expected_version = ?3
                           AND importance_score IS NULL AND confidence IS NULL
                           AND classification IS NULL AND reason_code IS NULL",
                        params![
                            batch_id,
                            decision.item_id,
                            decision.expected_version,
                            decision.importance_score,
                            decision.confidence,
                            decision.classification.as_str(),
                            decision.reason_code,
                        ],
                    )?;
                    if completed != 1 {
                        return Err(Error::Invariant(
                            "mail triage response did not match its claimed item".to_owned(),
                        ));
                    }
                    let changed = transaction.execute(
                        "UPDATE mail_items SET classification = ?2, importance_score = ?3,
                            confidence = ?4, decision_source = 'ai', decision_reason = ?5,
                            action_ciphertext = coalesce(?6, action_ciphertext),
                            updated_at = ?7, version = version + 1
                         WHERE id = ?1 AND version = ?8
                           AND classification = 'review' AND sensitive_kind IS NULL",
                        params![
                            decision.item_id,
                            decision.classification.as_str(),
                            decision.importance_score,
                            decision.confidence,
                            decision.reason_code,
                            decision.action_ciphertext,
                            now,
                            decision.expected_version,
                        ],
                    )?;
                    applied = applied.saturating_add(u32::try_from(changed).unwrap_or(0));
                }
                let changed = transaction.execute(
                    "UPDATE mail_triage_batches SET status = 'succeeded', response_id = ?2,
                        upstream_request_id = ?3, input_tokens = ?4, cached_input_tokens = ?5,
                        output_tokens = ?6, total_tokens = ?7, cost_microusd = ?8,
                        completed_at = ?9 WHERE id = ?1 AND status = 'claimed'",
                    params![
                        batch_id,
                        response_id,
                        upstream_request_id,
                        usage.input_tokens,
                        usage.cached_input_tokens,
                        usage.output_tokens,
                        usage.total_tokens,
                        cost_microusd,
                        now,
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "mail triage batch was already completed".to_owned(),
                    ));
                }
                Ok(applied)
            })
    }

    pub fn fail_mail_triage_batch(&self, batch_id: &str, failure_code: &str) -> Result<()> {
        if failure_code.is_empty() || failure_code.len() > 80 {
            return Err(invalid("mail triage failure code is invalid"));
        }
        let changed = self.database.connect()?.execute(
            "UPDATE mail_triage_batches SET status = 'failed', failure_code = ?2,
                completed_at = ?3 WHERE id = ?1 AND status = 'claimed'",
            params![batch_id, failure_code, now_utc()],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(
                "mail triage batch was already completed".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn purge_expired_mail_metadata(&self, as_of: DateTime<Utc>) -> Result<(u64, u64)> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let timestamp = as_of.to_rfc3339();
                transaction.execute(
                    "DELETE FROM mail_report_items
                     WHERE report_id IN (SELECT id FROM mail_reports WHERE expires_at <= ?1)",
                    [&timestamp],
                )?;
                let reports = transaction.execute(
                    "DELETE FROM mail_reports WHERE expires_at <= ?1",
                    [&timestamp],
                )?;
                transaction.execute(
                    "DELETE FROM mail_report_items
                     WHERE mail_item_id IN (SELECT id FROM mail_items WHERE expires_at <= ?1)",
                    [&timestamp],
                )?;
                transaction.execute(
                    "DELETE FROM mail_feedback
                     WHERE mail_item_id IN (SELECT id FROM mail_items WHERE expires_at <= ?1)",
                    [&timestamp],
                )?;
                let items = transaction.execute(
                    "DELETE FROM mail_items WHERE expires_at <= ?1",
                    [&timestamp],
                )?;
                transaction.execute(
                    "DELETE FROM mail_oauth_states WHERE expires_at <= ?1",
                    [&timestamp],
                )?;
                Ok((items as u64, reports as u64))
            })
    }

    pub fn mail_ops_status(&self) -> Result<MailOpsStatus> {
        let connection = self.database.connect()?;
        let (account_count, connected, reconnect): (i64, i64, i64) = connection.query_row(
            "SELECT count(*),
                    coalesce(sum(CASE WHEN status = 'connected' THEN 1 ELSE 0 END), 0),
                    coalesce(sum(CASE WHEN status = 'reconnect_required' THEN 1 ELSE 0 END), 0)
             FROM mail_accounts WHERE status <> 'disabled'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let (important, review): (i64, i64) = connection.query_row(
            "SELECT
                coalesce(sum(CASE WHEN classification = 'important' AND acknowledged_at IS NULL THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN classification = 'review' THEN 1 ELSE 0 END), 0)
             FROM mail_items",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (latest_success_at, last_error_code): (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT max(last_success_at),
                    (SELECT last_error_code FROM mail_sync_state
                     WHERE last_error_code IS NOT NULL ORDER BY updated_at DESC LIMIT 1)
             FROM mail_sync_state",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
        let (queue_depth, retry_count, dead_letter_count): (i64, i64, i64) = connection.query_row(
            "SELECT
                coalesce(sum(CASE WHEN run.status IN ('pending', 'running') THEN 1 ELSE 0 END), 0)
                    + (SELECT count(*) FROM mail_webhook_events WHERE status = 'accepted'),
                coalesce(sum(CASE WHEN run.status = 'retry_wait' THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN run.status = 'dead_letter' THEN 1 ELSE 0 END), 0)
             FROM scheduler_runs AS run
             JOIN scheduler_jobs AS job ON job.id = run.job_id
             WHERE job.kind LIKE 'mail.%'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let mut account_statement = connection.prepare(
            "SELECT account.id, account.provider, account.status, sync.last_success_at,
                    sync.next_expected_at, sync.watch_expires_at, sync.last_error_code,
                    sync.consecutive_failures
             FROM mail_accounts AS account
             JOIN mail_sync_state AS sync ON sync.account_id = account.id
             WHERE account.status <> 'disabled'
             ORDER BY account.provider, account.id",
        )?;
        let accounts = account_statement
            .query_map([], |row| {
                let provider: String = row.get(1)?;
                Ok(MailAccountOpsStatus {
                    account_id: row.get(0)?,
                    provider: MailProvider::try_from(provider.as_str()).map_err(to_sql_error)?,
                    status: row.get(2)?,
                    last_success_at: row.get(3)?,
                    next_expected_at: row.get(4)?,
                    watch_expires_at: row.get(5)?,
                    last_error_code: row.get(6)?,
                    consecutive_failures: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let triage_claimed_count: i64 = connection.query_row(
            "SELECT count(*) FROM mail_triage_batches WHERE status = 'claimed'",
            [],
            |row| row.get(0),
        )?;
        let latest_triage: Option<(String, Option<String>, Option<String>)> = connection
            .query_row(
                "SELECT status, failure_code, completed_at FROM mail_triage_batches
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(MailOpsStatus {
            account_count: nonnegative_u32(account_count),
            connected_count: nonnegative_u32(connected),
            reconnect_required_count: nonnegative_u32(reconnect),
            unacknowledged_important_count: nonnegative_u32(important),
            review_count: nonnegative_u32(review),
            latest_success_at,
            last_error_code,
            queue_depth: nonnegative_u32(queue_depth),
            retry_count: nonnegative_u32(retry_count),
            dead_letter_count: nonnegative_u32(dead_letter_count),
            accounts,
            triage_claimed_count: nonnegative_u32(triage_claimed_count),
            latest_triage_status: latest_triage.as_ref().map(|value| value.0.clone()),
            latest_triage_failure_code: latest_triage.as_ref().and_then(|value| value.1.clone()),
            latest_triage_completed_at: latest_triage.and_then(|value| value.2),
        })
    }

    pub fn store_mail_mutation_receipt(
        &self,
        idempotency_key: &str,
        operation: &str,
        request_sha256: &str,
        response: &Value,
    ) -> Result<()> {
        validate_sha256(request_sha256, "mail mutation request digest")?;
        if idempotency_key.is_empty() || idempotency_key.len() > 128 {
            return Err(invalid("mail idempotency key is invalid"));
        }
        let response_json = serde_json::to_string(response)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let existing: Option<(String, String, String)> = transaction.query_row(
                "SELECT operation, request_sha256, response_json FROM mail_mutation_receipts
                 WHERE idempotency_key = ?1",
                [idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional()?;
                if let Some(existing) = existing {
                    if existing
                        != (
                            operation.to_owned(),
                            request_sha256.to_owned(),
                            response_json.clone(),
                        )
                    {
                        return Err(Error::Conflict(
                            "mail idempotency key was reused with different data".to_owned(),
                        ));
                    }
                    return Ok(());
                }
                transaction.execute(
                    "INSERT INTO mail_mutation_receipts(
                    idempotency_key, operation, request_sha256, response_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        idempotency_key,
                        operation,
                        request_sha256,
                        response_json,
                        now_utc()
                    ],
                )?;
                Ok(())
            })
    }

    pub fn get_mail_mutation_receipt(
        &self,
        idempotency_key: &str,
        operation: &str,
        request_sha256: &str,
    ) -> Result<Option<Value>> {
        let row: Option<(String, String, String)> = self
            .database
            .connect()?
            .query_row(
                "SELECT operation, request_sha256, response_json FROM mail_mutation_receipts
             WHERE idempotency_key = ?1",
                [idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((saved_operation, saved_sha256, response_json)) = row else {
            return Ok(None);
        };
        if saved_operation != operation || saved_sha256 != request_sha256 {
            return Err(Error::Conflict(
                "mail idempotency key was reused with different data".to_owned(),
            ));
        }
        Ok(Some(serde_json::from_str(&response_json)?))
    }
}

fn validate_mail_receipt_input(idempotency_key: &str, request_sha256: &str) -> Result<()> {
    validate_sha256(request_sha256, "mail mutation request digest")?;
    if idempotency_key.is_empty() || idempotency_key.len() > 128 {
        return Err(invalid("mail idempotency key is invalid"));
    }
    Ok(())
}

fn mail_receipt_reference_in_transaction(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
    operation: &str,
    request_sha256: &str,
    reference_name: &str,
) -> Result<Option<String>> {
    let existing: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT operation, request_sha256, response_json
             FROM mail_mutation_receipts WHERE idempotency_key = ?1",
            [idempotency_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((saved_operation, saved_sha256, response_json)) = existing else {
        return Ok(None);
    };
    if saved_operation != operation || saved_sha256 != request_sha256 {
        return Err(Error::Conflict(
            "mail idempotency key was reused with different data".to_owned(),
        ));
    }
    let response: Value = serde_json::from_str(&response_json)?;
    response
        .get(reference_name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 80)
        .map(str::to_owned)
        .map(Some)
        .ok_or_else(|| Error::Invariant("mail mutation receipt is invalid".to_owned()))
}

fn insert_mail_reference_receipt_in_transaction(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
    operation: &str,
    request_sha256: &str,
    reference_name: &str,
    reference_id: &str,
) -> Result<()> {
    if !matches!(reference_name, "accountId" | "itemId")
        || reference_id.is_empty()
        || reference_id.len() > 80
    {
        return Err(invalid("mail mutation receipt reference is invalid"));
    }
    let response = serde_json::to_string(&json_reference(reference_name, reference_id))?;
    transaction.execute(
        "INSERT INTO mail_mutation_receipts(
            idempotency_key, operation, request_sha256, response_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            idempotency_key,
            operation,
            request_sha256,
            response,
            now_utc(),
        ],
    )?;
    Ok(())
}

fn json_reference(name: &str, id: &str) -> Value {
    Value::Object(
        [(name.to_owned(), Value::String(id.to_owned()))]
            .into_iter()
            .collect(),
    )
}

fn query_mail_account(
    connection: &rusqlite::Connection,
    account_id: &str,
) -> Result<EncryptedMailAccount> {
    connection
        .query_row(
            "SELECT account.id, account.provider, account.email_ciphertext,
                account.email_blind_index, account.display_name_ciphertext, account.status,
                sync.last_success_at, sync.next_expected_at, sync.watch_expires_at,
                sync.last_error_code, coalesce(sync.consecutive_failures, 0),
                account.created_at, account.updated_at, account.version
         FROM mail_accounts AS account
         LEFT JOIN mail_sync_state AS sync ON sync.account_id = account.id
         WHERE account.id = ?1",
            [account_id],
            map_mail_account,
        )
        .map_err(Into::into)
}

fn map_mail_account(row: &Row<'_>) -> rusqlite::Result<EncryptedMailAccount> {
    let provider: String = row.get(1)?;
    Ok(EncryptedMailAccount {
        id: row.get(0)?,
        provider: MailProvider::try_from(provider.as_str()).map_err(to_sql_error)?,
        email_ciphertext: row.get(2)?,
        email_blind_index: row.get(3)?,
        display_name_ciphertext: row.get(4)?,
        status: row.get(5)?,
        last_success_at: row.get(6)?,
        next_expected_at: row.get(7)?,
        watch_expires_at: row.get(8)?,
        last_error_code: row.get(9)?,
        consecutive_failures: row.get::<_, u32>(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        version: row.get(13)?,
    })
}

fn query_mail_item(connection: &rusqlite::Connection, item_id: &str) -> Result<EncryptedMailItem> {
    connection
        .query_row(
            "SELECT item.id, item.account_id, account.provider,
                item.provider_message_ciphertext, item.received_at,
                item.sender_ciphertext, item.sender_domain_ciphertext,
                item.subject_ciphertext, item.summary_ciphertext, item.action_ciphertext,
                item.deadline, item.classification, item.importance_score, item.confidence,
                item.decision_source, item.decision_reason, item.sensitive_kind,
                item.acknowledged_at, item.version
         FROM mail_items AS item
         JOIN mail_accounts AS account ON account.id = item.account_id
         WHERE item.id = ?1",
            [item_id],
            map_mail_item,
        )
        .map_err(Into::into)
}

fn map_mail_item(row: &Row<'_>) -> rusqlite::Result<EncryptedMailItem> {
    let provider: String = row.get(2)?;
    let classification: String = row.get(11)?;
    let deadline: Option<String> = row.get(10)?;
    Ok(EncryptedMailItem {
        id: row.get(0)?,
        account_id: row.get(1)?,
        provider: MailProvider::try_from(provider.as_str()).map_err(to_sql_error)?,
        provider_message_ciphertext: row.get(3)?,
        received_at: row.get(4)?,
        sender_ciphertext: row.get(5)?,
        sender_domain_ciphertext: row.get(6)?,
        subject_ciphertext: row.get(7)?,
        summary_ciphertext: row.get(8)?,
        action_ciphertext: row.get(9)?,
        deadline: deadline
            .map(|value| NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(to_sql_error))
            .transpose()?,
        classification: MailClassification::try_from(classification.as_str())
            .map_err(to_sql_error)?,
        importance_score: row.get(12)?,
        confidence: row.get(13)?,
        decision_source: row.get(14)?,
        decision_reason: row.get(15)?,
        sensitive_kind: row.get(16)?,
        acknowledged_at: row.get(17)?,
        received_version: row.get(18)?,
    })
}

fn map_mail_report(row: &Row<'_>) -> rusqlite::Result<EncryptedMailReport> {
    let report_date: String = row.get(1)?;
    Ok(EncryptedMailReport {
        id: row.get(0)?,
        report_date: NaiveDate::parse_from_str(&report_date, "%Y-%m-%d").map_err(to_sql_error)?,
        slot: row.get(2)?,
        summary_ciphertext: row.get(3)?,
        important_count: row.get(4)?,
        review_count: row.get(5)?,
        generated_at: row.get(6)?,
    })
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(format!("{label} is invalid")));
    }
    Ok(())
}

fn nonnegative_u32(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

fn to_sql_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    use super::*;
    use crate::TmHome;

    fn core_with_candidate() -> Result<(TmCore, String)> {
        let temporary = tempdir()?;
        let home = TmHome::new(temporary.keep());
        let core = TmCore::open(home)?;
        let account = core.store_mail_account(StoreMailAccountInput {
            provider: MailProvider::Gmail,
            email_ciphertext: "cipher-email".to_owned(),
            email_blind_index: "a".repeat(64),
            display_name_ciphertext: None,
            credential_kind: "oauth_refresh_token".to_owned(),
            secret_ciphertext: "cipher-token".to_owned(),
            scope: Some("gmail.readonly".to_owned()),
        })?;
        let item_id = core.store_mail_item(StoreMailItemInput {
            account_id: account.id,
            provider_message_sha256: "b".repeat(64),
            provider_message_ciphertext: "cipher-message".to_owned(),
            thread_sha256: None,
            received_at: Utc::now() - Duration::minutes(2),
            sender_ciphertext: "cipher-sender".to_owned(),
            sender_domain_ciphertext: Some("cipher-domain".to_owned()),
            sender_domain_blind_index: Some("c".repeat(64)),
            subject_ciphertext: "cipher-subject".to_owned(),
            summary_ciphertext: Some("cipher-summary".to_owned()),
            action_ciphertext: None,
            deadline: None,
            classification: MailClassification::Review,
            importance_score: 50,
            confidence: 40,
            decision_source: "rule".to_owned(),
            decision_reason: "ai_candidate".to_owned(),
            sensitive_kind: None,
        })?;
        Ok((core, item_id))
    }

    fn claim_candidate(core: &TmCore, item_id: &str) -> Result<String> {
        core.claim_mail_triage_batch(
            "mail-test-request",
            &"d".repeat(64),
            "mail-triage-v1",
            "gpt-5.4-nano-2026-03-17",
            &[(item_id.to_owned(), 1)],
            (Utc::now() - Duration::days(1))..(Utc::now() + Duration::days(31)),
        )
    }

    #[test]
    fn failed_triage_binding_prevents_automatic_rebilling() -> Result<()> {
        let (core, item_id) = core_with_candidate()?;
        assert_eq!(core.list_mail_ai_candidates(Utc::now(), 10)?.len(), 1);
        let batch = claim_candidate(&core, &item_id)?;
        assert!(core.list_mail_ai_candidates(Utc::now(), 10)?.is_empty());
        core.fail_mail_triage_batch(&batch, "openai_failed")?;
        assert!(core.list_mail_ai_candidates(Utc::now(), 10)?.is_empty());
        Ok(())
    }

    #[test]
    fn claimed_triage_item_completes_exactly_once() -> Result<()> {
        let (core, item_id) = core_with_candidate()?;
        let batch = claim_candidate(&core, &item_id)?;
        let decisions = [MailTriageDecision {
            item_id: item_id.clone(),
            expected_version: 1,
            importance_score: 82,
            confidence: 91,
            classification: MailClassification::Important,
            reason_code: "action_required".to_owned(),
            action_ciphertext: None,
        }];
        let usage = AiTokenUsage {
            input_tokens: 100,
            cached_input_tokens: 0,
            output_tokens: 20,
            total_tokens: 120,
        };
        assert_eq!(
            core.complete_mail_triage_batch(
                &batch,
                "response-1",
                Some("request-1"),
                usage,
                100,
                &decisions,
            )?,
            1
        );
        let item = core.mail_item(&item_id)?;
        assert_eq!(item.classification, MailClassification::Important);
        assert_eq!(item.received_version, 2);
        assert!(
            core.complete_mail_triage_batch(
                &batch,
                "response-1",
                Some("request-1"),
                usage,
                100,
                &decisions,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn mail_item_mutation_and_receipt_replay_are_atomic() -> Result<()> {
        let (core, item_id) = core_with_candidate()?;
        let request_sha = "e".repeat(64);
        let first = core.acknowledge_mail_item_with_receipt(
            &item_id,
            1,
            "desktop-mail:test-ack",
            &request_sha,
        )?;
        let replay = core.acknowledge_mail_item_with_receipt(
            &item_id,
            1,
            "desktop-mail:test-ack",
            &request_sha,
        )?;
        assert_eq!(first.received_version, 2);
        assert_eq!(replay.received_version, 2);
        assert_eq!(first.acknowledged_at, replay.acknowledged_at);
        Ok(())
    }

    #[test]
    fn oauth_state_is_single_use_and_expiry_is_enforced() -> Result<()> {
        let temporary = tempdir()?;
        let core = TmCore::open(TmHome::new(temporary.keep()))?;
        let state_sha256 = "f".repeat(64);
        core.create_mail_oauth_state(MailOAuthState {
            state_sha256: state_sha256.clone(),
            pkce_verifier_ciphertext: "cipher-verifier".to_owned(),
            return_uri_ciphertext: "cipher-return".to_owned(),
            expires_at: (Utc::now() + Duration::minutes(10)).to_rfc3339(),
        })?;
        core.consume_mail_oauth_state(&state_sha256, Utc::now())?;
        assert!(
            core.consume_mail_oauth_state(&state_sha256, Utc::now())
                .is_err()
        );

        let expired_sha256 = "1".repeat(64);
        core.create_mail_oauth_state(MailOAuthState {
            state_sha256: expired_sha256.clone(),
            pkce_verifier_ciphertext: "cipher-verifier".to_owned(),
            return_uri_ciphertext: "cipher-return".to_owned(),
            expires_at: (Utc::now() - Duration::minutes(1)).to_rfc3339(),
        })?;
        assert!(
            core.consume_mail_oauth_state(&expired_sha256, Utc::now())
                .is_err()
        );
        Ok(())
    }
}
