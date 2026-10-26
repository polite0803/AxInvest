# Changelog

All notable changes to AxAgent will be documented in this file.
## [v2.9.9] - 2026-09-23

### ✨ New Features
- 行业编排前端页面实现 + seed 数据补充
- 工作流模板转换、一致性检查与前端编辑器增强
- 🚀 新增 WorkflowExecutor 工作流执行面板 + 模板运行按钮
- 新增工作流工具系统与引擎命令
- 模板列表按领域分组 + 工作流导入弹窗 + invoke 参数 snake_case 修正
- 工作流模板按业务域分组展示，系统模板页过滤仅显示 isSystem
- 领域/行业工作流种子 v4 升级 + i18n JSON 深度合并 driver + 托盘幂等防护
- 能力图谱 + 叙事结构 + 认知编排器重构
- 能力路由系统 + 文档导出 + 认知编排增强
- 能力渐进式披露三层落地（索引层目录 + CapabilityView 定义层）
- 能力体系深化 — 动态工具/会话状态/上下文引擎整合 + 工具链重构
- OPC需求发现模块 + scanner重构 + 上游合并适配
- Wiki 知识库重构 + RAG/内存优化 + v131 migration + Vitest Windows 盘符修复
- 上游 Wiki 重构合并 + OPC 全链路拓展 + 工作流调度
- v132 memory 访问索引 + RAG 管线优化 + 统一搜索源权重 + GraphView 重构
- 恢复 OPC 业务主干（21 文件）+ migration 顺延 v133-v136
- 恢复 stash 增量（36 文件）— scanner 增强 + i18n + OPC 命令改进
- 恢复 stash 增量（36 文件）— scanner 增强 + i18n + OPC 命令改进
- 恢复完整本地业务代码 — 14 OPC crate + 7 股票业务 crate + entities 模块声明 + 编译修复
- 🚀 恢复本地 OPC + 股票业务后端模块（从 992947df^ 恢复 6 个股票 crate + 55 个实体文件 + 适配 HEAD 实体定义，cargo check 全部通过）
- 🚀 P0-1 Windows 真沙箱 + P0-2 ApprovalPolicy 策略化审批（codex 对标）
- 🚀 P0-3 session_events 持久化基础设施
- 🚀 P0-3e agent_resume_from_events 跨进程恢复诊断命令
- 🚀 P1-1 TokenBudget Compact 自动压缩
- 添加 ApplyPatchTool — 原子性多文件 git-style diff 应用
- 添加 McpAgentServer — 把 agent 能力暴露为 MCP stdio server
- McpAgentServer 接入 SessionManager — agent_status / agent_cancel 真实化
- wiring 层注入 McpAgentServer + agent_result.session_id
- 新增 agent_session_status / agent_session_cancel / agent_session_list 命令
- HarnessAgentAdapter 注入 SessionManager，execute 返回真实 session_id
- ReActEngine 运行时取消 + HarnessAgentAdapter 集成
- AgentSessionRepository 补 get_by_id / get_by_conversation_id 查询
- per-session cancel token + memory/DB two-level status lookup
- 深层取消 - LLM API 调用过程中也能立即响应 cancel
- AgentSessionRepository::list_all + SessionManager.list_session_ids DB 回退
- agent session DTOs + invoke wrappers
- 新增 run_workflow 工具 + workflow_types 扩展 + capability_load
- astock 数据工具接入统一注册表 + 金融域路由优先级提升
- 新增 lifecycle hooks + DAO migration v134 + DefaultModelSettings 重构
- stock-analysis 工作流生命周期钩子 + astock-eastmoney 供应商增强
- OPC 需求发现独立页面化 + rt-workflow 引擎增强 + astock-eastmoney 供应商升级
- 域包卡片「启用/停用 + 本体域 + 能力承诺」增强
- 客户 B端/C端区分 + 地理信息标注 + 列表/地图双视图
- 知识源目录导入向导 + .rs→.md 引用门禁豁免 + rustls 升级
- 股票分析业务迭代 — 维度目录/反思统计/依赖指标新增，移除 executionBridge，修正 LLM 节点错误码误导


### 🎨 Styling
- 🎨 style(office): AgentCard 标题区域 minWidth 0→48 避免窄屏溢出
- rustfmt 合并 14 个 seed_industry_* 的 workflow_types import 为单行
- 🎨 style: 清理解构中冗余的未使用绑定标识符
- 🎨 style: DefaultModelSettings 日志文案英文化


### 🐛 Bug Fixes
- 修复启动时嵌套 runtime 错误和主线程栈溢出
- 修复 start_cron_scheduler 缺少 app 参数导致的编译错误
- 使用 tauri-build API 替代手动 manifest 嵌入，解决链接器重复资源错误
- 简化 build.rs，使用 Tauri 默认 manifest 解决 14001 启动错误
- 恢复 /MANIFEST:EMBED 链接器选项以修复 Windows 错误 14001
- 🐛 fix: 修复 Windows manifest 冲突和 cron 表达式兼容性问题
- 修复 stock-analysis 模板 trigger_config 缺失 cron 值的问题
- 🐛 修复 clippy field_reassign_with_default 警告
- 🐛 修复 rt-workflow 测试模块缺少 NodeStatus 导入
- 对齐上游 DTO camelCase 重构 - 修复设置面板组件字段名
- 修复 workflow 测试 fixtures 中 snake_case 字段与 serde camelCase 不兼容的问题
- 🐛 修复 Windows manifest 重复嵌入导致链接错误
- update test JSON to use camelCase field names for serde rename_all compatibility
- update serialization_contracts test JSON keys to camelCase for serde rename_all compatibility
- 🐛 修复 oxlint const-comparisons 警告：WorkflowEditor 中逻辑运算符两侧相同
- 移除 workflowEditorStore 中硬编码中文错误消息
- ConsistencyMode Default 改用 derive + #[default]
- 修复上游合并导致的编译错误
- switch 节点端口校验兼容后端 snake_case branch_ 前缀
- 补齐 6 个后端错误码在 11 语言的 error 段翻译
- 补 WorkflowTemplateData 测试构造的 cluster_id/route_path 字段
- 打通三层路由写入路径 — model_to_active_model 不再清零 cluster_id/route_path
- 🐛 fix(workflow): snake_case edge 兼容 + violation chip 直接输出数字
- 补齐 office.roleTag 角色标签 11 语言翻译（77 条）
- 修复上游合并引入的 11 语言 JSON 语法错误
- embedding 永久 HTTP 错误快速失败 + workflowStore 测试修正 invoke 参数名
- wiki_graph_cache 反序列化旧缓存失败兜底 + TemplateList 权威领域关键词映射
- 补齐 9 语言相对 zh-CN 缺失的 395 个翻译 key (contracts errors=0)
- approval_edges_build_correctly 适配 v4 丰富拓扑 (审批节点 ap-accounting)
- clippy eq(&row.id) 引用修复 + 另一测试 approval_accounting → ap-accounting
- 3 处本地优化 — workflow_conversions warn→debug + OPC_INDUSTRY_DOMAIN 映射 + is_permanent_http_error range contains
- 容器节点 config 旧数据兼容兜底 — 避免属性面板 TypeError 炸页
- v113 统一知识图谱新字段全链路补全 + 连接池扩容 + SystemTemplateList 瘦身
- v113 knowledge_graph 新字段回填 — import_lemonhu_graph 补 source_type/source_id/node_type/external_id
- 修复系统模板Tab因onOpenSystemEditor prop守卫导致渲染业务模板列表的bug
- 系统模板过滤从 tags 改为 is_preset && is_public 领域隔离
- 系统模板过滤改为按 route_path L1 system 领域判断
- get_workflow_template 按 id 精准读取，移除领域过滤（list 命令才有过滤语义）
- HallucinationGuardConfig.match_threshold 和 ConsistencyCheckConfig.deviation_threshold 加 serde(default) 兼容旧数据
- TemplateList 领域筛选器和分组标题改用 capabilityDomain i18n 翻译
- 🐛 fix: 系统模板列表空 — dao upsert 补 route_path/cluster_id 更新 + 版本号递增触发重灌
- iter_overeager_cloned — filter 再 clone 避免无意义拷贝 + cargo fmt
- 消除 clippy 1.97 在 cognitive.rs 上的 2 处新增 lint
- 修复会话状态清理缺口 + skill 查询语义测试修正
- Skill 持久化补齐 prompt_body 字段 + 版本号 2.9.9
- save_dynamic_workflow 裸 map_err → ErrorResponse 错误码
- 工具策略位置纠正 + P5 降级保护收紧 + 单元测试
- llm_wiki log_lint_history 裸 map_err 改为 ErrorResponse
- v133 测试依赖版本号修正 + browserMock 清理废弃 case
- 对齐 tauri-plugin-updater 版本（NPM 2.10 → 2.11，匹配 Rust 侧）
- markstream-react v2 API 适配 + 依赖刷新
- n8n 导入工具占位降级 + 相似度阈值调优
- 补齐 MCP_AGENT_SESSION_NOT_FOUND / CANCEL_FAILED 错误码的 11 语言翻译
- 修复 job_object.rs cfg(windows) 缺失 + linux_sandbox.rs Drop 冲突
- 用 ptr::read 绕过 E0509，Linux CI 友好
- into_inner 改用 associated function 语法，兼容 nightly Rust 1.97
- 移除 linux_sandbox 多余 mut + job_object stub 补 # Safety 文档
- bash_sandboxed_path_end_to_end 加 Linux 分支，不再硬断言 expect_err
- 修复契约违规与 conversations 测试编译
- conversations 测试专用 import 加 #[cfg(test)] 保护，消除生产编译 unused import
- server_main 访问私有 commands 模块——lib.rs pub use re-export ensure_opc_config_synced
- 适配上游 v134 hooks_config 字段——补齐本地 workflow_template 构造点
- capability 嵌入维度缓存修复启动白屏 + pace-calc/凯利公式编译问题
- market_list 的 DB 访问下沉到 analysis-engine，保持 command 分层纪律
- 补全知识源同步类型与 i18n 缺失 key
- 统一 selectRequired 措辞为「选择已有的知识源」同步 11 种语言 selectRequired 译文为「选择已有知识源」语义（update 模式必填提示）。
- 修复 DATA_DRIVEN_COLUMN.evidence 引用腐烂（行号 1751→2872）
- PG 集成测试 notes 计数移到 bootstrap 之后，适配 CI 全新空库
- 旧路由重定向改用 /chat?ws=<tab> URL 形式，修复 /knowledge Chromium E2E 失效
- 目录导入向导挂载即崩——summarizeOutcome 读 null importResult
- pg_cjk_fts 改走生产初始化链 create_pool，修复全新空库 relation notes does not exist
- pg_cjk_fts 前置 CREATE EXTENSION vector 并断言 bootstrap 无 abort，修复空库建表被截断
- pg_cjk_fts 建表前先建 ax_cjk_ngram 函数，解除全新库生成列内联引用函数的段序依赖
- 修复 7 处 check-single-source-facts 行号引用腐烂（空行/不存在文件）
- EvolutionDriftPanel parseTrigger i18n 化 — 5 处硬编码中文改为 t()
- demand_discovery E0425 + services.rs unused_variables (Android mobile cfg 门控)
- list_portfolio 类型标注 + trigger_stock_evolution 错误码归一


### 📝 Documentation
- 修正 domain_ontology 中 .rhai 脚本行号引用


### 📦 Miscellaneous
- 合并上游更新 (fix: 修复启动时嵌套 runtime 错误和主线程栈溢出)
- 更新 settingsStore 错误日志为英文
- 合并上游更新 (upstream/master)
- Merge remote-tracking branch 'upstream/master'
- 📦 升级版本号至 2.9.8
- Merge upstream/master
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- trigger CI run
- rerun CI after i18n fix
- 合并上游 AxAgent 更新 (WorkflowExecutor 工作流执行面板)
- Merge remote-tracking branch 'upstream/master'
- 清理 6 语言残留 foo 测试 key
- 合并上游 AxAgent 更新（cluster_id/route_path 根因修复）
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- merge upstream master (json_merge_driver 零冲突)
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Loop AI systemPrompt 从硬编码中文迁移到 t() + clippy redundant_closure 修复
- Merge remote-tracking branch 'upstream/master'
- list_system_templates 加 tracing 日志排查 cognitive_router 匹配
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Revert "fix: 系统模板过滤从 tags 改为 is_preset && is_public 领域隔离"
- Revert "fix: 修复系统模板Tab因onOpenSystemEditor prop守卫导致渲染业务模板列表的bug"
- workflow_conversions nodes/edges 反序列化失败时输出 tracing error 日志
- get_workflow_template 返回时打印 nodes/edges 数量 + 前端 store console.log 追踪
- WorkflowEditor useEffect 打印 isSystemTemplate 实参值
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- 清理 AxInvest 股票业务模块（1092 文件）
- 合并上游 7 个 commit + 本地清理
- 行尾统一LF + scanner修正 + CI检查补强
- WikiEditorPage 日志文案统一英文
- 合并上游 Wiki 重构 + v131→v132 migration 顺延 + LF 修复
- 合并上游 v132 memory 访问索引 + RAG 管线优化 + GraphView 重构
- 清理 Rhai 变量双下划线前缀 + vitest 运行脚本封装
- 合并 upstream/master 3 个提交 + markstream v2 类型修复
- 依赖包升级 — Rust patch + npm patch + dprint 全家桶 + monaco-editor
- 📦 清理废弃的规划与评审文档
- 合并 upstream/master 27 个提交（MCP Agent Server + Session 持久化 + Windows 沙箱 + agent crate 重构清理）
- 移除误跟踪的 Windows 系统缓存文件（%SystemDrive%/.../Caches）
- Merge remote-tracking branch 'upstream/master'
- 合并后 locale 行尾统一为 LF（json_merge_driver 绕过 eol 规范化）
- Merge remote-tracking branch 'upstream/master'
- Merge remote-tracking branch 'upstream/master'
- 补齐 AIPanel/WorkflowExecutor 内部日志与 prompt 上下文的豁免基线（行号漂移）
- 修 contracts 棘轮超基线 + 升级 calamine 修复 quick-xml 高危漏洞
- ipc-schemas 诊断信息加入硬编码白名单（非 UI 文本，走 allowlist 豁免）
- update layer-discipline baseline + dprint format + CRLF fix


### 🔧 CI / Build
- 移除 android-actions/setup-android，改用托管 runner 预装 SDK（v3 新输入白名单致 sdkmanager 装 legacy 'tools' 包挂死）


### 🔨 Refactoring
- 使用 tauri-build 官方 API 嵌入 Windows manifest
- 🔨 修复 rt-workflow clippy 警告
- 🔨 修复 approx_constant clippy 警告
- 全量补齐 DTO serde rename_all 注解并对齐前端 camelCase
- 🔨 全量前端 DTO 字段 snake_case → camelCase 命名规范统一
- 模板领域分组瘦身 — 砍掉关键词映射，回归 L1 权威域
- 删除废弃 delegation/fork_bridge 模块 + 工具注册路径整合
- 收敛 workflow 接缝到 capability_registry + AppState 瘦身 + SEAM_NOT_READY 错误码
- 🔨 agent crate 清理 + P0-3 跨进程上下文重建基础设施 + runtime 格式统一
- 删除 coordinator/adapter/config 等冗余模块，简化 agent crate 架构
- clean_output 上移至 harness，解除 agent→runtime-core 越界依赖
- 恢复重构代码——stock-analysis 上移 harness + narrative_structure/audit_log_service 清理
- 行业导航页切换至 IndustryPages 组件体系 + 后端 fork 分歧防护


### 🧪 Testing
- workflowStore 断言 isPreset → is_preset 对齐 invoke 参数

