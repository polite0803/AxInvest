// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件命令反查：三条判据
 * ① 只有**启用**的插件参与匹配（禁用插件声明了同名命令也不能用）；
 * ② 同一命令只枚举一次（`plugin_list` 会触发内置插件落盘同步，不是纯内存读）；
 * ③ 枚举失败不缓存（宿主未就绪时下次重试）。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

function plugin(id: string, enabled: boolean, commands: string[]) {
  return { id, name: id, enabled, commands: commands.map((name) => ({ name, description: "" })) };
}

/** 每次拿一份**全新模块**：`resolved` 是模块级缓存，跨用例必须隔离。 */
async function load() {
  vi.resetModules();
  const mod = await import("../pluginCommandResolve");
  return mod.resolvePluginForCommand;
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("resolvePluginForCommand", () => {
  it("跳过未启用插件，返回启用插件的 id", async () => {
    const resolve = await load();
    invokeMock.mockResolvedValue([
      plugin("stock@external", false, ["parse_analysis_intent"]),
      plugin("stock@bundled", true, ["parse_analysis_intent"]),
    ]);

    expect(await resolve("parse_analysis_intent")).toBe("stock@bundled");
    expect(invokeMock).toHaveBeenCalledWith("plugin_list");
  });

  it("同一命令只枚举一次（第二次走缓存，不再发 IPC）", async () => {
    const resolve = await load();
    invokeMock.mockResolvedValue([plugin("stock@bundled", true, ["parse_analysis_intent"])]);

    expect(await resolve("parse_analysis_intent")).toBe("stock@bundled");
    expect(await resolve("parse_analysis_intent")).toBe("stock@bundled");
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("没有插件声明该命令 ⇒ null", async () => {
    const resolve = await load();
    invokeMock.mockResolvedValue([plugin("stock@bundled", true, ["stock_get_quote"])]);

    expect(await resolve("parse_analysis_intent")).toBeNull();
  });

  it("枚举失败 ⇒ null，且不缓存（下次重试会再发 IPC）", async () => {
    const resolve = await load();
    invokeMock.mockRejectedValueOnce(new Error("host not ready"));
    invokeMock.mockResolvedValueOnce([plugin("stock@bundled", true, ["parse_analysis_intent"])]);

    expect(await resolve("parse_analysis_intent")).toBeNull();
    expect(await resolve("parse_analysis_intent")).toBe("stock@bundled");
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });
});
