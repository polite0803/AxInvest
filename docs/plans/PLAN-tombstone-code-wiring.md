# PLAN：墓碑代码「实现并接入」任务清单

- **日期**：2026-09-17
- **来源**：`AUDIT-tombstone-code-2026-09-16.md` 🚀 二、实现并接入（6 条）
- **目标**：把 6 处带「P2/M2/迁移完成前/待接入」注释的**伪活掩码点**从「保留的死态」转为「接入生效」，消除 `#[allow(dead_code)]` 的同时兑现注释声明的功能承诺。
- **纪律**：每项完成后删除对应 `#[allow(dead_code)]`；遵守 `AGENTS.md` 铁律（分层、命令注册、DTO camelCase、删除前确认零引用）；提交前 `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace` 三门禁。

---

## 任务总览

| # | 位置 | 掩码 | 声明能力 | 排期 |
|---|---|---|---|---|
| T1 | `analysis-engine/opc/domain_pack.rs`（5 字段） | `#[allow(dead_code)]` | learning 配置文件名消费 | P2 |
| T2 | `src/app_state.rs` infra / gateway_state | `#[allow(dead_code)]` | 子状态分组访问器 | P1 Task 3.1 |
| T3 | `runtime/gateway_metrics.rs` | `#[allow(dead_code)]` | StatsD 推送导出 | P3 |
| T4 | `runtime/connection_pool.rs` | `#[allow(dead_code)]` | 会话级真实连接复用 | P2 |
| T5 | `runtime/health_check.rs` | `#[allow(dead_code)]` | DB 真实探活 | P3 |
| T6 | `quant/lib.rs` `kline_provider` | `#[allow(dead_code)]` | K线+Quote→Bar 转换 | M2 |
| T7 | `src/app_state.rs` `persistent_runner` | （占位闭包） | 持久化重试调度 | P2 |
| T8 | `trajectory/pattern_analyzer.rs` `PatternType` | `#[allow(dead_code)]` | ModuleStructure/ErrorHandling 提取 | P3 |

> 注：审计报告原「6 条」中 `app_state.rs` 拆出 T2/T7 两条（不同声明能力），合计 8 项。

> **实施状态（2026-09-17）**：T1 ✅ / T2 ✅ / T3 ✅ / T5 ✅ / T7 ✅ / T8 ✅ 已完成。
> **三门禁验证结果**：`cargo fmt --check` ✅、`cargo clippy --workspace --all-targets -- -D warnings` ✅（14 个 crate 全绿）、`cargo test` ⚠️ **10 项预存失败（非本次引入）**。
> T4 ⏳ 等待上游 MemoryProvider 运行时接入；T6 ⏳ 等待 M2 里程碑。

---

## ✅ T1. `domain_pack.rs` learning 配置字段接入

- **定位**：[domain_pack.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/analysis-engine/src/opc/domain_pack.rs#L52-56) `OpDomainPack.learning` 字段（含 `default_learning_file()` 默认值函数，同文件 :75）。
- **现状**：`learning` 字段仅在结构体定义 + 默认值函数中使用，无消费方。兄弟字段 `analysis`（→`load_domain_pack_analysis`）、`runtime`（→`load_domain_pack_runtime`）均有读取路径，唯 `learning` 无。
- **接入动作**：
  1. 实现 `learning` 配置的读取函数（参照 `load_domain_pack_runtime` 形态），供 `opc_domain_pack_actions::domain_pack_learning_config_path`（注释中已引用该路径）消费。
  2. 删除 `#[allow(dead_code)]`。
- **验收**：`learning.yaml` 存在时被加载；缺失时回退默认值；`cargo clippy` 无 dead_code 告警。
- **依赖**：需确认 `learning.yaml` 的 schema 与读取模块是否已存在（`opc_domain_pack_actions` 是否已有挂载点）。

## ✅ T2. `app_state.rs` 子状态分组访问器接入

- **定位**：[app_state.rs:437-447](file:///d:/OneManager/AxInvest/src-tauri/src/app_state.rs#L437-447)。
- **现状**：6 个子状态结构（infra/gateway_state/task/agent/memory/skill/learning/tool）启动时由顶层字段 clone 构造，注释「200+ commands/* 调用点保持原样，新代码可用分组访问器」。仅 `infra`/`gateway_state` 带 `#[allow(dead_code)]`，其余子状态可能已被新代码消费。
- **接入动作**：
  1. 定位 P1 Task 3.1 迁移进度：grep `app_state.\(infra\|gateway_state\)` 确认零新消费方。
  2. 若新代码已开始用分组访问器 → 在剩余高频调用点（commands/* 内重复 `state.xxx.clone()` 模式）改用 `app_state.infra.xxx` / `app_state.gateway_state.xxx`。
  3. 确认全部子状态均被引用后删 `#[allow(dead_code)]`。
- **验收**：两个字段各有 ≥1 处真实调用；clippy 无告警。
- **风险**：迁移面大（200+ 文件），建议按「新代码强制走访问器 + 存量逐步迁移」渐进，不追求一次性全量。

## ✅ T3. `gateway_metrics.rs` StatsD 推送导出

- **定位**：[gateway_metrics.rs:68-71](file:///d:/OneManager/AxInvest/src-tauri/crates/runtime/src/gateway_metrics.rs#L68-71) `MetricsCollector.config` 字段，注释「StatsD 推送导出（enable_statsd/statsd_host/port）待导出器实现后消费；collection_interval 由调用方调度循环使用」。
- **接入动作**：
  1. 在 `MetricsCollector` 增加 `push_statsd()`（或后台 task），读取 `config.enable_statsd/statsd_host/port`，将 counters/gauges/histograms/summaries 周期 push 到 StatsD（UDP，dogstatsd 或标准 statsd 协议）。
  2. 由现有调度循环（collection_interval）触发。
  3. 删除 `#[allow(dead_code)]`。
- **验收**：`enable_statsd=true` 时指标按 interval 推送；false 时零开销。
- **依赖**：需引入 statsd 客户端依赖或手写 UDP 发送（**优先复用**：grep 仓库是否已有 UDP/metrics 发送先例）。

## ⏳ T4. `connection_pool.rs` 会话级真实连接复用

- **定位**：[connection_pool.rs:257-264](file:///d:/OneManager/AxInvest/src-tauri/crates/runtime/src/connection_pool.rs#L257-264) `SessionPool.pool: ConnectionPool<C>` 字段，注释「会话级真实连接复用（pool.acquire 路径）需待上游会话提供方（MemoryProvider 运行时）接入后才产生消费方，当前仅固定 API 形状」。
- **接入动作**：
  1. 上游 MemoryProvider 运行时接入后，在 `SessionPool::acquire` 路径真实调用 `pool.acquire()` 复用连接，而非当前固定形状。
  2. 删除 `#[allow(dead_code)]`。
- **验收**：`acquire` 返回真实连接；无连接时按 `session_ttl` 清理。
- **依赖**：**阻塞于上游** MemoryProvider 运行时接入（独立任务），本项标记为「等待依赖」。

## ✅ T5. `health_check.rs` DB 真实探活

- **定位**：[health_check.rs:192-197](file:///d:/OneManager/AxInvest/src-tauri/crates/runtime/src/health_check.rs#L192-197) `DatabaseHealthCheck.db_url` 字段，注释「真实 DB 探活（TcpStream/sqlx 连接测试）需 DB 驱动依赖，runtime crate 未引入；当前 check() 返回恒定健康态」。
- **接入动作**：
  1. runtime crate 引入 DB 驱动（sqlx 或 axagent-dao 连接复用——**遵循 harness 分层**，runtime 属 wiring 可依赖 dao）或改为 TcpStream 探测（无新依赖、轻量）。
  2. `DatabaseHealthCheck::check()` 用 `db_url` 真实探测，失败返回 `HealthCheckResult::unhealthy`。
  3. 删除 `#[allow(dead_code)]`。
- **验收**：DB 不可达时健康检查返回 unhealthy；可达时 healthy。
- **权衡**：TcpStream 探测零依赖但只能验端口；sqlx 探测能验连接但引入依赖。默认**优先 TcpStream**（轻量），如需认证级探活再上 sqlx。

## ⏳ T6. `quant/lib.rs` kline_provider 接入

- **定位**：[crates/quant/src/lib.rs:41-44](file:///d:/OneManager/AxInvest/src-tauri/crates/quant/src/lib.rs#L41-44) `mod kline_provider`，注释「M2 基础设施（K 线 + Quote → Bar 转换），M1 阶段回测直接接收 Vec<Bar>，尚未在命令层使用」。
- **接入动作**：M2 排期：在命令层/回测入口接入 `kline_provider`，将 K 线 + Quote 转换为 Bar 后喂给回测引擎；删除 `#[allow(dead_code)]`。
- **验收**：命令层存在一条 K 线 → Bar → 回测 的调用链。
- **依赖**：M2 里程碑（M1 回测直接收 Vec<Bar>，本项是 M2 的转换层）。

## ✅ T7. `persistent_runner` 占位 executor 实现

- **定位**：[app_state.rs:409-412](file:///d:/OneManager/AxInvest/src-tauri/src/app_state.rs#L409-412) `persistent_runner` 字段，注释「当前 executor 闭包为占位实现（返回 `Err("not implemented")`），真正的 SessionManager 适配器需后续实现。启用配置后守护线程会运行，但实际执行会失败并记录 warn 日志」。
- **接入动作**：
  1. 实现 `spawn_daemon` 中 executor 闭包的 SessionManager 适配器：每 60 秒查 pending session，调用 SessionManager 真正调度执行。
  2. 若 `UnifiedConfig.persistent_runner.enabled` 为 false（默认）保持零开销不变。
  3. 无真实用例 → 与产品确认是否保留该能力；放弃则移除整个字段与守护循环。
- **验收**：enabled=true 时后台守护能调度 pending session 执行（非恒 Err）；false 时零开销。
- **风险**：涉及 SessionManager 生命周期，建议先写单测验证守护循环调度语义再接入真适配器。

## ✅ T8. `pattern_analyzer.rs` ModuleStructure/ErrorHandling 提取实现

- **定位**：[pattern_analyzer.rs:27-39](file:///d:/OneManager/AxInvest/src-tauri/crates/trajectory/src/pattern_analyzer.rs#L27-39) `PatternType` enum，注释「两种代码模式尚未实现提取逻辑：`extract_coding_patterns` 目前只产出 Naming / Indentation / Comment 三类。模块对外声明支持 5 类，补提取逻辑时直接消费这两个 variant 即可，勿删」。
- **接入动作**：
  1. 在 `extract_coding_patterns` 中补 `ModuleStructure`（模块/文件结构模式）与 `ErrorHandling`（错误处理模式）的提取逻辑。
  2. 删除 `#[allow(dead_code)]`。
- **验收**：5 类 pattern 均能产出；相关单测覆盖新增两类。
- **决策点**：若该分析维度被产品放弃 → 从 enum 与对外文档（文件头 + `CodingPatternSummary`）同步移除，删除即收敛。

---

## 执行顺序建议

1. **低风险先清**：T1（单字段、有兄弟范式可循）→ T5（独立、轻量）。
2. **依赖阻塞**：T4（等上游 MemoryProvider）、T6（等 M2 里程碑）——登记依赖，不阻塞其余。
3. **需决策**：T7（产品确认保留/放弃）、T8（确认 ModuleStructure/ErrorHandling 是否在产品范围）。
4. **渐进迁移**：T2（大面迁移）、T3（需协议选型）。

## 验收总则

- 每项完成后删除对应 `#[allow(dead_code)]` 且 `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- 无新增死代码掩码；新消费方走 harness trait / 既有调用路径，不重复造轮子。
- 跨端契约字段（`push_notification.rs:44`、`stock_analysis.rs:58`、`init/database.rs:8`）不在本清单内，维持现状。