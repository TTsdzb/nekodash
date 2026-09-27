# 验证记录

日期：2026-09-27。上游基线：`v1.273.1` / `8bbc8f58fef71148a94fb5c0ff808f79b057337d`。

## 已完成的本地检查

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过，包含应用入口、构建脚本、通信库、测试及示例 |
| `cargo check -p nekodash --locked` | Linux Slint 应用目标编译通过 |
| `cargo test -p nekodash-core --locked` | 24 项通过，另有 1 项显式启用的核心联调测试 |
| 隔离 Mihomo 联调测试 | 1 项通过，核心版本 `1.19.31`，Linux x64 |
| Android arm64 通信库 | 交叉编译检查通过，NDK `29.0.13113456`、API 28 |
| Windows x64 通信库 | `x86_64-pc-windows-gnu` 交叉编译检查通过 |

Rust：`1.98.1 (48a229cea 2026-09-01)`。Slint：`1.18.1`。
Clippy 的生成代码例外仅作用于 Slint 生成模块，手写应用代码和通信库继续使用工作区检查规则。

## 自动化测试范围

- HTTP 方法、路径、JSON 请求体、Bearer 鉴权与成功的空响应。
- 中文及特殊字符名称、反向代理路径前缀、延迟测试查询参数和代理集内节点定位。
- 规则数组、稀疏数字索引对象、`size: -1` 以及核心扩展字段。
- 鉴权错误、接口不可用、服务端错误、重定向、解析失败、超时和超大响应。
- HTTP 分块传输的体积与总超时限制。
- 外部配置下载、重定向与鉴权隔离；下载失败时核心状态保持原样。
- 批量延迟测试的并发上限、单项失败与取消。
- 实例切换取消在途请求，旧会话结果标识失效。
- WebSocket 断线恢复、畸形消息、取消、鉴权终止、心跳、队列溢出和消息大小限制。
- HTTPS/WSS 的附加 CA、默认不信任测试证书、主机名校验。
- 连接配置持久化、原子替换、选择与删除、schema 校验和 Unix 文件权限。

## 实际核心验证

测试自行创建临时核心进程、独立目录、控制端口、代理集、规则集和 HTTP 测试目标。
验证了版本、节点、代理集、规则集、规则、连接快照，节点选择、模式修改、
代理集/规则集更新、健康检查、节点及组延迟、规则禁用状态、缓存清理、
配置重载，以及速率/内存/连接 WebSocket 数据。

联调发现规则 `size` 的缺省值为 `-1`，已修正模型并加入回归用例。
延迟测试的本地目标保留 10ms 响应时间，以符合核心将 0ms 结果视为失败的行为。
依据：[规则响应](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/rules.go)、
[延迟接口](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/proxies.go)。

测试结束会等待自己启动的进程退出，并释放临时资源。

## 后续平台验证

`.github/workflows/core.yml` 已配置 Linux、Windows MSVC、macOS arm64、macOS Intel
的通信库原生测试及 Android arm64 编译检查。远端 CI 结果待仓库发布后记录。
Windows、macOS 和 Android 的设备运行验证随应用界面与打包阶段进行。
Linux Wayland/X11 的视觉、输入及性能验证也属于界面阶段。
