// SPDX-License-Identifier: AGPL-3.0-only

import { investPath } from "@/lib/investRoutes";
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
 * （跳 /invest）是两条路径，各自复制同一段逻辑时极易分叉 ——
 * 历史上 ?code 与 ?stockCode 两个参数名各写各的、消费端只认其一，
 * 造成「能切到分析页但搜索栏不预填」的静默失效；又例如 2026-10-28 旧路由删除后，
 * 独立页分支仍拼 `/stock-analysis`，点股票直接落 404。
 * 目标 URL 一律经 `@/lib/investRoutes` 的 `investPath` 构造，不在本文件拼字符串。
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

    // 独立页：统一落到投资工作区（2026-10-28 起 `/stock-analysis` 旧路由已删除，
    // 还往那儿跳会直接命中 `path="*"` 的 404 页）。
    // 视图固定切到 analysis —— 点股票无论从哪里来，用户想要的都是「分析这只票」；
    // `?code` 交由 StockAnalysisPage 的预填 effect 消费（与 InvestHub 内跳转同一套参数）。
    navigate(
      investPath({ tab: "workspace", stockCode: code, stockName: name, view: "analysis" }),
      { replace: true },
    );
  }, [location.pathname, navigate, searchParams, setSearchParams]);
}
