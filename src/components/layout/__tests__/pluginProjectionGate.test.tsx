// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件投影首帧 gate 回归。
 *
 * 首帧空快照既可能是「后端还没投影完」，也可能是「插件确实被禁用了」。
 * 旧实现把两者都当空表 ⇒ 冷启动直达链闪 404、旧链误跳 /opc。
 * 本组用例渲染 ContentArea 全路由表，分别锁住等待 / 就绪 / 失败 / 缺失四种分支。
 */

import { act, render, screen } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// 用可控外部 store 驱动 mock hook：ready 翻转后必须真正触发订阅者重渲染。
const projection = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const state = {
    ready: false,
    error: null as unknown,
    mappings: [] as Array<Record<string, string>>,
    pages: [] as Array<Record<string, unknown>>,
  };
  return {
    state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    set(patch: Partial<typeof state>) {
      Object.assign(state, patch);
      for (const listener of listeners) { listener(); }
    },
  };
});

vi.mock("@/components/layout/AppHeader", () => ({ AppHeader: () => null }));
vi.mock("@/hooks/useIpcHealth", () => ({ useIpcHealth: () => ({ ok: true }) }));
vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("antd", () => ({
  Button: ({ children, onClick }: { children?: React.ReactNode; onClick?: () => void }) => (
    <button type="button" onClick={onClick}>{children}</button>
  ),
  Result: ({ status, title }: { status?: string; title?: React.ReactNode }) => (
    <div data-testid="result" data-status={status}>{title}</div>
  ),
  Spin: () => <div data-testid="projection-loading" />,
  Typography: { Text: ({ children }: { children?: React.ReactNode }) => <span>{children}</span> },
}));
vi.mock("@/lib/pluginRouteRegistry", async () => {
  const { useSyncExternalStore } = await import("react");
  return {
    usePluginDomainRouteMappings: () => useSyncExternalStore(projection.subscribe, () => projection.state.mappings),
    usePluginPageRoutes: () => useSyncExternalStore(projection.subscribe, () => projection.state.pages),
    usePluginPageRoutesReady: () => useSyncExternalStore(projection.subscribe, () => projection.state.ready),
    getPluginPageRoutesError: () => projection.state.error,
    refreshPluginPageRoutes: vi.fn(async () => []),
  };
});
vi.mock("@/components/plugin/PluginPageRouteHost", () => ({
  PluginPageRouteHost: () => <div data-testid="plugin-page-host" />,
}));

const DOMAIN_PAGE_EXPORTS = [
  "FinanceInvestPage",
  "AccountingPage",
  "SalesGrowthPage",
  "ProjectManagementPage",
  "ConsultingPage",
  "EcommercePage",
  "SoftwareDevPage",
  "SecurityPage",
  "GeospatialPage",
  "AiResearchPage",
  "ContentMediaPage",
  "DesignPage",
  "EducationPage",
  "GameDevPage",
] as const;
vi.mock(
  "@/pages/opc/domains/DomainPages",
  () => Object.fromEntries(DOMAIN_PAGE_EXPORTS.map((name) => [name, () => <div data-testid="domain-page" />])),
);

function LocationProbe() {
  return <div data-testid="loc">{useLocation().pathname}</div>;
}

async function renderAt(path: string) {
  const { ContentArea } = await import("../ContentArea");
  return render(
    <MemoryRouter initialEntries={[path]}>
      <LocationProbe />
      <ContentArea />
    </MemoryRouter>,
  );
}

describe("插件投影首帧 gate", () => {
  beforeEach(() => {
    projection.set({ ready: false, error: null, mappings: [], pages: [] });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("插件页直达在 ready 前等待，不闪 404；就绪后渲染插件页", async () => {
    await renderAt("/plugins/opc-security-bundled/opc-security");
    expect(screen.getByTestId("projection-loading")).toBeInTheDocument();
    expect(screen.queryByTestId("result")).toBeNull();

    await act(async () => {
      projection.set({
        ready: true,
        pages: [{
          id: "opc-security@bundled:opc-security",
          path: "/plugins/opc-security-bundled/opc-security",
          navKey: "plugin-page:opc-security@bundled:opc-security",
          navLabel: "安全合规",
          navLabelIsI18nKey: false,
          navGroup: "packs",
          icon: null,
          navOrder: 0,
          hidden: false,
          pluginName: "security",
          pluginSlug: "opc-security-bundled",
          pageId: "opc-security",
        }],
      });
    });
    expect(await screen.findByTestId("plugin-page-host")).toBeInTheDocument();
  });

  it("旧链在 ready 前不跳 /opc；映射缺失时停在明确 404", async () => {
    await renderAt("/opc/domain/security");
    expect(screen.getByTestId("projection-loading")).toBeInTheDocument();
    expect(screen.getByTestId("loc")).toHaveTextContent("/opc/domain/security");

    await act(async () => {
      projection.set({ ready: true });
    });
    expect(screen.getByTestId("loc")).toHaveTextContent("/opc/domain/security");
    expect(screen.getByTestId("result")).toHaveAttribute("data-status", "404");
    expect(screen.getByTestId("result")).toHaveTextContent("plugin.pages.empty");
  });

  it("映射存在时旧链渲染对应插件页并带上 page context", async () => {
    await renderAt("/opc/domain/security");
    await act(async () => {
      projection.set({
        ready: true,
        mappings: [{
          legacyPath: "/opc/domain/security",
          pluginSlug: "opc-security-bundled",
          pageId: "opc-security",
          navKey: "plugin-page:opc-security@bundled:opc-security",
        }],
      });
    });
    expect(await screen.findByTestId("plugin-page-host")).toBeInTheDocument();
    expect(screen.getByTestId("loc")).toHaveTextContent("/opc/domain/security");
  });

  it("插件禁用后其直达链不再可达：就绪但无投影即 404", async () => {
    await renderAt("/plugins/opc-security-bundled/opc-security");
    await act(async () => {
      projection.set({ ready: true });
    });
    expect(screen.getByTestId("result")).toHaveTextContent("404");
    expect(screen.queryByTestId("plugin-page-host")).toBeNull();
  });

  it("普通 404 不被插件投影卡住", async () => {
    await renderAt("/definitely-missing");
    expect(screen.getByTestId("result")).toHaveTextContent("404");
    expect(screen.queryByTestId("projection-loading")).toBeNull();
  });

  it("投影超时给出失败态与重试，而不是无限 loading", async () => {
    vi.useFakeTimers();
    await renderAt("/plugins/opc-security-bundled/opc-security");
    expect(screen.getByTestId("projection-loading")).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    expect(screen.queryByTestId("projection-loading")).toBeNull();
    expect(screen.getByTestId("result")).toHaveTextContent("plugin.pages.loadFailed");
  });
});
