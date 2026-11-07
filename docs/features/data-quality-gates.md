# 数据质量与门禁

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：data-quality-gates

## 定位

这条链路解决「分析结果看起来正常、其实是拿不到数据 / 上游分析师不可信 / 因子大量缺失」的静默失效：先由一个独立节点给每轮分析打**数据质量等级（DQI）**，再把等级折算成**仓位与置信度的硬约束**；同时用一层字段级校验拦住「自相矛盾的分析师结论」。仓库自身的静态门禁属于同一思路的工程侧延伸：**把「看着绿」变成「真的绿」**。

入口：股票分析工作流模板里的 `data-quality` 节点（脚本 `src-tauri/src/commands/data-quality.rhai`，经 `include_str!` 嵌入模板节点 `code`，见 `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:5570`），以及 `portfolio-mgr` 节点里的降级分支。

## 当前行为

### 1. DQI 分级与熔断

**分级**：`grade ∈ {A,B,C,D,F}`，由三项加权得分决定 —— 报告质量均值 ×0.35 + 工具可信度 ×0.35 + 因子完整度 ×100 ×0.30，阈值 **85 / 65 / 45 / 25**（`src-tauri/src/commands/data-quality.rhai:1624-1636`）。A 级意味着三项都优秀；因子完整度已计入总分，所以 A 级不再二次惩罚因子缺失（该设计理由见 `src-tauri/src/commands/portfolio-mgr.rhai:2784-2790`）。

**熔断（跨轮）**：观测写入 `data_quality_observations`，写侧只判一次「本轮是否异常」并落成 `abnormal` 列，读侧只数布尔，不重算等级（`axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:13-37`）。判据是：

- `abnormal = grade ∈ {C,D,F} 或 grade 为 NULL/空串`；A/B 都是正常档；未识别字母按异常处理（保守）（`axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:66-76`）；
- `DQI_FUSE_STREAK = 3`（含本轮），`DQI_STREAK_LOOKBACK = 12`（3 的 4 倍，容忍一次重跑/回补）（`:57`、`:64`）；
- `is_fused = consecutive_abnormal >= 3`（`:79-81`）。

**注入与生效**：运行变量注入 `dqi_streak` / `dqi_fuse_flag` / `dqi_observations` / `dqi_orphans`（`src-tauri/src/commands/stock_workflow/hooks.rs:525-543`）；取数失败时注入空计数且 `fuse_flag=0`——熔断是收紧动作，取不到证据不能当作「有问题」（`:513-523`）。注入发生在**本轮运行开始处**，不含本轮尚未落库的观测，所以界面显示的连续轮数会比落库后可数的少 1（`:509-512` 明确这是分层使然）。熔断后的直接效果是「决策可信度受限、本轮置信度上限按最保守档收紧」（`axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:23-25` 的模块说明）。

**F 级的三重后果**：在 `portfolio-mgr` 里，权重坍缩分支统一执行 `position_pct = 0.0`、`decision_confidence × 0.5`、`signal_strength × 0.5`（`src-tauri/src/commands/portfolio-mgr.rhai:2819-2824`），并记 `R-206`（F 级）/ `R-203`（权重坍缩）决策轨迹（`:2843-2853`）。注意：**代码不强制改写 `action` 字段**，只把仓位清零；「观望」是 0 仓位在展示层的读法，不是这里写入的枚举值。

（对应版本：v3.0.0；熔断引入 6f3c5f308，B 级判据收窄见 `axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:20-34`，孤儿观测修正 c6494fb8b）

### 2. 权重塌缩三档策略

判定输入是 `dqi_grade_str` 与 `untrusted_count`（上游 `strict_mode` 降级会注入 `__untrusted=true`，折算成 0–6 个不可信信号，`src-tauri/src/commands/portfolio-mgr.rhai:212-223`）。先按质量等级确定阈值：

| 档位 | 条件 | `total_weight / max_weight` 阈值 |
|---|---|---|
| A 级豁免 | `grade == "A"` 且 `untrusted_count == 0` | `0.0`（永不触发，且跳过 untrusted / F 判定） |
| B 级 | `grade == "B"`，或 A 级但存在 untrusted | `< 0.15` |
| C/D/F 级 | 其余 | `< 0.3` |

坍缩总判定：`!A级 && (权重比 < 阈值 || untrusted_count >= 2 || grade == "F")`（`src-tauri/src/commands/portfolio-mgr.rhai:2791-2811`）。

坍缩后的**三分支降级**（`:2813-2833`）：

- **分支 A**：`weights_collapsed` → 0% 仓位 + `confidence × 0.5` + `signal_strength × 0.5`（`R-206`/`R-203`）；
- **分支 B**：`untrusted_count == 1` 且 `posterior >= 0.70` → 仓位上限 30%，`confidence × 0.85`，保留买入/增持（`R-205`）；
- **分支 C**：`untrusted_count == 1` 且 `posterior < 0.70` → 禁止加仓，`confidence × 0.70`（`R-205`）。

所有乘法都经 `clamp(..., 0.0, 100.0)`，不会把置信度乘出 0–100 之外（`:2822-2832`）。

（对应版本：v3.0.0，当前实现；三档阈值与分支权重在该版本窗口内无独立提交可定位）

### 3. 字段级校验：自洽性、方向、量程

`data-quality.rhai` 的 `validate_verdict_schema` 是**纯检测、不改写上游数据**（`src-tauri/src/commands/data-quality.rhai:571-580` 明确记录了「此前名为 validate_and_fix 的自动修复从未生效、已删除」）。三项检查：

1. **`bull_score + bear_score ≈ 100`**：偏离 >10（即和 <90 或 >110）判「评分不自洽」；偏离 5–10 判「轻微偏离」；阈值与搬出前逐字一致（`src-tauri/src/commands/data-quality.rhai:556-568`）。
2. **`verdict` 与分数方向一致**：文本含「看多/看涨/偏多」但 `bull_score <= bear_score`，或含「看空/看跌/偏空」但 `bear_score <= bull_score`，判「逻辑矛盾」（`:530-546`）。
3. **`confidence` 量程 0–100**：`> 100` 报「置信度越界」；`0 < conf < 10` 另报「置信度极低」；负值不进入这两条检查（`:600-610`）。

处置强度不同：`bull+bear` 偏离只**提示**、不剥夺投票资格（它属于产出形态问题）；方向与分数互相打脸才升级为**不可信**（两个字段互相否定，下游任取其一都会读反）（`:521-529`）。「不可信」经 `diagnostics` / `diagnostics_by_tier` 透出到面板与反馈表。

（对应版本：v3.0.0，当前实现；字段校验收敛无独立提交可定位）

### 4. 数值字段覆写 vs 定性字段只校验

`value-verify.rhai` 是价值投资链的**唯一确定性出口**：只覆写**数值**字段（`intrinsic_value_range` / `margin_of_safety` / `ideal_buy_price`），`verdict` / `report` 等定性字段一律不碰（`src-tauri/src/commands/value-verify.rhai:1`、`:14-24`、`:72`）。

覆写不是「一律覆盖」：

- **合规判定**先看 LLM 原文是否已经命中正确数值（含区间文本），命中就不覆写——宁可少覆写，也不产生幻觉数字（`:36-45`）；
- 上游 `content` 不是合法 JSON 时**不硬失败、不覆写**，保留原文并留痕（`:45`、`:346-357`）；
- **硬不可用**（`dcf.available=false` / `upsidePct` 缺失 / `assumptions.applicable=false`）→ 用固定文案的「理想买入价」，`margin_of_safety` 落 `null`（`:30-34`、`:148-160`）；
- **软衰减**（`assumptions.is_fallback_anchor=true`）→ 代理锚仍是数值，**可以引用**；本节点不处置衰减，置信度衰减由提示词层承担（`:33-35`）。

`verdict` 形态归一（扁平/嵌套统一，补齐 `verdict` / `bull_score` / `bear_score` / `confidence`）也在这个节点做（`:47-75`），因为它是该链唯一的确定性出口；但**数值回填与定性只校验的边界不混**：形态归一只保证字段在位，不改变结论文字。

（对应版本：v3.0.0，当前实现；`value-verify.rhai` 无独立提交可定位）

### 5. 静态门禁口径

仓库被拆成 8 个 Cargo workspace 根（`axagent-core` / `-kit` / `-ai` / `-infra` / `-quant` / `-plugins` / `-runtime` / `src-tauri`）。任何「只在 `src-tauri` 跑」的 fmt/clippy 都只覆盖根包，其余 crate 完全不检查而门禁照样绿（`scripts/cargo-all-workspaces.mjs:4-11`）。因此统一由 `scripts/cargo-all-workspaces.mjs` 逐根执行：

| 门禁 | 实际命令 | 覆盖面要点 |
|---|---|---|
| 格式 | `cargo fmt --all --check`（`scripts/cargo-all-workspaces.mjs:33`，CI `:616`） | 必须带 `--all`，否则 `crates/*` 不被检查 |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings`（`scripts/cargo-all-workspaces.mjs:34-36`，CI `:619`） | 缺 `--all-targets` 漏 test/bench；缺 `--all-features` 漏 feature-gated 代码 |
| 测试 | CI：`cargo nextest run --workspace`（`scripts/cargo-all-workspaces.mjs:38`，CI `:738`）；本地 Windows 直跑 `cargo test` 必须 `__TAURI_WORKSPACE__=true`（`AGENTS.md:122-152`） | 不设该变量，链接 tauri 的测试 exe 启动即报 `STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)` |
| 前端 lint | CI：`npx oxlint src --max-warnings=0`（`.github/workflows/ci.yml:442`）；本地镜像：`node node_modules/oxlint/bin/oxlint src --max-warnings=0`（`scripts/ci-check.mjs:619`） | 不用 ESLint：TS 7.0.2 与 typescript-eslint 的 peerDeps（`<6.1.0`）冲突，`scripts/ci-check.mjs:608-610` |

本地镜像入口 `node scripts/ci-check.mjs --quick` 覆盖「纯 Node 静态门 + dprint + oxlint + tsc + rustfmt」；全量仍在所有阶段完成后跑一次（`scripts/ci-check.mjs:9`、`:256`）。

（对应版本：v3.0.0+（unreleased，2026-11-01 起按 8 根执行；证据 8dfaeb4db、`CHANGELOG.md:100`））

## 关键代码入口

- 后端命令/工作流接线：
  - DQI 观测写入 `src-tauri/src/commands/stock_workflow/core.rs:69`
  - 熔断变量注入 `src-tauri/src/commands/stock_workflow/hooks.rs:527-537`
  - 模板版本门 `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:1643`（`TEMPLATE_VERSION = 194`）
- 引擎/业务层：
  - DQI 分级 `src-tauri/src/commands/data-quality.rhai:1624-1636`
  - 字段校验 `src-tauri/src/commands/data-quality.rhai:581-610`
  - 权重塌缩/降级 `src-tauri/src/commands/portfolio-mgr.rhai:2791-2833`
  - 数值覆写 `src-tauri/src/commands/value-verify.rhai:14-45`
  - 熔断读写下沉 `axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:56-81`
- 前端：
  - `src/types/stock-analysis.ts`（DQI / 决策结果类型）
  - `src/components/stock-analysis/RecommendationPanel.tsx`（仓位与置信度展示）
- 数据表/迁移：
  - `data_quality_observations`（实体 `axagent-core/crates/entities/src/data_quality_observations.rs:35`，`abnormal` 列 `:51`）
  - 门禁回归 `axagent-runtime/crates/rt-workflow/tests/data_quality_placeholder_gate.rs:1`

## 配置与开关

- `DQI_FUSE_STREAK = 3`、`DQI_STREAK_LOOKBACK = 12`、`DQI_SCOPE_GLOBAL = "global"`（`axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:54-64`）。刻意做成代码常量而非面板变量：可调参数要登记五处，漏一处就是「纸面可调」（`:39-46`）。
- 等级阈值 `85 / 65 / 45 / 25` 与权重 `0.35 / 0.35 / 0.30`（`src-tauri/src/commands/data-quality.rhai:1624-1636`）。
- 评分自洽阈值 `>10` 不自洽 / `5–10` 轻微偏离（`src-tauri/src/commands/data-quality.rhai:555-567`）。
- 权重坍缩阈值 `0.0 / 0.15 / 0.3`、单点不可信分界 `posterior = 0.70`、`untrusted_count >= 2`（`src-tauri/src/commands/portfolio-mgr.rhai:2804-2817`）。
- 模板版本 `TEMPLATE_VERSION = 194`：改 `data-quality.rhai` / `portfolio-mgr.rhai` 正文必须升版，否则存量库被 `existing.version >= TEMPLATE_VERSION` 版本门静默挡住（`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:1643`）。
- 工作流节点失败策略：`data-quality` 节点 `continue_on_fail = true`，整段没跑成会以 `NULL` 等级计入异常（`axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:17-19`）。

## 已知限制与坑

1. **B 级曾被误判为异常**：历史判据是 `grade != "A"`，生产库 4 轮全 B 导致连续误熔断；2026-10-10 收窄为 C/D/F/NULL，并回填了历史 `abnormal` 列（`axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:20-34`）。若看到旧数据里 B 行 `abnormal=1`，先确认是否已迁移。
2. **「观望」不是写出来的 action**：F 级只清零 `position_pct` 并减半置信度，不改写 `action` 枚举（`src-tauri/src/commands/portfolio-mgr.rhai:2819-2833`）。下游若直接读 `action` 会看到「买入」而仓位为 0，必须同时读仓位。
3. **评分偏离只提示、不降权**：`bull+bear` 偏离 100 不剥夺投票资格，方向矛盾才会；把前者当成不可信会与现行裁定相悖（`src-tauri/src/commands/data-quality.rhai:521-529`）。
4. **`confidence` 负值不报错**：校验拦 `> 100` 与 `0 < conf < 10`，负值被 `conf >= 0.0` 前置条件跳过（缺字段同样按「不知道」跳过）；见 `src-tauri/src/commands/data-quality.rhai:600-610`。
5. **模板升级是静默门**：`existing.version >= TEMPLATE_VERSION` 时直接跳过；并发会话或更高常量的构建写过库，本批改动会被静默挡住。门上有三道路由漂移门（辩论轮数 / 逐档辩论 / 上下文瘦身）会无视版本门重建，但它们只认图内容、不认阈值改动（`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:2593-2626`）。
6. **本地 `cargo test` 与 CI 不同径**：CI 走 nextest 不需要 `__TAURI_WORKSPACE__`；Windows 本地直跑 `cargo test` 必须设置，否则报 `0xc0000139`（`AGENTS.md:122-152`）。
7. **实体注释与判据已漂移**：`data_quality_observations` 的 `abnormal` 注释仍写 `grade ∈ {B,C,D,F}`（`axagent-core/crates/entities/src/data_quality_observations.rs:49-50`），而现行 DAO 判据是 A/B 都正常、C/D/F/NULL 才异常（`axagent-infra/crates/dao/src/repo/data_quality_fuse.rs:66-76`）。以 DAO 为准；这一处是 2026-10-10 收窄判据时漏改的文档。
8. **`--quick` 不是全量**：它只跑纯 Node 静态门 + dprint + oxlint + tsc + rustfmt，不含 clippy / test；全量门禁只在所有阶段完成后跑一次（`scripts/ci-check.mjs:9`、`:256`）。

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `gate-coverage-ledger.md` | active | v3.0.0 | 门禁覆盖面缺口冻结台账（2026-10-31）；**若台账条目已在当前实现修复，以代码为准**（台账自述只减不增） |
| `AUDIT-gates-and-headless-scheduler-2026-10-26.md` | done | v2.9.9 | 三门可配置 + 无头调度的决策审计；「降级观望」口径与本文件 F 级处置同源 |
| `AUDIT-gate-and-daemon-impact-2026-10-26.md` | active | v2.9.9 | 三门放开的业务价值量化与常驻调度影响面；与熔断/观望策略互为背景 |
| `PLAN-reco-signal-quality.md` | active | v2.9.9 | 荐股信号质量治理（置信度无预测力）；与本文件字段级 `confidence` 校验相关 |
| `PLAN-dcf-inapplicable-panel-gate.md` | active | v2.9.9 | DCF 不适用形态的闸口扩展；`value-verify.rhai` 硬不可用分支的上游依据 |
| `PLAN-asof-replay-quality-attribution.md` | active | v2.9.9 | 回放 52.1/C 级误判归因（S1–S8 + R8/R9a/R9b）；gap_reason 如实归因的直接来源 |
| `AUDIT-data-quality-dual-algorithm-2026-09-14.md` | active | v2.9.7 | 双算法根因（决策恒 C / 弹窗恒 A）与四套 DQI 阈值收敛；本文件分级口径的前身 |

## 版本变更记录

| 版本 | 日期 | 该功能点在本版本改了什么 | 证据（提交短哈希或 CHANGELOG 版本节） |
|---|---|---|---|
| v2.9.7 | 2026-09-14 | 数据质量双算法审计与统一：四套 DQI 阈值收敛为一份 | `AUDIT-data-quality-dual-algorithm-2026-09-14.md` |
| v3.0.0 | 2026-09-26 | 回放质量归因：豁免维度 `gap_reason` 如实归因（模板 v83） | `df0fbc77a` |
| v3.0.0 | 2026-09-26 | 方向冲突退出工具可信度扣分（模板 v84） | `648ada196` |
| v3.0.0 | 2026-09-26 | 词表缺席 / 失败标记定向抑制（模板 v85–v86） | `566e4bf55`、`f27c040ce` |
| v3.0.0 | 2026-09-27 | 断句烧穿 CodeNode 操作预算修复（模板 v87） | `f11579c5b` |
| v3.0.0 | 2026-10-02 | 置信度口径统一 + 数据质量缺席分列（模板 v118–v122） | `51ddfec17`；`CHANGELOG.md:163` |
| v3.0.0 | 2026-10-08 | 数据质量熔断上线（跨轮连续异常 → 收紧置信度） | `6f3c5f308`；`CHANGELOG.md:167` |
| v3.0.0 | 2026-10-28 | 孤儿观测计入熔断判据（连续轮数 N 不跨空档） | `c6494fb8b`；`CHANGELOG.md:345` |
| unreleased | 2026-11-01 | 门禁改按 8 个 workspace 根全量执行（fmt / clippy / test） | `8dfaeb4db`；`CHANGELOG.md:100` |
