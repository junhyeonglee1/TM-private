use std::{fmt, sync::Arc};

#[cfg(not(test))]
use std::env;

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

#[cfg(not(test))]
pub(super) const EXPENSE_DATA_KEY_ENV: &str = "TM_EXPENSE_DATA_KEY_V1";
pub(super) const EXPENSE_EXPECTED_KEY_FINGERPRINT_ENV: &str = "TM_EXPENSE_EXPECTED_KEY_FINGERPRINT";
pub(super) const EXPENSE_DATA_KEY_VERSION: u32 = 1;

const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const MAX_PLAINTEXT_BYTES: usize = 4 * 1024;
const KEY_PREFIX: &str = "tm_exp_v1_";
const KEY_FINGERPRINT_PREFIX: &str = "tm_exp_kfp_v1_";
const KEY_FINGERPRINT_DOMAIN: &[u8] = b"tm-expense:key-fingerprint:v1\0";
const BLIND_INDEX_DOMAIN: &[u8] = b"tm-expense:blind-index:v1\0";
const IDEMPOTENT_NONCE_DOMAIN: &[u8] = b"tm-expense-idempotent-nonce-v1\0";
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub(super) struct ExpenseCrypto {
    key: Arc<Zeroizing<[u8; KEY_BYTES]>>,
}

impl fmt::Debug for ExpenseCrypto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExpenseCrypto")
            .field("key_version", &EXPENSE_DATA_KEY_VERSION)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EncryptedExpenseValue {
    pub key_version: u32,
    pub nonce: String,
    pub ciphertext: String,
    pub blind_index: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpenseCryptoError {
    MissingKey,
    InvalidKey,
    ValueTooLarge,
    RandomnessUnavailable,
    EncryptionFailed,
    InvalidEnvelope,
    DecryptionFailed,
    InvalidUtf8,
    KeyVerificationFailed,
    RolloutLocked,
}

impl fmt::Display for ExpenseCryptoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingKey => "expense data encryption key is not configured",
            Self::InvalidKey => "expense data encryption key is invalid",
            Self::ValueTooLarge => "expense sensitive value exceeds the size limit",
            Self::RandomnessUnavailable => "secure expense nonce generation failed",
            Self::EncryptionFailed => "expense sensitive value encryption failed",
            Self::InvalidEnvelope => "expense encrypted value is invalid",
            Self::DecryptionFailed => "expense encrypted value could not be authenticated",
            Self::InvalidUtf8 => "expense decrypted value is not UTF-8",
            Self::KeyVerificationFailed => "expense encryption key verification failed",
            Self::RolloutLocked => "expense rollout is locked pending key verification",
        })
    }
}

impl std::error::Error for ExpenseCryptoError {}

impl ExpenseCrypto {
    #[cfg(not(test))]
    pub(super) fn from_env() -> Result<Self, ExpenseCryptoError> {
        let encoded = Zeroizing::new(
            env::var(EXPENSE_DATA_KEY_ENV).map_err(|_| ExpenseCryptoError::MissingKey)?,
        );
        Self::from_encoded_key(&encoded)
    }

    fn from_encoded_key(encoded: &str) -> Result<Self, ExpenseCryptoError> {
        let encoded = encoded
            .trim()
            .strip_prefix(KEY_PREFIX)
            .unwrap_or(encoded.trim());
        let key = Zeroizing::new(decode_key(encoded).ok_or(ExpenseCryptoError::InvalidKey)?);
        if key.iter().all(|byte| *byte == 0) {
            return Err(ExpenseCryptoError::InvalidKey);
        }
        Ok(Self { key: Arc::new(key) })
    }

    #[cfg(test)]
    pub(super) fn for_test() -> Result<Self, ExpenseCryptoError> {
        Self::from_encoded_key("9dbdfe3b08213d08a84217a7f1b735f86e2ed20b4a87ffedee422661a7219d49")
    }

    pub(super) fn encrypt(
        &self,
        field: &str,
        aad: &[u8],
        plaintext: &str,
    ) -> Result<EncryptedExpenseValue, ExpenseCryptoError> {
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err(ExpenseCryptoError::ValueTooLarge);
        }
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| ExpenseCryptoError::RandomnessUnavailable)?;
        self.encrypt_with_nonce(field, aad, plaintext, nonce)
    }

    pub(super) fn encrypt_idempotent(
        &self,
        field: &str,
        aad: &[u8],
        plaintext: &str,
        idempotency_key: &str,
    ) -> Result<EncryptedExpenseValue, ExpenseCryptoError> {
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err(ExpenseCryptoError::ValueTooLarge);
        }
        let mut nonce_input = Zeroizing::new(Vec::with_capacity(
            IDEMPOTENT_NONCE_DOMAIN.len()
                + idempotency_key.len()
                + field.len()
                + aad.len()
                + plaintext.len()
                + 3,
        ));
        nonce_input.extend_from_slice(IDEMPOTENT_NONCE_DOMAIN);
        nonce_input.extend_from_slice(idempotency_key.as_bytes());
        nonce_input.push(0);
        nonce_input.extend_from_slice(field.as_bytes());
        nonce_input.push(0);
        nonce_input.extend_from_slice(aad);
        nonce_input.push(0);
        nonce_input.extend_from_slice(plaintext.as_bytes());
        let nonce_material = hmac_sha256(&**self.key, &nonce_input);
        let mut nonce = [0_u8; NONCE_BYTES];
        nonce.copy_from_slice(&nonce_material[..NONCE_BYTES]);
        self.encrypt_with_nonce(field, aad, plaintext, nonce)
    }

    fn encrypt_with_nonce(
        &self,
        field: &str,
        aad: &[u8],
        plaintext: &str,
        nonce: [u8; NONCE_BYTES],
    ) -> Result<EncryptedExpenseValue, ExpenseCryptoError> {
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err(ExpenseCryptoError::ValueTooLarge);
        }
        let cipher = XChaCha20Poly1305::new((&**self.key).into());
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad,
                },
            )
            .map_err(|_| ExpenseCryptoError::EncryptionFailed)?;
        Ok(EncryptedExpenseValue {
            key_version: EXPENSE_DATA_KEY_VERSION,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
            blind_index: self.blind_index(field, plaintext),
        })
    }

    pub(super) fn decrypt(
        &self,
        key_version: u32,
        nonce: &str,
        ciphertext: &str,
        aad: &[u8],
    ) -> Result<String, ExpenseCryptoError> {
        if key_version != EXPENSE_DATA_KEY_VERSION {
            return Err(ExpenseCryptoError::InvalidEnvelope);
        }
        let nonce = URL_SAFE_NO_PAD
            .decode(nonce)
            .map_err(|_| ExpenseCryptoError::InvalidEnvelope)?;
        let nonce: [u8; NONCE_BYTES] = nonce
            .try_into()
            .map_err(|_| ExpenseCryptoError::InvalidEnvelope)?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(ciphertext)
            .map_err(|_| ExpenseCryptoError::InvalidEnvelope)?;
        if ciphertext.len() < 16 || ciphertext.len() > MAX_PLAINTEXT_BYTES + 16 {
            return Err(ExpenseCryptoError::InvalidEnvelope);
        }
        let cipher = XChaCha20Poly1305::new((&**self.key).into());
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad,
                    },
                )
                .map_err(|_| ExpenseCryptoError::DecryptionFailed)?,
        );
        std::str::from_utf8(&plaintext)
            .map(str::to_owned)
            .map_err(|_| ExpenseCryptoError::InvalidUtf8)
    }

    pub(super) fn blind_index(&self, field: &str, value: &str) -> String {
        let normalized = Zeroizing::new(normalize_blind_value(value));
        let mut message = Zeroizing::new(Vec::with_capacity(
            BLIND_INDEX_DOMAIN.len() + field.len() + normalized.len() + 1,
        ));
        message.extend_from_slice(BLIND_INDEX_DOMAIN);
        message.extend_from_slice(field.as_bytes());
        message.push(0);
        message.extend_from_slice(normalized.as_bytes());
        let digest = hmac_sha256(&**self.key, &message);
        encode_hex(&digest[..])
    }

    pub(super) fn key_fingerprint(&self) -> String {
        let digest = hmac_sha256(&**self.key, KEY_FINGERPRINT_DOMAIN);
        format!("{KEY_FINGERPRINT_PREFIX}{}", encode_hex(&digest[..]))
    }
}

fn normalize_blind_value(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut hmac = <HmacSha256 as Mac>::new_from_slice(key)
        .expect("HMAC-SHA256 accepts the fixed expense key length");
    hmac.update(message);
    let mut tag = hmac.finalize().into_bytes();
    let mut output = Zeroizing::new([0_u8; 32]);
    output.copy_from_slice(&tag);
    tag[..].zeroize();
    output
}

fn decode_key_hex(value: &str) -> Option<[u8; KEY_BYTES]> {
    if value.len() != KEY_BYTES * 2 {
        return None;
    }
    let mut key = [0_u8; KEY_BYTES];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        key[index] = (high << 4) | low;
    }
    Some(key)
}

fn decode_key(value: &str) -> Option<[u8; KEY_BYTES]> {
    decode_key_hex(value).or_else(|| decode_key_base64(value))
}

fn decode_key_base64(value: &str) -> Option<[u8; KEY_BYTES]> {
    let mut output = Zeroizing::new([0_u8; 48]);
    let decoded = STANDARD
        .decode_slice(value, &mut output[..])
        .or_else(|_| STANDARD_NO_PAD.decode_slice(value, &mut output[..]))
        .or_else(|_| URL_SAFE_NO_PAD.decode_slice(value, &mut output[..]))
        .ok()?;
    if decoded != KEY_BYTES {
        return None;
    }
    let mut key = [0_u8; KEY_BYTES];
    key.copy_from_slice(&output[..KEY_BYTES]);
    Some(key)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn encode_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    use super::{ExpenseCrypto, ExpenseCryptoError, KEY_FINGERPRINT_PREFIX, KEY_PREFIX};

    fn crypto() -> ExpenseCrypto {
        ExpenseCrypto::from_encoded_key(
            "9dbdfe3b08213d08a84217a7f1b735f86e2ed20b4a87ffedee422661a7219d49",
        )
        .expect("test key")
    }

    #[test]
    fn xchacha_round_trip_binds_ciphertext_to_aad() {
        let crypto = crypto();
        let encrypted = crypto
            .encrypt(
                "merchant",
                b"posting:row-1:merchant",
                "  Example Merchant  ",
            )
            .expect("encrypt value");
        assert_eq!(encrypted.key_version, 1);
        assert_eq!(encrypted.nonce.len(), 32);
        assert_eq!(encrypted.blind_index.len(), 64);
        assert_eq!(
            crypto
                .decrypt(
                    encrypted.key_version,
                    &encrypted.nonce,
                    &encrypted.ciphertext,
                    b"posting:row-1:merchant",
                )
                .expect("decrypt value"),
            "  Example Merchant  "
        );
        assert_eq!(
            crypto.decrypt(
                encrypted.key_version,
                &encrypted.nonce,
                &encrypted.ciphertext,
                b"posting:row-2:merchant",
            ),
            Err(ExpenseCryptoError::DecryptionFailed)
        );
    }

    #[test]
    fn blind_index_is_normalized_and_field_separated() {
        let crypto = crypto();
        assert_eq!(
            crypto.blind_index("merchant", "example merchant"),
            "f87380476bb58619ea4261916c635775cb9c775fece3b3921926436e2f90ada1"
        );
        assert_eq!(
            crypto.blind_index("merchant", " Example   MERCHANT "),
            crypto.blind_index("merchant", "example merchant")
        );
        assert_ne!(
            crypto.blind_index("merchant", "example merchant"),
            crypto.blind_index("counterparty", "example merchant")
        );
        let other = ExpenseCrypto::from_encoded_key(&"22".repeat(32)).expect("other key");
        assert_ne!(
            crypto.blind_index("payment_method", "1234"),
            other.blind_index("payment_method", "1234"),
            "low-entropy identifiers must remain dependent on the secret key"
        );
    }

    #[test]
    fn idempotent_encryption_replays_without_reusing_nonce_for_changed_plaintext() {
        let crypto = crypto();
        let first = crypto
            .encrypt_idempotent(
                "merchant",
                b"tm-expense:v1:source:row:merchant",
                "Example Merchant",
                "import-request-1",
            )
            .expect("encrypt first value");
        let replay = crypto
            .encrypt_idempotent(
                "merchant",
                b"tm-expense:v1:source:row:merchant",
                "Example Merchant",
                "import-request-1",
            )
            .expect("encrypt replay value");
        let changed = crypto
            .encrypt_idempotent(
                "merchant",
                b"tm-expense:v1:source:row:merchant",
                "Changed Merchant",
                "import-request-1",
            )
            .expect("encrypt changed value");
        assert_eq!(first, replay);
        assert_eq!(first.nonce, "EtQxhdadYeEL_ehoQRITgzU6EBbW-Ct5");
        assert_ne!(first.nonce, changed.nonce);
    }

    #[test]
    fn keys_must_be_nonzero_32_byte_hex() {
        assert!(ExpenseCrypto::from_encoded_key("abcd").is_err());
        assert!(ExpenseCrypto::from_encoded_key(&"00".repeat(32)).is_err());
        assert!(ExpenseCrypto::from_encoded_key(&"11".repeat(32)).is_ok());
        assert!(
            ExpenseCrypto::from_encoded_key(&format!("{KEY_PREFIX}{}", "11".repeat(32))).is_ok()
        );
        assert!(
            ExpenseCrypto::from_encoded_key(&format!(
                "{KEY_PREFIX}{}",
                URL_SAFE_NO_PAD.encode([0x11_u8; 32])
            ))
            .is_ok()
        );
    }

    #[test]
    fn key_fingerprint_is_domain_separated_and_deterministic() {
        let first = ExpenseCrypto::from_encoded_key(&"11".repeat(32)).expect("first key");
        let second = ExpenseCrypto::from_encoded_key(&"22".repeat(32)).expect("second key");
        assert_eq!(
            first.key_fingerprint(),
            "tm_exp_kfp_v1_acde74882428fa9681f2c3dae8bd3d59b729a84b8e79f6911af04117ab2644dd"
        );
        assert!(first.key_fingerprint().starts_with(KEY_FINGERPRINT_PREFIX));
        assert_ne!(first.key_fingerprint(), second.key_fingerprint());
        assert!(!format!("{first:?}").contains(&"11".repeat(32)));
    }

    #[test]
    fn ciphertext_authentication_rejects_a_different_key() {
        let encrypted = crypto()
            .encrypt("memo", b"tm-expense:v1:row:memo", "private memo")
            .expect("encrypt memo");
        let other = ExpenseCrypto::from_encoded_key(&"22".repeat(32)).expect("other key");
        assert_eq!(
            other.decrypt(
                encrypted.key_version,
                &encrypted.nonce,
                &encrypted.ciphertext,
                b"tm-expense:v1:row:memo",
            ),
            Err(ExpenseCryptoError::DecryptionFailed)
        );
    }
}
