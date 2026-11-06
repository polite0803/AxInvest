// SPDX-License-Identifier: AGPL-3.0-only

import { PortfolioDashboard } from "@/components/stock-analysis/PortfolioDashboard";
import { WatchlistPage } from "@/components/stock-analysis/WatchlistPage";
import { useWorkspaceStore } from "@/stores";
import { Briefcase, Star } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 监控视图 — 工作区中栏的"监控"视图。
 *
 * 组合自选股管理 + 持仓 dashboard，通过子 Tab 切换。
 *
 * 裁定 9（PLAN-monitor-chain-closure 第 11 节，2026-10-15）：这两块**刻意不合并**成一个
 * dashboard —— 它们是两个实体、两条刷新路（`watchlist_items` vs 组合快照），各自有自己的
 * 空态与缺席语义；合并属结构调整，归下一轮独立批次。原注释写的「阶段 4 会整合为统一
 * dashboard」是一句**指向不存在的计划**的承诺（阶段 4 没有这个子项）⇒ 已改为如实描述现状。
 */
export function MonitorView() {
  const { t } = useTranslation();
  const currentStockCode = useWorkspaceStore((s) => s.currentStockCode);
  const [subTab, setSubTab] = useState<"watchlist" | "portfolio">("watchlist");

  return (
    <div className="flex flex-col h-full">
      {/* 子 Tab 切换器 */}
      <div
        className="flex items-center gap-1 px-3 py-1.5"
        style={{ borderBottom: "1px solid var(--border)", flexShrink: 0 }}
      >
        <button
          type="button"
          onClick={() => setSubTab("watchlist")}
          className="flex items-center gap-1 px-3 py-1 rounded text-sm transition-colors"
          style={{
            background: subTab === "watchlist" ? "var(--accent)" : "transparent",
            color: subTab === "watchlist" ? "var(--accent-fg)" : "var(--muted)",
          }}
        >
          <Star size={14} />
          {t("workspace.view.watchlist")}
        </button>
        <button
          type="button"
          onClick={() => setSubTab("portfolio")}
          className="flex items-center gap-1 px-3 py-1 rounded text-sm transition-colors"
          style={{
            background: subTab === "portfolio" ? "var(--accent)" : "transparent",
            color: subTab === "portfolio" ? "var(--accent-fg)" : "var(--muted)",
          }}
        >
          <Briefcase size={14} />
          {t("workspace.view.portfolio")}
        </button>
        {currentStockCode && (
          <span className="ml-auto text-sm" style={{ color: "var(--muted)" }}>
            {t("workspace.view.currentStock")}: {currentStockCode}
          </span>
        )}
      </div>

      {/* 内容区 */}
      <div className="flex-1 overflow-auto">
        {subTab === "watchlist" ? <WatchlistPage /> : <PortfolioDashboard />}
      </div>
    </div>
  );
}
