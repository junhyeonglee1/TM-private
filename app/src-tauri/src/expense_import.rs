use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read, Seek},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use calamine::{Data, DataType, Range, Reader, Sheets, Xlsx, open_workbook_auto_from_rs};
use chrono::{NaiveDate, NaiveDateTime};
use quick_xml::{
    Reader as XmlReader, XmlVersion,
    encoding::{Decoder as XmlDecoder, DecodingReader},
    events::{BytesStart, Event},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub(crate) const MAX_EXPENSE_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DECRYPTED_BYTES: usize = 40 * 1024 * 1024;
const MAX_SHEETS: usize = 10;
const MAX_ROWS: usize = 5_000;
const MAX_COLUMNS: usize = 100;
const MAX_RANGE_CELLS: usize = MAX_ROWS * MAX_COLUMNS;
const MAX_CELL_CHARS: usize = 16_384;
const MAX_ZIP_ENTRIES: usize = 2_000;
const MAX_ZIP_ENTRY_BYTES: u64 = 40 * 1024 * 1024;
const MAX_ZIP_TOTAL_BYTES: u64 = 80 * 1024 * 1024;
const MAX_ZIP_COMPRESSION_RATIO: u64 = 100;
const MAX_RELATIONSHIP_BYTES: u64 = 2 * 1024 * 1024;
const MAX_WORKBOOK_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const MAX_STYLES_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const MAX_STYLE_NUMFMTS: usize = MAX_COLUMNS * MAX_SHEETS;
const MAX_STYLE_CELL_XFS: usize = MAX_ROWS;
const MAX_BIFF_SST_CONTINUE_RECORDS: usize = 4_096;
const MAX_CFB_ENTRIES: usize = 2_000;
const CFB_FREE_SECTOR: u32 = 0xffff_ffff;
const CFB_END_OF_CHAIN: u32 = 0xffff_fffe;
const CFB_FAT_SECTOR: u32 = 0xffff_fffd;
const CFB_DIFAT_SECTOR: u32 = 0xffff_fffc;
const PREVIEW_ROWS: usize = 100;
const MAX_SAFE_AMOUNT_MINOR: i64 = 9_007_199_254_740_991;
const SESSION_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_SESSIONS: usize = 8;
const OLE_MAGIC: &[u8] = &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExpenseAdapter {
    KbCardUsageV1,
    KbAccountHistoryV1,
    KakaoPayMoneyV1,
}

impl ExpenseAdapter {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::KbCardUsageV1 => "kb_card_usage_v1",
            Self::KbAccountHistoryV1 => "kb_account_history_v1",
            Self::KakaoPayMoneyV1 => "kakaopay_money_v1",
        }
    }

    const fn source_label(self) -> &'static str {
        match self {
            Self::KbCardUsageV1 => "KB 신용카드",
            Self::KbAccountHistoryV1 => "KB 계좌·체크카드",
            Self::KakaoPayMoneyV1 => "카카오페이머니",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpenseImportPreviewRow {
    pub row_number: usize,
    pub occurred_at: String,
    pub kind: String,
    pub amount_minor: i64,
    pub currency: String,
    pub display_name: String,
    pub needs_review: bool,
    pub excluded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpenseImportCounts {
    pub parsed: usize,
    pub new: usize,
    pub duplicate: usize,
    pub settlement_candidate: usize,
    pub excluded: usize,
    pub unconfirmed: usize,
    pub rejected: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpenseImportPreview {
    pub status: &'static str,
    pub session_id: Option<String>,
    pub adapter: Option<ExpenseAdapter>,
    pub source_label: Option<String>,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub password_required: bool,
    pub counts: ExpenseImportCounts,
    pub rows: Vec<ExpenseImportPreviewRow>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreviewExpenseImportInput {
    pub path: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedExpenseFile {
    pub path: PathBuf,
    pub file_sha256: String,
    pub adapter: ExpenseAdapter,
    /// One-way identity used to keep separate cards/accounts in separate ledgers.
    /// The source value itself is discarded before the preview leaves this parser.
    pub source_discriminator_fingerprint: String,
    pub source_label: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub rejected_rows: usize,
    pub rows: Vec<ParsedExpenseRow>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedExpenseRow {
    pub row_number: usize,
    pub occurred_at: String,
    pub kind: String,
    pub category: String,
    pub direction: String,
    pub amount_minor: i64,
    pub currency: String,
    pub merchant: Option<String>,
    pub counterparty: Option<String>,
    pub note: Option<String>,
    pub payment_method_fingerprint: Option<String>,
    pub row_fingerprint: String,
    pub needs_review: bool,
    pub excluded: bool,
}

#[derive(Debug)]
struct ExpenseImportSession {
    created_at: Instant,
    parsed: ParsedExpenseFile,
}

#[derive(Debug, Default)]
pub(crate) struct ExpenseImportSessions {
    entries: Mutex<HashMap<String, ExpenseImportSession>>,
}

impl ExpenseImportSessions {
    pub(crate) fn insert(
        &self,
        session_id: String,
        parsed: ParsedExpenseFile,
    ) -> Result<(), String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "지출 가져오기 세션을 열 수 없습니다.".to_owned())?;
        entries.retain(|_, session| session.created_at.elapsed() <= SESSION_TTL);
        if entries.len() >= MAX_SESSIONS {
            return Err("열려 있는 지출 가져오기 미리보기가 너무 많습니다.".to_owned());
        }
        Uuid::parse_str(&session_id)
            .map_err(|_| "지출 가져오기 미리보기 세션이 올바르지 않습니다.".to_owned())?;
        if entries.contains_key(&session_id) {
            return Err("같은 지출 가져오기 미리보기 세션이 이미 열려 있습니다.".to_owned());
        }
        entries.insert(
            session_id,
            ExpenseImportSession {
                created_at: Instant::now(),
                parsed,
            },
        );
        Ok(())
    }

    pub(crate) fn get(&self, session_id: &str) -> Result<ParsedExpenseFile, String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "지출 가져오기 세션을 열 수 없습니다.".to_owned())?;
        entries.retain(|_, session| session.created_at.elapsed() <= SESSION_TTL);
        let session = entries.get(session_id).ok_or_else(|| {
            "지출 가져오기 미리보기가 만료됐습니다. 파일을 다시 선택하세요.".to_owned()
        })?;
        Ok(session.parsed.clone())
    }

    pub(crate) fn remove(&self, session_id: &str) -> Result<(), String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "지출 가져오기 세션을 확인할 수 없습니다.".to_owned())?;
        entries.remove(session_id);
        Ok(())
    }
}

impl ParsedExpenseFile {
    pub(crate) fn preview(&self, session_id: String) -> ExpenseImportPreview {
        let excluded = self.rows.iter().filter(|row| row.excluded).count();
        let unconfirmed = self.rows.iter().filter(|row| row.needs_review).count();
        ExpenseImportPreview {
            status: "ready",
            session_id: Some(session_id),
            adapter: Some(self.adapter),
            source_label: Some(self.source_label.clone()),
            period_start: Some(self.period_start.to_string()),
            period_end: Some(self.period_end.to_string()),
            password_required: false,
            counts: ExpenseImportCounts {
                parsed: self.rows.len(),
                new: self.rows.len(),
                duplicate: 0,
                settlement_candidate: 0,
                excluded,
                unconfirmed,
                rejected: self.rejected_rows,
            },
            rows: self
                .rows
                .iter()
                .take(PREVIEW_ROWS)
                .map(|row| ExpenseImportPreviewRow {
                    row_number: row.row_number,
                    occurred_at: row.occurred_at.clone(),
                    kind: row.kind.clone(),
                    amount_minor: row.amount_minor,
                    currency: row.currency.clone(),
                    display_name: row
                        .merchant
                        .as_deref()
                        .or(row.counterparty.as_deref())
                        .unwrap_or("표시 정보 없음")
                        .to_owned(),
                    needs_review: row.needs_review,
                    excluded: row.excluded,
                })
                .collect(),
        }
    }
}

pub(crate) fn password_required_preview() -> ExpenseImportPreview {
    ExpenseImportPreview {
        status: "password_required",
        session_id: None,
        adapter: Some(ExpenseAdapter::KakaoPayMoneyV1),
        source_label: Some(ExpenseAdapter::KakaoPayMoneyV1.source_label().to_owned()),
        period_start: None,
        period_end: None,
        password_required: true,
        counts: ExpenseImportCounts {
            parsed: 0,
            new: 0,
            duplicate: 0,
            settlement_candidate: 0,
            excluded: 0,
            unconfirmed: 0,
            rejected: 0,
        },
        rows: Vec::new(),
    }
}

pub(crate) fn parse_expense_file(
    input: &PreviewExpenseImportInput,
    decrypt: impl FnOnce(&[u8], &str) -> Result<Vec<u8>, String>,
) -> Result<Option<ParsedExpenseFile>, String> {
    let path = canonical_expense_path(&input.path)?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "지출 파일 확장자를 확인할 수 없습니다.".to_owned())?;
    if !matches!(extension.as_str(), "xls" | "xlsx") {
        return Err("현재는 KB·카카오페이 XLS 또는 XLSX 파일만 지원합니다.".to_owned());
    }
    let metadata = fs::metadata(&path).map_err(|_| "지출 파일을 읽을 수 없습니다.".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXPENSE_FILE_BYTES {
        return Err("지출 파일 크기가 허용 범위를 벗어났습니다.".to_owned());
    }
    let encrypted_bytes =
        fs::read(&path).map_err(|_| "지출 파일을 읽을 수 없습니다.".to_owned())?;
    let file_sha256 = hex_sha256(&encrypted_bytes);
    let mut workbook_bytes = if extension == "xlsx" && encrypted_bytes.starts_with(OLE_MAGIC) {
        let Some(password) = input.password.as_deref().filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let decrypted = decrypt(&encrypted_bytes, password)?;
        if decrypted.len() > MAX_DECRYPTED_BYTES {
            return Err("복호화한 지출 파일이 안전 제한을 초과했습니다.".to_owned());
        }
        decrypted
    } else {
        encrypted_bytes
    };

    if extension == "xlsx" && !workbook_bytes.starts_with(b"PK") {
        return Err("XLSX 파일 형식이 올바르지 않습니다.".to_owned());
    }
    if extension == "xls" && !workbook_bytes.starts_with(OLE_MAGIC) {
        return Err("XLS 파일 형식이 올바르지 않습니다.".to_owned());
    }
    if extension == "xlsx" {
        validate_ooxml_container(&workbook_bytes)?;
    } else {
        validate_xls_container_for_import(&mut workbook_bytes)?;
    }

    let mut workbook = open_workbook_auto_from_rs(Cursor::new(workbook_bytes))
        .map_err(|_| "지출 통합문서를 열 수 없습니다.".to_owned())?;
    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() || sheet_names.len() > MAX_SHEETS {
        return Err("지출 통합문서의 시트 수가 허용 범위를 벗어났습니다.".to_owned());
    }

    if let Sheets::Xlsx(xlsx) = &mut workbook {
        validate_xlsx_workbook_ranges(xlsx, &sheet_names)?;
    }

    let mut total_rows = 0_usize;
    let mut parsed_candidate: Option<(
        ExpenseAdapter,
        String,
        NaiveDate,
        NaiveDate,
        usize,
        Vec<ParsedExpenseRow>,
    )> = None;
    for sheet_name in sheet_names {
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|_| "지출 통합문서의 시트를 읽을 수 없습니다.".to_owned())?;
        validate_range(&range)?;
        total_rows = total_rows
            .checked_add(range.get_size().0)
            .ok_or_else(|| "지출 통합문서의 전체 행 수를 계산할 수 없습니다.".to_owned())?;
        if total_rows > MAX_ROWS {
            return Err("지출 통합문서의 전체 행 수가 안전 제한을 초과했습니다.".to_owned());
        }
        if let Some(adapter) = detect_adapter(&range) {
            let (rows, rejected_rows) = match adapter {
                ExpenseAdapter::KbCardUsageV1 => parse_kb_card(&range)?,
                ExpenseAdapter::KbAccountHistoryV1 => parse_kb_account(&range)?,
                ExpenseAdapter::KakaoPayMoneyV1 => parse_kakao_pay(&range)?,
            };
            if rows.is_empty() {
                continue;
            }
            let mut dates = rows.iter().filter_map(|row| {
                NaiveDate::parse_from_str(&row.occurred_at[..10], "%Y-%m-%d").ok()
            });
            let first = dates
                .next()
                .ok_or_else(|| "지출 파일의 거래 기간을 확인할 수 없습니다.".to_owned())?;
            let (actual_period_start, actual_period_end) = dates
                .fold((first, first), |(start, end), date| {
                    (start.min(date), end.max(date))
                });
            let (period_start, period_end) = declared_coverage_period(&range)
                .unwrap_or((actual_period_start, actual_period_end));
            if actual_period_start < period_start || actual_period_end > period_end {
                return Err("지출 파일의 조회 기간과 거래 날짜가 일치하지 않습니다.".to_owned());
            }
            let source_fingerprint = source_discriminator_fingerprint(&range, adapter, &rows);
            if let Some((
                current_adapter,
                current_source_fingerprint,
                current_period_start,
                current_period_end,
                current_rejected_rows,
                current_rows,
            )) = parsed_candidate.as_mut()
            {
                if *current_adapter != adapter {
                    return Err(
                        "서로 다른 형식의 거래 시트가 한 파일에 함께 있어 가져올 수 없습니다."
                            .to_owned(),
                    );
                }
                if *current_source_fingerprint != source_fingerprint {
                    return Err(
                        "서로 다른 카드·계좌의 거래 시트가 한 파일에 함께 있어 가져올 수 없습니다."
                            .to_owned(),
                    );
                }
                *current_period_start = (*current_period_start).min(period_start);
                *current_period_end = (*current_period_end).max(period_end);
                *current_rejected_rows = current_rejected_rows
                    .checked_add(rejected_rows)
                    .ok_or_else(|| "지출 거부 행 수를 계산할 수 없습니다.".to_owned())?;
                current_rows.extend(rows);
            } else {
                parsed_candidate = Some((
                    adapter,
                    source_fingerprint,
                    period_start,
                    period_end,
                    rejected_rows,
                    rows,
                ));
            }
        }
    }

    let Some((
        adapter,
        source_discriminator_fingerprint,
        period_start,
        period_end,
        rejected_rows,
        rows,
    )) = parsed_candidate
    else {
        return Err("지원하는 KB·카카오페이 거래내역 형식을 찾지 못했습니다.".to_owned());
    };
    Ok(Some(ParsedExpenseFile {
        path,
        file_sha256,
        adapter,
        source_discriminator_fingerprint,
        source_label: adapter.source_label().to_owned(),
        period_start,
        period_end,
        rejected_rows,
        rows,
    }))
}

fn source_discriminator_fingerprint(
    range: &Range<Data>,
    adapter: ExpenseAdapter,
    _rows: &[ParsedExpenseRow],
) -> String {
    let labels: &[&str] = match adapter {
        ExpenseAdapter::KbCardUsageV1 => &["카드번호", "카드 번호"],
        ExpenseAdapter::KbAccountHistoryV1 => &["계좌번호", "계좌 번호"],
        ExpenseAdapter::KakaoPayMoneyV1 => &[],
    };
    for row in range.rows().take(20) {
        for (index, cell) in row.iter().enumerate() {
            let label = cell_text(cell);
            if !labels.iter().any(|candidate| label.contains(candidate)) {
                continue;
            }
            if let Some(identity) = row
                .iter()
                .skip(index + 1)
                .take(3)
                .map(cell_text)
                .find(|value| !value.trim().is_empty())
            {
                return hex_sha256(
                    format!(
                        "tm-expense:source-identity:v1:{}:{identity}",
                        adapter.as_str()
                    )
                    .as_bytes(),
                );
            }
        }
    }

    hex_sha256(
        format!(
            "tm-expense:source-identity:v1:{}:{}",
            adapter.as_str(),
            "default"
        )
        .as_bytes(),
    )
}

pub(crate) fn verify_source_unchanged(parsed: &ParsedExpenseFile) -> Result<(), String> {
    let metadata = fs::metadata(&parsed.path)
        .map_err(|_| "미리보기 이후 지출 원본 파일을 찾을 수 없습니다.".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXPENSE_FILE_BYTES {
        return Err("미리보기 이후 지출 원본 파일 상태가 변경됐습니다.".to_owned());
    }
    let bytes = fs::read(&parsed.path)
        .map_err(|_| "미리보기 이후 지출 원본 파일을 다시 읽을 수 없습니다.".to_owned())?;
    if hex_sha256(&bytes) != parsed.file_sha256 {
        return Err(
            "미리보기 이후 지출 원본 파일이 변경됐습니다. 다시 미리보기 하세요.".to_owned(),
        );
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy)]
struct SheetCellBounds {
    min_row: Option<u32>,
    max_row: u32,
    min_column: Option<u32>,
    max_column: u32,
}

impl SheetCellBounds {
    fn observe(&mut self, row: u32, column: u32) -> Result<(), String> {
        validate_sheet_position(row, column)?;
        self.min_row = Some(self.min_row.map_or(row, |current| current.min(row)));
        self.max_row = self.max_row.max(row);
        self.min_column = Some(
            self.min_column
                .map_or(column, |current| current.min(column)),
        );
        self.max_column = self.max_column.max(column);
        Ok(())
    }

    fn height(self) -> Result<usize, String> {
        let Some(min_row) = self.min_row else {
            return Ok(0);
        };
        usize::try_from(self.max_row - min_row + 1)
            .map_err(|_| "The worksheet row span exceeds the safety limit.".to_owned())
    }

    fn area(self) -> Result<usize, String> {
        let Some(min_row) = self.min_row else {
            return Ok(0);
        };
        let min_column = self
            .min_column
            .ok_or_else(|| "The worksheet cell bounds are invalid.".to_owned())?;
        let height = usize::try_from(self.max_row - min_row + 1)
            .map_err(|_| "The worksheet row span exceeds the safety limit.".to_owned())?;
        let width = usize::try_from(self.max_column - min_column + 1)
            .map_err(|_| "The worksheet column span exceeds the safety limit.".to_owned())?;
        height
            .checked_mul(width)
            .ok_or_else(|| "The worksheet cell span overflowed.".to_owned())
    }
}

fn validate_sheet_position(row: u32, column: u32) -> Result<(), String> {
    let row = usize::try_from(row)
        .map_err(|_| "The worksheet row address exceeds the safety limit.".to_owned())?;
    let column = usize::try_from(column)
        .map_err(|_| "The worksheet column address exceeds the safety limit.".to_owned())?;
    if row >= MAX_ROWS || column >= MAX_COLUMNS {
        return Err("The worksheet cell address exceeds the safety limit.".to_owned());
    }
    Ok(())
}

fn validate_xlsx_workbook_ranges<RS: Read + Seek>(
    workbook: &mut Xlsx<RS>,
    sheet_names: &[String],
) -> Result<(), String> {
    let mut total_rows = 0_usize;
    let mut total_cells = 0_usize;
    let mut total_dense_cells = 0_usize;

    for sheet_name in sheet_names {
        let mut reader = workbook
            .worksheet_cells_reader(sheet_name)
            .map_err(|_| "The XLSX worksheet cannot be preflighted safely.".to_owned())?;
        let mut bounds = SheetCellBounds::default();
        while let Some(cell) = reader
            .next_cell()
            .map_err(|_| "The XLSX worksheet cell stream is invalid.".to_owned())?
        {
            if cell.get_value().is_empty() {
                continue;
            }
            total_cells = total_cells
                .checked_add(1)
                .ok_or_else(|| "The XLSX worksheet cell count overflowed.".to_owned())?;
            if total_cells > MAX_RANGE_CELLS {
                return Err("The XLSX worksheet cell count exceeds the safety limit.".to_owned());
            }
            let (row, column) = cell.get_position();
            bounds.observe(row, column)?;
        }

        total_rows = total_rows
            .checked_add(bounds.height()?)
            .ok_or_else(|| "The XLSX worksheet row count overflowed.".to_owned())?;
        total_dense_cells = total_dense_cells
            .checked_add(bounds.area()?)
            .ok_or_else(|| "The XLSX worksheet cell span overflowed.".to_owned())?;
        if total_rows > MAX_ROWS || total_dense_cells > MAX_RANGE_CELLS {
            return Err("The XLSX worksheet dimensions exceed the safety limit.".to_owned());
        }
    }
    Ok(())
}

fn canonical_expense_path(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err("지출 파일 경로가 올바르지 않습니다.".to_owned());
    }
    fs::canonicalize(trimmed).map_err(|_| "선택한 지출 파일을 찾을 수 없습니다.".to_owned())
}

fn validate_range(range: &Range<Data>) -> Result<(), String> {
    let (height, width) = range.get_size();
    let max_rows = u32::try_from(MAX_ROWS)
        .map_err(|_| "지출 시트 행 안전 제한이 올바르지 않습니다.".to_owned())?;
    let max_columns = u32::try_from(MAX_COLUMNS)
        .map_err(|_| "지출 시트 열 안전 제한이 올바르지 않습니다.".to_owned())?;
    let exceeds_address_limit = range
        .end()
        .is_some_and(|(last_row, last_column)| last_row >= max_rows || last_column >= max_columns);
    if height > MAX_ROWS || width > MAX_COLUMNS || exceeds_address_limit {
        return Err("지출 시트의 행 또는 열 수가 안전 제한을 초과했습니다.".to_owned());
    }
    for cell in range.used_cells().map(|(_, _, cell)| cell) {
        if cell_text(cell).chars().count() > MAX_CELL_CHARS {
            return Err("지출 시트의 셀 문자열이 안전 제한을 초과했습니다.".to_owned());
        }
    }
    Ok(())
}

fn read_cfb_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| "The XLS compound document header is truncated.".to_owned())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_cfb_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| "The XLS compound document is truncated.".to_owned())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_cfb_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset.saturating_add(8))
        .ok_or_else(|| "The XLS compound document is truncated.".to_owned())?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

fn cfb_sector(
    bytes: &[u8],
    sector_size: usize,
    sector_count: usize,
    sector_id: u32,
) -> Result<&[u8], String> {
    let sector_index = usize::try_from(sector_id)
        .ok()
        .filter(|index| *index < sector_count)
        .ok_or_else(|| "The XLS compound document references an invalid sector.".to_owned())?;
    let start = sector_index
        .checked_add(1)
        .and_then(|index| index.checked_mul(sector_size))
        .ok_or_else(|| "The XLS compound document sector offset overflowed.".to_owned())?;
    let end = start
        .checked_add(sector_size)
        .ok_or_else(|| "The XLS compound document sector offset overflowed.".to_owned())?;
    bytes
        .get(start..end)
        .ok_or_else(|| "The XLS compound document sector is truncated.".to_owned())
}

fn add_cfb_fat_sector(
    sector_id: u32,
    sector_count: usize,
    expected_count: usize,
    sectors: &mut Vec<u32>,
    seen: &mut [bool],
) -> Result<(), String> {
    if sector_id == CFB_FREE_SECTOR {
        return Ok(());
    }
    let index = usize::try_from(sector_id)
        .ok()
        .filter(|index| *index < sector_count)
        .ok_or_else(|| "The XLS DIFAT references an invalid FAT sector.".to_owned())?;
    if sectors.len() >= expected_count || seen[index] {
        return Err("The XLS DIFAT contains duplicate or excess FAT sectors.".to_owned());
    }
    seen[index] = true;
    sectors.push(sector_id);
    Ok(())
}

fn cfb_fat_entry_offset(
    sector_size: usize,
    sector_count: usize,
    fat_sectors: &[u32],
    sector_id: u32,
) -> Result<usize, String> {
    let index = usize::try_from(sector_id)
        .ok()
        .filter(|index| *index < sector_count)
        .ok_or_else(|| "The XLS FAT lookup references an invalid sector.".to_owned())?;
    let entries_per_sector = sector_size / 4;
    let fat_sector_index = index / entries_per_sector;
    let entry_index = index % entries_per_sector;
    let fat_sector_id = *fat_sectors
        .get(fat_sector_index)
        .ok_or_else(|| "The XLS FAT table is incomplete.".to_owned())?;
    let fat_sector_index = usize::try_from(fat_sector_id)
        .ok()
        .filter(|index| *index < sector_count)
        .ok_or_else(|| "The XLS compound document references an invalid sector.".to_owned())?;
    fat_sector_index
        .checked_add(1)
        .and_then(|index| index.checked_mul(sector_size))
        .and_then(|offset| offset.checked_add(entry_index * 4))
        .ok_or_else(|| "The XLS compound document sector offset overflowed.".to_owned())
}

fn cfb_fat_entry(
    bytes: &[u8],
    sector_size: usize,
    sector_count: usize,
    fat_sectors: &[u32],
    sector_id: u32,
) -> Result<u32, String> {
    let offset = cfb_fat_entry_offset(sector_size, sector_count, fat_sectors, sector_id)?;
    read_cfb_u32(bytes, offset)
}

fn decode_cfb_entry_name(entry: &[u8]) -> Result<String, String> {
    let name_bytes = usize::from(read_cfb_u16(entry, 64)?);
    if !(2..=64).contains(&name_bytes) || name_bytes % 2 != 0 {
        return Err("The XLS directory entry name is invalid.".to_owned());
    }
    let units = entry[..name_bytes]
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    if units.last() != Some(&0) || units[..units.len() - 1].contains(&0) {
        return Err("The XLS directory entry name is invalid.".to_owned());
    }
    String::from_utf16(&units[..units.len() - 1])
        .map_err(|_| "The XLS directory entry name is invalid UTF-16.".to_owned())
}

fn is_forbidden_cfb_entry_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    let visible = normalized.trim_start_matches(char::is_control);
    matches!(
        visible,
        "_vba_project_cur"
            | "vba"
            | "macros"
            | "project"
            | "projectwm"
            | "vba_project"
            | "objectpool"
            | "ocxname"
            | "activex"
            | "ole"
            | "ole10native"
            | "package"
    ) || visible.starts_with("mbd")
        || visible.contains("activex")
        || visible.starts_with("olepres")
}

struct XlsContainerPreflight {
    workbook_path: PathBuf,
    legacy_fat_marker_offset: Option<usize>,
}

fn preflight_xls_container(bytes: &[u8]) -> Result<XlsContainerPreflight, String> {
    if bytes.len() < 512
        || bytes.len() as u64 > MAX_EXPENSE_FILE_BYTES
        || bytes.get(..OLE_MAGIC.len()) != Some(OLE_MAGIC)
    {
        return Err("The XLS compound document header is invalid.".to_owned());
    }
    let major_version = read_cfb_u16(bytes, 26)?;
    let byte_order = read_cfb_u16(bytes, 28)?;
    let sector_shift = read_cfb_u16(bytes, 30)?;
    if byte_order != 0xfffe || !matches!((major_version, sector_shift), (3, 9) | (4, 12)) {
        return Err("The XLS compound document version is unsupported.".to_owned());
    }
    let sector_size = 1_usize
        .checked_shl(u32::from(sector_shift))
        .ok_or_else(|| "The XLS compound document sector size is invalid.".to_owned())?;
    if bytes.len() < sector_size.saturating_mul(2) || !bytes.len().is_multiple_of(sector_size) {
        return Err("The XLS compound document length is invalid.".to_owned());
    }
    let sector_count = bytes.len() / sector_size - 1;
    let fat_sector_count = usize::try_from(read_cfb_u32(bytes, 44)?)
        .map_err(|_| "The XLS FAT sector count is invalid.".to_owned())?;
    if fat_sector_count == 0 || fat_sector_count > sector_count {
        return Err("The XLS FAT sector count is invalid.".to_owned());
    }

    let mut fat_sectors = Vec::with_capacity(fat_sector_count);
    let mut seen_fat = vec![false; sector_count];
    for index in 0..109 {
        add_cfb_fat_sector(
            read_cfb_u32(bytes, 76 + index * 4)?,
            sector_count,
            fat_sector_count,
            &mut fat_sectors,
            &mut seen_fat,
        )?;
    }

    let difat_sector_count = usize::try_from(read_cfb_u32(bytes, 72)?)
        .map_err(|_| "The XLS DIFAT sector count is invalid.".to_owned())?;
    if difat_sector_count > sector_count {
        return Err("The XLS DIFAT sector count is invalid.".to_owned());
    }
    let mut difat_sector_id = read_cfb_u32(bytes, 68)?;
    let mut seen_difat = vec![false; sector_count];
    let difat_entries_per_sector = sector_size / 4 - 1;
    for _ in 0..difat_sector_count {
        let difat_index = usize::try_from(difat_sector_id)
            .ok()
            .filter(|index| *index < sector_count)
            .ok_or_else(|| "The XLS DIFAT chain is invalid.".to_owned())?;
        if seen_difat[difat_index] || seen_fat[difat_index] {
            return Err("The XLS DIFAT chain contains a cycle or overlap.".to_owned());
        }
        seen_difat[difat_index] = true;
        let difat_sector = cfb_sector(bytes, sector_size, sector_count, difat_sector_id)?;
        for index in 0..difat_entries_per_sector {
            add_cfb_fat_sector(
                read_cfb_u32(difat_sector, index * 4)?,
                sector_count,
                fat_sector_count,
                &mut fat_sectors,
                &mut seen_fat,
            )?;
        }
        difat_sector_id = read_cfb_u32(difat_sector, difat_entries_per_sector * 4)?;
    }
    if difat_sector_id != CFB_END_OF_CHAIN || fat_sectors.len() != fat_sector_count {
        return Err("The XLS DIFAT chain or FAT sector count is inconsistent.".to_owned());
    }
    if seen_fat
        .iter()
        .zip(&seen_difat)
        .any(|(fat, difat)| *fat && *difat)
    {
        return Err("The XLS FAT and DIFAT sectors overlap.".to_owned());
    }
    for sector_id in 0..sector_count {
        let next = cfb_fat_entry(
            bytes,
            sector_size,
            sector_count,
            &fat_sectors,
            sector_id as u32,
        )?;
        if let Ok(next_index) = usize::try_from(next)
            && next_index < sector_count
            && (seen_fat[next_index] || seen_difat[next_index])
        {
            return Err("The XLS FAT chain references a reserved metadata sector.".to_owned());
        }
    }

    let mut legacy_fat_marker_offset = None;
    for (index, is_fat) in seen_fat.iter().enumerate() {
        if !*is_fat {
            continue;
        }
        let marker = cfb_fat_entry(bytes, sector_size, sector_count, &fat_sectors, index as u32)?;
        if marker == CFB_FAT_SECTOR {
            continue;
        }
        let fat_sector = cfb_sector(bytes, sector_size, sector_count, fat_sectors[0])?;
        let mut unused_fat_entries_are_free = true;
        for entry_index in sector_count..(sector_size / 4) {
            if read_cfb_u32(fat_sector, entry_index * 4)? != CFB_FREE_SECTOR {
                unused_fat_entries_are_free = false;
                break;
            }
        }
        let is_supported_legacy_marker = marker == CFB_END_OF_CHAIN
            && major_version == 3
            && fat_sector_count == 1
            && difat_sector_count == 0
            && index + 1 == sector_count
            && unused_fat_entries_are_free;
        if !is_supported_legacy_marker || legacy_fat_marker_offset.is_some() {
            return Err("The XLS FAT sector marker is invalid.".to_owned());
        }
        legacy_fat_marker_offset = Some(cfb_fat_entry_offset(
            sector_size,
            sector_count,
            &fat_sectors,
            index as u32,
        )?);
    }
    for (index, is_difat) in seen_difat.iter().enumerate() {
        if *is_difat
            && cfb_fat_entry(bytes, sector_size, sector_count, &fat_sectors, index as u32)?
                != CFB_DIFAT_SECTOR
        {
            return Err("The XLS DIFAT sector marker is invalid.".to_owned());
        }
    }

    let declared_directory_sectors = usize::try_from(read_cfb_u32(bytes, 40)?)
        .map_err(|_| "The XLS directory sector count is invalid.".to_owned())?;
    if major_version == 3 && declared_directory_sectors != 0 {
        return Err("The XLS version 3 directory sector count is invalid.".to_owned());
    }
    let entries_per_directory_sector = sector_size / 128;
    let max_directory_sectors = MAX_CFB_ENTRIES.div_ceil(entries_per_directory_sector);
    let mut seen_directory = vec![false; sector_count];
    let mut directory_sector_id = read_cfb_u32(bytes, 48)?;
    let mut directory_sector_count = 0_usize;
    let mut nonempty_entry_count = 0_usize;
    let mut workbook_name = None;
    let mut root_seen = false;
    while directory_sector_id != CFB_END_OF_CHAIN {
        let directory_index = usize::try_from(directory_sector_id)
            .ok()
            .filter(|index| *index < sector_count)
            .ok_or_else(|| "The XLS directory chain references an invalid sector.".to_owned())?;
        if seen_directory[directory_index]
            || seen_fat[directory_index]
            || seen_difat[directory_index]
        {
            return Err("The XLS directory chain contains a cycle or overlap.".to_owned());
        }
        seen_directory[directory_index] = true;
        directory_sector_count = directory_sector_count
            .checked_add(1)
            .filter(|count| *count <= max_directory_sectors)
            .ok_or_else(|| "The XLS compound document exceeds the entry limit.".to_owned())?;
        let directory_sector = cfb_sector(bytes, sector_size, sector_count, directory_sector_id)?;
        for slot in 0..entries_per_directory_sector {
            let entry_id = (directory_sector_count - 1) * entries_per_directory_sector + slot;
            let entry_start = slot * 128;
            let entry = &directory_sector[entry_start..entry_start + 128];
            let object_type = entry[66];
            if entry_id == 0 && object_type != 5 {
                return Err("The XLS root directory entry is invalid.".to_owned());
            }
            if object_type == 0 {
                continue;
            }
            if !matches!(object_type, 1 | 2 | 5) {
                return Err("The XLS directory entry type is invalid.".to_owned());
            }
            nonempty_entry_count = nonempty_entry_count
                .checked_add(1)
                .filter(|count| *count <= MAX_CFB_ENTRIES)
                .ok_or_else(|| "The XLS compound document exceeds the entry limit.".to_owned())?;
            if object_type == 5 {
                if entry_id != 0 || root_seen {
                    return Err("The XLS root directory entry is duplicated.".to_owned());
                }
                root_seen = true;
            }

            let name = decode_cfb_entry_name(entry)?;
            if is_forbidden_cfb_entry_name(&name)
                || (object_type == 1 && entry[80..96].iter().any(|byte| *byte != 0))
            {
                return Err(
                    "XLS files containing macros, OLE objects, or ActiveX are not accepted."
                        .to_owned(),
                );
            }
            let normalized = name.to_ascii_lowercase();
            if object_type == 2 && matches!(normalized.as_str(), "workbook" | "book") {
                let stream_len = read_cfb_u64(entry, 120)?;
                if (major_version == 3 && stream_len > u64::from(u32::MAX))
                    || stream_len == 0
                    || stream_len > MAX_EXPENSE_FILE_BYTES
                    || workbook_name.replace(name).is_some()
                {
                    return Err("The XLS workbook stream is invalid or duplicated.".to_owned());
                }
            }
        }

        let next = cfb_fat_entry(
            bytes,
            sector_size,
            sector_count,
            &fat_sectors,
            directory_sector_id,
        )?;
        if next != CFB_END_OF_CHAIN && usize::try_from(next).map_or(true, |id| id >= sector_count) {
            return Err("The XLS directory chain terminator is invalid.".to_owned());
        }
        directory_sector_id = next;
    }
    if !root_seen || nonempty_entry_count == 0 {
        return Err("The XLS compound document contains no valid directory.".to_owned());
    }
    if major_version == 4 && declared_directory_sectors != directory_sector_count {
        return Err("The XLS directory sector count is inconsistent.".to_owned());
    }
    let workbook_name = workbook_name
        .ok_or_else(|| "The XLS file must contain exactly one workbook stream.".to_owned())?;
    Ok(XlsContainerPreflight {
        workbook_path: PathBuf::from(format!("/{workbook_name}")),
        legacy_fat_marker_offset,
    })
}

fn validate_xls_container_for_import(bytes: &mut [u8]) -> Result<(), String> {
    // Parse and bound the raw directory/FAT first. cfb::Entries::walk intentionally is not used:
    // an adversarial unbalanced sibling tree can otherwise allocate before caller-side limits run.
    let preflight = preflight_xls_container(bytes)?;
    if let Some(offset) = preflight.legacy_fat_marker_offset {
        bytes
            .get_mut(offset..offset.saturating_add(4))
            .ok_or_else(|| "The XLS FAT sector marker is truncated.".to_owned())?
            .copy_from_slice(&CFB_FAT_SECTOR.to_le_bytes());
    }
    let mut compound = cfb::CompoundFile::open(Cursor::new(&*bytes))
        .map_err(|_| "The XLS compound document is invalid.".to_owned())?;
    let mut stream = compound
        .open_stream(&preflight.workbook_path)
        .map_err(|_| {
            "The XLS workbook stream must be located directly under the root.".to_owned()
        })?;
    let mut workbook = Vec::new();
    (&mut stream)
        .take(MAX_EXPENSE_FILE_BYTES + 1)
        .read_to_end(&mut workbook)
        .map_err(|_| "The XLS workbook stream cannot be read.".to_owned())?;
    if workbook.len() as u64 > MAX_EXPENSE_FILE_BYTES {
        return Err("The XLS workbook stream exceeds the safety limit.".to_owned());
    }
    validate_biff_workbook_stream(&workbook)
}

#[cfg(test)]
fn validate_xls_container(bytes: &[u8]) -> Result<(), String> {
    let mut normalized = bytes.to_vec();
    validate_xls_container_for_import(&mut normalized)
}

fn validate_biff_workbook_stream(bytes: &[u8]) -> Result<(), String> {
    let mut offset = 0_usize;
    let mut saw_bof = false;
    let mut saw_sst = false;
    let mut sheet_offsets = Vec::new();
    let mut workbook_biff_version = None;
    let mut self_referencing_supbooks = Vec::new();
    let mut continuation_chain = 0_usize;
    while offset < bytes.len() {
        let Some(record) = read_biff_record(bytes, offset)? else {
            break;
        };
        let record_type = record.record_type;
        let payload = record.payload;

        if record_type == 0x003c {
            if payload.is_empty() {
                return Err("The XLS stream contains an empty continuation record.".to_owned());
            }
            continuation_chain = continuation_chain
                .checked_add(1)
                .ok_or_else(|| "The XLS continuation count overflowed.".to_owned())?;
            if continuation_chain > MAX_BIFF_SST_CONTINUE_RECORDS {
                return Err("The XLS stream has too many continuation records.".to_owned());
            }
        } else {
            continuation_chain = 0;
        }

        if matches!(record_type, 0x0009 | 0x0209 | 0x0409 | 0x0809) {
            if payload.len() < 4 {
                return Err("The XLS BOF record is invalid.".to_owned());
            }
            saw_bof = true;
            let substream_type = u16::from_le_bytes([payload[2], payload[3]]);
            let biff_version = normalized_biff_version(
                u16::from_le_bytes([payload[0], payload[1]]),
                substream_type,
            );
            if substream_type == 0x0040 {
                return Err("XLS macro sheets are not accepted.".to_owned());
            }
            if substream_type == 0x0005 && workbook_biff_version.replace(biff_version).is_some() {
                return Err("The XLS workbook contains duplicate global substreams.".to_owned());
            }
        }
        if record_type == 0x0085 {
            if payload.len() < 6 {
                return Err("The XLS BoundSheet record is invalid.".to_owned());
            }
            if matches!(payload[5] & 0x0f, 0x01 | 0x06) {
                return Err("XLS macro or VBA module sheets are not accepted.".to_owned());
            }
            if sheet_offsets.len() >= MAX_SHEETS {
                return Err("The XLS workbook sheet count exceeds the safety limit.".to_owned());
            }
            let sheet_offset = usize::try_from(u32::from_le_bytes([
                payload[0], payload[1], payload[2], payload[3],
            ]))
            .map_err(|_| "The XLS worksheet offset is invalid.".to_owned())?;
            if sheet_offsets.contains(&sheet_offset) {
                return Err("The XLS workbook contains duplicate worksheet offsets.".to_owned());
            }
            sheet_offsets.push(sheet_offset);
        }
        if record_type == 0x01ae {
            if payload.len() != 4 || workbook_biff_version.is_none_or(|version| version < 0x0600) {
                return Err("The XLS SupBook record is invalid.".to_owned());
            }
            let character_count = u16::from_le_bytes([payload[2], payload[3]]);
            if !matches!(character_count, 0x0401 | 0x3a01) {
                return Err("XLS external workbook links are not accepted.".to_owned());
            }
            if self_referencing_supbooks.len() >= MAX_SHEETS {
                return Err("The XLS supporting-link count exceeds the safety limit.".to_owned());
            }
            self_referencing_supbooks.push(character_count == 0x0401);
        }
        if record_type == 0x0017 {
            let biff_version = workbook_biff_version.ok_or_else(|| {
                "The XLS ExternSheet record precedes its workbook BOF.".to_owned()
            })?;
            validate_biff_extern_sheet(payload, biff_version, &self_referencing_supbooks)?;
        }
        if record_type == 0x00fc {
            if saw_sst {
                return Err("The XLS workbook contains duplicate SST records.".to_owned());
            }
            saw_sst = true;
            validate_biff_sst(bytes, payload, record.next_offset)?;
        }
        if record_type == 0x013d && (payload.len() % 2 != 0 || payload.len() / 2 > MAX_SHEETS) {
            return Err("The XLS RRTabId sheet allocation exceeds the safety limit.".to_owned());
        }
        if matches!(
            record_type,
            0x0023 | 0x0059 | 0x005a | 0x005d | 0x00d3 | 0x01b8
        ) {
            return Err(
                "XLS external links, embedded objects, and macro projects are not accepted."
                    .to_owned(),
            );
        }
        offset = record.next_offset;
    }
    if !saw_bof {
        return Err("The XLS workbook stream has no BIFF BOF record.".to_owned());
    }

    let mut total_rows = 0_usize;
    let mut total_cells = 0_usize;
    let mut total_dense_cells = 0_usize;
    let mut total_formula_cells = 0_usize;
    let mut total_peak_reserved_cells = 0_usize;
    for sheet_offset in sheet_offsets {
        let stats = validate_biff_sheet(bytes, sheet_offset)?;
        total_rows = total_rows
            .checked_add(stats.bounds.height()?)
            .ok_or_else(|| "The XLS worksheet row count overflowed.".to_owned())?;
        total_cells = total_cells
            .checked_add(stats.raw_cells)
            .ok_or_else(|| "The XLS worksheet cell count overflowed.".to_owned())?;
        total_dense_cells = total_dense_cells
            .checked_add(stats.bounds.area()?)
            .and_then(|count| count.checked_add(stats.formula_bounds.area().ok()?))
            .ok_or_else(|| "The XLS worksheet allocation size overflowed.".to_owned())?;
        total_formula_cells = total_formula_cells
            .checked_add(stats.formula_cells)
            .ok_or_else(|| "The XLS worksheet formula count overflowed.".to_owned())?;
        total_peak_reserved_cells = total_peak_reserved_cells
            .checked_add(stats.peak_reserved_cells)
            .ok_or_else(|| "The XLS worksheet declared size overflowed.".to_owned())?;
        if total_rows > MAX_ROWS
            || total_cells > MAX_RANGE_CELLS
            || total_dense_cells > MAX_RANGE_CELLS.saturating_mul(2)
            || total_formula_cells > MAX_RANGE_CELLS
            || total_peak_reserved_cells > MAX_RANGE_CELLS
        {
            return Err("The XLS workbook dimensions exceed the safety limit.".to_owned());
        }
    }
    Ok(())
}

fn validate_biff_extern_sheet(
    payload: &[u8],
    biff_version: u16,
    self_referencing_supbooks: &[bool],
) -> Result<(), String> {
    if biff_version < 0x0600 {
        if payload.len() < 2 {
            return Err("The legacy XLS ExternSheet record is truncated.".to_owned());
        }
        let sheet_name_bytes = usize::from(payload[0]);
        if sheet_name_bytes == 0
            || payload[1] != 0x03
            || payload.len() != sheet_name_bytes.saturating_add(2)
        {
            return Err("Legacy XLS external workbook links are not accepted.".to_owned());
        }
        return Ok(());
    }

    if payload.len() < 2 {
        return Err("The XLS ExternSheet record is truncated.".to_owned());
    }
    let reference_count = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
    let expected_length = reference_count
        .checked_mul(6)
        .and_then(|length| length.checked_add(2))
        .ok_or_else(|| "The XLS ExternSheet allocation overflowed.".to_owned())?;
    if payload.len() != expected_length || reference_count > MAX_RANGE_CELLS {
        return Err("The XLS ExternSheet record exceeds the safety limit.".to_owned());
    }
    for reference in payload[2..].chunks_exact(6) {
        let supbook_index = usize::from(u16::from_le_bytes([reference[0], reference[1]]));
        if self_referencing_supbooks.get(supbook_index) != Some(&true) {
            return Err("XLS external workbook links are not accepted.".to_owned());
        }
    }
    Ok(())
}

fn normalized_biff_version(version: u16, substream_type: u16) -> u16 {
    match version {
        0x0200 | 0x0002 | 0x0007 | 0x0300 | 0x0400 | 0x0500 => 0x0500,
        0x0600 => 0x0600,
        0 if substream_type == 0x1000 => 0x0500,
        _ => 0x0600,
    }
}

fn validate_biff_sst<'a>(
    bytes: &'a [u8],
    payload: &'a [u8],
    mut offset: usize,
) -> Result<(), String> {
    if payload.len() < 8 {
        return Err("The XLS SST record is truncated.".to_owned());
    }
    let total = usize::try_from(u32::from_le_bytes([
        payload[0], payload[1], payload[2], payload[3],
    ]))
    .map_err(|_| "The XLS SST total count is invalid.".to_owned())?;
    let unique = usize::try_from(u32::from_le_bytes([
        payload[4], payload[5], payload[6], payload[7],
    ]))
    .map_err(|_| "The XLS SST unique count is invalid.".to_owned())?;
    if unique > total || total > MAX_RANGE_CELLS || unique > MAX_RANGE_CELLS {
        return Err("The XLS SST counts exceed the safety limit.".to_owned());
    }

    let mut segments = vec![&payload[8..]];
    let mut data_bytes = payload.len() - 8;
    let mut continuation_count = 0_usize;
    while offset < bytes.len() {
        let Some(record) = read_biff_record(bytes, offset)? else {
            break;
        };
        if record.record_type != 0x003c {
            break;
        }
        if record.payload.is_empty() {
            return Err("The XLS SST contains an empty continuation record.".to_owned());
        }
        continuation_count = continuation_count
            .checked_add(1)
            .ok_or_else(|| "The XLS SST continuation count overflowed.".to_owned())?;
        if continuation_count > MAX_BIFF_SST_CONTINUE_RECORDS {
            return Err("The XLS SST has too many continuation records.".to_owned());
        }
        data_bytes = data_bytes
            .checked_add(record.payload.len())
            .ok_or_else(|| "The XLS SST stream size overflowed.".to_owned())?;
        if data_bytes > MAX_EXPENSE_FILE_BYTES as usize {
            return Err("The XLS SST stream exceeds the safety limit.".to_owned());
        }
        segments.push(record.payload);
        offset = record.next_offset;
    }

    let mut cursor = BiffSstCursor::new(segments);
    for _ in 0..unique {
        cursor.parse_string()?;
    }
    if !cursor.is_exhausted() {
        return Err("The XLS SST contains more strings than declared.".to_owned());
    }
    Ok(())
}

#[derive(Debug)]
struct BiffSstCursor<'a> {
    segments: Vec<&'a [u8]>,
    segment_index: usize,
    offset: usize,
}

impl<'a> BiffSstCursor<'a> {
    fn new(segments: Vec<&'a [u8]>) -> Self {
        Self {
            segments,
            segment_index: 0,
            offset: 0,
        }
    }

    fn advance_empty_segments(&mut self) {
        while self
            .segments
            .get(self.segment_index)
            .is_some_and(|segment| self.offset == segment.len())
        {
            self.segment_index += 1;
            self.offset = 0;
        }
    }

    fn is_exhausted(&mut self) -> bool {
        self.advance_empty_segments();
        self.segment_index == self.segments.len()
    }

    fn read_header(&mut self, length: usize) -> Result<&'a [u8], String> {
        let segment = self
            .segments
            .get(self.segment_index)
            .ok_or_else(|| "The XLS SST string header is truncated.".to_owned())?;
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= segment.len())
            .ok_or_else(|| {
                "The XLS SST string header is split across continuation records.".to_owned()
            })?;
        let start = self.offset;
        self.offset = end;
        Ok(&segment[start..end])
    }

    fn read_u8_header(&mut self) -> Result<u8, String> {
        Ok(self.read_header(1)?[0])
    }

    fn read_u16_header(&mut self) -> Result<u16, String> {
        let bytes = self.read_header(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32_header(&mut self) -> Result<u32, String> {
        let bytes = self.read_header(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn parse_string(&mut self) -> Result<(), String> {
        self.advance_empty_segments();
        let character_count = usize::from(self.read_u16_header()?);
        if character_count > MAX_CELL_CHARS {
            return Err("The XLS SST string exceeds the character limit.".to_owned());
        }
        let flags = self.read_u8_header()?;
        let rich_run_count = if flags & 0x08 != 0 {
            usize::from(self.read_u16_header()?)
        } else {
            0
        };
        let extension_length = if flags & 0x04 != 0 {
            usize::try_from(self.read_u32_header()?)
                .map_err(|_| "The XLS SST extension length is invalid.".to_owned())?
        } else {
            0
        };

        self.read_characters(character_count, flags & 0x01 != 0)?;
        let rich_run_bytes = rich_run_count
            .checked_mul(4)
            .ok_or_else(|| "The XLS SST rich-text run size overflowed.".to_owned())?;
        self.skip_bytes(rich_run_bytes)?;
        self.skip_bytes(extension_length)
    }

    fn read_characters(
        &mut self,
        mut remaining_characters: usize,
        mut high_byte: bool,
    ) -> Result<(), String> {
        while remaining_characters > 0 {
            let segment = self
                .segments
                .get(self.segment_index)
                .ok_or_else(|| "The XLS SST string data is truncated.".to_owned())?;
            let width = if high_byte { 2 } else { 1 };
            let available_bytes = segment.len() - self.offset;
            let available_characters = available_bytes / width;
            if available_characters == 0 {
                if available_bytes != 0 {
                    return Err("The XLS SST string data is split inside a character.".to_owned());
                }
                high_byte = self.start_character_continuation()?;
                continue;
            }
            let consumed_characters = available_characters.min(remaining_characters);
            let consumed_bytes = consumed_characters
                .checked_mul(width)
                .ok_or_else(|| "The XLS SST string size overflowed.".to_owned())?;
            self.offset += consumed_bytes;
            remaining_characters -= consumed_characters;

            if remaining_characters > 0 {
                if self.offset != segment.len() {
                    return Err("The XLS SST string data is split inside a character.".to_owned());
                }
                high_byte = self.start_character_continuation()?;
            }
        }
        Ok(())
    }

    fn start_character_continuation(&mut self) -> Result<bool, String> {
        let current = self
            .segments
            .get(self.segment_index)
            .ok_or_else(|| "The XLS SST string data is truncated.".to_owned())?;
        if self.offset != current.len() {
            return Err("The XLS SST string continuation is invalid.".to_owned());
        }
        self.segment_index += 1;
        self.offset = 0;
        let option = self
            .segments
            .get(self.segment_index)
            .and_then(|segment| segment.first())
            .copied()
            .ok_or_else(|| "The XLS SST continuation option is missing.".to_owned())?;
        self.offset = 1;
        Ok(option & 0x01 != 0)
    }

    fn skip_bytes(&mut self, mut remaining: usize) -> Result<(), String> {
        while remaining > 0 {
            self.advance_empty_segments();
            let segment = self
                .segments
                .get(self.segment_index)
                .ok_or_else(|| "The XLS SST rich or extended data is truncated.".to_owned())?;
            let available = segment.len() - self.offset;
            let consumed = available.min(remaining);
            self.offset += consumed;
            remaining -= consumed;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct BiffRecord<'a> {
    record_type: u16,
    payload: &'a [u8],
    next_offset: usize,
}

fn read_biff_record(bytes: &[u8], offset: usize) -> Result<Option<BiffRecord<'_>>, String> {
    let remaining = bytes
        .get(offset..)
        .ok_or_else(|| "The XLS BIFF record offset is invalid.".to_owned())?;
    if remaining.len() < 4 {
        if remaining.iter().all(|byte| *byte == 0) {
            return Ok(None);
        }
        return Err("The XLS BIFF record header is truncated.".to_owned());
    }
    let record_type = u16::from_le_bytes([remaining[0], remaining[1]]);
    let record_len = usize::from(u16::from_le_bytes([remaining[2], remaining[3]]));
    if record_type == 0 && record_len == 0 {
        if remaining.iter().all(|byte| *byte == 0) {
            return Ok(None);
        }
        return Err("The XLS BIFF stream contains invalid padding.".to_owned());
    }
    let payload_start = offset
        .checked_add(4)
        .ok_or_else(|| "The XLS BIFF record offset overflowed.".to_owned())?;
    let payload_end = payload_start
        .checked_add(record_len)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "The XLS BIFF record is truncated.".to_owned())?;
    Ok(Some(BiffRecord {
        record_type,
        payload: &bytes[payload_start..payload_end],
        next_offset: payload_end,
    }))
}

#[derive(Debug, Default, Clone, Copy)]
struct BiffSheetStats {
    bounds: SheetCellBounds,
    formula_bounds: SheetCellBounds,
    raw_cells: usize,
    value_cells: usize,
    formula_cells: usize,
    peak_reserved_cells: usize,
}

impl BiffSheetStats {
    fn observe_cell(&mut self, row: u16, column: u16) -> Result<(), String> {
        self.raw_cells = self
            .raw_cells
            .checked_add(1)
            .ok_or_else(|| "The XLS worksheet cell count overflowed.".to_owned())?;
        if self.raw_cells > MAX_RANGE_CELLS {
            return Err("The XLS worksheet cell count exceeds the safety limit.".to_owned());
        }
        self.bounds.observe(u32::from(row), u32::from(column))?;
        self.value_cells = self
            .value_cells
            .checked_add(1)
            .ok_or_else(|| "The XLS worksheet value allocation overflowed.".to_owned())?;
        if self.value_cells > MAX_RANGE_CELLS {
            return Err("The XLS worksheet value allocation exceeds the safety limit.".to_owned());
        }
        Ok(())
    }

    fn observe_formula(&mut self, row: u16, column: u16) -> Result<(), String> {
        self.observe_cell(row, column)?;
        self.formula_cells = self
            .formula_cells
            .checked_add(1)
            .ok_or_else(|| "The XLS worksheet formula allocation overflowed.".to_owned())?;
        if self.formula_cells > MAX_RANGE_CELLS {
            return Err(
                "The XLS worksheet formula allocation exceeds the safety limit.".to_owned(),
            );
        }
        self.formula_bounds
            .observe(u32::from(row), u32::from(column))?;
        Ok(())
    }

    fn observe_cell_span(
        &mut self,
        row: u16,
        first_column: u16,
        last_column: u16,
    ) -> Result<(), String> {
        if last_column < first_column {
            return Err("The XLS MulRK cell span is invalid.".to_owned());
        }
        let span = usize::from(last_column - first_column) + 1;
        self.raw_cells = self
            .raw_cells
            .checked_add(span)
            .ok_or_else(|| "The XLS worksheet cell count overflowed.".to_owned())?;
        if self.raw_cells > MAX_RANGE_CELLS {
            return Err("The XLS worksheet cell count exceeds the safety limit.".to_owned());
        }
        self.value_cells = self
            .value_cells
            .checked_add(span)
            .ok_or_else(|| "The XLS worksheet value allocation overflowed.".to_owned())?;
        if self.value_cells > MAX_RANGE_CELLS {
            return Err("The XLS worksheet value allocation exceeds the safety limit.".to_owned());
        }
        self.bounds
            .observe(u32::from(row), u32::from(first_column))?;
        self.bounds.observe(u32::from(row), u32::from(last_column))
    }
}

fn validate_biff_sheet(bytes: &[u8], sheet_offset: usize) -> Result<BiffSheetStats, String> {
    let first = read_biff_record(bytes, sheet_offset)?
        .ok_or_else(|| "The XLS worksheet offset does not reference a BOF record.".to_owned())?;
    if !matches!(first.record_type, 0x0009 | 0x0209 | 0x0409 | 0x0809) || first.payload.len() < 4 {
        return Err("The XLS worksheet offset does not reference a valid BOF record.".to_owned());
    }
    let substream_type = u16::from_le_bytes([first.payload[2], first.payload[3]]);
    if !matches!(substream_type, 0x0010 | 0x0020) {
        return Err("The XLS BoundSheet offset references an unsupported substream.".to_owned());
    }
    let biff_version = normalized_biff_version(
        u16::from_le_bytes([first.payload[0], first.payload[1]]),
        substream_type,
    );

    let mut stats = BiffSheetStats::default();
    let mut offset = first.next_offset;
    let mut saw_eof = false;
    let mut merge_count = 0_usize;
    let mut formula_string_pending = false;
    while offset < bytes.len() {
        let record = read_biff_record(bytes, offset)?
            .ok_or_else(|| "The XLS worksheet substream ended before EOF.".to_owned())?;
        let payload = record.payload;
        if formula_string_pending && !matches!(record.record_type, 0x0207 | 0x003c) {
            return Err("The XLS formula string result is missing.".to_owned());
        }
        match record.record_type {
            0x000a => {
                saw_eof = true;
                break;
            }
            0x0200 => validate_biff_dimensions(payload, &mut stats)?,
            0x0006 => {
                validate_biff_scalar_cell(payload, 20, true, &mut stats)?;
                formula_string_pending =
                    payload[6] == 0x00 && payload[12] == 0xff && payload[13] == 0xff;
            }
            0x0203 => validate_biff_scalar_cell(payload, 14, false, &mut stats)?,
            0x0204 | 0x00d6 => {
                validate_biff_scalar_cell(payload, 8, false, &mut stats)?;
                validate_biff_cell_string(&payload[6..], biff_version)?;
            }
            0x0205 => validate_biff_scalar_cell(payload, 8, false, &mut stats)?,
            0x027e | 0x00fd => validate_biff_scalar_cell(payload, 10, false, &mut stats)?,
            0x0207 => {
                if !formula_string_pending {
                    return Err(
                        "The XLS formula string record has no preceding formula.".to_owned()
                    );
                }
                validate_biff_formula_string(payload, biff_version)?;
                formula_string_pending = false;
            }
            0x00bd => validate_biff_mul_rk(payload, &mut stats)?,
            0x00e5 => {
                if payload.len() < 2 || (payload.len() - 2) % 8 != 0 {
                    return Err("The XLS merged-cell record is invalid.".to_owned());
                }
                let declared = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
                if declared != (payload.len() - 2) / 8 {
                    return Err("The XLS merged-cell count is inconsistent.".to_owned());
                }
                for dimensions in payload[2..].chunks_exact(8) {
                    let first_row = u16::from_le_bytes([dimensions[0], dimensions[1]]);
                    let last_row = u16::from_le_bytes([dimensions[2], dimensions[3]]);
                    let first_column = u16::from_le_bytes([dimensions[4], dimensions[5]]);
                    let last_column = u16::from_le_bytes([dimensions[6], dimensions[7]]);
                    if last_row < first_row || last_column < first_column {
                        return Err("The XLS merged-cell range is invalid.".to_owned());
                    }
                    validate_sheet_position(u32::from(first_row), u32::from(first_column))?;
                    validate_sheet_position(u32::from(last_row), u32::from(last_column))?;
                }
                merge_count = merge_count
                    .checked_add(declared)
                    .ok_or_else(|| "The XLS merged-cell count overflowed.".to_owned())?;
                if merge_count > MAX_RANGE_CELLS {
                    return Err("The XLS merged-cell count exceeds the safety limit.".to_owned());
                }
            }
            _ => {}
        }
        offset = record.next_offset;
    }
    if !saw_eof {
        return Err("The XLS worksheet substream has no EOF record.".to_owned());
    }
    if stats.bounds.area()? > MAX_RANGE_CELLS || stats.formula_bounds.area()? > MAX_RANGE_CELLS {
        return Err("The XLS worksheet cell span exceeds the safety limit.".to_owned());
    }
    Ok(stats)
}

fn validate_biff_scalar_cell(
    payload: &[u8],
    minimum_length: usize,
    formula: bool,
    stats: &mut BiffSheetStats,
) -> Result<(), String> {
    if payload.len() < minimum_length {
        return Err("The XLS worksheet cell record is truncated.".to_owned());
    }
    let row = u16::from_le_bytes([payload[0], payload[1]]);
    let column = u16::from_le_bytes([payload[2], payload[3]]);
    if formula {
        stats.observe_formula(row, column)
    } else {
        stats.observe_cell(row, column)
    }
}

fn validate_biff_formula_string(payload: &[u8], biff_version: u16) -> Result<(), String> {
    if payload == [0, 0] {
        return Ok(());
    }
    let header_length = if biff_version >= 0x0600 { 3 } else { 2 };
    if payload.len() < header_length {
        return Err("The XLS formula string record is truncated.".to_owned());
    }
    let character_count = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
    if character_count > MAX_CELL_CHARS {
        return Err("The XLS formula string exceeds the character limit.".to_owned());
    }
    let width = if biff_version >= 0x0600 && payload[2] & 0x01 != 0 {
        2
    } else {
        1
    };
    let required = character_count
        .checked_mul(width)
        .and_then(|length| length.checked_add(header_length))
        .ok_or_else(|| "The XLS formula string size overflowed.".to_owned())?;
    if payload.len() < required {
        return Err("The XLS formula string record is truncated.".to_owned());
    }
    Ok(())
}

fn validate_biff_cell_string(payload: &[u8], biff_version: u16) -> Result<(), String> {
    if biff_version >= 0x0600 {
        return validate_biff_formula_string(payload, biff_version);
    }
    if payload.len() < 2 {
        return Err("The XLS cell string record is truncated.".to_owned());
    }
    let character_count = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
    if character_count > MAX_CELL_CHARS {
        return Err("The XLS cell string exceeds the character limit.".to_owned());
    }
    let required = character_count
        .checked_add(2)
        .ok_or_else(|| "The XLS cell string size overflowed.".to_owned())?;
    if payload.len() < required {
        return Err("The XLS cell string record is truncated.".to_owned());
    }
    Ok(())
}

fn validate_biff_mul_rk(payload: &[u8], stats: &mut BiffSheetStats) -> Result<(), String> {
    if payload.len() < 12 || !(payload.len() - 6).is_multiple_of(6) {
        return Err("The XLS MulRK record is invalid.".to_owned());
    }
    let row = u16::from_le_bytes([payload[0], payload[1]]);
    let first_column = u16::from_le_bytes([payload[2], payload[3]]);
    let last_column_offset = payload.len() - 2;
    let last_column =
        u16::from_le_bytes([payload[last_column_offset], payload[last_column_offset + 1]]);
    let declared_span = usize::from(
        last_column
            .checked_sub(first_column)
            .ok_or_else(|| "The XLS MulRK cell span is invalid.".to_owned())?,
    ) + 1;
    if declared_span != (payload.len() - 6) / 6 {
        return Err("The XLS MulRK cell count is inconsistent.".to_owned());
    }
    stats.observe_cell_span(row, first_column, last_column)
}

fn validate_biff_dimensions(payload: &[u8], stats: &mut BiffSheetStats) -> Result<(), String> {
    let (first_row, last_row, mut first_column, last_column) = match payload.len() {
        10 => (
            u32::from(u16::from_le_bytes([payload[0], payload[1]])),
            u32::from(u16::from_le_bytes([payload[2], payload[3]])),
            u16::from_le_bytes([payload[4], payload[5]]),
            u16::from_le_bytes([payload[6], payload[7]]),
        ),
        14 => (
            u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]),
            u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]),
            u16::from_le_bytes([payload[8], payload[9]]),
            u16::from_le_bytes([payload[10], payload[11]]),
        ),
        _ => return Err("The XLS Dimensions record is invalid.".to_owned()),
    };
    let area = if last_row == 0 || last_column == 0 {
        1
    } else {
        if first_column > 0x00ff || last_column < first_column {
            first_column = 0;
        }
        if last_row <= first_row || last_column <= first_column {
            return Err("The XLS Dimensions record has an invalid cell span.".to_owned());
        }
        let rows = usize::try_from(last_row - first_row)
            .map_err(|_| "The XLS Dimensions row span is invalid.".to_owned())?;
        let columns = usize::from(last_column - first_column);
        rows.checked_mul(columns)
            .ok_or_else(|| "The XLS Dimensions allocation size overflowed.".to_owned())?
    };
    if area > MAX_RANGE_CELLS {
        return Err("The XLS Dimensions allocation exceeds the safety limit.".to_owned());
    }
    let peak_reserved_cells = stats
        .value_cells
        .checked_add(area)
        .ok_or_else(|| "The XLS Dimensions allocation size overflowed.".to_owned())?;
    if peak_reserved_cells > MAX_RANGE_CELLS {
        return Err("The XLS Dimensions allocation exceeds the safety limit.".to_owned());
    }
    stats.peak_reserved_cells = stats.peak_reserved_cells.max(peak_reserved_cells);
    Ok(())
}

fn utf8_xml_decoder() -> XmlDecoder {
    XmlReader::from_str("").decoder()
}

fn xml_attribute_value(
    element: &BytesStart<'_>,
    expected_local_name: &[u8],
    decoder: XmlDecoder,
) -> Result<Option<String>, String> {
    let mut value = None;
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|_| "The XLSX XML attribute is invalid.".to_owned())?;
        if attribute.key.local_name().as_ref() != expected_local_name {
            continue;
        }
        if value.is_some() {
            return Err("The XLSX XML attribute is duplicated.".to_owned());
        }
        value = Some(
            attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
                .map_err(|_| "The XLSX XML attribute value is invalid.".to_owned())?
                .into_owned(),
        );
    }
    Ok(value)
}

fn validate_shared_strings_xml(bytes: &[u8]) -> Result<(), String> {
    let mut reader = XmlReader::from_reader(DecodingReader::new(bytes));
    reader.config_mut().expand_empty_elements = true;
    let decoder = utf8_xml_decoder();
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut root_depth = None;
    let mut root_closed = false;
    let mut declared_unique = None;
    let mut actual_unique = 0_usize;

    loop {
        buffer.clear();
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| "The XLSX shared strings XML is invalid.".to_owned())?
        {
            Event::Start(element) => {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| "The XLSX XML nesting depth overflowed.".to_owned())?;
                if root_depth.is_none() {
                    if root_closed || element.local_name().as_ref() != b"sst" {
                        return Err("The XLSX shared strings root element is invalid.".to_owned());
                    }
                    root_depth = Some(depth);
                    if let Some(value) = xml_attribute_value(&element, b"uniqueCount", decoder)? {
                        let count = value.parse::<usize>().map_err(|_| {
                            "The XLSX shared strings uniqueCount is invalid.".to_owned()
                        })?;
                        if count > MAX_RANGE_CELLS {
                            return Err(
                                "The XLSX shared strings uniqueCount exceeds the safety limit."
                                    .to_owned(),
                            );
                        }
                        declared_unique = Some(count);
                    }
                } else if element.local_name().as_ref() == b"si" {
                    actual_unique = actual_unique
                        .checked_add(1)
                        .ok_or_else(|| "The XLSX shared strings count overflowed.".to_owned())?;
                    if actual_unique > MAX_RANGE_CELLS {
                        return Err(
                            "The XLSX shared strings count exceeds the safety limit.".to_owned()
                        );
                    }
                }
            }
            Event::End(element) => {
                if root_depth == Some(depth) {
                    if element.local_name().as_ref() != b"sst" {
                        return Err("The XLSX shared strings root element is invalid.".to_owned());
                    }
                    root_depth = None;
                    root_closed = true;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "The XLSX XML nesting depth is invalid.".to_owned())?;
            }
            Event::DocType(_) => {
                return Err("XLSX XML must not contain a DOCTYPE declaration.".to_owned());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || !root_closed {
        return Err("The XLSX shared strings root element is invalid.".to_owned());
    }
    if declared_unique.is_some_and(|declared| declared != actual_unique) {
        return Err("The XLSX shared strings uniqueCount is inconsistent.".to_owned());
    }
    Ok(())
}

fn validate_workbook_xml(bytes: &[u8]) -> Result<(), String> {
    let mut reader = XmlReader::from_reader(DecodingReader::new(bytes));
    reader.config_mut().expand_empty_elements = true;
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut root_depth = None;
    let mut root_closed = false;
    let mut sheet_count = 0_usize;

    loop {
        buffer.clear();
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| "The XLSX workbook XML is invalid.".to_owned())?
        {
            Event::Start(element) => {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| "The XLSX XML nesting depth overflowed.".to_owned())?;
                if root_depth.is_none() {
                    if root_closed || element.local_name().as_ref() != b"workbook" {
                        return Err("The XLSX workbook root element is invalid.".to_owned());
                    }
                    root_depth = Some(depth);
                } else if element.local_name().as_ref() == b"sheet" {
                    sheet_count = sheet_count
                        .checked_add(1)
                        .ok_or_else(|| "The XLSX worksheet count overflowed.".to_owned())?;
                    if sheet_count > MAX_SHEETS {
                        return Err(
                            "The XLSX workbook sheet count exceeds the safety limit.".to_owned()
                        );
                    }
                }
            }
            Event::End(element) => {
                if root_depth == Some(depth) {
                    if element.local_name().as_ref() != b"workbook" {
                        return Err("The XLSX workbook root element is invalid.".to_owned());
                    }
                    root_depth = None;
                    root_closed = true;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "The XLSX XML nesting depth is invalid.".to_owned())?;
            }
            Event::DocType(_) => {
                return Err("XLSX XML must not contain a DOCTYPE declaration.".to_owned());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || !root_closed || sheet_count == 0 {
        return Err("The XLSX workbook contains no valid worksheet metadata.".to_owned());
    }
    Ok(())
}

fn validate_styles_xml(bytes: &[u8]) -> Result<(), String> {
    let mut reader = XmlReader::from_reader(DecodingReader::new(bytes));
    reader.config_mut().expand_empty_elements = true;
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut root_depth = None;
    let mut root_closed = false;
    let mut num_fmts_depth = None;
    let mut cell_xfs_depth = None;
    let mut num_fmt_count = 0_usize;
    let mut cell_xf_count = 0_usize;

    loop {
        buffer.clear();
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| "The XLSX styles XML is invalid.".to_owned())?
        {
            Event::Start(element) => {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| "The XLSX XML nesting depth overflowed.".to_owned())?;
                let local_name = element.local_name();
                if root_depth.is_none() {
                    if root_closed || local_name.as_ref() != b"styleSheet" {
                        return Err("The XLSX styles root element is invalid.".to_owned());
                    }
                    root_depth = Some(depth);
                } else if local_name.as_ref() == b"numFmts" {
                    if num_fmts_depth.is_some() {
                        return Err("The XLSX number format scope is invalid.".to_owned());
                    }
                    num_fmts_depth = Some(depth);
                } else if local_name.as_ref() == b"cellXfs" {
                    if cell_xfs_depth.is_some() {
                        return Err("The XLSX cell format scope is invalid.".to_owned());
                    }
                    cell_xfs_depth = Some(depth);
                } else if local_name.as_ref() == b"numFmt" && num_fmts_depth.is_some() {
                    num_fmt_count = num_fmt_count
                        .checked_add(1)
                        .ok_or_else(|| "The XLSX number format count overflowed.".to_owned())?;
                    if num_fmt_count > MAX_STYLE_NUMFMTS {
                        return Err(
                            "The XLSX number format count exceeds the safety limit.".to_owned()
                        );
                    }
                } else if local_name.as_ref() == b"xf" && cell_xfs_depth.is_some() {
                    cell_xf_count = cell_xf_count
                        .checked_add(1)
                        .ok_or_else(|| "The XLSX cell format count overflowed.".to_owned())?;
                    if cell_xf_count > MAX_STYLE_CELL_XFS {
                        return Err(
                            "The XLSX cell format count exceeds the safety limit.".to_owned()
                        );
                    }
                }
            }
            Event::End(element) => {
                if num_fmts_depth == Some(depth) {
                    if element.local_name().as_ref() != b"numFmts" {
                        return Err("The XLSX number format scope is invalid.".to_owned());
                    }
                    num_fmts_depth = None;
                }
                if cell_xfs_depth == Some(depth) {
                    if element.local_name().as_ref() != b"cellXfs" {
                        return Err("The XLSX cell format scope is invalid.".to_owned());
                    }
                    cell_xfs_depth = None;
                }
                if root_depth == Some(depth) {
                    if element.local_name().as_ref() != b"styleSheet" {
                        return Err("The XLSX styles root element is invalid.".to_owned());
                    }
                    root_depth = None;
                    root_closed = true;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "The XLSX XML nesting depth is invalid.".to_owned())?;
            }
            Event::DocType(_) => {
                return Err("XLSX XML must not contain a DOCTYPE declaration.".to_owned());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || !root_closed || num_fmts_depth.is_some() || cell_xfs_depth.is_some() {
        return Err("The XLSX styles XML structure is invalid.".to_owned());
    }
    Ok(())
}

fn validate_relationships_xml(bytes: &[u8]) -> Result<(), String> {
    let mut reader = XmlReader::from_reader(DecodingReader::new(bytes));
    reader.config_mut().expand_empty_elements = true;
    let decoder = utf8_xml_decoder();
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut root_depth = None;
    let mut root_closed = false;

    loop {
        buffer.clear();
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| "The XLSX relationships XML is invalid.".to_owned())?
        {
            Event::Start(element) => {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| "The XLSX XML nesting depth overflowed.".to_owned())?;
                if root_depth.is_none() {
                    if root_closed || element.local_name().as_ref() != b"Relationships" {
                        return Err("The XLSX relationships root element is invalid.".to_owned());
                    }
                    root_depth = Some(depth);
                } else if element.local_name().as_ref() == b"Relationship"
                    && xml_attribute_value(&element, b"TargetMode", decoder)?
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case("external"))
                {
                    return Err("XLSX files containing external links are not accepted.".to_owned());
                }
            }
            Event::End(element) => {
                if root_depth == Some(depth) {
                    if element.local_name().as_ref() != b"Relationships" {
                        return Err("The XLSX relationships root element is invalid.".to_owned());
                    }
                    root_depth = None;
                    root_closed = true;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "The XLSX XML nesting depth is invalid.".to_owned())?;
            }
            Event::DocType(_) => {
                return Err("XLSX XML must not contain a DOCTYPE declaration.".to_owned());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || !root_closed {
        return Err("The XLSX relationships XML structure is invalid.".to_owned());
    }
    Ok(())
}

fn validate_ooxml_container(bytes: &[u8]) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "XLSX ZIP 구조가 올바르지 않습니다.".to_owned())?;
    if archive.is_empty() || archive.len() > MAX_ZIP_ENTRIES {
        return Err("XLSX 내부 파일 수가 안전 제한을 초과했습니다.".to_owned());
    }

    let mut total_uncompressed = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| "XLSX 내부 파일을 검사할 수 없습니다.".to_owned())?;
        let normalized_name = entry.name().replace('\\', "/").to_ascii_lowercase();
        if normalized_name.starts_with('/') || normalized_name.split('/').any(|part| part == "..") {
            return Err("XLSX 내부 경로가 안전하지 않습니다.".to_owned());
        }
        if normalized_name.ends_with("vbaproject.bin")
            || normalized_name.starts_with("xl/embeddings/")
            || normalized_name.starts_with("xl/activex/")
            || normalized_name.starts_with("xl/externallinks/")
        {
            return Err("매크로·OLE·ActiveX가 포함된 XLSX는 가져올 수 없습니다.".to_owned());
        }

        let uncompressed = entry.size();
        let compressed = entry.compressed_size();
        if uncompressed > MAX_ZIP_ENTRY_BYTES {
            return Err("XLSX 내부 파일 크기가 안전 제한을 초과했습니다.".to_owned());
        }
        total_uncompressed = total_uncompressed
            .checked_add(uncompressed)
            .ok_or_else(|| "XLSX 압축 해제 크기를 계산할 수 없습니다.".to_owned())?;
        if total_uncompressed > MAX_ZIP_TOTAL_BYTES {
            return Err("XLSX 전체 압축 해제 크기가 안전 제한을 초과했습니다.".to_owned());
        }
        if compressed == 0 {
            if uncompressed > 0 {
                return Err("XLSX 압축 비율이 안전 제한을 초과했습니다.".to_owned());
            }
        } else if uncompressed > compressed.saturating_mul(MAX_ZIP_COMPRESSION_RATIO) {
            return Err("XLSX 압축 비율이 안전 제한을 초과했습니다.".to_owned());
        }

        if normalized_name.ends_with("/workbook.xml") || normalized_name == "workbook.xml" {
            if uncompressed > MAX_WORKBOOK_METADATA_BYTES {
                return Err("The XLSX workbook metadata exceeds the safety limit.".to_owned());
            }
            let capacity = usize::try_from(uncompressed)
                .map_err(|_| "The XLSX workbook metadata size is invalid.".to_owned())?;
            let mut workbook_metadata = Vec::with_capacity(capacity);
            entry
                .read_to_end(&mut workbook_metadata)
                .map_err(|_| "The XLSX workbook metadata cannot be inspected.".to_owned())?;
            validate_workbook_xml(&workbook_metadata)?;
        }

        if normalized_name.ends_with("/sharedstrings.xml") || normalized_name == "sharedstrings.xml"
        {
            let capacity = usize::try_from(uncompressed)
                .map_err(|_| "The XLSX shared strings size is invalid.".to_owned())?;
            let mut shared_strings = Vec::with_capacity(capacity);
            entry
                .read_to_end(&mut shared_strings)
                .map_err(|_| "The XLSX shared strings cannot be inspected.".to_owned())?;
            validate_shared_strings_xml(&shared_strings)?;
        }

        if normalized_name.ends_with("/styles.xml") || normalized_name == "styles.xml" {
            if uncompressed > MAX_STYLES_METADATA_BYTES {
                return Err("The XLSX styles metadata exceeds the safety limit.".to_owned());
            }
            let capacity = usize::try_from(uncompressed)
                .map_err(|_| "The XLSX styles metadata size is invalid.".to_owned())?;
            let mut styles = Vec::with_capacity(capacity);
            entry
                .read_to_end(&mut styles)
                .map_err(|_| "The XLSX styles metadata cannot be inspected.".to_owned())?;
            validate_styles_xml(&styles)?;
        }

        if normalized_name.ends_with(".rels") {
            if uncompressed > MAX_RELATIONSHIP_BYTES {
                return Err("XLSX 관계 파일 크기가 안전 제한을 초과했습니다.".to_owned());
            }
            let mut relationship = Vec::with_capacity(uncompressed as usize);
            entry
                .read_to_end(&mut relationship)
                .map_err(|_| "XLSX 관계 파일을 검사할 수 없습니다.".to_owned())?;
            validate_relationships_xml(&relationship)?;
        }
    }
    Ok(())
}

fn detect_adapter(range: &Range<Data>) -> Option<ExpenseAdapter> {
    let sample = range
        .rows()
        .take(15)
        .flat_map(|row| row.iter())
        .map(cell_text)
        .collect::<Vec<_>>()
        .join("|");
    if sample.contains("거래구분")
        && sample.contains("거래 후 잔액")
        && (sample.contains("결제 정보") || sample.contains("계좌 정보"))
    {
        Some(ExpenseAdapter::KakaoPayMoneyV1)
    } else if sample.contains("거래일시")
        && sample.contains("출금액")
        && sample.contains("입금액")
        && sample.contains("거래점")
    {
        Some(ExpenseAdapter::KbAccountHistoryV1)
    } else if sample.contains("이용일자")
        && sample.contains("가맹점")
        && sample.contains("이용금액")
    {
        Some(ExpenseAdapter::KbCardUsageV1)
    } else {
        None
    }
}

fn declared_coverage_period(range: &Range<Data>) -> Option<(NaiveDate, NaiveDate)> {
    const PERIOD_MARKERS: &[&str] = &["조회기간", "이용기간", "거래기간", "사용기간"];
    for row in range.rows().take(30) {
        let joined = row.iter().map(cell_text).collect::<Vec<_>>().join(" ");
        if !PERIOD_MARKERS.iter().any(|marker| joined.contains(marker)) {
            continue;
        }
        let mut dates = joined
            .split(|character: char| {
                !character.is_ascii_digit() && !matches!(character, '-' | '.' | '/')
            })
            .filter_map(|candidate| {
                let value = parse_datetime(&Data::String(candidate.to_owned()))?;
                NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d").ok()
            });
        let start = dates.next()?;
        let end = dates.next()?;
        if start <= end {
            return Some((start, end));
        }
    }
    None
}

fn parse_kakao_pay(range: &Range<Data>) -> Result<(Vec<ParsedExpenseRow>, usize), String> {
    let header = find_header_row(range, &["거래일시", "거래구분", "거래금액"])?;
    let mut rows = Vec::new();
    let mut rejected = 0;
    for (offset, row) in range.rows().enumerate().skip(header + 1) {
        if row.iter().all(is_empty) {
            continue;
        }
        let Some(occurred_at) = row.first().and_then(parse_datetime) else {
            if !is_statement_footer(row) {
                rejected += 1;
            }
            continue;
        };
        let transaction_type = row.get(1).map(cell_text).unwrap_or_default();
        let Some(signed_amount) = row.get(2).and_then(parse_amount) else {
            rejected += 1;
            continue;
        };
        let Some(amount_minor) = absolute_nonzero_amount(signed_amount) else {
            rejected += 1;
            continue;
        };
        let display = bounded_optional(row.get(5).map(cell_text));
        let (kind, category, direction, merchant, counterparty, needs_review, excluded) =
            if transaction_type.contains("결제취소") {
                ("refund", "income_refund", "in", display, None, false, false)
            } else if transaction_type.contains("결제") {
                ("purchase", "unresolved", "out", display, None, false, false)
            } else if transaction_type.contains("부족분충전") {
                (
                    "wallet_topup",
                    "transfer_settlement",
                    "in",
                    None,
                    display,
                    false,
                    true,
                )
            } else if transaction_type.contains("내계좌로_내보내기") {
                (
                    "wallet_withdrawal",
                    "transfer_settlement",
                    "out",
                    None,
                    display,
                    false,
                    true,
                )
            } else if transaction_type.contains("송금취소") || transaction_type.contains("기간만료")
            {
                (
                    "transfer_reversal",
                    "transfer_settlement",
                    "in",
                    None,
                    display,
                    false,
                    true,
                )
            } else if transaction_type.contains("받기") {
                ("p2p_in", "unresolved", "in", None, display, true, false)
            } else if transaction_type.contains("송금") {
                ("p2p_out", "unresolved", "out", None, display, true, false)
            } else {
                (
                    "unknown",
                    "unresolved",
                    if signed_amount < 0 { "out" } else { "in" },
                    None,
                    display,
                    true,
                    false,
                )
            };
        rows.push(parsed_row(
            ExpenseAdapter::KakaoPayMoneyV1,
            offset + 1,
            occurred_at,
            kind,
            category,
            direction,
            amount_minor,
            merchant,
            counterparty,
            Some(transaction_type),
            None,
            needs_review,
            excluded,
        ));
    }
    Ok((rows, rejected))
}

fn parse_kb_account(range: &Range<Data>) -> Result<(Vec<ParsedExpenseRow>, usize), String> {
    let header = find_header_row(range, &["거래일시", "출금액", "입금액"])?;
    let mut rows = Vec::new();
    let mut rejected = 0;
    for (offset, row) in range.rows().enumerate().skip(header + 1) {
        if row.iter().all(is_empty) {
            continue;
        }
        let Some(occurred_at) = row.first().and_then(parse_datetime) else {
            if !is_statement_footer(row) {
                rejected += 1;
            }
            continue;
        };
        let withdrawal = row
            .get(4)
            .and_then(parse_amount)
            .and_then(absolute_nonzero_amount)
            .unwrap_or_default();
        let deposit = row
            .get(5)
            .and_then(parse_amount)
            .and_then(absolute_nonzero_amount)
            .unwrap_or_default();
        if (withdrawal == 0) == (deposit == 0) {
            rejected += 1;
            continue;
        }
        let summary = row.get(1).map(cell_text).unwrap_or_default();
        let display = bounded_optional(row.get(2).map(cell_text));
        let memo = bounded_optional(row.get(3).map(cell_text));
        let transaction_code = row.get(8).map(cell_text).unwrap_or_default();
        let combined = format!("{summary} {transaction_code}");
        let (kind, category, direction, needs_review, excluded) = if withdrawal > 0
            && (combined.contains("국민카드") || combined.contains("카드대금"))
        {
            ("card_settlement", "transfer_settlement", "out", false, true)
        } else if withdrawal > 0 && transaction_code.contains("체크카드") {
            ("purchase", "unresolved", "out", false, false)
        } else if withdrawal > 0 {
            ("bank_out", "unresolved", "out", true, false)
        } else {
            ("bank_in", "unresolved", "in", true, false)
        };
        rows.push(parsed_row(
            ExpenseAdapter::KbAccountHistoryV1,
            offset + 1,
            occurred_at,
            kind,
            category,
            direction,
            withdrawal.max(deposit),
            (kind == "purchase").then(|| display.clone()).flatten(),
            (kind != "purchase").then_some(display).flatten(),
            memo.or_else(|| bounded_optional(Some(summary))),
            None,
            needs_review,
            excluded,
        ));
    }
    Ok((rows, rejected))
}

fn parse_kb_card(range: &Range<Data>) -> Result<(Vec<ParsedExpenseRow>, usize), String> {
    let layout = find_kb_card_layout(range)?;
    let mut rows = Vec::new();
    let mut rejected = 0;
    for (offset, row) in range.rows().enumerate().skip(layout.last_header_row + 1) {
        if row.iter().all(is_empty) {
            continue;
        }
        let Some(occurred_at) = row.get(layout.date).and_then(parse_datetime) else {
            if !is_statement_footer(row) {
                rejected += 1;
            }
            continue;
        };
        let Some(signed_amount) = row.get(layout.amount).and_then(parse_amount) else {
            rejected += 1;
            continue;
        };
        let Some(amount_minor) = absolute_nonzero_amount(signed_amount) else {
            rejected += 1;
            continue;
        };
        let card_fingerprint_source = layout
            .card
            .and_then(|column| row.get(column))
            .map(cell_text)
            .filter(|value| !value.trim().is_empty());
        let usage_type = bounded_optional(
            layout
                .usage_type
                .and_then(|column| row.get(column))
                .map(cell_text),
        );
        let merchant = bounded_optional(row.get(layout.merchant).map(cell_text));
        let payment_method_fingerprint = card_fingerprint_source
            .as_deref()
            .map(|value| hex_sha256(format!("tm-expense:payment-method:v1:{value}").as_bytes()));
        let is_refund = signed_amount < 0
            || usage_type
                .as_deref()
                .is_some_and(|value| value.contains("취소") || value.contains("환불"));
        rows.push(parsed_row(
            ExpenseAdapter::KbCardUsageV1,
            offset + 1,
            occurred_at,
            if is_refund { "refund" } else { "purchase" },
            if is_refund {
                "income_refund"
            } else {
                "unresolved"
            },
            if is_refund { "in" } else { "out" },
            amount_minor,
            merchant,
            None,
            join_note(None, usage_type),
            payment_method_fingerprint,
            false,
            false,
        ));
    }
    Ok((rows, rejected))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KbCardLayout {
    last_header_row: usize,
    date: usize,
    card: Option<usize>,
    usage_type: Option<usize>,
    merchant: usize,
    amount: usize,
}

fn find_kb_card_layout(range: &Range<Data>) -> Result<KbCardLayout, String> {
    let rows = range.rows().take(20).collect::<Vec<_>>();
    for first_row in 0..rows.len() {
        for depth in [2_usize, 1_usize] {
            let Some(header_rows) = rows.get(first_row..first_row.saturating_add(depth)) else {
                continue;
            };
            if depth == 2 && !is_kb_card_leaf_header_row(header_rows[1]) {
                continue;
            }
            let Some(date) = find_preferred_header_column(header_rows, &["이용일자"]) else {
                continue;
            };
            let Some(merchant) = find_preferred_header_column(header_rows, &["가맹점"]) else {
                continue;
            };
            let Some(amount) =
                find_preferred_header_column(header_rows, &["이용금액", "이용 금액", "금액"])
            else {
                continue;
            };
            if date == merchant || date == amount || merchant == amount {
                continue;
            }
            let last_header_row = first_row + depth - 1;
            if !range.rows().skip(last_header_row + 1).any(|row| {
                row.get(date).and_then(parse_datetime).is_some()
                    && row
                        .get(amount)
                        .and_then(parse_amount)
                        .and_then(absolute_nonzero_amount)
                        .is_some()
            }) {
                continue;
            }
            return Ok(KbCardLayout {
                last_header_row,
                date,
                card: find_preferred_header_column(
                    header_rows,
                    &["카드번호", "카드 번호", "이용카드"],
                )
                .filter(|column| ![date, merchant, amount].contains(column)),
                usage_type: find_preferred_header_column(header_rows, &["이용구분", "이용 구분"])
                    .filter(|column| ![date, merchant, amount].contains(column)),
                merchant,
                amount,
            });
        }
    }
    Err("카드 이용내역의 헤더를 찾지 못했습니다.".to_owned())
}

fn is_kb_card_leaf_header_row(row: &[Data]) -> bool {
    const LEAF_LABELS: &[&str] = &[
        "이용일자",
        "가맹점",
        "이용금액",
        "이용 금액",
        "금액",
        "카드번호",
        "카드 번호",
        "이용카드",
        "이용구분",
        "이용 구분",
        "승인번호",
        "승인 번호",
    ];
    row.iter()
        .filter(|cell| {
            let value = cell_text(cell);
            LEAF_LABELS.contains(&value.trim())
        })
        .take(2)
        .count()
        >= 2
}

fn find_preferred_header_column(rows: &[&[Data]], aliases: &[&str]) -> Option<usize> {
    for row in rows.iter().rev() {
        for alias in aliases {
            if let Some(column) = row.iter().position(|cell| cell_text(cell).trim() == *alias) {
                return Some(column);
            }
        }
        for alias in aliases {
            if let Some(column) = row.iter().position(|cell| cell_text(cell).contains(alias)) {
                return Some(column);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn parsed_row(
    adapter: ExpenseAdapter,
    row_number: usize,
    occurred_at: String,
    kind: &str,
    category: &str,
    direction: &str,
    amount_minor: i64,
    merchant: Option<String>,
    counterparty: Option<String>,
    note: Option<String>,
    payment_method_fingerprint: Option<String>,
    needs_review: bool,
    excluded: bool,
) -> ParsedExpenseRow {
    let fingerprint = [
        adapter.as_str(),
        &occurred_at,
        kind,
        direction,
        &amount_minor.to_string(),
        merchant.as_deref().unwrap_or_default(),
        counterparty.as_deref().unwrap_or_default(),
        note.as_deref().unwrap_or_default(),
        payment_method_fingerprint.as_deref().unwrap_or_default(),
    ]
    .join("\u{1f}");
    ParsedExpenseRow {
        row_number,
        occurred_at,
        kind: kind.to_owned(),
        category: category.to_owned(),
        direction: direction.to_owned(),
        amount_minor,
        currency: "KRW".to_owned(),
        merchant,
        counterparty,
        note,
        payment_method_fingerprint,
        row_fingerprint: hex_sha256(fingerprint.as_bytes()),
        needs_review,
        excluded,
    }
}

fn find_header_row(range: &Range<Data>, required: &[&str]) -> Result<usize, String> {
    range
        .rows()
        .take(20)
        .position(|row| {
            let joined = row.iter().map(cell_text).collect::<Vec<_>>().join("|");
            required.iter().all(|value| joined.contains(value))
        })
        .ok_or_else(|| "지출 파일의 헤더를 찾지 못했습니다.".to_owned())
}

fn parse_datetime(cell: &Data) -> Option<String> {
    if let Some(value) = cell.as_datetime() {
        return Some(value.format("%Y-%m-%dT%H:%M:%S").to_string());
    }
    let value = cell_text(cell);
    const DATETIME_FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S",
        "%Y.%m.%d %H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y.%m.%d %H:%M",
        "%Y/%m/%d %H:%M",
    ];
    for format in DATETIME_FORMATS {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value.trim(), format) {
            return Some(parsed.format("%Y-%m-%dT%H:%M:%S").to_string());
        }
    }
    const DATE_FORMATS: &[&str] = &["%Y-%m-%d", "%Y.%m.%d", "%Y/%m/%d", "%Y%m%d"];
    for format in DATE_FORMATS {
        if let Ok(parsed) = NaiveDate::parse_from_str(value.trim(), format) {
            return Some(format!("{parsed}T00:00:00"));
        }
    }
    None
}

fn parse_amount(cell: &Data) -> Option<i64> {
    let parsed = match cell {
        Data::Int(value) => *value,
        Data::Float(value) => {
            if !value.is_finite()
                || value.fract() != 0.0
                || value.abs() > MAX_SAFE_AMOUNT_MINOR as f64
            {
                return None;
            }
            *value as i64
        }
        Data::String(value) => {
            let trimmed = value.trim();
            let negative_parentheses = trimmed.starts_with('(') && trimmed.ends_with(')');
            if trimmed.starts_with('(') != trimmed.ends_with(')') {
                return None;
            }
            let normalized = trimmed
                .trim_matches(['(', ')'])
                .chars()
                .filter(|character| !matches!(character, ',' | '₩' | '원' | ' '))
                .collect::<String>();
            let parsed = normalized.parse::<i64>().ok()?;
            if negative_parentheses {
                parsed.checked_abs()?.checked_neg()?
            } else {
                parsed
            }
        }
        Data::Empty
        | Data::Bool(_)
        | Data::DateTime(_)
        | Data::DateTimeIso(_)
        | Data::DurationIso(_)
        | Data::Error(_) => return None,
    };
    (parsed.unsigned_abs() <= MAX_SAFE_AMOUNT_MINOR.unsigned_abs()).then_some(parsed)
}

fn absolute_nonzero_amount(value: i64) -> Option<i64> {
    value
        .checked_abs()
        .filter(|amount| *amount > 0 && *amount <= MAX_SAFE_AMOUNT_MINOR)
}

fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) | Data::DateTimeIso(value) | Data::DurationIso(value) => {
            value.trim().to_owned()
        }
        _ => cell.to_string().trim().to_owned(),
    }
}

fn is_empty(cell: &Data) -> bool {
    matches!(cell, Data::Empty) || cell_text(cell).is_empty()
}

fn is_statement_footer(row: &[Data]) -> bool {
    let label = row
        .iter()
        .take(8)
        .map(cell_text)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    [
        "합계",
        "총계",
        "조회기간",
        "출력일시",
        "안내사항",
        "유의사항",
        "※",
    ]
    .iter()
    .any(|marker| label.contains(marker))
}

fn bounded_optional(value: Option<String>) -> Option<String> {
    value
        .map(|item| redact_financial_identifiers(&item))
        .map(|item| item.trim().chars().take(500).collect::<String>())
        .filter(|item| !item.is_empty())
}

fn redact_financial_identifiers(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < characters.len() {
        if characters[index].is_ascii_digit() || characters[index] == '*' {
            let start = index;
            let mut digits = 0_usize;
            let mut masks = 0_usize;
            while index < characters.len()
                && (characters[index].is_ascii_digit()
                    || matches!(characters[index], '-' | ' ' | '*' | '•'))
            {
                digits += usize::from(characters[index].is_ascii_digit());
                masks += usize::from(characters[index] == '*');
                index += 1;
            }
            let token_length = index.saturating_sub(start);
            if digits >= 8 || (masks > 0 && digits >= 4 && token_length >= 8) {
                output.push_str("[금융식별번호 제거]");
            } else {
                output.extend(characters[start..index].iter());
            }
        } else {
            output.push(characters[index]);
            index += 1;
        }
    }
    output
}

fn join_note(left: Option<String>, right: Option<String>) -> Option<String> {
    let value = [left, right]
        .into_iter()
        .flatten()
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    (!value.is_empty()).then_some(value)
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use calamine::{
        Cell, Data, ExcelDateTime, ExcelDateTimeType, Range, Reader, Sheets,
        open_workbook_auto_from_rs,
    };
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::{
        BiffSheetStats, CFB_END_OF_CHAIN, CFB_FAT_SECTOR, CFB_FREE_SECTOR, ExpenseAdapter,
        MAX_BIFF_SST_CONTINUE_RECORDS, MAX_CELL_CHARS, MAX_CFB_ENTRIES, MAX_COLUMNS,
        MAX_RANGE_CELLS, MAX_ROWS, MAX_SAFE_AMOUNT_MINOR, MAX_STYLE_CELL_XFS, MAX_STYLE_NUMFMTS,
        MAX_STYLES_METADATA_BYTES, absolute_nonzero_amount, declared_coverage_period,
        detect_adapter, find_kb_card_layout, parse_amount, parse_datetime, parse_kakao_pay,
        parse_kb_account, parse_kb_card, redact_financial_identifiers,
        source_discriminator_fingerprint, validate_biff_dimensions, validate_biff_sheet,
        validate_biff_sst, validate_biff_workbook_stream, validate_ooxml_container, validate_range,
        validate_relationships_xml, validate_shared_strings_xml, validate_styles_xml,
        validate_workbook_xml, validate_xls_container, validate_xls_container_for_import,
        validate_xlsx_workbook_ranges,
    };

    fn string_range(rows: &[&[&str]]) -> Range<Data> {
        let mut cells = Vec::new();
        for (row_index, row) in rows.iter().enumerate() {
            for (column_index, value) in row.iter().enumerate() {
                cells.push(Cell::new(
                    (row_index as u32, column_index as u32),
                    Data::String((*value).to_owned()),
                ));
            }
        }
        Range::from_sparse(cells)
    }

    fn zip_with_entry(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start synthetic ZIP entry");
        writer
            .write_all(contents)
            .expect("write synthetic ZIP entry");
        writer.finish().expect("finish synthetic ZIP").into_inner()
    }

    fn compressed_zip_with_entry(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .expect("start compressed synthetic ZIP entry");
        writer
            .write_all(contents)
            .expect("write compressed synthetic ZIP entry");
        writer.finish().expect("finish synthetic ZIP").into_inner()
    }

    fn cfb_with_workbook(records: &[u8], dangerous_storage: Option<&str>) -> Vec<u8> {
        let mut compound =
            cfb::CompoundFile::create(Cursor::new(Vec::new())).expect("create synthetic CFB");
        if let Some(storage) = dangerous_storage {
            compound
                .create_storage(format!("/{storage}"))
                .expect("create synthetic dangerous storage");
        }
        {
            let mut workbook = compound
                .create_stream("/Workbook")
                .expect("create synthetic workbook stream");
            workbook
                .write_all(records)
                .expect("write synthetic BIFF records");
        }
        compound.into_inner().into_inner()
    }

    fn cfb_with_legacy_terminal_fat_marker(records: &[u8]) -> (Vec<u8>, usize, usize, u32) {
        let mut compound =
            cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new()))
                .expect("create synthetic CFB v3");
        {
            let mut workbook = compound
                .create_stream("/Workbook")
                .expect("create synthetic CFB v3 workbook stream");
            workbook
                .write_all(records)
                .expect("write synthetic CFB v3 BIFF records");
        }
        let mut bytes = compound.into_inner().into_inner();
        let sector_size = 512_usize;
        bytes[40..44].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            u16::from_le_bytes(
                bytes[26..28]
                    .try_into()
                    .expect("synthetic CFB v3 major version bytes"),
            ),
            3
        );
        assert_eq!(
            u16::from_le_bytes(
                bytes[30..32]
                    .try_into()
                    .expect("synthetic CFB v3 sector shift bytes"),
            ),
            9
        );
        assert_eq!(
            u32::from_le_bytes(
                bytes[40..44]
                    .try_into()
                    .expect("synthetic CFB v3 directory sector count bytes"),
            ),
            0
        );
        assert_eq!(bytes.len() % sector_size, 0);
        assert_eq!(
            u32::from_le_bytes(
                bytes[44..48]
                    .try_into()
                    .expect("synthetic CFB v3 FAT sector count bytes"),
            ),
            1
        );
        assert_eq!(
            u32::from_le_bytes(
                bytes[72..76]
                    .try_into()
                    .expect("synthetic CFB v3 DIFAT sector count bytes"),
            ),
            0
        );

        let old_fat_sector =
            u32::from_le_bytes(bytes[76..80].try_into().expect("synthetic FAT sector id"));
        let old_fat_start = (old_fat_sector as usize + 1) * sector_size;
        let fat_sector = bytes[old_fat_start..old_fat_start + sector_size].to_vec();
        let new_fat_sector = (bytes.len() / sector_size - 1) as u32;
        assert!((new_fat_sector as usize) < sector_size / 4);
        bytes.extend_from_slice(&fat_sector);
        bytes[76..80].copy_from_slice(&new_fat_sector.to_le_bytes());

        let new_fat_start = (new_fat_sector as usize + 1) * sector_size;
        let old_fat_entry_offset = new_fat_start + old_fat_sector as usize * 4;
        bytes[old_fat_entry_offset..old_fat_entry_offset + 4]
            .copy_from_slice(&CFB_FREE_SECTOR.to_le_bytes());
        let marker_offset = new_fat_start + new_fat_sector as usize * 4;
        bytes[marker_offset..marker_offset + 4].copy_from_slice(&CFB_END_OF_CHAIN.to_le_bytes());
        (bytes, marker_offset, old_fat_entry_offset, new_fat_sector)
    }

    fn push_biff_record(records: &mut Vec<u8>, record_type: u16, payload: &[u8]) {
        let payload_len = u16::try_from(payload.len()).expect("synthetic BIFF payload length");
        records.extend_from_slice(&record_type.to_le_bytes());
        records.extend_from_slice(&payload_len.to_le_bytes());
        records.extend_from_slice(payload);
    }

    fn utf16le_xml(value: &str) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xfe];
        for code_unit in value.encode_utf16() {
            bytes.extend_from_slice(&code_unit.to_le_bytes());
        }
        bytes
    }

    fn utf16be_xml(value: &str) -> Vec<u8> {
        let mut bytes = vec![0xfe, 0xff];
        for code_unit in value.encode_utf16() {
            bytes.extend_from_slice(&code_unit.to_be_bytes());
        }
        bytes
    }

    fn biff_dimensions(rows: usize, columns: usize) -> Vec<u8> {
        let mut dimensions = Vec::new();
        dimensions.extend_from_slice(&0_u32.to_le_bytes());
        dimensions.extend_from_slice(&(rows as u32).to_le_bytes());
        dimensions.extend_from_slice(&0_u16.to_le_bytes());
        dimensions.extend_from_slice(&(columns as u16).to_le_bytes());
        dimensions.extend_from_slice(&0_u16.to_le_bytes());
        dimensions
    }

    fn biff_workbook_with_sheet(sheet_records: &[(u16, Vec<u8>)]) -> Vec<u8> {
        biff_workbook_with_sheet_version(0x0600, sheet_records)
    }

    fn biff_workbook_with_sheet_version(
        biff_version: u16,
        sheet_records: &[(u16, Vec<u8>)],
    ) -> Vec<u8> {
        let mut records = Vec::new();
        let version = biff_version.to_le_bytes();
        push_biff_record(&mut records, 0x0809, &[version[0], version[1], 0x05, 0x00]);
        let bound_sheet_payload_start = records.len() + 4;
        push_biff_record(&mut records, 0x0085, &[0, 0, 0, 0, 0, 0]);
        push_biff_record(&mut records, 0x000a, &[]);
        let sheet_offset = u32::try_from(records.len()).expect("synthetic worksheet offset");
        records[bound_sheet_payload_start..bound_sheet_payload_start + 4]
            .copy_from_slice(&sheet_offset.to_le_bytes());
        push_biff_record(&mut records, 0x0809, &[version[0], version[1], 0x10, 0x00]);
        for (record_type, payload) in sheet_records {
            push_biff_record(&mut records, *record_type, payload);
        }
        push_biff_record(&mut records, 0x000a, &[]);
        records
    }

    fn xlsx_with_sheet_xml(sheet_xml: &str) -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;
        const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
        const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
        const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/worksheets/sheet1.xml", sheet_xml),
        ] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .expect("start synthetic XLSX entry");
            writer
                .write_all(contents.as_bytes())
                .expect("write synthetic XLSX entry");
        }
        writer.finish().expect("finish synthetic XLSX").into_inner()
    }

    #[test]
    fn detects_supported_adapters_from_headers() {
        let kakao = Range::from_sparse(vec![
            calamine::Cell::new((0, 0), Data::String("거래일시".to_owned())),
            calamine::Cell::new((0, 1), Data::String("거래구분".to_owned())),
            calamine::Cell::new((0, 2), Data::String("거래 후 잔액".to_owned())),
            calamine::Cell::new((0, 3), Data::String("결제 정보".to_owned())),
        ]);
        assert_eq!(
            detect_adapter(&kakao),
            Some(ExpenseAdapter::KakaoPayMoneyV1)
        );
    }

    #[test]
    fn parses_integer_money_without_floating_point_rounding() {
        assert_eq!(
            parse_amount(&Data::String("1,234원".to_owned())),
            Some(1_234)
        );
        assert_eq!(
            parse_amount(&Data::String("(5,000)".to_owned())),
            Some(-5_000)
        );
        assert_eq!(parse_amount(&Data::Float(1.0)), Some(1));
        assert_eq!(parse_amount(&Data::Float(1.5)), None);
        assert_eq!(parse_amount(&Data::Float(f64::EPSILON / 2.0)), None);
        assert_eq!(parse_amount(&Data::Float(f64::NAN)), None);
        assert_eq!(parse_amount(&Data::Float(f64::INFINITY)), None);
        assert_eq!(parse_amount(&Data::Float(f64::NEG_INFINITY)), None);
        assert_eq!(
            parse_amount(&Data::Float(MAX_SAFE_AMOUNT_MINOR as f64)),
            Some(MAX_SAFE_AMOUNT_MINOR)
        );
        assert_eq!(
            parse_amount(&Data::Float(-(MAX_SAFE_AMOUNT_MINOR as f64))),
            Some(-MAX_SAFE_AMOUNT_MINOR)
        );
        assert_eq!(
            parse_amount(&Data::Float((MAX_SAFE_AMOUNT_MINOR + 1) as f64)),
            None
        );
        assert_eq!(
            parse_amount(&Data::Int(MAX_SAFE_AMOUNT_MINOR)),
            Some(MAX_SAFE_AMOUNT_MINOR)
        );
        assert_eq!(
            parse_amount(&Data::Int(-MAX_SAFE_AMOUNT_MINOR)),
            Some(-MAX_SAFE_AMOUNT_MINOR)
        );
        assert_eq!(parse_amount(&Data::Int(MAX_SAFE_AMOUNT_MINOR + 1)), None);
        assert_eq!(parse_amount(&Data::Int(i64::MIN)), None);
        assert_eq!(
            parse_amount(&Data::String(MAX_SAFE_AMOUNT_MINOR.to_string())),
            Some(MAX_SAFE_AMOUNT_MINOR)
        );
        assert_eq!(
            parse_amount(&Data::String((MAX_SAFE_AMOUNT_MINOR + 1).to_string())),
            None
        );
        assert_eq!(parse_amount(&Data::String("1.5".to_owned())), None);
        assert_eq!(parse_amount(&Data::Bool(true)), None);
        assert_eq!(absolute_nonzero_amount(0), None);
        assert_eq!(absolute_nonzero_amount(i64::MIN), None);
        assert_eq!(
            absolute_nonzero_amount(MAX_SAFE_AMOUNT_MINOR),
            Some(MAX_SAFE_AMOUNT_MINOR)
        );
        assert_eq!(absolute_nonzero_amount(MAX_SAFE_AMOUNT_MINOR + 1), None);
    }

    #[test]
    fn parses_supported_korean_statement_dates() {
        assert_eq!(
            parse_datetime(&Data::String("2026.08.01 13:14:15".to_owned())).as_deref(),
            Some("2026-08-01T13:14:15")
        );
        assert_eq!(
            parse_datetime(&Data::String("2026-08-01".to_owned())).as_deref(),
            Some("2026-08-01T00:00:00")
        );
        assert_eq!(
            parse_datetime(&Data::DateTime(ExcelDateTime::new(
                46_235.5,
                ExcelDateTimeType::DateTime,
                false,
            )))
            .as_deref(),
            Some("2026-08-01T12:00:00")
        );
    }

    #[test]
    fn compound_card_header_starts_after_the_second_header_row() {
        let range = string_range(&[
            &["이용일자", "카드번호", "이용구분", "", ""],
            &["", "", "", "가맹점", "이용금액"],
            &["2026-08-04", "1234-****-5678", "일시불", "합성상점", "7000"],
        ]);
        assert_eq!(
            find_kb_card_layout(&range)
                .expect("find split header")
                .last_header_row,
            1
        );
        let (rows, rejected) = parse_kb_card(&range).expect("parse split card header");
        assert_eq!(rows.len(), 1);
        assert_eq!(rejected, 0);
    }

    #[test]
    fn card_parser_uses_header_columns_from_the_current_kb_export() {
        let range = string_range(&[
            &[
                "이용일자",
                "이용카드",
                "이용구분",
                "가맹점",
                "승인번호",
                "이용금액",
            ],
            &[
                "2026-07-21",
                "합성카드",
                "일시불",
                "합성상점",
                "승인식별자",
                "7000",
            ],
        ]);

        let (rows, rejected) = parse_kb_card(&range).expect("parse current KB card export");
        assert_eq!(rejected, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].amount_minor, 7_000);
        assert_eq!(rows[0].merchant.as_deref(), Some("합성상점"));
        assert!(rows[0].payment_method_fingerprint.is_some());
    }

    #[test]
    fn card_parser_prefers_the_leaf_amount_column_in_a_grouped_header() {
        let range = string_range(&[
            &["이용일자", "이용카드", "이용구분", "가맹점", "이용금액", ""],
            &["", "", "", "", "승인번호", "금액"],
            &[
                "2026-07-22",
                "합성카드",
                "일시불",
                "합성상점",
                "12345678",
                "8100",
            ],
        ]);

        let (rows, rejected) = parse_kb_card(&range).expect("parse grouped KB card export");
        assert_eq!(rejected, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].amount_minor, 8_100);
    }

    #[test]
    fn statement_metadata_sets_full_coverage_without_inventing_dates() {
        let range = string_range(&[
            &["조회기간", "2026.08.01 ~ 2026.08.31"],
            &["거래일시", "출금액", "입금액"],
            &["2026-08-12", "5000", ""],
        ]);
        let (start, end) = declared_coverage_period(&range).expect("declared statement period");
        assert_eq!(start.to_string(), "2026-08-01");
        assert_eq!(end.to_string(), "2026-08-31");

        let no_period = string_range(&[&["출력일시", "2026.08.31"], &["2026-08-12", "5000", ""]]);
        assert_eq!(declared_coverage_period(&no_period), None);
    }

    #[test]
    fn three_supported_statement_shapes_parse_without_executing_cells() {
        let kakao = string_range(&[
            &["거래일시", "거래구분", "거래금액", "", "", "결제 정보"],
            &["2026.08.01 12:00:00", "받기", "12000", "", "", "친구"],
            &["2026.08.02 13:00:00", "송금", "-8000", "", "", "동료"],
        ]);
        let (kakao_rows, rejected) = parse_kakao_pay(&kakao).expect("parse KakaoPay rows");
        assert_eq!(rejected, 0);
        assert_eq!(kakao_rows[0].direction, "in");
        assert_eq!(kakao_rows[1].direction, "out");
        assert!(kakao_rows.iter().all(|row| row.needs_review));

        let account = string_range(&[
            &[
                "거래일시",
                "거래내용",
                "거래처",
                "메모",
                "출금액",
                "입금액",
                "",
                "",
                "거래구분",
            ],
            &[
                "2026-08-03 09:00",
                "체크카드",
                "합성상점",
                "",
                "5000",
                "",
                "",
                "",
                "체크카드",
            ],
        ]);
        let (account_rows, rejected) = parse_kb_account(&account).expect("parse KB account rows");
        assert_eq!(rejected, 0);
        assert_eq!(account_rows[0].kind, "purchase");

        let card = string_range(&[
            &["이용일자", "카드번호", "이용구분", "가맹점", "이용금액"],
            &[
                "2026-08-04",
                "1234-****-5678",
                "취소",
                "=DDE('never')",
                "-7000",
            ],
        ]);
        let (card_rows, rejected) = parse_kb_card(&card).expect("parse KB card rows");
        assert_eq!(rejected, 0);
        assert_eq!(card_rows[0].kind, "refund");
        assert_eq!(card_rows[0].merchant.as_deref(), Some("=DDE('never')"));
    }

    #[test]
    fn malformed_rows_are_rejected_but_statement_footers_are_ignored() {
        let card = string_range(&[
            &["이용일자", "카드번호", "이용구분", "가맹점", "이용금액"],
            &["날짜오류", "1234", "일시불", "합성상점", "7000"],
            &["합계", "", "", "", "7000"],
            &["2026-08-04", "1234", "일시불", "합성상점", "7000"],
        ]);
        let (rows, rejected) = parse_kb_card(&card).expect("parse rows around footer");
        assert_eq!(rows.len(), 1);
        assert_eq!(rejected, 1);
    }

    #[test]
    fn source_identity_is_one_way_and_masked_financial_numbers_are_removed() {
        let account = string_range(&[&["계좌번호", "123-456-789012"]]);
        let fingerprint =
            source_discriminator_fingerprint(&account, ExpenseAdapter::KbAccountHistoryV1, &[]);
        assert_eq!(fingerprint.len(), 64);
        assert!(!fingerprint.contains("123"));
        let fallback_a = source_discriminator_fingerprint(
            &string_range(&[&["거래일시"]]),
            ExpenseAdapter::KbAccountHistoryV1,
            &[],
        );
        let fallback_b = source_discriminator_fingerprint(
            &string_range(&[&["거래일시"], &["새 결제수단"]]),
            ExpenseAdapter::KbAccountHistoryV1,
            &[],
        );
        assert_eq!(fallback_a, fallback_b);
        assert_eq!(
            redact_financial_identifiers("카드 ****-****-1234"),
            "카드 [금융식별번호 제거]"
        );
        assert_eq!(
            redact_financial_identifiers("계좌 123-456-789012"),
            "계좌 [금융식별번호 제거]"
        );
    }

    #[test]
    fn xlsx_metadata_preflight_bounds_declared_and_actual_allocations() {
        let valid_shared_strings = br#"<?xml version="1.0"?>
<x:sst xmlns:x="urn:test" x:uniqueCount="2">
  <!-- <x:si/> --><![CDATA[<x:si/>]]><x:si/><x:si/>
</x:sst>"#;
        assert!(validate_shared_strings_xml(valid_shared_strings).is_ok());

        let oversized_unique_count =
            format!("<sst uniqueCount=\"{}\"><si/></sst>", MAX_RANGE_CELLS + 1);
        assert!(validate_shared_strings_xml(oversized_unique_count.as_bytes()).is_err());

        let ten_sheets = format!(
            "<workbook><sheets>{}</sheets></workbook>",
            "<sheet/>".repeat(10)
        );
        assert!(validate_workbook_xml(ten_sheets.as_bytes()).is_ok());
        let eleven_sheets = format!(
            "<workbook><sheets>{}</sheets></workbook>",
            "<sheet/>".repeat(11)
        );
        assert!(validate_workbook_xml(eleven_sheets.as_bytes()).is_err());

        let shared_strings_zip = zip_with_entry("xl/sharedStrings.xml", valid_shared_strings);
        assert!(validate_ooxml_container(&shared_strings_zip).is_ok());
        let workbook_zip = zip_with_entry("xl/workbook.xml", ten_sheets.as_bytes());
        assert!(validate_ooxml_container(&workbook_zip).is_ok());
    }

    #[test]
    fn event_xml_preflight_handles_namespaces_utf16_styles_and_relationship_values() {
        let shared_strings = br#"<?xml version="1.0"?>
<x:sst xmlns:x="urn:test" x:uniqueCount="2"><x:si/><x:si/></x:sst>"#;
        assert!(validate_shared_strings_xml(shared_strings).is_ok());
        assert!(validate_shared_strings_xml(br#"<sst uniqueCount="1"><si/><si/></sst>"#,).is_err());
        assert!(validate_shared_strings_xml(br#"<!DOCTYPE sst><sst uniqueCount="0"/>"#).is_err());

        let utf16_workbook = utf16le_xml(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<x:workbook xmlns:x="urn:test"><x:sheets><x:sheet/></x:sheets></x:workbook>"#,
        );
        assert!(validate_workbook_xml(&utf16_workbook).is_ok());

        let ignored_style_xfs = format!(
            "<styleSheet><cellStyleXfs>{}</cellStyleXfs><cellXfs><xf/></cellXfs></styleSheet>",
            "<xf/>".repeat(MAX_STYLE_CELL_XFS + 1)
        );
        assert!(validate_styles_xml(ignored_style_xfs.as_bytes()).is_ok());
        let excessive_cell_xfs = format!(
            "<styleSheet><cellXfs>{}</cellXfs></styleSheet>",
            "<xf/>".repeat(MAX_STYLE_CELL_XFS + 1)
        );
        assert!(validate_styles_xml(excessive_cell_xfs.as_bytes()).is_err());
        let excessive_num_fmts = format!(
            "<styleSheet><numFmts>{}</numFmts></styleSheet>",
            "<numFmt/>".repeat(MAX_STYLE_NUMFMTS + 1)
        );
        assert!(validate_styles_xml(excessive_num_fmts.as_bytes()).is_err());

        let internal_and_comment = br#"<Relationships>
<!-- TargetMode="External" -->
<Relationship TargetMode=" Internal " Target="inside.xml"/>
</Relationships>"#;
        assert!(validate_relationships_xml(internal_and_comment).is_ok());
        let external = br#"<r:Relationships xmlns:r="urn:test">
<r:Relationship r:TargetMode=" External " Target="https://example.invalid"/>
</r:Relationships>"#;
        assert!(validate_relationships_xml(external).is_err());
        let escaped_external = br#"<Relationships>
<Relationship TargetMode="Ext&#x65;rnal" Target="https://example.invalid"/>
</Relationships>"#;
        assert!(validate_relationships_xml(escaped_external).is_err());
        let similar_but_internal = br#"<Relationships>
<Relationship TargetMode="ExternalLink" Target="inside.xml"/>
</Relationships>"#;
        assert!(validate_relationships_xml(similar_but_internal).is_ok());
        let utf16_external = utf16le_xml(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Relationships><Relationship TargetMode="External"/></Relationships>"#,
        );
        assert!(validate_relationships_xml(&utf16_external).is_err());
        let utf16be_external = utf16be_xml(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Relationships><Relationship TargetMode="External"/></Relationships>"#,
        );
        assert!(validate_relationships_xml(&utf16be_external).is_err());

        let internal_relationships_zip =
            zip_with_entry("xl/_rels/workbook.xml.rels", internal_and_comment);
        assert!(validate_ooxml_container(&internal_relationships_zip).is_ok());
        let oversized_styles = zip_with_entry(
            "xl/styles.xml",
            &vec![b'x'; MAX_STYLES_METADATA_BYTES as usize + 1],
        );
        assert!(validate_ooxml_container(&oversized_styles).is_err());
    }

    #[test]
    fn xlsx_cells_are_streamed_and_bounded_before_dense_range_creation() {
        let safe_sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <dimension ref="A1:XFD1048576"/>
  <sheetData><row r="5000"><c r="CV5000" t="inlineStr"><is><t>x</t></is></c></row></sheetData>
</worksheet>"#;
        let safe_xlsx = xlsx_with_sheet_xml(safe_sheet);
        assert!(validate_ooxml_container(&safe_xlsx).is_ok());
        let mut safe_workbook =
            open_workbook_auto_from_rs(Cursor::new(safe_xlsx)).expect("open safe synthetic XLSX");
        let safe_names = safe_workbook.sheet_names();
        let Sheets::Xlsx(safe_reader) = &mut safe_workbook else {
            panic!("synthetic workbook must be XLSX");
        };
        assert!(validate_xlsx_workbook_ranges(safe_reader, &safe_names).is_ok());

        let sparse_sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1048576"><c r="XFD1048576" t="inlineStr"><is><t>x</t></is></c></row></sheetData>
</worksheet>"#;
        let sparse_xlsx = xlsx_with_sheet_xml(sparse_sheet);
        let mut sparse_workbook = open_workbook_auto_from_rs(Cursor::new(sparse_xlsx))
            .expect("open sparse synthetic XLSX without materializing its range");
        let sparse_names = sparse_workbook.sheet_names();
        let Sheets::Xlsx(sparse_reader) = &mut sparse_workbook else {
            panic!("synthetic workbook must be XLSX");
        };
        assert!(validate_xlsx_workbook_ranges(sparse_reader, &sparse_names).is_err());

        let styled_empty_sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1048576"><c r="XFD1048576" s="0"/></row></sheetData>
</worksheet>"#;
        let styled_empty_xlsx = xlsx_with_sheet_xml(styled_empty_sheet);
        let mut styled_empty_workbook = open_workbook_auto_from_rs(Cursor::new(styled_empty_xlsx))
            .expect("open synthetic XLSX containing a styled empty cell");
        let styled_empty_names = styled_empty_workbook.sheet_names();
        let Sheets::Xlsx(styled_empty_reader) = &mut styled_empty_workbook else {
            panic!("synthetic workbook must be XLSX");
        };
        assert!(validate_xlsx_workbook_ranges(styled_empty_reader, &styled_empty_names).is_ok());
    }

    #[test]
    fn legacy_biff_preflight_bounds_dimensions_cells_formulas_and_merges() {
        let mut allowed_dimensions = Vec::new();
        allowed_dimensions.extend_from_slice(&0_u32.to_le_bytes());
        allowed_dimensions.extend_from_slice(&(MAX_ROWS as u32).to_le_bytes());
        allowed_dimensions.extend_from_slice(&0_u16.to_le_bytes());
        allowed_dimensions.extend_from_slice(&(MAX_COLUMNS as u16).to_le_bytes());
        allowed_dimensions.extend_from_slice(&0_u16.to_le_bytes());
        let mut allowed_number = vec![0_u8; 14];
        allowed_number[0..2].copy_from_slice(&((MAX_ROWS - 1) as u16).to_le_bytes());
        allowed_number[2..4].copy_from_slice(&((MAX_COLUMNS - 1) as u16).to_le_bytes());
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[
                (0x0200, allowed_dimensions.clone()),
                (0x0203, allowed_number),
            ]))
            .is_ok()
        );

        let mut oversized_dimensions = allowed_dimensions;
        oversized_dimensions[4..8].copy_from_slice(&((MAX_ROWS + 1) as u32).to_le_bytes());
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(
                0x0200,
                oversized_dimensions,
            )]))
            .is_err()
        );

        let mut distant_number = vec![0_u8; 14];
        distant_number[0..2].copy_from_slice(&(MAX_ROWS as u16).to_le_bytes());
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(0x0203, distant_number,)]))
                .is_err()
        );

        let mut distant_formula = vec![0_u8; 20];
        distant_formula[2..4].copy_from_slice(&(MAX_COLUMNS as u16).to_le_bytes());
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(0x0006, distant_formula,)]))
                .is_err()
        );

        let mut distant_merge = vec![0_u8; 10];
        distant_merge[0..2].copy_from_slice(&1_u16.to_le_bytes());
        distant_merge[4..6].copy_from_slice(&(MAX_ROWS as u16).to_le_bytes());
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(0x00e5, distant_merge,)]))
                .is_err()
        );

        let mut inconsistent_mul_rk = vec![0_u8; 12];
        inconsistent_mul_rk[10..12].copy_from_slice(&1_u16.to_le_bytes());
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(
                0x00bd,
                inconsistent_mul_rk,
            )]))
            .is_err()
        );

        let mut invalid_offset = biff_workbook_with_sheet(&[]);
        invalid_offset[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_biff_workbook_stream(&invalid_offset).is_err());
    }

    #[test]
    fn legacy_biff_dimensions_model_current_length_and_repeated_reserve_calls() {
        let max_dimensions = biff_dimensions(MAX_ROWS, MAX_COLUMNS);
        let number = vec![0_u8; 14];

        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[
                (0x0200, max_dimensions.clone()),
                (0x0200, max_dimensions.clone()),
            ]))
            .is_ok()
        );
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[
                (0x0203, number.clone()),
                (0x0200, max_dimensions.clone()),
            ]))
            .is_err()
        );
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[
                (0x0200, max_dimensions.clone()),
                (0x0203, number),
                (0x0200, max_dimensions),
            ]))
            .is_err()
        );

        let zero_dimensions = vec![0_u8; 14];
        let mut allowed = BiffSheetStats {
            value_cells: MAX_RANGE_CELLS - 1,
            ..BiffSheetStats::default()
        };
        assert!(validate_biff_dimensions(&zero_dimensions, &mut allowed).is_ok());
        assert_eq!(allowed.peak_reserved_cells, MAX_RANGE_CELLS);
        let mut rejected = BiffSheetStats {
            value_cells: MAX_RANGE_CELLS,
            ..BiffSheetStats::default()
        };
        assert!(validate_biff_dimensions(&zero_dimensions, &mut rejected).is_err());
    }

    #[test]
    fn legacy_formula_strings_are_validated_without_double_counting() {
        let mut string_formula = vec![0_u8; 20];
        string_formula[12] = 0xff;
        string_formula[13] = 0xff;
        let formula_string = vec![1, 0, 0, b'x'];
        let valid = biff_workbook_with_sheet(&[
            (0x0006, string_formula.clone()),
            (0x0207, formula_string.clone()),
        ]);
        assert!(validate_biff_workbook_stream(&valid).is_ok());
        let sheet_offset = usize::try_from(u32::from_le_bytes([
            valid[12], valid[13], valid[14], valid[15],
        ]))
        .expect("synthetic worksheet offset");
        let stats = validate_biff_sheet(&valid, sheet_offset).expect("validate formula sheet");
        assert_eq!(stats.raw_cells, 1);
        assert_eq!(stats.value_cells, 1);
        assert_eq!(stats.formula_cells, 1);

        let legacy_formula_string = vec![3, 0, b'o', b'l', b'd'];
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet_version(
                0x0500,
                &[
                    (0x0006, string_formula.clone()),
                    (0x0207, legacy_formula_string),
                ],
            ))
            .is_ok()
        );

        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(
                0x0207,
                formula_string.clone(),
            )]))
            .is_err()
        );
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(
                0x0006,
                string_formula.clone(),
            )]))
            .is_err()
        );
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[
                (0x0006, vec![0_u8; 20]),
                (0x0207, formula_string.clone()),
            ]))
            .is_err()
        );
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[
                (0x0006, string_formula),
                (0x0207, formula_string.clone()),
                (0x0207, formula_string),
            ]))
            .is_err()
        );

        let mut oversized_label = vec![0_u8; 8];
        oversized_label[6..8].copy_from_slice(&((MAX_CELL_CHARS + 1) as u16).to_le_bytes());
        assert!(
            validate_biff_workbook_stream(&biff_workbook_with_sheet(&[(0x0204, oversized_label,)]))
                .is_err()
        );
    }

    #[test]
    fn legacy_biff_sst_counts_and_stream_size_are_bounded() {
        let mut valid = Vec::new();
        valid.extend_from_slice(&1_u32.to_le_bytes());
        valid.extend_from_slice(&1_u32.to_le_bytes());
        valid.extend_from_slice(&[0, 0, 0]);
        assert!(validate_biff_sst(&[], &valid, 0).is_ok());

        let mut oversized_count = Vec::new();
        oversized_count.extend_from_slice(&((MAX_RANGE_CELLS + 1) as u32).to_le_bytes());
        oversized_count.extend_from_slice(&((MAX_RANGE_CELLS + 1) as u32).to_le_bytes());
        assert!(validate_biff_sst(&[], &oversized_count, 0).is_err());

        let mut inconsistent = Vec::new();
        inconsistent.extend_from_slice(&1_u32.to_le_bytes());
        inconsistent.extend_from_slice(&1_u32.to_le_bytes());
        inconsistent.extend_from_slice(&[0, 0]);
        assert!(validate_biff_sst(&[], &inconsistent, 0).is_err());

        let oversized_stream = vec![0_u8; 8 + MAX_RANGE_CELLS * 3 + 1];
        assert!(validate_biff_sst(&[], &oversized_stream, 0).is_err());
    }

    #[test]
    fn legacy_biff_sst_parses_unicode_rich_extended_and_continued_strings() {
        let mut variants = Vec::new();
        variants.extend_from_slice(&4_u32.to_le_bytes());
        variants.extend_from_slice(&4_u32.to_le_bytes());
        variants.extend_from_slice(&[3, 0, 0, b'a', b'b', b'c']);
        variants.extend_from_slice(&[2, 0, 1, b'A', 0, 0xb0, 0x03]);
        variants.extend_from_slice(&[1, 0, 0x08, 1, 0, b'r', 0, 0, 0, 0]);
        variants.extend_from_slice(&[1, 0, 0x04, 3, 0, 0, 0, b'e', 1, 2, 3]);
        assert!(validate_biff_sst(&[], &variants, 0).is_ok());

        let mut header_boundary = Vec::new();
        header_boundary.extend_from_slice(&1_u32.to_le_bytes());
        header_boundary.extend_from_slice(&1_u32.to_le_bytes());
        header_boundary.extend_from_slice(&[4, 0, 0]);
        let mut header_boundary_continue = Vec::new();
        push_biff_record(
            &mut header_boundary_continue,
            0x003c,
            &[0, b'a', b'b', b'c', b'd'],
        );
        assert!(validate_biff_sst(&header_boundary_continue, &header_boundary, 0).is_ok());

        let mut compressed_to_unicode = Vec::new();
        compressed_to_unicode.extend_from_slice(&1_u32.to_le_bytes());
        compressed_to_unicode.extend_from_slice(&1_u32.to_le_bytes());
        compressed_to_unicode.extend_from_slice(&[3, 0, 0, b'a']);
        let mut unicode_continue = Vec::new();
        push_biff_record(&mut unicode_continue, 0x003c, &[1, b'b', 0, b'c', 0]);
        assert!(validate_biff_sst(&unicode_continue, &compressed_to_unicode, 0).is_ok());

        let mut unicode_to_compressed = Vec::new();
        unicode_to_compressed.extend_from_slice(&1_u32.to_le_bytes());
        unicode_to_compressed.extend_from_slice(&1_u32.to_le_bytes());
        unicode_to_compressed.extend_from_slice(&[3, 0, 1, b'a', 0]);
        let mut compressed_continue = Vec::new();
        push_biff_record(&mut compressed_continue, 0x003c, &[0, b'b', b'c']);
        assert!(validate_biff_sst(&compressed_continue, &unicode_to_compressed, 0).is_ok());

        let mut split_header = Vec::new();
        split_header.extend_from_slice(&1_u32.to_le_bytes());
        split_header.extend_from_slice(&1_u32.to_le_bytes());
        split_header.push(1);
        let mut split_header_continue = Vec::new();
        push_biff_record(&mut split_header_continue, 0x003c, &[0, 0, b'x']);
        assert!(validate_biff_sst(&split_header_continue, &split_header, 0).is_err());

        let mut split_rich_header = Vec::new();
        split_rich_header.extend_from_slice(&1_u32.to_le_bytes());
        split_rich_header.extend_from_slice(&1_u32.to_le_bytes());
        split_rich_header.extend_from_slice(&[1, 0, 0x08, 1]);
        let mut split_rich_continue = Vec::new();
        push_biff_record(&mut split_rich_continue, 0x003c, &[0, b'x', 0, 0, 0, 0]);
        assert!(validate_biff_sst(&split_rich_continue, &split_rich_header, 0).is_err());

        let mut split_extension_header = Vec::new();
        split_extension_header.extend_from_slice(&1_u32.to_le_bytes());
        split_extension_header.extend_from_slice(&1_u32.to_le_bytes());
        split_extension_header.extend_from_slice(&[1, 0, 0x04, 3, 0]);
        let mut split_extension_continue = Vec::new();
        push_biff_record(
            &mut split_extension_continue,
            0x003c,
            &[0, 0, b'x', 1, 2, 3],
        );
        assert!(validate_biff_sst(&split_extension_continue, &split_extension_header, 0).is_err());

        let mut continued_rich_data = Vec::new();
        continued_rich_data.extend_from_slice(&1_u32.to_le_bytes());
        continued_rich_data.extend_from_slice(&1_u32.to_le_bytes());
        continued_rich_data.extend_from_slice(&[1, 0, 0x08, 1, 0, b'r', 1, 2]);
        let mut continued_rich_tail = Vec::new();
        push_biff_record(&mut continued_rich_tail, 0x003c, &[3, 4]);
        assert!(validate_biff_sst(&continued_rich_tail, &continued_rich_data, 0).is_ok());

        let mut continued_extension_data = Vec::new();
        continued_extension_data.extend_from_slice(&1_u32.to_le_bytes());
        continued_extension_data.extend_from_slice(&1_u32.to_le_bytes());
        continued_extension_data.extend_from_slice(&[1, 0, 0x04, 3, 0, 0, 0, b'e', 1]);
        let mut continued_extension_tail = Vec::new();
        push_biff_record(&mut continued_extension_tail, 0x003c, &[2, 3]);
        assert!(
            validate_biff_sst(&continued_extension_tail, &continued_extension_data, 0,).is_ok()
        );

        let mut missing_continuation = Vec::new();
        missing_continuation.extend_from_slice(&1_u32.to_le_bytes());
        missing_continuation.extend_from_slice(&1_u32.to_le_bytes());
        missing_continuation.extend_from_slice(&[1, 0, 0]);
        assert!(validate_biff_sst(&[], &missing_continuation, 0).is_err());

        let mut lying_declaration = Vec::new();
        lying_declaration.extend_from_slice(&1_u32.to_le_bytes());
        lying_declaration.extend_from_slice(&1_u32.to_le_bytes());
        lying_declaration.extend_from_slice(&[1, 0, 0, b'a', 1, 0, 0, b'b']);
        assert!(validate_biff_sst(&[], &lying_declaration, 0).is_err());
    }

    #[test]
    fn legacy_biff_sst_allows_large_normal_data_but_bounds_continuation_chains() {
        const STRING_COUNT: usize = 100;
        const BIFF8_RECORD_PAYLOAD_LIMIT: usize = 8_224;
        let mut payload = Vec::new();
        payload.extend_from_slice(&(STRING_COUNT as u32).to_le_bytes());
        payload.extend_from_slice(&(STRING_COUNT as u32).to_le_bytes());
        payload.extend_from_slice(&(MAX_CELL_CHARS as u16).to_le_bytes());
        payload.push(0);
        let first_fragment = BIFF8_RECORD_PAYLOAD_LIMIT - payload.len();
        payload.extend(std::iter::repeat_n(b'a', first_fragment));
        let mut continuations = Vec::new();
        let mut first_tail = Vec::with_capacity(MAX_CELL_CHARS - first_fragment + 1);
        first_tail.push(0);
        first_tail.extend(std::iter::repeat_n(b'a', MAX_CELL_CHARS - first_fragment));
        push_biff_record(&mut continuations, 0x003c, &first_tail);
        for _ in 1..STRING_COUNT {
            let mut string = Vec::with_capacity(BIFF8_RECORD_PAYLOAD_LIMIT);
            string.extend_from_slice(&(MAX_CELL_CHARS as u16).to_le_bytes());
            string.push(0);
            let fragment = BIFF8_RECORD_PAYLOAD_LIMIT - string.len();
            string.extend(std::iter::repeat_n(b'a', fragment));
            push_biff_record(&mut continuations, 0x003c, &string);
            let mut tail = Vec::with_capacity(MAX_CELL_CHARS - fragment + 1);
            tail.push(0);
            tail.extend(std::iter::repeat_n(b'a', MAX_CELL_CHARS - fragment));
            push_biff_record(&mut continuations, 0x003c, &tail);
        }
        assert!(continuations.len() + payload.len() > 1_572_864);
        assert!(validate_biff_sst(&continuations, &payload, 0).is_ok());

        let mut allowed_chain = Vec::new();
        push_biff_record(&mut allowed_chain, 0x0809, &[0x00, 0x06, 0x05, 0x00]);
        for _ in 0..MAX_BIFF_SST_CONTINUE_RECORDS {
            push_biff_record(&mut allowed_chain, 0x003c, &[1]);
        }
        push_biff_record(&mut allowed_chain, 0x000a, &[]);
        assert!(validate_biff_workbook_stream(&allowed_chain).is_ok());

        let mut rejected_chain = allowed_chain;
        let eof = rejected_chain.split_off(rejected_chain.len() - 4);
        push_biff_record(&mut rejected_chain, 0x003c, &[1]);
        rejected_chain.extend_from_slice(&eof);
        assert!(validate_biff_workbook_stream(&rejected_chain).is_err());

        let mut empty_continue = Vec::new();
        push_biff_record(&mut empty_continue, 0x0809, &[0x00, 0x06, 0x05, 0x00]);
        push_biff_record(&mut empty_continue, 0x003c, &[]);
        push_biff_record(&mut empty_continue, 0x000a, &[]);
        assert!(validate_biff_workbook_stream(&empty_continue).is_err());
    }

    #[test]
    fn unsafe_ooxml_parts_and_external_relationships_are_rejected() {
        let macro_book = zip_with_entry("xl/vbaProject.bin", b"synthetic macro marker");
        assert!(validate_ooxml_container(&macro_book).is_err());

        let external = zip_with_entry(
            "xl/_rels/workbook.xml.rels",
            br#"<Relationships><Relationship TargetMode="External" Target="https://example.invalid"/></Relationships>"#,
        );
        assert!(validate_ooxml_container(&external).is_err());

        let spaced_external = zip_with_entry(
            "xl/_rels/workbook.xml.rels",
            br#"<Relationships><Relationship TargetMode = ' eXtErNaL ' Target="https://example.invalid"/></Relationships>"#,
        );
        assert!(validate_ooxml_container(&spaced_external).is_err());

        let external_link_part =
            zip_with_entry("xl/externalLinks/externalLink1.xml", br#"<externalLink/>"#);
        assert!(validate_ooxml_container(&external_link_part).is_err());

        let compression_bomb = compressed_zip_with_entry("xl/worksheets/sheet1.xml", &[0; 200_000]);
        assert!(validate_ooxml_container(&compression_bomb).is_err());
    }

    #[test]
    fn unsafe_legacy_xls_macro_objects_and_external_links_are_rejected() {
        let safe_records = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(validate_xls_container(&cfb_with_workbook(&safe_records, None)).is_ok());
        assert!(
            validate_xls_container(&cfb_with_workbook(&safe_records, Some("_VBA_PROJECT_CUR")))
                .is_err()
        );

        let external_supbook = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BOF
            0xae, 0x01, 0x04, 0x00, 0x01, 0x00, 0x03, 0x00, // external SupBook
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(validate_xls_container(&cfb_with_workbook(&external_supbook, None)).is_err());

        let internal_supbook_and_sheet = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0xae, 0x01, 0x04, 0x00, 0x01, 0x00, 0x01, 0x04, // self SupBook
            0x17, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, // internal XTI
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(
            validate_xls_container(&cfb_with_workbook(&internal_supbook_and_sheet, None)).is_ok()
        );

        let addin_supbook_and_sheet = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0xae, 0x01, 0x04, 0x00, 0x01, 0x00, 0x01, 0x3a, // add-in SupBook
            0x17, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, // add-in XTI
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(
            validate_xls_container(&cfb_with_workbook(&addin_supbook_and_sheet, None)).is_err()
        );

        let missing_supbook_reference = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0xae, 0x01, 0x04, 0x00, 0x01, 0x00, 0x01, 0x04, // self SupBook
            0x17, 0x00, 0x08, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, // invalid XTI index
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(
            validate_xls_container(&cfb_with_workbook(&missing_supbook_reference, None)).is_err()
        );

        let inconsistent_xti_count = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0xae, 0x01, 0x04, 0x00, 0x01, 0x00, 0x01, 0x04, // self SupBook
            0x17, 0x00, 0x08, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, // only one XTI
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(validate_xls_container(&cfb_with_workbook(&inconsistent_xti_count, None)).is_err());

        let legacy_internal_sheet = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x05, 0x05, 0x00, // BIFF5 workbook BOF
            0x17, 0x00, 0x08, 0x00, 0x06, 0x03, b'S', b'h', b'e', b'e', b't', b'2', 0x0a, 0x00,
            0x00, 0x00, // EOF
        ];
        assert!(validate_xls_container(&cfb_with_workbook(&legacy_internal_sheet, None)).is_ok());
        let legacy_external_sheet = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x05, 0x05, 0x00, // BIFF5 workbook BOF
            0x17, 0x00, 0x08, 0x00, 0x06, 0x01, b'B', b'o', b'o', b'k', b'.', b'x', 0x0a, 0x00,
            0x00, 0x00, // EOF
        ];
        assert!(validate_xls_container(&cfb_with_workbook(&legacy_external_sheet, None)).is_err());

        let unknown_version_internal_sheet = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x07, 0x05, 0x00, // unknown version -> BIFF8
            0xae, 0x01, 0x04, 0x00, 0x01, 0x00, 0x01, 0x04, // self SupBook
            0x17, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, // internal XTI
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(
            validate_xls_container(&cfb_with_workbook(&unknown_version_internal_sheet, None))
                .is_ok()
        );

        let embedded_object = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BOF
            0x5d, 0x00, 0x00, 0x00, // Obj
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(validate_xls_container(&cfb_with_workbook(&embedded_object, None)).is_err());
    }

    #[test]
    fn legacy_terminal_fat_marker_is_canonicalized_in_memory() {
        let safe_records = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        let (mut legacy, marker_offset, _, _) = cfb_with_legacy_terminal_fat_marker(&safe_records);
        assert_eq!(
            u32::from_le_bytes(
                legacy[marker_offset..marker_offset + 4]
                    .try_into()
                    .expect("synthetic legacy FAT marker bytes"),
            ),
            CFB_END_OF_CHAIN
        );

        validate_xls_container_for_import(&mut legacy)
            .expect("canonicalize a bounded legacy FAT marker");

        assert_eq!(
            u32::from_le_bytes(
                legacy[marker_offset..marker_offset + 4]
                    .try_into()
                    .expect("canonical synthetic FAT marker bytes"),
            ),
            CFB_FAT_SECTOR
        );
    }

    #[test]
    fn malformed_legacy_fat_marker_variants_remain_rejected() {
        let safe_records = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];

        let (mut free_marker, marker_offset, _, _) =
            cfb_with_legacy_terminal_fat_marker(&safe_records);
        free_marker[marker_offset..marker_offset + 4]
            .copy_from_slice(&CFB_FREE_SECTOR.to_le_bytes());
        assert!(validate_xls_container(&free_marker).is_err());

        let (mut nonterminal_fat, _, _, _) = cfb_with_legacy_terminal_fat_marker(&safe_records);
        nonterminal_fat.extend_from_slice(&[0_u8; 512]);
        assert!(validate_xls_container(&nonterminal_fat).is_err());

        let (mut reserved_pointer, _, spare_entry_offset, fat_sector) =
            cfb_with_legacy_terminal_fat_marker(&safe_records);
        reserved_pointer[spare_entry_offset..spare_entry_offset + 4]
            .copy_from_slice(&fat_sector.to_le_bytes());
        assert!(validate_xls_container(&reserved_pointer).is_err());

        let (mut nonfree_tail, _, _, fat_sector) =
            cfb_with_legacy_terminal_fat_marker(&safe_records);
        let sector_count = nonfree_tail.len() / 512 - 1;
        let first_tail_entry_offset = (fat_sector as usize + 1) * 512 + sector_count * 4;
        nonfree_tail[first_tail_entry_offset..first_tail_entry_offset + 4]
            .copy_from_slice(&CFB_END_OF_CHAIN.to_le_bytes());
        assert!(validate_xls_container(&nonfree_tail).is_err());
    }

    #[test]
    fn legacy_xls_container_walk_is_bounded_before_workbook_parsing() {
        let safe_records = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BIFF8 workbook BOF
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        let mut compound =
            cfb::CompoundFile::create(Cursor::new(Vec::new())).expect("create bounded CFB");
        {
            let mut workbook = compound
                .create_stream("/Workbook")
                .expect("create bounded workbook stream");
            workbook
                .write_all(&safe_records)
                .expect("write bounded BIFF records");
        }
        for index in 0..MAX_CFB_ENTRIES {
            compound
                .create_stream(format!("/entry{index:04}"))
                .expect("create bounded entry");
        }
        let too_many_entries = compound.into_inner().into_inner();
        assert!(validate_xls_container(&too_many_entries).is_err());
    }

    #[test]
    fn sheet_dimensions_and_cell_strings_are_bounded() {
        let last_allowed_cell = Range::from_sparse(vec![Cell::new(
            ((MAX_ROWS - 1) as u32, (MAX_COLUMNS - 1) as u32),
            Data::String("x".to_owned()),
        )]);
        assert!(validate_range(&last_allowed_cell).is_ok());

        let too_many_rows = Range::from_sparse(vec![Cell::new(
            (MAX_ROWS as u32, 0),
            Data::String("x".to_owned()),
        )]);
        assert!(validate_range(&too_many_rows).is_err());

        let too_many_columns = Range::from_sparse(vec![Cell::new(
            (0, MAX_COLUMNS as u32),
            Data::String("x".to_owned()),
        )]);
        assert!(validate_range(&too_many_columns).is_err());

        let too_long = Range::from_sparse(vec![Cell::new(
            (0, 0),
            Data::String("x".repeat(MAX_CELL_CHARS + 1)),
        )]);
        assert!(validate_range(&too_long).is_err());
    }
}
