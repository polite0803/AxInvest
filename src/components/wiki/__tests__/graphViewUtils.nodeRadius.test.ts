// 锁住 nodeDrawRadius 的契约（2026-09-16）
//
// 背景（归档缺陷）：节点此前只按**世界坐标半径**绘制（nodeSizeRef，默认 5）。
// 相机 fit 全图时 zoom 被压到 0.047~0.13 ⇒ 屏幕半径 0.24~0.6px（亚像素）
// ⇒ 24288 个点被抗锯齿稀释成 meanChroma≈8.6 的灰雾。
// 实测「关掉聚类模式」主画布 chroma>30 占比仅 **0.002%**、色相桶 0/36（肉眼即"完全空白"）。
//   ⚠ 早期记录的 0.14% 是**旧 crop（含 minimap）**测出的数字，不可比 —— 口径一变，
//     基线必须同口径重算（判据 #313）。
//
// ⚠ 这里刻意不只测「小 zoom 时有下限」：只断言函数自身会放过「函数修好了、
// 但调用点还在用裸世界半径」的实现（本缺陷正是靠调用点未被修正而存活的）。
// 所以额外把两条**跨侧一致性**不变量钉死：
//   · 屏幕半径恒 ≥ MIN_NODE_SCREEN_RADIUS —— 否则"下限"名不副实；
//   · zoom ≥ 0.4 时对默认尺寸**不生效** —— 否则会破坏「节点随缩放正常变大」的既有观感。
import { describe, expect, it } from "vitest";
import { MIN_NODE_SCREEN_RADIUS, nodeDrawRadius } from "../graphViewUtils";

// nodeSizeRef 的默认值（GraphView.tsx 里 `|| 5`）
const DEFAULT_NODE_WORLD_SIZE = 5;

describe("nodeDrawRadius", () => {
  it("屏幕半径恒 ≥ 下限（含极端退化 zoom）", () => {
    for (const z of [1e-6, 1e-4, 1e-3, 0.01, 0.05, 0.126, 0.2, 0.4, 1, 10]) {
      const r = nodeDrawRadius(DEFAULT_NODE_WORLD_SIZE, z);
      expect(Number.isFinite(r)).toBe(true);
      expect(r * z).toBeGreaterThanOrEqual(MIN_NODE_SCREEN_RADIUS - 1e-6);
    }
  });

  it("zoom ≥ 0.4 时不生效 —— 这条注释断言必须可证，否则会改掉放大后的既有观感", () => {
    for (const z of [0.4, 0.45, 0.5, 1, 2, 8]) {
      expect(nodeDrawRadius(DEFAULT_NODE_WORLD_SIZE, z)).toBe(DEFAULT_NODE_WORLD_SIZE);
    }
  });

  it("zoom < 0.4 时提升到屏幕下限（这正是「关掉聚类不空白」的量化依据）", () => {
    const z = 0.126; // 实测默认态的相机 zoom
    const r = nodeDrawRadius(DEFAULT_NODE_WORLD_SIZE, z);
    expect(r).toBeGreaterThan(DEFAULT_NODE_WORLD_SIZE);
    expect(r * z).toBeCloseTo(MIN_NODE_SCREEN_RADIUS, 6);
  });

  it("世界半径已大于下限时原样返回（只放大、绝不缩小）", () => {
    expect(nodeDrawRadius(50, 0.1)).toBe(50);
    expect(nodeDrawRadius(200, 0.05)).toBe(200);
  });

  it("zoom 增大 ⇒ 世界半径单调不增（屏幕半径单调增，不出现反向跳变）", () => {
    let prev = Infinity;
    for (const z of [0.01, 0.05, 0.1, 0.2, 0.3, 0.4, 0.6, 1, 3]) {
      const r = nodeDrawRadius(DEFAULT_NODE_WORLD_SIZE, z);
      expect(r).toBeLessThanOrEqual(prev + 1e-9);
      prev = r;
    }
  });

  it("退化 zoom（0 / 负数 / EPSILON）不产生 NaN 或 Infinity", () => {
    for (const z of [0, -1, Number.EPSILON]) {
      const r = nodeDrawRadius(DEFAULT_NODE_WORLD_SIZE, z);
      expect(Number.isFinite(r)).toBe(true);
    }
  });
});
