<p align="center">
  <h1 align="center">OS-Pulse</h1>
  <p align="center">
    轻量级、高性能的系统与容器监控工具，使用 Rust 构建。
    <br />
    <a href="README.md">English</a> · <a href="#功能特性">功能特性</a> · <a href="#快速开始">快速开始</a> · <a href="#截图">截图</a>
  </p>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.75%2B-orange.svg" alt="Rust"></a>
  <a href="https://github.com/OpenInfra-Labs/OS-Pulse/releases"><img src="https://img.shields.io/github/v/release/OpenInfra-Labs/OS-Pulse?color=green" alt="Release"></a>
</p>

---

## OS-Pulse 是什么？

**OS-Pulse** 是一个单二进制文件的监控代理与仪表盘，可实时监控宿主机系统 **和** Docker 容器。它以可配置的频率采集指标，存储历史数据用于趋势分析，并提供简洁的 Web UI —— 得益于 Rust，资源开销极低。

它既可以 **在宿主机上运行**，也可以 **在容器内运行**。

---

## 功能特性

### 系统资源监控
- **CPU** —— 每核使用率、频率、温度
- **内存** —— 总量 / 已用 / 可用 / 交换分区
- **磁盘 I/O** —— 每设备读写吞吐量
- **网络** —— 每接口带宽、数据包统计
- **进程列表** —— 按 CPU 和内存排序
- **负载均值** —— 1 / 5 / 15 分钟负载

### 容器监控
- **每容器指标** —— CPU、内存、网络 I/O、磁盘 I/O
- **容器状态** —— 运行中 / 已停止 / 重启次数
- **镜像信息** —— 名称、标签、大小、创建时间
- 自动发现新增 / 移除的容器

### 实时数据采集
- 高频采样（可配置间隔，默认 1 秒）
- 宿主机 + 容器指标同步采集
- 极低开销 —— 通常 < 1% CPU，< 20 MB 内存

### 历史数据存储
- **SQLite** —— 零配置，嵌入式（默认）
- **InfluxDB** —— 适用于大规模或集群部署
- **TimescaleDB** —— 基于 PostgreSQL 的时序数据库扩展
- 可配置保留策略
- 支持趋势分析和跨时段对比
- 内置 SQLite 历史表，记录系统与容器快照

### Web 仪表盘
- 简洁、响应式的单页 UI
- 所有关键指标的实时图表（系统 + 容器）
- 可选时间范围的历史趋势图
- 深色 / 浅色主题
- 无外部依赖 —— 所有静态资源内嵌于二进制文件中

### 当前版本已实现
- 后台采样任务持续采集宿主机与容器指标
- 采样间隔可通过 `OSP_INTERVAL`（秒）配置，默认 `1`
- 历史趋势接口：`GET /api/trends?minutes=60`（需登录）

### 鉴权与访问控制
- 首次启动时需在登录页创建初始账号
- 后续每次进入系统都必须先在登录页登录
- 基于 Token 的会话认证（HTTP-only Cookie）
- Token 有效期为 **3 天**，且每次鉴权操作都会自动续期

---

## 架构

```
┌─────────────────────────────────────────────┐
│                 OS-Pulse                     │
│                                             │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐ │
│  │  系统    │  │  容器    │  │  存储层   │ │
│  │  采集器  │  │  采集器  │  │ (SQLite / │ │
│  │ (sysinfo)│  │ (Docker) │  │ InfluxDB /│ │
│  └────┬─────┘  └────┬─────┘  │TimescaleDB)│ │
│       │              │        └─────┬──────┘ │
│       └──────┬───────┘              │        │
│              ▼                      │        │
│        ┌───────────┐               │        │
│        │  指标引擎  │◄──────────────┘        │
│        │           │                        │
│        └─────┬─────┘                         │
│              ▼                               │
│        ┌───────────┐                         │
│        │  Web UI   │  ← HTTP / WebSocket     │
│        │  (Axum)   │                         │
│        └───────────┘                         │
└─────────────────────────────────────────────┘
```

---

## 快速开始

### 环境要求

| 依赖 | 版本 |
|------|------|
| Rust 工具链 | 1.75+ |
| Docker（可选） | 20.10+ |

### 从源码构建

```bash
git clone https://github.com/OpenInfra-Labs/OS-Pulse.git
cd OS-Pulse
cargo build --release
```

二进制文件位于 `target/release/os-pulse`。

### 运行

```bash
# 使用默认设置启动（SQLite，1 秒间隔，端口 3000）
./target/release/os-pulse

# 自定义配置
./target/release/os-pulse \
  --interval 2 \
  --port 8080 \
  --storage influxdb \
  --influxdb-url http://localhost:8086
```

然后在浏览器中打开 **http://localhost:3000**。

首次访问会先进入登录页，并要求创建初始账号。

### 使用 Docker 运行

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

## 配置

OS-Pulse 支持通过 CLI 参数、环境变量或 TOML 配置文件进行配置。

| 参数 | CLI 标志 | 环境变量 | 默认值 | 说明 |
|------|----------|----------|--------|------|
| 采样间隔 | `--interval` | `OSP_INTERVAL` | `1` | 指标采集间隔（秒） |
| HTTP 端口 | `--port` | `OSP_PORT` | `3000` | Web 仪表盘端口 |
| 存储后端 | `--storage` | `OSP_STORAGE` | `sqlite` | `sqlite`、`influxdb` 或 `timescaledb` |
| 数据目录 | `--data-dir` | `OSP_DATA_DIR` | `./data` | SQLite 数据库文件目录 |
| 保留时间 | `--retention` | `OSP_RETENTION` | `7d` | 历史数据保留时长 |
| 日志级别 | `--log-level` | `OSP_LOG_LEVEL` | `info` | `trace`、`debug`、`info`、`warn`、`error` |

**配置文件示例**（`os-pulse.toml`）：

```toml
[general]
interval = 2          # 秒
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

## 截图

> _Web 仪表盘实现后将添加截图。_

<!--
![仪表盘概览](docs/screenshots/dashboard.png)
![容器指标](docs/screenshots/containers.png)
![历史趋势](docs/screenshots/trends.png)
-->

---

## 路线图

- [x] 项目脚手架
- [ ] 系统指标采集器（CPU、内存、磁盘、网络）
- [ ] 进程列表采集器
- [ ] Docker 容器指标采集器
- [ ] SQLite 存储后端
- [ ] InfluxDB 存储后端
- [ ] TimescaleDB 存储后端
- [ ] REST API
- [ ] WebSocket 实时推送
- [ ] Web 仪表盘（单页应用）
- [ ] 历史趋势图表
- [ ] 告警 / 阈值通知
- [ ] Prometheus 导出端点
- [ ] 插件系统（自定义采集器）
- [ ] ARM64 / 多架构 Docker 镜像

---

## 技术栈

| 组件 | Crate / 技术 |
|------|-------------|
| 异步运行时 | [Tokio](https://tokio.rs/) |
| HTTP 框架 | [Axum](https://github.com/tokio-rs/axum) |
| 系统信息 | [sysinfo](https://crates.io/crates/sysinfo) |
| Docker API | [bollard](https://crates.io/crates/bollard) |
| SQLite | [rusqlite](https://crates.io/crates/rusqlite) |
| 序列化 | [serde](https://serde.rs/) / [serde_json](https://crates.io/crates/serde_json) |
| 日志 | [tracing](https://crates.io/crates/tracing) |
| 前端图表 | [Chart.js](https://www.chartjs.org/)（内嵌） |
| 配置 | [toml](https://crates.io/crates/toml) / [clap](https://crates.io/crates/clap) |

---

## 贡献

欢迎贡献！请参阅 [CONTRIBUTING.md](CONTRIBUTING.md) 了解贡献指南。

1. Fork 本仓库
2. 创建功能分支（`git checkout -b feature/amazing-feature`）
3. 提交更改（`git commit -m 'feat: add amazing feature'`）
4. 推送到分支（`git push origin feature/amazing-feature`）
5. 创建 Pull Request

---

## 许可证

本项目基于 **MIT 许可证** 分发。详见 [LICENSE](LICENSE)。

---

<p align="center">
  使用 🦀 构建，来自 <a href="https://github.com/OpenInfra-Labs">OpenInfra Labs</a>
</p>
