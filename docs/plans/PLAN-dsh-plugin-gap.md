# PLAN · DSH 插件能力缺口表（Gap Analysis）

> 状态：done ｜ 日期：2026-10-27 ｜ 对应版本：v2.9.9
> 目的：在动手迁移前，把 AxInvest 的每个功能域映射到 DSH 现有插件，区分「直接替代 / 需适配 / 必须自建」。
> 口径：只做能力盘点与成本估算，不含实施步骤；人日为估算值，误差约 ±30%。

## 1. 盘点基线

| 维度 | 数量 | 来源 |
|---|---|---|
| 前端页面 | 41 个（其中多个重定向到 Chat，独立页面约 22 个） | `src/pages/` |
| 后端命令模块 | **184 个**（目录 + 文件） | `src-tauri/src/commands/` |
| Rust workspace crate | 47 个 | `src-tauri/Cargo.toml` |
| DSH 可安装插件包 | 约 250 个，40 个分组 | `cordis-composition-reference/references/packages.md` |
| 股票业务规模 | 后端约 18.8 万行；前端 238 文件 / 6.0 万行 | 实测统计 |
| 股票业务扩张速度 | 近 6 个月净增 18.6 万行（每月约 3.1 万行） | `git log --numstat` |

## 2. A 类：DSH 直接替代（自建归零）

这些是 AxInvest 里“通用基座”的部分。迁移时应**删除自研、改用 DSH 实现**，只保留配置与少量接线。

| # | 功能域 | AxInvest 现有模块 | DSH 对应插件 | 迁移动作 |
|---|---|---|---|---|
| A1 | 会话与消息 | `conversations`、`messages`、`message_continuation`、`conversation_categories`、`conversations_search`、`branches` | `dsh-session`、`dsh-agent`、`dsh-agent-loop`、`dsh-client-ui-chat`、`dsh-client-ui-conversation`、`dsh-client-ui-sidebar` | 数据迁移（JSONL/SQLite 转换） |
| A2 | 工具注册与执行 | `capability`、`local_tool`、`generated_tool`、`tool_recommender` | `dsh-tools`、`dsh-tool-*` | 删除自研注册表 |
| A3 | MCP | `mcp` | `dsh-mcp-client`、`dsh-mcp-resources` | 配置迁移 |
| A4 | 技能 | `skills`、`skills_hub`、`skill_decomposition`、`skill_workflow` | `dsh-skill`、`dsh-skill-filesystem`、`dsh-tool-skill` | 格式转换 |
| A5 | 插件市场 | `plugin`、`plugin_profile`、`marketplace` | `dsh-plugin-manager`、`dsh-plugin-market`、`dsh-client-ui-plugin-market` | 直接替换 |
| A6 | 终端 | `terminal`、`pty` | `dsh-terminal`、`dsh-tool-terminal`、`dsh-client-ui-sidebar-terminal` | 直接替换 |
| A7 | 文件 | `files`、`files_page`、`file_browser`、`file_cleanup` | `dsh-fs-local`、`dsh-tool-fs`、`dsh-tool-fs-search`、`dsh-client-ui-sidebar-files` | 直接替换 |
| A8 | 设置/主题/快捷键 | `settings`、`app_config`、`theme`、`onboarding` | `dsh-settings`、`dsh-client-ui-settings*`、`dsh-client-ui-theme`、`dsh-client-shortcuts` | 配置重写 |
| A9 | 审批/沙箱/凭据 | `approval_rules`、`sandbox`、`profile`（密钥部分） | `dsh-user-approval`、`dsh-sandbox-*`、`dsh-credentials-local` | 策略映射 |
| A10 | 子代理 | `sub_agent`、`multi_agent`、`fleet`、`orchestrator`、`parallel_execution`、`spawn_guard` | `dsh-subagent*`、`dsh-tool-subagent*` | 语义映射 |
| A11 | 调度/提醒 | `reminder`、`scheduled_task`、`nl_to_cron` | `dsh-schedule`、`dsh-tool-schedule` | 部分替换（见 B5） |
| A12 | Webhook | `webhook` | `dsh-webhook`、`dsh-webhook-github` | 直接替换 |
| A13 | 浏览器/computer-use | `browser`、`computer_control`、`screen_vision` | `dsh-browser-use*`、`dsh-computer-use*`（experimental） | 直接替换 |
| A14 | 语音/翻译 | （无独立模块，前端内嵌） | `dsh-experimental-speech-to-text*`、`dsh-experimental-translator` | 新增 |
| A15 | Office/文档 | `document_export`、`chart_generator`（部分） | `dsh-skill-office`、`dsh-office-to-pdf` | 直接替换 |
| A16 | 轨迹/DevTools | `tracer`、`evaluator`、`workflow_execution_stats` | `dsh-client-ui-trajectory`、`dsh-experimental-inspector` | 直接替换 |
| A17 | 目标/计划/待办 | `plan`、`proactive` | `dsh-goal`、`dsh-plan-mode`、`dsh-tool-todo` | 直接替换 |
| A18 | 附件/工作区 | `artifacts`、`cloud_workspace`、`device_sync` | `dsh-attachment-local`、`dsh-workspace`、`dsh-working-directory` | 直接替换 |

**A 类结论**：一次性迁移成本主要在数据转换与配置重写，约 **8–15 人日**；自研基座不再维护，长期每年省 **25–40 人日**。

## 3. B 类：需适配（DSH 有接缝，业务逻辑自建）

| # | 功能域 | AxInvest 现有模块 | DSH 对应 | 差距 | 人日 |
|---|---|---|---|---|---|
| B1 | 工作流 DAG | `workflows`、`workflow_ai*`、`workflow_template`、`workflow_tool`、`workflow_yaml`、`wf_des`、`work_engine` | `dsh-workflow-ptc`、`dsh-tool-workflow`、`dsh-client-ui-workflow-run` | DSH 是 JS 脚本编排；AxInvest 是 28 节点可视化 DAG + Rhai + 黑板快照 | 15–25 |
| B2 | 多智能体会议 | `agent*`（7 个）、`multi_agent_setup`、`agency_expert` | `dsh-subagent*`、`dsh-experimental-agent-team*` | 会议/辩论/投票语义需重写 | 8–12 |
| B3 | 多 LLM Provider | `providers`、`provider_ctx`、`provider_balance`、`local_model(s)`、`smart_router` | `dsh-llm` seam + DeepSeek/pi-ai 两个实现 | 只覆盖 DeepSeek；OpenAI/Anthropic/本地模型需自建 provider 插件 | 8–12 |
| B4 | OpenAI 兼容网关 | `gateway`、`gateway_link`、`platform_integration` | `dsh-api-gateway`、`dsh-api-remote-access` | 协议不同，需兼容层 | 6–10 |
| B5 | 任务中心/业务调度 | `background_tasks`、`recommendation_cron`、`pool_scan`、`market_close_sweep` | `dsh-jobs-local`、`dsh-tool-jobs`、`dsh-schedule` | 有后台 job，无业务任务编排/重试/看板 | 6–10 |
| B6 | 错误码/健康/诊断 | `error`、`error_code`、`error_classification`、`health`、`crash_report` | 无统一错误码体系 | 需自建错误码与诊断 | 4–6 |
| B7 | 备份/迁移/时间旅行 | `backup`、`migration`、`time_travel_batch`、`sessions_export` | `dsh-session-persistence-jsonl` | 业务回放需自建 | 5–8 |

**B 类结论**：一次性 **52–83 人日**；这部分是替代优先“省不掉”的，因为 DSH 只提供了接缝，业务语义仍要自己实现。

## 4. C 类：必须自建（市场无对应）

| # | 功能域 | AxInvest 现有模块 | 规模 | 人日 |
|---|---|---|---|---|
| C1 | 股票数据接入 | `stock_analysis_bridge`、`market_mainline`、`universe_scan`、`pool_scan`、`market_sim` 数据侧 | 4.8 万行，60 个 MCP 工具 | 5–8 |
| C2 | 股票分析编排 | `stock_analysis`、`stock_workflow`、`stock_analysis_setup`、`decision_gates`、`stock_analysis_rerun` | 约 6 万行命令 + 6.8 万行引擎 | 25–40 |
| C3 | 组合/交易/风控 | `paper_portfolio`、`paper`、`trade_intent`、`conditional_order`、`price_alert_upsert` | 约 1.5 万行 | 10–15 |
| C4 | 回测/量化/模拟 | `quant_backtest`、`backtest_validation`、`market_sim` | 约 1.2 万行 | 8–12 |
| C5 | 监控/预警/复盘 | `mover_recall*`、`market_close_sweep`、`recommendation_cron` 业务侧 | 约 1 万行 | 6–10 |
| C6 | 知识库/RAG/Wiki/记忆 | `knowledge*`、`unified_knowledge`、`llm_wiki`、`wiki`、`memory`、`knowledge_graph`、`index_jobs`、`retrieval_feedback` | 约 2 万行 | 20–30 |
| C7 | OPC（组织/人才/域包） | `opc*`（14 个模块）、`opc_capability_pack_*` | 14 个命令模块 | 8–12 |
| C8 | 需求发现/学习图谱/认知 | `demand_discovery`、`demand_llm_refine`、`learning_graph`、`cognitive`、`dream`、`narrative` | 约 1 万行 | 10–18 |
| C9 | FineTune/RL | `fine_tune`、`rl_training` | 约 0.6 万行 | 5–8 |
| C10 | 动态 UI/图表/图像 | `dynamic_ui`、`chart_generator`、`image_gen`、`image_gen_settings`、`screenshot_diagnosis` | 前端 1.5 万行 | 8–12 |
| C11 | 股票/知识库/OPC 前端 | `src/components/stock-analysis`（238 文件）、`stock-workspace`、`quant`、`opc` | 6.0 万行 | 30–45 |

**C 类结论**：一次性 **135–210 人日**；这是成本主体，且**不因 DSH 市场存在而减少**。

## 5. 成本汇总

| 口径 | 替代优先 | 全自建（不采用 DSH 现有插件） |
|---|---|---|
| A 类（基座） | 8–15 | 40–60 |
| B 类（适配） | 52–83 | 52–83（同样要做） |
| C 类（业务自建） | 135–210 | 135–210 |
| 集成/数据迁移/双轨 | 20–30 | 20–30 |
| **一次性合计** | **215–338 人日** | **247–383 人日** |
| **长期维护（每年）** | 约 60–90 | 约 85–130 |

> 说明：上一轮给出的 130–210 人日偏乐观，原因是按“插件数”估算而非按模块清点。本次按 184 个命令模块与 41 个页面逐域核对后上修。±30% 误差仍然适用。

**替代优先的真实收益不是“少写代码”，而是**：

1. 一次性省 **30–45 人日**（A 类基座不再自建）；
2. 长期每年省 **25–40 人日**（不必维护 agent loop、工具系统、终端、文件、设置、审批、沙箱、MCP、插件市场）；
3. 风险结构从“自研 15 个插件”变成“集成 1 个宿主 + 自建 8 个业务插件”。

## 6. 关键结论

1. **DSH 市场覆盖的是“通用基座”，不是“垂直业务”**。A 类 18 个域几乎全部可替代；C 类 11 个域几乎全部要自建。
2. **成本主体是股票业务（C2+C11 合计 55–85 人日）与知识库（C6，20–30 人日）**，二者都无市场替代。
3. **替代优先无法改变“业务层要重写”的事实**，只能把基座成本从 40–60 降到 8–15。
4. 如果预算有限，正确的切口是：先做 A 类替换 + C1（股票数据）+ 一个前端切片，用最小闭环验证宿主能力，而不是先啃 C2。

## 7. 待确认

- B1（工作流）是否接受从“可视化 DAG”改为“JS 脚本编排”？这决定 15–25 人日的走向。
- B3（Provider）是否必须保留 OpenAI/Anthropic/本地模型？若是，+8–12 人日。
- C6（知识库）是否在迁移范围内？若是，+20–30 人日。
