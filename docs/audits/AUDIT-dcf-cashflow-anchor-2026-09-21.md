# AUDIT — 「巴菲特估值」为何修了多次仍然错误（2026-09-21）

> 触发样本：`300308 中际旭创`，运行 `ee770189-adf6-4b0e-a593-4cae93e60cc7`
> （2026-09-21 05:37:11，`template_version=65`，现价 926.43）
> 用户现场："现价926.43的中际旭创，巴菲特估值80.63-156.77元" + 两名分析师被判低置信。

---

## 一、一句话结论

**`direct_fcf` 在生产上恒为 `None` —— 因为 vendor 从来没有取过现金流量表。**
DCF 因此**永远**用「近 5 年年报正净利均值 × 0.90」当现金流锚；而历次修复
（V74→P0-I→2026-09-14 的 `applicable` 判据）**全部作用在这个变量的下游**，
没有一次去核对它的上游取值链。**唯一能拦住该形态的护栏（适用性判据 ②）
因输入缺失而从未触发过一次。**

---

## 二、为什么「修了几次还是这样」—— 三层原因，逐条取证

### 层 1：它被正式诊断过，但**只裁决了其中一半**

`docs/audits/AUDIT-601166-valuation-decision-2026-09-14.md` §2 缺陷 **D2（P0）
「DCF 对银行股结构性失效」**已把根因写清：

> **① 把「净利润」当「自由现金流」** …… fallback 取 `近 5 年报正净利均值 811.7 亿 × 0.90`
> **③ 隐含 PE 与市场定价差 2.8 倍**
> **状态：未修，待产品裁决（见 §5）。**

§5 提出四个议题，其中：

| 议题 | 结局 |
|---|---|
| **Q1** 金融业（高杠杆）是否应禁用 DCF / 格雷厄姆？（建议：`debtRatioPct >= 80` 时标 `not_applicable`，`value_signal`/`margin_of_safety` 同步标注「不适用」） | **未被裁决** ⇒ D2 悬空至今 |
| **Q2** `perpetual_growth` 与 `growth` 方向一致性约束 | **已裁决并落地** —— 代码注释可证：「永续增长率符号一致性约束（2026-09-14，**用户裁决「直接强制」**）」(`mcp_tools.rs`) |

⇒ **Q1 从未被裁决，所以「估值不可信要给用户看见」这件事一次都没做。**
面板上至今照原样输出 `value_signal` 与 `margin_of_safety`（同文档 §2 末已指出：
「`is_fallback_anchor = true` 只让 f5 的 σ 乘 0.5，**但 `value_signal: '低估'`
与 `margin_of_safety: '充足 58.9%'` 照原样输出到面板**，且没有任何『该估值不可信』的标注」）。

### 层 2：历次修复**全部在 `direct_fcf` 的下游**，而它恒为 `None`

| 批次 | 做了什么 | 为什么没解决 |
|---|---|---|
| **V74**（2026-09-10） | 原实现 FCF≤0/股本缺失时返回 `(0,0,0)`，「无法估值」被编码成「估值为 0」⇒ 改为 `Option` + 归一化 fallback | 把「FCF 缺失」**当成「FCF≤0」**处理，于是**发明**了历史均值锚。问题不在「0 是垃圾值」，而在「0 从哪来」 |
| **P0-I**（2026-09-12） | 新增 `is_fallback_anchor`，f5 的 σ × 0.5 | 只压内部权重，**用户侧无任何标注**；且该锚本身没被质疑 |
| **P0-F**（2026-09-12） | `growth` 两条分支统一 `clamp` | 治档位乱序，与该锚无关 |
| **P1-A**（2026-09-11） | `annualized_eps` / `annualized_roe` 做 TTM 还原 | **同族缺陷只修了 EPS/ROE，没修 FCF** |
| **量程守卫**（2026-09-11/12） | 挡掉 `1e-10 × 现价` 级退化值与 `+6.18e8%` 级越界值 | 挡的是**量纲错**，不是**锚选错** |
| **`applicable` 三判据**（2026-09-14） | ①负债 > 80% ②FCF/净利背离 ③终值独裁 | **判据 ② 需要「当期真实 FCF」才能算 —— 而它恒为 `None`** ⇒ 永远不命中。601166 当时靠判据 ①（负债 91.6%）侥幸命中，掩盖了 ② 已死 |

→ **六次修复，零次检查输入。** 601166 那次之所以看起来成功，是因为**那个特定标的
由判据 ① 兜住了**；换成一家负债 36.1% 的制造业公司，三条判据全哑。

### 层 3（最关键）：一句**编造的诊断**在替每次修复背书

`basis` / `note` 恒输出：

> `"当期FCF≤0，改用近5年报正净利均值×0.90归一化锚定（周期底部）"`

而对 300308 这句是**双重错误**：
1. **它不是「周期底部」** —— 营收 +182.5%、ROE 62.6%、毛利 46.3%、净利创历史新高；
2. **它不是「FCF≤0」** —— 是**我们没取到 FCF**。真实 TTM 自由现金流为 **+28.69 亿（正）**。

这句文案把「我方采集缺陷」写成了「标的现金流属性」，于是：
* 每一次下游修复（造代理锚 / 加衰减 / 加判据）都**看起来已经对症**；
* 该句被 `value-investor` **原样引用进 `risk_flags`**（实测样本 `ee770189`：
  `"当期FCF≤0，DCF基于归一化锚定，绝对估值锚偏低"`）⇒ 假诊断直接进结论；
* 读到它的人（含前几轮的我）会去修**下游**，而不是去问「FCF 为什么是 0/负的」。

> **这就是「修了多次还是这样」的机制**：不是每次修错了，而是每次都在修
> **症状的下游**，而**上游有一句自信的假诊断**在指错方向。

---

## 三、证据链

### 3.1 DB 跨样本（49 条 `stock_analyses`）

| 口径 | 结果 |
|---|---|
| 含 DCF `note` 的样本 | 15 条，**15/15 都写「当期FCF≤0」** |
| 含 `is_fallback_anchor` 字段的样本（P0-I 之后） | 8 条，**8/8 = `true`** |

含 601166（兴业银行）、600031（三一重工）、603353、002353——**业务形态互不相同，
却 100% 走同一 fallback**。全库**没有一例**用过当期真实 FCF。

### 3.2 vendor 层：FCF 三列从未被填过

| vendor | `operating_cash_flow` / `capital_expenditure` |
|---|---|
| `eastmoney.rs:515-517`（主 vendor） | **写死 `None`** |
| `akshare` / `browser_eastmoney` / `baidu_stock` / `neodata` / `sina` | **写死 `None`** |
| `xueqiu.rs:218-219` | 尝试映射 `operate_cash_flow` / `capital_expenditure` |

⇒ `direct_fcf = free_cash_flow.or(ocf − capex)` **恒为 `None`**。

### 3.3 上游其实**一直有**这份数据（live 取证，2026-09-21）

`ZYZBAjaxNew`（代码在用的主要指标接口）实测 **141 个字段里没有任何现金流科目**：

```
SECURITY_CODE | REPORT_DATE | EPSJB | BPS | MGJYXJJE | TOTALOPERATEREVE | PARENTNETPROFIT
| … | CASH_RATIO | FCFF_FORWARD | FCFF_BACK | SS_OI | … （无 NETCASH_OPERATE / 无 capex）
```

同一站点的 **`xjllbAjaxNew`（现金流量表）**则直接给出：

```text
300308  2025-12-31   NETCASH_OPERATE = 10,896,126,160.03   CONSTRUCT_LONG_ASSET = 2,759,994,695.91
300308  2026-06-30   NETCASH_OPERATE =  1,799,674,832.02   CONSTRUCT_LONG_ASSET = 4,802,161,195.37
300308  2026-03-31   NETCASH_OPERATE =  3,367,110,538.13   CONSTRUCT_LONG_ASSET = 1,929,000,000.00
```

⚠️ `companyType` 必须随公司类型变化，**传错静默返回 `data: []`**：

| 公司类型 | companyType | 反例 |
|---|---|---|
| 通用 | 4 | `SZ300308` ct=4 → 3 行；ct=3 → **0 行** |
| 银行 | 3 | `SH601166` ct=3 → 3 行；ct=4 → **0 行** |
| 证券 / 保险 | 1 / 2 | `SH600030` ct=1 → 1 行；`SH601318` ct=2 → 1 行 |

类型可直接从同一接口的 `ORG_TYPE` 字段读（"通用"/"银行"/"证券"/"保险"），**不需要额外探测请求**。

### 3.4 锚定值精确复现（证明旧值确实是 5 年年报均值）

```text
2021 8.77 亿 / 2022 12.24 亿 / 2023 21.74 亿 / 2024 51.71 亿 / 2025 107.97 亿
均值 = 40.4865 亿 × 0.90 = 36.4378 亿
落库 fcf_anchor = 36.43782482 亿   ← 逐位吻合
```

而 **TTM 归母净利 = 204.53 亿**（2025FY 107.97 + 2026H1 136.51 − 2025H1 39.95）
⇒ **锚点比当前盈利能力低 5.6 倍**。

### 3.5 `value_signal` 与 `margin_of_safety` 在同一 payload 里互相打脸

| 字段 | 值 |
|---|---|
| `dcf.mid` / `upsidePct` | 139.22 / **−85.0%** |
| `margin_of_safety` | `"无（高估风险）"` / **−565.4%** |
| **`value_signal`** | **`"合理偏低"`** ← |
| `summary` | `"综合判断:合理偏低"` |

评分函数取证（`mcp_tools.rs`，修复前）：

```rust
let mut score = 0u32;                       // ← u32，无法为负
match mos_pct {
    Some(p) if p > 20.0 => score += 30,
    Some(p) if p > 10.0 => score += 20,
    Some(p) if p > 0.0  => score += 10,
    _ => {},                                // ← 负安全边际加 0 分（不是扣分）
}
score += f_score.min(9) * 5;                // F-Score 6 ⇒ +30
score += moat_score.min(100) / 5;           // 护城河 75 ⇒ +15
if oe_yield > 5.0 { … } else if oe_yield > 3.0 { … }   // 1.3% ⇒ +0
// 合计 = 45 ⇒ 45.. => "合理偏低"
```

⇒ **`45` 恰好就是「合理偏低」档的阈值，而它完全由 F-Score + 护城河贡献。
只要 F-Score ≥ 6 且护城河 ≥ 75，无论价格多高都稳落「合理偏低」** ——
估值维度在评分里**只加不减**。

`value-investor` 在正文里**点出了这个矛盾**（"算法 value_signal 为「合理偏低」系基于
成长股前瞻EPS逻辑，但绝对估值锚与现价偏离超80%"）——说明 LLM 看见了，但 prompt
要求它「**直接引用** `t-valuation.result.value_signal`」，于是矛盾原样进结论。

### 3.6 prompt 侧的逃生条件漏了两个已存在的标记

`value-investor.md` 与 `seed_stock_analysis.rs` 内嵌 prompt（**两份副本**）里，
「估值不可用」只认 `available=false` / `upsidePct=null`。
`assumptions.applicable`（2026-09-14 新增）与 `assumptions.is_fallback_anchor`（P0-I 新增）
**都不在逃生条件里** ⇒ 生产者打了标、消费者一个都不读 = 断链。

---

## 四、影响量化（样本 `ee770189`）

修复前 f5 估值因子（`portfolio-mgr.rhai`）：

```text
dcf_sig    = pm_saturate(-85.0, 40) = -0.680
graham_sig = pm_saturate(-81.9, 40) = -0.672
fused      = (0.7×(−0.680) + 0.3×(−0.672)) / 1.0 = −0.6776
σ          = −0.6776 × fscore(1.0) × moat(1.0) × fallback_decay(0.5) = −0.339
贡献       = σ × w = −0.339 × 0.165 = −0.0559
```

| 量 | 值 |
|---|---|
| Σ(σ·w) 全体因子 | +0.0736 |
| **f5 单独贡献** | **−0.0559**（占总额 76%，第二大负项，仅次于 f9 资金流 −0.0611） |
| posterior | 0.528 |
| `action_increase_threshold` | **0.53** |
| 后验映射系数 k（反推） | 0.3804 |
| **把 f5 中性化（σ=0）后** | Σ=0.1295 ⇒ posterior = **0.5493 > 0.53** ⇒ **动作会翻到「增持」** |
| f5 衰减减半 | Σ=0.1016 ⇒ posterior = 0.5386 ⇒ 同样翻档 |

⇒ **该退化锚单独改变了动作档位**（持有 ⇄ 增持），不是"仅影响置信度"。

---

## 五、本轮修复（已实施）

### F1 补取现金流量表 —— 让取值链 ① 分支与判据 ② 重新可执行

| 文件 | 改动 |
|---|---|
| `crates/astock-data/src/vendors/eastmoney.rs` | `get_financials` 新增 ④ `xjllbAjaxNew` 请求；按 `ORG_TYPE` 选 `companyType`；`NETCASH_OPERATE → operating_cash_flow`、`CONSTRUCT_LONG_ASSET → capital_expenditure`；`filled == 0` 升 warn（`data: []` 是静默失败形态） |
| `crates/astock-data/src/mcp_tools.rs` | 新增 `ttm_fcf()`：**镜像 `annualized_eps` 的 TTM 还原惯例**（年报直取 / 中报「上年年报 + 本期累计 − 上年同期累计」/ vendor 直供 `free_cash_flow` 兜底 / 全缺 ⇒ `None`）；`compute_dcf` 的 `direct_fcf` 与 `compute_owner_earnings` 改走它 |

**为什么不把年度化放在 vendor**：同一份 `FinancialReport` 内 `eps`/`roe` 是**年内累计口径**，
`ttm_fcf` 放在消费端才能与 `annualized_eps` 同构；vendor 只做忠实映射。

**实测效果（300308，TTM 口径）**

```text
2025FY  OCF 108.96 − capex 27.60 = +81.36 亿
2026H1  OCF  18.00 − capex 48.02 = −30.02 亿   （半年累计）
2025H1  OCF  32.18 − capex  9.54 = +22.65 亿
TTM   = 81.36 + (−30.02) − 22.65 = +28.69 亿        ← 为正
FCF/净利 = 28.69 / 204.53 = 0.1403 < 0.3            ← 判据 ②「量级脱钩」命中
```

⚠️ **不还原**会直接用 2026H1 的 −30.02 亿 ⇒ 把一家 TTM FCF 为正的公司判成
「当期 FCF ≤ 0」。这正是 `ttm_fcf` 存在的唯一理由（有反向对照测试）。

### F2 `basis` 三态化 —— 停止编造「周期底部」

```rust
const FCF_FALLBACK_BASIS: &str = "当期FCF≤0，改用近5年报正净利均值×0.90归一化锚定";
const FCF_MISSING_BASIS: &str =
    "现金流量表数据缺失（该数据源未提供 OCF/资本开支），改用近5年报正净利均值×0.90归一化代理锚定";
```

两态**处置相同**（锚都是历史代理），但**诊断必须说真话**：
「我方的采集缺陷」与「标的现金流为负」是两回事。

`is_fallback_anchor` 的派生同步从「等值于某一条文案」改为「**不是**当期真实 FCF」——
否则拆文案后缺失态会被**静默**标成 `false`，使 f5 衰减门在最该衰减的路径上失效。

### F3 `value_signal` 补负安全边际扣分

抽为可测函数 `value_signal_of(mos_pct, f_score, moat_score, oe_yield)`，
`score` 由 `u32` 改 `i32`，负安全边际按 `margin_of_safety.level` 的既有换档边界
（−20% / −50%）扣分。**正向档位与阈值一字未动**（不动既有正向样本）。

300308 实测：`−40 + 30 + 15 + 0 = 5` ⇒ **「高估」**（修复前为「合理偏低」）。

### F4 补上游 prompt 的逃生条件（**需升模板版本**）

`value-investor.md` + `seed_stock_analysis.rs` 内嵌 prompt（两份副本同改，防回流）：

* 「估值不可用」新增第 ③ 条 `assumptions.applicable == false`
  （三档数值仍输出，但**不是可靠估值证据**，`intrinsic_value_range` / `margin_of_safety`
  填 `null`，理想买入价写「无算法估值锚」）；
* 新增**锚定口径披露**要求：引用 DCF 区间**必须**一并引用 `assumptions.basis`；
* 新增**分歧处理**条款：算法锚与 LLM 判断分歧时照实报告，**不得**把算法没给的数字
  说成算法给的（直接针对本次「合理偏低 vs −85%」那种表述）。

`TEMPLATE_VERSION`: **64 → 66**。

> ⚠️ **版本门取证**：真库 `stock-analysis` 的 DB `version` = **65**
> （`updated_at = 1789940216663` ⇒ 2026-09-21 05:36:56），而文件常量当场是 **64**
> ⇒ **DB 已被别的路径写高**（`version_gate_tests` 注释把该状态列为已知情形）。
> 这正印证「常量 ≤ DB 现值 ⇒ 本批被静默挡住」——**直接写 65 会被 `>=` 门挡掉，必须跳 66**。
> 另已核实 v64 内容**确实在库**（`nodes` 含 `strip_verdict_blocks` /
> `placeholder_occurrences` / `soft_marker_suppression` / `dt_seat_blank`）。

### F5 测试

`crates/astock-data/src/mcp_tools.rs` 的 `valuation_tests` 新增 6 个：

| 测试 | 断言 |
|---|---|
| `ttm_fcf_uses_annual_report_directly` | 年报口径直接相减，不做还原/放大 |
| `ttm_fcf_restores_ttm_for_interim_reports` | TTM = +28.69 亿；**反向对照**：不还原为负 ⇒ 两者走不同分支 |
| `ttm_fcf_returns_none_when_any_segment_missing` | 缺任一段 ⇒ `None`（不得退化成单期值/0） |
| `ttm_fcf_falls_back_to_vendor_supplied_free_cash_flow` | vendor 直供 FCF 优先 |
| `dcf_basis_tells_three_states_apart` | 三态文案互不冒充；负值与缺数均标 `is_fallback_anchor`；**真实正 FCF 时判据 ② 不误命中** |
| `value_signal_can_express_overvaluation` | 300308 实测输入 ⇒「高估」；安全边际转正 ⇒ 档位跟着变（反向对照）；边界与单调性 |

**验证**：见 §七。另需注意既有测试
`dcf_normalizes_when_latest_loss_but_history_positive` 断言 note 含「归一化」——
「归一化」描述的是**锚定构造**（两态都成立），故两条文案均保留该词，
**只区分成因**；该测试的失败已按此口径修正文案而非改断言。

---

## 六、拍板项：采纳的推荐值与落地（**2026-09-21 已按推荐值修复**）

> 每节下面的候选列表保留原样（决策留痕）。本节先给**采纳的推荐值**与**落地位置**。

| 项 | 采纳的推荐值 | 为什么不选其它 | 落地 |
|---|---|---|---|
| **Q1** | **(c) 面板显式标注**，本轮不换锚口径 | (b) 换锚需新数据源（EV/EBITDA、前瞻 EPS）+「成长股」判据，且该判据必须按**数据形态**定义 ⇒ 独立批次；(c) 所需信号**已在产物里**（`valuationApplicability`），只是前端零消费 —— 属「接线」而非「设计」 | `agentOutput.ts::extractValuationApplicability`；store 4 个写入点；`ValueAssessmentPanel` 标注块；11 语言 ×6 key |
| **Q2** | **给 graham 腿加对称护栏：软衰减 ×0.5**（**不是**整腿退出） | 「整腿退出」会过度处置：`g` 顶死上界只让内在价值**系统性偏低**（看空夸大），数值**仍有信息量**，与 DCF 的 `is_fallback_anchor` **同族**（假设被压缩），不是 `applicable=false` 那种「数值没有经济含义」 | `compute_graham_value` 暴露 `growthClampedUpper`；模板注入 `valuation_graham_growth_clamped`；Rhai `graham_leg_decay`；`TEMPLATE_VERSION` 66 → **67** |
| **Q3(a)** | **只修取值域 bug**（消费端对齐 `宽阔/狭窄/无`）+ 等式门禁；**乘子方向语义单列登记** | 死分支是 bug（消费者恒拿不到）；但「乘子对**负信号**放大看空」是产品语义 —— 注释写的是「估值应享受溢价」，实现却是放大 \|信号\|，两者矛盾。按纪律**只登记不擅改** | `portfolio-mgr.rhai` 比对值 + `seed_consistency_tests::moat_level_vocabulary_matches_rhai_consumer` |
| **Q3(b)** | 并入 Q1 落地 | — | 同 Q1 |
| **Q3(c)** | **登记，本轮不做** | 属判据体系扩展（工具层「声称 vs 实际」对账），需独立设计；不是接线 | 见「§6.4 续办」 |

### Q1（沿用 2026-09-14 的 D2/Q1，已悬空 7 天）

DCF/Graham 两条腿对**高增长标的**结构性失效：即便喂进**真实** TTM FCF，
`10% 折现率 + 30% 增速上限 + 5 年预测期` 也**算不出 926 元**——
反解需要 FCF ≈ 242 亿（≈ 当期 OCF 的 2.2 倍）。
**这不是数据 bug，是模型口径选择。** 候选：

* (a) 维持现状：`applicable=false` 只让 reporter 填 `null`，数值仍算（本轮已实现）；
* (b) 给成长股换锚口径（EV/EBITDA、PEG、前瞻 EPS）—— 需定义"成长股"判据（建议按**数据形态**，不按行业标签）；
* (c) 面板显式标注「该估值锚为历史代理 / 模型前提不成立」（**Q1 原文的建议，至今未做**）。

### Q2（本轮新发现，与 F1 有交互，**请重点看**）

`portfolio-mgr.rhai` 的衰减门是 `dcf_leg_usable && is_fallback_anchor`。
F1 落地后本样本 `applicable` 变 `false` ⇒ `dcf_leg_usable = false` ⇒
**衰减门失效** ⇒ 只剩 graham 腿独裁且**不衰减**：

| 情形 | f5 σ | 相对现值 |
|---|---|---|
| 修复前（DCF+graham 融合 ×0.5 衰减） | **−0.339** | 基准 |
| 修复后（graham 独腿，无衰减） | **−0.672** | **翻倍** |

（动作档位不变：posterior 0.528 → ≈0.507，仍落「持有」0.48~0.53 区间；
但这暴露了设计假设——**默认"剩下的那条腿可信"**。对 300308，graham 腿
是 `EPS × 9.1`（g 被 clamp 在 30% 上界的极限 PE），同样不可信。）

候选：

* (a) 接受（衰减门只描述 DCF 腿，V77 注释即此意），把"两腿皆不可信"作为 Q1 的一部分处理；
* (b) 新增对称护栏：`applicable == false` 且另一腿增益被 `MAX_GROWTH` 顶死 ⇒ **f5 权重归零**
  （估值维度整体退出，而不是让剩余腿以小权重独裁）。

### Q3（小）

* `moat_mult` **是死分支**：Rhai 比对 `"宽护城河"`/`"wide"`，而 `t-valuation` 实际输出
  `moat.label = "宽阔"`（`compute_moat_score` 的取值域是「宽阔」/「狭窄」）
  ⇒ 恒为 `1.0`。修它会让负向拖累 ×1.15（**更看空**），且该乘子对**负信号**是
  「放大看空」——语义本身可疑，故未动。
* `valuationApplicability`（含 `dcfApplicable` / `reason`）**只写在 `portfolio-mgr` 的输出里，
  前端零消费** ⇒ 用户永远看不到「本标的 DCF 前提不成立」。
* 两名分析师被判低置信（催化剂 72 / 解禁观察 68，各 1 处失败标记）**归因正确、标记真实**
  （"数据获取失败（缺乏第三方研报支撑）"、"质押余额与平仓线未能获取"），
  属真缺口。但**同一份数据质量报告给全流程打 79.3/B「良好」**，而对
  上面这条 100% 命中的估值链断链**零告警** —— 数据质量判据度量的是
  「LLM 正文里有没有写失败词」，**不度量「工具声称有的数据是否真的取到了」**。

### 6.1 实施：Q2 的对称护栏（含量化复算）

F1 生效后本样本 `applicable=false` ⇒ DCF 腿整体剔除 ⇒ `fusion_wsum` 只剩 0.3
⇒ graham 腿**独占 100%**（相对原 30% 份额放大 3.33 倍），且 `f5_fallback_decay`
因联立条件 `dcf_leg_usable &&` 而失效 ⇒ σ 从 −0.339 **翻倍**到 −0.672。

用 §四 反推的后验映射系数 `k = 0.3804`（`posterior = 0.5 + k × Σ(σ·w)`）复算：

| 状态 | f5 σ | f5 贡献 | Σ(σ·w) | posterior | 档位 |
|---|---|---|---|---|---|
| 修复前（DCF+graham 融合 ×0.5） | −0.339 | −0.0559 | 0.0736 | 0.528 | 持有 |
| F1 生效（graham 独腿，无衰减） | −0.672 | −0.1109 | 0.0186 | **0.507** | 持有（更看空） |
| **F1 + 本护栏（×0.5）** | **−0.336** | **−0.0554** | **0.0741** | **0.528** | **持有（回到修复前量级）** |

⇒ 护栏的作用是**抵消 F1 引入的额外看空**，避免「修好数据源」反而把全部权重压到
另一条同样退化的腿上。

为什么是「软」而不是「硬」—— 三个信号的分工不可混用：

| 信号 | 语义 | 处置 |
|---|---|---|
| `dcf.assumptions.applicable == false` | 数值**没有经济含义**（企业自由现金流口径不成立） | **整腿剔除** |
| `dcf.assumptions.is_fallback_anchor == true` | 锚定是历史代理（系统性偏低） | 软衰减 ×0.5 |
| `graham.assumptions.growthClampedUpper == true` | 增长率假设被上限封顶（系统性偏低） | 软衰减 ×0.5（**本轮新增**） |

⚠️ 护栏只作用于 `graham_sig`，**不动** `f5_fallback_decay`（后者仍乘在 `fused_sig` 上）。
原因：把两者都改成「按腿衰减」会让 graham 腿不再被 DCF 的锚定缺陷连带惩罚 —— 那**是**
V77 注释声明的意图（「不要用 DCF 的缺陷惩罚与 FCF 无关的格雷厄姆信号」），但实现上
会改变历史样本的 f5 值，属**另一个语义变更**，需单独裁决，本轮不夹带。

### 6.2 实施：Q3(a) 的取值域 bug

`compute_moat_score` 的取值域是 `"宽阔" / "狭窄" / "无"`，而 Rhai 消费端比的是
`"宽护城河" / "wide"` ⇒ **乘子恒 1.0**。DB 实证（38 条样本）：
`moat.label` = 宽阔 2 / 狭窄 3 / 无 16 / 缺失 17 ⇒ 按旧比对值命中 **0/21**。

根因是**数据源认错**（不是笔误）：作者以为接的是 LLM 的 `verdict.moat_rating`
（那确实写「宽护城河」），实际 `input_mapping` 接的是算法字段 `moat.label`。

修法：消费端对齐算法取值域；新增 `moat_level_vocabulary_matches_rhai_consumer` 钉住
① 生产端档位 ② 消费端比对值（前缀相等）③ 禁止旧词汇回流 ④ 注入源不得改接 LLM 文本。

### 6.3 实施：Q1(c) 的前端接通

`valuationApplicability` 此前**只写进产物、前端零消费**。本轮：

1. `ValueAssessmentPanel` 新增标注块，**排在估值卡片之前**（否则用户读完数字才看到前提）；
2. `portfolio-mgr.rhai` 的该块新增 `anchorIsFallback` / `grahamGrowthClamped` ——
   原来只有 `dcfApplicable` / 两条腿是否参与 / `reason`；而**更常见**的误导不是
   「前提不成立」，而是「数值照常输出、但锚定是历史代理」，不标注用户就不知道分母是什么；
3. 提取走 `extractValuationApplicability`，兼容纯 JSON / `{content}` / `{result}` 三种包装；
   取不到返回 `null` 而**不伪造默认块**（「没这个字段」≠「字段说前提成立」）。

### 6.4 续办（登记于 2026-09-21 上半场；**下半场已逐项取证，见 §6.10**）

> §6.10 给出了每项的量化取证与推荐值；§6.7 的纯 bug 已修。以下保留原始登记文本以免丢失语境。

* **Q3(c) 数据质量面板的「声称 vs 实际」对账**：当前判据只度量「LLM 正文里有没有写失败词」，
  **不度量「工具声称有的数据是否真的取到了」** —— 所以那条 100% 命中的估值链断链
  （FCF 三列恒 `None`）在数据质量报告里**零告警**。属判据体系扩展，需独立设计。
* **moat 乘子的方向语义**：现在对负信号是「放大看空」，与注释「估值应享受溢价」矛盾。
  候选：作用于 \|σ\|（现状）/ 方向对称（看多 ×1.15、看空 ×0.87）/ 只在正信号段生效。
* **`f5_fallback_decay` 的连带惩罚**：见 §6.1 末尾 ⚠️。
* **Q1(b) 换锚口径**：EV/EBITDA / PEG / 前瞻 EPS；判据须按**数据形态**定义。

### 6.5 本轮新代码**自身**引入的 3 处缺陷（全部由门禁抓到，已修）

诚实登记：这三处不是历史遗留，是**我在 6.1–6.3 里写下的**。抓它们的是 `cargo clippy` 与
`cargo test` 的**编译期**报错（不是单测语义断言）—— 说明「编译类门禁」在这类低级错误上仍有不可替代的覆盖。

| # | 落点 | 形态 | 为什么必须修 | 修法 |
|---|---|---|---|---|
| ① | `seed_consistency_tests.rs:1313` | `include_str!("../../portfolio-mgr.rhai")` —— 路径基准是本文件所在目录，`../../` 解析到 **`src/portfolio-mgr.rhai`（不存在）** | 同目录的 `seed_stock_analysis.rs:3318` 本来用的是正确的 `../`；**同一份文件同一个引用，两处各写一个路径** ⇒ 正是本轮在治的病 | 改 `../portfolio-mgr.rhai`，并在注释里写明「基准目录 = `commands/stock_analysis_setup/`」 |
| ② | `seed_consistency_tests.rs:1319` | panic 文案里写了 `#{ }` 对象字面量示例 ⇒ Rust 把它当**格式化占位符**（`1 positional argument in format string, but no arguments were given`） | 一句「写给未来读者的解释」直接把测试二进制**编不出来** | 转义成 `#{{ }}` |
| ③ | `seed_stock_analysis.rs:146-150` | `///` 文档块与函数之间隔了一个**空行** ⇒ clippy `empty_line_after_doc_comments`（`-D warnings` 下构建失败） | 该块讲的是**常量**的安置理由，不是函数行为；直接并进函数文档是**语义错位** | 降级为 `//` 普通注释（并写明为何刻意不用 `///`） |

**可迁移的点**：②这类错的隐蔽性在于**修法不是改逻辑而是改字面量**，而它出现在的恰恰是
「解释错误原因」的那句话里 —— 越是想把话说清楚（写示例），越容易撞上转义。
**③**则是「文档注释的位置本身就是契约」：`///` + 空行 = 编译失败，`//` 则完全无害。

### 6.6 新门禁**自身**的 2 处假红（判据工具自证，已修）

§6.5 是「我写的产品代码」出错；本节是**我写来抓错的工具自己出错** —— 两次都表现为「门禁报红，但被测对象没错」。
这正是**判据工具自证**（R 组）：一条判据在能指控别人之前，必须先证明它不会恒红、也不会恒绿。

| # | 现象 | 真因（在**工具**不在被测对象） | 修法 |
|---|---|---|---|
| ④ | `moat_level_vocabulary_matches_rhai_consumer` 报 `left: ["string","宽阔","狭窄"]` ≠ `right: ["宽阔","狭窄","无"]` | 抽取器 `quoted_strings` 把**整段里的所有字面量**都抓了，包括守卫句 `type_of(valuation_moat) == "string"` 里的**类型名** `"string"` —— 它不是档位词，且左侧是函数调用 | 新增 `comparison_strings`：只保留 `<标识符> == "..."` 形态，`==` 左侧以 `)` / `]` 结尾者判为「函数返回值比较」而排除。**按语法形态判别**，不靠「维护一张类型名黑名单」 |
| ⑤ | `portfolio_mgr_rhai_compiles` 报 `Expression exceeds maximum complexity (line 94)` | 我的测试用 `rhai::Engine::new()` —— 其**实测**默认深度是 **`(32, 16)`**（★ 我最初凭印象写成 `(64, 32)`，见 §6.8），而生产两条路径都设了 **`(256, 256)`** ⇒ **测试比生产更严**，报出生产不会报的错 | 深度上限**从生产源码读出**（`expr_depths_in`），测试**不持有这个数字**；并新增断言「What-If 与重跑的配置必须相等」 |

**④ 的教训**：「在字符串字面量里找词」这个动作必须区分**词的角色**（档位值 / 类型名 / 提示语）。
用「黑名单排除已知类型名」的做法下轮还会漏 —— 要按**语法位置**（`==` 左侧是什么）判。

**⑤ 的教训（更要紧）**：**判据的引擎配置必须与生产逐项一致，且不能手抄**。
该值原本已在 `stock_analysis.rs` 与 `decision.rs` 各写一份，若测试再抄第三份，
三份漂移的方向决定了故障形态 —— 测试偏严 ⇒ **假红**（本次）；测试偏宽 ⇒ **放过真问题**（更危险）。
⇒ 凡是「测试要复刻生产某个参数」的场景，一律**从生产源码读取或从单点常量导入**，
并**顺手加一条「生产各处副本必须相等」的断言**（本次新增的就是这条）。

**修复结果**：`cargo test -p axagent --lib seed_consistency` ⇒ **16 passed / 0 failed**（`T2_EXIT=0`）；
`cargo clippy -p axagent --all-targets --all-features -- -D warnings` ⇒ **`CLIPPY_EXIT=0`**。
两条红都是门禁自身的问题，被测的生产代码**从头到尾没有被改**（本轮四个文件里，
`mcp_tools.rs` / `portfolio-mgr.rhai` / `seed_stock_analysis.rs` 的 sha256 在三轮之间**逐字节相同**，
只有测试文件在变）。

### 6.7 存量隐患（本轮**未改**，登记待裁决）：AST 缓存的键**不含解析期配置**

同一份 `portfolio-mgr.rhai` 被**四组不同配置**的引擎编译：

| 路径 | 引擎配置 | 出处 |
|---|---|---|
| 工作流 DAG 正常执行 | `set_max_expr_depths(1024, 1024)` + 安全限制 + wiring 注入 `pm_*` | `crates/rt-workflow/.../code_executor.rs:55` |
| What-If 回测 | `set_max_expr_depths(256, 256)`，**不注册** `pm_*` | `src/commands/stock_analysis.rs:199` |
| 决策重跑 | `set_max_expr_depths(256, 256)` + 安全限制 + `register_pm_functions` | `src/commands/stock_workflow/decision.rs:2693` |
| 本轮新增的语法门（测试） | 从 What-If 读出 | `seed_consistency_tests.rs` |

而 `crates/harness/src/rhai_ast_cache.rs:81` 的缓存键是 **`code_hash(code)`**，
`cache_key` **三处引用全部只在 `tracing!` 里**（第 88 / 98 / 105 行）⇒ **不参与分桶**。

**后果**：`max_expr_depth` 是**解析期**参数（它决定你能不能得到 AST），而缓存里存的是**已解析好的 AST**。
于是**先跑的那条路径**用自己的深度上限产出 AST 并写进全局缓存，**后跑者直接命中缓存返回 `Ok`** ——
它自己那份 `set_max_expr_depths` **根本没被使用**。具体表现为：

* 若 1024 那条先跑（DAG 正常执行是常路径）⇒ What-If 的 256 限制**形同虚设**；
* 若脚本某天出现 256–1024 之间的深度 ⇒ 故障**取决于启动后先点了哪个按钮**，重启后可能变化（flaky）。

`code_executor.rs:188` 的注释写着「缓存 AST 与 Engine 解耦，可被任意 Engine（含不同函数注册集）执行」——
该假设对**函数注册**成立（`fn` 解析与注册无关），对**解析期限制**不成立。

**候选修法**（未实施，需裁决）：① 缓存键并入解析期配置
（`get_or_compile_ast` 已经拿到 `&rhai::Engine`，可在**函数内部**读出深度参与哈希 ⇒ **零调用点改动、
不改变任何生产限制语义**，代价是同份 code 可能存多份 AST）；② 三处深度统一为 1024。
本轮的语法门测试**不受**此缺陷影响 —— 它的反向对照刻意改用 `Engine::compile` 绕开缓存（见测试内注释）。

### 6.8 探针实测：Rhai 两个深度参数的**真实语义**与**默认值反驳**

写 §6.6 时我断言「`Engine::new()` 默认 `(64, 32)`」——**这是凭印象写的，错了**。
用零锁探针（`rustc --extern rhai=<deps 里的 rlib>` 直编单文件，不碰 cargo 锁）实测：

```bash
rustc --edition 2024 --extern rhai=src-tauri/target/debug/deps/librhai-<hash>.rlib \
      -L dependency=src-tauri/target/debug/deps -o output/tmp-rhai-probe.exe \
      output/tmp-rhai-depth-probe.rs
./output/tmp-rhai-probe.exe src-tauri/src/commands/portfolio-mgr.rhai
```

```
Engine::new() 默认值: max_expr_depth=32  max_function_expr_depth=16      ← 不是 (64, 32)！
DEFAULT: ERR  Expression exceeds maximum complexity (line 94, position 33)

组合网格:
  (  64,  32) OK        (  38,  19) OK        ( 256, 256) OK
  (  64,  64) OK        (  64,  19) OK        (1024,1024) OK
  (  38,  32) OK        (  19,  64) ERR  line 303, position 32

最小可用 max_expr_depth（函数体外） = 38
最小可用 max_function_expr_depth（函数体内） = 19
```

**两个参数是两个作用面，不是叠加**：`rhai-1.26.0/src/parser.rs:3374` 在解析到 `fn` 定义时构造
新的 `ParseSettings`，并把 `max_expr_depth` **替换**为 `max_function_expr_depth()` ——
即**前者管函数体外的顶层代码，后者管 `fn` 体内**。这解释了两个看似矛盾的报错位置：

| 配置 | 谁先超 | 报错位置 |
|---|---|---|
| 默认 `(32, 16)` | 函数体内（16 < 19） | **L94**（在 `fn` 内） |
| `(19, 64)` | 函数体外（19 < 38） | **L303**（顶层） |

**顺带对账生产注释**（`stock_analysis.rs:196-199` 与 `decision.rs:2689-2693` 都写「256 为实测下限(48) 的 ~5 倍余量」）：

| 声称 | 实测（2026-09-21） | 判定 |
|---|---|---|
| 下限 48 | 顶层需 **38**、函数内需 **19** | 量级相符（历史版本的值，现已略降） |
| 256 是 ~5 倍余量 | 256 / 38 = **6.7×** | 基本属实（注释可信，非腐烂声称） |

⇒ 生产的 `(256, 256)` **足够宽**，§6.6 的修复方向正确。
⇒ **反过来也证实**：新门禁的反向对照断言 `strict.compile(code).is_err()` 会被满足
（默认 `(32, 16)` < 所需的 `(38, 19)`，报错的行列号与门禁日志**逐字一致**）。

**这一节的自我批评**：我在同一份报告里主张「参数不得手抄、须从生产读取」，
却在上一节把一个**库的默认值**凭印象写成 `(64, 32)`。更值得记的是**排错时的第一反应** ——
看到「二分得 38/19、两个都够、默认却失败」时，我先去查 Rhai 的源码语义，
而不是**先量一下我自己声称的那个默认值**。凭印象的数字会污染**整条推理链**。

### 6.9 门禁收口（最终轮，2026-09-21 08:29 前后）

改动集冻结为 4 个文件（`output/tmp-dcf-sha-final5.txt`，跑完复算 **4/4 逐字节一致** ⇒ 证据未漂移）：

```
T1  cargo test -p axagent-astock-data --lib            331 passed / 0 failed   T1_EXIT=0
T2  cargo test -p axagent --lib seed_consistency        16 passed / 0 failed   T2_EXIT=0
    cargo clippy -p axagent --all-targets --all-features -- -D warnings        CLIPPY_EXIT=0
    cargo fmt -p axagent / -p axagent-astock-data -- --check                   干净
```

两条本轮新增的关键门禁在最终轮均 `ok`：
`moat_level_vocabulary_matches_rhai_consumer`（§6.2）、`portfolio_mgr_rhai_compiles`（§6.6）。

**⚠ 覆盖范围不能只看 EXIT** —— 最终轮 clippy 段只打了 `Compiling axagent v2.9.9`，
**没有 `Checking axagent`**（`--all-targets` 下含 build script 单元 ⇒ cargo 报 `Compiling`）。
若按「只认 `Checking` 行」判覆盖，会误得出「clippy 没查 axagent」。实际取证改用
**fingerprint 的逐 target 产物名**，clippy 在自己的 0.4–0.5 分钟窗口内落了 16 个单元：

| target 族 | 产物名样例 | 覆盖的含义 |
|---|---|---|
| lib | `lib-axagent_lib.json` | 非 test 形态的库代码 |
| **test-lib** | **`test-lib-axagent_lib.json`** | **`#[cfg(test)]` 模块 ⇒ 本轮新增的两个门禁在此被 lint** |
| bin / test-bin | `bin-axagent.json`、`test-bin-axagent-server.json` | `--bins` |
| test-integration-test | `test-integration-test-skills_tests.json` 等 3 个 | `--tests` |
| test-bench | `test-bench-search_bench.json`、`test-bench-tool_exec_bench.json` | `--benches` |

⚠ 同一 target 在**不同工具**下 fingerprint 哈希不同（`test-lib` 有两个目录：`93a114f6130ac569` 属
`cargo test`、`b3cf15e4a625efde` 属 clippy）—— 数「活跃单元数」时要按工具分开看，
否则会把同一个 target 数成两个而虚增覆盖面。

---

## 6.10 续办项（§6.4 / §6.7）本轮取证与推荐值

> 本轮把 §6.7 的纯 bug **修掉**，并把 §6.4 的四个待裁决项**逐项量化成可拍板的推荐值**。
> 所有数字来自生产库（`output/tmp-continue-scan*.sql` / `.out`）与逐样本复算
> （`output/tmp-continue-analyze.mjs`，公式与 `portfolio-mgr.rhai` 逐字对齐）。

### 6.10.1 §6.7 已修：AST 缓存键缺**解析期**配置（**已落地**）

**缺陷**：缓存桶键 = `code_hash(code)`，而同一份 `portfolio-mgr.rhai` 被三处生产引擎编译，
深度上限分两套（`stock_analysis.rs:199` / `decision.rs:2693` = `(256,256)`；
`rt-workflow/.../code_executor.rs:55` = `(1024,1024)`）⇒ 共用一个桶，
**先跑者的配置决定缓存里那份 AST**，后跑者命中后自己的限制被静默绕过，
且故障形态**取决于按钮点击顺序**。模块头那句「缓存的 AST 可被任何 Engine 执行……
与 AST 本身无关」是**只对执行期成立**的辩护 —— 它正是该缺陷长期未被发现的成因。

**修法**（零调用点改动）：键改为 `code_hash(code) + "|" + parse_config_fingerprint(engine)`，
指纹取三个 rhai **公开 getter**：`max_expr_depth()` / `max_function_expr_depth()` /
`optimization_level()`（`api/limits.rs:261,272`、`api/optimize.rs:24`；
`optimization_level` 经 `api/compile.rs:207` 传进编译链 ⇒ **改写 AST 内容**，必须入键）。

**旋钮分类表（已写进函数文档）**：`set_max_expr_depths` + `set_optimization_level` = 解析期（入键）；
`set_max_operations` / `set_max_call_levels` / `set_max_variables` / `set_max_functions` /
`set_max_modules` / `set_max_string_size` / `set_max_array_size` / `set_max_map_size` = 执行期（**不入键**，
入了只会让缓存无谓分裂）；`set_allow_looping` / `set_allow_if_expr` / `set_allow_switch` /
`set_allow_statement_expr` / `set_disabled_symbols` / `set_strict_variables` / `set_fast_operators`
= 解析期但**本仓零调用点** ⇒ 未取指纹，并留「回流提醒」（谁开始设置就必须同步扩指纹）。

**测试 2 条**：
1. `strict_parse_limits_do_not_borrow_loose_cached_ast` —— 复现原症状（宽松先编 ⇒ 严格必须
   **自己失败**）。夹具的嵌套深度是**量出来的**：零锁探针（`output/tmp-rhai-depth-probe2.rs`）
   实测 `nested_array(12)` 最小可用深度 = **39**（`parens(40)`=83、`nested_array(40)`=123、
   `nested_if(40)`=125）⇒ 严格取 `(8,8)`、宽松取 `(64,64)`。夹具前提在测试内**自证**。
2. `different_parse_config_uses_distinct_cache_buckets` —— 两个旋钮各能把桶分开，
   且含**反向对照**：配置相同的新 Engine 实例**必须**命中同一 AST。
   ⚠ 没有这条反向对照，把随机数混进键也能让正向断言全过 —— 那是「把缓存切碎」的**假修复**。

**顺带修掉两处被本次改动**变成错误**的注释**（注释腐烂是本项目高频缺陷）：
模块头那句；`seed_consistency_tests.rs` 反向对照段里的「按 code 哈希分桶（`rhai_ast_cache.rs:81`）」
—— 行号已漂移（引用行号即会腐烂，改引符号名）。

**零锁预证（等构建锁期间先证死风险点）**：harness crate 的编译要排队 25 分钟量级，
若三个 getter 的名字/签名写错，白等一轮。故先用 `rustc --extern rhai=<deps rlib>` 单文件直编
（`output/tmp-rhai-fingerprint-probe.rs`）验证：三个 getter 可调用、`OptimizationLevel: Debug`
可用、且与 Rust 侧**逐字同格式**的指纹字符串确实把三套生产配置分开：

```
getter 可调用：max_expr_depth=32  max_function_expr_depth=16  optimization_level=Simple
What-If(256)  expr=256/fnexpr=256/opt=Simple      ← 与重跑指纹相同（两处都是 256/256，必须相同）
重跑(256)     expr=256/fnexpr=256/opt=Simple
DAG(1024)     expr=1024/fnexpr=1024/opt=Simple    ← 与上面不同（缺陷正是这两个共用一个桶）
默认(32/16)   expr=32/fnexpr=16/opt=Simple
opt=None      expr=32/fnexpr=16/opt=None          ← 优化级变化也被区分
```

### 6.10.2 Q1(b) 换锚：**不必新引数据源**，但候选之一已证伪

生产库 `financial_snapshots` 实况：**15,597 行 / 13 只 / 2021-09-13 → 2026-09-18（近 5 年日频）**。

| 列 | 非空量 | 判定 |
|---|---|---|
| `pe_ttm` / `pb` / `ps_ttm` / `pcf` | **15597 / 15597 = 100%** | 可用，且是**日频 5 年** |
| `ev_ebitda` | **0** | ❌ 列存在但**从未被填过** —— 与本审计主病（FCF 三列恒 `None`）同类 |
| `roe` / `gross_margin` / `debt_ratio` / `revenue_yoy` / `profit_yoy` | **全 0** | ❌ 同上 |

⇒ **「EV/EBITDA 换锚」不可行**（数据不存在；`backfill_financial_snapshots` 只从
`eastmoney:RPT_VALUEANALYSIS_DET` 取 pe/pb/ps/pcf）。这一条**否掉了原候选之一**，
且否掉它的正是本审计反复出现的那条：**字段存在 ≠ 被填过**。
⇒ 唯一**立即可用**的替代锚是 **PE/PB/PS 历史分位**：`valuation_band.rs` 已实现
（5/10/25/50/75/90/95 分位 + 当前分位 + `verdict`），判据可完全按**数据形态**定义
（分位阈值 25/75），不依赖行业/风格标签 —— 与「按数据形态而非标签下判据」的口径一致。

**推荐**：以 **PE 历史分位**作 DCF 之外的**第二锚腿**（而非直接替换 graham 腿），
理由是它同样不依赖 FCF 口径（对银行/高杠杆标的不失效），且与 graham 的「当前 PE 相对基准偏离」
（见 §二 的 PE 恒等式）**信息不重复**。
⚠ **限制（必须一并解决才能用于工作流）**：该表目前只在 `compute_valuation_band`
（前端开「估值带」图）时触发回填，覆盖仅 13 只 ⇒ 要让估值腿消费它，须让 t-valuation
也能触发（或要求）该回填，否则大批标的会退化成缺腿。

### 6.10.3 §6.4(b) `moat_mult` 方向语义：推荐候选②，但先说清它**改不动任何 action**

38 条样本里 `moat ∈ {宽阔,狭窄}` 仅 **5 条**（去重 4 只：002353 / 600031 / 601698 = 狭窄，
300308 = 宽阔），且 **5/5 的乘子前 σ 全为负** ⇒ 候选③「只在正信号段生效」对这 5 条
**等价于完全不生效**（可直接判定 ③ 不可取）。

| 样本 | 无乘子 | ①现状 `\|σ\|×` | ②方向对称 | ③仅正段 |
|---|---|---|---|---|
| 300308 宽阔 | −0.3388 | **−0.3896** | **−0.2880** | −0.3388 |
| 002353 狭窄 | −0.5623 | −0.5904 | −0.5342 | −0.5623 |
| 600031 狭窄 | −0.0259 | −0.0272 | −0.0246 | −0.0259 |
| 601698 狭窄 | −0.3513 | −0.3688 | −0.3337 | −0.3513 |
| **均值** | −0.3234 | −0.3531 | **−0.2937** | −0.3234 |

**推荐 ②方向对称**（看多 ×1.15 / 看空 ×**0.85** = `1 − premium`）：这才是注释「估值应享受溢价」的字面含义，
也与「护城河是给予估值溢价的依据」一致 —— 现状对**负信号放大**等于说「护城河越宽、
越该看空」，与自身声明相反。

**量化后果**：300308 Δσ = **+0.1016**；按 f5 权重 ≈0.21、posterior 映射 k≈0.3804 估算
Δposterior ≈ **+0.008** ⇒ **不可能翻转任何 action**（阈值带宽 0.10）。
⇒ 这是**一致性修正，不是结果修正**：风险低，但也不是收益项。**故仍请拍板**，不擅自改。

> **2026-09-21 更正留痕**：本节原写「看空 ×0.87」，与本节**表格里的数字**（按 ×0.85 算）
> 及 §6.10.4 的 `+0.15·graham_sig` 推导**不一致** —— 0.87 是「乘除对称」`1/1.15`，
> 0.85 是「加减对称」`1 − 0.15`，两者都自洽但只有后者被量化过。
> 已按 **0.85** 落地（见 §6.11.1），此处文字一并更正。

### 6.10.4 §6.4(c) `f5_fallback_decay` 分腿：量化结果**与直觉相反**

影响面：`is_fallback_anchor=true` **且**两腿齐全 = **8 / 38**（21%）。逐样本（σ 现状 → 分腿）：

| 样本 | dcf / graham | 现状 | 分腿 | Δ |
|---|---|---|---|---|
| 300308 | −85.0% / −81.9% | −0.3896 | −0.5055 | **−0.1159** |
| 601698 | −93.5% / −96.8% | −0.3688 | −0.4803 | −0.1114 |
| 603466 | −62.8% / −93.6% | −0.3189 | −0.4240 | −0.1051 |
| 001313 | −32.7% / −88.9% | −0.2609 | −0.3643 | −0.1035 |
| 600031 | +8.5% / −55.6% | −0.0272 | −0.1188 | −0.0916 |
| 601166 ×2 | +143% / +60.6% | +0.3640 | +0.4543 | **+0.0903** |
| **均值** | | −0.1284 | −0.1862 | **−0.0578** |

**反直觉点**：分腿衰减**不是**「更看多」，而是**更极端**。原因是可复算的：
现状 `fused_sig ×0.5` 会**同时**把 graham 腿减半；改成分腿后
（`0.7·(dcf×0.5) + 0.3·g` vs `0.5·(0.7·dcf + 0.3·g)`）差值恒为 **`+0.15·graham_sig`**
⇒ graham 腿恢复满强度，σ 就**按 graham 自己的符号**更极端。
8 条里 6 条 `graham_sig<0` ⇒ 更看空；601166（graham +60.7%）则更看多（+0.090，符号自洽）。

**口径判断**：V77 的注释明写本衰减「只描述 **DCF 腿**的 FCF 锚定是历史代理」，
并专门加 `dcf_leg_usable` 门以免惩罚 graham —— 分腿衰减是该声明的**完全实现**
（现状只实现了「DCF 被剔除时不惩罚 graham」，未实现「DCF 在位时也只惩罚 DCF」）。
**推荐改**；量化 Δposterior ≈ 0.005 ⇒ 同样**不改变任何 action**。
⚠ 改 `.rhai` 即改 CodeNode 内容 ⇒ 必须升 `TEMPLATE_VERSION`（不升则不落库）。

### 6.10.5 §6.4(a) Q3(c) 数据质量「声称 vs 事实」：落点已定位（**设计已定，未实现**）

**修正一个前提**：`data-quality` 节点**不是 LLM**，而是 **CodeNode + `data-quality.rhai`**
（`seed_stock_analysis.rs:3007` `include_str!("../data-quality.rhai")`，标题「数据质量确定性评分」）
⇒ 这一项有**确定性落点**，不必靠提示词。

**病灶精确到行**：`data-quality.rhai:852`
```rhai
if !present(valuation_dcf_upside) { missing_factors.push("估值上行空间"); }
```
它判的是**派生值是否存在**，而非**底层数据是否取到**。`valuation_dcf_upside` 在
「锚是历史代理（`is_fallback_anchor=true`）」时**照样存在** ⇒ 完整度记 ✔，
而 `operating_cash_flow` 恒 `None` 全链路零告警。这就是 38 条样本 dqi 从不报警的机制。
（顺带：该节点的 25 个注入变量**零白注入**，已核；阴性结果一并留证。）

**设计（最小且可判定）**：
1. `data-quality` 新增 input `valuation_dcf_basis` ← `t-valuation.result.content.dcf.assumptions.basis`
   （该字段今日 F2 已做成**三态**：「当期FCF≤0」/「现金流量表数据缺失」）
2. Rhai 侧**只把「现金流量表数据缺失」计入缺口**；「当期FCF≤0」**不算缺口**
   —— 那是标的真实经营状态，不是我方取数失败（同 F2 的两态区分纪律，**不可合并**）
3. 落进**独立新列表**（如 `upstream_data_gaps`）+ `warnings`，**不动 0.30 权重的
   `factor_completeness`**：避免「列表长度 ≠ 公式分母」这类不一致（Rust
   `compute_factor_completeness` 与 Rhai 两侧有口径约束，本仓有门禁守它）
4. 升 `TEMPLATE_VERSION` → **68**（`input_mapping` 变更即模板内容变更）

**为什么不本轮直接做**：需要同时决定「是否扣分 / 扣多少 / 前端如何呈现 / 是否进 grade」，
属独立设计。**但其中「只要告警、不扣分」的一半是零风险的** —— 若拍板这一半，下一步即可落地。

---

## 6.11 §6.4 四项拍板项的落地（**2026-09-21 用户裁决后已实现**）

> 用户裁决原文：`是`（对 §6.10 的四条推荐值全部采纳）。
> 本节记录**实际落地形态**，特别是与 §6.10 推荐值**有出入**的地方（不藏）。
> ⚠️ 与 §6.10 的两处**口径更正**见本节末。

### 6.11.1 四项落地对照

| 项 | §6.10 推荐值 | 实际落地 | 落点 |
|---|---|---|---|
| ① `moat_mult` 方向 | 方向对称（看多 ×1.15 / 看空 ×0.87） | **方向对称：看多 ×1.15 / 看空 ×0.85** | `portfolio-mgr.rhai` `let moat_mult` |
| ② `f5_fallback_decay` | 分腿（只压 DCF 腿） | **分腿**：`dcf_anchor_decay` 进融合内、只乘 DCF 腿 | `portfolio-mgr.rhai` 融合表达式 |
| ③ Q3(c) 上游缺口 | 只告警不扣分 | **`upstream_data_gaps` + warnings，不进任何公式** | `data-quality.rhai` |
| ④ Q1(b) 第二锚腿 | PE 历史分位作 DCF 之外的第二锚腿 | **第三锚腿**，权重 0.56 / 0.24 / 0.20 | `portfolio-mgr.rhai` + 新 ToolNode |

**① 的口径更正（诚实登记）**：§6.10.3 的**文字**写「看空 ×0.87」，但该节**表格里的数字**
（300308 `−0.3388 → −0.2880`）是按 `1 − 0.15 = 0.85` 算的 —— 表格与 §6.10.4 的
`+0.15·graham_sig` 推导、以及 `output/tmp-continue-analyze.mjs` 的 `moatMult = 1 - premium`
三处**互相印证 0.85**，只有标题行那句文字写成了 `1/1.15 = 0.87`（**乘除对称**）。
两种「对称」都自洽，但 Δ 现值只有按 0.85 算过 ⇒ 落地取 **0.85**，
并把那句文字更正为 `×0.85（= 1 − premium）`。**对 action 无影响**（两者 Δposterior 均 < 0.01）。

**④ 的口径说明**：§6.10.2 的措辞是「以 PE 历史分位作 DCF 之外的**第二锚腿**」，
容易读成「总共有两条腿」。实际实现是**三条腿并存**（DCF + graham + 分位带），
「第二」指「第二个**非 DCF** 的独立锚来源」。权重设计见 6.11.3。

### 6.11.2 ③ 的落法：**加布尔量，而不是匹配文案**

§6.10.5 的设计写「Rhai 侧只把『现金流量表数据缺失』计入缺口」，字面读像是让 Rhai
去匹配 `basis` 的文案。**不这么做**，理由在 `portfolio-mgr.rhai` 已有前例：

> `valuation_dcf_anchor_is_fallback` 当初正是从 `== FCF_FALLBACK_BASIS` 的**等值比较**
> 改成布尔量的 —— 因为 `basis` 是给人与 LLM 看的诊断文本，**文案一改判据就静默失效**。

而本轮的 `basis` 恰好刚被拆成三态（`"当期FCF"` / `"当期FCF≤0，…"` /
`"现金流量表数据缺失（该数据源未提供 OCF/资本开支），…"`），**任何等值匹配都会失效**，
子串匹配则只是把脆性藏得更深。故：

- `astock-data::DcfAssumptions` 新增 **`fcf_data_missing: bool`**，
  判据 `fcf_basis == FCF_MISSING_BASIS`（**常量等值**，与 `is_fallback_anchor` 同源同纪律）
- 落库字段 `dcf.assumptions.fcf_data_missing` → 注入 `valuation_dcf_fcf_data_missing`
- `data-quality.rhai` 消费该布尔量，**不碰文案**

**两个字段正交**（这是设计要点，也是测试的对照点）：

| 情形 | `is_fallback_anchor` | `fcf_data_missing` | 处置 |
|---|---|---|---|
| 当期 FCF 真为负 | `true` | **`false`** | 软衰减（DCF 腿 ×0.5），**不**告警 |
| 现金流量表数据缺失 | `true` | **`true`** | 软衰减 + **上游缺口告警** |
| 当期真实正 FCF | `false` | `false` | 无处置 |

⇒ 测试 `dcf_basis_tells_three_states_apart` 在三个分支各加一条断言，
其中 (a) 真为负那支的 `assert!(!a.fcf_data_missing)` 是**反向对照** ——
缺了它，把字段实现成 `= is_fallback_anchor` 也能让 (b) 的正向断言全过。

`upstream_data_gaps` **刻意不动 `factor_completeness`**：Rust 侧
`pm_compute_factor_completeness` 的分母（9 项）与 Rhai 侧 `missing_factors` 的项数
绑死，掺进第三张表会出现「列表长度 ≠ 公式分母」。故本轮**只做「给用户看见」**，
扣分与否仍待裁决（§6.10.5 的未决部分**未被用户采纳**，保守起见不动）。

> **⚠️ 差点落空：`warnings` 是装饰字段**（本轮实测发现，未修、仅登记）
>
> `DataQualityReport.warnings`（`src/types/stock-analysis.ts:457`）有**类型声明但零渲染点** ——
> `DecisionBanner.tsx` 只渲染 `missing_factors` 与 `diagnostics`，前端从未读过 `warnings`。
> ⇒ 若 ③ 只把缺口 push 进 `warnings`，**用户看不到**，本项「给用户看见」的目的**直接落空**
> （而所有机械门禁都会全绿）。这正是本审计反复出现的「**写进产物 ≠ 有人消费**」同族。
>
> 故 ③ 实际做了**三处**接线才闭环：
> 1. `src/types/stock-analysis.ts` 新增 `upstream_data_gaps?: string[]`（带两态纪律说明）
> 2. `src/components/stock-analysis/DecisionBanner.tsx` 新增渲染块（橙色，区别于红色的
>    `missing_factors`；紧邻其下，便于对照）
> 3. 11 个 locale 新增 `stockAnalysis.upstreamDataGaps`
>
> 验证：`tsc --noEmit` 干净；`check-hardcoded-i18n.sh` 全过；
> `check-i18n-untranslated.mjs` 未翻译 0 项；`check_i18n.py` 全 11 locale 通过。
> ⚠️ `warnings` 本身要不要渲染属**产品决策**（渲染后会一次暴露大量既有告警）⇒ 未动。

### 6.11.3 ④ 的权重设计（**报告未给权重，此处补并明示**）

§6.10.2 只说「作第二锚腿」，**没有给权重**。落地取：

```
w_dcf = 0.56   w_graham = 0.24   w_band = 0.20      （三者和 = 1.0）
fusion = (0.56·dcf + 0.24·graham + 0.20·band) / Σ可用腿权重
```

选 0.56 / 0.24 / 0.20 而不是 0.7 / 0.3 / 0.2 的理由：

1. 前两腿**保持 7:3 内部比例**（0.56:0.24 = 7:3）；
2. band 缺位时 `Σ = 0.8` ⇒ dcf 份额 `0.56/0.8 = 0.7`、graham `0.24/0.8 = 0.3`
   ⇒ **逐位等于改动前**（这是向后兼容的判据，见 6.12 的 A1 段实测）；
3. 三者和恰为 1.0 ⇒ **band 腿份额恒为 0.20**（若写 0.7/0.3/0.2 则为 0.2/1.2 = 0.167），
   「新腿占两成」这句话在两种状态下都成立、可直接断言。

**信号映射**：`band_sig = clamp((50 − pct)/50, −1, +1)`，pct 为 PE 当前历史分位（0–100）。
- 用**线性**而非 `pm_saturate`：分位本身已是 `[0,100]` 有界量，50 是天然中性点；
  `pm_saturate` 需要再引入一个标定常数，且极值只到 ±0.5；
- `pct=25 → +0.5`、`pct=75 → −0.5`，与 `valuation_band.rs` 自带的 25/75 阈值口径一致；
- **可用性门**：`present(x) && 0 ≤ x ≤ 100`。亏损股（PE ≤ 0）与有效样本 < 20 时
  `currentPercentile` 为 `null` ⇒ 自动不可用（对亏损标的自动规避，无需额外白名单）。

**新 ToolNode `t-valuation-band`（本项最关键的接线）**：
`compute_valuation_band` 此前**唯一**的回填触发点是前端「估值带」图 ⇒ DB 实查全库
只有 **13 只**有 `financial_snapshots`。若估值腿要消费 `currentPercentile` 而覆盖率是
13/5000+，新腿就是**永久空壳**（恒不可用 ⇒ 表面只是「没启用」）。
故把它接进工作流：`t-valuation → t-valuation-band → portfolio-mgr`。

⚠️ **刻意不并入 `algo_tools` 表**：那张表有两处下游消费者（`raw-data` 的 `input_sources`
与「给每个 tool 节点显式挂边」的循环），加一行会连带把本节点拉进 raw 聚合，
并让 aggregator 的 description 里写死的「16 个工具节点（12 + 3 + 1）」计数腐烂。
本节点的唯一消费者是 portfolio-mgr，无需进 raw 聚合。

### 6.11.4 版本与配套

- `TEMPLATE_VERSION` **67 → 68**（三处 `.rhai` + 三处 `input_mapping` + 一个新节点全属模板内容）
- ⚠️ 本轮**未**核对 DB 现值（无 DB 连接做 `SELECT version`）—— 若真库已被并发路径写到 ≥ 68，
  本批会被 `>=` 门静默挡住。这是本项目已反复踩的坑，**留作下一步必查项**。

### 6.11.5 新增门禁：`valuation_leg_weights_degrade_to_legacy`

位置 `seed_consistency_tests.rs`，断言 f5 权重结构的**不变量**（4 条）：

| # | 断言 | 为什么 |
|---|---|---|
| ① | 恰有 3 个非零权重字面量 | 漏抓/多抓都要报红（`f64_literals_in` 不认整数与科学计数法 ⇒ 会静默漏抓） |
| ② | 三者之和 == 1.0 | ⇒ band 腿份额**恒为 0.20** |
| ③ | `w_dcf / (w_dcf + w_graham) == 0.7` | ⇒ band 缺位时 dcf 份额回到 0.7（**向后兼容的唯一保证**） |
| ④ | 最终 `clamp(...)` 内**不含** `dcf_anchor_decay` | 防分腿衰减被**二次施加** |

之所以需要它：0.56/0.24/0.20 的**选取理由**（三者和 1.0 ⇒ band 份额恒为两成，
而非 `0.2/1.2 = 0.167`）原本只写在注释里 —— 谁改成 `0.7/0.3/0.2` 都能编译、能跑、
注释也不报错。结构性不变量**只写注释等于没有守护**。

⚠️ 切片终点取 `let dcf_anchor_decay`（**不是** `let fused_sig =`）：后者会把
`dcf_anchor_decay` 的 `0.5 / 1.0` 一起圈进来 ⇒ 5 个非零字面量 ⇒ 数量断言假红（首版实测）。

---

## 6.12 回归对账：四项改动的**分量隔离**验证

> 脚本：`output/tmp-v79-replay.mjs`（38 条生产样本，公式与 `portfolio-mgr.rhai` 逐字对齐）
> 输出：`output/tmp-v79-replay.out`

本轮同时改了 f5 的**四件事**，其中三件会改 σ。若不把分量分开验，
「结果变了」既可能是新腿生效（预期）也可能是权重写错（缺陷）—— **无法区分**。
故断言拆成三条，各带明确预期值：

| 断言 | 子集 | 预期 | 实测 |
|---|---|---|---|
| **A1** 权重结构退化态 | `fb=false` 且 moat 中性，29/38 条 | 新 == 旧（逐位） | ✔ 最大绝对差 `1.1e-16` |
| **A2** 分腿衰减作用量 | `fb=true` | Δ 恰为 `+0.15·graham_sig` | ✔ 误差 ≤ `4.2e-17` |
| **A3** moat 方向对称 | moat ∈ {宽阔,狭窄}，5 条 | 符号保持、仅强度变 | ✔ 5/5 符号保持 |
| **B** 新腿非空壳 | 全 38 条 | 给定 pct ⇒ σ 必变 | ✔ 0 条未变 |

**A2 是本轮最强的自证**：§6.10.4 从代数上推出「分腿与不分腿之差恒为 `+0.15·graham_sig`」，
实测逐位吻合（`603466` Δ=−0.1051、`601166` Δ=+0.0904、`001313` Δ=−0.1035，误差 ~1e-17）
⇒ 实现与推导**是同一件事**，不是「看起来差不多」。

### 6.12.1 首版断言在真数据上假红（判据设计错误的记录）

首版断言写的是「band 缺位 ⇒ 新 == 旧（仅 moat 样本例外）」，在真数据上**报红**：
9 条偏离里 **4 条不是 moat 样本**，而是 `is_fallback_anchor=true` 的样本。
根因**不是实现错**，而是断言把「② 这个**预期变更**」当成了「不该有的偏离」——
把两个正交分量压进一条断言，必然要么假红要么假绿。
⇒ 这正是判据 #7「审计脚本自身会撒谎」的同族：**断言集合错误会伪装成被测对象缺陷**。

### 6.12.2 影响分级（**四项里只有一项有实质影响**）

> ⛔ **2026-09-21 当天更正：本表 ④ 一行作废。** 自查发现 `t-valuation-band` 调用的
> `compute_valuation_band` **未注册为工具**（详见 **§6.14**）⇒ 新腿恒不生效
> ⇒ **④ 的真实影响是 0**，`band_leg_usable` 恒 `false`、逐位退化回 `0.7/0.3`。
> ①②③ 三行不受影响（它们的实现路径不含该节点）。原文保留以供对照。

| 项 | Δσ 量级 | Δposterior | 是否可能翻转 action |
|---|---|---|---|
| ① moat 方向对称 | ≤ 0.10（5 条样本） | ≈ **0.008** | 否（阈值带宽 0.10） |
| ② f5 分腿 | ≤ 0.116（8 条样本） | ≈ **0.005** | 否 |
| ③ 上游缺口告警 | **0**（不进任何公式） | 0 | 否（设计如此） |
| ④ 第三锚腿 ~~≤ 0.615~~ | **0（实际）** | ~~≈ 0.049~~ → **0** | **否（恒不生效）** |

⇒ **③ 是零影响**（只加一张告警表）、**①②是一致性修正**（消除「声明与实现相反」，
不是结果修正）、**④ 原称"唯一实质变更"，实测为 0**（§6.14）。

### 6.12.3 ④ 的真实影响面（DB 实算，不是假设情境）

> ⛔ **同上作废**：本节全部数字（含「601698 是唯一从无到有」）都在
> 「新腿生效」的前提上计算，而该前提不成立。保留原文以记录推算过程本身。

分位数据不是假设出来的 —— 只读查询 `output/tmp-band-coverage.mjs` / `tmp-band-pct.mjs`：

- `financial_snapshots` 全表 **13 只**，且这 13 只**全部**落在 38 条样本内；
- 每只 1069–1211 条日频快照（2021-09 → 2026-09），`pe_ttm > 0` 占比 62%–100%；
- 样本去重 26 只 ⇒ **另 13 只无任何分位数据**（新腿对它们不可用 ⇒ 行为不变）；
- 38 条样本中 **17 条受影响 / 21 条不变**。

**真实分位（DB 实算，口径 = `current_percentile`）**：
`002335=36.95 002353=92.44 002837=86.00 300308=64.20 300620=82.37 600089=79.23
600876=53.58 601166=64.34 601231=85.43 601698=80.77 603353=85.99 603466=73.56 688072=33.44`

⇒ **11 / 13 只当前 PE 处于 5 年中位以上** ⇒ 分位腿在这批样本上多数给**负信号**，
与「38 条 σ 普遍为负」**同向** ⇒ 不会出现假设情境 `pct=20` 那种 +0.6 的强正信号。

**最大影响恰好来自「从无到有」那一类**：
`601698` 有一条样本原本 **两腿皆缺**（`f5_weight = 0`，该因子**完全不参与** posterior），
现在有了分位腿 ⇒ σ 从 `0.0000` 变成 `−0.6154`，**Δposterior ≈ 0.049**。
这不是「调参」而是「从无到有」，是本项语义变化最大的一条，
**必须在新样本上复跑才能确认真实后果**（本轮只能量到 σ 层面）。

次大的是 `688072`（+0.2082，pct=33.44 历史低位 ⇒ 把原本 −0.71 的强看空修正到 −0.50）
与 `002335`（+0.1928，pct=36.95）—— 这两条是「两腿齐全 + 分位腿与旧融合值**符号相反**」
的情形，归一化会把旧融合值向 0 拉（数学上 `Δ = 0.20 × (band_sig − fused_旧)`）。

### 6.12.4 顺带发现：旧复算脚本漏了 `applicable` 门（已在此脚本修正）

`output/tmp-continue-analyze.mjs` 第 38 行把 `dcfUsable` 写成 `dcfUpside !== null`，
**漏了生产侧的 `applicable == false ⇒ 整腿剔除` 门** ⇒ 对 `applicable=false` 的样本
（如 601166）算出的**绝对值偏离生产**。选项间的**差值**仍可比（门对三个选项同样作用），
故 §6.10 的**结论不受影响**；但绝对值不可当真。
**对本批 38 条的实际影响 = 0**（实测这 38 条的 `applicable` 全为 `true` 或缺失，
`output/tmp-v79-replay.out` 的 D 段输出「无 applicable=false 样本」）——
一并登记，避免下次有人拿旧脚本的数字当生产值。

### 6.12.5 零锁预证（等构建锁期间先证死风险点）

Rhai 侧是**纯文本**改动，语法错误只会在用户点按钮时才暴露（
`portfolio_mgr_rhai_compiles` 虽已覆盖，但那条测试要等 cargo）。故先用
`rustc --extern rhai=<deps rlib>` 单文件直编（`output/tmp-rhai-syntax-probe.rs` /
`tmp-rhai-syntax-run.mjs`）：

```
OK    portfolio-mgr.rhai      OK    data-quality.rhai
OK    risk-level.rhai         OK    regime-weights.rhai        失败 0 / 4
```

**踩到的坑（同类工具必看）**：首版脚本用「toolchain 目录序最后一个」挑 rustc ⇒ 选中
`stable`，而项目 `rust-toolchain.toml` 声明 **1.97.0** ⇒ 报
`E0514: found crate rhai compiled by an incompatible version of rustc`（7 个 rlib 全被拒）。
⇒ **toolchain 必须从 `rust-toolchain.toml` 读，不能猜**。

### 6.13 落库前置：真库版本核对（**本批的交付硬前置**）

版本门是 `existing.version >= TEMPLATE_VERSION ⇒ 跳过重建`（`>=`）。**代码全绿也不能
证明门会开门** —— 四道机械门禁只回答「能否编译」，没有一道回答「版本门会不会开」。
故本批收尾必须回真库读一次数字（只读，`psql -w` + `PGPASSWORD` 环境变量，口令不进 argv）：

```
stock-analysis | v65 | updated_at = 2026-09-21 05:36:56
```

快照表（`workflow_template_versions`）佐证同一时刻：

| 快照 | 落库时刻 | 含义 |
|---|---|---|
| `stock-analysis_v58` | 2026-09-21 00:22 | 上一批（v58）的**被替换前**副本 |
| `stock-analysis_v62` | 2026-09-21 05:28 | 第一次升级（58 → 62） |
| `stock-analysis_v64` | 2026-09-21 05:36 | 第二次升级（62 → 64 → 65） |

**结论：`65 < 68` ⇒ 门会开门，本批会重种。** ✔

**但同一份取证也暴露了两件事，必须登记：**

1. **66 / 67 两批常量变更从未落库**。DB 停在 65，而代码常量在改动前已是 67
   ⇒ 自今早 05:36 那次启动之后，**两批改动一行都没进过 DB**。它们会随本批 68
   **一次性合并落库**（种子是全量重建 `ActiveModel`，不需要逐版补）——所以**不丢**，
   但意味着「67 那一批我验过落库」这个说法**从来没有成立过**，此前任何基于 DB 的
   该批结论都只反映 65 版模板。

2. **§6.12 的回归对账只证到代数层面，没证到接线层面。**
   `tmp-v79-replay.mjs` 的 B 段（「新腿非空壳」）喂的是**构造值 `pct=20`**，
   它证明的是「`sigma` 公式里 band 腿确实参与融合」；它**不能**证明「生产链真的把
   `valuation_pe_percentile` 注入了 `portfolio-mgr`」——生产上该变量来自
   `t-valuation-band` 节点的 `result.content.metricPe.currentPercentile`，
   而那条变量定义要到**重种之后**才存在于 `workflow_templates.variables`。
   接线层面现有的证据只有**静态的两条**：
   `audit-inject-coverage.mjs --strict`（①差集空 / ②悬空映射空，已通过）
   与 seed 源码里 `nodes` / `edges` / `variables` 三处定义齐备。

   重种后的核对**已落成可直接执行的脚本**：`output/tmp-v79-postseed-verify.sql`
   （`psql -f`，只读，含 ①–⑥ 六段）。**它已在 v65（重种前）上真跑过一遍**，
   结果即**负对照**：

   | 段 | 判据 | **v65 实测（负对照）** | 重种后应为 |
   |---|---|---|---|
   | ① | `version` | **65** | **68** |
   | ② | 新节点 / 两条新边 | **f / f / f** | t / t / t |
   | ③ | 两个新注入变量 | **f / f** | t / t |
   | ④ | 规模 `vars / nodes / edges` | **360 / 62 / 153** | **362 / 63 / 155** |
   | ⑤ | **正控**（路径有效性） | `t / t / t` | 保持 t |
   | ⑤b | 双口径互证 | `?` 版与 `LIKE` 版**逐位一致** | 保持一致 |

   **⑤ 正控是这段脚本的命门，不是装饰。** ②③ 的判据形态是
   `jsonb_path_query_array(col,'$[*].<key>') ? '<id>'`，若 `<key>` 写错，
   `?` 会**恒为 f** ⇒ **无论重种成功与否都报 f**，永远长得像「用户没重启」。
   ⇒ 拿必然存在的旧节点 / 旧变量问同一表达式，全 `t` 才说明 ②③ 的 `f` 是**真的没有**。

   ⚠ **本段脚本第一版就在这里栽了一次（自证记录）**：我选的「必然存在的控制变量」
   `stock_code` 报 `f`。`stock_code` 看着必然存在，但它是 trigger 的
   `input_param`（见 ⑥：`first_node.config.config.stock_code = "{{stock_code}}"`），
   只是**被引用的模板变量名**，不一定是 `variables` 数组里的一条定义；
   换成 `analysis_depth`（`variables[0].name`）后 `t`。
   ⇒ **正控失败有两种含义（路径写错 / 控制量本身选错）**，必须先查清是哪一种，
   否则会拿一个假的「路径失效」去否定一个真实的「确实没有」。
   故 ⑤b 再加**双口径**（`?` 版 vs `LIKE` 版）互证，两者不一致才是真的路径问题。
   （同族判据 #7、#619：判据工具自身会撒谎，0 命中 ≠ 没问题。）

   **这三条为空的含义不是「实现有问题」，而是「用户还没重启 app」** ——
   种子由 `ensure_stock_analysis_experts_seeded` 在启动期执行，
   不启动就不会重种。**本批不能在「未重启」的前提下宣称已生效。**

   > ✅ **已闭环（2026-09-21 13:25:44）**：`tauri dev` watcher 因本批源码改动自动重编并重启，
   > 重种**已真实发生** —— 见下方「重种后实测」与 §6.17。**上段"用户还没重启"已不适用。**

   **重种后实测（同一脚本，2026-09-21 13:2x 实跑）**：

   | 段 | 判据 | v65（负对照） | **重种后实测** | 与预测 |
   |---|---|---|---|---|
   | ① | `version` | 65 | **68** | ✅ 符 |
   | ② | 新节点 / 两条新边 | f / f / f | **t / t / t** | ✅ 符 |
   | ③ | 两个新注入变量 | f / f | **f / f** | ❌ **判据问错了字段，见下** |
   | ④ | 规模 `vars / nodes / edges` | 360 / 62 / 153 | **360 / 63 / 155** | nodes/edges ✅ 符；**vars 不符** |
   | ⑤ | 正控（路径有效性） | t / t / t | **t / t / t** | ✅ 保持 |

   **③ 与 ④ 的 `vars` 不符不是缺陷，是这两个判据的字段选错了**（自我更正）：
   本批新增的两个名字（`valuation_pe_percentile` / `valuation_dcf_fcf_data_missing`）
   在 seed 里是**节点级 `config.input_mapping` 的条目**，
   **不是**模板级 `variables` 数组的元素 ⇒ 拿 `variables ? '<name>'` 问必然报 `f`，
   且 `var_count` 必然不变。用**正确字段**重查（`output/tmp-v79-postseed-verify2.sql`）：

   ```
   ① 文本级：pe_pct_in_nodes=t   flag_in_nodes=t
   ② 正控（同口径）：ctrl_old_mapping  = t   （必存在的旧映射名 valuation_dcf_upside）
                    pe_pct_in_variables_not_nodes = f   ⇒ 确认它不在 variables 里
   ③ 结构级：data-quality.input_mapping (1943 字符) / portfolio-mgr.input_mapping (4543 字符)
             —— 两个节点的 input_mapping 确实持有这两个名字
   ```

   ⇒ **本批的 end-to-end 落库是完整的**；原判据「`vars` 应恰为 362」**作废**
   （它建立在"新映射进 `variables`"的错误假设上，属"期望值自证"——期望值本身没被验证过）。
   ③ 的 `f / f` **一度被我读成「变量没进库、腿仍是空壳」**，是正控 + 双口径把它拦下来的：
   **正控全 `t` 说明路径有效，于是问题只能出在「我问的那个字段里本来就没有它」**。

---

## 6.14 ⛔ P0 自查发现：④ 新增的 `t-valuation-band` 调用了一个**未注册的工具**

> **这是我自己在本轮引入的缺陷，且是本批四项里最严重的一条。**
> 记录它的价值不在认错，而在：**四道 cargo 门禁 + 一道专门的工具解析门禁全绿，
> 却谁都没抓到它** —— 因为它们的覆盖面各差一跳。

### 6.14.1 缺陷本体：一条 5 跳的调用链，第 4 跳断

```text
t-valuation-band（ToolNode，tool_name = "compute_valuation_band"）
  ↓ ① rt-workflow 的 ToolExecutor 执行
    crates/rt-workflow/src/work_engine/executors/tool_executor.rs:122-140
    → context.tool_registry.execute_tool(tool_name, …)        ← 走 ToolRegistry
  ↓ ② ToolRegistry 里的 astock 条目由 stock_mcp_tools() 批量包装
    crates/tools/src/tools/astock_data.rs:118-127
    → StockMcpTool { name, … }，call 委托 execute_mcp_tool
  ↓ ③ crates/astock-data/src/mcp_tools.rs
    需要「schema 列表里有」＋「dispatch 里有」**两处都在**
  ↓ ④ ✘ **`compute_valuation_band` 两处都没有**
    · :432/:1202 compute_scoring        ✔
    · :495/:1261 compute_valuation      ✔
    · :507/:1577 compute_portfolio_risk ✔
    · :191/:1069 get_stock_dragon_tiger ✔
    · compute_valuation_band            ✘ schema 无、dispatch 无
  ↓ ⑤ 后果：ToolResolver 解析为 None
    → core.rs 的 Failed 分支 `emit degraded: true` **静默吞掉**
    → 节点标记 **completed**，但输出为空 ⇒ `valuation_pe_percentile` 恒空
    → `band_leg_usable = false` ⇒ `fusion_wsum` 只剩 dcf+graham ⇒ **归一化退化回 0.7/0.3**
```

**`compute_valuation_band` 只存在为 Tauri 命令**（`src/commands/stock_analysis.rs:3449-3455`，
`#[agent_command(...)] + #[tauri::command]`）。而 `#[agent_command]` 宏
（`crates/axagent-agent-macro/src/lib.rs:45-64`）**只做两件事**：生成
`CommandMetadata` 常量 + `inventory::submit!`。它的**唯一消费者**是
`tests/generate_command_docs.rs`（生成命令文档）——
**不存在任何「命令 → 工具」的桥**。⇒ 命令 ≠ 工具。

### 6.14.2 为什么四道 cargo 门禁全绿

| 门禁 | 它回答的问题 | 为何答不了本缺陷 |
|---|---|---|
| `cargo check --all-targets` | 能否编译 | 工具名是**字符串字面量**，编译期无法校验 |
| `clippy -D warnings` | 有无 lint 风险 | 同上 |
| `cargo fmt --check` | 格式 | 无关 |
| `cargo test` | 断言是否成立 | 见下（专门的门禁也漏了） |

### 6.14.3 为什么**专门的**解析门禁也假绿（判据层面，最值得记的一条）

`seed_consistency_tests.rs` 里**已有**一条正是为此而生的门禁
（`assert_all_resolvable`，:82-97；注释 :11-13 原文写着「运行时 ToolResolver 返回
None → 被 core.rs 的 Failed 分支 emit degraded: true **静默吞掉**…这是最隐蔽的故障形态」）。
它把声明的工具名与解析空间（`registry ∪ stock_mcp_tools ∪ industry_chain ∪ rhai`）比对。

**它漏掉本缺陷的原因是抽取口径**（:33-53）：

```rust
if let Some(start) = line.find("name: \"")      { … }   // ToolDef.name 形态
if let Some(start) = line.find("tool_name: \"") { … }   // RhaiToolDef 形态
```

只认这两种**文本形态**。而本轮新增的节点用的是**位置参数**形态：

```rust
nodes.push(tool_node(valuation_band_id, "估值分位带", "compute_valuation_band", …));
```

⇒ `compute_valuation_band` **从未进入 `declared` 集合** ⇒ 门禁连"这个词存在"都不知道
⇒ 静默通过。**这是「门禁覆盖率随代码形态漂移」的典型**：门禁本身没坏，
它只是不认识新写法（同族 #643：片段匹配面过宽/过窄，两个方向都会假绿）。

### 6.14.4 后果：§6.12 关于 ④ 的全部量化结论**作废**

§6.12.2 / §6.12.3 里 ④ 的影响面（「两腿齐全 ≈ 0.016」「两腿皆缺（从无到有）≈ 0.049」，
以及「601698 是唯一从无到有」）**建立在「新腿真的会生效」这个前提上，而该前提不成立**。

**真实后果**：`valuation_pe_percentile` 恒空 ⇒ `band_leg_usable` 恒 `false`
⇒ `fusion_wsum = 0.56 + 0.24`（或单腿）⇒ **逐位退化回改动前的 `0.7/0.3`**
⇒ ④ 的实际影响 = **0**。

⚠ 这**同时说明 §6.13 的核对脚本设计是对的、而我的读法是错的**：
重种后 ②（节点/边存在）会翻 `t`、③（变量存在）也会翻 `t`，
但**它们都只证明「声明进了模板」，不证明「工具能被调用」**——
正是判据 **#652** 那条「代数层面 ≠ 接线层面」的又一个实例，
只是这次断的不是变量注入，而是**更下游的工具注册**。

### 6.14.5 修法（**三个选项，各有语义代价，请拍板**）

| # | 修法 | 优点 | 代价 / 风险 |
|---|---|---|---|
| **A** | 在 `mcp_tools.rs` 加 schema + dispatch，**改用 client 在线拉**历史估值序列（`vendors/eastmoney.rs:697` 附近已有接口），再调同 crate 的纯函数 `valuation_band::compute_valuation_band()` | 同 crate、无需 DB、复用纯函数、与既有工具同形 | ~~在线版是每次现拉~~ **← 此句已被 §6.14.5.1 更正：`AStockClient::get_valuation_history` 自带缓存**。真实残余风险见 §6.14.5.1 末段 |
| **B** | 把「读表」能力下沉（`tools` crate 有 `tools/database.rs::get_db()`、`tools/opc.rs::get_db()` 先例），在新工具实现里用 `get_db()` + 复用 `valuation_band` 纯函数 | 保住「读本机表」语义 | 需把命令层 `load_financial_snapshots` 的查询**重复实现或下沉共享**（AGENTS.md 规则 12 要求不重复定义）；下沉面较大 |
| **C** | 撤掉 `t-valuation-band` 节点与两条边，④ 明确标记为**未落地** | 最诚实、零风险 | 放弃用户已裁决的 ④；`valuation_pe_percentile` 变量也应一并撤 |

**本轮不做任何一选**（三选一涉及数据来源语义 / 跨 crate 下沉，属需拍板的架构决策）。
**当前状态是安全的**：该节点被 degraded 静默吞掉 ⇒ **不破坏既有结论**，
①②③ 照常生效，④ 等于未生效 —— 但**必须让用户知道「④ 现在不生效」**，
不能沿用 §6.12 那句「唯一实质变更」。

### 6.14.5.1 推荐 **A**：拍板前补的三次取证，把 A 的两个疑点都消掉了

上表是**拍板前**写的，其中对 A 的两条担心（「要新引数据源」「每次现拉」）经复核**都不成立**：

| 疑点 | 复核结论 | 证据 |
|---|---|---|
| 「在线拉」要新引数据源 / 新依赖 | **不用**。取数接口与本工具**同 crate**，且 `AStockClient` 上**已有 `pub async fn get_valuation_history`** | `vendors/eastmoney.rs:700`（实现）+ `lib.rs:2264`（`pub` 委托） |
| 「每次现拉」⇒ 每次多打一次 API | **不成立**：`lib.rs:2264` 的方法**自带缓存**（`:2272` 先读缓存反序列化，miss 才走 vendor）⇒ 命中缓存时**零 API 调用** | `lib.rs:2264-2284` |
| 工具路径拿不到 vendor client | **拿得到**：`execute_mcp_tool(client: &AStockClient, …)` 的签名**本来就带 client**，全部 20+ 个网络工具都走 `client.get_xxx()` | `mcp_tools.rs:831-832`，`:943`–`:1120` |
| 纯计算部分要重写 | **不用**：`valuation_band.rs:120` 的 `compute_valuation_band<S: FinancialSnapshotLike>` 是**泛型纯函数**，trait 只要 4 个 getter；**astock-data crate 内零 DB 依赖** | `valuation_band.rs:120`、`:190-194`、`lib.rs:58`（已 `pub use`） |

**⇒ A 的实施面**（三处，无 DB、无跨 crate 搬迁、无重复定义）：

1. `mcp_tools.rs`：+1 schema（`stock_code` 必填、`years` 可选默认 5）+1 dispatch ⇒
   `client.get_valuation_history(code, years)` → 按**与命令层同一口径**裁剪 →
   本地 adapter（实现 `FinancialSnapshotLike` 的 4 个 getter）→ 调纯函数
   `valuation_band::compute_valuation_band(&code, &samples, samples.last())`。
   约 55 行。
2. `seed_consistency_tests.rs`：扩 `seed_tool_def_names` 抽取面（**必须同批**，见 §6.14.6）。
3. `seed_stock_analysis.rs`：**不改**（`t-valuation-band` 节点本身是对的，工具名已一致）。

**A 优于 B 的决定性理由**：B 的唯一卖点是「保住读本机表语义」，但这个卖点是**假的** ——
① 两条路径**本来就共用同一个 `AStockClient::get_valuation_history`（含同一份缓存）**
（命令 `stock_analysis.rs:3578` vs 拟新增的 dispatch）⇒ 原始数据同源，差别只在**是否落库**；
② B **也必须**调 `get_valuation_history` 才能填表（`financial_snapshots` 全项目无写入路径，
只有建表迁移 + 读命令）⇒ **B = A + DB 依赖 + 命令层下沉**，没有独立收益；
③ B 的覆盖面对首跑更差：上一轮 DB 实查 `financial_snapshots` **只有 13 只**
（§6.12.3），而样本集 **26 只**去重 ⇒ 表路径下**另 13 只**拿不到 band 腿（`verdict=insufficient`）
⇒ 静默退回 `0.7/0.3`；A 无此限制。

**A 的真实残余风险（必须写进交付说明，别掩盖）**：

- **前端图与工作流锚腿可能不一致**：前端 `ValuationBandChart` 走**命令**（读表、带「3 天陈旧则重拉」判定），
  工作流锚腿走**工具**（读在线缓存、不落库）⇒ 表陈旧时两者窗口/`current` 可能不同。
  缓解：两者共用同一缓存与同一裁剪口径；但**残余风险不为零**，需在报告与代码注释里写明。
- **同名不同层**：`commands::stock_analysis::compute_valuation_band`（Tauri 命令）与
  工具名 `compute_valuation_band`（MCP 工具）**同名**。这正是 K 组「一名多义的判」的高危形态 ⇒
  两处都必须留注释指明彼此关系（谁能被工作流调、谁不能）。
- **待验（A 的前置）**：`AStockClient` 的缓存 TTL 与命令层「3 天陈旧」判定并不等价，
  若缓存 TTL 短于 3 天，工具路径会拿到比表更新的数据 —— **这不算错，但必须在同一处说明**。


### 6.14.5.2 已按 **A** 落地（2026-09-21 用户裁决后）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/astock-data/src/mcp_tools.rs` | 新增**工具** `compute_valuation_band`：`stock_mcp_tools()` schema + `execute_mcp_tool` dispatch **两处成对** |
| 2 | `crates/astock-data/src/valuation_band.rs` | 新增 `impl FinancialSnapshotLike for ValuationSnapshot`、`since_date_from_years`、`clip_valuation_history`（**窗口口径单一来源**）+ 3 条回归测试 |
| 3 | `src/commands/stock_analysis.rs` | 命令层改调上述两个共享函数（删掉自己算 `since_date` 与自己 filter 的三行） |
| 4 | `src/commands/stock_analysis_setup/seed_consistency_tests.rs` | 抽取面扩到 `tool_node(...)` 的**位置参数**形态（仅字面量） |
| 5 | `src/commands/stock_analysis_setup/seed_stock_analysis.rs` | **只改注释**（原注释误称"调命令层"，并补断链机制说明） |

**实施中新增的两个陷阱（都不在原方案里，且**都会静默出错、都不报错**）**：

1. ⚠ **「年数入参」不足以定窗口**：`get_valuation_history(stock_code, years)` 的分页是
   `max_pages = want/PAGE_SIZE + 2`（`vendors/eastmoney.rs:707-710`）⇒ **会多取约 2 页 ≈ 2 年**。
   只"调同一个 vendor 函数"而不裁剪，工具路径的窗口会比声明值宽近一倍 ⇒ 与前端图分位不一致。
   这就是把 `clip_valuation_history` 提成共享函数的原因。
2. ⚠ **（更隐蔽）两侧原始顺序相反，而 `current` 的约定是 `last()`**：
   vendor 接口按 `TRADE_DATE` **降序**返回，且 `AStockClient::get_valuation_history`
   **原样返回不重排**（`eastmoney.rs:711` / `lib.rs:2295-2296`）；
   而命令层读表是 `.order_by_asc(SnapshotDate)`（**升序**）。
   ⇒ 工具路径若直接写 `snaps.last()`，会取到窗口内**最旧**那天的 PE 当"当前 PE"。
   **分位数本身与顺序无关**（`metric_band_from` 内部排序），受影响的**只有"当前分位"**
   ⇒ 表现像"数据不对"，极易被误当成上游数据问题。
   **修法**：把排序归一收进 `clip_valuation_history`（输出恒为升序），
   并在工具、命令两处各留一句"`last()` = 最新，靠谁保证"；
   另加 `clip_normalizes_descending_input_to_ascending` **反向对照**测试
   （输入降序，断言输出升序且 `last()` 是最新）。

**不需要升 `TEMPLATE_VERSION`**：本次改的是"工具是否注册"，模板的节点/边/变量**一字未改**
（`t-valuation-band` 节点、它的两条边、两个注入变量的名字与来源全部保持原样），
故常量仍为 **68**、版本门 `65 < 68` 依然放行。

**仍未解决的前置**：DB 实测仍 `version=65 / node_band=false / 360 vars·62 nodes·153 edges`
（2026-09-21 13:0x 核对）⇒ `tauri dev` 的 watcher **没有**在本次源码改动后重启 app。
**§6.13 的「用户重启才重种」仍然成立**；先前推测的「watcher 自动重启 ⇒ 自动重种」**未被证实**。

**⚠ 连带更正：§6.12.3 的「影响面」结论在新数据路径下**不再适用**。**
§6.12.3 是按**本机 `financial_snapshots` 表**（当时全库只有 **13 只**）估的覆盖面，结论是
「④ 只在 601698 上从无到有」。但工具路径是**在线取数**（12h 缓存），
覆盖面 = **东财有历史估值数据的全部标的**（实测单一标的可回溯 8 年+，`eastmoney.rs:695`）
⇒ **不再受「13 只」限制**，④ 的真实影响面**比原估更大**。
本节只登记这条口径变化，**不给新数字** —— 要重跑一次样本对账才能说（待授权）。

### 6.14.6 门禁抽取面：扩了一条，另一条**明确不修并登记**

**已落地**：`seed_tool_def_names` 现在同时抽取 `tool_node(...)` 的第 3 个**位置实参**（仅字面量），
新增函数 `tool_node_positional_tool_names`（逐字符括号平衡 + 顶层逗号切分；第 3 项是**变量**时跳过，
对应通用包装函数里的 `tool_node(id, title, tool_name, …)`）。
实测 `seed_stock_analysis.rs` 仅 **5 处** `tool_node(`：2 处变量（跳过）+ 3 处字面量
（`compute_valuation_band` / `get_stock_dragon_tiger` / `get_stock_institutional_visits`，**全部可解析**）
⇒ 扩面后**不会**产生假红；`seed_serenity.rs` 该形态 0 处，不受影响。

**明确不修、只登记**：`tool_assignments`（`&[(&str,&str,&str,&str)]`）与
`algo_tools`（`&[AlgoToolRow]`）两张表以**变量**把工具名传给 `tool_node`
（`for (tool_id, title, tool_name, …) in tool_assignments`）⇒ 文本级抽取**看不见**它们的名字。

- 试过的"任意元组第 3 元素"启发式**不可用**：它需要精确定位表边界，而这两个 seed 文件里
  `= &[` 开头的表有 **14 处**（`EMBEDDED_PROMPTS` / `PROFILE_TOOLS` / `RENAME_MAP` /
  `seed_serenity` 的 3 张…）。边界一判错就**溢出到无关元组** —— 实测把
  `research-manager` / `trader` / `rule-checker`（这些是 **agent profile id**，不是工具名）
  乃至 `tracing` 宏的参数都当成了工具名 ⇒ 会制造**大量假红**，比不扩更糟。
- 真正的修法有两条，都要额外面（**未授权，本轮不动**）：
  ① 在 seed 模块暴露**纯函数**（如 `declared_tool_node_names() -> Vec<&'static str>`）当门禁输入面
     —— 单点、无解析；代价是两张表要从函数内提到模块级
     （现为 `seed_stock_analysis_workflow_template` 内的**局部变量**）；
  ② 让 `seed_stock_analysis_workflow_template` 可被测试调用后读**产物节点** —— 但它现在是
     `pub(crate) async` 且需要状态，改造成本更高。
- **风险量级**：这 14 个工具若失联，后果与 §6.14 同型（`degraded` 静默吞掉），
  但它们**既有且长期在用**，不属本批引入。本批只把「新增节点最可能用的那种形态」
  （位置参数）纳入监测 —— **不能**宣称"同类盲区已消除"。

### 6.14.7 补一个自证缺口：新抽取面**原本没有正负对照自测**（同批已补）

扩面落地后，`seed_stock_analysis_tools_all_resolvable` **绿了**。但这条绿**不能**证明
"抽取面这次真的看见了这个节点" —— 抽取器少抽一个名字，下游断言**照样绿**
（只是少检查一项）。这与 §6.14.3 的假绿**同族**，只是方向相反（上次是漏抽导致漏检、
这次可能仍是漏抽却"恰好"名字已在解析空间里）。

而本文件里**另外两个同类抽取器都自带对照**：
`round_literal_scanner_detects_known_bad_forms`、`profile_tools_resolvable_scanner_discriminates`。
新抽取面**没有** —— 属"判据工具缺自证"（判据 #643 / #655 的同族）。

**同批已补** `tool_node_positional_scanner_discriminates`：# 正控①（位置参数字面量，含
嵌套括号/字符串内逗号括号的干扰）、正控②（变量形态与字面量**混排**）、
负控①（第 3 项是变量 ⇒ 必须跳过，这是防假红的守门人）、负控②（实参不足 3 个 ⇒ 不产出不 panic），
**外加端到端断言：直接在真 `seed_stock_analysis.rs` 上断言 `compute_valuation_band` 被这条规则看到**。

预检（`output/tmp-scanner-replica.mjs`，逐行复刻 Rust 实现对真源跑）结果：

```
✅ 正控① 位置参数字面量  实际=["tool_alpha","tool_beta"]
✅ 正控② 混排            实际=["tool_gamma"]
✅ 负控① 变量形态必须跳过 实际=[]
✅ 负控② 实参不足         实际=[]
真 seed 抽取到 3 个位置参数字面量工具名: compute_valuation_band / get_stock_dragon_tiger
                                        / get_stock_institutional_visits
✅ 端到端断言（须含 compute_valuation_band）    ✅ 反向对照（不得含变量名）
```

**这组数字本身就是本节的收口证据**：真 seed 里用位置参数声明的工具名恰好 3 个，
其中 **`compute_valuation_band`** 与 **`get_stock_institutional_visits`** 此前
**完全躲过**这条门禁（后者在"全域 registry"里所以侥幸绿；前者不在 ⇒ 就是 §6.14 的缺陷）。
⇒ 扩面**不是**形式主义：它把 2 个此前不可见的声明纳入监测。

---

## 6.15 编译门禁实测：一个构建基础设施失败，与被它掩盖的绿

### 6.15.1 四个数据点（同两条命令 × 两个 target 目录）

| 命令 | target 目录 | 结果 | 耗时 | 逐字证据 |
|---|---|---|---|---|
| `check -p axagent-astock-data --all-targets` | 共享 `target/` | ✅ `CHECK1_EXIT=0` | 18m41s | `Checking axagent-astock-data v0.1.0` |
| 同上 | `target-v79` | ✅ `ISO_EXIT=0` | 23m31s | 同上 |
| `check -p axagent --all-targets` | 共享 `target/` | ✅ `CHECK2_EXIT=0` | 16m58s | `Compiling axagent v2.9.9` + 30 条 `Checking axagent-*` + **零 error / 零 warning** |
| 同上 | `target-v79` | ❌ **失败于依赖** | — | `Checking axagent v2` 在日志中命中 **0** 次 |

**通过的那条是有效绿**：`tmp-v79-check.log` 里从 `axagent-entities` 一路到 `axagent-runtime` 共 30 个 crate 逐个 `Checking`，
`Finished dev profile`，且**全文零 `error:` / 零 `warning:`**（`--all-targets` 覆盖 `seed_stock_analysis.rs`
与新增门禁所在的 `seed_consistency_tests.rs`）。

### 6.15.2 归因：为什么判定「不是代码问题」

`target-v79` 的失败原文（`tmp-v79-check-agent.log`）：

```
error: failed to run custom build command for `zstd-sys v2.0.16+zstd.1.5.7`
  process didn't exit successfully: `…\target-v79\debug\build\zstd-sys-…\build-script-build` (exit code: 1)
  --- stderr          ← 空
  error occurred in cc-rs: command did not execute successfully (status code exit code: 2):
      "C:\…\MSVC\14.50.35717\bin\HostX64\x64\cl.exe" … "-c" "zstd/lib/legacy\zstd_v06.c"
```

**三条同时成立才能判「基础设施」**（缺一条就得继续怀疑代码）：

1. **同一条命令在另一个 target 目录绿**，且该目录里**同一个 crate 早已编好**（共享 target 的 `zstd-sys` 是热的）；
2. **失败点在依赖的 C 编译**（cc-rs 调 `cl.exe`），且该 crate 与本次改动**毫无关系**（本轮没碰 `zstd-sys`/`aws-lc-sys`、没动 feature、没动依赖）；
3. **失败日志中从未出现 `Checking <被测 crate>`** —— 目标 crate 根本没被检查过，它的状态是「未测」，不是「失败」。

⇒ 归因 **构建基础设施**，不是代码。⚠ **反向防呆**：这三条**不能**反过来用成「代码没问题」的证据 ——
代码无错的证据只来自 6.15.1 里那条真的逐 crate 跑完并零告警的日志。

**直接原因**：失败时**两套构建在并发编 C 依赖**（我的 `target-v79` + 用户的 `tauri dev`），
失败点之前已有 **293 条 `Compiling`**；`cl.exe` exit 2 且 `stderr` 完全为空，是 Windows 上
「进程被 OS 终止 / 资源耗尽」的形态，不是源码错误形态。附带一条取证障碍：
`cl` 的诊断被 **GBK→UTF-8 错读**成 `������ warning D9002 :����δ֪ѡ�`，
可见的只有 `-fvisibility=hidden` 这类 GCC 选项产生的 D9002 警告（非致命）。
**新 target 目录不是免费的绕锁手段** —— 它要重编全部 C 依赖（`aws-lc-sys` 600+ `.o`、`zstd-sys`），
在高并发下正是 C 编译器失败的放大器。绕锁前先看共享 target 是否已热。

### 6.15.3 一处误判记录：`build failed, waiting for other jobs to finish` ≠ 卡死

杀进程**之前**它的状态是：打完 `warning: build failed, waiting for other jobs to finish...` 后
**日志 110 秒不再增长**，且**下一条命令的日志文件根本没被创建**。
我第一遍的判读差点写成「队列死了」。

真身由**进程亲缘树**给出：

```
bash(36892) → bash(36144) → bash(9648) → cargo(36368) → cargo(25588)
                                              ├→ build-script-main(20600)  [target-v79\…\aws-lc-sys-…]
                                              └→ cl.exe × 19               [全部 target-v79]
```

⇒ cargo 在等**已派发**的 job 收尾（`aws-lc-sys` 还在继续追加 `cl.exe`：取值时 16 个，杀之前已 19 个），
期间**不会创建下一条命令的日志**。**判定队列存活要看亲缘树能否回到自己的 shell，不看日志是否增长。**
（对照记忆 #7 的「cargo 看起来卡死 ≠ 卡死」：那条讲 `build/*/out` 的 mtime 推进；
这条讲「失败后收尾期」的第三种假死形态。）

### 6.15.4 环境事实（对交付有直接影响）

进程表显示 `cargo run --no-default-features --features notification,computer-use`（= `npm run tauri dev`）
**正在运行**，其 `rustc axagent_analysis_engine` 子进程在写共享 `target/`。两个后果：

1. 用共享 target 的门禁会与它抢锁 —— `Blocking waiting for file lock on build directory` 是**正常等待**，
   不是失败。本轮 `check` 的两次抢锁（`tmp-v79-check.log` 第 1、6 行）都等到了并成功。
2. **若改动 Rust 源码，它的 watcher 会自动重编并重启 app** —— 而重种在启动期执行，
   **因此它本身可能就是「落库前置」（§6.13）的解法**。
   ⚠ 本轮**未验证** watcher 是否真的监听 `src-tauri/**` 并触发重启，故仅登记为**待验**，不作交付结论。

### 6.15.5 一条工具纪律：日志命名与 EXIT 判定

本轮的 `tmp-v79-check.log`（共享 target，`EXIT=0`）与 `tmp-v79-check-agent.log`（target-v79，failed）
**同前缀、结论完全相反**；而我第一遍扫描用 `^[A-Z_]*EXIT` 做 EXIT 判定，
该正则**不含数字** ⇒ 漏掉 `CHECK2_EXIT=0`，把已通过的它报成「进行中」。
⇒ ① 一次任务一个**唯一前缀**，并在一处登记「任务 ID → 日志文件」映射（本轮后半改用 `tmp-g-*`）；
② EXIT 判定用宽松式 `/EXIT[^=]*=\s*-?\d+/`，**不要把判定写成「有没有匹配某个特定拼写」**。

**更正（同一坑，本轮第二次踩）**：上一条我原本写的是 `/EXIT\s*=\s*-?\d+/` —— 它**仍然错**：
`EXIT\s*=` 要求 `EXIT` 后直接是空白或 `=`，而本批的实测行是 `EXIT_I1=0`（**中间有下划线+编号**）
⇒ 6 条日志**全部**被判成「[无 EXIT]」，靠读末行字符串才没误判。
⇒ 判据的**口径**必须写成 `[^=]*`，因为"我起的变量名里会带什么"是**会变的**。
同族 R 组「判据工具自证」、#658。

---

## 6.16 本批最终门禁矩阵（含一条**我自己引入的 clippy 红**）

| 门 | 命令 | 结果 |
|---|---|---|
| 格式/语法 | `cargo fmt --all -- --check` | ✅ 空输出 `EXIT=0`（兼语法门） |
| 类型 | `cargo check -p axagent-astock-data --all-targets` | ✅ `EXIT=0`（逐字 `Checking axagent-astock-data`） |
| 类型 | `cargo check -p axagent --all-targets` | ✅ `EXIT=0`（30+ crate 逐个 `Checking`，零 error/warning） |
| 测试 | `cargo test -p axagent-astock-data --lib` | ✅ **334 passed / 0 failed**（含本批新增 `clip_drops_empty_dates_and_out_of_window`、`clip_normalizes_descending_input_to_ascending`、`since_date_from_years_keeps_zero_year_semantics`） |
| 测试 | `cargo test -p axagent --lib seed_consistency` | ✅ **18 passed / 0 failed**（I 队列当时 17；L3 终轮 18 = 补入 §6.14.7 的 `tool_node_positional_scanner_discriminates`） |
| 测试 | `cargo test -p axagent --lib valuation` | ✅ 1 passed（`valuation_leg_weights_degrade_to_legacy`） |
| 前端类型 | `tsc --noEmit` | ✅ `EXIT=0` |
| 前端格式 | `dprint check` | ✅ 0 违规（顺手修 3 个未格式化文件，其中 `ValueAssessmentPanel.tsx` **不属本批**） |
| 注入覆盖 | `scripts/audit-inject-coverage.mjs --strict` | ✅ 差集空、悬空映射空 |
| **clippy** | `clippy -p axagent -p axagent-astock-data --all-targets --all-features -- -D warnings` | ✅ **零 warning**（**分头取证**：`axagent` 由 L1，`axagent-astock-data` 由 L5 单独补跑，见 §6.18） |

> **口径说明**：上表除 clippy 行外均为 **I 队列**（`3wDdCx`，`output/tmp-i-*`）的实测值。
> 之后代码又动过两次：① clippy 的 `doc_lazy_continuation` 修复；② 补抽取面自测（§6.14.7）；
> ③ seed 头注释的计数订正（§6.16.1）。中途列的 K 队列（`gQ3uLG`）**在 build 锁上等了 14m33s
> 一行未编**，按纪律止损；最终以 **L 队列**（`r6BJ4a`，`output/tmp-l-*`）为准。
> **若 L 队列未过，上表作废、以 L 为准。**
> **【收口】L 队列（`r6BJ4a`，`output/tmp-l-*`）四项全绿**（L1 clippy `EXIT=0` / L2 334 / L3 18 / L4 1）。
> 但收口时发现 clippy 那一行**证据不足**：L1 日志里没有 `Compiling axagent-astock-data`
> ⇒ 追加 **L5**（`E9qSfh`）对 `axagent-astock-data` 单独补跑，`EXIT_L5=0`、零 warning。**详见 §6.18。**

### 6.16.1 seed 头注释的三处「数值腐烂」（J 组，同批订正）

落库核对后回看 `seed_stock_analysis.rs` 的 V68 批次头注释，发现**三处与事实不符**
（本批唯一一处"报告以外"的自查发现）：

| 原文 | 实测 | 处置 |
|---|---|---|
| 「本批是三处 `.rhai` + 三处 `input_mapping`」 | 本批改动集（备份 `output/backup-2026-09-21-decisions4/`）= **2 个 rhai**（`portfolio-mgr.rhai`/`data-quality.rhai`）+ **2 条新映射**；「第三个」在源文件与 DB 两侧都无对应物 | 改为 2 / 2，并留订正痕 |
| 「新注入变量 **3 个**」（只列了 2 个） | DB 实测 `nodes` 文本内仅 2 个新名字 | 改为 **2 条**，并写明它们在**节点级 `input_mapping`** |
| 「本轮**未**核对 DB 现值（无 DB 连接）」 | 已核对：重种前 65（05:36:56）→ 重种后 **68**（13:25:44） | 改为「已核对」+ 推翻"watcher 不会重启"的旧推测 |

**通用教训**：「本轮新增 N 处」这类计数**没有任何门禁守着**，是本仓最先腐烂的注释类型
（J 组：多入口文档必有一份撒谎，**数值先腐烂**）。⇒ 计数要么写成可复核的判据，
要么**不写具体数字**只写"清单见下"。

### 6.16.2 那条 clippy 红：`doc_lazy_continuation`（我写的文档注释）

```
error: doc list item without indentation
  --> crates\astock-data\src\valuation_band.rs:209:5
   = note: `-D clippy::doc-lazy-continuation` implied by `-D warnings`
```

根因是 `clip_valuation_history` 的文档注释结构：我在**嵌套项目符号**（`///    - vendor（…）`）
之后写了三条**结论行**，缩进只到 4 格 ⇒ 被 CommonMark 当作上一条 bullet 的
**lazy continuation**（缩进不足），clippy 按 `-D warnings` 直接报错。

**修法不是"再加两个空格"**（那是让文档结构服从解析器），而是**消掉歧义**：
把嵌套 bullet 摊平成普通续行 —— 这样"缩进等级"这个易错量**不存在**了。
⇒ 通用教训：文档注释里**能不嵌套列表就不嵌套**；嵌套列表 + 后续结论段是最容易
被 Markdown 规范与 clippy 一起咬的结构。

**值得单独记的一点**：`cargo check` 与 `cargo test` **全绿**时这条 `-D warnings` 红才出现 ——
因为 `doc_lazy_continuation` 属 **clippy** 而非 rustc lints。
⇒ "check 绿 + test 绿 + fmt 绿"**不蕴含** clippy 绿（判据 #72 家族：编译通过 ≠ 门禁通过）。

---

## 6.17 端到端落库核实（**本批唯一的端到端证据，2026-09-21 13:25:44 重种后**）

重种由 `tauri dev` 的 watcher 因源码改动触发（**先前"watcher 不会重启 ⇒ 必须用户手动重启"的推测被推翻**）。
`output/tmp-v79-postseed-verify2.sql`（只读，含正控）：

### 6.17.1 新节点本体（与 4 个已工作的同类节点**逐字同形**）

```json
{ "id": "t-valuation-band", "type": "tool", "title": "估值分位带",
  "config": { "tool_name": "compute_valuation_band",
              "output_var": "t-valuation-band",
              "input_mapping": { "stock_code": "stock_code" } } }
```

对照（证明形态不是"新编的"）：

| 节点 | config |
|---|---|
| `t-dragon-tiger-data` | `{"tool_name":"get_stock_dragon_tiger","output_var":"t-dragon-tiger-data","input_mapping":{"stock_code":"stock_code"}}` |
| `t-institutional-visits` | `{"tool_name":"get_stock_institutional_visits","output_var":"…","input_mapping":{"stock_code":"stock_code"}}` |
| `t-scoring` | `{"tool_name":"compute_scoring",…,"input_mapping":{"stock_code":"stock_code"}}` |
| `t-risk` | `{"tool_name":"compute_portfolio_risk",…,"input_mapping":{"stock_codes":"stock_code"}}` |
| `t-valuation` | `{"tool_name":"compute_valuation","input_mapping":{"stock_code":"stock_code","dcf_growth_rate":"value_dcf_growth_rate",…}}` |
| **`t-valuation-band`（新增）** | **同形 ✔** |

> 附带更正：我最初查 `config->>'output_key'` 得空并一度存疑 ——
> **真实字段名是 `output_var`**。这是一小时内我**第三次**"字段名/口径猜错"
> （`variables` vs `input_mapping`、`^[A-Z_]*EXIT`、`output_key` vs `output_var`）。
> ⇒ 只读核对的**第一步永远是先 dump 一个真元素的完整形态**，再写判据。

### 6.17.2 两条边与新映射

| 项 | 实测 |
|---|---|
| 入边 | `e-t-valuation-t-valuation-band`：`t-valuation → t-valuation-band` ✔ |
| 出边 | `e-t-valuation-band-portfolio-mgr`：`t-valuation-band → portfolio-mgr` ✔ |
| 新映射 ① | `portfolio-mgr.input_mapping` 含 `valuation_pe_percentile` ✔（4543 字符） |
| 新映射 ② | `data-quality.input_mapping` 含 `valuation_dcf_fcf_data_missing` ✔（1943 字符） |
| 正控 | `valuation_dcf_upside`（必存在的旧映射名）同口径 = `t` ✔ |

### 6.17.2b 落库的 **rhai 脚本**已含本批全部标记（①②③④ 的库侧证据）

在 `workflow_templates.nodes::text`（即**落库模板**，非源文件）上做标记核对：

```
has_fusion_wsum        = true      ← ④ 三腿归一化
has_dcf_anchor_decay   = true      ← ② 分腿衰减（新名）
has_band_leg_usable    = true      ← ④ 第三腿可用性
has_old_f5_fallback_decay = false  ← ✅ 反向对照：旧名已彻底消失
has_upstream_data_gaps = true      ← ③ 只告警不扣分
weights_in_db          = 0.56 | 0.24 | 0.20   ← 三者全在
```

⇒ 四项裁决**不只落在源文件里**，而是**经种子真正写进了运行时会读的那份模板**。
其中 `has_old_f5_fallback_decay = false` 是**反向对照**（证明这个判据有区分力、
不是恒真的），这一点必须与三项 `true` 一起读才有意义。

### 6.17.3 工具双表（`output/tmp-mcp-tables.mjs`，重跑）

```
compute_valuation_band           schema=true dispatch=true   ← 本批新增，修复前两处皆 false
get_stock_dragon_tiger           schema=true dispatch=true
get_stock_institutional_visits   schema=true dispatch=true
compute_valuation / compute_scoring / compute_portfolio_risk  全 true

dispatch 有、schema 无: []
schema 有、dispatch 无: 9 个（全部 dojo_* / sector_precomputed_*，走另一条 dojo 执行器，属既有正常形态）
```

### 6.17.4 结论与**诚实的剩余边界**

- ✅ 四项裁决 + A 修法**全部端到端落库**：版本 68 / 新节点 / 两条边 / 两个注入映射 /
  工具双表成对 / **落库 rhai 含全部标记且旧名已消失**。
- ⚠ **仍未做**：**运行期**证据 —— 即"跑一次真实的 stock-analysis，确认
  `t-valuation-band` 真的产出 `metricPe.currentPercentile`、且 `portfolio-mgr` 的
  `fused_sig` 真的按 0.56/0.24/0.20 融合"。现有证据到"接线正确"为止，
  **不含"跑通一次"**（需在 UI 上真跑一次分析，用户侧动作）。
- ⚠ `financial_snapshots` 的**回填语义**随之变化：此前只有用户点前端"估值带"图才回填
  （DB 实查覆盖 13 只）；现在每次跑分析都会对**该标的**回填 ⇒
  覆盖度会随使用增长，但**不会**一次性补齐全市场。

---

## 6.18 门禁终局：clippy 双包**零 warning**，及一条「日志缺行 = 该包零证据」的判据

### 6.18.1 L 队列四项（`r6BJ4a`，`output/tmp-l-*`）

| 步 | 命令 | 结果 |
|---|---|---|
| L1 | `clippy -p axagent -p axagent-astock-data --all-targets --all-features -- -D warnings` | ✅ `EXIT_L1=0`（`Finished in 16m57s`，**全程 0 条 error/warning**） |
| L2 | `test -p axagent-astock-data --lib` | ✅ **334 passed / 0 failed** |
| L3 | `test -p axagent --lib seed_consistency` | ✅ **18 passed / 0 failed** |
| L4 | `test -p axagent --lib valuation` | ✅ 1 passed |

L2 中本批新增的 3 条测试**逐条确认在执行**（不是"源码里写了、但没被 test harness 收集"）：

```
test valuation_band::tests::clip_normalizes_descending_input_to_ascending ... ok   ← 反向对照
test valuation_band::tests::since_date_from_years_keeps_zero_year_semantics ... ok
test valuation_band::tests::clip_drops_empty_dates_and_out_of_window ... ok
```

### 6.18.2 疑点：clippy 日志里**没有我改过的那个包**

L1 日志全文只有 5 行：

```
    Blocking waiting for file lock on build directory
   Compiling axagent v2.9.9 (D:\OneManager\AxInvest\src-tauri)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 16m 57s
EXIT_L1=0
```

本批改动落在**两个** crate（`axagent` + `axagent-astock-data`），日志里却只有 `Compiling axagent`
⇒ cargo 把 `axagent-astock-data` 判为 **Fresh**。而 Fresh 的语义是**复用既有产物**：
此刻 build 目录里同时有 `tauri dev` 的 `cargo run`（**check 口径**）在写。
**若 `check` 与 `clippy` 共用 fingerprint，那 L1 对这个包就一个字都没检查** ——
即"命令里列了 `-p axagent-astock-data`"≠"它被 clippy 看过"。

### 6.18.3 零成本判据失败：fingerprint 目录 mtime **不是**有效判据

| 量 | 值 |
|---|---|
| `crates/astock-data/src/valuation_band.rs` mtimeMs | `1789970755601` |
| `crates/astock-data/src/mcp_tools.rs` mtimeMs | `1789967408560` |
| `.fingerprint/axagent-astock-data-f272d78f582a2052`（lib）mtimeMs | `1789971555087`（唯一晚于源文件者） |

单看像是在"改动之后"，但 **L5 实测把这条判据否掉了**：L5 的 `-v` 输出写明它真的调用了
`clippy-driver … --crate-name axagent_astock_data … --emit=dep-info,metadata`
（产物 `-C extra-filename=-edf86219f8b89988`，对应 fingerprint 目录 `…-edf86219f8b89988`），
而**该目录 mtime 是 `1789910539959`** —— 比源文件改动**早 1 小时 40 分**，
**没有随本轮重编而更新**。

⇒ fingerprint 目录 mtime 会滞后/不更新，用它推断"某包是否刚被编过"会得到**假阴性**。
与用户级记忆里 `[datetime]::Parse("…Z")` 那条坑同族：**先怀疑比较的基准量，再怀疑被测对象**。

### 6.18.4 定性的办法：让 cargo **自己**说（加 `-v`）

L5 的关键输出（`output/tmp-l5-clippy-astock.log`）：

```
     Checking axagent-astock-data v0.1.0 (D:\OneManager\AxInvest\src-tauri\crates\astock-data)
     Running `…\clippy-driver.exe … --crate-name axagent_astock_data … --crate-type lib --emit=…metadata …`
     Running `…\clippy-driver.exe … --crate-name axagent_astock_data … --test …`
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 46.85s
EXIT_L5=0
```

`--crate-name axagent_astock_data` 出现**两次**（lib 口径 + `--test` 口径）⇒ 本包两个 target
都真的过了 clippy-driver；`EXIT_L5=0` 且全日志零 `warning:` ⇒
**这个包的 clippy 绿是实测的，不是继承来的**。

### 6.18.5 新判据（L 组：门禁扩容 / 文档盲区）

> **clippy 门禁日志里没有**自己那个包的 **`Compiling`/`Checking` 行 ⇒ 该包本轮零证据，不得记作"通过"。**
> 只会 `Compiling axagent` 时，`-p axagent-astock-data` 只是**被列进了命令行**，不代表它被看过。
> **自证手段 = 加 `-v`**，断言输出里出现 `Running … clippy-driver … --crate-name <你的包> …`。
>
> 机理：`cargo clippy` 对 **workspace 成员**走 `RUSTC_WORKSPACE_WRAPPER`（第三方依赖走普通 rustc），
> 而该变量**不计入 fingerprint** ⇒ clippy 与 `check` 可能共享 fingerprint ⇒
> **`tauri dev`（`cargo run`/`cargo check`）在跑时，clippy 的 Fresh 判定可能来自一次 `check`。**
> 这是 L1 出现 Fresh 的机理，也是"并发工具污染门禁结论"的一个具体形态。

### 6.18.6 终局

| 项 | 状态 |
|---|---|
| fmt / check(×2) / test(×3) / tsc / dprint / inject-coverage | ✅ 全绿（§6.16 矩阵，L 队列终值） |
| clippy `-D warnings` — `axagent` | ✅ L1 实测（逐行在册） |
| clippy `-D warnings` — `axagent-astock-data` | ✅ **L5 实测**（`clippy-driver` 两次调用在册） |
| 本批 clippy 红是否全清 | ✅ `doc_lazy_continuation`（§6.16.2）是唯一一处，已修且 L1/L5 复跑零 warning |
| 端到端落库 | ✅ §6.17（v68 重种后） |

**门槛式结论**：本批**代码侧与门禁侧已闭合**；**唯一未闭合项仍是运行期证据**（§6.17.4），
需在 UI 真跑一次 stock-analysis 才能给出。

---

## 七、复现与验证命令

```bash
# ── DB 跨样本取证 ──
# ⚠ 凭据纪律：**口令不进 argv**（`psql "postgres://user:pw@…"` 会把口令写进进程表）。
#   正确形态 = `PGPASSWORD` 环境变量 + `-w`（never prompt，否则非 tty 下会永久挂起）。
PGPASSWORD='***' "C:/Program Files/PostgreSQL/18/bin/psql.exe" -w \
  -h localhost -p 5432 -U postgres -d axinvest -f output/tmp-dcf-audit.sql

# ── 落库前置：真库版本核对（只读）★ 本批交付硬前置 ──
# 版本门是 `existing.version >= TEMPLATE_VERSION ⇒ 跳过重建`，
# 所以「本批会不会生效」只由这一个数字决定，四道 cargo 门禁全部不回答它。
PGPASSWORD='***' "C:/Program Files/PostgreSQL/18/bin/psql.exe" -w \
  -h localhost -p 5432 -U postgres -d axinvest -f output/tmp-v79-postseed-verify.sql
# 读法：① version ② 新节点/边 ③ 新变量 ④ 规模 ⑤ 正控（必须全 t）⑤b 双口径

# 上游接口手验
curl -s "https://emweb.securities.eastmoney.com/PC_HSF10/NewFinanceAnalysis/ZYZBAjaxNew?type=1&code=SZ300308" | head -c 300
curl -s "https://emweb.securities.eastmoney.com/PC_HSF10/NewFinanceAnalysis/xjllbAjaxNew?companyType=4&reportDateType=0&reportType=1&dates=2026-06-30,2025-12-31,2025-06-30&code=SZ300308"

# 门禁
cd src-tauri
__TAURI_WORKSPACE__=true cargo test  -p axagent-astock-data --lib
__TAURI_WORKSPACE__=true cargo check -p axagent-astock-data --all-targets
__TAURI_WORKSPACE__=true cargo check -p axagent --all-targets
__TAURI_WORKSPACE__=true cargo test  -p axagent --lib dcf_availability_signal_layers_agree
__TAURI_WORKSPACE__=true cargo clippy -p axagent --all-targets --all-features -- -D warnings
cargo fmt -p axagent -- --check   # 锁无关，可先跑（且能当语法门：解析失败即报错）

# ⚠ 并发会话持有 build 锁时（症状：`Blocking waiting for file lock on build directory`）：
#   改用**独立 target 目录** ⇒ 零锁，代价是整条链重编一次。
#   判「真的在推进」只认 `target-v79/debug/build/*/out` 最新 mtime 在动，
#   不认「日志末行 / rustc 进程数 / deps 目录 mtime」——三者都会骗人（choke 点 = aws-lc-sys）。
__TAURI_WORKSPACE__=true CARGO_TARGET_DIR=target-v79 cargo check -p axagent-astock-data --all-targets
node output/tmp-v79-progress.mjs src-tauri/target-v79 output/tmp-v79-check-iso.log   # 判活探针

# 零锁 Rhai 语法预检（cargo 锁被占时也能跑；toolchain 必须从 rust-toolchain.toml 读，见判据 #650）
node output/tmp-rhai-syntax-run.mjs
```

**回滚点**：

| 批次 | 备份目录 | 内容 |
|---|---|---|
| 本批（V79 四项裁决） | **`output/backup-2026-09-21-decisions4/`** | **6 个文件 + `SHA256.txt`（逐文件核过）** |
| 前序（DCF 锚定取证） | `output/backup-2026-09-21-dcf-anchor/` | 4 个文件，改动前 sha256 已核 |

⚠ 备份纪律（用户级铁律）：备份后必须**立刻验证「备份副本 ≠ 工作区」**（逐文件 sha256）——
若全部相等，说明取样发生在改动之后，**等于没有回滚点**，须重做。

---

## 八、同轮自查追加：「两层 prompt 给出相反处置」——**这是我自己引入的缺陷**

> 记录在案的原因：它属于**本轮新增的门禁**所守的那一类，且**集合级一致性检查抓不到**。

### 8.1 缺陷本体

DCF 可用性判据被写在**两层** prompt 里，运行时**拼接后一起注入**，LLM 两层都读：

| 层 | 载体 | 落库路径 |
|---|---|---|
| 层① 专家 prompt | `agency_experts/stock-analysis/custom/value-investor.md` | `mod.rs` 的 `EMBEDDED_PROMPTS` 经 `include_str!` → `seed_agency_experts`（**无条件 UPSERT**，由 `ensure_stock_analysis_experts_seeded` 在**每次跑工作流前**调用，**无版本门**） |
| 层② 节点任务指令 | `seed_stock_analysis.rs` 里 value-investor 的 inline `system_prompt` | 经 `TEMPLATE_VERSION` 门写进 `workflow_templates.nodes` |

本轮我给层① 加了「`is_fallback_anchor != true` 才算可用，否则填 `null`」，
给层② 加了「锚定口径披露：数值**可以引用**」⇒ **同一个信号，两层给出相反处置**。

**权威口径在代码里**（`mcp_tools.rs` 的 `applicable` 计算处原文）：

> 当期 FCF 数据缺失：**不**判不适用（缺数据 ≠ 模型不成立），
> 由 fallback 锚定 + `is_fallback_anchor` 承担置信度衰减。

⇒ `applicable == false` = **硬不可用**（填 null）；`is_fallback_anchor == true` = **软衰减**（照常填 + 披露 + 下调 confidence）。
即：**层① 是我写错了，层② 是对的**。

### 8.2 为什么普通的「一致性检查」抓不到

两层**都提到**了 `is_fallback_anchor`，所以任何形如
「两边提到的标识符集合相等」的断言都会**全绿**。
必须断言的是**信号与其处置分档词的共现位置** —— 该信号只能落在「软衰减」段，不得落在「硬不可用」段。

### 8.3 修法

- 层① 把「三条件」改成**两类**：`【硬不可用】`（①②，填 null）/ `【软衰减】`（③，可引用但须披露）
- 层② 同步改为同一分档，并显式写明「与情形 ③ 同时命中时**按硬不可用处理**」
- 两层统一使用**带 `【】` 的唯一分段标记**（见 8.4 的坑）

### 8.4 新门禁：`dcf_availability_signal_layers_agree`

位置：`src-tauri/src/commands/stock_analysis_setup/seed_consistency_tests.rs`
（该文件已有「md 工具名引用 ↔ 解析空间」的链③检查，本条是**链④：两层 prompt 的处置一致性**）

断言：对**两层**分别切出硬段/软段，然后

| 断言 | 方向 |
|---|---|
| `is_fallback_anchor` **不在**硬段 | 软信号不得被降级 |
| `is_fallback_anchor` **在**软段 | 软信号不得被遗漏 |
| `applicable` **在**硬段 | **反向对照**：证明「硬段里什么都看不见」的假绿不成立 |
| `applicable == false` **不在**软段 | 硬信号不得被降级 |

**⚠ 踩到的坑（同类工具必须照抄这条）**：最初我用**裸词** `硬不可用` / `软衰减` 当分段标记，
但两层的**字段说明行**里也会提到这两个词（「DCF **硬不可用**时填 null；**软衰减**时照常填」），
`find(裸词)` 把分段点切到那一行 ⇒ 硬段退化成一行、软段吞掉整篇 ⇒
断言在**真文件上假红**（硬段里找不到 `applicable`）。
⇒ 分段标记必须是**带括号的唯一 token**，且测试内要**断言其出现次数恰为 1**（多于 1 次则分段点不确定）。

**区分力验证**（锁无关，用 Node 复刻同一套断言逻辑实跑）：

| 场景 | 结果 |
|---|---|
| ① 真实文件 | PASS |
| ② 负对照：软信号混入硬段 | FAIL（命中 `!hard.contains(fa)`） |
| ③ 负对照：软段丢掉信号名 | FAIL（命中 `soft.contains(fa)`） |
| ④ 负对照：分段标记重复 | 拒判（报「出现 2 次」） |

⇒ 真实文件 PASS 且三个负对照全被拦，断言**有区分力**（不是恒绿）。

### 8.5 可迁移教训

1. **同一语义写在两处 prompt 里 ⇒ 必漂移**。本仓已有 `seed_consistency_tests.rs` 这个专门的一致性门禁，
   但它当时只覆盖「标识符是否存在」，不覆盖「**同一信号是否得到同一处置**」——
   后者才是 LLM 会照着办事的东西。
2. **断言写完后必须在真文件上先跑一遍正例**（判据 #617）：本次若直接交 Rust 测试，
   会在「真文件」上报红，然后很容易被误判成「测试写得太严」而放宽 —— 实际是**分档点切错了**。
3. **两层的落库门不同**：层① 每次跑工作流前无条件 UPSERT（改 .md 立即生效）；
   层② 走 `TEMPLATE_VERSION`（DB 已是 65 ⇒ 常量须跳到 **66**，写 65 会被 `existing.version >= TEMPLATE_VERSION` 挡掉而静默不落库）。
