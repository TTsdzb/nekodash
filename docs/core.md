# 通信库

`nekodash-core` 在调用者提供的 Tokio runtime 上运行。runtime 需要启用 I/O 和定时器。
界面将网络工作交给后台 runtime，通过 Slint 事件循环接收结果。

## 连接和请求

使用 `Endpoint::new(id, label, url, secret)` 创建连接配置，再构造 `CoreClient`。
URL 支持 HTTP、HTTPS、IPv4、IPv6 和反向代理路径前缀。控制连接直连目标地址。

`version`、`config`、`proxies`、`proxy_providers`、`rules`、`rule_providers`、
`connections` 返回结构化数据。运行配置保存全部字段，节点等模型保留扩展字段。
规则列表兼容数组与以数字索引为键的对象，`size` 保留核心的有符号数值。

写接口对应 [协议清单](upstream-baseline.md)。`patch_config` 接受字段映射；
`load_config` 提交配置文本；`load_config_url` 下载配置后提交。
配置下载使用独立客户端，保留源地址的查询参数并跟随有限次数的重定向。
控制请求的鉴权限定在核心连接中。

`Probe` 指定节点、可选代理集、测试 URL 与核心超时。`test_batch` 限制并发数量，
逐项保留测试结果或错误。调用者可以通过 `CancellationToken` 取消当前批次。
客户端测试超时覆盖核心预算及网络余量。

`MaintenanceAction` 提供缓存清理、GEO 更新、重启、核心更新及托管面板更新。
收到超时后应查询实际状态再决定后续操作，因为服务器可能已经完成该操作。

## 实时订阅与会话

`subscribe` 支持连接、速率、内存和日志流，返回拥有后台任务的 `Subscription`。
日志可以指定级别。HTTP 与 WebSocket 使用相同的 TLS 信任配置；
私有控制器证书可通过 `ClientOptions::additional_ca_pem` 添加。

订阅提供连接状态、数据和结构化错误。传输断开后按上限退避重连，鉴权失败或
接口不可用时结束该订阅。心跳用于发现无响应连接，空闲日志流通过 Ping/Pong 保活。
单条消息及队列均有大小限制，消费者落后时收到 `Lagged` 错误与丢失事件数量。
`cancel` 停止订阅，释放对象会结束其后台任务。

`Session::switch` 创建新会话并取消此前会话。`RequestContext::run` 把请求结果与
会话标识一起返回；界面应用结果前使用 `Session::accepts` 检查标识。
这样可处理已经排入界面事件队列、但在切换后才被执行的旧回调。
订阅使用会话中的取消令牌，切换时一同结束。

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

示例读取版本、节点及规则数量，再接收一条速率数据。

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
