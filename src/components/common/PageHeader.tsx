import { type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import "./pageHeader.css";

interface PageHeaderProps {
  /** i18n 键，例如 "watchlist.title" */
  titleKey: string;
  /** 点击返回按钮时跳转的路径，默认 "/" */
  backTo?: string;
  /** 返回按钮的 i18n 键，默认 "nav.chat" */
  backLabelKey?: string;
  /** 标题右侧的元信息文本/节点 */
  meta?: ReactNode;
  /** 标题最右侧的按钮组 */
  actions?: ReactNode;
  /**
   * 标题最右侧、`actions` 之前的可选插槽（例如页面级时间锚点）。
   * 由调用方自行传入节点 —— 本组件不认识任何具体特性，才能留在中性目录。
   */
  anchor?: ReactNode;
}

/**
 * 通用页头：返回按钮 + 标题 + 右侧 meta/actions（+ 可选插槽）。
 *
 * 2026-11-05（B8-4a）由 `src/components/stock-analysis/_shared/PageHeader.tsx` 下沉至此。
 * 原 `showTimeAnchor?: boolean` 换成中性的 `anchor?: ReactNode` 插槽：组件不再反向
 * import `@/components/time-travel/PageTimeAnchor`（那会让中性目录依赖特性目录，触发
 * 组件中立性门禁）。
 *
 * 类名 `sa-header*` 刻意保留 —— 股票域 5 个单测按这些选择器断言（如
 * `stock-analysis/__tests__/BacktestPage.test.tsx` 的 `.sa-header-back`）。样式见
 * 同目录 `pageHeader.css`，由本模块自带 import，独立路由下也有样式（此前只在
 * `stock-analysis.css` 里，非股票页头部裸奔）。
 */
export function PageHeader({
  titleKey,
  backTo = "/",
  backLabelKey = "nav.chat",
  meta,
  actions,
  anchor,
}: PageHeaderProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <header className="sa-header">
      <button type="button" className="sa-header-back" onClick={() => navigate(backTo)}>
        ‹ {t(backLabelKey)}
      </button>
      <h2 className="sa-header-title">{t(titleKey)}</h2>
      {meta && <span className="sa-header-meta">{meta}</span>}
      <div className="ml-auto flex items-center gap-2">
        {anchor}
        {actions}
      </div>
    </header>
  );
}
