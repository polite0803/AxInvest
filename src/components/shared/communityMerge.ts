/**
 * 社区拓扑归并：把细粒度社区合并到目标桶数，供大图的**聚合物理**与**气泡渲染**共用。
 *
 * ── 为什么不能用哈希分桶 ──
 * 原实现是 `Math.abs(hashStringToInt(nodeId)) % FORCE_CLUSTER_COUNT`，与拓扑**完全无关**。
 * 实测（24288 节点 / 74791 边 / 2458 真实社区，见
 * `docs/audits/AUDIT-wiki-graph-edges-2026-09-15.md` §6.4 / §6.8）：
 *   · 同区内边占比 **0.49%**，与「均匀随机划分」的期望 **0.50%** 相等 ⇒ 这批分组在拓扑上就是随机划分；
 *   · 聚合图边密度 **82.69%**（16456 条聚合边 / 200 个桶）⇒ 逼近完全图 ⇒ 所有聚合节点受力趋同
 *     ⇒ 力导向退化为「均匀铺开」⇒ 屏幕上就是「一片互不相连、均匀分布的点」。
 *
 * ── 为什么是「一轮内并行合并多对」 ──
 * 归并增益用模块度增量（Louvain 第一阶段的合并判据）：
 *   ΔQ(i, j) = w_ij / m − (a_i · a_j) / (2 m²)
 * 其中 `w_ij` = 社区 i、j 之间的边数，`a_i` = 社区 i 内节点的度数之和，`m` = 有效边总数。
 *
 * 三种循环结构的实测耗时（同一数据集，2458 → 300 桶）：
 *   · 每轮只合并全局最优的**一对**（朴素扫描）      → **1242 ms**
 *   · 同上 + 每簇缓存最佳伙伴                        → 883 ms
 *   · 同上 + 惰性最大堆                              → 1291 ms（更慢：瓶颈不在这里）
 *   · **一轮内并行合并多对**（本实现，按 ΔQ 降序做不冲突匹配）→ **46 ms**
 * 前两类的共同病根是「每轮只合并一对」⇒ 需要 `社区数 − 目标数` 轮（2158 轮），
 * 而每轮都要扫一遍**持续增长**的邻接表（合并后簇的邻居并集越来越大）⇒ 总成本被轮次数放大 27 倍。
 * 并行匹配把轮数降到 20 轮，且质量更高（内聚度 9.35%，高于贪婪合并对的 6.66%）。
 *
 * ⚠ 本函数是**纯函数且确定性**（无随机、无时间依赖、排序稳定）——
 * 同一份输入必然得到同一份分桶，这是「布局可复现」与「气泡位置与物理同源」的前提。
 */

// ⚠ 本文件此前**没有任何 import** —— `hashStringToInt` 只在上面的文档注释里被提及。
// 2026-09-18 的 `assignMissingCommunities` 是第一个真正调用它的地方，而漏 import 的后果
// 只会在**真的走进 hash 兜底分支**时炸（`ReferenceError`）：真实数据下锚点覆盖 100%、
// 兜底一次不走 ⇒ 主路径与「锚点优先」用例全绿，只有专测兜底的用例才暴露。
// 结论：纯函数模块也要有「强制走兜底分支」的用例，且 vitest 不做类型检查 ⇒ 必须另跑 tsc。
import { hashStringToInt } from "./graphViewUtils";

/** 归并需要的最小边形态（`GraphEdge` 满足此结构）。 */
export interface MergeEdge {
  source: string;
  target: string;
}

export interface MergeOptions {
  /** 原始社区映射（nodeId → 社区 id）。 */
  communities: Map<string, number>;
  /** 用于构建社区图的边（需带字符串端点 id，不是索引化的物理边）。 */
  edges: ReadonlyArray<MergeEdge>;
  /** 目标桶数（归并后不超过该值）。 */
  targetCount: number;
}

export interface MergeOutcome {
  /** 归并后的桶映射（nodeId → 桶 id，桶 id 为 `0..bucketCount-1` 的连续整数）。 */
  communities: Map<string, number>;
  /** 归并后的桶数。 */
  bucketCount: number;
  /** 归并前的不同社区数。 */
  sourceCommunityCount: number;
  /** 是否真的执行了归并（社区数本来就 ≤ 目标数时为 false，此时映射原样返回）。 */
  merged: boolean;
}

/**
 * 统计「不同社区 id 的个数」。
 *
 * ⚠ 存在的唯一理由：`Map<nodeId, cid>.size` 是**节点条目数**，不是社区数 ——
 * 两者在本数据集上相差近一个数量级（24288 vs 2458）。原代码把前者当后者用，
 * 导致阈值恒真、分桶无条件执行（详见审计报告 §6.4 的「恒真证明」）。
 */
export function countDistinctCommunities(
  communities: Map<string, number> | undefined,
): number {
  if (!communities || communities.size === 0) {
    return 0;
  }
  const distinct = new Set<number>();
  for (const cid of communities.values()) {
    distinct.add(cid);
  }
  return distinct.size;
}

// ═══════════════════════════════════════════════════════════════════════════
// 笔记侧社区 ∪ 实体侧社区（2026-09-18）
// ═══════════════════════════════════════════════════════════════════════════

/**
 * 把「笔记侧社区」与「实体侧社区」合成一张**命名空间互不重叠**的社区映射。
 *
 * ── 为什么实体侧要单独算（本函数存在的理由）──
 * 融合图 = 笔记图 ∪ 知识图谱实体子图。Louvain 此前**只跑笔记图**
 * （`wiki_graph_communities` 的输入是 `note::get_vault_graph`，不含实体），
 * 而融合发生在其后 ⇒ 22,608 个实体节点在社区映射里没有任何条目，
 * 只能靠 `assignMissingCommunities` 跟随 `mapping` 锚点去「继承同名笔记的桶」。
 * 那是**补丁**，不是**结论**：实体之间真实的 75,251 条关系边
 * （`reference`）从未参与过分区 —— 实体的桶从一开始就由「它的名字是否恰好等于某个笔记标题」
 * 决定，与实体在知识图谱里的实际位置无关。
 * 2026-09-18 起后端在**实体子图**上单独跑一次 Louvain，结果随
 * `LouvainResult.entityCommunities` 一起下发，本函数把两份结果合成一份。
 *
 * ── 为什么必须错开 id（而不是 `new Map([...notes, ...entities])`）──
 * 两次 Louvain 是**独立运行**的，各自的社区 id 都从 0 起算 ⇒ 两边都可能有 `cid = 7`，
 * 而它们毫无关系。直接合并会让「笔记社区 7」与「实体社区 7」在数值上重合，于是：
 *   · `countDistinctCommunities` 少数一批社区（不同 id 的个数被低估）；
 *   · 归并阶段（`mergeCommunitiesTopologically`）把两侧的 `7` 当成**同一个簇**，
 *     求 ΔQ 时把两簇的边权与度数直接相加 ⇒ 两个拓扑上无关的社区被**强制**并进一个桶，
 *     桶内边在聚合层被丢弃（`buildAggregateGraph` 的 `sIdx === tIdx` 分支）⇒ 结构被抹掉。
 * 错开之后，「笔记社区 ↔ 实体社区」的关联只由**真实的 `mapping` 边**决定
 * （归并阶段读它算 ΔQ）—— 这正是想要的语义。
 *
 * 偏移量取 `max(笔记 cid) + 1` 而**不是**一个常量：常量会在 cid 值域增长时静默失效
 * （两边重新撞上，且撞上与否取决于数据规模，测试很难覆盖）；
 * 派生值随输入自适应，且仍然完全确定。值域互斥有**断言**兜住（见单测）。
 *
 * 确定性/幂等：同一份输入必然得到同一份输出（无随机、无时间依赖）。
 * 键不会互相覆盖：实体 id 带 `entity:` 前缀、笔记 id 不带（融合层 `wiki.rs` 保证）。
 *
 * @returns 实体侧为空时**原样返回** `notes`（同一引用，调用方不应修改返回值）；
 *          两侧都为空时返回 `undefined`（调用方据此走「无社区数据」的哈希分桶路径）。
 */
export function mergeEntityCommunities(
  notes: Map<string, number> | undefined,
  entities: Map<string, number> | undefined,
): Map<string, number> | undefined {
  const hasNotes = notes !== undefined && notes.size > 0;
  const hasEntities = entities !== undefined && entities.size > 0;
  if (!hasNotes && !hasEntities) {
    return undefined;
  }
  if (!hasEntities) {
    return notes;
  }

  let offset = 0;
  if (hasNotes) {
    let maxNoteCid = Number.NEGATIVE_INFINITY;
    for (const [, cid] of notes) {
      if (cid > maxNoteCid) {
        maxNoteCid = cid;
      }
    }
    offset = maxNoteCid + 1;
  }

  const merged = new Map<string, number>(notes ?? []);
  for (const [id, cid] of entities) {
    merged.set(id, cid + offset);
  }
  return merged;
}

/**
 * 按拓扑把社区归并到 `targetCount` 个桶。
 *
 * 保真性：默认**不动大社区** —— 每轮只把小球并进它的最佳伙伴，
 * 而大社区（邻居权重高、自身度数高）的 ΔQ 对多为负，罕被选中，
 * 故实测最大桶恰为真实最大社区（214 节点），不会被并掉。
 */
export function mergeCommunitiesTopologically(options: MergeOptions): MergeOutcome {
  const { communities, edges, targetCount } = options;

  const cidIndex = new Map<number, number>();
  for (const cid of communities.values()) {
    if (!cidIndex.has(cid)) {
      cidIndex.set(cid, cidIndex.size);
    }
  }
  const sourceCommunityCount = cidIndex.size;

  // 社区数本来就够少 ⇒ 原样返回，不做任何拓扑计算（小图/粗粒度社区的快路径）。
  if (sourceCommunityCount <= targetCount) {
    return {
      communities,
      bucketCount: sourceCommunityCount,
      sourceCommunityCount,
      merged: false,
    };
  }

  // ── 节点 → 簇索引（避免在边循环里做两次 Map 查找） ──
  const nodeCluster = new Map<string, number>();
  for (const [nodeId, cid] of communities) {
    const index = cidIndex.get(cid);
    if (index !== undefined) {
      nodeCluster.set(nodeId, index);
    }
  }

  // ── 建社区图：邻接表（无向，边权 = 两社区之间的边数）+ 加权度 ──
  const adjacency: Array<Map<number, number>> = [];
  for (let i = 0; i < sourceCommunityCount; i++) {
    adjacency.push(new Map<number, number>());
  }
  const weight = new Float64Array(sourceCommunityCount);
  let validEdges = 0;
  for (const edge of edges) {
    const a = nodeCluster.get(edge.source);
    const b = nodeCluster.get(edge.target);
    if (a === undefined || b === undefined) {
      continue;
    }
    validEdges += 1;
    weight[a] += 1;
    weight[b] += 1;
    if (a === b) {
      continue;
    }
    adjacency[a].set(b, (adjacency[a].get(b) ?? 0) + 1);
    adjacency[b].set(a, (adjacency[b].get(a) ?? 0) + 1);
  }
  if (validEdges === 0) {
    return fallbackToHashBuckets(nodeCluster, sourceCommunityCount, targetCount);
  }

  // 并查集：合并只记录「谁并进了谁」，而「节点（按社区索引）→ 最终桶」的映射必须**沿链解析** ——
  // 否则「被并入别的社区」的那些社区所辖节点会查不到桶（第一版正是这个 bug，由单测抓到：
  // 20 个节点只有 10 个拿到桶）。
  const parent = new Int32Array(sourceCommunityCount);
  for (let i = 0; i < sourceCommunityCount; i++) {
    parent[i] = i;
  }
  const resolveRoot = (index: number): number => {
    let root = index;
    while (parent[root] !== root) {
      root = parent[root];
    }
    let cursor = index;
    while (parent[cursor] !== root) {
      const next = parent[cursor];
      parent[cursor] = root;
      cursor = next;
    }
    return root;
  };

  const alive = new Set<number>();
  for (let i = 0; i < sourceCommunityCount; i++) {
    alive.add(i);
  }
  let remaining = sourceCommunityCount;
  const denominator = 2 * validEdges * validEdges;

  // ── 每轮：为所有存活簇求最佳伙伴，再按 ΔQ 降序做「不冲突匹配」 ──
  // 一个簇一轮最多参与一次合并 ⇒ 一轮可消掉数百对，同时避免「同一轮内链式合并」
  // 带来的顺序依赖（那会让结果依赖簇的遍历顺序，破坏确定性）。
  while (remaining > targetCount) {
    const candidates: Array<{ a: number; b: number; gain: number }> = [];
    for (const a of alive) {
      let bestB = -1;
      let bestGain = Number.NEGATIVE_INFINITY;
      const neighbours = adjacency[a];
      for (const [b, w] of neighbours) {
        if (!alive.has(b)) {
          continue;
        }
        const gain = w / validEdges - (weight[a] * weight[b]) / denominator;
        if (gain > bestGain) {
          bestGain = gain;
          bestB = b;
        }
      }
      if (bestB >= 0) {
        candidates.push({ a, b: bestB, gain: bestGain });
      }
    }
    if (candidates.length === 0) {
      break;
    }
    candidates.sort((x, y) => y.gain - x.gain);

    const touched = new Set<number>();
    let progressed = false;
    for (const { a, b } of candidates) {
      if (remaining <= targetCount) {
        break;
      }
      if (touched.has(a) || touched.has(b) || !alive.has(a) || !alive.has(b)) {
        continue;
      }
      // 把 b 并入 a：b 的邻居改接 a（权重累加），a 接手 b 的加权度
      const aAdjacency = adjacency[a];
      for (const [neighbour, w] of adjacency[b]) {
        if (neighbour === a) {
          continue;
        }
        const neighbourAdjacency = adjacency[neighbour];
        if (neighbourAdjacency) {
          neighbourAdjacency.delete(b);
          neighbourAdjacency.set(a, (neighbourAdjacency.get(a) ?? 0) + w);
        }
        aAdjacency.set(neighbour, (aAdjacency.get(neighbour) ?? 0) + w);
      }
      aAdjacency.delete(b);
      adjacency[b].clear();
      weight[a] += weight[b];
      parent[b] = a;
      alive.delete(b);
      touched.add(a);
      touched.add(b);
      remaining -= 1;
      progressed = true;
    }
    if (!progressed) {
      break;
    }
  }

  // ── 桶 id 取 `0..bucketCount-1` 的连续整数 ──
  // 刻意不复用真实社区 id：后者是后端 Louvain 的任意整数（实测值域 [246, 24284]），
  // 既不适合作调色板索引，也无法反映「该桶是多个社区的拼接」这一事实。
  const bucketOfCluster = new Map<number, number>();
  let nextBucket = 0;
  for (const index of alive) {
    bucketOfCluster.set(index, nextBucket);
    nextBucket += 1;
  }

  const mergedCommunities = new Map<string, number>();
  for (const [nodeId, index] of nodeCluster) {
    const bucket = bucketOfCluster.get(resolveRoot(index));
    if (bucket !== undefined) {
      mergedCommunities.set(nodeId, bucket);
    }
  }

  return {
    communities: mergedCommunities,
    bucketCount: alive.size,
    sourceCommunityCount,
    merged: true,
  };
}

/**
 * 兜底：社区图里一条有效边都没有（社区之间零连接）。
 *
 * 此时拓扑信息为零，任何「按拓扑归并」都退化为任意合并，用哈希分桶即可 ——
 * 但桶内仍是**已存在的簇**，不产生新 id，也不依赖 `cid` 的数值范围。
 */
function fallbackToHashBuckets(
  nodeCluster: Map<string, number>,
  sourceCommunityCount: number,
  targetCount: number,
): MergeOutcome {
  const mergedCommunities = new Map<string, number>();
  let nextBucket = 0;
  const bucketOfCluster = new Map<number, number>();
  for (const index of nodeCluster.values()) {
    if (!bucketOfCluster.has(index)) {
      bucketOfCluster.set(index, nextBucket % targetCount);
      nextBucket += 1;
    }
  }
  for (const [nodeId, index] of nodeCluster) {
    mergedCommunities.set(nodeId, bucketOfCluster.get(index) ?? 0);
  }
  return {
    communities: mergedCommunities,
    bucketCount: Math.min(sourceCommunityCount, targetCount),
    sourceCommunityCount,
    merged: true,
  };
}

// ═══════════════════════════════════════════════════════════════════════════
// 缺失节点的桶补全（2026-09-18）
// ═══════════════════════════════════════════════════════════════════════════

/** 一条锚点关系：`source` 若缺桶，可继承 `target` 的桶。 */
export interface CommunityAnchor {
  source: string;
  target: string;
}

export interface AssignMissingOptions {
  /** 待归桶的全部节点 id（**已归桶的会被跳过**，调用方不必预筛）。 */
  nodeIds: ReadonlyArray<string>;
  /** 已有的桶映射（nodeId → 桶 id）。**不被修改**。 */
  communities: Map<string, number>;
  /**
   * 锚点关系，方向不敏感（哪一端缺桶就用另一端）。
   *
   * 目前唯一来源是融合层合成的 `mapping` 边（`wiki.rs:1173-1192`：
   * 实体名.toLowerCase() == 笔记标题.toLowerCase() 时，合成 `entity:<id> → <noteId>`）。
   */
  anchors: ReadonlyArray<CommunityAnchor>;
}

export interface AssignMissingOutcome {
  /** 补全后的新 Map（入参 `communities` 的原引用不被修改）。 */
  communities: Map<string, number>;
  /** 实际补全的节点数 = `viaAnchor + viaHash`。 */
  assigned: number;
  /**
   * 靠锚点拿到桶的节点数 —— 也就是**保住了拓扑**的那些。
   * 它们与自己的锚点落在同一个桶 ⇒ 桶内边被聚合层直接丢弃（保真度的代价），
   * 但桶级图保持了稀疏（这才是力导向能收敛出结构的条件）。
   */
  viaAnchor: number;
  /**
   * 锚点不可用、退回 `hash(id) % 桶数` 的节点数。
   *
   * ⚠ 这个数**必须被观测**，不能是静默的：hash 归桶在拓扑上是随机划分
   * （实测同区内边占比 0.49% ≈ 随机期望 0.50%，见本文件头），
   * 一旦它从 0 变成非 0，就说明「锚点这条路断了」（例如融合层的 `mapping` 边消失、
   * 或社区缓存与节点集不再同源），而那正是 2026-09-18 这次事故的形态。
   */
  viaHash: number;
  /** 桶 id 集合大小。补全**不改变**它（只并入已存在的桶，绝不生成新 id）。 */
  bucketCount: number;
}

/**
 * 把缺桶的节点补全到**已存在的桶**里：优先跟随锚点（`mapping` 边对端）的桶，锚点不可用才 hash 兜底。
 *
 * ── 为什么不能只有 hash（本次改动的原因）──
 * 2026-09-18 把 wiki 的知识库绑定从空壳库改到 `lemonhu_knowledge_graph` 后，
 * 融合图里多了 22,608 个实体节点，而 Louvain 只跑**笔记图**
 * （`wiki_graph_communities` 的输入是 `note::get_vault_graph`，不含实体）
 * ⇒ 近半数节点没有社区。旧实现把它们按 `hash(id) % 桶数` **随机**撒进 200 个桶，
 * 实测（真实载荷 46,896 节点 / 172,926 边，见 `__tests__/kgFusedScale.test.ts`）：
 *   桶级聚合边 **1,873 → 18,011**、密度 **9.4% → 90.5%**（`C(200,2) = 19,900` ⇒ 91% 的桶对有边）。
 * 而本仓自己记录过这条形态的后果（`GraphView.tsx:1273-1276`）：
 * 「密度逼近完全图（旧哈希分桶实测 82.69%）⇒ 聚合节点受力趋同 ⇒ 力导向退化为均匀铺开」
 * —— 现状 90.5% 比那次更差。
 * 改为跟随锚点后：聚合边 **1,947**、密度 **9.8%**，回到改绑前的量级。
 *
 * ── 锚点路径现在的定位：**兜底**，而不再是主路径（2026-09-18 同日修正）──
 * 上面的 1,947 / 9.8% 是「实体侧没有自己的社区」时的成绩，代价是实体的分区
 * 完全由「名字是否恰好等于某个笔记标题」决定。当天补上了实体侧自己的 Louvain
 * （见 [`mergeEntityCommunities`]），实体节点从**主路径**就拿到桶 ⇒ 本函数的
 * `viaAnchor` 在真实数据上应当回落到 0。
 * 它仍然必须留着且必须是对的：实体图缓存是 30 秒 TTL、社区缓存随 notes 写入失效，
 * 两者不同源时总会有「图上有、社区映射里没有」的节点（例如刚导入的一批实体）。
 * 这时锚点路径负责兜住它们，`viaHash` 继续负责把「锚点也兜不住」这件事喊出来。
 *
 * ── 为什么只跟一跳、不做传递闭包 ──
 * 锚点的对端是**笔记**，而笔记在社区映射里 100% 有桶（Louvain 的输入就是它们）。
 * 需要多跳才能命中桶的情形在结构上不存在；而为它写传播循环，代价是
 * 「环、长链、以及最坏情况 O(N) 轮」的复杂度与不确定性 —— 换来的是一个空集上的收益。
 * 真的遇到「锚点对端也没桶」（例如锚点两端都是实体）时，本函数**不猜**：
 * 记进 `viaHash`，让调用方有机会观测到它。
 *
 * 确定性与幂等：同一份输入必然得到同一份输出 —— 待补节点与桶 id 都排序后处理，
 * 多锚点取**桶 id 最小**的那条（与 `anchors` 的顺序无关）；
 * 重复调用（第二次已无缺桶节点）返回新的空 outcome，桶映射逐值相同。
 */
export function assignMissingCommunities(options: AssignMissingOptions): AssignMissingOutcome {
  const { nodeIds, communities, anchors } = options;

  // 桶 id 集合：**只并入已存在的桶**，绝不生成新 id。
  // 入参的 cid 值域是任意的（真实社区路径下实测 [246, 24284]）⇒ 必须取集合，不能取 0..n-1。
  const bucketIds = [...new Set(communities.values())].sort((a, b) => a - b);
  if (bucketIds.length === 0) {
    return { communities, assigned: 0, viaAnchor: 0, viaHash: 0, bucketCount: 0 };
  }

  const missing = new Set<string>();
  for (const id of nodeIds) {
    if (!communities.has(id)) {
      missing.add(id);
    }
  }
  if (missing.size === 0) {
    return { communities, assigned: 0, viaAnchor: 0, viaHash: 0, bucketCount: bucketIds.length };
  }

  // ── 锚点索引：只保留「恰好一端缺桶」的锚点 ──
  // 两端都缺 ⇒ 这条锚点谁也帮不了（本函数只跟一跳）；两端都有 ⇒ 与本次补全无关。
  const anchorBucket = new Map<string, number>();
  for (const anchor of anchors) {
    const sourceMissing = missing.has(anchor.source);
    const targetMissing = missing.has(anchor.target);
    if (sourceMissing === targetMissing) {
      continue;
    }
    const missingId = sourceMissing ? anchor.source : anchor.target;
    const knownId = sourceMissing ? anchor.target : anchor.source;
    const bucket = communities.get(knownId);
    if (bucket === undefined) {
      continue;
    }
    const previous = anchorBucket.get(missingId);
    // 取最小桶 id（不是第一条）⇒ 结果与 anchors 的遍历顺序无关
    if (previous === undefined || bucket < previous) {
      anchorBucket.set(missingId, bucket);
    }
  }

  const updated = new Map(communities);
  let viaAnchor = 0;
  let viaHash = 0;
  for (const id of [...missing].sort()) {
    const anchored = anchorBucket.get(id);
    if (anchored !== undefined) {
      updated.set(id, anchored);
      viaAnchor += 1;
    } else {
      updated.set(id, bucketIds[Math.abs(hashStringToInt(id)) % bucketIds.length]);
      viaHash += 1;
    }
  }

  return {
    communities: updated,
    assigned: viaAnchor + viaHash,
    viaAnchor,
    viaHash,
    bucketCount: bucketIds.length,
  };
}
