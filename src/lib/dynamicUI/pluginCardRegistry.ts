// SPDX-License-Identifier: AGPL-3.0-only

/**
 * **插件卡片贡献注册表**（任务 #216 前置 ①）。
 *
 * ## 它解决什么（以及在此之前盘上有什么）
 *
 * 卡片此前只有两块互不相干的半机制：聊天侧的 `PluginSchemaCardSlot`（自由字符串 `schemaId`，
 * 既没有归属也没有 slot）与任务中心的 `adapters/registry.ts`（键是**任务源** `SourceId`，
 * 装不下「卡」：源可有多个 run，卡没有 run 行）。⇒ 真问题不是「键口径太窄」，
 * 而是**根本没有键的类型定义**。本文件补的就是这一层：
 *
 * - 键的类型：`PluginCardKey`（`${pluginId}:${id}`，只由宿主拼，插件写不出别人的前缀）；
 * - 落位：`surface` —— 词表**只开两个**（`chat` / `task_card`，2026-11-07 leader 裁定 1），
 *   与后端 `PluginCardSurface` 逐字同集；
 * - 归属：`pluginId` 来自**声明它的插件**（后端投影 `plugin_list_task_cards` 的 `id`），
 *   不由卡片数据自报 ⇒ 消息里写 `{card: "other:x"}` 只会命不中（未声明 ⇒ 不渲染）。
 *
 * ## 三条口径（照抄 `TasksCenterPage/adapters/registry.ts` 的既有裁定，不另立）
 *
 * 1. **只挂 `enabled` 的插件**：禁用插件的卡挂上去只会多一张永远空的卡。
 * 2. **先到先得 + warn**：同一键重复注册时**不覆盖**（leader 裁定 3）。宿主内置卡在模块加载期
 *    先占位 ⇒ 插件抢同名必然失败（宿主优先）；跨插件撞键在 `${pluginId}:${id}` 下本不可能，
 *    但跨插件指向**同一个 schemaId** 是合法的（两张卡共用一张卡体），故 `BY_SCHEMA` 只做首条映射。
 * 3. **不缓存第二份清单**：真相源恒为 `PluginManager` 的投影（禁区 4）。本模块只是
 *    「投影 → 宿主键」的只读索引，随插件启停/重装重新注册即可。
 *
 * ## 写权限不在这里判
 *
 * `actions[].mutating = true` 要求清单 `permissions[]` 含 `write`，这条闸在**后端装载期**
 * （`build_manifest_task_cards` 的 `TaskCardActionWritePermissionMissing`）——能投影出来的
 * 声明里不存在越权动作。前端**不要**再写一遍判定：`PluginSummaryDto` 不带 `permissions`，
 * 判也判不准，且两处判定漂移后会变成「装了却不显示」。
 */

import { invoke } from "@/lib/invoke";
import { pluginSchemaRowId } from "@/lib/pluginUiSchema";
import type { PluginCardSurfaceDto, PluginTaskCardActionDto, PluginTaskCardsInfoDto } from "@/types";
import { useSyncExternalStore } from "react";

/**
 * 注册表键：`${pluginId}:${id}`。
 *
 * 由宿主拼（[`cardKey`]），插件只声明自己的 `id` ⇒ 「跨插件抢键」这种写法在声明面上不存在。
 * 键里带 `:` 而 `id` 形状判据（后端 `is_declared_key_shape`）不允许 `:`，因此键可无损拆回两段。
 */
export type PluginCardKey = `${string}:${string}`;

/** 一条卡片贡献（宿主视角，字段全部由后端投影 + 宿主拼接得出）。 */
export interface PluginCardContribution {
  /** 宿主键，见 [`PluginCardKey`]。 */
  readonly key: PluginCardKey;
  /** 落位，见 [`PluginCardSurfaceDto`]。 */
  readonly surface: PluginCardSurfaceDto;
  /** 声明这张卡的插件 id（归属锚，不由卡片数据自报）。 */
  readonly pluginId: string;
  /** 卡片 id（插件内唯一，后端装载期已判）。 */
  readonly id: string;
  /** 显示标题（插件自带文案，不走宿主 i18n）。 */
  readonly title: string;
  /** 卡体 schema 的**声明 key**（本插件 `ui.json` 的 key）；缺省 ⇒ 纯计数卡。 */
  readonly detailSchema?: string;
  /**
   * 卡体 schema 的**行 id**（`plugin.{pluginId}.{key}`，宿主用 [`pluginSchemaRowId`] 拼）。
   * 缺省与 [`detailSchema`] 同时缺省。
   */
  readonly schemaId?: string;
  /** 计数 / 取数命令（本包 `commands[]` 里的名字）。 */
  readonly summaryCommand: string;
  readonly tone?: string;
  readonly order: number;
  /** 卡片动作（写权限闸已在后端装载期做过，这里只是透出声明）。 */
  readonly actions: readonly PluginTaskCardActionDto[];
}

const BY_KEY = new Map<PluginCardKey, PluginCardContribution>();
/** schemaId → 首条贡献：聊天侧旧键 `schemaId` 迁移到宿主键时用它证明「这是插件卡」。 */
const BY_SCHEMA_ID = new Map<string, PluginCardContribution>();
const BY_SURFACE = new Map<PluginCardSurfaceDto, PluginCardContribution[]>();

/** 合法 surface（与后端 `PluginCardSurface::LABELS` 逐字同集）。 */
const CARD_SURFACES: readonly PluginCardSurfaceDto[] = ["chat", "task_card"];

let registryVersion = 0;
const registryListeners = new Set<() => void>();

function bumpRegistryVersion(): void {
  registryVersion += 1;
  for (const listener of registryListeners) {
    listener();
  }
}

function subscribeRegistry(listener: () => void): () => void {
  registryListeners.add(listener);
  return () => {
    registryListeners.delete(listener);
  };
}

/** 订阅「注册表变更」；返回值只用来触发重渲染，不要拿它做业务判断。 */
export function useCardRegistryVersion(): number {
  return useSyncExternalStore(subscribeRegistry, () => registryVersion);
}

/** 宿主拼键的唯一入口（插件侧不拼键）。 */
export function cardKey(pluginId: string, id: string): PluginCardKey {
  return `${pluginId}:${id}`;
}

function isCardSurface(value: string): value is PluginCardSurfaceDto {
  return (CARD_SURFACES as readonly string[]).includes(value);
}

/**
 * 把 `plugin_list_task_cards` 的投影注册成卡片贡献。
 *
 * 只收 `enabled` 的插件；`surface` 不在词表内 / 键已存在 ⇒ 丢弃该条（前者是投影损坏，
 * 后者是先到先得）。**不抛错**：一条坏投影只应让它自己缺席，不该让整页卡片区炸掉。
 *
 * @returns 实际新增的条目数（0 表示没有可挂的，或全部被词表/先到先得挡下）。
 */
export function registerCardContributions(infos: readonly PluginTaskCardsInfoDto[]): number {
  let added = 0;
  for (const info of infos) {
    if (!info.enabled) {
      continue;
    }
    for (const card of info.taskCards) {
      if (!isCardSurface(card.surface)) {
        console.warn(
          `[pluginCardRegistry] 插件 ${info.id} 的卡片 ${card.id} 声明了未知 surface「${card.surface}」，已丢弃`,
        );
        continue;
      }
      const key = cardKey(info.id, card.id);
      if (BY_KEY.has(key)) {
        // 先到先得：宿主内置卡在模块加载期先占位 ⇒ 插件抢同名必然失败（宿主优先）。
        console.warn(`[pluginCardRegistry] 卡片键「${key}」已被占用，按先到先得忽略后一条`);
        continue;
      }
      const detailSchema = card.detailSchema ?? undefined;
      const contribution: PluginCardContribution = {
        key,
        surface: card.surface,
        pluginId: info.id,
        id: card.id,
        title: card.title,
        detailSchema,
        schemaId: detailSchema ? pluginSchemaRowId(info.id, detailSchema) : undefined,
        summaryCommand: card.summaryCommand,
        tone: card.tone ?? undefined,
        order: card.order,
        actions: card.actions,
      };
      BY_KEY.set(key, contribution);
      // 同一 schemaId 可被同插件的多张卡共用（合法），故只留首条映射、不告警。
      if (contribution.schemaId && !BY_SCHEMA_ID.has(contribution.schemaId)) {
        BY_SCHEMA_ID.set(contribution.schemaId, contribution);
      }
      const bucket = BY_SURFACE.get(contribution.surface) ?? [];
      bucket.push(contribution);
      BY_SURFACE.set(contribution.surface, bucket);
      added += 1;
    }
  }
  if (added > 0) {
    // 按 `order` 升序（同值保持注册顺序 —— sort 是稳定排序）。
    for (const bucket of BY_SURFACE.values()) {
      bucket.sort((a, b) => a.order - b.order);
    }
    bumpRegistryVersion();
  }
  return added;
}

/** 某个落位上的全部贡献（已按 `order` 升序）。返回只读快照，调用方不要就地改。 */
export function getContributions(surface: PluginCardSurfaceDto): readonly PluginCardContribution[] {
  return BY_SURFACE.get(surface) ?? [];
}

export function getContributionByKey(key: string): PluginCardContribution | undefined {
  return BY_KEY.get(key as PluginCardKey);
}

/**
 * 按卡体行 id 反查贡献（聊天侧**旧键** `schemaId` 的迁移入口）。
 *
 * 命中 ⇒ 这张卡确实由某个已启用插件声明过（归属由注册表证明）；
 * 不命中 ⇒ 消息里写的是一个未声明的裸 id，调用方必须**丢弃**（fail-closed）——
 * 否则「消息可指向任意人的 schema」这条缺口就被扩成默认路径。
 */
export function getContributionBySchemaId(schemaId: string): PluginCardContribution | undefined {
  return BY_SCHEMA_ID.get(schemaId);
}

let loadPromise: Promise<number> | null = null;

/**
 * 幂等加载卡片投影并注册（每个页面会话只发一次 IPC）。
 *
 * 两路投影（宿主 + 插件）并行取数，但**注册顺序恒宿主先**（`registerCardContributions`
 * 同步执行）⇒ 先到先得下插件抢不了宿主键（§130.3「宿主优先」）。宿主投影命令是
 * `task_card_host_cards`（纯静态组装、不依赖插件安装根），返回形状与
 * `PluginTaskCardsInfoDto` 逐字段同构，故同一 DTO 类型直达注册管线。
 *
 * 失败按「没有卡」处理（同 `TasksCenterPage` 对插件源的口径）：读不到投影只会少卡片，
 * 不该让聊天/任务中心报错。**不自动重试** —— 失败后重试会在同一次会话里反复打后端，
 * 而下一次启动/安装/启停会重新挂载并再试。
 */
export function ensureCardContributionsLoaded(): Promise<number> {
  loadPromise ??= Promise.all([
    invoke<PluginTaskCardsInfoDto[]>("task_card_host_cards").catch((error: unknown) => {
      console.warn("[pluginCardRegistry] 读取宿主卡片投影失败，按「没有宿主卡」处理", error);
      return [] as PluginTaskCardsInfoDto[];
    }),
    invoke<PluginTaskCardsInfoDto[]>("plugin_list_task_cards").catch((error: unknown) => {
      console.warn("[pluginCardRegistry] 读取插件卡片投影失败，按「没有插件卡」处理", error);
      return [] as PluginTaskCardsInfoDto[];
    }),
  ]).then(([hostInfos, pluginInfos]) => registerCardContributions(hostInfos) + registerCardContributions(pluginInfos));
  return loadPromise;
}

/**
 * 测试专用：清空注册表与加载缓存。
 *
 * 注册表是模块级单例（与 `adapters/registry.ts` 同形态），用例之间必须显式复位，
 * 否则前一个用例的贡献会渗进后一个（表现为「未声明的卡也能画出来」的假绿）。
 */
export function __resetCardRegistryForTests(): void {
  BY_KEY.clear();
  BY_SCHEMA_ID.clear();
  BY_SURFACE.clear();
  loadPromise = null;
  bumpRegistryVersion();
}
