// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 动作权限面的渲染树传递通道。
 *
 * `DynamicUIRenderer` 是「Schema 来源 → 动作权限」的唯一注入点：它在树根算出权限面并放进本
 * context，树内所有 `executeActions` 调用点（生命周期 onMount/onUnmount、事件绑定、表单提交）
 * 从这里取同一个对象。**不要**再给各组件加 `permissions` prop —— 递归渲染节点很多，
 * 逐个透传必然漏一处，而漏掉的那一处正是「动作照常发 IPC」的地方。
 *
 * `null` = 来源可信（用户 / Agent / 内置 / 内存态 Schema），维持改动前行为。
 */

import type { SkillPermissions } from "@/types";
import { createContext, useContext } from "react";

export const SchemaActionPermissionsContext = createContext<SkillPermissions | null>(null);

/** 读取当前渲染树继承的动作权限面；`undefined` 表示不受限。 */
export function useSchemaActionPermissions(): SkillPermissions | undefined {
  return useContext(SchemaActionPermissionsContext) ?? undefined;
}
