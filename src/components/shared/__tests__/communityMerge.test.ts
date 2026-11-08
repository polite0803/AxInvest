import { describe, expect, it } from "vitest";
import { countDistinctCommunities, mergeCommunitiesTopologically } from "../communityMerge";

/**
 * 构造一个「拓扑明确」的小图，用来验证归并是否尊重拓扑。
 *
 * 结构：两个**全连接团**（各 10 节点 / 45 条边），每个团内部再切成 2 个子社区（各 5 节点）。
 * 两个团之间**只连 1 条边**（a0—b0）。
 *
 * 于是「归并到 2 个桶」的正确答案是唯一的：团 A 的两个子社区合并、团 B 的两个子社区合并。
 * 任何把 a、b 混进同一个桶的结果都说明归并没有按拓扑进行。
 * （子社区用 101/102 与 202/203 这类大数且不相邻的 id，模仿后端 Louvain 的 cid 值域。）
 */
function buildClusteredGraph() {
  const communities = new Map<string, number>();
  const edges: Array<{ source: string; target: string }> = [];
  for (const { prefix, cid } of [{ prefix: "a", cid: 101 }, { prefix: "b", cid: 202 }]) {
    for (let i = 0; i < 10; i++) {
      communities.set(`${prefix}${i}`, i < 5 ? cid : cid + 1);
    }
    for (let i = 0; i < 10; i++) {
      for (let j = i + 1; j < 10; j++) {
        edges.push({ source: `${prefix}${i}`, target: `${prefix}${j}` });
      }
    }
  }
  edges.push({ source: "a0", target: "b0" });
  return { communities, edges };
}

function intraEdgeRatio(
  edges: ReadonlyArray<{ source: string; target: string }>,
  communities: Map<string, number>,
): number {
  const intra = edges.filter((e) => communities.get(e.source) === communities.get(e.target)).length;
  return intra / edges.length;
}

describe("countDistinctCommunities", () => {
  it("统计的是「不同社区 id 数」，不是 Map 的节点条目数", () => {
    const communities = new Map<string, number>([
      ["n1", 7],
      ["n2", 7],
      ["n3", 9],
    ]);
    expect(communities.size).toBe(3);
    expect(countDistinctCommunities(communities)).toBe(2);
  });

  it("空映射与 undefined 都返回 0", () => {
    expect(countDistinctCommunities(undefined)).toBe(0);
    expect(countDistinctCommunities(new Map())).toBe(0);
  });

  it("不可与节点数混用：节点数远大于社区数时两者必须可区分", () => {
    const communities = new Map<string, number>();
    for (let i = 0; i < 500; i++) {
      communities.set(`node-${i}`, i % 5);
    }
    expect(communities.size).toBe(500);
    expect(countDistinctCommunities(communities)).toBe(5);
  });
});

describe("mergeCommunitiesTopologically", () => {
  it("社区数不超过目标桶数时原样返回，不做任何归并", () => {
    const { communities, edges } = buildClusteredGraph();
    const outcome = mergeCommunitiesTopologically({ communities, edges, targetCount: 10 });
    expect(outcome.merged).toBe(false);
    expect(outcome.bucketCount).toBe(4);
    expect(outcome.sourceCommunityCount).toBe(4);
    expect(outcome.communities).toBe(communities);
  });

  it("按拓扑分团：团内合桶、跨团不混", () => {
    const { communities, edges } = buildClusteredGraph();
    const outcome = mergeCommunitiesTopologically({ communities, edges, targetCount: 2 });
    expect(outcome.merged).toBe(true);
    expect(outcome.bucketCount).toBe(2);
    const bucketOf = (id: string) => outcome.communities.get(id);
    expect(bucketOf("a0")).toBe(bucketOf("a5"));
    expect(bucketOf("b0")).toBe(bucketOf("b5"));
    expect(bucketOf("a0")).not.toBe(bucketOf("b0"));
  });

  it("归并后同区内边占比接近 1（拓扑保真，不是随机切分）", () => {
    const { communities, edges } = buildClusteredGraph();
    const outcome = mergeCommunitiesTopologically({ communities, edges, targetCount: 2 });
    expect(intraEdgeRatio(edges, outcome.communities)).toBeGreaterThan(0.9);
  });

  it("桶 id 是 0..bucketCount-1 的连续整数", () => {
    const { communities, edges } = buildClusteredGraph();
    const outcome = mergeCommunitiesTopologically({ communities, edges, targetCount: 2 });
    const buckets = [...new Set(outcome.communities.values())].sort((a, b) => a - b);
    expect(buckets).toEqual([0, 1]);
  });

  it("确定性：同一份输入两次归并得到完全相同的分桶", () => {
    const first = buildClusteredGraph();
    const second = buildClusteredGraph();
    const a = mergeCommunitiesTopologically({
      communities: first.communities,
      edges: first.edges,
      targetCount: 2,
    });
    const b = mergeCommunitiesTopologically({
      communities: second.communities,
      edges: second.edges,
      targetCount: 2,
    });
    expect([...a.communities.entries()].sort()).toEqual([...b.communities.entries()].sort());
  });

  it("不改变节点覆盖范围（不丢节点、不新增节点）", () => {
    const { communities, edges } = buildClusteredGraph();
    const outcome = mergeCommunitiesTopologically({ communities, edges, targetCount: 2 });
    expect(outcome.communities.size).toBe(communities.size);
    for (const id of communities.keys()) {
      expect(outcome.communities.has(id)).toBe(true);
    }
  });

  it("社区之间零连接时走兜底，桶 id 仍不越界", () => {
    const communities = new Map<string, number>([
      ["n1", 500],
      ["n2", 900],
      ["n3", 1200],
    ]);
    const outcome = mergeCommunitiesTopologically({ communities, edges: [], targetCount: 2 });
    expect(outcome.merged).toBe(true);
    expect(outcome.bucketCount).toBeLessThanOrEqual(2);
    for (const bucket of outcome.communities.values()) {
      expect(bucket).toBeGreaterThanOrEqual(0);
      expect(bucket).toBeLessThan(2);
    }
  });
});
