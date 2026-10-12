//! 加密适配器：飞书密钥 AES-256-GCM 加密、管理员口令 Argon2id 哈希。

use std::path::Path;

use aes_gcm::aead::{Aead, AeadCore, OsRng};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use alkh_domain::error::{PortError, PortResult};
use alkh_domain::ports::{PasswordHasher, SecretCipher};
use alkh_domain::settings::EncryptedSecret;
use argon2::password_hash::rand_core::RngCore;
use argon2::password_hash::{PasswordHash, SaltString};
// 注意：`argon2::PasswordHasher` 与本项目端口 trait `alkh_domain::ports::PasswordHasher` 同名，
// 这里用别名导入，避免歧义。
use argon2::{Argon2, PasswordHasher as Argon2PasswordHasher, PasswordVerifier};

/// AES-256-GCM 密钥加密器。主密钥来自 `ALKH_MASTER_KEY` 或数据目录下的 `master.key`。
pub struct AesGcmCipher {
    key: [u8; 32],
}

impl AesGcmCipher {
    pub fn from_bytes(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// 解析主密钥：接受 64 位 hex 或 base64（解码后 32 字节）。
    pub fn parse_master_key(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim();
        if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
            let bytes = hex::decode(raw)?;
            return Self::from_slice(&bytes);
        }
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw))?;
        Self::from_slice(&bytes)
    }

    fn from_slice(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes.len() != 32 {
            anyhow::bail!("主密钥必须为 32 字节（当前 {} 字节）", bytes.len());
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(bytes);
        Ok(Self { key })
    }

    /// 生成随机主密钥并以 0600 权限写入 `path`（首次启动时使用）。
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            return Self::parse_master_key(&raw);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let encoded = hex::encode(key);
        std::fs::write(path, &encoded)?;
        restrict_permissions(path)?;
        tracing::warn!(
            path = %path.display(),
            "未提供 ALKH_MASTER_KEY，已自动生成主密钥文件，请妥善备份（丢失将无法解密已存储的飞书 app_secret）"
        );
        Ok(Self::from_bytes(key))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.key).expect("32 字节密钥始终合法")
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

impl SecretCipher for AesGcmCipher {
    fn encrypt(&self, plaintext: &str) -> PortResult<EncryptedSecret> {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher()
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| PortError::Internal(format!("加密失败: {e}")))?;
        Ok(EncryptedSecret {
            nonce: engine.encode(nonce),
            ciphertext: engine.encode(ciphertext),
        })
    }

    fn decrypt(&self, secret: &EncryptedSecret) -> PortResult<String> {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD;
        let nonce = engine
            .decode(&secret.nonce)
            .map_err(|e| PortError::Internal(format!("nonce 解码失败: {e}")))?;
        if nonce.len() != 12 {
            return Err(PortError::Internal("nonce 长度必须为 12 字节".into()));
        }
        let ciphertext = engine
            .decode(&secret.ciphertext)
            .map_err(|e| PortError::Internal(format!("密文解码失败: {e}")))?;
        let plaintext = self
            .cipher()
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| PortError::Internal("解密失败：主密钥不匹配或数据已损坏".into()))?;
        String::from_utf8(plaintext)
            .map_err(|e| PortError::Internal(format!("明文不是合法 UTF-8: {e}")))
    }
}

/// Argon2id 口令哈希。
#[derive(Default)]
pub struct Argon2Hasher;

impl PasswordHasher for Argon2Hasher {
    fn hash(&self, password: &str) -> PortResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| PortError::Internal(format!("口令哈希失败: {e}")))
    }

    fn verify(&self, password: &str, phc: &str) -> PortResult<bool> {
        let parsed = match PasswordHash::new(phc) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "数据库中存的口令哈希无法解析");
                return Ok(false);
            }
        };
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> AesGcmCipher {
        AesGcmCipher::from_bytes([7u8; 32])
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let c = cipher();
        let secret = c.encrypt("super-secret-value").unwrap();
        assert_ne!(secret.ciphertext, "super-secret-value");
        assert_eq!(c.decrypt(&secret).unwrap(), "super-secret-value");
    }

    #[test]
    fn ciphertext_differs_each_time_because_of_random_nonce() {
        let c = cipher();
        let a = c.encrypt("same").unwrap();
        let b = c.encrypt("same").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let secret = cipher().encrypt("x").unwrap();
        let other = AesGcmCipher::from_bytes([9u8; 32]);
        assert!(other.decrypt(&secret).is_err());
    }

    #[test]
    fn master_key_parsing_accepts_hex_and_base64() {
        let raw = [3u8; 32];
        let hexform = hex::encode(raw);
        assert!(AesGcmCipher::parse_master_key(&hexform).is_ok());

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        assert!(AesGcmCipher::parse_master_key(&b64).is_ok());

        assert!(AesGcmCipher::parse_master_key("short").is_err());
    }

    #[test]
    fn password_hash_verifies() {
        let h = Argon2Hasher;
        let phc = h.hash("correct horse battery").unwrap();
        assert!(phc.starts_with("$argon2"));
        assert!(h.verify("correct horse battery", &phc).unwrap());
        assert!(!h.verify("wrong", &phc).unwrap());
        assert!(!h.verify("x", "not-a-phc").unwrap());
    }

    #[test]
    fn hashes_are_salted() {
        let h = Argon2Hasher;
        assert_ne!(h.hash("same").unwrap(), h.hash("same").unwrap());
    }
}
