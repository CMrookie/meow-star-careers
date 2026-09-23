//! 安全原语：密码哈希（argon2id）、令牌生成与摘要。集中在此便于审计与更换算法。

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use sha2::{Digest, Sha256};

/// 使用 argon2id 对明文密码哈希（返回 PHC 字符串，存库）
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// 校验明文密码是否匹配已存哈希
pub fn verify_password(password: &str, password_hash: &str) -> bool {
    PasswordHash::new(password_hash)
        .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

/// 生成随机会话令牌（32 位十六进制）并返回其 sha256 摘要（摘要才入库）
pub fn new_session_token() -> (String, String) {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let digest = sha256_hex(token.as_bytes());
    (token, digest)
}

/// 中国大陆手机号：11 位、1 开头、全数字。
/// 注册 / 登录 / 管理员引导（ADMIN_PHONE）共用同一判据，避免各处口径不一致。
pub fn is_valid_cn_phone(phone: &str) -> bool {
    phone.len() == 11 && phone.starts_with('1') && phone.bytes().all(|b| b.is_ascii_digit())
}

/// sha256 十六进制摘要
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
