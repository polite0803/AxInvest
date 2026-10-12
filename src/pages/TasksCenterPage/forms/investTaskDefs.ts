// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 投资类定时任务的 7 个种类定义（纯数据 + 纯载荷构造）。
 *
 * 历史背景：这些任务原先分散在「股票设置 → 定时分析 / 定时荐股」两个 tab，
 * 每类各走一个专用 `create_*_cron` 命令；它们与统一 `cron_job_store` 同源
 * （任务中心统一列表天然可见），只是创建时业务参数封装不同，故无法像自定义
 * 任务那样统一走 `create_scheduled_task`。
 *
 * 本文件只做「种类 → 命令 + camelCase 载荷」的映射，UI 在 InvestTaskFields；
 * 保持纯函数是为了让 TR-7.1 的载荷断言无需渲染组件。
 */

export type InvestTaskKind =
  | "stock-analysis"
  | "watchlist-scan"
  | "pool-scan"
  | "trend-screening"
  | "batch-reflection"
  | "decision-backtest"
  | "stock-recommendation";

export interface InvestKindDef {
  kind: InvestTaskKind;
  /** 种类名称 i18n 键（tasksCenter.create.kind.*） */
  nameKey: string;
  /** 种类说明 i18n 键（tasksCenter.create.kind.*Desc） */
  descKey: string;
  /** 表单初始值（含业务推荐的默认 cron 时点） */
  initialValues: Record<string, unknown>;
  /** 专用创建命令名 */
  command: string;
  /**
   * 由表单值构造命令参数。Tauri v2 命令宏默认 camelCase，
   * 故这里的键必须与 Rust 参数名的 camelCase 形态逐字一致。
   */
  buildArgs: (values: Record<string, unknown>) => Record<string, unknown>;
}

/** 仅收集非 undefined 的可选参数（null 是合法语义值，必须保留） */
function definedOnly(values: Record<string, unknown>, keys: string[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const key of keys) {
    if (values[key] !== undefined) {
      out[key] = values[key];
    }
  }
  return out;
}

export const INVEST_KIND_DEFS: InvestKindDef[] = [
  {
    kind: "stock-analysis",
    nameKey: "tasksCenter.create.kind.stockAnalysis",
    descKey: "tasksCenter.create.kind.stockAnalysisDesc",
    initialValues: {
      stockCode: "",
      stockName: "",
      cronExpression: "0 9 * * *",
    },
    command: "create_stock_cron",
    buildArgs: (v) => ({
      stockCode: v.stockCode,
      stockName: v.stockName,
      cronExpression: v.cronExpression,
    }),
  },
  {
    kind: "watchlist-scan",
    nameKey: "tasksCenter.create.kind.watchlistScan",
    descKey: "tasksCenter.create.kind.watchlistScanDesc",
    initialValues: {
      // 工作日 21:00：晚于收盘、数据落定后
      cronExpression: "0 21 * * 1-5",
    },
    command: "create_watchlist_scan_cron",
    buildArgs: (v) => ({
      cronExpression: v.cronExpression,
      enabled: true,
    }),
  },
  {
    kind: "pool-scan",
    nameKey: "tasksCenter.create.kind.poolScan",
    descKey: "tasksCenter.create.kind.poolScanDesc",
    initialValues: {
      // 工作日 17:00：晚于趋势智选 16:00，避免同时抢 astock 配额
      cronExpression: "0 17 * * 1-5",
    },
    command: "create_pool_scan_cron",
    buildArgs: (v) => ({
      cronExpression: v.cronExpression,
      enabled: true,
      // 三项均可选；留空走后端 PoolScanConfig 默认值
      ...definedOnly(v, ["lookbackDays", "maxStocks", "minConfidence"]),
    }),
  },
  {
    kind: "trend-screening",
    nameKey: "tasksCenter.create.kind.trendScreening",
    descKey: "tasksCenter.create.kind.trendScreeningDesc",
    initialValues: {
      // 与后端 recommendation_cron.rs 白名单逐字一致，传错直接报错
      chain: "serenity-screening",
      cronExpression: "0 16 * * 1-5",
    },
    command: "create_trend_screening_cron",
    buildArgs: (v) => ({
      cronExpression: v.cronExpression,
      enabled: true,
      workflowId: v.chain,
    }),
  },
  {
    kind: "batch-reflection",
    nameKey: "tasksCenter.create.kind.batchReflection",
    descKey: "tasksCenter.create.kind.batchReflectionDesc",
    initialValues: {
      period: "short",
      cronExpression: "0 18 * * *",
    },
    command: "create_batch_reflection_cron",
    buildArgs: (v) => ({
      cronExpression: v.cronExpression,
      period: v.period,
      // 旧设置页表单固定 false（到期/未到期都反思），行为平移
      dueOnly: false,
      enabled: true,
    }),
  },
  {
    kind: "decision-backtest",
    nameKey: "tasksCenter.create.kind.decisionBacktest",
    descKey: "tasksCenter.create.kind.decisionBacktestDesc",
    initialValues: {
      // 与后端 RunDecisionBacktestRequest 默认 [5,20,60] 一致：短/中/长窗口
      windows: [5, 20, 60],
      period: "all",
      maxPicks: 200,
      // 07:00：晚于反思族、早于收市；只读历史 K 线
      cronExpression: "0 7 * * *",
    },
    command: "create_decision_backtest_cron",
    buildArgs: (v) => ({
      cronExpression: v.cronExpression,
      // 「全部周期」必须传 null（后端 Option<String> 的 None）；
      // 传空串会被当成不存在的周期名 ⇒ 零产物却报成功（最隐蔽的静默失败）
      periodFilter: v.period === "all" ? null : v.period,
      tPlusNList: v.windows,
      maxPicks: v.maxPicks,
      enabled: true,
    }),
  },
  {
    kind: "stock-recommendation",
    nameKey: "tasksCenter.create.kind.recommendation",
    descKey: "tasksCenter.create.kind.recommendationDesc",
    initialValues: {
      // name 默认值在组件内用 i18n（default-name）填入，此处不硬编码
      name: "",
      periods: ["short", "mid"],
      // 50 与后端 RecoCronConfig 兜底值、模板变量 reco_min_confidence 出厂值三处一致
      minConfidence: 50,
      topN: 5,
      cronExpression: "30 15 * * *",
    },
    command: "create_recommendation_cron",
    buildArgs: (v) => ({
      name: v.name,
      cronExpression: v.cronExpression,
      periods: v.periods,
      minConfidence: v.minConfidence,
      topN: v.topN,
    }),
  },
];

export function getInvestKindDef(kind: InvestTaskKind): InvestKindDef {
  const def = INVEST_KIND_DEFS.find((d) => d.kind === kind);
  if (!def) {
    // 内部 invariant：kind 来自联合类型，命中说明映射表漏配（开发者错误，不进 UI）
    throw new Error(`Unknown investment task kind: ${kind}`);
  }
  return def;
}
