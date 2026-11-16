// SPDX-License-Identifier: AGPL-3.0-only
//
// 股票插件 bundle 页面入口（B2 垂直切片）。
//
// 协议帧 / 命令白名单 / ready / dispose 全部来自 vendored 桥（../vendor/sdk/），
// 本文件只做两件事：把十条 stock_* 命令经桥投递给宿主、把回执渲染成 DOM。
// 任何协议帧字面量都不得在这里重写（那会成为协议的第二份副本）。
//
// ⚠ 通道口径：vendored 白名单只有 `plugin_ui_action` / `plugin_execute_command` 两条
// （vendor/sdk/pluginBridgeProtocol.ts），所以十条 stock_* 命令必须走
// `client.executeCommand({ name, input })`，由宿主的 plugin_execute_command 再按 name
// 分发到插件 worker 的 COMMAND_* 臂。绝不能 `client.invoke("stock_*")`——那会在
// client 侧白名单被拒且一帧不发（vendor/sdk/pluginClientBridge.ts 的 invoke 实现）。
//
// 宿主约束（PluginPageHost）：bundle 入口必须顶层创建客户端 + 顶层 announceReady()，
// 否则容器停在 loading 直到 skill.timeoutError 白屏；卸载必须 dispose()。

import { createAxAgentPluginClient } from "../vendor/sdk/pluginClientBridge";

// 顶层求值即创建 + 就绪：不塞进 DOMContentLoaded 回调里延迟。
const client = createAxAgentPluginClient();
client.announceReady();

interface StockCall {
  /** 命令名（宿主 plugin_execute_command 的 name 形参）。 */
  name: string;
  /** 业务入参（宿主 plugin_execute_command 的 input 形参）。 */
  input?: Record<string, unknown>;
}

/**
 * 十条命令的定义。入参形状以插件 worker 的分派臂为准（main.rs）：
 * 需要针对性入参的三条给出代表值，其余无参命令不传 input（宿主按 {} 处理）。
 */
const CALLS: readonly StockCall[] = [
  { name: "stock_reflection_stats" },
  { name: "stock_reco_ic_stats", input: {} },
  { name: "stock_get_reco_strategy_weights" },
  { name: "stock_get_market_status" },
  { name: "stock_get_vendor_fallback_log" },
  { name: "stock_list_reco_history", input: { limit: 20, offset: 0 } },
  { name: "stock_get_reco_detail", input: { generatedAt: "", styleFilter: "" } },
  { name: "stock_get_reco_signal_history", input: { strategyId: "" } },
  { name: "stock_get_cached_recommendation", input: { period: "" } },
  { name: "stock_list_stock_tools" },
];

/** 空数据（null / undefined / 空数组 / 空对象）的显式文案——不伪装成空白或加载态。 */
const EMPTY_TEXT = "本环境不提供该数据";

/** 判断是否为「无数据」：null / undefined / 空数组 / 空对象。 */
function isEmpty(value: unknown): boolean {
  if (value === null || value === undefined) {
    return true;
  }
  if (Array.isArray(value)) {
    return value.length === 0;
  }
  if (typeof value === "object") {
    return Object.keys(value as Record<string, unknown>).length === 0;
  }
  return false;
}

/**
 * 宿主 plugin_execute_command 返回的是 JSON 文本字符串（plugin.rs 的 Result<String,String>）。
 * 尽力 JSON.parse 成结构化值再美化；解析不了就按原样字符串展示（真机 / browser-mock 都覆盖）。
 */
function parseReceipt(raw: unknown): unknown {
  if (typeof raw !== "string") {
    return raw;
  }
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return raw;
  }
}

/** 渲染单条命令的结果到 root。 */
function renderSection(root: HTMLElement, name: string, raw: unknown): void {
  const section = document.createElement("section");
  section.style.cssText = "margin:0 0 12px;padding:8px 10px;border:1px solid #ddd;border-radius:6px";

  const title = document.createElement("h2");
  title.style.cssText = "margin:0 0 6px;font-size:13px;font-weight:600";
  title.textContent = name;
  section.appendChild(title);

  const value = parseReceipt(raw);
  const pre = document.createElement("pre");
  pre.style.cssText = "margin:0;white-space:pre-wrap;word-break:break-all;font:12px/1.5 ui-monospace,monospace";
  pre.textContent = isEmpty(value) ? EMPTY_TEXT : JSON.stringify(value, null, 2);
  section.appendChild(pre);

  root.appendChild(section);
}

/** 页面主流程：逐条调用、各自 try/catch，单条失败不拖垮整页。 */
async function main(): Promise<void> {
  const root = document.getElementById("root");
  if (!root) {
    return;
  }

  for (const call of CALLS) {
    try {
      const raw = await client.executeCommand({
        name: call.name,
        ...(call.input === undefined ? {} : { input: call.input }),
      });
      renderSection(root, call.name, raw);
    } catch (error) {
      // 调用失败也如实渲染（错误对象不可 JSON 化，直接取 message）。
      renderSection(root, call.name, `调用失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }
}

// 顶层触发渲染（main 内部每条命令各自兜底，整体不会被单条失败打断）。
void main();

// 卸载钩子：解绑监听并拒绝在途调用，避免帧回调打在已卸载的产物上。
window.addEventListener("beforeunload", () => client.dispose());
