# PLAN：概念板块涨跌幅恒为 null——F10 主题报告只给成分、榜单拼接缺失（K 轮，300604 实证）

日期：2026-09-28 · 状态：已批准并实施——K1 提交 `030efd036`（board_ranking 6/6 绿；crate 全量 clippy 当前被另一会话未提交的 valuation.rs 拦红，与本改动无关）；剩 K2 = 重启重跑 300604 核对 · 触发：用户贴出行业分析师诊断行
「概念涨幅数据缺失：获取的概念板块未附带当日涨跌幅」（运行 `414a926c`，长川科技 300604）

## 一、取证：分析师的标记是对的，病灶是一行硬编码

1. `a-sector` 的按需工具调用记录：`get_stock_concept_blocks(300604)` 返回
  ```json
   {"industry":"存储芯片","concepts":[{"name":"电子","changePct":null},
    {"name":"半导体设备","changePct":null},{"name":"浙江板块","changePct":null}…]}
   ```
   ——**全部 null**，分析师如实写「概念涨幅数据缺失」⇒ data-quality 计 1 处失败标记，
   每一步都没撒谎。
2. 根因：`vendors/eastmoney.rs:3435` `BlockItem { name, change_pct: None }` ——
   概念名单来自 F10 主题报表 `RPT_F10_CORETHEME_BOARDTYPE`（**只提供成分关系，
   本来就没有涨跌幅列**），实现从未拼接行情源，字段自诞生起恒 null。
3. 可行补数通道实测（本机存活 host，push2\* 全族被 RST 的背景下）：
   `data.eastmoney.com/dataapi/bkzj/getbkzj?key=f3,f12,f14&code=m:90+t:N`
   - `t:3` 概念板块 504 条（含 f3 当日涨跌幅，百分数×100，如 140=1.40%）；
   - `t:2` 行业板块 496 条；`t:1` 地域板块 31 条（"浙江板块"这类名字在 t:1）。
   名单与成分按 **板块名 join** 即可补全（三表并一张 name→pct 映射）。

## 二、修法（K1，单点）

- `get_concept_blocks`（live）：拉 t:1/t:2/t:3 三张榜单合成 `name→changePct` 映射，
  填充 concepts 的 `change_pct`；join 不上的名字**保持 null**（不编 0%）；
  榜单请求失败 ⇒ 整函数退化为现行为（名单照常、pct 全 null），不因补数拖垮主结果；
- **as-of 语义不变**：该方法本就登记 `NoHistoricalSemantic`（快照是唯一历史通道），
  补全后的 pct 随快照一起落盘；历史日期的概念涨跌幅无可达源（BK 日 K 在 push2\*），
  不假装能回放；
- 解析逻辑抽成纯函数（`build_block_pct_map(&Value) -> HashMap<String,f64>` +
  `fill_block_change_pct`），零网络单测：命中填充 / 不命中保持 null / 榜单缺失降级；
- browser_eastmoney 兜底通道共用同一对纯函数（先查其现状，同构则同修）。

## 三、验证

- K1 单测绿 + `cargo test -p axagent-astock-data` 全量 + 定点 clippy；
- K2 运行期：重启重跑 300604 → 行业分析师报告不再出现「概念涨幅数据缺失」、
  该维度失败标记 1→0（数据质量表对应行恢复）。

## 四、不做

- 历史日期的概念涨跌幅回放（无源，物理不可得，快照起累积）；
- `regions` 数组的拆分重建（现实现把地域名单混进 concepts，join 后至少数值正确；
  名单归类重构与本缺陷无关，不扩范围）。