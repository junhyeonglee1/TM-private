use std::env;

#[cfg(windows)]
use std::{
    io::Write,
    os::windows::process::CommandExt,
    process::{Command, Stdio},
};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chacha20poly1305::{
    Key, XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tm_core::{EncryptedExpenseText, ExpenseCryptoProbe, TmCore};
use zeroize::{Zeroize, Zeroizing};

type HmacSha256 = Hmac<Sha256>;
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const KEY_PREFIX: &str = "tm_exp_v1_";
const PROBE_TEXT: &[u8] = b"tm-expense-key-probe-v1";
const PROBE_AAD: &str = "tm-expense:v1:crypto-probe:value";
const BLIND_INDEX_DOMAIN: &[u8] = b"tm-expense:blind-index:v1\0";
const IDEMPOTENT_NONCE_DOMAIN: &[u8] = b"tm-expense-idempotent-nonce-v1\0";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREDENTIAL_RESOURCE: &str = "TM Expense Data";
#[cfg(windows)]
const CREDENTIAL_USER: &str = "v1";

#[derive(Debug)]
pub(crate) struct ExpenseCrypto {
    key: Zeroizing<[u8; KEY_BYTES]>,
}

impl ExpenseCrypto {
    pub(crate) fn load_or_create(core: &TmCore) -> Result<Self, String> {
        if let Ok(value) = env::var("TM_EXPENSE_DATA_KEY_V1") {
            return Self::from_encoded(&value);
        }
        #[cfg(windows)]
        {
            if let Some(value) = load_windows_credential()? {
                return Self::from_encoded(&value);
            }
            ensure_ledger_allows_new_key(core)?;
            let mut key = [0_u8; KEY_BYTES];
            getrandom::fill(&mut key)
                .map_err(|_| "지출 암호화 키를 안전하게 생성할 수 없습니다.".to_owned())?;
            let encoded = format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(key));
            store_windows_credential(&encoded)?;
            key.zeroize();
            let persisted = load_windows_credential()?
                .ok_or_else(|| "Windows 지출 암호화 키 저장을 확인할 수 없습니다.".to_owned())?;
            return Self::from_encoded(&persisted);
        }
        #[cfg(not(windows))]
        {
            let _ = core;
            Err("TM_EXPENSE_DATA_KEY_V1 is required on this platform.".to_owned())
        }
    }

    fn from_encoded(value: &str) -> Result<Self, String> {
        let encoded = value
            .trim()
            .strip_prefix(KEY_PREFIX)
            .unwrap_or(value.trim());
        let mut decoded = decode_key(encoded)
            .ok_or_else(|| "TM_EXPENSE_DATA_KEY_V1 형식이 올바르지 않습니다.".to_owned())?;
        if decoded.len() != KEY_BYTES {
            decoded.zeroize();
            return Err("TM_EXPENSE_DATA_KEY_V1은 32바이트 키여야 합니다.".to_owned());
        }
        if decoded.iter().all(|byte| *byte == 0) {
            decoded.zeroize();
            return Err("TM_EXPENSE_DATA_KEY_V1은 0이 아닌 키여야 합니다.".to_owned());
        }
        let mut key = [0_u8; KEY_BYTES];
        key.copy_from_slice(&decoded);
        decoded.zeroize();
        Ok(Self {
            key: Zeroizing::new(key),
        })
    }

    pub(crate) fn ensure_probe(&self, core: &TmCore) -> Result<(), String> {
        match core
            .expense_crypto_probe()
            .map_err(|error| error.to_string())?
        {
            Some(probe) => {
                let plaintext = self.decrypt_probe(&probe)?;
                if plaintext.as_slice() != PROBE_TEXT {
                    return Err("지출 암호화 키가 기존 데이터와 일치하지 않습니다.".to_owned());
                }
            }
            None => {
                let (nonce, ciphertext) = self.encrypt_bytes(PROBE_TEXT, PROBE_AAD)?;
                core.initialize_expense_crypto_probe_if_ledger_empty(ExpenseCryptoProbe {
                    key_version: 1,
                    nonce,
                    ciphertext,
                    aad: PROBE_AAD.to_owned(),
                })
                .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    pub(crate) fn encrypt_text(
        &self,
        field: &str,
        plaintext: &str,
        aad: String,
    ) -> Result<EncryptedExpenseText, String> {
        let trimmed = plaintext.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 500 {
            return Err("암호화할 지출 표시값의 길이가 올바르지 않습니다.".to_owned());
        }
        let (nonce, ciphertext) = self.encrypt_bytes(trimmed.as_bytes(), &aad)?;
        Ok(EncryptedExpenseText {
            key_version: 1,
            nonce,
            ciphertext,
            aad,
            blind_index: self.blind_index(field, trimmed)?,
        })
    }

    pub(crate) fn encrypt_text_idempotent(
        &self,
        field: &str,
        plaintext: &str,
        aad: String,
        idempotency_key: &str,
    ) -> Result<EncryptedExpenseText, String> {
        let trimmed = plaintext.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 500 {
            return Err("암호화할 지출 표시값의 길이가 올바르지 않습니다.".to_owned());
        }
        let mut nonce_input = Zeroizing::new(Vec::with_capacity(
            IDEMPOTENT_NONCE_DOMAIN.len()
                + idempotency_key.len()
                + field.len()
                + aad.len()
                + trimmed.len()
                + 3,
        ));
        nonce_input.extend_from_slice(IDEMPOTENT_NONCE_DOMAIN);
        nonce_input.extend_from_slice(idempotency_key.as_bytes());
        nonce_input.push(0);
        nonce_input.extend_from_slice(field.as_bytes());
        nonce_input.push(0);
        nonce_input.extend_from_slice(aad.as_bytes());
        nonce_input.push(0);
        nonce_input.extend_from_slice(trimmed.as_bytes());
        let mut mac = <HmacSha256 as Mac>::new_from_slice(self.key.as_slice())
            .map_err(|_| "지출 멱등 암호화 nonce를 만들 수 없습니다.".to_owned())?;
        mac.update(&nonce_input);
        let material = mac.finalize().into_bytes();
        let mut nonce = [0_u8; NONCE_BYTES];
        nonce.copy_from_slice(&material[..NONCE_BYTES]);
        let ciphertext = self.encrypt_bytes_with_nonce(trimmed.as_bytes(), &aad, nonce)?;
        Ok(EncryptedExpenseText {
            key_version: 1,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext,
            aad,
            blind_index: self.blind_index(field, trimmed)?,
        })
    }

    pub(crate) fn decrypt_text(
        &self,
        value: &EncryptedExpenseText,
        expected_aad: &str,
    ) -> Result<String, String> {
        if value.key_version != 1 || value.aad != expected_aad {
            return Err("지원하지 않는 지출 암호화 키 버전입니다.".to_owned());
        }
        let nonce = decode_nonce(&value.nonce)?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(&value.ciphertext)
            .map_err(|_| "지출 암호문 형식이 올바르지 않습니다.".to_owned())?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.key.as_slice()));
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: value.aad.as_bytes(),
                },
            )
            .map_err(|_| "지출 암호문을 복호화할 수 없습니다.".to_owned())?;
        String::from_utf8(plaintext)
            .map_err(|_| "지출 암호문의 문자열 인코딩이 올바르지 않습니다.".to_owned())
    }

    fn decrypt_probe(&self, value: &ExpenseCryptoProbe) -> Result<Zeroizing<Vec<u8>>, String> {
        if value.key_version != 1 || value.aad != PROBE_AAD {
            return Err("지출 암호화 키 확인값이 올바르지 않습니다.".to_owned());
        }
        let nonce = decode_nonce(&value.nonce)?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(&value.ciphertext)
            .map_err(|_| "지출 암호화 키 확인값이 올바르지 않습니다.".to_owned())?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.key.as_slice()));
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: value.aad.as_bytes(),
                },
            )
            .map_err(|_| "지출 암호화 키가 기존 데이터와 일치하지 않습니다.".to_owned())?;
        Ok(Zeroizing::new(plaintext))
    }

    fn encrypt_bytes(&self, plaintext: &[u8], aad: &str) -> Result<(String, String), String> {
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce)
            .map_err(|_| "지출 암호화 nonce를 생성할 수 없습니다.".to_owned())?;
        let ciphertext = self.encrypt_bytes_with_nonce(plaintext, aad, nonce)?;
        Ok((URL_SAFE_NO_PAD.encode(nonce), ciphertext))
    }

    fn encrypt_bytes_with_nonce(
        &self,
        plaintext: &[u8],
        aad: &str,
        nonce: [u8; NONCE_BYTES],
    ) -> Result<String, String> {
        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.key.as_slice()));
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "지출 값을 암호화할 수 없습니다.".to_owned())?;
        Ok(URL_SAFE_NO_PAD.encode(ciphertext))
    }

    pub(crate) fn blind_index(&self, field: &str, plaintext: &str) -> Result<String, String> {
        let normalized = plaintext
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let mut mac = <HmacSha256 as Mac>::new_from_slice(self.key.as_slice())
            .map_err(|_| "지출 blind index 키를 만들 수 없습니다.".to_owned())?;
        mac.update(BLIND_INDEX_DOMAIN);
        mac.update(field.as_bytes());
        mac.update(&[0]);
        mac.update(normalized.as_bytes());
        Ok(format!("{:x}", mac.finalize().into_bytes()))
    }
}

fn ensure_ledger_allows_new_key(core: &TmCore) -> Result<(), String> {
    let has_probe = core
        .expense_crypto_probe()
        .map_err(|error| error.to_string())?
        .is_some();
    let has_payloads = core
        .has_encrypted_expense_payloads()
        .map_err(|error| error.to_string())?;
    if has_probe || has_payloads {
        return Err(
            "기존 지출 원장의 암호화 키를 찾지 못했습니다. 새 키를 만들지 않고 지출 기능을 잠갔습니다."
                .to_owned(),
        );
    }
    Ok(())
}

fn decode_key(value: &str) -> Option<Vec<u8>> {
    decode_hex_key(value)
        .or_else(|| URL_SAFE_NO_PAD.decode(value).ok())
        .or_else(|| STANDARD_NO_PAD.decode(value).ok())
        .or_else(|| STANDARD.decode(value).ok())
}

fn decode_hex_key(value: &str) -> Option<Vec<u8>> {
    if value.len() != KEY_BYTES * 2 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from((high << 4) | low).ok()
        })
        .collect()
}

fn decode_nonce(value: &str) -> Result<[u8; NONCE_BYTES], String> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "지출 nonce 형식이 올바르지 않습니다.".to_owned())?;
    decoded
        .try_into()
        .map_err(|_| "지출 nonce 길이가 올바르지 않습니다.".to_owned())
}

#[cfg(windows)]
fn load_windows_credential() -> Result<Option<String>, String> {
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
try {{
  $credential = $vault.Retrieve('{CREDENTIAL_RESOURCE}', '{CREDENTIAL_USER}')
  $credential.RetrievePassword()
  [Console]::Out.Write($credential.Password)
}} catch [System.Exception] {{
  if ($_.Exception.HResult -eq -2147023728) {{ exit 3 }}
  throw
}}"#
    );
    let output = Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Windows Credential Locker를 열 수 없습니다.".to_owned())?;
    if output.status.code() == Some(3) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err("Windows Credential Locker에서 지출 키를 읽을 수 없습니다.".to_owned());
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| "Windows 지출 키 인코딩이 올바르지 않습니다.".to_owned())?;
    Ok(Some(value))
}

#[cfg(windows)]
fn store_windows_credential(value: &str) -> Result<(), String> {
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$value = [Console]::In.ReadToEnd().Trim()
if ([string]::IsNullOrWhiteSpace($value)) {{ throw 'empty credential' }}
$vault = [Windows.Security.Credentials.PasswordVault,Windows.Security.Credentials,ContentType=WindowsRuntime]::new()
try {{
  $existing = $vault.Retrieve('{CREDENTIAL_RESOURCE}', '{CREDENTIAL_USER}')
  $vault.Remove($existing)
}} catch [System.Exception] {{
  if ($_.Exception.HResult -ne -2147023728) {{ throw }}
}}
$credential = [Windows.Security.Credentials.PasswordCredential,Windows.Security.Credentials,ContentType=WindowsRuntime]::new('{CREDENTIAL_RESOURCE}', '{CREDENTIAL_USER}', $value)
$vault.Add($credential)"#
    );
    let mut command = Command::new("powershell.exe");
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| "Windows Credential Locker를 열 수 없습니다.".to_owned())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Windows Credential Locker 입력을 열 수 없습니다.".to_owned())?;
    stdin
        .write_all(value.as_bytes())
        .map_err(|_| "Windows Credential Locker에 지출 키를 전달할 수 없습니다.".to_owned())?;
    drop(stdin);
    let status = child
        .wait()
        .map_err(|_| "Windows Credential Locker 저장 결과를 확인할 수 없습니다.".to_owned())?;
    if !status.success() {
        return Err("Windows Credential Locker에 지출 키를 저장할 수 없습니다.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ExpenseCrypto, KEY_PREFIX, ensure_ledger_allows_new_key};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use tm_core::{ExpenseCryptoProbe, TmCore, TmHome};

    #[test]
    fn encrypted_text_round_trip_preserves_aad() {
        let encoded = format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode([7_u8; 32]));
        let crypto = ExpenseCrypto::from_encoded(&encoded).expect("crypto");
        let encrypted = crypto
            .encrypt_text(
                "merchant",
                "테스트 가맹점",
                "tm-expense:v1:test:merchant".to_owned(),
            )
            .expect("encrypt");
        assert_eq!(
            crypto
                .decrypt_text(&encrypted, "tm-expense:v1:test:merchant")
                .expect("decrypt"),
            "테스트 가맹점"
        );
        assert!(
            crypto
                .decrypt_text(&encrypted, "tm-expense:v1:other:merchant")
                .is_err()
        );
        assert_eq!(encrypted.blind_index.len(), 64);
    }

    #[test]
    fn key_formats_match_cloud_configuration_and_blind_indexes_are_field_scoped() {
        assert!(ExpenseCrypto::from_encoded(&"11".repeat(32)).is_ok());
        assert!(ExpenseCrypto::from_encoded(&"00".repeat(32)).is_err());
        let encoded = format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode([7_u8; 32]));
        let crypto = ExpenseCrypto::from_encoded(&encoded).expect("crypto");
        assert_ne!(
            crypto
                .blind_index("merchant", " same value ")
                .expect("merchant index"),
            crypto
                .blind_index("counterparty", "same   value")
                .expect("counterparty index")
        );
        let other = ExpenseCrypto::from_encoded(&"22".repeat(32)).expect("other key");
        assert_ne!(
            crypto
                .blind_index("payment_method", "1234")
                .expect("local payment index"),
            other
                .blind_index("payment_method", "1234")
                .expect("other payment index")
        );

        let cloud_vector = ExpenseCrypto::from_encoded(
            "9dbdfe3b08213d08a84217a7f1b735f86e2ed20b4a87ffedee422661a7219d49",
        )
        .expect("known-vector key");
        assert_eq!(
            cloud_vector
                .blind_index("merchant", " Example   MERCHANT ")
                .expect("known-vector blind index"),
            "f87380476bb58619ea4261916c635775cb9c775fece3b3921926436e2f90ada1"
        );
    }

    #[test]
    fn idempotent_encryption_reuses_ciphertext_only_for_the_same_action_key() {
        let encoded = format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode([7_u8; 32]));
        let crypto = ExpenseCrypto::from_encoded(&encoded).expect("crypto");
        let encrypt = |key| {
            crypto
                .encrypt_text_idempotent(
                    "merchant",
                    "동일 가맹점",
                    "tm-expense:v1:test:merchant".to_owned(),
                    key,
                )
                .expect("encrypt")
        };
        let first = encrypt("desktop-expense:019mock-1");
        let replay = encrypt("desktop-expense:019mock-1");
        let new_action = encrypt("desktop-expense:019mock-2");
        assert_eq!(first, replay);
        assert_ne!(first.nonce, new_action.nonce);
        assert_ne!(first.ciphertext, new_action.ciphertext);
    }

    #[test]
    fn missing_key_never_rekeys_an_existing_expense_ledger() {
        let temporary = tempfile::tempdir().expect("temporary home");
        let core = TmCore::open(TmHome::new(temporary.path())).expect("core");
        ensure_ledger_allows_new_key(&core).expect("empty ledger allows first key");
        core.initialize_expense_crypto_probe(ExpenseCryptoProbe {
            key_version: 1,
            nonce: URL_SAFE_NO_PAD.encode([1_u8; 24]),
            ciphertext: URL_SAFE_NO_PAD.encode([2_u8; 32]),
            aad: "tm-expense:v1:crypto-probe:value".to_owned(),
        })
        .expect("probe");
        assert!(ensure_ledger_allows_new_key(&core).is_err());
    }
}
