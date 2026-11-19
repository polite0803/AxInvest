globalThis.AX_OPC_DOMAIN_DATA = {
  "id": "software_dev",
  "name": "软件开发完整流程",
  "description":
    "一人公司软件开发全流程（18 步）：需求→可行性→审批→架构→数据模型→API→环境搭建→编码→代码审查→审批→缺陷修复→文档→单元测试→集成测试→安全审查→审批→部署→运维交接。含 3 个人工审批闸门和完整数据流转。",
  "domain": "devops",
  "capabilities": [
    "tool:OpcListProjects",
    "tool:OpcCreateProject",
    "tool:FileWrite",
    "tool:WebSearch",
    "tool:OpcAddMilestone",
  ],
  "kpis": [
    {
      "key": "sprint_count",
      "name": "迭代数量",
      "unit": "次",
    },
    {
      "key": "code_coverage",
      "name": "代码覆盖率",
      "unit": "%",
    },
    {
      "key": "bug_fix_rate",
      "name": "缺陷修复率",
      "unit": "%",
    },
    {
      "key": "deploy_frequency",
      "name": "部署频率",
      "unit": "次/月",
    },
    {
      "key": "refactor_total_lines",
      "name": "代码总量",
      "unit": "行",
    },
    {
      "key": "refactor_total_files",
      "name": "文件总数",
      "unit": "个",
    },
    {
      "key": "refactor_avg_complexity",
      "name": "平均圈复杂度",
      "unit": "分",
    },
    {
      "key": "refactor_duplicate_ratio",
      "name": "重复代码率",
      "unit": "%",
    },
    {
      "key": "refactor_module_coupling",
      "name": "模块耦合度",
      "unit": "分",
    },
    {
      "key": "refactor_test_coverage",
      "name": "测试覆盖率",
      "unit": "%",
    },
    {
      "key": "refactor_batches_completed",
      "name": "完成批次数",
      "unit": "次",
    },
    {
      "key": "refactor_quality_score",
      "name": "质量评分",
      "unit": "分",
    },
    {
      "key": "refactor_golden_test_pass_rate",
      "name": "黄金测试通过率",
      "unit": "%",
    },
    {
      "key": "refactor_tacit_knowledge_retention",
      "name": "隐式知识保留率",
      "unit": "%",
    },
    {
      "key": "refactor_side_effect_equivalence",
      "name": "副作用等价率",
      "unit": "%",
    },
    {
      "key": "refactor_edge_case_retention",
      "name": "边界条件保留率",
      "unit": "%",
    },
  ],
  "workflowSteps": [
    {
      "id": "a-req",
      "name": "需求分析",
      "description": "分析用户需求",
    },
    {
      "id": "a-feasibility",
      "name": "可行性评审",
      "description": "评估技术可行性、资源和风险",
    },
    {
      "id": "a-feasibility-approval",
      "name": "可行性审批",
      "description": "批准项目进入架构设计阶段",
    },
    {
      "id": "a-arch",
      "name": "架构设计",
      "description": "设计系统架构和技术栈",
    },
    {
      "id": "a-data",
      "name": "数据模型设计",
      "description": "设计数据库实体、关系和索引",
    },
    {
      "id": "a-api",
      "name": "API设计",
      "description": "设计RESTful API接口",
    },
    {
      "id": "a-setup",
      "name": "项目环境搭建",
      "description": "初始化开发环境",
    },
    {
      "id": "a-code",
      "name": "编码实现",
      "description": "按设计实现代码",
    },
    {
      "id": "a-cr",
      "name": "代码审查",
      "description": "审查代码质量",
    },
    {
      "id": "a-cr-approval",
      "name": "代码审查审批",
      "description": "代码审查是否通过",
    },
    {
      "id": "a-fix",
      "name": "缺陷修复",
      "description": "根据代码审查意见修复代码缺陷",
    },
    {
      "id": "a-doc",
      "name": "文档编写",
      "description": "生成设计文档、API文档和开发指南",
    },
    {
      "id": "a-unit-test",
      "name": "单元测试",
      "description": "为核心模块编写单元测试",
    },
    {
      "id": "a-integration-test",
      "name": "集成测试",
      "description": "执行模块间集成测试和端到端测试",
    },
    {
      "id": "a-security",
      "name": "安全审查",
      "description": "执行安全审计",
    },
    {
      "id": "a-deploy-approval",
      "name": "部署审批",
      "description": "批准部署到目标环境",
    },
    {
      "id": "a-deploy",
      "name": "部署上线",
      "description": "执行部署、构建、数据库迁移",
    },
    {
      "id": "a-handoff",
      "name": "运维交接",
      "description": "生成运维文档和交接包",
    },
    {
      "id": "r-asset-scan",
      "name": "代码资产盘点",
      "description": "统计文件数/行数/模块结构",
    },
    {
      "id": "r-dep-graph",
      "name": "依赖关系分析",
      "description": "构建模块依赖图、识别循环依赖",
    },
    {
      "id": "r-complexity",
      "name": "复杂度扫描",
      "description": "圈复杂度/认知复杂度热点识别",
    },
    {
      "id": "r-smell-detect",
      "name": "坏味道检测",
      "description": "长方法/上帝类/重复代码检测",
    },
    {
      "id": "r-coupling-analyze",
      "name": "耦合度分析",
      "description": "高耦合模块识别与内聚性评估",
    },
    {
      "id": "r-risk-assess",
      "name": "风险评估",
      "description": "变更影响范围与回归风险评分",
    },
    {
      "id": "r-strategy",
      "name": "重构策略制定",
      "description": "渐进式/大爆炸/绞杀者模式选择",
    },
    {
      "id": "r-batch-plan",
      "name": "分批计划",
      "description": "按耦合度排序的模块分批",
    },
    {
      "id": "r-quality-baseline",
      "name": "质量基线建立",
      "description": "测试覆盖/性能基准/代码规范",
    },
    {
      "id": "r-rollback",
      "name": "回滚方案",
      "description": "分支策略/特性开关/数据回滚",
    },
    {
      "id": "r-pre-review",
      "name": "预审查",
      "description": "重构前代码审查",
    },
    {
      "id": "r-execute",
      "name": "分批执行",
      "description": "逐模块重构+持续集成",
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
      "id": "sd-code-quality",
      "name": "代码质量",
      "description": "代码符合编码规范，无明显 bug",
      "critical": true,
    },
    {
      "id": "sd-test-coverage",
      "name": "测试覆盖率",
      "description": "核心模块测试覆盖率不低于 70%",
      "critical": true,
    },
    {
      "id": "sd-security",
      "name": "安全性",
      "description": "无高危漏洞，符合 OWASP Top 10",
      "critical": true,
    },
    {
      "id": "sd-documentation",
      "name": "文档完整性",
      "description": "设计文档和 API 文档齐全",
      "critical": false,
    },
  ],
};
