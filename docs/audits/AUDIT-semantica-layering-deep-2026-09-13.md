# AUDIT — Semantica 分层维度专项深度对比（2026-09-13 · 三轮）

> 触发：用户提问「该开源仓库在分层上没有本地借鉴的内容吗？」
> 覆盖面：`AUDIT-semantica-vs-axinvest-2026-09-13.md` **12 个维度里不含「架构分层 / 依赖方向」** —— 本轮补上。
> 取证方式：① 全量静态依赖分析（AST 解析**绝对+相对**导入，见 `output/tmp-semantica-src/analyze_deps.py`）② `cargo metadata --no-deps` 解析 workspace 依赖矩阵（`analyze_crates.py`）③ 5 名并行侦察兵源码级取证 ④ 我方 grep 实证。**git 零接触。**

---

## 0. 结论（先说判词）

| 问题                                     | 答案                                                                                                                  |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| 分层**机制**上，Semantica 有可借鉴的吗？ | **几乎没有 —— 它的分层是失败的**。28 包长成「星型扇出 + 3 个跨包环 + 具体实现直连」，`core/` 自称核心却**零抽象基类** |
| 那本地就没什么可借的了？                 | **不。它有极高的「反面教材价值」，而且实证了我方一条一直在犯却没被抓的病。**                                          |
| 真正的收获                               | ★ **双方同病：都缺「模块级」分层护栏。区别只在护栏粒度** —— 见 §4。由此产出 4 条借鉴项（§6）                          |

---

## 1. 量化对比（硬数据）

| 指标                     | Semantica                                                                | AxInvest                                                               |
| ------------------------ | ------------------------------------------------------------------------ | ---------------------------------------------------------------------- |
| 顶层单元数               | **28 个包** / 377 py 文件                                                | **43 个 crate**                                                        |
| 依赖方向强制力           | **无**（Python 无编译期约束 + 无 import-linter/tach/grimp）              | **cargo 编译期强制 DAG**（环 = 编译不过）                              |
| 跨包/跨 crate 环         | **3 个 SCC**（见 §2.2）                                                  | **0**（结构性保证）                                                    |
| 扇入最高单元             | `utils` 25 / `provenance` 22                                             | **`axagent-harness` 36**                                               |
| 扇入最高单元是否为真契约 | ❌ `utils` 是杂物抽屉；`core/`（自称核心）扇入仅 1 且**零 ABC/Protocol** | ✅ `harness` **扇出 = 0**（零同 workspace 依赖），教科书级依赖倒置     |
| 拓扑分层                 | **星型**：`(top-level)`（cli.py 4871 行）扇出到 22 个包；15 个包扇入 0   | **8 层清晰 DAG**：L0 契约 → L8 主 crate（见 §3）                       |
| 后端切换                 | `if/elif backend_type == "..."` 字符串硬编码                             | 注册表：`HashMap<String, Arc<dyn ProviderAdapter>>` + `register()`     |
| 依赖注入                 | 以**模块级全局单例**为主（`_instance=None` + `get_instance()`）          | **真构造注入容器**：`RuntimeHarness::new(HarnessDeps{...})`            |
| 分层护栏脚本             | 无                                                                       | **无**（28 个 `scripts/` 里零个查依赖方向；前端 eslint 无 boundaries） |

---

## 2. 对方分层形态实证（这是失败样本，不是范本）

### 2.1 自称的 `core/` 不是抽象层

`core/__init__.py:51-63` 导出 `Config, ConfigManager, LifecycleManager, PluginRegistry, Semantica, MethodRegistry`。
`grep -r "ABC\|Protocol\|abstractmethod" semantica/core/` → **NONE**。
**判定：`core` 是「配置 + 生命周期 + 插件注册表 + 编排门面」的具体实现层，扇入 = 1（只有 top-level 用它）。**

### 2.2 三个跨包环（SCC，Python 无强制 → 长出来了）

| 环       | 成员                                | 性质                                                         |
| -------- | ----------------------------------- | ------------------------------------------------------------ |
| 2 元     | `semantic_extract ↔ split`          | 领域内互相调用（split 调 NERExtractor，extract 调 splitter） |
| **3 元** | `change_management ↔ kg ↔ ontology` | 版本治理嵌进领域图                                           |
| 2 元     | `context ↔ graph_store`             | 决策层 ↔ 存储层双向                                          |

对方用**函数内延迟导入**掩盖环（`ontology/engine.py:35` 有注释自陈：`# Deferred to avoid circular import: change_management → ontology → change_management`）。**环没消失，只是藏起来了。**

### 2.3 「有门面、无倒置」+ 具体实现直连（量化）

| 高层包    | → 具体实现                                         | 次数 | 证据                                                                         |
| --------- | -------------------------------------------------- | ---- | ---------------------------------------------------------------------------- |
| `context` | `graph_store.store.GraphStore`                     | 6    | `decision_methods.py:13`、`decision_query.py:84`、`decision_recorder.py:78`… |
| `context` | `vector_store.decision_embedding_pipeline`         | 2    | `context_retriever.py:86`、`decision_context.py:35`                          |
| `kg`      | `semantic_extract.{NER,Relation,Triplet}Extractor` | 3    | `kg/graph_builder.py`                                                        |
| `split`   | `semantic_extract.NERExtractor`                    | 2    | `split/methods.py`                                                           |

且它依赖的 `GraphStore` **本身是具体类内含 if/elif**（`graph_store/graph_store.py:573-602`）⇒ 依赖方向是「依赖具体门面」，**不是依赖抽象**。

### 2.4 三个 store 无共同基类

`OxigraphStore` / `Neo4jStore` / `QdrantStore` 全部无基类、无 Protocol；调度三处 if/elif（`triplet_store.py:99-155`、`graph_store.py:573-602`、`vector_store.py:178-250`）。
`triplet_store/registry.py:48 MethodRegistry` 是**方法覆盖注册表，不注册后端**。

### 2.5 唯一真抽象（全项目罕见）

`change_management/version_storage.py:101 class VersionStorage(ABC)` → `:166 InMemoryVersionStorage` / `:267 SQLiteVersionStorage`；`managers.py:39 class BaseVersionManager(ABC)`。
**这是全仓唯一「面向抽象 + 可换实现」的地方，值得借（→ §6 L3）。**

---

## 3. 我方分层形态实证（含硬数据）

**依赖倒置真成立**：`crates/harness/Cargo.toml` **零 `path = "../"`**（不含任何同 workspace 依赖），却被 **36 个 crate** 依赖。

**拓扑分层（Kahn）**：

```
L0 ← 最底层（契约/基础）: axagent-harness, axagent-agent-command-types, axagent-disk-cache, axagent-rt-dashboard, axagent-rt-theme
L1: agent, agent-macro, astock-data, cache, crdt, credential, crypto, device, document-parser,
    entities, gateway, kit, market-sim, mcp, notification, npm, orchestrator, plugins,
    prompt-guard, quant, rt-webhook, runtime-core, telemetry, schema-gen
L2: dao, providers, rt-messaging, search
L3: migration, rt-workflow, storage, trajectory
L4: analysis-engine          L5: company-runtime, tools
L6: runtime                  L7: axagent(主 crate, out=39)     L8: mobile
```

**真依赖注入**：`crates/runtime/src/harness.rs:24` `pub struct RuntimeHarness { persistence: Arc<dyn Persistence>, provider_registry: Arc<dyn ProviderRegistryTrait>, ... }`；`:44 fn new(deps: HarnessDeps)`，注释「所有依赖必须由调用方注入，不再硬编码具体实现」。
**真注册表**：`providers/registry.rs:19 adapters: HashMap<String, Arc<dyn ProviderAdapter>>`；`tools/registry.rs:140` 同构。
**编译期发现（对方完全没有）**：`inventory::submit!`（`axagent-agent-macro/src/lib.rs:63`）+ `inventory::collect!(CommandMetadata)`。

---

## 4. ★ 核心洞察：双方同病，只是护栏粒度不同

|                               | Semantica               | AxInvest                      |
| ----------------------------- | ----------------------- | ----------------------------- |
| 模块级（包内 / crate 内）护栏 | **无**                  | **无**                        |
| 跨模块护栏                    | 无 → 长出 3 个环 + 星型 | **有（cargo 强制）** → 0 环   |
| 后果                          | 全仓失控                | **失控只发生在 crate 边界内** |

Semantica 是「**没有护栏会长成什么样**」的活体标本。而我方**在 crate 边界内已经出现了同型前兆**，只是因为 cargo 挡住了跨 crate 的那一层，没有被暴露：

| 我方同型病                       | 证据                                                                                                                                                                                                                                                      | 对应 Semantica 的哪一条                      |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| `commands` 绕过业务层**直连 DB** | **97 个** commands 文件 `use sea_orm::`/`axagent_entities::`；`stock_workflow/core.rs:19-23` 直接 INSERT `stock_analyses`                                                                                                                                 | 同 `context → graph_store` 直连具体实现      |
| **命令之间横向互调**             | `cognitive.rs:1090` 调 `agent::agent_query`、`:1736 workflow_execute`、`:1790 plan_generate`                                                                                                                                                              | 同 `semantic_extract ↔ split` 环             |
| **域内延迟导入规避层次**         | `src/commands` 函数体内 `use` **504 处**（生产相关 dao/runtime/harness **67 处**），如 `evaluator.rs:188`、`gateway.rs:1069`                                                                                                                              | 同对方 `# Deferred to avoid circular import` |
| 契约层膨胀混入实现               | `harness`：`pub trait` **231** vs `pub struct` **1067**；`cognitive_router.rs:1008 impl CognitiveRouter for DefaultCognitiveRouter`（具体算法）、`service_registry.rs:698 pub static SERVICE_REGISTRY: OnceLock<>`（全局单例）、`test_support.rs` 2039 行 | 同对方 `core/`「名为核心、实为实现」         |
| 前端分层反向依赖                 | `domain/conversationStore.ts:21-25` import `../feature/{agent,category,plan,trajectory}Store` —— **共 10 条 domain→feature**；`shared/`、`devtools/` 干净                                                                                                 | 同对方 `context ↔ graph_store` 双向          |
| 同名 store 双定义                | `shared/workspaceStore.ts:30` 与 `feature/workspaceStore.ts:64` 均导出 `useWorkspaceStore`，`stores/index.ts:8` 被迫改名                                                                                                                                  | 同对方 `utils` 泛化命名                      |

**⚠️ 关键**：我方的分层纪律目前**只写在 `AGENTS.md`（规则文件）**，而 `eslint.config.js` 无 `no-restricted-imports`/boundaries，28 个 `scripts/` 无一条查依赖方向，CI 无对应门禁 ⇒ **这是铁律 11「规则文件 ≠ 检查脚本 ≠ CI 真跑 ≠ 代码真遵守」的教科书实例。四链只成立了第一链。**

---

## 5. ⚠️ 本轮新增的我方缺陷（分层视角）

### N4（新）`axagent-crdt` 是死 crate：1111 行 / 6 文件，全项目零消费方

| 检查项                                      | 结果                                          |
| ------------------------------------------- | --------------------------------------------- |
| Rust 侧 `use axagent_crdt`（排除自身）      | **0 命中**                                    |
| `Cargo.toml` 中声明 `axagent-crdt` 的消费方 | **仅自身**（`crates/crdt/Cargo.toml:2`）      |
| CI / scripts / 配置引用                     | **0**                                         |
| 正对照（证明检法有效）                      | `axagent-kit` 被 **11 个 Cargo.toml** 依赖 ✅ |

内容物：`gcounter.rs`(137) / `lww_register.rs`(156) / `or_set.rs`(160) / `rga.rs`(422) / `version_vector.rs`(151) —— **CRDT 六种数据结构全实现了，但没有任何人用**。用户记忆里「多 Agent 协作（Swarm + 辩论 + CRDT）」的 CRDT 部分，在代码层**是孤岛**。
→ 对应铁律 24（「无引用」≠「可删」）：这里已做传递闭包（源码 + Cargo + CI + scripts 四路），**可以判定为死 crate**，但删/接需用户裁决。

### N5（新）`axagent-kit` 是「杂物抽屉层」：37 个模块 / 9438 行，被 11 个 crate 依赖

内含跨领域混杂：`billing`、`marketplace(_service)`、`sandbox_runner`、`screen_capture`、`screen_vision`、`ui_automation`、`computer_control`、`secure_store`、`token_counter`、`markdown_parser`、`html_cleaner`、`schema_validator`、`workflow_version`、`memory_forgetting`…
且带**重依赖**：`reqwest` / `scraper` / `image` / `enigo` / `keyring` / `parking_lot`。
⇒ **任何 crate 只想用一个 `token_counter` 或 `path` 工具，都要拖入整个杂物抽屉**（编译时间 + 依赖面）。

> 对照：Semantica 的 `utils`（扇入 25）也是同型，但它只有 11 文件 / 5180 行。**我方的抽屉更大更重。**

### N6（污染性）`harness` 非纯契约不只是洁癖问题，而是**编译扇出问题**

`harness` 扇入 **36**（43 个 crate 里的 36 个）。它内部混入具体实现（`cognitive_router.rs:1008` 的默认路由算法、`util_fns.rs` 241 行工具函数、`test_support.rs` 2039 行 mock）⇒ **改一个实现细节会触发 36 个 crate 重编**。
这直接对冲了 `AGENTS.md` 里「sccache 缓存 50G 避免全量重编」的优化意图。

---

## 6. 可借鉴项（借纪律，不借依赖）

| 编号   | 内容                                              | 来源（正面 or 反面）                                                                        | 治什么                                                                                                                | 优先级 |
| ------ | ------------------------------------------------- | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- | ------ |
| **L1** | **模块级分层护栏（CI 脚本 + eslint boundaries）** | **反面**（对方零护栏 → 3 环 + 星型）                                                        | N1-N6 全部；铁律 11 的第 3/4 链                                                                                       | **P0** |
| **L2** | **供应商适配器惰性注册**（门面 + 按需 import）    | 正面（`graph_store/graph_store.py:525` 门面，分支内才 `from .neo4j_store import`）          | `providers/registry.rs:70 register_all_builtins()` 硬编码 13 个 adapter **即时 `Arc::new` 全量实例化** → Tauri 冷启动 | P1     |
| **L3** | **版本存储抽象 + InMemory 实现**                  | 正面（`change_management/version_storage.py:101 VersionStorage(ABC)` + `:166/:267` 双实现） | `TEMPLATE_VERSION` / `kit/src/workflow_version.rs` 无存储抽象 ⇒ 单测无法替换后端                                      | P2     |
| **L4** | **节点内算法级覆写钩子**（MethodRegistry 模式）   | 正面（`core/registry.py:46`；各 store 业务函数先查 `method_registry.get(task, name)`）      | `rt-workflow` 节点只能整只替换，不能覆写单步算法                                                                      | P3     |

### L1 落地方案（具体到规则，可直接写脚本）

新增 `scripts/check-layer-discipline.mjs`（挂进 `npm run contracts` + CI），规则：

| # | 规则                                                                                      | 现状违规量                    |
| - | ----------------------------------------------------------------------------------------- | ----------------------------- |
| 1 | `src/commands/**` 不得 `use sea_orm::` / `use axagent_entities::`（须经 dao/service）     | **97 文件**                   |
| 2 | `src/commands/*.rs` 不得跨模块调 `crate::commands::X::`                                   | 3 处已知                      |
| 3 | `crates/harness/src/**` 不得出现 `use axagent_*`（锁死扇出=0，防未来破坏）                | 0（**当前合规，用脚本锁住**） |
| 4 | 每个 `crates/*` 必须有 ≥1 workspace 内消费方                                              | **1（crdt）**                 |
| 5 | `src/stores/domain/**` 不得 import `../feature/**`（eslint `no-restricted-imports` 分区） | **10 条**                     |

> 注：规则 1 的 97 处是**存量**，直接开成 error 会红一片。建议按「新代码拦截 + 存量 allowlist 只减不增」推进 —— 与项目现有 `check-hardcoded-i18n.sh` 的 allowlist 模式一致。

---

## 7. 不确定项（**未验证，不要当结论**）

1. 97 个 `use dao/sea_orm` 的 commands 文件中，未逐一区分「真持久化调用」与「仅引用类型」，实际越界量可能略低于 97。
2. 未做前端 80 个 `feature/` store 的**全量**内部依赖图，横向耦合总数（15 条）基于相对路径 import 统计，可能偏低。
3. `storage` 与 `dao` 对 `stored_file` 的「元数据 / 字节」拆分（`dao/src/stored_file_repository.rs:9 StoredFile` vs `storage/src/file_store.rs:14 SavedFile`）是否也存在于其它实体，未穷举。
4. `RuntimeHarness` 内部 `harness.rs:115` 直接 `ManagedToolAdapter::with_prefix(...)` 的**影响面**未评估（是否只是默认值装配、有无覆写通道）。
5. 对方 `_AuthManagerBase`（`graph_store/amazon_neptune.py:122`）在 `NEO4J_AVAILABLE=True` 时绑定的 `AuthManager` 是否为真 ABC，未读其定义。
