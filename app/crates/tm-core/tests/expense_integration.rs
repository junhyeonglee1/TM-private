use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tm_core::{
    ConfirmRecurringPaidInput, CreateRecurringExpenseInput, EncryptedExpenseText, Error,
    ExpenseCategory, ExpenseCryptoProbe, ExpenseDirection, ExpenseEventKind, ExpenseImportAdapter,
    ExpenseImportPreview, ExpenseImportPreviewInput, ExpenseImportPreviewRow,
    ExpenseMutationCommand, ExpenseMutationRequest, ExpenseReportFact, ExpenseReportObservation,
    ExpenseReportStatus, ExpenseReviewFilter, ExpenseReviewReason, ExpenseReviewStatus,
    ExpenseSourceKind, ExpenseTransactionFilter, MatchRecurringExpenseInput,
    NormalizedExpenseImport, NormalizedExpenseRow, OverrideExpenseTransactionInput,
    RecurringAmountKind, RecurringDueRule, RecurringExpenseItem, RecurringExpenseStatus,
    RecurringOccurrenceStatus, ResolveExpenseReviewInput, Result, SaveExpenseReportInput, TmCore,
    TmHome, UpdateExpenseSourceStatusInput, UpdateRecurringExpenseInput, expense_text_aad,
};
use uuid::Uuid;

fn fixture() -> Result<(TempDir, TmCore)> {
    let temporary = tempfile::tempdir()?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn preview_and_import(
    core: &TmCore,
    input: NormalizedExpenseImport,
) -> Result<tm_core::ExpenseImportResult> {
    let preview = core.preview_expense_import(&input)?;
    core.import_expenses(&preview.session_id, input)
}

fn redacted_preview(input: &NormalizedExpenseImport) -> ExpenseImportPreviewInput {
    ExpenseImportPreviewInput {
        adapter: input.adapter,
        source_kind: input.source_kind,
        source_fingerprint: input.source_fingerprint.clone(),
        file_sha256: input.file_sha256.clone(),
        normalized_sha256: input.normalized_sha256.clone(),
        coverage_start: input.coverage_start,
        coverage_end: input.coverage_end,
        rejected_count: input.rejected_count,
        rows: input
            .rows
            .iter()
            .map(|row| ExpenseImportPreviewRow {
                stable_key: row.stable_key.clone(),
                row_sha256: row.row_sha256.clone(),
                source_row_number: row.source_row_number,
                occurred_at: row.occurred_at.clone(),
                posted_date: row.posted_date,
                direction: row.direction,
                amount_minor: row.amount_minor,
                currency: row.currency.clone(),
                kind: row.kind,
                category_hint: row.category_hint,
                merchant_blind_index: row.merchant.as_ref().map(|value| value.blind_index.clone()),
                payment_method_fingerprint: row.payment_method_fingerprint.clone(),
                external_reference_fingerprint: row.external_reference_fingerprint.clone(),
            })
            .collect(),
    }
}

fn empty_import(
    source_fingerprint: char,
    file_fingerprint: char,
    coverage_start: NaiveDate,
    coverage_end: NaiveDate,
    rejected_count: u32,
) -> NormalizedExpenseImport {
    NormalizedExpenseImport {
        adapter: ExpenseImportAdapter::KbAccountHistoryV1,
        source_kind: ExpenseSourceKind::Account,
        source_fingerprint: digest(source_fingerprint),
        file_sha256: digest(file_fingerprint),
        normalized_sha256: digest(file_fingerprint),
        coverage_start,
        coverage_end,
        rejected_count,
        rows: Vec::new(),
    }
}

fn digest(value: char) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn encrypted(context: &str, field: &str, fingerprint: char) -> EncryptedExpenseText {
    EncryptedExpenseText {
        key_version: 1,
        nonce: "nonce-for-synthetic-test".to_owned(),
        ciphertext: "ciphertext-for-synthetic-test-value".to_owned(),
        aad: expense_text_aad(context, field),
        blind_index: digest(fingerprint),
    }
}

#[allow(clippy::too_many_arguments)]
fn row(
    source_fingerprint: &str,
    stable_key: &str,
    row_number: u32,
    date: NaiveDate,
    kind: ExpenseEventKind,
    direction: ExpenseDirection,
    amount_minor: i64,
    category_hint: Option<ExpenseCategory>,
    reference_fingerprint: char,
) -> NormalizedExpenseRow {
    let context = format!("{source_fingerprint}:{stable_key}");
    NormalizedExpenseRow {
        stable_key: stable_key.to_owned(),
        row_sha256: digest(reference_fingerprint),
        source_row_number: row_number,
        occurred_at: format!("{date}T12:00:00+09:00"),
        posted_date: date,
        direction,
        amount_minor,
        currency: "KRW".to_owned(),
        kind,
        category_hint,
        merchant: Some(encrypted(&context, "merchant", reference_fingerprint)),
        counterparty: None,
        memo: None,
        payment_method_fingerprint: Some(digest('f')),
        external_reference_fingerprint: Some(digest(reference_fingerprint)),
    }
}

fn july_import() -> NormalizedExpenseImport {
    let source_fingerprint = digest('a');
    let month_start = NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid date");
    let month_end = NaiveDate::from_ymd_opt(2026, 7, 31).expect("valid date");
    let rows = vec![
        row(
            &source_fingerprint,
            "purchase",
            1,
            month_start,
            ExpenseEventKind::Purchase,
            ExpenseDirection::Debit,
            10_000,
            Some(ExpenseCategory::Food),
            '1',
        ),
        row(
            &source_fingerprint,
            "refund",
            2,
            month_start.succ_opt().expect("valid date"),
            ExpenseEventKind::Refund,
            ExpenseDirection::Credit,
            2_000,
            None,
            '2',
        ),
        row(
            &source_fingerprint,
            "settlement-received",
            3,
            NaiveDate::from_ymd_opt(2026, 7, 3).expect("valid date"),
            ExpenseEventKind::SettlementReceived,
            ExpenseDirection::Credit,
            3_000,
            None,
            '3',
        ),
        row(
            &source_fingerprint,
            "settlement-sent",
            4,
            NaiveDate::from_ymd_opt(2026, 7, 4).expect("valid date"),
            ExpenseEventKind::SettlementSent,
            ExpenseDirection::Debit,
            1_000,
            None,
            '4',
        ),
        row(
            &source_fingerprint,
            "fee",
            5,
            NaiveDate::from_ymd_opt(2026, 7, 5).expect("valid date"),
            ExpenseEventKind::Fee,
            ExpenseDirection::Debit,
            100,
            None,
            '5',
        ),
        row(
            &source_fingerprint,
            "unknown-p2p",
            6,
            NaiveDate::from_ymd_opt(2026, 7, 6).expect("valid date"),
            ExpenseEventKind::UnknownP2p,
            ExpenseDirection::Debit,
            500,
            None,
            '6',
        ),
        row(
            &source_fingerprint,
            "card-payment",
            7,
            NaiveDate::from_ymd_opt(2026, 7, 7).expect("valid date"),
            ExpenseEventKind::CardPayment,
            ExpenseDirection::Debit,
            10_000,
            None,
            '7',
        ),
    ];
    NormalizedExpenseImport {
        adapter: ExpenseImportAdapter::KbAccountHistoryV1,
        source_kind: ExpenseSourceKind::Account,
        source_fingerprint,
        file_sha256: digest('b'),
        normalized_sha256: digest('c'),
        coverage_start: month_start,
        coverage_end: month_end,
        rejected_count: 0,
        rows,
    }
}

fn recurring_input(amount_minor: i64) -> CreateRecurringExpenseInput {
    let id = Uuid::now_v7().to_string();
    let context = format!("recurring:{id}");
    CreateRecurringExpenseInput {
        id,
        name: encrypted(&context, "name", 'd'),
        category: ExpenseCategory::OttSubscriptions,
        vendor: Some(encrypted(&context, "vendor", 'e')),
        amount_minor,
        currency: "KRW".to_owned(),
        payment_method_fingerprint: Some(digest('f')),
        start_date: NaiveDate::from_ymd_opt(2027, 1, 31).expect("valid date"),
        end_date: None,
        memo: None,
        reminder_days: 7,
        amount_kind: RecurringAmountKind::Fixed,
        interval_months: 1,
        due_rule: RecurringDueRule::SpecificDay,
        due_day: Some(31),
        status: RecurringExpenseStatus::Active,
    }
}

fn recurring_candidate_import(
    source_fingerprint: &str,
    stable_key: &str,
    date: NaiveDate,
    digest_marker: char,
) -> NormalizedExpenseImport {
    let mut purchase = row(
        source_fingerprint,
        stable_key,
        1,
        date,
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        10_000,
        Some(ExpenseCategory::OttSubscriptions),
        digest_marker,
    );
    let context = format!("{source_fingerprint}:{stable_key}");
    purchase.merchant = Some(encrypted(&context, "merchant", 'e'));
    NormalizedExpenseImport {
        adapter: ExpenseImportAdapter::KbCardUsageV1,
        source_kind: ExpenseSourceKind::Card,
        source_fingerprint: source_fingerprint.to_owned(),
        file_sha256: digest(digest_marker),
        normalized_sha256: digest(digest_marker),
        coverage_start: date,
        coverage_end: date,
        rejected_count: 0,
        rows: vec![purchase],
    }
}

#[test]
fn expense_key_initialization_is_allowed_only_before_any_ledger_state() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let initial = core.expense_key_initialization_status()?;
    assert!(initial.ledger_empty);
    assert!(!initial.key_initialized);
    assert!(initial.key_initialization_allowed);

    let probe = ExpenseCryptoProbe {
        key_version: 1,
        nonce: "nonce-for-expense-key-guard".to_owned(),
        ciphertext: "ciphertext-for-expense-key-guard".to_owned(),
        aad: "tm-expense:v1:crypto-probe:value".to_owned(),
    };
    core.initialize_expense_crypto_probe_if_ledger_empty(probe)?;
    let initialized = core.expense_key_initialization_status()?;
    assert!(initialized.ledger_empty);
    assert!(initialized.key_initialized);
    assert!(!initialized.key_initialization_allowed);

    let (_temporary_with_preview, core_with_preview) = fixture()?;
    core_with_preview.preview_expense_import(&july_import())?;
    let after_preview = core_with_preview.expense_key_initialization_status()?;
    assert!(!after_preview.ledger_empty);
    assert!(!after_preview.key_initialized);
    assert!(!after_preview.key_initialization_allowed);
    assert!(matches!(
        core_with_preview.initialize_expense_crypto_probe_if_ledger_empty(ExpenseCryptoProbe {
            key_version: 1,
            nonce: "nonce-after-preview".to_owned(),
            ciphertext: "ciphertext-after-preview".to_owned(),
            aad: "tm-expense:v1:crypto-probe:value".to_owned(),
        }),
        Err(Error::Conflict(_))
    ));
    Ok(())
}

#[test]
fn schema_fifteen_imports_reconciles_and_summarizes_without_plaintext() -> Result<()> {
    let (_temporary, core) = fixture()?;
    assert_eq!(core.health()?.schema_version, 15);
    let probe = ExpenseCryptoProbe {
        key_version: 1,
        nonce: "nonce-for-expense-key-probe".to_owned(),
        ciphertext: "ciphertext-for-expense-key-probe".to_owned(),
        aad: "tm-expense:v1:crypto-probe:value".to_owned(),
    };
    assert_eq!(core.initialize_expense_crypto_probe(probe.clone())?, probe);
    assert_eq!(core.expense_crypto_probe()?, Some(probe));

    let input = july_import();
    let preview = core.preview_expense_import(&input)?;
    assert_eq!(preview.new_count, 7);
    assert_eq!(preview.duplicate_count, 0);
    assert_eq!(preview.settlement_candidate_count, 3);
    assert_eq!(preview.unconfirmed_count, 1);

    let imported = core.import_expenses(&preview.session_id, input.clone())?;
    assert_eq!(imported.new_count, 7);
    assert_eq!(imported.excluded_count, 1);
    assert_eq!(imported.review_count, 2);
    assert!(!imported.idempotent_replay);
    let replay_preview = core.preview_expense_import(&input)?;
    let replay = core.import_expenses(&replay_preview.session_id, input.clone())?;
    assert!(replay.idempotent_replay);
    assert_eq!(replay.batch_id, imported.batch_id);
    let preview = core.preview_expense_import(&input)?;
    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.duplicate_count, 7);

    let month = NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid date");
    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    let purchase = transactions
        .items
        .iter()
        .find(|transaction| transaction.kind == ExpenseEventKind::Purchase)
        .expect("purchase transaction");
    let expected_crypto_context = format!("{}:purchase", digest('a'));
    assert_eq!(
        purchase.crypto_context.as_deref(),
        Some(expected_crypto_context.as_str())
    );
    assert!(
        serde_json::to_value(purchase)?
            .get("cryptoContext")
            .is_none()
    );
    let summary = core.get_expense_month_summary(month)?;
    assert_eq!(summary.status, ExpenseReportStatus::Provisional);
    let krw = summary
        .currencies
        .iter()
        .find(|item| item.currency == "KRW")
        .expect("KRW summary");
    assert_eq!(krw.gross_purchase_minor, 10_000);
    assert_eq!(krw.refunds_minor, 2_000);
    assert_eq!(krw.settlement_received_minor, 3_000);
    assert_eq!(krw.settlement_sent_minor, 1_000);
    assert_eq!(krw.fees_minor, 100);
    assert_eq!(krw.net_personal_spend_minor, 6_100);
    assert_eq!(krw.unconfirmed_outflow_minor, 500);

    let reviews = core.list_expense_reviews(ExpenseReviewFilter {
        month_start: Some(month),
        status: Some(ExpenseReviewStatus::Pending),
        cursor: None,
        limit: 50,
    })?;
    assert_eq!(reviews.items.len(), 2);
    let review = reviews
        .items
        .iter()
        .find(|review| review.reason == ExpenseReviewReason::UnknownP2p)
        .expect("unknown P2P review");
    for forbidden_kind in [
        ExpenseEventKind::UnknownP2p,
        ExpenseEventKind::ExternalTransfer,
        ExpenseEventKind::ManualRecurring,
    ] {
        assert!(matches!(
            core.resolve_expense_review(
                &review.id,
                ResolveExpenseReviewInput {
                    expected_version: review.version,
                    kind: forbidden_kind,
                    category: ExpenseCategory::Other,
                    duplicate_of_event_id: None,
                    related_event_id: None,
                    personal_amount_minor: None,
                    create_rule: false,
                },
            ),
            Err(Error::InvalidInput(_))
        ));
    }
    core.resolve_expense_review(
        &review.id,
        ResolveExpenseReviewInput {
            expected_version: review.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Food,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;
    let resolved = core.get_expense_month_summary(month)?;
    assert_eq!(resolved.status, ExpenseReportStatus::Confirmed);
    assert_eq!(resolved.completeness.pending_review_count, 0);
    assert_eq!(resolved.currencies[0].net_personal_spend_minor, 6_600);
    Ok(())
}

#[test]
fn preview_receipts_are_atomic_single_use_and_bind_the_redacted_content() -> Result<()> {
    let (temporary, core) = fixture()?;
    let input = july_import();
    let preview_request = ExpenseMutationRequest {
        idempotency_key: "preview-july-file".to_owned(),
        actor: "windows-admin".to_owned(),
        command: ExpenseMutationCommand::PreviewImport(redacted_preview(&input)),
    };
    let first = core.execute_expense_mutation(preview_request.clone())?;
    assert!(!first.replayed);
    let first_preview: ExpenseImportPreview = serde_json::from_value(first.value)?;
    let replay = core.execute_expense_mutation(preview_request.clone())?;
    assert!(replay.replayed);
    let replay_preview: ExpenseImportPreview = serde_json::from_value(replay.value)?;
    assert_eq!(first_preview.session_id, replay_preview.session_id);

    let mut conflicting_preview = redacted_preview(&input);
    conflicting_preview.rows[0].amount_minor += 1;
    assert!(matches!(
        core.execute_expense_mutation(ExpenseMutationRequest {
            idempotency_key: preview_request.idempotency_key,
            actor: "windows-admin".to_owned(),
            command: ExpenseMutationCommand::PreviewImport(conflicting_preview),
        }),
        Err(Error::Conflict(_))
    ));

    let mut changed_after_preview = input.clone();
    changed_after_preview.rows[0]
        .merchant
        .as_mut()
        .expect("synthetic merchant")
        .blind_index = digest('z');
    assert!(matches!(
        core.import_expenses(&first_preview.session_id, changed_after_preview),
        Err(Error::Conflict(_))
    ));

    let import_request = ExpenseMutationRequest {
        idempotency_key: "commit-july-file".to_owned(),
        actor: "windows-admin".to_owned(),
        command: ExpenseMutationCommand::Import {
            preview_session_id: first_preview.session_id.clone(),
            input: input.clone(),
        },
    };
    let committed = core.execute_expense_mutation(import_request.clone())?;
    assert!(!committed.replayed);
    let committed_replay = core.execute_expense_mutation(import_request)?;
    assert!(committed_replay.replayed);
    assert_eq!(committed.value, committed_replay.value);
    assert!(matches!(
        core.import_expenses(&first_preview.session_id, input.clone()),
        Err(Error::Conflict(_))
    ));

    let expired_preview = core.preview_expense_import(&input)?;
    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    connection.execute(
        "UPDATE expense_import_preview_sessions
         SET created_at = '1999-01-01T00:00:00.000Z',
             expires_at = '2000-01-01T00:00:00.000Z'
         WHERE id = ?1",
        [&expired_preview.session_id],
    )?;
    drop(connection);
    assert!(matches!(
        core.import_expenses(&expired_preview.session_id, input),
        Err(Error::Conflict(_))
    ));
    Ok(())
}

#[test]
fn expense_amounts_stay_within_the_json_safe_integer_boundary() -> Result<()> {
    const MAX_SAFE_MINOR: i64 = 9_007_199_254_740_991;
    let (_temporary, core) = fixture()?;
    let source_fingerprint = digest('5');
    let date = NaiveDate::from_ymd_opt(2027, 12, 1).expect("valid date");
    let valid = NormalizedExpenseImport {
        adapter: ExpenseImportAdapter::KbCardUsageV1,
        source_kind: ExpenseSourceKind::Card,
        source_fingerprint: source_fingerprint.clone(),
        file_sha256: digest('1'),
        normalized_sha256: digest('1'),
        coverage_start: date,
        coverage_end: date,
        rejected_count: 0,
        rows: vec![row(
            &source_fingerprint,
            "max-safe",
            1,
            date,
            ExpenseEventKind::Purchase,
            ExpenseDirection::Debit,
            MAX_SAFE_MINOR,
            Some(ExpenseCategory::Other),
            '1',
        )],
    };
    let valid_preview = core.preview_expense_import(&valid)?;
    core.import_expenses(&valid_preview.session_id, valid.clone())?;
    let mut aggregate_overflow = valid.clone();
    aggregate_overflow.file_sha256 = digest('4');
    aggregate_overflow.normalized_sha256 = digest('4');
    aggregate_overflow.rows[0] = row(
        &source_fingerprint,
        "second-max-safe",
        1,
        date,
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        MAX_SAFE_MINOR,
        Some(ExpenseCategory::Other),
        '4',
    );
    let overflow_preview = core.preview_expense_import(&aggregate_overflow)?;
    assert!(matches!(
        core.import_expenses(&overflow_preview.session_id, aggregate_overflow),
        Err(Error::Invariant(_))
    ));
    assert_eq!(
        core.list_expense_transactions(ExpenseTransactionFilter {
            month_start: date,
            cursor: None,
            limit: 50,
        })?
        .items
        .len(),
        1
    );
    let mut invalid = valid.clone();
    invalid.file_sha256 = digest('2');
    invalid.normalized_sha256 = digest('2');
    invalid.rows[0] = row(
        &source_fingerprint,
        "unsafe",
        1,
        date,
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        MAX_SAFE_MINOR + 1,
        Some(ExpenseCategory::Other),
        '2',
    );
    assert!(matches!(
        core.preview_expense_import(&invalid),
        Err(Error::InvalidInput(_))
    ));

    let mut recurring = recurring_input(MAX_SAFE_MINOR);
    recurring.start_date = date;
    core.create_recurring_expense(recurring.clone())?;
    assert!(core.has_encrypted_expense_payloads()?);
    assert!(matches!(
        core.initialize_expense_crypto_probe_if_ledger_empty(ExpenseCryptoProbe {
            key_version: 1,
            nonce: "nonce-for-late-probe".to_owned(),
            ciphertext: "ciphertext-for-late-probe".to_owned(),
            aad: "tm-expense:v1:crypto-probe:value".to_owned(),
        }),
        Err(Error::Conflict(_))
    ));
    recurring.amount_minor = MAX_SAFE_MINOR + 1;
    assert!(matches!(
        core.create_recurring_expense(recurring),
        Err(Error::InvalidInput(_))
    ));

    let reversed = ExpenseImportPreviewInput {
        adapter: ExpenseImportAdapter::KbCardUsageV1,
        source_kind: ExpenseSourceKind::Card,
        source_fingerprint,
        file_sha256: digest('3'),
        normalized_sha256: digest('3'),
        coverage_start: NaiveDate::from_ymd_opt(2027, 12, 31).expect("valid date"),
        coverage_end: date,
        rejected_count: 1,
        rows: Vec::new(),
    };
    assert!(matches!(
        core.preview_expense_import_redacted(&reversed),
        Err(Error::InvalidInput(_))
    ));

    let mut transplanted = recurring_input(10_000);
    transplanted.name.aad = expense_text_aad("recurring:another-record", "name");
    assert!(matches!(
        core.create_recurring_expense(transplanted),
        Err(Error::InvalidInput(_))
    ));
    Ok(())
}

#[test]
fn excluded_transactions_can_be_overridden_and_month_snapshots_only_change_on_mutation()
-> Result<()> {
    let (temporary, core) = fixture()?;
    let month = NaiveDate::from_ymd_opt(2027, 10, 1).expect("valid date");
    let month_end = NaiveDate::from_ymd_opt(2027, 10, 31).expect("valid date");
    let source_fingerprint = digest('a');
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('b'),
            normalized_sha256: digest('c'),
            coverage_start: month,
            coverage_end: month_end,
            rejected_count: 0,
            rows: vec![row(
                &source_fingerprint,
                "override-card-payment",
                1,
                NaiveDate::from_ymd_opt(2027, 10, 8).expect("valid date"),
                ExpenseEventKind::CardPayment,
                ExpenseDirection::Debit,
                12_000,
                None,
                'd',
            )],
        },
    )?;
    let excluded = core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start: month,
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .next()
        .expect("excluded card payment");
    assert_eq!(excluded.status, tm_core::ExpenseEventStatus::Excluded);

    let database_path = TmHome::new(temporary.path()).database_path();
    let before: (String, u64, String) = Connection::open(&database_path)?.query_row(
        "SELECT aggregate_sha256, version, generated_at
         FROM expense_month_reports WHERE month_start = ?1 AND currency = 'KRW'",
        [month],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    core.get_expense_month_summary(month)?;
    core.get_expense_month_summary(month)?;
    let after_reads: (String, u64, String) = Connection::open(&database_path)?.query_row(
        "SELECT aggregate_sha256, version, generated_at
         FROM expense_month_reports WHERE month_start = ?1 AND currency = 'KRW'",
        [month],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(before, after_reads);

    let request = ExpenseMutationRequest {
        idempotency_key: "override-card-payment-once".to_owned(),
        actor: "desktop-user".to_owned(),
        command: ExpenseMutationCommand::OverrideTransaction {
            event_id: excluded.id.clone(),
            input: OverrideExpenseTransactionInput {
                expected_version: excluded.version,
                kind: ExpenseEventKind::Purchase,
                category: ExpenseCategory::Other,
                duplicate_of_event_id: None,
                related_event_id: None,
                personal_amount_minor: None,
                clear_personal_amount: false,
                clear_related_event: false,
                create_rule: false,
            },
        },
    };
    let first = core.execute_expense_mutation(request.clone())?;
    assert!(!first.replayed);
    let replay = core.execute_expense_mutation(request)?;
    assert!(replay.replayed);
    let overridden: tm_core::ExpenseTransaction = serde_json::from_value(first.value)?;
    assert_eq!(overridden.status, tm_core::ExpenseEventStatus::Confirmed);
    assert_eq!(overridden.kind, ExpenseEventKind::Purchase);
    assert_eq!(
        core.get_expense_month_summary(month)?.currencies[0].gross_purchase_minor,
        12_000
    );
    let after_override: (String, u64) = Connection::open(&database_path)?.query_row(
        "SELECT aggregate_sha256, version
         FROM expense_month_reports WHERE month_start = ?1 AND currency = 'KRW'",
        [month],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_ne!(after_override.0, before.0);
    assert_eq!(after_override.1, before.1 + 1);
    assert!(matches!(
        core.override_expense_transaction(
            &excluded.id,
            OverrideExpenseTransactionInput {
                expected_version: excluded.version,
                kind: ExpenseEventKind::Purchase,
                category: ExpenseCategory::Other,
                duplicate_of_event_id: None,
                related_event_id: None,
                personal_amount_minor: None,
                clear_personal_amount: false,
                clear_related_event: false,
                create_rule: false,
            },
        ),
        Err(Error::Conflict(_))
    ));
    Ok(())
}

#[test]
fn schema_fifteen_semantic_validation_rejects_ledger_corruption() -> Result<()> {
    {
        let (temporary, core) = fixture()?;
        let input = july_import();
        preview_and_import(&core, input)?;
        let database_path = TmHome::new(temporary.path()).database_path();
        let connection = Connection::open(&database_path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = OFF;
             DROP TRIGGER expense_postings_immutable_delete;
             DELETE FROM expense_postings
             WHERE id = (SELECT id FROM expense_postings ORDER BY id LIMIT 1);
             CREATE TRIGGER expense_postings_immutable_delete
             BEFORE DELETE ON expense_postings
             BEGIN
                 SELECT RAISE(ABORT, 'expense postings are immutable');
             END;",
        )?;
        drop(connection);
        drop(core);
        assert!(matches!(
            TmCore::open(TmHome::new(temporary.path())),
            Err(Error::Invariant(_))
        ));
    }

    {
        let (temporary, core) = fixture()?;
        core.initialize_expense_crypto_probe(ExpenseCryptoProbe {
            key_version: 1,
            nonce: "nonce-for-expense-key-probe".to_owned(),
            ciphertext: "ciphertext-for-expense-key-probe".to_owned(),
            aad: "tm-expense:v1:crypto-probe:value".to_owned(),
        })?;
        let database_path = TmHome::new(temporary.path()).database_path();
        let connection = Connection::open(&database_path)?;
        connection.execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE expense_crypto_metadata SET key_version = 2;",
        )?;
        drop(connection);
        drop(core);
        assert!(matches!(
            TmCore::open(TmHome::new(temporary.path())),
            Err(Error::Invariant(_))
        ));
    }

    {
        let (temporary, core) = fixture()?;
        preview_and_import(&core, july_import())?;
        let database_path = TmHome::new(temporary.path()).database_path();
        let connection = Connection::open(&database_path)?;
        connection.execute_batch(
            "DROP TRIGGER expense_event_postings_immutable_update;
             UPDATE expense_event_postings SET posting_role = 'supporting'
             WHERE rowid = (SELECT rowid FROM expense_event_postings ORDER BY rowid LIMIT 1);
             CREATE TRIGGER expense_event_postings_immutable_update
             BEFORE UPDATE ON expense_event_postings
             BEGIN
                 SELECT RAISE(ABORT, 'expense event posting links are immutable');
             END;",
        )?;
        drop(connection);
        drop(core);
        assert!(matches!(
            TmCore::open(TmHome::new(temporary.path())),
            Err(Error::Invariant(_))
        ));
    }
    Ok(())
}

#[test]
fn schema_fifteen_semantics_reject_rule_unique_index_contract_drift() -> Result<()> {
    for replacement in [
        "DROP INDEX idx_expense_rules_classification_unique;
         CREATE UNIQUE INDEX idx_expense_rules_classification_unique
             ON expense_rules(merchant_blind_index)
             WHERE rule_kind = 'classification';",
        "DROP INDEX idx_expense_rules_recurring_match_unique;
         CREATE UNIQUE INDEX idx_expense_rules_recurring_match_unique
             ON expense_rules(
                 recurring_expense_id,
                 merchant_blind_index,
                 coalesce(payment_method_fingerprint, '')
             )
             WHERE rule_kind = 'recurring_match' AND merchant_blind_index <> '';",
    ] {
        let (temporary, core) = fixture()?;
        let database_path = TmHome::new(temporary.path()).database_path();
        let connection = Connection::open(&database_path)?;
        connection.execute_batch(replacement)?;
        drop(connection);
        drop(core);

        assert!(matches!(
            TmCore::open(TmHome::new(temporary.path())),
            Err(Error::Invariant(_))
        ));
    }
    Ok(())
}

#[test]
fn restore_accepts_the_same_crypto_probe_and_rejects_a_conflicting_key_probe() -> Result<()> {
    let (temporary, core) = fixture()?;
    core.initialize_expense_crypto_probe(ExpenseCryptoProbe {
        key_version: 1,
        nonce: "nonce-for-expense-key-probe-a".to_owned(),
        ciphertext: "ciphertext-for-expense-key-probe-a".to_owned(),
        aad: "tm-expense:v1:crypto-probe:value".to_owned(),
    })?;
    let backup = core.create_backup()?;
    core.restore_backup(&backup.path)?;
    preview_and_import(&core, july_import())?;

    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    connection.execute(
        "UPDATE expense_crypto_metadata
         SET nonce = 'nonce-for-expense-key-probe-b',
             ciphertext = 'ciphertext-for-expense-key-probe-b'",
        [],
    )?;
    drop(connection);
    assert!(matches!(
        core.restore_backup(&backup.path),
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        core.expense_crypto_probe()?,
        Some(ExpenseCryptoProbe {
            key_version: 1,
            nonce: "nonce-for-expense-key-probe-b".to_owned(),
            ciphertext: "ciphertext-for-expense-key-probe-b".to_owned(),
            aad: "tm-expense:v1:crypto-probe:value".to_owned(),
        })
    );
    assert_eq!(
        core.list_expense_transactions(ExpenseTransactionFilter {
            month_start: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid date"),
            cursor: None,
            limit: 50,
        })?
        .items
        .len(),
        7
    );
    Ok(())
}

#[test]
fn restore_merge_forward_preserves_latest_mutable_expense_projections() -> Result<()> {
    let (temporary, core) = fixture()?;
    let month = NaiveDate::from_ymd_opt(2026, 9, 1).expect("valid date");
    let source_fingerprint = digest('m');
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('n'),
            normalized_sha256: digest('n'),
            coverage_start: month,
            coverage_end: month,
            rejected_count: 0,
            rows: vec![row(
                &source_fingerprint,
                "mutable-restore",
                1,
                month,
                ExpenseEventKind::UnknownP2p,
                ExpenseDirection::Debit,
                6_000,
                None,
                'o',
            )],
        },
    )?;
    let review = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 20,
        })?
        .items
        .into_iter()
        .find(|candidate| candidate.reason == ExpenseReviewReason::UnknownP2p)
        .expect("unknown P2P review");
    let resolved = core.resolve_expense_review(
        &review.id,
        ResolveExpenseReviewInput {
            expected_version: review.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Food,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: Some(6_000),
            create_rule: true,
        },
    )?;
    core.claim_expense_report_attempt_for_report(month, month, "restore-ai-attempt")?;
    let old_backup = core.create_backup()?;

    let updated = core.override_expense_transaction(
        &resolved.transaction.id,
        OverrideExpenseTransactionInput {
            expected_version: resolved.transaction.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Shopping,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            clear_personal_amount: true,
            clear_related_event: false,
            create_rule: true,
        },
    )?;
    assert_eq!(updated.personal_amount_minor, None);
    let staged = SaveExpenseReportInput {
        month_start: month,
        aggregate_sha256: digest('p'),
        prompt_version: "expense-v1".to_owned(),
        model: "gpt-5.4-nano-2026-03-17".to_owned(),
        title: "Latest staged result".to_owned(),
        summary: "Must survive restoration of an older backup".to_owned(),
        facts: vec![ExpenseReportFact {
            fact_id: "currency:KRW:net_personal_spend".to_owned(),
            metric: "net_personal_spend".to_owned(),
            currency: "KRW".to_owned(),
            amount_minor: 6_000,
        }],
        observations: vec![ExpenseReportObservation {
            text: "Grounded".to_owned(),
            fact_ids: vec!["currency:KRW:net_personal_spend".to_owned()],
        }],
        alerts: Vec::new(),
        next_month_checks: Vec::new(),
        input_tokens: 10,
        cached_input_tokens: 2,
        output_tokens: 5,
        total_tokens: 15,
        cost_microusd: 100,
        latency_ms: 20,
    };
    core.stage_expense_report_attempt_result("restore-ai-attempt", &staged)?;

    core.restore_backup(&old_backup.path)?;
    let restored = core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start: month,
            cursor: None,
            limit: 20,
        })?
        .items
        .into_iter()
        .find(|candidate| candidate.id == updated.id)
        .expect("restored transaction");
    assert_eq!(restored.category, ExpenseCategory::Shopping);
    assert_eq!(restored.personal_amount_minor, None);
    assert_eq!(
        core.get_staged_expense_report_attempt_result("restore-ai-attempt")?,
        Some(staged)
    );
    let database_path = TmHome::new(temporary.path()).database_path();
    let connection = Connection::open(database_path)?;
    let saved_rule: (String, i64) = connection.query_row(
        "SELECT category, count(*)
         FROM expense_rules
         WHERE rule_kind = 'classification'
         GROUP BY category",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(saved_rule, ("shopping".to_owned(), 1));
    let allocation_count: i64 = connection.query_row(
        "SELECT count(*) FROM expense_allocations WHERE event_id = ?1",
        [&updated.id],
        |row| row.get(0),
    )?;
    assert_eq!(allocation_count, 0);
    Ok(())
}

#[test]
fn recurring_dates_clamp_and_versions_preserve_prior_months() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let item = core.create_recurring_expense(recurring_input(10_000))?;
    let february = NaiveDate::from_ymd_opt(2027, 2, 1).expect("valid date");
    let february_occurrences = core.recurring_expense_occurrences(february)?;
    assert_eq!(february_occurrences.len(), 1);
    assert_eq!(
        february_occurrences[0].due_date,
        NaiveDate::from_ymd_opt(2027, 2, 28).expect("valid date")
    );

    let updated = core.update_recurring_expense(
        &item.id,
        UpdateRecurringExpenseInput {
            expected_version: item.version,
            effective_from_month: NaiveDate::from_ymd_opt(2027, 3, 1).expect("valid date"),
            name: item.name.clone(),
            category: item.category,
            vendor: item.vendor.clone(),
            amount_minor: 20_000,
            currency: "USD".to_owned(),
            payment_method_fingerprint: item.payment_method_fingerprint.clone(),
            start_date: item.start_date,
            end_date: item.end_date,
            memo: item.memo.clone(),
            reminder_days: item.reminder_days,
            amount_kind: item.amount_kind,
            interval_months: item.interval_months,
            due_rule: item.due_rule,
            due_day: item.due_day,
            status: item.status,
            auto_match_enabled: item.auto_match_enabled,
        },
    )?;
    assert_eq!(updated.version, 2);
    assert_eq!(updated.amount_minor, 10_000);
    assert_eq!(updated.currency, "KRW");
    let current_view = core
        .list_recurring_expenses()?
        .into_iter()
        .find(|candidate| candidate.id == item.id)
        .expect("current recurring view");
    assert_eq!(current_view.amount_minor, 10_000);
    assert_eq!(current_view.currency, "KRW");
    assert_eq!(current_view.version, 2);
    assert_eq!(
        core.recurring_expense_occurrences(february)?[0].expected_amount_minor,
        10_000
    );
    let march = NaiveDate::from_ymd_opt(2027, 3, 1).expect("valid date");
    assert_eq!(
        core.recurring_expense_occurrences(march)?[0].expected_amount_minor,
        20_000
    );
    assert_eq!(
        core.recurring_expense_occurrences(february)?[0].currency,
        "KRW"
    );
    assert_eq!(
        core.recurring_expense_occurrences(march)?[0].currency,
        "USD"
    );
    core.delete_recurring_expense(&item.id, updated.version)?;
    assert!(
        core.list_recurring_expenses()?
            .iter()
            .all(|candidate| candidate.id != item.id)
    );
    assert!(
        core.recurring_expense_occurrences(march)?
            .iter()
            .all(|occurrence| occurrence.recurring_expense_id != item.id)
    );

    let current_month = NaiveDate::from_ymd_opt(2026, 8, 1).expect("current test month");
    let mut current_input = recurring_input(30_000);
    current_input.start_date = current_month;
    current_input.due_day = Some(15);
    let current_item = core.create_recurring_expense(current_input)?;
    let effective_now = core.update_recurring_expense(
        &current_item.id,
        UpdateRecurringExpenseInput {
            expected_version: current_item.version,
            effective_from_month: current_month,
            name: current_item.name.clone(),
            category: current_item.category,
            vendor: current_item.vendor.clone(),
            amount_minor: 35_000,
            currency: "USD".to_owned(),
            payment_method_fingerprint: current_item.payment_method_fingerprint.clone(),
            start_date: current_item.start_date,
            end_date: current_item.end_date,
            memo: current_item.memo.clone(),
            reminder_days: current_item.reminder_days,
            amount_kind: current_item.amount_kind,
            interval_months: current_item.interval_months,
            due_rule: current_item.due_rule,
            due_day: current_item.due_day,
            status: current_item.status,
            auto_match_enabled: current_item.auto_match_enabled,
        },
    )?;
    assert_eq!(effective_now.amount_minor, 35_000);
    assert_eq!(effective_now.currency, "USD");
    let reloaded = core
        .list_recurring_expenses()?
        .into_iter()
        .find(|candidate| candidate.id == current_item.id)
        .expect("current effective recurring item");
    assert_eq!(reloaded, effective_now);
    Ok(())
}

#[test]
fn recurring_schedule_handles_month_edges_leap_years_intervals_and_statuses() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let february_2027 = NaiveDate::from_ymd_opt(2027, 2, 1).expect("valid date");
    for due_day in [28_u8, 29, 30, 31] {
        let mut input = recurring_input(10_000 + i64::from(due_day));
        input.start_date = NaiveDate::from_ymd_opt(2027, 1, 1).expect("valid date");
        input.due_day = Some(due_day);
        let item = core.create_recurring_expense(input)?;
        let occurrence = core
            .recurring_expense_occurrences(february_2027)?
            .into_iter()
            .find(|occurrence| occurrence.recurring_expense_id == item.id)
            .expect("February occurrence");
        assert_eq!(occurrence.due_date.day(), 28);
    }

    let mut month_end_input = recurring_input(20_000);
    month_end_input.start_date = NaiveDate::from_ymd_opt(2027, 4, 1).expect("valid date");
    month_end_input.due_rule = RecurringDueRule::LastDay;
    month_end_input.due_day = None;
    let month_end_item = core.create_recurring_expense(month_end_input)?;
    let april = NaiveDate::from_ymd_opt(2027, 4, 1).expect("valid date");
    let april_occurrence = core
        .recurring_expense_occurrences(april)?
        .into_iter()
        .find(|occurrence| occurrence.recurring_expense_id == month_end_item.id)
        .expect("last-day occurrence");
    assert_eq!(april_occurrence.due_date.day(), 30);

    let mut first_day_input = recurring_input(21_000);
    first_day_input.start_date = april;
    first_day_input.due_rule = RecurringDueRule::FirstDay;
    first_day_input.due_day = None;
    let first_day_item = core.create_recurring_expense(first_day_input)?;
    let first_day_occurrence = core
        .recurring_expense_occurrences(april)?
        .into_iter()
        .find(|occurrence| occurrence.recurring_expense_id == first_day_item.id)
        .expect("first-day occurrence");
    assert_eq!(first_day_occurrence.due_date, april);

    let mut leap_input = recurring_input(22_000);
    leap_input.start_date = NaiveDate::from_ymd_opt(2028, 2, 29).expect("valid leap date");
    leap_input.interval_months = 12;
    leap_input.due_day = Some(29);
    let leap_item = core.create_recurring_expense(leap_input)?;
    let non_leap_february = NaiveDate::from_ymd_opt(2029, 2, 1).expect("valid date");
    let non_leap_occurrence = core
        .recurring_expense_occurrences(non_leap_february)?
        .into_iter()
        .find(|occurrence| occurrence.recurring_expense_id == leap_item.id)
        .expect("annual leap-day occurrence");
    assert_eq!(
        non_leap_occurrence.due_date,
        NaiveDate::from_ymd_opt(2029, 2, 28).expect("valid date")
    );

    for interval_months in [2_u8, 3, 6, 12] {
        let mut input = recurring_input(30_000 + i64::from(interval_months));
        input.start_date = NaiveDate::from_ymd_opt(2027, 1, 1).expect("valid date");
        input.interval_months = interval_months;
        input.due_day = Some(1);
        let item = core.create_recurring_expense(input)?;
        let skipped_month = NaiveDate::from_ymd_opt(2027, 2, 1).expect("valid date");
        assert!(
            core.recurring_expense_occurrences(skipped_month)?
                .iter()
                .all(|occurrence| occurrence.recurring_expense_id != item.id)
        );
        let due_month = NaiveDate::from_ymd_opt(
            if interval_months == 12 { 2028 } else { 2027 },
            if interval_months == 12 {
                1
            } else {
                1 + u32::from(interval_months)
            },
            1,
        )
        .expect("valid interval month");
        assert!(
            core.recurring_expense_occurrences(due_month)?
                .iter()
                .any(|occurrence| occurrence.recurring_expense_id == item.id)
        );
    }

    for status in [
        RecurringExpenseStatus::Paused,
        RecurringExpenseStatus::Ended,
    ] {
        let mut input = recurring_input(40_000);
        input.status = status;
        let item = core.create_recurring_expense(input)?;
        assert!(
            core.recurring_expense_occurrences(february_2027)?
                .iter()
                .all(|occurrence| occurrence.recurring_expense_id != item.id)
        );
    }
    Ok(())
}

#[test]
fn matching_an_imported_transaction_atomically_replaces_manual_payment() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.create_recurring_expense(recurring_input(10_000))?;
    let february = NaiveDate::from_ymd_opt(2027, 2, 1).expect("valid date");
    let occurrence = core.recurring_expense_occurrences(february)?[0].clone();
    let paid = core.confirm_recurring_expense_paid(
        &occurrence.occurrence_key,
        ConfirmRecurringPaidInput {
            expected_version: occurrence.version,
            amount_minor: None,
            paid_date: Some(NaiveDate::from_ymd_opt(2027, 2, 28).expect("valid date")),
        },
    )?;
    assert_eq!(paid.status, RecurringOccurrenceStatus::Paid);

    let source_fingerprint = digest('8');
    let imported = NormalizedExpenseImport {
        adapter: ExpenseImportAdapter::KbCardUsageV1,
        source_kind: ExpenseSourceKind::Card,
        source_fingerprint: source_fingerprint.clone(),
        file_sha256: digest('9'),
        normalized_sha256: digest('0'),
        coverage_start: february,
        coverage_end: NaiveDate::from_ymd_opt(2027, 2, 28).expect("valid date"),
        rejected_count: 0,
        rows: vec![row(
            &source_fingerprint,
            "actual-recurring-charge",
            1,
            NaiveDate::from_ymd_opt(2027, 2, 27).expect("valid date"),
            ExpenseEventKind::Purchase,
            ExpenseDirection::Debit,
            10_000,
            Some(ExpenseCategory::OttSubscriptions),
            'a',
        )],
    };
    preview_and_import(&core, imported)?;
    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: february,
        cursor: None,
        limit: 50,
    })?;
    let actual = transactions
        .items
        .iter()
        .find(|transaction| !transaction.is_provisional)
        .expect("imported transaction");
    let matched = core.match_recurring_expense(
        &occurrence.occurrence_key,
        MatchRecurringExpenseInput {
            expected_version: occurrence.version,
            event_id: actual.id.clone(),
            enable_future_auto_match: false,
        },
    )?;
    assert_eq!(matched.status, RecurringOccurrenceStatus::Matched);
    assert_eq!(matched.actual_event_id.as_deref(), Some(actual.id.as_str()));
    let after = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: february,
        cursor: None,
        limit: 50,
    })?;
    assert!(after.items.iter().any(|transaction| {
        transaction.is_provisional
            && transaction.status == tm_core::ExpenseEventStatus::Excluded
            && transaction.exclusion_reason.as_deref() == Some("manual_replaced")
    }));
    assert_eq!(
        core.get_expense_month_summary(february)?.currencies[0].gross_purchase_minor,
        10_000
    );
    Ok(())
}

#[test]
fn first_manual_match_learns_a_safe_rule_and_auto_matches_the_next_occurrence() -> Result<()> {
    let (temporary, core) = fixture()?;
    let mut recurring = recurring_input(10_000);
    recurring.vendor = None;
    recurring.payment_method_fingerprint = None;
    recurring.start_date = NaiveDate::from_ymd_opt(2027, 3, 1).expect("valid date");
    recurring.due_day = Some(15);
    let item = core.create_recurring_expense(recurring)?;
    let march = NaiveDate::from_ymd_opt(2027, 3, 1).expect("valid date");
    let march_occurrence = core
        .recurring_expense_occurrences(march)?
        .into_iter()
        .find(|occurrence| occurrence.recurring_expense_id == item.id)
        .expect("March occurrence");

    let source_fingerprint = digest('7');
    let mut march_row = row(
        &source_fingerprint,
        "learn-rule-march",
        1,
        NaiveDate::from_ymd_opt(2027, 3, 15).expect("valid date"),
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        10_000,
        Some(ExpenseCategory::OttSubscriptions),
        '1',
    );
    march_row.merchant = Some(encrypted(
        &format!("{source_fingerprint}:learn-rule-march"),
        "merchant",
        'm',
    ));
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbCardUsageV1,
            source_kind: ExpenseSourceKind::Card,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('1'),
            normalized_sha256: digest('1'),
            coverage_start: march,
            coverage_end: NaiveDate::from_ymd_opt(2027, 3, 31).expect("valid date"),
            rejected_count: 0,
            rows: vec![march_row],
        },
    )?;
    let march_event = core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start: march,
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|transaction| transaction.amount_minor == 10_000)
        .expect("March purchase");
    let matched = core.match_recurring_expense(
        &march_occurrence.occurrence_key,
        MatchRecurringExpenseInput {
            expected_version: march_occurrence.version,
            event_id: march_event.id.clone(),
            enable_future_auto_match: true,
        },
    )?;
    assert_eq!(matched.status, RecurringOccurrenceStatus::Matched);

    let learned = core
        .list_recurring_expenses()?
        .into_iter()
        .find(|candidate| candidate.id == item.id)
        .expect("learned recurring item");
    assert!(learned.vendor.is_none());
    assert_eq!(learned.payment_method_fingerprint, Some(digest('f')));
    assert!(learned.auto_match_enabled);
    assert_eq!(learned.version, 2);

    let mut second = recurring_input(20_000);
    second.vendor = None;
    second.payment_method_fingerprint = None;
    second.start_date = march;
    second.due_day = Some(15);
    let second = core.create_recurring_expense(second)?;
    let second_occurrence = core
        .recurring_expense_occurrences(march)?
        .into_iter()
        .find(|occurrence| occurrence.recurring_expense_id == second.id)
        .expect("second March occurrence");
    assert!(matches!(
        core.match_recurring_expense(
            &second_occurrence.occurrence_key,
            MatchRecurringExpenseInput {
                expected_version: second_occurrence.version,
                event_id: march_event.id.clone(),
                enable_future_auto_match: false,
            },
        ),
        Err(Error::Conflict(_))
    ));

    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    let learned_rule: (String, String) = connection.query_row(
        "SELECT merchant_blind_index, payment_method_fingerprint
         FROM expense_rules
         WHERE recurring_expense_id = ?1 AND rule_kind = 'recurring_match'",
        [&item.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(learned_rule, (digest('m'), digest('f')));
    let stored_vendor_key_version: Option<u32> = connection.query_row(
        "SELECT vendor_key_version FROM recurring_expense_items WHERE id = ?1",
        [&item.id],
        |row| row.get(0),
    )?;
    assert!(stored_vendor_key_version.is_none());
    drop(connection);

    let april = NaiveDate::from_ymd_opt(2027, 4, 1).expect("valid date");
    let mut april_row = row(
        &source_fingerprint,
        "learn-rule-april",
        1,
        NaiveDate::from_ymd_opt(2027, 4, 15).expect("valid date"),
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        10_000,
        Some(ExpenseCategory::OttSubscriptions),
        '2',
    );
    april_row.merchant = Some(encrypted(
        &format!("{source_fingerprint}:learn-rule-april"),
        "merchant",
        'm',
    ));
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbCardUsageV1,
            source_kind: ExpenseSourceKind::Card,
            source_fingerprint,
            file_sha256: digest('2'),
            normalized_sha256: digest('2'),
            coverage_start: april,
            coverage_end: NaiveDate::from_ymd_opt(2027, 4, 30).expect("valid date"),
            rejected_count: 0,
            rows: vec![april_row],
        },
    )?;
    let april_occurrence = core
        .recurring_expense_occurrences(april)?
        .into_iter()
        .find(|occurrence| occurrence.recurring_expense_id == item.id)
        .expect("April occurrence");
    assert_eq!(april_occurrence.status, RecurringOccurrenceStatus::Matched);
    assert!(april_occurrence.actual_event_id.is_some());
    assert!(
        core.list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(april),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?
        .items
        .is_empty()
    );
    let revoked = core.update_recurring_expense(
        &learned.id,
        UpdateRecurringExpenseInput {
            expected_version: learned.version,
            effective_from_month: NaiveDate::from_ymd_opt(2026, 8, 1).expect("valid date"),
            name: learned.name.clone(),
            category: learned.category,
            vendor: learned.vendor.clone(),
            amount_minor: learned.amount_minor,
            currency: learned.currency.clone(),
            payment_method_fingerprint: learned.payment_method_fingerprint.clone(),
            start_date: learned.start_date,
            end_date: learned.end_date,
            memo: learned.memo.clone(),
            reminder_days: learned.reminder_days,
            amount_kind: learned.amount_kind,
            interval_months: learned.interval_months,
            due_rule: learned.due_rule,
            due_day: learned.due_day,
            status: learned.status,
            auto_match_enabled: false,
        },
    )?;
    assert!(!revoked.auto_match_enabled);
    assert!(
        !core
            .list_recurring_expenses()?
            .into_iter()
            .find(|candidate| candidate.id == learned.id)
            .expect("revoked recurring item")
            .auto_match_enabled
    );
    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    let rule_count: u32 = connection.query_row(
        "SELECT count(*) FROM expense_rules
         WHERE recurring_expense_id = ?1 AND rule_kind = 'recurring_match'",
        [&learned.id],
        |row| row.get(0),
    )?;
    assert_eq!(rule_count, 0);
    Ok(())
}

#[test]
fn recurring_candidate_amount_ranges_follow_fixed_estimate_and_limit_rules() -> Result<()> {
    for (amount_kind, actual_amount, expected_candidate) in [
        (RecurringAmountKind::Fixed, 10_000, true),
        (RecurringAmountKind::Fixed, 10_001, false),
        (RecurringAmountKind::Estimate, 9_500, true),
        (RecurringAmountKind::Estimate, 10_500, true),
        (RecurringAmountKind::Estimate, 9_499, false),
        (RecurringAmountKind::Estimate, 10_501, false),
        (RecurringAmountKind::Limit, 9_999, true),
        (RecurringAmountKind::Limit, 10_001, false),
    ] {
        let (_temporary, core) = fixture()?;
        let mut recurring = recurring_input(10_000);
        recurring.start_date = NaiveDate::from_ymd_opt(2027, 7, 1).expect("valid date");
        recurring.due_day = Some(15);
        recurring.amount_kind = amount_kind;
        core.create_recurring_expense(recurring)?;

        let source_fingerprint = digest('8');
        let date = NaiveDate::from_ymd_opt(2027, 7, 15).expect("valid date");
        let mut purchase = row(
            &source_fingerprint,
            "amount-range",
            1,
            date,
            ExpenseEventKind::Purchase,
            ExpenseDirection::Debit,
            actual_amount,
            Some(ExpenseCategory::OttSubscriptions),
            '1',
        );
        purchase.merchant = Some(encrypted(
            &format!("{source_fingerprint}:amount-range"),
            "merchant",
            'e',
        ));
        preview_and_import(
            &core,
            NormalizedExpenseImport {
                adapter: ExpenseImportAdapter::KbCardUsageV1,
                source_kind: ExpenseSourceKind::Card,
                source_fingerprint,
                file_sha256: digest('1'),
                normalized_sha256: digest('1'),
                coverage_start: date,
                coverage_end: date,
                rejected_count: 0,
                rows: vec![purchase],
            },
        )?;
        let has_candidate = core
            .list_expense_reviews(ExpenseReviewFilter {
                month_start: Some(NaiveDate::from_ymd_opt(2027, 7, 1).expect("valid date")),
                status: Some(ExpenseReviewStatus::Pending),
                cursor: None,
                limit: 50,
            })?
            .items
            .iter()
            .any(|review| review.reason == ExpenseReviewReason::RecurringMatchCandidate);
        assert_eq!(
            has_candidate, expected_candidate,
            "unexpected candidate result for {amount_kind:?} at {actual_amount}"
        );
    }
    Ok(())
}

#[test]
fn recurring_match_candidates_cross_month_boundaries_within_the_five_day_window() -> Result<()> {
    for (start_date, due_day, transaction_date, marker) in [
        (
            NaiveDate::from_ymd_opt(2027, 8, 1).expect("valid date"),
            1,
            NaiveDate::from_ymd_opt(2027, 7, 30).expect("valid date"),
            '1',
        ),
        (
            NaiveDate::from_ymd_opt(2027, 7, 31).expect("valid date"),
            31,
            NaiveDate::from_ymd_opt(2027, 8, 2).expect("valid date"),
            '2',
        ),
    ] {
        let (_temporary, core) = fixture()?;
        let mut recurring = recurring_input(10_000);
        recurring.start_date = start_date;
        recurring.due_day = Some(due_day);
        let item = core.create_recurring_expense(recurring)?;
        let source_fingerprint = digest(marker);
        preview_and_import(
            &core,
            recurring_candidate_import(
                &source_fingerprint,
                "cross-month-recurring",
                transaction_date,
                marker,
            ),
        )?;
        let transaction_month =
            NaiveDate::from_ymd_opt(transaction_date.year(), transaction_date.month(), 1)
                .expect("valid transaction month");
        let candidates = core
            .list_expense_reviews(ExpenseReviewFilter {
                month_start: Some(transaction_month),
                status: Some(ExpenseReviewStatus::Pending),
                cursor: None,
                limit: 50,
            })?
            .items
            .into_iter()
            .filter(|review| {
                review.reason == ExpenseReviewReason::RecurringMatchCandidate
                    && review.recurring_expense_id.as_deref() == Some(item.id.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 1);
    }
    Ok(())
}

#[test]
fn recurring_match_rejects_credit_refunds_and_accepts_reviewed_debit_transfers() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let mut recurring = recurring_input(10_000);
    recurring.start_date = NaiveDate::from_ymd_opt(2027, 8, 1).expect("valid date");
    recurring.due_day = Some(15);
    let item = core.create_recurring_expense(recurring)?;
    let month = NaiveDate::from_ymd_opt(2027, 8, 1).expect("valid date");
    let occurrence = core
        .recurring_expense_occurrences(month)?
        .into_iter()
        .find(|candidate| candidate.recurring_expense_id == item.id)
        .expect("August occurrence");
    let source_fingerprint = digest('9');
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('1'),
            normalized_sha256: digest('1'),
            coverage_start: month,
            coverage_end: NaiveDate::from_ymd_opt(2027, 8, 31).expect("valid date"),
            rejected_count: 0,
            rows: vec![
                row(
                    &source_fingerprint,
                    "credit-refund",
                    1,
                    NaiveDate::from_ymd_opt(2027, 8, 14).expect("valid date"),
                    ExpenseEventKind::Refund,
                    ExpenseDirection::Credit,
                    10_000,
                    None,
                    '1',
                ),
                row(
                    &source_fingerprint,
                    "debit-transfer",
                    2,
                    NaiveDate::from_ymd_opt(2027, 8, 15).expect("valid date"),
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Debit,
                    10_000,
                    None,
                    '2',
                ),
            ],
        },
    )?;
    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    let refund = transactions
        .items
        .iter()
        .find(|event| event.kind == ExpenseEventKind::Refund)
        .expect("refund");
    assert!(matches!(
        core.match_recurring_expense(
            &occurrence.occurrence_key,
            MatchRecurringExpenseInput {
                expected_version: occurrence.version,
                event_id: refund.id.clone(),
                enable_future_auto_match: false,
            },
        ),
        Err(Error::InvalidInput(_))
    ));
    let transfer = transactions
        .items
        .iter()
        .find(|event| event.kind == ExpenseEventKind::UnknownP2p)
        .expect("debit transfer");
    let matched = core.match_recurring_expense(
        &occurrence.occurrence_key,
        MatchRecurringExpenseInput {
            expected_version: occurrence.version,
            event_id: transfer.id.clone(),
            enable_future_auto_match: false,
        },
    )?;
    assert_eq!(matched.status, RecurringOccurrenceStatus::Matched);
    Ok(())
}

#[test]
fn expense_mutation_receipts_and_ai_attempts_are_durable() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let request = ExpenseMutationRequest {
        idempotency_key: "recurring-create-idempotency".to_owned(),
        actor: "desktop-user".to_owned(),
        command: ExpenseMutationCommand::CreateRecurring(recurring_input(10_000)),
    };
    let first = core.execute_expense_mutation(request.clone())?;
    assert!(!first.replayed);
    let replay = core.execute_expense_mutation(request)?;
    assert!(replay.replayed);
    let first_item: RecurringExpenseItem = serde_json::from_value(first.value)?;
    let replay_item: RecurringExpenseItem = serde_json::from_value(replay.value)?;
    assert_eq!(first_item.id, replay_item.id);

    let month = NaiveDate::from_ymd_opt(2026, 8, 1).expect("valid date");
    let report_month = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date");
    let binding = core.bind_expense_report_request("bound-request", report_month, &digest('6'))?;
    assert!(!binding.replayed);
    assert!(
        core.bind_expense_report_request("bound-request", report_month, &digest('6'))?
            .replayed
    );
    assert!(matches!(
        core.bind_expense_report_request("bound-request", month, &digest('6')),
        Err(Error::Conflict(_))
    ));
    for index in 1..=8 {
        let target_month = if index == 1 { report_month } else { month };
        let claim = core.claim_expense_report_attempt_for_report(
            month,
            target_month,
            &format!("attempt-{index}"),
        )?;
        assert_eq!(claim.attempt_number, index);
        assert_eq!(claim.report_month_start, target_month);
    }
    assert!(matches!(
        core.claim_expense_report_attempt(month, "attempt-9"),
        Err(Error::Conflict(_))
    ));
    let replay = core.claim_expense_report_attempt_for_report(month, report_month, "attempt-1")?;
    assert!(replay.replayed);
    assert_eq!(replay.attempt_number, 1);
    let staged = SaveExpenseReportInput {
        month_start: report_month,
        aggregate_sha256: digest('7'),
        prompt_version: "expense-v1".to_owned(),
        model: "gpt-5.4-nano-2026-03-17".to_owned(),
        title: "Staged result".to_owned(),
        summary: "Aggregate-only recovery payload".to_owned(),
        facts: vec![ExpenseReportFact {
            fact_id: "currency:KRW:net_personal_spend".to_owned(),
            metric: "net_personal_spend".to_owned(),
            currency: "KRW".to_owned(),
            amount_minor: 10_000,
        }],
        observations: vec![ExpenseReportObservation {
            text: "Grounded".to_owned(),
            fact_ids: vec!["currency:KRW:net_personal_spend".to_owned()],
        }],
        alerts: Vec::new(),
        next_month_checks: Vec::new(),
        input_tokens: 10,
        cached_input_tokens: 2,
        output_tokens: 5,
        total_tokens: 15,
        cost_microusd: 100,
        latency_ms: 20,
    };
    assert_eq!(
        core.stage_expense_report_attempt_result("attempt-1", &staged)?,
        staged
    );
    assert_eq!(
        core.get_staged_expense_report_attempt_result("attempt-1")?,
        Some(staged.clone())
    );
    core.stage_expense_report_attempt_result("attempt-1", &staged)?;
    let succeeded =
        core.claim_expense_report_attempt_for_report(month, report_month, "attempt-1")?;
    assert_eq!(
        succeeded.status,
        tm_core::ExpenseReportAttemptStatus::Succeeded
    );
    let mut conflicting = staged;
    conflicting.title = "Different paid result".to_owned();
    assert!(matches!(
        core.stage_expense_report_attempt_result("attempt-1", &conflicting),
        Err(Error::Conflict(_))
    ));
    let failed = core.fail_expense_report_attempt("attempt-2", "timeout")?;
    assert_eq!(failed.status, tm_core::ExpenseReportAttemptStatus::Failed);
    assert_eq!(failed.failure_code.as_deref(), Some("timeout"));
    assert!(!failed.replayed);
    assert!(
        core.fail_expense_report_attempt("attempt-2", "timeout")?
            .replayed
    );
    assert!(matches!(
        core.fail_expense_report_attempt("attempt-2", "transport"),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        core.fail_expense_report_attempt("attempt-3", "unsupported"),
        Err(Error::InvalidInput(_))
    ));
    Ok(())
}

#[test]
fn report_completeness_uses_gap_free_coverage_and_latest_superseding_batches() -> Result<()> {
    let (temporary, core) = fixture()?;
    let january = NaiveDate::from_ymd_opt(2027, 1, 1).expect("valid date");
    preview_and_import(
        &core,
        empty_import(
            'a',
            '1',
            january,
            NaiveDate::from_ymd_opt(2027, 1, 10).expect("valid date"),
            0,
        ),
    )?;
    preview_and_import(
        &core,
        empty_import(
            'a',
            '2',
            NaiveDate::from_ymd_opt(2027, 1, 12).expect("valid date"),
            NaiveDate::from_ymd_opt(2027, 1, 31).expect("valid date"),
            0,
        ),
    )?;
    let gap = core.get_expense_month_summary(january)?;
    assert_eq!(gap.status, ExpenseReportStatus::Provisional);
    assert_eq!(gap.completeness.active_source_count, 1);
    assert_eq!(gap.completeness.covered_source_count, 0);

    preview_and_import(
        &core,
        empty_import(
            'a',
            '3',
            NaiveDate::from_ymd_opt(2027, 1, 11).expect("valid date"),
            NaiveDate::from_ymd_opt(2027, 1, 11).expect("valid date"),
            0,
        ),
    )?;
    let filled = core.get_expense_month_summary(january)?;
    assert_eq!(filled.status, ExpenseReportStatus::Confirmed);
    assert_eq!(filled.completeness.covered_source_count, 1);

    let rejected = preview_and_import(
        &core,
        empty_import(
            'b',
            '4',
            january,
            NaiveDate::from_ymd_opt(2027, 1, 31).expect("valid date"),
            1,
        ),
    )?;
    assert_eq!(
        core.get_expense_month_summary(january)?.status,
        ExpenseReportStatus::Incomplete
    );
    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    connection.execute(
        "UPDATE expense_import_batches
         SET created_at = '2000-01-01T00:00:00.000Z'
         WHERE id = ?1",
        [&rejected.batch_id],
    )?;
    drop(connection);
    preview_and_import(
        &core,
        empty_import(
            'b',
            '5',
            january,
            NaiveDate::from_ymd_opt(2027, 1, 31).expect("valid date"),
            0,
        ),
    )?;
    let superseded = core.get_expense_month_summary(january)?;
    assert_eq!(superseded.completeness.rejected_row_count, 0);
    assert_eq!(superseded.completeness.active_source_count, 2);
    assert_eq!(superseded.completeness.covered_source_count, 2);
    assert_eq!(superseded.status, ExpenseReportStatus::Confirmed);

    preview_and_import(
        &core,
        empty_import(
            'c',
            '6',
            NaiveDate::from_ymd_opt(2027, 3, 1).expect("valid date"),
            NaiveDate::from_ymd_opt(2027, 3, 31).expect("valid date"),
            0,
        ),
    )?;
    let before_new_source = core.get_expense_month_summary(january)?;
    assert_eq!(before_new_source.completeness.active_source_count, 2);
    assert_eq!(before_new_source.status, ExpenseReportStatus::Confirmed);

    let sources = core.list_expense_sources()?;
    for source in sources
        .iter()
        .filter(|source| source.coverage_start == Some(january))
    {
        let updated = core.update_expense_source_status(
            &source.id,
            UpdateExpenseSourceStatusInput {
                expected_version: source.version,
                required_for_complete_report: true,
                is_active: false,
            },
        )?;
        assert!(!updated.is_active);
        assert!(matches!(
            core.update_expense_source_status(
                &source.id,
                UpdateExpenseSourceStatusInput {
                    expected_version: source.version,
                    required_for_complete_report: true,
                    is_active: true,
                },
            ),
            Err(Error::Conflict(_))
        ));
    }
    let march = NaiveDate::from_ymd_opt(2027, 3, 1).expect("valid date");
    let after_closing_old_sources = core.get_expense_month_summary(march)?;
    assert_eq!(
        after_closing_old_sources.completeness.active_source_count,
        1
    );
    assert_eq!(
        after_closing_old_sources.status,
        ExpenseReportStatus::Confirmed
    );

    let february = NaiveDate::from_ymd_opt(2027, 2, 1).expect("valid date");
    let missing = core.get_expense_month_summary(february)?;
    assert_eq!(missing.completeness.covered_source_count, 0);
    assert_eq!(missing.status, ExpenseReportStatus::Incomplete);
    Ok(())
}

#[test]
fn ai_aggregate_deltas_require_two_confirmed_months_and_saved_facts_round_trip() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let source_fingerprint = digest('6');
    let january = NaiveDate::from_ymd_opt(2027, 1, 1).expect("valid date");
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbCardUsageV1,
            source_kind: ExpenseSourceKind::Card,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('1'),
            normalized_sha256: digest('1'),
            coverage_start: january,
            coverage_end: NaiveDate::from_ymd_opt(2027, 1, 31).expect("valid date"),
            rejected_count: 0,
            rows: vec![
                row(
                    &source_fingerprint,
                    "january-food",
                    1,
                    NaiveDate::from_ymd_opt(2027, 1, 10).expect("valid date"),
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    10_000,
                    Some(ExpenseCategory::Food),
                    '1',
                ),
                row(
                    &source_fingerprint,
                    "january-groceries",
                    2,
                    NaiveDate::from_ymd_opt(2027, 1, 11).expect("valid date"),
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    3_000,
                    Some(ExpenseCategory::Groceries),
                    '2',
                ),
            ],
        },
    )?;

    let february = NaiveDate::from_ymd_opt(2027, 2, 1).expect("valid date");
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbCardUsageV1,
            source_kind: ExpenseSourceKind::Card,
            source_fingerprint,
            file_sha256: digest('3'),
            normalized_sha256: digest('3'),
            coverage_start: february,
            coverage_end: NaiveDate::from_ymd_opt(2027, 2, 28).expect("valid date"),
            rejected_count: 0,
            rows: vec![
                row(
                    &digest('6'),
                    "february-food",
                    1,
                    NaiveDate::from_ymd_opt(2027, 2, 10).expect("valid date"),
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    12_000,
                    Some(ExpenseCategory::Food),
                    '3',
                ),
                row(
                    &digest('6'),
                    "february-shopping",
                    2,
                    NaiveDate::from_ymd_opt(2027, 2, 11).expect("valid date"),
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    5_000,
                    Some(ExpenseCategory::Shopping),
                    '4',
                ),
            ],
        },
    )?;

    for month in [january, february] {
        let reviews = core.list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?;
        for review in reviews.items {
            if review.reason != ExpenseReviewReason::CategoryConfirmation {
                continue;
            }
            core.resolve_expense_review(
                &review.id,
                ResolveExpenseReviewInput {
                    expected_version: review.version,
                    kind: ExpenseEventKind::Purchase,
                    category: review.suggested_category.unwrap_or(ExpenseCategory::Other),
                    duplicate_of_event_id: None,
                    related_event_id: None,
                    personal_amount_minor: None,
                    create_rule: false,
                },
            )?;
        }
        assert_eq!(
            core.get_expense_month_summary(month)?.status,
            ExpenseReportStatus::Confirmed
        );
    }

    let aggregate = core.expense_report_aggregate(february)?;
    let fact = |id: &str| {
        aggregate
            .facts
            .iter()
            .find(|fact| fact.fact_id == id)
            .map(|fact| fact.amount_minor)
    };
    assert_eq!(fact("delta:currency:KRW:net_personal_spend"), Some(4_000));
    assert_eq!(fact("delta:category:KRW:food"), Some(2_000));
    assert_eq!(fact("delta:category:KRW:shopping"), Some(5_000));
    assert_eq!(fact("delta:category:KRW:groceries"), Some(-3_000));

    let march = NaiveDate::from_ymd_opt(2027, 3, 1).expect("valid date");
    let incomplete = core.expense_report_aggregate(march)?;
    assert_eq!(incomplete.report_status, ExpenseReportStatus::Incomplete);
    assert!(
        incomplete
            .facts
            .iter()
            .all(|fact| !fact.fact_id.starts_with("delta:"))
    );

    let cited_fact = aggregate.facts.first().cloned().expect("aggregate fact");
    let saved = core.save_expense_report(SaveExpenseReportInput {
        month_start: february,
        aggregate_sha256: aggregate.aggregate_sha256.clone(),
        prompt_version: "expense-v1".to_owned(),
        model: "gpt-5.4-nano-2026-03-17".to_owned(),
        title: "Synthetic report".to_owned(),
        summary: "Aggregate-only test report".to_owned(),
        facts: aggregate.facts.clone(),
        observations: vec![ExpenseReportObservation {
            text: "A grounded observation".to_owned(),
            fact_ids: vec![cited_fact.fact_id.clone()],
        }],
        alerts: Vec::new(),
        next_month_checks: vec!["Check recurring variance".to_owned()],
        input_tokens: 100,
        cached_input_tokens: 20,
        output_tokens: 50,
        total_tokens: 150,
        cost_microusd: 1_000,
        latency_ms: 20,
    })?;
    assert_eq!(saved.facts, aggregate.facts);
    assert_eq!(core.get_expense_report(&saved.id)?, Some(saved.clone()));
    assert_eq!(
        core.get_cached_expense_report(&aggregate.aggregate_sha256, "expense-v1")?,
        Some(saved)
    );

    assert!(matches!(
        core.save_expense_report(SaveExpenseReportInput {
            month_start: february,
            aggregate_sha256: digest('9'),
            prompt_version: "expense-v2".to_owned(),
            model: "gpt-5.4-nano-2026-03-17".to_owned(),
            title: "Invalid report".to_owned(),
            summary: "Unknown citation".to_owned(),
            facts: vec![ExpenseReportFact {
                fact_id: "known".to_owned(),
                metric: "net_personal_spend".to_owned(),
                currency: "KRW".to_owned(),
                amount_minor: 1,
            }],
            observations: vec![ExpenseReportObservation {
                text: "Not grounded".to_owned(),
                fact_ids: vec!["unknown".to_owned()],
            }],
            alerts: Vec::new(),
            next_month_checks: Vec::new(),
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 1,
            total_tokens: 2,
            cost_microusd: 1,
            latency_ms: 1,
        }),
        Err(Error::InvalidInput(_))
    ));
    Ok(())
}

#[test]
fn recurring_candidate_requires_a_stable_interval_and_is_not_duplicated() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let source_fingerprint = digest('a');
    for (stable_key, date, marker) in [
        (
            "stable-january",
            NaiveDate::from_ymd_opt(2026, 1, 15).expect("valid date"),
            '1',
        ),
        (
            "stable-march",
            NaiveDate::from_ymd_opt(2026, 3, 15).expect("valid date"),
            '2',
        ),
        (
            "stable-may",
            NaiveDate::from_ymd_opt(2026, 5, 15).expect("valid date"),
            '3',
        ),
        (
            "stable-july",
            NaiveDate::from_ymd_opt(2026, 7, 15).expect("valid date"),
            '4',
        ),
    ] {
        preview_and_import(
            &core,
            recurring_candidate_import(&source_fingerprint, stable_key, date, marker),
        )?;
    }
    let reviews = core.list_expense_reviews(ExpenseReviewFilter {
        month_start: None,
        status: Some(ExpenseReviewStatus::Pending),
        cursor: None,
        limit: 100,
    })?;
    assert_eq!(
        reviews
            .items
            .iter()
            .filter(|review| {
                review.reason == ExpenseReviewReason::RecurringRegistrationCandidate
            })
            .count(),
        1
    );

    let (_irregular_temporary, irregular_core) = fixture()?;
    let irregular_source_fingerprint = digest('b');
    for (stable_key, date, marker) in [
        (
            "irregular-january",
            NaiveDate::from_ymd_opt(2026, 1, 15).expect("valid date"),
            '5',
        ),
        (
            "irregular-february",
            NaiveDate::from_ymd_opt(2026, 2, 15).expect("valid date"),
            '6',
        ),
        (
            "irregular-april",
            NaiveDate::from_ymd_opt(2026, 4, 15).expect("valid date"),
            '7',
        ),
    ] {
        preview_and_import(
            &irregular_core,
            recurring_candidate_import(&irregular_source_fingerprint, stable_key, date, marker),
        )?;
    }
    let irregular_reviews = irregular_core.list_expense_reviews(ExpenseReviewFilter {
        month_start: None,
        status: Some(ExpenseReviewStatus::Pending),
        cursor: None,
        limit: 100,
    })?;
    assert!(
        irregular_reviews
            .items
            .iter()
            .all(|review| { review.reason != ExpenseReviewReason::RecurringRegistrationCandidate })
    );
    Ok(())
}

#[test]
fn classification_rules_only_reclassify_allowed_rows_and_preserve_accounting_semantics()
-> Result<()> {
    let (_temporary, core) = fixture()?;
    let source_fingerprint = digest('d');
    let month = NaiveDate::from_ymd_opt(2027, 5, 1).expect("valid date");
    let import_one = |stable_key: &str,
                      day: u32,
                      kind: ExpenseEventKind,
                      direction: ExpenseDirection,
                      marker: char| {
        let mut imported_row = row(
            &source_fingerprint,
            stable_key,
            1,
            NaiveDate::from_ymd_opt(2027, 5, day).expect("valid date"),
            kind,
            direction,
            10_000,
            None,
            marker,
        );
        imported_row.merchant = Some(encrypted(
            &format!("{source_fingerprint}:{stable_key}"),
            "merchant",
            'v',
        ));
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest(marker),
            normalized_sha256: digest(marker),
            coverage_start: NaiveDate::from_ymd_opt(2027, 5, day).expect("valid date"),
            coverage_end: NaiveDate::from_ymd_opt(2027, 5, day).expect("valid date"),
            rejected_count: 0,
            rows: vec![imported_row],
        }
    };

    preview_and_import(
        &core,
        import_one(
            "rule-source",
            1,
            ExpenseEventKind::UnknownP2p,
            ExpenseDirection::Debit,
            '1',
        ),
    )?;
    let source_review = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|review| review.reason == ExpenseReviewReason::UnknownP2p)
        .expect("unknown transfer review");
    core.resolve_expense_review(
        &source_review.id,
        ResolveExpenseReviewInput {
            expected_version: source_review.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Food,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            create_rule: true,
        },
    )?;

    let correction_source = digest('e');
    let mut correction_row = row(
        &correction_source,
        "rule-correction",
        1,
        month,
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        10_000,
        Some(ExpenseCategory::Food),
        '8',
    );
    correction_row.occurred_at = "2027-05-01T13:00:00+09:00".to_owned();
    correction_row.external_reference_fingerprint = None;
    correction_row.merchant = Some(encrypted(
        &format!("{correction_source}:rule-correction"),
        "merchant",
        'v',
    ));
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbCardUsageV1,
            source_kind: ExpenseSourceKind::Card,
            source_fingerprint: correction_source,
            file_sha256: digest('8'),
            normalized_sha256: digest('8'),
            coverage_start: month,
            coverage_end: month,
            rejected_count: 0,
            rows: vec![correction_row],
        },
    )?;
    let correction_review = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|review| review.reason == ExpenseReviewReason::AmbiguousMirror)
        .expect("rule correction review");
    core.resolve_expense_review(
        &correction_review.id,
        ResolveExpenseReviewInput {
            expected_version: correction_review.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Shopping,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            create_rule: true,
        },
    )?;

    for (stable_key, day, kind, direction, marker) in [
        (
            "rule-applied",
            2,
            ExpenseEventKind::UnknownP2p,
            ExpenseDirection::Debit,
            '2',
        ),
        (
            "protected-refund",
            3,
            ExpenseEventKind::Refund,
            ExpenseDirection::Credit,
            '3',
        ),
        (
            "protected-card-payment",
            4,
            ExpenseEventKind::CardPayment,
            ExpenseDirection::Debit,
            '4',
        ),
        (
            "corrected-rule-applied",
            5,
            ExpenseEventKind::UnknownP2p,
            ExpenseDirection::Debit,
            '5',
        ),
        (
            "purchase-rule-credit-mismatch",
            6,
            ExpenseEventKind::UnknownP2p,
            ExpenseDirection::Credit,
            '6',
        ),
    ] {
        preview_and_import(&core, import_one(stable_key, day, kind, direction, marker))?;
    }

    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    let find = |day| {
        transactions
            .items
            .iter()
            .find(|transaction| transaction.posted_date.day() == day)
            .expect("transaction for day")
    };
    assert_eq!(find(2).kind, ExpenseEventKind::Purchase);
    assert_eq!(find(2).category, ExpenseCategory::Shopping);
    assert_eq!(find(2).status, tm_core::ExpenseEventStatus::Confirmed);
    assert_eq!(find(3).kind, ExpenseEventKind::Refund);
    assert_eq!(find(3).category, ExpenseCategory::RefundIncome);
    assert_eq!(find(4).kind, ExpenseEventKind::CardPayment);
    assert_eq!(find(4).status, tm_core::ExpenseEventStatus::Excluded);
    assert_eq!(find(4).exclusion_reason.as_deref(), Some("card_payment"));
    assert_eq!(find(5).kind, ExpenseEventKind::Purchase);
    assert_eq!(find(5).category, ExpenseCategory::Shopping);
    assert_eq!(find(6).kind, ExpenseEventKind::UnknownP2p);
    assert_eq!(find(6).status, tm_core::ExpenseEventStatus::Unconfirmed);

    let credit_review = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|review| review.transaction.posted_date.day() == 6)
        .expect("credit rule mismatch review");
    core.resolve_expense_review(
        &credit_review.id,
        ResolveExpenseReviewInput {
            expected_version: credit_review.version,
            kind: ExpenseEventKind::Refund,
            category: ExpenseCategory::RefundIncome,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            create_rule: true,
        },
    )?;
    preview_and_import(
        &core,
        import_one(
            "refund-rule-debit-mismatch",
            7,
            ExpenseEventKind::UnknownP2p,
            ExpenseDirection::Debit,
            '7',
        ),
    )?;
    let debit_mismatch = core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start: month,
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|transaction| transaction.posted_date.day() == 7)
        .expect("debit rule mismatch transaction");
    assert_eq!(debit_mismatch.kind, ExpenseEventKind::UnknownP2p);
    assert_eq!(
        debit_mismatch.status,
        tm_core::ExpenseEventStatus::Unconfirmed
    );
    assert!(debit_mismatch.pending_review_id.is_some());
    Ok(())
}

#[test]
fn personal_allocation_overrides_summary_and_redacted_export_totals() -> Result<()> {
    let (temporary, core) = fixture()?;
    let date = NaiveDate::from_ymd_opt(2026, 9, 10).expect("valid date");
    let source_fingerprint = digest('8');
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('9'),
            normalized_sha256: digest('0'),
            coverage_start: date,
            coverage_end: date,
            rejected_count: 0,
            rows: vec![row(
                &source_fingerprint,
                "shared-purchase",
                1,
                date,
                ExpenseEventKind::UnknownP2p,
                ExpenseDirection::Debit,
                10_000,
                None,
                'a',
            )],
        },
    )?;
    let review = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(NaiveDate::from_ymd_opt(2026, 9, 1).expect("valid date")),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .next()
        .expect("unknown P2P review");
    core.resolve_expense_review(
        &review.id,
        ResolveExpenseReviewInput {
            expected_version: review.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Food,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: Some(6_000),
            create_rule: false,
        },
    )?;

    let settlement_date = date.succ_opt().expect("valid date");
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('c'),
            normalized_sha256: digest('d'),
            coverage_start: settlement_date,
            coverage_end: settlement_date,
            rejected_count: 0,
            rows: vec![row(
                &source_fingerprint,
                "shared-settlement",
                1,
                settlement_date,
                ExpenseEventKind::UnknownP2p,
                ExpenseDirection::Credit,
                4_000,
                None,
                'b',
            )],
        },
    )?;
    let settlement_review = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(NaiveDate::from_ymd_opt(2026, 9, 1).expect("valid date")),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|item| item.transaction.amount_minor == 4_000)
        .expect("settlement review");
    core.resolve_expense_review(
        &settlement_review.id,
        ResolveExpenseReviewInput {
            expected_version: settlement_review.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: Some(review.transaction.id.clone()),
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;

    let month_start = NaiveDate::from_ymd_opt(2026, 9, 1).expect("valid date");
    let transactions = core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start,
            cursor: None,
            limit: 50,
        })?
        .items;
    let purchase = transactions
        .iter()
        .find(|item| item.id == review.transaction.id)
        .expect("split purchase");
    let settlement = transactions
        .iter()
        .find(|item| item.id == settlement_review.transaction.id)
        .expect("linked settlement");
    assert_eq!(purchase.personal_amount_minor, Some(6_000));
    assert_eq!(
        settlement.related_event_id.as_deref(),
        Some(purchase.id.as_str())
    );
    let purchase = core.override_expense_transaction(
        &purchase.id,
        OverrideExpenseTransactionInput {
            expected_version: purchase.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Shopping,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            clear_personal_amount: false,
            clear_related_event: false,
            create_rule: false,
        },
    )?;
    let settlement = core.override_expense_transaction(
        &settlement.id,
        OverrideExpenseTransactionInput {
            expected_version: settlement.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            clear_personal_amount: false,
            clear_related_event: false,
            create_rule: false,
        },
    )?;
    assert_eq!(purchase.personal_amount_minor, Some(6_000));
    assert_eq!(
        settlement.related_event_id.as_deref(),
        Some(purchase.id.as_str())
    );
    assert!(matches!(
        core.override_expense_transaction(
            &purchase.id,
            OverrideExpenseTransactionInput {
                expected_version: purchase.version,
                kind: ExpenseEventKind::Purchase,
                category: ExpenseCategory::Shopping,
                duplicate_of_event_id: None,
                related_event_id: None,
                personal_amount_minor: None,
                clear_personal_amount: true,
                clear_related_event: false,
                create_rule: false,
            },
        ),
        Err(Error::InvalidInput(_))
    ));

    let summary =
        core.get_expense_month_summary(NaiveDate::from_ymd_opt(2026, 9, 1).expect("valid date"))?;
    let krw = summary
        .currencies
        .iter()
        .find(|item| item.currency == "KRW")
        .expect("KRW summary");
    assert_eq!(krw.gross_purchase_minor, 10_000);
    assert_eq!(krw.settlement_received_minor, 4_000);
    assert_eq!(krw.net_personal_spend_minor, 6_000);
    assert_eq!(summary.categories[0].amount_minor, 6_000);
    assert_eq!(summary.daily[0].amount_minor, 6_000);

    let export = core.export_json()?;
    let aggregate = export["expenseMonthlyAggregates"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["month"] == "2026-09" && item["currency"] == "KRW")
        })
        .expect("redacted September aggregate");
    assert_eq!(aggregate["grossPurchaseMinor"], 10_000);
    assert_eq!(aggregate["settlementReceivedMinor"], 4_000);
    assert_eq!(aggregate["netPersonalSpendMinor"], 6_000);

    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    let stored_amount: i64 = connection.query_row(
        "SELECT amount_minor FROM expense_allocations
         WHERE event_id = ?1 AND allocation_kind = 'personal'",
        [&review.transaction.id],
        |row| row.get(0),
    )?;
    assert_eq!(stored_amount, 6_000);
    drop(connection);

    let cleared = core.override_expense_transaction(
        &settlement.id,
        OverrideExpenseTransactionInput {
            expected_version: settlement.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            clear_personal_amount: false,
            clear_related_event: true,
            create_rule: false,
        },
    )?;
    assert!(cleared.related_event_id.is_none());
    Ok(())
}

#[test]
fn settlement_links_validate_currency_total_and_retry_atomically() -> Result<()> {
    let (temporary, core) = fixture()?;
    let month = NaiveDate::from_ymd_opt(2026, 10, 1).expect("valid date");
    let source_fingerprint = digest('c');
    let mut usd_purchase = row(
        &source_fingerprint,
        "usd-purchase",
        4,
        NaiveDate::from_ymd_opt(2026, 10, 4).expect("valid date"),
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        5_000,
        Some(ExpenseCategory::Shopping),
        '4',
    );
    usd_purchase.currency = "USD".to_owned();
    let mut usd_settlement = row(
        &source_fingerprint,
        "usd-settlement",
        5,
        NaiveDate::from_ymd_opt(2026, 10, 5).expect("valid date"),
        ExpenseEventKind::UnknownP2p,
        ExpenseDirection::Credit,
        1_000,
        None,
        '5',
    );
    usd_settlement.currency = "USD".to_owned();
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('d'),
            normalized_sha256: digest('e'),
            coverage_start: month,
            coverage_end: NaiveDate::from_ymd_opt(2026, 10, 31).expect("valid date"),
            rejected_count: 0,
            rows: vec![
                row(
                    &source_fingerprint,
                    "krw-purchase",
                    1,
                    month,
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    10_000,
                    Some(ExpenseCategory::Food),
                    '1',
                ),
                row(
                    &source_fingerprint,
                    "settlement-received",
                    2,
                    NaiveDate::from_ymd_opt(2026, 10, 2).expect("valid date"),
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Credit,
                    3_000,
                    None,
                    '2',
                ),
                row(
                    &source_fingerprint,
                    "settlement-sent",
                    3,
                    NaiveDate::from_ymd_opt(2026, 10, 3).expect("valid date"),
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Debit,
                    2_000,
                    None,
                    '3',
                ),
                usd_purchase,
                usd_settlement,
                row(
                    &source_fingerprint,
                    "excess-settlement",
                    6,
                    NaiveDate::from_ymd_opt(2026, 10, 6).expect("valid date"),
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Credit,
                    6_000,
                    None,
                    '6',
                ),
            ],
        },
    )?;

    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    let krw_purchase_id = transactions
        .items
        .iter()
        .find(|event| {
            event.kind == ExpenseEventKind::Purchase
                && event.currency == "KRW"
                && event.amount_minor == 10_000
        })
        .map(|event| event.id.clone())
        .expect("KRW purchase");
    let usd_purchase_id = transactions
        .items
        .iter()
        .find(|event| {
            event.kind == ExpenseEventKind::Purchase
                && event.currency == "USD"
                && event.amount_minor == 5_000
        })
        .map(|event| event.id.clone())
        .expect("USD purchase");
    let reviews = core.list_expense_reviews(ExpenseReviewFilter {
        month_start: Some(month),
        status: Some(ExpenseReviewStatus::Pending),
        cursor: None,
        limit: 50,
    })?;
    let find_review = |amount_minor, currency: &str| {
        reviews
            .items
            .iter()
            .find(|review| {
                review.transaction.amount_minor == amount_minor
                    && review.transaction.currency == currency
            })
            .cloned()
            .expect("matching settlement review")
    };
    let received = find_review(3_000, "KRW");
    let sent = find_review(2_000, "KRW");
    let usd = find_review(1_000, "USD");
    let excess = find_review(6_000, "KRW");

    for (review, kind) in [
        (&received, ExpenseEventKind::SettlementReceived),
        (&sent, ExpenseEventKind::SettlementSent),
    ] {
        core.resolve_expense_review(
            &review.id,
            ResolveExpenseReviewInput {
                expected_version: review.version,
                kind,
                category: ExpenseCategory::TransferSettlement,
                duplicate_of_event_id: None,
                related_event_id: Some(krw_purchase_id.clone()),
                personal_amount_minor: None,
                create_rule: false,
            },
        )?;
    }

    assert!(matches!(
        core.resolve_expense_review(
            &usd.id,
            ResolveExpenseReviewInput {
                expected_version: usd.version,
                kind: ExpenseEventKind::SettlementReceived,
                category: ExpenseCategory::TransferSettlement,
                duplicate_of_event_id: None,
                related_event_id: Some(krw_purchase_id.clone()),
                personal_amount_minor: None,
                create_rule: false,
            },
        ),
        Err(Error::InvalidInput(_))
    ));
    let pending_usd = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|review| review.id == usd.id)
        .expect("currency mismatch rolls back the review");
    assert_eq!(pending_usd.version, usd.version);
    core.resolve_expense_review(
        &usd.id,
        ResolveExpenseReviewInput {
            expected_version: usd.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: Some(usd_purchase_id.clone()),
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;

    assert!(matches!(
        core.resolve_expense_review(
            &excess.id,
            ResolveExpenseReviewInput {
                expected_version: excess.version,
                kind: ExpenseEventKind::SettlementReceived,
                category: ExpenseCategory::TransferSettlement,
                duplicate_of_event_id: None,
                related_event_id: Some(krw_purchase_id.clone()),
                personal_amount_minor: None,
                create_rule: false,
            },
        ),
        Err(Error::InvalidInput(_))
    ));
    core.resolve_expense_review(
        &excess.id,
        ResolveExpenseReviewInput {
            expected_version: excess.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;

    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    let krw_links: u32 = connection.query_row(
        "SELECT count(*) FROM expense_allocations
         WHERE related_event_id = ?1
           AND allocation_kind IN ('settlement_received', 'settlement_sent')",
        [&krw_purchase_id],
        |row| row.get(0),
    )?;
    assert_eq!(krw_links, 2);
    let usd_links: u32 = connection.query_row(
        "SELECT count(*) FROM expense_allocations WHERE related_event_id = ?1",
        [&usd_purchase_id],
        |row| row.get(0),
    )?;
    assert_eq!(usd_links, 1);
    let excess_links: u32 = connection.query_row(
        "SELECT count(*) FROM expense_allocations WHERE event_id = ?1",
        [&excess.transaction.id],
        |row| row.get(0),
    )?;
    assert_eq!(excess_links, 0);

    let summary = core.get_expense_month_summary(month)?;
    let krw = summary
        .currencies
        .iter()
        .find(|item| item.currency == "KRW")
        .expect("KRW summary");
    assert_eq!(krw.net_personal_spend_minor, 3_000);
    let usd_summary = summary
        .currencies
        .iter()
        .find(|item| item.currency == "USD")
        .expect("USD summary");
    assert_eq!(usd_summary.net_personal_spend_minor, 4_000);
    Ok(())
}

#[test]
fn full_reimbursement_allows_zero_and_one_minor_unit_over_rolls_back() -> Result<()> {
    let (temporary, core) = fixture()?;
    let month = NaiveDate::from_ymd_opt(2026, 10, 1).expect("valid date");
    let exact_purchase_date = NaiveDate::from_ymd_opt(2026, 10, 10).expect("valid date");
    let exact_settlement_date = NaiveDate::from_ymd_opt(2026, 10, 11).expect("valid date");
    let exact_final_settlement_date = NaiveDate::from_ymd_opt(2026, 10, 12).expect("valid date");
    let excess_purchase_date = NaiveDate::from_ymd_opt(2026, 10, 13).expect("valid date");
    let excess_settlement_date = NaiveDate::from_ymd_opt(2026, 10, 14).expect("valid date");
    let split_purchase_date = NaiveDate::from_ymd_opt(2026, 10, 15).expect("valid date");
    let split_settlement_date = NaiveDate::from_ymd_opt(2026, 10, 16).expect("valid date");
    let source_fingerprint = digest('7');
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('8'),
            normalized_sha256: digest('9'),
            coverage_start: exact_purchase_date,
            coverage_end: split_settlement_date,
            rejected_count: 0,
            rows: vec![
                row(
                    &source_fingerprint,
                    "exact-purchase",
                    1,
                    exact_purchase_date,
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    10_000,
                    Some(ExpenseCategory::Food),
                    '1',
                ),
                row(
                    &source_fingerprint,
                    "exact-settlement",
                    2,
                    exact_settlement_date,
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Credit,
                    4_000,
                    None,
                    '2',
                ),
                row(
                    &source_fingerprint,
                    "exact-final-settlement",
                    3,
                    exact_final_settlement_date,
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Credit,
                    6_000,
                    None,
                    '3',
                ),
                row(
                    &source_fingerprint,
                    "excess-purchase",
                    4,
                    excess_purchase_date,
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    10_000,
                    Some(ExpenseCategory::Shopping),
                    '4',
                ),
                row(
                    &source_fingerprint,
                    "excess-settlement",
                    5,
                    excess_settlement_date,
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Credit,
                    10_001,
                    None,
                    '5',
                ),
                row(
                    &source_fingerprint,
                    "split-purchase",
                    6,
                    split_purchase_date,
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Debit,
                    10_000,
                    None,
                    '6',
                ),
                row(
                    &source_fingerprint,
                    "split-settlement",
                    7,
                    split_settlement_date,
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Credit,
                    10_000,
                    None,
                    '7',
                ),
            ],
        },
    )?;

    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    let purchase_id_on = |date| {
        transactions
            .items
            .iter()
            .find(|transaction| transaction.posted_date == date)
            .map(|transaction| transaction.id.clone())
            .expect("purchase transaction")
    };
    let exact_purchase_id = purchase_id_on(exact_purchase_date);
    let excess_purchase_id = purchase_id_on(excess_purchase_date);
    let split_purchase_id = purchase_id_on(split_purchase_date);

    let reviews = core.list_expense_reviews(ExpenseReviewFilter {
        month_start: Some(month),
        status: Some(ExpenseReviewStatus::Pending),
        cursor: None,
        limit: 50,
    })?;
    let review_on = |date| {
        reviews
            .items
            .iter()
            .find(|review| review.transaction.posted_date == date)
            .cloned()
            .expect("expense review")
    };
    let exact_settlement = review_on(exact_settlement_date);
    let exact_final_settlement = review_on(exact_final_settlement_date);
    let excess_settlement = review_on(excess_settlement_date);
    let split_purchase = review_on(split_purchase_date);
    let split_settlement = review_on(split_settlement_date);

    core.resolve_expense_review(
        &split_purchase.id,
        ResolveExpenseReviewInput {
            expected_version: split_purchase.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Food,
            duplicate_of_event_id: None,
            related_event_id: None,
            personal_amount_minor: Some(1_000),
            create_rule: false,
        },
    )?;
    core.resolve_expense_review(
        &exact_settlement.id,
        ResolveExpenseReviewInput {
            expected_version: exact_settlement.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: Some(exact_purchase_id.clone()),
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;
    let partial_purchase = core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start: month,
            cursor: None,
            limit: 50,
        })?
        .items
        .into_iter()
        .find(|transaction| transaction.id == exact_purchase_id)
        .expect("partially reimbursed purchase");
    assert_eq!(partial_purchase.personal_amount_minor, Some(6_000));
    core.resolve_expense_review(
        &exact_final_settlement.id,
        ResolveExpenseReviewInput {
            expected_version: exact_final_settlement.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: Some(exact_purchase_id.clone()),
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;

    assert!(matches!(
        core.resolve_expense_review(
            &excess_settlement.id,
            ResolveExpenseReviewInput {
                expected_version: excess_settlement.version,
                kind: ExpenseEventKind::SettlementReceived,
                category: ExpenseCategory::TransferSettlement,
                duplicate_of_event_id: None,
                related_event_id: Some(excess_purchase_id),
                personal_amount_minor: None,
                create_rule: false,
            },
        ),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        core.resolve_expense_review(
            &split_settlement.id,
            ResolveExpenseReviewInput {
                expected_version: split_settlement.version,
                kind: ExpenseEventKind::SettlementReceived,
                category: ExpenseCategory::TransferSettlement,
                duplicate_of_event_id: None,
                related_event_id: Some(split_purchase_id),
                personal_amount_minor: None,
                create_rule: false,
            },
        ),
        Err(Error::InvalidInput(_))
    ));

    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    let exact_purchase = transactions
        .items
        .iter()
        .find(|transaction| transaction.id == exact_purchase_id)
        .expect("exact purchase");
    let exact_settlement = transactions
        .items
        .iter()
        .find(|transaction| transaction.id == exact_settlement.transaction.id)
        .expect("exact settlement");
    assert_eq!(exact_purchase.personal_amount_minor, None);
    assert_eq!(
        exact_settlement.related_event_id.as_deref(),
        Some(exact_purchase.id.as_str())
    );

    let connection = Connection::open(TmHome::new(temporary.path()).database_path())?;
    let generated_personal_allocations: u32 = connection.query_row(
        "SELECT count(*) FROM expense_allocations
         WHERE event_id = ?1 AND allocation_kind = 'personal'
           AND allocation_source = 'settlement'",
        [&exact_purchase_id],
        |row| row.get(0),
    )?;
    assert_eq!(generated_personal_allocations, 0);
    let rejected_links: u32 = connection.query_row(
        "SELECT count(*) FROM expense_allocations
         WHERE event_id IN (?1, ?2)
           AND allocation_kind IN ('settlement_received', 'settlement_sent')",
        params![
            &excess_settlement.transaction.id,
            &split_settlement.transaction.id
        ],
        |row| row.get(0),
    )?;
    assert_eq!(rejected_links, 0);

    let summary = core.get_expense_month_summary(month)?;
    let krw = summary
        .currencies
        .iter()
        .find(|item| item.currency == "KRW")
        .expect("KRW summary");
    assert_eq!(krw.net_personal_spend_minor, 11_000);
    assert_eq!(
        summary
            .categories
            .iter()
            .find(|item| item.category == ExpenseCategory::Food)
            .map(|item| item.amount_minor),
        Some(1_000)
    );
    assert_eq!(
        summary
            .categories
            .iter()
            .find(|item| item.category == ExpenseCategory::Shopping)
            .map(|item| item.amount_minor),
        Some(10_000)
    );
    assert!(
        summary
            .categories
            .iter()
            .all(|item| item.category != ExpenseCategory::TransferSettlement)
    );
    Ok(())
}

#[test]
fn full_reimbursement_across_months_is_attributed_to_the_purchase() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let purchase_month = NaiveDate::from_ymd_opt(2026, 10, 1).expect("valid date");
    let settlement_month = NaiveDate::from_ymd_opt(2026, 11, 1).expect("valid date");
    let purchase_date = NaiveDate::from_ymd_opt(2026, 10, 31).expect("valid date");
    let settlement_date = NaiveDate::from_ymd_opt(2026, 11, 1).expect("valid date");
    let source_fingerprint = digest('a');
    preview_and_import(
        &core,
        NormalizedExpenseImport {
            adapter: ExpenseImportAdapter::KbAccountHistoryV1,
            source_kind: ExpenseSourceKind::Account,
            source_fingerprint: source_fingerprint.clone(),
            file_sha256: digest('b'),
            normalized_sha256: digest('c'),
            coverage_start: purchase_date,
            coverage_end: settlement_date,
            rejected_count: 0,
            rows: vec![
                row(
                    &source_fingerprint,
                    "month-edge-purchase",
                    1,
                    purchase_date,
                    ExpenseEventKind::Purchase,
                    ExpenseDirection::Debit,
                    10_000,
                    Some(ExpenseCategory::Food),
                    'd',
                ),
                row(
                    &source_fingerprint,
                    "month-edge-settlement",
                    2,
                    settlement_date,
                    ExpenseEventKind::UnknownP2p,
                    ExpenseDirection::Credit,
                    10_000,
                    None,
                    'e',
                ),
            ],
        },
    )?;

    let purchase_id = core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start: purchase_month,
            cursor: None,
            limit: 10,
        })?
        .items
        .into_iter()
        .find(|transaction| transaction.posted_date == purchase_date)
        .map(|transaction| transaction.id)
        .expect("purchase transaction");
    let settlement_review = core
        .list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(settlement_month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor: None,
            limit: 10,
        })?
        .items
        .into_iter()
        .find(|review| review.transaction.posted_date == settlement_date)
        .expect("settlement review");
    core.resolve_expense_review(
        &settlement_review.id,
        ResolveExpenseReviewInput {
            expected_version: settlement_review.version,
            kind: ExpenseEventKind::SettlementReceived,
            category: ExpenseCategory::TransferSettlement,
            duplicate_of_event_id: None,
            related_event_id: Some(purchase_id),
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;

    let purchase_summary = core.get_expense_month_summary(purchase_month)?;
    let purchase_krw = purchase_summary
        .currencies
        .iter()
        .find(|item| item.currency == "KRW")
        .expect("purchase month KRW summary");
    assert_eq!(purchase_krw.gross_purchase_minor, 10_000);
    assert_eq!(purchase_krw.net_personal_spend_minor, 0);
    assert!(purchase_summary.categories.is_empty());

    let settlement_summary = core.get_expense_month_summary(settlement_month)?;
    let settlement_krw = settlement_summary
        .currencies
        .iter()
        .find(|item| item.currency == "KRW")
        .expect("settlement month KRW summary");
    assert_eq!(settlement_krw.settlement_received_minor, 10_000);
    assert_eq!(settlement_krw.net_personal_spend_minor, 0);
    assert!(settlement_summary.categories.is_empty());
    Ok(())
}

#[test]
fn exact_cross_source_mirror_without_a_trusted_reference_requires_review() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let month = NaiveDate::from_ymd_opt(2026, 11, 1).expect("valid date");
    let occurred_date = NaiveDate::from_ymd_opt(2026, 11, 8).expect("valid date");
    let imports = [
        (
            ExpenseImportAdapter::KbAccountHistoryV1,
            ExpenseSourceKind::Account,
            digest('a'),
            "account-row",
            '1',
            true,
        ),
        (
            ExpenseImportAdapter::KbCardUsageV1,
            ExpenseSourceKind::Card,
            digest('b'),
            "card-row",
            '2',
            true,
        ),
        (
            ExpenseImportAdapter::KakaopayMoneyV1,
            ExpenseSourceKind::Wallet,
            digest('c'),
            "wallet-row-without-merchant",
            '3',
            false,
        ),
    ];
    let mut results = Vec::new();
    for (adapter, source_kind, source_fingerprint, stable_key, marker, has_merchant) in imports {
        let mut purchase = row(
            &source_fingerprint,
            stable_key,
            1,
            occurred_date,
            ExpenseEventKind::Purchase,
            ExpenseDirection::Debit,
            10_000,
            Some(ExpenseCategory::Shopping),
            marker,
        );
        purchase.external_reference_fingerprint = None;
        purchase.merchant = if has_merchant {
            Some(encrypted(
                &format!("{source_fingerprint}:{stable_key}"),
                "merchant",
                'e',
            ))
        } else {
            None
        };
        results.push(preview_and_import(
            &core,
            NormalizedExpenseImport {
                adapter,
                source_kind,
                source_fingerprint,
                file_sha256: digest(marker),
                normalized_sha256: digest(marker),
                coverage_start: occurred_date,
                coverage_end: occurred_date,
                rejected_count: 0,
                rows: vec![purchase],
            },
        )?);
    }
    assert_eq!(results[0].excluded_count, 0);
    assert_eq!(results[1].excluded_count, 0);
    assert_eq!(results[2].excluded_count, 0);

    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    assert_eq!(transactions.items.len(), 3);
    assert!(
        transactions
            .items
            .iter()
            .all(|event| { event.exclusion_reason.as_deref() != Some("cross_source_mirror") })
    );
    let reviews = core.list_expense_reviews(ExpenseReviewFilter {
        month_start: Some(month),
        status: Some(ExpenseReviewStatus::Pending),
        cursor: None,
        limit: 50,
    })?;
    assert_eq!(
        reviews
            .items
            .iter()
            .filter(|review| review.reason == ExpenseReviewReason::AmbiguousMirror)
            .count(),
        1
    );
    assert_eq!(
        core.get_expense_month_summary(month)?.currencies[0].gross_purchase_minor,
        30_000
    );
    Ok(())
}

#[test]
fn same_source_identical_looking_purchases_are_not_heuristically_deduplicated() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let source_fingerprint = digest('4');
    let date = NaiveDate::from_ymd_opt(2027, 9, 10).expect("valid date");
    let mut first = row(
        &source_fingerprint,
        "coffee-first",
        1,
        date,
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        5_000,
        Some(ExpenseCategory::Cafe),
        '1',
    );
    let mut second = row(
        &source_fingerprint,
        "coffee-second",
        2,
        date,
        ExpenseEventKind::Purchase,
        ExpenseDirection::Debit,
        5_000,
        Some(ExpenseCategory::Cafe),
        '2',
    );
    for (stable_key, purchase) in [("coffee-first", &mut first), ("coffee-second", &mut second)] {
        purchase.external_reference_fingerprint = None;
        purchase.merchant = Some(encrypted(
            &format!("{source_fingerprint}:{stable_key}"),
            "merchant",
            'c',
        ));
    }
    let input = NormalizedExpenseImport {
        adapter: ExpenseImportAdapter::KbCardUsageV1,
        source_kind: ExpenseSourceKind::Card,
        source_fingerprint,
        file_sha256: digest('3'),
        normalized_sha256: digest('3'),
        coverage_start: date,
        coverage_end: date,
        rejected_count: 0,
        rows: vec![first, second],
    };
    let preview = core.preview_expense_import(&input)?;
    assert_eq!(preview.new_count, 2);
    assert_eq!(preview.duplicate_count, 0);
    let result = core.import_expenses(&preview.session_id, input)?;
    assert_eq!(result.new_count, 2);
    assert_eq!(result.duplicate_count, 0);
    assert_eq!(result.excluded_count, 0);
    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: NaiveDate::from_ymd_opt(2027, 9, 1).expect("valid date"),
        cursor: None,
        limit: 50,
    })?;
    assert_eq!(transactions.items.len(), 2);
    assert!(transactions.items.iter().all(|event| {
        event.status == tm_core::ExpenseEventStatus::Confirmed
            && event.duplicate_of_event_id.is_none()
    }));
    Ok(())
}

#[test]
fn near_time_cross_source_mirrors_require_review_without_pagination_duplicates() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let month = NaiveDate::from_ymd_opt(2027, 6, 1).expect("valid date");
    let date = NaiveDate::from_ymd_opt(2027, 6, 9).expect("valid date");
    let mut event_ids = Vec::new();
    for (adapter, source_kind, source_fingerprint, stable_key, marker, hour) in [
        (
            ExpenseImportAdapter::KbAccountHistoryV1,
            ExpenseSourceKind::Account,
            digest('a'),
            "near-account",
            '1',
            12,
        ),
        (
            ExpenseImportAdapter::KbCardUsageV1,
            ExpenseSourceKind::Card,
            digest('b'),
            "near-card",
            '2',
            13,
        ),
    ] {
        let mut purchase = row(
            &source_fingerprint,
            stable_key,
            1,
            date,
            ExpenseEventKind::Purchase,
            ExpenseDirection::Debit,
            10_000,
            Some(ExpenseCategory::Shopping),
            marker,
        );
        purchase.external_reference_fingerprint = None;
        purchase.occurred_at = format!("{date}T{hour:02}:00:00+09:00");
        purchase.merchant = Some(encrypted(
            &format!("{source_fingerprint}:{stable_key}"),
            "merchant",
            'm',
        ));
        preview_and_import(
            &core,
            NormalizedExpenseImport {
                adapter,
                source_kind,
                source_fingerprint,
                file_sha256: digest(marker),
                normalized_sha256: digest(marker),
                coverage_start: date,
                coverage_end: date,
                rejected_count: 0,
                rows: vec![purchase],
            },
        )?;
        event_ids = core
            .list_expense_transactions(ExpenseTransactionFilter {
                month_start: month,
                cursor: None,
                limit: 50,
            })?
            .items
            .into_iter()
            .map(|event| event.id)
            .collect();
    }
    assert_eq!(event_ids.len(), 2);

    let all_reviews = core.list_expense_reviews(ExpenseReviewFilter {
        month_start: Some(month),
        status: Some(ExpenseReviewStatus::Pending),
        cursor: None,
        limit: 50,
    })?;
    let ambiguous = all_reviews
        .items
        .iter()
        .find(|review| review.reason == ExpenseReviewReason::AmbiguousMirror)
        .cloned()
        .expect("ambiguous mirror review");
    let suggested_target = ambiguous
        .suggested_duplicate_of_event_id
        .clone()
        .expect("suggested duplicate target");
    assert_ne!(suggested_target, ambiguous.transaction.id);

    let mut cursor = None;
    let mut paged_ids = Vec::new();
    loop {
        let page = core.list_expense_reviews(ExpenseReviewFilter {
            month_start: Some(month),
            status: Some(ExpenseReviewStatus::Pending),
            cursor,
            limit: 1,
        })?;
        paged_ids.extend(page.items.into_iter().map(|review| review.id));
        let Some(next) = page.next_cursor else {
            break;
        };
        cursor = Some(next);
    }
    let unique_ids = paged_ids.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(unique_ids.len(), paged_ids.len());
    assert_eq!(paged_ids.len(), all_reviews.items.len());

    core.resolve_expense_review(
        &ambiguous.id,
        ResolveExpenseReviewInput {
            expected_version: ambiguous.version,
            kind: ExpenseEventKind::Purchase,
            category: ExpenseCategory::Shopping,
            duplicate_of_event_id: Some(suggested_target),
            related_event_id: None,
            personal_amount_minor: None,
            create_rule: false,
        },
    )?;
    let transactions = core.list_expense_transactions(ExpenseTransactionFilter {
        month_start: month,
        cursor: None,
        limit: 50,
    })?;
    assert_eq!(transactions.items.len(), 2);
    assert_eq!(
        transactions
            .items
            .iter()
            .filter(|event| event.status == tm_core::ExpenseEventStatus::Excluded)
            .count(),
        1
    );
    assert_eq!(
        core.get_expense_month_summary(month)?.currencies[0].gross_purchase_minor,
        10_000
    );
    Ok(())
}
