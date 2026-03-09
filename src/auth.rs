use std::collections::HashMap;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use rand_core::OsRng;
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::json_error;
use crate::models::*;
use crate::now_ts;

pub(crate) async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let token = extract_cookie(req.headers(), AUTH_COOKIE_NAME);
    let Some(token) = token else {
        return unauth_response(req.uri().path());
    };

    let session = validate_and_extend_session(&state, &token);
    let Some(auth_session) = session else {
        return unauth_response(req.uri().path());
    };

    req.extensions_mut().insert(auth_session);
    let mut response = next.run(req).await;
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&build_auth_cookie(&token)).expect("valid set-cookie"),
    );
    response
}

fn unauth_response(path: &str) -> Response {
    if path.starts_with("/api/") {
        return json_error(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    Redirect::to("/login").into_response()
}

pub(crate) fn state_needs_setup(state: &AppState) -> bool {
    let db = state.db.lock().expect("db lock");
    let users: i64 = db
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap_or(0);
    users == 0
}

pub(crate) fn resolve_session_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<AuthSession> {
    let token = extract_cookie(headers, AUTH_COOKIE_NAME)?;
    validate_and_extend_session(state, &token)
}

fn validate_and_extend_session(state: &AppState, token: &str) -> Option<AuthSession> {
    let now = now_ts();
    let expires_at = now + TOKEN_LIFETIME_SECS;
    let db = state.db.lock().expect("db lock");

    let session = db
        .query_row(
            "
            SELECT users.id, users.username, sessions.expires_at
            FROM sessions
            JOIN users ON users.id = sessions.user_id
            WHERE sessions.token = ?1
            ",
            params![token],
            |row| {
                let user_id: i64 = row.get(0)?;
                let username: String = row.get(1)?;
                let session_expiry: i64 = row.get(2)?;
                Ok((user_id, username, session_expiry))
            },
        )
        .optional()
        .ok()
        .flatten()?;

    if session.2 < now {
        let _ = db.execute("DELETE FROM sessions WHERE token = ?1", params![token]);
        return None;
    }

    let _ = db.execute(
        "UPDATE sessions SET expires_at = ?1, last_seen = ?2 WHERE token = ?3",
        params![expires_at, now, token],
    );

    Some(AuthSession {
        user_id: session.0,
        username: session.1,
    })
}

pub(crate) fn create_session_token(state: &AppState, user_id: i64) -> Option<String> {
    let now = now_ts();
    let expires_at = now + TOKEN_LIFETIME_SECS;
    let token = Uuid::new_v4().to_string();
    let db = state.db.lock().expect("db lock");

    let inserted = db.execute(
        "INSERT INTO sessions(token, user_id, expires_at, last_seen) VALUES (?1, ?2, ?3, ?4)",
        params![token, user_id, expires_at, now],
    );
    if inserted.is_ok() {
        Some(token)
    } else {
        None
    }
}

pub(crate) fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let hashed = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| "hash error".to_string())?
        .to_string();
    Ok(hashed)
}

pub(crate) fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(v) => v,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub(crate) fn extract_cookie(headers: &HeaderMap, key: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    let mut map = HashMap::new();
    for part in raw.split(';') {
        let trimmed = part.trim();
        if let Some((k, v)) = trimmed.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map.get(key).cloned()
}

pub(crate) fn build_auth_cookie(token: &str) -> String {
    format!(
        "{}={}; Max-Age={}; Path=/; HttpOnly; SameSite=Lax",
        AUTH_COOKIE_NAME, token, TOKEN_LIFETIME_SECS
    )
}

pub(crate) fn clear_auth_cookie() -> String {
    format!(
        "{}=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax",
        AUTH_COOKIE_NAME
    )
}

pub(crate) fn with_auth_cookie<T: IntoResponse>(body: T, token: &str) -> Response {
    let mut response = body.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&build_auth_cookie(token)).expect("valid set-cookie"),
    );
    response
}
