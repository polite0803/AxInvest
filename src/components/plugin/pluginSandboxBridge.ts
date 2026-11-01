// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件内容沙箱桥 —— 面板（panel）与页面（page）两种装载粒度**共用**的那一条桥。
 *
 * 为什么单独成模块（任务 #39 / `PLAN-shell-plus-plugins.md` §0.12.5 裁定 A）：
 * 插件贡献的前端内容运行在沙箱 iframe 里，它能向宿主索取什么后端能力，必须只有**一份**定义。
 * 这份定义原先写在 `PluginPanelHost.tsx` 内、只有 panel 粒度；页面粒度容器（走 asset 协议
 * 装载插件 bundle 那条路）落地时若照抄一份，就会出现两份命令清单 —— 而「谁能动哪条命令」
 * 正是插件化最不能出现两套口径的地方。抽到本模块后，panel 与 page 都 import 同一份白名单、
 * 走同一个桥工厂，**不存在第二套 IPC 桥**。
 *
 * 三条不变量由 `__tests__/pluginSandboxBridge.test.ts` 逐条钉住：
 * ① 命令集就是这两条，且 panel 与 page 用的是同一份（要加通道必须回来改这里）；
 * ② `pluginId` 由宿主绑定 —— iframe 传来的值一律丢弃，否则一个页面/面板能驱动另一个插件的 worker；
 * ③ 白名单外的命令当场拒绝，且**不发出 IPC**（不是"发了再被后端拒"）。
 *
 * 刻意不做的事：**不按 page/panel 拆权限子集**。两条命令都是「动作回流到本插件自己的 worker」
 * 这一种形态（`plugin_ui_action` 走 UI 动作、`plugin_execute_command` 走命名命令），面板与页面
 * 之间没有可机械区分的最小集；真正的收窄要靠后端的能力分级（`plugin_execute_command` 的
 * schema/权限语义正由另一会话改造），前端先拆子集只会在两边口径漂移时炸掉。
 *
 * 拒绝文案由调用方注入（`deny`）：panel 用 `dashboard.pluginPanels.commandDenied`，
 * 页面粒度容器将来用自己的措辞 —— 所以本模块不新增任何 i18n key。
 */

import { invoke } from "@/lib/invoke";
import type { SkillHostApi, SkillPermissions } from "@/sdk/types";

/**
 * 插件沙箱仅有的两条通道。**不含通用 invoke 白名单** —— 插件内容能要求的后端能力到此为止，
 * 想多一条都得回来改这里（并且同步 `__tests__/pluginSandboxBridge.test.ts` 的不变量①）。
 */
export const PLUGIN_BRIDGE_COMMANDS = ["plugin_ui_action", "plugin_execute_command"] as const;

/** 白名单里的命令字面量类型；调用方用它约束自己转发时的命令名。 */
export type PluginBridgeCommand = (typeof PLUGIN_BRIDGE_COMMANDS)[number];

/** 拒绝原因的构造函数（由宿主按粒度注入文案），入参是被拒绝的命令名。 */
export type BridgeDeniedTranslator = (command: string) => string;

/**
 * 沙箱侧的宿主 API。**必须是宿主自己构造**：`pluginId` 在这里绑定，
 * iframe 传来的同名字段一律丢弃（不变量②），否则跨插件越权。
 */
export function createPluginBridgeApi(
  pluginId: string,
  deny: BridgeDeniedTranslator,
): SkillHostApi {
  return {
    invoke: async <T = unknown>(command: string, args?: Record<string, unknown>): Promise<T> => {
      // 不变量③：先判定再发 IPC，顺序不可交换 —— 反过来写会让白名单外的命令打到后端。
      if (!isPluginBridgeCommand(command)) {
        throw new Error(deny(command));
      }
      const { pluginId: _ignored, ...rest } = args ?? {};
      return invoke<T>(command, { ...rest, pluginId });
    },
    // 沙箱不接宿主事件总线：`emit` 存在只是为了满足 SkillHostApi 形状。
    // 若将来要开事件，得先给它一条独立的白名单（events 面），不要顺手扩 commands。
    emit: (): void => {},
  };
}

/**
 * 装载沙箱时传给容器的权限面。**panel 与 page 共用本函数**（不变量①的落点）：
 * 页面粒度容器不得另写一份命令清单，否则「不扩白名单」这条无法机械保证。
 */
export function createPluginBridgePermissions(): SkillPermissions {
  return { commands: [...PLUGIN_BRIDGE_COMMANDS] };
}

/** 命令是否在桥的白名单内（供宿主在装载前做前置校验；判定与本模块的清单同源）。 */
export function isPluginBridgeCommand(command: string): command is PluginBridgeCommand {
  return (PLUGIN_BRIDGE_COMMANDS as readonly string[]).includes(command);
}
