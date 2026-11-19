globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "geospatial",
  "name": "地理信息完整流程",
  "description": "地理数据分析、空间查询、地图可视化、位置服务",
  "domain": "data_analysis",
  "capabilities": [
    "tool:OpcSearchWiki",
    "tool:WebSearch",
    "tool:OpcCreateContentAsset",
    "tool:OpcCreateProject",
    "tool:FileWrite",
  ],
  "kpis": [
    {
      "key": "task_count",
      "name": "任务数量",
      "unit": "个",
    },
    {
      "key": "completion_rate",
      "name": "完成率",
      "unit": "%",
    },
  ],
  "workflowSteps": [
    {
      "id": "step1",
      "name": "分析阶段",
      "description": "需求分析",
    },
    {
      "id": "step2",
      "name": "设计阶段",
      "description": "方案设计",
    },
    {
      "id": "step3",
      "name": "执行阶段",
      "description": "执行实施",
    },
    {
      "id": "step4",
      "name": "评估阶段",
      "description": "结果评估",
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
      "id": "geo-accuracy",
      "name": "数据准确性",
      "description": "地理数据精度达到规定标准",
      "critical": true,
    },
    {
      "id": "geo-analysis",
      "name": "空间分析",
      "description": "空间分析方法合理，结果可靠",
      "critical": true,
    },
    {
      "id": "geo-visualization",
      "name": "可视化质量",
      "description": "地图和可视化输出清晰准确",
      "critical": false,
    },
    {
      "id": "geo-completeness",
      "name": "数据完整性",
      "description": "地理数据覆盖完整，无缺失",
      "critical": false,
    },
  ],
};
