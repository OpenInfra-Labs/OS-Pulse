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
- **SQLite** —— 零配置，嵌入式单文件数据库
- 内置系统与容器指标快照历史表
- 聚合趋势桶（15 分钟 / 1 小时 / 6 小时 / 24 小时），自动汇总
- 自动清理超过 48 小时的原始历史数据

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
- 容器趋势接口（按容器筛选）：`GET /api/trends/containers?minutes=60&name=<container>`
- Dashboard 支持时间范围快速切换：`15m / 1h / 6h / 24h`

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
│  │  采集器  │  │  采集器  │  │  (SQLite)  │ │
│  │ (sysinfo)│  │ (Docker) │  │            │ │
│  └────┬─────┘  └────┬─────┘  │            │ │
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
# 使用默认设置启动（SQLite，1 秒采样，端口 3000）
./target/release/os-pulse

# 自定义采样间隔（秒）
OSP_INTERVAL=2 ./target/release/os-pulse
```

然后在浏览器中打开 **http://localhost:3000**。

首次访问会先进入登录页，并要求创建初始账号。

> **注意：** 如需监控 Docker 容器，请确保运行 OS-Pulse 的用户有权访问 Docker Socket（`/var/run/docker.sock`）。

### 使用 Docker 运行

目前没有预构建的 Docker 镜像，请使用项目内的 `Dockerfile` 从源码构建：

```bash
# 构建镜像
docker build -t os-pulse .

# 运行容器
docker run -d \
  --name os-pulse \
  -p 3000:3000 \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  os-pulse
```

### Docker Compose

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

### 使用 Rust 容器快速体验

如果本地未安装 Rust 工具链，可以直接在 Rust 容器内构建和运行：

 ```bash
docker run -it --rm \
  -p 3000:3000 \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -v "$(pwd)":/app \
  -w /app \
  rust:1.85-bookworm \
  bash -c "cargo build --release && ./target/release/os-pulse"
```

---

## 配置

OS-Pulse 通过环境变量进行配置。

| 环境变量 | 默认值 | 说明 |
|----------|--------|------|
| `OSP_INTERVAL` | `1` | 指标采集间隔（秒） |

Web 仪表盘固定监听 **3000** 端口。数据存储在工作目录下的 SQLite 数据库文件（`os_pulse.db`）中。

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
- [x] 系统指标采集器（CPU、内存、磁盘、网络）
- [x] Docker 容器指标采集器
- [x] SQLite 存储后端
- [x] REST API
- [x] Web 仪表盘（单页应用）
- [x] 历史趋势图表（系统 + 容器）
- [x] Token 认证与会话管理
- [x] Dockerfile 容器化部署
- [ ] 进程列表采集器
- [ ] InfluxDB 存储后端
- [ ] TimescaleDB 存储后端
- [ ] WebSocket 实时推送
- [ ] 告警 / 阈值通知
- [ ] Prometheus 导出端点
- [ ] 插件系统（自定义采集器）
- [ ] ARM64 / 多架构 Docker 镜像
- [ ] 发布 Docker 镜像（GHCR）

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
