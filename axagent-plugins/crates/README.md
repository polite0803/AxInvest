# axagent-plugins 宿主侧 crate 与独立插件仓

> 主仓只保留 4 个 workspace 成员：`plugins`（宿主运行时）、`schema-gen`（构建工具）、
> `plugin-opc-core` 与 `plugin-testkit`（共享 SDK）。17 个插件统一 worker + `v3.0.1`；
> 可交付插件源码的权威位置是主仓外的 17 个独立 Git 仓。本目录中同名的可交付 crate 由
> `axagent-plugins/Cargo.toml` 的 `exclude` 隔离，只作生成模板源，不参与主仓 Cargo workspace，
> 也不得直接作为发布源；模板源由 `scripts/generate-opc-worker-source.mjs` 生成并逐字节校验。

> 本目录下 crate 都属「插件层」。harness 的那套角色（foundation / consumer / implementor /
> hybrid / wiring）判的是 `axagent-*` 库 crate 之间的依赖方向；**插件侧可执行**不在其中，
> 单列一行，判据是「只依赖 foundation + 协议 crate，不反向依赖主壳或宿主运行时」。

| crate                           | 角色                                      | 允许依赖                                           | 说明                                                                                                                      |
| ------------------------------- | ----------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `plugins`                       | 宿主侧插件运行时                          | harness + 各 implementor（wiring 口径）            | 清单发现、启停、长驻 worker 管理、T2 命令/接缝分派                                                                        |
| `stock-core`                    | **独立仓内 foundation**                   | `axagent-harness` 契约 + 纯数据工具                | 股票域纯函数；权威副本随 `stock` 独立仓发布，主仓同名目录仅为历史隔离副本                                                 |
| `plugin-stock`                  | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-stock-core`      | 股票域 worker；发布源为 `AxInvest-plugin-repos/stock`                                                                     |
| `plugin-opc`                    | **独立仓内插件侧可执行**                  | `axagent-plugin-proto`（协议 crate）               | OPC 公共 worker；发布源为 `AxInvest-plugin-repos/opc`                                                                     |
| `plugin-opc-core`               | **foundation**（零 `axagent-*` 业务依赖） | `axagent-plugin-proto` + `serde_json`              | OPC 域包共享命令实现：泛型 `Peer<R, W>` 的 `opc.store` helper 与已迁域包命令，公共 worker 与各域 worker 复用              |
| `plugin-opc-accounting`         | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | accounting 域包 worker；发布源为 `AxInvest-plugin-repos/opc-accounting`                                                   |
| `plugin-opc-sales-growth`       | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | sales_growth 域包 worker；发布源为 `AxInvest-plugin-repos/opc-sales-growth`                                               |
| `plugin-opc-content-media`      | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | content_media 域包 worker；发布源为 `AxInvest-plugin-repos/opc-content-media`                                             |
| `plugin-opc-game-dev`           | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | game_dev 域包 worker；完整命令集试点，发布源为 `AxInvest-plugin-repos/opc-game-dev`                                       |
| `plugin-opc-security`           | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | security 域包 worker；发布源为 `AxInvest-plugin-repos/opc-security`                                                       |
| `plugin-opc-ai-research`        | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | ai_research 域包 worker；发布源为 `AxInvest-plugin-repos/opc-ai-research`                                                 |
| `plugin-opc-consulting`         | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | consulting 域包 worker；发布源为 `AxInvest-plugin-repos/opc-consulting`                                                   |
| `plugin-opc-design`             | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | design 域包 worker；发布源为 `AxInvest-plugin-repos/opc-design`                                                           |
| `plugin-opc-ecommerce`          | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | ecommerce 域包 worker；发布源为 `AxInvest-plugin-repos/opc-ecommerce`                                                     |
| `plugin-opc-education`          | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | education 域包 worker；发布源为 `AxInvest-plugin-repos/opc-education`                                                     |
| `plugin-opc-finance-invest`     | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | finance_invest 域包 worker；另有 `batch_validate_entities` 读 op，发布源为 `AxInvest-plugin-repos/opc-finance-invest`     |
| `plugin-opc-geospatial`         | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | geospatial 域包 worker；发布源为 `AxInvest-plugin-repos/opc-geospatial`                                                   |
| `plugin-opc-project-management` | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | project_management 域包 worker；发布源为 `AxInvest-plugin-repos/opc-project-management`                                   |
| `plugin-opc-software-dev`       | **独立仓内插件侧可执行**                  | `axagent-plugin-proto` + `axagent-plugin-opc-core` | software_dev 域包 worker；另有 `batch_validate_entities` 读 op，发布源为 `AxInvest-plugin-repos/opc-software-dev`         |
| `plugin-wiki`                   | **独立仓内插件侧可执行**                  | `axagent-plugin-proto`                             | Wiki worker；发布源为 `AxInvest-plugin-repos/wiki`                                                                        |
| `plugin-testkit`                | **SDK 发布物 / foundation**               | `axagent-plugin-proto` + `serde_json`              | 插件作者侧契约快照与测试夹具：构造 describe / shutdown / command 帧，解码声明并断言命令回执；不启动进程、不依赖宿主运行时 |
| `schema-gen`                    | 构建期工具                                | harness                                            | IPC 类型生成器；无运行时消费方                                                                                            |

`plugin-proto` 本体在 `axagent-core/crates/plugin-proto`（foundation，零 `axagent-*` 依赖），
是宿主与插件 worker 之间帧协议的**唯一权威**：两侧都 `use` 同一份类型，不得各写一套 JSON 形状。
