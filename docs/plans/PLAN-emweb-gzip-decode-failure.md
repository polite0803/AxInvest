# PLAN：emweb 财报接口 gzip 响应炸穿解码——reqwest 未开自动解压（L 轮，002371 实证）

日期：2026-09-28 · 状态：已批准并实施——L1 提交 `cfceda84e`（`cargo check -p axagent-astock-data` 绿；lib.rs 与另一会话的 `mod valuation` 改动同文件，已按 hunk 只暂存本轮两处 `.gzip(true)`）；剩 L2 = 重启重跑 002371 核对 · 触发：用户贴出日志——002371（北方华创）财务数据
5 源全灭（eastmoney/akshare 同 URL `error decoding response body`、browser_eastmoney
「no financials data」、xueqiu 缺 list、baidu 要 Acs-Token），DAG 被
`数据质量不足` 整体跳过。

## 一、取证：不是端点死了，是**同一端点的响应有两种编码，客户端只认一种**

本机 curl 对照（2026-09-28 15:47）：

| 请求 | HTTP | Content-Encoding | 首字节 | 判定 |
|---|---|---|---|---|
| `ZYZBAjaxNew?code=SZ002371` | 200 | **gzip** | `1f 8b` | 客户端解不开 ⇒「decoding error」 |
| `ZYZBAjaxNew?code=SH600519` | 200 | （无） | `{\r\n "pages"` | 正常 |

机理：Tengine + `Cache-Control: public, max-age=120` + `Vary: *`——CDN 边缘节点
把浏览器流量（带 `Accept-Encoding: gzip`）拿到的 **gzip 变体缓存**后，
按 URL 键原样吐给不带该头的我们；哪只票命中 gzip 缓存取决于最近一次回源者是谁
⇒「有的票能取、有的票整链全灭、过两分钟可能自己好」的随机形态。
`akshare` vendor 请求的也是同一 emweb URL ⇒ 同死；重试 3 次全撞同一缓存 ⇒ 徒劳。

**客户端侧证据**：workspace `reqwest = { features = ["json"] }`（根 Cargo.toml:69），
astock-data 三处 `Client::builder()`（`vendors/eastmoney.rs:46`、`lib.rs:633`、`lib.rs:720`）
均未开 `.gzip(true)`——reqwest 的解压是 feature+builder 双开关，缺哪个都不解。

## 二、修法（L1，最小两处）

1. 根 `Cargo.toml`：reqwest features `["json"]` → `["json", "gzip"]`；
2. astock-data 三处 builder 补 `.gzip(true)`（reqwest 会自动带
   `Accept-Encoding: gzip` 并解压，两种缓存变体从此同吃）；
3. 验证：本机复跑 002371 分析（K2 同车）——financials 应一次命中 eastmoney 主源；
   门禁 `cargo check -p axagent-astock-data` + 全量编译（feature 加在 workspace，
   影响面 = 所有用 reqwest 的 crate，纯增量能力，不改变既有请求行为）。

## 三、不做

- browser_eastmoney「no financials data」与 xueqiu 缺 list 属各自解析/凭证问题，
  本次主源修复后它们回到「兜底不该被触发」的位置，不扩范围；
- 不给重试链加更长退避——缓存变体问题不是瞬态故障，解压修好后重试语义恢复原设计。