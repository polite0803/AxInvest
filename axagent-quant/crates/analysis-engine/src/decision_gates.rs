// SPDX-License-Identifier: AGPL-3.0-only

//! 决策否决门开关（**产品决策可配置化**，2026-10-26 用户裁决）
//!
//! ## 为什么有这个模块
//!
//! 此前三道否决门是**写死在 Rhai 脚本里的单向否决**（只拦买入、不抬信号）：
//!
//! | 门 | 判据 | 落点 |
//! |---|---|---|
//! | 风控门 | 风险档 = 高 / 极高 ⇒ 买入→持有 / 观望 | `portfolio_formula::apply_risk_veto`（4 份档位脚本 + 主链各调一次） |
//! | 情绪门 | 全市场封板率 < 0.5 ⇒ 不出买入/增持 | `portfolio-mgr-h-ultra-short.rhai` 的 `breadth_required` |
//! | 估值门 | DCF upside ≤ −35% 或 PE ≥ 自身 5 年 80 分位 ⇒ 不出买入/增持 | `portfolio-mgr-h-long.rhai` 的 `pm_overvalued_veto` |
//!
//! 2026-10-26 实测（生产库，`horizon_decisions[*].originalAction` 口径）：
//! 10-16 起共 24 条档内原始买入 ⇒ **9 条保留买入、15 条被改写**，
//! 且 **15 条 100% 来自风控门**（情绪门与估值门拦下 0 条）。
//! 被否决 ⇒ `position = 0` ⇒ 该档「不可执行」⇒ 顶层选档拿不到买入档
//! ⇒ **顶层「买入」自 10-16 起 209 次分析为 0**。
//!
//! ⇒ 本模块把这三道门做成**运行期可切换的开关**，让产品决策（风控优先 vs 可执行性优先）
//! **不必改图、不必升模板版本**即可调整。
//!
//! ## 为什么是「运行期读内存 + 启动/改配置时刷新」而不是「每轮读库」
//!
//! Rhai 宿主函数是**同步闭包**（`register_fn` 不接受 async），无法在脚本里 await DB。
//! 本模块沿用本仓既有范式（`conditional_order_bridge::ConditionalOrderBridge`）：
//! **DB 是定义的唯一权威 → 进程内 static 存快照 → `refresh()` 时同步**。
//!
//! ## 语义（关键）
//!
//! * `enabled = false`（默认）⇒ **完全保持改动前的行为**（三门照常否决）。
//! * `enabled = true` ⇒ **该门不再否决买入**（但**不抬信号**：门只单向拦，不反向造信号）。
//! * 三门**互相独立**：可以只放开风控门（保留情绪/估值的安全垫）。
//! * 「档内买入是否被顶层采纳」是**第四个独立开关**（`tier_buy`）——
//!   它管的是**顶层派生**（选档后是否采纳档内买入），与三门否决是两层事。
//!
//! ## 单一真相源纪律
//!
//! 本模块是**开关状态的唯一真相源**；Rhai 侧只能经 `pm_gate_enabled` 读，
//! **不得**在脚本里再写一份字面量默认值（那是「同名不同义」的温床 —— 见 MEMORY-RULES #738）。

use std::sync::atomic::{AtomicU8, Ordering};

/// 三道否决门 + 顶层采纳档内买入，共 4 个开关。
///
/// 编码用 `AtomicU8` 的 bit 位：0 = 关（默认，保持原行为），1 = 开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// 风控门：风险档 高/极高 ⇒ 禁止加仓/禁止持仓。
    RiskVeto,
    /// 情绪门：全市场封板率不足 ⇒ 不出买入/增持。
    Sentiment,
    /// 估值门：严重高估 ⇒ 不出买入/增持。
    Valuation,
    /// 顶层派生：顶层是否采纳档内的「买入」结论。
    TierBuy,
}

impl Gate {
    /// 稳定的字符串键（DB `settings.key` 与 Rhai 调用参数共用）。
    ///
    /// ⚠ 键名一旦落库就**不可改**（改了等于换了一批配置，用户已设的值全部失联）
    /// ⇒ 与 DB 侧键名一一对应，改动须同步迁移脚本。
    pub fn key(self) -> &'static str {
        match self {
            Gate::RiskVeto => "gate.risk_veto_enabled",
            Gate::Sentiment => "gate.sentiment_enabled",
            Gate::Valuation => "gate.valuation_enabled",
            Gate::TierBuy => "gate.tier_buy_enabled",
        }
    }

    /// 按字符串键解析（未知键 ⇒ `None`，**不静默当作 false**）。
    pub fn from_key(key: &str) -> Option<Gate> {
        match key {
            "gate.risk_veto_enabled" => Some(Gate::RiskVeto),
            "gate.sentiment_enabled" => Some(Gate::Sentiment),
            "gate.valuation_enabled" => Some(Gate::Valuation),
            "gate.tier_buy_enabled" => Some(Gate::TierBuy),
            _ => None,
        }
    }

    /// 该门在 bit 掩码中的位置。
    const fn bit(self) -> u8 {
        match self {
            Gate::RiskVeto => 1 << 0,
            Gate::Sentiment => 1 << 1,
            Gate::Valuation => 1 << 2,
            Gate::TierBuy => 1 << 3,
        }
    }
}

/// 四门状态位图（bit 0..3 对应 `Gate::bit`）。
static GATES: AtomicU8 = AtomicU8::new(0);

/// **测试冻结计数器**（原子）。
///
/// 存在理由：门状态是**进程级全局**，而种子一致性测试会**真的跑 Rhai 脚本**
/// ⇒ 脚本里的 `pm_gate_enabled` 会读到全局状态 ⇒ 若另一个测试此刻把门打开，
/// 那些断言「门关时行为」或「`entryGatePassed` 必须为 false」的测试会**随机变红**
/// （且红的原因与被测代码无关 —— 这类「幽灵红」比红本身更费时间）。
///
/// ⇒ `with_gates_disabled` 期间把计数置正 ⇒ `is_open` **一律返回 false**
/// ⇒ 脚本无论怎么跑都读到「门全关」。
///
/// ⚠ **为什么不用读写锁**（曾经的写法，已踩坑）：
///   `RwLock` 的**写锁不可重入** —— 持写锁的线程再去拿读锁会**死锁**。
///   而本模块的读侧正是 `is_open`（脚本执行时同步调用）⇒ 写锁 + 读锁 = 死锁。
///   实测症状：`cargo test -p axagent --lib` 的测试二进制跑 26 分钟、CPU 仅 207s
///   （典型「锁等待」而非计算）⇒ 必须杀进程才能继续。
///   ⇒ 改用**原子计数器**：无锁、可嵌套、无死锁。
static TEST_FREEZE_DEPTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 读取某门当前是否**放开**（`true` = 不再否决）。
///
/// 默认全 `false` ⇒ **与本模块引入前的行为逐位一致**。
pub fn is_open(gate: Gate) -> bool {
    // 冻结中 ⇒ 一律按「关」读（测试隔离；见 `TEST_FREEZE_DEPTH` 的文档）。
    if TEST_FREEZE_DEPTH.load(Ordering::Acquire) > 0 {
        return false;
    }
    GATES.load(Ordering::Relaxed) & gate.bit() != 0
}

/// 测试辅助：在闭包执行期间**冻结**门状态（读侧一律按「关」）。
///
/// 用于「脚本行为依赖门状态」的测试 ⇒ 与其它测试隔离，避免幽灵红。
///
/// ⚠ **可嵌套**（计数器语义）⇒ 内层调用不会提前解冻。
/// ⚠ 生产路径**不得**调用（本函数只服务测试）。
pub fn with_gates_disabled<F: FnOnce()>(f: F) {
    TEST_FREEZE_DEPTH.fetch_add(1, Ordering::AcqRel);
    f();
    TEST_FREEZE_DEPTH.fetch_sub(1, Ordering::AcqRel);
}

/// 覆盖式写入四门状态（供 `refresh` 与设置保存后调用）。
///
/// 未知门 ⇒ 忽略（**不静默开**：拼错的键名不得变成「放开风控门」）。
pub fn set_state(
    open_risk_veto: bool,
    open_sentiment: bool,
    open_valuation: bool,
    open_tier_buy: bool,
) {
    let mut bits = 0u8;
    if open_risk_veto {
        bits |= Gate::RiskVeto.bit();
    }
    if open_sentiment {
        bits |= Gate::Sentiment.bit();
    }
    if open_valuation {
        bits |= Gate::Valuation.bit();
    }
    if open_tier_buy {
        bits |= Gate::TierBuy.bit();
    }
    GATES.store(bits, Ordering::Relaxed);
}

/// 按 DB 原始字符串值刷新单个门（`"true"` / `"false"` / 其它）。
///
/// ⚠ **只有字面量 `"true"` 才算开**：其它一切（含 `"1"` / `"yes"` / 空串）都算关。
/// 理由：本开关控制的是「是否绕过风控」——**判错的方向必须偏向保守**。
pub fn refresh_from_raw(gate: Gate, raw: Option<&str>) {
    let open = raw.map(|v| v.trim() == "true").unwrap_or(false);
    let mut bits = GATES.load(Ordering::Relaxed);
    if open {
        bits |= gate.bit();
    } else {
        bits &= !gate.bit();
    }
    GATES.store(bits, Ordering::Relaxed);
}

/// 当前四门状态（供设置面板回显 / 审计留痕）。
pub fn current_state() -> Vec<(Gate, bool)> {
    vec![
        (Gate::RiskVeto, is_open(Gate::RiskVeto)),
        (Gate::Sentiment, is_open(Gate::Sentiment)),
        (Gate::Valuation, is_open(Gate::Valuation)),
        (Gate::TierBuy, is_open(Gate::TierBuy)),
    ]
}

/// 供 Rhai 宿主函数用的字符串 → 门解析（Rhai 侧只传字符串）。
///
/// 未知门名 ⇒ 返回 `false`（**不放行**）：脚本里若传错门名，保守方向是「仍按原门否决」。
pub fn is_open_by_name(name: &str) -> bool {
    match Gate::from_key(name) {
        Some(g) => is_open(g),
        None => false,
    }
}

/// 从 DB（`settings` 表）**全量刷新**四门状态。
///
/// DB 是定义的唯一权威（沿用本仓 `conditional_order_bridge` 的范式：
/// **表 → 进程内快照**），Rhai 侧只读快照、**不查库**。
///
/// ⚠ 读不到的键 ⇒ **按关处理**（保守方向）：DB 里没有该行 = 用户没开 = 保持原行为。
/// ⚠ 读失败（DB 不可达）⇒ 保持**当前**内存状态不变（不因一次查询失败把已开的门关掉）。
pub async fn refresh_from_db(db: &sea_orm::DatabaseConnection) -> Result<[bool; 4], String> {
    use axagent_dao::repo::settings as settings_repo;

    let mut states = [false; 4];
    let mut read_error: Option<String> = None;

    for (idx, gate) in
        [Gate::RiskVeto, Gate::Sentiment, Gate::Valuation, Gate::TierBuy].into_iter().enumerate()
    {
        match settings_repo::get_setting(db, gate.key()).await {
            Ok(raw) => {
                // 只有字面量 "true" 才算开（判错方向偏向保守，见 refresh_from_raw 的注）。
                states[idx] = raw.as_deref().map(|v| v.trim() == "true").unwrap_or(false);
            },
            Err(e) => {
                read_error = Some(format!("读取 {} 失败: {e}", gate.key()));
                break;
            },
        }
    }

    if let Some(err) = read_error {
        // ⚠ 不写内存状态（保持上一次的有效快照）—— 一次查询失败不该把已开的门关掉。
        return Err(err);
    }

    set_state(states[0], states[1], states[2], states[3]);
    Ok(states)
}

/// 写入单个门到 DB 并**立即**生效（供设置命令调用）。
///
/// ⚠ 写 DB 成功但写内存失败不可能（内存写是无失败的原子操作）⇒ 顺序是先写 DB 再写内存。
pub async fn persist_and_apply(
    db: &sea_orm::DatabaseConnection,
    gate: Gate,
    open: bool,
) -> Result<(), String> {
    use axagent_dao::repo::settings as settings_repo;

    settings_repo::set_setting(db, gate.key(), if open { "true" } else { "false" })
        .await
        .map_err(|e| format!("写入 {} 失败: {e}", gate.key()))?;
    refresh_from_raw(gate, Some(if open { "true" } else { "false" }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 各测试之间共享 static 状态 ⇒ 用一把串行锁把测试整体串起来，
    /// 避免并行测试互相污染（`GATES` 是进程级全局）。
    // SAFETY: std::sync::Mutex 不跨 await —— `with_reset` 是同步函数，guard 只在其执行期间持有，
    // 全程无 await 点。依据 `clippy.toml` 的合法例外（形态同 `monitor.rs` 的 Recorder）。
    #[allow(clippy::disallowed_types)]
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_reset<F: FnOnce()>(f: F) {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_state(false, false, false, false);
        f();
        set_state(false, false, false, false);
    }

    #[test]
    fn default_state_keeps_every_gate_closed() {
        with_reset(|| {
            // 默认全关 ⇒ 与本模块引入前逐位一致（这是「不改变既有行为」的保证）。
            for (gate, open) in current_state() {
                assert!(!open, "门 {:?} 默认必须关闭", gate);
            }
        });
    }

    #[test]
    fn opening_one_gate_does_not_open_others() {
        with_reset(|| {
            set_state(true, false, false, false);
            assert!(is_open(Gate::RiskVeto));
            assert!(!is_open(Gate::Sentiment));
            assert!(!is_open(Gate::Valuation));
            assert!(!is_open(Gate::TierBuy));
        });
    }

    #[test]
    fn gate_keys_roundtrip() {
        with_reset(|| {
            for (gate, _) in current_state() {
                assert_eq!(Gate::from_key(gate.key()), Some(gate));
            }
            assert_eq!(Gate::from_key("gate.not_a_gate"), None);
        });
    }

    #[test]
    fn freeze_makes_reads_report_closed_and_is_nestable() {
        // ⚠ **回归锁**：冻结必须让**读侧**返回 false（脚本在冻结期间执行时读到的必须是「门关」），
        //   且**嵌套调用不得提前解冻**。
        //
        //   曾经的实现用 RwLock：持写锁的线程再拿读锁 ⇒ **死锁**
        //   （实测：主 crate 测试二进制跑 26 分钟、CPU 仅 207s ⇒ 杀进程才继续）。
        //   ⇒ 现实现是**原子计数器**（无锁 ⇒ 不可能死锁）。
        set_state(true, true, true, true);
        // 冻结中：读侧一律 false。
        with_gates_disabled(|| {
            assert!(!is_open(Gate::RiskVeto), "冻结期间读侧必须报「关」");
            assert!(!is_open(Gate::TierBuy), "冻结期间读侧必须报「关」");
            // 嵌套：内层结束后仍应冻结。
            with_gates_disabled(|| {
                assert!(!is_open(Gate::Sentiment), "嵌套冻结内读侧必须报「关」");
            });
            assert!(!is_open(Gate::Sentiment), "内层冻结结束后，外层仍在冻结 ⇒ 读侧仍必须是「关」");
        });
        // 冻结结束后恢复真实状态。
        assert!(is_open(Gate::RiskVeto), "冻结结束后必须读到真实状态（计数器归零）");
        set_state(false, false, false, false);
    }

    #[test]
    fn only_literal_true_opens_a_gate() {
        with_reset(|| {
            // 「判错偏向保守」：非字面量 true 一律按关处理。
            for raw in ["1", "yes", "TRUE", " truex", "", "null"] {
                refresh_from_raw(Gate::RiskVeto, Some(raw));
                assert!(
                    !is_open(Gate::RiskVeto),
                    "raw={raw:?} 不得把风控门打开（只有字面量 \"true\" 才算开）"
                );
            }
            refresh_from_raw(Gate::RiskVeto, Some(" true "));
            assert!(is_open(Gate::RiskVeto), "前后空白应容忍");
            refresh_from_raw(Gate::RiskVeto, None);
            assert!(!is_open(Gate::RiskVeto), "缺席 ⇒ 关");
        });
    }

    #[test]
    fn unknown_gate_name_is_never_open() {
        with_reset(|| {
            set_state(true, true, true, true);
            assert!(!is_open_by_name("gate.typo"), "未知门名不得放行");
            assert!(is_open_by_name("gate.risk_veto_enabled"));
        });
    }
}
