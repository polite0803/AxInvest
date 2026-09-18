/**
 * 标签布局：从「候选标签」里挑出**实际该画的那几个**，让任何缩放级别下的文字都不糊成一团。
 *
 * ── 为什么必须抽成一份实现（2026-09-17）──
 * 仓库里曾有**两条**标签绘制路径，判据不一致：
 *   · `drawNodesOptimized`（全折叠/无社区路径）—— 有「网格分格 + 矩形占位」双重去重；
 *   · `drawExpandedCommunity`（部分展开 / **隐式聚合**路径）—— 只有「按度数取 Top-N」，
 *     **没有任何空间去重**，250~500 个标签直接叠着画。
 * 而 `shouldUseClusterRender = (aggActive || clusterMode || forceCluster) && geomReady`
 * ⇒ **大图自动聚类（aggActive=true）必然命中后者** ⇒ 用户放大到能看清单个社区时，
 * 看到的是一片完全重叠的字（实测见 `docs/audits/AUDIT-wiki-graph-ux-2026-09-17.md`）。
 * 同一语义两处实现、只有一处做了正确的事 —— 这正是「必然腐烂」的温床（判据 #299），
 * 因此这里收敛为唯一实现，两条路径都调它。
 *
 * ── 判据链（顺序不可换）──
 *   ① **网格分格**：按候选包围盒最短边 / `gridDivisions` 切格，每格只留 `size` 最大者。
 *      为什么不能直接按 `size` 取 Top-N：高度数节点在空间上是**聚集**的，Top-N 会把
 *      标签全堆在同一片区域（实测 6000 节点图左侧糊成一堵白墙）。
 *   ② **cap 截断**：仍超上限时按 `size` 降序截断。
 *   ③ **矩形占位**：逐个尝试放置，与已放置矩形相交者跳过。
 *      ① 决定「分布」，③ 决定「互不重叠」，缺一不可。
 *
 * ── 为什么矩形要留白（gapRatio）──
 * 旧实现只判「几何相交」，于是相邻标签**紧贴**（边挨边）—— 屏幕上就是一整块连续文字，
 * 读不出词的边界，观感与重叠无异。故矩形四周各外扩 `gapRatio × 字号`：
 * 宁可少画几个，也不画成一堵墙。这是本次「看得清」的主要来源。
 *
 * ── 量纲（易错点）──
 * 全部比较都在**世界坐标**里做，因为绘制发生在 `ctx.scale(zoom, zoom)` 之后，
 * 且字号已经按 `屏幕字号 / zoom` 换算过 ⇒ 世界坐标里的「距离/尺寸」与屏幕成**同一比例**，
 * 等价于屏幕坐标比较。**不要**在这里再乘/除一次 zoom，否则就是二次缩放（同族缺陷：边线宽
 * 曾写成 `baseWidth * zoom`，落屏变成 ∝ zoom² 恒亚像素）。
 *
 * 纯函数且**确定性**：无随机、无时间依赖、比较函数在 `size` 相同时按 `id` 兜底排序 ——
 * 同一份输入必然挑选出同一批标签，否则每帧选出的集合会抖动。
 */

/** 一个候选标签（对应一个可能被标注的节点）。 */
export interface LabelCandidate {
  id: string;
  /** 节点中心（世界坐标）。 */
  x: number;
  /** 节点中心（世界坐标）。 */
  y: number;
  /** 节点绘制半径（世界坐标）—— 同时作为「重要度」排序键（度数代理）。 */
  size: number;
  /** 已经截断过的显示文本。 */
  title: string;
}

export interface LabelSelectOptions {
  /** 世界单位的字号（= 屏幕目标字号 / zoom）。 */
  fontSizeWorld: number;
  /** 本帧最多画多少个标签（硬上限）。 */
  cap: number;
  /** 标签相对节点下缘的下移量（世界单位）。 */
  labelOffsetWorld: number;
  /** 文本宽度测量（世界单位）；由调用方注入 `ctx.measureText` —— 纯函数不碰 canvas。 */
  measure: (text: string) => number;
  /** 网格等分数：候选包围盒最短边 / 该值 = 格边长。默认 12。 */
  gridDivisions?: number;
  /** 矩形四周留白（相对字号的倍数）。默认 0.3。 */
  gapRatio?: number;
  /** 占位矩形数量上限（防 O(k²) 无界增长）。默认 400。 */
  maxPlaced?: number;
}

/** 被选中的标签：附带算好的绘制锚点，调用方直接 `fillText(title, labelX, labelY)`。 */
export interface PlacedLabel extends LabelCandidate {
  labelX: number;
  /** 标签矩形**左上角** y（世界坐标）—— 与 `textBaseline = "top"` 配合。 */
  labelY: number;
}

const DEFAULT_GRID_DIVISIONS = 12;
const DEFAULT_GAP_RATIO = 0.3;
const DEFAULT_MAX_PLACED = 400;

export function selectLabelsToDraw(
  candidates: ReadonlyArray<LabelCandidate>,
  opts: LabelSelectOptions,
): PlacedLabel[] {
  const cap = Math.max(0, Math.floor(opts.cap));
  if (candidates.length === 0 || cap === 0) {
    return [];
  }
  const gridDivisions = opts.gridDivisions ?? DEFAULT_GRID_DIVISIONS;
  const gapRatio = opts.gapRatio ?? DEFAULT_GAP_RATIO;
  const maxPlaced = opts.maxPlaced ?? DEFAULT_MAX_PLACED;
  const { fontSizeWorld, labelOffsetWorld, measure } = opts;

  // ── ① 网格分格：每格留 size 最大者 ──
  // 网格尺度取**候选包围盒**（不是视口世界范围）：视口范围随缩放变化、与节点分布尺度脱钩，
  // zoom 大时网格会退化成「每节点一格」= 等于没筛（已踩过的坑）。
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const c of candidates) {
    if (c.x < minX) {
      minX = c.x;
    }
    if (c.y < minY) {
      minY = c.y;
    }
    if (c.x > maxX) {
      maxX = c.x;
    }
    if (c.y > maxY) {
      maxY = c.y;
    }
  }
  const cellSize = Math.max(1e-6, Math.min(maxX - minX, maxY - minY) / gridDivisions);
  const bestPerCell = new Map<string, LabelCandidate>();
  for (const c of candidates) {
    // 全部候选共用一个坐标原点，格边界必须是**绝对**刻度（用 minX/minY 平移），
    // 否则同一处节点在不同帧会被分到不同格 ⇒ 选出的集合逐帧抖动。
    const cellKey = `${Math.floor((c.x - minX) / cellSize)},${Math.floor((c.y - minY) / cellSize)}`;
    const prev = bestPerCell.get(cellKey);
    if (!prev || c.size > prev.size || (c.size === prev.size && c.id < prev.id)) {
      bestPerCell.set(cellKey, c);
    }
  }

  // ── ② cap 截断 ──
  let pool = [...bestPerCell.values()];
  if (pool.length > cap) {
    pool = pool.slice().sort((a, b) => (b.size - a.size) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    pool.length = cap;
  }

  // ── ③ 矩形占位（含留白）──
  const padW = fontSizeWorld * gapRatio;
  const rectH = fontSizeWorld * (1 + gapRatio);
  const placed: { x0: number; y0: number; x1: number; y1: number }[] = [];
  const out: PlacedLabel[] = [];
  for (const c of pool) {
    const labelX = c.x;
    const labelY = c.y + c.size + labelOffsetWorld;
    const halfW = measure(c.title) / 2 + padW;
    const rect = {
      x0: labelX - halfW,
      y0: labelY,
      x1: labelX + halfW,
      y1: labelY + rectH,
    };
    let overlaps = false;
    for (const q of placed) {
      if (!(rect.x1 < q.x0 || rect.x0 > q.x1 || rect.y1 < q.y0 || rect.y0 > q.y1)) {
        overlaps = true;
        break;
      }
    }
    if (overlaps) {
      continue;
    }
    placed.push(rect);
    out.push({ ...c, labelX, labelY });
    if (placed.length >= maxPlaced) {
      break;
    }
  }
  return out;
}
