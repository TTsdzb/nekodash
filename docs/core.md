# 通信库

`nekodash-core` 在调用者提供的 Tokio runtime 上运行。runtime 需要启用 I/O 和定时器。
界面将网络工作交给后台 runtime，通过 Slint 事件循环接收结果。

## 连接和请求

使用 `Endpoint::new(id, label, url, secret)` 创建连接配置，再构造 `CoreClient`。
URL 支持 HTTP、HTTPS、IPv4、IPv6 和反向代理路径前缀。控制连接直连目标地址。

`version`、`config`、`proxies`、`proxy_providers`、`rules`、`rule_providers`、
`connections` 返回结构化数据。运行配置保存全部字段，节点等模型保留扩展字段。
规则列表兼容数组与以数字索引为键的对象，`size` 保留核心的有符号数值。
订阅流量及到期字段同样保留有符号数值。`dns_query` 读取核心的 `Status` 字段，
返回 DNS 状态码及可选回答记录。

写接口对应 [协议清单](upstream-baseline.md)。`patch_config` 接受字段映射；
`load_config` 提交配置文本；`load_config_url` 下载配置后提交。
配置下载使用独立客户端，保留源地址的查询参数并跟随有限次数的重定向。
控制请求的鉴权限定在核心连接中。

`Probe` 指定节点、可选代理集、测试 URL 与核心超时。`batch_tests` 返回 `BatchTest`，
每次 `recv` 产生一项 `Completed`，包含原始输入索引、节点结果、完成数量及总数。
最后产生一次 `Finished`，包含完成数量及取消状态。输入索引用于区分同名节点。
批次限制并发数量，由后台异步任务持续读取事件推进；释放批次会丢弃其在途请求。

`test_batch` 汇总为 `BatchOutcome`，其中 `results` 保留取消前已完成的结果，
`cancelled` 表示是否提前结束。调用者可以通过 `CancellationToken` 或批次的 `cancel`
取消操作。客户端测试超时覆盖核心预算及网络余量。

`MaintenanceAction` 提供缓存清理、GEO 更新、重启、核心更新及托管面板更新。
收到超时后应查询实际状态再决定后续操作，因为服务器可能已经完成该操作。

## 实时订阅与会话

`subscribe` 支持连接、速率、内存和日志流，返回拥有后台任务的 `Subscription`。
日志可以指定级别。HTTP 与 WebSocket 使用相同的 TLS 信任配置；
私有控制器证书可通过 `ClientOptions::additional_ca_pem` 添加。

订阅提供连接状态、数据和结构化错误。传输断开后按上限退避重连，鉴权失败或
接口不可用时结束该订阅。速率、内存和连接流在连续两个心跳周期未收到帧时重连。
日志流在空闲时通过带鉴权的 `/version` 请求检查核心可达性，沿用 HTTP 的超时与
TLS 配置；检查期间继续接收日志并响应取消。收到新帧后取消正在进行的检查。
单条消息及队列均有大小限制，消费者落后时收到 `Lagged` 错误与丢失事件数量。
`cancel` 停止订阅，释放对象会结束其后台任务。

`Session::switch` 创建新会话并取消此前会话。`RequestContext::run` 返回
`SessionEvent<T> { token, result }`，成功、错误和取消结果均携带会话标识。
`RequestContext::subscribe` 和 `batch_tests` 返回的句柄也为接收事件添加同一标识，
包括建立订阅失败、解码错误及队列溢出。切换时，旧请求、订阅和测速批次一同取消。

界面在事件实际进入 UI 线程后，先调用 `Session::accepts(&event.token)`，再处理
`event.result`，以处理已经排入界面队列的旧回调。同一实例重新连接也会生成新的标识。

## 恢复与维护操作

断线恢复、应用回到前台或网络变化时，应用调用 `Session::recover`。它同步更新会话标识、
重建 HTTP 客户端并取消旧会话，然后返回可交给后台 runtime 执行的 `RecoveryOperation`。
应用应合并同一轮断线中多个订阅触发的恢复请求。

`RecoveryOperation::run` 在 `RecoveryOptions` 指定的时间内等待核心可达，默认每 500ms
检查一次版本，持续稳定 1 秒后读取配置、节点、代理集、规则、规则集和连接快照。
网络及服务暂时失败时继续检查；鉴权失败及时结束。各资源的能力或解析错误分别保存在
`CoreSnapshot`，`is_complete` 表示是否所有资源都读取成功。

调用顺序如下：

1. 创建恢复操作，克隆 `operation.context()`，界面进入正在连接状态。
2. 在后台执行 `operation.run().await`，将完整 `SessionEvent<RecoveryReport>` 发给界面。
3. 检查会话标识，处理 `report.snapshot`，应用新快照。
4. 使用该 context 的 `subscribe` 重建页面需要的订阅，继续检查每个事件的标识。

`Session::maintain(action, options)` 使用相同流程，并在检查可达性前发送一次维护请求。
`RecoveryReport.command` 保留该请求的响应结果：

| 结果 | 含义 |
| --- | --- |
| `Acknowledged` | 核心返回成功响应 |
| `ReportedFailure` | 核心明确返回错误响应，保留状态码与错误信息 |
| `Indeterminate` | 传输、等待或取消使结果无法确认，核心可能已经执行操作 |

`report.snapshot` 单独表示后续读取结果。核心可达性与维护操作的执行结果分别展示；
是否完成升级等具体操作，需要结合核心后续状态判断。`command_timeout` 控制等待响应的
时间，`timeout` 控制随后恢复的时间预算。切换会话会取消整个流程，已知的请求结果仍保留
在带原会话标识的报告中。协议依据见
[重启接口](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/restart.go)及
[升级接口](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/hub/route/upgrade.go)。

## 错误与持久化

错误含类别、操作名、HTTP 状态及脱敏说明，便于界面本地化及选择恢复动作。
JSON 解析失败保留行列与类别；HTTP 失败保留有限长度、经过核心密钥脱敏的消息。
客户端限制响应体大小。GET 成功响应仍需通过模型解析，写操作接受空的成功响应。

`EndpointStore` 支持增加、更新、选择和删除连接。存储包含 schema 版本；
加载时验证端点与引用关系。写入使用同目录临时文件、同步及原子替换。
调用者应在后台线程使用应用的用户数据目录。Unix 文件权限为 `0600`，
其他平台继承用户数据目录的访问控制。文件包含连接密钥，应按应用私有设置处理。

## 只读检查示例

在运行环境中设置 `NEKODASH_ENDPOINT` 与 `NEKODASH_SECRET` 后执行：

```sh
cargo run -p nekodash-core --example inspect --locked
```

示例演示恢复会话、读取快照、检查事件标识，再接收一条速率数据。

## 隔离核心测试

```sh
MIHOMO_TEST_BIN=/usr/bin/mihomo cargo test -p nekodash-core --locked \
  --test live_mihomo -- --ignored --nocapture
```

测试创建独立临时目录、配置和回环控制端口，延迟测试的目标也由本地测试服务提供。
结束时回收自己创建的核心进程。常规测试使用本地模拟服务，可直接运行：

```sh
cargo test -p nekodash-core --locked
```
