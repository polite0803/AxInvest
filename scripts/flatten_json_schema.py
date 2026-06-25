#!/usr/bin/env python3
"""扁平化分析师prompt的JSON Schema：从15+字段减少到5字段扁平结构。

对所有 stock-analyst / debater / risk-evaluator 角色的prompt：
- 保留自然语言分析框架
- 将 JSON Schema 从复杂嵌套改为扁平5字段
- 更新少样本示例
- 移除过度复杂的自检项

用法: python scripts/flatten_json_schema.py
"""

import re, os

BASE = os.path.join("src-tauri", "agency_experts", "stock-analysis")

# 统一的扁平JSON Schema（各角色共用）
FLAT_SCHEMA = """\
## 输出格式

按以下结构输出JSON。`report`字段写你的完整分析（自然语言，可包含表格/清单），
其余字段是机读评分。

```json
{
  "report": "你的完整分析报告（自然语言，含关键数据引用和结论推理，可包含Markdown表格）",
  "verdict": "看多 | 偏多 | 中性 | 偏空 | 看空",
  "bull_score": 0,
  "bear_score": 0,
  "confidence": 0
}
```

- `bull_score` / `bear_score`: 0-100 整数，分开打分
- `confidence`: 0-100 整数，基于数据完整度和信号清晰度自评
- 所有的数据支持、推理过程、风险提示请写在 `report` 中，不要遗漏关键论据
"""

FLAT_EXAMPLE = """\
## 参考示例

```json
{
  "report": "## 趋势分析\\n近20日价格区间收敛至28.5-32.0。均线系统：5日/10日/20日三条均线纠缠，无明确方向。\\n\\n## 量价分析\\n近5日成交量较20日均量缩35%，缩量震荡表示多空双方均不积极。\\n\\n## 行业对比\\n个股相对行业排名中等偏上，无明显板块效应。\\n\\n## 结论\\n当前处于震荡格局，无明确突破信号，建议观望。",
  "verdict": "中性",
  "bull_score": 40,
  "bear_score": 50,
  "confidence": 70
}
```"""

def flatten_analyst(filepath):
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()
    
    orig = content
    
    # 1. 替换 JSON Schema 部分
    output_section_pattern = r'## 输出 JSON Schema（严格遵循，不要新增字段）\n\n```json\n.*?```'
    content = re.sub(output_section_pattern, FLAT_SCHEMA, content, flags=re.DOTALL)
    
    # 2. 替换 "## 少样本（good）" 部分（含其内部内容直到下一个 ## 或文件尾）
    content = re.sub(
        r'## 少样本（good）\n\n```json\n.*?```\n',
        FLAT_EXAMPLE + '\n',
        content,
        flags=re.DOTALL
    )
    
    # 3. 替换 "## 少样本（bad，反例）" 部分
    content = re.sub(
        r'## 少样本（bad，反例）\n\n```json\n.*?```\n',
        '',
        content,
        flags=re.DOTALL
    )
    
    # 4. 简化自检清单（保留前3条核心检查，去掉过于具体的）
    self_check_pattern = r'## 自检（输出前必过）\n\n- .+'
    simple_check = """\
## 自检

- [ ] `bull_score` 与 `bear_score` 是否分开打分（0-100整数）？
- [ ] `confidence` 是否如实反映数据完整度？
- [ ] `report` 中是否包含了关键数据引用和推理过程？\
"""
    content = re.sub(self_check_pattern, simple_check, content, flags=re.DOTALL)
    
    if content != orig:
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(content)
        return True
    return False

def flatten_debater(filepath):
    """Debater角色使用不同的扁平结构"""
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()
    
    orig = content
    
    debate_schema = """\
## 输出格式

按以下结构输出JSON。`report`字段写你的完整辩论观点（自然语言，引用具体数据），
其余字段是机读评分。

```json
{
  "report": "你的辩论观点全文，引用分析师的证据作为支撑",
  "stance": "bullish | bearish",
  "strength_score": 0,
  "confidence": 0
}
```

- `strength_score`: 0-100 整数，你对自己立场的信心强度
- `confidence`: 0-100 整数，基于数据完整度自评\
"""
    
    # 替换JSON Schema
    output_section = r'## 输出 JSON Schema（严格遵循，不要新增字段）\n\n```json\n.*?```'
    content = re.sub(output_section, debate_schema, content, flags=re.DOTALL)
    
    # 替换少样本
    content = re.sub(
        r'## 少样本（good）\n\n```json\n.*?```\n',
        '',
        content,
        flags=re.DOTALL
    )
    content = re.sub(r'## 少样本（bad，反例）\n\n```json\n.*?```\n', '', content, flags=re.DOTALL)
    
    # 替换自检
    content = re.sub(r'## 自检（输出前必过）\n\n- .+', '## 自检\n\n- [ ] 观点是否有足够的数据支撑？\n- [ ] stance 与 strength_score 是否一致？', content, flags=re.DOTALL)
    
    if content != orig:
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(content)
        return True
    return False

def flatten_risk(filepath):
    """Risk-evaluator使用带position的扁平结构"""
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()
    
    orig = content
    
    risk_schema = """\
## 输出格式

按以下结构输出JSON。`report`字段写你的完整风险评估（自然语言，含具体理由），
其余字段是机读评分。

```json
{
  "report": "你的风险评估观点全文",
  "stance": "aggressive | conservative | neutral",
  "position_pct": 0,
  "confidence": 0
}
```

- `position_pct`: 0-100 整数，你建议的仓位比例
- `confidence`: 0-100 整数，基于数据完整度自评\
"""
    
    content = re.sub(
        r'## 输出 JSON Schema（严格遵循，不要新增字段）\n\n```json\n.*?```',
        risk_schema,
        content,
        flags=re.DOTALL
    )
    
    content = re.sub(
        r'## 少样本（good）\n\n```json\n.*?```\n', '', content, flags=re.DOTALL
    )
    content = re.sub(
        r'## 少样本（bad，反例）\n\n```json\n.*?```\n', '', content, flags=re.DOTALL
    )
    content = re.sub(
        r'## 自检（输出前必过）\n\n- .+',
        '## 自检\n\n- [ ] position_pct 是否有充分的风险依据？\n- [ ] 是否考虑了最坏情景？',
        content,
        flags=re.DOTALL
    )
    
    if content != orig:
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(content)
        return True
    return False


if __name__ == '__main__':
    analysts = [
        "market-analyst.md", "sentiment-analyst.md", "news-analyst.md",
        "fundamentals-analyst.md", "policy-analyst.md", "hot-money-tracker.md",
        "lockup-watcher.md", "research-analyst.md", "sector-analyst.md",
        "catalyst-analyst.md", "data-quality-inspector.md",
        "social-media-analyst.md", "volume-price-analyst.md",
    ]
    
    debaters = [
        "bull-researcher.md", "bear-researcher.md",
        "bull-r2.md", "bear-r2.md",
        "bull-r3.md", "bear-r3.md",
        "debate-convergence.md",
    ]
    
    risks = [
        "aggressive-debator.md", "conservative-debator.md", "neutral-debator.md",
        "risk-convergence.md", "rule-checker.md",
    ]
    
    count = 0
    for f in analysts:
        path = os.path.join(BASE, f)
        if os.path.exists(path):
            if flatten_analyst(path):
                print(f"[analyst]  {f}")
                count += 1
    
    for f in debaters:
        path = os.path.join(BASE, f)
        if os.path.exists(path):
            if flatten_debater(path):
                print(f"[debater]  {f}")
                count += 1
    
    for f in risks:
        path = os.path.join(BASE, f)
        if os.path.exists(path):
            if flatten_risk(path):
                print(f"[risk]     {f}")
                count += 1
    
    print(f"\n已处理 {count} 个文件")
