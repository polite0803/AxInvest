// SPDX-License-Identifier: AGPL-3.0-only
//!
//! A-2 回补验证 CLI —— 把 `reco_picks` 里**已有的**真实候选全部跑一遍 T+N 验证。
//!
//! ## 为什么需要这个 bin
//!
//! 决策回测的实现 (`run_decision_backtest_inner`) 有两个既有入口，但对「回补验证」
//! 都不合用：
//!
//! | 入口 | 问题 |
//! |---|---|
//! | Tauri 命令 `run_decision_backtest` | 前端**没有**触发点（全仓搜 `runDecisionBacktest` 零命中） |
//! | cron（`create_decision_backtest_cron`） | 分支**强制 `dryRun=false`** ⇒ 探测阶段没法先看结果 |
//!
//! 而回补验证必须**先 dry-run 看 `report.total` 是否接近预期**（去重键写错会把不同的票
//! 误判成重复，样本静默变少），所以需要一个能控制 `dryRun` 的入口。
//!
//! 本 bin 直接调**命令层与 cron 层共用的同一实现**，不复制任何业务逻辑。
//! 同形先例：`axagent-batch-rerun` 调 `run_batch_reflection_inner`。
//!
//! ## 用法
//!
//! ```text
//! cargo run --bin axagent-validate-backfill -- --help
//! cargo run --bin axagent-validate-backfill -- --t-plus-n 2              # dry-run（默认）
//! cargo run --bin axagent-validate-backfill -- --t-plus-n 2 --apply      # 写库
//! cargo run --bin axagent-validate-backfill -- --t-plus-n 2,5 --apply
//! ```
//!
//! ## 口径提醒（详见 `PLAN-a2-backfill-validation.md`）
//!
//! - `--max-picks` 默认 **2000**（不是业务默认的 200）：真实候选约 598 条，不放大就只覆盖 1/3；
//! - `--t-plus-n 90` 目前**一条都不可能成熟**（行情库只到 2026-10-23，最老的 pick 之后
//!   只有 54 个交易日）⇒ 跑了只会写满 `insufficient`；
//! - 已存在的 `(pick_id, t_plus_n)` 行**会被跳过**（`write_decision_validations` 幂等），
//!   所以补算 1,145 行 `insufficient` 之前**必须先删掉它们**，否则本 bin 跑多少次都不补。

use std::sync::Arc;

use axagent_lib::{
    AppState, RunDecisionBacktestRequest, axagent_home, create_app_state, init_database_with_dir,
    run_decision_backtest_inner,
};

const HELP: &str = "\
axagent-validate-backfill —— 回补决策验证（reco_picks → decision_validations）

用法:
  axagent-validate-backfill [选项]

选项:
  --t-plus-n <N[,M...]>  验证窗口（默认 2,5 —— 只跑当前确实能成熟的短窗口）
  --period <P>           只跑指定档位（short | mid | long | ultrashort）
  --max-picks <N>        pick 上限（默认 2000；业务默认 200 会漏掉大半候选）
  --fill-factors         给已有判定行补 factor_snapshot（不重算收益，不改 hit_outcome）
  --backfill-history     补齐全市场收盘历史（幂等：只补缺失行，不覆盖既有行）
  --fill-replay-outcomes 给回放分析补 outcome 判定（R-1：回放样本 18 → ~91）
  --backfill-performance  把「已成熟但从未物化」的反思样本回填进 strategy_performance
                         （R-3r：校准样本的分母；幂等，可重复跑）
  --run-reflection       只跑批量反思（**不重跑任何分析**；R-3r：反思不产出 ⇒ 校准样本不增长）
  --reflect-max <N>      --run-reflection 单轮处理上限（默认 20）
  --target-days <N>      回补目标交易日数（默认 250；UI 按钮硬编码为 90，这是历史只有 90 天的原因）
  --limit <N>            本批最多补多少只股票（默认 500，全市场约 12 批）
  --time-travel          发起时间旅行回放批次（R-2：把回放日从 3 个推向 30+）
  --as-of <D[,D...]>     指定回放锚点日（不给则自动选「快照至少一源齐全且 T+2 已到期」的日）
  --tt-min-confidence <N> 回放批次置信度门槛（默认 50）
  --tt-top-n <N>         回放批次每档 top-N（默认 20）
  --tt-timeout <SEC>     单批次等待上限秒数（默认 1800）
  --apply                真正写库（**不加则只 dry-run，不写任何行**）
  --help                 显示本帮助

关于 --time-travel:
  - 回放日的硬约束是 **as_of + N 个交易日 ≤ 今天**（否则 T+N 判不出来）
  - 建池快照**至少一源**齐全即可回放（缺的那源在 as-of 下走 vendor 历史通道，
    取不到即空榜；池子由另一源 + 静态兜底名单组成）。⚠ 两源都缺的日子仍排除。
    2026-10-26 起放宽：此前要求双源齐全，把 05-27/06-10/06-11/08-18/09-04/09-07/09-09
    这些「只有 get_industry_ranking 一源」的日子全部排除了 ⇒ long/mid 档的校准样本
    只能等 wall-clock（long 需 T+90 ⇒ 2027-01）。放宽后 **06-10 之后恰好 90 根 K 线**
    ⇒ 回放 06-10 立刻能让 long 档成熟、09-07 让 mid 档成熟。
  - 档位成熟只看「行情库里决策日之后有没有 N 根 K 线」，**不看今天**
    ⇒ 28/90 档在**足够早的回放日**上能当场判
  - 批次是**两段式**：guard 内选股与分析（此刻的「今天」= as_of），退出 guard 后做四档反思

关于 --fill-factors:
  - 只处理 `factor_snapshot IS NULL` 的判定行，**已算过的不会被覆盖**（幂等）
  - 因子只依赖收盘价与当日涨跌幅 ⇒ 与 T+N 窗口无关，**不受持仓期成熟度限制**
  - 序列严格截断到决策日（`left(generated_at,10)`），杜绝前视偏差
  - bar 不足 21 根的行会被跳过并计数，**不产假值**
  - quintile / 分位不在本步计算：需要当日全市场横截面，单票路径拿不到

说明:
  - 已存在的 (pick_id, t_plus_n) 会被跳过，不会覆盖前次结果
  - T+90 的成熟只看「行情库里决策日之后有没有 90 根 K 线」，**不看今天**
    ⇒ 回放足够早的日子（如 2026-06-10，其后恰好 90 根）时，T+90 能当场判
";

/// 命令行参数
struct Args {
    t_plus_n: Vec<i32>,
    max_picks: u32,
    period: Option<String>,
    apply: bool,
    /// 只补因子快照，不跑回测（B-3/F-2）
    fill_factors: bool,
    /// 补齐全市场收盘历史（解除 90 天密度上限）
    backfill_history: bool,
    /// 给回放分析补 outcome 判定（R-1）
    fill_replay_outcomes: bool,
    /// 是否把回放代 pick 一并验证（默认 false；打开后写入行仍带 as_of_date，两代可分）
    include_replay: bool,
    /// 回补目标交易日数
    target_days: u32,
    /// 本批最多补多少只股票
    limit: usize,
    /// 发起时间旅行回放批次（R-2）
    time_travel: bool,
    /// 显式指定 as_of 日（逗号分隔）；不给则自动选「快照至少一源齐全且 T+2 已到期」的日
    as_of: Option<Vec<String>>,
    /// 回放批次的最低置信度门槛
    tt_min_confidence: u8,
    /// 回放批次每档 top-N
    tt_top_n: usize,
    /// 单个批次的等待上限（秒）
    tt_timeout: u64,
    /// 是否执行逐只 replay 分析（默认**关闭**：LLM 免费档限速下实测单日 30~60 分钟，
    /// 而盈利能力样本走 `reco_picks → decision_validations`，不依赖该阶段）
    tt_analyze: bool,
    /// 回填「已成熟但从未物化」的反思样本进 `strategy_performance`（R-3r）。
    ///
    /// #31 四档物化通道是随反思执行内联跑的 ⇒ 只有该通道上线（2026-10-16）之后
    /// 执行的反思才有物化行；更早的成熟反思（实测 82 条里 50 条）从未进过
    /// `strategy_performance` ⇒ 校准样本的分母长期偏小 ⇒ `calibrationReady` 恒假
    /// ⇒ 置信封顶 ⇒ 动作降级 ⇒ 顶层「买入」为 0。
    ///
    /// 幂等（键 = `reflection-horizon:{决策 id}:{档}`）⇒ 可重复跑。
    backfill_performance: bool,
    /// 只跑一轮/多轮**批量反思**（不重跑任何分析）。
    ///
    /// 与 `axagent-batch-rerun --reflect-after` 的区别：后者**先重跑分析**再反思
    /// （副作用大：覆盖原分析、重置反思行）；本模式只消费**存量 pending 反思行**。
    ///
    /// 用途（R-3r）：校准样本的分母来自 `strategy_performance`，而物化是反思执行的
    /// 内联副产物 ⇒ **反思不产出 ⇒ 校准样本不增长** ⇒ 逐档样本量永远到不了门槛。
    run_reflection: bool,
    /// `--run-reflection` 的单轮处理上限（默认 20）
    reflect_max: u32,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut out = Args {
            // 默认只跑 2/5：当前能成熟的只有短窗（见 HELP）
            t_plus_n: vec![2, 5],
            max_picks: 2000,
            period: None,
            apply: false,
            fill_factors: false,
            backfill_history: false,
            fill_replay_outcomes: false,
            include_replay: false,
            // 250 个交易日 ≈ 一年：非重叠 T+5 窗口可达 ~50 个，才能把可检测 α 压到 0.5pp
            target_days: 250,
            limit: 500,
            time_travel: false,
            as_of: None,
            tt_min_confidence: 50,
            tt_top_n: 20,
            tt_timeout: 1800,
            // 默认关闭逐只分析：LLM 免费档限速下它会把单日批次拖到 30~60 分钟，
            // 而盈利能力样本不需要它（见 `TimeTravelBatchRequest::analyze`）。
            tt_analyze: false,
            backfill_performance: false,
            run_reflection: false,
            reflect_max: 20,
        };
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            match a.as_str() {
                "--t-plus-n" => {
                    let raw = it.next().ok_or("--t-plus-n 需要一个值（如 2 或 2,5）")?;
                    let mut v = Vec::new();
                    for part in raw.split(',') {
                        let n: i32 = part
                            .trim()
                            .parse()
                            .map_err(|e| format!("--t-plus-n 解析失败 `{part}`: {e}"))?;
                        if n <= 0 {
                            return Err(format!("--t-plus-n 必须是正数，收到 {n}"));
                        }
                        v.push(n);
                    }
                    if v.is_empty() {
                        return Err("--t-plus-n 至少给一个值".to_string());
                    }
                    out.t_plus_n = v;
                },
                "--period" => {
                    let p = it.next().ok_or("--period 需要一个值")?;
                    const VALID: [&str; 4] = ["short", "mid", "long", "ultrashort"];
                    if !VALID.contains(&p.as_str()) {
                        return Err(format!("--period 只接受 {VALID:?}，收到 `{p}`"));
                    }
                    out.period = Some(p);
                },
                "--max-picks" => {
                    let raw = it.next().ok_or("--max-picks 需要一个数字")?;
                    let n: u32 =
                        raw.parse().map_err(|e| format!("--max-picks 解析失败 `{raw}`: {e}"))?;
                    // 实现侧硬上限是 .min(2000)，这里提前拦，避免"传了 5000 却只跑 2000"的静默截断
                    if n == 0 || n > 2000 {
                        return Err(format!("--max-picks 必须在 1..=2000，收到 {n}"));
                    }
                    out.max_picks = n;
                },
                "--fill-factors" => out.fill_factors = true,
                "--backfill-history" => out.backfill_history = true,
                "--fill-replay-outcomes" => out.fill_replay_outcomes = true,
                "--include-replay" => out.include_replay = true,
                "--target-days" => {
                    let raw = it.next().ok_or("--target-days 需要一个数字")?;
                    let n: u32 =
                        raw.parse().map_err(|e| format!("--target-days 解析失败 `{raw}`: {e}"))?;
                    // 上限 500：接口单次 get_klines 的实际返回上限，再大也拿不到更多
                    if !(30..=500).contains(&n) {
                        return Err(format!("--target-days 必须在 30..=500，收到 {n}"));
                    }
                    out.target_days = n;
                },
                "--limit" => {
                    let raw = it.next().ok_or("--limit 需要一个数字")?;
                    let n: usize =
                        raw.parse().map_err(|e| format!("--limit 解析失败 `{raw}`: {e}"))?;
                    if n == 0 || n > 6000 {
                        return Err(format!("--limit 必须在 1..=6000，收到 {n}"));
                    }
                    out.limit = n;
                },
                "--time-travel" => out.time_travel = true,
                "--tt-analyze" => out.tt_analyze = true,
                "--backfill-performance" => out.backfill_performance = true,
                "--run-reflection" => out.run_reflection = true,
                "--reflect-max" => {
                    let raw = it.next().ok_or("--reflect-max 需要一个值（如 20）")?;
                    let n: u32 = raw
                        .trim()
                        .parse()
                        .map_err(|e| format!("--reflect-max 解析失败 `{raw}`: {e}"))?;
                    if n == 0 {
                        return Err("--reflect-max 必须大于 0".to_string());
                    }
                    out.reflect_max = n;
                },
                "--as-of" => {
                    let raw = it.next().ok_or("--as-of 需要一个值（如 2026-10-16 或 a,b）")?;
                    let v: Vec<String> = raw
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if v.is_empty() {
                        return Err("--as-of 至少要给一个日期".to_string());
                    }
                    out.as_of = Some(v);
                },
                "--tt-min-confidence" => {
                    let raw = it.next().ok_or("--tt-min-confidence 需要一个数字")?;
                    let n: u8 = raw
                        .parse()
                        .map_err(|e| format!("--tt-min-confidence 解析失败 `{raw}`: {e}"))?;
                    out.tt_min_confidence = n;
                },
                "--tt-top-n" => {
                    let raw = it.next().ok_or("--tt-top-n 需要一个数字")?;
                    let n: usize =
                        raw.parse().map_err(|e| format!("--tt-top-n 解析失败 `{raw}`: {e}"))?;
                    out.tt_top_n = n;
                },
                "--tt-timeout" => {
                    let raw = it.next().ok_or("--tt-timeout 需要一个秒数")?;
                    let n: u64 =
                        raw.parse().map_err(|e| format!("--tt-timeout 解析失败 `{raw}`: {e}"))?;
                    if n == 0 {
                        return Err("--tt-timeout 必须为正数".to_string());
                    }
                    out.tt_timeout = n;
                },
                "--apply" => out.apply = true,
                "-h" | "--help" => {
                    println!("{HELP}");
                    std::process::exit(0);
                },
                other => return Err(format!("未知参数: {other}（用 --help 看用法）")),
            }
        }
        Ok(out)
    }
}

async fn run() -> Result<(), String> {
    let args = Args::parse()?;

    // ── 初始化：与 `axagent-batch-rerun` / `server_main.rs` 同路径，无 Tauri 依赖 ──
    let app_dir = axagent_home();
    std::fs::create_dir_all(&app_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let db_result =
        init_database_with_dir(app_dir).await.map_err(|e| format!("数据库初始化失败: {e}"))?;
    let state: Arc<AppState> = Arc::new(create_app_state(db_result).await?);

    // ── 工作流工具 resolver 注入（R-3d，2026-10-26 补）──
    //
    // ⚠ **本 bin 此前漏了这一步**（`axagent-batch-rerun` 早就补上，见其
    // `axagent-batch-rerun.rs:784` 的注释；`init/services.rs:2148` 的应用路径也有）。
    // 漏了的后果（本轮实测，2026-10-26）：
    //   `WorkEngine::run_workflow` 的「按需延迟注册」段（`engine/mod.rs:2478`）
    //   依赖 `tool_resolver` 才能把工作流 `tool_defs` 里声明的工具注册进 Engine；
    //   resolver 为空 ⇒ `工具 'get_industry_ranking' 未注册`（`tool_executor.rs:177`）
    //   ⇒ **所有数据节点 Failed** ⇒ 种子池空 ⇒ **候选 0 行**。
    //
    //   症状极具误导性：批次状态是 `completed`、`荐股 0 行`、
    //   「荐股与趋势智选均无候选（可能当日数据缺失或阈值过高）」——
    //   **长得像「那天没数据」**，实际是**装配缺一步**。
    //
    // 与 R-3c 的宿主函数缺失**同族**（CLI 路径缺 wiring）⇒ 同一处纪律：
    // 「CLI 路径要用的装配，必须在 CLI 路径装齐」。
    //
    // ⚠ 该注入**已下沉到 `create_app_state`**（2026-10-26）⇒ 此处调用是**幂等兜底**
    // （`set_tool_resolver` 已有防呆：已有 resolver 时新注入被忽略）。
    // 保留双点是为「将来 `create_app_state` 的调用顺序被改动」时不至于又缺一步。
    axagent_lib::init::services::inject_work_engine_tool_resolver(&state).await;

    // ── 库身份断言：把「静默降级」变成硬失败 ──
    //
    // `init_database_with_dir` 在 Postgres 不可达时会**默认降级到本地 SQLite**。
    // 对本 bin 而言那是最坏的形态：空库里匹配 0 条 pick，输出与「没有可验证样本」
    // 一模一样 —— 一个不会被察觉的假阴性。（同族纪律：D 组 fail-open / 降级吞 Err。）
    //
    // ⚠ 只打印 scheme 前缀，**绝不打印 URL 全文** —— PG 连接串里带口令。
    let db_ident = state.harness.db_path();
    if !db_ident.starts_with("postgres://") && !db_ident.starts_with("postgresql://") {
        let scheme = db_ident.split("://").next().unwrap_or("<无法识别>");
        return Err(format!(
            "连到的是 `{scheme}`，不是 Postgres ⇒ 极可能是 Postgres 不可达、\
             已静默降级到本地 SQLite。\n\
             此时「0 条候选」不代表真的没有，而是空库。\n\
             处置：① 确认 Postgres 可达（`node scripts/pg-connect.mjs sql \"select 1\"`）；\n\
                ② 或用 AXAGENT_DB_FALLBACK_TO_SQLITE=0 重跑，使降级变成初始化硬报错。"
        ));
    }

    // ── 模式分派：补因子快照（B-3/F-2）与回测补算是两条独立路径 ──
    if args.fill_factors {
        return fill_factors_mode(state.harness.db(), args.apply).await;
    }
    // ── 模式分派：回填成熟反思样本进 strategy_performance（R-3r）──
    if args.backfill_performance {
        return backfill_performance_mode(state.harness.db(), args.apply).await;
    }
    // ── 模式分派：只跑批量反思（不重跑分析）（R-3r）──
    if args.run_reflection {
        return run_reflection_mode(&state, args.reflect_max).await;
    }
    // ── 模式分派：给回放分析补 outcome（R-1）──
    if args.fill_replay_outcomes {
        return fill_replay_outcomes_mode(state.harness.db(), args.apply).await;
    }
    // ── 模式分派：补齐全市场收盘历史（解除 90 天密度上限）──
    if args.backfill_history {
        return backfill_history_mode(
            state.harness.db(),
            &state.astock_client,
            args.target_days,
            args.limit,
            args.apply,
        )
        .await;
    }

    // ── 模式分派：时间旅行回放批次（R-2）──
    if args.time_travel {
        return time_travel_mode(
            &state,
            args.as_of,
            args.tt_min_confidence,
            args.tt_top_n,
            args.tt_timeout,
            args.tt_analyze,
        )
        .await;
    }

    println!("=== axagent-validate-backfill ===");
    println!(
        "模式      : {}",
        if args.apply {
            "APPLY（写库）"
        } else {
            "DRY-RUN（不写库）"
        }
    );
    println!("t_plus_n  : {:?}", args.t_plus_n);
    println!("period    : {}", args.period.as_deref().unwrap_or("<全部>"));
    println!("max_picks : {}", args.max_picks);
    println!("exclude_synthetic: true（兜底合成不是真实决策）");
    println!();

    let request = RunDecisionBacktestRequest {
        t_plus_n_list: Some(args.t_plus_n),
        exclude_synthetic: Some(true),
        max_picks: Some(args.max_picks),
        dry_run: Some(!args.apply),
        period_filter: args.period,
        include_replay: Some(args.include_replay),
    };

    let resp = run_decision_backtest_inner(state.harness.db(), &state.astock_client, request)
        .await
        .map_err(|e| format!("回测执行失败: {e}"))?;

    // ── 输出：先给「样本量」与「跳过量」，再给命中率 ──
    //
    // 顺序有讲究：`total` 明显小于预期时，先看它才能发现「去重键过严」这类静默缩样，
    // 而不是先被一个漂亮的命中率引开。
    println!("--- 结果 ---");
    println!("report.total   : {}", resp.report.total);
    println!("written_count  : {}", resp.written_count);
    println!("skipped_count  : {}", resp.skipped_count);
    println!("data_source    : {}", resp.data_source);
    println!("generation_floor: {}", resp.generation_floor);
    println!();

    println!("--- by_action ---");
    for (k, s) in &resp.report.by_action {
        println!(
            "{k:<6} n={:<4} hit={:<3} partial={:<3} miss={:<3} false_hit={:<2} insuff={:<3} \
             hit_rate={:.1}% strict={:.1}% net_win={:.1}% gross_avg={:.2}% net_avg={:.2}%",
            s.total,
            s.hit,
            s.partial,
            s.miss,
            s.false_hit,
            s.insufficient,
            s.hit_rate * 100.0,
            s.strict_win_rate * 100.0,
            s.net_win_rate * 100.0,
            s.avg_return_pct,
            s.net_avg_return_pct,
        );
    }
    println!();

    println!("--- by_style ---");
    for (k, s) in &resp.report.by_style {
        println!(
            "{k:<10} n={:<4} strict={:.1}% net_win={:.1}% net_avg={:.2}% (denom net={})",
            s.total,
            s.strict_win_rate * 100.0,
            s.net_win_rate * 100.0,
            s.net_avg_return_pct,
            s.net_win_denom,
        );
    }

    if !args.apply {
        println!();
        println!("（DRY-RUN：以上结果未写库。确认样本量合理后加 --apply 再跑一次。）");
    }
    Ok(())
}

/// 给已有判定行补 `factor_snapshot`（B-3 / F-2）。
///
/// ## 为什么单独成一个模式，而不是塞进回测路径
///
/// 回测路径 `fetch_and_validate` 用 **API K 线**（默认前复权），而因子要能与
/// SQL 侧的横截面分析**逐值比对**。两套价格源若不一致（复权方式不同），
/// `dist_lo20` / `rev20` 会系统性偏差 —— 那种偏差不会报错，只会让 IC 归因悄悄失真。
///
/// ⇒ 本模式统一用 `market_daily_close`（与 SQL 分析同源），保证口径可对。
///
/// ## 前视偏差
///
/// 序列严格过滤 `trade_date <= 决策日` 后才交给 `compute_factors`。
/// **截断在这里做，不在因子模块里做** —— 模块文档已声明该分工。
///
/// ## 幂等
///
/// 只处理 `factor_snapshot IS NULL` 的行；已算过的不会被覆盖，
/// 因此可以反复跑（例如后来补进了新的判定行）。
/// 只跑批量反思（**不重跑任何分析**）—— R-3r 的样本供给侧。
///
/// ## 为什么必须有这个入口
///
/// 校准样本的分母是 `strategy_performance`，而物化是**反思执行的内联副产物**
/// （`#31` 四档通道在 `perform_single_reflection` 内部调用）⇒
/// **反思不产出 ⇒ 校准样本不增长** ⇒ 逐档样本量永远到不了
/// `CALIBRATION_MIN_SAMPLES` ⇒ `calibrationReady` 恒假 ⇒ 置信封顶 ⇒
/// 动作联动降级 ⇒ 顶层「买入」为 0（R-3h 死循环的最后一环）。
///
/// ## 为什么不用 `axagent-batch-rerun --reflect-after`
///
/// 那个入口**先重跑分析再反思**：重跑会**原地覆盖**原分析行、**重置**反思 pending 行
/// （清结论列）⇒ 副作用大且会改变代际。本入口只消费**存量 pending 行**。
///
/// ## 终止判据
///
/// 用**库内 pending 计数**（不是解析返回 JSON 的字段名）—— 后者一旦字段改名就会
/// `unwrap_or(0)` 静默变 0 而导致「第一轮就假装跑完」（判据自身撒谎那一类）。
async fn run_reflection_mode(state: &AppState, per_round: u32) -> Result<(), String> {
    use axagent_entities::stock_reflections;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = state.harness.db();
    let master_key = state.harness.master_key_owned();

    println!("=== axagent-validate-backfill / --run-reflection ===");
    println!("模式  : 只跑批量反思（不重跑分析、不动已有分析行）");
    println!("单轮  : {per_round} 条（--reflect-max 可调）");
    println!();

    let count_pending = || async {
        use sea_orm::PaginatorTrait;
        stock_reflections::Entity::find()
            .filter(stock_reflections::Column::Status.eq("pending"))
            .count(db)
            .await
            .unwrap_or(0)
    };

    // 反思**必须**在 live 模式（as-of = None）下跑 —— 它要看的是分析日之后的真实行情，
    // 被 as-of 截断就什么都看不到（2026-09-23 实测：残留 as-of ⇒ 反思全降级「无行情」）。
    println!(
        "[run-reflection] 生效的 as-of = {:?}（期望 None ⇒ live）",
        axagent_astock_data::as_of::current_as_of().map(|c| c.as_string())
    );

    let pending_before = count_pending().await;
    println!("[run-reflection] 当前 pending = {pending_before}");
    if pending_before == 0 {
        println!("无 pending 行，跳过。");
        return Ok(());
    }

    const MAX_ROUNDS: u32 = 20;
    for round in 1..=MAX_ROUNDS {
        let before = count_pending().await;
        if before == 0 {
            println!("[run-reflection] pending 已清空，停止。");
            break;
        }
        let r = axagent_lib::run_batch_reflection_inner(
            db,
            state.astock_client.as_ref(),
            &state.work_engine,
            state.vector_store.as_ref(),
            &master_key,
            Some(per_round),
            Some(&state.trajectory_storage),
            // ⚠ **必须带 `due_only = true`**（与 `reflection-due-daily` cron 同口径）：
            // `require_mature` 由 `filter.due_only` 决定（`reflection.rs:2908`），
            // 而成熟门的作用是「快照没到持有期终点 ⇒ 保持 pending，不写终评」。
            // 绕过它会让**未成熟的行也去调 LLM**；LLM 无 verdict 时状态被写成
            // `failed: REFLECTION_OUTPUT_INCOMPLETE` ⇒ 该行**永久失去被再次处理的机会**
            //（2026-10-26 实测：误标 15 行，恢复后 pending 回到 91）。
            //
            // `due_only` 同时把「hindsight_date 未到」的行排除在本轮之外（到期筛选下推到 SQL）。
            Some(&axagent_lib::ReflectionFilter {
                period: None,
                due_only: true,
                min_confidence: None,
                depth_override: None,
            }),
            None,
        )
        .await;
        match r {
            Ok(v) => println!("[run-reflection] 第 {round} 轮: {v}"),
            Err(e) => println!("[run-reflection] 第 {round} 轮失败: {e}"),
        }
        let after = count_pending().await;
        if after >= before {
            println!("[run-reflection] pending 未减少（{before} → {after}）⇒ 本轮不可推进，停止。");
            break;
        }
    }

    let pending_after = count_pending().await;
    println!();
    println!("[run-reflection] 结束：pending {pending_before} → {pending_after}");
    Ok(())
}

async fn backfill_performance_mode(
    db: &sea_orm::DatabaseConnection,
    apply: bool,
) -> Result<(), String> {
    use sea_orm::{ConnectionTrait, Statement};

    let backend = db.get_database_backend();

    println!("=== axagent-validate-backfill / --backfill-performance ===");
    println!(
        "模式  : {}",
        if apply {
            "APPLY（写库）"
        } else {
            "DRY-RUN（不写库）"
        }
    );
    println!("目标  : 已有四周期 JSON 但从未物化进 strategy_performance 的反思行");
    println!(
        "口径  : 只补「已成熟且可判定」的档（status ∈ {{mature, legacy}} 且 wasCorrect ∈ {{0,1}}）"
    );
    println!("幂等  : 物化键 = reflection-horizon:{{决策 id}}:{{档}} ⇒ 可重复跑");
    println!();

    // ── 1. 待回填目标（干跑也要先看清要动什么）──
    let rows = db
        .query_all_raw(Statement::from_string(
            backend,
            r#"SELECT r.id AS reflection_id,
                      r.original_analysis_id AS analysis_id,
                      r.stock_code,
                      r.horizon,
                      coalesce(a.template_version, -1) AS tv,
                      length(r.horizon_results_json) AS hz_len
               FROM stock_reflections r
               LEFT JOIN stock_analyses a ON a.id = r.original_analysis_id
               WHERE r.horizon_results_json IS NOT NULL
                 AND r.horizon_results_json <> ''
                 AND NOT EXISTS (
                   SELECT 1 FROM strategy_performance sp
                   WHERE sp.source_ref = 'reflection-horizon:' || r.original_analysis_id || ':' || coalesce(r.horizon, '')
                 )
               ORDER BY r.created_at"#
                .to_string(),
        ))
        .await
        .map_err(|e| format!("读取待回填反思失败: {e}"))?;

    println!("待回填反思行: {}", rows.len());
    if !apply {
        for r in rows.iter().take(20) {
            let rid: String = r
                .try_get("", "reflection_id")
                .map_err(|e| format!("取 reflection_id 失败: {e}"))?;
            let code: String =
                r.try_get("", "stock_code").map_err(|e| format!("取 stock_code 失败: {e}"))?;
            let hz: Option<String> =
                r.try_get("", "horizon").map_err(|e| format!("取 horizon 失败: {e}"))?;
            let tv: i32 = r.try_get("", "tv").map_err(|e| format!("取 tv 失败: {e}"))?;
            println!("  {rid}  {code}  档={hz:?}  代际={tv}");
        }
        if rows.len() > 20 {
            println!("  …（其余 {} 行省略）", rows.len() - 20);
        }
        println!();
        println!("DRY-RUN 结束：加 --apply 执行回填。");
        return Ok(());
    }

    // ── 2. 执行回填（幂等）──
    let (scanned, materialized) =
        axagent_analysis_engine::evolution_drift::backfill_reflection_horizon_performance(db)
            .await?;
    println!("扫描反思行 {scanned} 条，物化档位样本 {materialized} 条");

    // ── 3. 验收：逐档样本量（校准门槛 = 30）──
    let rows2 = db
        .query_all_raw(Statement::from_string(
            backend,
            r#"SELECT count(*) AS total,
                      count(*) FILTER (WHERE source_ref LIKE 'reflection-horizon:%') AS ch31
               FROM strategy_performance"#
                .to_string(),
        ))
        .await
        .map_err(|e| format!("统计 strategy_performance 失败: {e}"))?;
    for r in &rows2 {
        let total: i64 = r.try_get("", "total").unwrap_or(0);
        let ch31: i64 = r.try_get("", "ch31").unwrap_or(0);
        println!("strategy_performance 总行 {total}（其中 #31 四档行 {ch31}）");
    }

    Ok(())
}

async fn fill_factors_mode(db: &sea_orm::DatabaseConnection, apply: bool) -> Result<(), String> {
    use axagent_analysis_engine::factors::{FACTOR_VERSION, FactorBar, compute_factors};
    use sea_orm::{ConnectionTrait, Statement, Value};
    use std::collections::BTreeMap;

    let backend = db.get_database_backend();

    println!("=== axagent-validate-backfill / --fill-factors ===");
    println!(
        "模式  : {}",
        if apply {
            "APPLY（写库）"
        } else {
            "DRY-RUN（不写库）"
        }
    );
    println!("因子  : chg1 / dist_lo20 / rev20 / posshare20（factor_version={FACTOR_VERSION:.1}）");
    println!("价格源: market_daily_close（与 SQL 横截面分析同源，排除复权口径漂移）");
    println!("截断  : 严格取 trade_date <= left(generated_at,10) 的 bar（防前视偏差）");
    println!();

    // ── 1. 待补目标：判定成立、且快照为空 ──
    let rows = db
        .query_all_raw(Statement::from_string(
            backend,
            r#"SELECT stock_code, left(generated_at, 10) AS decision_date
               FROM decision_validations
               WHERE factor_snapshot IS NULL
                 AND hit_outcome IN ('hit','miss','partial','false_hit')
               GROUP BY stock_code, left(generated_at, 10)
               ORDER BY stock_code, decision_date"#
                .to_string(),
        ))
        .await
        .map_err(|e| format!("读取待补目标失败: {e}"))?;

    let mut by_stock: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in &rows {
        let code: String =
            r.try_get("", "stock_code").map_err(|e| format!("取 stock_code 失败: {e}"))?;
        let d: String =
            r.try_get("", "decision_date").map_err(|e| format!("取 decision_date 失败: {e}"))?;
        by_stock.entry(code).or_default().push(d);
    }
    let target_pairs: usize = by_stock.values().map(|v| v.len()).sum();
    println!("待补 (股票×决策日) : {target_pairs}（涉及 {} 只股票）", by_stock.len());
    if target_pairs == 0 {
        println!("\n没有需要补的行（已全部有快照）。");
        return Ok(());
    }
    println!();

    // ── 2. 逐股票取一次历史，再按决策日截断逐日计算 ──
    let mut scanned = 0usize;
    let mut computed = 0usize;
    let mut updated = 0u64;
    let mut skipped_insufficient = 0usize;
    let mut skipped_no_history = 0usize;
    let mut samples: Vec<String> = Vec::new();

    for (code, dates) in &by_stock {
        let hist_rows = db
            .query_all_raw(Statement::from_sql_and_values(
                backend,
                "SELECT trade_date, close, change_pct FROM market_daily_close \
                 WHERE stock_code = $1 ORDER BY trade_date",
                vec![Value::from(code.as_str())],
            ))
            .await
            .map_err(|e| format!("取 {code} 行情失败: {e}"))?;

        let mut hist: Vec<(String, FactorBar)> = Vec::with_capacity(hist_rows.len());
        for h in &hist_rows {
            let d: String =
                h.try_get("", "trade_date").map_err(|e| format!("取 trade_date 失败: {e}"))?;
            let close: f64 = h.try_get("", "close").map_err(|e| format!("取 close 失败: {e}"))?;
            let change_pct: f64 =
                h.try_get("", "change_pct").map_err(|e| format!("取 change_pct 失败: {e}"))?;
            hist.push((d, FactorBar { close, change_pct }));
        }
        if hist.is_empty() {
            skipped_no_history += dates.len();
            continue;
        }

        for dd in dates {
            scanned += 1;
            // 前视偏差防线：只保留决策日（含）之前的 bar。日期是 ISO 文本，字典序即时间序。
            let bars: Vec<FactorBar> =
                hist.iter().filter(|(d, _)| d.as_str() <= dd.as_str()).map(|(_, b)| *b).collect();

            let Some(f) = compute_factors(&bars) else {
                skipped_insufficient += 1;
                continue;
            };
            // 与 `updated` 分开计数：dry-run 不走 UPDATE，若共用 `updated` 会打出
            // 「可写入 0 条」的假零（实测踩过：386 条全部可算却被读成 0）。
            computed += 1;
            if samples.len() < 5 {
                samples.push(format!(
                    "{code} {dd} bars={:<3} chg1={:>7.2} dist_lo20={:>7.2} rev20={:>8.2} posshare20={:>5.0}",
                    bars.len(),
                    f.chg1,
                    f.dist_lo20,
                    f.rev20,
                    f.posshare20
                ));
            }
            if !apply {
                continue;
            }
            let json = serde_json::to_string(&f.to_snapshot())
                .map_err(|e| format!("序列化因子失败: {e}"))?;
            let res = db
                .execute_raw(Statement::from_sql_and_values(
                    backend,
                    r#"UPDATE decision_validations SET factor_snapshot = $1
                       WHERE stock_code = $2 AND left(generated_at, 10) = $3
                         AND factor_snapshot IS NULL
                         AND hit_outcome IN ('hit','miss','partial','false_hit')"#,
                    vec![Value::from(json), Value::from(code.as_str()), Value::from(dd.as_str())],
                ))
                .await
                .map_err(|e| format!("写 {code} {dd} 因子失败: {e}"))?;
            updated += res.rows_affected();
        }
    }

    // ── 3. 结果：先给「跳过」再看「写入」──
    //
    // 顺序有讲究：跳过量异常时先看它，才能发现「行情缺段 / 历史不足」这类静默缩样，
    // 而不是先被「写入成功」引开。
    println!("--- 计算样本（前 5 条）---");
    for s in &samples {
        println!("  {s}");
    }
    println!();
    println!("--- 结果 ---");
    println!("扫描 (股票×决策日) : {scanned}");
    println!(
        "跳过·无行情      : {skipped_no_history}（该股票在 market_daily_close 里没有任何记录）"
    );
    println!("跳过·bar 不足     : {skipped_insufficient}（截断后不足 21 根，不产假值）");
    println!("可算 (算完的)     : {computed}");
    // 自洽校验：扫描数必须等于「可算 + 两类跳过」。不等说明有分支既没计数也没跳过
    // （漏计数会打出漂亮的假绿，这条是唯一的对账口）。
    let expected = computed + skipped_insufficient + skipped_no_history;
    println!(
        "对账              : scanned={scanned} vs 可算+跳过={expected} ⇒ {}",
        if expected == scanned {
            "一致"
        } else {
            "❌ 不一致，检查分支漏计数"
        }
    );
    if apply {
        println!("已写入行数        : {updated}");
    } else {
        println!("可写入行数（DRY） : {computed}");
    }
    println!();

    if !apply {
        println!("（DRY-RUN：未写库。确认样本合理后加 --apply 再跑一次。）");
    } else {
        println!("提示：因子已落库。分位（quintile）需在全市场横截面上另算，不在本步。");
        println!("      与 SQL 重算的逐值比对见 docs/audits/AUDIT-factor-evidence-2026-10-25.md。");
    }
    Ok(())
}

/// 给回放分析补 `outcome` 判定（R-1）。
///
/// ## 为什么回放的 outcome 全是空的
///
/// 生产侧唯一的回写路径是 `sync_outcomes_to_stock_analyses`（`backtest_validation.rs`），
/// 它的匹配键是 **`stock_analyses.analysis_date` == `reco_picks.generated_at` 的日期部分**。
/// 而回放行里：`analysis_date` = as_of（历史日），`generated_at` = **跑批的墙钟时间**
/// ⇒ 两边永远不相等，一条也匹配不上。
/// 叠加 `run_decision_backtest` 现在显式只吃 `as_of_date IS NULL`（实盘代），
/// 回放样本**根本没有进入验证链**（实测 125 条分析仅 18 条有 outcome，86% 空）。
///
/// ## 判定口径（复用单一来源，不另造）
///
/// - 收益：`market_daily_close` 收盘价，`(exit/entry − 1) × 100 − 成本`
/// - 成本：`costs::DEFAULT_ROUND_TRIP_COST_PCT`（0.25%，与 G4 同源）
/// - 判定：`compute_hit_outcome(action, net, None, None)` → `hit_outcome_to_binary_outcome`
///   两者都是 `hit_rate_backtest` 的公开单点，**win/loss 规则不在这里重写**
///
/// ⇒ 与实盘链**同一把尺子**，读数才有资格互相比。
///
/// ## 不猜的部分（宁可跳过也不产假值）
///
/// - `decision_action='数据缺失'` → 跳过并计数（那本来就不是一次判定）
/// - 持有期取 `decision_expected_holding_days`；缺失时按 `decision_time_horizon` 显式映射，
///   其中 `短期` → 5（历史遗留中文值，与 `short` 同义；登记为本轮的 D-2 族）
/// - 前向 bar 不足（未成熟）→ 跳过并计数，**绝不把短窗口当长窗口**
async fn fill_replay_outcomes_mode(
    db: &sea_orm::DatabaseConnection,
    apply: bool,
) -> Result<(), String> {
    use axagent_analysis_engine::costs::DEFAULT_ROUND_TRIP_COST_PCT;
    use axagent_analysis_engine::hit_rate_backtest::{
        compute_hit_outcome, hit_outcome_to_binary_outcome,
    };
    use sea_orm::{ConnectionTrait, Statement, Value};
    use std::collections::BTreeMap;

    let backend = db.get_database_backend();

    println!("=== axagent-validate-backfill / --fill-replay-outcomes ===");
    println!(
        "模式 : {}",
        if apply {
            "APPLY（写库）"
        } else {
            "DRY-RUN（不写库）"
        }
    );
    println!("成本 : {DEFAULT_ROUND_TRIP_COST_PCT:.2}%（costs 模块单一来源）");
    println!("判定 : compute_hit_outcome + hit_outcome_to_binary_outcome（与实盘链同一把尺子）");
    println!(
        "收益 : market_daily_close 收盘价，as_of 起算，持有 decision_expected_holding_days 个交易日"
    );
    println!();

    let rows = db
        .query_all_raw(Statement::from_string(
            backend,
            r#"SELECT id, stock_code, as_of_date, decision_action,
                      decision_time_horizon, decision_expected_holding_days
               FROM stock_analyses
               WHERE analysis_kind = 'replay'
                 AND (outcome IS NULL OR outcome = 'pending')
                 AND as_of_date IS NOT NULL
                 AND decision_action IS NOT NULL
               ORDER BY stock_code, as_of_date"#
                .to_string(),
        ))
        .await
        .map_err(|e| format!("读取待判回放分析失败: {e}"))?;

    // (stock_code) -> Vec<(id, as_of, action, horizon, hold_days)>
    type Row = (String, String, String, Option<String>, Option<i64>);
    let mut by_stock: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for r in &rows {
        let id: String = r.try_get("", "id").map_err(|e| format!("取 id 失败: {e}"))?;
        let code: String =
            r.try_get("", "stock_code").map_err(|e| format!("取 stock_code 失败: {e}"))?;
        let as_of: String =
            r.try_get("", "as_of_date").map_err(|e| format!("取 as_of_date 失败: {e}"))?;
        let action: String = r
            .try_get("", "decision_action")
            .map_err(|e| format!("取 decision_action 失败: {e}"))?;
        let horizon: Option<String> = r.try_get("", "decision_time_horizon").ok();
        let hold_days: Option<i64> = r.try_get("", "decision_expected_holding_days").ok();
        by_stock.entry(code).or_default().push((id, as_of, action, horizon, hold_days));
    }
    let target = by_stock.values().map(|v| v.len()).sum::<usize>();
    println!("待判回放分析 : {target} 条（涉及 {} 只股票）", by_stock.len());
    if target == 0 {
        println!("\n没有需要补的回放 analysis。");
        return Ok(());
    }
    println!();

    let mut scanned = 0usize;
    let mut judged = 0usize;
    let mut wrote = 0u64;
    let mut win = 0usize;
    let mut loss = 0usize;
    let mut skip_no_data = 0usize;
    let mut skip_no_action = 0usize;
    let mut skip_no_horizon = 0usize;
    let mut skip_immature = 0usize;
    let mut skip_no_history = 0usize;
    let mut samples: Vec<String> = Vec::new();

    for (code, list) in &by_stock {
        let hist_rows = db
            .query_all_raw(Statement::from_sql_and_values(
                backend,
                "SELECT trade_date, close FROM market_daily_close \
                 WHERE stock_code = $1 ORDER BY trade_date",
                vec![Value::from(code.as_str())],
            ))
            .await
            .map_err(|e| format!("取 {code} 行情失败: {e}"))?;
        let mut hist: Vec<(String, f64)> = Vec::with_capacity(hist_rows.len());
        for h in &hist_rows {
            let d: String =
                h.try_get("", "trade_date").map_err(|e| format!("取 trade_date 失败: {e}"))?;
            let c: f64 = h.try_get("", "close").map_err(|e| format!("取 close 失败: {e}"))?;
            hist.push((d, c));
        }
        if hist.is_empty() {
            skip_no_history += list.len();
            continue;
        }

        for (id, as_of, action, horizon, hold_days) in list {
            scanned += 1;

            // ① 动作 → 引擎口径；数据缺失不是判定
            let action_eng = match action.as_str() {
                "数据缺失" => {
                    skip_no_data += 1;
                    continue;
                },
                "买入" => "buy",
                "卖出" | "减持" => "sell",
                "持有" | "观望" => "hold",
                _ => {
                    skip_no_action += 1;
                    continue;
                },
            };

            // ② 持有期：优先显式天数，否则按档位映射（不猜未登记的值）
            let n: i64 = match hold_days.filter(|v| *v > 0) {
                Some(v) => v,
                None => match horizon.as_deref() {
                    Some("ultra_short") => 2,
                    Some("short") | Some("短期") => 5,
                    Some("mid") => 28,
                    Some("long") => 90,
                    _ => {
                        skip_no_horizon += 1;
                        continue;
                    },
                },
            };

            // ③ 定位入场 bar：as_of（含）之前最后一根
            let Some(idx) = hist.iter().rposition(|(d, _)| d.as_str() <= as_of.as_str()) else {
                skip_no_history += 1;
                continue;
            };
            let exit_idx = idx + n as usize;
            if exit_idx >= hist.len() || n <= 0 {
                // 前向 bar 不足 ⇒ 该档未成熟。绝不用短窗口冒充长窗口。
                skip_immature += 1;
                continue;
            }
            let entry = hist[idx].1;
            let exit = hist[exit_idx].1;
            // NaN / INF 一并排除（`!(x > 0.0)` 的原写法对 NaN 成立，对 +INF 不成立 ⇒ 改用显式判定）
            if !entry.is_finite() || entry <= 0.0 || !exit.is_finite() || exit <= 0.0 {
                skip_no_history += 1;
                continue;
            }

            // ④ 净收益（扣成本）→ 同一套判定函数
            let net = (exit / entry - 1.0) * 100.0 - DEFAULT_ROUND_TRIP_COST_PCT;
            let Some(hit) = compute_hit_outcome(action_eng, Some(net), None, None) else {
                skip_immature += 1;
                continue;
            };
            let Some(outcome) = hit_outcome_to_binary_outcome(Some(hit.as_str())) else {
                // insufficient
                skip_immature += 1;
                continue;
            };
            judged += 1;
            if outcome == "win" {
                win += 1;
            } else {
                loss += 1;
            }
            if samples.len() < 6 {
                samples.push(format!(
                    "{code} {as_of} {:<4} T+{:<2} entry={entry:>8.2} exit={exit:>8.2} \
                     net={net:>7.2}% → {hit} → {outcome}",
                    action, n
                ));
            }

            if !apply {
                continue;
            }
            let res = db
                .execute_raw(Statement::from_sql_and_values(
                    backend,
                    r#"UPDATE stock_analyses SET outcome = $1, updated_at = $2
                       WHERE id = $3
                         AND analysis_kind = 'replay'
                         AND (outcome IS NULL OR outcome = 'pending')"#,
                    vec![
                        Value::from(outcome),
                        Value::from(chrono::Utc::now().timestamp_millis()),
                        Value::from(id.as_str()),
                    ],
                ))
                .await
                .map_err(|e| format!("写 {code} {as_of} outcome 失败: {e}"))?;
            wrote += res.rows_affected();
        }
    }

    println!("--- 判定样本（前 6 条）---");
    for s in &samples {
        println!("  {s}");
    }
    println!();
    println!("--- 结果 ---");
    println!("扫描        : {scanned}");
    println!("可判定      : {judged}（win {win} / loss {loss}）");
    println!("跳过·数据缺失: {skip_no_data}");
    println!("跳过·动作无法映射: {skip_no_action}");
    println!("跳过·无持有期: {skip_no_horizon}");
    println!("跳过·未成熟  : {skip_immature}（前向 bar 不足，不用短窗口冒充）");
    println!("跳过·无行情  : {skip_no_history}");
    // 对账：扫描数必须等于「可判定 + 全部跳过」。不等说明有分支漏计数。
    let accounted =
        judged + skip_no_data + skip_no_action + skip_no_horizon + skip_immature + skip_no_history;
    println!(
        "对账        : scanned={scanned} vs 可判定+跳过={accounted} ⇒ {}",
        if accounted == scanned {
            "一致"
        } else {
            "❌ 不一致，检查分支漏计数"
        }
    );
    if apply {
        println!("已写入行数  : {wrote}");
    } else {
        println!("可写入行数（DRY）: {judged}");
    }
    println!();
    if !apply {
        println!("（DRY-RUN：未写库。确认口径后加 --apply 再跑。）");
    } else {
        println!("提示：此口径与实盘链同源（成本 + 判定函数）。可与 live outcome 直接对比。");
    }
    Ok(())
}

/// 补齐全市场收盘历史（解除「全市场只有 90 天」的密度上限）。
///
/// ## 为什么这件事比继续做分析更值得
///
/// `AUDIT-factor-strategy-counterfactual` 的结论停在「方向为正但未证实」，
/// 唯一原因是有效独立窗口只有 13 个。而限制不在算法、不在因子，在**数据**：
/// 全市场密度从 2026-06-11 起才够，**90 个交易日封顶**。
/// 但 `get_klines` 单次就能取 500 根 —— 只是没人补过
/// （UI 命令 `backfill_market_close_history` 把 `TARGET_DAYS` 硬编码成 90）。
///
/// ## 为什么提高目标天数几乎不涨成本
///
/// 回补对每只票只发**一次** `get_klines(code, "daily", target_days + 1)`。
/// 90 → 250 变的是单次请求的 limit，不是请求次数。
/// ⇒ 500 只/批 × 120ms 间隔，约 12 批跑完全市场。
///
/// ## 为什么安全
///
/// - `on_conflict do_nothing`：**只补缺失行**，既有快照行（amount 真值更权威）不被覆盖
/// - 幂等：已够 `target_days` 的票被 `codes_needing_history` 排除，可断点续跑
/// - 不写 `change_pct = 0` 伪值（模块头硬约束：0 会把「拿不到」伪装成「没涨」）
async fn backfill_history_mode(
    db: &sea_orm::DatabaseConnection,
    client: &std::sync::Arc<axagent_astock_data::AStockClient>,
    target_days: u32,
    limit: usize,
    apply: bool,
) -> Result<(), String> {
    use axagent_analysis_engine::market_close_store::{
        backfill_close_history_for_codes, codes_needing_history,
    };

    println!("=== axagent-validate-backfill / --backfill-history ===");
    println!(
        "模式     : {}",
        if apply {
            "APPLY（写库）"
        } else {
            "DRY-RUN（不写库）"
        }
    );
    println!("目标天数 : {target_days} 个交易日");
    println!("本批上限 : {limit} 只");
    println!("限速     : 120ms/票（与 UI 命令一致）");
    println!();

    let entries = codes_needing_history(db, target_days, limit).await?;
    println!("待补票数（本批）: {}", entries.len());
    if entries.is_empty() {
        println!("\n没有需要补的票（已全部达到 {target_days} 天）。");
        return Ok(());
    }
    if !apply {
        println!();
        println!("（DRY-RUN：未写库。确认批大小后加 --apply 再跑。）");
        println!("提示：全市场约 12 批；每批单独执行本命令即可，幂等、可断点续跑。");
        return Ok(());
    }

    let started = std::time::Instant::now();
    let stats = backfill_close_history_for_codes(db, client, &entries, target_days, 120).await?;
    let secs = started.elapsed().as_secs_f64().max(0.001);

    // 顺序有讲究：先看「没补到」的三类分档，再看「补了多少」。
    // `no_data` 是答案（未上市/退市），不是故障；混进 failed 会让人以为通道坏了。
    println!();
    println!("--- 结果 ---");
    println!("处理票数     : {}", stats.codes);
    println!("新增行数     : {}", stats.rows);
    println!("未新增 (no_new): {}（可用日线不足或有行已齐）", stats.no_new);
    println!("无行情 (no_data): {}（未上市/退市/已合并 —— 是答案不是故障）", stats.no_data);
    println!("失败         : {}", stats.failed);
    if let Some(ref e) = stats.first_error {
        println!("首条失败原因 : {e}");
    }
    if stats.cancelled {
        println!("⚠ 被取消：剩余票未取，已写入的行保持有效");
    }
    println!("耗时         : {secs:.1}s（{:.1} 票/秒）", stats.codes as f64 / secs);
    println!();
    println!("提示：本命令只补「行数不足」的票。跑完全部批次后，用下列 SQL 复核密度：");
    println!("  SELECT count(*) FILTER (WHERE c>=5000) AS dense_days");
    println!(
        "  FROM (SELECT trade_date, count(*) c FROM market_daily_close GROUP BY trade_date) t;"
    );
    Ok(())
}

// ── 模式：时间旅行回放批次（R-2，2026-10-25）────────────────────────────
//
// 目标：把「回放日」从当前的 3 个推到 30+ 个。回放日的可判定性有一条硬约束：
// **as_of + N 个交易日 ≤ 今天**，否则 T+N 判不出来（那天的行情还没发生）。
// 因此「今天能回放哪些日」逐日变化，调度必须**每天跑一次**而不是一次跑完。
//
// 本模式做两件事：
// ① 算出**今天**哪些 as_of 日同时满足「建池快照至少一源齐全」与「T+2 已到期」；
// ② 对这些日各发一个时间旅行批次，并轮询到终态后汇报产出。
//
// ⚠ 批次内部是**两段式**：guard 内跑选股与分析（"今天"= as_of），
// 退出 guard 后才做四档反思（此时"今天"恢复 ⇒ 2/5 档当场可判）。
// 28/90 档要等自然成熟，**时间旅行加速不了**（快照历史长度是它的墙）。

/// 轮询批次直到终态。
#[allow(clippy::type_complexity)]
async fn wait_batch_terminal(
    db: &sea_orm::DatabaseConnection,
    batch_id: &str,
    timeout_secs: u64,
) -> Result<(String, i32, i32, i32, i32, Option<String>), String> {
    use axagent_dao::repo::time_travel_batch_runs::fetch_batch_by_id;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut pool_errors = 0u32;
    loop {
        match fetch_batch_by_id(db, batch_id).await {
            Ok(Some(m)) => {
                let terminal =
                    matches!(m.status.as_str(), "completed" | "partial" | "failed" | "interrupted");
                if terminal {
                    return Ok((m.status, m.total, m.ok, m.fail, m.reco_picks, m.message));
                }
            },
            Ok(None) => {},
            Err(e) => {
                // 连接池瞬时饥饿（批次自身的并发取数占满池）**不是**「批次失败」。
                // 误报成失败会把成功的批次记成失败 ⇒ 计数与日志都失真。
                let msg = e.to_string();
                if msg.contains("Connection pool") || msg.contains("pool timed out") {
                    pool_errors += 1;
                    if pool_errors > 20 {
                        return Err(format!("批次 {batch_id} 连续 20 次读库失败（连接池）: {msg}"));
                    }
                } else {
                    return Err(format!("读批次失败: {msg}"));
                }
            },
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("批次 {batch_id} 等待超过 {timeout_secs}s 仍未到终态"));
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// 等全局批次锁空闲（最多 `max_wait` 秒）。
///
/// ## 为什么必须等
///
/// 批次是**后台执行**的：命令只返回 `batch_id`，真正的活儿在另一个 runtime 上跑。
/// 若本 CLI 的等待上限先到（分析阶段逐只跑 LLM 工作流，实测单日 30~60 分钟），
/// 而此时**直接进入下一个 as_of 日**，新批次会撞上 `time_travel_batch_lock`
/// （`try_lock_owned`，不排队）⇒ 被拒 `ALREADY_RUNNING` ⇒ **该日被整个跳过**。
///
/// 而「跳过」是最坏的形态：它看起来像「那天气数不足」，实际是「上一批还没跑完」。
/// ⇒ 进入下一日之前必须确认锁已释放。
async fn wait_lock_free(lock: &Arc<tokio::sync::Mutex<()>>, max_wait: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(max_wait);
    loop {
        if let Ok(permit) = lock.clone().try_lock_owned() {
            drop(permit); // 立刻释放：只是确认「空闲」，不是要占着
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

/// 时间旅行模式：逐个 as_of 日发批次并等待终态。
async fn time_travel_mode(
    state: &AppState,
    explicit_dates: Option<Vec<String>>,
    min_confidence: u8,
    top_n: usize,
    timeout_secs: u64,
    analyze: bool,
) -> Result<(), String> {
    use axagent_astock_data::calendar;
    use axagent_lib::{TimeTravelBatchRequest, run_time_travel_batch_inner};

    let db = state.harness.db();
    let today = calendar::beijing_today();
    let master_key = state.harness.master_key_owned();

    // ── 1. 选定要跑的 as_of 日 ──
    let candidates: Vec<String> = match explicit_dates {
        Some(v) => v,
        None => {
            // 与 cron 分支共用同一份「今天可回放哪些日」判定（不在两处各算一遍）。
            let replayable = axagent_lib::list_replayable_as_of_dates(db, 2).await?;
            replayable.into_iter().map(|(d, _ago)| d).collect()
        },
    };

    if candidates.is_empty() {
        return Err("没有可回放的 as_of 日：建池双源快照与 T+2 到期条件同时满足的日期为空".into());
    }

    println!("=== 时间旅行回放批次（R-2）===");
    println!("今天(北京)    : {today}");
    println!("候选 as_of   : {} 个", candidates.len());
    println!("min_confidence: {min_confidence} ｜ top_n: {top_n} ｜ 单批超时: {timeout_secs}s");
    println!();

    let mut ok_batches = 0usize;
    let mut failed_batches = 0usize;
    let mut total_picks = 0i32;

    for d in &candidates {
        let nd = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map_err(|e| format!("as_of 日期 `{d}` 解析失败: {e}"))?;
        let ago = calendar::trading_days_between(nd, today);
        let request = TimeTravelBatchRequest {
            date: Some(d.clone()),
            trading_days_ago: None,
            periods: Vec::new(),
            min_confidence,
            top_n,
            reflect: true,
            analyze,
        };
        // 发起失败重试：两类瞬时失败都不该让该日被「跳过」。
        // ① `ALREADY_RUNNING` ⇒ 上一批还没跑完 ⇒ 等锁释放；
        // ② 连接池饥饿（`Connection pool timed out`）⇒ 批次自身的并发取数占满了
        //    20 连接的池（`dao/src/db.rs` max_connections=20 / acquire_timeout=15s）
        //    ⇒ 等一会让批次跑完，池自然释放。
        // 「跳过」是最坏形态：它长得像「那日不可回放」，实际是「上一批还没跑完」。
        let mut attempt = 0u32;
        let batch_id = loop {
            match run_time_travel_batch_inner(
                state.harness.db(),
                &state.astock_client,
                &state.work_engine,
                &state.vector_store,
                &master_key,
                &state.trajectory_storage,
                &state.time_travel_batch_lock,
                request.clone(),
            )
            .await
            {
                Ok(id) => break id,
                Err(e) => {
                    let busy = e.contains("ALREADY_RUNNING");
                    let pool_starved =
                        e.contains("Connection pool timed out") || e.contains("Connection pool");
                    if (busy || pool_starved) && attempt < 3 {
                        attempt += 1;
                        if busy {
                            println!("[{d}] 上一批仍在运行，等锁释放后重试（第 {attempt} 次）…");
                            if !wait_lock_free(&state.time_travel_batch_lock, timeout_secs).await {
                                println!("[{d}] 等锁超时，放弃该日");
                                break String::new();
                            }
                        } else {
                            println!(
                                "[{d}] 连接池忙（批次并发取数占满 20 连接），第 {attempt} 次重试…"
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(30 * attempt as u64))
                                .await;
                        }
                        continue;
                    }
                    failed_batches += 1;
                    println!("[{d}] 发起失败: {e}");
                    break String::new();
                },
            }
        };
        if batch_id.is_empty() {
            continue;
        }
        let (status, total, ok, fail, picks, msg) =
            match wait_batch_terminal(db, &batch_id, timeout_secs).await {
                Ok(v) => v,
                Err(e) => {
                    // 等待超时 ⇒ 批次仍在后台跑。**必须等它把锁放掉**，
                    // 否则下一日会被 `ALREADY_RUNNING` 拒掉（该日被跳过）。
                    println!("[{d}] {e}");
                    if !wait_lock_free(&state.time_travel_batch_lock, timeout_secs).await {
                        println!("[{d}] 等锁超时（批次可能仍在跑），后续日期可能受影响");
                    }
                    failed_batches += 1;
                    continue;
                },
            };
        total_picks += picks;
        if status == "completed" || status == "partial" {
            ok_batches += 1;
        } else {
            failed_batches += 1;
        }
        println!(
            "[{d}] {status} ｜ 距今 {ago} 交易日 ｜ 候选 {total} ｜ ok {ok} ｜ fail {fail} \
             ｜ 荐股 {picks} 行{}",
            msg.map(|m| format!(" ｜ {m}")).unwrap_or_default()
        );
    }

    println!();
    println!("--- 汇总 ---");
    println!("成功批次   : {ok_batches}");
    println!("失败批次   : {failed_batches}");
    println!("荐股总行数 : {total_picks}");
    println!();
    println!(
        "下一步：对回放 pick 跑验证（--include-replay），并检查 as_of_date 覆盖是否真的变多："
    );
    println!(
        "  SELECT as_of_date, count(*) FROM reco_picks WHERE as_of_date IS NOT NULL GROUP BY 1;"
    );
    Ok(())
}

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    if let Err(e) = run().await {
        eprintln!("[validate-backfill] 失败: {e}");
        std::process::exit(1);
    }
}
