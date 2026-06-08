# Vendor Trait As-Of 重构(完全无简化时间旅行)

**作者:** plan-engineering · **日期:** 2026-06-08
**状态:** Draft v1 · **目标版本:** AxInvest v0.8.x
**父设计:** [2026-06-08-time-travel-mode-design.md](./2026-06-08-time-travel-mode-design.md)
**范围:** 补齐 §3.2 / §4.1 中"vendor trait 不感知 as_of"导致的全部简化路径

---

## 1. 背景

父设计在 §3.2 决策 C 中提到"vendor trait 不感知 as_of,采用 lib.rs wrapper 收口"。本设计推翻该决策,改用 **vendor 内部感知 as_of + lib.rs 路由层按 capability 决策**。

### 1.1 现状(待消除的简化)

AStockClient 27 个方法,有 3 类简化:
- **C 档 10 个整方法跳过**:HotStocks / IndustryRanking / ClsFlash / MoneyFlow / MarginData / NorthBoundHolding / ConsensusEPS / ConceptBlocks / Peers / OptionPCR
- **A 档 12 个 truncate 模式**:vendor 返回全量,lib.rs 按日期截断(实际是 "全量 + 截断" 而非"as of 切片",vendor 限 100 条时截断后空集)
- **D 档 1 个 bug**:`get_market_dragon_tiger` 无 is_asof_active 守卫,replay 模式返回 today 数据

### 1.2 目标

- ✅ 27 个方法在 as-of 模式下**全部返回真实历史切片**(而非简化空集)
- ✅ live 模式一行不改(向后兼容)
- ✅ 每个 vendor 自己声明 as-of 能力,lib.rs 路由层查表决策
- ✅ 显式不可得的数据(3 个 NoHistoricalSemantic)走本地 SQLite 缓存,默认关闭,配置启用

### 1.3 非目标

- 不重构 vendor 业务逻辑
- 不引入新 web 框架 / 新 HTTP 库
- 不破坏现有 97/97 astock-data 测试

---

## 2. 核心抽象:AsOfCapability

### 2.1 4 变体 enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsOfCapability {
    /// vendor 原生支持日期参数(URL 带 begin/end)
    /// 调 vendor.get_x_with_asof() 时 vendor 内部自动加日期参数
    NativeDateParam,

    /// 用 K 线最后一行合成(已存在 quote_from_klines,扩展到所有"实时报价"类)
    SynthesizeFromKline,

    /// 无历史语义(概念性数据)
    /// as-of 模式下查本地 SQLite 缓存(Section 5);cache miss 时返回空 + record_degradation
    NoHistoricalSemantic,

    /// vendor 不支持,但接受"vendor 返回全量 + lib.rs 截断"兜底
    /// 仅作过渡,新 vendor 适配完成后移除
    Fallthrough,
}
```

### 2.2 Trait 扩展

```rust
#[async_trait]
pub trait StockVendor: Send + Sync {
    // ── 现有 27 个方法签名不变 ──
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError>;
    // ... 其他 26 个

    // ── 新增:vendor 自报 as-of 能力 ──
    /// vendor 声明自己 (method) 的 as-of 处理能力
    /// 默认 Fallthrough(走 lib.rs 截断)
    fn asof_capability(method: &str) -> AsOfCapability {
        let _ = method;
        AsOfCapability::Fallthrough
    }

    // ── 新增:vendor 内部 as-of 数据获取(默认实现 = 调原方法) ──
    /// vendor 内部读 current_asof(),自行决定调哪个端点
    /// 老的 vendor 不重写时 = 调 get_quote,走 lib.rs 截断(向后兼容)
    async fn get_quote_with_asof(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        self.get_quote(stock_code).await
    }
    // ... 其他 26 个 with_asof 方法,默认实现都是调原方法
}
```

### 2.3 申报示例(eastmoney 完整 23 个)

| method | capability | 说明 |
|--------|-----------|------|
| get_quote | SynthesizeFromKline | 用 K 线最后一行合成,非交易日 fallback 上一交易日 + record_degradation |
| get_klines | NativeDateParam | EM 支持 `begin=YYYYMMDD&end=YYYYMMDD` |
| get_financials | NativeDateParam | EM 支持按 report_date 区间 |
| get_news | NativeDateParam | EM 新闻可按时间窗 |
| get_money_flow | NativeDateParam | EM 资金流向可按日期 |
| get_dragon_tiger | NativeDateParam | EM 龙虎榜有日期参数 |
| get_lockup_schedule | NativeDateParam | EM 解禁可按日期 |
| get_margin_data | NativeDateParam | EM 融资融券有日期 |
| get_north_bound_holding | NativeDateParam | EM 北向持仓快照有日期 |
| get_sector_info | SynthesizeFromKline | 用历史 K 线 + 行业分类合成 |
| get_shareholder_trades | NativeDateParam | EM 股东交易有日期 |
| get_dividend_records | NativeDateParam | EM 除权除息有日期 |
| search_stock | Fallthrough | 搜索是当下语义,不支持历史 |
| get_research_reports | NativeDateParam | EM 研报可按日期 |
| get_consensus_eps | NativeDateParam | EM 一致预期有日期 |
| get_concept_blocks | NoHistoricalSemantic | 概念分类是当下状态(用户可启用本地缓存) |
| get_announcements | NativeDateParam | EM 公告可按时间窗 |
| get_market_dragon_tiger | NativeDateParam | **修复 D 档 bug**:加日期参数 |
| get_hot_stocks | NoHistoricalSemantic | 热门股是当下榜单 |
| get_industry_ranking | NoHistoricalSemantic | 行业排名是当下 |
| get_cls_flash | NoHistoricalSemantic | 财联社快讯是当下 |
| get_north_bound_flow | NativeDateParam | EM 北向资金流有日期 |
| get_block_trades | NativeDateParam | EM 大宗交易有日期 |
| get_institutional_visits | NativeDateParam | EM 机构调研有日期 |
| get_index_quotes | SynthesizeFromKline | 指数用历史 K 线合成 |
| get_peers | NoHistoricalSemantic | 同行对比是当下(用户可启用本地缓存) |
| get_option_pcr | NativeDateParam | EM 期权 PCR 有日期 |

**统计**: 23/23 全部有非 Fallthrough 能力(eastmoney),其中 18 NativeDateParam + 3 SynthesizeFromKline + 3 NoHistoricalSemantic(本地缓存可选)

---

## 3. Lib.rs 路由层

### 3.1 路由函数形态

```rust
impl AStockClient {
    async fn dispatch<T, F, W>(
        &self,
        method: &str,
        stock_code: &str,
        live_fn: F,    // 原 live 路径
        asof_fn: W,    // 内部走 capability 决策
    ) -> Result<T, DataError>
    where
        F: Future<Output = Result<T, DataError>>,
        W: Future<Output = Result<T, DataError>>,
    {
        if !as_of::is_asof_active() {
            live_fn.await
        } else {
            asof_fn.await
        }
    }
}
```

实际更简洁的写法:每个方法内部直接 `if as_of::is_asof_active() { 走 capability } else { 走原路径 }`,不抽宏。

### 3.2 Capability 决策伪代码

```rust
async fn dispatch_get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
    if !as_of::is_asof_active() {
        return self.dispatch_get_quote_live(stock_code).await;
    }

    let as_of = as_of::current_as_of().unwrap();
    for name in self.routing.vendors_for("get_quote", &self.routing.quote) {
        let vendor = self.find_vendor(name).unwrap();
        match vendor.asof_capability("get_quote") {
            AsOfCapability::NativeDateParam => {
                if let Ok(q) = vendor.get_quote_with_asof(stock_code).await {
                    if !q.timestamp.is_empty() {
                        return Ok(q);
                    }
                }
            }
            AsOfCapability::SynthesizeFromKline => {
                // 拉 as_of_date 当日(或上一交易日)K 线
                let cutoff = effective_trading_day(as_of.as_of_date);
                if let Ok(klines) = self.get_klines(stock_code, "daily", 30).await {
                    if let Some(q) = Self::quote_from_klines(stock_code, &klines) {
                        return Ok(q);
                    }
                }
            }
            AsOfCapability::NoHistoricalSemantic => {
                // 查本地 SQLite
                if let Some(q) = self.local_cache.get_quote(stock_code, as_of.as_of_date) {
                    return Ok(q);
                }
            }
            AsOfCapability::Fallthrough => {
                // 不应到达(每个 vendor 适配后都是 NativeDateParam 等)
            }
        }
    }
    Err(DataError::AsOfDegraded { ... })
}
```

### 3.3 消除简化的具体清单

| 旧简化 | 新行为 |
|--------|--------|
| `get_hot_stocks` 整方法跳过 | vendor 调历史端点(eastmoney/ths/baidu 都有),本地缓存可选 |
| `get_industry_ranking` 整方法跳过 | 同上 |
| `get_cls_flash` 整方法跳过 | 同上 |
| `get_money_flow` 整方法跳过 | eastmoney/baidu 都支持 `date=YYYYMMDD` |
| `get_margin_data` 整方法跳过 | eastmoney 有日期参数 |
| `get_north_bound_holding` 静默 `tracing::warn` | 改 `asof_capability = NativeDateParam` + 用 with_asof |
| `get_consensus_eps` 整方法跳过 | 改 `NativeDateParam`,按发布日期过滤 |
| `get_concept_blocks` 整方法跳过 | `NoHistoricalSemantic` + 本地缓存 |
| `get_peers` 整方法跳过 | `NoHistoricalSemantic` + 本地缓存 |
| `get_option_pcr` 整方法跳过 | `NativeDateParam`,EM 支持日期 |
| `get_market_dragon_tiger` 无守卫 | 加 `asof_capability = NativeDateParam` + with_asof |
| A 档 12 个 truncate 模式 | 仍可保留截断(防御兜底),但优先用 vendor with_asof |

---

## 4. 阶段 1 范围(P0 + P1,2 周)

### 4.1 P0 任务(0.5 周)

- [ ] 新建 `crates/astock-data/src/as_of_capability.rs`
  - 定义 `AsOfCapability` enum(4 变体)
  - 为 trait 加 `asof_capability()` 默认返回 Fallthrough
  - 为 trait 加 27 个 `*_with_asof()` 方法,默认实现 = 调原方法
- [ ] `lib.rs` 加 `dispatch_*` helper 函数
- [ ] 单元测试:enum + 默认实现 + dispatch 路径覆盖(8+ 个测试)
- [ ] `cargo test -p axagent-astock-data`:97/97 不回归

### 4.2 P1 任务(1.5 周,eastmoney 主力)

- [ ] eastmoney.rs:`asof_capability()` 完整 23 个方法声明
- [ ] eastmoney.rs:重写 18 个 `*_with_asof()`,URL 加 `begin/end` 或 `date` 参数
- [ ] eastmoney.rs:`get_quote_with_asof` 改用 K 线合成(复用 `quote_from_klines`)
- [ ] eastmoney.rs:`get_sector_info_with_asof` / `get_index_quotes_with_asof` 用 K 线合成
- [ ] eastmoney.rs:vendor 级测试(给定 as_of_date,验证 URL 包含正确 begin/end)— 8+ 个测试
- [ ] lib.rs:`dispatch_get_quote` 等 5 个关键方法的 capability 决策实现
- [ ] lib.rs:集成测试(eastmoney 路径 23 个方法,as-of 模式下全部返回非空)— 23 个测试
- [ ] `cargo test -p axagent-astock-data`:97 旧 + 31 新 = 128/128

### 4.3 P1 验收标准

- ✅ eastmoney 路径 23/23 方法在 as-of 模式下返回真实历史数据
- ✅ live 模式一字不改
- ✅ 旧 97 测试 0 回归
- ✅ 新 31 测试全过
- ✅ `get_market_dragon_tiger` bug 修复
- ✅ 10 个 `if is_asof_active { return Ok(vec![]) }` 简化的 eastmoney 路径全部消除

### 4.4 完成后用户决策

P1 完成后,保留能力让用户选择:
- 继续 P2(akshare + baidu + cninfo,2 周)
- 继续 P3(ths + tencent + sina + iwencai + mootdx,1.5 周)
- 继续 P4(lib.rs 全量切到 capability 决策,1 周)
- 继续 P5(本地 SQLite 缓存 + 后台 sweep,1 周)
- 中止重构,保留 P1 成果

---

## 5. 未来阶段概要(P2-P6,7 周)

| 阶段 | 范围 | 工时 |
|------|------|------|
| P2 | akshare + baidu + cninfo 适配(23 个方法 × 3 vendor) | 2 周 |
| P3 | ths + tencent + sina + iwencai + mootdx 适配(23 个方法 × 5 vendor) | 1.5 周 |
| P4 | lib.rs 27 个方法全部切到 capability 决策 + 删除 10 处简化 | 1 周 |
| P5 | market_data_history SQLite 表 + 后台 sweep job + 配置开关 | 1 周 |
| P6 | 集成测试 + 回归 + 文档 | 0.5 周 |

---

## 6. 风险与缓解

| 风险 | 缓解 |
|------|------|
| akshare/ths 反爬严格 → 部分历史端点 403 | 走 Fallthrough 兜底,降级到 lib.rs 截断 + 显式 record_degradation |
| mootdx 是 C 库,无 HTTP 时间参数 | 适配为 Fallthrough 或 NoHistoricalSemantic |
| 9 vendor 全部适配后仍 ~15% 覆盖不到真正"无简化" | 走本地 SQLite 缓存,仍能 100% 覆盖 |
| 测试无法对所有 vendor 全部做端到端(需要真实 API 响应) | 分两层:vendor 级 mock HTTP 响应;lib.rs 级跑 eastmoney 真实路径 |

---

## 7. 度量

- **简化率**:当前 ~50% 方法被简化(C 档 37% + D 档 4% + A 档逻辑性简化)
- **P1 目标简化率**:eastmoney 路径 0%(23/23)
- **P4 目标简化率**:0%(27/27,所有 vendor 路径无简化)
- **"as-of 模式返回非空"的方法数**:27(100%)

---

## 8. 验收命令

```bash
# P0 验收
cd d:\OneManager\AxInvest\src-tauri
cargo test -p axagent-astock-data --lib  # 97 旧 + 8 新 = 105

# P1 验收
cargo test -p axagent-astock-data --lib  # 105 + 23 新 = 128

# live 模式不破坏
cargo test -p axagent-astock-data --lib is_asof_active_false_in_live  # 仍过

# 前端
cd d:\OneManager\AxInvest
pnpm tsc --noEmit  # 0 errors
pnpm vitest run    # 627/627 不回归
```

---

## 9. 相关文件

- 父设计:`docs/superpowers/specs/2026-06-08-time-travel-mode-design.md`
- 父计划:`docs/superpowers/plans/2026-06-08-time-travel-mode-plan.md`
- 现有 trait:`src-tauri/crates/astock-data/src/vendors/mod.rs`
- 现有 lib.rs:`src-tauri/crates/astock-data/src/lib.rs:240-300`
- 现有 as_of:`src-tauri/crates/astock-data/src/as_of.rs`
- 现有 disk_cache(Section 5 复用):`src-tauri/crates/astock-data/src/disk_cache.rs`
