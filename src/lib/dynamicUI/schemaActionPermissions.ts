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
 * 来源**只取 `DynamicUISchemaRecord.origin`**（合法值集 `builtin | user | ai | plugin`）。
 * 该字段**不是请求方能左右的东西**：`create_dynamic_ui_schema` 在函数体里把 `origin` **硬写成
 * `"user"`**、`owner_id` 恒为空串（`src-tauri/src/commands/dynamic_ui.rs` 的 origin 锚，
 * 提交 `43803e5ac`），请求体的 `origin` / `ownerId` 两个入参被静默丢弃；插件来源的行只能由
 * 插件安装通道直接落库。
 * （此处原文写的是「后端 `resolve_origin` 写入」—— **该函数全仓零命中**，属文档幻觉，
 * 真实落点即上述硬写；当时正是这条幻觉让「origin 可自报」的缺口在被引用而非在被修复。）
 * `ownerId` 只在 `origin === "plugin"` 时非空，是归属标识，不参与「可信 / 不可信」判定。
 *
 * - 可信（`builtin` / `user` / `ai`）⇒ 返回 `undefined`，即**不注入权限面**，行为与改动前逐字一致。
 * - 其余一切（`plugin` / 空 / 未知字面量）⇒ 返回受限权限面。**判不出来源一律按不可信处理**：
 *   新增一种 origin 时若忘了在这里登记，得到的是「被收紧」而不是「被放行」。
 *
 * ## 权限面粒度
 *
 * 插件来源只开**四条只读 / 自身表单数据**命令（见 `PLUGIN_SCHEMA_ALLOWED_COMMANDS`），
 * **不再给 `dynamic_ui_*` 通配**。理由是一条完整的提权链：`create_dynamic_ui_schema`
 * 把 `origin` 硬写成 `"user"`、`owner_id` 恒为空（`commands/dynamic_ui.rs` 的 origin 锚），
 * 而 `"user"` 在本判据里属**可信来源** ⇒ 插件 Schema 只要声明一个 invoke 该命令的动作，
 * 就能造出一行「可信」Schema，那一行的动作再执行时拿到 `undefined` 权限面 = 完全不受限。
 * `update_dynamic_ui_schema` / `delete_dynamic_ui_schema` 当年的同一风险**后端已闭合**
 * （提交 `43803e5ac`）：两条命令都先过统一闸门 `load_schema_for_command` —— 目标行的 `origin`
 * 不在 `TRUSTED_SCHEMA_ORIGINS` 内即拒绝（`builtin` / `user` / `ai` 放行，`plugin` / 空串 /
 * 未知值一律拒），插件拥有的 Schema 只能由插件安装 / 卸载通道改删，内置行另由 `is_builtin` 挡。
 * ⇒ 原文「它们目前只挡 `builtin`，不按 `owner_id` 限定目标 ⇒ 插件来源的动作能改写或删除
 * 用户／别的插件的 Schema」已不成立。
 * ⚠ 后端**只按 origin 白名单、不看 `owner_id`** 是**有意的正确形态**，不是漏了归属校验：
 * 非插件行的 `owner_id` 恒为空串（`create` 硬写空），加它零区分度 ⇒ 后续批**请勿再来「补
 * owner_id 校验」**。本层仍对这三条写命令拒绝放行：前端权限面是第一道、后端闸门是第二道
 * （纵深防御），且 `create` 造出的行永远是可信的 `user` ⇒「插件自造不受限 Schema」那条路
 * 必须在前端就断掉。
 *
 * `navigate` / `events` / `storeRead` / `storeWrite` 全部**显式给空数组**：这些 executor
 * 的判据也是「有 permissions 就走白名单」，空数组 = 一律拒绝，与「不传 permissions」的
 * 放行语义相反，故必须显式列出而不是省略。`storeRead: []` 同时管住 dataSource 那条路
 * （`DataBindingEngine.assertDataSourceCoveredByPermissions`）—— 它原来只按
 * `dynamic_ui_` 前缀放行，`type: "store"` 的数据源等于直读任意已注册 store。
 *
 * ✅ 曾记为「已知残余（本层不解决）」的那条已由后端闭合（提交 `43803e5ac`）：**三条**
 * `*_form_data` 命令（save / get / delete —— 原文写「四条」是把白名单里的
 * `get_dynamic_ui_schema` 一起算了进去，后端实际只有三条）现在同样先过
 * `load_schema_for_command` ⇒ `schemaId` 指向插件拥有或并不存在的宿主即拒绝，
 * 插件不再能读写他人 Schema 的表单数据。当时的顾虑（后端不做归属校验）正是那次提交补的闸门。
 * ⚠ 仍存的读面残余（本层不管、后端有意放行）：`get_dynamic_ui_schema` 是**不限归属的只读投影**
 * ⇒ 任何来源都能读到他人 Schema 的定义，但**改不动**（写命令全部过闸）。
 */

import { useDynamicUIStore } from "@/stores/feature/dynamicUIStore";
import { TRUSTED_SCHEMA_ORIGINS } from "@/types";
import type { DynamicUISchemaRecord, SkillPermissions } from "@/types";

/**
 * 插件来源 Schema 唯一可用的命令面。
 *
 * 逐条列出、不给通配：新增一种 origin 或新增一条 `dynamic_ui_*` 命令时，得到的是
 * 「默认拒绝」而不是「默认放行」。表单数据三条是插件页面向宿主持久化自身状态的唯一通道，
 * `get_dynamic_ui_schema` 供页面按 id 取自身子 Schema。
 */
export const PLUGIN_SCHEMA_ALLOWED_COMMANDS: readonly string[] = [
  "get_dynamic_ui_schema",
  "get_dynamic_ui_form_data",
  "save_dynamic_ui_form_data",
  "delete_dynamic_ui_form_data",
];

/**
 * 可信来源：用户 / Agent / 内置。这三类维持改动前的行为（不注入权限面）。
 *
 * 词表只此一份，落在 `@/types` 的 `TRUSTED_SCHEMA_ORIGINS`（与后端 `commands/dynamic_ui.rs`
 * 同名常量同集合）：dev mock 的 update / delete 归属闸门读同一份。本文件原先自带一份字面量、
 * mock 另写一套判据，正是「mock 比真机宽松」那次分叉的形态 ⇒ 新增 origin 词汇只改一处。
 */
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
