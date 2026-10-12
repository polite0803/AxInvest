// SPDX-License-Identifier: AGPL-3.0-only

import { Tabs, Typography } from "antd";
import { ListChecks } from "lucide-react";
import { type ReactNode, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";
import { CreateJobsMenu } from "./CreateJobsMenu";
import { OverviewTab } from "./OverviewTab";
import { RunDetailDrawer } from "./RunDetailDrawer";
import { RunsTab } from "./RunsTab";
import { ScheduledJobsTab } from "./ScheduledJobsTab";

const { Title } = Typography;

/**
 * 任务中心 v2（参考 Temporal / Windmill / GitHub Actions）：
 * - overview：健康统计 + 需关注 + Live 进行中 + 近 24h
 * - runs：跨全部任务源的统一运行表（过滤 / 统一详情抽屉）
 * - schedules：Cron 计划定义管理（原定时任务面板）
 *
 * 页面结构对齐其它中心页（InvestHub 等）：全局 AppHeader 提供返回栏，
 * 正文仅放 三级标题行 + small Tabs；Tab 与 URL query `tab` 同步。
 * 历史锚点兼容：scheduled→schedules、manual→runs。
 */
type TasksTabKey = "overview" | "runs" | "schedules";

/** Tab 正文：统一内边距 + 纵向滚动（各 tab 面板不重复设置 padding）。 */
function TabBody({ children }: { children: ReactNode }) {
  return (
    <div style={{ padding: "12px 16px 16px", height: "100%", overflowY: "auto" }}>
      {children}
    </div>
  );
}

const TAB_KEYS: readonly TasksTabKey[] = ["overview", "runs", "schedules"];

/** 旧版 tab 锚点 → v2 tab */
const LEGACY_TAB: Record<string, TasksTabKey> = {
  scheduled: "schedules",
  manual: "runs",
};

function normalizeTab(value: string | null): TasksTabKey {
  if (value === null) {
    return "overview";
  }
  if ((TAB_KEYS as readonly string[]).includes(value)) {
    return value as TasksTabKey;
  }
  return LEGACY_TAB[value] ?? "overview";
}

export function TasksCenterPage() {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();

  const activeKey = normalizeTab(searchParams.get("tab"));

  const handleTabChange = useCallback(
    (key: string) => {
      const next = normalizeTab(key);
      const params = new URLSearchParams(searchParams);
      if (next === "overview") {
        params.delete("tab");
      } else {
        params.set("tab", next);
      }
      setSearchParams(params, { replace: true });
    },
    [searchParams, setSearchParams],
  );

  const goRuns = useCallback(() => {
    handleTabChange("runs");
  }, [handleTabChange]);

  const items = [
    {
      key: "overview",
      label: t("tasksCenter.tab.overview"),
      children: (
        <TabBody>
          <OverviewTab onViewRuns={goRuns} />
        </TabBody>
      ),
    },
    {
      key: "runs",
      label: t("tasksCenter.tab.runs"),
      children: (
        <TabBody>
          <RunsTab />
        </TabBody>
      ),
    },
    {
      key: "schedules",
      label: t("tasksCenter.tab.schedules"),
      children: (
        <TabBody>
          <ScheduledJobsTab />
        </TabBody>
      ),
    },
  ];

  return (
    <div className="flex h-full w-full min-h-0 flex-col">
      <div
        className="flex items-center justify-between gap-3 flex-wrap"
        style={{ padding: "12px 16px 0", background: "var(--color-bg-container)" }}
      >
        <Title level={3} style={{ margin: 0 }}>
          <ListChecks size={20} style={{ marginRight: 8, verticalAlign: "middle" }} />
          {t("tasksCenter.title")}
        </Title>
        <CreateJobsMenu onSelectSchedule={() => handleTabChange("schedules")} />
      </div>

      <Tabs
        activeKey={activeKey}
        onChange={handleTabChange}
        destroyInactiveTabPane={false}
        items={items}
        className="invest-hub-tabs ax-fill-tabs"
        tabBarStyle={{
          margin: 0,
          padding: "0 16px",
          background: "var(--color-bg-container)",
          borderBottom: "1px solid var(--color-border-secondary)",
        }}
        tabBarGutter={24}
        size="small"
      />

      <RunDetailDrawer />
    </div>
  );
}
