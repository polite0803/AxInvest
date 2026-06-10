#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""拆分被污染的反思复盘.json 并跑同一个分析。"""
import json, sys, re
sys.path.insert(0, r"d:\OneManager\AxInvest")
import importlib.util
spec = importlib.util.spec_from_file_location("analyze_wf", r"d:\OneManager\AxInvest\_analyze_wf.py")
mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod)

fp = r"C:\Users\polit\Downloads\A股反思复盘.json"
with open(fp, "rb") as f: raw = f.read().decode("utf-8")
print(f"原始文件大小: {len(raw)} chars")

# 找第二个 { 开头（即 reflection 对象）
m = re.search(r'\{\s*\n\s*"id":\s*"stock-reflection"', raw)
if not m:
    print("找不到 reflection 起点"); sys.exit(1)
start = m.start()
print(f"reflection 起点偏移: {start}")
# 找该对象对应的尾部 }  -- 从 start 用括号匹配
depth = 0; end = -1
for i, ch in enumerate(raw[start:], start):
    if ch == '{': depth += 1
    elif ch == '}':
        depth -= 1
        if depth == 0: end = i+1; break
print(f"reflection 终点: {end}, 长度 {end-start}")

obj_text = raw[start:end]
wf = json.loads(obj_text)
print(f"reflection 解析成功: id={wf['id']} name={wf['name']}")

# 临时写到新文件
out = r"C:\Users\polit\Downloads\_reflection_clean.json"
with open(out, "w", encoding="utf-8") as f: json.dump(wf, f, ensure_ascii=False)
print(f"已保存到: {out}")
mod.analyze(out)
