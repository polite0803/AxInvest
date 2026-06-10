"""
最终验证：JSON 语法 + 关键解析（模拟 i18next 嵌套查找）
"""
import json
import re
from pathlib import Path

ROOT = Path(r"d:\OneManager\AxInvest")
ZH = ROOT / "src/i18n/locales/zh-CN.json"
EN = ROOT / "src/i18n/locales/en-US.json"

# 1. JSON 语法
for p in (ZH, EN):
    try:
        with open(p, "r", encoding="utf-8") as f:
            data = json.load(f)
        print(f"[OK] {p.name} parsed ({len(json.dumps(data, ensure_ascii=False))} bytes)")
    except Exception as e:
        print(f"[FAIL] {p.name}: {e}")
        raise

# 2. 模拟 i18next 嵌套解析
def lookup(data, dotted):
    cur = data
    for p in dotted.split("."):
        if isinstance(cur, dict) and p in cur:
            cur = cur[p]
        else:
            return None
    return cur

with open(ZH, "r", encoding="utf-8") as f:
    zh = json.load(f)
with open(EN, "r", encoding="utf-8") as f:
    en = json.load(f)

# 之前有问题的 3 个键 + 抽样的 5 个新增键
SAMPLE = [
    "workflow.run",
    "workflow.save",
    "workflow.settings",
    "workflow.legend.title",
    "workflow.triggerNode.statusActive",
    "workflow.props.aggregStrategy",
    "workflow.props.matchModeRegex",
    "workflow.versionHistory.rollbackConfirm",
    "workflow.swarmNode.agents",
    "workflow.nodeTypes.phaseSeparator",
    "workflow.decorativeContainerNoEdges",
    "workflow.groupNode.untitled",
]

print("\n=== i18next nested resolution (zh / en) ===")
for k in SAMPLE:
    z = lookup(zh, k)
    e = lookup(en, k)
    print(f"  {k:48s}  zh={z!r:30s}  en={e!r}")

# 3. 总键数
def count_nested(obj, prefix=""):
    total = 0
    for k, v in obj.items():
        cur = f"{prefix}.{k}" if prefix else k
        if isinstance(v, dict):
            total += count_nested(v, cur)
        else:
            total += 1
    return total

print(f"\n[ZH] total workflow.* leaf keys: {count_nested(zh.get('workflow', {}), 'workflow')}")
print(f"[EN] total workflow.* leaf keys: {count_nested(en.get('workflow', {}), 'workflow')}")
