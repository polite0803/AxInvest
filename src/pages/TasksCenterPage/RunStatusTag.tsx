// SPDX-License-Identifier: AGPL-3.0-only

import { Tag, Tooltip } from "antd";
import { useTranslation } from "react-i18next";
import type { RunBadge, RunState } from "./model";

const STATE_COLOR: Record<RunState, string> = {
  queued: "default",
  running: "processing",
  waiting: "warning",
  paused: "warning",
  succeeded: "success",
  failed: "error",
  canceled: "default",
};

const BADGE_COLOR: Record<RunBadge, string> = {
  partial: "warning",
  retrying: "processing",
  interrupted: "default",
  stale: "error",
  skipped: "default",
};

/** 统一状态标签：主状态 Tag + 可选 badge（部分成功/中断/跳过/僵死）。 */
export function RunStatusTag({
  state,
  badges,
}: {
  state: RunState;
  badges?: RunBadge[];
}) {
  const { t } = useTranslation();
  return (
    <span style={{ display: "inline-flex", gap: 4, alignItems: "center" }}>
      <Tag color={STATE_COLOR[state]}>{t(`tasksCenter.state.${state}`)}</Tag>
      {badges?.map((b) => (
        <Tooltip key={b} title={t(`tasksCenter.badge.${b}Tip`)}>
          <Tag color={BADGE_COLOR[b]}>{t(`tasksCenter.badge.${b}`)}</Tag>
        </Tooltip>
      ))}
    </span>
  );
}
