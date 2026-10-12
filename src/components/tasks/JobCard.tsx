// SPDX-License-Identifier: AGPL-3.0-only

import { Card, theme } from "antd";
import type { ReactNode } from "react";

interface JobCardProps {
  /** 作业图标（lucide 等，16-18px），渲染在统一的圆角底块中 */
  icon: ReactNode;
  /** 作业名称 */
  title: ReactNode;
  /** 一句话作业说明（头部副标题） */
  description?: ReactNode;
  /** 头部右侧操作区（立即运行 / 暂停等） */
  extra?: ReactNode;
  /** 标题行下方的「最近状态条」：运行中 Tag / 最近一次结果 / 统计 */
  status?: ReactNode;
  /** 作业主体：参数表单、进度明细等（无主体时不传） */
  children?: ReactNode;
  testId?: string;
}

/**
 * 手动作业统一卡片规格（任务中心 → 手动作业）：
 *
 * ```
 * ┌──────────────────────────────────────────────┐
 * │ [icon] 作业名称                  [操作区]    │
 * │        作业说明                              │
 * │ 最近状态条（可选）                           │
 * │ ──────────────────────────────────────────── │
 * │ 作业主体（参数 / 进度 / 结果）               │
 * └──────────────────────────────────────────────┘
 * ```
 *
 * 三张作业卡（投资管道 / 批次复盘 / 回放扫描）共用本壳，保证图标底块、
 * 标题密度、操作区位置、状态条位置完全一致（AC-8 视觉一致性）。
 */
export function JobCard({ icon, title, description, extra, status, children, testId }: JobCardProps) {
  const { token } = theme.useToken();

  return (
    <Card
      size="small"
      data-testid={testId}
      style={{ marginBottom: token.marginMD }}
      styles={{ body: { padding: token.paddingLG } }}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <div
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg"
            style={{ background: token.colorFillTertiary, color: token.colorTextSecondary }}
          >
            {icon}
          </div>
          <div className="min-w-0">
            <div className="m-0 text-sm font-semibold leading-5">{title}</div>
            {description
              ? (
                <div className="mt-0.5 text-xs leading-5" style={{ color: token.colorTextSecondary }}>
                  {description}
                </div>
              )
              : null}
          </div>
        </div>
        {extra ? <div className="shrink-0">{extra}</div> : null}
      </div>

      {status ? <div className="mt-3">{status}</div> : null}

      {children ? <div className="mt-3">{children}</div> : null}
    </Card>
  );
}
