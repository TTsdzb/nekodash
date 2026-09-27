# 验证记录

日期：2026-09-27。上游基线：`v1.273.1` / `8bbc8f58fef71148a94fb5c0ff808f79b057337d`。
通信回归、隔离核心联调、格式、Clippy 及 Android/Windows 交叉编译检查于 2026-09-28 更新。
Slint 应用首轮界面及本地运行检查于 2026-09-28 更新。

## 已完成的本地检查

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过，包含应用入口、构建脚本、通信库、测试及示例 |
| `cargo check -p nekodash --locked` | Linux Slint 应用目标编译通过 |
| 通信库常规测试 | 43 项通过，另有 1 项显式启用的核心联调测试；本次以 `--include-ignored` 一并运行 |
| 隔离 Mihomo 联调测试 | 1 项通过，核心版本 `1.19.31`，Linux x64 |
| Android arm64 通信库 | 交叉编译检查通过，NDK `29.0.13113456`、API 28 |
| Windows x64 通信库 | `x86_64-pc-windows-gnu` 交叉编译检查通过 |

Rust：`1.98.1 (48a229cea 2026-09-01)`。Slint：`1.18.1`。
Clippy 的生成代码例外仅作用于 Slint 生成模块，手写应用代码和通信库继续使用工作区检查规则。

## 自动化测试范围

- HTTP 方法、路径、JSON 请求体、Bearer 鉴权与成功的空响应。
- 中文及特殊字符名称、反向代理路径前缀、延迟测试查询参数和代理集内节点定位。
- 规则数组、稀疏数字索引对象、`size: -1` 以及核心扩展字段。
- DNS `Status`、PTR 回答、无回答的 DNS 状态码；订阅统计字段的负数、缺省值与 `i64` 边界。
- 鉴权错误、接口不可用、服务端错误、重定向、解析失败、超时和超大响应。
- HTTP 分块传输的体积与总超时限制。
- 外部配置下载、重定向与鉴权隔离；下载失败时核心状态保持原样。
- 批量延迟测试的并发上限、逐项结果、原始输入索引、单项失败、空批次与取消后保留结果。
- 实例切换和重新连接时取消在途请求、订阅与批次；旧成功、错误及取消事件的会话标识失效。
- 恢复时重复检查版本及刷新快照，暂时失败重试、能力差异、鉴权失败、总超时及取消。
- 维护请求成功、明确报错、响应超时但操作已生效；发送次数保持为一次，恢复结果独立记录。
- WebSocket 断线恢复、畸形消息、取消、鉴权终止、心跳、队列溢出和消息大小限制。
- 日志空闲后继续接收，HTTP 存活探测失败重连、鉴权失败终止、探测期间接收和取消，以及周期数据流超时。
- HTTPS/WSS 的附加 CA、默认不信任测试证书、主机名校验。
- 连接配置持久化、原子替换、选择与删除、schema 校验和 Unix 文件权限。

## 实际核心验证

测试自行创建临时核心进程、独立目录、控制端口、代理集、规则集、UDP DNS 服务及 HTTP 测试目标。
验证了版本、节点、代理集、规则集、规则、连接快照，节点选择、模式修改、
代理集/规则集更新、健康检查、节点及组延迟、规则禁用状态、缓存清理、
配置重载，以及速率/内存/连接 WebSocket 数据。
DNS 查询得到本地服务返回的 `127.0.0.42`；HTTP 订阅的 `expire=-1` 与其他代理集一起正确读取。
日志流在 `silent` 级别持续空闲八个心跳周期，随后正常取消。
会话维护流程还验证了缓存清理后的完整快照刷新；在 Linux 上重启测试自己创建的核心，
随后读取全部快照并重建速率订阅。该平台的核心通过 `exec` 保留 PID，测试继续使用原
子进程句柄清理。其他平台的重启联调需要能跟踪替代子进程的测试设施。

联调发现规则 `size` 的缺省值为 `-1`，已修正模型并加入回归用例。
延迟测试的本地目标保留 10ms 响应时间，以符合核心将 0ms 结果视为失败的行为。
依据：[规则响应](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/rules.go)、
[延迟接口](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/proxies.go)。
DNS 和订阅字段分别对照
[DNS 响应](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/dns.go)与
[订阅模型](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/adapter/provider/subscription_info.go)验证；
日志存活检测依据[核心日志处理器](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/server.go)实现。

完整本地回归命令：

```sh
MIHOMO_TEST_BIN=/usr/bin/mihomo cargo test -p nekodash-core --locked -- --include-ignored
```

测试结束会等待自己启动的进程退出，并释放临时资源。

## 首轮 UI 验证（2026-09-28）

- 应用状态测试 9 项：统计增量与计数回退、关闭历史保留、策略组连接范围、图表边界、数值排序、设置校验与原子读写、测速地址选择。
- 工作区共 52 项常规测试通过；格式、Clippy 和 Slint 静态检查通过。
- Linux 调试应用实际运行，Slint MCP 无窗口渲染；桌面 1280×820、窄屏 390×844。
- 两种尺寸都验证了错误 Secret 提示及重试、实时连接、七页导航、规则开关，以及通过表格关闭一条真实回环测试连接。
- 桌面额外验证节点选择，并从核心 API 确认已选节点。测试使用独立 Mihomo `1.19.31`。
- 截图已逐页检查；回归脚本和执行方法见 [UI 实现与对照](ui.md)。
- 完整应用库的 Android arm64、Windows x64 GNU 交叉编译检查通过。Android 使用 NDK `29.0.13113456`、API 28。

截图测试覆盖当前流程；完整上游交互对照和各平台原生运行继续按 UI 对照表推进。

## 后续平台验证

`.github/workflows/core.yml` 已配置 Linux、Windows MSVC、macOS arm64、macOS Intel
的工作区原生测试及 Android arm64 应用库编译检查。远端 CI 结果待仓库发布后记录。
Windows、macOS 和 Android 的设备运行验证随应用界面与打包阶段进行。
Linux Wayland/X11 的视觉、输入及性能验证也属于界面阶段。
