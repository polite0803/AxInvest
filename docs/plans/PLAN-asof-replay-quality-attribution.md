# PLAN：回放运行数据质量误判修复（截图 52.1/C 级归因）

> 状态：active ｜ 日期：2026-09-26 ｜ 对应版本：v2.9.9


日期：2026-09-26 · 状态：S1–S8 + R8/R9a/R9b 全部完成（10 提交），全量门禁终跑中

提交链：`04b24a17d`(S1) → `e585cb489`(S2) → `0123b51ba`(S3) → `f8e9fc568`(S2 lint) → `c18cba4e1`(S3 判据)
→ `0183b3b44`(S5) → `df0fbc77a`(S6，模板 v83) → `957d26838`(S7+S8，S8 即 R8) → `648ada196`(R9a，模板 v84)
→ `566e4bf55`(R9b，模板 v85)。
S5–S8 验证：astock-data lib 381/381、门禁 16/16、三 crate clippy -D warnings、fmt 全绿；
S7 数据源经 curl 实测（300642@2026-09-22 返回 TOTAL_SHARES=162272241、PE_TTM、SECURITY_NAME_ABBR=透景生命）。
运行期核对前提：**需重启应用**（模板 v85 在启动时重新 seed 才生效），再重跑 300642 as_of=09-22。

## 一、取证结论（先纠正前提）

**截图那次运行不是 live 模式，而是 as-of 回放。** 证据链：

1. 面板全文「综合 52.1 = 报告 55.1×0.35 + 工具 27.0×0.35 + 因子 77.8×0.30；15 种标记、共 17 次；平均置信度 48.0；字段校验 2 个」在 `session_logs/agnes-3.0-flash.jsonl` 中**逐字命中**唯一运行：`stock=300642, analysis_id=a30b15d1, as_of_date=2026-09-22`。
2. 按运行分段聚合工具结果（同日志）：
   - **live 运行（as_of=None）工具层健康**：300179 ok=56/empty=1；688114 ok=28/empty=2；300642 三次 live 均 ok≥26、empty≤3（空的只有 margin/pledge/option_pcr，属个股真实无数据）。新闻/搜索/快讯/资金流全部有数据。
   - **回放运行才是空结果重灾区**：a30b15d1(as_of 09-22) ok=12/empty=25；511abde4(as_of 09-22) ok=59/empty=56；bff3eed7(as_of 09-24) ok=44/empty=48。
3. 本机 curl 实测 4 个核心东财接口（searchadapter / np-listapi 7x24 / reportapi / push2his fflow）**全部存活且有真实数据**——排除「行情源今天挂了」。

回放里 news/search/cls_flash 等空是**设计性降级**（`as_of.rs` 决策矩阵：搜索是当下语义，as-of 下 `return Ok(vec![])` + `record_degradation`）。缺陷不在数据源，在**降级事实没有被系统看见和被质量节点正确归因**。

## 二、缺陷清单（按根因）

### R1 降级报告跨 spawn 丢失 →「0 个降级」假象（主缺陷）

- `core.rs:902` 把整个 DAG `tokio::spawn` 出去，包在 `with_optional_asof(...with_degradation_log(...))` 里；但工作流引擎内部**每个分析师节点又各自 spawn**（`as_of.rs` 头注与 2026-09-23 修复记录都证实这一点）。
- `record_degradation`（`as_of.rs:311`）对 task-local 的写入用 `let _ = DEGRADATION_LOG.try_with(...)`——子任务不在 scope 内，**静默失败**；只有全局环形缓冲收到。
- 消费端 `core.rs:1079/1414` 用的是 `take_asof_degradation_report()`（**只读 task-local**）⇒ 父任务永远拿到空表 ⇒ blackboard `degraded.count=0`、面板「0 个降级」，而实际该次回放有 6+ 个维度设计性降级。
- 这与 2026-09-19/09-23 修过的 AS_OF 跨 spawn 问题是**同一族**，当时只给 `AS_OF` 做了全局回退，`DEGRADATION_LOG` 的消费侧没有跟上。

### R2 data-quality 节点不区分「as-of 设计性降级」与「工具真失败」

- `data-quality.rhai` 的 `tool_credibility()`（:488-497）与失败标记词表（:205-211）对 live/回放一视同仁：回放中分析师如实写「数据缺失」→ 计 15 种/17 次标记 → 工具可信度 27 → 综合 C 级。
- 后果：回放的 C 级评分**不可与 live 评分横向比较**，且会污染反思/进化链路（trajectory 侧按质量分做信号）。
- 修法依赖 R1（要先能拿到该次运行的降级清单，才能把「已记录降级的维度」从失败标记/可信度惩罚中剔除或单独标注）。

### R3 回放缺「估值上行空间、主力净流入」两因子

- `data-quality.rhai:891/:895` 判定缺失；上游是 DCF 依赖的财务现金流与 `get_money_flow`。回放模式下 push2his fflow `lmt=5` 取到的是 as_of **之后**的 5 天，截断后为 None ⇒ 因子恒缺。属 `project-asof-gaps` 记录的 B 桶同类（时间泄露桶已修，「as_of 前取数」未做）。
- 修法：fflow 请求带 `end=as_of_date`（push2his 支持 beg/end 参数，需 curl 验证），as-of 模式下取截止日前 5 日。

### R4 情绪面 bull+bear=115 不自洽（已扣分，仅记录）

- 字段校验已捕获并扣 6 分，属 LLM 输出问题，暂不动提示词；若 R1–R3 修后仍高频复发再单独立项。

## 三、修复阶段

| 阶段 | 内容 | 触点 |
|---|---|---|
| S1 | `DEGRADATION_LOG` 消费侧改为「运行边界内的全局缓冲切片」：进入运行时记录全局 total 水位，结束时取水位之后的条目（含 task-local 已有条目合并去重）；`take_asof_degradation_report` 保留但 core.rs 两处消费点换为新函数 | `as_of.rs`、`stock_workflow/core.rs:1079/1414` |
| S2 | data-quality 节点消费降级清单：被 `record_degradation` 覆盖的维度（method↔分析师映射）其失败标记改计为「as-of 设计性降级」，不再进 `placeholder_*` 惩罚与 `tool_credibility` 扣分；summary 追加「回放模式：N 个维度按设计降级」 | `data-quality.rhai`、`seed_stock_analysis.rs` 注入变量 |
| S3 | `get_money_flow` as-of 分支取截止日前数据；补 asof 边界测试（新 `fflow_asof_tests` 模块） | `astock-data/src/vendors/eastmoney.rs`、`lib.rs` as-of 路由。**执行时修正方案**：实测 push2his fflow/daykline **忽略 beg/end**（传 end=20260918 与不传结果相同，恒返最近 ~120 交易日）⇒ 改为「lmt=0 拉全窗 + 本地按截止日过滤」（形态对齐 north_bound as-of 先例），eastmoney 对 get_money_flow 改申报 NativeDateParam；同批修复 live 路径排序 bug（接口升序返回而代码取 history[0] 当最新 ⇒ 顶层字段一直拿的是窗口内最老一天） |
| S4 | 回归验证：用 `axagent-batch-rerun` 重跑 300642 as_of=2026-09-22 一次，核对 degraded.count>0、失败标记数下降、综合分口径变化符合预期；live 跑一次 601899 确认无回归 | 手工 |

门禁：各阶段定点 `cargo clippy -p <crate> --all-targets -- -D warnings`；全部完成后按 CI 口径跑一次全量三条（`cargo fmt --all --check` / `clippy --workspace --all-targets --all-features -- -D warnings` / `__TAURI_WORKSPACE__=true cargo test --workspace`）+ `npm run test:run` 相关用例。rhai 侧有 `rt-workflow/tests/data_quality_placeholder_gate.rs` 门禁需同步更新判据。

## 四、明确不做

- 不改任何东财接口的 live 取数路径（live 已被证实健康）。
- 不动 news_archive 语料积累（B 桶数据积累是时间问题，非代码问题）。
- 不为此评分新建前端第二套算法（前端继续只消费 rhai 输出）。

## 五、二轮（S5–S7）：修复后重跑 300642 as_of=09-22（`1ad42f59`）暴露的新缺陷

运行期验证结论：S1–S3 均生效（【as-of 回放】前缀出现、失败标记清零、主力净流入恢复 ⇒ 因子 77.8→88.9、综合 52.1→60.0）。同时暴露三处新问题：

### R5 豁免过宽 — 全局缓冲跨运行污染（S2 的已知边界成真）

warnings 显示**全部 10 维**「按设计降级」，含技术面/基本面这种本轮实际有数的维度。根因：
`pm_asof_degraded_methods` 只按 as_of 日期过滤全局环形缓冲，而缓冲里残留**同截止日更早的修复前运行**
（511abde4/a30b15d1）记的降级条目（如 get_money_flow、get_financials）。日期过滤挡不住同日期旧运行。
后果不止文案误导——**真工具故障在回放里也会被无差别豁免**。
修法：S1 的水位线复用——`as_of.rs` 增设进程级「当前运行降级基线」（`AtomicU64`，工作流入口在
`deg_watermark` 同一处写入），`pm_asof_degraded_methods` 改按 `seq > 基线` 过滤而非日期。
前提与 S1 相同：工作流有全局 permit 串行，可接受。

### R6 豁免维度的差距原因自相矛盾

面板四行低置信的差距原因写「**报告无失败标记**、字段齐全——非数据缺口」，但豁免生效后
`ph_n` 恒 0，该句对「有标记但被豁免」的维度是**假话**。修法：`diag_for` 增第 9 参
`asof_exempted`，为真时输出「as-of 回放：本维度上游按设计降级（当下语义约束），失败标记已豁免、不计扣分」。

### R7 回放「估值上行空间」恒缺 — DCF 拿不到总股本

`compute_valuation` 返回 `dcf.note="总股本不可用，DCF不可用"` ⇒ `upsidePct=null` ⇒ f5 恒缺。
根因：回放里 quote 走 `SynthesizeFromKline`（按截止日 K 线合成），合成结果只有
price/pre_close/change_pct/volume/amount，**没有总股本/总市值字段**；DCF 需要股本。
修法（倾向 a，b 作备选）：
  a. `compute_valuation` 在 as-of 下股本回退顺序：合成 quote → `get_stock_capital_structure`/基本面快照里
     的 `total_shares`（股本变化慢，用最近披露值并在 assumptions 里标注股本口径日期，避免假装是截止日值）；
  b. 若 a 的源在回放里也拿不到 ⇒ 保持缺失，但把「总股本不可用」并入 `upstream_data_gaps` 显式告警（只告警不扣分，同既有口径）。
实施前先验证：300642 的股本在 as-of 路径哪个源可得（curl datacenter `RPT_F10_BASIC` 或既有 get_financials 字段）。

| 阶段 | 触点 |
|---|---|
| S5 | `as_of.rs`（基线 setter/getter）、`core.rs` 入口写基线、`rhai_pm.rs` 改按基线过滤、门禁测试补「旧运行条目不得豁免本轮」用例 |
| S6 | `data-quality.rhai`（diag_for 第 9 参 + 文案）、门禁测试补文案断言、模板 v83 |
| S7 | `mcp_tools.rs` compute_valuation 股本回退 + 定点测试；或退化为 upstream_data_gaps 告警 |

验证：全量门禁三条 + 应用里再跑一次 300642 as_of=09-22（预期：豁免清单收敛到真实降级维度、
差距原因不再出现「报告无失败标记」假话、估值上行空间按 S7 方案二选一地解决或显式告警）。

## 六、三轮新增（用户指示：live 与 as-of 分开检查）

### R8 as-of 模式股票名称=代码（已修，`957d26838` = S8）

实证：`quote_from_klines`（lib.rs:1293）把合成 quote 的 `name` 硬编码为股票代码；
工作流/历史记录的「股票名称」取自 quote ⇒ 回放运行全部显示代码（日志中
「股票名称: `300642`」直接印证）。采纳方案 a：S7 的 `get_valuation_snapshot_asof`
顺带返回 `SECURITY_NAME_ABBR`（同一行数据，零额外请求），as-of quote 回填时
name 仍为代码才替换。300642 实测回放名恢复为「透景生命」。

### R9 live 模式评分口径问题（用户裁定「继续R9」后立项，R9a/R9b 已实施）

43 次 live 运行无一 A 级（C/B 之间，工具可信度 19~69），但取数层健康。失分构成：
① 分析师把「该股无龙虎榜/无期权/北向停披」等**合法数据缺席**写成词表命中语 ⇒ 按工具故障扣分；
② avg_conf 天然 55~65 ⇒ 基座分低再叠 gap/校验罚；③ 字段校验每 1 个 −3。

- **R9a（`648ada196`，模板 v84）**：移除 severe_direction_conflict 对 tool_credibility 的 −20 罚。
  实证 30/40 运行命中——分歧本身就是多分析师独立结论的设计特性，扣分=系统性偏置。
  先例：2026-09-24 移除 good_count。字段与 warning 保留，仅不再计分。
- **R9b（`566e4bf55`，模板 v85）**：词表「缺席类」定向抑制（`soft_marker_suppression()`）——
  停披/无机构覆盖/无期权覆盖/暂无数据/配置未注入等措辞 co-occur 时该标记不计失败。
  ⚠ 试过把「数据缺失」降为软标记，3 个历史测试转红：抑制是**整篇级 co-occur**，
  同报告真缺口会被连带吞掉（正是 2026-09-21「整篇抑制」警告的形态）⇒「数据缺失」维持硬标记。
- 未实施（产品口径后续候选）：评分标尺按 live 分布重定标 / LLM 输出 schema 拆分
  「数据可得性」与「结论信心」两字段。
