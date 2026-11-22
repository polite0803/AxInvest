globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "finance_invest",
  "name": "投顾经营",
  "description": "财务分析 → 投资建议。CFO 视角，基于实际经营数据生成可执行的财务建议。",
  "domain": "finance",
  "capabilities": [
    "tool:OpcGetDashboard",
    "tool:OpcListKpis",
    "tool:OpcSearchWiki",
    "tool:OpcListProjects",
    "tool:OpcGetFinancialReport",
    "tool:OpcRecordKpi",
    "tool:OpcListCustomers",
    "tool:WebSearch",
    "tool:OpcFinanceInvestOpcGetDashboard",
    "tool:OpcFinanceInvestOpcListKpis",
    "tool:OpcFinanceInvestOpcSearchWiki",
    "tool:OpcFinanceInvestOpcListProjects",
    "tool:OpcFinanceInvestOpcGetFinancialReport",
    "tool:OpcFinanceInvestOpcRecordKpi",
    "tool:OpcFinanceInvestBatchValidateEntities",
    "tool:OpcFinanceInvestOpcListCustomers",
    "tool:OpcFinanceInvestWebSearch",
    "tool:OpcFinanceInvestRunAutomationRules",
  ],
  "kpis": [
    {
      "key": "total_assets",
      "name": "总资产",
      "unit": "CNY",
    },
    {
      "key": "revenue_growth",
      "name": "营收增长率",
      "unit": "%",
    },
    {
      "key": "risk_level",
      "name": "风险等级",
      "unit": "级",
    },
    {
      "key": "roi",
      "name": "投资回报率",
      "unit": "%",
    },
  ],
  "workflowSteps": [
    {
      "id": "a-report",
      "name": "财务数据获取",
      "description": "获取财务报表和运营数据",
    },
    {
      "id": "a-validate",
      "name": "数据验证",
      "description": "校验财务数据的完整性和合理性",
    },
    {
      "id": "a-risk",
      "name": "风险评估",
      "description": "基于财务数据计算风险指标",
    },
    {
      "id": "a-advice",
      "name": "投资建议",
      "description": "基于财务分析和风险评估生成投资建议",
    },
  ],
  "dataSources": [
    {
      "id": "stock_quote",
      "chain": [
        "astock",
        "db",
        "cache",
      ],
    },
    {
      "id": "stock_financials",
      "chain": [
        "astock",
        "db",
        "cache",
      ],
    },
    {
      "id": "stock_klines",
      "chain": [
        "astock",
        "db",
        "cache",
      ],
    },
    {
      "id": "stock_money_flow",
      "chain": [
        "astock",
        "cache",
      ],
    },
    {
      "id": "market_news",
      "chain": [
        "astock",
        "web",
        "cache",
      ],
    },
  ],
  "acceptanceCriteria": [
    {
      "id": "fi-analysis-accuracy",
      "name": "分析准确性",
      "description": "财务分析和投资建议准确无误",
      "critical": true,
    },
    {
      "id": "fi-risk-control",
      "name": "风险控制",
      "description": "已识别并评估主要风险",
      "critical": true,
    },
    {
      "id": "fi-actionable",
      "name": "可操作性",
      "description": "建议清晰可执行",
      "critical": false,
    },
  ],
};
