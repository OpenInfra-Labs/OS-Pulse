mod auth;
mod collectors;
mod db;
mod handlers;
mod models;

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::Connection;

use crate::models::AppState;

#[tokio::main]
async fn main() {
    let connection = Connection::open("os_pulse.db").expect("open sqlite database");
    db::init_db(&connection).expect("init database");
    db::rebuild_recent_system_aggregates(&connection).expect("rebuild recent aggregates");

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

    tokio::spawn(collectors::background_sampler(state.clone()));

    let protected = Router::new()
        .route("/dashboard", get(handlers::dashboard_page))
        .route("/api/me", get(handlers::api_me))
        .route("/api/metrics", get(handlers::api_metrics))
        .route("/api/trends", get(handlers::api_trends))
        .route("/api/trends/containers", get(handlers::api_container_trends))
        .route("/api/action", post(handlers::api_action))
        .route("/api/auth/logout", post(handlers::api_logout))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ));

    let app = Router::new()
        .route("/", get(handlers::root_redirect))
        .route("/login", get(handlers::login_page))
        .route("/styles.css", get(handlers::styles_css))
        .route("/api/auth/state", get(handlers::api_auth_state))
        .route("/api/auth/setup", post(handlers::api_auth_setup))
        .route("/api/auth/login", post(handlers::api_auth_login))
        .merge(protected)
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:3000".parse().expect("valid socket address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    println!("OS-Pulse running at http://{}", addr);
    axum::serve(listener, app).await.expect("start server");
}

pub(crate) fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn now_ts_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn json_error(code: StatusCode, msg: &str) -> Response {
    (code, Json(serde_json::json!({ "error": msg }))).into_response()
}
