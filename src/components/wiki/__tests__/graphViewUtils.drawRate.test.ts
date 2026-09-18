// 锁住 viewportDrawRate 的契约（2026-09-17）
//
// 背景（归档缺陷「放大反而丢一半」）：节点层降采样此前是
//   const nodeSampleRate = isLargeGraph ? 0.5 : 1.0;   // isLargeGraph = nodes.length > 5000
// ⇒ 24288 节点的图**恒**只画 50%，与 zoom 无关。两层后果：
//   ① 放大后视口内候选已被 gridIndex 裁到几百个（远在预算内），却仍被砍一半 ——
//      纯损失，用户报「放大不到需要的细节」；
//   ② 位图区间（`spriteUsableAtZoom`，即 zoom ≤ `SPRITE_MAX_ZOOM`）是**无采样全量**烘制
//      （buildBigGraphSpriteCache 遍历 nodes 全量）⇒ 同一视图跨 SPRITE_MAX_ZOOM
//      两侧点数不同 = 视觉跳变。
// 边层同族且更硬：大图分支 `zoom < 0.3 ? 0.15 : zoom < 0.5 ? 0.3 : 0.5`
// ⇒ `zoom ≥ 0.5` 后**封顶 0.5**，放大到 5 倍也只有一半边，与节点层应有的
// 「放大后全画」正好相反。
//
// ⚠ 这里刻意不只测「预算内返回 1」：那只断言了函数自身，会放过「函数修好了、调用点
//   却把**全图**节点数当候选传进去」的实现 —— 本缺陷正是靠调用点取错量纲而存活的
//   （同族范式见 graphViewUtils.nodeRadius.test.ts 的注释）。所以额外钉死三条跨侧不变量：
//   · 超预算时「实绘数 = rate × candidateCount」恒 ≤ budget（否则「预算」名不副实）；
//   · **放大态的候选量级**（视口内，几百~几千）必须落在「不采样」区间；
//   · 单调性 —— 候选越多越稀，绝不出现反向跳变。
import { describe, expect, it } from "vitest";
import { EDGE_DRAW_BUDGET, NODE_DRAW_BUDGET, viewportDrawRate } from "../graphViewUtils";

// 本数据集实测值（24288 节点 / 74791 边 / fit 态 zoom 0.25，
// 逐帧归因见 output/verify-graph-2026-09-15）
const TOTAL_NODES = 24288;
const TOTAL_EDGES = 74791;
const FIT_ZOOM = 0.25;

/** 视口内候选数 ≈ 全图 × (fitZoom / zoom)²（均匀密度近似，用于把 zoom 换算成候选量级）。 */
const candidatesAtZoom = (zoom: number, total = TOTAL_NODES): number => total * Math.pow(FIT_ZOOM / zoom, 2);

describe("viewportDrawRate（判据钉在**候选数**上，不是全图规模）", () => {
  it("候选不超预算 ⇒ 1（全画）—— 这条就是「放大后不再丢一半」的判据", () => {
    for (const n of [1, 100, 600, 3000, 11999, NODE_DRAW_BUDGET]) {
      expect(viewportDrawRate(n, NODE_DRAW_BUDGET)).toBe(1);
    }
  });

  it("放大态的候选必须全画（旧判据在此恒得 0.5 —— 回归即红）", () => {
    // 放大到 zoom ≥ 0.45 时，视口内候选 ≈ 7500（< 12000）⇒ 必须全画。
    for (const zoom of [0.45, 0.6, 1, 2, 5]) {
      const candidates = candidatesAtZoom(zoom);
      expect(candidates).toBeLessThan(NODE_DRAW_BUDGET);
      expect(viewportDrawRate(candidates, NODE_DRAW_BUDGET)).toBe(1);
    }
  });

  it("fit 态定标：rate ≈ 0.494 —— 与改前恒 0.5 等价 ⇒ 性能不回退", () => {
    const rate = viewportDrawRate(TOTAL_NODES, NODE_DRAW_BUDGET);
    expect(rate).toBeCloseTo(NODE_DRAW_BUDGET / TOTAL_NODES, 10);
    // 与旧行为（0.5）的偏差必须 < 2%，否则 fit 态性能会前移
    expect(Math.abs(rate - 0.5)).toBeLessThan(0.02);
  });

  it("边层定标：fit 态 ≈ 0.16（改前实测 0.15，同量级）", () => {
    const ratio = 1; // fit 态 idSet.size ≈ posMap.size
    const rate = viewportDrawRate(TOTAL_EDGES * ratio, EDGE_DRAW_BUDGET);
    expect(rate).toBeCloseTo(0.1604, 3);
    expect(Math.abs(rate - 0.15)).toBeLessThan(0.02);
  });

  it("超预算时实绘数恒 ≤ budget（预算的语义本身）", () => {
    for (const n of [12001, 20000, 50000, 24288, 200000, 1e6]) {
      const rate = viewportDrawRate(n, NODE_DRAW_BUDGET);
      expect(rate).toBeLessThan(1);
      expect(rate * n).toBeLessThanOrEqual(NODE_DRAW_BUDGET + 1e-6);
    }
  });

  it("候选数单调增 ⇒ rate 单调不增（不出现反向跳变）", () => {
    let prev = Infinity;
    for (const n of [1, 100, 6000, 12000, 12001, 20000, 24288, 100000, 1e6]) {
      const rate = viewportDrawRate(n, NODE_DRAW_BUDGET);
      expect(rate).toBeLessThanOrEqual(prev + 1e-12);
      prev = rate;
    }
  });

  it("跨 SPRITE_MAX_ZOOM 两侧：**两侧都全画** ⇒ 跳变消失（这条对应位图区间的一致性）", () => {
    // 位图区间内（zoom ≤ 0.4）节点由位图无采样全量烘制；改后矢量侧同区间也全画。
    for (const zoom of [0.36, 0.39, 0.4, 0.41, 0.45]) {
      expect(viewportDrawRate(candidatesAtZoom(zoom), NODE_DRAW_BUDGET)).toBe(1);
    }
  });

  it("退化输入一律返回 1（fail-open：**画不出来比画得少更糟**）", () => {
    for (const n of [Number.NaN, Infinity, -Infinity, 0, -1]) {
      expect(viewportDrawRate(n, NODE_DRAW_BUDGET)).toBe(1);
    }
    // budget 非法时同样不启用采样（交给上层视口裁剪兜底）
    for (const b of [0, -1, Number.NaN, Infinity]) {
      expect(viewportDrawRate(TOTAL_NODES, b)).toBe(1);
    }
  });

  it("返回值为有限数且落在 (0, 1]（绝不产生 0 = 整层消失）", () => {
    for (const n of [1, 12000, 12001, 24288, 1e9]) {
      const rate = viewportDrawRate(n, NODE_DRAW_BUDGET);
      expect(Number.isFinite(rate)).toBe(true);
      expect(rate).toBeGreaterThan(0);
      expect(rate).toBeLessThanOrEqual(1);
    }
  });
});
