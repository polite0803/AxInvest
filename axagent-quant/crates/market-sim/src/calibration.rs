//! 参数校准器 —— 扫描参数网格，寻找使模拟产出最接近 A 股 Stylized Facts 的参数组合。
//!
//! ## 校准结果（2026-07-03）
//!
//! 100 组拉丁超立方 × 5s/路径 扫描，Bar 聚合 10ms：
//!
//! ```text
//! mm_spread_bps:         35
//! mm_quote_size:         634
//! noise_act_prob:        0.27
//! noise_price_noise_bps: 32
//! momentum_threshold:    0.0035
//! score:                 3.18
//! ```
//!
//! 这些参数已导出为 `BEST_PARAMS` 常量，当前被以下位置消费：
//!
//! - `src-tauri/src/commands/market_sim.rs` —— 三个前端可见命令
//!   （`market_sim_run` / `market_sim_run_mc` / `market_sim_run_strategy`）
//! - `src-tauri/src/commands/wf_des.rs` —— DES 集成命令（当前无 UI 入口）
//!
//! ⚠️ 本节原表述为「被 `MarketSimulationTool` 和 `QuantStrategyAgent` 默认使用」，
//! 该表述**不成立**：前者类型在全仓不存在，后者并未引用 `BEST_PARAMS`。
//! 注释是这里唯一的规格，故保留此更正说明以防反复。

//! ## 用法
//!
//! ```rust,no_run
//! use axagent_market_sim::{CalibrationRunner, CalibrationParam};
//!
//! let mut runner = CalibrationRunner::new(1000, 5);
//! let results = runner.run();
//! if let Some(best) = results.first() {
//!     println!("最佳参数: {:?}", best.param);
//!     println!("评分: {:.2}", best.score);
//! }
//! ```

use serde::{Deserialize, Serialize};

use crate::agent::{ExchangeAgent, MarketMakerAgent, MomentumAgent, NoiseAgent, ValueAgent};
use crate::config::SimConfig;
use crate::kernel::SimKernel;
use crate::stylized_facts::{StylizedFacts, TargetRange};
use crate::types::*;

/// 校准参数组合
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CalibrationParam {
    /// 做市商价差（基点）
    pub mm_spread_bps: i64,
    /// 做市商每档挂单量
    pub mm_quote_size: u64,
    /// 噪声 Agent 下单概率
    pub noise_act_prob: f64,
    /// 噪声 Agent 价格噪声（基点）
    pub noise_price_noise_bps: i64,
    /// 动量 Agent 阈值
    pub momentum_threshold: f64,
}

impl Default for CalibrationParam {
    fn default() -> Self {
        Self {
            mm_spread_bps: 30,
            mm_quote_size: 500,
            noise_act_prob: 0.3,
            noise_price_noise_bps: 30,
            momentum_threshold: 0.003,
        }
    }
}

/// 校准结果：参数 + 评分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationResult {
    pub param: CalibrationParam,
    pub score: f64,
    pub stylized_facts: StylizedFactsF64,
    pub total_trades: usize,
}

/// 「成交不足」阈值：低于它无法计算风格化事实（`StylizedFacts`），分数无意义。
pub const MIN_TRADES_FOR_FACTS: usize = 20;

/// 哨兵分数：**模拟跑通但成交不足** ⇒ 无效结果。
pub const SCORE_NO_TRADES: f64 = 999.0;

/// 哨兵分数：**模拟内核直接返回 `Err`** ⇒ 无效结果。
pub const SCORE_SIM_FAILED: f64 = 9999.0;

/// 「有效分数」的上界：`score >= SCORE_VALID_MAX` 一律视为哨兵/无效。
///
/// 判据用 `< SCORE_NO_TRADES` 而非 `!= 999.0`：拟合误差是连续量，
/// 未来哨兵取值改变时这里仍能把「异常大的分数」拦住。
pub const SCORE_VALID_MAX: f64 = SCORE_NO_TRADES;

impl CalibrationResult {
    /// 本次参数是否给出**可比较**的校准结果。
    ///
    /// 为什么必须有这个方法：`run()` 只做升序排序，**没有任何有效性判断**，
    /// 而哨兵恰好是大数、排完序躺在末尾 —— 于是「**全部**参数都失败」时
    /// `results[0]` 仍会被上层当作「最佳参数」打印出来（分数渲染成 `N/A`，
    /// 但「最佳参数」的结论照样成立）。这就是「把失败当最优」的成因。
    pub fn is_valid(&self) -> bool {
        self.total_trades >= MIN_TRADES_FOR_FACTS && self.score < SCORE_VALID_MAX
    }
}

/// 最佳校准参数（A 股，2026-07-03）
///
/// 来源：`test_full_calibration_scan`，100 组参数 × 5s/路径
pub const BEST_PARAMS: CalibrationParam = CalibrationParam {
    mm_spread_bps: 35,
    mm_quote_size: 634,
    noise_act_prob: 0.27,
    noise_price_noise_bps: 32,
    momentum_threshold: 0.0035,
};

/// Stylized Facts 的浮点表示（用于序列化输出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StylizedFactsF64 {
    pub kurtosis: f64,
    pub hurst: f64,
    pub lb_pvalue: f64,
    pub leverage_corr: f64,
    pub passed: Vec<String>,
    pub failed: Vec<String>,
}

/// 参数网格扫描器
pub struct CalibrationRunner {
    reference_price: Price,
    /// 每组参数运行的虚拟时间（ns）
    pub sim_duration_ns: SimTimestamp,
    /// Bar 聚合窗口（纳秒），默认 10ms
    pub bar_ns: u64,
    /// 要测试的参数组合（可手动设置，默认等间距网格）
    pub params: Vec<CalibrationParam>,
}

impl CalibrationRunner {
    /// 创建校准器
    ///
    /// - `reference_price`: 参考价（分）
    /// - `n_grid`: 每个维度的网格点数（默认网格: 5^5=3125 组，实际用拉丁超立方采样 N 组）
    pub fn new(reference_price: Price, n_params: usize) -> Self {
        let params = Self::generate_latin_hypercube(n_params);
        Self {
            reference_price,
            sim_duration_ns: 600_000_000_000, // 10 分钟虚拟时间
            bar_ns: 10_000_000,               // 10ms Bar 窗口
            params,
        }
    }

    /// 设置模拟时长
    pub fn with_duration(mut self, duration_ns: SimTimestamp) -> Self {
        self.sim_duration_ns = duration_ns;
        self
    }

    /// 生成拉丁超立方采样参数
    fn generate_latin_hypercube(n: usize) -> Vec<CalibrationParam> {
        let count = n.clamp(20, 5000);

        // 各参数的范围
        let spread_range = (10.0, 100.0); // bps
        let qty_range = (100.0, 2000.0);
        let prob_range = (0.05, 0.8);
        let noise_range = (5.0, 100.0); // bps
        let mom_range = (0.001, 0.01);

        let mut params = Vec::with_capacity(count);

        for i in 0..count {
            // 用确定性伪随机产生拉丁超立方采样
            let t = i as f64 / count as f64;
            let phi1 = (i as f64 * 1.618033988749895).fract();
            let phi2 = (i as f64 * std::f64::consts::E).fract();
            let phi3 = (i as f64 * std::f64::consts::PI).fract();
            let phi4 = (i as f64 * std::f64::consts::SQRT_2).fract();
            let phi5 = (i as f64 * 0.5772156649).fract();

            // 分层 + 抖动
            let u1 = (t + phi1 * 1.0 / count as f64).fract();
            let u2 = (t + phi2 * 1.0 / count as f64).fract();
            let u3 = (t + phi3 * 1.0 / count as f64).fract();
            let u4 = (t + phi4 * 1.0 / count as f64).fract();
            let u5 = (t + phi5 * 1.0 / count as f64).fract();

            params.push(CalibrationParam {
                mm_spread_bps: (spread_range.0 + u1 * (spread_range.1 - spread_range.0)).round()
                    as i64,
                mm_quote_size: (qty_range.0 + u2 * (qty_range.1 - qty_range.0)).round() as u64,
                noise_act_prob: prob_range.0 + u3 * (prob_range.1 - prob_range.0),
                noise_price_noise_bps: (noise_range.0 + u4 * (noise_range.1 - noise_range.0))
                    .round() as i64,
                momentum_threshold: mom_range.0 + u5 * (mom_range.1 - mom_range.0),
            });
        }

        params
    }

    /// 用给定参数运行一次模拟，返回 Stylized Facts
    fn run_single(&self, param: &CalibrationParam, seed: u64) -> CalibrationResult {
        let price = self.reference_price;

        let config = SimConfig {
            max_time_ns: self.sim_duration_ns,
            seed,
            stock_code: "000001".to_string(),
            reference_price: price,
            default_latency_ns: 100,
            ..Default::default()
        };

        let mut kernel = SimKernel::new(config);

        // 注册 Agent
        kernel.register(Box::new(ExchangeAgent::with_tick_size("exchange", 1)));

        // 做市商
        kernel.register(Box::new(MarketMakerAgent::new(
            "mm",
            param.mm_spread_bps,
            param.mm_quote_size,
            5000,
            0.1,
            500_000, // 0.5ms 报价间隔
            price,
        )));

        // 动量
        if param.momentum_threshold > 0.0 {
            kernel.register(Box::new(MomentumAgent::new(
                "momentum",
                10, // lookback
                param.momentum_threshold,
                200,
                3000,
                1_000_000, // 1ms
                price as f64,
            )));
        }

        // 噪声 × 3（不同间隔错开）
        for i in 0..3 {
            kernel.register(Box::new(NoiseAgent::new(
                format!("noise_{}", i),
                400_000 + i as u64 * 150_000, // 400-700μs
                param.noise_act_prob,
                50,
                param.noise_price_noise_bps,
                price,
                seed + i as u64, // 修复 H3.6: 传递种子保证可复现
            )));
        }

        // 价值 Agent × 1
        kernel.register(Box::new(ValueAgent::new(
            "value",
            (price as f64 * 1.02) as i64,
            30,
            300,
            3000,
            2_000_000,
        )));

        match kernel.run() {
            Ok(result) => {
                let total_trades = result.trades.len();
                let facts = if total_trades >= MIN_TRADES_FOR_FACTS {
                    // 使用 Bar 聚合（10ms 窗口）
                    StylizedFacts::from_bars(&result.trades, self.bar_ns)
                } else {
                    StylizedFacts {
                        kurtosis: 0.0,
                        skewness: 0.0,
                        hurst_exponent: 0.5,
                        lb_stat: 0.0,
                        lb_pvalue: 1.0,
                        leverage_corr: 0.0,
                        n_observations: total_trades,
                        passed: Vec::new(),
                        failed: vec![format!(
                            "成交不足: {} < {}",
                            total_trades, MIN_TRADES_FOR_FACTS
                        )],
                    }
                };

                let score = if total_trades >= MIN_TRADES_FOR_FACTS {
                    facts.score(&TargetRange::default())
                } else {
                    SCORE_NO_TRADES
                };

                CalibrationResult {
                    param: *param,
                    score,
                    stylized_facts: StylizedFactsF64 {
                        kurtosis: facts.kurtosis,
                        hurst: facts.hurst_exponent,
                        lb_pvalue: facts.lb_pvalue,
                        leverage_corr: facts.leverage_corr,
                        passed: facts.passed,
                        failed: facts.failed,
                    },
                    total_trades,
                }
            },
            Err(e) => CalibrationResult {
                param: *param,
                score: SCORE_SIM_FAILED,
                stylized_facts: StylizedFactsF64 {
                    kurtosis: 0.0,
                    hurst: 0.5,
                    lb_pvalue: 1.0,
                    leverage_corr: 0.0,
                    passed: vec![],
                    failed: vec![format!("模拟失败: {}", e)],
                },
                total_trades: 0,
            },
        }
    }

    /// 运行全部参数扫描
    ///
    /// 修复 L-11: 当前实现是串行的（map + collect）。
    /// 由于 run_single 需要 &mut self（共享可变状态如 RNG），无法直接用 rayon par_iter。
    /// 若需并行化，应将 run_single 改为接受独立状态（如克隆 self 或拆分不可变部分），
    /// 然后用 rayon 或 tokio task 并行执行。当前规模（通常 < 100 个参数组合）下
    /// 串行性能可接受，并行化收益有限。
    pub fn run(&mut self) -> Vec<CalibrationResult> {
        // 修复 M-DEF-5: 原 if/else 两个分支都调用 `self.run_single(param, seed)`，
        // 是死代码（可能源于早期"测试模式跳过部分扫描"的实验残留）。
        // 直接调用 run_single，简化逻辑。
        let mut results: Vec<CalibrationResult> = self
            .params
            .iter()
            .enumerate()
            .map(|(i, param)| {
                let seed = 1000 + i as u64;
                self.run_single(param, seed)
            })
            .collect();

        // 升序：有效分数的量级远小于哨兵，故哨兵自然落到末尾。
        // ⚠ 「排在末尾」**不等于**「不会被选中」—— 全部失败时 `results[0]` 仍是哨兵。
        // 取「最佳」必须走 `best_valid` / `best_n`（两者按 `is_valid` 过滤），
        // **不要直接读 `results[0]`**。
        results.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// 最佳**有效**结果；全部参数都失败时返回 `None`。
    ///
    /// **不要用 `results[0]` 代替本方法** —— 全失败时它是哨兵值（见 [`SCORE_SIM_FAILED`]）。
    ///
    /// ⚠ **本方法不依赖入参有序**。分数是「拟合误差」，**越小越好**，故取有效项中
    /// `score` 最小者。曾经的写法是 `find(|r| r.is_valid())`，它把「入参已升序」
    /// 这一前置条件**藏进了实现**：`run()` 恰好排过序因而正确，但本方法是公开 API，
    /// 任何调用方传入乱序切片都会**静默拿到错误答案**（不报错、结果看着也合理）。
    /// 这与「哨兵被当成最优」同属一族缺陷 —— 都是把「排序/有效性」的先验
    /// 隐式寄托在调用方身上。对已排序的 `run()` 输出，`min_by` 与原 `find` 等价。
    pub fn best_valid<'a>(
        &self,
        results: &'a [CalibrationResult],
    ) -> Option<&'a CalibrationResult> {
        results
            .iter()
            .filter(|r| r.is_valid())
            .min_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// 获取最佳 N 个结果 —— **只取有效结果**。
    ///
    /// 无效结果（成交不足 / 模拟失败）不入选：其哨兵分数只用于排序占位，
    /// 不表示可比优劣。全部无效时返回空 `Vec`（而不是"前 N 个哨兵"）。
    ///
    /// 与 [`best_valid`](Self::best_valid) 同样**不依赖入参有序**：内部自行按
    /// `score` 升序取前 `n`，避免「调用方忘了排序 ⇒ 返回的不是最佳 N 个」。
    pub fn best_n(&self, results: &[CalibrationResult], n: usize) -> Vec<CalibrationResult> {
        let mut valid: Vec<CalibrationResult> =
            results.iter().filter(|r| r.is_valid()).cloned().collect();
        valid.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
        valid.truncate(n);
        valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calibrator_creates_params() {
        let runner = CalibrationRunner::new(1000, 50);
        assert_eq!(runner.params.len(), 50);
        assert!(runner.params[0].mm_spread_bps >= 10);
        assert!(runner.params[0].mm_spread_bps <= 100);
    }

    #[test]
    fn test_full_calibration_scan() {
        // 完整校准扫描：100 组拉丁超立方参数，500ms/路径
        let mut runner = CalibrationRunner::new(1000, 100);
        runner.sim_duration_ns = 5_000_000_000; // 5s
        let results = runner.run();

        assert!(!results.is_empty());
        println!("\n=== A 股校准扫描结果 (Bar 聚合) ===");
        println!("参数组合数: {} | 每路径时长: {}s | Bar 窗口: {}ms", results.len(), 5, 10);
        println!();

        // 前 5 名
        println!("--- Top 5 ---");
        for (i, r) in results.iter().take(5).enumerate() {
            print_best(i + 1, r);
        }

        // 后 3 名（最差）
        println!("\n--- Bottom 3 ---");
        for (i, r) in results.iter().rev().take(3).enumerate() {
            print_best(results.len() - i, r);
        }

        // 最佳参数简介 —— 必须走 best_valid：全失败时 results[0] 是哨兵，
        // 直接读它就会把「一次都没跑成」报告成「最佳参数」。
        let best = match runner.best_valid(&results) {
            Some(b) => b,
            None => panic!(
                "校准扫描无任何有效结果：{} 组参数全部命中哨兵（成交不足 < {} 或模拟失败）\
                 ⇒ 先修仿真链路，不得按「最佳参数」采信",
                results.len(),
                MIN_TRADES_FOR_FACTS
            ),
        };
        println!("\n--- 最佳参数（首个**有效**结果；哨兵不计入） ---");
        println!("  mm_spread_bps:        {}", best.param.mm_spread_bps);
        println!("  mm_quote_size:        {}", best.param.mm_quote_size);
        println!("  noise_act_prob:       {:.4}", best.param.noise_act_prob);
        println!("  noise_price_noise_bps: {}", best.param.noise_price_noise_bps);
        println!("  momentum_threshold:   {:.5}", best.param.momentum_threshold);
        println!("  score:                {:.2}", best.score);
        println!("  trades:               {}", best.total_trades);

        assert!(results[0].score < results[results.len() - 1].score);
    }

    fn print_best(rank: usize, r: &CalibrationResult) {
        // 判据不再是魔数 `score >= 100.0`，而是有效性谓词
        // （哨兵只是「无效」的一种；未来新增无效形态无需再改这里）
        let score_str = if r.is_valid() {
            format!("{:.2}", r.score)
        } else {
            "N/A".to_string()
        };
        println!(
            "  #{:<3} score={:>8} trades={:<5} spread={:>3}bps size={:>4} noise_p={:.2} noise_bps={:>3} mom={:.4}{}",
            rank,
            score_str,
            r.total_trades,
            r.param.mm_spread_bps,
            r.param.mm_quote_size,
            r.param.noise_act_prob,
            r.param.noise_price_noise_bps,
            r.param.momentum_threshold,
            if r.is_valid() { "" } else { "  ⚠无效" }
        );
    }

    /// 选择逻辑的**纯单元测试**（不跑仿真）。
    ///
    /// 为什么用合成数据：真实扫描是否产生哨兵取决于内核行为与成交数，
    /// 无法稳定复现「全失败」这一关键形态；只有合成数据能把三种形态
    /// （全失败 / 部分失败 / 全有效）全部锁死。
    #[test]
    fn test_best_selection_ignores_sentinels() {
        let mk = |score: f64, trades: usize| CalibrationResult {
            param: CalibrationParam::default(),
            score,
            stylized_facts: StylizedFactsF64 {
                kurtosis: 0.0,
                hurst: 0.5,
                lb_pvalue: 1.0,
                leverage_corr: 0.0,
                passed: vec![],
                failed: vec![],
            },
            total_trades: trades,
        };

        // ⚠ 本切片**故意不排序**（`3.0` 排在 `12.5` 之后）：选择函数不得依赖
        // 「入参已升序」这一隐式前置条件。`run()` 恰好排过序，但 `best_valid`
        // 是公开 API，调用方完全可能传乱序/手工构造的切片 —— 曾经的
        // `find(|r| r.is_valid())` 在这里会返回 `12.5`，静默给出错误答案。
        // **不要为了让本测试变绿而给下面这个 vec 排序**（那等于删掉守卫）。
        let results = vec![
            mk(SCORE_SIM_FAILED, 0), // 内核 Err
            mk(SCORE_NO_TRADES, 0),  // 成交不足
            mk(12.5, MIN_TRADES_FOR_FACTS),
            mk(3.0, 100),
        ];
        let runner = CalibrationRunner::new(1000, 20);

        assert!(!results[0].is_valid(), "模拟失败必须判无效");
        assert!(!results[1].is_valid(), "成交不足必须判无效");
        assert!(results[2].is_valid() && results[3].is_valid(), "两项应有效");

        let best = runner.best_valid(&results).expect("存在有效结果时应返回最优");
        assert_eq!(best.score, 3.0, "最优必须取自**有效**结果");

        let top2 = runner.best_n(&results, 2);
        assert_eq!(top2.len(), 2, "应只返回 2 个有效结果");
        assert!(top2.iter().all(|r| r.is_valid()), "best_n 不得夹带哨兵");
        // 是「最**优**的 n 个」而非「最**先**的 n 个有效项」—— 无序入参下必须自行排序
        assert_eq!(top2[0].score, 3.0, "best_n 首项必须是全局最小有效分数");
        assert_eq!(top2[1].score, 12.5, "best_n 次项应为次小有效分数");

        // 反向对照：全失败 ⇒ 不得凭空造出「最佳」
        let all_bad = vec![mk(SCORE_SIM_FAILED, 0), mk(SCORE_NO_TRADES, 3)];
        assert!(runner.best_valid(&all_bad).is_none(), "全失败时不得返回「最佳参数」");
        assert!(runner.best_n(&all_bad, 5).is_empty(), "全失败时 best_n 必须为空");
    }

    #[test]
    fn test_trade_pipeline_debug() {
        use crate::agent::*;
        use crate::config::*;
        use crate::kernel::*;

        // 仅 1 个 MM + 1 个 Noise，跑 100ms
        let mut kernel = SimKernel::new(SimConfig {
            max_time_ns: 100_000_000,
            default_latency_ns: 1_000,
            reference_price: 1000,
            ..Default::default()
        });

        kernel.register(Box::new(ExchangeAgent::with_tick_size("exchange", 1)));
        kernel.register(Box::new(MarketMakerAgent::new("mm", 30, 500, 5000, 0.1, 1_000_000, 1000)));
        kernel.register(Box::new(NoiseAgent::new("noise", 1_000_000, 0.8, 200, 50, 1000, 42)));

        let start = std::time::Instant::now();
        match kernel.run() {
            Ok(r) => {
                let elapsed = start.elapsed();
                println!(
                    "[DEBUG] trades={} events={} wall={}ms | sim={}ns agents={} orders={}",
                    r.trades.len(),
                    r.total_events,
                    r.wall_clock_ms,
                    r.sim_time_ns,
                    r.stats.agent_count,
                    r.stats.total_orders
                );
                assert!(r.total_events > 0, "应该有事件被处理");
                // Wall time should be measurable
                println!("[DEBUG] wall elapsed={}ms", elapsed.as_millis());
            },
            Err(e) => panic!("模拟失败: {}", e),
        }
    }
}
