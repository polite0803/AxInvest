globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "education",
  "name": "教育培训流程",
  "description": "课程设计 → 内容制作 → 学员管理。COO + CMO 协作。",
  "domain": "content_creation",
  "capabilities": [
    "tool:WebSearch",
    "tool:OpcCreateContentAsset",
    "tool:OpcCreateLandingPage",
    "tool:FileWrite",
  ],
  "kpis": [
    {
      "key": "student_count",
      "name": "学员数量",
      "unit": "人",
    },
    {
      "key": "course_completion_rate",
      "name": "课程完成率",
      "unit": "%",
    },
    {
      "key": "avg_score",
      "name": "平均分数",
      "unit": "分",
    },
    {
      "key": "student_satisfaction",
      "name": "学员满意度",
      "unit": "%",
    },
  ],
  "workflowSteps": [
    {
      "id": "a-curriculum",
      "name": "课程设计",
      "description": "设计课程目标、大纲和评估方式",
    },
    {
      "id": "a-content",
      "name": "内容制作",
      "description": "制作课程内容讲义、练习和测验",
    },
    {
      "id": "a-enroll",
      "name": "学员管理",
      "description": "创建学员信息，设置学习路径",
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
      "id": "ed-outcomes",
      "name": "学习成果",
      "description": "学员掌握核心知识点",
      "critical": true,
    },
    {
      "id": "ed-engagement",
      "name": "参与度",
      "description": "学员积极参与互动",
      "critical": false,
    },
    {
      "id": "ed-application",
      "name": "实践应用",
      "description": "知识可应用于实际场景",
      "critical": false,
    },
  ],
};
