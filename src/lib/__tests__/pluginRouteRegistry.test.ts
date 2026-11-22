// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import {
  getPluginDomainRouteMappings,
  getPluginPageRoutes,
  getPluginPageRoutesError,
  getPluginPageRoutesReady,
  pluginPageNavItem,
  refreshPluginPageRoutes,
} from "@/lib/pluginRouteRegistry";
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

  it("慢 invoke 期间连续启停，最后一次快照胜出", async () => {
    let resolvePages!: (value: PluginPagesInfoDto[]) => void;
    let resolvePacks!: (value: PluginDomainPacksInfoDto[]) => void;
    const slowPages = new Promise<PluginPagesInfoDto[]>((resolve) => {
      resolvePages = resolve;
    });
    const slowPacks = new Promise<PluginDomainPacksInfoDto[]>((resolve) => {
      resolvePacks = resolve;
    });

    vi.mocked(invoke)
      .mockImplementationOnce(() => slowPages)
      .mockImplementationOnce(() => slowPacks)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([]);

    const first = refreshPluginPageRoutes();
    const second = refreshPluginPageRoutes();
    // 第一轮尚未回包时插件已被禁用 ⇒ 旧结果必须丢弃，worker 继续消费新一轮。
    resolvePages(pages);
    resolvePacks(packs);
    await Promise.all([first, second]);
    expect(getPluginPageRoutes()).toEqual([]);
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

  it("域包投影失败不阻断插件页入口，只丢旧链映射", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(pages)
      .mockRejectedValueOnce(new Error("domain pack projection unavailable"));
    await refreshPluginPageRoutes();
    expect(getPluginPageRoutes()).toHaveLength(1);
    expect(getPluginPageRoutes()[0]?.legacyPath).toBeUndefined();
    expect(getPluginDomainRouteMappings()).toEqual([]);
    expect(getPluginPageRoutesError()).toBeNull();
  });

  it("首帧 gate 区分失败与空表：ready 后可读错误，成功刷新后清空", async () => {
    const failure = new Error("plugin manager unavailable");
    vi.mocked(invoke).mockRejectedValueOnce(failure);
    vi.mocked(invoke).mockResolvedValueOnce([]);
    await expect(refreshPluginPageRoutes()).rejects.toThrow(failure);
    expect(getPluginPageRoutesReady()).toBe(true);
    expect(getPluginPageRoutesError()).toBe(failure);

    vi.mocked(invoke)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([]);
    await refreshPluginPageRoutes();
    expect(getPluginPageRoutes()).toEqual([]);
    expect(getPluginPageRoutesError()).toBeNull();
  });

  it("旧链映射带 navKey 与页面同源，插件禁用后一并清空", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(pages)
      .mockResolvedValueOnce(packs);
    await refreshPluginPageRoutes();
    expect(getPluginDomainRouteMappings()).toEqual([{
      legacyPath: "/opc/domain/security",
      pluginSlug: "opc-security-bundled",
      pageId: "opc-security",
      navKey: "plugin-page:opc-security@bundled:opc-security",
    }]);

    vi.mocked(invoke)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([]);
    await refreshPluginPageRoutes();
    expect(getPluginDomainRouteMappings()).toEqual([]);
  });
});
