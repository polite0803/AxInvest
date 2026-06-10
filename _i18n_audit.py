"""
扫描工作流 i18n 键状态：
- 代码侧：所有 t("workflow.*") 调用
- locale 侧：nested `workflow` 对象下的所有键 + 顶层 flat `workflow.*` 键
- 输出：缺失键清单 + 重复 flat 键清单
"""
import json
import re
import os
from pathlib import Path
from collections import defaultdict

ROOT = Path(r"d:\OneManager\AxInvest")
SRC = ROOT / "src"
ZH = ROOT / "src/i18n/locales/zh-CN.json"
EN = ROOT / "src/i18n/locales/en-US.json"

# 1. 收集所有 t("workflow.*") 调用
code_keys = set()
pattern = re.compile(r't\(\s*["\']workflow\.([a-zA-Z0-9_.]+)["\']\s*[,)]')
for ts_file in SRC.rglob("*.ts"):
    if ".test." in ts_file.name:
        continue
    try:
        text = ts_file.read_text(encoding="utf-8")
    except Exception:
        continue
    for m in pattern.findall(text):
        code_keys.add(m)
for tsx_file in SRC.rglob("*.tsx"):
    if ".test." in tsx_file.name:
        continue
    try:
        text = tsx_file.read_text(encoding="utf-8")
    except Exception:
        continue
    for m in pattern.findall(text):
        code_keys.add(m)

print(f"[CODE] t(\"workflow.*\") distinct keys: {len(code_keys)}")

# 2. 收集 locale 文件中的 nested + flat 键
def collect_keys(locale_path):
    with open(locale_path, "r", encoding="utf-8") as f:
        data = json.load(f)
    nested = set()
    flat = set()
    if "workflow" in data and isinstance(data["workflow"], dict):
        def walk(prefix, obj):
            for k, v in obj.items():
                cur = f"{prefix}.{k}" if prefix else k
                if isinstance(v, dict):
                    walk(cur, v)
                else:
                    nested.add(cur)
        walk("", data["workflow"])
    for k in data.keys():
        if k.startswith("workflow.") and isinstance(data[k], str):
            flat.add(k[len("workflow."):])
    return nested, flat, data

zh_nested, zh_flat, zh_data = collect_keys(ZH)
en_nested, en_flat, en_data = collect_keys(EN)

print(f"[ZH] nested keys: {len(zh_nested)}, flat keys: {len(zh_flat)}")
print(f"[EN] nested keys: {len(en_nested)}, flat keys: {len(en_flat)}")

# 3. 交叉对比
zh_missing = code_keys - zh_nested
en_missing = code_keys - en_nested
zh_only_in_flat = zh_flat - zh_nested
en_only_in_flat = en_flat - en_nested

print(f"\n=== MISSING in zh nested: {len(zh_missing)} ===")
for k in sorted(zh_missing):
    print(f"  {k}")
print(f"\n=== MISSING in en nested: {len(en_missing)} ===")
for k in sorted(en_missing):
    print(f"  {k}")
print(f"\n=== flat-only (legacy dup) in zh: {len(zh_only_in_flat)} ===")
for k in sorted(zh_only_in_flat):
    print(f"  workflow.{k}")
print(f"\n=== flat-only (legacy dup) in en: {len(en_only_in_flat)} ===")
for k in sorted(en_only_in_flat):
    print(f"  workflow.{k}")
