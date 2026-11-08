// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 聚合布局的**纯函数层**（2026-09-16 抽取）。
 *
 * 为什么必须单独一层：`buildAggregatePhysics` / `applyAggregateLayout` 原本内联在
 * GraphView 组件里，依赖 6 个 ref，因此**「这套参数到底会不会自平衡」无法在测试里复现** ——
 * 任何标定脚本都只能把构建逻辑重抄一遍，而重抄的那一刻，被测对象与标定对象就分叉了
 * （判据 #7/#313 的同族：测的必须是真对象）。
 * 抽出后组件与标定/回归测试调用**同一个** `buildAggregateGraph`，
 * 参数标定结果才有资格写进源码常量。
 *
 * 本文件不持有任何 ref / DOM / React 依赖 —— 纯输入输出，可在 jsdom 之外直接跑。
 */
import type { PhysicsConfig, PhysicsEdge, PhysicsNode } from "./graphPhysics";
import { hashStringToInt } from "./graphViewUtils";

export interface AggregateGraphInput {
  /** 全部真实物理节点（成员节点会被**按引用**并入结果，坐标由调用方回写）。 */
  nodes: PhysicsNode[];
  /** 带字符串端点的边（非索引化的物理边）。 */
  edges: ReadonlyArray<{ source: string; target: string }>;
  /** nodeId → 社区/桶 id（应为归并后的 effectiveCommunities）。 */
  communities: Map<string, number>;
  /** 参与聚合力导向的布局单元（隐式聚合 = 全部社区；显式折叠 = 被折叠的社区）。 */
  layoutUnits: ReadonlySet<number>;
  /** 可选：布局单元的初始位置（一般来自 `clusterGeomRef` 的质心）。 */
  centroidOf?: (cid: number) => { cx: number; cy: number } | undefined;
  /**
   * 播种用的力参数（**单一真相源** = 调用方传入的 `AGG_PHYSICS_CONFIG`）。
   *
   * 无质心时按目标平衡半径 `R* = √(repulsion·M/gravity)` 做**圆盘**播种（`r = R*·√u`）。
   * 旧实现是 `r = 400` 的**圆环**：① 形状本身就是环 ⇒ 首屏中心空白；
   * ② 400 与标定平衡半径（`R*` ≈ 1.2e3）差 3 倍 ⇒ 需约 40s 才重排成实心盘（AUDIT §6.12.6-2）。
   * ⚠ **必填而非可选**：可选会给「忘了传」的调用点留下静默退回旧圆环的路径，
   * 而那恰恰是要修的那个缺陷（判据同族：不能靠缺省值静默退化）。
   */
  seedPhysics: { repulsion: number; gravity: number };
}

export interface AggregateGraph {
  nodes: PhysicsNode[];
  edges: PhysicsEdge[];
  cidToNodeIdx: Map<number, number>;
  /** 每个社区的真实成员数（**未经** `max(1, …)` 夹取），供质量与半径共用同源口径。 */
  memberCount: Map<number, number>;
}

/**
 * 统计每个社区的**真实成员数**（O(N) 一次遍历）。
 *
 * ⚠ 单一真相源：调用方用它判断「社区数是否允许隐式聚合」，建图用它算聚合质量与半径 ——
 * 两侧各写一遍遍历，就会出现「判据用 A 口径、建图用 B 口径」的漂移。
 * 大图打开时若社区粒度细，naive 的 O(C×N) 双重循环可达数千万次 Map 查找，主线程卡死数秒。
 */
export function countMembers(
  nodes: ReadonlyArray<{ id: string }>,
  communities: Map<string, number>,
): Map<number, number> {
  const memberCount = new Map<number, number>();
  for (const node of nodes) {
    const cid = communities.get(node.id);
    if (cid !== undefined) {
      memberCount.set(cid, (memberCount.get(cid) ?? 0) + 1);
    }
  }
  return memberCount;
}

/**
 * 由「真实节点图 + 社区映射 + 布局单元集合」构建聚合物理图。
 *
 * 语义要点（改动前请先读）：
 *   · 每个布局单元 → **1 个**聚合节点，质量 = `max(1, 成员数 × 0.6)`；
 *   · 不在布局单元内的真实节点**按引用**并入 `nodes`，物理就地更新它们的 x/y；
 *   · 聚合边 = 把两端各自映射到布局单元后**去重合并**（多边归并为单一拓扑张力，
 *     `restLength` 只减不增 —— 保留更紧凑的那条）。
 *   · 聚合节点的初速**必须非零**：`stepPhysics` 在「全节点静止且未传 communities」时
 *     会直接 early return，初速为 0 会让聚合物理永远不启动（白构建一场）。
 *     用 cid 派生确定性方向而非随机 —— 重建后朝向不变，不会每帧抖动。
 */
export function buildAggregateGraph(input: AggregateGraphInput): AggregateGraph {
  const { nodes: allNodes, edges, communities, layoutUnits, centroidOf, seedPhysics } = input;
  const aggNodes: PhysicsNode[] = [];
  const idToIdx = new Map<string, number>();
  const cidToNodeIdx = new Map<number, number>();

  // 预计算每个社区成员数（与调用方判断「能否隐式聚合」时**同源**，见 countMembers）
  const memberCount = countMembers(allNodes, communities);

  // 目标尺度：`R* = √(repulsion·M/gravity)`。M 必须用**与建图同一口径**的质量
  // （`max(1, count·0.6)`），否则「算 R* 的 M」与「实际播下的 M」不同源 ⇒ 半径系统性偏。
  let massSum = 0;
  for (const cid of layoutUnits) {
    massSum += Math.max(1, (memberCount.get(cid) ?? 0) * 0.6);
  }
  const seedRadius = aggregateSeedRadius(seedPhysics.repulsion, seedPhysics.gravity, massSum);

  for (const cid of layoutUnits) {
    const geom = centroidOf?.(cid);
    const count = memberCount.get(cid) ?? 0;
    const idx = aggNodes.length;
    const id = `__agg__${cid}`;
    // 初值必须**确定性**（同 cid 每次重建给同一朝向）⇒ 用 cid 派生种子，不用 Math.random
    const rnd = mulberry32(hashStringToInt(`agg:${cid}`) >>> 0);
    const seedAngle = rnd() * Math.PI * 2;
    // `r = R*·√u` ⇒ 面积均匀 = **圆盘**；旧实现 `r = 400` 恒定 = 圆环 ⇒ 首屏中心空白
    const seedR = seedRadius * Math.sqrt(rnd());
    aggNodes.push({
      id,
      x: geom?.cx ?? Math.cos(seedAngle) * seedR,
      y: geom?.cy ?? Math.sin(seedAngle) * seedR,
      vx: Math.cos(seedAngle) * 0.4,
      vy: Math.sin(seedAngle) * 0.4,
      fx: 0,
      fy: 0,
      mass: Math.max(1, count * 0.6), // 聚合质量 = 成员数加权
      fixed: false,
      kind: "source",
      idx,
    });
    idToIdx.set(id, idx);
    cidToNodeIdx.set(cid, idx);
  }

  // 不在布局单元内的成员 + 零散节点 → 真实物理节点（共享引用，就地更新）
  for (const node of allNodes) {
    const cid = communities.get(node.id);
    if (cid !== undefined && layoutUnits.has(cid)) { continue; }
    idToIdx.set(node.id, aggNodes.length);
    aggNodes.push(node);
  }

  // 聚合边：遍历全部边，把端点映射到聚合/真实节点索引，去重合并
  const aggEdges: PhysicsEdge[] = [];
  const seen = new Map<number, number>();
  const edgeKey = (a: number, b: number) => (a < b ? a * 100000 + b : b * 100000 + a);
  for (const em of edges) {
    const sCid = communities.get(em.source);
    const tCid = communities.get(em.target);
    const sIsUnit = sCid !== undefined && layoutUnits.has(sCid);
    const tIsUnit = tCid !== undefined && layoutUnits.has(tCid);
    const sKey = sIsUnit ? `__agg__${sCid}` : em.source;
    const tKey = tIsUnit ? `__agg__${tCid}` : em.target;
    const sIdx = idToIdx.get(sKey);
    const tIdx = idToIdx.get(tKey);
    if (sIdx === undefined || tIdx === undefined || sIdx === tIdx) { continue; }
    const key = edgeKey(sIdx, tIdx);
    const existing = seen.get(key);
    if (existing !== undefined) {
      // 合并重复边：保留更紧凑的 restLength（多边归并为单一拓扑张力）
      const e = aggEdges[existing];
      if (e.restLength > 140) { e.restLength = 140; }
      continue;
    }
    seen.set(key, aggEdges.length);
    aggEdges.push({
      source: sKey,
      target: tKey,
      restLength: 140,
      stiffness: 0.8,
      damping: 0.6,
      sourceIdx: sIdx,
      targetIdx: tIdx,
    });
  }

  return { nodes: aggNodes, edges: aggEdges, cidToNodeIdx, memberCount };
}

/**
 * 尺度归一化：把布局**等比**缩回半跨度 `halfSpan` 以内，返回实际使用的缩放因子
 * （未触发时为 1）。
 *
 * 为什么需要它：见 `AGG_PHYSICS_CONFIG` 上方的推导更正 —— 该力模型**存在**有限平衡半径
 * `R* = √(repulsion·M/gravity)`（`M` = 总质量 = 桶数 × 平均质量）。⚠ 不要按「圆盘内部均匀
 * 面密度」做解析积分：二维 1/r² **没有壳层定理**，边界处积分随源点趋近而发散，会得到
 * `√(6·repulsion·m²·N/gravity)` 这种在 m≈73 时**偏大 20.9 倍**的错式（本注释曾照抄过它，
 * 2026-09-17 修正）。旧参数 70000/0.01 ⇒ `R* ≈ 3.2e5`，而实测半跨度只到 `6.5e3`
 * ⇒ 距平衡点 **49 倍** ⇒ 外部表现就是「看起来无界膨胀」。参数已按标定值重设（有效区附近），
 * 本函数退化为**安全阀**：物理若因拓扑异常（极端度分布、用户拖拽施加的外力）越界，
 * 等比缩回即可，形状完全保留。
 *
 * ⚠ 速度必须**同时等比**缩放：否则缩放后速度相对新尺度过大，下一帧立刻把整体又推出去，
 * 形成「每帧缩回 ↔ 每帧膨胀」的对抗，表现为持续抖动、布局永不静止。
 *
 * ⚠ 必须对**隐式聚合与显式折叠两条路径**都生效（2026-09-16 修复）：此前只在
 * `applyAggregateLayout`（隐式）里调用，而显式全折叠的几何来自 `refreshClusterGeom`
 * 对成员原始坐标的包围盒 ⇒ 该模式完全没有尺度控制，实测只达到隐式路径 1/13 的
 * 像素覆盖率（0.488% vs 5.379%）。
 */
export function normalizeAggregateScale(nodes: PhysicsNode[], halfSpan: number): number {
  let maxAbs = 0;
  for (const n of nodes) {
    const a = Math.abs(n.x) > Math.abs(n.y) ? Math.abs(n.x) : Math.abs(n.y);
    if (a > maxAbs) { maxAbs = a; }
  }
  if (!(maxAbs > halfSpan)) { return 1; }
  const f = halfSpan / maxAbs;
  for (const n of nodes) {
    n.x *= f;
    n.y *= f;
    n.vx *= f;
    n.vy *= f;
  }
  return f;
}

/**
 * 目标平衡半径 `R* = √(repulsion·M/gravity)`（Barnes-Hut 远场 + 恒定大小重力场）。
 *
 * 推导与**四组实测对账**（`span/(2R*)` = 1.12 / 1.12 / 1.25 / 1.18）见 GraphView.tsx
 * `AGG_PHYSICS_CONFIG` 上方注释与 `AUDIT-wiki-graph-edges-2026-09-15.md` §6.12.1。
 * ⚠ 不要用「圆盘内均匀面密度」解析积分替代：二维 1/r² 没有壳层定理，那样会得到
 * 偏大 20 倍以上的错式（本文件注释曾照抄过）。
 *
 * 退化输入（M ≤ 0 或参数非正）返回 0 ⇒ 播种落在原点附近，由物理自行展开（确定性、不抛）。
 * 这个函数是**播种**与**标定**共用的同一点 —— 两侧各写一遍就会漂移。
 */
export function aggregateSeedRadius(repulsion: number, gravity: number, totalMass: number): number {
  if (!(totalMass > 0) || !(gravity > 0) || !(repulsion > 0)) { return 0; }
  return Math.sqrt((repulsion * totalMass) / gravity);
}

/**
 * 确定性 PRNG（mulberry32）。播种必须可复现：同 cid 每次重建给出**同一**初值，
 * 且不留全局状态（对比 `Math.random` 会让每次重建朝向都变 ⇒ 布局每帧抖动）。
 */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6D2B79F5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// 聚合物理配置（2026-09-16 参数标定；2026-09-17 自 GraphView.tsx 迁入）
//
// 为什么放在纯函数层：标定脚本与回归测试必须 import **同一份**常量 —— 定义在组件文件里时，
// 测试要么手拉整个组件（React/DOM 依赖），要么手抄数值，而手抄的那一刻「被测对象」与
// 「生产配置」就分叉了（同族：判据 #7/#313）。
// ─────────────────────────────────────────────────────────────────────────────

// ── 聚合物理配置：以「社区」为单位的力导向（2026-09-16 参数标定）──
// 与跑 24288 原始节点的主物理是**两套独立世界**：本配置的规模 = 社区数
// （200 个桶，上限 MAX_AGG_PHYS_NODES），因此参数按「200 个团块互相推开、且整体收在
// 可视范围」来定，而不是按「两万点铺满画布」。
//   · repulsion 承担铺开 —— 太小则社区重叠、气泡糊成一片
//   · gravity 承担收拢 —— 太小则 200 个社区被斥力推到无穷远、画面只剩几个孤点
//   · springForce 取弱值 —— 社区间的聚合边只表达拓扑张力，不应把相邻社区拽在一起
//
// ⚠⚠ 参数由**真实数据标定**得出，不是推导也不是试凑（2026-09-16）：
//   标定资产：真实 24288 节点 / 74791 边 → 拓扑归并 2458 → **200 桶**（communityMerge），
//   聚合节点 200 / 聚合边 1837 / 质量 = max(1, 成员数×0.6) ∈ [1, 553]，
//   用**生产代码** `buildAggregateGraph` + `stepPhysics` 扫描（脚本见报告 §6.12 复现资产）。
//
//   【教科书推导为什么不可靠】`gravity·d ≈ repulsion/d² ⇒ d=(rep/grav)^(1/3)` 是错的：
//   stepPhysics 的重力是**恒定大小的向心力**（graphPhysics.ts:360），力平衡式里没有与 d
//   成正比的项。
//
//   【正确的平衡半径】Barnes-Hut 远场下，距「总质量 M = Σmᵢ」的团块 R 处的节点受到的净斥力
//   `F = repulsion·m·M/R²`（这就是 barnesHutForce 的叶子公式，远场时整块被当点质量），
//   重力在开启 gravityScalesWithMass 后为 `F = gravity·m`，相消即
//       **R* = √(repulsion·M/gravity)**      （M = 总质量 = 桶数 × 平均质量）
//   ⚠ 为什么不按「圆盘内部均匀面密度」解析积分（上一版就是这么错的）：二维下 1/r² 定律
//   **没有壳层定理**，边界处积分随源点趋近而发散，拿不到干净闭式。所以这里只写**能用实测
//   对账**的远场式，内部是否实心交给实测判（见下方「形状修正」）。
//
//   【用四组实测把系数钉住】rep=600 / gravity=8 / M=200×72.9=14580 ⇒ R*=1046 ⇒
//   span*=2R*≈2092；实测终态 2400~2600，系统偏高 15%~25%（团块非点质量 + 最外节点不在
//   质量边缘）。四组同弹簧参数下 span/(2√(rep·M/g)) = **1.12 / 1.12 / 1.25 / 1.18** ——
//   在 rep/g 变化 1.75 倍的区间内保持一致 ⇒ 该式可用。
//   （留痕：上一版此处写的是 R* ≈ √(6·repulsion·m²·N/gravity)，在 m≈73 时**偏大 20.9 倍**。
//     同一个注释连着两版都是错公式 ⇒ 本版只保留有实测对账的那个。）
//
//   【旧参数为什么表现为「无界膨胀」】rep=70000 / gravity=0.01 ⇒ R* ≈ 3.2e5，
//   而实测半跨度只到 6.5e3 ⇒ 距平衡点还有 **49 倍**，可视时间内永远走不到
//   （世界跨度 5712 → 13040，相机 zoom 0.126 → 0.047，200 个社区在屏幕上只剩 2px）。
//
//   【扫描结论】平衡跨度由 **repulsion/gravity 比值**决定（M 固定时）——
//   四组参数实测终态 span ∝ (rep/g)^p，p 实测 **0.53~0.59 ≈ 0.5**，与 R*=√(rep·M/g) 相符；
//   比值 ≈0.0117~0.0133 的三组 span 均在 **2400~2600**，远超「必须精确调到某个点」的敏感度：
//     g=3.5 rep=300 → span 2441~2513 ｜ g=5 rep=400 → 2422~2547 ｜ g=8 rep=600 → 2365~2608
//   取 **gravity=8 / repulsion=600**（比值 1/75）：绝对量级最大 ⇒ 到达
//   maxVelocity 最快、收敛最省帧数；且实测 rms/R = 0.687~0.715，
//   与「均匀实心圆盘」的理论值 0.707 最接近（= 形状最均匀、不是空心环）。
//
//   【形状修正 ring → disk】环状/中心空白的直接来源是**重力语义与质量语义不一致**：
//   原式 `a = gravity/mass`，而聚合节点质量跨 3 个数量级（1~553）⇒ 214 人的社区受到的
//   向心加速度只有 1 人社区的 1/553 ⇒ 大社区被斥力推到外圈、小社区堆在中心。
//   开启 `gravityScalesWithMass`（重力变成均匀加速度场）+ `keepSimulating` 后实测
//   `inner(<0.4R)` 命中 0.16（= 均匀圆盘的理论面积比 0.4²），确实变成实心盘。
//
//   【必须同时开 keepSimulating】见 PhysicsConfig 的注释：该兜底把「受力但速度慢」
//   误判为「静止」并清零速度。把 repulsion 从 70000 降到 600 后，稳态速度
//   `v∞ = a·dt/(1-damping) = 3.4a` 会低于阈值 0.1，若不开此开关，系统会在**受力未平衡**
//   时被判静止并**永久冻结** —— 实测 20000 步内 span 恒等于播种半径、maxV 恒为 0.00，
//   看起来像"已收敛"（判据 #8/#313：统计量恒等 ⇒ 先怀疑测量工具）。
export const AGG_PHYSICS_CONFIG: PhysicsConfig = {
  theta: 0.6,
  // 600 → 5400（2026-09-17，③-D）：`communityRadius` 系数 2.2 → 6 后团块半径放大
  // 2.3~4.4 倍，团间距必须同步抬高，否则只是把「一团糊的点」换成「一团糊的团」。
  // 定值来自 `graphAggregate.settle.test.ts` case E 的四档实测对账（同一条 dnn 分布上
  // 用新旧两套半径分别算几何）：
  //   rep   diaOverGap(典型团/间距)   overlapRatio      vs 改动前(1.13 / 0.455)
  //   600      2.34                    0.94             恶化
  //   2400     1.34                    0.575            恶化
  //   5400     1.03                    0.38             **优于**
  //   9600     0.83                    0.145            更优，但 fit 态团尺寸再缩 42%
  // ⇒ 取 5400：这是「典型团穿插开始优于改动前」的转折点，而 fit 态团直径只缩 20%
  //   （9600 要缩 42%，全局观感代价过大）。
  // ⚠ 已知残余：重尾巨桶（921 成员 ⇒ r=192）的穿插仍比改动前差（diaMaxOverGap
  //   2.75 → 5.25）—— 旧公式的上限 44 把重尾压平了，新公式为满足判据①必须放开它。
  //   根治需要团间碰撞消解（把重叠的团推开），不在本次范围。
  repulsion: 5400,
  gravity: 8,
  damping: 0.85,
  dt: 0.6,
  springForce: 0.04,
  springDamping: 0.9,
  maxVelocity: 12,
  // 重力 = 均匀加速度场（力 ∝ 自身质量）—— 见上方「形状修正」；不加则回到环状分布
  gravityScalesWithMass: true,
  // 关掉「全体近乎静止即早退」的性能兜底 —— 见上方「必须同时开 keepSimulating」
  keepSimulating: true,
};

/** 聚合布局的半跨度上限（世界坐标，按 L∞ 度量）。超出即**等比**缩回，形状完全保留。
 *
 *  2026-09-16 起它的**角色变了**：参数标定后物理自己就收敛在 span ≈ 2400~2600
 *  （见 AGG_PHYSICS_CONFIG 的扫描结论）⇒ 本常量退化为**安全阀**，正常情况下不触发。
 *  留着它是因为「物理越界」在真实数据上并非不可能：用户拖拽施加外力、社区粒度突变
 *  （桶数从 200 变到 800）、极端度分布 —— 这些都会让平衡点外移。
 *  触发时的语义是「把尺度拿回来」，等比缩放不改变形状。
 *
 *  为什么不靠它当主机制（上一版的教训）：它只保证**尺寸**可控，不保证**形状**正确 ——
 *  实测把 200 个已经把跨度涨到 13040 的社区等比缩到 3000，得到的仍然是
 *  「外圈密、中心空」的环状分布（rms/R ≈ 0.95、inner ≈ 0.01），
 *  因为等比缩放不改变构型。形状必须由物理跑对（见 gravityScalesWithMass）。
 *
 *  取 3600（2026-09-17，③-D 由 1500 两度上调）：repulsion 600 → 5400 之后
 *  实测平衡跨度升到 5306（case E，接退火、生产同源判据）⇒ 旧值 1500 会**每帧触发**
 *  归一化，把物理真实的平衡尺度压成常数 —— 那正是「判据的量纲与另一机制的副作用同源」
 *  这个坑（见 `AGG_SETTLE_PX` 第一版的教训）。
 *
 *  ⚠ 本值是 **`maxAbs`（L∞ 半径）** 的口径，**不是** span（`max(W,H)`）：
 *  `normalizeAggregateScale` 比的正是 `max(|x|,|y|)`。第一版按「span ≤ 2×halfSpan」
 *  把它定成 3000，实测仍触发 **72 次**（8000 步）—— 量纲错配让判据对触发完全无感。
 *  3600 ≈ 实测 maxAbs 峰值 + 20% 余量。回归断言见 case G：
 *  `normTriggered === 0` 且 `maxAbsPeak ≤ halfSpan`。
 *
 *  fit 后的观感由它和 `communityRadius` 的上限**共同**决定：fit zoom 走真实节点 bbox
 *  （`handleFitAll`：`min(0.8W/bboxW, 0.8H/bboxH, 2)`），而 bbox ≈ 聚合跨度 + 2×maxRadius
 *  ⇒ 抬 rep 会降 zoom、抬社区半径也会降 zoom，两者都会缩小屏幕上的团尺寸。 */
export const AGG_LAYOUT_HALF_SPAN = 3600;

/** 聚合物理的收敛判据：**屏幕上看得见的最大位移**小于 `AGG_SETTLE_PX` 像素、且连续保持
 *  `AGG_SETTLE_STEPS` 步 ⇒ 判定已收敛，停止步进。
 *
 *  为什么用「屏幕位移」而不是既有的 `isSystemStable(nodes, 0.15)`：
 *  后者在真实聚合图上**永远不成立** —— 质量跨 3 个数量级（1~553）叠加 maxVelocity 夹取，
 *  静置 8000 步后仍有 47%~94% 的节点速度 > 1（实测，见报告 §6.12）⇒
 *  `idleCounterRef` 永不增长 ⇒ 渲染循环不跳帧 ⇒ 24288 个节点一直 60fps 重绘。
 *
 *  ⚠⚠ 判据的量纲**不能与另一个机制的副作用同源**（本闸第一版就是这么错的，当天被实测推翻）：
 *  第一版量的是「L∞ 半跨度的相对变化」，而半跨度正是 `normalizeAggregateScale` 的**输出** ——
 *  归一化一触发就把它钉成 `AGG_LAYOUT_HALF_SPAN` 常数 ⇒ relChange 恒为 0 ⇒ 60 步后必然
 *  判「已收敛」，**与布局是否真的停了无关**。第二版改为量**位移**：位置是归一化的输入侧量，
 *  且「最大位移 × zoom」直接就是用户眼睛能看到的变化。同族判据见 #255/#313。 */
export const AGG_SETTLE_PX = 0.5;
export const AGG_SETTLE_STEPS = 60;

// ─────────────────────────────────────────────────────────────────────────────
// 收敛检测（2026-09-17 自 GraphView 抽出）
//
// 为什么必须抽成纯函数：这段判据的**阈值**（`AGG_SETTLE_PX` / `AGG_SETTLE_STEPS`）是要被
// 标定的对象，而标定必须用**同一份代码**跑 —— 内联在组件里时，标定脚本只能把判据重抄一遍，
// 抄的那一刻「被测对象」就与生产分叉了（同族：判据 #7/#313）。
// ─────────────────────────────────────────────────────────────────────────────

export interface AggregateSettleState {
  /** 窗口起点快照（`2n` 个分量），空数组 ⇒ 本步只建窗口、不做判定。 */
  snap: Float64Array;
  calmSteps: number;
  settled: boolean;
}

export function createAggregateSettleState(): AggregateSettleState {
  return { snap: new Float64Array(0), calmSteps: 0, settled: false };
}

/**
 * 语义 = 「最近 `steps` 步内，没有任何聚合节点的**累计**位移超过 `px` 个屏幕像素」。
 *
 * 两个刻意的设计点（都有实测教训，勿轻易改回）：
 *   · 用「相对**窗口起点**的累计漂移」而不是「相邻两步之差」—— 后者对每步 0.049px 的匀速
 *     漂移判「静」，而 10 秒能漂 5px，肉眼可见（见 AGG_SETTLE_PX 注释）。
 *   · 换算到**屏幕像素**（`drift × zoom`）而不是世界坐标 —— 与「用户是否还看得见变化」同量纲。
 *
 * ⚠ 判据的量纲**不能与另一个机制的副作用同源**：第一版量的是「L∞ 半跨度的相对变化」，而
 * 半跨度正是 `normalizeAggregateScale` 的**输出**（一触发就被钉成 `AGG_LAYOUT_HALF_SPAN`）
 * ⇒ relChange 恒 0 ⇒ 60 步后必然判「已收敛」，与布局是否真的停了无关。
 *
 * @returns `driftPx` 本步漂移（供日志）；`justSettled` 是否在本步**首次**判为收敛。
 */
export function updateAggregateSettle(
  state: AggregateSettleState,
  nodes: ReadonlyArray<{ x: number; y: number }>,
  zoom: number,
  px: number = AGG_SETTLE_PX,
  steps: number = AGG_SETTLE_STEPS,
): { driftPx: number; justSettled: boolean } {
  const n = nodes.length;
  if (state.snap.length !== n * 2) {
    state.snap = new Float64Array(n * 2);
    for (let i = 0; i < n; i++) {
      state.snap[2 * i] = nodes[i].x;
      state.snap[2 * i + 1] = nodes[i].y;
    }
    state.calmSteps = 0;
    return { driftPx: 0, justSettled: false };
  }
  let maxD2 = 0;
  for (let i = 0; i < n; i++) {
    const dx = nodes[i].x - state.snap[2 * i];
    const dy = nodes[i].y - state.snap[2 * i + 1];
    const d2 = dx * dx + dy * dy;
    if (d2 > maxD2) { maxD2 = d2; }
  }
  const driftPx = Math.sqrt(maxD2) * zoom;
  if (driftPx >= px) {
    // 还在动 ⇒ 把窗口起点挪到当前位置，重新计时
    for (let i = 0; i < n; i++) {
      state.snap[2 * i] = nodes[i].x;
      state.snap[2 * i + 1] = nodes[i].y;
    }
    state.calmSteps = 0;
  } else {
    state.calmSteps++;
  }
  let justSettled = false;
  if (!state.settled && state.calmSteps >= steps) {
    state.settled = true;
    justSettled = true;
  }
  return { driftPx, justSettled };
}

// ─────────────────────────────────────────────────────────────────────────────
// 布局退火（2026-09-17 新增）：让聚合物理**真的停下来**
//
// 【为什么必须有它】标定 A 组用的是**均匀质量**（200 × 73，无重尾），实测
// `stuckAtMax = 0`（一个节点都没贴 maxVelocity）、`maxV` 只有 0.84~3.03，
// 然而 `settledAt = -1`（5000 步内闸一次都没触发）、`calmSteps` 只到 1~2。
// ⇒ 「永不收敛」**不是重尾质量的锅**，而是阻尼系统在平衡点附近的**残余漂移**：
//   速度由 `v∞ = a·dt/(1-damping)` 决定，只要合力 a 不严格为 0，速度就不衰减到 0
//   —— 而 Barnes-Hut 每步重建 quad 树，节点跨越 cell 边界时远场近似会跳变，
//   合力因此**不会**严格为 0（这是算法的固有性质，不是缺陷）。
//
// 【用户可见的代价】`maxV ≈ 0.84~3.03` ⇒ 每步位移 `v·dt = 0.5~1.8` 世界单位，
// 60 步累计 30~108 世界单位，`zoom = 0.2` 时是 **6~21 屏幕像素** ⇒ 远超闸的 0.5px
// ⇒ `calmSteps` 每步被重置 ⇒ 闸永不触发 ⇒ **物理永不停止 ⇒ 渲染永不跳帧**
// ⇒ 同一个根因的两个症状：「图永远在飘」+「一直卡」。
//
// 【为什么是退火而不是改阈值】闸的阈值（`AGG_SETTLE_PX = 0.5px`）量的是「用户是否
// 还看得见变化」，这个语义没有问题；有问题的是**布局永远不会不动**这个事实。
// 把阈值从 0.5px 放宽到 20px 能让闸变绿，但用户依然看得见那 20px 的漂移 ——
// 那是把标定变成自证（判据 #329 同族）。退火让布局**真的停下来**，
// 闸不需要改一个字节就会自然满足。
//
// 【为什么按步数触发，而不是按「尺度是否稳定」】实测 case B（重尾）在 2000→8000 步
// 的 span 只从 1946 涨到 2112（**持续同向膨胀**），每 300 步的相对变化仅 0.42%
// ⇒ 「变化率小」无法把「缓慢膨胀中」与「已平衡」区分开（同族陷阱：拿一个与机制
// 副作用同源的量当判据，见 `AGG_SETTLE_PX` 第一版的教训）。按步数触发没有这个问题，
// 代价只是把「成形期」钉成一个常数，而该常数由实测给出。
// ─────────────────────────────────────────────────────────────────────────────

/** 退火前的「全温」步数：这段时间物理不受退火影响，自由演化到成形。
 *
 *  取值依据：标定实测 span 在 2000 步内即接近终态（case A 1582 / case B 2112），
 *  取 900（≈15 秒 @60 步/秒）留余量 —— 退火开始得越晚越安全，代价只是等待时间。
 *
 *  ⚠ 「60 步/秒」不是修辞，而是下方 `AGG_PHYS_STEP_MS` 钉死的**物理步频**。
 *  两者的耦合必须放在一起看：改了这个速率，就等于改了这个常数的真实含义。 */
export const AGG_ANNEAL_START_STEPS = 900;

/** 聚合物理的**固定步长**（毫秒）：物理步频 = 1000/这个值 = 60 步/秒。
 *
 *  【为什么需要它 —— 一次实测推翻的假象】
 *  改前物理步进的节流是「每 6 帧一步」（`frameCounterRef.current % 6 === 0`），
 *  即步频 = fps/6。设计时默认 fps≈60 ⇒ 10 步/秒，够用。但实测（fixture 24288 节点、
 *  浏览器真机探针）fit 态 fps 只有 **10.8** ⇒ 实际 ~1.8 步/秒 ⇒ 需要 1100 步的
 *  「全温 900 + 退火到闸触发」要 **~611 秒**，而探针只预热 45 秒（= 531 步，连全温期
 *  都没走完）⇒ **退火从未开始**、`settled` 零命中。
 *
 *  ⇒ 教训：「每 N 帧一步」把**物理时间**绑在了**渲染帧率**上，而帧率正是要被修复的
 *    那个量（节点圆画得慢 ⇒ fps 低）。于是形成正反馈：
 *        绘制慢 ⇒ fps 低 ⇒ 物理更慢 ⇒ 布局停不下来 ⇒ 位图构建闸不放行 ⇒ 绘制继续慢
 *    唯一能打断这个环的位图，恰好被它自己挡住了。
 *  ⇒ 所以步频必须由**墙上时间**决定，与 fps 无关。 */
export const AGG_PHYS_STEP_MS = 1000 / 60;

/** 单帧最多补多少物理步（封顶「追帧债」）。
 *
 *  没有它，切回前台 / 长任务后累积的秒级时间差会在**一帧内**补出几千步
 *  ⇒ 主线程一次性卡死（比跳步更糟）。取 8 步 = 133ms 的追帧上限：
 *  恰好覆盖 fps 低到 7.5 的情形（8 步/帧 × 7.5 帧/秒 = 60 步/秒，账能走平），
 *  再低就自愿降速（宁可物理变慢，也不让主线程爆）。 */
export const AGG_PHYS_MAX_STEPS_PER_FRAME = 8;

/** 退火期每步把速度上限乘上它。指数衰减 ⇒ 观感是「越来越慢」，不是「突然冻住」。 */
export const AGG_ANNEAL_DECAY = 0.97;

/** 退火终值下限（乘在 `maxVelocity` 上）。
 *
 *  取值依据：要让闸触发，需 `maxVelocity·scale·dt·60·zoom < AGG_SETTLE_PX`。
 *  取最坏情形 `zoom = 1`（视野越放大、闸越严）：`12·scale·0.6·60 < 0.5`
 *  ⇒ `scale < 1.16e-3`。取 4e-4 留 2.9 倍余量 ⇒ 任何 zoom ≤ 2.9 都满足。
 *  ⚠ 它同时是「残余抖动」的上界：`12 × 4e-4 = 0.0048` 世界单位/步 ⇒
 *    `zoom = 2.9` 时屏幕位移 0.008px/步，远低于人眼阈值。 */
export const AGG_ANNEAL_MIN_SCALE = 4e-4;

export interface AggregateAnnealState {
  /** 退火时间轴：已推进的物理步数。 */
  steps: number;
  /** 当前温度因子 ∈ [`AGG_ANNEAL_MIN_SCALE`, 1]，乘到 `maxVelocity` 上。 */
  scale: number;
}

export function createAggregateAnnealState(): AggregateAnnealState {
  return { steps: 0, scale: 1 };
}

/** 用户改了布局（拖动节点 / 重建聚合图）⇒ 重新加热到全温并重新计时。 */
export function reheatAggregateAnneal(state: AggregateAnnealState): void {
  state.steps = 0;
  state.scale = 1;
}

/**
 * 推进一步退火，返回**本步应使用的速度上限**。
 *
 * 前 `startSteps` 步返回 `baseMaxVelocity`（全温）；之后每步乘 `decay`，
 * 直到 `baseMaxVelocity × minScale`。
 *
 * ⚠ 为什么衰减 `maxVelocity` 而不是 `repulsion`/`gravity`：后者会改变**平衡尺度**
 *   `R* = √(rep·M/g)`（只有等比缩放才保尺度，但那等于把「力变小」再乘回去），
 *   而 `maxVelocity` 是纯运动学上限 —— 衰减它只压**速度**，不动**构型**。
 *   实测 case A 的 `v∞ = 0.84~3.03` **远低于**原上限 12 ⇒ 上限从不生效 ⇒
 *   只要它降到 `v∞` 以下，速度就被真实夹住并继续下降（这正是退火生效的机制）。
 */
export function updateAggregateAnneal(
  state: AggregateAnnealState,
  baseMaxVelocity: number,
  startSteps: number = AGG_ANNEAL_START_STEPS,
  decay: number = AGG_ANNEAL_DECAY,
  minScale: number = AGG_ANNEAL_MIN_SCALE,
): number {
  state.steps++;
  if (state.steps > startSteps) {
    state.scale = Math.max(minScale, state.scale * decay);
  }
  return baseMaxVelocity * state.scale;
}
