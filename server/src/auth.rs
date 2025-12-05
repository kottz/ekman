use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::http::{HeaderMap, HeaderValue, header};
use base32::{Alphabet, encode as b32_encode};
use chrono::{DateTime, Duration, Utc};
use totp_rs::{Algorithm, Secret, TOTP};
use turso::Connection;

use crate::{Error, Result, db};

const COOKIE_NAME: &str = "ekman_session";
const SESSION_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub totp_secret: String,
    pub totp_enabled: bool,
}

/// Extract user from session cookie.
pub async fn user_from_headers(conn: &mut Connection, headers: &HeaderMap) -> Result<AuthUser> {
    let token = extract_token(headers).ok_or(Error::Unauthorized)?;

    let mut stmt = conn
        .prepare("SELECT user_id, expires_at FROM sessions WHERE token = ?")
        .await?;

    let row = stmt
        .query_row([token.as_str()])
        .await
        .map_err(|_| Error::Unauthorized)?;
    let user_id: i64 = row.get(0)?;
    let expires: String = row.get(1)?;
    let expires_at = db::parse_timestamp(&expires)?;

    if expires_at < db::now() {
        conn.execute("DELETE FROM sessions WHERE token = ?", [token.as_str()])
            .await?;
        return Err(Error::Unauthorized);
    }

    user_by_id(conn, user_id).await
}

pub async fn user_by_username(conn: &mut Connection, username: &str) -> Result<AuthUser> {
    let mut stmt = conn
        .prepare("SELECT id, username, password_hash, totp_secret, totp_enabled FROM users WHERE username = ?")
        .await?;

    let row = stmt
        .query_row([username])
        .await
        .map_err(|_| Error::Unauthorized)?;
    Ok(AuthUser {
        id: row.get(0)?,
        username: row.get(1)?,
        password_hash: row.get(2)?,
        totp_secret: row.get(3)?,
        totp_enabled: row.get::<bool>(4)?,
    })
}

pub async fn user_by_id(conn: &mut Connection, id: i64) -> Result<AuthUser> {
    let mut stmt = conn
        .prepare(
            "SELECT id, username, password_hash, totp_secret, totp_enabled FROM users WHERE id = ?",
        )
        .await?;

    let row = stmt.query_row([id]).await?;
    Ok(AuthUser {
        id: row.get(0)?,
        username: row.get(1)?,
        password_hash: row.get(2)?,
        totp_secret: row.get(3)?,
        totp_enabled: row.get::<bool>(4)?,
    })
}

pub async fn create_session(
    conn: &mut Connection,
    user_id: i64,
) -> Result<(String, DateTime<Utc>)> {
    let token = generate_token();
    let expires_at = db::now() + Duration::days(SESSION_DAYS);

    conn.execute(
        "INSERT INTO sessions (user_id, token, expires_at) VALUES (?, ?, ?)",
        (user_id, token.as_str(), db::timestamp(expires_at)),
    )
    .await?;

    Ok((token, expires_at))
}

pub async fn delete_session(conn: &mut Connection, token: &str) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE token = ?", [token])
        .await?;
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| Error::Internal(format!("hash error: {e}")))
}

pub fn verify_password(password: &str, hash: &str) -> Result<()> {
    let parsed =
        PasswordHash::new(hash).map_err(|e| Error::Internal(format!("invalid hash: {e}")))?;

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| Error::Unauthorized)
}

pub fn verify_totp(secret: &str, code: &str) -> Result<()> {
    let bytes = Secret::Encoded(secret.into())
        .to_bytes()
        .map_err(|e| Error::Internal(format!("invalid totp secret: {e}")))?;

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("ekman".into()),
        "ekman".into(),
    )
    .map_err(|e| Error::Internal(format!("totp error: {e}")))?;

    let valid = totp
        .check_current(code)
        .map_err(|e| Error::Internal(format!("totp check error: {e}")))?;

    if valid {
        Ok(())
    } else {
        Err(Error::Unauthorized)
    }
}

pub fn generate_totp_secret(username: &str) -> Result<(String, String)> {
    let mut bytes = [0u8; 20];
    use argon2::password_hash::rand_core::RngCore;
    OsRng.fill_bytes(&mut bytes);

    let secret = b32_encode(Alphabet::Rfc4648 { padding: false }, &bytes);

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes.to_vec(),
        Some("ekman".into()),
        username.into(),
    )
    .map_err(|e| Error::Internal(format!("totp error: {e}")))?;

    Ok((secret, totp.get_url()))
}

pub fn session_cookie(token: &str, expires_at: DateTime<Utc>) -> HeaderValue {
    let max_age = expires_at
        .signed_duration_since(db::now())
        .num_seconds()
        .max(0);
    let value = format!("{COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}");
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

pub fn clear_cookie() -> HeaderValue {
    let value = format!("{COOKIE_NAME}=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax");
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn extract_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let mut iter = part.trim().splitn(2, '=');
        if let (Some(name), Some(value)) = (iter.next(), iter.next())
            && name == COOKIE_NAME
        {
            return Some(value.into());
        }
    }
    None
}

fn generate_token() -> String {
    use argon2::password_hash::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

mod hex {
    pub fn encode(bytes: [u8; 32]) -> String {
        let mut s = String::with_capacity(64);
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}
