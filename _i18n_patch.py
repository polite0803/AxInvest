"""
补齐 workflow.* 缺失键（zh-CN + en-US）。
策略：
- 直接修改 JSON 对象的 nested `workflow` 字典
- 保持文件其余部分完全不动（缩进、引号、键顺序）
- 输出原文件经 json.dump(indent=2, ensure_ascii=False) 以保证格式统一
"""
import json
import re
from pathlib import Path
from collections import OrderedDict

ROOT = Path(r"d:\OneManager\AxInvest")
ZH = ROOT / "src/i18n/locales/zh-CN.json"
EN = ROOT / "src/i18n/locales/en-US.json"

# 缺失键的中英文翻译表
# 顺序：调用点上下文 → 含义 → 默认值（来自代码的 defaultValue 或推断）
TRANSLATIONS = {
    # === canvasTitle ===
    "canvasTitle.clickToRename": ("点击重命名", "Click to rename"),
    # === containerNode ===
    "containerNode.branchTimeout": ("分支超时", "Branch timeout"),
    "containerNode.nodes": ("节点", "nodes"),
    # === 顶层 ===
    "decorativeContainerNoEdges": ("装饰容器不能有边", "Decorative containers cannot have edges"),
    # === groupNode ===
    "groupNode.folded": ("已折叠", "folded"),
    "groupNode.untitled": ("分组", "Group"),
    # === leftPanel ===
    "leftPanel.layout": ("布局", "Layout"),
    # === legend ===
    "legend.title": ("节点颜色", "Node Colors"),
    # === node ===
    "node.clickToCollapsePorts": ("点击折叠端口", "Click to collapse ports"),
    "node.clickToExpandPorts": ("点击展开端口", "Click to expand ports"),
    "node.inputs": ("输入", "Inputs"),
    "node.outputs": ("输出", "Outputs"),
    "node.type": ("类型", "Type"),
    # === nodeTypes ===
    "nodeTypes.groupFrame": ("分组框", "Group Frame"),
    "nodeTypes.phaseSeparator": ("阶段分隔", "Phase Separator"),
    "nodeTypes.storage": ("存储", "Storage"),
    # === parallelNode ===
    "parallelNode.decorative": ("装饰", "Decorative"),
    # === phaseSeparator ===
    "phaseSeparator.defaultLabel": ("阶段", "Phase"),
    # === props (Aggregator / Switch / Parallel / Condition) ===
    "props.aggregAll": ("全部（数组）", "All (array)"),
    "props.aggregConcat": ("拼接（字符串）", "Concat (string)"),
    "props.aggregConcatHint": ("将字符串拼接", "Joins string values together"),
    "props.aggregCount": ("计数", "Count"),
    "props.aggregLlmHint": ("使用大模型总结各分支", "Use an LLM to summarize the branches"),
    "props.aggregLlmSummarize": ("LLM 总结", "LLM Summarize"),
    "props.aggregMerge": ("合并（对象）", "Merge (object)"),
    "props.aggregMergeHint": ("将多个对象合并成一个", "Merge multiple objects into one"),
    "props.aggregStrategy": ("聚合策略", "Aggregation Strategy"),
    "props.aggregSum": ("求和（数值）", "Sum (numeric)"),
    "props.aggregWeighted": ("加权（数值）", "Weighted (numeric)"),
    "props.aggregWeightedHint": ("按权重对各输入求加权和", "Compute a weighted sum over inputs"),
    "props.branchTimeoutMs": ("超时（毫秒）", "Timeout (ms)"),
    "props.defaultCase": ("默认分支（兜底）", "Default case (fallback)"),
    "props.defaultModel": ("使用默认模型", "Use default model"),
    "props.degradeOnTimeout": ("超时降级", "Degrade on timeout"),
    "props.expressionHint": ("表达式基于上一节点的 _value 计算，返回布尔值", "Expression is evaluated against the previous node's _value; result is a boolean"),
    "props.expressionPlaceholder": ("_value > 100", "_value > 100"),
    "props.llmRoutingModel": ("模型（可选）", "Model (optional)"),
    "props.llmRoutingPrompt": ("路由提示词", "Routing Prompt"),
    "props.llmRoutingPromptPlaceholder": ("请根据输入选择最匹配的分支：{{branches}}", "Pick the best branch for the input: {{branches}}"),
    "props.matchModeContains": ("包含", "Contains"),
    "props.matchModeExact": ("精确", "Exact"),
    "props.matchModeExpression": ("表达式", "Expression"),
    "props.matchModeRegex": ("正则", "Regex"),
    "props.summarizeModel": ("模型（可选）", "Model (optional)"),
    "props.summarizePrompt": ("总结提示词", "Summarize prompt"),
    "props.summarizePromptPlaceholder": ("请总结以下输入：{{inputs}}", "Please summarize the following inputs: {{inputs}}"),
    "props.useLlmRouting": ("LLM 智能路由", "LLM Smart Routing"),
    "props.waitForAll": ("等待所有输入", "Wait for all inputs"),
    "props.weights": ("权重（逗号分隔）", "Weights (comma-separated)"),
    "props.weightsPlaceholder": ("例如 0.5, 0.3, 0.2", "e.g. 0.5, 0.3, 0.2"),
    # === swarmNode ===
    "swarmNode.agents": ("代理", "Agents"),
    "swarmNode.rounds": ("轮次", "rounds"),
    # === triggerNode ===
    "triggerNode.configure": ("配置", "Configure"),
    "triggerNode.configureHint": ("点击配置触发器", "Click to configure the trigger"),
    "triggerNode.statusActive": ("已启用", "Active"),
    "triggerNode.statusDisabled": ("已禁用", "Disabled"),
    "triggerNode.statusUnconfigured": ("未配置", "Unconfigured"),
    # === versionHistory ===
    "versionHistory.latest": ("最新", "Latest"),
    "versionHistory.rollback": ("回滚", "Rollback"),
    "versionHistory.rollbackConfirm": ("确认回滚到版本 {{version}}？", "Rollback to version {{version}}?"),
    "versionHistory.rollbackTo": ("回滚到 {{v}}", "Rollback to {{v}}"),
    "versionHistory.rolledBack": ("已回滚到版本 {{version}}", "Rolled back to version {{version}}"),
}

assert len(TRANSLATIONS) == 62, f"expected 62, got {len(TRANSLATIONS)}"

def set_nested(d, dotted_key, value):
    parts = dotted_key.split(".")
    cur = d
    for p in parts[:-1]:
        if p not in cur or not isinstance(cur[p], dict):
            cur[p] = {}
        cur = cur[p]
    cur[parts[-1]] = value

def patch(locale_path, lang):
    with open(locale_path, "r", encoding="utf-8") as f:
        data = json.load(f, object_pairs_hook=OrderedDict)
    idx = lang  # 0=zh, 1=en
    missing = []
    for k, trans in TRANSLATIONS.items():
        value = trans[idx]
        # 检查是否已经存在（避免覆盖已有翻译）
        cur = data.get("workflow", {})
        exists = True
        for p in k.split("."):
            if isinstance(cur, dict) and p in cur:
                cur = cur[p]
            else:
                exists = False
                break
        if exists and isinstance(cur, str):
            print(f"  [SKIP] {locale_path.name}: {k} already exists -> {cur!r}")
            continue
        if "workflow" not in data or not isinstance(data["workflow"], dict):
            data["workflow"] = OrderedDict()
        set_nested(data["workflow"], k, value)
        missing.append(k)
    print(f"  [{locale_path.name}] added {len(missing)} keys")
    with open(locale_path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
    return missing

print("=== Patching zh-CN.json ===")
patch(ZH, 0)
print("=== Patching en-US.json ===")
patch(EN, 1)
print("Done.")
