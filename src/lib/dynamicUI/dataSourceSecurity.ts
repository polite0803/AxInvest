// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 动态 UI 数据源安全边界（共享模块）。
 *
 * 供 DataBindingEngine 运行时校验与 SchemaValidator 结构校验复用，
 * 避免白名单逻辑在多处重复定义造成漂移。
 */

/**
 * 允许从动态 UI 数据源调用的 Tauri 命令**显式枚举**白名单。
 *
 * 安全约束：dataSource.type === "api" 且 method === "invoke" 的 endpoint 来自 schema
 * （Agent 生成或用户导入，属不可信输入）。为避免构造任意 schema 绕过前端权限调用任意
 * 已注册 Tauri 命令（未鉴权 IPC），仅放行动态 UI 自身的命令。
 *
 * ⚠ 为什么是**逐条枚举**而不是前缀集合（#204 的成因）：本仓 Tauri 命令名一律**动词前置**
 * （`get_dynamic_ui_schema` / `list_dynamic_ui_schemas` / …），历史上这里写的是
 * `startsWith("dynamic_ui_")` —— 对本仓任何真实命令**恒为 false**，invoke 数据源在生产里
 * 一条都走不通。前缀这种形态本身就是这次缺陷的载体，故改为与命令清单同源、可机械核对的枚举。
 *
 * 权威来源：`src-tauri/src/commands/dynamic_ui.rs` 中全部 `#[tauri::command]` 标注的函数
 * （注册投影见 `src-tauri/src/register_commands.rs` 的 `commands::dynamic_ui::` 段，共 16 条）。
 * 后端新增/删除 dynamic_ui 命令时，必须同步修改本清单 —— 漏加的后果是「该命令在数据源里
 * 不可用」（fail-closed），不会放行未登记命令。
 *
 * ⚠ 这条**只是第一道**（对**所有来源**同视的「能不能被数据源触达」），含
 * `create/update/delete_dynamic_ui_schema` 三条写命令。插件等受限来源的第二道在
 * `DataBindingEngine.assertDataSourceCoveredByPermissions`，按该渲染树的权限面逐条判，
 * 判据见 `schemaActionPermissions.ts`（插件档只开 `get_dynamic_ui_schema` 一条）。
 * 改这一层不构成对插件的收口，改那一层才构成。
 */
export const ALLOWED_INVOKE_COMMANDS: readonly string[] = [
  "list_dynamic_ui_schemas",
  "get_dynamic_ui_schema",
  "create_dynamic_ui_schema",
  "update_dynamic_ui_schema",
  "delete_dynamic_ui_schema",
  "list_dynamic_ui_schema_versions",
  "get_dynamic_ui_schema_version",
  "restore_dynamic_ui_schema_version",
  "save_dynamic_ui_form_data",
  "get_dynamic_ui_form_data",
  "delete_dynamic_ui_form_data",
  "edit_dynamic_ui_schema_nl",
  "generate_dynamic_ui_schema_nl",
  "list_dynamic_ui_pins",
  "pin_dynamic_ui_schema",
  "unpin_dynamic_ui_schema",
];

const ALLOWED_INVOKE_COMMAND_SET = new Set(ALLOWED_INVOKE_COMMANDS);

/** 允许的 fetch 协议白名单（仅 http/https，禁止 file://, javascript:, data: 等）。 */
export const ALLOWED_FETCH_PROTOCOLS = ["https:", "http:"];

/**
 * fetch 数据源允许的 HTTP 方法。只读（GET）与提交表单（POST）两种真实用法；
 * PUT/PATCH/DELETE 等一律拒绝 —— 数据源语义里不存在「Schema 声明式改写远端资源」。
 */
export const ALLOWED_FETCH_METHODS: readonly string[] = ["GET", "POST"];

/**
 * fetch 数据源允许的请求头（小写比较）。只放行内容协商类头；
 * Authorization / Cookie / X-* 等一律拒绝 —— header 是第三方 Schema 可自由伪造的跨域凭据面。
 */
export const ALLOWED_FETCH_HEADER_NAMES: readonly string[] = ["content-type", "accept"];

/**
 * fetch `RequestInit` 允许出现的字段全集。`credentials` / `mode` / `redirect` / `signal` 等
 * 全部不在内：这些字段改变跨域凭据与重定向行为，不应由不可信 Schema 声明。
 */
const ALLOWED_FETCH_INIT_FIELDS: readonly string[] = ["method", "headers", "body"];

/** `toSafeFetchInit` 的拒绝分支返回值：错误键 + 插值参数，由调用方翻译后抛出。 */
export interface SafeFetchInitRejection {
  ok: false;
  messageKey: string;
  params: Record<string, unknown>;
}

/** `toSafeFetchInit` 的成功返回值：仅含白名单字段的干净 RequestInit。 */
export interface SafeFetchInitSuccess {
  ok: true;
  init: RequestInit;
}

/**
 * 把不可信 Schema 声明的 fetch `params` 收敛成最小 RequestInit（#206）。
 *
 * 历史缺陷：`:114` 曾把 Schema 原文整个当 `RequestInit` 透传给 fetch —— `method`、任意
 * `headers`（含 Authorization）、`body`、`credentials` 全由第三方声明。本函数是唯一出口：
 * - `params` 缺省 ⇒ `{}`（GET 无体）；
 * - 非普通对象（数组 / 标量）⇒ 拒；
 * - 出现白名单外字段 ⇒ 拒（逐字段，不静默剔除——静默剔除会让 Schema 作者误以为生效）；
 * - `method` 只允许 GET/POST（大小写不敏感，归一为大写）；
 * - `headers` 只允许内容协商类头名（大小写不敏感），值必须为字符串；
 * - `body` 必须为字符串，且只允许配合 POST。
 */
export function toSafeFetchInit(params: unknown): SafeFetchInitSuccess | SafeFetchInitRejection {
  if (params === undefined || params === null) {
    return { ok: true, init: {} };
  }
  if (typeof params !== "object" || Array.isArray(params)) {
    return { ok: false, messageKey: "dataBinding.fetchInitFieldForbidden", params: { field: "params" } };
  }
  const raw = params as Record<string, unknown>;

  for (const key of Object.keys(raw)) {
    if (!ALLOWED_FETCH_INIT_FIELDS.includes(key)) {
      return { ok: false, messageKey: "dataBinding.fetchInitFieldForbidden", params: { field: key } };
    }
  }

  const init: RequestInit = {};

  if (raw.method !== undefined) {
    if (typeof raw.method !== "string") {
      return { ok: false, messageKey: "dataBinding.fetchMethodForbidden", params: { method: String(raw.method) } };
    }
    const method = raw.method.toUpperCase();
    if (!ALLOWED_FETCH_METHODS.includes(method)) {
      return { ok: false, messageKey: "dataBinding.fetchMethodForbidden", params: { method } };
    }
    init.method = method;
  }

  if (raw.headers !== undefined) {
    if (typeof raw.headers !== "object" || raw.headers === null || Array.isArray(raw.headers)) {
      return { ok: false, messageKey: "dataBinding.fetchInitFieldForbidden", params: { field: "headers" } };
    }
    const headers: Record<string, string> = {};
    for (const [name, value] of Object.entries(raw.headers as Record<string, unknown>)) {
      if (!ALLOWED_FETCH_HEADER_NAMES.includes(name.toLowerCase()) || typeof value !== "string") {
        return { ok: false, messageKey: "dataBinding.fetchInitFieldForbidden", params: { field: `headers.${name}` } };
      }
      headers[name] = value;
    }
    init.headers = headers;
  }

  if (raw.body !== undefined) {
    // body 只配合 POST 存在：GET 带体在多数实现里被丢弃或报错，放行只会制造「以为生效」
    if (typeof raw.body !== "string" || init.method !== "POST") {
      return { ok: false, messageKey: "dataBinding.fetchInitFieldForbidden", params: { field: "body" } };
    }
    init.body = raw.body;
  }

  return { ok: true, init };
}

/**
 * 校验 Tauri 命令名是否在显式枚举白名单内。
 */
export function isAllowedInvokeEndpoint(endpoint: string): boolean {
  return ALLOWED_INVOKE_COMMAND_SET.has(endpoint);
}

/**
 * 校验 fetch URL 是否使用受控协议且可解析。
 */
export function isAllowedFetchUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return ALLOWED_FETCH_PROTOCOLS.includes(parsed.protocol);
  } catch {
    return false;
  }
}
