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
- **SQLite** — zero-config, embedded, single-file database
- Built-in history tables for system and container metric snapshots
- Aggregated trend buckets (15 m / 1 h / 6 h / 24 h) with automatic roll-up
- Auto-cleanup of raw history older than 48 hours

### Web Dashboard
- Clean, responsive single-page UI
- Real-time charts for all key metrics (system + containers)
- Historical trend graphs with selectable time ranges
- Dark / light theme
- No external dependencies — assets are embedded in the binary

### Implemented in Current Version
- Background sampler collects host + container metrics continuously
- Sampling interval configurable via `OSP_INTERVAL` (seconds, default `1`)
- Historical trend API: `GET /api/trends?minutes=60` (login required)
- Container trend API with name filter: `GET /api/trends/containers?minutes=60&name=<container>`
- Dashboard supports quick time range switching: `15m / 1h / 6h / 24h`

### Authentication & Access Control
- First-time startup requires creating the initial account on the login page
- All subsequent access is gated by login before entering the dashboard
- Token-based session authentication (HTTP-only cookie)
- Token lifetime is **3 days** and automatically extends on each authenticated operation

---

## Architecture

```
┌─────────────────────────────────────────────┐
│                 OS-Pulse                     │
│                                             │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐ │
│  │ System   │  │Container │  │  Storage   │ │
│  │ Collector│  │ Collector│  │  (SQLite)  │ │
│  │ (sysinfo)│  │ (Docker) │  │            │ │
│  └────┬─────┘  └────┬─────┘  │            │ │
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
# Start with default settings (SQLite, 1s sampling, port 3000)
./target/release/os-pulse

# Custom sampling interval (seconds)
OSP_INTERVAL=2 ./target/release/os-pulse
```

Then open **http://localhost:3000** in your browser.

On first access, you will be redirected to the login page and asked to create the initial account.

> **Note:** To monitor Docker containers, make sure the user running OS-Pulse has access to the Docker socket (`/var/run/docker.sock`).

### Run with Docker

No pre-built image is published yet. Start a Rust container and build inside it:

```bash
# 1. Create a persistent Rust container
docker run -dit \
  --name os-pulse \
  -p 3000:3000 \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  rust:latest

# 2. Enter the container
docker exec -it os-pulse bash

# 3. Clone, build and run (inside the container)
git clone https://github.com/OpenInfra-Labs/OS-Pulse.git
cd OS-Pulse
cargo build --release
./target/release/os-pulse
```

Then open **http://localhost:3000** from your host browser.

> **Tip:** The container keeps running in the background. You can re-attach with `docker exec -it os-pulse bash` at any time.

### Docker Compose

Alternatively, use the included `Dockerfile` for a self-contained image:

```yaml
version: "3.8"
services:
  os-pulse:
    build: .
    ports:
      - "3000:3000"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    environment:
      - OSP_INTERVAL=1
    restart: unless-stopped
```

---

## Configuration

OS-Pulse is configured via environment variables.

| Env Var | Default | Description |
|---------|---------|-------------|
| `OSP_INTERVAL` | `1` | Metric collection interval in seconds |

The web dashboard always listens on **port 3000**. Data is stored in an SQLite database (`os_pulse.db`) in the working directory.

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
- [x] System metrics collector (CPU, memory, disk, network)
- [x] Docker container metrics collector
- [x] SQLite storage backend
- [x] REST API
- [x] Web dashboard (single-page)
- [x] Historical trend charts (system + container)
- [x] Token-based authentication & session management
- [x] Dockerfile for containerised deployment
- [ ] Process list collector
- [ ] InfluxDB storage backend
- [ ] TimescaleDB storage backend
- [ ] WebSocket real-time push
- [ ] Alerting / threshold notifications
- [ ] Prometheus export endpoint
- [ ] Plugin system for custom collectors
- [ ] ARM64 / multi-arch Docker images
- [ ] Published Docker image (GHCR)

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
