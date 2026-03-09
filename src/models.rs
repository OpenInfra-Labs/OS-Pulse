use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub(crate) const TOKEN_LIFETIME_SECS: i64 = 3 * 24 * 60 * 60;
pub(crate) const AUTH_COOKIE_NAME: &str = "osp_token";
pub(crate) const MAX_TREND_POINTS: usize = 96;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) db: Arc<Mutex<Connection>>,
    pub(crate) latest: Arc<Mutex<Option<MetricsResponse>>>,
    pub(crate) network_baseline: Arc<Mutex<Option<NetworkBaseline>>>,
    pub(crate) disk_baseline: Arc<Mutex<Option<DiskBaseline>>>,
    #[cfg(target_os = "macos")]
    pub(crate) disk_xfrs_baseline: Arc<Mutex<Option<DiskXfrsBaseline>>>,
    #[cfg(target_os = "linux")]
    pub(crate) disk_ops_baseline: Arc<Mutex<Option<DiskOpsBaseline>>>,
    pub(crate) last_cleanup_day: Arc<Mutex<i64>>,
    pub(crate) sample_interval_secs: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct NetworkBaseline {
    pub(crate) ts: i64,
    pub(crate) rx_total: u64,
    pub(crate) tx_total: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct DiskBaseline {
    pub(crate) ts: i64,
    pub(crate) read_total: u64,
    pub(crate) write_total: u64,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
pub(crate) struct DiskXfrsBaseline {
    pub(crate) ts: i64,
    pub(crate) xfrs_total: u64,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
pub(crate) struct DiskOpsBaseline {
    pub(crate) ts: i64,
    pub(crate) ops_total: u64,
}

#[derive(Clone)]
pub(crate) struct AuthSession {
    pub(crate) user_id: i64,
    pub(crate) username: String,
}

#[derive(Deserialize)]
pub(crate) struct AuthPayload {
    pub(crate) username: String,
    pub(crate) password: String,
}

#[derive(Serialize)]
pub(crate) struct AuthStateResponse {
    pub(crate) needs_setup: bool,
    pub(crate) logged_in: bool,
}

#[derive(Serialize)]
pub(crate) struct MeResponse {
    pub(crate) user_id: i64,
    pub(crate) username: String,
}

#[derive(Serialize, Clone)]
pub(crate) struct SystemMetrics {
    pub(crate) cpu_percent: f32,
    pub(crate) memory_total_bytes: u64,
    pub(crate) memory_used_bytes: u64,
    pub(crate) memory_percent: f32,
    pub(crate) disk_total_bytes: u64,
    pub(crate) disk_used_bytes: u64,
    pub(crate) disk_percent: f32,
    pub(crate) disk_iops: f64,
    pub(crate) network_rx_bytes: u64,
    pub(crate) network_tx_bytes: u64,
    pub(crate) process_count: usize,
    pub(crate) load_1: f64,
    pub(crate) load_5: f64,
    pub(crate) load_15: f64,
}

#[derive(Serialize, Clone)]
pub(crate) struct ContainerMetrics {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) cpu_percent: f64,
    pub(crate) memory_used_bytes: u64,
    pub(crate) memory_limit_bytes: u64,
    pub(crate) network_rx_bytes: u64,
    pub(crate) network_tx_bytes: u64,
    pub(crate) disk_read_bytes: u64,
    pub(crate) disk_write_bytes: u64,
    pub(crate) image: String,
    pub(crate) tag: String,
    pub(crate) restart_count: i64,
}

#[derive(Serialize, Clone)]
pub(crate) struct MetricsResponse {
    pub(crate) system: SystemMetrics,
    pub(crate) containers: Vec<ContainerMetrics>,
    pub(crate) ts: i64,
}

#[derive(Deserialize)]
pub(crate) struct TrendQuery {
    pub(crate) minutes: Option<u32>,
}

#[derive(Serialize)]
pub(crate) struct TrendPoint {
    pub(crate) ts: i64,
    pub(crate) cpu_percent: f32,
    pub(crate) memory_percent: f32,
    pub(crate) disk_iops: f64,
    pub(crate) network_rx_bytes: u64,
    pub(crate) network_tx_bytes: u64,
    pub(crate) container_count: usize,
}

#[derive(Serialize)]
pub(crate) struct TrendResponse {
    pub(crate) points: Vec<TrendPoint>,
    pub(crate) requested_minutes: u32,
    pub(crate) available_minutes: u32,
    pub(crate) returned_points: usize,
    pub(crate) sampled: bool,
}

#[derive(Deserialize)]
pub(crate) struct ContainerTrendQuery {
    pub(crate) minutes: Option<u32>,
    pub(crate) name: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ContainerTrendPoint {
    pub(crate) ts: i64,
    pub(crate) cpu_percent: f64,
    pub(crate) memory_used_bytes: u64,
    pub(crate) memory_limit_bytes: u64,
    pub(crate) network_total_bytes: u64,
    pub(crate) disk_io_total_bytes: u64,
}

#[derive(Serialize)]
pub(crate) struct ContainerTrendResponse {
    pub(crate) selected: Option<String>,
    pub(crate) available: Vec<String>,
    pub(crate) points: Vec<ContainerTrendPoint>,
}
