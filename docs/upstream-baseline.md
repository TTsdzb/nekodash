# 上游基线

## 固定版本

- 项目：[MetaCubeX/metacubexd](https://github.com/MetaCubeX/metacubexd)
- Tag：[`v1.273.1`](https://github.com/MetaCubeX/metacubexd/releases/tag/v1.273.1)
- 提交：`8bbc8f58fef71148a94fb5c0ff808f79b057337d`
- 登记日期：2026-09-27
- 参考形态：浏览器面板连接已有核心。
- 核心兼容目标：MetaCubeX 官方 Mihomo。
- 验收：页面布局、信息内容、功能和交互；主题采用 Nekodash 的配色。

## 核心通信清单

下表以固定提交的 `packages/ui/composables/useApi.ts`、`useQueries.ts`、
`useWebSocket.ts`、`useBatchLatencyTest.ts`、`useReverseDns.ts` 和
`packages/ui/pages/config.vue` 为依据。源码引用均应使用上面的提交。

| 功能 | 核心协议 |
| --- | --- |
| 版本与连接检查 | `GET /version` |
| 读取、修改运行配置 | `GET /configs`、`PATCH /configs` |
| 重载与加载配置文本 | `PUT /configs?force=true`，`{path, payload}` |
| 节点、代理组与选择 | `GET /proxies`、`PUT /proxies/:name` |
| 自动组恢复自动选择 | `DELETE /proxies/:name` |
| 节点、组延迟测试 | `GET /proxies/:name/delay`、`GET /group/:name/delay` |
| 代理集查询、更新、健康检查 | `GET /providers/proxies`、`PUT /providers/proxies/:name`、`GET /providers/proxies/:name/healthcheck` |
| 代理集内指定节点测试 | `GET /providers/proxies/:provider/:node/healthcheck` |
| 规则、规则集与更新 | `GET /rules`、`GET /providers/rules`、`PUT /providers/rules/:name` |
| 规则启用状态 | `PATCH /rules/disable` |
| 连接快照与关闭连接 | `GET /connections`、`DELETE /connections`、`DELETE /connections/:id` |
| DNS 查询与反向查询 | `GET /dns/query?name=…&type=…` |
| DNS、Fake IP 缓存清理 | `POST /cache/dns/flush`、`POST /cache/fakeip/flush` |
| GEO 数据更新 | `POST /configs/geo` |
| 核心重启、更新及其托管面板更新 | `POST /restart`、`POST /upgrade`、`POST /upgrade/ui` |
| 实时连接、速率、内存与日志 | WebSocket `/connections`、`/traffic`、`/memory`、`/logs?level=…` |

节点名、组名、代理集名按单个 URL 路径段编码。HTTP 鉴权使用 Bearer；
WebSocket 使用 `token` 查询参数。基础 URL 可以包含反向代理路径前缀。
每个连接的请求与数据流独立，切换连接时清理旧订阅并隔离延迟到达的结果。

## 界面与应用功能跟踪

后续界面阶段逐项对照：连接配置；概览与统计；节点和代理集；延迟及可达性测试；
连接表、搜索筛选和关闭操作；规则及规则集；日志；运行配置；DNS 工具；
设置与备份；语言、快捷键和移动端布局。各项验收记录包含参考入口、操作步骤、
预期结果、实际结果与截图。复杂功能以固定版本实际可见的入口及能力条件为准。

## 同步上游

1. 记录新 tag 与完整提交，保留此前基线及验证记录。
2. 比较两个提交的面板页面、组件、状态、接口、语言资源与测试。
3. 将变动映射到此清单及对应 Rust/Slint 模块，区分功能、交互和协议变化。
4. 先更新协议测试及通信实现，再更新界面与对照记录。
5. 分阶段提交；完成验证后更新基线。
