// SPDX-License-Identifier: AGPL-3.0-only

/**
 * **聊天消息侧的 schema 卡片插槽**（B8-1 实现半边；形态裁定见 `PLAN-shell-plus-plugins.md` §93；
 * 键迁移见任务 #216 前置 ①）。
 *
 * 职责只有一个：把「一张消息能带哪些卡片」这件事收成一个**可校验的契约**，然后把每一张交给
 * `PluginSchemaCard` 渲染。不认插件、不取数据（除注册表投影那一次只读 IPC）——
 * 消息里有什么规格就画什么。
 *
 * ── 契约：键走**卡片贡献注册表**（2026-11-07 迁移） ───────────────────────────────
 *
 * 载体仍是 `Message.meta?: Record<string, unknown>`（"Additional metadata for extensibility"），
 * 键仍是 `meta.pluginCards`，值是条目数组。**卡片的身份改由注册表键表达**：
 *
 * ```json
 * { "pluginCards": [{ "card": "stock:evolution-approvals", "dataContext": { "rows": [...] } }] }
 * ```
 *
 * - `card` = 卡片贡献注册表的宿主键 `${pluginId}:${id}`（`src/lib/dynamicUI/pluginCardRegistry.ts`
 *   的 `cardKey`）。键只能由宿主拼，`schemaId` 也由宿主按 `plugin.{pluginId}.{key}` 拼
 *   ⇒ 消息**写不出**指向别人 schema 的卡；未声明 / 未启用 / surface 不匹配 ⇒ 不渲染。
 * - 旧键 `{ schemaId: "..." }` 仍然受理，但只在该 id **能被注册表反查到某条 `chat` 贡献**时
 *   （`getContributionBySchemaId`）。裸 `schemaId` 一律丢弃：它没有归属，画的是「谁的 schema
 *   都行」——扩键后这条若继续放行，等于把零风险变成默认路径。
 *   ⚠ 迁移成本为零：本仓**零生产者**（插件产出入消息 meta 的通道还没建），
 *   旧写法的唯一消费方是单测固定装置。
 *
 * 归属为什么可以在前端判：注册表里的条目全部来自后端投影（`plugin_list_task_cards`）——
 * `pluginId` 是投影里的插件 id、`schemaId` 由宿主用 `pluginSchemaRowId` 拼，归属是**结构性**的，
 * 不需要再回查 schema 记录的 `ownerId`（那条路要求改 `PluginSchemaCard`，本轮不动它）。
 *
 * ── 为什么不抛错 ────────────────────────────────────────────────────────────
 *
 * 载荷来自消息（可能是旧版本插件、可能被手工改过）。一条坏条目只应让**它自己**不出现，
 * 不该让整条消息的渲染树炸掉：解析对每条独立校验，坏条目丢弃，其余照常渲染。
 */

import {
  ensureCardContributionsLoaded,
  getContributionByKey,
  getContributionBySchemaId,
  type PluginCardContribution,
  type PluginCardKey,
  useCardRegistryVersion,
} from "@/lib/dynamicUI/pluginCardRegistry";
import type { DynamicAction } from "@/types";
import { useEffect, useMemo } from "react";
import { PluginSchemaCard, type PluginSchemaCardSpec } from "./PluginSchemaCard";

/** `Message.meta` 里承载卡片规格的键。改名等于改契约，先找本文件的消费方。 */
export const PLUGIN_SCHEMA_CARDS_META_KEY = "pluginCards";

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * 消息里的一条卡片条目（**未解析归属**，解析在 `resolveChatCards`）。
 *
 * 两条键至少有一条：`card`（新，注册表宿主键）或 `schemaId`（旧，卡体行 id）。
 */
export interface PluginSchemaCardEntry {
  /** 新键：注册表宿主键 `${pluginId}:${id}`。 */
  card?: PluginCardKey;
  /** 旧键：动态 UI 行 id（`plugin.{pluginId}.{key}`）；仅当注册表能反查到才受理。 */
  schemaId?: string;
  dataContext?: Record<string, unknown>;
  title?: string;
}

/**
 * 把 `Message.meta` 规范成条目数组。
 *
 * 判据（逐条独立，坏条目只丢自己）：
 * - `meta` 不是对象 / 没有 `pluginCards` / 值不是数组 ⇒ `[]`（静默，等同「这条消息没有卡片」）；
 * - `card` 是非空字符串且含 `:` ⇒ 取用；否则看 `schemaId` 是否非空字符串；
 * - 两者都不成立 ⇒ 丢弃（**归属不明不画**，fail-closed）；
 * - `dataContext` 不是普通对象 ⇒ 置空（schema 里没有数据节点时本来也不需要它）；
 * - `title` 不是字符串 ⇒ 缺省（仅少一个标题，卡片照画）。
 */
export function parsePluginSchemaCards(
  meta: Record<string, unknown> | null | undefined,
): PluginSchemaCardEntry[] {
  if (!isPlainRecord(meta)) {
    return [];
  }
  const raw = meta[PLUGIN_SCHEMA_CARDS_META_KEY];
  if (!Array.isArray(raw)) {
    return [];
  }
  const entries: PluginSchemaCardEntry[] = [];
  for (const item of raw) {
    if (!isPlainRecord(item)) {
      continue;
    }
    const card = typeof item.card === "string" && item.card.includes(":")
      ? (item.card as PluginCardKey)
      : undefined;
    const schemaId = typeof item.schemaId === "string" && item.schemaId.length > 0
      ? item.schemaId
      : undefined;
    if (!card && !schemaId) {
      continue;
    }
    entries.push({
      card,
      schemaId,
      dataContext: isPlainRecord(item.dataContext) ? item.dataContext : undefined,
      title: typeof item.title === "string" && item.title.length > 0 ? item.title : undefined,
    });
  }
  return entries;
}

/** 已解析归属的聊天卡：卡片规格 + 贡献（贡献用于回流时带出 `pluginId`）。 */
export interface ResolvedChatCard {
  readonly contribution: PluginCardContribution;
  readonly spec: PluginSchemaCardSpec;
}

/**
 * 把条目解析成可渲染的卡片（归属由注册表证明），并**按注册表当前状态过滤**。
 *
 * 解析规则：
 * - `card` 键 ⇒ 必须命中注册表且 `surface === "chat"`；否则丢弃（未声明 / 已禁用 / 落位不符）。
 * - 旧键 `schemaId` ⇒ 必须能反查到 `surface === "chat"` 的贡献；否则丢弃（归属不明，fail-closed）。
 * - 贡献没有卡体（`schemaId` 缺省）⇒ 丢弃：聊天侧一张没有卡体的卡只是空白块。
 *
 * `title` 优先取消息里给的（那张卡在这一条消息里的具体标题），缺省用声明的卡标题；
 * `schemaId` 恒取**宿主拼的那份**（消息给的串只用于反查，不进渲染链）。
 */
export function resolveChatCards(entries: readonly PluginSchemaCardEntry[]): ResolvedChatCard[] {
  const resolved: ResolvedChatCard[] = [];
  for (const entry of entries) {
    const contribution = entry.card
      ? getContributionByKey(entry.card)
      : entry.schemaId
      ? getContributionBySchemaId(entry.schemaId)
      : undefined;
    if (!contribution || contribution.surface !== "chat" || !contribution.schemaId) {
      continue;
    }
    resolved.push({
      contribution,
      spec: {
        schemaId: contribution.schemaId,
        dataContext: entry.dataContext,
        title: entry.title ?? contribution.title,
      },
    });
  }
  return resolved;
}

export interface PluginSchemaCardSlotProps {
  /** 消息的 `Message.meta`；缺省或不合契约 ⇒ 不渲染任何东西。 */
  meta?: Record<string, unknown> | null;
  /** 动作回流钩子（可选）。不传则维持动态 UI 既有行为，插槽不自行决定由谁处理。 */
  onAction?: (action: DynamicAction, spec: PluginSchemaCardSpec) => void;
}

/**
 * 一条消息的卡片插槽。挂在 AI 消息内容之后（`ChatViewMessages.tsx` 的 ai-content-wrapper 之后）。
 *
 * 首次遇到带卡片的载荷时**幂等**拉一次卡片投影（`ensureCardContributionsLoaded`）；
 * 读失败按「没有插件卡」处理（不报错、不白屏），与任务中心对插件源的口径一致。
 */
export function PluginSchemaCardSlot({ meta, onAction }: PluginSchemaCardSlotProps) {
  const entries = useMemo(() => parsePluginSchemaCards(meta), [meta]);
  // 订阅注册表：投影异步到达后要重渲染，否则卡片永远停在「注册表里还没有」的空态。
  const registryVersion = useCardRegistryVersion();
  const cards = useMemo(
    () => resolveChatCards(entries),
    // registryVersion 是外置 store 的快照值：它变了说明注册表变了，必须重算。
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entries, registryVersion],
  );

  useEffect(() => {
    if (entries.length > 0) {
      void ensureCardContributionsLoaded();
    }
  }, [entries.length]);

  if (cards.length === 0) {
    return null;
  }
  return (
    <>
      {cards.map(({ contribution, spec }, index) => (
        <PluginSchemaCard
          key={`${contribution.key}-${index}`}
          spec={spec}
          onAction={onAction ? (action) => onAction(action, spec) : undefined}
        />
      ))}
    </>
  );
}
