// SPDX-License-Identifier: AGPL-3.0-only

import { RLTrainingPanel } from "@/components/devtools/RLTrainingPanel";
import {
  buildDevToolsSubSearch,
  DEFAULT_DEVTOOLS_SUB,
  DEVTOOLS_SUBS,
  type DevToolsSub,
  isDevToolsSub,
  parseDevToolsSub,
} from "@/lib/devtoolsSubTabs";
import { BenchmarkRunner } from "@/pages/DevTools/BenchmarkRunner";
import { ToolRecommender } from "@/pages/DevTools/ToolRecommender";
import { TraceExplorer } from "@/pages/DevTools/TraceExplorer";
import { FineTunePage } from "@/pages/FineTunePage";
import { Tabs } from "antd";
import { BrainCircuit, Gauge, Search, Trophy, Wand2 } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";

/**
 * 开发者工具统一页面。
 * 合并原 5 个独立侧栏导航项为 1 项，内部 Tab 切换：
 * 追踪浏览器 / 基准测试 / 工具推荐 / 模型微调 / 强化学习训练。
 *
 * 子页是**受控**的，真相源在 URL（`/chat?ws=devtools&sub=<sub>`）。
 * 此前用 `defaultActiveKey`，URL 里的子页信息既无法表达、也无法深链或刷新恢复 ——
 * 打开 `/devtools/benchmark` 永远落在首个 Tab（「追踪浏览器」）。
 */
export function DevToolsPage() {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();

  const activeSub = parseDevToolsSub(searchParams.toString()) ?? DEFAULT_DEVTOOLS_SUB;

  const handleSubChange = (key: string) => {
    // 脏值（含旧值残留、手改 URL）不写回，避免把非法 sub 持久化进地址栏
    if (!isDevToolsSub(key)) {
      return;
    }
    setSearchParams((prev) => new URLSearchParams(buildDevToolsSubSearch(prev.toString(), key)), {
      replace: true,
    });
  };

  const tabLabel = (icon: ReactNode, text: string) => (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      {icon}
      {text}
    </span>
  );

  /** 子页元数据。用 `Record<DevToolsSub, …>` 保证新增子页时编译器穷举报错。 */
  const tabPanels: Record<DevToolsSub, { icon: ReactNode; labelKey: string; panel: ReactNode }> = {
    "trace-explorer": {
      icon: <Search size={14} />,
      labelKey: "nav.devtools.traceExplorer",
      panel: <TraceExplorer />,
    },
    benchmark: {
      icon: <Gauge size={14} />,
      labelKey: "nav.devtools.benchmark",
      panel: <BenchmarkRunner />,
    },
    "tool-recommender": {
      icon: <Wand2 size={14} />,
      labelKey: "nav.devtools.toolRecommender",
      panel: <ToolRecommender />,
    },
    "fine-tune": {
      icon: <BrainCircuit size={14} />,
      labelKey: "nav.devtools.fineTune",
      panel: <FineTunePage />,
    },
    "rl-training": {
      icon: <Trophy size={14} />,
      labelKey: "nav.devtools.rlTraining",
      panel: <RLTrainingPanel />,
    },
  };

  // 顺序与 DEVTOOLS_SUBS 一致；key 直接复用子页 key，杜绝「Tab key 与 sub 对不上」
  const tabItems = DEVTOOLS_SUBS.map((sub) => ({
    key: sub,
    label: tabLabel(tabPanels[sub].icon, t(tabPanels[sub].labelKey)),
    children: tabPanels[sub].panel,
  }));

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <Tabs
        activeKey={activeSub}
        onChange={handleSubChange}
        items={tabItems}
        className="ax-fill-tabs"
        style={{ padding: "0 16px" }}
        tabBarStyle={{ flexShrink: 0, marginBottom: 0 }}
        destroyOnHidden
      />
    </div>
  );
}
