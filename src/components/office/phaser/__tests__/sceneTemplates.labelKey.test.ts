// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { INVESTMENT_OFFICE_TEMPLATE } from "../investSceneTemplates";
import {
  assignSeedRooms,
  DEFAULT_OFFICE_TEMPLATE,
  SCENE_DOMAIN_SLUGS,
  SCENE_TEMPLATES,
  sceneTemplateDescKey,
  sceneTemplateLabelKey,
} from "../sceneTemplates";

describe("sceneTemplateLabelKey 显示名单源化", () => {
  it("行业场景走权威源 opc.domains.<slug>", () => {
    const finance = SCENE_TEMPLATES.find((tpl) => tpl.slug === "finance_invest");
    expect(finance).toBeDefined();
    expect(sceneTemplateLabelKey(finance!)).toBe("opc.domains.finance_invest");
    expect(sceneTemplateDescKey(finance!)).toBe("opc.domains.finance_invest_desc");
  });

  it("通用/注入场景保留 office.scene.<displayNameKey>", () => {
    expect(sceneTemplateLabelKey(DEFAULT_OFFICE_TEMPLATE)).toBe("office.scene.default_office");
    expect(sceneTemplateDescKey(DEFAULT_OFFICE_TEMPLATE)).toBe("office.scene.default_office_desc");
    expect(SCENE_DOMAIN_SLUGS.has(INVESTMENT_OFFICE_TEMPLATE.slug)).toBe(false);
    expect(sceneTemplateLabelKey(INVESTMENT_OFFICE_TEMPLATE)).toBe("office.scene.investment_office");
  });

  it("SCENE_DOMAIN_SLUGS 覆盖且仅覆盖 9 个与域包同名 slug 的行业场景", () => {
    const domainScenes = SCENE_TEMPLATES.filter((tpl) => SCENE_DOMAIN_SLUGS.has(tpl.slug));
    expect(domainScenes).toHaveLength(9);
    // 集合内每个 slug 都必须有对应场景，防「集合加了、场景没加」的悬空声明
    for (const slug of SCENE_DOMAIN_SLUGS) {
      expect(SCENE_TEMPLATES.some((tpl) => tpl.slug === slug)).toBe(true);
    }
  });
});

describe("assignSeedRooms 建房即成队排房", () => {
  it("首个成员落 defaultRoomId，其后轮转覆盖所有房间", () => {
    const finance = SCENE_TEMPLATES.find((tpl) => tpl.slug === "finance_invest")!;
    const rooms = assignSeedRooms(8, finance);
    expect(rooms).toHaveLength(8);
    expect(rooms[0]).toBe(finance.defaultRoomId);
    // 8 人 4 间 ⇒ 每间恰 2 人
    const byRoom: Record<string, number> = {};
    for (const r of rooms) {
      byRoom[r] = (byRoom[r] ?? 0) + 1;
    }
    for (const room of finance.rooms) {
      expect(byRoom[room.id]).toBe(2);
    }
  });

  it("成员数超过房间数仍轮转、为 0 时返回空", () => {
    expect(assignSeedRooms(0, DEFAULT_OFFICE_TEMPLATE)).toEqual([]);
    const rooms = assignSeedRooms(10, DEFAULT_OFFICE_TEMPLATE);
    expect(rooms).toHaveLength(10);
    expect(new Set(rooms).size).toBe(DEFAULT_OFFICE_TEMPLATE.rooms.length);
  });
});
