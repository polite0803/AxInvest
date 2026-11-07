# AUDIT — 本体 vs 现有 DAG / 知识图谱 / WIKI 的关系

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


> 日期：2026-09-14 ｜ 触发：用户质疑「你通篇都没有分析 ontology 和现有的 DAG、WIKI 的关系」
> 结论：**质疑成立。** 本体在这个项目里的位置不是「新增一个模块」，而是**补上已有三套类型系统缺失的那一层**——而我把第四套并列地摆在了旁边。

---

## 0. 一句话定位

这个代码库里**已经有三套类型系统在跑**，但它们全部只描述「**载体/过程**」（这是哪种节点、怎么流转），
**没有一套描述「领域」**（流过去的是哪个概念、两者之间是什么关系）。
领域层今天**是自由文本 String**（`entity_type` / `relation_type` / `link_type`），**零词汇表约束**。

⇒ **本体的唯一正当位置 = 领域层（TBox），并向上约束载体层、向下为关系表提供值域与 domain/range。**
而我交付的 `domain_ontology.rs` 是一套**与上述任何一套都零交集的并列词汇表**。

---

## 1. 现存四套类型位置（实测）

| # | 位置 | 词汇表 | 形态 | 维度 | 有没有消费 |
|---|---|---|---|---|---|
| 1 | 工作流 DAG | `WorkflowNode` 30+ 变体<br>`EdgeType` 9 变体 | Rust enum | **过程**（谁执行/怎么流转） | ✅ 引擎主路径 |
| 2 | 统一图谱（载体） | `GraphNodeType` 4 值<br>`GraphSourceType` 4 值 | Rust enum | **来源**（节点从哪来） | ❌ **0 消费**（见 §3） |
| 3 | WIKI 页面 | `PageType` 8 值 | Rust enum | **载体形态**（这是哪种页面） | ✅ **进打分链**（见 §2） |
| 4 | **领域层** | **无** | **自由文本 `String`** | **领域** | —（无约束，见 §4） |
| 5 | 我新加的本体 | `CLASSES` 11 / `OBJECT_PROPERTIES` 4 / `METRICS` 4 / `AXIOMS` 7 | Rust 常量数组 | 领域 | ❌ **0 消费**（已登记 §46.10） |

**关键：第 5 套与第 1/2/3/4 套零交集。** 逐一核对：
- `EdgeType`（`direct`/`conditionTrue`/`loopBack`/`parallelBranch`/`merge`/`error`/`grouping`/`debateRound`）里没有 `depends_on` / `occupies` / `measured_by`
- `PageType`（`entity`/`concept`/`source_summary`/`comparison`/`index`/`overview`/`note`/`unknown`）里没有 `Company` / `ChainStage` / `Chokepoint`
- 图谱的 `relation_type` 实际取值只有 `causes` / `contains` / `follows`，没有 `produces` / `measured_by`

⇒ 我的本体**不是任何东西的上游，也不是任何东西的下游**——它是一块孤岛。孤立的必然性不在实现质量，在**选位错了**。

---

## 2. 决定性证据：这个库里**已经有一份手写本体**，而且它**有真实消费**

`crates/harness/src/graph_dtos.rs:319-337`：

```rust
fn compute_type_affinity(&self, page_a: &str, page_b: &str) -> f64 {
    let type_a = self.get_page_type(page_a);
    let type_b = self.get_page_type(page_b);

    if type_a == type_b {
        match type_a {
            PageType::Entity | PageType::Concept => 1.0,
            PageType::SourceSummary => 0.5,
            _ => 0.3,
        }
    } else {
        match (&type_a, &type_b) {
            (PageType::Entity, PageType::Concept) | (PageType::Concept, PageType::Entity) => 0.5,
            _ => 0.0,          // ← 「其余类型对一律禁止」= 一条不相交公理
        }
    }
}
```

这一段就是**本体论意义上的三件事同时在场**：

| 本体构成 | 在这段代码里 |
|---|---|
| 类（TBox） | `PageType` 8 个类 |
| **不相交/兼容公理** | 跨类型只允许 `Entity↔Concept`，**其余全 0.0** |
| **关系被推理消费** | `graph_dtos.rs:79-84`：`total_score = direct_link×3 + source_overlap×4 + adamic_adar×1.5 + **type_affinity×1.0**` |

⇒ **它不是文档，它是活的**：类型对直接改变节点相关性打分，进而影响 `get_top_k_related` 的排序、
影响 `graph_insights.rs:136` 的「Cross-type link」惊奇度（`+1.5`）、影响 `:238` 的知识缺口检测
（「Concept/Entity 且度=0」→ 报告 UnlinkedConcept）。

**对照我交付的 `domain_ontology.rs`：11 类 / 4 关系 / 7 公理，生产消费 = 4 行，全是 3 个权重数字。**

⇒ 这个对比给出了本体的**正确形态模板**：**本体在这个代码库里长什么样，答案是「`compute_type_affinity` 那样，只是它被硬编码进了函数体」。**
所以要做的事不是「再声明一套」，而是**把这张表提出来、扩成类层次 + 带 domain/range 的关系 + 公理，再让它反过来驱动这个函数**。

---

## 3. 顺带挖出：第 2 套词汇表（v113 统一多源图谱）是**死的**

| 声明 | 实测消费 |
|---|---|
| `GraphSourceType` 4 值（`KnowledgeBase`/`Wiki`/`Memory`/`ObsidianVault`），`harness/src/knowledge_graph.rs:12-21` | **定义文件外 0 命中** |
| `GraphNodeType` 4 值（`Entity`/`Note`/`MemoryItem`/`ObsidianNote`），`:51-60`；`as_str()` 提供 `entity`/`note`/`memory_item`/`obsidian_note` | `as_str()` **0 次调用**；`memory_item`/`obsidian_note` **全仓仅出现在定义处** |
| `CreateMultiSourceEntityInput`（`:78-103`） | **仅定义 + trait 签名**，`src-tauri/src` 无任何实现 |
| `knowledge_entities.node_type` 实际写入值 | 9 处写入中 **8 处硬编码 `String::from("entity")`**；仅 `commands/analyst_feedback.rs:83` 透传 `req.node_type` |

⇒ v113「统一知识图谱支持多源节点」这一层，**枚举声明了、列加了、写入端全部绕过枚举写死 `"entity"`**，
`GraphSourceType` 与 `CreateMultiSourceEntityInput` 更是**零消费 / 无实现**。
——**这与我 `domain_ontology.rs` 的病是同一个病**（「声明 ≠ 接线」），只是它比我先得。
说明这不是我个人的失误，是**这个仓库的结构性习惯**：先声明词汇表，不接线。

---

## 4. 领域层今天完全没有词汇表（本体真正该填的洞）

| 字段 | 位置 | 类型 | 值域约束 |
|---|---|---|---|
| `knowledge_entities.entity_type` | `entities/src/knowledge_entities.rs` | `String` | **无**（doc 注释写「如 person / organization / concept」——注释而已） |
| `knowledge_relations.relation_type` | `entities/src/knowledge_relations.rs:12` | `String NOT NULL` | **无**（全仓仅 3 个已知取值：`causes` / `contains` / `follows`，见下） |
| `note_links.link_type` | `entities/src/note_links.rs` | `String NOT NULL` | **无**（写入端写死 `"wikilink"`，`dao/src/repo/note.rs:684,693`） |
| `wiki_pages.page_type` | `entities/src/wiki_pages.rs` | `String` | **无**（由 LLM 自由输出，`agent/src/ingest_pipeline.rs:405` 提示词只说 `One of "entity","concept","source-summary","comparison"`） |

**关系类型的全部已知取值**（穷举实测）：

| 取值 | 出处 | 有 domain/range 吗 |
|---|---|---|
| `causes` | `harness/src/knowledge_graph.rs:112` 唯一被声明的常量 `CAUSAL_RELATION_TYPE` | ❌ |
| `contains` | `dao/src/repo/conversation.rs:716` **字面量** | ❌ |
| `follows` | `dao/src/repo/conversation.rs:735` **字面量** | ❌ |

⇒ 今天可以往 `knowledge_relations` 写入 `(Company)-[causes]->(Note)`、`(Product)-[follows]->(Chokepoint)`，
**没有任何机制会报错**。这就是「有图无本体」：**有实体、有边、有类型字符串，但没有类型系统。**
本体该做的**正是这件事**——而我没做。

---

## 5. 本体与三者的正确关系（层次图）

```
                    ┌──────────────────────────────────────────┐
     领域层 TBox →   │  domain_ontology (本体的正确位置)         │
     （今天不存在）   │  类层次 / 关系+domain-range / 公理        │
                    └───────┬──────────────┬───────────────┬───┘
                            │ 生成/校验     │ 值域约束       │ 生成/校验
                            ▼              ▼               ▼
              ┌──────────────────┐ ┌────────────────┐ ┌──────────────────────┐
      过程层   │ DAG 数据流端口     │ │ 图谱关系表      │ │ WIKI 类型对亲和度      │
              │ WorkflowEdge      │ │ relation_type   │ │ compute_type_affinity │
              │ **今天无类型**     │ │ **今天自由文本** │ │ **今天硬编码 match**   │
              └──────────────────┘ └────────────────┘ └──────────────────────┘
                        │                  │                      │
                        ▼                  ▼                      ▼
              节点/边的执行语义      实体+边的存储         相关性打分 / 惊奇度 / 缺口检测
                                     （ABox 该落这儿）        （推理消费今天在这儿）
```

**三条正确的接法**（对应上图的三个箭头）：

1. **→ WIKI（第 3 套，最容易、收益立刻）**：把 `compute_type_affinity` 的硬编码表**提成公理声明**，
   函数改为查本体。立刻获得：① 本体第一次拥有**真实生产消费方**；
   ② 新增 `PageType` 时**不加亲和度就编译/测试失败**（今天是静默落进 `_ => 0.0`）；
   ③ 「跨类型只允许 Entity↔Concept」从代码注释变成**可查询、可测试的公理**。
   注意 `graph_dtos.rs:158-163` 的 `unwrap_or(PageType::Unknown)` 是 **fail-open**：
   类型解析失败静默变 `Unknown`，然后亲和度落 `_ => 0.3`（同型）/`0.0`（异型）——**这条也得一并治**。

2. **→ 知识图谱（第 4 套洞，最实质）**：给 `entity_type` / `relation_type` 建**受约束值域**，
   按 domain/range 校验入边。今天的 3 个关系值 + `CAUSAL_RELATION_TYPE` 常量应成为本体的一部分。
   ⚠ 这一步要**同时改写入端**（`conversation.rs:716/735` 等字面量），否则又是一次「声明 ≠ 接线」。

3. **→ DAG（第 1 套，最重、需产品决策）**：`WorkflowEdge.source_handle/target_handle` 今天是
   `Option<String>`（`workflow_types.rs:1619-1623`），**端口无类型** ⇒ 引擎无法回答
   「这条边上传的是哪个类的实例」。要让本体约束 DAG，需要给端口加类型标注，
   **这是编辑器 + 引擎 + 模板三方改动**，不是顺手活。

---

## 6. 落点清单与推荐顺序

| 序 | 动作 | 改动面 | 立刻兑现什么 | 风险 |
|---|---|---|---|---|
| **A1** | 亲和度表提成本体公理，`compute_type_affinity` 改查本体 | `harness` 2 文件 + 测试 | **本体首次有真实生产消费**；新增类型漏声明即失败 | 低（打分权重值不变） |
| **A2** | 修 `get_page_type` 的 fail-open：解析失败须上报/计数，不得静默 `Unknown` | `graph_dtos.rs` + 调用点 | 消除静默降级 | 低 |
| **B1** | `relation_type` / `entity_type` 受约束值域 + domain/range 校验 | `harness` + `dao` 写入端 3~5 处 | 图谱**有类型系统**；非法边可拦 | 中（要动写入端，可能有存量非法数据） |
| **B2** | 清理 v113 死词汇表：`GraphSourceType` 0 消费、`CreateMultiSourceEntityInput` 无实现 | `harness` + 决策 | 消除「声明未接线」存量 | 中（需先确认多源图谱还要不要做） |
| **C1** | DAG 端口加类型标注，用本体当端口类型词汇表 | `harness` + `rt-workflow` + 前端编辑器 + 模板 | 数据流可静态校验 | **高**（产品决策 + 存量模板迁移） |

**我的建议：A1 → A2 立刻做（小、且 A1 是本轮唯一能真正把「零消费」变成「有消费」的动作），
B1 作为 O1 的主线，C1 单独立项。** B2 需要你先给一句话：v113 的「统一多源图谱」还要不要做。

---

## 7. 需要你裁决

1. **A1/A2 是否开工？**（我倾向直接开——这是纯内部重构，行为不变，且是现成的正确模板）
2. **v113 多源图谱（`GraphSourceType` / `CreateMultiSourceEntityInput`）还要不要做？**
   - 要做 ⇒ 本体需覆盖 4 个来源类的映射
   - 不做 ⇒ 应删除死声明（否则它永远是我这次犯的同一个错的先例）
3. **C1（DAG 端口类型化）是否立项？** 这是本体能不能真正约束「过程层」的分水岭，但改动面最大。

---

## 8. 本次分析的方法论沉淀（可复用）

排查「本体该放哪」的正确起手式，**不是**读 OWL 资料，而是**枚举仓库里已有的类型系统**：

```
grep -rn "type.*: String\|_type TEXT"  → 找出所有「本该是类型、现在是自由文本」的位置
grep -rn "match (&.*_type, &.*_type)"  → 找出所有「手写的类型对兼容表」= 嵌入式本体
grep -rn "enum .*Type"                 → 找出所有已声明的词汇表
再对每个枚举统计「定义文件外的消费点」    → 立刻分出「活的 / 死的」
```

判据：**如果一个仓库里已经有 `match (type_a, type_b)` 这样的手写表，那它就已经有本体了——
只是埋在函数体里。本体的工作是把那张表提出来并让它可被测试，不是另建一套。**
