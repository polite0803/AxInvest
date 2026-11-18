// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import { getPluginPageRoutes, pluginPageNavItem, refreshPluginPageRoutes } from "@/lib/pluginRouteRegistry";
import type { PluginDomainPacksInfoDto, PluginPagesInfoDto } from "@/types";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: vi.fn() };
});

const pages: PluginPagesInfoDto[] = [
  {
    id: "opc-security@bundled",
    name: "security",
    version: "1.0.0",
    description: "",
    author: null,
    enabled: true,
    pages: [{ id: "opc-security", title: "安全合规", entry: "pages/index.html", loading: "bundle" }],
  },
];

const packs: PluginDomainPacksInfoDto[] = [
  {
    id: "opc-security@bundled",
    name: "security",
    version: "1.0.0",
    description: "",
    author: null,
    enabled: true,
    domainPacks: [{
      id: "security",
      pluginId: "plugin-opc-security",
      transportOwner: "plugin-opc-common",
      page: { id: "domain:devops-security", path: "/opc/domain/security" },
      capabilities: [],
      commands: [],
      calls: [],
    }],
  },
];

describe("pluginRouteRegistry", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("刷新后登记启用的插件域页", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(pages)
      .mockResolvedValueOnce(packs);
    await refreshPluginPageRoutes();
    expect(getPluginPageRoutes()).toHaveLength(1);
    expect(getPluginPageRoutes()[0]?.path).toBe("/plugins/opc-security-bundled/opc-security");
  });

  it("后端不再返回禁用插件时，刷新会清空旧快照", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([]);
    await refreshPluginPageRoutes();
    expect(getPluginPageRoutes()).toEqual([]);
  });

  it("导航项保留插件纯文本标题与独立路径", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(pages)
      .mockResolvedValueOnce(packs);
    await refreshPluginPageRoutes();
    expect(pluginPageNavItem(getPluginPageRoutes()[0]!)).toEqual({
      key: "plugin-page:opc-security@bundled:opc-security",
      labelKey: "安全合规",
      path: "/plugins/opc-security-bundled/opc-security",
      isPlugin: true,
      pluginName: "security",
    });
  });

  it("投影加载失败时清空旧快照，避免禁用后残留入口", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(pages)
      .mockResolvedValueOnce(packs);
    await refreshPluginPageRoutes();
    expect(getPluginPageRoutes()).toHaveLength(1);

    vi.mocked(invoke).mockRejectedValueOnce(new Error("plugin manager unavailable"));
    vi.mocked(invoke).mockResolvedValueOnce([]);
    await expect(refreshPluginPageRoutes()).rejects.toThrow("plugin manager unavailable");
    expect(getPluginPageRoutes()).toEqual([]);
  });
});
