// 锁住 communityPalette 与 communityRadius 的契约（2026-09-16）
//
// 背景（归档缺陷）：调色板原为 **12 色**，而拓扑归并后的桶数是 **200**
// ⇒ 取色退化为 `communityPalette[cid % 12]`，200 个社区被压进 12 个色相
// ⇒ 以颜色为刻度的「分组可见性」量化（相邻节点同色率）上限被表示空间容量卡死
// （实测 1.062×，阈值 1.5 —— 即便分组在空间上完美分离也到不了）。
//
// ⚠ 这里刻意**不只测长度**：只断言 `length >= 200` 的话，一个「扩了长度但生成算法
// 大量碰撞」的实现照样能过 —— 而碰撞等于没扩（判据 #289）。所以三条都测：
// 容量、**唯一性**、以及「相邻 cid 必须异色」这个真正的目的。
import { describe, expect, it } from "vitest";
import {
  buildNodeColorCache,
  COMMUNITY_PALETTE_SIZE,
  COMMUNITY_RADIUS_MAX,
  COMMUNITY_RADIUS_MIN,
  communityPalette,
  communityRadius,
  getNodeColorMap,
} from "../graphViewUtils";

// 历史前 12 色（与迁移前逐字节一致）—— 小图 / 社区数 ≤ 12 时取色不得漂移
const LEGACY_12 = [
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

describe("communityPalette", () => {
  it("容量 ≥ 目标桶数（200），使 cid % length 成为恒等映射", () => {
    expect(COMMUNITY_PALETTE_SIZE).toBeGreaterThanOrEqual(200);
    expect(communityPalette.length).toBe(COMMUNITY_PALETTE_SIZE);
  });

  it("前 12 项与历史调色板逐字节一致（小图观感不漂移）", () => {
    expect(communityPalette.slice(0, LEGACY_12.length)).toEqual(LEGACY_12);
  });

  it("全表颜色唯一 —— 否则「扩容量」等于没扩", () => {
    const uniq = new Set(communityPalette);
    expect(uniq.size).toBe(communityPalette.length);
  });

  it("相邻 cid 必异色（分组的颜色可分辨性，这才是扩调色板的目的）", () => {
    let sameAdjacent = 0;
    for (let cid = 1; cid < communityPalette.length; cid++) {
      if (communityPalette[cid] === communityPalette[cid - 1]) { sameAdjacent++; }
    }
    expect(sameAdjacent).toBe(0);
  });

  it("全部取值是合法 #rrggbb", () => {
    for (const c of communityPalette) {
      expect(c).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });
});

describe("communityRadius", () => {
  it("随成员数单调不减，且被夹在上下界内", () => {
    expect(communityRadius(0)).toBe(COMMUNITY_RADIUS_MIN);
    let prev = -Infinity;
    for (const count of [1, 2, 5, 20, 100, 500, 5000, 100000]) {
      const r = communityRadius(count);
      expect(r).toBeGreaterThanOrEqual(COMMUNITY_RADIUS_MIN);
      expect(r).toBeLessThanOrEqual(COMMUNITY_RADIUS_MAX);
      expect(r).toBeGreaterThanOrEqual(prev);
      prev = r;
    }
  });

  it("判据①（2026-09-17 ③-D 的核心契约）：`r/√n ≥ 2·nodeSize/√π` —— 它决定「放大能否看清」", () => {
    // 推导：团内相邻节点屏幕间距 = `r·√(π/n)·zoom`、节点屏幕半径 = `nodeSize·zoom`
    // ⇒ 可分辨（间距 ≥ 2 倍半径）⟺ **`r/√n ≥ 2·nodeSize/√π`**。
    // ⚠ `zoom` 在两边同比出现、被完全约掉 ⇒ 这条判据**与放大倍数无关**：
    //   放大 N 倍只是把同样糊的一团整体放大 ⇒ 用户报的「放大到上限 5 倍仍看不清」
    //   不可能靠调缩放范围解决（实测旧系数 2.2 给出 `minSepRatio = 1.45`，差 3.9 倍）。
    const required = (2 * 5) / Math.sqrt(Math.PI); // nodeSize 典型值 5（区间 [4,22]）
    for (const count of [10, 73, 121, 500, 921, 1434]) {
      expect(communityRadius(count) / Math.sqrt(count)).toBeGreaterThanOrEqual(required);
    }
  });

  it("判据① 的失效边界也被钉住：n 超出 `(MAX−BASE)²/K²` 后上限封顶 ⇒ 不再满足", () => {
    // 这条断言的作用是**防止有人调小上限后以为「测试全绿 = 判据仍成立」**：
    // `COMMUNITY_RADIUS_MAX` 是判据① 的唯一失效来源，改它必须同步重跑标定。
    const required = (2 * 5) / Math.sqrt(Math.PI);
    expect(communityRadius(4000) / Math.sqrt(4000)).toBeLessThan(required);
  });

  it("退化输入不产生 NaN（成员数为 0 / 负数）", () => {
    expect(Number.isFinite(communityRadius(0))).toBe(true);
    expect(Number.isFinite(communityRadius(-5))).toBe(true);
  });
});

// ── 节点类型色（2026-10-28 收编进类目色板）──
//
// 收编前的缺陷：`getNodeColorMap` 里 note/concept 走 antd 语义 token、entity/source 是手抄
// hex；而 `buildNodeColorCache` 在「没传 token」时另有一份四色全 hex 的表 —— 同一节点
// 经两条路径取到不同颜色。现在两份合成一份，取值全部来自 `@/theme/categoricalColors`。
function node(id: string, type: string) {
  return {
    id,
    title: id,
    type: type as never,
    tags: [],
    linkCount: 1,
    backlinkCount: 0,
    path: `/${id}`,
  };
}

describe("getNodeColorMap", () => {
  const KINDS = ["note", "concept", "entity", "source"] as const;

  it("亮 / 暗两档各自四类互异 —— 撞色等于图上分不出类型", () => {
    for (const isDark of [false, true]) {
      const map = getNodeColorMap(isDark);
      const colors = KINDS.map((k) => map[k]);
      expect(new Set(colors).size).toBe(KINDS.length);
    }
  });

  it("四类颜色随明暗档变化（不是一档写死两档共用）", () => {
    for (const kind of KINDS) {
      expect(getNodeColorMap(false)[kind]).not.toBe(getNodeColorMap(true)[kind]);
    }
  });

  it("buildNodeColorCache 无社区分色时逐类等于 getNodeColorMap（单一来源）", () => {
    for (const isDark of [false, true]) {
      const map = getNodeColorMap(isDark);
      const cache = buildNodeColorCache(KINDS.map((k) => node(k, k)), undefined, isDark);
      for (const kind of KINDS) {
        expect(cache.get(kind)).toBe(map[kind]);
      }
    }
  });

  it("词表外的后端类型静默落 note 色（这是已登记的产品待裁，不是取色故障）", () => {
    const cache = buildNodeColorCache([node("x", "daily")], undefined, false);
    expect(cache.get("x")).toBe(getNodeColorMap(false).note);
  });

  it("有社区分色时优先社区色，且仍取自 communityPalette（与聚合彩球同源）", () => {
    const communities = new Map([["n1", 3]]);
    const cache = buildNodeColorCache([node("n1", "note")], communities, false);
    expect(cache.get("n1")).toBe(communityPalette[3]);
  });
});
