use std::{env, fmt, sync::Arc};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

pub(super) const MAIL_DATA_KEY_ENV: &str = "TM_MAIL_DATA_KEY_V1";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const MAX_PLAINTEXT_BYTES: usize = 8 * 1024;
const KEY_PREFIX: &str = "tm_mail_v1_";
const FINGERPRINT_PREFIX: &str = "tm_mail_kfp_v1_";
const FINGERPRINT_DOMAIN: &[u8] = b"tm-mail:key-fingerprint:v1\0";
const BLIND_INDEX_DOMAIN: &[u8] = b"tm-mail:blind-index:v1\0";
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub(super) struct MailCrypto {
    key: Arc<Zeroizing<[u8; KEY_BYTES]>>,
}

impl fmt::Debug for MailCrypto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("MailCrypto").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MailCryptoError {
    MissingKey,
    InvalidKey,
    ValueTooLarge,
    RandomnessUnavailable,
    EncryptionFailed,
    InvalidEnvelope,
    DecryptionFailed,
    InvalidUtf8,
    KeyVerificationFailed,
}

impl fmt::Display for MailCryptoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingKey => "mail data encryption key is not configured",
            Self::InvalidKey => "mail data encryption key is invalid",
            Self::ValueTooLarge => "mail sensitive value exceeds the size limit",
            Self::RandomnessUnavailable => "secure mail nonce generation failed",
            Self::EncryptionFailed => "mail sensitive value encryption failed",
            Self::InvalidEnvelope => "mail encrypted value is invalid",
            Self::DecryptionFailed => "mail encrypted value could not be authenticated",
            Self::InvalidUtf8 => "mail decrypted value is not UTF-8",
            Self::KeyVerificationFailed => "mail encryption key verification failed",
        })
    }
}

impl std::error::Error for MailCryptoError {}

impl MailCrypto {
    #[cfg(not(test))]
    pub(super) fn from_env() -> Result<Self, MailCryptoError> {
        let encoded =
            Zeroizing::new(env::var(MAIL_DATA_KEY_ENV).map_err(|_| MailCryptoError::MissingKey)?);
        Self::from_encoded_key(&encoded)
    }

    #[cfg(test)]
    pub(super) fn from_env() -> Result<Self, MailCryptoError> {
        Self::from_encoded_key("5b4b63916627163207a0030a3089f7146f4090286b317fdbf7c6ce49d33ccb68")
    }

    fn from_encoded_key(value: &str) -> Result<Self, MailCryptoError> {
        let value = value
            .trim()
            .strip_prefix(KEY_PREFIX)
            .unwrap_or(value.trim());
        let key = decode_key(value).ok_or(MailCryptoError::InvalidKey)?;
        if key.iter().all(|byte| *byte == 0) {
            return Err(MailCryptoError::InvalidKey);
        }
        Ok(Self {
            key: Arc::new(Zeroizing::new(key)),
        })
    }

    pub(super) fn encrypt(
        &self,
        field: &str,
        aad: &[u8],
        plaintext: &str,
    ) -> Result<String, MailCryptoError> {
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err(MailCryptoError::ValueTooLarge);
        }
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| MailCryptoError::RandomnessUnavailable)?;
        let cipher = XChaCha20Poly1305::new((&**self.key).into());
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad,
                },
            )
            .map_err(|_| MailCryptoError::EncryptionFailed)?;
        Ok(format!(
            "v1.{}.{}",
            URL_SAFE_NO_PAD.encode(nonce),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ))
    }

    pub(super) fn decrypt(&self, envelope: &str, aad: &[u8]) -> Result<String, MailCryptoError> {
        let mut parts = envelope.split('.');
        if parts.next() != Some("v1") {
            return Err(MailCryptoError::InvalidEnvelope);
        }
        let nonce = parts.next().ok_or(MailCryptoError::InvalidEnvelope)?;
        let ciphertext = parts.next().ok_or(MailCryptoError::InvalidEnvelope)?;
        if parts.next().is_some() {
            return Err(MailCryptoError::InvalidEnvelope);
        }
        let nonce: [u8; NONCE_BYTES] = URL_SAFE_NO_PAD
            .decode(nonce)
            .map_err(|_| MailCryptoError::InvalidEnvelope)?
            .try_into()
            .map_err(|_| MailCryptoError::InvalidEnvelope)?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(ciphertext)
            .map_err(|_| MailCryptoError::InvalidEnvelope)?;
        if ciphertext.len() < 16 || ciphertext.len() > MAX_PLAINTEXT_BYTES + 16 {
            return Err(MailCryptoError::InvalidEnvelope);
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
                .map_err(|_| MailCryptoError::DecryptionFailed)?,
        );
        std::str::from_utf8(&plaintext)
            .map(str::to_owned)
            .map_err(|_| MailCryptoError::InvalidUtf8)
    }

    pub(super) fn blind_index(&self, field: &str, value: &str) -> String {
        let normalized = Zeroizing::new(
            value
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
        );
        let mut message = Zeroizing::new(Vec::with_capacity(
            BLIND_INDEX_DOMAIN.len() + field.len() + normalized.len() + 1,
        ));
        message.extend_from_slice(BLIND_INDEX_DOMAIN);
        message.extend_from_slice(field.as_bytes());
        message.push(0);
        message.extend_from_slice(normalized.as_bytes());
        encode_hex(&hmac_sha256(&self.key[..], &message))
    }

    pub(super) fn key_fingerprint(&self) -> String {
        format!(
            "{FINGERPRINT_PREFIX}{}",
            encode_hex(&hmac_sha256(&self.key[..], FINGERPRINT_DOMAIN))
        )
    }
}

fn hmac_sha256(key: &[u8], value: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut hmac = <HmacSha256 as Mac>::new_from_slice(key)
        .expect("HMAC-SHA256 accepts the fixed mail key length");
    hmac.update(value);
    let mut tag = hmac.finalize().into_bytes();
    let mut output = Zeroizing::new([0_u8; 32]);
    output.copy_from_slice(&tag);
    tag[..].zeroize();
    output
}

fn decode_key(value: &str) -> Option<[u8; KEY_BYTES]> {
    if value.len() == KEY_BYTES * 2 {
        let mut key = [0_u8; KEY_BYTES];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            key[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
        }
        return Some(key);
    }
    let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
    decoded.try_into().ok()
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
    use super::MailCrypto;

    #[test]
    fn encrypted_mail_is_aad_bound_and_key_is_not_debugged() {
        let crypto = MailCrypto::from_env().expect("test key");
        let encrypted = crypto
            .encrypt("subject", b"mail:item-1:subject", "private subject")
            .expect("encrypt");
        assert_eq!(
            crypto
                .decrypt(&encrypted, b"mail:item-1:subject")
                .expect("decrypt"),
            "private subject"
        );
        assert!(crypto.decrypt(&encrypted, b"mail:item-2:subject").is_err());
        assert!(!format!("{crypto:?}").contains("5b4b"));
        assert_eq!(
            crypto.blind_index("email", "User@Example.com"),
            crypto.blind_index("email", " user@example.COM ")
        );
    }
}
