use std::{
    collections::HashMap,
    net::SocketAddr,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use bollard::{
    container::{InspectContainerOptions, ListContainersOptions, StatsOptions},
    Docker,
};
use futures_util::StreamExt;
use rand_core::OsRng;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sysinfo::{CpuExt, DiskExt, NetworkExt, ProcessExt, System, SystemExt};
use uuid::Uuid;

const TOKEN_LIFETIME_SECS: i64 = 3 * 24 * 60 * 60;
const AUTH_COOKIE_NAME: &str = "osp_token";
const MAX_TREND_POINTS: usize = 96;

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    latest: Arc<Mutex<Option<MetricsResponse>>>,
    network_baseline: Arc<Mutex<Option<NetworkBaseline>>>,
    disk_baseline: Arc<Mutex<Option<DiskBaseline>>>,
    disk_xfrs_baseline: Arc<Mutex<Option<DiskXfrsBaseline>>>,
    #[cfg(target_os = "linux")]
    disk_ops_baseline: Arc<Mutex<Option<DiskOpsBaseline>>>,
    last_cleanup_day: Arc<Mutex<i64>>,
    sample_interval_secs: u64,
}

#[derive(Clone, Copy)]
struct NetworkBaseline {
    ts: i64,
    rx_total: u64,
    tx_total: u64,
}

#[derive(Clone, Copy)]
struct DiskBaseline {
    ts: i64,
    read_total: u64,
    write_total: u64,
}

#[derive(Clone, Copy)]
struct DiskXfrsBaseline {
    ts: i64,
    xfrs_total: u64,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
struct DiskOpsBaseline {
    ts: i64,
    ops_total: u64,
}

#[derive(Clone)]
struct AuthSession {
    user_id: i64,
    username: String,
}

#[derive(Deserialize)]
struct AuthPayload {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct AuthStateResponse {
    needs_setup: bool,
    logged_in: bool,
}

#[derive(Serialize)]
struct MeResponse {
    user_id: i64,
    username: String,
}

#[derive(Serialize, Clone)]
struct SystemMetrics {
    cpu_percent: f32,
    memory_total_bytes: u64,
    memory_used_bytes: u64,
    memory_percent: f32,
    disk_total_bytes: u64,
    disk_used_bytes: u64,
    disk_percent: f32,
    disk_iops: f64,
    network_rx_bytes: u64,
    network_tx_bytes: u64,
    process_count: usize,
    load_1: f64,
    load_5: f64,
    load_15: f64,
}

#[derive(Serialize, Clone)]
struct ContainerMetrics {
    name: String,
    status: String,
    cpu_percent: f64,
    memory_used_bytes: u64,
    memory_limit_bytes: u64,
    network_rx_bytes: u64,
    network_tx_bytes: u64,
    disk_read_bytes: u64,
    disk_write_bytes: u64,
    image: String,
    tag: String,
    restart_count: i64,
}

#[derive(Serialize, Clone)]
struct MetricsResponse {
    system: SystemMetrics,
    containers: Vec<ContainerMetrics>,
    ts: i64,
}

#[derive(Deserialize)]
struct TrendQuery {
    minutes: Option<u32>,
}

#[derive(Serialize)]
struct TrendPoint {
    ts: i64,
    cpu_percent: f32,
    memory_percent: f32,
    disk_iops: f64,
    network_rx_bytes: u64,
    network_tx_bytes: u64,
    container_count: usize,
}

#[derive(Serialize)]
struct TrendResponse {
    points: Vec<TrendPoint>,
    requested_minutes: u32,
    available_minutes: u32,
    returned_points: usize,
    sampled: bool,
}

#[derive(Deserialize)]
struct ContainerTrendQuery {
    minutes: Option<u32>,
    name: Option<String>,
}

#[derive(Serialize)]
struct ContainerTrendPoint {
    ts: i64,
    cpu_percent: f64,
    memory_used_bytes: u64,
    memory_limit_bytes: u64,
    network_total_bytes: u64,
    disk_io_total_bytes: u64,
}

#[derive(Serialize)]
struct ContainerTrendResponse {
    selected: Option<String>,
    available: Vec<String>,
    points: Vec<ContainerTrendPoint>,
}

#[tokio::main]
async fn main() {
    let connection = Connection::open("os_pulse.db").expect("open sqlite database");
    init_db(&connection).expect("init database");
    rebuild_recent_system_aggregates(&connection).expect("rebuild recent aggregates");

    let sample_interval_secs = std::env::var("OSP_INTERVAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1);

    let state = AppState {
        db: Arc::new(Mutex::new(connection)),
        latest: Arc::new(Mutex::new(None)),
        network_baseline: Arc::new(Mutex::new(None)),
        disk_baseline: Arc::new(Mutex::new(None)),
        disk_xfrs_baseline: Arc::new(Mutex::new(None)),
        #[cfg(target_os = "linux")]
        disk_ops_baseline: Arc::new(Mutex::new(None)),
        last_cleanup_day: Arc::new(Mutex::new(-1)),
        sample_interval_secs,
    };

    tokio::spawn(background_sampler(state.clone()));

    let protected = Router::new()
        .route("/dashboard", get(dashboard_page))
        .route("/api/me", get(api_me))
        .route("/api/metrics", get(api_metrics))
        .route("/api/trends", get(api_trends))
        .route("/api/trends/containers", get(api_container_trends))
        .route("/api/action", post(api_action))
        .route("/api/auth/logout", post(api_logout))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let app = Router::new()
        .route("/", get(root_redirect))
        .route("/login", get(login_page))
        .route("/styles.css", get(styles_css))
        .route("/api/auth/state", get(api_auth_state))
        .route("/api/auth/setup", post(api_auth_setup))
        .route("/api/auth/login", post(api_auth_login))
        .merge(protected)
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:3000".parse().expect("valid socket address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    println!("OS-Pulse running at http://{}", addr);
    axum::serve(listener, app).await.expect("start server");
}

fn ensure_system_metrics_agg_schema(conn: &Connection) -> rusqlite::Result<()> {
    let mut has_legacy_disk_io_sum = false;
    let mut stmt = conn.prepare("PRAGMA table_info(system_metrics_agg)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "disk_io_sum" {
            has_legacy_disk_io_sum = true;
            break;
        }
    }

    if !has_legacy_disk_io_sum {
        return Ok(());
    }

    conn.execute_batch(
        "
        ALTER TABLE system_metrics_agg RENAME TO system_metrics_agg_legacy;

        CREATE TABLE system_metrics_agg (
            window_minutes INTEGER NOT NULL,
            bucket_start_ms INTEGER NOT NULL,
            bucket_end_ms INTEGER NOT NULL,
            cpu_sum REAL NOT NULL,
            memory_sum REAL NOT NULL,
            disk_iops_sum REAL NOT NULL,
            network_sum REAL NOT NULL,
            network_rx_sum REAL NOT NULL DEFAULT 0,
            network_tx_sum REAL NOT NULL DEFAULT 0,
            container_sum REAL NOT NULL,
            samples INTEGER NOT NULL,
            PRIMARY KEY(window_minutes, bucket_start_ms)
        );

        INSERT INTO system_metrics_agg(
            window_minutes, bucket_start_ms, bucket_end_ms,
            cpu_sum, memory_sum, disk_iops_sum,
            network_sum, network_rx_sum, network_tx_sum,
            container_sum, samples
        )
        SELECT
            window_minutes, bucket_start_ms, bucket_end_ms,
            cpu_sum, memory_sum, COALESCE(disk_iops_sum, 0),
            network_sum, COALESCE(network_rx_sum, 0), COALESCE(network_tx_sum, 0),
            container_sum, samples
        FROM system_metrics_agg_legacy;

        DROP TABLE system_metrics_agg_legacy;
        CREATE INDEX IF NOT EXISTS idx_system_agg_window_end ON system_metrics_agg(window_minutes, bucket_end_ms);
        ",
    )?;

    Ok(())
}

fn rebuild_recent_system_aggregates(conn: &Connection) -> rusqlite::Result<()> {
    let now_sec = now_ts();
    let now_ms = now_ts_ms();
    let windows = [15_u32, 60_u32, 360_u32, 1440_u32];

    for minutes in windows {
        let from_ts = now_sec - (minutes as i64 * 60);
        let bucket_ms = (minutes as i64 * 60 * 1000) / MAX_TREND_POINTS as i64;

        let mut stmt = conn.prepare(
            "
            SELECT ts, cpu_percent, memory_percent, disk_iops,
                   network_total_bytes, network_rx_bytes, network_tx_bytes, container_count
            FROM system_metrics_history
            WHERE ts >= ?1
            ORDER BY ts ASC
            ",
        )?;

        let rows = stmt.query_map(params![from_ts], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i64>(4)? as f64,
                row.get::<_, i64>(5).unwrap_or(0) as f64,
                row.get::<_, i64>(6).unwrap_or(0) as f64,
                row.get::<_, i64>(7)? as f64,
            ))
        })?;

        #[derive(Default)]
        struct AggRow {
            bucket_end_ms: i64,
            cpu_sum: f64,
            memory_sum: f64,
            disk_iops_sum: f64,
            network_sum: f64,
            network_rx_sum: f64,
            network_tx_sum: f64,
            container_sum: f64,
            samples: i64,
        }

        let mut bucketed: HashMap<i64, AggRow> = HashMap::new();
        for row in rows {
            let (ts, cpu, mem, disk_iops, net_total, net_rx, net_tx, containers) = row?;
            let sample_ms = ts * 1000;
            let bucket_start_ms = sample_ms - (sample_ms % bucket_ms);
            let bucket_end_ms = bucket_start_ms + bucket_ms;
            let agg = bucketed.entry(bucket_start_ms).or_default();
            agg.bucket_end_ms = bucket_end_ms;
            agg.cpu_sum += cpu;
            agg.memory_sum += mem;
            agg.disk_iops_sum += disk_iops;
            agg.network_sum += net_total;
            agg.network_rx_sum += net_rx;
            agg.network_tx_sum += net_tx;
            agg.container_sum += containers;
            agg.samples += 1;
        }

        let keep_from_ms = now_ms - (minutes as i64 * 60 * 1000);
        conn.execute(
            "DELETE FROM system_metrics_agg WHERE window_minutes = ?1 AND bucket_end_ms >= ?2",
            params![minutes as i64, keep_from_ms],
        )?;

        for (bucket_start_ms, agg) in bucketed {
            conn.execute(
                "
                INSERT OR REPLACE INTO system_metrics_agg(
                    window_minutes, bucket_start_ms, bucket_end_ms,
                    cpu_sum, memory_sum, disk_iops_sum,
                    network_sum, network_rx_sum, network_tx_sum,
                    container_sum, samples
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ",
                params![
                    minutes as i64,
                    bucket_start_ms,
                    agg.bucket_end_ms,
                    agg.cpu_sum,
                    agg.memory_sum,
                    agg.disk_iops_sum,
                    agg.network_sum,
                    agg.network_rx_sum,
                    agg.network_tx_sum,
                    agg.container_sum,
                    agg.samples,
                ],
            )?;
        }
    }

    Ok(())
}

async fn root_redirect(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user = resolve_session_from_headers(&state, &headers);
    if user.is_some() {
        Redirect::to("/dashboard").into_response()
    } else {
        Redirect::to("/login").into_response()
    }
}

async fn login_page() -> impl IntoResponse {
    Html(include_str!("../assets/login.html"))
}

async fn dashboard_page() -> impl IntoResponse {
    Html(include_str!("../assets/dashboard.html"))
}

async fn styles_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../assets/styles.css"),
    )
}

async fn api_auth_state(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let needs_setup = state_needs_setup(&state);
    let logged_in = resolve_session_from_headers(&state, &headers).is_some();
    Json(AuthStateResponse {
        needs_setup,
        logged_in,
    })
}

async fn api_auth_setup(
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
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "Unable to create session")
}

async fn api_auth_login(
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
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "Unable to create session")
}

async fn api_logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
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

async fn api_me(
    axum::extract::Extension(session): axum::extract::Extension<AuthSession>,
) -> impl IntoResponse {
    Json(MeResponse {
        user_id: session.user_id,
        username: session.username,
    })
}

async fn api_action() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

async fn api_metrics(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(latest) = state.latest.lock().expect("latest lock").clone() {
        return Json(latest);
    }

    let snapshot = collect_metrics_snapshot(&state).await;
    persist_snapshot(&state, &snapshot);
    *state.latest.lock().expect("latest lock") = Some(snapshot.clone());
    Json(snapshot)
}

async fn api_trends(
    State(state): State<AppState>,
    Query(query): Query<TrendQuery>,
) -> impl IntoResponse {
    let requested = query.minutes.unwrap_or(60).clamp(5, 7 * 24 * 60);
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

    let rows = match stmt.query_map(params![minutes as i64, from_ms, MAX_TREND_POINTS as i64], |row| {
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
        let (rx_sum, tx_sum) = if network_rx_sum == 0.0 && network_tx_sum == 0.0 {
            estimate_network_split(&state, network_sum as u64)
        } else {
            (network_rx_sum as u64, network_tx_sum as u64)
        };
        Ok(TrendPoint {
            ts: bucket_end_ms / 1000,
            cpu_percent: (cpu_sum / c) as f32,
            memory_percent: (memory_sum / c) as f32,
            disk_iops: disk_iops_sum / c,
            network_rx_bytes: rx_sum,
            network_tx_bytes: tx_sum,
            container_count: (container_sum / c).round() as usize,
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

async fn api_container_trends(
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

async fn auth_middleware(
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

fn collect_system_metrics(state: &AppState) -> SystemMetrics {
    let mut system = System::new_all();
    system.refresh_all();

    let memory_total_bytes = normalize_memory_to_bytes(system.total_memory());
    let memory_used_bytes = normalize_memory_to_bytes(system.used_memory());
    let memory_percent = if memory_total_bytes == 0 {
        0.0
    } else {
        (memory_used_bytes as f32 / memory_total_bytes as f32) * 100.0
    };

    let (disk_total_bytes, disk_available_bytes) = select_disk_usage(&system);
    let disk_used_bytes = disk_total_bytes.saturating_sub(disk_available_bytes);
    let disk_percent = if disk_total_bytes == 0 {
        0.0
    } else {
        (disk_used_bytes as f32 / disk_total_bytes as f32) * 100.0
    };

    let mut network_rx_total = 0_u64;
    let mut network_tx_total = 0_u64;
    for (_, data) in system.networks() {
        network_rx_total += data.total_received();
        network_tx_total += data.total_transmitted();
    }

    let mut disk_read_total = 0_u64;
    let mut disk_write_total = 0_u64;
    for process in system.processes().values() {
        let usage = process.disk_usage();
        disk_read_total += usage.total_read_bytes;
        disk_write_total += usage.total_written_bytes;
    }

    let now = now_ts();
    let (network_rx_bytes, network_tx_bytes) = {
        let mut baseline = state.network_baseline.lock().expect("network baseline lock");
        let mut rx_bps = 0_u64;
        let mut tx_bps = 0_u64;

        if let Some(prev) = *baseline {
            let delta_t = (now - prev.ts).max(1) as u64;
            let delta_rx = network_rx_total.saturating_sub(prev.rx_total);
            let delta_tx = network_tx_total.saturating_sub(prev.tx_total);
            rx_bps = delta_rx / delta_t;
            tx_bps = delta_tx / delta_t;
        }

        *baseline = Some(NetworkBaseline {
            ts: now,
            rx_total: network_rx_total,
            tx_total: network_tx_total,
        });

        (rx_bps, tx_bps)
    };

    let estimated_iops = {
        let mut baseline = state.disk_baseline.lock().expect("disk baseline lock");
        let mut io_bps = 0_u64;

        if let Some(prev) = *baseline {
            let delta_t = (now - prev.ts).max(1) as u64;
            let delta_read = disk_read_total.saturating_sub(prev.read_total);
            let delta_write = disk_write_total.saturating_sub(prev.write_total);
            io_bps = (delta_read + delta_write) / delta_t;
        }

        *baseline = Some(DiskBaseline {
            ts: now,
            read_total: disk_read_total,
            write_total: disk_write_total,
        });

        (io_bps as f64) / 4096.0
    };

    let disk_iops = true_disk_iops_from_iostat(state, now).unwrap_or(estimated_iops);

    let load = system.load_average();
    SystemMetrics {
        cpu_percent: system.global_cpu_info().cpu_usage(),
        memory_total_bytes,
        memory_used_bytes,
        memory_percent,
        disk_total_bytes,
        disk_used_bytes,
        disk_percent,
        disk_iops,
        network_rx_bytes,
        network_tx_bytes,
        process_count: system.processes().len(),
        load_1: load.one,
        load_5: load.five,
        load_15: load.fifteen,
    }
}

fn true_disk_iops_from_iostat(state: &AppState, now_ts: i64) -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        let total_xfrs = read_macos_total_xfrs()?;
        let mut baseline = state
            .disk_xfrs_baseline
            .lock()
            .expect("disk xfrs baseline lock");

        let iops = if let Some(prev) = *baseline {
            let delta_t = (now_ts - prev.ts).max(1) as f64;
            let delta_xfrs = total_xfrs.saturating_sub(prev.xfrs_total) as f64;
            delta_xfrs / delta_t
        } else {
            0.0
        };

        *baseline = Some(DiskXfrsBaseline {
            ts: now_ts,
            xfrs_total: total_xfrs,
        });

        Some(iops)
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "linux")]
        {
            let total_ops = read_linux_total_disk_ops()?;
            let mut baseline = state
                .disk_ops_baseline
                .lock()
                .expect("disk ops baseline lock");

            let iops = if let Some(prev) = *baseline {
                let delta_t = (now_ts - prev.ts).max(1) as f64;
                let delta_ops = total_ops.saturating_sub(prev.ops_total) as f64;
                delta_ops / delta_t
            } else {
                0.0
            };

            *baseline = Some(DiskOpsBaseline {
                ts: now_ts,
                ops_total: total_ops,
            });

            Some(iops)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = state;
            let _ = now_ts;
            None
        }
    }
}

#[cfg(target_os = "macos")]
fn read_macos_total_xfrs() -> Option<u64> {
    let output = Command::new("iostat").args(["-Id"]).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8(output.stdout).ok()?;
    for line in text.lines().rev() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        if cols[0].parse::<f64>().is_ok() {
            if let Ok(x) = cols[1].parse::<u64>() {
                return Some(x);
            }
            if let Ok(xf) = cols[1].parse::<f64>() {
                return Some(xf as u64);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_linux_total_disk_ops() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/diskstats").ok()?;
    let mut total = 0_u64;

    for line in content.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 8 {
            continue;
        }
        let dev = cols[2];
        if dev.starts_with("loop")
            || dev.starts_with("ram")
            || dev.starts_with("fd")
            || dev.starts_with("sr")
            || dev.starts_with("dm-")
            || dev.starts_with("md")
        {
            continue;
        }
        let reads_completed = cols[3].parse::<u64>().ok()?;
        let writes_completed = cols[7].parse::<u64>().ok()?;
        total = total.saturating_add(reads_completed.saturating_add(writes_completed));
    }

    Some(total)
}

fn normalize_memory_to_bytes(value: u64) -> u64 {
    #[cfg(target_os = "macos")]
    {
        value
    }
    #[cfg(not(target_os = "macos"))]
    {
        value.saturating_mul(1024)
    }
}

fn select_disk_usage(system: &System) -> (u64, u64) {
    #[cfg(target_os = "macos")]
    {
        if let Some(data_disk) = system
            .disks()
            .iter()
            .find(|disk| disk.mount_point() == Path::new("/System/Volumes/Data"))
        {
            return (data_disk.total_space(), data_disk.available_space());
        }

        if let Some(root_disk) = system
            .disks()
            .iter()
            .find(|disk| disk.mount_point() == Path::new("/"))
        {
            return (root_disk.total_space(), root_disk.available_space());
        }

        if let Some(primary_disk) = system
            .disks()
            .iter()
            .filter(|disk| !disk.is_removable())
            .max_by_key(|disk| disk.total_space())
        {
            return (primary_disk.total_space(), primary_disk.available_space());
        }
    }

    let mut disk_total_bytes = 0_u64;
    let mut disk_available_bytes = 0_u64;
    for disk in system.disks() {
        disk_total_bytes += disk.total_space();
        disk_available_bytes += disk.available_space();
    }
    (disk_total_bytes, disk_available_bytes)
}

async fn collect_metrics_snapshot(state: &AppState) -> MetricsResponse {
    let system = collect_system_metrics(state);
    let containers = collect_container_metrics().await;
    MetricsResponse {
        system,
        containers,
        ts: now_ts(),
    }
}

fn persist_snapshot(state: &AppState, snapshot: &MetricsResponse) {
    let db = state.db.lock().expect("db lock");
    let _ = db.execute(
        "
        INSERT INTO system_metrics_history(
            ts, cpu_percent, memory_percent, disk_percent, disk_iops,
            network_total_bytes, network_rx_bytes, network_tx_bytes,
            process_count, load_1, load_5, load_15, container_count
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ",
        params![
            snapshot.ts,
            snapshot.system.cpu_percent,
            snapshot.system.memory_percent,
            snapshot.system.disk_percent,
            snapshot.system.disk_iops,
            snapshot.system.network_rx_bytes + snapshot.system.network_tx_bytes,
            snapshot.system.network_rx_bytes as i64,
            snapshot.system.network_tx_bytes as i64,
            snapshot.system.process_count as i64,
            snapshot.system.load_1,
            snapshot.system.load_5,
            snapshot.system.load_15,
            snapshot.containers.len() as i64,
        ],
    );

    update_system_aggregates(&db, snapshot);

    for container in &snapshot.containers {
        let _ = db.execute(
            "
            INSERT INTO container_metrics_history(
                ts, name, status, cpu_percent, memory_used_bytes, memory_limit_bytes,
                network_rx_bytes, network_tx_bytes, disk_read_bytes, disk_write_bytes,
                image, tag, restart_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ",
            params![
                snapshot.ts,
                container.name,
                container.status,
                container.cpu_percent,
                container.memory_used_bytes as i64,
                container.memory_limit_bytes as i64,
                container.network_rx_bytes as i64,
                container.network_tx_bytes as i64,
                container.disk_read_bytes as i64,
                container.disk_write_bytes as i64,
                container.image,
                container.tag,
                container.restart_count,
            ],
        );
    }
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

fn update_system_aggregates(db: &Connection, snapshot: &MetricsResponse) {
    let windows = [15_u32, 60_u32, 360_u32, 1440_u32];
    let sample_ms = snapshot.ts * 1000;
    let network_rx = snapshot.system.network_rx_bytes;
    let network_tx = snapshot.system.network_tx_bytes;
    let network_total = network_rx + network_tx;
    let container_count = snapshot.containers.len() as f64;

    for minutes in windows {
        let bucket_ms = (minutes as i64 * 60 * 1000) / MAX_TREND_POINTS as i64;
        let bucket_start_ms = sample_ms - (sample_ms % bucket_ms);
        let bucket_end_ms = bucket_start_ms + bucket_ms;
        let keep_from_ms = sample_ms - (minutes as i64 * 60 * 1000);

        let _ = db.execute(
            "
            INSERT INTO system_metrics_agg(
                window_minutes, bucket_start_ms, bucket_end_ms,
                cpu_sum, memory_sum, disk_iops_sum,
                network_sum, network_rx_sum, network_tx_sum,
                container_sum, samples
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)
            ON CONFLICT(window_minutes, bucket_start_ms) DO UPDATE SET
                bucket_end_ms = excluded.bucket_end_ms,
                cpu_sum = system_metrics_agg.cpu_sum + excluded.cpu_sum,
                memory_sum = system_metrics_agg.memory_sum + excluded.memory_sum,
                disk_iops_sum = system_metrics_agg.disk_iops_sum + excluded.disk_iops_sum,
                network_sum = system_metrics_agg.network_sum + excluded.network_sum,
                network_rx_sum = system_metrics_agg.network_rx_sum + excluded.network_rx_sum,
                network_tx_sum = system_metrics_agg.network_tx_sum + excluded.network_tx_sum,
                container_sum = system_metrics_agg.container_sum + excluded.container_sum,
                samples = system_metrics_agg.samples + 1
            ",
            params![
                minutes as i64,
                bucket_start_ms,
                bucket_end_ms,
                snapshot.system.cpu_percent as f64,
                snapshot.system.memory_percent as f64,
                snapshot.system.disk_iops,
                network_total as f64,
                network_rx as f64,
                network_tx as f64,
                container_count,
            ],
        );

        if minutes != 1440 {
            let _ = db.execute(
                "DELETE FROM system_metrics_agg WHERE window_minutes = ?1 AND bucket_end_ms < ?2",
                params![minutes as i64, keep_from_ms],
            );
        }
    }
}

async fn background_sampler(state: AppState) {
    loop {
        let snapshot = collect_metrics_snapshot(&state).await;
        persist_snapshot(&state, &snapshot);
        maybe_cleanup_raw_history(&state, snapshot.ts);
        *state.latest.lock().expect("latest lock") = Some(snapshot);

        tokio::time::sleep(Duration::from_secs(state.sample_interval_secs)).await;
    }
}

fn maybe_cleanup_raw_history(state: &AppState, now_ts: i64) {
    let current_day = now_ts / 86_400;
    {
        let mut last_day = state.last_cleanup_day.lock().expect("cleanup day lock");
        if *last_day == current_day {
            return;
        }
        *last_day = current_day;
    }

    let keep_from = now_ts - 24 * 60 * 60;
    let db = state.db.lock().expect("db lock");
    let _ = db.execute(
        "DELETE FROM system_metrics_history WHERE ts < ?1",
        params![keep_from],
    );
    let _ = db.execute(
        "DELETE FROM container_metrics_history WHERE ts < ?1",
        params![keep_from],
    );
}

async fn collect_container_metrics() -> Vec<ContainerMetrics> {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let list_opts = ListContainersOptions::<String> {
        all: true,
        ..Default::default()
    };
    let containers = match docker.list_containers(Some(list_opts)).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for summary in containers {
        let id = match summary.id {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };

        let name = summary
            .names
            .as_ref()
            .and_then(|v| v.first())
            .map(|s| s.trim_start_matches('/').to_string())
            .unwrap_or_else(|| id.chars().take(12).collect());
        let status = summary
            .status
            .clone()
            .or(summary.state.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let image_full = summary.image.unwrap_or_else(|| "unknown".to_string());
        let (image, tag) = split_image(&image_full);

        let inspect_opts = InspectContainerOptions { size: false };
        let restart_count = docker
            .inspect_container(&id, Some(inspect_opts))
            .await
            .ok()
            .and_then(|i| i.restart_count)
            .unwrap_or(0);

        let stats_opts = StatsOptions {
            stream: false,
            one_shot: true,
        };
        let mut stream = docker.stats(&id, Some(stats_opts));

        let mut cpu_percent = 0.0_f64;
        let mut memory_used_bytes = 0_u64;
        let mut memory_limit_bytes = 0_u64;
        let mut network_rx_bytes = 0_u64;
        let mut network_tx_bytes = 0_u64;
        let mut disk_read_bytes = 0_u64;
        let mut disk_write_bytes = 0_u64;

        if let Some(Ok(stats)) = stream.next().await {
            memory_used_bytes = stats.memory_stats.usage.unwrap_or(0);
            memory_limit_bytes = stats.memory_stats.limit.unwrap_or(0);

            let cpu_total = stats
                .cpu_stats
                .cpu_usage
                .total_usage as f64;
            let pre_cpu_total = stats
                .precpu_stats
                .cpu_usage
                .total_usage as f64;
            let system_cpu = stats.cpu_stats.system_cpu_usage.unwrap_or_default() as f64;
            let pre_system_cpu = stats.precpu_stats.system_cpu_usage.unwrap_or_default() as f64;
            let cpu_delta = cpu_total - pre_cpu_total;
            let system_delta = system_cpu - pre_system_cpu;

            let online_cpus = stats
                .cpu_stats
                .online_cpus
                .unwrap_or_else(|| {
                    stats
                        .cpu_stats
                        .cpu_usage
                        .percpu_usage
                        .as_ref()
                        .map(|v| v.len() as u64)
                        .unwrap_or(1)
                }) as f64;

            if cpu_delta > 0.0 && system_delta > 0.0 {
                cpu_percent = (cpu_delta / system_delta) * online_cpus * 100.0;
            }

            if let Some(networks) = stats.networks {
                for (_, net) in networks {
                    network_rx_bytes += net.rx_bytes;
                    network_tx_bytes += net.tx_bytes;
                }
            }

            if let Some(entries) = stats.blkio_stats.io_service_bytes_recursive {
                for entry in entries {
                    let op = entry.op.to_uppercase();
                    if op == "READ" {
                        disk_read_bytes += entry.value;
                    } else if op == "WRITE" {
                        disk_write_bytes += entry.value;
                    }
                }
            }
        }

        result.push(ContainerMetrics {
            name,
            status,
            cpu_percent,
            memory_used_bytes,
            memory_limit_bytes,
            network_rx_bytes,
            network_tx_bytes,
            disk_read_bytes,
            disk_write_bytes,
            image,
            tag,
            restart_count,
        });
    }

    result
}

fn split_image(image: &str) -> (String, String) {
    if let Some((name, tag)) = image.rsplit_once(':') {
        (name.to_string(), tag.to_string())
    } else {
        (image.to_string(), "latest".to_string())
    }
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

fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sessions (
            token TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            last_seen INTEGER NOT NULL,
            FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS system_metrics_history (
            ts INTEGER PRIMARY KEY,
            cpu_percent REAL NOT NULL,
            memory_percent REAL NOT NULL,
            disk_percent REAL NOT NULL,
            disk_iops REAL NOT NULL DEFAULT 0,
            network_total_bytes INTEGER NOT NULL,
            network_rx_bytes INTEGER NOT NULL DEFAULT 0,
            network_tx_bytes INTEGER NOT NULL DEFAULT 0,
            process_count INTEGER NOT NULL,
            load_1 REAL NOT NULL,
            load_5 REAL NOT NULL,
            load_15 REAL NOT NULL,
            container_count INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS system_metrics_agg (
            window_minutes INTEGER NOT NULL,
            bucket_start_ms INTEGER NOT NULL,
            bucket_end_ms INTEGER NOT NULL,
            cpu_sum REAL NOT NULL,
            memory_sum REAL NOT NULL,
            disk_iops_sum REAL NOT NULL,
            network_sum REAL NOT NULL,
            network_rx_sum REAL NOT NULL DEFAULT 0,
            network_tx_sum REAL NOT NULL DEFAULT 0,
            container_sum REAL NOT NULL,
            samples INTEGER NOT NULL,
            PRIMARY KEY(window_minutes, bucket_start_ms)
        );

        CREATE TABLE IF NOT EXISTS container_metrics_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            cpu_percent REAL NOT NULL,
            memory_used_bytes INTEGER NOT NULL,
            memory_limit_bytes INTEGER NOT NULL,
            network_rx_bytes INTEGER NOT NULL,
            network_tx_bytes INTEGER NOT NULL,
            disk_read_bytes INTEGER NOT NULL,
            disk_write_bytes INTEGER NOT NULL,
            image TEXT NOT NULL,
            tag TEXT NOT NULL,
            restart_count INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);
        CREATE INDEX IF NOT EXISTS idx_system_history_ts ON system_metrics_history(ts);
        CREATE INDEX IF NOT EXISTS idx_system_agg_window_end ON system_metrics_agg(window_minutes, bucket_end_ms);
        CREATE INDEX IF NOT EXISTS idx_container_history_ts ON container_metrics_history(ts);
        CREATE INDEX IF NOT EXISTS idx_container_history_name_ts ON container_metrics_history(name, ts);
        ",
    )?;

    let _ = conn.execute(
        "ALTER TABLE system_metrics_history ADD COLUMN disk_iops REAL NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE system_metrics_agg ADD COLUMN disk_iops_sum REAL NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE system_metrics_agg ADD COLUMN network_rx_sum REAL NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE system_metrics_agg ADD COLUMN network_tx_sum REAL NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE system_metrics_history ADD COLUMN network_rx_bytes INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE system_metrics_history ADD COLUMN network_tx_bytes INTEGER NOT NULL DEFAULT 0",
        [],
    );
    ensure_system_metrics_agg_schema(conn)?;
    Ok(())
}

fn state_needs_setup(state: &AppState) -> bool {
    let db = state.db.lock().expect("db lock");
    let users: i64 = db
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap_or(0);
    users == 0
}

fn resolve_session_from_headers(state: &AppState, headers: &HeaderMap) -> Option<AuthSession> {
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

fn create_session_token(state: &AppState, user_id: i64) -> Option<String> {
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

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let hashed = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| "hash error".to_string())?
        .to_string();
    Ok(hashed)
}

fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(v) => v,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ts_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn extract_cookie(headers: &HeaderMap, key: &str) -> Option<String> {
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

fn build_auth_cookie(token: &str) -> String {
    format!(
        "{}={}; Max-Age={}; Path=/; HttpOnly; SameSite=Lax",
        AUTH_COOKIE_NAME, token, TOKEN_LIFETIME_SECS
    )
}

fn clear_auth_cookie() -> String {
    format!(
        "{}=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax",
        AUTH_COOKIE_NAME
    )
}

fn with_auth_cookie<T: IntoResponse>(body: T, token: &str) -> Response {
    let mut response = body.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&build_auth_cookie(token)).expect("valid set-cookie"),
    );
    response
}

fn json_error(code: StatusCode, msg: &str) -> Response {
    (code, Json(serde_json::json!({ "error": msg }))).into_response()
}
