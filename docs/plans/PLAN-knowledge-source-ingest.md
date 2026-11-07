> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 知识库增长更新入口 — 实施计划

> **目标**：让知识源（Wiki / Obsidian / RAG）成为可**持续增长、自动更新**的入口，支持网络爬取、开源知识库导入、Wiki 页面增量更新。
> **状态**：P1–P4 全部实现完毕（2026-08-01），详见文末「实施记录」
> **版本**：v1.1 / 2026-08-01

---

## 1. 背景与核心判断

AxAgent 已有成熟的知识底座：Wiki 双体系（经典 + LLM）、RAG 导入管道（10+ 格式 + sqlite-vec）、WebFetchTool、持久化索引队列、cron 调度。

**缺的不是基础设施，是「源管理 + 抓取器 + 更新闭环」这一段。**

| 层 | 现状 | 缺口 | 对策 |
|----|------|------|------|
| 源管理 | `wiki_sources` 表（来源登记，已有 content_hash） | 无调度周期/状态/上次抓取字段 | **扩展字段**，不新建表 |
| 抓取 | `WebFetchTool`（单页）、arXiv 抓取 | 无 RSS / GitHub / 站点批量；网页→MD 质量不足 | 新增抓取器家族，统一输出规范化 MD |
| 加工 | 无 | 重复导入、冲突、链接不规范化 | 指纹去重 → 冲突检测 → MD 规范化 |
| 入库 | `wiki_notes_*` + `index_queue` + `MemoryService` | 无 | 全复用 |
| 调度 | `scheduled_task`（cron）+ `wiki_sync_queue` | 无增量更新逻辑 | 指纹对比，变了才动 |

---

## 2. 已核实的关键事实（2026-08-01）

### 2.1 `wiki_sources` 表现有结构（`src-tauri/crates/entities/src/wiki_sources.rs`）

| 字段 | 类型 | 用途 |
|------|------|------|
| id | String PK | 来源 ID |
| wiki_id | String | 关联的 wiki |
| source_type | String | 来源类型（**扩展取值** url/rss/github/obsidian/local/file） |
| source_path | String | URL / 文件路径 |
| title | String | 标题 |
| mime_type | String | MIME |
| size_bytes | i64 | 大小 |
| **content_hash** | String | **内容指纹（已有！去重复用）** |
| metadata_json | Json? | 扩展配置（URL 参数、fetch 选项） |
| created_at / updated_at | i64 | 时间戳 |

**P2 需新增字段（migration）**：`schedule_cron TEXT NULL`、`last_fetched_at INTEGER NULL`、`status TEXT DEFAULT 'active'`（active/paused）。

### 2.2 可复用挂接点清单

| 能力 | 位置 | API |
|------|------|-----|
| 单页抓取 | `crates/tools/src/tools/web_fetch.rs` | WebFetchTool（含 `is_safe_url_deep` URL 安全校验） |
| HTTP client | `crates/providers/src/lib.rs` | `build_default_http_client()` |
| Wiki 页面 CRUD | `src-tauri/src/commands/wiki.rs` | `wiki_notes_create/update/delete/get/search` |
| RAG 索引入口 | `src/index_queue.rs` | `enqueue_job_sync(JOB_TYPE_INDEX_DOCUMENT)` |
| Wiki 同步事件 | `wiki_sync_queue` 表 | `wiki_sync_enqueue/process_pending`（SyncEventType: note_created/updated/deleted） |
| cron 调度 | `src-tauri/src/commands/scheduled_task.rs` | CronJobStore（5 字段 cron） |
| 文档解析 | `crates/document-parser` | `extract_text(path, mime)` + OCR |
| Memory 写入 | `crates/trajectory/src/memory_providers/service.rs` | MemoryService::AddMemoryRequest |
| LLM 加工 | `src-tauri/src/commands/llm_wiki.rs` | `llm_wiki_ingest/compile/query` |
| 前端页面 | `src/pages/KnowledgeHubPage.tsx`、`src/components/wiki/IngestPanel.tsx` | 可扩展入口 |

---

## 3. 总体架构：五层管道

```
知识源 Source ──→ 抓取器 Fetch ──→ 内容加工 Process ──→ 入库 Store ──→ 更新调度 Refresh
  URL / RSS       网页/RSS/Git      指纹去重→冲突→      Wiki 页面      cron 定时
  GitHub/本地      目录扫描          MD 规范化          RAG 索引       事件触发
                                                       Memory 抽取    增量 diff
```

### 3.1 增量更新闭环（核心设计）

```
知识源 → 抓取 → 规范化 MD → sha256 指纹
                              ├─ 指纹 == content_hash → 跳过（不碰 wiki、不碰索引）
                              └─ 指纹 != content_hash → wiki_notes_update（留版本历史）
                                                    → wiki_sync_queue 事件
                                                    → index_queue 重索引该页
                                                    → 更新 content_hash
```

**原则：内容变了才动**。挂几十个源也不会打爆索引队列，且每个页面天然带可回滚版本。

---

## 4. 分阶段实施计划

### P1 — MVP：URL 单页抓取闭环（全复用，无新依赖）

**目标**：输入 URL → 生成 Wiki 页面 + 自动 RAG 索引，手动触发，跑通闭环。

**改动清单**：
1. 新增 `fetch_url_to_wiki { url, title?, wiki_id? }` 命令
   - 复用 WebFetchTool 抓取 → 主文本 → 规范化 MD（frontmatter: `source`, `url`, `fetched_at`）
   - → `wiki_notes_create`（已存在则按 title 策略处理）→ `index_queue` enqueue
   - 注册：`commands/mod.rs` + `lib.rs` generate_handler![]
2. 前端：`IngestPanel.tsx` 增加 URL 输入 + 结果展示（新建/已存在/失败）
   - 类型在 `src/types/`，i18n 补齐 11 语言

**验收**：命令返回 note id + 状态；页面出现在 Wiki 编辑器/图谱；RAG 搜索可命中；`clippy -D warnings`、`cargo fmt`、`npm run typecheck`、`npm run format` 全绿。

### P2 — 源管理 + RSS + 定时刷新

**目标**：知识源变成「清单 + 调度」的持续更新机制。

**改动清单**：
1. **migration**：`wiki_sources` 加 `schedule_cron` / `last_fetched_at` / `status`；`source_type` 扩展取值
2. **源管理命令**：`knowledge_source_list/create/update/delete/fetch_now/fetch_all`（DAO 扩展 `crates/dao/src/repo/wiki.rs`）
3. **RSS 抓取器**：引入 `feed-rs`；url/rss 统一走规范化 MD → sha256 指纹对比 → 增量更新（见 3.1）
4. **cron 定时刷新**：复用 `scheduled_task` 注册任务定期调 `knowledge_source_fetch_all`
5. **前端**：KnowledgeHubPage 新增「知识源」Tab（列表/增删改/立即抓取/调度周期）

**依赖**：feed-rs（纯 Rust，无 C 依赖）

**验收**：注册一个 RSS 源，定时刷新只更新变化的页面，重复内容零写入；前端 Tab 完整可操作。

### P3 — 开源知识库导入 + 站点批量

**改动清单**：
1. **GitHub 导入**：引入 `git2`（vendored-libgit2）→ `github_repo_import { repo, path_filter }`：clone docs 目录 → 逐文件 document-parser → 指纹去重 → 入库 + 索引
2. **sitemap 批量**：`sitemap_crawl { base_url }` → 解析 sitemap.xml → 批量入源
3. **Obsidian vault 双向同步**：复用 `wiki_import_obsidian_vault` + 新增导出/差异同步

**依赖**：git2（打包体积 +~30MB，注意 vendored 配置）

**风险**：git2 编译时间与体积；GitHub API 限流（优先 git clone 协议）。

### P4 — 智能增量

**改动清单**：
1. 页面级 diff 更新（复用 `wiki_note_versions/restore_version` 版本历史）
2. 冲突自动处理：新 > 旧，或标记 pending 待人工确认（对齐 KNOWLEDGE.md 冲突规则）
3. AI 摘要/翻译（复用 `llm_wiki`）
4. 精华自动抽取进 Memory（`MemoryService`）

---

## 5. 技术选型

| 项 | 选型 | 理由 |
|----|------|------|
| RSS/Atom 解析 | `feed-rs` | 纯 Rust 无 C 依赖 |
| 网页→MD | 在 WebFetchTool 主文本上自研轻量转换 | 已有 KitHtmlCleaner，避免重型依赖 |
| Git 拉取 | `git2`（vendored） | 原生 libgit2，Tauri 打包需注意 |
| 内容指纹 | sha256（规范化 MD 后计算） | 标准库，无依赖 |
| 抓取安全 | 复用 `is_safe_url_deep` + robots.txt + 限速 + 超时 | WebFetchTool 已有 URL 安全校验 |

## 6. 风险与对策

| 风险 | 对策 |
|------|------|
| 爬虫合规（robots/限流/IP 黑名单） | robots.txt 检查 + 请求限速 + 缓存；RSS/官方 API 优先于硬爬 |
| 重复导入污染知识库 | 指纹去重（content_hash 已有） |
| 冲突覆盖用户手工编辑 | 冲突检测：新 > 旧 或标记 pending；版本历史可回滚 |
| git2 打包体积 | vendored 编译配置提前验证，或降级为 shell git clone |
| 索引队列爆炸 | 指纹对比跳过未变化内容，只重索引变化的页 |

## 7. 任务清单（TaskList 关联）

| # | 阶段 | 任务 | 状态 |
|---|------|------|------|
| 2 | P1 | P1-1 实现 fetch_url_to_wiki 后端命令 | ✅ 完成 |
| 1 | P1 | P1-2 前端「抓取到知识库」UI 入口 | ✅ 完成 |
| 6 | P2 | P2-1 扩展 wiki_sources 表 + 源管理 CRUD | ✅ 完成 |
| 3 | P2 | P2-2 RSS 抓取器 + 指纹去重 | ✅ 完成 |
| 5 | P2 | P2-3 前端知识源管理 Tab + cron 定时刷新 | ✅ 完成 |
| 4 | P3 | P3-1 GitHub 开源知识库导入 | ✅ 完成 |
| 7 | P3 | P3-2 sitemap 批量爬取 + Obsidian vault 双向同步 | ✅ 完成（vault 双向复用既有 wiki_import_obsidian_vault + wiki_export_markdown） |
| 8 | P4 | P4 智能增量：diff + 冲突 + AI 摘要进 Memory | ✅ 完成（diff 走 wiki_note_versions；冲突=用户编辑保护；精华自动沉淀 Memory） |

---

## 8. 实施记录（2026-08-01，v1.1）

### 新增/修改文件清单

**后端**
- `src-tauri/crates/dao/src/migrations/v114_wiki_sources_schedule.rs` — wiki_sources 加 schedule_cron/last_fetched_at/status（SQLite PRAGMA 幂等 + PG IF NOT EXISTS）
- `src-tauri/crates/entities/src/wiki_sources.rs` — 实体加 3 字段
- `src-tauri/crates/harness/src/wiki_dtos.rs` — WikiSource / InsertWikiSourceInput 加 3 字段
- `src-tauri/crates/dao/src/repo/wiki_source_repository.rs` — model_to_dto / insert 补字段
- `src-tauri/crates/dao/src/repo/wiki.rs` — 新增 list_all_sources / get_source_by_id / update_source_fields / update_source_fetch_meta / delete_source_by_id
- `src-tauri/crates/agent/src/ingest_pipeline.rs` — 2 处 InsertWikiSourceInput 补字段
- `src-tauri/src/commands/knowledge_source.rs`（新）— 全部命令 + 单元测试
- `src-tauri/src/commands/mod.rs` + `src-tauri/src/register_commands.rs` — 模块声明与命令注册（10 个命令）
- `src-tauri/src/init/services.rs` — CronScheduler handler 加 knowledge_source_fetch_all 分支
- `src-tauri/Cargo.toml` — 新增 feed-rs 依赖

**前端**
- `src/types/wiki.ts` — KnowledgeSource / FetchUrlResult / FetchSourceResult 等类型
- `src/stores/feature/knowledgeSourceStore.ts`（新）+ `src/stores/index.ts` 导出
- `src/components/settings/KnowledgeSourceTab.tsx`（新）— URL 抓取 / 源管理 / GitHub 导入 / sitemap / 定时刷新
- `src/components/settings/SourceManager.tsx` — 新增「知识源」Tab
- `src/i18n/locales/*.json` — 11 语言补 key（脚本：scripts/i18n-add-knowledge-source*.mjs）

### 实现决策（与 v1.0 计划的偏差）

| 计划 | 实际 | 理由 |
|------|------|------|
| P3 GitHub 用 git2 | **GitHub Git Trees API + raw 下载**（零新依赖） | 避免 libgit2 vendored 编译（10+ 分钟）与 +30MB 打包体积；未认证 60 req/h 个人够用 |
| P4 AI 摘要/翻译 | 复用既有 llm_wiki（不新增） | 已有能力，避免重复实现 |
| P4 冲突处理 | 用户编辑保护（user_edited=true 跳过覆盖） | 比「新>旧」更符合个人知识库语义 |
| cron 触发 | CronScheduler handler 新分支 task_type=knowledge_source_fetch_all + `knowledge_source_schedule_sync` 命令注册 | 复用现有调度循环，纯 DB 入队（不依赖 Tauri 事件） |

### 已实现命令

`fetch_url_to_wiki` / `knowledge_source_list` / `knowledge_source_create` / `knowledge_source_update` / `knowledge_source_delete` / `knowledge_source_fetch_now` / `knowledge_source_fetch_all` / `knowledge_source_schedule_sync` / `github_repo_import` / `sitemap_crawl`

### 验证状态

- [x] `cargo check -p axagent` 通过
- [x] `cargo clippy -p axagent --lib -- -D warnings` 通过（含复盘后 3 处 lint 修复）
- [x] `cargo clippy -p axagent-dao --lib -- -D warnings` 通过
- [x] `npm run typecheck` 通过
- [x] `npm run format`（dprint）通过
- [x] `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::knowledge_source` **5 passed**（复盘后解决 0xc0000139 根因，测试真实执行）
- [ ] axagent-dao 测试编译失败为 v113 预存问题（E0308/E0425 在 v113_unified_knowledge_graph.rs 测试代码，与本次改动无关）

---

*计划文档由 WorkBuddy 于 2026-08-01 制定并实施。*

---

## 9. 复盘与缺陷修复（2026-08-01，v1.2）

实施后全量复盘，发现并修复 8 项缺陷（按严重度）：

| # | 严重度 | 缺陷 | 修复 |
|---|--------|------|------|
| 1 | 高 | `github_repo_import` 的 raw URL 用 `/HEAD/`（怀疑 raw CDN 需分支名）——**curl 实测 raw CDN 支持 HEAD（HTTP 200），不成立** | 已验证，无需修复 |
| 2 | 高 | `parse_github_repo` 无法解析 `https://github.com/...` 完整 URL（split 后 parts[0]="https:" 不匹配分支）——自动修复为 position 定位 github.com 段 | 已修复 + 单测通过 |
| 3 | 高 | 前端新建源不传 wikiId（form 中 hidden 空值）→ 源无法关联 wiki | 后端 `knowledge_source_create` 空 wiki_id 兜底取第一个 wiki |
| 4 | 高 | frontmatter 注入：title/url 含引号、换行破坏 YAML frontmatter | `yaml_escape`（\\ 与 " 转义、换行转空格），url/rss/github 三处统一 |
| 5 | 中 | `knowledge_source_schedule_sync` 不幂等（CronJobStore::add 内存直接 push）→ 重复注册导致定时任务重复触发 | 按 name 查重 → `update` 复用；并加 5 字段 cron 校验 |
| 6 | 中 | 前端「配置」按钮死按钮（onViewConfig 未接线）+ 源编辑/暂停/启用 UI 缺失（P2-3 验收「增删改」只做到删） | 新增编辑 Modal（title/status/scheduleCron），configure 按钮打开编辑 |
| 7 | 中 | RSS 条目不沉淀 Memory（只有 url 源） | `created` 时 deposit_to_memory |
| 8 | 中 | GitHub 导入不登记知识源 → 无法增量更新 | 导入后登记 source_type="github" 源；`sync_one_source` 加 github 分支（复用 github_sync，指纹对比天然增量） |
| 9 | 低 | i18n 5 处 placeholder 硬编码违规 | 全部提取为 i18n key，check-hardcoded-i18n.sh 全绿 |
| 10 | 低 | 测试 exe 0xc0000139 曾被误判为「环境瞬态问题」 | **根因确认**：Windows 直接 `cargo test` 缺 `__TAURI_WORKSPACE__=true` 环境变量 → build.rs 的 Common Controls v6 manifest 不嵌入 → 链接 tauri 的测试 exe 启动即崩（tauri issue #11028）。已写入 AGENTS.md 常用命令区 |
| 11 | 低 | clippy：agent/main build.rs 的 `map_or` + `yaml_escape` 连续 replace | 均按 clippy 建议修复 |

**复盘方法论教训**：
- 单元测试因 0xc0000139 从未真正执行 → parse_github_repo 的 bug 未被测试捕获。**测试跑不起来 = 测试形同虚设**，修复后 5 个测试全部真实执行通过。
- 复盘时对 raw `/HEAD/` 的担心通过 curl 实测推翻，避免无效修复——**先验证再改**。

---

## 10. 第二轮复盘与缺陷修复（2026-08-01，v1.3）

针对增量更新管道的「承诺 vs 实现」逐条核查（doc:74 指纹去重、doc:52 事件、doc:201 用户编辑保护、doc:155 只重索引变化页），发现并修复 **3 致命 + 4 中级 = 7 项**缺陷：

| # | 严重度 | 缺陷 | 修复 |
|---|--------|------|------|
| 1 | 致命 | **`fetched_at` 时间戳混入 frontmatter** → 指纹每次抓取必变，增量闭环失效（「内容变了才动」从未生效，每次全量重写 + 重索引） | 从 `normalize_fetched_md`、`fetch_url_content`、`github_sync`、RSS 正文四处统一移除；抓取时间记录在 `wiki_sources.last_fetched_at`，不进正文 |
| 2 | 致命 | **`update_note` 传 content 即置 `user_edited=1`** → 第二次抓取更新后，第三次起被自身编辑保护永久 skipped | dao `note.rs` 新增 `update_note_from_pipeline`（只更新 title/content/content_hash/updated_at，不触碰 user_edited）；`upsert_note` 更新路径改用它 |
| 3 | 致命 | **`wiki_sync_queue` 事件未接入**（`wiki_sync_enqueue` 已注册但零调用）→ 增量更新不触发下游（RAG 重索引、Memory 抽取） | `upsert_note` 在 create/update 后 `enqueue_wiki_sync` 纯 DB 入队 `note_created`/`note_updated`（target_type="note"） |
| 4 | 中 | **`sitemap_crawl` 无条件 insert 无查重** → 重复抓取无限堆积同 URL 源 | 预载 `list_all_sources` 的 url 型源集合，循环内同 URL 已登记则跳过（action=skipped） |
| 5 | 中 | **20+ 处裸 `Err("中文...")` 无错误码** → 前端无法按码 i18n（违反 AGENTS.md 错误码规范） | `error_code.rs` 新增 `knowledge_source` 模块 **19 个码**；URL/HTTP/RSS/sitemap/GitHub/API 等全部错误码化（`ErrorResponse::err_with_detail`）；11 语言 `error` 段同步（`scripts/check-errorcode-alignment.mjs`：274 码全对齐） |
| 6 | 中 | 实际 `#[test]` 仅 5 个，与「全部命令+单测」表述不符 | 新增 `normalize_md_is_time_stable` 回归测试（两次规范化结果一致 + 指纹一致 + 无 fetched_at），防致命 1 复发 |
| 7 | 中 | **`upsert_note` 按标题查重** → 不同 URL 同标题互相覆盖 | 改为 `find_note_by_source_ref` 按 `source_refs` 精确匹配优先，标题兜底 |

**契约落地**：
- 新增 19 码均为 `KNOWLEDGE_SOURCE_*`，满足前端 `error.${code}` 翻译（`src/lib/errorI18n.ts` 命中即译、未命中回退 detail）。
- 错误码作为外部契约：`error_code.rs` 整文件 `#![allow(dead_code)]`，码只增不删；`check-errorcode-alignment.mjs` CI 校验前后端对齐。

**验证状态（v1.3）**：
- [x] `cargo fmt` 通过
- [x] `cargo check -p axagent --lib` 通过
- [x] `cargo clippy -p axagent --lib --tests -- -D warnings` 通过（0 警告）
- [x] `npm run format`（dprint）通过
- [x] `node scripts/check-errorcode-alignment.mjs` 通过（274 码 × 11 语言）
- [x] `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::knowledge_source` **6 passed**（含新增 `normalize_md_is_time_stable` 回归）
