// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { invoke } from "@/lib/invoke";
import { isStoreReadCovered, isWildcardMatch } from "@/lib/skillPermissions";
import { useExecutionStore } from "@/stores/feature/executionStore";
import type { DataSourceConfig, SkillPermissions } from "@/types";
import { isAllowedFetchUrl, isAllowedInvokeEndpoint } from "./dataSourceSecurity";
import { getNestedValue } from "./utils";

/**
 * 数据绑定引擎：解析 DataSourceConfig 并返回实际数据。
 *
 * 支持四种数据源类型：
 * - store：读取 Zustand Store 数据（支持响应式订阅）
 * - api：调用 Tauri invoke 或 fetch
 * - static：直接返回静态数据
 * - agent-generated：从 Agent 生成数据中获取
 */

export interface DataSourceSubscriber {
  unsubscribe: () => void;
}

/**
 * 来源受限的渲染树在取 dataSource 前必须过的第二道闸。
 *
 * `dataSourceSecurity` 那道是按**前缀**对所有 Schema 放行的，而 `dynamic_ui_` 前缀里含
 * `create/update/delete_dynamic_ui_schema` 一族写命令；插件来源的权限面（`commands` /
 * `storeRead`）若不在这里执行，dataSource 这条路就等于绕过了动作闸 —— 与
 * `executeActions` 那条是同一个绕过面（`store` 类型直读任意已注册 store，
 * 包括存放密钥的 `providerStore`）。
 *
 * `permissions === undefined` 表示不受限（可信来源 / 内存态 Schema），行为逐字不变。
 */
function assertDataSourceCoveredByPermissions(
  config: DataSourceConfig,
  permissions: SkillPermissions | undefined,
): void {
  if (!permissions) {
    return;
  }
  if (config.type === "api") {
    const { endpoint, method } = config.config as {
      endpoint: string;
      method: "invoke" | "fetch";
    };
    if (
      method === "invoke"
      && !isWildcardMatch(String(endpoint), permissions.commands ?? [])
    ) {
      throw new Error(i18n.t("actionRouter.commandPermissionDenied", { command: String(endpoint) }));
    }
  }
  if (config.type === "store") {
    const { storeName, selector } = config.config as {
      storeName: string;
      selector?: string;
    };
    if (!isStoreReadCovered(String(storeName), selector, permissions.storeRead ?? [])) {
      throw new Error(i18n.t("actionRouter.storeReadPermissionDenied", {
        storeName: String(storeName),
        field: selector ? ` "${selector}"` : "",
      }));
    }
  }
}

/**
 * 解析数据源配置，返回实际数据（非 Hook 版本，用于一次性获取）。
 */
export async function resolveDataSource(
  config: DataSourceConfig,
  permissions?: SkillPermissions,
): Promise<unknown> {
  assertDataSourceCoveredByPermissions(config, permissions);
  switch (config.type) {
    case "static":
      return (config.config as Record<string, unknown>).value;

    case "store": {
      const { storeName, selector } = config.config as {
        storeName: string;
        selector?: string;
      };
      const { getStoreRegistry } = await import("@/lib/storeRegistry");
      const store = getStoreRegistry().get(storeName);
      if (!store) {
        throw new Error(i18n.t("dataBinding.storeNotRegistered", { storeName }));
      }
      const state = store.get() as Record<string, unknown>;
      if (selector) {
        return getNestedValue(state, selector);
      }
      return state;
    }

    case "api": {
      const { endpoint, method, params } = config.config as {
        endpoint: string;
        method: "invoke" | "fetch";
        params?: unknown;
      };
      if (method === "invoke") {
        if (typeof endpoint !== "string" || !isAllowedInvokeEndpoint(endpoint)) {
          throw new Error(i18n.t("dataBinding.apiEndpointForbidden", { endpoint: String(endpoint) }));
        }
        return invoke<unknown>(endpoint, params as Record<string, unknown>);
      }
      // fetch 模式：仅放行受控协议的 http/https URL，防止 SSRF / file:// 等
      if (typeof endpoint !== "string" || !isAllowedFetchUrl(endpoint)) {
        throw new Error(i18n.t("dataBinding.apiEndpointForbidden", { endpoint: String(endpoint) }));
      }
      const response = await fetch(endpoint, params as RequestInit);
      if (!response.ok) {
        throw new Error(i18n.t("dataBinding.apiRequestFailed", { statusText: response.statusText }));
      }
      return response.json();
    }

    case "agent-generated": {
      const { generationId, selector } = config.config as {
        generationId: string;
        selector?: string;
      };
      const executionState = useExecutionStore.getState();
      const generation =
        (executionState as unknown as Record<string, unknown>) /* SAFE: dynamic UI data binding from execution state */[
          generationId
        ];
      if (!generation) {
        throw new Error(
          i18n.t("dataBinding.agentDataNotFound", { generationId }),
        );
      }
      if (selector) {
        return getNestedValue(generation as Record<string, unknown>, selector);
      }
      return generation;
    }

    default:
      throw new Error(i18n.t("dataBinding.unknownSourceType", { type: config.type }));
  }
}

/**
 * 订阅数据源变化，支持store响应式和轮询。
 * @param config 数据源配置
 * @param onData 数据更新回调
 * @returns 取消订阅函数
 */
export async function subscribeDataSource(
  config: DataSourceConfig,
  onData: (data: unknown) => void,
  onError?: (error: Error) => void,
  permissions?: SkillPermissions,
): Promise<DataSourceSubscriber> {
  let cancelled = false;

  // 一次判定覆盖下面全部三条出口：初始加载、轮询、以及 store.subscribe 那条**绕过
  // resolveDataSource 的直订阅** —— 初始加载失败时下面的 catch 只回调 onError，流程仍会
  // 继续往下走，所以拦必须发生在进订阅之前。
  //
  // 判拒走 onError 而非抛出：调用方（`DynamicUIRenderer` 的 `.then(...)`）没有 `.catch`，
  // 抛出等于一个未处理拒绝，且订阅方拿不到 subscriber 句柄。
  try {
    assertDataSourceCoveredByPermissions(config, permissions);
  } catch (err) {
    if (!cancelled && onError) {
      onError(err instanceof Error ? err : new Error(String(err)));
    }
    return {
      unsubscribe: () => {
        cancelled = true;
      },
    };
  }

  // 先执行一次初始加载
  try {
    const initialData = await resolveDataSource(config, permissions);
    if (!cancelled) {
      onData(initialData);
    }
  } catch (err) {
    if (!cancelled && onError) {
      onError(err instanceof Error ? err : new Error(String(err)));
    }
  }

  const cleanupFns: Array<() => void> = [];

  // 处理轮询
  if (config.polling && config.polling > 0) {
    const timer = window.setInterval(async () => {
      if (cancelled) { return; }
      try {
        const data = await resolveDataSource(config, permissions);
        if (!cancelled) { onData(data); }
      } catch (err) {
        if (!cancelled && onError) {
          onError(err instanceof Error ? err : new Error(String(err)));
        }
      }
    }, config.polling);
    cleanupFns.push(() => window.clearInterval(timer));
  }

  // Store 类型数据源 — 通过 StoreRegistry 获取 store 并执行响应式订阅
  // 如果已配置轮询，则跳过 store 订阅避免双重回调
  if (config.type === "store" && !(config.polling && config.polling > 0)) {
    const { storeName, selector } = config.config as {
      storeName: string;
      selector?: string;
    };
    const { getStoreRegistry } = await import("@/lib/storeRegistry");
    const store = getStoreRegistry().get(storeName);
    if (store && typeof store.subscribe === "function") {
      const unsubscribe = store.subscribe(() => {
        if (cancelled) { return; }
        const state = store.get() as Record<string, unknown>;
        const data = selector ? getNestedValue(state, selector) : state;
        onData(data);
      });
      cleanupFns.push(unsubscribe);
    }
  }

  return {
    unsubscribe: () => {
      cancelled = true;
      for (const fn of cleanupFns) {
        fn();
      }
    },
  };
}
