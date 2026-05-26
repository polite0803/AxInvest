[**English**](./README-EN.md) | [简体中文](./README.md) | **繁體中文** | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp;amp&utm_source=badge-featured&amp;amp;&amp;#10;&amp;amp&amp;amp;;utm_medium=badge&amp;amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - AI 驅動的智慧投資分析平台 | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>AI 驅動的智慧投資分析 | 多智慧體協作 | 本地優先</strong>
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

## 什麼是 AxInvest？

**AxInvest v2.3** 是一款 AI 驅動的智慧投資分析平台，基於 AxAgent 多智慧體框架構建。它將先進的 AI 智慧體能力與專業的 A 股投資分析深度融合，支援多模型提供商、AI 智慧體研究、可視化工作流編排、本地知識管理、內建 API 網關，覆蓋 **Windows / macOS / Linux / Android / iOS** 五大平台，並針對**桌面、平板、手機**三檔裝置自適應佈局。

AxInvest 的核心特色在於利用多智慧體對抗辯論、深度研究和事實核查等機制，為投資決策提供全面、客觀的分析支援。

---

## 截圖預覽

| 對話與模型選擇 | 多智慧體儀表盤 |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| 知識庫 RAG | 記憶與上下文 |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| 工作流編輯器 | API 網關 |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## 核心功能

### 📈 智慧投資分析

AxInvest 的核心特色模組，將 AI 智慧體能力與專業投資分析深度融合：

**多源資料聚合與降級**

- **9 大資料來源** — 騰訊財經、通達信 (mootdx)、東方財富、新浪財經、百度股票、同花順 (THS)、問財 (Iwencai)、巨潮資訊 (cninfo)、AKShare
- **22 種資料路由** — 每種資料類型配置多源降級路由，主源不可用時自動切換至備用源
- **並行資料採集** — `tokio::join!` 並行拉取 16 種個股資料 + 5 種市場資料，最大化採集效率
- **智慧快取** — LRU 記憶體快取（1000 條上限），行情 30s TTL / K 線 300s TTL，自動過期淘汰
- **健康檢查** — 供應商連通性探針（平安銀行 000001 做探針），支援執行時檢測資料來源可用性

**A 股市場識別與規則**

- **板塊識別** — 根據代碼前綴自動識別：滬主板(6)、科創板(688)、深主板(0)、創業板(3)、北交所(8)
- **漲跌停規則** — 科創板/創業板 ±20%、北交所 ±30%、主板 ±10%、ST 股 ±5%
- **交易日曆** — 內建 2025-2026 年 A 股節假日和調休工作日，支援交易日判斷

**個股資料（16 類）**

- **即時行情** — 價格、漲跌幅、成交量/額、換手率、PE/PB、總市值、漲停價/跌停價、ST 標識
- **K 線資料** — 7 種週期（5 分/15 分/30 分/60 分/日/週/月），含成交量、成交額、換手率
- **財務分析** — 營收、淨利潤、EPS、BPS、ROE、負債率、毛利率、淨利率、營收年增率、利潤年增率
- **資金流向** — 主力/超大單/大單/中單/小單淨流入
- **龍虎榜** — 營業部買賣金額、淨額、上榜原因
- **限售解禁** — 解禁日期、解禁股數、解禁比例、股東資訊
- **融資融券** — 融資買入額/餘額、融券賣出量/餘量
- **北向資金** — 持股數量、持股佔比、變動數量
- **行業分類** — 申萬一級/二級行業、概念板塊標籤
- **股東增減持** — 重要股東增減持動態、增減持原因
- **分紅記錄** — 除權除息日、每股分紅、送轉比例、股權登記日
- **研報聚合** — 券商研究報告，含機構、分析師、評級、目標價、EPS 預測
- **一致預期 EPS** — 機構一致預期 EPS、一致目標價、平均評級、評級數量
- **概念板塊** — 三維歸屬（行業/概念/地域），含板塊漲跌幅
- **公告檢索** — 巨潮資訊上市公司公告，含公告類型和 PDF 連結
- **新聞輿情** — 新聞標題/摘要/來源，含情緒評分

**市場資料（5 類）**

- **全市場龍虎榜** — 當日所有上榜股票，含淨買入、買賣金額
- **熱門股票** — 同花順強勢股，含漲跌幅、換手率、原因標籤、所屬板塊
- **行業排名** — 申萬行業漲跌幅、成交額、領漲股
- **財聯社快訊** — 即時財經快訊，含標題、內容、來源
- **北向資金流向** — 滬/深/合計分鐘級資金流向

**技術指標計算（indicators 模組）**

- **均線系統** — MA5/MA10/MA20/MA60，含排列狀態判斷（多頭/空頭/弱多頭/纏繞交叉）
- **MACD** — DIF/DEA/柱狀圖，含訊號判斷（金叉/死叉/多頭執行/空頭執行）
- **RSI** — RSI6/RSI12/RSI24，含訊號判斷（超買/超賣/強勢/弱勢/中性）
- **布林帶** — 上軌/中軌/下軌 (20,2)，含位置判斷（上軌以上/上軌區間/中軌附近/下軌區間/下軌以下）
- **乖離率** — MA5 乖離率、MA20 乖離率
- **量能分析** — 量比（當日量/5 日均量），含訊號判斷（放量上漲/縮量回呼/放量下跌/縮量上漲/正常）
- **支撐/壓力位** — 基於近期高低點和均線自動計算

**MCP 工具註冊（mcp_tools 模組）**

- 股票資料能力透過 MCP 協定註冊為標準工具，AI 智慧體可在對話中直接呼叫
- 註冊工具：search_stock、get_stock_quote、get_stock_kline、get_stock_financials、get_stock_news、get_stock_money_flow、get_stock_dragon_tiger 等

**AI 分析流水線（stock-analysis crate，23 個子模組）**

- **分析編排** — orchestrator（流水線編排）、pipeline（多階段管道）、runner（任務執行器）
- **決策引擎** — decision（投資決策）、signals（交易訊號生成）、rules（交易規則引擎）
- **風險評估** — risk（風險評估模型）、portfolio_risk（組合風險）、position_limits（倉位限制與合規）
- **選股與回測** — screener（多條件選股器）、backtest（策略回測引擎）、trading（交易策略框架）
- **價值投資** — value（價值分析）、value_investing（價值投資評估框架）
- **品質控制** — quality（資料品質檢查）、data_clean（資料清洗與預處理）、review（分析結果複核）
- **報告與評分** — report（分析報告生成）、scoring（綜合評分系統）
- **輔助模組** — key_levels（關鍵價位識別）、monitor（即時監控與預警）、plugin（分析外掛擴展）、prompts（AI 提示詞範本）

**前端分析元件（16 個）**

- StockAnalysisPage、StockQuoteCard、KLineChart、RiskMatrix、TradePanel
- DecisionBanner、DebatePanel、WatchlistPanel、PriceAlertPanel、CompareView
- AnalystReportGrid、AnalystReportCard、HistoricalAnalysisPanel、StockSearchBar
- AnalysisProgress、StockAnalysisSettingsModal、StockAnalysisChatIndicator

**對抗辯論與決策**

- **對抗辯論** — 多智慧體 Pro/Con 辯論，支援論點強度評分和反駁追蹤
- **決策橫幅** — 買入/賣出/持有決策可視化，含置信度和理由
- **AI 工作流整合** — 股票分析工作流與對話無縫銜接（stockWorkflowChatBridge）

### 🤖 AI 模型支援

- **多提供商支援** — 原生整合 OpenAI、Anthropic Claude、Google Gemini、Ollama、OpenClaw、Hermes 及所有 OpenAI 相容 API
- **多 Key 輪換** — 為每個提供商配置多個 API Key，自動輪換分發限流
- **本地模型推理** — 完整支援 Ollama 本地模型，包含 GGUF/GGML 檔案管理
- **Candle 推理引擎** — 內建 Candle 本地推理，支援 rerank/judge 介面，GGUF 按需下載
- **模型管理** — 遠端模型列表獲取，可自訂參數（temperature、max tokens、top-p 等）
- **串流輸出** — 即時逐 Token 渲染，支援可折疊的思考塊（Claude 擴展思考）
- **多模型對比** — 同時向多個模型提問，side-by-side 對比結果
- **函式呼叫** — 跨所有支援提供商的結構化函式呼叫
- **OpenAI Responses API** — 支援 OpenAI Responses 格式傳輸
- **即時 API** — 相容 OpenAI 即時 API 的 WebSocket 事件推送
- **影像生成** — AI 影像生成面板，支援多種模型和參數配置

### 🔐 AI 智慧體系統

智慧體系統基於精密架構構建（agent crate，70+ 原始碼檔案），具備以下特性：

- **ReAct 推理引擎** — 融合推理與行動，內建自驗證確保任務執行可靠
- **層級規劃器** — 將複雜任務分解為具有階段和依賴關係的結構化計畫
- **任務分解器** — 自動將複雜任務分解為可執行的子任務
- **思維鏈** — 智慧體決策推理的可視化，逐步分解
- **思維樹** — tree_of_thoughts 多路徑推理探索
- **深度研究** — 多源搜尋編排、引用追蹤與可信度評估
- **事實核查** — AI 驅動的事實驗證與來源分類
- **搜尋編排** — 多搜尋提供商協調，支援搜尋規劃和結果綜合
- **學術搜尋** — 學術文獻檢索和引用分析
- **計算機控制** — AI 控制的滑鼠點擊、鍵盤輸入、螢幕捲動，配合視覺模型分析
- **螢幕感知** — 截圖擷取和視覺模型分析，用於 UI 元素識別
- **視覺管線** — vision_pipeline 影像理解與分析
- **三級權限模式** — 預設（需要審批）、接受編輯（自動批准）、完全存取（無提示）
- **沙箱隔離** — 智慧體操作嚴格限制在指定工作目錄內
- **工具審批面板** — 即時顯示工具呼叫請求，支援逐條審批
- **成本追蹤** — 即時顯示每個對話的 Token 使用量和成本統計
- **暫停/恢復** — 隨時暫停智慧體執行，稍後恢復
- **檢查點系統** — 持久化檢查點用於崩潰恢復和對話重連
- **錯誤恢復引擎** — 自動錯誤分類、根因分析和恢復策略執行
- **循環檢測** — 自動檢測和中斷智慧體推理中的循環行為
- **主動模式** — 智慧體可主動提供建議和執行操作
- **目的管理** — 維護和追蹤智慧體的執行目的與上下文
- **自驗證** — self_verifier 自動驗證智慧體輸出正確性
- **反思器** — reflector 對推理過程進行反思和改進
- **引導輸入** — steer_manager 動態調整智慧體行為方向
- **事件匯流排** — event_bus / event_emitter 智慧體事件驅動架構
- **內容綜合** — content_synthesizer 多源資訊綜合與報告生成
- **引用追蹤** — citation_tracker 自動追蹤和標註資訊來源
- **可信度評估** — credibility_evaluator 評估資訊來源可信度
- **大綱建構** — outline_builder 自動建構研究大綱
- **模式管理** — schema_manager 管理輸出結構模式
- **專案記憶** — project_memory 專案級別的持久化記憶
- **環境探測** — environment_probe 自動探測執行環境資訊
- **健康檢查** — health_checker 智慧體健康狀態監控

### 👥 多智慧體協作

- **子智慧體協調** — 主從架構，coordinator 協調多個協作智慧體
- **並行執行** — 多個智慧體並行處理任務，支援依賴感知排程
- **對抗性辯論** — adversarial_debate Pro/Con 辯論輪次，支援論點強度評分和反駁追蹤
- **智慧體角色** — agent_roles 預定義角色（研究員、規劃師、開發者、評審員、綜合員）用於團隊協作
- **智慧體編排器** — 多智慧體團隊的中心化訊息路由和狀態管理
- **通訊圖譜** — graph_insights 智慧體互動和訊息流的可視化展示
- **共享黑板** — shared_blackboard / blackboard 跨智慧體共享狀態空間
- **Buddy 夥伴系統** — 可配置的智慧體夥伴，支援物種和屬性定義
- **共享記憶** — 跨智慧體共享的記憶體空間，支援統計和查詢
- **團隊 Cron 註冊** — 團隊級別的定時任務排程
- **專家系統** — agency_expert 領域專家智慧體
- **智慧體畫像** — agent_profile 智慧體個性與能力畫像管理

### ⭐ 技能系統

- **技能市場** — 內建市場，瀏覽和安裝社群貢獻的技能
- **技能建立** — 從提案自動建立技能，支援 Markdown 編輯器
- **技能進化** — skill_evolution 基於執行回饋的 AI 驅動的現有技能自動分析和改進
- **技能匹配** — skill_matcher 語義匹配，推薦與對話上下文相關的技能
- **技能分解** — 自動將複雜任務分解為可執行的原子技能（LLM 輔助/多輪/工作流驗證）
- **生成工具** — AI 自動產生並註冊新工具，擴展智慧體能力
- **技能中心** — skills_hub_adapter 集中的技能發現和配置管理介面
- **技能中心用戶端** — skills_hub_client 與遠端技能中心整合，支援社群分享
- **技能依賴檢查** — 自動檢測技能依賴和工具可用性
- **技能沙箱容器** — 技能在隔離環境中安全執行
- **原子技能** — atomic_skill 最小可執行技能單元
- **技能提案** — skill_proposal AI 驅動的技能建立提案

### 🔄 工作流系統

工作流引擎（rt-workflow crate）實現了基於 DAG 的任務編排系統：

- **可視化工作流編輯器** — 拖放式工作流設計器，支援節點連接和配置
- **16 種節點類型** — 觸發器、智慧體、LLM、條件、並行、循環、合併、延遲、工具、程式碼、子工作流、向量檢索、文件解析、驗證、結束、回退（fallback）
- **16 種屬性面板** — 每種節點類型對應獨立的配置面板
- **工作流範本** — 內建預設：程式碼審查、Bug 修復、文件、測試、重構、探索、效能、安全、功能開發
- **DAG 執行** — Kahn 演算法拓撲排序，支援循環檢測
- **並行排程** — 流水線式執行，快速步驟不等慢速步驟
- **重試策略** — 指數退避，每步可配置最大重試次數
- **部分完成** — 失敗的步驟不會阻塞獨立的下游步驟
- **版本管理** — 工作流範本版本控制，支援回滾
- **執行歷史** — 詳細記錄，支援狀態追蹤和除錯
- **AI 輔助** — AI 輔助工作流設計、節點推薦和智慧體提示詞最佳化
- **語義檢查** — 工作流語義驗證，檢測潛在問題
- **n8n 匯入** — 支援從 n8n 目錄匯入工作流
- **除錯面板** — 工作流執行過程的即時除錯和狀態查看
- **快取層** — cache_layer 工作流執行結果快取
- **市場** — workflow_marketplace 工作流範本市場與評審

### 📚 知識與記憶

- **知識庫（RAG）** — 多知識庫支援，支援文件上傳、自動解析、分塊和向量索引
- **混合搜尋** — 結合向量相似度搜尋與 BM25 全文排名
- **重排序** — Cross-encoder 重排序，提升檢索精度
- **三級召回管道** — AST 索引 + 向量搜尋 + FTS5 的多級召回機制
- **Self-RAG** — self_rag 自適檢索增強生成
- **查詢增強** — query_enhancement 查詢改寫與擴展
- **知識圖譜** — 知識關聯的實體關係可視化（實體、屬性、關係、流、介面）
- **Wiki 系統** — LLM Wiki 編譯器與驗證器，支援知識圖譜可視化與增量同步
- **Wiki 筆記** — 雙向連結筆記系統，支援圖譜檢視和自動連結同步
- **記憶系統** — 多命名空間記憶，支援手動錄入或 AI 自動提取
- **閉環記憶** — 整合 Honcho 和 Mem0 持久化記憶提供商
- **記憶遺忘** — memory_forgetting 基於時間的記憶衰減機制
- **FTS5 全文搜尋** — 跨對話、檔案、記憶的快速檢索
- **對話搜尋** — 跨所有對話對話的高級搜尋
- **上下文管理** — 靈活附加檔案、搜尋結果、知識片段、記憶、工具輸出
- **文件解析** — 多格式文件自動解析和內容提取
- **增量索引** — 檔案變更的增量索引更新
- **文字分塊** — text_chunker 智慧文字分塊策略
- **Token 預算** — token_budget 檢索結果 Token 預算控制

### 🌐 API 網關

- **本地 API 伺服器** — 內建 OpenAI 相容、Claude 和 Gemini 介面伺服器
- **外部連結** — 一鍵整合 Claude CLI、OpenCode，自動同步 API Key 和模型
- **Key 管理** — 產生、撤銷、啟用/停用存取 Key，支援描述
- **用量分析** — 按 Key、提供商、日期的請求量和 Token 使用量
- **SSL/TLS 支援** — 內建自簽名憑證，支援自訂憑證
- **請求日誌** — 完整記錄所有 API 請求和回應
- **配置範本** — Claude、Codex、OpenCode、Gemini 的預建範本
- **即時 API** — 相容 OpenAI 即時 API 的 WebSocket 事件推送
- **平台整合** — 支援釘釘、飛書、QQ、Slack、微信、WhatsApp、Telegram、Discord
- **網關診斷** — 連線診斷和程式策略管理
- **限流器** — API 請求速率限制和流量控制
- **持久化佇列** — 請求持久化佇列管理
- **股票 API** — stock_handlers 股票資料專用 API 端點
- **SSE 推送** — sse Server-Sent Events 即時事件推送

### 🔧 工具與擴展

- **MCP 協定** — 完整的模型上下文協定實現，支援 stdio 和 HTTP/WebSocket 傳輸
- **OAuth 認證** — MCP 伺服器的 OAuth 流程支援
- **MCP 自動啟動** — MCP 伺服器自動啟動和生命週期管理
- **MCP 工具橋接** — MCP 工具與智慧體工具系統的橋接
- **MCP 健康檢查** — mcp_health MCP 伺服器健康狀態監控
- **外掛系統** — OpenClaw 相容的三級外掛架構（內建/捆綁/外部），支援 npm 套件安裝、工具註冊、掛鉤與生命週期管理
- **外掛市場** — 內建市場 UI，支援 npm 搜尋安裝、確認彈窗
- **內建工具** — 40+ 工具模組：檔案操作（讀/寫/編輯/系統）、程式碼執行、搜尋（Grep/Glob）、Bash、Web 搜尋/抓取、計畫管理、Cron 排程、REPL、LSP、上下文管理、計算機控制、訊息推送、待辦事項、資料庫、DevOps、文件解析、Git、知識檢索、LSP、媒體處理、訊息推送、OCR、推送通知、系統資訊、任務系統、測試、工作區/工作樹等
- **工具權限系統** — 工具權限分類、規則管理和使用追蹤
- **Bash 安全** — 命令解析、路徑驗證和沙箱安全控制
- **LSP 用戶端** — 內建語言伺服器協定，支援程式碼補全和診斷
- **AST 索引** — 程式碼檔案的 AST 解析和索引建構
- **終端後端** — 支援本地、Docker 和 SSH 終端連接
- **瀏覽器自動化** — 透過 CDP 整合瀏覽器控制能力（導航、截圖、點擊、填寫、文字提取等）
- **UI 自動化** — 跨平台 UI 元素識別和控制
- **Git 工具** — Git 操作，支援分支檢測和衝突感知
- **工具推薦** — 基於上下文的智慧工具推薦引擎
- **工具編排** — 多工具協調執行和串流輸出
- **工具統計** — 工具使用頻率和效能統計
- **工具稽核** — audit 工具呼叫稽核日誌

### 📊 內容渲染

- **Markdown 渲染** — 完整支援程式碼高亮、LaTeX 數學公式、表格、任務列表
- **Monaco 程式碼編輯器** — 內建編輯器，支援語法高亮、複製、差異預覽
- **圖表渲染** — Mermaid 流程圖、D2 架構圖、ECharts 互動式圖表
- **產物面板** — 程式碼片段、HTML 草稿、React 元件、Markdown 筆記，支援即時預覽
- **四種預覽模式** — 程式碼（編輯器）、分屏（並排）、預覽（僅渲染）、React 元件預覽
- **對話檢查器** — 對話結構的樹形檢視，快速導航
- **引用面板** — 追蹤和顯示來源引用，支援可信度評分
- **資訊圖渲染** — 支援資訊圖可視化展示
- **圖表解釋器** — ChartInterpreter AI 驅動的圖表解讀
- **差異檢視器** — DiffViewer 程式碼差異對比

### 🛡️ 資料與安全

- **AES-256 加密** — API Key 和敏感資料使用 AES-256-GCM 加密
- **隔離儲存** — 應用狀態儲存在 `~/.axinvest/`，使用者檔案儲存在 `~/Documents/axinvest/`
- **自動備份** — 計畫備份到本地目錄或 WebDAV 儲存
- **S3 備份** — s3_backup 支援 Amazon S3 雲端備份
- **備份恢復** — 一鍵從歷史備份恢復
- **匯出選項** — PNG 截圖、Markdown、純文字、JSON 格式
- **儲存管理** — 可視化磁碟使用顯示和清理工具
- **儲存遷移** — storage_migration 版本間資料遷移
- **檔案授權** — 檔案存取授權和撤銷管理
- **操作稽核** — 關鍵操作的稽核日誌記錄
- **命令驗證** — command_validator 命令安全驗證
- **資源限制** — resource_limits 資源使用限制
- **沙箱執行** — sandbox_runner 隔離環境執行

### 🖥️ 桌面體驗

- **主題引擎** — 深色/淺色主題，支援跟隨系統或手動偏好
- **介面語言** — 11 種語言：簡體中文、繁體中文、英語、日語、韓語、法語、德語、西班牙語、俄語、印地語、阿拉伯語
- **系統匣** — 最小化到系統匣，不中斷後台服務
- **置頂視窗** — 視窗置頂於其他視窗之上
- **全域快捷鍵** — 可自訂快捷鍵叫出主視窗
- **QuickBar** — 快速存取浮動條，一鍵喚起
- **開機自啟** — 可選在系統啟動時執行
- **代理支援** — HTTP 和 SOCKS5 代理配置
- **自動更新** — 自動檢查版本，有更新時提示
- **命令面板** — `Cmd/Ctrl+K` 快速存取命令
- **引導精靈** — 首次使用的互動式引導和 Ollama 偵測
- **通知中心** — 統一的應用內通知管理
- **雲端工作區** — cloud_workspace 雲端工作區選擇
- **崩潰報告** — crash_report 自動崩潰報告收集
- **語音通話** — VoiceCall 語音對話能力

### 🔬 高級功能

- **深度研究** — 多源搜尋、引用追蹤、可信度評估與內容綜合
- **事實核查** — AI 驅動的事實驗證與來源分類
- **Cron 排程器** — 自動化任務排程，支援每日/每週/每月範本和自訂 cron 表達式
- **Webhook 系統** — 事件訂閱，支援工具完成、智慧體錯誤、對話結束通知
- **使用者畫像** — 自動學習程式碼風格、命名規範、縮排、註解風格、溝通偏好
- **RL 最佳化器** — 強化學習最佳化工具選擇和任務策略
- **LoRA 微調** — 使用 LoRA 進行本地訓練的自訂模型適配
- **主動建議** — 基於對話內容和使用者模式的上下文感知提示
- **上下文預測** — 預測使用者下一步操作並預取相關資源
- **夢境整合** — dream_consolidation 後台自動整合記憶與模式，最佳化長期知識
- **錯誤恢復** — 自動錯誤分類、根因分析和恢復建議
- **開發者工具** — Trace、Span、時間線可視化，用於除錯和效能分析
- **基準測試系統** — SWE-bench / Terminal-bench 任務效能評估和指標，帶評分卡
- **風格遷移** — style_migrator 將學習的程式碼風格偏好套用到生成的程式碼
- **儀表盤外掛** — 可擴展的儀表盤，支援自訂面板和小工具
- **協作共享** — CRDT 即時協作與一鍵對話分享
- **瀏覽器擴充功能** — Wiki Clipper 瀏覽器擴充功能，快速剪藏網頁到 LLM Wiki
- **Python SDK** — 提供 Python SDK 用於與 AxInvest 整合
- **智慧路由** — 請求智慧路由和分類
- **語義快取** — 基於語義的回應快取，減少重複計算
- **上下文壓縮** — 自動壓縮長上下文，最佳化 Token 使用
- **訊息批次處理** — 訊息批次傳送和最佳化
- **連線池** — 資料庫和 API 連線池管理
- **特性開關** — 可配置的功能特性開關系統
- **策略引擎** — 權限和操作策略的集中管理
- **資源治理** — 智慧體資源使用限制和治理
- **LAN 傳輸** — 區域網路檔案傳輸能力
- **共進化** — coevolution 技能與智慧體協同進化
- **行為學習** — behavior_learner / behavior_tracker 使用者行為學習與追蹤
- **偏好學習** — preference_learner 使用者偏好自動學習
- **內在獎勵** — intrinsic_reward 內在動機驅動的探索
- **過程獎勵** — process_reward 過程級獎勵訊號
- **TextGrad** — text_grad 基於文字梯度的自動最佳化
- **軌跡壓縮** — trajectory_compressor 長軌跡自動壓縮
- **提醒管理** — reminder_manager 智慧提醒排程
- **任務預取** — task_prefetcher 預測性任務資源預取

### 🛡️ 提示詞注入防護（Prompt-Guard）

- **四級防護體系** — L1 模式檢測（高風險攔截 + 中風險標記）→ L2 分隔符跳脫 → L3 XML 包裝器 → L4 信任標籤
- **Pipeline 編排器** — 多級檢測管道串聯，支援自訂風險閾值
- **Token Smuggling 檢測** — 針對編碼混淆和 Token 走私攻擊的專項檢測
- **分隔符跳脫檢測** — delimiter_escape 檢測提示詞分隔符逃逸攻擊
- **模式檢測** — pattern_detect 正則+啟發式注入模式匹配
- **信任標籤** — trust_labels 可信內容標記與驗證
- **Strict 模式** — 嚴格模式測試 + 中風險原因命名 + 自訂模式文件
- **全管道整合** — 已整合到 session / prompt / git / RAG 各環節

### 📱 行動端支援

- **Android 原生** — APK/AAB 建構，支援 arm64-v8a / armeabi-v7a / x86_64
- **iOS 原生** — IPA 建構，支援 arm64
- **自適應佈局** — 桌面/平板/手機三檔自動適配（useResponsive hook）
- **行動端導航** — Drawer 滑出導航 + 底部導航列 + 閃現式浮動按鈕
- **安全區適配** — Android 系統狀態列/導航列 CSS env() 自適應
- **CSP 最佳化** — Android WebView CSP 協定白名單
- **條件編譯** — `#[cfg(not(mobile))]` 桌面專屬功能（瀏覽器、計算機控制、桌面、QuickBar、終端、螢幕視覺）自動排除

---

## 技術架構

### 技術堆疊

| 層級 | 技術 |
|------|------|
| **框架** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **狀態管理** | Zustand 5 |
| **路由** | React Router 7 |
| **國際化** | i18next + react-i18next |
| **後端** | Rust 2024 + SeaORM 2 + SQLite |
| **向量資料庫** | sqlite-vec |
| **程式碼編輯器** | Monaco Editor |
| **圖表** | Mermaid + D2 + ECharts（CDN） |
| **終端** | xterm.js 6 |
| **工作流** | ReactFlow 11 |
| **圖表渲染** | @antv/infographic |
| **圖示** | Iconify + Lucide |
| **拖曳** | @dnd-kit |
| **建構** | Vite 8 + npm |
| **測試** | Vitest + Playwright + cargo-nextest |
| **格式化** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **行動端** | Tauri Android + iOS 原生建構 |
| **桌面端** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### 平台支援

| 平台 | 架構 |
|------|------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (模擬器) |
| iOS | arm64 |

### Rust 後端架構

後端組織為 Rust workspace，包含 **20 個** 專業化的 crates：

```
src-tauri/crates/
├── agent/            # AI 智慧體核心（70+ 原始碼檔案：ReAct 引擎、協調、規劃、深度研究、事實核查等）
├── astock-data/      # A 股資料來源（9 大資料來源、22 種資料路由、技術指標、交易日曆、MCP 工具註冊）
├── core/             # 核心工具（85+ 資料庫實體、40+ 儲存庫、RAG、加密、MCP、瀏覽器自動化、AST 索引等）
├── gateway/          # API 網關（HTTP 伺服器、認證、路由、OpenAI 相容介面、股票 API 端點）
├── migration/        # 資料庫遷移（5 個遷移：股票分析/自選組合/分析排程/價格預警/交易）
├── npm/              # npm 套件解析與註冊表
├── plugins/          # 外掛系統（OpenClaw 相容，npm 套件安裝，含範例外掛）
├── prompt-guard/     # 提示詞注入防護（L1-L4 多級檢測與防禦，4 種檢測器）
├── providers/        # 模型提供商介面卡（OpenAI、Anthropic、Gemini、Ollama、OpenClaw、Hermes、影像生成）
├── rt-dashboard/     # 儀表盤外掛系統
├── rt-messaging/     # 訊息網關（9 平台：釘釘/飛書/QQ/Slack/微信/WhatsApp/Telegram/Discord）
├── rt-theme/         # 主題引擎
├── rt-webhook/       # Webhook 伺服器與分發
├── rt-workflow/      # 工作流引擎（DAG 編排、16 種節點執行器、排程器、快取層）
├── runtime/          # 執行時服務（70+ 原始碼檔案：對話管理、MCP、終端、限流、Webhook、權限、基準測試等）
├── runtime-core/     # 執行時抽象層（公共類型、trait 定義、配置、特性開關、權限執行器）
├── stock-analysis/   # 智慧投資分析（23 個子模組：流水線、決策引擎、風險評估、回測、選股器、價值投資）
├── telemetry/        # 遙測與分散式追蹤（OpenTelemetry 相容）
├── tools/            # 工具系統（40+ 內建工具、Bash 安全、MCP 橋接、權限系統、編排、稽核）
└── trajectory/       # 學習系統（55+ 原始碼檔案：記憶、技能、RL、使用者畫像、夢境整合、風格遷移、共進化）
```

#### stock-analysis crate 模組結構（23 個子模組）

```
stock-analysis/
├── backtest.rs         # 策略回測引擎
├── data_clean.rs       # 資料清洗與預處理
├── decision.rs         # 投資決策引擎
├── key_levels.rs       # 關鍵價位識別
├── monitor.rs          # 即時監控與預警
├── orchestrator.rs     # 分析流水線編排
├── pipeline.rs         # 多階段分析管道
├── plugin.rs           # 分析外掛擴展
├── portfolio_risk.rs   # 投資組合風險評估
├── position_limits.rs  # 倉位限制與合規
├── prompts.rs          # AI 提示詞範本
├── quality.rs          # 資料品質檢查
├── report.rs           # 分析報告生成
├── review.rs           # 分析結果複核
├── risk.rs             # 風險評估模型
├── rules.rs            # 交易規則引擎
├── runner.rs           # 分析任務執行器
├── scoring.rs          # 綜合評分系統
├── screener.rs         # 選股器
├── signals.rs          # 交易訊號生成
├── trading.rs          # 交易策略框架
├── value.rs            # 價值分析
└── value_investing.rs  # 價值投資評估
```

#### astock-data crate 資料來源

| 資料來源 | 標識 | 支援的資料類型 |
|----------|------|---------------|
| 騰訊財經 | tencent | 即時行情、K 線 |
| 通達信 | mootdx | 即時行情、K 線 |
| 東方財富 | eastmoney | 行情、K 線、財務、資金流向、龍虎榜、限售解禁、融資融券、北向資金、行業分類、股東增減持、分紅、研報、全市場龍虎榜、財聯社快訊 |
| 新浪財經 | sina | 行情、K 線、新聞 |
| 百度股票 | baidu_stock | 行情、新聞、資金流向、龍虎榜、限售解禁、融資融券、北向資金、行業分類、股東增減持、分紅、研報、熱門股票、行業排名、概念板塊、北向資金流向 |
| 同花順 | ths | 行情、行業分類、一致預期 EPS、概念板塊、熱門股票、行業排名、北向資金流向 |
| 問財 | iwencai | 股票搜尋、行業分類、一致預期 EPS、概念板塊、熱門股票 |
| 巨潮資訊 | cninfo | 公告 |
| AKShare | akshare | 財務、新聞、一致預期 EPS、財聯社快訊 |

每種資料類型配置多源降級路由，當主資料來源不可用時自動切換至備用源。

#### astock-data 額外模組

| 模組 | 功能 |
|------|------|
| calendar | A 股交易日曆（2025-2026 年節假日 + 調休工作日） |
| indicators | 技術指標計算（MA/MACD/RSI/布林帶/乖離率/量比/支撐壓力位） |
| mcp_tools | MCP 工具註冊（股票資料能力註冊為 AI 可呼叫工具） |

### 前端架構

```
src/
├── stores/                    # Zustand 狀態管理（65 個 store）
│   ├── domain/               # 核心業務狀態（9 個）
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # 功能模組狀態（46 個）
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
│   ├── devtools/              # 開發者工具狀態（5 個）
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # 共享狀態（5 個）
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # React 元件（25 個模組）
│   ├── chat/                # 對話介面（100+ 元件：Agent 執行面板、分支對比、瀏覽器自動化、程式碼執行器、協作面板、深度研究、事實核查、Git 提交、影像生成/分析、知識檢索、記憶提取、模型路由、多模型展示、權限管理、外掛市場、反思面板、技能建立/進化、結構化思考、子智慧體卡片、工具呼叫卡片、軌跡回放、語音通話、Wiki 檢索、工作流進度等）
│   ├── stock-analysis/      # 智慧投資分析（16 個元件）
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
│   ├── workflow/            # 工作流編輯器（16 種節點 + 16 種屬性面板 + AI 面板 + 範本 + 除錯）
│   ├── gateway/             # API 網關 UI（概覽/金鑰/指標/監控/設定/範本/診斷）
│   ├── settings/            # 設定面板（50+ 元件：提供商/模型/MCP/知識/記憶/代理/快捷鍵/主題/工具/Webhook/Cron/股票分析配置等）
│   ├── terminal/            # 終端 UI（整合終端/Docker/SSH/後端選擇/路徑補全/斜線補全）
│   ├── skill/               # 技能編輯器與渲染器（動作鏈編輯/前端編輯器/沙箱容器/依賴檢查/統計面板）
│   ├── benchmark/           # 基準測試面板（配置/報告/選擇器/任務列表/結果）
│   ├── files/               # 檔案管理頁面
│   ├── fine-tune/           # LoRA 微調配置（資料集/訓練任務/LoRA 配置）
│   ├── link/                # 外部連結管理（概覽/模型/策略/技能/策略詳情）
│   ├── llm-wiki/            # LLM Wiki 編輯器（品質評分/同步狀態）
│   ├── proactive/           # 主動建議系統（上下文預測/預取指示器/建議列/提醒列表）
│   ├── wiki/                # Wiki 管理（反向連結/圖譜檢視/攝入/程式碼檢查/操作時間線/標籤聚合/版本歷史）
│   ├── devtools/            # Trace/Span 時間線（成本圖表/持續時間圖表/詳情/篩選器/列表）
│   ├── decomposition/       # 技能分解（分解預覽/工具依賴/工具生成/工具安裝）
│   ├── recommendation/      # 工具推薦面板
│   ├── style/               # 程式碼風格遷移（樣本/調整滑桿/對比/預覽面板）
│   ├── layout/              # 佈局元件（標題列/側邊欄/命令面板/全域複製/錯誤邊界/狀態列/通知鈴/使用者畫像模態框）
│   ├── help/                # 幫助面板
│   ├── notification/        # 通知中心
│   ├── search/              # 對話搜尋
│   ├── onboarding/          # 引導精靈（互動式教程/歡迎精靈）
│   ├── common/              # 通用元件（複製/圖示/模型參數滑桿/貼上）
│   └── shared/              # 共享元件（頭像編輯/模態框/圖表渲染/動態圖示/嵌入模型選擇/Emoji 選擇/知識庫圖示/MCP 圖示/模型選擇/Monaco 編輯器/命名空間圖示/搜尋提供商圖示）
│
├── pages/                    # 頁面元件（22 個頁面）
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
├── hooks/                    # React hooks（12 個）
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
├── lib/                      # 工具函數（33 個模組 + Web Worker）
│   ├── workers/            # Web Worker（heavy.worker.ts）
│   ├── actionRouter.ts     # 動作路由
│   ├── artifactRenderer.ts # 產物渲染
│   ├── chartGenerator.ts   # 圖表生成
│   ├── chatMarkdown.ts     # Markdown 渲染
│   ├── codeExecutor.ts     # 程式碼執行
│   ├── invoke.ts           # Tauri IPC 封裝
│   ├── skillActionExecutor.ts  # 技能動作執行
│   ├── skillEventBus.ts    # 技能事件匯流排
│   ├── skillLifecycle.ts   # 技能生命週期
│   ├── skillPermissions.ts # 技能權限
│   ├── storeRegistry.ts    # Store 註冊表
│   ├── tokenEstimator.ts   # Token 估算
│   ├── workflowLayout.ts   # 工作流佈局
│   └── ...                 # 其他工具模組
│
├── types/                    # TypeScript 類型定義（22 個）
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
└── i18n/                     # 11 種語言翻譯
```

## 快速開始

### 下載預建構版本

訪問 [Releases](https://github.com/polite0803/AxAgent/releases) 頁面，下載適合您平台的安裝程式。

### 從原始碼建構

#### 環境要求

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + Rust MSVC targets

#### 建構步驟

```bash
# 複製倉庫
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# 安裝依賴
npm install

# 開發模式
npm run tauri dev

# 僅建構前端
npm run build

# 建構桌面應用
npm run tauri build
```

建構產物位於 `src-tauri/target/release/`。

### 測試

```bash
# 單元測試
npm run test          # Vitest watch
npm run test:run      # Vitest 單次執行

# E2E 測試
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright UI 模式

# Rust 後端測試
cd src-tauri && cargo nextest run   # cargo-nextest（快 2-3x）
cd src-tauri && cargo test          # 標準測試

# 類型檢查
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# 程式碼格式化
npm run format        # dprint
cd src-tauri && cargo fmt

# CI 全量檢查
npm run ci:check
```

---

## 專案結構

```
AxInvest/
├── src/                         # 前端原始碼 (React + TypeScript)
│   ├── components/              # React 元件（25 個模組）
│   │   ├── chat/               # 對話介面（100+ 元件）
│   │   ├── stock-analysis/     # 智慧投資分析（16 個元件）
│   │   ├── workflow/           # 工作流編輯器（16 種節點 + 屬性面板 + AI 面板）
│   │   ├── gateway/            # API 網關元件
│   │   ├── settings/           # 設定面板（50+ 元件）
│   │   ├── terminal/           # 終端元件
│   │   ├── skill/              # 技能編輯器與渲染器
│   │   ├── benchmark/          # 基準測試
│   │   ├── files/              # 檔案管理
│   │   ├── fine-tune/          # LoRA 微調
│   │   ├── link/               # 外部連結
│   │   ├── llm-wiki/           # LLM Wiki
│   │   ├── proactive/          # 主動建議
│   │   ├── wiki/               # Wiki 管理
│   │   ├── devtools/           # 開發者工具
│   │   ├── decomposition/      # 技能分解
│   │   ├── recommendation/     # 工具推薦
│   │   ├── style/              # 程式碼風格
│   │   ├── layout/             # 佈局元件
│   │   ├── help/               # 幫助面板
│   │   ├── notification/       # 通知中心
│   │   ├── search/             # 對話搜尋
│   │   ├── onboarding/         # 引導精靈
│   │   ├── common/             # 通用元件
│   │   └── shared/             # 共享元件
│   ├── pages/                   # 頁面元件（22 個頁面）
│   ├── stores/                  # Zustand 狀態管理（65 個 store）
│   │   ├── domain/            # 核心業務狀態（9 個）
│   │   ├── feature/           # 功能模組狀態（46 個）
│   │   ├── devtools/          # 開發者工具狀態（5 個）
│   │   └── shared/            # 共享狀態（5 個）
│   ├── hooks/                   # React hooks（12 個）
│   ├── lib/                     # 工具函數（33 個模組 + Web Worker）
│   ├── types/                   # TypeScript 類型定義（22 個）
│   ├── sdk/                     # SDK（TypeScript + Python）
│   └── i18n/                    # 11 種語言翻譯
│
├── src-tauri/                    # 後端原始碼 (Rust)
│   ├── crates/                  # Rust workspace（20 個 crates）
│   │   ├── agent/             # AI 智慧體核心（70+ 原始碼檔案）
│   │   ├── astock-data/       # A 股資料來源（9 大資料來源、22 種資料路由、技術指標、交易日曆）
│   │   ├── core/              # 核心工具（85+ 實體、40+ 儲存庫、RAG、加密、MCP）
│   │   ├── gateway/           # API 網關（含股票 API 端點）
│   │   ├── migration/         # 資料庫遷移（5 個遷移）
│   │   ├── npm/               # npm 套件解析
│   │   ├── plugins/           # 外掛系統
│   │   ├── prompt-guard/      # 提示詞注入防護
│   │   ├── providers/         # 模型提供商介面卡
│   │   ├── rt-dashboard/      # 儀表盤外掛
│   │   ├── rt-messaging/      # 訊息網關（9 平台）
│   │   ├── rt-theme/          # 主題引擎
│   │   ├── rt-webhook/        # Webhook 伺服器
│   │   ├── rt-workflow/       # 工作流引擎（16 種節點執行器）
│   │   ├── runtime/           # 執行時服務（70+ 原始碼檔案）
│   │   ├── runtime-core/      # 執行時抽象層
│   │   ├── stock-analysis/    # 智慧投資分析（23 個子模組）
│   │   ├── telemetry/         # 追蹤與指標
│   │   ├── tools/             # 工具系統（40+ 內建工具）
│   │   └── trajectory/        # 學習系統（55+ 原始碼檔案）
│   └── src/                    # Tauri 入口點（91 個命令模組）
│       ├── commands/          # 命令模組
│       │   ├── stock_analysis.rs        # 股票分析命令
│       │   ├── stock_analysis_setup.rs  # 股票分析配置
│       │   ├── stock_workflow.rs        # 股票工作流命令
│       │   ├── agency_expert.rs         # 專家智慧體
│       │   ├── agent_advanced.rs        # 高級智慧體
│       │   ├── agent_analytics.rs       # 智慧體分析
│       │   ├── agent_insight.rs         # 智慧體洞察
│       │   ├── agent_nudge.rs           # 智慧體提示
│       │   ├── agent_profile.rs         # 智慧體畫像
│       │   ├── agent_role.rs            # 智慧體角色
│       │   ├── background_tasks.rs      # 後台任務
│       │   ├── browser.rs              # 瀏覽器自動化
│       │   ├── chart_generator.rs       # 圖表生成
│       │   ├── cloud_workspace.rs       # 雲端工作區
│       │   ├── computer_control.rs      # 計算機控制
│       │   ├── context_breakdown.rs     # 上下文分解
│       │   ├── conversation_categories.rs  # 對話分類
│       │   ├── conversations_search.rs  # 對話搜尋
│       │   ├── crash_report.rs          # 崩潰報告
│       │   ├── dream.rs                # 夢境整合
│       │   ├── evolution.rs            # 技能進化
│       │   ├── fine_tune.rs            # LoRA 微調
│       │   ├── gateway.rs              # API 網關
│       │   ├── gateway_link.rs         # 外部連結
│       │   ├── generated_tool.rs        # 生成工具
│       │   ├── image_gen.rs            # 影像生成
│       │   ├── knowledge.rs            # 知識庫
│       │   ├── llm_wiki.rs             # LLM Wiki
│       │   ├── local_models.rs         # 本地模型
│       │   ├── mcp.rs                  # MCP 協定
│       │   ├── memory.rs              # 記憶系統
│       │   ├── message_continuation.rs  # 訊息續寫
│       │   ├── onboarding.rs           # 引導精靈
│       │   ├── parallel_execution.rs    # 並行執行
│       │   ├── plan.rs                 # 計畫管理
│       │   ├── platform_integration.rs  # 平台整合
│       │   ├── plugin.rs               # 外掛管理
│       │   ├── proactive.rs            # 主動建議
│       │   ├── prompt_templates.rs      # 提示詞範本
│       │   ├── providers.rs            # 模型提供商
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # 反思
│       │   ├── research.rs             # 深度研究
│       │   ├── rl.rs                   # 強化學習
│       │   ├── sandbox.rs              # 沙箱
│       │   ├── scheduled_task.rs        # 定時任務
│       │   ├── screen_vision.rs        # 螢幕視覺
│       │   ├── search.rs               # 搜尋
│       │   ├── session_share.rs         # 對話分享
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # 技能分解
│       │   ├── skills_hub.rs           # 技能中心
│       │   ├── tool_recommender.rs      # 工具推薦
│       │   ├── tracer.rs               # 追蹤
│       │   ├── user_profile.rs          # 使用者畫像
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # 工作引擎
│       │   ├── workflow_ai.rs          # AI 工作流
│       │   ├── workflow_template.rs     # 工作流範本
│       │   └── ...                     # 其他命令
│       ├── init/              # 初始化模組
│       ├── stock_scheduler.rs # 股票排程器
│       └── ...                # 其他核心模組
│
├── extension/                  # 瀏覽器擴充功能（Wiki Clipper：popup/content/background）
├── e2e/                        # Playwright E2E 測試（9 個測試套件）
├── scripts/                    # 建構與工具腳本
└── website/                    # 專案網站（VitePress，11 種語言文件）
```

## 資料目錄

```
~/.axinvest/                     # 配置目錄
├── axinvest.db                  # SQLite 資料庫
├── master.key                   # AES-256 主金鑰
├── vector_db/                   # 向量資料庫 (sqlite-vec)
└── ssl/                         # SSL 憑證

~/Documents/axinvest/           # 使用者檔案目錄
├── images/                     # 圖片附件
├── files/                      # 檔案附件
└── backups/                    # 備份檔案
```

---

## 常見問題

### macOS：提示「應用已損壞」或「無法驗證開發者」

由於應用未經過 Apple 簽名：

**1. 允許執行「任何來源」的應用**
```bash
sudo spctl --master-disable
```

然後前往 **系統設定 → 隱私與安全性 → 安全性**，選擇 **任何來源**。

**2. 移除隔離屬性**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. macOS Ventura+ 額外步驟**
前往 **系統設定 → 隱私與安全性**，點擊 **仍要打開**。

---

## 社群

- [LinuxDO](https://linux.do)

## 開源協定

本專案基於 [AGPL-3.0](LICENSE) 協定開源。
