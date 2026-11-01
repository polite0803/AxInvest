# 股票分析「低置信」根因审计（分析师数据质量面板）

- **日期**：2026-09-21
- **样本**：最新一轮工作流运行 —— `stock_analyses.id = a1dfe12c-0981-4220-859d-c826994c8dfe`
- **标的**：300308 中际旭创 ｜ **运行时间**：2026-09-20T16:22:39Z（北京时间 09-21 00:22:39）
- **模板版本**：`template_version = 62` ｜ **状态**：completed
- **数据来源**：PostgreSQL `axinvest`.`stock_analyses.blackboard_snapshot`（1.57 MB，含全部节点原始输出）
- **面板输出（截图）**：4 个节点判「⚠ 低置信」—— 催化剂(60/1 处)、基本面(60/1 处)、资金面(55/3 处)、情绪面(60/1 处)

> 结论先行：**面板没有渲染错误，与权威输出逐字对齐。**4 个「低置信」里，
> **2 个节点的证据是真实工具缺陷**（14/14 轮 100% 复现），
> **1 个节点的 3 处证据里有 2 处是词表假阳性**（把分析师的「澄清句」判成了缺口），
> 而这条链最终**真的改写了决策**（`positionPct = 0`）。

---

## 一、面板与权威输出对齐（先排除展示层 bug）

`data-quality.rhai` 输出的 `diagnostics` 与面板逐字一致，证明问题不在前端：

| 节点 | 缩写 | conf | status | placeholder_hits | 面板显示 |
|---|---|---|---|---|---|
| 催化剂分析师 | cat | 60.0 | low | 1 | ⚠ 低置信 ✔ |
| 基本面分析师 | fund | 60.0 | low | 1 | ⚠ 低置信 ✔ |
| 资金面分析师 | hm | 55.0 | low | 3 | ⚠ 低置信 ✔ |
| 解禁观察员 | lk | 60.0 | normal | 0 | ✅ 正常 ✔ |
| 技术面分析师 | mk | 72.0 | normal | 0 | ✅ 正常 ✔ |
| 新闻面分析师 | news | 70.0 | normal | 0 | ✅ 正常 ✔ |
| 政策面分析师 | pol | 58.0 | normal | 0 | ✅ 正常 ✔ |
| 研报分析师 | res | 70.0 | normal | 0 | ✅ 正常 ✔ |
| 行业分析师 | sec | 80.0 | normal | 0 | ✅ 正常 ✔ |
| 情绪面分析师 | sent | 60.0 | low | 1 | ⚠ 低置信 ✔ |

全局：`grade=B`，`score=79.2`，`avg_confidence=64.5`，`good_count=6`，`gap_count=0`，`degraded_count=4`。

**顺带解释一个看似矛盾的现象**：政策面 58 分显示「正常」，而催化剂 60 分显示「低置信」。
这不是阈值错乱——判据是 **A ∪ B**（`data-quality.rhai:798-802`）：报告文本失败标记（客观证据）优先于 LLM 自评 confidence（主观）。
58 分无标记 ⇒ normal；60 分有标记 ⇒ low。语义正确，但面板未向用户说明这条不对称规则。

---

## 二、6 处「失败标记」的逐条归因

`placeholder_total_hits = 6`。逐处回原文定位后，**实际归属 3 类根因**：

| # | 节点 | 命中词 | 原文片段 | 判定 |
|---|---|---|---|---|
| 1 | sent | `数据缺失` | 「龙虎榜…买卖方席位均为"0/0"，未显示营业部明细。无法确认拉萨团结路/东环路席位数量…**席位数据缺失**影响情绪极值判断」 | **真缺口**（根因 A） |
| 2 | fund | `为 null` | 「（注：行业同侪 PE/PB/ROE **数据源返回全为 null**，同业横向估值对比缺失）」 | **真缺口**（根因 B） |
| 3 | hm | `数据缺失` | 「**龙虎榜席位与北向数据缺失**，无法验证机构与外资方向」 | **真缺口**（根因 A） |
| 4 | hm | `数据不可用` | 「本维度**数据不可用**，标注为技术停披而非无数据」 | **假阳性**（根因 C） |
| 5 | hm | `无数据` | 同上句「…**而非无数据**」——同一句话被二次命中 | **假阳性**（根因 C） |
| 6 | cat | `为 null` | 「同业可比（光模块三巨头）PE/PB 字段在 peer 工具中**为 null**，横向估值锚缺失」 | **真缺口**（根因 B） |

**即：6 处命中 = 4 处真实工具缺口 + 2 处同一句澄清语的误报（33%）。**

---

## 三、根因 A：龙虎榜席位恒为「0/0」（类型不匹配 + 语义误解）

### 现象
`t-dragon-tiger-data` 返回的**每一条**记录都是 `"deptName":"买入0席位/卖出0席位"`，而同一记录的金额字段是真实值（买入 106.0 亿 / 卖出 77.6 亿 / 净 28.39 亿）。**全部 14 轮历史运行 100% 复现**。

### 反事实取证（直接请求东财原始接口）
```
GET https://datacenter-web.eastmoney.com/api/data/v1/get?
    reportName=RPT_DAILYBILLBOARD_DETAILS&columns=ALL&
    filter=(SECURITY_CODE%3D%22300308%22)&...
→ HTTP 200，36 个字段，其中：
    BUY_SEAT     = 13331          (number)
    SELL_SEAT    = 13133          (number)
    BUY_SEAT_NEW = "13331"        (string ← 注意类型)
    SELL_SEAT_NEW= "13133"        (string)
```

### 代码缺陷
`src-tauri/crates/astock-data/src/vendors/eastmoney.rs:960-962`：
```rust
let buy_seat  = r["BUY_SEAT_NEW"].as_i64().unwrap_or(0);   // ← 对 JSON 字符串取 i64 ⇒ 恒 None ⇒ 恒 0
let sell_seat = r["SELL_SEAT_NEW"].as_i64().unwrap_or(0);
let dept_name = format!("买入{}席位/卖出{}席位", buy_seat, sell_seat);
```
同一缺陷在 `browser_eastmoney.rs:378,383` 重复一份。

### 三重问题（不止是类型）
1. **类型不匹配静默兜底**：`as_i64()` 对字符串返回 `None`，被 `unwrap_or(0)` 吞掉 ⇒ 解析失败被伪装成「0 席位」，无任何日志/降级信号。
2. **语义理解错误**：`eastmoney.rs:932` 的注释写着 `BUY_SEAT_NEW/SELL_SEAT_NEW: 买入/卖出营业部数量`。实际返回的是**席位编号**（上交所交易单元代码 `13331`），不是数量。即使修好类型，产出 `买入13331席位/卖出13133席位` 依然是错的。
3. **数据粒度不足**：`RPT_DAILYBILLBOARD_DETAILS` 是**汇总级**报表（只有买卖总额），本就不含营业部明细。分析师真正想要的「拉萨团结路/东环路席位」需要另一个明细报表——**当前工具从未实现过**。

⇒ 分析师写「席位数据缺失」是**如实且正确**的。这条不是数据源的问题，是工具解析层的缺陷。

---

## 四、根因 B：同侪估值字段硬编码 `None`（功能从未实现）

### 现象
基本面 / 催化剂 / 行业 三个分析师的报告均提到「同侪 PE/PB 全为 null」。

### 代码缺陷
`src-tauri/crates/astock-data/src/vendors/eastmoney.rs:2238-2246`：
```rust
.map(|r| PeerComparison {
    stock_code: r["SECURITY_CODE"].as_str().unwrap_or("").to_string(),
    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
    pe: None,          // ← 硬编码，从未取值
    pb: None,          // ← 硬编码，从未取值
    roe: None,         // ← 硬编码，从未取值
    change_pct: 0.0,
    market_cap: None,
})
```

**这三个字段不是「数据源返回 null」，也不是解析失败——是代码里写死的 `None`。** 该工具只返回了板块内股票名单（代码+名称），估值维度从未实现。

`get_peers` 的两步查询用的是 `RPT_F10_CORETHEME_BOARDTYPE`（板块成分表），该报表本身也不含 PE/PB/ROE，所以即便补取值也需要换报表。

⇒ 分析师写「数据源返回全为 null，同业横向估值对比缺失」描述**准确**，但归因（"数据源"）**不准确**，真实根因是**代码未实现**。

---

## 五、根因 C：词表判据的假阳性——「越认真澄清，越被判缺口」

`data-quality.rhai:176-182` 的 `placeholder_markers()` 做**纯子串包含**匹配，无法识别否定/排除语气。

### 本轮实例（hm 资金面）
分析师原文：
> 「北向自 2024-08-16 监管停披净流入数据，当前返回为成交额（沪 1611.7 亿、深 1727.5 亿），无净流入信号，无法用于个股北向判断；**本维度数据不可用，标注为技术停披而非无数据**。」

分析师的意图是**明确排除**「这是数据源故障」这一误解，结果同时命中 `数据不可用` 和 `无数据` 两个词——**一句话贡献了 hm 全部 3 处命中里的 2 处**。

### 跨轮次同类误报（抽查 3 轮）
| 轮次 | 节点 | 原文片段 | 性质 |
|---|---|---|---|
| 09-13 600031 | sent | 「期权隐含情绪指标返回空（**视为该维度暂无数据/无期权覆盖**）」 | 正常无覆盖，非故障 |
| 09-13 600031 | news | 「期权PCR数据返回null（**该维度暂无数据**，对消息面分析影响低）」 | 同上 |
| 09-12 603353 | news | 「未检索到立案调查…（数据缺口：无监管函类记录，**暂判定为"无监管事件"，非"无数据源"**）」 | 明确排除数据源问题 |
| 09-14 601698 | lk | `<!-- VERDICT: {"bear_points":[...,"质押数据缺失"]} -->` | 正常风险点罗列被计入 |

**共同形态：误报全部来自「澄清 / 否定 / 排除」句式和 VERDICT 内的正常字段。** 分析师越严谨地标注「这不是数据缺口」，越容易被误判为数据缺口。

---

## 六、影响链：低置信真的改写了决策（不是纯展示）

`trader` 节点的决策轨迹原文（`llm_decision_json.decision_trail`）：
> 「…但风险分歧65>50 + **dqi_score 79.2 中情绪/基本/资金/催化剂低置信** → 采用 risk-convergence 保守仓位 0%」

最终落库：`action = 观望`、`decision_position_pct = 0`。

### 两处链路断点
| 层 | data_gaps / 缺口信息 | 状态 |
|---|---|---|
| LLM（trader） | **5 条真实缺口**：H股正文未解析 / 龙虎榜席位缺失(0/0) / 同业PE-PB缺失 / 北向停披 / K线缺口 | ✅ 识别准确 |
| 词表（data-quality） | 6 处标记，其中 2 处误报 | ⚠ 33% 假阳性 |
| 量化（portfolio-mgr） | `data_gaps = []`（**空**） | ⚠ 口径只测存在性（见下方订正） |

> **⚠ 2026-09-21 订正（实施阶段核对源码后推翻）**：上文原把 `data_gaps = []` 判为
> 「缺口信息在此丢失 / 量化链路没有接收 data-quality 的降级清单」——**该归因错误**。
> 核对 `portfolio-mgr.rhai:2031-2045` 后确认：该 `data_gaps` 是**独立计算**的
> （判据为 `present(x)`，即节点/字段**是否存在**），从不读取 data-quality 的输出，
> 因此不存在「没接收」这回事。本轮工具**全都返回了**数据
> （`t-dragon-tiger-data` 有 JSON、`t-fundamentals-data` 有 peers），
> 只是**内容**为空（席位恒 `0/0`、PE 恒 `null`）⇒ 存在性判据必然为真 ⇒ `data_gaps` 必为空。
> ⇒ 真实缺陷是「**判据口径只测存在性、测不出内容级缺失**」，而非链路断点。修复见 §十一。
> （教训：把「判据口径不足」直接读成「链路未接线」，是两种检法完全相反的缺陷——
> 前者要放宽口径，后者要找缺失的入边。此处未先 grep 源码就下了结论。）

---

## 七、判据稳定性：8 轮横向对比

| 轮次 | 标的 | grade | score | avg_conf | good | degraded | placeholder_nodes | hits |
|---|---|---|---|---|---|---|---|---|
| 09-20 | 300308 | B | 79.2 | 64.5 | 6 | 4 | 情绪面/基本面/资金面/催化剂 | 6 |
| 09-14 | 601166 | B | 66.9 | 62.9 | 7 | 1 | 情绪面 | 2 |
| 09-14 | 601698 | B | 70.9 | 65.5 | 6 | 3 | 基本面/资金面/解禁观察/行业 | 5 |
| 09-14 | 001313 | B | 68.6 | 64.9 | 6 | 3 | 情绪面/基本面/资金面 | 4 |
| 09-13 | 600031 | C | 54.7 | 57.4 | 4 | 4 | 技术面/情绪面/新闻面/基本面/催化剂 | 9 |
| 09-12 | 601166 | C | 61.3 | 64.5 | 5 | 5 | 情绪面/基本面/资金面/解禁观察/行业 | 7 |
| 09-12 | 603466 | C | 64.9 | 65.5 | 6 | 3 | 技术面/资金面/解禁观察 | 4 |
| 09-12 | 603353 | C | 50.3 | 58.5 | 4 | 4 | 情绪面/新闻面/资金面/研报 | 6 |

**关键对照**：`买入0席位` 这个底层缺陷在 **14/14 轮全部存在**（零变化），但被判 low 的节点组合**每一轮都不同**（技术面、新闻面、研报、行业忽有忽无）。

⇒ **面板测的是「LLM 报告里的措辞」，不是「数据缺口本身」。** 同一底层数据状态，因 LLM 表达差异而得出不同诊断，诊断结果缺乏跨轮可比性。

---

## 八、附带发现

### 8.1 面板的「预期数据」指向了错误的工具
`data-quality.rhai:836-847` 的 `expected_data` 是**硬编码字符串**，与实际缺口来源不符：

| 节点 | 面板写的预期数据 | 实际缺口来源 |
|---|---|---|
| 情绪面 | 「市场情绪、资金流向、散户/机构态度（**t-sentiment-data**）」 | `t-dragon-tiger-data`（龙虎榜，**独立节点**） |
| 资金面 | 「游资动向、主力资金净流入（**t-hotmoney-data: get_stock_money_flow**）」 | `t-dragon-tiger-data` + 北向字段 |

⇒ 面板把用户引向 `t-hotmoney-data`，而真正要修的是 `eastmoney.rs::get_dragon_tiger` 和 `get_peers`。**归因指向错误，会误导排查方向。**

### 8.2 `degraded_count = 4` 被算出，但不参与任何评分惩罚
`data-quality.rhai:363-375` 的 `tool_credibility` 只惩罚 `gap_count`（confidence < 0）与 `good_count`：

```
tool_credibility = avg_conf(64.5) − gap惩罚(0) − good惩罚(0) = 64.5
最终 61.5 = 64.5 − validation_penalty(3，来自行业 bull+bear=113)
```

反事实验算：若把 4 个 degraded 节点的自评剔除，`avg_conf` 反而**上升**到 68.3（剩余 6 个节点普遍更高）。
⇒ **`degraded` 在公式结构上无法产生惩罚。** 报告含失败标记的代价只通过 `report_quality` 的 `-15` 占位符扣分间接体现一次。

### 8.3 「N 处」是词种去重，不是出现次数
`placeholder_hits` 按**词表逐词最多计一次**。hm 面板显示「3 处」，原文中实际出现 4+ 次（`数据缺失` 出现 2 次只计 1）。面板数字与用户直觉不一致。

### 8.4 行业分析师的真实质量问题被淹没
唯一与数据无关的真实缺陷是 `bull_score + bear_score = 113`（应≈100，扣 3 分），属 LLM 自评不自洽——它和「数据缺口」被混在同一个面板里展示，语义不同却同等呈现。

---

## 九、修复建议（按优先级，标注是否需产品决策）

| # | 项 | 位置 | 类型 | 建议 |
|---|---|---|---|---|
| P0-1 | 龙虎榜席位类型 + 语义 | `eastmoney.rs:960-962`、`browser_eastmoney.rs:378-383` | **纯 bug** | 改为 `as_str()`/兼容数字，并按真实语义重命名字段（`seat_code` 而非「席位数量」）；`unwrap_or(0)` 改为显式缺失标记，禁止静默兜底 |
| P0-2 | 龙虎榜营业部明细 | 同上（需换报表） | **需产品决策** | 当前报表不含营业部明细。要支撑「拉萨天团」类规则，需接入明细接口；否则应**下调该维度在 prompt 中的预期**，别让分析师反复写缺口 |
| P1-1 | 同侪估值字段 | `eastmoney.rs:2241-2243` | **功能未实现** | 需决策：补一个真正含 PE/PB/ROE 的同业接口，或明确宣告 `get_peers` 只提供名单、在 prompt 中删除「同侪估值对比」要求 |
| P1-2 | 词表假阳性 | `data-quality.rhai:176-182, 186-206` | **需产品决策** | ① 匹配前剥离 VERDICT 注释块；② 对「非/而非/不是/暂无/不适用/视为」等否定前缀做前瞻排除；③ 区分「缺口描述」与「缺口澄清」 |
| P1-3 | 量化层 `data_gaps` 断点 | `portfolio-mgr` / `data_gaps` 链路 | **需产品决策** | `pm_result.data_gaps = []` 而 LLM 有 5 条。确认量化层是否应接收 data-quality 的降级清单 |
| P2-1 | `expected_data` 硬编码 | `data-quality.rhai:836-847` | **需产品决策** | 改为从 DAG 边/节点声明派生，或至少补上 `t-dragon-tiger-data` |
| P2-2 | `degraded` 不参与评分 | `data-quality.rhai:363-375, 796-802` | **需产品决策** | 决定 `degraded_count` 是否应进入 `tool_credibility` 惩罚项 |
| P2-3 | 「N 处」口径 | `data-quality.rhai:186-206` | 展示口径 | 面板区分「命中词种数」与「实际出现次数」 |
| P2-4 | 判据不对称说明 | 前端 `AnalystDataQualityModal` | 展示口径 | 说明「有失败标记时判据优先于自评分」这一规则 |

---

## 十、取证方法（可复现）

1. 从 `stock_analyses` 取 `created_at` 最大的一条 `completed` 记录，导出 `blackboard_snapshot`。
2. 解析顶层 `data_quality_summary`（data-quality 节点权威输出）。
3. 对 `report.a-*` 报告正文用与 `placeholder_markers()` 相同的词表逐处定位上下文。
4. 对 `result.t-*` 工具节点核对返回形态。
5. **反事实**：直接请求东财原始接口，验证字段名/类型/粒度，排除「数据源不提供」这一假设。
6. 横向统计历史各轮的 `placeholder_nodes`，判断判据稳定性和缺陷复现率。

> ⚠ 本次分析全程只读：未执行任何 git 命令，未修改源码。所有结论锚定在真实运行产物与源码行号上。
> （该「只读」约束仅适用于 §一~§十 的**取证**阶段；§十一 的**实施**阶段已修改源码，改动清单见该节。）

---

## 十一、实施记录（2026-09-21）

**实施顺序与 §九 相反**：先做外部接口反事实探测，再动代码 —— 因为 P0-2 / P1-1 的走向
取决于「数据源到底提不提供」，猜不得（§三 的教训正是凭字段名猜语义）。

### 11.1 外部接口探测结论（决定 P0-2 / P1-1 的走向）

| 报表 / 域名 | 结论 |
|---|---|
| `RPT_BILLBOARD_DAILYDETAILSBUY` / `...SELL` | ✅ **可用**：营业部级明细，含 `OPERATEDEPT_NAME`（实测返回「机构专用」「中国国际金融股份有限公司上海分公司」）、`BUY`/`SELL`/`NET` |
| `RPT_VALUEANALYSIS_DET` | ✅ **可用**：含 `PE_TTM` / `PB_MRQ` / `TOTAL_MARKET_CAP` / `CHANGE_RATE`，且支持 `(SECURITY_CODE in (...))` **批量**查询 |
| `RPT_BILLBOARD_TRADEDETAILS`、`RPT_STOCK_VALUATION`、`RPT_DMSK_VALUATION`、`RPT_INDUSTRY_VALUATION`、`RPT_BOARD_CONS_DETAIL` | ❌ 报表配置不存在（已排除） |
| `RPT_F10_CORETHEME_BOARDTYPE`（`get_peers` 现用） | 全字段核对：**只有代码/名称/板块，无任何估值字段** ⇒ §四 结论成立 |
| `push2*.eastmoney.com` 全系 | ⚠ 本机探测全部 `socket hang up`（仅 `push2delay` 返回 200）。本批改动**完全不依赖 push2**，规避了该不确定性 |

### 11.2 逐项落地

| # | 项 | 状态 | 改动位置 | 说明 |
|---|---|---|---|---|
| P0-1 | 龙虎榜席位类型 + 语义 | ✅ 已修 | `eastmoney.rs::get_dragon_tiger`、`browser_eastmoney.rs` 同名函数 | 不再读 `BUY_SEAT_NEW`（字符串 ⇒ `as_i64()` 恒 None ⇒ 被 `unwrap_or(0)` 吞）；改接营业部明细报表，`dept_name` = 真实 `OPERATEDEPT_NAME`。**两个报表都失败时返回 Err 而非空 vec** —— 消灭「请求失败伪装成未上榜」这一同型静默兜底 |
| P0-2 | 龙虎榜营业部明细 | ✅ **已补数据**（原判「需产品决策」，探测后确认接口存在，选补数据而非下调 prompt 预期） | 同上 | 顺带**统一了 vendor 语义**：`baidu_stock` 的 `dept_name` 本来就是营业部名，此前 eastmoney 给的是席位编号 ⇒ 两个 vendor 同名字段语义不一致 |
| P1-1 | 同侪估值字段 | ✅ 已补 4/5 | `eastmoney.rs::get_peers` | 批量查 `RPT_VALUEANALYSIS_DET` 并按 `TRADE_DATE` 取最新一行：`pe`←PE_TTM、`pb`←PB_MRQ、`market_cap`←TOTAL_MARKET_CAP、`change_pct`←CHANGE_RATE。**`roe` 保持 `None`** —— 该报表不含 ROE，需另接财报报表；**不猜值、不用近似量冒充** |
| P1-2 | 词表假阳性 | ✅ 已修 | `data-quality.rhai` | ① `strip_verdict_blocks` 剥离 `<!-- VERDICT -->`；② `soft_marker_suppression` 对**软标记**做**定向**否定短语抑制（硬标记不抑制 ⇒ 真缺口保留）。**不做整篇抑制**：本轮 hm 报告同时含真缺口（数据缺失）与澄清语，整篇抑制会连带抹掉真缺口 |
| P1-3 | 量化层 `data_gaps` | ✅ 已补内容级判据 | `portfolio-mgr.rhai:2038+` | 新增「龙虎榜 entries 全部无实质营业部名」判据；口径与既有先例 `risk_pe <= 0.0` 一致。同时订正 §六 的错误归因（见该节订正块） |
| P2-1 | `expected_data` 硬编码 | ✅ 已修正 | `data-quality.rhai` diagnostics | 情绪面/资金面补 `t-dragon-tiger-data`；基本面/催化剂/行业补 `get_stock_peers` |
| P2-2 | `degraded` 是否入惩罚 | ✅ **决定不入惩罚**，并在代码内写明理由与「不要顺手补上」 | `data-quality.rhai::tool_credibility` | 三条依据：① 与 `avg_conf` 同源自评；② 反算证明两者负相关（剔除后 avg_conf 反升）⇒ 叠加即双重计数；③ 语义是「措辞/自评」而非「工具可信度」。另在 `warnings` 显式说明，避免被后续维护者误读为漏实现 |
| P2-3 | 「N 处」口径 | ✅ 已区分 | `data-quality.rhai` + 前端 + 11 语言 | 新增 `placeholder_occurrences` / `placeholder_total_occurrences`（与 hits **同判定集**，共用剥离与抑制规则，否则两个口径会互相矛盾）；面板并列显示「N 种标记（共出现 M 次）」 |
| P2-4 | 判据不对称说明 | ✅ 已加 | 前端 `AnalystDataQualityModal` + 11 语言 | 新增 `dqJudgePriorityNote`，说明「含失败标记即优先判 low」这条 A ∪ B 规则为何使 58 分显示正常、60 分显示低置信 |

**未做项**：无 §九 条目被跳过。`roe` 是唯一未补齐的字段（见 P1-1 说明），已按「显式缺失」而非「伪造」处理。

### 11.3 新增发现（实施阶段才暴露的第三个后果）

`portfolio-mgr.rhai:1004` 用 `dept.contains("机构")` 识别「机构专用」席位并计入筹码面机构信号（权重 0.15）。
席位名恒为 `买入0席位/卖出0席位` 时该判据**永不成立** ⇒ `inst_buy`/`inst_sell` 恒 0
⇒ `inst_net` 恒 0 ⇒ 被 `if inst_net.abs() > 0.01` 跳过 ⇒ **机构信号 100% 静默失效，且全程零告警**。

这比面板显示更实质：面板只是把问题**显示**出来，而这里的问题**从未被记录**。
修复 P0-1 后该信号自动恢复（`dept_name` 现为「机构专用」等真实席位名，含「机构」二字）。

### 11.4 模板版本（改了必须升，否则仓库修了 DB 仍是坏的）

`data-quality.rhai` / `portfolio-mgr.rhai` 均经 `include_str!` 嵌入 stock-analysis 模板节点的 `code`
（`seed_stock_analysis.rs:2951` / `:3266`）⇒ 改脚本必须升 `TEMPLATE_VERSION`，
否则版本门 `existing.version >= TEMPLATE_VERSION` 让种子直接跳过。

- **真库核对（只读查询）**：`stock-analysis` 的 DB `version` = **62**（运行产物 `template_version=62` 亦佐证）
- 常量 63 → **64** ⇒ **v63 与 v64 两批会一并落库**（v63 此前已写常量但未落库）

### 11.5 验证与回归防线

新增可执行门禁 `src-tauri/crates/rt-workflow/tests/data_quality_placeholder_gate.rs`：

- 现有 `rhai_syntax_check.rs` **只做 `engine.compile`** —— 抓不到「Function not found」类运行时错误，也锁不住判据行为。
- 新测试**注入真实报告片段并执行整个脚本**，断言：澄清句不计命中、真缺口仍计命中、VERDICT 块被剥离、三类历史误报样本 0 命中、五类真缺口语必检出、`diagnostics.status` 与 `low_confidence_analysts` 口径一致。
- 同时**钉住一个引擎契约**：判据依赖 Rhai `StringPackage`（`split`/`replace`）。
  ⚠ `data-quality.rhai:131-140` 的历史注释称「`replace` 不可用」，那与当前实况不符
  （`Engine::new()` 含 `StandardPackage`；`rt-workflow`/`harness` 均如此构造）。
  新测试含 `rhai_string_package_available` 探针，若未来有人打开 `no_string` feature，会先红。

### 11.6 实施后复核（补 §11.1 的形态层 + 消费端核查）

#### A. 外部接口的**字段类型**实测（不止「能不能用」）

改前特意先测类型，因为 §三 的教训正是「凭字段名/可用性猜语义」：
若新接的 `BUY`/`SELL` 也是**字符串**，本次修复就会把缺陷从席位名**搬到金额上**
（换个地方继续静默为 0）。

| 字段 | 实测 JSON 类型 | 结论 |
|---|---|---|
| `OPERATEDEPT_NAME` | `string` | ✅ 真营业部名：`机构专用` / `中国国际金融股份有限公司上海分公司` / `高盛(中国)证券有限责任公司上海浦东新区世纪大道证券营业部` / `沪股通专用` |
| `BUY` / `SELL` / `NET` | `number` | ✅ `as_f64()` 有效（与旧字段 `BUY_SEAT_NEW` 是 `string` 形成对照 —— 那正是原缺陷根源） |
| `BUY` / `SELL` | 可含 **`null`** | `as_f64()` → `None` → `0.0`。语义上「该席位在该方向无金额」成立（如 600552 买入明细行 `SELL=null`），**不是**同型静默兜底 |

抽样 4 只（300308 / 300567 / 688361 / 600552）× 买/卖两张报表，全部返回 `rows>0`。

#### B. 返回形态变化的消费端核查（**编译器不设防**的变更）

本次改的不是返回**类型**，而是**基数**：由「按日 1 行（汇总级）」变为「按席位 N 行（明细级）」。
签名不变、调用点无需改、编译器**零提示** ⇒ 属「编译器不设防」的改动，
必须人工枚举消费端查**行数假设**。

| 消费端 | 是否假设行数 | 结论 |
|---|---|---|
| `portfolio-mgr.rhai:997-1018`（筹码面机构信号） | 否 | `inst_net = (inst_buy − inst_sell) / (total_buy + total_sell)` 是**比率**，随后 `clamp(inst_net*2.0, -0.3, 0.3) * 0.15` 双向兜住 ⇒ **行数变多不会放大** |
| `AStockClient::truncate_dragon_tiger_by_asof`（`lib.rs:1028`） | 否 | 只按 `date <= as_of_date` 过滤，与行数无关 |
| `astock-data` 既有单测（`lib.rs:5020` 的 `dt()` 构造器） | 否 | `DragonTigerEntry` **字段集未变**（7 个字段）⇒ 无构造器连锁 |
| `data-quality.rhai` 的 `sent` / `hm` 报告判据 | 否 | 只做文本匹配 |

#### C. 新增 tripwire 的「会告警」证明

`portfolio-mgr.rhai:2048-2072` 的 `dt_seat_blank` 判据在修复后**恒为 false** ——
它是一条**回归哨兵**，不是活判据。按「0 命中的判据必须先证会告警」，两个方向都逐字核过：

- **会命中**：旧实现产出 `dept_name = "买入0席位/卖出0席位"` ⇒ 含「席位」且不含「营业部」⇒ 命中
  （该占位串形态已由 §三 的历史运行记录证实：14/14 轮恒为该值）
- **不命中**：真实营业部名（`…证券营业部`）与 `机构专用` / `沪股通专用` 均不含「席位」二字

> ⚠ 这一条是**构造性论证**（按字符串形态推演），**未**用可执行用例实测 ——
> 该判据内联在 2399 行的 `portfolio-mgr.rhai` 中，抽出执行需注册全套宿主函数，
> 成本远高于收益。此处如实标注，不冒充实测。

#### D. 测试自身的两处教训（已写进测试文件注释）

1. **手抄常量漂移**：`EXTERNAL_VARS` 是从节点 `input_mapping` **手抄**的清单，
   首次实测即因把 `money_flow` / `lockup_bundle` / `announcements` 误写成
   它们在脚本内**派生**出的变量名（`mf_main_net_inflow` / `lb_shareholder_trades_len` / `ann_count`），
   导致脚本执行到 828 行报 `ErrorVariableNotFound`。
   ⇒ 已加**等式门禁** `external_vars_match_node_input_mapping`：
   从 seed 源文件结构性抽取 `input_mapping` 的键集，与 `EXTERNAL_VARS` 做**双向差集**；
   并含提取器自检（键数 ≥39 + 5 个已知键探针），防「提取器失效 ⇒ 0 命中 ⇒ 恰好对上」的假绿。
   `input_mapping` 的权威来源是 `seed_stock_analysis.rs` 的 data-quality CodeNode（共 **39** 项）。
2. **Rhai 裸表达式语句返回 `()`**：`engine.eval::<String>(r#""a-b".replace("-","+")"#)`
   报 `ErrorMismatchOutputType("string", "()")` —— 无副作用的裸表达式被脚本优化器丢弃；
   必须写成 `let s = "a-b"; s.replace("-", "+")`。同一测试内的 `let p = …; p[0]` 形式则正常。

   实测细化（第二轮）：Rhai 脚本末句若是**方法调用**，返回值会被丢弃、整段返回 `()`
   —— 连 `let s = "a-b"; s.replace("-","+")` 也返回 `()`，而末句为**索引表达式**（`p[0]`）时正常。
   判别要点：报 `MismatchOutputType` **而非** `FunctionNotFound` ⇒ 函数**存在且跑通了**，
   只是值被丢 —— 别误读成「函数不可用」（`data-quality.rhai:134` 的历史注释
   「`replace` 100% 报 Function not found」即属此类误读，实测可执行）。
   最终改法：末句显式补 `()`，只断言 `is_ok`，与末句形态解耦。

#### E. 判据回归门禁实测结果（8/8 通过）

`__TAURI_WORKSPACE__=true cargo test -p axagent-rt-workflow --test data_quality_placeholder_gate` → **EXIT=0，8 passed / 0 failed**

| 测试 | 锁定的判据 | 结果 |
|---|---|---|
| `clarification_sentence_is_not_counted_as_gap` | 本轮 hm 真实报告：澄清句不再计缺口，真缺口仍计 1 处，`status` 仍为 `low` | ✅ |
| `hard_marker_survives_clarification` | 硬标记 `数据缺失` 不被澄清语连带抑制（防「修假阳性改成假阴性」） | ✅ |
| `verdict_comment_block_is_stripped` | `<!-- VERDICT -->` 不参与匹配；**反向对照**（正文自有标记仍命中） | ✅ |
| `historical_false_positive_samples_are_clean` | 3 类跨轮历史误报样本 0 命中 | ✅ |
| `genuine_gaps_still_detected` | 5 类真缺口语必检出（逐例标注命中标记名） | ✅ |
| `low_confidence_list_uses_same_threshold_as_diagnostics` | `diagnostics.status` 与 `low_confidence_analysts` 口径一致 | ✅ |
| `rhai_string_package_available` | 引擎契约钉子（`split` 可用；`replace` 可执行） | ✅ |
| `external_vars_match_node_input_mapping` | 手抄清单 ↔ `input_mapping` 双向相等（含提取器自检） | ✅ |

**两次失败都是测试自身的错，不是脚本的错**（首跑 6/8 → 修完 8/8）：
① `EXTERNAL_VARS` 手抄漂移（见 D.1）；② `genuine_gaps_still_detected` 第 1 例期望值估错
（把「同业横向估值**对比**缺失」误当成命中 `数据缺失`，实际只命中 `为 null`）。
⇒ 修法是**把期望值算出来而非估**：用 Node 按真词表对全部 9 条断言逐条复算后才重跑。

> **注**：`cargo fmt --all --check` 与 `tsc` / `dprint` / i18n 三道 / type-consistency 均 EXIT=0；
> `cargo clippy --all-targets --all-features -- -D warnings` 的结果见 §十二。

### 11.7 后续（同日 16:25）：面板「作用域错位」—— 用户质问「所有分析师节点都是这个结论，这是造假吗」

**现象**：打开任意一张分析师卡片的「数据质量」弹窗，顶部四数恒为 `79 / B / 6-4-0 / 共 10 项检查`。

**结论：不是造假，是展示层的**作用域错位** —— 顶部四数取自 data-quality 节点的**全局**输出对象，
与 `expertId` 无关；而 `data-quality.rhai` 的返回值里**根本没有** per-analyst 的 `score`/`grade`，
前端在物理上渲染不出「本节点的等级」。**

| 面板元素 | 绑定字段 | 作用域 |
|---|---|---|
| 79 圆环 / B 大字 | `report.score` / `report.grade` | **全局** |
| 6 / 4 / 0 三个 Tag | `good_count` / `degraded_count` / `gap_count` | **全局**（10 个分析师的分布计数） |
| 「共 10 项检查」 | `total_analysts` | **全局**（= 有 verdict 输出的分析师**个数**，不是检查项数） |
| 下方表格「本节点诊断」 | `resolveAnalystDiagnosis(expertId, …)` | ✅ per-node |

10 张卡片复用同一 modal（`AnalystReportCard.tsx:533 / :761`，只换 `name`/`expertId`）⇒ 顶部必然全同。

**数值本身是真的**：与 §一 的记录逐字一致（`score=79.2 / grade=B / good=6 / gap=0 / degraded=4`），
且 §七 的 8 轮横向表显示 score 跨轮各不相同（79.2 / 66.9 / 70.9 / 68.6 / 54.7(C) / …）
⇒ 恒等只发生在**同一轮内的不同节点之间**，属设计使然，**不是伪造**。

#### 改动 A —— 展示层加作用域标注（纯前端）

- 顶部区块新增标题「全局总览（N 个分析师聚合）」+ 一句说明（`dqGlobalScopeTitle` / `dqGlobalScopeNote`，11 语言齐备）。
- 「共 10 项检查」→「10 个分析师」：原 key `dataQualityCheckCount` 绑的却是 `total_analysts`，
  **文案与数据语义不符**。改用新 key `dqAnalystCount`，旧 key 在 11 语言中**全部删除**
  （零读取端的旧 key 是噪声，不留）。

#### 改动 B —— 补 per-node 事实量（前后端）

- `data-quality.rhai`：`diag_for` 新增第 7 参 `rq`，返回 map 补 `"report_quality": rq`；
  10 处调用点各补本节点的 `mk_q … cat_q`。
  ⚠ 这些 `*_q` **本来就算好了**（`:518-527`），只是**只被累加进全局 `report_quality_avg`、从未输出**
  ⇒ 典型「算了但没输出」，与 §八.2「诊断出来了 ≠ 进入评分了」同族。
- 面板表格新增「本节点报告质量」行（≥80 / ≥50 / 其余 三档**着色仅为快速指示，不是第二套等级**）。
- `DataQualityDiagItem` 增 `report_quality?: number`（可选 ⇒ 旧快照降级为「旧版快照无此字段」）。
- store 侧**无需改动**：`dataQualitySummary` 存的是整个节点输出 JSON 字符串，新字段自动随之下发。

⚠ **刻意不给 per-node 字母等级** —— 那正是 2026-09-14 才修掉的「两套同名等级」
（同一份报告在弹窗判 A、在决策链被判降级并把全局拉到 C）。本次只补**事实量**，
并在 `data-quality.rhai::diag_for`、`types/stock-analysis.ts`、面板组件三处注释里写明「不要顺手派生等级」。

#### 生效前置（不升版 = 改了不生效）

`data-quality.rhai` 经 `include_str!` 嵌入 stock-analysis 模板节点 `code`（`seed_stock_analysis.rs:3118`）
⇒ 必须升 `TEMPLATE_VERSION`：**70 → 71**（真库只读实测现值 = 70，取值判据见 §11.4）。
需应用重启（或任何触发 `seed_stock_analysis_workflow_template` 的路径）才会重种；
否则 DB 里仍是旧脚本，面板照旧不显示该行。

#### 真库只读实测（17:20 复核）：v71 事件**尚未发生**

不满足于「源码声明了 71」，直接连真库探针（只读，`workflow_templates`）：

| 探针 | 实测 | 含义 |
|---|---|---|
| `SELECT version … WHERE id='stock-analysis'` | **`70`** | 源码 `TEMPLATE_VERSION = 71`，**DB 仍 70** ⇒ 未重种 |
| `nodes LIKE '%ph_n, po_n, rq%'` | `false` | 库里的 rhai 仍是 **6 参**形态 |
| `nodes LIKE '%"report_quality": rq%'` | `false` | 新字段**不在** DB 模板中 |
| `nodes LIKE '%diag_for%'` | `true` | 脚本本身在（`nodes` 列 `length = 360241`，rhai 经 `include_str!` 整段嵌入） |
| `count(*) FROM workflow_template_versions WHERE template_id='stock-analysis'` | `41` | 历史重种次数 |

**对用户可见行为的直接影响（三条必须同时说，否则会被读成「改了没用」）**：

1. **A（纯前端）已立即生效** —— 作用域标注与新文案取自前端源码，dev / build 直接读源码，**不依赖后端重种**；
2. **B 的表格行会出现，但值走降级分支** ⇒ 显示「旧版快照无此字段」（后端此刻仍返 6 参旧脚本产出的 JSON）；
3. **已有历史运行快照永远不会有该字段** —— `dataQualitySummary` 存的是**每轮运行**的节点输出 JSON；
   即便重种成功，也需**重跑一次分析**才会出现真实数值。

**表名坑（探针方法学）**：模板节点**不在**独立的 `workflow_nodes` 表里，而是 `workflow_templates.nodes`
（**text** 列、JSON 串）。写库探针前先查 `information_schema.columns` —— 猜表名会拿到「关系不存在」，
这是**假阴性**：读数看起来像「没有这张表」，实际是「我猜错了名」。同族判据见 §11.4 的取值口径。

#### 验证（本轮）

| 门禁 | 命令 | 结果 |
|---|---|---|
| rhai 判据回归（**含本轮新增 per-node 断言**） | `cargo test -p axagent-rt-workflow --test data_quality_placeholder_gate` | EXIT=0，**9 passed**（原 8 + 新增 1） |
| rhai 全脚本编译 | 同上 `--test rhai_syntax_check` | EXIT=0，8 passed |
| 前端作用域回归（**本轮新增**） | `vitest run …/AnalystDataQualityModal.scope.test.tsx` | EXIT=0，4 passed |
| 股票分析全量前端 | `vitest run src/components/stock-analysis src/lib/__tests__` | EXIT=0，87 files / 1094 passed |
| `tsc --noEmit` | — | EXIT=0 |
| `dprint check`（改动文件） | — | EXIT=0 |
| i18n key 齐备（**自写校验**） | 11 locale × 6 新 key + 旧 key 零残留 | ALL GREEN |
| `scripts/check_i18n.py` | — | EXIT=0（代码引用的 key 在 zh-CN 全有定义） |
| `scripts/i18n-scan.mjs --strict` | — | EXIT=0 |
| `cargo check -p axagent --all-targets`（含 `seed_stock_analysis.rs` 常量改动） | — | EXIT=0（Finished in 8m33s） |
| `cargo clippy -p axagent-rt-workflow --all-targets -- -D warnings` | 首跑 **EXIT=101**：仅本轮新增测试 1 处 `unnecessary_get_then_check`（`d.get(..).is_some()` → `d.contains_key(..)`）⇒ 已修 | **复跑 EXIT=0**（2m53s） |
| rhai 判据门禁复跑 | 同上 `--test data_quality_placeholder_gate` | **EXIT=0，9 passed** |
| `cargo fmt --check`（全仓） | 首跑 EXIT=1：仅本轮新增测试 2 处 ⇒ 已修。**复跑仍 EXIT=1，但报错文件已换成 `crates/analysis-engine/src/position_limits.rs:63` —— 非本轮改动** | 见下 |
| `cargo fmt -p axagent-rt-workflow --check` / `cargo fmt -p axagent --check` | 定点只查本轮触及的两个包 | **均 EXIT=0** |

**并发归因：`position_limits.rs` 不是本轮改的。** 该文件 mtime = `08:53:49`，
落在本轮改动区间内（`data-quality.rhai` 08:27 / 本轮测试文件 08:57）却**从未被本轮触碰**；
更关键的是它在**第一次** `cargo fmt --check`（08:45 前跑完）里**并未报出**，是**复跑时才浮现** ——
两次检查之间只发生了本轮对新测试文件的两次编辑，且该文件不在其中
⇒ 结论：**有并发会话在本任务进行中写入了未格式化代码**。
按纪律**只登记、不代改**（改它既越权，也可能与并发写入冲突；且它不影响本轮的验证结论，
因为定点 `cargo fmt -p` 对本轮触及的两个包均为 EXIT=0）。

**两条红都落在同一个新增文件里，且都是本轮自己引入的** —— 这正好复现
`AGENTS.md` 已写明的约束：「必须 `--all-targets`，否则 test / bench target 不被 lint，
而**新写的测试代码正是高发区**」。三次门禁的报错文件分别为
`data_quality_placeholder_gate.rs:424 / :440 / :423`，全部是新测试的形态问题，
**没有一条指向生产代码或既有文件** —— 这是区分「本轮引入」与「既有红」时的关键读数。

**新增断言为什么必须是「穷举 10 个节点」**：Rhai **没有编译期检查**，
`diag_for` 加参数后若漏改某个调用点，`engine.compile` 与其它测试都可能照过
（错误只在执行到那一行时才炸）。故 `diagnostics_expose_per_node_report_quality` 逐一断言
10 个分析师都存在该字段，再断言「有报告 > 0 / 无报告 = 0 / 两节点可区分」——
最后一条正是本次修复的目的（若该字段退化成常量，面板仍显示同一组数字）。
思路与既有 `external_vars_match_node_input_mapping` 的「双向锁」一致：
**手抄 N 处无法单点声明时，就用穷举把漏改钉死。**

**本轮踩到的两个「测试自身」的坑**（都是判据写错，不是被测对象错，值得复用）：

1. **antd Modal 走 portal 挂 `document.body`，同一用例内连续 `render` 两次会串台** ——
   反向断言 `expect(hotMoney).not.toContain("…score=88")` 假红，读起来像「取错了诊断条目」，
   实为上一个弹窗未清理。修法：把 `cleanup()` 放进渲染辅助函数内（而非只放 `beforeEach`）。
2. **`cargo fmt --check` 的首跑红是本轮自己引入的** —— 报错文件正是新增测试，
   与 §12.3 记录的「既有红」是两回事；**区分二者是采信门禁结果的前提**，
   不能因「有一处红」就推定整批改动不合规，也不能因「大部分绿」就忽略它。

### 11.8 后续（同日 17:04–17:20）：逐节点表格补「报告质量」列

§11.7 里登记为「**属产品决策、未擅改**」的那一项（`DecisionBanner` 逐节点表格加列），
用户明确要求实施后落地如下。

| # | 文件 | 改动 |
|---|---|---|
| 1 | `src/lib/dataQualityDiagnosis.ts` | **新增 `reportQualitySeverity(rq): DiagSeverity \| null`** —— 把三档阈值抽成**单一来源** |
| 2 | `AnalystDataQualityModal.tsx` | 改用该 helper；**新增第 4 态 `unknown`**（`MinusCircleFilled` + `var(--muted)`） |
| 3 | `DecisionBanner.tsx` | 逐节点表加「报告质量」列（列序 **4**，紧跟「失败标记」）；`undefined ⇒ "—"` + tooltip；档位取 helper，调色板沿用本文件的 tailwind 色阶 |
| 4 | `locales/*.json` × 11 | 新增 **1** 个 key `stockAnalysis.dqTableReportQuality`（真翻译） |
| 5 | `__tests__/DecisionBanner.test.tsx` | 新增 4 用例 + **升级天真 i18n mock** |
| 6 | `src/lib/__tests__/dataQualityDiagnosis.test.ts` | **新建**，6 用例锁三档边界 |

**为什么先抽 helper 再动组件**：三档阈值若在 modal 与表格**各写一份**，就会出现
「**同一个数值在两处显示不同颜色**」—— 正是本日全仓在清的「同语义多份判据」形态。
故先收敛到 `@/lib/dataQualityDiagnosis`，两处都只调它，并各自保留既有调色板
（modal 用 antd 语义色、表格用 tailwind 色阶；**函数只出档位不出颜色**，不要误以为色值可复用）。

**`0` 与 `undefined` 必须分开**（本轮顺带修正的一处旧行为）：
`0` 是**合法值**（该节点未注入报告，`report_quality("", conf)` 的取值），
`undefined` 是**旧快照缺字段**。此前 modal 把 `undefined` 归入 `good`
—— 绿色对勾旁边写着「旧版快照无此字段」，等于**把「不知道」渲染成「好」**。
现 modal 走第 4 态 `unknown`（灰点），表格走 `—` + tooltip。

**验证（本轮全绿）**：

| 门禁 | 结果 |
|---|---|
| `tsc --noEmit` | EXIT=0 |
| `dprint check`（本轮 17 个文件） | EXIT=0 |
| `oxlint --max-warnings=0` | **全量** `src` EXIT=0（1262 files / 0 warnings） |
| `vitest run src/components/stock-analysis src/lib/__tests__/dataQualityDiagnosis.test.ts` | **28 files / 155 passed** |
| 本轮三项测试（DecisionBanner / Modal.scope / lib 新测试） | **3 files / 20 passed** |
| `scripts/check_i18n.py` / `i18n-scan.mjs --strict` | 均 EXIT=0 |
| 11 语言键集奇偶（自写校验） | **15129 keys，0 missing 0 extra** |
| `vitest run`（**全量**） | 170 passed / **4 failed**（2004 tests）⇒ 见下「负载型 flaky」 |

**全量的 4 个失败 = 负载型 flaky，非本轮引入。** 失败项为 `App.d2`（markstream D2 loader）、
`DemandDiscoveryPage`（OPC 录入）、`McpServerSettings`（env blur）、
`ScreenerToWorkspace.integration`（超短线 tab 跳转）—— 分布在**完全无关的域**，
失败时耗时 **5–25 s**（超时特征，全量并行下 CPU 争用）；**单跑这 4 个文件 = 4 passed / 15 tests，EXIT=0**。
⇒ 归因判据仍是三条：**单跑是否复现 / 域是否相关 / 耗时是否呈超时特征**。
（与上轮记录的 `DemandDiscoveryPage` 并发 flaky 同一形态。）

**本轮追加 2 个用例锁第 4 态 `unknown`**（`MinusCircleFilled` 计数为 1 / 0 两向），
使「把『不知道』渲染成『好』」这个旧行为不可能静默回归。

**两条既有约定（登记，未改）**：

1. **`.gitignore:54` 有 `docs/`** ⇒ dprint 的文件发现**完全不含 `docs/**`**
   （`dprint check docs/…md` 报 `No files found to format with the specified plugins…`，退出码 **14**）。
   即：本目录下的审计报告**不受格式门禁覆盖**，也**不进版本库**（本地留档）。
   ⚠ 看到退出码 14 不要当格式错 —— 它是「**没找到文件**」。
2. `zh-TW.json` 的 `workspace.collapseComparison` 为空串（其余 10 语言有值）；
   `viz.text` / `viz.title` 在 11 语言全空（疑似有意）。

**本轮三个坑 —— 全部是「测试自身」，不是被测对象**（与 §11.7 的两条同族）：

1. **antd Modal 走 portal** ⇒ 数据质量表格位于「显示详情」Modal 内部（`open={expanded}`，初值 false）：
   既不点开**根本不渲染**，渲染了也**不在 `render()` 返回的 container 里**。
   首版在 container 里查 `thead th` 恒得空数组，4 个用例全红，而报错读起来像「列没加进去」。
   修法：`fireEvent.click` 打开 → 断言面换 `document.body` → 用**表头特征文本**定位目标表格。
2. **天真的 i18n mock**：`t: (key, fallback) => fallback ?? key` 遇到
   `t(key, { good, total, avg })` 会把**插值对象**当 React child 渲染 ⇒
   `Objects are not valid as a React child (found: object with keys {good, total, avg})`。
   修法：按形态分流（字符串 ⇒ 兜底原文；对象 ⇒ 拼成 `key|k=v`）。
3. **弱断言不足以定位**：`container.textContent).toContain("88")` 不能证明它落在「报告质量」列
   （同行的**置信度**可能正好也是 88）⇒ 用 `tr.children[4].textContent` + 一条**表头顺序**断言互锁。

**登记（未改）**：`zh-TW.json` 的 `workspace.collapseComparison` 是**空字符串**（其余 10 语言有值）；
`viz.text` / `viz.title` 在 11 语言**全空**（疑似有意）。属**文案决策**，未擅改。

## 十二、验证实录（2026-09-21）

改动共触及 4 个 crate + 前端。下表把「跑过的门禁 / 命令 / 结果 / 归属」一并留档，
便于复现与区分**本轮引入**与**既有红**。

### 12.1 与本次改动直接相关的门禁

| 门禁 | 命令 | 结果 |
|---|---|---|
| Rust 格式化 | `cargo fmt --all --check` | ✅ EXIT=0 |
| 判据回归（**核心证据**） | `cargo test -p axagent-rt-workflow --test data_quality_placeholder_gate` | ✅ **8 passed / 0 failed** |
| 取数 crate 测试 | `cargo test -p axagent-astock-data --all-targets` | ✅ **336 passed / 0 failed**（lib 324 + parsing 5 + vendor 7），含 `truncate_dragon_tiger_*` 两条 |
| 前端类型 | `tsc --noEmit` | ✅ EXIT=0 |
| 前端格式 | `dprint check --incremental=false` | ✅ EXIT=0 |
| 前端类型一致性 | `scripts/type-consistency.mjs` | ✅ 0 errors / 436 warnings（警告级） |
| i18n 键对齐 | `python scripts/check_i18n.py --strict` | ✅ 全部通过（代码引用的 key 全有定义） |
| i18n 硬编码 | `scripts/i18n-scan.mjs --strict` | ✅ EXIT=0 |
| i18n 未翻译值 | `scripts/check-i18n-untranslated.mjs` | ✅ EXIT=0 |
| 行尾一致性 | `bash scripts/check-line-endings.sh` | ✅ EXIT=0（新增 `.rs` 与 11 个 locale 均为**纯 LF**） |
| Rust lint（全量，AGENTS.md 口径） | `cargo clippy --workspace --all-targets -- -D warnings` | ⚠ `EXIT=0` 但 **0 行 `Checking`** ⇒ 自身未检查任何 crate（不构成「本轮跑过」的证据；旗标变体互不通用 ⇒ 必有一次同旗标干净通过，见 12.4 / 12.4.7）|
| Rust lint（**CI 逐字**） | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ **EXIT=0**（真实重检 `axagent` / `agent` / `search` / **`rt-workflow`** / `tools` / `runtime`）见 12.4.1 |
| Rust lint（定点，补 `astock-data` / `harness`） | `cargo clippy -p axagent-astock-data -p axagent-harness --all-targets -- -D warnings`（先 touch 强制重算） | ✅ **EXIT=0**（含 `Checking axagent-harness` + `Checking axagent-astock-data`）见 12.4.3 |

### 12.2 CI 前端门禁逐条（`ci-check.mjs --frontend-only` 因既有红中断，故改为逐条单跑）

✅ 领域语义自检 / 门禁 · 分层护栏自检 · 领域本体自检 / 一致性 · 能力域单一真相源自检 / 门禁 ·
声明表 evidence 自检 / `--ci` · 端口公理写路径自检 / 接线 · input_mapping 自检 / 源路径 ·
同一事实多份载体自检 · ID 数据边界 · 后端错误码 i18n 对齐 · SQL 方言守卫自检 / 正式

> ⚠ `ci-check.mjs` 遇首个失败即 `process.exit(1)`，**后续步骤全部未执行** ——
> 这不代表它们通过。故必须逐条单跑，不能用「跑了 ci-check 没报别的错」当结论。

### 12.3 既有红（**非本轮引入，只登记不代改**）

| 门禁 | 报红内容 | 归属举证 |
|---|---|---|
| `check-layer-discipline.mjs` | 超基线 3 处：`backtest_validation.rs` 的 `commands-no-direct-db` 10>9、`commands-no-sibling-call` 5>1；`crates/crdt` 的 `crate-has-consumer` 1>0 | 文件 mtime：289.8 / 26644.7 分钟前；本轮改动全在 35 分钟内 |
| `check-single-source-facts.mjs --ci` | `domain_ontology.rs` 引用 `bottleneck-calc.rhai` 越界 9 处（183/192/198/200/211/212/213/214/221，而该文件仅 **180** 行） | 引用方 mtime 5969.9 分钟前、被引用文件 895.6 分钟前；且被引用文件与本轮改的 `data-quality.rhai`（1080 行）**不是同一个文件** |
| `check-rust-raw-map-err.mjs` | 10 处裸 `map_err(\|e\| e.to_string())` | **全部集中在 `market_mainline.rs` 一个文件**，mtime 322.5 分钟前 |

### 12.4 关于 clippy：两条口径 + **结果的采信判据**

- AGENTS.md 的门禁写作 `cargo clippy --workspace --all-targets -- -D warnings`；
- `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml` 实际执行的是
  `cargo clippy --all-targets --all-features -- -D warnings`（**多 `--all-features`、少 `--workspace`**；
  在无 `default-members` 的 workspace 根执行等价于全量成员）。

两者差异在 `--all-features`：它会额外编译 feature-gated 代码。本轮改动**未新增任何 feature**，
但为与 CI 逐字同源，三条都跑了（结果见下）。

**执行状态**：

| # | 命令 | 结果 | 采信度 |
|---|---|---|---|
| ① | `cargo clippy --workspace --all-targets -- -D warnings` | 01:41 发起 → **01:52:35 完成，`CLIPPY_EXIT=0`**（`Finished in 11m 53s`） | ⚠ **借用他人结论** —— 0 行 `Checking`，自身未检查任何 crate；但旗标变体互不通用 ⇒ 该 fresh 状态必来自一次**同旗标**的干净通过（见下）|
| ② | `cargo clippy --all-targets --all-features -- -D warnings`（**CI 逐字**） | 01:53 → **02:21:35 完成，`CLIPPY_CI_EXIT=0`**（真实重检 15 个 crate） | ✅ **采信**（12.4.1）|
| ③ | `cargo clippy -p axagent-astock-data -p axagent-harness --all-targets -- -D warnings`（touch 强制重算） | 02:23 → **02:39:03 完成，`CLIPPY_PIN_EXIT=0`**（`Checking` 命中被 touch 的两个 crate） | ✅ **采信**（12.4.3）|

> 最终结论：**本轮 4 个改动 crate 全部通过 clippy**（证据矩阵见 12.4.5）。
> ① 自身**未检查任何 crate**，故**不构成「本轮跑过」的证据**；真正闭环靠 ② + ③（+ 实验 A 交叉坐实）。

**为什么 ① 不能当「本轮跑过」的证据** —— 分「已实测」与「已证伪的旧假说」两层写，不混：

**已实测（可采信）**：

1. **`Finished in Xs` 的计时含锁等待**。`01:41:42 发起 → 01:52:35 完成` = 11m35s ≈ 报告的 `11m 53s`，
   而该次运行的日志里 **0 行 `Checking` / `Compiling`**。
   ⇒ 这 11m53s **不是**编译时间；本次运行**没有重建任何单元**。
   ⇒ 由此得到一条可复用判据：**「耗时长」不能当作「真编译了」的证据**。
2. **`Finished` 报 0 单元重建时，该次运行的 lint 结论不覆盖任何 crate** ——
   这是 `cargo` 增量缓存的固有语义：没有 `Checking` 行 ⇒ 没有任何 crate 在这一轮被检查。
3. 本仓判据 **#72 ⑥** 已给出正确口径：判定要拿「**本轮改动 crate 的传递下游闭包**」
   去比对 `Checking` 名单；**缺了受影响 crate ⇒ 必须显式补跑**，「EXIT=0」不能替代这一步。
   本轮 ① 的名单为**空集**，而我的改动涉及 4 个 crate ⇒ **依 #72 ⑥ 即为不合规**，
   故补跑 ② ③，并另做实验 A 作为交叉证据（见 12.4.4）。

**旧假说（⚠ 已被实测推翻，见下）**：

- 我曾推断：`-D warnings` 经 `CLIPPY_ARGS` 环境变量传给 `clippy-driver`，而 cargo 的单元指纹
  可能不哈希该环境变量 ⇒ 若缓存由**不带 `-D warnings`**的运行产生，则带 `-D warnings` 的运行会
  因「无事可做」而**不重建、不重报警告**，从而产出 `EXIT=0` 假绿。
- **该假说已被推翻**（2026-09-21 03:02~03:05 的受控实验，见 12.4.7）。
  实测：两种旗标变体的**新鲜度互不共享** —— 若真的「不进指纹」，[3] 必然命中 [2] 刚建立的状态，
  而 [3] 重编了。⇒ **「缓存由不带 `-D` 的运行产生」这一前提在本环境不成立**，
  故「带 `-D` 却复用不带 `-D` 的缓存」这条路径**不存在**。

**⇒ 由此对 ① 的重新评价（修正前文的「不采信」）**：

- 前提：`-D warnings` 与不带 `-D` 的编译状态**互不通用**（实测）。
- 那么 ① 报 0 行 `Checking` 只能有一种解释：**先前存在一次旗标完全相同的运行**
  （`--workspace --all-targets -- -D warnings`），在那份源码上跑过并通过。
- 而在 `-D warnings` 下**任何 warning 都会变成 error**，进程会非 0 退出、该单元**不会被记为成功**
  ⇒ 「fresh」本身**蕴含有一次 `-D` 下的干净通过**。
- 结论：**① 的 `EXIT=0` 方向上可信，但它不是「本轮独立跑过」的证据** ——
  它借用了他人的结论，自身没有检查任何 crate。
  **证据链仍以 ② ③ 为准**（附实验 A 交叉坐实零警告）。
- ⚠ 顺带修正：① 的用户可观察信号「`Finished in 11m 53s`」依然**不可用作「真编译了」的证据** ——
  那段时间是排队等锁（见上文第 1 点）。**只有 `Checking` 行数能说明检查面。**

⇒ 既然 ① 不可用，就需要另起运行补出证据。首先想到的是 **CI 逐字**那条
（`--all-targets --all-features -- -D warnings`）—— 它既闭合 CI 口径，又可能因 flag 不同而强制重算。
注意我原先的理由「`--all-features` 会改变 feature 统一结果，故必然重算」**只对有 `[features]` 的 crate 成立**
（见 12.4.2 的实测修正）；但实测 ② **确实**做了真实重算，故 ② 的结果可信。

#### 12.4.1 ② 的结果（CI 逐字，**可信**）

| 项 | 值 |
|---|---|
| 命令 | `cargo clippy --all-targets --all-features -- -D warnings`（与 `scripts/ci-check.mjs`、`.github/workflows/ci.yml` 逐字一致） |
| 起止 | 01:53:04 → 02:21:35 |
| cargo 报告 | `Finished 'dev' profile in 28m 30s`（**含 ~19 分钟等锁**，因为 `start→取到锁` 就花了那么久） |
| **真实重算证据** | ✅ 日志含 **11 行 `Compiling` + 4 行 `Checking`**，覆盖 `axagent`（主 crate）、`axagent-agent`、`axagent-search`、**`axagent-rt-workflow`**、`axagent-tools`、`axagent-runtime`、`tauri` / `wry` / `tauri-runtime-wry` 及 10 个 `tauri-plugin-*` |
| 结论 | ✅ **`CLIPPY_CI_EXIT=0`，0 warning**（`-D warnings` 下任何 warning 都会变成 error 并退出非 0） |

`Checking axagent-rt-workflow` 这一行直接覆盖了**本轮新增的测试文件**
`crates/rt-workflow/tests/data_quality_placeholder_gate.rs` —— 即本轮新增测试代码**已过 clippy**。

#### 12.4.2 实测修正：`--all-features` **不一定**强制重算

② 的日志里**没有** `axagent-astock-data` / `axagent-harness`。查明原因：

```
crates/astock-data/Cargo.toml  → 无 [features] 段，无 optional 依赖
crates/harness/Cargo.toml      → 无 [features] 段
```

⇒ 这两个 crate **没有任何 feature 可变**，`--all-features` 与默认解析出**完全相同**的 feature 集
⇒ 指纹不变 ⇒ **不被重检**。

> **结论（推翻我在 12.4 初稿里的写法）**：`--all-features` 只对**声明了 `[features]` 的 crate**
> 改变指纹；对无 feature 的 crate 等于没加。**「换个 flag 就能强制重算」这个直觉不能照搬** ——
> 必须先确认目标 crate 的 feature 面。
>
> 真正**无条件**强制重算的手段是**改源文件 mtime**（`touch`）或换 `--target-dir`。
> 前者代价小，但需先证明**内容零变化**（sha256 前后一致）—— 见 12.4.3。

#### 12.4.3 定点强制重算（补 `astock-data` / `harness` 的 clippy 证据）

因上述原因，`astock-data`（本轮改了 2 个文件）与 `harness`（本轮改了 1 个文件）在本会话内
**始终没有 clippy 的正向证据**。故 touch 这 3 个文件强制重算：

| 文件 | sha256（前 12） | 内容 | mtime |
|---|---|---|---|
| `crates/astock-data/src/vendors/eastmoney.rs` | `d686962e4d22` | 未变 ✅ | 已推进 ✅ |
| `crates/astock-data/src/vendors/browser_eastmoney.rs` | `dd988b313316` | 未变 ✅ | 已推进 ✅ |
| `crates/harness/src/domain_semantics.rs` | `a264d3116fef` | 未变 ✅ | 已推进 ✅ |

⇒ 3 个文件**逐字节未变**，仅 mtime 推进。随后定点跑
`cargo clippy -p axagent-astock-data -p axagent-harness --all-targets -- -D warnings`：

| 项 | 值 |
|---|---|
| 起止 | 02:23:00 → 02:39:03（`Finished in 16m 01s`，其中约 14 分钟是等锁） |
| **真实重算证据** | ✅ `Checking axagent-harness v2.9.9` + `Checking axagent-astock-data v0.1.0` —— 正是被 touch 的两个 crate |
| 结论 | ✅ **`CLIPPY_PIN_EXIT=0`，0 warning** |

#### 12.4.4 交叉实验 A：不带 `-D warnings` 的重检（**证 `astock-data` 零警告**）

run ③ 是用 `-D warnings` 拿到的 `EXIT=0` —— 但 `EXIT=0` 在 `-D warnings` 下有两种含义：
「没有警告」或「警告没被重新报出」。为把两者分开，另做一次**不带 `-D warnings`** 的强制重检：
`touch` `eastmoney.rs` 后跑 `cargo clippy -p axagent-astock-data --all-targets`（无 `--` 参数）。

| 项 | 值 |
|---|---|
| 起止 | 02:40:12 → 02:41:19（`Finished in 1m 06s`） |
| `Checking` 行 | ✅ `Checking axagent-harness` + `Checking axagent-astock-data` |
| **`warning` 行数** | **0** —— 日志全文只有 2 行 `Checking` + `Finished` + `A_EXIT=0` |
| 结论 | ✅ 该次运行**真实重检**了两个 crate，且**一条警告都没发出** |

⇒ 这条证据的价值在于**它不依赖 `-D warnings`**：既然重检后**零警告被输出**，
那么「有没有 `-D`」这个变量**与 `astock-data` / `harness` 的结论无关** ——
run ③ 的 `EXIT=0` 因此被独立坐实，**不是**「警告没被重报」造成的。

> **补充（12.4.7 完成后回看）**：这层论证现在**更硬**了 —— 既然两种旗标变体**互不共享编译状态**，
> 那么这次「不带 `-D`」的运行**必然**是真实的重新编译（而非复用 `-D` 的产物），
> 故它输出的「零条 `warning`」是对 `astock-data` / `harness` 的**一手读数**，无任何复用嫌疑。

> 附带产出：该实验本是为验证 12.4 的「旧假说」而设计的 A/B 两段（A 段如上）。
> **B 段最终取到读数**（第三次尝试，19:00:26 完成，日志见 `output/tmp-fp-probeB.log`）：
> `Checking axagent-harness` + `Checking axagent-astock-data`，`Finished in 16m 45s`（含等锁）。
> ⇒ **A 与 B 之间闭包内零源码改动**（最新 `.rs` mtime 18:40:12Z < A 结束 18:43:22Z），
> 唯一差异是 `-- -D warnings`，而 B **重编了** ⇒ 直接推翻旧假说。
> 为把它做成受控结论，另跑了三连实验，见 **12.4.7**。

#### 12.4.5 clippy 证据汇总（本轮 4 个改动 crate 全覆盖）

| 本轮改动的 crate | 改动内容 | 覆盖它的 clippy 运行 | 该次运行的 `Checking` 行 | 结果 |
|---|---|---|---|---|
| `axagent-astock-data` | `vendors/eastmoney.rs`（`get_dragon_tiger` 重写、`get_peers` 补估值）、`vendors/browser_eastmoney.rs` | ③ 定点（touch 强制）**＋ 实验 A（不带 `-D`）** | ✅ `Checking axagent-astock-data` ×2 | ✅ EXIT=0（实验 A 另证**零警告输出**）|
| `axagent-harness` | `src/domain_semantics.rs`（`dqi_score` 的 evidence 行号） | ③ 定点（touch 强制）**＋ 实验 A** | ✅ `Checking axagent-harness` ×2 | ✅ EXIT=0 |
| `axagent-rt-workflow` | **新增** `tests/data_quality_placeholder_gate.rs` | ② CI 逐字 | ✅ `Checking axagent-rt-workflow` | ✅ EXIT=0 |
| `axagent`（主 crate） | 经 `include_str!` 嵌入的 `data-quality.rhai`（rhai 不参与 Rust lint，但主 crate 已重检） | ② CI 逐字 | ✅ `Compiling axagent` | ✅ EXIT=0 |
| — | （对照）① 全量 | — | ⚠ **空集** | ⛔ 自身**不覆盖任何 crate** ⇒ 不构成「本轮跑过」的证据（但旗标变体互不通用 ⇒ 该 fresh 必来自一次同旗标的干净通过，见 12.4.7）|

> **判据（可复用）**：一条 clippy 结果要能采信，必须同时满足
> ① `EXIT=0`；② 日志中出现**覆盖目标 crate 的 `Checking` 行**。
> 缺 ② 时 `EXIT=0` 只说明「cargo 认为无事可做」，**不说明 lint 跑过**。
> 这与本仓判据 **#72 ⑥** 同一口径（拿「改动 crate 的下游闭包」去比对名单，缺了必须显式补跑）。
> **强制重算的有效手段**（按可靠性排序）：
> 1. **touch 源文件**（先证 sha256 未变）—— 无条件可用，实测有效（12.4.4）；
> 2. **换 `--target-dir`** —— 无条件可用，代价 = 全量重编（本机 sccache 为空，不划算）；
> 3. **改旗标变体**（如加/去 `-- -D warnings`）—— 实测确实换状态，**但会与旧状态互相失效**，
>    交替跑等于每一步都重编（12.4.7）；且 `--all-features` **只在目标 crate 有 `[features]` 时**才有效（12.4.2）。
> ⇒ **推荐姿势：固定一种旗标变体（与 CI 同源那条），需要强制重算时用 touch**。
> 不要在验证过程中来回切换旗标 —— 那会白付多次重编。

#### 12.4.6 环境背景（非代码问题）

同项目**多会话共用一个 `src-tauri/target`**。判定归属只看 `Get-CimInstance` 的 `CommandLine` ——
期间观测到至少 3 个并发 `cargo clippy --workspace --all-targets`、2 个 `cargo check --workspace`、
1 个 `cargo test --workspace`、1 个 `cargo run --no-default-features`（有人在跑应用）。

> 排障要点：**`rustc` 进程为 0 不代表死锁** —— cargo 在**跑测试**时同样持有 build 锁。
> 另一条：**`Finished in Xs` 的计时含锁等待**，故「耗时长」不能当作「编译工作量大」的证据。

> 备用证据（与 clippy 相互独立）：`cargo check`/`test` 已覆盖本轮 4 个 crate 的类型正确性
> （`astock-data` **336 测试**、`rt-workflow` **8 测试**全过）。

#### 12.4.7 受控实验：两种 clippy 旗标变体的**新鲜度互不共享**（推翻旧假说）

**设计**：把「带 `-- -D warnings`」与「不带」两种变体**背靠背**交替跑，中间**不做任何源码改动**，
看每一步是「命中（0 行 `Checking`）」还是「重编（有 `Checking`）」。控制变量只剩那一个旗标。

| 步 | 命令 | 耗时 | `Checking` 行 | 判定 |
|---|---|---|---|---|
| [1] | `cargo clippy -p axagent-astock-data --all-targets -- -D warnings` | **1.73s** | **0** | 命中（复用上一步 `-D` 状态）|
| [2] | `cargo clippy -p axagent-astock-data --all-targets`（**不带 `-D`**） | **1m 10s** | 2（`harness` + `astock-data`） | **重编** |
| [3] | 同 [1]（**带 `-D`**） | **1m 02s** | 2（`harness` + `astock-data`） | **重编** |

三步之间**零源码改动**（同一 shell、连续执行）。三条读数互相锁定：

- **[1] 命中** ⇒ 同变体之间新鲜度可复用（距上一次同变体运行已约 17 分钟）；
- **[2] 重编** ⇒ 换变体后**不复用**；
- **[3] 重编** ⇒ 换回原变体**也不复用** [2] 刚建立的状态。
  ⚠ 若两种变体共享同一份新鲜度，[3] **必须**是命中 —— 实测重编，故 **共享假设被证伪**。

**⇒ 结论（纯读数，不含机理推断）**：**带 `-- -D warnings` 与不带 `-D` 的 clippy 占两套互不通用的编译状态**，
交替运行会**每一步都重编**。

**⇒ 三条实践含义**：

1. **旧假说被推翻**：「缓存由不带 `-D` 的运行产生 ⇒ 带 `-D` 会复用 ⇒ 警告不重报 ⇒ 假绿」
   —— 该路径在本环境**不存在**。
2. **验证命令要固定一种变体**，不要在「带 `-D`」与「不带」之间来回切 —— 每次切换都付一次重编
   （本仓 3 个 crate 约 1~2 分钟，全 workspace 更久）。
3. 顺带解释了一个此前疑点：实验 A（不带 `-D`）与 B（带 `-D`）**写入的指纹目录不同**
   （`bcefa6b2…` vs `edf86219…`），两目录 JSON 的差异字段是 **`profile`**（只有两个固定取值），
   而 `rustflags` / `features` 完全相同。

> ⚠ **机制未查明，不写成机理**：本轮只确定「两套状态互不通用」这一**行为**。
> 实测到的不一致点：指纹目录里 **A（不带 `-D`）落 `bcefa6b2`，而 [2]（同样不带 `-D`）落 `edf86219`**
> —— 说明「目录 ↔ 旗标」**不是简单一一对应**（`--all-targets` 下 lib 会以普通模式与 test 模式各编一次，
> 而本探针只跟踪了 `lib-axagent_astock_data.json` 这一个文件名）。故此处**只留行为与判据，不写成因**。
> 复现脚本：`output/tmp-list-fp.mjs`（列出该 lib 单元全部指纹目录及 `profile`）；
> 原始日志：`output/tmp-fp-decide.log`、`output/tmp-fp-pingpong.log`。

### 12.5 关于 vitest 的 5 个失败（**非缺陷，已证伪**）

全套 `vitest run` 报 `5 failed / 1923 passed`，失败项：
`DemandDiscoveryPage` ×2（`Test timed out in 15000ms`）、`McpServerSettings`（timeout）、
`kgFusedScale`（timeout）、`DatabaseSettings`（`expect(sw).toBeDisabled()` 抢跑）。

**证伪步骤**（三重证据）：
1. **隔离复跑**这 4 个文件 → **4 files / 45 tests 全过，EXIT=0**（耗时从 24s 降到 6s）
2. **零引用交集**：4 个失败测试文件对 `stock-analysis` / `AnalystDataQualityModal` / `dqPlaceholder` 的引用数 = 0
3. **mtime**：失败文件 4244 ~ 58987 分钟前（数天前），本轮改动 1~35 分钟

根因是**并发负载**：当时有 4 个 cargo 进程抢 CPU（另有两个会话在跑 workspace check）。
`DatabaseSettings` 那条的具体机理：`Form.useWatch("db_type")` 首渲染返回 `undefined`，
`disabled={undefined === "sqlite"}` 尚未置上而断言已抢跑。

---

## 十三、追加复核与增补修复（2026-09-21 凌晨）

> 触发：用户再次下达「根据以上分析修复代码」。本节先做**逐条反向核对** ——
> 直接锚定被测对象自身形态（grep 源码 / 重跑门禁），**不引用 §十一 的报告结论**，
> 以排除「报告说改了、文件其实没改」这类假绿；再处理 §12.3 里可安全修的既有红。

### 13.1 §九 逐条落地复核（grep 锚点，非引用结论）

| # | 复核锚点（源码实态） | 结果 |
|---|---|---|
| P0-1 / P0-2 | 不再读 `BUY_SEAT_NEW`，改接 `RPT_BILLBOARD_DAILYDETAILSBUY/SELL`，`dept_name` ← `OPERATEDEPT_NAME` | ✅ `eastmoney.rs:953,1004`；`browser_eastmoney.rs:366,406` |
| P1-1 | `get_peers` 接 `RPT_VALUEANALYSIS_DET`；`roe` 显式保持 `None` | ✅ `eastmoney.rs:2285,2296` |
| P1-2 | `strip_verdict_blocks` / `soft_marker_suppression` | ✅ `data-quality.rhai:202,212` |
| P1-3 | `dt_seat_blank` 内容级判据 | ✅ `portfolio-mgr.rhai:2048,2066,2070` |
| P2-1 | `expected_data` 归因修正 | ✅ 同在 `data-quality.rhai` |
| P2-2 | `degraded` 不入惩罚（代码内写死理由） | ✅ |
| P2-3 | `placeholder_occurrences` 口径 + 输出字段 | ✅ `data-quality.rhai:259,1060`；`AnalystDataQualityModal.tsx:400`；`types/stock-analysis.ts:447`；11 语言齐 |
| P2-4 | 判据不对称说明 | ✅ `AnalystDataQualityModal.tsx:441` + 11 语言 |
| 版本门 | `TEMPLATE_VERSION` | ✅ `seed_stock_analysis.rs:128` = **64** |
| 回归防线 | 可执行判据门禁文件存在 | ✅ `crates/rt-workflow/tests/data_quality_placeholder_gate.rs` |

**当前时点实测**（非引用上轮记录）：

```
__TAURI_WORKSPACE__=true cargo test -p axagent-rt-workflow --test data_quality_placeholder_gate
→ 8 passed / 0 failed，EXIT=0（0.10s）
```
日志含 `Compiling axagent-harness / kit / entities / dao / rt-workflow` ⇒ **真实重编**，
按 #635 采信判据（须有覆盖改动 crate 的 `Compiling`/`Checking` 行）**有效**。

### 13.2 增补修复：`market_mainline.rs` 10 处裸 `map_err`（§12.3 第 3 项 → 已消）

**归属**：该文件 2026-09-20 刚被整体重写（含「历史订正」注释：10 个 `Err("功能已移除")` 存根 → 真实调用），
裸 `map_err` 是那次**新写代码**遗留；mtime 514 分钟前（非在飞）。

**判据**：`scripts/check-rust-raw-map-err.mjs` 是**硬拦**（非棘轮），`EXIT=1`；修法由脚本自身给出兜底形态，
仓内 20+ 处先例（`agent_profile.rs:28`、`agent_analytics.rs:17`、`cognitive.rs:2626`）
⇒ 属「纯 bug 可顺手修」，非语义决策项。

**改动**：
- 新增 `use crate::commands::error::{CommandError, ErrorCategory};`
  （`error` 在 `SHARED_INFRA_MODULES` 豁免表内 ⇒ 不计 `commands-no-sibling-call`，复跑已证）
- 10 处 `.map_err(|e| e.to_string())` → `.map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))`
  （兜底码 `COMMON_INTERNAL`）

⚠ **首轮编译失败，暴露一处形态差异（本轮最有价值的教训）**：
`market_mainline_delete_by_date` 是**直接返回**该表达式（**无 `?`**），签名 `Result<u64, String>`；
统一替换后类型不符：

```
error[E0308]: mismatched types
   --> src\commands\market_mainline.rs:118:5
    | expected `Result<u64, String>`, found `Result<u64, ErrorResponse>`
```

⇒ 改为 `Ok(delete_mainlines_by_date(...).await.map_err(...)?)`。

**教训**：同一个 `map_err` 字面量在**「带 `?` 的语句」**与**「直接返回的表达式」**两种位置上，
替换后类型不同 —— 批量 `replace_all` 前必须先分清这两类位置。
门禁脚本只认字面量、**看不出类型**；`cargo check` 才是这条改动的唯一有效判据。

**验证**：
- `check-rust-raw-map-err.mjs` → `OK: 未发现裸 map_err(|x| x.to_string())`，EXIT=0
- `cargo check -p axagent --all-targets` → 日志含 `Compiling axagent` → `Finished in 7m53s`，**EXIT=0**
- `check-layer-discipline.mjs` 复跑 → 仍为原班底 3 处（**无新增违规**）
- 格式化：`cargo fmt -p axagent -- --check` 的差异**仅在本文件**（他人在飞文件均合规）
  ⇒ 用单文件 `rustfmt --edition 2024` 而非 `cargo fmt -p`，避免触碰并发会话的在飞文件
- clippy 定点：见 13.4

### 13.3 未改项（**登记 + 单列告知，需决策**）

| 项 | 门禁 | 处置 |
|---|---|---|
| `domain_ontology.rs` 引用 `bottleneck-calc.rhai` 越界 9 处 | `check-single-source-facts --ci` | ⏸ 未改，见 13.3.1 |
| `check-layer-discipline` 超基线 3 处 | `check-layer-discipline` | ⏸ 未改，见 13.3.2 |

#### 13.3.1 为什么 9 处越界**不能**机械改行号

`bottleneck-calc.rhai` 已从「自身实现三力计算」重构为 **180 行薄壳**：脚本 `:121` 调
`bottleneck_node_score(...)`，真实计算在 **Rust** `stock_workflow/rhai_bottleneck.rs:250`。

mtime 佐证：`bottleneck-calc.rhai` **1120 分钟前** / `rhai_bottleneck.rs` **1056 分钟前**（**同批**），
而 `domain_ontology.rs` **6194 分钟前**（**早 4 天**）⇒ 引用写于旧脚本时代，**重构后消费端未同步**。

⚠ **关键：该门禁只抓「越界」，抓不到「行号在范围内但语义已漂移」**。实测另 3 处**未报但已错**：

| 位置 | 该行现在的内容 | 语义 |
|---|---|---|
| `Industry` evidence → `:159` | `let reliability_summary = …` | ✗ 行业分类 ≠ 可靠度汇总 |
| `ChainNode` evidence → `:143` | `"has_financial_data": has_financial,` | ✗ 产业链环节 ≠ 输出字段 |
| 模块头注释 → `:175-177` | 「第三部分：执行入口」+ `main()` | ✗ 「顶层权重 fallback」已不在此处 |

⇒ **只修 9 处越界 = 门禁变绿而引用依然全错**（假修复）。真正待定的是 **evidence 该指向哪一侧**：

- **(a)** 薄壳脚本里现在的对应行（取值 `:121-125`、分档 `:148`）；或
- **(b)** **Rust 权威实现** `rhai_bottleneck.rs::bottleneck_node_score`（脚本注释自称「收敛为单一权威口径」）

倾向 **(b)**（真源已迁移），但 `domain_ontology.rs:11-13` 的模块约定写明 evidence
「**无一例外**指向 `agency_experts/stock-analysis/*.md` 或 `commands/bottleneck-calc.rhai`」
⇒ 引入新载体须同步改约定文档，且 `check-decl-evidence.mjs` / `check-ontology-consistency.mjs`
可能对载体有约束 ⇒ **属语义决策，不擅自改**。

#### 13.3.2 为什么 3 处棘轮超基线不代改

- **`backtest_validation.rs`**：`commands-no-direct-db` 10>9、`commands-no-sibling-call` 5>1
  （mtime 514 分钟前）—— 由**新增功能代码**引入的分层违规。处置只有「把逻辑下沉到 crate」或「调基线」，
  两者都取决于该功能的设计意图 ⇒ 归属方决策。
- **`crates/crdt`**：`crate-has-consumer` 1>0（`Cargo.toml` mtime **26869 分钟前 ≈ 18.7 天**）
  —— workspace 成员但**零 Cargo 消费者**。要么接线、要么摘除成员，同样需决策。

### 13.4 clippy 定点（`-p axagent`，与 CI 同旗标）

```
cargo clippy -p axagent --all-targets --all-features -- -D warnings
→ EXIT=0
→ 日志含 `Compiling axagent v2.9.9 (D:\OneManager\AxInvest\src-tauri)`  ← 改动所在的主 crate
→ 另有 35 行 `Checking <子crate>`（harness / astock-data / rt-workflow / analysis-engine / …）
→ `^warning|^error` 行数 = 0
→ Finished in 10m 14s
```

> ⚠ **判据细化（本轮实测）**：主 crate 出现在 **`Compiling`** 行而**不是** `Checking` 行
> （它需要先跑 `build.rs` 生成 `register_commands.rs`）。
> 故 #635 的「数 `Checking` 行」须放宽为「数**覆盖改动 crate 的** `Compiling`/`Checking` 行」——
> 若只按 `Checking` 数，会得出「主 crate 未被检查」的**假红**结论（本轮差点如此）。
> 两侧名单都核对过：`-p axagent` 的闭包 = 主 crate + 35 子 crate，与改动范围（仅主 crate）相符。

---

