#!/usr/bin/env python3
"""Convert all analyst/debater/risk-evaluator prompts from JSON output to VERDICT tag format.

Changes:
- Output format: natural language + <!-- VERDICT: {...} --> tag at end
- Removes old JSON schema requirement
- Simplifies examples
"""

import re, os, glob

BASE = os.path.join("src-tauri", "agency_experts", "stock-analysis")

files = glob.glob(os.path.join(BASE, "*.md")) + glob.glob(os.path.join(BASE, "custom", "*.md"))
exclude = ["portfolio-manager", "research-manager", "quality-fallback", "reflection"]
files = [f for f in files if not any(x in f for x in exclude)]

ANALYST_FMT = """\
## 输出格式

输出你的完整分析报告（自然语言，可包含Markdown表格/清单/推理过程），
然后在**末尾另起一行**追加机读标签：

```
<!-- VERDICT: {"verdict": "看多", "bull_score": 65, "bear_score": 35, "confidence": 70} -->
```

VERDICT标签字段说明：
- `verdict`: "看多 | 偏多 | 中性 | 偏空 | 看空"
- `bull_score` / `bear_score`: 0-100整数
- `confidence`: 0-100整数

**关键规则**：
1. 报告正文是自由自然语言，任意格式都可以
2. VERDICT标签必须是输出内容的**最后一行**
3. VERDICT内部JSON必须合法（键名用双引号、无尾逗号）
"""

DEBATER_FMT = """\
## 输出格式

输出你的完整辩论观点（自然语言，可包含表格/引用/推理），
然后在**末尾另起一行**追加机读标签：

```
<!-- VERDICT: {"stance": "bullish", "strength_score": 65, "confidence": 70} -->
```

- `stance`: "bullish | bearish"
- `strength_score`: 0-100整数
- `confidence`: 0-100整数
"""

RISK_FMT = """\
## 输出格式

输出你的完整风险评估（自然语言），
然后在**末尾另起一行**追加机读标签：

```
<!-- VERDICT: {"stance": "aggressive", "position_pct": 50, "confidence": 70} -->
```

- `stance`: "aggressive | conservative | neutral"
- `position_pct`: 0-100整数，建议仓位
- `confidence`: 0-100整数
"""

DEBATE_CONV_FMT = """\
## 输出格式

输出你的完整收敛分析（自然语言），
然后在**末尾另起一行**追加机读标签：

```
<!-- VERDICT: {"consensus_score": 65, "direction": "bullish", "confidence": 70} -->
```

- `consensus_score`: 0-100整数，60+视为基本共识
- `direction`: "bullish | bearish | neutral | divided"
- `confidence`: 0-100整数
"""

EXAMPLE = """\
## 参考示例

```
近20日价格区间收敛至28.5-32.0，均线系统纠缠。成交量较20日均量缩35%。

**结论**：当前处于震荡格局，无明确突破信号，建议观望。

<!-- VERDICT: {"verdict": "中性", "bull_score": 40, "bear_score": 50, "confidence": 70} -->
```"""

count = 0
for fp in files:
    with open(fp, 'r', encoding='utf-8') as f:
        content = f.read()
    orig = content

    basename = os.path.basename(fp)
    
    # Determine format based on filename patterns
    is_debate_conv = "debate-convergence" in basename
    is_debater = any(r in basename for r in ["bull-", "bear-"]) and not is_debate_conv
    is_trader = "trader" in basename and "research" not in basename
    is_risk = any(r in basename for r in ["aggressive", "conservative", "neutral", "risk-convergence", "rule-checker"])
    
    if is_debate_conv:
        fmt = DEBATE_CONV_FMT
    elif is_debater:
        fmt = DEBATER_FMT
    elif is_risk:
        fmt = RISK_FMT
    elif is_trader:
        fmt = ANALYST_FMT  # trader already handled separately
    else:
        fmt = ANALYST_FMT
    
    # Only update files that still have "## 输出格式" section
    if "## 输出格式" not in content:
        continue
    
    # Replace the output format section
    content = re.sub(
        r'## 输出格式\n.*?(?=\n## |\Z)',
        fmt,
        content,
        flags=re.DOTALL
    )
    
    # Replace reference example
    content = re.sub(
        r'## 参考示例\n.*?(?=\n## |\n```\n|```\n)',
        EXAMPLE + "\n",
        content,
        flags=re.DOTALL
    )
    
    # Remove stale 字段口径 section
    content = re.sub(r'\n字段口径：\n\n- .+', '', content, flags=re.DOTALL)
    
    # Remove 少样本 bad examples
    content = re.sub(r'## 少样本（bad，反例）\n\n.*?(?=\n## |\Z)', '', content, flags=re.DOTALL)
    
    # Simplify self-check
    content = re.sub(
        r'## 自检（输出前必过）\n\n- .+',
        '## 自检\n\n- [ ] 报告正文是否完整覆盖了所有关键论据？\n- [ ] VERDICT标签是否在输出的最后一行？\n- [ ] VERDICT内JSON是否合法？',
        content,
        flags=re.DOTALL
    )
    
    if content != orig:
        with open(fp, 'w', encoding='utf-8') as f:
            f.write(content)
        print(f"[OK] {basename}")
        count += 1

print(f"\nTotal: {count} files updated")
