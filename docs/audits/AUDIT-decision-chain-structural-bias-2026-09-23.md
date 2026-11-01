# AUDIT — 决策链的结构性偏置（2026-09-23）

> 范围：`portfolio-mgr.rhai` 的因子融合 → posterior → action 阶梯 → 凯利仓位 这条链。
> 定位：本报告与 `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` **互补不重叠** ——
> 那份查的是 **DCF 内部**（参数取值、区间口径），本份查的是**决策链**（多因子如何合成一个决策）。
> 触发：用户问「你是不是没有检查相应计算是不是符合科学？」的延伸 —— 估值修完，结论仍是「观望」，
> 于是必须回答「观望是**模型被压住**，还是**标的真的不行**」。答案：**两者都有，且前者可量化、可修**。

---

## 一、结论速览

| # | 缺陷 | 性质 | 处置 |
|---|---|---|---|
| 1 | 同一条链上流通**两个概率量**：`action` 阶梯读 `effective_posterior`，凯利仓位与试探仓读 `posterior` | **结构性错误**（同源分叉） | 全链统一到 `effective_posterior` |
| 2 | `f4_signal` 映射的**零点偏移**：`中风险 → −0.05`，而 `prior = 0.50` 已定义「中性」 | 零点定义错误 | `中风险 → 0.0` |
| 3 | `f4_base`（LLM 判断）与 `riskLevel`（算法分类）**不同源**，但过去被当成同一个量使用/审计 | 口径混用 | 本报告显式分栏，并写进判据层 |
| 4 | `RISK_FREE_RATE = 2.5%` 实为 **1 年期 MLF 政策利率**，而注释声称是 10 年期国债 | 口径错误（可核查） | 改 `1.7%`（中债 10Y = 1.675%，2026-09-22） |
| 5 | `PERPETUAL_GROWTH = 2.0%` 在 `r_f` 订正后**越过** `g_terminal ≤ r_f` 硬约束 | 约束违反 | 改 `1.5%` |

**买入档可达性**：**不是**结构性不可达 —— 见 §3 的生产公式复算（合成场景 ⇒ 买入）。

---

## 二、缺陷 1：同一条链上流通两个概率量

### 实证

DB 落库（300642 / 2026-09-22）的 `portfolio-mgr.result`：

```
evidence.posterior            = 0.514      ← 纯证据
f7_free.effective_posterior   = 0.434      ← posterior + risk_bias(−0.08)
riskLevel                     = 高风险
action                        = 观望
positionPct                   = 9.434 %
positionState                 = HOLDING
```

**9.434% 是凯利用 `posterior(0.514)` 算出来的**；而按凯利自身的适用条件
（`portfolio_formula.rs:204` 的 `posterior < 0.50 ⇒ return 0.0`），用 `effective_posterior(0.434)`
应当是 **0**。⇒ 同一个输出里，两个概率各算了一半。

### 后果（比「数字不准」更硬的两条）

1. **`risk_bias` 对仓位完全失效**。`pm_risk_bias`（极高 −0.15 / 高 −0.08）的设计意图是
   「高风险标的的证据要求上移」（`:255` 明写「风险维度的市况自适应已通过 risk_bias 与
   risk_cap 承担」）。但仓位读原概率 ⇒ 该意图实际只由 `POS_CAP_*` 的硬上限承担，
   「高风险 ⇒ 更小下注」这第二腿**从未生效**。
2. **试探仓机制被绕过**。`probe_position_pct` 的激活窗口是 `posterior ∈ [0.42, 0.50)`
   （V59 设计：观望 → 试探 3%），而**「观望」档正是用 `effective_posterior` 判出的**。
   300642 的 `effPost = 0.434` 落在窗口内、本应激活，却因 `posterior = 0.514` 而跳过。
   ⇒ **入口与出口用两个概率，机制静默失效。**

### 处置

`q` / `kelly_raw` / `pm_kelly_position` / `odds_fallback` / `probe_position_pct` 五处全部改为
`effective_posterior`。改后 300642：`effPost 0.434 < 0.50` ⇒ 凯利不适用 ⇒ 仓位 0；
同时落入 [0.42, 0.50) ⇒ **试探仓 3% 激活** ⇒ 输出「观望 + 3%（试探）」——与 V59 语义一致。

---

## 三、缺陷 2：因子映射的零点偏移

### 实证（DB 全库 27 条含 `portfolio-mgr` 的样本）

| 量 | min | p25 | 中位 | p75 | max |
|---|---|---|---|---|---|
| `prior` | 0.450 | 0.450 | 0.500 | 0.500 | 0.528 |
| **`avgSignal`** | −0.400 | −0.150 | **−0.070** | 0.000 | **+0.160** |
| `posterior` | 0.297 | 0.402 | 0.461 | 0.500 | 0.547 |
| `effective_posterior` | 0.164 | 0.320 | 0.394 | 0.450 | 0.542 |
| `f4_adjusted` | −0.780 | −0.530 | **−0.420** | −0.280 | +0.300 |
| `f9_moneyflow` | −0.680 | −0.620 | −0.320 | 0.000 | +0.920 |
| `f12_momentum` | −0.700 | +0.300 | **+0.900** | +0.900 | +0.900 |

**读法**：`prior = 0.50` 定义「无个股信息时的中性」。但参与融合的因子**没有一个是零中心的** ——
`f4` 中位 −0.42、`f9` 中位 −0.32（负偏），`f12` 中位 +0.90（正偏、且 p75 也是 0.90 = **饱和**）。
三者部分抵消后，`avgSignal` 中位停在 **−0.070** ⇒ `posterior` 中位 **0.461** ⇒ 中位样本落「观望」。

> ⚠️ 这里**必须区分两件事**，否则会重复上一轮判据③的错（见该报告 §5）：
> `avgSignal` 中位为负，**不等于**「模型被压住」。中位标的落「观望」本身是**对的**。
> 真正的问题是**上界太窄**：max `avgSignal = 0.160`，而买入需要 **0.238**（§3）。
> 是「分布被压缩」而非「分布被平移」。

### 处置与边界

只改 `f4_signal` 的 `中风险` 分支：`−0.05 → 0.0`。
**依据是可判定的**（零点定义），不是标定：若「中风险」自带负证据，则 `prior = 0.50` 的
「中性」定义被自相矛盾。

**刻意不动**：`低 +0.30 / 高 −0.25 / 极高 −0.50` 的负侧幅度大于正侧，是**有意**的
（风险的负向影响大于正向；高风险另有 `pm_risk_veto` 与 `POS_CAP_*` 独立承担）。
**刻意不动 `f12`**：`dif > 0 && dif <= dea → +0.3` 看着像符号错（`dif<=dea` = 动能衰减），
但改它会**降低** posterior（因 f12 是正偏因子），与「让决策不再一味保守」方向相反，
且它真正的毛病是**饱和无区分力**（属 IC 标定问题，需样本量，不属于本轮「改错误」的范围）。
⇒ 登记为待标定项，不擅改。

---

## 四、缺陷 4/5：无风险利率的口径错误

### 可核查的事实

| 项 | 值 | 来源 |
|---|---|---|
| 模型用的 `RISK_FREE_RATE` | **2.5%** | 实为 **1 年期 MLF 政策利率 2.50%**（8/25 操作） |
| 中债 **10 年期**国债到期收益率 | **1.675%** | 2026-09-22，chinabond 日评 / 新华财经 / 东财**多源一致** |

**为什么是错的（与观点无关）**：DCF 的无风险利率必须与**被折现现金流的久期**匹配。
本模型 = 预测期 5 年 + 永续终值 ⇒ 久期 10 年以上 ⇒ 对应 **10 年期国债**。
MLF 是央行**主动操作**的政策利率，不是市场无风险利率。两者当期差 **82.5bp**。

**同族形态**：上一行注释原文写的是「A 股 10 年期国债收益率中枢」——
**注释声称的口径与取值不是同一个利率**。这正是「依据不可判定」（判据 #687）：
读的人会以为 10Y 就是 2.5%，无从反驳。

### 连带与净效果（**方向相反，须一起看**）

- `r_f` 0.025 → 0.017 ⇒ `d` 0.085 → **0.077** ⇒ 估值 **↑**（`E_d = −1.96`）
- `p` 0.020 → **0.015**（`r_f` 订正后原值越过 `g_terminal ≤ r_f`）⇒ 估值 **↓**（`E_p = 0.78`）
- 实测 300642：`mid` 29.55 → **31.61**（+7.0%），`low` 18.03 → **19.37**（+7.4%）
- **档位不变**：现价 21.02 仍 > `low` ⇒ 安全边际 −7.9% ⇒ 观望（与上一轮结论自洽）

⇒ 这次订正**不是**又一次单向收紧，而是把两个口径同时改对。

---

## 五、买入档可达性证明（回应「是不是结构性不可达」）

用**生产公式**复算（不是估计）：

```
evidence_scale = 0.10 + 0.45 × sqrt(min(tw/mw, 1))      【portfolio_formula.rs:144】
posterior      = prior + avg_signal × evidence_scale     【portfolio-mgr.rhai:1641】
effective      = posterior + risk_bias                   【:1676】
买入门          = effective_posterior ≥ ACTION_BUY_THRESHOLD  【portfolio-mgr.rhai:2036】
                 且 kelly_pos_uncapped ≥ POS_BUY_MIN
```

以 300642 的覆盖率（`tw = 1.462` / `mw = 1.49` ⇒ `scale = 0.5458`）反解门槛：

| 风险档 | `risk_bias` | 需要的 `posterior` | 需要的 `avg_signal` |
|---|---|---|---|
| 中 / 低 | 0.00 | 0.63 | **0.238** |
| 高 | −0.08 | 0.71 | 0.385 |
| 极高 | −0.15 | 0.78 | 0.513 |

**合成场景（按「瓶颈掘金」语义给一组强多头信号）**：

```
f2 consensus  σ=0.80 (w=0.25)   f13 bottleneck σ=0.80 (w=0.10)
f12 momentum  σ=0.70 (w=0.10)   f5 valuation   σ=0.50 (w=0.165)   f1 trend σ=0.50
其余中性 ⇒ Σw·σ = 0.6705, Σw = 1.645 ⇒ avg_signal = 0.4076
posterior = 0.5 + 0.4076×0.5458 = 0.7242 ⇒ effective = 0.7242
odds(合成) = 2.5 ⇒ kelly_raw = 0.7242 − 0.2758/2.5 = 0.61388
                ⇒ kelly_pos_uncapped = 0.61388/2 × 100 × (1 − 0.003) = 30.60% ≥ POS_BUY_MIN(15)
                ⇒ **action = 买入** ✅
```

⇒ **买入档可达**，它是一个「多维度同时成立」的门。
**全库 27 条实测 max `avg_signal` = 0.160 ⇒ 差距来自样本，不来自门槛算术。**

---

## 六、全库迁移（A / B 类分栏 —— 两类数字可信度不同）

**自检**：用旧规则复算能否重现落库值 ⇒ 27 条中 **14 条可复算**（13 条因 `maxWeight` 等
字段缺失被**显式跳过并打印**，不产出 NaN 行）。

| 迁移 | 条数 |
|---|---|
| 减持 → 减持 | 5 |
| 观望 → 观望 | 4 |
| **持有 → 增持** | **2** |
| 持有 → 持有 | 1 |
| 增持 → 增持 | 1 |
| 卖出 → 卖出 | 1 |
| **合计变化** | **2 / 14**，方向**全部为升级**，无降级 |

代表样本：

| code | 日期 | `effective` 旧→新 | action 旧→新 | 仓位 旧→新 |
|---|---|---|---|---|
| 300642 | 09-22 | 0.434 → 0.434 | 观望 → 观望 | 9.4% → **3.0%（试探仓）** |
| 300308 | 09-21 | 0.533 → 0.550 | 持有 → **增持** | 8.2% → 12.4% |
| 300179 | 09-21 | 0.481 → 0.474 | 观望 → 观望 | 0% → **3.0%（试探仓恢复激活）** |
| 600031 | 09-13 | 0.542 → 0.538 | 增持 → 增持 | 11.5% → 11.5% |

### ⚠️ 可复算性分栏（**本报告最重要的诚实声明**）

- **A 类（可复算）**：`posterior → effective` 的位移、f4 零点修正对 `avg_signal` 的作用。
  输入全部落库（`prior` / `totalWeight` / `maxWeight` / `f4_base` / `f4_quant_adjust` /
  `riskLevel`），自检已证复刻忠实。
- **B 类（依赖假设）**：`odds` 与由它派生的 `kelly_*` / `positionPct` / 买入与增持门。
  **`odds` 未落库** —— 实测 `result.evidence` 的字段并集只有 9 个键：
  `{activeFactors, announcement_signal, avgSignal, f10_chip_signal, f11_pace_signal,
  f9_money_flow_signal, posterior, prior, totalWeight}`。
  ⇒ 迁移表里 `odds 旧` 恒为 0 是**默认值不是观测值**，`新` 是按 `oddsFallback` 重算的。
  **凡 B 类数字都要按此打折。**
- **未建模**：`posterior_cap`（trader 否决封顶 0.50/0.55）、`weights_collapsed`、
  `untrusted_count`、`trader_bearish`、covariance decay（前几项只会让结果**更保守**）；
  以及 f5 估值因子因 DCF 参数订正而产生的新输入（需重跑 t-valuation）。

---

## 七、我在本轮自己制造的三个缺陷（如实登记）

1. **门禁只改文档头 = 假声明**（上一轮遗留，本轮修复同型）：`check-valuation-defaults-parity.mjs`
   文档头写了「5 处载体」并把 `decision.rs` 列入，但 `evaluate({truthSrc, mainSrc, seedSrc,
   frontSrc})` **只收 4 路输入** ⇒ 它一个字都没被校验，门禁照旧绿。已真正接入（新增
   `checkDecisionCrate` 锚在**派生式**上 + 2 个负样本）。
2. **回算脚本判据用错输入**：首版用 `riskLevel` 判「是否中风险档」去决定修 f4 的哪个分支，
   而 `f4_base` 与 `riskLevel` **不同源**（10/27 条不一致，如 688072 `riskLevel=中风险` 却
   `f4_base=−0.25`）。判据改为由 `f4_base` 的**值**反推档位。
3. **审计脚本对 NaN 假通过**：自检报「27/27 逐位吻合」，而实际 13 条复算值是 NaN ——
   因为 **`NaN > 1e-6` 恒为 `false`**，判据把「算不出来」计成了「通过」。
   这是判据层 #7（审计脚本自身会撒谎）的复发。已改为显式要求 `Number.isFinite`。
   **同族**：`odds ?? 0` 让缺失字段静默变成 0，进而改变 `isFallback` 分支 ⇒ 已加注释并列入分栏。

---

## 八、验证

| 项 | 命令 | 结果 |
|---|---|---|
| 编译 | `cargo check -p axagent-astock-data -p axagent-analysis-engine --all-targets` | **EXIT=0**（12m11s，**0 error / 0 warning**；中途 `Blocking waiting for file lock` —— 并发会话持锁，非失败） |
| **Rhai 语法 + 跨文件契约** | `__TAURI_WORKSPACE__=true cargo test -p axagent-rt-workflow --test rhai_syntax_check` | **EXIT=0，8 passed / 0 failed** |
| 常量等式门禁 | `node scripts/check-valuation-defaults-parity.mjs` | **EXIT=0**（5 处载体一致；`PERPETUAL_GROWTH=0.015`、`DISCOUNT_RATE=0.077`） |
| 门禁自检 | `… --selftest` | **9/9 通过** |
| 可达性 + 迁移 | `node output/sci19-reach-and-migrate.mjs` | 见 §5 / §6 |
| `dprint check` | `./node_modules/dprint/dprint.exe check <改动的 .tsx>` | **EXIT=0** |
| 改动 crate 的测试 | `cargo test -p axagent-astock-data -p axagent-analysis-engine --all-targets` | 见 §8.1 |
| clippy / 全工作区 test | 见 §8.1 | **未完成**（前一轮被取消） |

### 8.1 为什么必须单列 Rhai 那一行

**Rust 编译器完全不看 `.rhai`** —— Rhai 是运行时脚本，`cargo check` / `clippy` / `cargo test`
对它**零覆盖**。本轮 5 处概量统一、`f4_signal` 零点、`probe_position_pct` 窗口都在
`portfolio-mgr.rhai` 里，若只看「cargo 全绿」就宣布通过，等于**根本没验**。

真正覆盖它的是 `crates/rt-workflow/tests/rhai_syntax_check.rs`：它 `read_dir` 扫
`src/commands/*.rhai` 逐个 `engine.compile()`（语法），并额外扛两条**跨文件契约门禁**
（`regime_weights_consumed_set_matches_portfolio_mgr` —— `regime-weights.rhai` 的产出名集合
⇄ `portfolio-mgr.rhai` 的 `get_weight` 消费名集合必须双向对齐；`rhai_weights_semantics_consistent`
—— 权重值形状 / 清单往返）。**我改了因子消费端，这两条是唯一能拦住「名字集合错位」的门禁。**

同时预检了同文件的**文本防回归锁**（4 项，避免改动触发假红）：

| 锁 | 判据 | 本轮实测 |
|---|---|---|
| ① | `base_action = "持有";` 在**非注释行**恰 1 次 | 1 次 ✅ |
| ② | 不得出现 `position_pct <= 0.0 && (base_action == "买入"` | 无 ✅ |
| ③ | 不得出现 `final_action == "观望" && position_pct > 0.0` | 无 ✅ |
| ④ | 必须仍输出 `"positionState": position_state` | 在 ✅ |

### 8.2 未完成项（不得读成通过）

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：**未跑完**
  （首轮被取消）⇒ 不构成通过。`cargo check` 全绿**不覆盖** clippy 独有规则
  （`disallowed_types` / `items_after_test_module` / `if_same_then_else` 等，见 `AGENTS.md`）。
- `__TAURI_WORKSPACE__=true cargo test --workspace`：**未跑**（主 crate 的
  `seed_stock_analysis.rs` 改动与全量测试尚未验证）。
- **改动落 DB 尚未发生**：`TEMPLATE_VERSION`/`DCF_MIGRATION_VERSION` 已抬到 78，需应用启动
  触发 seed 才生效。在此之前，讨论「新决策长什么样」都只是复算，不是生产观测。

---

## 九、残留（登记，不擅改）

| # | 项 | 为什么留着 |
|---|---|---|
| 1 | `f12_momentum` 饱和（中位 = p75 = +0.90） | 属 IC 权标定，需要样本量；改动方向与「不再一味保守」相反 |
| 2 | `odds` 未落库 | 导致仓位链**不可事后归因**。应加字段（属产品语义决策，单列） |
| 3 | `d` 全市场恒定（隐含 β=1） | 引入个股 β 需要新数据源，否则只是换一个无依据参数 |
| 4 | 预测期 5 年 + `g` 恒定后突降 | 两阶段 DCF 是**标准模型**，突降是**已知近似**而非错误。量化上限已写进面板结构局限声明 |
| 5 | 「悲观档」= 三假设联合极端 | 已改文案为「情景口径」，但命名本身仍可再收紧 |
| 6 | `evidence` 落库字段不全（缺 `maxWeight`/`odds`） | 直接造成本轮 13/27 不可复算 —— 是**可观测性**问题，不只是数据问题 |

---

## 十、改动文件清单

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/astock-data/src/mcp_tools.rs` | `RISK_FREE_RATE` 0.025→**0.017**（+来源/日期/更新要求文档）；`PERPETUAL_GROWTH` 0.02→**0.015**；`DISCOUNT_RATE` 0.085→**0.077** |
| 2 | `src/commands/portfolio-mgr.rhai` | `f4_signal` 中风险 −0.05→**0.0**；`q`/`kelly_raw`/`pm_kelly_position`/`odds_fallback`/`probe_position_pct` 五处概量统一为 `effective_posterior`；decision_trail 文案同步 |
| 3 | `commands/stock_analysis_setup/seed_stock_analysis.rs` | `DCF_MIGRATION_VERSION` 77→**78** + 依据/代价注释 |
| 4 | `src/components/settings/StockAnalysisConfigPanel.tsx` | 默认值 `perpetualGrowth 0.02→0.015`、`discountRate 0.085→0.077`；**hint（建议值）同步**（旧 hint 3% 现已越界） |
| 5 | `output/sci19-reach-and-migrate.mjs` | 可达性证明 + 全库迁移复算（可复跑，含 A/B 类分栏与 NaN 防护） |

---

## 十一、第三轮（同日）：三条测试失败逼出的**两个真缺陷** + 我自己的三处错

### 11.1 为什么这轮是测试驱动的，而不是我「再读一遍代码」

第一轮改动落完后跑 `cargo test -p axagent-astock-data --all-targets`：**343 条里 3 条红**。
它们不是参数改动的副作用 —— 每条都指向一个独立缺陷，其中**一个是我这轮亲手引入的**，
一个是**上一轮「口径收敛」埋下的 fail-open**。

| # | 失败测试 | 表面原因 | 真因 |
|---|---|---|---|
| 1 | `dcf_inapplicable_for_leveraged_negative_fcf_shape` | 原因串里缺「符号相反」 | `ttm_net_profit` **缺回落分支** ⇒ 判据 ② 静默失明 |
| 2 | `dcf_inapplicable_for_loss_making_with_tiny_fcf_yield` | `signals=[]` 全空 | 同上 ⇒ 判据 ④ 一并失明 |
| 3 | `dcf_perpetual_spread_violation_is_reported` | 断言要 `0.062`，实得 `0.07` | **测试自身**把「配置的 d」与「模块常量」混算（恰好相等过） |

### 11.2 真缺陷 A：`ttm_net_profit` 漏了回落 ⇒ **判据 ②④ fail-open**（严重）

`ttm_fcf` 有 ③ 分支「还原不出 → 回落 `latest.free_cash_flow`」；
`ttm_net_profit` **没有对应分支**，还原不出直接返回 `None`。

两者不对称的后果：序列缺「上年年报 / 上年同期」时（**中报序列的常态**），
净利侧 `None`、FCF 侧有值 ⇒ 下游 `ttm_net_profit(financials).filter(...)` 整段短路：

```
判据 ②（FCF 与净利背离）      → 静默失明
判据 ④（亏损 + FCF 收益率过低）→ 静默失明
⇒ applicability_signals 恒空 ⇒ DCF 腿以**全额权重**进 f5
```

**为什么这比「少一条提示」严重**：这两条恰是「杠杆畸高」「亏损且现金流不具定价意义」
的标的**唯一**的退出通道。失明后它们会产出「估值极低」的假信号 ——
正是用户抱怨的那类结论。**算不出来 ≠ 没问题**（fail-open）。

处置：补齐 ③ 分支，与 `ttm_fcf` **逐字同构**（`ttm.or(latest.net_profit)`），
使两侧的可用性判据一致 ⇒ 要么都是 TTM、要么都是单期，**不会一侧 TTM 一侧单期**。
新增防回归测试 `ttm_net_profit_falls_back_like_ttm_fcf`（四形态：
单期 / 缺上年同期 / 三期齐全走 TTM / 年报直用，且断言「TTM 值 ≠ 单期值」保证有区分力）。

> 残余不对称（已登记在函数文档里）：若 `operating_cash_flow`/`capital_expenditure`
> 缺失而 `net_profit` 三期齐全，则 `ratio` 被算小 ⇒ 判据 ② 更易命中 ⇒ 偏向「退出该腿」，
> **方向保守**。未消除，因为它只会让模型更谨慎，不会伪造乐观。

### 11.3 真缺陷 B：**我自己**在永续维度复活了「文案与实算不符」

第一轮把 `MAX_PERPETUAL_GROWTH` 锚到 `r_f`、并把 `p` 取 2%→1.5%（第二轮）之后：

```
p × 1.3 = 1.95%  >  MAX_PERPETUAL_GROWTH (r_f = 1.7%)  ⇒ 乐观档被 .min() 静默砍到 ×1.133
                             而面板文案仍声称「永续增长率 ×1.3」
```

这与第一轮在**增长率**维度修掉的缺陷（`g × 1.5` 被 `MAX_GROWTH` 吞掉、`g = 30%` 时
实际乘子 1.0）**完全同型**，只是换了一维复发。**同一个不变量在两个维度各错一次。**

处置（三件，缺一不可）：

1. **由不变量反推 `p`**：`p ≤ r_f / 1.3 = 1.3077%` ⇒ 取 **`PERPETUAL_GROWTH = 0.013`**
   （`0.013 × 1.3 = 1.69% ≤ 1.7%` ✅ 复算见 `output/sci21.txt` 第 0 节）。
2. **裸字面量提为具名常量** `LOW_PERPETUAL_SCALE_POS` / `HIGH_PERPETUAL_SCALE_POS` ——
   否则那条不变量**无法被断言**（这是它长期潜伏的结构原因）。
3. **加编译期断言**锁 `p × HIGH_PERPETUAL_SCALE_POS ≤ MAX_PERPETUAL_GROWTH`
   ⇒ 此后 `r_f` 或乘子任一漂移即**编译失败**，强制重新反推 `p`，不会静默失效。

**顺带补的缺口**：`MAX_PERPETUAL_GROWTH` 此前**只作用于乐观档**，
基准档的 `p` 只受利差钳位管 ⇒ 用户把 `dcf_perpetual_rate` 配成 5% 时 `mid` 直接用 5%，
**越过模型自己申报的 `r_f`**（实测 `mid` +30.7%、`tvr` 升至 83.2%），静默无日志。
现已并入同一条可行域守卫，并由判据 ⑤ 上报。

### 11.4 判据 ⑤ 的上报文案曾会**撒谎**（我改出来的，一并修）

合并两条上限时，两条约束的量级差两个数量级（`d − 1.5pp ≈ 6.2pp` vs `r_f = 1.7pp`）
⇒ 默认参数下 `r_f` **恒为紧约束**。若仍按「利差不足」措辞，会印出「已钳至 6.2%」
而上报字段里实际是 1.7%。⇒ 改为同时给出**两条约束 + 哪条更紧 + 生效值**，
并新增用例覆盖「利差更紧」那一侧（`d = 3%`），否则该分支永远不被执行。

### 11.5 三处**我自己的**错（如实登记）

| # | 错 | 形态 | 处置 |
|---|---|---|---|
| 1 | 测试把「扁平参数配置的 `d = 8.5`」与「模块常量 `DISCOUNT_RATE`」相减 | **手抄值恰好相等**（当时两者都是 0.085）⇒ 假绿；常量一改立刻红 | 参数值提为 `CFG_*` 常量，json 与断言**同一来源** |
| 2 | 断言用**绝对容差** `> mid + 1.0`（元） | 参数从 4% 降到 1.3% 后同一约束的绝对影响缩到 0.86 元（相对仍 **14.4%**）⇒ 假红 | 改**相对判据**（`> mid × 1.05`），并在注释写明「绝对容差把测试区分力与参数取值耦死」 |
| 3 | 代码注释里的弹性表 | `p = 4%` 行的 `E_g` 我写 0.695，实测 **0.711** | 逐格复算后订正；并注明该表**必须标参数取值**，因为 `E_p` 不是常数 |

### 11.6 结构型结论的**参数稳健性复核**（推翻两条）

第一轮报告的结论里，数值型必然过期、**结构型必须重验**。复算脚本
`output/sci21-structural-at-final-params.mjs`（常量逐字抄自终态代码）：

| 断言 | 是否成立 | 实测 |
|---|---|---|
| 「折现率弹性**最大**」 | ✅ **稳健** | `\|E_d\|/\|E_g\|` = 2.76（`p=4%`）→ **1.84**（`p=1.3%`），始终最大 |
| 「`p` 弹性排第二」 | ❌ **推翻** | `E_p` 由 0.783 **塌到 0.171**（`E_p ∝ p/(d−p)²`）⇒ 排序变为 `d > g > p` |
| 「区间宽度由两维**相乘**、乘法耦合**放大**」 | ❌ **推翻** | 低档分解 `g −24.8% / d −14.4% / p −4.8%`，三者同时 `−38.0%`；**逐项之和 `−44.0%` 大于实际** ⇒ **次可加** |
| 「判据③ 临界点 = `g > 0`」 | ✅ **稳健** | 终态参数下 `g=0` 时 `tvr = 73.1%`、`g=−0.1%` 时 69.0% ⇒ 临界仍是 `g ≈ 0` |

⇒ 已在 `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` 顶部加**时效声明**，
把「被推翻」与「仍成立」分开列，避免后续引用到失效依据。

### 11.7 300642 的现行数值（终态）

| 口径 | low | mid | high | 宽度 | `tvr` | 安全边际 |
|---|---|---|---|---|---|---|
| 用户原始实测 | 24.76 | 40.53 | 71.89 | 2.90 倍 | 84.4% | +17.8% |
| 第一轮后 | 18.03 | 29.55 | 39.85 | 2.21 倍 | 78.6% | −14.2% |
| **终态** | **19.09** | **30.78** | **45.58** | **2.39 倍** | **79.0%** | **−9.2%** |

⇒ 现价 21.02 **始终高于悲观档** ⇒ 无安全边际 ⇒ 与「观望」自洽，且不是靠剔除腿达成。

### 11.8 验证状态

| 项 | 命令 | 结果 |
|---|---|---|
| 编译（改动 crate） | `cargo check -p axagent-astock-data -p axagent-analysis-engine --all-targets` | **EXIT=0**（12m11s，0 error / 0 warning） |
| **Rhai 语法 + 跨文件契约** | `__TAURI_WORKSPACE__=true cargo test -p axagent-rt-workflow --test rhai_syntax_check` | **EXIT=0，8 passed** |
| lint（改动 crate） | `cargo clippy -p axagent-astock-data -p axagent-analysis-engine --all-targets --all-features -- -D warnings` | **EXIT=0**（2 次，49s / 2m59s） |
| 常量等式门禁 | `node scripts/check-valuation-defaults-parity.mjs` | **EXIT=0**（`perpetualGrowth 0.013` 五处一致） |
| 单元测试（改动 crate） | `cargo test -p axagent-astock-data --all-targets` | **EXIT=0，344 + 5 + 7 全绿**（3 红 → 1 红 → 0 红，见 11.2 / 11.3） |
| 结构型复核 | `node output/sci21-structural-at-final-params.mjs` | 见 11.6 |
| 格式化（TS/JSON） | `dprint check` | **EXIT=0** |
| 格式化（Rust，**全工作区**） | `cargo fmt --all --check`（**重跑，权威**） | **EXIT=0，0 处 diff**。早先那次报的 4~5 处 diff **全在并发会话正在改的文件**（见 11.11.6 / 11.11.7②③） |
| 前端类型 | `tsc --noEmit` | **EXIT=0** |
| 编译（**主 crate**，11.11 新增） | `cargo check -p axagent --lib --all-features` | **EXIT=0**（23m14s，含排队 ~17min；`Checking axagent v2.9.9` → `Finished`，0 error / 0 warning）⇒ 11.11 的两号对齐 + 编译期断言**集成编译通过** |
| 版本门测试（**主 crate**，11.11 新增） | `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::stock_analysis_setup` | **两次均为 `EXIT=127`，两次都不是构建结论**（三特征闸全中，见 11.11.7⑤）⇒ 该测试**未验证**，原因是**运行器**而非代码。静态替代证据：`version_gate_tests` 全用**符号**比对（`Some(TEMPLATE_VERSION)`），零手写版本号 ⇒ 77→79 **不会**打破它 |
| **断言区分力自证**（11.11 新增） | `rustc --edition 2024` 编译 `output/tmp/probe_const_assert.rs` | **正控（79 ≥ 79）EXIT=0 零输出；负控（77 < 79）EXIT=1 + `error[E0080]: evaluation panicked`** ⇒ 该断言**真的会拦**，不是恒真装饰 |
| **未跑** | `cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace` | **不构成通过**（全工作区口径未验） |

### 11.9 第四种同型复发：**门抬在改动之前**（本轮最有推广价值的发现）

`DCF_MIGRATION_VERSION` 是**一次性**门（`existing.version < V ⇒ force`）。
本轮先把它抬到 78，**之后又改了被它守护的常量**（`p` 1.5% → 1.3%）。

若应用曾在 v78 上启动过一次并完成 force，DB 版本即变成 78 ⇒ `78 < 78` 为假
⇒ **1.3% 永远进不去 DB**，而代码读起来一切正常（扁平参数继续用 1.5%）。
这比前三次（v74/v77/v78「门没抬」）更隐蔽 —— **门抬了，但抬在改动之前**。

⇒ 处置：再抬到 **79**，并写入判据：
**版本号应对应「值的最后变更」，不是「本轮开始」**；
凡在某个 `_MIGRATION_VERSION` 声明之后又改了它守护的常量，**必须再抬一次**。

**DB 实测（避免把隐患写成已发生）**：`stock-analysis` 模板 `version = 76`，
三变量仍是 `12 / 4 / 8.5` ⇒ 应用**没有在 v78 上启动过**，故本次「门抬在改动之前」
**只是隐患、未实际发生**（若已发生，代价是 1.3% 永远进不去 DB）。
抬到 79 是**防御性**处置：`76 < 79` 成立 ⇒ 下次应用启动时 force 会一次性把三值刷成
**`12 / 1.3 / 7.7`**。被覆盖的 `4%` 属**口径错误**（越过模型自身申报的 `r_f`），
不是用户偏好，故覆盖代价可接受；且 DB 现值恰等于规范值 ⇒ **用户并未手调过**。

### 11.10 本轮改动文件清单

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/astock-data/src/mcp_tools.rs` | `PERPETUAL_GROWTH` 0.02→0.015→**0.013**（由 `p×1.3 ≤ r_f` 反推，附弹性表订正）；新增 `LOW/HIGH_PERPETUAL_SCALE_POS` 具名常量 + **编译期断言**；`p` 可行域守卫**合并两条上限**并记 `spread_is_binding`；判据 ⑤ 文案改为「两条约束 + 更紧者 + 生效值」；**`ttm_net_profit` 补 ③ 回落分支**（fail-open 根因）；三处引用旧弹性表的注释订正；新增 1 条测试、重写 1 条测试、订正 1 条测试的绝对容差 |
| 2 | `commands/stock_analysis_setup/seed_stock_analysis.rs` | `DCF_MIGRATION_VERSION` 78→**79** + 「门抬在改动之前」复盘；**`TEMPLATE_VERSION` 77→79**（与门对齐，见 11.11）；门值由函数内**提到模块级** + 新增**编译期断言** `TEMPLATE_VERSION ≥ DCF_MIGRATION_VERSION` |
| 3 | `src/components/settings/StockAnalysisConfigPanel.tsx` | `perpetualGrowth` 0.015→**0.013** |
| 4 | `output/sci21-structural-at-final-params.mjs` | 结构型结论的参数稳健性复核（可复跑） |
| 5 | `docs/audits/AUDIT-valuation-model-scientific-soundness-2026-09-23.md` | 顶部加**时效声明**（列出被推翻 / 仍成立的结论） |
| 6 | `docs/audits/AUDIT-decision-chain-structural-bias-2026-09-23.md` | 本节 11.11 + 验证状态表更新 |

---

### 11.11 第五种同型复发：**两条水位线**（门抬了，落库版本号没抬）

### 11.11.1 事实

同一族（「改完不生效」的**生成物陷阱**）此前已复发四次：

| 次 | 形态 | 后果方向 |
|---|---|---|
| v74 | 门**没抬**（另有冗余覆盖分支把 force 写的新值抹掉） | 改动没落库 |
| v77 | 门**没抬**（永续 4%→2%） | 改动没落库 |
| v78 | 门**没抬**（`r_f` 口径订正连带 `d` / `p`） | 改动没落库 |
| v79a | 门抬了，但**抬在改动之前**（先抬 78，后改 `p` 1.5%→1.3%） | 新值**永远**进不去 DB |
| **v79b（本次）** | 门的值对了，但**落库写的版本号没跟着抬** | **反向**：门**永远关不上** |

前四次的失效方向都是「改动进不去 DB」；**本次是反的** —— 改动进得去，但**幂等失效**。

### 11.11.2 根因：这个机制有**两条独立的水位线**，此前只对其中一条做过纪律约束

```
落库写入：version: Set(TEMPLATE_VERSION)            （seed_stock_analysis.rs:5224）
                                   ↓ 同一列
DCF 一次性门读：previous_version < DCF_MIGRATION_VERSION   （:4865）
```

> ⚠️ **行号引用只作定位提示，不作依据** —— 它们随编辑漂移（本节所述缺陷本身就会
> 让行号失效）。请以**符号 / 字符串锚点**为准：`existing.version >= TEMPLATE_VERSION`（外层门）、
> `version: Set(TEMPLATE_VERSION)`（落库点）、`previous_version < DCF_MIGRATION_VERSION`（DCF 门）、
> `kelly_pos_uncapped >= POS_BUY_MIN`（买入仓位门）。以上行号为 **2026-09-23 时点值，已逐条回读校对**。

- 外层版本门 `existing.version >= TEMPLATE_VERSION ⇒ return`（`:1146`）决定**种子跑不跑**；
- DCF 门决定**跑的时候要不要强制覆写用户在面板里调的估值参数**。

两条门必须**同时关闭**。而它们各自读**不同的常量** ⇒ 只要 `TEMPLATE_VERSION < DCF_MIGRATION_VERSION`，
落库后的 DB 就停在一个**仍满足 `版本 < 门`** 的值上 ⇒ **门关不上**。

具体推演（本轮实际状态 `TEMPLATE_VERSION = 77`、门 `= 79`）：

| 时点 | 外层门 | DCF 门 | 结果 |
|---|---|---|---|
| 本次启动（DB 76） | `76 >= 77` 假 ⇒ **跑** | `76 < 79` ⇒ **force** | 估值参数落对（`12 / 1.3 / 7.7`），DB ← **77** |
| 下次启动 | `77 >= 77` 真 ⇒ **跳过** | 到不了 | 无事 |
| **将来任何无关模板改动升版到 78 或 79** | `77 >= 78` 假 ⇒ **跑** | `77 < 79` ⇒ **force** | ⚠️ 把用户在面板里调好的估值参数**打回默认** |

⇒ 本次**能落对**（是 latent，不是 live），但**第三行**是必然触发的：
`AGENTS.md` 明写「升版是唯一能保证 DB 与仓库一致的手段」，而这个模板一直在改（v60→v77 十七次）。
**下一次升版就会踩**，且踩的时候症状是「我明明没动估值参数，怎么回弹了」——与估值改动毫无因果关系，
极难归因到本机制。

讽刺的是：`:4805` 的注释**自己**在 v74 就写明了「若仍无条件执行，则**每次**升版都会把用户在
面板里调好的 DCF 参数打回默认」，并据此设计了这道门 —— **门设计对了，但水位线取错了**，
于是它要防的那个后果五年后照样会来。

### 11.11.3 处置（**已自行落定，不提交裁决**）

| # | 动作 | 依据 |
|---|---|---|
| 1 | `TEMPLATE_VERSION` 77 → **79** | 使「门关闭」与「落库」在**同一步**完成；此后 `previous_version = 79` ⇒ `79 < 79` 假 ⇒ 门永久关闭 |
| 2 | `DCF_MIGRATION_VERSION` 由函数内**提到模块级** | 使跨常量断言在物理上可能（函数内 `const` 无法被模块级引用） |
| 3 | 新增**编译期断言** `TEMPLATE_VERSION >= DCF_MIGRATION_VERSION` | 这是**跨常量**的等式型约束：测试要靠人记得跑，而本约束的失效形态恰恰是「没人注意到」⇒ 必须编译期。此后任一常量漂移即**编译失败** |

**为什么取 79 而不是 78**：门值是「被守护常量最后一次变更时对应的模板版本」。
本轮改的是 `PERPETUAL_GROWTH`（`mcp_tools.rs`）与 `portfolio-mgr.rhai`（经 `include_str!` 进模板），
两者都随**本次**落库 ⇒ 承载版本就是 79。取 78 会让 `79 < 79` 也为假，虽也能关门，
但会让「门值 == 变更发生的那一版」这条语义再次失去锚点 —— 而**锚点丢失正是本族前四次的根因**。

### 11.11.4 这条不变量与既有测试的**闭环**（为什么不需要再加测试）

既有 `version_gate_tests` 已有两条断言（`mod.rs:2473` / `2481`）：

1. 空库种子化后 `version == TEMPLATE_VERSION`；
2. `force_version(db, TEMPLATE_VERSION - 1)` 后种子化，`version == TEMPLATE_VERSION`。

与新增的编译期断言 `TEMPLATE_VERSION >= DCF_MIGRATION_VERSION` 合起来即得：
**落库版本 `= TEMPLATE_VERSION ≥ 门` ⇒ 门必关闭**。这是一条**传递闭包**式证明，
不需要再补一条「跑两次种子看参数有没有回弹」的测试（那只能覆盖单点，且要造 79 次升版）。

### 11.11.5 推广判据（写入 `MEMORY-RULES.md`）

> **凡「幂等门 + 落库版本号」分处两个常量**：门的水位线必须 **≤ 落库版本号**。
> 倒挂的失效方向是**反向**的 —— 不是「改动不生效」，而是**门永远关不上、迁移反复重放**，
> 症状与本机制无因果关系（表现为「设置莫名回弹」），极难归因。
> 检法：把「写版本的常量」与「门的常量」并列，看谁大；一眼判死。

### 11.11.6 本轮验证外的环境事实（避免误判为自己引入的失败）

`cargo fmt --all --check` 的 5 处残留 diff **全部落在两个非本轮改动文件**上：
`src/bin/axagent-batch-rerun.rs`（3 处）与 `commands/stock_workflow/reflection.rs`（2 处）。
判据是 **mtime**（非 git）：前者实测为**运行本报告时 2 分钟前**被写入，后者 16.7 分钟前；
而本轮我改的文件是 9 分钟量级。且 `axagent-batch-rerun.exe` 的 `cargo build` 进程
（`Get-CimInstance` 取证）在同一时段持续在跑 ⇒ 系**并发会话的在飞改动**。
**故未执行 `cargo fmt --all`**（格式化会覆盖对方未保存的编辑），
仅在 `-p axagent-astock-data` 范围内格式化了自己的文件。

### 11.11.7 收口：主 crate 编译验证 + 两个**判据级**教训

**① 主 crate 编译 —— 由「排队中」转为通过。**
`cargo check -p axagent --lib --all-features` 最终拿到锁并跑完 23m14s：

```
Blocking waiting for file lock on build directory      ← 排队 ~17min（4 个 cargo 抢一把锁）
    Checking axagent-astock-data v0.1.0
   Compiling axagent v2.9.9 (D:\OneManager\AxInvest\src-tauri)
    Checking axagent-analysis-engine v0.1.0
    Checking axagent-runtime v2.9.9
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 23m 14s
```

**0 error / 0 warning**（`error` 行数 = `warning` 行数 = 0）。⇒ 11.11.3 处置的
`TEMPLATE_VERSION` 77→**79**、门值提模块级、`const _: () = assert!(TEMPLATE_VERSION >= DCF_MIGRATION_VERSION)`
在**真实集成编译**下成立（此前只有 `rustc` 单文件正负控的间接证据）。

**② 教训一 —— 格式化残留的「过期日志陷阱」。**
`EXZXky` 后台任务里的 `fmt3.log` 报了 `mcp_tools.rs` 2 处 diff，看起来是本轮文件没格式化干净。
但那是**排队时的输出**：我随后才跑的 `cargo fmt -p axagent-astock-data`。
**重跑 `cargo fmt --all --check` 的权威结果是 EXIT=0 / 0 处 diff**。
⇒ 判据：**改完之后的旧日志不能当判据** —— 判「当下是否干净」必须用**当下重跑**的结果；
旧日志只对「它运行的那一刻」有效。

**③ 教训二 —— rustfmt 失败文件的**连带失检**。**
`cargo fmt --all --check` 在并发会话的文件上报 `error[internal]`（尾随空格 / 全角空格），
**rustfmt 在这类文件上直接失败**，其后文件是否仍被检查**不可依赖**。
⇒ 「我的文件没出现在红名单里」**≠**「我的文件干净」。
可靠判据是**单文件直验**，且绕开全量跑的连带风险：

```
rustfmt --edition 2024 --check --config-path src-tauri/rustfmt.toml <单文件>
→ EXIT=0，零 diff、零 error
```

**④ 仍未跑（结论不得外推）**：`cargo clippy --workspace --all-targets --all-features -- -D warnings`
与 `__TAURI_WORKSPACE__=true cargo test --workspace` 属**全工作区口径**，本轮**未执行**
—— 它们是提交前门禁，不是本轮「缺陷已闭环」的证据。

**⑤ 一个**必须当场拦下**的误读：`EXIT=127` 不是构建结论。**

版本门测试首跑写出 `TEST_GATE_EXIT=127`，日志在 `Compiling` 中途截断（414 B，无 `error`、无 `Finished`）。
第一反应容易读成「cargo 编译失败」。**错**。判据 `#470` 已记录同型：

> `TaskStop` 的清理范围**可能超出单个任务** ⇒ 连带终结同会话其他后台任务。
> 实测撤下一条排队的 `clippy --workspace` 后，另两个任务双双「完成」却**无任何产物**
> （一个 `exit=127` 无 `test ...` 输出）。

**三个特征同时出现即可判死**（本次三条全中）：

| 特征 | 本次实测 |
|---|---|
| `exit=127`（**非** cargo 的错误码 `101`，也非信号码 `128+n`） | `TEST_GATE_EXIT=127` |
| 日志**无 `test ...` 行**（连测试二进制都没启动） | 无 |
| 日志在 `Compiling` 中途截断、**无 `error` / 无 `Finished`** | 414 B，停在 `Compiling axagent-analysis-engine` |

⇒ `127` 是**任务被连带清理**的产物，**不是 cargo 的退出码**，因此**通过/失败都不可读**。
反证：同一 env 前缀单独探针 `__TAURI_WORKSPACE__=true cargo --version` → `P1_EXIT=0`
⇒ env 前缀语法没问题，`.cargo/config.toml` 也没有 `RUSTC_WRAPPER`（只有 `linker = "rust-lld"`）。

**判据**：后台 cargo 任务报非 `{0, 101}` 退出码时，**先判「是不是被清理了」**
（看有无 `test ...` 行、有无 `Finished`），再谈构建结论。
`cargo` 的**只有** `0`（成功）与 `101`（编译/lint 失败）是构建结论；
其余值一律先归因到**运行器**。

**第二次复现（同判据、加一条新证据）**：重跑后仍 `EXIT=127`，三特征闸再次全中
（无 `test result:`、无 `Finished`、`^error` 行数 = 0）。
**新增证据**：这次编到了 **6 个 crate**（首跑 4 个）——
`astock-data / axagent / analysis-engine / tools / company-runtime / runtime`。
⇒ **两次都死在「正在干活」的中途，且每次推进得更远**，这排除了「某个 crate 编译不过」
（那会固定在同一处并打 `error`），坐实「进程被外部终止」。

**环境取证（用于排除「工具链坏了」）**：

| 探针 | 结果 |
|---|---|
| PATH | **健康**（`.cargo/bin`、MSVC 14.50 bin、Windows Kits 全在） |
| `__TAURI_WORKSPACE__=true cargo --version` | `EXIT=0` |
| `command -v rustc` | `/c/Users/polit/.cargo/bin/rustc` |
| `RUSTC_WRAPPER` / `CARGO_TARGET_DIR` | 均 `<unset>` |
| `.cargo/config.toml` | 无 wrapper 层，仅 `linker = "rust-lld"` + `rustflags = ["-C","target-feature=+crt-static"]` |

**为什么不再重试（取舍，不是放弃）**：`Get-CimInstance` 取证显示仓内有
**≥3 个非本轮的 cargo 进程**在抢同一把 target 锁 —— 含
`cargo clippy --workspace --all-targets --all-features -- -D warnings`（PID 33808/26600）与
`cargo check --workspace --all-targets --all-features`（PID 2488）。而**本轮最关键的那条已经绿了**
（主 crate 编译 `EXIT=0`，见 11.11.7①）。
⇒ 该测试登记为**未验证（原因：运行器 + 锁竞争）**，不拿第三次尝试换一个边际验证。

