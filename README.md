# Nekodash

<img src="assets/icons/app-128.png" width="80" height="80" alt="NekoDash">

Rust + Slint 编写的跨平台 Mihomo 管理面板。连接本机或局域网中的官方 Mihomo，
查看运行状态、选择节点、检查连接与规则，并调整运行配置。

现已接通首轮 Slint 界面，包括连接管理、概览、代理、规则、连接、流量、日志与配置。
页面按固定上游版本逐项对照，具体进度见 [UI 实现与对照](docs/ui.md)。

## 项目结构

- `crates/nekodash-core`：异步 HTTP/WebSocket 客户端、数据模型、连接会话及配置存储。
- `src`、`ui`：Rust 应用入口与 Slint 界面。
- [上游基线](docs/upstream-baseline.md)：参考版本、协议及功能跟踪。
- [开发约定](docs/development.md)：代码要求、平台目标与验证方式。
- [通信库使用](docs/core.md)：接口、错误、实时订阅及示例。
- [验证记录](docs/verification.md)：已执行检查与平台状态。
- [图标与主题](docs/themes.md)：主题来源、Material 构建方式与三组截图对比。
- [构建与发布](docs/releases.md)：tag 自动构建、Android 签名配置及各平台安装方式。

## 开发

本次开发使用 Rust 1.98.1、Slint 1.18.1，依赖由 `Cargo.lock` 固定。

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p nekodash --locked
```

构建 Release：

```sh
cargo build -p nekodash --release --locked
```

发布配置使用 LTO、体积优化、符号裁剪及单个 codegen unit。
