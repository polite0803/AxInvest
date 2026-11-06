// SPDX-License-Identifier: AGPL-3.0-only

import { Tooltip } from "@/components/layout/Tooltip";
import { BacktestPage } from "@/components/stock-analysis/BacktestPage";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import { Clock } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

/**
 * 回测视图 — 工作区中栏的"回测"视图。
 *
 * 直接复用现有 BacktestPage 组件；顶部追加"时间旅行回放"入口按钮。
 * 回放工作台的落点由 `BUILTIN_PAGE_PATH.investReplay` 给出（`/invest/replay`）——
 * ⚠ 不要再写 `BUILTIN_PAGE_PATH["replay-workbench"]`：该键随 2026-10-28 的
 * 「股票旧路由删除」一并移除，取它得到 `undefined`，`navigate(undefined)` 不跳转
 * 且不报错（点了没反应的静默失效）。
 */
export function BacktestView() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  return (
    <div className="relative h-full">
      <Tooltip title={t("replayWorkbench.title")} placement="left">
        <button
          type="button"
          onClick={() => navigate(BUILTIN_PAGE_PATH.investReplay)}
          className="absolute top-2 right-3 z-10 flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors hover:opacity-80"
          style={{
            background: "var(--surface)",
            border: "1px solid var(--border)",
            color: "var(--color-text-secondary)",
          }}
          aria-label={t("replayWorkbench.title")}
        >
          <Clock size={12} />
          {t("replayWorkbench.title")}
        </button>
      </Tooltip>
      <BacktestPage />
    </div>
  );
}
