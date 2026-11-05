// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 动态 UI Schema 的动作权限面解析（**按来源**判定，fail-closed）。
 *
 * ## 为什么需要这一层
 *
 * `dataSource` 那条路早就有白名单（`SchemaValidator` 用 `ALLOWED_INVOKE_PREFIXES` 校验
 * `config.endpoint`），但**声明式动作**（`events[].actions[].type === "invoke"`）走的是另一条路：
 * `EventHandlerEngine.executeActions` 构造 `ActionContext` 时若不传 `permissions`，
 * `ActionRouter` 的 invoke executor 会**跳过**白名单判定直接 `invoke()`（判据是
 * `if (ctx.permissions) {…}`）。插件 `schema.json` 收敛出来的 Schema（`origin = "plugin"`、
 * `ownerId = pluginId`）正是经这条渲染链执行 onMount / onClick 动作的
 * ⇒ 不注入权限面，就等于把「插件不进主 origin」这条裁定在声明式动作上作废。
 *
 * ## 判据（唯一来源字段）
 *
 * 来源**只取 `DynamicUISchemaRecord.origin`**（后端 `resolve_origin` 写入的合法值集：
 * `builtin | user | ai | plugin`）。`ownerId` 只在 `origin === "plugin"` 时非空，是归属标识，
 * 不参与「可信 / 不可信」判定。
 *
 * - 可信（`builtin` / `user` / `ai`）⇒ 返回 `undefined`，即**不注入权限面**，行为与改动前逐字一致。
 * - 其余一切（`plugin` / 空 / 未知字面量）⇒ 返回受限权限面。**判不出来源一律按不可信处理**：
 *   新增一种 origin 时若忘了在这里登记，得到的是「被收紧」而不是「被放行」。
 *
 * ## 权限面粒度
 *
 * 插件来源只开 `dynamic_ui_*`（与 `dataSourceSecurity.ALLOWED_INVOKE_PREFIXES` 同一条前缀，
 * 同一个理由：动作来自不可信 schema）。`navigate` / `events` / `storeRead` / `storeWrite` 全部
 * **显式给空数组**：这些 executor 的判据也是「有 permissions 就走白名单」，空数组 = 一律拒绝，
 * 与「不传 permissions」的放行语义相反，故必须显式列出而不是省略。
 *
 * ⚠ 已知残余（本层不解决，见交付报告）：`dynamic_ui_*` 里包含 `delete/update/create_dynamic_ui_schema`
 * 等写命令，而它们目前**不按 ownerId 限定目标**；要真正收窄得在后端做归属校验。
 */

import { useDynamicUIStore } from "@/stores/feature/dynamicUIStore";
import type { DynamicUISchemaRecord, SkillPermissions } from "@/types";

/** 插件来源 Schema 唯一可用的命令面（与 dataSource 白名单同粒度）。 */
export const PLUGIN_SCHEMA_ALLOWED_COMMANDS: readonly string[] = ["dynamic_ui_*"];

/** 可信来源：用户 / Agent / 内置。这三类维持改动前的行为（不注入权限面）。 */
const TRUSTED_SCHEMA_ORIGINS: ReadonlySet<string> = new Set(["builtin", "user", "ai"]);

export function isTrustedSchemaOrigin(origin: unknown): boolean {
  return typeof origin === "string" && TRUSTED_SCHEMA_ORIGINS.has(origin);
}

/**
 * 插件来源（含一切判不出来的来源）使用的受限权限面。
 * 每次返回新对象：`ActionContext.permissions` 会被下游读取，共享可变对象没有收益。
 */
export function createPluginSchemaPermissions(): SkillPermissions {
  return {
    commands: [...PLUGIN_SCHEMA_ALLOWED_COMMANDS],
    events: [],
    storeRead: [],
    storeWrite: [],
    navigate: [],
  };
}

/**
 * 由来源字面量解析动作权限面。
 * @returns `undefined` = 不受限（可信来源）；对象 = 受限权限面（插件 / 未知来源）
 */
export function permissionsForSchemaOrigin(origin: unknown): SkillPermissions | undefined {
  if (isTrustedSchemaOrigin(origin)) {
    return undefined;
  }
  return createPluginSchemaPermissions();
}

/**
 * 由 Schema 记录解析动作权限面。
 *
 * `record` 为空表示**非持久化**的 Schema（Agent 实时生成、可视化编辑器、预览），
 * 它们不来自插件 `schema.json`，维持现状。只要记录存在，来源判不出来就收紧。
 */
export function permissionsForSchemaRecord(
  record: Pick<DynamicUISchemaRecord, "origin"> | null | undefined,
): SkillPermissions | undefined {
  if (!record) {
    return undefined;
  }
  return permissionsForSchemaOrigin(record.origin);
}

/**
 * 渲染入口兜底：按 `schemaId` 从 store 里找已加载的记录并解析权限面。
 *
 * 覆盖「直接以路由打开插件 Schema」那条入口（`DynamicPageViewer` 拿到的 `pluginId`
 * 同样是从记录推出来的）—— 那条路不在本次可改的文件面内，故在这里按 id 兜住。
 * 只在 `currentSchema.id === schemaId` 或 `schemas` 列表命中时才判定：
 * id 不匹配一律当作「无记录」⇒ 不收紧（不会误伤内存态的用户 / Agent Schema）。
 */
export function resolveSchemaActionPermissions(
  schemaId: string | null | undefined,
): SkillPermissions | undefined {
  if (!schemaId) {
    return undefined;
  }
  const { currentSchema, schemas } = useDynamicUIStore.getState();
  const record = currentSchema?.id === schemaId
    ? currentSchema
    : schemas.find((s) => s.id === schemaId);
  return permissionsForSchemaRecord(record);
}
