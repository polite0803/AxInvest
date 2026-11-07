// SPDX-License-Identifier: AGPL-3.0-only

import { useExecutionStore } from "@/stores/feature/executionStore";

const EMPTY: never[] = [];

/**
 * 指定会话的轨迹列表、加载态、详情缓存及拉取动作。
 *
 * 防竞态语义（调用方自行维护 loadingId 比对）与原组件实现保持一致，
 * 本 hook 只做切片透传。
 *
 * 活源是 executionStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * executionStore（组件中立性），统一从这里取。
 */
export function useTrajectories(conversationId: string) {
  const trajectories = useExecutionStore(
    (s) => s.trajectoriesByConversation[conversationId] || EMPTY,
  );
  const loadingList = useExecutionStore((s) => s.loadingTrajectories);
  const details = useExecutionStore((s) => s.trajectoryDetails);
  const fetchList = useExecutionStore((s) => s.fetchTrajectoryList);
  const fetchDetail = useExecutionStore((s) => s.fetchTrajectoryDetail);

  return { trajectories, loadingList, details, fetchList, fetchDetail };
}
