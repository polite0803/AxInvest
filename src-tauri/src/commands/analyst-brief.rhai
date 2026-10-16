// analyst-brief.rhai — 分析师摘要生成器（v133 按持有期分档 → v161 每档一份产出）
//
// 在逐档分析师实例结束后运行，从各实例的 verdict map 中提取结构化评分和
// 关键论据（bull_points / bear_points），压缩为一份轻量摘要供**同档**辩论阶段使用。
//
// ── v161：本脚本仍是**一份**，但图上有**四个**消费者节点 ──
// 摘要节点从「一份合并 brief」改成「每档各一份产出」（`analyst-brief--ultra_short` /
// `--short` / `--mid` / `--long`），每个节点只吃**本档**的逐档分析师实例。
// 实现方式刻意选的是「一份脚本 + 四份映射」，而不是「四份脚本」或「一份脚本读全部键」：
//   · 四份脚本 ⇒ `format_analyst` 那段评分/论据抽取逻辑复制四遍（禁区 12 的重复定义），
//     且四份会各自漂移；
//   · 一份脚本读 `{短名}__{档}` 全部 21 个键 ⇒ 每个节点都会把四档的腿**都拼进正文**，
//     四份产出逐字相同 = `PLAN-four-horizon-workflow-alignment.md` §五十三 明令禁止的
//     「把一份合并 brief 复制四遍」伪装（面板四格各自显示，读者以为是四个独立摘要）。
//   ⇒ 于是键名**去掉档位后缀**（`a_market_raw` 而不是 `a_market_raw__mid`），
//     某条腿属不属于本档，由 Rust 侧那张 `input_mapping` 决定；脚本只看「注入没注入」。
//     档位归属这份事实只存在于 `tiered`（权威矩阵）一处，脚本不再重复声明一次。
//
// V58 修复(2026-07-23): input_mapping 已改为 {id}.content.verdict，
// resolve_var_path 在 Rust 侧自动解析 content JSON 并下钻到 verdict 字段，
// 注入到 Rhai 的变量已经是 verdict map（或 () 表示无数据）。
// 不再需要在 Rhai 中做字符串解析。
//
// ⚠ Rhai **不能枚举 scope 变量**（脚本内无法遍历「注入了哪些变量」），
//   所以下面 9 条读取**必须显式列出**，且必须列满「四档子集的并集」
//   （少列一条 = 那一腿在**它所属的那些档**的摘要里永久缺席，而没人报错）。
//   两侧同集由 `seed_consistency_tests.rs::analyst_brief_keys_match_tiered_instances` 机械锁住：
//   它逐档比对「本档节点的 input_mapping 键集」与「该档 `analyst_subset()` 短名集」，
//   并单独断言脚本的 present() 集 == 四档短名的**并集**。
//   命名规则：短名词根见 seed 的 `analyst_short_key`（id 域用 `--`，Rhai 变量名用 `_`，
//   标识符不允许 `-`）。
//
// ⚠ 每条读取都包 `if present(...)`：
//   · 未注入的变量经 code_executor 的 V57 扫描自动补 unit ⇒ present 为 false ⇒ 跳过；
//     这一支现在承担**两**种缺席，二者都不该报错：① 本档子集不含该分析师（结构性缺席），
//     ② 该腿实例失败/无产出（运行期缺席）；
//   · 拿不到值就整段不输出该条（与「数据不可用」同族的「缺席不伪装」），
//     而不是让整份摘要因一个实例失败而抛出。
//   · ⚠ 缺席段**不写「本档无此维度」占位文本** —— 结构性缺席（如超短档没有基本面）
//     与运行期缺席（基本面跑了但挂了）在文本层必须可区分，判据在 Rust 侧的节点清单上，
//     不在摘要文本里造第二份事实。

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
// 标题不写档位名：本摘要属于哪一档，由**节点 id / 节点标题**与辩手 prompt 各自声明
// （辩手的 system_prompt 由 seed 按 `Period::label_zh()` 逐字追加）。脚本里没有第二份
// 档名事实，也就没有「脚本说中期、节点 id 写 mid、prompt 说 28 日」三处各抄一遍的漂移面。
let brief = "## 分析师摘要（本持有周期的分析师实例，缺席的维度不出现）\n\n";

// 读取顺序 = 短名词根的稳定序（与 `analyst_short_key` 的表同域，不按档分组）。
if present(a_market_raw) { brief += format_analyst("行情技术面", a_market_raw); }
if present(a_sentiment_raw) { brief += format_analyst("市场情绪", a_sentiment_raw); }
if present(a_hot_money_raw) { brief += format_analyst("游资动向", a_hot_money_raw); }
if present(a_lockup_raw) { brief += format_analyst("解禁减持", a_lockup_raw); }
if present(a_fundamentals_raw) { brief += format_analyst("基本面估值", a_fundamentals_raw); }
if present(a_research_raw) { brief += format_analyst("券商研报", a_research_raw); }
if present(a_policy_raw) { brief += format_analyst("宏观政策", a_policy_raw); }
if present(a_sector_raw) { brief += format_analyst("行业景气度", a_sector_raw); }
if present(a_catalyst_raw) { brief += format_analyst("催化剂与叙事", a_catalyst_raw); }

brief
