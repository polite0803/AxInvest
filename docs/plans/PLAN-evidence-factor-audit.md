# PLAN-evidence-factor-audit —— 证据引用审计改判据：从文本相似度换成决策因子对账

> 状态：active ｜ 日期：2026-10-16 ｜ 对应版本：v2.9.9


立档：2026-10-16　执行人：Qoder　状态：**待批（未开工）**

## 〇、为什么要换判据（现网实证，不是推测）

取数正门 `node scripts/pg-connect.mjs sql`；复算脚本 `.workbuddy/tmp/cite_repro_v2.mjs`（按当前 Rust 语义逐位移植）。

| 读数 | 值 |
|---|---|
| `stock_analyses` 中 `status='completed'` | 156 行 |
| reasoning 为「计算轨迹串」（含 ` \| `） | **129 行（83%）** |
| reasoning 为分析师散文（含 `。`） | 9 行 |
| reasoning 为 Rhai 降级诊断串 | 3 行（已被 `decision_degraded` 闸口拦下） |
| reasoning 为空 / 无快照 | 23 行 / 15 行 |
| 最近 40 行逐条复算 | **27 行 100%、13 行 0%，全部是同一形态轨迹串** |

失真机理：轨迹串不含 `。！？\n` ⇒ 拆句恒得 **1 条 claim** ⇒ 面板只有 0%/100% 两个读数。而命中与否实测由 claim 的**唯一字符数**决定（MISS 组 47–87、HIT 组 84–158；两组「claim 字符被报告覆盖率」同为 0.6–0.78，报告去重字符恒 ~300）⇒ `MATCH_THRESHOLD=0.15` 实际等价于「claim ≥ 约 85 个唯一字符」。claim 长度来自 `src-tauri/src/commands/portfolio-mgr.rhai:3619` 的 14 个可选 `${*_note}` 注解段本轮碰巧打了几条。

结论：现判据测的是**文案长度**，不是证据。调阈值、拆段均已实测否决（见 `docs/audits/` 与 `src-tauri/crates/analysis-engine/src/evidence_citation.rs:197` 的留档注释）。

## 一、新判据的原料（全部已落库，不新增采集）

以 600406 · 2026-10-15（`id=cf71078f-f61c-4ee9-b29c-1f7c861a2498`）为样本实测：

1. **腿集**：`decision_json.evidence.factors.{f1..f7,f9..f13}`，每条 `{name, sigma, weight}`（12 条，无 f8）。生产者 `src-tauri/src/commands/portfolio-mgr.rhai:3663`。
2. **闭合基准**：`evidence.weightSum / avgSignal / prior / posterior`。文档化不变式在 `portfolio-mgr.rhai:3660`：`Σ(σ·w)/weightSum == avgSignal`。
3. **快照命名空间**：`report.*`（22，带档后缀）/ `result.*`（44，工具与分支节点产出）/ `_raw.*`（90）/ `params.*`（53）/ `value.*`（2）。
4. **归因权威表（Rust 侧既有，禁止另抄一份）**：
   - `src-tauri/crates/analysis-engine/src/evidence_weight.rs:404` `DECISION_LEG_ANALYST`：fN → owner 分析师（f4/f6/f7 刻意 `None`＝元约束，不是分析师证据）
   - `src-tauri/crates/analysis-engine/src/evidence_weight.rs:448` `VERDICT_FACTOR_SOURCES`：分支因子名 → `kind ∈ {tierScore, sharedTool, verdict}` + channel
   - `src-tauri/crates/analysis-engine/src/evidence_weight.rs:433` `UNBRIDGED_ANALYST_IDS`：无腿分析师的显式登记
   - `src-tauri/crates/harness/src/holding_period.rs:344` `Period::scoring_node_id()`、`Period::verdict_spec()`：本档应参与腿集与逐档评分节点

### σ 可回算性实测（决定阶段 1 能做到什么程度）

| 腿 | σ 实测 | 本记录内可对的基准 | 判定 |
|---|---|---|---|
| f4 risk | −0.03 | `decision_json.risk_quant.f4_adjusted` = −0.03 | **零公式自洽 ✔** |
| f11 pace | 0.31 | `blackboard.result.pace-calc.pace_signal` = 0.31 | **零公式自洽 ✔** |
| f1 trend | 0.56 | `result.t-scoring.content.totalScore` = 78，需 `(78−50)/50` | 需归一化口径（公式在 `portfolio-mgr.rhai:328`）|
| f2 consensus | 0.1 | 同上，`portfolio-mgr.rhai:336` 同式 | 需归一化口径 |
| f9 / f10 / f12 | 0.771 / 0 / 0.3 | `evidence.f9_money_flow_signal`=0.77、`f10_chip_signal`、`f12_momentum_signal`（2 位 vs 3 位） | 需容差；且是**同脚本伴生键**，非独立上游 |
| f3 / f5 / f6 / f13 | 0.44 / 0.406 / 0 / 0 | 上游为 `a-catalyst` verdict / `t-valuation-band` 分位 / `data-quality` / serenity 合成 | 需逐腿定基准，阶段 0 普查 |

## 二、四项判据（每条腿各给一态，不折算成单一相似度分）

- **L1 上游在场**：按腿的 `source.kind` 分流查快照——`verdict` → `report.<owner>[--档]` 非空；`tierScore` → 本档评分节点产出；`sharedTool` → `result.<channel 对应节点>` 非空。owner/kind/channel 一律由第一节四张权威表现场推导。
- **L2 σ 可回算**：σ 与本记录内已落库的上游原值按**同一口径**自洽。口径分两档：① 零公式同键自洽（f4/f11 实测通过）；② 需归一化口径（f1/f2）⇒ 是否把 `(x−50)/50` 上收为单一权威见待裁定 **P-B1**。
- **L3 参与融合**：`weight > 0` 且该腿进入 `Σ(σ·w)`；全局闭合 `|Σ(σ·w)/weightSum − avgSignal| ≤ 容差`，容差由 σ 取 3 位、w 取 4 位的舍入界推出，**不写死常数**。
- **L4 缺席有交代**：`weight == 0` 的腿必须在 `data_gaps`、腿的 `constraint` 或 `UNBRIDGED_ANALYST_IDS` 里有显式登记，否则记「静默缺席」。实测本行 f6/f7/f10/f13 四条 w=0 而 `data_gaps` 是**空数组**、`decision_trail` 也是空数组 ⇒ 该态今天必然亮红，属真实缺陷而非判据过严。

## 三、呈现层新口径

- 统计卡三格改为**三条独立计数**：`参与融合 8/12`、`σ 可回算 5/8`、`上游在场 7/8`。不再提供单一「数据支撑率」百分比；若要保留一个总数字，取 **L1∧L2∧L3 全中的腿数 / 本档应参与腿数**（分母见 **P-B3**）。
- 逐腿一行：腿名 / σ / w / owner 显示名（复用 `evidence_citation.rs:112` `analyst_display_name`）/ 来源通道 / 四态标签。
- 「来源分析师」不再把 miss 写入的字面量 `"unknown"` 计入（现缺陷：`evidence_citation.rs:296`）。
- 三种缺席各占一句、互不合并（对齐既有裁定）：本轮无决策（沿用 `decision_degraded`）／该腿无上游产出（L1 红）／口径未上收故本记录内无法回算（L2 未接入）。
- 11 种语言同步补 key（禁区 2）。

## 四、退役面

`evidence_citation.rs` 的文本匹配器整体退役：`W_JACCARD`/`W_NUMBER`/`MATCH_THRESHOLD`、Jaccard 与 `number_overlap` 打分、按句拆 claim、`has_data_support` 只在命中分支算的那段。保留并复用：`analyst_display_name`、`EvidenceCitation`/`CitationReport` 的骨架按新判据重定义、`decision_degraded` 闸口（`src-tauri/src/commands/stock_analysis.rs:1632`）。`citations_to_markdown` 改读新结构。命令改名与否见 **P-B2**。

## 五、阶段切分

- **阶段 0｜普查（只读取数，零改动）** —— 把下面 5 个未知数变成读数，任一不成立就回到本节改判据定义：
  1. **逐档评分节点的产出键**：`Period::scoring_node_id()` 声明 `t-scoring-hour|week|month|quarter`，但实测**最近 49 条 completed 行里 0 条**快照含该字面量，只有 `result.t-scoring` ⇒ 定性：节点未跑 / 产出并键 / 快照裁剪。（L1 的 tierScore 分支能否成立，全押这一条。）
  2. 12 条腿各自的 `channel → result.*` 键映射在现网的命中率（逐 vendor 读臂，不许由「查不到」静默表达）。
  3. L2 可回算覆盖率 across 现网：零公式自洽（f4/f11）在 113 条有 factors 的行里各命中多少；容差取多少才不误伤。
  4. `data_gaps` / `decision_trail` 填充率（决定 L4 是「普遍亮红」还是「普遍空转」）。
  5. 主档 `evidence.factors` 腿集与 `decision_json.decisionsByHorizon.<档>.legs[]` 腿集是否同域（实测两侧词汇不同：`trend` vs `trendStrength`；且分支腿**未落 owner/source**）⇒ 审计对象取哪一侧。
- **阶段 1｜Rust 判据层**：新模块（建议 `crates/analysis-engine/src/evidence_audit.rs`，角色 implementor，仅依赖 harness + entities 之外的 implementor 兄弟）实现 L1–L4，复用第一节四张表；单元测试 + 每条判据一个负控（拆在真正决定该断言的那道守卫上）。
- **阶段 2｜命令与 DTO**：`extract_evidence_citations` 换实现或改名（P-B2），`#[serde(rename_all = "camelCase")]`，前端 `src/types/` 同步。
- **阶段 3｜面板**：`src/components/stock-analysis/EvidenceCitationPanel.tsx` 三条计数 + 逐腿四态；组件测试按「三种缺席各占一句」逐态锁，每个破坏样本只让对应那一条红。
- **阶段 4｜门禁**：新门（判据侧：12 腿的 owner/channel 必须能从权威表推导且落库键可定位，缺项走带日期豁免并每次打印）+ 自证打在**未修的树**上；收尾按 CI 口径三条全量（`cargo fmt --all --check` / `clippy --workspace --all-targets --all-features -D warnings` / `__TAURI_WORKSPACE__=true cargo test --workspace`）+ 前端 `npm run ci:check:quick`。

## 六、待裁定（2026-10-16 用户按编号一次点完，全部照推荐放行）

- **P-B1｜归一化口径上收 harness** ✅ 批准。`(x−50)/50` 从 `portfolio-mgr.rhai:328/336` 上收为 Rust 单一权威，
  Rhai 改读注入函数 ⇒ 同批 `TEMPLATE_VERSION` bump，存量行不回算 f1/f2。
- **P-B2｜命令改名** ✅ 批准。`extract_evidence_citations` → `audit_decision_evidence`，
  同批清掉前端旧调用方（禁区 12）。
- **P-B3｜总数字分母** ✅ 批准取**本档 `verdict_spec().participating` 腿集**，写进面板文案。
- **P-B4｜存量行不回算 L4** ✅ 批准。早于本批上线日的行显示「缺席交代未采集」而不是红。

## 七、执行状态

### 阶段 0｜现网普查 —— **已完成**（2026-10-16，只读取数，零改动）

| # | 未知数 | 读数 | 结论 |
|---|---|---|---|
| 0-1 | 逐档评分节点产出键 | `Period::scoring_node_id()` 声明的 `t-scoring-hour\|week\|month\|quarter` **确实建在子模板** `src-tauri/src/commands/stock_analysis_setup/horizon_tier_template.rs:224/352/468/593`，其 `tier_score` 入参逐条指向各自档节点（`:280/409/524/650`）；但**最近 49 条 completed 行的主快照里 0 条含该字面量**，只有共享的 `result.t-scoring`。本库**无 `node_executions` 表**、`tool_executions` 0 行 ⇒ 没有任何持久化通路能取到子图节点原值 | tierScore 类腿的 L1 **只能判「派生量在场」**（`result.h_<档>` 的 `scoreSource='tier_native'` + `scoringScale` + `scoringTrend`，实测 49/49 齐），**不得**判「totalScore 原值可对账」 |
| 0-2 | 12 腿 channel → 快照键 | 分支侧权威表 `VERDICT_FACTOR_SOURCES` 的 12 个 channel 全部能在主快照 `result.*` 定位（`t-valuation-band`/`t-lockup-data`/`t-consensus-data`/`t-macro-data`/`t-limitup-pool`/`t-sector-data`/`pace-calc`/`t-scoring`…）；`verdict` 类落到 `report.<owner>--<档>` | L1 的 sharedTool / verdict 两臂**有数据通路** |
| 0-3 | L2 可回算率 | 有 factors 的 113 行：f4 σ ↔ `risk_quant.f4_adjusted` 逐位相等 74/113；**按两侧舍入界推容差**（factors 侧 3 位 ±0.0005、risk_quant 侧 2 位 ±0.005 ⇒ 0.0055）后 **112/113 在容差内、0 条超差**（余 1 条该侧为 null）。f11↔`evidence.f11_pace_signal` 112/113、f12↔`f12_momentum_signal` 112/113 | 容差必须由舍入界**推出**（写死 0.01 会把 0.0055 边界的行判错）；f11/f12 的基准是同脚本伴生键，**证明自洽不证明来源真实**，面板措辞要分开 |
| 0-4 | L4 料 | 主档 `data_gaps` 非空 **24/156（15%）**、`decision_trail` 非空 68/156（44%）；逐档侧 `result.h_ultra_short` 的 `absentLegs` **49/49 在场且非空**、`dataGaps` **49/49 非空** | L4 的料在**逐档侧**，不在主档侧 |
| 0-5 | 两侧腿集是否同域 | **不同域且不可互推**：主档 `evidence.factors` 用 f 词汇（trend/consensus/…，12 条，有 σ/w 与落库基准）；逐档 `result.h_<档>.legs[]` 与 `decision_json.decisionsByHorizon.<档>.legs[]` 用分支词汇（momentumSignal/eventCatalyst/breadthState/…，本档 3 条左右，有 role/weight/constraint 与 absentLegs，但 **owner 与 source 未落库**，须由 `Period::factor_owner` + `VERDICT_FACTOR_SOURCES` 现场推导）。两侧只共享 owner 分析师，**是多对多、不是逐腿同一性** | ⇒ 见下「判据修订」，本项**推翻了 §二 的单腿集假设** |

### 判据修订（与 §二 的偏差，逐条列名）

阶段 0 的 0-1 / 0-4 / 0-5 三条读数合推出：**四项判据不可能都作用在同一套腿上**。§二 原写法（一套腿集跑 L1–L4）作废，改为**两面**：

- **A 面｜主档 12 腿**（`evidence.factors.fN`）承担 **L2 σ 可回算 + L3 参与融合**。
  L3 全局闭合 `|Σ(σ·w)/weightSum − avgSignal| ≤ 容差`，容差同 0-3 口径由舍入界推出。
  L4 在 A 面**不做**（0-4：主档 `data_gaps` 仅 15% 非空，P-B4 已批存量不回算）。
- **B 面｜本档分支腿**（`result.h_<档>.legs[]`）承担 **L1 上游在场 + L3 参与（`role=direction` 且 `weight>0`）+ L4 缺席有交代**（`absentLegs` ∪ `dataGaps` ∪ `qualified` ∪ `not_applicable`）。
  P-B3 批准的总分母 = `Period::verdict_spec().participating` ⇒ **落在 B 面**。
- 两面各自出计数，**不得并排成同一个可比数字**（既有裁定：两套计数域不得收敛为一个数）。面板两段：「本档分支腿审计」+「主档因子闭合对账」。
- tierScore 臂的 L1 措辞按 0-1 收窄为「派生量在场」，不得写成「已核对上游评分」。

### 阶段 1｜Rust 判据层 —— **已落地**（未提交）

- 新模块 `src-tauri/crates/analysis-engine/src/evidence_audit.rs`：A/B 两面、`UpstreamState` 四态、
  `Reconcile` 三态、`Closure` 三态，归因与来源全部现场推导自既有权威表（无第二份抄本）。
- 容差**全部由舍入位数推导**（`rounding_bound` / `sigma_basis_tolerance` / `closure_tolerance`），
  并配门 `rounding_digits_match_the_rhai_literals`：`include_str!` 直读 `portfolio-mgr.rhai`，
  逐字锁 σ 3 位 / 权重 4 位 / 基准与 avgSignal 2 位四处字面量。
- 阶段 0 读数驱动的两处口径收窄已写进实现与测试：tierScore 臂只能判「派生量在场」；
  channel 是工具名时判 `NotChecked`（**不得**降级成缺席）。
- 缺席登记里的腿（`absentLegs` / `dataGaps`）**显式成行** + 本档应参与却两处都没登记的腿判
  `SilentAbsent` —— 这两条是本模块 L4 的全部信号，也是测试先红才补出来的实现缺口。

### 阶段 2｜命令与 DTO —— **已落地**（未提交）

- `extract_evidence_citations` → `audit_decision_evidence`（裁定 P-B2）；`register_commands.rs`
  已由 build.rs 重生成，旧命令名零残留。
- 退役旧匹配器：`evidence_citation.rs` 收窄到只剩 `analyst_display_name`（`extract_citations`、
  `citations_to_markdown`、`EvidenceCitation`、`CitationReport`、`W_*`、`MATCH_THRESHOLD`
  及全部旧测试已删；全仓零引用已核实）。
- `harness/src/domain_semantics.rs` 移除 `axinvest.evidence.match_confidence` 登记（载体已不存在）。

### 阶段 3｜面板与 i18n —— **已落地**（未提交）

- `EvidenceCitationPanel.tsx` 重写：两面区块 + 三条计数 + 逐腿四态，删单一「数据支撑率」读数；
  `evidenceConfidenceBar.ts` 与其测试因零引用一并删除。
- 11 语言 `stockAnalysis.evidenceCitation` 段重排为 24 个顶层键（保留 7 + 新增 17，其中 6 组是嵌套）；
  `supported` 因 `AnalystReportGrid.tsx:819` 仍在引用而**回补**（删早了一次，由 `check_i18n --strict` 抓出）。
- 读数：面板测试 5/5 绿；`typecheck` / `dprint` / `oxlint` / `i18n strict` / `i18n key-parity` /
  `i18n placeholder` / `i18n untranslated` / `check-contracts` 八门全绿。

### 阶段 4｜门禁 —— **已落**（除全量三条）

- 舍入常量门（见阶段 1）已在终局跑通。
- 终局读数（2026-10-17 取，均为**修复后**的最终源码）：

  | 门 | 读数 |
  |---|---|
  | `cargo check -p axagent-analysis-engine --all-targets` | EXIT=0 |
  | `cargo test -p axagent-analysis-engine --lib` | **724 passed / 0 failed**（10 条新测试全绿，含舍入常量门） |
  | `cargo clippy -p axagent-analysis-engine --all-targets -D warnings` | EXIT=0 |
  | `cargo clippy -p axagent-harness --all-targets -D warnings` | EXIT=0（15m28s） |
  | `cargo check -p axagent --all-targets`（主 crate） | EXIT=0（31m21s，0 错误） |
  | `rustfmt --edition 2024 --check`（三个改动文件） | 空输出 |
  | 前端八门 + `npm run ci:check:quick`（含全仓 `cargo fmt` 检查） | 全绿 |
  | `src/components/stock-analysis` 子树 vitest | 414/414 |

- **clippy 定点曾红 2 条**（`evidence_audit.rs:375/408` 的 `clippy::manual_contains`）。
  修法**没照抄 lint 建议**：它给的 `participating_set.contains(&factor)` 在
  `Vec<&'static str>` + 短生命周期 `&str` 参数上**类型不成立**；改成 `HashSet<String>`
  走 `Borrow<str>` 通道，判定语义与原来逐字一致（`expected_branch_legs` 的取值仍由
  `expected_leg_count_comes_from_the_authority` 对着权威 spec 现算锁定）。
- 期间一度红过两条，都是**判据/夹具自身**的问题、不是实现偷偷放过：① 期望值手敲错
  （`sigma_reconciled_legs` 应 3 写成 2）；② 负控抓出「只出现在 `dataGaps` 散文里的腿不成行」
  —— 已把 `absentLegs` ∪ `dataGaps` 合并成同一份缺席清单（原先 `dataGaps` 只用于置标、
  从不促成一行，`dataGaps`-only 的缺席会被整条漏掉）。
- 全量 CI 口径三条（2026-10-17 按 CI 逐字跑，日志 `.workbuddy/tmp/full_gate.log`）：**三条红，
  但零条落在本批**。逐条归因（本批文件此时已由用户提交为 `6bf545b1a`，工作树只剩并行会话的 8 个在飞文件）：

  | 门 | 读数 | 归属 |
  |---|---|---|
  | `cargo fmt --all --check` | EXIT=1，4 处 diff | 全在 `seed_consistency_tests.rs:4302/4453`、`seed_stock_analysis.rs:4040/4093`（并行会话在飞） |
  | `clippy --workspace --all-targets --all-features -D warnings` | EXIT=101，2 条 `unused variable` | `seed_stock_analysis.rs:4678`（`all_convergences`）、`:4682`（`all_briefs`）—— 重构中途残留；**clippy 遇首错短路**，其后的 crate 未被检查 |
  | `__TAURI_WORKSPACE__=true cargo test --workspace` | EXIT=101，首套件即红 | 主 crate lib **657 passed / 25 failed**，25 条全在 `commands::stock_analysis_setup::{version_gate_tests 10, fast_workflow_derivation_tests 10, horizon_tier_fanout_tests 4, seed_consistency_tests 1}`（A4 逐档化在飞）；**fail-fast ⇒ 其余 crate 套件未跑**，故这不是「全仓」读数 |

  ⇒ 本批的终局证据是**定点那一栏**（全绿）；全量栏要等并行会话落地后重跑才算数。
  按分档规矩不动他们的文件：那两条 `unused variable` 属逻辑类、25 红是他们在改的行为。

### P-B1 与 P-B5 是同一缺口的两半（新发现，并入下一批）

f1/f2 的 σ 基准 `totalScore` **住在快照**（`result.t-scoring.content.totalScore`，与主图
`input_mapping` 同路径，见 `seed_stock_analysis.rs:5729`），**不在 `decision_json`** ⇒

- 只做 P-B1（归一化口径上收 harness）不足以让 f1/f2 进 L2 —— 对账实现还必须能读**快照路径**；
- 而「channel → 快照键」的对应关系正是 **P-B5** 要裁的那张表。

⇒ 建议合并为一批：先定 P-B5（登记 channel→快照键 的权威 + 门），再把 P-B1 的口径上收与
f1/f2 基准一起做，然后才把 f1/f2 加进 `FACTOR_SIGMA_BASIS`。

## 八、第二批（P-B5 + P-B1）执行状态 —— **已落地**（2026-10-17，未提交）

### P-B5｜通道 → 快照键 权威表

- 新增 `evidence_weight::CHANNEL_SNAPSHOT_KEYS` + `channel_snapshot_key()`：**7 条 sharedTool
  通道全部有落库键**（此前只有 3 条因「channel 字面量恰好等于节点键」才判得了）。
  顺带实证一个坑：通道写 `get_consensus_eps` 而工具注册名是 `get_stock_consensus_eps`
  —— 靠字面量相等去猜键，本来就不可能全对。
- L1 的 sharedTool 臂改走该表，并且**语义收紧**：表里有映射而本记录缺该键 ⇒ **静默缺席**
  （那个节点本轮没出数），不再笼统报「不适用」；只有表外通道才报 `NotChecked`。
- 两条门（`evidence_weight.rs` 末尾新 mod）：
  `every_shared_tool_channel_has_a_snapshot_key`（含正控 + 「tierScore 通道不得入表」负控）
  与 `snapshot_node_ids_exist_in_seed`（`include_str!` 直读播种源码逐节点核对 + 编造 id 负控）。

### P-B1｜归一化口径上收

- **没有新建权威**：查重发现 `(x−50)/50` 已在 `leg_signal.rs:81` 存在一份（逐档腿
  `trendStrength ← totalScore`）⇒ 把它**提取**成 `portfolio_formula::score_to_signal`
  （该模块正是既有 `pm_*` 宿主函数的委托目标，模块头写着「Rhai 脚本只做规则编排」）。
- 五处内联全部收口：主链四处（f1/f2/f6/f13）经新宿主函数 `pm_score_to_signal` 调用；
  逐档腿那一处直接调同一函数。**刻意不钳制**（越界要能被调用方看见），由
  `out_of_range_is_not_silently_clamped` 锁住，防有人顺手加 `.clamp()` 把「上游给了非法分」
  静默抹平。
- 宿主函数**双载**（`i64` + `f64`）：`totalScore` 来自 JSON 数值 ⇒ Rhai 侧是 INT，
  只注册 f64 会在运行时报 `Function not found` ⇒ 整段降级。
- `TEMPLATE_VERSION` 161 → **162**（脚本内容变了必须重灌）。
- 两条门：`score_to_signal_tests`（中点/两端/真记录锚点 78→0.56 + 不钳制）与
  `score_normalization_is_not_inlined_anywhere`（主链四个调用点逐个点名 + 内联式零命中 +
  leg_signal 不再内联）。

### 面板与 i18n 的连带改动（同批）

- L1 换了两个**新的不适用原因码**（`TIER_SCORE_NOT_IN_SNAPSHOT` / `NO_CHANNEL_MAPPING`）替换原先的
  `NO_SNAPSHOT_KEY` ⇒ 面板映射与 11 语言同步换新，退役的 `noSnapshotKey` 从 11 语言删除
  （`notChecked` 组现为 5 键：新 2 + `noOwner` / `unregisteredFactor` / `unknownReason`）。
- 原因句只出现在 antd `Tooltip` 的 title 里，而 rc-tooltip 的正文**懒渲染**（不 hover 不进 DOM）
  ⇒ 渲染断言只能断到状态标签、断不到「哪一种不适用」。故把「原因码 → 文案键」抽成**具名导出**
  `notCheckedReasonKey()` 直测：四个现役码各有专属文案 + 退役码/空串走兜底（负控）。
  兜底文案是「后端返回了未登记的原因码」—— **不猜原因**，避免把未知说成已知。

### f1 进 L2；f2 **如实留白**（新发现）

- `FACTOR_SIGMA_BASIS` 升级为 `(腿, 文档, 路径, 变换)`：f1 走
  `BasisDoc::Snapshot` + `/result.t-scoring/content/totalScore` + `ScoreToSignal`
  （**调同一个函数**，绝不在这里复述公式）。`resolve_path` 支持**途中剥 JSON 字符串壳**
  —— 快照里工具节点是 `{"content": "<JSON 字符串>"}` 双层形态。
- 实测锚点：600406 那条 `totalScore = 78` ⇒ `score_to_signal(78) = 0.56` = 落库 σ，逐位相等。
- **f2 不入表**：它的输入 `consensusScore` 取自收敛节点，而该节点正随「逐档实例化」在飞 ——
  同一条记录上老形态的键是裸 `debate-convergence.content.consensus_score`、新形态按 seed
  指向 `debate-convergence--mid.*`（v161），**两者都不在 `result.*` 里**。给一条会随模板
  漂移的静态路径＝把「对不上」伪装成「对过了」⇒ 留白，等节点形态落定再接。

### 本批读数

| 门 | 读数 |
|---|---|
| `test -p axagent-analysis-engine --lib` | **730 passed / 0 failed**（本批新增 7 条全绿） |
| pm 脚本四套件（`days_for_rhai` / `horizon_lock` / `tier_params` / `rhai_syntax_check`） | **32 条全绿**（本批 `.rhai` 改动的直接受影响面） |
| `EvidenceCitationPanel.test.tsx` | **6/6**（新增「原因码→文案键」纯映射用例） |
| `check_i18n` / `key-parity` / `placeholder` | 全通过 |
| `check-rhai-numeric-typing.mjs`（27 个 .rhai） | 形态一致（我的改动不含 `type_of` 守卫，未被该门触及） |
| `tsc --noEmit` / `dprint check`（本批文件） | 空输出 |
| `cargo fmt -p axagent-analysis-engine --check` | 空输出（**用 cargo fmt，不用裸 rustfmt**：后者风格版不同，会对既有文件多报 diff） |
| `cargo clippy -p axagent-analysis-engine --all-targets -D warnings` | **EXIT=0**（15m56s） |
| 主 crate `check -p axagent --all-targets` | **EXIT=0**（20m16s；含 `rhai_pm.rs` 宿主函数与模板 162） |

**`npm run ci:check:quick` 本批被挡在一处「不属于本批」的红上**（同上一批的处置）：
`audit-inject-coverage.mjs` 的 ① 报 `consensusScore` 无注入来源 —— 归因**由构造证明**：
该门第 176 行只收 `present(...)` 里的标识符，第 336 行用 `("键","值")` **双字面量**正则取注入来源；
`git show HEAD` 里那行是字面量 `("consensusScore", "debate-convergence.content.consensus_score")`（门当时绿），
而工作树里被并行会话改成 `("consensusScore", rep_conv_path.as_str())`（v161 逐档实例化）⇒ 正则取不到来源。
本批的 `.rhai` 改动（内联公式换函数调用）对该判据不可见。**逻辑类，不替他改**。
