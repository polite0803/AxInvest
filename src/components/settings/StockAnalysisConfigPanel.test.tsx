import { describe, expect, it } from "vitest";

/**
 * 这些测试锁定 StockAnalysisConfigPanel 默认变量列表必须与
 * `src-tauri/src/commands/stock_analysis_setup.rs` 中 stock-analysis 模板 v19
 * 的 snake_case key 一一对应。如果后端模板升级了变量名，这里也要同步更新。
 *
 * 测试不直接 import 组件里的函数（避免引入 antd 等重依赖），而是通过
 * 静态扫描源文件 + 字符串断言来验证同步关系。
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const PANEL_PATH = resolve(__dirname, "./StockAnalysisConfigPanel.tsx");
const RUST_SETUP_PATH = resolve(
  __dirname,
  "../../../src-tauri/src/commands/stock_analysis_setup/mod.rs",
);
/** 变量表权威源：所有模板变量的默认值/描述都定义在这里 */
const SEED_VARS_PATH = resolve(
  __dirname,
  "../../../src-tauri/src/commands/stock_analysis_setup/seed_variables.rs",
);
/** 工作流节点定义（portfolio-mgr 的 input_mapping 在这里） */
const SEED_WF_PATH = resolve(
  __dirname,
  "../../../src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs",
);
/** portfolio-mgr 的消费点（rhai 顶部的 present() 守卫 = 参数真正被读取的地方） */
const PORTFOLIO_MGR_RHAI_PATH = resolve(
  __dirname,
  "../../../src-tauri/src/commands/portfolio-mgr.rhai",
);

/**
 * 运行时由 Rust 注入的上下文变量 —— 不走模板 variables，故 seed_variables.rs 里没有定义。
 * 生产者举例：`stock_workflow/hooks.rs` 计算 sim_* 后注入；
 * `stock_workflow/core.rs` 注入 stock_code / holdings_json / portfolio_cash 等。
 * 这类「同名映射但非模板变量」是正常设计，不应被判为断链。
 */
const RUNTIME_INJECTED = new Set([
  "actual_outcome",
  "reflection_depth",
  "stock_lessons",
  "sim_stability",
  "sim_liquidity",
  "sim_impact",
  "stock_code",
  "stock_sector",
  "serenity_context",
  "holdings_json",
  "portfolio_cash",
]);

/**
 * ⚠️ 已于 2026-09-11 清空 —— 保留空集合是为了让「是否有人重新打开这个后门」可断言。
 *
 * 原本登记在此的 29 项「面板声明但未接线」参数已完成全项目核实并清理：
 *   · 22 项在全部 Rust / rhai 源码中**零消费点**
 *     （moat_* 4 项对应 `analysis-engine/src/value.rs` 的硬编码
 *     `roe_years_above_15 >= 3`；screener_* 7 项无任何读取点；
 *     rev_* / val_pe_* / val_pb_* / cap_* 8 项、pos_min_cash_pct、
 *     pos_max_turnover_pct、trading_price_deviation_limit 同类）；
 *   ·  6 项 limit_up_* 虽出现在 `crates/tools/src/tools/finance.rs`，但读取来源是
 *     ToolNode args（`tv_f64(args, "limit_up_w_trend", 40.0)`），**不是模板变量**；
 *   ·  1 项 pos_max_turnover_pct 仅出现在 reflection.rs 的注释举例文本里。
 * 确认全部为死配置后，已从面板 `getDefaultVariables()` 与 `resolve()` 分组中移除。
 * 界面行为不变：`resolve(names).filter(Boolean)` 本来就滤掉了这些不存在的变量。
 *
 * **禁止再向此集合添加条目**。新增参数必须补齐三件套，而不是把未接线参数
 * 登记在这里绕过检查：① `seed_variables.rs` 定义变量；② 对应 `input_mapping`
 * 同名映射；③ 消费点从硬编码改为读取该变量。缺任一即为「配置项空接线」。
 */
const KNOWN_UNWIRED_PARAMS = new Set<string>([]);

function readPanelSource(): string {
  return readFileSync(PANEL_PATH, "utf8");
}

function readRustSource(): string {
  return readFileSync(RUST_SETUP_PATH, "utf8");
}

/** 从变量表权威源 seed_variables.rs 抽取所有 `name: "xxx".into(), var_type: ...` 定义 */
function extractVarNamesFromSeedVariables(): Set<string> {
  const src = readFileSync(SEED_VARS_PATH, "utf8");
  const re = /name:\s*"([a-z_][a-z0-9_]*)"\.into\(\),\s*\n\s*var_type:\s*"(number|string|boolean|enum)"/g;
  const names = new Set<string>();
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) { names.add(m[1]); }
  return names;
}

/**
 * 抽取 portfolio-mgr input_mapping 中的「同名映射」`("x", "x")`。
 * 同名映射要求目标变量在变量表里真实存在，否则 rhai 的
 * `if present(x) {...} else { 硬编码默认值 }` 会永远走 else 分支 —— 静默失效。
 */
/**
 * 抽取 portfolio-mgr 已接入的「决策参数」映射目标名。
 *
 * 必须覆盖两种形式，否则会把「已改成单一权威源派生」误判成断链：
 *   ① 字面量同名映射 `("x", "x")` —— 其他节点的历史写法；
 *   ② `m.extend(PORTFOLIO_MGR_TUNABLE_PARAMS...)` —— portfolio-mgr 的决策参数
 *      自 v72 起由常量 `PORTFOLIO_MGR_TUNABLE_PARAMS` 统一派生（该常量同时是
 *      反思 prompt 清单与前端面板分组的权威源），真实映射目标在其数组字面量里。
 */
function extractSameNameMappings(): string[] {
  const src = readFileSync(SEED_WF_PATH, "utf8");
  const names = new Set<string>();
  const re = /\("([a-z_][a-z0-9_]*)",\s*"\1"\)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) { names.add(m[1]); }

  if (/m\.extend\(\s*PORTFOLIO_MGR_TUNABLE_PARAMS/.test(src)) {
    const blockRe = /const PORTFOLIO_MGR_TUNABLE_PARAMS[^=]*=\s*\[([\s\S]*?)\];/;
    const block = blockRe.exec(src);
    if (block) {
      const itemRe = /"([a-z_][a-z0-9_]*)"/g;
      let im: RegExpExecArray | null;
      while ((im = itemRe.exec(block[1])) !== null) { names.add(im[1]); }
    }
  }
  return [...names];
}

/** 抽取 PORTFOLIO_MGR_TUNABLE_PARAMS 常量内容（不依赖 input_mapping 的消费形式） */
function extractPortfolioMgrTunableParams(): string[] {
  const src = readFileSync(SEED_WF_PATH, "utf8");
  const blockRe = /const PORTFOLIO_MGR_TUNABLE_PARAMS[^=]*=\s*\[([\s\S]*?)\];/;
  const block = blockRe.exec(src);
  if (!block) { return []; }
  const names = new Set<string>();
  const itemRe = /"([a-z_][a-z0-9_]*)"/g;
  let im: RegExpExecArray | null;
  while ((im = itemRe.exec(block[1])) !== null) { names.add(im[1]); }
  return [...names];
}

/**
 * 抽取面板 toolGroups 的「分组名 → 该分组实际展示的变量名」映射。
 *
 * 分组结构（StockAnalysisConfigPanel.tsx）：
 *   { tool: "portfolio_mgr_risk", label: t("..."), vars: resolve(["a", "b"]) }
 *
 * ⚠️ 必须按 `resolve([...])` 抽取，不能按 `b("x", ...)` 抽取 —— 前者才是
 * 界面真正渲染的集合（`resolve(names).filter(Boolean)` 会滤掉模板里不存在的名字），
 * 后者只是默认变量声明表，写进声明却没进分组 = 界面上永远看不到。
 */
function extractPanelToolGroups(): Map<string, string[]> {
  const src = readPanelSource();
  const groupRe = /tool:\s*"([A-Za-z_][A-Za-z0-9_]*)",\s*\n\s*label:\s*[^\n]*\n\s*vars:\s*resolve\(\[([\s\S]*?)\]\)/g;
  const out = new Map<string, string[]>();
  let m: RegExpExecArray | null;
  while ((m = groupRe.exec(src)) !== null) {
    const names = new Set<string>();
    const itemRe = /"([a-z_][a-z0-9_]*)"/g;
    let im: RegExpExecArray | null;
    while ((im = itemRe.exec(m[2])) !== null) { names.add(im[1]); }
    out.set(m[1], [...names]);
  }
  return out;
}

/**
 * 抽取 portfolio-mgr.rhai 里 `"effective_params": #{ ... }` 的字段名。
 *
 * 这是决策参数的**第三处派生点**（另两处是 input_mapping 与反思清单）：
 * 权威源定义「有哪些可调参数」，effective_params 输出「它们本次各自生效成什么值」，
 * 反思 / 演进据此归因。字段集合必须与权威源一致，否则观测面残缺 ——
 * 例如 `trader_cap_min_weight` 曾长期缺席：反思能看到 `f7_weight=0.1`，
 * 却看不到判定它是否达标的门槛，无法判断「f7 被压制」还是「门槛太高」。
 */
function extractEffectiveParamsFields(): string[] {
  const src = readFileSync(PORTFOLIO_MGR_RHAI_PATH, "utf8");
  const anchor = src.indexOf('"effective_params"');
  if (anchor < 0) { return []; }
  const braceStart = src.indexOf("#{", anchor);
  if (braceStart < 0) { return []; }
  // 花括号配对（rhai map 字面量；字符串里不含未转义的 { }）
  let depth = 0;
  let end = src.length;
  for (let i = braceStart + 1; i < src.length; i++) {
    if (src[i] === "{") { depth++; }
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) {
        end = i + 1;
        break;
      }
    }
  }
  const block = src.slice(braceStart, end);
  const out: string[] = [];
  const re = /"([a-z][a-z0-9_]*)":/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(block)) !== null) {
    // source 是元信息（"template" | "rhai_default"），本身不是可调参数
    if (m[1] !== "source" && !out.includes(m[1])) { out.push(m[1]); }
  }
  return out;
}

/** 从 `b("name", ...)` 调用中抽取出所有的变量名 */
function extractVarNamesFromPanel(): string[] {
  const src = readPanelSource();
  const re = /b\(\s*"([a-z_][a-z0-9_]*)"/g;
  const names = new Set<string>();
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) { names.add(m[1]); }
  return [...names];
}

/** 从 rust 文件的 `name: "...".into(),` 中抽取出种子化变量名 */
function extractVarNamesFromRust(): string[] {
  const src = readRustSource();
  // 排除后端不种子化的占位（如 old_variables 序列化后再 read 时的 key）
  // 仅匹配 v19 段：`name: "...".into(),` 且 var_type 是 number/string/boolean/enum
  const re = /name:\s*"([a-z_][a-z0-9_]*)"\.into\(\),\s*\n\s*var_type:\s*"(number|string|boolean|enum)"/g;
  const names = new Set<string>();
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) { names.add(m[1]); }
  return [...names];
}

describe("StockAnalysisConfigPanel 默认变量与后端模板 v19 同步", () => {
  it("UI defaults 全部使用 snake_case（无 camelCase / 混合格式）", () => {
    const src = readPanelSource();
    // 旧的 camelCase key 不应再出现
    const deprecated = [
      "analysis_maxDebateRounds",
      "analysis_maxConcurrent",
      "analysis_klinePeriod",
      "analysis_klineLimit",
      "analysis_newsLimit",
      "analysis_temperature",
      "analysis_maxTokens",
      "analysis_timeoutSecs",
      "rule_rsiOverbought",
      "rule_rsiOversold",
      "rule_biasLimit",
      "rule_volumeSignalBlock",
      "rule_bearLowScore",
      "rule_autoStopLossPct",
      "pos_maxSingleStockPct",
      "pos_maxTotalPositions",
      "pos_maxSectorExposurePct",
      "value_dcfGrowthRate",
      "value_dcfPerpetualRate",
      "value_dcfDiscountRate",
      "value_moatThreshold",
      "value_fScoreBuyThreshold",
      "value_safetyMarginMin",
      "monitor_pollIntervalSecs",
      "monitor_changePctThreshold",
      "monitor_turnoverThreshold",
      "tool_timeoutSecs",
      "tool_retryMax",
    ];
    for (const k of deprecated) {
      expect(src, `面板中不应再出现旧 key "${k}"`).not.toContain(`"${k}"`);
    }
  });

  it("UI 暴露了 agent_/tool_ 运行时关键参数", () => {
    const names = extractVarNamesFromPanel();
    for (
      const v of [
        "agent_temperature",
        "agent_max_tokens",
        "agent_timeout_secs",
        "agent_retry_max",
        "tool_timeout_secs",
        "tool_retry_max",
        "max_concurrent",
        "debate_rounds",
        "analysis_depth",
      ]
    ) {
      expect(names, `缺少运行时参数 ${v}`).toContain(v);
    }
  });

  it("UI 暴露了 A 类补全参数，且这些参数确实已接线（面板声明 ∧ 变量表定义）", () => {
    const names = extractVarNamesFromPanel();
    const seed = extractVarNamesFromSeedVariables();
    // 2026-09-11：原列表含 23 项「面板声明但变量表无定义」的死配置
    // （moat_* / screener_* / pos_min_cash_pct / rev_* / val_* / cap_* /
    //  trading_price_deviation_limit），此断言实际上在守护**虚假的完整性** ——
    // 它要求面板暴露一批改了不生效的参数。已一并移除，并补上 seed 侧断言。
    const must = [
      "scoring_boll",
      // 信号
      "signal_ma_fast",
      "signal_ma_slow",
      "signal_breakout_volume_mult",
      // 关键价位
      "keylevel_lookback_days",
      "keylevel_touch_tolerance_pct",
      "keylevel_min_touches",
      // 推荐器
      "reco_trend_enabled",
      "reco_reversion_enabled",
      "reco_value_enabled",
      "reco_capital_enabled",
      "reco_watchlist_enabled",
      "reco_min_confidence",
      // 风险/仓位扩展
      "risk_max_drawdown_limit",
      "risk_max_daily_loss_pct",
      "risk_correlation_lookback_days",
      "kelly_min_win_rate",
      "kelly_min_odds",
      // 监控告警
      "monitor_alert_cooldown_secs",
      "monitor_min_severity",
      "monitor_channels",
      // 决策回溯
      "decision_max_history_per_stock",
      // 技术指标 B1
      "macd_fast",
      "macd_slow",
      "macd_signal",
      "boll_period",
      "boll_stddev",
      "volume_lookback",
      "volume_surge_ratio",
      "volume_shrink_ratio",
      // 推荐器策略参数 B3
      "trend_kline_limit",
      "trend_amount_ratio_min",
      "rev_rsi_short_max",
      // 风险模型扩展 B4
      "risk_sharpe_annualization",
      "risk_kelly_heavy_threshold",
      "risk_kelly_medium_threshold",
    ];
    for (const v of must) {
      expect(names, `参数未在面板声明: ${v}`).toContain(v);
      expect(
        seed.has(v),
        `参数未在变量表定义 → rhai/工具读取时 present() 恒假、静默走硬编码，`
          + `面板改了不生效（配置项空接线）: ${v}`,
      ).toBe(true);
    }
  });

  it("PORTFOLIO_MGR_TUNABLE_PARAMS 每项都在变量表中有定义（决策参数单一权威源 vs 变量表）", () => {
    const seed = extractVarNamesFromSeedVariables();
    const missing = extractPortfolioMgrTunableParams().filter((k) => !seed.has(k));
    expect(
      missing,
      `可调参数清单里登记但变量表未定义 → 反思清单会剔除它们、rhai 走硬编码默认值。`
        + `请补 seed_variables.rs 定义: ${missing.join(", ")}`,
    ).toEqual([]);
  });

  it("面板不重复暴露 vendor_*（由 DataVendorsTab 全权管理）", () => {
    const names = extractVarNamesFromPanel();
    for (
      const v of [
        "vendor_tencent",
        "vendor_eastmoney",
        "vendor_sina",
        "vendor_ths",
        "vendor_cninfo",
        "vendor_baidu_stock",
        "vendor_iwencai",
        "vendor_akshare",
        "vendor_mootdx",
      ]
    ) {
      expect(names, `vendor_* 不应在参数面板里出现，${v} 改由 DataVendorsTab 管理`).not.toContain(v);
    }
  });

  it("面板定义的每个工作流参数 key 在后端模板 v19 中也存在（vendor_* 排除）", () => {
    const panel = new Set(extractVarNamesFromPanel());
    const rust = new Set(extractVarNamesFromRust());
    // 排除 vendor_*：它们由 DataVendorsTab 单独管理，避免两边同时写入竞态。
    // 排除 actual_outcome / reflection_depth / stock_lessons / actual_market_text：
    // **反思复盘工作流**的运行时注入变量，非用户配置参数。
    // ⚠️ 扫描源是 `stock_analysis_setup/mod.rs`（同时承载 reflection 模板定义），
    // 因此这类变量会被带进来 —— 判定依据是「由 Rust 在运行时写入、模板 variables
    // 里没有、也不该有面板入口」，与上一条注释的注入族同源，不是把检查绕过。
    const runtimeInjected = new Set([
      "actual_outcome",
      "reflection_depth",
      "stock_lessons",
      "actual_market_text",
    ]);
    const missingInPanel = [...rust]
      .filter((k) => !k.startsWith("vendor_"))
      .filter((k) => !runtimeInjected.has(k))
      .filter((k) => !panel.has(k));
    expect(missingInPanel, `后端已种子化但面板缺失: ${missingInPanel.join(", ")}`).toEqual([]);
  });

  it("KNOWN_UNWIRED_PARAMS 必须为空（禁止用技术债清单绕过接线检查）", () => {
    expect(
      [...KNOWN_UNWIRED_PARAMS],
      "该集合已于 2026-09-11 清空（29 项死配置已移除）。新增参数请补齐三件套："
        + "seed_variables.rs 定义 + input_mapping 映射 + 消费点读取，不要登记到此集合",
    ).toEqual([]);
  });

  it("面板声明的参数必须在变量表权威源 seed_variables.rs 中有定义（或为运行时注入 / 已知技术债）", () => {
    const seed = extractVarNamesFromSeedVariables();
    const unwired = extractVarNamesFromPanel().filter(
      (k) =>
        !seed.has(k)
        && !RUNTIME_INJECTED.has(k)
        && !KNOWN_UNWIRED_PARAMS.has(k)
        && !k.startsWith("vendor_"),
    );
    expect(
      unwired,
      `以下参数既无变量定义也无消费方。新增参数请补 seed_variables.rs 定义 + input_mapping，`
        + `不要加入 KNOWN_UNWIRED_PARAMS: ${unwired.join(", ")}`,
    ).toEqual([]);
  });

  it("portfolio-mgr input_mapping 的同名映射必须在变量表中有定义（或为运行时注入）", () => {
    const seed = extractVarNamesFromSeedVariables();
    const missing = extractSameNameMappings().filter(
      (k) => !seed.has(k) && !RUNTIME_INJECTED.has(k),
    );
    expect(
      missing,
      `同名映射指向不存在的变量 → rhai 的 present() 恒假、参数静默走硬编码默认值`
        + `（2026-09-11「配置项空接线」缺陷形态）: ${missing.join(", ")}`,
    ).toEqual([]);
  });

  it("市况先验 regime_prior_* 既有变量定义也已接入 input_mapping", () => {
    const seed = extractVarNamesFromSeedVariables();
    const mappings = new Set(extractSameNameMappings());
    for (const p of ["regime_prior_bull", "regime_prior_sideways", "regime_prior_bear"]) {
      expect(seed.has(p), `${p} 未在 seed_variables.rs 中定义`).toBe(true);
      expect(mappings.has(p), `${p} 未接入 portfolio-mgr input_mapping`).toBe(true);
    }
  });

  it("因子融合门 trader_cap_min_weight 四件套齐备（rhai 守卫 ∧ 变量定义 ∧ 权威源登记 ∧ 面板分组）", () => {
    // 该参数曾是「配置项空接线」的隐蔽形态：portfolio-mgr.rhai 早就写了
    // `if present(trader_cap_min_weight) { ... } else { 0.08 }` 守卫，看似可配置，
    // 但变量表从未定义该名、input_mapping 也无同名映射 → present() 恒假，
    // 永远走硬编码 0.08；且因守卫存在，读代码会误以为它已接线。
    // 因此必须逐环断言，缺任一环即静默失效。
    const rhai = readFileSync(PORTFOLIO_MGR_RHAI_PATH, "utf8");
    expect(
      rhai,
      "portfolio-mgr.rhai 缺少 present(trader_cap_min_weight) 守卫 → 参数改了也不被读取",
    ).toContain("present(trader_cap_min_weight)");

    const seed = extractVarNamesFromSeedVariables();
    expect(seed.has("trader_cap_min_weight"), "未在 seed_variables.rs 定义变量").toBe(true);

    expect(
      extractPortfolioMgrTunableParams(),
      "未登记进 PORTFOLIO_MGR_TUNABLE_PARAMS → 反思清单看不到、input_mapping 不注入",
    ).toContain("trader_cap_min_weight");

    expect(extractVarNamesFromPanel(), "面板未声明该参数").toContain("trader_cap_min_weight");
    expect(
      readPanelSource(),
      "面板仅声明未放入配置分组 → resolve().filter(Boolean) 会把它滤掉，界面看不到",
    ).toMatch(/resolve\(\[\s*"trader_cap_min_weight"\s*\]\)/);
  });

  it("面板「决策参数」分组（portfolio_mgr_*）与权威源 PORTFOLIO_MGR_TUNABLE_PARAMS 双向一致", () => {
    // 单向断言（权威源 ⊆ 变量表）只保证「后端能读到」，不保证「用户能看到」。
    // 本测试补上分组维度的双向核对：
    //   ① 分组暴露的名字必须在权威源内 —— 否则反思清单/input_mapping 都不注入，
    //      用户在界面改了不生效（反向的「配置项空接线」）；
    //   ② 权威源登记的每项都必须能在某个分组里看到 —— 否则「可调」只是纸面，
    //      用户没有入口。`cost_pct` 正是这个形态：权威源登记、input_mapping 注入、
    //      反思清单可见，但面板零出现，只能改 DB。
    const authoritative = new Set(extractPortfolioMgrTunableParams());
    const groups = extractPanelToolGroups();
    const mgrGroups = [...groups.entries()].filter(([tool]) => tool.startsWith("portfolio_mgr_"));
    expect(
      mgrGroups.length,
      "未找到任何 portfolio_mgr_* 分组 —— 分组命名约定已变，请同步本测试的过滤条件",
    ).toBeGreaterThan(0);

    const flattened: string[] = [];
    const notInAuthoritative: string[] = [];
    for (const [tool, vars] of mgrGroups) {
      for (const v of vars) {
        flattened.push(v);
        if (!authoritative.has(v)) { notInAuthoritative.push(`${tool} → ${v}`); }
      }
    }
    expect(
      notInAuthoritative,
      "面板分组暴露了权威源之外的参数 → 反思清单与 input_mapping 都不会注入，"
        + "用户改了不生效。请改为登记进 PORTFOLIO_MGR_TUNABLE_PARAMS 或从分组移除",
    ).toEqual([]);

    const visible = new Set(flattened);
    const noUiEntry = [...authoritative].filter((k) => !visible.has(k));
    expect(
      noUiEntry,
      "权威源登记为「可调」但没有任何面板分组暴露 → 用户无入口调整（只能改 DB），"
        + "「可调」沦为纸面声明。请补 b() 声明 + resolve() 分组，或从权威源移除",
    ).toEqual([]);
  });

  it("生效快照 effective_params 的字段集合与权威源一致（反思/演进观测面完整性）", () => {
    // 第三处派生点。前面几条测试保证了「参数能被读到」「用户能看到」，
    // 本条保证「反思/演进能观测到它实际取了什么值」—— 这是自动优化的前提：
    // 观测面残缺时，优化器只能看到因子权重、看不到判定该权重的门槛，
    // 归因必然错位（实测失效案例：trader_cap_min_weight 长期缺席）。
    const authoritative = extractPortfolioMgrTunableParams();
    const observed = extractEffectiveParamsFields();
    expect(
      observed.length,
      "未解析到 effective_params 字段 —— rhai 结构已变，请同步本测试的解析逻辑",
    ).toBeGreaterThan(0);

    const missing = authoritative.filter((p) => !observed.includes(p));
    expect(
      missing,
      "权威源登记为可调，但生效快照不输出其实际取值 → 反思/演进无法归因该参数，"
        + "请补进 portfolio-mgr.rhai 的 effective_params",
    ).toEqual([]);

    const extra = observed.filter((p) => !authoritative.includes(p));
    expect(
      extra,
      "生效快照输出了权威源之外的参数 → 它不会出现在反思参数清单里，"
        + "观测面与「可调集合」定义漂移。请登记进 PORTFOLIO_MGR_TUNABLE_PARAMS 或从快照移除",
    ).toEqual([]);
  });
});
