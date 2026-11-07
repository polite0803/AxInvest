// SPDX-License-Identifier: AGPL-3.0-only

import { useBackupStore } from "@/stores/feature/backupStore";

/**
 * 标题栏备份入口的备份配置读面（自 TitleBar 抽出，组件中立性消键）：
 * 自动备份设置 + 拉取 action。
 *
 * 保持原组件的无 selector 全量订阅形态（store 任意字段变化都重渲染），
 * 重渲染时机与原实现逐字一致，不收窄订阅面。
 */
export function useTitleBarBackup() {
  const { backupSettings, loadBackupSettings } = useBackupStore();
  return { backupSettings, loadBackupSettings };
}
