use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use calamine::{Data, DataType, Range, Reader, open_workbook_auto_from_rs};
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub(crate) const MAX_EXPENSE_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DECRYPTED_BYTES: usize = 40 * 1024 * 1024;
const MAX_SHEETS: usize = 10;
const MAX_ROWS: usize = 5_000;
const MAX_COLUMNS: usize = 100;
const MAX_CELL_CHARS: usize = 16_384;
const MAX_ZIP_ENTRIES: usize = 2_000;
const MAX_ZIP_ENTRY_BYTES: u64 = 40 * 1024 * 1024;
const MAX_ZIP_TOTAL_BYTES: u64 = 80 * 1024 * 1024;
const MAX_ZIP_COMPRESSION_RATIO: u64 = 100;
const MAX_RELATIONSHIP_BYTES: u64 = 2 * 1024 * 1024;
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
    let workbook_bytes = if extension == "xlsx" && encrypted_bytes.starts_with(OLE_MAGIC) {
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
        validate_xls_container(&workbook_bytes)?;
    }

    let mut workbook = open_workbook_auto_from_rs(Cursor::new(workbook_bytes))
        .map_err(|_| "지출 통합문서를 열 수 없습니다.".to_owned())?;
    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() || sheet_names.len() > MAX_SHEETS {
        return Err("지출 통합문서의 시트 수가 허용 범위를 벗어났습니다.".to_owned());
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

fn canonical_expense_path(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err("지출 파일 경로가 올바르지 않습니다.".to_owned());
    }
    fs::canonicalize(trimmed).map_err(|_| "선택한 지출 파일을 찾을 수 없습니다.".to_owned())
}

fn validate_range(range: &Range<Data>) -> Result<(), String> {
    let (height, width) = range.get_size();
    if height > MAX_ROWS || width > MAX_COLUMNS {
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

fn cfb_sector<'a>(
    bytes: &'a [u8],
    sector_size: usize,
    sector_count: usize,
    sector_id: u32,
) -> Result<&'a [u8], String> {
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

fn cfb_fat_entry(
    bytes: &[u8],
    sector_size: usize,
    sector_count: usize,
    fat_sectors: &[u32],
    sector_id: u32,
) -> Result<u32, String> {
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
    let fat_sector = cfb_sector(bytes, sector_size, sector_count, fat_sector_id)?;
    read_cfb_u32(fat_sector, entry_index * 4)
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

fn preflight_xls_container(bytes: &[u8]) -> Result<PathBuf, String> {
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
    if bytes.len() < sector_size.saturating_mul(2) || bytes.len() % sector_size != 0 {
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
    for (index, is_fat) in seen_fat.iter().enumerate() {
        if *is_fat
            && cfb_fat_entry(bytes, sector_size, sector_count, &fat_sectors, index as u32)?
                != CFB_FAT_SECTOR
        {
            return Err("The XLS FAT sector marker is invalid.".to_owned());
        }
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
    Ok(PathBuf::from(format!("/{workbook_name}")))
}

fn validate_xls_container(bytes: &[u8]) -> Result<(), String> {
    // Parse and bound the raw directory/FAT first. cfb::Entries::walk intentionally is not used:
    // an adversarial unbalanced sibling tree can otherwise allocate before caller-side limits run.
    let workbook_path = preflight_xls_container(bytes)?;
    let mut compound = cfb::CompoundFile::open(Cursor::new(bytes))
        .map_err(|_| "The XLS compound document is invalid.".to_owned())?;
    let mut stream = compound.open_stream(&workbook_path).map_err(|_| {
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

fn validate_biff_workbook_stream(bytes: &[u8]) -> Result<(), String> {
    let mut offset = 0_usize;
    let mut saw_bof = false;
    while offset < bytes.len() {
        let remaining = &bytes[offset..];
        if remaining.len() < 4 {
            if remaining.iter().all(|byte| *byte == 0) {
                break;
            }
            return Err("The XLS BIFF record header is truncated.".to_owned());
        }
        let record_type = u16::from_le_bytes([remaining[0], remaining[1]]);
        let record_len = usize::from(u16::from_le_bytes([remaining[2], remaining[3]]));
        if record_type == 0 && record_len == 0 {
            if remaining.iter().all(|byte| *byte == 0) {
                break;
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
        let payload = &bytes[payload_start..payload_end];

        if matches!(record_type, 0x0009 | 0x0209 | 0x0409 | 0x0809) {
            if payload.len() < 4 {
                return Err("The XLS BOF record is invalid.".to_owned());
            }
            saw_bof = true;
            let substream_type = u16::from_le_bytes([payload[2], payload[3]]);
            if substream_type == 0x0040 {
                return Err("XLS macro sheets are not accepted.".to_owned());
            }
        }
        if record_type == 0x0085 {
            if payload.len() < 6 {
                return Err("The XLS BoundSheet record is invalid.".to_owned());
            }
            if matches!(payload[5] & 0x0f, 0x01 | 0x06) {
                return Err("XLS macro or VBA module sheets are not accepted.".to_owned());
            }
        }
        if record_type == 0x01ae {
            if payload.len() < 4 {
                return Err("The XLS SupBook record is invalid.".to_owned());
            }
            let character_count = u16::from_le_bytes([payload[2], payload[3]]);
            if !matches!(character_count, 0x0401 | 0x3a01) {
                return Err("XLS external workbook links are not accepted.".to_owned());
            }
        }
        if matches!(
            record_type,
            0x0017 | 0x0023 | 0x0059 | 0x005a | 0x005d | 0x00d3 | 0x01b8
        ) {
            return Err(
                "XLS external links, embedded objects, and macro projects are not accepted."
                    .to_owned(),
            );
        }
        offset = payload_end;
    }
    if !saw_bof {
        return Err("The XLS workbook stream has no BIFF BOF record.".to_owned());
    }
    Ok(())
}

fn validate_ooxml_container(bytes: &[u8]) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "XLSX ZIP 구조가 올바르지 않습니다.".to_owned())?;
    if archive.len() == 0 || archive.len() > MAX_ZIP_ENTRIES {
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

        if normalized_name.ends_with(".rels") {
            if uncompressed > MAX_RELATIONSHIP_BYTES {
                return Err("XLSX 관계 파일 크기가 안전 제한을 초과했습니다.".to_owned());
            }
            let mut relationship = Vec::with_capacity(uncompressed as usize);
            entry
                .read_to_end(&mut relationship)
                .map_err(|_| "XLSX 관계 파일을 검사할 수 없습니다.".to_owned())?;
            let has_target_mode_attribute = relationship
                .windows(b"targetmode".len())
                .any(|window| window.eq_ignore_ascii_case(b"targetmode"));
            if has_target_mode_attribute {
                return Err("외부 링크가 포함된 XLSX는 가져올 수 없습니다.".to_owned());
            }
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
    let header = find_compound_header_row(range, &["이용일자", "가맹점", "이용금액"])?;
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
        let Some(signed_amount) = row.get(4).and_then(parse_amount) else {
            rejected += 1;
            continue;
        };
        let Some(amount_minor) = absolute_nonzero_amount(signed_amount) else {
            rejected += 1;
            continue;
        };
        let card_fingerprint_source = row
            .get(1)
            .map(cell_text)
            .filter(|value| !value.trim().is_empty());
        let usage_type = bounded_optional(row.get(2).map(cell_text));
        let merchant = bounded_optional(row.get(3).map(cell_text));
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

fn find_compound_header_row(range: &Range<Data>, required: &[&str]) -> Result<usize, String> {
    let rows = range.rows().take(20).collect::<Vec<_>>();
    rows.iter()
        .enumerate()
        .find_map(|(index, row)| {
            let current = row.iter().map(cell_text).collect::<Vec<_>>().join("|");
            if required.iter().all(|value| current.contains(value)) {
                return Some(index);
            }
            let next = rows.get(index + 1).copied().unwrap_or(&[]);
            let joined = row
                .iter()
                .chain(next.iter())
                .map(cell_text)
                .collect::<Vec<_>>()
                .join("|");
            required
                .iter()
                .all(|value| joined.contains(value))
                .then_some(index + 1)
        })
        .ok_or_else(|| "카드 이용내역의 헤더를 찾지 못했습니다.".to_owned())
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
    if let Some(value) = cell.as_i64() {
        return Some(value);
    }
    if let Some(value) = cell.as_f64() {
        if value.is_finite() && value.fract().abs() < f64::EPSILON {
            return i64::try_from(value as i128).ok();
        }
    }
    let value = cell_text(cell);
    let trimmed = value.trim();
    let negative_parentheses = trimmed.starts_with('(') && trimmed.ends_with(')');
    let normalized = trimmed
        .trim_matches(['(', ')'])
        .chars()
        .filter(|character| !matches!(character, ',' | '₩' | '원' | ' '))
        .collect::<String>();
    let parsed = normalized.parse::<i64>().ok()?;
    if negative_parentheses {
        parsed.checked_abs()?.checked_neg()
    } else {
        Some(parsed)
    }
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

    use calamine::{Cell, Data, ExcelDateTime, ExcelDateTimeType, Range};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::{
        ExpenseAdapter, MAX_CELL_CHARS, MAX_CFB_ENTRIES, MAX_COLUMNS, MAX_ROWS,
        MAX_SAFE_AMOUNT_MINOR, absolute_nonzero_amount, declared_coverage_period, detect_adapter,
        find_compound_header_row, parse_amount, parse_datetime, parse_kakao_pay, parse_kb_account,
        parse_kb_card, redact_financial_identifiers, source_discriminator_fingerprint,
        validate_ooxml_container, validate_range, validate_xls_container,
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
        assert_eq!(parse_amount(&Data::Float(1.5)), None);
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
            find_compound_header_row(&range, &["이용일자", "가맹점", "이용금액"])
                .expect("find split header"),
            1
        );
        let (rows, rejected) = parse_kb_card(&range).expect("parse split card header");
        assert_eq!(rows.len(), 1);
        assert_eq!(rejected, 0);
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
    fn unsafe_ooxml_parts_and_external_relationships_are_rejected() {
        let macro_book = zip_with_entry("xl/vbaProject.bin", b"synthetic macro marker");
        assert!(validate_ooxml_container(&macro_book).is_err());

        let external = zip_with_entry(
            "xl/_rels/workbook.xml.rels",
            br#"<Relationship TargetMode="External" Target="https://example.invalid"/>"#,
        );
        assert!(validate_ooxml_container(&external).is_err());

        let spaced_external = zip_with_entry(
            "xl/_rels/workbook.xml.rels",
            br#"<Relationship TARGETMODE = ' External ' Target="https://example.invalid"/>"#,
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

        let embedded_object = [
            0x09, 0x08, 0x04, 0x00, 0x00, 0x06, 0x05, 0x00, // BOF
            0x5d, 0x00, 0x00, 0x00, // Obj
            0x0a, 0x00, 0x00, 0x00, // EOF
        ];
        assert!(validate_xls_container(&cfb_with_workbook(&embedded_object, None)).is_err());
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
