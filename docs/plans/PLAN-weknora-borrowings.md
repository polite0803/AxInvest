# PLAN — WeKnora 借鉴清单与落地方案

> 状态：active ｜ 日期：2026-09-17 ｜ 对应版本：v2.9.7


- **日期**：2026-09-14
- **上游审计**：`AUDIT-weknora-vs-axinvest-2026-09-14.md`
- **状态**：**P0 五项已落地代码（W2 / W4 / W5 / W10 / W12，2026-09-15）** —— 但 **W10 实测未生效**（三层缺口，见 **§9**），W9 经复核**判据不成立**（见 **§9**，**未删**）；W1 / W3 / W6–W8 / W11 / W16 待排期。执行明细见 **§8**，更正见 **§9**。
  > ⚠ 验证状态：`cargo fmt --check` 通过、三个门禁脚本通过；`cargo clippy --workspace --all-targets` **曾报 1 处 `error[E0308]`**（`init/services.rs:2013`，我写的 W10 接线，已修，**重跑未完成**）；`cargo test` **尚未跑过**。
  > **在 clippy + test 全绿前不要把本文件读作「已验证」。**
- **参考边界**：WeKnora 为 **MIT**，但清单中所有条目一律按「**用自己的 Rust 栈重新实现**」给出，**不搬运代码**。
- **编号规则**：`W1–W20`（WeKnora 首轮），**编号不重用**。

---

## 0. 优先级总表

| # | 动作 | 性质 | 依据 | 量级 | 依赖 |
|---|---|---|---|---|---|
| **W1** | 分块加 `ContextHeader` 章节面包屑（参与 embedding、不进 Content） | 🟡 借鉴对方**真实现** | AUDIT §1.1-2 / §4-A1 | 小 | 无 |
| **W2** | `HybridConfig` 接线（让权重真的可配） | 🔴 我方自有缺陷（借鉴对方「权重真读取」的作法） | AUDIT §4-A4 | 小 | 无 |
| **W3** | 保护块：表格/代码/公式不被切断 | 🟡 借鉴对方**真实现** | AUDIT §1.1-3 / §4-A3 | 中 | 无 |
| **W4** | 修「文档解析假成功」（OCR 空 → 仍 `ready`） | 🔴 我方自有缺陷（不借鉴，对方更差） | AUDIT §4-A10 | 小 | 无 |
| **W5** | 旧向量删除失败必须补偿/上报 | 🔴 我方自有缺陷 | AUDIT §4-A7 | 小 | 无 |
| **W6** | 父子块（父不进索引 + 命中子回填父） | 🟡 借鉴对方**真实现** | AUDIT §1.1-1 | **大** | 建议 W1 先落地 |
| **W7** | 表格跨块表头继承 | 🟡 借鉴对方真实现 | AUDIT §1.1-3 | 中 | 依赖 W3 |
| **W8** | 检索质量 span 字段（来源拆分/分数/top-N/降级标志） | 🟡 借鉴对方**真实现** | AUDIT §1.1-5 | 中 | 无 |
| **W9** | 清理四个零调用模块（recall_pipeline / vector_cache / search.semantic_cache / incremental_indexer） | 🔴 我方自有缺陷（按铁律 #3「被覆盖 ⇒ 删」） | AUDIT §4-A5 | 小 | 需先确认非预留 API |
| **W10** | GraphRAG 接线（`entity_graph_provider` 恒 `None`） | 🔴 我方自有缺陷 | AUDIT §4-A8 | 中 | 无 |
| **W11** | chunk 编辑加版本历史 | 🟡 借鉴对方真实现 | AUDIT §4-A6 | 中 | 无 |
| **W12** | `record_hits` 写库失败必须上报 | 🔴 我方自有缺陷 | AUDIT §4-A9 | 小 | 无 |
| **W13** | 记忆写入加 pending 确认闸门 | 🟡 借鉴对方**真实现**（高价值） | AUDIT §1.1-6 | 中 | 需确认我方记忆系统现状 |
| **W14** | 记忆双时态 supersede（`invalid_at` + `superseded_by`） | 🟡 借鉴对方真实现 | AUDIT §1.1-7 | 中 | 可与 W13 合并 |
| **W15** | Wiki 增量三态对账（replace / retractStale） | 🟡 借鉴对方真实现 | AUDIT §1.1-9 | **大** | 无 |
| **W16** | 检索加 MMR 多样性 | 🟡 借鉴对方真实现 | AUDIT §3 | 小 | 无 |
| **W17** | 同名默认值单一真源检查 | 🟢 元机制（对方 8 处冲突是活体标本） | AUDIT §5-5/6 | 中 | 无 |
| **W18** | 「假配置」检测（配置项零读取点）加入现有门禁 | 🟢 元机制（对方 W7 病历直接可用） | AUDIT §5-7 | 中 | 无 |
| **W19** | fail-open 出口检查（对方 4 处 fail-open 反向产出） | 🟢 元机制 | AUDIT §5-2/3/4/12 | 中 | 无 |
| **W20** | 检索链路 token 预算审计（我方独有，防回归） | 🟢 元机制 | AUDIT §6-5 | 小 | 无 |

> **性质图例**：🔴 我方自有缺陷｜🟡 借鉴对方｜🟢 元机制｜⛔ 反面教材（见 §「明确不做」）

---

## 1. P0 —— 分块质量与数据正确性（W1–W5）

### W1 🟡 分块加章节面包屑（`ContextHeader`）

**问题**：`crates/search/src/text_chunker.rs` 按 `#`~`######` 切分后**丢弃层级路径**。一个 `## 第三节 / ### 3.2 供应链瓶颈` 下的段落，切出来就是孤立文本，embedding 只看到段落本身，看不到「它属于哪一章」。grep `breadcrumb|context_header|EmbeddingContent` **零命中**。

**借鉴来源**：`chunker/splitter.go:18-49`（**对方真实现，逐字复核通过**）。它的 `Chunk` 结构把 `Content` 与 `ContextHeader` **分开存**，并暴露 `EmbeddingContent()` 在**喂 embedding 时**拼接：

```go
// 关键：Content 是源文档原文切片，ContextHeader 只参与 embedding
func (c Chunk) EmbeddingContent() string {
    body := strings.TrimSpace(c.Content)
    if c.ContextHeader == "" { return body }
    return c.ContextHeader + "\n\n" + body
}
```

它保住的不变式是 `End - Start == runeCount(Content)` —— 因此**高亮、回填、文档重建、按 offset 定位全都对得上**。对方注释明确写了这一点，且该不变式被三处代码共同守卫。

**目标状态**：
1. `TextChunk` 增加 `context_header: Option<String>`（存标题路径，如 `"文档标题 > 第二章 > 2.1 需求"`）；
2. `chunk_by_markdown_headings` 在切分时**维护标题栈**（H1→H6），为每个 chunk 填 `context_header`；
3. 新增 `embedding_content()` 方法：`context_header` + `\n\n` + `content`，**仅 embedding 与关键词检索用**；
4. **写入向量库时**用 `embedding_content()`；**返回给 LLM 时**仍用 `content`（保持可引用性）。

**验证方式**：
- 单测：给定三级标题的 Markdown，断言每个 chunk 的 `context_header` 是完整路径、且 `content` 不含标题；
- 不变式断言：`chunk.end - chunk.start == content.chars().count()`（防回归 —— 这是对方守的最重要的一条）；
- 反事实：同一段落放在不同章节下，embedding 文本必须不同。

**风险**：
- ⚠️ **这是行为变更**：embedding 输入变了 ⇒ **存量向量与新向量语义不一致**，必须重建索引（或至少接受短期混合语义）。需确认重建入口可用。
- ⚠️ 不要顺手把标题写进 `content` —— 那会破坏 offset 不变式（对方踩过的坑，它专门写了注释）。

---

### W2 🔴 `HybridConfig` 接线

**问题**：`crates/harness/src/rag_config.rs:123-141` 定义了 `enabled` / `vector_weight` / `bm25_weight` / `sparse_weight` / `fusion`，挂在 `RAGPipelineConfig.hybrid` 上。但**全项目零读取点** —— `rag.rs:340` 写死 `HybridSearchOptions { top_k, ..Default::default() }`（0.7/0.3/RRF/60），`commands/wiki.rs:734-736` 也写死。用户设为 `false` **不生效且无告警**。

**借鉴来源**：WeKnora 的权重来自租户配置**且真被读取**（`knowledgebase_search_fusion.go:84-86` `retrievalCfg.GetEffectiveRRFK()` / `GetEffectiveRRFWeights()`）。注意它自己也有**假配置**（`RetrievalConfig.VectorThreshold` 主链路不读，AUDIT §5-7）—— **只借鉴"真读取"这一半，不借鉴它的假配置**。

**目标状态**：`HybridSearchOptions` 从 `RAGPipelineConfig.hybrid` 构造，删除两处写死点。`enabled=false` 时走纯向量（或纯关键词）路径并**在结果里标注降级**。

**验证方式**：
- 单测：把 `vector_weight` 改成 0.1/0.9，断言召回排序**确实变化**（当前恒不变）；
- 负向：`enabled=false` 时断言不进入 BM25 分支；
- 反查：`grep -rn "HybridSearchOptions"` 确认无第三处写死点。

**风险**：⚠️ 接线后权重**真的生效** ⇒ 检索结果会变，属**行为变更**。需先确认现有默认值 0.7/0.3 是否就是期望值（若是，则接线只是让配置可改，结果不变；若配置里存的是别的值，结果会变）。

---

### W3 🟡 保护块：表格 / 代码块 / 公式不被切断

**问题**：`text_chunker.rs` 无任何保护块机制（grep `protected|table|fence|code_block|formula` **零命中**）。长表格跨块后**列名丢失**，代码块被从中间截断。`CODE_CHUNK_SIZE=120` 常量（`:11-18`）存在但**无任何引用**。

**借鉴来源**：`chunker/splitter.go:117-126`（7 条正则把 LaTeX 块 / 图片 / 链接 / 表头+分隔行 / 表行 / 代码块 / 行内代码划为不可切区域）+ `splitter.go:308,334-361`（超限保护块的强制切碎兜底，`maxProtectedSize=7500`）。

**目标状态**：`find_break_point`（`text_chunker.rs:301+`）在选择切割点前，先识别保护块区间；切割点**不得落在保护块内部**；单个保护块超过 `chunk_size` 时按行/空格强制切（并记录该块被降级）。

**验证方式**：
- 单测：含 40 行 Markdown 表格的文档，断言**没有** chunk 从表格中间断开；
- 单测：含 ```` ```rust ... ``` ```` 的文档，断言代码块完整落在同一 chunk；
- 边界：超过 `chunk_size` 的巨型代码块走强制切碎路径且不 panic。

**风险**：⚠️ 分块结果改变 ⇒ **需重建索引**。且保护块会**放大块体积**（表格可能撑大 chunk），需检查是否触发 embedding 长度上限 —— 建议实现时同时加 `max_protected_size` 上限（对方是 7500）。

---

### W4 🔴 修「文档解析假成功」

**问题**：`document-parser/src/lib.rs:33,43` OCR 失败时 `unwrap_or_default()` ⇒ 返回**空字符串**；`rag.rs:603-605` 得到 `Ok(vec![])`；`indexing.rs:828-830` 直接 `return Ok(())`；而文档状态在 `indexing.rs:768` 已被置为 **`ready`**。结果：**扫描件解析全挂，用户看到"索引成功"，实际知识库零内容**。

**我方原有**：这是**我方自有缺陷**，WeKnora 同类问题更严重（`image_multimodal.go:338-342` 同样静默）—— **不借鉴，反向修**。

**目标状态**：解析产出为空且原文非空时，状态置 `failed` 并附错误码（或在 `ready` 上增加 `chunk_count == 0` 的显著标记）；OCR 不可用应**明确报"OCR 未安装"**而非返回空串。

**验证方式**：单测 —— 喂一个文本层为空的 PDF + 无 tesseract 环境，断言状态是 `failed` 而非 `ready`；反查 `grep -rn "unwrap_or_default" crates/document-parser/src/lib.rs`。

**风险**：⚠️ 会让此前"看起来成功"的文档**变成失败** —— 用户可见的状态变化，需确认前端对 `failed` 的展示与重试入口就绪。

---

### W5 🔴 旧向量删除失败必须补偿

**问题**：`src/indexing.rs:940` `let _ = vector_store.delete_document_embeddings(...)` —— 删除失败被丢弃。后果：文档编辑后**旧内容永久可被召回**（与 `:936-937` 注释声明的 R1 修复目标直接相悖）。

**目标状态**：删除失败必须（a）上报错误码、（b）置文档为需重建状态，或（c）进入重试队列。三者选一，**不得静默**。

**验证方式**：注入失败（mock 返回 Err），断言不返回成功；集成测试：编辑文档后查向量库，断言旧 chunk 数归零。

**风险**：低。这是纯 bug 修复。

---

## 2. P1 —— 检索质量与接线（W6–W12）

### W6 🟡 父子块

**问题**：无父子结构（`parent_chunk` 零命中）。命中一个 2000 字符的块，给 LLM 的就是这 2000 字符 —— 上下文不足。

**借鉴来源**：`chat_pipeline/merge.go:317-344`（**逐字复核通过**）。设计要点：
- **只有子块进向量索引**（父块不需要 embedding，省索引成本）；
- 命中子块后 **`r.Content = parent_content + "\n\n" + child_content`**，即**用父块全文替换返回内容**，并保留 `SubChunkID` 记录真正命中的子块（供引用定位）；
- 父块与子块**同表用 `chunk_type` 区分**（`parent_text` vs `text`），省一张表；
- 额外洞见：对方把 `ImageInfo` **作用域限定到命中的子块**，避免图片密集的父块把兄弟页的 OCR/Caption 全塞进上下文（`merge.go:326,338`）—— 这条细节比父子块本身更值钱。

**目标状态**：
1. `TextChunk` 增加 `parent_chunk_id: Option<...>`；向量表元数据增加该字段与 `chunk_type`；
2. 分块时先生成父块（如 `chunk_size * 4`），再把父块按现有逻辑切成子块；
3. 索引只写子块；检索命中子块后按 `parent_chunk_id` 取父块内容做**上下文回填**；
4. 回填后的 `content` 与原始命中子块**分开保存**（对方用 `ContentRewritten` 标志），保证引用可定位。

**验证方式**：
- 单测：断言向量库里 `chunk_type='parent'` 的行数为 0（父不进索引）；
- 端到端：构造一个"答案分散在父块不同子块"的查询，断言回填后 LLM 上下文包含完整父块；
- 反事实：`SubChunkID` 必须指向真正命中的子块（用于前端高亮）。

**风险**：
- ⚠️ **大改 + 需迁移 + 需重建索引**（新列 + 双写）。
- ⚠️ 回填会**放大上下文长度** ⇒ 可能挤爆 token 预算。我方有 `apply_rag_token_budget`（对方**没有**），**回填必须走预算裁剪**，否则会退化成"上下文被截断到只剩父块开头"。
- 建议：先做 W1，观察面包屑带来的召回提升是否已足够；父子块作为独立一轮评估。

---

### W7 🟡 表格跨块表头继承

**问题**：表格被切开后，后续 chunk 只有数据行、没有列名，LLM 无法判断每列含义。

**借鉴来源**：`chunker/header_tracker.go:67-141` + `splitter.go:454-496`。在 flush 一个 chunk 时，若下一个 chunk 仍属同一张表，**把表头行重写进下一个 chunk 首部**；并用 `headerAlreadyPresent` / `headerColumnMismatch` 两个守卫**防重复注入与串表**。

**目标状态**：`flush` 时检测「本块以表格数据行结尾 且 下一块以表格数据行开头」⇒ 注入表头行；注入前后做列数一致性校验。

**验证方式**：单测 —— 60 行表格切成 3 块，断言每块首行都是表头且**只出现一次**；异构表相邻时断言不串表头。

**风险**：依赖 W3（保护块）先落地，否则切点不可控。

---

### W8 🟡 检索质量 span 字段

**问题**：我方检索链路的可观测**不足以事后诊断**：命中数、来源拆分、分数分布、是否降级都没有结构化记录。`record_hits` 写库失败还被 `unwrap_or_default()` 吞掉（W12）。

**借鉴来源**：`tracing/langfuse/retrieval_obs.go:25-61`（**对方真实现，本项是它最强的一处**）。`retrieve` span 固化：
- `total_hits` / **`vector_hits`** / **`keyword_hits`**（来源拆分 —— 事后能答"是向量没召到还是关键词没召到"）
- `by_retriever[]`：engine / retriever / count
- `top_hits[]`：rank / chunk_id / score / **match_type** / preview

以及 `rerank.go:297-308`：**同时留 `composite_results`（重排后）与 `final_results`（MMR 后）**，并显式标注 **`threshold_degraded`**。

**目标状态**：我方的检索轨迹（现有 tracing 基建）补齐这组字段；**降级事件必须进 trace 而非只打日志**（对方自己这点没做到 —— `search.go:429-435` embedding 降级只 Warn 不进 span，**不借鉴它的这个偷懒**）。特别要覆盖我方已存在但不可观测的降级路径：`fts_only_fallback`（`rag.rs:425-444`）、`resolve_embedding_provider` 失败、单源检索跳过（`rag.rs:1463-1470`）。

**验证方式**：
- 单测：构造一次 hybrid 检索，断言 trace 里 `vector_hits + keyword_hits == total_hits`；
- 反事实：把 embedding provider 配错，断言 trace 里有 `degraded=true` + `reason`（当前只有一条 `tracing::warn!`）。

**风险**：低。纯增量，不改检索语义。

---

### W9 🔴 清理四个零调用模块

**问题**：`crates/search/src/` 下 `recall_pipeline.rs`（348 行）/ `vector_cache.rs`（216 行）/ `semantic_cache.rs`（1220 行）/ `incremental_indexer.rs`（302 行）**四模块零生产调用**。其中 `semantic_cache.rs`（1220 行）只被死代码 `recall_pipeline.rs` 引用，且与在用的 `src/semantic_cache/mod.rs` **同名不同物** —— 极易误读。

**性质**：🔴 我方自有缺陷。**不借鉴对方** —— 对方同类问题更重（`service/graph.go` 1040 行零调用）。

**目标状态**：按铁律 #3「被覆盖 ⇒ 删」处置。删除前需确认：① 非对外「预留扩展位 API」（`hybrid_search.rs:17-20` 的 sparse 字段是显式声明为扩展位的，**保留**）；② 非测试专用。

**验证方式**：删除前用四路传递闭包确认无消费者 —— ① 源码 `use`（排除自身）② 包管理清单 ③ CI/scripts/配置引用 ④ **正对照**（拿一个已知被依赖的模块验证检法有效）。删除后 `cargo check --workspace --all-targets` + `clippy` 全绿。

**风险**：⚠️ 若其中某个是「已规划但未接线」的功能，删除会丢失实现。**建议先登记为待裁决，而非直接删** —— 尤其 `semantic_cache.rs` 的 1220 行。

---

### W10 🔴 GraphRAG 接线

**问题**：`crates/search/src/rag_pipeline.rs:17,21,150` 定义了 `entity_graph_provider` 与图增强分支；`dao/src/knowledge_graph_provider.rs:81` 有 `EntityGraphProvider` 实现；但**唯一构造点** `rag.rs:2340-2345` 第 4 参**恒传 `None`** ⇒ 图增强检索**永不执行**。

**借鉴来源**：WeKnora 的图检索**接通了**（`chat_pipeline/search_entity.go:106` → `:171` 合入 `SearchResult`，`MatchType: "graph"`），可按它的**接线位置**参考。但它的图查询只有**单跳 + `n.name CONTAINS` 子串匹配** —— **不借鉴其查询实现**（我方本体有 55 种关系类型，理应做得更好）。

**目标状态**：把 `EntityGraphProvider` 实例注入 `RAGPipeline::new`，让图增强分支真的可执行；并明确「图检索结果在融合中的权重」与「是否受 token 预算约束」。

**验证方式**：单测断言 `entity_graph_provider.is_some()`；端到端构造一个需要关联实体才能答的查询，断言召回结果里含 `MatchType::Graph` 的条目。

**风险**：⚠️ **这是行为变更** —— 接线后图检索**真的开始返回结果**，可能改变现有答案。建议加 feature flag 灰度。

---

### W11 🟡 chunk 编辑加版本历史

**问题**：我方**有** chunk 编辑（`commands/knowledge.rs:1057` `update_chunk_content`、`:1097/1298` `update_chunk_embedding`）**但没有版本历史**（grep `chunk_revision|chunk_version` 零命中）—— 改完即覆盖，无法 diff / 回滚。

**借鉴来源**：`repository/wiki_page.go:113-129`（**逐字复核通过**）：把「快照旧版本 + 写入新版本」放进**同一事务**，快照用 `OnConflict(page_id, version) DoNothing` 保证**重试幂等**，更新用 `WHERE id=? AND version=?` **乐观锁**（失败回滚内存版本号）。对方在 chunk 侧也做了同构实现（chunk 编辑 + 版本 + diff + 回滚 + 自动重索引）。

**目标状态**：chunk 内容变更时同事务写快照行；版本列 + 乐观锁；保留策略设上限（对方：软窗口 50 / 硬上限 200）。

**验证方式**：单测 —— 并发两次更新，断言第二次因版本冲突失败而非静默覆盖；回滚后断言产生**新版本**而非覆盖历史。

**风险**：低-中。新增表 + 事务改造。

---

### W12 🔴 `record_hits` 失败必须上报

**问题**：`src/indexing.rs:1202-1203` `record_hits(...).unwrap_or_default()` —— 写库失败 ⇒ 反馈 ID 列表为空 ⇒ **反馈闭环静默断链**。

**目标状态**：失败至少 `tracing::warn!` + 计数指标；不得静默为空。

**验证方式**：mock 失败，断言有可观测信号。

---

## 3. P1/P2 —— 记忆与 Wiki（W13–W16）

### W13 🟡 记忆写入加 pending 确认闸门

**问题**：若我方记忆系统允许 LLM 从对话中自动抽取并**直接生效**，则「模型猜的东西」会进入每轮 prompt，是信任崩塌的高发点。

**借鉴来源**：`memory/service.go:472-485`（**对方完成度最高的一处设计，逐字复核通过**）：

```go
func statusForWrite(item types.MemoryItem) string {
    if item.Inferred && item.Origin != types.MemoryOriginExplicit && item.Origin != types.MemoryOriginManual {
        return types.MemoryStatusPending
    }
    return types.MemoryStatusActive
}
```

关键在**唯一写入函数里分流**：LLM 推断的 → `pending`，**永不注入 prompt**，等用户确认；用户显式说的 → 直接 `active`。配套 `ConfirmItem` / `RejectItem`（拒绝 = 删除 + 写墓碑防复活）。

**目标状态**：把「谁说的」与「系统猜的」在**唯一写入点**分流；pending 态必须有确认入口（后端通道 + 前端可见）。

**验证方式**：单测 —— 构造 `Inferred=true, Origin=llm` 的写入，断言状态为 `pending` 且不进入任何 prompt 组装路径；负向 —— `Origin=user` 则直接 active。

**风险**：⚠️ 需先确认我方记忆系统现状（本轮**未审计**我方记忆写入路径）。若我方当前根本不自动抽取，则本条不适用。

---

### W14 🟡 记忆双时态 supersede

**借鉴来源**：`memory/service.go:317-424`（**真实现**）。要点：
- 用 `NormalizedKey`（CJK 逐字 + 去噪声后缀）做**主题归一化**，冲突消解**不调 LLM**（读路径零模型调用）；
- 旧行**不删除**，保 `invalid_at` + `superseded_by` ⇒ 可回答「上个月信的是什么」；
- 同题同文直接返回旧行（不 churn 时间戳）；
- 时效闭环：`ExpiresAt` 过期归档 + `ArchiveLowestRanked` 容量淘汰 + `TouchUsed` 更新 `last_used_at`。

**目标状态**：记忆表加 `valid_from`/`invalid_at`/`superseded_by`/`expires_at`/`last_used_at`/`use_count`；写入时按主题键 supersede 而非覆盖。

**验证方式**：单测 —— 两次矛盾写入后，断言第一次的读结果为 `invalid` 但仍可查；断言读路径**零 LLM 调用**。

**风险**：中。需确认与我方现有记忆表结构兼容。

---

### W15 🟡 Wiki 增量三态对账

**问题**：若我方 Wiki/知识文档更新时是「追加」语义，会导致**知识重复累积**。

**借鉴来源**：`wiki_ingest_batch.go:1531-1559`（**真实现**）。文档重解析时对新旧 slug 做**三态对账**：
- 旧 slug 仍在新集合 ⇒ 同时发 **retract + add**（把 LLM 的 append 语义纠正为 **replace**）
- 旧 slug 不再出现 ⇒ `retractStale`（剥离引用，可删页）
- summary 页 ⇒ 整体覆盖

**目标状态**：我方 Wiki（`llm_wiki_compile`）或知识文档的重新生成路径引入同构对账，避免重复累积。

**验证方式**：单测 —— 同一文档两次 ingest（内容微调），断言页面的知识块数**不增长**（replace 生效），不再出现的主题被标记 stale。

**风险**：**大**。需先确认我方 `WikiCompiler` 当前的更新语义（本轮**未审计**）。

---

### W16 🟡 检索加 MMR 多样性

**问题**：全 `src-tauri` grep `\bmmr\b` **零命中**。当前召回缺少多样性控制，Top-K 容易被同源近似段落占满。

**借鉴来源**：`chat_pipeline/rerank.go:223` `applyMMR(..., lambda=0.7)`（真实现）。对方在**重排之后、最终截断之前**插入 MMR。

**目标状态**：在 rerank 之后加 MMR 阶段，λ 可配；默认关闭或 λ=0.7（需实测）。

**验证方式**：单测 —— 构造 5 个高度相似的候选，断言 MMR 后的 Top-3 不全是近似段落；断言 λ=1.0 时退化为原排序。

**风险**：⚠️ 会改变 Top-K 构成 ⇒ 属行为变更。建议加配置开关并默认保持现状。

---

## 4. 元机制（W17–W20）—— 护栏，不碰业务逻辑

> 依据 M7：**对方在某维度彻底失败 ⇒ 最高价值** —— 下面三条直接来自它的病历。

### W17 🟢 同名默认值单一真源检查

**对方病历**：chunk size/overlap **8 处定义冲突**（512/80、76、50、160、1536、0、2048/0、1600/160），separators **3 套**；检索参数 5 处冲突（阈值 0.15/0.2/0.5/0.7、TopK 50/30/10）。它自己在 `splitter.go:76-80` 注释里写「single source of truth」，实际有 5 处冲突 —— **自证注释污染**的教科书案例。

**目标状态**：写检查脚本 —— 对一组登记在案的常量名（chunk size / overlap / top_k / threshold / weight），grep 全仓所有字面量定义点，**多于 1 处即报错**并列出位置。

**验证方式**：先用现网代码跑一遍（应能报出我方跨 crate 的重复定义，如 `chunk_size` / `top_k` 的默认值）；再对已知单一真源验证**不误报**（正对照）。

**风险**：低。纯新增脚本。

---

### W18 🟢 「假配置」检测（配置项零读取点）

**对方病历（最有价值的一条）**：`RetrievalConfig.VectorThreshold` 等经 API 可编辑但**主链路不读**；`SearchParams.OnlyRecommended`、`PipelineRequest.VectorDatabase` **全仓零读取点**。用户改了配置，系统假装接受了。

**我方已有同型缺陷**：`HybridConfig`（W2，AUDIT §4-A4）就是同一个病。

**目标状态**：扩展现有门禁（`scripts/check-contracts.mjs`）增加一条检查：**结构体里带 `#[serde]` 的配置字段，若全仓无读取点（排除定义、序列化、测试、Clone/Debug derive），则报告**。

**验证方式**：
- 反查：跑一遍应能命中 `HybridConfig` 全部 5 个字段（当前是坏的）；
- 正对照：拿一个真被读取的字段（如 `rerank.enabled`）断言**不误报**。

**风险**：中。需处理 derive 宏与 `Clone` 的误报，建议先产出**报告模式**（不阻断 CI），人工确认后再收紧。

---

### W19 🟢 fail-open 出口检查

**对方病历（4 处 fail-open）**：
- rerank API 失败 ⇒ 原序进下游（`rerank.go:130-144`）
- VLM OCR+Caption 全空 ⇒ `return nil` 视为成功（`image_multimodal.go:338-342`）
- 分块三层全被拒 ⇒ 仍返回 legacy 结果，**且日志是 Debug 级**（`strategy.go:43-57`）
- **DLQ 写入失败 ⇒ 仍删队列行 ⇒ 任务永久消失**（`wiki_ingest.go:1244-1250`）

**我方同型缺陷**：`fts_only_fallback` 静默降级、单源检索 `Err` 只 warn 后跳过、`let _ =` 满链路（AUDIT §4）。

**目标状态**：把「降级/吞错」的检法固化成规则 —— 对检索/索引链路的关键路径，**`let _ =` 与 `Err(_) =>` 分支必须带 `// SAFETY:` 或明确的理由注释**，否则 lint 报告。可直接复用 `AGENTS.md` 已有的 `#[allow(clippy::disallowed_types)] + // SAFETY:` 模板形态。

**验证方式**：先跑报告模式，统计现网数量；与已知的 `let _ =` 清单（AUDIT §4 / 我方侦察兵异常吞没清单）比对是否覆盖。

**风险**：低（仅报告）。

---

### W20 🟢 检索链路 token 预算审计（防回归）

**我方优势项**：`apply_rag_token_budget`（`agent/mod.rs:2727`）**WeKnora 完全没有**（grep 零命中）。

**风险点**：W6（父子块回填）会**显著放大上下文长度**，若不经预算裁剪，会把「上下文不足」换成「上下文被截断」。

**目标状态**：为 RAG 注入链路加一条测试 —— 断言注入 prompt 的检索内容**总长度不超过预算**；在 W6 落地后**必须重跑**。

**验证方式**：单测 + 反事实（构造超大父块，断言预算裁剪生效）。

---

## 5. 明确不做（防范围蔓延）

| # | 不做什么 | 原因 |
|---|---|---|
| ⛔ 1 | **不照抄 10 个检索后端抽象** | 我方只需 SQLite sqlite-vec / PG pgvector 两后端；对方的 10 后端里有 2 个（infinity / elasticfaiss）是**零 driver 的空壳枚举** |
| ⛔ 2 | **不引入 gRPC sidecar 文档解析服务** | 对方 `docreader` 是为了多语言解析（Python 生态）；我方是桌面单进程，引入 sidecar 会破坏打包与离线可用性。**只借鉴它的"按图像覆盖率判定扫描页"思路** |
| ⛔ 3 | **不加 `tenant_id` 列 / 不做多租户隔离** | 我方是单用户桌面应用，无租户概念 |
| ⛔ 4 | **不引入 Neo4j 或任何外部图数据库** | 对方图谱默认关（`NEO4J_ENABLE != "true"` ⇒ nil），且它的内存图谱 1040 行是死代码；我方 Louvain + 关系表**已强于它** |
| ⛔ 5 | **不照抄 `wiki_boost`** | 它自己**断链**（`ChunkTypeWikiPage` 零写入点，boost 恒空转）。我方 `RAGSourceType::Wiki` 已接通 —— **防的是退化，不是借鉴** |
| ⛔ 6 | **不照抄它的 env-gated 测试习惯** | 对方 15+ 处测试靠环境变量静默跳过 ⇒ CI 中等于不存在。我方新增测试**不得**用「缺环境变量就 return」的形态 |
| ⛔ 7 | **不照抄 `only-new-issues` 式 lint 门禁** | 对方 golangci 只查新增代码，存量违规永远不受约束 ⇒ 我方门禁应全量 |
| ⛔ 8 | **不做 `query_knowledge_graph` 式"可视化外壳"** | 对方该工具的 `edges` 初始化后**从不填充**，返回空图却报成功 —— 典型的"有 UI 没数据" |

---

## 6. 执行前置条件（需用户决策）

| 事项 | 类型 | 说明 |
|---|---|---|
| **重建索引** | ⚠️ 行为变更 | W1（embedding 输入变）/ W3（分块结果变）/ W6（新列）均需重建向量索引。**必须先确认重建入口可用且耗时可接受** |
| **W2 接线后结果会变** | ⚠️ 行为变更 | 需确认 `HybridConfig` 里存的值是否等于当前写死的 0.7/0.3 |
| **W4 会让文档变 failed** | ⚠️ 用户可见状态变化 | 需确认前端对 `failed` 的展示 + 重试入口就绪 |
| **W9 删除 4 个模块** | ⚠️ 不可逆 | 其中 `semantic_cache.rs` 1220 行 —— **建议先登记待裁决，不直接删** |
| **W10 图检索开始返回结果** | ⚠️ 行为变更 | 建议 feature flag 灰度 |
| **W13/W15 前提未验证** | ❓ 信息缺口 | 本轮**未审计我方记忆写入路径与 WikiCompiler 更新语义** —— 需先补一次定向审计再定方案 |
| **W1/W3/W6 顺序** | 🔧 建议 | 建议 **W1 →（观察效果）→ W3 → W6**，而非并行，便于归因 |

---

## 7. 一句话总结

> **先修自己的接线（W2/W4/W5/W10/W12），再借鉴它的分块工程质量（W1/W3/W6/W7）与可观测（W8）**；
> 记忆赛道的 W13/W14 与 Wiki 的 W15 价值高但**前提未验证**，需补一次我方定向审计；
> W17–W19 三条护栏直接来自对方病历，成本低、防复发，建议**与业务改动并行推进**。

---

## 8. 执行记录（2026-09-15）

### 已落地

| # | 动作 | 关键改动 | 验证 |
|---|---|---|---|
| **W2** | `HybridConfig` 接线 + **加权 RRF** | 新增唯一装配入口 `hybrid_options_from_config`（`hybrid_search.rs`）；`search_with_filter` / `search` / `collect_rag_context*` 逐层透传 `Option<&HybridConfig>`；`RAGPipeline` 持 `hybrid` 字段；`indexing.rs::load_hybrid_config(db)` 从全局设置读取；删除 `rag.rs:340` 与 `commands/wiki.rs:733` 两处写死 | 单测 `test_hybrid_options_from_config_reads_user_weights`（含未知 fusion 回退 RRF）+ `test_weighted_rrf_weights_change_ranking`（**要求名次反转** —— 权重未被读取时必失败） |
| **W4** | 文档解析假成功（三层） | ① `ocr_fallback` 的三种失败（未安装 / 调用失败 / 线程退出）由 `Ok("")` 改 `Err(原因)`；② PDF 分支「文本层空 + OCR 空」由 `Ok("")` 改 `Err`，并区分「OCR 不可用」与「确实没文字」；③ `prepare_chunks` 的 `ParseAndChunk` 空解析结果由 `Ok(vec![])` 改 `Err` | ⚠ 未加自动化测试（需跨 crate 注入 `DocumentParser`，见下「未做」） |
| **W5** | 向量删除失败不再静默（**8 处**） | 删除型命令（`memory.rs` / `knowledge.rs` / `wiki.rs`）⇒ 删向量失败**即返回错误并保留 DB 记录**；重索引路径（`indexing.rs`）⇒ 上报使该笔记按索引失败处理；`capability_indexer_impl::remove_index` ⇒ 汇总失败后返回 `Err`（此前恒 `Ok`）；移除 `indexing.rs` 批量循环里与 `index_wiki_note` **重复**的清理点 | 门禁 `check-contracts` 段 H 棘轮：静默丢弃 `let _ =` **284 → 278**，基线已同步下调 |
| **W10** | GraphRAG 接线（**默认关闭**） | 新增 `search::entity_graph` 注入点（与 `sources::set_sources` 同构）；`rag.rs:2340` 第 4 参改取注入值；wiring（`init/services.rs`）按 `ragPipelineConfig.entityGraph.enabled` 决定是否注入 | 未注入 ⇒ `None` ⇒ 行为与接线前一致（单测锁定「未注入返回 `None`」契约） |
| **W12** | `record_hits` 失败可观测 | `repo/retrieval_hit.rs`：逐条 `warn!` 改为**汇总 `error!`（失败数 / 总数）**；`src-tauri/src/commands/conversations/streaming.rs:1196` 的 `unwrap_or_default()` 改显式 `match`（不吞错，也不中断对话流） | — |

### 执行中发现的额外事实（已补入 `AUDIT-weknora-vs-axinvest-2026-09-14.md` §4.1）

- **加权 RRF 是 W2 的必要组成**：默认 `fusion=rrf` 下原 `merge_results_rrf` 只收 `rrf_k`，两路权重**完全不参与** ⇒ 只接线不改融合算法的话，配置依然「看起来生效、实际不生效」。
- **A7 的范围是 8 处而非 1 处**，且含**删除命令** —— 会让已删除的记忆 / 文档继续被召回。
- **`remove_index` 恒返回 `Ok`** ⇒ 4 个调用点的 `if let Err(e)` 是死代码。
- **`rag.rs` 的 L2 阈值恒不生效已实证**（返回分是 RRF 量纲 ≈1.0，阈值语义是 L2 距离 20.0）—— 登记待裁决，不在本轮擅改。

### 未做（明确理由）

| # | 项 | 理由 |
|---|---|---|
| **W9** | 删除 4 个零调用模块 | **不可逆**（含 `semantic_cache.rs` 1220 行）；按 §6 前置条件属用户裁决项，本轮只登记 |
| **W1 / W3** | 分块面包屑 / 保护块 | 属**行为变更**且需重建向量索引；建议按 §6 的顺序（W1 → 观察 → W3 → W6）单独排期 |
| **W10 的启用** | 让图检索真正返回结果 | 图检索结果会进入注入 prompt 的上下文 ⇒ 可见行为变更；需先确认知识图谱内确有可用实体 / 关系数据 |
| 补自动化测试 | W4「空解析 ⇒ Err」、W5「删除失败 ⇒ 不返回成功」 | 前者需向 search crate 注入 `DocumentParser`（`sources::parser()` 未注入即 panic，现有测试全部绕开 `ParseAndChunk`）；后者需替换具体类型 `VectorStore`（非 trait，无法 mock）。两者都需先做测试基建，本轮不引入 |

---

## 9. 第二轮复核更正（2026-09-15）—— **W9 判据不成立，W10 未生效**

§8 交付后回到源码逐条复核 W9 / W10，**两处定性与事实不符**。原判保留不删（见 §0 各行），更正如下。

### 9.1 W9 🟥 —— 「被覆盖 ⇒ 删」对四个模块**全部不成立**

PLAN 的依据是「铁律 #3 被覆盖 ⇒ 删」，而其自列前置条件是「**需先确认非预留 API**」——**该前提从未被验证**。实测：

| 模块 | 行 | 实测结论 | 判据 |
|---|---|---|---|
| `recall_pipeline.rs` | 348 | 代码检索的**查询侧编排**（L1 `FileIndex` → L2 `AstIndex` → L3 向量）。上游两索引由 `src/indexing_triggers.rs` 在生产填充（`cloud_workspace.rs` 云同步后触发），**但两索引的全部查询方法**（`search_by_path`/`search_functions`/`search_classes`/`find_callers`/`search_all`/`all_entries`）**全仓零调用** | 整条「代码检索」**只写不读**；且 `indexing_triggers` 建的是 blocking task 里的 **in-memory SQLite，写完即丢**。**无物覆盖** |
| `incremental_indexer.rs` | 302 | 上述两索引的**增量维护**（notify 监听） | 生产只有 `scan_directory` 全量扫描，**零增量更新路径**。**无物覆盖** |
| `vector_cache.rs` | 216 | 检索**结果级**缓存（TTL+LRU，键 = collection + embedding + top_k） | `crates/cache` 仅有 `EmbeddingCache`(key→Vec<f32>) 与 `TextHashCache`(key→String) 两个**精确键**缓存，**无结果级缓存**；且 `crates/disk-cache/src/lib.rs:6` 与 `cache/cache_snapshot.rs:6` 的文档**已把 `VectorSearchCache` 当作存在的 L1 同伴来写** ⇒ 位置空缺。**无物覆盖** |
| `semantic_cache.rs` | 1220 | 在用版 `src/semantic_cache/mod.rs`（488 行）是**LLM 响应的 SHA-256 哈希缓存**，自述第 7 行「Future: embedding-based cosine similarity search」 | 这份正是该「Future」的**已实现版**，且**缓存层次不同**（RAG 检索结果 vs LLM 响应） ⇒ **不是重复实现，无物覆盖** |

**顺带暴露三条独立的「注释/名单撒谎」**（无论 W9 怎么处置都应修）：

1. `runtime/src/mode_selector.rs` 的 `SPEED_MODE_ESSENTIAL` / `SPEED_MODE_DISABLED` **零消费点**（两条常量无人读），却宣称极速模式能按模块开关，并把 `recall_pipeline` / `code_engine` / `lsp_client` 列为「必备」
2. `crates/disk-cache/src/lib.rs:6` + `cache/cache_snapshot.rs:6` 把 `VectorSearchCache` 写成已存在的 L1 缓存（该名字在 `cache_snapshot.rs` 里**只出现在文档注释**）
3. 索引查询 API 全零调用 ⇒ 「代码检索」只写不读

**处置建议（二选一，不再是「删」）**：
- **(a) 接线**：把 `recall_pipeline` 接到查询侧（前置：索引需从 in-memory 改为可跨请求复用）、`incremental_indexer` 接文件监听、`vector_cache` 接检索结果缓存 —— 实为 **3 个独立特性**，不是清理工作；
- **(b) 登记为「未完成特性」**：代码层标注保留原因与缺口，并修掉上述三条撒谎注释/名单。

### 9.2 W10 🟥 —— 定性层次错了：**接线 ≠ 生效**，共三层缺口

| 层 | 缺口 | 证据 |
|---|---|---|
| ① 配置键**自造** | 我按 `ragPipelineConfig.entityGraph.enabled` 决定是否注入，但该 key **全仓只存在于我新写的代码里** | `RAGPipelineConfig`（`harness/src/rag_config.rs:197`）只有 `query_enhancement` / `rerank` / `self_rag` / `hybrid`；前端 TS 类型、设置 UI、`config/*.yaml` **均无**。⇒ **是一个永远没人能打开的开关** |
| ② **出口缺失** | 阶段 4 把结果写入 `PipelineOutput.graph_context`，**全仓零读取** | `rag_pipeline.rs:192` 赋值；唯一消费点 `rag.rs:2402-2418` 只读 `output.results` / `output.quality` ⇒ **结果被丢** |
| ③ **id 契约未验** | 阶段 4 传 `knowledge_base_id = src_ref.container_id`（形如 `kb_xxx` / `mem_xxx`），`search_entities` 按 `knowledge_entities.kb_id` 过滤 | `search_entities_with_filter` 文档自述「当 `kb_id` 为空字符串时**不按知识库过滤（搜索全部）**」⇒ 两个 id 域确实不同，**不同域即恒返回空** |

**并且图检索已有一条活路径（Path A）**：`rag.rs::collect_knowledge_graph_context`（`:679`）走 `sources::knowledge().search_entities`（`sources` 注入点是通的）→ 格式化为 `[Knowledge Graph - <kb_id>]` → 进 `context_parts` → **进 prompt**；由 `collect_cross_source_graph_context`（`:713`）消费，`:722` / `:1513` / `:2498` 均在使用。

⇒ **「W10 启用」须先裁决 Path A / Path B**：
- **Path A**（现状，已活）：只给**实体清单**，无 relations / neighbors；
- **Path B**（`graph_enhanced_search`，更丰富：实体 + 关系 + 邻居 + `context_text`）：**出口缺失**，补完才能生效；若不做去重，会与 Path A **重复注入**同一批实体。

### 9.3 本轮我自己的两处错误（自证，登记以免重犯）

1. **把「正在编译」当成「编译通过」** —— 我曾据「`tauri dev` 的 rustc 在编 `axagent_lib`」宣称 dev profile 编译全过，实为**推断**；随后 clippy 报 `error[E0308]`（`init/services.rs:2013`，W10 接线：`s.rag_pipeline_config` 是 `serde_json::Value` 而非 `Option`，`.and_then` 类型不匹配）⇒ **用户那次构建很可能就是失败在这行**。**判据：进程在跑 ≠ 成功；须有退出码或产物。**
2. **自造配置键**（见 9.2 ①）—— 属「可配置 ⇒ 四条独立链（schema / UI 入口 / 读取点 / 生效点）」缺三条的形态。

### 9.4 待裁决（本轮不改代码，等确认）

| # | 事项 | 选项 |
|---|---|---|
| 1 | **W9 处置** | (a) 接线做成完整特性（3 个独立特性）／(b) 登记未完成 + 修撒谎注释 |
| 2 | **W10 路径** | Path A 保持现状／Path B 补出口（并决定是否与 A 去重）／两者都做 |
| 3 | **W10 开关** | 补 `RAGPipelineConfig.entity_graph` 类型化字段 + 前端 UI 开关（需 11 语言 i18n），或**默认开启不设开关** |

---

## 10. 第三轮：W10 落地（2026-09-15）—— 出口已补，**但根因是 DTO 字段名分裂**

§9.4 三项裁决结果：**W9 → 全部接线**；**W10 路径 → 补 Path B 出口**；**W10 开关 → 补类型字段 + UI 开关**。本节记录 W10 的执行与**执行中发现的更深根因**。

### 10.1 🔴 最深一层根因：`RerankConfig` 的 wire 名违反禁区 13 ⇒ **整个 RAG 面板的设置全部静默失效**

这不是 W10 独有的问题，而是「W10 怎么接都不会生效」的**真正原因**。链条：

| 环节 | 事实 |
|---|---|
| 后端声明 | `harness/src/rag_config.rs:22` `RerankConfig`：`#[serde(rename = "type")] pub backend: String` ⇒ **wire 名是 `type`** |
| 唯一产出方 | 前端（`settingsStore.ts` 的 `ragPipelineConfig.rerank` + `KnowledgeBaseDocuments.tsx::persistRagConfig`）写的是 **`backend`**；`config/` 与后端**均无任何产出方写 `type`**（已 grep 全仓确认） |
| 后果 | `serde_json::from_value::<RAGPipelineConfig>` 因「缺必填字段 `type`」**恒失败** ⇒ `indexing.rs::load_rag_pipeline_config` 的 `unwrap_or_default()` **静默吞掉** ⇒ **`queryEnhancement` / `rerank` / `selfRag` / `hybrid` / `entityGraph` 五组设置全部回落 Rust 默认值** |
| 外显 | RAG 设置面板里所有开关「点了没用」，且**无任何日志**指向原因。`entityGraph.enabled` 同样恒为 `false` ⇒ **W10 的开关永远不可能打开** |

**同类第二处（独立缺陷）**：`indexing.rs` 用 **snake_case** 键手抠 JSON（`.get("query_enhancement")` / `.get("self_rag")`），而落盘 JSON 是 camelCase（`queryEnhancement` / `selfRag`）⇒ 这两组的「是否启用」判据**恒为 `false`**；`rerank` 恰好两命名同形，所以只有它一直是对的 —— 这正是该 bug 长期潜伏的原因（看到打开 rerank 后管线动了，就以为另外两个也对）。

**归正依据**：AGENTS.md 禁令 **禁区 13「DTO 字段命名（全站统一 camelCase）」** —— Rust 字段保持 snake_case，靠 `rename_all = "camelCase"` 输出 camelCase，**禁止**用 `rename` 造出与字段名无关的 wire 名；TS 侧一律 camelCase。故裁定 **后端与解析方为违规方**，改后端，不改前端。

| 修复 | 位置 | 内容 |
|---|---|---|
| ① 归正 wire 名 | `rag_config.rs:37` | 删除 `#[serde(rename = "type")]` ⇒ wire 名回到 `backend`（= 字段名的 camelCase）。不保留 `alias`：已确认无任何产出方写 `type`，留别名只会制造「一名多义」 |
| ② 键名改 camelCase | `indexing.rs` 5 处 | `.get("query_enhancement")` → `.get("queryEnhancement")`（4 处）、`.get("self_rag")` → `.get("selfRag")`（3 处），并就地写明为何必须 camelCase |
| ③ 解析失败必须出声 | `indexing.rs:1157` | `load_rag_pipeline_config` 由 `.ok()` 静默链改为显式 `match`：读设置失败 / 反序列化失败均打 **warn**（含错误信息 + 原始 JSON 前 512 字节，按字符边界安全截断）。`Value::Null`（= 用户从未保存过 RAG 设置，`AppSettings::default()` 的形态）仍静默回落，不误报 |
| ④ 缺字段不再整份失效 | `rag_config.rs:22/77`、`types/rag_voice_etc.rs::EnhancementConfig` | 三处加**结构体级** `#[serde(default)]` ⇒ 任一字段缺失时回落该结构体 `Default` 的同名字段值。**必须用结构体级**：逐字段 `#[serde(default)]` 取的是**字段类型**的 `Default`（`usize`→0、`String`→""），会把 `topN` / `maxVariants` 变成 0，反而更坏 |
| ⑤ 前端写全 DTO 形状 | `KnowledgeBaseDocuments.tsx::persistRagConfig` | 展开既有配置在前、UI 拥有的字段在后、**必填字段补齐**（缺省值对齐 `settingsStore.ts`）。三件事缺一不可：展开在后 ⇒ 缺省值覆盖用户原值；UI 字段前后各写一遍 ⇒ TS1117 重复键；不补齐 ⇒ TS 类型编译不过且后端解析可能失败 |
| ⑥ 删幽灵字段 | `types/knowledge.ts` `RerankConfig`、`settingsStore.ts` | 删 `ollamaEndpoint` —— 后端 `RerankConfig` **无**此字段（写入被 serde 当未知键忽略，不起任何作用）；`SelfRagConfig.ollamaEndpoint` 保留（后端确有） |

### 10.2 契约测试（**这是之前缺失的那层守卫**）

`rag_config.rs::tests`（该文件原先**没有任何测试**）：

| 用例 | 钉住的契约 |
|---|---|
| `frontend_shaped_config_must_parse` | 前端默认值真源（`settingsStore.ts` 的 `ragPipelineConfig`）形状**必须**能被 Rust 解析，且**逐组断言值真的被读到**（`rerank.backend == "rule"`、`top_n == 5`、`hybrid.vector_weight == 0.7`、`entityGraph.enabled == true` …）。**只断言 `is_ok()` 不够** —— 字段名分叉时也可能被结构体级 default 兜住而「解析成功但全是默认值」，那种假阳性正是本用例要拦的 |
| `partial_config_falls_back_per_field_instead_of_failing` | 部分字段缺失 ⇒ 按字段回落结构体默认值，**不得**整份失败 |
| `null_config_falls_back_to_all_defaults` | `Value::Null` 必须 `Err` ⇒ 调用方必须 `unwrap_or_default()` 兜底，不得 `unwrap` |

### 10.3 Path B 出口补齐（`search/src/rag.rs`）

**根因坐实**：`RagContextResult.graph_context` 的 5 个构造点**全部恒写 `graph_context: None`**（含两条活路径的返回处），且 `PipelineOutput.graph_context` 的消费点数为 **0** —— 即「能力跑通了但没有出口」。「补出口」的落点**不是**把这个字段填上（它自身也零读取），而是把图检索文本**折进 `context_parts`**（= 真正进 prompt 的那一元）。

| 改动 | 位置 | 说明 |
|---|---|---|
| 新增取数 helper | `rag.rs:814` `search_entity_graph` | legacy 路径专用：直接取已注入的 provider 做一次图检索。**只对 Knowledge 源调用**（`graph_enhanced_search` 按 `knowledge_entities.kb_id` 过滤，Memory/Wiki 的 `container_id` 不同域 ⇒ 必然空结果 + 噪声 warn） |
| 新增折叠 helper | `rag.rs:868` `fold_entity_graph_context` | 两条路径共用的出口转换：滤掉空白 `context_text`、命中打 info、**「开关已开 + 确有 Knowledge 源 + 零命中」打 warn**（这是 9.2 ③ id 域不匹配 / 图谱未构建的唯一现场线索） |
| legacy 路径 | `rag.rs:1454` 累积 → `:1616` 折进 `extra_context` → `:1628` 填字段 | **不依赖向量命中**：向量没命中时图谱仍可能按实体名匹配到东西（pipeline 路径拿不到这个机会，见下） |
| pipeline 路径 | `rag.rs:2438` 累积 → `:2554` 捕获 `output.graph_context` → `:2654` 折进 → `:2665` 填字段 | 阶段 4 的产出此前在此被整包丢弃 |
| provider 收窄 | `rag.rs:2517` | 从「对每个源、每个增强查询都传」收窄为 **`qi == 0` 且 `source_type == Knowledge`**：多查询增强下避免 N× 重复图查询（与 self-RAG 只跑首查询同一策略）；非 Knowledge 源不传（同上 id 域理由） |
| fuse 分支补出口 | `rag.rs` `fuse_rag_context_results` | 该函数按融合后的 `source_results` **重算** `context_parts` ⇒ 逐 run 已折叠的回链上下文（**Path A 的 `kg_context` 也一并**）会被丢掉。改为保留首个非空 `graph_context`，并在调用点补回一次（补一次天然去重） |
| legacy 路径新增成本 | — | 仅当开关打开（provider 已注入）时才有一次图查询；关闭时 `search_entity_graph` 立即 `return None`，零开销 |

**覆盖确认**：`collect_rag_context_with_pipeline_from_refs` 内 4 个 `return`（`queries.len()>1` 的 fuse 分支、单查询委托 legacy 分支、`sources.is_empty()` 早退、函数末尾）与 `collect_rag_context_from_refs` 的 2 个 `return` 逐一核对，均已覆盖。

### 10.4 W10 开关（四条链全部补齐）

「可配置」需四条独立链同时存在，缺一条即为假开关。§9.2 ① 只补了「读取点」，本轮补齐：

| 链 | 落点 |
|---|---|
| ① schema 字段 | `RAGPipelineConfig.entity_graph: EntityGraphConfig { enabled }`（`rag_config.rs`，camelCase wire 名 `entityGraph`） |
| ② UI 入口 | `KnowledgeBaseDocuments.tsx` 的 Graph-RAG `Switch`（`id="knowledge-settings-switch-entity-graph"`） |
| ③ 读取点 | `indexing.rs::load_rag_pipeline_config`（唯一类型化解析入口）→ `init/services.rs` 按 `entity_graph.enabled` 决定是否注入 provider |
| ④ 生效点 | `rag.rs` 两条路径的图检索 + `fold_entity_graph_context` 折进 `context_parts` |
| i18n | `settings.rag.entityGraph.title` / `.desc` ×**11 语言**（全部校验可解析、非空、`dprint check` 通过、`check-i18n-untranslated` EXIT=0） |

**⚠ 生效时机**：provider 走 `OnceLock` 注入（与 `sources::set_sources` / `tools::parser::set_parser` 同构，符合本仓既有约定），**运行期不可变更** ⇒ **开关改动需重启应用**。此点已写入 11 语言的 `.desc` 文案，避免用户误判为「点了没用」。

### 10.5 顺带修正的其它缺陷

| 缺陷 | 位置 | 说明 |
|---|---|---|
| `is_entity_graph_enabled()` 零生产调用点 | `entity_graph.rs` | 原仅供「启动日志与自检」，实际无人调用。现由 `fold_entity_graph_context` 的零命中告警消费 ⇒ 有真实消费端 |
| **恒真式测试**（等于没有测试） | `entity_graph.rs::tests` | 原用例为 `if !is_entity_graph_enabled() { assert!(entity_graph_provider().is_none()) }` —— 条件与断言是同一件事（`is_none()` ⟺ `!is_some()` ⟺ `!enabled()`），**永远不可能失败**。换成有区分力的注入契约用例；并另加 `fold_entity_graph_context` 的纯函数单测（空白过滤 / 透出），该函数在真机上只有开着图谱开关才走得到，靠人工跑很难发现它往 prompt 注入空段落 |

### 10.6 验证矩阵

| 门禁 | 结果 |
|---|---|
| `cargo check -p axagent-search`（touch 强制失效后） | ✅ `Checking axagent-search` + `Finished`，**0 error / 0 warning**（首次跑因「日志与改动时间窗重叠」按铁律作废，重跑取证） |
| `tsc --noEmit` | ✅ EXIT=0（**首跑即报错**：我上一轮在 `persistRagConfig` 里写出 TS1117 重复键 —— 上轮只跑了 i18n 校验、**没跑 tsc**，如实登记） |
| `check-i18n-untranslated.mjs` | ✅ EXIT=0（未翻译 0 / 新增 0） |
| `check-hardcoded-i18n.sh --strict` | ✅ EXIT=0（Rule 1/2/3 + Strict 全 PASS） |
| `dprint check`（11 语言包 + 3 个改动的 TS 文件） | ✅ EXIT=0 |
| `check-contracts.mjs` | ✅ PASS（errors=0 warnings=6，与基线一致） |
| `check-errorcode-alignment.mjs` | ✅ 413 码 ↔ 11 语言对齐，无孤儿码 |
| `check-rust-raw-map-err.mjs` | ✅ 无裸 `map_err` |
| `check-layer-discipline.mjs` | ❌ EXIT=1 —— **与本轮无关**：新越界 3 处全在 `commands/stock_workflow/{sim_hook,rhai_pm}.rs`，两文件最后修改于 **09-14**（本轮改动在 09-15 09:0x）⇒ 属用户自己在 `stock_workflow` 的进行中工作，未擅改，仅登记 |
| `cargo test -p axagent-harness` | ✅ **EXIT=0**：`684 passed; 0 failed`（**修前 683 passed / 1 failed**）—— 含 §10.8 修好的引用腐烂断言 + 我新加的 3 条 `rag_config` 契约测试；doc-tests `42 passed` |
| `cargo test --workspace --no-fail-fast`（第 1 次） | ❌ **`EXIT≠0`，且一条测试都没跑** —— 不是测试失败，是 `axagent-rt-workflow` 的 **test target 编译失败**（`E0308 ×2`），cargo 直接中止 ⇒ `--no-fail-fast` 对**编译错误无效**（它只管测试失败，不管编译失败）。**整份日志 0 个 `test result:` 行**，即 30+ crate 的测试本轮**仍全部未执行** |
| `cargo test --workspace --no-fail-fast --exclude axagent-rt-workflow`（第 2 次） | ⏸ **主动撤销**（运行 1m50s、无产出）。原因：与**并行会话**抢同一 `target` 锁 —— 实测 5 对 cargo 在排队（全是对方 agent 发起），而我上一次 `--workspace` 跑已把对方 10:56 的请求锁到 12:19；再由我持锁 30+ 分钟会直接卡住对方 6 项待办的编译验证 ⇒ **让锁**。留待对方收工后补跑 |
| `cargo clippy -p axagent-harness --all-targets -- -D warnings`（**补跑**） | ✅ **`CLIPPY_EXIT=0`**，`Finished dev profile in 20m32s`，**0 error / 0 warning**（实测 20m38s，含 ~11 分钟排队等锁）。**为什么必须补**：clippy 第 4 跑（09:28）之后我还改过 `harness/src/knowledge_graph.rs` 的测试代码（给腐烂断言加「自报当前真值」），**clippy 未覆盖该次改动** ⇒ 按「修完必全量重跑」不得省。补跑后 4 跑结论对**当前源码树**成立 |

#### 为何「30+ crate 从未跑过」会反复发生（两型叠加）

| 跑 | 中止机制 | 后果 |
|---|---|---|
| 第 1 次（fail-fast） | **测试失败** → 停在第 1 个二进制 | 只跑了 `axagent_lib` |
| 第 2 次（`--no-fail-fast`） | **编译失败** → 整个 run 中止 | 跑了 **0** 个二进制 |

⇒ **判据：只要日志里没有 `test result:` 行，就只能说「没跑」，不能说「通过」**。这是本会话第三次踩「假绿/假红同源」：**测量工具本身没产出，却被当成了结果**。

#### clippy 四跑记录（**每跑都暴露了上一跑看不见的东西**）

| 跑 | 结果 | 说明 |
|---|---|---|
| 1 | ❌ **环境故障** | `拒绝访问 (os error 5)` 写 `axagent-market-sim` 的 fingerprint。按技能分流：`error[E...]`/`could not compile` 计数 = **0** ⇒ 非代码问题 |
| 2 | ❌ **真 lint** | 环境故障清除后现出 `clippy::manual_option_as_slice`（`rag.rs:2392`，我写的 `.as_ref().map_or(&[], std::slice::from_ref)`）⇒ 改 `Option::as_slice()` |
| 3 | ❌ **真 lint ×3** | `clippy::useless_conversion`（`String::from(...)` 包一个已返回 `String` 的函数）：`commands/knowledge.rs:508`、`memory.rs:329`、`wiki.rs:373`、`Err(String::from(...))` 一处 —— **全部来自上一轮 W5 的改动**，上轮同样没跑 clippy ⇒ 如实登记为遗留 |
| 4 | ✅ **`CLIPPY_EXIT=0`** | `Finished dev profile in 8m09s`，**0 error / 0 warning**。定性依据：`ErrorResponse::from_error` 返回 `Self` ⇒ `String::from(...)` 是**必要**转换（全仓数百处，均不告警）；`err_with_detail` 返回 `String` ⇒ `String::from(...)` 多余（仅这 3 处，全在上一轮 W5 改动的文件里）|

⚠ **每次都必须全量重跑**：clippy 遇首个 error 即短路。第 3 跑日志能佐证「`axagent` 是最后一个被检查的 crate」（`Checking axagent` 出现在末尾）⇒ 其余成员已全过；但 `--all-targets` 下 bin / integration test 目标可能未被 lint，只有 EXIT=0 才算数。

**环境故障处置**（按 `cargo-win-lock-troubleshoot` 技能）：清 `target/debug/` 下 `.cargo-build-lock` / `.cargo-lock` / `.cargo-artifact-lock`（三个都留在不可访问状态）+ **只改名出错的那一个** fingerprint 目录（25 个 `axagent-market-sim-*` 里只坏了 1 个，保留其余 24 个省编译）。该「靶向而非全量」的改进已回写技能。


### 10.7 登记待裁决（本轮**不擅改**）

| # | 事项 | 为什么需要裁决 |
|---|---|---|
| 1 | `use_pipeline` 的判据语义 | ✅ **已修 → §12.2**：统一到类型化配置（`load_rag_pipeline_config`）。原「原始 JSON 是否 true」的判据与 `RerankConfig::default().enabled == true` 在 `ragPipelineConfig == Value::Null` 时**互斥** |
| 2 | `crossEncoderModel` 默认值两处不一致 | ✅ **已修 → §12.3**：找到权威产物流（下载清单）⇒ 前端默认值缺 `.Q4_K_M.gguf` 后缀（指向永不存在的文件）。新增 `RERANKER_MODEL_FILENAME` 常量 + 棘轮测试 |
| 3 | `vector_cache` / `recall_pipeline` / `incremental_indexer` | 按用户裁决「全部接线」，属 **3 个独立特性**（前置：索引需从「blocking task 内 in-memory SQLite，写完即丢」改为可跨请求复用）；§9.1 三条说谎注释**已按实测改正 → §12.5**（且原描述机制有误，见该节） |
| 4 | `effective_threshold` 空洞过滤 | ✅ **已修 → §12.1**：不是「统一量纲」这么轻 —— 实测为**用户可见故障**。四条融合路径三个量纲，前端默认值 `0.1` 在默认 RRF 路径下把结果**全滤掉** |
| 5 | W1 / W3 / W7、W6、W8 / W11 / W16 | 见 §0 与 §8「未做」：W1/W3/W7 均改 chunk 产物 ⇒ 需重建向量索引，按 W1 → 观察 → W3 → W7 顺序单独排期 |
| 6 | `rt-workflow` test target 编译失败（`E0308` ×2） | **不在本轮范围**（mtime 10:55 > 我的 clippy 通过 09:28），属用户进行中工作 ⇒ 未擅改。修法已备：两处 `&ctx` → `&ctx.variables`。**在你确认前，`cargo test --workspace` 永远编译不过**。详见 §10.9 |


### 10.8 顺带修复：`knowledge.rs` 行号引用整体漂移（**本轮改动造成**，非遗留）

`cargo test -p axagent-harness` 报 1 例失败，**归因是我本轮 W5 修复的副作用**，如实登记：

| 项 | 内容 |
|---|---|
| 症状 | `knowledge_graph.rs:1111` 断言红：`evidence 指向的行不讲 rtype ⇒ 引用腐烂。第 1738 行实为：lines += 1;` |
| 根因 | 本轮把 `commands/knowledge.rs:503-512` 的单行 `let _ = state.vector_store.delete_document_embeddings(..)`（**吞错**）改写成显式错误传播（3 行注释 + `map_err` 展开）⇒ **净增 7 行** ⇒ 该文件后段**所有**行号 +7 |
| 波及面 | `harness/src/knowledge_graph.rs` 有 **8 处**按行号引用 `commands/knowledge.rs`（4 个语义位置）⇒ **同时**腐烂 |
| 指纹 | 4 处偏移量**完全相同**（都是 +7）—— 单一来源、单一事件的强证据（逐处独立核对行内容确认） |
| 修法 | `output/fix-evidence-linenos.mjs` 定点替换，带三重自检（替换次数需与 grep 实测一致 / 打印前后对照 / 行数必须不变）：`1782→1789`×3、`1767→1774`×1、`1798→1805`×2、`1738→1745`×2 |
| 复跑 | `check-decl-evidence.mjs`：**内容存疑 3 → 1**（`in_industry` / `has_concept` 转 ✔，`DATA_DRIVEN_COLUMN` 转白名单 `·`）；剩 1 条 `[EdgeType] mapping → wiki.rs:1132` 属 **既有且刻意**（合成边本就不带 `relationType`，注释即证据）⇒ 非本次引入，不动 |
| 加固 | ① `test_csv_import_relations_are_declared` 失败信息**自报当前真值**（现场扫 `let rtype = fields[2]` 的行号并打进 panic 消息）⇒ 下次腐烂零心智成本；② 该测试 doc 记录本表**第三次**漂移（`1725 → 1738 → 1745`） |
| 全仓核查 | 按行号引用 `commands/knowledge.rs` 的**只有** `harness/src/knowledge_graph.rs`；`dao/src/repo/knowledge_graph.rs:179` 与 `analysis-engine/src/knowledge_loader.rs:3` 是**无行号的纯文件名**引用 ⇒ 天然抗漂移，无需处理 |

**教训（已回写 `rust-consistency-audit` 技能，新增「型⑥ 我自己的改动使既有行号引用整体腐烂」）**：
改**被引用的**源文件前，先 `grep -rn "<文件名>:[0-9]"` 问「谁按行号引用我」；
软判据（脚本 EXIT=3）**CI 容忍 ⇒ 腐烂会照样合入**，必须靠单测硬拦。


### 10.9 阻塞门禁但**非本轮**的缺陷：`rt-workflow` test target 编译失败（用户进行中工作，**未擅改**）

`cargo test --workspace` 首次执行在第 1 个编译单元就中止，**不是测试失败，是编译失败**：

| 项 | 内容 |
|---|---|
| 报错 | `E0308` ×2：`crates/rt-workflow/src/work_engine/executors/condition_executor.rs:472`、`:482` |
| 实际形态 | 两行调 `super::super::resolve_var_path(path, &ctx)`，但被调者是 `executors/mod.rs:106` 的**宽松版** `resolve_var_path(path, &HashMap<String, Value>)`；`ctx` 是 `ExecutionState` ⇒ 类型不符 |
| 对照 | 同测试模块的**前 3 条**（`:446` / `:453` / `:461`）调的是**本地严格版** `resolve_var_path(path, &ExecutionState)`（`:396`），故编译通过 —— 说明新写的测试想同时覆盖两个变体，只是顺手把 `&ctx` 也传给了宽松版 |
| **归属（决定性）** | `condition_executor.rs` mtime = **09-15 10:55**；我的 clippy 第 4 跑（`EXIT=0` / 0 warning）日志 mtime = **09-15 09:28** ⇒ 该错误**产生于我的门禁通过之后 1.5 小时**，且落在 `lc-conceive`（OPC 文学创作者 persona）这条**用户正在编辑的线**上（同一批还有 `opc_workflow_kpi_hook.rs`、`opc_workflows/industry_pack.rs`、`sim_hook.rs`）⇒ **非本轮引入** |
| 最小修法 | 两处的 `&ctx` → **`&ctx.variables`**（已验证：① 类型匹配 `&HashMap<String, Value>`；② 断言仍成立 —— `"lc-conceive.content.persona"` 走 `auto_parse` 穿透 JSON 字符串 content 得 `Some("X")`，`"lc-conceive.persona"` 导航失败后 fallback 平键直查 `variables.get("lc-conceive.persona")` = `None`） |
| 本轮处置 | **不擅自改**（该文件属用户进行中工作，且有未保存缓冲区风险）⇒ 改用 `--exclude axagent-rt-workflow` 单独跑本轮门禁，并在最终答复中**单列告知** |
| ⚠ 连带影响 | 该错误让 **`--no-fail-fast` 失效**（它只对测试失败生效，对编译失败无效）⇒ 整份日志 **0 个 `test result:` 行**，`--exclude` 才是正解 |


### 11. 收口后遗留清单（2026-09-15 13:2x 复盘）

#### 11.1 阻塞类（不解决则门禁永远不完整）

| # | 事项 | 归属 | 状态 / 出路 |
|---|---|---|---|
| 1 | `rt-workflow` test target `E0308` ×2 | **你（§10.9）** | 两处 `&ctx` → `&ctx.variables`。**在修之前 `cargo test --workspace` 必编译不过** |
| 2 | `cargo test --workspace` **从未执行到任何一条测试** | 环境 | ① 首次 fail-fast 只跑第 1 个二进制；② 加了 `--no-fail-fast` 后因上述编译错误整个 run 中止。⇒ 待①修好 + 你 cargo 队列清空后补跑（**30+ crate 的测试本会话全程为零证据**） |
| 3 | ~~**`MEMORY.md` 注入层到顶**~~ | 我（记忆治理） | ✅ **已修 → §12.6**：6832 / 7168 B（95%）→ **5950 B（83%，余量 1218 B）**。压缩手法：① 把 11 项待裁决清单**外迁**到 `DETAIL §59`（该节本就叫「从注入层外迁」）；② 纪律 5–8 / 11 与 13 组判据表去冗余；③ 新增 W9/W10 状态行。**外迁前逐条核实目标节确实存在**（否则等于删除） |

#### 11.2 本轮新发现、但本轮不具备修复条件

| # | 事项 | 事实（已 grep 取证） | 建议 |
|---|---|---|---|
| 4 | ~~`RagContextResult.graph_context` = 写 4 处 / 读 0 处~~ | ✅ **已修 → §12.4**：取「删字段」路线（判据 #183：写入 N 处、读取 M=0 ⇒ 只有删字段/补消费端两条路；前端消费端已实证不存在）。原私有传递改用 `search_entity_graph_for_sources` 显式重查，且**顺手修掉一处语义漂移** |
| 5 | ~~`entityGraph` 开关改完必须重启应用~~ | ✅ **已修 → §12.4**：`OnceLock` → `RwLock<Option<Arc<..>>>`（`RwLock::new` 是 `const` ⇒ 仍可当 `static`）；新增幂等 `sync_entity_graph_provider(db)`，启动装配与 `save_settings` **共用同一函数**（同文件既有 telemetry / 沙箱策略的热应用先例）；11 语言文案从「需重启」改为「保存后即时生效」 |
| 6 | 你的 OPC 在改代码引入 **2 个 `dead_code`** | `constant NODE_TOLERANCE is never used` + `field industry_id is never read`，落在 `axagent (lib)`/`(lib test)`；涉及文件 mtime **11:50 / 12:04**，晚于我的 clippy 通过（09:28）⇒ **非本轮** | 会让 `clippy -D warnings` 门禁红。**未擅改，仅登记** |

#### 11.3 明确未做（已在 §0 / §8 / §10.7 登记过，非遗漏）

- **W9「全部接线」**：`vector_cache` / `recall_pipeline` / `incremental_indexer` 三个独立特性 —— 前置是索引要从「blocking task 内 `rusqlite::Connection::open_in_memory()`，写完即丢」改成可跨请求复用。**§9.1 的三条说谎注释已修（→ §12.5），但三个特性本身仍未接线**
- ~~**§10.7 #1 / #2 / #4**~~：✅ 全部已修 → §12.1 / §12.2 / §12.3
- **§10.7 #5**：W1 / W3 / W6 / W7 / W8 / W11 / W16（W1/W3/W7 改 chunk 产物 ⇒ 需重建向量索引，按 W1 → 观察 → W3 → W7 排期）
- `check-layer-discipline.mjs` 3 处越界（`commands/stock_workflow/{sim_hook,rhai_pm}.rs`，你的进行中工作）
- **§12.7 登记的 1 项新发现**：`RerankPipeline::execute` 把阶段分数丢弃（`rerank_score` 实为融合分），需裁决分数标尺后再改

#### 11.4 已闭环（本轮真正修掉的）

DTO wire 名分裂（**最深根因**，§10.1）· Path B 出口三路径补齐（legacy / pipeline / fuse，§10.3）· `knowledge.rs` 行号漂移 8 处（§10.8）· `TS1117` · `clippy::useless_conversion` ×3 · 幽灵字段 `RerankConfig.ollamaEndpoint` · 恒真测试改可证伪 · `RagContextResult.context_parts` 出口（图检索文本**真正进 prompt** 的那一元）

**加上本轮 §12 的 9 项**：死字段 `graph_context` 删除 · `entityGraph` 开关热更新 · `use_pipeline` 判据统一 · reranker 文件名单一真源 · **阈值量纲统一（含一处用户可见故障）** · `MEMORY.md` 注入层 95%→83% · 三条「类型存在 ≠ 已接线」的撒谎注释 · 源类型字符串三份手写副本收敛到 `RAGSourceType::as_str`（**并修掉它自带的 `E0507` 签名错**） · `page_type.rs` 4 处行号引用腐烂修复（§12.10）


## 12. 第三轮（2026-09-15 13:2x–14:xx）：§10.7#4 量纲统一 + §11 遗留收口

### 12.1 §10.7#4 —— **不是「量纲不统一」这么轻，是一处用户可见故障**

#### 12.1.1 实测：四条融合路径，三个互不可比的量纲

`HybridSearchResult.combined_score` 在全仓被当作**同一个数**使用（`min_score` 过滤、rerank 候选、`retrieval_threshold` 过滤、记忆 tier 加权、前端排序）。但实测四条产出路径给出三个标尺：

| 路径 | 原 `combined_score` | 值域实测 |
|---|---|---|
| 向量-only（`finish_vector_only`）| `1 - L2` | 可负，无上界语义 |
| 加权（`merge_results_weighted`）| `nv*vw + nb*bw` | [0,1]，但 `nv` 以**本次结果集最大距离**归一（相对） |
| **RRF（默认）**（`merge_results_rrf`）| `Σ w/(k+rank+1)` | **≈0.017–0.033**（k=60，w=0.7/0.3） |
| 纯 FTS（`fts_only_search_with_filter`）| 原始 BM25 | 无上界 |

而 `rag::search_with_filter` 的映射是 `score = 1 - combined_score`（该行**本身没错**，是输入错），5 个过滤点又都按「L2 距离 ≤ 20」判断 ⇒

- **默认 RRF 路径下 `score ∈ [0.967, 0.983]`**：`score <= 20.0` **恒真** ⇒ 过滤是空的（这正是「空洞过滤」）；
- 而前端设置面板把 `retrievalThreshold` 默认为 **`0.1`**（`KnowledgeBaseDocuments.tsx:959/1055`、`MemorySettings.tsx:1133/1308` 的 `?? 0.1`，**实测 4 处**），保存时写进库 ⇒ 若某站点把它**直接当距离上限**用，则 `0.967 <= 0.1` **恒假** ⇒ 该知识库对 RAG 一条上下文都不贡献。

> **证据分级（2026-09-15 复核后如实降级，勿读成「已实测」）**
>
> - **可证（已测并锁进测试）**：旧值域 = 0.967–0.983。`test_pre_fix_rrf_scale_was_unusable_by_any_threshold`
>   钉住的正是这一点：**在这个值域下，「≤ 20.0 型上限」恒真、「≤ 0.1 型上限」恒假** —— 两种上限**都不能用**。
> - **不能复证**：修前那 5 个站点**各自**写的是哪个表达式。**无 git（铁律）、无 `output/backup-2026-09-15/`**
>   ⇒ 原文不可取回；上面第 2 条（「恒假 ⇒ 零上下文」）依赖「存在站点直接拿阈值当上限」这一前提。
> - **支持该前提的证据**：① 修复时留下的行内注释（`commands/knowledge.rs:549-552`，当时旧码在眼前）明写
>   「前端默认值 `0.1` 又让过滤恒假（一条都留不下）」；② `commands/paper.rs` 那份本地副本是 **`2.0_f32`**
>   而**不是** `20.0`，且注释自称「与 collect_rag_context 一致」⇒ **站点间的常量本来就各不相同**，
>   「有的站点用阈值本身、有的用 20.0」在形态上是成立的。
> - **反面记录（必须保留）**：`.workbuddy/memory/2026-09-15.md:65`（**修复之前**那一轮）把它记成
>   「RRF 分 ≈0.98 恒小于阈值 20.0」⇒ **只看到「空洞过滤」**。两条记录都对同一值域成立，差别在**表达式形态**。
> - ⇒ **诚实结论**：P0 的**用户可见形态**（「打开过知识库设置并保存」的用户检索静默失效）**依赖站点表达式**，
>   我**不能**声称逐站点已取证；**能**声称的是「旧值域使两种上限都失效，其中「阈值直用」那一型表现为零返回，
>   且已有两处旁证」。修法（统一换算 + 互补性测试 + 两侧都锁）对**两种形态同时成立**，故不因该不确定性而改变。

#### 12.1.2 修法：统一到「相关度 ∈ [0,1]」，**只统一标尺、不翻转方向**

1. **归一（只缩放、不改排序）** —— `hybrid_search.rs`
   - 向量派生一律走 `l2_distance_to_similarity`：`1 - clamp(L2 / DEFAULT_MAX_L2_DISTANCE, 0, 1)`（**绝对**标尺，保住「L2 ≥ 20 就是无关」的原始意图；与 `rag_pipeline` 里既有的 `1 - (score/20).min(1)` 同口径）
   - 排名 / BM25 派生走 `normalize_by_max`（**相对**标尺 —— RRF 本身不含绝对质量信号，这一点无法伪造）。加权路径的向量侧从「本次最大距离」改为上述绝对标尺，修掉「同一 L2 值在不同融合算法下算出不同 `vector_score`」
2. **两个投影、一个解释** —— 用户配置的 `retrieval_threshold` 统一读作**相关度下限 ∈ [0,1]**：
   - `similarity_floor_from_threshold(cfg) -> f32`（原形；供消费端 `score` 是**相关度**的场合）
   - `distance_ceiling_from_similarity_floor(cfg) -> f32 = 1 - 上式`（供消费端 `score` 是**距离**的场合）
   - 为什么按「下限」：(a) 前端默认值 `0.1` 只有在「下限」语义下才宽松无害；(b) UI 只有一个 0–1 数字框、标签「检索阈值」，无单位提示；(c) 读成「距离上限」会让 `0.1` = 「只留 L2 ≤ 0.1 的近乎重复项」⇒ 结果集恒空。
   - ⚠ **混用两个投影的后果是反向筛选**（留下的恰好是最不相关的那批），不是报错 —— 故两者都加了文档互引 + 互补性测试。
3. **5 个过滤点全部改走共享函数**，删掉 4 份 `20.0` 与 1 份 `2.0` 的本地副本。其中 `commands/paper.rs` 原为 `2.0_f32` 且注释写「与 collect_rag_context 一致」（并不一致）—— `2.0` 正是 `rag.rs` 里**已被修掉的坏值**（该文件测试注释：「修复前为 2.0，过滤了几乎所有结果」）⇒ 那次修复漏了这份副本。

#### 12.1.3 顺带修掉的 4 个真 bug（同一区域，均非「量纲」问题）

| # | 位置 | 缺陷 | 性质 |
|---|---|---|---|
| 1 | `rag.rs` pipeline 路径 | `let (source_top_k, _threshold, dims) = { let (sk, _, d) = …; (…, sk, d) }` —— **丢弃真实阈值 `th`**，又把 `sk`（`top_k` 计数，`usize`）填进 `f32` 阈值槽位。因槽位被 `_threshold` 绑定且从未使用，**其类型无人约束 ⇒ `usize` 照样编译通过**，「阈值被丢弃」零编译期信号 | 类型混淆 + 丢弃 |
| 2 | `hybrid_search.rs` 混合分支 | `.take(top_k)` **在 `sort_by` 之前**，而输入来自 `score_map.into_values()`（HashMap，**顺序不定**）⇒ 随机留下 top_k 个。另两条收尾路径都是「先排序后截断」，只有这里反了 | 排序/截断次序 |
| 3 | `fts_only_search_with_filter` | `.filter(\|r\| options.min_score.is_some_and(…))` —— `min_score == None` 时谓词**恒假** ⇒ **无过滤条件时反而把所有结果丢掉**，纯 FTS 降级路径恒返回空。`finish_vector_only` 用的是 `is_none_or`，混合分支是显式 `if let Some` ⇒ 三份收尾逻辑不一致 | 三份实现不一致 |
| 4 | `rag.rs` pipeline 路径 | `RagRetrievedItem.score` 被写入 `RerankedChunk.score`（= `rerank_score` = **相关度、越大越相关**），而 legacy 路径写的是 `VectorSearchResult.score`（**距离、越小越相关**）⇒ **同一字段两个反向标尺**。pipeline 是**默认**路径（`RerankConfig::default().enabled == true`）⇒ 该路径下 `apply_memory_tier_weight` 的 `-= tier_bonus` 与升序排序**全部反向生效**（层级/重要度越高的记忆越靠后） | 同一字段方向相反 |

#### 12.1.4 回归锁（新增 5 条，全部可证伪）

- `test_similarity_floor_converts_to_distance_ceiling`：`0.1 ⇒ 0.9`；未设/负/NaN/∞ ⇒ 不过滤；**两个投影必须互补**（改一个忘另一个 = 反向筛选）
- `test_threshold_filter_is_neither_hollow_nor_total`：阈值 `0.1` 下**既不得清空、也必须真的丢掉最差一档**（两侧同时锁）
- `test_pre_fix_rrf_scale_was_unusable_by_any_threshold`：把归一前值域喂给旧表达式，证明 `<= 20.0` 恒真 **且** `<= 0.1` 恒假
- `test_rrf_scores_are_normalized_into_unit_range` / `test_vector_only_finish_normalizes_and_filters` / `test_top_k_keeps_the_most_relevant_not_an_arbitrary_subset`：归一值域 + `min_score` 生效 + 截断留给最相关者
- `test_normalize_by_max_is_nan_free` / `test_l2_similarity_is_absolute_and_clamped`：除零与饱和点夹取
- **重写了一条说谎的旧测试**：`test_default_l2_threshold_is_reasonable` 原本**自己声明** `let default_max_distance = 20.0;` 再断言它 ≥10 —— 即断言自己的字面量，改生产常量它**不会红**（等于没有测试）。改为直接钉住生产常量本体

#### 12.1.5 ⚠ 相对标尺的固有代价（如实登记，别当没这回事）

RRF / BM25 派生分数归一到 [0,1] 是**按本次结果集最大值**归一的**相对**标尺 ⇒ 它**不带绝对质量信号**：知识库里一条相关内容都没有时，最差的那条仍会得 `1.0`。故「相关度下限」在 RRF 路径上是「相对最好的一半」，不是「绝对够好」。这是 RRF 的固有性质（排名融合本就不含原始分数量纲），**不是本次引入的**，但此前因为过滤器是空的所以从未暴露。若需要绝对质量门，须改用 `FusionAlgorithm::Weighted`（向量侧已是绝对标尺）—— 该取舍已写入 `DEFAULT_MAX_L2_DISTANCE` 的文档。


### 12.2 §10.7#1 —— `use_pipeline` 判据统一（含一个默认行为变更）

`indexing.rs::collect_rag_context` / `collect_rag_context_with_sources` 原读**原始 JSON 是否存在且为 true**（`unwrap_or(false)`），与 `RerankConfig::default().enabled == true` 在 `ragPipelineConfig == Value::Null` 时**互斥**。现统一走类型化配置（`load_rag_pipeline_config`）：

```rust
let pipeline_config = load_rag_pipeline_config(db).await;
let use_pipeline = pipeline_config.query_enhancement.enabled
    || pipeline_config.rerank.enabled
    || pipeline_config.self_rag.enabled;
```

**⚠ 行为变更（净效果）**：全新安装（无 `ragPipelineConfig`）现在**走 pipeline 路径**（受 RuleReranker + self-RAG 质检），此前走 legacy 路径。理由：前端默认值真源 `settingsStore.ts` 与后端 `RerankConfig::default()` **本来就一致地写着「rerank 开启」**，两处人写的默认值长期互斥才是缺陷；以类型化配置为准则「UI 上显示的」与「实际跑的行为」一致。同时删掉了 pipeline 分支里第二次 `serde_json::from_value`（同一配置解析两遍、两处判据）。


### 12.3 §10.7#2 —— reranker 文件名单一真源

**先找权威产物**：`model_downloader.rs:74` 下载的是 `bge-reranker-v2-m3.Q4_K_M.gguf`，`reranker.rs` 的兜底也是它 ⇒ **「文件必须存在」这一硬约束在后端**，前端默认值 `"bge-reranker-v2-m3"`（**缺 `.Q4_K_M.gguf` 后缀**）指向一个永不存在的文件。

| 落点 | 改动 |
|---|---|
| `harness::rag_config` | 新增顶层 `pub const RERANKER_MODEL_FILENAME`；`RerankConfig::default().cross_encoder_model` 改为取它 |
| `search::model_downloader::preset_models` | 清单文件名改为取同一常量（此前是第 3 份字面量） |
| `search::reranker` | 两处 `unwrap_or_else(‖ "….gguf")` 兜底改为取常量 |
| 前端 `settingsStore.ts` / `KnowledgeBaseDocuments.tsx` | 补上 `.Q4_K_M.gguf` 后缀（共 2 处） |
| **棘轮测试** | `reranker_preset_matches_rerank_config_default`：下载清单里的 reranker 文件名 **必须 ==** `RerankConfig::default().cross_encoder_model`。失败形态是**运行期加载模型失败**（不是编译错误），故必须由测试钉住 |


### 12.4 §11.2#4/#5 —— 死字段删除 + 开关热更新

| 项 | 取证 | 处置 |
|---|---|---|
| `RagContextResult.graph_context` | 写入 7 处（`indexing.rs:1355/1503`、`rag.rs:1442/1628/2399/2424/2663`）、读取 **0** 处；TypeScript 侧 grep `graphContext\|graph_context` 于 `src/` ⇒ **No matches found** | **删字段**（判据 #183：M=0 ⇒ 只有「删」或「补消费端」两条路；前端消费端已实证不存在）。唯一的同模块私有传递改用新 helper `search_entity_graph_for_sources` 按 `effective_query` 显式重查 —— 顺带修掉一处语义漂移：旧的 `find_map` 取的是**增强后** `fq` 的结果，而 Path A 用 `effective_query` |
| `entityGraph` 需重启 | `set_entity_graph_provider` 为 `OnceLock` ⇒ 运行期不可变更；UI 文案已写「重启后生效」 | `OnceLock` → `RwLock<Option<Arc<dyn EntityGraphProvider>>>`（`RwLock::new` 是 `const` ⇒ 仍可作 `static`；临界区只有一次赋值 ⇒ 中毒时 `into_inner()` 容忍安全）；新增 `clear_entity_graph_provider`（**关开关必须显式清除**，不能「什么都不做」）；新增幂等 `indexing::sync_entity_graph_provider(db)`，**启动装配与 `save_settings` 共用**（`save_settings` 里已有 telemetry / 沙箱策略同款「保存后立即生效」先例） |
| 测试 | 原 `set_provider_turns_enabled_on` 首条断言是**恒真式**（`!is_enabled()` ⇒ `provider().is_none()` 是同一件事） | 删掉恒真首断言；加 `static TEST_LOCK`（进程级单例必须串行，否则并行测试互踩出假红）；新增 `provider_can_be_cleared_and_reinjected`（注入→清除→**重新注入**三段，锁住热更新路径） |
| i18n | 11 语言 `.desc` 原写「需重启应用后生效」 | 改为「保存后即时生效」（`output/patch-i18n-entitygraph-hotreload.mjs`，幂等，`changed=11 skipped=0 failed=0 EXIT=0`）<br>**⚠ 改文案前必须证明新文案为真**（否则等于把「准确的坏消息」换成「好听的假话」）。本项的证据链已逐跳核过：① `static ENTITY_GRAPH_PROVIDER: RwLock<Option<Arc<dyn EntityGraphProvider>>> = RwLock::new(None);`（`RwLock::new` 是 `const` ⇒ 仍可作 `static`）② `clear_entity_graph_provider`（`entity_graph.rs:58`）③ `sync_entity_graph_provider(db)` 的**两个真实调用点**：`commands/settings.rs:111`（保存时）与 `init/services.rs:2014`（启动时）④ **两条检索路径都在每次调用时重读全局** —— Path A 走 `is_entity_graph_enabled()`/`entity_graph_provider()`（`rag.rs:866`/`834`）；Path B 的 `graph_provider` 在循环内取 `crate::entity_graph::entity_graph_provider()`（`rag.rs:2577`）且 `RAGPipeline::new`（`:2581`）**每次调用都重建** ⇒ 不存在构造期快照残留。<br>⇒ 结论：两条路径均**下次检索生效**，无需重启 ✅ |


### 12.5 §9.1 三条「撒谎注释」——**PLAN 原描述机制有误，按实测改正**

原描述：「`SPEED_MODE_ESSENTIAL`/`SPEED_MODE_DISABLED` **零消费点**」。实测**不成立**：两者各被 `enter_speed_mode`（`mode_selector.rs:125/131`）真实读取。真正的事实更重：

| 项 | 实测 | 改法 |
|---|---|---|
| `mode_selector.rs` | `ModeSelector::new` 全仓唯一调用点在本文件 `mod tests`（`:226`）；`ModuleRegistry` 生产实例化点 **0**；无 init 装配 / 无 Tauri command / 无 UI 入口 ⇒ **整个双模式特性无生产入口**（「常量有人读」≠「特性可用」） | 模块文档加「⚠ 接线状态」，把 `one-click toggle` 标注为**设计意图而非当前行为**，并列出三条实证 |
| `crates/disk-cache/src/lib.rs:6` | 把 `VectorSearchCache` 与 `EmbeddingCache` 并列成「已存在的 L1 缓存」。实际该**类型存在**（`search/src/vector_cache.rs:16`，含 TTL+LRU 与单测），但 `VectorSearchCache::new` 全仓仅出现在**它自己的测试**里 ⇒ **从未被实例化到生产路径** | 文档改为「已接线的只有 EmbeddingCache / TextHashCache」，并点明「类型存在 ≠ 已接线」 |
| `cache/cache_snapshot.rs:6` + 字段 `:28` | 同一处错误；且 `vector_search_cache` 字段是**真 schema 字段**，`CacheSnapshot::new` 只写 `Vec::new()` ⇒ **恒为空**，且 `cache_persister` 每次关机把它写盘 | 保留字段（未完成特性占位，删除会丢线索；且删字段可、加字段需 `serde(default)`）但标注「当前恒为空」与原因 |

**教训（已回写 `rust-consistency-audit`）**：判「某常量/字段是死的」时，**先查它被谁读、再查那个读取者是否可达** —— 两级都要。只查第一级会得出「有人消费」的假阴性，只查第二级会得出「零消费点」的错误机制描述；两者都不等于「特性可用」。


### 12.6 §11.1#3 —— `MEMORY.md` 注入层 95% → 83%

6832 / 7168 B → **5950 B（余量 336 B → 1218 B）**。**外迁前逐条核实目标节存在**（否则「外迁」等于删除）：`DETAIL §59`（该节标题即「待裁决清单（11 项）—— 从注入层外迁」）、`§36.1/§36.2/§36.5/§58.5`，以及 RULES `#7/#72/#143/#147/#149/#151/#152/#163/#164/#174/#175/#179/#183/#184` 均已存在。手法：① 11 项待裁决**整体外迁**；② 纪律 5–8/11 tersify；③ 13 组判据表去粗体、括号改反引号；④ 新增 W9/W10 状态行。另把 RULES `#72` 从「三型假阳性」升级为**五型**（补「日志 mtime 早于改动即作废」与「lint-only 规则 `check`/`test` 都不跑」）。


### 12.7 本轮新发现、**未擅改**（需你裁决）

| # | 事项 | 事实 | 为什么需要你 |
|---|---|---|---|
| 1 | `RerankPipeline::execute` **丢弃阶段分数** | `reranker.rs:460-464` 按阶段返回的 `score_map` 排序，但**从不写回 `r.combined_score`**；`:479` 的 `rerank_score: r.combined_score` 因此是**融合分**而非重排分 ⇒ 字段名撒谎（`RerankConfig` 的云端/交叉编码器相关性分数被丢弃，只用了它的**排序**）。同理 `original_score` 与 `rerank_score` 恒等 | 修法是「把阶段分数写回」，但需先定**每个 rerank 后端（rule / cross-encoder / 云端 API / LLM）分数标尺**并统一到 [0,1]，否则写回后会污染 `top_n`/`rule_filter_keep` 的语义。属**同一类量纲问题**，但动的是重排链而非检索链，建议单独一轮 |
| 2 | 阈值过滤的**位置**（先截断后过滤） | 5 个过滤点都是 `search → truncate(top_k) → filter`，故阈值可能让返回数少于 `top_k`（即使截断线以下还有合格项）。`HybridSearchOptions.min_score`（消费端在四条路径内部）是更正确的位置，其文档也写着「需要收窄时由调用方另行设置」—— 但**全仓无人设置** | 改为「先过滤后截断」需给 `rag::search_with_filter` / `rag_pipeline::execute_with_filter` 各加一个入参（后者已有 12 参），且会改掉「未设阈值时按 top_k 截断」的既有语义面。收益是检索质量、代价是签名churn 与一次全链路复测 |
| 3 | 🔴 **W9 前置已查实，且比「未接线」更重：索引管线报成功、产物被丢** | **调用链是真的**：`commands/cloud_workspace.rs:191`（sync 后）与 `:232`（push 后）各调 `trigger_post_sync_indexing_for_cloud_workspace(cache_dir)`，返回值**被消费**并打印 `"Post-sync/Push indexing complete: {} files, {} AST nodes"`。<br>**但索引是函数局部变量**：`index_workspace_blocking` 内 `rusqlite::Connection::open_in_memory()` 建 `FileIndex`（`:49`）与 `AstIndex`（`:81`），函数返回即 drop ⇒ **索引结果不可达**；`AppState` 里 `grep file_index\|ast_index` **零命中**（无长期槽位）。<br>⇒ **用户可见形态**：日志/UI 肯定地说「已索引 N 个文件、M 个 AST 节点」，而检索侧**永远查不到这些文件**（`RecallPipeline` / `IncrementalIndexer` / `VectorSearchCache` 三者生产实例化点全为 0）。**给出肯定的错误信号**，与 §⑨e 的 F2（风控警示永远绿）同型。<br>**硬约束**：`RecallPipeline<'a>`（`recall_pipeline.rs:71`）持 `&FileIndex` / `&AstIndex` ⇒ 接线**必然**要求索引跨请求存活；而 `rusqlite::Connection` 非 `Sync`，共享方式（`Mutex`/`Arc`）、放哪个槽、以及**是否落盘**（`:memory:` 只活在单次调用内）都是**产品决策**。<br>另：`IncrementalIndexer` 有**两个同名定义**（`:39` 带字段 / `:267` 单元素结构），后者是 `#[cfg(target_os = "android")]` **平台桩** ⇒ 非缺陷（又是一个「同名不同物」） | 你已裁决「全部接线」，但本轮把前置**查实为一条独立的 P1 缺陷 + 两个产品决策**（索引落盘？共享方式？），工作量与风险都属独立一轮；**不建议塞进本轮的已验证改动集**（会作废正在跑的验证） |
| 4 | **「同一事实的多份载体」只被门禁覆盖了一部分**（§12.10 的根因，两处实例） | ① **行号引用**：`check-decl-evidence.mjs` 只扫 `harness/src/knowledge_graph.rs` 的 `evidence:` 字段形态 ⇒ `page_type.rs` 的 rustdoc 引用**早已腐烂**（本轮实测 4 处 +10/+10/+10/+3），`PLAN` 本文 §10.3 两处偏 +54/+47，**两次都不是被检查发现的**。<br>② **reranker 文件名**：Rust 侧已是真真源（`RERANKER_MODEL_FILENAME`，`rag_config.rs:27`，且 `model_downloader.rs:479-491` 有**跨 crate 绑定测试**锁住「下载清单 == 类型默认值」✅），但**前端 2 处字面量无锁**（`settingsStore.ts:152`、`KnowledgeBaseDocuments.tsx:272`）—— 全仓 `grep RERANKER_MODEL_FILENAME\|crossEncoderModel\|bge-reranker scripts/` **零命中** ⇒ 当前值虽已修对，**可再次漂移**（当初就是前端这份缺后缀） | 「把这两类载体纳入门禁」是新增能力、不是纯 bug 修：需决定扫描面（全仓 rustdoc？只限声明块？TS 字面量按常量名反查？）、软/硬判据（判据 **#147**：措辞类会抖，行号越界/常量值不等类不会）、以及复用现有脚本还是新开。故**只登记** |


> **2026-09-15 第四轮更新：本表 4 项已全部裁决并落地** ——
> #1（rerank 阶段分数标尺）+ #2（先过滤后截断）→ **§12.13**；#3（索引管线 P1：落盘/槽位/非 `Sync` 共享）→ **§12.11**；
> #4（「同一事实多份载体」纳门禁）→ **§12.12**。下表内容保留为**裁决前的事实快照**。

### 12.8 验证矩阵（本轮）

> 日志目录：`src-tauri/output/verify-2026-09-15b/`

| 门禁 | 结果 |
|---|---|
| `cargo fmt --all --check` | ⚠ **EXIT=1，但 12 处残留差异全部不在我改的文件里**。我的 3 个文件（`search/src/hybrid_search.rs`、`search/src/rag.rs`、`commands/knowledge.rs`）**首跑确实违规**（6 个位置），已用 `cargo fmt -p axagent-search -p axagent` **只格式化自己的包**修掉（不跑 `--all`，避免改到别人的进行中文件），复查后我的文件 **0 差异** ✅<br>残留 12 处 = `analysis-engine/…/industry_kpi_service.rs`（你的 OPC）、`rt-workflow/{condition_executor.rs, tests/loop_e2e_workflow.rs}`（你的 §10.9）、`tools/…/narrative_structure.rs` ⇒ **非本轮**，未擅改 |
| `cargo fmt -p axagent-harness -p axagent-search -- --check`（补跑，14:07） | ✅ **EXIT=0**（`fmt-harness.exit`）—— 覆盖**当时版本**的 `page_type.rs` |
| `cargo fmt -p axagent-search -p axagent -p axagent-harness -- --check`（**二次补跑，14:2x**） | ⚠ **EXIT=1，但 2 处差异全部不是我的文件**：`src/commands/opc_industry_actions.rs:2326`、`src/commands/opc_workflow_kpi_hook.rs:522`（**你的 OPC 进行中工作**）⇒ **本轮这 7 个文件 0 差异** ✅<br>**这一行为什么叫它「时序漏洞」**：链里的 fmt 跑在 **14:07**，而 `rag.rs` 在 **14:10** 才改 ⇒ 原步骤对最新版本**已失效**。若不补跑，上表就会留下一条「对旧版本成立」的绿灯 —— 正是 §12.9 那句「结论 → 源码树会过期」的第二次实例 |
| `cargo check -p axagent-search --all-targets` | ❌→✅ **首轮 EXIT=101（2 × `E0507`）→ 修复后 `EXIT=0`，0 error / 0 warning**（`check-search.log`，`Finished dev profile in 6m 27s`）<br>错因：`rag.rs:1618` / `:2669` `cannot move out of \`src_ref.source_type\` which is behind a shared reference` —— **我自己的 bug**：新建的 `RAGSourceType::as_str` 写成了 `const fn as_str(self)`（按值），而 `RAGSourceType` 只派生 `Clone`（无 `Copy`）、调用点持 `&RAGSourceRef` ⇒ move。**已修**（改 `&self` + rustdoc 注明原因）。<br>⚠ 同一表达式里有**两种 `.as_str()`**：`RagSourceResult.source_type`（`String`）走 `String::as_str`，不受本次签名影响 |
| `cargo check -p axagent-harness --all-targets` | ✅ **EXIT=0，0 error / 0 warning**（`check-harness.log`，2m11s）—— 覆盖 §12.10 的 `page_type.rs` 改动（含 test targets） |
| `cargo test -p axagent-harness` | ✅ **EXIT=0** —— **684 + 42 + 6(doctest)，0 failed / 0 FAILED / 0 panicked**（`test-harness.log`）。其中 `page_type::tests` **12 条全过**，含 §12.10 直接牵动的 `test_observed_write_values_are_in_vocabulary`（D3 回归锁）与 `test_as_str_is_unique_and_roundtrips` |
| `cargo check -p axagent` | ⏸ 首轮止于 `axagent-search`（依赖先编）⇒ 该 crate 自身尚未被检查，随串行批次覆盖 |
| `cargo test -p axagent-search` | ✅ **EXIT=0** —— lib **179 条：178 passed / 0 failed / 1 ignored**（`test-search.log`）+ 4 个 doctest 全过。**本轮新增/重写的 13 条全部在场且通过**（`hybrid_search` 8 条，含 `test_weighted_rrf_weights_change_ranking` —— 该用例要求**名次反转**，权重未被读取时必红；`rag` 4 条含重写后的 `test_default_l2_threshold_is_reasonable`；`entity_graph` 2 条）。唯一 ignored 是**既有**的 `reranker::tests::test_voyage_backend_with_invalid_key_falls_back`（原因串写明需外网真实 Voyage API）⇒ 非本轮、非新增 |
| `cargo clippy -p axagent-search -p axagent --all-targets -- -D warnings` | ⏸ 同上（lint-only 规则，**不得由 `check` 全绿推定** —— 判据 #72⑤ / #201）<br>**→ 收口指针**：**§12.16.3 未闭口 1**（本段升至三包含 `-p axagent-harness`，正在等锁排队） |
| `tsc --noEmit` / `dprint check` / i18n / 错误码 / 契约脚本 | ✅ **全部 EXIT=0**：`tsc2.log` **0 字节 = 0 错误** · `dprint2.log` **EXIT=0** · `hardcoded2.log`（`--strict`）**全 PASS** · `i18n2.log` **未翻译 0 / 新增 0** · `errcode2.log` **413 个错误码 ↔ 11 语言一致，无孤儿码**<br>说明：本轮前端只改了 i18n **值**（11 语言 `entityGraph.desc`）与一处默认值后缀，故与 §10.6 基线一致 |
| `check-decl-evidence.mjs` | ⚠ **EXIT=3**（软判据命中的既定码，判据 #147）—— `decl-evidence2.log` 报 **`腐烂 0`**、仅剩 1 条**刻意保留**的 `[EdgeType] mapping → wiki.rs:1132`（合成边本就不带 `relationType`，注释即证据）⇒ 与 14:01 那次**完全一致，无回归** |
| `check-layer-discipline.mjs`（复跑） | ❌ **EXIT=1：7 处超基线，全部在别的会话的文件里，本轮零命中** —— `commands-no-direct-db`：`opc_workflow_kpi_hook.rs`、`stock_workflow/sim_hook.rs`；`commands-no-sibling-call`：`opc_industry_runtime.rs`（5>3）、`opc_learning_hook.rs`（4>3）、`opc_workflow_kpi_hook.rs`、`stock_workflow/{sim_hook,rhai_pm}.rs`。<br>⚠ **基线数从记录中的 3 处涨到 7 处**，增量全来自**并行会话正在做的 OPC 工作**（非本轮、未擅改）。棘轮本身工作正常（`stores-domain-no-feature` 13 处 == 基线 13 ⇒ 不报） |

> ⚠ **首轮真编译证据立刻抓出 2 个真错**（§10.6 那句「不得把「已改」读成「已验证」」不是形式主义）。
> §12.1 的量纲改动本身未报错；错的是**同一个改动里顺手做的字符串收敛**（`as_str` 签名）。
> 这正好印证判据 **#72 型②**：错误文件与我清单里改的是同一个文件，但**错误机制 ≠ 我以为的机制** ——
> 若不真跑，这次会被记成「10 项全绿」。

### 12.9 验证为什么排了两轮队（与 §10.6 同一纪律）

`cargo` 的锁是**按 target 目录**的，与「跑哪个包」无关。

- **队列 1**：13:43 起并行会话的 `cargo check --lib -p axagent`（13:43:51）与
  `cargo test -p axagent-rt-workflow --test loop_executor_integration`（13:49:19）在跑，
  我 13:53 发起的 `cargo check` 落到 `Blocking waiting for file lock on build directory`，**等了 15m45s**。
- **队列 2**：对方 14:06:33 又起 `cargo check -p axagent`，14:06:39 起
  `cargo test -p axagent-rt-workflow --lib resolve_var_path`；我 14:07:22 发起的串行批次
  （`run-gates.sh`，见上表）再次排队。

按既定纪律**不抢锁、不打断**（上一轮我持 `--workspace` 锁 1h53m 已把对方 10:56 的请求饿到 12:19）。
⇒ 结论只对**跑到时**的源码树成立。本轮该纪律还踩到一个反面：

> ⚠ **「补跑结论对当前源码树成立」是一个会过期的断言**（本文件上一版就是这么写的）。
> 我在此之后**又改了两次源码**（14:06 `page_type.rs` 行号修复、14:10 `rag.rs` 的 `as_str` 签名），
> 于是那条断言当时即已失效。**凡是「结论 → 源码树」的绑定，必须同时记下源码树的 mtime/哈希，
> 否则补跑日志会被后来者当成对最新树有效。** 本轮改为：日志目录内保留
> `run-gates.sh` + 每步 `.exit` 码，且「最后一次源码改动时间」写进 §12.10 末尾。


### 12.10 连带修复：`page_type.rs` 的 **4 处行号引用已腐烂**（且**无门禁覆盖**）

**为什么现在才发现**：`check-decl-evidence.mjs` 的扫描面是 `harness/src/knowledge_graph.rs` 的
「声明块 + `evidence:` 字段」形态，**不覆盖 `page_type.rs`**。后者的 `文件:行` 引用写在 rustdoc 与
行内注释里 ⇒ **该形态没有任何门禁**，自 09-14 起静默腐烂。

| 引用（修复前） | 真值（2026-09-15 逐行实测） | 漂移 |
|---|---|---|
| `src/commands/wiki.rs:1650`（`Daily` 写入方） | **1660** `page_type: Some("daily".to_string())` | **+10** |
| `src/commands/wiki.rs:1896`（`Knowledge` 写入方） | **1906** `page_type: Some("knowledge".to_string())` | **+10** |
| `src/commands/wiki.rs:1396`（`Synced` 写入方） | **1406** `page_type: Some("synced".to_string())` | **+10** |
| `crates/agent/src/wiki_compiler.rs:779`（`Log`） | **782** `"log" => ""`（779 是 `"comparison"` 分支） | **+3** |

**判据的可复用之处**：三个 `wiki.rs` 锚点**一致偏 +10**（+ 同表里另外 4 个跨文件引用
`opc_knowledge.rs:254` / `knowledge_source.rs:237` / `services.rs:375` / `wiki_compiler.rs:671` **全部仍然正确**）
—— 多点一致偏差 ⇒ 先信它是**真位移**，而不是我读错了行（判据 #8 的正向用法）。

**手法**：`src-tauri/output/fix-page-type-linenos.mjs`，三重自检 ——
① 每处替换的**实际命中次数必须等于声明次数**（不等即 exit 1 不落盘）；
② 打印 before/after 供复核；③ **总行数必须不变**（本次只允许改行号，不许增删行）。
自证结果：**7 条规则全部精确命中，489 → 489 行**，落盘后 4 个新行号逐行比对真实内容全部吻合。

> ⚠ **`src-tauri/output/` 在 `.gitignore` 里**（另一会话已实测：`.gitignore:134`）⇒ 上面那个脚本**会消失**。
> 故把可复用的骨架 + 「先备份、命中数写死、行数不变、跑完逐行读回」四条纪律已写进技能
> `rust-consistency-audit/references/method-09-cross-layer-semantics.md`（陷阱 ②）。
> 本文件保留的只是**本次证据**，不是可执行文件。

**这是第二次踩同一个坑**（§10.8 是 `commands/knowledge.rs` 的 8 处）。判据 **#155** 已写明
「改动在文件中部插入行 ⇒ 全仓 `文件:行` 引用当场腐烂」且「修法必须**全仓正则普查**」；
本轮新增的经验是：**普查是一次性的，配门禁才是长效** —— 未被门禁覆盖的文件会**第二次**再烂，
而两次都不是被检查发现的，是「顺手核一下」核出来的。

**同族登记（未逐条改，按本文件惯例）**：
- §10.3 表里的 `rag.rs:774` / `rag.rs:848` 现真值为 **828**（`search_entity_graph_for_sources`）/
  **848**（`fold_entity_graph_context`），偏 **+54 / +47**；§10.3 与 §11 的行号属**当时的快照**，
  本文件对此的既有处置是**只记录漂移事件、不重写历史**（§10.8 即如此）⇒ 读 §10.3 时**按函数名定位**。
- 反过来，本轮新写的 §12.7 的行号**已逐行复核**：`reranker.rs:460-464`（`sort_by` 用阶段 `score_map`）
  与 `:479`（`rerank_score: r.combined_score`）与描述吻合 ✅。
- **本文件自己也烂了一处（QA 时抓到，已修）**：§12.1.1 引的前端行号 `KnowledgeBaseDocuments.tsx:960/1056`
  实测应为 **959/1055** —— 根因是**同一轮里对该 `.tsx` 跑过一次 `dprint fmt` 把两行并成一行**（§12.3 的
  `crossEncoderModel` 那处），于是其后**全文件行号 −1**。这正是「**同一轮内先写文档引用、后格式化源文件**」
  造成的自伤：写引用时是对的，fmt 之后就不再是。⇒ 纪律：**凡对源文件跑过 fmt，之前写的 `文件:行` 引用一律作废**。

### 12.11 第四轮 · 索引管线 P1 裁定（§12.7#3）——**落盘＝是 / 不加槽位 / 不跨 `.await` 共享**

#### 12.11.1 先查「有没有消费者」，再决定要不要修仓库

`§12.7#3` 把前置查成了「一条独立的 P1 缺陷 + 两个产品决策」。本轮先把**消费者数量**量到确凿（判据 #101：交付物必须逐项统计自己的生产消费点）：

| 类型 | 生产实例化点（实测，`grep` 排除 `target/` 与自身单测） |
|---|---|
| `FileIndex` / `AstIndex` | **1 处**：`src/indexing_triggers.rs:58` / `:90`（且产物被丢，见下） |
| `RecallPipeline` | **0**（全仓仅 `crates/search/src/recall_pipeline.rs:71/78` 定义 + `:326/343` 自身单测） |
| `IncrementalIndexer` | **0**（仅 `incremental_indexer.rs:238` 自身单测） |
| `VectorSearchCache` | **0**（此事实**前会话已如实登记**于 `crates/cache/src/cache_snapshot.rs:35`、`crates/disk-cache/src/lib.rs:13`） |
| `AppState` 索引槽位 | **0**（`grep file_index\|ast_index` 于 `src/` 零命中） |

> ⚠ 看起来像消费者的两处**都不是**：`crates/runtime/src/mode_selector.rs:72-81` 的
> `"file_index"`/`"ast_index"`/`"recall_pipeline"` 只是**字符串**（且该模块自身无生产入口，
> 已在该文件 `:27-43` 如实登记）；`AppState::indexing_semaphore` 虽**确有**消费点
> （`commands/conversations/mod.rs:1245`），但它闸的是**文档**索引，不是代码索引。

#### 12.11.2 三项裁定

| 决策 | 裁定 | 依据 |
|---|---|---|
| **索引是否落盘** | **是** → `<app_data_dir>/index.db` | 落盘是「产物可达」的前提。改动前 `open_in_memory()` 且是局部变量 ⇒ 返回即 drop，扫描**必然**丢失；落盘后至少变成**持久可达**。⚠ 刻意**不**与 sea-orm 的 `axagent.db` 合并：两侧是独立访问层，同库会争 schema 与写锁 |
| **放哪个槽** | **本轮不加** | 消费者为 0 时加槽位 = 制造**新的**悬空链（判据 **#152**「接线点须落在公共下游」）。**将来**真接线时的既定形态：`AppState.file_index: Arc<Mutex<FileIndex>>` / `ast_index: Arc<Mutex<AstIndex>>`，且**必须与第一个真实消费者同批落地**。**不接受 `OnceLock`**（不可替换，测试与多工作区换不了） |
| **非 `Sync` 如何共享** | **不跨 `.await` 共享** | `rusqlite::Connection` 是 `Send` 但**非 `Sync`**。`Arc<Mutex<FileIndex>>` 本身是 `Send+Sync`，但 `RecallPipeline<'a>` 借用 `&FileIndex` ⇒ **全部**索引访问都收进 `spawn_blocking` 闭包（闭包内无 `.await`）。**禁止**把 `MutexGuard` 持过 `.await`（会让 future 非 `Send`，且是这类代码最常见的错误形态） |

#### 12.11.3 落盘**当场暴露**的一处潜伏缺陷（内存库时代不可能被发现）

`FileIndex::scan_directory` 的陈旧清理只删 **root 之外**的行（`path NOT LIKE '<root>%'`），
**从不删 root 之内「已从磁盘消失」的行**。内存库每次调用都是新建的 ⇒ 该缺陷**结构上不可观测**；
一旦落盘，它就是**持久幽灵行**（任何未来的消费者都会拿到已删除文件的结果）。

⇒ 本轮补齐「**root 切片全量替换**」：扫前先 `remove_by_prefix(root)` 清该 root 的旧切片。
两个索引各补/修一处，**并且**顺带修真了一个既有缺陷：

| 位置 | 改动 | 为什么 |
|---|---|---|
| `file_index.rs` `remove_by_prefix` | `LIKE 'prefix%'` → `substr(path,1,length(?1)) = ?1` | 扫描根由 `WorkspaceUri::cache_path` 生成，形态 `<authority>_<md5[..8]>`（`crates/storage/src/workspace_uri.rs:95`）—— **必定含 `_`**，而 `_` 是 `LIKE` 通配符 ⇒ `LIKE` 会把首字符不同的**兄弟目录**一并删掉 |
| `file_index.rs` `scan_directory` | 同上（`NOT LIKE` → `substr(...) <> ?1`） | 同一根因；`NOT LIKE` 方向下是**漏删**，比误删隐蔽 |
| `ast_index.rs` **新增** `remove_by_prefix` | 5 张表逐表删（`file_path` ×4 + `caller_file`） | 与 `FileIndex` 对称。`AstIndex::index_file` 只按**单文件** `DELETE`，对「文件被删除/改名」无能为力 |

#### 12.11.4 诚实化：那条「肯定但为假」的成功日志

改动前调用方打印 `"Post-sync/Push indexing complete: {N} files, {M} AST nodes"`
—— 日志断言成功，而产物**已被丢弃**（与「风控警示永远绿」同型）。
改为如实描述，并**把产物落在哪儿写进报告**（`IndexingReport` 新增 `index_path` / `stale_rows_removed`）：

```
Post-sync indexing: N files + M AST nodes -> <app_data_dir>/index.db
  (replaced K stale rows; warn: no retrieval consumer yet, see PLAN §12.11)
```

> **为什么不顺手把 `RecallPipeline` 接上**：那需要先造一个**用户可见入口**（UI 或 agent 工具），
> 而「入口长什么样」是产品决策，不是接线细节。在没有入口的情况下把 `RecallPipeline` 塞进
> `AppState`，只会把「索引产物丢失」升级成「持久死数据 + 无人读的新槽位」——
> 即本文件反复出现的**入边缺失型**断链。⇒ 本轮只做**有据**的两件：落盘（产物可达）+ 日志诚实。

### 12.12 第四轮 · 「同一事实的多份载体」纳门禁（§12.7#4）

新增 `scripts/check-single-source-facts.mjs`（+ 棘轮基线 `scripts/single-source-facts-allowlist.json`），
覆盖 §12.7#4 点名的两类载体。**设计上最贵的一课发生在实现过程中**，逐条记下。

#### 12.12.1 扫描器自身撒了两次谎（都是「把解析器盲区当缺陷」）

| 版本 | 报出 | 真实 | 根因 |
|---|---|---|---|
| v1 | **274 处「客观错」** | **0** 处真腐烂 | 引用多半是**裸文件名**（`wiki.rs:1660` / `mod.rs:196` / `tool_executor.rs:177`）—— 出处没写路径。解析器只试「仓库/`src-tauri`/`src-tauri/crates` 三个基准目录」⇒ 全部落到「文件不存在」。**274 条里一条真的都没有**（判据 #7 的教科书形态） |
| v2 | 仍有 ~10 处 | 其中 **8 处是假阳性** | ① `--> src/main.rs:4:5`（cargo 诊断的 `行:列`）被读成 `src/main.rs:5`；② `:64-70` 的**区间端点**落在空行被硬判；③ `rhai-1.26.0/src/…`、`sea-orm-2.0.1/src/…` 是**第三方 crate 源码**，本就不该在本仓解析 |

三条修法（都配了负样本，判据 #147②）：

1. **三分法**（照抄 `check-decl-evidence.mjs` 的既定判据，它早已写明）：`LOCATED`（可唯一定位）
   / `NONLOC`（裸同名多份且后缀不能消歧，如 `mod.rs` 345 份）/ `BROKEN`（真不存在）。
   **只有 BROKEN 与「越界 / 空行」才是失败**；判据：同名多份时先用**引用串的路径后缀**消歧
   （`init/state.rs` 只对应一个真实文件），仍不行才落 NONLOC。
2. **`行:列` 守卫**：负向先行断言必须跑在**整行**上，**不能对切片用** ——
   切片会让 `:4:5` 里第二个 `:` 因为「处于是字符串开头」而通过断言。
3. **区间端点 `blankOk`** + **第三方源码块级判定**：`//` 连续注释块里出现
   `name-x.y.z/` 形态（`EXTERNAL_MARK`）⇒ 该块内的引用一律 `EXTERNAL`，不判失败。
   取**块级**而非行级：实测 `seed_content_media.rs:867-876` 是一个 doc 块，
   只在其第 867 行写了 `rhai-1.26.0/src/engine.rs`，而第 869/876 行把路径省略成
   `packages/pkg_std.rs:21-31` / `array_basic.rs:59` ⇒ 行级判据必然漏。

#### 12.12.2 覆盖范围**自陈**（判据 #16：不自陈 ⇒ 「0 腐烂」会被读成「全查过了」）

每次运行都打印四分法计数，并把 `NONLOC` 的条目**点名列出**（附「补上路径即可纳入检查」）：

```
扫描面：1916 个 .rs ⇒ 447 条「文件:行」引用（去重后 354 条）
分类：LOCATED 139（硬判据 + 软判据都跑）｜NONLOC 207（无法定位 ⇒ 未检查）
      ｜EXTERNAL 6（第三方 crate 源码）｜BROKEN/越界/空行 2
```

**未覆盖的明确声明**：Markdown 文档（PLAN/AGENTS.md 里的 `文件:行` 属散文，抖动大）、
`scripts/*.mjs`（自带合成样本）、前端 `.ts/.tsx` 里的行号引用**均不扫**。
⇒ 本门禁是**抽样守卫**，不是全称保证。

#### 12.12.3 两类判据的分流（判据 #147）+ 三条退出码的实测证据

| 类 | 例 | 退出码 |
|---|---|---|
| **客观错** | 文件不存在 / 行号越界 / 指向空行 / `crossEncoderModel` ≠ `RERANKER_MODEL_FILENAME` 真源 | **1** |
| **软判据** | 引用出处那一行已找不到「同句引号里的值」（疑似腐烂，需人判） | **3**（`--ci` 降级为 0） |
| 脚本自身失效 | 扫描面为空 / 一条引用都没抽到 / 真源常量读不到 / 前端载体一个都找不到 | **2** |

**三条码都在真实源码树上跑出来了**（不是合成夹具）：
① 未建基线时首跑 → `EXIT=1` 且同时存在 3 条软命中 ⇒ 证明硬拦优先于软判据；
② 建基线后 → `EXIT=3`（只剩软命中）；③ 同树 `--ci` → `EXIT=0` ⇒ 证明降级真发生。

#### 12.12.4 棘轮基线与 bootstrap 陷阱

基线 `scripts/single-source-facts-allowlist.json` **只减不增**（`--update-baseline` 在
「出现新增客观错」时**拒绝写入**）。**当前 0 条**（初始 2 条已于 2026-09-15 15:1x **修掉**，非放宽）：

| 键 | 事实 | 处置 |
|---|---|---|
| `stock_workflow/reflection.rs::stock_analysis_setup.rs:4538` | 引用**已不存在的文件** —— `stock_analysis_setup.rs` 已拆成同名**目录**（`stock_analysis_setup/{mod.rs, seed_stock_analysis.rs}`），是拆分遗留的陈旧引用（与 §10.8/§12.10 同族） | ✔ 改指 `src/commands/stock_analysis_setup/mod.rs:1525` |
| 同上 `:4552` | 同上（同一引用的区间终点） | ✔ 同上 |

**故意不擅改**：注释说的是「内联 system_prompt 引用了 `{{stock_code}}`/`{{stock_name}}`」，
而候选锚点至少有两处（`mod.rs:2211` 的 `auto-position-plan` 与 `mod.rs:2245` 的
`auto-stop-loss-review`），**选错锚点比留着陈旧引用更坏**（会给出一个指向错误位置的"可信"引用）。
⇒ 按判据 #16 进基线 + 在此登记，等定位到确切的旧行内容再改。

> **上面这段判断已被推翻，留作教训（2026-09-15 15:1x 更正）**：
> 当时是**只按行号找** —— `mod.rs` 里当然没有 4538 这个行号 ⇒ 落进「无法判定」。
> 破解点是**改用内容找锚**：注释点名了它引用的 prompt 含 `{{stock_code}}` 与 `{{stock_name}}`
> 两个占位符，直接 grep 这两个字面量 ⇒ **唯一命中** `src/commands/stock_analysis_setup/mod.rs:1525`
> （`reflection-agent` 的 `system_prompt`，字面即 `目标股票代码: {{stock_code}}，股票名称: {{stock_name}}`）。
> 上文列的两个候选锚点（`mod.rs:2211` / `mod.rs:2245`）**是别的 agent**（`auto-position-plan` /
> `auto-stop-loss-review`），与注释描述的**反思** agent 不是同一个对象 —— 选它们才是错的。
> **教训：行号是坐标，内容才是身份。行号失效时不要停在「无法判定」，回到引用行去读它的语义谓词。**

> ⚠ **bootstrap 陷阱（实测踩到并修）**：`--update-baseline` 的「拒绝新增」守卫写成 `if (added.length > 0 && baseline.known)`，
> 而 `loadBaseline()` 在文件不存在时返回 `{known: []}` —— **`[]` 在 JS 里是真值** ⇒
> 首次 bootstrap 被误判成「已有基线 ⇒ 拒绝新增」，基线**永远写不进去**、脚本永远退 1。
> 正判据必须是 `fs.existsSync(BASELINE_PATH)`，不是取值的真值性。

#### 12.12.5 接线（判据 #149 三问 + #147③ 两侧逐字同源）

| #149 三问 | 本例 |
|---|---|
| ① 挂到有真依赖的 job？ | ✅ 纯 Node、秒级、不依赖 `npm ci`/构建产物 ⇒ 与同族门禁一起放 `frontend-check` 的**纯 Node 静态门禁段**（`dprint`/`tsc`/`build` 之前，最快失败） |
| ② 有 SKIP 分支？ | ✅ 无 SKIP 分支；但**「凭据/配置缺失」型早退已改为退 2**（脚本自身失效），绝不用退 0 冒充通过 |
| ③ 读开发机专有配置？ | ✅ 不读。扫描面全是仓库内文件（第三方 crate 源码走 `EXTERNAL` 分类，**不去读** `~/.cargo/registry`） |

- CI：`.github/workflows/ci.yml` 新增 step「Check single-source facts (line-ref rot / reranker filename drift)」
- 本地镜像：`scripts/ci-check.mjs` 新增两条 `step(...)`；**两处命令行逐字相同**
  （`node scripts/check-single-source-facts.mjs --selftest` / `… --ci`）
- `package.json` 新增 `check:facts` / `check:facts:selftest` / `check:facts:ci` / `check:facts:baseline`

### 12.13 第四轮 · §12.7#1/#2 落地摘要

- **#1 重排阶段分数丢弃**：先**逐一实测四类 rerank 后端的分数标尺**（`RuleReranker` 五项加权和 /
  `CrossEncoderReranker` 走 `InferenceEngine::rerank` 的 sigmoid 归一 / Cohere-Jina-Voyage 的
  `relevance_score`）⇒ **四者同为「相似度 ∈[0,1]、大 = 相关」**，与融合分 `combined_score` **同标尺**
  ⇒ 写回 `rerank_score` 安全，并把标尺表写进 `RerankBackend` trait 文档（含「将来接入无界 logits
  后端必须先在适配器内归一」的禁令）。
- **#2 先过滤后截断**：`HybridSearchOptions.min_score` **本来就实现了**「过滤 → 排序 → 截断」，
  缺的只是**从来没有调用方设过它**（`rag.rs:174` 与 `hybrid_options_from_config` 都硬写 `None`）。
  ⇒ 新增签名参数 `min_similarity: Option<f32>`，自 `command → indexing → rag → HybridSearchOptions.min_score`
  全程穿透，并删除 5 处「先截断后过滤」的重复后置过滤。
  ⚠ 同时发现本地 cross-encoder 是**安慰剂**：`load_candle_model(ModelKind::Reranker)` 恒返回 `None`，
  且 `worker_main` 的 `WorkMsg::Rerank` 分支**只调 `heuristic_rerank`**（不像相邻的 `Judge` 分支有
  `match &loaded`）⇒ GGUF 被下载、加载、然后**从不使用**。已如实登记，未擅自实现 candle BERT。

**本轮最后一次源码改动时间**（供后续判断本文件结论是否过期 —— 表内任何绿灯只对**晚于**这些时间的产物有效）：

| 文件 | mtime |
|---|---|
| `crates/search/src/rag_pipeline.rs` | 14:47:24 |
| `crates/search/src/rag.rs` | 14:51:02 |
| `src/indexing.rs` | 14:51:12 |
| `src/commands/knowledge.rs` | 14:51:24 |
| `src/commands/memory.rs` | 14:51:33 |
| `src/commands/paper.rs` | 14:51:43 |
| `src/init/state.rs` | 14:51:59 |
| `src/commands/unified_knowledge.rs` | 14:51:51 |
| `src/commands/wiki.rs` | 14:52:17 |
| `crates/search/src/file_index.rs` | 14:57:30 |
| `crates/search/src/ast_index.rs` | 14:57:34 |
| `src/indexing_triggers.rs` | 14:57:49 |
| `src/commands/cloud_workspace.rs` | 14:57:57 |
| `crates/search/src/reranker.rs` | **15:06:49**（本轮最后一处源码改动） |
| `src/commands/stock_workflow/reflection.rs` | 15:09:31 |
| `crates/harness/src/knowledge_graph.rs` | **15:11:45** |

> ⚠ **本表上一版记得过早，已更正**：原表写 `rag.rs 14:10:05` / `knowledge.rs 13:58:27`，
> 而这两个文件的实际 mtime 是 **14:51:02 / 14:51:24**（即 §12.13 的 #2 穿参编辑本身）。
> 少算的那 41 分钟恰好是**改动最集中**的一段 ⇒ 用原表读「绿灯」会把**改动前**的产物当成
> **改动后**的有效证据（判据 #175：日志 mtime 早于改动即作废）。**手抄 mtime 必然腐烂**，
> 机械做法见 §12.14.4。

（前端：11 个 `src/i18n/locales/*.json`、`src/stores/feature/settingsStore.ts`、`src/components/settings/KnowledgeBaseDocuments.tsx` —— 见 §12.3；这些在 `dprint2.log` / `tsc2.log` 之后未再改动。）

### 12.14 第四轮验证：**两处自伤 + 一次「门禁报 0 却真腐烂」**

本轮**没有**再发现产品缺陷；三处问题里**两处是我自己改出来的**。逐条留证，因为它们比产品缺陷更值得记。

#### 12.14.1 自伤一：`RerankPipeline::execute` 的 `E0382`（先 move 后借用）

写回阶段分数的第一版把查询内联进了结构体字段字面量：

```rust
.map(|(i, r)| RerankedResult {
    id: r.id,                                                  // ← r.id 在此被 move
    …
    rerank_score: stage_scores.get(&r.id).copied()…,            // ← E0382：move 之后再借用
})
```

`cargo check -p axagent-search` 报 `error[E0382]: borrow of moved value: r.id`（`reranker.rs:533`）。
修法：**先算再 move**（`let rerank_score = …;` 提到结构体之前），并把原因写进注释。
⇒ 教训：结构体字面量里的字段**求值顺序是书写顺序**，任何「后面字段要用前面字段的同一个值」
都必须提前成局部变量。这类错误编译器能抓，**代价只是必须真跑一次编译** ——
而本轮它恰好被连跑两次（第一次因为会话被截断、修复后从未编译，第二次才浮出来）。

#### 12.14.2 自伤二：**本门禁报 0 处客观错，而真腐烂就在眼皮下**

给 `commands/knowledge.rs` 加一个函数参数（`min_similarity`，见 §12.13 #2）⇒ 该文件净增 4 行
⇒ `harness::knowledge_graph::DATA_DRIVEN_COLUMN.evidence = "src/commands/knowledge.rs:1745"`
所指的行变成了 `continue;`（真正读 `rtype` 的 `let rtype = fields[2]` 已移到 **1749**）。

| 检查者 | 结果 |
|---|---|
| `check-single-source-facts.mjs`（本轮新建） | **0 处客观错** —— 1745 行确实存在、非空 ⇒ 结构判据全过 |
| `check-decl-evidence.mjs`（既有） | 软判据（该行是否还讲 rtype）—— CI 容忍 |
| `cargo test -p axagent-harness` | **`EXIT=101`** ← **唯一真正抓住它的** |

**为什么本门禁抓不到**（能力边界，已写进脚本头部自陈）：软判据只比对**引号包裹的字面量**
（`quotedTokens`），而 `evidence` 串里的引用含 `/`，被 `if (t.includes("/")) continue` 刻意排除
⇒ 这条形态下软判据无从触发。**结论：本脚本只保证「引用的坐标还在」，不保证「坐标上讲的还是那件事」。**
语义层归 `check-decl-evidence.mjs` + harness 单测，**两者不可互替**。

> 这是 **type ⑥ 的当日复发**：`rust-consistency-audit` 技能里早已记过「我自己的改动使既有行号引用
> 整体腐烂」，且举的就是**同一个文件、同一个测试**。**「记住」没有挡住它** ⇒ 已把该技能条目
> 更新为「改完任何 `.rs` 就跑 `node scripts/check-single-source-facts.mjs`」+ 明写其能力边界。
> 防复发的机制只能是**改完就跑**，不能是「记得小心」。

#### 12.14.3 第四轮验证矩阵（实测值，含反例）

| 命令 | 结果 | 说明 |
|---|---|---|
| `check-single-source-facts.mjs --selftest` | **`EXIT=0`** 19/19 | 含 5 条负样本（越界 / 空行 / 同名多份 / 去重 / 常量缺失退 2） |
| `check-single-source-facts.mjs --ci` | **`EXIT=0`** | `LOCATED 137 ｜ NONLOC 220 ｜ EXTERNAL 6 ｜ BROKEN/越界/空行 0`；软判据 0 命中 |
| `check-single-source-facts.mjs --update-baseline` | 基线 **2 → 0** | 两条存量**已修**（见 §12.12.4），不是放宽 |
| `cargo test -p axagent-search` | **`EXIT=0`**（107s） | ⇒ **item #1/#2 的 `min_similarity` 穿参真编译通过、单测全绿** |
| `cargo test -p axagent-harness` | ⚠ 首跑 **`EXIT=101`**（683 passed / 1 failed） | 失败即 §12.14.2 的 evidence 腐烂；已修，**待复跑** |
| `cargo check -p axagent-harness` | `EXIT=0`（69s） | |
| `cargo check -p axagent-search --all-targets` | ⚠ 首跑 `EXIT=101` | 即 §12.14.1 的 `E0382`；已修 |
| `cargo check -p axagent --all-targets` | ⏳ **未取得终值** | 见 §12.14.5 |
| `cargo fmt --check` | ⚠ `EXIT=1` | 差分含**其它会话**的文件；只应对本轮触碰的文件 `cargo fmt`，不整仓格式化 |

#### 12.14.4 「最后一次改动时间」不该手抄

§12.13 的表就是活教材：手抄必然漏（漏了 41 分钟、13 个文件）。
**机械做法**：结论只对「在**本文件 mtime 之后**重新跑出来的产物」有效 ——
判定时直接比 `产物 mtime > 被改文件 mtime`（比 epoch 毫秒，**不要比 `DateTime` 对象 / ticks**：
Z 后缀经 PowerShell 解析会变 `Kind=Local`，东八区静默差 8 小时）。

#### 12.14.5 未闭口项（如实登记，勿当已绿）

- `cargo check -p axagent --all-targets` 与 `cargo clippy -p axagent … -- -D warnings` **尚无终值**：
  本轮该命令先后被两个因素打断 —— ① 一个会话持 cargo `target` 锁（`Blocking waiting for file lock`）；
  ② 本会话的后台批次被 **2 分钟默认超时**掐断（`clippy.exit` 未落地可证）。
  ⇒ `commands/{knowledge,memory,paper,wiki}.rs` 的**类型检查仍未被真跑过一次**（此前 `E0425` 属名字解析阶段，
  类型检查尚未开始；其后 `E0382` 又把它挡在后面）。**必须补跑，不可由「其它 crate 绿」推定。**
- `scripts/ci-check.mjs` 与 `package.json` 的三条命令镜像（§12.12.5）需在补跑时一并复验。

（↑ 本节两条未闭口**已在 §12.15.5 收口**：`check -p axagent --all-targets` 取得终值 `EXIT=0`；
命令行镜像**逐字同源**已复验。**但 §12.14.3 那个 ⏳ 行本身保留不动** —— 它记录的是当时的事实。）

---

### 12.15 第四轮收口续（第五段验证）：**「同一形态按信噪比分两档」新硬判据** + 行号**第四次**漂移全修 + 并发**两次**作废

本段没有新产品缺陷。**三件事**：① 上一次的「门禁报 0 却真腐烂」被追到底，并**改成了一条可量化的硬判据**；
② 行号引用的**第四次**漂移（本轮之前只有 1/4 被修）；③ 一天内**第二次**「并发会话替我把错误改好了」。
所有量化数字都是**当次源码树**的实测值（判据见 §12.14.4）。

#### 12.15.1 行号第四次漂移：7 处，而**只有被硬判据罩住的那 1 条**被修好了

给 `src/commands/knowledge.rs` 的 `graph_import` 加参数（净 +4 行，§12.13#2）⇒ 该文件整体下移 4 行
⇒ `crates/harness/src/knowledge_graph.rs` 里指向它的引用**一起漂移**：

| 引用处 | 原值 → 真值 | 载体 | 漂移后落在 | 谁抓到的 |
|---|---|---|---|---|
| `:489` `in_industry` | 1789 → **1793** | `evidence:` 字符串 | `continue;` | 只有软判据（`check-decl-evidence.mjs` 内容存疑） |
| `:498` `has_concept` | 1774 → **1778** | `evidence:` 字符串 | `continue;` | 同上 |
| `:533` `has_chairman`（`employ_` 族） | 1805 → **1809** | `evidence:` 字符串 | `}` | 同上 |
| `:518` / `:539` / `:1087` / `:1308` | 1805→1809 / 1745→1749 / 1789→1793 ×2 | rustdoc 注释 | `}` / `continue;` / `continue;` ×2 | 同上 |
| `:577` `DATA_DRIVEN_COLUMN.evidence` | 1745 → **1749** | `evidence:` 字符串 | `continue;` | **`cargo test -p axagent-harness` 的 `test_csv_import_relations_are_declared`（硬判据）** |

**关键事实**：当日**只有最后一行**被修好（另一并发会话按单测报错信息改的），
另外 3 条 `evidence:` 引用**继续腐烂到本段才被扫出来**。
⇒ 这就是新判据 **#199**（**软判据进了 CI 就等于不存在**：`--ci` 把「内容存疑」降级成「打印不失败」
是 #147 的正当设计，但**没人看 = 没检查**）的**首个实证**。

> 这也是既有判据 **#155**（改动在文件中部插入 ⇒ 全仓 `文件:行` 引用当场腐烂）的**第二次复发**。
> **#155 只说了「修法必须全仓正则普查」，没说「怎么让它别再烂」。** 本段的答案是**加硬判据**（§12.15.2）。

#### 12.15.2 新硬判据：**先量后改**，同一形态按信噪比拆两档（判据 #198）

⛔ **不要凭直觉决定「指向哪一行算错」**。先写只读探针把全仓引用按「所指行形态」分桶、逐条打印原文，
再数信噪比。实测口径：1916 个 `.rs` ⇒ 445 条引用（171 条可定位）：

| 形态 | 命中 | 其中真腐烂 | 判定 |
|---|---|---|---|
| 指向**裸控制流语句**（`continue;` / `break;` / `return;`） | **7** | **7（100%）** | ✅ **可硬拦**（客观错，退 1） |
| 指向**收尾括号**（`}` / `},` / `);` / `} else {`） | 5 | 3 处是**区间终点**、2 处是「整段」松散写法 | ⚠ **只能软**（退 3） |
| 指向其它正常代码行 | 159 | 本门禁**看不见** | 归语义面（§12.15.6） |

两条配套实现细节（都实测踩过）：
1. **区间终点必须与起点分开判定**：`:1913-1916` 的 `1916` 本来就是收尾行 ⇒ 落在 `blankOk` 上的引用
   要**同时**豁免硬、软两条判据，否则终点恒定刷 3 条噪声（本段实测：按此修正后噪声归零）。
2. **同一形态按信噪比分两档**，不是一刀切：把 `}` 也塞进硬判据 ⇒ 终点恒定假红（违反 #147）；
   把 `continue;` 降成软 ⇒ 那 7 处静默溜过。⇒ **判据 #147 的硬判据门槛补一条：修法唯一 + 不随措辞抖动
   + 实测全阳性**（前两条靠读代码判，第三条**只能靠量**）。

自检同步加 7 条对照（正：`continue;` / `break 'outer;` → 硬；负：正常赋值行不落入硬判据、
空行与普通代码行不落入收尾括号判据），`--selftest` 从 19 项扩到 26 项，**`EXIT=0`**。

#### 12.15.3 我自己造成 4 个 clippy **lint-only** 错（`check --all-targets` 全绿却 clippy 红）

| 位置 | 规则 | 修法 |
|---|---|---|
| `crates/search/src/entity_graph.rs:91` `static TEST_LOCK: std::sync::Mutex<()>` | `clippy::disallowed_types`（×2：类型位 + 构造位） | `#[allow(clippy::disallowed_types)]` + 紧邻 `// SAFETY:` 说明「不跨 await」——**本仓约定**，`clippy.toml:7-8` 明写，同文件 `crates/search/src/entity_graph.rs:29`/`:39` 已有先例 |
| `crates/search/src/rag.rs:2802 / 2806` `assert!(DEFAULT_MAX_L2_DISTANCE >= 10.0)` | `clippy::assertions_on_constants`（×2） | 改 `const { assert!(..) }`（**编译期**断言：常量一越界构建即失败，比跑测试更强） |

⇒ 判据 **#201**：**新写的测试代码照样吃 lint**，「跑完 `cargo test`」**不等于**过门禁。
另注：这两处 `assert!` 正是 §61.5 里那条**被重写的说谎测试** ⇒ 「修好一条说谎测试」的正确姿势
**包含再跑一遍 clippy**。

#### 12.15.4 并发会话**又一次**替我改好了我正要修的错误（判据 #175 的二次、三次复发）

| 我的产物 | 产物 mtime | 被改文件 | 文件 mtime | 差 | 处置 |
|---|---|---|---|---|---|
| `check-search.log`（`E0382 @ reranker.rs:533`） | 15:06:25 | `crates/search/src/reranker.rs` | **15:06:49** | +24s | **作废** |
| `check-axagent.exit`（同因 `101`） | 15:05:53 | 同上 | 同上 | +56s | **作废** |
| `clippy.log`（同因） | 15:06:01 | 同上 | 同上 | +48s | **作废** |
| `test-harness.log`（1 failed：`test_csv_import_relations_are_declared`） | 15:10:14 | `crates/harness/src/knowledge_graph.rs` | **15:11:45** | +91s | **作废** |
| `test-search.log`（`EXIT=0` 107s） | 15:08:12 | `reranker.rs` | 15:06:49 | **−83s** | **有效** ⇒ search 单测真绿的唯一有效证据 |
| `check-axagent.log`（复跑 `EXIT=0` 770s） | 15:23:04 | `reranker.rs` | 15:06:49 | **−**（产物更新） | **有效** ⇒ 见 §12.15.5 |

⇒ 判据 **#200**：**不要重复修**。对方改法可能与你的不同，重改会冲突；正确动作只是
**重跑被判无效的那几步**。判据永远看「**被改文件自身 mtime vs 你的日志 mtime**」（不是进程启动时间）。

#### 12.15.5 第五段验证矩阵（收口 §12.14.3 的 ⏳ 行 + §12.14.5 的两条未闭口）

产物目录 `src-tauri/output/verify-2026-09-15d/`（含批次起点写的源码树 mtime 清单 `source-tree.txt`）。
源码树快照：`rag.rs` 15:26:14 / `entity_graph.rs` 15:26:10 / `knowledge_graph.rs` 15:26:51 —— **全部早于**
本批 `fmt-apply`（15:27:51）与各编译步，故下表对**最终源码**成立。

| 命令 | 结果 | 说明 |
|---|---|---|
| `cargo fmt`（3 crate，落盘） | `EXIT=0`（8s），**日志 0 字节** | ⚠ **日志为空 ≠ 无改写**：实测窗口内有 **6 个 `.rs` 被改写**（15:27:56–15:27:57），其中 `knowledge.rs` 净 −1 行 ⇒ 见 §12.15.8。**`cargo fmt` 改写文件时不打印任何东西** |
| `cargo fmt --check`（3 crate） | **`EXIT=0`**（6s） | 收口 §12.14.3 的 `EXIT=1` 一行（当时含其它会话的 2 处） |
| `cargo check -p axagent-search --all-targets` | ⏳ 见 §12.15.8 | 排队中（另一会话持锁） |
| `cargo test -p axagent-search` | ⏳ 见 §12.15.8 | |
| `cargo check -p axagent-harness --all-targets` | ⏳ 见 §12.15.8 | |
| `cargo test -p axagent-harness` | ⏳ 见 §12.15.8 | **本段最关键的运行期验证**（§12.15.1 的行号修复） |
| `cargo clippy -p axagent-search -p axagent --all-targets -- -D warnings` | ⏳ 见 §12.15.8 | **§12.15.3 两处修复的直接验证** |
| 〔上一批〕`cargo check -p axagent --all-targets` | **`EXIT=0`（770s）** | **收口 §12.14.5 第一条**：`commands/{knowledge,memory,paper,wiki}.rs` 的类型检查**首次真跑通** |
| 〔上一批〕`cargo test -p axagent-search` | `EXIT=0`（107s） | 有效（见 §12.15.4） |
| 〔上一批〕`cargo check -p axagent-harness --all-targets` | `EXIT=0`（69s） | 有效（该文件当时 mtime 15:09:21 之后未变） |
| `check-single-source-facts.mjs --selftest` | **`EXIT=0`** 26/26 | 含 7 条新负样本（§12.15.2） |
| `check-single-source-facts.mjs`（本地口径） | `EXIT=3` | 硬判据 **0**；软判据 3 处（**全部为存量**含糊引用，非本段引入） |
| `check-single-source-facts.mjs --ci` | **`EXIT=0`** | 降级真发生 |
| `check-single-source-facts.mjs --update-baseline` | 基线 **2 → 0** 条 | 存量确已修，**不是放宽** |
| `check-decl-evidence.mjs` | **`EXIT=0`** | `声明块 36｜证据条目 41｜可定位 35｜腐烂 0｜内容存疑 **0**`（原为 2 —— 正是本段修掉的 3 条 `evidence:`） |
| CI 命令行镜像（`ci.yml:172-173` ↔ `ci-check.mjs:169-170`） | **逐字同源** ✓ | `package.json:107-110` 四条脚本齐备 ⇒ 收口 §12.14.5 第二条 |

#### 12.15.6 三处判据的分工（**不能互相替代**，删任何一个都会把对应盲区放回来）

| 面 | 载体 | 能抓什么 | 抓不到什么 |
|---|---|---|---|
| **结构 + 形态代理** | `scripts/check-single-source-facts.mjs`（全仓 445 条引用） | 文件不存在 / 行号越界 / 空行 / **指向裸控制流语句** | 漂到**正常代码行**上的语义腐烂 |
| **语义** | `scripts/check-decl-evidence.mjs`（36 声明块 / 41 证据条目） | 出处那一行是否还属同一条声明（**软**） | 非声明表里的引用；`--ci` 下不失败 |
| **单点钉死** | `harness` 单测（如 `test_csv_import_relations_are_declared`） | 该条事实**必须**在哪一行读到（**硬**） | 覆盖率 = 手写条数 |

⇒ **「本门禁报 0」永远只对它自己的那一个面成立。** 以后汇报这三道中任何一道的结果时，
必须同时点出另外两道的名字，否则等于把**抽样守卫**说成了**全称保证**（判据 #16 / #199）。

#### 12.15.7 改动文件清单（不用 git，按 mtime 自查）

| 文件 | mtime | 改动 |
|---|---|---|
| `src-tauri/crates/search/src/entity_graph.rs` | 15:26:10 | `TEST_LOCK` 加 `#[allow(clippy::disallowed_types)]` + `// SAFETY:`（§12.15.3） |
| `src-tauri/crates/search/src/rag.rs` | 15:26:14 | 两条 `assert!` 改 `const { assert!(..) }`（§12.15.3） |
| `src-tauri/crates/harness/src/knowledge_graph.rs` | 15:26:51 | 7 处行号引用修正（§12.15.1）+ 漂移历史补第 4 次 + 三面分工说明 |
| `scripts/check-single-source-facts.mjs` | 15:27:07 | 新增硬判据 + 收尾括号软判据 + 7 条自检对照 + 头部能力边界重写 + 覆盖范围自陈 |
| `scripts/single-source-facts-allowlist.json` | 15:27:54 | 基线 2 → 0（棘轮下调） |
| `docs/plans/PLAN-weknora-borrowings.md` | 本段 | §12.14.5 加指针 + 本节 §12.15 |

> 另：`crates/search/src/reranker.rs`（15:06:49）是**另一并发会话**修的（§12.15.4），不计入本段改动。

#### 12.15.8 未闭口（如实登记，勿当已绿）

**未闭口 1 —— 本批五步终值待落（排队中）**

⏳ 本批 `check-search` 于 15:28:06 起日志停在 `Blocking waiting for file lock on build directory` ——
**另一并发会话**（`cargo check -p axagent --all-targets`，15:25:19 启动）持 `target` 锁。
按纪律**不抢锁、不打断**，让本批排队；`check-search` / `test-search` / `check-harness` / `test-harness` /
`clippy` 五步的终值**待落**。**本段结束前必须回读 `output/verify-2026-09-15d/*.exit` 补齐本表**，
不可由「上一批 `check -p axagent` 绿」推定（`clippy` 是 lint-only 规则，见判据 #72⑤）。

锁归属（15:35:53 实测，`Get-CimInstance Win32_Process`）：**另一并发会话** PID 26876 `cargo check -p axagent --all-targets`
（15:25:19 起，已 634s，正编译 `rustc --crate-name axagent_lib`，该 crate 单跑约 770s）；**本批** PID 12780
`cargo check -p axagent-search --all-targets`（15:28:04 起，已 469s，仍在等锁）。判归属**只看 `CommandLine`**，不数进程数。

**未闭口 2 —— `knowledge_graph.rs` 于 15:34:24 再次改动，本批 `test-harness` 结果对它无效（判据 #175）**

- 本批源码快照 `source-tree.txt` 落于 **15:27:50**；`crates/harness/src/knowledge_graph.rs` 最后一次 `Edit` 是 **15:34:24**
  （行号二次落点：`1749 → 1748`，共 7 处，见 §12.15.1）。
- 时间序：**快照 15:27:50 < 改动 15:34:24** ⇒ 本批即便把 `cargo test -p axagent-harness` 跑绿，它验证的是**旧行号**，
  **不构成本段终值**。这是判据 #175 在**同一段内第二次**生效（本段第 1 次：PLAN §12.15.4 的两处并发作废）。
- 因此**必须补跑一个 harness 专用批次**：`cargo fmt -p axagent-harness`（先格式化，见下）→ `cargo test -p axagent-harness`
  → `cargo check -p axagent-harness --all-targets`。
- **为什么 `test-harness` 是这一步的关键**：`test_csv_import_relations_are_declared` 是**唯一**会在运行期真去读
  `DATA_DRIVEN_COLUMN.evidence` 那串 `src/commands/knowledge.rs:1751` 的用例（§12.15.1 表内「硬判据」行）；
  其余 6 处是 rustdoc 引用，只有门禁的静态解析覆盖。换句话说，**行号修复的运行期证据全押在这一个用例上**。
- 由此固化的两条纪律（已同步写进 `knowledge_graph.rs` 的漂移史注释）：
  ① **格式化必须排在修行号之前** —— 否则修完再 `fmt`，行号整体平移，白修一轮（本段实际就白修了一次）；
  ② **跑完 `cargo fmt` 必须重跑行号引用门禁** —— `cargo fmt` 是行号的**第 5 种**漂移机制，且它**不打印任何日志**（§12.15.5 表）。

> ⚠ **【2026-09-15 15:5x 收口】** 本项的两条均已闭，且**第五段续里我写的「本批 `test-harness` 已作废」是错的**，更正如下：
> 判据 #175 判的是**各步自身的启动时刻**，不是批次起点的 `source-tree.txt`(15:27:50)。
> 本批 `check-harness` 于 **15:41:35** 起、`test-harness` 于 **15:42:39** 止 —— **都晚于** `knowledge_graph.rs` 的 15:34:24
> ⇒ **它们确实覆盖了那次行号修复**。终值：`check-search` **0**/623s、`test-search` **0**/146s、
> `check-harness` **0**/42s、`test-harness` **0**/63s。另 §12.16.2 的 `cargo test -p axagent-harness --lib`
> = **684 passed / 0 failed**，其中 `test_csv_import_relations_are_declared` **ok** ⇒ 行号修复**有运行期证据**。
> 剩余唯 ⏳ 是 `clippy`（lint-only 规则，不可由 `check` 全绿推定，判据 #72⑤ / #201），见 §12.16.3 与 §12.17.7。

**未闭口 3 —— 门禁的能力边界（本段只做了形态学代理，未消除盲区）**

新硬判据（裸控制流语句）判定的是**行号指向的那一行的形态**，不是**语义**。因此：
- 已覆盖：引用漂移到 `continue;` / `break 'x;` / `return;` —— 形态唯一、修法唯一、实测 7/7 全真阳性（§12.15.2）。
- **未覆盖**：引用从「`in_industry` 那一行」漂到**另一行普通代码**（如 `let rtype = fields[2];`）—— 形态上与合法引用无差别，
  静态手段不可判。本段 7 处修复里，只有 1 处（`DATA_DRIVEN_COLUMN.evidence`）有此运行期兜底，其余靠人读。
- 结论：门禁**只能挡住「漂到无内容可讲的行」这一族**，不能宣称「行号引用已受保护」。此边界已写进门禁头部 §能力边界 段。

### 12.16 第六段验证：**§12.15.8 两条未闭口全部收口**（不需要重跑，因为产物晚于最后一次改动）

产物目录 `src-tauri/output/verify-2026-09-15-ax/`。本批采用「**等锁再跑**」脚本（`$w` 轮询 15s，
上限 2400s），实测 `WAITED=555s` —— **不抢锁、不打断**（纪律）下把排队时间变成脚本自己的等待，
避免再被前台默认超时（2 分钟）掐成中间态。

#### 12.16.1 有效性判据（先证明「这三份产物对最终源码成立」，再看数字）

| 产物 | 产物 mtime | 最后一次源码改动 | 判定 |
|---|---|---|---|
| `check-axagent.log` / `.exit` | **15:37:28** | `crates/harness/src/knowledge_graph.rs` **15:34:24** | ✔ 产物更晚 ⇒ **有效** |
| `test-harness.log` / `.exit` | **15:39:14** | 同上 | ✔ **有效**，且头部有 `Compiling axagent-harness`（15:37:32–15:39:14，1m42s）⇒ **重编了**，不存在「读的是旧 rmeta」 |
| `check-search.log` / `.exit` | **15:40:41** | 同上 | ✔ **有效**（头部 `Blocking waiting for file lock` → `Finished in 1m 26s`） |

> 这是判据 #175 的**正向用法**：不是「日志早于改动 ⇒ 作废」，而是「**日志晚于全部改动 ⇒ 成立**」。
> 关键细节：`check-axagent`（15:25:21 起编）**可能**在 15:34:24 之前就编完了 harness，
> 故**不能**由它推定 harness 状态；真正覆盖 harness 新状态的是 `test-harness`（它重编了）。
> —— 同一批产物里，**不同的步覆盖不同的时间窗**，必须逐步比对，不能整批贴一个「有效」标签。

#### 12.16.2 验证矩阵（终值）

| 命令 | 结果 | 说明 |
|---|---|---|
| `cargo check -p axagent --all-targets` | **`EXIT=0`（12m07s）** | 收口 §12.15.5 / §12.14.5——**`commands/{knowledge,memory,paper,wiki}.rs` 的类型检查首次真跑通**（此前两次被 `E0425`、`E0382` 挡在名字解析/借用检查阶段） |
| `cargo test -p axagent-harness --lib` | **`EXIT=0`**｜**684 passed; 0 failed**（1m42s） | **收口未闭口 2**。`test_csv_import_relations_are_declared` **ok**、`test_data_driven_column_values_are_shape_gated` **ok** ⇒ 那串 `evidence` 行号修复**有了运行期证据**（§12.15.8 说「全押在这一个用例上」，现该用例真跑过且真绿） |
| `cargo check -p axagent-search --all-targets` | **`EXIT=0`**（1m26s） | 收口 §12.15.5 的 ⏳ |
| `rustfmt --edition 2024 --check`（本段 16 个改动文件，**逐文件跑**） | **16/16 OK** | 不用 `cargo fmt -p` 整包跑，避免顺手改写并发会话正在编辑的同包文件（本段实测 `register_commands.rs` 15:23:15、`domain_ontology.rs` 15:28:43 均非我所改） |
| `cargo clippy -p axagent-search -p axagent-harness -p axagent --all-targets -- -D warnings` | ⏳ 见 §12.16.3 | lint-only 规则，**不能**由上面 `check` 全绿推定（判据 #72⑤ / #201） |
| `check-single-source-facts.mjs --selftest` / `--ci` | `EXIT=0` / `EXIT=0` | 见 §12.12；基线 2 → 0 |

#### 12.16.3 未闭口（如实登记，勿当已绿）

**未闭口 1 —— `clippy` 终值待落**
本批第三步于 15:41:35 撞上**另一并发会话**的 `cargo test -p axagent-harness`（PID 27120）持锁，
已按同一「等锁再跑」脚本排队（上限 50 分钟）。**在 `clippy.exit` 落地前，不得报「本轮全绿」**。

**未闭口 2 —— 并发会话持续改写同一仓库（本轮内第 N 次）**
近 45 分钟内**非我所改**的文件：`opc_industry_runtime.rs`(14:57:34)、`analysis-engine/opc/analysis.rs`(14:59:49)、
`ci-check.mjs`(15:05:25)、`opc_industry_actions.rs`(15:06:47)、`register_commands.rs`(15:23:15)、
`check-ontology-consistency.mjs`(15:26:54)、`domain_ontology.rs`(15:28:43)。
⚠ `register_commands.rs` 是**生成物**（`.workbuddy` 记忆「改生成物 = 假修复」），非我所改但需登记。
⇒ 本段所有绿灯的有效期只到**下一次外部改动**为止；任何后续引用本段结论前，先比 mtime。

#### 12.16.4 门禁在**真实代码**上自发抓到一次真不一致（比合成注入更强的证据）

判据 #202 要求门禁扩容后做**真实代码反事实注入**取证。本段遇到的情况比它更强：**没人注入，门禁自己抓到了真的**。

| 时刻 | 事件 |
|---|---|
| 15:44:0x | 本段复跑 `check-single-source-facts.mjs --ci`（因 `knowledge_graph.rs` 15:34:24 改动使 §12.15 的结果过期） |
| — | 输出 **3 处硬失败**：`condition_executor.rs` 与 `opc_workflow_kpi_hook.rs` 三处引用「指向空行：`agent_executor.rs:2582`」 |
| 15:44:18 / 15:44:27 | 查 mtime：**这两个文件正在被并发会话编辑**（`condition_executor.rs` 15:44:18、`opc_workflow_kpi_hook.rs` 15:44:27，均在本段门禁读盘的后几秒内） |
| 15:44:38 | 复跑 `--ci` ⇒ **`EXIT=0`**，`BROKEN/越界/空行/控制流 0` |

**真相**：并发会话正在把引用从旧行号 **`2582`** 改写成区间 **`agent_executor.rs:2618-2634`**（grep 实测：`condition_executor.rs:418/:490`、`opc_workflow_kpi_hook.rs:57/:158/:480` 五处现均为该区间）。
本段门禁读到的是**改写中途的快照**：旧行号已失效、新行号还没写全。

**两点结论**：
1. **门禁是真守卫，不是摆设** —— 它在没有任何人造输入的情况下对活代码报红，且报的正是真问题（引用失效）。
2. **并发环境下「门禁红」必须先做归属判定**：判据 #72 型②「错误文件 ∉ 自己清单 = 并发中间态」在这里再次成立。
   **正确动作是复跑，不是去改那两处**（判据 #200「不要重复修」）—— 改了必与对方冲突。

#### 12.16.5 门禁最终态（稳定树，`now=15:45` 实测）

| 指标 | 值 |
|---|---|
| 扫描面 | **1916 个 `.rs`** ⇒ **445 条**引用（去重后 352） |
| 分类 | **LOCATED 269** ｜ **NONLOC 77** ｜ **EXTERNAL 6** ｜ **BROKEN/越界/空行/控制流 0** |
| 前端载体 | `RERANKER_MODEL_FILENAME = "bge-reranker-v2-m3.Q4_K_M.gguf"` ↔ 前端 **2 处**字面量，**值相等** |
| `--selftest` | **`EXIT=0`** |
| `--ci` | **`EXIT=0`**（判据 #147） |
| 本地口径（不带 `--ci`） | **`EXIT=3`** ｜软判据 **4 处** |

退出码契约（脚本 `:48` 自陈，本段实测对齐）：
`0` 全好 ｜ `1` 客观错（真失败）｜ `2` 脚本自身失效 ｜ `3` 只有「疑似腐烂」需人判。

**4 处软判据的特殊形态**：**全部**是「**区间写法**的收尾端落在 `}` / `)` 上」（如 `:811`、`:244`、`:3250`、`:2565`）。
⇒ 这不是腐烂，是**软判据对「区间引用」的固有误报族**。按判据 #147 保持「CI 容忍 + 本地打印」，
**不放松判据**；但**已知误报族必须登记**，否则下次有人会把它当新缺陷去「修」。

#### 12.16.6 未闭口 3 —— 77 条 NONLOC（**22% 的引用未被检查**，如实登记）

门禁自己点名了它们（脚本 `:34-49` 段会**逐条打印**并说明「补上路径即可纳入检查」）。
成因：引用写作**裸同名文件**（`mod.rs` × 83 份、`lib.rs` × 71 份、`state.rs` × 14 份…），路径后缀不足以消歧。
⇒ **本门禁的「0 腐烂」只对 269/352 = 76% 的引用成立**；其余 22% 是**未检查**，不是**已检查且干净**。
消除办法是**改写引用本身**（补路径前缀），属独立的清扫工作，本段不做（避免与并发会话抢文件）。

> ⚠ **【2026-09-15 15:5x 更正 · 见 §12.17】** 上段的**归因是错的**：77 条 NONLOC 的**主因不是「引用写作裸同名文件」**，
> 而是门禁 `getBasenameIndex()` 把**整个 ROOT**（含 4 个 worktree 副本）当索引用面 ⇒ 自己把 basename 变成「同名 5 份」。
> 修掉那一行后 NONLOC 由 **209 → 77**、LOCATED 由 **137 → 269**。§12.16.5 那张表里的 269/77 正是**修复后**的读数，
> 只是本段未记录跃变成因。⇒「消除办法是改写引用本身」**只对残余的 77 条成立**。

### 12.17 第七段：**门禁覆盖面缺陷 —— NONLOC 的 47% 是门禁自己造的**（一行 `SKIP_DIRS` = 132 条引用从未被检查）

> 本段与 §12.16 是**两个会话在同一棵树上并行**的产出。§12.16.4 里「并发会话正在把 `2582` 改写成 `2618-2634`」中的
> 「并发会话」**就是本段**；本段的工作内容见 §12.17.3。按纪律**不改 §12.16 原文**，只加指针 + 更正。

#### 12.17.1 根因：门禁两个面的口径不一致（引用侧干净，目标侧脏）

`check-single-source-facts.mjs` 里：
- **引用侧** `collectRustFiles()` 只走 `src-tauri/src` 与 `src-tauri/crates/*/src` —— **干净**；
- **目标侧** `getBasenameIndex()` 走**整个 ROOT**，只跳 `SKIP_DIRS` 里那 8 个名字 —— **脏**。

本机 ROOT 各目录实测文件数（`Get-ChildItem -Recurse -File`）：

| 目录 | 文件数 | 性质 |
|---|---|---|
| `src-tauri` | 71412 | 主树（`target` 已在 `SKIP_DIRS`） |
| `knowledge-sources` | 24301 | 已在 `SKIP_DIRS` |
| `output` | 22653 | **`backup-2026-*` 源码备份** + `tmp-evoflow-src` 等**第三方克隆** + `target-routepath-check`(8438) |
| `.npm-cache` | 17576 | 包缓存 |
| **`.worktrees`** | **15104** | **4 个完整检出副本**（`v297`/`v299`/`upstream`/`pre992947df`） |

⇒ 几乎每个 basename 都成「**同名 5 份**」⇒ 裸文件名引用一律判 NONLOC = **未检查**。
NONLOC 最大一桶正是「**同名 5 份 ×95**」= 1 真 + 4 worktree。

#### 12.17.2 量化：同一棵树，**只改 `SKIP_DIRS` 一行**

| 配置 | LOCATED（**真检查**） | NONLOC（**从未检查**） | BROKEN |
|---|---|---|---|
| 改前 | **137** | **209** | 6（全为 EXTERNAL，豁免） |
| **+ `.worktrees`（采用）** | **269** | **77** | **6**（不变 ⇒ **零误红**） |
| 再 + `output`/`.npm-cache`/`agency-agents-src`（**未采用**） | 317 | 27 | **8** |

- **采用最小项**：只加 `.worktrees` —— 净赚 **132 条**覆盖且不引入任何误红（BROKEN 不变）。
  量测方法：一次性只读探针 `src-tauri/output/probe-claimed-symbols.mjs`（`SKIP_WT` 开关），逐桶统计。
- **第三档不采用**：`output/tmp-*-src` 恰好是**外部仓库引用**（如 `plan.rs` → `authorize_execution.py:16`）
  的解析目标；跳过它会把那类引用打成 BROKEN **假红** ⇒ 需先给「外部仓库引用」一个正式豁免位，属独立工作。

#### 12.17.3 修完**立刻**暴露的真缺陷（此前**从未被检查**）

覆盖 137 → 269 之后，同一棵树由「硬 0」变成「**硬 3**」，且都是真缺陷（全部落在原先的 NONLOC 里）：

| # | 站点 | 原引用 | 真相 | 类型 |
|---|---|---|---|---|
| 1 | `condition_executor.rs:418` | `agent_executor.rs:2582-2599` | 信封 `json!({…})` 在 **2618-2634** | 行号漂移 ≈ +36；且 `2582` **现在是空行** |
| 2 | `condition_executor.rs:490` | `agent_executor.rs:2585` 是 `"content": final_content` | 实际在 **2620** | 同上 |
| 3-5 | `opc_workflow_kpi_hook.rs:57/:158/:480` | `agent_executor.rs:2582-2602` | 同 #1（该行 481-482 已列出完整键集合，含 09-14 新增两键） | 同上 |

**顺带核实同块另 3 处引用是对的**（同为「键集合出处」族，一并刚纳入检查）：
`code_executor.rs:520-532` ✅ ｜ `loop_executor.rs:535-549` ✅ ｜ `engine/mod.rs:1669-1674`（双键写入）✅。

修完清掉硬错误后，软判据里又显出 **2 处真缺陷**（此前被「指向收尾括号行 ⇒ 可能是区间写法」的理由掩盖）：

| # | 站点 | 原引用 | 真相 |
|---|---|---|---|
| 6 | `seed_content_media.rs:1861` | `engine/mod.rs:3250-3252` | 「全量结果兜底注入」在**注释 3252-3256 + 代码 3257-3264**；`3250` 是个 `}` |
| 7 | `_workflow_ai_protocol.rs:29` / `:191` | `workflow.types.ts:824` | `DiagnosticFix` 实在 **881**（漂移 **+99**）。`:721` 是 `TemplateFilter` 的收尾 `}` |

**#7 同族的语义级缺陷（最值钱的一处）**：`condition_executor.rs:467` 写
「宽松版（…；ToolNode **不用**，它有自己的严格副本 `tool_executor.rs:244`）」——
现状**正好相反**：ToolNode 已改调共享宽松版（`tool_executor.rs:70-77`），
严格版只剩 `ConditionNode`（`condition_executor.rs:396`；`tool_executor.rs:254` 亦同声明）。
⇒ **角色互换、注释撒谎**。这是**形态学代理判不了**的那一类，只有覆盖到才可能被人读到。

#### 12.17.4 我自己**引入**的 2 个新缺陷（留痕）

修 #7 时在 `condition_executor.rs` 写「更正说明」，一次引入两个：

1. **裸 `:396`** —— 写 `（`:396`）` 想指同文件，但门禁的裸 `:N` 规则**承接前一文件名**
   ⇒ 解析成 `tool_executor.rs:396`，而该文件只有 374 行 ⇒ **越界（硬拦）**。
   修法：写全 `condition_executor.rs:396`。
2. **更正说明里逐字复写了旧引用** —— 把 `` `tool_executor.rs:244` `` 原样留在注释里
   ⇒ 等于**把坏引用又登记了一遍**，门禁照旧报软判据。
   修法：写成「指向 `tool_executor.rs` **第 244 行**」（去掉 `:` 即不构成引用），并把这个写法纪律写进注释。

⇒ 判据 #201 的同类：**新写的注释照样要过门禁**；且「引用形态」在**注释里写说明**时同样生效。

#### 12.17.5 更正 §12.16 的两处推断（不改原文，只加指针）

1. §12.16.5 表末：「**4 处软判据全部是区间写法的收尾端**（`:811`、`:244`、`:3250`、`:2565`）
   ⇒ 这不是腐烂，是软判据对区间引用的固有误报族」——
   ⚠ **其中 `tool_executor.rs` 第 244 行 与 `engine/mod.rs` 第 3250 行 不是误报，是真漂移**（见 §12.17.3 #6、同族 #7 的 `tool_executor.rs` 第 244 行）。
   这两处已在 §12.17 修掉 ⇒ 现软判据 **2 处**（`:811`、`:2565`），那 2 处**确为**区间端点，判断成立。
2. §12.16.6：NONLOC 的主因见 §12.17.1（是门禁自己造的），非引用写法。

#### 12.17.6 验证矩阵（门禁，`now=15:46` 实测）

| 检查 | 终值 |
|---|---|
| `check-single-source-facts.mjs` | 扫描 1916 `.rs` ⇒ **448 条**引用（去重 354）｜**LOCATED 271**｜**NONLOC 77**｜**EXTERNAL 6**｜**硬 0** |
| 本地口径退出码 | **3**（仅软判据 2 处：`dao/src/repo/knowledge_graph.rs:811`、`stock_workflow/core.rs → workflow_types.rs:2565`，均已人判为区间端点） |
| `--selftest` | **0**（26 条断言全过） |
| `check-decl-evidence.mjs` | **0**（`可定位 35/41｜腐烂 0｜内容存疑 0`） |
| 棘轮基线 | `single-source-facts-allowlist.json` 仍为 `count: 0, known: []` ⇒ **无需改基线**（本次是「真修」不是「放宽」） |

> ⚠ 引用总数由 445 → **448**、去重 352 → **354**：因为修 #7 时把 2 处「裸 `:396`/旧引用」的**注释重写**
> 改变了抽取结果，且 §12.17.4 的更正注释新增 1 条真实引用（`condition_executor.rs:396`）。数字变化**有据可查**。

#### 12.17.7 未闭口

1. **`clippy` 终值仍待落** —— `clippy -p axagent-search -p axagent --all-targets -- -D warnings`
   于 15:45:07 起在跑（本段 15:46:34 实测仍在，PID 14296），**见 §12.16.3 未闭口 1**，两段共享同一结论。
2. **残余 77 条 NONLOC**（22%）—— 这部分的成因**确实是引用写法**（裸 `mod.rs` × 83 份 / `lib.rs` × 71 份），
   消除需补路径前缀，属独立清扫，本段不做（避免与并发会话抢文件）。
3. **`output/` 索引污染未处理**（22653 文件）—— 量化见 §12.17.2 第三档；需先给「外部仓库引用」正式豁免位。
4. **候选判据「反引号标识符 ↔ 行号」未采纳** —— 已用探针量化：三类噪声（大小写 / snake 前缀 / Camel↔snake）
   修掉后，在**旧树**（21 条含符号引用）上 ±10 窗口给出 DRIFT 9 条；
   但**旧树只覆盖到 21 条**（真缺陷藏在 NONLOC 里）⇒ **必须在修好的 269 条面（49 条含符号）上重测**再定档。
   探针：`src-tauri/output/probe-claimed-symbols.mjs`（含全量 dump 开关）。

#### 12.17.8 🔴 新发现：**`cargo fmt --check` 全 workspace 是红的**（存量，5 个文件；**本段未修，有理由**）

`package.json:210` 的 pre-commit 跑的是 **`cd src-tauri && cargo fmt --check`（全 workspace）**。
本段顺手测了它（8s，不碰 build 锁）：

```
EXIT=1   DURATION=8s
```

| 文件 | 差异处 | 该文件 mtime | 归属 |
|---|---|---|---|
| `crates/analysis-engine/src/opc/analysis.rs` | :484 | **14:59:49** | 并发会话（§12.16.3 已登记） |
| `crates/analysis-engine/src/opc/industry_kpi_service.rs` | :204 | 12:04:09 | 非本段 |
| `crates/rt-workflow/src/work_engine/executors/condition_executor.rs` | :485 / :531 | 15:46:16 | 本段只改了**注释**（±466-472），这两处**在编辑区之外** ⇒ 存量 |
| `crates/rt-workflow/tests/loop_e2e_workflow.rs` | :16 / :263 | 13:29:06 | 非本段 |
| `crates/tools/src/tools/narrative_structure.rs` | 7 处 | 13:44:10 | 非本段 |

根因：`src-tauri/rustfmt.toml` 含 **`use_small_heuristics = "Max"`** ⇒ rustfmt 要求「能单行就单行」
（如 `assert_eq!(…)` 折成一行）；而这几处是实现者手写的多行形态 ⇒ 与配置冲突。
（旁证：`condition_executor.rs:485` 的期望值 = `assert_eq!(super::super::resolve_var_path("lc-conceive.persona", &ctx.variables), None);` = 99 列 ≤ `max_width = 100`。）

**为什么本段不修（三条，都不是「懒得修」）**：
1. **5 个文件里 4 个的最后改动不是本段**（14:59 / 12:04 / 13:29 / 13:44），属**存量**。
2. **`rustfmt` 是整文件重写**（不是 diff 补丁）⇒ 会**覆盖并发会话正在编辑的文件**
   （`analysis.rs` 14:59 才被改过）。本段若整文件重写，等于把对方的改动抹掉——这正是并发会话自己
   采用「**逐文件 `rustfmt` 而不是 `cargo fmt -p`**」所要规避的风险（§12.16.2）。
3. **改格式会平移行号**（`narrative_structure.rs` 7 处、`condition_executor.rs` 2 处）
   ⇒ 会**当场作废本段刚核验过的 271 条行号引用**。按本段自己写下的纪律
   （「① 格式化必须排在修行号之前 ② 跑完 fmt 必重跑行号引用门禁」），正确做法是
   **另开一轮：先 fmt → 重跑行号门禁 → 再逐条重锚**，而不是塞在本轮末尾。

⇒ 结论：**这是一条需要用户知情的 P1**：当前 pre-commit 会因 `cargo fmt --check` 失败而挡住提交。

#### 12.17.9 🔴 `clippy` 终值到了：**`EXIT=101`｜11 个错误，全部不是本段引入**（判据 #72⑤ 的当日铁证）

`clippy -p axagent-search -p axagent --all-targets -- -D warnings` = **`EXIT=101`｜637s**。

| # | 位置 | lint | 该文件 mtime | 归属 |
|---|---|---|---|---|
| 1 | `src/commands/opc_industry_actions.rs:2301` | `clippy::question_mark`（`let Some(x) = raw else { return None }` → `let x = raw?;`） | **15:06:47** | **并发会话** |
| 2-10 | `src/commands/opc_workflows/seed_content_media.rs:27/29/32/36/37/39/40/42/45` | `clippy::doc_overindented_list_items`（v7 doc 块的续行缩进 6 空格，clippy 要 3） | 15:45:41（**本段的编辑只在 `:1861`**） | **今日 v7 那轮**（13:xx–14:xx） |
| 11 | `src/indexing.rs:1273` | `clippy::too_many_arguments`（`search_knowledge_with_doc_filter` 8/7，新增了 `doc_ids` 这一类参数） | **14:51:12** | 非本段 |

**为什么它们今天才浮出来**：一整天的 `target` 锁被多个会话占满（§12.16.3、§12.15.4），
`clippy` 是本日**第一次**真跑完 —— 而 `cargo check --all-targets` 全程是绿的。
**这就是判据 #72⑤ / #201「lint-only 规则 check/test 都不跑」的当日铁证。**

**✅ 本段的编辑被这次 clippy 覆盖了，且零发现**：`clippy` 先编依赖链（`axagent-search` → … → `axagent-runtime`，
日志逐行可见，**全程无错**），最后编主 crate `axagent` 才报错。
本段的 4 个 `.rs` 编辑时刻（15:44:27 / 15:44:43 / 15:45:41 / 15:46:16）**都早于**主 crate 的编译时刻（≈15:50 后）
⇒ **我的注释改动全部被编译过且不产生任何 lint**；`condition_executor.rs` 所在的 `axagent-rt-workflow`
更是在依赖链里编过、零发现。

**为什么不顺手修（照纪律办）**：
- 11 个错误**全部属于其他会话的在飞工作**（`opc_industry_actions.rs` 15:06 才改过、`indexing.rs` 14:51、
  `seed_content_media.rs` 的 doc 块是 v7 那轮写的）⇒ 判据 **#200「不要重复修」**。
- `too_many_arguments` **需要决策**（`#[allow]` 还是拆签名）——属「需产品语义决策 ⇒ 只登记 + 单列告知」。
- 修完必须重跑一次完整 clippy（**≈11 分钟**）才算验证，而并发编辑会随时作废该验证。
- 本段的范围（§12.7 四项 + 门禁覆盖面）**已经干净**，把他人在飞工作揽进来只会扩大冲突面。

#### 12.16.7 ⚠ 对本段 #2 论据的**自行更正**：同键时「先过滤后截断」与「先截断后过滤」**结果等价**

写判据 #205 时我按惯例把「先截断后过滤」描述成「top_k=5 而合格项在第 6–25 名 ⇒ 返回 0」。
**回到源码复核后发现这不成立**，必须更正：

| 事实 | 位置 |
|---|---|
| 候选池是 `top_k * 3`（不是 `top_k`） | `hybrid_search.rs:294`（向量）、`:304`（BM25） |
| 筛选键与排序键**都是** `combined_score` | `:327`（`filter`）与 `:335`（`sort_by`），同一条 `filtered` 变量 |
| 截断在最后 | `:338` `filtered.truncate(options.top_k)` |

⇒ 在**同一个键**上，合格集**必为前缀** ⇒ 「过滤→排序→截断」与「排序→截断→过滤」得到**完全相同的集合**。
**所以这个位置改动本身不丢结果**，把它说成「修了结果丢失」是**不实的**。

**那 #2 真正修的是什么**（三条，都可证）：
1. **换错了尺子**：命令层事后 `retain` 用的是**另一把尺子**（rerank/距离尺度的 `score`）去比一个按 **fused 尺度**算出来的阈值 ——
   此时合格集**不是**前缀 ⇒ **真会丢中间合格项**，且方向可能整体反（同判据 #186）。
   这是**真缺陷**，且只在「筛选键 ≠ 排序键」时出现。
2. **作用于已截断集合**：事后过滤跑在 `top_k` 条上，`top_k*3` 的候选池里另外 `2*top_k` 条**根本没机会**参与 ——
   在同一把尺子下这不改变结果，但在换尺子的场景下有实质影响。
3. **语义只有一处**：原先 6 个调用点各写一遍阈值逻辑（4 个 command + legacy RAG + pipeline RAG），
   现在收敛到 `HybridSearchOptions.min_score` 一处 ⇒ 这才是本项的主要收益（**结构**，不是**丢结果**）。

**方法论结论（已写进 #205 与 `MEMORY-DETAIL`）**：写「X 会丢 N 条结果」这类断言前，
必须先确认**筛选键与排序键是否是同一个量**；不是则结论成立，是则**必须改口为「结构一致性」**。
⇒ 这正是纪律「**反向断言须锚定被测对象自身形态**」在**正向断言**上的对偶：正向断言同样要锚定被测对象的**实际形态**，
不能拿一个「听起来合理」的故障场景代替实测。
⇒ 另一后果：该改动在 `hybrid_search` 层**不可单测**（同键 ⇒ 无观测差异），回归锁只能落在
「命令层不再存在第二把尺子」这一侧（同 #186 的互补性测试）。**未加该侧测试 = 本项仍缺硬锁**，见下。

---

### 12.18 第八段：`clippy` 终值 —— **11 个 error 里有 1 个是我引入的**（已修）+ **CI 的 clippy 硬门禁当前必红**

> 本段接 §12.16（我）/ §12.17（并发会话）。§12.17.9 已就同一份 clippy 结果写过判断，
> 本段**不改其原文**，只在 §12.18.5 加指针更正**一处成因**。

#### 12.18.1 首轮结果，以及它同时是「第五型短路」的**当日第三次复发**

`cargo clippy -p axagent-search -p axagent-harness -p axagent --all-targets -- -D warnings` = **`EXIT=101`｜11 errors**。

⚠ **它在主 crate `axagent` 报错即中止** ⇒ `axagent-harness` 那一包**当轮结果一个都没取得**
（日志第 3 行 `Checking axagent-harness` 之后直接跳到 `axagent` 的 error，再没有它的 lint 输出）。
这是技能 `rust-consistency-audit` 里**「编译假阳性第五型」的当日第三次复发** —— 「`-p` 列全了」≠「都检查了」。

补法（实测有效）：**拆成单包调用**，因为单包**不会编主 crate** ⇒ 不被它的失败短路。

| 单包调用 | 结果 | 耗时 |
|---|---|---|
| `clippy -p axagent-search --all-targets -- -D warnings` | **`EXIT=0`** | 50.3s |
| `clippy -p axagent-harness --all-targets -- -D warnings` | **`EXIT=0`** | 31.5s |

⇒ **item #1（`reranker.rs` 阶段分数 + 标尺）与 item #3（`file_index.rs` / `ast_index.rs`）clippy 零告警**，有终值。

#### 12.18.2 11 个 error 的归属（**按 mtime + 因果，不按位置**）

| # | 位置 | lint | 该文件 mtime | 归属 |
|---|---|---|---|---|
| 1 | `src/commands/opc_industry_actions.rs:2301` | `clippy::question_mark` | **15:06:47** | 并发会话 |
| 2-10 | `src/commands/opc_workflows/seed_content_media.rs:27/29/32/36/37/39/40/42/45` | `clippy::doc_overindented_list_items` | **15:45:41** | v7 模板那轮 |
| **11** | **`src/indexing.rs:1273` `search_knowledge_with_doc_filter`** | **`clippy::too_many_arguments`（8/7）** | **14:51:12** | **＝本段（item #2）** |

#### 12.18.3 #11 确实是**本段引入**的：7 → 8 撞上「阈值型 lint」

`search_knowledge_with_doc_filter` 原本 **7 个实参** —— **恰好卡在 `too_many_arguments` 阈值（= 7）上**。
item #2 按用户要求「改动需加签名参数」穿入 `min_similarity: Option<f32>` ⇒ **7 → 8** ⇒ 越界。

**为什么它今天才浮出来**（三条叠加，缺一不成）：
1. **它是 lint-only** —— `cargo check -p axagent --all-targets` **`EXIT=0`**、`cargo test` 全绿（#72⑤ / #201）；
2. **阈值是整数** —— 加参数的人看不到「我正在越过 7」这个事实，只有 clippy 知道；
3. **本日 `target` 锁被多会话占满** —— 这是当天**第一次** clippy 真跑完（§12.17.9 同结论）。

**修法裁决：`#[allow]` 而非收敛成 options 结构体**（三条理由，非图省事）：
1. **同文件已有 7 处同类例外**（`:727` / `:780` / `:858` / `:902` / `:1061` / `:1382` / `:1503`）⇒ 加 `allow` 是**遵循既有惯例**，收敛反而制造第 2 种风格；
2. **两个参数语义独立**（`doc_ids` 限**范围**、`min_similarity` 限**相关度**），合并只会把「调用点看得见的两个语义」藏进一个匿名结构体（与 §12.16.4「可读性优先于参数计数」一致）；
3. 收敛会**同时改 `search_knowledge` 的签名形态**（两者原本平行），扩大改动面 —— 而本项的目标是「阈值语义收敛到检索层一处」，不是签名重构。

落地形态：`#[allow(clippy::too_many_arguments)]` + 一段写清「**为什么不收敛**」的 doc 注释（照 `dao/src/repo/workflow_tool.rs:57` 的先例写法）。

#### 12.18.4 🔴 连带发现：**CI 的 clippy 硬门禁当前必红**（不是「本地工具链更新」，是同一把尺子）

| 事实 | 位置 |
|---|---|
| CI 挂的是 `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `ci.yml:262-266` |
| CI **钉死** `toolchain: "1.97.0"` | `ci.yml:233-236` |
| 本地 `rustc 1.97.0` / `clippy 0.1.97` | `rustc --version` / `cargo clippy --version` |

⇒ **两侧工具链逐字一致** ⇒ 那 10 处（非本段引入）在 CI 上**同样会报**；且 CI 的面是 `--workspace --all-features`，
**比本段测的 `-p axagent` 更大** ⇒ **此刻 CI 的 `rust-check` job 必然失败**（除非 `should-run-heavy` 把它 skip 掉）。
这与 `cargo check --all-targets` 全程全绿形成对照 —— **#72⑤「lint-only 规则 `check`/`test` 都不跑」的当日第二次铁证**。

#### 12.18.5 更正 §12.17.9 的一处成因（不改原文，只加指针）

§12.17.9 表内 #11 行：「`src/indexing.rs:1273` … **归属 = 非本段**」——
**归属列（mtime 14:51:12 / 非本段）正确**；但同段的解释句写「**新增了 `doc_ids` 这一类参数**」，
**成因说错了**：`doc_ids` 不是新增的（它一直就在），把 7 顶到 8 的是 **`min_similarity`**（本段 item #2）。
⇒ 该行现在是「**其他会话的在飞工作**」，实际是「**本段 item #2 的回归**」，已在本段修掉。

#### 12.18.6 那 10 处**不顺手修**（照纪律；与 §12.17.9 独立得出同结论）

1. **所在两个文件正被并发会话编辑**（`opc_industry_actions.rs` 15:06、`seed_content_media.rs` 15:45）
   ⇒ 手工改或 `rustfmt` 整文件重写都可能被其缓冲区**覆盖回去**（改了等于没改，还扩大冲突面）；
2. **判据 #200「不要重复修」** —— §12.17.9 已独立登记同一批并给出理由；
3. **纪律「报告不擅改」**：这 10 处虽属「纯 bug（可顺手修）」一类，但**归属他人 + 在飞**，超出本段作业面。

⇒ **只登记 + 单列告知用户**，等裁决。**但这条是本段最要紧的交付**：它意味着用户当前那次 CI 跑不出来。

#### 12.18.7 验证矩阵（本段终值）

| 检查 | 终值 |
|---|---|
| `cargo clippy -p axagent-search --all-targets -- -D warnings` | **`EXIT=0`**（50.3s） |
| `cargo clippy -p axagent-harness --all-targets -- -D warnings` | **`EXIT=0`**（31.5s） |
| `cargo clippy -p axagent --all-targets -- -D warnings -A clippy::doc_overindented_list_items -A clippy::question_mark` | **`EXIT=0`**（8m45s）｜**零 error / 零 warning** ⇒ 除那 10 处**已归属他人**的 lint 外，`axagent` 包无任何 clippy 问题 |
| `cargo clippy -p axagent --all-targets -- -D warnings`（**零豁免 = 验收口径**） | **`EXIT=101`｜11 errors**（10 处非本段，见 §12.18.2） |
| `rustfmt --edition 2024 --check` × 16 个改动文件 | **16/16 OK**（含修 `#[allow]` 之后的 `indexing.rs`） |
| `check-single-source-facts.mjs --selftest / --ci / 本地` | **0 / 0 / 3**（软判据 2 处，均为区间端点，见 §12.18.9） |
| `check-single-source-facts.mjs` 分类 | **LOCATED 271｜NONLOC 77｜EXTERNAL 6｜BROKEN 0** |

**✅ #11 的修复已被这次重跑覆盖验证**（两条独立证据）：
1. 该日志里 **`indexing.rs` 一次都没出现**（`Select-String "indexing\.rs"` **0 命中**）⇒ 加 `#[allow]` 后 `too_many_arguments` 不再触发；
2. 本段改过的 8 个文件里，**唯一被 clippy 报过的是 `indexing.rs:1273`**，而它出现在**未豁免那次的全清单**里；
   其余 7 个（`indexing_triggers.rs` / `commands/{knowledge,memory,paper,wiki,unified_knowledge}.rs` / `init/state.rs` / `cloud_workspace.rs`）
   在**未豁免**那次的全清单里一次都没出现 ⇒ 它们的 clippy 状态由**未豁免**那次背书，**不是**靠 `-A` 遮出来的。

#### 12.18.8 未闭口（如实登记，勿当已绿）

1. **`axagent` 包的「零豁免」验收口径仍未通过** —— 卡在 10 处**非本段**的 lint 上（已归属 §12.18.2 + 已登记 §12.18.6）。
   本段能做的**取证已完成**（§12.18.7 第 1 行）：豁免那 10 处后 **`EXIT=0`｜零 error 零 warning** ⇒ **除它们之外零问题**。
   ⚠ **`-A` 结果只作取证，不作验收**：不得把「除豁免项外干净」写成「clippy 通过」；验收口径仍是**零豁免全量**，
   而它当前**红**（根因在他人作业面）。
2. **`-A` 生效位置**（易错）：必须写在 `-D warnings` **之后**（lint level 按命令行顺序覆盖）。
3. **item #2 仍缺「命令层不再有第二把尺子」的硬锁**（§12.16.7 未闭口 2，未变）。
4. **77 条 NONLOC**（22% 未检查，见 §12.17.7 未闭口 2，未变）。
5. 并发会话仍在写同一仓 ⇒ 本节绿灯有效期**只到下一次外部改动**。

#### 12.18.9 复核：并发会话对门禁脚本的改动是**加强**，不是放宽

本段独立复核了 §12.17 对 `scripts/check-single-source-facts.mjs`（mtime **15:43:20**）的改动：

| 复核项 | 结果 | 判定 |
|---|---|---|
| `SKIP_DIRS` 全集 | `target`/`node_modules`/`.git`/`dist`/`knowledge-sources`/`.output`/`.vite`/`.monaco`/**`.worktrees`**（9 项） | ✔ 与 §12.17.2 一致（加 `.worktrees` ⇒ LOCATED 137 → 271、**BROKEN 不变**=零误红） |
| `--selftest` 断言数 | **27**（原 19） | ✔ **加强**，未削弱 |
| CI 挂载 | `ci.yml:172-173` **只一处**；`ci-check.mjs:169-170` 逐字同源 | ✔ 本段 §12.16 的去重保持有效 |
| 本地口径软判据 | **2 处**（`dao/src/repo/knowledge_graph.rs:811`、`stock_workflow/core.rs → workflow_types.rs:2565`） | ✔ 与 §12.17.6 一致；两处**均为区间端点**（§12.17.5 已核实 `tool_executor.rs` 第 244 行 / `engine/mod.rs` 第 3250 行 是真漂移、已修） |

⇒ 本段的 item #4 交付**已被两会话共同收口**；门禁处于「覆盖面 271 条 + 棘轮基线 0 + 三模式全绿」的稳定态。

#### 12.18.10 方法论（已写进 `rust-consistency-audit` 技能）

1. **型⑦「加签名参数静默撞阈值型 lint」**：`too_many_arguments` 阈值 = **7** ⇒ 加/删参数**前先数实参个数**；
   `check --all-targets` + `test` 全绿**不能**推定 clippy 干净（#72⑤）。
2. **第五型补法**：主 crate 失败会吞掉同一次调用里其他包的结果 ⇒
   **拆单包调用**（单包不编主 crate）**＋ `-A` 豁免取证**（且 `-A` 只作取证）。
3. **环境**：多会话**共享 `%TEMP%`** ⇒ 落盘临时文件名必须带会话前缀/时间戳，否则会读到**对方**的旧内容
   （本段实测：用短名 `$env:TEMP\r1.txt` 读到了对方早期一轮的门禁输出）。

#### 12.18.11 ⚠ 同日自查的**第二处自伤**：三处**代码注释**仍在复述已被推翻的因果（#128 族）

§12.16.7 与 `MEMORY-RULES` #205 更正了「先截断后过滤 ⇒ 丢合格项」这一因果，
但**代码注释没跟着改** —— 三处仍在复述：

| 位置 | 复述的旧说法 |
|---|---|
| `crates/search/src/rag.rs`（`search_with_filter` doc） | 「配 5 条 + 阈值 0.6 可能返回 0 条，而候选集里其实有 20 条过阈值」 |
| 同上（legacy 调用点） | 「过阈值的 20 条里只有排名前 5 条有机会被检查」 |
| `crates/search/src/rag_pipeline.rs`（`execute_with_filter` doc） | 「候选集里其实有 20 条都过了阈值」 |
| `src/commands/knowledge.rs`（`search_knowledge` 调用点） | 「已截断成 top_k ⇒ 配 5 条 + 阈值可能返回 0 条」 |

⇒ **教训（新）**：**文档更正不覆盖代码注释**。PLAN / RULES 改完就以为「口径已统一」是错的 ——
代码注释才是以后每个读代码的人（和每个 AI 会话）看到的那一份。

已改：四处全部改写为准确因果（**两把尺子是主因**、**结构收敛是主收益**），并**留变形痕迹**
（把 `20 条` 写成 `二十条`）—— 否则「更正说明里逐字复述旧写法」会让旧说法在未来的**子串审计**里
**恒命中**（`MEMORY-RULES` #128 原样重演；§12.17.4 已在注释层实测过一次同类）。
⚠ 这条更正**作废了** 16:13:51 与 16:20 那两次 `test-search` 产物（判据 #175），故本段末**整批重跑**。



