# DCF 周期顶点现金流锚归一化（赛力斯 601127 实证）实施计划

## 一、仓库调研结论（已完成，含精确对账）

### 1.1 事故复现：四个展示数字与代码常量精确闭合

用户截图：现价「五十多」、DCF 区间 **412.62–1400.75 元**、安全边际 **683.0（%）**、财务健康「差」。

以仓内真实常量（`valuation.rs` 两阶段模型：中性 d=7.7%、p=1.3%；保守档 d=8.7%、g×0.6、p×0.7；乐观档 g×1.5 上限 45%、p 上限 1.7%；5 年期、终值利差下限 1.5pp）反解：

| 量 | 值 | 闭合验证 |
|---|---|---|
| 现价 | 52.70 元 | `(412.62−52.70)/52.70 = 682.96%` = 截图 683.0 ✓（它是**上行空间%**，不是中性档——上一轮我一度误读为中性档，已更正） |
| 基准增速 g | **30.0%（机械上限）** | `robust_annual_growth` 含 +104%/+129%/+85% 等爆发年，截尾均值仍 ≥30%，被 `MAX_GROWTH=0.30` 钳死 |
| 每股锚定 FCF | **15.90 元** | 保守档单位倍数 25.95 × 15.90 = 412.6 ✓；乐观档单位倍数 88.11 × 15.90 = 1400.9 ✓ |
| 年锚定 FCF | **≈239 亿**（总股本 ~15.05 亿） | 与「最近完整年报 FCF（OCF−资本开支）」口径一致 |
| 锚 FCF 收益率 | **≈30.2%**（市值 ~793 亿） | 所有现有护栏均不触 |

**结论：模型数学自洽，错在输入。** 赛力斯年报 OCF 远超净利（FCF/净利 ≈ 4），差额主要来自扩张期应付账款/合同负债随营收翻倍而膨胀——**营运资本一次性释放**，不可持续、不能复利。市场只给约 3.3× 锚 FCF 的定价，模型按永续给了 20×+。

### 1.2 为什么现有五道闸门全部放行（设计缺陷，非偶发）

锚选择与闸门代码位置：

- 锚选择：[mcp_tools.rs 4081-4125](file:///d:/OneManager/AxInvest/src-tauri/crates/astock-data/src/mcp_tools.rs#L4081-L4125)
- 适用性判据：[mcp_tools.rs 4341-4498](file:///d:/OneManager/AxInvest/src-tauri/crates/astock-data/src/mcp_tools.rs#L4341-L4498)
- 主口径资格（结论层）：[valuation.rs 551-589](file:///d:/OneManager/AxInvest/src-tauri/crates/astock-data/src/valuation.rs#L551-L589)

1. 判据 ② 只有**下半区** `FCF/净利 < 0.3`（`FCF_NP_DIVERGENCE_MIN`），没有上半区——FCF 是净利 4 倍时无信号。
2. 锚归一化只有**下半区** `FCF/净利 < 0.6`（`FCF_TO_NP_MIN_FOR_ANCHOR`）换所有者收益代理锚；上半区不存在。
3. 主口径资格两条地板（FCF 收益率 ≥1%、乐观档覆盖现价 ≥100%）全是**下界**——30% 收益率、26 倍覆盖率轻松越过，反向 DCF（此时会输出「现价隐含 FCF 崩塌」）**不启动**。
4. 量程守卫上限是「现价 ×100」=5270 元，8 倍偏差无感。
5. 「财务健康：差」是 LLM 另一路输出，与 DCF 无交叉校验（本次不动它，理由见 §5 范围）。

这是 301269 / 600276 / 600887 / 300308 / 300642 之后**同一病根的反向形态**：前几例锚被系统性扣小（扩张性资本开支、单期污染），本例锚被周期顶营运资本**抬高**。前 20 次修复都是「单标的 + 单侧门」，本方案补的是**对称的结构性缺口**，不写任何行业/标的特判。

### 1.3 设计原则（防止第 21 次回归）

- **只锚数据形态，不锚行业标签**（沿用模块铁律）。
- **优先换锚，不轻易关模型**：与下半区 0.6 降级线对称处理——换成跨周期代理锚 + `is_fallback_anchor=true`（f5 自动降权），而不是又加一道「不适用」把腿砍掉（判据 ③ 撤销史已证明滥砍的代价）。
- **两道防线复用既有机制**：① 锚选择层归一化（治疗）；② 主口径资格补 FCF 收益率**天花板**（兜底，与 1% 地板严格对称）。
- 增长率 30% 钳位**本次不改**：换锚后低档位 ≈ 现价，结论自动回归「合理」；改增长模型是独立的大改动，夹带只会制造新回归。

## 二、改动文件与模块

- `src-tauri/crates/astock-data/src/mcp_tools.rs`
  - 常量区（~L3102 `FCF_TO_NP_MIN_FOR_ANCHOR` 附近）：新增 2 个常量 + 完整文档注释（含赛力斯实证表）。
  - 新增 helper：年报 FCF 序列（含负值年份），供峰值检测。
  - `compute_dcf` 锚选择（L4085-4125）：在现有 `<0.6` 代理臂后新增**上半区峰值代理臂 ①c**。
  - 不新增 applicability 判据（理由见 §1.3/§4）。
- `src-tauri/crates/astock-data/src/valuation.rs`
  - 新增 `FCF_YIELD_MAX_FOR_DCF_PRIMARY = 0.15`（与 `FCF_YIELD_MIN_FOR_DCF_PRIMARY=0.01` 对称的天花板，含实证余量文档）。
  - `build_conclusion` 的 `dcf_ok` 合取天花板条件；`dcf_reject_reason` 增加对应分支（headline/not_applicable 逐字复用既有机制）。
  - `ConclusionInputs` 不变（`fcf_yield` 已存在；注意 mcp_tools L1722 传的是**归一化后**的 `a.fcf_anchor`，天花板自动作用于代理锚）。
- 前端：**不改**。换臂后 `applicable` 仍为 true，区间照常显示但数值已回归合理；天花板拒主口径时区间显示策略与 600276 覆盖率失败完全相同，已有 UI 路径承接。
- i18n：**不改**。新增文案是算法层中文诊断（与现有 `fcf_basis`/`dcf_note` 同性质，前端原样渲染，从无 i18n key 先例）。

## 三、实施步骤（依赖序）

1. **新增 helper `annual_fcf_series(financials, max_years)`**：近 N 个年报（`report_date.contains("-12-31")`、`take(N)`）逐年 OCF−资本开支，**保留负值年份**，跳过 None。与 `latest_annual_fcf` 共用单年计算逻辑（抽 `annual_fcf_of(report)` 小函数，避免重复定义，守禁区 12）。
2. **新增常量**（mcp_tools.rs 常量区）：
   - `FCF_TO_NP_MAX_FOR_ANCHOR: f64 = 1.5`：稳态企业 FCF/净利 0.7–1.2（引用本仓既有判据文档原文区间），>1.5 只能来自营运资本释放或资本开支远低于折旧，均不可永续。
   - `ANCHOR_PEAK_HISTORY_MULT: f64 = 2.0`：最近年报 FCF ≥ 近 5 年年报 FCF 均值（**含亏损年份**）的 2 倍 ⇒ 周期/扩张顶点一次性形态。要求序列 ≥2 个年度值，否则不判峰值（缺数据不臆断）。
3. **锚选择新增 ①c 臂**（L4092 `Some(np)` 分支内，`<0.6` 臂之后）：
   - 条件：`v/np > FCF_TO_NP_MAX_FOR_ANCHOR` **且** 峰值检测命中（ratio 必须 `is_finite()`，防 NaN 误判）；
   - 新锚 = `normalized_annual_profit(financials, 5) × 0.90`（近 5 年报**正净利年均值**，与 ②③ 代理臂同函数同系数；无正净利年则回落 `np×0.90`）；
   - `fcf_basis` 文案：`"{anchor_basis}显著高于净利（FCF/净利=R.xx>1.5）且为近5年峰值（≥2×多年均值，含亏损年份）：增量主要来自扩张期营运资本一次性释放（应付/合同负债膨胀），不具永续性，改用近5年报正净利均值×0.90跨周期代理锚定"`；
   - 自动获得 `is_fallback_anchor=true`（L4135 判据：basis ≠ 两个真实口径常量），f5 降权与诊断面板**零接线自动生效**。
   - 赛力斯推算效果：锚 239 亿 → 正净利年均（约 0.2/24/60/70 亿四年）×0.9 ≈ 35 亿，每股 ≈2.3 元 ⇒ 保守档 ≈ 60 元（≈现价，上行 +13%，结论「合理」档），乐观档 ≈ 200 元上下。
4. **valuation.rs 天花板**：
   - 常量 `FCF_YIELD_MAX_FOR_DCF_PRIMARY: f64 = 0.15`，文档写明：健康侧实测 600406=5.9%、600887=6.6%、600276=2.75%，病侧赛力斯归一化前 30.2%，阈值有 >2 倍空档；价值投资原理上周期顶 FCF 收益率虚高正是「勿外推顶点 earnings」的经典信号。
   - `dcf_ok` 增加 `&& i.fcf_yield.is_some_and(|y| y <= FCF_YIELD_MAX_FOR_DCF_PRIMARY)`（None 不命中维持原状语义：无收益率数据不靠本门放行/否决——与地板的 None 语义对齐方式在实现时按现有 `else if` 链逐一核对，保证 None 不被天花板误杀）。
   - 拒绝原因分支（放在地板分支之后）：`"锚定FCF收益率高达X% > 15%：该现金流为周期/扩张顶点一次性释放，不具永续性，正向DCF不具备主口径资格"`，主口径自动让位反向 DCF（`reverse_dcf` 同源使用归一化锚，见 L1705-1717，输出「现价隐含 FCF 需逐年回落」的有信息量结论）。
5. **测试**（全部追加到两文件现有 `#[cfg(test)] mod` **末尾**，守 AGENTS.md 内联测试位置规则；夹具复用既有 builder，实现时先 grep `fn make_report`/`FinancialReport {` 找helper）：
   - **T1 赛力斯形态**（mcp_tools）：5 期年报——前 3 期 FCF 为负/微利且净利为负，近两期净利 60/70 亿、最新年报 FCF 239 亿；股本 15.05 亿；price=52.70。断言：锚 ≈ 正净利均值×0.9（误差 1%，绝不等于 239 亿）；`is_fallback_anchor==true`；basis 含「峰值」与「营运资本」；保守档每股 <100 元。
   - **T2 稳态高现金流不误伤**：连续 5 年 FCF/净利 ≈1.8 且最新值 <2× 多年均值 ⇒ 锚保持真实 FCF、`is_fallback_anchor==false`。
   - **T3 边界**：ratio=1.5 恰好不替换（严格 `>`）；ratio 为 NaN 不替换。
   - **T4 序列不足**：只有 1 个年度 FCF 时峰值检测不命中（即使 ratio>1.5）。
   - **T5 天花板**（valuation.rs `build_conclusion`）：fcf_yield=0.30、coverage=26、applicable=true、reverse 存在 ⇒ `primary_method=="reverse_dcf"` 且 not_applicable 文案含 15%；fcf_yield=0.10 同输入 ⇒ 主口径仍是 dcf。
   - 回归保护：现有全部 DCF 测试（600406/600887/600276/301269/000858/600519/300308/300642/301302/688114 等夹具，其收益率 ≤6.6%、FCF/净利均不在新形态内）逐个核对预期不变。
6. **格式化与门禁**：
   - `cargo fmt --all`（必须 `--all`，根 `cargo fmt --check` 不覆盖 crates）；
   - 定点：`cargo clippy -p axagent-astock-data --all-targets -- -D warnings`；
   - 定点测试（Windows）：`$env:__TAURI_WORKSPACE__='true'; cargo test -p axagent-astock-data --all-targets`；
   - 下游连带：`cargo test -p axagent-stock-analysis`（消费 assumptions 的决策层）；
   - 全部完成后按 CI 口径跑**一次**全量兜底：`cargo clippy --workspace --all-targets --all-features -- -D warnings` + `__TAURI_WORKSPACE__=true cargo test --workspace`。
7. **真实数据对账（可选，非阻塞）**：新增 `#[ignore]` 测试 `#[cfg(ignore)] fn repro_601127_live()`，用仓内既有 eastmoney client 拉 601127 年报 OCF/资本开支/净利并打印，人工 `cargo test -- --ignored` 触发一次（走应用自身网络栈，不经脚本直连），确认 239 亿量级与 OCF 字段映射无误；若实测锚并非来自 OCF-峰值而是字段错位，则回查 vendor 映射（`eastmoney.rs` 的 `NETCASH_OPERATE`/`CONSTRUCT_LONG_ASSET`），本计划主体仍成立但需另开修复。

## 四、依赖与注意事项

- `reverse_dcf` 与正向 DCF 共用 `DcfAssumptions.fcf_anchor`（L1705-1717 K1 同源纪律），换锚后反向口径自动同源，不会产生新的口径矛盾。
- 天花板作用于**归一化后**锚的收益率（L1722 `a.fcf_anchor / mv`）：赛力斯归一化后约 4.4%，正常过关；天花板只兜底未被 ①c 捕获的其他膨胀形态。
- 不改增长率模型、不改前端、不加 applicability 新判据、不动 rhai——三处刻意不做，控制爆炸半径。
- 浮点：所有比值判断加 `is_finite()`；严格不等号方向在 T3 锁定（本仓有 lint 机械建议改坏 NaN 语义的前科）。
- 文案中数值与常量必须 `format!` 插值，禁止硬编码阈值字面量（守 L4061-4065「文案不许对常量撒谎」的同源教训）。

## 五、明确不做（范围边界）

- 「财务健康：差 × DCF 极度低估」的**通用交叉冲突告警**（需要 LLM 标签参与，跨 prompt/前端/冲突引擎，属独立需求）——本次根因消除后该并置不会再出现；如后续仍要通用机制，单开 PLAN。
- 增长率均值回归/H-model 衰减（30% 钳位替换为逐年 fade）——独立模型变更，影响全部标的与大量基线测试。
- 量程守卫、面板渲染、i18n、rhai 脚本均不动。

## 六、验证清单

- [ ] T1–T5 新测试通过
- [ ] astock-data 既有全部测试零改动通过（逐个夹具确认预期未漂移）
- [ ] stock-analysis 下游测试通过
- [ ] `cargo fmt --all --check` 干净
- [ ] 定点 clippy `--all-targets -D warnings` 干净
- [ ] 全量 clippy + workspace 测试兜底一次
- [ ] 赛力斯形态手算：修复后保守档/上行空间/主口径三点与计划推算一致（≈60 元 / ≈+13% / dcf 或 reverse_dcf），不再出现 412-1400 与 +683%

## 七、风险与应对

- **风险 1：误伤真实「现金牛」**（FCF 持续高于净利，如类加盟/轻资产）。应对：①c 是**双条件**（ratio>1.5 **且** ≥2× 自身多年均值），稳态高现金流 T2 锁死不命中；另有 T4 数据不足保护。
- **风险 2：正净利年均值对「困境反转股」仍偏高**。应对：代理臂标记 `is_fallback_anchor=true`，f5 已按既有规则降权；且 ④ 号判据（亏损+低收益率）与天花板仍在，最坏情况是主口径让位反向 DCF，不会出虚假买点。
- **风险 3：真实锚并非 OCF 峰值而是 vendor 字段错位**。应对：步骤 7 的 `--ignored` 真实对账；如证实字段问题，vendor 映射另修，本方案的对称闸门作为防线保留。
- **风险 4：全量门禁耗时**。应对：遵循 AGENTS.md 分层策略，阶段内定点、收尾全量（sccache 已配 50G）。
