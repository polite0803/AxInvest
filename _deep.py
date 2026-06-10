#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import json
from collections import defaultdict, Counter

def dig(path):
    print("="*78)
    print(f"DEEP DIVE: {path}")
    print("="*78)
    with open(path, "r", encoding="utf-8") as f:
        wf = json.load(f)
    nodes = wf["nodes"]; edges = wf["edges"]
    nmap = {n["id"]: n for n in nodes}

    # 列出所有节点摘要
    print(f"\n[所有 {len(nodes)} 个节点]  (id, type, title, pos)")
    for n in nodes:
        p = n.get("position") or {}
        print(f"  {n['id']:28s}  {n.get('type','?'):14s}  ({p.get('x',0):6.0f},{p.get('y',0):6.0f})  {n.get('title','')[:40]}")

    # 每条边
    print(f"\n[所有 {len(edges)} 条边]")
    for e in edges:
        s = e.get("source") or e.get("from"); t = e.get("target") or e.get("to")
        sn = nmap.get(s, {}).get("title", s); tn = nmap.get(t, {}).get("title", t)
        print(f"  {s:25s} -> {t:25s}   ({sn} -> {tn})")

    # 触发器配置
    print(f"\n[trigger node 完整配置]")
    tr = next(n for n in nodes if n.get("type") == "trigger")
    print(json.dumps(tr, ensure_ascii=False, indent=2))

    # 孤立节点 p-analysts
    p = nmap.get("p-analysts")
    if p:
        print(f"\n[p-analysts 完整节点定义]")
        print(json.dumps(p, ensure_ascii=False, indent=2))

    # 入度 = 12 的 raw-data, 查看上游都是谁
    print(f"\n[raw-data 上游]")
    for e in edges:
        if (e.get("target") or e.get("to")) == "raw-data":
            s = e.get("source") or e.get("from")
            print(f"  <- {s}  ({nmap[s].get('title','')})")

    print(f"\n[debate-bull-bear 上游]")
    for e in edges:
        if (e.get("target") or e.get("to")) == "debate-bull-bear":
            s = e.get("source") or e.get("from")
            print(f"  <- {s}  ({nmap[s].get('title','')})")

    # 节点类型 = parallel 的两个
    print(f"\n[parallel 节点]")
    for n in nodes:
        if n.get("type") == "parallel":
            print(f"  id={n['id']}  title={n.get('title')}")
            print(f"    config: {json.dumps(n.get('config'), ensure_ascii=False)}")

    # tool 节点
    print(f"\n[tool 节点]")
    for n in nodes:
        if n.get("type") == "tool":
            cfg = n.get("config", {})
            print(f"  {n['id']:25s}  title={n.get('title',''):20s}  tool={cfg.get('tool_name') or cfg.get('name') or '?'}")

dig(r"C:\Users\polit\Downloads\_reflection_clean.json")
print()
dig(r"C:\Users\polit\Downloads\A股多维度分析.json")
