# PLAN — Semantica 借鉴清单

> 状态：active ｜ 日期：2026-09-15 ｜ 对应版本：v2.9.7


配套：`AUDIT-semantica-vs-axinvest-2026-09-13.md`（总报告）+ **`AUDIT-semantica-ontology-deep-2026-09-13.md`（本体专项，二轮）**
许可：MIT ｜ **全部条目为「用我方栈重新实现设计意图」，非代码移植**

> ⚠️ **本清单的硬规则**：每条都标注「**源项目自己是否真做到了**」。凡源项目为 **占位 / 默认关闭 / 未接线** 的，只能参考设计意图，**绝不能参考实现**。

> 🚫 **执行状态（2026-09-14 01:50 首次追加 / 02:35 更新）**
> - 截至 09-13 11:31 写完起 **零引用、零执行**，根因：**本文件不在每轮注入层**，只有被显式 `Read` 才可见 ⇒ 写完即沉底。
>   处置：3 个 P0（`O1` / `O2` / `L1`）已补登记进 `.workbuddy/memory/MEMORY.md` 待裁决区。
> - **09-14 01:52**：`O2` **首版落地**（语义扫描器）→ 见 O2 节「首次执行」。
> - **09-14 02:20**：`O1` **首版落地** —— 概念注册表 `src-tauri/crates/harness/src/domain_semantics.rs`
>   （**24 概念 / 10 表面名 / 覆盖率 100%**）；`O2` 脚本改为**读该表**（不再自带清单）→ 见 O1 节「首次执行」。
> - **09-14 02:45**：**四项待裁决全部执行**（量纲修复 / 校准哨兵 / 量纲化 newtype / CI 门禁）→ 见 O1 节
>   「第二次执行」与 O2 节「接入 CI」；报告 `docs/audits/AUDIT-domain-semantics-fixes-2026-09-14.md`。
>   **门禁语义定死**：`--strict` **只拦 A 段（未登记量纲）**；PLAN 病点属**待办态**，默认只报告
>   （拦它会逼人删条目 ⇒ 信号被消灭），需要时显式 `--strict-plan`。
> - **09-14 03:05**：**`L1` 落地**（借鉴清单里**最后一条未执行的 P0**）—— `scripts/check-layer-discipline.mjs`
>   5 规则 + 棘轮 + 自检 51 格 + 活体自证；`ci.yml` / `ci-check.mjs` 接线；补 `ci.yml` 测试守卫
>   → 见 L1 节「✅ 执行」；报告 `docs/audits/AUDIT-layer-discipline-l1-2026-09-14.md`。
>   ⚠ **L1 原表的实测数字与同判据实测不符**（规则 2：原 3 处 → 实测 84 处），已在 L1 节逐条登记。
> - **09-14 03:15 起**：**真 O1 落地**（用户裁定「瓶颈三力·含接线」）。用户当场质疑
>   「到目前为止，我还是没有见到任何类似 ontology 的任务落地」—— 判定**成立**：
>   02:20 的「概念注册表」是**字段层横切查找表**（24 行平铺 / 无父子 / 无关系 / 无公理），**不是本体**；
>   03:05 前后本轮产出（L1 门禁、`ci.yml` 守卫、删空转守卫）**全是「门禁的门禁」，本体零产出**。
>   本轮新增 `harness/src/domain_ontology.rs`（11 类 / 4 关系 / 4 度量 / 7 公理 / 4 分档 / 2 分歧）
>   + 权威源唯一化（7 处副本中的 seed 侧 6 个数字清零）+ `check-ontology-consistency.mjs`（selftest 26/26）
>   + CI 接线 + 破坏性活体自证 → 见 O1 节「第三次执行」；
>   报告 `docs/audits/AUDIT-domain-ontology-bottleneck-2026-09-14.md`。
>   **零行为变更**（`format_weight` 保尾零 ⇒ 种子产物逐字节不变 ⇒ 不需升 `TEMPLATE_VERSION`）。
> - 其余 **14 条仍未执行**（`L1` 已于 03:05 落地；P0 三项 `O1`/`O2`/`L1` 均已有落地产物，
>   但 `O1` 尚有「`Unit` 缺 `Count`/价格、newtype 仅接 1/24 概念、分项权重未接线」等尾巴）。
> - **纪律：任一条开工 / 完成 / 否决，必须同步改本段**，否则同等沉底。

---

## 优先级总表

| 优先级 | 条目                                                 | 性质                              | 源项目是否真做到                                                                                 | 是否行为变更                                         |
| ------ | ---------------------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------- |
| **P0** | B1 决策「为什么」设成硬门                            | 🔴 我方自有缺陷 + 🟡 借鉴对方     | ✅ **真做到**（唯一一条）                                                                        | 是（会让现有弱理由决策被判违规）                     |
| **P0** | B2 决策记录双写者收敛                                | 🔴 我方自有缺陷                   | —                                                                                                | 是                                                   |
| **P1** | B3 决策溯源标准导出                                  | 🟡 借鉴对方                       | ⚠️ 部分（PROV-O 真实现，但**默认不挂载**，需显式传 `provenance_manager`）                        | 否（纯新增）                                         |
| **P1** | B4 假数产出点修复（`accuracy_for_stock`）            | 🔴 我方自有缺陷                   | —                                                                                                | 是（会真开始返回数字，或改为显式不可用）             |
| **P1** | B5 自检入口（doctor 等价物）                         | 🟡 借鉴对方                       | ✅ 真做到（`cli.py:810-971` 真探测）                                                             | 否                                                   |
| **P2** | B6 `fetch_similar_cases` 语义修正                    | 🔴 我方自有缺陷                   | —                                                                                                | 是                                                   |
| **P2** | B7 核心依赖集用测试锁死                              | 🟢 元机制                         | ✅ 真做到（`test_issue_1513_slim_core.py:30-64` 硬断言 22）                                      | 否                                                   |
| **P2** | B8 CI 缺失面补齐（覆盖率）                           | 🟢 元机制                         | ❌ **源项目自己完全没有**（零 coverage 配置）                                                    | 否                                                   |
| **P3** | B9 MCP 工具危险度分级                                | 🟡 借鉴对方                       | ❌ **源项目自己没做**（两套 server 均无 readOnlyHint/destructiveHint）                           | 否                                                   |
| **P0** | O1 领域概念形式化（**概念注册表 + 领域本体**）       | 🔴 我方自有缺陷 + 🟡 借鉴对方     | ✅ **真做到**（`ontology/` 9,298 行：类/属性/domain/range/单位）                                 | **否**（声明 + newtype + 本体 + 副本收敛均零行为变更；存量冲突 09-14 02:45 已裁决执行） |
| **P0** | O2 语义合法性出口校验（CI 门禁）                     | 🟡 借鉴对方                       | ✅ **真做到**（`quality_gate.py` 12 类 issue + 4 阈值 + 5 度量）                                 | 否（已接 CI；`--strict` **只拦未登记量纲**，当前 0） |
| **P1** | O3 模板变更提案审批流                                | 🟡 借鉴对方                       | ✅ **真做到**（`/propose`→`/approve`→`/publish` + 版本 diff）                                    | 是（升版流程从「改了就重种子」变为「先出影响报告」） |
| **P1** | O4 静默失效链的目标化回归测试                        | 🟡 借鉴对方                       | ✅ **真做到**（`tests/ontology/test_shacl_target_namespace.py` 330 行）                          | 否                                                   |
| **P0** | **L1 模块级分层护栏（CI 脚本 + eslint boundaries）** | 🔴 我方自有缺陷 + ⛔ **反面教材** | ❌ **源项目自己最缺**（零分层护栏 ⇒ 长出 3 个跨包环 + 星型扇出）                                 | 否（新增检查，但会先报存量违规）                     |
| **P1** | **L2 供应商适配器惰性注册**                          | 🟡 借鉴对方                       | ✅ 真做到（`graph_store/graph_store.py:525` 轻门面 + 分支内按需 import）                         | 否（纯优化，改冷启动）                               |
| **P2** | **L3 版本存储抽象 + InMemory 实现**                  | 🟡 借鉴对方                       | ✅ 真做到（`change_management/version_storage.py:101 VersionStorage(ABC)` + `:166/:267` 双实现） | 否（纯新增抽象层）                                   |
| **P3** | **L4 节点内算法级覆写钩子**                          | 🟡 借鉴对方                       | ✅ 真做到（`core/registry.py:46` MethodRegistry，业务函数先查 `get(task, name)`）                | 是（会改变节点执行语义）                             |

---

## 详细条目

### B1 — 决策「为什么」设成硬门 【P0】

- **问题**：我方决策记录没有任何机制强制「推理依据」与「归因主体」非空。`stock_analyses.decision_reasoning` 是自由文本，可以写「—」或空串照样落库；`decision_tracker::DecisionRecord` 连字段都没有。
- **借鉴来源**：`context/decision_models.py:86-103` + `context_graph.py:4871-4878` —— 默认策略门 `required_metadata: ["decision_maker"]`、`max_reasoning_length: 1000`，且 `PolicyEngine.check_compliance` 异常分支 **`return False`（fail-closed，`policy_engine.py:396-398`）**、未知规则键也 `return False`（`:935-938`）。
- **源项目是否真做到**：✅ **是**。这是本次审计中唯一一条「它既设计了又真的实现了」的机制（规则本身是硬编码而非配置外置，这一点不要学）。
- **目标状态**：新增一个**落库前的硬门**：`decision_reasoning` 非空且非纯符号、决策来源字段（`decision_maker` 等价物 = 链尾节点 ID）必须存在；不满足 → 落库失败并显式带归因（铁律 36：fail-fast 必须带归因）。**门必须是单一咽喉**（铁律 30），不要在多处复制判定。
- **验证方式**：构造「reasoning 为空」的 fixture → 断言落库被拒且错误消息含具体缺失字段；正对照：正常样本必须通过（铁律 45）。
- **风险**：**行为变更** —— 现有历史数据可能大量不满足 ⇒ 需先跑一次全量存量统计再决定作用于「新写入」还是「含存量」。

---

### B2 — 决策记录双写者收敛 【P0】

- **问题**：见 AUDIT §3 N1。
- **目标状态**：二选一 —— ① 若 `decision_tracker` 的 NDJSON 无真实消费者（须先做**传递闭包**核查，铁律 46）⇒ 删；② 若确有消费者 ⇒ 让它读 `stock_analyses` 而非自建文件。
- **验证方式**：grep `load_records` / `record_decision` / `accuracy_for_stock` 的全部调用链，输出「消费者有无替代路径」判定；正对照必须包含间接调用（薄包装会让真实实现落在引用图外）。
- **风险**：若 NDJSON 被回测链路消费，删除会断链 ⇒ **必须先做完传递闭包**。

---

### B3 — 决策溯源标准导出 【P1】

- **问题**：我方零标准格式导出。决策一旦出问题，只能靠人读 JSON。
- **借鉴来源**：`provenance/manager.py:1256-1268` 用 rdflib 真出 W3C PROV-O 三元组（`prov:Entity/Activity/Agent/wasDerivedFrom/qualifiedGeneration`），`export/export_provenance.py:53-67`。
- **源项目是否真做到**：⚠️ **部分** —— `export_provenance.py:27` 的 `provenance: bool = False`，**默认不追踪**，且模块级 `record_decision` 构造 `DecisionRecorder(graph_store)` 时**未传 `provenance_manager`** ⇒ `_track_decision_provenance` 直接 `return`（`decision_recorder.py:593-594`）。**参考设计意图，不要参考这个默认值。**
- **目标状态**：不要求上 RDF 全家桶。最小可用 = **定一份稳定的「决策溯源导出契约」**（版本化 JSON/JSON-LD，字段与 `ExecutionTrace` 对齐：输入 / 各节点输出 / 落库决策 / 引用的数据源与时间锚点 `as_of`），并保证**默认开启**（与源项目相反）。我方已有的 `ExecutionTrace` + `AsOfSource` 已具备导出所需的全部原料。
- **验证方式**：导出 → 用独立解析器 round-trip 读回 → 断言决策 ID、时间锚点、节点来源三者可完整重建。
- **风险**：无（纯新增）。唯一决策点：是否要求 JSON-LD 兼容。

---

### B4 — 假数产出点修复 【P1】

- **问题**：AUDIT §3 N2 —— `accuracy_for_stock` 恒返回 `(0, 0, avg_conf)`，`_buys/_sells/_holds` 未被使用。任何消费方读到的「命中/总数」都是 0/0。
- **目标状态**：按铁律 32（谁吞了这个 Err）先查消费方 —— 若无消费方 ⇒ 删函数；若有 ⇒ 必须先实现真实「与后市股价对比」的命中判定，**或** 明确返回 `None` / 显式 `NotImplemented` 错误（禁止返回看起来像数据的 0/0）。
- **验证方式**：grep 调用链；正对照用一条已知有历史记录的股票跑通。
- **风险**：**行为变更** —— 修好后会真开始返回数字，消费方 UI 可能出现此前从未出现过的数值。

---

### B5 — 自检入口 【P1】

- **问题**：我方排查一次环境问题要手工拼 `pg-connect.mjs` + `session_logs` mtime + `TEMPLATE_VERSION` 查询。
- **借鉴来源**：`cli.py:810-971` 的 `semantica doctor`：Python 版本 / 图存储 ping / 向量库 import / **embedding 后端真实例化并跑探针 embed** / API key / config / 日志目录。
- **源项目是否真做到**：✅ 是（真探测，非打印版本）。⚠️ 但它把 API key **前 8 位打进输出**（`cli.py:926`）—— 这个不要学。
- **目标状态**：一条命令输出：PG 连通性 + 当前 `TEMPLATE_VERSION` vs 代码常量 + embedding 服务（`localhost:8091`）真实探针 + 必需 vendor token 是否存在 + 工作流最近一次执行时间。
- **验证方式**：故意停掉 embedding 服务 → 断言 doctor 报红且明确指认是哪一项。
- **风险**：无。

---

### B6 — `fetch_similar_cases` 语义修正 【P2】

- **问题**：AUDIT §3 N3。
- **前置条件（必做）**：按铁律 51，**先枚举 `stock_analyses.outcome` 的真实取值域**（自造输入的测试证明不了任何事）。
- **目标状态**：要么让「相似」名副其实（同股 + 同行业 + 同 regime + 语义近邻），要么把函数名与注入描述改为真实语义（「本股近 90 天亏损案例」）—— **禁止名不符实**。
- **验证方式**：SQL 直查 `outcome` 的 distinct 取值 + 计数。
- **风险**：改描述是纯文案；改算法会让注入内容变化 ⇒ 影响 LLM 输入，属**行为变更**。

---

### B7 — 核心依赖集用测试锁死 【P2｜元机制】

- **借鉴来源**：`tests/test_issue_1513_slim_core.py:30-64` 硬断言 `len(normalized_names)==22`。
- **源项目是否真做到**：✅ 真做到（有测试守门，且实测 core deps 确为 22）。
- **目标状态**：Rust 侧等价物 = 给「默认 feature 集」或「release 必备依赖集」写一条断言测试。我方已知 `axagent-quant` 导致 release 构建内存高 —— 这正是该机制该管的事。
- **验证方式**：故意加一个重依赖 → 断言失败。
- **风险**：无（纯新增测试）。

---

### B8 — CI 缺失面：覆盖率 【P2｜元机制·反向借鉴】

- **事实**：我方与对方**都没有覆盖率门禁**（对方全仓零 `[tool.coverage]`/`--cov`；我方 vitest 也无 coverage 配置）。
- **目标状态**：**先只统计不卡**（避免铁律 23：会误报的门禁最终会被禁用），跑稳一段时间后再对**关键链**（决策落库 / 引擎调度）设定 `fail_under`。
- **风险**：直接卡门禁会立刻红一片 ⇒ 分阶段。

---

### B9 — MCP 工具危险度分级 【P3】

- **借鉴来源**：MCP 协议的 `readOnlyHint` / `destructiveHint` / `annotations` 标准字段。
- **源项目是否真做到**：❌ **完全没做** —— `semantica_mcp`（21 工具）与 `semantica/mcp_server`（16 工具）两套并存，其中含 `update_node`、`delete_node`（软删），**grep `readOnlyHint|destructiveHint|annotations` 零命中**。这是**纯设计意图借鉴**。
- **目标状态**：我方 `crates/mcp` 的工具定义补分级字段，让调用方能区分只读与破坏性工具。
- **风险**：无（附加字段）。

---

## 本体侧借鉴（二轮新增）

> 来源：`AUDIT-semantica-ontology-deep-2026-09-13.md`。**借纪律，不借依赖** —— 不上 RDF/OWL/SHACL 栈（见「明确不做」第 1 条）。

### O1 — 领域概念形式化（概念注册表）【P0】

- **问题**：投资领域概念（瓶颈 / 供给刚性 / 需求弹性 / 不可替代性 / 护城河 / DQI / 风险等级 / 估值）的**语义定义散落三处**：LLM prompt、`.rhai` 公式、Rust 常量。**没有任何单一权威源**。直接后果：
  - `total_mv` 单位**元**（B 侧）/ 万元（C 侧）混用
  - 格雷厄姆公式 `g` 单位不一致
  - `posterior` **一物三义**
  - `f6=(dqi-50)/50` ⇒ C 级 61.3 分反成 **+0.226 看多**（负值域翻转）
- **借对方什么**（`ontology/` 的做法，非代码）：① 每个概念是一个**显式对象**（不是散落的字符串）② 带 `domain` / `range` / **单位** ③ 命名规范统一（`naming_conventions.py:71-395`）④ IRI/标识铸造有唯一入口（`namespace_manager.py:102-288`）
- **我方目标状态**：建 `src-tauri/crates/harness/src/domain_concepts.rs`（或 `config/domain-concepts.yaml`）—— 每概念声明
  `name` / `kind`(class|property|metric) / `unit` / `value_range` / `produced_by`(节点 id) / `consumed_by`(节点 id 列表) / `description`
- **验收标准（可证伪）**：任一概念（如 `total_mv`）的**单位声明在全仓只有一处**；其余位置 grep 到的是**引用**而非字面量
  > ⚠️ **2026-09-14 实检更正**：本条**误判了问题形态**。实证是「**同一个名字**承载多个**不同概念**、各自内部自洽」
  > （`confidence` 10 个、`score` 3 个、`min_confidence` 3 个）⇒ 正确形态不是「单位声明唯一」，而是
  > 「**按承载者区分并逐条登记**」。原标准若照做，只会把 10 个概念硬压成 1 个 —— 那是把二义**固化**而非消除。
- **风险**：首轮会**暴露存量语义冲突**。按铁律 7，**报告不擅改** —— 冲突清单单列交裁决
- **首次执行（2026-09-14 02:20 · 已落地 v1）**：`src-tauri/crates/harness/src/domain_semantics.rs`
  - **结构**：`Unit` enum（`Ratio` / `Percent` / `Cny` / `Loss`）+ `ConceptDecl`
    （`id` / `name` / `carrier` / `unit` / `min` / `max` / `meaning` / `evidence`）+ `CONCEPTS` 常量表。
  - **规模**：**24 个概念 / 10 个表面名**，每条均带可核对的源码锚点（`evidence` = `路径:行`）。
  - **消歧键是 `carrier`**：`confidence` 一名 **10 概念**（0–100 ×2、0–1 ×8）；`score` **3 个**；
    `min_confidence` **3 个**；`position_pct` **2 个**。同一名字下 `carrier` **必须互不相同**（有测试锁定）。
  - **防腐烂**：`test_ambiguous_names_locked` 锁死多义名单 —— 新增多义而不同步登记 ⇒ 测试红。
  - **与 O2 的联动**：O2 脚本从「自带清单」改为**解析本表**；自检新增 9 项注册表断言
    （含「空文本解析出 0 条」「缺 `unit` 的条目被丢弃」两个**负向对照**）。
  - **落地效果**：扫描器 A 段由 v1 的「33 处冲突」变为 **`未登记量纲 = 0`、覆盖率 100%（560/560）**；
    并据此**补登记 3 个此前漏掉的概念**（`CalibrationResult.score` 的损失值量纲、
    `ValueAssessmentAgent` 与 `CostAwareRouter` 的 `min_confidence`）。
  - **定位**：**纯声明 + 纯查询，零行为变更**（`cargo test -p axagent-harness` 全绿）。
  - **未做**：`Unit` 暂无 `Count`（计数）与「价格」变体（价格与 `Cny` 同量纲、但语义不同）—— 出现时再补。
- **第二次执行（2026-09-14 02:45 · 补上「执行侧」）**：注册表只是**声明**，拦不住新代码写错量纲
  （`f64` 之间互相赋值永远合法）。本轮补上**量纲化 newtype**，把归属从「靠人看」变成「靠编译」：
  - `Ratio01`（`[0,1]`）与 `Percent100`（`[0,100]`）：**构造即钳制**（`NaN → ZERO`）、
    `is_in_range()` 判原始值（钳制不掩盖越界）、跨单位**只经 `From` 显式换算**、
    `#[serde(transparent)]` 保证 **JSON 仍是裸数字**（前端 `matchConfidence * 100` 契约不变）。
  - **首个消费端 = 刚被证明写错量纲的那个字段**：`EvidenceCitation.match_confidence: f64 → Ratio01`。
  - **能力边界（写进报告 §4.4）**：newtype **保护不到跨语言边界**（TS 侧只有 `number`）⇒
    前端同步删掉 `Math.min(x*100, 100)` 的**静默压平**，越界改为红色告警。
    那个 `Math.min` 正是本次缺陷潜伏很久的原因（把「恒满格」伪装成正常渲染）。
  - **假测试警告**：钳制型 newtype 会让「在范围内」断言**恒真** ⇒ 量纲回归症状是
    「**饱和到满值**」，回归测试断言写成「命中但未饱和」（`0.05 < max < 0.99`）。
- **第三次执行（2026-09-14 03:15 起 · 用户裁定「瓶颈三力·含接线」）**：**这才是第一条真本体**。
  前两次产出的是**字段层横切注册表**（24 行平铺 / 无父子 / 无关系 / 无公理），形态上不是 ontology；
  本轮按用户选定方向，把第一刀落在**真正有重复病**的地方。
  - **新增** `src-tauri/crates/harness/src/domain_ontology.rs`（806 行 / 17 测试）：
    **11 类**（2 根 + 4 层继承链 `IndustryElement → IndustryChain → ChainStage → Chokepoint`）／
    **4 对象关系**（`occupies` / `produces` / `depends_on` / `measured_by`，带 `domain`→`range`，两端须为已声明类）／
    **4 度量**（带单位 + 值域 `[0,100]`）／**7 公理**（每条 `enforced_by` 指名强制它的测试）／
    **4 分档**／**2 口径分歧登记**。查询能力：`is_subclass_of`（传递闭包）/ `superclass_chain` /
    `subclasses_of` / `band_for_score`。**零行为变更**。
  - **副本收敛（本次核心收益）**：三力权重实测量出 **7 处副本**（此前只发现 4 处）——
    `bottleneck-calc.rhai:175-177`、**`strategy-scorer.rhai:212-214`（第二条独立脚本自持一份兜底，本轮新发现）**、
    两处分档阈值、`seed_serenity.rs:1352-1380`（变量值 3 + 描述文本 3 = 6 个数字）、
    `chokepoint-identifier.md:101-102`、`bottleneck-calc.rhai:3,:173` 散文注释。
    ⇒ **权威源唯一** `BOTTLENECK_WEIGHTS`；`seed_serenity.rs` 里**已无任何权重数字字面量**，
    值走 `json!(bw.supply)`、描述文本走 `format_weight(bw.supply)`。
  - **`format_weight` 是唯一入文通道**：`format!("{}", 0.30_f64)` 得 `"0.3"`（丢尾零）⇒
    种子产物会变、触发重种子却无线索。`format_weight` 固定 `{:.2}` ⇒ 产物**逐字节不变** ⇒
    **不需要**递增 `TEMPLATE_VERSION`。公理 A7 + 测试 `test_format_weight_keeps_trailing_zero`
    （含 `assert_ne!` 负控）锁死这一点。
  - **两种副本两种处置（刻意不合并）**：**数值副本** = 同口径抄多份 ⇒ **硬拦**（CI 红）；
    **口径分歧** = 两个不同定义并存 ⇒ **只登记**（`OPEN_DIVERGENCES`）+ 报告，`--strict-divergence`
    须显式开启。混为一谈会导致「擅自替业务裁决」或「把真缺陷降级成备注」。
  - **门禁** `scripts/check-ontology-consistency.mjs`（**2026-09-15 扩充后 1287 行 / selftest 47/47 / 实跑 exit 0**；
    扩充前为 774 行 / 26 selftest）：
    ① 权威源解析失败 ⇒ `exit 3`（**绝不静默报 0 违规**）② **站点清单是契约**（`expect` 命中数为 0 ⇒ 失败，
    因为只可能是代码重构或模式腐烂）③ **自锚定**（脚本的脆弱不得伪装成代码的问题 —— 实测踩过）④
    **双向守卫**（正向查接线引用 + 反向查字面量 + 锚点存在性，即「声明 ≠ 接线」）⑤ 扫描面自证（读 0 文件 ⇒ exit 1）
    ⑥ **引用类字段必须验指向**（`enforced_by` 指向的 `#[test] fn`、`evidence` / `Divergence::site` 指向的文件
    必须存在 —— 自由文本字段只断言「非空」⇒ 改名后「不存在」与「通过了」在门禁看来是同一件事）；
    ⑦ **只验文件、不验行号**（行号必然随重构漂移，判据 #186–#189）。
    2026-09-15 扩充的三类现场：三力**内部**权重（`*_PARTS` 在 2 个 rhai 里 6 处 / 16 个字面量，此前
    `parseParts()` 解析了却**从未进入 `problems.push`**，判据 #197「悬空解析」）+ `enforced_by` 指向校验
    + `evidence`/`site` 文件存在性；顺带实现了被 `package.json` 承诺却只声明未实现的 `--list`。
    ⇒ **门禁覆盖 ≠ 有人消费**：数字与引用不再腐烂，但本体的类/关系/查询接口生产消费方仍为 0
    （见 `AUDIT-ontology-vs-palantir-2026-09-15.md` §2.2 / §2.4.2）。
  - **活体自证** `PROBE_OK=true`：漂移 `else { 0.35 }`→`0.40` ⇒ 门禁 exit 1 且指名现场
    （`权重副本与权威源不一致：期望 0.35，实际 0.4`）；还原后 sha256 **逐字节相同**。
  - **⚠ 待裁决 D1/D2**（**未擅改**，属产品语义决策）：
    **D1** 复合分定义式 —— prompt `chokepoint-identifier.md:101` 写**等权 `/3`**，rhai 实现是**加权 `Σwᵢfᵢ`**；
    若按等权则 `BOTTLENECK_WEIGHTS` 应全为 `1/3`、前端「可调权重」即**假配置**。
    **D2** 分档 —— prompt `:102` 写 `80/60`，rhai 是 `75/55/35`；**先决问题是两侧是否对同一个量分档**。
  - **未做**：分项权重（`SUPPLY_RIGIDITY_PARTS` 等）已在**本体中声明并被测试锁定，但未接线**
    ⇒ 现状是「声明了却没人引用的权威源」；`strategy-scorer.rhai` 的独立兜底仍只是**监控**未收敛。
  - **报告**：`docs/audits/AUDIT-domain-ontology-bottleneck-2026-09-14.md`。

### O2 — 语义合法性出口校验（CI 门禁）【P0】

- **问题**：我方 `scripts/` 下有 20+ 契约脚本，但**清一色是结构检查**（引用是否为零 / i18n key 是否齐全 / 行尾是否 LF / 命令是否注册）。**没有任何一条在检查「字段的值在语义上是否合法」**
- **借对方什么**：`quality_gate.py` 的三件套 —— ① **issue 分类**（12 类，如 `MISSING_DOMAIN` / `UNKNOWN_RANGE` / `INVALID_DATATYPE_RANGE`）② **阈值门**（`max_errors=0` / `min_coverage` / `fail_on_warnings`）③ **覆盖率度量**（`class_coverage` / `property_coverage`，见 `:347-364`）
- **我方目标状态**：新增 `scripts/check-domain-semantics.mjs`，四类检查：
  1. **值域**：`price > 0`、`confidence ∈ [0,100]`、`position_pct ∈ [0,100]`
     > ⚠️ **2026-09-14 实检更正**：本条原文写的是 `confidence ∈ [0,1]` / `position ∈ [0,1]` —— **错**。
     > 代码实证：`harness/dashboard_report.rs:260` 判 `>100.0` 越界、`recommender/mod.rs:650` 为 `clamp(0.0,100.0)`。
     > 这是「没有权威源 ⇒ 凭直觉设期望值 ⇒ 写进 PLAN 也不自知」的实例（铁律 75：反向断言须锚定被测对象自身形态）。
  2. **单位**：同量纲字段单位一致（依赖 O1 的注册表）
  3. **必填**：决策对象必须有 `reasoning` / `decision_maker`（与 B1 呼应）
  4. **互斥**：`action=BUY` 时 `stop_loss` 必填；`action=WAIT` 时 `target_price` 应为空
- **验收标准**：必须在 `.github/workflows/ci.yml` 里**真跑**。对应铁律 11 —— 「规则文件 ≠ 检查脚本 ≠ CI 真跑 ≠ 代码真遵守」四条独立链缺一不可
- **风险**：首跑必然报一批存量违规 ⇒ **先 `--report-only` 出报告，确认后再转门禁**（否则会像对方 `--soft-fail` 那样被绕过，铁律 16）
- **首次执行（2026-09-14 01:52 · report-only）**：`scripts/check-domain-semantics.mjs` 已落地。
  - 自检：**7 组正负对照全过**；扫描面自证 **3074 个源文件**（0 ⇒ 非 0 退出）。
  - **PLAN 登记的 4 个病点只有 1 条经得起实检，而那条是「PLAN 自己写错」**（见上方检查项 1 的更正）。
  - ⚠️ **v1 结论有误，已于 v2 更正**：v1 报「域内同名多量纲 2 个概念（`confidence` / `score`）」——
    **措辞与判据都是错的**。v1 的数据结构是「一个名字一个期望量纲」+「路径前缀当域」，而现实是
    「一个名字承载多个概念」⇒ 该结构无法表达现实，必然把一半的出现误报成「冲突」；
    `scope` 用位置代替身份，还把 `workflow_ai/generate.rs`（节点推荐）算进了投资域。
  - **v2 改造（配合 O1）**：清单改读注册表；判据改为「现场量纲簇**是否被登记概念覆盖**」——
    未覆盖 ⇒ 真异常（A 段），被覆盖 ⇒ 已知多义（B 段，**非缺陷**）。自检扩至 **7 + 9 项**。
  - 报告 → `output/domain-semantics-report-2026-09-14.txt`（94 行）
  - **未做**：接入 `.github/workflows/ci.yml`（验收标准要求真跑）。需先确认存量清单，再转门禁。
- **接入 CI（2026-09-14 02:45 · 验收标准达成）**：`.github/workflows/ci.yml` → `frontend-check`
  紧跟「Check line endings (LF)」之后：
  ```yaml
  - name: Check domain semantics (unit registry / one-name-many-concepts)
    run: |
      node scripts/check-domain-semantics.mjs --selftest
      node scripts/check-domain-semantics.mjs --strict
  ```
  放这么前的理由与行尾检查相同：**纯 Node、不依赖 `npm ci`、几秒内出结果**。
  - **门禁语义（本轮的设计决策）**：`--strict` **只拦 A 段（未登记量纲 = 由代码推出的硬事实）**；
    PLAN 病点属**待办态** ⇒ 默认只报告。**拦待办态会逼人删条目 —— 信号不是被解决，是被消灭。**
    需要拦时显式 `--strict-plan`（人工巡检用，不设为默认）。
  - **门禁自证**：判定抽成纯函数 `gateExit()` 并自检**四格**（含「默认放行」那一格）——
    从不 fail 的门禁与没有门禁等价；不覆盖默认格的话，别人把默认值改反就静默失效。
  - **结案项转正向守卫**：`O-claim-3` 原文检查「PLAN 写了 `position ∈ [0,1]`」，而 PLAN 已更正
    ⇒ 该检查会**永远报同一句「PLAN 写错」**（垃圾信号）。改写为正向守卫
    「`position_pct` 恒为 0–100，不得再出现 0–1 口径」，新增 `已结案` verdict。
  - **当前门禁状态**：`--strict` **exit 0** ✅（A 段 0 / 覆盖率 100%）；`--strict-plan` 亦 exit 0
    （4 条 PLAN 病点：`已消失` 1 / `已结案` 1 / `查无实据` 2）。

### O3 — 模板变更提案审批流【P1】

- **问题**：`TEMPLATE_VERSION` 升版是**「改了就重种子」** —— 无影响面报告、无审批、无回滚点（只能 `cp` 到 `output/backup-<日期>/`）
- **借对方什么**：`explorer/routes/ontology.py:3233-3565` 的 **proposal → approve → reject → publish + comment** 审批链，配 `/versions/{uri}`（版本列表）与 `/drafts/{uri}`（草稿）
- **我方目标状态**：升版脚本先产出 **diff 影响报告**，含
  ① 节点增删改清单 ② 参数增删改清单（对照 `PORTFOLIO_MGR_TUNABLE_PARAMS` 28 项）③ **受影响的存量决策记录数**（`stock_analyses` 按版本过滤）④ 建议回滚点
  人工确认后才执行 `setval` + 重种子
- **风险**：升版流程变长。换来的是**可追溯 + 可回滚 + 影响面可见**

### O4 — 静默失效链的目标化回归测试【P1】

- **问题**：我方铁律 9 / 10 / 18 / 19 / 23 / 25 全是**静默失效族**，但大多只停留在「知道有这回事」，**没有专属测试锁死** —— 下次重构照样复发
- **借对方什么**：`tests/ontology/test_shacl_target_namespace.py`（**330 行，专测一个静默失效场景**：命名空间错配 ⇒ pySHACL 报 conforming、验证静默通过）。**一个静默失效 = 一个专属测试文件**，这是它的纪律
- **我方目标状态**：把「静默失效清单」逐条转成测试：
  | 静默失效链                                     | 测试断言                                                      |
  | ---------------------------------------------- | ------------------------------------------------------------- |
  | `?code` vs `?stockCode` 双参数名（铁律 19）    | 写入端用 `?code`，断言消费端两个名字都能取到                  |
  | `browserMock.ts` 未 mock 命令返空壳（铁律 23） | 断言「未 mock 的命令**必须显式报错或标注**，不得静默返 `[]`」 |
  | 同一 sentinel id 重复定义（铁律 18）           | 改一处后断言另一处**立刻可见**                                |
  | Switch 默认分支放行（铁律 47）                 | 断言「无 `matched_label` 时不得放行非默认档」                 |
- **验收标准**：每条测试必须**在修复前红、修复后绿** —— 否则它测的不是那个缺陷

---

## 分层侧借鉴（三轮新增）

配套专项：`AUDIT-semantica-layering-deep-2026-09-13.md`

### L1 — 模块级分层护栏（CI 脚本 + eslint boundaries）【P0】

- **问题**：我方分层纪律**只写在 `AGENTS.md`（规则文件）** —— `eslint.config.js` 无 `no-restricted-imports` / boundaries（已核实全文），`scripts/` **28 个契约脚本零个查依赖方向**，CI 无对应门禁 ⇒ **铁律 11 的四链只成立了第一链**。
- **实证后果**（全部文件级证据）：

  | 越界形态                             | 存量                  | 证据                                                                  |
  | ------------------------------------ | --------------------- | --------------------------------------------------------------------- |
  | `commands` 绕过业务层**直连 DB**     | **97 文件**           | `stock_workflow/core.rs:19-23`、`evaluator.rs:188`、`gateway.rs:1069` |
  | 命令之间**横向互调**                 | 3 处                  | `cognitive.rs:1090` / `:1736` / `:1790`                               |
  | 函数体内**延迟导入**（规避层次）     | 504 处（生产相关 67） | `src/commands/**`                                                     |
  | 前端 `domain → feature` **反向依赖** | **10 条**             | `domain/conversationStore.ts:21-25`                                   |
  | 无人依赖的 **crate**                 | **1**                 | `axagent-crdt`（1,111 行零消费方，见 N4）                             |
  | 契约层**混入实现**                   | —                     | `harness`：`pub trait` 231 vs `pub struct` 1067（见 N6）              |

- **借对方什么**：**不是借机制 —— 对方最缺的就是这个。** 是借**反面实证**：Semantica 无任何分层护栏 + Python 无编译期约束 ⇒ 长成**星型扇出（cli 扇出 22）+ 3 个跨包环 + `context→GraphStore` 具体类直连 6 处 + `core/` 零抽象基类**。它证明：**没有模块级护栏，crate 边界内的病会扩散成整仓的病。**
- **落地方案**：新增 `scripts/check-layer-discipline.mjs`，挂进 `npm run contracts` + CI：

  | # | 规则                                                                                  | 现状违规                  |
  | - | ------------------------------------------------------------------------------------- | ------------------------- |
  | 1 | `src/commands/**` 不得 `use sea_orm::` / `use axagent_entities::`（须经 dao/service） | 97 文件                   |
  | 2 | `src/commands/*.rs` 不得跨模块调 `crate::commands::X::`                               | 3 处                      |
  | 3 | `crates/harness/src/**` 不得出现 `use axagent_*`（**锁死扇出 = 0**，防未来破坏）      | 0（当前合规，用脚本锁住） |
  | 4 | 每个 `crates/*` 必须有 ≥1 个 workspace 内消费方                                       | 1（crdt）                 |
  | 5 | `src/stores/domain/**` 不得 import `../feature/**`（eslint 分区）                     | 10 条                     |

- **推进方式**：规则 1 / 5 有存量违规，**不可直接 error**。采用与 `check-hardcoded-i18n.sh` 一致的 **allowlist「只减不增」** 模式。
- **验收标准**：① 新增越界代码会被 CI 拦住（**自证有效**，铁律 16）；② allowlist 行数只能下降；③ 脚本须**自陈覆盖范围**（哪些层没查）。

#### ✅ 执行（2026-09-14 03:05）—— 验收标准三条全部达成

**交付**：`scripts/check-layer-discipline.mjs`（5 规则 + 棘轮 + 自检 + 覆盖自陈）｜
`scripts/layer-discipline-allowlist.json`｜`ci.yml` 新步骤｜`ci-check.mjs` 补 4 条｜
`package.json` 新增 `check:layers*`｜`src/__tests__/ci.workflow.test.ts`（`ci.yml` 守卫）
→ 报告 `AUDIT-layer-discipline-l1-2026-09-14.md`

**实测基线**（**不采信本计划原表的数字** —— 原表由上一轮审计得出、与同判据实测不符）：

| 规则 | 级别 | 实测 | 原表 | 差异 |
|---|---|---|---|---|
| `commands-no-direct-db` | 棘轮 | 656 / 122 文件 | 97 文件 | 我方含**行内全路径** |
| `commands-no-sibling-call` | 棘轮 | **84 / 36 文件** | 3 处 | 原表**低估**（其列的 3 处只是其中 3 条） |
| `harness-fanout-zero` | **硬拦** | **0** ✅ | 0 | 一致；已升级为违规即 fail |
| `crate-has-consumer` | 棘轮 | **0** ✅ | 1（crdt） | **crdt 已有消费方**（`crates/device/Cargo.toml:11`） |
| `crate-dir-not-orphan` | 棘轮 | **1 → 0** ✅ | — | **新增规则**：`crates/scanner` 孤儿目录 —— **2026-09-14 03:33 用户裁决「删」，已删除并撤销基线** |
| `stores-domain-no-feature` | 棘轮 | 13 / 4 文件 | 10 条 | 另覆盖 `@/stores/feature/…` 别名 |

**关键设计（与原方案的偏离，均有理由）**：

1. **棘轮按「规则 + 文件」计数，不按行号** —— 行随格式化漂移 ⇒ 基线天天失配 ⇒ 最终被人关掉。
2. **规则 3 升级为 `hard`（违规即 fail）**：实测 0 ⇒ 可以真正上锁，不必等棘轮。
3. **规则 4 拆成 4a/4b**：只统计 **workspace 成员**（非成员是「孤儿目录」另一种病），
   并豁免 `crate-type` 含 `staticlib`/`cdylib` 的 **FFI 产物**（`crates/axagent-mobile` 无 Cargo 消费方是设计）。
4. **规则 2 引入 `SHARED_INFRA_MODULES` 豁免（8 个）**：`error` / `error_code` / `constants` / `_shared_state` 等
   是**公共库**，调它们不是「命令横向互调」。不豁免 ⇒ 2635 处噪声里只有 84 处是真问题。
5. **不挂进 `npm run contracts`**（偏离原方案文字）：CI 跑的是 `npm run contracts -- --only=a,c,d,f,g,h`，
   若把它做成 `check-contracts.mjs` 的规则 `i`，会被 `--only` 列表**静默跳过**
   ⇒ 「接了线但没接活」。改为**独立脚本 + 独立 CI 步骤 + 独立 selftest**。

**三类假阳性已被修掉而非写进基线**（详见报告 §3）：字符串字面量内的路径、共享基础设施模块、FFI 产物。

**自证**：`--selftest` **51 格**（5 规则正负对照 + `stripStringLiterals` + 归属判定 + `gateExit` 四格含默认放行）；
**活体自证**：插入越界探针 ⇒ `exit 1` 并精确指认「新越界文件」⇒ 删除探针 ⇒ `exit 0`。

### L2 — 供应商适配器惰性注册【P1】

- **问题**：`crates/providers/src/registry.rs:70 register_all_builtins()` **硬编码 13 个 adapter 并即时 `Arc::new(XxxAdapter::new())` 全量实例化** ⇒ Tauri 桌面端冷启动即构造全部供应商。
- **借对方什么**：`graph_store/graph_store.py:525` 的「**轻门面 + 重后端按需 import**」—— 门面类只持 `backend_type` 字符串，**只在命中该后端的分支内**才 `from .neo4j_store import Neo4jStore`；用不到的驱动根本不 import。
- **我方目标状态**：`register(key, factory)` 惰性化 —— `HashMap<String, Box<dyn Fn() -> Arc<dyn ProviderAdapter> + Send + Sync>>`，首次 `get(key)` 才实例化。
- **风险**：低（纯延迟化）。需先确认**有无「注册即需副作用」**（读环境变量 / 建连接 / 探活）。

### L3 — 版本存储抽象 + InMemory 实现【P2】

- **问题**：模板版本治理（`TEMPLATE_VERSION` 重种子 + `crates/kit/src/workflow_version.rs`）**无存储抽象**，直接绑 DB ⇒ 单测无法替换后端，只能打真库。
- **借对方什么**：`change_management/version_storage.py:101 class VersionStorage(ABC)` + `:166 InMemoryVersionStorage` / `:267 SQLiteVersionStorage`；`managers.py:39 BaseVersionManager(ABC)` 面向 ABC 编程 —— **这是对方全仓唯一「面向抽象 + 可换实现」的地方**（其余 3 个 store 全是 if/elif）。
- **我方目标状态**：定义 `trait VersionStorage`（读 / 写 / 列版本 / 取 diff），提供 `PgVersionStorage`（生产）+ `InMemoryVersionStorage`（单测）。
- **验收标准**：版本相关单测**不再依赖真 PG**。

### L4 — 节点内算法级覆写钩子【P3】

- **问题**：`rt-workflow` 节点只能**整只替换**（换节点类型），无法覆写**单步算法**（例如只换 `bottleneck-calc` 里的一条公式而不动其余）。
- **借对方什么**：`core/registry.py:46 MethodRegistry` —— 各 store 的业务函数**统一先查** `method_registry.get(task, name)`，命中则走用户覆写版（`triplet_store/methods.py:158/190/218`）。「同一 API、单步可覆写」的轻量扩展点。
- **⚠️ 行为变更 + 前置依赖**：会改变节点执行语义；且**必须在 O1（概念注册表）之后做** —— 否则覆写无类型/单位校验，会再造一批静默失效。

---

## 明确不做（防范围蔓延）

| 不做                                             | 原因                                                                                                                                                                                                                                                                                                                  |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 引入 RDF / OWL / SHACL / triple store 全家桶     | 我方栈是关系库 + JSON，为**本体表达力**改造全栈不成比例。O1/O2 已用「Rust 常量注册表 + CI 脚本」的最小方案覆盖同等纪律。**⚠️ 注意：不引入依赖 ≠ 不借纪律** —— 对方本体层真正值钱的是 `quality_gate.py` 的「分类 + 阈值 + 度量」三件套与 `test_shacl_target_namespace.py` 的测试纪律，这两样**我方要借**（见 O2 / O4） |
| 复刻前向链 / Rete / Datalog                      | 我方已有 Rhai 公式层；对方这三个引擎**除 forward_chain 外全部无生产调用**，投入产出为负                                                                                                                                                                                                                               |
| 复刻 `ContextGraph` 内存图模型                   | 我方 PG 持久化强于它；且它的多后端是 `if/elif` 硬编码，不是真抽象                                                                                                                                                                                                                                                     |
| 引入 `find_similar_decisions` 的词级 Jaccard     | 比我方现有检索弱（见 AUDIT §2 维度 4）                                                                                                                                                                                                                                                                                |
| 抄它的 CI 形态                                   | 对方 CI 无 lint / 无 mypy / 无覆盖率，**弱于我方**                                                                                                                                                                                                                                                                    |
| 抄 `analyze_decision_impact` 的 0.5/0.3/0.2 加权 | 我方 `CausalEdgeStats` 有 Welford 方差与置信度封顶，更严谨                                                                                                                                                                                                                                                            |
| 抄 `doctor` 打印 API key 前 8 位                 | 信息泄露                                                                                                                                                                                                                                                                                                              |
| **把 97 个 commands 一次性迁到 service 层**      | L1 的目标是**阻止继续劣化**，不是重写存量。存量走 allowlist「只减不增」；一次性大迁移 = 高回归风险 + 与业务收益无关。分层审计报告 §7 已注明「97」含「仅引用类型」，越界量可能低于 97 —— **改前须先精确区分**                                                                                                          |

---

## 执行前置条件（需用户决策）

1. **B1 / B4 / B6 / O1 均为行为变更**：按铁律 59 的反向要求，改之前必须先证明「上游候选集合本身是干净的」——即先跑一次**存量数据统计**（有多少决策 `reasoning` 为空 / 有多少 `accuracy_for_stock` 的调用方 / `outcome` 的真实取值域 / **各领域概念单位的实际使用情况**）。
2. **O2 必须先以 `--report-only` 落地**：直接转门禁会在首次运行就全红，进而被绕过（对方 `defender-for-devops.yml:79 --soft-fail` 就是前车之鉴，铁律 16）。
3. **B2 的删除方向**取决于传递闭包结果，**不可先斩后奏**。
4. 本清单**不触碰 git**：改动清单由我在报告里逐条列出，回滚副本置于 `output/backup-2026-09-13/`。
