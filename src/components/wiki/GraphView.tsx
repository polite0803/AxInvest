// SPDX-License-Identifier: AGPL-3.0-only
/**
 * Wiki 图谱视图 V2：Canvas 2D 自研渲染器 + 自研物理引擎。
 *
 * 向 Obsidian 图谱看齐的设计目标：
 * - 持续的力导向物理模拟，节点永远在做微小的"呼吸"运动
 * - 节点：径向渐变 glow + 脉动光晕 + 社区染色
 * - 边：基础线 + 粒子流动动画（沿边移动的光点）
 * - 交互：拖拽回弹、hover 邻居高亮、滚轮缩放、平移
 * - 性能：Canvas 2D 轻松处理万级节点
 */

import { Tooltip } from "@/components/layout/Tooltip";
import { Button, Card, Empty, Popover, theme, Typography } from "antd";
import {
  Download,
  Eye,
  Fullscreen,
  Maximize2,
  RefreshCw,
  SlidersHorizontal,
  Sparkles,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import {
  type CSSProperties,
  forwardRef,
  memo,
  type MouseEvent as ReactMouseEvent,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  assignMissingCommunities,
  type AssignMissingOutcome,
  countDistinctCommunities,
  mergeCommunitiesTopologically,
  mergeEntityCommunities,
} from "./communityMerge";
import {
  AGG_LAYOUT_HALF_SPAN,
  AGG_PHYS_MAX_STEPS_PER_FRAME,
  AGG_PHYS_STEP_MS,
  AGG_PHYSICS_CONFIG,
  buildAggregateGraph,
  countMembers,
  createAggregateAnnealState,
  createAggregateSettleState,
  normalizeAggregateScale,
  reheatAggregateAnneal,
  updateAggregateAnneal,
  updateAggregateSettle,
} from "./graphAggregate";
import {
  buildNeighborMap,
  buildNodeMap,
  buildPhysicsEdges,
  computeCommunityCentroids,
  initializePositions,
  isSystemStable,
  type NeighborMap,
  type NodeMap,
  type PhysicsConfig,
  type PhysicsEdge,
  type PhysicsNode,
  stepPhysics,
} from "./graphPhysics";
import type { WorkerMessage, WorkerResponse } from "./graphPhysics.worker";
import {
  applySavedLayout,
  buildNodeColorCache,
  clamp,
  clearLayout,
  COMMUNITY_BUBBLE_RADIUS_SCALE,
  communityPalette,
  communityRadius,
  EDGE_DRAW_BUDGET,
  edgeTypeLabels,
  escapeXml,
  getEdgeTypeStylesMap,
  getNodeSize,
  hashStringToInt,
  loadLayout,
  NODE_DRAW_BUDGET,
  nodeDrawRadius,
  parseColor,
  saveLayout,
  SPRITE_BAKE_ZOOM,
  spriteUsableAtZoom,
  viewportDrawRate,
} from "./graphViewUtils";
import { type LabelCandidate, selectLabelsToDraw } from "./labelLayout";

// ── P7: 生产诊断日志收敛到单一 DEBUG_GRAPH 开关 ──
// 优先级：URL 显式指定 >（dev 下）localStorage。
//   · `?debugGraph=1` → 强制开启，**生产构建同样生效**。本项目用 `BrowserRouter`（App.tsx:570）
//     ⇒ 直接放在 search 里即可：`/llm-wiki/<vault>/graph?debugGraph=1`。
//     下面同时也读 hash 内的查询串（`#/xxx?debugGraph=1`），仅作路由方式变更时的兜底 —— 当前**不需要**写 hash。
//   · `?debugGraph=0` → 强制关闭，便于 A/B 对照实验
//   · 未指定时：dev 读 `localStorage["DEBUG_GRAPH"]`，生产恒 false（沿用旧语义，避免生产 DevTools 卡顿）
// ⚠ 2026-09-17 增补 URL 分支的原因：旧实现首行就是 `if (!import.meta.env.DEV) return false;`
//   ⇒ 生产构建里**所有** debugLog 都是死代码，而布局/物理类问题恰恰只在生产构建的性能特征下才复现。
//   想把「用日志判生产行为」这条路走通，就必须有一条不依赖 `import.meta.env.DEV` 的开关。
const DEBUG_GRAPH = (() => {
  try {
    const fromQuery = new URLSearchParams(window.location.search).get("debugGraph");
    const hash = window.location.hash;
    const qi = hash.indexOf("?");
    const fromHash = qi >= 0 ? new URLSearchParams(hash.slice(qi + 1)).get("debugGraph") : null;
    const raw = fromQuery ?? fromHash;
    if (raw === "1" || raw === "true") { return true; }
    if (raw === "0" || raw === "false") { return false; }
  } catch {
    // window/location 不可用（非浏览器环境）时按「未指定」处理
  }
  if (!import.meta.env.DEV) { return false; }
  try {
    return localStorage.getItem("DEBUG_GRAPH") === "true";
  } catch {
    return false;
  }
})();

function debugLog(...args: unknown[]): void {
  if (DEBUG_GRAPH) {
    console.log(...args);
  }
}

// ── 预热物理配置：随 init 消息传给 Worker，在 Worker 内完成初始布局收敛 ──
// 之前在主线程同步执行 warmupPhysics，几万节点时冻结 UI 数秒；现移入 Worker。
const WARMUP_PHYSICS_CONFIG: PhysicsConfig = {
  theta: 0.6,
  repulsion: 30000,
  gravity: 0.002,
  damping: 0.85,
  dt: 0.4,
  springForce: 0.06,
  springDamping: 0.9,
  maxVelocity: 10,
};

// ── 聚合物理配置：以「社区」为单位的力导向（2026-09-16 参数标定）──
// ⚠ 2026-09-17 迁移：AGG_PHYSICS_CONFIG / AGG_LAYOUT_HALF_SPAN / AGG_SETTLE_PX /
// AGG_SETTLE_STEPS 及**全部标定推导注释**已迁至 graphAggregate.ts 的「聚合物理配置」节。
// 迁移动机：标定与回归测试必须 import **同一份**常量 —— 常量定义在组件文件里时，测试只能
// 手拉整个组件（React/DOM/i18n 依赖）或手抄数值，而手抄的那一刻「被测对象」与「生产配置」
// 就分叉了（同族：判据 #7/#313）。

// ─────────────────────────────────────────────────────────────────────────────
// 公共类型（保持向后兼容）
// ─────────────────────────────────────────────────────────────────────────────

/**
 * 节点类型 —— ⚠ 这里**不是**后端 `PageType` 词汇表的镜像，而是「**前端有配色的类型集**」。
 *
 * 后端 `PageType` 有 14 个变体（D3 后），本联合类型只有 4 个：其余值（`doc` / `daily` /
 * `knowledge` / `knowledge_document` / `synced` / `log` / `comparison` / `index` / `overview` …）
 * 在 `buildNodeColorCache`（`graphViewUtils.ts:191`）落 `typeMap[node.type] || typeMap.note`
 * ⇒ **静默显示成 `note` 的颜色**，与真正的笔记无法区分。
 *
 * 是否为新类型配色 / 加 `wiki.graph.nodeType.*` 标签属**产品决策**（要选色、要翻译）
 * ⇒ 本轮只登记未实施；`GraphData.unresolvedTypes` 的警示条负责让「后端不认识的类型」
 * 至少可见（两者是**不同集合**：那是「后端词表外」，这里是「前端无配色」）。
 */
export type GraphNodeType = "note" | "concept" | "entity" | "source";
export type GraphEdgeType = "link" | "backlink" | "reference" | "derived_from" | "contradicts" | "mapping";

export interface GraphNode {
  id: string;
  title: string;
  type: GraphNodeType;
  tags: string[];
  linkCount: number;
  backlinkCount: number;
  path: string;
  x?: number;
  y?: number;
}

export interface GraphEdge {
  source: string;
  target: string;
  /** 渲染类别 —— 决定「画成什么样」，**不**代表这条边是什么关系 */
  type: GraphEdgeType;
  /**
   * **本体关系 id**（`has_concept` / `in_industry` / `董事` …）。
   *
   * `?` 是刻意的：只有**知识库实体关系边**才带它（笔记链接 / 合成边为 `undefined`），
   * 且后端在为空时不写该字段（`skip_serializing_if`）⇒ 消费方按「可能不存在」处理。
   *
   * ⚠ 它是**开放词表**：实测 DB 侧 56 个值 / 112937 行，其中 53 个是中文职位名
   * （由导入的数据文件决定，见后端 `DATA_DRIVEN_COLUMN`）。所以这里刻意**不**声明
   * 联合类型 —— 声明了就等于承诺「已经穷举」，而下一个 CSV 导入就会打破它。
   * 展示侧的出口是 `relationLegend`（图例里的关系类型分布），刻意不做白名单校验。
   */
  relationType?: string;
}

/**
 * 一个「标签无法被后端解释」的边在图中造成的规模（P2-b，2026-09-14）。
 *
 * 与 `UnresolvedTypeStat` 对称（那条管节点），是后端
 * `GraphData.unresolved_relations` 的前端形态。
 *
 * ⚠ 它**不是**「前端没有专属配色的关系类型」计数 —— 后者由图例的
 * `relationLegend` 回答。两者混用会让这条提示对存量数据刷屏
 * （实测 53 个中文关系类型 / 74325 行）。
 */
export interface UnresolvedRelationStat {
  /** 边上那个后端解释不了的原始标签 */
  rawType: string;
  /** 该字面量在图中出现的**边**数 */
  count: number;
  /** 样例边标识（形如 `source -> target`），最多 3 个 */
  sampleEdgeIds: string[];
}

/**
 * 端点**不在节点集里**的边统计（P0，2026-09-17）—— 后端 `GraphData.dangling_edges` 的前端形态。
 *
 * # 它与上面两个「未识别」统计不是一回事
 *
 * 那两条管**词汇表**（后端认不认识这个类型字面量），这一条管**图的自洽性**
 * （边的两端在不在这一份 nodes 里）。一条边可以类型完全合规却两端都缺失 ——
 * 而且那种边**一条都不会被画出来**（见 `drawEdgesOptimized` 的 `idSet` 判据）。
 *
 * **2026-09-18 起后端不再把它们留在 `edges` 里**（`GraphData::retain_resolved_edges`）：
 * 此前它们既画不出来、又被算进工具栏的 `N edges`，于是「边数」这个数本身就是错的。
 * 现在 `edges` 只含画得出来的边，而**本字段保存淘汰规模**（不是「还剩多少」）——
 * 界面因此照旧能告警，但读到的边数不再撒谎。
 *
 * 三项缺失分类**互斥**（`missingSourceOnly + missingTargetOnly + missingBoth === dangling`）。
 *
 * ⚠ 与 `unresolvedRelations` 的 serde 约定**相反**：后端对本字段总是序列化
 * （`dangling === 0` 也写），所以前端这里**不用 `?`**：
 * 「查过了，很干净」与「这份数据没统计过」必须能区分（前者 `totalEdges > 0 && dangling === 0`，
 * 后者 `totalEdges === 0`）。旧版缓存 JSON 缺该字段时后端补 `default` ⇒ 得到 `totalEdges === 0`。
 */
export interface DanglingEdgeSummary {
  /**
   * 统计基准：**淘汰前**参与统计的边数。
   *
   * ⚠ **不再等于 `data.edges.length`**（2026-09-18 起后端会把端点缺失的边从 `edges`
   * 摘掉）：两者相差正好 `dangling`，恒等式 `edges.length + dangling === totalEdges`
   * 始终成立。别把它当 `edges.length` 的别名用。
   */
  totalEdges: number;
  /** 至少一端缺失的边数 */
  dangling: number;
  /** **只有** source 缺失 */
  missingSourceOnly: number;
  /** **只有** target 缺失 */
  missingTargetOnly: number;
  /** 两端都缺失 */
  missingBoth: number;
  /** 样例边标识（形如 `source -> target`），最多 3 个 */
  sampleEdgeIds: string[];
}

/**
 * 一个「后端不认识的类型字面量」在图中造成的降级规模（A2-升级，2026-09-14）。
 *
 * 后端 `PageType` 词汇表外的 `node_type` 会让该节点的关系亲和度落兜底值。
 * 此前这个信号只存在于后端日志里，UI 只能看到「节点看着都对但连边少」；
 * 该字段把它送到界面上。
 */
export interface UnresolvedTypeStat {
  /** 节点上的原始 type 字面量 */
  rawType: string;
  /** 该字面量在图中出现的节点数 */
  count: number;
  /** 样例节点 id（最多 3 个），用于定位来源 */
  sampleNodeIds: string[];
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
  /**
   * 未识别类型统计。
   *
   * `?` 是刻意的：后端在清单为空时**不写这个字段**（`skip_serializing_if`），
   * 且旧版缓存 JSON 也没有它 ⇒ 消费方必须按「可能不存在」处理。
   */
  unresolvedTypes?: UnresolvedTypeStat[];
  /**
   * **边**侧的同一件事：标签无法被后端解释的边统计。
   *
   * 同 `unresolvedTypes`：`?` 是刻意的（空清单后端不写该字段，旧缓存也没有）。
   */
  unresolvedRelations?: UnresolvedRelationStat[];
  /**
   * 端点缺失（画不出来）的边统计（P0，2026-09-17）。
   *
   * 与上面两条的 serde 约定**相反**：后端**总是**序列化该字段（哪怕全 0）。
   * 但这里仍标 `?` —— 前端内部也会构造 `GraphData`（mock / 单测 / 局部视图），
   * 那些路径不经过后端。所以消费方要按「可能不存在」处理，判据看 `totalEdges`：
   * * `undefined` 或 `totalEdges === 0` ⇒ 这份数据没统计过，**不代表干净**；
   * * `totalEdges > 0 && dangling === 0` ⇒ 查过了，干净。
   */
  danglingEdges?: DanglingEdgeSummary;
}

export type LayoutMode = "force" | "radial" | "hierarchy";

export interface GraphViewProps {
  data: GraphData;
  wikiId?: string;
  onNodeClick?: (nodeId: string) => void;
  onNodeDoubleClick?: (nodeId: string) => void;
  onNodeHover?: (nodeId: string | null) => void;
  onContextMenu?: (nodeId: string, position: { x: number; y: number }) => void;
  onDeleteNode?: (nodeId: string) => void;
  onDeselect?: () => void;
  highlightedNodeIds?: Set<string>;
  selectedNodeId?: string | null;
  communities?: Map<string, number>;
  /**
   * **实体侧**（知识图谱）子图单独跑 Louvain 的社区映射：`entity:<id>` → 社区 id。
   *
   * 后端在实体子图上单独跑一次（`LouvainResult.entityCommunities`，见
   * `communityMerge.mergeEntityCommunities` 的说明），与 `communities`（笔记侧）
   * 是**两次独立运行** ⇒ 两侧 cid 值域会重合，**不能**直接合并。
   * 组件内一律经 `mergeEntityCommunities` 错开命名空间后再使用。
   *
   * 缺失（undefined）时行为与引入本属性之前逐值相同：实体节点靠 `mapping` 锚点
   * 继承同名笔记的桶（`assignMissingCommunities`）。
   */
  entityCommunities?: Map<string, number>;
  showMinimap?: boolean;
}

export interface GraphViewHandle {
  focusOnNode: (nodeId: string) => void;
}

// ─────────────────────────────────────────────────────────────────────────────
// 常量与配色
// ─────────────────────────────────────────────────────────────────────────────

const EMPTY_SET: ReadonlySet<string> = new Set<string>();

/**
 * 图例里「知识库关系类型」分节最多列几个。
 *
 * 为什么要有上限：实测 `lemonhu_knowledge_graph` 一个库就有 **55** 种关系类型
 * ⇒ 全列会把图例撑爆。超出的部分只报个数（`relationTypesMore`）。
 */
const RELATION_LEGEND_TOP_N = 8;

/**
 * requestIdleCallback 安全封装：macOS WKWebView（Safari 引擎）不支持该 API，
 * 裸调用会抛 ReferenceError 导致 LOD 更新 / 聚合几何刷新 / 位图缓存重建全部中断。
 * 不支持时降级为 setTimeout(0)（立即在下一事件循环执行）。
 */
function scheduleIdle(callback: () => void, timeout: number): void {
  if (typeof requestIdleCallback === "function") {
    requestIdleCallback(callback, { timeout });
  } else {
    setTimeout(callback, 0);
  }
}

// 布局持久化 / 配色 / 节点尺寸 / 颜色工具已抽至 ./graphViewUtils（F8 拆分）。

/**
 * 「无社区节点退回 hash 分桶」的告警节流（2026-09-18）。
 *
 * 为什么必须有：`assignMissingCommunities` 的锚点路径（实体继承其 `mapping` 笔记的桶）
 * 在真实数据上覆盖 **100%**（22,608/22,608，见 `__tests__/kgFusedScale.test.ts`），
 * 也就是说 hash 兜底**本不该被走到**。一旦 `viaHash > 0`，就说明锚点这条链断了
 * （`mapping` 边消失 / 社区缓存与节点集不再同源 / 桶集合为空）——
 * 而那个形态**在图上无法自证**：它表现为「布局散、看不出结构」，
 * 与「这份数据本来就没有结构」不可区分。所以必须留下痕迹。
 *
 * 节流按**值变化**而不是时间：`(补全数, 锚点数, hash 数, 桶数)` 的组合变了才报。
 * 时间节流会在数据真的变化那一刻闭嘴（与后端 `should_warn_dangling` 同款理由）。
 */
let lastCommunityFallbackSignature: string | null = null;

function warnOnHashFallback(wikiId: string | undefined, outcome: AssignMissingOutcome): void {
  if (outcome.viaHash === 0) {
    return;
  }
  const signature = `${outcome.assigned}/${outcome.viaAnchor}/${outcome.viaHash}/${outcome.bucketCount}`;
  if (signature === lastCommunityFallbackSignature) {
    return;
  }
  lastCommunityFallbackSignature = signature;
  console.warn(
    `[GraphAggregate] ${outcome.viaHash} / ${outcome.assigned} 个节点既没有社区归属、`
      + `也没有可继承的桶 ⇒ 退回 hash 随机分桶（wiki ${wikiId ?? "(未提供)"}，桶数 ${outcome.bucketCount}）。`
      + "hash 分桶会让桶级图逼近完全图、力导向退化为均匀铺开；"
      + "请检查融合层的 mapping 边是否还在、社区缓存是否与节点集同源。",
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// 核心组件
// ─────────────────────────────────────────────────────────────────────────────

interface Particle {
  edgeIndex: number;
  progress: number; // 0..1
  speed: number;
  size: number;
  color: string;
}

const GraphViewInner = forwardRef<GraphViewHandle, GraphViewProps>(({
  data,
  wikiId,
  onNodeClick,
  onNodeDoubleClick,
  onNodeHover,
  onContextMenu,
  onDeleteNode,
  onDeselect,
  highlightedNodeIds,
  selectedNodeId,
  communities,
  entityCommunities,
  showMinimap = true,
}, ref) => {
  const { token } = theme.useToken();
  const { t } = useTranslation();

  // token 的实时引用：渲染循环/数据 effect 通过 ref 读取最新 token，
  // 主题切换无需重建物理世界，只需重算颜色缓存（见 token 主题 effect）。
  const tokenRef = useRef(token);
  tokenRef.current = token;

  // 原始图数据/社区引用：供主题 effect 重算颜色缓存（数据 effect 不再依赖 token）
  const dataRef = useRef<GraphData | null>(null);
  const rawCommunitiesRef = useRef<Map<string, number> | null>(null);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number>(0);

  // Worker 相关
  const workerRef = useRef<Worker | null>(null);
  const workerInitializedRef = useRef(false);
  const workerErrorCountRef = useRef(0); // Worker 连续错误计数，超阈值降级到主线程
  const workerResultRef = useRef<
    {
      positions: Float64Array;
      velocities: Float64Array;
      stable: boolean;
      tick: number;
    } | null
  >(null);
  const pendingStepRef = useRef(false);
  // 追踪已处理的 Worker tick：只有 Worker 返回新结果时才更新节点/重建网格，
  // 避免每帧都用旧结果重算 O(N) 网格索引（大图下每秒 60 次 × 20k 节点 = 灾难性）
  const lastProcessedTickRef = useRef(-1);
  // L2/L3 修复：收敛期重型重建（gridIndex / 聚类几何 / 位图缓存）限流计数器，
  // 按收到的 Worker result 步数计数
  const workerStepCounterRef = useRef(0);
  // 隐式聚合下成员坐标由回写派生（applyAggregateLayout）⇒ 依赖坐标的派生缓存必须随之
  // 重建，各自独立限流：gridIndex 每 24 帧、大图位图每 600 帧（O(N+E)，不能每帧做）
  const lastAggGridRebuildRef = useRef(-1_000_000);
  const lastAggSpriteRebuildRef = useRef(-1_000_000);
  // 聚合布局接管坐标后相机需要跟随（见 applyAggregateLayout 注释）。
  // 上一次自动对齐时的聚合节点包围盒 —— 布局尺度和中心显著变化时才重新对齐，
  // 避免每帧 fit（会与用户的滚轮缩放打架，且相机永远在动）。
  const lastAggFitBBoxRef = useRef<{ spanX: number; spanY: number; cx: number; cy: number } | null>(null);
  const lastAggFitFrameRef = useRef(-1_000_000);
  // ── 聚合物理的「已收敛」闸（2026-09-16 新增）──
  // 为什么要显式判收敛：本引擎的既有稳定判据 `isSystemStable(nodes, 0.15)` 在质量跨 3 个
  // 数量级（聚合质量 1~553）+ maxVelocity 夹取的组合下**永远不成立**（实测静置 8000 步后
  // 仍有 47%~94% 的节点速度 > 1）⇒ 渲染循环拿不到 idle 信号 ⇒ 24288 个节点 60fps 一直重绘。
  // 而本轮的标定配置又必须关掉 `keepSimulating` 的旧兜底（否则会在未平衡时误判静止并永久
  // 冻结）。两者叠加的结果是「物理永远跑、画面永远重绘」—— 比原缺陷更贵。
  // 所以收敛判据改成**可观测且与显示尺度直接相关**的量：布局跨度连续 N 步几乎不变。
  // 触发后停止步进（速度**不清零**，用户拖拽/重建时可直接续跑），idle 随之恢复。
  const aggSettleRef = useRef(createAggregateSettleState());
  // 布局退火状态（2026-09-17）：把速度上限按温度因子衰减，让物理**真的停下来**。
  // 与 aggSettleRef 是一对：退火负责「让布局停」，settle 负责「确认它停了并停止步进」。
  // 缺退火时 settle 永不触发（实测均匀质量 5000 步都不触发，见 graphAggregate.ts 的
  // 「布局退火」段）⇒ 物理永不停止 ⇒ 渲染永不跳帧。详见该段注释。
  const aggAnnealRef = useRef(createAggregateAnnealState());
  // 聚合物理的**步进时钟**（2026-09-17）：把「物理步数」与「渲染帧数」解耦。
  //
  // 【为什么必须解耦 —— 这是一个正反馈陷阱，实测已踩】
  // 改前 `shouldRun` 里有一项 `frameCounterRef.current % 6 === 0`（「稳定降频」），
  // 物理步频因此 = fps / 6。实测（fixture 24288 节点、浏览器真机探针）fps 只有 **10.8**
  // ⇒ 物理仅 ~1.8 步/秒。而退火的设计前提是 **60 步/秒**（`AGG_ANNEAL_START_STEPS = 900`
  // 的注释写的是「≈15 秒 @60 步/秒」）⇒ 需要 1100 步的收敛过程要 **~611 秒**，
  // 而实测只预热 45 秒（= 531 步 < 900 全温期）⇒ **退火从未开始** ⇒ `settled` 零命中。
  // 于是形成闭环：
  //     绘制慢（12000 个节点圆/帧）⇒ fps 低 ⇒ `% 6` 降频后物理更慢 ⇒ 布局停不下来
  //     ⇒ 位图因 `layoutStable` 为假而永不构建（构建闸要求布局已停）
  //     ⇒ 节点层只能是矢量路径 ⇒ 绘制继续慢 …
  // **唯一能打断它的位图，恰好被它自己挡住了。**
  //
  // 【修法】不再问「这是第几帧」，改问「距离上一步走了多少真实时间」——
  // 累积真实毫秒，按固定步长（1/60 s）换算成本帧应补的步数。
  // 这样物理步频恒为 60 步/秒（与帧率无关），退火耗时可预期；fps 低时只是「每帧多补几步」。
  // 上限 `AGG_PHYS_MAX_STEPS_PER_FRAME` 是必须的：切回前台 / 长任务后 `elapsed` 可能是秒级，
  // 不设上限会一次性补爆（单帧几千步）反而卡死。
  //
  // ⚠ 与 `idleCounterRef`（渲染跳帧）**职责不同**，不要合并：
  //   那个管「还要不要重绘」，这个管「物理该走多少步」。渲染可以跳帧，物理不该跳步 ——
  //   物理一旦被跳步，就等于「用帧率当物理时间」，正是本次要修掉的东西。
  // ⚠ `lastMs` 的初值是 `-1`（哨兵，不是 0）：第一帧不能拿「页面加载至今」当 `elapsed`
  //   —— 那会在聚合物理刚激活时凭空补一批步数。哨兵让首帧零债，从第二帧开始计时。
  const aggClockRef = useRef({ lastMs: -1, accMs: 0 });
  // 用户是否**亲手**改过相机（滚轮 / 拖拽平移 / 缩放按钮 / 快捷键）。
  // 这是「视角归属」的唯一判据 —— 用户接管后不再自动对齐。
  const cameraTouchedByUserRef = useRef(false);

  // 物理节点和边（在 ref 中持久化，不触发 React 重渲染）
  const physNodesRef = useRef<PhysicsNode[]>([]);
  const physEdgesRef = useRef<PhysicsEdge[]>([]);
  const particlesRef = useRef<Particle[]>([]);
  const nodeMetaRef = useRef<Map<string, GraphNode>>(new Map());
  const nodeColorRef = useRef<Map<string, string>>(new Map());
  const nodeSizeRef = useRef<Map<string, number>>(new Map());
  /**
   * **逐节点图集精灵**缓存（每色一张，尺寸与 `NODE_NODE_SPRITE_SIZE` 同源）：绘制时按 **9 参**
   * `drawImage(img, sx,sy,sw,sh, dx,dy,dw,dh)` 从图集里裁一格贴到节点位置
   * （见 `drawNodesOptimized`）。
   *
   * ⚠ 勿与下方 `spriteCacheRef` 混为一谈（2026-09-17 登记「sprite 一名两义」）：
   *   本项 = 「**每节点一格**」（图集切片，9 参，每采样窗口实测 2.7~9.3 万次，在跑）；
   *   那项 = 「**整张图一张位图**」（5 参，构建受多道门控，见报告 §6.15-#2）。
   *   两者唯一共同点是名字里都有 sprite。
   */
  const nodeSpriteCacheRef = useRef<Map<string, HTMLCanvasElement>>(new Map());
  const edgeMetaRef = useRef<
    {
      source: string;
      target: string;
      type: GraphEdgeType;
      animated: boolean;
      color: string;
      width: number;
      sourceIdx: number;
      targetIdx: number;
    }[]
  >([]);

  // 预构建的邻居表和节点索引（缓存复用，避免每帧重建）
  const neighborMapCacheRef = useRef<NeighborMap>(new Map());
  const nodeMapCacheRef = useRef<NodeMap>(new Map());

  // 预渲染的背景画布（避免每帧重建渐变）
  const bgCacheRef = useRef<HTMLCanvasElement | null>(null);
  const bgCacheSizeRef = useRef({ w: 0, h: 0 });

  /**
   * **大图位图**缓存：把所有节点/边预渲染到**一张**离屏 Canvas，每帧只做 1 次 **5 参**
   * `drawImage(img, dx,dy,dw,dh)` ⇒ 彻底消除万级节点下每帧 5 万+ 矢量 Canvas 操作导致的
   * 主线程阻塞。`spriteWorldBBoxRef` 是它的**配对件**：5 参调用必须以世界 bbox 作目标矩形，
   * 重复乘一次 cam.zoom 会把位图搬到错位置并缩到 camZ² 倍（见各处渲染路径的修复注释）。
   *
   * ⚠ 与上方 `nodeSpriteCacheRef`（**逐节点**图集、**9 参**）不是一回事 —— 2026-09-17 登记的
   *   「sprite 一名两义」：本项是「整张图一张位图」，那项是「每节点一格」。
   */
  const spriteCacheRef = useRef<HTMLCanvasElement | null>(null);
  const spriteWorldBBoxRef = useRef({ minX: -5000, minY: -5000, maxX: 5000, maxY: 5000 });
  /**
   * 位图的**新鲜度锚点**（2026-09-17 启用位图时新增）。
   *
   * 为什么必须有：位图是**快照**，而聚合物理在退火完成前每步会让节点移动最多
   * `maxVelocity·dt = 7.2` 世界单位 —— 一个 600 帧的重建周期内布局能跑出几千世界单位，
   * 把旧位图贴上去就是「整张图错位」。所以「位图存不存在」**不足以**说明它能用，
   * 还必须证明「它与当前布局没走样」。
   *
   * 判据 = 烘制时抽样记录的若干节点坐标，与当前坐标的**最大偏差 × zoom**（屏幕像素）。
   * 超过 `SPRITE_STALE_PX` 即视为过期 ⇒ 消费侧让位给矢量路径、构建侧触发重建。
   * 这样两侧判据**同源**，这也是「构建了却永远画不出来」不可能再复发的结构保证。
   * 抽样（而非全量）是为了让这个判据每帧可负担：全量 2.4 万次遍历 × 每帧，
   * 而抽样 256 个点的漂移上界与全量在统计上同量级（运动是连续场，不是逐点独立噪声）。
   */
  const spriteAnchorRef = useRef<{ ids: string[]; xs: number[]; ys: number[] } | null>(null);
  /** 位图允许的最大「走样」屏幕像素。取 2px ≈ 一个节点半径：低于它错位肉眼不可辨。 */
  const SPRITE_STALE_PX = 2;
  const FORCE_BITMAP_THRESHOLD = 3000; // 超过此节点数时强制使用位图模式

  // 相机变换
  const cameraRef = useRef({ x: 0, y: 0, zoom: 1 });
  // 数据加载后自动适应视图一次。初始布局半径 / localStorage 里已保存的坐标都可能
  // 远大于视口（见 graphPhysics.initializePositions 的注释），不自动 fit 时打开图谱
  // 只能看到一片空白加几个孤点。handleFitAll 定义在本组件后段，经 ref 传递。
  const autoFitPendingRef = useRef(true);
  const fitAllRef = useRef<(() => void) | null>(null);

  // 交互状态
  const dragRef = useRef<{ nodeId: string } | null>(null);
  const panRef = useRef<{ startX: number; startY: number; camX: number; camY: number } | null>(null);
  const hoverNodeRef = useRef<string | null>(null);
  const selectedNodeIdRef = useRef<string | null>(null);
  const highlightSetRef = useRef<Set<string> | undefined>(undefined);

  // 脉动相位
  const phaseRef = useRef(0);
  const frameCounterRef = useRef(0);
  const stableFrameCounterRef = useRef(0);
  /**
   * 空闲计数的**三档阈值**（2026-09-17 收口；此前是散落在各消费点的魔数 12 / 30 / 60）。
   * 必须保持 `PHYSICS_DECIMATE ≤ RENDER_DECIMATE < SKIP_FRAMES`：
   * 阶梯是「物理节流 → 渲染降频 → 整帧跳过」，倒过来会让跳帧先于降频生效。
   */
  const IDLE_PHYSICS_DECIMATE_FRAMES = 12;
  const IDLE_RENDER_DECIMATE_FRAMES = 30;
  const IDLE_SKIP_FRAMES = 60;
  /**
   * 连续「物理已收敛 **且** 无交互」的帧数。
   *
   * ⚠ **不要把它读成「这一帧没画」**（2026-09-17，首屏全空的成因）：
   *   它由**两个物理分支**累加（worker 结果路径 `:1938` / 主线程物理路径 `:1950`），
   *   而这两个分支都在渲染闸 `if (shouldRender && !workerNotReadyLargeGraph)` **之前**执行
   *   ⇒ 它与「本帧到底画没画」是两件独立的事。2.4 万节点下 Worker 预热 ≈4.1s，这期间一帧
   *   都没画而它已越过 `IDLE_SKIP_FRAMES` ⇒ 闸一开就永久命中跳帧（该分支在背景绘制之前
   *   return）⇒ 首屏永久全空。这就是 `hasPaintedFrameRef` 存在的理由。
   *
   * ⚠ **它同时被四类语义消费**（保名不改以免 20+ 处 churn，四类语义在此一次性登记）：
   *   ① 物理节流 —— `% IDLE_PHYSICS_DECIMATE_FRAMES`（主线程物理路径）
   *   ② 渲染降频 —— `> IDLE_RENDER_DECIMATE_FRAMES` ⇒ 每 2 帧画一次（30fps）
   *   ③ 整帧跳过 —— `> IDLE_SKIP_FRAMES` ⇒ 直接 return（**必须**配合 `hasPaintedFrameRef`）
   *   ④ 收敛判据 —— 粒子模块 `const isStable = … > 0`、`:3924` 的 `> 30` 等，把它当
   *      「物理是否已稳定」用；而 ①②③ 读的是「闲置程度」。二者相近但**不等价**：
   *      ①②③ 关心「多久没人动」，④ 关心「物理动了没有」。
   *   ⇒ 新增消费点前先确认要的是哪一个，别再默认复用。
   *
   * ⚠ **跳帧早退会饿死渲染路径后段的惰性工作**（2026-09-17 发现，报告 §6.15-#2）：
   *   大图位图构建（`requestIdleCallback{timeout:1000}`）位于 ③ 的 return **之后** ⇒ 在
   *   「已收敛且无交互」这个**最常态**下它永远到不了。任何新增的惰性维护任务都必须检查
   *   自己是否落在跳帧早退的下游。
   */
  const idleCounterRef = useRef(0);
  /**
   * 「是否已经画满过至少一帧」—— **空闲跳帧的前置条件**（2026-09-17 新增）。
   *
   * 为什么必须有它：`idleCounterRef` 由**物理分支**累加，而物理分支位于渲染闸
   * `if (shouldRender && !workerNotReadyLargeGraph)` **之前**，因此「idle 计数」与
   * 「这一帧到底画没画」是**两件独立的事**。2.4 万节点下 Worker 预热约 4.1s，这期间
   * 渲染闸全程关闭（一帧都没画过）而 idle 已越过 60 ⇒ 闸一打开就永久命中跳帧分支
   * （该分支在背景绘制之前 return）⇒ **首屏永久全空**，直到用户碰一下鼠标才恢复。
   * 它同时是 resize 的必需项：`canvas.width = …` 会清空画布，画面必须在下一帧重建。
   */
  const hasPaintedFrameRef = useRef(false);

  // 鱼眼 / 聚类 状态
  const fisheyeEnabledRef = useRef(false);
  const clusterModeRef = useRef(false);
  // 自动 force cluster 标记：区分自动触发和用户手动开启的聚类模式
  // 自动触发时默认展开社区让用户看到真实节点；用户手动开启时保持全折叠
  const isAutoForceClusterRef = useRef(false);
  // 粒子流动默认开启（对齐 Obsidian 的动态美感；大规模节点自动降级）
  const particlesEnabledRef = useRef(true);
  // ── 社区聚合折叠 ──
  // 折叠的社区集合（聚类模式下默认全折叠；点击聚合节点展开/收起）
  const collapsedRef = useRef<Set<number>>(new Set());
  const hoverClusterRef = useRef<number | null>(null);
  // LOD 缩放阈值：渐进式展开，类似地图缩放细节
  const LOD_THRESHOLDS = {
    COLLAPSED: 0.5, // zoom < 0.5: 全折叠
    VIEWPORT: 1.0, // 0.5 <= zoom < 1: 视口内展开
    EXPANDED: 2.0, // 1 <= zoom < 2: 视口+邻近展开
    ALL: 4.0, // zoom >= 2: 全部展开
  };
  // 上次 LOD 级别，防抖用
  const lastLodLevelRef = useRef(0);
  // 手动展开的社区（用户点击展开的，不会因缩放折叠回去）
  const manualExpandedRef = useRef<Set<number>>(new Set());
  // 每帧最多新增展开的社区数（防止一次性展开过多导致卡顿）
  const MAX_EXPAND_PER_FRAME = 5;
  // 聚合节点几何缓存：cid → { 质心, 半径, 计数, 代表名 }（低频刷新）
  const clusterGeomRef = useRef<
    Map<number, { cx: number; cy: number; r: number; count: number; label: string }>
  >(new Map());
  // ── 聚合物理（聚类折叠模式下物理只模拟聚合节点 + 未折叠节点，而非全部底层节点）──
  // 折叠社区的成员节点不参与物理（数量级骤降），聚合节点坐标驱动 clusterGeom。
  const aggPhysRef = useRef<
    {
      nodes: PhysicsNode[];
      edges: PhysicsEdge[];
      cidToNodeIdx: Map<number, number>;
      neighborMap: NeighborMap;
      /** true = 隐式聚合：零折叠（大图自动路径）下把全部社区当布局单元。
       *  此时成员节点**不进物理**，其坐标由 applyAggregateLayout 从聚合节点派生回写。 */
      implicit: boolean;
    } | null
  >(null);
  // 展开/收起状态变化时触发重渲染
  const [, setClusterCollapseVersion] = useState(0);
  const mouseScreenRef = useRef({ x: 0, y: 0, active: false });
  const communityCentroidsRef = useRef<Map<number, { cx: number; cy: number; count: number }>>(new Map());
  // 聚类气泡（drawClusterRegions）稳定态缓存（P9）：节点位置/折叠集合变化或每 30 帧才重建分组与渐变，
  // 图完全静止时直接复用缓存，避免每 5 帧全量 O(N) 分组 + 为每个社区新建 radialGradient。
  const clusterRegionCacheRef = useRef<{
    lastFrame: number;
    dirty: boolean;
    lastCollapsed: Set<number> | null;
    regions: Map<number, { cx: number; cy: number; rx: number; ry: number; grad: CanvasGradient }>;
  }>({ lastFrame: -9999, dirty: true, lastCollapsed: null, regions: new Map() });
  // communities prop 的 ref 镜像，供 useCallback / 事件回调读取最新值而无需将其加入依赖
  const communitiesRef = useRef<Map<string, number> | undefined>(undefined);
  useEffect(() => {
    // 优先使用哈希合并后的虚拟聚类映射
    communitiesRef.current = effectiveCommunitiesRef.current ?? communities;
  }, [communities]);

  const gridIndexRef = useRef<Map<string, string[]>>(new Map());
  const GRID_CELL_SIZE = 80;
  // minimap 包围盒缓存：系统稳定时复用，避免每 15 帧全量遍历计算
  const minimapBBoxRef = useRef<{ minX: number; minY: number; maxX: number; maxY: number } | null>(null);

  // ── 性能 LOD 阈值（万级节点保障） ──
  const GLOW_NODE_LIMIT = 2000; // 超过此节点数：普通节点不绘制 glow，仅交互节点保留
  const MINIMAP_REDRAW_INTERVAL = 15; // minimap 重绘间隔（帧），大图避免每帧全量遍历
  // 节点屏幕像素下限 / 绘制半径口径已收敛到 graphViewUtils.nodeDrawRadius（见其文档注释）：
  // 绘制侧与命中侧必须取同一函数，否则会出现「看得见、点不中」。
  // 节点数超过此值且 communities 可用时，打开自动进入聚类折叠聚合视图，
  // 物理只模拟聚合节点（几十个），从根本上避免万级节点全量力导向收敛导致的卡死。
  const AUTO_CLUSTER_THRESHOLD = 3000;
  // 聚合物理规模上限：聚合节点 + 未折叠节点数超过此值时，放弃力导向（仅静态显示），
  // 防止社区粒度极细（甚至每节点一社区）时聚合物理规模仍达万级，主线程每帧 O(n log n) 卡死不响应。
  const MAX_AGG_PHYS_NODES = 800;
  // 目标聚类桶数（大图）：**不同社区 id 的个数**超过此值时，按拓扑归并到该桶数（communityMerge.ts）。
  // 取值依据（审计报告 §6.8.3，真实 24288 节点 / 74791 边 / 2458 社区，N=100~2000 全量扫描见该节）：
  //   N=200 在四项判据上同时占优 —— modularity Q **0.6607**（N=300 为 0.6592、N=400 为 0.6570）、
  //   聚合边 **1837 条**（N=300 为 2687、N=400 为 3463）⇒ 力导向负担最小、
  //   等效气泡直径 **81px**（N=300 为 67px）、每聚合节点平均度 18.4（现行哈希分桶为 165）。
  //   仅 NMI(vs 真实社区) 略低于更大桶数（0.7185，N=300 为 0.7520）—— 该判据偏好更细的桶。
  //   归并耗时 46ms（N=200 / N=300 同为 46ms，N=800 为 122ms），不构成约束。
  //   ⚠ 判据选择：曾用「内聚度（桶内边/桶内节点对数）」得出 N=300 更优，但该判据被**桶大小分布**
  //   主导（拓扑归并的桶呈幂律分布，一个 900 的巨桶会把分母抬到 C(900,2) 从而压低该值），
  //   不能跨策略/跨 N 比较；改用对桶数稳健的 modularity Q 与 NMI 后结论反转。
  // ⚠ 必须 ≤ MAX_AGG_PHYS_NODES：气泡层判据是 `clusterCount <= MAX_AGG_PHYS_NODES`，
  // 一旦目标桶数超过它，归并结果会被判为「不可画」⇒ 气泡层永久不可达。
  // 该不变量由使用处的 Math.min 强制保证，不靠两个常量「碰巧一大一小」。
  const TARGET_CLUSTER_COUNT = 200;
  // 主线程物理规模上限：超过此节点数时，fallback 主线程物理一律禁用（静态显示）。
  // fallback 是 Worker 未就绪时的兜底；若在大图上每帧跑全量 O(n log n) 力导向，
  // 主线程会被完全阻塞、鼠标键盘全部无响应。大图等待 Worker 就绪即可，绝不走主线程物理。
  const MAX_MAIN_THREAD_PHYSICS = 1500;

  // 有效社区映射（考虑哈希合并后的虚拟聚类）
  const effectiveCommunitiesRef = useRef<Map<string, number> | undefined>(undefined);

  // 统一的社区查找函数：所有代码路径必须使用这个，不能直接用 communities prop
  // 因为哈希合并后的虚拟聚类映射存在 effectiveCommunitiesRef 中
  const getCommunityId = useCallback((nodeId: string): number | undefined => {
    return effectiveCommunitiesRef.current?.get(nodeId);
  }, []);

  // 哈希字符串转整数（用于节点到虚拟聚类的稳定分桶）
  // 2026-09-16 迁至 graphViewUtils：buildAggregateGraph 需要同一个哈希派生聚合节点初速，
  // 两处各一份实现必然分叉 ⇒ 提升为单一真相源。

  // 屏幕恒定线宽 → 世界坐标线宽。
  //
  // 渲染循环对场景施加了 ctx.scale(cam.zoom, cam.zoom)（见 render 内变换），
  // 因此写进 ctx.lineWidth 的值是「世界坐标单位」，落屏时会**再乘一次 zoom**。
  // 要让屏幕上恒为 screenPx 宽，世界坐标必须写 screenPx / zoom。
  //
  // ⚠ 历史缺陷：该位置原写作 `baseWidth * (zoom * 1.5)`，
  // ⇒ 屏幕线宽 = 0.3 × 1.5·zoom × zoom = 0.45·zoom²。
  // zoom ≤ 0.5 时恒 < 0.11px（再叠加 0.12~0.3 的 globalAlpha 后几乎不可见），
  // 且「越缩小线越细」——与缩放补偿的意图正好相反，是本图「一片无关系的点」的直接成因之一。
  // 同文件 minimap 视口框（ctx.lineWidth = 1 / cameraRef.current.zoom）是正确写法的既有先例。
  function worldEdgeWidth(screenPx: number, zoom: number): number {
    if (!(zoom > 0)) { return screenPx; }
    return screenPx / zoom;
  }

  const minimapRef = useRef<HTMLCanvasElement>(null);
  const [minimapOpen, setMinimapOpen] = useState(true);
  const minimapDragRef = useRef(false);

  // wikiId ref，用于布局持久化
  const wikiIdRef = useRef<string | undefined>(wikiId);
  wikiIdRef.current = wikiId;

  // 渲染缓存：posMap 和预计算的邻居集合，避免每帧重建 O(N)/O(E)
  const posMapRef = useRef<Map<string, PhysicsNode>>(new Map());
  // N6 修复：统计弹窗 Zoom 值的 DOM 引用，渲染循环中直接写 textContent 实时刷新
  const statsZoomTextRef = useRef<HTMLSpanElement | null>(null);
  const neighborsRef = useRef<Map<string, Set<string>>>(new Map());

  const [fisheyeEnabled, setFisheyeEnabled] = useState(false);
  const [clusterMode, setClusterMode] = useState(false);
  const [particlesEnabled, setParticlesEnabled] = useState(false);

  // Tooltip: 节点内容用 useState (低频更新)，位置用 ref + DOM 操作 (高频更新)
  const [tooltipNodeIdState, setTooltipNodeIdState] = useState<string | null>(null);
  const tooltipNodeIdRef = useRef<string | null>(null);
  const tooltipPosRef = useRef({ x: 0, y: 0 });
  const tooltipVisibleRef = useRef(false);
  const tooltipRef = useRef<HTMLDivElement | null>(null);

  // 同步 tooltip 节点 ID 到 ref（供渲染循环使用，避免闭包过期）
  useEffect(() => {
    tooltipNodeIdRef.current = tooltipNodeIdState;
  }, [tooltipNodeIdState]);

  // 尺寸
  const [dimensions, setDimensions] = useState({ width: 800, height: 600 });
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [statsOpen, setStatsOpen] = useState(false);
  const [legendOpen, setLegendOpen] = useState(false);

  // 边类型可见性筛选
  const [visibleEdgeTypes, setVisibleEdgeTypes] = useState<Set<GraphEdgeType>>(
    new Set(["link", "backlink", "reference", "derived_from", "contradicts", "mapping"]),
  );
  const visibleEdgeTypesRef = useRef(visibleEdgeTypes);
  visibleEdgeTypesRef.current = visibleEdgeTypes;

  // P2-c（2026-09-14）：知识库关系类型分布。
  //
  // 背景：后端在 2026-09-14 之前把实体关系边的 `relation_type` **读出来又扔掉**
  // （`edge_type` 恒为常量 `"reference"`）⇒ DB 实测 56 个关系类型 / 112937 行
  // 在图上完全不可分辨（`lemonhu` 一个库就有 74766 条边 / 55 种类型）。
  // 后端现在把真实关系 id 放进 `edge.relationType`，**这里是它唯一的展示出口** ——
  // 没有这个出口，那个字段就又是一次「能力已存在但无人消费」。
  //
  // 刻意**不**做白名单：开放词表（下一个 CSV 导入就会引入新值），
  // 所以只统计、不校验、不丢弃。
  const relationLegend = useMemo(() => {
    const counts = new Map<string, number>();
    for (const e of data.edges) {
      const rt = e.relationType;
      if (!rt) { continue; }
      counts.set(rt, (counts.get(rt) ?? 0) + 1);
    }
    if (counts.size === 0) { return null; }
    const sorted = [...counts.entries()].sort(
      (a, b) => b[1] - a[1] || a[0].localeCompare(b[0]),
    );
    return {
      edges: sorted.reduce((sum, [, n]) => sum + n, 0),
      types: sorted.length,
      top: sorted.slice(0, RELATION_LEGEND_TOP_N),
      rest: Math.max(0, sorted.length - RELATION_LEGEND_TOP_N),
    };
  }, [data.edges]);

  const toggleEdgeType = useCallback((type: GraphEdgeType) => {
    setVisibleEdgeTypes((prev) => {
      const next = new Set(prev);
      if (next.has(type)) {
        next.delete(type);
      } else {
        next.add(type);
      }
      return next;
    });
  }, []);

  // 同步 selected/highlight 到 ref
  useEffect(() => {
    selectedNodeIdRef.current = selectedNodeId ?? null;
  }, [selectedNodeId]);
  useEffect(() => {
    highlightSetRef.current = highlightedNodeIds && highlightedNodeIds.size > 0 ? highlightedNodeIds : undefined;
  }, [highlightedNodeIds]);
  useEffect(() => {
    fisheyeEnabledRef.current = fisheyeEnabled;
  }, [fisheyeEnabled]);
  useEffect(() => {
    clusterModeRef.current = clusterMode;
  }, [clusterMode]);
  useEffect(() => {
    particlesEnabledRef.current = particlesEnabled;
  }, [particlesEnabled]);

  // ── 聚类模式切换：按钮与快捷键 'l' **共用同一个入口** ──
  // 归档缺陷（2026-09-16）：两条入口此前各写一套 —— 工具栏按钮清了
  // isAutoForceClusterRef、快捷键没清 ⇒ 同一个用户操作（关掉聚类）在两条入口下
  // 走不同的渲染分支，其中一条整屏空白（判据 #299）。收敛为单一入口，
  // 顺带消除「同一语义两处实现」这个必然腐烂的温床。
  const toggleClusterMode = useCallback(() => {
    const next = !clusterModeRef.current;
    // 用户手动切换优先于自动行为：大图自动 force cluster 的推断不再覆盖用户意图
    isAutoForceClusterRef.current = false;
    clusterModeRef.current = next; // 同步 ref：渲染循环下一帧即读到新值
    setClusterMode(next);
    // 切换聚类模式会重建布局语义（隐式聚合 ↔ 显式折叠）⇒ 恢复自动对齐。
    // 这不是「无视用户视角」：布局整个重来后沿用旧视角只会再次「画了但看不见」，
    // 所以新周期需要重新对齐一次（用户随后仍可自由缩放/平移，一旦操作即再次停手）。
    cameraTouchedByUserRef.current = false;
    lastAggFitBBoxRef.current = null;
    lastAggFitFrameRef.current = -1_000_000;
  }, []);

  // 聚类模式切换：开启时默认全折叠（聚合视图），关闭时清空
  useEffect(() => {
    if (clusterMode) {
      // 自动 force cluster 模式下跳过全折叠初始化，保持展开状态让用户看到真实节点
      if (isAutoForceClusterRef.current) {
        return;
      }
      // 使用 effectiveCommunitiesRef（可能是哈希合并后的虚拟聚类）
      const ec = effectiveCommunitiesRef.current;
      if (ec) {
        const all = new Set<number>();
        for (const cid of ec.values()) {
          all.add(cid);
        }
        // 排除当前选中节点所在社区
        if (selectedNodeIdRef.current) {
          const selCid = getCommunityId(selectedNodeIdRef.current);
          if (selCid !== undefined) {
            all.delete(selCid);
          }
        }
        collapsedRef.current = all;
        refreshClusterGeom();
        buildAggregatePhysics();
        setClusterCollapseVersion((v) => v + 1);
      }
    } else {
      // 用户关闭聚类模式：清除自动 force cluster 标志，并**退化为隐式聚合**。
      // 原先这里直接 `aggPhysRef.current = null` ⇒ aggActive 归 false ⇒ 24288 个原始
      // 节点重新接管力导向，用户会看到「刚聚好的团又炸开」，社区在坐标空间里再次交织。
      // 重建为 implicit（零折叠 ⇒ 全部社区作布局单元）后：关掉聚类 = 看真实节点，
      // 但节点仍按社区成团，两种视图共享同一套坐标，切换不再抖。
      isAutoForceClusterRef.current = false;
      collapsedRef.current = new Set();
      hoverClusterRef.current = null;
      buildAggregatePhysics();
      refreshClusterGeom();
      setClusterCollapseVersion((v) => v + 1);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clusterMode, communities]);

  // 社区可见性筛选
  const [visibleCommunities, setVisibleCommunities] = useState<Set<number>>(new Set());
  const visibleCommunitiesRef = useRef(visibleCommunities);
  visibleCommunitiesRef.current = visibleCommunities;

  // 社区筛选预计算：缓存全量社区集合和筛选状态，避免每帧在绘制函数内重建
  const visibleCommunitiesAllSetRef = useRef<Set<number>>(new Set());
  const hasCommunityFilterRef = useRef(false);

  const toggleCommunity = useCallback((cid: number) => {
    setVisibleCommunities((prev) => {
      const next = new Set(prev);
      if (next.has(cid)) {
        next.delete(cid);
      } else {
        next.add(cid);
      }
      return next;
    });
  }, []);

  // 当 communities 数据变化时，初始化可见的社区
  useEffect(() => {
    if (!communities) { return; }
    const uniqueCommunities = new Set<number>();
    for (const cid of communities.values()) {
      uniqueCommunities.add(cid);
    }
    setVisibleCommunities(uniqueCommunities);
  }, [communities]);

  // 预计算社区筛选状态：全量社区集合 + 是否启用筛选
  useEffect(() => {
    if (!communities) {
      visibleCommunitiesAllSetRef.current = new Set();
      hasCommunityFilterRef.current = false;
      return;
    }
    const allCids = new Set<number>();
    for (const cid of communities.values()) {
      allCids.add(cid);
    }
    visibleCommunitiesAllSetRef.current = allCids;
    hasCommunityFilterRef.current = visibleCommunities.size < allCids.size;
  }, [communities, visibleCommunities]);

  // 选中/导航到折叠社区内的节点时：自动展开该社区，确保目标可见
  useEffect(() => {
    if (!selectedNodeId || !clusterModeRef.current || !effectiveCommunitiesRef.current) {
      return;
    }
    const cid = getCommunityId(selectedNodeId);
    if (cid !== undefined && collapsedRef.current.has(cid)) {
      const next = new Set(collapsedRef.current);
      next.delete(cid);
      collapsedRef.current = next;
      // 标记为手动展开，防止 LOD 自动折叠
      const manualNext = new Set(manualExpandedRef.current);
      manualNext.add(cid);
      manualExpandedRef.current = manualNext;
      refreshClusterGeom();
      buildAggregatePhysics();
      setClusterCollapseVersion((v) => v + 1);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedNodeId, communities]);

  // 选中节点时自动聚焦（搜索定位 / 点击导航）
  const prevSelectedRef = useRef<string | null>(null);
  // 画布交互（点击/拖拽/右键/触摸）触发的选中不聚焦——用户已在节点旁，
  // 相机突变会破坏拖拽手感；仅外部驱动（搜索定位/列表导航/笔记跳转）时聚焦
  const suppressAutoFocusRef = useRef(false);
  useEffect(() => {
    if (!selectedNodeId || selectedNodeId === prevSelectedRef.current) {
      return;
    }
    prevSelectedRef.current = selectedNodeId;
    if (suppressAutoFocusRef.current) {
      suppressAutoFocusRef.current = false;
      return;
    }
    // 延迟到下一帧，确保物理节点已就绪
    requestAnimationFrame(() => {
      const nodes = physNodesRef.current;
      const node = nodes.find((n) => n.id === selectedNodeId);
      if (!node) { return; }
      // 平滑移动相机到节点位置（400ms 缓动，避免相机突变割裂感）
      const cam = cameraRef.current;
      const targetZoom = Math.max(cam.zoom, 1.5);
      const targetX = -node.x * targetZoom;
      const targetY = -node.y * targetZoom;
      const startX = cam.x;
      const startY = cam.y;
      const startZoom = cam.zoom;
      const duration = 400;
      const startTime = performance.now();
      const animate = (now: number) => {
        const elapsed = now - startTime;
        const t = Math.min(elapsed / duration, 1);
        const ease = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
        cam.x = startX + (targetX - startX) * ease;
        cam.y = startY + (targetY - startY) * ease;
        cam.zoom = startZoom + (targetZoom - startZoom) * ease;
        if (t < 1) {
          requestAnimationFrame(animate);
        }
      };
      requestAnimationFrame(animate);
    });
  }, [selectedNodeId]);

  // 容器尺寸监听（rAF 去抖，避免拖动窗口时高频触发渲染重建）
  useEffect(() => {
    const el = containerRef.current;
    if (!el) { return; }
    let rafId = 0;
    const update = () => setDimensions({ width: el.clientWidth, height: el.clientHeight });
    update();
    const scheduleUpdate = () => {
      if (rafId) { return; }
      rafId = requestAnimationFrame(() => {
        rafId = 0;
        setDimensions({ width: el.clientWidth, height: el.clientHeight });
      });
    };
    const ro = new ResizeObserver(scheduleUpdate);
    ro.observe(el);
    return () => {
      if (rafId) { cancelAnimationFrame(rafId); }
      ro.disconnect();
    };
  }, []);

  // 全屏状态
  useEffect(() => {
    const handle = () => setIsFullscreen(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", handle);
    return () => document.removeEventListener("fullscreenchange", handle);
  }, []);

  // 数据变化 → 重建物理世界
  useEffect(() => {
    if (!data || data.nodes.length === 0) { return; }

    // 供主题 effect 重算颜色缓存（数据 effect 不依赖 token，主题切换不重建图）
    dataRef.current = data;
    // ⚠ 与下方 `effectiveCommunities` 用**同一份合成映射**（实体侧社区也错开命名空间）。
    // 这里若退回只存 `communities`，主题切换时会用「只有笔记侧」的映射重算颜色缓存，
    // 而彩球/气泡用的是合成映射 ⇒ 实体节点的颜色与它所属的桶对不上（N5 那类错位的复发形态）。
    rawCommunitiesRef.current = mergeEntityCommunities(communities, entityCommunities) ?? null;

    // 清空 minimap 包围盒缓存（节点集已变化，旧缓存失效）
    minimapBBoxRef.current = null;

    // 新数据集 → 请求一次自动适应视图（Worker 预热完成后执行）
    autoFitPendingRef.current = true;

    // 构建物理节点
    const pNodes: PhysicsNode[] = data.nodes.map((n, i) => ({
      id: n.id,
      x: n.x ?? 0,
      y: n.y ?? 0,
      vx: 0,
      vy: 0,
      fx: 0,
      fy: 0,
      mass: 1 + (n.linkCount + n.backlinkCount) * 0.2,
      fixed: false,
      kind: n.type,
      idx: i,
    }));

    // 首次布局：优先从 localStorage 加载已保存的布局
    let layoutApplied = false;
    if (wikiId) {
      const saved = loadLayout(wikiId);
      if (saved) {
        layoutApplied = applySavedLayout(pNodes, saved);
        // D7: 仅当布局成功恢复时才恢复相机视角（布局不匹配时视角会偏移）
        if (layoutApplied && saved.camera) {
          cameraRef.current.x = saved.camera.x;
          cameraRef.current.y = saved.camera.y;
          cameraRef.current.zoom = saved.camera.zoom;
        }
      }
    }

    // 若无已保存布局或匹配率太低（applySavedLayout 返回 false），则使用圆形布局
    if (!layoutApplied) {
      initializePositions(pNodes, dimensions.width, dimensions.height);
    }

    // 邻接表 → 物理边
    const adjacency = new Map<string, Set<string>>();
    for (const n of data.nodes) { adjacency.set(n.id, new Set()); }
    for (const e of data.edges) {
      adjacency.get(e.source)?.add(e.target);
      adjacency.get(e.target)?.add(e.source);
    }
    const avgDegree = data.edges.length > 0 ? (data.edges.length * 2) / data.nodes.length : 1;
    const pEdges = buildPhysicsEdges(adjacency, pNodes, avgDegree);

    physNodesRef.current = pNodes;
    physEdgesRef.current = pEdges;

    // ── 预热迭代：移交给 Worker 在 init 时后台执行 ──
    // 此前在主线程同步跑 40~80 次 Barnes-Hut，几万节点首开会冻结 UI 数秒（
    // 见下方"不在主线程同步跑 stepPhysics"的说明）。现在只计算预热参数，
    // 随 Worker init 消息传入，由 Worker 完成初始布局收敛，主线程保持响应。
    // 预热迭代数降为 20~30 次（Worker 单步更快），保证 ready 快速返回，
    // 剩余收敛由渲染循环的持续 STEP 完成。
    let warmupIters = 0;
    if (!layoutApplied) {
      warmupIters = pNodes.length > 5000 ? 20 : 30;
    }

    // 构建渲染缓存：posMap (O(N) 一次性) + 邻居集合 (O(E) 一次性)
    const posMap = new Map<string, PhysicsNode>();
    for (const n of pNodes) { posMap.set(n.id, n); }
    posMapRef.current = posMap;
    neighborsRef.current = adjacency; // 已在上方构建

    // 构建物理引擎缓存：邻居表 + 节点索引（供 stepPhysics 复用，避免每帧重建）
    neighborMapCacheRef.current = buildNeighborMap(pEdges);
    nodeMapCacheRef.current = buildNodeMap(pNodes);

    // 重置稳定计数器，强制物理引擎重新运行
    stableFrameCounterRef.current = 0;

    // 构建网格空间索引
    const gridIndex = new Map<string, string[]>();
    for (const n of pNodes) {
      const gx = Math.floor(n.x / GRID_CELL_SIZE);
      const gy = Math.floor(n.y / GRID_CELL_SIZE);
      const key = `${gx},${gy}`;
      const bucket = gridIndex.get(key);
      if (bucket) {
        bucket.push(n.id);
      } else {
        gridIndex.set(key, [n.id]);
      }
    }
    gridIndexRef.current = gridIndex;

    // 节点元数据
    const metaMap = new Map<string, GraphNode>();
    const sizeMap = new Map<string, number>();
    for (const n of data.nodes) {
      metaMap.set(n.id, n);
      sizeMap.set(n.id, getNodeSize(n));
    }
    nodeMetaRef.current = metaMap;
    nodeSizeRef.current = sizeMap;

    // 边元数据（用于渲染），直接存储 sourceIdx/targetIdx 避免渲染循环中的 Map 查找
    const edgeStyles = getEdgeTypeStylesMap(tokenRef.current);
    const idToIdx = new Map<string, number>();
    for (let i = 0; i < pNodes.length; i++) {
      idToIdx.set(pNodes[i].id, i);
    }
    edgeMetaRef.current = data.edges.map((e) => {
      const style = edgeStyles[e.type] || edgeStyles.link;
      return {
        source: e.source,
        target: e.target,
        type: e.type,
        animated: style.animated,
        color: style.color,
        width: style.width,
        sourceIdx: idToIdx.get(e.source) ?? -1,
        targetIdx: idToIdx.get(e.target) ?? -1,
      };
    });

    // 粒子系统（动态上限：大图场景自动减少粒子数）
    const particleNodeCount = pNodes.length;
    const maxParticles = particleNodeCount > 10000 ? 300 : particleNodeCount > 5000 ? 1000 : 4000;
    const particles: Particle[] = [];
    for (let i = 0; i < data.edges.length; i++) {
      if (particles.length >= maxParticles) { break; }
      const em = edgeMetaRef.current[i];
      if (em.animated) {
        // 每条动画边 1-2 个粒子
        const count = em.type === "reference" ? 2 : 1;
        for (let j = 0; j < count; j++) {
          if (particles.length >= maxParticles) { break; }
          particles.push({
            edgeIndex: i,
            progress: Math.random(),
            speed: 0.003 + Math.random() * 0.004,
            size: em.type === "reference" ? 2.5 : 1.8,
            color: em.color,
          });
        }
      }
    }
    particlesRef.current = particles;

    // 初始布局收敛交由 Worker 完成（见下文 Worker init + 渲染循环持续 STEP）。
    // 不在主线程同步跑 stepPhysics：几万节点时 Barnes-Hut 单步即数百 ms，
    // 主线程同步迭代会冻结 UI 数秒。Worker 就绪前节点保持 initial/保存布局即可。

    // ── 预计算有效社区映射（在 Worker 初始化之前执行） ──
    // 大图（>3000节点）必须进入聚类模式，无论社区粒度如何。
    //
    // ⚠ 2026-09-18：先把**笔记侧**与**实体侧**两份社区合成一张映射再往下走 ——
    // 两份是**两次独立 Louvain** 的产物，cid 值域会重合，必须错开命名空间
    // （原因与后果见 `communityMerge.mergeEntityCommunities` 的文件头）。
    // 放在 forceCluster 判定**之前**：小图分支同样需要实体节点有桶，
    // 否则两条分支会对同一份数据给出不同的染色/聚合结果。
    const fusedCommunities = mergeEntityCommunities(communities, entityCommunities);
    let effectiveCommunities: Map<string, number> | undefined = fusedCommunities;
    const shouldForceCluster = pNodes.length > AUTO_CLUSTER_THRESHOLD;
    if (shouldForceCluster) {
      // 桶数上限取「目标桶数」与「聚合物理上限」的较小者 —— 把「归并桶数 ≤ 气泡层阈值」
      // 这条不变量钉成代码，而不是依赖两个常量恰好一大一小。
      const targetClusterCount = Math.min(TARGET_CLUSTER_COUNT, MAX_AGG_PHYS_NODES);
      const sourceCommunities = effectiveCommunities;
      const distinctCommunities = countDistinctCommunities(sourceCommunities);
      if (!sourceCommunities) {
        // 没有社区数据 ⇒ 无拓扑可用，这是唯一保留哈希分桶的路径。
        // 桶数用 targetClusterCount，故桶 id 仍落在已有范围内、不会越界。
        const hashMap = new Map<string, number>();
        for (const n of pNodes) {
          hashMap.set(n.id, Math.abs(hashStringToInt(n.id)) % targetClusterCount);
        }
        effectiveCommunities = hashMap;
      } else if (distinctCommunities > targetClusterCount) {
        // ⚠ 判据修正（2026-09-16）：原判据 `effectiveCommunities.size > MAX_AGG_PHYS_NODES` 取错了量 ——
        // `.size` 是 **Map 的节点条目数**（本数据集 24288），不是**不同社区 id 的个数**（2458）。
        // 而该分支仅在 pNodes.length > 3000 时进入 ⇒ `节点数 > 3000 > 800` **恒真**
        // ⇒ 归并无条件执行、真实社区被整体丢弃（审计报告 §6.4 有完整恒真证明）。
        //
        // ⚠ 分桶内容改为**拓扑感知归并**（communityMerge.ts）：原实现是 `hash(nodeId) % 200`，
        // 与拓扑无关 —— 实测其同区内边占比 0.49%，与随机划分期望 0.50% 相等；
        // 且聚合图边密度 82.69%（逼近完全图）⇒ 聚合节点受力趋同 ⇒ 力导向退化为均匀铺开，
        // 宏观上就是「一片互不相连、均匀分布的点」（审计报告 §6.3 / §6.8.3）。
        const outcome = mergeCommunitiesTopologically({
          communities: sourceCommunities,
          edges: data.edges,
          targetCount: targetClusterCount,
        });
        effectiveCommunities = outcome.communities;
      } else {
        // 社区粒度已足够粗（不同社区数 ≤ 目标桶数）⇒ 直接用真实社区，不做任何归并。
        effectiveCommunities = sourceCommunities;
      }
      // 关键修复：在 forceCluster 模式下，确保所有节点都被映射到社区。
      // 即使原始 communities 数据已经存在，也可能只覆盖了部分节点。
      // 补全缺失节点的社区分配，确保 buildAggregatePhysics 能正确处理所有边。
      if (effectiveCommunities) {
        let hasMissingNodes = false;
        for (const n of pNodes) {
          if (!effectiveCommunities.has(n.id)) {
            hasMissingNodes = true;
            break;
          }
        }
        if (hasMissingNodes) {
          // 补全缺失节点：并入**已存在的**桶，绝不生成新 id。
          // ⚠ 原实现是 `hash(n.id) % FORCE_CLUSTER_COUNT`（裸的 0..199 取值）——
          // 在「哈希桶」时代它恰好落在已有桶范围内，但在真实社区路径下
          // （cid 实测值域 [246, 24284]）会凭空造出一批不在集合里的桶。
          // 现在按「已有桶 id 集合」索引，语义与 cid 的数值范围无关。
          //
          // ⚠⚠ 2026-09-18：补全顺序改为「**先跟随锚点，再 hash 兜底**」。
          // 只按 hash 补全是错的 —— 这不是观感判断，是本仓自己写下的判据（见下方引用）。
          // 成因：融合进来的实体节点在 notes-only 的社区映射里没有条目
          // （Louvain 的输入是 `note::get_vault_graph`，而融合发生在其后），
          // 近半数节点被**随机**撒进 200 个桶 ⇒ 桶级聚合边 1,873 → **18,011**、
          // 密度 9.4% → **90.5%**（C(200,2)=19,900 ⇒ 91% 的桶对有边）。
          // 而上方「拓扑感知归并」那段注释（本文件 :1273-1276）记录过这条形态的后果：「密度逼近完全图 ⇒ 聚合节点受力趋同
          // ⇒ 力导向退化为均匀铺开，宏观上就是一片互不相连、均匀分布的点」。
          // 锚点就是融合层为「实体 ↔ 同名笔记」合成的 `mapping` 边
          // （`wiki.rs:1173-1192`）—— 实体继承其笔记的桶，桶级图因此保持稀疏
          // （实测聚合边 18,011 → 1,947、密度回到 9.8%）。
          // 逻辑在 `communityMerge.assignMissingCommunities`（纯函数、可单测、确定性）。
          //
          // ⚠⚠ 同日晚些时候：**实体侧已有自己的社区**（后端在实体子图上单独跑 Louvain，
          // 见本节开头的 `mergeEntityCommunities`）⇒ 走到这里的缺桶节点应当只剩
          // 「图上新出现、社区缓存还没覆盖」的那批。锚点路径因此从「主路径」降为「兜底」，
          // 但判据不变：`viaHash > 0` 仍然是「锚点这条路也断了」的信号
          // （`warnOnHashFallback` 继续按值变化节流告警）。
          const anchors: Array<{ source: string; target: string }> = [];
          for (const em of data.edges) {
            if (em.type === "mapping") {
              anchors.push({ source: em.source, target: em.target });
            }
          }
          const assignment = assignMissingCommunities({
            nodeIds: pNodes.map((n) => n.id),
            communities: effectiveCommunities,
            anchors,
          });
          effectiveCommunities = assignment.communities;
          warnOnHashFallback(wikiIdRef.current, assignment);
        }
      }
      // 关键：更新 effectiveCommunitiesRef，供 Worker 初始化和后续代码使用
      effectiveCommunitiesRef.current = effectiveCommunities;
    } else {
      // 小图（≤ AUTO_CLUSTER_THRESHOLD）：直接用**合成后**的原始社区，不做归并/补全。
      // 这里同样是 `fusedCommunities`（含实体侧）—— 理由见本节开头。
      effectiveCommunitiesRef.current = effectiveCommunities;
    }

    // ── 节点颜色缓存 ──
    // N5 修复：颜色缓存构建必须放在 effectiveCommunities 计算之后——
    // force-cluster 哈希合并模式下按"虚拟聚类 ID"染色，与聚合彩球/气泡
    // （communityPalette[cid]）取色一致，否则展开社区后内部节点颜色与彩球不对应。
    // 普通模式下 effectiveCommunitiesRef.current 即**合成后的**原始社区（笔记侧 ∪ 实体侧），
    // 行为相对引入实体侧社区之前不变（那时实体侧为空、合成退化为恒等）。
    const colorCommunities = effectiveCommunitiesRef.current ?? communities;
    nodeColorRef.current = buildNodeColorCache(data.nodes, colorCommunities, tokenRef.current);
    buildNodeSpriteCache();

    // ── 初始化物理 Worker ──
    // 销毁旧 Worker
    if (workerRef.current) {
      workerRef.current.postMessage({ type: "destroy" } as WorkerMessage);
      workerRef.current.terminate();
      workerRef.current = null;
      workerInitializedRef.current = false;
      lastProcessedTickRef.current = -1;
    }

    const worker = new Worker(
      new URL("./graphPhysics.worker.ts", import.meta.url),
      { type: "module" },
    );
    workerRef.current = worker;

    // ── 零拷贝初始化：使用 Float64Array + Transfer List ──
    const workerConfig: PhysicsConfig = {
      theta: 0.5,
      repulsion: 18000,
      gravity: 0.003,
      damping: 0.82,
      dt: 0.35,
      springForce: 0.08,
      springDamping: 0.85,
      maxVelocity: 8,
    };

    // ── 零拷贝初始化：使用 Float64Array + Int32Array + Transfer List ──
    // 彻底消除字符串数组的 structured clone 开销（2万+ 字符串序列化阻塞主线程数秒）
    // 节点布局：[x, y, vx, vy, fx, fy, mass, fixed(0/1), kind(enum), idx] = 10 floats
    // 边布局：[sIdx, tIdx, restLength, stiffness, damping] = 5 floats
    const nodeCount = pNodes.length;
    const edgeCount = pEdges.length;
    const NODE_STRIDE = 10;
    const EDGE_STRIDE = 5;
    const nodeBuffer = new Float64Array(nodeCount * NODE_STRIDE);
    const edgeBuffer = new Float64Array(edgeCount * EDGE_STRIDE);
    // 节点类型枚举映射（用 Uint8Array 传输，避免字符串序列化）
    const kindToEnum = new Map<string, number>();
    const nodeKindEnum = new Uint8Array(nodeCount);
    // 直接构建 节点索引 → 社区ID 映射（Int32Array，零拷贝传输）
    // 避免 Worker 中用 nodeIds 反查 communities 的二次构建开销
    const nodeIdxToCommunity = new Int32Array(nodeCount).fill(-1);
    const ecLookup = effectiveCommunities;
    for (let i = 0; i < nodeCount; i++) {
      const n = pNodes[i];
      const base = i * NODE_STRIDE;
      nodeBuffer[base] = n.x;
      nodeBuffer[base + 1] = n.y;
      nodeBuffer[base + 2] = n.vx;
      nodeBuffer[base + 3] = n.vy;
      nodeBuffer[base + 4] = n.fx;
      nodeBuffer[base + 5] = n.fy;
      nodeBuffer[base + 6] = n.mass;
      nodeBuffer[base + 7] = n.fixed ? 1 : 0;
      // kind 枚举化
      let kindVal = kindToEnum.get(n.kind);
      if (kindVal === undefined) {
        kindVal = kindToEnum.size;
        kindToEnum.set(n.kind, kindVal);
      }
      nodeKindEnum[i] = kindVal;
      nodeBuffer[base + 8] = kindVal;
      nodeBuffer[base + 9] = n.idx;
      // 社区映射（直接用节点 ID 查找）
      if (ecLookup) {
        const cid = ecLookup.get(n.id);
        if (cid !== undefined) {
          nodeIdxToCommunity[i] = cid;
        }
      }
    }

    for (let e = 0; e < edgeCount; e++) {
      const edge = pEdges[e];
      const eBase = e * EDGE_STRIDE;
      edgeBuffer[eBase] = edge.sourceIdx;
      edgeBuffer[eBase + 1] = edge.targetIdx;
      edgeBuffer[eBase + 2] = edge.restLength;
      edgeBuffer[eBase + 3] = edge.stiffness;
      edgeBuffer[eBase + 4] = edge.damping;
    }

    const initMsg: WorkerMessage = {
      type: "init",
      payload: {
        nodes: [],
        edges: [],
        config: workerConfig,
        communities: undefined, // 已通过 nodeIdxToCommunity 传递，不再需要
        // 预热参数：Worker init 时在后台完成初始布局收敛（避免主线程同步冻结）
        warmupIterations: warmupIters,
        warmupConfig: warmupIters > 0 ? WARMUP_PHYSICS_CONFIG : undefined,
        compact: {
          nodeBuffer,
          edgeBuffer,
          nodeIdxToCommunity,
          nodeKindEnum,
          nodeCount,
          edgeCount,
        },
      },
    };

    // 使用 Transfer List 实现零拷贝：所有 ArrayBuffer 所有权直接转移到 Worker
    // 彻底消除 structured clone 开销（之前 2万+ 字符串序列化阻塞主线程数秒）
    worker.postMessage(initMsg, [
      nodeBuffer.buffer,
      edgeBuffer.buffer,
      nodeIdxToCommunity.buffer,
      nodeKindEnum.buffer,
    ]);

    worker.onmessage = (e: MessageEvent<WorkerResponse>) => {
      const msg = e.data;
      if (msg.type === "ready") {
        workerInitializedRef.current = true;
        workerErrorCountRef.current = 0;
        // 预热完成（节点已大致收敛）→ 自动适应视图一次，保证首屏能看到整张图
        if (autoFitPendingRef.current) {
          autoFitPendingRef.current = false;
          setTimeout(() => fitAllRef.current?.(), 0);
        }
      } else if (msg.type === "result") {
        workerResultRef.current = {
          positions: msg.payload.positions,
          velocities: msg.payload.velocities,
          stable: msg.payload.stable,
          tick: msg.payload.tick,
        };
        pendingStepRef.current = false;
        workerErrorCountRef.current = 0;
      } else if (msg.type === "error") {
        console.error("[GraphWorker]", msg.message);
        pendingStepRef.current = false;
        workerErrorCountRef.current++;
        // 连续 3 次错误：terminate 并降级到主线程物理
        if (workerErrorCountRef.current >= 3 && workerRef.current === worker) {
          console.warn("[GraphWorker] persistent errors, falling back to main-thread physics");
          worker.terminate();
          workerRef.current = null;
          workerInitializedRef.current = false;
        }
      }
    };

    // ── 大图自动聚合：设置折叠状态（延迟计算放到 requestIdleCallback 或下一帧） ──
    if (shouldForceCluster) {
      const comm = effectiveCommunitiesRef.current;

      // 关键：同步更新 communitiesRef（buildAggregatePhysics 依赖它）
      communitiesRef.current = comm;

      // 自动 force cluster 模式：默认不折叠社区，让用户看到真实节点
      // 用户可通过 UI 手动切换聚类模式来折叠/展开
      isAutoForceClusterRef.current = true;
      collapsedRef.current = new Set();
      clusterModeRef.current = true;
      // 注意：不在此处同步调用 refreshClusterGeom/buildAggregatePhysics
      // 这两个函数在 2万+ 节点下是 O(N) + O(E)，会阻塞主线程数秒
      // 改为延迟到 Worker ready 后再计算（Worker ready 回调中处理）
      setClusterMode(true);

      // Worker ready 回调中处理聚合几何和物理构建
      const originalOnMessage = worker.onmessage.bind(worker);
      worker.onmessage = (e: MessageEvent<WorkerResponse>) => {
        originalOnMessage(e);
        if (e.data.type === "ready") {
          // 用 setTimeout(0) 而非 requestIdleCallback：
          // requestIdleCallback 在高负载下可能长时间不触发，导致 clusterGeom 始终为空
          // setTimeout(0) 会立即在下一个事件循环中执行，确保聚类数据尽快就绪
          setTimeout(() => {
            if (!clusterModeRef.current) { return; }
            refreshClusterGeom();
            buildAggregatePhysics();
            setClusterCollapseVersion((v) => v + 1);
          }, 0);
        }
      };
    }

    // 组件卸载时销毁 Worker，避免线程泄漏和内存堆积
    return () => {
      if (workerRef.current === worker) {
        worker.postMessage({ type: "destroy" } as WorkerMessage);
        worker.terminate();
        workerRef.current = null;
        workerInitializedRef.current = false;
      }
    };
    // 依赖不含 token：主题切换不再触发物理世界重建（布局保留），
    // 颜色更新由下方"主题 effect"单独处理。
  }, [data, communities, entityCommunities]);

  // ── 主题 effect：token 变化时只重算颜色缓存，不重建物理世界 ──
  // 数据 effect 已不依赖 token；此处保证明暗主题切换后节点/边/粒子/背景颜色即时更新，
  // 同时保留当前布局与相机状态，避免此前"切换主题导致布局重置"的问题。
  useEffect(() => {
    const d = dataRef.current;
    if (!d || d.nodes.length === 0) { return; }

    // 节点颜色（社区色 palette 为常量，类型色随主题更新）
    // N5 修复：优先使用 effectiveCommunities（force-cluster 哈希合并后的虚拟聚类），
    // 与聚合彩球颜色保持一致；未启用聚类时即原始 communities
    nodeColorRef.current = buildNodeColorCache(
      d.nodes,
      effectiveCommunitiesRef.current ?? rawCommunitiesRef.current ?? undefined,
      token,
    );

    // 边样式颜色/宽度
    const edgeStyles = getEdgeTypeStylesMap(token);
    const meta = edgeMetaRef.current;
    if (meta) {
      for (const m of meta) {
        const style = edgeStyles[m.type] || edgeStyles.link;
        m.color = style.color;
        m.width = style.width;
        m.animated = style.animated;
      }
    }

    // 粒子颜色跟随边颜色
    const particles = particlesRef.current;
    if (particles) {
      for (const p of particles) {
        const em = meta[p.edgeIndex];
        if (em) { p.color = em.color; }
      }
    }

    // 重建节点精灵缓存 + 清空背景渐变缓存（颜色来自 token）
    buildNodeSpriteCache();
    bgCacheRef.current = null;
    minimapBBoxRef.current = null;
  }, [token]);

  // 主动画循环
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) { return; }

    const ctx = canvas.getContext("2d");
    if (!ctx) { return; }

    let running = true;

    // 预渲染背景到离屏画布
    const ensureBackground = (w: number, h: number) => {
      const cache = bgCacheRef.current;
      if (cache && bgCacheSizeRef.current.w === w && bgCacheSizeRef.current.h === h) {
        return cache;
      }
      const offscreen = document.createElement("canvas");
      offscreen.width = w;
      offscreen.height = h;
      const offCtx = offscreen.getContext("2d")!;

      // 绘制背景（纯色渐变，无网格点阵——对齐 Obsidian 的干净感）
      const grad = offCtx.createRadialGradient(w / 2, h / 2, 0, w / 2, h / 2, Math.max(w, h) * 0.7);
      grad.addColorStop(0, token.colorBgContainer);
      grad.addColorStop(1, token.colorBgElevated);
      offCtx.fillStyle = grad;
      offCtx.fillRect(0, 0, w, h);

      bgCacheRef.current = offscreen;
      bgCacheSizeRef.current = { w, h };
      return offscreen;
    };

    const render = () => {
      if (!running) { return; }

      const dpr = window.devicePixelRatio || 1;
      const w = dimensions.width;
      const h = dimensions.height;

      // ── Worker 未就绪时的大图保护：节点数 > 3000 且 Worker 未就绪时，
      // 跳过完整渲染（只保留上一帧画面），避免在主线程用 fallback 渲染 20k 节点。
      // Worker 初始化通常 < 500ms，此期间显示加载指示器即可。
      // ⚠ 2026-09-17 **上移**到跳帧判据之前：跳帧同样必须知道「闸是关的」——
      //   闸关闭期间那些帧一帧都没画，把它们计成「闲置」正是首屏全空的成因（见下）。
      const workerNotReadyLargeGraph = !workerInitializedRef.current && physNodesRef.current.length > 3000;

      // ── 空闲跳帧：系统闲置超过 1 秒且无交互时，完全跳过 Canvas 绘制 ──
      // 节点位置由 Worker/物理模拟驱动，稳定后画面不变；跳帧避免每帧 O(N+E) 遍历
      // 大图（万级节点）下这是关键优化：将 60fps 全量渲染降为按需渲染
      //
      // ⚠ 2026-09-17 修复「大图首屏永久空白」（`AUDIT-wiki-graph-edges-2026-09-15` §6.14-①）：
      //   旧判据只有 `idleCounterRef.current > 60`，而该计数由**物理分支**累加 —— 物理分支在
      //   渲染闸**之前**，与「这一帧画没画」完全无关。2.4 万节点下 Worker 预热实测 ≈4.1s，
      //   这期间渲染闸全程关闭（一帧都没画过），而 idle 已累到 >60 ⇒ 闸一打开就**永久**命中
      //   本分支（本分支在背景绘制**之前** return）⇒ 画布上只剩预热期画下的背景，
      //   **首屏全空**，直到用户碰一下鼠标（现名 `isFrameChanging`，见下方跳帧分支）才恢复。
      //   两条修正缺一不可：
      //     (a) **至少画满过一帧**才允许跳帧 ——「还没画」不等于「不需要画」；
      //     (b) 闸关闭期间清零 idle —— 没画的帧不计入「闲置」。
      if (workerNotReadyLargeGraph) {
        idleCounterRef.current = 0;
      }
      const canSkipFrame = hasPaintedFrameRef.current && !workerNotReadyLargeGraph;
      if (idleCounterRef.current > IDLE_SKIP_FRAMES && canSkipFrame) {
        // ⚠ 与渲染主体的 `isViewInteracting` **故意不同名也不同义**（2026-09-17 登记）：
        //   跳帧必须把**拖动节点**也算成「画面在变」——拖动中每次 mousemove 都在改节点坐标，
        //   跳掉就会表现为「拖不动」；而渲染主体那边拖动由 `hasDrag` 单独处理（物理步进、
        //   命中检测各有分支），不需要在这里重复包含 dragRef。二者勿合并成同一个谓词。
        const isFrameChanging = mouseScreenRef.current.active || !!dragRef.current || !!panRef.current;
        if (!isFrameChanging) {
          rafRef.current = requestAnimationFrame(render);
          return;
        }
      }

      // ── 绘制降频：空闲超过 0.5 秒时，每 2 帧才绘制一次 ──
      // 物理仍以 60fps 运行，但 Canvas 渲染降为 30fps
      const isIdleSlow = idleCounterRef.current > IDLE_RENDER_DECIMATE_FRAMES;
      const shouldRender = !isIdleSlow || frameCounterRef.current % 2 === 0;

      // （`workerNotReadyLargeGraph` 已上移到本函数开头、跳帧判据之前，见上方说明）

      if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
        canvas.width = w * dpr;
        canvas.height = h * dpr;
        canvas.style.width = `${w}px`;
        canvas.style.height = `${h}px`;
        // 尺寸变化时重置背景缓存
        bgCacheRef.current = null;
        // ⚠ 赋 width/height 会**清空画布**（画面此刻是白的）⇒ 必须同时解除空闲跳帧，
        //   否则 resize 之后同样要等下一次鼠标交互画面才回来（与首屏全空同族）。
        hasPaintedFrameRef.current = false;
        idleCounterRef.current = 0;
      }

      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      // 绘制缓存背景（一次性拷贝，避免每帧重建渐变）
      const bg = ensureBackground(w, h);
      ctx.drawImage(bg, 0, 0);

      // 相机变换
      const cam = cameraRef.current;
      ctx.save();
      ctx.translate(w / 2 + cam.x, h / 2 + cam.y);
      ctx.scale(cam.zoom, cam.zoom);

      const nodes = physNodesRef.current;
      const edges = physEdgesRef.current;
      frameCounterRef.current++;

      // ── 计算有效社区映射（优先使用哈希合并后的虚拟聚类） ──
      const effCommunities = effectiveCommunitiesRef.current ?? communities;

      // ── Worker 物理步进 + 帧间插值 ──
      const worker = workerRef.current;
      const workerReady = workerInitializedRef.current;
      const hasDrag = !!dragRef.current;
      // ⚠ **不含 dragRef** —— 与跳帧分支的 `isFrameChanging` 不是同一个判据（2026-09-17 登记
      //   为「同名不同义」—— 两处原本共用一个名字，极易被误当成同一件事）：
      //   拖动有独立的 `hasDrag` 分支（物理步进 `!hasDrag`、命中检测），这里只表达
      //   「鼠标在画布内 / 正在平移画布」。
      const isViewInteracting = mouseScreenRef.current.active || !!panRef.current;

      // P9: 交互（拖拽/平移）时节点位置持续变化 → 气泡缓存置脏，下一帧重建
      if (isViewInteracting) {
        clusterRegionCacheRef.current.dirty = true;
      }

      // 预先获取聚合物理状态供 LOD 逻辑使用
      const aggPhys = aggPhysRef.current;
      const aggActive = aggPhys !== null && aggPhys.nodes.length > 0;

      // ── LOD 渐进式聚类展开：根据缩放级别自动展开/折叠社区 ──
      // 类似地图缩放：缩得越近，看到的细节越多
      // 关键修复：自动 force cluster 模式下跳过 LOD 折叠，确保首次打开就能看到节点
      //
      // ⚠ 隐式聚合下**整体禁用** LOD（2026-09-16）。两套语义互斥：
      //   · 隐式聚合 = 物理单位是社区、但**成员节点仍可见**（坐标由回写派生）；
      //   · LOD 折叠 = 被折叠社区的成员**不绘制**、也不参与物理。
      // LOD 一折叠 collapsed 就被填满，而第一版把 implicit 绑在「collapsed 为空」上
      // ⇒ implicit 翻 false ⇒ 而 implicit=false 又让 LOD 被跳过 ⇒ **互相锁死**，
      // 结果是成员坐标冻结（实测 10 秒后逐位不变）而聚合物理仍在跑（气泡中心 ±1400→±3700）。
      // 现在两者按**用户模式**严格互斥：只有「用户手动开启聚类模式」才走 LOD；
      // 关闭聚类 / 大图自动聚类一律走隐式聚合（这也与 :1169「默认不折叠社区，让用户
      // 看到真实节点」一致）。关掉 LOD 的代价可接受：物理已是社区级（无性能压力），
      // 绘制侧本就有视口裁剪 + 大图 0.5 降采样。
      if (clusterModeRef.current && !isAutoForceClusterRef.current && aggActive) {
        const zoom = cam.zoom;
        const geom = clusterGeomRef.current;

        // 自动 force cluster 模式：默认全展开，用户缩放后才启用 LOD
        // 避免首次打开时因 zoom < 0.5 导致 LOD 0 全折叠，节点完全不可见
        if (isAutoForceClusterRef.current && !isViewInteracting) {
          // 保持 collapsed 为空集（全展开），不执行 LOD 折叠
          collapsedRef.current = new Set();
          lastLodLevelRef.current = -1; // 重置 LOD 状态，下次交互时重新计算
        } else {
          // 计算当前 LOD 级别
          let lodLevel = 0;
          if (zoom >= LOD_THRESHOLDS.ALL) { lodLevel = 3; }
          else if (zoom >= LOD_THRESHOLDS.EXPANDED) { lodLevel = 2; }
          else if (zoom >= LOD_THRESHOLDS.VIEWPORT) { lodLevel = 1; }

          // LOD 变化时重新计算折叠状态（防抖：至少保持 5 帧）
          if (lodLevel !== lastLodLevelRef.current && frameCounterRef.current % 5 === 0) {
            lastLodLevelRef.current = lodLevel;

            const newCollapsed = new Set<number>();
            const expandedInThisFrame: number[] = [];
            const prevCollapsedSize = collapsedRef.current.size;

            // 视口范围（世界坐标）
            const viewW = cam.zoom > 0 ? w / cam.zoom : 0;
            const viewH = cam.zoom > 0 ? h / cam.zoom : 0;
            const vx0 = -cam.x / cam.zoom - viewW / 2;
            const vy0 = -cam.y / cam.zoom - viewH / 2;
            const vx1 = -cam.x / cam.zoom + viewW / 2;
            const vy1 = -cam.y / cam.zoom + viewH / 2;

            for (const [cid, g] of geom) {
              // 手动展开的永远保持展开
              if (manualExpandedRef.current.has(cid)) { continue; }

              if (lodLevel === 0) {
                // LOD 0: 全折叠
                newCollapsed.add(cid);
              } else if (lodLevel === 1) {
                // LOD 1: 仅视口内展开
                const inViewport = g.cx >= vx0 && g.cx <= vx1 && g.cy >= vy0 && g.cy <= vy1;
                if (!inViewport) { newCollapsed.add(cid); }
              } else if (lodLevel === 2) {
                // LOD 2: 视口 + 邻近区域展开（2x 视口范围）
                const marginX = viewW;
                const marginY = viewH;
                const inExpanded = g.cx >= vx0 - marginX && g.cx <= vx1 + marginX
                  && g.cy >= vy0 - marginY && g.cy <= vy1 + marginY;
                if (!inExpanded) { newCollapsed.add(cid); }
              }
              // lodLevel === 3: 全展开（newCollapsed 保持空）
            }

            // 渐进式展开：限制每帧新增展开的社区数
            if (newCollapsed.size < collapsedRef.current.size) {
              // 有新的展开，限制数量
              const toExpand = [];
              for (const cid of collapsedRef.current) {
                if (!newCollapsed.has(cid) && !manualExpandedRef.current.has(cid)) {
                  toExpand.push(cid);
                }
              }
              // 按距离视口中心排序，优先展开近处的
              const cx = (vx0 + vx1) / 2;
              const cy = (vy0 + vy1) / 2;
              toExpand.sort((a, b) => {
                const ga = geom.get(a);
                const gb = geom.get(b);
                if (!ga || !gb) { return 0; }
                const da = Math.hypot(ga.cx - cx, ga.cy - cy);
                const db = Math.hypot(gb.cx - cx, gb.cy - cy);
                return da - db;
              });

              // 计算展开后预计的物理节点数
              const expandedCount = toExpand.length;
              const newAggNodeCount = (aggPhys?.nodes.length ?? 0) + expandedCount * 10; // 粗略估算

              // 如果展开后会超出物理节点限制，只展开部分
              const maxExpand = newAggNodeCount > MAX_AGG_PHYS_NODES
                ? Math.max(1, Math.floor((MAX_AGG_PHYS_NODES - (aggPhys?.nodes.length ?? 0)) / 10))
                : MAX_EXPAND_PER_FRAME;

              for (let i = 0; i < Math.min(maxExpand, toExpand.length); i++) {
                newCollapsed.delete(toExpand[i]);
                expandedInThisFrame.push(toExpand[i]);
              }
            }

            // 物理节点数保护：如果当前聚合物理已超限，强制折叠最远的非手动社区
            if (aggPhys && aggPhys.nodes.length > MAX_AGG_PHYS_NODES) {
              const cx = (vx0 + vx1) / 2;
              const cy = (vy0 + vy1) / 2;
              const collapsible = [];
              for (const cid of newCollapsed) {
                if (manualExpandedRef.current.has(cid)) { continue; }
                const g = geom.get(cid);
                if (!g) { continue; }
                collapsible.push({ cid, dist: Math.hypot(g.cx - cx, g.cy - cy), count: g.count });
              }
              // 按距离从远到近排序，折叠最远的
              collapsible.sort((a, b) => b.dist - a.dist);
              let currentOver = aggPhys.nodes.length - MAX_AGG_PHYS_NODES;
              for (const { cid, count } of collapsible) {
                if (currentOver <= 0) { break; }
                newCollapsed.add(cid);
                currentOver -= count;
              }
            }

            collapsedRef.current = newCollapsed;

            debugLog("[GraphView] LOD update", {
              lodLevel,
              newCollapsedSize: newCollapsed.size,
              prevCollapsedSize,
            });

            // LOD 变化导致折叠集合改变 → 重建聚合物理集
            if (newCollapsed.size !== prevCollapsedSize) {
              // 放到 requestIdleCallback 中：避免阻塞下一帧渲染
              scheduleIdle(() => {
                refreshClusterGeom();
                buildAggregatePhysics();
                setClusterCollapseVersion((v) => v + 1);
              }, 500);
            }
          }
        }
      }

      // ── 聚合物理分支（聚类折叠模式）：物理只模拟聚合节点 + 未折叠节点 ──
      // 折叠社区成员不参与力导向模拟（数量级骤降），聚合节点坐标回写 clusterGeom，
      // 驱动折叠社区几何/聚合边/聚合节点渲染。万级节点打开不卡死的核心。
      if (aggActive) {
        // ⚠ 2026-09-16：两条路径**统一**使用 AGG_PHYSICS_CONFIG。
        // 此前显式折叠走另一套（repulsion 18000 / gravity 0.003）且不做尺度归一化，
        // 于是同一份数据在 A/B（隐式）与 C（显式）两个模式下的像素覆盖率差 13 倍
        // （5.379% vs 0.488%）。两套参数的差异没有任何设计依据 —— 聚合物理的单位
        // 在两种模式下都是「社区」，参数就该一致；不一致只会让标定结果互相失效。
        // ⚠ 2026-09-17：`config` 的定义**下移到 `shouldRun` 内**（见下方）—— 退火是
        // 「每推进一步物理就衰减一次」的量，必须在真正步进时才推进。放这里会让被
        // `shouldRun` 跳过的帧也消耗退火时间轴 ⇒ 退火跑在墙上时钟而非物理步数上，
        // 节流一变（`frame % 6`）衰减速度就跟着变，参数不可复现。
        // 规模保护：聚合物理节点过多（社区粒度极细）时放弃力导向，仅静态显示聚合节点，
        // 聚合节点坐标保持质心，避免主线程每帧 O(n log n) 力导向导致完全不响应。
        // 拖拽仍有效（mouse 事件直接写 node.x/y），不受此限制。
        const aggOver = aggPhys.nodes.length > MAX_AGG_PHYS_NODES;
        // 收敛闸：已收敛（跨度稳定）即视为稳定 ⇒ 停止步进 + idle 恢复跳帧。
        // hasDrag 时解除：用户正在改布局，应当继续模拟。
        const settle = aggSettleRef.current;
        const anneal = aggAnnealRef.current;
        if (hasDrag && settle.settled) {
          settle.settled = false;
          settle.calmSteps = 0;
          // 用户亲手改布局 ⇒ 回到全温，否则「拖动后布局不再演化」（冻在退火终值上）
          reheatAggregateAnneal(anneal);
        }
        const stable = aggOver ? false : (settle.settled || isSystemStable(aggPhys.nodes, 0.15));
        if (isViewInteracting) {
          idleCounterRef.current = 0;
        } else if (stable) {
          idleCounterRef.current++;
        } else {
          idleCounterRef.current = 0;
        }
        // ── 物理步进：按**墙上时间**补固定步长，与渲染帧率解耦（2026-09-17）──
        //
        // 改前这里是「非交互时每 6 帧一步」（`frameCounterRef.current % 6 === 0`）。
        // 那个写法把**物理时间**绑在了**渲染帧率**上，而帧率恰恰是本次要修的对象
        // （fit 态每帧 12000 个节点圆 ⇒ 实测 fps 10.8）⇒ 物理只有 1.8 步/秒
        // ⇒ 需要 1100 步的退火收敛要 611 秒 ⇒ 探针 45 秒内退火从未开始、`settled` 零命中。
        // 详细归因与那条正反馈链见 graphAggregate.ts 的 `AGG_PHYS_STEP_MS` 注释。
        const nowMs = performance.now();
        const clock = aggClockRef.current;
        let elapsedMs = clock.lastMs >= 0 ? nowMs - clock.lastMs : 0;
        // 时钟回退 / NaN（`performance.now()` 单调，但组件里没有别的保证）：按零债处理
        if (!Number.isFinite(elapsedMs) || elapsedMs < 0) { elapsedMs = 0; }
        clock.lastMs = nowMs;
        // 累积器**封顶** = 单帧补步上限 × 步长 ⇒ `stepsDue ≤ AGG_PHYS_MAX_STEPS_PER_FRAME`
        // 是精确成立的（不是「大致不超」）。封顶是必需的：切回前台 / 长任务后
        // 时间差可能是秒级，不封顶会在一帧内补出几千步，比跳步更糟。
        clock.accMs = Math.min(
          clock.accMs + elapsedMs,
          AGG_PHYS_MAX_STEPS_PER_FRAME * AGG_PHYS_STEP_MS,
        );
        // 交互中保证每帧至少一步：用户拖节点 / 平移时的直觉是「我动它就动」，
        // 若此刻恰好欠着不到一步的时间差，会有「拖不动」的观感。
        if (isViewInteracting && clock.accMs < AGG_PHYS_STEP_MS) {
          clock.accMs = AGG_PHYS_STEP_MS;
        }
        const stepsDue = Math.floor(clock.accMs / AGG_PHYS_STEP_MS);
        clock.accMs -= stepsDue * AGG_PHYS_STEP_MS;
        const shouldRun = !aggOver && !settle.settled && stepsDue > 0;
        if (shouldRun) {
          // 本帧跑过几步（诊断用），以及循环里最后一次归一化因子（仅日志）
          let normFactor = 1;
          // 逐步：物理 → 归一化 → 收敛检测，**与标定测试 `runCalib` 严格同序**
          // （那个顺序是标定结论成立的前提，改动顺序等于让标定失效）。
          for (let s = 0; s < stepsDue; s++) {
            // 退火：本步的速度上限 = base × 温度因子（见 graphAggregate.ts 的「布局退火」段）。
            // ⚠ 不改 `AGG_PHYSICS_CONFIG` 常量本身 —— 它同时是 `seedPhysics` 与标定测试的
            //   输入；就地改会让「播种尺度」跟着温度漂移（播种必须在全温下算 R*）。
            // ⚠ 必须**逐步**取（不能一帧取一次再复用）：退火的时间轴单位是「物理步」，
            //   一帧 8 步却只衰减 1 次会让退火实际比标定慢 8 倍。
            const config: PhysicsConfig = {
              ...AGG_PHYSICS_CONFIG,
              maxVelocity: updateAggregateAnneal(anneal, AGG_PHYSICS_CONFIG.maxVelocity),
            };
            stepPhysics(
              aggPhys.nodes,
              aggPhys.edges,
              config,
              undefined,
              undefined,
              undefined,
              aggPhys.neighborMap,
            );
            // ── 尺度归一化（**两种模式都做**，2026-09-16，登记项 §6.11.7-4）──
            // 放在这里是「一处只做一件事」：物理只决定形状，绝对尺度由纯函数层钉死。
            // 之前只在隐式路径（applyAggregateLayout 内）调用 ⇒ 显式全折叠完全没有尺度控制，
            // 其几何来自 refreshClusterGeom 对**主物理**原始坐标的包围盒（Ω(10⁴)）
            // ⇒ 同样 200 个簇，隐式路径 zoom≈0.45、显式路径 zoom≈0.047。
            // 返回的因子仅用于日志（未触发时恒为 1），不参与后续计算。
            normFactor = normalizeAggregateScale(aggPhys.nodes, AGG_LAYOUT_HALF_SPAN);
            // ── 收敛检测（屏幕位移口径：与「用户是否还看得见变化」同量纲）──
            // 判据本体已抽到 graphAggregate.ts 的 `updateAggregateSettle`（纯函数）。
            // 抽出的动机：它的阈值（AGG_SETTLE_PX / AGG_SETTLE_STEPS）正是**要被标定的对象**，
            // 而标定必须用同一份代码跑 —— 内联在这里时，标定脚本只能重抄一遍判据，
            // 抄的那一刻被测对象就与生产分叉（同族：判据 #7/#313）。
            const settleRes = updateAggregateSettle(settle, aggPhys.nodes, cameraRef.current.zoom);
            if (settleRes.justSettled) {
              debugLog("[GraphView] aggregate layout settled", {
                implicit: aggPhys.implicit,
                calmSteps: settle.calmSteps,
                driftPx: settleRes.driftPx.toFixed(4),
                normFactor: normFactor.toFixed(4),
                zoom: cameraRef.current.zoom.toFixed(3),
                frame: frameCounterRef.current,
                // 新增（2026-09-17）：退火是否真的在推进、推进到哪 —— 这两项是
                // 「退火跑在物理步上还是墙上时钟上」的直接证据（前者才是设计意图）。
                annealSteps: anneal.steps,
                annealScale: anneal.scale.toFixed(6),
              });
            }
            // 已经停了 ⇒ 本帧剩余步数不跑。**同时丢掉那部分时间债**（不还）：
            // 布局已定，补步只是空转；把它攒着会在下一帧（用户一交互）突然释放。
            if (settle.settled) { break; }
          }
          // ── 以下都是 O(N) 级，**每帧只做一次**（放进上面的步循环会让成本 × 8）──
          // 依据：「物理坐标 → 渲染坐标」的同步在同一帧内做多次没有意义 ——
          //     只画最后一帧的坐标，中间态本来就看不见。
          // 聚合节点坐标 → 回写 clusterGeom，驱动折叠社区几何/聚合边/聚合节点渲染
          const geom = clusterGeomRef.current;
          for (const [cid, idx] of aggPhys.cidToNodeIdx) {
            const gn = aggPhys.nodes[idx];
            const g = geom.get(cid);
            if (g) {
              g.cx = gn.x;
              g.cy = gn.y;
            }
          }
          // 隐式聚合：把聚合节点位置**派生**给成员节点（黄金角 + 面积均匀环，
          // 半径与气泡同源）。这是「聚合物理真正激活」的另一半 —— 只让 200 个聚合节点
          // 受力、却让 24288 个成员停在旧位置，社区在画面上依旧不分离。
          // 派生改写的是 physNodesRef 内的同一批对象引用，posMapRef 持有相同引用，
          // 绘制路径自动同步。
          // ⚠ 显式折叠下也必须调用（只是内部不会派生成员）：相机跟随那一支对两种模式
          //   都必要 —— 否则归一化把坐标缩到 ≤1500 后，相机仍停在主物理的 fit（zoom 0.047），
          //   200 个簇会被挤成屏幕上的小点，等于「修了尺度、没修可见性」。
          applyAggregateLayout();
          // 气泡几何/缓存已因坐标变化失效，下一帧重建
          clusterRegionCacheRef.current.dirty = true;
          if (frameCounterRef.current % 120 === 0) {
            debugLog("[GraphView] aggregate scale normalize", {
              implicit: aggPhys.implicit,
              normFactor: normFactor.toFixed(4),
              zoom: cameraRef.current.zoom.toFixed(3),
              stepsDue,
            });
          }
          // 不再用聚合节点覆盖 gridIndex —— 会导致 drawExpandedCommunity 找不到原始节点
          // 原始节点的 gridIndex 已在数据初始化时构建，保持不变
          // 聚合节点的位置变化通过 clusterGeom 的 cx/cy 回写驱动渲染
        }

        // ── 大图位图构建（**必须在 `shouldRun` 之外**，2026-09-17）──
        //
        // 为什么不能放在 `shouldRun` 里：那个条件的第二项就是 `!settle.settled`，
        // 而位图的构建前提恰恰是 `settle.settled === true`（布局停了才会有新鲜位图，
        // 见 `spriteNeedRebuild` 的注释）—— 两个条件**互斥**。放在里面会得到
        // 「位图永远不构建 ⇒ 消费侧永远没有缓存可画」，即这次「启用位图」完全落空。
        // 这个漏洞是先写完再推演执行序时发现的（不是测试抓到的），所以在此写明。
        //
        // 位置语义：聚合模式下主物理被暂停（`aggPhys.implicit === true` 时不请求 worker 步进），
        // 所以这里是**聚合模式下唯一**的位图构建点。成员坐标由上面 `applyAggregateLayout`
        // 派生（改写的是 `physNodesRef` 内的同一批对象引用），所以取它烘制与画面同源。
        // ⚠ 节流必须给「首次构建」开一个口子（2026-09-17 补）：改前是
        //   `frameCounterRef.current - lastAggSpriteRebuildRef.current >= 60` 无条件前置，
        //   于是「布局刚停、位图该烘」的那一刻还要空等最多 60 帧 —— 实测 fps 10.8
        //   ⇒ 白等 **5.5 秒**，而这 5.5 秒里用户看的正是 12000 个节点圆逐帧重绘，
        //   也就是本次要消除的东西。首次构建（`spriteCacheRef === null`）不该受节流：
        //   它不是「重复劳动」，而是整条链的启动点。
        const sinceSpriteRebuild = frameCounterRef.current - lastAggSpriteRebuildRef.current;
        if (
          aggPhys.implicit
          && (spriteCacheRef.current === null || sinceSpriteRebuild >= 60)
          && spriteNeedRebuild(
            physNodesRef.current.length,
            cameraRef.current.zoom,
            settle.settled,
          )
        ) {
          lastAggSpriteRebuildRef.current = frameCounterRef.current;
          spriteCacheRef.current = buildBigGraphSpriteCache(physNodesRef.current);
        }

        // 关键修复：aggActive 模式下仍然需要应用 Worker 结果更新原始节点位置。
        // 之前设置 workerResultRef.current = null 导致原始节点位置从未被更新，
        // 节点停留在初始化时的随机位置，而边使用 clusterGeom 中已更新的聚合位置，
        // 造成"只显示边、不显示节点"的问题。
        if (worker && workerReady && !hasDrag) {
          // 请求下一个 Worker 步进
          // 隐式聚合下**暂停**原始节点物理：此时 24288 个节点的坐标全部由
          // applyAggregateLayout 从聚合节点派生；若同时让 Worker 继续步进并回传位置，
          // 两套坐标每帧互相覆盖 ⇒ 节点持续抖动、气泡与节点团错位。
          if (
            !pendingStepRef.current && !hasDrag && aggPhys.implicit !== true
            && (isViewInteracting || frameCounterRef.current % 12 === 0)
          ) {
            const workerConfig: PhysicsConfig = {
              theta: 0.5,
              repulsion: 18000,
              gravity: 0.003,
              damping: 0.82,
              dt: 0.35,
              springForce: 0.08,
              springDamping: 0.85,
              maxVelocity: 8,
            };

            // P8: 社区质心由 Worker 内部维护，主线程不再每 12 帧 O(N) 重算 + 序列化传输
            worker.postMessage({
              type: "step",
              payload: {
                config: workerConfig,
              },
            } as WorkerMessage);
            pendingStepRef.current = true;
          }

          // 应用 Worker 返回的结果到原始节点位置。
          // 隐式聚合下**跳过**：节点坐标已由聚合布局派生，而 Worker 回传的是「另一套
          // 世界」的位置（且暂停 step 后只是陈旧值），应用它会立刻覆盖派生结果，
          // 表现为节点被拽回旧位置、气泡与节点团错位。
          const result = aggPhys.implicit === true ? null : workerResultRef.current;
          if (result && result.positions) {
            const hasNewResult = result.tick !== lastProcessedTickRef.current;
            if (hasNewResult) {
              lastProcessedTickRef.current = result.tick;
              // P9: Worker 返回新位置 → 气泡缓存置脏，下一帧重建
              clusterRegionCacheRef.current.dirty = true;
              const n = nodes.length;
              // P6: posMap 存节点对象引用，位置更新经同一对象自动同步，无需再逐条回写。
              // 折叠社区内节点不渲染，跳过其位置回写，避免每 12 帧对全量节点做无意义更新。
              const collapsedSet = collapsedRef.current;
              for (let i = 0; i < n; i++) {
                const node = nodes[i];
                if (node.fixed) { continue; }
                if (collapsedSet.size > 0) {
                  const cid = effCommunities?.get(node.id);
                  if (cid !== undefined && collapsedSet.has(cid)) { continue; }
                }
                node.x = result.positions[i * 2];
                node.y = result.positions[i * 2 + 1];
                node.vx = result.velocities[i * 2];
                node.vy = result.velocities[i * 2 + 1];
              }

              // 重建 gridIndex（仅在有新结果时重建）
              const gridIndex = new Map<string, string[]>();
              for (const n2 of nodes) {
                const gx = Math.floor(n2.x / GRID_CELL_SIZE);
                const gy = Math.floor(n2.y / GRID_CELL_SIZE);
                const key = `${gx},${gy}`;
                const bucket = gridIndex.get(key);
                if (bucket) {
                  bucket.push(n2.id);
                } else {
                  gridIndex.set(key, [n2.id]);
                }
              }
              gridIndexRef.current = gridIndex;

              // 稳定检测
              if (result.stable && !isViewInteracting) {
                idleCounterRef.current++;
              } else {
                idleCounterRef.current = 0;
              }
            }
          }
        } else if (worker && !workerReady && !hasDrag) {
          // Worker 未就绪时清空结果标记，避免使用旧结果
          workerResultRef.current = null;
        }
      } else if (worker && workerReady && nodes.length > 0) {
        const enableClusters = clusterModeRef.current && effCommunities;

        // 拖拽时同步位置到 Worker
        if (hasDrag) {
          const dragNode = nodes.find((n) => n.id === dragRef.current!.nodeId);
          if (dragNode) {
            worker.postMessage({
              type: "update",
              payload: {
                nodeIdx: dragNode.idx,
                x: dragNode.x,
                y: dragNode.y,
                fixed: dragNode.fixed,
                vx: dragNode.vx,
                vy: dragNode.vy,
              },
            } as WorkerMessage);
          }
        }

        // 请求下一个物理步进（无 pending 时；稳定后降频到每 12 帧一次，减少 worker 空转）
        if (!pendingStepRef.current && !hasDrag && (isViewInteracting || frameCounterRef.current % 12 === 0)) {
          const config: PhysicsConfig = {
            theta: 0.5,
            repulsion: 18000,
            gravity: 0.003,
            damping: 0.82,
            dt: 0.35,
            springForce: 0.08,
            springDamping: 0.85,
            maxVelocity: 8,
            clusterForce: enableClusters ? 0.15 : undefined,
          };

          // P8: communities 与社区质心均由 Worker 内部维护（init 时 nodeIdxToCommunity 已零拷贝传输），
          // 主线程不再每 12 帧 O(N) 重算质心 + Object.fromEntries 序列化
          worker.postMessage({
            type: "step",
            payload: {
              config,
            },
          } as WorkerMessage);
          pendingStepRef.current = true;
        }

        // 应用 Worker 返回的结果到物理节点
        const result = workerResultRef.current;
        if (result && result.positions) {
          // 关键优化：只有 Worker 返回新结果（tick 变化）时才更新节点和重建网格
          // 否则每帧都会用旧结果重算 O(N) 操作，大图下是性能灾难
          const hasNewResult = result.tick !== lastProcessedTickRef.current;
          if (hasNewResult) {
            lastProcessedTickRef.current = result.tick;
            // P9: Worker 返回新位置 → 气泡缓存置脏，下一帧重建
            clusterRegionCacheRef.current.dirty = true;
            const n = nodes.length;
            for (let i = 0; i < n; i++) {
              const node = nodes[i];
              if (!node.fixed) {
                node.x = result.positions[i * 2];
                node.y = result.positions[i * 2 + 1];
                node.vx = result.velocities[i * 2];
                node.vy = result.velocities[i * 2 + 1];
              }
            }

            // 同步重建 gridIndex：Worker 返回新位置后更新网格索引。
            // L2 修复：收敛期不再每步全量重建（2 万节点下单次 10-30ms，收敛期
            // 每步都做会持续占死主线程），改为每 5 步一次，stable 后重建最终版。
            // 间隔内命中检测使用上一次索引，位置滞后 ≤5 步，物理模拟下可接受。
            workerStepCounterRef.current++;
            if (result.stable || workerStepCounterRef.current % 5 === 0) {
              const gridIndex = new Map<string, string[]>();
              for (const n2 of nodes) {
                const gx = Math.floor(n2.x / GRID_CELL_SIZE);
                const gy = Math.floor(n2.y / GRID_CELL_SIZE);
                const key = `${gx},${gy}`;
                const bucket = gridIndex.get(key);
                if (bucket) {
                  bucket.push(n2.id);
                } else {
                  gridIndex.set(key, [n2.id]);
                }
              }
              gridIndexRef.current = gridIndex;
            }

            // 如果处于聚类模式且 Worker ready，更新聚类几何
            // 仅在非聚合物理模式下（aggActive 下由聚合物理节点回写）
            // L2 修复：与 gridIndex 同门限流，收敛期避免每步都排 O(N) 的 idle 任务
            if (
              clusterModeRef.current && !aggActive
              && (result.stable || workerStepCounterRef.current % 5 === 0)
            ) {
              requestIdleCallback(() => {
                refreshClusterGeom();
              }, { timeout: 200 });
            }

            // 大图位图缓存：异步重建
            // ⚠ 2026-09-17：判据**统一到 `spriteNeedRebuild`**（构建闸 = 消费闸）。
            //   改前这里是「节点数 + zoom + 每 30 步」，与消费侧可达域不相交 ⇒
            //   位图被构建却永远画不出来（实测 `drawImage` 5 参一次都没出现）。
            //   `layoutStable` 传 `result.stable` —— 主物理 worker 的稳定信号。
            //   聚合模式下主物理被暂停、回调不再推进，那时由 `applyAggregateLayout`
            //   那一处用 `settle.settled` 构建（那是聚合模式下唯一的构建点）。
            if (
              !isViewInteracting
              && spriteNeedRebuild(nodes.length, cameraRef.current.zoom, result.stable)
            ) {
              requestIdleCallback(() => {
                spriteCacheRef.current = buildBigGraphSpriteCache(nodes);
              }, { timeout: 1000 });
            }
          }

          // 稳定检测：即使没有新结果，也基于上一次的 stable 状态更新 idle 计数
          if (result.stable && !isViewInteracting) {
            idleCounterRef.current++;
          } else {
            idleCounterRef.current = 0;
          }
        }
      } else if (nodes.length > 0 && !hasDrag) {
        // 回退：没有 Worker 时用原来的主线程物理（兼容 fallback）
        // 大图保护：主线程物理只对中小图可用；超过 MAX_MAIN_THREAD_PHYSICS 时放弃力导向（静态显示）。
        // 否则每帧全量 O(n log n) 会让主线程完全阻塞、主应用无响应。大图等待 Worker 就绪即可。
        const mainThreadSafe = nodes.length <= MAX_MAIN_THREAD_PHYSICS;
        const stable = mainThreadSafe ? isSystemStable(nodes, 0.15) : true;
        if (stable && !isViewInteracting) {
          idleCounterRef.current++;
        } else {
          idleCounterRef.current = 0;
        }
        const shouldRunPhysics = mainThreadSafe
          && (isViewInteracting || !stable || idleCounterRef.current % IDLE_PHYSICS_DECIMATE_FRAMES === 0);
        if (shouldRunPhysics) {
          const enableClusters = clusterModeRef.current && effCommunities;
          let centroids = communityCentroidsRef.current;
          if (enableClusters && frameCounterRef.current % 3 === 0) {
            centroids = computeCommunityCentroids(nodes, effCommunities!);
            communityCentroidsRef.current = centroids;
          }
          const config: PhysicsConfig = {
            theta: 0.5,
            repulsion: 18000,
            gravity: 0.003,
            damping: 0.82,
            dt: 0.35,
            springForce: 0.08,
            springDamping: 0.85,
            maxVelocity: 8,
            clusterForce: enableClusters ? 0.15 : undefined,
          };
          stepPhysics(
            nodes,
            edges,
            config,
            undefined,
            enableClusters ? effCommunities : undefined,
            enableClusters ? centroids : undefined,
            neighborMapCacheRef.current,
          );
          // gridIndex 重建改为异步（主线程 fallback 模式，节点数 <= 1500，影响较小）
          if (frameCounterRef.current % 3 === 0) {
            scheduleIdle(() => {
              const gridIndex = new Map<string, string[]>();
              for (const n of nodes) {
                const gx = Math.floor(n.x / GRID_CELL_SIZE);
                const gy = Math.floor(n.y / GRID_CELL_SIZE);
                const key = `${gx},${gy}`;
                const bucket = gridIndex.get(key);
                if (bucket) {
                  bucket.push(n.id);
                } else {
                  gridIndex.set(key, [n.id]);
                }
              }
              gridIndexRef.current = gridIndex;
            }, 100);
          }
        }
      }

      phaseRef.current += 0.02;

      // 获取当前交互状态（绘制阶段需要）
      const hovered = hoverNodeRef.current;
      const selected = selectedNodeIdRef.current;

      // 计算鱼眼参数（世界坐标下的鼠标位置 + 放大因子）
      const fisheye = computeFisheye();

      // 计算当前视口的世界坐标范围（用于视口裁剪）
      const viewWorld = {
        x0: (-w / 2 - cam.x) / cam.zoom - 50,
        y0: (-h / 2 - cam.y) / cam.zoom - 50,
        x1: (w / 2 - cam.x) / cam.zoom + 50,
        y1: (h / 2 - cam.y) / cam.zoom + 50,
      };

      // 绘制社区聚类区域（背景层；5 帧一次降频）。
      // 聚合折叠视图下由聚合节点表达社区，跳过气泡避免视觉重叠
      // 规模保护：社区数失控（粒度极细至万级）时跳过气泡，避免为每个"社区"绘制
      // 上万 radial-gradient 气泡 + 标签 → 主线程每 5 帧一次全量绘制仍会卡死。
      // D5: 不全折叠时才画气泡——折叠社区由聚类标记表达，不重复绘制气泡。
      // 因此条件从 collapsedRef.current.size === 0（全局）改为 < communities.size（逐个社区判断）。
      //
      // ⚠ 判据修正（2026-09-15）：原判据 `communities.size <= MAX_AGG_PHYS_NODES` 取错了量 ——
      // communities 是 Map<nodeId, cid>，其 size 是**节点条目数**（本数据集 24288），
      // 不是**不同社区 id 的个数**（200 个虚拟桶）；把它与 MAX_AGG_PHYS_NODES(800)
      // 这个「聚合物理规模上限」比较 ⇒ 在本数据集上恒为 false ⇒ 气泡层与「分组」表达**永久不可达**。
      // 这是「判据量取错」而非「阈值设小」，调阈值无用。
      // 正确判据 = 不同社区 id 的个数，该值即 communityCentroidsRef.size
      // （由 refreshClusterGeom 基于 effectiveCommunitiesRef 回填，与聚合物理**同源**，
      //  故气泡位置与节点实际聚集位置天然对齐）。
      const clusterCount = communityCentroidsRef.current.size;
      if (
        clusterModeRef.current && clusterCount > 0
        && clusterCount <= MAX_AGG_PHYS_NODES
        && frameCounterRef.current % 5 === 0
        && collapsedRef.current.size < clusterCount
      ) {
        drawClusterRegions(ctx, nodes);
      }

      // 聚合几何已在 Worker ready 回调和 LOD 切换时异步计算
      // 渲染循环中不再同步调用 refreshClusterGeom()，避免 O(N) 阻塞主线程
      // 聚合物理激活时由聚合物理节点回写驱动，非激活时使用上次计算结果
      const forceCluster = nodes.length > AUTO_CLUSTER_THRESHOLD;

      // 绘制（传入视口范围用于裁剪）
      // Worker 未就绪的大图：跳过完整渲染，避免主线程 fallback 卡死
      if (shouldRender && !workerNotReadyLargeGraph) {
        // 强制聚类：节点数 > 3000 时自动进入聚类渲染模式
        // 但如果 clusterGeom 还没准备好，强制走原始渲染路径
        const geomReady = clusterGeomRef.current.size > 0;
        const shouldUseClusterRender = (aggActive || clusterModeRef.current || forceCluster) && geomReady;

        // 位图节点层的「阻塞」判据 —— **拆成两个，因为两条消费路径的语义不同**。
        //
        // 拖动：**两条路径都必须让位**。拖动中的节点坐标每帧都在变而位图是快照 ⇒ 拖影；
        //   且拖动期聚合布局未停（`spriteNeedRebuild` 要求 `settle.settled`）⇒ 位图不会重建。
        const spriteBlockedByDrag = !!dragRef.current;
        // hover / 选中：**只有 `drawNodesOptimized` 那条路径才有节点级高亮** ——
        //   ×1.5 放大 / 邻域 ×1.1 / 其余 alpha 0.15 / 涟漪圈，全在它内部（`ripplePhase`，
        //   全文件仅此一处）；而 `drawExpandedCommunity` 的节点层是**统一 `globalAlpha=0.85`
        //   的圆**，一个 hover/selected 分支都没有（它的交互反馈走 DOM tooltip + 边高亮，
        //   而边**始终**是矢量、不受位图影响）。
        //   ⇒ 在 `drawExpandedCommunity` 那里让位是「本帧重建 O(N) 个矢量圆」换**零视觉差异**。
        //   实测（2026-09-17 v4 探针）：hover 命中节点的那 33 帧 arc/帧 = 12309，
        //   未命中的 111 帧 arc/帧 = 195（= minimap 的 200）⇒ **63×**，而画面完全相同。
        //   ⚠ 用户实际看到的 fit 全图态恒走 `drawExpandedCommunity` ⇒ 这 63× 全部发生在
        //     他最常看的那个状态里。
        //
        // ⚠ `!!hovered` 的双重否定是必须的：`hovered` 是 `string | null`（节点 id），
        //   少了它整个表达式退化成 `string | boolean`，传进声明为 `boolean` 的判据会
        //   被 tsc 拦下（实测 TS2345 × 5）。别为了「看起来简洁」删掉。
        const spriteBlockedByHighlight = !!hovered || !!selected;
        const spriteBlocked = spriteBlockedByDrag || spriteBlockedByHighlight;

        // 关键诊断日志：每 60 帧输出一次渲染路径状态
        // ⚠ 2026-09-17 增补 sprite 三字段：位图这条链的「构建侧 / 消费侧」一直是**分立**的
        //   判据（也正是矛盾所在），只报 `spriteCache` 的尺寸无法回答「它到底生效了没有」。
        //   `spriteInUse` 是本帧的真实消费判据，`spriteDrift` 是构建质量（是否过期）。
        if (frameCounterRef.current % 60 === 0) {
          const camInfo = cameraRef.current;
          const vpW = camInfo.zoom > 0 ? w / camInfo.zoom : 0;
          const vpH = camInfo.zoom > 0 ? h / camInfo.zoom : 0;
          debugLog("[GraphView] render path", {
            forceCluster,
            aggActive,
            clusterMode: clusterModeRef.current,
            autoForce: isAutoForceClusterRef.current,
            geomReady,
            shouldUseClusterRender,
            nodes: nodes.length,
            posMapSize: posMapRef.current.size,
            gridIndexCells: gridIndexRef.current?.size ?? 0,
            collapsedSize: collapsedRef.current.size,
            zoom: camInfo.zoom.toFixed(2),
            viewport: { w: vpW.toFixed(0), h: vpH.toFixed(0) },
            spriteCache: spriteCacheRef.current
              ? `${spriteCacheRef.current.width}x${spriteCacheRef.current.height}`
              : null,
            spriteDriftPx: Number(spriteDriftPx(camInfo.zoom).toFixed(2)),
            // `spriteInUse` 取**生产路径**（`drawExpandedCommunity`）的判据 ⇒ 只有拖动会挡它。
            // 这正是「位图在这一帧到底替没替掉节点层」的答案；另两个字段是它的两个分量。
            spriteInUse: spriteNodeLayerReady(nodes.length, camInfo.zoom, spriteBlockedByDrag),
            spriteBlockedByDrag,
            spriteBlockedByHighlight,
            // 退火进度（2026-09-17）：`annealSteps` 是「物理步进是不是真的在按
            // 60 步/秒推进」的**直接证据**。改前物理步频 = fps/6（实测 ≈1.8 步/秒），
            // 而该指标在 45 秒预热后应当 ≈2700；若显著偏小即说明又回到了按帧步进。
            aggAnnealSteps: aggAnnealRef.current.steps,
            aggAnnealScale: Number(aggAnnealRef.current.scale.toFixed(6)),
            aggSettled: aggSettleRef.current.settled,
          });
        }

        // 主渲染路径本帧绘制的节点数：
        // -1 = 已通过其他方式绘制（聚类标记 / 位图 / 矢量 fallback），安全阀无需介入
        // >=0 = drawExpandedCommunity 实际绘制的节点数，为 0 时安全阀兜底
        let expandedNodesDrawn = -1;

        if (shouldUseClusterRender) {
          // ── 聚类模式：极简渲染策略 ──
          // 全折叠时只画小型聚类标记 + 聚合边
          // 展开社区时才画内部节点
          const activeCommunities = effectiveCommunitiesRef.current ?? communities;
          const totalCommunities = activeCommunities ? new Set(activeCommunities.values()).size : 0;
          // forceCluster 模式下：只有用户手动触发时才强制全折叠
          // 自动 force cluster（isAutoForceClusterRef=true）时保持部分展开，让用户看到真实节点
          //
          // ⚠ 判据修正（2026-09-16，修复「手动关掉聚类模式 ⇒ 主画布完全空白」）：
          // 原第一分支 `(forceCluster && !isAutoForceClusterRef.current)` 拿「用户在大图上
          // 手动切换过聚类模式」这个**推断**去代表「社区已全部折叠」这个**事实**。但点一下
          // ◈ 只把该标志置 false（见 :5050 附近的 onClick），**不会**填充 collapsed
          // ⇒ 进入「全折叠」分支，而分支内逐个 `if (!collapsed.has(cid)) continue`
          // ⇒ 一个聚类标记都画不出（drawnClusters=0）；此时 aggPhys 又为 null（用户关聚类时
          // 被显式置空）⇒ 0 条聚合边；而 expandedNodesDrawn 恒为 -1 ⇒ 安全阀也不介入
          // ⇒ **整屏空白**。四环全部有代码证据。
          // 修复：钉在真实状态上 —— 只有「确实有社区被折叠、且折叠覆盖了全部社区」
          // 才走全折叠视图；否则一律走展开视图（没折叠，本来就该看到真实节点）。
          const allCollapsed = totalCommunities > 0
            && collapsedRef.current.size > 0
            && collapsedRef.current.size >= totalCommunities;

          if (allCollapsed) {
            // ── 全折叠：只画聚类标记（最大15px）+ 聚合边 ──
            const geom = clusterGeomRef.current;
            const aggPhysLocal = aggPhysRef.current;
            // 视口信息（用于调试日志）
            const camLocal = cameraRef.current;
            const zoomLocal = camLocal.zoom;
            const viewW = zoomLocal > 0 ? w / zoomLocal : 0;
            const viewH = zoomLocal > 0 ? h / zoomLocal : 0;
            const vx0Local = -camLocal.x / zoomLocal - viewW / 2;
            const vy0Local = -camLocal.y / zoomLocal - viewH / 2;
            const vx1Local = -camLocal.x / zoomLocal + viewW / 2;
            const vy1Local = -camLocal.y / zoomLocal + viewH / 2;
            if (frameCounterRef.current % 60 === 0) {
              debugLog("[GraphView] forceCluster render state", {
                forceCluster,
                aggActive,
                clusterMode: clusterModeRef.current,
                totalCommunities,
                allCollapsed,
                geomSize: geom.size,
                aggPhysNull: aggPhysLocal === null,
                aggPhysNodes: aggPhysLocal?.nodes.length ?? 0,
                aggPhysEdges: aggPhysLocal?.edges.length ?? 0,
                collapsedSize: collapsedRef.current.size,
                camera: { x: camLocal.x.toFixed(0), y: camLocal.y.toFixed(0), zoom: zoomLocal.toFixed(2) },
                viewport: {
                  x0: vx0Local.toFixed(0),
                  y0: vy0Local.toFixed(0),
                  x1: vx1Local.toFixed(0),
                  y1: vy1Local.toFixed(0),
                },
              });
            }
            if (geom.size > 0) {
              // Obsidian 风格聚合边：细线条、柔和透明度、动态宽度
              const zoom = cameraRef.current.zoom;
              // 屏幕恒定 1px（低缩放略细，避免密集区糊成一片）：世界坐标 = 1/zoom
              const aggDynamicWidth = worldEdgeWidth(zoom < 0.3 ? 0.8 : 1.0, zoom);
              const aggAlpha = zoom < 0.3 ? 0.12 : zoom < 0.5 ? 0.2 : 0.3;

              if (aggPhysLocal && aggPhysLocal.edges.length > 0) {
                ctx.save();
                ctx.strokeStyle = token.colorBorder;
                ctx.lineWidth = aggDynamicWidth;
                ctx.globalAlpha = aggAlpha;
                const aggBatchPaths = new Map<string, Path2D>();
                const aggSampleRate = zoom < 0.3 ? 0.3 : zoom < 0.5 ? 0.6 : 1.0;

                for (let i = 0; i < aggPhysLocal.edges.length; i++) {
                  const e = aggPhysLocal.edges[i];
                  const sNode = aggPhysLocal.nodes[e.sourceIdx];
                  const tNode = aggPhysLocal.nodes[e.targetIdx];
                  if (!sNode || !tNode) { continue; }
                  if (
                    !isInView(sNode.x, sNode.y, viewWorld, 30) || !isInView(tNode.x, tNode.y, viewWorld, 30)
                  ) { continue; }
                  // 确定性降采样：N4 修复——用 source+target 稳定散列替代索引等差
                  // （(i * 77777) % 1000 与边序号线性相关，低采样率时保留边呈周期条纹）
                  if (aggSampleRate < 1.0) {
                    const hash = (Math.abs(hashStringToInt(e.source + e.target)) % 1000) / 1000;
                    if (hash > aggSampleRate) { continue; }
                  }
                  // 聚合边统一用一种颜色和宽度
                  let path = aggBatchPaths.get("default");
                  if (!path) {
                    path = new Path2D();
                    aggBatchPaths.set("default", path);
                  }
                  path.moveTo(sNode.x, sNode.y);
                  path.lineTo(tNode.x, tNode.y);
                }
                for (const path of aggBatchPaths.values()) {
                  ctx.stroke(path);
                }
                ctx.globalAlpha = 1;
                ctx.restore();
              }

              // 聚类标记（小圆形，最大15px）
              ctx.save();
              let drawnClusters = 0;
              let skippedClusters = 0;
              for (const [cid, g] of geom) {
                if (!collapsedRef.current.has(cid)) {
                  skippedClusters++;
                  if (skippedClusters <= 3) {
                    debugLog("[GraphView] cluster skipped (not collapsed)", { cid });
                  }
                  continue;
                }
                if (!isInView(g.cx, g.cy, viewWorld, 30)) { continue; }
                const color = communityPalette[cid % communityPalette.length];
                // 半径必须含屏幕像素下限：`Math.min(15, g.r)` 是**世界坐标**上限，
                // 而低 zoom 下 15 世界单位在屏幕上只有 0.7px —— 实测 C 阶段（显式全折叠）
                // 200 个标记的屏幕半径全部落在 0.5-0.8px，整屏 chroma>30 仅 0.046%，
                // 肉眼即"空白"。nodeDrawRadius 只在亚像素时才放大，zoom 大时无影响。
                const maxR = nodeDrawRadius(Math.min(15, g.r), cam.zoom);
                // 主体
                ctx.globalAlpha = 0.85;
                ctx.beginPath();
                ctx.arc(g.cx, g.cy, maxR, 0, Math.PI * 2);
                ctx.fillStyle = color;
                ctx.fill();
                drawnClusters++;
                // 标签：D4 修复——阈值从 0.8 降至 0.3，总览低 zoom 下聚合彩球也有标注。
                // 字号随缩放动态调整（世界坐标保持约 12px，通过 zoom 换算），配合 measureText 截断。
                if (cam.zoom >= 0.3) {
                  const fontSize = 12 / cam.zoom;
                  ctx.globalAlpha = 0.9;
                  ctx.font = `${fontSize.toFixed(1)}px Inter, system-ui, sans-serif`;
                  ctx.textAlign = "center";
                  ctx.textBaseline = "top";
                  ctx.fillStyle = token.colorText;
                  const label = `${g.label} (${g.count})`;
                  // 限制标签最大宽度，超过则截断
                  const maxLabelWidth = 80 / cam.zoom;
                  let displayLabel = label;
                  const metrics = ctx.measureText(label);
                  if (metrics.width > maxLabelWidth) {
                    const ellipsis = "…";
                    let w = ctx.measureText(ellipsis).width;
                    let i = 0;
                    while (w < maxLabelWidth && i < label.length) {
                      i++;
                      w = ctx.measureText(label.slice(0, i) + ellipsis).width;
                    }
                    displayLabel = label.slice(0, i) + ellipsis;
                  }
                  ctx.fillText(displayLabel, g.cx, g.cy + maxR + fontSize);
                }
              }
              // 本帧一个聚类标记都没画出来 ⇒ 主动交还给终极安全阀兜底。
              // （此前该分支不改 expandedNodesDrawn，其值恒为 -1，而 -1 的语义是
              //  「已通过其他方式绘制、无需介入」⇒ 安全阀被误导为「已画好」⇒ 整屏空白无人兜。）
              if (drawnClusters === 0) { expandedNodesDrawn = 0; }
              if (frameCounterRef.current % 60 === 0) {
                debugLog("[GraphView] cluster draw stats", {
                  totalGeom: geom.size,
                  drawn: drawnClusters,
                  skipped: skippedClusters,
                  collapsedSize: collapsedRef.current.size,
                });
              }
              ctx.restore();
            } else {
              // ── Fallback：聚合几何尚未就绪（Worker ready 回调延迟计算），
              // 退回非聚类渲染路径，确保节点和边始终可见。
              // 临时清除 collapsed 集，避免 drawNodesOptimized/drawEdgesOptimized
              // 因 clusterModeRef.current 为 true 而跳过折叠社区的节点/边。 ──
              const prevCollapsed = collapsedRef.current;
              collapsedRef.current = new Set();
              // 边与粒子**始终**走矢量路径（位图不含边：它承载不了边类型 / 社区 /
              // 高亮三层筛选语义，见 buildBigGraphSpriteCache 的注释）。位图只替节点层。
              drawEdgesOptimized(ctx, nodes, fisheye, viewWorld);
              drawParticlesOptimized(ctx, nodes, fisheye, viewWorld);
              if (spriteNodeLayerReady(nodes.length, cam.zoom, spriteBlocked)) {
                drawSpriteNodeLayer(ctx);
              } else {
                drawNodesOptimized(ctx, nodes, fisheye, viewWorld);
              }
              collapsedRef.current = prevCollapsed;
            }
          } else {
            // ── 部分展开：绘制展开社区的节点和边 ──
            if (activeCommunities) {
              // 位图节点层（2026-09-17 启用）：这是**用户实际看到的那个状态** ——
              // 大图恒满足 `shouldUseClusterRender`，而 `activeCommunities` 恒非 null
              // （`effectiveCommunitiesRef.current ?? communities`）⇒ 改前位图的三处
              // 消费点一个都到不了，位图「被构建却永远画不出来」。
              // 语义等价性：位图烘的是「按同一 collapsed 集过滤后的全量节点」
              // （`clusterActive` 时跳过折叠社区的节点），与 drawExpandedCommunity 的
              // `isNodeVisible` 判据**同源**；且位图**只含节点**，边/标签/气泡/聚类标记
              // 仍由该函数内部照旧绘制。
              const useSpriteNodes = spriteNodeLayerReady(
                nodes.length,
                cam.zoom,
                // ⚠ 这里**只**用拖动作判据，不含 hover/选中：本函数的节点层没有
                //   节点级高亮（见 `spriteBlockedByHighlight` 的注释），让位纯属白付 O(N)。
                spriteBlockedByDrag,
              );
              if (useSpriteNodes) { drawSpriteNodeLayer(ctx); }
              // N1 修复：接收实际绘制节点数，供下方安全阀判断（此前返回值被丢弃，
              // expandedNodesDrawn 恒为 -1，兜底条件永不触发）
              expandedNodesDrawn = drawExpandedCommunity(
                ctx,
                nodes,
                viewWorld,
                activeCommunities,
                cam.zoom,
                useSpriteNodes,
              );
            } else {
              // ── Fallback：无社区数据时退回非聚类渲染路径。
              // 临时清除 collapsed 集，避免因 clusterModeRef.current 为 true
              // 而跳过折叠社区的节点/边。 ──
              const prevCollapsed2 = collapsedRef.current;
              collapsedRef.current = new Set();
              // 边与粒子**始终**走矢量路径（位图不含边，见 buildBigGraphSpriteCache）。
              drawEdgesOptimized(ctx, nodes, fisheye, viewWorld);
              drawParticlesOptimized(ctx, nodes, fisheye, viewWorld);
              if (spriteNodeLayerReady(nodes.length, cam.zoom, spriteBlocked)) {
                drawSpriteNodeLayer(ctx);
              } else {
                drawNodesOptimized(ctx, nodes, fisheye, viewWorld);
              }
              collapsedRef.current = prevCollapsed2;
            }
          }
        } else {
          // ── 非聚类模式：使用原始渲染路径 ──
          // 边与粒子**始终**走矢量路径（位图不含边，见 buildBigGraphSpriteCache）；
          // 位图只替节点层，且由 `spriteNodeLayerReady` 单点判据决定（与构建侧共用）。
          drawEdgesOptimized(ctx, nodes, fisheye, viewWorld);
          drawParticlesOptimized(ctx, nodes, fisheye, viewWorld);
          if (spriteNodeLayerReady(nodes.length, cam.zoom, spriteBlocked)) {
            drawSpriteNodeLayer(ctx);
          } else {
            drawNodesOptimized(ctx, nodes, fisheye, viewWorld);
          }
        }

        // ── 终极安全阀：主渲染路径本帧没画出任何节点时兜底 ──
        // 仅在主渲染路径（drawExpandedCommunity / 全折叠标记）本帧实际绘制 0 个节点时兜底，
        // 防止社区过滤/聚类逻辑导致节点不可见；正常帧不再叠加绘制，
        // 避免双重绘制导致的亮度失真与 hover/selected 高亮被覆盖。
        // ⚠ 条件放宽（2026-09-16）：原条件还要求 `isAutoForceClusterRef.current`，
        // 而用户一点 ◈ 就把它置 false ⇒ **恰恰在导致整屏空白的那个状态下安全阀失效**，
        // 空得毫无提示。安全阀的语义是「本帧一个节点都没画出来」，与模式标志无关。
        if (nodes.length > 0 && expandedNodesDrawn === 0) {
          const maxDraw = 3000;
          let drawn = 0;
          ctx.save();
          ctx.globalAlpha = 0.85;
          for (const node of nodes) {
            if (drawn >= maxDraw) { break; }
            if (!isInView(node.x, node.y, viewWorld, 30)) { continue; }
            const color = nodeColorRef.current.get(node.id) || token.colorPrimary;
            const size = (nodeSizeRef.current.get(node.id) || 5) * 1.0;
            ctx.fillStyle = color;
            ctx.beginPath();
            // 安全阀自身也必须含屏幕像素下限 —— 它的存在目的就是「防止节点不可见」，
            // 若它画出来的同样是亚像素点，兜底等于失效（同根因族，判据 #301）。
            ctx.arc(node.x, node.y, nodeDrawRadius(size, cameraRef.current.zoom), 0, Math.PI * 2);
            ctx.fill();
            drawn++;
          }
          ctx.restore();
          if (frameCounterRef.current % 60 === 0) {
            debugLog("[GraphView] safety net nodes drawn", { drawn });
          }
        }
        // 本帧已完成「完整绘制」（节点/社区气泡/位图/安全阀都在上方本块内）——
        // 从此刻起才允许空闲跳帧：`hasPaintedFrameRef` 是跳帧的前置条件。
        hasPaintedFrameRef.current = true;
      }

      ctx.restore();

      // 同步 tooltip DOM 位置（每帧更新，不走 React）
      if (tooltipRef.current) {
        if (tooltipVisibleRef.current && tooltipNodeIdRef.current) {
          tooltipRef.current.style.left = `${tooltipPosRef.current.x}px`;
          tooltipRef.current.style.top = `${tooltipPosRef.current.y}px`;
          tooltipRef.current.style.display = "block";
        } else {
          tooltipRef.current.style.display = "none";
        }
      }

      // N6 修复：统计弹窗 Zoom 值实时刷新（每 15 帧直接写 DOM，不走 React 重渲染）
      if (statsZoomTextRef.current && frameCounterRef.current % 15 === 0) {
        statsZoomTextRef.current.textContent = `${cameraRef.current.zoom.toFixed(2)}×`;
      }

      if (showMinimap && minimapOpen && minimapRef.current && frameCounterRef.current % MINIMAP_REDRAW_INTERVAL === 0) {
        const mmCanvas = minimapRef.current;
        const mmCtx = mmCanvas.getContext("2d");
        if (mmCtx) {
          drawMinimap(mmCtx, nodes);
        }
      }

      rafRef.current = requestAnimationFrame(render);
    };

    rafRef.current = requestAnimationFrame(render);
    return () => {
      running = false;
      cancelAnimationFrame(rafRef.current);
    };
    // communities 异步加载后会变化：加入依赖使渲染循环闭包拿到最新值，
    // 否则聚类气泡/社区筛选/聚合折叠全部读不到社区数据（stale closure）
  }, [dimensions, token, communities]);

  function getScreenToWorld(sx: number, sy: number): { x: number; y: number } {
    const cam = cameraRef.current;
    const w = dimensions.width;
    const h = dimensions.height;
    return {
      x: (sx - w / 2 - cam.x) / cam.zoom,
      y: (sy - h / 2 - cam.y) / cam.zoom,
    };
  }

  // ── 鱼眼放大镜 ──
  // 鼠标位置附近的节点会被放大，形成类似 Obsidian 的局部放大效果
  const FISHEYE_RADIUS = 180; // 世界坐标下的影响半径
  const FISHEYE_STRENGTH = 0.45; // 放大强度 (0~1)

  interface FisheyeState {
    active: boolean;
    worldX: number;
    worldY: number;
    radius: number;
    strength: number;
  }

  function computeFisheye(): FisheyeState {
    const m = mouseScreenRef.current;
    if (!fisheyeEnabledRef.current || !m.active) {
      return { active: false, worldX: 0, worldY: 0, radius: FISHEYE_RADIUS, strength: 0 };
    }
    const world = getScreenToWorld(m.x, m.y);
    return {
      active: true,
      worldX: world.x,
      worldY: world.y,
      radius: FISHEYE_RADIUS / cameraRef.current.zoom,
      strength: FISHEYE_STRENGTH,
    };
  }

  // 根据鱼眼计算节点的缩放倍率
  function fisheyeScale(nodeX: number, nodeY: number, fisheye: FisheyeState): number {
    if (!fisheye.active) { return 1; }
    const dx = nodeX - fisheye.worldX;
    const dy = nodeY - fisheye.worldY;
    const dist = Math.sqrt(dx * dx + dy * dy);
    if (dist > fisheye.radius) { return 1; }
    // 平滑衰减：距离越近放大越多
    const t = 1 - dist / fisheye.radius;
    return 1 + fisheye.strength * t * t * (3 - 2 * t); // smoothstep
  }

  // ── 社区聚类区域渲染 ──
  // D1: 质心数据在 refreshClusterGeom 中与 clusterGeom 同步回填，Worker 主路径下不再为空。
  // D5: 部分折叠时仅折叠社区绘制气泡（展开社区显示真实节点、不画），全折叠时由聚类标记表达、不画。
  // P9: 稳定态缓存——节点位置/折叠集合变化或每 30 帧才重建分组与渐变，静止时直接复用。
  function drawClusterRegions(ctx: CanvasRenderingContext2D, nodes: PhysicsNode[]) {
    const activeCommunities = effectiveCommunitiesRef.current ?? communities;
    if (!activeCommunities) { return; }
    const centroids = communityCentroidsRef.current;
    if (centroids.size === 0) { return; }
    const collapsed = collapsedRef.current;
    const cache = clusterRegionCacheRef.current;

    // 折叠集合变化（LOD / 手动切换）→ 强制重建
    if (collapsed !== cache.lastCollapsed) {
      cache.lastCollapsed = collapsed;
      cache.dirty = true;
    }

    // 全折叠：由聚类标记（彩球）+ 聚合边表达社区，跳过气泡避免视觉重叠
    if (collapsed.size >= centroids.size) { return; }

    // 稳定态判定：非脏且距上次重建不足 30 帧 → 直接复用缓存绘制
    const needsRebuild = cache.dirty || frameCounterRef.current - cache.lastFrame >= 30;
    if (needsRebuild) {
      cache.dirty = false;
      cache.lastFrame = frameCounterRef.current;
      cache.regions.clear();

      // 按社区分组收集节点位置（D5: 部分折叠时展开社区不画气泡，直接跳过）
      const communityNodes = new Map<number, { sx: number; sy: number }[]>();
      for (const node of nodes) {
        const cid = activeCommunities.get(node.id);
        if (cid === undefined) { continue; }
        if (collapsed.size > 0 && !collapsed.has(cid)) { continue; }
        const list = communityNodes.get(cid) ?? [];
        list.push({ sx: node.x, sy: node.y });
        communityNodes.set(cid, list);
      }

      // 为每个社区计算包围盒 + radialGradient，写入缓存
      for (const [cid, points] of communityNodes) {
        if (points.length < 2) { continue; }
        const color = communityPalette[cid % communityPalette.length];

        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const p of points) {
          if (p.sx < minX) { minX = p.sx; }
          if (p.sy < minY) { minY = p.sy; }
          if (p.sx > maxX) { maxX = p.sx; }
          if (p.sy > maxY) { maxY = p.sy; }
        }
        const cx = (minX + maxX) / 2;
        const cy = (minY + maxY) / 2;
        // ⚠ 半径**不再**按包围盒算（2026-09-16 修复）。
        // 原实现 `(maxX-minX)/2 + 40`：在「社区成员尚未被布局分离」时，每个社区的包围盒
        // ≈ 整张画布 ⇒ 200 个半径 ≈0.78×画布对角线的极淡椭圆完全重叠 ⇒ 视觉产物只剩
        // 「一片均匀染色」，气泡层形同不存在（实测 p50/p95 = 0.778/0.922，判据 #287/#298）。
        // 改用与「成员散布半径」同源的公式 ⇒ 气泡恒为「刚好包住这一团节点」的大小。
        const rr = communityRadius(points.length) * COMMUNITY_BUBBLE_RADIUS_SCALE;
        const rx = rr;
        const ry = rr;

        const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, Math.max(rx, ry));
        grad.addColorStop(0, hexToRgba(color, 0.12));
        grad.addColorStop(0.6, hexToRgba(color, 0.06));
        grad.addColorStop(1, hexToRgba(color, 0));
        cache.regions.set(cid, { cx, cy, rx, ry, grad });
      }
    }

    // 用缓存绘制气泡 + 标签
    for (const [cid, region] of cache.regions) {
      ctx.save();
      ctx.fillStyle = region.grad;
      ctx.beginPath();
      ctx.ellipse(region.cx, region.cy, region.rx, region.ry, 0, 0, Math.PI * 2);
      ctx.fill();

      // 社区标签
      const centroid = centroids.get(cid);
      if (centroid && centroid.count >= 2) {
        ctx.globalAlpha = 0.5;
        // N3 修复：字号处于世界坐标系，除以 zoom 保证任何缩放级别下屏幕字号恒定（11px）
        const labelFontSize = 11 / (cameraRef.current.zoom || 1);
        ctx.font = `bold ${labelFontSize.toFixed(1)}px Inter, system-ui, sans-serif`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillStyle = communityPalette[cid % communityPalette.length];
        ctx.fillText(
          t("wiki.graph.clusterLabel", { id: cid }) + ` · ${centroid.count}`,
          region.cx,
          region.cy - region.ry + 14,
        );
      }
      ctx.restore();
    }
  }

  // ── 优化绘制函数：带视口裁剪，跳过屏幕外元素 ──

  // 构建大图位图缓存：将所有节点/边预渲染到离屏 Canvas
  // 万级节点下每帧 5 万+ 矢量操作是卡死根因，位图模式将其降为 1 次 drawImage
  function buildBigGraphSpriteCache(nodes: PhysicsNode[]): HTMLCanvasElement | null {
    if (nodes.length === 0) { return null; }

    // 计算节点分布 bounding box —— 聚类模式下只计算可见节点
    // 但 auto force cluster 模式下必须包含所有节点
    const clusterActive = clusterModeRef.current && !isAutoForceClusterRef.current;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    let hasVisible = false;
    for (const n of nodes) {
      if (clusterActive) {
        const cid = getCommunityId(n.id);
        if (cid !== undefined && collapsedRef.current.has(cid)) { continue; }
      }
      if (n.x < minX) { minX = n.x; }
      if (n.y < minY) { minY = n.y; }
      if (n.x > maxX) { maxX = n.x; }
      if (n.y > maxY) { maxY = n.y; }
      hasVisible = true;
    }
    // 如果所有节点都被折叠，使用全量范围
    if (!hasVisible) {
      minX = -500;
      minY = -500;
      maxX = 500;
      maxY = 500;
    }

    // Padding 覆盖整个可视范围 —— ⚠ 必须**与布局尺度成比例**，不能是固定世界单位。
    // 理由：位图的可用性判据（`spriteNodeLayerReady`）只看 zoom，**不判视口是否越界**，
    // 所以 padding 就是「用户能平移多远而不出图」的全部余量。旧值 800 是按当时的平衡
    // 跨度 2100 定的（≈ span×0.38 ⇒ fit 态屏幕余量 800×0.25 = 200px）；2026-09-17 ③-D
    // 把平衡跨度抬到 ~5400（repulsion 600→5400）后，固定 800 只剩 80px 屏幕余量
    // ⇒ 一平移位图就出界、外侧节点变空白。取 span×0.38 保持屏幕余量不变。
    // 下限 300 防小图 padding 过小（小图本来也不走位图路径，此处只求不荒谬）。
    const rawSpan = Math.max(maxX - minX, maxY - minY);
    const padding = Math.max(300, rawSpan * 0.38);
    minX -= padding;
    minY -= padding;
    maxX += padding;
    maxY += padding;
    spriteWorldBBoxRef.current = { minX, minY, maxX, maxY };

    // 记录新鲜度锚点（见 `spriteAnchorRef` 的注释）：抽样固定步长，烘制时与消费时
    // 用**同一批 id** ⇒ 偏差可比。抽满 256 个点即止（`Math.max(1, ...)` 防 N < 256 时步长 0）。
    {
      const step = Math.max(1, Math.floor(nodes.length / 256));
      const ids: string[] = [];
      const xs: number[] = [];
      const ys: number[] = [];
      for (let i = 0; i < nodes.length && ids.length < 256; i += step) {
        ids.push(nodes[i].id);
        xs.push(nodes[i].x);
        ys.push(nodes[i].y);
      }
      spriteAnchorRef.current = { ids, xs, ys };
    }

    const worldW = maxX - minX;
    const worldH = maxY - minY;

    // 限制离屏 Canvas 最大尺寸，防止内存溢出
    // L1 修复：原逻辑只限单边 16384，不限面积——大世界下 16384²×4 ≈ 1GB RGBA，
    // 浏览器可能分配失败导致位图模式黑屏。增加面积上限 4096²（≈64MB RGBA），
    // 取三个约束的最小缩放比。
    const MAX_CANVAS = 16384;
    const MAX_SPRITE_AREA = 4096 * 4096;
    let scale = Math.min(
      1,
      MAX_CANVAS / Math.max(worldW, worldH),
      Math.sqrt(MAX_SPRITE_AREA / Math.max(1, worldW * worldH)),
    );
    // 保底分辨率：极端分散布局下 sprite 最大边不低于 512px（保底后面积 ≤512²，仍在面积上限内）
    scale = Math.max(scale, 512 / Math.max(worldW, worldH));
    const cw = Math.max(1, Math.ceil(worldW * scale));
    const ch = Math.max(1, Math.ceil(worldH * scale));

    const oc = document.createElement("canvas");
    oc.width = cw;
    oc.height = ch;
    const octx = oc.getContext("2d")!;

    // 世界坐标 → 离屏坐标变换
    octx.save();
    octx.scale(scale, scale);
    octx.translate(-minX, -minY);

    // ⚠ 烘制内容**只有节点，没有边**（2026-09-17 启用位图时删掉了边那一支）。
    // 为什么必须去掉：边在矢量路径上有四层「这个渲染路径给不了」的语义 ——
    // `visibleEdgeTypesRef`（用户关掉的边类型）、`visibleCommunitiesRef`（社区筛选）、
    // hover/selected 高亮加宽、以及与节点采样解耦的端点判据。位图是一张**无条件
    // 全画**的栅格，把它当边用等于让「已关闭的边类型重新出现」「高亮边不加宽」。
    // 改前这段一直在（且从未被消费过，所以缺陷没暴露）；启用位图之前必须先删掉，
    // 否则「启用」就是把一个隐藏缺陷变成可见缺陷。
    // 代价可忽略：边的绘制本来就是 Path2D 批量 stroke（视口裁剪 + 采样后每帧 1 次
    // `stroke`），性能黑洞不在这里，在那约 1.2 万次 `arc`。
    const nodeColors = nodeColorRef.current;

    // 批量绘制节点（按颜色 + 半径合并）
    // ⚠ 半径走 `nodeDrawRadius(size, SPRITE_BAKE_ZOOM)`，不再是旧的 `size * 1.2`：
    //   前者把**屏幕下限**按烘制参考 zoom 换算成世界半径烘进来（见 graphViewUtils 的
    //   `SPRITE_BAKE_ZOOM`）—— 这正是「位图在 fit 态不再糊成灰雾」的机制，也是
    //   `spriteUsableAtZoom` 的阈值能从「单边 0.278」放开成区间的依据。
    //   副作用是所有小节点归一到同一个半径 ⇒ 分桶数骤降 ⇒ 烘制更快。
    const nodeBatches = new Map<string, Path2D>();
    const nodeSizes = nodeSizeRef.current;
    for (const n of nodes) {
      if (clusterActive) {
        const ncid = getCommunityId(n.id);
        if (ncid !== undefined && collapsedRef.current.has(ncid)) { continue; }
      }
      const color = nodeColors.get(n.id) || token.colorPrimary;
      const r = nodeDrawRadius(nodeSizes.get(n.id) || 6, SPRITE_BAKE_ZOOM);
      const key = `${color}|${r.toFixed(1)}`;
      if (!nodeBatches.has(key)) { nodeBatches.set(key, new Path2D()); }
      const p = nodeBatches.get(key)!;
      // 用 arc 添加到 Path2D
      p.moveTo(n.x + r, n.y);
      p.arc(n.x, n.y, r, 0, Math.PI * 2);
    }
    for (const [key, path] of nodeBatches) {
      const [color] = key.split("|");
      octx.fillStyle = color;
      octx.fill(path);
    }

    octx.restore();
    return oc;
  }

  // ── 位图消费的两个 helper（2026-09-17 抽出）──
  //
  // 为什么必须抽成**单点判据**：改前构建侧判据（节点数 + zoom，见 worker 回调里的
  // 重建、以及 applyAggregateLayout 里的重建）与消费侧可达域（三条全是「非聚类」或
  // 「聚类 fallback」的支路）**不相交** —— 真实大图恒满足 `shouldUseClusterRender`
  // 且 `activeCommunities` 恒非 null（`effectiveCommunitiesRef.current ?? communities`），
  // 于是走 `drawExpandedCommunity`，三处一个都到不了 ⇒ 位图被构建（付最多 64MB 离屏
  // 分配 + idle 里 O(N) 绘制）却**永远画不出来**（实测 5 参 `drawImage` 一次都没出现）。
  // 只要两侧都走同一个函数，「付了成本拿不到收益」在结构上就不可能再发生。

  /** 位图与当前布局的**最大走样**（屏幕像素）。锚点缺失 ⇒ `Infinity`（= 不可用）。
   *
   *  为什么不看「位图存不存在」而看这个：位图是快照，聚合物理在退火完成前每步能移动
   *  最多 `maxVelocity·dt = 7.2` 世界单位 ⇒ 一个重建周期内就能错位几千世界单位。
   *  `Infinity` 而不是 0 是**故意 fail-closed**：没有锚点 = 无法证明位图与布局一致，
   *  那就必须当它过期（否则启用位图的第一步就是贴一张错位的图）。 */
  function spriteDriftPx(zoom: number): number {
    const anchor = spriteAnchorRef.current;
    if (!anchor || anchor.ids.length === 0) { return Infinity; }
    const posMap = posMapRef.current;
    let maxD2 = 0;
    let compared = 0;
    for (let i = 0; i < anchor.ids.length; i++) {
      const cur = posMap.get(anchor.ids[i]);
      if (!cur) { continue; }
      compared++;
      const dx = cur.x - anchor.xs[i];
      const dy = cur.y - anchor.ys[i];
      const d2 = dx * dx + dy * dy;
      if (d2 > maxD2) { maxD2 = d2; }
    }
    // ⚠ 抽样缺失必须 fail-closed，不能返回 0：`maxD2` 的初值是 0，若一个锚点都查不到
    //   （节点被重建、id 全换），返回值就是 0 = 「完全没走样」= **假的「新鲜」**，
    //   于是贴上一张与当前布局无关的旧位图。要求至少比中一半抽样点才认这个结论。
    if (compared * 2 < anchor.ids.length) { return Infinity; }
    return Math.sqrt(maxD2) * zoom;
  }

  /** 位图能否用于**本帧的节点层**。`blocked` = 「有高亮或拖动」——
   *  拖动中的节点坐标每帧都在变，而位图是快照 ⇒ 用了就是拖影。 */
  function spriteNodeLayerReady(
    nodeCount: number,
    zoom: number,
    blocked: boolean,
  ): boolean {
    return nodeCount > FORCE_BITMAP_THRESHOLD
      && !blocked
      && spriteCacheRef.current !== null
      && spriteUsableAtZoom(zoom)
      && spriteDriftPx(zoom) <= SPRITE_STALE_PX;
  }

  /** 位图是否需要（重新）构建 —— **构建闸，与消费闸同源**。
   *
   *  改前构建闸是「节点数 + zoom」（见 worker 回调与 applyAggregateLayout 两处），
   *  而消费侧的可达域只有「非聚类分支」或「聚类 fallback」三条支路 —— 真实大图恒满足
   *  `shouldUseClusterRender` 且 `activeCommunities` 恒非 null ⇒ 走
   *  `drawExpandedCommunity`，三条一个都到不了 ⇒ 位图**被构建却永远画不出来**
   *  （实测 `drawImage` 参数个数集合恒为 {3}）。现在两侧都要求同一组条件
   *  （节点数 / zoom 可用区间 / 布局已停 / 走样不超阈值），
   *  「付了成本拿不到收益」在结构上不可能再发生。
   *
   *  ⚠ `layoutStable` 是**关键**、且不是「优化」：位图是快照，而聚合物理在退火完成前
   *  每步让节点移动最多 `maxVelocity·dt = 7.2` 世界单位 —— 在 `zoom = 0.2` 下只要
   *  0.5 步就走样超过 `SPRITE_STALE_PX`。也就是说**演化期根本不存在新鲜位图**：
   *  不加这个条件就会「每帧重建、每帧立刻过期」，把 64MB 离屏分配变成持续的 GC 压力
   *  （这正是改前 600 帧节流注释在担心的事）。加上之后语义变得干净：
   *  布局停下 → 构建一次 → 走样恒 ≈ 0 → 位图长期有效（布局不变就不需要重建）。
   */
  function spriteNeedRebuild(
    nodeCount: number,
    zoom: number,
    layoutStable: boolean,
  ): boolean {
    if (nodeCount <= FORCE_BITMAP_THRESHOLD) { return false; }
    if (!spriteUsableAtZoom(zoom)) { return false; }
    if (!layoutStable) { return false; }
    if (!spriteCacheRef.current) { return true; }
    return spriteDriftPx(zoom) > SPRITE_STALE_PX;
  }

  /** 把位图当作「本帧的节点层」画出来。
   *  边 / 粒子 / 标签 / 气泡 / 聚类标记**都不在位图里**，由调用方另行绘制
   *  （原因见 `buildBigGraphSpriteCache` 里「烘制内容只有节点，没有边」那段）。
   *  ⚠ 目标矩形**就是世界坐标 bbox**：本帧 ctx 已施加 `cam.zoom`，再乘一次会把位图
   *    搬到 `minX·camZ²` 处、尺寸缩到 `camZ²` 倍（zoom=0.24 时「位置错 400px、
   *    尺寸缩到 6%」，2026-09-16 已修）。 */
  function drawSpriteNodeLayer(ctx: CanvasRenderingContext2D): void {
    const sprite = spriteCacheRef.current;
    if (!sprite) { return; }
    const bbox = spriteWorldBBoxRef.current;
    ctx.drawImage(
      sprite,
      bbox.minX,
      bbox.minY,
      bbox.maxX - bbox.minX,
      bbox.maxY - bbox.minY,
    );
  }

  function isInView(
    x: number,
    y: number,
    view: { x0: number; y0: number; x1: number; y1: number },
    margin = 80,
  ): boolean {
    return x >= view.x0 - margin && x <= view.x1 + margin && y >= view.y0 - margin && y <= view.y1 + margin;
  }

  function drawExpandedCommunity(
    ctx: CanvasRenderingContext2D,
    _nodes: PhysicsNode[],
    viewWorld: { x0: number; y0: number; x1: number; y1: number },
    activeCommunities: Map<string, number>,
    // ⚠ zoom 必须由调用方传入（= 本帧 ctx.scale 实际用的那一个），不能在函数内部
    // 再读一次 cameraRef.current.zoom。实测两者会不同（同一帧内 ref 已被更新），
    // 表现为「同一批 arc 出现两种屏幕半径」：B 阶段 2/3 的节点所处的绘制变换是
    // 0.0472，而下限却按 0.0543 计算 ⇒ 屏幕半径 1.74px，漏掉了 2px 下限。
    zoom: number,
    // 节点层是否已由大图位图代劳（2026-09-17 启用位图时新增）。
    // 位图**只含节点**，所以本函数仍要画边 / 标签 / 气泡 / 聚类标记 —— 跳过的只有
    // 那一轮逐节点 `arc`（实测每帧约 1.2 万次，是该路径的绝对性能主体）。
    // 传 true 时本函数返回 -1（= 「已通过其他方式绘制」，让终极安全阀不要重复兜底）。
    nodesDrawnBySprite = false,
  ) {
    const collapsedSet = collapsedRef.current;
    const edgeMeta = edgeMetaRef.current;
    const posMap = posMapRef.current;
    const gridIndex = gridIndexRef.current;

    // 收集展开社区的节点（不在 collapsed 中的社区）
    // 关键修复：节点无社区 ID 时也应绘制，不能被跳过。
    // 只有当节点有社区 ID 且该社区被折叠时才跳过。
    const expandedNodeIds = new Set<string>();

    // 判断节点是否可见的辅助函数
    const isNodeVisible = (id: string): boolean => {
      const cid = activeCommunities.get(id);
      // 无社区 ID 的节点始终可见；有社区 ID 且社区未折叠时可见
      return cid === undefined || !collapsedSet.has(cid);
    };

    // 第一优先级：使用网格索引（O(可见区域) 效率高）
    if (gridIndex) {
      const gx0 = Math.floor(viewWorld.x0 / GRID_CELL_SIZE);
      const gy0 = Math.floor(viewWorld.y0 / GRID_CELL_SIZE);
      const gx1 = Math.floor(viewWorld.x1 / GRID_CELL_SIZE);
      const gy1 = Math.floor(viewWorld.y1 / GRID_CELL_SIZE);

      for (let gx = gx0; gx <= gx1; gx++) {
        for (let gy = gy0; gy <= gy1; gy++) {
          const bucket = gridIndex.get(`${gx},${gy}`);
          if (!bucket) { continue; }
          for (const id of bucket) {
            if (isNodeVisible(id)) {
              expandedNodeIds.add(id);
            }
          }
        }
      }
    }

    // Fallback 1：posMap 遍历（覆盖网格索引未命中的节点）
    if (expandedNodeIds.size === 0 && posMap.size > 0) {
      for (const [id, node] of posMap) {
        if (!isInView(node.x, node.y, viewWorld, 20)) { continue; }
        if (isNodeVisible(id)) {
          expandedNodeIds.add(id);
        }
      }
    }

    // Fallback 2：直接遍历 _nodes 数组（最终兜底，确保节点不会因任何过滤逻辑丢失）
    if (expandedNodeIds.size === 0 && _nodes.length > 0) {
      for (const node of _nodes) {
        if (!isInView(node.x, node.y, viewWorld, 20)) { continue; }
        if (isNodeVisible(node.id)) {
          expandedNodeIds.add(node.id);
        }
      }
    }

    // Fallback 3：终极兜底，跳过所有社区过滤，直接绘制所有视口内节点
    if (expandedNodeIds.size === 0 && _nodes.length > 0) {
      for (const node of _nodes) {
        if (!isInView(node.x, node.y, viewWorld, 20)) { continue; }
        expandedNodeIds.add(node.id);
      }
    }

    // 节点已由位图代劳 ⇒ 返回 -1（「已通过其他方式绘制」），否则安全阀会在
    // `expandedNodesDrawn === 0` 时再画一遍（双画 ⇒ 亮度失真 + 高亮被覆盖）。
    if (expandedNodeIds.size === 0) { return nodesDrawnBySprite ? -1 : 0; }

    // 降采样：**按本帧视口内候选数**（不是全图规模）决定 —— 判据见 NODE_DRAW_BUDGET。
    // 改前 `isLargeGraph ? 0.5 : 1.0` 只看全图节点数（`nodes.length > 5000`），于是
    // 放大到视口内只剩几百个候选时仍被砍一半（纯损失），且与位图区间的「无采样全量」
    // 不一致 ⇒ 跨 SPRITE_MAX_ZOOM 时点数跳变。
    const nodeSampleRate = viewportDrawRate(expandedNodeIds.size, NODE_DRAW_BUDGET);
    const visibleNodes: { id: string; x: number; y: number; size: number; color: string }[] = [];

    for (const id of expandedNodeIds) {
      // 确定性采样：使用节点 ID 的哈希，确保每帧绘制相同的节点
      if (nodeSampleRate < 1) {
        const hash = Math.abs(hashStringToInt(id));
        // 万分位而非百分位：rate 现在可以是任意比值（如 0.494），百分位比较的量化步长
        // 1% 会让极小的 rate 退化成「只留 hash%100===0 的 1%」而非「按比例留存」。
        if (hash % 10000 >= nodeSampleRate * 10000) { continue; }
      }
      const node = posMap.get(id);
      if (!node) { continue; }
      if (!isInView(node.x, node.y, viewWorld, 20)) { continue; }
      const color = nodeColorRef.current.get(id) || token.colorPrimary;
      const size = nodeSizeRef.current.get(id) || 5;
      visibleNodes.push({ id, x: node.x, y: node.y, size, color });
    }

    // 观测通道（2026-09-17）：把采样率的**调用点量纲**暴露出来 —— 否则「rate 取 1」
    // 无法与「rate 恒 0.5」在外部区分（两者都只表现为 arc 数变少/变多）。
    // 探针据此确认「放大后候选数真的降进了预算」而不是判据被改回了全图规模。
    if (frameCounterRef.current % 30 === 0) {
      debugLog("[GraphView] node draw rate", {
        candidates: expandedNodeIds.size,
        nodeRate: Number(nodeSampleRate.toFixed(4)),
        drawn: visibleNodes.length,
      });
    }

    // 绘制节点（位图节点层已代劳时整段跳过）
    // ⚠ 半径走 nodeDrawRadius：含屏幕像素下限，避免 zoom 小时整批节点亚像素化
    // （详见 graphViewUtils.nodeDrawRadius 的文档注释与 MIN_NODE_SCREEN_RADIUS）。
    // zoom ≥ 0.4 时下限自动失效、恢复「节点随缩放变大」的既有观感。
    // 位图烘制用的是**同一个下限**（只是把参考 zoom 换成 SPRITE_BAKE_ZOOM），
    // 所以两条路径算出的节点屏幕半径同量级 —— 切换时不会出现「点突然变胖/变瘦」。
    if (!nodesDrawnBySprite) {
      ctx.save();
      for (const node of visibleNodes) {
        ctx.globalAlpha = 0.85;
        ctx.beginPath();
        ctx.arc(node.x, node.y, nodeDrawRadius(node.size, zoom), 0, Math.PI * 2);
        ctx.fillStyle = node.color;
        ctx.fill();
      }
      ctx.restore();
    }

    // 绘制标签（zoom 足够时）
    if (zoom >= 0.4 && visibleNodes.length > 0) {
      ctx.save();
      ctx.textAlign = "center";
      ctx.textBaseline = "top";
      // 字号处于世界坐标系，必须除以 zoom 换算，保证任何缩放级别下屏幕字号恒定（10~12px）。
      // 修复前直接用屏幕像素值：zoom=0.4 时屏幕仅 3.6px 不可读，zoom=5 时 60px 巨大。
      const screenFontSize = zoom >= 1 ? 12 : zoom >= 0.6 ? 11 : 10;
      const fontSize = screenFontSize / zoom;
      // 标签与节点的间距同样换算为世界坐标
      const labelOffset = 3 / zoom;
      ctx.font = `${fontSize.toFixed(1)}px Inter, system-ui, sans-serif`;
      // ⚠ 2026-09-17 修复：此处**曾经只按度数取 Top-N、没有任何空间去重**，
      // 而大图自动聚类（aggActive）的节点恰好全走这条路径 ⇒ 放大到能看清单个社区时，
      // 250~500 个标签直接叠着画，字压字糊成一团（用户报「无法再放大、看不清」）。
      // 现改调 `selectLabelsToDraw`：网格分格（分布）+ cap 截断 + 矩形占位（互不重叠、含留白），
      // 与 `drawNodesOptimized` 共用同一份实现（同一语义只允许一处实现，判据 #299）。
      const cap = visibleNodes.length > 4000 ? 120 : visibleNodes.length > 1500 ? 250 : 500;
      ctx.fillStyle = token.colorText;
      ctx.globalAlpha = 0.85;
      const labelCandidates: LabelCandidate[] = [];
      for (const node of visibleNodes) {
        const meta = nodeMetaRef.current.get(node.id);
        if (!meta) { continue; }
        labelCandidates.push({
          id: node.id,
          x: node.x,
          y: node.y,
          size: node.size,
          title: meta.title.length > 18 ? meta.title.slice(0, 16) + "…" : meta.title,
        });
      }
      const placedLabels = selectLabelsToDraw(labelCandidates, {
        fontSizeWorld: fontSize,
        labelOffsetWorld: labelOffset,
        cap,
        measure: (text) => ctx.measureText(text).width,
      });
      for (const d of placedLabels) {
        ctx.fillText(d.title, d.labelX, d.labelY);
      }
      ctx.globalAlpha = 1;
      ctx.restore();
    }

    // 绘制边（只连接展开社区的节点）
    // Obsidian 风格：更细的线宽、更柔和的透明度、动态降采样
    // N2 修复：与 drawEdgesOptimized 对齐——补充边类型筛选、社区筛选、
    // hover/selected 相关边高亮，以及交互时普通边减淡
    //
    // ⚠ 端点判据修正（2026-09-15）：原判据用**降采样后的** visibleNodes 建 idSet，
    // 而 visibleNodes 在大图下只保留约 50% 节点（nodeSampleRate；该「按全图规模固定 0.5」
    // 的判据已于 2026-09-17 改为按视口预算，见 NODE_DRAW_BUDGET —— 但「端点判据必须与
    // 节点采样解耦」这个结论与采样率怎么取无关，仍然成立）⇒ 一条边要被画出来
    // 需要两端各自命中采样，保活率 = 0.5 × 0.5 = 0.25；再乘边自身采样率（0.15~0.5）
    // 后只剩 4%~13%。本数据集 74791 条边 ⇒ 实际参与绘制约 5600 条，
    // 且被保留的端点是**哈希随机**的 ⇒ 屏幕上剩下的点之间恰好没有边。
    // 这正是「一片互不相连的点」的直接成因：点少一半，边少四分之三。
    // 节点降采样只应影响**点的绘制密度**，不应充当边的可见性判据。
    // 现改为：端点集 = 本帧候选节点全集（expandedNodeIds ∩ posMap ∩ 视口），与节点采样解耦。
    if (edgeMeta.length > 0 && expandedNodeIds.size > 1) {
      const idSet = new Set<string>();
      for (const id of expandedNodeIds) {
        const node = posMap.get(id);
        if (!node) { continue; }
        if (!isInView(node.x, node.y, viewWorld, 20)) { continue; }
        idSet.add(id);
      }
      const zoom = cameraRef.current.zoom;
      const hovered = hoverNodeRef.current;
      const selected = selectedNodeIdRef.current;
      const visibleTypes = visibleEdgeTypesRef.current;
      const hasCommunityFilter = hasCommunityFilterRef.current;
      const visibleCommunitiesSet = visibleCommunitiesRef.current;
      // 仅「有高亮节点」（hover / 选中）—— **不含拖动**，与渲染分派处的 `hasHighlightOrDrag`
      // 不是同一个判据（2026-09-17 收口时拆分，此前三处共用 `hasActiveInteraction`）。
      const hasHighlight = !!hovered || !!selected;

      // 动态采样率：与节点层**共用同一判据**（`viewportDrawRate`），只把候选换成边数。
      // 改前是两套 zoom 分档，且大图分支在 `zoom >= 0.5` 后**封顶 0.5** ⇒ 放大到 5 倍
      // 也只有一半边，与节点层应有的「放大后全画」正好相反。
      // 候选边数用「视口内节点占比」折算：稀疏图的边数随可见顶点数近似线性，而
      // `idSet` 就是本帧视口内的候选节点集。定标：fit 态 idSet.size ≈ 全量
      // ⇒ rate ≈ 12000 / 74791 ≈ 0.16（改前实测 0.15，同量级 ⇒ 顶点预算不前移）；
      // zoom ≥ 1 时视口内候选边约 4700 条 ⇒ rate = 1（全画）。
      const visibleNodeRatio = idSet.size / Math.max(1, posMap.size);
      const edgeSampleRate = viewportDrawRate(edgeMeta.length * visibleNodeRatio, EDGE_DRAW_BUDGET);

      // 观测通道（与节点层同款）：边层的「候选数折算」是否成立，只能在这里看见。
      if (frameCounterRef.current % 30 === 0) {
        debugLog("[GraphView] edge draw rate", {
          totalEdges: edgeMeta.length,
          visibleNodes: idSet.size,
          edgeRate: Number(edgeSampleRate.toFixed(4)),
        });
      }
      // ⚠ 改前此处是 `if (hasHighlight) { edgeSampleRate = 1.0; }`（2026-09-17 移除）。
      //   「交互时保证 relevant 边完整呈现」这个目的**已经由采样判据自身满足** ——
      //   下方是 `if (!isRelevant && edgeSampleRate < 1.0)`，relevant 边**不参与采样**：
      //   无论 rate 取多少它都会被画出来。所以把 rate 抬到 1.0 是**重复保障**，代价却是
      //   本帧要为全部 74791 条普通边构造 Path2D（实测约 1.5×10⁵ 个顶点/帧）。
      //   而交互期普通边本来就已被压到 alpha 0.08（见下方
      //   `ctx.globalAlpha = hasHighlight ? 0.08 : normalAlpha`）⇒ 0.08 透明度下
      //   「15% 密度」与「100% 密度」肉眼不可分，交互期的观感由 relevant 边主导。

      // 动态线宽：屏幕恒定 1px（低缩放略细），世界坐标 = 1/zoom，见 worldEdgeWidth 说明
      const dynamicWidth = worldEdgeWidth(zoom < 0.3 ? 0.8 : 1.0, zoom);

      ctx.save();
      const batchPaths = new Map<string, { path: Path2D; color: string; width: number }>();
      const relevantPaths = new Map<string, { path: Path2D; color: string; width: number }>();

      for (let i = 0; i < edgeMeta.length; i++) {
        const em = edgeMeta[i];
        // ⚠ 这一行会**静默丢弃**两类边，且它们的规模**不在这里统计**（D-2，2026-09-17）：
        //   ① 端点不在**节点集**里（真悬空：实体被跨库合并搬走、或后端按域过滤掉了节点）
        //      —— 规模由后端 `GraphData.danglingEdges` 统计，并已在图谱页的提示条上显示；
        //   ② 端点在节点集里但**不在本帧视口**内（`idSet` 是视口级，见上方它怎么建出来的）
        //      —— 这是正常裁切，不该报警。
        //   ⇒ **不要在这里加计数**：本行拿到的是①②的**混合量**，与后端那个纯悬空量
        //     不是同一口径，拿去比对必然「不等」，只会制造假不一致。
        //      「边被丢了多少」这个问题只有后端能按域回答（`audit_kb_domain_consistency`）。
        if (!idSet.has(em.source) || !idSet.has(em.target)) { continue; }

        // 边类型筛选：用户关闭的类型不绘制
        if (!visibleTypes.has(em.type)) { continue; }

        // 社区筛选：开启过滤后，只画两端社区都可见的边
        if (hasCommunityFilter) {
          const sCid = getCommunityId(em.source);
          const tCid = getCommunityId(em.target);
          const sVisible = sCid === undefined || visibleCommunitiesSet.has(sCid);
          const tVisible = tCid === undefined || visibleCommunitiesSet.has(tCid);
          if (!sVisible || !tVisible) { continue; }
        }

        const sNode = posMap.get(em.source);
        const tNode = posMap.get(em.target);
        if (!sNode || !tNode) { continue; }
        if (!isInView(sNode.x, sNode.y, viewWorld, 10) && !isInView(tNode.x, tNode.y, viewWorld, 10)) { continue; }

        // hover/selected 相关边：单独收集，高亮绘制
        const isRelevant = (hovered && (em.source === hovered || em.target === hovered))
          || (selected && (em.source === selected || em.target === selected));

        // 确定性降采样：P11 用 source+target 稳定散列替代索引等差，避免保留边呈周期条纹
        if (!isRelevant && edgeSampleRate < 1.0) {
          const hash = (Math.abs(hashStringToInt(em.source + em.target)) % 1000) / 1000;
          if (hash > edgeSampleRate) { continue; }
        }

        // 高亮边 = 普通边 2 倍屏幕宽。
        // 不再用 Math.max(0.5, ...) 兜底：0.5 是世界坐标下限，低 zoom 下它反而
        // 把屏幕宽压到 0.5·zoom px（亚像素），使高亮边比普通边更看不见。
        const width = dynamicWidth * (isRelevant ? 2 : 1) * (em.width / 0.4);
        const key = `${em.color}|${width.toFixed(2)}`;
        const store = isRelevant ? relevantPaths : batchPaths;
        let entry = store.get(key);
        if (!entry) {
          entry = { path: new Path2D(), color: em.color, width };
          store.set(key, entry);
        }
        entry.path.moveTo(sNode.x, sNode.y);
        entry.path.lineTo(tNode.x, tNode.y);
      }

      // Obsidian 风格透明度；交互时普通边减淡，突出相关边
      const normalAlpha = zoom < 0.3 ? 0.12 : zoom < 0.5 ? 0.2 : 0.3;
      ctx.globalAlpha = hasHighlight ? 0.08 : normalAlpha;
      for (const entry of batchPaths.values()) {
        ctx.strokeStyle = entry.color;
        ctx.lineWidth = entry.width;
        ctx.stroke(entry.path);
      }
      // 相关边高亮：更宽、更不透明
      if (relevantPaths.size > 0) {
        ctx.globalAlpha = 0.85;
        for (const entry of relevantPaths.values()) {
          ctx.strokeStyle = entry.color;
          ctx.lineWidth = entry.width;
          ctx.stroke(entry.path);
        }
      }
      ctx.globalAlpha = 1;
      ctx.restore();
    }

    // 位图已画节点 ⇒ 返回 -1（「已通过其他方式绘制」）。
    // 否则安全阀会在 `expandedNodesDrawn === 0` 时再画一遍节点 ⇒ 双画（亮度失真、
    // hover/selected 高亮被覆盖），而 -1 的语义在安全阀注释里本来就写着「含位图」。
    if (nodesDrawnBySprite) { return -1; }
    return visibleNodes.length;
  }

  function drawEdgesOptimized(
    ctx: CanvasRenderingContext2D,
    nodes: PhysicsNode[],
    fisheye: FisheyeState,
    viewWorld: { x0: number; y0: number; x1: number; y1: number },
  ) {
    const edgeMeta = edgeMetaRef.current;
    const hovered = hoverNodeRef.current;
    const selected = selectedNodeIdRef.current;
    const visibleTypes = visibleEdgeTypesRef.current;
    const visibleCommunitiesSet = visibleCommunitiesRef.current;
    const zoom = cameraRef.current.zoom;

    // Obsidian 风格连线：根据缩放级别动态调整
    // 低缩放时降采样 + 更透明，高缩放时全量 + 更清晰
    const totalEdges = edgeMeta.length;
    // ⚠ 此处原有 `const hasHighlight = hovered || !!selected;`（2026-09-17 删除）：
    //   它唯一的两个用途就是给下面两道闸加 `!hasHighlight` 守卫，而那正是本函数
    //   「鼠标一进图就从采样跳到全量」的成因（见下方两道闸的注释）。

    // 降采样率：缩放越低，采样率越低
    let sampleRate = 1.0;
    if (zoom < 0.2) {
      sampleRate = 0.2; // 极低缩放：只画 20% 的边
    } else if (zoom < 0.4) {
      sampleRate = 0.4; // 低缩放：只画 40% 的边
    } else if (zoom < 0.6) {
      sampleRate = 0.7; // 中低缩放：画 70% 的边
    }

    // 大图边数量保护（**索引截断**，与下面的散列采样是两道独立闸）
    // ⚠ 两个 `!hasHighlight` 守卫已移除（2026-09-17）：它们让「鼠标停到一个节点上」
    //   把本帧的边构建量从 采样率² 直接抬到 100%（74791 条）。
    //   而「relevant 边完整呈现」这个目的在下面由 `!isRelevant` 单独放行 → 不依赖这两道闸。
    let edgeLimit = totalEdges;
    if (totalEdges > 50000) {
      edgeLimit = Math.floor(totalEdges * sampleRate);
    }

    // 动态线宽：屏幕恒定 1px（低缩放略细，避免密集区糊成一片），
    // 世界坐标 = 1/zoom，见 worldEdgeWidth 说明
    const dynamicWidth = worldEdgeWidth(zoom < 0.3 ? 0.8 : 1.0, zoom);

    const hasCommunityFilter = hasCommunityFilterRef.current;
    const batchPaths = new Map<string, { path: Path2D; color: string; width: number }>();

    // ⚠ 循环上界从 `edgeLimit` 改成 `totalEdges`：索引截断必须**下移到 relevant 判据之后**，
    //   否则 relevant 边只要索引超出截断点就永远画不出来（而它正是交互时唯一要看的边）。
    for (let i = 0; i < totalEdges; i++) {
      const em = edgeMeta[i];

      if (!visibleTypes.has(em.type)) { continue; }

      // ⚠ isRelevant 必须**先算**（改前它算在采样之后 ⇒ 采样无法用 `!isRelevant` 作守卫，
      //   于是只能靠「交互时整体关掉采样」来兜 —— 那正好是本函数的成本尖峰）。
      const isRelevant = hovered && (em.source === hovered || em.target === hovered)
        || selected && (em.source === selected || em.target === selected);

      // 两道闸**只作用于非 relevant 边**：索引截断 + 散列采样
      // （P11 用 source+target 稳定散列替代索引等差，避免保留边呈周期条纹）
      if (!isRelevant) {
        if (i >= edgeLimit) { continue; }
        if (sampleRate < 1.0) {
          const hash = (Math.abs(hashStringToInt(em.source + em.target)) % 1000) / 1000;
          if (hash > sampleRate) { continue; }
        }
      }

      const sNode = nodes[em.sourceIdx];
      const tNode = nodes[em.targetIdx];
      if (!sNode || !tNode) { continue; }

      const sCid = getCommunityId(em.source);
      const tCid = getCommunityId(em.target);
      const skipClusterCollapse = isAutoForceClusterRef.current;
      const sCollapsed = !skipClusterCollapse && clusterModeRef.current && sCid !== undefined
        && collapsedRef.current.has(sCid);
      const tCollapsed = !skipClusterCollapse && clusterModeRef.current && tCid !== undefined
        && collapsedRef.current.has(tCid);
      const sGeom = sCollapsed ? clusterGeomRef.current.get(sCid!) : undefined;
      const tGeom = tCollapsed ? clusterGeomRef.current.get(tCid!) : undefined;
      const s: { x: number; y: number } = sGeom ? { x: sGeom.cx, y: sGeom.cy } : sNode;
      const t: { x: number; y: number } = tGeom ? { x: tGeom.cx, y: tGeom.cy } : tNode;

      if (!isInView(s.x, s.y, viewWorld) && !isInView(t.x, t.y, viewWorld)) { continue; }

      if (hasCommunityFilter) {
        const sVisible = sCid === undefined || visibleCommunitiesSet.has(sCid);
        const tVisible = tCid === undefined || visibleCommunitiesSet.has(tCid);
        if (!sVisible || !tVisible) { continue; }
      }

      // ⚠ 已删除 `if (zoom < 0.15 && !isRelevant) { continue; }`（2026-09-15）。
      // 那不是淡出而是**整段跳过**：手动缩小到 zoom < 0.15（handleZoomOut 下限 0.05）
      // 后本路径一条普通边都不画，等于「缩得越小越看不到边」。
      // 低缩放的可见性由 worldEdgeWidth 的屏幕恒定线宽 + normalAlpha 分级共同保证，
      // 不需要再硬丢弃几何。

      if (isRelevant) {
        const sScale = fisheyeScale(s.x, s.y, fisheye);
        const tScale = fisheyeScale(t.x, t.y, fisheye);
        const avgScale = (sScale + tScale) / 2;
        ctx.beginPath();
        ctx.moveTo(s.x, s.y);
        const dx = t.x - s.x;
        const dy = t.y - s.y;
        const mx = (s.x + t.x) / 2;
        const my = (s.y + t.y) / 2;
        const curveAmount = Math.min(30, Math.sqrt(dx * dx + dy * dy) * 0.15);
        const nx = -dy / (Math.sqrt(dx * dx + dy * dy) || 1);
        const ny = dx / (Math.sqrt(dx * dx + dy * dy) || 1);
        const cpX = mx + nx * curveAmount;
        const cpY = my + ny * curveAmount;
        ctx.quadraticCurveTo(cpX, cpY, t.x, t.y);
        ctx.strokeStyle = em.color;
        // 高亮边 = 普通边 2 倍屏幕宽（fisheye 放大系数仍作用在世界坐标上）
        ctx.lineWidth = dynamicWidth * 2 * avgScale;
        ctx.globalAlpha = 0.85;
        ctx.stroke();
      } else {
        const width = dynamicWidth * (em.width / 0.4);
        const key = `${em.color}|${width.toFixed(2)}`;
        let entry = batchPaths.get(key);
        if (!entry) {
          entry = { path: new Path2D(), color: em.color, width };
          batchPaths.set(key, entry);
        }
        if (nodes.length < 5000 && zoom >= 0.3) {
          const dx = t.x - s.x;
          const dy = t.y - s.y;
          const mx = (s.x + t.x) / 2;
          const my = (s.y + t.y) / 2;
          const curveAmount = Math.min(20, Math.sqrt(dx * dx + dy * dy) * 0.08);
          const len = Math.sqrt(dx * dx + dy * dy) || 1;
          const cpX = mx + (-dy / len) * curveAmount;
          const cpY = my + (dx / len) * curveAmount;
          entry.path.moveTo(s.x, s.y);
          entry.path.quadraticCurveTo(cpX, cpY, t.x, t.y);
        } else {
          entry.path.moveTo(s.x, s.y);
          entry.path.lineTo(t.x, t.y);
        }
      }
    }

    if (batchPaths.size > 0) {
      // Obsidian 风格透明度：正常 0.35，hover/选中时更淡 0.1
      const normalAlpha = zoom < 0.3 ? 0.15 : zoom < 0.5 ? 0.25 : 0.35;
      const hoverAlpha = 0.08;
      ctx.globalAlpha = (hovered || selected) ? hoverAlpha : normalAlpha;
      const batchFeScale = fisheye.active ? fisheyeScale(fisheye.worldX, fisheye.worldY, fisheye) : 1;
      for (const entry of batchPaths.values()) {
        ctx.strokeStyle = entry.color;
        ctx.lineWidth = entry.width * batchFeScale;
        ctx.stroke(entry.path);
      }
      ctx.globalAlpha = 1;
    }
  }

  function drawParticlesOptimized(
    ctx: CanvasRenderingContext2D,
    nodes: PhysicsNode[],
    fisheye: FisheyeState,
    viewWorld: { x0: number; y0: number; x1: number; y1: number },
  ) {
    // 粒子默认关闭（对齐 Obsidian 静态细边），开关在工具栏/快捷键 p
    if (!particlesEnabledRef.current) { return; }
    const zoom = cameraRef.current.zoom;
    if (zoom < 0.5) { return; }

    const particles = particlesRef.current;
    const edgeMeta = edgeMetaRef.current;
    const visibleTypes = visibleEdgeTypesRef.current;

    const isStable = idleCounterRef.current > 0;

    // 稳定时粒子每 3 帧才更新一次位置
    if (!isStable || idleCounterRef.current % 3 === 0) {
      for (const p of particles) {
        p.progress += p.speed;
        if (p.progress > 1) { p.progress -= 1; }
      }
    }

    for (const p of particles) {
      const em = edgeMeta[p.edgeIndex];
      if (!em) { continue; }
      if (!visibleTypes.has(em.type)) { continue; }

      // 直接数组访问，避免 Map 查找
      const s = nodes[em.sourceIdx];
      const t = nodes[em.targetIdx];
      if (!s || !t) { continue; }

      // 聚类折叠模式：折叠社区内的边不画粒子（由聚合节点/聚合边表达）
      if (clusterModeRef.current) {
        const sCid = getCommunityId(em.source);
        const tCid = getCommunityId(em.target);
        if (
          (sCid !== undefined && collapsedRef.current.has(sCid))
          || (tCid !== undefined && collapsedRef.current.has(tCid))
        ) {
          continue;
        }
      }

      const x = s.x + (t.x - s.x) * p.progress;
      const y = s.y + (t.y - s.y) * p.progress;

      // 视口裁剪：粒子不在视口内时跳过
      if (!isInView(x, y, viewWorld, 30)) { continue; }

      const scale = fisheyeScale(x, y, fisheye);
      const alpha = 0.6 + 0.4 * Math.sin(p.progress * Math.PI * 2);
      // 稳定时跳过 shadowBlur（开销大）；用直接属性设置替代 save/restore
      if (!isStable) {
        ctx.shadowColor = p.color;
        ctx.shadowBlur = 6 * scale;
      }
      ctx.fillStyle = p.color;
      ctx.globalAlpha = alpha;
      // 屏幕下限**同源**（判据 #311）：粒子与节点同为「按世界坐标半径绘制的图元」，
      // 同样会在 zoom 压小时亚像素化。此前这里用裸 `p.size`（默认 ~2）⇒ 在
      // zoom=0.5 时屏幕半径只有 1px，抗锯齿后即稀释成背景灰 —— 与节点同一缺陷。
      // 注意本函数已有 `zoom < 0.5 return` 的前置守卫，所以下限只在
      // `p.size × zoom < 2` 时生效，正常放大观感不变。
      ctx.beginPath();
      ctx.arc(x, y, nodeDrawRadius(p.size, zoom) * scale, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
    ctx.shadowBlur = 0;
  }

  function drawNodesOptimized(
    ctx: CanvasRenderingContext2D,
    nodes: PhysicsNode[],
    fisheye: FisheyeState,
    viewWorld: { x0: number; y0: number; x1: number; y1: number },
  ) {
    const phase = phaseRef.current;
    const hovered = hoverNodeRef.current;
    const selected = selectedNodeIdRef.current;
    const highlight = highlightSetRef.current;
    const hasHighlight = highlight && highlight.size > 0;

    const neighbors = neighborsRef.current;
    const neighborsOfHovered = hovered ? (neighbors.get(hovered) || EMPTY_SET) : EMPTY_SET;
    const neighborsOfSelected = selected ? (neighbors.get(selected) || EMPTY_SET) : EMPTY_SET;

    const visibleCommunitiesSet = visibleCommunitiesRef.current;
    const hasCommunityFilter = hasCommunityFilterRef.current;

    const zoom = cameraRef.current.zoom;
    const showAllLabels = zoom >= 0.35 && !hasHighlight;
    const isLargeGraph = nodes.length > GLOW_NODE_LIMIT;

    // ── 关键性能优化：使用网格索引获取视口内的节点，避免遍历所有节点 ──
    const gridIndex = gridIndexRef.current;
    const nodeMap = posMapRef.current; // id -> PhysicsNode 映射
    const visibleNodeIds = new Set<string>();

    if (gridIndex && nodes.length > 1000) {
      // 大图模式：使用网格索引
      const gx0 = Math.floor(viewWorld.x0 / GRID_CELL_SIZE);
      const gy0 = Math.floor(viewWorld.y0 / GRID_CELL_SIZE);
      const gx1 = Math.floor(viewWorld.x1 / GRID_CELL_SIZE);
      const gy1 = Math.floor(viewWorld.y1 / GRID_CELL_SIZE);

      for (let gx = gx0; gx <= gx1; gx++) {
        for (let gy = gy0; gy <= gy1; gy++) {
          const bucket = gridIndex.get(`${gx},${gy}`);
          if (bucket) {
            for (const id of bucket) {
              visibleNodeIds.add(id);
            }
          }
        }
      }
    } else {
      // 小图模式：直接遍历所有节点（小图性能影响不大）
      for (const node of nodes) {
        if (isInView(node.x, node.y, viewWorld)) {
          visibleNodeIds.add(node.id);
        }
      }
    }

    // 只绘制视口内的节点
    // 交互外标签（showAllLabels）延后收集：万级节点全量画白字标签会重叠成
    // 一团白色浓雾（截图实锤），且每帧上万次 fillText 是性能黑洞。
    const deferredLabels: { id: string; x: number; y: number; size: number; alpha: number }[] = [];
    for (const nodeId of visibleNodeIds) {
      const node = nodeMap.get(nodeId);
      if (!node) { continue; }

      // 聚类折叠模式：折叠社区的节点由聚合节点替代，不单独绘制
      // 但在 auto force cluster 模式下，必须绘制所有节点
      if (clusterModeRef.current && !isAutoForceClusterRef.current) {
        const ncid = getCommunityId(node.id);
        if (ncid !== undefined && collapsedRef.current.has(ncid)) { continue; }
      }

      if (hasCommunityFilter) {
        const cid = getCommunityId(node.id);
        if (cid !== undefined && !visibleCommunitiesSet.has(cid)) { continue; }
      }

      const color = nodeColorRef.current.get(node.id) || token.colorPrimary;
      const baseSize = nodeSizeRef.current.get(node.id) || 6;

      const feScale = fisheyeScale(node.x, node.y, fisheye);

      let size = baseSize * feScale;
      let alpha = 1;
      let glowAlpha = 0.4;
      let glowRadius = baseSize * 2.5 * feScale;
      let showLabel = false;

      const isSelected = selected === node.id;
      const isHovered = hovered === node.id;

      if (isSelected) {
        size = baseSize * 1.8 * feScale;
        glowAlpha = 0.8;
        glowRadius = baseSize * 3 * feScale;
        showLabel = true;
      } else if (isHovered) {
        size = baseSize * 1.5 * feScale;
        glowAlpha = 0.6;
        glowRadius = baseSize * 2.5 * feScale;
        showLabel = true;
      } else if (selected && neighborsOfSelected.has(node.id)) {
        size = baseSize * 1.2 * feScale;
        glowAlpha = 0.3;
        showLabel = true;
      } else if (hovered && neighborsOfHovered.has(node.id)) {
        size = baseSize * 1.1 * feScale;
        glowAlpha = 0.25;
      } else if (hasHighlight && !highlight!.has(node.id)) {
        alpha = 0.15;
        glowAlpha = 0;
        size = baseSize * 0.8 * feScale;
      } else if (hovered || selected) {
        alpha = 0.15;
        glowAlpha = 0;
        size = baseSize * 0.85 * feScale;
      }

      const pulse = 1 + Math.sin(phase + node.x * 0.01) * 0.08;
      const finalSize = size * pulse;

      const isInteractNode = isSelected || isHovered
        || (selected && neighborsOfSelected.has(node.id))
        || (hovered && neighborsOfHovered.has(node.id));

      if (glowAlpha > 0 && zoom >= 0.6 && (isInteractNode || !isLargeGraph)) {
        if (idleCounterRef.current === 0) {
          ctx.shadowColor = color;
          ctx.shadowBlur = glowRadius;
        }
        ctx.globalAlpha = glowAlpha * alpha;
        ctx.beginPath();
        ctx.arc(node.x, node.y, finalSize, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        ctx.shadowBlur = 0;
      }

      ctx.globalAlpha = alpha;
      const screenR = finalSize * cameraRef.current.zoom;
      const sprite = nodeSpriteCacheRef.current.get(color);
      if (sprite && screenR >= 4) {
        const dstSize = finalSize * 2;
        ctx.drawImage(
          sprite,
          0,
          0,
          NODE_SPRITE_SIZE,
          NODE_SPRITE_SIZE,
          node.x - finalSize,
          node.y - finalSize,
          dstSize,
          dstSize,
        );
      } else {
        ctx.fillStyle = color;
        ctx.fillRect(node.x - finalSize, node.y - finalSize, finalSize * 2, finalSize * 2);
      }

      if (isHovered) {
        const ripplePhase = phase * 0.5;
        const rippleBase = finalSize * 2.5;
        for (let ri = 0; ri < 2; ri++) {
          const rp = (ripplePhase + ri * 0.5) % 1;
          ctx.globalAlpha = (ri === 0 ? 0.35 : 0.18) * (1 - rp);
          ctx.strokeStyle = color;
          // 涟漪圈宽同样处于世界坐标系：写固定 1.2 会被 zoom 再乘一次，
          // 低缩放时细到看不见（与边线宽同型缺陷）
          ctx.lineWidth = worldEdgeWidth(1.2, zoom);
          ctx.beginPath();
          ctx.arc(node.x, node.y, rippleBase + rp * 26, 0, Math.PI * 2);
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
      }

      if (showLabel) {
        const meta = nodeMetaRef.current.get(node.id);
        if (meta) {
          ctx.save();
          ctx.globalAlpha = alpha * 0.9;
          // N3 修复：字号处于世界坐标系，必须除以 zoom 换算（feScale 为鱼眼放大系数，保留）。
          // 修复前 zoom=0.35 时屏幕字号仅 ~4px 不可读，zoom=5 时 60px 巨大
          ctx.font = `${Math.round((12 * feScale) / zoom)}px Inter, system-ui, sans-serif`;
          ctx.textAlign = "center";
          ctx.textBaseline = "top";
          const label = meta.title.length > 15 ? meta.title.slice(0, 13) + "…" : meta.title;
          ctx.fillStyle = token.colorText;
          ctx.fillText(label, node.x, node.y + finalSize + 4);
          ctx.restore();
        }
      } else if (showAllLabels) {
        // 交互外标签延后绘制（见 deferredLabels 声明处的说明）
        deferredLabels.push({ id: node.id, x: node.x, y: node.y, size: finalSize, alpha });
      }
    }

    // 非交互标签：挑选逻辑收敛到 `labelLayout.selectLabelsToDraw`
    // （网格分格 → cap → 矩形占位），与 `drawExpandedCommunity` 共用**同一份**实现。
    // 2026-09-17 改动前：这里是内联的三段式，而 `drawExpandedCommunity` 只有「按度数 Top-N」，
    // 两条路径判据不一致 ⇒ 大图自动聚类（走后者）时标签无去重全叠。现已消除该分叉。
    if (deferredLabels.length > 0) {
      const labelFontSize = Math.round(12 / zoom);
      ctx.save();
      ctx.font = `${labelFontSize}px Inter, system-ui, sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "top";
      ctx.fillStyle = token.colorText;
      ctx.globalAlpha = 0.9;
      const labelCandidates: LabelCandidate[] = [];
      for (const d of deferredLabels) {
        const meta = nodeMetaRef.current.get(d.id);
        if (!meta) { continue; }
        labelCandidates.push({
          id: d.id,
          x: d.x,
          y: d.y,
          size: d.size,
          title: meta.title.length > 15 ? meta.title.slice(0, 13) + "…" : meta.title,
        });
      }
      const placedLabels = selectLabelsToDraw(labelCandidates, {
        fontSizeWorld: labelFontSize,
        // ⚠ 这个 `4` 是**世界坐标**常量（原实现如此），屏幕等效间距 = 4 × zoom。
        // 本轮**故意不顺手改成 `4 / zoom`**：该分支在当前大图配置（隐式聚合）下走不到，
        // 改了无法实测验证 —— 「无法验证的改动等于未验证的改动」。已作为量纲疑点登记。
        labelOffsetWorld: 4,
        cap: 500,
        measure: (text) => ctx.measureText(text).width,
      });
      for (const d of placedLabels) {
        ctx.fillText(d.title, d.labelX, d.labelY);
      }
      ctx.globalAlpha = 1;
      ctx.restore();
    }
    ctx.globalAlpha = 1;

    if (fisheye.active) {
      ctx.save();
      ctx.strokeStyle = token.colorPrimary;
      ctx.globalAlpha = 0.15;
      ctx.lineWidth = 1 / cameraRef.current.zoom;
      ctx.setLineDash([4 / cameraRef.current.zoom, 4 / cameraRef.current.zoom]);
      ctx.beginPath();
      ctx.arc(fisheye.worldX, fisheye.worldY, fisheye.radius, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.restore();
    }
  }

  // ── 颜色工具 ──
  function lightenColor(color: string, percent: number): string {
    const c = parseColor(color);
    if (!c) { return color; }
    const r = clamp(c.r + percent, 0, 255);
    const g = clamp(c.g + percent, 0, 255);
    const b = clamp(c.b + percent, 0, 255);
    return `rgb(${r},${g},${b})`;
  }

  function darkenColor(color: string, percent: number): string {
    const c = parseColor(color);
    if (!c) { return color; }
    const r = clamp(c.r - percent, 0, 255);
    const g = clamp(c.g - percent, 0, 255);
    const b = clamp(c.b - percent, 0, 255);
    return `rgb(${r},${g},${b})`;
  }

  function hexToRgba(color: string, alpha: number): string {
    const c = parseColor(color);
    if (!c) { return color; }
    return `rgba(${c.r},${c.g},${c.b},${alpha})`;
  }

  const NODE_SPRITE_SIZE = 128;

  function preRenderNodeSprite(color: string): HTMLCanvasElement {
    const canvas = document.createElement("canvas");
    canvas.width = NODE_SPRITE_SIZE;
    canvas.height = NODE_SPRITE_SIZE;
    const ctx = canvas.getContext("2d")!;
    const cx = NODE_SPRITE_SIZE / 2;
    const cy = NODE_SPRITE_SIZE / 2;
    const radius = NODE_SPRITE_SIZE * 0.47;

    const grad = ctx.createRadialGradient(
      cx - radius * 0.3,
      cy - radius * 0.3,
      0,
      cx,
      cy,
      radius,
    );
    grad.addColorStop(0, lightenColor(color, 40));
    grad.addColorStop(0.7, color);
    grad.addColorStop(1, darkenColor(color, 20));

    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.arc(cx, cy, radius, 0, Math.PI * 2);
    ctx.fill();
    return canvas;
  }

  function buildNodeSpriteCache() {
    const colors = new Set<string>();
    for (const color of nodeColorRef.current.values()) {
      colors.add(color);
    }
    const cache = new Map<string, HTMLCanvasElement>();
    for (const color of colors) {
      cache.set(color, preRenderNodeSprite(color));
    }
    nodeSpriteCacheRef.current = cache;
  }

  // ── 交互事件 ──

  // 构建聚合物理集：聚类折叠模式下，只对「聚合节点 + 未折叠节点」做物理。
  // 折叠社区的成员节点不进入物理（数量级骤降，物理规模 = 社区数 + 未折叠成员数），
  // 从根本上避免万级节点全量力导向收敛导致的卡死。
  const buildAggregatePhysics = useCallback(() => {
    const collapsed = collapsedRef.current;
    const communitiesMap = communitiesRef.current;
    const allNodes = physNodesRef.current;
    const edgeMeta = edgeMetaRef.current;
    if (!communitiesMap || allNodes.length === 0) {
      debugLog("[GraphView] buildAggregatePhysics early return", {
        communitiesMapNull: communitiesMap === null,
        collapsedSize: collapsed.size,
        allNodesLength: allNodes.length,
      });
      aggPhysRef.current = null;
      return;
    }
    // 成员计数与建图共用**同一个** countMembers 口径（graphAggregate），
    // 避免「判据用一个数、建图用另一个数」（此处只用于判断能否隐式聚合）。
    const memberCount = countMembers(allNodes, communitiesMap);

    // ── 隐式聚合（2026-09-16 修复）──
    // 原实现在 `collapsed.size === 0` 时硬早退，而**大图自动路径恰恰恒为「零折叠」**
    // （见 :1169-1173 注释「默认不折叠社区，让用户看到真实节点」）⇒ 聚合物理**永不激活**
    // ⇒ 力导向跑的是全部 24288 个原始节点 ⇒ 各社区成员散布全图、在坐标空间里从未被分离
    // ⇒ 气泡半径（按包围盒算）退化为「半个画布对角线」（判据 #298 因果链 ①→②→③）。
    // 修复：零折叠时把**全部社区**都当作布局单元（隐式聚合）。聚合物理规模 = 社区数
    // （本数据集 200，上限 MAX_AGG_PHYS_NODES）；成员节点不进物理，其坐标改由
    // applyAggregateLayout 从聚合节点派生回写 —— 这样「聚合物理真正激活」才有意义：
    // 力导向的单位从「原始节点」变成「社区」，社区之间才会真正分开。
    //
    // ⚠ 判据只取决于**用户模式**，绝不依赖 `collapsed` 的内容（第二次踩坑）：
    // 第一版写成 `collapsed.size === 0`，于是 LOD 一折叠（zoom 低 + 鼠标进画布就会发生）
    // 就把 implicit 翻成 false；而 implicit=false 又会让 LOD 被跳过 ⇒ **再也回不到 true**
    // （互相锁死）。表现为成员坐标在约 10 秒后**逐位冻结**、而聚合物理仍在跑
    // （实测：节点绘制坐标三次采样完全相同，气泡中心却从 ±1400 漂到 ±3700）。
    // 现在：关闭聚类 / 大图自动聚类 ⇒ 隐式聚合（成员可见）；用户手动开启聚类
    // ⇒ 显式折叠（成员由聚合彩球替代，不需要回写）。
    const implicitLayout = !clusterModeRef.current || isAutoForceClusterRef.current;
    const implicitAggregation = implicitLayout
      && memberCount.size > 0
      && memberCount.size <= MAX_AGG_PHYS_NODES;
    const layoutUnits: ReadonlySet<number> = implicitAggregation
      ? new Set(memberCount.keys())
      : collapsed;
    if (layoutUnits.size === 0) {
      debugLog("[GraphView] buildAggregatePhysics early return", {
        communitiesMapNull: false,
        collapsedSize: collapsed.size,
        allNodesLength: allNodes.length,
        communityCount: memberCount.size,
      });
      aggPhysRef.current = null;
      return;
    }

    // 每个布局单元（折叠社区 或 隐式聚合下的全部社区）→ 1 个聚合物理节点。
    // 2026-09-16：构建逻辑整体迁至 graphAggregate.ts（纯函数层）—— 抽出的**唯一**动机是
    // 「这套参数到底会不会自平衡」必须能在测试里用**同一份构建代码**复现，否则标定脚本
    // 只能重抄一遍建图逻辑，抄的那一刻被测对象就分叉了（判据 #7/#313 同族）。
    const built = buildAggregateGraph({
      nodes: allNodes,
      edges: edgeMeta,
      communities: communitiesMap,
      layoutUnits,
      centroidOf: (cid) => clusterGeomRef.current.get(cid),
      // 播种力参数与物理参数**同源**（避免「算 R* 的 M」与「实际播下的 M」两套口径）。
      // 首屏无质心（clusterGeom 还空）时按 R* 做圆盘播种，而不是旧的 r=400 圆环
      // —— 后者正是「首屏中心空白、约 40s 才成形」的根因（AUDIT §6.12.6-2）。
      seedPhysics: AGG_PHYSICS_CONFIG,
    });

    // 聚合边（含去重合并）与成员计数均已由 buildAggregateGraph 产出（见 graphAggregate.ts）。
    // 进入新的隐式聚合周期（首次构建 / 从显式全折叠切回）⇒ 相机需要重新对齐一次。
    // 不重置的话，布局重来（用户「关掉聚类 → 再打开 → 再关掉」）时会因为
    // 「bbox 与上次相同」而跳过对齐，又回到「画了但看不见」。
    if (implicitAggregation && aggPhysRef.current?.implicit !== true) {
      lastAggFitBBoxRef.current = null;
      lastAggFitFrameRef.current = -1_000_000;
    }
    // 换了一套布局单元（折叠集合变了 / 首次构建）⇒ 上一轮的「已收敛」结论作废。
    // 不重置的话，用户折叠一个社区后会看到新布局被旧结论立刻冻结。
    aggSettleRef.current = createAggregateSettleState();
    // 同理，退火也要重来：新布局单元 = 新的演化周期。不重置的话会继承上一轮的低温度
    // （甚至终值 4e-4）⇒ 新布局一出生就被冻住，永远散不开。
    reheatAggregateAnneal(aggAnnealRef.current);

    aggPhysRef.current = {
      nodes: built.nodes,
      edges: built.edges,
      cidToNodeIdx: built.cidToNodeIdx,
      neighborMap: buildNeighborMap(built.edges),
      implicit: implicitAggregation,
    };
    debugLog("[GraphView] buildAggregatePhysics success", {
      aggNodes: built.nodes.length,
      aggEdges: built.edges.length,
      cidToNodeIdx: built.cidToNodeIdx.size,
      implicitAggregation,
    });
  }, []);

  // ── 聚合布局 → 成员坐标回写 + 相机跟随（2026-09-16 新增）──
  // 隐式聚合下成员节点不进物理，它们的坐标必须由聚合节点位置派生 —— 否则「聚合物理
  // 真正激活」只会表现为多画了几条聚合边，节点本身仍停在旧位置，社区依旧不分离。
  // 派生规则：每个社区成员按「黄金角 + 面积均匀环」铺在聚合节点周围的圆内，
  // 半径取 communityRadius(count)，与背景气泡半径**同源**（见 graphViewUtils），
  // 这样「节点团实际占据的范围」与「气泡画多大」不会各算各的而错位。
  // 写入的是 physNodesRef 内的同一批对象引用，而 posMapRef 持有相同引用 ⇒ 绘制路径自动同步。
  //
  // ⚠ 2026-09-16 起本函数对**显式折叠**也生效（此前 `!agg.implicit` 直接 return）：
  //   显式折叠下成员坐标由**另一套**机制决定（Worker 主物理），这里只做「相机跟随」。
  //   不这样做的话，显式折叠的坐标尺度仍来自主物理（Ω(10⁴)）而成员派生尺度是 ≤1500
  //   ⇒ 两条路径的像素覆盖率差 13 倍（0.488% vs 5.379%）。尺度归一化已上移到调用点
  //   （见 step 循环），因此这里不再自带归一化 —— 一处只做一件事。
  const applyAggregateLayout = useCallback(() => {
    const agg = aggPhysRef.current;
    if (!agg) { return; }
    const communitiesMap = communitiesRef.current;
    if (!communitiesMap) { return; }

    const groups = new Map<number, PhysicsNode[]>();
    for (const node of physNodesRef.current) {
      const cid = communitiesMap.get(node.id);
      if (cid === undefined) { continue; }
      const g = groups.get(cid);
      if (g) { g.push(node); }
      else { groups.set(cid, [node]); }
    }

    // 成员坐标派生**只对隐式聚合**做：显式折叠下成员节点不渲染，其坐标由主物理
    // （Worker）决定；若这里也去改写，会与 Worker 回写互相覆盖（同族前科见 step 循环注释）。
    const GOLDEN = 0.6180339887498949;
    let moved = 0;
    if (agg.implicit) {
      for (const [cid, members] of groups) {
        const idx = agg.cidToNodeIdx.get(cid);
        if (idx === undefined) { continue; }
        const center = agg.nodes[idx];
        if (!center) { continue; }
        const n = members.length;
        const r = communityRadius(n);
        for (let k = 0; k < n; k++) {
          const m = members[k];
          // 用户拖拽固定的节点保持原位，不被派生逻辑拽回
          if (m.fixed) { continue; }
          const angle = 2 * Math.PI * ((k * GOLDEN) % 1);
          const ring = Math.sqrt((k + 0.5) / n);
          m.x = center.x + r * ring * Math.cos(angle);
          m.y = center.y + r * ring * Math.sin(angle);
          m.vx = 0;
          m.vy = 0;
          moved++;
        }
      }
    }
    if (frameCounterRef.current % 60 === 0) {
      debugLog("[GraphView] applyAggregateLayout", {
        implicit: agg.implicit,
        communities: groups.size,
        moved,
        sample: agg.nodes.slice(0, 3).map((n) => ({ x: n.x.toFixed(0), y: n.y.toFixed(0) })),
      });
    }

    // ── 依赖坐标的派生缓存必须随之重建（各自限流）──
    // 上面改写了**全部**成员节点的 x/y，两处缓存的 key/参照系都由坐标算出：
    //   · gridIndex —— 绘制路径按它做「视口 → 候选节点」粗筛，key 由坐标算出 ⇒
    //     沿用旧坐标会查不到实际已在视口内的节点（漏画，且随迭代越漏越多）。
    //   · 大图位图 —— sprite 与 spriteWorldBBox 是一对，bbox 决定 drawImage 的
    //     位置与缩放 ⇒ 二者不同源时位图会被画到错误的尺度/位置（观感≈空白）。
    // 两者都是 O(N)（位图还含 O(E)），必须限流，不能每帧做。
    // ⚠ 只在成员的坐标真的被改过（implicit）时重建：显式折叠下坐标没动，
    //   重建是纯浪费（位图那一支尤其 —— 一次可达 4096² ≈ 64MB 分配）。
    const frame = frameCounterRef.current;
    if (agg.implicit && frame - lastAggGridRebuildRef.current >= 24) {
      lastAggGridRebuildRef.current = frame;
      const gridIndex = new Map<string, string[]>();
      for (const node of physNodesRef.current) {
        const gx = Math.floor(node.x / GRID_CELL_SIZE);
        const gy = Math.floor(node.y / GRID_CELL_SIZE);
        const key = `${gx},${gy}`;
        const bucket = gridIndex.get(key);
        if (bucket) { bucket.push(node.id); }
        else { gridIndex.set(key, [node.id]); }
      }
      gridIndexRef.current = gridIndex;
    }

    // ── 相机跟随聚合布局（周期性对齐，直到用户接管视角）──
    // 为什么必须做：隐式聚合把节点坐标的**尺度**整个换掉了 —— 从「原始力导向铺满视口」
    // 换成「AGG_PHYSICS_CONFIG 决定的社区级分布」。两者量纲不同，沿用旧相机会出现
    // 「节点确实画了、却落在视口外或挤成一条窄带」的**假空白**（实测 screenX 跨度仅
    // 45px / 画布 1357px，气泡中心仅 7.4% 落在视口内）。这类问题肉眼与像素判据都会
    // 误判成「没有渲染」，所以对齐相机是这次改动不可省的一环。
    // 对齐策略：布局的尺度和中心还在变时持续跟随（限流 + 变化阈值），
    // 一旦用户自己动过相机就永久停手（尊重用户操作）。
    if (!cameraTouchedByUserRef.current && frame - lastAggFitFrameRef.current >= 120) {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (const n of agg.nodes) {
        if (n.x < minX) { minX = n.x; }
        if (n.y < minY) { minY = n.y; }
        if (n.x > maxX) { maxX = n.x; }
        if (n.y > maxY) { maxY = n.y; }
      }
      if (Number.isFinite(minX) && Number.isFinite(maxX)) {
        const spanX = maxX - minX;
        const spanY = maxY - minY;
        const ccx = (minX + maxX) / 2;
        const ccy = (minY + maxY) / 2;
        const prev = lastAggFitBBoxRef.current;
        const scale = Math.max(spanX, spanY, 1);
        const changed = prev === null
          || Math.abs(spanX - prev.spanX) / Math.max(prev.spanX, 1) > 0.15
          || Math.abs(spanY - prev.spanY) / Math.max(prev.spanY, 1) > 0.15
          || Math.hypot(ccx - prev.cx, ccy - prev.cy) / scale > 0.1;
        if (changed) {
          fitAllRef.current?.();
          lastAggFitFrameRef.current = frame;
          lastAggFitBBoxRef.current = { spanX, spanY, cx: ccx, cy: ccy };
          debugLog("[GraphView] aggregate layout camera fit", {
            spanX: spanX.toFixed(0),
            spanY: spanY.toFixed(0),
            zoom: cameraRef.current.zoom.toFixed(3),
          });
        }
      }
    }
  }, []);

  // 刷新聚合节点几何（质心/半径/计数/代表名）。O(N) 遍历，低频调用（每 6 帧 / 切换时）
  const refreshClusterGeom = useCallback(() => {
    const activeCommunities = effectiveCommunitiesRef.current ?? communities;
    const nodeCount = physNodesRef.current.length;
    // 强制聚类模式：节点数超过阈值时也需要计算聚类几何
    const isForceCluster = nodeCount > AUTO_CLUSTER_THRESHOLD;
    if (!activeCommunities || (!clusterModeRef.current && !isForceCluster)) {
      if (frameCounterRef.current % 60 === 0) {
        debugLog("[GraphView] refreshClusterGeom early return", {
          activeCommunitiesNull: activeCommunities === null,
          clusterMode: clusterModeRef.current,
          isForceCluster,
        });
      }
      clusterGeomRef.current = new Map();
      // D1: 几何不可用时空置质心缓存，保持 drawClusterRegions 的 early return 语义一致
      communityCentroidsRef.current = new Map();
      return;
    }
    const buckets = new Map<
      number,
      { sx: number; sy: number; count: number; bestId: string | null; bestDegree: number }
    >();
    const nodes = physNodesRef.current;
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];
      const cid = activeCommunities.get(node.id);
      if (cid === undefined) { continue; }
      const b = buckets.get(cid) ?? { sx: 0, sy: 0, count: 0, bestId: null, bestDegree: -1 };
      b.sx += node.x;
      b.sy += node.y;
      b.count += 1;
      const meta = nodeMetaRef.current.get(node.id);
      const deg = (meta?.linkCount ?? 0) + (meta?.backlinkCount ?? 0);
      if (deg > b.bestDegree) {
        b.bestDegree = deg;
        b.bestId = node.id;
      }
      buckets.set(cid, b);
    }
    const next = new Map<number, { cx: number; cy: number; r: number; count: number; label: string }>();
    for (const [cid, b] of buckets) {
      const cx = b.sx / b.count;
      const cy = b.sy / b.count;
      // 半径与「成员散布半径」同源（communityRadius）—— 两处若各算各的会错位
      const r = communityRadius(b.count);
      const title = b.bestId ? (nodeMetaRef.current.get(b.bestId)?.title ?? "") : "";
      const label = title.length > 14 ? title.slice(0, 12) + "…" : title || `#${cid}`;
      next.set(cid, { cx, cy, r, count: b.count, label });
    }
    clusterGeomRef.current = next;
    // D1: 同步回填社区质心缓存，供 drawClusterRegions（背景气泡）使用。
    // refreshClusterGeom 在 Worker ready 回调 / LOD 切换 / 折叠切换时都会被调用，
    // 使 Worker 主路径下 communityCentroidsRef 不再为空，恢复气泡渲染。
    const centroidMap = new Map<number, { cx: number; cy: number; count: number }>();
    for (const [cid, g] of next) {
      centroidMap.set(cid, { cx: g.cx, cy: g.cy, count: g.count });
    }
    communityCentroidsRef.current = centroidMap;
    // 质心变化 → 气泡缓存置脏，下一帧重建
    clusterRegionCacheRef.current.dirty = true;
    if (frameCounterRef.current % 60 === 0) {
      const positions = [];
      let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
      for (const [cid, g] of next) {
        positions.push({ cid, cx: g.cx.toFixed(0), cy: g.cy.toFixed(0), r: g.r.toFixed(0), count: g.count });
        minX = Math.min(minX, g.cx);
        maxX = Math.max(maxX, g.cx);
        minY = Math.min(minY, g.cy);
        maxY = Math.max(maxY, g.cy);
      }
      debugLog("[GraphView] refreshClusterGeom success", {
        bucketCount: buckets.size,
        nextSize: next.size,
        bbox: { minX: minX.toFixed(0), maxX: maxX.toFixed(0), minY: minY.toFixed(0), maxY: maxY.toFixed(0) },
        sample: positions.slice(0, 5),
      });
    }
  }, [communities]);

  // 切换社区折叠状态（点击聚合节点）
  const toggleCluster = useCallback((cid: number) => {
    const next = new Set(collapsedRef.current);
    const manualNext = new Set(manualExpandedRef.current);
    if (next.has(cid)) {
      next.delete(cid);
      // 手动展开的社区标记，防止 LOD 自动折叠
      manualNext.add(cid);
    } else {
      next.add(cid);
      // 手动折叠的社区，从手动展开列表移除
      manualNext.delete(cid);
    }
    collapsedRef.current = next;
    manualExpandedRef.current = manualNext;
    // 立即刷新聚合几何（展开/收起后质心渲染立即生效）
    refreshClusterGeom();
    // 折叠集合变化 → 重建聚合物理集（聚合节点/未折叠成员集合都变了）
    buildAggregatePhysics();
    setClusterCollapseVersion((v) => v + 1);
  }, [refreshClusterGeom, buildAggregatePhysics]);

  // 聚合节点命中检测（聚类模式 + 折叠社区）
  const findClusterAt = useCallback((sx: number, sy: number): number | null => {
    if (!clusterModeRef.current) { return null; }
    const world = getScreenToWorld(sx, sy);
    for (const [cid, geom] of clusterGeomRef.current) {
      if (!collapsedRef.current.has(cid)) { continue; }
      const dx = world.x - geom.cx;
      const dy = world.y - geom.cy;
      const hitR = geom.r * 1.6; // 含外圈光晕
      if (dx * dx + dy * dy < hitR * hitR) {
        return cid;
      }
    }
    return null;
  }, [dimensions]);

  const findNodeAt = useCallback((sx: number, sy: number): string | null => {
    const world = getScreenToWorld(sx, sy);
    const grid = gridIndexRef.current;
    const gx = Math.floor(world.x / GRID_CELL_SIZE);
    const gy = Math.floor(world.y / GRID_CELL_SIZE);
    const posMap = posMapRef.current;

    for (let dy = -1; dy <= 1; dy++) {
      for (let dx = -1; dx <= 1; dx++) {
        const key = `${gx + dx},${gy + dy}`;
        const ids = grid.get(key);
        if (!ids || ids.length === 0) { continue; }
        for (let i = ids.length - 1; i >= 0; i--) {
          const id = ids[i];
          const n = posMap.get(id);
          if (!n) { continue; }
          // 聚类折叠模式：折叠社区的节点被聚合节点覆盖，不参与命中
          if (clusterModeRef.current) {
            const cid = getCommunityId(id);
            if (cid !== undefined && collapsedRef.current.has(cid)) { continue; }
          }
          const size = nodeSizeRef.current.get(id) || 6;
          // ⚠ 命中半径必须与**绘制半径**取同一口径（nodeDrawRadius）：绘制侧含屏幕像素
          // 下限，若命中仍按裸世界半径判定，就会出现「看得见、点不中」——屏幕上是 2px
          // 的点，可命中区却只有 0.63px。二者同源是「视觉半径 ↔ 命中半径」的判据。
          const hitSize = nodeDrawRadius(size, cameraRef.current.zoom);
          const wx = n.x - world.x;
          const wy = n.y - world.y;
          if (wx * wx + wy * wy < hitSize * hitSize) {
            return id;
          }
        }
      }
    }
    return null;
  }, [dimensions]);

  const MINIMAP_W = 200;
  const MINIMAP_H = 150;

  const drawMinimap = useCallback((mmCtx: CanvasRenderingContext2D, nodes: PhysicsNode[]) => {
    if (nodes.length === 0) { return; }

    // 系统稳定时复用缓存包围盒；运动中或无缓存时重算
    const stable = idleCounterRef.current > 30;
    let bbox = stable ? minimapBBoxRef.current : null;
    if (!bbox) {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (const n of nodes) {
        if (n.x < minX) { minX = n.x; }
        if (n.y < minY) { minY = n.y; }
        if (n.x > maxX) { maxX = n.x; }
        if (n.y > maxY) { maxY = n.y; }
      }
      bbox = { minX, minY, maxX, maxY };
      minimapBBoxRef.current = bbox;
    }
    let { minX, minY, maxX, maxY } = bbox;
    const bboxW = Math.max(maxX - minX, 1);
    const bboxH = Math.max(maxY - minY, 1);
    const padX = bboxW * 0.1;
    const padY = bboxH * 0.1;
    minX -= padX;
    maxX += padX;
    minY -= padY;
    maxY += padY;

    const scale = Math.min(MINIMAP_W / (maxX - minX), MINIMAP_H / (maxY - minY));
    const offsetX = (MINIMAP_W - (maxX - minX) * scale) / 2;
    const offsetY = (MINIMAP_H - (maxY - minY) * scale) / 2;

    mmCtx.clearRect(0, 0, MINIMAP_W, MINIMAP_H);
    mmCtx.fillStyle = token.colorBgContainer;
    mmCtx.fillRect(0, 0, MINIMAP_W, MINIMAP_H);

    const gridSize = 20;
    mmCtx.fillStyle = hexToRgba(token.colorText, 0.05);
    for (let x = gridSize; x < MINIMAP_W; x += gridSize) {
      for (let y = gridSize; y < MINIMAP_H; y += gridSize) {
        mmCtx.beginPath();
        mmCtx.arc(x, y, 0.5, 0, Math.PI * 2);
        mmCtx.fill();
      }
    }

    // 聚合折叠模式：minimap 与主视图一致——折叠社区画聚合点，展开社区画真实节点
    const clusterActive = clusterModeRef.current && collapsedRef.current.size > 0;
    if (clusterActive) {
      const geom = clusterGeomRef.current;
      // 折叠社区 → 聚合点（社区色，更大）
      for (const [cid, g] of geom) {
        if (!collapsedRef.current.has(cid)) { continue; }
        const mx = (g.cx - minX) * scale + offsetX;
        const my = (g.cy - minY) * scale + offsetY;
        mmCtx.fillStyle = communityPalette[cid % communityPalette.length];
        mmCtx.beginPath();
        mmCtx.arc(mx, my, 2.6, 0, Math.PI * 2);
        mmCtx.fill();
      }
      // 展开社区 → 真实节点（小点，降采样）
      const nodeStep = nodes.length > 20000 ? 8 : nodes.length > 8000 ? 4 : nodes.length > 3000 ? 2 : 1;
      for (let i = 0; i < nodes.length; i += nodeStep) {
        const n = nodes[i];
        const cid = getCommunityId(n.id);
        if (cid !== undefined && collapsedRef.current.has(cid)) { continue; }
        const color = nodeColorRef.current.get(n.id) || token.colorPrimary;
        const mx = (n.x - minX) * scale + offsetX;
        const my = (n.y - minY) * scale + offsetY;
        mmCtx.fillStyle = color;
        mmCtx.beginPath();
        mmCtx.arc(mx, my, 1.8, 0, Math.PI * 2);
        mmCtx.fill();
      }
    } else {
      // 普通模式：节点绘制降采样（大图概览无需逐点绘制）
      const nodeStep = nodes.length > 20000 ? 8 : nodes.length > 8000 ? 4 : nodes.length > 3000 ? 2 : 1;
      for (let i = 0; i < nodes.length; i += nodeStep) {
        const n = nodes[i];
        const color = nodeColorRef.current.get(n.id) || token.colorPrimary;
        const mx = (n.x - minX) * scale + offsetX;
        const my = (n.y - minY) * scale + offsetY;
        mmCtx.fillStyle = color;
        mmCtx.beginPath();
        mmCtx.arc(mx, my, 1.8, 0, Math.PI * 2);
        mmCtx.fill();
      }
    }

    const cam = cameraRef.current;
    const vx = ((-cam.x / cam.zoom) - minX) * scale + offsetX;
    const vy = ((-cam.y / cam.zoom) - minY) * scale + offsetY;
    const vw = (dimensions.width / cam.zoom) * scale;
    const vh = (dimensions.height / cam.zoom) * scale;

    mmCtx.save();
    mmCtx.strokeStyle = token.colorPrimary;
    mmCtx.lineWidth = 1.5;
    mmCtx.globalAlpha = 0.8;
    mmCtx.strokeRect(vx - vw / 2, vy - vh / 2, vw, vh);
    mmCtx.fillStyle = hexToRgba(token.colorPrimary, 0.08);
    mmCtx.fillRect(vx - vw / 2, vy - vh / 2, vw, vh);
    mmCtx.restore();
  }, [token, dimensions, communities]);

  const getMinimapWorldBounds = useCallback(() => {
    const nodes = physNodesRef.current;
    if (nodes.length === 0) { return null; }
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const n of nodes) {
      if (n.x < minX) { minX = n.x; }
      if (n.y < minY) { minY = n.y; }
      if (n.x > maxX) { maxX = n.x; }
      if (n.y > maxY) { maxY = n.y; }
    }
    const bboxW = Math.max(maxX - minX, 1);
    const bboxH = Math.max(maxY - minY, 1);
    const padX = bboxW * 0.1;
    const padY = bboxH * 0.1;
    minX -= padX;
    maxX += padX;
    minY -= padY;
    maxY += padY;
    const scale = Math.min(MINIMAP_W / (maxX - minX), MINIMAP_H / (maxY - minY));
    const offsetX = (MINIMAP_W - (maxX - minX) * scale) / 2;
    const offsetY = (MINIMAP_H - (maxY - minY) * scale) / 2;
    return { minX, minY, scale, offsetX, offsetY };
  }, []);

  const handleMinimapNavigate = useCallback((mmX: number, mmY: number) => {
    const bounds = getMinimapWorldBounds();
    if (!bounds) { return; }
    const wx = (mmX - bounds.offsetX) / bounds.scale + bounds.minX;
    const wy = (mmY - bounds.offsetY) / bounds.scale + bounds.minY;
    const cam = cameraRef.current;
    const targetZoom = Math.max(cam.zoom, 1);
    cam.x = -wx * targetZoom;
    cam.y = -wy * targetZoom;
    cam.zoom = targetZoom;
  }, [getMinimapWorldBounds]);

  const handleMinimapMouseDown = useCallback((e: ReactMouseEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    const rect = minimapRef.current!.getBoundingClientRect();
    const mmX = e.clientX - rect.left;
    const mmY = e.clientY - rect.top;
    minimapDragRef.current = true;
    handleMinimapNavigate(mmX, mmY);
  }, [handleMinimapNavigate]);

  const handleMinimapMouseMove = useCallback((e: ReactMouseEvent<HTMLCanvasElement>) => {
    if (!minimapDragRef.current) { return; }
    const rect = minimapRef.current!.getBoundingClientRect();
    const mmX = e.clientX - rect.left;
    const mmY = e.clientY - rect.top;
    handleMinimapNavigate(mmX, mmY);
  }, [handleMinimapNavigate]);

  const handleMinimapMouseUp = useCallback(() => {
    minimapDragRef.current = false;
  }, []);

  const handleMouseDown = useCallback((e: ReactMouseEvent<HTMLCanvasElement>) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;

    // 聚合节点点击：展开/收起社区（优先于普通节点/平移）
    const clusterId = findClusterAt(sx, sy);
    if (clusterId !== null) {
      suppressAutoFocusRef.current = true;
      toggleCluster(clusterId);
      return;
    }

    const nodeId = findNodeAt(sx, sy);

    if (nodeId) {
      suppressAutoFocusRef.current = true;
      const node = posMapRef.current.get(nodeId);
      if (node) {
        node.fixed = true;
        dragRef.current = { nodeId };
        onNodeClick?.(nodeId);
      }
    } else {
      // 开始平移
      panRef.current = { startX: e.clientX, startY: e.clientY, camX: cameraRef.current.x, camY: cameraRef.current.y };
      onDeselect?.();
    }
  }, [findNodeAt, findClusterAt, toggleCluster, onNodeClick, onDeselect]);

  const handleMouseMove = useCallback((e: ReactMouseEvent<HTMLCanvasElement>) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;

    // 记录鼠标屏幕位置（供鱼眼使用）
    mouseScreenRef.current = { x: sx, y: sy, active: true };

    if (dragRef.current) {
      const world = getScreenToWorld(sx, sy);
      const node = posMapRef.current.get(dragRef.current!.nodeId);
      if (node) {
        node.x = world.x;
        node.y = world.y;
        node.vx = 0;
        node.vy = 0;
      }
    } else if (panRef.current) {
      const dx = e.clientX - panRef.current.startX;
      const dy = e.clientY - panRef.current.startY;
      cameraRef.current.x = panRef.current.camX + dx;
      cameraRef.current.y = panRef.current.camY + dy;
      cameraTouchedByUserRef.current = true; // 用户拖拽平移 ⇒ 停掉聚合布局的自动对齐
    } else {
      // hover 检测
      // 聚合节点 hover 优先（聚类折叠模式）
      const clusterId = findClusterAt(sx, sy);
      if (clusterId !== null) {
        if (hoverClusterRef.current !== clusterId) {
          hoverClusterRef.current = clusterId;
          canvasRef.current!.style.cursor = "pointer";
        }
        if (hoverNodeRef.current) {
          hoverNodeRef.current = null;
          onNodeHover?.(null);
          tooltipVisibleRef.current = false;
          setTooltipNodeIdState(null);
        }
        return;
      }
      hoverClusterRef.current = null;

      const nodeId = findNodeAt(sx, sy);
      if (nodeId !== hoverNodeRef.current) {
        hoverNodeRef.current = nodeId;
        onNodeHover?.(nodeId);
        canvasRef.current!.style.cursor = nodeId ? "pointer" : "grab";

        // 节点变化：更新内容（低频 React 渲染）+ 位置（ref）
        if (nodeId) {
          const tooltipX = Math.min(sx + 16, dimensions.width - 260);
          const tooltipY = Math.min(sy + 16, dimensions.height - 160);
          tooltipPosRef.current = { x: tooltipX, y: tooltipY };
          tooltipVisibleRef.current = true;
          setTooltipNodeIdState(nodeId);
        } else {
          tooltipVisibleRef.current = false;
          setTooltipNodeIdState(null);
        }
      } else if (nodeId) {
        // 同一节点移动：只更新位置（ref，无 React 渲染）
        const tooltipX = Math.min(sx + 16, dimensions.width - 260);
        const tooltipY = Math.min(sy + 16, dimensions.height - 160);
        tooltipPosRef.current = { x: tooltipX, y: tooltipY };
      }
    }
  }, [findNodeAt, findClusterAt, onNodeHover, dimensions]);

  const handleMouseUp = useCallback(() => {
    if (dragRef.current) {
      const node = posMapRef.current.get(dragRef.current!.nodeId);
      if (node) {
        node.fixed = false;
        node.fx = 0;
        node.fy = 0;
        // 同步到 Worker：释放节点
        const worker = workerRef.current;
        if (worker) {
          worker.postMessage({
            type: "update",
            payload: {
              nodeIdx: node.idx,
              x: node.x,
              y: node.y,
              fixed: false,
              vx: 0,
              vy: 0,
            },
          } as WorkerMessage);
        }
      }
      dragRef.current = null;

      // 拖拽结束后保存布局到 localStorage
      if (wikiIdRef.current) {
        saveLayout(wikiIdRef.current, physNodesRef.current, cameraRef.current);
      }
    }
    panRef.current = null;
  }, []);

  const handleMouseLeave = useCallback(() => {
    hoverNodeRef.current = null;
    hoverClusterRef.current = null;
    mouseScreenRef.current = { x: 0, y: 0, active: false };
    tooltipVisibleRef.current = false;
    setTooltipNodeIdState(null);
    onNodeHover?.(null);
    if (dragRef.current) {
      const node = posMapRef.current.get(dragRef.current!.nodeId);
      if (node) {
        node.fixed = false;
        const worker = workerRef.current;
        if (worker) {
          worker.postMessage({
            type: "update",
            payload: {
              nodeIdx: node.idx,
              x: node.x,
              y: node.y,
              fixed: false,
              vx: 0,
              vy: 0,
            },
          } as WorkerMessage);
        }
      }
      dragRef.current = null;
    }
    panRef.current = null;
  }, [onNodeHover]);

  const handleWheel = useCallback((e: React.WheelEvent<HTMLCanvasElement>) => {
    // 注意：React 的 onWheel 是 passive 事件，不能调用 preventDefault
    // 阻止默认滚动已通过原生非被动监听实现（见 useEffect 中的 wheel 监听）
    const rect = canvasRef.current!.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;

    const zoomFactor = e.deltaY < 0 ? 1.1 : 0.9;
    const oldZoom = cameraRef.current.zoom;
    const newZoom = Math.max(0.05, Math.min(5, oldZoom * zoomFactor));
    // 用户亲手缩放 ⇒ 视角归用户，聚合布局的自动对齐（applyAggregateLayout）就此停手
    cameraTouchedByUserRef.current = true;

    // 缩放以鼠标位置为中心
    const worldBefore = getScreenToWorld(sx, sy);
    cameraRef.current.zoom = newZoom;
    const worldAfter = getScreenToWorld(sx, sy);
    cameraRef.current.x += (worldAfter.x - worldBefore.x) * newZoom;
    cameraRef.current.y += (worldAfter.y - worldBefore.y) * newZoom;
  }, [dimensions]);

  const handleDoubleClick = useCallback((e: ReactMouseEvent<HTMLCanvasElement>) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const nodeId = findNodeAt(sx, sy);
    if (nodeId) {
      suppressAutoFocusRef.current = true;
      onNodeDoubleClick?.(nodeId);
    }
  }, [findNodeAt, onNodeDoubleClick]);

  const handleContextMenu = useCallback((e: ReactMouseEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    const rect = canvasRef.current!.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const nodeId = findNodeAt(sx, sy);
    if (nodeId) {
      suppressAutoFocusRef.current = true;
      onContextMenu?.(nodeId, { x: e.clientX, y: e.clientY });
    }
  }, [findNodeAt, onContextMenu]);

  // 原生非被动 wheel 监听：React 的 onWheel 为被动模式，preventDefault 无效
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) { return; }
    const preventWheel = (e: WheelEvent) => {
      e.preventDefault();
    };
    canvas.addEventListener("wheel", preventWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", preventWheel);
  }, []);

  // 原生非被动 touchmove 监听：React 的 onTouchMove 为被动模式，preventDefault 无效
  // 阻止触摸滚动，让画布可以处理拖拽和缩放手势
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) { return; }
    const preventTouchMove = (e: TouchEvent) => {
      e.preventDefault();
    };
    canvas.addEventListener("touchmove", preventTouchMove, { passive: false });
    return () => canvas.removeEventListener("touchmove", preventTouchMove);
  }, []);

  // ── 触摸事件处理 ──
  const touchStateRef = useRef<{
    lastDist?: number;
    startX?: number;
    startY?: number;
    camX?: number;
    camY?: number;
  }>({});

  // N7 修复：移动端长按 500ms 触发上下文菜单（等价桌面端右键 onContextMenu）
  const longPressTimerRef = useRef<number | null>(null);
  const cancelLongPress = useCallback(() => {
    if (longPressTimerRef.current !== null) {
      window.clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }, []);
  // 组件卸载时清理未触发的长按定时器
  useEffect(() => cancelLongPress, [cancelLongPress]);

  const handleTouchStart = useCallback((e: React.TouchEvent<HTMLCanvasElement>) => {
    if (e.touches.length === 1) {
      const touch = e.touches[0];
      const rect = canvasRef.current!.getBoundingClientRect();
      const sx = touch.clientX - rect.left;
      const sy = touch.clientY - rect.top;
      const nodeId = findNodeAt(sx, sy);

      if (nodeId) {
        suppressAutoFocusRef.current = true;
        const node = posMapRef.current.get(nodeId);
        if (node) {
          node.fixed = true;
          dragRef.current = { nodeId };
          onNodeClick?.(nodeId);
        }
      } else {
        panRef.current = {
          startX: touch.clientX,
          startY: touch.clientY,
          camX: cameraRef.current.x,
          camY: cameraRef.current.y,
        };
        onDeselect?.();
      }

      // 记录触摸起始位置，用于长按检测
      touchStateRef.current.startX = touch.clientX;
      touchStateRef.current.startY = touch.clientY;
      touchStateRef.current.camX = cameraRef.current.x;
      touchStateRef.current.camY = cameraRef.current.y;

      // 长按 500ms 后在起始位置检测节点并呼出上下文菜单；
      // 期间移动超过阈值或抬起/第二根手指按下都会取消（见 move/end 处理）
      cancelLongPress();
      longPressTimerRef.current = window.setTimeout(() => {
        longPressTimerRef.current = null;
        const st = touchStateRef.current;
        if (st.startX === undefined || st.startY === undefined) { return; }
        const lpRect = canvasRef.current?.getBoundingClientRect();
        if (!lpRect) { return; }
        const lpNodeId = findNodeAt(st.startX - lpRect.left, st.startY - lpRect.top);
        if (!lpNodeId) { return; }
        suppressAutoFocusRef.current = true;
        // 长按呼出菜单后结束按住拖拽状态，避免节点悬挂在 fixed 状态
        if (dragRef.current) {
          const dragNode = posMapRef.current.get(dragRef.current.nodeId);
          if (dragNode) {
            dragNode.fixed = false;
            dragNode.fx = 0;
            dragNode.fy = 0;
          }
          dragRef.current = null;
        }
        onContextMenu?.(lpNodeId, { x: st.startX, y: st.startY });
      }, 500);
    } else if (e.touches.length === 2) {
      // 双指缩放
      cancelLongPress();
      const t1 = e.touches[0];
      const t2 = e.touches[1];
      const dx = t1.clientX - t2.clientX;
      const dy = t1.clientY - t2.clientY;
      touchStateRef.current.lastDist = Math.sqrt(dx * dx + dy * dy);
      dragRef.current = null;
      panRef.current = null;
    }
  }, [findNodeAt, onNodeClick, onDeselect, onContextMenu, cancelLongPress]);

  const handleTouchMove = useCallback((e: React.TouchEvent<HTMLCanvasElement>) => {
    // 注意：React 的 onTouchMove 是 passive 事件，不能调用 preventDefault
    // 阻止默认滚动已通过原生非被动监听实现（见 useEffect 中的 touchmove 监听）

    if (e.touches.length === 1) {
      const touch = e.touches[0];
      const rect = canvasRef.current!.getBoundingClientRect();
      const sx = touch.clientX - rect.left;
      const sy = touch.clientY - rect.top;

      mouseScreenRef.current = { x: sx, y: sy, active: true };

      // 移动超过阈值取消长按（视为拖拽/平移手势）
      if (touchStateRef.current.startX !== undefined) {
        const movedX = Math.abs(touch.clientX - touchStateRef.current.startX);
        const movedY = Math.abs(touch.clientY - (touchStateRef.current.startY ?? 0));
        if (movedX > 10 || movedY > 10) { cancelLongPress(); }
      }
      if (dragRef.current) {
        const world = getScreenToWorld(sx, sy);
        const node = posMapRef.current.get(dragRef.current!.nodeId);
        if (node) {
          node.x = world.x;
          node.y = world.y;
          node.vx = 0;
          node.vy = 0;
        }
      } else if (panRef.current) {
        const dx = touch.clientX - panRef.current.startX;
        const dy = touch.clientY - panRef.current.startY;
        cameraRef.current.x = panRef.current.camX + dx;
        cameraRef.current.y = panRef.current.camY + dy;
        cameraTouchedByUserRef.current = true; // 用户手势平移 ⇒ 停掉聚合布局的自动对齐
      }
    } else if (e.touches.length === 2) {
      // 双指缩放
      cancelLongPress();
      const t1 = e.touches[0];
      const t2 = e.touches[1];
      const dx = t1.clientX - t2.clientX;
      const dy = t1.clientY - t2.clientY;
      const dist = Math.sqrt(dx * dx + dy * dy);

      if (touchStateRef.current.lastDist) {
        const scale = dist / touchStateRef.current.lastDist;
        const oldZoom = cameraRef.current.zoom;
        const newZoom = Math.max(0.05, Math.min(5, oldZoom * scale));

        const rect = canvasRef.current!.getBoundingClientRect();
        const centerX = (t1.clientX + t2.clientX) / 2 - rect.left;
        const centerY = (t1.clientY + t2.clientY) / 2 - rect.top;

        const worldBefore = getScreenToWorld(centerX, centerY);
        cameraRef.current.zoom = newZoom;
        cameraTouchedByUserRef.current = true; // 用户手势 ⇒ 停掉聚合布局的自动对齐
        const worldAfter = getScreenToWorld(centerX, centerY);
        cameraRef.current.x += (worldAfter.x - worldBefore.x) * newZoom;
        cameraRef.current.y += (worldAfter.y - worldBefore.y) * newZoom;
      }

      touchStateRef.current.lastDist = dist;
    }
  }, [cancelLongPress]);

  const handleTouchEnd = useCallback((e: React.TouchEvent<HTMLCanvasElement>) => {
    cancelLongPress();
    if (dragRef.current) {
      const node = posMapRef.current.get(dragRef.current!.nodeId);
      if (node) {
        node.fixed = false;
        node.fx = 0;
        node.fy = 0;
      }
      dragRef.current = null;

      // 拖拽结束后保存布局
      if (wikiIdRef.current) {
        saveLayout(wikiIdRef.current, physNodesRef.current, cameraRef.current);
      }
    }
    panRef.current = null;
    touchStateRef.current.lastDist = undefined;

    // 触摸结束后检测是否为点击（移动距离小于阈值）
    if (e.changedTouches.length === 1 && touchStateRef.current.startX !== undefined) {
      const touch = e.changedTouches[0];
      const movedX = Math.abs(touch.clientX - touchStateRef.current.startX);
      const movedY = Math.abs(touch.clientY - (touchStateRef.current.startY ?? 0));
      if (movedX < 5 && movedY < 5) {
        // 这是一次点击，已在 touchstart 中处理
      }
    }
  }, [cancelLongPress]);

  // 键盘导航 + 删除（带确认）
  const pendingDeleteRef = useRef<string | null>(null);
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const isInputFocused = target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable;

      // 空格键：聚焦画布
      if (e.key === " " && !isInputFocused) {
        e.preventDefault();
        containerRef.current?.focus();
      }

      if (e.key === "Escape") {
        pendingDeleteRef.current = null;
        onDeselect?.();
      }

      // 方向键平移视图
      if (!isInputFocused) {
        const panStep = 50 / cameraRef.current.zoom;
        const panSpeed = e.shiftKey ? panStep * 2 : panStep;

        // 视角类快捷键（方向键平移 / +- 缩放 / 0 复位 / f 聚焦）一旦按下，视角就归用户：
        // 聚合布局的自动对齐（applyAggregateLayout）就此停手 —— 与滚轮、拖拽平移同一判据，
        // 汇聚在此处而不是逐个 case 里打标记，避免以后新增快捷键时漏标。
        if (
          ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "+", "=", "-", "_", "0", "f", "F"]
            .includes(e.key)
        ) {
          cameraTouchedByUserRef.current = true;
        }

        switch (e.key) {
          case "ArrowUp":
            e.preventDefault();
            cameraRef.current.y += panSpeed;
            break;
          case "ArrowDown":
            e.preventDefault();
            cameraRef.current.y -= panSpeed;
            break;
          case "ArrowLeft":
            e.preventDefault();
            cameraRef.current.x += panSpeed;
            break;
          case "ArrowRight":
            e.preventDefault();
            cameraRef.current.x -= panSpeed;
            break;
          case "+":
          case "=":
            e.preventDefault();
            cameraRef.current.zoom = Math.min(5, cameraRef.current.zoom * 1.2);
            break;
          case "-":
          case "_":
            e.preventDefault();
            cameraRef.current.zoom = Math.max(0.05, cameraRef.current.zoom / 1.2);
            break;
          case "0":
            e.preventDefault();
            cameraRef.current.zoom = 1;
            cameraRef.current.x = 0;
            cameraRef.current.y = 0;
            break;
          case "f":
          case "F":
            // 聚焦选中节点
            if (selectedNodeIdRef.current) {
              const node = posMapRef.current.get(selectedNodeIdRef.current);
              if (node) {
                const targetZoom = Math.max(cameraRef.current.zoom, 1.5);
                cameraRef.current.x = -node.x * targetZoom;
                cameraRef.current.y = -node.y * targetZoom;
                cameraRef.current.zoom = targetZoom;
              }
            }
            break;
          case "h":
          case "H":
            // 切换鱼眼模式（同步 state 使工具栏按钮状态一致）
            fisheyeEnabledRef.current = !fisheyeEnabledRef.current;
            setFisheyeEnabled(fisheyeEnabledRef.current);
            break;
          case "l":
          case "L":
            // 切换聚类模式（与工具栏 ◈ 按钮共用同一入口，避免两条路径语义漂移）
            toggleClusterMode();
            break;
          case "p":
          case "P":
            // 切换粒子流动（默认关闭；同步 state 使工具栏按钮状态一致）
            particlesEnabledRef.current = !particlesEnabledRef.current;
            setParticlesEnabled(particlesEnabledRef.current);
            break;
        }
      }

      // Delete/Backspace 删除（需二次确认）
      if ((e.key === "Delete" || e.key === "Backspace") && selectedNodeIdRef.current && !isInputFocused) {
        // 阻止 Backspace 在浏览器中触发"返回上一页"，避免误操作离开图谱页
        e.preventDefault();
        const nodeId = selectedNodeIdRef.current;
        if (pendingDeleteRef.current === nodeId) {
          pendingDeleteRef.current = null;
          onDeleteNode?.(nodeId);
        } else {
          pendingDeleteRef.current = nodeId;
          setTimeout(() => {
            if (pendingDeleteRef.current === nodeId) {
              pendingDeleteRef.current = null;
            }
          }, 1500);
        }
      }
    };
    const el = containerRef.current;
    el?.addEventListener("keydown", handleKey);
    return () => el?.removeEventListener("keydown", handleKey);
  }, [onDeleteNode, onDeselect, toggleClusterMode]);

  useEffect(() => {
    const handle = () => {
      minimapDragRef.current = false;
    };
    window.addEventListener("mouseup", handle);
    return () => window.removeEventListener("mouseup", handle);
  }, []);

  // ── 工具栏操作 ──

  const handleZoomIn = useCallback(() => {
    cameraRef.current.zoom = Math.min(5, cameraRef.current.zoom * 1.2);
    cameraTouchedByUserRef.current = true; // 用户主动缩放 ⇒ 停掉聚合布局的自动对齐
  }, []);
  const handleZoomOut = useCallback(() => {
    cameraRef.current.zoom = Math.max(0.05, cameraRef.current.zoom / 1.2);
    cameraTouchedByUserRef.current = true;
  }, []);
  const handleFitAll = useCallback(() => {
    const nodes = physNodesRef.current;
    if (nodes.length === 0) { return; }
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    // cluster mode 下折叠节点的位置仍是原始坐标（远离聚合质心），
    // 若参与包围盒会导致 fitAll 后聚合节点挤在角落；
    // 此模式下用聚合几何 + 未折叠节点计算包围盒
    const clusterGeoms = clusterGeomRef.current;
    const collapsed = collapsedRef.current;
    const communitiesMap = communitiesRef.current;
    const isClusterActive = clusterModeRef.current && communitiesMap && collapsed.size > 0;
    for (const n of nodes) {
      if (isClusterActive && communitiesMap) {
        const cid = communitiesMap.get(n.id);
        if (cid !== undefined && collapsed.has(cid)) {
          continue; // 折叠节点不参与包围盒
        }
      }
      if (n.x < minX) { minX = n.x; }
      if (n.y < minY) { minY = n.y; }
      if (n.x > maxX) { maxX = n.x; }
      if (n.y > maxY) { maxY = n.y; }
    }
    // 加入聚合节点的包围盒
    if (isClusterActive) {
      for (const [, geom] of clusterGeoms) {
        if (geom.cx < minX) { minX = geom.cx; }
        if (geom.cy < minY) { minY = geom.cy; }
        if (geom.cx > maxX) { maxX = geom.cx; }
        if (geom.cy > maxY) { maxY = geom.cy; }
      }
    }
    if (!isFinite(minX)) { return; }
    const bboxW = maxX - minX;
    const bboxH = maxY - minY;
    const targetZoom = Math.min(
      (dimensions.width * 0.8) / Math.max(bboxW, 1),
      (dimensions.height * 0.8) / Math.max(bboxH, 1),
      2,
    );
    cameraRef.current.x = -(minX + maxX) / 2 * targetZoom;
    cameraRef.current.y = -(minY + maxY) / 2 * targetZoom;
    cameraRef.current.zoom = targetZoom;
  }, [dimensions]);
  // 暴露给 Worker ready 回调（该回调在组件前段的 effect 中，拿不到此处定义的闭包）
  useEffect(() => {
    fitAllRef.current = handleFitAll;
  }, [handleFitAll]);
  const handleFullscreenToggle = useCallback(() => {
    if (isFullscreen) {
      document.exitFullscreen();
    } else {
      containerRef.current?.requestFullscreen();
    }
  }, [isFullscreen]);
  const handleExportPNG = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) { return; }
    const link = document.createElement("a");
    link.download = `wiki-graph-${Date.now()}.png`;
    link.href = canvas.toDataURL("image/png");
    link.click();
  }, []);

  const handleExportHD = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) { return; }
    // 高清导出：2x 分辨率
    const scale = 2;
    const hdCanvas = document.createElement("canvas");
    hdCanvas.width = canvas.width * scale;
    hdCanvas.height = canvas.height * scale;
    const ctx = hdCanvas.getContext("2d");
    if (!ctx) { return; }
    ctx.scale(scale, scale);
    ctx.drawImage(canvas, 0, 0);
    const link = document.createElement("a");
    link.download = `wiki-graph-hd-${Date.now()}.png`;
    link.href = hdCanvas.toDataURL("image/png");
    link.click();
  }, []);

  const handleExportSVG = useCallback(() => {
    const nodes = physNodesRef.current;
    const edges = physEdgesRef.current;
    const nodeMeta = nodeMetaRef.current;
    const colorCache = nodeColorRef.current;
    const sizeCache = nodeSizeRef.current;
    const visibleTypes = visibleEdgeTypesRef.current;

    if (nodes.length === 0) { return; }

    // 计算边界框
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const n of nodes) {
      if (n.x < minX) { minX = n.x; }
      if (n.y < minY) { minY = n.y; }
      if (n.x > maxX) { maxX = n.x; }
      if (n.y > maxY) { maxY = n.y; }
    }

    const padding = 50;
    const viewBoxW = maxX - minX + padding * 2;
    const viewBoxH = maxY - minY + padding * 2;
    const offsetX = -minX + padding;
    const offsetY = -minY + padding;

    const svgParts: string[] = [];
    svgParts.push(`<?xml version="1.0" encoding="UTF-8"?>`);
    svgParts.push(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${viewBoxW} ${viewBoxH}" width="${viewBoxW}" height="${viewBoxH}">`,
    );
    svgParts.push(`<rect width="100%" height="100%" fill="${escapeXml(token.colorBgContainer)}"/>`);

    // 绘制边
    for (let i = 0; i < edges.length; i++) {
      const em = edgeMetaRef.current[i];
      if (!em || !visibleTypes.has(em.type)) { continue; }
      const s = nodes[em.sourceIdx];
      const t = nodes[em.targetIdx];
      if (!s || !t) { continue; }
      const x1 = s.x + offsetX;
      const y1 = s.y + offsetY;
      const x2 = t.x + offsetX;
      const y2 = t.y + offsetY;
      svgParts.push(
        `<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="${
          escapeXml(em.color)
        }" stroke-width="${em.width}" opacity="0.7"/>`,
      );
    }

    // 绘制节点
    for (const node of nodes) {
      const meta = nodeMeta.get(node.id);
      if (!meta) { continue; }
      const color = colorCache.get(node.id) || token.colorPrimary;
      const size = sizeCache.get(node.id) || 6;
      const cx = node.x + offsetX;
      const cy = node.y + offsetY;
      svgParts.push(`<circle cx="${cx}" cy="${cy}" r="${size}" fill="${escapeXml(color)}" opacity="0.9"/>`);
      // 标签
      const label = meta.title.length > 20 ? meta.title.slice(0, 18) + "…" : meta.title;
      svgParts.push(
        `<text x="${cx}" y="${cy + size + 12}" text-anchor="middle" font-size="10" fill="${
          escapeXml(token.colorText)
        }" font-family="Inter, system-ui, sans-serif">${escapeXml(label)}</text>`,
      );
    }

    svgParts.push(`</svg>`);

    const svgBlob = new Blob([svgParts.join("\n")], { type: "image/svg+xml" });
    const url = URL.createObjectURL(svgBlob);
    const link = document.createElement("a");
    link.download = `wiki-graph-${Date.now()}.svg`;
    link.href = url;
    link.click();
    URL.revokeObjectURL(url);
  }, [token]);
  const handleRelaunchLayout = useCallback(() => {
    const nodes = physNodesRef.current;

    // 清除已保存的布局缓存
    if (wikiIdRef.current) {
      clearLayout(wikiIdRef.current);
    }

    initializePositions(nodes, dimensions.width, dimensions.height);

    // 集群力模式下，重置时同步社区质心，Worker step 会据此收敛
    const activeCommunities = effectiveCommunitiesRef.current ?? communities;
    const enableClusters = clusterModeRef.current && activeCommunities;
    const centroids = enableClusters
      ? computeCommunityCentroids(nodes, activeCommunities!)
      : undefined;
    if (enableClusters) {
      communityCentroidsRef.current = centroids!;
    }

    // 同步新布局到 Worker（避免主线程同步跑 Barnes-Hut 冻结 UI）
    const worker = workerRef.current;
    if (worker && workerInitializedRef.current) {
      const positions = new Float64Array(nodes.length * 2);
      for (let i = 0; i < nodes.length; i++) {
        positions[i * 2] = nodes[i].x;
        positions[i * 2 + 1] = nodes[i].y;
      }
      worker.postMessage({ type: "reset", payload: { positions } } as WorkerMessage);
      pendingStepRef.current = false;
    } // Worker 未就绪时：主线程短暂收敛（仅小图，避免大图卡顿——大图 Worker 几乎总是就绪）
    else if (nodes.length <= 8000) {
      const config: PhysicsConfig = {
        theta: 0.5,
        repulsion: 18000,
        gravity: 0.003,
        damping: 0.82,
        dt: 0.35,
        springForce: 0.08,
        springDamping: 0.85,
        maxVelocity: 8,
        clusterForce: enableClusters ? 0.15 : undefined,
      };
      for (let i = 0; i < 30; i++) {
        stepPhysics(
          nodes,
          physEdgesRef.current,
          config,
          undefined,
          enableClusters ? communities : undefined,
          centroids,
        );
      }
    }

    // 保存新布局
    if (wikiIdRef.current) {
      saveLayout(wikiIdRef.current, nodes);
    }

    const gridIndex = new Map<string, string[]>();
    for (const n of nodes) {
      const gx = Math.floor(n.x / GRID_CELL_SIZE);
      const gy = Math.floor(n.y / GRID_CELL_SIZE);
      const key = `${gx},${gy}`;
      const bucket = gridIndex.get(key);
      if (bucket) {
        bucket.push(n.id);
      } else {
        gridIndex.set(key, [n.id]);
      }
    }
    gridIndexRef.current = gridIndex;
  }, [dimensions, communities]);

  const focusOnNode = useCallback((nodeId: string) => {
    const node = posMapRef.current.get(nodeId);
    if (!node) { return; }

    const cam = cameraRef.current;
    const targetZoom = Math.max(cam.zoom, 1.5);
    const targetX = -node.x * targetZoom;
    const targetY = -node.y * targetZoom;

    const startX = cam.x;
    const startY = cam.y;
    const startZoom = cam.zoom;
    const duration = 400;
    const startTime = performance.now();

    const animate = (now: number) => {
      const elapsed = now - startTime;
      const t = Math.min(elapsed / duration, 1);
      const ease = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;

      cam.x = startX + (targetX - startX) * ease;
      cam.y = startY + (targetY - startY) * ease;
      cam.zoom = startZoom + (targetZoom - startZoom) * ease;

      if (t < 1) {
        requestAnimationFrame(animate);
      }
    };
    requestAnimationFrame(animate);
  }, []);

  useImperativeHandle(ref, () => ({
    focusOnNode,
  }), [focusOnNode]);

  // ── 渲染 UI ──

  if (data.nodes.length === 0) {
    return (
      <Card
        style={{
          height: "100%",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          borderRadius: 12,
          background: `linear-gradient(135deg, ${token.colorBgContainer}08, ${token.colorBgContainer}15)`,
          border: `1px solid ${token.colorBorderSecondary}30`,
        }}
      >
        <Empty description={t("wiki.graph.empty")} />
      </Card>
    );
  }

  const ctrlBtnStyle: CSSProperties = {
    width: 26,
    height: 26,
    minWidth: 26,
    padding: 0,
    borderRadius: 7,
    background: `${token.colorBgContainer}e6`,
    backdropFilter: "blur(8px)",
    border: `1px solid ${token.colorBorderSecondary}30`,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    transition: "all 0.15s ease",
  };

  const hoverBtnStyle = (e: ReactMouseEvent) => {
    const el = e.currentTarget as HTMLElement;
    el.style.background = token.colorBgTextHover;
    el.style.transform = "scale(1.05)";
  };
  const leaveBtnStyle = (e: ReactMouseEvent) => {
    const el = e.currentTarget as HTMLElement;
    el.style.background = `${token.colorBgContainer}e6`;
    el.style.transform = "scale(1)";
  };

  const nodeCount = data.nodes.length;
  const edgeCount = data.edges.length;

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      className="outline-none focus-visible:outline-2 focus-visible:outline-offset-2"
      style={{ width: "100%", height: "100%", position: "relative" }}
    >
      <canvas
        ref={canvasRef}
        role="application"
        aria-label={t("wiki.graph.canvasAriaLabel")}
        style={{
          display: "block",
          width: "100%",
          height: "100%",
          cursor: dragRef.current ? "grabbing" : "grab",
          touchAction: "none",
        }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseLeave}
        onDoubleClick={handleDoubleClick}
        onContextMenu={handleContextMenu}
        onWheel={handleWheel}
        onTouchStart={handleTouchStart}
        onTouchMove={handleTouchMove}
        onTouchEnd={handleTouchEnd}
      />

      {/* 左上角：筛选 */}
      <div style={{ position: "absolute", top: 10, left: 10, zIndex: 10 }}>
        <Popover
          open={legendOpen}
          onOpenChange={setLegendOpen}
          trigger="click"
          placement="bottomLeft"
          arrow={false}
          styles={{ root: { width: 280 }, container: { padding: "12px 14px" } }}
          content={
            <div style={{ display: "flex", flexDirection: "column", gap: 8, fontSize: 11 }}>
              <div style={{ color: token.colorTextSecondary, fontSize: 11, marginBottom: 4 }}>
                {t("wiki.graph.edgeTypes")}
              </div>
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                {(Object.keys(edgeTypeLabels) as GraphEdgeType[]).map((et) => {
                  const isVisible = visibleEdgeTypes.has(et);
                  const style = getEdgeTypeStylesMap(token)[et];
                  return (
                    <button
                      key={et}
                      onClick={() => toggleEdgeType(et)}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 4,
                        padding: "2px 6px",
                        borderRadius: 4,
                        border: `1px solid ${isVisible ? style.color : token.colorBorderSecondary}`,
                        background: isVisible ? `${style.color}15` : "transparent",
                        cursor: "pointer",
                        opacity: isVisible ? 1 : 0.5,
                        transition: "opacity 0.15s",
                        fontSize: 11,
                        color: isVisible ? style.color : token.colorTextSecondary,
                      }}
                    >
                      <svg width="20" height="8">
                        <line
                          x1="0"
                          y1="4"
                          x2="20"
                          y2="4"
                          stroke={isVisible ? style.color : token.colorBorderSecondary}
                          strokeWidth={style.width}
                        />
                      </svg>
                      <span>{t(edgeTypeLabels[et])}</span>
                    </button>
                  );
                })}
              </div>
              {/* 知识库关系类型分布（P2-c）——`edge.relationType` 的唯一展示出口 */}
              {relationLegend && (
                <>
                  <div
                    style={{
                      color: token.colorTextSecondary,
                      fontSize: 11,
                      marginBottom: 4,
                      marginTop: 8,
                      borderTop: `1px solid ${token.colorBorderSecondary}`,
                      paddingTop: 8,
                    }}
                  >
                    {t("wiki.graph.relationTypes", {
                      edges: relationLegend.edges,
                      types: relationLegend.types,
                    })}
                  </div>
                  <div
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      gap: 2,
                      maxHeight: 150,
                      overflowY: "auto",
                    }}
                  >
                    {relationLegend.top.map(([name, n]) => (
                      <div
                        key={name}
                        style={{
                          display: "flex",
                          justifyContent: "space-between",
                          gap: 8,
                          fontSize: 11,
                          color: token.colorTextSecondary,
                        }}
                      >
                        <span
                          title={name}
                          style={{
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {name}
                        </span>
                        <span style={{ color: token.colorTextTertiary, flexShrink: 0 }}>{n}</span>
                      </div>
                    ))}
                    {relationLegend.rest > 0 && (
                      <div style={{ fontSize: 11, color: token.colorTextTertiary, marginTop: 2 }}>
                        {t("wiki.graph.relationTypesMore", { count: relationLegend.rest })}
                      </div>
                    )}
                  </div>
                </>
              )}

              {/* 社区筛选 */}
              {communities && communities.size > 0 && (
                <>
                  <div style={{ color: token.colorTextSecondary, fontSize: 11, marginBottom: 4, marginTop: 8 }}>
                    {t("wiki.graph.communities")}
                  </div>
                  <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                    {(() => {
                      const uniqueCids = new Set<number>();
                      for (const cid of communities.values()) {
                        uniqueCids.add(cid);
                      }
                      return Array.from(uniqueCids).map((cid) => {
                        const isVisible = visibleCommunities.has(cid);
                        const color = communityPalette[cid % communityPalette.length];
                        return (
                          <button
                            key={cid}
                            onClick={() => toggleCommunity(cid)}
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: 4,
                              padding: "2px 6px",
                              borderRadius: 4,
                              border: `1px solid ${isVisible ? color : token.colorBorderSecondary}`,
                              background: isVisible ? `${color}15` : "transparent",
                              cursor: "pointer",
                              opacity: isVisible ? 1 : 0.5,
                              transition: "opacity 0.15s",
                              fontSize: 11,
                              color: isVisible ? color : token.colorTextSecondary,
                            }}
                          >
                            <span
                              style={{
                                width: 8,
                                height: 8,
                                borderRadius: "50%",
                                background: isVisible ? color : token.colorBorderSecondary,
                              }}
                            />
                            <span>{t("wiki.graph.clusterLabel", { id: cid })}</span>
                          </button>
                        );
                      });
                    })()}
                  </div>
                </>
              )}
            </div>
          }
        >
          <Button
            size="small"
            type="text"
            icon={<SlidersHorizontal size={13} />}
            style={ctrlBtnStyle}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
            title={t("wiki.graph.legend")}
          />
        </Popover>
      </div>

      {/* 右上角：统计 */}
      <div style={{ position: "absolute", top: 10, right: 10, zIndex: 10 }}>
        <Popover
          open={statsOpen}
          onOpenChange={setStatsOpen}
          trigger="click"
          placement="bottomRight"
          arrow={false}
          styles={{ root: { width: 180 }, container: { padding: "10px 14px" } }}
          content={
            <div style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
              <Typography.Text type="secondary" style={{ fontSize: 11 }}>{t("wiki.graph.stats")}</Typography.Text>
              <span>{t("wiki.graph.nodes")}: {nodeCount}</span>
              <span>{t("wiki.graph.edges")}: {edgeCount}</span>
              <span>
                Zoom: <span ref={statsZoomTextRef}>{cameraRef.current.zoom.toFixed(2)}×</span>
              </span>
            </div>
          }
        >
          <Button
            size="small"
            type="text"
            style={{ ...ctrlBtnStyle, fontSize: 10, fontWeight: 600, color: token.colorTextSecondary }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
            title={t("wiki.graph.stats")}
          >
            {nodeCount}
          </Button>
        </Popover>
      </div>

      {/* 底部中央：工具栏 */}
      <div
        style={{
          position: "absolute",
          bottom: 10,
          left: "50%",
          transform: "translateX(-50%)",
          zIndex: 10,
          display: "flex",
          alignItems: "center",
          gap: 2,
          padding: "3px 8px",
          borderRadius: 16,
          background: `${token.colorBgContainer}f0`,
          backdropFilter: "blur(16px)",
          border: `1px solid ${token.colorBorderSecondary}30`,
          boxShadow: `0 2px 8px ${token.colorBgMask}20`,
        }}
      >
        <Tooltip title={t("wiki.graph.zoomIn")}>
          <button
            onClick={handleZoomIn}
            style={{ ...ctrlBtnStyle, width: 24, height: 24, minWidth: 24, background: "transparent", border: "none" }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            <ZoomIn size={14} />
          </button>
        </Tooltip>
        <Tooltip title={t("wiki.graph.zoomOut")}>
          <button
            onClick={handleZoomOut}
            style={{ ...ctrlBtnStyle, width: 24, height: 24, minWidth: 24, background: "transparent", border: "none" }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            <ZoomOut size={14} />
          </button>
        </Tooltip>
        <Tooltip title={t("wiki.graph.fitView")}>
          <button
            onClick={handleFitAll}
            style={{ ...ctrlBtnStyle, width: 24, height: 24, minWidth: 24, background: "transparent", border: "none" }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            <Maximize2 size={14} />
          </button>
        </Tooltip>
        <div style={{ width: 1, height: 14, background: token.colorBorderSecondary, margin: "0 2px" }} />
        {/* 鱼眼放大镜 toggle */}
        <Tooltip title={fisheyeEnabled ? t("wiki.graph.fisheyeOn") : t("wiki.graph.fisheyeOff")}>
          <button
            onClick={() => setFisheyeEnabled((v) => !v)}
            style={{
              ...ctrlBtnStyle,
              width: 24,
              height: 24,
              minWidth: 24,
              background: fisheyeEnabled ? `${token.colorPrimary}20` : "transparent",
              border: "none",
              color: fisheyeEnabled ? token.colorPrimary : token.colorTextSecondary,
            }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            <Eye size={14} />
          </button>
        </Tooltip>
        {/* 聚类模式 toggle */}
        <Tooltip title={clusterMode ? t("wiki.graph.clusterOff") : t("wiki.graph.clusterOn")}>
          <button
            onClick={toggleClusterMode}
            style={{
              ...ctrlBtnStyle,
              width: 24,
              height: 24,
              minWidth: 24,
              background: clusterMode ? `${token.colorPrimary}20` : "transparent",
              border: "none",
              color: clusterMode ? token.colorPrimary : token.colorTextSecondary,
              fontSize: 11,
              fontWeight: 700,
            }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            ◈
          </button>
        </Tooltip>
        {/* 粒子流动 toggle（默认关闭，对齐 Obsidian 静态细边） */}
        <Tooltip title={particlesEnabled ? t("wiki.graph.particlesOn") : t("wiki.graph.particlesOff")}>
          <button
            onClick={() => setParticlesEnabled((v) => !v)}
            style={{
              ...ctrlBtnStyle,
              width: 24,
              height: 24,
              minWidth: 24,
              background: particlesEnabled ? `${token.colorPrimary}20` : "transparent",
              border: "none",
              color: particlesEnabled ? token.colorPrimary : token.colorTextSecondary,
            }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            <Sparkles size={14} />
          </button>
        </Tooltip>
        <div style={{ width: 1, height: 14, background: token.colorBorderSecondary, margin: "0 2px" }} />
        <Tooltip title={t("wiki.graph.fullscreen")}>
          <button
            onClick={handleFullscreenToggle}
            style={{ ...ctrlBtnStyle, width: 24, height: 24, minWidth: 24, background: "transparent", border: "none" }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            <Fullscreen size={14} />
          </button>
        </Tooltip>
        <div style={{ width: 1, height: 14, background: token.colorBorderSecondary, margin: "0 2px" }} />
        {/* 导出下拉菜单 */}
        <Popover
          trigger="click"
          placement="top"
          arrow={false}
          styles={{ root: { width: 140 }, container: { padding: "4px" } }}
          content={
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              <button
                onClick={handleExportPNG}
                style={{
                  padding: "6px 12px",
                  background: "transparent",
                  border: "none",
                  borderRadius: 4,
                  cursor: "pointer",
                  fontSize: 12,
                  color: token.colorText,
                  textAlign: "left",
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = token.colorPrimaryBg;
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = "transparent";
                }}
              >
                {t("wiki.graph.exportPNG")}
              </button>
              <button
                onClick={handleExportHD}
                style={{
                  padding: "6px 12px",
                  background: "transparent",
                  border: "none",
                  borderRadius: 4,
                  cursor: "pointer",
                  fontSize: 12,
                  color: token.colorText,
                  textAlign: "left",
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = token.colorPrimaryBg;
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = "transparent";
                }}
              >
                {t("wiki.graph.exportHD")}
              </button>
              <button
                onClick={handleExportSVG}
                style={{
                  padding: "6px 12px",
                  background: "transparent",
                  border: "none",
                  borderRadius: 4,
                  cursor: "pointer",
                  fontSize: 12,
                  color: token.colorText,
                  textAlign: "left",
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = token.colorPrimaryBg;
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = "transparent";
                }}
              >
                {t("wiki.graph.exportSVG")}
              </button>
            </div>
          }
        >
          <Tooltip title={t("wiki.graph.exportPNG")}>
            <button
              style={{
                ...ctrlBtnStyle,
                width: 24,
                height: 24,
                minWidth: 24,
                background: "transparent",
                border: "none",
              }}
              onMouseEnter={hoverBtnStyle}
              onMouseLeave={leaveBtnStyle}
            >
              <Download size={14} />
            </button>
          </Tooltip>
        </Popover>
        <Tooltip title={t("wiki.graph.relayout")}>
          <button
            onClick={handleRelaunchLayout}
            style={{ ...ctrlBtnStyle, width: 24, height: 24, minWidth: 24, background: "transparent", border: "none" }}
            onMouseEnter={hoverBtnStyle}
            onMouseLeave={leaveBtnStyle}
          >
            <RefreshCw size={14} />
          </button>
        </Tooltip>
      </div>

      {/* Hover Tooltip — DOM ref 定位，内容用 React 渲染（仅节点变化时） */}
      <div
        ref={tooltipRef}
        style={{
          position: "absolute",
          zIndex: 20,
          pointerEvents: "none",
          maxWidth: 250,
          background: `${token.colorBgContainer}f5`,
          backdropFilter: "blur(12px)",
          border: `1px solid ${token.colorBorderSecondary}`,
          borderRadius: 10,
          padding: "10px 14px",
          boxShadow: `0 4px 16px ${token.colorBgMask}30`,
          transition: "left 0.06s ease-out, top 0.06s ease-out",
          display: "none",
        }}
      >
        {tooltipNodeIdState && (() => {
          const meta = nodeMetaRef.current.get(tooltipNodeIdState);
          if (!meta) { return null; }
          const nodeColor = nodeColorRef.current.get(tooltipNodeIdState) || token.colorPrimary;
          const communityId = getCommunityId(tooltipNodeIdState);
          return (
            <>
              {/* 标题 */}
              <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 6 }}>
                <span
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: "50%",
                    background: nodeColor,
                    boxShadow: `0 0 6px ${nodeColor}80`,
                    flexShrink: 0,
                  }}
                />
                <span
                  style={{
                    fontWeight: 600,
                    fontSize: 13,
                    color: token.colorText,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {meta.title}
                </span>
              </div>

              {/* 类型 + 社区 */}
              <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginBottom: 6 }}>
                <span
                  style={{
                    fontSize: 10,
                    padding: "1px 6px",
                    borderRadius: 4,
                    background: `${nodeColor}20`,
                    color: nodeColor,
                    fontWeight: 500,
                  }}
                >
                  {
                    /* `defaultValue` 不可省：后端 `node_type` 是**开放列**（D3 实测词汇表外还有
                      `doc` 等真实写入值）⇒ 缺 key 时若不兜底，i18next 会把 key 原文
                      （`wiki.graph.nodeType.doc`）当文案渲染出来。宁显示原始类型名。 */
                  }
                  {t(`wiki.graph.nodeType.${meta.type}`, {
                    defaultValue: meta.type,
                  })}
                </span>
                {communityId !== undefined && (
                  <span
                    style={{
                      fontSize: 10,
                      padding: "1px 6px",
                      borderRadius: 4,
                      background: `${communityPalette[communityId % communityPalette.length]}20`,
                      color: communityPalette[communityId % communityPalette.length],
                      fontWeight: 500,
                    }}
                  >
                    {t("wiki.graph.clusterLabel", { id: communityId })}
                  </span>
                )}
              </div>

              {/* 统计 */}
              <div style={{ display: "flex", gap: 10, fontSize: 11, color: token.colorTextSecondary, marginBottom: 6 }}>
                <span>{t("wiki.graph.linksCount", { count: meta.linkCount })}</span>
                <span>{t("wiki.graph.backlinksCount", { count: meta.backlinkCount })}</span>
                <span>{t("wiki.graph.totalDegree", { count: meta.linkCount + meta.backlinkCount })}</span>
              </div>

              {/* 路径 */}
              <div
                style={{
                  fontSize: 10,
                  color: token.colorTextTertiary,
                  wordBreak: "break-all",
                  maxHeight: 32,
                  overflow: "hidden",
                }}
              >
                {meta.path}
              </div>

              {/* 标签 */}
              {meta.tags.length > 0 && (
                <div style={{ display: "flex", gap: 3, flexWrap: "wrap", marginTop: 6 }}>
                  {meta.tags.slice(0, 5).map((tag) => (
                    <span
                      key={tag}
                      style={{
                        fontSize: 9,
                        padding: "0 4px",
                        borderRadius: 3,
                        background: token.colorFillSecondary,
                        color: token.colorTextSecondary,
                      }}
                    >
                      #{tag}
                    </span>
                  ))}
                  {meta.tags.length > 5 && (
                    <span style={{ fontSize: 9, color: token.colorTextTertiary }}>+{meta.tags.length - 5}</span>
                  )}
                </div>
              )}
            </>
          );
        })()}
      </div>

      {showMinimap && (
        <div
          style={{
            position: "absolute",
            bottom: 50,
            right: 10,
            zIndex: 10,
            display: "flex",
            flexDirection: "column",
            alignItems: "flex-end",
            gap: 4,
          }}
        >
          <button
            onClick={() => setMinimapOpen((v) => !v)}
            style={{
              ...ctrlBtnStyle,
              width: 22,
              height: 22,
              minWidth: 22,
              fontSize: 10,
              fontWeight: 700,
              color: token.colorTextSecondary,
              cursor: "pointer",
            }}
            title={minimapOpen ? t("wiki.graph.collapseMinimap") : t("wiki.graph.expandMinimap")}
            aria-label={minimapOpen ? t("wiki.graph.collapseMinimap") : t("wiki.graph.expandMinimap")}
          >
            {minimapOpen ? "▾" : "▴"}
          </button>
          {minimapOpen && (
            <canvas
              ref={minimapRef}
              width={MINIMAP_W}
              height={MINIMAP_H}
              role="application"
              aria-label={t("wiki.graph.minimapAriaLabel")}
              onMouseDown={handleMinimapMouseDown}
              onMouseMove={handleMinimapMouseMove}
              onMouseUp={handleMinimapMouseUp}
              style={{
                borderRadius: 8,
                background: `${token.colorBgContainer}f0`,
                backdropFilter: "blur(12px)",
                border: `1px solid ${token.colorBorderSecondary}`,
                boxShadow: `0 4px 16px ${token.colorBgMask}30`,
                cursor: minimapDragRef.current ? "grabbing" : "crosshair",
              }}
            />
          )}
        </div>
      )}
    </div>
  );
});

export const GraphView = memo(GraphViewInner);
export { GraphView as default };
