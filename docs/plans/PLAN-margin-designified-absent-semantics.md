# PLAN：两融「设计上没有」被压成裸 null——F 轮语义分层（粤海饲料 001313 实证）

> 状态：active ｜ 日期：2026-09-27 ｜ 对应版本：v2.9.9


日期：2026-09-27 · 状态：已批准并实施——F1+F2+F3 提交 `00c0cf6cc`（rt-workflow 门禁 21/21、astock-data 43/43、两 crate clippy/fmt、主 crate check 全绿；模板升 v88）；剩 F4 = 重启重跑 001313 核对 · 前置：`PLAN-dcf-fallback-anchor-applicability.md`（E1 已提交 `4e5e011eb`）

## 一、用户批准的「方案 1（接两融通道）」被取证推翻

对 18:22 运行 `c6399466`（粤海饲料 001313，74/B，工具可信度 58.2）的逐层核验：

1. **通道存在且全链路健康**：MCP 工具 `get_stock_margin_data`（`mcp_tools.rs:202`）、
   报表 `RPTA_WEB_RZRQ_GGMX`（`eastmoney.rs:2201-2246`）、路由 `margin:
   eastmoney→browser_eastmoney→baidu_stock`（`lib.rs:276`）、as-of 登记（`eastmoney.rs:3574`）、
   快照（`daily_snapshot.rs:41`）俱在；两个分析师 prompt 都明言消费融资余额。
2. **分析师真调了工具**：`a-sentiment`/`a-hot-money` 的 `tool_calls_made` 均有
   `get_stock_margin_data(001313) → content:"null"，is_error:false`。
3. **对照实验（本机 curl）**：同报表同 filter 键——600519 返回完整 RZYE/RQYE 明细
   （3989 页），001313 返回 `9201 返回数据为空` ⇒ **粤海饲料非融资融券标的**，
   空是正确答案（`datacenter_reports_no_data` 的「空是答案」族，文档注释点名过 RZRQ）。

⇒ 无通道可接。58.2 里两融相关扣分的真病灶在**语义丢失**，不在数据缺失。

## 二、真病灶：live 路径把「设计上没有」压成与故障同形的裸 `null`

- as-of 路径已有三档语义分层（`lib.rs:3498-3522`：StructuralGap / **NoData=「非两融标的，
  或该日无披露」** / Failure）——但只在回放分支；
- **live 路径**（`lib.rs:3544` 起）：vendor 空数据 → 注释「空数据是业务层面的无数据（如该股票
  无融资融券）」→ 静默 `return Ok(None)` → handler 序列化 `"null"` →
  分析师只见裸 null、不知原因 → 如实写「**融资余额数据缺失**」→ data-quality 按
  工具故障计标（正确执行词表——它收到的措辞就是失败动词）。
- 对比：北向停披有 `timestamp` 内嵌说明句（本轮 hm 报告写了「停披」语境）⇒ 两融缺同款送达。

## 三、修复阶段（F 轮，逐阶段批准）

### F1 tool 契约：`get_stock_margin_data` 输出结构化「设计上没有」（astock-data）

- live `Ok(None)` 时 handler 输出改为
  `{"available":false,"reason":"该证券非融资融券标的（或该日无披露），设计上无此数据"}`；
  vendor 真故障走 `is_error` 既有通道不变；类型签名不动（分层在 handler 层做）；
- 测试：handler 形态单测（None⇒available:false+reason；Some⇒原样），零网络。

### F2 分析师 prompt：设计上没有 ⇒ 按声明写（agency_experts 两个 md）

- `sentiment-analyst.md` / `hot-money-tracker.md`：读到 `available:false` 时写
  「该券非两融标的，无融资余额数据（设计性缺席）」，禁用「数据缺失/获取失败」措辞。

### F3 data-quality：句级抑制表补「非两融标的」语境（模板 v88）

- `数据缺失`/`获取失败` 的短语组追加「非两融标的」——与 F2 措辞闭环；真故障句不受影响；
- 门禁 `data_quality_placeholder_gate.rs` 加本轮真实句样本 + seed 注释 v88。

### F4 运行期核对（用户动作）

重启重跑 001313：预期两融相关 2 标记消失、工具可信度 58.2 → ~72、综合 ≈ 79（B+）。

## 四、不做

- 为 001313 硬造两融数据（非标的即无披露义务，任何源都给不出）；
- 期权 PCR 豁免（sent 报告已自带「A股个股通常无期权」括注，词表现有「无覆盖」族句级可判，
  本轮未误伤它，不动）。