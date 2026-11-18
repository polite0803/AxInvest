globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "game_dev",
  "name": "游戏开发完整流程",
  "description": "游戏设计、关卡设计、数值平衡、游戏测试",
  "domain": "ai_media",
  "capabilities": [
    "tool:WebSearch",
    "tool:FileWrite",
    "tool:FileRead"
  ],
  "kpis": [
    {
      "key": "task_count",
      "name": "任务数量",
      "unit": "个"
    },
    {
      "key": "completion_rate",
      "name": "完成率",
      "unit": "%"
    }
  ],
  "workflowSteps": [
    {
      "id": "step1",
      "name": "分析阶段",
      "description": "需求分析"
    },
    {
      "id": "step2",
      "name": "设计阶段",
      "description": "方案设计"
    },
    {
      "id": "step3",
      "name": "执行阶段",
      "description": "执行实施"
    },
    {
      "id": "step4",
      "name": "评估阶段",
      "description": "结果评估"
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
      "id": "gd-gameplay",
      "name": "游戏玩法",
      "description": "玩法有趣且具有足够深度",
      "critical": true
    },
    {
      "id": "gd-balance",
      "name": "平衡性",
      "description": "游戏数值和系统保持平衡",
      "critical": true
    },
    {
      "id": "gd-performance",
      "name": "性能表现",
      "description": "游戏帧率稳定，无明显卡顿",
      "critical": true
    },
    {
      "id": "gd-player-satisfaction",
      "name": "玩家满意度",
      "description": "玩家测试反馈积极",
      "critical": false
    }
  ]
};
