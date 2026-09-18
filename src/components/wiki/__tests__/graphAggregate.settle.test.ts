// 收敛闸（AGG_SETTLE_PX / AGG_SETTLE_STEPS）的**契约测试 + 标定**（2026-09-17）
//
// 为什么要有这个文件（AUDIT §6.12.6-3 登记项）：该闸此前只有**一档**实测，且终态探针（t=160s）
// 里**观测不到触发** ⇒ 「阈值是否合适」在生产里没有观测点。要判定它，只有一条路：
// 用**生产同一份**常量 + **生产同一份**判据函数（`updateAggregateSettle`，2026-09-17 自
// GraphView 抽出），在**生产同规模**的图上跑到（或跑不到）静止。
//
// ⚠ 别为了「让闸触发」而调物理参数：那会把标定变成自证（判据 #329 同族）。
// ⚠ 判据函数必须是被测对象本身，不能在本文件里重抄一遍检测逻辑（判据 #7/#313）。
import { describe, expect, it } from "vitest";
import {
  AGG_ANNEAL_START_STEPS,
  AGG_LAYOUT_HALF_SPAN,
  AGG_PHYSICS_CONFIG,
  AGG_SETTLE_STEPS,
  aggregateSeedRadius,
  buildAggregateGraph,
  createAggregateAnnealState,
  createAggregateSettleState,
  normalizeAggregateScale,
  updateAggregateAnneal,
  updateAggregateSettle,
} from "../graphAggregate";
import { buildNeighborMap, stepPhysics } from "../graphPhysics";
import type { PhysicsNode } from "../graphPhysics";
import { communityRadius } from "../graphViewUtils";

/** 生产实测（harness A 阶段相机缩放，见 AUDIT §6.12.4 判据 ②）。 */
const A_STAGE_ZOOM = 0.2046;

function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6D2B79F5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function spanOf(nodes: ReadonlyArray<{ x: number; y: number }>): number {
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const n of nodes) {
    if (n.x < minX) { minX = n.x; }
    if (n.x > maxX) { maxX = n.x; }
    if (n.y < minY) { minY = n.y; }
    if (n.y > maxY) { maxY = n.y; }
  }
  return Math.max(maxX - minX, maxY - minY);
}

function maxSpeed(nodes: ReadonlyArray<{ vx: number; vy: number }>): number {
  let m = 0;
  for (const n of nodes) {
    const s = Math.hypot(n.vx, n.vy);
    if (s > m) { m = s; }
  }
  return m;
}

/** 生产同规模：200 桶 + 1837 条社区间边（fixture 实测口径）。 */
const COMPS = 200;
const EDGE_COUNT = 1837;
/** 生产平均成员数（24288 / 200 ÷ 1.66 ≈ 73；质量为 `max(1, count×0.6)`）。 */
const MEMBERS = 73;

// ─────────────────────────────────────────────────────────────────────────────
// ③-D 标定用的**社区几何**判据（2026-09-17；全部定义在世界坐标里 ⇒ 与 zoom 无关）
//
// 为什么可以脱离 zoom 判：放大只把「团半径」与「团间距」同比放大，**比值不变**
// ⇒ 「放大到 5 倍还是糊」不可能靠放大倍数解决，只可能是这两个比值本身不达标：
//   ① 团内：`r·√(π/n) ≥ 2·nodeSize`（间距 ≥ 两倍半径）⟺ `r/√n ≥ 2·nodeSize/√π`
//   ② 团间：最近邻两团中心距 ≥ `r_i + r_j`（否则成员点云互相穿插）
// ─────────────────────────────────────────────────────────────────────────────

/** 生产 `getNodeSize` 的典型值（区间 [4,22]）。§③-D 的所有屏幕换算都以它为准。 */
const NODE_SIZE_TYPICAL = 5;
/** 判据 ① 的阈值：`2·nodeSizeTypical/√π` ≈ 5.642。 */
const SEP_REQUIRED = (2 * NODE_SIZE_TYPICAL) / Math.sqrt(Math.PI);

/** 标定专用：把归一化的 `halfSpan` 传成不可能触发的值。
 *
 *  ⚠ 归一化超界时把布局**等比缩回**，会把物理真实的平衡尺度掩盖成常数（这正是
 *  `AGG_SETTLE_PX` 第一版踩过的坑）；标定 R* 时必须**以不触发的方式排除**它，
 *  而不是「让它参与然后解释结果」。 */
const CALIB_NO_CAP = 1e9;

interface CommunityGeometry {
  /** 最近邻团中心距的 p10 / p50（世界坐标）。 */
  dnnP10: number;
  dnnP50: number;
  /** 「最近邻距 < r_i + r_j」的社区占比 ⇒ 团块互相穿插的比例。 */
  overlapRatio: number;
  /** 全局最差一对的 `中心距 − 半径和`；≥0 表示无任何穿插。 */
  minSlack: number;
  /** 判据 ① 的实测值：`min over 成员数≥10 的社区 of r/√n`，须 ≥ SEP_REQUIRED。 */
  minSepRatio: number;
  maxRadius: number;
  maxCount: number;
  /** 社区半径的中位数（世界坐标）。 */
  rP50: number;
  /** 判据 ③：**团直径 / 团间距**（`2·rP50 / dnnP50`）。>1 ⇒ 典型相邻团互相重叠。
   *  它与 zoom **无关**（团尺寸与间距同比缩放）⇒ 这是「放大能否看清」的硬指标之一。 */
  diaOverGap: number;
  /** 判据 ③ 的**尾部**：`2·maxRadius / dnnP50` —— 重尾巨桶与邻居的穿插程度。
   *  单独报是因为它由分布尾部决定，典型值好看不代表巨桶不糊。 */
  diaMaxOverGap: number;
}

function communityGeometry(
  agg: ReadonlyArray<PhysicsNode>,
  memberCount: ReadonlyMap<number, number>,
  radiusOf: (count: number) => number = communityRadius,
): CommunityGeometry {
  const cs: { x: number; y: number; r: number }[] = [];
  for (const n of agg) {
    if (!n.id.startsWith("__agg__")) { continue; }
    const cnt = memberCount.get(Number(n.id.slice("__agg__".length))) ?? 0;
    cs.push({ x: n.x, y: n.y, r: radiusOf(cnt) });
  }
  const rs = cs.map((c) => c.r).sort((a, b) => a - b);
  const dnn: number[] = [];
  const overlapped = new Set<number>();
  let minSlack = Infinity;
  for (let i = 0; i < cs.length; i++) {
    let best = Infinity;
    for (let j = 0; j < cs.length; j++) {
      if (i === j) { continue; }
      const d = Math.hypot(cs[i].x - cs[j].x, cs[i].y - cs[j].y);
      if (d < best) { best = d; }
      const slack = d - cs[i].r - cs[j].r;
      if (slack < minSlack) { minSlack = slack; }
      if (slack < 0) {
        overlapped.add(i);
        overlapped.add(j);
      }
    }
    dnn.push(best);
  }
  dnn.sort((a, b) => a - b);
  const gap = dnn[Math.floor(dnn.length * 0.5)] ?? 0;
  let minSepRatio = Infinity;
  let maxCount = 0;
  for (const [, cnt] of memberCount) {
    if (cnt > maxCount) { maxCount = cnt; }
    if (cnt < 10) { continue; }
    const ratio = radiusOf(cnt) / Math.sqrt(cnt);
    if (ratio < minSepRatio) { minSepRatio = ratio; }
  }
  return {
    dnnP10: Number((dnn[Math.floor(dnn.length * 0.1)] ?? 0).toFixed(1)),
    dnnP50: Number((dnn[Math.floor(dnn.length * 0.5)] ?? 0).toFixed(1)),
    overlapRatio: Number((overlapped.size / Math.max(1, cs.length)).toFixed(3)),
    minSlack: Number(minSlack.toFixed(1)),
    minSepRatio: Number(minSepRatio.toFixed(3)),
    maxRadius: Number(Math.max(0, ...cs.map((c) => c.r)).toFixed(0)),
    maxCount,
    rP50: Number((rs[Math.floor(rs.length * 0.5)] ?? 0).toFixed(1)),
    diaOverGap: Number((gap === 0 ? Infinity : (2 * (rs[Math.floor(rs.length * 0.5)] ?? 0)) / gap).toFixed(2)),
    diaMaxOverGap: Number((gap === 0 ? Infinity : (2 * (rs[rs.length - 1] ?? 0)) / gap).toFixed(2)),
  };
}

/** ⚠ **仅作对照基线，不是被测对象**：改动前的社区半径公式 `clamp(8+2.2√n, 10, 44)`。
 *
 *  为什么要留一份：判据 ②（团间穿插）与 fit 态屏幕尺寸都**依赖 r 的绝对值**，
 *  只在新公式内部横向比（rep 400→9600）无法回答「相对改动前是改善还是退化」——
 *  那正是用户要的结论。生产实现已换成 `communityRadius`，所以这份副本只用于
 *  在同一条 dnn 分布上重算几何，口径与生产完全同源（dnn 由同一份物理产出）。 */
const communityRadiusBefore = (count: number) => Math.max(10, Math.min(44, 8 + Math.sqrt(Math.max(0, count)) * 2.2));

/** 跑一次标定：`membersOf(i)` 给第 i 个社区的成员数（决定质量分布）。
 *
 *  `useAnneal` 把「生产同一份退火」接进循环（2026-09-17）。它的存在是为了让
 *  case D 能做**对照实验**：同一分布、同一判据函数、同一物理，只切换退火开关 ——
 *  两组结论必须相反，否则这个修复没有区分力（判据 #329 同族）。 */
function runCalib(
  label: string,
  membersOf: (i: number) => number,
  maxSteps: number,
  cfg = AGG_PHYSICS_CONFIG,
  useAnneal = false,
  halfSpan = AGG_LAYOUT_HALF_SPAN,
) {
  const nodes: PhysicsNode[] = [];
  const communities = new Map<string, number>();
  for (let c = 0; c < COMPS; c++) {
    const cnt = membersOf(c);
    for (let m = 0; m < cnt; m++) {
      const id = `c${c}n${m}`;
      nodes.push({ id, x: 0, y: 0, vx: 0, vy: 0, fx: 0, fy: 0, mass: 1, fixed: false, kind: "note", idx: 0 });
      communities.set(id, c);
    }
  }
  const r = rng(20260917);
  const seen = new Set<string>();
  const edges: { source: string; target: string }[] = [];
  while (edges.length < EDGE_COUNT) {
    const a = Math.floor(r() * COMPS);
    const b = Math.floor(r() * COMPS);
    if (a === b) { continue; }
    const key = a < b ? `${a}-${b}` : `${b}-${a}`;
    if (seen.has(key)) { continue; }
    seen.add(key);
    // 端点取**成员节点**（生产里聚合边的端点就是真实节点）
    edges.push({
      source: `c${a}n${Math.floor(r() * membersOf(a))}`,
      target: `c${b}n${Math.floor(r() * membersOf(b))}`,
    });
  }
  const built = buildAggregateGraph({
    nodes,
    edges,
    communities,
    layoutUnits: new Set(Array.from({ length: COMPS }, (_, i) => i)),
    seedPhysics: AGG_PHYSICS_CONFIG,
  });
  const neighborMap = buildNeighborMap(built.edges);
  const agg = built.nodes;
  const settle = createAggregateSettleState();
  const anneal = createAggregateAnnealState();
  let settledAt = -1;
  let normTriggered = 0;
  let spanMin = Infinity;
  let spanMax = -Infinity;
  // ⚠ `maxAbs`（L∞ 半径）才是与 `halfSpan` **同量纲**的量 —— `normalizeAggregateScale`
  //   比的正是 `max(|x|,|y|)`；用 `span`（= max(W,H)）判「归一化该不该触发」是量纲错配。
  let maxAbsPeak = 0;
  const trace: string[] = [];

  for (let step = 1; step <= maxSteps; step++) {
    // 与生产**同序**：退火算本步上限 → 物理 → 归一化 → 检测
    const stepCfg = useAnneal
      ? { ...cfg, maxVelocity: updateAggregateAnneal(anneal, cfg.maxVelocity) }
      : cfg;
    stepPhysics(agg, built.edges, stepCfg, undefined, undefined, undefined, neighborMap);
    // 与组件**同序**：物理 → 归一化 → 检测（GraphView 里检测紧跟 normalizeAggregateScale）
    if (normalizeAggregateScale(agg, halfSpan) !== 1) { normTriggered++; }
    const { driftPx, justSettled } = updateAggregateSettle(settle, agg, A_STAGE_ZOOM);
    if (justSettled && settledAt < 0) { settledAt = step; }
    const sp = spanOf(agg);
    if (sp < spanMin) { spanMin = sp; }
    if (sp > spanMax) { spanMax = sp; }
    for (const n of agg) {
      const a = Math.abs(n.x) > Math.abs(n.y) ? Math.abs(n.x) : Math.abs(n.y);
      if (a > maxAbsPeak) { maxAbsPeak = a; }
    }
    if (step % 2000 === 0 || step === maxSteps) {
      trace.push(
        `step=${step} span=${sp.toFixed(0)} driftPx=${
          driftPx.toFixed(4)
        } calm=${settle.calmSteps} settled=${settle.settled} maxV=${maxSpeed(agg).toFixed(3)} temp=${
          anneal.scale.toFixed(6)
        }`,
      );
    }
  }

  // 「速度贴上限」的节点数：这是「driftPx 恒等于 `maxVelocity·dt·zoom`」的直接判据
  // —— 速度被夹住 ⇒ 每步位移恒为 `maxVelocity·dt` ⇒ 累计漂移恒定 ⇒ 闸永远不会判静。
  // ⚠ 统计量恒等必须先怀疑测量工具（判据 #8）：所以这里**不**用「driftPx 恒等」下结论，
  //   而是去找产生它的机制（速度夹持）与产生机制的原因（质量重尾 → 力不平衡）。
  const capV = cfg.maxVelocity * 0.99;
  const stuckAtMax = agg.filter((n) => Math.hypot(n.vx, n.vy) >= capV).length;

  let massSum = 0;
  for (let c = 0; c < COMPS; c++) { massSum += Math.max(1, membersOf(c) * 0.6); }
  const rStar = aggregateSeedRadius(cfg.repulsion, cfg.gravity, massSum);
  const finalSpan = spanOf(agg);
  const out = {
    label,
    maxSteps,
    massSum: Math.round(massSum),
    rStar: Math.round(rStar),
    span2R: Math.round(2 * rStar),
    finalSpan: Math.round(finalSpan),
    ratio: Number((finalSpan / (2 * rStar)).toFixed(3)),
    settledAt,
    normTriggered,
    finalCalm: settle.calmSteps,
    finalMaxV: Number(maxSpeed(agg).toFixed(4)),
    stuckAtMax,
    spanMin: Math.round(spanMin),
    spanMax: Math.round(spanMax),
    maxAbsPeak: Math.round(maxAbsPeak),
    capMaxV: cfg.maxVelocity,
  };
  console.log("[calib settle]", JSON.stringify(out));
  for (const line of trace) { console.log("    ", line); }
  return { ...out, nodes: agg, memberCount: built.memberCount, cfg, halfSpan };
}

describe("标定：生产规模跑到静止（回答「闸会不会触发、何时触发」）", () => {
  it("A. 均匀质量（200 × 73）—— 用来验证 R* 公式本身", () => {
    const out = runCalib("uniform", () => MEMBERS, 5000);
    // 均匀质量下实测 span/(2R*) ≈ 1.0（重尾分布才会偏高 15%~25%，见 case B）——
    // 这条断言把 §6.12.1 的平衡半径公式从「四组实测定系数」升级为「同分布下可复现」
    // ⚠ 容差下限必须随 rep/g 下移：`R* = √(rep·M/g)` 是**远场**近似（把团块当点质量），
    //   而 rep 越高、团块越大，实际几何比越低于 1 —— 实测序列（同分布、只变 rep）：
    //     600 → 0.976 ｜ 2400 → 0.94 ｜ 5400 → 0.792~0.90 ｜ 9600 → 0.81~0.87
    //   ⇒ 下限取 0.75（覆盖实测最低 0.792）；上限 1.15 不动（高估仍是必须拦下的失败模式）。
    expect(out.ratio).toBeGreaterThan(0.75);
    expect(out.ratio).toBeLessThan(1.15);
    // 尺度受控用**与 halfSpan 同量纲**的 maxAbs 判（span = max(W,H) 量纲不同）
    expect(out.maxAbsPeak).toBeLessThanOrEqual(AGG_LAYOUT_HALF_SPAN + 1);
  }, 30000);

  it("B. 重尾质量（成员数 1~921 ⇒ 质量 1~553，逼近生产）—— 闸会不会触发", () => {
    // 生产质量 max(1, 成员数×0.6) ∈ [1, 553] ⇒ 成员数 ∈ [1, 921]；对数均匀抽样，
    // 均值 ≈ 135（生产 avg 72.9，但重尾形状一致）
    const lnMin = Math.log(1);
    const lnMax = Math.log(921);
    const membersOf = (i: number) => {
      const u = rng(0x51 + i * 7919)();
      return Math.max(1, Math.round(Math.exp(lnMin + u * (lnMax - lnMin))));
    };
    const out = runCalib("heavy-tail", membersOf, 8000);
    // 尺度必须受控（归一化即使跑也不允许越界）—— 这是 AGG_LAYOUT_HALF_SPAN 的职责
    expect(out.finalSpan).toBeLessThanOrEqual(AGG_LAYOUT_HALF_SPAN * 2 + 1e-6);
    // 布局必须真的在演化：若 maxV 恒 0/span 恒等于播种半径，说明又踩到「冻结」老坑
    expect(out.finalMaxV).toBeGreaterThan(0);
    expect(out.massSum).toBeGreaterThan(8000);
    // ⚠ 实测发现（写进断言，因为它是**机制**而非结论）：重尾质量下有节点**贴住 maxVelocity 上限**
    // ⇒ 每步位移恒为 `maxVelocity·dt` ⇒ 累计漂移恒定（driftPx 六个采样点逐字节相同）
    // ⇒ 闸永不判静。这正是「统计量恒等 ⇒ 先怀疑测量工具」的正确用法：先找机制，
    // 找到后确认**不是**测量坏了（判据 #8）。均匀质量下该值为 0 ⇒ 有区分力。
    expect(out.stuckAtMax).toBeGreaterThan(0);
    // 「闸是否触发」不在断言里 —— 它是**观测**（见 console 输出的 settledAt / finalCalm / ratio），
    // 会随参数变化；把它写死进断言等于把标定结论当成契约。
  }, 30000);

  it("C. 诊断：抬高 maxVelocity 上限 ⇒「贴上限」**依然存在**（排除「上限太小」这一解释）", () => {
    const lnMin = Math.log(1);
    const lnMax = Math.log(921);
    const membersOf = (i: number) => {
      const u = rng(0x51 + i * 7919)();
      return Math.max(1, Math.round(Math.exp(lnMin + u * (lnMax - lnMin))));
    };
    const out = runCalib("heavy-tail-highcap", membersOf, 3000, { ...AGG_PHYSICS_CONFIG, maxVelocity: 400 });
    expect(out.capMaxV).toBe(400);
    // 关键实测结论：上限 12 → 400 后，**仍有 48 个节点贴住新上限**（不是消失）。
    // ⇒ 「driftPx 恒定」不是「上限太小」造成的，而是重尾质量下系统持续需要高速 ⇒
    //   抬高上限只会让尺度更失控（本次实测 normTriggered 由 0 升到 19，归一化开始频繁介入）。
    expect(out.stuckAtMax).toBeGreaterThan(0);
    expect(out.finalMaxV).toBeGreaterThan(AGG_PHYSICS_CONFIG.maxVelocity * 10);
  }, 30000);

  it("D. 退火（同分布对照）：不退火**永不触发**，接上退火**必定触发** —— 「跳」的修复判据", () => {
    // 与 case B 同一个分布与种子 ⇒ 两组唯一变量就是 `useAnneal`。
    // ⚠ 断言里**不含任何阈值放宽**：判据函数（updateAggregateSettle）与它的阈值
    //   （AGG_SETTLE_PX / AGG_SETTLE_STEPS）一个字节都没动 —— 退火改的是物理，
    //   不是「什么算静止」。
    const lnMin = Math.log(1);
    const lnMax = Math.log(921);
    const membersOf = (i: number) => {
      const u = rng(0x51 + i * 7919)();
      return Math.max(1, Math.round(Math.exp(lnMin + u * (lnMax - lnMin))));
    };
    const off = runCalib("anneal-off", membersOf, 3000, AGG_PHYSICS_CONFIG, false);
    const on = runCalib("anneal-on", membersOf, 3000, AGG_PHYSICS_CONFIG, true);

    // ① 区分力（本测试的核心）：同一分布、同一判据，只切换退火 ⇒ 结论相反
    expect(off.settledAt).toBe(-1);
    expect(on.settledAt).toBeGreaterThan(0);
    // ② 退火必须晚于全温期启动 —— 否则布局会在成形之前被冻住（"半成品"）。
    //    上界同样要钉：退火太慢等于没修（实测 1112 步，余量 35%）。
    expect(on.settledAt).toBeGreaterThan(AGG_ANNEAL_START_STEPS);
    expect(on.settledAt).toBeLessThan(AGG_ANNEAL_START_STEPS + 600);
    // ③ 尺度不能被退火破坏：退火只压速度上限，不动 rep/gravity ⇒ 平衡尺度 R* 不变，
    //    所以终态跨度必须与不退火时**同量级**。容差 ±10% —— 实测差 1.7%（2081 vs 2118），
    //    而「冻在 900 步的早期形态」会小 ~10% ⇒ 这个容差刚好能抓到那种失败模式；
    //    ±20% 抓不到（那就是个没有区分力的容差）。
    expect(on.finalSpan).toBeGreaterThan(off.finalSpan * 0.9);
    expect(on.finalSpan).toBeLessThan(off.finalSpan * 1.1);
    // ④ 退火的直接机制证据：贴住速度上限的节点必须**归零**（现状是 39 个）
    expect(off.stuckAtMax).toBeGreaterThan(0);
    expect(on.stuckAtMax).toBe(0);
  }, 30000);

  it("E. ③-D 标定：把社区半径抬到可分辨后，团间距（R*）要同步抬多少", () => {
    // 判据 ①（团内可分辨）**只由 communityRadius 决定**，与 rep/g 无关 ⇒ 本 case 的
    // 变量是判据 ②（团间穿插）。新社区半径下每个团都变大了，若团间距不跟着涨，
    // 200 个团会互相吞并 —— 那只是把「一团糊的点」换成「一团糊的团」。
    const lnMin = Math.log(1);
    const lnMax = Math.log(921);
    const membersOf = (i: number) => {
      const u = rng(0x51 + i * 7919)();
      return Math.max(1, Math.round(Math.exp(lnMin + u * (lnMax - lnMin))));
    };
    // ⚠ 归一化必须**以不触发的方式排除**（见 CALIB_NO_CAP）：它超界时把布局**等比缩回**，
    //   会把物理真实的平衡尺度掩盖成常数 `halfSpan`。本次要标的恰恰是那个平衡尺度。
    // R* = √(rep·M/g) ⇒ 要 R* 涨 k 倍，rep/g 须涨 k² 倍。基准 600/8 = 75。
    const sets = [[600, 8], [2400, 8], [5400, 8], [9600, 8]] as const;
    for (const [rep, g] of sets) {
      const cfg = { ...AGG_PHYSICS_CONFIG, repulsion: rep, gravity: g };
      // 两组：不接退火（物理能自由跑到自己的平衡点）vs 接退火（生产实际路径）。
      // 若两者 span 出现数量级差异，说明「退火只压速度、不改 R*」这个前提在
      // 高 rep 下失效（v∞ 涨过 maxVelocity ⇒ 构型被速度上限限制）—— 那本身就是要报的结论。
      const free = runCalib(`d3d${rep}-free`, membersOf, 6000, cfg, false, CALIB_NO_CAP);
      const annealed = runCalib(`d3d${rep}-anneal`, membersOf, 6000, cfg, true, CALIB_NO_CAP);
      // 同一条 dnn 分布上重算两套半径 ⇒ 「相对改动前是改善还是退化」才有同口径可比。
      // 只在**同组 dnn** 内比较：rep 变了 dnn 就变，跨组比半径没意义。
      const after = communityGeometry(annealed.nodes, annealed.memberCount, communityRadius);
      const before = communityGeometry(annealed.nodes, annealed.memberCount, communityRadiusBefore);
      console.log(
        "[d3d]",
        JSON.stringify({
          rep,
          g,
          rStar: annealed.rStar,
          spanFree: free.finalSpan,
          spanAnneal: annealed.finalSpan,
          settledAt: annealed.settledAt,
          normTriggered: annealed.normTriggered,
          after,
          before,
        }),
      );
    }
    // ⚠ 这里不写死任何阈值断言：参数由本轮实测选定后才固化成**回归断言**（见 case F），
    //   先写断言等于把「还没标定出来的数」当成契约（判据 #329 同族）。
    expect(SEP_REQUIRED).toBeGreaterThan(5.6);
  }, 120000);

  it("F. 诊断：高 rep 下 span/(2R*) 掉到 0.84、settledAt 恒 1112 —— 是限速还是没跑完", () => {
    // 这是 case E 里**必须先证伪**的一条：`settledAt` 在四档 rep 下逐字相同（1112），
    // 而 `span/(2R*)` 从 0.97 单调掉到 0.84 ⇒ 两种可能：
    //   ① 物理确实到了自己的平衡点（那 0.84 就是真实几何比，读数可信）；
    //   ② `maxVelocity = 12` 把速度夹住 ⇒ 布局在**跑到平衡点之前**被限速冻住，
    //      而退火进一步压速 ⇒ 闸判"静止" ⇒ **假收敛**（判据 #8：恒等量先怀疑测量）。
    // 判据：不接退火、跑 30000 步，看 span 轨迹在 6000 步之后是否仍单调上涨；
    // 并对照 `maxVelocity = 100`（解除限速）—— 若抬上限后 span 显著变大，
    // 说明 ② 成立，「抬 rep」的收益正在被速度上限吃掉。
    const lnMin = Math.log(1);
    const lnMax = Math.log(921);
    const membersOf = (i: number) => {
      const u = rng(0x51 + i * 7919)();
      return Math.max(1, Math.round(Math.exp(lnMin + u * (lnMax - lnMin))));
    };
    const sets: [number, number][] = [[2400, 12], [2400, 100], [9600, 12], [9600, 100]];
    const summary: Record<string, number>[] = [];
    for (const [rep, cap] of sets) {
      const res = runCalib(
        `d3d-rep${rep}-cap${cap}`,
        membersOf,
        30000,
        { ...AGG_PHYSICS_CONFIG, repulsion: rep, maxVelocity: cap },
        false,
        CALIB_NO_CAP,
      );
      summary.push({
        rep,
        cap,
        rStar: res.rStar,
        finalSpan: res.finalSpan,
        ratio: Number((res.finalSpan / (2 * res.rStar)).toFixed(3)),
        maxV: res.finalMaxV,
        stuckAtMax: res.stuckAtMax,
      });
    }
    console.log("[d3d-F]", JSON.stringify(summary));
    expect(summary.length).toBe(sets.length);
  }, 300000);

  it("G. 回归：③-D 选定参数下 —— 归一化不触发、退火仍收敛、判据①达标、典型穿插不劣于改动前", () => {
    // 这一条把 case E 的**标定结论**固化成契约：参数一旦被改动，这里立刻报红。
    // ⚠ 对照必须是**同口径的「现状」**=「旧半径公式 × 改动前的 rep=600」。
    //   拿「同一 rep 下的新旧半径」互比是错的 —— 那是「抬 r 却没抬 rep」的中间态，
    //   结论必然是「新半径更穿插」，证不了「新方案相对现状是否更好」。
    const lnMin = Math.log(1);
    const lnMax = Math.log(921);
    const membersOf = (i: number) => {
      const u = rng(0x51 + i * 7919)();
      return Math.max(1, Math.round(Math.exp(lnMin + u * (lnMax - lnMin))));
    };
    // ⚠ 这里**故意不传** CALIB_NO_CAP：本 case 要证的正是「生产参数下归一化不触发」，
    //   传 NO_CAP 等于先把被测机制关掉、再宣称它没问题（同族：「豁免禁塞业务」）。
    const now = runCalib("d3d-prod", membersOf, 8000, AGG_PHYSICS_CONFIG, true);
    const baseline = runCalib(
      "d3d-legacy",
      membersOf,
      8000,
      { ...AGG_PHYSICS_CONFIG, repulsion: 600 },
      true,
    );
    const after = communityGeometry(now.nodes, now.memberCount, communityRadius);
    const baseGeo = communityGeometry(baseline.nodes, baseline.memberCount, communityRadiusBefore);

    // ① 安全阀不触发：物理自己收敛在 halfSpan 内（归一化是安全阀，不是主机制）
    expect(now.normTriggered).toBe(0);
    // 判据用**与 halfSpan 同量纲**的 maxAbs（L∞ 半径），不是 span —— 本 case 第一版就是
    // 拿 span 判「归一化该不该触发」，结果它对触发 72 次完全无感（量纲错配，同族 #255/#313）。
    expect(now.maxAbsPeak).toBeLessThanOrEqual(AGG_LAYOUT_HALF_SPAN);
    // ② 抬 rep 16 倍后，退火仍能在全温期之后收敛（实测 settledAt 恒 1112，四档 rep 相同）
    expect(now.settledAt).toBeGreaterThan(AGG_ANNEAL_START_STEPS);
    // ③ 判据①：团内可分辨 —— 本次修复的核心诉求，与 rep 无关
    expect(after.minSepRatio).toBeGreaterThanOrEqual(SEP_REQUIRED);
    // ④ 判据③：典型团穿插**不劣于**改动前 —— 这是「必须同步抬 rep」的正当性依据
    expect(after.diaOverGap).toBeLessThanOrEqual(baseGeo.diaOverGap);
    expect(after.overlapRatio).toBeLessThan(baseGeo.overlapRatio);
    console.log(
      "[d3d-G]",
      JSON.stringify({
        baseline: { span: baseline.finalSpan, ...baseGeo },
        now: { span: now.finalSpan, ...after },
      }),
    );
  }, 120000);
});

describe("updateAggregateSettle：契约（语义与量纲）", () => {
  const px = (x: number, y: number) => ({ x, y });

  it("完全静止 ⇒ 连续 AGG_SETTLE_STEPS 步后**首次**判 settled（且只报一次）", () => {
    const st = createAggregateSettleState();
    const nodes = [px(0, 0), px(10, 20)];
    expect(updateAggregateSettle(st, nodes, 1).justSettled).toBe(false); // 第一步只建窗口
    let reports = 0;
    for (let i = 0; i < AGG_SETTLE_STEPS + 5; i++) {
      if (updateAggregateSettle(st, nodes, 1).justSettled) { reports++; }
    }
    expect(st.settled).toBe(true);
    expect(reports).toBe(1);
  });

  it("「累计漂移」而非「相邻两步之差」：每步 0.02px 的匀速漂移，60 步累计 1.2px > 0.5 ⇒ **不** settled", () => {
    const st = createAggregateSettleState();
    const nodes = [px(0, 0)];
    updateAggregateSettle(st, nodes, 1);
    for (let i = 1; i <= AGG_SETTLE_STEPS + 10; i++) {
      nodes[0].x = i * 0.02; // 屏幕位移 = 0.02px/步（zoom = 1）
      updateAggregateSettle(st, nodes, 1);
    }
    expect(st.settled).toBe(false);
    // ⚠ 不能断言 calmSteps === 0：阈值是「累计 ≥ 0.5px 就重置窗口」，0.02px/步 ⇒ 每 25 步
    //    重置一次，calmSteps 在 0..24 之间循环。若判据写成「相邻两步之差」（0.02 < 0.5 恒成立），
    //    calmSteps 会一路涨到 60 ⇒ settled = true ——「< 60」才是这条测试的区分点。
    expect(st.calmSteps).toBeLessThan(AGG_SETTLE_STEPS);
  });

  it("阈值按「屏幕上看得见」判定：同一段世界位移，zoom 不同 ⇒ 结论相反（有区分力）", () => {
    // 世界累计位移恒为 0.6（60 步 × 0.01）⇒ 屏幕位移 = 0.6 × zoom
    const runAt = (zoom: number) => {
      const st = createAggregateSettleState();
      const nodes = [{ x: 0, y: 0 }];
      updateAggregateSettle(st, nodes, zoom);
      for (let i = 1; i <= AGG_SETTLE_STEPS + 1; i++) {
        nodes[0].x = i * 0.01;
        updateAggregateSettle(st, nodes, zoom);
      }
      return st;
    };
    expect(runAt(0.5).settled).toBe(true); // 屏幕 0.3px < 0.5 ⇒ 判「静」
    expect(runAt(2).settled).toBe(false); // 屏幕 1.2px > 0.5 ⇒ 判「动」
  });

  it("节点数变化（重建布局）⇒ 重新建窗口，不沿用旧快照", () => {
    const st = createAggregateSettleState();
    updateAggregateSettle(st, [px(0, 0), px(1, 1)], 1);
    const res = updateAggregateSettle(st, [px(9, 9)], 1);
    expect(res.driftPx).toBe(0);
    expect(st.snap.length).toBe(2);
    expect(st.calmSteps).toBe(0);
  });

  it("px 参数可注入（标定用），不影响默认阈值", () => {
    const st = createAggregateSettleState();
    const nodes = [px(0, 0)];
    updateAggregateSettle(st, nodes, 1);
    nodes[0].x = 0.3; // 默认 0.5 ⇒ 不动
    expect(updateAggregateSettle(st, nodes, 1).driftPx).toBeCloseTo(0.3, 6);
    expect(st.calmSteps).toBe(1);
  });
});
