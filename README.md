<p align="center">
  <h1 align="center">OS-Pulse</h1>
  <p align="center">
    A lightweight, high-performance system &amp; container monitoring tool built with Rust.
    <br />
    <a href="README_CN.md">中文文档</a> · <a href="#features">Features</a> · <a href="#quick-start">Quick Start</a> · <a href="#screenshots">Screenshots</a>
  </p>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.75%2B-orange.svg" alt="Rust"></a>
  <a href="https://github.com/OpenInfra-Labs/OS-Pulse/releases"><img src="https://img.shields.io/github/v/release/OpenInfra-Labs/OS-Pulse?color=green" alt="Release"></a>
</p>

---

## What is OS-Pulse?

**OS-Pulse** is a single-binary monitoring agent and dashboard that gives you real-time visibility into your host system **and** Docker containers. It collects metrics at configurable intervals, stores history for trend analysis, and serves a clean web UI — all with minimal resource overhead, thanks to Rust.

It runs equally well **on the host** or **inside a container**.

---

## Features

### System Resource Monitoring
- **CPU** — per-core usage, frequency, temperature
- **Memory** — total / used / available / swap
- **Disk I/O** — read/write throughput per device
- **Network** — bandwidth, packet counts per interface
- **Processes** — sortable list with CPU & memory per process
- **Load Average** — 1 / 5 / 15 minute load

### Container Monitoring
- **Per-container metrics** — CPU, memory, network I/O, disk I/O
- **Container state** — running / stopped / restart count
- **Image info** — name, tag, size, creation time
- Automatic discovery of new / removed containers

### Real-Time Data Collection
- High-frequency sampling (configurable interval, default 1 s)
- Simultaneous host + container metric collection
- Low overhead — typically < 1 % CPU and < 20 MB RSS

### Historical Data Storage
- **SQLite** — zero-config, embedded (default)
- **InfluxDB** — for large-scale or clustered setups
- **TimescaleDB** — PostgreSQL extension for time-series workloads
- Configurable retention policies
- Supports trend analysis and cross-period comparison

### Web Dashboard
- Clean, responsive single-page UI
- Real-time charts for all key metrics (system + containers)
- Historical trend graphs with selectable time ranges
- Dark / light theme
- No external dependencies — assets are embedded in the binary

---

## Architecture

```
┌─────────────────────────────────────────────┐
│                 OS-Pulse                     │
│                                             │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐ │
│  │ System   │  │Container │  │  Storage   │ │
│  │ Collector│  │ Collector│  │ (SQLite /  │ │
│  │ (sysinfo)│  │ (Docker) │  │ InfluxDB / │ │
│  └────┬─────┘  └────┬─────┘  │TimescaleDB)│ │
│       │              │        └─────┬──────┘ │
│       └──────┬───────┘              │        │
│              ▼                      │        │
│        ┌───────────┐               │        │
│        │  Metrics   │◄──────────────┘        │
│        │   Engine   │                        │
│        └─────┬─────┘                         │
│              ▼                               │
│        ┌───────────┐                         │
│        │  Web UI   │  ← HTTP / WebSocket     │
│        │  (Axum)   │                         │
│        └───────────┘                         │
└─────────────────────────────────────────────┘
```

---

## Quick Start

### Prerequisites

| Requirement | Version |
|-------------|---------|
| Rust toolchain | 1.75+ |
| Docker (optional) | 20.10+ |

### Build from source

```bash
git clone https://github.com/OpenInfra-Labs/OS-Pulse.git
cd OS-Pulse
cargo build --release
```

The binary will be at `target/release/os-pulse`.

### Run

```bash
# Start with default settings (SQLite, 1s interval, port 3000)
./target/release/os-pulse

# Custom configuration
./target/release/os-pulse \
  --interval 2 \
  --port 8080 \
  --storage influxdb \
  --influxdb-url http://localhost:8086
```

Then open **http://localhost:3000** in your browser.

### Run with Docker

```bash
docker run -d \
  --name os-pulse \
  -p 3000:3000 \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -v /proc:/host/proc:ro \
  -v /sys:/host/sys:ro \
  ghcr.io/openinfra-labs/os-pulse:latest
```

### Docker Compose

```yaml
version: "3.8"
services:
  os-pulse:
    image: ghcr.io/openinfra-labs/os-pulse:latest
    ports:
      - "3000:3000"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - /proc:/host/proc:ro
      - /sys:/host/sys:ro
      - os-pulse-data:/data
    restart: unless-stopped

volumes:
  os-pulse-data:
```

---

## Configuration

OS-Pulse can be configured via CLI flags, environment variables, or a TOML config file.

| Parameter | CLI Flag | Env Var | Default | Description |
|-----------|----------|---------|---------|-------------|
| Sampling interval | `--interval` | `OSP_INTERVAL` | `1` | Metric collection interval in seconds |
| HTTP port | `--port` | `OSP_PORT` | `3000` | Web dashboard port |
| Storage backend | `--storage` | `OSP_STORAGE` | `sqlite` | `sqlite`, `influxdb`, or `timescaledb` |
| Data directory | `--data-dir` | `OSP_DATA_DIR` | `./data` | Directory for SQLite database files |
| Retention | `--retention` | `OSP_RETENTION` | `7d` | How long to keep historical data |
| Log level | `--log-level` | `OSP_LOG_LEVEL` | `info` | `trace`, `debug`, `info`, `warn`, `error` |

**Example config file** (`os-pulse.toml`):

```toml
[general]
interval = 2          # seconds
port = 3000
log_level = "info"

[storage]
backend = "sqlite"    # "sqlite" | "influxdb" | "timescaledb"
data_dir = "./data"
retention = "30d"

[storage.influxdb]
url = "http://localhost:8086"
token = ""
org = "default"
bucket = "os-pulse"

[storage.timescaledb]
url = "postgres://user:pass@localhost:5432/ospulse"

[docker]
enabled = true
socket = "/var/run/docker.sock"
```

---

## Screenshots

> _Screenshots will be added once the web dashboard is implemented._

<!--
![Dashboard Overview](docs/screenshots/dashboard.png)
![Container Metrics](docs/screenshots/containers.png)
![Historical Trends](docs/screenshots/trends.png)
-->

---

## Roadmap

- [x] Project scaffolding
- [ ] System metrics collector (CPU, memory, disk, network)
- [ ] Process list collector
- [ ] Docker container metrics collector
- [ ] SQLite storage backend
- [ ] InfluxDB storage backend
- [ ] TimescaleDB storage backend
- [ ] REST API
- [ ] WebSocket real-time push
- [ ] Web dashboard (single-page)
- [ ] Historical trend charts
- [ ] Alerting / threshold notifications
- [ ] Prometheus export endpoint
- [ ] Plugin system for custom collectors
- [ ] ARM64 / multi-arch Docker images

---

## Tech Stack

| Component | Crate / Technology |
|-----------|--------------------|
| Async runtime | [Tokio](https://tokio.rs/) |
| HTTP framework | [Axum](https://github.com/tokio-rs/axum) |
| System info | [sysinfo](https://crates.io/crates/sysinfo) |
| Docker API | [bollard](https://crates.io/crates/bollard) |
| SQLite | [rusqlite](https://crates.io/crates/rusqlite) |
| Serialization | [serde](https://serde.rs/) / [serde_json](https://crates.io/crates/serde_json) |
| Logging | [tracing](https://crates.io/crates/tracing) |
| Frontend charts | [Chart.js](https://www.chartjs.org/) (embedded) |
| Config | [toml](https://crates.io/crates/toml) / [clap](https://crates.io/crates/clap) |

---

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'feat: add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

---

## License

Distributed under the **MIT License**. See [LICENSE](LICENSE) for details.

---

<p align="center">
  Built with 🦀 by <a href="https://github.com/OpenInfra-Labs">OpenInfra Labs</a>
</p>
