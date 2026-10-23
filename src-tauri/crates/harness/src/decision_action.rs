//! 决策 action（操作档位）值域权威定义 + 归一化。
//!
//! ## 为什么在 harness 而不是 analysis-engine（2026-09-14 迁入）
//!
//! 本模块初版放在 `analysis-engine`，但 `harness` / `market-sim` / `notification` /
//! `astock-data` **都不依赖** analysis-engine ⇒ 这些 crate 的消费端永远够不到唯一
//! 归一化器，只能各自 `match` 中文字面量。这正是「修了定义端、漏改的消费端仍
//! fail-open」的**结构性成因**，靠逐个 grep 消费端治不了根。
//!
//! 按 AGENTS.md 规则 12（共享数据模型 / 契约的权威定义在 `axagent-harness`），
//! 上移至 harness（harness 内部零依赖，可安全承载）。
//! `analysis-engine::decision_action` 以 re-export 形式保留，旧路径与旧调用点不变。
//!
//! ## 为什么需要这个模块
//!
//! 同一「操作档位」语义在项目里散成了多套互不相交的字符串值域：
//!
//! | 来源 | 值域 |
//! |---|---|
//! | `portfolio-mgr.rhai`（公式实际产出） | 买入 / 增持 / 持有 / 观望 / 减持 / 卖出 |
//! | `harness::dashboard_report` | 强烈买入 / 买入 / 增持 / 持有 / 减持 / 卖出（**无「观望」**） |
//! | 前端 `StockAction` | BUY / INCREASE / HOLD / REDUCE / SELL / WAIT / UNCERTAIN (+ UNAVAILABLE) |
//! | 回测策略映射 | 中文 + 部分英文（**缺 `WAIT`**） |
//! | 风控否决 `apply_risk_veto` | **仅中文** |
//!
//! 消费端各自 `match` 字符串造成两类静默失效：
//!
//! - **fail-open**：`apply_risk_veto` 遇到英文值域时不匹配任何分支，直接放行
//!   「极高风险禁止持仓」的否决，且不报错；
//! - **兜底错档**：`map_action_to_strategy_id` 缺 `WAIT` 分支，同义的英文 `WAIT`
//!   落 `watchlist`，而中文「观望」落 `capital` —— 同一语义两条入口两个策略。
//!
//! 本模块提供**唯一**归一化入口。新增或修改值域必须在此登记；消费端只允许匹配
//! `ActionKind`，不得再各自 `match` 字符串字面量（铁律：同名语义散成多套值域 ⇒
//! 必有一处 fail-open）。
//!
//! ## 分档判据（2026-10-22 裁定：全局只能一处权威定义，且分档不得重叠、不得语义模糊）
//!
//! 八档按「变动意图 × 当前持仓」两轴定义，**互斥且穷尽** —— 任一记录只能落进一档：
//!
//! | 档 | 判据 | token | 中文 |
//! |---|---|---|---|
//! | [`ActionKind::Buy`] | 有变动意图 ∧ 目标仓位变动幅度 ≥ +T_hi | BUY | 买入 |
//! | [`ActionKind::Increase`] | 有变动意图 ∧ 0 < 变动幅度 < +T_hi | INCREASE | 增持 |
//! | [`ActionKind::Reduce`] | 有变动意图 ∧ -T_lo < 变动幅度 < 0 | REDUCE | 减持 |
//! | [`ActionKind::Sell`] | 有变动意图 ∧ 变动幅度 ≤ -T_lo | SELL | 卖出 |
//! | [`ActionKind::Hold`] | 无变动意图 ∧ **当前有仓位** | HOLD | 持有 |
//! | [`ActionKind::Wait`] | 无变动意图 ∧ **当前无仓位** | WAIT | 观望 |
//! | [`ActionKind::Uncertain`] | **有**决策数据但方向判不出 | UNCERTAIN | 不确定 |
//! | [`ActionKind::Unavailable`] | **无**决策数据（DB 空 / 取数链路失败） | UNAVAILABLE | 数据缺失 |
//!
//! 三条硬规矩（`scripts/check-decision-action-single-source.mjs` 按此对账五面载体）：
//!
//! 1. 兜底不得占用业务档名：「判不出」一律 `Uncertain`、「没数据」一律 `Unavailable`，
//!    不得写 `Wait` / `Hold` —— 那正是历史上「观望」一词同时表示「无仓位不加仓」与
//!    「先验不可得」两义的成因，也是 6 行 `持有+EMPTY` / `观望+HOLDING` 矛盾记录的成因。
//! 2. 后置规则（风控否决 / 封顶 / 降级）只允许改变「变动意图与幅度」，档名仍按上表两轴
//!    产出；改写来历走 `actionSource` 字段，由呈现层成句说明，不靠档名表达。
//! 3. 别名（加仓 / 减仓 / 清仓 / 强烈买入 / 英文 token…）是**解析入口**不是档位，
//!    只允许登记在本模块的 [`ACTION_ALIASES`] 与 [`normalize_action`] 里。

use serde::{Deserialize, Serialize};

/// 「决策动作缺失」的**显式哨兵**。
///
/// 历史实现用两种方式表达缺失：`unwrap_or_default()` 得到空串（前端解析后退化成
/// 「观望」）、以及硬编码 `"uncertain"`。两者都把「没有决策」伪装成业务语义 ——
/// 缺失不是结论。凡 DB 中 `decision_action` 为空，对外一律下发本哨兵，
/// 由展示层渲染「数据缺失 / 无法判断」而非任何操作建议。
pub const ACTION_UNAVAILABLE: &str = "UNAVAILABLE";

/// 决策 action 的规范档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    /// 买入（含 dashboard 值域的「强烈买入」）
    Buy,
    /// 增持（含「加仓」）
    Increase,
    /// 持有（有仓位且不打算变动）
    Hold,
    /// 观望（无仓位且不打算建仓）
    Wait,
    /// 减持（含「减仓」）
    Reduce,
    /// 卖出（含「强烈卖出」/ 清仓）
    Sell,
    /// 有决策数据但无法判断方向（解析失败 / 证据不足）
    Uncertain,
    /// 决策数据本身缺失
    Unavailable,
}

impl ActionKind {
    /// 规范英文 token。
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Increase => "INCREASE",
            Self::Hold => "HOLD",
            Self::Wait => "WAIT",
            Self::Reduce => "REDUCE",
            Self::Sell => "SELL",
            Self::Uncertain => "UNCERTAIN",
            Self::Unavailable => ACTION_UNAVAILABLE,
        }
    }

    /// 规范中文标签 —— 与 `portfolio-mgr.rhai` 的 6 档对齐。
    ///
    /// `Uncertain` / `Unavailable` 在公式值域里**没有对应档位**，返回 `None`：
    /// 它们不是操作建议，不应被塞进 6 档之一。
    pub fn as_cn(self) -> Option<&'static str> {
        match self {
            Self::Buy => Some("买入"),
            Self::Increase => Some("增持"),
            Self::Hold => Some("持有"),
            Self::Wait => Some("观望"),
            Self::Reduce => Some("减持"),
            Self::Sell => Some("卖出"),
            Self::Uncertain | Self::Unavailable => None,
        }
    }

    /// 写回存储 / 下发前端用的中文标签。
    ///
    /// 6 档用规范标签；`Uncertain` / `Unavailable` 用中文哨兵 —— 它们不是操作建议，
    /// 但必须以**可识别**的形态落库，不能借用「观望」的位置。
    pub fn as_storage_cn(self) -> &'static str {
        match self {
            Self::Buy => "买入",
            Self::Increase => "增持",
            Self::Hold => "持有",
            Self::Wait => "观望",
            Self::Reduce => "减持",
            Self::Sell => "卖出",
            Self::Uncertain => "不确定",
            Self::Unavailable => "数据缺失",
        }
    }

    /// 是否隐含「持有多头」（风控否决的判定对象）。
    pub fn implies_long(self) -> bool {
        matches!(self, Self::Buy | Self::Increase | Self::Hold)
    }
    /// 是否隐含「建仓 / 加仓」意图。
    pub fn implies_buy(self) -> bool {
        matches!(self, Self::Buy | Self::Increase)
    }
    /// 是否隐含「降低多头敞口」意图（减持 / 卖出）。
    ///
    /// 与 [`Self::implies_buy`] 对称且互斥（同一档不可能两者皆真）。此前消费端各自写
    /// `matches!(action, "卖出" | "减持")`，只认中文 —— 链路上 action 一旦是英文 token
    /// 或别名（`SELL` / `clear` / `强烈卖出`），回环否决与丢档判定**静默不触发**
    /// （与模块头记的 `apply_risk_veto` fail-open 同一失效机制）。
    pub fn implies_reduce(self) -> bool {
        matches!(self, Self::Reduce | Self::Sell)
    }

    /// 是否为「没有操作意图」的档位（观望 / 不确定 / 缺失）。
    pub fn is_no_action(self) -> bool {
        matches!(self, Self::Wait | Self::Uncertain | Self::Unavailable)
    }
}

/// 全部可识别别名（用于「宽松匹配」场景）。
///
/// 存在的意义：部分消费端面对的是 **LLM 自由文本**（如「买入（分批建仓）」），
/// 严格 normalize_action 会返回 None，但该值域显然合法。这类消费端需要
/// 「至少包含一个已知别名」的宽松判据 —— 别名集合必须来自本模块，
/// 不得各消费端自写一份（那正是「同名语义散成多套值域」的复发路径）。
///
/// 比较前调用方须先 to_lowercase；中文项不受影响。
pub const ACTION_ALIASES: &[&str] = &[
    "buy",
    "strong_buy",
    "买入",
    "强烈买入",
    "increase",
    "add",
    "增持",
    "加仓",
    "hold",
    "持有",
    "wait",
    "watch",
    "观望",
    "等待",
    "reduce",
    "trim",
    "减持",
    "减仓",
    "sell",
    "strong_sell",
    "clear",
    "卖出",
    "强烈卖出",
    "uncertain",
    "不确定",
    "无法判断",
    "unavailable",
    "数据缺失",
];

/// 把任意来源的 action 字符串归一化为 [`ActionKind`]。
///
/// 接受中英文、大小写不敏感、首尾空白容错。
///
/// **不接受的输入返回 `None`** —— 调用方必须显式处理该分支。历史上正是
/// 「未识别就当成观望 / 直接放行」这类默认值造成了 fail-open 与错档兜底。
pub fn normalize_action(raw: &str) -> Option<ActionKind> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.to_ascii_uppercase().as_str() {
        "买入" | "BUY" | "STRONG_BUY" | "强烈买入" => Some(ActionKind::Buy),
        "增持" | "INCREASE" | "ADD" | "加仓" => Some(ActionKind::Increase),
        "持有" | "HOLD" => Some(ActionKind::Hold),
        "观望" | "等待" | "WAIT" | "WATCH" => Some(ActionKind::Wait),
        "减持" | "减仓" | "REDUCE" | "TRIM" => Some(ActionKind::Reduce),
        "卖出" | "SELL" | "CLEAR" | "STRONG_SELL" | "强烈卖出" => Some(ActionKind::Sell),
        "UNCERTAIN" | "不确定" | "无法判断" => Some(ActionKind::Uncertain),
        "UNAVAILABLE" | "数据缺失" => Some(ActionKind::Unavailable),
        _ => None,
    }
}

/// 归一化后转回规范中文标签；非 6 档值（Uncertain / Unavailable / 未识别）返回 `None`。
pub fn normalize_to_cn(raw: &str) -> Option<&'static str> {
    normalize_action(raw).and_then(ActionKind::as_cn)
}

/// verdict（方向结论）→ action（操作档位）的**降维**映射。
///
/// 依据 `trader.md` 的一致性表：「买入 / 增持 → 看多」「卖出 / 减持 → 看空」
/// 「持有 / 观望 → 中性」。反向映射有损（一对二），因此只在 LLM 未给结构化
/// `action` 字段时作为兜底使用。
///
/// ⚠️ `"不确定" / "无法判断"` 映射为 [`ActionKind::Uncertain`] **而非**「观望」：
/// 方向未知与「判断为中性」是两回事，把前者压成后者等于替 LLM 下了一个它没下的结论。
pub fn verdict_to_action(verdict: &str) -> Option<ActionKind> {
    match verdict.trim() {
        "看多" | "看涨" | "做多" | "多头" | "BUY" => Some(ActionKind::Buy),
        "看空" | "看跌" | "做空" | "空头" | "SELL" => Some(ActionKind::Sell),
        "中性" | "震荡" | "持有" | "HOLD" => Some(ActionKind::Hold),
        "不确定" | "无法判断" | "无法确定" | "UNCERTAIN" => Some(ActionKind::Uncertain),
        "观望" | "WAIT" => Some(ActionKind::Wait),
        _ => None,
    }
}

/// 是否为「决策缺失」哨兵（大小写 / 空白容错）。
pub fn is_unavailable(raw: &str) -> bool {
    matches!(normalize_action(raw), Some(ActionKind::Unavailable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 中英文值域归一化到同一档位() {
        assert_eq!(normalize_action("买入"), Some(ActionKind::Buy));
        assert_eq!(normalize_action("BUY"), Some(ActionKind::Buy));
        assert_eq!(normalize_action("  buy "), Some(ActionKind::Buy));
        assert_eq!(normalize_action("强烈买入"), Some(ActionKind::Buy));
        assert_eq!(normalize_action("观望"), Some(ActionKind::Wait));
        assert_eq!(normalize_action("WAIT"), Some(ActionKind::Wait));
        assert_eq!(normalize_action("减持"), Some(ActionKind::Reduce));
        assert_eq!(normalize_action("TRIM"), Some(ActionKind::Reduce));
    }

    #[test]
    fn 未识别值返回_none_而不是默认档位() {
        // 关键：不得退化成「观望」—— 那正是 fail-open 的来源
        assert_eq!(normalize_action(""), None);
        assert_eq!(normalize_action("   "), None);
        assert_eq!(normalize_action("待明日观察"), None);
        assert_eq!(normalize_action("规避"), None);
    }

    #[test]
    fn 缺失哨兵与不确定互不混淆() {
        assert!(is_unavailable("UNAVAILABLE"));
        assert!(is_unavailable("数据缺失"));
        assert!(!is_unavailable("不确定"));
        assert!(!is_unavailable("观望"));
    }

    #[test]
    fn 中文标签只在_6_档内返回() {
        assert_eq!(normalize_to_cn("HOLD"), Some("持有"));
        assert_eq!(normalize_to_cn("UNCERTAIN"), None);
        assert_eq!(normalize_to_cn(ACTION_UNAVAILABLE), None);
    }

    #[test]
    fn 别名集覆盖全部可识别取值() {
        // 正对照：normalize_action 能识别的值，其小写形态必须出现在 ACTION_ALIASES 中
        for raw in [
            "买入",
            "BUY",
            "增持",
            "INCREASE",
            "持有",
            "HOLD",
            "观望",
            "WAIT",
            "减持",
            "REDUCE",
            "卖出",
            "SELL",
            "UNCERTAIN",
            "UNAVAILABLE",
        ] {
            let low = raw.to_lowercase();
            assert!(ACTION_ALIASES.iter().any(|a| low.contains(a)), "别名集遗漏 {raw}");
        }
        // 负对照：无关文本不得命中
        let junk = "待明日观察".to_lowercase();
        assert!(!ACTION_ALIASES.iter().any(|a| junk.contains(a)));
    }

    #[test]
    fn 语义谓词覆盖各档() {
        assert!(ActionKind::Buy.implies_long());
        assert!(ActionKind::Hold.implies_long());
        assert!(!ActionKind::Wait.implies_long());
        assert!(ActionKind::Increase.implies_buy());
        assert!(!ActionKind::Hold.implies_buy());
        assert!(ActionKind::Unavailable.is_no_action());
    }

    /// 「建仓」与「降敞口」两谓词必须互斥，且英文 token / 别名经归一后同样命中 ——
    /// 消费端因此只需调谓词，不必再各自 `matches!(action, "卖出" | "减持")` 比字面量。
    #[test]
    fn 买卖谓词互斥且别名同样命中() {
        let all = [
            ActionKind::Buy,
            ActionKind::Increase,
            ActionKind::Hold,
            ActionKind::Wait,
            ActionKind::Reduce,
            ActionKind::Sell,
            ActionKind::Uncertain,
            ActionKind::Unavailable,
        ];
        for k in all {
            assert!(!(k.implies_buy() && k.implies_reduce()), "{:?} 同时命中建仓与降敞口", k);
        }
        assert!(ActionKind::Sell.implies_reduce());
        assert!(ActionKind::Reduce.implies_reduce());
        assert!(!ActionKind::Hold.implies_reduce());
        assert!(!ActionKind::Wait.implies_reduce());
        for raw in ["SELL", "clear", "强烈卖出", "TRIM", "减仓"] {
            assert_eq!(normalize_action(raw).map(ActionKind::implies_reduce), Some(true), "{raw}");
        }
        for raw in ["BUY", "加仓", "strong_buy"] {
            assert_eq!(normalize_action(raw).map(ActionKind::implies_buy), Some(true), "{raw}");
        }
    }

    /// 裁定「分档不得重叠」的机器化形式：八档的 token 与中文标签都必须**一一对应**，
    /// 任何两档共用同一个 token 或同一个中文，界面与库就会出现「同一个词两义」。
    #[test]
    fn 八档的token与中文标签一一对应无重复() {
        let all = [
            ActionKind::Buy,
            ActionKind::Increase,
            ActionKind::Hold,
            ActionKind::Wait,
            ActionKind::Reduce,
            ActionKind::Sell,
            ActionKind::Uncertain,
            ActionKind::Unavailable,
        ];
        let tokens: Vec<&str> = all.iter().map(|k| k.as_token()).collect();
        let cn: Vec<&str> = all.iter().map(|k| k.as_storage_cn()).collect();
        assert_eq!(
            tokens.iter().collect::<std::collections::HashSet<_>>().len(),
            8,
            "token 有重复档"
        );
        assert_eq!(
            cn.iter().collect::<std::collections::HashSet<_>>().len(),
            8,
            "中文标签有重复档"
        );
        // 六档「操作建议」的 as_cn 必须与 as_storage_cn 同字；两哨兵档必须无 as_cn
        // （as_cn 有值 = 会被当成一条可执行建议下发）
        for k in all {
            match k.as_cn() {
                Some(label) => assert_eq!(label, k.as_storage_cn(), "{:?} 两套中文不一致", k),
                None => assert!(
                    matches!(k, ActionKind::Uncertain | ActionKind::Unavailable),
                    "{:?} 是操作档却没有中文，值域清单会漏它",
                    k
                ),
            }
        }
    }

    /// 裁定「兜底不得占用业务档名」：判不出与没数据各有自己的档，且都不隐含持仓方向。
    #[test]
    fn 兜底两档不占业务档名() {
        assert_eq!(normalize_action("无法判断"), Some(ActionKind::Uncertain));
        assert_eq!(normalize_action("数据缺失"), Some(ActionKind::Unavailable));
        for k in [ActionKind::Uncertain, ActionKind::Unavailable] {
            assert!(k.is_no_action(), "{:?} 不得被当成有操作意图", k);
            assert!(!k.implies_long(), "{:?} 不得隐含持有多头", k);
            assert!(!k.implies_buy(), "{:?} 不得隐含建仓意图", k);
        }
        // 无法识别的值必须返回 None 由调用方显式处置，不得默认落到「观望」
        assert_eq!(normalize_action("待明日观察"), None);
    }

    /// 两轴定义的可判性：持有隐含有多头、观望不隐含 —— 这正是「无变动意图」两档的分流依据。
    #[test]
    fn 持有与观望按持仓状态分流() {
        assert!(ActionKind::Hold.implies_long());
        assert!(!ActionKind::Wait.implies_long());
        assert!(!ActionKind::Hold.implies_buy());
        assert!(!ActionKind::Wait.implies_buy());
    }
}
