//! 股票分析专家/角色/Profile 自动种子化到 agency_experts/agent_roles/agent_profiles 表。
//! 使用 include_str! 编译期嵌入 .md 内容，打包后无需文件 I/O。

use super::{
    PROFILE_TOOLS, build_analyst_input_mapping, force_variable_value, merge_variable_values,
    resolve_debate_rounds,
};
use crate::commands::error_code::stock_setup;

/// portfolio-mgr 的「可调决策参数」清单 —— rhai 顶部 `present()` 守卫**实际读取**的
/// 28 个变量。**这是「可调参数」的单一权威源**，以下三处必须由它派生、不得各自维护：
///
///   1. portfolio-mgr 节点的 `input_mapping` 同名映射（本文件 seed 函数内）
///   2. 反思 prompt 的 `{{tunable_params_catalog}}` 清单（`stock_workflow/reflection.rs`）
///   3. 前端 `StockAnalysisConfigPanel` 的「决策参数」配置分组
///
/// 数组顺序 = **决策相关性顺序**（决策链上游优先）：市况先验 → 因子融合门 →
/// 行动阈值 → 仓位 → 风险仓位上限 → 风险分类阈值 → 交易成本。反思 LLM 收到的
/// 清单按此顺序渲染，注意力优先落在对 action 影响最直接的参数上。
/// （注：`input_mapping` 的 Rust 类型是 `HashMap<String, String>`，本身无序，
///  顺序仅对反思清单的线性渲染与前端分组有意义。）
///
/// 为什么要显式登记而不是按名字前缀扫全变量表：变量表还承载大量模型级参数
/// （`risk_free_rate` / `risk_hhi_*` / `risk_sharpe_annualization` 等），它们是
/// 估值与风险**计算**的输入，改了不会改变 portfolio-mgr 的决策边界，列给 LLM
/// 只会稀释注意力并诱发无意义建议。（v31 曾用前缀扫描，实测混入 41 项。）
///
/// 完备性判据（可复核）：本数组 = `portfolio-mgr.rhai` 中全部 `present(x)` 且 x
/// 属阈值型参数的集合，零遗漏零多余。审计方法：提取 rhai 的 `present()` 集合，
/// 减去「数据输入类」（来自 input_mapping 的 value 侧与运行时注入），差集应为空。
///
/// 新增可调参数时三处齐备才生效：① `seed_variables.rs` 定义变量；
/// ② `portfolio-mgr.rhai` 顶部加 `if present(x) { x } else { 默认 }` 守卫；
/// ③ 在此数组登记。
pub(crate) const PORTFOLIO_MGR_TUNABLE_PARAMS: [&str; 28] = [
    // ── 市况先验（决策起点：无个股证据时对上涨的基础概率，0-1）──
    "regime_prior_bull",
    "regime_prior_sideways",
    "regime_prior_bear",
    // ── 因子融合门（f7 交易员因子权重低于此值时，其看空信号不再封顶后验概率）──
    "trader_cap_min_weight",
    // ── action 决策阈值（后验概率 → 买入/增持/持有/观望/减持 的分档边界）──
    "action_buy_threshold",
    "action_increase_threshold",
    "action_hold_threshold",
    "action_watch_threshold",
    "action_reduce_threshold",
    // ── 仓位阈值（未封顶的凯利意愿仓位 → 买入/增持 的最小门槛，%）──
    //     2026-09-12 P0-11: 判定读 kelly_pos_uncapped（不施加 pos_cap_* 封顶），
    //     修正「极高风险上限 10 < pos_buy_min 15」的口径错配 —— 这是**一致性修复**：
    //     解除了该风险档下仓位门的算术不可达，但**不等于「买入」已可达**。
    //     实测 13/13 action 无变化，真因是证据链 `avg_signal` 恒 ≤0
    //     （`effective_posterior` 可达上限 0.5783 < `action_buy_threshold` 0.63）。
    //     另注：极高/高风险标的即便过了此门，仍会被 `pm_risk_veto` 按政策降级。
    "pos_buy_min",
    "pos_increase_min",
    // ── 风险等级仓位上限（按 overall_risk 约束 positionPct，不影响上述门槛）──
    "pos_cap_extreme",
    "pos_cap_high",
    "pos_cap_mid",
    // ── 风险分类阈值（财务/波动指标 → 风险等级，进而触发风控否决）──
    "risk_debt_extreme",
    "risk_vol_extreme",
    "risk_sharpe_extreme",
    "risk_vol_high",
    "risk_dd_high",
    "risk_roe_high",
    "risk_debt_high",
    "risk_vol_low",
    "risk_sharpe_low",
    "risk_dd_low",
    "risk_roe_low",
    "risk_debt_low",
    "risk_growth_low",
    // ── 交易成本（1-成本 折损，凯利仓位修正）──
    "cost_pct",
];

/// `algo_tools` 表行类型：(节点 id, 标题, 工具名, 参数名, 额外扁平映射, x, y)。
///
/// 抽为 type alias 是 `clippy::type_complexity` 的硬性要求 —— 七元组内含嵌套切片
/// （`&[(&str, &str)]`），内联书写在 `-D warnings` 下会直接编译失败。
/// 全部字段均为字符串字面量 / 常量，故统一用 `'static`。
type AlgoToolRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
    f64,
    f64,
);

/// `stock-analysis` 模板的 DB 版本号。
///
/// ## 取值判据（**不是**「比上一版 +1」）
///
/// 版本门是 `existing.version >= TEMPLATE_VERSION ⇒ return Ok(())`（`>=`，不是 `==`），
/// 所以常量必须**严格大于 DB 现值**才能放行重建。DB 现值可被别的路径写高
/// （并发会话 / 带更高常量的构建 / 历史遗留），**不能**按注释里的版本序列顺推。
///
/// 2026-09-20 实测（真库 + 快照表双重取证）：常量停留在 55，而 DB 已是 **58**
/// ⇒ **v53 / v54 / v55 三批改动全部未落库**，而 `cargo check / clippy / fmt / test`
/// 四道门全绿 —— 它们只回答「代码能否编译」，答不了「版本门会不会开门」。
/// 依据见 `workflow_template_versions` 快照表：最后一次成功落库是 2026-09-18 06:04:22
/// （写 `stock-analysis_v57` 快照 ⇒ DB 变 58），此后一次都没有。
///
/// 2026-09-20 二次升版至 **60**：v55 那批（Rhai 下沉）与 v60 这批
/// （降级兜底档保守化 + 节点 config 删 `modelRole`）需一并落库。
///
/// 2026-09-20 三次升版至 **61**：f13 瓶颈因子权重改走 `get_weight`
/// （`portfolio-mgr.rhai` + `regime-weights.rhai` 两个嵌入节点 `code` 同批改）。
/// ⚠️ 该批**未在真库核对现值**（当时无 DB 连接）—— 若真库 `version` 已 ≥ 61
/// （并发会话 / 更高常量的构建写过），本批仍会被门挡住。核验命令（只读）：
///   `SELECT id, name, version FROM workflow_templates WHERE id='stock-analysis';`
///
/// 2026-09-21 四次升版至 **64**：数据质量链修复 —— `data-quality.rhai`
/// （词表假阳性抑制 + expected_data 归因）与 `portfolio-mgr.rhai`
/// （data_gaps 内容级缺口判据）两个嵌入节点 `code` 同批改。
/// **本批已真库核对**（只读查询，2026-09-21）：`stock-analysis` 的 DB `version` = **62**，
/// 而上一批常量已写 63 却未落库（本轮运行产物 `template_version=62` 佐证）
/// ⇒ **v63 与 v64 两批会一并**在下次启动时落库。
///
/// 2026-09-21 五次升版至 **66**：估值链修复（value-investor 节点 prompt 扩
/// 「估值不可用」判据 + 锚定口径披露 + 分歧处理）。配套的 Rust 侧同批改动
/// （`astock-data`：现金流取数补齐、`ttm_fcf` 还原、`basis` 三态、
/// `value_signal` 负安全边际）**不随模板走**，随二进制生效。
/// **本批已真库核对**（只读查询，2026-09-21 05:5x）：`stock-analysis` 的 DB
/// `version` = **65**（`updated_at` = 1789940216663 ⇒ 2026-09-21 05:36:56），
/// 而文件常量当场是 **64** ⇒ **DB 已被别的路径写高**（`version_gate_tests`
/// 的注释把这种状态列为已知情形：「模拟 DB 现值被别的路径写高」）。
/// 这正好印证：**常量 ≤ DB 现值时本批会被版本门静默挡住**。
/// 故本次必须跳到 **66**（> 65）才能落库 —— 直接写 65 会被 `>=` 门挡掉。
/// 已核 v64 内容**确实在库**（`nodes` 含 `strip_verdict_blocks` /
/// `placeholder_occurrences` / `soft_marker_suppression` / `dt_seat_blank`）。
///
/// ## 2026-09-21 再升 66 → 67
///
/// 本轮新增 `valuation_graham_growth_clamped` 注入（格雷厄姆腿的增长率封顶标记，
/// 见 `input_mapping` 处注释）。与上次同理：**注入变量表是模板内容的一部分**，
/// 不升版本号则 DB 里的 v66 行永不更新 ⇒ 新变量在 Rhai 侧恒 `present() == false`
/// ⇒ 新加的护栏**静默不生效**。这类「改了但没落地」正是本项目已证的陷阱
/// （版本门是 `>=`，常量 ≤ DB 现值即被挡）。
///
/// ## 2026-09-21 再升 67 → 68（审计 §6.4 四项拍板同批）
///
/// 本批是**两处 `.rhai` + 两处新增 `input_mapping` 条目 + 一个新 ToolNode**，全部属模板内容。
///
/// ⚠️ 计数订正（2026-09-21 落库核对后）：原写「三处 `.rhai` + 三处 `input_mapping`」，
/// 实测为 **2 个 rhai 文件**（`portfolio-mgr.rhai` / `data-quality.rhai`）+
/// **2 条新增注入映射**（DB 实测 `nodes` 文本内仅出现这两个新名字，
/// 且模板级 `variables` 计数 360 **未变**——映射在**节点级** `config.input_mapping`，
/// 不是模板变量）。「三处」的第三个在源文件与 DB 两侧都找不到对应物 ⇒ 按实测改。
/// 教训同 J 组：「本轮新增 N 处」这类计数没有任何门禁守着，**最先腐烂**。
///
/// 1. `portfolio-mgr.rhai`：`moat_mult` 方向对称（看多 ×1.15 / 看空 ×0.85）、
///    `dcf_anchor_decay` 分腿（只压 DCF 腿）、新增第三条锚腿 `band_sig`
///    （权重 0.56 / 0.24 / 0.20，band 缺位时退化为改动前的 0.7 / 0.3）。
/// 2. `data-quality.rhai`：新增 `upstream_data_gaps`（**只告警不扣分**）。
/// 3. 新 ToolNode `t-valuation-band`（`compute_valuation_band`）+ 两条边
///    （`t-valuation → t-valuation-band → portfolio-mgr`）+
///    `raw-data` 的 aggregator description **未改**（刻意不把它并入 `algo_tools`，
///    故「16 个工具节点」的计数仍然成立）。
/// 4. 新增注入映射 **2 条**：`valuation_pe_percentile`（portfolio-mgr ←
///    `t-valuation-band.result.content.metricPe.currentPercentile`）、
///    `valuation_dcf_fcf_data_missing`（data-quality ←
///    `t-valuation.result.content.dcf.assumptions.fcf_data_missing`）。
///    二者都写在**节点级 `config.input_mapping`**，**不是**模板级 `variables`
///    ⇒ 重种后 `variables` 计数**不变**（实测 360）。别拿 `variables` 当核对字段。
///
/// ⚠️ 与上批同理：Rust 侧同批改动（`astock-data::DcfAssumptions` 新增
/// `fcf_data_missing` 布尔量、`commands/stock_analysis.rs` 未改）**不随模板走**。
/// ✅ **本批已真库核对**（只读，2026-09-21）：重种前 `version` = **65**
/// （`updated_at` = 2026-09-21 05:36:56，即 66/67 两批常量变更**从未落库**）；
/// `tauri dev` 的 watcher 因本批源码改动自动重编重启后
/// 重种**真实发生** ⇒ `version` = **68**、`updated_at` = 2026-09-21 13:25:44、
/// nodes 62 → **63**、edges 153 → **155**、新节点与两条边齐备。
/// ⚠️ 先前"watcher 不会自动重启 ⇒ 必须用户手动重启才重种"的推测**已被推翻**：
/// 实测它**会**重启（代价是 clippy/测试队列要跟它抢 build 锁，一次 15 分钟起）。
/// 只读核验：`SELECT version FROM workflow_templates WHERE id='stock-analysis';`
/// ⚠️ **v69（2026-09-21）**：`portfolio-mgr.rhai` 的 reasoning 结论名改走展示档
/// （详见函数内 v69 注释块）⇒ 本常量 68 → **69**，旧库存需重种才拿到新脚本。
/// ⚠️ **v70（2026-09-21）**：`portfolio-risk-gate.rhai` 覆盖 `action`/`positionPct` 时
/// 同步重算三个派生面（`positionState` / reasoning 结论名 / 止损止盈档）。
/// **必须独立升版**，不能与 v69 合并成一个号：v69 落库后 `tauri dev` 的 watcher
/// 会因源码改动自动重编重启并完成重种（见上文 v68 批的实测），
/// 此时旧库存已是 v69 ⇒ 再改 `portfolio-risk-gate.rhai` 若不升号，DB 里那份
/// 仍是未修复的旧脚本，改了等于没改。
/// ⚠️ **v72（2026-09-21）**：`a-lockup` 的**第二个前置 ToolNode** —— `t-pledge-data`
/// （调 MCP 工具 `get_stock_pledge_data`）。补的是**契约缺口**：
/// `lockup-watcher.md` 的方法论与自检要求分析「质押比例 > 50% 高警戒线 / 质押风险敞口」，
/// 但该文件的 `data_sources` 与 `PROFILE_TOOLS` 白名单**都不含质押工具**，其唯一上游
/// `t-lockup-data` 调的 `get_stock_lockup_bundle` 只聚合解禁 / 增减持 / 大宗交易**三方**。
/// ⇒ 每轮都在逼模型给一个**取不到的维度**编理由。全库实测：15 轮里 12 轮写了质押缺口，
/// 措辞从「质押数据缺失」（相对准确）漂移到「质押数据获取失败（工具调用被拒绝）」
/// 这种**伪归因**（详见 `AUDIT-pledge-attribution-2026-09-21.md`）。
/// 本版三处同时接线：① 本模板新增节点 + 两条边；② `PROFILE_TOOLS["lockup-watcher"]`
/// 授权 `get_stock_pledge_data`（LLM 侧可主动补取）；③ 专家 frontmatter 的
/// `data_sources` 补声明（与白名单同源，避免再次只改一侧）。
/// **必须升版**：节点与边写进 `workflow_templates.nodes|edges`，不升版旧库存照旧缺节点，
/// 改动静默失效。
/// ⚠️ **v73（2026-09-21）**：`data-quality.rhai` 新增**伪归因判据**（P2-1）+
/// `input_mapping` 新增 10 条 `{分析师}_tool_calls`。
/// 判据把「报告所说的工具被拒绝」与**真实调用记录**（`tool_calls_made` 的 `is_error`）
/// 交叉核对，两级结论：① 报告称工具被拒而本轮**零失败记录** ⇒ 疑似编造；
/// ② 有失败记录但报告**未点名被拒工具** ⇒ 归因含糊（无法确认影响哪个维度）。
/// 结论并入 `gap_reason`（复用既有展示通道，不新增字段 / 前端行 / i18n key）。
/// **必须升版**：`data-quality.rhai` 经 `include_str!` 嵌入本模板的 `data-quality`
/// 节点 `code` 字段，不升版 DB 里仍是旧脚本 —— 判据写了等于没写。
/// ⚠️ **v74（2026-09-21）**：**DCF 估值参数强制校正**（`force_variable_value` ×3）。
/// 三个参数早在 2026-09-12 就已校准（折现率 10→8.5、永续 3→4、默认增长 8→12，
/// 注释理由：「原 10% 偏高」「原 3% 偏低」「原 8% 偏保守，系统性低估成长股」），
/// 但 **DB 存量始终是校准前的原值** —— `merge_variable_values` 的语义是
/// 「新定义 + 无条件保留旧值」，故只改代码常量与种子默认值是**假修复**。
/// 实测（本版前，688114 华大智造一次运行）：DB 三值恰为 10 / 3 / 8，
/// 使 `dcf.mid = 24.23` —— 校正后应为 **37.27（低估 35%）**，且三者同向压低估值。
/// 本版起在变量合并之后覆写，法同 v48 的 `debate_rounds`。
/// **必须升版**：版本门是 `existing.version >= TEMPLATE_VERSION ⇒ return`，
/// 不升版则整个种子流程被跳过，覆写永不执行（判据写了等于没写）。
/// ⚠️ **v75（2026-09-21）**：`portfolio-mgr.rhai` **PE 三态分流**（亏损 ≠ 数据缺失）。
/// 配合上游三家 vendor 放开 `filter(|v| *v > 0.0)`（负 PE 不再被抹成 None ——
/// 负值＝亏损，是有效信息），本脚本把 `risk_pe` 拆成三态：
///   `!present` 真缺失 / `== 0.0` vendor 占位（两者登记缺口）、
///   `< 0.0` 亏损企业（记 f4 note，**不进缺口清单**）。
/// 原口径 `!present || <= 0.0` 把亏损与缺失合流 ⇒ 688114 实证：库里
/// `pe_ttm = −144.08` 有值、决策链仍报「PE数据(t-risk)」缺口，并连带把一致性
/// 评分的 data_gaps 维度打成 0 分、UI 渲染成「本次「观望」为数据不足导致的
/// 被动降级，非看空判断」（方向被说反）。
/// **必须升版**：`portfolio-mgr.rhai` 经 `include_str!` 嵌入本模板的
/// `portfolio-mgr` 节点 `code` 字段，不升版 DB 里仍是旧脚本。
/// ⚠️ **v76（2026-09-22）**：**废除「展示档」派生** —— 结论名回归**方向档** `action`。
/// 实证（300642 透景生命，同日两次运行）：`action` 两次都是「观望」，用户看到的
/// 结论名却是「观望」/「持有」。根因是**循环判据** ——
/// `display_action ← position_state ← position_pct ← 试探仓 ← LLM trader 的定性词`
/// （`!trader_dir_bearish`）⇒ LLM 一个措辞变化经三级旁路改写了结论名。
/// 变动三处，**必须同进退**（本文件此前就因漏改一处而留下同源矛盾）：
/// 1. `portfolio-mgr.rhai`：删 `display_action`，`reasoning` 回归 `final_action`；
///    顺带 `disagreement_mod` 缺失分支 `1.0 → 0.7`（修「数据缺失 ⇒ 置信度反而更高」倒挂）。
/// 2. `portfolio-risk-gate.rhai`：删同源副本 `gate_display_action`，
///    `align_decision_label` 的目标档改用 `final_action`。
/// 3. 前端 `src/lib/stock-analysis-utils.ts::resolveDisplayAction`：改为**恒等**。
///
/// ⚠️ `positionState` **保留**：它仍是独立的展示轴，风控门覆盖仓位后照旧重算；
/// 只是**不再**用于反推结论名。
///
/// **必须升版**：两个 rhai 均经 `include_str!` 嵌入本模板节点 `code` 字段，
/// 不升版则版本门 `existing.version (75) >= TEMPLATE_VERSION` 成立 ⇒ 改动不生效。
///
/// ## v77(2026-09-23)：把「证据可用性」的三处声明变成实现
///
/// 三处改动同属一个契约 —— **腿/因子必须自己声明「本次是否构成证据」，不构成者退出
/// 权重，而不是以 0 值占分母**。三处此前都是「注释声明了、实现没做」（详见各自注释）：
///
/// 1. `portfolio-mgr.rhai` f6：`σ = 0`（`dqi ≥ 50` 数据够用 / `dqi < 25` F 级中性化，
///    两种语义都是「无信息」）时 `f6_weight` 归零 —— 与同文件 f11
///    （`pace_degraded ⇒ f11_weight = 0`）、f13（条件计入）**同范式**，此前未推广。
/// 2. `portfolio-mgr.rhai` f7：`σ = 0`（`trader_direction` 落「中性」档，是生产样本
///    的常态）时 `f7_weight` 归零 ⇒ `total_weight` 分母只统计真正提供方向的因子。
///    已核对 `posterior_cap` 最小权重门 / `no_f7` 纯净对照 / `f7_weight_pct` 三个
///    下游消费点，行为不变或更正确（论证见该处注释）。
/// 3. `portfolio-mgr.rhai` band 腿 + `input_mapping`：新增
///    `valuation_band_verdict` 注入并读取 —— 该腿的注释早已声明「有效性门须按数据
///    形态判 verdict」，但 `verdict` **在全仓 Rhai 侧零注入**（只有
///    `valuation_pe_percentile` 一条）⇒ 在样本不足（`verdict = "insufficient"`，
///    而 `currentPercentile` 仍可能有值）时该腿以**满强度**入场。
/// 4. `portfolio-mgr.rhai` **交易价位**：新增 `targetPrice`/`stopLoss` **绝对价格**输出
///    （`现价 × (1 ∓ 档位%)`，档位由 `timeHorizon` 唯一决定；原内联两处的档位表同时
///    收敛为单一定义 `sl_pct`/`tp_pct`）。
///    此前该 map **只输出百分比**，绝对价格键从不产出 ⇒ 展示层 `targetPrice` 恒 `—`、
///    止损位被 LLM 自填的**单端值**顶替（300642：LLM 给 `stopLoss=17.9`，
///    而公式本可给出 `24.80 / 19.34`）⇒ 用户读到「没有目标价，却有止损」。
///    与 `decision.rs::merge_price_fields_from_llm` 形成**公式优先**（公式键非空 ⇒
///    不再覆盖为 LLM 值）；展示层 `pair_trade_prices` 保留作成对性兜底。
///    rhai `catch` 兜底路径同步输出两键为 `()`（**同 key 集**）—— 异常记录不得携带
///    任何交易结论（沿用该处 B3-c 原则）。
///
/// ⚠️ **本版改变决策数值**（第 1/2 项）：凡 Σ(σ·w) 中 f6 或 f7 为 0 的样本（实测近 24 条样本中
/// f6 σ=0 占 9 条、f7 σ=0 占 8 条），`total_weight` 分母减小 ⇒ `avg_signal` 按剩余
/// 活跃因子重新归一化，信号强度更真实（旧口径把「缺测」当成「观测到中性」计入）。
/// 与 `max_weight`（刻意按 default 计入）的配合会让 `evidence_scale` 同步下降，
/// 属良性耦合（证据量减少 ⇒ 后验收缩）。
/// ⚠️ **第 4 项不改变** action / 仓位 / 置信度 —— 只新增两个展示用价格键。
///
/// ### v77 续（同日）：估值模型参数的科学性订正（第 5–7 项，**改变全部标的的估值**）
///
/// 起因：用户质询「你是不是没有检查相应计算是不是符合科学？」。复算验证方式 —— 由
/// `high/mid` 与 `low/mid` 两比值联立反解出 300642 的 `growth = 17.78%`、
/// `FCF/股 = 0.9819` 元，代入 `mcp_tools::compute_dcf` 复算得 `24.76 / 40.53 / 71.89`，
/// 与面板显示**逐位吻合**（`low/mid` 偏差 0.01%）⇒ 下列结论基于生产公式真值。
///
/// 5. `PERPETUAL_GROWTH` **4% → 2%**（`= RISK_FREE_RATE` 派生）。原值**违反模型自身
///    申报的无风险利率 2.5%**：隐含「该企业永续增速高于无风险利率」= 高于经济体长期
///    名义增速，违反终值约束 `g_terminal ≤ risk-free rate`（Damodaran）。
///    定量：300642 的 `mid` 由 40.53 → ≈31.4（**−22.6%**）⇒ 修正前「低估 92.8%」
///    这一结论里约 **22 个百分点**纯来自一个无依据、且与自身前提冲突的取值。
///    `DISCOUNT_RATE` 同时拆成 `RISK_FREE_RATE + EQUITY_RISK_PREMIUM` 两个**具名**分量，
///    并由**编译期断言**锁死三者等式（数值仍 8.5%，但依据显式化、且三者无法各自漂移）。
/// 6. **折现率纳入区间**（`RISK_STRESS_SPREAD = 1pp`，只加在悲观档）。依据：折现率的
///    弹性是三者中最大的（300642：`|E_d| = 1.96` vs `E_p = 0.78` vs `E_g = 0.71`；
///    `d` ±1pp 使估值变动 **1.60 倍**，大于 `p` 整个 ±30% 档的 1.54 倍）⇒ 原实现把
///    弹性**最大**的参数固定在常量上，区间只覆盖两个弹性最小的参数，**归因错误**。
///    乐观档**不**下调折现率（上界只由经营假设决定，不靠降要求回报灌水）。
/// 7. **档位乘子与 clamp 的冲突修复**：原 `high_growth` 与 `growth` 共用
///    `max_growth` 上界 ⇒ `g ≥ 20%` 时 `g × 1.5` 被砍回中性档（`g = 30%` 时实际
///    乘子 = 1.0），而文案仍声称「×1.5」⇒ **口径与实算不符**（全库 6 条高终值占比
///    样本里 **2 条** `g` 正好顶在 30%）。现上界改为 `max_growth × 1.5`。
///    `MAX_PERPETUAL_GROWTH` 由裸 `0.05` 改锚 `RISK_FREE_RATE` —— 原值只是「刚好压住
///    `0.04 × 1.3`」凑出来的，改 `PERPETUAL_GROWTH` 会让被截断的样本集**静默变化**。
///    同日新增 `MIN_TERMINAL_SPREAD`（1.5pp）守卫：`p`/`d` 均用户可配而 `pct()` 只守
///    `0 < raw ≤ 100` ⇒ `p ≥ d` 可达，此时终值分母塌到地板、估值被放大到
///    `FCF₅ × 2000` 且**静默无日志**；命中即上报为适用性信号 ⑤（腿整体退出）。
///
/// ⚠️ **第 5/6 项改变全部标的的估值号**（第 7 项只影响 `g ≥ 20%` 与永续触顶的样本）。
/// ⚠️ **第 5 项必须配合 `DCF_MIGRATION_VERSION` 由 76 抬到 77** —— 否则 DB 存量的
/// `value_dcf_perpetual_rate = 4.0` 会经扁平参数**优先于模块常量**生效，改动等于没改
/// （同型事故见 v74 注释「只改代码常量是假修复」）。
///
/// ⚠️ **v79（2026-09-23）：由 77 抬到 79 —— 与 `DCF_MIGRATION_VERSION` 对齐**。
///    落库写的是本常量（落库点 = 本文件的 `version: Set(TEMPLATE_VERSION)`），而 DCF 一次性门
///    读的是 `DCF_MIGRATION_VERSION` ⇒ 二者构成一条不变量：
///    **`TEMPLATE_VERSION ≥ DCF_MIGRATION_VERSION`**。
///    倒挂（本常量更低）的后果**不是**「迁移不生效」，而是**反向**的：DB 会停在一个
///    **仍满足 `版本 < 迁移门`** 的值上 ⇒ 门**永远关不上** ⇒ 下一次为**无关**的模板改动
///    升版时会**再次**触发 DCF force，把用户在面板里调好的估值参数打回默认
///    （正是下方 DCF force 段长注释自己警告的那条「每次升版都会把用户调好的参数打回默认」）。
///    本次 77 < 79 即命中该形态（`mcp_tools.rs` 的常量改了、门抬了，但**承载它的模板
///    版本号没跟着抬**）—— 属该族**第五种形态**：前四次是「门没抬 / 抬在改动之前」，
///    这次是「门抬了而落库版本号没抬」。
///    处置：抬到 **79**（= 门的值）⇒ 本次落库后 DB 停在 79，门与版本号**同时关闭**。
///    该不变量已落成**编译期断言**（紧随下方 `DCF_MIGRATION_VERSION`），此后任一常量
///    漂移即编译失败，不再依赖「改的人记得」。
///
/// ⚠️ **v81（2026-09-24）：由 80 抬到 81 —— 修复四周期闭包「按名调用」的运行时失败**。
///    v80 引入的阶段1/阶段2 四周期代码把 `let f = |...|` 定义的**闭包**当普通函数按名
///    调用（`f(x)`）：Rhai 的按名调用只查「AST 内脚本 `fn` 库 + 宿主 native 函数」，
///    **不查作用域里的 FnPtr**（`rhai/src/func/call.rs::exec_fn_call`）
///    ⇒ 必抛 `ErrorFunctionNotFound: sl_pct_for (&str | ImmutableString | String)`（实报
///    line 2505），被上层按「执行异常」**降级为保守决策**（action=观望、confidence=0）。
///    处置：全部改为 `f.call(...)`（同 `strategy-scorer.rhai:9` 既有约定），并把
///    「闭包按名调用」写进 `rhai_registry.rs` 的静态门禁防复发；同批扫描发现
///    `reflection-comparator.rhai` 的 `sink` 闭包同一写法（会让不可信节点扫描恒空、
///    `untrusted_count` 恒 0），一并修正。
/// ⚠️ **v82（2026-09-26）：`data-quality.rhai` as-of 回放「设计性降级」豁免**。
///    实证（300642 as_of=2026-09-22）：回放中搜索/快讯/政策新闻等按「当下语义」设计性
///    返回空（astock-data as_of 决策矩阵，record_degradation 留痕），分析师如实写
///    「无法获取」却被失败标记词表按**工具故障**扣分 ⇒ tool_credibility 27、综合 C 级，
///    回放评分既不可与 live 横比、又以错误归因污染反思链。
///    本版：新增宿主函数 `pm_asof_degraded_methods`（rhai_pm.rs，live 恒 "[]"），
///    脚本按 method→维度映射豁免对应分析师的占位扣分与失败清单，
///    输出新增 `asof_replay` / `asof_designed_dims` / `asof_degraded_methods` 三字段，
///    summary 带「【as-of 回放】」前缀。判据见 `PLAN-asof-replay-quality-attribution.md`，
///    门禁在 `rt-workflow/tests/data_quality_placeholder_gate.rs`（S2 四条）。
///    **必须升版**：`data-quality.rhai` 经 `include_str!` 嵌入本模板节点 `code` 字段。
/// ⚠️ **v83（2026-09-26）：S5/S6 —— 回放重跑（1ad42f59）暴露的两处修正**。
///    S5（宿主侧，rhai_pm.rs/core.rs/as_of.rs）：豁免判据由「按 as_of 日期过滤缓冲」
///    改为「按本轮运行基线 seq 过滤」——日期口径挡不住同截止日旧运行的残留条目，
///    实测把全部 10 维无差别豁免、连真工具故障一起抹掉。
///    S6（本脚本）：`diag_for` 增第 9/10 参（asof_ex / raw_ph_n）。豁免生效后 ph 恒 0，
///    原「非数据缺口：报告无失败标记」分支对被豁免维度是**假话**（标记存在，只是不计），
///    现输出「as-of 回放：本维度上游按设计降级…已豁免不计扣分」。
///    **必须升版**：`data-quality.rhai` 经 `include_str!` 嵌入本模板节点 `code` 字段。
/// ⚠️ **v84（2026-09-26）：R9a —— 方向冲突退出 tool_credibility 扣分**。
///    实证（40 次 live 运行逐条提取）：severe_direction_conflict 命中 30/40，
///    判据 ≈「10 个分析师里凑出两边各 2 个 conf≥50」——多空辩论架构的设计常态
///    被当成恒触发的 −20 系统性偏置，且与「上游工具可信度」语义无因果
///    （同 2026-09-24 移除 good_count 惩罚的判据）。只去扣分、不去信号：
///    direction_conflict / disagreement 字段与 warnings 全部保留。
///    **必须升版**：`data-quality.rhai` 经 `include_str!` 嵌入本模板节点 `code` 字段。
/// ⚠️ **v85（2026-09-26）：R9b —— 词表缺席类定向抑制（live 误伤修复）**。
///    40 次 live 运行报告句抽样：标记词大量命中于**合法缺席语境**（北向监管停披、
///    无机构覆盖、无期权覆盖、auto_stop_loss_pct 配置未注入、「非空值」肯定式）。
///    `未注入` 降为软标记（只配配置参数否定词）；`数据不可用/无数据/均为空/返回空/空值`
///    补充缺席否定短语。`数据缺失` **维持硬标记** —— 门禁实证正文级共现抑制会吞同篇
///    真缺口（clarification/hard_marker 两判据当场抓回）。动词类硬标记判据不变。
///    **必须升版**：`data-quality.rhai` 经 `include_str!` 嵌入本模板节点 `code` 字段。
pub(crate) const TEMPLATE_VERSION: i32 = 85;

/// DCF 估值参数**一次性**迁移门的水位线。
///
/// 语义：`previous_version < 本值` ⇒ 强制把 DB 存量的
/// `value_dcf_{discount,perpetual,growth}_rate` 覆写为 `seed_variables` 的派生默认值。
/// 取值判据**不是**「比上一版 +1」，而是**「被守护的常量最后一次变更时对应的
/// `TEMPLATE_VERSION`」**（沿革与每次抬门的理由见调用点 DCF force 段的长注释）。
///
/// ## 为什么提到模块级
///
/// ① 让紧随的编译期断言能引用它（见下）；② 与 `TEMPLATE_VERSION` 并排，使
/// 「两者必须满足 `TEMPLATE_VERSION ≥ DCF_MIGRATION_VERSION`」这条约束**可见**。
pub(crate) const DCF_MIGRATION_VERSION: i32 = 79;

// 编译期断言：`TEMPLATE_VERSION ≥ DCF_MIGRATION_VERSION`。
// 为什么必须是**编译期**而不是测试：它是**跨常量**的等式型约束，编译期断言让它无法被
// 「改一个忘一个」绕过（测试要靠人记得跑，而这条约束的失效形态恰恰是「没人注意到」）。
// 也不要退回「把两个数值写在一起」—— 那正是手抄，会各自漂移（见 `AGENTS.md` 规范）。
const _: () = assert!(
    TEMPLATE_VERSION >= DCF_MIGRATION_VERSION,
    "版本号与 DCF 迁移门水位线倒挂：`TEMPLATE_VERSION` 必须 ≥ `DCF_MIGRATION_VERSION`。\
     落库写的是前者、门读的是后者 ⇒ 倒挂会让 DB 停在低于门的版本上，门永远关不上，\
     下次无关升版会把用户面板调好的 DCF 参数打回默认。详见两个常量的文档注释。"
);

// ## 为什么提到模块级
//
// 让 `mod.rs` 末尾的 `version_gate_tests` 能直接引用它去做守门断言
// （低于它的旧版本必须被升级、高于它的版本必须被跳过）。
// ⚠ 本块刻意**不写 `///`**：它讲的是常量的**安置理由**，不是常量语义 ——
// 并进 `TEMPLATE_VERSION` 或 `DCF_MIGRATION_VERSION` 任一者的文档都是**语义错位**；
// 而普通 `//` 注释不参与文档归属 ⇒ 不会触发 clippy `empty_line_after_doc_comments`
// （`-D warnings` 下该 lint 直接让构建失败）。

pub(crate) async fn seed_stock_analysis_workflow_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    use axagent_entities::workflow_template;
    use axagent_harness::hallucination_guard::HallucinationGuardConfig;
    use axagent_harness::workflow_types::{
        AgentNode, AgentNodeConfig, AggregatorNode, AggregatorNodeConfig, BackoffType, Branch,
        CodeNode, CodeNodeConfig, DebateNode, DebateNodeConfig, DegradeStrategy, EdgeType, EndNode,
        EndNodeConfig, ErrorConfig, JsonSchema, JsonSchemaProperty, MergeStrategy,
        NotificationNode, NotificationNodeConfig, OnFailureAction, OutputMode, ParallelNode,
        ParallelNodeConfig, Position, RetryConfig, StorageNode, StorageNodeConfig, SubGraph,
        SwitchCase, SwitchNode, SwitchNodeConfig, ToolDef, ToolNode, ToolNodeConfig, TriggerConfig,
        TriggerNode, TriggerType, ValidationAssertion, ValidationNode, ValidationNodeConfig,
        Variable, WorkflowEdge, WorkflowNode, WorkflowNodeBase, WorkflowRetryPolicy,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    // 与原链派生源（快速链种子从本行派生）共用同一常量，避免字面量两处漂移
    const TEMPLATE_ID: &str = SOURCE_TEMPLATE_ID;

    // V3(2026-08-09): data-quality.rhai 全量缺陷修复——
    //   P0: count_chars replace 崩溃（Rhai 默认 Engine 无 StringPackage）+ pm_compute_factor_completeness
    //       Option 参数注册不可调用（Rhai 1.25 多 Option 参数闭包 Function not found）→ Dynamic 参数
    //   P1-1: money_flow/lockup_bundle/announcements 注入为 map 的 type_of 判断修复
    //   P2-2: trader_direction 类型防御；P2-3: n 动态推导；P3-1/2/4: count_chars/consistency_bonus/diag_for
    //   v4: 移除 bear-r3 → t-dragon-tiger-data 入边——与 t-dragon-tiger-data → a-hot-money
    //       构成回环（a-hot-money 在辩论链上游），Kahn 检测拒绝启动（"Cycle detected"）。
    //       龙虎榜取数改为入度 0 启动节点，天然先于 a-hot-money 完成。
    //   v5: 声明 hooks_config（precheck/enhance/persist 三钩子）——变量增强统一由
    //       stock-analysis-enhance 钩子执行，对话直执行路径与业务封装路径变量零漂移。
    // v6: 强制重种子，落库 a-fundamentals 的 market_regime input_mapping。
    //   背景：fundamentals-analyst.md（运行时从磁盘加载）引用 {{market_regime}}，
    //   而 DB 模板 v5 的 inputMapping 只有 stock_lessons，导致运行期
    //   VARIABLE_NOT_FOUND → a-fundamentals 4 次重试全失败 → 辩论链/t-scoring/
    //   t-valuation/portfolio-mgr 全部停摆 → 决策降级为观望（2026-09-08 实证）。
    //   种子代码 963-967 行的映射逻辑早已存在，但 v5>=v5 跳过重种子，静默失效。
    // v7: a-fundamentals 失败降级而非断链——debate-bull-bear / data-quality
    //   开 continue_on_fail（2026-09-08）；引擎侧同步新增确定性错误不可重试分类。
    // v8: max_concurrent 3→8（2026-09-08）——DB 存量 v7 实际值仍为 3（旧种子遗留），
    //   3 个并发槽被 429 重试节点占住不放时其余分析师排队等待，事实串行化。
    // v9: cls-risk-level 死锁修复（2026-09-08 实证）——① timeout 30→60s + 重试 +
    //   fallback_label；② v-validate 改依赖 t-risk（拆除 cls-risk-level→v-validate
    //   死锁边）。data-quality/portfolio-mgr/portfolio-risk-gate 的 continue_on_fail=true
    //   已在 v7/v8 落库，本版本递增确保上述改动重种子生效。
    //   ⚠️ v55(2026-09-20) 起①的三重防线已无必要：该节点改为 Rhai CodeNode 后不再发起
    //     LLM 调用，超时成因（agnes-3.0-flash 34.95s > 30s）从根上消失 ⇒ timeout 60→10s、
    //     重试关闭；②的拆边仍然有效（v-validate 依赖 t-risk，与本节点无关）。
    // v10: 决策尾链全链容错（2026-09-08 遗留风险排查）——v-validate/research-mgr/
    //   trader/quality-gate/quality-fallback/decision-explainer/notify-result/
    //   store-result/end-output 全部开 continue_on_fail：尾链任一上游失败不再死锁，
    //   已产出的决策必达 notify/store/end。失败降级依据：Agent context_sources 缺失
    //   静默跳过、Rhai present() 守卫 + Null 注入。例外：rule-check 保持 false——
    //   portfolio-risk-gate（硬裁决）失败时决策本身不存在，尾链继续跑只会产出
    //   垃圾解释，死锁 halted 是诚实语义。
    // v11: input_mapping 包装对齐（2026-09-09）——ToolNode 输出结构已变为
    //   {node_id, result: {content: <json_string>, tool_name}}，数据在 result.content
    //   字符串里；AgentNode content parse 后为 {report, verdict}，结构化字段在 verdict
    //   层。全部 data-quality/trader/portfolio-mgr/pace-calc/portfolio-risk-gate 的
    //   input_mapping 穿透 .content / .verdict；resolve_var_path 终值恢复「不 auto-parse」
    //   语义（JSON 字符串原样到达 rhai，契合消费端 type_of=="string" 契约）；
    //   compute_valuation 补 dcf/graham/fScore camelCase 别名块（上行空间百分比语义）。
    // v12 (2026-09-09 晚): pace-calc.rhai 两个顶层 map（base_score_map/source_weight_map）
    //   挪进 classify_event/classify_source 函数体 —— Rhai 脚本函数不捕获任何调用
    //   作用域，顶层 let AND 顶层 const 对函数均不可见（实测 const 同样报
    //   ErrorVariableNotFound）。此前资金流/公告数据为空时脚本提前降级未触达
    //   两个分类函数，v11 修好数据链后首次走到 → Variable not found:
    //   source_weight_map（line 114 in classify_source, called at line 247）。
    // v13(2026-09-10): cat_verdict 映射 a-catalyst.content → a-catalyst.content.verdict
    //   （终值不 auto-parse 导致 rhai 收到 JSON 字符串，extract_conf 恒 -1 → 催化剂恒 missing）
    // v14(2026-09-10): portfolio-mgr.rhai 决策死锁修复（DB 实证近 30 天 41 次分析 100% 观望 0%）:
    //   ① odds_fallback 新增试探档 posterior∈[0.42,0.50) → 1.5x（原 <0.50 恒 0）
    //   ② 新增试探仓机制 R-207：posterior∈[0.42,0.50) 且 odds>0 时给 3% 试探仓位，
    //      观望升级为持有（与趋势智选试探仓语义对齐）
    // v15(2026-09-10): a-catalyst 从 OutputMode::Json 改回默认 Text 模式（与其他 9 个
    //   分析师统一「报告文本在前 + 末尾 VERDICT 标签」）。根因：Json schema 注入要求
    //   纯 JSON 禁止标签，与 catalyst-analyst.md 第 7 步残留的 Text 指令自相矛盾，
    //   模型两头都执行 → JSON+标签混合体（用户实测）。催化剂特有字段改由 VERDICT
    //   标签承载（executor 分支 A 原样进 verdict map），下游映射路径全部不变。
    //   ③ trader 数据异常从「清零/强制卖出」改为「降级」：仓位上限 10%，激进动作封顶持有
    // v16(2026-09-10): 信号层激活修复（90 天审计实证 30 条决策 29 观望 1 卖出 0 买入，仓位全 0）:
    //   ① portfolio-mgr.rhai f9 资金流字段名兼容：get_stock_money_flow 实际输出
    //      camelCase(mainNetInflow/superLargeNet/largeNet/mediumNet)，原代码按
    //      snake_case 读取恒 () → f9 恒 0。DB 实证 002837 主力净流入 +5010 万在场，
    //      f9=0 —— 唯一稳定的正信号源被字段名错配杀死。
    //   ② pace-calc.rhai 输入防御解包：{content:"..."} 包装对象统一解包后再解析
    //      （301302 实证 pace_degraded=true, reason="无有效事件输入"，而
    //      t-catalyst-data 公告数据明明在场）。
    // v17(2026-09-10 晚): pace-calc.rhai P 维度符号翻转修复。tanh 归一化后错误地
    //   再乘 p_score.sign() 二次应用符号 → 所有负极性(L-1/L-2/L-3 利空)被翻成
    //   正 P 值。603290 实证: p_raw=-0.88(L-2业绩暴雷) → P=+0.53。利好路径
    //   (正×1=正)不受影响故从未暴露。修复: clamp(tanh_val) 保号。
    // v18(2026-09-10): value-investor 接入 t-risk 结构化基本面硬数据
    //   (context_sources+显式边+数据约束 prompt)，护城河/财务健康 60% 权重
    //   从 LLM 叙述改为真实财报锚定。
    // v19(2026-09-10 深夜): 热点股动量通道（601231 涨停板实测 0% 复盘）:
    //   ① portfolio-mgr.rhai 新增 f12 动量因子（MACD 趋势状态 + RSI 调制，
    //      权重 0.10，f1↔f12 协方差衰减 25%）——此前因子集无正向动量通道，
    //      涨停/趋势动能只能通过 totalScore(+0.1 上限)微量表达；
    //   ② S-503 冲击成本阈值 50→150bps（实测常态 100~200bps，50bps 必触发）。
    // v28(2026-09-11): 决策链 P0 修复 ——
    //   ① f9 资金流归一化分母修正（旧式自身归一化 → 恒 ±0.67 伪二值，
    //      近 30 天 39 次分析 0% 买入的系统性负偏置来源之一）；
    //      新增 portfolio-mgr input_mapping: kline_json → t-scoring.result.content.kline_json
    //      作为「近 5 日平均成交额」分母来源（volume × close 估算）；
    //   ② extract_decision_json 硬化：候选决策必须含非空 action，
    //      否则逐级兜底到保守占位（消除 DB 中 decision_action 为 NULL 的
    //      "completed 但无决策" 行）。
    //   注：DB 端该版本已通过直写模板行落地（version 27→28），
    //      常量同步为 28 使后续 seed 版本门稳定（>= 即跳过，不覆盖 UI 自定义）。
    // v29(2026-09-11): 决策链 P1 六项改动（科学合理决策导向）——
    //   ① P1-A prior 改由市况**方向**派生：旧映射 market_regime_prior →
    //      market_regime.confidence 是分类置信度（bull/bear 分支公式相同，
    //      仅反映偏离 MA60 幅度，且常态被夹到下界 → 实测恒 0.5），
    //      等于屏蔽市况信息。改为由 market_regime_state 派生
    //      （bull 0.55 / sideways 0.50 / bear 0.45，可经 input_mapping 覆盖）。
    //   ② P1-B f4 风险因子成长风格豁免：营收增速 ≥40%/≥25%/≥15% 时，
    //      波动率/回撤/PE 偏高三项风格相关惩罚分别打 0.35/0.55/0.75 折；
    //      夏普、PE≤0（亏损）、风险分类保持原样（非风格问题）。
    //   ③ P1-C f7 posterior_cap 收敛：加最小权重门（<0.08 不再一票否决）、
    //      触发阈值下移（-0.5/-0.2 → -0.6/-0.35）、封顶值上调（0.48/0.53 → 0.50/0.55）。
    //   ④ P1-D f9 权重重估：regime-weights money_flow base 0.08 → 0.12，
    //      并在 f1↔f9 共振衰减处加 ≥0.06 权重门（避免二次惩罚）。
    //      重估后 bear 市况有效权重 0.042 → 0.063。
    //   ⑤ P1-E 模拟门修复与阈值重估（hooks.rs）：sim_stability 括号错位修复
    //      （恒 1.0 → 恢复动态范围，S-501 阈值 0.3→0.15）；sim_impact 量级修正
    //      （实测恒顶格 200 → 恢复 5~150bps 区分度，S-503 阈值 150→60bps）。
    //   ⑥ P1-F 契约对齐：regime-weights.rhai 分桶输出（factor_weights 仅含被
    //      portfolio-mgr 消费的 5 个因子；未消费的 catalyst/risk/data_quality/
    //      trade_signal 移入 unconsumed_suggestions，消除 48.6% 的死计算）。
    //   注：DB 端该版本已通过直写模板行落地（version 28→29），常量同步为 29。
    // v30(2026-09-11): 决策参数去硬编码（可配置 + 可被反思/演进优化）——
    //   修复一处长期存在的四处断链：portfolio-mgr 的 input_mapping 早已映射
    //   24 个决策参数（action_* / pos_* / risk_* 的同名变量），但
    //   seed_variables.rs 从未定义过这些变量，于是：
    //     ① context.variables 查不到 → rhai 的 present() 恒假
    //        → 全部静默走 .rhai 内硬编码默认值，「D7/D8 可配置」形同虚设；
    //     ② 前端 StockAnalysisConfigPanel 的 portfolio_mgr_action /
    //        portfolio_mgr_risk 两个分组被 .filter(Boolean) 整组过滤（界面空白）；
    //     ③ 反思/演进产出的参数建议（apply_param_suggestions）按名字查变量，
    //        找不到即 tracing::warn 静默丢弃 → 参数优化链路 100% 失效；
    //     ④ 反思侧 PortfolioMgrParamSet 用 buy_threshold / cap_high 短名，
    //        与本模板 action_buy_threshold / pos_cap_high 全名不一致。
    //   本版改动：
    //     a) seed_variables.rs 补齐 26 个变量 = 3 个市况先验
    //        （regime_prior_bull/sideways/bear，可被反思直接校准）
    //        + 23 个决策参数（action 阈值 5 / 仓位阈值 2 / 风险仓位上限 3 /
    //        风险分类阈值 13）；cost_pct 原已存在，不重复定义。
    //     b) portfolio-mgr input_mapping 新增 regime_prior_bull/sideways/bear
    //        三条同名映射（其余 24 条原本就在，补齐变量后即真正生效）。
    //     c) stock_analysis.rs::apply_param_suggestions 增加 PARAM_ALIASES 别名表
    //        + camelCase→snake_case 归一 + 数值有限性守卫，并返回
    //        applied/skipped 明细（不再静默丢弃）。
    //     d) 前端变量字段名兼容：DB 存 snake_case（var_type），前端类型是
    //        camelCase（varType），此前直接读 v.varType 恒 undefined，导致所有
    //        参数控件退化成纯文本框（数字无 Slider、布尔无 Switch、枚举无 Select）。
    //   注：DB 端该版本已通过直写模板行落地（version 29→30），常量同步为 30。
    // v31(2026-09-11): 打通「先验/参数可被反思与演进优化」的剩余两个断点 ——
    //   v30 只解决了「变量存在且名字能对上」，优化链路仍缺两环：
    //     ① 反思 prompt（reflection-agent.system_prompt）的 params_suggestion
    //        只给了 "param": "参数名" 的自由占位符，**没有可调参数清单**，
    //        LLM 不知道存在 regime_prior_* / action_* / risk_* 可建议，只能凭
    //        训练先验自造名字 → 反思侧对先验提出建议的概率≈0。本版新增
    //        {{tunable_params_catalog}} 占位符（清单运行时按变量表派生，
    //        单一权威源），并要求 param 严格取自清单。
    //     ② 演进侧 PortfolioMgrParamSet（WFO 校准目标）不含市况先验，
    //        default_grid() 全部候选的先验恒等于默认值 →「网格搜索先验」
    //        等价于没有搜索。本版为 ParamSet 增加
    //        prior_bull / prior_sideways / prior_bear 三字段
    //        （默认 0.55/0.50/0.45，与 regime_prior_* 变量一致），
    //        default_grid() 增加 2 组先验敏感候选，
    //        try_parse_param_suggestion 支持短名/camelCase/模板全名三种写法，
    //        WFO 结果 JSON 增加 priorBull/priorSideways/priorBear 输出字段。
    //   ③ 可调参数清单收窄 + 单一权威源：v31 初版用「名字前缀扫全变量表」派生
    //      {{tunable_params_catalog}}，实测混入 41 项，其中 risk_free_rate /
    //      risk_hhi_* / risk_sharpe_annualization 等属**模型级参数**（只影响估值与
    //      风险的计算结果，不移动 portfolio-mgr 的决策边界），列给 LLM 稀释注意力。
    //      改为由常量 PORTFOLIO_MGR_TUNABLE_PARAMS 显式登记，收窄到 28 项，
    //      顺序即决策相关性顺序；同一常量同时驱动 input_mapping 派生与前端分组。
    //   ④ 补齐第 28 项 trader_cap_min_weight：portfolio-mgr.rhai 早有
    //      `if present(trader_cap_min_weight) { ... } else { 0.08 }` 守卫，
    //      但变量表从未定义该名、input_mapping 也无同名映射 → present() 恒假，
    //      永远走硬编码 0.08（「配置项空接线」的隐蔽形态：守卫存在让人误以为已接线）。
    //      本版补变量定义，使其真正可配置、可被反思/演进优化。
    //   注：本版 prompt 内容变更，必须升版才能覆盖 DB 中已落地的 v30 模板行；
    //      变量值仍由 merge_variable_values 按 name 保留用户自定义。
    //      （③④ 两项与 ①② 同批落库 —— 截至本版 DB 仍为 v30，故无需再递增版本号；
    //        判据：DB 模板行 version < TEMPLATE_VERSION 时本次种子必然执行。）
    // v32(2026-09-12): 估值配置**真正接线**（C2 路径 Z）—— 修复「面板可写但零处生效」。
    //   背景：设置面板的 `value_*` 变量此前在生产链**无任何消费方** —— t-valuation 的
    //   input_mapping 只映射 stock_code，rhai 中零 `present()`，WhatIf 的覆盖值也未进入
    //   invokeParams。与此同时 `astock-data` 用模块常量 0.10/0.03/0.08 **硬编码**执行，
    //   与面板显示值（8.5/4.0/12.0）不一致 —— 本质是「A 股校准只改到了未被消费的
    //   `decision::ValueConfig`，没改到实际执行的常量」。
    //   本版三处改动：
    //     ① `compute_valuation` 新增**扁平参数**解析
    //        （`dcf_growth_rate` / `dcf_perpetual_rate` / `dcf_discount_rate`，
    //         **百分数**口径，后端单点 `/100` 换算）；优先级
    //        `valuation_config`(object) > 扁平参数 > 模块常量默认。
    //     ② t-valuation 的 input_mapping 接入 3 个 `value_dcf_*` 变量
    //        （`tool_node` 闭包新增 `extra` 参数支持多映射）。
    //     ③ `td_val` 的 ToolDef schema 补齐这 3 个参数声明。
    //   为什么必须扁平化：`ToolNodeConfig.input_mapping` 是 `HashMap<String, String>`，
    //   dispatcher 把 value 当**变量名**查（`crates/rt-workflow/src/work_engine/dispatcher.rs:463-466`），**构造不出嵌套
    //   object** → `valuation_config` 路线在模板侧不可达；即便强行传标量，
    //   `from_value::<ValuationConfig>` 失败也会被 `.ok()` 吞掉，症状是「改了设置但估值
    //   毫无变化」这类最难查的静默失效。
    //   未接线说明：另 3 个变量 `value_moat_threshold` / `value_fscore_buy` /
    //   `value_safety_margin` 对应 `decision::ValueConfig` 的分级判据，而 astock-data
    //   的分级逻辑（`f_score_level` 7/5/3、`mos_level` 30/15、`compute_moat_score` 70/40）
    //   用的是**不同口径**的硬编码阈值，强行接线会改变分级语义 → 本次**不接**，另行评估。
    //   注：变量值仍由 merge_variable_values 按 name 保留用户自定义。
    // v33(2026-09-12): 移除 data-quality 的 f7「交易方向」**死因子**。
    //   背景：data-quality 节点是 trader 的**上游**（trader 的 dqi_score 由本节点提供），
    //   而 `trader → data-quality` 的边因循环依赖早已被删除（见本函数末尾注释）
    //   ⇒ data-quality 的 `trader_direction` input_mapping 永远解析不到值（死映射）
    //   ⇒ f7 因子恒缺失 ⇒ factor_completeness 上限被钉在 0.9、综合分恒定少 3.0 分
    //   （score = 报告×0.35 + 工具×0.35 + 因子完整度×100×0.30），且前端
    //   「分析师数据差距详情」面板恒显示「缺失因子：交易方向」这条无法消除的假告警。
    //   本版四处同步改动：
    //     ① `portfolio_formula::compute_factor_completeness` 删除 `trader_direction`
    //        参数与 f7 计分块，分母 10 → 9（4 个单测同步更新期望值）；
    //     ② `rhai_pm.rs` 的注册闭包 10 参 → 9 参；
    //     ③ `data-quality.rhai` 删除 trader_direction_safe 准备、调用传参
    //        与 `missing_factors.push("交易方向")`；
    //     ④ 本文件删除 data-quality 的 `trader_direction` input_mapping。
    //   **注意：f7 在决策链中并未失效** —— portfolio-mgr 有自己的 trader_direction
    //   映射（trader.content.verdict.verdict）且 `e-trader-portfolio-mgr` 边存在，
    //   `f7_signal` 取值正常。本次仅修正「上游节点评估下游因子」这一架构错位。
    //   DB 版本核对：落版时 DB 主表已是 v32（2026-09-12 07:16 重种子），故必须升 v33
    //   才能触发重种子，使本改动落库。
    // v34(2026-09-12): data-quality.rhai 打通「报告文本失败标记」与「status/评分」两条链路。
    //   背景（603353 实证）：脚本内原有两套互不联通的「数据质量」判定 ——
    //     路径 A `has_placeholder`（依据报告**文本**，客观）：只用于 report_quality 扣 15 分
    //       + 一句 warnings，且 `:840-843` 把 10 个节点 OR 成**一个布尔**（丢失「是哪个」）；
    //     路径 B `diag_for`（依据 LLM **自评** confidence，主观）：生成每节点 status 与面板表格。
    //   ⇒ 同一份报告里，失败标记最多（5 处「无法获取/返回空」）的 `hm`(cf=55) 显示「✅ 正常」，
    //     而标记为 0、数据齐全的 `sec`(cf=45) 被判「⚠️ 低置信」；`lk`(cf=65, 2 处标记) 同样误判。
    //     面板呈现的是「模型觉得自己有多确定」，而非「数据到底缺没缺」。
    //   本版 data-quality.rhai 四处修复：
    //     ① `placeholder_markers` 从 12 个形态扩充到 21 个（补 `为 null`/`返回空`/`数据不可用`/
    //        `无数据`/`=null`/`均为空`/`获取失败`/`未能获取`/`空值`），
    //        实测原表假阴性 27/258 = 10.5%（占全部真问题报告 25%）；
    //        新增 `placeholder_hits` 返回命中清单（折叠「占位报告」/「占位」重复计数）；
    //     ② `diag_for` 增加 `ph_n` 参数，status 判定改为 **A ∪ B**（报告含失败标记即至少 low），
    //        并按证据来源输出确定归因，替换原「可能原因：A / B / C」三选一免责罗列；
    //     ③ `good_count` 判据加 `ph == 0` 条件，新增 `degraded_count`
    //        （自评 ≥50 但报告含失败标记）—— 修复前这类节点被计为 good，虚高 tool_credibility；
    //     ④ `warnings` 与输出新增逐节点清单 `placeholder_nodes` / `placeholder_node_ids` /
    //        `placeholder_total_hits`，前端新增「失败标记」列。
    //   ⚠️ 本版会改变**结论数值**（非仅显示）：markers 扩充使更多报告触发 -15 分，
    //      good_count 下降使 tool_credibility 的 good 惩罚更易触发 ⇒ score 整体下移，
    //      评级可能跨级。属预期内的口径修正。
    //      （注：good 惩罚已于 2026-09-24 整体移除，本项现仅剩 -15 占位符扣分生效，
    //        理由见 data-quality.rhai 的 tool_credibility 注释。）
    //   DB 版本核对：落版时 DB 主表已是 v33（2026-09-12 用户重跑 603353 时重种子），
    //   故必须升 v34 才能触发重种子，使本改动落库。
    //
    // v35(2026-09-12): portfolio-mgr.rhai 决策可信度三处修复 ——
    //   ① sanity「检查 1」（targetPrice <= stopLoss ⇒ 判数据异常）改为**按 trader 方向豁免**：
    //      看空方案的标准结构本就是 targetPrice(下方目标) < currentPrice < stopLoss(上方止损)，
    //      旧实现按做多语义无条件校验，把所有看空样本打成 R-202「trader数据异常」
    //      （实证：targetPrice 1.8 <= stopLoss 2.34 且 action=卖出 仍被判异常）。
    //      豁免只针对「字段倒置」；检查 2（偏离 >70%）与检查 3（价格 <1 元）照常执行。
    //   ② odds_source 归因条件与 odds 实际生效分支对齐 ——
    //      旧实现只判字段是否存在，产出「赔率=0.0(trader)」假归因（该 0.0 实际来自 odds_fallback）。
    //   ③ trader 数据异常文案不再拼接 no-op 的「action已从卖出修正为卖出」。
    //   ④ 输出字段 `isContradictory` 同样加**看空豁免** —— 其旧定义 `targetPrice <= stopLoss`
    //      与 ① 是同源缺陷（隐含做多语义），看空样本恒为 true ⇒ DecisionBanner 对方向自洽的
    //      「卖出」样本挂出「⚠ 决策矛盾」红标。现判据与 ① 完全对齐（`&& !trader_dir_bearish`）。
    //   ⚠️ 本版可能改变**结论数值**：③ 仅文案、② 仅归因、④ 仅前端标记；① 使原本被误判的看空样本
    //      trader_data_valid 恢复 true ⇒ 仓位不再被「异常降级」压到 ≤10%（但赔率仍走 fallback，
    //      posterior<0.42 时凯利与仓位仍为 0）。属预期内的口径修正。
    //   前端配套（无需重种子，随构建生效）：DecisionHeroBar 的「决策冲突」判据改用
    //      agreementBreakdown.actionOk，不再误用后端 isContradictory（该字段语义是「价格字段倒置」，
    //      v35 起已对看空方案豁免，但表达「公式 vs LLM 是否分歧」仍须用 agreementBreakdown）；
    //      DecisionTrustNotice 不再把「卖出/减持」的零仓位渲染成「观望被动降级」。
    //
    // v36(2026-09-12): 重跑 v35 后的新样本（603353, 09-12 13:33）暴露两处新缺陷 ——
    //   ① **试探仓与「观望」行动档脱钩 ⇒ 产出「减持 + 3% 仓位」**
    //      实测：posterior=0.428 落入试探窗口 [0.42,0.50) ⇒ 给 3%；但 action 阶梯读
    //      effective_posterior = posterior + risk_bias(高风险=-0.08) = 0.348 ⇒ 减持区
    //      [0.30,0.38)。两者同源不同口径 ⇒ 高风险档下 posterior∈[0.42,0.46) 必然重叠，
    //      于是输出「action=减持 且 positionPct=3%」，与 LLM trader 的「减持/0%」直接打架。
    //      修法：position_pct 赋值移入 base_action 之后的 R-207 块，仅在「观望」档生效
    //      （试探仓的设计语义本就是「观望升级为持有」）；窗口命中但档位不符则落 SKIPPED trail。
    //      配套：空头判据由「targetPrice 低于现价 ≥15%」的**幅度代理**（trader_bearish）
    //      改为方向语义（trader_dir_bearish，v35 新增）—— 本样本缺口仅 -4.46% 故旧判据不拦截，
    //      但 trader 已明确 verdict=看空/减持/0%，与护栏注释「空头预测不激活」不符。
    //   ② **trader_confidence 单位不一致 ⇒ f7 退化为伪二值**
    //      输入映射 trader_confidence ← trader.content.verdict.confidence 是 LLM 的 0~100 口径
    //      （DB 实证 70 / 92 / 85），而 V65 计算公式按 0~1 设计（注释「0.5 为中性」）⇒
    //      conf*0.6*combined_mod 恒 ≥4 ≫ 上限 0.6，被 clamp 顶在边界 ⇒ f7 ∈ {±0.6, 0} 三值，
    //      conf / evidence_mod(引用密度) / risk_mod(风险等级) 三重调制全部失效。
    //      与 V70 的 f9（恒定 ±0.67）同类。后果更重：f7 单因子贡献 ±0.06，而本轮样本
    //      Σ(σ·w) = -0.0595 —— f7 恰为唯一负项，直接决定 avg_signal 的符号与 posterior 走向。
    //      修法：conf > 1 视为 0~100 口径除以 100（兼容 i64/f64 注入）。
    //   ⚠️ 本版改变**结论数值**：② 使 f7 由 -0.6 变为 -0.294（本样本）⇒ avg_signal
    //      由 -0.0408 升至 -0.0198 ⇒ posterior 0.428 → 0.439；① 使 positionPct 由 3 → 0。
    //      两者叠加后本样本最终为「减持 / 0%」，与 LLM trader 一致（详见记忆 2026-09-12 复算）。
    //
    // v37(2026-09-12): 报告导出「三板块恒空」的**数据侧**落地（对应审计 §7.7 / PLAN P0-H L3）。
    //   · 新增 2 个独立 ToolNode（不配 Agent）：
    //       - `t-index-quotes`          ← `get_index_quotes`（**无参**，用新增的
    //                                     `tool_node_noarg` 闭包，避免注入无意义的 stock_code 映射）
    //       - `t-institutional-visits`  ← `get_stock_institutional_visits`
    //   · 两者挂 `trigger` 入边 + `→ raw-data` 入边，并加入 `raw_input_sources`
    //     ⇒ `raw.combined.source_count` 由 **14 → 16**，`result[]` 多 2 项。
    //   · 配套修 `vendors/eastmoney.rs::get_institutional_visits` 的**三层静默失效**
    //     （报表名 9501 + 排序键不存在 + 字段名全错，详见该处注释）。
    //   · **不新增**「同行对比 / 期权PCR」节点（报告已删这两个板块：前者 `get_peers`
    //     用概念板块冒充行业、后者对个股恒 `None`，均为独立缺陷，已登记）。
    //   ⚠️ **本版不改变任何决策数值**：raw-data 的消费者只有 `generate_stock_report`
    //      （读 `raw.combined`），而 portfolio-mgr 已是 CodeNode、**不设 context_sources**，
    //      `raw-data-aggregated` 变量在全仓**零消费点**（已 grep 实证）⇒ 新增数据节点
    //      不会进入任何 LLM 上下文，也不参与任何因子/后验计算。纯「报告出口」修复。
    // v39(2026-09-13): trader `targetPrice` 方向语义约束 + 「目标价=现价」显式判无效（603466 实证）。
    //   背景：用户报告「同一工作流里巴菲特估值（内在价值 4.44–5.57 / 理想买入价 4.44 元以下）
    //   与仪表盘目标价（13.27 = 现价）完全矛盾」。溯源结论：**不是同一结论自相矛盾，
    //   而是两条语义完全不同的链共用「目标价」这个词**，且中间有一段静默失效：
    //   · 公式侧 `portfolio-mgr.rhai` **从不产出绝对价格**（只有 stopLossPct/takeProfitPct），
    //     故 `decision_json` 里 `targetPrice`/`stopLoss` 键**根本不存在**（DB 实证）；
    //     ⚠️ **订正（2026-09-23，v77 第 4 项）：本条自本日起不再成立** —— 公式侧现已产出
    //     两键的绝对价格（`现价 × (1 ∓ 档位%)`），且优先于 LLM 值。v39 当时只补了
    //     prompt 约束 + R-204 留痕，**未修这条断链的源头**；2026-09-23 的 300642 样本正是
    //     该未修部分复现（LLM 合规留空 `targetPrice`、仍填 `stopLoss=17.9`，展示层于是
    //     同时显示「目标价 —」与「止损价 17.90」）。原文保留以存证 v39 的判断。
    //   · `decision.rs::merge_price_fields_from_llm` 的注释写着「缺键时用 trader 的 LLM 输出兜底，
    //     否则仪表盘目标价恒显示 —」，因公式侧**永远**缺键 ⇒ 这个「兜底」是**唯一来源**
    //     （3 个调用点 2323/2484/2639 全部命中）⇒ 仪表盘目标价 100% 来自 LLM 自填值；
    //   · LLM 在「持有」档把 targetPrice 填成 = currentPrice（其 decision_trail 原话
    //     「targetPrice=13.27（持有目标不追高）」），而当时 schema 描述只有两个字「目标价」。
    //   本版改动两处：
    //   ① trader 的 `system_prompt` 新增「价位字段方向语义」段 + `targetPrice`/`stopLoss`
    //      的 JSON schema 描述补齐方向（看多 > 现价 / 看空 < 现价 / 持有观望不填 / 禁止等于现价）；
    //   ② `portfolio-mgr.rhai` 新增检查 4（R-204）：`|targetPrice − currentPrice| / currentPrice < 0.5%`
    //      ⇒ 置 `trader_price_no_info`，记 `decision_trail`(INFO) + 推入 `data_gaps` +
    //      `kelly_note` 追加告警 + `oddsSource` 由「波动率fallback」改标「trader价格信号无效」。
    //      **刻意不置 `trader_data_valid = false`**：「无信息」≠「数据错误」，后者会触发
    //      R-202 降档 + 仓位封 10%，等于凭空压仓位（与 V59「单点失败只降级不全盘观望」同源）。
    //   ⚠️ 本版**不改变任何 action / 仓位数值**：等值时下方各判据（方向推断 / upside /
    //      odds_from_trader 均为严格 > <）本就两侧都不成立、已自动走 `odds_fallback`，
    //      本版只补**留痕与准确归因**（消除「静默失效」）。真正改变后续行为的只有 ①。
    // v38(2026-09-12): f5 估值因子「fallback 锚定置信度衰减」+ 决策卡文案格式化。
    //   · 新增 `input_mapping`：`valuation_dcf_anchor_is_fallback`
    //     ← `t-valuation.result.content.dcf.assumptions.is_fallback_anchor`
    //     （该布尔量由 `compute_dcf` 的 `FCF_FALLBACK_BASIS` 判定并随 assumptions 落库）。
    //   · 消费点 `portfolio-mgr.rhai`：命中 fallback 时 `f5 σ × 0.5`（只压置信度、不动权重）。
    //   · `portfolio-mgr.rhai` 的 reasoning 串改为格式化输出（原 `置信=45.74106680714534`）。
    //   ⚠️ **本版会改变决策数值**（与 v37 不同）：凡是 DCF 走了「当期FCF≤0 ⇒ 近5年报正净利
    //      均值×0.90」这条 fallback 的标的，f5 贡献腰斩 ⇒ posterior 向先验回缩。
    //      典型样本 603353：`dcf.upsidePct = −90.8`、f5 原 σ = −0.696（权重 0.21 为全因子
    //      最大、再乘 regime_mod 1.4）⇒ 该因子单独贡献 −0.1462，占净负贡献 Σ(σ·w) 的 87%。
    //      ⚠️ **但已实测反证「衰减能翻转 action」**：用 DB 存档 σ/w 正算复现 v37
    //      （posterior 残差 0.035pt）后扫描 —— k=0.5 → eff 0.3027→**0.3318（仍「减持」）**；
    //      即使 k=0（σ 完全中性化）→ 0.3610 **仍「减持」**；把所有 σ<0 因子全部归零，
    //      上限也只有 eff=**0.4327（「观望」）**，距 ACTION_HOLD_THRESHOLD(0.48) 还差 0.047。
    //      故本版定位是**置信度纠正**（阻止退化锚定冒充证据）+ 文案格式化，
    //      **不是 action 翻转器**；603353 的「减持」由 `prior=0.45(bear)` +
    //      `risk_bias=−0.08(高风险)` + 正向因子合计仅 0.157 共同决定。
    //   ⚠️ **需复核的边界**：若后续发现「当期真实 FCF」样本占比极低，则 0.5 这个系数
    //      等于对所有标的普遍降权，应改为面板可调参数（走「参数四道门」）再重新标定。
    // v40(2026-09-13): 决策矛盾/缺陷修复批（同日审计 AUDIT-xingye-601166-decision-2026-09-13.md）——
    //   ① `quality-gate` case 表达式由 `_value` 改写为 `value`：Rhai 拒绝一切下划线开头的
    //      标识符（`MalformedIdentifier`），旧写法**恒解析失败 ⇒ 恒回落 default_case**。
    //      DB 实证：`data-quality.result.grade="C"` 却 `matched_label="low-quality"`
    //      ⇒ C 级数据被误判为低质量，决策尾链被整体路由到保守的 quality-fallback
    //      （LLM 文案覆盖公式裁决）。执行器侧同时加了规范化（历史 `_value` 模板仍兼容），
    //      见 rt-workflow `switch_executor::normalize_case_expr` + 回归单测。
    //   ② `decision-explainer` 规则对照表重写：旧表 R-204 写作「零仓位修正」（实为
    //      「价格信号无信息量」）、R-200 写作「高风险风控否决」（实为「极高风险档位禁止
    //      持仓」），且列出的 R-403/R-404 **无任何生产者**（即「消费端列了、产出端从未发出」；
    //      2026-09-16 前其唯一出现处是 `divergence-log.rs` 的分类 match 分支，该文件因
    //      「双 schema 收敛到实体版」已删除 ⇒ 现全仓**零产出点**），
    //      又完全漏掉风控门的 R-206～R-210 ⇒ 解释官按错表臆造 rule_id（实证：summary
    //      写 R-206、rule_trace 写 R-204，同一份输入两个编号）。新表按「公式决策层 /
    //      组合风控门 / 技术面否决」分组，并显式声明「未在上下文中出现的编号不得写入
    //      rule_trace」。
    //   ③ 落库口径（Rust 侧）：`extract_decision_json` 改为取「本次运行**实际走到的链尾
    //      决策**」，优先级 = quality-fallback(D/F 档) > portfolio-risk-gate > portfolio-mgr。
    //      修复前只认 portfolio-mgr ⇒ 风控门的仓位修正被整体丢弃，与 rule-check /
    //      decision-explainer 报出的结论并存两套真相（DB 实证 601166：报告说「已按 R-206
    //      下调至 0%」、界面仍显示 8.85%）。
    //      ⚠️ **顺序关键**：风控门吃 portfolio-mgr 的公式结果，在 quality-gate **之前**
    //      就已产出；若把 gate 排到最高优先级，就会把 D/F 档「已被质量门替代」的公式决策
    //      重新抬成结论，直接推翻 V40 的 fallback 语义。故 quality-fallback 必须最优先
    //      （正常档它是 Skipped、无 result，不会误命中）。
    //   ④ `portfolio-mgr` f6（数据质量）因子改为**只惩罚、不给分**：旧式 `(dqi-50)/50` 把
    //      「数据完备」当成看多方向信号（C 级 61.3 分 → +0.226 正贡献 × 权重 0.15），属
    //      类别错误 —— 数据质量度量的是**证据可靠度**，不是标的涨跌方向。改为 dqi≥50 → 0
    //      （够用即中性）、dqi<50 → 线性负分（越缺越扣）。
    //      ⚠️ 会**普遍小幅下移** posterior（凡 dqi>50 的标的）。若确需「高质量数据略微加分」
    //      的建模意图，应改为调制 confidence 而非 posterior 方向。
    //   ⑤ `portfolio-mgr` 风险分类「真困境」条件（C）的修正：
    //      ① ~~A 金融业豁免负债率判据~~ —— **v51 已删除**：按行业归属打标签是错的。
    //        一家 ROE<0 且在收缩的银行正是最该顶格的标的，白名单却把真困境一起放行；
    //        且行业口径一变判据即整体失效（实测 stock_sector 是**门类级** "金融"）。
    //      ② 极高风险硬规则追加「真困境」第三条件（ROE<0 亏损 或 营收增速 <−10%），
    //        防单一指标顶格。**这是唯一保留、也是唯一需要的判据** —— 它观测的是
    //        事实（真在亏损 / 真在萎缩），与行业名无关，故天然覆盖全部行业。
    //      ⚠️ 会改变决策数值：601166 由「极高风险 → 减持」变为「中风险 → 持有」
    //      （DB 存档画像 91.6/−0.3/4.8/21.1/8.7/0.399 已正算复现旧判据的「极高风险」）。
    //   ⑥ posterior 一物三义（原始后验 / 生效后验 / 展示值）⇒ 新增 `posteriorRaw` 字段，
    //      并把 reasoning 串改为同时标注「先验=… / 后验=… / 生效后验=…」；
    //      `regime-weights.rhai` 的输出字段 `prior` 更名 `regime_confidence`（该值实为
    //      市况**分类置信度**，与 portfolio-mgr 的先验 0.45 同名不同义 —— 铁律 25）。
    //   ⑦ `core.rs` 退出紧迫度加 `has_holding` 前置：空仓股不再产出「减持 60 分 / 观望 30 分」
    //      这类无持仓可执行的紧迫度（「零仓位」不是需要紧急退出的证据）。
    //   ⚠️ **本版会改变决策数值**：①③④⑤⑦ 均影响最终 action/positionPct，方向是
    //      「解除被误顶格 / 被误降级的情形」—— 此前被 quality-fallback 误接管、以及被
    //      「金融业恒极高风险」压在保守档的标的，将回到公式链结果（仍受风控门约束）。
    // v41(2026-09-13): 三一重工 600031 复核批（AUDIT-sany-600031-decision-2026-09-13.md）——
    //   本版**只改展示串与文档，不改任何决策数值**：
    //   ① `portfolio-mgr` reasoning 的 `生效后验` 除数笔误：`*100/10` 把 0.54 显示成 5.4
    //      （与紧邻的 `后验=0.54` 不同量纲，DB 实证 600031）。改为 `*100/100`。
    //   ② `computation_logs` 的 `sig` 同型笔误（上一行笔误的来源）：`avg_signal=0.16`
    //      显示成 `sig=1.6`，而同一次输出的 `evidence.avgSignal` 是 0.16 ⇒ 同量两刻度。
    //      决策解释官会读本串做归因，刻度不一致会让它误判信号强度（铁律 15）。
    //   ③ 对 v40 ③ 的补充说明：落库优先级「链尾优先」**只有在引擎修好 Switch 默认分支
    //      之后才是正确的**。此前 `quality-gate` 的默认分支会被无条件执行（见 rt-workflow
    //      `dag_store::skip_disabled_branch_nodes` 的 2026-09-13 扩展），于是 A/B/C 档下
    //      `quality-fallback` 照样产出 result，「链尾优先」就误取了 LLM 兜底 ——
    //      DB 实证 600031：界面 `增持 11.5%/中风险`、落库 `减持 5%/高风险`。引擎修复后
    //      正常档的 quality-fallback 为 Skipped（无 result），优先级链才与真实路由一致。
    // v41(2026-09-14): 数据质量等级阈值统一 —— portfolio-mgr 不再按 dqi_score 数值
    //   自行分档（私有阈值 90/75/60 与 85/65），改为消费 data-quality.rhai 输出的
    //   grade 字符串。背景：同一个 dqi_score 被四处不同阈值判成不同字母等级，
    //   其中 grade="C"（45≤score<65）的股票在置信度上限那套里按 D 级限额，
    //   用户界面显示"C 级"却被按 D 级限流。现阈值只保留 data-quality.rhai 一份。
    //   配套改动：① `dqi_grade` 映射新增到 portfolio-mgr 的 input_mapping；
    //             ② portfolio-mgr.rhai 的 confidence_quality_cap / dqi_collapsed /
    //                dqi_a_level / dqi_high_quality 四处分档改读 grade_str；
    //             ③ grade 缺失时置信度上限由 fail-open 的 99.9 改为最保守的 60；
    //             ④ data-quality.rhai 的 report_quality 补齐至满量程 100
    //                （原各维度上限合计仅 80，使 score 天花板被钉在 93、A 级近乎不可达）。
    //   注意：④ 会普遍上移历史 grade 分布，属口径纠正而非放宽标准。
    // v42(2026-09-14): 决策 action 语义收敛（P0/P1）——
    //   ① portfolio-mgr.rhai 的 `f7_free_action` 阈值改为复用 ACTION_* 命名常量
    //      （原先硬编码了第二份 0.63/0.53/0.48/0.38/0.30，参数覆盖即分叉）；
    //   ② portfolio-mgr.rhai 输出新增正交字段 `positionState`
    //      （EMPTY/OPENING/HOLDING/TRIMMING），把「持有 vs 观望」从靠 positionPct
    //      互改的隐式关系显式化（action 取值本身保持不变，行为兼容）；
    //   ③ portfolio-mgr.rhai 的 Rhai 异常兜底不再输出 action="观望"，改用显式
    //      缺失哨兵「数据缺失」—— 脚本崩了不等于判断为观望。
    //   ④ trader.md 强化 verdict 与 action 的字段边界（verdict 不得写进 action）。
    //   注意：③ 会让异常路径的 decision_action 由「观望」变为「数据缺失」，
    //   前端按「数据缺失」渲染，属语义纠正而非展示回退。
    // v43(2026-09-14): 移除后端「观望 ⇄ 持有」互改（两轴正交化的第二步）——
    //   ① portfolio-mgr.rhai 试探仓块不再把 base_action 由「观望」改写为「持有」；
    //   ② 「position_pct<=0 ⇒ action 降级为观望」分支删除；
    //   ③ 「final_action==观望 ⇒ position_pct=0」分支删除（否则试探仓被静默清零）。
    //   语义变化：action 只承载方向强度，不再由仓位改写；落库的 action 保真。
    //   展示层仍按 positionState / positionPct 派生「持有 / 观望」文案（职责未变）。
    //   连带影响：空仓看多的记录 action 由「观望」变为「买入」，会创建价格告警
    //   （原被 is_no_action 过滤）；回测 was_correct 判定对象同步改变。
    // v45(2026-09-14): 新增「仿真验证」节点（sim-verify）—— **图上呈现环节，执行在落库之后**。
    //   ① 动机：仿真此前完全在 DAG 之外（跑完不落库、需在页面手点）；而决策链上的
    //      `sim_*`（S-501~503）只是 K 线统计代理，并非真仿真。
    //   ② 形态：节点定义保留在模板中（位置紧随 store-result），但 **enabled=false
    //      且不连任何边** ⇒ 图上可见、不参与调度、不占工作流执行时长。
    //      两条必须成对出现：孤立会让它被当成就绪节点立即执行；保留出边则会让
    //      store-result 永久 Pending（disabled 节点不进 done_or_skipped 集）。
    //   ③ 真执行：决策落库后由挂钩 `stock_workflow/sim_hook.rs` 触发（完整分析在
    //      `core.rs` 的挂载点，重跑决策在 `rerun_decision` —— 两者共用同一函数），
    //      内部调用 `market_sim_service::run_mc_core` 单一实现，结果写回 blackboard_snapshot
    //      的 `sim-verify` 键并 emit `simulation-ready` 就绪事件。
    //      节点自带的脚本 `sim-verify.rhai`（→ 宿主函数 `sim_run_mc`）在本节点被
    //      `enabled=false` 的前提下**不会执行**，仅在有人手动打开该节点时才走 ——
    //      详见该脚本头部说明与 `PLAN-market-sim-value.md` 9.9。
    //   ④ 不阻滞决策：在所有会改决策的节点（portfolio-mgr → portfolio-risk-gate
    //      → quality-gate）之后、且**落库之后**才跑；只产出补充信息，不产出
    //      action / positionPct / confidence ⇒ 不可能回灌决策。
    //   前端消费：股票分析页「模拟仿真」tab 顶部区块自动展示，无需手点运行。
    // v46(2026-09-14): `sim-verify.rhai` 头部说明重写（原文写的是「插在
    //   decision-explainer 之后、是链上最末节点」，与落点乙实际形态矛盾）。
    //   脚本随节点体 `include_str!` 存入快照模板 ⇒ 按纪律升版，保证 DB 里的
    //   脚本文本与仓库一致（本次为注释级变更，无行为差异）。
    // v47(2026-09-14): 强制重新种子化（用户裁决「照常重新种子化模板」）——
    //   **本版无任何模板内容变更**，是一次纯版本推进。理由：`sim-verify.rhai`
    //   的文本在 v46 声明之后**仍被再次修改**（实测 mtime：脚本 06:07 > 本文件 04:25），
    //   而该脚本经 `include_str!("../sim-verify.rhai")` 嵌入节点体 ⇒ 若 v46 的种子
    //   恰好跑在这两次写之间，DB 里存的**是旧脚本文本**，而版本门
    //   `existing.version >= TEMPLATE_VERSION` 会让它此后**永远不再更新**
    //   （这正是「改完不生效」的典型形态：仓库对、DB 错，且没有任何提示）。
    //   升版是唯一能保证 DB 与仓库一致的手段。
    //   ⚠️ 代价（已与用户确认接受）：本版会**覆盖 DB 中对该模板 nodes/edges 的任何
    //   手工调整**，仅 `variables` 的自定义值在升级前被保留（见下方 `old_variables`）。
    // v48(2026-09-14): 多空辩论 **3 轮 → 1 轮**（用户裁决：「max_rounds 改 1」）。
    //   依据 `AUDIT-601166-chain-break-2026-09-14.md`：当前「3 轮辩论」本来就是假象 ——
    //     ① `build_debate_body_dispatch` 对第 2+ 轮已 `Completed` 的 body 节点
    //        **直接复用上次对象返回**（不重跑 LLM），收敛检测拿同一对象比相似度 ⇒
    //        恒 1.0 ⇒ 必然判「已收敛」，`total_rounds` 恒为 2；
    //     ② 辩手 prompt **不消费** `__debate_history__` / `__debate_round__` ⇒
    //        即便重跑也零信息增量（2026-09-08 已实证 18 次调用里 12 次纯浪费）。
    //   而链式 `contextSources` 累积使**链尾请求体全局最大**（601166 实测 85.8KB），
    //   直接决定 provider 侧 `header_timeout = 10 + ⌊body/64KB⌋×5` = **15s** 这一
    //   最严档位 ⇒ 链尾辩手是结构上最脆弱的一环。
    //   改 1 轮后：debater_steps = [bull-r1, bear-r1]（4 个纯复制的轮次节点及其边
    //   由下方循环自动不再生成），链尾请求体从 85.8KB 降到 ~33KB（回到 10s 档），
    //   同时省掉一次必败的 ~45s 重跑。**语义不降级**：被删的 bull-r2/bear-r2/
    //   bull-r3/bear-r3 本来就只是首轮输出的复制品。
    //   变量 `debate_rounds` 保留（前端「多空辩论轮数」仍可见）但本版**强制覆写为 1**
    //   —— 因为 `merge_variable_values` 会让 DB 旧值（3）覆盖新默认值，不覆写就等于没改。
    // v49(2026-09-14): **修复 v48 的悬空入边**（v48 的回归，必须升版才生效）。
    //   缺陷：v48 把轮数改成 1 后 `bull-r2/bear-r2/bull-r3/bear-r3` 不再生成，
    //   但 `t-scoring` 的入边仍写死 `edge("e-bear-r3-t-scoring", "bear-r3", "t-scoring")`
    //   ⇒ 悬空入边 ⇒ `create_workflow` **启动期硬失败**（不是运行期降级）：
    //     `创建工作流失败: Node 't-scoring' depends on non-existent 'bear-r3'`
    //   用户侧表现为「启动失败」，整条链连建模都过不去。
    //
    //   ⚠️ **为什么必须升版、不能只改代码**：报错发生在 `ensure_stock_analysis_experts_seeded`
    //   **之后**（种子先跑、`create_workflow` 后跑）⇒ v48 的坏模板**已经落库**。
    //   而本函数开头的版本门是 `existing.version >= TEMPLATE_VERSION`：
    //   若仍写 48，DB 里 version=48 会让种子**直接跳过** ⇒ 仓库已修、DB 仍是坏的，
    //   用户重跑会**一字不差地再撞同一个错**。这是「改完不生效」的教科书形态。
    //
    //   修复内容：
    //     ① `e-bear-r3-t-scoring` 的源节点与边名改为 `format!("...{debate_max_rounds}")` 派生；
    //     ② 同批清掉两处「撒谎的边名」`e-bull-r3-p-risk-assess` / `e-bear-r3-debate-convergence`
    //        （源早已参数化，只有名字写死 —— 不影响调度，但会误导排查）。
    //   守护测试（防再犯）：`seed_consistency_tests::debater_round_refs_are_parameterized`
    //   + 负控 `round_literal_scanner_detects_known_bad_forms`：扫描 seed 源码里
    //   **以字面量形式出现在边 source/target 位置**的 `bull-rN`/`bear-rN`，两种写法
    //   （`edge()` 闭包调用、`WorkflowEdge { source: "x".into() }`）都覆盖，零容忍。
    // v50(2026-09-14): 接线 **DCF 模型适用性门**（配套改动在 `astock-data` 与
    //   `portfolio-mgr.rhai`）。
    //   ⚠️ 自述更正（v51 期间发现）：原文写「`portfolio-mgr.rhai` 是编译产物、不需
    //      升版」—— **这是错的**。该文件经 `include_str!` 被写进本模板 `portfolio-mgr`
    //      CodeNode 的 `code` 字段（见下方 `let pm_code = include_str!`），是 DB 模板
    //      内容的一部分 ⇒ 改 rhai 公式**必须**升 TEMPLATE_VERSION，否则版本门让种子
    //      跳过、DB 里仍是旧公式（「改完不生效」的生成物陷阱）。
    //   背景：601166 估值面板写「内在价值 40.17–49.26 元（低估 143%）」，决策卡写
    //   「观望 + 0% 仓位」—— 两个结论并列且无任何解释，用户直接质问「天方夜谭」。
    //   根因：拿「近 5 年净利均值 × 0.90 = 730.5 亿」当**自由现金流**给银行折现，
    //   而银行没有「企业自由现金流」概念 ⇒ 该数值没有经济含义，不是「低估」。
    //   本版新增两条 `input_mapping`：
    //     · `valuation_dcf_applicable`        ← `dcf.assumptions.applicable`
    //     · `valuation_dcf_inapplicable_reason` ← `dcf.assumptions.inapplicable_reason`
    //   消费者 `portfolio-mgr.rhai`（V77）：`applicable == false` 时把 DCF 这一腿
    //   **整体剔除**（不是衰减 —— 衰减=承认它还有信息量），并由 graham 腿独立承担
    //   估值维度；两腿都不可用时 `f5_weight = 0`（「无法判断」既不加分也不扣分）。
    //   同时在 `reasoning` 与输出 `valuationApplicability` 里给出剔除原因，
    //   让「估值说低估、决策说不买」之间有可读的因果解释。
    //   ⚠️ 判据锚定**数据形态**（杠杆畸高 / FCF 与净利背离 / 负增长且终值独裁），
    //   **不锚定行业标签** ⇒ 银行、保险、券商、地产、重资产周期自动全覆盖，
    //   无需维护任何白名单（这是「不要只修银行股」的落地方式）。
    // v51(2026-09-14): 删除 `portfolio-mgr.rhai` 风险分类中的**行业白名单豁免**
    //   （判据只按数据形态、不按行业归属）。属**公式内容变更** ⇒ 必须升版，才能让
    //   DB 模板里的 `portfolio-mgr` CodeNode.code 被重新写入。
    //   配套：`analysis-engine/portfolio_formula.rs::classify_risk`（参考实现）同步
    //   删除 `sector` 形参与 `is_financial_sector`，避免两处漂移。
    // v52(2026-09-19): 核心方法论入决策 —— `portfolio-mgr.rhai` 新增 **f13 瓶颈因子**，
    //   把「瓶颈掘金」三力合成分接入 posterior 融合。此前 `serenity_context` 虽已由
    //   本模板的 input_mapping 注入（`("serenity_context", "serenity_context")`），
    //   但脚本侧**零引用** ⇒ 恒不参与决策，只能作 LLM 上下文（详见 f13 段注释）。
    //   属**公式内容变更** ⇒ 必须升版，否则版本门让种子跳过、DB 模板里
    //   `portfolio-mgr` CodeNode.code 仍是旧公式（即下方 v50 注释所记的
    //   「改完不生效」生成物陷阱）。
    //   同批改动：A3 —— 给 V47 空头否决补「输入可信」前置条件（联立 `trader_data_valid`），
    //   并新增 `R-201-SUPPRESSED` 留痕（异常输入被抑制的否决必须可见）。
    //   ⚠️ 代价同前：本版会覆盖 DB 中对该模板 nodes/edges 的手工调整
    //   （`variables` 的自定义值在升级前保留）。
    // v53(2026-09-19): 阶段一 A2 —— `detect_earnings_surprise` 的 ToolDef 新增入参
    //   `consensus_eps_is_estimated`，把「这个一致预期 EPS 是估算值」这一事实**递到工具
    //   面前**，使它在算「超预期」之前能先拒判。原先拿「板块常数反推」的估算 EPS 当真
    //   EPS 用，会产出**假超预期**信号并一路流进决策。
    //   属**工具定义内容变更** ⇒ 必须升版：ToolDef 的入参表与 description 同样是 DB 模板
    //   内容的一部分，而版本门是 `existing.version >= TEMPLATE_VERSION`
    //   （见本函数开头的版本门 —— 不是 `==`，所以留在旧号上会**永久跳过**种子），DB 里将仍是**没有
    //   这个入参**的旧声明 ⇒ Agent 无从填写、`detect_earnings` 永远走不到拒判分支。
    //   这就是 v50 注释所记的「改完不生效」生成物陷阱的又一次同型复发。
    //   配套（须同批生效，缺一则退化为半截修复）：
    //     · `astock-data`：`ConsensusEPS` 新增 `is_estimated` / `estimate_source` 并在
    //       所有构造点显式赋值（财报值 = false、板块常数兜底 = true）；
    //     · `tools::finance::detect_earnings`：读标志，为 true 时拒判超预期
    //       （返回 `surprise_pct = null` + `预期基准不可靠`，而非编一个数）。
    //   若只升版不落地消费端 ⇒ 「声明了入参但没人消费」；只落地消费端不升版 ⇒
    //   「有拒判分支但拿不到标志」。两种半截状态都必须避免。
    // v54(2026-09-19): 阶段一 ② —— 删除 `portfolio-mgr` 节点的 6 条**悬空**
    //   input_mapping（注入了 scope 但脚本全文零引用 ⇒ 白注入）：
    //     `risk_gross_margin` / `trader_action` / `trader_data_gaps` /
    //     `trader_position_pct` / `trader_stop_loss_pct` / `trader_take_profit_pct`。
    //   依据：`scripts/audit-inject-coverage.mjs` ② 段零命中（手写 target 61 → 55）；
    //   已排除「靠动态键读取」——脚本内 `scope.get` / `scope.iter` / `scope.contains` /
    //   `get_value` / `eval(` **全 0 命中**。对照组证明删除有区分度：邻居
    //   `trader_direction`(8) / `trader_confidence`(8) / `trader_target_price`(34) /
    //   `trader_stop_loss`(21) / `risk_sharpe`(15) / `risk_roe`(8) / `risk_debt_ratio`(2)
    //   均被真读，故保留。
    //   属**节点 input_mapping 内容变更** ⇒ 必须升版（理由同 v53：版本门是 `>=`
    //   而非 `==`，留在旧号上 DB 里那 6 条白注入会永久留着）。
    //   ⚠ 勿误删 `trader_action` 的**同名映射**：`reflection-comparator` 节点另有一条
    //   （`mod.rs` 内的 `("trader_action", "sub-analysis.trader.content.action")`，
    //   源路径与本节不同），`reflection-comparator.rhai` 在 `is_unit(trader_action)` 处
    //   真读它 ⇒ 独立映射、本版不动。此处不写行号，认字符串。
    // v55(2026-09-20): `cls-risk-level` 节点由 **LlmClassifierNode 改为 CodeNode（Rhai）**。
    //   原节点每轮运行耗 4.8s（真库 76 次调用均值），并曾 5 次撞 60s 超时、
    //   使整条运行落 `partially_completed`；而其 prompt 通篇是**确定性阈值规则**
    //   （并明写「不要计算综合评分，直接按规则判定」）⇒ 等于用 LLM 跑一个 if-else。
    //   改为 Rhai 后 <10ms、零 token、零超时（同 `data-quality` 已验证的 LLM→Rhai 路径）。
    //   **口径 1:1 照抄原 prompt**：阈值、档位、优先级均未调整；原 prompt 原文完整保留在
    //   `risk-level.rhai` 头部注释作为口径权威源。配套改动：
    //     · 新增 `src/commands/risk-level.rhai`（`include_str!` 嵌入节点 code 字段）
    //     · `portfolio-mgr` 的 input_mapping 源路径 `cls-risk-level.category`
    //       → `cls-risk-level.result.category`（CodeNode 的输出被包在 `result` 里）
    //   属**节点类型 + 节点内容变更** ⇒ 必须升版（理由同 v53/v54：版本门是 `>=` 而非
    //   `==`，留在 54 上 DB 里那个 LLM 节点会永久留着，新脚本一字不落地不生效）。
    //   ⚠️ 影子比对（真库 59 条可比对样本，`output/rhai-shadow-report.txt`）：
    //   档位一致率 64.4%，posterior 位移**均值 +0.15pp**（双向抵消，无系统性放松），
    //   最大上抬 +4.63pp / 最大下压 -3.60pp。分歧主因是**原 LLM 判定自身不一致**
    //   （实证：`debt=91.6,growth=-0.3` 判高风险，`debt=92.2,growth=+8.3` 判极高风险
    //   —— 后者负债率更高、增长为正，档位反而更严）。即「换确定性实现」本身就消除了
    //   这部分随机性，属改造目的而非回归。
    //   已知未实现项（数据不可得，与原 LLM 版同等受限，非本次引入）：prompt 的
    //   「极高风险①ST/*ST/退市股」与「低风险③无ST/重大负面公告」两条判据 ——
    //   t-risk 输出不含 ST 标记与公告数据，原 LLM 同样拿不到。启用需在
    //   `astock-data` 的 compute_portfolio_risk 补 `isST` 字段（属改口径，待批）。
    //   ⚠️ 代价同前：本版会覆盖 DB 中对该模板 nodes/edges 的手工调整
    //   （`variables` 的自定义值在升级前保留）。
    //   ⚠️⚠️ **升版判据是「严格大于 DB 现值」，不是「比上一版 +1」** —— 2026-09-20 实测：
    //   真库 `workflow_templates` 里 `stock-analysis` 的 `version` 早已是 **58**
    //   （`updated_at` = 2026-09-18 06:04:22），而本常量按 v52→v53→v54→v55 的
    //   注释序列只走到 **55** ⇒ `existing.version (58) >= TEMPLATE_VERSION (55)` 成立
    //   ⇒ 种子走版本门里的 `return Ok(())` **直接跳过** ⇒ 本节全部改动（含
    //   `risk-level.rhai` 经 `include_str!` 嵌入的 code 字段）**一字不落库**，
    //   节点在 DB 里仍是旧的 `LlmClassifierNode`。
    //   这是「改完不生效」的**第二种形态**：不是常量没升，而是**升得不够高**
    //   （同族：A 组「改生成物 = 假修复」、`PLAN-declarative-schema-sync` 的版本门陷阱）。
    //   ⇒ 取值 **59**（严格 > 58）。核验命令（只读）：
    //     `SELECT id, name, version FROM workflow_templates WHERE id='stock-analysis';`
    //
    //   ⚠️⚠️ **2026-09-20 二次取证把爆炸半径扩大了**：查 `workflow_template_versions`
    //   快照表发现最后一次成功落库是 09-18 06:04:22（写 `stock-analysis_v57` 快照
    //   ⇒ DB 变 58），此后**一次都没有** ⇒ 卡在门口的不是 v55 一批，而是
    //   **v53 + v54 + v55 三批**（逐条实测见下）：
    //     · v53：`detect_earnings_surprise` 的 `consensus_eps_is_estimated` 入参 ——
    //       DB 节点 JSON 里该串 0 命中；而配套的 Rust 侧（`astock-data` 的
    //       `ConsensusEPS.is_estimated` + `tools::finance::detect_earnings` 拒判）
    //       **已随二进制生效** ⇒ 恰是 v53 注释自己警告的
    //       「有拒判分支但拿不到标志」半截状态；
    //     · v54：`portfolio-mgr` 的 6 条悬空 `input_mapping` 仍在 DB
    //       （实测 `trader_action` / `trader_data_gaps` / `trader_position_pct` /
    //        `trader_stop_loss_pct` / `trader_take_profit_pct` 均未删）；
    //     · v55：`cls-risk-level` 在 DB 里仍是 `llmClassifier`（旧 prompt 原文
    //       「你是专业风险分析师…」完整在场），且 DB 的 7 个 `code` 节点里没有它。
    //   ⇒ 本次升到 59 会**一次性落库三批改动**，不只是本节的 Rhai 下沉。
    //
    //   常量本体已提到模块级（见文件顶部 `pub(crate) const TEMPLATE_VERSION`），
    //   供 `mod.rs` 末尾的 `version_gate_tests` 直接引用。
    //
    // v60(2026-09-20): 四项产品语义裁决落地，其中两项触及本模板内容。
    //   ① **`risk-level.rhai` 降级兜底档由「中风险」改「高风险」（保守档）**：
    //      - 下游 `portfolio-mgr` 只用 `category` 算 f4 因子（`portfolio-mgr.rhai`
    //        的 switch：中 -0.05 / 高 -0.25），**并不消费 `degraded`** ⇒ 原中性
    //        兜底在决策链上等价于「我判定它是中风险」，把「没判定」伪装成「已判定」；
    //      - 本档表征的是**未知**，未知作用在资金方向上不可逆 ⇒ 取代价较小的一侧；
    //      - **存量影响面 = 0**：真库 59 条可比对样本里 `degraded` 零命中
    //        （`output/rhai-shadow-report.txt` 中「LLM 档缺失(f4=0.0)」的 4 条其
    //        vol 均存在）⇒ 只影响未来「风险画像数据缺失」的异常场景，**不改写任何
    //        既有结论**。
    //      ⚠️ 只改 `degraded` 分支：**规则全不命中**那一支仍是「中风险」——那一档是
    //      prompt 明写的真判定（「A股大多数股票应落在中风险档」），不是降级。
    //      `degraded` / `warnings` 输出字段不变（诊断面板与复盘消费）。
    //   ② **节点 config 删除 `modelRole` 装饰键**：该键 Rust 侧**零读取**
    //      （唯一命中是 serde alias 反序列化进 `AgentNodeConfig.model_role` 字段，
    //      而该字段本身无任何消费点），且面板给的词表（`quick_think`/`deep_think`）
    //      与 DB 存量值（`stock-analyst`/`debater`/…）**互不相交**；角色的正式载体是
    //      `agent_profile_id`（字段注释明写「唯一标识角色的方式」）⇒ 属被取代的旧机制
    //      残留，删除止血。DB 存量键不迁移（`AgentNodeConfig` 无 `deny_unknown_fields`，
    //      serde 忽略未知键，不影响反序列化）。
    //   属**节点 code + 节点 config 内容变更** ⇒ 必须升版（理由同 v53/v54/v55：
    //   版本门是 `>=` 而非 `==`）。
    //   ⚠️ 代价同前：本版会覆盖 DB 中对该模板 nodes/edges 的手工调整
    //   （`variables` 的自定义值在升级前保留）。
    //   ⚠️ 取值 **60**（严格 > DB 现值 58，判据同 v55 节：不是「比上一版 +1」）。
    //   ⚠️ 同批（不依赖升版、立即生效的）还有：`dao` 的 `split_node_model` —— 让节点
    //   `config.model` 的 `providerId::modelId` 复合值真正拆出供应商（此前整串被当
    //   model id 直传 API），以及 `LlmClassifierPropertyPanel` 补齐模型/阈值/降级档控件。
    //
    // v61(2026-09-20): **f13 瓶颈因子权重由硬编码改走 `get_weight`（接 regime 白名单）**。
    //   产品裁决：待裁决项②「f13 权重硬编码 0.10 是否保留」→ 选「改走 get_weight」。
    //   改动落在**两个** `include_str!` 嵌入节点 `code` 的 rhai 文件：
    //     ① `portfolio-mgr.rhai`：`f13_weight = f13_default;`
    //        → `f13_weight = get_weight(f_weights, "bottleneck", f13_default);`
    //        （`f13_default = 0.10` 保留为 fallback，语义不变）
    //     ② `regime-weights.rhai`：`factors` 表新增 `bottleneck` 条目
    //        （base 0.10 / bull 0.9 / bear 1.2 / neutral 1.0 / volatile 1.3），
    //        并加入 `factor_names` 与 `consumed_names` 白名单。
    //
    //   ⚠️ **两处必须同批改，只改一侧都是空接**：
    //     · 只改 ① ⇒ `factor_weights` 里没有 `bottleneck`（白名单外 ⇒ 被丢进
    //       `unconsumed_suggestions`）⇒ `get_weight` 恒取 fallback 0.10，等价于没改；
    //     · 只改 ② ⇒ pm 仍读硬编码 ⇒ 该条目产出后无人消费，又是一处「死计算」
    //       （P1-F 契约当初清理的正是这一形态）。
    //   这类「两文件同改才算一次改动」的耦合，是 `include_str!` 之外还要靠本注释
    //   记录的根本原因 —— 编译器看得见语法错，看不见「两头没接上」。
    //
    //   存量影响面：base 0.10 × neutral 1.0 = 0.10 ⇒ **中性市况下与变更前逐值相同**，
    //   仅 bull/bear/volatile 三档产生位移（±0.01~0.03 权重，相对全因子总权重
    //   ≈1.49 为 0.7%~2.0%，不足以单独翻转档位；若需精确量化可用
    //   `output/rhai-shadow-report` 同型脚本做反事实扫描）。
    //   `max_weight` **不随之改**：它是「活跃因子**默认**权重之和」，f1/f2/f5/f9/f10
    //   走 get_weight 后同样用各自 default 计入，f13 保持一致 ⇒ 非瓶颈路径分母
    //   仍为 1.49、瓶颈路径仍为 1.59，`evidence_scale` / `weights_collapsed`
    //   口径**零回归**。
    //   属**节点 code 内容变更** ⇒ 必须升版（理由同 v53~v60：版本门是 `>=` 而非 `==`）。
    //   ⚠️ 取值 **61**（严格 > 上一常量 60；判据同 v55 节：不是「比上一版 +1」）。
    //   ⚠️ 代价同前：本版会覆盖 DB 中对该模板 nodes/edges 的手工调整
    //   （`variables` 的自定义值在升级前保留）。
    //
    // v69(2026-09-21): **reasoning 的结论名改走「展示档」** —— 修「挂角说观望、结论说持有」。
    //   实证（2026-09-21 13:52 的一条「持有 + 0% 仓位」记录）：历史卡挂角 Tag 显示
    //   「观望」，点开结论文本却是「决策=持有」，用户读到同一条记录内部两个名字。
    //   根因：V76 拆轴后**展示层**统一走 `resolveDisplayAction`（中性档 + EMPTY ⇒ 观望），
    //     而 `portfolio-mgr.rhai` 拼 reasoning 用的仍是**原始方向档** `final_action`
    //     ⇒ 同一份 JSON 里 `action`（原始轴）与 `reasoning`（人读文本）各自为政。
    //   改动落在 `portfolio-mgr.rhai`（`include_str!` 嵌入本模板 `portfolio-mgr` 节点）：
    //     · 新增 `display_action`：非中性档原样 / 中性档 + EMPTY ⇒ 观望 / 其余 ⇒ 持有
    //       —— 判据与前端 `resolveDisplayAction` **逐条对齐**，改一侧必须改另一侧；
    //     · `reasoning` 的 `决策=${final_action}` → `决策=${display_action}`。
    //     · `action` 字段**不动**：它是方向强度轴，前端双视角对比（公式 vs LLM）与
    //       「问 AI」prompt 都依赖原值，在此派生会掩盖真实分歧。
    //   ⚠️ 存量数据：已落库的历史行 reasoning 仍是旧文本，由前端
    //     `stock-analysis-utils::alignReasoningDecisionLabel` 在解析层对齐（幂等，
    //     只改开头第一个 `决策=X`；`风控否决:持有→观望` 这类**过程留痕**不动）。
    //   ⚠️ 曾登记为「未修」：`portfolio-risk-gate.rhai` 覆盖 `action` 时**不更新** reasoning
    //     的结论名（只追加 `| [风控门] ...`）。**现已修**：该门末尾调 `align_decision_label`
    //     把结论名对齐到最终方向档（v76 起目标档 = `final_action`），落库文本自洽。
    //   属**节点 code 内容变更** ⇒ 必须升版（理由同 v53~v61：版本门是 `>=` 而非 `==`）。
    //   ⚠️ 取值 **69**（严格 > 上一常量 68）。
    //   ⚠️ **上述派生已于 v76（2026-09-22）废除** —— 见文件顶部 `TEMPLATE_VERSION`
    //     的 v76 说明（循环判据：用本次建议仓位反推本次展示名）。本段保留为历史留痕，
    //     **不要**据此认为代码里还存在 `display_action`。
    //
    // v71(2026-09-21): **`data-quality.rhai` 逐节点诊断补 `report_quality`** ——
    //   修「10 个分析师的数据质量弹窗显示同一组数字」（用户质问「这是造假吗」）。
    //   根因：面板顶部四数（score / grade / good / degraded / gap）取自 data-quality 节点的
    //     **全局**输出对象，而该对象里根本没有 per-analyst 的 score/grade ⇒ 前端物理上
    //     渲染不出「本节点的等级」，10 张分析师卡片复用同一 modal（只换 name/expertId）。
    //   改动落在 `data-quality.rhai`（`include_str!` 嵌入本模板 `data-quality` 节点）：
    //     · `diag_for` 新增第 7 参 `rq`，返回 map 补 `"report_quality": rq`；
    //     · 10 处调用点各补本节点**已算好但此前被平均掉**的 `*_q`
    //       （`report_quality(text, conf)`，与全局 `report_quality_avg` 同源同口径）。
    //   ⚠️ 只补**事实量**，不派生 per-node 字母等级 —— 那会再造「两套同名等级」，
    //     正是 2026-09-14 才修掉的坑（见 `AnalystDataQualityModal.tsx` 头部注释）。
    //   属**节点 code 内容变更** ⇒ 必须升版（版本门是 `>=`，非 `==`）。
    //   ⚠️ 取值 **71**（严格 > DB 现值 70，2026-09-21 只读查询实测）。

    tracing::info!(
        "[stock_analysis_setup] seed_stock_analysis_workflow_template 开始: TEMPLATE_ID={TEMPLATE_ID}, TEMPLATE_VERSION={TEMPLATE_VERSION}"
    );

    // 升级前保留旧模板的变量自定义值，在函数体外声明以延长生命周期
    let mut old_variables: Option<String> = None;
    // D11（2026-09-22）：记录旧模板版本号，供「存量一次性迁移」判定（见下方 DCF force 处）。
    // `None` = 模板此前不存在（首次创建）⇒ 视为需要执行迁移。
    let mut previous_version: Option<i32> = None;

    if let Some(existing) =
        workflow_template::Entity::find_by_id(TEMPLATE_ID).one(db).await.map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL)
                .with_detail(format!("查询工作流模板失败: {e}"))
        })?
    {
        tracing::info!(
            "[stock_analysis_setup] 找到已有模板 v{}: nodes.len={}, edges.len={}",
            existing.version,
            existing.nodes.len(),
            existing.edges.len()
        );

        // 检查节点数据完整性：如果节点或边为空，即使版本号满足也需要强制重新种子化
        let nodes_empty =
            existing.nodes.is_empty() || existing.nodes == "[]" || existing.nodes == "null";
        let edges_empty =
            existing.edges.is_empty() || existing.edges == "[]" || existing.edges == "null";

        if nodes_empty || edges_empty {
            tracing::warn!(
                "[stock_analysis_setup] 模板 v{} 节点/边数据为空 (nodes_empty={}, edges_empty={})，强制重新种子化",
                existing.version,
                nodes_empty,
                edges_empty
            );
        } else if existing.version >= TEMPLATE_VERSION {
            tracing::info!(
                "[stock_analysis_setup] 模板已是最新版本 v{}，跳过种子化 (nodes={}, edges={})",
                existing.version,
                existing.nodes.len(),
                existing.edges.len()
            );
            return Ok(());
        }

        tracing::info!(
            "[stock_analysis_setup] 更新股票分析工作流模板 v{} → v{TEMPLATE_VERSION}",
            existing.version
        );
        // 写版本快照（复用 update_workflow_template 的 snapshot 机制）
        let ver_id = format!("{}_v{}", TEMPLATE_ID, existing.version);
        if axagent_entities::workflow_template_version::Entity::find_by_id(&ver_id)
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("查重失败: {e}"))
            })?
            .is_none()
        {
            use sea_orm::ActiveModelTrait;
            let snapshot = axagent_entities::workflow_template_version::ActiveModel {
                id: Set(ver_id.clone()),
                template_id: Set(TEMPLATE_ID.to_string()),
                name: Set(existing.name.clone()),
                description: Set(existing.description.clone()),
                icon: Set(existing.icon.clone()),
                tags: Set(existing.tags.clone()),
                version: Set(existing.version),
                is_preset: Set(existing.is_preset),
                is_editable: Set(existing.is_editable),
                is_public: Set(existing.is_public),
                trigger_config: Set(existing.trigger_config.clone()),
                nodes: Set(existing.nodes.clone()),
                edges: Set(existing.edges.clone()),
                input_schema: Set(existing.input_schema.clone()),
                output_schema: Set(existing.output_schema.clone()),
                variables: Set(existing.variables.clone()),
                error_config: Set(existing.error_config.clone()),
                created_at: Set(chrono::Utc::now().timestamp_millis()),
            };
            snapshot.insert(db).await.map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("写入版本快照失败: {e}"))
            })?;
            tracing::info!("[stock_analysis_setup] 旧版本快照已保存: {ver_id}");
        }
        old_variables = existing.variables.clone();
        previous_version = Some(existing.version);
        // ⚠️ 2026-09-22 更正：落库其实是 DELETE（下方 `delete_by_id`）+ INSERT，**不是** UPDATE；
        // 变量 `value` 的「保留用户自定义值」由 `merge_variable_values`（:4641 调用）逐名完成。
        // 原注释「用 UPDATE 替代 DELETE」与实现不符，会误导后续改动。
    } else {
        tracing::info!("[stock_analysis_setup] 模板 {TEMPLATE_ID} 不存在，将创建新模板");
    }

    let now = chrono::Utc::now().timestamp_millis();

    let tool_node = |id: &str,
                     title: &str,
                     tool_name: &str,
                     output_var: &str,
                     arg_key: &str,
                     extra: &[(&str, &str)],
                     parent_id: Option<&str>,
                     x: f64,
                     y: f64|
     -> WorkflowNode {
        let mut input_mapping = std::collections::HashMap::new();
        input_mapping.insert(arg_key.to_string(), "stock_code".to_string());
        // C2 路径 Z(2026-09-12): 额外「工具参数名 → 变量名」映射。
        //
        // 用途：把设置面板的 `value_dcf_*` 变量注入 `compute_valuation` 的**扁平参数**
        // （`dcf_growth_rate` / `dcf_perpetual_rate` / `dcf_discount_rate`，
        //  百分数口径，后端单点 `/100` 换算）。
        //
        // 为什么不用 `valuation_config` object：`ToolNodeConfig.input_mapping` 是
        // `HashMap<String, String>`，dispatcher 把 value 当**变量名**查
        // （`crates/rt-workflow/src/work_engine/dispatcher.rs:463-466`），**无法构造嵌套 object** —— 而嵌套 object
        // 会在 `serde_json::from_value::<ValuationConfig>` 失败后被 `.ok()` 静默吞掉，
        // 表现为「改了设置但毫无效果」。这是本函数存在的全部理由。
        for (arg_name, var_name) in extra {
            input_mapping.insert((*arg_name).to_string(), (*var_name).to_string());
        }
        WorkflowNode::Tool(ToolNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("获取数据: {tool_name}")),
                position: Position { x, y },
                retry: RetryConfig { enabled: true, max_retries: 2, ..Default::default() },
                timeout: None, // 继承 RunOptions.tool_timeout（来自 tool_timeout_secs 设置）,用户可在面板中控制
                enabled: true,
                parent_id: parent_id.map(String::from),
                compensation: None,
                continue_on_fail: false,
            },
            config: ToolNodeConfig {
                tool_name: tool_name.into(),
                input_mapping,
                output_var: output_var.into(),
            },
        })
    };

    // 无参工具节点（`input_mapping` 为空）。
    //
    // 为什么需要单独一个闭包：`tool_node` 会**无条件**插入
    // `input_mapping[arg_key] = "stock_code"`，对 `get_index_quotes`
    // （MCP `inputSchema.properties = {}`，本身不接收任何参数）虽然「多余参数会被工具实现
    // 忽略」（`dispatch_tool` 的 `get_index_quotes` 分支不读 arguments），功能上无害，
    // 但会在节点上留下「该工具需要 stock_code」的**错误声明** ——
    // 这正是本次审计反复遇到的「映射声明与真实契约不符」形态（见记忆铁律 6）。
    // 故显式建一个不注入任何映射的变体，而不是靠「反正会被忽略」蒙过去。
    let tool_node_noarg =
        |id: &str, title: &str, tool_name: &str, x: f64, y: f64| -> WorkflowNode {
            WorkflowNode::Tool(ToolNode {
                base: WorkflowNodeBase {
                    id: id.into(),
                    title: title.into(),
                    description: Some(format!("获取数据: {tool_name}")),
                    position: Position { x, y },
                    retry: RetryConfig { enabled: true, max_retries: 2, ..Default::default() },
                    timeout: None,
                    enabled: true,
                    parent_id: None,
                    compensation: None,
                    continue_on_fail: false,
                },
                config: ToolNodeConfig {
                    tool_name: tool_name.into(),
                    input_mapping: std::collections::HashMap::new(),
                    output_var: id.into(),
                },
            })
        };

    // ── ToolDef 参数 schema 辅助构建 ──
    fn sc_prop(desc: &str) -> JsonSchemaProperty {
        JsonSchemaProperty {
            schema_type: "string".into(),
            description: Some(desc.into()),
            default: None,
            enum_values: None,
            format: None,
        }
    }
    fn sc_prop_default(desc: &str, default: &str) -> JsonSchemaProperty {
        JsonSchemaProperty {
            schema_type: "string".into(),
            description: Some(desc.into()),
            default: Some(serde_json::Value::String(default.into())),
            enum_values: None,
            format: None,
        }
    }
    fn int_prop(desc: &str, default: Option<i64>) -> JsonSchemaProperty {
        JsonSchemaProperty {
            schema_type: "integer".into(),
            description: Some(desc.into()),
            default: default.map(|v| serde_json::json!(v)),
            enum_values: None,
            format: None,
        }
    }
    fn stock_code_params() -> Option<JsonSchema> {
        let mut props = std::collections::HashMap::new();
        props.insert("stock_code".into(), sc_prop("6位股票代码，如 600519"));
        Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(props),
            required: Some(vec!["stock_code".into()]),
            items: None,
        })
    }
    fn no_params() -> Option<JsonSchema> {
        Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        })
    }
    fn data_params() -> Option<JsonSchema> {
        let mut props = std::collections::HashMap::new();
        props.insert(
            "data".into(),
            JsonSchemaProperty {
                schema_type: "string".into(),
                description: Some("JSON 格式的数值数组或数据序列".into()),
                default: None,
                enum_values: None,
                format: None,
            },
        );
        Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(props),
            required: None,
            items: None,
        })
    }

    // 常用工具定义
    let td_quote = ToolDef {
        name: "get_stock_quote".into(),
        description: Some("获取股票实时行情：现价、涨跌幅、PE、PB、市值".into()),
        parameters: stock_code_params(),
    };
    let mut kline_props = std::collections::HashMap::new();
    kline_props.insert("stock_code".into(), sc_prop("6位股票代码"));
    kline_props.insert("period".into(), sc_prop_default("周期: daily/weekly/monthly", "daily"));
    kline_props.insert("limit".into(), int_prop("K线数量", Some(120)));
    let td_kline = ToolDef {
        name: "get_stock_kline".into(),
        description: Some("获取K线数据：OHLCV，可指定周期和数量".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(kline_props),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_fin = ToolDef {
        name: "get_stock_financials".into(),
        description: Some("获取财务数据：营收、净利润、EPS、ROE、毛利率等".into()),
        parameters: stock_code_params(),
    };
    // Phase 2: 预聚合的基本面分析报告（markdown 格式）。
    // 由 a-fundamentals 节点通过 t-fundamentals-data 预拉,作为冷启动 context 输入,
    // 避免 LLM 在大量原始财报上重复计算同比/环比/健康度等基础比率。
    let td_fundamentals_report = ToolDef {
        name: "get_fundamentals_report_markdown".into(),
        description: Some(
            "获取基本面分析报告(预聚合 markdown):含 PE/PB/ROE/同比环比/估值带/0-100 健康度评分 \
             与质量等级。返回字符串,直接消费"
                .into(),
        ),
        parameters: stock_code_params(),
    };
    let mut news_props = std::collections::HashMap::new();
    news_props.insert("stock_code".into(), sc_prop("6位股票代码"));
    news_props.insert("limit".into(), int_prop("新闻数量", Some(30)));
    let td_news = ToolDef {
        name: "get_stock_news".into(),
        description: Some("获取近期新闻公告".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(news_props),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_mf = ToolDef {
        name: "get_stock_money_flow".into(),
        description: Some("获取资金流向：主力/超大单/大单/中单/小单净流入".into()),
        parameters: stock_code_params(),
    };
    let td_social_sentiment = ToolDef {
        name: "get_social_sentiment".into(),
        description: Some("获取社交舆情：股吧帖子数/情感倾向/看多看空比例".into()),
        parameters: stock_code_params(),
    };
    let td_score = ToolDef {
        name: "compute_scoring".into(),
        description: Some("计算技术评分：基于趋势、偏离度、MACD、成交量、RSI、支撑阻力".into()),
        parameters: stock_code_params(),
    };
    let td_val = ToolDef {
        name: "compute_valuation".into(),
        description: Some(
            "计算估值指标：DCF、F-Score、护城河量化、安全边际。可选估值参数（**百分数**口径，\
             如 dcf_discount_rate=8.5 表示折现率 8.5%）由模板变量注入，省略则用后端默认值"
                .into(),
        ),
        // C2 路径 Z(2026-09-12): 补齐扁平参数 schema。此前仅声明 stock_code，
        // 而 t-valuation 的 input_mapping 会注入 3 个 dcf_* 参数 —— schema 与
        // 实际入参不一致会让工具面板/校验看不到真实入参。
        parameters: {
            let num_prop = |desc: &str| JsonSchemaProperty {
                schema_type: "number".into(),
                description: Some(desc.into()),
                default: None,
                enum_values: None,
                format: None,
            };
            let mut props = std::collections::HashMap::new();
            props.insert("stock_code".into(), sc_prop("6位股票代码，如 600519"));
            props.insert("dcf_growth_rate".into(), num_prop("DCF 增长率 (百分数，12 = 12%)"));
            props.insert("dcf_perpetual_rate".into(), num_prop("DCF 永续增长率 (百分数，4 = 4%)"));
            props.insert("dcf_discount_rate".into(), num_prop("DCF 折现率 (百分数，8.5 = 8.5%)"));
            Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: Some(props),
                required: Some(vec!["stock_code".into()]),
                items: None,
            })
        },
    };
    let mut risk_props = std::collections::HashMap::new();
    risk_props.insert("stock_codes".into(), sc_prop("逗号分隔的股票代码列表"));
    risk_props.insert("weights".into(), sc_prop("逗号分隔的持仓权重(0-1)，不填则等权"));
    let td_risk = ToolDef {
        name: "compute_portfolio_risk".into(),
        description: Some("计算组合风险：总市值、集中度、风险等级".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(risk_props),
            required: Some(vec!["stock_codes".into()]),
            items: None,
        }),
    };
    // ── 新增 12 个金融模型 ToolDef ──
    let td_maxdd = ToolDef {
        name: "calc_max_drawdown".into(),
        description: Some("计算最大回撤比例".into()),
        parameters: data_params(),
    };
    let td_sharpe = ToolDef {
        name: "calc_sharpe_ratio".into(),
        description: Some("计算夏普比率".into()),
        parameters: data_params(),
    };
    let td_var = ToolDef {
        name: "calc_var".into(),
        description: Some("历史模拟法 VaR 计算".into()),
        parameters: data_params(),
    };
    let td_pe_pct = ToolDef {
        name: "calc_pe_percentile".into(),
        description: Some("PE 历史分位数".into()),
        parameters: data_params(),
    };
    let td_peg = ToolDef {
        name: "calc_peg".into(),
        description: Some("PEG 估值指标".into()),
        parameters: data_params(),
    };
    let td_ma_cross = ToolDef {
        name: "detect_ma_cross".into(),
        description: Some("MA 金叉死叉检测".into()),
        parameters: data_params(),
    };
    let td_kelly = ToolDef {
        name: "calc_kelly".into(),
        description: Some("凯利公式仓位计算".into()),
        parameters: data_params(),
    };
    let td_rp = ToolDef {
        name: "calc_risk_parity".into(),
        description: Some("风险平价权重计算".into()),
        parameters: data_params(),
    };
    // ── 新增 9 个数据 API ToolDef ──
    let td_research = ToolDef {
        name: "get_stock_research_reports".into(),
        description: Some("获取券商研报".into()),
        parameters: stock_code_params(),
    };
    let td_consensus = ToolDef {
        name: "get_stock_consensus_eps".into(),
        description: Some("获取一致性预期EPS".into()),
        parameters: stock_code_params(),
    };
    let td_concepts = ToolDef {
        name: "get_stock_concept_blocks".into(),
        description: Some("获取概念板块归属".into()),
        parameters: stock_code_params(),
    };
    let td_announce = ToolDef {
        name: "get_stock_announcements".into(),
        description: Some("获取公司公告".into()),
        parameters: stock_code_params(),
    };
    let td_north = ToolDef {
        name: "get_north_bound_flow".into(),
        description: Some("获取北向资金流向".into()),
        parameters: no_params(),
    };
    let td_dragon = ToolDef {
        name: "get_market_dragon_tiger".into(),
        description: Some("获取龙虎榜数据".into()),
        parameters: no_params(),
    };
    let td_hot = ToolDef {
        name: "get_hot_stocks".into(),
        description: Some("获取市场热门股".into()),
        parameters: no_params(),
    };
    let td_industry = ToolDef {
        name: "get_industry_ranking".into(),
        description: Some("获取行业涨跌排名".into()),
        parameters: no_params(),
    };
    let td_cls = ToolDef {
        name: "get_cls_flash".into(),
        description: Some("获取财联社实时快讯".into()),
        parameters: no_params(),
    };
    let td_search_news = ToolDef {
        name: "search_news".into(),
        description: Some("按关键词搜索财经新闻，用于验证催化剂/CapEx/行业趋势".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([
                (
                    "keyword".into(),
                    JsonSchemaProperty {
                        schema_type: "string".into(),
                        description: Some("搜索关键词".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
                (
                    "limit".into(),
                    JsonSchemaProperty {
                        schema_type: "integer".into(),
                        description: Some("返回条数".into()),
                        default: Some(serde_json::json!(10)),
                        enum_values: None,
                        format: None,
                    },
                ),
            ])),
            required: Some(vec!["keyword".into()]),
            items: None,
        }),
    };
    let mut kdj_props = std::collections::HashMap::new();
    kdj_props.insert("klines_json".into(), sc_prop("K线JSON(含high/low/close)"));
    kdj_props.insert("n".into(), int_prop("KDJ周期N", Some(9)));
    let td_kdj = ToolDef {
        name: "compute_kdj".into(),
        description: Some("计算 KDJ 随机指标".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(kdj_props),
            required: None,
            items: None,
        }),
    };
    let td_obv = ToolDef {
        name: "compute_obv".into(),
        description: Some("计算 OBV 能量潮".into()),
        parameters: {
            let mut p = std::collections::HashMap::new();
            p.insert("klines_json".into(), sc_prop("K线JSON(含close/volume)"));
            Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: Some(p),
                required: None,
                items: None,
            })
        },
    };
    // ── P2: 事件检测 + 组合分析 ToolDef ──
    let mut earn_props = std::collections::HashMap::new();
    earn_props.insert("actual_eps".into(), sc_prop("实际EPS"));
    earn_props.insert("consensus_eps".into(), sc_prop("一致预期EPS"));
    // 2026-09-19（A2）：把「基准是否为估算值」显式暴露给调用方。
    //   来源是 `ConsensusEPS.is_estimated`（序列化为 `isEstimated`）—— 调用方从
    //   `get_stock_consensus_eps` 拿到该字段后应原样传进来，否则本工具会拿一个
    //   板块常数当真基准算相对量。
    earn_props.insert(
        "consensus_eps_is_estimated".into(),
        sc_prop("一致预期EPS是否为估算值(true 时本工具拒判超预期)"),
    );
    let td_earnings = ToolDef {
        name: "detect_earnings_surprise".into(),
        description: Some(
            "检测业绩超预期/低于预期（consensus_eps 为估算值时拒判：假基准算出的相对量无意义）"
                .into(),
        ),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(earn_props),
            required: None,
            items: None,
        }),
    };
    let mut pledge_props = std::collections::HashMap::new();
    pledge_props.insert("pledge_pct".into(), sc_prop("质押比例(%)"));
    pledge_props.insert("warning_line".into(), sc_prop("预警线(默认50)"));
    pledge_props.insert("liquidation_line".into(), sc_prop("平仓线(默认70)"));
    let td_pledge = ToolDef {
        name: "detect_pledge_risk".into(),
        description: Some("检测大股东质押风险".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(pledge_props),
            required: None,
            items: None,
        }),
    };
    let td_corr = ToolDef {
        name: "calc_correlation_matrix".into(),
        description: Some("计算收益率相关系数矩阵".into()),
        parameters: {
            let mut p = std::collections::HashMap::new();
            p.insert("returns_matrix_json".into(), sc_prop("收益率矩阵JSON(二维数组)"));
            Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: Some(p),
                required: None,
                items: None,
            })
        },
    };
    // ── P3: 独立新能力 ToolDef ──
    let mut mc_props = std::collections::HashMap::new();
    mc_props.insert("current_price".into(), sc_prop("当前价格"));
    mc_props.insert("annual_return".into(), sc_prop("年化收益率(默认0.08)"));
    mc_props.insert("annual_volatility".into(), sc_prop("年化波动率(默认0.3)"));
    mc_props.insert("days".into(), int_prop("模拟天数", Some(30)));
    mc_props.insert("simulations".into(), int_prop("模拟次数", Some(1000)));
    let td_mc = ToolDef {
        name: "run_monte_carlo".into(),
        description: Some("蒙特卡洛模拟价格路径".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(mc_props),
            required: None,
            items: None,
        }),
    };
    let mut ind_props = std::collections::HashMap::new();
    ind_props.insert("stock_pe".into(), sc_prop("个股PE"));
    ind_props.insert("stock_growth".into(), sc_prop("个股增长率"));
    ind_props.insert("industry_avg_pe".into(), sc_prop("行业平均PE"));
    ind_props.insert("industry_avg_growth".into(), sc_prop("行业平均增长率"));
    let td_ind = ToolDef {
        name: "analyze_industry_position".into(),
        description: Some("行业内估值/增长对比分析".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(ind_props),
            required: None,
            items: None,
        }),
    };
    let td_block = ToolDef {
        name: "get_stock_block_trades".into(),
        description: Some("获取大宗交易记录：成交价、成交量、买卖方营业部、折价率".into()),
        parameters: stock_code_params(),
    };
    let td_visit = ToolDef {
        name: "get_stock_institutional_visits".into(),
        description: Some("获取机构调研记录：调研日期、机构数量、调研内容".into()),
        parameters: stock_code_params(),
    };
    let td_idx = ToolDef {
        name: "get_index_quotes".into(),
        description: Some("获取大盘指数行情（上证指数、深证成指、创业板指）".into()),
        parameters: no_params(),
    };
    let td_peers = ToolDef {
        name: "get_stock_peers".into(),
        description: Some("获取同行业可比公司估值（PE/PB/ROE/涨跌幅/市值）".into()),
        parameters: stock_code_params(),
    };
    let td_pcr = ToolDef {
        name: "get_stock_option_pcr".into(),
        description: Some("获取期权PCR（看跌/看涨比率和持仓量比率，市场情绪前瞻指标）".into()),
        parameters: stock_code_params(),
    };
    let td_lockup = ToolDef {
        name: "get_stock_lockup".into(),
        description: Some("获取限售解禁日程（解禁日期、股数、比例、股东名称）".into()),
        parameters: stock_code_params(),
    };
    let td_lockup_bundle = ToolDef {
        name: "get_stock_lockup_bundle".into(),
        description: Some("获取筹码面分析数据（解禁+增减持+大宗交易三方聚合）".into()),
        parameters: stock_code_params(),
    };
    let td_sh_trades = ToolDef {
        name: "get_stock_shareholder_trades".into(),
        description: Some("获取大股东增减持记录（变动类型、数量、均价、原因）".into()),
        parameters: stock_code_params(),
    };
    let td_dividend = ToolDef {
        name: "get_stock_dividend_records".into(),
        description: Some("获取除权除息/分红送配记录".into()),
        parameters: stock_code_params(),
    };
    let td_nb_holding = ToolDef {
        name: "get_stock_north_bound".into(),
        description: Some("获取北向资金个股持仓（持股数量、占比）".into()),
        parameters: stock_code_params(),
    };
    let td_dt = ToolDef {
        name: "get_stock_dragon_tiger".into(),
        description: Some("获取个股龙虎榜数据（营业部买卖、上榜原因）".into()),
        parameters: stock_code_params(),
    };
    let td_margin = ToolDef {
        name: "get_stock_margin_data".into(),
        description: Some("获取融资融券数据（融资买入额、余额、融券卖出量、余量）".into()),
        parameters: stock_code_params(),
    };
    // v72(2026-09-21): 股权质押数据（`get_stock_pledge_data`）。
    //
    // ⚠️ **这个 ToolDef 是「白名单授权」生效的**前置条件**，漏了它就是假修复**：
    //   节点侧 `config.tools` 由
    //   `tool_names.iter().filter_map(|tn| tool_def_map.get(tn).cloned())` 生成 ——
    //   `filter_map` 对查不到的名字**静默丢弃**（不报错、不留痕）。
    //   只往 `PROFILE_TOOLS` 加名字而漏了这里 ⇒ 界面上「已授权」，LLM 手里却没有该工具
    //   ⇒ 这一维度仍恒缺，与修复前**行为完全一致**。
    //   守护测试：`seed_consistency_tests::profile_tools_are_all_declared_in_tool_def_map`。
    //
    // 与既有的 `td_pledge`（`detect_pledge_risk`，阈值**判定**工具）区分：
    //   那个要求调用方先自备 `pledge_pct`，本 ToolDef 取的正是那个输入（`pledge_ratio`）。
    let td_pledge_data = ToolDef {
        name: "get_stock_pledge_data".into(),
        description: Some(
            "获取股权质押数据（大股东质押总比例/质押股数/质押笔数/控股股东质押比例/风险等级）"
                .into(),
        ),
        parameters: stock_code_params(),
    };
    // P0 修复(2026-07-22): 移除未实现的 get_announcement_content ToolDef
    // 该工具在 mcp_tools.rs 中未注册 dispatch，调用会触发 "Unknown MCP tool" 错误
    let td_sector_info = ToolDef {
        name: "get_stock_sector_info".into(),
        description: Some("获取行业分类（申万一级/二级、概念板块标签）".into()),
        parameters: stock_code_params(),
    };
    let td_candlestick_patterns = ToolDef {
        name: "detect_candlestick_patterns".into(),
        description: Some("检测 K 线形态（吞没/锤子/晨星等 12 种）".into()),
        parameters: data_params(),
    };
    let td_divergence = ToolDef {
        name: "detect_divergence".into(),
        description: Some("检测价量背离（RSI 顶底背离 + OBV 背离）".into()),
        parameters: data_params(),
    };

    // 工具名 → ToolDef 映射（用于按名查找，给节点填充 config.tools）
    let tool_def_map: std::collections::HashMap<&str, ToolDef> = [
        ("get_stock_quote", td_quote.clone()),
        ("get_stock_kline", td_kline.clone()),
        ("get_stock_financials", td_fin.clone()),
        // Phase 2: 基本面报告(markdown)由 t-fundamentals-data 节点调用
        ("get_fundamentals_report_markdown", td_fundamentals_report.clone()),
        ("get_stock_news", td_news.clone()),
        ("get_stock_money_flow", td_mf.clone()),
        ("get_social_sentiment", td_social_sentiment.clone()),
        ("compute_scoring", td_score.clone()),
        ("compute_valuation", td_val.clone()),
        ("compute_portfolio_risk", td_risk.clone()),
        (
            "search_stock",
            ToolDef {
                name: "search_stock".into(),
                description: Some(
                    "按代码或名称模糊搜索A股。keyword 必须是完整的中文名称（如'中国卫通'）或 6 位数字代码（如'601698'），禁止传入拼音片段".into(),
                ),
                parameters: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("keyword".into(), sc_prop("完整中文名称或6位数字代码，如'中国卫通'或'601698'"));
                    Some(JsonSchema {
                        schema_type: "object".into(),
                        description: None,
                        properties: Some(props),
                        required: Some(vec!["keyword".into()]),
                        items: None,
                    })
                },
            },
        ),
        ("get_hot_stocks", td_hot.clone()),
        ("get_industry_ranking", td_industry.clone()),
        ("get_stock_announcements", td_announce.clone()),
        ("get_stock_consensus_eps", td_consensus.clone()),
        ("compute_kdj", td_kdj.clone()),
        ("compute_obv", td_obv.clone()),
        ("get_cls_flash", td_cls.clone()),
        ("search_news", td_search_news.clone()),
        ("get_north_bound_flow", td_north.clone()),
        ("get_market_dragon_tiger", td_dragon.clone()),
        ("get_stock_research_reports", td_research.clone()),
        ("get_stock_concept_blocks", td_concepts.clone()),
        ("get_stock_block_trades", td_block.clone()),
        ("get_stock_institutional_visits", td_visit.clone()),
        ("get_index_quotes", td_idx.clone()),
        ("get_stock_peers", td_peers.clone()),
        ("get_stock_option_pcr", td_pcr.clone()),
        ("get_stock_lockup", td_lockup.clone()),
        ("get_stock_lockup_bundle", td_lockup_bundle.clone()),
        ("get_stock_shareholder_trades", td_sh_trades.clone()),
        ("get_stock_dividend_records", td_dividend.clone()),
        ("get_stock_north_bound", td_nb_holding.clone()),
        ("get_stock_dragon_tiger", td_dt.clone()),
        ("get_stock_margin_data", td_margin.clone()),
        // v72(2026-09-21): 质押数据。**必须在此登记** —— 只写 PROFILE_TOOLS 而漏这里，
        //   授权会被 `filter_map` 静默丢弃（见 td_pledge_data 定义处的说明）。
        ("get_stock_pledge_data", td_pledge_data.clone()),
        ("get_stock_sector_info", td_sector_info.clone()),
        ("detect_candlestick_patterns", td_candlestick_patterns.clone()),
        ("detect_divergence", td_divergence.clone()),
        ("detect_earnings_surprise", td_earnings.clone()),
    ]
    .into_iter()
    .collect();

    // 从 ToolDef 列表生成 "可用工具" prompt 片段
    fn tool_prompt(tools: &[ToolDef]) -> String {
        if tools.is_empty() {
            return String::new();
        }
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        format!(
            "\n\n你可以调用以下工具获取最新数据或计算指标：{}。请先调用相关工具获取数据，再基于返回结果进行分析。",
            names.join("、")
        )
    }

    let agent = |id: &str,
                 title: &str,
                 expert_id: &str,
                 parent_id: Option<&str>,
                 x: f64,
                 y: f64|
     -> WorkflowNode {
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("股票分析: {expert_id}")),
                position: Position { x, y },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 3, // v24: 从 2 提升到 3，GLM-5.1 429 限流可持续 30s+
                    base_delay_ms: 3000, // v24: 从 1000 提升到 3000，避免短退避对限流无效
                    max_delay_ms: 60000, // v24: 从 30000 提升到 60000
                    backoff_type: BackoffType::Exponential,
                },
                timeout: None, // 继承 RunOptions.step_timeout（来自 agent_timeout_secs 设置）,让用户在面板中可调
                enabled: true,
                parent_id: parent_id.map(String::from),
                compensation: None,
                continue_on_fail: false,
            },
            config: AgentNodeConfig {
                // inline system_prompt 只放任务指令，专家 prompt 由 agent_profile 自动加载，
                // 行情数据通过 context_sources 由上游 Tool 节点输出自动注入
                // P0 回退(v16):inline prefix 回退到 v14 之前的形式 —— 不在
                //   inline prefix 中用 {{stock_code}}/{{stock_name}} Slot。
                //   原因:v14/v15 改动在 inline prefix 引入 Slot 后,某些
                //   context.variables 注入路径下 render_prompt 失败,导致所有
                //   Agent 节点返回 "暂无数据"。stock_code/stock_name 改为通过
                //   expert .md prompt 头部 "{{stock_code}} / {{stock_name}}"
                //   primacy 锚点注入,避开 inline prefix 的风险。
                system_prompt: format!(
                    "你的任务: {title}\n\n重要原则：\n1. 如果上游数据节点返回为空，请主动调用可用工具获取补充数据。\n2. 如果经过补充获取仍然无法获得某些数据，请在分析报告中诚实标记该维度数据获取失败的状态，并评估该缺失对分析结论的影响程度。\n3. 始终针对目标股票给出明确的观点（看多/看空/中性）和论据。\n4. 工具返回空数组或空对象有两种可能：①该数据源暂无法获取（技术问题）；②该股票在该维度无数据（如无机构覆盖）。请在报告中明确区分两种情况并评估对分析的影响。\n5. 如果你是研报分析师，目标是从券商研报、一致预期EPS、机构调研等维度给出观点。如果这些数据源返回空，说明该股票暂无机构覆盖，请标注'无机构覆盖'并基于公司基本面、行业地位、新闻公告等公开信息给出独立分析。",
                ),
                context_sources: vec![],
                // 通过 input_mapping 自动注入股票代码/名称到 system_prompt
                input_mapping: [
                    ("stock_code".to_string(), "stock_code".to_string()),
                    ("stock_name".to_string(), "stock_name".to_string()),
                ]
                .into_iter()
                .collect(),
                output_var: id.into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(32768),
                tools: vec![],
                exposed_tools: vec![],
                output_mode: OutputMode::Text,
                agent_profile_id: Some(format!("stock-{expert_id}")),
                max_tool_rounds: None,
                execution_mode: None,
                rag_source_ids: vec![],
                consistency_check: None,
                // V74 关闭: hallucination_guard 锚定检查。
                // 原因：LLM 输出是分析结论（自然语言），source_context 是工具返回的 JSON 数据，
                // 格式天然不匹配导致 2-gram 匹配率极低（实测分数 0.0-0.39，阈值 0.4），
                // 几乎所有 AgentNode 都触发 WARN 误报，无实际拦截价值。
                // portfolio-mgr.rhai 已有 __untrusted 兜底机制（来自 a-policy 等数据缺失场景），
                // 不依赖 hallucination_guard 的输出。
                hallucination_guard: Some(HallucinationGuardConfig {
                    enabled: false,
                    match_threshold: 0.4,
                }),
                // H4.1: fallback_model 不在此硬编码。用户可在工作流编辑器中为单个 Agent 节点
                // 配置 model（主模型），agent_executor 校验失败时若 fallback_model ≠ model 则触发重试。
                // 股票分析模板默认不设 fallback，由项目默认模型一致性保证。
                fallback_model: None,
                task_scene: None,
                // stream_chunk_timeout_secs: 300s（5 分钟）
                // 默认 120s 在大上下文场景下偶发 TTFB >120s 导致超时重试浪费时间
                // （参见 debate-convergence 节点的同类修复注释）
                stream_chunk_timeout_secs: Some(300),
            },
        })
    };

    let edge = |id: &str, source: &str, target: &str| -> WorkflowEdge {
        WorkflowEdge {
            id: id.into(),
            source: source.into(),
            source_handle: None,
            target: target.into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        }
    };

    let mut nodes: Vec<WorkflowNode> = Vec::new();
    let mut edges: Vec<WorkflowEdge> = Vec::new();

    // Trigger
    nodes.push(WorkflowNode::Trigger(TriggerNode {
        base: WorkflowNodeBase {
            id: "trigger".into(),
            title: "开始分析".into(),
            description: Some("输入股票代码启动分析".into()),
            // F-1 修复: 3×3 网格最右列 x=1240+200=1440, 居中 trigger x=520
            position: Position { x: 520.0, y: 0.0 },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({"stock_code": "{{stock_code}}"}),
        },
    }));

    // 9 个分析师 + catalyst-analyst
    let analysts = [
        ("a-market-analyst", "技术面分析：K线形态、MACD/RSI、支撑阻力位", "market-analyst"),
        ("a-sentiment", "市场情绪分析：资金流向、散户/机构态度", "sentiment-analyst"),
        ("a-news", "新闻公告影响评估", "news-analyst"),
        ("a-fundamentals", "基本面估值分析：PE/PB/ROE等", "fundamentals-analyst"),
        ("a-policy", "宏观政策与行业政策影响分析", "policy-analyst"),
        ("a-hot-money", "游资动向与主力资金追踪", "hot-money-tracker"),
        ("a-lockup", "解禁减持与质押风险排查", "lockup-watcher"),
        ("a-research", "券商研报观点汇总", "research-analyst"),
        ("a-sector", "行业景气度与轮动分析", "sector-analyst"),
        ("a-catalyst", "催化剂与叙事完整度评估", "catalyst-analyst"),
    ];
    let a_ids: Vec<&str> = analysts.iter().map(|(id, _, _)| *id).collect();

    // 为每个分析师插入对应的数据获取 Tool 节点
    // 注：节点工具决定了下游 analyst 拿到的"前置数据"。LLM agent 自身仍可调用
    // PROFILE_TOOLS 中的工具，但首屏/冷启动数据由这些 tool 节点预拉。
    //
    // F-8 修复: 顺序必须与上面的 `analysts` 数组完全一致：
    //   [0] a-market-analyst   ↔ t-market-data
    //   [1] a-sentiment        ↔ t-sentiment-data
    //   [2] a-news             ↔ t-news-data
    //   [3] a-fundamentals     ↔ t-fundamentals-data
    //   [4] a-policy           ↔ t-policy-data
    //   [5] a-hot-money        ↔ t-hotmoney-data   (原: t-research-data 错位)
    //   [6] a-lockup           ↔ t-lockup-data     (原: t-hotmoney-data 错位)
    //   [7] a-research         ↔ t-research-data   (原: t-lockup-data 错位)
    //   [8] a-sector           ↔ t-sector-data
    // 错位会导致 hot-money analyst 拿到研报数据、research analyst 拿到解禁数据，
    // 9 个分析师产出的报告与各自的角色语义不符。
    // ── v72(2026-09-21)：a-lockup 的第二个前置数据源（股权质押）──
    // 单点声明节点 id，供下面三处复用：① 本文件新增 ToolNode；② `a-lockup` 的
    //   `context_sources`；③ 两条边（trigger→节点、节点→a-lockup）。
    // 契约缺口背景（详见 `AUDIT-pledge-attribution-2026-09-21.md`）：
    //   `lockup-watcher.md` 的方法论第 4 条与自检清单都要求分析「质押比例 > 50%
    //   高警戒线 / 质押风险敞口」，而其唯一上游 `t-lockup-data` 调的是
    //   `get_stock_lockup_bundle`（解禁 + 增减持 + 大宗交易**三方**聚合，
    //   结构上不含质押）⇒ 该维度**每轮必缺**，模型只能自己给缺口编原因
    //   （全库 15 轮里 12 轮写了质押缺口，最远漂到「工具调用被拒绝」的伪归因）。
    //
    // ⚠️ **不**并进下面那张 `tool_assignments` 表：它与 `analysts` 数组**按下标
    //   一一对应**（`a_ids[i]` + 每个 analyst 恰好一条 branch），加一行会错配/越界。
    const PLEDGE_TOOL_ID: &str = "t-pledge-data";

    let tool_assignments: &[(&str, &str, &str, &str)] = &[
        ("t-market-data", "获取K线+行情", "get_stock_kline", "stock_code"),
        // 修复(2026-07-21): t-sentiment-data 改为 get_social_sentiment。
        // 原调用 get_stock_news 与 t-policy-data/t-news-data 重复,且 news API
        // 失败时 sentiment-analyst 拿不到任何前置数据。改为 get_social_sentiment:
        //   1) 真正对齐前端 "舆情" label —— 股吧帖子数/情感倾向/看多看空比例
        //   2) 与 t-hotmoney-data (get_stock_money_flow) 解耦,避免重复
        //   3) PROFILE_TOOLS 中仍保留 get_stock_news/get_stock_money_flow,
        //      LLM 可按需调用补充新闻和资金流数据
        ("t-sentiment-data", "获取股吧社交舆情", "get_social_sentiment", "stock_code"),
        // 修复(2026-07-21): t-news-data 改回 get_stock_news。
        // 2026-07-11 改为 get_stock_announcements 是因为 a-news 拿不到新闻,但
        // 导致前端 label "新闻" 与实际数据源 "公告" 错位,且与 t-catalyst-data
        // 重复调用同一工具。改回 get_stock_news 让 "新闻" label 真对应新闻,
        // a-news 仍可通过 PROFILE_TOOLS 调用 get_stock_announcements 补充公告。
        ("t-news-data", "获取近期新闻", "get_stock_news", "stock_code"),
        // 修复 P1: 基本面分析师前置数据改用 get_stock_financials（财报）而非
        // get_consensus_eps（一致预期），让 a-fundamentals 启动时就能拿到
        // 营收/利润/资产负债等核心财务数据。
        //
        // Phase 2: 升级为 get_fundamentals_report_markdown —— 工作流引擎在 a-fundamentals
        // 启动前预拉"预聚合 markdown 报告"(健康度评分/估值带/同比环比/质量等级)。
        // LLM 启动时直接消费 markdown,引用 system_pre_computed 字段
        // (health_score / valuation_state / safety_margin_pct / yoy_*),
        // 避免在大量原始财报上重复计算基础比率。
        // 注意: PROFILE_TOOLS 中仍保留 get_stock_financials,LLM 需要做更细颗粒分析时可主动调用。
        (
            "t-fundamentals-data",
            "获取基本面报告(markdown)",
            "get_fundamentals_report_markdown",
            "stock_code",
        ),
        // 修复(2026-07-22): 改用新增的 get_stock_policy_news 工具。
        // 该工具基于股票所属行业做关键词搜索("政策/规划/通知/补贴"),
        // 相比原 get_stock_news(综合新闻)能更精准命中政策类内容,
        // 避免与 t-news-data 重复调用同一工具。
        //
        // 数据来源:东方财富搜索 API(基于行业关键词),无需对接政府网站。
        // 限制:返回的是新闻摘要(非政策原文),a-policy 分析师需基于摘要做推断。
        ("t-policy-data", "获取政策新闻", "get_stock_policy_news", "stock_code"),
        // F-8 重排: a-hot-money 前置改为资金流向工具
        ("t-hotmoney-data", "获取资金流向", "get_stock_money_flow", "stock_code"),
        // F-8 重排: a-lockup 前置改为解禁质押工具
        ("t-lockup-data", "获取解禁+增减持+大宗交易", "get_stock_lockup_bundle", "stock_code"),
        // F-8 重排: a-research 前置改为研报工具
        ("t-research-data", "获取研报+新闻", "get_stock_research_reports", "stock_code"),
        ("t-sector-data", "获取行情+行业排名", "get_industry_ranking", "stock_code"),
        // 修复(2026-07-21): t-catalyst-data 保留 get_stock_announcements。
        // 这是唯一调用 get_stock_announcements 的前置 ToolNode,避免与 t-news-data
        // 重复调用同一工具导致缓存击穿 + 双份失败警告。
        ("t-catalyst-data", "获取公司公告", "get_stock_announcements", "stock_code"),
    ];

    // ── Phase 1: ParallelNode 作为视觉分组，包裹 9 组 Tool + Agent ──
    // F-1 修复: 布局从"2 列 9 行"改为"3 列 3 行"网格。
    //   原布局 (x=20 单一列, 9 行 80px) 存在 3 类重叠：
    //     1) trigger (x=250, y=0) 与 a-market-analyst (x=240, y=40) 边界框重叠 ~7600 px²
    //     2) p-analysts 容器 (x=300, y=200) 与 a-fundamentals (x=240, y=200)
    //        等 3 行 analyst 节点重叠
    //     3) 单一纵列 9 行总高 720px 浪费大量垂直空间
    //   新布局: 3×3 网格,col_width=480 (tool 200 + gap 40 + agent 200 + 余量 40)
    //     col_x = [40, 520, 1000]
    //     tool x = col_x[col], agent x = col_x[col] + 240
    //     row_y = 100 + row*120  (节点高 80, 行距 40)
    //   trigger 居中放置 x=580 (3 列总宽 1200, 居中后左侧 580),y=0
    //   p-analysts 容器 (20, 80) 起,完整包络 3×3 网格
    let col_x = [40.0_f64, 520.0, 1000.0];
    let row_y_base = 100.0;
    // FIX: agent 节点高度 160px, 之前 row_dy=120 导致连续行重叠 40px
    let row_dy = 180.0;
    let mut analyst_branches: Vec<Branch> = Vec::with_capacity(tool_assignments.len());
    for (i, (tool_id, tool_title, tool_name, arg_key)) in tool_assignments.iter().enumerate() {
        let analyst_id = a_ids[i];
        let col = i % 3;
        let row = i / 3;
        let x_tool = col_x[col];
        let y = row_y_base + row as f64 * row_dy;
        nodes.push(tool_node(
            tool_id,
            // F-2 修复: 原本硬编码 "获取数据" 导致 9 个 tool 节点 title 完全一致、
            // 编辑器画布无法区分。改用 tool_assignments 中已经声明的中文描述。
            tool_title,
            tool_name,
            tool_id,
            arg_key,
            &[],
            Some("p-analysts"),
            x_tool,
            y,
        ));
        edges.push(edge(&format!("e-trigger-{tool_id}"), "trigger", tool_id));
        edges.push(edge(&format!("e-{tool_id}-{analyst_id}"), tool_id, analyst_id));
        analyst_branches.push(Branch {
            id: format!("branch-{analyst_id}"),
            title: tool_title.to_string(),
            steps: vec![tool_id.to_string(), analyst_id.to_string()],
            branch_timeout_ms: None,
            degrade_strategy: Default::default(),
        });
    }

    // ── v72(2026-09-21)：a-lockup 的第二个前置 ToolNode —— 股权质押 ──
    // 调 MCP 工具 `get_stock_pledge_data`（`crates/astock-data/src/mcp_tools.rs`，
    //   schema 与 dispatch 两处成对已在），返回 pledge_ratio / pledge_shares /
    //   controlling_pledge_ratio / risk_level —— 正是 `lockup-watcher.md` 要的
    //   「质押比例 > 50% 高警戒线 / 质押风险敞口」。
    //
    // ⚠️ 工具名必须写**字面量**（不可引常量）：`seed_consistency_tests` 的
    //   `tool_node_positional_tool_names` 只抽位置参数中的字面量，写成变量会让本节点
    //   从「seed 声明 ↔ ToolResolver 解析空间」一致性门禁里**整个消失** ——
    //   即 V79 的 `t-valuation-band` 形态（工具名未注册却无人报警，节点 completed
    //   但输出为空、无报错）。写成字面量后，该门禁会替我们断言
    //   `get_stock_pledge_data` 确实落在解析空间内。
    nodes.push(tool_node(
        PLEDGE_TOOL_ID,
        "获取股权质押数据",
        "get_stock_pledge_data",
        PLEDGE_TOOL_ID,
        "stock_code",
        &[],
        Some("p-analysts"),
        // 布局 (col 1, row 3)：row 3 的 col 0 已被 a-catalyst 占用，col 1/2 空置。
        //   放在网格内 ⇒ 被 p-analysts 分组框包络，且与任何节点不重叠。
        520.0,
        row_y_base + 3.0 * row_dy,
    ));
    // 入边：与其它数据 ToolNode 同形（trigger 扇出），先于 a-lockup 完成。
    edges.push(edge("e-trigger-t-pledge-data", "trigger", PLEDGE_TOOL_ID));
    // 出边：质押数据 → a-lockup。这条边是**供给**，`context_sources` 是**消费声明**，
    //   两者必须同时存在（只写一边 ⇒ 变量仍不进 `context.variables`）。
    edges.push(edge("e-t-pledge-data-a-lockup", PLEDGE_TOOL_ID, "a-lockup"));

    // 工具由模板节点 config.tools 统一管理
    // 第 10 个 a-catalyst 放置在 3×3 网格下方（col 0, row 3），作为额外独立行
    for (i, (id, title, _expert)) in analysts.iter().enumerate() {
        let tool_id = tool_assignments[i].0;
        let _fixed_tool_name = tool_assignments[i].2;
        let col = i % 3;
        let row = i / 3;
        let x_agent = col_x[col] + 240.0;
        let row_y = row_y_base + row as f64 * row_dy;
        let mut an = agent(id, title, _expert, Some("p-analysts"), x_agent, row_y);
        if let WorkflowNode::Agent(ref mut a) = an {
            // v72: a-lockup 有**两个**前置 ToolNode —— 解禁三方 bundle（`t-lockup-data`）
            //   + 股权质押（`t-pledge-data`）。`context_sources` 少列一个 ⇒ 该节点的输出
            //   不进 `context.variables` ⇒ 分析师报告里这一维度恒缺。与 portfolio-mgr
            //   那批「写进 input_mapping ≠ 有人供给」是**同一失效机制**（见本文件
            //   「修复 portfolio-mgr 因子输入全空」段的注释）。
            a.config.context_sources = if *id == "a-lockup" {
                vec![tool_id.to_string(), PLEDGE_TOOL_ID.to_string()]
            } else {
                vec![tool_id.to_string()]
            };
            // fundamentals-analyst prompt 引用了 {{market_regime}}，
            // 从工作流变量 market_regime.regime 注入（bull/bear/sideways 状态字符串）
            if *id == "a-fundamentals" {
                a.config
                    .input_mapping
                    .insert("market_regime".to_string(), "market_regime.regime".to_string());
            }
            // catalyst-analyst 需要 3 轮：R1 读公告→确认催化剂,R2 调 K线/概念验证,R3 综合评估叙事
            a.config.max_tool_rounds = if *id == "a-catalyst" {
                Some(3)
            } else {
                Some(2)
            };
            let tool_names = PROFILE_TOOLS
                .iter()
                .find(|(k, _)| **k == **_expert)
                .map(|(_, v)| *v)
                .unwrap_or(&[]);
            a.config.tools =
                tool_names.iter().filter_map(|&tn| tool_def_map.get(tn).cloned()).collect();
            a.config.exposed_tools = vec![];
            // V70 修复(2026-09-10): a-catalyst 从 OutputMode::Json 改回默认 Text 模式，
            // 与其他 9 个分析师统一为「自然语言报告在前 + 末尾 <!-- VERDICT: {...} --> 标签」。
            // 根因：Json 模式的 schema 注入（要求纯 JSON、禁止 VERDICT 标签）与
            // catalyst-analyst.md 工作流程第 7 步残留的 Text 模式指令（"末尾追加 VERDICT
            // 机读标签"）自相矛盾，模型两头都执行 → 输出 JSON + 标签的混合体。
            // Text 模式下 agent_executor 分支 A 将标签 JSON 原样放入 verdict map，
            // catalyst_level/institutional_trace/narrative_completeness 等特有字段不丢失
            // （portfolio-mgr 与 pace-calc 的映射路径不变）。
            a.config.system_prompt =
                format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
            // 环 A: 注入历史反思教训，让分析师看到该股之前的错因和改进建议
            a.config.input_mapping =
                std::collections::HashMap::from([("stock_lessons".into(), "stock_lessons".into())]);
        }
        nodes.push(an);
    }

    // 分析师节点 → c-need-debate 的出边（编辑器可视化 + 运行时依赖）
    for aid in &a_ids {
        edges.push(edge(&format!("e-{aid}-debate"), aid, "debate-bull-bear"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 【装饰节点 / Decorative Container】p-analysts
    // ═══════════════════════════════════════════════════════════════════════
    // 语义：视觉分组容器，包裹 9 组 (Tool + Agent) 子节点
    // 调度：容器本身在引擎中立即 Completed（不参与流程控制）
    //      - wait_for_all=true, aggregation=All: 等所有子节点完成后聚合
    //      - auto_input_from_parent=false: 不自动从父节点拉数据
    //      - 实际依赖通过显式 edge 表达（e-trigger-{tool_id} 和 e-{tool_id}-{aid}）
    // parent_id：仅供前端编辑器嵌套渲染用，运行时调度忽略此字段
    //
    // 为什么不直接用 Edges 表达？
    //   前端需要把 9 个 Tool 和 9 个 Agent 画在一个可折叠的分组框内，
    //   单纯靠 edge 拓扑无法表达"视觉从属关系"。ParallelNode 在此
    //   充当"虚拟容器"，是 workflow_types 中两种角色之一的产物：
    //     1) 真正的并行控制器（wait_for_all + 聚合）
    //     2) 纯装饰性容器（仅前端展示，调度无意义）  ← 属于此类
    // ═══════════════════════════════════════════════════════════════════════
    nodes.push(WorkflowNode::Parallel(ParallelNode {
        base: WorkflowNodeBase {
            id: "p-analysts".into(),
            title: "10 维度分析师分组".into(),
            description: Some("行情/情绪/新闻/基本面/政策/游资/解禁/研报/行业/催化剂".into()),
            // F-1 修复: 原 (300, 200) 恰好压在 a-fundamentals (240, 200) 上。
            //   3×3 网格范围 x∈[40, 1400] y∈[100, 460],容器左上放 (20, 80),
            //   让前端能正确按 bbox 渲染分组框。
            position: Position { x: 20.0, y: 80.0 },
            retry: RetryConfig::default(),
            timeout: Some(120),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: ParallelNodeConfig {
            branches: analyst_branches,
            wait_for_all: true,
            timeout: Some(600),
            aggregation: Some(MergeStrategy::All),
            auto_input_from_parent: false, // 不自动从父节点接收输入
            sub_graph: None,               // v23+：稍后通过 inject_container_subgraphs 注入
        },
    }));
    // 前端验证要求容器节点有至少一条入边/出边，这里添加伪边绕过"死分支"检查。
    // 运行时容器立即完成，这些边不影响调度。
    edges.push(WorkflowEdge {
        id: "e-trigger-p-analysts".into(),
        source: "trigger".into(),
        source_handle: None,
        target: "p-analysts".into(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: None,
    });
    // V58 修复(2026-07-23): 删除 e-p-analysts-brief 伪边（p-analysts 是装饰性容器，
    // 立即完成且不携带子节点结果），改为直接从 10 个分析师节点连到 analyst-brief，
    // 确保：1) DAG 调度等所有分析师完成后才执行 analyst-brief；
    //       2) deps_results 包含 10 个分析师节点的输出，input_mapping 路径解析正确。
    // 前端验证要求容器节点有至少一条入边/出边，入边 e-trigger-p-analysts 已满足。

    // ── analyst-brief（分析师摘要）：CodeNode 聚合10份VERDICT评分+关键论据 ──
    // 替代原「辨手直接加载10份全量报告」方案，大幅降低辩论阶段上下文体积。
    // 输出 analyst-brief 字符串，经 input_mapping 接收10个分析师的 .content。
    //
    // 时序：p-analysts 全部完成后运行 → debate-bull-bear 依赖此摘要。
    {
        let ab_code = include_str!("../analyst-brief.rhai").to_string();
        let ab_input: std::collections::HashMap<String, String> = a_ids
            .iter()
            .map(|id| {
                let short = match *id {
                    "a-market-analyst" => "a_market_raw",
                    "a-sentiment" => "a_sentiment_raw",
                    "a-news" => "a_news_raw",
                    "a-fundamentals" => "a_fundamentals_raw",
                    "a-policy" => "a_policy_raw",
                    "a-hot-money" => "a_hot_money_raw",
                    "a-lockup" => "a_lockup_raw",
                    "a-research" => "a_research_raw",
                    "a-sector" => "a_sector_raw",
                    "a-catalyst" => "a_catalyst_raw",
                    _ => id,
                };
                // V58 修复(2026-07-23): 直接下钻到 .content.verdict，
                // resolve_var_path 会自动解析 content JSON 字符串并提取 verdict map。
                // 避免 Rhai 脚本中 json_parse 字符串解析的不可靠性。
                //
                // V68 修复(2026-09-10): 删除 a-catalyst 的 `.content` 特例（V60 引入）。
                // V60 时代 a-catalyst 是扁平 JSON（verdict 为字符串）；V62 通用 VERDICT
                // 重构后所有分析师 content 统一为 {"report", "verdict":{...}} 嵌套，
                // `.content` 终值不 auto-parse → rhai 收到 JSON 字符串 → format_analyst
                // 判"数据不可用"，辩论阶段催化剂维度恒缺失（002837 实证）。
                // 统一走 .content.verdict（中途穿透 parse），与 data-quality 的
                // cat_verdict 同批修复。
                (short.to_string(), format!("{id}.content.verdict"))
            })
            .collect();
        nodes.push(WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: "analyst-brief".into(),
                title: "分析师摘要（VERDICT评分+关键论据）".into(),
                description: Some(
                    "将10位分析师的VERDICT评分和bull_points/bear_points压缩为摘要，供辩论阶段使用"
                        .into(),
                ),
                position: Position { x: 50.0, y: 1150.0 },
                retry: RetryConfig::default(),
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: ab_code,
                output_var: "analyst-brief".into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: ab_input,
            },
        }));
    }
    // V58 修复: 添加 10 条从分析师节点直连 analyst-brief 的 edges，
    // 确保 deps_results 包含所有分析师输出，且等所有分析师完成后才执行 analyst-brief。
    for aid in a_ids.iter() {
        edges.push(WorkflowEdge {
            id: format!("e-{aid}-brief"),
            source: (*aid).into(),
            source_handle: None,
            target: "analyst-brief".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        });
    }
    // analyst-brief 直接喂给辩论阶段。个别维度缺数据的兜底由 brief 内的
    // 「**数据不可用**」标记处理——辩手看到标记自然知道跳过该维度。
    // 整份 brief 全空（极低概率）时辩手仍能输出「数据不足」的降级辩论，
    // 下游 pipeline 不会因缺少辩论输出而断裂。
    edges.push(WorkflowEdge {
        id: "e-brief-debate".into(),
        source: "analyst-brief".into(),
        source_handle: None,
        target: "debate-bull-bear".into(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: None,
    });

    // Phase 2: 决策检查点 — 记录分析师完成状态，辩论始终执行
    // 分析师节点已直接连接 DebateNode（无中间条件节点）

    // ── 辩论轮数（DAG 展开为 N 轮顺序执行） ──
    // v59（2026-09-21）起从变量 `debate_rounds` 读取，不再硬编码 1 轮。
    // 每轮展开一对独立辩手节点（bull-rN/bear-rN），各自真跑一次 LLM，
    // 经 expert persona + context_sources（引用前序轮次输出）产出不同内容，
    // 即「真多轮」。该值同时用于下游锚点（bear-r{debate_max_rounds}）与落库变量，
    // 由 `resolve_debate_rounds` 从 DB 旧变量/默认值求值，保证三处同源（防悬空边）。
    let debate_max_rounds: usize = resolve_debate_rounds(old_variables.as_deref());

    // ═══════════════════════════════════════════════════════════════════════
    // 【装饰节点 / Decorative Container】debate-bull-bear
    // ═══════════════════════════════════════════════════════════════════════
    // 语义：多空辩论的视觉分组容器，按轮展开展开辩论元数据（轮数由 debate_max_rounds 决定）
    // 调度：容器本身在引擎中立即 Completed；辩手由下方循环展开的独立实体节点承载，
    //      容器只把 debater_steps 配置列给引擎驱动，不返回辩论结果。
    //      - debater_steps: 2 × debate_max_rounds 个**独立**辩手节点（bull-r1..bull-rN /
    //        bear-r1..bear-rN），每轮一对、各带不同 expert persona 与 context_sources
    //        （引用前序轮次输出）⇒ 各自真跑一次 LLM，即「真多轮」。
    //      - max_rounds: 固定 1（v59/2026-09-21）。轮次由 distinct 节点边承担，
    //        max_rounds 若 = debate_max_rounds 会对整批 debater_steps 做 N 遍轮播，
    //        第 2+ 遍被引擎「Completed 复用」短路 → 假多轮浪费。保持 1 = 单遍跑完。
    //      - convergence_prompt/model: 配置就绪但当前未启用（v59 起收敛由
    //        debate-convergence 节点承担，无需引擎级相似度收敛循环）。
    //
    // ⚠️ 关键陷阱（P0 已修复）：
    //   历史 bug：曾将 value-investor 的入边连到本容器，导致 value-investor
    //   在容器 Completed 时立即启动——拿到的是"辩论配置"而非"辩论结果"。
    //   正确接法：value-investor 应等待最后一个真实辩手节点完成，即
    //   `bear-r{debate_max_rounds}`。⚠️ 禁止硬编码 `bear-r3` —— 轮数一变就会变成
    //   悬空入边，整条下游链静默 Skipped（见 debater_round_refs_are_parameterized）。
    //
    // 真实调度依赖链（首轮 bull-r1 启动条件）：
    //   trigger → tool → a-* → debate-bull-bear → bull-r1 → bear-r1 →
    //   bull-r2 → bear-r2 → … → bull-rN → bear-rN（轮次由下方循环的边串联）
    // parent_id：仅供前端编辑器嵌套渲染用
    //
    // ⚠️ 坐标约定（FIX: 所有节点位置为画布绝对坐标）：
    //   容器 debate-bull-bear 放在 (DEBATE_X, DEBATE_Y)
    //   辩手节点 x = DEBATE_X + 20px（容器内偏移）
    //   辩手节点 y = DEBATE_Y + 40px + round*2*180px（按轮次纵向排列）
    //   前端 WorkflowEditor 通过 parentId 减去容器坐标得到相对坐标交给 ReactFlow。
    // ═══════════════════════════════════════════════════════════════════════
    const DEBATE_X: f64 = 300.0;
    const DEBATE_Y: f64 = 1280.0;
    nodes.push(WorkflowNode::Debate(DebateNode {
        base: WorkflowNodeBase {
            id: "debate-bull-bear".into(),
            title: "多空辩论".into(),
            description: Some(format!(
                "{debate_max_rounds} 轮多空辩论：多方构建论点 → 空方反驳 → 循环"
            )),
            position: Position { x: DEBATE_X, y: DEBATE_Y },
            retry: RetryConfig { enabled: true, max_retries: 1, ..Default::default() },
            timeout: Some(900),
            enabled: true,
            parent_id: None,
            compensation: None,
            // v7: 容错降级。2026-09-08 实证：a-fundamentals 失败时本节点作为
            // 调度枢纽被上游 Failed 边阻塞 → 整条辩论链/t-scoring/portfolio-mgr
            // 全部停摆 → 决策降级观望。辩手只消费 analyst-brief（自带降级标注，
            // 缺基本面时显示"数据不可用"），本容器仅做调度转发，可安全放行。
            continue_on_fail: true,
        },
        config: DebateNodeConfig {
            debater_steps: (0..debate_max_rounds)
                .flat_map(|r| vec![format!("bull-r{}", r + 1), format!("bear-r{}", r + 1)])
                .collect(),
            // v59（2026-09-21）：固定 1 —— 单遍驱动全部 distinct 辩手节点。
            // 轮次由独立节点的边串联，不要设成 debate_max_rounds（会重复轮播被短路）。
            max_rounds: 1,
            convergence_prompt: None,
            convergence_model: None,
            topic_var: "trigger.output".into(),
            output_var: String::new(),
            sub_graph: None, // v23+：稍后通过 inject_container_subgraphs 注入
        },
    }));

    // DebateNode 的子节点：按轮次展开多方辩手和空方辩手
    // parentId 指向容器节点，前端将它们渲染在 DebateNode 内部
    // 位置：容器内 20px 左偏移，按轮次纵向排列（绝对坐标 = 容器坐标 + 偏移）
    // v16 历史工具集：辩手节点 v71 改为纯决策节点后不再使用，保留定义供未来恢复参考。
    // v17+ 可考虑给空方注入估值/风险类特色工具(td_var / td_maxdd / td_pledge / td_corr)。
    let _bull_tools = vec![
        td_quote.clone(),
        td_kline.clone(),
        td_fin.clone(),
        td_news.clone(),
        td_score.clone(),
        td_earnings.clone(),
        td_ma_cross.clone(),
        td_candlestick_patterns.clone(),
        td_divergence.clone(),
    ];

    for round in 0..debate_max_rounds {
        let round_num = round + 1;
        let bull_id = format!("bull-r{round_num}");
        let bear_id = format!("bear-r{round_num}");
        // R1 走 bull-researcher / bear-researcher（初始论证型），R2 走 bull-r2 / bear-r2
        // （质询型），R3 走 bull-r3 / bear-r3（最终反驳型）。R2/R3 工具集一致：
        // 都需要 compute_scoring / compute_valuation 核实对方论据中的技术/估值假设。
        let bull_expert = match round_num {
            2 => "bull-r2",
            3 => "bull-r3",
            _ => "bull-researcher",
        };
        let bear_expert = match round_num {
            2 => "bear-r2",
            3 => "bear-r3",
            _ => "bear-researcher",
        };
        let bull_title = format!("多方研究员·第{round_num}轮");
        let bear_title = format!("空方研究员·第{round_num}轮");
        // 绝对坐标 = 容器基准 + 内部偏移
        let bull_x = DEBATE_X + 20.0;
        let bull_y = DEBATE_Y + 40.0 + (round * 2) as f64 * 180.0;
        let bear_x = DEBATE_X + 20.0;
        let bear_y = DEBATE_Y + 40.0 + (round * 2 + 1) as f64 * 180.0;

        // 多方辩手：首轮无前置辩论上下文，后续轮次引用所有前序辩论输出
        let mut bull_an =
            agent(&bull_id, &bull_title, bull_expert, Some("debate-bull-bear"), bull_x, bull_y);
        if let WorkflowNode::Agent(ref mut a) = bull_an {
            // R1 用 bull_tools 工具集(含 get_stock_quote/kline/financials/news 等基础数据工具,
            //   LLM 能直接调通拿数据,产出论据)。
            // R2/R3 走 PROFILE_TOOLS 路径(质询/反驳需技术评分+估值工具)。
            // 修复(v16):R1/R2/R3 多空辩手统一用 bull_tools(基础数据工具集)。
            //   之前 R2/R3 走 PROFILE_TOOLS(只有 compute_scoring / compute_valuation
            //   计算工具)—— R2/R3 没有上游数据节点,LLM 拿不到 stock_quote / kline
            //   / financials / news 等基础数据,工具调用全部返回空,导致 R2/R3
            //   输出 "暂无数据"。
            //   R2 质询 / R3 反驳的角色由 bull-r2.md / bear-r2.md / bull-r3.md /
            //   bear-r3.md prompt 控制,与工具集无关。
            // ⚠️ 上方「R2/R3 走 PROFILE_TOOLS」「统一用 bull_tools」两段均为历史
            //   记录，已被下方 `tools = vec![]`（纯决策节点）覆盖，
            //   勿据其推断实际工具集（自证注释污染）。
            // 修复(阶段 4):辩论子节点加 1 次重试。LLM 偶发超时/429 是单点失败
            //   主因,max_retries=0 导致整链雪崩(bear-r1 拿不到 bull-r1 上下文则
            //   后续轮次全部"暂无数据")。1 次重试覆盖 ~95% 瞬时失败。
            //   ⚠️ 同批加的 "180s 超时" 已被下方 `timeout = None` 覆盖（改回继承
            //   RunOptions.step_timeout）；且该 `max_retries=1` 直到 v48 才首次
            //   真正生效 —— 容器体路径此前是裸 dispatch，从未消费本配置（B1 修复）。
            a.base.retry = RetryConfig { enabled: true, max_retries: 1, ..Default::default() };
            // 超时继承 RunOptions.step_timeout（来自 agent_timeout_secs 设置），用户可在面板控制
            a.base.timeout = None;
            // P0 修复(2026-07-22): 移除 bull_tools，改为纯决策节点。
            // 原问题：bull_tools 含 td_quote/td_kline/td_fin 等需要 stock_code 的工具，
            // 但 input_mapping 未注入 stock_code（只有分析师评分），LLM 会传空值。
            // 辩手的 context_sources 已包含 analyst-brief 摘要（评分+关键论据），无需重新获取。
            a.config.tools = vec![];
            a.config.exposed_tools = vec![];
            a.config.system_prompt = format!(
                "{}\n\n--- 数据约束 ---\n\
                 你是辩论辩手，所有数据来自分析师摘要节点和前序辩论输出，禁止调用任何工具重新获取数据。\n\
                 基于分析师摘要中的评分数据和关键论据进行论证/质询/反驳。",
                a.config.system_prompt
            );
            a.config.max_tool_rounds = Some(0);
            // 注入前序轮次辩论输出 + analyst-brief 摘要作为上下文
            // （替代原先加载10份全量报告，由 analyst-brief CodeNode 聚合）
            let mut ctx: Vec<String> = Vec::new();
            for r in 1..round_num {
                ctx.push(format!("bull-r{r}"));
                ctx.push(format!("bear-r{r}"));
            }
            // 使用 analyst-brief CodeNode 的输出替代10份全量分析师报告
            ctx.push("analyst-brief".to_string());
            a.config.context_sources = ctx;
            // 注：评分数据已嵌入 analyst-brief 文本，不再额外通过 input_mapping 注入 30 个字段
            a.config.input_mapping = [].into_iter().collect();
        }
        nodes.push(bull_an);

        // 空方辩手：引用本轮多方输出 + 前序轮次辩论输出
        let mut bear_an =
            agent(&bear_id, &bear_title, bear_expert, Some("debate-bull-bear"), bear_x, bear_y);
        if let WorkflowNode::Agent(ref mut a) = bear_an {
            // 同 bull_an:R1/R2/R3 空方统一用 bull_tools。
            // 修复(阶段 4):同 bull_an,加 1 次重试 + 180s 超时,避免 LLM 瞬时失败
            //   导致辩论链雪崩(详见 bull_an 注释)。
            a.base.retry = RetryConfig { enabled: true, max_retries: 1, ..Default::default() };
            // 超时继承 RunOptions.step_timeout（来自 agent_timeout_secs 设置），用户可在面板控制
            a.base.timeout = None;
            // P0 修复(2026-07-22): 移除 bull_tools，改为纯决策节点。
            // 原问题：bull_tools 含 td_quote/td_kline/td_fin 等需要 stock_code 的工具，
            // 但 input_mapping 未注入 stock_code（只有分析师评分），LLM 会传空值。
            // 辩手的 context_sources 已包含 analyst-brief 摘要（评分+关键论据），无需重新获取。
            a.config.tools = vec![];
            a.config.exposed_tools = vec![];
            a.config.system_prompt = format!(
                "{}\n\n--- 数据约束 ---\n\
                 你是辩论辩手，所有数据来自分析师摘要节点和前序辩论输出，禁止调用任何工具重新获取数据。\n\
                 基于分析师摘要中的评分数据和关键论据进行论证/质询/反驳。",
                a.config.system_prompt
            );
            a.config.max_tool_rounds = Some(0);
            // 注入前序轮次 + 本轮多方输出 + analyst-brief 摘要作为上下文
            let mut ctx: Vec<String> = Vec::new();
            for r in 1..round_num {
                ctx.push(format!("bull-r{r}"));
                ctx.push(format!("bear-r{r}"));
            }
            ctx.push(bull_id.clone());
            // 使用 analyst-brief 替代全量分析师报告
            ctx.push("analyst-brief".to_string());
            a.config.context_sources = ctx;
            // 评分数据已嵌入 analyst-brief 文本，不再额外注入
            a.config.input_mapping = [].into_iter().collect();
        }
        nodes.push(bear_an);

        // ── 轮次依赖边 ──
        if round == 0 {
            // 首轮：从 DebateNode 容器出发
            edges.push(edge(&format!("e-debate-bull-r{round_num}"), "debate-bull-bear", &bull_id));
        } else {
            // 后续轮次：上一轮空方完成后启动本轮多方
            let prev_bear = format!("bear-r{}", round);
            edges.push(edge(&format!("e-r{round}-bull-r{round_num}"), &prev_bear, &bull_id));
        }
        // 每轮：多方 → 空方（空方看到多方论点后反驳）
        edges.push(edge(&format!("e-bull-r{round_num}-bear-r{round_num}"), &bull_id, &bear_id));
    }

    // ── debate-convergence（辩论收敛分析）──
    // 读取全部辩手输出（v48 起 2 个），输出 consensus_score 供 portfolio-mgr 公式使用。
    // 入边从 bear-r{debate_max_rounds} 出发，确保等真辩论结束后再启动收敛。
    // 出边到 value-investor 和 portfolio-mgr，确保收敛结果在决策前可用。
    {
        let last_debate_node = format!("bear-r{debate_max_rounds}");
        let mut dc = agent(
            "debate-convergence",
            "辩论结果收敛：consensus_score 聚合",
            "debate-convergence",
            None,
            500.0,
            1420.0,
        );
        if let WorkflowNode::Agent(ref mut a) = dc {
            // 动态构建 context_sources：仅引用辩手输出（6 轮 R1→R3 完整辩论轨迹）。
            //
            // v69 优化(2026-07-22): 移除 10 个分析师节点,context_sources 从 16 → 6。
            // 理由:
            //   1. debate-convergence prompt 的核心职责是"收敛辩论",evidence 源是 R1-R3
            //      辩手输出(含 report 全文 + VERDICT 标签),不依赖分析师原文。
            //   2. 辩手 R1(bull/bear-researcher.md)已消化分析师报告,prompt 明确写
            //      "所有原始信号已经在上游 10 位分析师的报告中",convergence 无需重复读取。
            //   3. 分析师的 bull_score/bear_score/consensus_score 通过 input_mapping
            //      独立通道注入(agent_executor.rs 中 context_sources 和 input_mapping
            //      是两条独立路径,input_mapping 从全局 blackboard 提取,不依赖
            //      context_sources),30 个评分字段仍可用。
            // 预期效果: input tokens ~30k-40k → ~15k-20k(减少约 50%),缓解 LLM TTFB 超时。
            let mut ctx: Vec<String> = Vec::new();
            for r in 1..=debate_max_rounds {
                ctx.push(format!("bull-r{r}"));
                ctx.push(format!("bear-r{r}"));
            }
            a.config.context_sources = ctx;
            // 纯决策节点：tools 默认为空（agent 闭包），无需工具调用轮次
            a.config.max_tool_rounds = Some(0);
            a.config.output_mode = OutputMode::Json; // 输出结构化 JSON，确保 consensus_score / aggregate_prediction 被 input_mapping 解析
            a.config.input_mapping = build_analyst_input_mapping(&a_ids);
            // #1 修复(2026-07-22): debate-convergence 上下文极大
            // (6 轮辩手 + 30 个 input_mapping 结构化字段, ~15k-20k input tokens),
            // LLM 处理大上下文 TTFB 偶发 >120s 触发 stream chunk timeout。
            // 1) stream_chunk_timeout_secs: 300s（5 分钟）— 单 chunk 等待余量
            // 2) base.timeout: 900s（15 分钟）— 节点级总超时,避免外层 600s 兜底先触发
            a.config.stream_chunk_timeout_secs = Some(300);
        }
        if let WorkflowNode::Agent(ref mut a) = dc {
            a.base.timeout = Some(900);
        }
        nodes.push(dc);
        edges.push(edge(
            &format!("e-{last_debate_node}-debate-convergence"),
            &last_debate_node,
            "debate-convergence",
        ));
    }

    // ── value-investor（巴菲特框架）：在辩论之后、与风险评估并行运行 ──
    // 入边从 bear-r{debate_max_rounds} 出发，确保等真辩论收敛后再启动
    // （debate-bull-bear 是 DebateNode 容器，立即 Completed，返回的是配置而非辩论结果）
    {
        let vi_id = "value-investor";
        let vi_title = "以巴菲特-芒格价值投资理念评估该标的，分析护城河、财务健康度、管理层、安全边际，输出结构化估值框架";
        let vi_y = 1540.0;
        let last_debate_node = format!("bear-r{debate_max_rounds}");
        let mut vi = agent(vi_id, vi_title, "value-investor", None, 20.0, vi_y);
        if let WorkflowNode::Agent(ref mut a) = vi {
            a.config.context_sources = vec![
                "a-fundamentals".into(),
                "a-research".into(),
                "a-sector".into(),
                // 改为辩论最后一轮空方的输出（真辩论结论），而非 DebateNode 容器
                last_debate_node.clone(),
                "debate-convergence".into(),
                // V60 修复(2026-07-23): 接入 t-valuation 客观估值数据
                // 原问题：context_sources 只含 LLM 叙述(a-fundamentals/a-research/a-sector)，
                // 没有结构化财务数据。LLM 拿不到 PE/ROE/FCF/增速等原始数字，
                // 无法计算 PEG 或相对估值，只能凭叙述"猜"内在价值，出于保守本能
                // 必然给出低于现价的估值（"目标价值很低"问题的根因）。
                // t-valuation 提供 result.dcf.{low,mid,high,upsidePct}、
                // result.graham.upsidePct、result.fScore.score、result.moat.label 等
                // 客观算法估值，作为 LLM 估值的锚点。
                "t-valuation".into(),
                // V73(2026-09-10): 接入 t-risk 结构化基本面硬数据。
                // value-investor 的护城河(35%)/财务健康(25%)两维度依赖 ROE/负债率/
                // 毛利率阈值判断，此前只能靠 a-fundamentals 的 LLM 叙述转述，
                // 幻觉直接污染 60% 权重的评分。t-risk(compute_portfolio_risk)
                // 的 stockRiskProfile.{roeTTMPct,debtRatioPct,grossMarginPct,
                // revenueGrowthYoYPct} 来自真实财报提取，使三维度全部硬数据锚定。
                "t-risk".into(),
            ];
            // P0 修复(2026-07-22): 移除所有工具，改为纯决策节点。
            // 原问题：tools 含 get_stock_financials/compute_valuation 等需要 stock_code
            // 的工具，但 input_mapping 未注入 stock_code，LLM 会传空值。
            // value-investor 的 context_sources 已包含 a-fundamentals/a-research/a-sector
            // + 辩论结果 + t-valuation，基本面和估值数据已通过上游注入。
            a.config.tools = vec![];
            a.config.exposed_tools = vec![];
            a.config.max_tool_rounds = Some(0);
            a.config.output_mode = OutputMode::Json;
            a.config.system_prompt = format!(
                "{}\n\n--- 数据约束 ---\n\
                 你是价值投资评估官，所有数据来自上游节点输出，禁止调用任何工具重新获取数据。\n\
                 - 基本面叙述: 来自 a-fundamentals（LLM 分析文本）\n\
                 - 研报数据: 来自 a-research\n\
                 - 行业数据: 来自 a-sector\n\
                 - 辩论共识: 来自 debate-convergence\n\
                 - **客观估值数据**: 来自 t-valuation（结构化算法结果）\n\
                   - result.dcf.{{low,mid,high}}: DCF 内在价值区间（不可用时为 null）\n\
                   - result.dcf.available / result.dcf.note: DCF 可用性与估值口径说明\n\
                   - result.dcf.upsidePct: **保守档**上行空间百分比（2026-09-23 起基准由 mid 改为 low）\n\
                     —— 即「**最保守增长假设下**」的折价幅度（正值=低估，负值=高估；不可用时为 null）\n\
                   - result.dcf.midUpsidePct: 中性档上行空间（**仅供叙述**「中性假设下能涨多少」；\n\
                     裁决口径一律用 upsidePct，不得用 midUpsidePct 或直接拿 mid 当「真实价值」）\n\
                   - result.graham.upsidePct: 格雷厄姆上行空间\n\
                   - result.fScore.score: Piotrosky F-Score（0-9，越高越好）\n\
                   - result.moat.label: 护城河评级\n\
                 - **结构化基本面硬数据**: 来自 t-risk（真实财报提取，V73 接入）\n\
                   - result.stockRiskProfile.roeTTMPct: ROE(TTM)百分比——护城河评级的权威依据（宽>20/窄15-20/无<15）\n\
                   - result.stockRiskProfile.debtRatioPct: 负债率百分比——财务健康度权威依据（<50健康/50-60良好/60-70一般/>70差）\n\
                   - result.stockRiskProfile.grossMarginPct: 毛利率百分比\n\
                   - result.stockRiskProfile.revenueGrowthYoYPct: 营收同比增速百分比\n\
                 \n\
                 **关键**: t-valuation 是客观算法估值，作为你的估值锚点。\n\
                 你的 intrinsic_value_range 应参考 result.dcf.{{low,mid,high}} 区间，\n\
                 margin_of_safety 应参考 result.dcf.upsidePct。\n\
                 对成长股，参考 result.dcf.upsidePct 判断是否「合理偏低」，\n\
                 不要一味给出低于现价的保守估值。\n\
                 \n\
                 **【硬不可用】处理（V74 原两条 + 2026-09-21 扩第 ③ 条）**:\n\
                 情形 ①  result.dcf.available=false\n\
                 情形 ②  result.dcf.upsidePct=null\n\
                   （①② 的含义是当期FCF≤0且近5年报无正净利年度＝持续亏损，\n\
                     DCF/格雷厄姆算法估值均不适用，value_signal=「无法估值」）\n\
                 情形 ③  result.dcf.assumptions.applicable=false\n\
                   含义：**模型前提对该标的不成立**。判据见 assumptions.applicability_signals\n\
                   —— 负债率 > 80%（净利由杠杆驱动），或净利为正但**当期真实自由现金流**\n\
                   与盈利量级脱钩（FCF/净利 < 0.3）/ 符号相反。此时三档数值**仍会出现**\n\
                   在 result.dcf 里，但它**不是可靠估值证据**，不得当作内在价值引用。\n\
                   实证 601166：DCF 给 +143% 上行空间，同一条输出里 LLM 给「观望/0% 仓位」。\n\
                 命中 ①②③ 任一即（合称**硬不可用**）：\n\
                 ① intrinsic_value_range 与 margin_of_safety 填 null；\n\
                 ② 理想买入价写「无算法估值锚，需采用清算价值/重置成本等替代方法」；\n\
                 ③ **禁止输出 0 元买入价、0.00 元 DCF 或 0% 安全边际冒充算法估值**——\n\
                 估值不可用 ≠ 估值为 0，把 null 当 0 是数据语义污染。\n\
                 \n\
                 **锚定口径披露＝【软衰减】（2026-09-21 新增；与上段并列，勿并入硬不可用）**:\n\
                 result.dcf.assumptions.is_fallback_anchor=true 表示锚定**不是当期真实自由现金流**，\n\
                 而是「近 5 年年报正净利均值 × 0.90」的历史代理（口径原文见 assumptions.basis）。\n\
                 该代理回溯且**系统性偏低** —— 实测 300308（2026-09-21）代理锚比同期 TTM 自由\n\
                 现金流低约 5.6 倍，对成长/转型标的尤甚。此时数值**可以引用**（三个字段照常填），\n\
                 但必须：① 在 report 中点明口径，不得陈述成与当期现金流等价的结论；\n\
                 ② 在 risk_flags 中加一条标注该口径；③ 相应**下调 confidence**。\n\
                 与情形 ③ 同时命中时（真实 FCF ≤ 0 会同时触发两者）**按硬不可用处理**。\n\
                 \n\
                 **分歧处理（2026-09-21 新增）**: 算法锚（result.dcf.upsidePct / value_signal）\n\
                 与你的判断分歧时，**照实报告分歧**并说明你的理由 —— 不要把算法没给的数字\n\
                 说成是算法给的。实测 300308（2026-09-21）产出过\n\
                 「算法 value_signal 为『合理偏低』，但绝对估值锚与现价偏离超 80%」\n\
                 这种自相矛盾表述（当时 value_signal 的评分函数无法表达高估，已修）。",
                a.config.system_prompt
            );
            // 环 A: 注入历史反思教训
            a.config.input_mapping =
                std::collections::HashMap::from([("stock_lessons".into(), "stock_lessons".into())]);
        }
        nodes.push(vi);
        edges.push(edge("e-debate-value-investor", &last_debate_node, vi_id));
        // value-investor 的 context_sources 中 debate-convergence 需要显式边，
        // 否则只在 bear-r3 完成后就调度，debate-convergence 还没跑完
        edges.push(edge("e-convergence-value-investor", "debate-convergence", vi_id));
        // V60 修复: t-valuation 加入 context_sources，需要显式边等待其完成，
        // 否则 t-valuation 的输出不会进入 value-investor 的变量池。
        // 拓扑链：bear-r3 → t-scoring → t-valuation → value-investor
        edges.push(edge("e-valuation-value-investor", "t-valuation", vi_id));
        // V73: t-risk 同理——context_sources 里的节点必须显式边等待，
        // 否则 compute_portfolio_risk 的输出不进变量池。
        // 拓扑链：t-valuation → t-risk → value-investor（无回环，t-risk 仅依赖 t-valuation）
        edges.push(edge("e-t-risk-value-investor", "t-risk", vi_id));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 【装饰节点 / Decorative Container】p-risk-assess
    // ═══════════════════════════════════════════════════════════════════════
    // 语义：风险评估的视觉分组容器，包裹 3 个并行风险偏好 Agent
    // 调度：与 p-analysts 相同——容器立即 Completed，子节点独立调度
    //      - aggressive-debator / conservative-debator / neutral-debator
    //      - 3 个子节点共享同一份风险输入（来自聚合后的辩论+分析师输出）
    //      - 实际依赖通过显式 edge 表达：e-bear-r3 → p-risk-assess（容器）→ 3 个子节点
    //        容器完成是"瞬时"的，3 个子节点会同时被引擎解锁
    // parent_id：仅供前端编辑器嵌套渲染用
    //
    // 与 p-analysts 的区别：
    //   p-analysts 包裹 9 组 (Tool+Agent) 强调"数据预拉 + 分析"两阶段
    //   p-risk-assess 包裹纯 Agent，强调"同输入多视角并行评估"
    //
    // ⚠️ 坐标约定（FIX: 所有节点位置为画布绝对坐标）：
    //   容器 p-risk-assess 放在 (RISK_X, RISK_Y)
    //   子节点 x = RISK_X + 20px, y = RISK_Y + 40px + i*180px
    //   前端 WorkflowEditor 通过 parentId 减去容器坐标得到相对坐标。
    // ═══════════════════════════════════════════════════════════════════════
    const RISK_X: f64 = 300.0;
    const RISK_Y: f64 = 1800.0;
    nodes.push(WorkflowNode::Parallel(ParallelNode {
        base: WorkflowNodeBase {
            id: "p-risk-assess".into(),
            // F-3 修复: 原本 title="风险评估" 与下面的 t-risk (compute_portfolio_risk) 同名，
            // 编辑器画布上无法区分视觉分组与单 tool。改为"三档风险评估分组"。
            title: "三档风险评估分组".into(),
            description: Some("三种风险偏好并行评估".into()),
            position: Position { x: RISK_X, y: RISK_Y },
            retry: RetryConfig::default(),
            timeout: Some(600),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: ParallelNodeConfig {
            branches: vec![
                Branch {
                    id: "risk-agg".into(),
                    title: "激进评估".into(),
                    steps: vec!["risk-agg".into()],
                    branch_timeout_ms: None,
                    // P0 修复: 改用 UseDefault，超时后注入 null 到 workflow.results，
                    // 确保下游 research-mgr 的 context_sources 能找到 risk-agg 变量，
                    // 避免 "context_sources 变量未在 context.variables 中找到" ERROR。
                    degrade_strategy: DegradeStrategy::UseDefault,
                },
                Branch {
                    id: "risk-con".into(),
                    title: "保守评估".into(),
                    steps: vec!["risk-con".into()],
                    branch_timeout_ms: None,
                    degrade_strategy: DegradeStrategy::UseDefault,
                },
                Branch {
                    id: "risk-neu".into(),
                    title: "中性评估".into(),
                    steps: vec!["risk-neu".into()],
                    branch_timeout_ms: None,
                    degrade_strategy: DegradeStrategy::UseDefault,
                },
            ],
            wait_for_all: true,
            aggregation: Some(MergeStrategy::All),
            auto_input_from_parent: false,
            timeout: Some(600),
            sub_graph: None, // v23+：稍后通过 inject_container_subgraphs 注入
        },
    }));
    edges.push(edge(
        "e-debate-p-risk-assess",
        &format!("bear-r{debate_max_rounds}"),
        "p-risk-assess",
    ));
    // risk 节点的 context_sources 中 bull-r3/t-scoring/t-valuation/debate-convergence
    // 需要显式边等待；否则 bear-r3 完成后就调度，但缺少边连接的节点输出
    // 不会进入 deps_results/exec_ctx.variables，导致 context_sources 报 ERROR。
    // 注：t-valuation 虽已可到达（链 bear-r3→t-scoring→t-valuation），但无直接边
    // 则 bull-r3/t-scoring 的输出不进入变量池。
    edges.push(edge(
        &format!("e-bull-r{debate_max_rounds}-p-risk-assess"),
        &format!("bull-r{debate_max_rounds}"),
        "p-risk-assess",
    ));
    edges.push(edge("e-scoring-p-risk-assess", "t-scoring", "p-risk-assess"));
    edges.push(edge("e-valuation-p-risk-assess", "t-valuation", "p-risk-assess"));
    edges.push(edge("e-convergence-p-risk-assess", "debate-convergence", "p-risk-assess"));

    for (i, (rid, rtitle, rexpert, _rtools)) in [
        (
            "risk-agg",
            "以最激进的风险偏好评估该股票",
            "aggressive-debator",
            vec![
                td_score.clone(),
                td_risk.clone(),
                td_maxdd.clone(),
                td_var.clone(),
                td_kelly.clone(),
                td_mc.clone(),
                td_divergence.clone(),
            ],
        ),
        (
            "risk-con",
            "以最保守的风险偏好评估该股票",
            "conservative-debator",
            vec![
                td_score.clone(),
                td_risk.clone(),
                td_sharpe.clone(),
                td_maxdd.clone(),
                td_pledge.clone(),
                td_corr.clone(),
                td_divergence.clone(),
            ],
        ),
        (
            "risk-neu",
            "以中性风险偏好评估该股票",
            "neutral-debator",
            vec![
                td_score.clone(),
                td_risk.clone(),
                td_val.clone(),
                td_pe_pct.clone(),
                td_peg.clone(),
                td_rp.clone(),
                td_ind.clone(),
                td_divergence.clone(),
            ],
        ),
    ]
    .iter()
    .enumerate()
    {
        let risk_y = RISK_Y + 40.0 + i as f64 * 180.0;
        let risk_x = RISK_X + 20.0;
        let mut an = agent(rid, rtitle, rexpert, Some("p-risk-assess"), risk_x, risk_y);
        if let WorkflowNode::Agent(ref mut a) = an {
            // P0 修复(2026-07-22): 移除 rtools，改为纯决策节点。
            // 原问题：rtools 含 td_score/td_risk/td_val/td_sharpe 等需要 stock_code 或
            // kline_json 的工具，但 input_mapping 未注入 stock_code，LLM 会传空值。
            // 风险评估的 context_sources 已包含所有分析师报告 + 辩论结果 + t-scoring/
            // t-valuation/t-risk，数据已完整注入，无需重新计算。
            a.config.tools = vec![];
            a.config.max_tool_rounds = Some(0);
            a.config.system_prompt = format!(
                "{}\n\n--- 数据约束 ---\n\
                 你是风险评估师，所有数据来自上游分析师报告、辩论结果和技术评分节点，禁止调用任何工具重新获取或计算。\n\
                 - 技术评分: 来自 t-scoring\n\
                 - 估值数据: 来自 t-valuation\n\
                 - 风险评分: 来自 t-risk\n\
                 基于上述数据以{}评估该股票的风险。",
                a.config.system_prompt, rtitle
            );
            // 修复：风险评估 Agent 需要读到上游分析师报告 + 辩论结果 + 技术指标，
            // 否则 LLM 没有分析素材，不会主动调用工具。
            a.config.context_sources = vec![
                "a-market-analyst".into(),
                "a-sentiment".into(),
                "a-news".into(),
                "a-fundamentals".into(),
                "a-policy".into(),
                "a-hot-money".into(),
                "a-lockup".into(),
                "a-research".into(),
                "a-sector".into(),
                "a-catalyst".into(),
                format!("bull-r{debate_max_rounds}"),
                format!("bear-r{debate_max_rounds}"),
                "debate-convergence".into(),
                "t-scoring".into(),
                "t-valuation".into(),
            ];
            a.config.input_mapping = {
                let mut m = build_analyst_input_mapping(&a_ids);
                // 注入辩论收敛的 consensus_score 供 Kelly 公式使用
                // 路径规则（V29 修复）：AgentNode 输出包裹在 {role, content: <json_string>, ...} 中，
                // resolve_var_path 遇到 Value::String 会自动 from_str 解析后再继续下钻，
                // 因此必须用 .content.field 路径访问 AgentNode 业务字段。
                m.insert(
                    "consensus_score".to_string(),
                    "debate-convergence.content.consensus_score".to_string(),
                );
                m
            };
        }
        nodes.push(an);
        // p-risk-assess 容器 → 子节点依赖边：防止子节点被独立调度
        edges.push(edge(&format!("e-p-risk-{rid}"), "p-risk-assess", rid));
    }

    // ── AggregatorNode: 聚合三种风险偏好评估结果 ──
    nodes.push(WorkflowNode::Aggregator(AggregatorNode {
        base: WorkflowNodeBase {
            id: "agg-risk".into(),
            title: "风险偏好聚合".into(),
            description: Some("聚合激进/保守/中性三种风险偏好评估".into()),
            position: Position { x: 300.0, y: 2400.0 },
            retry: RetryConfig::default(),
            timeout: Some(60),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: AggregatorNodeConfig {
            strategy: "all".into(),
            input_sources: vec!["risk-agg".into(), "risk-con".into(), "risk-neu".into()],
            output_var: "risk-aggregated".into(),
            wait_for_all: true,
            weights: vec![],
            summarize_prompt: None,
            summarize_model: None,
            sub_graph: None,
        },
    }));
    for rid in &["risk-agg", "risk-con", "risk-neu"] {
        edges.push(edge(&format!("e-{rid}-agg-risk"), rid, "agg-risk"));
    }

    // ── P1-3: 三档风险辩论收敛（agg-risk 之后、算法工具之前）──
    // 读取三方风险评估输出，分析分歧并生成收敛报告。
    // 收敛输出结构见 risk-convergence.md
    {
        let mut rc = agent(
            "risk-convergence",
            "三档风险辩论收敛：分歧分析与综合裁决",
            "risk-convergence",
            None,
            300.0,
            2550.0,
        );
        if let WorkflowNode::Agent(ref mut a) = rc {
            a.config.context_sources =
                vec!["risk-agg".into(), "risk-con".into(), "risk-neu".into()];
            // 纯决策节点：tools 默认为空（agent 闭包），无需工具调用轮次
            a.config.max_tool_rounds = Some(0);
            a.config.output_mode = OutputMode::Json; // V54 修复: 纯JSON输出,使 content.disagreement_score 可解析
        }
        nodes.push(rc);
        edges.push(edge("e-agg-risk-risk-convergence", "agg-risk", "risk-convergence"));
    }

    // ── 算法 Tool 节点：仅 3 个核心评分/估值/风控（独立画布节点，parent_id = None）──
    // 位置：risk-convergence 节点 (300, 2550) 之后横排，间距 180
    // 位置：agg-risk 节点 (300, 2400) 之后横排，间距 180
    //
    // C2 路径 Z(2026-09-12)：`t-valuation` 额外接入设置面板的 3 个 `value_dcf_*` 变量。
    // 这是「面板可写但零处生效」缺陷的修复点 —— 此前 6 个 `value_*` 变量在整个生产链
    // 无任何消费方，面板改值对估值零影响。
    // 变量默认值见 `seed_variables.rs`（12.0 / 4.0 / 8.5，**百分数**口径，与后端
    // `ValuationConfig::from_flat_arguments` 的 D1 约定一致）；键名（工具参数）与变量名
    // 刻意不同名，避免「参数名 == 变量名」的巧合掩盖映射错误。
    const VALUATION_FLAT_ARGS: [(&str, &str); 3] = [
        ("dcf_growth_rate", "value_dcf_growth_rate"),
        ("dcf_perpetual_rate", "value_dcf_perpetual_rate"),
        ("dcf_discount_rate", "value_dcf_discount_rate"),
    ];
    let algo_tools: &[AlgoToolRow] = &[
        ("t-scoring", "技术评分", "compute_scoring", "stock_code", &[], 300.0, 2700.0),
        (
            "t-valuation",
            "估值计算",
            "compute_valuation",
            "stock_code",
            &VALUATION_FLAT_ARGS[..],
            480.0,
            2700.0,
        ),
        // F-3 修复: title 由 "风险评估" 改为 "组合风险计算"，避免与
        // 上面的 p-risk-assess 容器（"三档风险评估分组"）同名混淆。
        ("t-risk", "组合风险计算", "compute_portfolio_risk", "stock_codes", &[], 660.0, 2700.0),
    ];
    for (tool_id, title, tool_name, arg_key, extra, x, y) in algo_tools {
        nodes.push(tool_node(tool_id, title, tool_name, tool_id, arg_key, extra, None, *x, *y));
    }
    // ⚠️ P0 修复(2026-09-14,v48 回归)：**源节点必须参数化**。
    //   曾写死 `edge("e-bear-r3-t-scoring", "bear-r3", "t-scoring")` ——
    //   v48 把 debate_max_rounds 改成 1 后 bull-r2/bear-r2/bull-r3/bear-r3 不再生成，
    //   这条边就成了**悬空入边**：`create_workflow` 直接拒绝启动，报
    //   `Node 't-scoring' depends on non-existent 'bear-r3'`（整条链连建模都过不去）。
    //   边名同为写死的 `e-bear-r3-...`，一并参数化，避免「名字撒谎」。
    //   守护测试：`seed_consistency_tests::debater_round_refs_are_parameterized`。
    edges.push(edge(
        &format!("e-bear-r{debate_max_rounds}-t-scoring"),
        &format!("bear-r{debate_max_rounds}"),
        "t-scoring",
    ));
    edges.push(edge("e-t-scoring-t-valuation", "t-scoring", "t-valuation"));
    edges.push(edge("e-t-valuation-t-risk", "t-valuation", "t-risk"));

    // ── V79(2026-09-21)：估值分位带（独立 ToolNode，供给 f5 的**第三条锚腿**）──
    // 调 **MCP 工具** `compute_valuation_band`（`crates/astock-data/src/mcp_tools.rs`）：
    //   · 在线取东财历史估值日序列（`AStockClient::get_valuation_history`，12h 缓存）
    //     → 按窗口裁剪 → 算 PE/PB/PS 分位带 + 当前分位；**不落库**。
    //
    // ⚠️ 曾经写错、且错了也**没有任何报错**（V79 首次实现，同批修）：
    //   初版把这个工具名当成了**命令层**的同名函数
    //   `commands::stock_analysis::compute_valuation_band`。但 `#[agent_command]` 宏
    //   （`crates/axagent-agent-macro/src/lib.rs:45-64`）只生成 `CommandMetadata` 并注册元数据，
    //   其唯一消费者是命令文档生成测试 —— **全仓不存在「命令 → 工具」的桥**。
    //   而 ToolNode 走 `ToolRegistry`（`rt-workflow/.../tool_executor.rs:122-140`），
    //   表内条目由 `crates/tools/src/tools/astock_data.rs:118` 从 `stock_mcp_tools()`
    //   逐个包装 ⇒ `ToolResolver` 返 `None` ⇒ `core.rs` 的 Failed 分支 `emit degraded: true`
    //   **静默吞掉**（节点 completed、输出为空、无报错）⇒ `valuation_pe_percentile` 恒空
    //   ⇒ 这条腿成了**永久空壳**、逐位退化回 0.7/0.3。
    //   现工具已进 `stock_mcp_tools()` schema 与 dispatch **两处成对**；
    //   `seed_consistency_tests` 的门禁也已扩到能看见本节点的**位置参数**形态。
    //
    // ⚠️ 为什么必须放进**工作流**、而不是等用户点前端「估值带」图：
    //   前端那张图走的是**命令**（读/回填本机表），工作流这条腿走**工具**（在线、不落库）；
    //   决策正确性不该挂在「用户是否点过某个 UI」上。两者窗口口径共享，故结论一致。
    //   该表此前的**唯一**回填触发点就是前端那个命令（DB 实查全库只有 13 只有数据）——
    //   这也是「不要把决策依赖塞进 UI 路径」的实证。
    //
    // ⚠️ 与 `algo_tools` 表**分开声明**（刻意不往那张表里加行）：`algo_tools` 有两处
    //   下游消费者 —— `raw-data` 的 `input_sources` 与「给每个 tool 节点显式挂边」的循环，
    //   两者都按行数生成内容；加一行会连带把本节点拉进 raw 聚合，并让 aggregator 的
    //   description 里写死的「16 个工具节点（12 个数据源 + 3 个算法 + 1 个龙虎榜）」计数腐烂。
    //   本节点的**唯一**消费者是 portfolio-mgr（经 `input_mapping`），无需进 raw 聚合。
    let valuation_band_id = "t-valuation-band";
    nodes.push(tool_node(
        valuation_band_id,
        "估值分位带",
        "compute_valuation_band",
        valuation_band_id,
        "stock_code",
        &[],
        None,
        1020.0, // x: 接在 t-valuation (480) / t-risk (660) / dragon-tiger (840) 之后的第四格
        2700.0, // y: 与 algo_tools 同行
    ));
    // 入边 `t-valuation → t-valuation-band`：估值算完再算分位。分位在数据上并不依赖估值，
    //   挂这条边是为了让它落在同一条水平工具链上（画布不出现孤立节点），
    //   且**不会**构成回环（本节点无出边指向 t-risk / raw-data）。
    edges.push(edge("e-t-valuation-t-valuation-band", "t-valuation", valuation_band_id));
    // ── P1/P2 新增: 龙虎榜数据获取节点（独立 ToolNode，不配 Agent）──
    // 独立创建以保持 raw-data 聚合的完整性，同时直接供给 portfolio-mgr 做筹码面分析增强。
    // 龙虎榜数据包含机构席位买卖动向、游资席位上榜频率等，是 f10 筹码面因子的重要补充。
    let dragon_tiger_id = "t-dragon-tiger-data";
    nodes.push(tool_node(
        dragon_tiger_id,
        "获取个股龙虎榜明细",
        "get_stock_dragon_tiger",
        dragon_tiger_id,
        "stock_code",
        &[],
        None,
        840.0,  // x: 接在 t-risk (660) 之后
        2700.0, // y: 与 algo_tools 同行
    ));
    // ⚠️ 不给 t-dragon-tiger-data 挂 bear-r3 入边（v4 修复 Cycle detected）：
    //   它的消费方 a-hot-money 在辩论链上游（a-hot-money → debate-bull-bear → … → bear-r3），
    //   若再挂 bear-r3 → t-dragon-tiger-data 入边就构成回环
    //   a-hot-money → debate 链 → bear-r3 → t-dragon-tiger-data → a-hot-money，
    //   引擎 create_workflow 的 Kahn 检测直接拒绝启动（"Cycle detected in workflow"）。
    //   作为入度 0 的启动节点（与其他 t-* 数据工具一致），它天然先于 a-hot-money 完成，
    //   数据依赖（a-hot-money 的 context_sources 消费 dragon_tiger 变量）由出边保证。

    // ── 阶段2（PROPOSAL-stock-decision-four-horizon.md）：多周期评分节点 ──
    // 给 portfolio-mgr 的短/中/长档注入**各自周期**的技术评分，供 decisionsByHorizon 用 f1
    //   独立重融合（rhai 侧读取 totalScore_short / totalScore_mid / totalScore_long）。
    // 复用老 t-scoring 同一工具 `compute_scoring`，仅以静态参数 `period` 区分数据周期
    //   （2.1 已在 astock-data/mcp_tools.rs 为 compute_scoring 增加 period 支持）。
    // 入边链 t-scoring → t-scoring-week → t-scoring-month：既是评分周期的先后（周→月），
    //   也让新节点有来源边、避免被 validate_workflow 判为 orphan（同 t-valuation-band 先例）。
    // ⚠️ 不并入 `algo_tools` / `raw_input_sources`：raw-data 聚合的「16 个工具节点」计数与
    //   description 是写死的，且这两档评分只供 portfolio-mgr 消费，无需进 raw 聚合。
    nodes.push(tool_node(
        "t-scoring-week",
        "技术评分（周线）",
        "compute_scoring",
        "t-scoring-week",
        "stock_code",
        &[("period", "weekly")],
        None,
        1140.0,
        2700.0,
    ));
    nodes.push(tool_node(
        "t-scoring-month",
        "技术评分（月线）",
        "compute_scoring",
        "t-scoring-month",
        "stock_code",
        &[("period", "monthly")],
        None,
        1260.0,
        2700.0,
    ));
    edges.push(edge("e-t-scoring-t-scoring-week", "t-scoring", "t-scoring-week"));
    edges.push(edge("e-t-scoring-week-t-scoring-month", "t-scoring-week", "t-scoring-month"));
    // 长档复用月线评分（月线已是最长期权周期，不新增季度节点——阶段2按此拍板）
    edges.push(edge("e-t-scoring-week-portfolio-mgr", "t-scoring-week", "portfolio-mgr"));
    edges.push(edge("e-t-scoring-month-portfolio-mgr", "t-scoring-month", "portfolio-mgr"));

    // ── P0-H L3（2026-09-12）: 报告导出所需的 2 个数据节点 ──
    // 背景：`generate_stock_report` 的「机构调研」与「大盘指数」两个板块此前恒空。
    // 实测（603353, 模板 v35）确认这两个数据源**工具已实现且上游可用**，只是模板从未抓取：
    //   · `get_index_quotes`                —— 实测 push2his `stock/get` 返回真实点位
    //                                          （上证指数 3888.11 / −1.18%）
    //   · `get_stock_institutional_visits`  —— 数据源修好报表名后（见
    //                                          `vendors/eastmoney.rs`）`RPT_ORG_SURVEY`
    //                                          返回 20 条真实调研记录
    // 故各挂一个独立 ToolNode：既供报告导出消费，也随 raw-data 聚合进入下游上下文。
    //
    // ⚠️ 命名契约：这两个 id 与 `raw.combined.result[].node_id` 一一对应，
    //    报告命令 `stock_analysis.rs::generate_stock_report` **按 node_id 取值**，
    //    改名会同时打断报告导出（该处有同步注释）。
    //
    // 同时**不新增**「同行对比 / 期权PCR」两个节点（报告已删这两个板块），理由实测如下：
    //   · 同行对比 —— `get_peers` 取的是「精准**概念**板块」而非行业。实测 603353
    //     （和顺石油，加油站零售）返回的「同行」是高压快充/存储芯片/先进封装/半导体概念
    //     （因其 2026-03-19 收购奎芯科技的公告被打上半导体标签）⇒ 数据源可用但**语义错误**，
    //     渲染出来会主动误导用户。修它属独立的数据源语义缺陷（已登记）。
    //   · 期权PCR —— `get_option_pcr` 对**非 ETF 代码硬编码 `return Ok(None)`**
    //     （实现注释明写「个股期权 PCR 无稳定公开 API」）⇒ 在任何个股上都是代码级死路径。
    const REPORT_EXTRA_TOOL_IDS: [&str; 2] = ["t-index-quotes", "t-institutional-visits"];
    nodes.push(tool_node_noarg(
        REPORT_EXTRA_TOOL_IDS[0],
        "获取大盘指数行情",
        "get_index_quotes",
        840.0,  // x: 接在 t-dragon-tiger-data (840, 2700) 下方
        2880.0, // y: 单独一行，避免与 algo_tools / dragon-tiger 重叠
    ));
    nodes.push(tool_node(
        REPORT_EXTRA_TOOL_IDS[1],
        "获取机构调研记录",
        "get_stock_institutional_visits",
        REPORT_EXTRA_TOOL_IDS[1],
        "stock_code",
        &[],
        None,
        1020.0,
        2880.0,
    ));
    // 与 `tool_assignments` 下的其他 t-* 数据工具一致：显式挂 trigger 入边，
    // 让它们在 DAG 里是「有来源的启动节点」，而不是无入边的孤立节点
    // （`validate_workflow` 的 orphan / data_blackhole 类规则对无入边工具节点会告警）。
    // 无环：trigger 无入边，新节点只流向 raw-data。
    for id in REPORT_EXTRA_TOOL_IDS {
        edges.push(edge(&format!("e-trigger-{id}"), "trigger", id));
    }

    // ── P3 (real-nodes): raw-data 聚合节点 ──
    // 把 13 个 t-* / algo 工具节点的输出聚合成单个 raw 对象，供 portfolio-mgr 决策时
    // 通过 context_sources 读取 "raw-data-aggregated" 变量。
    //
    // F-5 修复: 显式追加 e-raw-data-portfolio-mgr 边。
    //   原设计 raw-data 入度 12、出度 0，仅靠 portfolio-mgr.context_sources 消费。
    //   1) 上游 validate_workflow 会把 raw-data 标为"data_blackhole"硬错误
    //   2) 画布上 raw-data 与 portfolio-mgr 之间无连线，可视化上看像断头
    //   aggregator 节点本身是纯数据合并（不调 LLM），调度等待成本可忽略；
    //   加边后 portfolio-mgr 启动前的等待时间依然是 max(trader, raw-data)，
    //   raw-data 远快于 trader，无可观察的延迟变化。
    let raw_input_sources: Vec<String> = algo_tools
        .iter()
        .map(|(id, _, _, _, _, _, _)| id.to_string())
        .chain(tool_assignments.iter().map(|(id, _, _, _)| id.to_string()))
        .chain(std::iter::once(dragon_tiger_id.to_string()))
        // P0-H L3（2026-09-12）：新增 t-index-quotes / t-institutional-visits
        .chain(REPORT_EXTRA_TOOL_IDS.iter().map(|s| s.to_string()))
        .collect();
    nodes.push(WorkflowNode::Aggregator(AggregatorNode {
        base: WorkflowNodeBase {
            id: "raw-data".into(),
            title: "原始数据聚合".into(),
            description: Some(
                "聚合 16 个工具节点的原始输出（12 个数据源 + 3 个算法 + 1 个龙虎榜）".into(),
            ),
            position: Position { x: 840.0, y: 2700.0 },
            retry: RetryConfig::default(),
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: AggregatorNodeConfig {
            strategy: "all".into(),
            input_sources: raw_input_sources,
            output_var: "raw-data-aggregated".into(),
            wait_for_all: true,
            weights: vec![],
            summarize_prompt: None,
            summarize_model: None,
            sub_graph: None,
        },
    }));
    // 修复 Defect #8: 为 raw-data 显式添加 13 个 tool 节点的入边。
    // 之前只有 1 条 e-t-risk-raw-data 边，依赖关系是"隐性"的（依赖 t-risk 是
    // 13 个 tool 节点中最深的间接前置）。改成显式声明后，调度器会等待所有 13
    // 个上游 tool 节点都完成才启动 raw-data，input_sources 才有数据可读。
    // 迭代器自然包含 e-t-risk-raw-data（来自 algo_tools 末项）。
    for src in algo_tools
        .iter()
        .map(|(id, _, _, _, _, _, _)| *id)
        .chain(tool_assignments.iter().map(|(id, _, _, _)| *id))
        .chain(std::iter::once(dragon_tiger_id))
        // P0-H L3（2026-09-12）：新增 2 个数据节点同样需要显式入边，
        // 否则调度器不会等它们完成，raw-data 聚合时 input_sources 取不到值。
        .chain(REPORT_EXTRA_TOOL_IDS.iter().copied())
    {
        edges.push(edge(&format!("e-{src}-raw-data"), src, "raw-data"));
    }
    // F-5: 显式出边到 portfolio-mgr，让上游 validate_workflow 的"data_blackhole"
    //      规则不再误报，同时让画布上能看到 raw-data → portfolio-mgr 的连线。
    //      注意：portfolio-mgr 已改为 CodeNode，不设 context_sources，
    //      raw-data 通过显式边 e-raw-data-portfolio-mgr 确保调度依赖。
    edges.push(edge("e-raw-data-portfolio-mgr", "raw-data", "portfolio-mgr"));

    // ── CodeNode: 风险等级确定性分类（v55：原 LlmClassifierNode 下沉 Rhai）──
    // 口径来源 = 原 LlmClassifier 的 prompt 原文，已完整保留在 `risk-level.rhai` 头部注释，
    // 阈值 / 档位 / 优先级 1:1 转写，未做任何调整。改因与实测差异见本文件 v55 段说明。
    {
        let rl_code = include_str!("../risk-level.rhai").to_string();
        nodes.push(WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                // 节点 id 保持不变 —— 三条链按它索引，改名会断链：
                //   ① 边 e-t-risk-cls-risk / e-cls-risk-level-portfolio-mgr
                //   ② 前端 stockWorkflowChatBridge / stockAnalysisStore 的阶段映射与回放集合
                //   ③ research-mgr 的 context_sources 引用的是 output_var（"risk-level"），
                //      与节点 id 不同名，两者都不可动
                id: "cls-risk-level".into(),
                title: "风险等级分类".into(),
                description: Some(
                    "按阈值规则做确定性风险等级分类（Rhai），输出风险等级与判定依据".into(),
                ),
                position: Position { x: 300.0, y: 3000.0 },
                // 确定性脚本：无网络、无 LLM ⇒ 不存在「瞬时故障可重试」的场景，重试已无意义
                // （原 LLM 版因 agnes-3.0-flash 34.95s > 30s 超时曾开 2 次重试，见 v9 记录）。
                retry: RetryConfig::default(),
                // 原 60s 是给 LLM 的；Rhai 纯计算实测 <10ms，与 data-quality 同样留 10s 余量。
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                // 原为 false，与「其失败不应有任何阻断性代价」的声明**不符** —— v9 注释自己
                // 记录了它曾阻塞 v-validate 造成全链死锁。Rhai 版不会失败，保留 true 作双保险：
                // 万一脚本异常，portfolio-mgr 仍有算法分类兜底
                // （`portfolio-mgr.rhai` 的 `present(overall_risk_llm)` 守卫会走"无数据"分支），
                // 不会像 cof=false 那样锁死整条决策链。
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: rl_code,
                // research-mgr 的 context_sources 按此变量名索引
                output_var: "risk-level".into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    // t-risk 是 ToolNode ⇒ stockRiskProfile 在 result.content 中
                    (
                        "risk_volatility",
                        "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct",
                    ),
                    ("risk_drawdown", "t-risk.result.content.stockRiskProfile.maxDrawdownPct"),
                    ("risk_sharpe", "t-risk.result.content.stockRiskProfile.sharpeRatio"),
                    ("risk_roe", "t-risk.result.content.stockRiskProfile.roeTTMPct"),
                    // 毛利率：原 prompt 的高风险条件 A（毛利率<10%）与低风险条件
                    // （毛利率>20%）都要用它 ⇒ 本节点必须注入。
                    // ⚠️ 与 portfolio-mgr 的同名映射无关：后者在 v54 已删除（其脚本不读该变量），
                    //    本节点是**独立**映射，删除其一不影响另一。
                    ("risk_gross_margin", "t-risk.result.content.stockRiskProfile.grossMarginPct"),
                    ("risk_debt_ratio", "t-risk.result.content.stockRiskProfile.debtRatioPct"),
                    (
                        "risk_revenue_growth",
                        "t-risk.result.content.stockRiskProfile.revenueGrowthYoYPct",
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }));
    }
    edges.push(edge("e-t-risk-cls-risk", "t-risk", "cls-risk-level"));

    // ── Validation: 结果完整性校验 ──
    nodes.push(WorkflowNode::Validation(ValidationNode {
        base: WorkflowNodeBase {
            id: "v-validate".into(),
            title: "结果完整性校验".into(),
            description: Some("确保分析报告包含必要字段，缺失时降级处理".into()),
            position: Position { x: 300.0, y: 3300.0 },
            retry: RetryConfig::default(),
            timeout: Some(60),
            enabled: true,
            parent_id: None,
            compensation: None,
            // v10: 上游 t-risk（ToolNode）失败时仍派发本节点——断言 exists 失败走
            // on_fail=skip → Skipped（计入完成集），下游 data-quality/research-mgr
            // 不被阻塞。cof=false 时 t-risk Failed 会永久阻塞 v-validate（死锁）。
            continue_on_fail: true,
        },
        config: ValidationNodeConfig {
            assertions: vec![ValidationAssertion {
                assertion_type: "exists".into(),
                expected: None,
                actual: Some("t-risk.output".into()),
                expression: None,
            }],
            on_fail: "skip".into(),
            max_retries: 1,
        },
    }));
    // v9 修复(2026-09-08 死锁实证)：原边 cls-risk-level → v-validate 是死锁根源——
    // v-validate 的 continue_on_fail=false，cls-risk-level 一旦 Failed，该 Direct 边
    // 永久阻塞 v-validate，进而锁死 data-quality/research-mgr/trader/portfolio-mgr
    // 整条决策链（引擎死锁检测只做单遍 Skipped 传播后直接 break）。
    // v-validate 校验的是 t-risk.output（断言 exists），改为依赖 t-risk 语义更正确。
    // ⚠️ v55(2026-09-20) 订正：原注释称本节点「只是 portfolio-mgr.rhai 确定性风险分类的
    //   LLM 兜底、其失败不应有任何阻断性代价」—— 该表述与实测不符：① 节点已改为 Rhai
    //   CodeNode，「LLM 兜底」不再成立；② 其输出经 portfolio-mgr.rhai 的 f4_signal 承担
    //   f4 因子的**全部**权重（真库实测 26/27 条非零），删除或失败并非"无代价"。
    //   本节点自身的 continue_on_fail 亦已由 false 改为 true（理由见其节点定义处）。
    edges.push(edge("e-t-risk-v-validate", "t-risk", "v-validate"));

    // ── P1-4 修复: data-quality 确定性评分（CodeNode + Rhai，替代原 LLM Agent）──
    // 原 LLM Agent 需 5-10 秒 + token 消耗，改为 Rhai 确定性脚本 <10ms。
    // 基于 10 个分析师的 confidence + data_gaps 布尔值做聚合评分，
    // 输出 grade(A-F) + score(0-100) 与 Agent 版本格式一致。
    // 下游 quality-gate SwitchNode 和 portfolio-mgr 无需修改。
    {
        let dq_id = "data-quality";
        let dq_code = include_str!("../data-quality.rhai").to_string();
        nodes.push(WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: dq_id.into(),
                title: "数据质量确定性评分".into(),
                description: Some("基于分析师信心+数据缺口做确定性评分，输出 grade/score".into()),
                position: Position { x: 840.0, y: 3300.0 },
                retry: RetryConfig::default(),
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                // v7: 容错降级。input_mapping 引用 10 个分析师输出，其中任一失败
                // （如 a-fundamentals）时缺失路径注入 Null（code_executor 语义），
                // data-quality.rhai 有 present() 守卫按缺失计分，可安全降级运行，
                // 不应因单个分析师失败而阻塞 portfolio-mgr/trader。
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: dq_code,
                output_var: dq_id.into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    // P1 修复(2026-07-23): resolve_var_path("{id}.content.verdict.confidence") 全员 null，
                    // 改为 {id}.content.verdict 接收整份 verdict map，在 data-quality.rhai 内提取 confidence。
                    ("mk_verdict", "a-market-analyst.content.verdict"),
                    ("sent_verdict", "a-sentiment.content.verdict"),
                    ("news_verdict", "a-news.content.verdict"),
                    ("fund_verdict", "a-fundamentals.content.verdict"),
                    ("pol_verdict", "a-policy.content.verdict"),
                    ("hm_verdict", "a-hot-money.content.verdict"),
                    ("lk_verdict", "a-lockup.content.verdict"),
                    ("res_verdict", "a-research.content.verdict"),
                    ("sec_verdict", "a-sector.content.verdict"),
                    // V68 修复(2026-09-10): cat_verdict 映射从 .content 改为 .content.verdict。
                    // 原映射是 P2 修复(2026-07-23)按「OutputMode::Json 扁平 schema + 终值
                    // auto-parse」的现实写的；但 agent_executor 后来加了 V62 通用 VERDICT
                    // 标签重构，a-catalyst 的 content 现为 {"report", "verdict":{...}} 嵌套
                    // JSON **字符串**（resolve_var_path 终值不 auto-parse）。rhai 收到的是
                    // string，extract_conf(type_of != "map") 恒返回 -1.0 → 催化剂分析师被
                    // data-quality 恒判 missing（2026-09-08 002837 实证）。改映射到
                    // .content.verdict 后中途穿透 parse，rhai 收到 map，与其他 9 个分析师一致。
                    // V70(2026-09-10): a-catalyst 已改回 Text 模式，此映射与其他 9 个完全同构。
                    ("cat_verdict", "a-catalyst.content.verdict"),
                    // P1-B3 新增(2026-07-24): 拿 10 个分析师的报告正文，算法化 report_quality_score。
                    // AgentExecutor OutputMode::Text 把 LLM 输出包装为 {report, verdict} JSON，
                    // 因此 .content.report 直接是字符串正文（含自然语言分析，不含 VERDICT 标签）。
                    ("mk_report", "a-market-analyst.content.report"),
                    ("sent_report", "a-sentiment.content.report"),
                    ("news_report", "a-news.content.report"),
                    ("fund_report", "a-fundamentals.content.report"),
                    ("pol_report", "a-policy.content.report"),
                    ("hm_report", "a-hot-money.content.report"),
                    ("lk_report", "a-lockup.content.report"),
                    ("res_report", "a-research.content.report"),
                    ("sec_report", "a-sector.content.report"),
                    ("cat_report", "a-catalyst.content.report"),
                    // ── V67 修复(2026-07-29): 映射分析师 __untrusted 标记 ──
                    // agent_executor 在 strict_mode 降级时于 NodeOutput 顶层注入 __untrusted=true。
                    // data-quality.rhai 需读取此标记,将不可信分析师排除出 good_count,
                    // 避免中性兜底 confidence=50 被当成有效信号推高 tool_credibility_score。
                    ("mk_untrusted", "a-market-analyst.__untrusted"),
                    ("sent_untrusted", "a-sentiment.__untrusted"),
                    ("news_untrusted", "a-news.__untrusted"),
                    ("fund_untrusted", "a-fundamentals.__untrusted"),
                    ("pol_untrusted", "a-policy.__untrusted"),
                    ("hm_untrusted", "a-hot-money.__untrusted"),
                    ("lk_untrusted", "a-lockup.__untrusted"),
                    ("res_untrusted", "a-research.__untrusted"),
                    ("sec_untrusted", "a-sector.__untrusted"),
                    ("cat_untrusted", "a-catalyst.__untrusted"),
                    // ── P2-1(2026-09-21): 每个分析师的**真实工具调用记录** ──
                    // 供 `data-quality.rhai::attribution_note()` 交叉核对「报告所说的
                    // 工具被拒绝」是否成立。实证（`AUDIT-pledge-attribution-2026-09-21.md`）：
                    //   a-lockup 报告写「质押数据获取失败（工具调用被拒绝）」，而本轮
                    //   **没有任何质押工具调用**，唯一 `is_error` 的是
                    //   `get_stock_margin_data` 且原因是**限流**（并行节点撞 200ms 间隔）
                    //   ⇒ 使用者被引向「权限问题」，真因完全在别处。
                    //
                    // 路径形态 `{node_id}.tool_calls_made` —— AgentNode 输出的顶层字段，
                    // 与 `seed_serenity.rs` 的 `a-candidate-mapper.tool_calls_made` 同源。
                    // ⚠ 解析不到时注入 Null（`code_executor` 语义）⇒ rhai 侧
                    //   `attribution_note` 的守卫①按「无可核对数据」放行，**不**误判编造。
                    //   （这也是为什么该守卫不可删：删了会把「测不出」判成「有问题」。）
                    ("mk_tool_calls", "a-market-analyst.tool_calls_made"),
                    ("sent_tool_calls", "a-sentiment.tool_calls_made"),
                    ("news_tool_calls", "a-news.tool_calls_made"),
                    ("fund_tool_calls", "a-fundamentals.tool_calls_made"),
                    ("pol_tool_calls", "a-policy.tool_calls_made"),
                    ("hm_tool_calls", "a-hot-money.tool_calls_made"),
                    ("lk_tool_calls", "a-lockup.tool_calls_made"),
                    ("res_tool_calls", "a-research.tool_calls_made"),
                    ("sec_tool_calls", "a-sector.tool_calls_made"),
                    ("cat_tool_calls", "a-catalyst.tool_calls_made"),
                    // ── 因子数据完整度评估（供 pm_compute_factor_completeness 使用）──
                    // 2026-09-09 包装对齐修复：ToolNode 输出结构为
                    //   {node_id, result: {content: <json_string>, tool_name}}
                    // 数据在 result.content 字符串里，路径必须穿透 .content；
                    // resolve_var_path 终值不再 auto-parse，rhai 收到 JSON 字符串
                    // 后按原契约 json_parse/safe_parse 解析。
                    ("total_score", "t-scoring.result.content.totalScore"),
                    ("consensus_score", "debate-convergence.content.consensus_score"),
                    // a-catalyst 是 AgentNode，content parse 后为 {report, verdict}，
                    // catalyst_level 在 verdict 层（实测值 "L1普通消息"）
                    ("catalyst_level", "a-catalyst.content.verdict.catalyst_level"),
                    (
                        "risk_volatility",
                        "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct",
                    ),
                    ("valuation_dcf_upside", "t-valuation.result.content.dcf.upsidePct"),
                    // 2026-09-21: DCF 锚定**缺数**标记 ← `dcf.assumptions.fcf_data_missing`。
                    //   与 `is_fallback_anchor` **正交**：后者两态（现金流真为负 / 数据缺失）
                    //   都为 true，本字段**只在缺数态**为 true ⇒ 用于区分
                    //   「我方取数失败」与「标的现金流真为负」。
                    //   消费点：本节点 Rhai 侧的 `upstream_data_gaps`（**只告警、不扣分**，
                    //   不进 `pm_compute_factor_completeness` 的分母）。
                    //   ⚠️ 不要改成按 `dcf.assumptions.basis` 文案判分支 —— 文案是给人看的
                    //   诊断文本，一改判据就静默失效（P0-I 当初正是为此改用布尔量）。
                    (
                        "valuation_dcf_fcf_data_missing",
                        "t-valuation.result.content.dcf.assumptions.fcf_data_missing",
                    ),
                    // 2026-09-12: 原 `trader_direction` → "trader.content.verdict.verdict"
                    // 映射已删除（死映射）。data-quality 是 trader 的**上游**（trader 的
                    // dqi_score 来自本节点），而 trader → data-quality 的边因循环依赖被移除
                    // （见本函数末尾注释）⇒ 该路径永远解析不到值 ⇒ f7 因子恒缺失、
                    // factor_completeness 上限 0.9、UI 恒显示「缺失因子：交易方向」假告警。
                    // 该因子已从 data-quality 的评估集移除（分母 10 → 9）；
                    // f7 的可用性由 portfolio-mgr（trader 下游）消费。
                    ("money_flow", "t-hotmoney-data.result.content"),
                    ("lockup_bundle", "t-lockup-data.result.content"),
                    ("announcements", "t-catalyst-data.result.content"),
                    ("pace_signal", "pace-calc.result.pace_signal"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }));
        edges.push(edge("e-v-validate-data-quality", "v-validate", dq_id));
        // P1 修复(2026-07-24): data-quality 需要读到10个分析师的 verdict 输出，
        // 但它的依赖链仅含 v-validate → cls-risk-level，这两者都不依赖分析师，
        // 导致 data-quality 在分析师之前就跑完了。
        // 添加从每个分析师的边确保 data-quality 等待所有分析师完成。
        for aid in &a_ids {
            edges.push(edge(&format!("e-{aid}-data-quality"), aid, dq_id));
        }
        // 因子数据完整度评估：data-quality 需要等待 ToolNode 完成以获取因子数据
        edges.push(edge("e-t-scoring-data-quality", "t-scoring", dq_id));
        edges.push(edge("e-t-risk-data-quality", "t-risk", dq_id));
        edges.push(edge("e-t-valuation-data-quality", "t-valuation", dq_id));
        edges.push(edge("e-t-hotmoney-data-quality", "t-hotmoney-data", dq_id));
        edges.push(edge("e-t-lockup-data-quality", "t-lockup-data", dq_id));
        edges.push(edge("e-t-catalyst-data-quality", "t-catalyst-data", dq_id));
        edges.push(edge("e-pace-calc-data-quality", "pace-calc", dq_id));
        edges.push(edge("e-debate-convergence-data-quality", "debate-convergence", dq_id));
        // 修复循环依赖: 移除 trader → data-quality 边
        // 原循环: data-quality → trader → data-quality 导致 CycleDetected 错误
        // 2026-09-12: 该边缺失的后果已一并处理 —— 边不存在意味着 data-quality 永远读不到
        // trader 输出，因此 f7「交易方向」已从 data-quality 的因子集移除（分母 10 → 9），
        // 不再保留会静默失效的 input_mapping。f7 由 portfolio-mgr（trader 下游）消费：
        // portfolio-mgr 的 trader_direction 映射与 e-trader-portfolio-mgr 边均正常。
    }

    // research-mgr → trader → portfolio-mgr
    let mut rm = agent(
        "research-mgr",
        "综合风险评估：总体风险评级与主要风险点清单",
        "research-manager",
        None,
        240.0,
        3600.0,
    );
    if let WorkflowNode::Agent(ref mut a) = rm {
        // v10: 上游辩论/收敛节点（LLM）失败时仍派发——context_sources 缺失静默跳过，
        // input_mapping 的 consensus_score 缺失降级为空。cof=false 时 debate-convergence
        // 等 LLM 节点失败会让 research-mgr 卡 Pending，进而锁死 trader/portfolio-mgr。
        a.base.continue_on_fail = true;
        a.config.context_sources = vec![
            "value-investor".into(),
            "t-scoring".into(),
            "t-valuation".into(),
            "t-risk".into(),
            // V29 修复: 改为引用三档风险评估的原始 AgentNode，而非聚合后的数组
            // AggregatorNode strategy="all" 的 result 是数组，无法用对象字段路径导航，
            // 因此 research-mgr 直接消费三个原始风险辩手的输出。
            // V67 修复: 移除 "risk-aggregated"——V29 注释明确说不引用聚合数组，
            // 但该字段遗留未删，导致 research-mgr 报 "context_sources 变量未找到" ERROR。
            "risk-agg".into(),
            "risk-con".into(),
            "risk-neu".into(),
            "risk-level".into(),
            // V29 修复: input_mapping 引用 debate-convergence，需在 context_sources 中声明
            "debate-convergence".into(),
        ];
        // ── 结构化参数注入（结构化参数方案 Phase 2）──
        // 注入风险的结构化评分，使 research-mgr 可在 system_prompt 中
        // 直接使用 risk_level 等值，无需从文本中重新提取。
        //
        // 路径规则（V29 修复）：
        // - LlmClassifierNode: {category, model, ...} → 直接 .category
        //   ⚠️ v55(2026-09-20): 本模板已无 LlmClassifierNode（唯一实例 cls-risk-level 已下沉
        //     Rhai CodeNode）⇒ 该节点现须按 CodeNode 规则穿透 `.result`。本行规则对其他
        //     模板中的 LlmClassifierNode 仍然适用，故保留。
        // - AgentNode: {role, content: <json_string>, ...} → .content.field
        //   （resolve_var_path 遇到 Value::String 会自动 from_str 解析后再下钻）
        // - AggregatorNode strategy="all": result 是数组，不支持对象字段路径导航，
        //   改为直接引用原始 AgentNode 的 .content.position_pct
        a.config.input_mapping = [
            // P1 修复(3.2 信息隔离): 提案阶段禁止暴露风险预算参数
            // 原 overall_risk / agg_risk_pos / cons_risk_pos / neut_risk_pos 已移除
            // AgentNode(Json mode) 输出包裹在 {role, content: <json_string>, ...} 中
            ("consensus_score", "debate-convergence.content.consensus_score"),
            ("stock_lessons", "stock_lessons"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        // P0 修复(2026-07-22): research-mgr 改为纯决策节点，移除全部 14 个计算工具。
        // 原问题与 trader 同款：
        //   1) td_score/td_val/td_risk 与上游 t-scoring/t-valuation/t-risk 重复
        //   2) td_maxdd/td_sharpe/td_var/td_kelly 等需要 stock_code 或 kline_json 参数，
        //      但 input_mapping 未注入 stock_code，LLM 会传空值触发无效重试
        // 正确架构：综合评估基于 context_sources 注入的上游数据（t-scoring/t-valuation/
        // t-risk/risk-agg/risk-con/risk-neu/debate-convergence），无需重新计算。
        a.config.tools = vec![];
        a.config.system_prompt = format!(
            "{}\n\n--- 数据约束 ---\n\
             你是综合风险评估官，所有需要的数据已通过输入上下文注入，禁止调用任何工具重新获取或计算。\n\
             - 技术评分/指标: 来自 t-scoring（currentPrice/indicators/totalScore）\n\
             - 估值数据: 来自 t-valuation\n\
             - 风险评分: 来自 t-risk\n\
             - 三档风险评估: 来自 risk-agg/risk-con/risk-neu\n\
             - 辩论共识: 来自 debate-convergence（consensus_score）\n\
             基于上述数据综合评估总体风险评级与主要风险点清单。",
            a.config.system_prompt
        );
        a.config.max_tool_rounds = Some(0);
        a.config.exposed_tools = vec![];
    }
    nodes.push(rm);
    edges.push(edge("e-value-investor-research-mgr", "value-investor", "research-mgr"));
    edges.push(edge("e-v-validate-research-mgr", "v-validate", "research-mgr"));

    // trader: 执行方案 — 实时行情 + 技术指标 + 凯利仓位
    let mut trader = agent(
        "trader",
        "制定A股交易方案：入场价、目标价、止损价、仓位比例。遵守T+1和涨跌停规则",
        "trader",
        None,
        240.0,
        3900.0,
    );
    if let WorkflowNode::Agent(ref mut a) = trader {
        // v10: 上游 research-mgr/data-quality 失败时仍派发——research-mgr 仅在
        // context_sources（缺失静默跳过），input_mapping 不引用它；dqi_score 缺失
        // 时 prompt 槽位降级为空，LLM 按保守方向决策。下游 portfolio-mgr.rhai /
        // portfolio-risk-gate.rhai 对 trader_* 缺失均有 present() 守卫 + 波动率 fallback。
        a.base.continue_on_fail = true;
        // P2 修复: 扩展 context_sources 覆盖所有 input_mapping 引用的上游节点
        // （显式依赖原则：input_mapping 引用的上游节点必须有关联边或 context_sources）
        // t-scoring: factor_weights 因子权重 | risk-convergence: risk_disagreement 风险分歧度
        // data-quality: dqi_score 数据质量
        a.config.context_sources = vec![
            "research-mgr".into(),
            "debate-convergence".into(),
            "t-scoring".into(),
            "risk-convergence".into(),
            "data-quality".into(),
        ];
        a.config.output_mode = OutputMode::Json;
        // P0 根因修复(2026-07-22): trader 改为纯决策节点，移除所有数据获取工具。
        // 原设计问题：
        //   1) td_quote/td_kline/td_mf 需要 stock_code，但 Agent 节点工具参数由 LLM
        //      自主生成，LLM 不知道 stock_code（input_mapping 只注入 system_prompt 文本），
        //      导致空 stock_code → 6 vendor × 2 轮无效重试（浪费 3.4 分钟）。
        //   2) compute_atr/kelly/mc 等需要 kline_json 参数，LLM 无法可靠地从
        //      system_prompt 复制 120 根 K 线 JSON 到工具参数。
        // 正确架构：数据由上游 t-scoring 获取并注入，trader 基于注入数据做决策。
        // ATR/Kelly/MC 等复杂计算应由独立 Code 节点完成（后续优化）。
        a.config.tools = vec![]; // 纯决策节点，无需工具
        a.config.system_prompt = format!(
            "{}\n\n--- 数据约束 ---\n\
             你是交易方案制定者，所有需要的数据已通过输入上下文注入，禁止调用任何工具重新获取数据。\n\
             - 当前价: 参考【reference_price】\n\
             - 技术指标: 参考【technical_indicators】(含 ma5/ma20/macd_dif/rsi14/boll_upper/boll_lower 等)\n\
             - 综合评分: 参考【total_score】\n\
             - 共识评分: 参考【consensus_score】\n\
             - 风险分歧: 参考【risk_disagreement】(>50 时保守)\n\
             - 数据质量: 参考【dqi_score】(<50 时保守)\n\
             基于上述数据直接制定交易方案，输出入场价、目标价、止损价、仓位比例。\n\
             \n--- 价位字段方向语义（必须遵守）---\n\
             - targetPrice 是**方向性目标**：看多(买入/增持)取 > reference_price 的上涨目标；\n\
               看空(减持/卖出)取 < reference_price 的下跌目标。\n\
             - stopLoss 与 targetPrice 必须分居 reference_price 两侧，禁止 targetPrice <= stopLoss。\n\
             - **持有/观望：不要输出 targetPrice**。若确实要输出，禁止填成等于 reference_price 的数值\n\
               —— 等于现价不含任何方向信息，会被下游判为「无效价格信号」丢弃，\n\
               还会让仪表盘「目标价」显示成与估值结论（内在价值/理想买入价）看似矛盾的数值。\n\
             - 目标价偏离现价超过 70% 会被判为异常数据，请保持量级合理。",
            a.config.system_prompt
        );
        a.config.max_tool_rounds = Some(0); // 禁用工具调用轮次
        a.config.input_mapping = [
            ("consensus_score", "debate-convergence.content.consensus_score"),
            ("stock_lessons", "stock_lessons"),
            // P1 修复: 注入标准参考价，确保 trader 与 portfolio-mgr 使用相同的 currentPrice
            // 避免 trader 自行调用 get_stock_quote 获取的实时价与 t-scoring 缓存的 currentPrice
            // 不一致导致的系统性分歧。
            // 2026-09-09: ToolNode result 为 {content: <json_string>, tool_name}，穿透 .content
            ("reference_price", "t-scoring.result.content.currentPrice"),
            // P2 修复: 注入因子权重，使 trader 知道哪些因子在公式中权重更高
            // factor_weights 是 JSON 对象 {trend:{weight}, macd:{weight}, ...}
            ("factor_weights", "t-scoring.result.content.factor_backtest.factors"),
            // P2 修复: 注入风险分歧度，使 trader 知道三位风险评估师的分歧程度
            // 分歧高(>50)时 trader 应避免过度自信
            // 2026-09-09: risk-convergence content parse 后为 {report, verdict}，分歧度在 verdict 层
            ("risk_disagreement", "risk-convergence.content.verdict.disagreement_score"),
            // P2 修复: 注入数据质量评分，使 trader 知道当前数据覆盖度
            // dqi_score 0-100，低分时 trader 应保守操作
            // V58 修复: data-quality 是 CodeNode，score 在 .result 里
            ("dqi_score", "data-quality.result.score"),
            // P0 修复(2026-07-22): 注入 t-scoring 完整技术指标，替代 get_stock_quote/kline。
            // 包含 ma5/ma20/bias_ma5/macd_dif/macd_dea/rsi14/boll_upper/boll_lower 等，
            // trader 可直接读取指标制定交易方案，无需重新调用行情工具。
            ("technical_indicators", "t-scoring.result.content.indicators"),
            ("total_score", "t-scoring.result.content.totalScore"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    }
    nodes.push(trader);
    edges.push(edge("e-research-mgr-trader", "research-mgr", "trader"));
    // P2 修复: 为 trader 新增的 input_mapping 入口加显式边
    // t-scoring → trader: 因子权重和参考价
    edges.push(edge("e-t-scoring-trader-p2", "t-scoring", "trader"));
    // risk-convergence → trader: 风险分歧度
    edges.push(edge("e-risk-convergence-trader-p2", "risk-convergence", "trader"));
    // data-quality → trader: 数据质量评分
    edges.push(edge("e-data-quality-trader-p2", "data-quality", "trader"));

    // portfolio-mgr: 最终决策 — 确定性计算（CodeNode + Rhai）
    // ── 结构化参数方案 Phase 3 ──
    // 原为 Agent 节点（LLM 执行公式），现改为 CodeNode（Rhai 确定性执行）。
    //
    // 公式逻辑（与 `portfolio-mgr.rhai` 中的实现保持一致）：
    //   confidence = clamp(totalScore + adjustment, 0, 100)
    //   adjustment = 共识调整 + 数据质量调整 + 风险调整 + 催化剂加成 + 机构加成
    let pm_code = include_str!("../portfolio-mgr.rhai").to_string();
    let pm = WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            id: "portfolio-mgr".into(),
            title: "投资组合经理（确定性决策）".into(),
            description: Some("基于结构化参数，用确定性公式计算最终决策".into()),
            position: Position { x: 240.0, y: 4200.0 },
            retry: RetryConfig::default(),
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: true,
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: pm_code,
            output_var: "portfolio-mgr".into(),
            tool_name: None,
            execute_directly: true,
            input_mapping: {
                // 静态映射：上游节点输出路径 → 变量名
                let mut m: Vec<(&str, &str)> = vec![
                    // 2026-09-09 包装对齐修复：ToolNode 输出 result 为
                    // {content: <json_string>, tool_name}，数据穿透 .content 取；
                    // AgentNode（trader/a-catalyst）content parse 后为 {report, verdict}，
                    // 结构化字段在 verdict 层。
                    ("totalScore", "t-scoring.result.content.totalScore"),
                    // 阶段2（PROPOSAL-stock-decision-four-horizon.md）：三档周期独立评分。
                    // 供 portfolio-mgr.rhai 的 decisionsByHorizon 做 f1 周期重融合；未注入（上游
                    //   缺陷/数据不足）时 input_mapping 自动补 unit ⇒ rhai 端退化为日线档。
                    //   短=周线 / 中=月线 / 长=月线（长档复用月线，阶段2拍板不新增季度节点）。
                    ("totalScore_short", "t-scoring-week.result.content.totalScore"),
                    ("totalScore_mid", "t-scoring-month.result.content.totalScore"),
                    ("totalScore_long", "t-scoring-month.result.content.totalScore"),
                    // AgentNode 输出包裹在 {role, content: <json_string>, ...} 中
                    // V29 修复: data-quality 是 AgentNode，无 .result 字段，必须走 .content.
                    // V58 修复: data-quality 实为 CodeNode（Rhai），输出结构为
                    //   {status, result: {grade, score, ...}, input_params, node_id, params}
                    //   score 字段在 .result 里，旧路径 "data-quality.score" 无法穿透
                    //   CodeNode 包装，导致 dqi_score 缺失 → f6_weight=0 → total_weight
                    //   下降触发 weights_collapsed 误坍缩。
                    ("dqi_score", "data-quality.result.score"),
                    // 2026-09-14: 质量分档的唯一权威是 data-quality.rhai 输出的 grade 字符串。
                    //   此前 portfolio-mgr 直接用 dqi_score 数值 + 自己的两套字母阈值
                    //   （confidence_quality_cap 用 90/75/60，dqi_a_level/high_quality 用 85/65），
                    //   与 data-quality.rhai 的 85/65/45/25 各说各话 —— 同一个 score 被四处
                    //   判成不同等级，且 grade="C" 的股票在置信度上限那套里其实按 D 级处理。
                    //   现改为消费 grade，阈值全项目只保留 data-quality.rhai 这一份。
                    //   dqi_score 保留传递：Rhai 公式仍需其连续值算 f6_signal 与提示文本。
                    ("dqi_grade", "data-quality.result.grade"),
                    // P1/P2: 因子回测数据（compute_scoring 工具附加输出）
                    ("factor_weights", "t-scoring.result.content.factor_backtest.factors"),
                    // P1-1: 市场状态权重调节（regime-weights.rhai）替代纯回测权重
                    // 牛市→趋势↑, 熊市→估值/风险↑, 高波动→全降权
                    // V58 修复: regime-weights 是 CodeNode，factor_weights 在 .result 里
                    ("regime_factor_weights", "regime-weights.result.factor_weights"),
                    // market_regime 是 core.rs 注入的工作流变量（非 t-scoring 节点输出）
                    ("market_regime_prior", "market_regime.confidence"),
                    ("market_regime_state", "market_regime.regime"),
                    // P1-7 修复: 风险分类器（v55 起为 CodeNode/Rhai）作为算法分类的 fallback
                    // Rhai 脚本先基于 t-risk stockRiskProfile 做确定性算法分类，
                    // 仅当数据缺失时回退到此分类结果。
                    // 注意：该节点的 id 是 "cls-risk-level"（output_var 才是 "risk-level"），
                    // 边与 input_mapping 必须以节点 id 为准，否则 context.variables 查不到。
                    // v55(2026-09-20): 该节点已由 LlmClassifierNode 改为 CodeNode ⇒ 其输出被包在
                    // `result` 里，路径**必须穿透 .result**（同 dqi_grade 的
                    // "data-quality.result.grade" 形态）。源字段名仍是 `category` 未变，
                    // 故 portfolio-mgr.rhai 读到的仍是「低风险/中风险/高风险/极高风险」四选一字符串，
                    // 消费端零改（该脚本 :484 的 f4_signal 查表与 :455 的 fallback 均按字符串匹配）。
                    ("overall_risk_llm", "cls-risk-level.result.category"),
                    // AgentNode(Json mode) 输出包裹在 {role, content: <json_string>, ...} 中
                    // 2026-09-09: content parse 后为 {report, verdict}，catalyst_level 在 verdict 层
                    ("catalyst_level", "a-catalyst.content.verdict.catalyst_level"),
                    ("consensusScore", "debate-convergence.content.consensus_score"),
                    // V65: trader 输出完整 6 维度字段（与 portfolio-mgr 同维度对齐用于双视角对比）
                    // 旧字段保留: trader_direction/trader_target_price/trader_stop_loss 供 f7 兼容路径
                    // 2026-09-09: trader content parse 后为 {report, verdict:{action, confidence, ...}}，
                    // 结构化字段全部在 verdict 层
                    // 2026-09-19: 删除 ("trader_action", "trader.content.verdict.action")。
                    //   依据：`scripts/audit-inject-coverage.mjs` ② 段零命中 ⇒ 本脚本全文不读它
                    //   （且无 scope.get/iter 一类动态访问）⇒ 白注入。动作语义由下一行的
                    //   `trader_direction` 承载（脚本内读 8 处）。
                    //   ⚠ 勿误删 reflection-comparator 的同名映射：那是**另一个节点**，位于
                    //     `mod.rs`，认字符串 `("trader_action", "sub-analysis.trader.content.action")`；
                    //     其脚本 `reflection-comparator.rhai` 在 `is_unit(trader_action)` 处真读
                    //     ⇒ 独立映射、本版不动。
                    //     （此处不写行号：本仓任何一次插入都会让行号失效，已实测腐烂两轮。）
                    ("trader_direction", "trader.content.verdict.verdict"),
                    ("trader_confidence", "trader.content.verdict.confidence"),
                    // currentPrice: 从 t-scoring 工具节点（get_stock_quote）获取，可靠数据源。
                    // 不用 trader.content.currentPrice，因为 LLM 不一定输出该字段。
                    ("current_price", "t-scoring.result.content.currentPrice"),
                    ("trader_target_price", "trader.content.verdict.targetPrice"),
                    ("trader_stop_loss", "trader.content.verdict.stopLoss"),
                    ("trader_time_horizon", "trader.content.verdict.timeHorizon"),
                    ("trader_holding_days", "trader.content.verdict.expectedHoldingDays"),
                    // V65 新增: trader 对比字段
                    // 2026-09-19 订正: 原注释「trader 6 维度对比字段（f7 可消费更丰富的 LLM 信号）」
                    //   与代码事实不符 —— 实测 f7 只消费**下面保留的 2 个**
                    //   （trader_risk_level 读 3 处、trader_evidence_count 读 4 处）；
                    //   其余 4 个本脚本全文零引用，已删（白注入，`audit-inject-coverage.mjs` ② 段）。
                    //   删除项: trader_position_pct / trader_stop_loss_pct /
                    //           trader_take_profit_pct / trader_data_gaps。
                    //   注：止损/目标价语义由既有 tracker_stop_loss / trader_target_price 承载（分别读 21/34 处）。
                    ("trader_risk_level", "trader.content.verdict.riskLevel"),
                    ("trader_evidence_count", "trader.content.verdict.evidence_cited"),
                    // V50 修复: 接入 risk-convergence 的三方风险分歧度
                    // 避免该 LLM 节点（约5-10s）的输出被浪费
                    ("risk_disagreement", "risk-convergence.content.verdict.disagreement_score"),
                    // V51 新增: 估值因子数据源
                    // t-valuation 输出 DCF/格雷厄姆上行空间，用于 f5_signal 估值因子
                    // 2026-09-09: 穿透 .content（工具端已补 dcf/graham/fScore camelCase 别名块）
                    // P0-I(2026-09-12): 锚定来源标记 —— `true` 表示 FCF 锚定来自
                    // 「当期FCF≤0 ⇒ 近5年报正净利均值×0.90」的历史代理，而非当期真实 FCF。
                    // 消费点：portfolio-mgr.rhai 的 f5 置信度衰减（σ × 0.5）。
                    // 注意：新变量只需在 rhai 里用 `present(...)` 包裹即可安全缺省，
                    //   无需在本映射里补占位（rt-workflow V57 会自动补 unit 默认值）。
                    (
                        "valuation_dcf_anchor_is_fallback",
                        "t-valuation.result.content.dcf.assumptions.is_fallback_anchor",
                    ),
                    // v50(2026-09-14): DCF **模型适用性**门 —— 前提假设对本标是否成立。
                    // `dcf.assumptions.applicable == false` 表示「本标的不满足 DCF 前提」，
                    // 判据由 `compute_dcf` 按**数据形态**给出（杠杆畸高 / FCF 与净利背离 /
                    // 负增长且终值独裁），**不按行业标签** —— 银行、保险、券商、地产、
                    // 重资产周期自动全部覆盖，无需白名单。
                    // 消费点：portfolio-mgr.rhai 的 f5 —— 命中即把 DCF 这一腿**整体剔除**
                    // （衰减 ≠ 剔除：衰减等于承认它还有部分信息量，而这里的语义是「该数值
                    // 没有经济含义」）。601166 实证：给 40.17–49.26 元 vs 现价 18.15，
                    // 而 LLM 层给「观望 + 0%」——同一份输出互相打脸。
                    (
                        "valuation_dcf_applicable",
                        "t-valuation.result.content.dcf.assumptions.applicable",
                    ),
                    // 不适用原因（多条以 `；` 连接），用于决策留痕与诊断面板逐条展示。
                    (
                        "valuation_dcf_inapplicable_reason",
                        "t-valuation.result.content.dcf.assumptions.inapplicable_reason",
                    ),
                    ("valuation_dcf_upside", "t-valuation.result.content.dcf.upsidePct"),
                    ("valuation_graham_upside", "t-valuation.result.content.graham.upsidePct"),
                    // 2026-09-21: 格雷厄姆腿的**增长率封顶**标记 —— `true` 表示
                    //   `revenue_yoy` 超过 `MAX_GROWTH(30%)` 被封顶，即代入公式的增长率
                    //   低于实际增速 ⇒ 内在价值系统性偏低 ⇒ `upsidePct` 偏负（看空被夸大）。
                    //   消费点：portfolio-mgr.rhai 的 f5，对 graham 腿**软衰减**（σ × 0.5）。
                    //   ⚠️ 与 `applicable=false` 的区别：那是「数值没有经济含义」⇒ 整腿剔除；
                    //   这里只是「假设被压缩」⇒ 仍有信息量，降信即可。两者不可混用。
                    //   300308 实证（2026-09-22 量纲修复**后**）：EPS 18.47 × (8.5 + 2×30)
                    //   = 1265.20，现价 926.43 ⇒ `upsidePct = +36.6`。
                    //   （修复前误按小数算作 `EPS × 9.1 = 168.08` ⇒ `upsidePct = −81.9`；
                    //    该样本是量纲修复仅有的 2 个符号翻转样本之一。）
                    //   该值可用纯 PE 恒等式复现（两边 EPS 相消）⇒ 近乎不含公司特定信息；
                    //   封顶方向性结论不变 —— 实际增速被 MAX_GROWTH 压低 ⇒ 价值仍系统性偏低。
                    //   缺该字段（旧模板）时 Rhai 侧按「未封顶」处理。
                    (
                        "valuation_graham_growth_clamped",
                        "t-valuation.result.content.graham.assumptions.growthClampedUpper",
                    ),
                    ("valuation_fscore", "t-valuation.result.content.fScore.score"),
                    ("valuation_moat", "t-valuation.result.content.moat.label"),
                    // ── V79(2026-09-21) 第三锚腿：PE 历史分位 ──
                    // ← `t-valuation-band`（ToolNode，调 `compute_valuation_band`）的
                    //   `metricPe.currentPercentile`（0 = 近 5 年最便宜 / 100 = 最贵）。
                    //
                    // 为什么需要第三条腿（审计 §6.10.2）：
                    //   · DCF 腿在 `applicable=false`（杠杆畸高 / FCF 与净利背离 / 负增长独裁）
                    //     时被**整腿剔除**，而命中的恰是银行/保险/券商/地产/重资产周期
                    //     ⇒ 估值维度只剩 graham 一条腿，而 graham 是**纯 PE 恒等式**
                    //     （EPS 约掉）⇒ 近乎不含公司特定信息；
                    //   · graham 腿自身在高增长标的上又因 `revenue_yoy` 被封顶到 30%
                    //     而系统性偏低。
                    //   PE 分位**既不依赖 FCF、也不依赖增长率假设** ⇒ 对上述两类标的都有效，
                    //   且与 graham 的信息**不重复**（graham = 相对 8.5+2g/债券收益率的绝对偏离；
                    //   本腿 = 相对**自身 5 年历史**的位置）。
                    //
                    // 取值域：`currentPercentile: Option<f64>`；「PE ≤ 0（亏损）」与
                    //   「有效样本 < 20」两种情形为 null ⇒ rhai 侧按 `present()` 判该腿不可用，
                    //   **按可用腿归一化** —— band 缺失时前两腿份额逐位等于 0.7/0.3（见 rhai）。
                    // ⚠️ 本节点也会**顺带回填** `financial_snapshots`（5 年日频，~1200 行）。
                    //   该表此前只在用户点开前端「估值带」图时才回填，覆盖仅 13 只 ⇒
                    //   若不接本节点，绝大多数标的的 `currentPercentile` 恒为 null、
                    //   第三腿恒不可用（新腿会退化成永不生效的空壳）。
                    (
                        "valuation_pe_percentile",
                        "t-valuation-band.result.content.metricPe.currentPercentile",
                    ),
                    // 2026-09-23: 同时注入 `verdict` —— 让 rhai 侧声明的「有效性门须按
                    //   **数据形态**判」真正可落地。
                    //
                    // 病根（声明与实现不一致）：`portfolio-mgr.rhai` 该腿的注释明写
                    //   「`compute_valuation_band` 的 verdict 也可能因样本不足为
                    //   `insufficient`，故此处只接值落在 [0,100] 的分位」，但实现
                    //   **只查了值域、从未读 verdict** ⇒ 有效样本不足的标的只要
                    //   `currentPercentile` 恰好有值，就以**满强度**进入融合。
                    //   `valuation_band.rs` 对样本 < `MIN_SAMPLES` 的处置本来就是
                    //   `verdict = "insufficient"` ⇒ 该状态是**生产端已给出**的质量声明，
                    //   消费端漏接（与 DCF 腿的 `applicable` / graham 腿的
                    //   `growthClampedUpper` 两条质量声明同族，唯独本腿此前无声明可用）。
                    ("valuation_band_verdict", "t-valuation-band.result.content.verdict"),
                    // V52 新增: t-risk 算法风险分类数据源
                    // 用确定性算法替代 LLM 分类器（消除 LLM 不一致性）
                    // v55(2026-09-20) 订正: cls-risk-level 亦已下沉 Rhai ⇒ 本模板内不再存在
                    //   「LLM 分类器」。注意本算法分类（portfolio-mgr.rhai 的 RISK_* 常量）
                    //   与 cls-risk-level 的 Rhai 口径（照抄原 prompt 阈值）**并非同一套阈值**
                    //   （实测 6 处不同，见 risk-level.rhai 头部）⇒ 二者仍是两个独立口径，
                    //   不是同一判定的重复计算。
                    // t-risk 是 ToolNode, stockRiskProfile 在 result.content 中
                    (
                        "risk_volatility",
                        "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct",
                    ),
                    ("risk_drawdown", "t-risk.result.content.stockRiskProfile.maxDrawdownPct"),
                    ("risk_sharpe", "t-risk.result.content.stockRiskProfile.sharpeRatio"),
                    ("risk_roe", "t-risk.result.content.stockRiskProfile.roeTTMPct"),
                    // 2026-09-19: 删除 ("risk_gross_margin", "...stockRiskProfile.grossMarginPct") ——
                    //   本脚本全文零引用（`audit-inject-coverage.mjs` ② 段零命中），且全仓除本处外
                    //   无第二处引用 ⇒ 纯白注入。同批删除的另 5 条见本节点上文注释。
                    //   保留邻居 risk_sharpe / risk_roe / risk_debt_ratio（脚本内分别读 15/8/2 处，实测）。
                    ("risk_debt_ratio", "t-risk.result.content.stockRiskProfile.debtRatioPct"),
                    // v51(2026-09-14): 删除 ("stock_sector", "stock_sector") —— 该映射在
                    // portfolio-mgr.rhai 中的唯一消费者是 classify_risk 的「金融业行业白名单
                    // 豁免」，白名单已删（判据只按数据形态、不按行业归属）⇒ 映射悬空，一并删。
                    // ⚠️ 工作流变量 stock_sector 本身**保留**：它仍被 portfolio-risk-gate 的
                    //    行业暴露上限检查消费（见本文件 portfolio-risk-gate 节点的映射）。
                    (
                        "risk_revenue_growth",
                        "t-risk.result.content.stockRiskProfile.revenueGrowthYoYPct",
                    ),
                    ("risk_pe", "t-risk.result.content.stockRiskProfile.peTTM"),
                    // V53 修复: 从瓶颈掘金工作流传入的上下文标记
                    // 告诉 portfolio-mgr"当前分析的股票来自 Serenity 筛选",
                    // 允许风险分类器对瓶颈股特征（高波动/扩张期）做评分修正
                    ("screening_source", "screening_source"),
                    // X1 桥接: Serenity 瓶颈分析上下文（serenity_score / bottleneck_product 等）
                    // 由 core.rs 在 screening_source=serenity 时注入为工作流变量
                    ("serenity_context", "serenity_context"),
                    // ── P1 新增: 资金面因子 f9 数据源 ──
                    // t-hotmoney-data 输出 get_stock_money_flow 的 JSON 字符串
                    // Rhai 中用 json_parse() 解析后提取主力净流入占比
                    // 2026-09-09: 数据在 result.content 字符串中，终值保持字符串原样
                    ("money_flow", "t-hotmoney-data.result.content"),
                    // ── V70(2026-09-11): f9 归一化分母 ──
                    // 旧公式用资金净额自身做分母 → 恒 ±0.67 伪二值信号（DB 实证）。
                    // 新公式以「近 5 日平均成交额」为分母，成交额取自 t-scoring 内嵌的
                    // kline_json（日线数组，amount 上游常为 0，故用 volume × close 估算）。
                    ("kline_json", "t-scoring.result.content.kline_json"),
                    // ── P1 新增: 筹码面因子 f10 数据源 ──
                    // t-lockup-data 输出 get_stock_lockup_bundle 的 JSON 字符串
                    // 含解禁/增减持/大宗交易三方信息
                    ("lockup_bundle", "t-lockup-data.result.content"),
                    // ── P2 新增: 龙虎榜数据源（f10 筹码面增强）──
                    // t-dragon-tiger-data 输出 get_stock_dragon_tiger 的 JSON 字符串
                    // 含机构席位买卖、游资动向、上榜原因等
                    ("dragon_tiger", "t-dragon-tiger-data.result.content"),
                    // ── P2 新增: 公告风险信号（f3 催化剂增强）──
                    // t-catalyst-data 输出 get_stock_announcements 的 JSON 字符串
                    // 含公告标题/类型/日期，用于关键词风险检测
                    ("announcements", "t-catalyst-data.result.content"),
                    // ── V55 新增: 上游 strict_mode 兜底哨兵 ──
                    // 每个 AgentNode 在 strict_mode 降级时会在顶层注入 __untrusted=true。
                    // portfolio-mgr.rhai 累加这些哨兵，任意一个为 true 即触发 weights_collapsed
                    // 兜底（强制观望+空仓+confidence 对半），避免 LLM 失败的 50/50 兜底
                    // 被当成有效信号继续融合。
                    ("untrusted_trader", "trader.__untrusted"),
                    ("untrusted_research_mgr", "research-mgr.__untrusted"),
                    ("untrusted_catalyst", "a-catalyst.__untrusted"),
                    ("untrusted_debate_conv", "debate-convergence.__untrusted"),
                    ("untrusted_data_quality", "data-quality.__untrusted"),
                    ("untrusted_risk_conv", "risk-convergence.__untrusted"),
                    // ── PACE 情绪因子 f11: pace-calc CodeNode 输出 pace_signal ──
                    // V58 修复: pace-calc 是 CodeNode，pace_signal/pace_degraded 在 .result 里
                    ("pace_signal", "pace-calc.result.pace_signal"),
                    // P2-2: pace 降级标志（valid_event_count==0 时 pace-calc 设置）
                    ("pace_degraded", "pace-calc.result.pace_degraded"),
                    // ── 技术否决（technical-veto）输入：从 t-scoring 的完整指标获取 ──
                    ("rsi_14", "t-scoring.result.content.indicators.rsi14"),
                    ("macd_dif", "t-scoring.result.content.indicators.macdDif"),
                    ("macd_dea", "t-scoring.result.content.indicators.macdDea"),
                    // ── 市场模拟门（S-501~503）：core.rs 从个股 K 线注入的模拟指标 ──
                    ("sim_stability", "sim_stability"),
                    ("sim_liquidity", "sim_liquidity"),
                    ("sim_impact", "sim_impact"),
                    // ── D7/D8: 可调决策参数（来自 workflow_template.variables，经 settings 页面持久化）──
                    // V71(2026-09-11): 这批同名映射此前是**空映射** —— 映射目标变量从未在
                    //   seed_variables.rs 中定义，导致 context.variables 查不到 → rhai 的
                    //   present() 恒假 → 全部静默走 rhai 内硬编码默认值，「可配置」形同虚设；
                    //   且前端配置面板分组被 .filter(Boolean) 整组过滤（界面空白）、
                    //   反思参数建议被静默丢弃。补齐变量定义后本批映射即真正生效。
                    // V72(2026-09-11): 改为由 PORTFOLIO_MGR_TUNABLE_PARAMS 单一权威源派生，
                    //   不再手写这批同名映射 —— 映射表 / 反思清单 / 前端分组共用一份定义，
                    //   杜绝「加了变量却漏加映射」这类静默漂移（见下方 m.extend）。
                    //   注：input_mapping 是 HashMap（无序），派生只保证**集合**一致；
                    //   决策相关性顺序仅在反思清单的线性渲染中生效。
                ];
                m.extend(PORTFOLIO_MGR_TUNABLE_PARAMS.iter().map(|n| (*n, *n)));
                m.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
            },
        },
    });
    nodes.push(pm);
    edges.push(edge("e-trader-portfolio-mgr", "trader", "portfolio-mgr"));
    edges.push(edge("e-research-mgr-portfolio-mgr", "research-mgr", "portfolio-mgr"));

    // ── P1-E13: portfolio-risk-gate 组合风控门（CodeNode + Rhai）──
    // 在 portfolio-mgr 之后、rule-check 之前运行。
    // 职责：单股仓位上限、行业暴露、持仓数量、风险档位否决、空头强制卖出、组合归一化
    // 输出保留 portfolio-mgr 的所有字段，仅覆盖 action/positionPct/riskLevel，追加 risk_gate 元数据
    let prg_code = include_str!("../portfolio-risk-gate.rhai").to_string();
    let prg = WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            id: "portfolio-risk-gate".into(),
            title: "组合风控门".into(),
            description: Some(
                "组合层风控：仓位上限/行业暴露/风险档位否决/空头强制卖出/组合归一化".into(),
            ),
            position: Position { x: 480.0, y: 4200.0 },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: true,
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: prg_code,
            output_var: "portfolio-risk-gate".into(),
            tool_name: None,
            execute_directly: true,
            input_mapping: [
                // portfolio-mgr 的完整 result 对象（保留所有字段，覆盖调整字段）
                ("pm_result", "portfolio-mgr.result"),
                // 当前价（用于计算新增仓位市值 + 空头检测）
                // 2026-09-09: ToolNode result 为 {content: <json_string>, tool_name}，穿透 .content
                ("current_price", "t-scoring.result.content.currentPrice"),
                // 目标价（用于空头检测：target < current × 0.85 → 强制卖出）
                // trader content parse 后为 {report, verdict}，targetPrice 在 verdict 层
                ("target_price", "trader.content.verdict.targetPrice"),
                // 工作流变量（core.rs 注入）
                ("stock_code", "stock_code"),
                ("stock_sector", "stock_sector"),
                ("holdings_json", "holdings_json"),
                ("portfolio_cash", "portfolio_cash"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        },
    });
    nodes.push(prg);
    // portfolio-mgr → portfolio-risk-gate（主依赖）
    edges.push(edge("e-portfolio-mgr-risk-gate", "portfolio-mgr", "portfolio-risk-gate"));
    // t-scoring → portfolio-risk-gate（current_price 输入）
    edges.push(edge("e-t-scoring-risk-gate", "t-scoring", "portfolio-risk-gate"));
    // trader → portfolio-risk-gate（target_price 输入）
    edges.push(edge("e-trader-risk-gate", "trader", "portfolio-risk-gate"));

    // ── P1-1: regime-weights 市场状态权重调节（CodeNode + Rhai）──
    // 在 portfolio-mgr 之前运行，输出调节后的因子权重。
    // 牛市→趋势+资金面权重↑，熊市→估值+风险权重↑，高波动→所有因子降权
    {
        let rw_code = include_str!("../regime-weights.rhai").to_string();
        nodes.push(WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: "regime-weights".into(),
                title: "市场状态权重调节".into(),
                description: Some("基于市场状态动态调节因子权重".into()),
                position: Position { x: 20.0, y: 4200.0 },
                retry: RetryConfig::default(),
                timeout: Some(5),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: rw_code,
                output_var: "regime-weights".into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    // market_regime 是 core.rs 注入的工作流变量（非 t-scoring 节点输出）
                    ("market_regime_state", "market_regime.regime"),
                    ("market_regime_prior", "market_regime.confidence"),
                    ("market_regime_volatility", "market_regime.volatility"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }));
        edges.push(edge("e-t-scoring-regime-weights", "t-scoring", "regime-weights"));
        edges.push(edge("e-regime-weights-portfolio-mgr", "regime-weights", "portfolio-mgr"));
    }

    // debate-convergence → portfolio-mgr: 显式边确保 consensus_score 在公式执行前就绪
    edges.push(edge("e-debate-convergence-portfolio-mgr", "debate-convergence", "portfolio-mgr"));
    // V29 修复: debate-convergence → research-mgr / trader 显式边
    // research-mgr 和 trader 的 input_mapping 引用 debate-convergence.content.consensus_score，
    // 加显式边确保共识分数在节点执行前就绪（符合显式依赖原则）
    edges.push(edge("e-debate-convergence-research-mgr", "debate-convergence", "research-mgr"));
    edges.push(edge("e-debate-convergence-trader", "debate-convergence", "trader"));
    // P0-3 修复: risk-convergence → research-mgr 显式边
    // research-mgr 的 context_sources 引用 risk-agg/risk-con/risk-neu 的原始输出，
    // 但 DAG 中仅有 debate-convergence → research-mgr 边，不保证风险辩手已完成。
    // 添加 risk-convergence → research-mgr 边，使 research-mgr 等待三个风险辩手
    // 全部完成后再调度，避免 risk-con 超时重试期间 context_sources 变量缺失。
    edges.push(edge("e-risk-convergence-research-mgr", "risk-convergence", "research-mgr"));
    // data-quality → portfolio-mgr: 显式边确保 dqi_score 在 Rhai 公式执行前就绪
    edges.push(edge("e-data-quality-portfolio-mgr", "data-quality", "portfolio-mgr"));
    // V50 修复: risk-convergence → portfolio-mgr 显式边
    // risk-convergence 的三方分歧度(disagreement_score)已被加入 input_mapping，
    // 需要显式边确保在执行 portfolio-mgr 前就绪
    edges.push(edge("e-risk-convergence-portfolio-mgr", "risk-convergence", "portfolio-mgr"));
    // V52: t-risk → portfolio-mgr 显式边
    // portfolio-mgr 需要 t-risk.stockRiskProfile 数据做算法风险分类
    edges.push(edge("e-t-risk-portfolio-mgr", "t-risk", "portfolio-mgr"));
    // ── P1 新增: f9 资金面因子数据源 → portfolio-mgr 显式边 ──
    // t-hotmoney-data 输出资金流向数据，portfolio-mgr 的 input_mapping 引用
    // "money_flow" → "t-hotmoney-data.result"，需要显式边确保调度顺序
    edges.push(edge("e-t-hotmoney-data-portfolio-mgr", "t-hotmoney-data", "portfolio-mgr"));
    // ── P1 新增: f10 筹码面因子数据源 → portfolio-mgr 显式边 ──
    // t-lockup-data 输出解禁/增减持/大宗交易数据，portfolio-mgr 的 input_mapping 引用
    // "lockup_bundle" → "t-lockup-data.result"，需要显式边确保调度顺序
    edges.push(edge("e-t-lockup-data-portfolio-mgr", "t-lockup-data", "portfolio-mgr"));
    // ── P2 新增: 龙虎榜数据源 → portfolio-mgr 显式边 ──
    edges.push(edge("e-t-dragon-tiger-data-portfolio-mgr", "t-dragon-tiger-data", "portfolio-mgr"));
    // ── 2026-07-25 修复: 龙虎榜数据 → a-hot-money 显式边 ──
    // t-dragon-tiger-data 输出 get_stock_dragon_tiger 结果（机构席位/游资动向/上榜原因），
    // a-hot-money 需要此数据做资金面追踪分析。LLM 仍可通过 PROFILE_TOOLS 调 get_stock_dragon_tiger
    // 补充，但有了这条边 + input_mapping 注入后可免 LLM 自行调用。
    edges.push(edge("e-t-dragon-tiger-data-hot-money", "t-dragon-tiger-data", "a-hot-money"));
    // ── P2 新增: 公告数据源 → portfolio-mgr 显式边 ──
    // t-catalyst-data 输出公司公告列表，用于公告关键词风险检测
    edges.push(edge("e-t-catalyst-data-portfolio-mgr", "t-catalyst-data", "portfolio-mgr"));

    // ── 修复 portfolio-mgr 因子输入全空（决策塌成全零空壳）: 补齐缺失的显式边 ──
    // portfolio-mgr 是 CodeNode（无 context_sources），其结构化输入仅来自 edges 直接上游节点。
    // 下方 5 个节点的输出被 portfolio-mgr.input_mapping 引用，但此前缺指向它的边，
    // 导致这些节点不进入 context.variables，因子输入（totalScore / catalyst_level /
    // 估值因子 / pace_signal / LLM 风险兜底）全部取不到 → 后验塌成先验 0.5 →
    // action=观望、positionPct=0、confidence=0 的全零空壳。
    // 与下方 p-risk-assess 补 e-scoring-p-risk-assess 边的修复同源（见行 1421 注释）。
    edges.push(edge("e-t-scoring-portfolio-mgr", "t-scoring", "portfolio-mgr"));
    edges.push(edge("e-a-catalyst-portfolio-mgr", "a-catalyst", "portfolio-mgr"));
    edges.push(edge("e-t-valuation-portfolio-mgr", "t-valuation", "portfolio-mgr"));
    // V79(2026-09-21): 新增的 `t-valuation-band` 同样必须在此补边 —— 它被
    //   portfolio-mgr 的 `input_mapping.valuation_pe_percentile` 引用，缺边则变量不进入
    //   `context.variables` ⇒ 第三锚腿恒不可用。与上面 5 条**完全同一失效机制**
    //   （这正是本段注释开头描述的缺陷族：写进 input_mapping ≠ 有人供给）。
    edges.push(edge("e-t-valuation-band-portfolio-mgr", "t-valuation-band", "portfolio-mgr"));
    edges.push(edge("e-pace-calc-portfolio-mgr", "pace-calc", "portfolio-mgr"));
    edges.push(edge("e-cls-risk-level-portfolio-mgr", "cls-risk-level", "portfolio-mgr"));

    // ── PACE 情绪因子（f11）: pace-calc.rhai — 基于公告的四维情绪向量计算 ──
    // pace-calc.rhai 已实现完整的 PACE 计算逻辑（Polarity/Actuality/Credibility/Expectation），
    // 基于 t-catalyst-data 的公告数据输出 pace_signal（[-1, 1]）。
    // 输出: {pace_vector:{P,A,C,E}, pace_signal, ...}
    // portfolio-mgr 消费 pace_signal 作为 f11_signal。
    {
        let pace_id = "pace-calc";
        let pace_code = include_str!("../pace-calc.rhai").to_string();
        nodes.push(WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: pace_id.into(),
                title: "PACE 情绪因子计算（f11）".into(),
                description: Some(
                    "基于公告数据计算四维情绪向量，输出 pace_signal 作为 f11 因子信号".into(),
                ),
                // 放在 portfolio-mgr 左侧同一行，与 regime-weights 对称
                position: Position { x: 460.0, y: 4200.0 },
                retry: RetryConfig::default(),
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: true, // PACE 失败不应阻断主流程
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: pace_code,
                output_var: pace_id.into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    // 主事件源：公告数据（fallback 路径，pace-calc 会从中提取 event_type）
                    // 2026-09-09: 数据在 result.content 字符串中（终值保持字符串原样，
                    // pace-calc.rhai 按契约 json_parse）
                    ("announcement_events", "t-catalyst-data.result.content"),
                    // 资金流向数据（用于背离修正）
                    // 2026-09-09: MoneyFlow 序列化为 camelCase（mainNetInflow），
                    // 且数据在 result.content 字符串层
                    ("money_flow_net", "t-hotmoney-data.result.content.mainNetInflow"),
                    // 资金流向历史序列（近5日，含当日），用于趋势背离判断
                    // 注：当前 get_money_flow 实现只返回最新一日（无 history 字段），
                    // 此映射取不到值时 pace-calc 按缺失降级
                    ("money_flow_history", "t-hotmoney-data.result.content.history"),
                    // 板块 ETF 资金流向（用于协同增强）- 暂未接入
                    ("sector_etf_direction", ""),
                    // 历史 P 值 - 暂未接入（需要 upstream LLM 长期输出）
                    ("p_history", ""),
                    // LLM 事件源：a-catalyst 输出（含 catalyst_level/confidence/verdict）
                    // P0 修复(2026-07-22): 原为空占位导致 pace-calc 在 a-catalyst 完成前执行，
                    // 且 llm_events 恒为空。现在接入 a-catalyst.content，pace-calc.rhai 会
                    // 从 catalyst_level 中提取事件类型。
                    // V58 修复: a-catalyst 是 AgentNode（输出 {role, content, ...}），
                    // 不是 ToolNode/CodeNode，路径应为 .content 而非 .result
                    ("llm_events", "a-catalyst.content"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }));
        // pace-calc 依赖 t-catalyst-data 和 t-hotmoney-data（工具节点）
        edges.push(edge("e-t-catalyst-data-pace-calc", "t-catalyst-data", pace_id));
        edges.push(edge("e-t-hotmoney-data-pace-calc", "t-hotmoney-data", pace_id));
        // P0 修复(2026-07-22): 添加 pace-calc → a-catalyst 依赖边
        // 原缺失此边导致 pace-calc 在 a-catalyst 完成前就执行，llm_events 恒为 null
        edges.push(edge("e-a-catalyst-pace-calc", "a-catalyst", pace_id));
        // pace-calc → portfolio-mgr: pace_signal 作为 f11 输入
        edges.push(edge("e-pace-calc-portfolio-mgr", pace_id, "portfolio-mgr"));
    }

    // ── P3 (real-nodes): rule-check 规则检查 Agent ──
    // 在 portfolio-mgr 完成后启动，对照硬性规则阈值（RSI/乖离率/止损/放量下跌/空头排列）
    // 检查交易方案是否违规，输出 violations / corrections / force_signals
    {
        let rc_id = "rule-check";
        let rc_title = "硬性规则检查：RSI超买/乖离率追高/缺失止损/放量下跌/空头排列";
        let rc_y = 4200.0;
        let mut rc = agent(rc_id, rc_title, "rule-checker", None, 700.0, rc_y);
        if let WorkflowNode::Agent(ref mut a) = rc {
            a.config.context_sources = vec![
                "portfolio-risk-gate".into(),
                "t-scoring".into(),
                "t-valuation".into(),
                "t-risk".into(),
                "trader".into(),
            ];
            // P0 修复(2026-07-22): rule-check 改为纯决策节点，移除全部工具。
            // 原问题与 trader 同款：context_sources 已包含 t-scoring/t-valuation/t-risk，
            // 这些上游 ToolNode 的输出就是 compute_scoring/compute_valuation/
            // compute_portfolio_risk 的计算结果。LLM 重新调用这些工具属于重复获取，
            // 且 compute_* 工具需要 stock_code 参数，Agent 节点工具参数由 LLM 自主
            // 生成，容易传空值触发无效重试。rule-check 的职责是对照硬性规则阈值
            // 检查交易方案是否违规，所需技术指标/估值/风控数据已通过 context_sources
            // 注入，无需重新计算。
            a.config.tools = vec![];
            a.config.exposed_tools = vec![];
            a.config.max_tool_rounds = Some(0);
            a.config.system_prompt = format!(
                "{}\n\n--- 数据约束 ---\n\
                 你是硬性规则检查员，所有需要的数据已通过输入上下文注入，禁止调用任何工具重新获取或计算。\n\
                 - 技术指标(RSI/MACD/乖离率等): 参考 t-scoring 的 indicators\n\
                 - 估值数据(DCF/格雷厄姆/F-Score): 参考 t-valuation\n\
                 - 风险指标(波动率/最大回撤/夏普比率): 参考 t-risk\n\
                 - 交易方案(入场价/目标价/止损价): 参考 trader\n\
                 - 组合决策(action/positionPct): 参考 portfolio-risk-gate\n\
                 基于上述数据直接检查交易方案是否违反硬性规则（RSI超买/乖离率追高/缺失止损/放量下跌/空头排列），\n\
                 输出 violations / corrections / force_signals。",
                a.config.system_prompt
            );
        }
        nodes.push(rc);
        // ── P1-E13: portfolio-risk-gate → rule-check（替代原 portfolio-mgr → rule-check）──
        // rule-check 现在从组合风控门获取最终决策（含风控调整），而非直接从 portfolio-mgr
        edges.push(edge("e-risk-gate-rule-check", "portfolio-risk-gate", rc_id));
        edges.push(edge("e-rule-check-quality-gate", rc_id, "quality-gate"));
        // data-quality → quality-gate: 显式边确保 data-quality 变量在 switch 判断前就绪
        edges.push(edge("e-data-quality-quality-gate", "data-quality", "quality-gate"));
    }

    // ── SwitchNode: 数据质量门禁 ──
    // 检查 data-quality Agent 的 JSON 输出中的 grade 字段（A/B/C/D/F），C 级以上继续，D/F 走降级路径。
    // data-quality 输出为 JSON 格式，resolve_var_path 导航到 content.grade 提取等级。
    nodes.push(WorkflowNode::Switch(SwitchNode {
        base: WorkflowNodeBase {
            id: "quality-gate".into(),
            title: "数据质量门禁".into(),
            description: Some("检查数据质量等级，A/B/C 级以上继续，D/F 走保守降级路径".into()),
            position: Position { x: 700.0, y: 4500.0 },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
            // v10: rule-check（LLM）失败时仍派发 switch——其 input_var 是
            // data-quality.result.grade（确定性节点输出），与 rule-check 无关，
            // 可正常路由；rule-check 输出缺失由下游 context_sources 静默跳过兜底。
            continue_on_fail: true,
        },
        config: SwitchNodeConfig {
            // data-quality 是 CodeNode（Rhai），输出结构为
            //   {status, result: {grade, score, ...}, input_params, node_id, params}
            // resolve_var_path 需要导航到 .result.grade 才能取到等级字段。
            // 旧值 "data-quality.grade" 无法穿透 CodeNode 的 result 包装，
            // 导致 A 级数据被误判为低质量，路由到 quality-fallback 保守路径
            // （其 prompt 不输出 confidence 字段 → 前端显示"持有 0%"）。
            input_var: "data-quality.result.grade".into(),
            cases: vec![SwitchCase {
                // v40(2026-09-13): 注入变量名由 `_value` 改为 `value`。
                //   Rhai 拒绝一切下划线开头的标识符 ⇒ 旧写法 `_value == "A" || …`
                //   在 `SwitchExecutor` 里恒解析失败、恒回落 default_case
                //   （实证：grade="C" 却 matched_label="low-quality"）。
                //   执行器侧已加 normalize（历史 `_value` 写法仍兼容），此处同步为规范写法。
                value: "value == \"A\" || value == \"B\" || value == \"C\"".into(),
                label: "acceptable".into(),
            }],
            default_case: Some("low-quality".into()),
            match_mode: "expression".into(),
            use_llm: None,
            llm_prompt: None,
            llm_model: None,
            output_var: "quality-gate-result".into(),
        },
    }));

    // ── Agent: 降级处理路径（数据质量不足时生成保守决策）──
    {
        let fq_id = "quality-fallback";
        let fq_title = "数据不足→保守决策：持仓不变/减仓观望";
        let fq_y = 4500.0;
        let mut fq = agent(fq_id, fq_title, "quality-fallback", None, 20.0, fq_y);
        if let WorkflowNode::Agent(ref mut a) = fq {
            // v10: quality-gate 自身失败（如 data-quality 失败致 grade 缺失）时，
            // switch 的两条出边依赖 source Failed + target cof 才能放行——本节点与
            // decision-explainer 必须同时 cof=true，否则 default 分支永久 Pending。
            a.base.continue_on_fail = true;
            a.config.context_sources = vec![
                "rule-check".into(),
                "data-quality".into(),
                "t-scoring".into(),
                "t-valuation".into(),
                "t-risk".into(),
            ];
            a.config.output_mode = OutputMode::Json;
            // P0 修复(2026-07-22): 移除 td_quote/td_kline/td_score——context_sources 已包含
            // t-scoring/t-valuation/t-risk，上游数据已注入。原配置与 trader 同款问题：
            // LLM 重新获取数据会传入空 stock_code，触发无效重试。
            a.config.tools = vec![];
            a.config.system_prompt =
                "数据质量评估为 D 或 F，上游分析数据不可靠。你需要在数据不足的情况下做出最保守的投资决策。\
                 所有需要的数据已通过输入上下文注入（t-scoring 的 currentPrice/indicators/totalScore、\
                 t-valuation 的估值、t-risk 的风险评分），禁止调用任何工具重新获取数据。\
                 输出JSON格式（严格模式）：{\"action\":\"持有/减持/卖出\",\"positionPct\":0-20,\"confidence\":20-40,\"riskLevel\":\"高风险\",\"reasoning\":\"保守决策理由\"}\
                 规则：action 只能是'持有'/'减持'/'卖出'（禁止买入/增持）；positionPct 0-20（保守低仓位）；\
                 confidence 20-40（数据不足时置信度低，D级给30-40，F级给20-30）；riskLevel 固定为'高风险'。\
                 只输出上述JSON对象，前后不要有任何其他文字"
                    .to_string();
            a.config.exposed_tools = vec![];
            a.config.max_tool_rounds = Some(0);
        }
        nodes.push(fq);
        // Switch 出边：
        //   case "acceptable" → notify-result（source_handle = 匹配的 case label）
        //   default → quality-fallback（无 source_handle）
        edges.push(WorkflowEdge {
            id: "e-quality-gate-notify".into(),
            source: "quality-gate".into(),
            source_handle: Some("acceptable".into()),
            target: "notify-result".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: Some("通过 ✓".into()),
        });
        edges.push(WorkflowEdge {
            id: "e-quality-gate-quality-fallback".into(),
            source: "quality-gate".into(),
            source_handle: None,
            target: fq_id.into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: Some("降级 →".into()),
        });
    }
    // quality-fallback 降级完成后同样触发 explainer
    edges.push(edge("e-quality-fallback-explainer", "quality-fallback", "decision-explainer"));

    // ── P0 补: decision-explainer（三明治第三段）──
    // 在 portfolio-risk-gate 组合风控门完成后，用 LLM 生成决策依据说明书 + 规则追溯码
    // 输入: portfolio-risk-gate 的 final_action / confidence / reasoning / decision_trail
    // 输出: 自然语言解释文案，带规则追溯码 R-xxx
    {
        let de_id = "decision-explainer";
        let de_title = "决策解释：将硬裁决结果翻译为自然语言说明书，附带规则追溯码";
        let mut de = agent(de_id, de_title, "explainer", None, 700.0, 4400.0);
        if let WorkflowNode::Agent(ref mut a) = de {
            // v10: explainer 是全链失败率最高的节点（LLM + input_mapping 依赖
            // portfolio-risk-gate），其失败绝不能阻断 notify/store/end——决策已由
            // portfolio-risk-gate 硬裁决产出，explainer 只是"翻译说明书"（增强信息）。
            // 同时与 quality-fallback cof=true 配对，覆盖 switch 失败场景（见上）。
            a.base.continue_on_fail = true;
            a.config.context_sources = vec![
                "portfolio-risk-gate".into(),
                "rule-check".into(),
                "t-scoring".into(),
                "t-risk".into(),
            ];
            a.config.output_mode = OutputMode::Json;
            // P0 修复(2026-07-22): 移除 td_quote/td_score——context_sources 已包含 t-scoring，
            // 上游 currentPrice/indicators/totalScore 已注入。explainer 职责是翻译规则引擎
            // 裁决结果为自然语言，不需要重新获取行情或计算评分。
            a.config.tools = vec![];
            a.config.exposed_tools = vec![];
            a.config.max_tool_rounds = Some(0);
            a.config.system_prompt = format!(
                "{}\n{}",
                "你是投资决策解释官。输入是符号系统（Rhai 规则引擎）的硬裁决结果，你的任务是将裁决翻译为用户可读的决策依据说明书。",
                "输出 JSON 格式（严格模式）：\n\
                 {\n\
                   \"summary\": \"一段话摘要（50-100字），包含最终行动、仓位、置信度\",\n\
                   \"explanation\": \"详细的决策依据说明（200-300字），解释为什么做出这个决策\",\n\
                   \"rule_trace\": [\n\
                     {\"rule_id\": \"R-xxx\", \"status\": \"PASS/VETOED/DOWNGRADED\", \"description\": \"规则的通俗解释\"}\n\
                   ],\n\
                   \"risk_comment\": \"风险提示（如有）\",\n\
                   \"confidence_note\": \"置信度解读\"\n\
                 }\n\
                 规则追溯码对照表（以下为代码中**实际存在**的编号，禁止臆造未列出的编号）：\n\
                 【公式决策层 portfolio-mgr】\n\
                 R-200 风险否决降档 | R-201 空头预测降档 | R-202 trader数据异常\n\
                 R-203 因子权重坍缩 | R-204 价格信号无信息量（|targetPrice−currentPrice|/currentPrice<0.5%，非数据错误）\n\
                 R-205 单点数据不可信部分降级 | R-206 数据质量坍缩 | R-207 试探仓（posterior∈[0.42,0.50) 且赔率>0）\n\
                 【组合风控门 portfolio-risk-gate（最终生效层）】\n\
                 R-200 极高风险档位禁止持仓 | R-201 空头预测强制卖出（目标价<现价×0.85）\n\
                 R-206 单股仓位超上限已下调 | R-207 组合总价值为0仓位清零 | R-208 风控否决\n\
                 R-209 组合总仓位超100%归一化 | R-210 已持有该股本次为加仓\n\
                 【技术面否决（由 portfolio-mgr 产出，消费 t-scoring 的 RSI/MACD）】\n\
                 R-401 RSI>80追高否决 | R-402 RSI<20恐慌否决 | R-405 RSI>80且MACD DIF<0双重否决\n\
                 ⚠️ R-200/R-201/R-206/R-207 在「公式决策层」与「风控门」各自有含义：\n\
                    判定归属时以输入上下文里对应的 reasons / reasoning 文本为准，不要只看编号。\n\
                    R-403/R-404 在代码中**没有任何实现**，不得写入 rule_trace。\n\
                    若某规则未在上下文中出现，不要把它写进 rule_trace。\n\
                 只输出上述JSON对象，前后不要有任何其他文字"
            );
            a.config.input_mapping = [
                // P1-E13: decision-explainer 现在从 portfolio-risk-gate 读取最终决策
                // portfolio-risk-gate 是 CodeNode，输出结构为
                //   {status, result: {action, confidence, ..., risk_gate: {...}}, ...}
                // 保留了 portfolio-mgr 的所有字段，并覆盖了被风控门调整的字段
                ("pm_action", "portfolio-risk-gate.result.action"),
                ("pm_confidence", "portfolio-risk-gate.result.confidence"),
                ("pm_position_pct", "portfolio-risk-gate.result.positionPct"),
                ("pm_reasoning", "portfolio-risk-gate.result.reasoning"),
                ("pm_risk_level", "portfolio-risk-gate.result.riskLevel"),
                ("pm_stop_loss", "portfolio-risk-gate.result.stopLossPct"),
                ("pm_take_profit", "portfolio-risk-gate.result.takeProfitPct"),
                ("pm_decision_trail", "portfolio-risk-gate.result.decision_trail"),
                ("pm_target_timeframe", "portfolio-risk-gate.result.targetTimeframe"),
                ("pm_computation_logs", "portfolio-risk-gate.result.computation_logs"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        }
        nodes.push(de);
        // quality-gate 的 acceptable 路径 → decision-explainer
        //（覆盖原来的 e-quality-gate-notify，下面重新建边到 notify-result）
    }
    // ── 重定向: quality-gate acceptable → decision-explainer ──
    // 替换步骤 1: 注册新边 (source_handle="acceptable" → decision-explainer)
    edges.push(WorkflowEdge {
        id: "e-quality-gate-explainer".into(),
        source: "quality-gate".into(),
        source_handle: Some("acceptable".into()),
        target: "decision-explainer".into(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: Some("通过 ✓→解释".into()),
    });
    // 替换步骤 2: 删除旧边 e-quality-gate-notify（遍历时过滤掉）
    edges.retain(|e| e.id != "e-quality-gate-notify");
    // ── sim-verify: 仿真验证（**仅图示**，不参与 DAG 执行）──
    //
    // 为什么必须「enabled=false」且「不连任何边」——两条缺一会破坏调度：
    //   1. `compute_ready_nodes`（rt-workflow/…/dag_store.rs:304-315）以「入度为 0」
    //      判定就绪 ⇒ 只把节点改成孤立（无入边）会让它被当成就绪节点**立即执行一次**。
    //   2. `remaining_deps`（同文件 :218-260）按边计数、且只认 `done_or_skipped`
    //      的上游 ⇒ 若给 `enabled=false` 的节点保留出边，它永远不进 done 集，
    //      下游 `store-result` 会**永久 Pending**。
    //   ⇒ 只有同时满足「enabled=false + 无边」，才能得到一个「图上可见但不参与调度」
    //     的节点。
    //
    // 真正的仿真执行在**决策落库之后**由后端挂钩触发（`stock_workflow/core.rs`，
    // 与 `price_alerts` 自动创建同一挂载点），因此**不占用工作流执行时长**。
    // 保留该节点的唯一目的是让流程图如实呈现「决策之后有一个仿真环节」，
    // 位置紧随 `store-result`（持久化）之后。
    {
        let sv_code = include_str!("../sim-verify.rhai").to_string();
        let sv = WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: "sim-verify".into(),
                title: "仿真验证（决策后压力测试）".into(),
                description: Some(
                    "决策落库后由后端挂钩自动触发（不在 DAG 执行链上，不占工作流时长）：\
                     对该标的跑蒙特卡洛多场景压力测试，产出「最坏情形」视角的补充信息。\
                     只读上游、不产出决策字段，不会阻滞或改写决策。"
                        .into(),
                ),
                position: Position { x: 700.0, y: 4800.0 },
                retry: RetryConfig::default(),
                timeout: Some(30),
                // ⚠️ 见上方注释：enabled=false 与「无边」必须成对出现
                enabled: false,
                parent_id: None,
                compensation: None,
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: sv_code,
                output_var: "sim-verify".into(),
                tool_name: None,
                execute_directly: true,
                // 该节点不参与调度，input_mapping 仅作「需要哪些输入」的声明留档；
                // 实际取值由 core.rs 挂钩处从 t-scoring 结果与全局变量中读取。
                input_mapping: [
                    ("stock_code", "stock_code"),
                    ("current_price", "t-scoring.result.content.currentPrice"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        });
        nodes.push(sv);
    }

    // explainer 完成后通知 + 持久化（直连，不经 sim-verify —— 见上方说明）
    edges.push(edge("e-explainer-notify", "decision-explainer", "notify-result"));
    edges.push(edge("e-explainer-store", "decision-explainer", "store-result"));

    // ── NotificationNode: 分析完成通知 ──
    nodes.push(WorkflowNode::Notification(NotificationNode {
        base: WorkflowNodeBase {
            id: "notify-result".into(),
            title: "分析完成通知".into(),
            description: Some("股票分析完成后发送通知".into()),
            position: Position { x: 300.0, y: 4500.0 },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
            // v10: 通知发送失败不应阻断 store-result/end-output（消息固定文本，
            // 无下游消费其输出）。
            continue_on_fail: true,
        },
        config: NotificationNodeConfig {
            channel: "system".into(),
            message: "股票分析已完成，请查看决策结果".into(),
            webhook_url: None,
            recipients: vec![],
            subject: Some("股票分析完成".into()),
            enabled: true,
            output_var: "notification".into(),
        },
    }));
    // 注：移除 e-portfolio-mgr-notify 直连，notify-result 现在仅由 rule-check 完成后触发

    // ── StorageNode: 分析结果持久化 ──
    // 将完整分析结果（portfolio-risk-gate 最终决策）写入 SQLite history 表，供后续回测/复盘引用。
    nodes.push(WorkflowNode::Storage(StorageNode {
        base: WorkflowNodeBase {
            id: "store-result".into(),
            title: "分析结果持久化".into(),
            description: Some("写入分析结果到历史记录表".into()),
            position: Position { x: 300.0, y: 4800.0 },
            retry: RetryConfig { enabled: true, max_retries: 2, ..Default::default() },
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
            // v10: 持久化失败（已带 retry 2）不应阻断 end-output——最终输出直接取
            // portfolio-risk-gate，不消费 storage-result。
            continue_on_fail: true,
        },
        config: StorageNodeConfig {
            backend: "sqlite".into(),
            operation: "insert".into(),
            input_var: "portfolio-risk-gate".into(),
            collection: "analysis_history".into(),
            key_var: None,
            output_var: "storage-result".into(),
        },
    }));
    edges.push(edge("e-notify-store-result", "notify-result", "store-result"));
    // store-result 直接从 portfolio-risk-gate 取决策变量，绕过 state.variables 查找
    edges.push(edge("e-risk-gate-store-result", "portfolio-risk-gate", "store-result"));

    // EndNode: 把 portfolio-risk-gate 输出提升为工作流顶层输出
    nodes.push(WorkflowNode::End(EndNode {
        base: WorkflowNodeBase {
            id: "end-output".into(),
            title: "最终输出".into(),
            description: Some("将 portfolio-risk-gate 决策结果提升到工作流输出".into()),
            position: Position { x: 300.0, y: 5100.0 },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
            // v10: store-result 失败时仍提升 portfolio-risk-gate 输出为顶层结果，
            // 保证工作流正常 Completed 且决策可达前端。
            continue_on_fail: true,
        },
        config: EndNodeConfig { output_var: Some("portfolio-risk-gate".into()) },
    }));
    edges.push(edge("e-store-end", "store-result", "end-output"));

    // 构建 input_schema / output_schema / variables
    let mut input_props = std::collections::HashMap::new();
    input_props.insert(
        "stock_code".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("股票代码，如 000001、600519".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    let input_schema_val = serde_json::to_string(&JsonSchema {
        schema_type: "object".to_string(),
        description: Some("股票分析运行时输入".to_string()),
        properties: Some(input_props),
        required: Some(vec!["stock_code".to_string()]),
        items: None,
    })
    .unwrap();

    let mut output_props = std::collections::HashMap::new();
    output_props.insert(
        "action".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("投资决策: 买入/增持/持有/减持/卖出".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "positionPct".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some("建议仓位百分比 (0-100)".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "targetPrice".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some(
                "目标价。方向语义必须与 action 一致：看多(买入/增持)须 > currentPrice；\
                 看空(减持/卖出)须 < currentPrice；持有/观望**不要输出该字段**。\
                 禁止输出等于 currentPrice 的数值——等值不含方向信息，会被下游判为无效价格信号。"
                    .to_string(),
            ),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "stopLoss".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some(
                "止损价。看多方向在 currentPrice 下方、看空方向在 currentPrice 上方；\
                 必须与 targetPrice 分居现价两侧（禁止 targetPrice <= stopLoss 的倒置）。"
                    .to_string(),
            ),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "reasoning".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("决策理由 (300字以内)".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "riskLevel".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("风险等级: 低/中/高".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "confidence".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some("置信度 (0-100)".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "riskSource".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("风险分类来源: 算法/LLM回退/无数据".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    let output_schema_val = serde_json::to_string(&JsonSchema {
        schema_type: "object".to_string(),
        description: Some("股票分析最终决策输出".to_string()),
        properties: Some(output_props),
        required: None,
        items: None,
    })
    .unwrap();

    // ── 模板变量定义（拆分到 seed_variables.rs） ──
    use super::seed_variables::build_template_variables;
    let variables: Vec<Variable> = build_template_variables();

    let variables_val = serde_json::to_string(&variables).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化变量失败: {e}"))
    })?;

    // ── 合并旧版本的变量值（保留用户自定义的评分权重/阈值等）──
    let variables_val = match old_variables {
        Some(ref ov) if !ov.is_empty() => {
            merge_variable_values(&variables_val, ov).unwrap_or_else(|_| variables_val.clone())
        },
        _ => variables_val,
    };

    // ── 辩论轮数：建图展开轮数与落库变量值强制同源 ──
    // `debate_max_rounds`（resolve_debate_rounds）驱动 DAG 展开几对 bull/bear 节点、
    // 下游锚点 bear-rN。此处用同一值覆写变量表，保证「建图、下游锚点、变量表」
    // 三处严格一致 —— 用户在前端把「多空辩论轮数」设成几，重建后即真跑几轮，
    // 不会出现变量表与 DAG 展开数不匹配的悬空入边。
    let variables_val =
        force_variable_value(&variables_val, "debate_rounds", serde_json::json!(debate_max_rounds));

    // ── DCF 估值参数：存量**一次性**迁移（v74 引入；D11 修复于 2026-09-22）──
    //
    // 目标：把 DB 存量里「2026-09-12 校准前」的旧值迁到校准后的值。取值一律引用
    // `seed_variables` 的常量（与变量定义同源，不在此手抄数值 —— 手抄会让口径漂移）。
    //
    // ⚠️ 为什么必须是「一次性」（D11 实测）：本 force 自 v74 引入起**从未生效过** ——
    //    落库构造点的 `variables` 分支会再用 `old_variables` 逐名把 `value` 覆盖回旧值，
    //    而该分支位于本 force **之后** ⇒ 刚写进去的新值被抹掉。DB 实证：活行 `version=75`
    //    （= 种子成功跑过多次，版本号就是它写的）而三参数仍是校准前的旧值。
    //    该覆盖分支与 `merge_variable_values` 语义完全重复，已删除（见落库处注释）。
    //
    // ⚠️ 为什么门必须锚在「旧版本 < DCF_MIGRATION_VERSION」而不是无条件执行：
    //    删掉冗余覆盖之后 force 会真的生效；若仍无条件执行，则**每次**升版都会把用户
    //    在面板里调好的 DCF 参数打回默认。定为「旧版本 < 76」⇒ 恰好只在本次校正存量时
    //    执行一次，此后尊重用户设置。`previous_version == None`（首次创建模板）也执行
    //    ⇒ 首次落库即为校准后值。
    //
    // 影响量化（688114 实测）：`discount 10→8.5` + `perpetual 3→4` 使
    // `dcf.mid` 24.23 → 37.27（+53.8%），区间 16.33-38.84 → 22.39-71.14。
    //
    // ⚠️ **v77（2026-09-23）：门由 76 抬到 77 —— 第二次一次性校正（永续 4%→2%）**。
    //    为什么必须抬：本门是「旧版本 < `DCF_MIGRATION_VERSION`」**一次性**门。
    //    v76 那次校正过后 DB 已停在 76 ⇒ 若不同步抬门，`PERPETUAL_GROWTH` 从
    //    0.04 改到 0.02 将**只改代码常量与种子默认值**，而 DB 存量变量仍是 4.0；
    //    又因为 t-valuation 的 input_mapping 把 `value_dcf_perpetual_rate` 作为
    //    **扁平参数**传入（优先于模块常量），生效的是 4.0 ⇒ **改动等于没改**。
    //    这正是 v74 那次「只改代码常量是假修复」的同型复发，故沿用同一处置。
    //    代价（明示）：会把这期间用户在面板里手调过的永续增长率一并覆盖为 2.0。
    //    取此代价的理由：4.0% **超过模型自身申报的无风险利率 2.5%**，违反终值约束
    //    `g_terminal ≤ risk-free rate`，属**口径错误**而非偏好差异，不应被「用户设置」
    //    固化（详见 `astock_data::mcp_tools::PERPETUAL_GROWTH` 的文档注释）。
    //
    // ⚠️ **v78（2026-09-23 同日二改）：门由 77 抬到 78 —— 无风险利率口径订正**。
    //    改了什么：`RISK_FREE_RATE` 0.025 → **0.017**（旧值实为 1 年期 MLF 政策利率
    //    2.50%，而 DCF 需要与现金流久期匹配的 **10 年期国债** —— 中债 10Y 实测 1.675%）；
    //    连带 `DISCOUNT_RATE` 0.085 → **0.077**、`PERPETUAL_GROWTH` 0.02 → **0.015**
    //    （订正 `r_f` 后原 2.0% 越过了 `g_terminal ≤ r_f` 的硬约束）。
    //    为什么必须抬门：与 v77 完全同因 —— 不平抬，DB 存量的 `discount_rate=8.5` 会
    //    经扁平参数继续生效，代码常量改动**等于没改**（v74/v77 同型复发第三次）。
    //    代价（明示）：再次覆盖面板手调值。DB 实测（sci10）：三参数当前恰等于 v76 规范值
    //    （`12 / 4 / 8.5`）⇒ **用户并未手调过**，本轮覆盖的实际代价仍为零，可核对。
    //    注：`DEFAULT_DCF_*_PCT` 三个常量均由 `mcp_tools::*` 派生（`* 100.0`），
    //    故本次无需改动 `seed_variables.rs` —— 它随常量自动取到 `12.0 / 1.5 / 7.7`。
    //
    // ⚠️ **v79（2026-09-23 同日三改）：门由 78 再抬到 79 —— `PERPETUAL_GROWTH` 二次订正**。
    //    改了什么：`PERPETUAL_GROWTH` 0.015 → **0.013**。
    //    为什么：v78 把 `MAX_PERPETUAL_GROWTH` 锚到 `r_f = 1.7%` 后，乐观档的
    //    `p × 1.3 = 1.95%` **越过该上限**被静默砍到 `×1.133`，而面板文案仍声称「×1.3」
    //    —— 与 v77 在**增长率**维度修掉的缺陷同型（当时 `g × 1.5` 被 `MAX_GROWTH` 吞掉）。
    //    处置：由不变量 `p × 1.3 ≤ r_f` 反推 `p ≤ 1.3077%` ⇒ 取 **1.3%**，
    //    并在 `mcp_tools.rs` 加**编译期断言**锁死（此后任一常量漂移即编译失败）。
    //
    //    ⚠️⚠️ **为什么改了常量却必须再抬一次门（本次的关键教训）**：
    //    这是一个**一次性**门（`existing.version < DCF_MIGRATION_VERSION`）。若应用在
    //    v78 上启动过一次并完成了 force，则 DB 版本已变成 78 ⇒ 我再改常量到 1.3% 时
    //    `76 < 78` 已为假、`78 < 78` 也为假 ⇒ **新值永远进不去 DB**，而代码里读起来
    //    一切正常（flat 参数继续用 1.5%）—— **第四种同型复发**，且比前三者更隐蔽：
    //    前三次是「门没抬」，这次是「门抬了但抬在改动之前」。
    //    ⇒ 判据：**凡在某个 `_MIGRATION_VERSION` 声明之后又改了被它守护的常量，
    //       必须再抬一次**；等价地，「版本号」应对应「值的最后变更」，不是「本轮开始」。
    //
    // ⚠️⚠️ **v79 补记（第五种形态）：门抬了，但「落库版本号」没跟着抬**。
    //    前四次的形态都是「门的值 vs 被守护常量」错位；本次新暴露的是**另一条独立水位线**：
    //    落库写的是 `TEMPLATE_VERSION`（落库点：`version: Set(TEMPLATE_VERSION)`），而本门读的是 `DCF_MIGRATION_VERSION`。
    //    两者一旦倒挂（`TEMPLATE_VERSION < 门`），DB 会停在一个**仍满足 `版本 < 门`** 的值上
    //    ⇒ **门永远关不上** ⇒ 此后为任何**无关**模板改动升版都会**再次**触发本 force，
    //    把用户在面板里调好的 DCF 参数打回默认。
    //    ⇒ 判据：**幂等门的「水位线」必须 ≤ 落库版本号**（门关闭与落库在同一步完成）；
    //       等价地，`_MIGRATION_VERSION` 的取值不得**超前**于承载它的 `TEMPLATE_VERSION`。
    //    处置：`TEMPLATE_VERSION` 77 → **79**（与门对齐），并在两常量旁加**编译期断言**
    //    `TEMPLATE_VERSION >= DCF_MIGRATION_VERSION` ⇒ 此后不再依赖「改的人记得」。
    //    门值本体见模块级 `DCF_MIGRATION_VERSION`（提到模块级正是为了让这条断言成立）。
    let variables_val = if previous_version.is_none_or(|v| v < DCF_MIGRATION_VERSION) {
        let v = force_variable_value(
            &variables_val,
            "value_dcf_discount_rate",
            serde_json::json!(super::seed_variables::DEFAULT_DCF_DISCOUNT_RATE_PCT),
        );
        let v = force_variable_value(
            &v,
            "value_dcf_perpetual_rate",
            serde_json::json!(super::seed_variables::DEFAULT_DCF_PERPETUAL_RATE_PCT),
        );
        force_variable_value(
            &v,
            "value_dcf_growth_rate",
            serde_json::json!(super::seed_variables::DEFAULT_DCF_GROWTH_RATE_PCT),
        )
    } else {
        variables_val
    };

    // ── Phase 3/4: Rhai 综合评分工具 + ErrorConfig ──
    use crate::commands::error::ErrorResponse;
    use axagent_harness::workflow_types::RhaiToolDef;
    let stock_score_rhai = r##"
// 综合评分脚本：技术面(30%) + 基本面(25%) + 情绪面(20%) + 资金面(15%) + 政策面(10%)
let w_tech = ctx.variables.weight_technical ?? 30.0;
let w_fund = ctx.variables.weight_fundamental ?? 25.0;
let w_sent = ctx.variables.weight_sentiment ?? 20.0;
let w_flow = ctx.variables.weight_money_flow ?? 15.0;
let w_pol = ctx.variables.weight_policy ?? 10.0;

let tech = ctx.results["a-market-analyst"] ?? 50.0;
let fund = ctx.results["a-fundamentals"] ?? 50.0;
let sent = ctx.results["a-sentiment"] ?? 50.0;
let flow = ctx.results["a-hot-money"] ?? 50.0;
let pol = ctx.results["a-policy"] ?? 50.0;

let score = (tech * w_tech + fund * w_fund + sent * w_sent + flow * w_flow + pol * w_pol) / 100.0;
#{
    score: score,
    level: if score >= 80 { "强烈推荐" }
           else if score >= 60 { "推荐" }
           else if score >= 40 { "中性" }
           else { "回避" }
}
"##;
    let rhai_tool_defs: Vec<RhaiToolDef> = vec![RhaiToolDef {
        tool_name: "compute_stock_score".into(),
        description: Some("综合技术面/基本面/情绪面/资金面/政策面计算 0-100 评分".into()),
        code: stock_score_rhai.into(),
    }];
    let tool_defs_val = serde_json::to_string(&rhai_tool_defs).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("序列化 Rhai 工具定义失败: {e}"))
    })?;

    let error_config = ErrorConfig {
        retry_policy: Some(WorkflowRetryPolicy {
            max_retries: 3,
            base_delay_ms: 1000,
            max_delay_ms: 30000,
        }),
        on_failure: OnFailureAction::ContinueWithDefault,
        error_branch: None,
        compensation_steps: None,
    };

    let error_config_val = serde_json::to_string(&error_config).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("序列化 ErrorConfig 失败: {e}"))
    })?;

    /// 将子图节点坐标从绝对坐标转换为相对容器的偏移。
    /// 种子数据中的节点坐标是画布绝对坐标，但编辑器 Phase 3 的 subGraph 注入
    /// 将 subGraph 节点 position 视为相对容器的偏移（editor 叠加容器 position
    /// 计算绝对坐标），因此必须在注入前转换。
    fn adjust_positions_to_relative(
        mut sub_nodes: Vec<WorkflowNode>,
        container_id: &str,
        all_nodes: &[WorkflowNode],
    ) -> Vec<WorkflowNode> {
        let container_pos = all_nodes
            .iter()
            .find(|n| n.base_id() == container_id)
            .map(|n| n.base().position.clone())
            .unwrap_or(Position { x: 0.0, y: 0.0 });
        for node in sub_nodes.iter_mut() {
            match node {
                WorkflowNode::Trigger(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Agent(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Llm(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Condition(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Parallel(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Loop(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Merge(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Delay(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Validation(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::SubWorkflow(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::DocumentParser(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::VectorRetrieve(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::End(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::HttpRequest(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Switch(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::DatabaseQuery(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Notification(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Approval(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::FileOperation(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::DataTransformer(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::WebhookSend(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Logging(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::LlmClassifier(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Aggregator(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Email(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Debate(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Swarm(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Storage(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Tool(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Code(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::MultiAgent(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::WorkflowRef(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
            }
        }
        sub_nodes
    }

    // ── 注入容器节点子图（subGraph）用于编辑器嵌套渲染 ──
    // 子图仅在编辑器的 ReactFlow 渲染层中用于坐标转换（绝对→相对），
    // 运行时引擎仍从顶层 nodes 读取所有节点。
    // 编辑器保存时会自动去重（上游 WorkflowEditor.tsx save 路径过滤 subGraph 子节点）。
    let container_nodes: &[&str] = &["p-analysts", "debate-bull-bear", "p-risk-assess"];
    for &cid in container_nodes {
        let child_ids: Vec<String> = nodes
            .iter()
            .filter(|n| n.base().parent_id.as_deref() == Some(cid))
            .map(|n| n.base_id().to_string())
            .collect();
        if child_ids.is_empty() {
            continue;
        }
        let child_node_ids: std::collections::HashSet<&str> =
            child_ids.iter().map(|s| s.as_str()).collect();
        let sub_edges: Vec<WorkflowEdge> = edges
            .iter()
            .filter(|e| {
                child_node_ids.contains(e.source.as_str())
                    && child_node_ids.contains(e.target.as_str())
            })
            .cloned()
            .collect();
        let sub_nodes: Vec<WorkflowNode> =
            nodes.iter().filter(|n| child_node_ids.contains(n.base_id())).cloned().collect();
        let sub_graph = SubGraph {
            // 子图节点坐标必须相对于容器（Phase 3 编辑器将 subGraph 节点视为相对偏移，
            // 计算绝对坐标时叠加 container.position）。种子数据中的坐标是绝对坐标，
            // 因此在注入前转换为相对坐标。
            nodes: adjust_positions_to_relative(sub_nodes, cid, &nodes),
            edges: sub_edges,
        };
        // 注入到容器节点 config 中
        for n in nodes.iter_mut() {
            if n.base_id() != cid {
                continue;
            }
            match n {
                WorkflowNode::Parallel(p) => {
                    p.config.sub_graph = Some(sub_graph);
                },
                WorkflowNode::Debate(d) => {
                    d.config.sub_graph = Some(sub_graph);
                },
                _ => {},
            }
            break;
        }
    }
    // ── 运行时自证：模板引用的 agent_profile_id 必须都已注册 ──
    // 背景（2026-09-14）：节点 `decision-explainer` 的 profile（`stock-explainer`）曾在
    // `.md` / `EMBEDDED_PROMPTS` / `EXPERT_ROLE_MAP` 三处**全缺** —— profile 不会被建行，
    // agent_executor 解析 profile 得 None 后**只打一条 WARN 就跳过 expert 提示词**，
    // 节点照常 completed，属静默降级（无报错、无失败、只有提示词悄悄变弱）。
    // 此处在写库前把越界引用暴露到日志，让「模板声明了但没注册」这类断链在种子化阶段
    // 就能被发现。编译期的三表一致性由
    // `seed_consistency_tests::expert_registration_tables_are_consistent` 守住。
    {
        let known: std::collections::HashSet<String> = super::EXPERT_ROLE_MAP
            .iter()
            .map(|(expert_id, _)| format!("stock-{expert_id}"))
            .collect();
        let mut unknown: Vec<(&str, &str)> = Vec::new();
        for n in &nodes {
            if let WorkflowNode::Agent(a) = n {
                if let Some(pid) = a.config.agent_profile_id.as_deref() {
                    if pid.starts_with("stock-") && !known.contains(pid) {
                        unknown.push((n.base_id(), pid));
                    }
                }
            }
        }
        if !unknown.is_empty() {
            tracing::warn!(
                template_id = TEMPLATE_ID,
                count = unknown.len(),
                "模板存在未注册的 agent_profile_id（对应 expert 提示词会被 agent_executor \
                 静默跳过，节点仍 completed）: {:?}",
                unknown
            );
        }
    }
    // 写入 DB
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化节点失败: {e}"))
    })?;
    // DEBUG: 验证前几个 Tool 节点的 type 字段
    for n in nodes.iter().take(5) {
        let json = serde_json::to_string(n).unwrap_or_default();
        let preview = if json.len() > 200 {
            &json[..200]
        } else {
            &json
        };
        tracing::info!(node_id = %n.base_id(), json_preview = %preview, "seed_node_type");
    }
    let edges_json = serde_json::to_string(&edges).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化边失败: {e}"))
    })?;
    let tags = serde_json::to_string(&["stock", "analysis", "A股"]).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化标签失败: {e}"))
    })?;

    // 先删再插，避免 SeaORM .save() 对已存在记录的 update 失败
    let _ = workflow_template::Entity::delete_by_id(TEMPLATE_ID).exec(db).await;

    // P0 软门禁（C1，2026-09-14）：种子的端口公理 —— 结构性死链在此被记录（不阻断启动）。
    // 判据复用 harness 的 `warn_port_axioms_json`，不在本文件另写一份。
    axagent_harness::workflow_port_axioms::warn_port_axioms_json(
        &format!("stock_analysis_setup:seed_stock_analysis:{TEMPLATE_ID}"),
        &nodes_json,
        &edges_json,
    );

    workflow_template::ActiveModel {
        hooks_config: Set(Some(
            // 生命周期钩子声明（v5）：precheck/enhance 由引擎在 DAG 主循环前调用，
            // persist 在终态后调用。见 stock_workflow/hooks.rs。
            serde_json::to_string(&serde_json::json!({
                "pre_exec": ["stock-analysis-precheck", "stock-analysis-enhance"],
                "post_exec": ["stock-analysis-persist"],
            }))
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("序列化 hooks_config 失败: {e}"))
            })?,
        )),
        id: Set(TEMPLATE_ID.to_string()),
        cluster_id: Set(Some("equity".to_string())),
        route_path: Set(Some("/finance/equity/multi-dim-analysis".to_string())),
        name: Set("A股多维度分析".to_string()),
        description: Set(Some(
            "10 维度分析师 → LLM 智能辩论 → 价值投资（巴菲特框架）→ 3 风险维度 → Rhai 评分 → 交易方案 → 投资决策"
                .to_string(),
        )),
        icon: Set("chart-bar".into()),
        tags: Set(Some(tags)),
        version: Set(TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(Some(
            serde_json::to_string(&TriggerConfig {
                trigger_type: TriggerType::Schedule,
                config: serde_json::json!({
                    // 标准单 cron 字段，供 ScheduleTriggerConfig 反序列化使用
                    "cron": "0 9 * * 1-5",
                    // 多时段 map，供独立调度器（start_stock_pipeline）使用
                    "schedules": {
                        "morning": "0 9 * * 1-5",
                        "afternoon": "0 14 * * 1-5",
                    },
                    "enabled": true,
                    "timezone": "Asia/Shanghai",
                }),
            })
            .unwrap_or_default(),
        )),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(Some(input_schema_val)),
        output_schema: Set(Some(output_schema_val)),
        // ⚠️ 2026-09-22（D11）：此处原有「再用 `old_variables` 逐名把 `value` 覆盖回旧值」的
        // 分支，已删除 —— 它与上游 `merge_variable_values`（:4641 调用）**语义完全重复**
        // （同为「逐名保留旧值」），且位于 DCF force **之后**，会把 force 刚写的新值抹掉
        // （这正是 D11 的成因）。逐情形对账证明删除后行为等价、且不丢失任何能力
        // （`merge` 还多带 RENAME_MAP 别名迁移）。详见
        // `AUDIT-300642-run-variance-2026-09-22.md` §13.10.6。
        variables: Set(Some(variables_val)),
        error_config: Set(Some(error_config_val)),
        composite_source: Set(None),
        tool_defs: Set(Some(tool_defs_val)),
        mission_hash: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("写入工作流模板失败: {e}")))?;

    tracing::info!(
        "[stock_analysis_setup] 股票分析工作流模板已种子化完成: TEMPLATE_ID={TEMPLATE_ID}, VERSION={TEMPLATE_VERSION}"
    );
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════════════
// 快速分析链（`stock-analysis-fast`）种子 —— 从 `stock-analysis` **派生**，不重抄节点字面量
// ══════════════════════════════════════════════════════════════════════════════════════
//
// 设计依据：`PLAN-stock-analysis-fast-workflow.md`（2026-09-24 已批准）。三条硬约束：
//   H1 不动 `stock-analysis` 的图 —— 本段代码只**读**那一行，其节点 / 边 / 配置一行不改；
//   H2 两链共用同一份 .rhai 脚本 —— 本文件是**节点配置层**，两链差异只允许落在这里；
//   H3 落库 / 前端读取与原链一致 —— 故落库三节点沿用同名 id（`quality-fallback` >
//      `portfolio-risk-gate` > `portfolio-mgr`，见 `stock_workflow/decision.rs`）。
//
// ## 为什么是「派生」而不是「再手抄一份节点」
//
// 快速链的段 A（取数 / 算法 / 聚合 / 简报）与段 E（落库契约）在节点配置上与原链**逐字相同**，
// 差别只在「段 B/C/D 换成 Jev 分类节点」与「中间的 23 个 agent 节点不建」。
// 若在此重抄 `t-scoring` / `portfolio-mgr` 等节点的字面量，就出现**双份定义**：
// 原链改一处（例如给某数据工具补一个 input_mapping）快速链必然静默漂移 ——
// 这类漂移不报错、只在运行期表现为「某个变量恒缺」，正是 `AGENTS.md` 铁律 12 要拦的形态。
// ⇒ 本段改为**从已落库的 `stock-analysis` 行派生**：按 id 保留子集 → 边裁剪 → 孤儿修复
//   （规则见 `repair_orphan_nodes`）。源图一变，派生结果自动跟着变，无需人工同步任何常量。
//
// ## 为什么门禁是「内容比对」而不是「版本号比较」
//
// 兄弟种子用 `existing.version >= TEMPLATE_VERSION` 作门，前提是「模板内容由常量定义」。
// 本模板的内容由**派生源**定义 ⇒ 版本门会引入一个必须人工维护的同步项
// （源升到 v81 而快速链忘了抬版本号 ⇒ 快速链静默停在旧图，且无从察觉）。
// 内容比对没有这个必填项：派生结果与库中值不一致就重建，一致就跳过。
// 代价是**编辑器里对快速链的手工改动会在下次启动被覆盖** —— 这是本模板作为
// 「代码定义的派生资产」的既定取舍（原链同理，只是它的覆盖点是升版那一刻）。

/// 派生源模板 id。
///
/// 单一事实来源：`seed_stock_analysis_workflow_template` 内的 `TEMPLATE_ID` 也引用本常量
/// （两处字面量「stock-analysis」曾各写一份，改一处忘一处会直接让派生读错源）。
pub(crate) const SOURCE_TEMPLATE_ID: &str = "stock-analysis";

/// 快速链模板 id（前端 `src/components/stock-analysis/StockSearchBar.tsx` 的
/// `FAST_TEMPLATE_ID` 与此同值；两处都是模板的入口契约）。
pub(crate) const FAST_TEMPLATE_ID: &str = "stock-analysis-fast";

/// 快速链模板写在 `workflow_templates.version` 上的版本号。
///
/// ⚠ 它**不是**重建判据（判据是内容比对，理由见本段开头）。保留该列是因为编辑器的
/// 版本快照、前端展示与 `update_workflow_template` 都读它；取 1 = 「派生资产第 1 版」。
const FAST_TEMPLATE_VERSION: i32 = 1;

/// 派生时**必须**存在于源图的节点 id —— 缺失即报错，不允许静默少一个节点。
///
/// 判据：快速链段 A（取数 / 算法 / 聚合 / 简报）+ 段 E（落库契约）的全部节点。
/// 源图删改这些 id 时编译不会报错，只能靠这道运行期门把「派生结果悄悄缺一块」变成显式失败。
const FAST_REQUIRED_NODE_IDS: &[&str] = &[
    // 触发器
    "trigger",
    // 段 A · 数据源：10 个维度数据 + 质押 + 指数行情 + 机构调研 + 龙虎榜
    "t-market-data",
    "t-sentiment-data",
    "t-news-data",
    "t-fundamentals-data",
    "t-policy-data",
    "t-hotmoney-data",
    "t-lockup-data",
    "t-research-data",
    "t-sector-data",
    "t-catalyst-data",
    "t-pledge-data",
    "t-index-quotes",
    "t-institutional-visits",
    "t-dragon-tiger-data",
    // 段 A · 算法腿（含依赖顺序：scoring → valuation → band / risk / week → month）
    "t-scoring",
    "t-valuation",
    "t-valuation-band",
    "t-risk",
    "t-scoring-week",
    "t-scoring-month",
    // 段 A · 聚合与简报
    "raw-data",
    "analyst-brief",
    "data-quality",
    // 段 E · 决策与落库契约
    "portfolio-mgr",
    "portfolio-risk-gate",
    "rule-check",
    "quality-gate",
    "quality-fallback",
    FAST_EXPLAINER_NODE_ID,
    "notify-result",
    "store-result",
    "end-output",
];

/// 派生时「源图有就一并保留、没有也不报错」的节点 id。
///
/// 这两个是 portfolio-mgr 的因子侧输入（f11 PACE / 因子权重调节），失败本身
/// `continue_on_fail = true`，故快速链缺它们不会断链 —— 但也正因为如此，
/// 它们的缺失会静默发生 ⇒ 归入本清单而不是 `FAST_REQUIRED_NODE_IDS`，
/// 由 `validate_fast_workflow_graph` 保证「保留了就必须接上边」。
const FAST_OPTIONAL_NODE_IDS: &[&str] = &["regime-weights", "pace-calc"];

/// 快速链的「交易结论」组装节点 id（`output_var` 是 `trader`，与原链同名变量对齐）。
///
/// 该节点**不在源图里**，由 [`apply_fast_chain_overrides`] 直接构造 ⇒ 不进
/// `FAST_REQUIRED_NODE_IDS`（那份清单的语义是「源图必须具备」）。
const FAST_TRADER_PROXY_NODE_ID: &str = "trader-proxy";

/// 原链 `trader` 变量的取值前缀（AgentNode 输出：`{report, verdict:{...}}`）。
const TRADER_AGENT_PREFIX: &str = "trader.content.verdict.";

/// 快速链 `trader` 变量的取值前缀 —— 多一层 `result`，因为快速链的 `trader` 由
/// [`FAST_TRADER_PROXY_NODE_ID`] 这个 **Code 节点**产出，而 engine 会把 Code 节点的
/// 返回值包进 `result`（`{status, language, result, input_params, node_id, params}`）。
const TRADER_FAST_PREFIX: &str = "trader.result.content.verdict.";

/// 快速链 `quality-gate` 的判据变量：`data-quality` 的因子完整度（0-100 口径）。
///
/// 不用 `data-quality.result.grade` 的理由见 [`apply_fast_chain_overrides`] 的文档。
const FAST_QUALITY_GATE_INPUT_VAR: &str = "data-quality.result.factor_completeness_pct";

/// 快速链 `quality-gate` 的 acceptable 表达式（`value` 由 `SwitchExecutor` 注入）。
///
/// 阈值 60 = 9 项因子里至少 6 项可测。快速链实测可测 7 项（见上方文档）⇒ 77.8 通过；
/// 若上游再失败 2 项以上（5/9 = 55.6）则走保守降级，与门禁本意一致。
const FAST_QUALITY_GATE_CASE_EXPR: &str = "value >= 60.0";

/// 快速链**复用**的简报节点 id（node id 与 `output_var` 都保持源图原名）。
///
/// 复用而非新建 id 的理由见 [`apply_fast_chain_overrides`] 的 ④：段 B 的 `j-*` 判定节点
/// 与段 F 的引用都写 `analyst-brief.result.*`，换 id 会让这些引用一起漂移。
const FAST_BRIEF_NODE_ID: &str = "analyst-brief";

/// 段 F 的解释节点 id（沿用源图同名节点，快速链只改它的 `context_sources`）。
///
/// 单点定义的理由：它同时出现在保留集 [`FAST_REQUIRED_NODE_IDS`] 与 ⑥ 步的查找里，
/// 写成字面量两处会各自漂移（改一处漏一处 ⇒ ⑥ 步报「缺少节点」或静默不生效）。
const FAST_EXPLAINER_NODE_ID: &str = "decision-explainer";

/// 派生图 `analyst-brief` 节点的输入映射（快速链专属：14 个数据源 + 6 条算法腿）。
///
/// 值一律取 `<节点>.result.content`：ToolNode 的 `content` 是 JSON **字符串**，而
/// `resolve_var_path` 终值不 auto-parse（见 `executors/mod.rs` 的同名注释）⇒
/// `raw-digest.rhai` 内统一 `load()` 解析。这与 `data-quality` 的同类映射同口径。
const FAST_BRIEF_INPUTS: [(&str, &str); 20] = [
    ("market_data", "t-market-data.result.content"),
    ("sentiment_data", "t-sentiment-data.result.content"),
    ("news_data", "t-news-data.result.content"),
    ("fundamentals_data", "t-fundamentals-data.result.content"),
    ("policy_data", "t-policy-data.result.content"),
    ("hotmoney_data", "t-hotmoney-data.result.content"),
    ("lockup_data", "t-lockup-data.result.content"),
    ("research_data", "t-research-data.result.content"),
    ("sector_data", "t-sector-data.result.content"),
    ("catalyst_data", "t-catalyst-data.result.content"),
    ("pledge_data", "t-pledge-data.result.content"),
    ("index_quotes", "t-index-quotes.result.content"),
    ("institutional_visits", "t-institutional-visits.result.content"),
    ("dragon_tiger_data", "t-dragon-tiger-data.result.content"),
    ("algo_scoring", "t-scoring.result.content"),
    ("algo_valuation", "t-valuation.result.content"),
    ("algo_valuation_band", "t-valuation-band.result.content"),
    ("algo_risk", "t-risk.result.content"),
    ("algo_scoring_week", "t-scoring-week.result.content"),
    ("algo_scoring_month", "t-scoring-month.result.content"),
];

/// 快速链下 `data-quality` 的**逐维度输入重指向**表：`(诊断缩写, Jev 判定节点 id, 简报段键)`。
///
/// ## 为什么必须重指向
///
/// 源图 `data-quality` 的 50 路 `input_mapping` 里有 41 路引用那 10 个 `a-*` 分析师节点
/// （10 路 `{abbr}_verdict` + 10 路 `{abbr}_report` + 10 路 `{abbr}_untrusted` +
/// 10 路 `{abbr}_tool_calls` + 1 路 `catalyst_level`），而快速链**不建**这 10 个 Agent
/// 节点（H1：原图不动；本链只保留非 agent 资产）⇒ 这 41 路在派生图里全部解析为 Null：
///   · `*_verdict` 全 Null ⇒ `extract_conf` 逐路返回 `-1.0` ⇒ `gap_count = 10`
///     ⇒ `tool_credibility = clamp(30 - 40, 0, 100) = 0`；
///   · `*_report` 全空 ⇒ `report_quality_avg = 0`；
///   ⇒ `score ≈ 0.35×0 + 0.35×0 + 0.30×factor_completeness ≈ 23` ⇒ **恒 F**，
///     且 10 条诊断全 `missing`（数据质量弹窗所见症状）。
///
/// ## 重指向到哪
///
/// 快速链**复用**同一个 `data-quality` 节点与同一份 `data-quality.rhai`（非 agent 资源
/// 不另造、不分支），差异只落在本表声明的配置层 —— 分析师的两类产物在本链**仍然存在**，
/// 只是换了产地：
///   · `{abbr}_verdict` ← `j-<维度>` 　　　　　　= 该维度的 **Jev 判定（结果输出）**；
///   · `{abbr}_report`  ← `analyst-brief.result.<段键>` = 该维度的 **数据获取摘要**；
///   · `{abbr}_untrusted` ← `j-<维度>.degraded`　　　= 判定器降级标记（形态同原链的
///     `a-*.__untrusted`：`true` 表示「这条不是真判定」）⇒ 让 LLM 调用失败降级为兜底档
///     的维度**计入 gap**，而不是被当成有效判定抬高 grade；
///   · `{abbr}_tool_calls` ← `j-<维度>.tool_calls_made` = 形态对齐（判定器不产工具调用
///     记录 ⇒ 恒 Null ⇒ `attribution_note` 守卫①「无可核对数据」放行，不误判编造）。
///
/// ⚠ **这 4 类键一个都不能删**：`code_executor` 只注入 `input_mapping` 里声明过的键，
///   删键会让 `data-quality.rhai` 在 `present(mk_untrusted)` 处直接抛
///   "Variable not found"（节点整体失败，比读到 Null 更糟）⇒ 只能改指向、不能删键。
///
/// ⚠ `catalyst_level` 的处置见 [`apply_fast_chain_overrides`] ⑦ —— 它必须指向**字符串**
///   （`missing_factors` 里有 `catalyst_level == ""` 的字符串比较，map 参与比较会抛错）。
const FAST_DQ_DIMENSIONS: [(&str, &str, &str); 10] = [
    ("mk", "j-market", "market"),
    ("sent", "j-sentiment", "sentiment"),
    ("news", "j-news", "news"),
    ("fund", "j-fundamentals", "fundamentals"),
    ("pol", "j-policy", "policy"),
    ("hm", "j-hotmoney", "hotmoney"),
    ("lk", "j-lockup", "lockup"),
    ("res", "j-research", "research"),
    ("sec", "j-sector", "sector"),
    ("cat", "j-catalyst", "catalyst"),
];

// ════════════════════════════════════════════════════════════════════════════════════
// 段 B/C/D/E —— Jev 判定节点（`llmClassifier`）的单点定义
// ════════════════════════════════════════════════════════════════════════════════════

/// 段 B 的 10 个**维度判定**：`(节点 id, 标题, 简报段键, 维度名, 判据, 类别, 兜底档)`。
///
/// `简报段键` 就是 `raw-digest.rhai` 的输出键 ⇒ `input_var` 拼成
/// `analyst-brief.result.<键>`：**逐维度只喂自己那一段**。
///
/// ⚠ 这是对 `PLAN-stock-analysis-fast-workflow.md` §2 的**有意收窄**：PLAN 写的是
///   10 个节点共用 `input_var = analyst-brief.result`（整包）。整包 ≈16k 字符，
///   10 个节点各喂一遍既费 token 又稀释注意力 —— 而 F1 登记的正是「长 state 精度劣化」。
///   收窄后单节点的 state 只有几百字符，且「哪一段喂哪个维度」在图上一眼可查。
///   段 B 的 3 个**汇总**节点才需要跨维度信息，它们读聚合器 `jev_verdicts`
///   （粒度是判定结论而非原文），同样不需要整包。
///
/// 前 6 个是**指标类**（`raw-digest.rhai` 已把原始 JSON 渲染为「字段=值」短文本），
/// 后 4 个是**原文类**（新闻 / 政策 / 研报 / 公告原文，直接喂 LLM —— 这正是 F1 风险
/// 登记的对象，上线前须做影子比对）。
// 七元组逐位含义在下方各条目里注释：始终以字面元组声明（不抽 type 别名），
// 让声明处即可看到每列的类型 —— 与本仓既有先例同一手法（`dojo_sdk.rs:51` 等）。
#[allow(clippy::type_complexity)]
const FAST_JEV_DIMENSIONS: [(&str, &str, &str, &str, &str, [&str; 3], &str); 10] = [
    (
        "j-market",
        "市场量价判定",
        "market",
        "市场量价（K 线 / 成交量 / 换手 / 涨跌幅）",
        "- 支持：放量上行、站稳关键均线、缩量回调后回稳等明确偏多形态\n\
         - 反对：放量下跌、有效破位、连续阴线等明确偏空形态\n\
         - 中性：量价平淡、信号互相抵消，或数据不足以支撑方向",
        ["支持", "中性", "反对"],
        "中性",
    ),
    (
        "j-sentiment",
        "市场情绪判定",
        "sentiment",
        "市场情绪（舆情倾向 / 讨论热度 / 情绪指标）",
        "- 支持：舆情偏正面、关注度上升且以看多讨论为主\n\
         - 反对：舆情偏负面、恐慌情绪蔓延或质疑集中\n\
         - 中性：舆情平淡、正负相当，或数据不足以支撑方向",
        ["支持", "中性", "反对"],
        "中性",
    ),
    (
        "j-fundamentals",
        "基本面判定",
        "fundamentals",
        "基本面（盈利 / 成长 / 估值 / 商誉）",
        "- 支持：营收与利润增长、ROE 稳健、估值不贵\n\
         - 反对：业绩下滑、毛利承压、商誉或负债存在明显隐患\n\
         - 中性：基本面平稳无亮点，或数据不足以支撑方向",
        ["支持", "中性", "反对"],
        "中性",
    ),
    (
        "j-hotmoney",
        "资金流向判定",
        "hotmoney",
        "资金流向（主力净流入 / 大单 / 龙虎榜）",
        "- 支持：主力资金持续净流入、大单买盘占优\n\
         - 反对：主力资金持续净流出、大单卖盘占优\n\
         - 中性：资金进出均衡、金额很小，或数据不足以支撑方向",
        ["支持", "中性", "反对"],
        "中性",
    ),
    (
        "j-lockup",
        "解禁减持判定",
        "lockup",
        "解禁与减持（解禁时点 / 规模 / 减持计划）",
        "- 支持：近期无解禁压力，或解禁规模占比很小\n\
         - 反对：临近大比例解禁，或已披露明确的减持计划\n\
         - 中性：有解禁但规模与股价影响不明，或数据不足以支撑方向",
        ["支持", "中性", "反对"],
        "中性",
    ),
    (
        "j-sector",
        "行业板块判定",
        "sector",
        "行业与板块（行业排名 / 板块涨跌 / 同业比较）",
        "- 支持：所属行业排名靠前、板块资金与涨幅领先\n\
         - 反对：所属行业排名靠后、板块持续走弱\n\
         - 中性：行业表现平淡，或数据不足以支撑方向",
        ["支持", "中性", "反对"],
        "中性",
    ),
    (
        "j-news",
        "新闻面判定",
        "news",
        "新闻面（下述新闻原文）",
        "- 利好：出现明确的经营 / 订单 / 合作 / 业绩类正面消息\n\
         - 利空：出现明确的处罚 / 诉讼 / 亏损 / 事故类负面消息\n\
         - 中性：新闻与公司基本面无关，或正负消息相当",
        ["利好", "中性", "利空"],
        "中性",
    ),
    (
        "j-policy",
        "政策面判定",
        "policy",
        "政策面（下述政策原文）",
        "- 利好：政策明确扶持该行业（补贴 / 规划 / 准入放开）\n\
         - 利空：政策明确收紧该行业（限制 / 加税 / 监管趋严）\n\
         - 中性：政策与该公司所属行业无直接关系",
        ["利好", "中性", "利空"],
        "中性",
    ),
    (
        "j-research",
        "研报面判定",
        "research",
        "研报面（下述研报原文）",
        "- 利好：多数研报给出买入 / 增持评级，或上调盈利预测与目标价\n\
         - 利空：多数研报下调评级或目标价，提示明确风险\n\
         - 中性：研报观点分歧、评级中性，或仅作事实性跟踪",
        ["利好", "中性", "利空"],
        "中性",
    ),
    (
        "j-catalyst",
        "公告催化判定",
        "catalyst",
        "公告与催化（下述公告原文）",
        "- 利好：出现并购重组、重大订单、股权激励等正向催化\n\
         - 利空：出现减持、质押爆仓、监管问询等负向催化\n\
         - 中性：公告为常规事项（例会决议 / 例行披露），无实质催化",
        ["利好", "中性", "利空"],
        "中性",
    ),
];

/// 段 B 的 3 个**汇总判定**节点 id（入参是聚合器 `jev_verdicts` 的判定数组）。
const FAST_JEV_SUMMARY_NODE_IDS: [&str; 3] = ["j-direction", "j-conviction", "j-divergence"];

/// 段 C 的 4 个**对抗辩论**判定节点 id（入参同为 `jev_verdicts`）。
const FAST_JEV_DEBATE_NODE_IDS: [&str; 4] =
    ["j-bull-strength", "j-bear-strength", "j-winner", "j-disagreement"];

/// 段 E 的 3 个**决策契约**判定节点 id。
const FAST_JEV_DECISION_NODE_IDS: [&str; 3] = ["j-confidence", "j-target", "j-stop"];

/// `trader-proxy` 的 6 个 Jev 入参节点 id —— 每个都必须有一条 `→ trader-proxy` 的边。
///
/// ⚠ 只加 `input_mapping` 不加边是**静默失效**：DAG 只保证「有边才等」，
///   `trader-proxy` 会先于它们执行、读到 Null 并走 `present()` 降级分支
///   （方向恒中性、置信度缺失、档位不参与夹逼），全程零报错。
const FAST_JEV_TRADER_INPUTS: [&str; 6] =
    ["j-direction", "j-confidence", "j-conviction", "j-risk-level", "j-target", "j-stop"];

/// 段 B 维度判定的聚合器变量名：`input_sources` 装节点 id，`output_var` 与 id **同名**
/// （`AggregatorExecutor::collect_sources` 按 `context.variables.get(id)` 取值）。
///
/// ⚠ **必须无连字符**，这是硬要求而非风格：`LlmClassifierExecutor` 的 prompt 插值正则
///   是 `\{([a-zA-Z0-9_.]+)\}`（`llm_classifier_executor.rs:487`），**不含连字符**
///   ⇒ 段 D 的 prompt 里写 `{jev_verdicts.result}` 能替换，写 `{j-direction.category}`
///   会**原样留在 prompt 里**（不报错、静默失效）。
const FAST_JEV_VERDICTS_VAR: &str = "jev_verdicts";

/// 段 C 辩论判定的聚合器变量名（命名约束同 [`FAST_JEV_VERDICTS_VAR`]）。
const FAST_JEV_DEBATE_VAR: &str = "jev_debate";

/// Jev 判定节点的画布布局（只影响前端可读性，不参与调度）。
const JEV_LAYOUT_X0: f64 = 120.0;
const JEV_LAYOUT_Y0: f64 = 3000.0;
const JEV_LAYOUT_DX: f64 = 240.0;
const JEV_LAYOUT_DY: f64 = 130.0;
const JEV_LAYOUT_PER_ROW: usize = 5;

/// 一个 Jev 判定节点的单点定义。
struct JevNodeSpec {
    id: &'static str,
    title: &'static str,
    /// `LlmClassifierExecutor` 的**单一路径**入参。**不得留空** —— 留空会把
    /// `context.variables` 全部拼进 prompt（该执行器的 `input_var.is_empty()` 分支），
    /// 30 个节点的原始 JSON 直接撑爆 32k 上下文预算。
    input_var: String,
    categories: Vec<&'static str>,
    /// LLM 调用失败（或置信度低于阈值）时的降级档。
    fallback: &'static str,
    prompt: String,
    /// `Some(t)` ⇒ 执行器走 JSON 模式并在输出里带 `confidence`
    /// （详见 `seed_serenity_fast::classifier_node` 的文档）。
    confidence_threshold: Option<f64>,
    /// 显式补边的上游节点 id —— **必须**与 `input_var` 路径首段、prompt 内 `{...}`
    /// 占位符首段一致（理由见 [`FAST_JEV_TRADER_INPUTS`] 的 ⚠）。
    upstreams: Vec<&'static str>,
}

/// 段 B 维度判定的 prompt 组装（三段式：维度 → 判据 → 输出约束）。
///
/// 类别词表由 `categories` **派生**而非另写一遍 ⇒ 改类别时 prompt 自动跟随，
/// 不会出现「prompt 让模型输出 A、`categories` 里只有 B」的错配 —— 那种错配不报错，
/// 只会让执行器的 `matched` 落到「原样文本」回退分支，把自由文本塞进 `category`。
fn dimension_prompt(dimension: &str, criteria: &str, categories: &[&str]) -> String {
    format!(
        "你是 A 股判定器，**只**判定「{dimension}」这一个维度，不要综合其它维度、\
         不要给操作建议。\n\
         判据：\n{criteria}\n\
         只输出类别名称（{} 三选一），不要输出解释、标点或换行。",
        categories.join(" / ")
    )
}

/// 构造段 B/C/D/E 的全部 Jev 判定节点定义（顺序即画布布局顺序）。
///
/// 节点 id 一律取自上面那几个 `const` 数组（不在本函数里另写字面量）：
/// 聚合器的 `input_sources`、`trader-proxy` 的补边清单都按同一份 const 生成
/// ⇒ 「节点少了 / id 拼错」会在构造期直接对不上，而不是静默少一条依赖。
fn fast_jev_nodes() -> Vec<JevNodeSpec> {
    let mut specs: Vec<JevNodeSpec> = Vec::new();

    // ── 段 B：10 个维度判定（逐维度只喂自己那一段）──
    for (id, title, key, dimension, criteria, categories, fallback) in FAST_JEV_DIMENSIONS {
        specs.push(JevNodeSpec {
            id,
            title,
            input_var: format!("{FAST_BRIEF_NODE_ID}.result.{key}"),
            categories: categories.to_vec(),
            fallback,
            prompt: dimension_prompt(dimension, criteria, &categories),
            confidence_threshold: None,
            upstreams: vec![FAST_BRIEF_NODE_ID],
        });
    }

    // ── 段 B 汇总：3 个跨维度判定（入参 = 聚合器 `jev_verdicts` 的判定数组）──
    let [j_direction, j_conviction, j_divergence] = FAST_JEV_SUMMARY_NODE_IDS;
    let verdicts_in = format!("{FAST_JEV_VERDICTS_VAR}.result");
    specs.push(JevNodeSpec {
        id: j_direction,
        title: "方向结论判定",
        input_var: verdicts_in.clone(),
        categories: vec!["看多", "中性", "看空"],
        fallback: "中性",
        prompt: "你是 A 股方向判定器。输入文本是 10 个独立维度的判定结果数组\
                 （每项含 category 与 node_id）。\n\
                 请综合它们给出一句话方向结论：\n\
                 - 看多：多数维度偏多（支持 / 利好），且没有明确的反对项\n\
                 - 看空：多数维度偏空（反对 / 利空），且没有明确的支持项\n\
                 - 中性：多空相当、维度间分歧明显，或有效判定不足 3 项\n\
                 注意「反对 / 利空」与「支持 / 利好」**等权**，不要因为反对项数量少就忽略它。\n\
                 只输出类别名称（看多 / 中性 / 看空），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });
    specs.push(JevNodeSpec {
        id: j_conviction,
        title: "结论强度判定",
        input_var: verdicts_in.clone(),
        categories: vec!["强", "中", "弱"],
        fallback: "中",
        prompt: "你是 A 股结论强度判定器。输入文本是 10 个独立维度的判定结果数组。\n\
                 判断这些维度对**同一方向**的支撑有多强（中性维度不计入分母）：\n\
                 - 强：≥6 个维度指向同一方向，且反向维度 ≤1 个\n\
                 - 中：4~5 个维度指向同一方向，或方向一致但有效判定偏少\n\
                 - 弱：方向分散（多空各半），或大量维度为中性 / 缺失\n\
                 只输出类别名称（强 / 中 / 弱），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });
    specs.push(JevNodeSpec {
        id: j_divergence,
        title: "维度一致性判定",
        input_var: verdicts_in.clone(),
        categories: vec!["一致", "分歧", "严重分歧"],
        fallback: "分歧",
        prompt: "你是 A 股维度一致性判定器。输入文本是 10 个独立维度的判定结果数组。\n\
                 只看维度之间是否互相矛盾（中性维度不计入分母）：\n\
                 - 一致：反向维度占有效判定的比例 ≤20%\n\
                 - 分歧：反向维度占比在 20%~40% 之间\n\
                 - 严重分歧：反向维度占比 >40%，或同时出现强支持与强反对\n\
                 只输出类别名称（一致 / 分歧 / 严重分歧），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });

    // ── 段 C：4 个对抗辩论判定（同为跨维度判定，与段 B 汇总并发）──
    //
    // 对照原链的 `bull-r1` / `bear-r1` / `debate-convergence`：那里是「多方 Agent 与
    // 空方 Agent 各写一篇论述、再由收敛节点汇总」，本链改为**直接判定强弱**——
    // 省掉两轮生成，这也是「快速」的主要来源之一。
    let [j_bull, j_bear, j_winner, j_disagreement] = FAST_JEV_DEBATE_NODE_IDS;
    specs.push(JevNodeSpec {
        id: j_bull,
        title: "多方论据强度",
        input_var: verdicts_in.clone(),
        categories: vec!["强", "中", "弱"],
        fallback: "中",
        prompt: "你是 A 股多方论据强度判定器。输入文本是 10 个独立维度的判定结果数组。\n\
                 只评估**偏多**（支持 / 利好）那一侧的论据有多硬：\n\
                 - 强：≥4 个维度明确偏多，且覆盖量价 / 基本面 / 资金面中至少两类\n\
                 - 中：2~3 个维度明确偏多，或偏多维度集中在同一类\n\
                 - 弱：≤1 个维度偏多\n\
                 只输出类别名称（强 / 中 / 弱），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });
    specs.push(JevNodeSpec {
        id: j_bear,
        title: "空方论据强度",
        input_var: verdicts_in.clone(),
        categories: vec!["强", "中", "弱"],
        fallback: "中",
        prompt: "你是 A 股空方论据强度判定器。输入文本是 10 个独立维度的判定结果数组。\n\
                 只评估**偏空**（反对 / 利空）那一侧的论据有多硬：\n\
                 - 强：≥4 个维度明确偏空，且覆盖量价 / 基本面 / 资金面中至少两类\n\
                 - 中：2~3 个维度明确偏空，或偏空维度集中在同一类\n\
                 - 弱：≤1 个维度偏空\n\
                 只输出类别名称（强 / 中 / 弱），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });
    specs.push(JevNodeSpec {
        id: j_winner,
        title: "多空裁决",
        input_var: verdicts_in.clone(),
        categories: vec!["多方", "空方", "平局"],
        fallback: "平局",
        prompt: "你是 A 股多空对决裁判。输入文本是 10 个独立维度的判定结果数组。\n\
                 比较偏多与偏空两侧的论据：\n\
                 - 多方：偏多维度的数量与强度都明显占优\n\
                 - 空方：偏空维度的数量与强度都明显占优\n\
                 - 平局：两侧相当，或有效判定不足以分出胜负\n\
                 只输出类别名称（多方 / 空方 / 平局），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });
    specs.push(JevNodeSpec {
        id: j_disagreement,
        title: "分歧度判定",
        input_var: verdicts_in.clone(),
        categories: vec!["低", "中", "高"],
        fallback: "中",
        prompt: "你是 A 股分歧度判定器。输入文本是 10 个独立维度的判定结果数组。\n\
                 给出维度之间的分歧程度（下游用于置信度修正）：\n\
                 - 低：几乎所有有效维度同向\n\
                 - 中：存在 1~3 个反向维度\n\
                 - 高：反向维度 ≥4 个，或多空论据势均力敌\n\
                 只输出类别名称（低 / 中 / 高），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });

    // ── 段 D：9 个风险 / 估值判定 ──
    //
    // 前两个（`j-risk-level` / `j-risk-score`）是**跨维度**判定：入参是算法风险腿的
    // 指标文本，prompt 内再注入段 B / 段 C 的判定数组 ⇒ 三条上游边都要显式补。
    // 其余 7 个是**单维度**判定：入参就是 `raw-digest.rhai` 里对应那一段。
    let risk_indicators_in = format!("{FAST_BRIEF_NODE_ID}.result.algo_risk");
    let cross_inputs = vec![FAST_BRIEF_NODE_ID, FAST_JEV_VERDICTS_VAR, FAST_JEV_DEBATE_VAR];
    specs.push(JevNodeSpec {
        id: "j-risk-level",
        title: "风险等级判定",
        input_var: risk_indicators_in.clone(),
        categories: vec!["低", "中", "高", "极高"],
        // 兜底取「高」而不是「中」：风险档位的错判代价不对称（低估风险 ⇒ 仓位过大）。
        fallback: "高",
        prompt: format!(
            "你是 A 股风险等级判定器。输入文本是算法风险腿的指标输出（波动率 / 回撤 / Beta）。\n\
             同时参考维度判定与多空裁决：\n\
             维度判定：{{{FAST_JEV_VERDICTS_VAR}.result}}\n\
             多空裁决：{{{FAST_JEV_DEBATE_VAR}.result}}\n\
             判据：\n\
             - 低：波动率与回撤温和，且维度判定无偏空项\n\
             - 中：指标中性，或偏空维度不超过 2 个\n\
             - 高：高波动 / 深回撤，或偏空维度占多数\n\
             - 极高：指标处于极端区间（如年化波动率 >60% 或最大回撤 >40%），\
             或偏空维度压倒性多数\n\
             只输出类别名称（低 / 中 / 高 / 极高），不要输出解释。"
        ),
        confidence_threshold: None,
        upstreams: cross_inputs.clone(),
    });
    specs.push(JevNodeSpec {
        id: "j-risk-score",
        title: "风险强度判定",
        input_var: risk_indicators_in.clone(),
        categories: vec!["高", "中", "低"],
        fallback: "中",
        prompt: format!(
            "你是 A 股风险强度评分器。输入文本是算法风险腿的指标输出。\n\
             另参考：\n\
             维度判定：{{{FAST_JEV_VERDICTS_VAR}.result}}\n\
             多空裁决：{{{FAST_JEV_DEBATE_VAR}.result}}\n\
             给出风险强度的三档判断（与 `j-risk-level` 同源不同粒度，供交叉校验）：\n\
             - 高：风险指标处于近一年偏极端位置，或偏空维度占多数\n\
             - 中：风险指标与维度判定都落在中性区间\n\
             - 低：风险指标温和且维度判定偏多\n\
             只输出类别名称（高 / 中 / 低），不要输出解释。"
        ),
        confidence_threshold: None,
        upstreams: cross_inputs.clone(),
    });
    for (id, title, key, dimension, categories, fallback) in [
        (
            "j-pledge-risk",
            "质押风险判定",
            "pledge",
            "股东质押风险",
            ["是", "否", "数据不足"],
            "数据不足",
        ),
        (
            "j-lockup-risk",
            "解禁风险判定",
            "lockup",
            "限售解禁风险",
            ["是", "否", "数据不足"],
            "数据不足",
        ),
        (
            "j-goodwill-risk",
            "商誉风险判定",
            "fundamentals",
            "商誉减值风险",
            ["是", "否", "数据不足"],
            "数据不足",
        ),
    ] {
        specs.push(JevNodeSpec {
            id,
            title,
            input_var: format!("{FAST_BRIEF_NODE_ID}.result.{key}"),
            categories: categories.to_vec(),
            fallback,
            prompt: dimension_prompt(
                dimension,
                &format!(
                    "- 是：数据中出现明确的风险信号（{}）\n\
                     - 否：数据明确显示无该风险\n\
                     - 数据不足：输入文本无有效数据（如显示「（无数据）」）",
                    risk_criteria(id)
                ),
                &categories,
            ),
            confidence_threshold: None,
            upstreams: vec![FAST_BRIEF_NODE_ID],
        });
    }
    let valuation_in = format!("{FAST_BRIEF_NODE_ID}.result.algo_valuation");
    let band_in = format!("{FAST_BRIEF_NODE_ID}.result.algo_valuation_band");
    specs.push(JevNodeSpec {
        id: "j-valuation-signal",
        title: "估值信号判定",
        input_var: band_in,
        categories: vec!["低估", "合理", "高估", "无法估值"],
        fallback: "无法估值",
        prompt: "你是 A 股估值信号判定器。输入文本是估值分位算法腿的输出\
                 （形如 verdict = deep_value / undervalued / fair / expensive / overvalued / insufficient）。\n\
                 把算法结论翻译为四档：\n\
                 - 低估：verdict 为 deep_value 或 undervalued\n\
                 - 合理：verdict 为 fair\n\
                 - 高估：verdict 为 expensive 或 overvalued\n\
                 - 无法估值：verdict 为 insufficient，或输入文本无有效分位数据\n\
                 只输出类别名称（低估 / 合理 / 高估 / 无法估值），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_BRIEF_NODE_ID],
    });
    specs.push(JevNodeSpec {
        id: "j-margin-of-safety",
        title: "安全边际判定",
        input_var: valuation_in.clone(),
        categories: vec!["强", "中", "弱"],
        fallback: "中",
        prompt: "你是 A 股安全边际判定器。输入文本是估值算法腿的输出（DCF / 相对估值）。\n\
                 判断现价相对内在价值的安全边际：\n\
                 - 强：现价明显低于内在价值（上行空间大且估值依据可靠）\n\
                 - 中：现价与内在价值接近，或上行空间存在但依据一般\n\
                 - 弱：现价已高于内在价值，或估值依据不可用（无安全边际）\n\
                 只输出类别名称（强 / 中 / 弱），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_BRIEF_NODE_ID],
    });
    specs.push(JevNodeSpec {
        id: "j-moat",
        title: "护城河判定",
        input_var: valuation_in.clone(),
        categories: vec!["宽", "窄", "无"],
        fallback: "窄",
        prompt: "你是 A 股护城河判定器。输入文本是基本面与估值算法腿的输出。\n\
                 判断公司的竞争壁垒：\n\
                 - 宽：具备明确的定价权 / 高壁垒（垄断地位、高毛利且稳定、强品牌）\n\
                 - 窄：有一定壁垒但可被侵蚀\n\
                 - 无：同质化竞争、毛利承压，或输入数据不足以支持壁垒判断\n\
                 只输出类别名称（宽 / 窄 / 无），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_BRIEF_NODE_ID],
    });
    specs.push(JevNodeSpec {
        id: "j-dcf-evidence-usable",
        title: "DCF 证据可用性判定",
        input_var: valuation_in,
        categories: vec!["可用", "不可用"],
        fallback: "不可用",
        prompt: "你是 DCF 证据可用性判定器。输入文本是估值算法腿的输出，其中 dcf 段含\n\
                 assumptions（口径）与 applicable / is_fallback_anchor / fcf_data_missing 等标记。\n\
                 判断 DCF 结果能否作为决策依据：\n\
                 - 可用：DCF 有效（applicable 为真），且**不是**兜底锚\
                 （is_fallback_anchor 为假）、自由现金流数据未缺失（fcf_data_missing 为假）\n\
                 - 不可用：以上任一条件不满足，或输入文本无 DCF 段\n\
                 只输出类别名称（可用 / 不可用），不要输出解释。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_BRIEF_NODE_ID],
    });

    // ── 段 E：3 个决策契约判定 ──
    let [j_confidence, j_target, j_stop] = FAST_JEV_DECISION_NODE_IDS;
    specs.push(JevNodeSpec {
        id: j_confidence,
        title: "结论置信度判定",
        input_var: verdicts_in.clone(),
        categories: vec!["高", "中", "低"],
        fallback: "中",
        // Some(0.0)：要 `confidence` 数值，但不因阈值降级（见 classifier_node 文档）。
        // `trader-proxy` 读 `.confidence`，缺失时退到 `j-conviction` 的档位。
        confidence_threshold: Some(0.0),
        prompt: "你是 A 股结论置信度评估器。输入文本是 10 个独立维度的判定结果数组。\n\
                 评估「对这只标的的整体结论」的置信度（不是单个维度）：\n\
                 - 高：有效判定 ≥8 项且方向一致\n\
                 - 中：有效判定 4~7 项，或方向基本一致但有少量反向项\n\
                 - 低：有效判定 ≤3 项，或维度间分歧严重\n\
                 按 JSON 输出 label（高 / 中 / 低 三选一）与 confidence（0.0~1.0，\
                 表示你对该档位的把握）。"
            .to_string(),
        upstreams: vec![FAST_JEV_VERDICTS_VAR],
    });
    // ⚠ `categories` 的字面量必须与 `trader-proxy.rhai` 的 `tier_pct()` 词表**逐字一致**
    //   （`"5%"` / `"10%"` / `"20%"` / `"30%"`）：该函数只认这 4 个字符串，
    //   写 `"+5%"` 之类会静默返回 () ⇒ 档位既不参与夹逼、也不参与兜底换算。
    specs.push(JevNodeSpec {
        id: j_target,
        title: "目标涨幅档位判定",
        input_var: format!("{FAST_BRIEF_NODE_ID}.result.algo_valuation"),
        categories: vec!["5%", "10%", "20%", "30%"],
        fallback: "10%",
        prompt: "你是 A 股目标涨幅档位判定器。输入文本是估值算法腿的输出\
                 （DCF 上行空间 / 相对估值）。\n\
                 在四档中选出**该标的上行空间的上界**：\n\
                 - 5%：上行空间有限，估值已接近合理上限\n\
                 - 10%：存在温和上行空间\n\
                 - 20%：上行空间较大且有估值依据支撑\n\
                 - 30%：上行空间显著（低估明显且依据可靠）\n\
                 不确定时输出 10%。\n\
                 只输出档位（5% / 10% / 20% / 30%），不要输出解释或其它文字。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_BRIEF_NODE_ID],
    });
    specs.push(JevNodeSpec {
        id: j_stop,
        title: "止损幅度档位判定",
        input_var: risk_indicators_in,
        categories: vec!["5%", "10%", "20%", "30%"],
        fallback: "10%",
        prompt: "你是 A 股止损幅度档位判定器。输入文本是风险算法腿的输出（波动率 / 回撤）。\n\
                 在四档中选出**该标的应容忍的止损距离上界**：\n\
                 - 5%：波动温和，窄止损即可\n\
                 - 10%：波动中等\n\
                 - 20%：波动较大，需给足空间\n\
                 - 30%：波动剧烈（极端行情或高 Beta）\n\
                 不确定时输出 10%。\n\
                 只输出档位（5% / 10% / 20% / 30%），不要输出解释或其它文字。"
            .to_string(),
        confidence_threshold: None,
        upstreams: vec![FAST_BRIEF_NODE_ID],
    });

    specs
}

/// 三个单维度风险判定的「是」判据文案（`j-pledge-risk` / `j-lockup-risk` / `j-goodwill-risk`）。
///
/// 单列成函数而不是内联进循环：判据是**业务语义**（质押警戒线 / 解禁比例 / 商誉占比），
/// 与三个节点的 id 没有一一对应的机械关系，内联会让「哪段判据配哪个节点」不可复核。
fn risk_criteria(id: &str) -> &'static str {
    match id {
        "j-pledge-risk" => "控股股东质押比例偏高、接近平仓线，或近期质押公告密集",
        "j-lockup-risk" => "未来 3 个月内有较大比例解禁，或已披露减持计划",
        "j-goodwill-risk" => "商誉占净资产比例超过 30%，存在减值风险",
        _ => "数据中出现明确的风险信号",
    }
}

/// 构造一个 Jev 判定节点（`llmClassifier`），模型与画布坐标在此统一决定。
fn jev_classifier_node(
    spec: &JevNodeSpec,
    model: Option<&str>,
    index: usize,
) -> axagent_harness::workflow_types::WorkflowNode {
    super::seed_serenity_fast::classifier_node(
        spec.id,
        spec.title,
        &spec.prompt,
        spec.categories.iter().map(|c| (*c).to_string()).collect(),
        None,
        &spec.input_var,
        Some(spec.fallback),
        spec.confidence_threshold,
        model.map(str::to_string),
        JEV_LAYOUT_X0 + (index % JEV_LAYOUT_PER_ROW) as f64 * JEV_LAYOUT_DX,
        JEV_LAYOUT_Y0 + (index / JEV_LAYOUT_PER_ROW) as f64 * JEV_LAYOUT_DY,
    )
}

/// 构造一个 Jev 判定聚合器（`aggregator`）。
///
/// `strategy` 保持 `"all"`（输出 `result` 为各源值的**数组**）—— **不可**改成
/// `"merge"`：各判定节点的输出对象含同名字段（`category` / `model` / `node_id` …），
/// merge 会互相覆盖，只剩最后一项。
///
/// `output_var` 与节点 id **同名**：`AggregatorExecutor::collect_sources` 按
/// `context.variables.get(id)` 取源值，而每个 `j-*` 节点的 `output_var` 就是其 id
/// （见 `seed_serenity_fast::classifier_node`）—— 两处命名约定必须一致，否则聚合器
/// 收集到的是全 `Null` 数组（不报错）。
///
/// `continue_on_fail` 保持 `false`（与源图 `agg-risk` / `raw-data` 同）：上游 `j-*` 已
/// 配 `continue_on_fail: true` + `fallback_label`，聚合器本身没有可降级的语义。
fn jev_aggregator_node(
    id: &str,
    title: &str,
    description: &str,
    sources: Vec<String>,
    x: f64,
    y: f64,
) -> axagent_harness::workflow_types::WorkflowNode {
    use axagent_harness::workflow_types::{
        AggregatorNode, AggregatorNodeConfig, Position, RetryConfig, WorkflowNode, WorkflowNodeBase,
    };

    WorkflowNode::Aggregator(AggregatorNode {
        base: WorkflowNodeBase {
            id: id.into(),
            title: title.into(),
            description: Some(description.into()),
            position: Position { x, y },
            retry: RetryConfig::default(),
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: AggregatorNodeConfig {
            strategy: "all".into(),
            input_sources: sources,
            output_var: id.into(),
            wait_for_all: true,
            weights: vec![],
            summarize_prompt: None,
            summarize_model: None,
            sub_graph: None,
        },
    })
}

/// 派生结果（节点 + 边）。全部经不变式校验后才允许落库。
struct DerivedFastGraph {
    nodes: Vec<axagent_harness::workflow_types::WorkflowNode>,
    edges: Vec<axagent_harness::workflow_types::WorkflowEdge>,
}

/// 取变量路径的**首段**：`t-risk.result.content.grade` → `t-risk`；`trigger` → `trigger`。
///
/// 首段就是「这个值由哪个节点写入 `context.variables`」——节点的 `output_var` 与
/// `input_mapping` 的路径首段按同一约定书写（全模板一贯如此）。
fn mapping_root(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.split('.').next().unwrap_or(trimmed))
}

/// 节点**自己声明**读了哪些节点的产出（仅取 input_mapping / input_sources 的路径首段）。
fn node_declared_sources(node: &axagent_harness::workflow_types::WorkflowNode) -> Vec<String> {
    use axagent_harness::workflow_types::WorkflowNode;

    let mut roots: Vec<String> = Vec::new();
    let mut push_all = |values: Vec<&str>| {
        roots.extend(values.into_iter().filter_map(|v| mapping_root(v).map(str::to_string)));
    };
    match node {
        WorkflowNode::Tool(n) => {
            push_all(n.config.input_mapping.values().map(String::as_str).collect());
        },
        WorkflowNode::Code(n) => {
            push_all(n.config.input_mapping.values().map(String::as_str).collect());
        },
        WorkflowNode::Agent(n) => {
            push_all(n.config.input_mapping.values().map(String::as_str).collect());
        },
        // 聚合节点的 `input_sources` 装的直接就是上游节点 id（无路径），首段即自身。
        WorkflowNode::Aggregator(n) => {
            push_all(n.config.input_sources.iter().map(String::as_str).collect());
        },
        _ => {},
    }
    roots.sort();
    roots.dedup();
    roots
}

/// 构造一条 `Direct` 边（与源种子内 `edge(...)` 闭包同形）。
fn direct_edge(
    id: &str,
    source: &str,
    target: &str,
) -> axagent_harness::workflow_types::WorkflowEdge {
    use axagent_harness::workflow_types::{EdgeType, WorkflowEdge};

    WorkflowEdge {
        id: id.into(),
        source: source.into(),
        source_handle: None,
        target: target.into(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: None,
    }
}

/// 孤儿修复：给裁剪后**入度为 0** 的非 trigger 节点补入边。
///
/// 两种来源，按优先级：
///   1. 保留集内、该节点**自己声明读取**的节点（`node_declared_sources`）——
///      这是最准的判据：「我读谁的产出，我就等谁」；
///   2. 兜底挂 `trigger`：仅当它不读保留集内任何节点时适用，即「入参只来自全局变量
///      （`stock_code` 等）」的节点（数据工具、以 `stock_code` 为唯一入参的算法节点）。
///
/// 为什么不给所有孤儿一律挂 trigger：`analyst-brief` / `data-quality` 这类节点的入参是
/// **上游节点的产出**，挂 trigger 会让它们在数据未就绪时就跑完（DAG 只保证「有边才等」），
/// 结果是「跑得很快、全是空」——比缺节点更难查。故此类节点必须先尝试按声明依赖接线；
/// 声明不到保留集内任何节点时（说明其上游在本图已被整体裁掉）才退回 trigger，并打 WARN。
fn repair_orphan_nodes(
    nodes: &[axagent_harness::workflow_types::WorkflowNode],
    edges: &mut Vec<axagent_harness::workflow_types::WorkflowEdge>,
    keep: &std::collections::HashSet<&str>,
) {
    use axagent_harness::workflow_types::WorkflowNode;
    use std::collections::HashMap;

    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for e in edges.iter() {
        *in_degree.entry(e.target.as_str()).or_insert(0) += 1;
    }

    let mut added: Vec<axagent_harness::workflow_types::WorkflowEdge> = Vec::new();
    for node in nodes {
        if matches!(node, WorkflowNode::Trigger(_)) {
            continue;
        }
        let id = node.base_id();
        if in_degree.get(id).copied().unwrap_or(0) > 0 {
            continue;
        }
        let deps: Vec<String> = node_declared_sources(node)
            .into_iter()
            .filter(|d| d.as_str() != id && keep.contains(d.as_str()))
            .collect();
        if deps.is_empty() {
            tracing::warn!(
                "[stock_analysis_setup] 快速链派生：节点 `{id}` 裁剪后失去全部入边，且未声明读取保留集内任何节点 \
                 ⇒ 挂到 trigger 下（前提：它的入参只取全局变量）"
            );
            added.push(direct_edge(&format!("e-trigger-{id}"), "trigger", id));
        } else {
            for dep in deps {
                added.push(direct_edge(&format!("e-{dep}-{id}"), &dep, id));
            }
        }
    }
    edges.append(&mut added);
}

/// 快速链专属的**节点配置层**改造（H1：源图一行不改；H2：两链差异只允许落在配置层）。
///
/// 六件事：
///   ① 追加 `trader-proxy` 组装节点（Code，`output_var = "trader"`），并接线
///      `data-quality → trader-proxy → portfolio-mgr`；
///   ② 把派生图内**所有**指向 `trader.content.verdict.*` 的映射整体改前缀为
///      `trader.result.content.verdict.*`（详见下方）；
///   ③ 摘掉 `portfolio-mgr` 的 `dqi_score` / `dqi_grade`，并把 `quality-gate` 的判据
///      从 `data-quality.result.grade` 换成 `factor_completeness_pct`（详见下方）；
///   ④ 把复用 id 的 `analyst-brief` 节点换成快速链专属脚本 + 专属输入（详见下方）；
///   ⑤ 加入段 B/C/D/E 的全部 Jev 判定节点 + 两个聚合器，并**显式补全**它们之间的边
///      （详见下方）；
///   ⑥ 把段 B–E 的 Jev 输出与 `analyst-brief` 补进 `decision-explainer` 的
///      `context_sources`（**只改上下文、不产生边**，详见下方）。
///
/// ## ② 为什么必须改前缀
///
/// 原链的 `trader` 是 **AgentNode** 的输出（`{role, content: <json>, ...}`），
/// 映射写 `trader.content.verdict.*`；快速链的 `trader` 由 **CodeNode**
/// （[`FAST_TRADER_PROXY_NODE_ID`]）产出，engine 会再包一层 `result`
/// ⇒ 不改前缀则全部解析为 Null（f7 因子失效、`odds` 恒走 fallback、
/// `portfolio-risk-gate` 的 `target_price` 为空）。
///
/// ⚠ 受影响的**不止** `portfolio-mgr`：`portfolio-risk-gate` 也有
/// `("target_price", "trader.content.verdict.targetPrice")`。故此处按**前缀**在
/// 全图 Code 节点上统一改写，而不是硬编码某一两个节点的键名 —— 后者在源图新增
/// 一个消费 `trader` 的节点时会静默漏改。
/// 前缀之外的 `trader.*` 路径（如 `trader.__untrusted`）**刻意不改**：那些字段
/// 快速链本就拿不到（`trader-proxy` 不产出 `__untrusted`），改与不改都是 Null，
/// 改动只会制造「看起来对齐了」的错觉。
///
/// ## ③ 的必要性（快速链的阻断性缺口，2026-09-24 定位）
///
/// `data-quality.rhai` 的 `score` 有 35% 权重来自「分析师报告质量」、35% 来自
/// 「工具可信度」—— 两者的输入是 10 个分析师的 `*_verdict` / `*_report`。
/// 快速链**不建**这 10 个 Agent 节点（H1 + 本链的既定设计）⇒ 两项恒为 0
/// （`report_quality_avg = 0`；`tool_credibility` 走缺报告路径 base=30、gap 阶梯满档罚 40
/// ⇒ clamp 0）
/// ⇒ `score ≈ factor_completeness × 30 ≈ 23` ⇒ **恒判 grade F**。后果两条，都是阻断级：
///   a. `portfolio-mgr.rhai` 的 `dqi_collapsed = (dqi_grade_str == "F")` ⇒ 仓位钉死 0%；
///   b. `quality-gate` 的 `A/B/C` 判据恒不成立 ⇒ 恒走 `low-quality` ⇒ `quality-fallback`
///      （LLM 保守决策）—— 而落库优先级是 `quality-fallback` **最高**
///      （见 `stock_workflow/decision.rs`）⇒ Jev 判定链被整体绕开。
///
/// 处置（**纯配置层，两链脚本零改动**）：快速链不再消费 `data-quality` 的
/// `grade` / `score`（它们在本链测不到报告质量，取值没有意义），改用本链**真实可测**的
/// `factor_completeness_pct`（9 项因子里 7 项可测：`consensus_score` 与 `catalyst_level`
/// 的提供方 —— `debate-convergence` / `a-catalyst` —— 不在本链）。
/// 摘掉映射后 `portfolio-mgr.rhai` 的既有分支自然接管：
/// `dqi_grade_str = ""` ⇒ `confidence_quality_cap = 60.0`（该脚本注释明写
/// 「数据质量检查不可用 ⇒ 按最保守档」）、`dqi_collapsed = false`、
/// `f6_signal = 0` ⇒ `f6_weight = 0`（f6 退出权重）。
/// ⇒ 快速链的置信度上限被压到 60 —— 这是**诚实**的：本链确实没有分析师报告可核。
///
/// ## ④ 为什么 `analyst-brief` 要「换脚本、留 id」
///
/// 原链的 `analyst-brief` 是**分析师摘要**：入参是 10 个分析师的 `.content.verdict`，
/// 脚本（`analyst-brief.rhai`）逐份读 `bull_score` / `bear_score` 拼 Markdown。
/// 快速链**不建**这 10 个 Agent 节点（H1 + 本链既定设计）⇒ 该节点在本链的处境是：
///   a. 10 条入边全部被裁掉，`input_mapping` 全部解析为 `()` ⇒ 输出恒为十行
///      「数据不可用」；
///   b. 它的**唯一**消费者是辩论链（源图 `e-brief-debate` → `debate-bull-bear`），
///      而辩论链在本链不存在；`decision-explainer` 的 `context_sources` 也不含它
///      ⇒ 派生图里它**零消费者**，纯属「跑完就扔」。
///
/// 处置分两问，答案是分开的：
///   · **要不要留这个节点**：留。段 B 的 `j-*` 判定节点需要一个「把 14 路原始数据
///     压成可喂 LLM 的短文本」的上游，而「分维度裁剪」正是这个节点在本链的对应职能
///     —— 删掉它等于让每个 `j-*` 各自去啃原始 JSON。
///   · **用哪个脚本**：换。原脚本的输入契约是 verdict map（`bull_score`/`bear_score`），
///     只改 `input_mapping` 指向数据源**并不能**让它产出有用文本 —— 它仍然只认那两个
///     字段，输出依旧是十行「数据不可用」（这正是「方案 C 只改输入源」不自洽之处）。
///     故改用快速链专属脚本 `raw-digest.rhai`（新资产，不是原脚本的副本或版本分支：
///     输入契约、输出契约、消费者三者全不同，见该文件头部注释）。
///
/// **node id 与 `output_var` 保持 `analyst-brief` 不变**：段 B 的 `input_var` 与段 F 的
/// 引用都写 `analyst-brief.result.<段>`，换 id 会让这些引用一并漂移；而
/// `repair_orphan_nodes` 会按新 `input_mapping` 声明的 20 个上游自动补边
/// （故此处**不**手工加边，避免与孤儿修复重复）。
///
/// ## ⑤ 为什么 Jev 节点必须**手工**补边（不能指望孤儿修复）
///
/// 两个机制性原因，缺一即静默失效：
///   1. `node_declared_sources` **不识别** `LlmClassifier` 的 `input_var`
///      （该函数的 `_ => {}` 分支）⇒ 对 29 个 `j-*` 节点它一律返回空 ⇒ 孤儿修复只会
///      把它们挂到 `trigger` 下，DAG 立刻退化为「谁都不等谁」；
///   2. 孤儿修复的 `keep` 只含**源图** id，而 `jev_verdicts` / `jev_debate` 是新增节点
///      ⇒ 即使识别得到上游，也会因不在 `keep` 内而被过滤掉。
///
/// 故本步按 [`JevNodeSpec::upstreams`]（单点定义）+ 两个聚合器的源清单**显式补边**，
/// 并在测试里逐条断言「每条声明都有对应的边」—— 这正是 `FAST_JEV_TRADER_INPUTS`
/// 文档所警告的「只加映射不加边是静默失效」的机械防线。
///
/// 另注：`trader-proxy` 的 6 个 Jev 入参同样在此补边。只加 `input_mapping` 不加边时，
/// `trader-proxy` 会先于它们执行、读到 Null 并走 `present()` 降级分支（方向恒中性、
/// 档位不参与夹逼），全程零报错。
///
/// ## ⑥ 为什么 `decision-explainer` 补的是 `context_sources` 而不是边
///
/// 该节点是**源图既有**节点（`agent()` + `context_sources` + `input_mapping`），
/// 在快速链里保留原样。它的 `context_sources` 原本是
/// `[portfolio-risk-gate, rule-check, t-scoring, t-risk]` —— 四个来源在快速链**全部保留**
/// ⇒ 入度 ≥ 1，`repair_orphan_nodes` 根本不会看它（它只处理入度为 0 的节点）。
///
/// 问题不在「断链」而在「读不到判断依据」：原链的解释官复述的是 `portfolio-mgr` 的
/// 因子/风控裁决，而快速链的**判断主体**是段 B–E 的 Jev 判定。不补来源 ⇒ 解释文案
/// 只能罗列规则编号，无法回答「凭什么这么判」（H3 要求结论可解析，段 F 是唯一
/// 承载自然语言论据的节点）。
///
/// 补 `context_sources` 而**不补边**是刻意的：
///   · `context_sources` 只影响 prompt 拼装，不参与 DAG 依赖 —— 补边会改变调度语义
///     （且 `repair_orphan_nodes` 已在 ⑥ 之前跑完，此处的边只能手工加，反而多一处
///     与声明不同步的风险）；
///   · 本节点能读到这些值，靠的是**既有链路顺序**（段 B–E / `analyst-brief` →
///     `trader-proxy` → `portfolio-mgr` → `portfolio-risk-gate` → `quality-gate` →
///     本节点），而非本步声明的依赖 ⇒ 该前提若被改动，这里会静默读到 Null。
///     故本步不引入新边，把「顺序前提」写进注释与 ⑥ 步代码注释，供改动者看到。
///
/// ## ⑦ 为什么 `data-quality` 必须改输入、又必须**手工**补边
///
/// 该节点是**原样复用**的非 agent 资源（同一 id、同一份 `data-quality.rhai`、不另造脚本），
/// 但它的 50 路 `input_mapping` 里有 41 路指向那 10 个 `a-*` 分析师节点 —— 本链不建
/// 这些 Agent ⇒ 41 路全解析为 Null ⇒ `gap_count = 10`、`report_quality_avg = 0`
/// ⇒ grade **恒 F**、10 条诊断全 `missing`。这不是「快速链取数失败」，而是本节点没接到
/// 本链的真实产物。改法与其判据逐项写在本函数的 ⑦ 步与 [`FAST_DQ_DIMENSIONS`] 的文档里。
///
/// 边必须手工补的理由与 ⑤ 同源但不是同一个：`repair_orphan_nodes` 只处理**入度为 0** 的
/// 节点，而 `data-quality` 裁剪后仍有 7 条入边 ⇒ 修复器**根本不会看它**。
/// 只改映射不补边 ⇒ 它先于 `analyst-brief` / `j-*` 执行、读到全 Null，且全程零报错。
fn apply_fast_chain_overrides(
    nodes: &mut Vec<axagent_harness::workflow_types::WorkflowNode>,
    edges: &mut Vec<axagent_harness::workflow_types::WorkflowEdge>,
    decision_model: Option<&str>,
) -> Result<(), String> {
    use axagent_harness::workflow_types::{
        CodeNode, CodeNodeConfig, Position, RetryConfig, SwitchCase, WorkflowNode, WorkflowNodeBase,
    };

    // ── ① 追加组装节点 ──
    if nodes.iter().any(|n| n.base_id() == FAST_TRADER_PROXY_NODE_ID) {
        return Err(format!(
            "派生结果已含 `{FAST_TRADER_PROXY_NODE_ID}` —— 该 id 是快速链保留 id，\
             源图若确实新增了同名节点请先改名（否则两处定义会互相覆盖）"
        ));
    }
    nodes.push(WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            id: FAST_TRADER_PROXY_NODE_ID.into(),
            title: "交易结论组装（快速链）".into(),
            description: Some(
                "确定性重建与原链 `trader` 同形的结论：方向/置信度/风险档取自 Jev 判定，\
                 目标价/止损价由算法腿换算"
                    .into(),
            ),
            position: Position { x: 840.0, y: 3900.0 },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
            // 与 `portfolio-mgr` 同策略：本节点失败不应让整链停在 Pending
            // （下游 `present()` 守卫会把缺失降级为中性，见 trader-proxy.rhai 的输出契约）。
            continue_on_fail: true,
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: include_str!("../trader-proxy.rhai").to_string(),
            output_var: "trader".into(),
            tool_name: None,
            execute_directly: true,
            input_mapping: [
                // 算法腿。⚠ 本节点的入边只有 `data-quality` + `j-*`（见下），而快速链的
                //   `data-quality` 已不再声明依赖算法腿（它的 10 个分析师上游全被裁掉，
                //   孤儿修复把它挂到了 `trigger` 下）⇒ 算法腿的**就绪顺序由 `j-*` 传递保证**：
                //   6 个 `j-*` 入边全部（直连或经 `jev_verdicts`）依赖 `analyst-brief`，
                //   而 `analyst-brief` 的 `FAST_BRIEF_INPUTS` 含这 6 条算法腿。
                //   故**不可**删掉 `j-*` 到本节点的边 —— 那会让本节点提前执行、
                //   三条路径全解析为 Null，且全程零报错（静默失效）。
                ("current_price", "t-scoring.result.content.currentPrice"),
                ("dcf_upside", "t-valuation.result.content.dcf.upsidePct"),
                ("volatility", "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct"),
                // 逐维度诊断（证据清单来源之一；本链多为 missing，见脚本内的证据清单注释）
                ("dq_diagnostics", "data-quality.result.diagnostics"),
                // 段 B 的 10 个维度判定数组（证据清单来源之二：非中性判定即决策论据）。
                // ⚠ 聚合器 `jev_verdicts` 的 `output_var` 与节点 id 同名 ⇒ 变量名即该 id。
                ("j_dimensions", "jev_verdicts.result"),
                // Jev 判定（段 B/C/D 由本函数的 ⑤ 步创建并补边）
                ("j_direction", "j-direction.category"),
                ("j_confidence", "j-confidence.confidence"),
                ("j_conviction", "j-conviction.category"),
                ("j_risk_level", "j-risk-level.category"),
                ("j_target", "j-target.category"),
                ("j_stop", "j-stop.category"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        },
    }));
    // 边：`data-quality → trader-proxy → portfolio-mgr`。
    // ⚠ `j-*` / `jev_verdicts` 的入边**不在此处补** —— 那些节点由本函数的 ⑤ 步创建，
    //   接线也放在 ⑤ 步（创建与接线同处，避免两份清单各自漂移）。
    // `portfolio-risk-gate` 不必单独接线：它已有 `portfolio-mgr → 它` 的边，
    // 而 `portfolio-mgr` 等 `trader-proxy` ⇒ 传递保证 target_price 就绪。
    edges.push(direct_edge(
        &format!("e-data-quality-{FAST_TRADER_PROXY_NODE_ID}"),
        "data-quality",
        FAST_TRADER_PROXY_NODE_ID,
    ));
    edges.push(direct_edge(
        &format!("e-{FAST_TRADER_PROXY_NODE_ID}-portfolio-mgr"),
        FAST_TRADER_PROXY_NODE_ID,
        "portfolio-mgr",
    ));

    // ── ② + ③ 逐节点改写 ──
    let mut rewritten = 0usize;
    let mut dropped_dqi: Vec<String> = Vec::new();
    for node in nodes.iter_mut() {
        let WorkflowNode::Code(code) = node else { continue };
        let is_portfolio_mgr = code.base.id == "portfolio-mgr";
        code.config.input_mapping.retain(|key, value| {
            // ③ 只摘 `portfolio-mgr` 的 dqi 映射：`quality-gate` 走的是自己的 input_var，
            //    其他节点（如 `portfolio-risk-gate`）本就不读这两项。
            if is_portfolio_mgr && (key == "dqi_score" || key == "dqi_grade") {
                dropped_dqi.push(key.clone());
                return false;
            }
            // ② 前缀改写
            if let Some(rest) = value.strip_prefix(TRADER_AGENT_PREFIX) {
                *value = format!("{TRADER_FAST_PREFIX}{rest}");
                rewritten += 1;
            }
            true
        });
    }
    if dropped_dqi.is_empty() {
        return Err("派生出的 `portfolio-mgr` 未含预期的 `dqi_score` / `dqi_grade` 映射 —— \
             源图的该节点已改，请核对后再决定快速链的 data-quality 处置（见本函数文档 ③）"
            .to_string());
    }
    if rewritten == 0 {
        return Err("派生图内没有任何 `trader.content.verdict.*` 映射被改写 —— \
             源图已改（`portfolio-mgr` / `portfolio-risk-gate` 应各有一条以上），\
             快速链的 trader 通路会整体失效"
            .to_string());
    }

    // ── ③ quality-gate 判据 ──
    let qg = nodes
        .iter_mut()
        .find(|n| n.base_id() == "quality-gate")
        .ok_or_else(|| "派生结果缺少 `quality-gate` 节点".to_string())?;
    let WorkflowNode::Switch(sw) = qg else {
        return Err("`quality-gate` 在源图里不是 Switch 节点 —— 派生假设已失效".to_string());
    };
    sw.config.input_var = FAST_QUALITY_GATE_INPUT_VAR.to_string();
    sw.config.cases = vec![SwitchCase {
        value: FAST_QUALITY_GATE_CASE_EXPR.to_string(),
        label: "acceptable".to_string(),
    }];

    // ── ④ `analyst-brief`：换脚本 + 换输入（id / output_var 不变，理由见函数文档 ④）──
    //
    // ⚠ 顺序要求：本步必须在 `repair_orphan_nodes` **之前** —— 孤儿修复读的是节点的
    //   `input_mapping` 声明，此处换完映射它才能把 20 个上游正确接上；若反过来，
    //   它只会看到原链那 10 个已被裁掉的 Agent 上游，从而把本节点挂到 `trigger` 下。
    let brief = nodes
        .iter_mut()
        .find(|n| n.base_id() == FAST_BRIEF_NODE_ID)
        .ok_or_else(|| format!("派生结果缺少 `{FAST_BRIEF_NODE_ID}` 节点"))?;
    let WorkflowNode::Code(brief) = brief else {
        return Err(format!("`{FAST_BRIEF_NODE_ID}` 在源图里不是 Code 节点 —— 派生假设已失效"));
    };
    if brief.config.output_var != FAST_BRIEF_NODE_ID {
        return Err(format!(
            "`{FAST_BRIEF_NODE_ID}` 的 output_var 已改为 `{}` —— 段 B/段 F 的引用都写 \
             `{FAST_BRIEF_NODE_ID}.result.*`，此处必须同名",
            brief.config.output_var
        ));
    }
    brief.base.title = "原始数据分维度摘要（快速链）".into();
    brief.base.description = Some(
        "把 14 路原始数据与 6 条算法腿的产出按维度裁剪为短文本段，供 Jev 判定节点消费；\
         每段独立降级，缺数据只显示「（无数据）」"
            .into(),
    );
    brief.config.code = include_str!("../raw-digest.rhai").to_string();
    brief.config.input_mapping =
        FAST_BRIEF_INPUTS.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();

    // ── ⑤ 段 B/C/D/E：Jev 判定节点 + 两个聚合器 + 显式边 ──
    let specs = fast_jev_nodes();
    for (index, spec) in specs.iter().enumerate() {
        if nodes.iter().any(|n| n.base_id() == spec.id) {
            return Err(format!(
                "派生结果已含 `{}` —— `j-*` 是快速链保留前缀，\
                 源图若确实新增了同名节点请先改名（否则两处定义会互相覆盖）",
                spec.id
            ));
        }
        nodes.push(jev_classifier_node(spec, decision_model, index));
        for upstream in &spec.upstreams {
            edges.push(direct_edge(&format!("e-{upstream}-{}", spec.id), upstream, spec.id));
        }
    }

    // 两个聚合器把「多节点判定」收敛成**单个数组变量**：`LlmClassifierExecutor` 的
    // `input_var` 只接受单一路径，没有聚合器就只能把 10 个判定各自写一遍 prompt 占位符
    // （而占位符正则不含连字符，`{j-market.category}` 这类写法根本不替换）。
    let agg_row = specs.len().div_ceil(JEV_LAYOUT_PER_ROW);
    let verdict_sources: Vec<String> =
        FAST_JEV_DIMENSIONS.iter().map(|(id, ..)| (*id).to_string()).collect();
    nodes.push(jev_aggregator_node(
        FAST_JEV_VERDICTS_VAR,
        "维度判定汇总",
        "把 10 个维度判定的输出收敛为数组，供段 B 汇总 / 段 C 辩论 / 段 E 置信度与 \
         trader-proxy 的证据清单读取",
        verdict_sources.clone(),
        JEV_LAYOUT_X0,
        JEV_LAYOUT_Y0 + agg_row as f64 * JEV_LAYOUT_DY,
    ));
    for id in &verdict_sources {
        edges.push(direct_edge(
            &format!("e-{id}-{FAST_JEV_VERDICTS_VAR}"),
            id,
            FAST_JEV_VERDICTS_VAR,
        ));
    }

    let debate_sources: Vec<String> =
        FAST_JEV_DEBATE_NODE_IDS.iter().map(|id| (*id).to_string()).collect();
    nodes.push(jev_aggregator_node(
        FAST_JEV_DEBATE_VAR,
        "多空辩论汇总",
        "把 4 个辩论判定的输出收敛为数组，供段 D 的风险等级 / 风险强度交叉参考",
        debate_sources.clone(),
        JEV_LAYOUT_X0,
        JEV_LAYOUT_Y0 + (agg_row + 1) as f64 * JEV_LAYOUT_DY,
    ));
    for id in &debate_sources {
        edges.push(direct_edge(&format!("e-{id}-{FAST_JEV_DEBATE_VAR}"), id, FAST_JEV_DEBATE_VAR));
    }

    // `trader-proxy` 的 Jev 入边（理由见本函数文档 ⑤ 末段）。
    for id in FAST_JEV_TRADER_INPUTS {
        edges.push(direct_edge(
            &format!("e-{id}-{FAST_TRADER_PROXY_NODE_ID}"),
            id,
            FAST_TRADER_PROXY_NODE_ID,
        ));
    }
    edges.push(direct_edge(
        &format!("e-{FAST_JEV_VERDICTS_VAR}-{FAST_TRADER_PROXY_NODE_ID}"),
        FAST_JEV_VERDICTS_VAR,
        FAST_TRADER_PROXY_NODE_ID,
    ));

    // ── ⑥ 段 F：把段 B–E 的 Jev 输出与 `analyst-brief` 补进解释节点的上下文 ──
    //
    // 源图该节点的 `context_sources` 是 `[portfolio-risk-gate, rule-check, t-scoring,
    // t-risk]`，四者在快速链**全部保留** ⇒ 它在派生图里入度 ≥ 1，孤儿修复不会碰它
    // （`repair_orphan_nodes` 只处理入度为 0 的节点）。但它读不到任何 Jev 判定 ⇒
    // 解释文案只能复述公式层裁决，无法回答「凭什么这么判」—— 而快速链的判断主体
    // 正是 Jev。故此处补入段 B–E 全部 Jev 输出 + `analyst-brief`（PLAN §2 段 F）。
    //
    // ⚠ `context_sources` **不产生边** —— `node_declared_sources` 只读 `input_mapping`
    //   / `input_sources`，故本步既不改 DAG、也不影响本节点的执行时机。
    //   它能读到这些值的前提是「它们先于本节点执行」，该前提由既有链路保证：
    //   段 B–E 与 `analyst-brief` 都是 `trader-proxy` 的上游，而 `trader-proxy` 又在
    //   `portfolio-mgr` → `portfolio-risk-gate` → `quality-gate` 之前，
    //   本节点排在 `quality-gate` 之后 ⇒ 执行时这些变量早已写入。
    //   若将来有人把本节点提到 `portfolio-risk-gate` 之前，这里会读到 Null 且零报错
    //   （Agent 的上下文拼装对缺失变量是静默跳过），故把该前提记在此处。
    let jev_sources: Vec<String> = FAST_JEV_DIMENSIONS
        .iter()
        .map(|(id, ..)| (*id).to_string())
        .chain(FAST_JEV_SUMMARY_NODE_IDS.iter().map(|id| (*id).to_string()))
        .chain(FAST_JEV_DEBATE_NODE_IDS.iter().map(|id| (*id).to_string()))
        .chain(FAST_JEV_DECISION_NODE_IDS.iter().map(|id| (*id).to_string()))
        .chain(
            [FAST_JEV_VERDICTS_VAR, FAST_JEV_DEBATE_VAR, FAST_BRIEF_NODE_ID]
                .into_iter()
                .map(str::to_string),
        )
        .collect();
    let explainer = nodes
        .iter_mut()
        .find(|n| n.base_id() == FAST_EXPLAINER_NODE_ID)
        .ok_or_else(|| format!("派生结果缺少 `{FAST_EXPLAINER_NODE_ID}` 节点"))?;
    let WorkflowNode::Agent(explainer) = explainer else {
        return Err(format!(
            "`{FAST_EXPLAINER_NODE_ID}` 在源图里不是 Agent 节点 —— 派生假设已失效"
        ));
    };
    for id in &jev_sources {
        // 去重是必要的：`context_sources` 若含重复项，引擎会把同一份输出拼两遍。
        if !explainer.config.context_sources.iter().any(|s| s == id) {
            explainer.config.context_sources.push(id.clone());
        }
    }
    // 立即把长度取出来：下面 ⑦ 还要可变借用 `nodes`，而 `explainer` 的借用会一直活到
    // 它最后一次被使用 —— 在 `tracing!` 里现取会与之冲突（E0499）。
    let explainer_ctx_len = explainer.config.context_sources.len();

    // ── ⑦ `data-quality` 的分析师侧输入重指向（逐项判据见 [`FAST_DQ_DIMENSIONS`]）──
    //
    // **位置**：放在 ⑤ 之后（`j-*` 已全部创建）。
    //
    // **为什么不靠孤儿修复补边**：`repair_orphan_nodes` 只处理**入度为 0** 的节点，而
    // `data-quality` 在边裁剪后仍有 7 条入边（`t-scoring` / `t-risk` / `t-valuation` /
    // `t-hotmoney-data` / `t-lockup-data` / `t-catalyst-data` / `pace-calc`）⇒ 它**不会**
    // 被修复器看到。只改映射不补边 ⇒ 本节点先于 `analyst-brief` / `j-*` 执行、读到全 Null，
    // 且全程零报错（静默失效）⇒ 这 11 条边必须在此**显式**补齐。
    let mut dq_rewrites = 0usize;
    {
        let dq = nodes
            .iter_mut()
            .find(|n| n.base_id() == "data-quality")
            .ok_or_else(|| "派生结果缺少 `data-quality` 节点".to_string())?;
        let WorkflowNode::Code(dq) = dq else {
            return Err("`data-quality` 在源图里不是 Code 节点 —— 派生假设已失效".to_string());
        };

        for (abbr, jev_id, segment) in FAST_DQ_DIMENSIONS {
            // 4 类键同形改写：只换**产地**，不换形态。
            let rewrites: [(String, String); 4] = [
                (format!("{abbr}_verdict"), jev_id.to_string()),
                (format!("{abbr}_report"), format!("{FAST_BRIEF_NODE_ID}.result.{segment}")),
                (format!("{abbr}_untrusted"), format!("{jev_id}.degraded")),
                (format!("{abbr}_tool_calls"), format!("{jev_id}.tool_calls_made")),
            ];
            for (key, value) in rewrites {
                // ⚠ 用 `get_mut` 而不是 `insert`：键**必须已存在**（源图定义的 50 路之一），
                //   缺键说明源图已改 ⇒ 本步的假设失效，必须显式失败而不是静默新增一路。
                let slot = dq.config.input_mapping.get_mut(&key).ok_or_else(|| {
                    format!(
                        "`data-quality` 缺少 `{key}` 映射 —— 源图该节点已改，\
                         快速链的分析师侧输入重指向假设已失效"
                    )
                })?;
                *slot = value;
                dq_rewrites += 1;
            }
            edges.push(direct_edge(&format!("e-{jev_id}-data-quality"), jev_id, "data-quality"));
        }

        // `catalyst_level`：源图取自 `a-catalyst.content.verdict.catalyst_level`（等级文案），
        // 快速链没有该 Agent ⇒ 改取 `j-catalyst.category`（同属**非空字符串**，同时满足
        // `pm_compute_factor_completeness` 的存在性判据与 `missing_factors` 的
        // `== ""` / `== "无"` 判据）。**不可**改指 `j-catalyst` 本身（map）——
        // 字符串比较遇 map 会抛运行期错误，整个节点失败（比少一个因子严重得多）。
        let (_, j_catalyst, _) = FAST_DQ_DIMENSIONS
            .iter()
            .find(|(_, _, segment)| *segment == "catalyst")
            .ok_or_else(|| "`FAST_DQ_DIMENSIONS` 缺少 catalyst 维度定义".to_string())?;
        let slot = dq
            .config
            .input_mapping
            .get_mut("catalyst_level")
            .ok_or_else(|| "`data-quality` 缺少 `catalyst_level` 映射".to_string())?;
        *slot = format!("{j_catalyst}.category");
        dq_rewrites += 1;

        // `consensus_score` 是**唯一**保留原路径的一路（不改写，也刻意不删）：
        // 它取自 `debate-convergence.content.consensus_score`，而快速链没有辩论收敛节点，
        // 也没有等价的**数值**共识产物（`j-divergence` 给的是「一致/分歧/严重分歧」类别串，
        // 改指它会在 `consensus_score <= 0.0` 的数值比较处抛错）⇒ 保留原路径、恒 Null、
        // 「共识评分」诚实记为缺失因子，而不是拿不同量纲的值冒充（本仓禁止假声明）。
    }
    edges.push(direct_edge(
        &format!("e-{FAST_BRIEF_NODE_ID}-data-quality"),
        FAST_BRIEF_NODE_ID,
        "data-quality",
    ));

    tracing::info!(
        "[stock_analysis_setup] 快速链配置层改造完成：新增 `{FAST_TRADER_PROXY_NODE_ID}`，\
         trader 前缀改写 {rewritten} 处，摘除 portfolio-mgr 的 {dropped_dqi:?}，\
         quality-gate 判据改为 `{FAST_QUALITY_GATE_INPUT_VAR}`，\
         `{FAST_BRIEF_NODE_ID}` 换用 raw-digest.rhai（{} 路输入），\
         Jev 判定节点 {} 个（段 B/C/D/E）+ 聚合器 2 个（{} / {}），\
         `data-quality` 分析师侧重指向 {dq_rewrites} 路（本链无 `a-*`），\
         `{FAST_EXPLAINER_NODE_ID}` 上下文来源补至 {} 个",
        FAST_BRIEF_INPUTS.len(),
        specs.len(),
        FAST_JEV_VERDICTS_VAR,
        FAST_JEV_DEBATE_VAR,
        explainer_ctx_len
    );
    Ok(())
}

/// 从源图派生快速链的节点与边。
///
/// 步骤：① 保留集 = 必备（缺失即错）∪ 可选（存在才留）；② 按保留集筛选节点；
/// ③ 父节点被裁掉时清空 `parent_id`（否则画布分组指向不存在的父级）；
/// ④ 边裁剪（两端都在保留集才留）；⑤ 配置层改造 + 追加段 B/C/D/E 的 Jev 判定节点与
/// 其显式边（见 [`apply_fast_chain_overrides`]）；⑥ 孤儿修复（见 [`repair_orphan_nodes`]）。
///
/// `decision_model` 是段 B/C/D/E 全部 `j-*` 判定节点共用的 Jev 决策模型
/// （`None` ⇒ 各节点留空 `model`，运行时回落会话模型）。
///
/// ## 为什么不给被裁掉的「Trader Agent」补一张 `enabled=false` 的占位节点
///
/// `trader` 是全链共享变量（`portfolio-mgr` 从 `trader.content.verdict.*` 读方向 / 置信度 /
/// 目标价 / 止损）。曾考虑保留源图字面量作占位以「声明存在」，实测两条理由否掉它：
///   1. **禁用节点不写变量**：`compute_ready_nodes` 用 `n.base_enabled()` 过滤调度
///      （`dag_store.rs:312`），`enabled=false` 的节点永不执行 ⇒ 它既不写 `trader`，
///      也就完全不能缓解「下游读不到值」——占位只保留了一个**画布上的空盒子**；
///   2. **它会污染终态**：死锁处理里「禁用节点」只有在**直接上游全部 Failed/Skipped** 时
///      才被判 `SKIPPED_DISABLED`（`engine/mod.rs:2879-2892`）；而上游是 Completed 时该分支
///      `return None` ⇒ 节点永远停在 Pending，整个 run 被降级为 `PartiallyCompleted`。
///
/// ⇒ 结论：真正的组装由下游任务的 `trader-proxy` 节点（Code，`output_var = "trader"`）
///    以**可执行**的形式承接；在那之前本链的 `trader_*` 输入走既有 `present()` 守卫 +
///    波动率 fallback（与源链「trader 失败」时同一降级路径），不做任何假声明。
fn derive_fast_workflow_graph(
    source_nodes: &[axagent_harness::workflow_types::WorkflowNode],
    source_edges: &[axagent_harness::workflow_types::WorkflowEdge],
    decision_model: Option<&str>,
) -> Result<DerivedFastGraph, String> {
    use std::collections::{HashMap, HashSet};

    let source_ids: HashSet<&str> = source_nodes.iter().map(|n| n.base_id()).collect();

    // ① 保留集
    let mut keep: HashSet<&str> = HashSet::new();
    for id in FAST_REQUIRED_NODE_IDS {
        if !source_ids.contains(*id) {
            return Err(format!(
                "源模板 `{SOURCE_TEMPLATE_ID}` 缺少必备节点 `{id}` —— \
                 若源图确实删改了该节点，需同步更新 FAST_REQUIRED_NODE_IDS 并确认快速链仍然成立"
            ));
        }
        keep.insert(*id);
    }
    for id in FAST_OPTIONAL_NODE_IDS {
        if source_ids.contains(*id) {
            keep.insert(*id);
        }
    }

    // ② 节点（保持源图顺序，便于人工比对派生结果与源图）
    let mut nodes: Vec<axagent_harness::workflow_types::WorkflowNode> =
        source_nodes.iter().filter(|n| keep.contains(n.base_id())).cloned().collect();

    // ③ 容器归属：父节点未保留 ⇒ 清空，避免指向不存在的父级
    for node in nodes.iter_mut() {
        let parent_dropped =
            node.base().parent_id.as_deref().is_some_and(|parent| !keep.contains(parent));
        if parent_dropped {
            node.base_mut().parent_id = None;
        }
    }

    // ④ 边裁剪 + **完全同形的重复边**合并
    //
    // ⚠ 源图**自身**就带重复边：`e-pace-calc-portfolio-mgr` 在源链里被 push 了两次
    //   （portfolio-mgr 依赖段与 pace-calc 段各一次，两端与 handle 完全相同，纯冗余）。
    //   H1 禁止改源图，而快速链的 `validate_fast_workflow_graph` ① 要求边 id 唯一
    //   ⇒ 在此合并（保留首条）并打 warn，让该源图缺陷**可见**，而不是静默带进新图。
    //
    // 判据边界（**不可**放宽为「按 id 去重」）：只有 6 个字段全同才算冗余副本；
    //   若 id 相同而两端 / handle / 类型不同，那是**真冲突**（两条不同依赖共用一个 id），
    //   机械合并会静默丢掉一条依赖 ⇒ 直接报错，交由人修源图。
    let mut edges: Vec<axagent_harness::workflow_types::WorkflowEdge> = Vec::new();
    let mut seen: HashMap<&str, &axagent_harness::workflow_types::WorkflowEdge> = HashMap::new();
    for e in source_edges
        .iter()
        .filter(|e| keep.contains(e.source.as_str()) && keep.contains(e.target.as_str()))
    {
        if let Some(prev) = seen.get(e.id.as_str()) {
            let identical = prev.source == e.source
                && prev.target == e.target
                && prev.source_handle == e.source_handle
                && prev.target_handle == e.target_handle
                && prev.edge_type == e.edge_type
                && prev.label == e.label;
            if identical {
                tracing::warn!(
                    "[stock_analysis_setup] 源模板 `{SOURCE_TEMPLATE_ID}` 的边 `{id}` \
                     重复出现且完全同形（{src} → {tgt}），快速链已合并为一条；\
                     建议修正源图（快速链只读源图，不在派生中改它）",
                    id = e.id,
                    src = e.source,
                    tgt = e.target
                );
                continue;
            }
            return Err(format!(
                "源模板 `{SOURCE_TEMPLATE_ID}` 的边 id `{id}` 被两条**不同**依赖共用：\
                 `{prev_src} → {prev_tgt}` 与 `{src} → {tgt}`。这无法机械合并（合并会丢依赖），\
                 请先在源图里给其中一条换 id",
                id = e.id,
                prev_src = prev.source,
                prev_tgt = prev.target,
                src = e.source,
                tgt = e.target
            ));
        }
        seen.insert(e.id.as_str(), e);
        edges.push(e.clone());
    }

    // ⑤ 快速链专属的**配置层**改造（追加 `trader-proxy`、改写 `trader` 前缀、
    //    换掉 `quality-gate` 判据）—— 必须排在孤儿修复之前：新节点自带入边，
    //    否则孤儿修复会给它挂到 `trigger` 上，丢掉「等 data-quality 就绪」的语义。
    apply_fast_chain_overrides(&mut nodes, &mut edges, decision_model)?;

    // ⑥ 孤儿修复
    repair_orphan_nodes(&nodes, &mut edges, &keep);

    Ok(DerivedFastGraph { nodes, edges })
}

/// 派生结果的结构不变式（任一不成立 ⇒ 拒绝落库，而不是把一个「看似完整」的坏图写进 DB）。
///
/// 判据：
///   ① 节点 id 唯一、边 id 唯一（重复 id 会让前端画布与调度器各自按不同假设工作）；
///   ② 无悬挂边（两端都必须存在）、无自环；
///   ③ 除 trigger 外每个节点入度 ≥ 1（**孤立节点**会「看起来在图里，实际永不执行」）；
///   ④ trigger 可达全部节点（③ 只保证有入边，不保证与 trigger 连通）；
///   ⑤ 无环（Kahn）。
fn validate_fast_workflow_graph(
    nodes: &[axagent_harness::workflow_types::WorkflowNode],
    edges: &[axagent_harness::workflow_types::WorkflowEdge],
) -> Result<(), String> {
    use std::collections::{HashMap, HashSet, VecDeque};

    const TRIGGER_ID: &str = "trigger";

    let mut ids: HashSet<&str> = HashSet::new();
    for n in nodes {
        if !ids.insert(n.base_id()) {
            return Err(format!("存在重复节点 id `{}`", n.base_id()));
        }
    }
    if !ids.contains(TRIGGER_ID) {
        return Err(format!("缺少 `{TRIGGER_ID}` 节点"));
    }

    // ①② 边
    let mut edge_ids: HashSet<&str> = HashSet::new();
    for e in edges {
        if !edge_ids.insert(e.id.as_str()) {
            return Err(format!("存在重复边 id `{}`", e.id));
        }
        if !ids.contains(e.source.as_str()) || !ids.contains(e.target.as_str()) {
            return Err(format!("悬挂边 `{}`（{} → {}）", e.id, e.source, e.target));
        }
        if e.source == e.target {
            return Err(format!("自环边 `{}`（{}）", e.id, e.source));
        }
    }

    // 邻接 + 入度
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for id in ids.iter() {
        adjacency.entry(id).or_default();
        in_degree.entry(id).or_insert(0);
    }
    for e in edges {
        adjacency.entry(e.source.as_str()).or_default().push(e.target.as_str());
        *in_degree.entry(e.target.as_str()).or_insert(0) += 1;
    }

    // ③ 无孤立节点
    for n in nodes {
        let id = n.base_id();
        if id == TRIGGER_ID {
            continue;
        }
        if in_degree.get(id).copied().unwrap_or(0) == 0 {
            return Err(format!("孤立节点 `{id}`（入度 0，永不执行）"));
        }
    }

    // ④ trigger 可达全部节点
    let mut seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::from([TRIGGER_ID]);
    seen.insert(TRIGGER_ID);
    while let Some(cur) = queue.pop_front() {
        for next in adjacency.get(cur).into_iter().flatten().copied() {
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    if seen.len() != ids.len() {
        let mut unreachable: Vec<&str> =
            ids.iter().copied().filter(|id| !seen.contains(id)).collect();
        unreachable.sort();
        return Err(format!("以下节点从 `{TRIGGER_ID}` 不可达: {unreachable:?}"));
    }

    // ⑤ 无环（Kahn：反复摘除入度 0 的节点，摘不完即有环）
    let mut work: HashMap<&str, usize> = in_degree.clone();
    let mut queue: VecDeque<&str> =
        work.iter().filter(|(_, d)| **d == 0).map(|(id, _)| *id).collect();
    let mut removed = 0usize;
    while let Some(cur) = queue.pop_front() {
        removed += 1;
        for next in adjacency.get(cur).into_iter().flatten() {
            if let Some(d) = work.get_mut(next) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    queue.push_back(next);
                }
            }
        }
    }
    if removed != ids.len() {
        return Err(format!("图存在环（Kahn 仅摘除 {removed}/{} 个节点）", ids.len()));
    }

    Ok(())
}

/// 种子化快速链模板 —— 从已落库的 `stock-analysis` 行派生。
///
/// ⚠ 调用顺序要求：必须在 `seed_stock_analysis_workflow_template` **之后**调用
/// （源行不存在时本函数直接报错，不做任何静默降级）。
pub(crate) async fn seed_stock_analysis_fast_workflow_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    use crate::commands::error::ErrorResponse;
    use axagent_entities::workflow_template;
    use axagent_harness::workflow_types::{
        TriggerConfig, TriggerType, Variable, WorkflowEdge, WorkflowNode,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    // ── 读派生源 ─────────────────────────────────────────────────────────────────────
    let source = workflow_template::Entity::find_by_id(SOURCE_TEMPLATE_ID)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL)
                .with_detail(format!("查询派生源模板失败: {e}"))
        })?
        .ok_or_else(|| {
            ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!(
                "派生源模板 `{SOURCE_TEMPLATE_ID}` 不存在：快速链必须在它之后种子化"
            ))
        })?;

    let source_nodes: Vec<WorkflowNode> = serde_json::from_str(&source.nodes).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("解析派生源节点失败（源图 JSON 可能已损坏）: {e}"))
    })?;
    let source_edges: Vec<WorkflowEdge> = serde_json::from_str(&source.edges).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("解析派生源边失败（源图 JSON 可能已损坏）: {e}"))
    })?;

    // ── 派生 + 校验 ─────────────────────────────────────────────────────────────────
    // Jev 决策模型：段 B/C/D/E 的 29 个 `j-*` 节点共用（解析逻辑与趋势智选快速链同一份，
    // 见 `seed_serenity_fast::resolve_decision_model`；未配置 ⇒ 留空回落会话模型）。
    let decision_model = super::seed_serenity_fast::resolve_decision_model(db).await;
    let DerivedFastGraph { nodes, edges } =
        derive_fast_workflow_graph(&source_nodes, &source_edges, decision_model.as_deref())
            .map_err(|msg| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("派生快速链失败: {msg}"))
            })?;
    validate_fast_workflow_graph(&nodes, &edges).map_err(|msg| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("快速链图结构校验失败（拒绝落库）: {msg}"))
    })?;

    let nodes_json = serde_json::to_string(&nodes).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化快速链节点失败: {e}"))
    })?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化快速链边失败: {e}"))
    })?;

    // ── 变量：默认值来自同一份 `seed_variables`（两链共用一套面板参数），并保留用户改过的值 ──
    let existing =
        workflow_template::Entity::find_by_id(FAST_TEMPLATE_ID).one(db).await.map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL)
                .with_detail(format!("查询快速链模板失败: {e}"))
        })?;

    use super::seed_variables::build_template_variables;
    let variables: Vec<Variable> = build_template_variables();
    let variables_default = serde_json::to_string(&variables).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化变量失败: {e}"))
    })?;
    let variables_val = match existing.as_ref().and_then(|row| row.variables.as_deref()) {
        Some(old) if !old.is_empty() => {
            merge_variable_values(&variables_default, old).unwrap_or(variables_default)
        },
        _ => variables_default,
    };

    // ── 内容门禁（判据见本段开头）：图与变量都一致 ⇒ 跳过 ─────────────────────────────
    // ⚠ 一律走 `super::same_json`（逐值比较）而非字符串比较 —— 理由见其文档
    //   （`variables` 经 `merge_variable_values` 往返后 key 顺序会变）。
    if let Some(row) = existing.as_ref() {
        let same_graph =
            super::same_json(&row.nodes, &nodes_json) && super::same_json(&row.edges, &edges_json);
        let same_variables =
            row.variables.as_deref().is_some_and(|v| super::same_json(v, &variables_val));
        if same_graph && same_variables {
            tracing::info!(
                "[stock_analysis_setup] 快速链模板与派生结果一致，跳过种子化 (TEMPLATE_ID={FAST_TEMPLATE_ID}, nodes={}, edges={})",
                nodes.len(),
                edges.len()
            );
            return Ok(());
        }
        tracing::info!(
            "[stock_analysis_setup] 快速链模板与派生结果不一致，重建 (same_graph={same_graph}, same_variables={same_variables}, nodes={}, edges={})",
            nodes.len(),
            edges.len()
        );
    }

    let tags = serde_json::to_string(&["stock", "analysis", "A股", "fast"]).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化标签失败: {e}"))
    })?;

    // 先删再插，与兄弟种子同一手法（SeaORM 的 .save() 对已存在行的 update 不可靠）
    // 首次播种时该行本就不存在，`delete_by_id` 对 0 行命中不报错，只有 DB 层真出错才落日志。
    if let Err(e) = workflow_template::Entity::delete_by_id(FAST_TEMPLATE_ID).exec(db).await {
        tracing::warn!("[stock_analysis_setup] 重建快速链模板前删除旧行失败 (非致命): {e}");
    }

    // 软门禁：与兄弟种子同一份判据（harness 的端口公理扫描），在此记录结构性死链，不阻断启动
    axagent_harness::workflow_port_axioms::warn_port_axioms_json(
        &format!("stock_analysis_setup:seed_stock_analysis_fast:{FAST_TEMPLATE_ID}"),
        &nodes_json,
        &edges_json,
    );

    let now = chrono::Utc::now().timestamp_millis();

    workflow_template::ActiveModel {
        hooks_config: Set(Some(
            // 与源模板同一组钩子：precheck / enhance 保证两条链的变量注入口径零漂移
            // （同一份 `stock-analysis-enhance`），persist 在业务封装路径下自动跳过。
            serde_json::to_string(&serde_json::json!({
                "pre_exec": ["stock-analysis-precheck", "stock-analysis-enhance"],
                "post_exec": ["stock-analysis-persist"],
            }))
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("序列化 hooks_config 失败: {e}"))
            })?,
        )),
        id: Set(FAST_TEMPLATE_ID.to_string()),
        cluster_id: Set(Some("equity".to_string())),
        // 与源模板**刻意不同**的路由：两条链是两个入口，路由键必须分开。
        route_path: Set(Some("/finance/equity/fast-analysis".to_string())),
        name: Set("A股快速分析（Jev 判定链）".to_string()),
        description: Set(Some(
            "Jev 秒级判定链：取数 → 多维分类 → 公式层决策 → 结论解释；不含分析师/辩论/风控的长篇论述"
                .to_string(),
        )),
        icon: Set("flash".into()),
        tags: Set(Some(tags)),
        version: Set(FAST_TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        // ⚠ 必须 Manual：`init/trigger_recovery.rs` 会为**任何**声明 Schedule 的模板注册
        // 定时触发器（它按 `trigger_type` 分流，不看模板 id）⇒ 若这里抄源模板的
        // `0 9 * * 1-5`，快速链会被每日自动执行一次，与本链「按钮手动触发」的定位冲突。
        trigger_config: Set(Some(
            serde_json::to_string(&TriggerConfig {
                trigger_type: TriggerType::Manual,
                config: serde_json::json!({"stock_code": "{{stock_code}}"}),
            })
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("序列化 trigger_config 失败: {e}"))
            })?,
        )),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        // 输入/输出 schema、错误配置、工具清单：与源模板同源同形（复用而非重写，
        // 避免两链在 IPC 契约层出现细微差异）。
        input_schema: Set(source.input_schema.clone()),
        output_schema: Set(source.output_schema.clone()),
        variables: Set(Some(variables_val)),
        error_config: Set(source.error_config.clone()),
        composite_source: Set(None),
        tool_defs: Set(source.tool_defs.clone()),
        mission_hash: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("写入快速链模板失败: {e}"))
    })?;

    tracing::info!(
        "[stock_analysis_setup] 快速链模板已种子化完成: TEMPLATE_ID={FAST_TEMPLATE_ID}, 派生源={SOURCE_TEMPLATE_ID}, nodes={}, edges={}",
        nodes.len(),
        edges.len()
    );
    Ok(())
}

// ⚠ 本测试模块**必须**留在文件末尾：`clippy::items_after_test_module` 只在 clippy 下暴露
//   （`cargo check` / `cargo test` 都不跑），插在中间会让后续所有代码踩该 lint。
#[cfg(test)]
mod fast_workflow_derivation_tests {
    use super::{
        FAST_BRIEF_INPUTS, FAST_BRIEF_NODE_ID, FAST_DQ_DIMENSIONS, FAST_EXPLAINER_NODE_ID,
        FAST_JEV_DEBATE_NODE_IDS, FAST_JEV_DEBATE_VAR, FAST_JEV_DECISION_NODE_IDS,
        FAST_JEV_DIMENSIONS, FAST_JEV_SUMMARY_NODE_IDS, FAST_JEV_TRADER_INPUTS,
        FAST_JEV_VERDICTS_VAR, FAST_QUALITY_GATE_CASE_EXPR, FAST_QUALITY_GATE_INPUT_VAR,
        FAST_REQUIRED_NODE_IDS, FAST_TEMPLATE_ID, FAST_TRADER_PROXY_NODE_ID, SOURCE_TEMPLATE_ID,
        TRADER_AGENT_PREFIX, TRADER_FAST_PREFIX, fast_jev_nodes,
        seed_stock_analysis_fast_workflow_template, seed_stock_analysis_workflow_template,
        validate_fast_workflow_graph,
    };
    use axagent_entities::workflow_template;
    use axagent_harness::workflow_types::{WorkflowEdge, WorkflowNode};
    use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

    async fn fresh_db() -> axagent_dao::db::DbHandle {
        axagent_dao::db::create_test_pool().await.expect("建临时测试库失败")
    }

    /// 先把派生源种出来，再种快速链（顺序即生产顺序，缺一不可）。
    async fn seed_both(db: &DatabaseConnection) {
        seed_stock_analysis_workflow_template(db).await.expect("原链种子化应成功");
        seed_stock_analysis_fast_workflow_template(db)
            .await
            .expect("快速链种子化应成功（派生源已就位）");
    }

    async fn row(db: &DatabaseConnection, id: &str) -> workflow_template::Model {
        workflow_template::Entity::find_by_id(id)
            .one(db)
            .await
            .expect("查模板失败")
            .unwrap_or_else(|| panic!("模板 `{id}` 应已存在"))
    }

    fn parse_nodes(model: &workflow_template::Model) -> Vec<WorkflowNode> {
        serde_json::from_str(&model.nodes).expect("nodes 应是 JSON 数组")
    }

    fn parse_edges(model: &workflow_template::Model) -> Vec<WorkflowEdge> {
        serde_json::from_str(&model.edges).expect("edges 应是 JSON 数组")
    }

    async fn set_name(db: &DatabaseConnection, id: &str, name: &str) {
        let mut am: workflow_template::ActiveModel = row(db, id).await.into();
        am.name = Set(name.to_string());
        am.update(db).await.expect("改 name 失败");
    }

    /// 删掉指定模板里的某个节点（**只动该行**，模拟「库里的图与常量定义不一致」）。
    async fn drop_node(db: &DatabaseConnection, id: &str, node_id: &str) {
        let model = row(db, id).await;
        let mut nodes: Vec<serde_json::Value> =
            serde_json::from_str(&model.nodes).expect("nodes 应是 JSON 数组");
        nodes.retain(|n| n.get("id").and_then(|v| v.as_str()) != Some(node_id));
        let mut am: workflow_template::ActiveModel = model.into();
        am.nodes = Set(serde_json::to_string(&nodes).expect("序列化失败"));
        am.update(db).await.expect("改 nodes 失败");
    }

    fn node_ids(model: &workflow_template::Model) -> Vec<String> {
        parse_nodes(model).iter().map(|n| n.base_id().to_string()).collect()
    }

    /// 核心测试：派生结果必须是「必备齐全 + 不含源图 agent + 结构合法」的图。
    #[tokio::test]
    async fn derived_fast_graph_is_complete_and_structurally_valid() {
        let handle = fresh_db().await;
        let db = &handle.conn;

        // H1 的机械守门：**先取源图快照**，再种快速链，最后逐字比对 —— 派生只读源行。
        seed_stock_analysis_workflow_template(db).await.expect("原链种子化应成功");
        let source_before = row(db, SOURCE_TEMPLATE_ID).await;
        let (nodes_before, edges_before) =
            (source_before.nodes.clone(), source_before.edges.clone());

        seed_stock_analysis_fast_workflow_template(db).await.expect("快速链种子化应成功");

        let source_after = row(db, SOURCE_TEMPLATE_ID).await;
        assert_eq!(nodes_before, source_after.nodes, "H1 被破坏：快速链种子改动了源图的 nodes");
        assert_eq!(edges_before, source_after.edges, "H1 被破坏：快速链种子改动了源图的 edges");

        let fast = row(db, FAST_TEMPLATE_ID).await;
        let fast_nodes = parse_nodes(&fast);
        let fast_edges = parse_edges(&fast);
        let ids: std::collections::HashSet<String> = node_ids(&fast).into_iter().collect();

        // ① 必备节点齐全（含段 E 落库契约三节点 —— 缺任一项落库会走占位决策分支）
        for id in FAST_REQUIRED_NODE_IDS {
            assert!(ids.contains(*id), "快速链缺必备节点 `{id}`");
        }
        for id in [
            "quality-fallback",
            "portfolio-risk-gate",
            "portfolio-mgr",
            FAST_TRADER_PROXY_NODE_ID,
            "end-output",
        ] {
            assert!(ids.contains(id), "快速链缺落库/契约节点 `{id}`");
        }

        // ② 快速链的 Agent 节点必须**恰为**段 E 契约内的那些（分析师 / 辩论 / 风控 / 研究员
        //    一个都不许留）。期望集由「源图 ∩ FAST_REQUIRED_NODE_IDS」现算，不在测试里再抄一份名单。
        let source_agent_ids: std::collections::HashSet<String> = parse_nodes(&source_after)
            .iter()
            .filter(|n| matches!(n, WorkflowNode::Agent(_)))
            .map(|n| n.base_id().to_string())
            .collect();
        assert!(
            source_agent_ids.len() >= 20,
            "源图 Agent 节点数异常（{} 个），测试前提可能已失效",
            source_agent_ids.len()
        );
        let fast_agent_ids: std::collections::HashSet<String> = fast_nodes
            .iter()
            .filter(|n| matches!(n, WorkflowNode::Agent(_)))
            .map(|n| n.base_id().to_string())
            .collect();
        let expected_agent_ids: std::collections::HashSet<String> = FAST_REQUIRED_NODE_IDS
            .iter()
            .filter(|id| source_agent_ids.contains(**id))
            .map(|id| (*id).to_string())
            .collect();
        assert_eq!(
            fast_agent_ids, expected_agent_ids,
            "快速链的 Agent 节点集应恰为段 E 契约内的 Agent；差集里的源图 Agent 说明裁剪没生效"
        );
        assert!(
            fast_agent_ids.len() < source_agent_ids.len(),
            "快速链不该保留与源图等量的 Agent 节点"
        );

        // ③ 结构不变式（与种子内同一份判据）：无重复 id / 无悬挂边 / 无孤立节点 / trigger 全可达 / 无环
        validate_fast_workflow_graph(&fast_nodes, &fast_edges).expect("派生图应满足全部结构不变式");

        // ④ trigger 必须能走到 end-output（否则链跑完没有聚合输出）
        let adjacency: std::collections::HashMap<&str, Vec<&str>> = fast_edges.iter().fold(
            Default::default(),
            |mut acc: std::collections::HashMap<&str, Vec<&str>>, e| {
                acc.entry(e.source.as_str()).or_default().push(e.target.as_str());
                acc
            },
        );
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec!["trigger"];
        seen.insert("trigger");
        while let Some(cur) = stack.pop() {
            for next in adjacency.get(cur).into_iter().flatten() {
                if seen.insert(*next) {
                    stack.push(*next);
                }
            }
        }
        assert!(seen.contains("end-output"), "trigger 无法到达 end-output");

        // ⑤ H1 的**脚本级**守门：派生只改派生副本，源图的 `analyst-brief` 必须仍是原脚本。
        //    判据锚在两侧各自独有的函数名上（原脚本 `format_analyst` / 新脚本 `render_nested`），
        //    而不是「包含某个共享工具名」——后者在任一脚本被改时都会假绿。
        let brief_code_of = |graph: &[WorkflowNode]| -> String {
            graph
                .iter()
                .find_map(|n| match n {
                    WorkflowNode::Code(c) if c.base.id == FAST_BRIEF_NODE_ID => {
                        Some(c.config.code.clone())
                    },
                    _ => None,
                })
                .unwrap_or_else(|| panic!("图里应含 `{FAST_BRIEF_NODE_ID}` Code 节点"))
        };
        let src_code = brief_code_of(&parse_nodes(&source_after));
        assert!(
            src_code.contains("fn format_analyst"),
            "H1 被破坏：源图 `{FAST_BRIEF_NODE_ID}` 的脚本已被改动"
        );
        assert!(
            !src_code.contains("fn render_nested"),
            "H1 被破坏：源图 `{FAST_BRIEF_NODE_ID}` 被换成了快速链脚本"
        );

        // ⑥ 派生侧：脚本已换、`output_var` 未换、20 路输入全部接线到位。
        let fast_brief_code = brief_code_of(&fast_nodes);
        assert!(
            fast_brief_code.contains("fn render_nested"),
            "快速链 `{FAST_BRIEF_NODE_ID}` 应换用 raw-digest.rhai（分维度摘要）"
        );
        assert!(
            !fast_brief_code.contains("fn format_analyst"),
            "快速链 `{FAST_BRIEF_NODE_ID}` 不该再跑原脚本 —— 本链没有 verdict map 可读"
        );
        let fast_brief = fast_nodes
            .iter()
            .find_map(|n| match n {
                WorkflowNode::Code(c) if c.base.id == FAST_BRIEF_NODE_ID => Some(c),
                _ => None,
            })
            .expect("快速链应含 analyst-brief Code 节点");
        assert_eq!(
            fast_brief.config.output_var, FAST_BRIEF_NODE_ID,
            "`output_var` 必须与节点 id 同名 —— 段 B/段 F 的引用都写 `{FAST_BRIEF_NODE_ID}.result.*`"
        );
        for (key, path) in FAST_BRIEF_INPUTS {
            assert_eq!(
                fast_brief.config.input_mapping.get(key).map(String::as_str),
                Some(path),
                "快速链 `{FAST_BRIEF_NODE_ID}` 的 `{key}` 映射不符"
            );
            let root = path.split('.').next().unwrap_or(path);
            assert!(ids.contains(root), "`{key}` 的上游 `{root}` 不在派生图内");
            assert!(
                adjacency.get(root).is_some_and(|v| v.contains(&FAST_BRIEF_NODE_ID)),
                "`{root}` → `{FAST_BRIEF_NODE_ID}` 的边缺失（孤儿修复应已补上）"
            );
        }
    }

    /// 门禁三态：内容一致跳过 → 图被改动则重建。
    #[tokio::test]
    async fn fast_seed_skips_when_identical_and_rebuilds_when_differs() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_both(db).await;

        assert_eq!(row(db, FAST_TEMPLATE_ID).await.version, super::FAST_TEMPLATE_VERSION);

        // ① 内容一致 ⇒ 跳过：哨兵名必须存活（比比对 updated_at 更可靠 —— 同一毫秒内的两次写入无法区分）
        set_name(db, FAST_TEMPLATE_ID, "SENTINEL-KEEP").await;
        seed_stock_analysis_fast_workflow_template(db).await.expect("二次种子化应成功");
        assert_eq!(
            row(db, FAST_TEMPLATE_ID).await.name,
            "SENTINEL-KEEP",
            "内容一致时应跳过重建（哨兵被覆盖 ⇒ 门禁失效，每次启动都会重写模板）"
        );

        // ② 图不一致 ⇒ 重建：删掉一个节点后再种子化，必须恢复到完整图
        drop_node(db, FAST_TEMPLATE_ID, "t-pledge-data").await;
        seed_stock_analysis_fast_workflow_template(db).await.expect("重建应成功");
        let rebuilt = row(db, FAST_TEMPLATE_ID).await;
        assert!(node_ids(&rebuilt).contains(&"t-pledge-data".to_string()), "内容不一致时应重建");
        assert_ne!(rebuilt.name, "SENTINEL-KEEP", "重建应写回常量定义的名字");
    }

    /// 源图缺必备节点时必须**报错**（而不是静默派生出一个缺一块的图）。
    #[tokio::test]
    async fn missing_required_node_fails_loudly() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_both(db).await;

        // 制造「库里源图 ≠ 常量定义」的形态：删掉源图的一个必备节点
        drop_node(db, SOURCE_TEMPLATE_ID, "t-pledge-data").await;
        let err = seed_stock_analysis_fast_workflow_template(db)
            .await
            .expect_err("源图缺必备节点时应报错");
        assert!(err.contains("t-pledge-data"), "错误信息应点出缺失的节点 id，实际: {err}");
    }

    /// 配置层改造（`apply_fast_chain_overrides`）的三件事必须逐项落地。
    ///
    /// 这是 H2 的机械守门：两链差异**只允许**落在配置层（脚本零改动、源图零改动），
    /// 而配置层漏做不会报错 —— 只会静默降级（f7 因子失效 / `target_price` 为空 /
    /// 仓位钉死 0% / Jev 判定链被 `quality-fallback` 绕开），故逐项断言。
    #[tokio::test]
    async fn fast_chain_config_overrides_are_applied() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_both(db).await;

        let fast = row(db, FAST_TEMPLATE_ID).await;
        let nodes = parse_nodes(&fast);
        let edges = parse_edges(&fast);

        // ① 组装节点：输出变量 / 直执行 / 内嵌脚本口径
        let proxy = nodes
            .iter()
            .find(|n| n.base_id() == FAST_TRADER_PROXY_NODE_ID)
            .expect("快速链应含 trader-proxy 组装节点");
        let WorkflowNode::Code(proxy) = proxy else {
            panic!("`{FAST_TRADER_PROXY_NODE_ID}` 应是 Code 节点");
        };
        assert_eq!(proxy.config.output_var, "trader", "组装节点必须产出 `trader`（下游映射的根）");
        assert!(proxy.config.execute_directly, "组装节点应直执行（不经过工具层）");
        assert!(
            proxy.config.code.contains(TRADER_FAST_PREFIX),
            "内嵌脚本应自行产出 `{TRADER_FAST_PREFIX}*` 形态 —— engine 会给 CodeNode 输出再包一层 \
             `result`，前缀不对则下游全解析为 Null"
        );

        // ② 接线：`data-quality → trader-proxy → portfolio-mgr`
        let has_edge = |s: &str, t: &str| {
            edges.iter().any(|e| e.source.as_str() == s && e.target.as_str() == t)
        };
        assert!(
            has_edge("data-quality", FAST_TRADER_PROXY_NODE_ID),
            "缺少 data-quality → 组装节点的边"
        );
        assert!(
            has_edge(FAST_TRADER_PROXY_NODE_ID, "portfolio-mgr"),
            "缺少 组装节点 → portfolio-mgr 的边"
        );

        // ③ 前缀改写：全图无旧前缀残留，且 9 条 trader 路径逐条指向新前缀
        //    （8 条在 `portfolio-mgr` + 1 条在 `portfolio-risk-gate`）。
        //    ⚠ 判据**不能**是「key 以 `trader_` 开头」—— `portfolio-mgr` 另有
        //    `trader_cap_min_weight` 这类**面板变量自映射**（值就是变量名本身，
        //    走「参数四道门」的变量通路，与 trader 节点输出无关）。
        let expected_trader_paths: [(&str, &str, &str); 9] = [
            ("portfolio-mgr", "trader_direction", "verdict"),
            ("portfolio-mgr", "trader_confidence", "confidence"),
            ("portfolio-mgr", "trader_target_price", "targetPrice"),
            ("portfolio-mgr", "trader_stop_loss", "stopLoss"),
            ("portfolio-mgr", "trader_time_horizon", "timeHorizon"),
            ("portfolio-mgr", "trader_holding_days", "expectedHoldingDays"),
            ("portfolio-mgr", "trader_risk_level", "riskLevel"),
            ("portfolio-mgr", "trader_evidence_count", "evidence_cited"),
            ("portfolio-risk-gate", "target_price", "targetPrice"),
        ];

        for node in &nodes {
            let WorkflowNode::Code(code) = node else { continue };
            for (key, value) in &code.config.input_mapping {
                assert!(
                    !value.starts_with(TRADER_AGENT_PREFIX),
                    "`{}` 的映射 `{key}` 仍是旧前缀 `{TRADER_AGENT_PREFIX}`（快速链的 trader 由 \
                     CodeNode 产出，不改前缀必为 Null）: {value}",
                    code.base.id
                );
            }
        }

        for (node_id, key, suffix) in expected_trader_paths {
            let code = nodes
                .iter()
                .find_map(|n| match n {
                    WorkflowNode::Code(c) if c.base.id == node_id => Some(c),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("快速链缺节点 `{node_id}`"));
            let expected = format!("{TRADER_FAST_PREFIX}{suffix}");
            assert_eq!(
                code.config.input_mapping.get(key).map(String::as_str),
                Some(expected.as_str()),
                "`{node_id}` 的 `{key}` 应指向 `{expected}`"
            );
        }

        // ④ dqi 映射已摘：留着会让 data-quality 恒判 F ⇒ 仓位钉死 0% + Jev 链被绕开
        let mgr_dqi_keys: Vec<&String> = nodes
            .iter()
            .find_map(|n| match n {
                WorkflowNode::Code(c) if c.base.id == "portfolio-mgr" => Some(c),
                _ => None,
            })
            .expect("快速链应含 portfolio-mgr 节点")
            .config
            .input_mapping
            .keys()
            .filter(|k| *k == "dqi_score" || *k == "dqi_grade")
            .collect();
        assert!(
            mgr_dqi_keys.is_empty(),
            "`portfolio-mgr` 不该再消费 data-quality 的 score/grade（本链测不到报告质量），\
             实际残留: {mgr_dqi_keys:?}"
        );

        // ⑤ quality-gate 判据换成本链真实可测的因子完整度
        let qg = nodes
            .iter()
            .find(|n| n.base_id() == "quality-gate")
            .expect("快速链应含 quality-gate 节点");
        let WorkflowNode::Switch(qg) = qg else {
            panic!("quality-gate 应是 Switch 节点");
        };
        assert_eq!(qg.config.input_var, FAST_QUALITY_GATE_INPUT_VAR);
        assert_eq!(qg.config.cases.len(), 1, "快速链只保留一条判据");
        assert_eq!(qg.config.cases[0].value, FAST_QUALITY_GATE_CASE_EXPR);
        assert_eq!(qg.config.cases[0].label, "acceptable");
        assert_eq!(
            qg.config.default_case.as_deref(),
            Some("low-quality"),
            "默认分支应保持不变（不合格时仍走 low-quality → quality-fallback）"
        );

        // ⑥ `analyst-brief`：换脚本 + 换输入，但 id / output_var 一律不动
        //    （段 B 的 `input_var` 与段 F 的引用都写 `analyst-brief.result.*`）。
        let brief = nodes
            .iter()
            .find(|n| n.base_id() == FAST_BRIEF_NODE_ID)
            .expect("快速链应含 analyst-brief 节点");
        let WorkflowNode::Code(brief) = brief else {
            panic!("`{FAST_BRIEF_NODE_ID}` 应是 Code 节点");
        };
        assert_eq!(
            brief.config.output_var, FAST_BRIEF_NODE_ID,
            "`output_var` 改名会让段 B / 段 F 的引用整体失效"
        );
        assert!(
            brief.config.code.contains("fn render_nested"),
            "应换成快速链专属的 raw-digest.rhai（原脚本只认 verdict map 的 bull/bear_score）"
        );
        assert!(
            !brief.config.code.contains("fn format_analyst"),
            "不该再跑原 `analyst-brief.rhai` —— 本链没有 10 份 verdict map 可读"
        );
        assert_eq!(
            brief.config.input_mapping.len(),
            FAST_BRIEF_INPUTS.len(),
            "输入映射条数应恰为 14 路数据源 + 6 条算法腿"
        );
        for (key, path) in FAST_BRIEF_INPUTS {
            assert_eq!(
                brief.config.input_mapping.get(key).map(String::as_str),
                Some(path),
                "`{FAST_BRIEF_NODE_ID}` 的 `{key}` 应指向 `{path}`"
            );
        }
        assert!(
            brief.base.title.contains("快速链"),
            "标题应标明是快速链专属口径，避免与源图同名节点混淆：{}",
            brief.base.title
        );
    }

    /// ⑦ 步的机械守门：`data-quality` 的分析师侧输入必须**重指向本链真实存在的产物**。
    ///
    /// 唯一理由：这 41 路映射在派生图里全部解析为 Null **不会报错** —— 只会让 `score ≈ 23`
    /// 恒判 F ⇒ `portfolio-mgr` 仓位钉死 0% + 数据质量弹窗 10 条全 `missing`。而「不改脚本、
    /// 不新建节点」是 H1/H2 的边界：本步只允许落在配置层，故既要断言**改对了**、也要断言
    /// **没越界**（脚本未换、无 `a-*` 残留）。
    #[tokio::test]
    async fn fast_chain_data_quality_inputs_are_repointed() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_both(db).await;

        let fast = row(db, FAST_TEMPLATE_ID).await;
        let nodes = parse_nodes(&fast);
        let edges = parse_edges(&fast);
        let ids: std::collections::HashSet<String> = node_ids(&fast).into_iter().collect();
        let has_edge = |s: &str, t: &str| {
            edges.iter().any(|e| e.source.as_str() == s && e.target.as_str() == t)
        };

        let dq = nodes
            .iter()
            .find_map(|n| match n {
                WorkflowNode::Code(c) if c.base.id == "data-quality" => Some(c),
                _ => None,
            })
            .expect("快速链应含 data-quality Code 节点");

        // ① 节点本体与脚本一律复用（H1/H2）：`output_var` 未换名、脚本仍是原 `data-quality.rhai`
        //    （判据锚在脚本独有的函数名上 —— 换成任何别的脚本都会翻脸）。
        assert_eq!(dq.config.output_var, "data-quality");
        assert!(
            dq.config.code.contains("fn report_quality")
                && dq.config.code.contains("fn is_untrusted"),
            "`data-quality` 必须复用同一份 `data-quality.rhai`（本链不另造脚本、不做版本分支）"
        );

        // ② 4 类键**逐项**：键必须仍在（删键 ⇒ Rhai 在 `present(mk_untrusted)` 处抛
        //    "Variable not found" ⇒ 节点整体失败，比读到 Null 更糟），且形态与源图一一对应 ——
        //    verdict=map / report=文本 / untrusted=**裸 bool**（`mk_untrusted == true` 是直接比较）/
        //    tool_calls=数组或 Null（`attribution_note` 的守卫①）。
        for (abbr, jev_id, segment) in FAST_DQ_DIMENSIONS {
            let cases = [
                (format!("{abbr}_verdict"), (*jev_id).to_string(), "verdict"),
                (
                    format!("{abbr}_report"),
                    format!("{FAST_BRIEF_NODE_ID}.result.{segment}"),
                    "report",
                ),
                (format!("{abbr}_untrusted"), format!("{jev_id}.degraded"), "untrusted"),
                (format!("{abbr}_tool_calls"), format!("{jev_id}.tool_calls_made"), "tool_calls"),
            ];
            for (key, want, kind) in cases {
                assert_eq!(
                    dq.config.input_mapping.get(&key).map(String::as_str),
                    Some(want.as_str()),
                    "`{abbr}_{kind}` 应指向 `{want}`（键不得删、指向不得留旧路径）"
                );
            }
            assert!(
                has_edge(jev_id, "data-quality"),
                "缺边 `{jev_id}` → `data-quality` —— 本节点会先于上游执行、读到全 Null 且**零报错**"
            );
        }

        // ③ `catalyst_level` 必须指向**字符串**：`missing_factors` 里有 `catalyst_level == ""` 的
        //    字符串比较，指到 map（如 `j-catalyst` 本身）会抛运行期错误 —— 比少一个因子严重得多。
        assert_eq!(
            dq.config.input_mapping.get("catalyst_level").map(String::as_str),
            Some("j-catalyst.category")
        );
        assert!(
            has_edge(FAST_BRIEF_NODE_ID, "data-quality"),
            "缺边 `{FAST_BRIEF_NODE_ID}` → `data-quality` —— 10 路 `*_report` 与 `catalyst_level` 都依赖它"
        );

        // ④ 全表不得再有 `a-*` 残留，且每个上游根都必须在派生图内（否则恒 Null）。
        //    `consensus_score` 是**唯一**刻意保留的原路径：它取自
        //    `debate-convergence.content.consensus_score`，而本链既无辩论收敛节点、也无等价的
        //    **数值**共识产物（`j-divergence` 给的是类别串，改指它会在 `<= 0.0` 的数值比较处抛错）
        //    ⇒ 保留原路径、恒 Null，把「共识评分」诚实记为缺失因子。
        for (key, value) in &dq.config.input_mapping {
            if key == "consensus_score" {
                assert!(
                    value.starts_with("debate-convergence"),
                    "`consensus_score` 是本表唯一刻意保留的原路径，实际: {value}"
                );
                continue;
            }
            assert!(
                !value.starts_with("a-"),
                "`data-quality` 的映射 `{key}` 仍指向已被裁掉的分析师节点: {value}"
            );
            let root = value.split('.').next().unwrap_or(value);
            assert!(ids.contains(root), "`{key}` 的上游根 `{root}` 不在派生图内 ⇒ 恒 Null");
        }
    }

    /// 抠出 prompt 里的 `{...}` 占位符（取值口径与 `render_template` 同源：首个 `}` 之前）。
    fn placeholders(prompt: &str) -> Vec<&str> {
        prompt.split('{').skip(1).filter_map(|rest| rest.split('}').next()).collect()
    }

    /// T7 的机械守门：段 B/C/D/E 的 Jev 判定节点必须**全部**入图，且每条声明都有对应的边。
    ///
    /// 唯一理由：漏补一条边**不会报错**。`node_declared_sources` 不识别
    /// `LlmClassifier` 的 `input_var`（`_ => {}` 分支），`repair_orphan_nodes` 的 `keep`
    /// 又不含新增节点 ⇒ 该节点会先于上游执行、读到 Null 并静默走 `fallback_label`
    /// （方向恒中性、档位不参与夹逼、证据清单为空）。故逐条断言。
    #[tokio::test]
    async fn fast_chain_jev_nodes_are_wired() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_both(db).await;

        let fast = row(db, FAST_TEMPLATE_ID).await;
        let nodes = parse_nodes(&fast);
        let edges = parse_edges(&fast);
        let ids: std::collections::HashSet<String> = node_ids(&fast).into_iter().collect();
        let has_edge = |s: &str, t: &str| {
            edges.iter().any(|e| e.source.as_str() == s && e.target.as_str() == t)
        };

        // ① 单点定义的节点数：段 B 10 维度 + 3 汇总 + 段 C 4 辩论 + 段 D 9 风险/估值 + 段 E 3 契约
        let specs = fast_jev_nodes();
        assert_eq!(
            specs.len(),
            29,
            "段 B/C/D/E 的 Jev 节点数应为 29（10 + 3 + 4 + 9 + 3）—— 少一个即某个判定没进图"
        );

        // ② 逐节点：在图内 / 是 llmClassifier / output_var 同名 / input_var 非空且与定义一致 /
        //    每条声明的上游都有边（**本测试的核心断言**）
        for spec in &specs {
            let node = nodes
                .iter()
                .find(|n| n.base_id() == spec.id)
                .unwrap_or_else(|| panic!("快速链缺 Jev 节点 `{}`", spec.id));
            let WorkflowNode::LlmClassifier(c) = node else {
                panic!("`{}` 应是 llmClassifier 节点", spec.id);
            };
            assert_eq!(c.config.output_var, spec.id, "`{}` 的 output_var 必须与 id 同名", spec.id);
            assert!(
                !c.config.input_var.is_empty(),
                "`{}` 的 input_var 不得留空 —— 留空会把全部 variables 拼进 prompt（32k 预算直接爆）",
                spec.id
            );
            assert_eq!(
                c.config.input_var, spec.input_var,
                "`{}` 的 input_var 与单点定义不符",
                spec.id
            );
            assert_eq!(
                c.config.categories,
                spec.categories.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
                "`{}` 的 categories 与单点定义不符",
                spec.id
            );
            assert_eq!(
                c.config.fallback_label.as_deref(),
                Some(spec.fallback),
                "`{}` 应显式配置兜底档（LLM 调用失败时降级而非 Failed，避免下游死锁）",
                spec.id
            );

            let root = spec.input_var.split('.').next().unwrap_or(&spec.input_var);
            assert!(
                spec.upstreams.contains(&root),
                "`{}` 的 input_var 根段 `{root}` 不在 upstreams {:?} 内",
                spec.id,
                spec.upstreams
            );
            for upstream in &spec.upstreams {
                assert!(ids.contains(*upstream), "`{}` 的上游 `{upstream}` 不在派生图内", spec.id);
                assert!(
                    has_edge(upstream, spec.id),
                    "缺边 `{upstream}` → `{}` —— 该节点会先于上游执行并静默走兜底档",
                    spec.id
                );
            }
        }

        // ③ 四个 const 组的 id 必须全部落在单点定义里（防「改了 const 没改构造」）
        let mut const_ids: Vec<&str> = FAST_JEV_DIMENSIONS.iter().map(|(id, ..)| *id).collect();
        const_ids.extend(FAST_JEV_SUMMARY_NODE_IDS);
        const_ids.extend(FAST_JEV_DEBATE_NODE_IDS);
        const_ids.extend(FAST_JEV_DECISION_NODE_IDS);
        for id in const_ids {
            assert!(specs.iter().any(|s| s.id == id), "单点定义缺 `{id}`（const 与构造已脱节）");
        }

        // ④ 两个聚合器：id / output_var / strategy / 源清单 / 逐条边
        for (var, sources) in [
            (
                FAST_JEV_VERDICTS_VAR,
                FAST_JEV_DIMENSIONS.iter().map(|(id, ..)| *id).collect::<Vec<_>>(),
            ),
            (FAST_JEV_DEBATE_VAR, FAST_JEV_DEBATE_NODE_IDS.to_vec()),
        ] {
            let node = nodes
                .iter()
                .find(|n| n.base_id() == var)
                .unwrap_or_else(|| panic!("快速链缺聚合器 `{var}`"));
            let WorkflowNode::Aggregator(agg) = node else {
                panic!("`{var}` 应是 aggregator 节点");
            };
            assert_eq!(agg.config.output_var, var, "聚合器的 output_var 必须与 id 同名");
            assert_eq!(agg.config.strategy, "all", "`merge` 会因同名字段互相覆盖而只剩最后一项");
            assert!(agg.config.wait_for_all, "必须等齐全部源，否则聚合出的是半截数组");
            let expected: Vec<String> = sources.iter().map(|s| (*s).to_string()).collect();
            assert_eq!(agg.config.input_sources, expected, "`{var}` 的源清单不符");
            for src in &expected {
                assert!(ids.contains(src), "`{var}` 的源 `{src}` 不在派生图内");
                assert!(has_edge(src, var), "缺边 `{src}` → `{var}`");
            }
        }

        // ⑤ `trader-proxy` 的 Jev 通路：6 个判定 + 1 个维度数组，共 7 条边 + `j_dimensions` 映射
        for id in FAST_JEV_TRADER_INPUTS.iter().copied().chain([FAST_JEV_VERDICTS_VAR]) {
            assert!(
                has_edge(id, FAST_TRADER_PROXY_NODE_ID),
                "缺边 `{id}` → `{FAST_TRADER_PROXY_NODE_ID}`"
            );
        }
        let proxy = nodes
            .iter()
            .find_map(|n| match n {
                WorkflowNode::Code(c) if c.base.id == FAST_TRADER_PROXY_NODE_ID => Some(c),
                _ => None,
            })
            .expect("快速链应含 trader-proxy 节点");
        assert_eq!(
            proxy.config.input_mapping.get("j_dimensions").map(String::as_str),
            Some("jev_verdicts.result"),
            "证据清单来源 ①（段 B 维度判定）必须接通 —— 缺它则 f7 的证据修正恒按 0.4 下界"
        );

        // ⑥ 两处「静默失效」高发点的逐字锁定
        let j_conf = nodes
            .iter()
            .find_map(|n| match n {
                WorkflowNode::LlmClassifier(c) if c.base.id == "j-confidence" => Some(c),
                _ => None,
            })
            .expect("快速链应含 j-confidence");
        assert_eq!(
            j_conf.config.confidence_threshold,
            Some(0.0),
            "j-confidence 要的是 confidence **数值**：阈值必须为 0（`None` 则根本不输出 confidence；\
             正数则低置信时被替换成兜底档）"
        );
        for id in ["j-target", "j-stop"] {
            let node = nodes
                .iter()
                .find_map(|n| match n {
                    WorkflowNode::LlmClassifier(c) if c.base.id == id => Some(c),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("快速链应含 `{id}`"));
            assert_eq!(
                node.config.categories,
                vec!["5%", "10%", "20%", "30%"],
                "`{id}` 的档位字面量必须与 `trader-proxy.rhai` 的 `tier_pct()` 词表逐字一致"
            );
        }

        // ⑦ prompt 占位符：根段必须在图内，且**不得含连字符** ——
        //    `render_template` 的正则是 `\{([a-zA-Z0-9_.]+)\}`，含 `-` 的占位符永不替换、
        //    也永不报错（`{j-direction.category}` 这类写法是最容易踩的静默失效）。
        for node in &nodes {
            let WorkflowNode::LlmClassifier(c) = node else { continue };
            for ph in placeholders(&c.config.prompt) {
                assert!(
                    ph.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'),
                    "`{}` 的 prompt 占位符 `{{{ph}}}` 含非法字符（如连字符）—— 不会被替换且不报错",
                    c.base.id
                );
                let root = ph.split('.').next().unwrap_or(ph);
                assert!(
                    ids.contains(root),
                    "`{}` 的 prompt 占位符 `{{{ph}}}` 指向图外变量 `{root}`",
                    c.base.id
                );
            }
        }

        // ⑧ 段 F：`decision-explainer` 的上下文必须覆盖段 B–E 全部 Jev 输出 + `analyst-brief`。
        //    该步**不产生边**（只影响 prompt 拼装）⇒ 漏做同样零报错，只能靠本断言守。
        let explainer = nodes
            .iter()
            .find_map(|n| match n {
                WorkflowNode::Agent(a) if a.base.id == FAST_EXPLAINER_NODE_ID => Some(a),
                _ => None,
            })
            .unwrap_or_else(|| panic!("快速链应含 `{FAST_EXPLAINER_NODE_ID}`（Agent 节点）"));
        let sources: std::collections::HashSet<&str> =
            explainer.config.context_sources.iter().map(String::as_str).collect();
        assert_eq!(
            sources.len(),
            explainer.config.context_sources.len(),
            "`{FAST_EXPLAINER_NODE_ID}` 的 context_sources 含重复项 —— 同一份输出会被拼两遍"
        );
        let mut expected: Vec<&str> = FAST_JEV_DIMENSIONS.iter().map(|(id, ..)| *id).collect();
        expected.extend(FAST_JEV_SUMMARY_NODE_IDS);
        expected.extend(FAST_JEV_DEBATE_NODE_IDS);
        expected.extend(FAST_JEV_DECISION_NODE_IDS);
        expected.extend([FAST_JEV_VERDICTS_VAR, FAST_JEV_DEBATE_VAR, FAST_BRIEF_NODE_ID]);
        for id in expected {
            assert!(
                sources.contains(id),
                "`{FAST_EXPLAINER_NODE_ID}` 的 context_sources 缺 `{id}` —— \
                 解释文案读不到该判定，且缺失时静默跳过、零报错"
            );
        }
        // 源图原有的四个来源必须保留（本步只做追加，不替换）
        for id in ["portfolio-risk-gate", "rule-check", "t-scoring", "t-risk"] {
            assert!(
                sources.contains(id),
                "`{FAST_EXPLAINER_NODE_ID}` 原有的 `{id}` 来源被覆盖 —— 本步只允许追加"
            );
        }
    }
}
