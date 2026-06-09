#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""工作流 JSON 结构分析器：检测布局错误。"""
import json, sys, os
from collections import defaultdict, Counter

def bbox(node, default_w=200, default_h=80):
    p = node.get("position") or {}
    x = p.get("x", 0); y = p.get("y", 0)
    sz = node.get("size") or {}
    w = sz.get("width", default_w); h = sz.get("height", default_h)
    return (x, y, x + w, y + h, w, h)

def overlap(a, b):
    ax1, ay1, ax2, ay2, _, _ = a
    bx1, by1, bx2, by2, _, _ = b
    ix = max(0, min(ax2, bx2) - max(ax1, bx1))
    iy = max(0, min(ay2, by2) - max(ay1, by1))
    return ix * iy

def analyze(path):
    print("="*78)
    print(f"FILE: {path}")
    print("="*78)
    with open(path, "r", encoding="utf-8") as f:
        wf = json.load(f)

    print(f"\n[id]            {wf.get('id')}")
    print(f"[name]          {wf.get('name')}")
    print(f"[description]   {wf.get('description')}")
    print(f"[version]       {wf.get('version')}")
    print(f"[is_preset]     {wf.get('is_preset')}    [editable] {wf.get('is_editable')}    [public] {wf.get('is_public')}")
    print(f"[trigger]       {json.dumps(wf.get('trigger_config'), ensure_ascii=False)[:160]}")

    nodes = wf.get("nodes", [])
    edges = wf.get("edges", [])
    groups = wf.get("groups", []) or wf.get("containers", []) or []
    print(f"\n[counts] nodes={len(nodes)}  edges={len(edges)}  groups={len(groups)}")

    ids = [n.get("id") for n in nodes]
    dup_ids = [k for k, v in Counter(ids).items() if v > 1]
    if dup_ids:
        print(f"\n[ERR] 重复节点 ID: {dup_ids}")

    no_pos = [n.get("id") for n in nodes if not n.get("position")]
    if no_pos:
        print(f"\n[ERR] 缺 position 的节点: {no_pos}")

    disabled = [n.get("id") for n in nodes if n.get("enabled") is False]
    if disabled:
        print(f"\n[WARN] 被禁用的节点: {disabled}")

    id_set = set(ids)
    bad_edges = []
    for e in edges:
        s = e.get("source") or e.get("from")
        t = e.get("target") or e.get("to")
        if s not in id_set or t not in id_set:
            bad_edges.append(e)
    if bad_edges:
        print(f"\n[ERR] 边指向不存在的节点 ({len(bad_edges)}):")
        for e in bad_edges[:10]:
            print(f"      {e}")

    loops = [e for e in edges if (e.get("source") or e.get("from")) == (e.get("target") or e.get("to"))]
    if loops:
        print(f"\n[WARN] 自环边 ({len(loops)}):")
        for e in loops[:5]:
            print(f"      {e}")

    in_deg = Counter(); out_deg = Counter()
    for e in edges:
        s = e.get("source") or e.get("from")
        t = e.get("target") or e.get("to")
        if s in id_set: out_deg[s] += 1
        if t in id_set: in_deg[t] += 1
    isolated = [nid for nid in id_set if in_deg[nid] == 0 and out_deg[nid] == 0 and nid != "trigger"]
    no_in = [nid for nid in id_set if in_deg[nid] == 0 and out_deg[nid] > 0]
    no_out = [nid for nid in id_set if in_deg[nid] > 0 and out_deg[nid] == 0 and nid != wf.get("end_id", "end")]
    print(f"\n[孤立节点] (无入度且无出度, 已排除 trigger): {len(isolated)} 个 -> {isolated[:10]}")
    if len(isolated) > 10: print(f"      ... 还有 {len(isolated)-10} 个")
    print(f"[无入度非 trigger 节点] (起点, 应只有 1 个): {no_in}")
    print(f"[无出度非 end 节点]   (终态): {no_out}")

    multi_in = {nid: c for nid, c in in_deg.items() if c >= 3}
    if multi_in:
        print(f"\n[WARN] 入度 >=3 的节点 (汇流瓶颈?):")
        for nid, c in sorted(multi_in.items(), key=lambda x: -x[1]):
            n = next(n for n in nodes if n["id"] == nid)
            print(f"      {nid:30s} 入度={c:2d}  type={n.get('type'):14s}  title={n.get('title','')}")

    if nodes:
        xs = [n["position"]["x"] for n in nodes if n.get("position")]
        ys = [n["position"]["y"] for n in nodes if n.get("position")]
        print(f"\n[画布尺寸] x∈[{min(xs):.0f},{max(xs):.0f}]  y∈[{min(ys):.0f},{max(ys):.0f}]  "
              f"跨度 {max(xs)-min(xs):.0f} x {max(ys)-min(ys):.0f}")

    box = {n["id"]: bbox(n) for n in nodes if n.get("position")}
    overlap_pairs = []
    keys = list(box.keys())
    for i, a in enumerate(keys):
        for b in keys[i+1:]:
            if overlap(box[a], box[b]) > 0:
                overlap_pairs.append((a, b, overlap(box[a], box[b])))
    if overlap_pairs:
        print(f"\n[ERR] 节点边界框重叠 ({len(overlap_pairs)} 对):")
        for a, b, area in overlap_pairs[:15]:
            na = next(n for n in nodes if n["id"] == a)
            nb = next(n for n in nodes if n["id"] == b)
            print(f"      {a:25s}  <->  {b:25s}  area={area:.0f}  ({na.get('title','')} / {nb.get('title','')})")
    else:
        print(f"\n[OK]  节点之间无边界框重叠")

    centers = {nid: ((box[nid][0]+box[nid][2])/2, (box[nid][1]+box[nid][3])/2) for nid in box}
    def seg_intersect(p1, p2, p3, p4):
        def ccw(A, B, C): return (C[1]-A[1])*(B[0]-A[0]) > (B[1]-A[1])*(C[0]-A[0])
        return ccw(p1,p3,p4) != ccw(p2,p3,p4) and ccw(p1,p2,p3) != ccw(p1,p2,p4)
    crossings = 0
    edge_list = [(e.get("source") or e.get("from"), e.get("target") or e.get("to")) for e in edges]
    for i, (s1, t1) in enumerate(edge_list):
        if s1 not in centers or t1 not in centers: continue
        p1, p2 = centers[s1], centers[t1]
        for s2, t2 in edge_list[i+1:]:
            if s2 not in centers or t2 not in centers: continue
            if len({s1,t1,s2,t2}) < 4: continue
            p3, p4 = centers[s2], centers[t2]
            if seg_intersect(p1, p2, p3, p4):
                crossings += 1
    print(f"\n[边线交叉数] (中心点近似, 共享端点不计): {crossings}")

    type_cnt = Counter(n.get("type","?") for n in nodes)
    print(f"\n[节点类型分布]")
    for t, c in type_cnt.most_common():
        print(f"      {t:20s} x {c}")

    if groups:
        print(f"\n[分组 groups] 共 {len(groups)} 个")
        for g in groups[:20]:
            print(f"      id={g.get('id'):25s}  children={g.get('childrenIds') or g.get('nodes') or '?'}  "
                  f"pos={g.get('position')}  size={g.get('size')}")
    else:
        print(f"\n[分组] 无 groups 字段 (key 名可能不同)")

    print(f"\n[顶层 keys]: {list(wf.keys())}")

    titles = Counter(n.get("title","") for n in nodes)
    dup_titles = {k:v for k,v in titles.items() if v>1}
    if dup_titles:
        print(f"\n[WARN] 重复 title (可能重复节点):")
        for t, c in dup_titles.items():
            print(f"      {t}  x{c}")
            for n in nodes:
                if n.get("title") == t:
                    print(f"           - id={n.get('id'):25s} pos=({n.get('position',{}).get('x'):.0f},{n.get('position',{}).get('y'):.0f})  type={n.get('type')}")

    parent_map = {n["id"]: n.get("parentId") for n in nodes}
    bad_parent = [nid for nid, p in parent_map.items() if p and p not in id_set]
    if bad_parent:
        print(f"\n[ERR] parentId 指向不存在的节点: {bad_parent}")

if __name__ == "__main__":
    for p in sys.argv[1:]:
        analyze(p)
        print()
