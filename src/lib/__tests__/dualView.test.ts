import {
  _resetDualViewRegistry,
  getDualView,
  isDualViewEnabled,
  listDualViews,
  registerDualView,
} from "@/lib/dualView";
import { afterEach, describe, expect, it, vi } from "vitest";

describe("dualView registry", () => {
  afterEach(() => _resetDualViewRegistry());

  it("registerDualView 存储并允许 getDualView 读取", () => {
    registerDualView({
      id: "test-1",
      title: "Test 1",
      icon: "Box",
      defaultTab: "analyze",
      compact: () => "compact",
      panel: () => "panel",
    });
    expect(getDualView("test-1")?.title).toBe("Test 1");
  });

  it("重复注册同 id 给出警告", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    registerDualView({
      id: "dup",
      title: "A",
      icon: "Box",
      defaultTab: "analyze",
      compact: () => "a",
      panel: () => "a",
    });
    registerDualView({
      id: "dup",
      title: "B",
      icon: "Box",
      defaultTab: "analyze",
      compact: () => "b",
      panel: () => "b",
    });
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("duplicate registration"));
    expect(getDualView("dup")?.title).toBe("B"); // 后注册的覆盖
    warn.mockRestore();
  });

  /**
   * 2026-10-29 修复回归：HMR 重求值 ≠ 真冲突。
   *
   * 原实现只看 `registry.has(id)` 就告警 ⇒ Vite 热更新任一 `xxxDualView.tsx`
   * 或其依赖（如 `RecommendationPanel.tsx`）时，该模块被重新求值、`registerDualView`
   * 再跑一次，控制台就刷出 `[dualView] duplicate registration: screener`。
   * 但全仓 `id: "screener"` 只有一处，声明属性逐字相同 ⇒ 纯属噪音。
   *
   * 判据是「声明属性全等」，**不是**「compact/panel 引用相同」—— HMR 必然重建闭包，
   * 拿引用比对会把每次热更新都判成冲突。下面第二条用例正是锁这一点。
   */
  it("同声明属性重注册（HMR 重求值）不告警，且函数引用换新也照样静默", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const make = () => ({
      id: "hmr",
      title: "HMR",
      icon: "Filter",
      defaultTab: "market" as const,
      // 每次调用都产出**新引用** —— 模拟 HMR 重建闭包
      compact: () => "compact",
      panel: () => "panel",
    });
    registerDualView(make());
    registerDualView(make());
    expect(warn).not.toHaveBeenCalled();
    expect(getDualView("hmr")?.title).toBe("HMR");
    warn.mockRestore();
  });

  it("声明属性有任一不同 ⇒ 仍按真冲突告警", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    registerDualView({
      id: "conflict",
      title: "A",
      icon: "Box",
      defaultTab: "analyze",
      compact: () => "a",
      panel: () => "a",
    });
    // 只改 defaultTab，其余相同：仍必须告警（否则两个面板的同名视图会静默互顶）
    registerDualView({
      id: "conflict",
      title: "A",
      icon: "Box",
      defaultTab: "market",
      compact: () => "b",
      panel: () => "b",
    });
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("duplicate registration"));
    warn.mockRestore();
  });

  it("isDualViewEnabled 区分 noDualView 标记", () => {
    registerDualView({
      id: "blocked",
      title: "X",
      icon: "Box",
      defaultTab: "analyze",
      compact: () => "",
      panel: () => "",
      noDualView: true,
    });
    expect(isDualViewEnabled("blocked")).toBe(false);
  });

  it("isDualViewEnabled 未注册 id 返回 false", () => {
    expect(isDualViewEnabled("nonexistent")).toBe(false);
  });

  it("listDualViews 返回所有已注册", () => {
    registerDualView({ id: "a", title: "A", icon: "Box", defaultTab: "analyze", compact: () => "", panel: () => "" });
    registerDualView({ id: "b", title: "B", icon: "Box", defaultTab: "analyze", compact: () => "", panel: () => "" });
    expect(listDualViews()).toHaveLength(2);
  });

  it("_resetDualViewRegistry 清空注册表", () => {
    registerDualView({ id: "x", title: "X", icon: "Box", defaultTab: "analyze", compact: () => "", panel: () => "" });
    _resetDualViewRegistry();
    expect(getDualView("x")).toBeUndefined();
  });
});
