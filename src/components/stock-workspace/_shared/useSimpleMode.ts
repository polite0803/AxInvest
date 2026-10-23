// SPDX-License-Identifier: AGPL-3.0-only

import { useUIStore, useWorkspaceStore } from "@/stores";

/**
 * 工作区是否按「简洁」密度呈现（决策条等紧凑形态）。
 * 移动端没有模式开关（开关只渲染在桌面 Header），按简洁处理。
 */
export function useSimpleMode(): boolean {
  const userMode = useWorkspaceStore((s) => s.userMode);
  const deviceLayout = useUIStore((s) => s.deviceLayout);
  return userMode === "simple" || deviceLayout === "mobile";
}

/**
 * 是否渲染专业次级区块（共识 / 实验轨迹 / 证据审计 / 调试工具）。
 *
 * 只在「能看到模式开关」的场景跟随用户选择：独立整页与移动端没有开关，
 * 必须恒展开 —— 否则这些内容既被藏起来、用户又无处切回。
 */
export function useShowAdvancedPanels(isInWorkspace: boolean): boolean {
  const userMode = useWorkspaceStore((s) => s.userMode);
  const deviceLayout = useUIStore((s) => s.deviceLayout);
  if (!isInWorkspace || deviceLayout === "mobile") { return true; }
  return userMode === "professional";
}
