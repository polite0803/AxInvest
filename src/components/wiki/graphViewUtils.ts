// SPDX-License-Identifier: AGPL-3.0-only

// GraphView 纯函数工具集（F8 拆分第一步：从 GraphView.tsx 搬出与组件状态无关的
// 布局持久化 / 配色 / 节点尺寸 / 颜色工具，降低主文件体积）。
// 仅 import 类型（type-only），运行时与 GraphView.tsx 无相互依赖。

import { theme } from "antd";
import type { PhysicsNode } from "./graphPhysics";
import type { GraphEdgeType, GraphNode, GraphNodeType } from "./GraphView";

export type TokenType = ReturnType<typeof theme.useToken>["token"];

// ── 社区调色板：必须 ≥ 桶数 ──
//
// 归档缺陷（2026-09-16）：原调色板只有 12 色，而拓扑归并后的桶数是 200
// ⇒ 取色表达式 `communityPalette[cid % 12]` 把 200 个社区压进 12 个色相，
// 相邻社区颜色近乎随机 ⇒ 以「相邻节点同色率 / 随机期望」为刻度的分组可见性判据
// **上限被表示空间容量卡死**（实测 1.062×，而阈值是 1.5 —— 即便分组在空间上完美分离
// 也到不了）。这类缺陷的正确修法是**扩表示空间**，不是调阈值（判据 #289）。
//
// 前 12 项保持与历史完全一致（小图 / 社区数 ≤ 12 时取色逐字节不变，避免观感与既有
// 截图漂移）；第 13 项起按黄金角在 HSL 空间生成，直到 COMMUNITY_PALETTE_SIZE。
const BASE_COMMUNITY_COLORS = [
  "#5B8FF9",
  "#61DDAA",
  "#65789B",
  "#F6BD16",
  "#7262FD",
  "#78D3F8",
  "#9661BC",
  "#F6903D",
  "#008685",
  "#F08BB4",
  "#1E90FF",
  "#32CD32",
];

/** 调色板容量上限。取 200 = 现行「社区 → 聚合并」的目标桶数，使 `cid % len` 恒等映射。 */
export const COMMUNITY_PALETTE_SIZE = 200;

function hslToHex(h: number, s: number, l: number): string {
  const sN = s / 100;
  const lN = l / 100;
  const c = (1 - Math.abs(2 * lN - 1)) * sN;
  const hp = ((h % 360) + 360) % 360 / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  let r = 0;
  let g = 0;
  let b = 0;
  if (hp < 1) {
    r = c;
    g = x;
  } else if (hp < 2) {
    r = x;
    g = c;
  } else if (hp < 3) {
    g = c;
    b = x;
  } else if (hp < 4) {
    g = x;
    b = c;
  } else if (hp < 5) {
    r = x;
    b = c;
  } else {
    r = c;
    b = x;
  }
  const m = lN - c / 2;
  const to255 = (v: number) => Math.round(Math.min(255, Math.max(0, (v + m) * 255)));
  return `#${[to255(r), to255(g), to255(b)].map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

function buildCommunityPalette(): string[] {
  const out = [...BASE_COMMUNITY_COLORS];
  // 黄金角步进色相：相邻序号色相差 ≈ 137.5°（而非规则色轮的等距分格，
  // 后者在取模回绕处会出现肉眼难分的近邻色）；饱和度 / 明度各取三档交替，
  // 在不丢色相分辨力的前提下把可用色域再翻数倍。
  const GOLDEN_ANGLE = 137.508;
  const SAT = [64, 78, 56];
  const LIGHT = [56, 46, 68];
  for (let i = out.length; i < COMMUNITY_PALETTE_SIZE; i++) {
    const k = i - BASE_COMMUNITY_COLORS.length;
    out.push(hslToHex((k * GOLDEN_ANGLE) % 360, SAT[k % 3], LIGHT[k % 3]));
  }
  return out;
}

export const communityPalette = buildCommunityPalette();

/** `communityRadius` 的四个参数 —— 拆成常量是为了让**标定脚本与测试引用同一份数值**
 *  （否则标定对象的参数会与生产分叉，同族判据 #7/#313）。 */
export const COMMUNITY_RADIUS_BASE = 10;
/** 系数：须 > `2·nodeSizeTypical/√π` ≈ 5.642，见 `communityRadius` 注释的推导。 */
export const COMMUNITY_RADIUS_K = 6;
export const COMMUNITY_RADIUS_MIN = 16;
export const COMMUNITY_RADIUS_MAX = 240;

/** 社区视觉半径（世界坐标）—— 成员散布半径与背景气泡半径**必须同源**。
 *
 *  归档缺陷（2026-09-16）：气泡半径原按**节点包围盒**算（`(maxX-minX)/2 + 40`），
 *  当社区成员尚未在坐标空间里被分离时，每个社区的 bbox ≈ 整张画布
 *  ⇒ 200 个半径 ≈ 0.78×画布对角线的极淡椭圆完全重叠 ⇒ 视觉产物只剩「一片均匀染色」
 *  （实测 p50/p95 = 0.778/0.922；判据 #287/#298）。
 *  本函数是「社区在坐标空间里实际占据多大」的唯一真相源：布局分发（applyAggregateLayout）
 *  与气泡绘制（drawClusterRegions）都取它，避免两处各算各的而错位。
 *
 *  ── 系数 2.2 → 6 的判据（2026-09-17，③-D）─────────────────────────────────
 *  用户报「放大到上限 5 倍仍然看不清细节」。定量根因**不是**放大倍数，而是团内
 *  **相邻节点的间距 / 节点半径**这个比值太小，且该比值**与 zoom 无关**：
 *      相邻节点屏幕间距 = r·√(π/n)·zoom       （面积均匀铺设 ⇒ 间距 = √(面积/数量)）
 *      节点屏幕半径     = nodeSize·zoom        （nodeDrawRadius 只设下限、无上限）
 *      ⇒ 可分辨（间距 ≥ 2×半径）⟺ **r/√n ≥ 2·nodeSize/√π**
 *  这个式子里 zoom 被完全约掉 ⇒ 把一团糊的点整体放大 N 倍，仍是同样糊的一团。
 *  旧系数 2.2 给出 r/√n ≤ 2.2（n 大时还被上限 44 顶住，实际更小），而 `getNodeSize`
 *  实测典型值 5 ⇒ 需要 r/√n ≥ 2×5/√π ≈ **5.642** ⇒ 旧公式差 2.6 倍。
 *
 *  取 K=6（>5.642，余量 6%）的依据是**判据本身**：因为 6 > 2·nodeSize/√π 对
 *  nodeSize ≤ 5.3 恒成立 ⇒ 常见尺寸的节点在任意 zoom 下都能分开。上限抬到 240
 *  （旧 44）的依据：真实归并后最大桶 921 成员 ⇒ 需 6√921 ≈ 182，44 会把它压到
 *  1/4；240 允许到 n≈1434 才封顶，覆盖实测重尾分布的全部桶。
 *
 *  ⚠ 它的代价是团块变大 ⇒ 团与团会互相穿插，而团间距由**另一套尺度**决定
 *  （聚合物理的平衡半径 R* = √(repulsion·M/gravity)）⇒ 两者必须同步标定，
 *  只改这里会让社区互相吞并。标定见 `graphAggregate.settle.test.ts` case E。 */
export function communityRadius(count: number): number {
  return Math.max(
    COMMUNITY_RADIUS_MIN,
    Math.min(
      COMMUNITY_RADIUS_MAX,
      COMMUNITY_RADIUS_BASE + Math.sqrt(Math.max(0, count)) * COMMUNITY_RADIUS_K,
    ),
  );
}

/** 背景气泡半径 = 社区半径的固定倍数（气泡略包住节点团，而非与之相等）。 */
export const COMMUNITY_BUBBLE_RADIUS_SCALE = 1.15;

/** 节点在屏幕上的最小可见半径（屏幕像素）。低于 ~1px 的点在抗锯齿后会被
 *  稀释成背景灰 —— 实测 2.4 万个屏幕半径 0.24px 的点铺满画布，
 *  主画布「高饱像素」（chroma>30）占比只有 **0.002%**（画布 1140×768，已排除 minimap）。
 *
 *  ⚠ 该 0.002% 是**同口径**基线：早期记录的 0.14% 系旧 crop（1357×768）测得，
 *  而旧 crop **包含 minimap**（其自身非空 8%~12%）⇒ 0.14% 几乎全由缩略图贡献，
 *  与「主画布上节点是否可见」无关。裁剪口径一变，基线必须同口径重算（判据 #313）。
 *
 *  取 2.0 的依据：实测每帧可见节点约 1 万个（24288 × 当时的大图采样率 0.5；
 *  该「按全图规模固定 0.5」的判据已于 2026-09-17 改为按视口预算 —— 见
 *  `NODE_DRAW_BUDGET`：fit 态 rate ≈ 0.494，与旧值等价，故本标定仍然成立），
 *  画布 1357×768 ⇒ 单点 π×2² ≈ 12.6 px² ⇒ 理论上限覆盖 12.1%，
 *  扣除簇内重叠约 6%~8% —— 落在「能看出点云结构与颜色分组、又不糊成实心色块」的区间。
 *  实测对照：修后 A 阶段 5.379%、B 阶段 2.083%（改前分别为 2.693% / 0.002%）。 */
export const MIN_NODE_SCREEN_RADIUS = 2.0;

/** 位图烘制所用的参考 zoom：**屏幕下限就是按它换算成世界半径烘进去的**。
 *
 *  为什么需要这个常量（它替换了原来那句「把下限烘进位图是做不到的」）：
 *  位图是一张**栅格**，节点半径在烘制那一刻被固定成世界尺寸。但它落屏时的缩放比
 *  **恰好就是 `zoom`** —— 因为烘制覆盖的是世界 bbox，而 `drawImage` 的目标矩形
 *  也是同一个世界 bbox（见 `spriteWorldBBoxRef` 的注释），两者抵消。
 *  于是存在恒等关系：**「位图里 r 世界单位」在屏幕上恒为 `r · zoom` 像素**。
 *  既然有这个关系，就**可以**把屏幕下限烘进去：烘制时把节点半径抬到
 *  `max(真实半径, MIN_NODE_SCREEN_RADIUS / SPRITE_BAKE_ZOOM)` 即可 ——
 *  画布尺寸不用变大（烘制与世界 1:1），代价只是低 zoom 下节点比真实几何略粗，
 *  而那**正是矢量路径在做的同一件事**（`nodeDrawRadius` 抬下限），观感一致。
 *
 *  取值 0.12（2026-09-17 由 0.18 下调）：③-D 把聚合平衡跨度抬到 ~5400 之后，
 *  `handleFitAll` 的 bbox 同比变大 ⇒ **实测 fit 全图态 zoom 从 0.25 掉到 0.14**
 *  （probe-d3d-0917），而 0.18 的阈值会让位图在 fit 态**直接失效** ——
 *  实测 `sprite=false`、主画布 arc/帧回到 11770（= 退回到启用位图之前的水位）。
 *  ⇒ 旧依据「实测 fit zoom ∈ [0.20, 0.33]」随本次改动失效，按下调后的 0.14 重取
 *  （0.12 留 14% 余量）。烘入的世界半径 = 2.0 / 0.12 ≈ 16.7。
 *
 *  ⚠ 它与 `SPRITE_MAX_ZOOM`、`AGG_LAYOUT_HALF_SPAN`、`communityRadius` 是**联动**的：
 *  后两者决定 fit zoom，前者决定位图在哪段 zoom 区间可用。改任一个都要重跑
 *  `probe-d3d-0917.mjs` 确认 fit 档 `sprite=true` 且 arc/帧 ≈ 0。 */
export const SPRITE_BAKE_ZOOM = 0.12;

/** 位图烘制时使用的**最小**节点世界半径。
 *
 *  ⚠ 它不是「实测出来的一句声明」（原值 7.2 是硬写的），而是从两个已声明的量
 *  **派生** ⇒ 改 `MIN_NODE_SCREEN_RADIUS` 或 `SPRITE_BAKE_ZOOM` 时自动跟随。
 *  与 `buildBigGraphSpriteCache` 的烘制式 `(nodeSizeRef.get(id) ?? 6) * 1.2` 配合：
 *  取默认尺寸 6 ⇒ 7.2 < 11.1 ⇒ 下限生效（这正是「位图不再是灰雾」的机制）。 */
export const BIG_GRAPH_SPRITE_MIN_WORLD_RADIUS = MIN_NODE_SCREEN_RADIUS / SPRITE_BAKE_ZOOM;

/** 位图可用的 zoom 区间**上界**。
 *
 *  为什么需要一个上界（原判据只有下限）：位图节点半径随 zoom 线性增长
 *  （`16.7 · zoom`），而矢量路径的 `nodeDrawRadius` 是 `max(真实半径, 2/zoom)`。
 *  两者的**等价点**（位图点屏幕半径 = 矢量点屏幕半径）由下式给出：
 *      `(2 / SPRITE_BAKE_ZOOM) · z = 真实半径`  ⇒  `z = 5 × 0.12 / 2 = 0.30`
 *  超过它位图里的点会**明显比真实几何胖**（zoom = 1 时 16.7px vs 5px）。
 *  而高 zoom 下视口裁剪本来就有效（只画视口内的那部分点），矢量路径并不慢 ——
 *  位图在这个区间**既没必要也不准确**。
 *
 *  ⚠ 0.4 → 0.30（2026-09-17）：等价点随 `SPRITE_BAKE_ZOOM` 同步下移，
 *  两者必须一起改，否则「新 bake + 旧上限」会让位图在 0.30~0.40 区间画出偏胖的点。 */
export const SPRITE_MAX_ZOOM = 0.3;

/** 大图位图在该 zoom 下**是否可用**（否则走矢量路径）。
 *
 *  【这个判据修的是「两处守卫自相矛盾」（2026-09-17 实测登记的登记项）】
 *  改前是 `zoom × 7.2 ≥ 2.0` ⇒ `zoom ≥ 0.278`，而生产 fit 全图态实测 `zoom = 0.2046`
 *  ⇒ **位图在它唯一有价值的那个状态（fit 全图：24288 个节点全在视口内、矢量路径每帧
 *  要画约 1.2 万个 `arc`）下恰好不可用**；反过来在放大态它可用，但那时视口裁剪已生效、
 *  矢量路径本来就不慢。即：**收益区间与可用区间不相交** ⇒ 位图被构建
 *  （付最多 64MB 离屏分配 + idle 里 O(N) 绘制）却永远画不出来 —— 实测
 *  5 参 `drawImage` 一次都没出现过（参数个数集合恒为 {3}，全部来自背景/minimap）。
 *  把屏幕下限烘进烘制半径（见 `SPRITE_BAKE_ZOOM`）后，下限不再是**可用性**的约束，
 *  判据退化为一个纯粹的**区间**判断，收益区间（fit 态）被包进来。
 *
 *  fail-closed：`zoom` 为 0 / 负数 / NaN 一律 false ⇒ 走矢量路径
 *  （那条路径有屏幕下限，绝不会出现亚像素灰雾）。 */
export function spriteUsableAtZoom(zoom: number): boolean {
  return zoom >= SPRITE_BAKE_ZOOM && zoom <= SPRITE_MAX_ZOOM;
}

/** 节点绘制/命中半径（**世界坐标**）。
 *
 *  归档缺陷（2026-09-16）：节点此前只按**世界坐标半径**绘制（nodeSizeRef，默认 5）。
 *  相机 fit 全图时 zoom 会被压到 0.047~0.13，世界半径 5 的节点在屏幕上只剩 0.24~0.6px
 *  —— 亚像素。24288 个亚像素点在抗锯齿后被稀释成 meanChroma≈8.6 的灰雾：实测
 *  「关掉聚类模式」主画布 chroma>30 占比 **0.002%**、色相桶 **0/36**
 *  （肉眼与判据都等于"完全空白"）。
 *
 *  这与聚类开关无关：**任何**把 zoom 压小的路径都会命中同一缺陷（同族）。
 *  同一陷阱在本文件布局持久化的注释里已有一次前科（"一团亚像素噪点缩进屏幕"）。
 *
 *  ⚠ 这是**视觉半径与命中半径的唯一真相源**：绘制侧（drawExpandedNodes / 终极安全阀）
 *  与交互侧（findNodeAt）都必须取它。两处口径一旦分叉，就会出现「看得见、点不中」。
 *  zoom ≥ 0.4 时 worldSize ≥ 2.0/0.4 = 5 = nodeSizeRef 默认值 ⇒ 下限自动不生效，
 *  因此「节点随缩放正常变大」的既有观感不受影响。 */
export function nodeDrawRadius(worldSize: number, zoom: number): number {
  const minWorld = MIN_NODE_SCREEN_RADIUS / Math.max(zoom, 1e-6);
  return worldSize < minWorld ? minWorld : worldSize;
}

/** 每帧**节点**绘制预算（个）：本帧视口内候选超过它才降采样，否则全画。
 *
 *  【这个判据修的是「放大反而丢一半节点」（2026-09-17）】
 *  改前是 `isLargeGraph ? 0.5 : 1.0`，而 `isLargeGraph` 只看**全图**节点数
 *  （`nodes.length > 5000`）⇒ 24288 节点的图**恒**只画 50%，与 zoom 无关。两层后果：
 *    ① 放大后视口内候选已由 gridIndex 裁到几百~几千个（远在预算内），却仍被砍一半
 *       —— 纯损失，用户「越放大点越少」（同一缺陷还曾在 2026-09-15 造成「边少四分之三」，
 *       见 `drawExpandedCommunity` 里端点判据的修正注释）；
 *    ② 与位图区间行为不一致：位图（`spriteUsableAtZoom`，即 zoom ≤ `SPRITE_MAX_ZOOM`）
 *       是**无采样全量**烘制（`buildBigGraphSpriteCache` 遍历 nodes 全量）⇒ 同一视图在
 *       阈值两侧点数不同 = 跨阈值视觉跳变。
 *  现判据钉在「本帧视口内候选数」上：不超预算 ⇒ rate = 1。
 *
 *  取值 12000 的依据：改前 fit 全图态实测 `visibleNodes ≈ 12309`
 *  （= 24288 × 0.5，逐帧 arc 归因见 `output/verify-graph-2026-09-15`）
 *  ⇒ 取 12000 使 fit 态 rate ≈ 0.494，**与旧行为等价、性能不回退**；
 *  而 zoom ≥ 0.36 起候选降入预算 ⇒ rate = 1（全画）。 */
export const NODE_DRAW_BUDGET = 12000;

/** 每帧**边**绘制预算（条）。与 `NODE_DRAW_BUDGET` 同族，但取值依据独立：
 *  改前 fit 全图态边采样率 0.15（`zoom < 0.3` 分档）⇒ 本数据集 74791 条边实绘约
 *  1.1 万条、约 2.2 万个 Path2D 顶点（逐帧顶点归因实测 22422）。
 *  取 12000 使 fit 态 rate ≈ 0.16 —— 与旧值同量级，**顶点预算不前移**。 */
export const EDGE_DRAW_BUDGET = 12000;

/** 「本帧候选数 → 绘制比例」的**唯一**判据（节点层与边层共用）。
 *
 *  为什么必须同一处：两层此前是**两套独立分档**（节点看全图规模、边看 zoom 区间），
 *  于是同一帧里二者的稀疏度互不相关，观感上「点密边稀 / 点稀边密」随 zoom 无规律漂移；
 *  且边层大图分支在 `zoom ≥ 0.5` 后**封顶 0.5** ⇒ 放大到 5 倍也只有一半边，
 *  与节点层应有的「放大后全画」正好相反。收敛到一处后两层同源。
 *
 *  fail-open（返回 1 = 全画）而非返回 0：**画不出来比画得少更糟** —— 0 意味着整层消失，
 *  正是「全折叠空白」「位图被构建却画不出来」那类事故的形态。退化输入
 *  （NaN / Infinity / budget ≤ 0）一律返回 1，把「画多少」交回上层视口裁剪。 */
export function viewportDrawRate(candidateCount: number, budget: number): number {
  if (!Number.isFinite(candidateCount) || !Number.isFinite(budget)) { return 1; }
  if (!(budget > 0) || !(candidateCount > budget)) { return 1; }
  return budget / candidateCount;
}

/** 字符串 → 稳定 32 位整数哈希（用于「节点 → 虚拟聚类/聚合单元」的确定性分桶，
 *  以及渲染抖动种子：**必须确定性**，随机数会让重建后的布局/抖动每帧变化）。
 *
 *  2026-09-16 由 GraphView 本地函数迁出：`buildAggregateGraph`（graphAggregate.ts）
 *  需要同一个哈希来派生聚合节点的初速方向。两处各留一份实现必然分叉，
 *  因此提升为单一真相源。 */
export function hashStringToInt(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) - hash + str.charCodeAt(i)) | 0;
  }
  return hash;
}

export const getNodeColorMap = (token: TokenType): Record<GraphNodeType, string> => ({
  note: token.colorPrimary,
  concept: token.colorSuccess,
  entity: "#FA8C16",
  source: "#EB2F96",
});

// ── 布局持久化：localStorage 存储节点坐标 ──

// v2：旧版 initializePositions 用 `nodes.length * 2` 作布局半径（6000 节点 → 12000），
// 缓存下来的坐标尺度远大于视口，命中后不会重新初始化，即使补了自动 fit 也只会把
// 一团亚像素噪点缩进屏幕。升版前缀让这批陈旧布局自然失效（旧键在 LRU 清理中淘汰）。
const LAYOUT_STORAGE_PREFIX = "wiki_graph_layout_v2_";
// LRU 上限：最多保留 10 个 wiki 的布局，超出按 savedAt 时间淘汰最旧的
const LAYOUT_MAX_ENTRIES = 10;
// 单 wiki 布局超过此节点数则不持久化（避免万级节点序列化 500KB+ 逼近配额）
const LAYOUT_MAX_NODES = 2000;

interface SavedLayout {
  positions: Record<string, { x: number; y: number }>;
  savedAt: number;
  // D7: 相机视角持久化，刷新后回到上次导航区域（仅在非默认视角时保存）
  camera?: { x: number; y: number; zoom: number };
}

function pruneLayoutStorage(currentWikiId: string): void {
  // 收集所有布局条目，按 savedAt 升序，超出上限时删除最旧
  const entries: Array<{ wikiId: string; savedAt: number }> = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i);
    if (!key || !key.startsWith(LAYOUT_STORAGE_PREFIX)) { continue; }
    const wid = key.slice(LAYOUT_STORAGE_PREFIX.length);
    if (wid === currentWikiId) { continue; }
    try {
      const raw = localStorage.getItem(key);
      if (!raw) { continue; }
      const layout = JSON.parse(raw) as SavedLayout;
      entries.push({ wikiId: wid, savedAt: layout.savedAt || 0 });
    } catch {
      // 损坏的条目直接删除
      localStorage.removeItem(key);
    }
  }
  entries.sort((a, b) => a.savedAt - b.savedAt);
  // 已有条目数（不含当前）+ 当前 1 个 > 上限 → 删除最旧的
  const excess = entries.length + 1 - LAYOUT_MAX_ENTRIES;
  for (let i = 0; i < excess; i++) {
    localStorage.removeItem(LAYOUT_STORAGE_PREFIX + entries[i].wikiId);
  }
}

export function saveLayout(
  wikiId: string,
  nodes: PhysicsNode[],
  camera?: { x: number; y: number; zoom: number },
): void {
  // 节点数超阈值时不持久化（避免逼近 localStorage 配额）
  if (nodes.length > LAYOUT_MAX_NODES) { return; }
  try {
    const positions: Record<string, { x: number; y: number }> = {};
    for (const node of nodes) {
      positions[node.id] = { x: node.x, y: node.y };
    }
    const layout: SavedLayout = {
      positions,
      savedAt: Date.now(),
    };
    // D7: 仅在缩放偏离默认视角时持久化相机，避免默认视角的冗余存储
    if (camera && Math.abs(camera.zoom - 1) > 0.01) {
      layout.camera = { x: camera.x, y: camera.y, zoom: camera.zoom };
    }
    // 写入前做 LRU 清理，确保不超过 LAYOUT_MAX_ENTRIES
    pruneLayoutStorage(wikiId);
    localStorage.setItem(LAYOUT_STORAGE_PREFIX + wikiId, JSON.stringify(layout));
  } catch {
    // localStorage 可能已满，静默忽略
  }
}

export function loadLayout(wikiId: string): SavedLayout | null {
  try {
    const raw = localStorage.getItem(LAYOUT_STORAGE_PREFIX + wikiId);
    if (!raw) { return null; }
    return JSON.parse(raw) as SavedLayout;
  } catch {
    return null;
  }
}

/** 清除指定 wiki 的已保存布局（重新布局前调用） */
export function clearLayout(wikiId: string): void {
  try {
    localStorage.removeItem(LAYOUT_STORAGE_PREFIX + wikiId);
  } catch {
    // 静默忽略
  }
}

export function applySavedLayout(nodes: PhysicsNode[], saved: SavedLayout): boolean {
  let matched = 0;
  for (const node of nodes) {
    const savedPos = saved.positions[node.id];
    if (savedPos) {
      node.x = savedPos.x;
      node.y = savedPos.y;
      matched++;
    }
  }
  // 匹配率低于 30% 时整体放弃：清空位置，返回 false 让 initializePositions 重新圆形布局
  if (matched < nodes.length * 0.3) {
    return false;
  }
  // 匹配率 ≥ 30% 但部分未匹配：给未匹配节点做圆形分布，避免堆叠在原点
  const unmatched = nodes.filter((n) => !saved.positions[n.id]);
  if (unmatched.length > 0) {
    const radius = Math.max(200, Math.sqrt(unmatched.length) * 30);
    unmatched.forEach((n, i) => {
      const angle = (i / unmatched.length) * Math.PI * 2;
      n.x = Math.cos(angle) * radius;
      n.y = Math.sin(angle) * radius;
    });
  }
  return true;
}

/**
 * 各边的渲染样式。
 *
 * # ⚠ 2026-09-17：`link` / `backlink` 的色号必须与**画布底**拉开对比度
 *
 * 原实现把最常见的两类边（`link` 是全部 wikilink，实测一个 wiki 有 74,791 条；
 * `backlink` 次之）给了 `token.colorBorderSecondary` / `colorBorder`——
 * 这两个 token 的语义是「**分割线**」：它们只需要在**同色系面板上**分出一档，
 * 不需要在**画布底色**上被看见。而画布底色是 `colorBgLayout` 一族的暖色深底
 * （实测暗色主题下 `rgb(31,26,23)`），与 `#303030` 的亮度差只有约 2/255。
 *
 * 再乘上绘制时的 `globalAlpha`（低缩放 0.12，见 `drawEdgesOptimized`），
 * 屏幕上实际落下的对比度约 **2/255 ≈ 0.8%** —— 像素实测「亮度差 0~3% 的区间里
 * 一个像素都没有」，也就是**一条边也看不见**。这正是用户报的
 * 「公司/个人/行业之间没有任何关联关系」的视觉成因之一
 * （另一半是域不一致导致边被整条丢弃，见 `DanglingEdgeSummary`）。
 *
 * 改用**文本级** token（`colorTextTertiary` / `colorTextSecondary`）：它们按定义
 * 就必须在背景上可读，且带 alpha ⇒ 在 canvas 上与 `globalAlpha` 相乘后
 * 落在「淡但看得见」的区间（暗色下约 5%，即改动前的 6 倍以上）。
 *
 * ⚠ **不要动 `link.width`**：`GraphView.tsx` 的
 * `const width = dynamicWidth * (isRel ? 2 : 1) * (em.width / 0.4);`
 * 把 `0.4` 当作**全体边**的相对线宽基准（那是手抄常量，与这里的 `link.width`
 * 同值）。改它会等比缩放所有边的线宽，而不是只改 `link`。
 * 若要把那个基准收敛成单一来源，属独立改动，需同步改两处。
 */
export const getEdgeTypeStylesMap = (
  token: TokenType,
): Record<GraphEdgeType, { color: string; width: number; animated: boolean }> => ({
  link: { color: token.colorTextTertiary, width: 0.4, animated: true },
  backlink: { color: token.colorTextSecondary, width: 0.5, animated: true },
  reference: { color: token.colorSuccess, width: 0.5, animated: true },
  derived_from: { color: token.colorWarning, width: 0.5, animated: false },
  contradicts: { color: token.colorError, width: 0.6, animated: false },
  mapping: { color: token.colorInfo, width: 0.4, animated: true },
});

export const edgeTypeLabels: Record<GraphEdgeType, string> = {
  link: "wiki.graph.edgeType.link",
  backlink: "wiki.graph.edgeType.backlink",
  reference: "wiki.graph.edgeType.reference",
  derived_from: "wiki.graph.edgeType.derived",
  contradicts: "wiki.graph.edgeType.contradicts",
  mapping: "wiki.graph.edgeType.mapping",
};

// 节点颜色缓存：nodeId → color
export function buildNodeColorCache(
  nodes: GraphNode[],
  communities?: Map<string, number>,
  token?: TokenType,
): Map<string, string> {
  const cache = new Map<string, string>();
  const typeMap = token ? getNodeColorMap(token) : {
    note: "#1890ff",
    concept: "#52c41a",
    entity: "#fa8c16",
    source: "#eb2f96",
  };
  for (const node of nodes) {
    if (communities && communities.has(node.id)) {
      const cid = communities.get(node.id)!;
      cache.set(node.id, communityPalette[cid % communityPalette.length]);
    } else {
      cache.set(node.id, typeMap[node.type] || typeMap.note);
    }
  }
  return cache;
}

export function getNodeSize(node: GraphNode): number {
  const degree = node.linkCount + node.backlinkCount;
  if (node.type === "entity") { return Math.max(6, Math.min(22, 6 + degree * 0.8)); }
  if (node.type === "concept") { return Math.max(5, Math.min(18, 5 + degree * 0.6)); }
  return Math.max(4, Math.min(15, 4 + degree * 0.4));
}

// ── XML 转义（SVG 导出防注入） ──
export function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

// ── 颜色工具：支持 #RRGGBB / #RRGGBBAA / #RGB / rgb()/rgba() ──
export interface RGBA {
  r: number;
  g: number;
  b: number;
  a: number;
}

export function parseColor(color: string): RGBA | null {
  if (!color) { return null; }
  const hexMatch = color.match(/^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/);
  if (hexMatch) {
    const hex = hexMatch[1];
    if (hex.length === 3) {
      return {
        r: parseInt(hex[0] + hex[0], 16),
        g: parseInt(hex[1] + hex[1], 16),
        b: parseInt(hex[2] + hex[2], 16),
        a: 255,
      };
    }
    return {
      r: parseInt(hex.slice(0, 2), 16),
      g: parseInt(hex.slice(2, 4), 16),
      b: parseInt(hex.slice(4, 6), 16),
      a: hex.length === 8 ? parseInt(hex.slice(6, 8), 16) : 255,
    };
  }
  const rgbMatch = color.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)(?:\s*,\s*([\d.]+))?\s*\)$/);
  if (rgbMatch) {
    return {
      r: parseInt(rgbMatch[1], 10),
      g: parseInt(rgbMatch[2], 10),
      b: parseInt(rgbMatch[3], 10),
      a: rgbMatch[4] !== undefined ? Math.round(parseFloat(rgbMatch[4]) * 255) : 255,
    };
  }
  return null;
}

export function clamp(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, v));
}
