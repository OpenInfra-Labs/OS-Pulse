use axum::{
    Json,
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect},
};
use rusqlite::{OptionalExtension, params};

use crate::auth::*;
use crate::collectors::collect_metrics_snapshot;
use crate::db::persist_snapshot;
use crate::models::*;
use crate::{json_error, now_ts, now_ts_ms};

pub(crate) async fn root_redirect(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user = resolve_session_from_headers(&state, &headers);
    if user.is_some() {
        Redirect::to("/dashboard").into_response()
    } else {
        Redirect::to("/login").into_response()
    }
}

pub(crate) async fn login_page() -> impl IntoResponse {
    Html(include_str!("../assets/login.html"))
}

pub(crate) async fn dashboard_page() -> impl IntoResponse {
    Html(include_str!("../assets/dashboard.html"))
}

pub(crate) async fn styles_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../assets/styles.css"),
    )
}

pub(crate) async fn api_auth_state(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let needs_setup = state_needs_setup(&state);
    let logged_in = resolve_session_from_headers(&state, &headers).is_some();
    Json(AuthStateResponse {
        needs_setup,
        logged_in,
    })
}

pub(crate) async fn api_auth_setup(
    State(state): State<AppState>,
    Json(payload): Json<AuthPayload>,
) -> impl IntoResponse {
    if payload.username.trim().is_empty() || payload.password.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "Username and password are required");
    }
    if !state_needs_setup(&state) {
        return json_error(StatusCode::CONFLICT, "Account already initialized");
    }

    let password_hash = match hash_password(&payload.password) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Hashing failed"),
    };

    let user_id = {
        let db = state.db.lock().expect("db lock");
        let inserted = db.execute(
            "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)",
            params![payload.username.trim(), password_hash, now_ts()],
        );
        if inserted.is_err() {
            return json_error(StatusCode::CONFLICT, "Username already exists");
        }
        db.last_insert_rowid()
    };

    let token = create_session_token(&state, user_id);
    if let Some(token) = token {
        return with_auth_cookie(Json(serde_json::json!({ "ok": true })), &token).into_response();
    }
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unable to create session",
    )
}

pub(crate) async fn api_auth_login(
    State(state): State<AppState>,
    Json(payload): Json<AuthPayload>,
) -> impl IntoResponse {
    if payload.username.trim().is_empty() || payload.password.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "Username and password are required");
    }

    let user = {
        let db = state.db.lock().expect("db lock");
        db.query_row(
            "SELECT id, password_hash FROM users WHERE username = ?1",
            params![payload.username.trim()],
            |row| {
                let id: i64 = row.get(0)?;
                let hash: String = row.get(1)?;
                Ok((id, hash))
            },
        )
        .optional()
        .ok()
        .flatten()
    };

    let Some((user_id, hash)) = user else {
        return json_error(StatusCode::UNAUTHORIZED, "Invalid credentials");
    };

    if !verify_password(&payload.password, &hash) {
        return json_error(StatusCode::UNAUTHORIZED, "Invalid credentials");
    }

    let token = create_session_token(&state, user_id);
    if let Some(token) = token {
        return with_auth_cookie(Json(serde_json::json!({ "ok": true })), &token).into_response();
    }
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unable to create session",
    )
}

pub(crate) async fn api_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = extract_cookie(&headers, AUTH_COOKIE_NAME) {
        let db = state.db.lock().expect("db lock");
        let _ = db.execute("DELETE FROM sessions WHERE token = ?1", params![token]);
    }

    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&clear_auth_cookie()).expect("valid clear cookie"),
    );
    response
}

pub(crate) async fn api_me(
    axum::extract::Extension(session): axum::extract::Extension<AuthSession>,
) -> impl IntoResponse {
    Json(MeResponse {
        user_id: session.user_id,
        username: session.username,
    })
}

pub(crate) async fn api_action() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

pub(crate) async fn api_metrics(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(latest) = state.latest.lock().expect("latest lock").clone() {
        return Json(latest);
    }

    let snapshot = collect_metrics_snapshot(&state).await;
    persist_snapshot(&state, &snapshot);
    *state.latest.lock().expect("latest lock") = Some(snapshot.clone());
    Json(snapshot)
}

pub(crate) async fn api_trends(
    State(state): State<AppState>,
    Query(query): Query<TrendQuery>,
) -> impl IntoResponse {
    let requested = query.minutes.unwrap_or(15).clamp(5, 7 * 24 * 60);
    let minutes = normalize_window_minutes(requested);
    let now_ms = now_ts_ms();
    let from_ms = now_ms - (minutes as i64 * 60 * 1000);

    let db = state.db.lock().expect("db lock");
    let mut stmt = match db.prepare(
        "
        SELECT bucket_end_ms, cpu_sum, memory_sum, disk_iops_sum,
               network_sum, network_rx_sum, network_tx_sum, container_sum, samples
        FROM system_metrics_agg
        WHERE window_minutes = ?1 AND bucket_end_ms >= ?2
        ORDER BY bucket_end_ms ASC
        LIMIT ?3
        ",
    ) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Query failed"),
    };

    let rows = match stmt.query_map(
        params![minutes as i64, from_ms, MAX_TREND_POINTS as i64],
        |row| {
            let bucket_end_ms: i64 = row.get(0)?;
            let cpu_sum: f64 = row.get(1)?;
            let memory_sum: f64 = row.get(2)?;
            let disk_iops_sum: f64 = row.get(3)?;
            let network_sum: f64 = row.get(4)?;
            let network_rx_sum: f64 = row.get(5)?;
            let network_tx_sum: f64 = row.get(6)?;
            let container_sum: f64 = row.get(7)?;
            let samples: i64 = row.get(8)?;
            let c = samples.max(1) as f64;
            let avg_net = (network_sum / c) as u64;
            let (rx_avg, tx_avg) = if network_rx_sum == 0.0 && network_tx_sum == 0.0 {
                estimate_network_split(&state, avg_net)
            } else {
                ((network_rx_sum / c) as u64, (network_tx_sum / c) as u64)
            };
            Ok(TrendPoint {
                ts: bucket_end_ms / 1000,
                cpu_percent: (cpu_sum / c) as f32,
                memory_percent: (memory_sum / c) as f32,
                disk_iops: disk_iops_sum / c,
                network_rx_bytes: rx_avg,
                network_tx_bytes: tx_avg,
                container_count: (container_sum / c).round() as usize,
            })
        },
    ) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Query failed"),
    };

    let mut points = Vec::new();
    for row in rows {
        if let Ok(point) = row {
            points.push(point);
        }
    }

    let returned_points = points.len();
    let available_minutes = if returned_points >= 2 {
        let first_ts = points.first().map(|p| p.ts).unwrap_or(0);
        let last_ts = points.last().map(|p| p.ts).unwrap_or(0);
        ((last_ts - first_ts).max(0) / 60) as u32
    } else {
        0
    };
    let sampled = false;

    Json(TrendResponse {
        points,
        requested_minutes: minutes,
        available_minutes,
        returned_points,
        sampled,
    })
    .into_response()
}

pub(crate) async fn api_container_trends(
    State(state): State<AppState>,
    Query(query): Query<ContainerTrendQuery>,
) -> impl IntoResponse {
    let minutes = query.minutes.unwrap_or(60).clamp(5, 7 * 24 * 60);
    let from_ts = now_ts() - (minutes as i64 * 60);

    let db = state.db.lock().expect("db lock");

    let mut list_stmt = match db.prepare(
        "
        SELECT DISTINCT name
        FROM container_metrics_history
        WHERE ts >= ?1
        ORDER BY name ASC
        ",
    ) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Query failed"),
    };

    let list_rows = match list_stmt.query_map(params![from_ts], |row| row.get::<_, String>(0)) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Query failed"),
    };

    let mut available = Vec::new();
    for row in list_rows {
        if let Ok(name) = row {
            available.push(name);
        }
    }

    let selected = query
        .name
        .as_ref()
        .filter(|name| available.iter().any(|item| item == *name))
        .cloned()
        .or_else(|| available.first().cloned());

    let Some(selected_name) = selected.clone() else {
        return Json(ContainerTrendResponse {
            selected: None,
            available,
            points: Vec::new(),
        })
        .into_response();
    };

    let mut stmt = match db.prepare(
        "
        SELECT ts, cpu_percent, memory_used_bytes, memory_limit_bytes,
               network_rx_bytes, network_tx_bytes, disk_read_bytes, disk_write_bytes
        FROM container_metrics_history
        WHERE ts >= ?1 AND name = ?2
        ORDER BY ts ASC
        ",
    ) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Query failed"),
    };

    let rows = match stmt.query_map(params![from_ts, selected_name], |row| {
        let network_rx: i64 = row.get(4)?;
        let network_tx: i64 = row.get(5)?;
        let disk_read: i64 = row.get(6)?;
        let disk_write: i64 = row.get(7)?;
        Ok(ContainerTrendPoint {
            ts: row.get(0)?,
            cpu_percent: row.get(1)?,
            memory_used_bytes: row.get::<_, i64>(2)? as u64,
            memory_limit_bytes: row.get::<_, i64>(3)? as u64,
            network_total_bytes: (network_rx + network_tx).max(0) as u64,
            disk_io_total_bytes: (disk_read + disk_write).max(0) as u64,
        })
    }) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Query failed"),
    };

    let mut points = Vec::new();
    for row in rows {
        if let Ok(point) = row {
            points.push(point);
        }
    }

    points = downsample_container_trend_points(points, MAX_TREND_POINTS);

    Json(ContainerTrendResponse {
        selected,
        available,
        points,
    })
    .into_response()
}

fn estimate_network_split(state: &AppState, total: u64) -> (u64, u64) {
    if let Some(latest) = state.latest.lock().expect("latest lock").clone() {
        let rx = latest.system.network_rx_bytes;
        let tx = latest.system.network_tx_bytes;
        let sum = rx.saturating_add(tx);
        if sum > 0 {
            let rx_ratio = rx as f64 / sum as f64;
            let rx_est = (total as f64 * rx_ratio).round() as u64;
            let tx_est = total.saturating_sub(rx_est);
            return (rx_est, tx_est);
        }
    }
    (total, 0)
}

fn normalize_window_minutes(requested: u32) -> u32 {
    if requested <= 15 {
        15
    } else if requested <= 60 {
        60
    } else if requested <= 360 {
        360
    } else {
        1440
    }
}

fn downsample_container_trend_points(
    points: Vec<ContainerTrendPoint>,
    max_points: usize,
) -> Vec<ContainerTrendPoint> {
    if points.len() <= max_points || max_points == 0 {
        return points;
    }

    let first_ts = points.first().map(|p| p.ts).unwrap_or(0);
    let last_ts = points.last().map(|p| p.ts).unwrap_or(first_ts);
    if first_ts >= last_ts {
        return points.into_iter().take(max_points).collect();
    }

    let span = (last_ts - first_ts + 1) as i128;
    let bucket_count = max_points;

    #[derive(Clone, Copy)]
    struct ContainerAgg {
        ts: i64,
        cpu: f64,
        mem_used: f64,
        mem_limit: f64,
        net: f64,
        disk: f64,
        count: u32,
    }

    let mut buckets: Vec<Option<ContainerAgg>> = vec![None; bucket_count];
    for point in points {
        let offset = (point.ts - first_ts) as i128;
        let mut idx = ((offset * bucket_count as i128) / span) as usize;
        if idx >= bucket_count {
            idx = bucket_count - 1;
        }

        match &mut buckets[idx] {
            Some(agg) => {
                agg.ts = point.ts;
                agg.cpu += point.cpu_percent;
                agg.mem_used += point.memory_used_bytes as f64;
                agg.mem_limit += point.memory_limit_bytes as f64;
                agg.net += point.network_total_bytes as f64;
                agg.disk += point.disk_io_total_bytes as f64;
                agg.count += 1;
            }
            None => {
                buckets[idx] = Some(ContainerAgg {
                    ts: point.ts,
                    cpu: point.cpu_percent,
                    mem_used: point.memory_used_bytes as f64,
                    mem_limit: point.memory_limit_bytes as f64,
                    net: point.network_total_bytes as f64,
                    disk: point.disk_io_total_bytes as f64,
                    count: 1,
                });
            }
        }
    }

    buckets
        .into_iter()
        .flatten()
        .map(|agg| {
            let c = agg.count.max(1) as f64;
            ContainerTrendPoint {
                ts: agg.ts,
                cpu_percent: agg.cpu / c,
                memory_used_bytes: (agg.mem_used / c) as u64,
                memory_limit_bytes: (agg.mem_limit / c) as u64,
                network_total_bytes: (agg.net / c) as u64,
                disk_io_total_bytes: (agg.disk / c) as u64,
            }
        })
        .collect()
}
