# 存储层与数据库迁移

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：storage-migration

## 定位

存储层横跨两个 foundation / implementor crate：

- **数据定义**：`axagent-core/crates/entities`（201 个 `.rs` 文件）——SeaORM `Model` 与表结构，是共享数据模型的唯一权威，只依赖 `axagent-harness`。
- **数据访问**：`axagent-infra/crates/dao`（`src/repo/` 下 97 个 `.rs` 文件）——repository 模式封装查询，另有连接池、迁移执行器、声明式 schema 收敛引擎。
- **平台数据迁移**：`axagent-infra/crates/migration`——在 `~/.axagent`、`~/.openclaw`、`~/.hermes` 等目录之间导入/检测平台数据，**不是 schema 迁移**。
- **缓存 / CRDT**：`axagent-infra/crates/cache` 与 `axagent-runtime/crates/crdt`。

> 注意：仓库中不存在根目录下的 `crates/entities`、`crates/dao`、`crates/migration`，三者分别在 `axagent-core/` 与 `axagent-infra/` 两个 workspace 根下。

## 当前行为

### 实体定义（SeaORM）

- 每个表一个文件，实体以 `#[sea_orm(table_name = "…")]` 标注。示例：`axagent-core/crates/entities/src/paper_positions.rs:12-44`、`axagent-core/crates/entities/src/paper_portfolios.rs:15-35`。
- `entities` 是 foundation 叶子节点，仅依赖 `axagent-harness`；不得反向依赖任何实现层。
- 表字段类型统一为 `String` / `f64` / `i64` / `Option<T>`，时间戳用 `i64` 秒级存整数（如 `created_at`、`updated_at`）。

（对应版本：当前实现）

### repository 模式

- 复杂查询集中在 `axagent-infra/crates/dao/src/repo/`，按领域一文件（如 `knowledge_graph.rs`），简单 CRUD 仍可在 entity 层用 sea-orm。
- repository 对外暴露的输入输出与 `axagent-harness` 的 DTO 对齐，避免在 dao 层另造一套类型。
- 连接创建入口：`create_pool`（`axagent-infra/crates/dao/src/db.rs:269`）、`create_pool_for_profile`（`axagent-infra/crates/dao/src/db.rs:393`）、测试用 `create_test_pool`（`axagent-infra/crates/dao/src/db.rs:827`）。

（对应版本：当前实现）

### 迁移清单：v100 基线 + v101–v233 增量，现已清空

- **历史双层架构**：`v100_consolidated` 是上游全部 DDL（表 / 索引 / 触发器 / 种子数据）的单一基线，`v101–v233` 是本地各功能模块的增量。见 `axagent-infra/crates/dao/src/migrations/mod.rs:13-14`。
- **当前状态**：`CURRENT_VERSION = 233`（`axagent-infra/crates/dao/src/migrations/mod.rs:53`），`MIGRATIONS = &[]`（`:112`）。本模块的执行路径成为 no-op（`:7`）。
- **`CURRENT_VERSION` 必须保持 233、不得改 0**：理由见常量自身文档 `axagent-infra/crates/dao/src/migrations/mod.rs:102`，以及与「本程序认识的最高版本号」的语义绑定 `:343-345`。
- **高水位线风险**：`run_migrations` 只看 `MAX(version)`，任何**后补进清单的中间版本号**会被永久跳过。实测出现过 `applied_max = 227 == CURRENT_VERSION` 而 v101 / v139 从未执行的静默降级。见 `axagent-infra/crates/dao/src/migrations/mod.rs:234-259`。
- **护栏**：若有人把迁移加回 `MIGRATIONS`，测试会直接变红并提示先重读 `MIGRATIONS` / `run_migrations` / `missing_versions` 三处文档。见 `axagent-infra/crates/dao/src/migrations/mod.rs:862-863`。

（对应版本：当前实现，清空动作发生于 2026-09-16）

### 声明式 schema 收敛（reconcile）

- 入口分三块：`expected`（期望 schema）、`extras`（索引 / FTS / 约束等附加声明）、`apply`（差异落地）。见 `axagent-infra/crates/dao/src/reconcile/mod.rs:1-28`。
- `expected` 由 entities + extras 生成期望 schema，再与实际 DB 比较产出动作。见 `axagent-infra/crates/dao/src/reconcile/expected.rs:1-32`。
- 结论：**引擎是唯一建表来源**，迁移清单清空后新增表通过「改 entity + 改 extras + 启动时 reconcile」完成，而不是写新迁移。

（对应版本：v2.9.7，`PLAN-declarative-schema-sync.md`）

### SQLite / PostgreSQL 方言差异

| 能力 | SQLite | PostgreSQL | 证据 |
|---|---|---|---|
| 全文检索 | FTS5 虚拟表；`messages_fts` 用 `tokenize = 'unicode61'`，`trajectories_fts` 用 `porter` | `tsvector` + GIN 索引 | `axagent-infra/crates/dao/src/reconcile/extras.rs:292-346` |
| 中文检索 | 无 `ngram` 分词器，CJK 检索能力受限 | 可用词典/扩展 | 同上 |
| JSON | 文本列 + 应用层序列化 | `JSONB` + 生成列 `PatternTsvDecl` | 同上 |
| 索引声明 | 普通索引 / 唯一索引 | 另有 GIN、表达式索引 | `axagent-infra/crates/dao/src/reconcile/extras.rs:292-346` |

- 后端选择与连接建立：`axagent-infra/crates/dao/src/db.rs:269`（`create_pool`）按连接串分流 SQLite 文件与 PG 连接。
- 默认库路径：`default_db_path`（`axagent-infra/crates/dao/src/db.rs:328`）；按 profile 的路径：`profile_db_path`（`:352`）。
- 连接串脱敏：`redact_db_url`（`axagent-infra/crates/dao/src/db.rs:38`），用于日志与错误输出。

（对应版本：v2.9.4，`CHANGELOG.md:905`「v119 迁移：PostgreSQL JSONB」）

### 缓存与 FTS5

- **`EmbeddingCache`**：`axagent-infra/crates/cache/src/cache.rs:39`，容量上限由构造参数 `max_entries` 决定（`:49`），提供 `get` / `insert` / `remove` / `clear` / `import_entries`。
- **`TextHashCache`**：`axagent-infra/crates/cache/src/cache.rs:123`，同样按 `max_entries` 限容（`:133`），用于文本哈希去重。
- **FTS5 表**由 `extras` 声明式创建，不在 entity 层；SQLite 与 PG 各走一套声明，见 `axagent-infra/crates/dao/src/reconcile/extras.rs:292-346`。

（对应版本：当前实现）

### CRDT 存储

- 算法族：RGA 序列、OR-Set、G-Counter、LWW-Register、VersionVector。见 `axagent-runtime/crates/crdt/src/lib.rs:1-30`。
- 持久化表：`sync_change_logs`，字段 `id` / `device_id` / `doc_id` / `revision` / `op` / `shadow` / `created_at`。见 `axagent-core/crates/entities/src/sync_change_log.rs:30-55`。
- 语义：变更日志是 CRDT 同步的追加流，`op` 为操作类型，`shadow` 保存操作后的影子状态；同步方按 `revision` 与 `device_id` 合并。

（对应版本：当前实现）

### 唯一约束缺失与幂等回填

- `knowledge_graph` 表**没有唯一约束**（历史遗留），因此依赖自然键 `(source_id, relation, target_id)` 查找去重。见 `axagent-infra/crates/dao/src/repo/knowledge_graph.rs:612-628`。
- 写入策略：`insert` 前先按自然键查重，已存在则复用既有行，形成**幂等回填**；重复跑同一批回填不会产生重复边。见 `axagent-infra/crates/dao/src/repo/knowledge_graph.rs:649` 附近。

（对应版本：当前实现）

### 平台数据迁移（≠ schema 迁移）

- `axagent-infra/crates/migration/src/migration/mod.rs:1-28` 负责识别与导入 `~/.axagent`、`~/.openclaw`、`~/.hermes` 三个平台目录的数据。
- schema 迁移与建表**不在**该 crate，而在 `axagent-infra/crates/dao/src/migrations` 与 `reconcile`。两者不可混称。

（对应版本：当前实现）

## 关键代码入口

**后端命令**

- 数据库初始化与状态查询命令分散在 `src-tauri/src/commands/`（本次未逐条取证命令名，避免与其它路线重复）。

**引擎**

- 连接与方言：`axagent-infra/crates/dao/src/db.rs`
- 迁移执行器：`axagent-infra/crates/dao/src/migrations/mod.rs`
- 声明式收敛：`axagent-infra/crates/dao/src/reconcile/mod.rs`、`expected.rs`、`extras.rs`
- repository：`axagent-infra/crates/dao/src/repo/`（97 个文件）
- 缓存：`axagent-infra/crates/cache/src/cache.rs`
- CRDT：`axagent-runtime/crates/crdt/src/lib.rs`
- 平台迁移：`axagent-infra/crates/migration/src/migration/mod.rs`

**前端**

- 本功能无独立前端页面；数据经命令面暴露给各业务页。

**数据表 · 迁移**

- 实体：`axagent-core/crates/entities/src/`（201 个文件）
- 同步日志表：`axagent-core/crates/entities/src/sync_change_log.rs`
- 建表来源：声明式 `reconcile`（迁移清单为空）

## 配置与开关

- **数据库路径**：默认 `default_db_path()`（`axagent-infra/crates/dao/src/db.rs:328`）；按 profile 切换用 `profile_db_path()`（`:352`）与 `create_pool_for_profile()`（`:393`）。
- **连接串**：支持 SQLite 文件与 PostgreSQL 连接串两种形态，由 `create_pool` 分流（`axagent-infra/crates/dao/src/db.rs:269`）。
- **声明式收敛开关**：启动时由 `reconcile` 自动执行；`expected` / `extras` 的差异决定是否 `apply`。
- **迁移清单**：`MIGRATIONS` 恒为空数组，`CURRENT_VERSION` 恒为 233——二者都是硬约束，不是可配置项。
- **缓存容量**：`EmbeddingCache::new(max_entries)`、`TextHashCache::new(max_entries)`，由调用方传入，代码内无硬编码默认值。

## 已知限制与坑

1. **不能把迁移加回 `MIGRATIONS`**：护栏会红，且高水位线会让后补的中间版本号永久跳过（`axagent-infra/crates/dao/src/migrations/mod.rs:234-259`、`:862-863`）。
2. **`CURRENT_VERSION` 不能改成 0**：它是「本程序认识的最高版本号」的另一半判据，改了会让存量库判定失效（`axagent-infra/crates/dao/src/migrations/mod.rs:102`、`:343-345`）。
3. **SQLite 中文检索弱**：FTS5 未启用 `ngram`，CJK 分词依赖 `unicode61`，中文召回率低于 PG（`axagent-infra/crates/dao/src/reconcile/extras.rs:292-346`）。
4. **方言特性不可平移**：PG 的 `JSONB` / GIN / 生成列在 SQLite 下降级为文本列与普通索引，写 SQL 时必须按方言分支。
5. **`knowledge_graph` 无唯一约束**：并发写入仍可能产生重复边，幂等只对串行回填有效（`axagent-infra/crates/dao/src/repo/knowledge_graph.rs:612-628`）。
6. **两个「migration」同名不同义**：`crates/migration` 是平台数据导入，`dao/src/migrations` 是 schema 版本表；查问题时先确认在哪一层。
7. **缓存无 TTL**：`EmbeddingCache` / `TextHashCache` 只按 `max_entries` 淘汰，不按时间过期；数据更新后需显式 `remove` / `clear`。
8. **实体文件数量易被旧文档写错**：当前 `axagent-core/crates/entities/src` 为 201 个 `.rs`，`axagent-infra/crates/dao/src/repo` 为 97 个 `.rs`。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.9.4 | 2026-08-14 | v119 迁移：PostgreSQL JSONB 支持 | `CHANGELOG.md:905` |
| v2.9.6 | 2026-08-20 | v223 自愈迁移 | `CHANGELOG.md:752` |
| v2.9.7 | 2026-08-24 | 声明式 schema 同步（reconcile / expected） | `PLAN-declarative-schema-sync.md` |
| 未发布 | — | 存储层随 8 workspace 拆分归入 `axagent-core` / `axagent-infra` | `CHANGELOG.md:119` |

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `PLAN-memory-kb-reflow-id-space.md` | active | v2.9.9 | 知识库回流 ID 空间 |
| `AUDIT-db-upsert-dialect-2026-09-17.md` | active | v2.9.7 | upsert 方言差异 |
| `AUDIT-kg-cross-kb-merge-2026-09-17.md` | done | v2.9.7 | 知识图谱跨库合并 |
| `PLAN-tombstone-code-wiring.md` | done | v2.9.7 | 墓碑码接线 |
| `AUDIT-tombstone-code-2026-09-16.md` | done | v2.9.7 | 墓碑码审计 |
| `AUDIT-wiki-graph-edges-2026-09-15.md` | active | v2.9.7 | Wiki 图谱边 |
| `AUDIT-crdt-integration-2026-09-13.md` | active | v2.9.7 | CRDT 集成 |
| `PLAN-causal-edge-layer.md` | 未标注 | v2.9.7 | 因果边层 |
