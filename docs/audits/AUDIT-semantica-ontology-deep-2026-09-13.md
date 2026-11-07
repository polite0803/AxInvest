# AUDIT（专项）— Semantica 的 Ontology 层 vs AxInvest

> 状态：active ｜ 日期：2026-09-13 ｜ 对应版本：v2.9.7


> 日期：2026-09-13
> 取证方式：源码级（`output/tmp-semantica-src/semantica-main`，git 零接触）
> 关联：`AUDIT-semantica-vs-axinvest-2026-09-13.md`（总报告）、`PLAN-semantica-borrowings.md`

---

## 0. 结论修正声明（我上次错了）

用户在上一轮指出「分析不够深入，其 ontology 应该是明显优于本地的」。**这个判断成立，我上一轮的结论是错的。**

### 我错在哪（三条，逐条认领）

| # | 上一轮的错误结论                                           | 事实                                                                                                                                    | 错的根因                                                                                             |
| - | ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| 1 | 「`validate_graph` **全仓无调用点**」                      | 真实调用点在 `explorer/routes/ontology.py:2950`（`POST /shacl/validate`）与 `cli.py:3109/3421`                                          | grep 范围只扫了 `semantica/` 内部，**没扫 `explorer/routes/` 和 `cli.py`** —— 典型的「入口枚举不全」 |
| 2 | 维度 7 写成「Semantica（形式化）；AxInvest（**真接线**）」 | 这是**伪对冲**。我把「本体（OWL/SHACL/SKOS）」偷换成了「约束治理（参数四道门）」，用「我们的约束真接线」去抵消「他们的本体更形式化」    | **检法用错了**：用「机制是否接线」这一把尺子去量所有维度                                             |
| 3 | 总报告把 ontology 归入「出口不严型」的证据链               | 出口不严**成立**（`pipeline` 缺 handler 静默直通、`reasoning/` 四引擎只有一个接线），但**本体层是例外** —— 它是这个仓库里质量最高的部分 | 把「仓库整体病型」直接套到了「单个模块」上                                                           |

### 为什么这个错误值得单独记一笔

我上一轮的检法是「**声称的能力有没有被生产调用**」。这个检法对「推理引擎」「pipeline」是对的，对「本体」是错的：

- 推理引擎的价值 = **被执行**（不被调用就是死代码）
- 本体的价值 = **可表达 / 可发布 / 可校验 / 可互操作**（它本身就是产物，不一定要在某个运行时链路里被 call）

**拿「有没有被调用」去否决一个本体实现，就像拿「这台印刷机今天印了多少张」去否决它印的字典好不好。** 这是我这次要沉淀的方法论教训。

---

## 1. 对方本体栈全貌

`semantica/ontology/` 是一个**独立子系统**：**25 个模块 / 9,298 行**，模块划分遵循**正统本体工程方法论**（不是随手堆的）。

```
semantica/ontology/
├── engine.py                681  OntologyEngine（统一门面，承接生成/校验/版本对比/SKOS）
├── ontology_generator.py   1394  6 阶段生成管道 + SHACLGenerator + OntologyOptimizer
├── quality_gate.py          855  本体质量门（CI 友好，12 类 issue + 4 阈值 + 5 度量）
├── property_generator.py    517  属性推断（object/data 分离 + domain/range 推断）
├── reuse_manager.py         510  本体复用（对齐评估 / 外部导入 / 合并）
├── owl_generator.py         466  OWL 序列化（rdflib 真出三元组）
├── class_inferrer.py        436  类推断 + 层级构建 + 环检测
├── associative_class.py     410  关联类（n-ary 关系形式化）
├── ontology_validator.py    404  SHACL 校验封装（pyshacl）
├── competency_questions.py  399  能力问题（CQ）管理 —— 本体工程正统需求方法
├── naming_conventions.py    395  命名规范（PascalCase 类 / camelCase 属性 / 单数 / 名词短语）
├── ontology_evaluator.py    390  本体评估（完整性 / 缺口 / 粒度 / 关系完备性）
├── ontology_documentation.py 344  本体文档（作者/贡献者/标注/Markdown 生成）
├── namespace_manager.py     322  命名空间与 IRI 铸造
├── requirements_spec.py     262  需求规格（CQ 挂载 + 需求追溯）
├── module_manager.py        261  本体模块化（imports 解析）
├── methods.py               198  方法注册表
├── config.py                169  配置（文件 + 环境变量双层）
├── registry.py              137  方法注册中心
├── llm_generator.py         133  LLM 辅助生成本体
├── ontology_provenance.py    69  本体变更溯源
├── relationship_utils.py     58  关系端点解析
└── vocabulary/semantica-ns.ttl 243  ★ 手工精修的发布级 OWL 词汇表
```

**配套的治理面**（这才是它真正的完整度）：

| 面          | 规模                                                  | 说明                                                                                    |
| ----------- | ----------------------------------------------------- | --------------------------------------------------------------------------------------- |
| REST API    | `explorer/routes/ontology.py` **3,784 行 / 30+ 路由** | 含 `/skos/*`、`/alignments`、`/shacl/*`、`/proposals/*`、`/versions/*`                  |
| SPARQL 端点 | `explorer/routes/sparql.py` 310 行                    | 仅放行 SELECT/ASK/CONSTRUCT/DESCRIBE + 并发信号量 + 行数上限                            |
| 导出器      | `semantica/export/` **11,625 行 / 18 个导出器**       | owl / rdf / json / lpg / parquet / arrow / neo4j_csv / arango_aql / vector / distance … |
| 测试        | `tests/ontology/` **5,881 行**                        | 含 `test_shacl_target_namespace.py` 330 行（专测静默失效）                              |
| CLI         | `cli.py:3109–3422`                                    | 多个 ontology 子命令（校验/生成/导出）                                                  |

---

## 2. 逐子能力档位表（**不能一概而论**）

| 子能力                 | 判定                 | 关键证据（文件:行）                                                                                                                                                                                                                                                                                                               |
| ---------------------- | -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **SHACL 生成器**       | **真实现·质量高**    | `ontology_generator.py:803-814` 声明 6 阶段管道：`_build_class_index` → `_generate_node_shapes` → `_attach_property_shapes` → `_propagate_inheritance`（**迭代、防环**）→ `_apply_quality_tier`（strict 档置 `closed=True`）→ `serialize`（Turtle/JSON-LD/N-Triples）                                                             |
| **OWL 生成器**         | **真实现**           | `owl_generator.py:210-290` rdflib 真加三元组：`(class_uri, RDF.type, OWL.Class)` / `RDFS.subClassOf` / `RDFS.domain` / `RDFS.range` / `OWL.versionInfo`                                                                                                                                                                           |
| **命名空间管理**       | **真实现**           | `namespace_manager.py:102-288`：`generate_class_iri` / `generate_property_iri` / `generate_individual_iri` + `_to_pascal_case` / `_to_camel_case` + `validate_iri`                                                                                                                                                                |
| **命名规范**           | **真实现**           | `naming_conventions.py:71-395`：类名/属性名/本体名**校验 + 建议 + 归一化**三件套；`_is_singular` / `_is_noun_phrase` / `_is_verb_phrase` 文法判断                                                                                                                                                                                 |
| **质量门**             | **真实现·设计好**    | `quality_gate.py:96-140` 阈值三件（`min_coverage`/`max_errors`/`max_warnings`）+ `fail_on_warnings`；`:142-387` 产出 12 类 issue（`MISSING_DOMAIN`/`UNKNOWN_DOMAIN`/`ORPHAN_CLASS`/`INVALID_DATATYPE_RANGE`/`UNKNOWN_RANGE`…）；`:347-364` 计算 `class_coverage` / `property_coverage`                                            |
| **本体评估器**         | **真实现**           | `ontology_evaluator.py:84-365`：completeness / gaps / suggestions / class_granularity / relation_completeness                                                                                                                                                                                                                     |
| **本体治理平台**       | **真实现**           | `explorer/routes/ontology.py:3233-3565`：`/propose` → `/proposals/{id}/approve` · `/reject` · `/publish` · `/comment` + `/versions/{uri}` + `/drafts/{uri}` —— **完整的变更提案审批流**                                                                                                                                           |
| **SPARQL 端点**        | **真实现**           | `explorer/routes/sparql.py:34` 正则白名单 4 类语句；`:187-196` 并发信号量 + 行数上限                                                                                                                                                                                                                                              |
| **SKOS 词表管理**      | **真实现（有前提）** | `engine.py:409-590`：`list_vocabularies` / `list_concepts` / `search_concepts` 真跑 SPARQL（`CONTAINS(LCASE(...))`）。**前提：必须注入 `TripletStore`，否则 `raise ProcessingError`**                                                                                                                                             |
| **能力问题（CQ）**     | **部分实现**         | 方法论框架真：`competency_questions.py:41-72` 数据结构 + `requirements_spec.py` 需求追溯 + 接进质量门（`quality_gate.py:365-373` → `metrics["competency_question_coverage"]`）；**但判据弱**：`:241-277` `_can_ontology_answer` 自注 `(basic heuristic)`，实现是**关键词子串匹配**（`word in class_name_lower`，仅取 len>3 的词） |
| **模块化 / 复用管理**  | **框架真·内容薄**    | `module_manager.py:86-206` 真做 imports 解析；`reuse_manager.py` 的 `research_ontology` 内**明写 `# Try to load from URI (placeholder)`**，且 `known_ontologies` 只有 3 个通用元数据词表（`reuse_manager.py:92-107`：foaf / dublin-core / schema.org），**无 LOV 对接**                                                           |
| **领域本体模板**       | **薄壳**             | `domain_ontologies.py:74-130`：docstring（`:1-16`）声称 5 个领域（healthcare/finance/legal/research/cybersecurity），**实际只实现 2 个**，且 healthcare 6 类 3 属性、finance 4 类 2 属性 —— 是 demo，不是领域本体                                                                                                                 |
| **`semantica-ns.ttl`** | **真实现·手工精修**  | 243 行，**每个 term 都锚定到产出它的 call site**（`export/rdf_exporter.py` 等），并带 issue 溯源（#1100 / #1104 / #1105 / #1114 / #1121 / #1146 / #1154）                                                                                                                                                                         |

**档位小结**：真实现 12 项 / 部分实现 1 项 / 薄壳 1 项 / 框架真内容薄 1 项。**这与我上一轮「占位或未接线」的概括完全不同。**

---

## 3. 对方本体层最值得学的：**静默失效陷阱已被系统性修复**

这是本次审计**含金量最高的发现**。SHACL/RDF 世界的静默失效陷阱与我方踩过的「断链」是同一族问题，但对方**每一条都踩过、修过、留了注释和 issue 号、并写了目标化测试**：

| 陷阱                                              | 后果                                                                                          | 对方的修复与证据                                                                                                                                                                                                                                                |
| ------------------------------------------------- | --------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **shapes 的 target 指向自己的命名空间**           | `sh:targetClass` 匹配不到任何数据 ⇒ **pySHACL 报 conforming（验证永远通过）**                 | `ontology_generator.py:839-848` 把 `target_namespace` 与 `base_uri` **刻意分离**，并在注释里写明后果；`ontology_generator.py:995-1031` 优先从「术语自己携带的 IRI」推断数据所在命名空间。**专属测试 330 行**（`tests/ontology/test_shacl_target_namespace.py`） |
| **base_uri 尾部 `#` 被规范化掉**                  | `...manufacturing#` → `...manufacturing#/` ⇒ 每个 URI 落错命名空间 ⇒ **SHACL 静默 target 空** | `ontology_generator.py:852-861` 显式保留 `#`，并把 `.../ns////` 折叠成单斜杠                                                                                                                                                                                    |
| **类名与属性名共用一张 IRI 映射表**               | 同名的属性会**静默拿到类的 IRI**                                                              | `ontology_generator.py:794-800` 拆成 `class_iris` / `property_iris` 两张表，注释写明「a single map would silently give it the class's IRI」                                                                                                                     |
| **无条件把「无 domain 的属性」挂到所有 shape 上** | **断言了本体从未声明的约束** ⇒ 误报                                                           | `ontology_generator.py:845-848` 参数 `attach_domainless_properties` **默认 False**，注释：「It invents a constraint the ontology never stated」                                                                                                                 |
| **自由文本名字直接拼进 IRI**                      | `"Customer Account"` → IRI 含空格 ⇒ 严格解析器**直接拒绝**                                    | `ontology_generator.py:772-784` `_mint_term_iri` 做 percent-encode                                                                                                                                                                                              |
| **构造器参数经 `generate(**options)` 转发**       | 参数被**静默丢弃**                                                                            | `engine.py:251-253` 显式改走构造器，并留注释：「These two are constructor-only on SHACLGenerator, so forwarding them through `generate(**options)` silently dropped them」                                                                                      |

> **这六条里有四条，与我方铁律 13（配置项必须两个注入点）、铁律 18（同一 sentinel id 重复定义 ⇒ 改动即静默返空）、铁律 25（clamp 恒等边界）是同一族问题。** 对方在 RDF 语义层的踩坑深度，比我方在业务参数层的踩坑深度**更系统**：它有 issue 号、有注释、有专属测试，形成了「踩坑 → 记录 → 测试锁死」的闭环。

---

## 4. 我方「本体层」的真实形态（**证明是零，且不可替代**）

| 我方资产                                  | 实际是什么                                                                                                                                                                                | 为什么**不是**本体                                                                                     |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `crates/harness/src/schema.rs`            | **节点契约** `NodeContract`：`SchemaFormat::json_schema` + `PrimitiveType`，用于工作流节点**输入/输出校验**（comment 自述：「确保上游输出符合 Schema 才能流转到下游，告别黑盒文本传递」） | 这是**接口契约**，约束的是「一次调用的载荷形状」；本体约束的是「领域里存在什么类、什么关系、什么公理」 |
| `crates/harness/src/workflow_types.rs:62` | `JsonSchema` / `JsonSchemaProperty`                                                                                                                                                       | 同上，参数校验用                                                                                       |
| `crates/harness/src/knowledge_graph.rs`   | `GraphNodeType` / `GraphSourceType` / `ExtractedEntity` / `ExtractedRelation` —— **多源实体抽取图 + 增强检索**                                                                            | 这是**实例层图谱**（类的填充），其类型是**代码内枚举**，不可发布、不可推理、无 domain/range 公理       |
| `knowledge-sources/lemonhu/`              | `nodes.csv` + `edges.csv` + `link_graph.json`（24,288 节点 / 74,791 边）                                                                                                                  | **数据文件**，不是可发布的本体                                                                         |
| `crates/harness/src/as_of.rs`             | 时间锚点（结构化截止 / 新闻实时 / FutureDate 契约）                                                                                                                                       | 时间语义，非本体                                                                                       |
| 前端 `src/**`                             | 零命中                                                                                                                                                                                    | —                                                                                                      |

**结论：我方在「形式化本体」这一维度是 0，不是「弱」。** 我有 OWL/RDFS/SKOS/SPARQL 的全仓 grep 证据：Rust 侧零实现、TS 侧零实现。

而且**不可替代**：「参数四道门」（后端变量 → 前端 `resolve` 白名单 → 消费点 `present()`/`input_mapping` → 控件量程）解决的是**「一个参数在四道关卡上是否都通了」**；本体解决的是**「这个领域里究竟有哪些概念、它们之间允许什么关系、什么值域是合法的」**。前者是**管道完整性**，后者是**领域语义的单一权威来源**。

> 映射到我方的实际痛点：`total_mv` 单位（元 vs 万元）、`f6=(dqi-50)/50` 的负值域翻转、格雷厄姆 `g` 单位不一致、`posterior` 一物三义 —— **这些全是「领域概念的语义没有被形式化」的后果**。参数四道门管不住它们，因为四道门都在问「参数通没通」，没有一道门在问「这个字段在领域里该是什么类型、什么单位、什么值域」。

---

## 5. 为什么这是「层」的差距，而不是「好坏」的差距

|                | Semantica                                                               | AxInvest                                             |
| -------------- | ----------------------------------------------------------------------- | ---------------------------------------------------- |
| **形式化程度** | OWL 类/属性/公理 + SHACL 形状约束 + SKOS 词表                           | 代码内枚举 + JSON Schema 载荷校验                    |
| **可发布性**   | `.ttl` / `.owl` / JSON-LD，可被任何 RDF 工具消费                        | 无外部可消费形式                                     |
| **可推理**     | RDFS/OWL 蕴含（本仓库推理层未全接线，但**本体本身支持**）               | 无                                                   |
| **可互操作**   | 与 FOAF/DC/schema.org 对齐（`owl:equivalentClass` / `skos:exactMatch`） | 无                                                   |
| **变更治理**   | proposal → approve → publish + 版本 diff/影响报告                       | `TEMPLATE_VERSION` 升版即重种子，无审批/无 diff      |
| **域内容厚度** | ⚠️ 薄（领域模板 2 个 demo）                                             | 厚（A 股投资领域知识，但以 md 文档 + rhai 公式散落） |

**这张表说明一件事**：对方有**形式化的骨架但领域内容是 demo**；我方有**厚实的领域内容但没有形式化骨架**。

> 所以「借鉴」的正确姿势**不是「抄一个 OWL 栈」**（我方是 A 股投资决策系统，不需要通用本体引擎），而是：**用它的骨架，装我方的内容**。

---

## 6. 我方该拿的（4 条做法，**不抄实现**）

| #      | 做法                                                                                                                                                                                          | 直接治我方什么病                                                                                            | 优先级 |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------ |
| **O1** | **领域概念形式化**：把「瓶颈 / 供给刚性 / 需求弹性 / 不可替代性 / 护城河」等**投资领域概念**定义成带 `domain / range / unit / 值域` 的**类型注册表**（可先用 Rust 常量表或 YAML，不必上 RDF） | `total_mv` 单位、`f6` 负值域、格雷厄姆 `g` 单位、`posterior` 一物三义 —— **都是「概念语义无处声明」**       | **P0** |
| **O2** | **SHACL 式出口校验**：给每个决策字段定义「值域 + 单位 + 必填 + 互斥」四类约束，做成**CI 门禁脚本**（对应对方 `quality_gate.py` 的 12 类 issue + 阈值）                                        | 我方有 20+ 契约脚本，但**没有一条是「语义合法性」检查**（全是「引用是否为零」「key 是否齐全」这类结构检查） | **P0** |
| **O3** | **本体/模板变更提案审批流**：`TEMPLATE_VERSION` 升版加一步「生成 diff 影响报告 → 人工确认」，对齐对方 `/propose` → `/approve` → `/publish`                                                    | v40→v41 升版是「改了就重种子」，**没有影响面报告**，出问题只能回滚 `cp` 备份                                | P1     |
| **O4** | **目标化静默失效测试**：照 `test_shacl_target_namespace.py`（330 行，专测「命名空间错配 ⇒ 验证静默通过」）的模式，为我方每一条「静默失效链」写**专属回归测试**                                | 我方铁律 9/10/18/19/23 全是静默失效，但**大多只有「知道有这回事」没有「测试锁死」**                         | P1     |

**明确不做**：不引入 rdflib/pyshacl/RDF 三元组栈、不做 SPARQL 端点、不做 OWL 导出。我方是业务系统，不是语义基础设施 —— 借**纪律**，不借**依赖**。

---

## 7. 方法论教训（写进技能）

| #      | 教训                                                                                                                                                                                                    | 适用场景               |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| **M1** | **「有没有被生产调用」不是万能尺子**。对「引擎/管道/执行器」是有效检法；对「本体 / 格式 / 契约 / 规范」这类**本身就是产物**的能力，必须换尺子：**表达力 / 形式化程度 / 可发布性 / 可互操作 / 变更治理** | 审计任何「定义类」能力 |
| **M2** | **禁止用「我方也有 X」做伪对冲**。我上一轮把「本体」对冲成「约束治理」，看似平衡，实为偷换概念。正确做法：**先承认维度落后，再论证「落后是否要紧」**                                                    | 所有对比审计           |
| **M3** | **判「无调用点」必须扫全入口**：`semantica/` 内部 → `explorer/routes/`（REST）→ `cli.py` → `tests/`。上一轮漏了后三者，导致误判                                                                         | 与本项目铁律 10 同源   |
| **M4** | **整体病型不能下推到单个模块**。「仓库是出口不严型」≠「每个模块都出口不严」。本体层就是反例                                                                                                             | 所有对比审计           |
| **M5** | **模块 docstring 与实现要分开验**。`domain_ontologies.py` 声称 5 领域、实现 2 个；`reuse_manager.research_ontology` 自称 placeholder —— **docstring 是宣称，代码才是事实**                              | 与本项目铁律 33 同源   |

---

## 8. 不确定项（**未验证，不要当结论**）

1. `semantica/change_management/` 的 `VersionManager` / `diff_ontologies` / `generate_change_report` 深度未逐行核实（`engine.py:615-681` 引用到，但实现未读）。
2. `tests/ontology/` 5,881 行中，**弱断言（仅 `is not None` / 仅不抛错）的占比未测**。
3. `/proposals/*` 审批流是否接入真实前端 UI（只有 REST 层证据，未查前端）。
4. `OntologyEngine` 在生产调用时**是否真注入了 `TripletStore`**（`engine.py:33` `self.store = config.get("store")`）—— 若未注入，SKOS/对齐类方法全部不可用。**这一条决定「SKOS 词表管理」实际是否可用。**
5. 领域本体模板的 `finance` 域（Account/Transaction/Bank/Customer）与投资决策**无关**，是否另有投资域模板未在 `domain_ontologies.py` 中（未全仓搜）。

---

## 9. 一句话总结

> **对方的本体层不是「占位」，而是这个仓库里唯一一个「既设计了、又真实现了、还被系统性治理过」的完整子系统**（25 模块 / 9,298 行本体 + 3,784 行 REST 治理面 + 11,625 行导出 + 5,881 行测试）。我方在该维度是 **0**，且现有的「参数四道门 + JSON Schema 契约 + 实例图谱」**不能替代**它。真正的借鉴点是**四条纪律（O1–O4）**，不是**一套依赖**。
