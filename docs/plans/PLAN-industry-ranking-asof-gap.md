# PLAN：行业排名 as-of 合成全灭——push2\* 族本机被阻断 + 快照归属日按「今天」判致周末永不回填（G 轮）

> 状态：active ｜ 日期：2026-09-27 ｜ 对应版本：v2.9.9


日期：2026-09-27 · 状态：已批准并实施（修订版，见文末「实施记录」）· 触发：用户重跑后降级面板「真故障（需要修）·1：
`get_industry_ranking … 板块指数日 K 全部取数失败 HTTP request failed`（eastmoney 与
browser_eastmoney 同报）。

## 一、根因（实测端点普查，2026-09-27 19:40）

合成链（`vendors/eastmoney.rs:986 synthesize_industry_ranking`）：
名单 = `data.eastmoney.com/dataapi/bkzj/getbkzj`（**存活**，返回 31 板块）；
日 K = `push2his.eastmoney.com/api/qt/stock/kline/get`（`board_kline_url` :922）——
本机对 **push2 / push2delay / push2his 全族 connection reset**（curl exit=56，0.1s 秒断，
Defender 级阻断，与 money_flow / 301302 冷链事故同源）。两 vendor 只是换传输不换 host，
兜底同死。

替代历史源普查结论——**当日可达的公开源里没有 BK 板块历史日 K**：

| 候选 | 结果 |
|---|---|
| 腾讯 `kline/kline`（90.BK0475 / bk0475 / sBK0475 / sh880475，含 qfq 变体） | `param error` 或 `day:[]` |
| 新浪 `hq.sinajs.cn/list=bk0475` | 空串（无此品种） |
| 百度 `finance.pae` 板块历史 | ResultCode 3203 |
| `data.eastmoney.com` bkzj 带 `date` 参数 | 参数被忽略，恒返回当日快照 |
| datacenter 报表（猜 RPTIndustryQuote / RPTA_WEB_BK_HIS） | 9501 报表不存在 |
| 雪球 BK kline | 无 token（既有取证） |
| 东财 web 板块页 | 其前端同样调 push2his ⇒ 同死 |

旁注：LIVE 路径不受影响（bkzj 当日名单自带 `f3` 当日涨跌幅 ⇒ live 行业排名正常）；
受灾面 = **as-of 回放**里截止日早于今天、且当日无快照可查的行业排名。
面板里另两条「取数窗口到顶」（get_quote 休市回退、get_research_reports 如实无数据）
语义正确，不动。

## 二、修法（两件套，均为既有机制对齐，不发明新轮子）

### G1 降级归因改判：「探测的源取数失败」→「该维度只有当日通道，历史不可达」

- 现状把「源被本机阻断 + 本方法无任何历史通道」标成**真故障（需要修）**，与
  `get_social_sentiment`（面板分类「回放不适用（无需修）：该维度只有当日快照一条通道」）
  语义相同却分档不同 ⇒ 归因说谎（对用户是误导，对开发是伪工单）。
- 修法：`get_industry_ranking` 的 as-of 探测全失败且失败形态为传输层阻断
  （RST，非 5xx/解析）⇒ `record_degradation` 用 `StructuralGap` 档 + 文案
  「行业排名历史依赖 push2\* 族（本机不可达），自 G2 快照累积之日起可回放」。

### G2 快照累积：把 `get_industry_ranking` 纳入每日快照通道

- 对齐 `daily_snapshot.rs` 既有先例（get_social_sentiment / get_hot_stocks 同为
  「当下语义、单通道」类 ⇒ 当日拉到的 live 排名按日期落快照，回放读快照）；
- 效果：G2 上线之日起的日期可完整回放；**历史日期不回补**（无源可回补，这是物理事实），
  由 G1 的文案如实声明；
- 风险注：快照容量 10_000 条、行业排名每日 1 条全市场值（非 per-stock），量级可忽略。

### G3 运行期验证

重跑一次 live（落快照）→ 以当日为截止日重跑回放 → 面板该条应显示「命中快照」；
以更早日期回放 → 应显示 StructuralGap 文案而非红色真故障。

## 三、不做

- 绕过本机网络阻断（IP 直连/改 SNI 等对抗 Defender 的手段）——环境既定事实，不对抗；
- 用成分股 K 线合成板块涨跌幅——成分名单同样只在 push2 族，绕不开阻断；
- 继续猜 datacenter 报表名——两次已证不存在，收益/成本比差。

## 四、实施记录（2026-09-27 晚，批准后取证修订）

批准后又跑了两项取证，**方案实质修订**：

1. **G1 归因改判取消**。`AsofProbe::kind()` 的 T14 分档语义（源报错 ⇒ Failure）本身诚实——
   push2\* 被阻断确实是「该修的故障」，把它降成 StructuralGap 反而掩盖问题。不动。
2. **G2 机制已存在但归属日有 bug（真缺陷）**：`get_industry_ranking` 早在
   `SNAPSHOT_METHODS` 清单内，回放读取 + 合成回写俱在；缺的是**采集侧永远采不到 09-24**——
   `start_daily_snapshot_sweep` 的闸门是 `is_trading_day(今天)` + 归属日 `Local::now()`：
   09-24（周四）收盘后紧接着 09-25 中秋 + 09-26/27 周末，会话全部落在非交易日 ⇒ 整轮 skip。
   实证：`~/.axagent/astock_daily_snapshot.json` 全库仅一条 `daily:get_industry_ranking:2026-09-09`，
   文件 mtime 10:04（今天零写入）。
3. **修法（已实施）**：新增纯函数 `daily_snapshot::snapshot_target_date(now)`——归属日 =
   最近一个**已完成**交易日（交易日盘中 15:00 前返回 None 等收盘；非交易日返回上一交易日 ⇒
   周末会话自动回填）；`run_daily_snapshot_sweep` 改收 `date` 参数，两个调用方
   （后台 tick / 手动命令）统一走该判据；手动命令盘中回落到 `previous_trading_day(昨天)`
   （⚠ 不能回落 `previous_trading_day(今天)`——交易日当天即返回当天，会把盘中半日数据
   当收盘值落盘）。单测 `snapshot_target_date_backfills_last_trading_day` 用
   09-24/25/27 真实日历场景锁死。

### 同日旁证：live(18:22) 与 as-of(19:32) 同日结论「观望 vs 减持」的归因

因子级 diff（两运行 portfolio-mgr evidence.factors）：f1/f4/f5/f10/f12 完全一致；
差异 = ① **LLM 采样方差**：trader 节点 观望/中性 conf60 → 减持/看空 conf68（f7 0→−0.328）、
共识分析师 +0.26→−0.30（f2，研报输入两边皆空 ⇒ 纯 LLM 方差）；② **回放输入缩水**：
行业排名（本 G 案）、舆情快照、期权 PCR 在回放侧为空。生效后验 0.42 vs 0.37 恰好跨在
观望/减持分档边界（≈0.38）两侧——量化决策层对同一输入是确定性的，变的是输入面。
## 五、G2''（2026-09-27 深夜，001313 回放面板复查发现的两个残余缺陷）

G2' 提交后重跑，面板该行仍是「真故障 · 1」。取证不是「猜没修好」而是直接读快照文件
`~/.axagent/astock_daily_snapshot.json`（mtime 20:20 = G2' 之后的一轮采集），结果：

| key | 值 |
|---|---|
| `daily:get_industry_ranking:2026-09-24` | **`[]`** |
| `daily:get_hot_stocks:2026-09-24` | **`[]`** |
| `daily:get_cls_flash:2026-09-24` | 有内容 |
| `daily:get_index_quotes:2026-09-24` | 有内容 |
| `daily:get_industry_ranking:2026-09-09` | 有内容（31 板块） |

⇒ 采集侧确实在跑（G2' 生效），但落盘的是**空数组**；而 live 通道本身是好的
（实测 `data.eastmoney.com/dataapi/bkzj/getbkzj` HTTP 200 / 31 行）。两条独立缺陷：

### 缺陷① 读取侧仍按锚点原值拼 key（G2' 只补了写入侧）

`symbols`：`snapshot_target_date` 把归属日改成最近已完成交易日，但
`try_daily_snapshot(method, &ctx.as_of_date)` 用的是**原值**。用户锚点选周日 09-27 ⇒
查 `daily:{method}:2026-09-27`，永远 miss —— 周末回填的快照一条也取不到，
面板继续去打被本机 RST 的 push2\* 合成链。
修法：`daily_snapshot::key_date()`，在三个 key 构造器（全市场 / 个股级 / keyword 级）
统一归一到 `previous_trading_day`。放在 key 构造处而非各调用方 ⇒ 读写同一判据、
不可能再各走各的。口径与 `get_quote` 的休市回退一致（09-27 → 09-24）。

### 缺陷② 空结果被当成有效快照落盘，且判重闸门把它锁死

`run_daily_snapshot_sweep` 的全市场臂写作 `Ok(r) => to_string(&r)` —— 空列表序列化成
`"[]"`（2 字符，非空串）就通过了末尾的 `if !json.is_empty()`；后台 tick 的闸门又是
`has_daily_snapshot("get_index_quotes", date)` ⇒ `"[]"` 也算「已有」⇒ **之后每轮整轮
skip，该日永远修不好**。
空从哪来：采集任务无 task-local 作用域，`current_as_of()` 回落读**进程级全局栈**
（`with_optional_asof` 会为并发的回放压栈）⇒ 回放分析在跑时触发的采集走的是 as-of 分支：
行业排名探测 push2\* 失败 → `Ok(vec![])`；热股榜结构性无通道 → `Ok(vec![])`。
这解释了同一轮里 cls_flash / index_quotes（有日期通道）有值、另两条空。
修法两件套：① `daily_snapshot::usable()` 在 get/get_stock/get_keyword/has/has_keyword
的唯一出口过滤 `"[]"` / `""` / `"null"` ⇒ 旧文件里的中毒条目自愈、下一轮重采；
② `run_daily_snapshot_sweep` 进函数即 `enter_global_asof(None)` 压一层「显式 live」
（RAII，按 token 只移除自己那层，不动别人的作用域；不能用 `set_global_asof(None)`——
它清空整个栈，会把正在跑的回放踢出 as-of 模式）。

### 验证口径（需重启应用）

1. 重启后等一次 tick（或手动触发采集）⇒ `daily:get_industry_ranking:<上一交易日>` 应有 31 条；
2. 锚点选非交易日（周末/中秋）回放 ⇒ 该行不再出现（命中快照，零降级）；
3. 若该行仍红 ⇒ 说明 live 的 getbkzj 也在失败，是另一条事实，按红档处理（正确归因）。
