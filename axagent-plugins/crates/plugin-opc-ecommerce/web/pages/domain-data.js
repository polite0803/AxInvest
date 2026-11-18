globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "ecommerce",
  "name": "电商运营流程",
  "description": "选品分析 → 上架落地页 → 客户管理。CPO + COO 交叉协作。",
  "domain": "content_creation",
  "capabilities": [
    "tool:WebSearch",
    "tool:OpcSearchWiki",
    "tool:OpcListCustomers",
    "tool:OpcCreateContentAsset",
    "tool:OpcCreateLandingPage"
  ],
  "kpis": [
    {
      "key": "gmv",
      "name": "商品交易总额",
      "unit": "CNY"
    },
    {
      "key": "order_count",
      "name": "订单数量",
      "unit": "单"
    },
    {
      "key": "conversion_rate",
      "name": "转化率",
      "unit": "%"
    },
    {
      "key": "return_rate",
      "name": "退货率",
      "unit": "%"
    }
  ],
  "workflowSteps": [
    {
      "id": "a-product",
      "name": "选品分析",
      "description": "分析市场和客户需求，确定产品方向"
    },
    {
      "id": "a-page",
      "name": "上架落地页",
      "description": "创建产品落地页"
    },
    {
      "id": "a-customer",
      "name": "客户管理",
      "description": "创建客户记录，关联产品信息"
    }
  ],
  "dataSources": [
    {
      "id": "business_stats",
      "chain": [
        "db",
        "cache"
      ]
    }
  ],
  "acceptanceCriteria": [
    {
      "id": "ec-conversion",
      "name": "转化率",
      "description": "页面转化率达到目标",
      "critical": true
    },
    {
      "id": "ec-satisfaction",
      "name": "客户满意度",
      "description": "客户体验良好",
      "critical": false
    },
    {
      "id": "ec-profit",
      "name": "利润率",
      "description": "保持合理利润率",
      "critical": false
    }
  ]
};
