# PLAN — 估值配置接线（C2 路径 Z）

> 立项时间：2026-09-12
> 状态：**已排期（待启动）**
> 前置：**路径 Y 已执行**（2026-09-12，见 §0）
> 关联报告：`output/legacy-items-c2-c1-t1-20260911.md` §2

---

## 0. 前置状态：路径 Y 已完成

`astock-data/src/mcp_tools.rs:1918-1920` 三常量已与 `analysis-engine::decision::ValueConfig`
的 A 股校准值对齐：

| 常量               | 原值 | 现值      | 依据（`decision.rs:216-227`）                      |
| ------------------ | ---- | --------- | -------------------------------------------------- |
| `PERPETUAL_GROWTH` | 0.03 | **0.04**  | 接近长期名义 GDP 增速（原 3% 偏低）                |
| `DISCOUNT_RATE`    | 0.10 | **0.085** | 无风险利率 2.5% + 6% 风险溢价（原 10% 偏高）       |
| `DEFAULT_GROWTH`   | 0.08 | **0.12**  | 优秀公司平均增速（原 8% 偏保守，系统性低估成长股） |

**路径 Y 解决的是「实际执行值 = 设计值」**；**路径 Z 解决的是「面板显示值 = 实际执行值」**。
两者互补，Y 不替代 Z。

---

## 1. 排期目标

让设置面板与 WhatIf 面板中的 6 个 `value_*` 变量**真正影响估值计算**，消除
「三处可见、零处生效」缺陷。

### 1.1 验收总标准（Definition of Done）

1. 在设置面板把「折现率」从 8.5 改成 6.0 → **下一次执行**的 `dcf.mid` 按 6% 折现（可量化验证）
2. 在 WhatIf 面板覆盖折现率 → `compute_what_if` 的估值分量随覆盖值变化
3. 不改任何变量时的输出 == 路径 Y 后的默认输出（**零回归**）
4. `TEMPLATE_VERSION` = 32 且 DB 模板行 `version` = 32（重种子生效）
5. 全门禁绿（clippy --workspace --all-targets -D warnings / fmt / tsc / dprint / i18n 11 语言）

---

## 2. 里程碑

| #      | 里程碑       | 主要改动                                                                    | 依赖  | 验收判据                                                                               |
| ------ | ------------ | --------------------------------------------------------------------------- | ----- | -------------------------------------------------------------------------------------- |
| **M0** | 参数扁平化   | `mcp_tools.rs::compute_valuation` 新增 6 个扁平参数解析 + 单元测试          | 无    | 传 `dcf_discount_rate=6.0` 时输出与内部构造 `ValuationConfig{discount_rate:0.06}` 等价 |
| **M1** | 种子接线     | `seed_stock_analysis.rs` 的 `tool_node` 闭包支持多映射；t-valuation 挂 6 项 | M0    | 种子后 DB 模板 JSON 中 t-valuation 的 `input_mapping` 含 7 项（stock_code + 6 变量）   |
| **M2** | 模板升版     | `seed_stock_analysis.rs:254` `TEMPLATE_VERSION` 31 → **32** + 版本注释      | M1    | 启动后 DB 模板 `version=32`；日志出现「更新股票分析工作流模板 v31 → v32」              |
| **M3** | 前端单位对齐 | `StockAnalysisConfigPanel.tsx` / `WhatIfBacktest.tsx` 传参口径 + i18n       | M0    | 面板输入 8.5 → 后端收到 8.5（百分数口径），执行侧换算为 0.085                          |
| **M4** | 生产验证     | 重编译二进制 + 实跑 + DB 复核                                               | M1-M3 | 见 §1.1 四项                                                                           |

**推荐实施顺序**：M0 → M1 → M2（后端一次性完成，一次编译）→ M3（前端，独立）→ M4。
**M0-M2 必须合并为一个改动批次**，否则「改了参数但没接线」会产生新的中间态死配置。

---

## 3. 关键设计决策（已定，实施时不得偏离）

### D1 — 单位口径：**百分数入口，后端单点换算**

**背景**：`ValuationConfig` 同 struct 内存在**两套单位** ——
`bond_yield` 默认 `4.4`（百分数），`discount_rate` 默认 `0.10`（小数）。
若扁平参数口径不约定，会出现「折现率 850%」级事故。

**决策**：扁平参数一律采用**百分数口径**，与前端面板现有显示值（8.5 / 4.0 / 12.0）一致：

```rust
// 扁平参数（arguments 层，百分数）
"dcf_discount_rate"  => 8.5    // 百分数
"dcf_perpetual_rate" => 4.0    // 百分数
"dcf_growth_rate"    => 12.0   // 百分数

// 内部换算（唯一换算点，mcp_tools.rs）
discount_rate:  raw / 100.0    // → 0.085
perpetual_growth: raw / 100.0  // → 0.04
default_growth: raw / 100.0    // → 0.12
```

**理由**：① 与面板/`ValueConfig::default()` 完全一致，用户心智零切换；
② 换算集中在一个函数内，可被单元测试覆盖；③ 避免在 11 语言 i18n 与 UI 层散落 `/100`。

**必须同时补的守卫**：`bond_yield` 保持百分数（不动），并在 `ValuationConfig` 文档注释
中显式标注每个字段的单位，防止后续再混。

### D2 — 接线方式：**扁平参数 + 扩展 `tool_node` 闭包**

**约束（决定性）**：`ToolNodeConfig.input_mapping` 是 `HashMap<String, String>`，
`dispatcher.rs:463-466` 把 value 当作**变量名**查（`context.variables.get(v)`），
**无法构造嵌套 object**。因此 `compute_valuation(arguments.valuation_config = {...})`
这条路走不通（`from_value` 会失败并被 `.ok()` 吞掉，静默回退常量）。

**决策**：在 `compute_valuation` 内新增扁平参数解析（6 项），与既有
`valuation_config` object 参数**并存**（object 优先，向后兼容）：

```rust
// 优先级：valuation_config(object) > 扁平参数 > 常量默认
let valuation_config = arguments.get("valuation_config")
    .and_then(|v| serde_json::from_value::<ValuationConfig>(v.clone()).ok())
    .or_else(|| flatten_valuation_config(&arguments));  // ← M0 新增
```

**`tool_node` 闭包改造**（`seed_stock_analysis.rs:348-386`）：
当前签名固定 `input_mapping.insert(arg_key, "stock_code")`，需扩展为可变映射：

```rust
let tool_node = |id, title, tool_name, output_var, arg_key, extra: &[(&str, &str)], parent_id, x, y| {
    let mut input_mapping = HashMap::new();
    input_mapping.insert(arg_key.to_string(), "stock_code".to_string());
    for (arg, var) in extra { input_mapping.insert(arg.to_string(), var.to_string()); }
    ...
};
```

已有调用点需补 `&[]`（约 10+ 处，机械改动）。

### D3 — 单一真相源：`ValueConfig` 归属

**现状**：同一组参数在**两处独立维护** —— `analysis-engine::decision::ValueConfig`
（设计值，无消费方）与 `astock-data::mcp_tools` 常量（执行值）。路径 Y 只是让两边
**数值相等**，仍是两份拷贝。

**决策（M0 顺带完成，低风险版本）**：不搬迁类型（搬迁会牵动 `decision.rs` 的
`ValueConfig` 消费链），改为在 `astock-data` 的常量定义处**注释指向**校准依据
（路径 Y 已做），并在 `decision.rs` 的 `ValueConfig` 上加注释标明「执行侧现为
`astock-data::src-tauri/crates/astock-data/src/mcp_tools.rs:1918-1920`，两份必须同步」。

**遗留（不阻塞，记录在案）**：真正的单一真相源（由 `seed_variables.rs` 派生默认值）
留待本 PLAN 完成后评估；若 M3 实现后 `ValueConfig` 仍无消费方，应重新评估其存废。

---

## 4. 技术陷阱清单（实施前必读）

| #  | 陷阱                                                                 | 后果                                                                                                                                                 | 规避                                                                          |
| -- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| T1 | `input_mapping` 只能做「参数名 → 变量名」一对一                      | 嵌套 object 静默回退默认值，改动看起来「没效果」                                                                                                     | 必须走扁平参数（D2）                                                          |
| T2 | `ValuationConfig` 内 `bond_yield` 百分数 / `discount_rate` 小数 混用 | 850% 折现率                                                                                                                                          | D1 单点换算 + 字段注释                                                        |
| T3 | 变量未在 DB 落地（`settings` 表 95 行内无 `stock_analysis_config`）  | `load_value_config` 读空 key 恒返回 default                                                                                                          | 本方案变量走 `seed_variables.rs`（模板 variables），不走 settings             |
| T4 | 忘记升 `TEMPLATE_VERSION`                                            | `existing.version >= TEMPLATE_VERSION` 直接跳过 → **静默不生效**                                                                                     | M2 强制项 + M4 验 DB version                                                  |
| T5 | 只改执行侧 `input_mapping` 但未重编译二进制                          | 模板 DB 已更新、运行时代码仍是旧的                                                                                                                   | M4 必须重编译后实跑                                                           |
| T6 | 前端 WhatIf 的 `configOverrides` 未进 `invokeParams`                 | 覆盖框形同虚设                                                                                                                                       | M3 明确改 `WhatIfBacktest.tsx:108-121` 的 `invokeParams` 拼装                 |
| T7 | 只跑 `cargo check -p axagent` 等全链                                 | 20+ 分钟白等                                                                                                                                         | 改 `astock-data` 只 `check -p axagent-astock-data`；改 seed 才需 `-p axagent` |
| T8 | `high_perpetual` 的硬编码上限 `0.05`（`mcp_tools.rs:2143`）          | 路径 Y 后 `0.04×1.3=0.052` **触顶 0.05**，乐观档从 97.89 跳到 167.45（+71%，远超中性档 +52%）；且与同文件 `MIN_GROWTH/MAX_GROWTH` 具名常量风格不一致 | M0 顺带提为具名常量 `MAX_PERPETUAL_GROWTH = 0.05` 并加注释                    |

---

## 5. 风险

| 风险                                    | 等级 | 影响                         | 缓解                                                                                                   |
| --------------------------------------- | ---- | ---------------------------- | ------------------------------------------------------------------------------------------------------ |
| 面板值生效后改变全样本 DCF 输出         | 中   | 用户历史结论可能变化         | 路径 Y 已评估：决策不变（仍「卖出」），仅一致性提升；M4 对比前后样本                                   |
| `tool_node` 闭包签名变更波及 10+ 调用点 | 低   | 编译期暴露，无静默风险       | 机械补 `&[]`；编译驱动                                                                                 |
| 重种子覆盖用户自定义变量                | 中   | 用户在模板里改过的变量被重置 | `seed_stock_analysis.rs:340` 已有「UPDATE 替代 DELETE 保留用户自定义变量」逻辑，需在 M2 复核其覆盖范围 |
| `ValueConfig` 仍无消费方（D3 未彻底）   | 低   | 技术债残留                   | 已在 D3 明确留待评估，不阻塞                                                                           |

---

## 6. 工作量与拆分

| 里程碑 | 改文件                                                             | 预估改动量                  | 编译成本                             |
| ------ | ------------------------------------------------------------------ | --------------------------- | ------------------------------------ |
| M0     | `astock-data/src/mcp_tools.rs`                                     | +60 行（解析 + 2 测试）     | `check -p axagent-astock-data`（快） |
| M1     | `stock_analysis_setup/seed_stock_analysis.rs`                      | +30 行（闭包签名 + 调用点） | `check -p axagent`（慢，与 M2 合并） |
| M2     | 同上文件 :254                                                      | 1 行 + 注释                 | 与 M1 同批                           |
| M3     | `StockAnalysisConfigPanel.tsx` / `WhatIfBacktest.tsx` / 11 locales | ~80 行                      | tsc + dprint（秒级）                 |
| M4     | 无                                                                 | —                           | 重编译二进制 + 实跑                  |

---

## 7. 启动检查清单（M0 开工前）

- [ ] 确认 `astock-data` 无其他并行编译任务（避免 cargo 锁冲突）
- [ ] 备份 `output/backup-2026-09-12/`（`mcp_tools.rs` / `seed_stock_analysis.rs`）
- [ ] 记录当前 DB 模板 `version` 与 t-valuation 的 `input_mapping` 快照（用于 M4 对比）
- [ ] 记录基线样本：`002353` 当前 `dcf.mid`（路径 Y 后应变化，作为 M0 的「不变」基准）

---

## 8. 实施进度（2026-09-12）

| 里程碑            | 状态            | 实际改动                                                                                                                                                                                                    |
| ----------------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **M0** 参数扁平化 | ✅ 完成         | `mcp_tools.rs`：新增 `ValuationConfig::from_flat_arguments`（百分数 → 小数单点换算 + 越界守卫）、`compute_valuation` 加 `.or_else(...)`、`MAX_PERPETUAL_GROWTH` 具名常量（T8）；新增 5 条单测               |
| **M1** 种子接线   | ✅ 完成         | `seed_stock_analysis.rs`：`tool_node` 闭包新增 `extra` 参数（一对一「工具参数名 → 变量名」多映射）；`VALUATION_FLAT_ARGS` 常量；t-valuation 挂 3 个 `value_dcf_*`；`td_val` 的 ToolDef schema 补齐 3 个参数 |
| **M2** 模板升版   | ✅ 完成         | `TEMPLATE_VERSION` 31 → **32** + v32 版本注释（含未接线的 3 个阈值变量说明）                                                                                                                                |
| **M3** 前端对齐   | ✅ 完成（M3-a） | 设置面板侧由 M1 闭环；WhatIf 侧按 M3-a **重命名接线** —— 见 §8.1                                                                                                                                            |
| **M4** 生产验证   | ⏳ 待做         | 需重编译二进制 + 实跑；门禁已过（`cargo check -p axagent` 通过、`tsc`/`dprint`/`oxlint`/`i18n`/`check:eol` 全绿）                                                                                           |

### 8.1 M3 的核实与实施（M3-a 已落地，2026-09-12）

> ⚠️ **本节更正上一轮（同日早些时候）的结论。** 上一轮写「WhatIf 面板的滑块是幻觉 UI
> —— 全部零消费」，依据是 grep `portfolio-mgr.rhai` 的 80 个 `present()`。**该结论部分错误**：
> WhatIf 的滑块接的不是 `compute_what_if`，而是 `replay_tool_chain` —— 一条 **Rust 原生简化链**，
> 它确实消费 `configOverrides`。上一轮只查了 Rhai 脚本，漏看了这条链，导致把「3 个失效」误判为
> 「全部失效」。教训：**判定「参数是否被消费」必须穷举所有执行链，而非只查其中一条。**

#### 真实消费面（`replay_tool_chain`，`src/commands/stock_analysis.rs`）

| 面板原滑块                                                                                                    | 后端消费 key              | 生效？                                         |
| ------------------------------------------------------------------------------------------------------------- | ------------------------- | ---------------------------------------------- |
| `scoring_trend` / `scoring_deviation` / `scoring_macd` / `scoring_volume` / `scoring_rsi` / `scoring_support` | 同名（`tv()` 直读）       | ✅ 生效                                        |
| `kelly_fraction`                                                                                              | `kelly_fraction`          | ✅ 生效                                        |
| `risk_max_drawdown_limit`                                                                                     | `risk_max_drawdown_limit` | ✅ 生效，但**默认值分叉**（面板 15 / 后端 20） |
| `value_dcf_growth_rate` / `value_dcf_discount_rate` / `value_safety_margin`                                   | 无                        | ❌ 不生效                                      |

另有一处**命名错配**：后端读 `fscore_buy_threshold`，而变量表与面板叫 `value_fscore_buy`
→ 面板传值永不命中，恒走默认 7.0（又一种「空接线」形态：消费点存在但对不上名）。

⇒ 准确结论是：**10 个滑块中 7 个生效、3 个失效**；且决策层参数（`action_*` / `pos_cap_*` /
`regime_prior_*` 等 `PORTFOLIO_MGR_TUNABLE_PARAMS` 成员）**完全没有 UI 入口**，
只能经 `blackboardSnapshot` 被动注入。

#### M3-a 实施内容

**后端（`stock_analysis.rs`）**

1. `WhatIfRequest` 新增 `param_overrides: HashMap<String, Value>` —— 决策层参数直传入口，
   优先级**高于** `blackboard_snapshot`（快照注入时主动跳过同名 key，避免 scope 内重复压栈
   导致 `present()` 语义变形）。
2. 注入逻辑抽出 `push_json_to_scope()`，供「快照注入」与「覆盖注入」共用，
   消除两套重复的类型分派（AGENTS.md 铁律 12）。
3. `replay_tool_chain` 把 `config_overrides` 中**非自消费**的 key 透传给 `compute_what_if`
   —— **这是本次的关键接线**：此前面板即使传 `pos_cap_high` 也进不了 Rhai。
4. 新增常量 `REPLAY_CONSUMED_OVERRIDE_KEYS`（13 项）显式登记回放链自消费项，
   避免同一参数同时影响「简化评分链」与「决策层公式」两处、导致调试无法归因。
5. 修正两处错配：`fscore_buy_threshold` → `value_fscore_buy`；
   `risk_max_drawdown_limit` 默认 20 → 15（对齐变量表 / 设置面板）。

**前端（`WhatIfBacktest.tsx`）**

1. 删除 3 个失效的 `value_dcf_*` 滑块 —— What-If 的估值输入来自快照，
   该场景原理上不重算 DCF。
2. 参数改为**按后端真实消费方分组**渲染，6 组 37 项：评分权重 / 决策阈值 / 仓位上限 /
   市况先验 / 风险分档 / 其他（凯利·成本·F-Score）。分组标题即点名消费链路，
   便于日后复查「面板可调但引擎不读」。
3. 默认值与 i18n 标签取自 `getDefaultVariables()`（`StockAnalysisConfigPanel` 改为按名导出）
   —— 单一权威源，不再复制第二份清单（这正是 `risk_max_drawdown_limit` 默认值分叉的成因）。
4. **回放结果回显**：修复前 `replay_tool_chain` 的返回值被 `await` 后**直接丢弃**，
   用户调完参数点「应用配置到后端重算」看不到任何反馈，无法判断调参是否生效
   —— 属于「接线了但不可观测」。现展示「综合评分 / 决策 / 仓位 / 风险等级」，
   并透出 `dataDegraded` 降级告警。顺带修掉「未选记录时提前 `return` 导致按钮永久转圈」。

**i18n**：新增 11 个 key × 11 语言（6 个分组标题 + 5 个回放结果文案），共 121 条。

#### 本次未做（需另立需求）

- `computeDecisionBackend`（主路径，走 `blackboardSnapshot`）未接 `paramOverrides`：
  其参数语义是「实验参数」而非「配置覆写」，且已有快照通道可用。
- M3-b（移除无效滑块）已被 M3-a 取代 —— 失效项不再删除，而是**替换为有效项**。

#### M3-a 验证结果（2026-09-12）

| 检查                                                                                                      | 结果                         |
| --------------------------------------------------------------------------------------------------------- | ---------------------------- |
| `cargo test -p axagent-astock-data --lib`                                                                 | ✅ **315 passed / 0 failed** |
| `cargo check -p axagent`                                                                                  | ✅ 零 error（8m13s）         |
| `cargo clippy -p axagent-astock-data --all-targets -- -D warnings`                                        | ✅ 1m01s 零 warning          |
| `cargo clippy -p axagent --all-targets -- -D warnings`                                                    | ✅ 8m17s 零 warning          |
| `tsc --noEmit` / `dprint check` / `oxlint` / `check:eol` / `check:i18n` / `check-hardcoded-i18n --strict` | ✅ 全绿                      |
| 37 项参数 i18n 标签在 11 语言中的覆盖                                                                     | ✅ 零缺失                    |

**clippy 抓出的真错误**（`-D warnings` 的价值体现）：M1 把 `algo_tools` 由六元组改七元组后，
`clippy::type_complexity` 直接判失败（`seed_stock_analysis.rs:2081`，七元组含嵌套切片）。
抽为模块级 `type AlgoToolRow` 后通过。**这是同一处元组结构改动的第二次踩坑**
（第一次是两处 `.map(|(id,_,_,_,_,_)| ...)` 解构遗漏）⇒ 改动元组结构时必须同时检查：
① 全部解构点；② 类型注解是否触发 `type_complexity`；③ 是否该直接换成 named struct。

#### 尚待完成：M4 生产验证

需重编译二进制 + 重启应用（触发 v32 模板重种子）+ 选一条历史记录实跑，
核对 What-If 面板调参后 `replay_tool_chain` 的返回值确实变化。
