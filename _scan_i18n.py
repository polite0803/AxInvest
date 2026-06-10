#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""扫描所有 t("workflow.*") 调用，列出 locale 文件中缺失的 key。"""
import os, re, json
from pathlib import Path

ROOT = Path(r"d:\OneManager\AxInvest\src")
ZH = Path(r"d:\OneManager\AxInvest\src\i18n\locales\zh-CN.json")
EN = Path(r"d:\OneManager\AxInvest\src\i18n\locales\en-US.json")

# 收集所有 t("workflow.xxx") 调用
pattern = re.compile(r'''t\(\s*["'](workflow\.[A-Za-z0-9_.]+)["']''')
keys_used = set()
for ts_file in ROOT.rglob("*.ts*"):
    try:
        text = ts_file.read_text(encoding="utf-8")
    except Exception:
        continue
    for m in pattern.findall(text):
        keys_used.add(m)

# 收集 locale 已有的所有 workflow.* key（含嵌套）
def collect(obj, prefix=""):
    out = set()
    if isinstance(obj, dict):
        for k, v in obj.items():
            fk = f"{prefix}{k}"
            if isinstance(v, dict):
                out |= collect(v, fk + ".")
            else:
                out.add(fk)
    return out

zh_keys = collect(json.loads(ZH.read_text(encoding="utf-8")))
en_keys = collect(json.loads(EN.read_text(encoding="utf-8")))

missing_zh = sorted(k for k in keys_used if k not in zh_keys)
missing_en = sorted(k for k in keys_used if k not in en_keys)

print(f"=== 共有 {len(keys_used)} 个 t('workflow.*') 调用 ===")
print(f"=== zh-CN.json 缺 {len(missing_zh)} 个 ===")
for k in missing_zh:
    print(f"  - {k}")
print(f"=== en-US.json 缺 {len(missing_en)} 个 ===")
for k in missing_en:
    print(f"  - {k}")
