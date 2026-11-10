// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, isTauriMock, resolveMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  isTauriMock: vi.fn(),
  resolveMock: vi.fn(),
}));

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock, isTauri: isTauriMock }));
vi.mock("@/lib/pluginCommandResolve", () => ({ resolvePluginForCommand: resolveMock }));

import { invokePluginCommand } from "@/lib/pluginCommand";

beforeEach(() => {
  invokeMock.mockReset();
  isTauriMock.mockReset();
  resolveMock.mockReset();
});

describe("invokePluginCommand", () => {
  it("真机：按命令名反查插件后走 plugin_execute_command，并解析回执 JSON 文本", async () => {
    isTauriMock.mockReturnValue(true);
    resolveMock.mockResolvedValue("stock@bundled");
    invokeMock.mockResolvedValue(JSON.stringify([{ id: "run-1" }]));

    const rows = await invokePluginCommand<Array<{ id: string }>>("stock_get_pipeline_history", {
      limit: 20,
    });

    expect(resolveMock).toHaveBeenCalledWith("stock_get_pipeline_history");
    expect(invokeMock).toHaveBeenCalledWith("plugin_execute_command", {
      pluginId: "stock@bundled",
      name: "stock_get_pipeline_history",
      input: { limit: 20 },
    });
    expect(rows).toEqual([{ id: "run-1" }]);
  });

  it("真机：没有启用中的插件声明该命令 ⇒ 抛错且不派发（不得静默改写命令名）", async () => {
    isTauriMock.mockReturnValue(true);
    resolveMock.mockResolvedValue(null);

    await expect(invokePluginCommand("stock_get_pipeline_history")).rejects.toThrow(
      /no enabled plugin provides command/,
    );
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("真机：缺省入参传 {}（宿主命令签名是 Option<Value>，undefined 与空对象语义不同）", async () => {
    isTauriMock.mockReturnValue(true);
    resolveMock.mockResolvedValue("stock@bundled");
    invokeMock.mockResolvedValue("null");

    await expect(invokePluginCommand("stock_list_pending_evolution_plans")).resolves.toBeNull();
    expect(invokeMock).toHaveBeenCalledWith("plugin_execute_command", {
      pluginId: "stock@bundled",
      name: "stock_list_pending_evolution_plans",
      input: {},
    });
  });

  it("浏览器模式：直接把命令名派发给 mock，不查插件注册表（dev 无插件安装根）", async () => {
    isTauriMock.mockReturnValue(false);
    invokeMock.mockResolvedValue([]);

    await expect(invokePluginCommand("stock_get_pipeline_history", { limit: 5 })).resolves.toEqual(
      [],
    );
    expect(resolveMock).not.toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith("stock_get_pipeline_history", { limit: 5 });
  });
});
