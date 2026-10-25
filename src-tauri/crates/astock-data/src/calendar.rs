use chrono::{Datelike, Duration, NaiveDate, Timelike, Utc, Weekday};
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::LazyLock;

/// 远程拉取的 A 股节假日缓存(YYYY-MM-DD 格式)。
/// 启动时由 `init_holiday_calendar` 异步填充,is_trading_day 优先查这里。
/// 用 RwLock 允许运行时刷新(每年初拉一次新年日历)。
///
/// 锁选型:使用 `parking_lot::RwLock` 而非 `std::sync::RwLock` 或 `tokio::sync::RwLock`。
/// - 不用 `std::sync::RwLock`: 规则8 禁止(panic 会毒化,guard 跨 await 是 UB)。
/// - 不用 `tokio::sync::RwLock`: `is_trading_day` 是同步函数且被 7 个文件广泛调用,
///   改 async 会污染整个调用链;此处锁的持有时间极短(仅 contains/extend),从不跨 await。
/// - `parking_lot::RwLock` 的 guard 非 Send,编译器会阻止跨 await 持有(消除 UB 风险);
///   且 parking_lot 不实现 poison(消除 panic 毒化风险)。满足规则8的本意。
static REMOTE_HOLIDAYS: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// K 线自证出的「**确已开市**」日期(YYYY-MM-DD)。与 `REMOTE_HOLIDAYS` 互补：
/// 后者只会从「缺失的工作日」推休市（只增不减），对「把真实交易日误标成休市」无能为力 ——
/// 而那正是硬编码兜底表的失分形态（表里**未来日期**的条目在当天到来前无从证伪）。
/// 2026-09-30 实机验证实证：`2026-09-28/29/30` 被表误列连休 ⇒ `previous_trading_day(09-30)`
/// 错落 09-24，收盘采集把 09-30 的行情值标成 09-24 落库。本集合优先级**高于**兜底表。
static REMOTE_TRADING_DAYS: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// 把东方财富返回的 "20250101" 格式转换为 "2025-01-01"
fn em_date_to_iso(s: &str) -> String {
    if s.len() == 8 && s.chars().all(|c| c.is_ascii_digit()) {
        format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8])
    } else {
        s.to_string()
    }
}

/// 判断是否为A股交易日。
///
/// 优先级:
/// 0. K 线自证「确已开市」命中 → 交易日（**优先于其余全部判据**；未来日期唯一可推翻
///    兜底表误标的机制，见 `REMOTE_TRADING_DAYS`）
/// 1. 远程/K 线自证的休市集合命中 → 非交易日
/// 2. 周末判断(Sat/Sun)—— A 股周末永不开市，调休补班不影响股市
/// 3. 2025-2026 年硬编码节假日（兜底，已知有条目不准，见 `HARDCODED_HOLIDAYS` 注释）
///
/// 缺陷 A 修复:2027 年及以后不再依赖硬编码,而是依赖远程缓存(由东方财富 API 拉取 365 天滚动)。
///
/// ⚠ 取数**不要**依赖本函数决定"用哪一天的数据"—— 行情/K 线已改为按真实 K 线自证
/// （见 `AStockClient::quote_from_klines`）；本函数用于休市提示、定时任务跳过等场景。
pub fn is_trading_day(date: &NaiveDate) -> bool {
    let date_str = date.format("%Y-%m-%d").to_string();
    is_trading_day_with(&date_str, &REMOTE_TRADING_DAYS.read(), &REMOTE_HOLIDAYS.read())
}

/// 判据本体（纯函数，便于用局部集合测试，零全局副作用）。
///
/// `proven`: K 线自证「**确已开市**」；`remote`: 远程 / K 线自证休市。
/// 顺序即优先级——**正向自证先于一切**：兜底表里**未来日期**的条目在当天到来前
/// 无从证伪，只有「该日真实存在 K 线」能推翻它（2026-09-28/29/30 被误列连休即此形态）。
fn is_trading_day_with(date_str: &str, proven: &HashSet<String>, remote: &HashSet<String>) -> bool {
    if proven.contains(date_str) {
        return true;
    }
    if remote.contains(date_str) {
        return false;
    }
    let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
        return false;
    };
    // A 股周末永不开市，调休补班不影响股市（2026-09-26 删除「调休工作日」名单：
    // 被列为开市的 `2025-02-08`（周六）实测无 K 线，序列 02-05 / 02-06 / 02-07 / 02-10；
    // 留着它会让 `previous_trading_day` / `latest_trading_day` 算出不存在的行情日）。
    let w = date.weekday();
    if w == Weekday::Sat || w == Weekday::Sun {
        return false;
    }
    !HARDCODED_HOLIDAYS.contains(&date_str)
}

/// 硬编码兜底表（2025-2026 年节假日，简化版，不含临时休市）。
///
/// ⚠ 只在远程与 K 线自证都拿不到时兜底。两次纠错的共同教训是**表里的未来日期在当天
/// 到来前无从证伪**；现已由 `REMOTE_TRADING_DAYS`（K 线正向自证）推翻，纠错不必再等实机发现：
///
/// - 2026-09-27（301302 t-risk 超时根因，见 `PLAN-live-kline-cache-calendar-storm.md` C1）：
///   摘除 `2026-09-21` / `2026-09-22`（真实日 K 显示这两天正常开市，序列 …09-18、09-21、09-22、09-23）；
///   补 `2026-09-25`（中秋，周五休市）——腾讯日 K 止于 09-24 自证；该条目缺失曾使
///   `latest_trading_day()` 误报 09-25，击穿 K 线缓存新鲜性判据 ⇒ live 缓存风暴。
/// - 2026-09-30（实机验证）：摘除误列的 `2026-09-28/29/30`。上证日 K 有这三天、09-25 无；
///   库内 `601231` 行 close=23.74 与腾讯 09-30 收盘逐字相等 ⇒ 连休三天让
///   `previous_trading_day(09-30)` 错落 09-24，收盘采集把 09-30 的行情值贴 09-24 标签落库。
///
/// ⚠ 它**不得**决定"取哪一天的数据"：行情/K 线取数已改为「≤ 截止日的最后一根真实 K 线」
/// 自证（见 `AStockClient::quote_from_klines`），这张表只影响休市提示与快照采集的跳过判断。
const HARDCODED_HOLIDAYS: &[&str] = &[
    // 2025年
    "2025-01-01", // 元旦
    "2025-01-28",
    "2025-01-29",
    "2025-01-30",
    "2025-01-31",
    "2025-02-03",
    "2025-02-04", // 春节
    "2025-04-04",
    "2025-04-07", // 清明
    "2025-05-01",
    "2025-05-02",
    "2025-05-05", // 劳动节
    "2025-06-02", // 端午
    "2025-09-15",
    "2025-09-16", // 中秋
    "2025-10-01",
    "2025-10-02",
    "2025-10-03",
    "2025-10-06",
    "2025-10-07",
    "2025-10-08", // 国庆
    // 2026年
    "2026-01-01",
    "2026-01-02",
    "2026-02-16",
    "2026-02-17",
    "2026-02-18",
    "2026-02-19",
    "2026-02-20",
    "2026-02-23",
    "2026-02-24",
    "2026-03-23",
    "2026-03-24",
    "2026-04-20",
    "2026-04-21",
    "2026-05-18",
    "2026-05-19",
    "2026-06-08",
    "2026-06-09",
    "2026-09-25", // 中秋（周五休市；腾讯日 K 止于 09-24 自证）
    // 2026-09-28 / 29 / 30 已摘除：误列连休，见上方实证
    "2026-10-01",
    "2026-10-02",
];

/// 获取当前北京时间对应的"最新交易日"。
///
/// 规则：
/// - 若今天（北京日期）是交易日，返回今天；
/// - 否则返回 previous_trading_day(today)。
///
/// ⚠ K 线缓存新鲜性自 C2（2026-09-27）起由 TTL 全权，不再比对本日期
/// （见 `lib.rs` 的 `get_klines_with_adj`）；本函数现用于休市提示等展示场景。
pub fn latest_trading_day() -> NaiveDate {
    let (_, _, today) = beijing_now();
    previous_trading_day(today)
}

/// 查找指定日期之前最近的交易日(若 date 本身是交易日,返回 date)
/// 用于 as-of 模式下周末/节假日的 fallback: 用户选择回看周六,
/// 真实数据应该取周五收盘。
pub fn previous_trading_day(date: NaiveDate) -> NaiveDate {
    let mut candidate = date;
    for _ in 0..14 {
        // 14 天上限(春节+调休极端情况)
        if is_trading_day(&candidate) {
            return candidate;
        }
        candidate -= Duration::days(1);
    }
    // 兜底:连续14天都不是交易日(极端硬编码漏洞),返回原 date
    date
}

/// 查找指定日期之后最近的交易日(若 date 本身是交易日,返回 date)
pub fn next_trading_day(date: NaiveDate) -> NaiveDate {
    let mut candidate = date;
    for _ in 0..14 {
        if is_trading_day(&candidate) {
            return candidate;
        }
        candidate += Duration::days(1);
    }
    date
}

/// 从 `date` 起偏移 `n` 个交易日（时间旅行批次的 T+N / T-N 锚点算术）。
///
/// 语义：
/// - `n == 0`：原样返回 `date`（**不要求** `date` 本身是交易日，调用方需自行保证）；
/// - `n > 0`：`date` 之后第 `n` 个交易日（不含 `date`；若 `date` 非交易日则从次日方向起算，
///   即与「先归一到最近交易日再偏移」等价的直觉形态）；
/// - `n < 0`：`date` 之前第 `|n|` 个交易日（不含 `date`）。
///
/// 例：`nth_trading_day(2026-09-24(周四), 1)` = 2026-09-28（09-25 中秋休市 + 周末）。
///
/// ⚠ 遍历设 800 自然天上限（90 交易日 ≈ 126 自然日，留足春节连休余量）；
/// 触限返回最后扫描到的日期 —— 仅在日历数据极端缺失时发生，属兜底而非正常路径。
pub fn nth_trading_day(date: NaiveDate, n: i32) -> NaiveDate {
    if n == 0 {
        return date;
    }
    let mut candidate = date;
    let mut remaining = n.unsigned_abs();
    let step = if n > 0 {
        Duration::days(1)
    } else {
        Duration::days(-1)
    };
    for _ in 0..800 {
        candidate += step;
        if is_trading_day(&candidate) {
            remaining -= 1;
            if remaining == 0 {
                return candidate;
            }
        }
    }
    candidate
}

/// 半开区间 `[start, end)` 内的交易日数量；`end < start` 时按反向区间返回负值。
///
/// 用于批次预检（如判断 long 档 hindsight 是否已到期）与 UI 展示。跨度上限 800 自然天，
/// 超出部分不再计数（当前业务最大区间 90 交易日 ≈ 126 自然日，不触限）。
pub fn trading_days_between(start: NaiveDate, end: NaiveDate) -> i32 {
    if start == end {
        return 0;
    }
    let (mut cursor, target, sign) = if end > start {
        (start, end, 1i32)
    } else {
        (end, start, -1i32)
    };
    let mut count = 0;
    for _ in 0..800 {
        if cursor >= target {
            break;
        }
        if is_trading_day(&cursor) {
            count += 1;
        }
        cursor += Duration::days(1);
    }
    sign * count
}

/// 获取当前北京时间（UTC+8），正确处理日期跨夜。
///
/// 返回 (hour, minute, beijing_date)：
/// - hour/minute: 北京时间时分
/// - beijing_date: 北京日期（用于交易日判定）
///
/// 修复前: `is_trading_time()` 用 `now.date_naive()` (UTC 日期) 判定交易日，
/// 在 UTC 16:00-24:00（北京次日 00:00-08:00）期间会判错日期。
///
/// 公开给「要按北京墙上时钟判时段」的下游（条件单的时间窗与单日计数归属都读它），
/// 让它们不再各自 `Local::now()` 拼字符串 —— 机器时区一换，规则就跟着漂移。
pub fn beijing_now() -> (u32, u32, NaiveDate) {
    let now = Utc::now();
    let utc_hour = now.hour();
    let minute = now.minute();
    // UTC+8: 如果 UTC 时 + 8 >= 24，则北京已是次日
    let beijing_hour = (utc_hour + 8) % 24;
    let beijing_date = if utc_hour + 8 >= 24 {
        now.date_naive() + Duration::days(1)
    } else {
        now.date_naive()
    };
    (beijing_hour, minute, beijing_date)
}

/// A 股一天里的会话阶段（北京时间口径）。
///
/// 为什么要枚举而不是一个 bool：同一句「现在不是交易时间」在监控循环、行情监视器、
/// 状态接口三处要说的话不一样（盘前 / 午休 / 收盘后 / 非交易日），而 bool 只能把它们
/// 压成同一个读数。阶段 E 的告警冷却与 watcher 停摆都要能各自点名是哪一段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingSession {
    /// 非交易日（周末或已知休市，且无 K 线正向自证推翻）
    NonTradingDay,
    /// 交易日 09:30 之前的全部时段（自 00:00 起算，凌晨亦属此档；含 09:15–09:25 集合竞价，本枚举不单独区分）
    PreOpen,
    /// 上午连续竞价 09:30–11:30（左闭右开）
    Morning,
    /// 午间休市 11:30–13:00
    LunchBreak,
    /// 下午连续竞价 13:00–15:00（左闭右开）
    Afternoon,
    /// 15:00 收盘之后
    PostClose,
}

impl TradingSession {
    /// 连续竞价进行中 —— 只有这两段取实时价才有意义（其余时段行情是冻结的）。
    pub fn is_open(self) -> bool {
        matches!(self, Self::Morning | Self::Afternoon)
    }

    /// 跨语言边界的稳定档位码（IPC 契约，不是给用户看的文本）。
    ///
    /// 为什么不用 [`Self::status_label`]：那是中文串，直接端给前端会让 11 种语言里
    /// 9 种显示中文；而 `format!("{self:?}")` 又把 Debug 形态当成了契约 —— 改一次
    /// 变体名就静默改掉前端键。前端按本码查 i18n 键（`stockAnalysis.monitor.session.*`）。
    pub fn code(self) -> &'static str {
        match self {
            Self::NonTradingDay => "non_trading_day",
            Self::PreOpen => "pre_open",
            Self::Morning => "morning",
            Self::LunchBreak => "lunch_break",
            Self::Afternoon => "afternoon",
            Self::PostClose => "post_close",
        }
    }

    /// 给用户看的分档名。抽成纯方法是为了让「五档各有一句」可被测试钉住 ——
    /// 挂在墙上时钟上的话，非交易日那档一年只有几天能被读到。
    pub fn status_label(self) -> &'static str {
        match self {
            Self::Morning | Self::Afternoon => "交易中",
            Self::LunchBreak => "午休中，13:00恢复",
            Self::PreOpen => "距开盘",
            Self::PostClose => "已收盘",
            Self::NonTradingDay => "非交易日",
        }
    }
}

/// 会话划分的判据本体（纯函数，表驱动测试拆在这里，不依赖墙上时钟）。
///
/// 边界是**左闭右开**：09:30 与 13:00 属开市，11:30 与 15:00 属休市/收盘。
/// 这修掉的是原 `is_trading_time` 里两处整点判法不一致的形态
/// （`hour==11 && minute<=30` 让 11:30 整点算开市、`hour==15 && minute==0` 让 15:00 之后
/// 整整 59 分钟都算开市 —— 收盘后那一段既白打请求，又会让按时间冷却的告警拿冻结收盘价反复触发）。
pub fn trading_session_of(hour: u32, minute: u32, trading_day: bool) -> TradingSession {
    if !trading_day {
        return TradingSession::NonTradingDay;
    }
    let mins = hour * 60 + minute;
    if mins < 9 * 60 + 30 {
        TradingSession::PreOpen
    } else if mins < 11 * 60 + 30 {
        TradingSession::Morning
    } else if mins < 13 * 60 {
        TradingSession::LunchBreak
    } else if mins < 15 * 60 {
        TradingSession::Afternoon
    } else {
        TradingSession::PostClose
    }
}

/// 当前所处的会话阶段（北京时间 + 当日是否交易日）。
pub fn current_trading_session() -> TradingSession {
    let (hour, minute, today) = beijing_now();
    trading_session_of(hour, minute, is_trading_day(&today))
}

/// 判断当前是否为交易时间（连续竞价进行中）
pub fn is_trading_time() -> bool {
    current_trading_session().is_open()
}

/// 从东方财富 API 获取最新交易日历
pub async fn fetch_holiday_calendar() -> Result<Vec<String>, String> {
    let url = "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_TRADE_CALENDAR&columns=TRADE_DATE,IS_TRADING_DAY&pageSize=365&pageNumber=1";
    let resp = reqwest::get(url).await.map_err(|e| format!("获取交易日历失败: {}", e))?;
    let json: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;

    let holidays: Vec<String> = json["result"]["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|d| {
            let is_trading = d["IS_TRADING_DAY"].as_str().unwrap_or("1") == "0";
            let date_raw = d["TRADE_DATE"].as_str().unwrap_or("");
            let date = em_date_to_iso(date_raw);
            if !date.is_empty() && is_trading {
                Some(date)
            } else {
                None
            }
        })
        .collect();

    Ok(holidays)
}

/// 把远程拉取的节假日写入全局缓存。
/// 启动时调用一次,失败也不影响 is_trading_day(会 fallback 到硬编码 2025-2026)。
/// 返回写入的节假日条数。
pub fn populate_remote_holidays(holidays: Vec<String>) -> usize {
    let count = holidays.len();
    let mut cache = REMOTE_HOLIDAYS.write();
    cache.clear();
    cache.extend(holidays);
    count
}

/// 启动时初始化节假日缓存(异步,fire-and-forget)。
/// 不阻塞主流程,失败时静默退化(走硬编码)。
///
/// ⚠ 2026-09-27：其数据源 `RPTA_WEB_TRADE_CALENDAR` 报表已下线（实测 code=9501
/// 「报表配置不存在」），本函数的实际接线已被**指数日 K 自证**取代
/// （`derive_holidays_from_kline_dates`，由 `src/init/state.rs` 启动时调用）。
/// 保留仅为手动刷新命令（`stock_analysis.rs` 的日历查询）的兼容出口。
pub async fn init_holiday_calendar() -> Result<usize, String> {
    let holidays = fetch_holiday_calendar().await?;
    Ok(populate_remote_holidays(holidays))
}

/// C3（2026-09-27，301302 t-risk 超时根因）：用上证指数日 K「自证」交易日历，
/// 替代已下线的东财报表。纯函数，零全局状态，便于测试。
///
/// 输入：指数日 K 的日期序列（vendor 原始，任意顺序）与北京今日。
/// 规则：取 `[最早, 今日)` 区间内**未开市的工作日** ⇒ 休市：
/// - 内部空洞（国庆连休等，夹在两根 K 线之间）；
/// - 尾部空洞（如 2026-09-25 中秋：日 K 止于 09-24，09-25 无 K 线）。
///
/// ⚠ 权衡：vendor 数据若滞后一日，会把最近一个工作日误标休市 —— 代价只影响
/// 休市提示 / 快照采集跳过（K 线缓存新鲜性已由 C2 改为 TTL 全权，不再依赖本表），
/// 远小于漏标节假日的代价（漏标正是本次缓存风暴的源头）。`today` 本身永不标记。
///
/// ## 🔴 R-2 修复（2026-10-25）：尾部空洞**必须有上限**
///
/// 原实现把「序列最后一个日期之后、今天之前的每个工作日」一律标成休市。
/// 当 vendor 返回**残缺序列**时（限流 / WAF / 字段解析失败 / 只返回部分日期），
/// 这条规则会凭空造出**一整片假休市日** —— 实测：指数 K 线止于 2026-09-28 时，
/// 09-30 ~ 10-22 共 **18 个真实交易日**被误判为休市，
/// 于是时间旅行批次拒绝全部 2026-10 的锚点日（`不是 A 股交易日`）。
///
/// 现在分两段处理：
/// - **内部空洞**（`首根 < d ≤ 末根`）：序列覆盖了这段区间，缺失即真休市 —— 保留；
/// - **尾部空洞**（`末根 < d < today`）：只有当 `today − 末根 ≤ MAX_TAIL_GAP_DAYS`
///   时才推导；超出即视为「序列滞后」，**整段不推导**。
///
/// 另加两道防护：
/// - 序列根数 < [`MIN_BARS_FOR_DERIVATION`] ⇒ 样本太少，不推导；
/// - 内部空洞密度 > [`MAX_INTERNAL_HOLE_RATIO`] ⇒ 序列稀疏，整段放弃推导。
///
/// 依据（2026-10-25 实测）：腾讯 `fqkline` 对 `sh000001` 返回 60 根完整序列
/// （2026-07-24 ~ 2026-10-23），东财 `push2his` 被 RST 拦截 ⇒ 应用侧拿到的是残缺序列。
/// **「用尽可得数据」的前提是数据本身可信 —— 这里补上的就是那层校验。**
pub fn derive_holidays_from_kline_dates(dates: &[String], today: NaiveDate) -> Vec<String> {
    let parsed: std::collections::BTreeSet<NaiveDate> =
        dates.iter().filter_map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).collect();
    let Some(&first) = parsed.iter().next() else {
        return vec![];
    };
    let Some(&last) = parsed.iter().next_back() else {
        return vec![];
    };

    // 防护①：根数太少 ⇒ 样本不足以判断「缺失 = 休市」
    if parsed.len() < MIN_BARS_FOR_DERIVATION {
        return vec![];
    }

    // 防护②：内部空洞密度过高 ⇒ 序列稀疏（不是「密集休市」），整段放弃推导。
    // 判据用「缺失工作日 / 区间工作日总数」，阈值 30%：正常年份的休市占比 < 5%，
    // 春节+国庆连休的极端月份也 < 20%；超过 30% 只能解释为数据源残缺。
    let span_weekdays = count_weekdays(first, last);
    if span_weekdays > 0 {
        let present = count_dates_in(first, last, &parsed);
        let holes = span_weekdays - present;
        if (holes as f64) > (span_weekdays as f64) * MAX_INTERNAL_HOLE_RATIO {
            return vec![];
        }
    }

    let mut out = Vec::new();
    // 尾部可信性以「今天 − 末根」的**整体间距**判定：序列滞后 ⇒ 末根之后的整段
    // 都不可信（逐日判断会漏掉「离末根近但离今天远」的那些日子 —— 那正是
    // 10-09 这种：距末根 11 天、距今天 16 天，逐日判会把它算成「离今天够近」而误标）。
    let tail_gap_days = (today - last).num_days();
    let tail_trusted = tail_gap_days <= MAX_TAIL_GAP_DAYS;
    let mut d = first + Duration::days(1);
    while d < today {
        let w = d.weekday();
        if w != Weekday::Sat && w != Weekday::Sun && !parsed.contains(&d) {
            // 内部空洞（d ≤ last）始终可信；尾部空洞仅在序列足够新时才可信。
            if d <= last || tail_trusted {
                out.push(d.format("%Y-%m-%d").to_string());
            }
        }
        d += Duration::days(1);
    }
    out
}

/// 尾部空洞的最大可信跨度（自然日）。
///
/// 取 12 天：需覆盖「春节连休」这种最长休市（2026 年春节 02-16 ~ 02-24 共 9 天），
/// 同时把「序列滞后 18 天以上」的残缺序列挡在外面（实测残缺形态滞后 27 天）。
const MAX_TAIL_GAP_DAYS: i64 = 12;

/// 参与推导所需的最少 K 线根数。
///
/// 取 5：既能挡住「只有 1~2 根 ⇒ 区间极窄、几乎所有工作日都像空洞」这种退化输入，
/// 又不破坏 C3 回归（内部空洞 + 尾部空洞）用 5 根序列的既有判据。
///
/// ⚠ **拦住实测故障（2026-10-25）的是尾部空洞上限，不是本项** ——
/// 那次残缺序列有 ~45 根，根数完全正常，是「末根落后今天 27 天」触发了上限。
/// 两道护栏分工不同，别只留一道。
const MIN_BARS_FOR_DERIVATION: usize = 5;

/// 内部空洞密度上限（占区间工作日比例）。
const MAX_INTERNAL_HOLE_RATIO: f64 = 0.30;

/// `[from, to]` 区间内的工作日数量（含端点）。
fn count_weekdays(from: NaiveDate, to: NaiveDate) -> i64 {
    let mut n = 0i64;
    let mut d = from;
    while d <= to {
        let w = d.weekday();
        if w != Weekday::Sat && w != Weekday::Sun {
            n += 1;
        }
        d += Duration::days(1);
    }
    n
}

/// `[from, to]` 区间内落在给定集合中的日期数量。
fn count_dates_in(
    from: NaiveDate,
    to: NaiveDate,
    set: &std::collections::BTreeSet<NaiveDate>,
) -> i64 {
    let mut n = 0i64;
    let mut d = from;
    while d <= to {
        if set.contains(&d) {
            n += 1;
        }
        d += Duration::days(1);
    }
    n
}

/// 把 K 线自证出的休市日**合并**进全局远程缓存（不清空——与其它来源共存），
/// 同时把「确已开市」日期并入 `REMOTE_TRADING_DAYS`（正向自证，可推翻兜底表误标；
/// 2026-09-30 实机：09-28/29/30 被表误列连休，正向集合是唯一纠错机制）。
/// 返回本次新增的休市日条数。
pub fn populate_holidays_from_kline_dates(dates: &[String], today: NaiveDate) -> usize {
    let derived = derive_holidays_from_kline_dates(dates, today);
    // 两份集合分开取写锁（不嵌套）：is_trading_day 读侧先 TRADING 后 HOLIDAYS，
    // 若这里持其一再取另一，读侧持另一把等待本锁即构成死锁环。
    let added = {
        let mut cache = REMOTE_HOLIDAYS.write();
        let before = cache.len();
        cache.extend(derived);
        cache.len() - before
    };
    REMOTE_TRADING_DAYS
        .write()
        .extend(dates.iter().filter(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok()).cloned());
    added
}

/// 北京时间的今日（启动时 K 线日历自证用）
pub fn beijing_today() -> NaiveDate {
    beijing_now().2
}

/// 北京日 `date` 的 UTC 毫秒区间 `[00:00, 次日 00:00)`，即 A 股口径的「当日」。
///
/// 为什么要有它：多处按「当日」筛数据时各写各的，而 UTC 16:00–24:00 对应北京次日
/// 00:00–08:00 —— 同一个「当日」会有两套答案（`beijing_now` 修的就是这个形态）。
/// 阶段 B 的复盘窗口、阶段 E 的会话判定、阶段 G 的快照落日期都应走这一个helper。
pub fn beijing_day_window_ms(date: &NaiveDate) -> (i64, i64) {
    let start_naive = date.and_hms_opt(0, 0, 0).expect("日界 00:00 恒存在");
    // 北京 00:00 = UTC 前一日 16:00
    let start_utc = start_naive - Duration::hours(8);
    let end_utc = start_utc + Duration::hours(24);
    (start_utc.and_utc().timestamp_millis(), end_utc.and_utc().timestamp_millis())
}

/// 仅供测试:清空远程节假日缓存,恢复纯硬编码模式
#[cfg(test)]
pub fn clear_remote_holidays_for_test() {
    REMOTE_HOLIDAYS.write().clear();
    REMOTE_TRADING_DAYS.write().clear();
}

/// 获取距离下一个交易时间的描述。
///
/// 阶段 E-5：这里不再自己重写一遍时刻比较，分档一律走 `TradingSession`。
/// 旧形态给出过假读数：周六 16:00 命中 `hour >= 15` ⇒ 「已收盘」，而那天根本没有开盘这回事
/// （非交易日只能从 9–15 点之外的兜底分支走到，等于把周末读成了收盘）。
pub fn next_trading_time_desc() -> String {
    current_trading_session().status_label().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全局态测试串行锁（`is_trading_day` 的两份自证集合是进程级共享）。
    /// 凡对全局态 populate / clear 的测试必须持锁：两个写入方并行时，
    /// 一方的 clear 可能落在另一方的 populate 与 assert 之间 ⇒ flaky。
    static GLOBAL_STATE_TS: LazyLock<parking_lot::Mutex<()>> =
        LazyLock::new(|| parking_lot::Mutex::new(()));

    #[test]
    fn test_weekend_not_trading() {
        let sat = NaiveDate::from_ymd_opt(2025, 6, 7).unwrap(); // 周六
        let sun = NaiveDate::from_ymd_opt(2025, 6, 8).unwrap(); // 周日
        assert!(!is_trading_day(&sat));
        assert!(!is_trading_day(&sun));
    }

    #[test]
    fn test_weekday_is_trading() {
        let mon = NaiveDate::from_ymd_opt(2025, 6, 9).unwrap(); // 周一，非节假日
        assert!(is_trading_day(&mon));
    }

    #[test]
    fn test_holiday_not_trading() {
        let new_year = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(); // 元旦
        assert!(!is_trading_day(&new_year));
    }

    #[test]
    fn test_national_day_holiday() {
        let nd = NaiveDate::from_ymd_opt(2025, 10, 1).unwrap(); // 国庆
        assert!(!is_trading_day(&nd));
    }

    /// 反转回归（2026-09-26）：周末**一律非交易日**，不存在"调休补班所以股市开市"。
    ///
    /// 原断言恰好相反（`assert!(is_trading_day(2025-02-08))`），依据是国务院调休安排；
    /// 但 A 股周末从不开市——实测上证日 K 在 2025-02-08（周六）无数据
    /// （序列为 02-05/06/07 后直接 02-10）。留着的代价是 `previous_trading_day`
    /// 会算出一个不存在的行情日，回放里表现为"取到一天根本不存在的数据"。
    #[test]
    fn weekend_never_trades_even_on_makeup_workday() {
        let sat = NaiveDate::from_ymd_opt(2025, 2, 8).unwrap(); // 春节调休上班的周六
        assert_eq!(sat.weekday(), Weekday::Sat, "前提核对：这天确实是周六");
        assert!(!is_trading_day(&sat), "股市周末不开市，与是否调休上班无关");
    }

    /// 缺陷 A 修复:远程节假日缓存优先于硬编码
    #[test]
    fn test_remote_holiday_overrides_default() {
        let _serial = GLOBAL_STATE_TS.lock();
        clear_remote_holidays_for_test();
        // 2027-01-01 元旦 — 硬编码 2025-2026 没覆盖,默认会被认为"是交易日"
        let future = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        assert!(is_trading_day(&future), "默认(无远程缓存)2027 元旦应被当作交易日");
        // 注入远程缓存后,应被识别为节假日
        let n = populate_remote_holidays(vec!["2027-01-01".to_string()]);
        assert_eq!(n, 1);
        assert!(!is_trading_day(&future), "远程缓存应优先于硬编码");
        // 清理,不影响其他测试
        clear_remote_holidays_for_test();
    }

    #[test]
    fn test_em_date_to_iso() {
        assert_eq!(em_date_to_iso("20250101"), "2025-01-01");
        assert_eq!(em_date_to_iso("20261231"), "2026-12-31");
        // 非数字原样返回
        assert_eq!(em_date_to_iso("2025-01-01"), "2025-01-01");
        assert_eq!(em_date_to_iso(""), "");
    }

    /// C1 纠错回归（2026-09-27，301302 t-risk 超时根因）：
    /// 中秋 2026-09-25（周五）休市必须入表；09-21/22 被误标休市必须摘除。
    /// 实证来源：腾讯日 K 序列 …09-18、09-21、09-22、09-23、09-24（09-25 无）。
    #[test]
    fn mid_autumn_2026_is_closed_and_sept21_22_are_trading() {
        // 不 clear 远程缓存：本测试日期全在 2026-09，与唯一注入 2027 条目的
        // `test_remote_holiday_overrides_default` 无交集，clear 反而引入并行竞态。
        let fri = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        assert_eq!(fri.weekday(), Weekday::Fri, "前提核对：09-25 是周五");
        assert!(!is_trading_day(&fri), "2026 中秋（09-25 周五）休市");
        for d in [21, 22] {
            let day = NaiveDate::from_ymd_opt(2026, 9, d).unwrap();
            assert!(is_trading_day(&day), "2026-09-{d:02} 实际开市，不得标休市");
        }
        // 击穿缓存判据的那条链：周日的 previous_trading_day 必须落到 09-24（周四），
        // 而不是误报 09-25 ⇒ K 线缓存新鲜性校验（last=09-24）才能命中。
        let sun = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
        assert_eq!(
            previous_trading_day(sun),
            NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
            "09-25 缺表时这里会错报 09-25 ⇒ live K 线缓存恒判过期（缓存风暴根因）"
        );
    }

    /// C3 回归（2026-09-27）：指数日 K 自证日历 —— 内部空洞与尾部空洞都要推出，
    /// today 本身与序列里存在的日期不得误标。用 2027 独立日期段，纯函数零全局副作用。
    #[test]
    fn kline_derived_calendar_covers_inner_and_tail_gaps() {
        // 构造 2027-03：开市日 03-01..03-04、03-08；03-05(五) 模拟内部空洞；
        // 03-09(一)/03-10(二)/03-11(三) 模拟尾部空洞（K 线止于 03-08）；today=03-12(五)。
        let dates = ["2027-03-01", "2027-03-02", "2027-03-03", "2027-03-04", "2027-03-08"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let today = NaiveDate::from_ymd_opt(2027, 3, 12).unwrap();
        let got = derive_holidays_from_kline_dates(&dates, today);
        assert_eq!(
            got,
            vec!["2027-03-05", "2027-03-09", "2027-03-10", "2027-03-11"],
            "内部空洞 03-05 与尾部空洞 03-09..11 都必须推出；周末与 K 线日期不得混入"
        );
        // 空输入 ⇒ 空输出（启动拉取失败时不会误伤任何日期）
        assert!(derive_holidays_from_kline_dates(&[], today).is_empty());
    }

    /// 2026-09-30 实机纠错回归：兜底表摘除误列的 09-28/29/30（上证日 K 有这三天）；
    /// 且 K 线「确已开市」正向自证必须能推翻表内条目（未来日期唯一可推翻机制）。
    /// 用局部集合调纯函数，零全局副作用（不与全局态测试互扰）。
    #[test]
    fn hardcoded_mislabel_removed_and_proven_overrides_table() {
        let empty = HashSet::new();
        // ① 误列条目摘除：无任何自证也判开市（09-28/29/30 是周一/二/三，非周末）
        for d in [28u32, 29, 30] {
            assert!(
                is_trading_day_with(&format!("2026-09-{d:02}"), &empty, &empty),
                "2026-09-{d:02} 实际开市（上证日 K 自证），不得再列连休；误标会让 previous_trading_day(09-30) 错落 09-24，当日行情被贴上 09-24 标签落库"
            );
        }
        // ② 表内其余条目仍然生效（无自证时 10-01 国庆判休市）——证明兜底表未被整体摘除
        assert!(!is_trading_day_with("2026-10-01", &empty, &empty));
        // ③ 正向自证可推翻表内条目：模拟「表里未来日期又标错」，K 线自证能纠
        let proven: HashSet<String> = HashSet::from(["2026-10-01".to_string()]);
        assert!(is_trading_day_with("2026-10-01", &proven, &empty));
        // ④ 两自证集合同日冲突（理论上互斥，防回归）：以「确有成交」为准
        let remote: HashSet<String> = HashSet::from(["2026-10-01".to_string()]);
        assert!(is_trading_day_with("2026-10-01", &proven, &remote));
        // ⑤ 周末不受影响（09-25 周五休市 ⇒ 09-26 周六）
        assert!(!is_trading_day_with("2026-09-26", &empty, &empty));
    }

    /// 接线回归（2026-09-30）：启动路径 `populate_holidays_from_kline_dates` 必须把
    /// K 线日期并入「确已开市」集合，`is_trading_day` 据此翻转 —— 2029-06-16 是周六，
    /// 默认判休市，仅当正向集合被真正写入才会变交易日。持全局锁防并行 clear 竞态。
    #[test]
    fn populate_kline_dates_wires_proven_trading_days() {
        let _serial = GLOBAL_STATE_TS.lock();
        clear_remote_holidays_for_test();
        let sat = NaiveDate::from_ymd_opt(2029, 6, 16).unwrap();
        assert_eq!(sat.weekday(), Weekday::Sat, "前提核对：2029-06-16 是周六");
        assert!(!is_trading_day(&sat), "周六默认非交易日（基线）");
        let today = NaiveDate::from_ymd_opt(2029, 6, 18).unwrap();
        let n = populate_holidays_from_kline_dates(&["2029-06-16".to_string()], today);
        assert_eq!(n, 0, "该序列无缺失工作日（06-17 是周日）⇒ 无新增休市日");
        assert!(is_trading_day(&sat), "K 线里存在的日期必须被自证为交易日（推翻周末/表内误标）");
        clear_remote_holidays_for_test();
    }

    /// 交易日偏移算术：中秋 2026-09-25（周五休市）+ 周末形成 3 天非交易日缺口。
    #[test]
    fn nth_trading_day_skips_holiday_and_weekend() {
        let thu = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        assert_eq!(nth_trading_day(thu, 0), thu, "n=0 原样返回");
        // T+1：跳过 09-25 休市与 09-26/27 周末，落到 09-28（周一）
        assert_eq!(nth_trading_day(thu, 1), NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
        // T+2 = 09-29，T+3 = 09-30（实机自证开市）
        assert_eq!(nth_trading_day(thu, 2), NaiveDate::from_ymd_opt(2026, 9, 29).unwrap());
        assert_eq!(nth_trading_day(thu, 3), NaiveDate::from_ymd_opt(2026, 9, 30).unwrap());
        // T-1 = 09-23（周三）
        assert_eq!(nth_trading_day(thu, -1), NaiveDate::from_ymd_opt(2026, 9, 23).unwrap());
        // 反向可逆：T+3 再 T-3 回到起点
        assert_eq!(nth_trading_day(NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(), -3), thu);
    }

    /// 锚点本身是非交易日（如用户选了周日）：偏移从相邻方向起算，不停留在锚点。
    #[test]
    fn nth_trading_day_from_non_trading_anchor() {
        let sun = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
        assert_eq!(sun.weekday(), Weekday::Sun);
        // 向前：周日→周六→中秋→09-24（第 1 个交易日）；第 2 个 = 09-23
        assert_eq!(nth_trading_day(sun, -1), NaiveDate::from_ymd_opt(2026, 9, 24).unwrap());
        assert_eq!(nth_trading_day(sun, -2), NaiveDate::from_ymd_opt(2026, 9, 23).unwrap());
        // 向后：09-28
        assert_eq!(nth_trading_day(sun, 1), NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
        // n=0 不归一（调用方责任）
        assert_eq!(nth_trading_day(sun, 0), sun);
    }

    #[test]
    fn trading_days_between_counts_half_open_interval() {
        let thu = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        let mon = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        // [周四, 周一)：周四 1 个（周五休市、周末不计）
        assert_eq!(trading_days_between(thu, mon), 1);
        // 反向为负值
        assert_eq!(trading_days_between(mon, thu), -1);
        // 同点为 0
        assert_eq!(trading_days_between(thu, thu), 0);
        // [09-24, 10-01)：09-24/28/29/30 共 4 个（端点 10-01 国庆休市且为半开排除端）
        assert_eq!(trading_days_between(thu, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()), 4);
    }

    /// 阶段 E-5：会话边界的表驱动判据（纯函数，不读墙上时钟）。
    ///
    /// 拆在原缺陷上：旧 `is_trading_time` 对 11:30 与 15:00 两个整点用了两种判法
    /// （`minute<=30` 与 `minute==0`），其中 15:00–15:59 整段被判成开市。
    #[test]
    fn trading_session_boundaries_are_half_open() {
        use TradingSession::*;
        let d = true;
        let cases: &[((u32, u32), TradingSession, &str)] = &[
            ((9, 14), PreOpen, "集合竞价前"),
            ((9, 15), PreOpen, "开盘集合竞价起点仍算盘前"),
            ((9, 29), PreOpen, "9:29 未开市"),
            ((9, 30), Morning, "9:30 整点开市（左闭）"),
            ((11, 29), Morning, "上午最后一分钟"),
            ((11, 30), LunchBreak, "11:30 整点进午休（右开）"),
            ((12, 59), LunchBreak, "12:59 仍午休"),
            ((13, 0), Afternoon, "13:00 整点下午开市"),
            ((14, 59), Afternoon, "14:59 仍开市"),
            ((15, 0), PostClose, "15:00 整点收盘（右开）—— 旧形态这一段 59 分钟全算开市"),
            ((15, 30), PostClose, "15:30 收盘后"),
            ((3, 0), PreOpen, "凌晨与 09:14 同档：今日尚未开盘"),
        ];
        for ((h, m), want, why) in cases {
            assert_eq!(trading_session_of(*h, *m, d), *want, "{h:02}:{m:02} — {why}");
        }
        // 非交易日压过一切时段
        assert_eq!(trading_session_of(10, 0, false), NonTradingDay, "交易日判定优先于时段");
        // 只有两段算「开市」
        assert!(Morning.is_open() && Afternoon.is_open());
        assert!(!PreOpen.is_open() && !LunchBreak.is_open() && !PostClose.is_open());
        assert!(!NonTradingDay.is_open());
    }

    /// 阶段 E-5 的呈现面：五档各有自己的一句话，`next_trading_time_desc` 不再自带第二套判据。
    #[test]
    fn every_session_bucket_has_its_own_label() {
        use TradingSession::*;
        let labels: [(_, &str); 6] = [
            (NonTradingDay, "非交易日"),
            (PreOpen, "距开盘"),
            (Morning, "交易中"),
            (Afternoon, "交易中"),
            (LunchBreak, "午休中，13:00恢复"),
            (PostClose, "已收盘"),
        ];
        for (session, want) in labels {
            assert_eq!(session.status_label(), want, "{session:?} 的分档文案");
        }
        // 只有上午与下午两段该说「交易中」：多一行 ⇒ 有档位被塌进了这两段（旧 bool 判法的复发形态）
        assert_eq!(
            labels.iter().filter(|(_, label)| *label == "交易中").count(),
            2,
            "「交易中」的档位集合变了 ⇒ 上面的文案表要一起重看"
        );
    }

    /// 档位码是跨语言边界（IPC → i18n 键）的契约：六档必须两两不同，且不随变体名改写。
    #[test]
    fn session_codes_are_distinct_and_stable() {
        use TradingSession::*;
        let codes: [(_, &str); 6] = [
            (NonTradingDay, "non_trading_day"),
            (PreOpen, "pre_open"),
            (Morning, "morning"),
            (LunchBreak, "lunch_break"),
            (Afternoon, "afternoon"),
            (PostClose, "post_close"),
        ];
        for (session, want) in codes {
            assert_eq!(session.code(), want, "{session:?} 的档位码");
        }
        let uniq: std::collections::HashSet<&str> = codes.iter().map(|(_, c)| *c).collect();
        assert_eq!(uniq.len(), 6, "两档共用一个码 ⇒ 前端那句缺席文案会把两档说成同一件事");
    }

    // ── R-2 回归：残缺指数 K 线不得凭空造出假休市日（2026-10-25）──

    /// 构造一段「完整」的指数日 K 日期序列：区间内的工作日全部出现，
    /// 只剔除给定的休市日。
    fn synth_index_dates(from: NaiveDate, to: NaiveDate, holidays: &[&str]) -> Vec<String> {
        let hs: std::collections::BTreeSet<String> =
            holidays.iter().map(|s| s.to_string()).collect();
        let mut out = Vec::new();
        let mut d = from;
        while d <= to {
            let w = d.weekday();
            let s = d.format("%Y-%m-%d").to_string();
            if w != Weekday::Sat && w != Weekday::Sun && !hs.contains(&s) {
                out.push(s);
            }
            d += Duration::days(1);
        }
        out
    }

    /// **核心正控**：完整序列仍要能推出真休市（否则这次修复把功能改没了）。
    #[test]
    fn complete_series_still_derives_real_holidays() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let dates = synth_index_dates(
            NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            &["2026-09-25", "2026-10-01", "2026-10-02"],
        );
        assert!(dates.len() >= 55, "序列长度应足够，{}", dates.len());
        let derived = derive_holidays_from_kline_dates(&dates, today);
        for h in ["2026-09-25", "2026-10-01", "2026-10-02"] {
            assert!(derived.iter().any(|d| d == h), "真休市 {h} 必须被推出：{derived:?}");
        }
        assert!(
            !derived.iter().any(|d| d == "2026-10-09"),
            "10-09 有 K 线 ⇒ 绝不能被标成休市：{derived:?}"
        );
    }

    /// **核心回归（本次实测形态）**：序列残缺（止于 09-28）时，
    /// 绝不能把 09-30 ~ 10-22 这些**真实交易日**标成休市。
    ///
    /// 实测：指数 K 线止于 2026-09-28 时，原实现造出 18 个假休市日，
    /// 导致时间旅行批次拒绝全部 2026-10 的锚点日。
    #[test]
    fn stale_series_does_not_fabricate_holidays() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let full = synth_index_dates(
            NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            &["2026-09-25", "2026-10-01", "2026-10-02"],
        );
        // 模拟「vendor 只返回到 09-28」
        let stale: Vec<String> =
            full.iter().filter(|d| d.as_str() <= "2026-09-28").cloned().collect();
        assert!(stale.len() >= 40, "截断后仍应足够长，{}", stale.len());
        let derived = derive_holidays_from_kline_dates(&stale, today);
        for d in ["2026-09-30", "2026-10-09", "2026-10-12", "2026-10-22"] {
            assert!(
                !derived.iter().any(|x| x == d),
                "{d} 是真实交易日，不得被标成休市：{derived:?}"
            );
        }
        // 但序列**内部**的真休市仍要保留（09-25 在截断点之前）
        assert!(
            derived.iter().any(|x| x == "2026-09-25"),
            "内部真休市 09-25 必须保留：{derived:?}"
        );
    }

    /// 稀疏序列（根数少 / 缺失率过高）⇒ 整段放弃推导，而不是推出一堆假休市。
    #[test]
    fn sparse_series_is_rejected_entirely() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        // 只有 3 根 ⇒ 低于 MIN_BARS_FOR_DERIVATION（区间极窄时几乎每个工作日都像空洞）
        let few = synth_index_dates(
            NaiveDate::from_ymd_opt(2026, 10, 13).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            &[],
        );
        let few: Vec<String> = few.into_iter().take(3).collect();
        assert!(derive_holidays_from_kline_dates(&few, today).is_empty());
    }
}
