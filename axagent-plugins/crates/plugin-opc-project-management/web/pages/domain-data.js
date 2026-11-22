globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "project_management",
  "name": "项目管理完整流程",
  "description": "项目规划、进度跟踪、风险管理、团队协作",
  "domain": "devops",
  "capabilities": [
    "tool:OpcCreateProject",
    "tool:OpcAddMilestone",
    "tool:OpcListProjects",
    "tool:OpcProjectManagementOpcCreateProject",
    "tool:OpcProjectManagementOpcAddMilestone",
    "tool:OpcProjectManagementOpcListProjects",
    "tool:OpcProjectManagementRunAutomationRules",
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
      "id": "pm-schedule",
      "name": "进度控制",
      "description": "项目按时完成，偏差不超过10%",
      "critical": true,
    },
    {
      "id": "pm-cost",
      "name": "成本控制",
      "description": "项目成本在预算范围内",
      "critical": true,
    },
    {
      "id": "pm-quality",
      "name": "质量保证",
      "description": "交付物质量符合标准",
      "critical": true,
    },
    {
      "id": "pm-stakeholder",
      "name": "干系人满意度",
      "description": "干系人对项目结果满意",
      "critical": false,
    },
  ],
};
