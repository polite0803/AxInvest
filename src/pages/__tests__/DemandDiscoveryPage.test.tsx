// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
    linkedWorkflowId: null,
    implementedAt: null,
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

const MOCK_INVOICES = [
  {
    id: "inv-1",
    leadId: "lead-won-1",
    linkedWorkflowId: null,
    title: "库存同步系统",
    amount: 80000,
    currency: "CNY",
    status: "sent",
    issuedAt: 1700000500,
    paidAt: null,
    notes: null,
    createdAt: 1700000450,
    updatedAt: 1700000500,
  },
];

const MOCK_DELIVERY_SUMMARY = {
  wonLeads: 1,
  activeLeads: 2,
  invoiceCount: 1,
  paidCount: 0,
  revenues: [{ currency: "CNY", paidTotal: 0, issuedTotal: 80000 }],
  conversionRate: 0.5,
};

/** 默认 mock 里追加交付环命令（挂载时 loadInvoices 就会调用） */
function withDeliveryMocks(impl: (cmd: string) => Promise<unknown>) {
  return (cmd: string) => {
    if (cmd === "opc_list_invoices") {
      return Promise.resolve(MOCK_INVOICES);
    }
    if (cmd === "opc_get_delivery_summary") {
      return Promise.resolve(MOCK_DELIVERY_SUMMARY);
    }
    return impl(cmd);
  };
}

const MOCK_SUBSCRIPTIONS = [
  {
    id: "sub-1",
    keyword: "self-hosted wiki",
    enabled: true,
    intervalHours: 6,
    minScore: 60,
    platforms: [],
    lastScannedAt: null,
    lastHitCount: 0,
    createdAt: 1700000300,
    updatedAt: 1700000300,
  },
];

const MOCK_SCAN_POLICY = {
  concurrency: 4,
  rateLimitPerMin: 60,
  retryMax: 2,
  retryBackoffMs: 500,
  timeoutSecs: 15,
  dedupWindowHours: 168,
  maxLeadsPerScan: 200,
};

const MOCK_SUB_SUMMARY = {
  scannedSubscriptions: 1,
  totalSaved: 2,
  totalRefreshed: 1,
  highValueHits: 1,
  outcomes: [
    {
      subscriptionId: "sub-1",
      keyword: "self-hosted wiki",
      ok: true,
      error: null,
      hits: MOCK_LEADS,
    },
  ],
};

function renderPage() {
  return render(
    <AntApp>
      <DemandDiscoveryPage />
    </AntApp>,
  );
}

describe(
  "DemandDiscoveryPage",
  // 重交互用例（多轮 waitFor + antd 浮层）在全套并行的高负载下会超 5s 默认值；
  // 单文件跑全绿，故只对该 describe 放宽，不动全局 testTimeout。
  { timeout: 15_000 },
  () => {
    beforeEach(() => {
      vi.clearAllMocks();
      invokeMock.mockImplementation(
        withDeliveryMocks((cmd: string) => {
          if (cmd === "opc_list_platforms") {
            return Promise.resolve(MOCK_PLATFORMS);
          }
          if (cmd === "opc_list_leads") {
            return Promise.resolve(MOCK_LEADS);
          }
          if (cmd === "opc_discover_and_evaluate_leads") {
            return Promise.resolve(MOCK_SUMMARY);
          }
          if (cmd === "opc_list_subscriptions") {
            return Promise.resolve(MOCK_SUBSCRIPTIONS);
          }
          if (cmd === "opc_get_scan_policy") {
            return Promise.resolve(MOCK_SCAN_POLICY);
          }
          if (cmd === "opc_save_scan_policy") {
            return Promise.resolve((invokeMock.mock.calls.at(-1)?.[1] as { policy?: unknown })?.policy);
          }
          return Promise.reject(new Error(`unexpected command: ${cmd}`));
        }),
      );
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

    it("平台 tab 展示扫描策略面板，修改后保存调用配置命令", async () => {
      renderPage();

      // 切到平台 tab
      await userEvent.click(screen.getByText("opc.demand.platforms"));
      expect(await screen.findByText("opc.demand.scanPolicyTitle")).toBeInTheDocument();

      // 保存 → 透传当前策略
      await userEvent.click(screen.getByRole("button", { name: "opc.demand.savePolicy" }));
      await waitFor(() => {
        expect(screen.getByText("opc.demand.policySaved")).toBeInTheDocument();
      });
      const saveCalls = invokeMock.mock.calls.filter((c) => c[0] === "opc_save_scan_policy");
      expect(saveCalls).toHaveLength(1);
      expect(saveCalls[0][1]).toEqual({ policy: MOCK_SCAN_POLICY });
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

    it("未转化线索显示转工作流按钮，点击后调用转化命令", async () => {
      invokeMock.mockImplementation(withDeliveryMocks((cmd: string) => {
        if (cmd === "opc_list_platforms") {
          return Promise.resolve(MOCK_PLATFORMS);
        }
        if (cmd === "opc_list_leads") {
          return Promise.resolve(MOCK_LEADS);
        }
        if (cmd === "opc_convert_lead_to_workflow") {
          return Promise.resolve({ id: "demand:lead:lead-1" });
        }
        return Promise.reject(new Error(`unexpected command: ${cmd}`));
      }));

      renderPage();

      await waitFor(() => {
        expect(screen.getByRole("button", { name: "opc.demand.convertToWorkflow" })).toBeInTheDocument();
      });
      // antd Table 在 jsdom 下渲染测量用副本按钮（pointer-events: none），
      // userEvent 会拒绝点击，这里用 fireEvent 绕过指针交互检查
      fireEvent.click(screen.getByRole("button", { name: "opc.demand.convertToWorkflow" }));

      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("opc_convert_lead_to_workflow", { leadId: "lead-1" });
      });
    });

    it("点击能力匹配后弹窗展示结论（partial 带缺口域）", async () => {
      invokeMock.mockImplementation(withDeliveryMocks((cmd: string) => {
        if (cmd === "opc_list_platforms") {
          return Promise.resolve(MOCK_PLATFORMS);
        }
        if (cmd === "opc_list_leads") {
          return Promise.resolve(MOCK_LEADS);
        }
        if (cmd === "opc_match_lead_capabilities") {
          return Promise.resolve({
            leadId: "lead-1",
            verdict: "partial",
            bestScore: 0.71,
            matches: [
              {
                capabilityId: "cap-1",
                name: "CI/CD 流水线工作流",
                kind: "workflow",
                domain: "devops",
                retrievalScore: 0.71,
                summary: null,
              },
            ],
            requiredDomains: ["devops", "general"],
            missingDomains: ["general"],
            gapHint: "需求类型 development 需要以下能力域，当前能力库未覆盖：general",
          });
        }
        return Promise.reject(new Error(`unexpected command: ${cmd}`));
      }));

      renderPage();

      // antd Table 在 jsdom 下渲染测量用副本按钮（pointer-events: none），
      // userEvent 会拒绝点击，这里用 fireEvent 绕过指针交互检查
      await waitFor(() => {
        expect(screen.getByRole("button", { name: "opc.demand.capMatch" })).toBeInTheDocument();
      });
      fireEvent.click(screen.getAllByRole("button", { name: "opc.demand.capMatch" })[0]);

      // 结论标签与缺口域进入弹窗
      await waitFor(() => {
        expect(screen.getByText("opc.demand.capVerdict.partial")).toBeInTheDocument();
      });
      expect(screen.getByText("opc.demand.capMissingDomains")).toBeInTheDocument();
      expect(invokeMock).toHaveBeenCalledWith("opc_match_lead_capabilities", { leadId: "lead-1" });
    });

    it("交付页展示汇总与发票账本", async () => {
      renderPage();

      fireEvent.click(screen.getByText("opc.demand.delivery"));

      // 汇总卡片出现
      await waitFor(() => {
        expect(screen.getByText("opc.demand.delivStatWon")).toBeInTheDocument();
      });
      expect(screen.getByText("opc.demand.delivStatConversion")).toBeInTheDocument();
      // 发票表格出现
      expect(screen.getByText("库存同步系统")).toBeInTheDocument();
      expect(screen.getByText("opc.demand.invoiceStatus.sent")).toBeInTheDocument();
    });

    it("won 线索显示开票按钮，点击后调用开票命令", async () => {
      invokeMock.mockImplementation(
        withDeliveryMocks((cmd: string) => {
          if (cmd === "opc_list_platforms") {
            return Promise.resolve(MOCK_PLATFORMS);
          }
          if (cmd === "opc_list_leads") {
            return Promise.resolve([{ ...MOCK_LEADS[0], id: "lead-won-1", status: "won" }]);
          }
          if (cmd === "opc_create_invoice_from_lead") {
            return Promise.resolve(MOCK_INVOICES[0]);
          }
          return Promise.reject(new Error(`unexpected command: ${cmd}`));
        }),
      );

      renderPage();

      await waitFor(() => {
        expect(screen.getByRole("button", { name: "opc.demand.createInvoice" })).toBeInTheDocument();
      });
      fireEvent.click(screen.getByRole("button", { name: "opc.demand.createInvoice" }));

      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("opc_create_invoice_from_lead", {
          leadId: "lead-won-1",
        });
      });
    });

    it("订阅页展示词表，立即扫描走 onlyDue=false", async () => {
      invokeMock.mockImplementation(withDeliveryMocks((cmd: string) => {
        if (cmd === "opc_list_platforms") {
          return Promise.resolve(MOCK_PLATFORMS);
        }
        if (cmd === "opc_list_leads") {
          return Promise.resolve(MOCK_LEADS);
        }
        if (cmd === "opc_list_subscriptions") {
          return Promise.resolve(MOCK_SUBSCRIPTIONS);
        }
        if (cmd === "opc_run_subscription_scan") {
          return Promise.resolve(MOCK_SUB_SUMMARY);
        }
        return Promise.reject(new Error(`unexpected command: ${cmd}`));
      }));

      renderPage();

      // 切到订阅页
      fireEvent.click(screen.getByText("opc.demand.subscriptions"));

      await waitFor(() => {
        expect(screen.getByText("self-hosted wiki")).toBeInTheDocument();
      });

      // 立即扫描全部订阅 → onlyDue=false（忽略间隔）
      await userEvent.click(screen.getByRole("button", { name: "opc.demand.scanNow" }));

      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("opc_run_subscription_scan", { onlyDue: false });
      });
      // 扫描摘要出现
      await waitFor(() => {
        expect(screen.getByText("opc.demand.subStatHits")).toBeInTheDocument();
      });
    });

    it("启用定时扫描时把 cron 表达式传给装配命令", async () => {
      invokeMock.mockImplementation(withDeliveryMocks((cmd: string) => {
        if (cmd === "opc_list_platforms") {
          return Promise.resolve(MOCK_PLATFORMS);
        }
        if (cmd === "opc_list_leads") {
          return Promise.resolve(MOCK_LEADS);
        }
        if (cmd === "opc_list_subscriptions") {
          return Promise.resolve(MOCK_SUBSCRIPTIONS);
        }
        if (cmd === "opc_ensure_demand_scan_job") {
          return Promise.resolve({ id: "job-1" });
        }
        return Promise.reject(new Error(`unexpected command: ${cmd}`));
      }));

      renderPage();
      fireEvent.click(screen.getByText("opc.demand.subscriptions"));

      await waitFor(() => {
        expect(screen.getByRole("button", { name: "opc.demand.enableScheduledScan" })).toBeInTheDocument();
      });
      await userEvent.click(
        screen.getByRole("button", { name: "opc.demand.enableScheduledScan" }),
      );

      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("opc_ensure_demand_scan_job", {
          cronExpression: "0 */6 * * *",
        });
      });
    });
  },
);
