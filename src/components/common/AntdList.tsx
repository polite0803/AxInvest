/**
 * antd List 替代组件
 *
 * antd v6 已弃用 List（v7 将移除），此组件实现所需的 API 子集：
 * - dataSource + renderItem
 * - Item（actions, className, style, onClick）
 * - Item.Meta（avatar, title, description）
 * - size（small / default）
 * - grid（响应式 CSS grid）
 * - locale.emptyText
 */

import React from "react";

// ── Types ──

interface ListGrid {
  gutter?: number;
  xs?: number;
  sm?: number;
  md?: number;
  lg?: number;
  xl?: number;
  xxl?: number;
}

interface ListLocale {
  emptyText?: React.ReactNode;
}

interface ListProps<T> {
  dataSource?: T[];
  renderItem?: (item: T, index: number) => React.ReactNode;
  size?: "small" | "default" | "large";
  grid?: ListGrid;
  locale?: ListLocale;
  className?: string;
  header?: React.ReactNode;
  footer?: React.ReactNode;
  style?: React.CSSProperties;
  bordered?: boolean;
  itemLayout?: string;
  split?: boolean;
}

export interface ItemProps {
  children?: React.ReactNode;
  actions?: React.ReactNode[];
  className?: string;
  style?: React.CSSProperties;
  onClick?: () => void;
}

export interface MetaProps {
  avatar?: React.ReactNode;
  title?: React.ReactNode;
  description?: React.ReactNode;
}

// ── Helpers ──

function getGridBreakpoint(grid?: ListGrid): string {
  if (!grid) { return ""; }
  const cols: string[] = [];
  if (grid.xs) { cols.push(`grid-cols-${grid.xs}`); }
  if (grid.sm) { cols.push(`sm:grid-cols-${grid.sm}`); }
  if (grid.md) { cols.push(`md:grid-cols-${grid.md}`); }
  if (grid.lg) { cols.push(`lg:grid-cols-${grid.lg}`); }
  if (grid.xl) { cols.push(`xl:grid-cols-${grid.xl}`); }
  if (grid.xxl) { cols.push(`2xl:grid-cols-${grid.xxl}`); }
  return cols.join(" ");
}

// ── Sub-components ──

function Meta({ avatar, title, description }: MetaProps) {
  return (
    <div className="flex items-start gap-3">
      {avatar && <div className="shrink-0">{avatar}</div>}
      <div className="flex-1 min-w-0">
        {title && <div className="text-sm font-medium truncate">{title}</div>}
        {description && <div className="text-xs text-gray-500 mt-0.5 line-clamp-2">{description}</div>}
      </div>
    </div>
  );
}

function Item({ children, actions, className = "", style, onClick }: ItemProps) {
  return (
    <div
      className={`flex items-start justify-between px-2 py-3 border-b border-gray-100 hover:bg-gray-50/50 cursor-pointer ${className}`}
      style={style}
      onClick={onClick}
      role="listitem"
    >
      <div className="flex-1 min-w-0">{children}</div>
      {actions && actions.length > 0 && (
        <div className="flex items-center gap-1 ml-2 shrink-0">
          {actions.map((action, i) => <span key={i} onClick={(e) => e.stopPropagation()}>{action}</span>)}
        </div>
      )}
    </div>
  );
}

Item.Meta = Meta;

// ── Main List component ──

function ListInner<T>({
  dataSource,
  renderItem,
  size,
  grid,
  locale,
  className = "",
  style,
}: ListProps<T>) {
  const data = dataSource ?? [];
  const emptyText = locale?.emptyText ?? "No data";
  void size; // accept antd List API; Item handles its own padding

  if (data.length === 0) {
    return (
      <div className="flex justify-center items-center py-8 text-sm text-gray-400">
        {emptyText}
      </div>
    );
  }

  const gap = grid?.gutter ?? 4;

  if (grid) {
    return (
      <div
        className={`gap-${gap} ${getGridBreakpoint(grid)} ${className}`}
        style={{ display: "grid", ...style }}
        role="list"
      >
        {data.map((item, i) => <React.Fragment key={i}>{renderItem?.(item, i)}</React.Fragment>)}
      </div>
    );
  }

  return (
    <div className={`flex flex-col ${className}`} style={style} role="list">
      {data.map((item, i) => <React.Fragment key={i}>{renderItem?.(item, i)}</React.Fragment>)}
    </div>
  );
}

// ── Export ──

const List = ListInner as typeof ListInner & { Item: typeof Item };
List.Item = Item;

export { List };
export type { ListGrid, ListLocale, ListProps };
