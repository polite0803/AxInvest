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
// ⚠ 入参纪律（2026-11-17 红灯订正）：三条带参命令的入参**只从第一轮真实回执级联提取**，
// 提取不到就渲染「未发调用：原因」，绝不发空串占位——`period: ""` 对宿主 Period 枚举
// 必然反序列化失败，`generatedAt: ""` 精确等值查询恒不命中，两者都会被「调用失败/空态」
// 文案掩盖成伪装空态。
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

/** 单条命令的结局：真实回执 / 调用失败 / 未发调用。三种必须分列渲染，不得互相掩盖。 */
type Outcome =
  | { kind: "receipt"; raw: unknown }
  | { kind: "error"; message: string }
  | { kind: "skipped"; reason: string };

/** 第一轮：无依赖命令。list_reco_history 与 get_reco_strategy_weights 是级联三条的参数源。 */
const BASE_CALLS: readonly StockCall[] = [
  { name: "stock_reflection_stats" },
  { name: "stock_reco_ic_stats", input: {} },
  { name: "stock_get_reco_strategy_weights" },
  { name: "stock_get_market_status" },
  { name: "stock_get_vendor_fallback_log" },
  { name: "stock_list_reco_history", input: { limit: 20, offset: 0 } },
  { name: "stock_list_stock_tools" },
];

/**
 * 档位变体清单。权威来源：`axagent-core/crates/harness/src/holding_period.rs` 的 Period
 * 枚举（`#[serde(rename_all = "lowercase")]`，四变体）——本页无法 import 该定义，此处
 * 内联必须与其逐字一致，漂移由宿主报参数解析失败暴露。
 * **按长度降序排列**：`_short` 是 `_ultra_short` 的尾后缀，切分必须先长后短。
 */
const PERIOD_VARIANTS = ["ultra_short", "short", "long", "mid"] as const;

interface RecoRow {
  generatedAt?: unknown;
  period?: unknown;
  [key: string]: unknown;
}

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

/** 从 stock_list_reco_history 回执提取行数组（行 DTO camelCase：generatedAt/period/…）。 */
function parseRecoRows(raw: unknown): RecoRow[] {
  const value = parseReceipt(raw);
  if (!Array.isArray(value)) {
    return [];
  }
  return value.filter((row): row is RecoRow => row !== null && typeof row === "object");
}

/**
 * 从 stock_get_reco_strategy_weights 回执键切出首个策略 id。
 * 权重键形态是 `{strategy}_{period}`（权威：analysis-engine reco_loop.rs:1233，
 * 如 `trend_mid` / `capital_long`）。无 `{strategy}_{period}` 形态键 ⇒ null。
 */
function extractStrategyId(raw: unknown): string | null {
  const value = parseReceipt(raw);
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  for (const key of Object.keys(value as Record<string, unknown>)) {
    for (const period of PERIOD_VARIANTS) {
      const suffix = `_${period}`;
      if (key.endsWith(suffix) && key.length > suffix.length) {
        return key.slice(0, key.length - suffix.length);
      }
    }
  }
  return null;
}

/** 级联条目的结局：skip=null 表示参数已解析可执行；否则渲染 skip 原因（不发调用）。 */
type ResolvedDependent = { call: StockCall; skip: string | null };

/**
 * 级联三条命令：入参全部来自第一轮真实回执；拿不到真实值 ⇒ skip（不发空串）。
 * 语义核对（宿主侧实读）：
 * - stock_get_reco_detail：generated_at 精确等值查询（reco_history.rs get_reco_detail_core）；
 * - stock_get_cached_recommendation：period 反序列化为 Period 枚举（holding_period.rs lowercase）；
 * - stock_get_reco_signal_history：strategy_id 透传 run_signal_history（stock_analysis.rs signal_history_core）。
 */
function resolveDependentCalls(
  firstRound: ReadonlyMap<string, unknown>,
): readonly ResolvedDependent[] {
  const rows = parseRecoRows(firstRound.get("stock_list_reco_history"));
  const detailRow = rows.find(
    (row) => typeof row.generatedAt === "string" && row.generatedAt !== "",
  );
  const cachedRow = rows.find(
    (row) =>
      typeof row.period === "string"
      && (PERIOD_VARIANTS as readonly string[]).includes(row.period),
  );
  const strategyId = extractStrategyId(firstRound.get("stock_get_reco_strategy_weights"));

  return [
    detailRow
      ? {
        call: { name: "stock_get_reco_detail", input: { generatedAt: detailRow.generatedAt as string } },
        skip: null,
      }
      : {
        call: { name: "stock_get_reco_detail" },
        skip: "库中暂无荐股记录（缺 generatedAt），未发调用",
      },
    cachedRow
      ? {
        call: { name: "stock_get_cached_recommendation", input: { period: cachedRow.period as string } },
        skip: null,
      }
      : {
        call: { name: "stock_get_cached_recommendation" },
        skip: "库中暂无带合法档位（period）的荐股记录，未发调用",
      },
    strategyId
      ? {
        call: { name: "stock_get_reco_signal_history", input: { strategyId } },
        skip: null,
      }
      : {
        call: { name: "stock_get_reco_signal_history" },
        skip: "权重台账为空（无 {strategy}_{period} 键），无法确定策略 id，未发调用",
      },
  ];
}

/** 单条命令执行：宿主回执原样返回；错误收敛成 message。 */
async function invokeOne(call: StockCall): Promise<Outcome> {
  try {
    const raw = await client.executeCommand({
      name: call.name,
      ...(call.input === undefined ? {} : { input: call.input }),
    });
    return { kind: "receipt", raw };
  } catch (error) {
    return { kind: "error", message: error instanceof Error ? error.message : String(error) };
  }
}

/** 渲染单条命令的结局到 root：回执 / 空态 / 失败 / 未发调用，四态分列。 */
function renderSection(root: HTMLElement, name: string, outcome: Outcome): void {
  const section = document.createElement("section");
  section.style.cssText = "margin:0 0 12px;padding:8px 10px;border:1px solid #ddd;border-radius:6px";

  const title = document.createElement("h2");
  title.style.cssText = "margin:0 0 6px;font-size:13px;font-weight:600";
  title.textContent = name;
  section.appendChild(title);

  const pre = document.createElement("pre");
  pre.style.cssText = "margin:0;white-space:pre-wrap;word-break:break-all;font:12px/1.5 ui-monospace,monospace";
  if (outcome.kind === "error") {
    pre.textContent = `调用失败：${outcome.message}`;
  } else if (outcome.kind === "skipped") {
    pre.textContent = `未发调用：${outcome.reason}`;
  } else {
    const value = parseReceipt(outcome.raw);
    pre.textContent = isEmpty(value) ? EMPTY_TEXT : JSON.stringify(value, null, 2);
  }
  section.appendChild(pre);

  root.appendChild(section);
}

/** 页面主流程：第一轮七条独立命令 → 级联三条（参数来自第一轮回执，缺参显式跳过）。 */
async function main(): Promise<void> {
  const root = document.getElementById("root");
  if (!root) {
    return;
  }

  const firstRound = new Map<string, unknown>();
  for (const call of BASE_CALLS) {
    const outcome = await invokeOne(call);
    if (outcome.kind === "receipt") {
      firstRound.set(call.name, outcome.raw);
    }
    renderSection(root, call.name, outcome);
  }

  for (const dep of resolveDependentCalls(firstRound)) {
    const outcome: Outcome = dep.skip === null
      ? await invokeOne(dep.call)
      : { kind: "skipped", reason: dep.skip };
    renderSection(root, dep.call.name, outcome);
  }
}

// 顶层触发渲染（main 内部每条命令各自兜底，整体不会被单条失败打断）。
void main();

// 卸载钩子：解绑监听并拒绝在途调用，避免帧回调打在已卸载的产物上。
window.addEventListener("beforeunload", () => client.dispose());
