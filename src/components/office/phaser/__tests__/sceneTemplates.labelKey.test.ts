// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { INVESTMENT_OFFICE_TEMPLATE } from "../investSceneTemplates";
import {
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
