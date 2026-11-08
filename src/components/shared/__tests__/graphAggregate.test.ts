// 锁住聚合布局纯函数层的契约（2026-09-16）
//
// 为什么这些断言值得存在：`buildAggregateGraph` / `normalizeAggregateScale` 是
// 「聚合物理是否真的激活」「社区是否真的被分开」这两件事的**唯一**施工点，
// 而它们的调用方（GraphView 的 ref 世界）极难在测试里复现。把它们抽成纯函数后，
// 契约可以在这里被钉死，参数标定也才有可能用**同一份代码**跑（否则标定脚本只能重抄一遍）。
//
// 断言取向：只写「改了就会出可见缺陷」的性质，不重复实现内部逻辑。
import { describe, expect, it } from "vitest";
import {
  AGG_PHYSICS_CONFIG,
  aggregateSeedRadius,
  buildAggregateGraph,
  countMembers,
  normalizeAggregateScale,
} from "../graphAggregate";
import type { PhysicsNode } from "../graphPhysics";
import {
  BIG_GRAPH_SPRITE_MIN_WORLD_RADIUS,
  hashStringToInt,
  MIN_NODE_SCREEN_RADIUS,
  SPRITE_BAKE_ZOOM,
  SPRITE_MAX_ZOOM,
  spriteUsableAtZoom,
} from "../graphViewUtils";

function node(id: string, x = 0, y = 0, mass = 1): PhysicsNode {
  return { id, x, y, vx: 0, vy: 0, fx: 0, fy: 0, mass, fixed: false, kind: "note", idx: 0 };
}

/**
 * 播种力参数：**直接引用生产常量**，不手抄。
 *
 * ⚠ 2026-09-18 修正：此处原为手抄 `{ repulsion: 600, gravity: 8 }` 并附注释
 *   「与生产 `AGG_PHYSICS_CONFIG` 同值」—— 而生产值已在 2026-09-17（③-D 标定）
 *   改为 `repulsion: 5400`，注释变成了假话，本文件的形状判据也就一直跑在
 *   **生产不存在的参数**上（手抄常量必然腐烂：`GraphView.tsx` 里同一批值也另有一份）。
 *   改为引用后，参数标定与形状回归用的是**同一个来源**。
 *   （断言全部以 `TEST_PHYSICS` / `rStar` 自身为基准相对推导，故换值不改变判据语义。）
 */
const TEST_PHYSICS = { repulsion: AGG_PHYSICS_CONFIG.repulsion, gravity: AGG_PHYSICS_CONFIG.gravity };

describe("buildAggregateGraph", () => {
  // 2 个社区：A={a1,a2,a3}、B={b1,b2}，外加 1 个无社区节点 loose
  const nodes = [node("a1"), node("a2"), node("a3"), node("b1"), node("b2"), node("loose")];
  const communities = new Map([
    ["a1", 10],
    ["a2", 10],
    ["a3", 10],
    ["b1", 20],
    ["b2", 20],
  ]);
  const edges = [
    { source: "a1", target: "a2" }, // 单元内 → 丢弃
    { source: "a1", target: "b1" }, // A–B → 保留
    { source: "a2", target: "b2" }, // A–B → 与上一条去重合并
    { source: "a3", target: "loose" }, // A–loose
  ];

  it("每个布局单元 → 恰 1 个聚合节点，质量 = max(1, 成员数 × 0.6)", () => {
    const built = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10, 20]),
      seedPhysics: TEST_PHYSICS,
    });
    const agg = built.nodes.filter((n) => n.id.startsWith("__agg__"));
    expect(agg.length).toBe(2);
    expect(agg.find((n) => n.id === "__agg__10")!.mass).toBeCloseTo(3 * 0.6, 9);
    expect(agg.find((n) => n.id === "__agg__20")!.mass).toBeCloseTo(2 * 0.6, 9);
    expect(built.cidToNodeIdx.size).toBe(2);
  });

  it("不在布局单元内的真实节点**按引用**并入（物理就地更新其坐标，绘制路径自动同步）", () => {
    const built = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10]),
      seedPhysics: TEST_PHYSICS,
    });
    const loose = built.nodes[built.nodes.length - 1];
    expect(loose).toBe(nodes[5]); // 同一对象身份，不是拷贝
    // 社区 20 未被折叠 ⇒ 它的成员也作为真实节点并入
    expect(built.nodes.includes(nodes[3])).toBe(true);
    expect(built.nodes.includes(nodes[4])).toBe(true);
  });

  it("聚合边：端点映射到布局单元后去重合并，restLength 取 140", () => {
    const built = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10, 20]),
      seedPhysics: TEST_PHYSICS,
    });
    // a1–b1 与 a2–b2 折叠成同一条 A–B 边；a1–a2 是单元内边（丢弃）；
    // a3–loose 保留（loose 未折叠）
    const ab = built.edges.find((e) =>
      (e.source === "__agg__10" && e.target === "__agg__20")
      || (e.source === "__agg__20" && e.target === "__agg__10")
    );
    expect(ab).toBeDefined();
    expect(ab!.restLength).toBe(140);
    expect(built.edges.length).toBe(2);
    for (const e of built.edges) {
      expect(e.sourceIdx).toBe(built.nodes.findIndex((n) => n.id === e.source));
      expect(e.targetIdx).toBe(built.nodes.findIndex((n) => n.id === e.target));
    }
  });

  it("布局单元内的边被丢弃（不会退化成自环）", () => {
    const built = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10, 20]),
      seedPhysics: TEST_PHYSICS,
    });
    for (const e of built.edges) { expect(e.sourceIdx).not.toBe(e.targetIdx); }
  });

  it("聚合节点初速**非零** —— 否则 stepPhysics 的 anyMoving 兜底会让聚合物理永远不启动", () => {
    const built = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10, 20]),
      seedPhysics: TEST_PHYSICS,
    });
    for (const n of built.nodes) {
      if (!n.id.startsWith("__agg__")) { continue; }
      expect(Math.hypot(n.vx, n.vy)).toBeGreaterThan(0.01);
    }
  });

  it("同一 cid 的初速方向是**确定性**的（重建后朝向不变，不会每帧抖动）", () => {
    const a = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10, 20]),
      seedPhysics: TEST_PHYSICS,
    });
    const b = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10, 20]),
      seedPhysics: TEST_PHYSICS,
    });
    expect(a.nodes.map((n) => [n.x, n.y, n.vx, n.vy])).toEqual(b.nodes.map((n) => [n.x, n.y, n.vx, n.vy]));
  });

  it("centroidOf 提供质心时用作初位置（否则退回**按 R* 的圆盘**播种）", () => {
    const withCentroid = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10]),
      centroidOf: () => ({ cx: 777, cy: -333 }),
      seedPhysics: TEST_PHYSICS,
    });
    expect(withCentroid.nodes[0].x).toBe(777);
    expect(withCentroid.nodes[0].y).toBe(-333);
    const without = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10]),
      seedPhysics: TEST_PHYSICS,
    });
    // 社区 10 = 3 成员 ⇒ 质量 max(1, 3×0.6) = 1.8 ⇒ R* = √(repulsion×1.8/gravity)
    // （具体值与生产常量联动，故不写死数字，一律由 aggregateSeedRadius 现算）
    const rStar = aggregateSeedRadius(TEST_PHYSICS.repulsion, TEST_PHYSICS.gravity, 1.8);
    const r0 = Math.hypot(without.nodes[0].x, without.nodes[0].y);
    expect(r0).toBeLessThanOrEqual(rStar + 1e-9);
    expect(Math.abs(without.nodes[0].x)).toBeLessThanOrEqual(rStar + 1e-9);
    expect(Math.abs(without.nodes[0].y)).toBeLessThanOrEqual(rStar + 1e-9);
  });

  it("countMembers 与建图共用同一口径（真实成员数，不经 max(1,·) 夹取）", () => {
    const counted = countMembers(nodes, communities);
    expect(counted.get(10)).toBe(3);
    expect(counted.get(20)).toBe(2);
    const built = buildAggregateGraph({
      nodes,
      edges,
      communities,
      layoutUnits: new Set([10, 20]),
      seedPhysics: TEST_PHYSICS,
    });
    expect(built.memberCount.get(10)).toBe(counted.get(10));
  });
});

describe("聚合播种：R* 圆盘（旧实现是 r=400 圆环 ⇒ 首屏中心空白、约 40s 才成形）", () => {
  // 逼近生产规模：200 桶 × 73 成员（fixture 实测 200 桶 / 平均质量 72.9）
  const COMPS = 200;
  const MEMBERS = 73;
  const manyNodes: PhysicsNode[] = [];
  const manyCommunities = new Map<string, number>();
  for (let c = 0; c < COMPS; c++) {
    for (let m = 0; m < MEMBERS; m++) {
      const id = `c${c}n${m}`;
      manyNodes.push(node(id));
      manyCommunities.set(id, c);
    }
  }
  const massPerUnit = Math.max(1, MEMBERS * 0.6);
  const rStar = aggregateSeedRadius(TEST_PHYSICS.repulsion, TEST_PHYSICS.gravity, COMPS * massPerUnit);
  const built = buildAggregateGraph({
    nodes: manyNodes,
    edges: [],
    communities: manyCommunities,
    layoutUnits: new Set(Array.from({ length: COMPS }, (_, i) => i)),
    seedPhysics: TEST_PHYSICS,
  });
  const radii = built.nodes.map((n) => Math.hypot(n.x, n.y)).sort((a, b) => a - b);

  it("R* 与参数一致（√(rep·M/g)；M 用与建图同口径的 max(1, 成员数×0.6)）", () => {
    expect(rStar).toBeCloseTo(Math.sqrt((TEST_PHYSICS.repulsion * COMPS * massPerUnit) / TEST_PHYSICS.gravity), 9);
    expect(rStar).toBeGreaterThan(500);
  });

  it("**中位半径 ≈ 0.707·R***（面积均匀圆盘的特征值；若是圆环则为 1.00·R*）", () => {
    const median = radii[Math.floor(radii.length / 2)];
    expect(median / rStar).toBeGreaterThan(0.62);
    expect(median / rStar).toBeLessThan(0.79);
  });

  it("存在内圈（最小半径 ≪ R*）—— 这才是「中心不空白」的直接体现", () => {
    expect(radii[0] / rStar).toBeLessThan(0.15);
  });

  it("全部落在 R* 内（没人被播到外圈 ⇒ 不会一开始就承受外向净斥力）", () => {
    expect(radii[radii.length - 1]).toBeLessThanOrEqual(rStar * (1 + 1e-9));
  });

  it("退化输入（M=0 / 参数非正）⇒ 半径 0（落原点），不抛", () => {
    expect(aggregateSeedRadius(TEST_PHYSICS.repulsion, TEST_PHYSICS.gravity, 0)).toBe(0);
    expect(aggregateSeedRadius(0, TEST_PHYSICS.gravity, 100)).toBe(0);
    expect(aggregateSeedRadius(TEST_PHYSICS.repulsion, 0, 100)).toBe(0);
    expect(aggregateSeedRadius(-1, -1, -1)).toBe(0);
  });
});

describe("normalizeAggregateScale", () => {
  it("超界 ⇒ 等比缩到 halfSpan（按 L∞ 度量），返回实际因子", () => {
    const ns = [node("a", 3000, 0), node("b", 0, -1000), node("c", -100, 50)];
    const f = normalizeAggregateScale(ns, 1500);
    expect(f).toBeCloseTo(0.5, 9);
    expect(Math.max(...ns.map((n) => Math.max(Math.abs(n.x), Math.abs(n.y))))).toBeCloseTo(1500, 6);
    expect(ns[0].x).toBeCloseTo(1500, 6);
    expect(ns[1].y).toBeCloseTo(-500, 6);
  });

  it("速度**同比**缩放 —— 不缩放会形成「每帧缩回 ↔ 每帧膨胀」的对抗（持续抖动）", () => {
    const ns = [node("a", 3000, 0), node("b", 0, -1000)];
    ns[0].vx = 100;
    ns[0].vy = -50;
    const f = normalizeAggregateScale(ns, 1500);
    expect(ns[0].vx).toBeCloseTo(100 * f, 9);
    expect(ns[0].vy).toBeCloseTo(-50 * f, 9);
  });

  it("未超界 ⇒ 因子为 1 且**不改动任何坐标**（只缩小、绝不放大）", () => {
    const ns = [node("a", 100, 200), node("b", -300, 50)];
    const before = ns.map((n) => [n.x, n.y]);
    expect(normalizeAggregateScale(ns, 1500)).toBe(1);
    expect(ns.map((n) => [n.x, n.y])).toEqual(before);
  });

  it("等比 ⇒ 形状完全保留（任意两点的距离之比不变）", () => {
    const ns = [node("a", 5000, 0), node("b", 0, 4000), node("c", -1000, -1000)];
    const ratioBefore = Math.hypot(ns[0].x - ns[1].x, ns[0].y - ns[1].y)
      / Math.hypot(ns[1].x - ns[2].x, ns[1].y - ns[2].y);
    normalizeAggregateScale(ns, 1500);
    const ratioAfter = Math.hypot(ns[0].x - ns[1].x, ns[0].y - ns[1].y)
      / Math.hypot(ns[1].x - ns[2].x, ns[1].y - ns[2].y);
    expect(ratioAfter).toBeCloseTo(ratioBefore, 9);
  });

  it("退化输入（空数组 / 全零点）安全：全零点在本模型里是**合法**布局（原点即平衡）", () => {
    expect(normalizeAggregateScale([], 1500)).toBe(1);
    const ns = [node("a", 0, 0)];
    expect(normalizeAggregateScale(ns, 1500)).toBe(1);
    expect(Number.isFinite(ns[0].x)).toBe(true);
  });
});

describe("spriteUsableAtZoom（区间判据：**fit 态必须可用** —— 修「收益区间与可用区间不相交」）", () => {
  it("区间两端精确翻转", () => {
    expect(spriteUsableAtZoom(SPRITE_BAKE_ZOOM)).toBe(true);
    expect(spriteUsableAtZoom(SPRITE_BAKE_ZOOM * 0.99)).toBe(false);
    expect(spriteUsableAtZoom(SPRITE_MAX_ZOOM)).toBe(true);
    expect(spriteUsableAtZoom(SPRITE_MAX_ZOOM * 1.01)).toBe(false);
  });

  it("**收益区间必须判 true**：生产实测的 fit 全图 zoom", () => {
    // ⚠ 0.323 已从这组里**移出**（2026-09-17 ③-D）：`SPRITE_MAX_ZOOM` 随
    //   `SPRITE_BAKE_ZOOM` 由 0.4 下调到 0.30，而 0.323 是改前的 fit zoom ——
    //   它本就不在「2.4 万节点全在视口内」这个收益区间里（zoom 越大，视口内节点越少；
    //   0.323 时候选仅约 4.5 千 ⇒ 矢量路径每帧约 4.5 千个 arc，不构成性能黑洞）。
    //   收益区间的定义是「fit 全图 ⇒ 全量节点都在视口内」，对应的是**低** zoom 端。
    // 0.14 = ③-D 之后的实测 fit zoom（probe-d3d-0917），即本次必须覆盖的那个点。
    for (const z of [0.14, 0.2046, 0.25]) {
      expect(spriteUsableAtZoom(z)).toBe(true);
    }
  });

  it("烘制下限在区间内**确实成立**（这是「把屏幕下限烘进位图」的算术自证）", () => {
    // 位图落屏的缩放比恰好是 zoom ⇒ 烘入半径 × zoom 就是屏幕半径。
    // 区间内任一点都必须 ≥ 屏幕下限，否则「位图不再糊成灰雾」只是口号。
    // ⚠ 容差 1e-9 是必须的：`MIN/BAKE × BAKE` 在 IEEE754 下是 `1.9999999999999998`
    //   （0.18 非二进制精确值），严格 `>= 2` 会**假红**。这是「先把测量工具排除掉」
    //   的典型场景 —— 恒等式在浮点下不成立 ≠ 机制有问题。
    for (const z of [SPRITE_BAKE_ZOOM, 0.25, SPRITE_MAX_ZOOM]) {
      expect(BIG_GRAPH_SPRITE_MIN_WORLD_RADIUS * z).toBeGreaterThanOrEqual(MIN_NODE_SCREEN_RADIUS - 1e-9);
    }
  });

  it("低 zoom 失效区间仍判 false —— 那正是「关掉聚类整屏空白」的 zoom 区间", () => {
    // ⚠ 0.126 原本也在这组里，但 `SPRITE_BAKE_ZOOM` 下调到 0.12 后它已进入可用区间。
    //   这是**预期的**：下调阈值的目的就是让位图覆盖到更低的 zoom（本次 fit zoom 0.14）。
    //   下限改用常量表述，避免与 `SPRITE_BAKE_ZOOM` 脱钩。
    for (const z of [0.047, SPRITE_BAKE_ZOOM * 0.9]) {
      expect(spriteUsableAtZoom(z)).toBe(false);
    }
  });

  it("退化 zoom（0 / 负数 / NaN）判 false（fail-closed：宁可走矢量路径）", () => {
    expect(spriteUsableAtZoom(0)).toBe(false);
    expect(spriteUsableAtZoom(-1)).toBe(false);
    expect(spriteUsableAtZoom(Number.NaN)).toBe(false);
  });
});

describe("hashStringToInt（自 GraphView 迁出后仍是同一实现）", () => {
  it("确定性 + 同串同值", () => {
    expect(hashStringToInt("agg:10")).toBe(hashStringToInt("agg:10"));
    expect(hashStringToInt("agg:10")).not.toBe(hashStringToInt("agg:11"));
  });
});
