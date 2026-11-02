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
 * 页面（asset bundle）粒度上这三条的独立证据在 `__tests__/pluginPageBridge.test.ts`：
 * 那里从**第三方 bundle 发出的一帧**起步，逐帧打到本模块，再断言 IPC 有没有发生。
 *
 * ⚠ 本模块只做「宿主侧执行」，不持有命令名与字段名的**字面量**（任务 #46 的收敛）：
 * 那两处住在 `@/sdk/pluginBridgeProtocol` 与 `@/sdk/sandboxFrameProtocol`，
 * 因为走 asset 协议装载的插件自带产物要带上同一份契约，却不能依赖本模块（本模块 import
 * `@/lib/invoke`，即 Tauri IPC）。同源由 `src/sdk/__tests__/sandboxFrameProtocol.test.ts` 机械校验。
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
import {
  isPluginBridgeCommand,
  omitHostBoundPluginId,
  PLUGIN_BRIDGE_COMMANDS,
  type PluginBridgeCommand,
} from "@/sdk/pluginBridgeProtocol";
import type { SkillHostApi, SkillPermissions } from "@/sdk/types";

/**
 * 插件沙箱仅有的两条通道。**不含通用 invoke 白名单** —— 插件内容能要求的后端能力到此为止，
 * 想多一条都得回来改这里（并且同步 `__tests__/pluginSandboxBridge.test.ts` 的不变量①）。
 *
 * ⚠ 这里是从 `@/sdk/pluginBridgeProtocol` **再导出同一个数组对象**，不是第二份清单：
 * 字面量住在 SDK 侧的唯一原因是依赖面 —— 第三方 bundle 不能 import 本模块（那会把
 * `@/lib/invoke` 与 Tauri API 拖进插件产物），但它必须与宿主用同一条命令集。
 * 「谁能动哪条命令」的答案仍然只有一份，改名/加名只能发生在协议模块里。
 */
export { isPluginBridgeCommand, PLUGIN_BRIDGE_COMMANDS };
export type { PluginBridgeCommand };

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
      // 不变量②：剥掉 iframe 传来的 pluginId（剥的逻辑与 bundle 侧同一条函数），再注入宿主绑定的归属。
      return invoke<T>(command, { ...omitHostBoundPluginId(args), pluginId });
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
