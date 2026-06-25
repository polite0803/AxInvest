#!/usr/bin/env python3
"""Scan all prompt files for conflicts between VERDICT tag format and old JSON examples."""
import os, glob

BASE = "src-tauri/agency_experts/stock-analysis"
files = glob.glob(os.path.join(BASE, "*.md")) + glob.glob(os.path.join(BASE, "custom", "*.md"))

print(f"{'文件':35s} {'VERDICT':8s} {'旧少样本':10s} {'json块':8s} {'VERDICT示例':12s} {'拒绝指令':8s}")
print("-"*85)
issues = []
for fp in sorted(files):
    with open(fp, "r", encoding="utf-8") as f:
        content = f.read()
    basename = os.path.basename(fp)
    
    has_verdict = "VERDICT" in content
    has_old_example = "## 少样本（good）" in content
    has_json_block = "```json" in content
    has_verdict_example = "VERDICT:" in content or "VERDICT " in content
    has_refuse = "绝不允许" in content or "不要拒绝" in content
    
    print(f"{basename:35s} {str(has_verdict):8s} {str(has_old_example):10s} {str(has_json_block):8s} {str(has_verdict_example):12s} {str(has_refuse):8s}")
    
    problems = []
    if has_old_example:
        problems.append("残留[少样本(good)]旧JSON示例")
    if has_verdict and has_json_block:
        problems.append("VERDICT格式但含```json块（可能是旧示例）")
    if has_verdict and not has_verdict_example and "portfolio" not in basename:
        problems.append("VERDICT输出格式但无VERDICT示例")
    if not has_refuse and "portfolio" not in basename and "reflection" not in basename and "quality-fallback" not in basename:
        problems.append("缺[绝不允许拒绝回答]指令")
    if problems:
        issues.append((basename, problems))

print("\n=== 问题文件 ===")
for name, probs in issues:
    for p in probs:
        print(f"  {name}: {p}")
print(f"\n共检查 {len(files)} 个文件, {len(issues)} 个存在问题")
