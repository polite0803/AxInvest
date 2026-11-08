import { render, screen } from "@testing-library/react";
import { App as AntdApp } from "antd";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DataVendorsTab } from "../DataVendorsTab";

// invoke 按命令名分流：模板加载恒回空 variables（让 loaded 置真、渲染继续），
// get_vendor_fallback_log 由每条用例自己决定回什么形态。
const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }));

// 同 DecisionBanner.test.tsx 的替身口径：字符串 ⇒ 回 key 原文（这样三态文案可按 key 断言），
// 对象 ⇒ 拼成 `key|k=v`（这样能断言插值传的是几条）。
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, second?: unknown) => {
      if (second && typeof second === "object") {
        const kv = Object.entries(second as Record<string, unknown>)
          .map(([k, v]) => `${k}=${v}`)
          .join(",");
        return `${key}|${kv}`;
      }
      return key;
    },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

function mountTab(fallbackReply: () => Promise<unknown>) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_workflow_template") { return Promise.resolve({ variables: [] }); }
    return fallbackReply();
  });
  return render(
    <AntdApp>
      <DataVendorsTab />
    </AntdApp>,
  );
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("DataVendorsTab 换源记录的三种缺席", () => {
  it("返回值不是列表 ⇒ 说「本环境不提供」，不得伪装成「本次没有换源」", async () => {
    mountTab(() => Promise.resolve({}));
    expect(await screen.findByText("stockAnalysis.settings.vendors.fallbackUnavailable")).toBeTruthy();
    // 负控侧：空态那句必须同时**不出现**，否则两种缺席又并排成可比
    expect(screen.queryByText("stockAnalysis.settings.vendors.fallbackEmpty")).toBeNull();
  });

  it("命令报错 ⇒ 说读取失败", async () => {
    mountTab(() => Promise.reject(new Error("boom")));
    expect(await screen.findByText("stockAnalysis.settings.vendors.fallbackFailed")).toBeTruthy();
    expect(screen.queryByText("stockAnalysis.settings.vendors.fallbackUnavailable")).toBeNull();
  });

  it("确有记录 ⇒ 显示「首个健康候选 → 实际供数」，并把条数交给 scope 文案", async () => {
    mountTab(() =>
      Promise.resolve([
        {
          dataType: "news",
          stockCode: "300285",
          primaryVendor: "xueqiu",
          usedVendor: "eastmoney",
          reason: "primary_failed",
          timestamp: 1760000000000,
        },
      ])
    );
    expect(await screen.findByText("xueqiu → eastmoney")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.settings.vendors.fallbackScope|count=1")).toBeTruthy();
    // reason 恒为 primary_failed ⇒ 它不得出现在界面上冒充失败原因
    expect(screen.queryByText(/primary_failed/)).toBeNull();
  });
});
