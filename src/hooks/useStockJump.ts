// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";

/** 跳转目标：代码必填；名称可选（有则一并带入 URL）。 */
export interface StockJumpTarget {
  code: string;
  /** 股票名称。调用点（候选列表 / 筛选结果 / 最近访问）本来就知道它，
   *  必须一并带上：工作区壳层只能从 URL 或「最近访问」推断名称，
   *  首次访问的股票两者皆无，会把代码当名称渲染成 `600519 (600519)`。 */
  name?: string | null;
  /** 是否保留当前视图（默认切到 analysis）。工作区内的股票切换器应传 true。 */
  keepView?: boolean;
}

/**
 * 从任意候选列表跳转到「股票分析工作区」的唯一实现。
 *
 * 为什么必须收敛到一个函数：InvestHub 内点股（改 query）与独立页点股
 * （跳 /stock-analysis）是两条路径，各自复制同一段逻辑时极易分叉 ——
 * 历史上 ?code 与 ?stockCode 两个参数名各写各的、消费端只认其一，
 * 造成「能切到分析页但搜索栏不预填」的静默失效。
 *
 * 参数约定（消费端见 StockAnalysisPage / StockWorkspaceShell）：
 *   - ?tab=workspace  当前业务 tab
 *   - ?stockCode=xxx  当前股票代码（唯一真相源）
 *   - ?stockName=xxx  当前股票名称（可选，避免壳层退化成用代码当名称）
 *   - ?view=xxx       工作区视图
 */
export function useStockJump(): (target: StockJumpTarget) => void {
  const navigate = useNavigate();
  const location = useLocation();
  const [searchParams, setSearchParams] = useSearchParams();

  return useCallback(({ code, name, keepView }: StockJumpTarget) => {
    if (!code) { return; }

    if (location.pathname.startsWith("/invest")) {
      // InvestHub 内部：URL 是当前股票的唯一真相源，改写 query 即可
      const next = new URLSearchParams(searchParams);
      const prevCode = next.get("stockCode");
      next.set("tab", "workspace");
      next.set("stockCode", code);
      if (name) {
        next.set("stockName", name);
      } else if (prevCode !== code) {
        // 换了股票又拿不到名称 → 必须清掉上一只股票的名称，
        // 否则壳层会把 A 的名称贴在 B 的代码上
        next.delete("stockName");
      }
      if (!keepView) {
        next.set("view", "analysis");
      }
      setSearchParams(next, { replace: true });
      return;
    }

    // 独立页：跳旧路由，由 ContentArea 的 RedirectToInvest 归一成
    // /invest?tab=workspace&stockCode=…（原 query 全量保留）
    const qs = new URLSearchParams({ code });
    if (name) {
      qs.set("stockName", name);
    }
    navigate(`/stock-analysis?${qs.toString()}`, { replace: true });
  }, [location.pathname, navigate, searchParams, setSearchParams]);
}
