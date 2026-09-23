import { describe, expect, it } from "vitest";
import { handleCommand } from "../browserMock";
import { hasIpcSchema, IpcSchemaError, validateIpcResult } from "../ipc-schemas";

/**
 * D1（IPC 边界 zod 运行时校验）的回归锁。
 *
 * 分两层：
 *   ① 契约层 —— 证明校验**会告警**（正控），而不是恒放行；
 *   ② 端到端层 —— 证明 **browserMock 的返回值能过契约**。这一层是真正有价值的部分：
 *      契约与 mock 是两处独立维护的代码，任一侧漂移都会让浏览器模式从
 *      「静默无数据」变成「抛 IpcSchemaError」。2026-09-19 首次接入时正是如此 ——
 *      `get_stock_analysis` 落到 default 的 `get_` → `{}` 兜底，`{}` 不含 `id` 被判违约。
 */
describe("IPC 契约校验（D1）", () => {
  describe("get_stock_analysis", () => {
    it("形状合法 ⇒ 放行", () => {
      expect(() => validateIpcResult("get_stock_analysis", { id: "a1" })).not.toThrow();
    });

    it("缺 id ⇒ 抛 IpcSchemaError（正控：证明校验会告警）", () => {
      expect(() => validateIpcResult("get_stock_analysis", {})).toThrow(IpcSchemaError);
    });

    it("返回 null / 非对象 ⇒ 抛", () => {
      expect(() => validateIpcResult("get_stock_analysis", null)).toThrow(IpcSchemaError);
      expect(() => validateIpcResult("get_stock_analysis", [])).toThrow(IpcSchemaError);
    });

    it("错误里带命令名与字段路径，便于定位（不是裸 zod 报错）", () => {
      try {
        validateIpcResult("get_stock_analysis", {});
        throw new Error("应当抛错但没有");
      } catch (e) {
        expect(e).toBeInstanceOf(IpcSchemaError);
        const err = e as IpcSchemaError;
        expect(err.cmdName).toBe("get_stock_analysis");
        expect(err.message).toContain("get_stock_analysis");
      }
    });
  });

  describe("list_stock_analyses", () => {
    it("空数组 ⇒ 放行", () => {
      expect(() => validateIpcResult("list_stock_analyses", [])).not.toThrow();
    });

    it("合法列表项 ⇒ 放行", () => {
      expect(() =>
        validateIpcResult("list_stock_analyses", [
          { id: "a1", stockCode: "600519", status: "completed" },
          { id: "a2", stockCode: "000001", status: "running" },
        ])
      ).not.toThrow();
    });

    it("列表项缺必需字段 ⇒ 抛（正控）", () => {
      expect(() => validateIpcResult("list_stock_analyses", [{}])).toThrow(IpcSchemaError);
      expect(() => validateIpcResult("list_stock_analyses", [{ id: "a1", stockCode: "600519" }])).toThrow(
        IpcSchemaError,
      );
    });

    it("返回对象而非数组 ⇒ 抛（这正是 mock 层 get_ 兜底 `{}` 的形态）", () => {
      expect(() => validateIpcResult("list_stock_analyses", {})).toThrow(IpcSchemaError);
    });
  });

  describe("未登记命令", () => {
    it("直接放行（渐进式接入：未登记即不拦，避免一次性铺开造成大面积误报）", () => {
      expect(hasIpcSchema("some_unregistered_cmd")).toBe(false);
      expect(() => validateIpcResult("some_unregistered_cmd", { anything: 1 })).not.toThrow();
      expect(() => validateIpcResult("some_unregistered_cmd", undefined)).not.toThrow();
    });
  });

  describe("端到端：browserMock 返回值必须过契约（两侧咬合的回归锁）", () => {
    it("get_stock_analysis 的 mock 返回形状合法", async () => {
      const res = await handleCommand("get_stock_analysis", { analysisId: "a1" });
      expect(() => validateIpcResult("get_stock_analysis", res)).not.toThrow();
    });

    it("list_stock_analyses 的 mock 返回形状合法（显式声明 ⇒ 空数组）", async () => {
      const res = await handleCommand("list_stock_analyses", { limit: 30, offset: 0 });
      expect(Array.isArray(res)).toBe(true);
      expect(() => validateIpcResult("list_stock_analyses", res)).not.toThrow();
    });
  });
});
