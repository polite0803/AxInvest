// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App as AntApp } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DemandDiscoveryPage } from "../DemandDiscoveryPage";

const invokeMock = vi.fn();

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    // 保持 key 透传，便于用 i18n key 断言；插值参数忽略
    t: (key: string) => key,
  }),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const MOCK_PLATFORMS = [
  {
    id: "reddit",
    name: "Reddit",
    platformType: "scanner",
    enabled: true,
    baseUrl: "https://www.reddit.com",
    config: null,
    lastSyncAt: null,
    status: "idle",
    createdAt: 1700000000,
    updatedAt: 1700000000,
  },
];

const MOCK_LEADS = [
  {
    id: "lead-1",
    platform: "reddit",
    title: "Self-hosted wiki with semantic search",
    description: "We cannot use third-party SaaS.",
    budgetMin: null,
    budgetMax: null,
    budgetCurrency: "USD",
    contactName: null,
    contactEmail: null,
    contactPhone: null,
    sourceUrl: "https://www.reddit.com/r/selfhosted/comments/abc",
    status: "new",
    confidence: 0.62,
    painScore: 74,
    marketGapScore: 68,
    commercialValueScore: 72.5,
    opportunityLevel: "high",
    demandType: "tool_software",
    createdAt: 1700000000,
    updatedAt: 1700000000,
  },
];

const MOCK_SUMMARY = {
  totalScanned: 12,
  totalEvaluated: 12,
  totalSaved: 5,
  highValueCount: 3,
  leads: MOCK_LEADS,
};

function renderPage() {
  return render(
    <AntApp>
      <DemandDiscoveryPage />
    </AntApp>,
  );
}

describe("DemandDiscoveryPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "opc_list_platforms") {
        return Promise.resolve(MOCK_PLATFORMS);
      }
      if (cmd === "opc_list_leads") {
        return Promise.resolve(MOCK_LEADS);
      }
      if (cmd === "opc_discover_and_evaluate_leads") {
        return Promise.resolve(MOCK_SUMMARY);
      }
      return Promise.reject(new Error(`unexpected command: ${cmd}`));
    });
  });

  it("挂载时加载线索与平台配置", async () => {
    renderPage();

    await waitFor(() => {
      expect(screen.getByText("Self-hosted wiki with semantic search")).toBeInTheDocument();
    });

    const cmds = invokeMock.mock.calls.map((c) => c[0]);
    expect(cmds).toContain("opc_list_leads");
    expect(cmds).toContain("opc_list_platforms");
  });

  it("空关键词点击扫描时提示输入关键词，不发起扫描", async () => {
    renderPage();

    await userEvent.click(screen.getByRole("button", { name: "opc.demand.btnDiscover" }));

    await waitFor(() => {
      expect(screen.getByText("opc.demand.enterSearchQuery")).toBeInTheDocument();
    });
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "opc_discover_and_evaluate_leads"),
    ).toHaveLength(0);
  });

  it("输入关键词扫描后展示执行摘要", async () => {
    renderPage();

    await userEvent.type(screen.getByRole("textbox"), "self-hosted wiki");
    await userEvent.click(screen.getByRole("button", { name: "opc.demand.btnDiscover" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("opc_discover_and_evaluate_leads", {
        query: "self-hosted wiki",
      });
    });

    // 摘要卡片出现（统计项标题）
    await waitFor(() => {
      expect(screen.getByText("opc.demand.statSaved")).toBeInTheDocument();
    });
    expect(screen.getByText("opc.demand.statHighValue")).toBeInTheDocument();
  });
});
