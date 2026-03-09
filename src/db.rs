use std::collections::HashMap;

use rusqlite::{Connection, params};

use crate::models::*;
use crate::{now_ts, now_ts_ms};

pub(crate) fn init_db(conn: &Connection) -> rusqlite::Result<()> {
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

pub(crate) fn rebuild_recent_system_aggregates(conn: &Connection) -> rusqlite::Result<()> {
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

pub(crate) fn persist_snapshot(state: &AppState, snapshot: &MetricsResponse) {
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
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1)
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

pub(crate) fn maybe_cleanup_raw_history(state: &AppState, now_ts: i64) {
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
    let _ = db.execute(
        "DELETE FROM sessions WHERE expires_at < ?1",
        params![now_ts],
    );
}
