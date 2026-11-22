// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件桥协议 —— 沙箱内容（panel 的 `srcdoc` 与 page 的第三方 bundle）**两侧共用**的那一份契约。
 *
 * 定位（任务 #46）：`pluginSandboxBridge.ts` 是宿主**执行**白名单的地方，本模块是白名单与
 * 字段名**取值**的地方。分成两个模块的唯一理由是依赖面：
 * 宿主侧要 `@/lib/invoke`（Tauri IPC），而第三方 bundle 侧**绝不能**依赖它 ——
 * 那会把 Tauri API 拖进插件自己的产物里。于是「命令清单 + 宿主绑定字段 + 参数键名」
 * 这类两侧都要用的值必须住在一个零依赖模块里，否则 bundle 侧只能照抄一份字面串，
 * 抄错就是静默失效的桥（这正是本批要堵的缺口）。
 *
 * 三条不变量里与本模块有关的两条：
 * - **① 白名单就是这两条**：`PLUGIN_BRIDGE_COMMANDS` 是本文件里唯一写这两条命令名字面量的地方，
 *   宿主侧 `pluginSandboxBridge.ts` 的 `PLUGIN_BRIDGE_COMMANDS` 是从这里的**再导出**
 *   （同一个数组对象，不是第二份清单；`PluginPanelHost.ts` 的 panel 旧名也是同一个对象的别名）。
 *   要加第三条通道，只能改本文件那两个字面量，且必须同时改
 *   `src/components/plugin/__tests__/pluginSandboxBridge.test.ts` 与
 *   `src/components/plugin/__tests__/pluginPageBridge.test.ts` 的不变量①断言。
 * - **② `pluginId` 不可传**：{@link omitHostBoundPluginId} 是唯一实现，宿主绑定前与
 *   bundle 发出前各调一次同一函数 —— 不是两份「丢弃 pluginId」的逻辑。
 *
 * 刻意不做：不在本模块做任何权限判定或 IPC（那是宿主侧职责），不导出任何可绕过白名单的通用入口。
 *
 * @module sdk/pluginBridgeProtocol
 */

import { SANDBOX_RPC_METHODS } from "./sandboxFrameProtocol";

// ── 白名单（唯一字面量来源） ────────────────────────────────────────

/**
 * 插件沙箱仅有的两条通道，按用途命名。
 *
 * ⚠ 本对象是「谁能动哪条命令」的**唯一**答案。宿主侧的 `PLUGIN_BRIDGE_COMMANDS` 由
 * {@link PLUGIN_BRIDGE_COMMANDS} 派生，两侧同源 ⇒ 不可能出现「页面能调的面板调不到」
 * 或反之。想加通道请连同后端命令面、i18n 文案与两份不变量测试一起改，不要在本文件单独加。
 */
export const PLUGIN_BRIDGE_COMMAND_NAMES = {
  /** 动态 UI / 沙箱内容把用户动作回流给**该插件自己的** worker（后端 `plugin_ui_action`）。 */
  uiAction: "plugin_ui_action",
  /** 按名字调用该插件声明的命令（后端 `plugin_execute_command`）。 */
  executeCommand: "plugin_execute_command",
} as const;

/** 白名单命令名的联合类型（客户端 `invoke` 的参数类型 —— 类型层面就写不出别的命令）。 */
export type PluginBridgeCommand = (typeof PLUGIN_BRIDGE_COMMAND_NAMES)[keyof typeof PLUGIN_BRIDGE_COMMAND_NAMES];

/**
 * 白名单数组（**顺序即语义**：与沙箱容器 `permissions.commands` 的既有口径一致）。
 *
 * 由 {@link PLUGIN_BRIDGE_COMMAND_NAMES} 派生而非另写一遍，所以「数组里两条」与
 * 「名字表里两条」不可能各自漂移。宿主侧 `pluginSandboxBridge.ts` 再导出的就是这个对象。
 */
export const PLUGIN_BRIDGE_COMMANDS: readonly PluginBridgeCommand[] = [
  PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
  PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand,
] as const;

/**
 * 命令是否在插件桥白名单内。
 *
 * 与 {@link PLUGIN_BRIDGE_COMMANDS} 同源判定；宿主在装载前做前置校验用这一条，
 * 不在运行时另写 `includes` 字面量数组。
 */
export function isPluginBridgeCommand(command: string): command is PluginBridgeCommand {
  return (PLUGIN_BRIDGE_COMMANDS as readonly string[]).includes(command);
}

// ── 宿主绑定字段（不可传） ──────────────────────────────────────────

/** 由宿主绑定的字段名 —— 沙箱侧传任何值都必须被丢弃。 */
export const HOST_BOUND_PLUGIN_ID_FIELD = "pluginId";

/**
 * 沙箱侧可传的参数类型：任意键，**但 `pluginId` 不许出现**。
 *
 * `{ pluginId?: never }` 是 TS 表达「这个键不得出现」的惯用写法：调用点写
 * `{ pluginId: "other-pack" }` 直接编译报错（`string` 不能赋给 `never`），
 * 而运行时 {@link omitHostBoundPluginId} 再兜一次 —— 纯 JS 的第三方 bundle 没有编译期，
 * 只有运行期那一层拦得住它。两层都要，缺一层就留一个口子。
 */
export type PluginSandboxArgs = Record<string, unknown> & { [HOST_BOUND_PLUGIN_ID_FIELD]?: never };

/**
 * 丢弃沙箱侧传来的 `pluginId`，返回只剩业务参数的对象（**两侧唯一的实现**）。
 *
 * 用途不对称但函数同一个：
 * - 宿主侧（`pluginSandboxBridge.ts`）：先剥再注入宿主自己绑定的 `pluginId` ⇒ 外部无法改写归属；
 * - bundle 侧（`pluginClientBridge.ts`）：只剥不注入 ⇒ 帧里根本不出现该字段，
 *   少一个「插件以为自己能指定归属」的假接口。
 *
 * `args` 为 `undefined` 时返回空对象（与宿主既有 `args ?? {}` 行为逐字一致）。
 */
export function omitHostBoundPluginId(args?: Record<string, unknown>): Record<string, unknown> {
  if (!args) {
    return {};
  }
  const { [HOST_BOUND_PLUGIN_ID_FIELD]: _ignored, ...rest } = args;
  return rest;
}

// ── 命令的参数键名（与后端 DTO 对齐，非帧协议字段） ─────────────────

/**
 * `plugin_ui_action` 的业务参数键名 —— 对齐后端 `commands/plugin.rs::plugin_ui_action`
 * 的形参 `action`（Tauri v2 命令参数按 camelCase 校验，写错键名是 IPC 层拒绝）。
 *
 * ⚠ `plugin_execute_command` 的入参形状**不在此声明**：它的 schema / 权限语义沿
 * 「#30 命令通道改真授权 → #49 命令帧」继续演进（`PLAN-shell-plus-plugins.md` §16 还挂着它的
 * 后续 #59 / #60），这里写死键名就会两边口径漂移。客户端对它只做「剥掉 `pluginId`，其余原样透传」。
 * 任务号口径与 `pluginClientBridge.ts` 的 `executeCommand` 注释一致，同一件事不写两个号。
 */
export const PLUGIN_UI_ACTION_ARG_KEY = "action";

/**
 * bundle 客户端能调用的 RPC 方法集合（来自帧协议，不是新造的调用面）。
 *
 * 只有 `api.invoke` 一条：两条命令都经它转发，其余宿主方法（`ui.*` / `store.*` / `api.emit`）
 * 对第三方 bundle **不开放** —— 事件面需要独立的白名单（见 `pluginSandboxBridge.ts` 注释），
 * 宿主状态读写更是明确的不可越边界。本常量是对称差集测试的判据：客户端可调集合必须
 * 等于这里声明的集合，多一条即红。
 */
export const PLUGIN_BRIDGE_CLIENT_METHODS: readonly string[] = [SANDBOX_RPC_METHODS.apiInvoke] as const;
