// SPDX-License-Identifier: AGPL-3.0-only

import { invoke, isTauri } from "@/lib/invoke";
import { resolvePluginForCommand } from "@/lib/pluginCommandResolve";

/**
 * 执行插件命名命令（T2 命令桥的前端出口）。
 *
 * 真机：按命令名反查「哪个启用中的插件声明了它」（不写死插件 id，见
 * `pluginCommandResolve.ts`）⇒ 宿主 `plugin_execute_command` 派发。该命令出参是
 * 命令回执的 **JSON 文本**（`Result<String, String>`），这里解析成 `T` 再交给调用方。
 * 插件未装 / 未启用时抛错，与取数失败同一路径处置，不新造错误码。
 *
 * 浏览器模式（`npm run dev`）：没有插件安装根、没有子进程，`plugin_list` 也没有插件
 * 注册表 ⇒ 直接把命令名派发给 `browserMock` 的同名处理器（mock 已按插件命名逐条登记，
 * 见 `browserMock.ts` 的「股票插件命名命令」区）。真机不走这条分支。
 */
export async function invokePluginCommand<T>(
  name: string,
  input?: Record<string, unknown>,
): Promise<T> {
  if (!isTauri()) {
    return invoke<T>(name, input ?? {});
  }
  const pluginId = await resolvePluginForCommand(name);
  if (!pluginId) {
    throw new Error(`no enabled plugin provides command \`${name}\``);
  }
  const raw = await invoke<string>("plugin_execute_command", {
    pluginId,
    name,
    input: input ?? {},
  });
  return JSON.parse(raw) as T;
}
