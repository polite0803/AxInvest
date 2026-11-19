// SPDX-License-Identifier: AGPL-3.0-only

import { pluginRouteSlug, resolvePluginDomainPageRoutes } from "@/lib/pluginDomainPageRoutes";
import type { PluginDomainPacksInfoDto, PluginPagesInfoDto } from "@/types";
import { describe, expect, it } from "vitest";

const page = (id: string, loading: "bundle" | "schema" = "bundle") => ({
  id,
  title: `Page ${id}`,
  entry: loading === "bundle" ? "pages/index.html" : "schema-id",
  loading,
});

const pageInfo = (id: string, enabled: boolean, pages: ReturnType<typeof page>[]): PluginPagesInfoDto => ({
  id,
  name: id,
  version: "1.0.0",
  description: "",
  author: null,
  enabled,
  pages,
});

const packInfo = (
  id: string,
  enabled: boolean,
  domainPacks: PluginDomainPacksInfoDto["domainPacks"],
): PluginDomainPacksInfoDto => ({
  id,
  name: id,
  version: "1.0.0",
  description: "",
  author: null,
  enabled,
  domainPacks,
});

function domainPack(id: string): PluginDomainPacksInfoDto["domainPacks"][number] {
  return {
    id,
    pluginId: `plugin-${id}`,
    transportOwner: "plugin-opc-common",
    page: { id: `domain:${id}`, path: `/opc/domain/${id.replace(/_/g, "-")}` },
    capabilities: [],
    commands: [],
    calls: [],
  };
}

describe("pluginDomainPageRoutes", () => {
  it("插件 id 被转换成稳定 URL 段", () => {
    expect(pluginRouteSlug("opc-security@bundled")).toBe("opc-security-bundled");
    expect(pluginRouteSlug("opc-security")).toBe("opc-security");
  });

  it("只投影启用的域插件 bundle 页面", () => {
    const routes = resolvePluginDomainPageRoutes(
      [pageInfo("opc-security@bundled", true, [page("opc-security"), page("form", "schema")])],
      [packInfo("opc-security@bundled", true, [domainPack("security")])],
    );
    expect(routes).toHaveLength(1);
    expect(routes[0]).toMatchObject({
      pluginId: "opc-security@bundled",
      pluginName: "opc-security@bundled",
      pageId: "opc-security",
      path: "/plugins/opc-security-bundled/opc-security",
      legacyPath: "/opc/domain/security",
      navKey: "plugin-page:opc-security@bundled:opc-security",
    });
  });

  it("域包 id 中的下划线映射为运行期页面 id 的连字符", () => {
    const routes = resolvePluginDomainPageRoutes(
      [pageInfo("opc-ai-research@bundled", true, [page("opc-ai-research")])],
      [packInfo("opc-ai-research@bundled", true, [domainPack("ai_research")])],
    );
    expect(routes).toHaveLength(1);
    expect(routes[0]).toMatchObject({
      pageId: "opc-ai-research",
      legacyPath: "/opc/domain/ai-research",
    });
  });

  it("插件无域包或页面未启用时不产生路由", () => {
    expect(
      resolvePluginDomainPageRoutes(
        [pageInfo("opc-security@bundled", true, [page("opc-security")])],
        [packInfo("opc-security@bundled", true, [])],
      ),
    ).toEqual([]);
    expect(
      resolvePluginDomainPageRoutes(
        [pageInfo("opc-security@bundled", false, [page("opc-security")])],
        [packInfo("opc-security@bundled", true, [domainPack("security")])],
      ),
    ).toEqual([]);
  });

  it("page id 不适合 URL 段时跳过，避免拼接出歧义路由", () => {
    const routes = resolvePluginDomainPageRoutes(
      [pageInfo("opc-accounting", true, [page("opc-accounting"), page("bad/page")])],
      [packInfo("opc-accounting", false, [domainPack("accounting")])],
    );
    expect(routes).toEqual([]);
  });
});
