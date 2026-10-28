//! 条件单引擎 — 规则评估（纯判据，不碰数据库、不读墙上时钟）
//!
//! ## 设计
//!
//! 条件单 (ConditionalOrder) 是一条 if-then 规则：条件满足 → 产出动作（交易意图或通知）。
//! 定义的唯一权威是新表 `conditional_orders`（`axagent_entities::conditional_orders`），
//! 本模块只负责「给一个行情信号 + 一条规则 ⇒ 该怎么判」。
//!
//! ## 为什么判据要拆成纯函数
//!
//! 上一版把三件事写在了一起，每一件都必须有墙上时钟或数据源才能验，于是全都验不到：
//!
//! - 时间窗用 `chrono::Local::now()`（机器时区）拼 `"HH:MM"` 再做**字符串**比较，而调用方
//!   传进来的 `now_ms` 是 `Utc` —— 两个「现在」；判据本体又要求 `active_start`/`active_end`
//!   **同时**有值才生效，而所有构造点都传 `None/None` ⇒ 整条分支恒不成立（死分支）。
//! - 前收盘为 0 时除法得 NaN，所有比较都为假 ⇒ 静默判成「条件未满足」。
//! - 触发计数标在 `#[serde(skip)]` 上，每次加载归零且永不按日滚动 ⇒ 冷却与单日上限
//!   在实时路径上从未真正生效。
//!
//! 现在时间基准由调用方（桥接层）以 [`QuoteSignal`] 显式交进来，缺输入一律走
//! [`TriggerDecision::Indeterminate`] —— 「拿不到」不得读成「没触发」。

use serde::{Deserialize, Serialize};

/// 条件触发后的执行动作
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum OrderAction {
    /// 买入（股数）
    Buy { quantity: i32 },
    /// 卖出（`None` = 全部持仓，不是缺值）
    Sell { quantity: Option<i32> },
    /// 减仓（比例，0.0–1.0）
    Reduce { ratio: f64 },
    /// 仅发送通知
    Notify,
}

/// 条件类型 —— 每一类都自带用户给的阈值
///
/// 与退役掉的两个形态的区别：
/// - 价格类不再用「前收盘 × 固定百分比」冒充用户设的价（旧值是 7% 止损 / 15% 止盈 / 5% 突破）。
/// - `MACross`（需要日 K 序列）与 `Custom`（需要 Rhai）整体退役：引擎里留一个恒不成立的臂，
///   就是「声明了但永远不工作」的假旋钮。要接回来得先给它们的数据源接线，
///   创建口此刻会以显式错误拒绝这两个类别（见 `src-tauri/src/commands/conditional_order.rs`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConditionType {
    /// 现价跌破该价
    StopLoss { price: f64 },
    /// 现价突破该价
    TakeProfit { price: f64 },
    /// 现价上穿该压力位
    ResistanceBreak { price: f64 },
    /// 现价下穿该支撑位
    SupportBreak { price: f64 },
    /// 当日涨跌幅绝对值超过该百分比
    ChangePct { threshold: f64 },
    /// 换手率超过该百分比
    TurnoverRate { threshold: f64 },
}

impl ConditionType {
    /// 与新表 `condition_type` 列的取值一一对应（列里存这个串，读回时按它重建枚举）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StopLoss { .. } => "stop_loss",
            Self::TakeProfit { .. } => "take_profit",
            Self::ResistanceBreak { .. } => "resistance_break",
            Self::SupportBreak { .. } => "support_break",
            Self::ChangePct { .. } => "change_pct",
            Self::TurnoverRate { .. } => "turnover_rate",
        }
    }

    /// 阈值：价格类是绝对价，比率类是百分比数值。与 `as_str()` 成对，落库/读回共用一套口径。
    pub fn threshold_value(&self) -> f64 {
        match self {
            Self::StopLoss { price }
            | Self::TakeProfit { price }
            | Self::ResistanceBreak { price }
            | Self::SupportBreak { price } => *price,
            Self::ChangePct { threshold } | Self::TurnoverRate { threshold } => *threshold,
        }
    }

    /// 由 `(类别串, 阈值)` 重建。类别未知 ⇒ `None`（调用方负责报错，不静默兜成涨跌幅类）。
    pub fn from_stored(kind: &str, threshold: Option<f64>) -> Option<Self> {
        let value = threshold?;
        let parsed = match kind {
            "stop_loss" => Self::StopLoss { price: value },
            "take_profit" => Self::TakeProfit { price: value },
            "resistance_break" => Self::ResistanceBreak { price: value },
            "support_break" => Self::SupportBreak { price: value },
            "change_pct" => Self::ChangePct { threshold: value },
            "turnover_rate" => Self::TurnoverRate { threshold: value },
            _ => return None,
        };
        Some(parsed)
    }
}

/// 单条件单（内存态；与 `conditional_orders` 表一行同构）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalOrder {
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub condition: ConditionType,
    pub action: OrderAction,
    pub enabled: bool,
    /// 生效时间窗（北京时间 `"HH:MM"`）。任一侧存在即生效，缺的那侧按当日边界补；
    /// 两侧都缺 = 全天。
    pub active_start: Option<String>,
    pub active_end: Option<String>,
    /// 单日触发上限（`0` = 无限制，按北京日计）
    pub max_triggers_per_day: u32,
    /// 两次触发之间的最小间隔（分钟）
    pub cool_down_minutes: u32,
    /// 当日已触发次数（**必须与 `trigger_date` 成对读**：日期不是今天就视作 0）
    pub today_trigger_count: u32,
    /// `today_trigger_count` 归属的北京日（`YYYY-MM-DD`）
    pub trigger_date: Option<String>,
    /// 最后触发时刻（UTC ms）
    pub last_triggered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Default for ConditionalOrder {
    fn default() -> Self {
        Self {
            id: String::new(),
            stock_code: String::new(),
            stock_name: String::new(),
            condition: ConditionType::ChangePct { threshold: 3.0 },
            action: OrderAction::Notify,
            enabled: true,
            active_start: None,
            active_end: None,
            max_triggers_per_day: 3,
            cool_down_minutes: 30,
            today_trigger_count: 0,
            trigger_date: None,
            last_triggered_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }
}

/// 一次判定的输入信号。
///
/// 时间基准全部由调用方交进来（引擎自己不读墙上时钟）：`beijing_*` 两个字段来自
/// `astock_data::calendar::beijing_now()`，与告警冷却、复盘窗口同一个口径。
#[derive(Debug, Clone)]
pub struct QuoteSignal {
    pub current_price: f64,
    pub prev_close: f64,
    pub turnover_rate: Option<f64>,
    /// 判定时刻（UTC ms），冷却窗口用它
    pub now_ms: i64,
    /// 北京时间 `YYYY-MM-DD`，单日计数归属它
    pub beijing_date: String,
    /// 北京时间的当日分钟数，生效时段用它
    pub beijing_minutes_of_day: u32,
}

/// 判定结论
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerDecision {
    /// 条件成立且允许动作
    Trigger,
    /// **无法判定**：输入缺失或无效。调用方必须把它与「条件未满足」分开呈现，
    /// 否则「拿不到数据」会被读成「风控说没问题」。
    Indeterminate(String),
    /// 判定成立但被拦下不动作（禁用 / 冷却中 / 当日已达上限 / 不在生效时段 / 条件未满足）
    Suppressed(String),
}

/// `"HH:MM"` → 北京分钟数。格式不合法 ⇒ `None`（不得静默当成 0 点）。
fn parse_hhmm(value: &str) -> Option<u32> {
    let (hour, minute) = value.split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour * 60 + minute)
}

/// 当日分钟数 → `"HH:MM"`。右界 1440 显示成 `24:00`，读得出「当日结束」而不是绕回 00:00。
fn clock_text(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

impl ConditionalOrder {
    /// 当日生效计数：`trigger_date` 不是判定时的那条北京日 ⇒ 从 0 起算（跨日滚动）。
    pub fn effective_today_count(&self, beijing_date: &str) -> u32 {
        match self.trigger_date.as_deref() {
            Some(date) if date == beijing_date => self.today_trigger_count,
            _ => 0,
        }
    }

    /// 记下一次触发：跨日先把计数清零再累加，同时落时刻。
    ///
    /// 单独成一个方法而不是让调用方改字段，是因为旧形态正是「调用方改了自己的副本、
    /// 引擎里那份恒 0」⇒ 冷却与单日上限从来没真正生效过。
    pub fn note_triggered(&mut self, beijing_date: &str, now_ms: i64) {
        if self.trigger_date.as_deref() != Some(beijing_date) {
            self.today_trigger_count = 0;
            self.trigger_date = Some(beijing_date.to_string());
        }
        self.today_trigger_count += 1;
        self.last_triggered_at = Some(now_ms);
    }

    /// 生效时段判据。三种结论：`Within` / `Outside(reason)` / `BadWindow(reason)`。
    /// 抽成独立方法是为了让「单侧缺省」「跨日矛盾」「格式非法」三种形态各自可测。
    pub fn window_state(&self, minutes_of_day: u32) -> WindowState {
        let start = match self.active_start.as_deref() {
            None => None,
            Some(raw) => match parse_hhmm(raw) {
                Some(m) => Some(m),
                None => {
                    return WindowState::BadWindow(format!(
                        "生效时段起点格式非法（应为 HH:MM）：{raw}"
                    ));
                },
            },
        };
        let end = match self.active_end.as_deref() {
            None => None,
            Some(raw) => match parse_hhmm(raw) {
                Some(m) => Some(m),
                None => {
                    return WindowState::BadWindow(format!(
                        "生效时段终点格式非法（应为 HH:MM）：{raw}"
                    ));
                },
            },
        };
        let (lo, hi) = match (start, end) {
            (None, None) => return WindowState::Within,
            (Some(s), None) => (s, 24 * 60),
            (None, Some(e)) => (0, e),
            (Some(s), Some(e)) if e < s => {
                return WindowState::BadWindow(format!(
                    "生效时段终点早于起点：{} → {}",
                    clock_text(s),
                    clock_text(e)
                ));
            },
            (Some(s), Some(e)) => (s, e),
        };
        if minutes_of_day >= lo && minutes_of_day <= hi {
            WindowState::Within
        } else {
            WindowState::Outside(format!("不在生效时段 [{} – {}]", clock_text(lo), clock_text(hi)))
        }
    }

    /// 该不该触发。闸门顺序即优先级：禁用 → 无法判定（缺输入）→ 时间窗 → 冷却 → 单日上限 → 条件本体。
    ///
    /// 「无法判定」排在条件本体之前、闸门之前只排在禁用之后：前收盘坏掉或换手率拿不到时，
    /// 报「冷却中/未满足」都是假话，而报「无法判定」才能让下游知道自己这一轮是瞎的。
    pub fn judge(&self, signal: &QuoteSignal) -> TriggerDecision {
        if !self.enabled {
            return TriggerDecision::Suppressed("已禁用".into());
        }
        if let Some(reason) = self.input_gap(signal) {
            return TriggerDecision::Indeterminate(reason);
        }
        match self.window_state(signal.beijing_minutes_of_day) {
            WindowState::Within => {},
            // 时段没到 = 判定得出、只是不该动；配置矛盾 = 根本判不出该不该动，两者不能同串
            WindowState::Outside(reason) => return TriggerDecision::Suppressed(reason),
            WindowState::BadWindow(reason) => return TriggerDecision::Indeterminate(reason),
        }
        if let Some(last) = self.last_triggered_at {
            let elapsed_min = (signal.now_ms - last) / 60_000;
            if elapsed_min < self.cool_down_minutes as i64 {
                return TriggerDecision::Suppressed(format!(
                    "冷却中（已过 {}m，需 {}m）",
                    elapsed_min, self.cool_down_minutes
                ));
            }
        }
        let today = self.effective_today_count(&signal.beijing_date);
        if self.max_triggers_per_day > 0 && today >= self.max_triggers_per_day {
            return TriggerDecision::Suppressed(format!(
                "当日触发已达上限（{} 次）",
                self.max_triggers_per_day
            ));
        }

        match self.condition_meets(signal) {
            Ok(true) => TriggerDecision::Trigger,
            Ok(false) => TriggerDecision::Suppressed("条件未满足".into()),
            // input_gap 已先拦，这里不可达；仍如实传出不写成「未满足」
            Err(reason) => TriggerDecision::Indeterminate(reason),
        }
    }

    /// 判据本体。`Err` = 该条件所需输入拿不到（与「未满足」严格区分）。
    fn condition_meets(&self, signal: &QuoteSignal) -> Result<bool, String> {
        match self.condition {
            ConditionType::StopLoss { price } => Ok(signal.current_price <= price),
            ConditionType::TakeProfit { price } => Ok(signal.current_price >= price),
            ConditionType::ResistanceBreak { price } => Ok(signal.current_price > price),
            ConditionType::SupportBreak { price } => Ok(signal.current_price < price),
            ConditionType::ChangePct { threshold } => {
                Ok(change_pct(signal.current_price, signal.prev_close)? >= threshold)
            },
            ConditionType::TurnoverRate { threshold } => match signal.turnover_rate {
                Some(rate) => Ok(rate >= threshold),
                None => Err("该数据源不返回换手率，无法判定".into()),
            },
        }
    }

    /// 输入缺口体检（先于闸门跑）。
    fn input_gap(&self, signal: &QuoteSignal) -> Option<String> {
        if !signal.current_price.is_finite() || signal.current_price <= 0.0 {
            return Some("现价无效（≤0 或非有限值），本轮无法判定".into());
        }
        if matches!(self.condition, ConditionType::ChangePct { .. })
            && (signal.prev_close <= 0.0 || !signal.prev_close.is_finite())
        {
            return Some("前收盘无效（≤0 或非有限值），涨跌幅无法判定".into());
        }
        None
    }
}

/// 涨跌幅百分比绝对值（北京口径的「当日较前收盘」）。除零/NaN 都走 `Err`。
pub fn change_pct(current: f64, prev_close: f64) -> Result<f64, String> {
    if prev_close <= 0.0 || !prev_close.is_finite() || !current.is_finite() {
        return Err("前收盘或现价无效，涨跌幅无法计算".into());
    }
    Ok(((current - prev_close) / prev_close * 100.0).abs())
}

/// 生效时段三态
#[derive(Debug, Clone, PartialEq)]
pub enum WindowState {
    Within,
    Outside(String),
    /// 配置本身矛盾或格式非法 —— 不能当成「今天不生效」
    BadWindow(String),
}

/// 条件单引擎
#[derive(Default)]
pub struct ConditionalOrderEngine {
    orders: Vec<ConditionalOrder>,
}

impl ConditionalOrderEngine {
    pub fn new() -> Self {
        Self { orders: Vec::new() }
    }

    /// 设置条件单列表（全量替换；定义权威在新表，这里只是内存镜像）
    pub fn set_orders(&mut self, orders: Vec<ConditionalOrder>) {
        self.orders = orders;
    }

    pub fn orders(&self) -> &[ConditionalOrder] {
        &self.orders
    }

    /// 记一次触发并返回更新后的规则（调用方据此落库）。找不到该 id ⇒ `None`。
    ///
    /// 计数必须在**引擎这一份**上加：旧实现改的是调用方 clone 出来的 map，引擎里那份恒 0，
    /// 于是冷却与单日上限在实时路径上永不生效。
    pub fn record_trigger(
        &mut self,
        order_id: &str,
        beijing_date: &str,
        now_ms: i64,
    ) -> Option<ConditionalOrder> {
        let order = self.orders.iter_mut().find(|o| o.id == order_id)?;
        order.note_triggered(beijing_date, now_ms);
        Some(order.clone())
    }

    /// 评估某只股票的一条行情，返回该股票每条规则的结论（含未触发的原因）。
    ///
    /// 返回全部结论而不是只返触发的子集：调用方要把「无法判定」和「条件未满足」分开呈现，
    /// 只给触发集合就等于把这两种缺席又压成了一个读数。
    pub fn evaluate<'a>(&'a self, stock_code: &str, signal: &QuoteSignal) -> Vec<Evaluated<'a>> {
        self.orders
            .iter()
            .filter(|o| o.stock_code == stock_code)
            .map(|o| Evaluated { decision: o.judge(signal), order: o })
            .collect()
    }
}

/// 一条规则的评估结论
#[derive(Debug, Clone)]
pub struct Evaluated<'a> {
    pub order: &'a ConditionalOrder,
    pub decision: TriggerDecision,
}

impl Evaluated<'_> {
    pub fn is_trigger(&self) -> bool {
        self.decision == TriggerDecision::Trigger
    }
}

/// 一次触发产出的事实 —— 它同时是 IPC payload 与落库/建意图的素材。
///
/// 取代旧形态里 `format!("{:?}", order.condition)` 的 Rust `Debug` 串（那份曾直接端给前端），
/// 所有描述都由 [`ConditionType::describe`] / [`OrderAction::describe`] 给成人话。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConditionTrigger {
    pub order_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub condition: String,
    pub action: String,
    pub price: f64,
    pub change_pct: f64,
    /// 触发时刻（UTC ms）
    pub triggered_at_ms: i64,
    /// 判定依据（触发时的条件描述，供面板与复盘直读）
    pub reason: String,
}

impl ConditionType {
    /// 给用户看的一句条件描述（带单位）。
    pub fn describe(&self) -> String {
        match self {
            Self::StopLoss { price } => format!("跌破止损价 {price:.2}"),
            Self::TakeProfit { price } => format!("突破止盈价 {price:.2}"),
            Self::ResistanceBreak { price } => format!("上穿压力位 {price:.2}"),
            Self::SupportBreak { price } => format!("下穿支撑位 {price:.2}"),
            Self::ChangePct { threshold } => format!("当日涨跌幅达 {threshold:.2}%"),
            Self::TurnoverRate { threshold } => format!("换手率达 {threshold:.2}%"),
        }
    }
}

impl OrderAction {
    /// 给用户看的一句动作描述。`sell` 的 `None` 是「卖出全部持仓」这个确定语义，不是缺值。
    pub fn describe(&self) -> String {
        match self {
            Self::Buy { quantity } => format!("买入 {quantity} 股"),
            Self::Sell { quantity: Some(q) } => format!("卖出 {q} 股"),
            Self::Sell { quantity: None } => "卖出全部持仓".to_string(),
            Self::Reduce { ratio } => format!("减仓 {:.0}%", ratio * 100.0),
            Self::Notify => "仅通知".to_string(),
        }
    }

    /// 是否只发通知（通知不生成交易意图）
    pub fn is_notify_only(&self) -> bool {
        matches!(self, Self::Notify)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: &str = "2026-10-15";

    /// 十分钟一个刻度：`09:30` = 570
    fn signal(price: f64, prev_close: f64, minutes: u32) -> QuoteSignal {
        QuoteSignal {
            current_price: price,
            prev_close,
            turnover_rate: Some(8.0),
            now_ms: 1_760_000_000_000,
            beijing_date: DAY.to_string(),
            beijing_minutes_of_day: minutes,
        }
    }

    fn order(condition: ConditionType) -> ConditionalOrder {
        ConditionalOrder {
            id: "co_1".into(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            condition,
            action: OrderAction::Notify,
            ..Default::default()
        }
    }

    /// 阶段 F-2 的正主：价格类条件用**用户给的价**判，不再拿前收盘 × 固定百分比冒充。
    /// 前收盘故意给成两个极端值 —— 结论必须一模一样，否则就是旧形态的耦合复发。
    #[test]
    fn price_conditions_ignore_prev_close() {
        let stop = order(ConditionType::StopLoss { price: 12.0 });
        assert_eq!(stop.judge(&signal(11.9, 100.0, 600)), TriggerDecision::Trigger);
        assert_eq!(stop.judge(&signal(11.9, 11.0, 600)), TriggerDecision::Trigger);
        assert_eq!(
            stop.judge(&signal(12.5, 100.0, 600)),
            TriggerDecision::Suppressed("条件未满足".into())
        );

        let take = order(ConditionType::TakeProfit { price: 20.0 });
        assert_eq!(take.judge(&signal(20.0, 3.0, 600)), TriggerDecision::Trigger);
        assert_ne!(take.judge(&signal(19.99, 3.0, 600)), TriggerDecision::Trigger);
    }

    /// 判据三态的分工要能被调用方区分开，故这里逐态验一次类型而不是只验字符串。
    #[test]
    fn indeterminate_and_suppressed_are_different_types_not_two_spellings_of_one_thing() {
        let change = order(ConditionType::ChangePct { threshold: 3.0 });
        // 前收盘为 0 ⇒ 除法无意义：必须是「无法判定」，不得读成「条件未满足」
        assert!(matches!(change.judge(&signal(10.5, 0.0, 600)), TriggerDecision::Indeterminate(_)));
        // NaN 同理（`is_finite` 拦住，不能让所有比较静默为假）
        assert!(matches!(
            change.judge(&signal(10.5, f64::NAN, 600)),
            TriggerDecision::Indeterminate(_)
        ));

        let turnover = order(ConditionType::TurnoverRate { threshold: 10.0 });
        let mut blind = signal(10.0, 10.0, 600);
        blind.turnover_rate = None;
        match turnover.judge(&blind) {
            TriggerDecision::Indeterminate(reason) => {
                assert!(reason.contains("换手率"), "要点名缺的是哪个输入：{reason}");
            },
            other => panic!("换手率拿不到必须是无法判定，实测 {other:?}"),
        }
    }

    /// 时间窗：单侧给值就生效、缺的那侧补到当日边界；两侧都不给 = 全天。
    #[test]
    fn window_accepts_one_sided_and_absent_bounds() {
        let mut o = order(ConditionType::StopLoss { price: 12.0 });
        o.active_start = Some("09:30".into());
        o.active_end = None;
        assert!(
            matches!(
                o.judge(&signal(11.0, 11.0, 540)),
                TriggerDecision::Suppressed(reason) if reason.contains("不在生效时段")
            ),
            "09:00 还没进窗口"
        );
        match o.judge(&signal(11.0, 11.0, 570)) {
            TriggerDecision::Trigger => {},
            other => panic!("09:30 起生效，570 分应当放行：{other:?}"),
        }
        // 缺的那一侧必须真补到当日边界：11:00 与 23:00 都还在生效区内。
        // 实测把右界从 24:00 收成起点自身时，只有这两格会红 —— 上面 09:30 那点探不出来。
        for minutes in [660, 1380] {
            assert_eq!(
                o.judge(&signal(11.0, 11.0, minutes)),
                TriggerDecision::Trigger,
                "只给起点 ⇒ 右界补到当日结束（探第 {minutes} 分）"
            );
        }

        o.active_start = None;
        o.active_end = Some("15:00".into());
        assert_eq!(
            o.judge(&signal(11.0, 11.0, 540)),
            TriggerDecision::Trigger,
            "只给终点 ⇒ 左界补到 00:00，凌晨也算在窗内"
        );
        assert!(matches!(
            o.judge(&signal(11.0, 11.0, 15 * 60 + 1)),
            TriggerDecision::Suppressed(reason) if reason.contains("不在生效时段")
        ));

        o.active_end = None;
        assert!(matches!(o.window_state(3 * 60), WindowState::Within), "凌晨无窗 = 全天生效");
    }

    /// 配置矛盾与格式非法不能被读成「今天不生效」—— 必须是无法判定。
    #[test]
    fn broken_window_is_indeterminate_not_a_quiet_skip() {
        let mut o = order(ConditionType::StopLoss { price: 12.0 });
        o.active_start = Some("21:00".into());
        o.active_end = Some("02:00".into());
        assert!(matches!(o.judge(&signal(11.0, 11.0, 600)), TriggerDecision::Indeterminate(_)));

        o.active_start = Some("9:70".into());
        assert!(matches!(
            o.window_state(600),
            WindowState::BadWindow(_) // 分钟越界，不得静默当 0 点
        ));
    }

    /// 单日计数按北京日滚动：昨天的 3 次不吃今天的额度（旧形态计数被 skip 且永不滚动）。
    #[test]
    fn daily_cap_rolls_over_on_the_beijing_date() {
        let mut o = order(ConditionType::StopLoss { price: 12.0 });
        o.max_triggers_per_day = 3;
        o.today_trigger_count = 3;
        o.trigger_date = Some("2026-10-14".into());
        assert_eq!(o.effective_today_count(DAY), 0, "计数日期不是今天就归零");
        assert_eq!(o.judge(&signal(11.0, 11.0, 600)), TriggerDecision::Trigger);

        o.trigger_date = Some(DAY.to_string());
        assert!(matches!(
            o.judge(&signal(11.0, 11.0, 600)),
            TriggerDecision::Suppressed(reason) if reason.contains("上限")
        ));
    }

    #[test]
    fn cooldown_and_disable_are_suppressions_with_their_own_reasons() {
        let mut o = order(ConditionType::StopLoss { price: 12.0 });
        o.cool_down_minutes = 30;
        o.last_triggered_at = Some(1_760_000_000_000 - 10 * 60_000);
        assert!(matches!(
            o.judge(&signal(11.0, 11.0, 600)),
            TriggerDecision::Suppressed(reason) if reason.contains("冷却")
        ));

        o.last_triggered_at = None;
        o.enabled = false;
        assert!(matches!(
            o.judge(&signal(11.0, 11.0, 600)),
            TriggerDecision::Suppressed(reason) if reason == "已禁用"
        ));
    }

    /// 引擎返回**全部**结论而不是只返触发的子集：没触发的原因得留给呈现层分列。
    #[test]
    fn evaluate_returns_every_verdict_for_that_stock() {
        let mut engine = ConditionalOrderEngine::new();
        let mut other = order(ConditionType::StopLoss { price: 12.0 });
        other.id = "co_2".into();
        other.stock_code = "000001".into();
        let mut miss = order(ConditionType::StopLoss { price: 9.5 });
        miss.id = "co_3".into();
        engine.set_orders(vec![order(ConditionType::StopLoss { price: 12.0 }), miss, other]);

        let outcomes = engine.evaluate("600519", &signal(11.0, 11.0, 600));
        assert_eq!(outcomes.len(), 2, "同股两条都要出现在结论里");
        assert_eq!(outcomes.iter().filter(|e| e.is_trigger()).count(), 1);
        assert!(
            outcomes.iter().any(|e| e.decision == TriggerDecision::Suppressed("条件未满足".into())),
            "未触发那条要带着自己的原因回来"
        );
    }

    /// `(类别串, 阈值)` 的往返：与 `as_str` 成对，落库读回不得改类别；未知类别不兜底。
    #[test]
    fn condition_round_trips_through_its_stored_pair() {
        for condition in [
            ConditionType::StopLoss { price: 12.5 },
            ConditionType::TakeProfit { price: 30.0 },
            ConditionType::ResistanceBreak { price: 9.9 },
            ConditionType::SupportBreak { price: 8.8 },
            ConditionType::ChangePct { threshold: 5.0 },
            ConditionType::TurnoverRate { threshold: 10.0 },
        ] {
            let restored =
                ConditionType::from_stored(condition.as_str(), Some(condition.threshold_value()))
                    .unwrap_or_else(|| panic!("{} 应能读回", condition.as_str()));
            assert_eq!(restored, condition, "类别与阈值必须成对往返");
        }
        assert!(
            ConditionType::from_stored("ma_cross", Some(3.0)).is_none(),
            "退役掉的类别不得复活成默认值"
        );
        assert!(ConditionType::from_stored("stop_loss", None).is_none(), "缺阈值的行无法判定");
    }

    /// 计数落点：同一天累加、跨北京日先清零。旧形态把计数 `skip` 掉且没有归属日 ⇒ 上限恒不成立。
    #[test]
    fn note_triggered_accumulates_within_a_day_and_rolls_across_days() {
        let mut o = order(ConditionType::StopLoss { price: 12.0 });
        o.note_triggered(DAY, 1_760_000_000_000);
        assert_eq!((o.today_trigger_count, o.trigger_date.as_deref()), (1, Some(DAY)));
        o.note_triggered(DAY, 1_760_000_060_000);
        assert_eq!(o.today_trigger_count, 2, "同一北京日应累加");
        assert_eq!(o.last_triggered_at, Some(1_760_000_060_000), "冷却窗口读的是这个时刻");

        o.note_triggered("2026-10-16", 1_760_086_400_000);
        assert_eq!(o.today_trigger_count, 1, "换日必须先清零再计第一次");
        assert_eq!(o.trigger_date.as_deref(), Some("2026-10-16"));
    }
}
