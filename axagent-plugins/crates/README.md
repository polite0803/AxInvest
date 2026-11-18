# axagent-plugins workspace 成员与角色

> 本目录下 crate 都属「插件层」。harness 的那套角色（foundation / consumer / implementor /
> hybrid / wiring）判的是 `axagent-*` 库 crate 之间的依赖方向；**插件侧可执行**不在其中，
> 单列一行，判据是「只依赖 foundation + 协议 crate，不反向依赖主壳或宿主运行时」。

| crate                      | 角色                                      | 允许依赖                                           | 说明                                                                                                                                                                             |
| -------------------------- | ----------------------------------------- | -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `plugins`                  | 宿主侧插件运行时                          | harness + 各 implementor（wiring 口径）            | 清单发现、启停、长驻 worker 管理、T2 命令/接缝分派                                                                                                                               |
| `stock-core`               | **foundation**（零 `axagent-*` 依赖）     | 仅 `serde`                                         | 股票域**纯函数**（意图解析等）。宿主与插件**过渡期共用同一份实现**：插件只能依赖 foundation，放 implementor 里会被迫把 dao/entities/astock-data 拖进插件进程。终局随插件独立仓走 |
| `plugin-stock`             | **插件侧可执行**                          | `axagent-plugin-proto` + `axagent-stock-core`      | 股票域 worker 进程（工具 `get_stock_quote`、命令 `stock_get_quote` / `parse_analysis_intent`）                                                                                   |
| `plugin-opc`               | **插件侧可执行**                          | `axagent-plugin-proto`（协议 crate）               | OPC 域 worker 进程，业务命令全部走 T2 命令帧 + `opc.store` 接缝回宿主；不持 `DatabaseConnection`，不直接依赖 `dao` / `entities`                                                  |
| `plugin-opc-core`          | **foundation**（零 `axagent-*` 业务依赖） | `axagent-plugin-proto` + `serde_json`              | OPC 域包共享命令实现：泛型 `Peer<R, W>` 的 `opc.store` helper 与已迁域包命令，公共 worker 与各域 worker 复用                                                                     |
| `plugin-opc-accounting`    | **插件侧可执行**                          | `axagent-plugin-proto` + `axagent-plugin-opc-core` | accounting 域包 worker，只承接 `opc_list_invoices` / `opc_create_invoice` / `opc_transition_invoice`                                                                             |
| `plugin-opc-sales-growth`  | **插件侧可执行**                          | `axagent-plugin-proto` + `axagent-plugin-opc-core` | sales_growth 域包 worker，只承接客户创建 / 落地页列表 / 联系人列表三条命令                                                                                                       |
| `plugin-opc-content-media` | **插件侧可执行**                          | `axagent-plugin-proto` + `axagent-plugin-opc-core` | content_media 域包 worker，承接博客 / 内容资产 / 发布计划命令                                                                                                                    |
| `plugin-wiki`              | **插件侧可执行**                          | `axagent-plugin-proto`                             | Wiki 业务功能包 worker（P6.3 第一片）：只承接 `wiki_sync_note_to_knowledge_base`，经宿主 `wiki.store` 接缝完成笔记同步；不持 DB 句柄、不读文件                                   |
| `plugin-testkit`           | **SDK 发布物 / foundation**               | `axagent-plugin-proto` + `serde_json`              | 插件作者侧契约快照与测试夹具：构造 describe / shutdown / command 帧，解码声明并断言命令回执；不启动进程、不依赖宿主运行时                                                        |
| `schema-gen`               | 构建期工具                                | harness                                            | IPC 类型生成器；无运行时消费方                                                                                                                                                   |

`plugin-proto` 本体在 `axagent-core/crates/plugin-proto`（foundation，零 `axagent-*` 依赖），
是宿主与插件 worker 之间帧协议的**唯一权威**：两侧都 `use` 同一份类型，不得各写一套 JSON 形状。
