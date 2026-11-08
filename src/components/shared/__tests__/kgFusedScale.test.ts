// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 融合图（笔记 ∪ 知识图谱实体）在**聚合物理层**的规模不变量与归桶策略判据（2026-09-18）。
 *
 * 背景：2026-09-18 把 wiki「A股知识」的知识库绑定从空壳库 `f9b2b050` 改到
 * `lemonhu_knowledge_graph` 之后，融合图的节点从 24,288 涨到 **46,896**、边涨到 172,926，
 * 其中 **22,608 个实体节点在社区映射里没有任何条目** —— 因为 Louvain 只跑**笔记图**
 * （`wiki_graph_communities` 的输入是 `note::get_vault_graph`，不含实体），
 * 而融合是在它之后把实体并进来的。改绑前融合图 == 笔记图，社区 100% 覆盖，
 * 所以「近半数节点无社区」这个形态是**改绑才第一次出现**的。
 *
 * 于是必须钉死两类判据：
 *
 * ① **规模不变量**（漏了会静默退化成静态/空白）：
 *    · 聚合物理层节点数仍 ≤ `MAX_AGG_PHYS_NODES`（否则 `GraphView.tsx:1891`（`aggOver`）会
 *      放弃力导向、退化成静态显示，正是「大图看起来没反应」的形态）；
 *    · 聚合层里**不得**混入真实节点（真实节点会逐个参与物理 ⇒ 规模失控）；
 *    · 聚合边数 > 0（否则画面上无连线）。
 *
 * ② **归桶策略的布局质量**（漏了会「有数据但看不出结构」）：
 *    · 桶级聚合边密度必须留在改绑前的量级。判据不是审美，是本仓自己写下的
 *      `GraphView.tsx:1273-1276`：「密度逼近完全图（旧哈希分桶实测 82.69%）
 *      ⇒ 聚合节点受力趋同 ⇒ 力导向退化为均匀铺开」。
 *    · 无社区节点必须优先继承其 `mapping` 笔记的桶；hash 兜底一旦被走到要能**被观测**
 *      （`viaHash > 0`），不能静默退化。
 *
 * ⚠ 本文件**不是**组件级渲染测试：它测的是「抽取出来的纯函数层（`communityMerge` /
 *   `graphAggregate`）+ 组件内联的 `effectiveCommunities` 规则」这一组合。
 *   内联段只复刻了 3 条规则（且第 3 条已改为调用**真函数**，见下），
 *   断言也只落在不变量与策略对比上，不落在复刻的中间值上。
 */
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  assignMissingCommunities,
  type CommunityAnchor,
  countDistinctCommunities,
  mergeCommunitiesTopologically,
  mergeEntityCommunities,
} from "../communityMerge";
import { AGG_PHYSICS_CONFIG, buildAggregateGraph } from "../graphAggregate";
import type { PhysicsNode } from "../graphPhysics";
import { hashStringToInt } from "../graphViewUtils";

/** 组件的三个规模常量（GraphView.tsx:726 / :729 / :743）。此处**照抄**而非导出，见文件头警告。 */
const AUTO_CLUSTER_THRESHOLD = 3000;
const MAX_AGG_PHYS_NODES = 800;
const TARGET_CLUSTER_COUNT = 200;

interface Payload {
  nodes: Array<{ id: string; title?: string; type?: string }>;
  edges: Array<{ source: string; target: string; type?: string; relationType?: string }>;
  /** 后端缓存的社区映射：nodeId → cid。**只覆盖笔记 id**。 */
  communities: Record<string, number>;
  /**
   * 后端在**实体子图**上单独跑 Louvain 的结果：`entity:<id>` → cid（2026-09-18 起下发）。
   *
   * ⚠ 与 `communities` 是**两次独立运行** ⇒ 两侧 cid 值域会重合，
   * 消费方必须先经 `mergeEntityCommunities` 错开命名空间。
   * 可选：老 fixture / 后端旧缓存没有这个字段（此时实体节点只能靠 `mapping` 锚点兜底）。
   */
  entityCommunities?: Record<string, number>;
}

function toPhysics(payload: Payload): PhysicsNode[] {
  return payload.nodes.map((n, idx) => ({
    id: n.id,
    x: 0,
    y: 0,
    vx: 0,
    vy: 0,
    fx: 0,
    fy: 0,
    mass: 1,
    fixed: false,
    kind: n.id.startsWith("entity:") ? "entity" : "note",
    idx,
  }));
}

/** 融合层合成的 `mapping` 边 —— 归桶锚点的**唯一**来源（后端 `wiki.rs:1173-1192`）。 */
function mappingAnchors(edges: Payload["edges"]): CommunityAnchor[] {
  const anchors: CommunityAnchor[] = [];
  for (const e of edges) {
    if (e.type === "mapping") {
      anchors.push({ source: e.source, target: e.target });
    }
  }
  return anchors;
}

/**
 * 复刻 `GraphView.tsx:1242-1344` 的 `effectiveCommunities` 计算。
 *
 * 复刻了且仅复刻了三条规则：
 *   ① `pNodes.length > AUTO_CLUSTER_THRESHOLD` ⇒ 进入 forceCluster；
 *   ② 不同 cid 数 > `min(TARGET_CLUSTER_COUNT, MAX_AGG_PHYS_NODES)` ⇒ 调
 *      **真函数** `mergeCommunitiesTopologically`（不是复刻）；
 *   ③ 补全：**只在真的有缺桶节点时**调**真函数** `assignMissingCommunities`
 *      （锚点优先 + hash 兜底）。⚠ 这条在 2026-09-18 之前是复刻的 hash 逻辑，
 *      现在直接调生产函数 ⇒ 「测的对象」与「跑的对象」同一份。
 */
function effectiveCommunities(payload: Payload, pNodes: PhysicsNode[]) {
  const notes = new Map(Object.entries(payload.communities).map(([k, v]) => [k, Number(v)]));
  const entities = payload.entityCommunities
    ? new Map(Object.entries(payload.entityCommunities).map(([k, v]) => [k, Number(v)]))
    : undefined;
  // 与组件同源：笔记侧 ∪ 实体侧，**经真函数错开命名空间**（不是直接合并 —— 见其文件头）
  const source = mergeEntityCommunities(notes, entities) ?? notes;
  const forced = pNodes.length > AUTO_CLUSTER_THRESHOLD;
  const targetClusterCount = Math.min(TARGET_CLUSTER_COUNT, MAX_AGG_PHYS_NODES);
  let effective = source;
  let branch = "not-forced";

  if (forced) {
    const distinct = countDistinctCommunities(source);
    if (distinct > targetClusterCount) {
      effective = mergeCommunitiesTopologically({
        communities: source,
        edges: payload.edges,
        targetCount: targetClusterCount,
      }).communities;
      branch = "topological-merge";
    } else {
      branch = "as-is";
    }
    let hasMissing = false;
    for (const n of pNodes) {
      if (!effective.has(n.id)) {
        hasMissing = true;
        break;
      }
    }
    if (hasMissing) {
      effective = assignMissingCommunities({
        nodeIds: pNodes.map((n) => n.id),
        communities: effective,
        anchors: mappingAnchors(payload.edges),
      }).communities;
    }
  }

  const covered = pNodes.filter((n) => source.has(n.id)).length;
  // 笔记侧单独的覆盖数：用来对 fixture 自证「社区只覆盖笔记」这条**数据形态**，
  // 与 `covered`（合成后的实际覆盖）区分开 —— 两者在实体侧社区缺席时相等。
  const coveredByNotes = pNodes.filter((n) => notes.has(n.id)).length;
  return {
    effective,
    source,
    notes,
    branch,
    forced,
    covered,
    coveredByNotes,
    targetClusterCount,
  };
}

/** 按组件的隐式聚合路径建图：布局单元 = 全部（归并后的）社区。 */
function buildFusedAggregate(payload: Payload) {
  const pNodes = toPhysics(payload);
  const ec = effectiveCommunities(payload, pNodes);
  const layoutUnits = new Set(ec.effective.values());
  const agg = buildAggregateGraph({
    nodes: pNodes,
    edges: payload.edges,
    communities: ec.effective,
    layoutUnits,
    seedPhysics: { repulsion: AGG_PHYSICS_CONFIG.repulsion, gravity: AGG_PHYSICS_CONFIG.gravity },
  });
  const realNodes = agg.nodes.filter((n) => !n.id.startsWith("__agg__")).length;
  // 桶级图的密度：聚合边 / C(桶数, 2)。判据见文件头 ②。
  const pairs = layoutUnits.size < 2 ? 0 : (layoutUnits.size * (layoutUnits.size - 1)) / 2;
  return {
    ...ec,
    totalNodes: pNodes.length,
    totalEdges: payload.edges.length,
    buckets: layoutUnits.size,
    aggNodeCount: agg.nodes.length,
    aggEdgeCount: agg.edges.length,
    realNodesInAggregate: realNodes,
    density: pairs === 0 ? 0 : agg.edges.length / pairs,
  };
}

function assertInvariants(r: ReturnType<typeof buildFusedAggregate>) {
  // ① 每个节点在聚合前都必须有 cid（否则它会作为真实节点逐个进物理层）
  expect(r.realNodesInAggregate).toBe(0);
  // ② 聚合物理层节点数 ≤ MAX_AGG_PHYS_NODES —— 超了 GraphView.tsx:1891（`aggOver`）会放弃力导向
  expect(r.aggNodeCount).toBeLessThanOrEqual(MAX_AGG_PHYS_NODES);
  expect(r.buckets).toBeLessThanOrEqual(MAX_AGG_PHYS_NODES);
  // ③ 必须有边可画
  expect(r.aggEdgeCount).toBeGreaterThan(0);
  // ④ forceCluster 路径必须真的被走进（节点数远超阈值）
  expect(r.forced).toBe(true);
  // ⑤ 归并桶数落在目标值
  expect(r.buckets).toBeLessThanOrEqual(r.targetClusterCount);
}

/**
 * 真实载荷 fixture 的候选路径（按优先级）。
 *
 * ⚠ 必须列**候选**而不是写死一条：2026-09-18 的 ①「知识库二选一」那轮把
 * `kg-fused-payload-0918.json` 改名成了 `-before-delete-`、另出了一份
 * `-after-delete-`，而本文件写死的仍是**旧名** ⇒ `realFixturePresent` 静默变成 false，
 * 于是下面所有「真实 payload 下的口径自证」**一次都没跑过**，
 * 界面上却仍显示为「不跳过」（它确实不跳过，只是跑的是合成数据）。
 * 这类腐烂不报错、只降低覆盖 ⇒ 改成候选列表，并**打印实际用了哪一份**。
 */
const REAL_FIXTURE_CANDIDATES = [
  // ① 删库（f9b2b050）之后的实际产物：46,896 节点 / 172,650 边 —— 现状
  "output/verify-graph-2026-09-15/kg-fused-payload-after-delete-0918.json",
  // ① 之前：172,926 边。保留兼容；若命中它，口径自证里的边侧断言会按差异失败（刻意不兜）
  "output/verify-graph-2026-09-15/kg-fused-payload-0918.json",
].map((rel) => path.resolve(process.cwd(), rel));

const REAL_FIXTURE = REAL_FIXTURE_CANDIDATES.find((p) => fs.existsSync(p));
const realFixturePresent = REAL_FIXTURE !== undefined;
const realPayload: Payload = REAL_FIXTURE
  ? (JSON.parse(fs.readFileSync(REAL_FIXTURE, "utf8")) as Payload)
  : buildSyntheticFusedPayload();

describe("融合图聚合规模（真实 payload：改绑 lemonhu 之后的实际产物）", () => {
  it(`fixture ${realFixturePresent ? "存在 ⇒ 用真实数据断言" : "不存在 ⇒ 退回等规模合成数据（两者都断言，不跳过）"}`, () => {
    const r = buildFusedAggregate(realPayload);

    // 真实 fixture 下额外的口径自证（合成数据不含这些数字）
    if (realFixturePresent) {
      expect(r.totalNodes).toBe(46896);
      expect(r.coveredByNotes).toBe(24288); // 社区只覆盖笔记
      expect(r.totalNodes - r.coveredByNotes).toBe(22608); // 实体节点全部无社区
      // 该 fixture 是 ① 那轮的产物，**没有** `entityCommunities` 字段
      // ⇒ 合成步骤退化为恒等（返回笔记侧原引用）⇒ 两个覆盖数必须相等。
      // 这条同时把「mergeEntityCommunities 在实体侧缺席时不改变任何东西」钉在真实数据上。
      expect(r.covered).toBe(r.coveredByNotes);
      // 之前那版 fixture 把 `relation_type` 写进了 `type` ⇒ 与 DTO 契约相反。
      // 这条断言让「fixture 的 type 是渲染类别」这件事**可失败**，而不是靠注释。
      expect(new Set(realPayload.edges.map((e) => e.type))).toEqual(
        new Set(["link", "reference", "mapping"]),
      );
    } else {
      // 退回合成数据时必须留下痕迹：这条路径的判据口径与真机**不同源**
      // （合成数据的实体侧边是随机端点 ⇒ 桶级密度没有可比性）。
      // eslint-disable-next-line no-console -- 静默退化的信号必须可见
      console.warn(
        `[kgFusedScale] 未找到真实 fixture，已退回等规模合成数据；候选路径：\n  ${
          REAL_FIXTURE_CANDIDATES.join("\n  ")
        }`,
      );
    }

    assertInvariants(r);
    // eslint-disable-next-line no-console -- 规模判据是有量化意义的，打印出来供人工核对
    console.log(
      `[kgFusedScale] ${
        realFixturePresent ? `REAL(${path.basename(REAL_FIXTURE ?? "")})` : "SYNTH"
      } 节点 ${r.totalNodes}（有社区 ${r.covered} / 无社区 ${r.totalNodes - r.covered}）`
        + ` 边 ${r.totalEdges} 分支 ${r.branch} 桶 ${r.buckets}`
        + ` ⇒ 聚合节点 ${r.aggNodeCount} 聚合边 ${r.aggEdgeCount} 真实节点混入 ${r.realNodesInAggregate}`
        + ` 密度 ${(r.density * 100).toFixed(1)}%`,
    );
  });

  // ⚠ 本条现在测的是**兜底路径**：入参刻意只用笔记侧社区（= 后端未下发
  // `entityCommunities` 时的形态）。主路径（实体侧有自己的社区）在下面
  // 「实体侧单独算社区」那个 describe 里测。
  it("★ 兜底路径：实体侧社区缺席时，无社区节点**全部**由锚点（mapping 边）归位，hash 兜底一次都没走", () => {
    const pNodes = toPhysics(realPayload);
    const merged = mergeCommunitiesTopologically({
      communities: new Map(Object.entries(realPayload.communities).map(([k, v]) => [k, Number(v)])),
      edges: realPayload.edges,
      targetCount: TARGET_CLUSTER_COUNT,
    }).communities;
    const bucketIdsBefore = new Set(merged.values());
    // 实测锚点数：mapping 边数（后端按**节点**合成，非按名字去重 ⇒ 22,608 而非 22,606）
    const anchors = mappingAnchors(realPayload.edges);
    if (realFixturePresent) {
      expect(anchors.length).toBe(22608);
    }

    const outcome = assignMissingCommunities({
      nodeIds: pNodes.map((n) => n.id),
      communities: merged,
      anchors,
    });

    const missingCount = pNodes.filter((n) => !merged.has(n.id)).length;
    if (realFixturePresent) {
      expect(missingCount).toBe(22608);
      expect(outcome.assigned).toBe(missingCount);
      expect(outcome.viaAnchor).toBe(missingCount);
      // ⚠ 这条是**可失败**的：一旦锚点链断（mapping 边消失 / 社区缓存与节点集不同源），
      // 这里会变成正数 —— 那正是要被抓到的形态，而不是让布局悄悄变散。
      expect(outcome.viaHash).toBe(0);
    } else {
      expect(outcome.viaHash).toBe(0);
    }
    // 补全**不生成新桶**：所有被补的节点都落在原有桶 id 集合里
    expect(outcome.bucketCount).toBe(bucketIdsBefore.size);
    for (const [id, cid] of outcome.communities) {
      if (!merged.has(id)) {
        expect(bucketIdsBefore.has(cid)).toBe(true);
      }
    }
  });
});

describe("实体侧单独算社区：笔记侧 ∪ 实体侧，命名空间必须错开（2026-09-18）", () => {
  /**
   * 给合成 payload 补一份**实体侧**社区（模拟后端在实体子图上单独跑 Louvain 的结果）。
   *
   * 分区方式刻意取「按序号分块」：合成数据的实体关系边端点是**随机抽的**
   * （见 `buildSyntheticFusedPayload`）⇒ 它本来就没有真实拓扑可依，
   * 任何分区与它都不同源。所以本函数只用于验证**机制**
   * （命名空间互斥、覆盖、桶数上界），**不**用来衡量分区质量 ——
   * 后者只能在真实 payload 上测（见文件头与 `mergeEntityCommunities` 的说明）。
   */
  function withSyntheticEntityCommunities(payload: Payload, clusterCount = 900): Payload {
    const entities: Record<string, number> = {};
    let i = 0;
    for (const n of payload.nodes) {
      if (n.id.startsWith("entity:")) {
        entities[n.id] = i % clusterCount;
        i += 1;
      }
    }
    return { ...payload, entityCommunities: entities };
  }

  describe("mergeEntityCommunities（纯函数，小样本直测）", () => {
    it("实体侧为空 ⇒ 原样返回笔记侧（同一引用，不复制）", () => {
      const notes = new Map([["n1", 5], ["n2", 9]]);
      expect(mergeEntityCommunities(notes, undefined)).toBe(notes);
      expect(mergeEntityCommunities(notes, new Map())).toBe(notes);
    });

    it("两侧都为空 ⇒ undefined（调用方据此走「无社区数据 ⇒ 哈希分桶」那唯一一条路径）", () => {
      expect(mergeEntityCommunities(undefined, undefined)).toBeUndefined();
      expect(mergeEntityCommunities(new Map(), new Map())).toBeUndefined();
    });

    it("笔记侧为空、实体侧有 ⇒ 偏移为 0（无重叠可言），键值逐条在册", () => {
      const merged = mergeEntityCommunities(undefined, new Map([["entity:a", 3]]));
      expect([...(merged ?? [])]).toEqual([["entity:a", 3]]);
    });

    it("★ 值域互斥：实体 cid 全部严格大于最大笔记 cid，两侧交集为空", () => {
      const notes = new Map([["n1", 246], ["n2", 2703], ["n3", 12]]);
      const entities = new Map([["entity:a", 0], ["entity:b", 7], ["entity:c", 2703]]);
      const merged = mergeEntityCommunities(notes, entities);
      expect(merged).toBeDefined();
      const entries = [...(merged ?? [])];
      const noteCids = new Set(entries.filter(([id]) => notes.has(id)).map(([, c]) => c));
      const entityCids = entries.filter(([id]) => !notes.has(id)).map(([, c]) => c);
      // 用集合比较写，而不是只验上界 —— 前者对「偏移算错但有界」也能抓到
      expect(entityCids.filter((c) => noteCids.has(c))).toEqual([]);
      for (const c of entityCids) {
        expect(c).toBeGreaterThan(2703);
      }
    });

    it("不修改入参（两侧都不被就地改写）", () => {
      const notes = new Map([["n1", 1]]);
      const entities = new Map([["entity:a", 1]]);
      mergeEntityCommunities(notes, entities);
      expect([...notes]).toEqual([["n1", 1]]);
      expect([...entities]).toEqual([["entity:a", 1]]);
    });

    it("确定性：两次调用逐值相同，且新键不覆盖既有键", () => {
      const notes = new Map([["n1", 2], ["n2", 4]]);
      const entities = new Map([["entity:a", 1], ["entity:b", 2]]);
      const a = mergeEntityCommunities(notes, entities);
      const b = mergeEntityCommunities(notes, entities);
      expect([...(a ?? [])]).toEqual([...(b ?? [])]);
      expect(a?.size).toBe(4);
      expect(a?.get("n1")).toBe(2);
      expect(a?.get("n2")).toBe(4);
    });
  });

  it("★ 合成等规模数据：实体侧有社区 ⇒ 每个节点都有桶，锚点兜底一次都不走", () => {
    const ENTITY_CLUSTERS = 900;
    const payload = withSyntheticEntityCommunities(buildSyntheticFusedPayload(), ENTITY_CLUSTERS);
    expect(Object.keys(payload.entityCommunities ?? {}).length).toBe(22608);

    const pNodes = toPhysics(payload);
    const ec = effectiveCommunities(payload, pNodes);

    // ① 覆盖：归并后的映射必须覆盖**每一个**节点 —— 否则组件会退到锚点兜底
    expect(pNodes.filter((n) => !ec.effective.has(n.id)).length).toBe(0);
    expect(ec.covered).toBe(pNodes.length);

    // ② 兜底函数显式再跑一次：入参已是全覆盖的映射 ⇒ 三路计数必须全 0
    const assignment = assignMissingCommunities({
      nodeIds: pNodes.map((n) => n.id),
      communities: ec.effective,
      anchors: mappingAnchors(payload.edges),
    });
    expect(assignment.assigned).toBe(0);
    expect(assignment.viaAnchor).toBe(0);
    expect(assignment.viaHash).toBe(0);

    // ③ 规模不变量（与真实路径同一组判据）
    const r = buildFusedAggregate(payload);
    assertInvariants(r);

    // ④ 合成后的原始映射：条目数 = 全部节点；社区数 = 笔记侧 + 实体侧
    //    （值域互斥 ⇒ 可加；这条把「错开命名空间」钉在**等规模**数据上）
    expect(r.source.size).toBe(r.totalNodes);
    expect(countDistinctCommunities(r.source)).toBe(
      countDistinctCommunities(ec.notes) + ENTITY_CLUSTERS,
    );
  });

  it("★ 错开命名空间的必要性：直接合并会让「不同社区数」被低估（对拍，不是断言实现细节）", () => {
    const payload = withSyntheticEntityCommunities(buildSyntheticFusedPayload());
    const notes = new Map(Object.entries(payload.communities).map(([k, v]) => [k, Number(v)]));
    const entities = new Map(
      Object.entries(payload.entityCommunities ?? {}).map(([k, v]) => [k, Number(v)]),
    );
    const disjoint = mergeEntityCommunities(notes, entities);
    expect(disjoint).toBeDefined();
    // 朴素合并：两侧 cid 数值撞上就当成同一个社区（这正是要避免的）
    const naive = new Map([...notes, ...entities]);

    const disjointCount = countDistinctCommunities(disjoint);
    const naiveCount = countDistinctCommunities(naive);
    // 严格更大：差值恰等于「两侧 cid 值域的交集大小」被折叠掉的部分
    expect(disjointCount).toBeGreaterThan(naiveCount);
    expect(disjointCount).toBe(
      countDistinctCommunities(notes) + countDistinctCommunities(entities),
    );
    // eslint-disable-next-line no-console -- 这两个数是「错开 vs 朴素」的量化差异
    console.log(
      `[kgFusedScale] 社区数：错开命名空间 ${disjointCount} vs 朴素合并 ${naiveCount}`
        + `（笔记侧 ${countDistinctCommunities(notes)} + 实体侧 ${countDistinctCommunities(entities)}，被折叠 ${
          disjointCount - naiveCount
        } 个社区）`,
    );
  });

  it("主路径与锚点兜底给出的桶映射**不同**（证明实体侧社区被真的消费了，不是摆设）", () => {
    const payload = withSyntheticEntityCommunities(buildSyntheticFusedPayload());
    const pNodes = toPhysics(payload);
    const main = effectiveCommunities(payload, pNodes).effective;
    // 锚点兜底路径：只用笔记侧社区 + `mapping` 锚点（= 引入实体侧社区之前的形态）
    const notesOnly = mergeCommunitiesTopologically({
      communities: new Map(Object.entries(payload.communities).map(([k, v]) => [k, Number(v)])),
      edges: payload.edges,
      targetCount: TARGET_CLUSTER_COUNT,
    }).communities;
    const anchored = assignMissingCommunities({
      nodeIds: pNodes.map((n) => n.id),
      communities: notesOnly,
      anchors: mappingAnchors(payload.edges),
    }).communities;

    let differing = 0;
    for (const [id, cid] of main) {
      if (anchored.get(id) !== cid) {
        differing += 1;
      }
    }
    // 若两者逐值相同，说明实体侧社区**没有**参与归并（改动等于空转）—— 必须失败
    expect(differing).toBeGreaterThan(0);
    // eslint-disable-next-line no-console -- 量化「实体侧社区改变了多少节点的归属」
    console.log(`[kgFusedScale] 主路径与锚点兜底对 ${differing} / ${main.size} 个节点给出不同桶`);
  });
});

describe("聚合图密度：改绑前 / 旧哈希兜底 / 锚点优先（生产规则）", () => {
  // 为什么要比这三个：密度是**布局质量的一等判据**（见文件头 ②），
  // 而改绑把 22,608 个无社区节点 + 97,135 条实体侧边塞进了同一层聚合图。
  const payload = realPayload;

  /** 跑一次完整聚合，`assign` 决定「无社区节点」怎么归桶。 */
  function run(
    assign: (
      missingIds: string[],
      bucketIds: number[],
      merged: Map<string, number>,
    ) => Map<string, number>,
  ) {
    const onlyNotes = assign === assignNoteOnly;
    const nodes = onlyNotes ? payload.nodes.filter((n) => !n.id.startsWith("entity:")) : payload.nodes;
    const idSet = new Set(nodes.map((n) => n.id));
    const edges = onlyNotes ? payload.edges.filter((e) => idSet.has(e.source) && idSet.has(e.target)) : payload.edges;

    const pNodes = nodes.map((n, idx) => ({
      id: n.id,
      x: 0,
      y: 0,
      vx: 0,
      vy: 0,
      fx: 0,
      fy: 0,
      mass: 1,
      fixed: false,
      kind: n.id.startsWith("entity:") ? "entity" : "note",
      idx,
    })) as PhysicsNode[];

    // ①② 与组件一致：>3000 ⇒ forceCluster；不同 cid 数 > 200 ⇒ 拓扑归并（真函数）
    const source = new Map(Object.entries(payload.communities).map(([k, v]) => [k, Number(v)]));
    const merged = countDistinctCommunities(source) > TARGET_CLUSTER_COUNT
      ? mergeCommunitiesTopologically({ communities: source, edges, targetCount: TARGET_CLUSTER_COUNT }).communities
      : source;

    // ③ 归桶（三条策略的差异只在这一步）
    const bucketIds = [...new Set(merged.values())].sort((a, b) => a - b);
    const missing = pNodes.filter((n) => !merged.has(n.id)).map((n) => n.id);
    const assigned = assign(missing, bucketIds, merged);
    // 三种策略的返回值语义统一为「全部节点的最终桶」（A 的空 Map / B 只含补全部分 /
    // C 是 `assignMissingCommunities` 返回的完整新 Map），逐键覆盖 ⇒ 结果一致。
    const effective = new Map(merged);
    for (const [k, v] of assigned) {
      effective.set(k, v);
    }

    // 保真度：两端落进同一个桶的边会被聚合层**直接丢弃**（buildAggregateGraph 的 `sIdx === tIdx` 分支）
    const intraByType = new Map<string, number>();
    for (const e of edges) {
      const s = effective.get(e.source);
      const t = effective.get(e.target);
      if (s !== undefined && t !== undefined && s === t) {
        const k = e.type ?? "(无)";
        intraByType.set(k, (intraByType.get(k) ?? 0) + 1);
      }
    }

    const layoutUnits = new Set(effective.values());
    const agg = buildAggregateGraph({
      nodes: pNodes,
      edges,
      communities: effective,
      layoutUnits,
      seedPhysics: { repulsion: AGG_PHYSICS_CONFIG.repulsion, gravity: AGG_PHYSICS_CONFIG.gravity },
    });
    const pairs = layoutUnits.size < 2 ? 0 : (layoutUnits.size * (layoutUnits.size - 1)) / 2;
    return {
      nodes: pNodes.length,
      edges: edges.length,
      buckets: layoutUnits.size,
      aggEdges: agg.edges.length,
      realNodes: agg.nodes.filter((n) => !n.id.startsWith("__agg__")).length,
      density: pairs === 0 ? 0 : agg.edges.length / pairs,
      intraTotal: [...intraByType.values()].reduce((a, b) => a + b, 0),
      intraByType,
    };
  }

  /** 策略 A：只保留笔记（= 改绑前的融合图，当时实体侧为空） */
  function assignNoteOnly(): Map<string, number> {
    return new Map();
  }
  /** 策略 B：**旧**生产规则 —— 无社区节点一律按 hash 并入已有桶 */
  function assignHash(m: string[], b: number[]): Map<string, number> {
    return new Map(m.map((id) => [id, b[Math.abs(hashStringToInt(id)) % b.length]]));
  }
  /** 策略 C：**现**生产规则 —— 调真函数 `assignMissingCommunities`（入参就是真实 merged 映射） */
  function assignProduction(_m: string[], _b: number[], merged: Map<string, number>): Map<string, number> {
    return assignMissingCommunities({
      nodeIds: payload.nodes.map((n) => n.id),
      communities: merged,
      anchors: mappingAnchors(payload.edges),
    }).communities;
  }

  it("锚点优先必须显著低于旧 hash 兜底，且**与组件实际走的生产路径同一份结果**", () => {
    const A = run(assignNoteOnly); // 改绑前
    const B = run(assignHash); // 旧规则：hash 兜底
    const C = run(assignProduction); // 现规则：锚点优先
    const prod = buildFusedAggregate(payload); // 组件路径（真函数）

    for (const r of [A, B, C]) {
      expect(r.realNodes).toBe(0);
      expect(r.buckets).toBeLessThanOrEqual(MAX_AGG_PHYS_NODES);
      expect(r.aggEdges).toBeGreaterThan(0);
      // 桶内边只能被丢弃 ⇒ 聚合边数不可能超过跨桶边对数；同时两者都不超过总边数
      expect(r.intraTotal).toBeLessThanOrEqual(r.edges);
    }
    // ★ 生产路径 == 策略 C：这条把「组件用的是锚点规则而不是 hash」钉死。
    //   若有人把 GraphView 的补全改回 hash，这条会失败（策略 C 是独立构造的）。
    expect(C.aggEdges).toBe(prod.aggEdgeCount);
    expect(C.buckets).toBe(prod.buckets);
    expect(C.density).toBeCloseTo(prod.density, 12);
    // ★ 锚点优先必须优于旧规则（这是「该不该这么修」的判据，不是审美）
    expect(C.aggEdges).toBeLessThan(B.aggEdges);
    // 且回到改绑前的量级（不超过 A 的 2 倍 —— 容忍融合进来的实体侧边带来的新增桶对）
    expect(C.aggEdges).toBeLessThanOrEqual(A.aggEdges * 2);

    // eslint-disable-next-line no-console -- 这三个数是本次改绑的渲染侧代价与修复收益
    console.log(
      `[kgFusedScale] 聚合图密度对照（桶数 / 聚合边 / 密度 / 桶内被丢弃边）\n`
        + `  A 改绑前（仅笔记）   节点 ${A.nodes} 边 ${A.edges} ⇒ 桶 ${A.buckets} 聚合边 ${A.aggEdges} 密度 ${
          (A.density * 100).toFixed(1)
        }% 桶内 ${A.intraTotal}\n`
        + `  B 旧规则（hash 兜底）节点 ${B.nodes} 边 ${B.edges} ⇒ 桶 ${B.buckets} 聚合边 ${B.aggEdges} 密度 ${
          (B.density * 100).toFixed(1)
        }% 桶内 ${B.intraTotal}  ${JSON.stringify(Object.fromEntries(B.intraByType))}\n`
        + `  C 生产规则（锚点优先）节点 ${C.nodes} 边 ${C.edges} ⇒ 桶 ${C.buckets} 聚合边 ${C.aggEdges} 密度 ${
          (C.density * 100).toFixed(1)
        }% 桶内 ${C.intraTotal}  ${JSON.stringify(Object.fromEntries(C.intraByType))}`,
    );
  });
});

describe("assignMissingCommunities 的规则（纯函数，小样本直测）", () => {
  const base = new Map<string, number>([["n1", 7], ["n2", 3]]);

  it("锚点对端有桶 ⇒ 继承它（方向不敏感）", () => {
    const out = assignMissingCommunities({
      nodeIds: ["e1", "e2"],
      communities: base,
      anchors: [
        { source: "e1", target: "n1" },
        { source: "n2", target: "e2" }, // 反向书写同样成立
      ],
    });
    expect(out.communities.get("e1")).toBe(7);
    expect(out.communities.get("e2")).toBe(3);
    expect(out.viaAnchor).toBe(2);
    expect(out.viaHash).toBe(0);
  });

  it("多锚点 ⇒ 取**桶 id 最小**的一条，且与 anchors 的顺序无关（确定性）", () => {
    const a = assignMissingCommunities({
      nodeIds: ["e1"],
      communities: base,
      anchors: [{ source: "e1", target: "n1" }, { source: "e1", target: "n2" }],
    });
    const b = assignMissingCommunities({
      nodeIds: ["e1"],
      communities: base,
      anchors: [{ source: "e1", target: "n2" }, { source: "e1", target: "n1" }],
    });
    expect(a.communities.get("e1")).toBe(3); // min(7, 3)
    expect(b.communities.get("e1")).toBe(3);
  });

  it("锚点两端都缺桶 ⇒ 帮不上忙（只跟一跳）⇒ 记进 viaHash", () => {
    const out = assignMissingCommunities({
      nodeIds: ["e1", "e2"],
      communities: base,
      anchors: [{ source: "e1", target: "e2" }],
    });
    expect(out.viaAnchor).toBe(0);
    expect(out.viaHash).toBe(2);
    expect(out.assigned).toBe(2);
  });

  it("锚点对端没有桶（不在 communities 里）⇒ 同样退回 hash，且被计数", () => {
    const out = assignMissingCommunities({
      nodeIds: ["e1"],
      communities: base,
      anchors: [{ source: "e1", target: "ghost" }],
    });
    expect(out.viaAnchor).toBe(0);
    expect(out.viaHash).toBe(1);
  });

  it("桶集合为空 ⇒ 原样返回（不 panic、不造桶）", () => {
    const out = assignMissingCommunities({ nodeIds: ["e1"], communities: new Map(), anchors: [] });
    expect(out.assigned).toBe(0);
    expect(out.bucketCount).toBe(0);
    expect(out.communities.size).toBe(0);
  });

  it("不修改入参，且**不生成新桶 id**", () => {
    const input = new Map<string, number>([["n1", 7]]);
    const out = assignMissingCommunities({
      nodeIds: ["e1", "e2"],
      communities: input,
      anchors: [{ source: "e1", target: "n1" }],
    });
    expect(input.size).toBe(1); // 未被就地修改
    expect(out.communities.size).toBe(3);
    expect(out.communities.get("e1")).toBe(7); // 锚点继承
    // e2 没有锚点 ⇒ hash 兜底，值必须是**已有的**桶（此处唯一桶 = 7）
    expect(out.communities.get("e2")).toBe(7);
    expect(out.viaHash).toBe(1);
    expect(out.bucketCount).toBe(1);
  });

  it("幂等：对补全后的映射再调一次 ⇒ 零补全、逐值相同", () => {
    const first = assignMissingCommunities({
      nodeIds: ["e1", "e2"],
      communities: base,
      anchors: [{ source: "e1", target: "n1" }],
    });
    const second = assignMissingCommunities({
      nodeIds: ["e1", "e2"],
      communities: first.communities,
      anchors: [{ source: "e1", target: "n1" }],
    });
    expect(second.assigned).toBe(0);
    expect([...second.communities]).toEqual([...first.communities]);
  });
});

describe("合成等规模数据：近半数节点无社区时的退化形态", () => {
  it("无社区节点被并入**已存在的桶**，不产生新桶、也不混入真实节点", () => {
    const payload = buildSyntheticFusedPayload();
    const r = buildFusedAggregate(payload);
    assertInvariants(r);
    expect(r.buckets).toBeLessThanOrEqual(TARGET_CLUSTER_COUNT);
  });

  it("★ 把锚点拿掉 ⇒ hash 兜底被走到且**被计数**（证明兜底不是静默的）", () => {
    const payload = buildSyntheticFusedPayload();
    const pNodes = toPhysics(payload);
    const communities = new Map(
      Object.entries(payload.communities).map(([k, v]) => [k, Number(v)]),
    );
    const missing = pNodes.filter((n) => !communities.has(n.id)).length;
    const out = assignMissingCommunities({ nodeIds: pNodes.map((n) => n.id), communities, anchors: [] });
    expect(out.viaAnchor).toBe(0);
    expect(out.viaHash).toBe(missing);
    expect(out.assigned).toBe(missing);
  });

  it("对照：**全部**节点都有社区时同样是 0 真实节点混入（说明①不是靠数据巧合成立）", () => {
    const payload = buildSyntheticFusedPayload();
    // 给实体节点也补上社区（模拟「实体也有社区」这一理想状态）
    const cids = [...new Set(Object.values(payload.communities))];
    let i = 0;
    for (const n of payload.nodes) {
      if (!(n.id in payload.communities)) {
        payload.communities[n.id] = cids[i++ % cids.length];
      }
    }
    const r = buildFusedAggregate(payload);
    expect(r.covered).toBe(r.totalNodes);
    assertInvariants(r);
  });
});

/**
 * 等规模合成 payload：规模与结构对齐真实产物，但**不需要 DB**。
 *
 * 数字取自 2026-09-18 的真实测量（见 kg-fused-payload-dump-0918.txt）：
 *   笔记 24,288（2,458 社区）/ 实体 22,608（无社区）/ 边 74,791 + 75,527 + 22,608。
 * 用确定性 PRNG（mulberry32 的简化版），保证每次跑同一份数据。
 */
function buildSyntheticFusedPayload(): Payload {
  const NOTE_COUNT = 24288;
  const ENTITY_COUNT = 22608;
  const COMMUNITY_COUNT = 2458;
  const nodes: Payload["nodes"] = [];
  const communities: Record<string, number> = {};
  let seed = 0x9e3779b9;
  const rnd = () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = seed;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  for (let i = 0; i < NOTE_COUNT; i++) {
    const id = `note-${i}`;
    nodes.push({ id, title: `笔记 ${i}`, type: "note" });
    communities[id] = 246 + (i % COMMUNITY_COUNT);
  }
  for (let i = 0; i < ENTITY_COUNT; i++) {
    nodes.push({ id: `entity:e-${i}`, title: `实体 ${i}`, type: "concept" });
  }
  const edges: Payload["edges"] = [];
  const noteIds = nodes.slice(0, NOTE_COUNT).map((n) => n.id);
  const entityIds = nodes.slice(NOTE_COUNT).map((n) => n.id);
  // 74,791 条笔记互链
  for (let i = 0; i < 74791; i++) {
    edges.push({ source: noteIds[(rnd() * NOTE_COUNT) | 0], target: noteIds[(rnd() * NOTE_COUNT) | 0], type: "link" });
  }
  // 75,527 条实体关系（渲染类别是 `reference`，关系名在 `relationType` —— 对齐 DTO 契约）
  for (let i = 0; i < 75527; i++) {
    edges.push({
      source: entityIds[(rnd() * ENTITY_COUNT) | 0],
      target: entityIds[(rnd() * ENTITY_COUNT) | 0],
      type: "reference",
      relationType: "mentions",
    });
  }
  // 22,608 条 mapping（实体 ↔ 同序号笔记）
  for (let i = 0; i < ENTITY_COUNT; i++) {
    edges.push({ source: `entity:e-${i}`, target: `note-${i}`, type: "mapping" });
  }
  return { nodes, edges, communities };
}
