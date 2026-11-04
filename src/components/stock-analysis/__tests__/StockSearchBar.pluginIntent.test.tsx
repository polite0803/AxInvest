// SPDX-License-Identifier: AGPL-3.0-only

/**
 * B4-j 回归：搜索框的 NL 意图解析**走插件命令**（宿主同名命令已删）。
 *
 * 钉住三条：
 * ① 命中声明了 `parse_analysis_intent` 的启用插件 ⇒ 以 `plugin_execute_command` 调用，
 *    `pluginId` 来自 `plugin_list`（不写死）；
 * ② 回执里的代码直接填进 store，并立刻取行情（与旧宿主命令路径的可观测行为一致）；
 * ③ 插件调用失败 ⇒ 退回普通搜索（不抛到界面、不静默吞掉搜索本身）。
 */
import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StockSearchBar } from "../StockSearchBar";

const invokeMock = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function renderBar() {
  return render(
    <I18nextProvider i18n={i18n}>
      <App>
        <StockSearchBar />
      </App>
    </I18nextProvider>,
  );
}

const getQuoteSpy = vi.fn().mockResolvedValue(undefined);
const getKlineSpy = vi.fn().mockResolvedValue(undefined);
const searchStockSpy = vi.fn().mockResolvedValue(undefined);

beforeEach(() => {
  invokeMock.mockReset();
  useStockAnalysisStore.getState().reset();
  getQuoteSpy.mockClear();
  getKlineSpy.mockClear();
  searchStockSpy.mockClear();
  useStockAnalysisStore.setState({
    getStockQuote: getQuoteSpy as never,
    getStockKline: getKlineSpy as never,
    searchStock: searchStockSpy as never,
  });
});

/**
 * 触发回车提交。`data-testid` 可能落在 antd `Input.Search` 的包装节点上，
 * 因此取其中的 `<input>`；`userEvent.type` 在这个受控输入上不发 `onSearch`（实测），
 * 改用 `change` + `keyDown(Enter)`（antd 的 `onPressEnter` 就挂在 keyDown 上）。
 */
async function submit(text: string) {
  const host = screen.getByTestId("stock-analysis-search-input");
  const input = (host.tagName === "INPUT" ? host : host.querySelector("input")) as HTMLInputElement;
  fireEvent.change(input, { target: { value: text } });
  await act(async () => {
    fireEvent.keyDown(input, { key: "Enter", code: "Enter", keyCode: 13 });
  });
}

describe("StockSearchBar — NL 意图解析走插件命令", () => {
  it("按 plugin_list 找到启用插件 ⇒ 用它的 pluginId 调 parse_analysis_intent", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "plugin_list") {
        return [
          {
            id: "stock@external",
            name: "stock",
            enabled: false,
            commands: [{ name: "parse_analysis_intent", description: "" }],
          },
          {
            id: "stock@bundled",
            name: "stock",
            enabled: true,
            commands: [{ name: "parse_analysis_intent", description: "" }],
          },
        ];
      }
      if (cmd === "plugin_execute_command") {
        return JSON.stringify({
          rawInput: "调研宁德时代中线",
          stockQuery: "宁德时代",
          stockCode: "300750",
          timeHorizon: "mid",
          actionType: "research",
          success: true,
          description: "热门词命中",
        });
      }
      return undefined;
    });

    renderBar();
    await submit("调研宁德时代中线");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("plugin_execute_command", {
        pluginId: "stock@bundled",
        name: "parse_analysis_intent",
        input: { input: "调研宁德时代中线" },
      });
    });
    expect(useStockAnalysisStore.getState().stockCode).toBe("300750");
    expect(getQuoteSpy).toHaveBeenCalledWith("300750");
    // 注意：onChange 本身也会调 `searchStock(value)`（单参），所以这里只能钉
    // 「没有走回退分支」的那个双参形态。
    expect(searchStockSpy).not.toHaveBeenCalledWith("调研宁德时代中线", true);
  });

  it("插件命令失败 ⇒ 退回普通搜索", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "plugin_list") {
        return [{
          id: "stock@bundled",
          name: "stock",
          enabled: true,
          commands: [{ name: "parse_analysis_intent", description: "" }],
        }];
      }
      if (cmd === "plugin_execute_command") {
        throw new Error("plugin crashed");
      }
      return undefined;
    });

    renderBar();
    await submit("茅台");

    await waitFor(() => expect(searchStockSpy).toHaveBeenCalledWith("茅台", true));
    expect(getQuoteSpy).not.toHaveBeenCalled();
  });
});
