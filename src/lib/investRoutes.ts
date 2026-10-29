// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 投资中心（`/invest`）深链的**唯一构造入口**。
 *
 * 为什么必须收敛：2026-10-28 起全部股票旧顶级路由（`/stock-analysis/:id`、`/trade`、
 * `/watchlist`、`/backtest`、`/screener`、`/quant` …）已从 `ContentArea` 删除，落到
 * `path="*"` 的 404 页。任何仍拼旧路径的调用点都会在**运行时**才暴露 —— 类型系统
 * 拦不住字符串，单元测试也只断言自己拼的那一段（实测：任务中心单股分析详情抽屉的
 * `deepLink` 仍写 `/stock-analysis/${id}`，点「打开」直接 404）。
 *
 * 参数契约（消费端）：
 *   - `?tab=`        InvestHub 业务 tab（`screener` / `workspace` / …）
 *   - `?stockCode=`  当前股票代码（唯一真相源，`StockWorkspaceShell` 消费）
 *   - `?stockName=`  当前股票名称（可选；缺省会退化成「代码 (代码)」）
 *   - `?view=`       工作区视图（`analysis` / `monitor` / `trade` / `backtest` / …）
 *   - `?analysisId=` 指定要加载的历史分析记录 id（`StockAnalysisPage` 消费）
 */

import { BUILTIN_PAGE_PATH } from "./pageRegistry";

/** InvestHub 的合法业务 tab（与 InvestHub 的 `VALID_TABS` 同一集合）。 */
export type InvestTab =
  | "screener"
  | "workspace"
  | "cross-market"
  | "screenshot-diagnosis"
  | "paper-portfolio"
  | "quant";

/** 工作区视图（与 `WorkspaceView` 同一集合，去掉了移动端专有的 `more`）。 */
export type InvestWorkspaceView =
  | "analysis"
  | "monitor"
  | "trade"
  | "backtest"
  | "compare"
  | "review";

export interface InvestLinkTarget {
  /** 默认 `workspace`（单股相关跳转的绝大多数）。 */
  tab?: InvestTab;
  stockCode?: string | null;
  stockName?: string | null;
  /** 默认 `analysis`。传 `null` 表示不改写视图（保留用户当前视图）。 */
  view?: InvestWorkspaceView | null;
  /** 历史分析记录 id：定位到某一条既有分析结果。 */
  analysisId?: string | null;
  /** 其余附带的业务参数（如 `moverRun`），值为 `null` 时跳过。 */
  extra?: Record<string, string | null | undefined>;
}

/**
 * 构造 `/invest` 深链（不含 origin）。
 *
 * `stockName` **与 `stockCode` 同生共死**：只带名称会让工作区把上一只股票的名称
 * 贴到新代码上；反之只有代码时 `StockWorkspaceShell` 会退化成「代码 (代码)」。
 * 因此这里在缺 `stockCode` 时直接丢弃 `stockName`。
 */
export function investPath(target: InvestLinkTarget = {}): string {
  const params = new URLSearchParams();
  params.set("tab", target.tab ?? "workspace");
  if (target.stockCode) {
    params.set("stockCode", target.stockCode);
    if (target.stockName) {
      params.set("stockName", target.stockName);
    }
  }
  const view = target.view === undefined ? "analysis" : target.view;
  if (view !== null) {
    params.set("view", view);
  }
  if (target.analysisId) {
    params.set("analysisId", target.analysisId);
  }
  for (const [key, value] of Object.entries(target.extra ?? {})) {
    if (value) {
      params.set(key, value);
    }
  }
  return `${BUILTIN_PAGE_PATH["finance-investment"]}?${params.toString()}`;
}

/** 定位到某一条历史分析记录（单股分析详情 / 反思跳原记录 / 历史分析列表共用）。 */
export function analysisRecordPath(record: {
  id: string;
  stockCode?: string | null;
  stockName?: string | null;
}): string {
  return investPath({
    tab: "workspace",
    view: "analysis",
    stockCode: record.stockCode,
    stockName: record.stockName,
    analysisId: record.id,
  });
}
