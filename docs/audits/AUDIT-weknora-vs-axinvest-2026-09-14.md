# AUDIT — WeKnora vs AxInvest 源码级对比审计（RAG / Wiki 专项）

- **日期**：2026-09-14
- **对象**：`https://github.com/Tencent/WeKnora`（v0.8.0，Tencent 开源）
- **本地快照**：`output/tmp-weknora-src/WeKnora-main`（3525 文件 / 42 MB 源码级子集）
- **证据口径**：**一切以代码为准**。README / 注释与代码冲突时以代码为准并明确标注
- **取证方式**：GitHub API tree + raw 逐文件并发拉取，**未使用任何 git 命令**
- **审计范围**：仅 RAG / Wiki / 知识图谱 / 记忆 / 检索安全 / 可观测六域，**不含** sandbox / IM / skill catalog

---

## 0. 边界声明

| 项 | 说明 |
|---|---|
| 许可 | **MIT**（`LICENSE.txt` 头部，第三方组件另有条款）—— 可参考实现 |
| 本审计产出 | 即便 MIT，本文所有借鉴条目仍按「**用自己的 Rust 栈重新实现**」给出，**不做代码移植** |
| 栈差异 | WeKnora = Go 1.26 + Vue 3 + Python(docreader sidecar) + gRPC；AxInvest = Rust 2021 + Tauri 2 + React 19。**代码级移植不成立，机制级借鉴成立** |
| WeKnora 规模 | Go **2240 文件 / 551,417 行**（`internal/application/service` 单目录 118,705 行）；另有 docreader 80 文件（Python）、frontend 723 文件 |
| AxInvest RAG 规模 | `axagent-search` crate 21 文件 / 约 15,023 行；`document-parser` crate 2 文件 / 442 行 |

> **规模判断**：两家 RAG 主线代码量级接近（15k vs 检索链约 30k），不是「大项目抄小项目」的关系。

---

## 1. 总判断

> **两家仍是同一类病 —— 「声明的机制」与「运行的机制」断链 —— 但病型相反，检法不可互套。**

| | AxInvest | WeKnora |
|---|---|---|
| 病型 | **入边缺失型** | **出口不严型** |
| 表现 | 零件齐全、**线没接**：配置定义了不读、provider 恒传 `None`、模块有实现无调用 | 线接上了，但**默认关 / 异常静默放行 / 全失败也返回成功** |
| 典型 | `HybridConfig` 全死配置、`entity_graph_provider` 恒 `None`、`recall_pipeline`/`vector_cache`/`recall_pipeline.semantic_cache`/`incremental_indexer` 四模块零生产调用 | rerank 失败 fail-open、VLM OCR 全挂仍报成功、分块三层全被拒仍返回 legacy、`NEO4J_ENABLE` 默认关、`LANGFUSE_ENABLED` 默认关、DLQ 写入失败致任务永久消失 |
| 检法 | **入边供给检查**（每个声明必须有消费点） | **出口检查**（每个 gate 异常分支放行还是拦截） |

**推论**：AxInvest 要补的是「接线 + 出口不严」两件事，**不能**把 WeKnora 当范本整体照抄 —— 它自己有一堆 fail-open。

### 1.1 对方「既设计了又真的实现了」的清单（本问决定借鉴清单长度）

逐机制核对后，**真正落地的有 18 条**，但真正值得借鉴的是其中 **9 条**：

| # | 机制 | 判定 | 复核 |
|---|---|---|---|
| 1 | **父子块**：父块不进向量索引，子块命中后回填父块全文给 LLM | 真实现 | ✅ 逐字复核 `chat_pipeline/merge.go:317-344` |
| 2 | **`ContextHeader` 位置不变式**：标题面包屑参与 embedding 但不属于 Content | 真实现 | ✅ 逐字复核 `chunker/splitter.go:18-49` |
| 3 | **表格跨块表头继承**（`header_tracker`） | 真实现 | ✅ `splitter.go:454-496` |
| 4 | **加权 RRF 且权重真被读取**（租户级配置） | 真实现 | ✅ `knowledgebase_search_fusion.go:84-86` |
| 5 | **检索质量 span 字段**（来源拆分 / 分数 / top-N / 降级标志 / 重排前后对比） | 真实现 | ✅ `tracing/langfuse/retrieval_obs.go:25-61`、`rerank.go:297-308` |
| 6 | **记忆 pending 确认闸门**（LLM 推断的记忆必须用户确认才生效） | 真实现 | ✅ `memory/service.go:472-485` |
| 7 | **记忆双时态 supersede**（`invalid_at` + `superseded_by`，读路径零模型调用） | 真实现 | ✅ `memory/service.go:317-424` |
| 8 | **Wiki 快照与更新同事务 + 乐观锁 + 回滚新建版本** | 真实现 | ✅ `repository/wiki_page.go:113-129` |
| 9 | **Wiki 增量三态对账**（replace / retractStale / 覆盖 summary） | 真实现 | ✅ `wiki_ingest_batch.go:1531-1559` |

其余 9 条（分块降级链、chunk 编辑+版本、任务队列+DLQ、多后端统一契约、API key 默认拒绝、untrusted 包裹、SSRF 双重拦截、chunker preview、RAG 评估指标）**全部带有明确缺陷**，见 §5。

---

## 2. 维度骨架覆盖状态（防「整块漏维度」）

动手前先立骨架，逐项标注覆盖状态 —— **「未覆盖」也是合格交付，关键是不默默漏**。

| 大类 | 维度 | 覆盖 | 结论摘要 |
|---|---|---|---|
| ① 数据层 | 文档解析覆盖 | ✅ | 8 引擎 + Python docreader；OCR/VLM 在 Go 侧；**VLM 失败全静默** |
| | 分块策略 | ✅ | 3 tier + auto 画像；父子块；**默认值 8 处不一致** |
| | 向量存储 | ✅ | 10 真后端 + 2 空壳枚举；统一 `RetrieveEngine` 契约 |
| | 增量与一致性 | ⚠️ 部分 | Wiki 有增量对账；文档重索引的旧向量清理未查清 |
| ② 契约层 | chunk / 检索结果 schema | ✅ | `RetrieveParams`/`RetrieveResult`；chunk 有 `parent_chunk_id` |
| | 知识图谱本体 | ✅ | 固定 11 类枚举 + 每 KB 可覆盖；**无实体消歧** |
| ③ 分层与依赖 ★ | 依赖方向矩阵 | ✅ | **工具量化**：117 包；反向依赖仅 5 条；`internal/utils` 是 fan-in 41/fan-out 0 杂物抽屉 |
| | 分层护栏存在性 | ✅ | **零 depguard / 零架构测试 ⇒ 四链全不成立** |
| ④ 接口层 | REST / MCP / CLI | ✅ | 33 个 MCP tools（README 称 29，已过期）；MCP 网络传输缺 token 直接 `sys.exit(1)` |
| | 出口严不严 | ✅ | 大量 fail-open，见 §5 |
| ⑤ 业务层 | 召回链 | ✅ | hybrid RRF + 本地 expansion + LLM rewrite；**无 HyDE、无多查询分解** |
| | rerank | ✅ | 7 provider + MMR(λ=0.7) + 阈值降级重试；**失败 fail-open** |
| | GraphRAG | ✅ | 单跳 `n.name CONTAINS`；**内存图谱 1040 行是死代码** |
| | Agent 化检索（记忆） | ✅ | 记忆完成度高：五类 / pending / RRF / 双时态 / 淘汰 |
| ⑥ 时间语义 | 双时态 | ✅ | 记忆有 `ValidFrom/InvalidAt/ExpiresAt/LastUsedAt`；**检索打分主序不含最近性** |
| ⑦ 治理与门禁 | 四链 | ✅ | 见 §5「对方病历」第 12 条 |
| ⑧ 工程质量 | 测试与评估 | ✅ | 1027 个 `_test.go`；**15+ 处 env-gated 静默跳过**；评测指标齐全但**无 CI 回归** |
| ⑨ 可观测性 | 检索 trace | ✅ | 最强项：质量字段固化；**Langfuse 默认关闭** |
| ⑩ 安全 | 租户隔离 | ✅ | 隔离在**应用层**；DB 向量表**无 tenant 列** |

---

## 3. 逐维度对照（RAG 主线）

| 维度 | AxInvest | WeKnora | 谁强 |
|---|---|---|---|
| **分块单位与中文** | 字符级（`text_chunker.rs:250-251` 注释明确「不再被压缩成 1/3」）；默认 2000/200 | 字符级（rune）；默认 512/80，但**8 处定义冲突**（76 / 50 / 160 / 1536 / 0 / 2048） | **AxInvest**（单一真源） |
| **标题语义保留** | 按 `#`~`######` 切分，但**切完丢失层级路径**（grep `breadcrumb|context_header` 零命中） | **`ContextHeader` 面包屑 + 与 Content 分离**，抛 `EmbeddingContent()` 给 embedding | **WeKnora 完胜** |
| **父子块** | ❌ 无（`parent_chunk` 零命中；`sub_chunks` 只是局部临时变量） | ✅ 真实现：父不进索引，命中子回填父全文 | **WeKnora 完胜** |
| **结构原子性** | ❌ 无保护块（grep `protected|table|fence|formula` 零命中）；仅 `CODE_CHUNK_SIZE=120` 常量**无实现** | ✅ 7 条正则保护块 + 表头重写 + 超限块强制切碎 | **WeKnora 完胜** |
| **分层降级** | 无 tier 概念 | 3 tier 链 + `ValidateChunks` 校验器 + auto 画像选择 | WeKnora（但有缺陷） |
| **文档解析** | 4 依赖手写（`pdf-extract` + zip 解 docx/xlsx/pptx）；HTML **不剥标签**直读；OCR 走 `tesseract` 子进程，失败返回 `Ok("")` | 8 引擎 + 独立 Python docreader（gRPC/TLS，PDF 用 pypdfium2 按图像覆盖率判扫描页）；VLM OCR 在 Go 侧 | **WeKnora** |
| **检索后端** | 2（SQLite sqlite-vec / PG pgvector），双后端路由 | **10 真后端** + 统一 `RetrieveEngine` 契约 | WeKnora |
| **混合融合** | RRF k=60，0.7/0.3 —— 但**权重写死**，`HybridConfig` 是**死配置** | RRF，**权重来自租户配置且真被读取** | **WeKnora** |
| **chunk 级去重** | 跨源去重 `deduplicate_cross_source`（Jaccard 0.65，**source 级**） | chunk 级内容指纹 + token 重叠 ≥0.85 裁剪 | 各有所长 |
| **MMR 多样性** | ❌ 零命中 | ✅ `applyMMR(λ=0.7)` | **WeKnora** |
| **查询侧增强** | HyDE / MultiQuery / Decomposition **有实现但默认关** | LLM rewrite **默认开** + 本地 expansion **默认开**（无 HyDE） | WeKnora（我方是入边缺失） |
| **rerank** | rule（默认）/ cross-encoder 本地 / 云端 3 家 | 7 provider；**失败 fail-open** + 阈值自动降级 ×0.7 + 保底 top1 | 平手（各有 fail 模式） |
| **引用溯源** | `[cite:N]` token + `rebuild_context_with_citations` | `<ref id="cN"/>` 模型自产标记 + 系统展开 | 平手 |
| **token 预算** | ✅ `apply_rag_token_budget`（`agent/mod.rs:2727`） | ❌ **不存在**（零命中） | **AxInvest** |
| **GraphRAG** | trait 齐全，但 `entity_graph_provider` **恒传 `None`** ⇒ 永不执行 | ✅ 单跳接通（`search_entity.go:106,171`），但仅 `CONTAINS` 子串匹配 | WeKnora（我方是断链） |
| **图算法** | ✅ Louvain 社区发现且**接到 Wiki 图谱**（`dao/repo/knowledge_graph.rs:269`） | ❌ 零社区发现 / 零中心性 | **AxInvest 完胜** |
| **图谱本体** | ✅ 55 关系类型 + `EdgeTypeDecl`/`RelationDecl` + `PageType` 亲和度公理 | 11 类固定枚举 + 每 KB 可覆盖；**无消歧** | **AxInvest** |
| **Wiki → 检索** | ✅ `RAGSourceType::Wiki` 是真检索源 + Wiki↔KB 双向同步命令 | ❌ **`ChunkTypeWikiPage` 只有读点**（`wiki_boost.go:57,85`），全仓**零写入点** ⇒ boost 恒空转 | **AxInvest 完胜** |
| **Wiki 版本/回滚** | ✅ `wiki_page_versions` 表 | ✅ 全量快照 + 同事务 + 乐观锁 + 回滚新建版本 | 平手 |
| **chunk 编辑** | 有编辑（`update_chunk_content`）**但无版本历史** | ✅ 编辑 + 版本 + diff + 回滚 + 自动重索引 | WeKnora |
| **检索可观测** | 有 `tracing` crate、`record_hits`（失败被 `unwrap_or_default` 吞） | ✅ **质量字段固化**（`vector_hits`/`keyword_hits`/`top_hits`/`threshold_degraded`/重排前后） | **WeKnora 完胜** |
| **分层护栏** | ✅ `check-contracts.mjs:177-198` 段 D 机械强制 + CI 真跑（`ci.yml:98`） | ❌ 零 depguard / 零架构测试 | **AxInvest 完胜** |
| **多租户隔离** | 单用户本地应用，无租户概念 | 应用层逐一授权 KB；**DB 向量表无 tenant 列** | WeKnora（对我方不适用） |

---

## 4. 我方缺陷实证（本轮新发现，含文件:行）

| # | 缺陷 | 证据 | 优先级 |
|---|---|---|---|
| A1 | **分块丢失章节层级路径** —— 按 `#` 切分后，chunk 不携带「它属于哪一章」的信息，被切碎的 section 语义上下文丢失；embedding 只看到孤立的段落 | `crates/search/src/text_chunker.rs` grep `breadcrumb\|context_header\|EmbeddingContent` **零命中**；`chunk_by_markdown_headings`（`:78-163`）只做切分不记录路径 | **P0**（直接压低召回质量，且是 WeKnora 明证有效的解法） |
| A2 | **无父子块** —— 命中一个 2000 字符的块，返回给 LLM 的就是这 2000 字符，无法回填更大的上下文 | `crates/search/src` grep `parent_chunk` 零命中；仅 `sub_chunks` 局部变量（`text_chunker.rs:133,198`） | **P1** |
| A3 | **长表格 / 代码块会被切断** —— 无保护块机制，表格跨块后列名丢失 | `text_chunker.rs` grep `protected\|table\|fence\|code_block\|formula` **零命中**；`CODE_CHUNK_SIZE=120` 常量（`:11-18`）**无任何实现引用** | **P1** |
| A4 | **`HybridConfig` 是死配置** —— `enabled`/`vector_weight`/`bm25_weight`/`fusion` 定义了但零读取点，用户改配置不生效 | `crates/harness/src/rag_config.rs:83-138` 定义；`rag.rs:340` 写死 `HybridSearchOptions::default()`；`commands/wiki.rs:734-736` 也写死 0.7/0.3 | **P0** |
| A5 | **四模块零生产调用** —— `recall_pipeline.rs`(348行) / `semantic_cache.rs`(1220行) / `vector_cache.rs`(216行) / `incremental_indexer.rs`(302行) | 侦察兵穷尽搜索（内部模块 + commands + REST + tests）无生产调用点 | P1（按铁律 #3：被覆盖 ⇒ 删） |
| A6 | **chunk 编辑无版本历史** —— 有 `update_chunk_content` 但改完即覆盖，无法 diff / 回滚 | `commands/knowledge.rs:1044,1084,1285`；全仓 grep `chunk_revision\|chunk_version` **零命中** | P2 |
| A7 | **旧向量删除失败无补偿** —— 编辑后旧内容永久可被召回 | `src/indexing.rs:940` `let _ = vector_store.delete_document_embeddings(...)`（与 `:936-937` 注释声明的修复目标相悖） | **P0**（数据正确性） |
| A8 | **GraphRAG 永不执行** —— `RAGPipeline::new` 第 4 参恒传 `None` | `crates/search/src/rag.rs:2340-2345`；`rag_pipeline.rs:150` 分支恒不进入（`dao/knowledge_graph_provider.rs:81` 有实现但无入边） | P1 |
| A9 | **检索命中记录写库失败被吞** —— 反馈闭环静默断链 | `src/indexing.rs:1202-1203` `record_hits(...).unwrap_or_default()` | P1 |
| A10 | **文档状态机假成功** —— OCR 失败返回空串 → 零 chunk → 文档状态仍置 `ready` | `document-parser/src/lib.rs:33,43` `unwrap_or_default()`；`src/indexing.rs:768` 置 `ready`，`:828-830` `Ok(vec![])` 直接 `return Ok(())` | **P0** |

### 4.1 复核更正与补充（2026-09-15，实施轮自证）

实施 W1–W12 前对 §4 逐条回到源码复核，发现**两处事实偏差**与**四处补充**。
下文按「原判定 → 实测」列出；**原表保留不删**（保留审计时点的判断，便于对比）。

| # | 原判定 | 实测（2026-09-15） | 处置 |
|---|---|---|---|
| A9 | 「`src/indexing.rs:1202-1203` `record_hits(...).unwrap_or_default()`」 | **行号错**：`record_hits` 全仓唯一调用点是 `commands/conversations/streaming.rs:1196`。且该函数**恒返回 `Ok`**（逐条 `warn!` 后 `Ok(ids)`，`repo/retrieval_hit.rs:88-113`）⇒ `unwrap_or_default()` 是**死分支**；真正的吞没点在函数内部 —— 整批失败与全部成功返回**形态完全相同** | 判据成立、证据修正；已修（W12） |
| A10 | 「`lib.rs:33,43` `unwrap_or_default()` ⇒ OCR 失败返回空串」 | **只有 PDF 分支成立**：`:43`（图片分支）的空串会被 `:44-49` 转成 `Err`（fail-closed）；PDF 分支（`:29-39`）才是真「假成功」（文本层空 + OCR 空 ⇒ `Ok("")`） | 判据部分成立；已修（W4，三层） |
| A5 | 「四模块零生产调用」 | 补充：`tools::capability_indexer_impl::remove_index` 同样**恒返回 `Ok(())`**（四处 `if let Err = … { warn! }` 后 `Ok`）⇒ 4 个调用点的 `if let Err(e)` 分支是死代码 | 新增证据；已修 |
| A7 | 「`src/indexing.rs:940` 一处 `let _ =`」 | 补充：同形态**共 8 处**（`indexing.rs:940,1004` / `commands/memory.rs:326` / `commands/knowledge.rs:505,2096,2188` / `commands/wiki.rs:369` / `capability_indexer_impl.rs:384,392,400`）。其中 `memory.rs`、`knowledge.rs` 的**删除命令**会让已删除的记忆/文档继续被检索命中 | 范围扩大；已全部修 |
| A4 | 「`HybridConfig` 死配置；`rag.rs:340` / `wiki.rs:733` 写死」 | 补充**更严重的一层**：即便接线，默认 `fusion=rrf` 下 `merge_results_rrf` **只收 `rrf_k`、两路权重根本不参与** ⇒ 权重是「装饰性参数」 | 接线 + 实现加权 RRF（`weight/(k+rank+1)`）；已修 |
| A8 | 「`rag.rs:2340` 第 4 参恒 `None`」 | 补充：`dao::knowledge_graph_provider::KnowledgeGraphProvider::new` **全仓零调用点** ⇒ provider 从未被实例化（比「传 `None`」更强） | 已修（注入点 + wiring，**默认关闭**） |

**同时把 §7-5 从「未验证」升级为「已实证」**：

| 原 §7-5 | 「`rag.rs:1387` 的 `effective_threshold` 是否恒不生效 —— 侦察兵给出推断，未写测试验证」 |
|---|---|
| 实测 | **恒不生效已实证**：`search_with_filter` 返回 `score = 1.0 - combined_score`（RRF 量纲 ≈ 0.983~1.0），而 `collect_rag_context_from_refs` 按 **L2 距离**语义过滤（`default_max_distance = 20.0`）⇒ `score <= 20.0` 对任何 RRF 结果恒真。加权 RRF 不改变该结论（量纲仍是 ≈1.0） |
| 判定 | **登记待裁决**：修复需统一 score 语义（RRF 排名分 vs L2 距离），牵连多后端与前端展示，属产品语义决策，不在本轮擅改 |

---

## 5. 对方病历表（**借鉴前必读 —— 抄它的实现会把病一起抄进来**）

| # | 机制 | 声称 | 代码事实 | 判定 |
|---|---|---|---|---|
| 1 | **Wiki 参与检索** | README 暗示 Wiki 进向量库并被加权 | `ChunkTypeWikiPage` 全仓**只有读点**（`wiki_boost.go:57,85`），**零 `CreateChunks` 写入点** ⇒ `hasWikiChunk` 恒 false，boost 走空转快路径（`wiki_boost.go:62`） | **断链（入边缺失）** |
| 2 | **rerank 失败** | — | `chat_pipeline/rerank.go:130-144` API 错误仅 `pipelineWarn`，原检索序直接进下游 | **fail-open** |
| 3 | **VLM OCR / 图片** | 多模态理解 | `image_multimodal.go:271-286` OCR 报错仅 Warn；`:338-342` OCR+Caption 全空 → **`return nil`（视为成功）** ⇒ 扫描件知识静默不存在 | **fail-open** |
| 4 | **分块三层全被拒** | 「自适应 3 层」 | `chunker/strategy.go:43-57` 所有 tier 被 `ValidateChunks` 拒绝后仍返回 `lastOut`，且**日志级别是 Debug**（生产不可见） | **fail-open** |
| 5 | **默认值一致性** | — | chunk size/overlap **8 处定义冲突**：512/80、76、50、160、1536、0、2048/0、1600/160；separators **3 套** | 多处定义必不一致 |
| 6 | **检索参数一致性** | — | VectorThreshold 4 处（0.15/0.2/0.5/0.7）、EmbeddingTopK 3 处（50/30/10）、RerankThreshold 3 处 | 同上 |
| 7 | **假配置** | 租户可配检索参数 | `RetrievalConfig.VectorThreshold` 等经 API 可编辑，但**主问答链路 `KnowledgeQA` 不读**（走 `config.yaml`）；`SearchParams.OnlyRecommended`、`PipelineRequest.VectorDatabase` 全仓**零读取点** | **假配置** |
| 8 | **分数语义一致性** | 统一阈值 | 归一化**仅跨引擎时执行**（`knowledgebase_search_fanout.go:110`），单引擎快路径不归一化 ⇒ 阈值在不同后端语义不同尺；Milvus 用原始 cosine 过滤 | 部分实现 |
| 9 | **租户隔离** | enterprise-ready multi-workspace RBAC | 向量表**无 `tenant_id` 列**（`postgres/structs.go:15-29`），检索 SQL **不带租户条件**；隔离全靠应用层 `KBPermissions.Check` | 应用层单闸 |
| 10 | **图检索** | 知识图谱 | Neo4j 查询只有**单跳**且 `n.name CONTAINS` 子串匹配（`neo4j/repository.go:178-182`）；`query_knowledge_graph` 工具实为向量检索外壳，**`edges` 初始化后从不填充**（`:483,507`） | 部分实现 |
| 11 | **内存图谱** | — | `service/graph.go` 1040 行（PMI/DFS/Mermaid），`NewGraphBuilder` / `GetRelationChunks` **零调用者** | 死代码 |
| 12 | **DLQ 可靠性** | task queue + DLQ 归档失败任务 | `wiki_ingest.go:1244-1250` **DLQ Insert 失败仅 Warnf，随后仍 `DeleteByIDs`** ⇒ 该 op 从队列与 DLQ **同时消失 = 永久静默丢失** | **fail-open（数据丢失）** |
| 13 | **分层护栏** | — | 无 depguard、无架构测试；`.golangci.yml` 仅 3 个 linter 且 CI `only-new-issues: true`（存量违规不受约束） | **四链仅第一链** |
| 14 | **测试门禁** | CI 全量 `go test` | **15+ 处 env-gated 静默跳过**（memory PG 测试、feishu 真 API、sandbox docker/e2b、browserskill、chat 远程 API…）；`vectorstore_test.go:441` 无条件 `t.Skip` | CI 中等于不存在 |
| 15 | **RAG 质量评估** | — | 指标齐全（Recall/Precision/NDCG/MRR/MAP/BLEU/ROUGE + `dataset/samples/*.parquet`），但**无 CI 回归、无基线** | 人工工具 |
| 16 | **可观测性** | full Langfuse observability | `LANGFUSE_ENABLED` 未设且无 key ⇒ **默认关闭（noop）**，不报错 | **默认关闭** |
| 17 | **embedding 降级可观测** | — | 降级为关键词仅打日志（`search.go:429-435`），**降级原因不进 span**（rerank 降级却进了） | 部分实现 |

---

## 6. 我方强于对方的地方（不要妄自菲薄）

| # | 维度 | AxInvest | WeKnora |
|---|---|---|---|
| 1 | **分层护栏** | `check-contracts.mjs:177-198` 段 D 机械强制 `CONSUMER_CRATES` 只许依赖 `axagent-harness`，CI 真跑 | 零护栏（M7 反面教材） |
| 2 | **图算法** | Louvain 社区发现**真接线**到 Wiki 图谱（`wiki.rs:998,1182,1216` → 前端 `WikiGraphPage.tsx:133`） | 无任何社区发现 / 中心性 |
| 3 | **图谱本体表达力** | 55 关系类型 + `RELATION_DECLS`/`EDGE_TYPE_DECLS` + `PageType` 亲和度公理（`page_type.rs:5-11` 自述「本体首次有真实生产消费」） | 11 类枚举，**无消歧**，仅 `name` 精确合并 |
| 4 | **Wiki → RAG 接通** | `RAGSourceType::Wiki`（`rag.rs:1285-1313`）是真检索源 + Wiki↔KB 双向同步命令（`wiki.rs:1263-1341` / `:1346-1411`） | Wiki chunk **零写入点**，boost 空转 |
| 5 | **token 预算** | `apply_rag_token_budget`（`agent/mod.rs:2727`） | **不存在** |
| 6 | **分块单一真源** | 默认值集中在 `text_chunker.rs:11-18` | 8 处冲突 |
| 7 | **跨源去重** | `deduplicate_cross_source`（Jaccard 0.65，`rag.rs:1612-1741`） | 有 chunk 级去重，无跨源 |

---

## 7. 不确定项（**未验证，不要当结论用**）

1. **WeKnora 非 postgres/sqlite 后端**（milvus/qdrant/weaviate/doris/opensearch/es）的过滤条件构造**未逐字核对** —— 不得推定与 postgres 同构。
2. **Milvus `ann.WithRadius` 是否真形成下界过滤**未查证（可能阈值在该后端不生效）。
3. **WeKnora 生产是否真出现「同一请求多引擎」**从而真正触发归一化 —— 未查证。
4. **AxInvest 生产 embedding 维度**未读运行库（代码层无硬编码）。
5. **`rag.rs:1387` 的 `effective_threshold` 是否恒不生效** —— 侦察兵给出推断（score 是 RRF 值域约 0~0.033，而阈值语义是 L2 距离 20.0），**未写测试验证**，仅列为待验证。
6. **WeKnora「40k 文档」**声明无压测脚本佐证，仅机制存在。
7. 本轮**未审计** WeKnora 的 `cli/` / `mcp-server/` 内部实现细节与 `miniprogram/`。

---

## 8. 一句话总结

> WeKnora 的**分块工程质量**（`ContextHeader` 位置不变式 / 父子块回填 / 表头继承 / 保护块）与**检索可观测**（质量字段固化）、**记忆闸门**（pending 确认 + 双时态）是真本事，值得按机制重写进 AxInvest；
> 但它的**接线纪律**（wiki_boost 断链、NEO4J/Langfuse 默认关、4 处假配置、8 处默认值冲突）与**出口纪律**（rerank/VLM/分块全 fail-open、DLQ 丢数据）比 AxInvest 更差 —— **抄机制，别抄工程习惯**。
