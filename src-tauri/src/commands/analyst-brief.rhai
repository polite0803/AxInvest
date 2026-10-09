// analyst-brief.rhai — 分析师摘要生成器（v133：按持有期分档）
//
// 在逐档分析师实例结束后运行，从各实例的 verdict map 中提取结构化评分和
// 关键论据（bull_points / bear_points），按档归组压缩为一份轻量摘要供辩论阶段使用。
//
// V58 修复(2026-07-23): input_mapping 已改为 {id}.content.verdict，
// resolve_var_path 在 Rust 侧自动解析 content JSON 并下钻到 verdict 字段，
// 注入到 Rhai 的变量已经是 verdict map（或 () 表示无数据）。
// 不再需要在 Rhai 中做字符串解析。
//
// ⚠ v133（B2-2）：Rhai **不能枚举 scope 变量**（脚本内无法遍历「注入了哪些变量」），
//   所以下面 21 条读取**必须显式列出**；变量名集合与 Rust 侧 `tiered` 生成的
//   `analyst-brief` input_mapping 键集**同集**，由
//   `seed_consistency_tests.rs::analyst_brief_keys_match_tiered_instances` 机械锁住
//   （任一侧增删实例而另一侧没跟 ⇒ 该测试红，不会静默少一段摘要）。
//   命名规则：`{短名词根}__{tier}`（短名词根见 seed 的 `analyst_short_key`；
//   id 域用 `--`，Rhai 变量名用 `__` —— 标识符不允许 `-`）。
//
// ⚠ 每条读取都包 `if present(...)`：
//   · 未注入的变量经 code_executor 的 V57 扫描自动补 unit ⇒ present 为 false ⇒ 跳过；
//   · 拿不到值就跳过该条（与「数据不可用」同族的「缺席不伪装」），
//     而不是让整份摘要因一个实例失败而抛出。

fn present(x) { type_of(x) != "()" }

fn format_analyst(label, verdict) {
    // verdict 直接来自 input_mapping 的 {id}.content.verdict 解析结果
    if type_of(verdict) != "map" {
        return `#### ${label}\n**数据不可用**（该分析师报告解析失败，无法提取评分）\n\n`;
    }
    let bull = verdict["bull_score"];
    let bear = verdict["bear_score"];
    // 评分数据不可用时标记
    let bull_valid = type_of(bull) == "i64" || type_of(bull) == "f64";
    let bear_valid = type_of(bear) == "i64" || type_of(bear) == "f64";
    if !bull_valid && !bear_valid {
        return `#### ${label}\n**数据不可用**（评分字段缺失）\n\n`;
    }
    // 共识分数 = bull - bear
    let cons = if type_of(bull) == "i64" && type_of(bear) == "i64" {
        bull - bear
    } else if type_of(bull) == "f64" && type_of(bear) == "f64" {
        bull - bear
    } else if type_of(bull) == "i64" && type_of(bear) == "f64" {
        bull.to_float() - bear
    } else if type_of(bull) == "f64" && type_of(bear) == "i64" {
        bull - bear.to_float()
    } else { "" };

    let brief = `#### ${label}\n`;
    brief += `看多=${bull} 看空=${bear}`;
    if type_of(cons) == "i64" || type_of(cons) == "f64" {
        brief += ` 共识=${cons}`;
    }
    brief += "\n";

    // 可选：关键论据
    let bull_pts = verdict["bull_points"];
    if type_of(bull_pts) == "array" && bull_pts.len() > 0 {
        brief += "多方论据: " + join(bull_pts, "；") + "\n";
    }
    let bear_pts = verdict["bear_points"];
    if type_of(bear_pts) == "array" && bear_pts.len() > 0 {
        brief += "空方论据: " + join(bear_pts, "；") + "\n";
    }
    brief += "\n";
    brief
}

// ── 主逻辑 ──
let brief = "## 分析师综合摘要（按持有期分档）\n\n";

// ══ 超短线（2 日）：动量 + 情绪 + 游资 + 催化剂 ══
if present(a_market_raw__ultra_short) { brief += format_analyst("行情技术面（超短）", a_market_raw__ultra_short); }
if present(a_sentiment_raw__ultra_short) { brief += format_analyst("市场情绪（超短）", a_sentiment_raw__ultra_short); }
if present(a_hot_money_raw__ultra_short) { brief += format_analyst("游资动向（超短）", a_hot_money_raw__ultra_short); }
if present(a_catalyst_raw__ultra_short) { brief += format_analyst("催化剂与叙事（超短）", a_catalyst_raw__ultra_short); }

// ══ 短线（1-2 周）：再加 解禁/减持（供给冲击） ══
if present(a_market_raw__short) { brief += format_analyst("行情技术面（短线）", a_market_raw__short); }
if present(a_sentiment_raw__short) { brief += format_analyst("市场情绪（短线）", a_sentiment_raw__short); }
if present(a_hot_money_raw__short) { brief += format_analyst("游资动向（短线）", a_hot_money_raw__short); }
if present(a_lockup_raw__short) { brief += format_analyst("解禁减持（短线）", a_lockup_raw__short); }
if present(a_catalyst_raw__short) { brief += format_analyst("催化剂与叙事（短线）", a_catalyst_raw__short); }

// ══ 中线（3-8 周）：景气/估值/预期 + 趋势 ══
// v145：本段不再有「价值评估（中线）」——`value-investor--mid` 在图上是**辩论之后**的节点
// （入边 `e-debate-value-investor--mid` / `e-convergence-value-investor--mid`），
// brief 执行时它必然还没跑 ⇒ 原来那行 `present()` 恒假，是一段没人报错的死读取。
if present(a_market_raw__mid) { brief += format_analyst("行情技术面（中线）", a_market_raw__mid); }
if present(a_hot_money_raw__mid) { brief += format_analyst("游资动向（中线）", a_hot_money_raw__mid); }
if present(a_lockup_raw__mid) { brief += format_analyst("解禁减持（中线）", a_lockup_raw__mid); }
if present(a_fundamentals_raw__mid) { brief += format_analyst("基本面估值（中线）", a_fundamentals_raw__mid); }
if present(a_research_raw__mid) { brief += format_analyst("券商研报（中线）", a_research_raw__mid); }
if present(a_catalyst_raw__mid) { brief += format_analyst("催化剂与叙事（中线）", a_catalyst_raw__mid); }

// ══ 长线（3 个月+）：基本面/政策/行业 + 趋势 ══
// v145：同上，「价值评估（长线）」那行一并删除（理由见中线段注释）。
if present(a_market_raw__long) { brief += format_analyst("行情技术面（长线）", a_market_raw__long); }
if present(a_fundamentals_raw__long) { brief += format_analyst("基本面估值（长线）", a_fundamentals_raw__long); }
if present(a_research_raw__long) { brief += format_analyst("券商研报（长线）", a_research_raw__long); }
if present(a_policy_raw__long) { brief += format_analyst("宏观政策（长线）", a_policy_raw__long); }
if present(a_sector_raw__long) { brief += format_analyst("行业景气度（长线）", a_sector_raw__long); }
if present(a_catalyst_raw__long) { brief += format_analyst("催化剂与叙事（长线）", a_catalyst_raw__long); }

brief
