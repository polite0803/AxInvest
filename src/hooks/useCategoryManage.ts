// SPDX-License-Identifier: AGPL-3.0-only

import { useCategoryStore } from "@/stores/feature/categoryStore";

/**
 * categoryStore 的会话分类管理读面（CategoryManagerModal 专用，
 * 组件中立性消键，2026-11 #222 补洞批次）。
 *
 * 活源是 categoryStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * categoryStore（组件中立性），统一从这里取。原组件是无 selector 的整 store
 * 订阅（zustand 引用相等默认比较），此处保持同一订阅方式，返回形态不变。
 */
export function useCategoryManage() {
  return useCategoryStore();
}
