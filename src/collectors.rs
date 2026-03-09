#[cfg(target_os = "macos")]
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::time::Duration;

use bollard::container::{InspectContainerOptions, ListContainersOptions, StatsOptions};
use bollard::container::MemoryStatsStats;
use bollard::Docker;
use futures_util::StreamExt;
use sysinfo::{CpuExt, DiskExt, NetworkExt, ProcessExt, System, SystemExt};

use crate::db::{maybe_cleanup_raw_history, persist_snapshot};
use crate::models::*;
use crate::now_ts;

pub(crate) fn collect_system_metrics(state: &AppState) -> SystemMetrics {
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
        let mut baseline = state
            .network_baseline
            .lock()
            .expect("network baseline lock");
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
    // sysinfo 0.29+ returns memory in bytes on all platforms
    value
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

    #[cfg(not(target_os = "macos"))]
    {
        use std::path::Path;

        // Prefer the root mount point to avoid summing virtual filesystems
        if let Some(root_disk) = system
            .disks()
            .iter()
            .find(|disk| disk.mount_point() == Path::new("/"))
        {
            return (root_disk.total_space(), root_disk.available_space());
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

pub(crate) async fn collect_metrics_snapshot(state: &AppState) -> MetricsResponse {
    let system = collect_system_metrics(state);
    let containers = collect_container_metrics().await;
    MetricsResponse {
        system,
        containers,
        ts: now_ts(),
    }
}

pub(crate) async fn collect_container_metrics() -> Vec<ContainerMetrics> {
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
            let raw_usage = stats.memory_stats.usage.unwrap_or(0);
            memory_limit_bytes = stats.memory_stats.limit.unwrap_or(0);

            // Subtract filesystem cache to get actual working set memory,
            // matching what `docker stats` reports.
            let cache = match stats.memory_stats.stats {
                Some(MemoryStatsStats::V1(v1)) => v1.total_inactive_file,
                Some(MemoryStatsStats::V2(v2)) => v2.inactive_file,
                None => 0,
            };
            memory_used_bytes = raw_usage.saturating_sub(cache);

            let cpu_total = stats.cpu_stats.cpu_usage.total_usage as f64;
            let pre_cpu_total = stats.precpu_stats.cpu_usage.total_usage as f64;
            let system_cpu = stats.cpu_stats.system_cpu_usage.unwrap_or_default() as f64;
            let pre_system_cpu =
                stats.precpu_stats.system_cpu_usage.unwrap_or_default() as f64;
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

pub(crate) async fn background_sampler(state: AppState) {
    loop {
        let snapshot = collect_metrics_snapshot(&state).await;
        persist_snapshot(&state, &snapshot);
        maybe_cleanup_raw_history(&state, snapshot.ts);
        *state.latest.lock().expect("latest lock") = Some(snapshot);

        tokio::time::sleep(Duration::from_secs(state.sample_interval_secs)).await;
    }
}
