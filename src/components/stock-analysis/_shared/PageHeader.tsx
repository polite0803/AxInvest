import { PageTimeAnchor } from "@/components/time-travel/PageTimeAnchor";
import { type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

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
   * 是否渲染 live/as-of 锚点。默认不渲染 —— 使用本头部的股票业务页现已全部内嵌
   * InvestHub，世界观切换由投资中心标题栏唯一承载；只有独立整页（回放工作台）需要开启。
   */
  showTimeAnchor?: boolean;
}

/**
 * 股票业务页的统一头部：返回按钮 + 标题 + 右侧 meta/actions。
 * 复用 sa-header / sa-header-back / sa-header-title / sa-header-meta 样式。
 */
export function PageHeader({
  titleKey,
  backTo = "/",
  backLabelKey = "nav.chat",
  meta,
  actions,
  showTimeAnchor = false,
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
        {showTimeAnchor && <PageTimeAnchor />}
        {actions}
      </div>
    </header>
  );
}
