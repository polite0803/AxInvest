[**English**](./README-EN.md) | [简体中文](./README.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp&amp&utm_source=badge-featured&amp&amp;&amp;#10;&amp;amp&amp&amp;;utm_medium=badge&amp&amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - AI-Powered Intelligent Investment Analysis Platform | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>AI-Powered Intelligent Investment Analysis | Multi-Agent Collaboration | Local-First</strong>
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

## What is AxInvest?

**AxInvest v2.3** is an AI-powered intelligent investment analysis platform built on the AxAgent multi-agent framework. It deeply integrates advanced AI agent capabilities with professional A-share investment analysis, supporting multi-provider models, AI agent research, visual workflow orchestration, local knowledge management, and a built-in API gateway, covering **Windows / macOS / Linux / Android / iOS** platforms with adaptive layouts for **desktop, tablet, and mobile** devices.

The core feature of AxInvest lies in leveraging multi-agent adversarial debate, deep research, and fact-checking mechanisms to provide comprehensive and objective analytical support for investment decisions.

---

## Screenshots

| Chat & Model Selection | Multi-Agent Dashboard |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| Knowledge Base RAG | Memory & Context |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| Workflow Editor | API Gateway |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## Core Features

### 📈 Intelligent Investment Analysis

AxInvest's core feature module, deeply integrating AI agent capabilities with professional investment analysis:

**Multi-Source Data Aggregation & Failover**

- **9 Data Sources** — Tencent Finance, Tongdaxin (mootdx), East Money, Sina Finance, Baidu Stocks, THS (Tonghuashun), Iwencai, cninfo, AKShare
- **22 Data Routes** — Each data type is configured with multi-source failover routing, automatically switching to backup sources when the primary source is unavailable
- **Concurrent Data Collection** — `tokio::join!` concurrent fetching of 16 individual stock data types + 5 market data types, maximizing collection efficiency
- **Smart Caching** — LRU memory cache (1000 entry limit), 30s TTL for quotes / 300s TTL for K-line, automatic expiration and eviction
- **Health Checks** — Provider connectivity probes (Ping An Bank 000001 as probe), supporting runtime data source availability detection

**A-Share Market Identification & Rules**

- **Board Identification** — Automatic identification by code prefix: Shanghai Main Board (6), STAR Market (688), Shenzhen Main Board (0), ChiNext (3), BSE (8)
- **Price Limit Rules** — STAR Market/ChiNext ±20%, BSE ±30%, Main Board ±10%, ST Stocks ±5%
- **Trading Calendar** — Built-in 2025-2026 A-share holidays and adjusted working days, supporting trading day determination

**Individual Stock Data (16 Types)**

- **Real-Time Quotes** — Price, change percentage, volume/turnover, turnover rate, PE/PB, total market cap, limit-up/limit-down prices, ST indicator
- **K-Line Data** — 7 periods (5min/15min/30min/60min/daily/weekly/monthly), including volume, turnover, turnover rate
- **Financial Analysis** — Revenue, net profit, EPS, BPS, ROE, debt ratio, gross margin, net margin, YoY revenue growth, YoY profit growth
- **Capital Flow** — Main force / super-large / large / medium / small order net inflow
- **Dragon-Tiger List** — Brokerage branch buy/sell amounts, net amounts, listing reasons
- **Lock-up Expiry** — Expiry date, share count, ratio, shareholder information
- **Margin Trading** — Margin buy amount/balance, short sell volume/remaining volume
- **Northbound Capital** — Holding quantity, holding ratio, change in holdings
- **Industry Classification** — Shenwan Level 1/2 industries, concept sector tags
- **Shareholder Changes** — Major shareholder increase/decrease dynamics, reasons for changes
- **Dividend Records** — Ex-dividend date, dividend per share, bonus share ratio, record date
- **Research Reports** — Brokerage research reports, including institution, analyst, rating, target price, EPS forecasts
- **Consensus EPS** — Institutional consensus EPS, consensus target price, average rating, rating count
- **Concept Sectors** — Three-dimensional classification (industry/concept/region), including sector change percentages
- **Announcement Search** — cninfo listed company announcements, including announcement type and PDF links
- **News & Sentiment** — News headlines/summaries/sources, including sentiment scores

**Market Data (5 Types)**

- **Market-Wide Dragon-Tiger List** — All listed stocks for the day, including net buy, buy/sell amounts
- **Hot Stocks** — THS strong stocks, including change percentage, turnover rate, reason tags, affiliated sectors
- **Industry Rankings** — Shenwan industry change percentages, turnover, leading stocks
- **CLS Flashes** — Real-time financial news flashes, including title, content, source
- **Northbound Capital Flow** — Shanghai/Shenzhen/total minute-level capital flow

**Technical Indicator Calculation (indicators module)**

- **Moving Average System** — MA5/MA10/MA20/MA60, with alignment state detection (bullish/bearish/weak bullish/intertwined crossover)
- **MACD** — DIF/DEA/histogram, with signal detection (golden cross/death cross/bullish run/bearish run)
- **RSI** — RSI6/RSI12/RSI24, with signal detection (overbought/oversold/strong/weak/neutral)
- **Bollinger Bands** — Upper/middle/lower band (20,2), with position detection (above upper/upper zone/near middle/lower zone/below lower)
- **Bias Rate** — MA5 bias rate, MA20 bias rate
- **Volume Analysis** — Volume ratio (current day volume / 5-day average volume), with signal detection (volume up/shrinking pullback/volume down/shrinking up/normal)
- **Support/Resistance Levels** — Automatically calculated based on recent highs/lows and moving averages

**MCP Tool Registration (mcp_tools module)**

- Stock data capabilities are registered as standard tools via the MCP protocol, allowing AI agents to invoke them directly in conversations
- Registered tools: search_stock, get_stock_quote, get_stock_kline, get_stock_financials, get_stock_news, get_stock_money_flow, get_stock_dragon_tiger, etc.

**AI Analysis Pipeline (stock-analysis crate, 23 submodules)**

- **Analysis Orchestration** — orchestrator (pipeline orchestration), pipeline (multi-stage pipeline), runner (task executor)
- **Decision Engine** — decision (investment decisions), signals (trading signal generation), rules (trading rule engine)
- **Risk Assessment** — risk (risk assessment models), portfolio_risk (portfolio risk), position_limits (position limits & compliance)
- **Screener & Backtesting** — screener (multi-condition stock screener), backtest (strategy backtesting engine), trading (trading strategy framework)
- **Value Investing** — value (value analysis), value_investing (value investing evaluation framework)
- **Quality Control** — quality (data quality checks), data_clean (data cleaning & preprocessing), review (analysis result review)
- **Reports & Scoring** — report (analysis report generation), scoring (comprehensive scoring system)
- **Auxiliary Modules** — key_levels (key price level identification), monitor (real-time monitoring & alerts), plugin (analysis plugin extensions), prompts (AI prompt templates)

**Frontend Analysis Components (16)**

- StockAnalysisPage, StockQuoteCard, KLineChart, RiskMatrix, TradePanel
- DecisionBanner, DebatePanel, WatchlistPanel, PriceAlertPanel, CompareView
- AnalystReportGrid, AnalystReportCard, HistoricalAnalysisPanel, StockSearchBar
- AnalysisProgress, StockAnalysisSettingsModal, StockAnalysisChatIndicator

**Adversarial Debate & Decision**

- **Adversarial Debate** — Multi-agent Pro/Con debate with argument strength scoring and refutation tracking
- **Decision Banner** — Buy/Sell/Hold decision visualization with confidence and reasoning
- **AI Workflow Integration** — Seamless integration of stock analysis workflow with conversations (stockWorkflowChatBridge)

### 🤖 AI Model Support

- **Multi-Provider Support** — Native integration with OpenAI, Anthropic Claude, Google Gemini, Ollama, OpenClaw, Hermes and all OpenAI-compatible APIs
- **Multi-Key Rotation** — Configure multiple API keys per provider with automatic rotation to distribute rate limits
- **Local Model Inference** — Full support for Ollama local models with GGUF/GGML file management
- **Candle Inference Engine** — Built-in Candle local inference, supports rerank/judge interfaces, GGUF on-demand download
- **Model Management** — Remote model list fetching, customizable parameters (temperature, max tokens, top-p, etc.)
- **Streaming Output** — Real-time token-by-token rendering with collapsible thinking blocks (Claude extended thinking)
- **Multi-Model Comparison** — Ask the same question to multiple models simultaneously with side-by-side comparison
- **Function Calling** — Structured function calling across all supported providers
- **OpenAI Responses API** — Support for OpenAI Responses format streaming
- **Realtime API** — WebSocket event push compatible with OpenAI Realtime API
- **AI Image Generation** — AI image generation panel with multiple model and parameter configuration support

### 🔐 AI Agent System

The agent system is built on a sophisticated architecture (agent crate, 70+ source files), featuring:

- **ReAct Reasoning Engine** — Integrates reasoning and action with built-in self-verification for reliable task execution
- **Hierarchical Planner** — Decomposes complex tasks into structured plans with phases and dependencies
- **Task Decomposer** — Automatic breakdown of complex tasks into executable sub-tasks
- **Thought Chain** — Visualization of agent decision-making reasoning with step-by-step breakdown
- **Tree of Thoughts** — tree_of_thoughts multi-path reasoning exploration
- **Deep Research** — Multi-source search orchestration, citation tracking, and credibility assessment
- **Fact Checking** — AI-driven fact verification with source classification
- **Search Orchestration** — Multi-search provider coordination with search planning and result synthesis
- **Academic Search** — Academic literature retrieval and citation analysis
- **Computer Control** — AI-controlled mouse clicks, keyboard input, screen scrolling with vision model analysis
- **Screen Perception** — Screenshot capture and visual model analysis for UI element identification
- **Vision Pipeline** — vision_pipeline image understanding and analysis
- **Three Permission Levels** — Default (approval required), Accept Edits (auto-approve), Full Access (no prompts)
- **Sandbox Isolation** — Agent operations strictly confined to specified working directory
- **Tool Approval Panel** — Real-time display of tool call requests with per-item review
- **Cost Tracking** — Real-time token usage and cost statistics per session
- **Pause/Resume** — Pause agent execution anytime and resume later
- **Checkpoint System** — Persistent checkpoints for crash recovery and session resumption
- **Error Recovery Engine** — Automatic error classification, root cause analysis, and recovery strategy execution
- **Loop Detection** — Automatic detection and interruption of cyclic behavior in agent reasoning
- **Proactive Mode** — Agent can proactively offer suggestions and execute actions
- **Purpose Management** — Maintain and track agent execution purpose and context
- **Self-Verification** — self_verifier automatic verification of agent output correctness
- **Reflector** — reflector reflection and improvement on reasoning processes
- **Steering Input** — steer_manager dynamic adjustment of agent behavior direction
- **Event Bus** — event_bus / event_emitter agent event-driven architecture
- **Content Synthesis** — content_synthesizer multi-source information synthesis and report generation
- **Citation Tracking** — citation_tracker automatic tracking and annotation of information sources
- **Credibility Assessment** — credibility_evaluator evaluation of information source credibility
- **Outline Builder** — outline_builder automatic research outline construction
- **Schema Management** — schema_manager output structure schema management
- **Project Memory** — project_memory project-level persistent memory
- **Environment Probe** — environment_probe automatic runtime environment detection
- **Health Checker** — health_checker agent health status monitoring

### 👥 Multi-Agent Collaboration

- **Sub-Agent Coordination** — Master-slave architecture, coordinator coordinating multiple collaborative agents
- **Parallel Execution** — Multiple agents processing tasks in parallel with dependency-aware scheduling
- **Adversarial Debate** — adversarial_debate Pro/Con debate rounds with argument strength scoring and refutation tracking
- **Agent Roles** — agent_roles predefined roles (researcher, planner, developer, reviewer, synthesizer) for team collaboration
- **Agent Orchestrator** — Centralized message routing and state management for multi-agent teams
- **Communication Graph** — graph_insights visual representation of agent interactions and message flow
- **Shared Blackboard** — shared_blackboard / blackboard cross-agent shared state space
- **Buddy System** — Configurable agent buddies with species and attribute definitions
- **Shared Memory** — Cross-agent shared memory space with statistics and queries
- **Team Cron Registry** — Team-level scheduled task coordination
- **Expert System** — agency_expert domain expert agents
- **Agent Profile** — agent_profile agent personality and capability profile management

### ⭐ Skills System

- **Skills Marketplace** — Built-in marketplace for browsing and installing community-contributed skills
- **Skill Creation** — Auto-create skills from proposals with Markdown editor
- **Skill Evolution** — skill_evolution AI-powered automatic analysis and improvement of existing skills based on execution feedback
- **Skill Matching** — skill_matcher semantic matching to recommend relevant skills for conversation contexts
- **Skill Decomposition** — Automatic breakdown of complex tasks into executable atomic skills (LLM-assisted/multi-turn/workflow validation)
- **Generated Tools** — AI auto-generates and registers new tools to expand agent capabilities
- **Skills Hub** — skills_hub_adapter centralized skill discovery and configuration management interface
- **Skills Hub Client** — skills_hub_client integration with remote skills hub for community sharing
- **Skill Dependency Check** — Automatic detection of skill dependencies and tool availability
- **Skill Sandbox Container** — Skills execute safely in an isolated environment
- **Atomic Skill** — atomic_skill minimum executable skill unit
- **Skill Proposal** — skill_proposal AI-driven skill creation proposal

### 🔄 Workflow System

The workflow engine (rt-workflow crate) implements a DAG-based task orchestration system:

- **Visual Workflow Editor** — Drag-and-drop workflow designer with node connection and configuration
- **16 Node Types** — Trigger, Agent, LLM, Condition, Parallel, Loop, Merge, Delay, Tool, Code, SubWorkflow, VectorRetrieve, DocumentParser, Validation, End, Fallback
- **16 Property Panels** — Independent configuration panel for each node type
- **Workflow Templates** — Built-in presets: Code Review, Bug Fix, Documentation, Testing, Refactoring, Exploration, Performance, Security, Feature Development
- **DAG Execution** — Kahn's algorithm for topological sorting with cycle detection
- **Parallel Dispatch** — Pipeline-style execution where fast steps don't wait for slow ones
- **Retry Policy** — Exponential backoff with configurable max retries per step
- **Partial Completion** — Failed steps don't block independent downstream steps
- **Version Management** — Workflow template versioning with rollback support
- **Execution History** — Detailed recording with status tracking and debugging
- **AI Assistance** — AI-assisted workflow design, node recommendation, and agent prompt optimization
- **Semantic Check** — Workflow semantic validation to detect potential issues
- **n8n Import** — Support importing workflows from n8n directory
- **Debug Panel** — Real-time debugging and status viewing during workflow execution
- **Cache Layer** — cache_layer workflow execution result caching
- **Marketplace** — workflow_marketplace workflow template marketplace and review

### 📚 Knowledge & Memory

- **Knowledge Base (RAG)** — Multi-knowledgebase support with document upload, automatic parsing, chunking, and vector indexing
- **Hybrid Search** — Combines vector similarity search with BM25 full-text ranking
- **Reranking** — Cross-encoder reranking for improved retrieval precision
- **Three-Level Recall Pipeline** — Multi-level recall mechanism with AST index + vector search + FTS5
- **Self-RAG** — self_rag adaptive retrieval-augmented generation
- **Query Enhancement** — query_enhancement query rewriting and expansion
- **Knowledge Graph** — Entity relationship visualization of knowledge connections (entities, attributes, relations, flows, interfaces)
- **Wiki System** — LLM Wiki compiler and validator with knowledge graph visualization and incremental sync
- **Wiki Notes** — Bidirectional linked notes system with graph view and auto-link sync
- **Memory System** — Multi-namespace memory with manual entry or AI-powered automatic extraction
- **Closed-Loop Memory** — Integration with Honcho and Mem0 persistent memory providers
- **Memory Forgetting** — memory_forgetting time-based memory decay mechanism
- **FTS5 Full-Text Search** — Fast retrieval across conversations, files, and memories
- **Session Search** — Advanced search across all conversation sessions
- **Context Management** — Flexible attachment of files, search results, knowledge snippets, memories, tool outputs
- **Document Parser** — Multi-format document automatic parsing and content extraction
- **Incremental Indexer** — Incremental index updates for file changes
- **Text Chunker** — text_chunker intelligent text chunking strategies
- **Token Budget** — token_budget retrieval result token budget control

### 🌐 API Gateway

- **Local API Server** — Built-in OpenAI-compatible, Claude, and Gemini interface server
- **External Links** — One-click integration with Claude CLI, OpenCode with automatic API key and model sync
- **Key Management** — Generate, revoke, enable/disable access keys with descriptions
- **Usage Analytics** — Request volume and token usage by key, provider, and date
- **SSL/TLS Support** — Built-in self-signed certificates with custom certificate support
- **Request Logging** — Complete recording of all API requests and responses
- **Configuration Templates** — Pre-built templates for Claude, Codex, OpenCode, Gemini
- **Realtime API** — WebSocket event push compatible with OpenAI Realtime API
- **Platform Integrations** — Support for DingTalk, Feishu, QQ, Slack, WeChat, WhatsApp, Telegram, Discord
- **Gateway Diagnostics** — Connection diagnostics and program policy management
- **Rate Limiter** — API request rate limiting and traffic control
- **Persistent Queue** — Request persistent queue management
- **Stock API** — stock_handlers stock data dedicated API endpoints
- **SSE Push** — sse Server-Sent Events real-time event push

### 🔧 Tools & Extensions

- **MCP Protocol** — Full Model Context Protocol implementation with stdio and HTTP/WebSocket transports
- **OAuth Authentication** — OAuth flow support for MCP servers
- **MCP Autostart** — MCP server auto-start and lifecycle management
- **MCP Tool Bridge** — Bridge between MCP tools and agent tool system
- **MCP Health Check** — mcp_health MCP server health status monitoring
- **Plugin System** — OpenClaw-compatible three-tier plugin architecture (builtin/bundled/external) with npm package installation, tool registration, hooks, and lifecycle management
- **Plugin Marketplace** — Built-in marketplace UI with npm search, install, and confirmation dialogs
- **Built-in Tools** — 40+ tool modules: file operations (read/write/edit/system), code execution, search (Grep/Glob), Bash, Web search/fetch, plan management, Cron scheduling, REPL, LSP, context management, computer control, messaging, todo items, database, DevOps, document parsing, Git, knowledge retrieval, LSP, media processing, messaging, OCR, push notifications, system info, task system, testing, workspace/worktrees, etc.
- **Tool Permission System** — Tool permission classification, rule management, and usage tracking
- **Bash Security** — Command parsing, path validation, and sandbox security controls
- **LSP Client** — Built-in Language Server Protocol for code completion and diagnostics
- **AST Index** — Code file AST parsing and index building
- **Terminal Backends** — Support for Local, Docker, and SSH terminal connections
- **Browser Automation** — Integrated browser control via CDP (navigation, screenshots, clicks, form filling, text extraction, etc.)
- **UI Automation** — Cross-platform UI element identification and control
- **Git Tools** — Git operations with branch detection and conflict awareness
- **Tool Recommendation** — Context-aware intelligent tool recommendation engine
- **Tool Orchestration** — Multi-tool coordinated execution with streaming output
- **Tool Stats** — Tool usage frequency and performance statistics
- **Tool Audit** — audit tool call audit logging

### 📊 Content Rendering

- **Markdown Rendering** — Full support for code highlighting, LaTeX math, tables, task lists
- **Monaco Code Editor** — Embedded editor with syntax highlighting, copy, diff preview
- **Diagram Rendering** — Mermaid flowcharts, D2 architecture diagrams, ECharts interactive charts
- **Artifact Panel** — Code snippets, HTML drafts, React components, Markdown notes with live preview
- **Four Preview Modes** — Code (editor), Split (side-by-side), Preview (rendered only), React component preview
- **Session Inspector** — Tree view of session structure for quick navigation
- **Citation Panel** — Track and display source citations with credibility scoring
- **Infographic Rendering** — Support for infographic visualization display
- **Chart Interpreter** — ChartInterpreter AI-powered chart interpretation
- **Diff Viewer** — DiffViewer code diff comparison

### 🛡️ Data & Security

- **AES-256 Encryption** — API keys and sensitive data encrypted with AES-256-GCM
- **Isolated Storage** — Application state in `~/.axinvest/`, user files in `~/Documents/axinvest/`
- **Auto Backup** — Scheduled backups to local directories or WebDAV storage
- **S3 Backup** — s3_backup Amazon S3 cloud backup support
- **Backup Restore** — One-click restore from historical backups
- **Export Options** — PNG screenshots, Markdown, plain text, JSON formats
- **Storage Management** — Visual disk usage display with cleanup tools
- **Storage Migration** — storage_migration version-to-version data migration
- **File Authorization** — File access authorization and revocation management
- **Operation Audit** — Audit logging for critical operations
- **Command Validator** — command_validator command security validation
- **Resource Limits** — resource_limits resource usage limits
- **Sandbox Runner** — sandbox_runner isolated environment execution

### 🖥️ Desktop Experience

- **Theme Engine** — Dark/light themes with system-follow or manual preference
- **Interface Languages** — 11 languages: Simplified Chinese, Traditional Chinese, English, Japanese, Korean, French, German, Spanish, Russian, Hindi, Arabic
- **System Tray** — Minimize to tray without interrupting background services
- **Always on Top** — Pin window above others
- **Global Shortcuts** — Customizable shortcuts to summon main window
- **QuickBar** — Quick access floating bar, one-click summon
- **Auto Start** — Optional launch on system startup
- **Proxy Support** — HTTP and SOCKS5 proxy configuration
- **Auto Update** — Automatic version checking with update prompts
- **Command Palette** — `Cmd/Ctrl+K` for quick command access
- **Onboarding Wizard** — Interactive first-use guide with Ollama detection
- **Notification Center** — Unified in-app notification management
- **Cloud Workspace** — cloud_workspace cloud workspace selection
- **Crash Report** — crash_report automatic crash report collection
- **Voice Call** — VoiceCall voice conversation capability

### 🔬 Advanced Features

- **Deep Research** — Multi-source search, citation tracking, credibility assessment, and content synthesis
- **Fact Checking** — AI-driven fact verification with source classification
- **Cron Scheduler** — Automated task scheduling with daily/weekly/monthly templates and custom cron expressions
- **Webhook System** — Event subscriptions for tool completion, agent errors, session end notifications
- **User Profiling** — Automatic learning of coding style, naming conventions, indentation, comment style, communication preferences
- **RL Optimizer** — Reinforcement learning for tool selection and task strategy optimization
- **LoRA Fine-Tuning** — Custom model adaptation with local training using LoRA
- **Proactive Suggestions** — Context-aware nudges based on conversation content and user patterns
- **Context Prediction** — Predict user's next action and prefetch relevant resources
- **Dream Consolidation** — dream_consolidation background auto-consolidation of memories and patterns for long-term knowledge optimization
- **Error Recovery** — Automatic error classification, root cause analysis, and recovery suggestions
- **DevTools** — Trace, span, timeline visualization for debugging and performance analysis
- **Benchmark System** — SWE-bench / Terminal-bench task performance evaluation and metrics with score cards
- **Style Transfer** — style_migrator apply learned coding style preferences to generated code
- **Dashboard Plugins** — Extensible dashboard with custom panels and widgets
- **Collaboration** — CRDT real-time collaboration and one-click session sharing
- **Browser Extension** — Wiki Clipper browser extension for quick web clipping to LLM Wiki
- **Python SDK** — Python SDK for integration with AxInvest
- **Smart Router** — Intelligent request routing and classification
- **Semantic Cache** — Semantic-based response caching to reduce redundant computation
- **Context Compression** — Automatic compression of long contexts to optimize token usage
- **Message Batching** — Message batch sending and optimization
- **Connection Pool** — Database and API connection pool management
- **Feature Flags** — Configurable feature flag system
- **Policy Engine** — Centralized management of permission and operation policies
- **Resource Governor** — Agent resource usage limits and governance
- **LAN Transfer** — Local area network file transfer capability
- **Coevolution** — coevolution skill and agent co-evolution
- **Behavior Learning** — behavior_learner / behavior_tracker user behavior learning and tracking
- **Preference Learning** — preference_learner automatic user preference learning
- **Intrinsic Reward** — intrinsic_reward intrinsic motivation-driven exploration
- **Process Reward** — process_reward process-level reward signals
- **TextGrad** — text_grad text gradient-based automatic optimization
- **Trajectory Compression** — trajectory_compressor long trajectory automatic compression
- **Reminder Management** — reminder_manager smart reminder scheduling
- **Task Prefetch** — task_prefetcher predictive task resource prefetching

### 🛡️ Prompt Injection Protection (Prompt-Guard)

- **Four-Level Protection** — L1 pattern detection (high-risk block + medium-risk tag) → L2 delimiter escaping → L3 XML wrapper → L4 trust labels
- **Pipeline Orchestrator** — Multi-level detection pipeline chaining with customizable risk thresholds
- **Token Smuggling Detection** — Specialized detection for encoding obfuscation and token smuggling attacks
- **Delimiter Escape Detection** — delimiter_escape detection of prompt delimiter escape attacks
- **Pattern Detection** — pattern_detect regex + heuristic injection pattern matching
- **Trust Labels** — trust_labels trusted content marking and verification
- **Strict Mode** — Strict mode testing + medium-risk reason naming + custom mode documentation
- **Full Pipeline Integration** — Integrated into session / prompt / git / RAG workflows

### ⏰ Time Travel / As-Of Mode

> NEW 2026-06-08 — closes the analysis → recommendation → backtest look-ahead bias loop

- **Global Time Anchor** — A `LIVE` pill in the AppHeader switches the analysis world-view across three modes: Live / Replay / Backtest Sweep
- **Closed-World Assumption** — once a past date is picked, data after that date is invisible to the current analysis; all picks and backtests auto-anchor to as-of
- **9 Vendors Fully Adapted** — EastMoney / Tencent / Sina / Baidu / AkShare / THS / Cninfo / iwencai / mootdx, every one aware of AsOfContext
- **Two-Tier Cache Isolation** — L1 in-memory cache + L2 `market_data_history` table (with hash / TTL / access count); Live and Replay data never mix
- **3-Stage LLM Future-Reference Detection** — regex absolute date → tense phrase dictionary → optional LLM judge; on hit, sets `partial-valid: false`
- **HCI 4-Layer Visual Signals** — L1 header pill / L2 page-state bar / L3 timeline "⚠ N violation" chip / L4 data watermark
- **Replay Workbench Forced Reselect** — `/replay-workbench` route, picker is always blank on entry
- **First-Time Tour Bubble** — introduces the new capability, persisted via `tourSeen` so it never nags again
- **Switch-Back-to-Live Confirm Modal** — prevents accidentally dragging invalidated conclusions back into Live
- **Replay Visualization** — `ReplayBadge` shows "Replay · 2026-06-01"; `ReplayWatermark` stamps "as of ..." in the corner of panels

### 📱 Mobile Support

- **Android Native** — APK/AAB builds, supporting arm64-v8a / armeabi-v7a / x86_64
- **iOS Native** — IPA builds, supporting arm64
- **Adaptive Layout** — Desktop/tablet/mobile three-tier auto-adaptation (useResponsive hook)
- **Mobile Navigation** — Drawer slide-out navigation + bottom navigation bar + flash floating action button
- **Safe Area Adaptation** — Android system status bar/navigation bar CSS env() adaptation
- **CSP Optimization** — Android WebView CSP protocol whitelist
- **Conditional Compilation** — `#[cfg(not(mobile))]` desktop-only features (browser, computer control, desktop, QuickBar, terminal, screen vision) automatically excluded

---

## Technical Architecture

### Tech Stack

| Layer | Technology |
|-------|------------|
| **Framework** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **State Management** | Zustand 5 |
| **Routing** | React Router 7 |
| **i18n** | i18next + react-i18next |
| **Backend** | Rust 2024 + SeaORM 2 + SQLite |
| **Vector DB** | sqlite-vec |
| **Code Editor** | Monaco Editor |
| **Diagrams** | Mermaid + D2 + ECharts (CDN) |
| **Terminal** | xterm.js 6 |
| **Workflow** | ReactFlow 11 |
| **Chart Rendering** | @antv/infographic |
| **Icons** | Iconify + Lucide |
| **Drag & Drop** | @dnd-kit |
| **Build** | Vite 8 + npm |
| **Testing** | Vitest + Playwright + cargo-nextest |
| **Formatting** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **Mobile** | Tauri Android + iOS native builds |
| **Desktop** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### Platform Support

| Platform | Architectures |
|----------|---------------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (emulator) |
| iOS | arm64 |

### Rust Backend Architecture

The backend is organized as a Rust workspace with **20** specialized crates:

```
src-tauri/crates/
├── agent/            # AI Agent core (70+ source files: ReAct engine, coordination, planning, deep research, fact-checking, etc.)
├── astock-data/      # A-share data sources (9 data sources, 22 data routes, technical indicators, trading calendar, MCP tool registration)
├── core/             # Core utilities (85+ database entities, 40+ repositories, RAG, crypto, MCP, browser automation, AST index, etc.)
├── gateway/          # API Gateway (HTTP server, auth, routing, OpenAI-compatible interface, stock API endpoints)
├── migration/        # Database migrations (5 migrations: stock analysis/watchlist/analysis scheduling/price alerts/trading)
├── npm/              # npm package parsing & registry
├── plugins/          # Plugin system (OpenClaw-compatible, npm package installation, with example plugins)
├── prompt-guard/     # Prompt injection protection (L1-L4 multi-level detection & defense, 4 detectors)
├── providers/        # Model provider adapters (OpenAI, Anthropic, Gemini, Ollama, OpenClaw, Hermes, image generation)
├── rt-dashboard/     # Dashboard plugin system
├── rt-messaging/     # Messaging gateway (9 platforms: DingTalk/Feishu/QQ/Slack/WeChat/WhatsApp/Telegram/Discord)
├── rt-theme/         # Theme engine
├── rt-webhook/       # Webhook server & dispatch
├── rt-workflow/      # Workflow engine (DAG orchestration, 16 node executors, scheduler, cache layer)
├── runtime/          # Runtime services (70+ source files: session management, MCP, terminal, rate limiting, webhooks, permissions, benchmarking, etc.)
├── runtime-core/     # Runtime abstraction layer (common types, trait definitions, configuration, feature flags, permission enforcer)
├── stock-analysis/   # Intelligent investment analysis (23 submodules: pipeline, decision engine, risk assessment, backtesting, screener, value investing)
├── telemetry/        # Telemetry & distributed tracing (OpenTelemetry compatible)
├── tools/            # Tool system (40+ built-in tools, Bash security, MCP bridging, permission system, orchestration, audit)
└── trajectory/       # Learning system (55+ source files: memory, skills, RL, user profiling, dream consolidation, style transfer, coevolution)
```

#### stock-analysis crate module structure (23 submodules)

```
stock-analysis/
├── backtest.rs         # Strategy backtesting engine
├── data_clean.rs       # Data cleaning & preprocessing
├── decision.rs         # Investment decision engine
├── key_levels.rs       # Key price level identification
├── monitor.rs          # Real-time monitoring & alerts
├── orchestrator.rs     # Analysis pipeline orchestration
├── pipeline.rs         # Multi-stage analysis pipeline
├── plugin.rs           # Analysis plugin extensions
├── portfolio_risk.rs   # Portfolio risk assessment
├── position_limits.rs  # Position limits & compliance
├── prompts.rs          # AI prompt templates
├── quality.rs          # Data quality checks
├── report.rs           # Analysis report generation
├── review.rs           # Analysis result review
├── risk.rs             # Risk assessment models
├── rules.rs            # Trading rule engine
├── runner.rs           # Analysis task executor
├── scoring.rs          # Comprehensive scoring system
├── screener.rs         # Stock screener
├── signals.rs          # Trading signal generation
├── trading.rs          # Trading strategy framework
├── value.rs            # Value analysis
└── value_investing.rs  # Value investing evaluation
```

#### astock-data crate data sources

| Data Source | Identifier | Supported Data Types |
|-------------|-----------|---------------------|
| Tencent Finance | tencent | Real-time quotes, K-line |
| Tongdaxin (mootdx) | mootdx | Real-time quotes, K-line |
| East Money | eastmoney | Quotes, K-line, financials, capital flow, dragon-tiger list, lock-up expiry, margin trading, northbound capital, industry classification, shareholder changes, dividends, research reports, market-wide dragon-tiger list, CLS flashes |
| Sina Finance | sina | Quotes, K-line, news |
| Baidu Stocks | baidu_stock | Quotes, news, capital flow, dragon-tiger list, lock-up expiry, margin trading, northbound capital, industry classification, shareholder changes, dividends, research reports, hot stocks, industry rankings, concept sectors, northbound capital flow |
| THS (Tonghuashun) | ths | Quotes, industry classification, consensus EPS, concept sectors, hot stocks, industry rankings, northbound capital flow |
| Iwencai | iwencai | Stock search, industry classification, consensus EPS, concept sectors, hot stocks |
| cninfo | cninfo | Announcements |
| AKShare | akshare | Financials, news, consensus EPS, CLS flashes |

Each data type is configured with multi-source failover routing, automatically switching to backup sources when the primary source is unavailable.

#### astock-data additional modules

| Module | Function |
|--------|----------|
| calendar | A-share trading calendar (2025-2026 holidays + adjusted working days) |
| indicators | Technical indicator calculation (MA/MACD/RSI/Bollinger Bands/Bias/Volume Ratio/Support & Resistance) |
| mcp_tools | MCP tool registration (stock data capabilities registered as AI-callable tools) |

### Frontend Architecture

```
src/
├── stores/                    # Zustand state management (65 stores)
│   ├── domain/               # Core business state (9 stores)
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # Feature module state (46 stores)
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
│   ├── devtools/              # DevTools state (5 stores)
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # Shared state (5 stores)
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # React components (25 modules)
│   ├── chat/                # Chat interface (100+ components: Agent execution panel, branch comparison, browser automation, code executor, collaboration panel, deep research, fact-checking, Git commit, image generation/analysis, knowledge retrieval, memory extraction, model routing, multi-model display, permission management, plugin marketplace, reflection panel, skill creation/evolution, structured thinking, sub-agent cards, tool call cards, trajectory replay, voice call, Wiki retrieval, workflow progress, etc.)
│   ├── stock-analysis/      # Intelligent investment analysis (16 components)
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
│   ├── workflow/            # Workflow editor (16 node types + 16 property panels + AI panel + templates + debugging)
│   ├── gateway/             # API gateway UI (overview/keys/metrics/monitoring/settings/templates/diagnostics)
│   ├── settings/            # Settings panels (50+ components: providers/models/MCP/knowledge/memory/proxy/shortcuts/theme/tools/webhooks/cron/stock analysis config, etc.)
│   ├── terminal/            # Terminal UI (integrated terminal/Docker/SSH/backend selection/path completion/slash completion)
│   ├── skill/               # Skill editor & renderer (action chain editing/frontend editor/sandbox container/dependency check/stats panel)
│   ├── benchmark/           # Benchmark panels (configuration/reports/selector/task list/results)
│   ├── files/               # File management page
│   ├── fine-tune/           # LoRA fine-tuning config (dataset/training tasks/LoRA configuration)
│   ├── link/                # External link management (overview/models/strategy/skills/strategy details)
│   ├── llm-wiki/            # LLM Wiki editor (quality scoring/sync status)
│   ├── proactive/           # Proactive suggestion system (context prediction/prefetch indicator/suggestion bar/reminder list)
│   ├── wiki/                # Wiki management (backlinks/graph view/ingestion/linting/action timeline/tag aggregation/version history)
│   ├── devtools/            # Trace/Span timeline (cost chart/duration chart/details/filters/list)
│   ├── decomposition/       # Skill decomposition (decomposition preview/tool dependencies/tool generation/tool installation)
│   ├── recommendation/      # Tool recommendation panel
│   ├── style/               # Code style transfer (samples/adjustment sliders/comparison/preview panel)
│   ├── layout/              # Layout components (titlebar/sidebar/command palette/global copy/error boundary/status bar/notification bell/user profile modal)
│   ├── help/                # Help panel
│   ├── notification/        # Notification center
│   ├── search/              # Session search
│   ├── onboarding/          # Onboarding wizard (interactive tutorial/welcome wizard)
│   ├── common/              # Common components (copy/icons/model parameter sliders/paste)
│   └── shared/              # Shared components (avatar editing/modals/chart rendering/dynamic icons/embedding model selection/emoji selection/knowledge base icon/MCP icon/model selection/Monaco editor/namespace icon/search provider icon)
│
├── pages/                    # Page components (22 pages)
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
├── hooks/                    # React hooks (12)
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
├── lib/                      # Utility functions (33 modules + Web Worker)
│   ├── workers/            # Web Worker (heavy.worker.ts)
│   ├── actionRouter.ts     # Action routing
│   ├── artifactRenderer.ts # Artifact rendering
│   ├── chartGenerator.ts   # Chart generation
│   ├── chatMarkdown.ts     # Markdown rendering
│   ├── codeExecutor.ts     # Code execution
│   ├── invoke.ts           # Tauri IPC wrapper
│   ├── skillActionExecutor.ts  # Skill action execution
│   ├── skillEventBus.ts    # Skill event bus
│   ├── skillLifecycle.ts   # Skill lifecycle
│   ├── skillPermissions.ts # Skill permissions
│   ├── storeRegistry.ts    # Store registry
│   ├── tokenEstimator.ts   # Token estimation
│   ├── workflowLayout.ts   # Workflow layout
│   └── ...                 # Other utility modules
│
├── types/                    # TypeScript type definitions (22)
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
├── sdk/                      # SDK (including Python SDK)
│   ├── index.ts
│   ├── types.ts
│   ├── rpcBridge.ts
│   ├── sandboxTemplate.ts
│   └── python/              # Python SDK
│       ├── setup.py
│       └── axagent_sdk/__init__.py
│
└── i18n/                     # 11 language translations
```

## Getting Started

### Download Pre-built

Visit the [Releases](https://github.com/polite0803/AxAgent/releases) page and download the installer for your platform.

### Build from Source

#### Prerequisites

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + Rust MSVC targets

#### Build Steps

```bash
# Clone repository
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# Install dependencies
npm install

# Development mode
npm run tauri dev

# Build frontend only
npm run build

# Build desktop application
npm run tauri build
```

Build artifacts are located in `src-tauri/target/release/`.

### Testing

```bash
# Unit tests
npm run test          # Vitest watch
npm run test:run      # Vitest single run

# E2E tests
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright UI mode

# Rust backend tests
cd src-tauri && cargo nextest run   # cargo-nextest (2-3x faster)
cd src-tauri && cargo test          # Standard tests

# Type checking
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# Code formatting
npm run format        # dprint
cd src-tauri && cargo fmt

# CI full check
npm run ci:check
```

---

## Project Structure

```
AxInvest/
├── src/                         # Frontend source (React + TypeScript)
│   ├── components/              # React components (25 modules)
│   │   ├── chat/               # Chat interface (100+ components)
│   │   ├── stock-analysis/     # Intelligent investment analysis (16 components)
│   │   ├── workflow/           # Workflow editor (16 node types + property panels + AI panel)
│   │   ├── gateway/            # API gateway components
│   │   ├── settings/           # Settings panels (50+ components)
│   │   ├── terminal/           # Terminal components
│   │   ├── skill/              # Skill editor & renderer
│   │   ├── benchmark/          # Benchmark
│   │   ├── files/              # File management
│   │   ├── fine-tune/          # LoRA fine-tuning
│   │   ├── link/               # External links
│   │   ├── llm-wiki/           # LLM Wiki
│   │   ├── proactive/          # Proactive suggestions
│   │   ├── wiki/               # Wiki management
│   │   ├── devtools/           # DevTools
│   │   ├── decomposition/      # Skill decomposition
│   │   ├── recommendation/     # Tool recommendation
│   │   ├── style/              # Code style
│   │   ├── layout/             # Layout components
│   │   ├── help/               # Help panel
│   │   ├── notification/       # Notification center
│   │   ├── search/             # Session search
│   │   ├── onboarding/         # Onboarding wizard
│   │   ├── common/             # Common components
│   │   └── shared/             # Shared components
│   ├── pages/                   # Page components (22 pages)
│   ├── stores/                  # Zustand state management (65 stores)
│   │   ├── domain/            # Core business state (9 stores)
│   │   ├── feature/           # Feature module state (46 stores)
│   │   ├── devtools/          # DevTools state (5 stores)
│   │   └── shared/            # Shared state (5 stores)
│   ├── hooks/                   # React hooks (12)
│   ├── lib/                     # Utility functions (33 modules + Web Worker)
│   ├── types/                   # TypeScript type definitions (22)
│   ├── sdk/                     # SDK (TypeScript + Python)
│   └── i18n/                    # 11 language translations
│
├── src-tauri/                    # Backend source (Rust)
│   ├── crates/                  # Rust workspace (20 crates)
│   │   ├── agent/             # AI Agent core (70+ source files)
│   │   ├── astock-data/       # A-share data sources (9 data sources, 22 data routes, technical indicators, trading calendar)
│   │   ├── core/              # Core utilities (85+ entities, 40+ repositories, RAG, crypto, MCP)
│   │   ├── gateway/           # API Gateway (including stock API endpoints)
│   │   ├── migration/         # Database migrations (5 migrations)
│   │   ├── npm/               # npm package parsing
│   │   ├── plugins/           # Plugin system
│   │   ├── prompt-guard/      # Prompt injection protection
│   │   ├── providers/         # Model provider adapters
│   │   ├── rt-dashboard/      # Dashboard plugins
│   │   ├── rt-messaging/      # Messaging gateway (9 platforms)
│   │   ├── rt-theme/          # Theme engine
│   │   ├── rt-webhook/        # Webhook server
│   │   ├── rt-workflow/       # Workflow engine (16 node executors)
│   │   ├── runtime/           # Runtime services (70+ source files)
│   │   ├── runtime-core/      # Runtime abstraction layer
│   │   ├── stock-analysis/    # Intelligent investment analysis (23 submodules)
│   │   ├── telemetry/         # Tracing & metrics
│   │   ├── tools/             # Tool system (40+ built-in tools)
│   │   └── trajectory/        # Learning system (55+ source files)
│   └── src/                    # Tauri entry point (91 command modules)
│       ├── commands/          # Command modules
│       │   ├── stock_analysis.rs        # Stock analysis commands
│       │   ├── stock_analysis_setup.rs  # Stock analysis configuration
│       │   ├── stock_workflow.rs        # Stock workflow commands
│       │   ├── agency_expert.rs         # Expert agent
│       │   ├── agent_advanced.rs        # Advanced agent
│       │   ├── agent_analytics.rs       # Agent analytics
│       │   ├── agent_insight.rs         # Agent insights
│       │   ├── agent_nudge.rs           # Agent nudge
│       │   ├── agent_profile.rs         # Agent profile
│       │   ├── agent_role.rs            # Agent roles
│       │   ├── background_tasks.rs      # Background tasks
│       │   ├── browser.rs              # Browser automation
│       │   ├── chart_generator.rs       # Chart generation
│       │   ├── cloud_workspace.rs       # Cloud workspace
│       │   ├── computer_control.rs      # Computer control
│       │   ├── context_breakdown.rs     # Context breakdown
│       │   ├── conversation_categories.rs  # Conversation categories
│       │   ├── conversations_search.rs  # Conversation search
│       │   ├── crash_report.rs          # Crash report
│       │   ├── dream.rs                # Dream consolidation
│       │   ├── evolution.rs            # Skill evolution
│       │   ├── fine_tune.rs            # LoRA fine-tuning
│       │   ├── gateway.rs              # API gateway
│       │   ├── gateway_link.rs         # External links
│       │   ├── generated_tool.rs        # Generated tools
│       │   ├── image_gen.rs            # Image generation
│       │   ├── knowledge.rs            # Knowledge base
│       │   ├── llm_wiki.rs             # LLM Wiki
│       │   ├── local_models.rs         # Local models
│       │   ├── mcp.rs                  # MCP protocol
│       │   ├── memory.rs              # Memory system
│       │   ├── message_continuation.rs  # Message continuation
│       │   ├── onboarding.rs           # Onboarding wizard
│       │   ├── parallel_execution.rs    # Parallel execution
│       │   ├── plan.rs                 # Plan management
│       │   ├── platform_integration.rs  # Platform integration
│       │   ├── plugin.rs               # Plugin management
│       │   ├── proactive.rs            # Proactive suggestions
│       │   ├── prompt_templates.rs      # Prompt templates
│       │   ├── providers.rs            # Model providers
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # Reflection
│       │   ├── research.rs             # Deep research
│       │   ├── rl.rs                   # Reinforcement learning
│       │   ├── sandbox.rs              # Sandbox
│       │   ├── scheduled_task.rs        # Scheduled tasks
│       │   ├── screen_vision.rs        # Screen vision
│       │   ├── search.rs               # Search
│       │   ├── session_share.rs         # Session sharing
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # Skill decomposition
│       │   ├── skills_hub.rs           # Skills hub
│       │   ├── tool_recommender.rs      # Tool recommender
│       │   ├── tracer.rs               # Tracing
│       │   ├── user_profile.rs          # User profile
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # Work engine
│       │   ├── workflow_ai.rs          # AI workflow
│       │   ├── workflow_template.rs     # Workflow templates
│       │   └── ...                     # Other commands
│       ├── init/              # Initialization modules
│       ├── stock_scheduler.rs # Stock scheduler
│       └── ...                # Other core modules
│
├── extension/                  # Browser extension (Wiki Clipper: popup/content/background)
├── e2e/                        # Playwright E2E tests (9 test suites)
├── scripts/                    # Build & utility scripts
└── website/                    # Project website (VitePress, 11-language docs)
```

## Data Directories

```
~/.axinvest/                     # Configuration directory
├── axinvest.db                  # SQLite database
├── master.key                   # AES-256 master key
├── vector_db/                   # Vector database (sqlite-vec)
└── ssl/                         # SSL certificates

~/Documents/axinvest/            # User files directory
├── images/                      # Image attachments
├── files/                       # File attachments
└── backups/                     # Backup files
```

---

## FAQ

### macOS: "App Is Damaged" or "Cannot Verify Developer"

Since the application is not signed by Apple:

**1. Allow apps from "Anywhere"**
```bash
sudo spctl --master-disable
```

Then go to **System Settings → Privacy & Security → Security** and select **Anywhere**.

**2. Remove quarantine attribute**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. macOS Ventura+ additional step**
Go to **System Settings → Privacy & Security**, click **Open Anyway**.

---

## Community

- [LinuxDO](https://linux.do)

## License

This project is licensed under the [AGPL-3.0](LICENSE) License.
