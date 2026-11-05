// SPDX-License-Identifier: AGPL-3.0-only

import { useTaskCenterStore } from "@/stores";
import { Drawer, Empty } from "antd";
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { getAdapter } from "./adapters/registry";

/**
 * 统一运行详情入口：Runs 表/概览点击任意 run → 该源的详情形态。
 *
 * 本文件**不含任何源特定分支**（任务 #151 / PLAN §100）：详情形态由各适配器在
 * `renderDetail` 里声明，这里只按注册表取回并渲染。理由不是「代码更整齐」——
 * 改造前这里是五条 `selected.source === "…"` 字面量判断，其中一条 import 了股票域的
 * 详情组件，于是「新增一个任务源」和「主壳少一处股票耦合」都得回来改这个中性文件。
 *
 * 未声明 `renderDetail` 的源（插件贡献的源今天全部如此，§93 禁第三方 React 进主 origin）
 * 走下面的空抽屉兜底 —— 与改造前的表现逐字一致。
 */
export function RunDetailDrawer() {
  const { t } = useTranslation();
  const selected = useTaskCenterStore((s) => s.selectedRun);
  const selectRun = useTaskCenterStore((s) => s.selectRun);

  const close = useCallback(() => selectRun(null), [selectRun]);

  if (selected) {
    const detail = getAdapter(selected.source)?.renderDetail?.({ run: selected, close });
    if (detail) {
      return <>{detail}</>;
    }
  }

  // 兜底：源未声明详情形态，或该 run 的源已不在注册表里（插件被禁用等）。
  return (
    <Drawer
      title={selected?.name ?? ""}
      open={selected !== null}
      onClose={close}
      styles={{ wrapper: { width: 560 } }}
    >
      <Empty description={t("tasksCenter.runs.detailUnavailable")} />
    </Drawer>
  );
}
