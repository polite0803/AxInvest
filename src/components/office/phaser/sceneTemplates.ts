// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Phaser 像素办公室场景模板。
 *
 * 不依赖外部图片资源 — 房间布局/家具/装饰全部用 Phaser Graphics 现场绘制。
 *
 * 坐标系：左上角原点，单位为像素。默认画布 800×500。
 *
 * 下游扩展：通过 `registerSceneTemplate()` 追加自定义模板。
 * AxAgent 自身只提供"基础能力"（默认模板 + 通用家具 + 通用装饰）。
 */
import { useSyncExternalStore } from "react";

// ── 房间 ──────────────────────────────────────────────

export interface RoomRect {
  id: string;
  nameKey: string;
  x: number;
  y: number;
  width: number;
  height: number;
  color: number;
}

// ── 家具 ──────────────────────────────────────────────

export type FurnitureKind =
  | "desk"
  | "chair"
  | "plant"
  | "whiteboard"
  | "shelf"
  | "sofa"
  | "rug"
  | "lamp"
  | "painting"
  | "window"
  | "door";

export interface RoomFurniture {
  kind: FurnitureKind;
  x: number;
  y: number;
  /** 可选宽度/高度（部分家具有自定义尺寸） */
  w?: number;
  h?: number;
  /** 可选颜色覆盖 */
  color?: number;
}

// ── 装饰 ──────────────────────────────────────────────

export interface RoomDecoration {
  kind: "window" | "painting" | "lamp" | "rug" | "door";
  x: number;
  y: number;
  w?: number;
  h?: number;
  color?: number;
}

// ── 模板 ──────────────────────────────────────────────

export interface OfficeSceneTemplate {
  slug: string;
  displayNameKey: string;
  canvasWidth: number;
  canvasHeight: number;
  rooms: RoomRect[];
  defaultRoomId: string;
  /** 家具（key = roomId） */
  furniture?: Record<string, RoomFurniture[]>;
  /** 墙面装饰：窗户/画框/灯（key = roomId） */
  decorations?: Record<string, RoomDecoration[]>;
}

/** 默认办公室模板 — 4 房间布局 */
export const DEFAULT_OFFICE_TEMPLATE: OfficeSceneTemplate = {
  slug: "default_office",
  displayNameKey: "default_office",
  canvasWidth: 800,
  canvasHeight: 500,
  defaultRoomId: "workspace",
  rooms: [
    { id: "workspace", nameKey: "workspace", x: 40, y: 60, width: 360, height: 220, color: 0x1677ff },
    { id: "meeting", nameKey: "meeting", x: 440, y: 60, width: 320, height: 180, color: 0x52c41a },
    { id: "lounge", nameKey: "lounge", x: 440, y: 280, width: 320, height: 160, color: 0xfa8c16 },
    { id: "manager", nameKey: "manager", x: 40, y: 320, width: 360, height: 120, color: 0x722ed1 },
  ],
  furniture: {
    workspace: [
      { kind: "desk", x: 80, y: 130 },
      { kind: "chair", x: 80, y: 165 },
      { kind: "desk", x: 220, y: 130 },
      { kind: "chair", x: 220, y: 165 },
      { kind: "shelf", x: 310, y: 100 },
      { kind: "plant", x: 50, y: 80 },
      { kind: "plant", x: 330, y: 220 },
    ],
    meeting: [
      { kind: "whiteboard", x: 160, y: 80 },
      { kind: "plant", x: 50, y: 80 },
      { kind: "plant", x: 280, y: 80 },
      { kind: "plant", x: 50, y: 150 },
      { kind: "plant", x: 280, y: 150 },
      { kind: "rug", x: 160, y: 120, w: 200, h: 80, color: 0x52c41a },
    ],
    lounge: [
      { kind: "sofa", x: 110, y: 80 },
      { kind: "sofa", x: 240, y: 80 },
      { kind: "plant", x: 50, y: 130 },
      { kind: "plant", x: 280, y: 130 },
      { kind: "rug", x: 160, y: 110, w: 180, h: 60, color: 0xfa8c16 },
    ],
    manager: [
      { kind: "desk", x: 100, y: 70 },
      { kind: "chair", x: 100, y: 100 },
      { kind: "shelf", x: 240, y: 60 },
      { kind: "plant", x: 310, y: 100 },
    ],
  },
  decorations: {
    workspace: [
      { kind: "window", x: 320, y: 5, w: 60, h: 40 },
      { kind: "painting", x: 160, y: 5, w: 40, h: 30 },
      { kind: "lamp", x: 20, y: 5 },
    ],
    meeting: [
      { kind: "window", x: 120, y: 5, w: 80, h: 35 },
      { kind: "painting", x: 240, y: 5, w: 40, h: 25 },
      { kind: "lamp", x: 10, y: 5 },
    ],
    lounge: [
      { kind: "window", x: 120, y: 5, w: 80, h: 30 },
      { kind: "painting", x: 240, y: 5, w: 50, h: 25 },
      { kind: "lamp", x: 10, y: 5 },
    ],
    manager: [
      { kind: "window", x: 280, y: 5, w: 50, h: 25 },
      { kind: "painting", x: 140, y: 5, w: 40, h: 20 },
      { kind: "lamp", x: 10, y: 5 },
    ],
  },
};

/** 创业 LOFT 模板 — 开放空间 3 房间布局（验证 registerSceneTemplate 扩展点） */
export const STARTUP_LOFT_TEMPLATE: OfficeSceneTemplate = {
  slug: "startup_loft",
  displayNameKey: "startup_loft",
  canvasWidth: 800,
  canvasHeight: 500,
  defaultRoomId: "studio",
  rooms: [
    { id: "studio", nameKey: "studio", x: 40, y: 60, width: 460, height: 240, color: 0x0ea5e9 },
    { id: "warroom", nameKey: "warroom", x: 540, y: 60, width: 220, height: 240, color: 0xf59e0b },
    { id: "corner", nameKey: "corner", x: 40, y: 340, width: 720, height: 110, color: 0x8b5cf6 },
  ],
  furniture: {
    studio: [
      { kind: "desk", x: 90, y: 140 },
      { kind: "chair", x: 90, y: 175 },
      { kind: "desk", x: 220, y: 140 },
      { kind: "chair", x: 220, y: 175 },
      { kind: "desk", x: 350, y: 140 },
      { kind: "chair", x: 350, y: 175 },
      { kind: "shelf", x: 420, y: 90 },
      { kind: "plant", x: 60, y: 90 },
      { kind: "plant", x: 430, y: 230 },
    ],
    warroom: [
      { kind: "whiteboard", x: 110, y: 80 },
      { kind: "rug", x: 110, y: 140, w: 160, h: 80, color: 0xf59e0b },
      { kind: "plant", x: 40, y: 100 },
      { kind: "plant", x: 180, y: 200 },
    ],
    corner: [
      { kind: "sofa", x: 200, y: 55 },
      { kind: "plant", x: 60, y: 80 },
      { kind: "plant", x: 660, y: 80 },
      { kind: "rug", x: 360, y: 65, w: 260, h: 60, color: 0x8b5cf6 },
    ],
  },
  decorations: {
    studio: [
      { kind: "window", x: 300, y: 5, w: 70, h: 40 },
      { kind: "painting", x: 180, y: 5, w: 40, h: 30 },
      { kind: "lamp", x: 20, y: 5 },
      { kind: "lamp", x: 420, y: 5 },
    ],
    warroom: [
      { kind: "window", x: 70, y: 5, w: 60, h: 30 },
      { kind: "painting", x: 160, y: 5, w: 35, h: 25 },
      { kind: "lamp", x: 10, y: 5 },
    ],
    corner: [
      { kind: "window", x: 560, y: 5, w: 80, h: 28 },
      { kind: "painting", x: 380, y: 5, w: 50, h: 25 },
      { kind: "lamp", x: 20, y: 5 },
      { kind: "lamp", x: 680, y: 5 },
    ],
  },
};

// ── 行业场景已迁至域包数据（PLAN-office-auto-provision.md 阶段 4-①）──
// finance_invest / software_dev / accounting / ai_research / content_media /
// ecommerce / education / consulting / sales_growth 的布局改由
// config/opc/domain_packs/<id>/office_scene.yaml 提供，启动时经
// registerSceneTemplate 注入（迁移由一次性生成器完成，与 TS 版逐字段一致）。

export const SCENE_TEMPLATES: OfficeSceneTemplate[] = [
  DEFAULT_OFFICE_TEMPLATE,
  STARTUP_LOFT_TEMPLATE,
];

/**
 * 注册自定义场景模板（扩展点：域包 office_scene.yaml 与业务场景经此注入）。
 *
 * 数组是**就地 push** 的可变引用，React 侧必须经 `useSceneTemplates()` 订阅——
 * 域包模板是启动 IPC 异步注入的，不订阅就会把注入竞态固化成「下拉永远少几项」。
 */
export function registerSceneTemplate(template: OfficeSceneTemplate): void {
  if (!SCENE_TEMPLATES.some((t) => t.slug === template.slug)) {
    SCENE_TEMPLATES.push(template);
    registryVersion += 1;
    for (const listener of listeners) {
      listener();
    }
  }
}

// ── 注册表订阅（4-①：YAML 异步注入与 React 渲染的时序契约）──

const listeners = new Set<() => void>();
let registryVersion = 0;

export function subscribeSceneTemplates(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 注册表版本号——getSnapshot 必须返回不可变值（数组引用恒定，不能当快照） */
export function getSceneTemplatesVersion(): number {
  return registryVersion;
}

/** 订阅式读取场景模板注册表（域包 YAML 注入完成后自动重渲染） */
export function useSceneTemplates(): OfficeSceneTemplate[] {
  useSyncExternalStore(subscribeSceneTemplates, getSceneTemplatesVersion);
  return SCENE_TEMPLATES;
}

export function resolveSceneTemplate(slug?: string): OfficeSceneTemplate {
  if (!slug) { return DEFAULT_OFFICE_TEMPLATE; }
  return SCENE_TEMPLATES.find((t) => t.slug === slug) ?? DEFAULT_OFFICE_TEMPLATE;
}

// ── 显示名单源化（`PLAN-office-scene-domain-align.md` 阶段 1）──
//
// 行业场景的 slug 与 `config/opc/domain_packs/` 域包 id 逐字一致，其显示名/描述的
// 权威翻译在 `opc.domains.*`（11 语言齐备）；`office.scene.*` 只保留非域包的
// 通用/注入场景（default_office / startup_loft / investment_office）。

/** 行业场景 slug 集合 — 与 OPC 域包本体对齐（14 个）；门禁 check-office-scene-align 读同一清单。
 *  后 5 个（阶段 4-②）无 TS 内置模板，场景由域包 `office_scene.yaml` 启动注入——
 *  「slug 必须有同名场景」由门禁 rule a（TS∪YAML 合并面）把关。 */
export const SCENE_DOMAIN_SLUGS: ReadonlySet<string> = new Set([
  "finance_invest",
  "software_dev",
  "accounting",
  "ai_research",
  "content_media",
  "ecommerce",
  "education",
  "consulting",
  "sales_growth",
  "design",
  "project_management",
  "security",
  "geospatial",
  "game_dev",
]);

/** 显示名 i18n key：行业场景走权威源 `opc.domains.<slug>`，其余走 `office.scene.<displayNameKey>` */
export function sceneTemplateLabelKey(tpl: OfficeSceneTemplate): string {
  return SCENE_DOMAIN_SLUGS.has(tpl.slug) ? `opc.domains.${tpl.slug}` : `office.scene.${tpl.displayNameKey}`;
}

/** 描述 i18n key，命名空间选择逻辑同 sceneTemplateLabelKey */
export function sceneTemplateDescKey(tpl: OfficeSceneTemplate): string {
  return `${sceneTemplateLabelKey(tpl)}_desc`;
}

/** 给定房间与成员数，计算房间内均匀分布的初始坐标 */
export function distributeMembersInRoom(
  room: RoomRect,
  memberCount: number,
  index: number,
): { x: number; y: number } {
  if (memberCount <= 0) {
    return { x: room.x + room.width / 2, y: room.y + room.height / 2 };
  }
  const cols = Math.min(4, memberCount);
  const col = index % cols;
  const row = Math.floor(index / cols);
  const cellW = room.width / (cols + 1);
  const cellH = 50;
  return { x: room.x + cellW * (col + 1), y: room.y + 40 + row * cellH };
}

/** 「建房即成队」播种排房（PLAN-office-auto-provision.md 阶段 2）：
 *  从 defaultRoomId 起轮转分到各房间，避免全员堆在同一间。返回长度 = memberCount 的 roomId 列表。 */
export function assignSeedRooms(memberCount: number, template: OfficeSceneTemplate): string[] {
  const ordered = [
    template.defaultRoomId,
    ...template.rooms.map((r) => r.id).filter((id) => id !== template.defaultRoomId),
  ];
  return Array.from({ length: memberCount }, (_, i) => ordered[i % ordered.length] ?? template.defaultRoomId);
}
