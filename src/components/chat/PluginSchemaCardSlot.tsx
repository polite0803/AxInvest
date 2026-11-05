// SPDX-License-Identifier: AGPL-3.0-only

/**
 * **聊天消息侧的 schema 卡片插槽**（B8-1 实现半边；形态裁定见 `PLAN-shell-plus-plugins.md` §93）。
 *
 * 职责只有一个：把「一张消息能带哪些卡片」这件事收成一个**可校验的契约**，然后把每一张交给
 * `PluginSchemaCard` 渲染。不认插件、不取数据、不碰网络 —— 消息里有什么规格就画什么。
 *
 * ── 契约落在哪个字段（为什么不新增 DTO） ────────────────────────────────────────
 *
 * 载体是 `Message.meta?: Record<string, unknown>`（`src/types/index.ts:228`，注释原文
 * "Additional metadata for extensibility"）—— 这就是本仓为「消息带附加结构化载荷」留的既有口子，
 * 消费端读它不需要给 `Message` 加字段，也就不需要动 `src/types/**`（B8-3 批在占该目录）。
 *
 * 键：`meta.pluginCards`，值是 `PluginSchemaCardSpec[]`：
 *
 * ```json
 * { "pluginCards": [{ "schemaId": "release-note-card", "title": "发行说明",
 *                     "dataContext": { "rows": [{ "name": "…", "qty": 3 }] } }] }
 * ```
 *
 * ⚠ 生产者侧今天还没有人写这个键（插件产出入消息 meta 的通道不在本批：本批只做「宿主能渲染」
 * 这一件事）。所以在没有生产者之前，插槽在真实消息上是**渲染出空**（返回 `null`，不占位、不报错），
 * 不是「白色一块」—— 判据见下面的 `parsePluginSchemaCards`。
 *
 * ── 为什么不抛错 ────────────────────────────────────────────────────────────
 *
 * 载荷来自消息（可能是旧版本插件、可能被手工改过）。一条坏条目只应让**它自己**不出现，
 * 不该让整条消息的渲染树炸掉：解析对每条独立校验，坏条目丢弃，其余照常渲染。
 */

import type { DynamicAction } from "@/types";
import { useMemo } from "react";
import { PluginSchemaCard, type PluginSchemaCardSpec } from "./PluginSchemaCard";

/** `Message.meta` 里承载卡片规格的键。改名等于改契约，先找本文件的消费方。 */
export const PLUGIN_SCHEMA_CARDS_META_KEY = "pluginCards";

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * 把 `Message.meta` 规范成卡片规格数组。
 *
 * 判据（逐条独立，坏条目只丢自己）：
 * - `meta` 不是对象 / 没有 `pluginCards` / 值不是数组 ⇒ `[]`（静默，等同「这条消息没有卡片」）；
 * - 条目不是对象、`schemaId` 不是非空字符串 ⇒ 丢弃；
 * - `dataContext` 不是普通对象 ⇒ 置空（schema 里没有数据节点时本来也不需要它）；
 * - `title` 不是字符串 ⇒ 缺省（仅少一个标题，卡片照画）。
 */
export function parsePluginSchemaCards(
  meta: Record<string, unknown> | null | undefined,
): PluginSchemaCardSpec[] {
  if (!isPlainRecord(meta)) {
    return [];
  }
  const raw = meta[PLUGIN_SCHEMA_CARDS_META_KEY];
  if (!Array.isArray(raw)) {
    return [];
  }
  const specs: PluginSchemaCardSpec[] = [];
  for (const item of raw) {
    if (!isPlainRecord(item)) {
      continue;
    }
    const schemaId = item.schemaId;
    if (typeof schemaId !== "string" || schemaId.length === 0) {
      continue;
    }
    specs.push({
      schemaId,
      dataContext: isPlainRecord(item.dataContext) ? item.dataContext : undefined,
      title: typeof item.title === "string" && item.title.length > 0 ? item.title : undefined,
    });
  }
  return specs;
}

export interface PluginSchemaCardSlotProps {
  /** 消息的 `Message.meta`；缺省或不合契约 ⇒ 不渲染任何东西。 */
  meta?: Record<string, unknown> | null;
  /** 动作回流钩子（可选）。不传则维持动态 UI 既有行为，插槽不自行决定由谁处理。 */
  onAction?: (action: DynamicAction, spec: PluginSchemaCardSpec) => void;
}

/**
 * 一条消息的卡片插槽。挂在 AI 消息内容之后（`ChatViewMessages.tsx` 的 ai-content-wrapper 之后）。
 */
export function PluginSchemaCardSlot({ meta, onAction }: PluginSchemaCardSlotProps) {
  const cards = useMemo(() => parsePluginSchemaCards(meta), [meta]);
  if (cards.length === 0) {
    return null;
  }
  return (
    <>
      {cards.map((spec, index) => (
        <PluginSchemaCard
          key={`${spec.schemaId}-${index}`}
          spec={spec}
          onAction={onAction ? (action) => onAction(action, spec) : undefined}
        />
      ))}
    </>
  );
}
