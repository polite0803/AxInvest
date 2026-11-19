globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "security",
  "name": "安全合规完整流程",
  "description": "安全审计、合规检查、风险评估、安全策略",
  "domain": "devops",
  "capabilities": [
    "tool:OpcSearchWiki",
    "tool:WebSearch",
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
      "id": "sec-risk",
      "name": "风险覆盖",
      "description": "所有识别的风险均已评估和处理",
      "critical": true,
    },
    {
      "id": "sec-remediation",
      "name": "漏洞修复率",
      "description": "高危漏洞修复率达到100%",
      "critical": true,
    },
    {
      "id": "sec-compliance",
      "name": "合规性",
      "description": "符合相关安全法规和标准",
      "critical": true,
    },
    {
      "id": "sec-penetration",
      "name": "渗透测试",
      "description": "渗透测试无高危漏洞",
      "critical": true,
    },
  ],
};
