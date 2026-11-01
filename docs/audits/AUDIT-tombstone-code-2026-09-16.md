# AUDIT：墓碑代码全仓扫描

- **日期**：2026-09-16
- **范围**：全仓（Rust 后端 `src-tauri/**`、TS 前端 `src/**`、脚本 `scripts/**`、端到端 `e2e/**`）
- **类别维度**：① 从未被调用的死代码 ② 被注释掉的旧代码块 ③ 遗留临时/调试代码 ④ 已标记废弃未删除
- **处置分类**：✅ **删除** ｜ 🚀 **实现并接入** ｜ ⚠️ **技术债（单列）**
- **方法**：`cargo check --workspace`（dead_code/unused 告警）+ `tsc --noEmit`（noUnusedLocals/noUnusedParameters）+ 全仓 Grep 静态扫描 + 50 处 `#[allow(dead_code)]`/`#[allow(unused_*)` 掩码点逐一交叉引用审查
- **前置纪律**：`AGENTS.md` 铁律 12（删除前确认零引用）、铁律 13（DTO 命名）、Harness 分层约束

## 结论摘要

项目整体纪律良好（大量 `///` 命中为文档/用例，均被排除；`tsc` 全绿、`cargo check` 无 live 告警）。真正可"绝对安全"删除的墓碑**极少**；绝大多数 `#[allow(dead_code)]` 掩码点为**伪活**——分两种本质不同的情况：

- **应删除**：零引用死代码 / 纯调试残留 / 过时注解 / 空占位模块。
- **应实现并接入**：带"P2 接入后消费 / M2 接入 / 迁移完成前保留 / 待接入"显式注释的**未完成功能入口**——它们不是"该保留的死代码"，而是 TODO，正确处置是把功能**接入生效**（消除死态），而非维持现状。**纳入接入计划，删除会破坏演进契约。**

### 处置统计

| 处置 | 条目 | 说明 |
|---|---|---|
| ✅ 删除（已执行） | 9 | 空占位、纯调试、被注释旧代码、零引用字段 |
| ✅ 删除（待确认） | 2 | deprecated 闭环、e2e 桩改 skip |
| 🚀 实现并接入 | 6 | 均带未来接入/迁移注释的伪活功能入口 |
| ⚠️ 技术债 | 4 | 反模式 / 降级兜底 / 契约字段，不删但需处理 |

---

## ✅ 一、删除（死代码，清理掉）

### 1. 空占位模块 `src-tauri/src/util.rs`（已删）
- 定位：[util.rs](file:///d:/OneManager/AxInvest/src-tauri/src/util.rs) 全文仅 SPDX + "预留：通用工具模块占位"。
- 证据：全仓无任何 `util::` 引用；唯一附着点为 [lib.rs:50](file:///d:/OneManager/AxInvest/src-tauri/src/lib.rs#L50) 的 `mod util;`。
- 动作：删除 `util.rs` + 同步删除 `lib.rs` 的 `mod util;`。**✅ 已完成**

### 2. 从未被读取的私有字段 `mootdx.rs:416 QuoteResult.market`（已删）
- 定位：[mootdx.rs:416](file:///d:/OneManager/AxInvest/src-tauri/crates/astock-data/src/vendors/mootdx.rs#L416)。
- 证据：私有结构体字段全仓零读取（tests 的 `market` 是另一类型）。
- 动作：删除字段及其构造赋值，字节推进改 `_market`。**✅ 已完成**

### 3. `[DQ]` 一次性排查诊断日志群 16 处 `stockAnalysisStore.ts`（已删）
- 定位：[stockAnalysisStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/stockAnalysisStore.ts)。
- 证据：注释自述"必须打印/用于确认路径"，横跨 `loadAnalysis` 全链路，为一次性排查残留。
- 动作：删除全部 `[DQ]` `console.log/debug` 及前导注释。**✅ 已完成**

### 4. `WorkflowEditor.tsx` 误用 `console.warn` 当调试日志（已删）
- 定位：[WorkflowEditor.tsx](file:///d:/OneManager/AxInvest/src/components/workflow/WorkflowEditor.tsx)。
- 证据：成功路径也打 `warn`（含 Layout effect / 模板加载 trigger），非真实告警，纯调试。与 `applyAiChatAction`（:307 原）被注释同处。
- 动作：删除 `console.warn` 调试行；保留 `console.error` 失败路径；删除 `applyAiChatAction` 注释行。**✅ 已完成**

### 5. 被注释的旧 `drawdowns` 计算 `EquityCurveChart.tsx:42`（已删）
- 定位：[EquityCurveChart.tsx:42](file:///d:/OneManager/AxInvest/src/components/quant/charts/EquityCurveChart.tsx#L42)。
- 证据：回撤展示已不绘制，该行被注释保留。
- 动作：删除注释行。**✅ 已完成**

### 6. `src/lib.rs` 注释掉的 `init_concept_index`（已删）
- 定位：[lib.rs](file:///d:/OneManager/AxInvest/src-tauri/src/lib.rs)。
- 证据：注释注明随清理移除。
- 动作：删除注释片段。**✅ 已完成**

### 7. 被注释的旧调用 `src/lib/workers/index.ts:8-9`（已删）
- 定位：[index.ts:8-9](file:///d:/OneManager/AxInvest/src/lib/workers/index.ts#L8)。
- 证据：`estimateTokensAsync`/`processMarkdownAsync` 调用被注释。
- 动作：删除注释行。**✅ 已完成**

### 8. 调试 log `InvoicesTab.tsx:153` / `ChatInspector.tsx:63`（已删）
- 定位：[InvoicesTab.tsx:153](file:///d:/OneManager/AxInvest/src/pages/opc/components/InvoicesTab.tsx#L153)、[ChatInspector.tsx:63](file:///d:/OneManager/AxInvest/src/components/chat/ChatInspector.tsx#L63)。
- 动作：删除单行 `console.log`；同步将 `handleSelectBranch` 未用参数改 `_branchId`（避免 noUnusedParameters 回归）。**✅ 已完成**

### 9. `crypto.rs:293` `#[deprecated] decrypt_backup_key_v1`（待确认闭环）
- 定位：[crypto.rs:293](file:///d:/OneManager/AxInvest/src-tauri/crates/crypto/src/crypto.rs#L293)。
- 说明：v1 KDF 不安全，仅读旧备份保留。
- 动作：确认是否存在调用方走 `allow(deprecated)`；**无调用方则删除 + 同步删前端对应的 v1 解密分支**，有则保留并标注解锁日。

### 10. 未完成实现桩（待处理）
- 定位：`e2e/workflow-editor.spec.ts:98,105`（`throw TODO`）、`scripts/type-consistency.mjs:787`（TODO Search provider）。
- 动作：e2e 桩改 `test.skip`（保留待实现提示）；mjs TODO 为文档注释可保留。

---

## 🚀 二、实现并接入（未完成的功能入口，应接入生效，勿删）

> 共性判断：这些点**不是"该保留的死代码"**，而是带"P2 / M2 / 迁移完成前 / 待接入"注释的**功能缺口**。正确处置是排期把功能**接入正式流程**，让字段/变体/executor 真正被消费，从而消除死态。删除等于放弃声明中的能力。

### 1. `domain_pack.rs:54/139/172/184/214`（P2 接入后消费）
- 定位：[domain_pack.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/analysis-engine/src/opc/domain_pack.rs)，5 处字段级 `#[allow(dead_code)]`。
- 注释明确"P2 接入后消费"、被 `opc_data.rs` 引用。→ 接入分析引擎的数据包消费链路。

### 2. `app_state.rs:443` infra / gateway_state（迁移完成前保留）
- 定位：[app_state.rs:443](file:///d:/OneManager/AxInvest/src-tauri/src/app_state.rs#L443)。
- 注释"迁移完成前保留"。→ 挂到 P1 Task 3.1 迁移清单，迁移完成即消费或清理。

### 3. `runtime` 三字段（StatsD / 会话复用 / DB 驱动待接入）
- 定位：[gateway_metrics.rs:71](file:///d:/OneManager/AxInvest/src-tauri/crates/runtime/src/gateway_metrics.rs#L71)（StatsD 导出）、[connection_pool.rs:261](file:///d:/OneManager/AxInvest/src-tauri/crates/runtime/src/connection_pool.rs#L261)（会话级复用）、[health_check.rs:195](file:///d:/OneManager/AxInvest/src-tauri/crates/runtime/src/health_check.rs#L195)（DB 驱动）。
- 三处均为显式"待上游接入"预留。→ 接入对应特性后生效。

### 4. `quant/lib.rs:43` `kline_provider` 模块（M2 接入）
- 定位：[lib.rs:43](file:///d:/OneManager/AxInvest/src-tauri/crates/quant/src/lib.rs#L43)。
- 私有 mod 未 re-export，注释留待 M2 接入。→ M2 排期接入或明确放弃。

### 5. `app_state.rs:409` `persistent_runner` 占位 executor
- 定位：[app_state.rs:409](file:///d:/OneManager/AxInvest/src-tauri/src/app_state.rs#L409-412)。
- 闭包恒 `Err("not implemented")`，但字段为可配置占位（`enabled:false` 默认零开销）。
- → 实现真正的 SessionManager 适配器让 `start_background_services` 的守护循环可用，或从中移除。

### 6. `pattern_analyzer.rs:31` `ModuleStructure`/`ErrorHandling` variant
- 定位：[pattern_analyzer.rs:31](file:///d:/OneManager/AxInvest/src-tauri/crates/trajectory/src/pattern_analyzer.rs#L31)。
- 注释明示"功能未实现占位"（补提取逻辑时消费）。→ 补模式提取逻辑接入，或决策放弃该分析维度后删除。

---

## ⚠️ 三、技术债（非死代码，但需处理）

### 1. `CommandSuggest.tsx:232` mutating 组件原型逃逸
- 定位：[CommandSuggest.tsx:232](file:///d:/OneManager/AxInvest/src/components/chat/CommandSuggest.tsx#L232-238)。
- `FIXME` 标注反模式（直接改 React 组件原型暴露方法），被实际使用。→ 改共享 module 级 ref/context。

### 2. `evolutionStore.ts` "failed, using mock" 降级路径
- 定位：[evolutionStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/evolutionStore.ts)。
- 引擎停用后走 mock 兜底。→ 确认 `evolution`/`engine_mock` 后端是否停用；停用则删引擎，保留则删 mock 降级。

### 3. `executionStore.ts:898` `@deprecated` 旧版文本字段 + stackTrace warn
- 定位：[executionStore.ts:898](file:///d:/OneManager/AxInvest/src/stores/feature/executionStore.ts#L898)。
- 字段"仅展示兜底，不参与判定"，删除需同步展示消费方。→ 确认无展示方后删字段，否则换 logger。

### 4. 跨端 DTO 契约字段（勿删，Trace 用）
- 定位：[database.rs:8](file:///d:/OneManager/AxInvest/src-tauri/src/init/database.rs#L8)、[push_notification.rs:44](file:///d:/OneManager/AxInvest/src-tauri/crates/tools/src/tools/push_notification.rs#L44)、[stock_analysis.rs:58](file:///d:/OneManager/AxInvest/src-tauri/src/commands/stock_analysis.rs#L58)。
- 前端/跨 cfg 仍消费，属契约字段，删除会断契约。→ 保持，仅登记。

### 5. `apply_patch.rs:32,44` 过时 `#[allow(dead_code)]`
- 定位：[apply_patch.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/tools/src/tools/apply_patch.rs#L32)。
- `Hunk`/`DiffLine` 类型被使用，但 `Hunk.new_start`/`new_count` 仅构造未读。删 annotation 会暴露字段告警。→ 字段级针对性处理并确认 `new_*` 无未来校验场景后收敛。

---

## 关键负向结论（避免误删）

- 掩码点**绝大多数为伪活**：`state/agent.rs:28`（经 `Arc<AgentState>` 被 AppState 持有）、`memory.rs:10`（外部 10+ 处引用）、`trainer.rs:168`（仅 `mut` 冗余非死）、`_variant_tripwire:534`（编译穷尽守卫）、`screenshot_diagnosis.rs:476`（移动端编译守卫）等。**不是墓碑，勿删**。
- `stock_analysis_setup/mod.rs` 无死代码掩码；AGENTS.md 所述 `items_after_test_module` 是**文件结构问题**，非 dead code。
- 构建/测试内 `eprintln!`（`schema-gen/build.rs`、`hacker_news_scanner.rs:301`、`arxiv_scanner.rs:395`、`portfolio_monitor.rs:850`、`win_sandbox.rs:658` 等）多在 `#[cfg(test)]` 内或属生产请求日志，**合规豁免，保留**。

## 验证

- **✅ 已通过**：`cargo check --workspace`（退出0）+ 主 crate + `axagent-astock-data`；前端 `npm run typecheck`（退出0）。
- **⚠️ 待办**：提交前跑 `cargo clippy --workspace --all-targets -- -D warnings` 三门禁复核 Rust 相关改动。