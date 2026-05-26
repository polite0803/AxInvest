[**English**](./README-EN.md) | **简体中文** | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp&amp&utm_source=badge-featured&amp&amp;&amp;#10;&amp;amp&amp&amp;;utm_medium=badge&amp&amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - AI 驱动的智能投资分析平台 | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>AI 驱动的智能投资分析 | 多智能体协作 | 本地优先</strong>
</p>

<p align="center">
  <a href="https://github.com/polite0803/AxAgent/releases" target="_blank">
    <img src="https://img.shields.io/github/v/release/polite0803/AxAgent?style=flat-square" alt="Release">
  </a>
  <a href="https://github.com/polite0803/AxAgent/actions" target="_blank">
    <img src="https://img.shields.io/github/actions/workflow_status/polite0803/AxAgent/release.yml?style=flat-square" alt="Build">
  </a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20Android%20%7C%20iOS-blue?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/license-AGPL--3.0-green?style=flat-square" alt="License">
</p>

---

## 什么是 AxInvest？

**AxInvest v2.3** 是一款 AI 驱动的智能投资分析平台，基于 AxAgent 多智能体框架构建。它将先进的 AI 智能体能力与专业的 A 股投资分析深度融合，支持多模型提供商、AI 智能体研究、可视化工作流编排、本地知识管理、内置 API 网关，覆盖 **Windows / macOS / Linux / Android / iOS** 五大平台，并针对**桌面、平板、手机**三档设备自适应布局。

AxInvest 的核心特色在于利用多智能体对抗辩论、深度研究和事实核查等机制，为投资决策提供全面、客观的分析支持。

---

## 截图预览

| 对话与模型选择 | 多智能体仪表盘 |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| 知识库 RAG | 记忆与上下文 |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| 工作流编辑器 | API 网关 |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## 核心功能

### 📈 智能投资分析

AxInvest 的核心特色模块，将 AI 智能体能力与专业投资分析深度融合：

**多源数据聚合与降级**

- **9 大数据源** — 腾讯财经、通达信 (mootdx)、东方财富、新浪财经、百度股票、同花顺 (THS)、问财 (Iwencai)、巨潮资讯 (cninfo)、AKShare
- **22 种数据路由** — 每种数据类型配置多源降级路由，主源不可用时自动切换至备用源
- **并发数据采集** — `tokio::join!` 并发拉取 16 种个股数据 + 5 种市场数据，最大化采集效率
- **智能缓存** — LRU 内存缓存（1000 条上限），行情 30s TTL / K 线 300s TTL，自动过期淘汰
- **健康检查** — 供应商连通性探针（平安银行 000001 做探针），支持运行时检测数据源可用性

**A 股市场识别与规则**

- **板块识别** — 根据代码前缀自动识别：沪主板(6)、科创板(688)、深主板(0)、创业板(3)、北交所(8)
- **涨跌停规则** — 科创板/创业板 ±20%、北交所 ±30%、主板 ±10%、ST 股 ±5%
- **交易日历** — 内置 2025-2026 年 A 股节假日和调休工作日，支持交易日判断

**个股数据（16 类）**

- **实时行情** — 价格、涨跌幅、成交量/额、换手率、PE/PB、总市值、涨停价/跌停价、ST 标识
- **K 线数据** — 7 种周期（5 分/15 分/30 分/60 分/日/周/月），含成交量、成交额、换手率
- **财务分析** — 营收、净利润、EPS、BPS、ROE、负债率、毛利率、净利率、营收同比、利润同比
- **资金流向** — 主力/超大单/大单/中单/小单净流入
- **龙虎榜** — 营业部买卖金额、净额、上榜原因
- **限售解禁** — 解禁日期、解禁股数、解禁比例、股东信息
- **融资融券** — 融资买入额/余额、融券卖出量/余量
- **北向资金** — 持股数量、持股占比、变动数量
- **行业分类** — 申万一级/二级行业、概念板块标签
- **股东增减持** — 重要股东增减持动态、增减持原因
- **分红记录** — 除权除息日、每股分红、送转比例、股权登记日
- **研报聚合** — 券商研究报告，含机构、分析师、评级、目标价、EPS 预测
- **一致预期 EPS** — 机构一致预期 EPS、一致目标价、平均评级、评级数量
- **概念板块** — 三维归属（行业/概念/地域），含板块涨跌幅
- **公告检索** — 巨潮资讯上市公司公告，含公告类型和 PDF 链接
- **新闻舆情** — 新闻标题/摘要/来源，含情绪评分

**市场数据（5 类）**

- **全市场龙虎榜** — 当日所有上榜股票，含净买入、买卖金额
- **热门股票** — 同花顺强势股，含涨跌幅、换手率、原因标签、所属板块
- **行业排名** — 申万行业涨跌幅、成交额、领涨股
- **财联社快讯** — 实时财经快讯，含标题、内容、来源
- **北向资金流向** — 沪/深/合计分钟级资金流向

**技术指标计算（indicators 模块）**

- **均线系统** — MA5/MA10/MA20/MA60，含排列状态判断（多头/空头/弱多头/缠绕交叉）
- **MACD** — DIF/DEA/柱状图，含信号判断（金叉/死叉/多头运行/空头运行）
- **RSI** — RSI6/RSI12/RSI24，含信号判断（超买/超卖/强势/弱势/中性）
- **布林带** — 上轨/中轨/下轨 (20,2)，含位置判断（上轨以上/上轨区间/中轨附近/下轨区间/下轨以下）
- **乖离率** — MA5 乖离率、MA20 乖离率
- **量能分析** — 量比（当日量/5 日均量），含信号判断（放量上涨/缩量回调/放量下跌/缩量上涨/正常）
- **支撑/压力位** — 基于近期高低点和均线自动计算

**MCP 工具注册（mcp_tools 模块）**

- 股票数据能力通过 MCP 协议注册为标准工具，AI 智能体可在对话中直接调用
- 注册工具：search_stock、get_stock_quote、get_stock_kline、get_stock_financials、get_stock_news、get_stock_money_flow、get_stock_dragon_tiger 等

**AI 分析流水线（stock-analysis crate，23 个子模块）**

- **分析编排** — orchestrator（流水线编排）、pipeline（多阶段管道）、runner（任务执行器）
- **决策引擎** — decision（投资决策）、signals（交易信号生成）、rules（交易规则引擎）
- **风险评估** — risk（风险评估模型）、portfolio_risk（组合风险）、position_limits（仓位限制与合规）
- **选股与回测** — screener（多条件选股器）、backtest（策略回测引擎）、trading（交易策略框架）
- **价值投资** — value（价值分析）、value_investing（价值投资评估框架）
- **质量控制** — quality（数据质量检查）、data_clean（数据清洗与预处理）、review（分析结果复核）
- **报告与评分** — report（分析报告生成）、scoring（综合评分系统）
- **辅助模块** — key_levels（关键价位识别）、monitor（实时监控与预警）、plugin（分析插件扩展）、prompts（AI 提示词模板）

**前端分析组件（16 个）**

- StockAnalysisPage、StockQuoteCard、KLineChart、RiskMatrix、TradePanel
- DecisionBanner、DebatePanel、WatchlistPanel、PriceAlertPanel、CompareView
- AnalystReportGrid、AnalystReportCard、HistoricalAnalysisPanel、StockSearchBar
- AnalysisProgress、StockAnalysisSettingsModal、StockAnalysisChatIndicator

**对抗辩论与决策**

- **对抗辩论** — 多智能体 Pro/Con 辩论，支持论点强度评分和反驳追踪
- **决策横幅** — 买入/卖出/持有决策可视化，含置信度和理由
- **AI 工作流集成** — 股票分析工作流与对话无缝衔接（stockWorkflowChatBridge）

### 🤖 AI 模型支持

- **多提供商支持** — 原生集成 OpenAI、Anthropic Claude、Google Gemini、Ollama、OpenClaw、Hermes 及所有 OpenAI 兼容 API
- **多 Key 轮换** — 为每个提供商配置多个 API Key，自动轮换分发限流
- **本地模型推理** — 完整支持 Ollama 本地模型，包含 GGUF/GGML 文件管理
- **Candle 推理引擎** — 内置 Candle 本地推理，支持 rerank/judge 接口，GGUF 按需下载
- **模型管理** — 远程模型列表获取，可自定义参数（temperature、max tokens、top-p 等）
- **流式输出** — 实时逐 token 渲染，支持可折叠的思考块（Claude 扩展思考）
- **多模型对比** — 同时向多个模型提问，side-by-side 对比结果
- **函数调用** — 跨所有支持提供商的结构化函数调用
- **OpenAI Responses API** — 支持 OpenAI Responses 格式传输
- **实时 API** — 兼容 OpenAI 实时 API 的 WebSocket 事件推送
- **图像生成** — AI 图像生成面板，支持多种模型和参数配置

### 🔐 AI 智能体系统

智能体系统基于精密架构构建（agent crate，70+ 源文件），具备以下特性：

- **ReAct 推理引擎** — 融合推理与行动，内置自验证确保任务执行可靠
- **层级规划器** — 将复杂任务分解为具有阶段和依赖关系的结构化计划
- **任务分解器** — 自动将复杂任务分解为可执行的子任务
- **思维链** — 智能体决策推理的可视化，逐步分解
- **思维树** — tree_of_thoughts 多路径推理探索
- **深度研究** — 多源搜索编排、引用追踪与可信度评估
- **事实核查** — AI 驱动的事实验证与来源分类
- **搜索编排** — 多搜索提供商协调，支持搜索规划和结果综合
- **学术搜索** — 学术文献检索和引用分析
- **计算机控制** — AI 控制的鼠标点击、键盘输入、屏幕滚动，配合视觉模型分析
- **屏幕感知** — 截图捕获和视觉模型分析，用于 UI 元素识别
- **视觉管线** — vision_pipeline 图像理解与分析
- **三级权限模式** — 默认（需要审批）、接受编辑（自动批准）、完全访问（无提示）
- **沙箱隔离** — 智能体操作严格限制在指定工作目录内
- **工具审批面板** — 实时显示工具调用请求，支持逐条审批
- **成本追踪** — 实时显示每个会话的 token 使用量和成本统计
- **暂停/恢复** — 随时暂停智能体执行，稍后恢复
- **检查点系统** — 持久化检查点用于崩溃恢复和会话重连
- **错误恢复引擎** — 自动错误分类、根因分析和恢复策略执行
- **循环检测** — 自动检测和中断智能体推理中的循环行为
- **主动模式** — 智能体可主动提供建议和执行操作
- **目的管理** — 维护和追踪智能体的执行目的与上下文
- **自验证** — self_verifier 自动验证智能体输出正确性
- **反思器** — reflector 对推理过程进行反思和改进
- **引导输入** — steer_manager 动态调整智能体行为方向
- **事件总线** — event_bus / event_emitter 智能体事件驱动架构
- **内容综合** — content_synthesizer 多源信息综合与报告生成
- **引用追踪** — citation_tracker 自动追踪和标注信息来源
- **可信度评估** — credibility_evaluator 评估信息来源可信度
- **大纲构建** — outline_builder 自动构建研究大纲
- **模式管理** — schema_manager 管理输出结构模式
- **项目记忆** — project_memory 项目级别的持久化记忆
- **环境探测** — environment_probe 自动探测运行环境信息
- **健康检查** — health_checker 智能体健康状态监控

### 👥 多智能体协作

- **子智能体协调** — 主从架构，coordinator 协调多个协作智能体
- **并行执行** — 多个智能体并行处理任务，支持依赖感知调度
- **对抗性辩论** — adversarial_debate Pro/Con 辩论轮次，支持论点强度评分和反驳追踪
- **智能体角色** — agent_roles 预定义角色（研究员、规划师、开发者、评审员、综合员）用于团队协作
- **智能体编排器** — 多智能体团队的中心化消息路由和状态管理
- **通信图谱** — graph_insights 智能体交互和消息流的可视化展示
- **共享黑板** — shared_blackboard / blackboard 跨智能体共享状态空间
- **Buddy 伙伴系统** — 可配置的智能体伙伴，支持物种和属性定义
- **共享记忆** — 跨智能体共享的内存空间，支持统计和查询
- **团队 Cron 注册** — 团队级别的定时任务调度
- **专家系统** — agency_expert 领域专家智能体
- **智能体画像** — agent_profile 智能体个性与能力画像管理

### ⭐ 技能系统

- **技能市场** — 内置市场，浏览和安装社区贡献的技能
- **技能创建** — 从提案自动创建技能，支持 Markdown 编辑器
- **技能进化** — skill_evolution 基于执行反馈的 AI 驱动的现有技能自动分析和改进
- **技能匹配** — skill_matcher 语义匹配，推荐与对话上下文相关的技能
- **技能分解** — 自动将复杂任务分解为可执行的原子技能（LLM 辅助/多轮/工作流验证）
- **生成工具** — AI 自动生成并注册新工具，扩展智能体能力
- **技能中心** — skills_hub_adapter 集中的技能发现和配置管理界面
- **技能中心客户端** — skills_hub_client 与远程技能中心集成，支持社区分享
- **技能依赖检查** — 自动检测技能依赖和工具可用性
- **技能沙箱容器** — 技能在隔离环境中安全执行
- **原子技能** — atomic_skill 最小可执行技能单元
- **技能提案** — skill_proposal AI 驱动的技能创建提案

### 🔄 工作流系统

工作流引擎（rt-workflow crate）实现了基于 DAG 的任务编排系统：

- **可视化工作流编辑器** — 拖放式工作流设计器，支持节点连接和配置
- **16 种节点类型** — 触发器、智能体、LLM、条件、并行、循环、合并、延迟、工具、代码、子工作流、向量检索、文档解析、验证、结束、回退（fallback）
- **16 种属性面板** — 每种节点类型对应独立的配置面板
- **工作流模板** — 内置预设：代码审查、Bug 修复、文档、测试、重构、探索、性能、安全、功能开发
- **DAG 执行** — Kahn 算法拓扑排序，支持循环检测
- **并行调度** — 流水线式执行，快速步骤不等慢速步骤
- **重试策略** — 指数退避，每步可配置最大重试次数
- **部分完成** — 失败的步骤不会阻塞独立的下游步骤
- **版本管理** — 工作流模板版本控制，支持回滚
- **执行历史** — 详细记录，支持状态追踪和调试
- **AI 辅助** — AI 辅助工作流设计、节点推荐和智能体提示词优化
- **语义检查** — 工作流语义验证，检测潜在问题
- **n8n 导入** — 支持从 n8n 目录导入工作流
- **调试面板** — 工作流执行过程的实时调试和状态查看
- **缓存层** — cache_layer 工作流执行结果缓存
- **市场** — workflow_marketplace 工作流模板市场与评审

### 📚 知识与记忆

- **知识库（RAG）** — 多知识库支持，支持文档上传、自动解析、分块和向量索引
- **混合搜索** — 结合向量相似度搜索与 BM25 全文排名
- **重排序** — Cross-encoder 重排序，提升检索精度
- **三级召回管道** — AST 索引 + 向量搜索 + FTS5 的多级召回机制
- **Self-RAG** — self_rag 自适应检索增强生成
- **查询增强** — query_enhancement 查询改写与扩展
- **知识图谱** — 知识关联的实体关系可视化（实体、属性、关系、流、接口）
- **Wiki 系统** — LLM Wiki 编译器与验证器，支持知识图谱可视化与增量同步
- **Wiki 笔记** — 双向链接笔记系统，支持图谱视图和自动链接同步
- **记忆系统** — 多命名空间记忆，支持手动录入或 AI 自动提取
- **闭环记忆** — 集成 Honcho 和 Mem0 持久化记忆提供商
- **记忆遗忘** — memory_forgetting 基于时间的记忆衰减机制
- **FTS5 全文搜索** — 跨对话、文件、记忆的快速检索
- **会话搜索** — 跨所有对话会话的高级搜索
- **上下文管理** — 灵活附加文件、搜索结果、知识片段、记忆、工具输出
- **文档解析** — 多格式文档自动解析和内容提取
- **增量索引** — 文件变更的增量索引更新
- **文本分块** — text_chunker 智能文本分块策略
- **Token 预算** — token_budget 检索结果 token 预算控制

### 🌐 API 网关

- **本地 API 服务器** — 内置 OpenAI 兼容、Claude 和 Gemini 接口服务器
- **外部链接** — 一键集成 Claude CLI、OpenCode，自动同步 API Key 和模型
- **Key 管理** — 生成、撤销、启用/禁用访问 Key，支持描述
- **用量分析** — 按 Key、提供商、日期的请求量和 token 使用量
- **SSL/TLS 支持** — 内置自签名证书，支持自定义证书
- **请求日志** — 完整记录所有 API 请求和响应
- **配置模板** — Claude、Codex、OpenCode、Gemini 的预建模板
- **实时 API** — 兼容 OpenAI 实时 API 的 WebSocket 事件推送
- **平台集成** — 支持钉钉、飞书、QQ、Slack、微信、WhatsApp、Telegram、Discord
- **网关诊断** — 连接诊断和程序策略管理
- **限流器** — API 请求速率限制和流量控制
- **持久化队列** — 请求持久化队列管理
- **股票 API** — stock_handlers 股票数据专用 API 端点
- **SSE 推送** — sse Server-Sent Events 实时事件推送

### 🔧 工具与扩展

- **MCP 协议** — 完整的模型上下文协议实现，支持 stdio 和 HTTP/WebSocket 传输
- **OAuth 认证** — MCP 服务器的 OAuth 流程支持
- **MCP 自动启动** — MCP 服务器自动启动和生命周期管理
- **MCP 工具桥接** — MCP 工具与智能体工具系统的桥接
- **MCP 健康检查** — mcp_health MCP 服务器健康状态监控
- **插件系统** — OpenClaw 兼容的三级插件架构（内置/捆绑/外部），支持 npm 包安装、工具注册、钩子与生命周期管理
- **插件市场** — 内置市场 UI，支持 npm 搜索安装、确认弹窗
- **内置工具** — 40+ 工具模块：文件操作（读/写/编辑/系统）、代码执行、搜索（Grep/Glob）、Bash、Web 搜索/抓取、计划管理、Cron 调度、REPL、LSP、上下文管理、计算机控制、消息推送、待办事项、数据库、DevOps、文档解析、Git、知识检索、LSP、媒体处理、消息推送、OCR、推送通知、系统信息、任务系统、测试、工作区/工作树等
- **工具权限系统** — 工具权限分类、规则管理和使用追踪
- **Bash 安全** — 命令解析、路径验证和沙箱安全控制
- **LSP 客户端** — 内置语言服务器协议，支持代码补全和诊断
- **AST 索引** — 代码文件的 AST 解析和索引构建
- **终端后端** — 支持本地、Docker 和 SSH 终端连接
- **浏览器自动化** — 通过 CDP 集成浏览器控制能力（导航、截图、点击、填写、文本提取等）
- **UI 自动化** — 跨平台 UI 元素识别和控制
- **Git 工具** — Git 操作，支持分支检测和冲突感知
- **工具推荐** — 基于上下文的智能工具推荐引擎
- **工具编排** — 多工具协调执行和流式输出
- **工具统计** — 工具使用频率和性能统计
- **工具审计** — audit 工具调用审计日志

### 📊 内容渲染

- **Markdown 渲染** — 完整支持代码高亮、LaTeX 数学公式、表格、任务列表
- **Monaco 代码编辑器** — 内置编辑器，支持语法高亮、复制、差异预览
- **图表渲染** — Mermaid 流程图、D2 架构图、ECharts 交互式图表
- **产物面板** — 代码片段、HTML 草稿、React 组件、Markdown 笔记，支持实时预览
- **四种预览模式** — 代码（编辑器）、分屏（并排）、预览（仅渲染）、React 组件预览
- **会话检查器** — 会话结构的树形视图，快速导航
- **引用面板** — 追踪和显示来源引用，支持可信度评分
- **信息图渲染** — 支持信息图可视化展示
- **图表解释器** — ChartInterpreter AI 驱动的图表解读
- **差异查看器** — DiffViewer 代码差异对比

### 🛡️ 数据与安全

- **AES-256 加密** — API Key 和敏感数据使用 AES-256-GCM 加密
- **隔离存储** — 应用状态存储在 `~/.axinvest/`，用户文件存储在 `~/Documents/axinvest/`
- **自动备份** — 计划备份到本地目录或 WebDAV 存储
- **S3 备份** — s3_backup 支持 Amazon S3 云端备份
- **备份恢复** — 一键从历史备份恢复
- **导出选项** — PNG 截图、Markdown、纯文本、JSON 格式
- **存储管理** — 可视化磁盘使用显示和清理工具
- **存储迁移** — storage_migration 版本间数据迁移
- **文件授权** — 文件访问授权和撤销管理
- **操作审计** — 关键操作的审计日志记录
- **命令验证** — command_validator 命令安全验证
- **资源限制** — resource_limits 资源使用限制
- **沙箱运行** — sandbox_runner 隔离环境执行

### 🖥️ 桌面体验

- **主题引擎** — 深色/浅色主题，支持跟随系统或手动偏好
- **界面语言** — 11 种语言：简体中文、繁体中文、英语、日语、韩语、法语、德语、西班牙语、俄语、印地语、阿拉伯语
- **系统托盘** — 最小化到托盘，不中断后台服务
- **置顶窗口** — 窗口置顶于其他窗口之上
- **全局快捷键** — 可自定义快捷键调出主窗口
- **QuickBar** — 快速访问浮动条，一键唤起
- **开机自启** — 可选在系统启动时运行
- **代理支持** — HTTP 和 SOCKS5 代理配置
- **自动更新** — 自动检查版本，有更新时提示
- **命令面板** — `Cmd/Ctrl+K` 快速访问命令
- **引导向导** — 首次使用的交互式引导和 Ollama 检测
- **通知中心** — 统一的应用内通知管理
- **云工作区** — cloud_workspace 云端工作区选择
- **崩溃报告** — crash_report 自动崩溃报告收集
- **语音通话** — VoiceCall 语音对话能力

### 🔬 高级功能

- **深度研究** — 多源搜索、引用追踪、可信度评估与内容综合
- **事实核查** — AI 驱动的事实验证与来源分类
- **Cron 调度器** — 自动化任务调度，支持每日/每周/每月模板和自定义 cron 表达式
- **Webhook 系统** — 事件订阅，支持工具完成、智能体错误、会话结束通知
- **用户画像** — 自动学习代码风格、命名规范、缩进、注释风格、沟通偏好
- **RL 优化器** — 强化学习优化工具选择和任务策略
- **LoRA 微调** — 使用 LoRA 进行本地训练的自定义模型适配
- **主动建议** — 基于对话内容和用户模式的上下文感知提示
- **上下文预测** — 预测用户下一步操作并预取相关资源
- **梦境整合** — dream_consolidation 后台自动整合记忆与模式，优化长期知识
- **错误恢复** — 自动错误分类、根因分析和恢复建议
- **开发者工具** — Trace、Span、时间线可视化，用于调试和性能分析
- **基准测试系统** — SWE-bench / Terminal-bench 任务性能评估和指标，带评分卡
- **风格迁移** — style_migrator 将学习的代码风格偏好应用到生成的代码
- **仪表盘插件** — 可扩展的仪表盘，支持自定义面板和小组件
- **协作共享** — CRDT 实时协作与一键会话分享
- **浏览器扩展** — Wiki Clipper 浏览器扩展，快速剪藏网页到 LLM Wiki
- **Python SDK** — 提供 Python SDK 用于与 AxInvest 集成
- **智能路由** — 请求智能路由和分类
- **语义缓存** — 基于语义的响应缓存，减少重复计算
- **上下文压缩** — 自动压缩长上下文，优化 token 使用
- **消息批量处理** — 消息批量发送和优化
- **连接池** — 数据库和 API 连接池管理
- **特性开关** — 可配置的功能特性开关系统
- **策略引擎** — 权限和操作策略的集中管理
- **资源治理** — 智能体资源使用限制和治理
- **LAN 传输** — 局域网文件传输能力
- **共进化** — coevolution 技能与智能体协同进化
- **行为学习** — behavior_learner / behavior_tracker 用户行为学习与追踪
- **偏好学习** — preference_learner 用户偏好自动学习
- **内在奖励** — intrinsic_reward 内在动机驱动的探索
- **过程奖励** — process_reward 过程级奖励信号
- **TextGrad** — text_grad 基于文本梯度的自动优化
- **轨迹压缩** — trajectory_compressor 长轨迹自动压缩
- **提醒管理** — reminder_manager 智能提醒调度
- **任务预取** — task_prefetcher 预测性任务资源预取

### 🛡️ 提示词注入防护（Prompt-Guard）

- **四级防护体系** — L1 模式检测（高风险拦截 + 中风险标记）→ L2 分隔符转义 → L3 XML 包装器 → L4 信任标签
- **Pipeline 编排器** — 多级检测管道串联，支持自定义风险阈值
- **Token Smuggling 检测** — 针对编码混淆和 token 走私攻击的专项检测
- **分隔符转义检测** — delimiter_escape 检测提示词分隔符逃逸攻击
- **模式检测** — pattern_detect 正则+启发式注入模式匹配
- **信任标签** — trust_labels 可信内容标记与验证
- **Strict 模式** — 严格模式测试 + 中风险原因命名 + 自定义模式文档
- **全管道集成** — 已集成到 session / prompt / git / RAG 各环节

### 📱 移动端支持

- **Android 原生** — APK/AAB 构建，支持 arm64-v8a / armeabi-v7a / x86_64
- **iOS 原生** — IPA 构建，支持 arm64
- **自适应布局** — 桌面/平板/手机三档自动适配（useResponsive hook）
- **移动端导航** — Drawer 滑出导航 + 底部导航栏 + 闪现式浮动按钮
- **安全区适配** — Android 系统状态栏/导航栏 CSS env() 自适应
- **CSP 优化** — Android WebView CSP 协议白名单
- **条件编译** — `#[cfg(not(mobile))]` 桌面专属功能（浏览器、计算机控制、桌面、QuickBar、终端、屏幕视觉）自动排除

---

## 技术架构

### 技术栈

| 层级 | 技术 |
|------|------|
| **框架** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **状态管理** | Zustand 5 |
| **路由** | React Router 7 |
| **国际化** | i18next + react-i18next |
| **后端** | Rust 2024 + SeaORM 2 + SQLite |
| **向量数据库** | sqlite-vec |
| **代码编辑器** | Monaco Editor |
| **图表** | Mermaid + D2 + ECharts（CDN） |
| **终端** | xterm.js 6 |
| **工作流** | ReactFlow 11 |
| **图表渲染** | @antv/infographic |
| **图标** | Iconify + Lucide |
| **拖拽** | @dnd-kit |
| **构建** | Vite 8 + npm |
| **测试** | Vitest + Playwright + cargo-nextest |
| **格式化** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **移动端** | Tauri Android + iOS 原生构建 |
| **桌面端** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### 平台支持

| 平台 | 架构 |
|------|------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (模拟器) |
| iOS | arm64 |

### Rust 后端架构

后端组织为 Rust workspace，包含 **20 个** 专业化的 crates：

```
src-tauri/crates/
├── agent/            # AI 智能体核心（70+ 源文件：ReAct 引擎、协调、规划、深度研究、事实核查等）
├── astock-data/      # A 股数据源（9 大数据源、22 种数据路由、技术指标、交易日历、MCP 工具注册）
├── core/             # 核心工具（85+ 数据库实体、40+ 仓库、RAG、加密、MCP、浏览器自动化、AST 索引等）
├── gateway/          # API 网关（HTTP 服务器、认证、路由、OpenAI 兼容接口、股票 API 端点）
├── migration/        # 数据库迁移（5 个迁移：股票分析/自选组合/分析调度/价格预警/交易）
├── npm/              # npm 包解析与注册表
├── plugins/          # 插件系统（OpenClaw 兼容，npm 包安装，含示例插件）
├── prompt-guard/     # 提示词注入防护（L1-L4 多级检测与防御，4 种检测器）
├── providers/        # 模型提供商适配器（OpenAI、Anthropic、Gemini、Ollama、OpenClaw、Hermes、图像生成）
├── rt-dashboard/     # 仪表盘插件系统
├── rt-messaging/     # 消息网关（9 平台：钉钉/飞书/QQ/Slack/微信/WhatsApp/Telegram/Discord）
├── rt-theme/         # 主题引擎
├── rt-webhook/       # Webhook 服务器与分发
├── rt-workflow/      # 工作流引擎（DAG 编排、16 种节点执行器、调度器、缓存层）
├── runtime/          # 运行时服务（70+ 源文件：会话管理、MCP、终端、限流、Webhook、权限、基准测试等）
├── runtime-core/     # 运行时抽象层（公共类型、trait 定义、配置、特性开关、权限执行器）
├── stock-analysis/   # 智能投资分析（23 个子模块：流水线、决策引擎、风险评估、回测、选股器、价值投资）
├── telemetry/        # 遥测与分布式追踪（OpenTelemetry 兼容）
├── tools/            # 工具系统（40+ 内置工具、Bash 安全、MCP 桥接、权限系统、编排、审计）
└── trajectory/       # 学习系统（55+ 源文件：记忆、技能、RL、用户画像、梦境整合、风格迁移、共进化）
```

#### stock-analysis crate 模块结构（23 个子模块）

```
stock-analysis/
├── backtest.rs         # 策略回测引擎
├── data_clean.rs       # 数据清洗与预处理
├── decision.rs         # 投资决策引擎
├── key_levels.rs       # 关键价位识别
├── monitor.rs          # 实时监控与预警
├── orchestrator.rs     # 分析流水线编排
├── pipeline.rs         # 多阶段分析管道
├── plugin.rs           # 分析插件扩展
├── portfolio_risk.rs   # 投资组合风险评估
├── position_limits.rs  # 仓位限制与合规
├── prompts.rs          # AI 提示词模板
├── quality.rs          # 数据质量检查
├── report.rs           # 分析报告生成
├── review.rs           # 分析结果复核
├── risk.rs             # 风险评估模型
├── rules.rs            # 交易规则引擎
├── runner.rs           # 分析任务执行器
├── scoring.rs          # 综合评分系统
├── screener.rs         # 选股器
├── signals.rs          # 交易信号生成
├── trading.rs          # 交易策略框架
├── value.rs            # 价值分析
└── value_investing.rs  # 价值投资评估
```

#### astock-data crate 数据源

| 数据源 | 标识 | 支持的数据类型 |
|--------|------|---------------|
| 腾讯财经 | tencent | 实时行情、K 线 |
| 通达信 | mootdx | 实时行情、K 线 |
| 东方财富 | eastmoney | 行情、K 线、财务、资金流向、龙虎榜、限售解禁、融资融券、北向资金、行业分类、股东增减持、分红、研报、全市场龙虎榜、财联社快讯 |
| 新浪财经 | sina | 行情、K 线、新闻 |
| 百度股票 | baidu_stock | 行情、新闻、资金流向、龙虎榜、限售解禁、融资融券、北向资金、行业分类、股东增减持、分红、研报、热门股票、行业排名、概念板块、北向资金流向 |
| 同花顺 | ths | 行情、行业分类、一致预期 EPS、概念板块、热门股票、行业排名、北向资金流向 |
| 问财 | iwencai | 股票搜索、行业分类、一致预期 EPS、概念板块、热门股票 |
| 巨潮资讯 | cninfo | 公告 |
| AKShare | akshare | 财务、新闻、一致预期 EPS、财联社快讯 |

每个数据类型配置多源降级路由，当主数据源不可用时自动切换至备用源。

#### astock-data 额外模块

| 模块 | 功能 |
|------|------|
| calendar | A 股交易日历（2025-2026 年节假日 + 调休工作日） |
| indicators | 技术指标计算（MA/MACD/RSI/布林带/乖离率/量比/支撑压力位） |
| mcp_tools | MCP 工具注册（股票数据能力注册为 AI 可调用工具） |

### 前端架构

```
src/
├── stores/                    # Zustand 状态管理（65 个 store）
│   ├── domain/               # 核心业务状态（9 个）
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # 功能模块状态（46 个）
│   │   ├── agentProfileStore.ts
│   │   ├── agentStore.ts
│   │   ├── appConfigStore.ts
│   │   ├── backupStore.ts
│   │   ├── buddyStore.ts
│   │   ├── cacheStore.ts
│   │   ├── categoryStore.ts
│   │   ├── citationStore.ts
│   │   ├── continuationStore.ts
│   │   ├── decompositionStore.ts
│   │   ├── dreamStore.ts
│   │   ├── executionStore.ts
│   │   ├── expertStore.ts
│   │   ├── fileStore.ts
│   │   ├── gatewayLinkStore.ts
│   │   ├── gatewayStore.ts
│   │   ├── generatedToolStore.ts
│   │   ├── helpStore.ts
│   │   ├── knowledgeStore.ts
│   │   ├── llmWikiStore.ts
│   │   ├── localToolStore.ts
│   │   ├── mcpStore.ts
│   │   ├── memoryStore.ts
│   │   ├── nudgeStore.ts
│   │   ├── onboardingStore.ts
│   │   ├── planStore.ts
│   │   ├── platformStore.ts
│   │   ├── proactiveStore.ts
│   │   ├── promptTemplateStore.ts
│   │   ├── providerStore.ts
│   │   ├── searchStore.ts
│   │   ├── settingsStore.ts
│   │   ├── skillExtensionStore.ts
│   │   ├── skillStore.ts
│   │   ├── sourceStore.ts
│   │   ├── stockAnalysisStore.ts
│   │   ├── stockWorkflowChatBridge.ts
│   │   ├── styleStore.ts
│   │   ├── terminalStore.ts
│   │   ├── themeStore.ts
│   │   ├── topicGroupStore.ts
│   │   ├── trajectoryStore.ts
│   │   ├── userProfileStore.ts
│   │   ├── wikiStore.ts
│   │   ├── workEngineStore.ts
│   │   └── workflowEditorStore.ts
│   ├── devtools/              # 开发者工具状态（5 个）
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # 共享状态（5 个）
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # React 组件（25 个模块）
│   ├── chat/                # 对话界面（100+ 组件：Agent 执行面板、分支对比、浏览器自动化、代码执行器、协作面板、深度研究、事实核查、Git 提交、图像生成/分析、知识检索、记忆提取、模型路由、多模型展示、权限管理、插件市场、反思面板、技能创建/进化、结构化思考、子智能体卡片、工具调用卡片、轨迹回放、语音通话、Wiki 检索、工作流进度等）
│   ├── stock-analysis/      # 智能投资分析（16 个组件）
│   │   ├── StockAnalysisPage.tsx
│   │   ├── StockQuoteCard.tsx
│   │   ├── KLineChart.tsx
│   │   ├── RiskMatrix.tsx
│   │   ├── TradePanel.tsx
│   │   ├── DecisionBanner.tsx
│   │   ├── DebatePanel.tsx
│   │   ├── WatchlistPanel.tsx
│   │   ├── PriceAlertPanel.tsx
│   │   ├── CompareView.tsx
│   │   ├── AnalystReportGrid.tsx
│   │   ├── AnalystReportCard.tsx
│   │   ├── HistoricalAnalysisPanel.tsx
│   │   ├── StockSearchBar.tsx
│   │   ├── AnalysisProgress.tsx
│   │   └── StockAnalysisSettingsModal.tsx
│   │   └── StockAnalysisChatIndicator.tsx
│   ├── workflow/            # 工作流编辑器（16 种节点 + 16 种属性面板 + AI 面板 + 模板 + 调试）
│   ├── gateway/             # API 网关 UI（概览/密钥/指标/监控/设置/模板/诊断）
│   ├── settings/            # 设置面板（50+ 组件：提供商/模型/MCP/知识/记忆/代理/快捷键/主题/工具/Webhook/Cron/股票分析配置等）
│   ├── terminal/            # 终端 UI（集成终端/Docker/SSH/后端选择/路径补全/斜杠补全）
│   ├── skill/               # 技能编辑器与渲染器（动作链编辑/前端编辑器/沙箱容器/依赖检查/统计面板）
│   ├── benchmark/           # 基准测试面板（配置/报告/选择器/任务列表/结果）
│   ├── files/               # 文件管理页面
│   ├── fine-tune/           # LoRA 微调配置（数据集/训练任务/LoRA 配置）
│   ├── link/                # 外部链接管理（概览/模型/策略/技能/策略详情）
│   ├── llm-wiki/            # LLM Wiki 编辑器（质量评分/同步状态）
│   ├── proactive/           # 主动建议系统（上下文预测/预取指示器/建议栏/提醒列表）
│   ├── wiki/                # Wiki 管理（反向链接/图谱视图/摄入/代码检查/操作时间线/标签聚合/版本历史）
│   ├── devtools/            # Trace/Span 时间线（成本图表/持续时间图表/详情/过滤器/列表）
│   ├── decomposition/       # 技能分解（分解预览/工具依赖/工具生成/工具安装）
│   ├── recommendation/      # 工具推荐面板
│   ├── style/               # 代码风格迁移（样本/调整滑块/对比/预览面板）
│   ├── layout/              # 布局组件（标题栏/侧边栏/命令面板/全局复制/错误边界/状态栏/通知铃/用户画像模态框）
│   ├── help/                # 帮助面板
│   ├── notification/        # 通知中心
│   ├── search/              # 会话搜索
│   ├── onboarding/          # 引导向导（交互式教程/欢迎向导）
│   ├── common/              # 通用组件（复制/图标/模型参数滑块/粘贴）
│   └── shared/              # 共享组件（头像编辑/模态框/图表渲染/动态图标/嵌入模型选择/Emoji 选择/知识库图标/MCP 图标/模型选择/Monaco 编辑器/命名空间图标/搜索提供商图标）
│
├── pages/                    # 页面组件（22 个页面）
│   ├── ChatPage.tsx
│   ├── StockAnalysisPage.tsx
│   ├── KnowledgeHubPage.tsx
│   ├── MemoryPage.tsx
│   ├── WorkflowPage.tsx
│   ├── WorkflowMarketplace.tsx
│   ├── GatewayPage.tsx
│   ├── GatewayLinkPage.tsx
│   ├── LinkPage.tsx
│   ├── FilesPage.tsx
│   ├── FineTunePage.tsx
│   ├── SkillsPage.tsx
│   ├── WikiEditPage.tsx
│   ├── WikiEditorPage.tsx
│   ├── WikiGraphPage.tsx
│   ├── IngestPage.tsx
│   ├── QuickBarPage.tsx
│   ├── SettingsPage.tsx
│   ├── TerminalPage.tsx
│   └── DevTools/
│       ├── TraceExplorer.tsx
│       ├── BenchmarkRunner.tsx
│       └── ToolRecommender.tsx
│
├── hooks/                    # React hooks（12 个）
│   ├── useCommandPalette.ts
│   ├── useCopyToClipboard.ts
│   ├── useDebounce.ts
│   ├── useGlobalOverlayScrollbars.ts
│   ├── useGlobalShortcutManager.ts
│   ├── useKeyboardShortcuts.ts
│   ├── usePageRouting.ts
│   ├── useResolvedAvatarSrc.ts
│   ├── useResolvedDarkMode.ts
│   ├── useResponsive.ts
│   ├── useUpdateChecker.tsx
│   └── useVoiceChat.ts
│
├── lib/                      # 工具函数（33 个模块 + Web Worker）
│   ├── workers/            # Web Worker（heavy.worker.ts）
│   ├── actionRouter.ts     # 动作路由
│   ├── artifactRenderer.ts # 产物渲染
│   ├── chartGenerator.ts   # 图表生成
│   ├── chatMarkdown.ts     # Markdown 渲染
│   ├── codeExecutor.ts     # 代码执行
│   ├── invoke.ts           # Tauri IPC 封装
│   ├── skillActionExecutor.ts  # 技能动作执行
│   ├── skillEventBus.ts    # 技能事件总线
│   ├── skillLifecycle.ts   # 技能生命周期
│   ├── skillPermissions.ts # 技能权限
│   ├── storeRegistry.ts    # Store 注册表
│   ├── tokenEstimator.ts   # Token 估算
│   ├── workflowLayout.ts   # 工作流布局
│   └── ...                 # 其他工具模块
│
├── types/                    # TypeScript 类型定义（22 个）
│   ├── agent.ts
│   ├── agentProfile.ts
│   ├── artifact.ts
│   ├── backup.ts
│   ├── citation.ts
│   ├── evaluator.ts
│   ├── expert.ts
│   ├── index.ts
│   ├── knowledge.ts
│   ├── llmWiki.ts
│   ├── localTool.ts
│   ├── mcp.ts
│   ├── memory.ts
│   ├── nudge.ts
│   ├── permission.ts
│   ├── platform.ts
│   ├── proactive.ts
│   ├── search.ts
│   ├── stock-analysis.ts
│   ├── style.ts
│   ├── tracer.ts
│   └── wiki.ts
│
├── sdk/                      # SDK（含 Python SDK）
│   ├── index.ts
│   ├── types.ts
│   ├── rpcBridge.ts
│   ├── sandboxTemplate.ts
│   └── python/              # Python SDK
│       ├── setup.py
│       └── axagent_sdk/__init__.py
│
└── i18n/                     # 11 种语言翻译
```

## 快速开始

### 下载预构建版本

访问 [Releases](https://github.com/polite0803/AxAgent/releases) 页面，下载适合您平台的安装程序。

### 从源码构建

#### 环境要求

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + Rust MSVC targets

#### 构建步骤

```bash
# 克隆仓库
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# 安装依赖
npm install

# 开发模式
npm run tauri dev

# 仅构建前端
npm run build

# 构建桌面应用
npm run tauri build
```

构建产物位于 `src-tauri/target/release/`。

### 测试

```bash
# 单元测试
npm run test          # Vitest watch
npm run test:run      # Vitest 单次运行

# E2E 测试
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright UI 模式

# Rust 后端测试
cd src-tauri && cargo nextest run   # cargo-nextest（快 2-3x）
cd src-tauri && cargo test          # 标准测试

# 类型检查
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# 代码格式化
npm run format        # dprint
cd src-tauri && cargo fmt

# CI 全量检查
npm run ci:check
```

---

## 项目结构

```
AxInvest/
├── src/                         # 前端源码 (React + TypeScript)
│   ├── components/              # React 组件（25 个模块）
│   │   ├── chat/               # 对话界面（100+ 组件）
│   │   ├── stock-analysis/     # 智能投资分析（16 个组件）
│   │   ├── workflow/           # 工作流编辑器（16 种节点 + 属性面板 + AI 面板）
│   │   ├── gateway/            # API 网关组件
│   │   ├── settings/           # 设置面板（50+ 组件）
│   │   ├── terminal/           # 终端组件
│   │   ├── skill/              # 技能编辑器与渲染器
│   │   ├── benchmark/          # 基准测试
│   │   ├── files/              # 文件管理
│   │   ├── fine-tune/          # LoRA 微调
│   │   ├── link/               # 外部链接
│   │   ├── llm-wiki/           # LLM Wiki
│   │   ├── proactive/          # 主动建议
│   │   ├── wiki/               # Wiki 管理
│   │   ├── devtools/           # 开发者工具
│   │   ├── decomposition/      # 技能分解
│   │   ├── recommendation/     # 工具推荐
│   │   ├── style/              # 代码风格
│   │   ├── layout/             # 布局组件
│   │   ├── help/               # 帮助面板
│   │   ├── notification/       # 通知中心
│   │   ├── search/             # 会话搜索
│   │   ├── onboarding/         # 引导向导
│   │   ├── common/             # 通用组件
│   │   └── shared/             # 共享组件
│   ├── pages/                   # 页面组件（22 个页面）
│   ├── stores/                  # Zustand 状态管理（65 个 store）
│   │   ├── domain/            # 核心业务状态（9 个）
│   │   ├── feature/           # 功能模块状态（46 个）
│   │   ├── devtools/          # 开发者工具状态（5 个）
│   │   └── shared/            # 共享状态（5 个）
│   ├── hooks/                   # React hooks（12 个）
│   ├── lib/                     # 工具函数（33 个模块 + Web Worker）
│   ├── types/                   # TypeScript 类型定义（22 个）
│   ├── sdk/                     # SDK（TypeScript + Python）
│   └── i18n/                    # 11 种语言翻译
│
├── src-tauri/                    # 后端源码 (Rust)
│   ├── crates/                  # Rust workspace（20 个 crates）
│   │   ├── agent/             # AI 智能体核心（70+ 源文件）
│   │   ├── astock-data/       # A 股数据源（9 大数据源、22 种数据路由、技术指标、交易日历）
│   │   ├── core/              # 核心工具（85+ 实体、40+ 仓库、RAG、加密、MCP）
│   │   ├── gateway/           # API 网关（含股票 API 端点）
│   │   ├── migration/         # 数据库迁移（5 个迁移）
│   │   ├── npm/               # npm 包解析
│   │   ├── plugins/           # 插件系统
│   │   ├── prompt-guard/      # 提示词注入防护
│   │   ├── providers/         # 模型提供商适配器
│   │   ├── rt-dashboard/      # 仪表盘插件
│   │   ├── rt-messaging/      # 消息网关（9 平台）
│   │   ├── rt-theme/          # 主题引擎
│   │   ├── rt-webhook/        # Webhook 服务器
│   │   ├── rt-workflow/       # 工作流引擎（16 种节点执行器）
│   │   ├── runtime/           # 运行时服务（70+ 源文件）
│   │   ├── runtime-core/      # 运行时抽象层
│   │   ├── stock-analysis/    # 智能投资分析（23 个子模块）
│   │   ├── telemetry/         # 追踪与指标
│   │   ├── tools/             # 工具系统（40+ 内置工具）
│   │   └── trajectory/        # 学习系统（55+ 源文件）
│   └── src/                    # Tauri 入口点（91 个命令模块）
│       ├── commands/          # 命令模块
│       │   ├── stock_analysis.rs        # 股票分析命令
│       │   ├── stock_analysis_setup.rs  # 股票分析配置
│       │   ├── stock_workflow.rs        # 股票工作流命令
│       │   ├── agency_expert.rs         # 专家智能体
│       │   ├── agent_advanced.rs        # 高级智能体
│       │   ├── agent_analytics.rs       # 智能体分析
│       │   ├── agent_insight.rs         # 智能体洞察
│       │   ├── agent_nudge.rs           # 智能体提示
│       │   ├── agent_profile.rs         # 智能体画像
│       │   ├── agent_role.rs            # 智能体角色
│       │   ├── background_tasks.rs      # 后台任务
│       │   ├── browser.rs              # 浏览器自动化
│       │   ├── chart_generator.rs       # 图表生成
│       │   ├── cloud_workspace.rs       # 云工作区
│       │   ├── computer_control.rs      # 计算机控制
│       │   ├── context_breakdown.rs     # 上下文分解
│       │   ├── conversation_categories.rs  # 对话分类
│       │   ├── conversations_search.rs  # 对话搜索
│       │   ├── crash_report.rs          # 崩溃报告
│       │   ├── dream.rs                # 梦境整合
│       │   ├── evolution.rs            # 技能进化
│       │   ├── fine_tune.rs            # LoRA 微调
│       │   ├── gateway.rs              # API 网关
│       │   ├── gateway_link.rs         # 外部链接
│       │   ├── generated_tool.rs        # 生成工具
│       │   ├── image_gen.rs            # 图像生成
│       │   ├── knowledge.rs            # 知识库
│       │   ├── llm_wiki.rs             # LLM Wiki
│       │   ├── local_models.rs         # 本地模型
│       │   ├── mcp.rs                  # MCP 协议
│       │   ├── memory.rs              # 记忆系统
│       │   ├── message_continuation.rs  # 消息续写
│       │   ├── onboarding.rs           # 引导向导
│       │   ├── parallel_execution.rs    # 并行执行
│       │   ├── plan.rs                 # 计划管理
│       │   ├── platform_integration.rs  # 平台集成
│       │   ├── plugin.rs               # 插件管理
│       │   ├── proactive.rs            # 主动建议
│       │   ├── prompt_templates.rs      # 提示词模板
│       │   ├── providers.rs            # 模型提供商
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # 反思
│       │   ├── research.rs             # 深度研究
│       │   ├── rl.rs                   # 强化学习
│       │   ├── sandbox.rs              # 沙箱
│       │   ├── scheduled_task.rs        # 定时任务
│       │   ├── screen_vision.rs        # 屏幕视觉
│       │   ├── search.rs               # 搜索
│       │   ├── session_share.rs         # 会话分享
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # 技能分解
│       │   ├── skills_hub.rs           # 技能中心
│       │   ├── tool_recommender.rs      # 工具推荐
│       │   ├── tracer.rs               # 追踪
│       │   ├── user_profile.rs          # 用户画像
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # 工作引擎
│       │   ├── workflow_ai.rs          # AI 工作流
│       │   ├── workflow_template.rs     # 工作流模板
│       │   └── ...                     # 其他命令
│       ├── init/              # 初始化模块
│       ├── stock_scheduler.rs # 股票调度器
│       └── ...                # 其他核心模块
│
├── extension/                  # 浏览器扩展（Wiki Clipper：popup/content/background）
├── e2e/                        # Playwright E2E 测试（9 个测试套件）
├── scripts/                    # 构建与工具脚本
└── website/                    # 项目网站（VitePress，11 种语言文档）
```

## 数据目录

```
~/.axinvest/                     # 配置目录
├── axinvest.db                  # SQLite 数据库
├── master.key                   # AES-256 主密钥
├── vector_db/                   # 向量数据库 (sqlite-vec)
└── ssl/                         # SSL 证书

~/Documents/axinvest/           # 用户文件目录
├── images/                     # 图片附件
├── files/                      # 文件附件
└── backups/                    # 备份文件
```

---

## 常见问题

### macOS：提示「应用已损坏」或「无法验证开发者」

由于应用未经过 Apple 签名：

**1. 允许运行「任何来源」的应用**
```bash
sudo spctl --master-disable
```

然后前往 **系统设置 → 隐私与安全性 → 安全性**，选择 **任何来源**。

**2. 移除隔离属性**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. macOS Ventura+ 额外步骤**
前往 **系统设置 → 隐私与安全性**，点击 **仍要打开**。

---

## 社区

- [LinuxDO](https://linux.do)

## 开源协议

本项目基于 [AGPL-3.0](LICENSE) 协议开源。
