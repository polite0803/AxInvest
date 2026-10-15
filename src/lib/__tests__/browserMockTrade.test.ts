import { handleCommand } from "@/lib/browserMock";
import { describe, expect, it } from "vitest";

// 浏览器模式（`npm run dev`）走这份 mock。此前交易面板用到的命令全落到 default 兜底：
// `get_` 开头一律返回 `{}`、其余返回 `undefined` ⇒ 组合面板把 `{}` 当成有数据
// （它判 `typeof === "object"`）再往下读字段就是 TypeError，整个 tab 被错误边界兜住。
describe("browserMock 覆盖交易面板用到的命令", () => {
  // 关键判据：必须是 null，不能是 {} —— 面板用「非 null 且是 object」判有没有数据
  it("get_portfolio_dashboard 返回 null 而不是 {}", async () => {
    await expect(handleCommand("get_portfolio_dashboard", {})).resolves.toBeNull();
  });

  it("列表类命令返回数组而不是 undefined（调用方会读 .length）", async () => {
    for (const cmd of ["list_trades", "get_trade_positions", "parse_trades_csv"]) {
      const value = await handleCommand(cmd, {});
      expect(Array.isArray(value), `${cmd} 应返回数组，实际 ${typeof value}`).toBe(true);
    }
  });

  it("get_trading_enabled 返回布尔（面板按 typeof 决定是否采用）", async () => {
    const value = await handleCommand("get_trading_enabled", {});
    expect(typeof value).toBe("boolean");
  });

  it("import_trades 返回带 valid / failed / errors 的摘要结构", async () => {
    const summary = await handleCommand<Record<string, unknown>>("import_trades", {});
    expect(typeof summary.valid).toBe("number");
    expect(typeof summary.failed).toBe("number");
    expect(Array.isArray(summary.errors)).toBe(true);
  });

  // 负控：上面四条若不生效，兜底会给 get_ 返回 {}（object，非 null）——
  // 这条锁住「兜底本身仍在」，避免上面四条因兜底被改而失去意义。
  it("负控：未登记的命令仍走兜底（证明上面的显式 case 确实在起作用）", async () => {
    const value = await handleCommand("get_some_unregistered_probe", {});
    expect(value).toEqual({});
  });
});
