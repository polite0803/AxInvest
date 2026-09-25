// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { INVESTMENT_OFFICE_TEMPLATE } from "../investSceneTemplates";
import {
  assignSeedRooms,
  DEFAULT_OFFICE_TEMPLATE,
  type OfficeSceneTemplate,
  type RoomRect,
  SCENE_DOMAIN_SLUGS,
  SCENE_TEMPLATES,
  sceneTemplateDescKey,
  sceneTemplateLabelKey,
  STARTUP_LOFT_TEMPLATE,
} from "../sceneTemplates";

function syntheticScene(slug: string, roomIds: string[] = ["r1"]): OfficeSceneTemplate {
  const rooms: RoomRect[] = roomIds.map((id, i) => ({
    id,
    nameKey: id,
    x: 40 + i * 200,
    y: 60,
    width: 160,
    height: 120,
    color: 0x1677ff,
  }));
  return {
    slug,
    displayNameKey: slug,
    canvasWidth: 800,
    canvasHeight: 500,
    defaultRoomId: roomIds[0],
    rooms,
  };
}

describe("sceneTemplateLabelKey 显示名单源化", () => {
  it("行业场景走权威源 opc.domains.<slug>", () => {
    const finance = syntheticScene("finance_invest");
    expect(sceneTemplateLabelKey(finance)).toBe("opc.domains.finance_invest");
    expect(sceneTemplateDescKey(finance)).toBe("opc.domains.finance_invest_desc");
  });

  it("通用/注入场景保留 office.scene.<displayNameKey>", () => {
    expect(sceneTemplateLabelKey(DEFAULT_OFFICE_TEMPLATE)).toBe("office.scene.default_office");
    expect(sceneTemplateDescKey(DEFAULT_OFFICE_TEMPLATE)).toBe("office.scene.default_office_desc");
    expect(SCENE_DOMAIN_SLUGS.has(INVESTMENT_OFFICE_TEMPLATE.slug)).toBe(false);
    expect(sceneTemplateLabelKey(INVESTMENT_OFFICE_TEMPLATE)).toBe("office.scene.investment_office");
  });

  it("注册表：名册 14 个行业 slug；TS 内置自 4-① 起不再含行业场景（全部 YAML 注入）", () => {
    expect(SCENE_DOMAIN_SLUGS.size).toBe(14);
    expect(SCENE_TEMPLATES.filter((tpl) => SCENE_DOMAIN_SLUGS.has(tpl.slug))).toHaveLength(0);
    // 内置只剩两个通用场景；「slug 必须有同名场景（TS∪YAML）」由 check-office-scene-align rule a 把关
    expect(SCENE_TEMPLATES.map((t) => t.slug)).toEqual([
      DEFAULT_OFFICE_TEMPLATE.slug,
      STARTUP_LOFT_TEMPLATE.slug,
    ]);
  });
});

describe("assignSeedRooms 建房即成队排房", () => {
  it("首个成员落 defaultRoomId，其后轮转覆盖所有房间", () => {
    const finance = syntheticScene("finance_invest", ["trading", "analysis", "risk", "meeting"]);
    const rooms = assignSeedRooms(8, finance);
    expect(rooms).toHaveLength(8);
    expect(rooms[0]).toBe("trading");
    const byRoom: Record<string, number> = {};
    for (const r of rooms) {
      byRoom[r] = (byRoom[r] ?? 0) + 1;
    }
    for (const id of ["trading", "analysis", "risk", "meeting"]) {
      expect(byRoom[id]).toBe(2);
    }
  });

  it("成员数超过房间数仍轮转、为 0 时返回空", () => {
    expect(assignSeedRooms(0, DEFAULT_OFFICE_TEMPLATE)).toEqual([]);
    const rooms = assignSeedRooms(10, DEFAULT_OFFICE_TEMPLATE);
    expect(rooms).toHaveLength(10);
    expect(new Set(rooms).size).toBe(DEFAULT_OFFICE_TEMPLATE.rooms.length);
  });
});

describe("注册表订阅（4-① 时序契约）", () => {
  it("registerSceneTemplate 注入后版本递增且数组含新 slug", async () => {
    const mod = await import("../sceneTemplates");
    const before = mod.getSceneTemplatesVersion();
    const probe = syntheticScene("__probe_scene__");
    mod.registerSceneTemplate(probe);
    expect(mod.getSceneTemplatesVersion()).toBe(before + 1);
    expect(mod.SCENE_TEMPLATES.some((t) => t.slug === "__probe_scene__")).toBe(true);
    // 幂等：同 slug 再注册不递增
    const after = mod.getSceneTemplatesVersion();
    mod.registerSceneTemplate(probe);
    expect(mod.getSceneTemplatesVersion()).toBe(after);
    // 清理探针，防污染其它用例（直接 splice，注册表无注销 API——运行期不需要）
    const idx = mod.SCENE_TEMPLATES.findIndex((t) => t.slug === "__probe_scene__");
    mod.SCENE_TEMPLATES.splice(idx, 1);
  });
});
