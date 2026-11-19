globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "design",
  "name": "设计完整流程",
  "description": "UI/UX 设计、视觉设计、品牌设计、设计系统",
  "domain": "ai_media",
  "capabilities": [
    "tool:WebSearch",
    "tool:FileWrite",
    "tool:OpcCreateContentAsset",
    "tool:OpcCreateLandingPage",
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
      "id": "design-quality",
      "name": "设计质量",
      "description": "设计作品符合视觉规范和质量要求",
      "critical": true,
    },
    {
      "id": "design-creativity",
      "name": "创意性",
      "description": "设计具有创新性和独特性",
      "critical": true,
    },
    {
      "id": "design-ux",
      "name": "用户体验",
      "description": "设计满足用户体验要求",
      "critical": true,
    },
    {
      "id": "design-brand",
      "name": "品牌一致性",
      "description": "设计符合品牌视觉规范",
      "critical": false,
    },
  ],
};
