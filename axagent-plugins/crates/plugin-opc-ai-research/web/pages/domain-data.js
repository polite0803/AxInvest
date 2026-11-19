globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "ai_research",
  "name": "AI 科技研究报告",
  "description": "AI 技术与应用研究报告：需求分析 → 技术调研 → 原型验证 → 报告输出。CTO 视角，含防幻觉。",
  "domain": "data_analysis",
  "capabilities": [
    "tool:OpcListProjects",
    "tool:OpcSearchWiki",
    "tool:WebSearch",
    "tool:FileRead",
    "tool:Bash",
    "tool:FileWrite",
  ],
  "kpis": [
    {
      "key": "research_count",
      "name": "研究数量",
      "unit": "份",
    },
    {
      "key": "prototype_success_rate",
      "name": "原型成功率",
      "unit": "%",
    },
    {
      "key": "tech_readiness_score",
      "name": "技术成熟度",
      "unit": "分",
    },
    {
      "key": "research_impact_score",
      "name": "研究影响力",
      "unit": "分",
    },
  ],
  "workflowSteps": [
    {
      "id": "a-req",
      "name": "需求分析",
      "description": "分析AI研究需求，明确研究范围和评估标准",
    },
    {
      "id": "a-survey",
      "name": "技术调研",
      "description": "调研AI技术方案和最新进展",
    },
    {
      "id": "a-prototype",
      "name": "原型验证",
      "description": "搭建最小原型，验证关键假设",
    },
    {
      "id": "a-report",
      "name": "研究报告",
      "description": "生成AI研究报告",
    },
  ],
  "dataSources": [
    {
      "id": "business_stats",
      "chain": [
        "db",
        "cache",
      ],
    },
  ],
  "acceptanceCriteria": [
    {
      "id": "ai-research-accuracy",
      "name": "研究准确性",
      "description": "论文和技术解读是否准确",
      "critical": true,
    },
    {
      "id": "ai-research-coverage",
      "name": "研究覆盖度",
      "description": "是否覆盖指定的研究范围",
      "critical": false,
    },
    {
      "id": "ai-research-actionable",
      "name": "可操作性",
      "description": "结论是否可以指导实际应用",
      "critical": false,
    },
  ],
};
