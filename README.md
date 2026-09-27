# Nekodash

Rust + Slint 编写的跨平台核心管理面板。连接本机或局域网中的核心，
查看运行状态、选择节点、检查连接与规则，并调整运行配置。

当前开发阶段为核心通信与连接生命周期。界面按固定上游版本分阶段实现。

## 项目结构

- `crates/nekodash-core`：异步 HTTP/WebSocket 客户端、数据模型、连接会话及配置存储。
- `src`、`ui`：Rust 应用入口与 Slint 界面。
- [上游基线](docs/upstream-baseline.md)：参考版本、协议及功能跟踪。
- [开发约定](docs/development.md)：代码要求、平台目标与验证方式。
- [通信库使用](docs/core.md)：接口、错误、实时订阅及示例。
- [验证记录](docs/verification.md)：已执行检查与平台状态。

## 开发

本次开发使用 Rust 1.98.1、Slint 1.18.1，依赖由 `Cargo.lock` 固定。

```sh
cargo test -p nekodash-core --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p nekodash --locked
```

构建 Release：

```sh
cargo build -p nekodash --release --locked
```

发布配置使用 LTO、体积优化、符号裁剪及单个 codegen unit。
