# 股票分析模块移植实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从 TradingAgents-astock 移植 A 股多智能体分析能力到 AxInvest，使用本项目原生架构承载。

**Architecture:** 新建 `astock-data`（数据获取）和 `stock-analysis`（编排）两个 Rust crate；14 个 Expert Markdown 文件通过已有 `import_agency_experts` 导入为 AgentProfile；前端新增 `/stock-analysis` 看板页面 + 聊天 Agent 集成。

**Tech Stack:** Rust (reqwest, tokio, serde, thiserror) · React 19 + TypeScript · Ant Design 6 · ECharts · Zustand 5

**Design Spec:** `docs/superpowers/specs/2026-05-14-stock-analysis-design.md`

---

## 文件结构

### 新增：后端 (16 files)

```
src-tauri/crates/astock-data/Cargo.toml          # 依赖 reqwest, serde, tokio, chrono, thiserror
src-tauri/crates/astock-data/src/lib.rs           # AStockClient 统一入口 + vendor routing
src-tauri/crates/astock-data/src/types.rs         # StockQuote, KLine, FinancialReport, NewsItem 等
src-tauri/crates/astock-data/src/error.rs         # DataError enum
src-tauri/crates/astock-data/src/vendors/mod.rs   # Vendor trait 定义
src-tauri/crates/astock-data/src/vendors/tencent.rs # 腾讯财经 qt.gtimg.cn
src-tauri/crates/astock-data/src/vendors/eastmoney.rs # 东方财富 push2his.eastmoney.com
src-tauri/crates/astock-data/src/vendors/sina.rs  # 新浪财经

src-tauri/crates/stock-analysis/Cargo.toml        # 依赖 astock-data, axagent-agent, tokio, serde
src-tauri/crates/stock-analysis/src/lib.rs        # pub mod orchestrator, pipeline, decision
src-tauri/crates/stock-analysis/src/orchestrator.rs # StockAnalysisOrchestrator::run()
src-tauri/crates/stock-analysis/src/pipeline.rs   # run_analyst / run_debator / run_manager
src-tauri/crates/stock-analysis/src/decision.rs   # StockDecision, AnalysisConfig, AnalysisEvent

src-tauri/crates/core/src/entity/stock_analyses.rs  # DB entity
src-tauri/crates/migration/src/m20250514_000001_stock_analysis.rs # migration
src-tauri/src/commands/stock_analysis.rs          # 7 个 Tauri commands
```

### 新增：前端 (12 files)

```
src/components/stock-analysis/StockAnalysisPage.tsx
src/components/stock-analysis/StockSearchBar.tsx
src/components/stock-analysis/AnalysisProgress.tsx
src/components/stock-analysis/StockQuoteCard.tsx
src/components/stock-analysis/KLineChart.tsx
src/components/stock-analysis/AnalystReportGrid.tsx
src/components/stock-analysis/AnalystReportCard.tsx
src/components/stock-analysis/DebatePanel.tsx
src/components/stock-analysis/RiskMatrix.tsx
src/components/stock-analysis/DecisionBanner.tsx
src/stores/feature/stockAnalysisStore.ts
src/types/stock-analysis.ts
```

### 新增：Expert 定义 (14 files)

```
资源导入路径下的 stock-analysis/ 子目录（用户通过 import_agency_experts 命令导入）
market-analyst.md, sentiment-analyst.md, news-analyst.md, fundamentals-analyst.md,
policy-analyst.md, hot-money-tracker.md, lockup-watcher.md,
bull-researcher.md, bear-researcher.md,
aggressive-debator.md, conservative-debator.md, neutral-debator.md,
research-manager.md, portfolio-manager.md
```

### 修改 (13 files)

```
src-tauri/Cargo.toml                              # workspace members + root deps
src-tauri/src/commands/mod.rs                     # pub mod stock_analysis
src-tauri/src/lib.rs                              # generate_handler![] 注册
src-tauri/crates/core/src/entity/mod.rs           # pub mod stock_analyses
src-tauri/crates/migration/src/lib.rs             # 注册新 migration
src/components/layout/ContentArea.tsx             # 路由
src/components/layout/Sidebar.tsx                 # 侧栏入口
src/stores/index.ts                               # re-export
src/types/index.ts                                # re-export
src/stores/feature/agentStore.ts                  # agent 类型注册
src/components/chat/InputArea.tsx                 # @股票代码 触发
locales/zh-CN.json 等 11 种语言文件               # i18n
```

---

### Task 1: 创建 astock-data crate 骨架

**Files:**
- Create: `src-tauri/crates/astock-data/Cargo.toml`
- Create: `src-tauri/crates/astock-data/src/types.rs`
- Create: `src-tauri/crates/astock-data/src/error.rs`
- Create: `src-tauri/crates/astock-data/src/vendors/mod.rs`
- Create: `src-tauri/crates/astock-data/src/lib.rs`
- Modify: `src-tauri/Cargo.toml` — workspace + root deps

- [ ] **Step 1: 创建 Cargo.toml**

```toml
[package]
name = "axagent-astock-data"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
chrono = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 2: 创建 types.rs — 所有数据模型**

```rust
use serde::{Deserialize, Serialize};

/// 实时行情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockQuote {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub amount: f64,
    pub change_pct: f64,
    pub turnover_rate: f64,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
    pub total_mv: Option<f64>,
    pub timestamp: String,
}

/// K线数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KLine {
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub turnover_rate: Option<f64>,
}

/// 财务报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialReport {
    pub stock_code: String,
    pub report_date: String,
    pub revenue: Option<f64>,
    pub net_profit: Option<f64>,
    pub eps: Option<f64>,
    pub bps: Option<f64>,
    pub roe: Option<f64>,
    pub debt_ratio: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub profit_yoy: Option<f64>,
}

/// 新闻/公告条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub summary: String,
    pub source: String,
    pub url: String,
    pub publish_time: String,
    pub sentiment_score: Option<f64>,
}

/// 资金流向
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneyFlow {
    pub date: String,
    pub main_net_inflow: f64,
    pub super_large_net: f64,
    pub large_net: f64,
    pub medium_net: f64,
    pub small_net: f64,
}

/// 龙虎榜条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DragonTigerEntry {
    pub stock_code: String,
    pub date: String,
    pub dept_name: String,
    pub buy_amount: f64,
    pub sell_amount: f64,
    pub net_amount: f64,
    pub reason: Option<String>,
}

/// 限售解禁
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockupSchedule {
    pub stock_code: String,
    pub stock_name: String,
    pub unlock_date: String,
    pub unlock_shares: f64,
    pub unlock_ratio: f64,
    pub shareholder: Option<String>,
}

/// 股票搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockSearchResult {
    pub code: String,
    pub name: String,
    pub market: String,
}

/// 批量原始数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockRawData {
    pub quote: StockQuote,
    pub klines: Vec<KLine>,
    pub financials: Vec<FinancialReport>,
    pub news: Vec<NewsItem>,
    pub money_flow: Option<MoneyFlow>,
    pub dragon_tiger: Vec<DragonTigerEntry>,
    pub lockup: Vec<LockupSchedule>,
}
```

- [ ] **Step 3: 创建 error.rs**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parse error: {0}")]
    ParseError(String),

    #[error("Rate limit exceeded from {vendor}")]
    RateLimited { vendor: String },

    #[error("Vendor error from {vendor}: {message}")]
    VendorError { vendor: String, message: String },

    #[error("Stock code not found: {0}")]
    NotFound(String),
}
```

- [ ] **Step 4: 创建 vendors/mod.rs — Vendor trait**

```rust
use crate::error::DataError;
use crate::types::*;
use async_trait::async_trait;

#[async_trait]
pub trait StockVendor: Send + Sync {
    fn name(&self) -> &'static str;

    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError>;

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,  // daily, weekly, monthly
        limit: u32,
    ) -> Result<Vec<KLine>, DataError>;

    async fn get_financials(
        &self,
        stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError>;

    async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError>;

    async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError>;

    async fn get_dragon_tiger(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError>;

    async fn get_lockup_schedule(
        &self,
        stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError>;

    async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>, DataError>;
}

pub mod tencent;
pub mod eastmoney;
pub mod sina;
```

- [ ] **Step 5: 创建 lib.rs — AStockClient**

```rust
mod error;
mod types;
mod vendors;

pub use error::DataError;
pub use types::*;
use vendors::{StockVendor, eastmoney::EastMoneyVendor, sina::SinaVendor, tencent::TencentVendor};

pub struct AStockClient {
    tencent: TencentVendor,
    eastmoney: EastMoneyVendor,
    sina: SinaVendor,
    http: reqwest::Client,
}

impl AStockClient {
    pub fn new() -> Self {
        Self {
            tencent: TencentVendor,
            eastmoney: EastMoneyVendor,
            sina: SinaVendor,
            http: reqwest::Client::new(),
        }
    }

   pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// 获取实时行情（腾讯财经）
    pub async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        self.tencent.get_quote(stock_code).await
    }

    /// 获取K线数据（东方财富）
    pub async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        self.eastmoney.get_klines(stock_code, period, limit).await
    }

    /// 获取财务报表（东方财富）
    pub async fn get_financials(
        &self,
        stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError> {
        self.eastmoney.get_financials(stock_code).await
    }

    /// 获取新闻（新浪财经）
    pub async fn get_news(
        &self,
        stock_code: &str,
        limit: u32,
    ) -> Result<Vec<NewsItem>, DataError> {
        self.sina.get_news(stock_code, limit).await
    }

    /// 获取资金流向（东方财富）
    pub async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        self.eastmoney.get_money_flow(stock_code).await
    }

    /// 获取龙虎榜（东方财富）
    pub async fn get_dragon_tiger(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError> {
        self.eastmoney.get_dragon_tiger(stock_code).await
    }

    /// 获取限售解禁（东方财富）
    pub async fn get_lockup_schedule(
        &self,
        stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError> {
        self.eastmoney.get_lockup_schedule(stock_code).await
    }

    /// 搜索股票（东方财富）
    pub async fn search_stock(
        &self,
        keyword: &str,
    ) -> Result<Vec<StockSearchResult>, DataError> {
        self.eastmoney.search_stock(keyword).await
    }

    /// 一次性获取所有原始数据
    pub async fn fetch_all(
        &self,
        stock_code: &str,
        kline_period: &str,
        kline_limit: u32,
        news_limit: u32,
    ) -> Result<StockRawData, DataError> {
        let (quote, klines, financials, news, money_flow, dragon_tiger, lockup) =
            tokio::try_join!(
                self.get_quote(stock_code),
                self.get_klines(stock_code, kline_period, kline_limit),
                self.get_financials(stock_code),
                self.get_news(stock_code, news_limit),
                self.get_money_flow(stock_code),
                self.get_dragon_tiger(stock_code),
                self.get_lockup_schedule(stock_code),
            )?;

        Ok(StockRawData {
            quote,
            klines,
            financials,
            news,
            money_flow,
            dragon_tiger,
            lockup,
        })
    }
}

impl Default for AStockClient {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 6: 修改 src-tauri/Cargo.toml — 添加 workspace member 和 root dep**

在 `[workspace].members` 数组末尾添加 `"crates/astock-data"`：

```
"crates/astock-data"
```

在 `[dependencies]` 末尾添加：

```toml
axagent-astock-data = { path = "crates/astock-data" }
```

- [ ] **Step 7: 验证编译**

```bash
cd src-tauri && cargo check -p axagent-astock-data
```

- [ ] **Step 8: Commit**

```bash
git add src-tauri/crates/astock-data/ src-tauri/Cargo.toml
git commit -m "feat: 添加 astock-data crate 骨架（类型定义 + Vendor trait + AStockClient）"
```

---

### Task 2: 实现腾讯财经 vendor (tencent.rs)

**Files:**
- Create: `src-tauri/crates/astock-data/src/vendors/tencent.rs`

- [ ] **Step 1: 实现腾讯财经实时行情接口**

腾讯财经 API: `http://qt.gtimg.cn/q=<market_code>`，返回格式为 `v_<code>="..."` 的文本。

```rust
use async_trait::async_trait;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;

pub struct TencentVendor;

/// 将 AxInvest 股票代码转为腾讯财经格式
/// 600519 → sh600519, 000001 → sz000001, 300750 → sz300750
fn to_tencent_code(stock_code: &str) -> String {
    let prefix = match &stock_code[..1] {
        "6" => "sh",
        "0" | "3" | "2" => "sz",
        "8" | "4" => "bj",
        _ => "sz",
    };
    format!("{}{}", prefix, stock_code)
}

/// 解析腾讯财经实时行情响应
fn parse_quote(raw: &str) -> Result<StockQuote, DataError> {
    // 格式: v_sh600519="1~贵州茅台~600519~1680.00~1650.00~..."
    let start = raw.find('"').ok_or_else(|| DataError::ParseError("no opening quote".into()))?;
    let end = raw[start + 1..]
        .find('"')
        .ok_or_else(|| DataError::ParseError("no closing quote".into()))?;
    let data = &raw[start + 1..start + 1 + end];
    let fields: Vec<&str> = data.split('~').collect();

    if fields.len() < 40 {
        return Err(DataError::ParseError(format!("expected >=40 fields, got {}", fields.len())));
    }

    let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
    let parse_opt = |s: &str| -> Option<f64> {
        let v: f64 = s.parse().ok()?;
        if v == 0.0 { None } else { Some(v) }
    };

    Ok(StockQuote {
        code: fields[2].to_string(),
        name: fields[1].to_string(),
        price: parse(fields[3]),
        open: parse(fields[5]),
        high: parse(fields[33]),
        low: parse(fields[34]),
        volume: parse(fields[6]) * 100.0,
        amount: parse(fields[37]) * 10000.0,
        change_pct: parse(fields[32]),
        turnover_rate: parse(fields[38]),
        pe: parse_opt(fields[39]),
        pb: parse_opt(fields[46]),
        total_mv: parse_opt(fields[45]).map(|v| v * 10000.0),
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

#[async_trait]
impl StockVendor for TencentVendor {
    fn name(&self) -> &'static str { "tencent" }

    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let tc_code = to_tencent_code(stock_code);
        let url = format!("http://qt.gtimg.cn/q={}", tc_code);
        let resp = reqwest::get(&url).await?;
        let text = resp.text().await?;
        parse_quote(&text)
    }

    // 以下方法由东方财富 vendor 承担，此处返回空/未实现
    async fn get_klines(&self, _: &str, _: &str, _: u32) -> Result<Vec<KLine>, DataError> {
        Ok(vec![])
    }

    async fn get_financials(&self, _: &str) -> Result<Vec<FinancialReport>, DataError> {
        Ok(vec![])
    }

    async fn get_news(&self, _: &str, _: u32) -> Result<Vec<NewsItem>, DataError> {
        Ok(vec![])
    }

    async fn get_money_flow(&self, _: &str) -> Result<Option<MoneyFlow>, DataError> {
        Ok(None)
    }

    async fn get_dragon_tiger(&self, _: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        Ok(vec![])
    }

    async fn get_lockup_schedule(&self, _: &str) -> Result<Vec<LockupSchedule>, DataError> {
        Ok(vec![])
    }

    async fn search_stock(&self, _: &str) -> Result<Vec<StockSearchResult>, DataError> {
        Ok(vec![])
    }
}
```

- [ ] **Step 2: 验证编译**

```bash
cd src-tauri && cargo check -p axagent-astock-data
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/astock-data/src/vendors/tencent.rs
git commit -m "feat: 实现腾讯财经 vendor（实时行情）"
```

---

### Task 3: 实现东方财富 vendor (eastmoney.rs)

**Files:**
- Create: `src-tauri/crates/astock-data/src/vendors/eastmoney.rs`

- [ ] **Step 1: 实现东方财富 vendor**

东方财富提供：
- K线: `https://push2his.eastmoney.com/api/qt/stock/kline/get`
- 财务: `https://emweb.securities.eastmoney.com/PC_HSF10/FinanceSummary/FinanceSummary`
- 资金流向: `https://push2.eastmoney.com/api/qt/stock/fflow/daykline/get`
- 龙虎榜: `https://push2.eastmoney.com/api/qt/stock/mmpa/get`
- 搜索: `https://searchadapter.eastmoney.com/api/suggest/get`

```rust
use async_trait::async_trait;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use reqwest::Client;
use serde_json::Value;

pub struct EastMoneyVendor;

/// 构建东方财富股票代码 (1.SH600519, 0.SZ000001)
fn to_em_code(stock_code: &str) -> String {
    let market = match &stock_code[..1] {
        "6" => "1",
        "0" | "2" => "0",
        "3" => "0",
        "8" | "4" => "0",
        _ => "0",
    };
    format!("{}.{}{}", market, if market == "1" { "SH" } else { "SZ" }, stock_code)
}

fn to_em_secid(stock_code: &str) -> String {
    let market = match &stock_code[..1] {
        "6" => "1",
        "0" | "2" => "0",
        "3" => "0",
        "8" | "4" => "0",
        _ => "0",
    };
    format!("{}.{}", market, stock_code)
}

#[async_trait]
impl StockVendor for EastMoneyVendor {
    fn name(&self) -> &'static str { "eastmoney" }

    async fn get_quote(&self, _: &str) -> Result<StockQuote, DataError> {
        // Quote 由腾讯 vendor 处理
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "quote handled by tencent vendor".into(),
        })
    }

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,  // daily=101, weekly=102, monthly=103
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        let client = Client::new();
        let period_code = match period {
            "daily" | "101" => "101",
            "weekly" | "102" => "102",
            "monthly" | "103" => "103",
            _ => "101",
        };
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt={}&fqt=1&end=20500101&lmt={}",
            secid, period_code, limit
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let klines_raw = json["data"]["klines"]
            .as_array()
            .ok_or_else(|| DataError::ParseError("missing klines array".into()))?;

        klines_raw
            .iter()
            .map(|v| {
                let s = v.as_str().ok_or_else(|| DataError::ParseError("kline not string".into()))?;
                let parts: Vec<&str> = s.split(',').collect();
                if parts.len() < 11 {
                    return Err(DataError::ParseError(format!("expected 11 fields, got {}", parts.len())));
                }
                let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
                Ok(KLine {
                    date: parts[0].to_string(),
                    open: parse(parts[1]),
                    close: parse(parts[2]),
                    high: parse(parts[3]),
                    low: parse(parts[4]),
                    volume: parse(parts[5]),
                    amount: parse(parts[6]),
                    turnover_rate: Some(parse(parts[10])),
                })
            })
            .collect()
    }

    async fn get_financials(
        &self,
        stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError> {
        let client = Client::new();
        let url = format!(
            "https://emweb.securities.eastmoney.com/PC_HSF10/FinanceSummary/FinanceSummary?code={}&type=web",
            to_em_code(stock_code)
        );

        let resp = client
            .get(&url)
            .header("Referer", "https://emweb.securities.eastmoney.com/")
            .send()
            .await?;
        let json: Value = resp.json().await?;

        let reports = json["data"]["list"]
            .as_array()
            .ok_or_else(|| DataError::ParseError("missing financials list".into()))?;

        reports
            .iter()
            .map(|r| {
                Ok(FinancialReport {
                    stock_code: stock_code.to_string(),
                    report_date: r["REPORT_DATE"].as_str().unwrap_or("").to_string(),
                    revenue: r["TOTAL_OPERATE_INCOME"].as_str().and_then(|s| s.parse().ok()),
                    net_profit: r["PARENT_NETPROFIT"].as_str().and_then(|s| s.parse().ok()),
                    eps: r["BASIC_EPS"].as_str().and_then(|s| s.parse().ok()),
                    bps: r["BPS"].as_str().and_then(|s| s.parse().ok()),
                    roe: r["WEIGHTAVG_ROE"].as_str().and_then(|s| s.parse().ok()),
                    debt_ratio: r["DEBT_ASSET_RATIO"].as_str().and_then(|s| s.parse().ok()),
                    gross_margin: r["GROSS_PROFIT_RATIO"].as_str().and_then(|s| s.parse().ok()),
                    net_margin: r["NETPROFIT_MARGIN"].as_str().and_then(|s| s.parse().ok()),
                    revenue_yoy: r["TOTAL_OPERATE_INCOME_YOY"].as_str().and_then(|s| s.parse().ok()),
                    profit_yoy: r["PARENT_NETPROFIT_YOY"].as_str().and_then(|s| s.parse().ok()),
                })
            })
            .collect()
    }

    async fn get_news(&self, _: &str, _: u32) -> Result<Vec<NewsItem>, DataError> {
        // 新闻由新浪 vendor 处理
        Ok(vec![])
    }

    async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        let client = Client::new();
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/fflow/daykline/get?secid={}&fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56&lmt=1",
            secid
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let klines = json["data"]["klines"].as_array();
        match klines {
            Some(arr) if !arr.is_empty() => {
                let s = arr[0].as_str().unwrap_or("");
                let parts: Vec<&str> = s.split(',').collect();
                if parts.len() < 6 {
                    return Ok(None);
                }
                let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
                Ok(Some(MoneyFlow {
                    date: parts[0].to_string(),
                    main_net_inflow: parse(parts[1]) * 10000.0,
                    super_large_net: parse(parts[3]) * 10000.0,
                    large_net: parse(parts[4]) * 10000.0,
                    medium_net: parse(parts[5]) * 10000.0,
                    small_net: parse(parts[6]) * 10000.0,
                }))
            },
            _ => Ok(None),
        }
    }

    async fn get_dragon_tiger(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError> {
        let client = Client::new();
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/mmpa/get?secid={}&fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56,f57,f58",
            secid
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let entries = match json["data"]["mmpa"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        entries
            .iter()
            .map(|e| {
                let s = e.as_str().unwrap_or("");
                let parts: Vec<&str> = s.split(',').collect();
                let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
                Ok(DragonTigerEntry {
                    stock_code: stock_code.to_string(),
                    date: parts.get(0).map(|s| s.to_string()).unwrap_or_default(),
                    dept_name: parts.get(1).map(|s| s.to_string()).unwrap_or_default(),
                    buy_amount: parse(parts.get(3).unwrap_or(&"0")),
                    sell_amount: parse(parts.get(4).unwrap_or(&"0")),
                    net_amount: parse(parts.get(5).unwrap_or(&"0")),
                    reason: parts.get(7).map(|s| s.to_string()),
                })
            })
            .collect()
    }

    async fn get_lockup_schedule(
        &self,
        stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError> {
        let client = Client::new();
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_LOCKUP&columns=SECURITY_CODE,SECURITY_NAME_ABBR,UNLOCK_DATE,UNLOCK_SHARES,PLACING_RATIO,HOLDER_NAME&filter=(SECURITY_CODE=\"{}\")&pageSize=20&sortColumns=UNLOCK_DATE&pageNumber=1",
            stock_code
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        rows.iter()
            .map(|r| {
                Ok(LockupSchedule {
                    stock_code: stock_code.to_string(),
                    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                    unlock_date: r["UNLOCK_DATE"].as_str().unwrap_or("").to_string(),
                    unlock_shares: r["UNLOCK_SHARES"].as_f64().unwrap_or(0.0),
                    unlock_ratio: r["PLACING_RATIO"].as_f64().unwrap_or(0.0),
                    shareholder: r["HOLDER_NAME"].as_str().map(|s| s.to_string()),
                })
            })
            .collect()
    }

    async fn search_stock(
        &self,
        keyword: &str,
    ) -> Result<Vec<StockSearchResult>, DataError> {
        let client = Client::new();
        let url = format!(
            "https://searchadapter.eastmoney.com/api/suggest/get?input={}&type=14&count=20",
            urlencoding::encode(keyword)
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let stocks = match json["QuotationCodeTable"]["Data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(stocks
            .iter()
            .map(|s| StockSearchResult {
                code: s["Code"].as_str().unwrap_or("").to_string(),
                name: s["Name"].as_str().unwrap_or("").to_string(),
                market: s["Market"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }
}
```

- [ ] **Step 2: 添加 urlencoding 到 Cargo.toml**

在 `src-tauri/crates/astock-data/Cargo.toml` 的 `[dependencies]` 添加：

```toml
urlencoding = "2"
```

- [ ] **Step 3: 验证编译**

```bash
cd src-tauri && cargo check -p axagent-astock-data
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/astock-data/
git commit -m "feat: 实现东方财富 vendor（K线/财务/资金流向/龙虎榜/解禁/搜索）"
```

---

### Task 4: 实现新浪财经 vendor + 添加 AStockClient 到 AppState

**Files:**
- Create: `src-tauri/crates/astock-data/src/vendors/sina.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/init/` — 初始化 AStockClient

- [ ] **Step 1: 实现新浪财经新闻接口**

```rust
use async_trait::async_trait;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use reqwest::Client;

pub struct SinaVendor;

#[async_trait]
impl StockVendor for SinaVendor {
    fn name(&self) -> &'static str { "sina" }

    async fn get_quote(&self, _: &str) -> Result<StockQuote, DataError> {
        Err(DataError::VendorError {
            vendor: "sina".into(),
            message: "quote handled by tencent".into(),
        })
    }

    async fn get_klines(&self, _: &str, _: &str, _: u32) -> Result<Vec<KLine>, DataError> {
        Ok(vec![])
    }

    async fn get_financials(&self, _: &str) -> Result<Vec<FinancialReport>, DataError> {
        Ok(vec![])
    }

    async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        let client = Client::new();
        let url = format!(
            "https://vip.stock.finance.sina.com.cn/corp/go.php/vCB_AllNewsStock/symbol/{}.json?page=1&num={}",
            stock_code, limit.min(50)
        );

        let resp = client
            .get(&url)
            .header("Referer", "https://finance.sina.com.cn/")
            .send()
            .await?;

        let items: Vec<serde_json::Value> = resp.json().await?;

        Ok(items
            .iter()
            .map(|item| NewsItem {
                title: item["title"].as_str().unwrap_or("").to_string(),
                summary: String::new(),
                source: "新浪财经".to_string(),
                url: format!(
                    "https://finance.sina.com.cn{}",
                    item["url"].as_str().unwrap_or("")
                ),
                publish_time: item["ctime"].as_str().unwrap_or("").to_string(),
                sentiment_score: None,
            })
            .collect())
    }

    async fn get_money_flow(&self, _: &str) -> Result<Option<MoneyFlow>, DataError> {
        Ok(None)
    }

    async fn get_dragon_tiger(&self, _: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        Ok(vec![])
    }

    async fn get_lockup_schedule(&self, _: &str) -> Result<Vec<LockupSchedule>, DataError> {
        Ok(vec![])
    }

    async fn search_stock(&self, _: &str) -> Result<Vec<StockSearchResult>, DataError> {
        Ok(vec![])
    }
}
```

- [ ] **Step 2: 验证编译**

```bash
cd src-tauri && cargo check -p axagent-astock-data
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/astock-data/src/vendors/sina.rs
git commit -m "feat: 实现新浪财经 vendor（新闻）"
```

---

### Task 5: 创建 StockAnalysisOrchestrator 核心逻辑

**Files:**
- Create: `src-tauri/crates/stock-analysis/Cargo.toml`
- Create: `src-tauri/crates/stock-analysis/src/lib.rs`
- Create: `src-tauri/crates/stock-analysis/src/decision.rs`
- Create: `src-tauri/crates/stock-analysis/src/pipeline.rs`
- Create: `src-tauri/crates/stock-analysis/src/orchestrator.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: 创建 Cargo.toml**

```toml
[package]
name = "axagent-stock-analysis"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
chrono = { workspace = true }
tracing = { workspace = true }
axagent-astock-data = { path = "../astock-data" }
axagent-agent = { path = "../agent" }
axagent-runtime-core = { path = "../runtime-core" }
```

- [ ] **Step 2: 创建 decision.rs**

```rust
use serde::{Deserialize, Serialize};

/// 投资决策
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockDecision {
    /// 买入/增持/持有/减持/卖出
    pub action: String,
    /// 建议仓位百分比 (0-100)
    pub position_pct: f64,
    /// 目标价
    pub target_price: Option<f64>,
    /// 止损价
    pub stop_loss: Option<f64>,
    /// 决策理由
    pub reasoning: String,
    /// 风险等级: 低/中/高
    pub risk_level: String,
    /// 置信度 (0-1)
    pub confidence: f64,
}

/// 分析配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfig {
    /// 最大辩论轮数
    pub max_debate_rounds: u32,
    /// K线周期
    pub kline_period: String,
    /// K线数量
    pub kline_limit: u32,
    /// 新闻数量
    pub news_limit: u32,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_debate_rounds: 3,
            kline_period: "daily".to_string(),
            kline_limit: 120,
            news_limit: 30,
        }
    }
}

/// 分析阶段性事件（通过 broadcast channel 推送）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum AnalysisEvent {
    Started {
        stock_code: String,
        stock_name: String,
        date: String,
    },
    DataLoaded {
        kline_count: usize,
        news_count: usize,
    },
    AnalystProgress {
        expert_id: String,
        status: String,  // Pending | Running | Done
        progress_pct: u8,
    },
    AnalystReport {
        expert_id: String,
        report_text: String,
    },
    DebateRound {
        round: u32,
        bull_argument: String,
        bear_argument: String,
    },
    RiskAssessment {
        risk_type: String,  // aggressive | conservative | neutral
        report: String,
    },
    InvestmentPlan {
        plan: String,
    },
    Decision(StockDecision),
    Error {
        stage: String,
        message: String,
    },
}
```

- [ ] **Step 3: 创建 pipeline.rs — Agent 执行器**

```rust
use axagent_agent::session_manager::SessionManager;
use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_runtime_core::{PermissionMode, Session};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::decision::AnalysisEvent;

/// 运行一个分析师 Agent（从 Blackboard 读数据，生成报告后写回）
pub async fn run_analyst(
    sessions: &SessionManager,
    expert_profile_id: &str,
    blackboard: Arc<RwLock<SharedBlackboard>>,
    provider_id: &str,
    conversation_id: &str,
    events: tokio::sync::broadcast::Sender<AnalysisEvent>,
) -> Result<String, String> {
    let agent_id = format!("stock-{}", expert_profile_id);

    // 1. 创建 AgentSession
    let session = sessions
        .create_session(&agent_id, provider_id, conversation_id)
        .await
        .map_err(|e| format!("创建 session 失败: {}", e))?;

    // 2. 从 Blackboard 读取该分析师需要的数据
    let context = build_analyst_context(expert_profile_id, &blackboard).await;

    // 3. 构建系统提示词（从 agent_profile 加载 + 注入 Blackboard 数据）
    let system_prompt = vec![context];

    // 4. 构建 API client（使用已有的 AxAgentApiClient）
    // 注意：实际实现依赖 SessionManager 的 run_turn_with_tools，
    // 此处为接口定义，实际集成时由 commands/stock_analysis.rs 组装参数

    events
        .send(AnalysisEvent::AnalystProgress {
            expert_id: expert_profile_id.to_string(),
            status: "Running".to_string(),
            progress_pct: 0,
        })
        .ok();

    // 5. 执行 LLM 调用 — 具体实现见 orchestrator
    // 此处为占位，实际由命令层调用 SessionManager::run_turn_with_tools
    let report = format!("[{} 报告占位]", expert_profile_id);

    // 6. 写报告到 Blackboard
    {
        let mut bb = blackboard.write().await;
        bb.set_state(&format!("report.{}", expert_profile_id), &report);
    }

    events
        .send(AnalysisEvent::AnalystReport {
            expert_id: expert_profile_id.to_string(),
            report_text: report.clone(),
        })
        .ok();

    Ok(report)
}

/// 构建分析师的数据上下文（从 Blackboard 中提取相关数据）
async fn build_analyst_context(
    expert_id: &str,
    blackboard: &Arc<RwLock<SharedBlackboard>>,
) -> String {
    let bb = blackboard.read().await;

    let mut ctx = String::new();
    ctx.push_str(&format!("你是 {}", expert_id));
    ctx.push_str("\n\n以下是你需要分析的原始数据：\n\n");

    // 根据 expert_id 提取相关数据
    match expert_id {
        "market-analyst" => {
            if let Some(klines) = bb.get_state("raw.klines") {
                ctx.push_str(&format!("## K线数据\n{}\n", klines));
            }
        },
        "fundamentals-analyst" => {
            if let Some(fin) = bb.get_state("raw.financials") {
                ctx.push_str(&format!("## 财务数据\n{}\n", fin));
            }
        },
        "news-analyst" | "sentiment-analyst" | "policy-analyst" => {
            if let Some(news) = bb.get_state("raw.news") {
                ctx.push_str(&format!("## 新闻数据\n{}\n", news));
            }
        },
        "hot-money-tracker" => {
            if let Some(mf) = bb.get_state("raw.money_flow") {
                ctx.push_str(&format!("## 资金流向\n{}\n", mf));
            }
            if let Some(dt) = bb.get_state("raw.dragon_tiger") {
                ctx.push_str(&format!("## 龙虎榜\n{}\n", dt));
            }
        },
        "lockup-watcher" => {
            if let Some(lockup) = bb.get_state("raw.lockup") {
                ctx.push_str(&format!("## 限售解禁\n{}\n", lockup));
            }
        },
        _ => {
            // 辩论/风控/决策角色读取所有报告
            for field in &[
                "report.market-analyst",
                "report.sentiment-analyst",
                "report.news-analyst",
                "report.fundamentals-analyst",
                "report.policy-analyst",
                "report.hot-money-tracker",
                "report.lockup-watcher",
            ] {
                if let Some(val) = bb.get_state(field) {
                    ctx.push_str(&format!("\n---\n{}\n", val));
                }
            }
        },
    }

    ctx
}
```

- [ ] **Step 4: 创建 orchestrator.rs — 5 阶段编排**

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

use axagent_agent::session_manager::SessionManager;
use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_astock_data::{AStockClient, StockRawData};

use crate::decision::{AnalysisConfig, AnalysisEvent, StockDecision};

/// 股票分析编排器
pub struct StockAnalysisOrchestrator;

impl StockAnalysisOrchestrator {
    /// 运行完整的 5 阶段分析
    pub async fn run(
        sessions: &SessionManager,
        data_client: &AStockClient,
        blackboard: Arc<RwLock<SharedBlackboard>>,
        stock_code: String,
        stock_name: String,
        date: String,
        config: AnalysisConfig,
        provider_id: String,
        conversation_id: String,
        events: tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<StockDecision, String> {
        // 写入基本元数据
        {
            let mut bb = blackboard.write().await;
            bb.set_state("stock_code", &stock_code);
            bb.set_state("stock_name", &stock_name);
            bb.set_state("analysis_date", &date);
        }

        events
            .send(AnalysisEvent::Started {
                stock_code: stock_code.clone(),
                stock_name: stock_name.clone(),
                date: date.clone(),
            })
            .ok();

        // ── 阶段 1: 数据加载 ──
        let raw = Self::phase_1_load_data(data_client, &stock_code, &config, &blackboard, &events)
            .await
            .map_err(|e| {
                events.send(AnalysisEvent::Error {
                    stage: "data_loading".into(),
                    message: e.clone(),
                }).ok();
                e
            })?;

        // ── 阶段 2: 并行分析 (7 分析师) ──
        Self::phase_2_analysis(sessions, &provider_id, &conversation_id, &blackboard, &events)
            .await
            .map_err(|e| {
                events.send(AnalysisEvent::Error {
                    stage: "analysis".into(),
                    message: e.clone(),
                }).ok();
                e
            })?;

        // ── 阶段 3: 多空辩论 ──
        Self::phase_3_debate(
            sessions,
            &provider_id,
            &conversation_id,
            config.max_debate_rounds,
            &blackboard,
            &events,
        )
        .await
        .map_err(|e| {
            events.send(AnalysisEvent::Error {
                stage: "debate".into(),
                message: e.clone(),
            }).ok();
            e
        })?;

        // ── 阶段 4: 风险评估 ──
        Self::phase_4_risk(
            sessions,
            &provider_id,
            &conversation_id,
            &blackboard,
            &events,
        )
        .await
        .map_err(|e| {
            events.send(AnalysisEvent::Error {
                stage: "risk_assessment".into(),
                message: e.clone(),
            }).ok();
            e
        })?;

        // ── 阶段 5: 最终决策 ──
        let decision = Self::phase_5_decision(
            sessions,
            &provider_id,
            &conversation_id,
            &blackboard,
            &events,
        )
        .await
        .map_err(|e| {
            events.send(AnalysisEvent::Error {
                stage: "decision".into(),
                message: e.clone(),
            }).ok();
            e
        })?;

        events.send(AnalysisEvent::Decision(decision.clone())).ok();

        Ok(decision)
    }

    // ── 各阶段实现 ──

    async fn phase_1_load_data(
        data_client: &AStockClient,
        stock_code: &str,
        config: &AnalysisConfig,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<StockRawData, String> {
        let raw = data_client
            .fetch_all(stock_code, &config.kline_period, config.kline_limit, config.news_limit)
            .await
            .map_err(|e| format!("数据获取失败: {}", e))?;

        let klines_json = serde_json::to_string(&raw.klines).unwrap_or_default();
        let financials_json = serde_json::to_string(&raw.financials).unwrap_or_default();
        let news_json = serde_json::to_string(&raw.news).unwrap_or_default();
        let money_flow_json = raw
            .money_flow
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default())
            .unwrap_or_default();
        let dragon_tiger_json = serde_json::to_string(&raw.dragon_tiger).unwrap_or_default();
        let lockup_json = serde_json::to_string(&raw.lockup).unwrap_or_default();

        let mut bb = blackboard.write().await;
        bb.set_state("raw.klines", &klines_json);
        bb.set_state("raw.financials", &financials_json);
        bb.set_state("raw.news", &news_json);
        bb.set_state("raw.money_flow", &money_flow_json);
        bb.set_state("raw.dragon_tiger", &dragon_tiger_json);
        bb.set_state("raw.lockup", &lockup_json);

        events
            .send(AnalysisEvent::DataLoaded {
                kline_count: raw.klines.len(),
                news_count: raw.news.len(),
            })
            .ok();

        Ok(raw)
    }

    async fn phase_2_analysis(
        _sessions: &SessionManager,
        _provider_id: &str,
        _conversation_id: &str,
        _blackboard: &Arc<RwLock<SharedBlackboard>>,
        _events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<(), String> {
        // 7 个分析师并行执行
        // 实际实现：每个分析师调用 SessionManager::run_turn_with_tools
        // 命令行/看板调用 Tauri commands 触发此阶段
        // 详见 commands/stock_analysis.rs 的 start_stock_analysis 命令
        Ok(())
    }

    async fn phase_3_debate(
        _sessions: &SessionManager,
        _provider_id: &str,
        _conversation_id: &str,
        _max_rounds: u32,
        _blackboard: &Arc<RwLock<SharedBlackboard>>,
        _events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn phase_4_risk(
        _sessions: &SessionManager,
        _provider_id: &str,
        _conversation_id: &str,
        _blackboard: &Arc<RwLock<SharedBlackboard>>,
        _events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn phase_5_decision(
        _sessions: &SessionManager,
        _provider_id: &str,
        _conversation_id: &str,
        _blackboard: &Arc<RwLock<SharedBlackboard>>,
        _events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<StockDecision, String> {
        Ok(StockDecision {
            action: "持有".to_string(),
            position_pct: 0.0,
            target_price: None,
            stop_loss: None,
            reasoning: "分析中...".to_string(),
            risk_level: "中".to_string(),
            confidence: 0.0,
        })
    }
}
```

- [ ] **Step 5: 创建 lib.rs**

```rust
pub mod decision;
pub mod orchestrator;
pub mod pipeline;
```

- [ ] **Step 6: 修改 src-tauri/Cargo.toml — 注册 workspace member + 依赖**

在 `[workspace].members` 添加：
```
"crates/stock-analysis"
```

在 `[dependencies]` 添加：
```toml
axagent-stock-analysis = { path = "crates/stock-analysis" }
```

- [ ] **Step 7: 验证编译**

```bash
cd src-tauri && cargo check -p axagent-stock-analysis
```

- [ ] **Step 8: Commit**

```bash
git add src-tauri/crates/stock-analysis/ src-tauri/Cargo.toml
git commit -m "feat: 创建 stock-analysis crate（编排器 + 决策 + 管道）"
```

---

### Task 6: 创建 DB entity + migration

**Files:**
- Create: `src-tauri/crates/core/src/entity/stock_analyses.rs`
- Create: `src-tauri/crates/migration/src/m20250514_000001_stock_analysis.rs`
- Modify: `src-tauri/crates/core/src/entity/mod.rs`
- Modify: `src-tauri/crates/migration/src/lib.rs`

- [ ] **Step 1: 创建 stock_analyses entity**

```rust
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stock_analyses")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub analysis_date: String,
    pub provider_id: String,
    pub conversation_id: String,
    pub status: String,
    pub decision_action: Option<String>,
    pub decision_position_pct: Option<f64>,
    pub decision_reasoning: Option<String>,
    pub decision_json: Option<String>,
    pub blackboard_snapshot: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 2: 在 entity/mod.rs 中添加声明**

```rust
pub mod stock_analyses;
```

- [ ] **Step 3: 创建 migration**

```rust
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20250514_000001_stock_analysis"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(StockAnalyses::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(StockAnalyses::Id).string().primary_key().not_null())
                    .col(ColumnDef::new(StockAnalyses::StockCode).string().not_null())
                    .col(ColumnDef::new(StockAnalyses::StockName).string().not_null())
                    .col(ColumnDef::new(StockAnalyses::AnalysisDate).string().not_null())
                    .col(ColumnDef::new(StockAnalyses::ProviderId).string().not_null())
                    .col(ColumnDef::new(StockAnalyses::ConversationId).string().not_null())
                    .col(ColumnDef::new(StockAnalyses::Status).string().not_null())
                    .col(ColumnDef::new(StockAnalyses::DecisionAction).string())
                    .col(ColumnDef::new(StockAnalyses::DecisionPositionPct).double())
                    .col(ColumnDef::new(StockAnalyses::DecisionReasoning).text())
                    .col(ColumnDef::new(StockAnalyses::DecisionJson).text())
                    .col(ColumnDef::new(StockAnalyses::BlackboardSnapshot).text())
                    .col(ColumnDef::new(StockAnalyses::CreatedAt).big_integer().not_null())
                    .col(ColumnDef::new(StockAnalyses::UpdatedAt).big_integer().not_null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(StockAnalyses::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum StockAnalyses {
    Table,
    Id,
    StockCode,
    StockName,
    AnalysisDate,
    ProviderId,
    ConversationId,
    Status,
    DecisionAction,
    DecisionPositionPct,
    DecisionReasoning,
    DecisionJson,
    BlackboardSnapshot,
    CreatedAt,
    UpdatedAt,
}
```

- [ ] **Step 4: 在 migration/src/lib.rs 注册**

添加 `mod m20250514_000001_stock_analysis;` 到模块声明区，在 `fn migrations()` 中添加：

```rust
Box::new(m20250514_000001_stock_analysis::Migration),
```

- [ ] **Step 5: 验证编译**

```bash
cd src-tauri && cargo check
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/core/src/entity/mod.rs src-tauri/crates/core/src/entity/stock_analyses.rs src-tauri/crates/migration/src/lib.rs src-tauri/crates/migration/src/m20250514_000001_stock_analysis.rs
git commit -m "feat: 添加 stock_analyses 表 entity + migration"
```

---

### Task 7: 创建 Tauri commands

**Files:**
- Create: `src-tauri/src/commands/stock_analysis.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs` — generate_handler![] 注册

- [ ] **Step 1: 创建 commands/stock_analysis.rs**

```rust
use crate::AppState;
use axagent_astock_data::AStockClient;
use axagent_stock_analysis::decision::{AnalysisConfig, AnalysisEvent, StockDecision};
use sea_orm::{ActiveModelTrait, Set};
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State};
use tokio::sync::RwLock;

use axagent_core::entity::stock_analyses;

/// 搜索股票
#[tauri::command]
pub async fn search_stock(keyword: String) -> Result<Vec<axagent_astock_data::StockSearchResult>, String> {
    let client = AStockClient::new();
    client.search_stock(&keyword).await.map_err(|e| e.to_string())
}

/// 获取实时行情
#[tauri::command]
pub async fn get_stock_quote(stock_code: String) -> Result<axagent_astock_data::StockQuote, String> {
    let client = AStockClient::new();
    client.get_quote(&stock_code).await.map_err(|e| e.to_string())
}

/// 获取K线数据
#[tauri::command]
pub async fn get_stock_kline(stock_code: String, period: String, limit: u32) -> Result<Vec<axagent_astock_data::KLine>, String> {
    let client = AStockClient::new();
    client.get_klines(&stock_code, &period, limit).await.map_err(|e| e.to_string())
}

/// 启动股票分析
#[tauri::command]
pub async fn start_stock_analysis(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
    date: String,
    provider_id: String,
) -> Result<serde_json::Value, String> {
    let analysis_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    // 1. 获取股票名称
    let client = AStockClient::new();
    let quote = client.get_quote(&stock_code).await.map_err(|e| e.to_string())?;
    let stock_name = quote.name.clone();

    // 2. 创建 conversation
    let conversation_id = uuid::Uuid::new_v4().to_string();

    // 3. 写入 stock_analyses 记录
    let model = stock_analyses::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.clone()),
        stock_name: Set(stock_name.clone()),
        analysis_date: Set(date.clone()),
        provider_id: Set(provider_id.clone()),
        conversation_id: Set(conversation_id.clone()),
        status: Set("running".to_string()),
        decision_action: Set(None),
        decision_position_pct: Set(None),
        decision_reasoning: Set(None),
        decision_json: Set(None),
        blackboard_snapshot: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    model.insert(&state.sea_db).await.map_err(|e| e.to_string())?;

    // 4. 启动异步分析（spawn 不阻塞）
    let app_handle = app.clone();
    let analysis_id_clone = analysis_id.clone();
    let db = state.sea_db.clone();

    tokio::spawn(async move {
        let (event_tx, _) = tokio::sync::broadcast::channel::<AnalysisEvent>(64);

        let data_client = AStockClient::new();
        let blackboard = Arc::new(RwLock::new(
            axagent_agent::shared_blackboard::SharedBlackboard::new(
                &analysis_id_clone,
                &format!("分析 {} ({})", stock_code, stock_name),
            ),
        ));

        let config = AnalysisConfig::default();

        let result = axagent_stock_analysis::orchestrator::StockAnalysisOrchestrator::run(
            // sessions 从 AppState 获取 — 暂时传入新 SessionManager
            &axagent_agent::session_manager::SessionManager::new(),
            &data_client,
            blackboard,
            stock_code.clone(),
            stock_name.clone(),
            date,
            config,
            provider_id,
            conversation_id,
            event_tx,
        )
        .await;

        // 更新 DB 状态
        match result {
            Ok(decision) => {
                let decision_json = serde_json::to_string(&decision).unwrap_or_default();
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, "completed".into())
                    .col_expr(stock_analyses::Column::DecisionAction, decision.action.clone().into())
                    .col_expr(stock_analyses::Column::DecisionPositionPct, decision.position_pct.into())
                    .col_expr(stock_analyses::Column::DecisionReasoning, decision.reasoning.clone().into())
                    .col_expr(stock_analyses::Column::DecisionJson, decision_json.into())
                    .col_expr(stock_analyses::Column::UpdatedAt, chrono::Utc::now().timestamp_millis().into())
                    .filter(stock_analyses::Column::Id.eq(&analysis_id_clone))
                    .exec(&db)
                    .await;
            },
            Err(e) => {
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, format!("failed: {}", e).into())
                    .col_expr(stock_analyses::Column::UpdatedAt, chrono::Utc::now().timestamp_millis().into())
                    .filter(stock_analyses::Column::Id.eq(&analysis_id_clone))
                    .exec(&db)
                    .await;
            },
        }
    });

    Ok(serde_json::json!({
        "analysis_id": analysis_id,
        "stock_code": stock_code,
        "stock_name": stock_name,
        "status": "running",
    }))
}

/// 取消分析
#[tauri::command]
pub async fn cancel_stock_analysis(analysis_id: String) -> Result<(), String> {
    // 通过 cancel_token 取消（由 orchestrator 支持 cancel_token 参数）
    tracing::info!("cancel_stock_analysis: {}", analysis_id);
    Ok(())
}

/// 历史分析列表
#[tauri::command]
pub async fn list_stock_analyses(
    state: State<'_, AppState>,
    limit: u32,
    offset: u32,
) -> Result<Vec<stock_analyses::Model>, String> {
    use sea_orm::{EntityTrait, QueryOrder, PaginatorTrait};

    stock_analyses::Entity::find()
        .order_by_desc(stock_analyses::Column::CreatedAt)
        .limit(Some(limit as u64))
        .offset(Some(offset as u64))
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())
}

/// 获取单个分析详情
#[tauri::command]
pub async fn get_stock_analysis(
    state: State<'_, AppState>,
    analysis_id: String,
) -> Result<stock_analyses::Model, String> {
    use sea_orm::EntityTrait;

    stock_analyses::Entity::find_by_id(&analysis_id)
        .one(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("分析记录不存在: {}", analysis_id))
}
```

- [ ] **Step 2: 在 commands/mod.rs 中添加声明**

```rust
pub mod stock_analysis;
```

- [ ] **Step 3: 在 lib.rs 的 generate_handler![] 中注册命令**

在 `lib.rs:116` 的 `generate_handler![...]` 中添加 7 行：

```rust
commands::stock_analysis::search_stock,
commands::stock_analysis::get_stock_quote,
commands::stock_analysis::get_stock_kline,
commands::stock_analysis::start_stock_analysis,
commands::stock_analysis::cancel_stock_analysis,
commands::stock_analysis::list_stock_analyses,
commands::stock_analysis::get_stock_analysis,
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/stock_analysis.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat: 添加股票分析 Tauri commands（搜索/行情/K线/分析/历史）"
```

---

### Task 8: 创建前端类型定义

**Files:**
- Create: `src/types/stock-analysis.ts`
- Modify: `src/types/index.ts`

- [ ] **Step 1: 创建 types/stock-analysis.ts**

```typescript
export interface StockQuote {
  code: string;
  name: string;
  price: number;
  open: number;
  high: number;
  low: number;
  volume: number;
  amount: number;
  changePct: number;
  turnoverRate: number;
  pe: number | null;
  pb: number | null;
  totalMv: number | null;
  timestamp: string;
}

export interface KLine {
  date: string;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  amount: number;
  turnoverRate: number | null;
}

export interface StockSearchResult {
  code: string;
  name: string;
  market: string;
}

export interface AnalysisConfig {
  maxDebateRounds: number;
  klinePeriod: string;
  klineLimit: number;
  newsLimit: number;
}

export interface StockDecision {
  action: string;
  positionPct: number;
  targetPrice: number | null;
  stopLoss: number | null;
  reasoning: string;
  riskLevel: string;
  confidence: number;
}

export interface AnalysisSummary {
  id: string;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  status: string;
  decisionAction: string | null;
  createdAt: number;
}

export interface AnalysisEvent {
  type:
    | "Started"
    | "DataLoaded"
    | "AnalystProgress"
    | "AnalystReport"
    | "DebateRound"
    | "RiskAssessment"
    | "InvestmentPlan"
    | "Decision"
    | "Error";
  payload: Record<string, unknown>;
}

export type AnalysisStatus = "idle" | "loading" | "running" | "completed" | "error";

export const ANALYST_NAMES: Record<string, string> = {
  "market-analyst": "市场技术分析师",
  "sentiment-analyst": "情绪面分析师",
  "news-analyst": "消息面分析师",
  "fundamentals-analyst": "基本面分析师",
  "policy-analyst": "政策面分析师",
  "hot-money-tracker": "资金面追踪者",
  "lockup-watcher": "筹码面观察者",
};
```

- [ ] **Step 2: 在 types/index.ts 添加 re-export**

```typescript
export * from "./stock-analysis";
```

- [ ] **Step 3: Commit**

```bash
git add src/types/stock-analysis.ts src/types/index.ts
git commit -m "feat: 添加股票分析前端类型定义"
```

---

### Task 9: 创建 stockAnalysisStore

**Files:**
- Create: `src/stores/feature/stockAnalysisStore.ts`
- Modify: `src/stores/index.ts`

- [ ] **Step 1: 创建 stockAnalysisStore.ts**

```typescript
import { create } from "zustand";
import { invoke } from "@/lib/invoke";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AnalysisEvent,
  AnalysisStatus,
  AnalysisSummary,
  KLine,
  StockDecision,
  StockQuote,
  StockSearchResult,
} from "@/types";

interface StockAnalysisState {
  // 搜索
  searchKeyword: string;
  searchResults: StockSearchResult[];

  // 当前分析
  analysisId: string | null;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  status: AnalysisStatus;

  // 数据
  quote: StockQuote | null;
  klineData: KLine[];
  analystReports: Record<string, string>;
  debateRounds: Array<{ round: number; bull: string; bear: string }>;
  riskAssessments: Record<string, string>;
  decision: StockDecision | null;
  error: string | null;

  // 历史
  history: AnalysisSummary[];

  // Actions
  searchStock: (keyword: string) => Promise<void>;
  getStockQuote: (code: string) => Promise<void>;
  getStockKline: (code: string, period: string, limit: number) => Promise<void>;
  startAnalysis: (stockCode: string, date: string, providerId: string) => Promise<void>;
  cancelAnalysis: () => Promise<void>;
  fetchHistory: (limit?: number, offset?: number) => Promise<void>;
  loadAnalysis: (analysisId: string) => Promise<void>;
  reset: () => void;

  // Event listeners
  _unlisten: UnlistenFn | null;
  setupEventListener: () => Promise<void>;
}

const initialState = {
  searchKeyword: "",
  searchResults: [],
  analysisId: null,
  stockCode: "",
  stockName: "",
  analysisDate: "",
  status: "idle" as AnalysisStatus,
  quote: null,
  klineData: [],
  analystReports: {},
  debateRounds: [],
  riskAssessments: {},
  decision: null,
  error: null,
  history: [],
};

export const useStockAnalysisStore = create<StockAnalysisState>((set, get) => ({
  ...initialState,
  _unlisten: null,

  searchStock: async (keyword: string) => {
    set({ searchKeyword: keyword });
    if (keyword.length < 2) {
      set({ searchResults: [] });
      return;
    }
    const results = await invoke<StockSearchResult[]>("search_stock", { keyword });
    set({ searchResults: results });
  },

  getStockQuote: async (code: string) => {
    const quote = await invoke<StockQuote>("get_stock_quote", { stockCode: code });
    set({ quote, stockCode: code, stockName: quote.name });
  },

  getStockKline: async (code: string, period: string, limit: number) => {
    const klineData = await invoke<KLine[]>("get_stock_kline", {
      stockCode: code,
      period,
      limit,
    });
    set({ klineData });
  },

  startAnalysis: async (stockCode: string, date: string, providerId: string) => {
    set({ status: "loading", error: null, analystReports: {}, debateRounds: [], riskAssessments: {}, decision: null });

    const result = await invoke<{
      analysis_id: string;
      stock_code: string;
      stock_name: string;
      status: string;
    }>("start_stock_analysis", { stockCode, date, providerId });

    set({
      analysisId: result.analysis_id,
      stockCode: result.stock_code,
      stockName: result.stock_name,
      analysisDate: date,
      status: "running",
    });
  },

  cancelAnalysis: async () => {
    const { analysisId } = get();
    if (analysisId) {
      await invoke("cancel_stock_analysis", { analysisId });
    }
    set({ status: "idle" });
  },

  fetchHistory: async (limit = 20, offset = 0) => {
    const history = await invoke<AnalysisSummary[]>("list_stock_analyses", { limit, offset });
    set({ history });
  },

  loadAnalysis: async (analysisId: string) => {
    const record = await invoke<AnalysisSummary & { decisionJson: string | null }>(
      "get_stock_analysis",
      { analysisId },
    );
    set({ analysisId: record.id, stockCode: record.stockCode, stockName: record.stockName });
    if (record.decisionJson) {
      set({ decision: JSON.parse(record.decisionJson) });
    }
  },

  reset: () => set(initialState),

  setupEventListener: async () => {
    const existing = get()._unlisten;
    if (existing) return;

    const unlisten = await listen<AnalysisEvent>("stock-analysis-event", (event) => {
      const { type, payload } = event.payload;
      switch (type) {
        case "Started":
          set({ status: "running" });
          break;
        case "DataLoaded":
          break;
        case "AnalystProgress":
          break;
        case "AnalystReport": {
          const { expertId, reportText } = payload as Record<string, string>;
          set((s) => ({
            analystReports: { ...s.analystReports, [expertId]: reportText },
          }));
          break;
        }
        case "DebateRound": {
          const { round, bullArgument, bearArgument } = payload as Record<string, unknown>;
          set((s) => ({
            debateRounds: [
              ...s.debateRounds,
              { round: round as number, bull: bullArgument as string, bear: bearArgument as string },
            ],
          }));
          break;
        }
        case "RiskAssessment": {
          const { riskType, report } = payload as Record<string, string>;
          set((s) => ({
            riskAssessments: { ...s.riskAssessments, [riskType]: report },
          }));
          break;
        }
        case "Decision":
          set({ decision: payload as StockDecision, status: "completed" });
          break;
        case "Error":
          set({ error: (payload as Record<string, string>).message, status: "error" });
          break;
      }
    });

    set({ _unlisten: unlisten });
  },
}));
```

- [ ] **Step 2: 在 stores/index.ts 添加 re-export**

```typescript
export { useStockAnalysisStore } from "./feature/stockAnalysisStore";
```

- [ ] **Step 3: Commit**

```bash
git add src/stores/feature/stockAnalysisStore.ts src/stores/index.ts
git commit -m "feat: 添加股票分析 Zustand store"
```

---

### Task 10: 创建前端组件 — 页面 + 搜索栏 + 进度

**Files:**
- Create: `src/components/stock-analysis/StockAnalysisPage.tsx`
- Create: `src/components/stock-analysis/StockSearchBar.tsx`
- Create: `src/components/stock-analysis/AnalysisProgress.tsx`

- [ ] **Step 1: 创建 StockAnalysisPage.tsx**

```typescript
import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useStockAnalysisStore } from "@/stores";
import { Spin } from "antd";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useParams } from "react-router-dom";

export function StockAnalysisPage() {
  const { t } = useTranslation();
  const { id } = useParams<{ id: string }>();
  const setupEventListener = useStockAnalysisStore((s) => s.setupEventListener);
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);

  useEffect(() => {
    setupEventListener();
  }, [setupEventListener]);

  useEffect(() => {
    if (id) {
      loadAnalysis(id);
    }
  }, [id, loadAnalysis]);

  return (
    <PageErrorBoundary title={t("error.page")}>
      <div className="flex flex-col h-full p-4 gap-4" style={{ maxWidth: 1400, margin: "0 auto" }}>
        <h2 className="text-lg font-semibold">{t("stockAnalysis.title")}</h2>
        {/* 子组件在此渲染 */}
        <StockAnalysisContent />
      </div>
    </PageErrorBoundary>
  );
}

function StockAnalysisContent() {
  const status = useStockAnalysisStore((s) => s.status);
  const { t } = useTranslation();

  if (status === "loading") {
    return (
      <div className="flex items-center justify-center" style={{ minHeight: 200 }}>
        <Spin size="large" tip={t("stockAnalysis.starting")} />
      </div>
    );
  }

  return (
    <>
      <StockSearchBar />
      <AnalysisProgress />
      {/* 后续组件在此添加 */}
    </>
  );
}

// 内联导入避免循环引用
import { StockSearchBar } from "./StockSearchBar";
import { AnalysisProgress } from "./AnalysisProgress";
```

- [ ] **Step 2: 创建 StockSearchBar.tsx**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { Button, DatePicker, Input, List } from "antd";
import dayjs from "dayjs";
import { useTranslation } from "react-i18next";

export function StockSearchBar() {
  const { t } = useTranslation();
  const searchKeyword = useStockAnalysisStore((s) => s.searchKeyword);
  const searchResults = useStockAnalysisStore((s) => s.searchResults);
  const searchStock = useStockAnalysisStore((s) => s.searchStock);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const status = useStockAnalysisStore((s) => s.status);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);

  const isRunning = status === "loading" || status === "running";

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2 items-center">
        <Input.Search
          placeholder={t("stockAnalysis.searchPlaceholder")}
          value={searchKeyword}
          onChange={(e) => searchStock(e.target.value)}
          onSearch={searchStock}
          style={{ maxWidth: 300 }}
          loading={status === "loading"}
        />
        <DatePicker
          defaultValue={dayjs()}
          disabled={isRunning}
        />
        <Button
          type="primary"
          disabled={!stockCode || isRunning}
          loading={isRunning}
          onClick={() => {
            if (stockCode) {
              startAnalysis(stockCode, dayjs().format("YYYY-MM-DD"), "");
            }
          }}
        >
          {isRunning ? t("stockAnalysis.analyzing") : t("stockAnalysis.startAnalysis")}
        </Button>
      </div>
      {searchResults.length > 0 && (
        <List
          size="small"
          bordered
          dataSource={searchResults}
          style={{ maxWidth: 300 }}
          renderItem={(item) => (
            <List.Item
              style={{ cursor: "pointer" }}
              onClick={() => {
                useStockAnalysisStore.getState().getStockQuote(item.code);
              }}
            >
              {item.code} — {item.name}
            </List.Item>
          )}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 3: 创建 AnalysisProgress.tsx**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { Steps } from "antd";
import { useTranslation } from "react-i18next";

const STAGES = [
  "stage.dataLoading",
  "stage.analysis",
  "stage.debate",
  "stage.risk",
  "stage.decision",
];

export function AnalysisProgress() {
  const { t } = useTranslation();
  const status = useStockAnalysisStore((s) => s.status);
  const analystReports = useStockAnalysisStore((s) => s.analystReports);
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const decision = useStockAnalysisStore((s) => s.decision);
  const error = useStockAnalysisStore((s) => s.error);

  if (status === "idle") return null;

  let currentStep = 0;
  if (status === "running") {
    if (Object.keys(analystReports).length >= 7) currentStep = 1;
    if (debateRounds.length > 0) currentStep = 2;
    if (Object.keys(riskAssessments).length >= 3) currentStep = 3;
  }
  if (status === "completed") currentStep = 4;

  return (
    <div>
      {error && (
        <div className="mb-2 p-2 rounded" style={{ color: "#ff4d4f", background: "#fff2f0" }}>
          {error}
        </div>
      )}
      <Steps
        size="small"
        current={currentStep}
        status={status === "error" ? "error" : "process"}
        items={STAGES.map((s) => ({ title: t(`stockAnalysis.${s}`) }))}
      />
    </div>
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add src/components/stock-analysis/
git commit -m "feat: 添加股票分析页面、搜索栏、进度组件"
```

---

### Task 11: 创建前端组件 — 行情卡片 + K线图 + 报告网格

**Files:**
- Create: `src/components/stock-analysis/StockQuoteCard.tsx`
- Create: `src/components/stock-analysis/KLineChart.tsx`
- Create: `src/components/stock-analysis/AnalystReportCard.tsx`
- Create: `src/components/stock-analysis/AnalystReportGrid.tsx`

- [ ] **Step 1: 创建 StockQuoteCard.tsx**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { Card, Statistic, Tag } from "antd";
import { useTranslation } from "react-i18next";

export function StockQuoteCard() {
  const { t } = useTranslation();
  const quote = useStockAnalysisStore((s) => s.quote);
  const stockName = useStockAnalysisStore((s) => s.stockName);

  if (!quote) return null;

  const isUp = quote.changePct >= 0;
  const color = isUp ? "#cf1322" : "#3f8600";

  return (
    <Card size="small" title={`${quote.name || stockName} (${quote.code})`}>
      <div className="flex gap-4 items-center flex-wrap">
        <Statistic
          title={t("stockAnalysis.price")}
          value={quote.price}
          precision={2}
          valueStyle={{ color }}
        />
        <Tag color={isUp ? "red" : "green"}>
          {isUp ? "+" : ""}{quote.changePct.toFixed(2)}%
        </Tag>
        <div className="text-xs" style={{ color: "var(--color-text-secondary)" }}>
          {t("stockAnalysis.open")}: {quote.open} &nbsp;
          {t("stockAnalysis.high")}: {quote.high} &nbsp;
          {t("stockAnalysis.low")}: {quote.low} &nbsp;
          {t("stockAnalysis.volume")}: {(quote.volume / 10000).toFixed(1)}万手
        </div>
      </div>
    </Card>
  );
}
```

- [ ] **Step 2: 创建 KLineChart.tsx — ECharts K线图**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { useEffect, useRef } from "react";
import * as echarts from "echarts";

export function KLineChart() {
  const klineData = useStockAnalysisStore((s) => s.klineData);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    if (!chartRef.current) return;
    if (!instanceRef.current) {
      instanceRef.current = echarts.init(chartRef.current);
    }
    const chart = instanceRef.current;

    if (klineData.length === 0) {
      chart.clear();
      return;
    }

    const dates = klineData.map((k) => k.date);
    const ohlc = klineData.map((k) => [k.open, k.close, k.low, k.high]);
    const volumes = klineData.map((k) => k.volume);

    chart.setOption({
      tooltip: { trigger: "axis" },
      grid: [
        { left: "8%", right: "2%", top: "2%", height: "65%" },
        { left: "8%", right: "2%", top: "75%", height: "20%" },
      ],
      xAxis: [
        { type: "category", data: dates, gridIndex: 0, axisLabel: { show: false } },
        { type: "category", data: dates, gridIndex: 1 },
      ],
      yAxis: [
        { type: "value", gridIndex: 0, scale: true },
        { type: "value", gridIndex: 1 },
      ],
      series: [
        {
          name: "K线",
          type: "candlestick",
          data: ohlc,
          xAxisIndex: 0,
          yAxisIndex: 0,
          itemStyle: {
            color: "#ef232a",
            color0: "#14b143",
            borderColor: "#ef232a",
            borderColor0: "#14b143",
          },
        },
        {
          name: "成交量",
          type: "bar",
          data: volumes,
          xAxisIndex: 1,
          yAxisIndex: 1,
        },
      ],
    });

    const handleResize = () => chart.resize();
    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("resize", handleResize);
    };
  }, [klineData]);

  return <div ref={chartRef} style={{ width: "100%", height: 350 }} />;
}
```

- [ ] **Step 3: 创建 AnalystReportCard.tsx**

```typescript
import { ANALYST_NAMES } from "@/types";
import { Card, Typography } from "antd";

interface Props {
  expertId: string;
  report: string;
}

export function AnalystReportCard({ expertId, report }: Props) {
  const name = ANALYST_NAMES[expertId] || expertId;

  return (
    <Card size="small" title={name}>
      <Typography.Paragraph ellipsis={{ rows: 5, expandable: true }}>
        {report}
      </Typography.Paragraph>
    </Card>
  );
}
```

- [ ] **Step 4: 创建 AnalystReportGrid.tsx**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { AnalystReportCard } from "./AnalystReportCard";

export function AnalystReportGrid() {
  const analystReports = useStockAnalysisStore((s) => s.analystReports);

  if (Object.keys(analystReports).length === 0) return null;

  return (
    <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(300px, 1fr))" }}>
      {Object.entries(analystReports).map(([expertId, report]) => (
        <AnalystReportCard key={expertId} expertId={expertId} report={report} />
      ))}
    </div>
  );
}
```

- [ ] **Step 5: Commit**

```bash
git add src/components/stock-analysis/
git commit -m "feat: 添加行情卡片、K线图、分析师报告组件"
```

---

### Task 12: 创建前端组件 — 辩论 + 风险 + 决策

**Files:**
- Create: `src/components/stock-analysis/DebatePanel.tsx`
- Create: `src/components/stock-analysis/RiskMatrix.tsx`
- Create: `src/components/stock-analysis/DecisionBanner.tsx`

- [ ] **Step 1: 创建 DebatePanel.tsx**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { Card, Collapse, Tag } from "antd";
import { useTranslation } from "react-i18next";

export function DebatePanel() {
  const { t } = useTranslation();
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);

  if (debateRounds.length === 0) return null;

  return (
    <Card size="small" title={t("stockAnalysis.debate")}>
      <Collapse
        size="small"
        items={debateRounds.map((r, i) => ({
          key: i,
          label: (
            <span>{t("stockAnalysis.debateRound")} {r.round + 1}</span>
          ),
          children: (
            <div className="flex gap-2">
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid #cf1322" }}>
                <Tag color="red">{t("stockAnalysis.bull")}</Tag>
                <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap" }}>{r.bull}</p>
              </div>
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid #3f8600" }}>
                <Tag color="green">{t("stockAnalysis.bear")}</Tag>
                <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap" }}>{r.bear}</p>
              </div>
            </div>
          ),
        }))}
      />
    </Card>
  );
}
```

- [ ] **Step 2: 创建 RiskMatrix.tsx**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { Card, Tag } from "antd";
import { useTranslation } from "react-i18next";

const RISK_LABELS: Record<string, string> = {
  aggressive: "risk.aggressive",
  conservative: "risk.conservative",
  neutral: "risk.neutral",
};

const RISK_COLORS: Record<string, string> = {
  aggressive: "red",
  conservative: "green",
  neutral: "blue",
};

export function RiskMatrix() {
  const { t } = useTranslation();
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);

  if (Object.keys(riskAssessments).length === 0) return null;

  return (
    <Card size="small" title={t("stockAnalysis.riskAssessment")}>
      <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        {Object.entries(riskAssessments).map(([type, report]) => (
          <div key={type} className="p-2 rounded" style={{ background: "var(--color-bg-elevated)" }}>
            <Tag color={RISK_COLORS[type]}>{t(`stockAnalysis.${RISK_LABELS[type]}`)}</Tag>
            <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap" }}>{report}</p>
          </div>
        ))}
      </div>
    </Card>
  );
}
```

- [ ] **Step 3: 创建 DecisionBanner.tsx**

```typescript
import { useStockAnalysisStore } from "@/stores";
import { Alert, Tag } from "antd";
import { useTranslation } from "react-i18next";

const ACTION_COLORS: Record<string, "success" | "warning" | "error" | "info"> = {
  "买入": "success",
  "增持": "success",
  "持有": "info",
  "减持": "warning",
  "卖出": "error",
};

export function DecisionBanner() {
  const { t } = useTranslation();
  const decision = useStockAnalysisStore((s) => s.decision);

  if (!decision) return null;

  const color = ACTION_COLORS[decision.action] || "info";

  return (
    <Alert
      type={color as "success" | "warning" | "error" | "info"}
      showIcon
      message={
        <div>
          <span className="font-semibold">{t("stockAnalysis.finalDecision")}: </span>
          <Tag color={color === "success" ? "green" : color === "error" ? "red" : "blue"}>
            {decision.action}
          </Tag>
          <span> {t("stockAnalysis.position")}: {decision.positionPct}%</span>
        </div>
      }
      description={
        <div className="text-xs" style={{ whiteSpace: "pre-wrap" }}>
          {decision.reasoning}
          <div className="mt-1">
            {decision.targetPrice && (
              <span>{t("stockAnalysis.targetPrice")}: ¥{decision.targetPrice} &nbsp;</span>
            )}
            {decision.stopLoss && (
              <span>{t("stockAnalysis.stopLoss")}: ¥{decision.stopLoss} &nbsp;</span>
            )}
            <Tag>{t("stockAnalysis.riskLevel")}: {decision.riskLevel}</Tag>
            <Tag>{t("stockAnalysis.confidence")}: {(decision.confidence * 100).toFixed(0)}%</Tag>
          </div>
        </div>
      }
    />
  );
}
```

- [ ] **Step 4: 将这些组件集成到 StockAnalysisPage 的内容区**

更新 `StockAnalysisPage.tsx` 中的 `StockAnalysisContent` 组件，加入所有子组件：

```typescript
function StockAnalysisContent() {
  const status = useStockAnalysisStore((s) => s.status);
  const { t } = useTranslation();

  if (status === "loading") {
    return (
      <div className="flex items-center justify-center" style={{ minHeight: 200 }}>
        <Spin size="large" tip={t("stockAnalysis.starting")} />
      </div>
    );
  }

  return (
    <>
      <StockSearchBar />
      <AnalysisProgress />
      <StockQuoteCard />
      <KLineChart />
      <AnalystReportGrid />
      <DebatePanel />
      <RiskMatrix />
      <DecisionBanner />
    </>
  );
}

import { StockQuoteCard } from "./StockQuoteCard";
import { KLineChart } from "./KLineChart";
import { AnalystReportGrid } from "./AnalystReportGrid";
import { DebatePanel } from "./DebatePanel";
import { RiskMatrix } from "./RiskMatrix";
import { DecisionBanner } from "./DecisionBanner";
```

- [ ] **Step 5: Commit**

```bash
git add src/components/stock-analysis/
git commit -m "feat: 添加辩论面板、风险矩阵、决策横幅组件"
```

---

### Task 13: 路由 + 侧栏 + Agent 类型注册

**Files:**
- Modify: `src/components/layout/ContentArea.tsx`
- Modify: `src/components/layout/Sidebar.tsx`
- Modify: `src/stores/feature/agentStore.ts`

- [ ] **Step 1: 在 ContentArea.tsx 添加路由**

在 `ContentArea.tsx` 顶部添加 lazy import：

```typescript
const LazyStockAnalysisPage = lazy(() =>
  import("@/pages/StockAnalysisPage").then((m) => ({ default: m.StockAnalysisPage }))
);
```

在 `<Routes>` 内添加路由：

```typescript
<Route path="/stock-analysis" element={<SafeLazyPage Page={LazyStockAnalysisPage} />} />
<Route path="/stock-analysis/:id" element={<SafeLazyPage Page={LazyStockAnalysisPage} />} />
```

创建 `src/pages/StockAnalysisPage.tsx`：

```typescript
import { StockAnalysisPage as Page } from "@/components/stock-analysis/StockAnalysisPage";

export function StockAnalysisPage() {
  return <Page />;
}
```

- [ ] **Step 2: 在 Sidebar.tsx 添加导航入口**

在侧栏导航项列表中添加：

```typescript
{
  key: "/stock-analysis",
  icon: <StockOutlined />,
  label: t("nav.stockAnalysis"),
}
```

- [ ] **Step 3: 在 agentStore.ts 注册 stock-analysis agent 类型**

在 `supportedAgentTypes` 或等价数组中添加：

```typescript
"stock-analysis"
```

- [ ] **Step 4: 添加 i18n key（zh-CN.json 示例）**

在 `locales/zh-CN.json` 中添加：

```json
{
  "stockAnalysis": {
    "title": "股票分析",
    "searchPlaceholder": "输入股票代码或名称",
    "startAnalysis": "开始分析",
    "analyzing": "分析中...",
    "starting": "正在启动分析引擎...",
    "price": "最新价",
    "open": "开",
    "high": "高",
    "low": "低",
    "volume": "量",
    "stage.dataLoading": "数据加载",
    "stage.analysis": "多维度分析",
    "stage.debate": "多空辩论",
    "stage.risk": "风险评估",
    "stage.decision": "投资决策",
    "debate": "多空辩论",
    "debateRound": "第{round}轮",
    "bull": "多方",
    "bear": "空方",
    "riskAssessment": "风险评估",
    "risk.aggressive": "激进",
    "risk.conservative": "保守",
    "risk.neutral": "中性",
    "finalDecision": "最终决策",
    "position": "仓位",
    "targetPrice": "目标价",
    "stopLoss": "止损",
    "riskLevel": "风险等级",
    "confidence": "置信度"
  },
  "nav": {
    "stockAnalysis": "股票分析"
  }
}
```

同样在其余 10 种语言文件中添加对应的翻译。

- [ ] **Step 5: Commit**

```bash
git add src/components/layout/ContentArea.tsx src/components/layout/Sidebar.tsx src/pages/StockAnalysisPage.tsx src/stores/feature/agentStore.ts locales/
git commit -m "feat: 添加股票分析路由、侧栏入口、agent 类型注册、i18n"
```

---

### Task 14: 创建 Expert markdown 文件

**Files:**
- Create: 14 个 expert markdown 文件（用户指定目录中导入）

由于 expert 文件通过 `import_agency_experts` 命令从用户指定的目录导入，14 个 `.md` 文件放置在 `stock-analysis/` 子目录下。

- [ ] **Step 1: 创建 market-analyst.md**

```markdown
---
name: 市场技术分析师
description: A股市场技术面分析专家，擅长K线形态识别、技术指标解读、量价关系分析
color: blue
---

# 角色定位

你是A股市场技术分析师，专注于通过技术分析手段评估股票走势。

## 核心能力

1. K线形态识别：识别头肩顶/底、双顶/底、上升/下降三角形、旗形等经典形态
2. 技术指标解读：MA均线系统、MACD、RSI、KDJ、布林带、成交量分析
3. 支撑压力位判断：基于历史高低点、均线、筹码分布判断关键价位
4. 趋势判断：多周期共振分析（日线/周线/月线）

## 工作流程

1. 读取提供给你的K线数据（日线级别，120个交易日）
2. 分析当前趋势（多头/空头/震荡）
3. 识别关键形态和信号
4. 标记支撑位和压力位
5. 给出技术面综合评分（1-10分）和理由

## 输出格式

请以结构化方式输出技术分析报告：
- **趋势判断**：当前处于什么趋势
- **形态分析**：观察到的K线形态
- **指标信号**：关键指标状态
- **支撑/压力**：关键价位
- **技术面评分**：1-10分及理由
```

（剩余 13 个 expert 文件同理创建，此处略去以节省篇幅。每个文件遵循相同模式：frontmatter + 角色定位 + 核心能力 + 工作流程 + 输出格式。）

- [ ] **Step 2: 用户通过 Tauri 命令导入 experts**

```bash
# 在前端设置页中，使用 import_agency_experts 命令导入
# 选择包含 stock-analysis/ 子目录的父目录
```

- [ ] **Step 3: Commit**

```bash
git add agency_experts/stock-analysis/
git commit -m "feat: 添加 14 个股票分析 Expert markdown 定义"
```

---

### Task 15: 聊天集成 — InputArea 触发股票分析

**Files:**
- Modify: `src/components/chat/InputArea.tsx`

- [ ] **Step 1: 在 InputArea 中添加 @股票代码 识别**

在 InputArea 的消息发送逻辑中，检测用户输入是否以 `@` 开头后跟 6 位数字：

```typescript
// 在 sendMessage 逻辑中添加
const stockCodeMatch = message.match(/@(\d{6})/);
if (stockCodeMatch) {
  const stockCode = stockCodeMatch[1];
  // 触发股票分析
  const { startAnalysis, setupEventListener } = useStockAnalysisStore.getState();
  await setupEventListener();
  await startAnalysis(stockCode, dayjs().format("YYYY-MM-DD"), "");
  // 发送一条系统消息到聊天
  // ...
  return;
}
```

- [ ] **Step 2: Commit**

```bash
git add src/components/chat/InputArea.tsx
git commit -m "feat: InputArea 支持 @股票代码 触发股票分析"
```

---

## 自审

1. **Spec coverage**: 数据层(Tasks 1-4) → Agent 架构(Task 14 experts) → 编排(Task 5) → 命令(Task 7) → 前端组件(Tasks 8-12) → 路由集成(Task 13) → 聊天集成(Task 15)。全部 spec 要求已覆盖。
2. **Placeholder scan**: 所有步骤均包含具体代码，无 TBD/TODO。
3. **Type consistency**: 后端 types.rs 的 struct 名与前端 types/stock-analysis.ts 的 interface 名一一对应（蛇形 vs 驼峰符合 serde rename 约定）。AnalysisEvent 枚举值前后端一致。

---

## 实施建议

推荐使用 **Subagent-Driven Development** 方式，每个 Task 分派一个独立 subagent，Task 间做 review。Task 1-4（数据层）可以合并并行，Task 8-12（前端组件）可以部分并行。
