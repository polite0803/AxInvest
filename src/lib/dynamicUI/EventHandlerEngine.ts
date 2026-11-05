// SPDX-License-Identifier: AGPL-3.0-only

import { getActionRouter, getDefaultNavigate } from "@/lib/actionRouter";
import type { ActionContext } from "@/lib/actionRouter";
import type { DynamicAction, EventHandler, SkillPermissions, UISchema } from "@/types";

/**
 * 事件处理引擎：解析 EventHandler 并执行 DynamicAction。
 *
 * 统一执行路径：所有 action 都通过 executeActions 执行，
 * - update-schema 在本模块内处理（带 scope 隔离）
 * - 其他类型委托给 ActionRouter
 * - 执行每个 action 后回调 onAction（若提供），方便父组件拦截/监听
 */

export interface ExecuteActionsOptions {
  /** 执行上下文数据（dataContext、pageParams 等） */
  context?: Record<string, unknown>;
  /** action 执行前/后的回调，父组件可用于拦截、监听、追加处理 */
  onAction?: (action: DynamicAction) => void;
  /** 用于隔离 schema-update 事件的 scope id（通常是 DynamicUIRenderer 的 rendererId） */
  scope?: string;
  /** 路由导航函数；未提供时回退到模块级默认导航器 */
  navigate?: (path: string) => void;
  /**
   * 来源受限时的动作权限面（插件来源 Schema 注入）。
   *
   * ⚠ 语义是「**有就收紧**」：`ActionRouter` 各 executor 的判据是 `if (ctx.permissions)` 才走白名单，
   * 省略即放行。故不可信来源必须传出与本选项一致的对象，不能靠缺省值表达「不允许」。
   * `undefined` = 不受限，仅表示来源可信（用户 / Agent / 技能），不是「忘了传」的借口。
   */
  permissions?: SkillPermissions;
}

/**
 * 执行一组 DynamicAction（顺序执行）。
 * 每个 action 先回调 onAction（供父组件感知），再实际执行。
 */
export async function executeActions(
  actions: DynamicAction[],
  options: ExecuteActionsOptions = {},
): Promise<void> {
  const { context, onAction, scope, navigate, permissions } = options;
  const router = getActionRouter();
  const actionCtx: ActionContext = {
    skillName: context?.skillName
      ? String(context.skillName)
      : "DynamicUI",
    pageParams: (context?.pageParams as Record<string, string>) || {},
    navigate: navigate ?? getDefaultNavigate() ?? undefined,
    permissions,
    // 来源受限标记：`permissions` 非空即代表「按来源收紧」（可信来源传的是 undefined），
    // handler 执行器据此拒绝借目标 skill 的 manifest 放宽（见 actionRouter 的同名字段）。
    sourceRestricted: permissions !== undefined,
  };

  for (const action of actions) {
    if (onAction) {
      onAction(action);
    }
    if (action.type === "update-schema") {
      // 来源受限时禁止跨 renderer 广播（`scope: "__global__"`）：
      // 注入别的渲染树等于让不可信 schema 借目标树的权限面执行自己的动作。
      await executeUpdateSchema(action, scope, Boolean(permissions));
    } else {
      await router.execute(
        {
          mode: "declarative",
          action: {
            type: action.type,
            ...action.config,
          } as Parameters<typeof router.execute>[0] extends { action: infer A } ? A
            : never,
        } as Parameters<typeof router.execute>[0],
        actionCtx,
      );
    }
  }
}

/**
 * 处理 update-schema 动作。
 * 通过全局 CustomEvent 通知 DynamicUIRenderer 更新 Schema。
 * 事件 detail 带上 scope（rendererId），接收方按 scope 过滤避免多实例串扰。
 *
 * config 格式：
 * { schemaId: string, operation: 'replace' | 'append' | 'remove', path?: string, newSchema?: UISchema, scope?: string }
 * 若 config.scope 为 "__global__"，则广播到所有 renderer。
 */
async function executeUpdateSchema(
  action: DynamicAction,
  currentScope: string | undefined,
  restricted = false,
): Promise<void> {
  const config = action.config as {
    schemaId: string;
    operation: "replace" | "append" | "remove";
    path?: string;
    newSchema?: UISchema;
    scope?: string;
  };

  // 受限来源只能改自己那棵树：显式 `__global__` 降级为当前 scope；
  // 连当前 scope 都没有（非 renderer 上下文）时直接丢弃，不退回广播。
  const targetScope = restricted
    ? (config.scope && config.scope !== "__global__" ? config.scope : currentScope)
    : (config.scope ?? currentScope ?? "__global__");
  if (!targetScope) {
    console.warn("[DynamicUI] 受限来源的 update-schema 缺少 scope，已丢弃");
    return;
  }

  window.dispatchEvent(
    new CustomEvent("dynamic-ui:schema-update", {
      detail: {
        schemaId: config.schemaId,
        operation: config.operation,
        path: config.path,
        newSchema: config.newSchema,
        scope: targetScope,
      },
    }),
  );
}

/**
 * 处理事件处理器数组，返回 React 事件绑定对象。
 * 返回 { triggerName: handlerFunction } 格式，可直接展开到组件 props。
 *
 * @param handlers - Schema 中定义的事件处理器
 * @param context - 数据上下文
 * @param onAction - action 执行回调（透传给 executeActions）
 * @param scope - renderer scope id
 * @param navigate - 路由导航函数
 * @param permissions - 来源受限时的动作权限面（插件来源注入；省略 = 不受限）
 */
export function handleEvents(
  handlers: EventHandler[],
  context?: Record<string, unknown>,
  onAction?: (action: DynamicAction) => void,
  scope?: string,
  navigate?: (path: string) => void,
  permissions?: SkillPermissions,
): Record<string, (...args: unknown[]) => void> {
  const bindings: Record<string, (...args: unknown[]) => void> = {};

  // 按 trigger 分组聚合 actions，避免同一 trigger 的多个 handler 相互覆盖丢失
  const grouped = new Map<string, DynamicAction[]>();
  for (const handler of handlers) {
    const trigger = handler.trigger;
    if (trigger === "onMount" || trigger === "onUnmount") {
      continue;
    }
    const list = grouped.get(trigger) ?? [];
    list.push(...handler.actions);
    grouped.set(trigger, list);
  }

  for (const [trigger, actions] of grouped) {
    bindings[trigger] = (...args: unknown[]) => {
      const eventContext = { ...context, _eventArgs: args };
      void executeActions(actions, { context: eventContext, onAction, scope, navigate, permissions });
    };
  }

  return bindings;
}

/**
 * 获取需要执行的 mount / unmount 处理器。
 */
export function getLifecycleHandlers(
  handlers: EventHandler[],
): {
  onMount: DynamicAction[];
  onUnmount: DynamicAction[];
} {
  const onMount: DynamicAction[] = [];
  const onUnmount: DynamicAction[] = [];

  for (const handler of handlers) {
    if (handler.trigger === "onMount") {
      onMount.push(...handler.actions);
    } else if (handler.trigger === "onUnmount") {
      onUnmount.push(...handler.actions);
    }
  }

  return { onMount, onUnmount };
}
