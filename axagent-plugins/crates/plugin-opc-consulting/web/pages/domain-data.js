globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "consulting",
  "name": "行业咨询流程",
  "description": "行业调研 → 方案设计 → 交付报告。CEO 视角，战略级分析。",
  "domain": "data_analysis",
  "capabilities": [
    "tool:OpcSearchWiki",
    "tool:WebSearch",
    "tool:OpcGetDashboard",
    "tool:OpcCreateContentAsset",
    "tool:FileWrite",
    "tool:OpcConsultingOpcSearchWiki",
    "tool:OpcConsultingWebSearch",
    "tool:OpcConsultingOpcGetDashboard",
    "tool:OpcConsultingOpcCreateContentAsset",
    "tool:OpcConsultingFileWrite",
    "tool:OpcConsultingRunAutomationRules",
  ],
  "kpis": [
    {
      "key": "consulting_count",
      "name": "咨询项目数",
      "unit": "个",
    },
    {
      "key": "client_satisfaction",
      "name": "客户满意度",
      "unit": "%",
    },
    {
      "key": "delivery_quality_score",
      "name": "交付质量分",
      "unit": "分",
    },
    {
      "key": "market_insight_depth",
      "name": "市场洞察深度",
      "unit": "分",
    },
  ],
  "workflowSteps": [
    {
      "id": "a-intake",
      "name": "调研启动",
      "description": "明确咨询目标和调研范围",
    },
    {
      "id": "a-research",
      "name": "信息采集",
      "description": "系统性收集行业数据和情报",
    },
    {
      "id": "a-validate",
      "name": "调研验证",
      "description": "验证采集信息的质量和充分性",
    },
    {
      "id": "a-design",
      "name": "方案设计",
      "description": "设计针对性的咨询方案",
    },
    {
      "id": "a-approval",
      "name": "方案审批",
      "description": "客户审批咨询方案",
    },
    {
      "id": "a-report",
      "name": "报告撰写",
      "description": "生成完整的行业咨询报告",
    },
    {
      "id": "a-deliver",
      "name": "交付归档",
      "description": "整理最终交付物包",
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
      "id": "ic-insight-depth",
      "name": "洞察深度",
      "description": "分析深入业务本质",
      "critical": true,
    },
    {
      "id": "ic-recommendation",
      "name": "建议质量",
      "description": "建议具体可执行",
      "critical": true,
    },
    {
      "id": "ic-data-support",
      "name": "数据支撑",
      "description": "结论有充分数据支撑",
      "critical": false,
    },
  ],
};
