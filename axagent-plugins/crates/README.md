# axagent-plugins workspace 成员与角色

> 本目录下 crate 都属「插件层」。harness 的那套角色（foundation / consumer / implementor /
> hybrid / wiring）判的是 `axagent-*` 库 crate 之间的依赖方向；**插件侧可执行**不在其中，
> 单列一行，判据是「只依赖 foundation + 协议 crate，不反向依赖主壳或宿主运行时」。

| crate | 角色 | 允许依赖 | 说明 |
|---|---|---|---|
| `plugins` | 宿主侧插件运行时 | harness + 各 implementor（wiring 口径） | 清单发现、启停、长驻 worker 管理、T2 命令/接缝分派 |
| `stock-core` | **foundation**（零 `axagent-*` 依赖） | 仅 `serde` | 股票域**纯函数**（意图解析等）。宿主与插件**过渡期共用同一份实现**：插件只能依赖 foundation，放 implementor 里会被迫把 dao/entities/astock-data 拖进插件进程。终局随插件独立仓走 |
| `plugin-stock` | **插件侧可执行** | `axagent-plugin-proto` + `axagent-stock-core` | 股票域 worker 进程（工具 `get_stock_quote`、命令 `stock_get_quote` / `parse_analysis_intent`） |
| `plugin-opc` | **插件侧可执行** | `axagent-plugin-proto` + `axagent-company-runtime`（OPC 逻辑所在地） | OPC 域 worker 进程。**#36 第 1 批仅骨架**：零业务命令、声明空表；不持 `DatabaseConnection`，不直接依赖 `dao` / `entities` |
| `schema-gen` | 构建期工具 | harness | IPC 类型生成器；无运行时消费方 |

`plugin-proto` 本体在 `axagent-core/crates/plugin-proto`（foundation，零 `axagent-*` 依赖），
是宿主与插件 worker 之间帧协议的**唯一权威**：两侧都 `use` 同一份类型，不得各写一套 JSON 形状。
