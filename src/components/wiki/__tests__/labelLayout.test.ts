import { describe, expect, it } from "vitest";
import { type LabelCandidate, selectLabelsToDraw } from "../labelLayout";

// 度量注入用「每字符 6 世界单位」，避免依赖 canvas 的真实字体度量（jsdom 无字体）。
const measure = (t: string) => t.length * 6;
const mk = (id: string, x: number, y: number, size: number, title = "aaaa"): LabelCandidate => ({
  id,
  x,
  y,
  size,
  title,
});
const base = {
  fontSizeWorld: 10,
  labelOffsetWorld: 2,
  cap: 100,
  measure,
};

describe("selectLabelsToDraw", () => {
  it("空候选 / cap=0 ⇒ 不画任何标签", () => {
    expect(selectLabelsToDraw([], base)).toEqual([]);
    expect(selectLabelsToDraw([mk("a", 0, 0, 5)], { ...base, cap: 0 })).toEqual([]);
  });

  it("标签锚点 = 节点中心 + 半径 + 偏移（世界坐标）", () => {
    const out = selectLabelsToDraw([mk("a", 7, 10, 4)], base);
    expect(out).toHaveLength(1);
    expect(out[0].labelX).toBe(7);
    expect(out[0].labelY).toBe(16); // 10 + 4 + 2
  });

  it("网格分格：同一格内只保留 size 最大的（分布由格子决定，不是全局 Top-N）", () => {
    // 远处一点把包围盒撑到 100×100 ⇒ 格边长 ≈ 8.33 ⇒ a 与 b 落在同一格
    const out = selectLabelsToDraw(
      [mk("a", 0, 0, 1), mk("b", 0.001, 0, 5), mk("far", 100, 100, 2)],
      base,
    );
    const ids = out.map((d) => d.id);
    expect(ids).toContain("b");
    expect(ids).not.toContain("a"); // 同格内被 size 更大的 b 取代
  });

  it("矩形占位：世界坐标重叠的标签被跳过", () => {
    // 包围盒 110×110 ⇒ 格边长 ≈ 9.17 ⇒ a(0,0) 与 b(10,0) 在不同格，但矩形仍相交
    // a: halfW = 4*6/2 + 10*0.3 = 15 ⇒ x∈[-15,15]；b: x∈[-5,25] ⇒ 相交
    const out = selectLabelsToDraw(
      [mk("a", 0, 0, 5), mk("b", 10, 0, 4), mk("far", 110, 110, 1)],
      base,
    );
    const ids = out.map((d) => d.id);
    expect(ids).toContain("a");
    expect(ids).not.toContain("b");
    expect(ids).toContain("far");
  });

  it("留白生效：几何上「不重叠但紧贴」的标签也被跳过（gapRatio=0 时则保留）", () => {
    // b 在 a 右侧 28：占位矩形（有留白）相交 ⇒ 跳过；去掉留白后不交 ⇒ 保留
    const cands = [mk("a", 0, 0, 5), mk("b", 28, 0, 4), mk("far", 200, 200, 1)];
    const withGap = selectLabelsToDraw(cands, base);
    expect(withGap.map((d) => d.id)).not.toContain("b");

    const noGap = selectLabelsToDraw(cands, { ...base, gapRatio: 0 });
    expect(noGap.map((d) => d.id)).toContain("b");
  });

  it("cap 截断：超过上限时按 size 降序保留", () => {
    const cands = [
      mk("a", 0, 0, 1),
      mk("b", 100, 0, 9),
      mk("c", 0, 100, 5),
      mk("d", 100, 100, 3),
    ];
    const out = selectLabelsToDraw(cands, { ...base, cap: 2 });
    expect(out).toHaveLength(2);
    expect(out.map((d) => d.id).sort()).toEqual(["b", "c"]); // size 9 与 5
  });

  it("maxPlaced 是硬闸：即使互不重叠也不超过上限", () => {
    const cands = [
      mk("a", 0, 0, 5),
      mk("b", 100, 0, 4),
      mk("c", 0, 100, 3),
      mk("d", 100, 100, 2),
    ];
    expect(selectLabelsToDraw(cands, { ...base, maxPlaced: 1 })).toHaveLength(1);
  });

  it("确定性：同格同 size 时按 id 字典序取，且两次调用结果逐项相同（防逐帧抖动）", () => {
    const cands = [mk("z", 0, 0, 5), mk("a", 0.001, 0, 5), mk("far", 1000, 1000, 1)];
    const r1 = selectLabelsToDraw(cands, base);
    const r2 = selectLabelsToDraw(cands, base);
    expect(r1.map((d) => d.id)).toEqual(r2.map((d) => d.id));
    expect(r1.map((d) => d.id)).toContain("a");
    expect(r1.map((d) => d.id)).not.toContain("z");
  });

  it("不改动入参（纯函数）", () => {
    const cands = [mk("a", 0, 0, 5), mk("b", 100, 100, 4)];
    const before = JSON.stringify(cands);
    selectLabelsToDraw(cands, { ...base, cap: 1 });
    expect(JSON.stringify(cands)).toBe(before);
  });
});
