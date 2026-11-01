# PLAN · 主壳 + 按能力插件（= `PLAN-everything-is-plugin.md` 的 P5 阶段）

> **定位**：本文件**不另起架构**。组合模型（目标 A）与载入模型（目标 B）的判据、`PLAN-everything-is-plugin.md` §5.4 接缝 op 映射表、
> 握手/降级/环检测语义、分发链（local / npm / git + sha256 + `source_audit` + `source_build`）
> **全部沿用** `PLAN-everything-is-plugin.md` 与 `PLAN-plugin-gap-closure.md`。
> 本文件只解决它们没覆盖的那件事：**让「能力」第一次成为可独立编译、独立交付、可拔可换的产品。**
>
> 前四个阶段建的是**机制**（注册表 + 帧协议 + 远程门面），不是**分解**（把实现搬出主壳）。
> 今天机制已可用，而 43 个 workspace 成员全部静态链接在主壳里 ⇒ 用户看到的仍是单体。

---

## 0. 口径（2026-11-01 拍定，三条都是硬约束）

| # | 口径 | 对方案的直接后果 |
|---|---|---|
| 1 | **交付形态 = 已编译产物**（子进程 worker 可执行），插拔**不涉及动态编译** | `PLAN-everything-is-plugin.md` §12「运行期编译管线（生成→审计→编译→载入）」**整节移出本方案范围** —— 它是作者链路，不是交付链路 |
| 2 | **数据主权在宿主**：宿主暴露查询接缝 | 插件侧**禁止** `DatabaseConnection` / `Entity` / `ActiveModel` 出境；`dao` / `entities` / `storage` / `migration` / `crypto` / `credential` 留在壳内核，不做插件 |
| 3 | **粒度 = 按能力** | 插件边界 = **一条接缝族**（一组 provider、一组行情源、一组通知渠道），不是一个 crate、也不是一个业务页面 |

对齐性自检：口径 1 与既有 §1.1 的主路径（独立二进制 + 长驻 worker + 长度前缀 JSON 帧）**同形**，
故不需要新协议；口径 2 与既有排除项 `session.store`「数据主权在宿主」（`worker.rs:100-108` 注释、
`PLAN-plugin-gap-closure.md:38/47/91`）**同向**，本方案把它从「一条排除」升格为「设计约束」。

> **这三条口径只规定「怎么插拔」，不规定「终局长什么样」。** 终局形态见 **§0.5**，
> 三个形态级产品决定**已于 2026-11-01 拍定**（§0.5.8 / §0.6）。
> 阶段表 §6 已按定案重排；**唯一还悬着的形态问题是 §0.6.3 RAG 向量的切法**。

---

## 0.5 终局形态（一次定死；后续改动必须先改这一节，不许在阶段表里偷偷改形态）

### 0.5.1 产品形态（不是「一个安装包」—— 那是我把过渡态写错了）

**主壳发一个安装包；插件各自独立分发。** 主壳的安装包里**不打包**插件产物。
用户装了主壳之后，插件是**单独获得**的（下载 / 安装 / 启用 / 关闭 / 卸载），每个插件有自己的版本号与自己的发布节奏，
与主壳发版**解耦**。拔插的单位是 worker 可执行 + `plugin.json`，**不编译、不下载源码、不需要 Rust 工具链**。
设置页里的插件列表是**三态可见**的：清单资产可用 / worker 可用 / worker 启动失败（失败必须有可见状态，不许只写日志）。

### 0.5.1b 插件化的意义判据（任一条成立才值得做，全不成立就该退回模块化单体）

1. **作者解耦**：某个功能可以由不碰主仓的人/团队写和发版，主仓不审查它的源码。
2. **发布解耦**：第 15 家 LLM 上线不需要等主壳发版，也不需要重编主壳。
3. **可替换 / 可关**：换一个行情源、关掉一个通知渠道，动的不是主壳代码路径。
4. **体积与许可边界**：不用的能力不进用户机器；AGPL 边界以产物为单位而非以目录为单位。

> 这四条**没有一条**能靠「同一个仓库里的不同目录」成立 —— 同仓意味着一起构建、一起发版、一起被 CI 门禁，
> 那正是 §0.5.2 之前写错的地方。**所以插件独立仓是终局，不是可选项；同仓不同根只是脚手架期的权宜。**

### 0.5.2 仓库与构建图的终局

| 结构 | 位置 | 构建图 | 依赖面 |
|---|---|---|---|
| **主壳** | 主仓 | 1 张图：src-tauri（应用 + commands + 装配 + mobile）+ **全部 19 个内核 crate**（entities · dao · storage · migration · crypto · credential · cache · telemetry · rt-messaging · runtime · runtime-core · gateway · agent · orchestrator · kit · crdt · rt-theme · plugins 载入器 · schema-gen） | — |
| **SDK** | 主仓（发布物） | 与主壳同图；**对外发布为 crate**（harness + plugin-proto + plugin-testkit） | 插件唯一的编译期依赖 |
| **插件** | **各自独立仓库** | 每仓一张自己的图，自己的 `Cargo.lock`，自己的 CI，自己的发版 | **只**依赖已发布的 SDK + 第三方 crate |
| **L1 功能包**（股票 / OPC / 工作流编辑器…） | **各自独立仓库**，且**前端产物也在包内**（打包 JS + 清单声明页面） | 同上；额外要一条**前端产物装载链**（§0.10.2 T3） | 同上；Rust 侧**不许**再依赖主壳的 `commands` 或域 crate |

⚠ 原表把「插件」默认等同 L2 接缝插件，**漏了 L1 功能包的仓库位置** —— 而「除了主壳外其它都是插件」这条拍板，
没有 L1 这一行就只覆盖了接缝层（§0.10）。

⇒ 主仓里**不存放插件源码**。今天被劈成 **7 个子根**的那批 crate 全部收回主壳一张图（文件不搬，只撤 `[workspace]`），
因为它们是内核，不是能力。

**「脚手架期同仓」的退出判据（没有这条，过渡就会变成永久）**：满足三条即必须把某个插件挪出主仓 ——
① 它的依赖面已收敛到「只有 harness」；② SDK 已有可发布的版本号与 changelog；
③ 它的 CI 能在**不含主仓源码**的目录里独立跑绿。反过来说，**在 ①②③ 达成前，任何「同仓插件根」都只是临时工位**。

### 0.5.3 一个「能力」的终局定义（可机械检验，不再凭感觉切）

> ⚠ **本节原定义只覆盖了一类单元，2026-11-01 由 §0.10 逼出修正**。「一条接缝族」判得住 LLM provider，
判不住股票包（304 条命令 + 57 个工具 + 293 个前端文件 + 20 张表）。终局有**两类可插单元**，判据不同：

| 单元 | 边界 | 贡献面 | 判据 |
|---|---|---|---|
| **L2 提供方接缝插件** | **一条接缝族 + 它的 DTO + 它的 worker 产物** | 一类能力的第 N 个实现（provider / 行情源 / 通知渠道） | 本节原三条 |
| **L1 功能包插件** | **一个用户可见业务域**（股票分析、OPC、工作流编辑器） | **五面**：工具集 / 命令 / 页面 / 表 / 文案（§0.10.2 T1–T5） | 主壳对它**零静态引用**（`grep` 域 crate = 0）+ 页面入口由清单生成 + 卸载后其余功能不红 |

口径 3「粒度 = 按能力」约束的是 L2（一个插件不许只包半条接缝）；L1 的粒度是**业务域**，
它不是把口径 3 改松，而是多了一层 —— **两类的终局判据都必须能被机器检验**，否则又退回「分了目录的模块」。

终局判据三条，全中才算这个能力真的插件化了：

1. 宿主侧**不再出现**该能力的具体类型：没有 `Arc<AStockClient>` 这类字段、没有 `match strategy_type` 这类硬构造、
   没有 `init/**` 里对该能力实现 crate 的直接 `new`。
2. **新增同族第 N+1 个成员不改主壳代码**：第 15 家 LLM、第 7 个行情源、第 8 个策略 —— 只需插件根产物 + manifest 声明。
3. 卸载该插件后，宿主在**下一次调用**时明确失败（`SEAM_PROVIDER_UNAVAILABLE` / `SEAM_CALL_TIMEOUT`），
   不挂起、不静默降级、不让 UI 卡死。

按这三条判，终局的**能力全集**以 **§0.7 的登记表**为准（**登记 26 条** = A 11 + B 10 + C 5，逐条带行号证据；其中 **24 条有实证**，`context.engine` / `rule.custom` 经核为零外部实现者 ⇒ 降级待证，权威读数见 §0.9.4）。
本节原先列的那份按名字点出来的 12 条（LLM provider · 行情数据源 · 量化策略 · 市场仿真 · 通知渠道 · 文档解析 ·
RAG 向量后端 · MCP 服务器 · 工具集 · 平台适配器 · 反思/进化/优化器 · 业务规则）是**手工挑选、未经清点，已作废**；
其中「市场仿真」按 K2/K3 落进 §0.7.7 的**内核**桶（DES 每 tick 调用的 agent 变体，出进程即每步一次 IPC）——
这一条是我判的，不是拍板拍掉的，若要拉回能力面必须显式改 §0.7.7。
**内核不是能力**：会话存储、事件总线、数据访问、凭证、迁移 —— 它们永远是宿主。

### 0.5.4 数据与凭证的终局

插件**从不**持有 `DatabaseConnection` / `Entity` / `ActiveModel`，**从不**自己读凭据文件。
读写一律经宿主暴露的 `host.query`（命名查询 + 结构化过滤，不是 SQL；写侧白名单逐 op 开，表外即拒）；
凭证由宿主代持，插件按声明的 scope 借用，用完即收。
⇒ 终局形态里，`init/database.rs:397 / :401 / :300`、`state.rs:79` 那四处「宿主把连接反向灌进实现层全局」**不存在**。

### 0.5.5 版本与兼容的终局

帧协议**严格相等**握手（`plugin-proto` 的 `AXAGENT_PLUGIN_PROTO_VERSION`）⇒ 任何协议改动都要**重发全部 worker**，
这是常态成本而不是意外。终局要求：产物侧有版本矩阵记录（哪个壳版本 ↔ 哪些插件版本），
安装时展示来源 + sha256；**签名是否要**取决于 §0.5.8 的决定 (b)。

### 0.5.6 终局明确「不是」什么（防止再次被读成别的形态）

- **不是微服务**：没有分布式部署、没有网络 RPC、没有服务发现；worker 是本机子进程，帧是 stdin/stdout。
- **不是 WASM / cdylib 运行时**：口径 1 已定 worker 可执行。
- **不是运行期编译平台**：`PLAN-everything-is-plugin.md` §12 那套生成→审计→编译管线整节出范围。
- **不是前端插件市场**：前端通用贡献点是另案（`PLAN-plugin-gap-closure.md:26`）。
- **不是「一切皆插件」**：内核 19 个 crate 永久留在主壳，这是口径 2 的直接推论。

### 0.5.7 独立仓这个终局**强制**带来的六件事（不做完就不是插件，是分了目录的模块）

1. **SDK 必须可发布**：`harness` + `plugin-proto` + `plugin-testkit` 要有版本号、changelog、可被仓外解析
   （crates.io 公开 / 私有 registry / vendored 目录源三选一）。**这是终局的硬前置**，不是「生态需求可推迟」。
2. **harness 的 `pub` 面变成公开 API**：今天它是内部 trait 层，可以随手改；发布后 15 条接缝的签名、DTO 字段
   （camelCase 契约）、错误码常量都是**对外承诺**，要有废弃策略。⇒ **换来独立发版，付出内部重构自由**，
   这是这轮取舍的真实价格，必须认。
3. **协议改动的放大器**：帧协议严格相等握手（`plugin-proto` 的 `AXAGENT_PLUGIN_PROTO_VERSION`）在单仓里是
   「重编几个 bin」，在独立仓里是「跨 N 个仓库协调重发 + 用户侧升级」。⇒ 需要**版本矩阵**（哪个壳版本 ↔
   兼容哪些协议版本 ↔ 哪些插件版本），否则每次 D2/D4 那类改动都变成一次发布事故。
4. **契约测试必须在两侧都跑**：主壳跑「接缝表 ↔ 声明 ↔ op 路由」四方对账（§7 门 1），
   插件仓跑「我对 SDK 的实现符合契约快照」。中间靠 `plugin-testkit` 发布的夹具连接。单侧绿不算绿。
5. **分发通道与完整性**：产物从哪来（自有 CDN / GitHub Release / npm 包内二进制的现有 local-npm 通道）、
   sha256 钉死（`plugins/src/types.rs:880-887` 已有）。**若允许第三方投稿，签名从「登记」升级为「硬要求」**（D6）。
6. **本地开发体验**：插件仓作者要能一步调试 ⇒ SDK 侧提供 path override 模板（`[patch]` / 本地目录源），
   以及一个**参考插件仓**当模板（含 CI 配置、profile、三份工具配置）。没有模板，每个插件仓都会各自长歪。

### 0.5.8 三个产品决定 —— **已于 2026-11-01 拍定，见 §0.6**

- **(a) 能力覆盖范围 → 「全部能力面覆盖」**。拍板当时的措辞是「12 类全覆盖」，但 **12 是我手工挑的族，不是扫出来的**；
  机器核准后的范围见 **§0.7 登记表（登记 26 条 / 有实证 24 条）**，本节只锁「不许有豁免」这一意志。原「策略与 RAG 代价高于收益，可只覆盖前 8 类」的建议**未被采纳**，
  因此 §9 里对应的两条豁免作废，P5.7 / P5.8 变成必做项（RAG 的**切法**仍待单独拍，§0.6.3）。
- **(b) 谁能写插件 → 「第三方」**。签名、兼容窗口、权限模型、审核与分发通道全部升级为硬前置；
  并且它顶穿了「握手严格相等」与「清单无权限字段」两处现存设计 —— 逐条见 **§0.6.1 / §0.6.2**。
- **(c) provider → 「进」**。D2 流式协议改动成为必做，且必须与 (b) 的兼容窗口**同一次定版**（P5.0a）。

> 这三条定完之后，剩下的唯一形态级未决项是 **§0.6.3 RAG 向量的切法**（其余都是工序）。

---

## 0.6 三条拍板（2026-11-01 **定案**）与由此顶穿的现存设计

| 决定 | 拍板 | 立刻变成终局要件的东西 |
|---|---|---|
| (a) 能力覆盖 | **全覆盖（范围 = §0.7 登记的 26 条、其中有实证 24 条；拍板时写作「12 类」，那是我手工挑的口径，已由机械登记表替代）** | 需新建接缝 **13 条**（§0.7.4 的 10 + §0.7.5 有实证 3；两条待证成立则 15）+ 策略会话式批量 op + RAG 后端接缝 + MCP/工具集投影层 + Dojo 第二协议合流（§0.7.6）。**§9 里任何「某类不做」的豁免作废** |
| (b) 谁能写插件 | **第三方** | 签名（D6）、SDK 兼容窗口、审核与分发通道、权限模型 —— 全部从「登记/可推迟」升级为**硬前置**。§0.5.7 六件事一件不能省 |
| (c) provider | **进** | D2 流式分片协议改动，且因为它打破兼容 ⇒ 必须与 (b) 的兼容窗口**同时**落地，不能先改协议再补窗口 |

### 0.6.1 顶穿之一：握手是**严格相等**，第三方不可能被强制同步升级

`axagent-core/crates/plugin-proto/src/lib.rs:338`：

```rust
if self.proto_version != AXAGENT_PLUGIN_PROTO_VERSION {
    return Err(DeclarationError::VersionMismatch { declared, expected });
}
```

`AXAGENT_PLUGIN_PROTO_VERSION: u32 = 1`（`:61`）⇒ 今天**任何**协议改动都会让所有第三方 worker 当场被拒载。
单仓期这代价是「重编几个 bin」；(b) 之后它变成「用户机器上一批插件集体失效，且作者不在你 CI 里」。

> 可用的落点已经埋好了：每条能力声明带一个 `cap.version: String`，但 `validate()`（`:344-352`）**只校验非空**，
> 从不参与协商。⇒ 兼容窗口不是新造机制，是把已有字段用起来：协议从 `u32 相等` 改成
> **`(min_compat, current)` 区间 + 逐能力版本协商**，并在拒绝时返回可诊断的错误码（前端 11 语言）。
> ⚠ 这一条与 D2 流式、D4 ctx 投影是**同一批协议改动**，必须一起定版，否则连发三次打破兼容。

### 0.6.2 顶穿之二：清单里**没有任何权限字段**，「第三方能碰什么」无处声明、无处执法

`plugin-proto` 全量 grep `permissions` / `scopes` / `allowed_hosts` / `trust` ⇒ **零命中**。
而信任模型今天自己就承认不是边界（`axagent-plugins/crates/plugins/src/source_audit.rs:5-7` 原文）：

> 「**定位：形式合规检查，不是安全边界。** `build.rs` 与 proc-macro 在编译期以宿主用户身份执行任意代码，
> 不在任何运行期沙箱范围内 ⇒ 任何源码级检查都可能被绕过。」
> 任意路径文件读写、子进程调用**不拦截**，只做信任分级标注（`PLAN-everything-is-plugin.md` §8.2「无 OS 级约束」）。

⇒ 在「只有我方写插件」的前提下这可以接受（自己审自己）。**(b)=第三方之后不可接受**：
一个恶意 worker 拿到的进程权限 = 宿主用户权限（读 `~/.axagent` 下的凭证与 DB），
而 D4（`worker.rs:624` / `:1881` 远程工具丢弃宿主 `ToolContext`）已经把**唯一**那道运行期约束绕掉了。

**终局要求（三者缺一不可，顺序有依赖）**：

1. **manifest 声明式权限**：`requires`（文件路径模式 / 出站域名 / 宿主接缝子集 / 凭证 scope），未声明即拒 ——
   沿用注册表既有的「表外即拒，不猜不兜底」语义（`worker.rs:815`）。
2. **D4 必须先修**：宿主侧保留策略/审批/脱敏句柄，只把可序列化白名单投影下发 —— 否则权限声明形同虚设。
3. **产物签名 + 发布者身份**（D6 转正）+ 安装时逐条展示「这个插件申请了哪些权限」，用户显式同意。
   OS 级沙箱（容器 / Job Object / seccomp 一类）**不在本轮范围**，写清楚：权限模型是**声明 + 执法接缝 + 人工同意**，
   不是强隔离 —— 这是这仓能做到的上限，别把它说成沙箱。

### 0.6.3 与口径 2 的真冲突：RAG 要拆成**三件事**，结论各不相同（2026-11-01 实测修订）

> 本节初稿把「RAG」当成一格来判，导致口径含糊。实测后必须分层 —— **能进插件的和不能进插件的是不同的东西**：

| 层 | 是什么 | 能不能进插件 | 实测依据 |
|---|---|---|---|
| **A 算法面** | 嵌入模型 / 重排 / 检索策略 | ✅ **能**，走 §0.7.4 的 `rag.source` 接缝 | `search` 的 axagent-* 依赖只有 `disk-cache entities harness`、`document-parser` 只有 `harness` ⇒ **不认识任何业务域**，方向干净 |
| **B 存储面** | `vec0` 虚拟表 + FTS5 + `vec_collections` 注册表 + chunk 元数据 | ✘ **不能**（口径 2 的正面撞墙处） | `vector_store.rs:160` `VectorStore { db: DatabaseConnection }`；`:173 new(db)` 直接吃宿主连接；sqlite-vec 是**进程内扩展**，唯一注册点 `init/database.rs:300` ⇒ worker 里那份 sqlite 与宿主库**不是同一连接**，`vec0` 虚拟表无法跨进程共享；向量必须与 chunk 同库同事务（删 source 要级联清向量） |
| **C 业务功能面** | 知识库 / wiki / llm_wiki / sources / ingest / index_jobs（**131 条命令 / 11 827 行**，`AUDIT-capability-inventory` §2 第 6 域） | ✅ **能**，是 L1 功能包（§0.10 框架的第 4 个候选） | 非 KB 文件 import `axagent_search`/`axagent_document_parser` 仅 **72 处 / 25 文件**（股票是 152/28），最大头是宿主装配层 `src-tauri/src/indexing.rs` 18 处 + `init/state.rs` 7 处；KB 自有文件内引用 27 处随迁 |

**抽象边界其实已经在代码里，但位置错了**：`agent/src/query_engine.rs:46` 已定义 `pub trait VectorSearch`，`:42` 以 `Option<Arc<dyn VectorSearch>>` 注入 ——
所以 agent（consumer，依赖面确实只有 `axagent-harness`，`Cargo.toml` 无 search）根本不认识具体向量库。
⚠ 但这条契约**定义在 consumer crate 里**，插件 SDK（harness + plugin-proto + testkit）**不含 agent** ⇒
若要让第三方贡献向量后端，**P5.0a 前必须把 `VectorSearch` 下沉到 `harness`**，否则插件要依赖 consumer 层 = 违铁律 2。

**修订我初稿的一处错话**：原写「解 2 ⇒ 要重写内核的存储路径」。**不成立** ——
`vector_store.rs:149-152` 的自述与实现都是**双后端运行期按 `DatabaseConnection` 的 backend 分叉**（SQLite 走 `vec0`+FTS5 / PostgreSQL 走 `VECTOR(n)`+`tsvector` GIN），
且已有 `search/tests/pg_integration.rs` ⇒ **pgvector 路径已经写好了**，缺的不是代码而是「产品是否允许引入外部 PG」这条取舍。

⇒ 两种解仍在，但代价被重新校准：
- **解 1（推荐）**：接缝切在 A 层（嵌入 + 重排），B 层留内核。与口径 2 零冲突；代价是 RAG 那一格语义是「换算法」不是「换数据库」，UI 文案跟着改。
- **解 2**：把 B 层的后端定为**服务端**（pgvector 已可用，Qdrant/Milvus 需新写客户端）。**存储仍在宿主侧的库/服务里，插件只提供客户端** ⇒ 不算「把数据库装进插件」，但给单机桌面产品引入外部服务依赖。
- **B 层「作为插件产物交付」这条路线明确不做**：它同时违反口径 2 与 vec0 的进程内约束，改不动语义就只能是措辞游戏。

⇒ **解 1 / 解 2 仍需你拍一次**（这是产品取舍，不是我能替你选的判据空白）；但 **C 层（知识库业务功能包）不依赖这个决定**，可以独立排进 P6 序列。

---

## 0.7 提供方接缝（L2）登记表（2026-11-01 机械核准，替代 §0.5.3 末尾那份手工 12 条）

> ⚠ **本节不是「能力全集」**：它的候选集要求「有 ≥1 个外部实现者」，因而**结构上照不出策略位层**（路由族 9 个 trait 全在里面，外部实现者皆为 0）。全集口径见 **§0.9.4**。

> 这份表是 (a)「全覆盖」的**范围定义**。在此之前「12 类」是我从有 trait、有实现者、有调用点的族里**挑**的，
> 不是扫出来的 —— 挑出来的数不能当验收口径。本节把口径换成四条可复跑判据，逐条给行号证据，把能力数**定死**。

### 0.7.1 判据（一个能力必须同时过 K1–K4）

| 判据 | 内容 | 怎么检（本轮实际用的机器口径） |
|---|---|---|
| **K1 有契约** | 存在声明提供方契约的 trait | 扫 8 根全部 `.rs` 的 `^pub trait` |
| **K2 有键控分发点** | 主壳在**运行期**按字符串/枚举键选实现，而不是编译期 `new` 死一个 | `HashMap<Key, Arc<dyn T>>` 字段 / `fn register*(key: &str, impl)` / `fn get*(key: &str) -> Arc<dyn T>` / `match k.as_str()` 且块内 ≥3 个构造分支 |
| **K3 第三方可新增** | 新增同族第 N+1 个成员不改主壳代码（§0.5.3 判据 2） | 键集合来自运行期输入（设置表、清单、配置）而非 `'static` 字面量穷举 |
| **K4 帧可表达** | 入参/返回是纯数据；流式/句柄/借用除外，除非先改协议 | trait 方法签名扫描：`&mut self`、借用入参、`impl Stream`/`mpsc`、`DatabaseConnection`/`Arc<Registry>` 句柄 |

**K2 是把「能力」从「trait 数量」里救出来的那一条。** 只按 K1+「有外部实现者」扫出来是 232 个面，
其中 dao 的 20 多个 `*Repository` trait 只是可测试性分层，没有任何键控站点 —— 它们是**管线**，不是能力。

### 0.7.2 派生流水与敏感性（数字怎么来的，换个口径会变成多少）

| 口径 | 命中 | 备注 |
|---|---|---|
| K1：`pub trait` 总数（8 根 2004 个 `.rs`） | **315** | 含引擎内部变化点、测试替身、serde 派生面。**2026-11-01 两档各降 1**：`#30` 删除 `harness/src/dojo_extension.rs` 整文件（−460 行），同时少 1 个 `.rs` 与 1 个 `pub trait DojoExtension`（旧读数只记在 §0.12.4）。权威读数 = `node scripts/check-capability-register.mjs --dump-readings` 的 `k1.rsFiles / k1.pubTraits` |
| K1 + 有非 harness、非测试实现者 | **232** | ⚠ 这一档**不能**当能力数。**2026-11-01 第五道门复算得 269，表内另自记 210 —— 三数并存**：232 出自一次性 census 工件（`.tmp/traits-full.json`，未入库 ⇒ 谓词无法从仓内代码复现）；210 是单趟扫描缺陷（同谓词两趟 = 232，差值恰 22）；269 是本门最严格读法（不要求 trait 声明面、不排 std/派生噪声名）。**处置次序不能反**：先把 232 的谓词入库为 `k1DeclaredExternalImplTraitCount`、`--dump-readings` 验出 232，再把该锚点升 `hard:true`；反向必先假红 |
| K1 + K2（键控站点，四次扫描合并去漏报） | **51** 个 trait 面 | 含 15 条已建接缝的 trait |
| 51 → 同族合并 + K3/K4 判定 | **26 条能力** | ⇒ 本节表格 A+B+C 三组 |
| （对照）今天就能远程跑的接缝 | **13** | `axagent-plugins/crates/plugins/src/worker.rs:110` `SUPPORTED_REMOTE_SEAMS` |

> 报数纪律：以后凡是「能力数」，必须连带给出它用的是哪一档口径。单说「26」「27」「12」而不说口径，三者同样不可核对。

### 0.7.3 A 组：接缝已建，机制可用 —— 11 条

| 能力 ID | 契约（定义行） | 键控分发点（证据行） | 内置实现者 | 拆分阻塞 |
|---|---|---|---|---|
| `model.provider` | `axagent-core/crates/harness/src/provider.rs:25` | `axagent-ai/crates/providers/src/registry.rs:24`（`HashMap<String, Arc<dyn ProviderAdapter>>`）+ `:33 register` + `:44 get` | 14 | **D2** 流式：`chat_stream`/`speech_stream` 首帧即错 |
| `tool.set` | `axagent-core/crates/harness/src/registry.rs:130`（契约面 `harness/src/tool.rs:424 Tool`） | `axagent-runtime/crates/tools/src/registry.rs:183` | Tool impl **199** | **D4** `ToolContext` 被丢弃（安全，必须先修） |
| `platform.adapter` | `axagent-core/crates/harness/src/message_adapter.rs:83` + `channel_adapter.rs:25` | `axagent-runtime/crates/rt-messaging/src/message_gateway/platform_manager.rs:16` + `:104 get_adapter(&str)` | 10 | — |
| `message.callback` | `axagent-core/crates/harness/src/platform_callback.rs:21` | 接缝 `message.callback`（内置与插件同入口） | 2 | — |
| `webhook.dispatch` | `axagent-core/crates/harness/src/webhook_subscription.rs:154` | 接缝 + `axagent-infra/crates/rt-webhook/src/webhook_dispatcher.rs:9` | 3 | — |
| `system.prompt` | `axagent-core/crates/harness/src/context_contributor.rs:22` | 接缝 `system.prompt.{段名}` | 3 | — |
| `workflow.reflector` | `axagent-core/crates/harness/src/workflow_reflection.rs:190` | 接缝 | 3 | **D2**：`axagent-plugins/crates/plugins/src/worker.rs:1527` 起 `remote_method_not_supported` |
| `workflow.evolver` | `axagent-core/crates/harness/src/workflow_evolution.rs:172` | 接缝 | 3 | 同上（`axagent-plugins/crates/plugins/src/worker.rs:1551`–`:1714` 一批 `remote_method_not_supported`） |
| `workflow.optimizer` | `axagent-core/crates/harness/src/workflow_optimization.rs:80` | 接缝 | 2 | 同上 |
| `workflow.sandbox` | `axagent-core/crates/harness/src/workflow_evolution.rs:263` | 接缝 | 4 | — |
| `workflow.business_rule` | `axagent-core/crates/harness/src/business_rules.rs:71` | 接缝（已有 e2e worker 产物） | 3 | — |

### 0.7.4 B 组：K1+K2 成立但**没有接缝** —— 10 条，P5.0a 必须新建

| 建议能力 ID | 契约 | 键控分发点 | 内置实现者 | K4 代价（帧协议/主权） |
|---|---|---|---|---|
| `notify.channel` | `axagent-core/crates/harness/src/notification_channel.rs:195` | `axagent-runtime/crates/notification/src/dispatcher.rs:114` + `:143 register_channel` + `:149 unregister(&str)` | 6 | **最低**：DTO 全可序列化；去重/冷却留宿主 |
| `market.vendor` | `axagent-quant/crates/astock-data/src/vendors/mod.rs:7 StockVendor` | `astock-data/src/lib.rs:863 register_vendor(&str)` + `:1672` 键控取回 | 16 | `AS_OF` task-local 必须改显式入参；门面 `MarketDataProvider`（`harness/src/market_data.rs:192`）不单列 |
| `search.source` | `axagent-ai/crates/agent/src/search_provider.rs:95` | `axagent-ai/crates/agent/src/search_orchestrator.rs:28`（`HashMap<SourceType, Arc<dyn SearchProvider>>`） | 4 | `rate_limit()`/计费属宿主；`extract_content(url)` 返回大文本 ⇒ 需分块 |
| `image.gen` | `axagent-ai/crates/providers/src/image_gen.rs:64` | `name()` 自述键 + providers 注册面 | 2 | 字节流属 ②，要走产物落盘或分块 |
| `document.parser` | `axagent-core/crates/harness/src/search_sources.rs:62` | `axagent-infra/crates/document-parser/src/lib.rs:533`（`match ext` 37 个分支） | 1 | 37 分支是**编译期穷举** ⇒ K3 今天要改主壳，正是本条要拆的理由；另有两套 `OnceLock` 全局（`search/src/sources.rs:22`、`tools/src/parser.rs:13`）和一处硬编码绕过（`commands/knowledge.rs:1169`） |
| `rag.source` | `axagent-core/crates/harness/src/rag_provider.rs:7/40`（Embedding/Rerank）+ `axagent-infra/crates/search/src/rag.rs:45 RAGSource` | `search/src/rag.rs:941`（3 分支工厂） | 4 | **§0.6.3 未拍**；`VectorStore` 是持 `DatabaseConnection` 的 struct（`search/src/vector_store.rs:160`），sqlite-vec 进程内扩展 ⇒ 出进程即失效 |
| `mcp.server` | `axagent-core/crates/harness/src/mcp_service.rs:54 McpClientService` + `:48 McpServerStore` | `axagent-ai/crates/mcp/src/client_service_impl.rs:155 build_*` | 1 | 长连接 + 子进程句柄 ⇒ K4 属「会话式 op」，与 strategy 同批 |
| `strategy.quant` | `axagent-core/crates/harness/src/strategy_contract.rs:440` | `src-tauri/src/commands/quant_backtest.rs:363-430`（`match strategy_type` 硬构造） | 7 | **最高**：`&mut self` + `&mut StrategyCtx` 的 bar 级热循环 ⇒ 会话式批量 op（spawn/setParam/onBarBatch/drop） |
| `workflow.node` | `axagent-runtime/crates/rt-workflow/src/work_engine/node_executor_trait.rs:224` | `work_engine/dispatcher.rs:31`（`HashMap<&'static str, Arc<dyn NodeExecutorTrait>>`）+ `:397 get_executor(&str)` | 33 | 键是 `'static` 字面量 ⇒ K3 今天不成立；**粒度必须是「节点包」而非单节点**，否则每个节点一次 IPC |
| `memory.provider` | `axagent-ai/crates/trajectory/src/memory_provider.rs:63` | `provider_name()/provider_version()` 自述键 | 2 | `sync_turn(entries)` 可序列化；预取语义要与会话生命周期对齐 |

### 0.7.5 C 组：钩子/会话族，判可拆但先验 K4 —— 5 条（其中 2 条证据不足，见 §0.9.4）

| 能力 ID | 契约 | 证据 | 判定理由 / 待验 |
|---|---|---|---|
| `lifecycle.hook` | `axagent-core/crates/harness/src/workflow_lifecycle.rs:33` | 4 实现者；文档自述「业务侧实现 + 注册，WorkEngine 按模板声明调用」 | 天然第三方面；`HookExecContext` 是否可序列化待验 |
| `plugin.hook` | `axagent-core/crates/harness/src/plugin_hook.rs:159` | 14 方法（`pre_tool_call`/`post_tool_call`/`on_session_*`） | **与 D4 同域**：`ToolCallContext` 走白名单投影后才可拆；否则等于把守卫钩子外置成哑铃 |
| `context.engine` | `axagent-core/crates/harness/src/context_engine.rs:24` | `compact()/name()/supports(&Session)` | 压缩策略可拆（LLM 摘要 / 记忆 / 混合）；`supports(&Session)` 是借用入参 ⇒ 需投影成 DTO |
| `rule.custom` | `axagent-ai/crates/trajectory/src/constitution.rs:102` | `:118 HashMap<String, Arc<dyn CustomRuleChecker>>` | 自我改进宪法的自定义校验；与 `workflow.business_rule` 不同域，不并条 |
| `transport.kind` | `axagent-runtime/crates/rt-messaging/src/message_gateway.rs:168` | `:164 HashMap<TransportType, Arc<dyn TransportHandler>>`，4 实现者 | ⚠ **可能不过 K4**：`connect(endpoint)` 后持有长连接状态 ⇒ 除非改成会话式 op，否则判留内核 |

### 0.7.6 机制合流，不是能力 —— 1 条（必须处理，但不计入 26）

**已执行（2026-11-01，`#30`）**：`harness/src/dojo_extension.rs` 整文件删除（**−460 行**），`harness/src/lib.rs:332-337` 换成 4 行墓碑注释（337 是 `pub use` 收尾 `};`，按 332–336 删必编译不过）。删除门禁：8 个 `DojoExtension*` 符号在两文件之外命中 0（`.tmp/t30-dojo-gate.mjs`，20441 文件面；排除 `.tmp/`、`output/`、`docs/`、`.workbuddy/`）；翻案检索 `git log --all -S"impl DojoExtension for"` = 4 提交，`git grep` 到 `origin/master`/`upstream/master` 的 18 处命中全落在被删文件自身 ⇒ **历史与其它分支均无生产实现者，翻案条件未触发**。
它原来「是一套与 capability 接缝并行的第二插件协议」这一判据不变：自带键控注册表、自己声明工具与命令、自己带健康探针，却不在 15 条接缝里，也不在 `SUPPORTED_REMOTE_SEAMS`。
⇒ **已裁定并执行（§0.12.1）：废弃 trait 本体，把 `command_specs/execute_command` 的形状吸收进 L1 命令通道 T2**（生产实现者 0，唯一实现是测试替身）——不给零实现者的第二协议上「权限模型 + 对账 + 兼容窗口」三套成本。
   原记「CLI 派发口无 `input_schema` 且权限写死 `ReadOnly`」**判据低估了实情**：`PluginTool::check_permission`（`plugins/src/types.rs:645-684`）全仓零调用方，唯一引用它的 `execute_with_permission_check` 也零调用方 ⇒ 档位在 tools 与 commands **两条路上都是纯装饰**；本批只把命令通道改成真授权（按具体权限名判定，`implies_*` 一字未改），**tool 侧依旧是装饰**，另立批次。
   同批已修的假话：`manager.rs` 原注释自述「按空白切分为程序 + 参数」是假的（命令串整团交 `Command::new`、`args` 恒空，可执行文件查找按宿主 cwd ⇒ **相对路径声明的 command 今天根本跑不通**，见任务 #47）。CLI 派发口现位于 **`plugins/src/manager.rs:583`**（原 `:551`，因 +544 行改动而下移；镜像表同批改）。

### 0.7.7 判为内核，明确不拆（每条都给不过哪道判据）

| 面 | 不过的判据 | 理由 |
|---|---|---|
| `agent.loop`（`AgentTurnRunner`，已建接缝） | 语义 | 引擎本体。拆掉它就没有主壳，与 §0.5.6「主壳不是插件」直接冲突 ⇒ **接缝留着，实现永远内置** |
| `session.store` / `event.dispatch` | 既定口径 | §9 已定：提供面不远程化（插件是订阅方） |
| `session.log.invariant`（有接缝且可远程） | 语义 | 「模型所见可重建」是宿主对插件的**审计约束**；让被审计者自己出证不成立 ⇒ 留内核 |
| DES 仿真（`market-sim` `SimAgent`/`Oracle`，**27 个实现**，全在 crate 内） | K4 + 语义 | 实现者是同一内核内的 agent 变体；`kernel.rs:703` 每 tick 遍历全部 agents ⇒ 出进程即 tick × agent 次 IPC。**已裁定维持内核**，可插部分下放到「策略数据」层（场景 / 参数 / 脚本）—— 见 §0.12.2 |
| `command.handler`（`src-tauri/src/commands/agent/command_bridge.rs:1125`，15 实现者） | K3 | 主壳 IPC 路由机制本身；命令注册已由 `build.rs` 自动生成（禁区 7） |
| `harness.interceptor` / `reversible.effect` | K4 + 语义 | 拦截链与回滚栈依赖执行上下文（`&mut`、闭包、`EffectHandle`）；D4 修完才有讨论资格 |
| 数据访问层（`harness/src/repositories.rs` 20+ 个 `*Repository` trait）、`migration`、`storage`、`credential`、`crypto` | K2 | 只有 DI 站点、无键控选择；且凭证/加密是数据主权落点（口径 2） |
| `prompt.template`（`harness/src/prompt_provider.rs:29`）、`health.check`、`agent.def`（`harness/src/agent.rs:61`）、`vault.source` | K3 / 语义 | 模板取词与内置 Agent 定义属数据驱动（已可经配置/Markdown 贡献，不是产物插件）；健康探针应作为**插件清单自带**的自检面并入 §0.6.2 权限面，而不是独立能力 |
| `SeamInvoker`、`PluginManager`、`worker.rs` 帧协议本体 | 语义 | 插件机制自身 = 装载器，不能装载自己 |

### 0.7.8 这版登记表推翻 / 锁死了什么

- **锁死（后被 §0.9.4 修正）**：能力数曾锁为 **26**（A 11 + B 10 + C 5）。C 组里 `context.engine` / `rule.custom` 经核为**零外部实现者**（我引错了证据等级）⇒ 有实证的提供方接缝为 **24**，权威口径见 §0.9.4。
- **本轮剔除 `domain.pack`（域包 / OPC 能力包）**：用户口径为「域包已从产品移除」。**但工作树与口径不一致** —— 盘上仍有 14 个顶层 `opc_*` / `capability_pack_*` 命令文件共 **9887 行**（最大 `opc_capability_pack_actions.rs` 3486 行）、`analysis-engine/src/opc/` **26 个模块**、`company-runtime` **2064 行**、`harness/src/capability_pack_orchestration/mod.rs:38` 的 `CapabilityPackAdapter` 仍被注册。⇒ 它现在不是插件，是**一条待迁出的 L1 功能包**（P6.2 / §0.10.5：原「删除」口径已作废，OPC 与股票同属插件化刚需）。
- **推翻**：§0.5.3 末尾那句按名字列举的 12 条是手工挑选，已作废（正文保留划线注，防止又有人引用它）。
- **新增工作量**：需新建接缝 **13 条**（B 10 + C 3 有实证，另有 2 条待证 ⇒ 上限 15），按 §3 反推的 6 处固定成本 ⇒ **约 78–90 处改动点**；
  另加 §0.7.6 的 Dojo 协议合流一项。**§3 的 DTO 分级只覆盖了 7 条能力，15 条待建接缝（两条待证成立的上限；主口径 13 条）里还有 10 条未做 K4 分级**（§0.7.4 余 5 条 + §0.7.5 钩子族 5 条）⇒ P5.0a 的输入补齐项（任务 #29）。
- **登记但不实现**：第五道门 `scripts/check-capability-register.mjs`（复跑本节扫描、与本表逐行对账，能力数变化必须显式改表）—— **已裁定要建（§0.12.4 / 任务 #38）**。
  同批登记一条**预留接缝位** `rag.vector.store`（§0.12.3 的 3 个方言无关 trait 就是它的 op 面）：本轮**只占位不实现**，不进 P5 批次、不计入 13/15 的待建数。
  接线代价 = 两处同源注册 + 一次 `--selftest`；**这条要点头才做**，不擅自开工。

---

## 0.8 功能包层（L1）：工作流编辑器、股票分析今天为什么拆不干净（2026-11-01 端到端实读）

### 0.8.1 先立层级，并撤回我造出来的第二个数

本方案里「能力」只有**一个**可插拔单元定义（§0.5.3：一条接缝族 + DTO + worker 产物）。为避免再翻烧饼，层级钉死如下：

| 层 | 是什么 | 判据 | 与插件的关系 |
|---|---|---|---|
| **L0 内核** | 装载器、帧协议、会话/DB/迁移、凭证、权限模型 | 拆了就没有「宿主」 | ✖ 永不拆 |
| **L1 功能包** | 一个用户可见入口 + 它的页面/store/命令面/crate（工作流编辑器、股票分析在这一层） | 能不能整体关掉而产品不崩 | ⚠ **今天拆不干净**：六项前置未做（§0.8.3 + §0.8.5），不是原理上不可拆 |
| **L2 提供方接缝** | 同一功能的多实现可替换槽 | K1–K4 ⇒ §0.7 的 **26 条** | ✔ 唯一已成立的插件单元 |

**撤回声明**：我曾在本轮给出「L1 20 个功能包 + L2 26 = 46 个可拆单元」。**46 作废**——它把两层并列计数，等于凭空造第二种插件单元，与 §0.5.3 冲突。
L1 的正确身份是**接缝的消费者**，不是「27 里漏掉的一格」。本会话三次改数（12 → 27 → 46）的根因是同一个：
**我拿「我先扫到什么」当「架构是什么」**。此后本 PLAN 内任何计数必须绑死口径档位（§0.7.2 的敏感性表）。

### 0.8.2 实读对象：工作流编辑器（132 个前端文件 / 83 条命令（审计 19 域口径）/ 引擎 7366 行）

| 面 | 事实（全部指到行号） |
|---|---|
| 前端 | `src/pages/WorkflowPage.tsx:23`、`WorkflowMarketplace.tsx:186`；`src/components/workflow/` 132 文件（`WorkflowEditor.tsx` 2660 行、`Nodes/` 39 个）；store `workflowStore.ts` 1202 / `workflowEditorStore.ts` 3127 / `workEngineStore.ts` 542 |
| 命令 | 前端 57 个 `invoke` 名（约 128 处）；Rust 侧 `register_commands.rs` 共 **1542 条 / 164 模块**（口径 = `scripts/lib/command-domains.mjs` 的 `parseRegistry`，与两份 AUDIT 同源），工作流域（审计 19 域口径）**83 条 / 11 模块**（`workflows`=10、`work_engine`=21、`workflow_template`=16、`workflow_reflection`=8、`workflow_ai`=8…；`stock_workflow` 28 条归股票域，不计入本域） |
| 引擎 | `axagent-runtime/crates/rt-workflow/src/work_engine/engine/mod.rs` 7366 行；构造点 `src-tauri/src/init/state.rs:745`；句柄在 **`app_state.rs:345` `pub work_engine: Arc<WorkEngine>`** |
| 节点 | `NodeExecutorTrait`（`node_executor_trait.rs:224`）实现者 **33** 个，`rt-workflow/src/work_engine/dispatcher.rs:80-109` `init_builtin` 注册 26 个 + `engine/mod.rs:1214-1231` pending 补齐 |

### 0.8.3 四个卡点（按拆除难度由高到低；第 2 项已在 §0.8.5 改判为「成本」而非「不可行」）

1. **数据模型是编译期封闭 enum，穿透全域（最难，且不是「引用了一个 crate」）**
 `axagent-core/crates/harness/src/workflow_types.rs:1363-1366` `pub enum WorkflowNode`（`#[serde(tag = "type")]`，**32** 个 variant；旧记 33，2026-11-01 实点 `:1366-1418` 订正）
 + **`axagent-runtime/crates/rt-workflow/src/work_engine/node_executor_trait.rs:233`** 的 `node_type_name` 穷举 match（⚠ 该文件在 **rt-workflow**，不在 harness）+ `harness/src/workflow_node_deserializer.rs` 类型字符串表。
 ⇒ 种子数据、OPC、进化、认知路由全域都**按这个 enum 构造 DAG**（例 `stock_analysis_setup/seed_stock_analysis.rs:1450`）。
 **新增一个节点类型要改 8 处**（enum / match / deserializer / executor 新文件 / `rt-workflow/src/work_engine/dispatcher.rs:80-109` 注册 / `engine/schema_validator.rs:38-43` / 前端 `WorkflowEditor.tsx:110-146` nodeTypes 表 / 前端 PropertyPanels+i18n），
 而 `capability_registry.rs` 的 15 条接缝与 `worker.rs:110` 的 13 条远程接缝**都不含节点执行器**。⇒ L2 机制对节点类型零覆盖。
2. **WorkEngine 被 15+ 个域直接持有 —— 这是「拆掉的迁移成本」，不是「拆不动的理由」**（上一版把它当成否决判据，该因果作废，真因见 §0.8.5）。
 持有者清单：`cognitive.rs:1134/1717`（三层路由主 DAG 交给它跑）、`evolution_engine.rs:58-69/943`（实现 harness `WorkflowDagExecutor`）、
 `stock_pipeline/core.rs:20/222`、`pool_scan.rs:428`、`mover_recall_analysis.rs:388`、`time_travel_batch.rs:574`、
 `init/services.rs:2011-2018/2244`（cron）、`src-tauri/src/lib.rs:570`、`:728`（启动路径）、`opc_capability_pack_actions.rs:2315`。
 ⇒ 「把工作流卸掉」会让股票分析、自我进化、认知路由、定时任务**同时失去执行层** —— 这是**成本**，由 P6 的前置消化，不构成「永远不可拆」。
 ⚠ **依赖方向已核实为「域 → 引擎」的客户端关系**：`cognitive.rs:1249` 调 `state.work_engine.run_workflow(COGNITIVE_ROUTER_MAIN_ID, opts)`、
 `stock_workflow/core.rs:2035` 调 `state.work_engine.cancel_workflow(&workflow_id)`；而 `axagent-runtime/crates/rt-workflow/Cargo.toml`
 的 axagent-* 依赖只有 `crypto dao harness kit providers telemetry` —— **不含任何业务域 crate**。引擎**不认识任何域**，正如「页面都用了 tokio runtime」不代表页面拆不走。
 ⚠ **关键连带事实：股票分析的主管线本身就是 workflow** —— `src-tauri/src/lib.rs:83` `pub use commands::stock_workflow::core::run_stock_workflow_inner;`。
 所以 L1 功能包**不是 20 个并列的盒子，是一张有底座依赖的图**；我上一轮把它列成并列清单，那也是一处错。

3. **命令面是编译期生成的，插件带不进命令**（⚠ 严重度已由 §0.10.2 T2 下调：运行期通道在仓内**已存在**，缺的是登记与对账，不是新机制）
 `src-tauri/src/register_commands.rs:1` 自述「Auto-generated by build.rs — DO NOT EDIT」，1542 条全部静态来自 `commands/mod.rs`。
 ⇒ 功能包出仓时它的 83 条工作流命令（股票包 304 条、OPC 包 138 条，见 §0.10.1）无处注册。唯一的既有绕过是 §0.7.6 的 `DojoExtension::command_specs()/execute_command()`
 （`axagent-core/crates/harness/src/dojo_extension.rs:215`），它其实已经在做「贡献命令 + 贡献工具 + 自带健康探针」，但没登记、没权限模型、没进接缝体系。
4. **前端贡献面只有仪表盘一处**
 `plugins/src/types.rs` 有 `dashboard_panels` / `PluginDashboardPanel`，而 `route` / `menu` / `contribution` / `widget` **零命中**。
 页面入口本身是硬编码：`src/components/layout/WorkspaceHub.tsx:166-173` 的 `switch` + `src/lib/workspaceTabs.tsx:28`（union 类型）`:70`（常量表）。
 ⚠ 仓里**已有** DynamicPageViewer / `dynamicUIStore` 动态页机制，工作流没接进去 ⇒ 缺的不是能力，是把 tab 表改成数据驱动这一步。

### 0.8.4 由此定下的三件事

- **L1 插件化的前置是 6 项，不是「再补几条接缝」**：① `WorkflowNode` 封闭 enum 开放化（换成「类型字符串 + 契约声明」的注册模型）；② `WorkEngine` 的角色定性（底座 or 功能包，二者不可兼得；§0.8.3-2 已证明它今天当底座用，但方向是「域 → 引擎」，故可换轨）；
 ③ 命令贡献通道（要么升格 DojoExtension，要么给 `build.rs` 注册面加运行期入口）；④ 前端贡献点（tab/route/menu 数据驱动，复用已有 DynamicPage 机制）。
 **这 6 项（①–④ 见下，⑤⑥ 由 §0.8.5 新增）全部不在 P5 的任何阶段里** ⇒ 新增 **P6（功能包层插件化）**，排在 P5.11 之后，不与 L2 的 26 条混排。
- **`domain.pack` 已从 §0.7 剔除**（27 → 26）。它是**待迁出的 L1 功能包（P6.2）而非已定删除**：盘上仍有 14 个 `opc_*` / `capability_pack_*` 命令文件 9887 行（最大 `opc_capability_pack_actions.rs` 3486 行）、
 `analysis-engine/src/opc/` 26 个模块、`company-runtime` 2064 行、`harness/src/capability_pack_orchestration/mod.rs:38` 及 `state.rs` 的接缝注册仍在。
 扫描基线是**当前混合作业树**（`git status` 显示 `stock_analysis.rs`、`recommender/mod.rs`、`stock_workflow/serenity.rs` 等为其它会话未提交改动），
 所以任何计数在收口删除后要**重跑一遍 §0.7 的扫描**才成立。
- **`workflow.node` 保留在 §0.7.4 的 B 组**，但它的 K3 失败原因（键是 `&'static str` 穷举）与 §0.8.3-1 是同一件事 ⇒ **它必须先由 P6-① 解锁，不属于 P5.0a 能建的接缝**。


### 0.8.5 修正：股票分析拆不干净的真实原因（三条，都不是「基于工作流」）

依赖方向核实完毕后，重新定位病灶。**这三条都是可改的码问题，不是形态问题**：

1. **引擎里嵌了股票域知识（最硬的一条）** —— `rt-workflow/src/work_engine/executors/agent_executor.rs`（7129 行）生产区有 **32 行**股票/行情词：
 - `:4385` `const PREFETCHABLE_TOOLS: &[&str] = &[ "get_stock_quote", "get_stock_kline", … "get_stock_concept_blocks" ]`（十余个股票工具名硬编码在通用引擎里）；
 - `:1232-1246` 从执行上下文取 `stock_code` 并调 `prefetch_tool_results(context, tool_list, &stock_code, kline_limit)`；
 - `:854` 内置 prompt 文本直接写死 `search_stock / get_stock_financials`。
 ⇒ 「股票分析拆不走」的真实原因是**通用引擎认识股票**，不是股票用了引擎。
2. **节点执行器与接缝是双轨，且引擎那一轨是空壳** —— `executors/document_parser_executor.rs`（59 行）**不解析任何东西**：
 只从上下文变量取内容、回显 `"status": "parsed"` 与 `content_preview`，既不调 `DocumentParser` trait 也不取接缝；
 而 `executors/notification_executor.rs`（165 行）自己用 reqwest 打 slack / wecom / dingtalk / feishu 四种 webhook，
 与 L2 的 `notify.channel`（6 个实现：gotify / ntfy / pushplus / serverchan / …）**平台集合不重叠** ⇒ 同一能力两套平行实现。
 ⇒ 不收敛双轨就插件化，会得到「DAG 里点通知」与「设置页点通知」走不同代码、且只有一边可被第三方替换。
3. **原 §0.8.3-1 与 -3/-4 仍成立**：`WorkflowNode` 封闭 enum（新增节点要改 8 处）、命令注册编译期生成、前端贡献面只有 `dashboard_panels`。

**由此改判**：股票分析、路由编排、OPC **都能成为 L1 插件**，判据不再与「是否基于工作流」挂钩。
`P6.0` 前置从四条扩成**六条**：新增 ⑤ **引擎去域化**（`PREFETCHABLE_TOOLS` / `stock_code` / prompt 文本一律改为节点配置或接缝贡献）、
⑥ **节点执行器与接缝单轨化**（executor 只能是接缝的薄壳，禁止自带实现；`document_parser_executor` 空壳必须接上真接缝）。

---


## 0.9 策略位层（L3）：路由编排在这里 —— 也是我前几版计数的构造性盲区（2026-11-01 实读）

### 0.9.1 为什么路由从没进过表

§0.7 的候选集定义是「有 ≥1 个**非 harness、非测试**实现者」。路由族的实现者**全部为 0 个外部实现者** ⇒ 在筛选第一步就被整族丢弃，不是我数漏。

同批被丢弃的共有 **79 个 harness trait**（harness 共 233 个 `pub trait`，占 34%）。这不是长尾，是一整类可插单元：**策略位**（有契约、有主壳持有、有运行期决策，但只有一个内置实现）。

### 0.9.2 路由族的解剖（9 个 trait，逐条行号）

| trait | 契约 | 唯一实现 | 外部实现者 |
|---|---|---|---|
| `CognitiveRouter` | `harness/src/cognitive_router.rs:489`（文件 1948 行） | `DefaultCognitiveRouter` `cognitive_router.rs:1008` | **0** |
| `DomainRouter`（L1 域） | `harness/src/domain_router.rs:536`（1018 行） | `domain_router.rs:869` + `cognitive_router.rs:1486` | **0** |
| `ClusterRouter`（L2 簇） | `harness/src/cluster_router.rs:225`（446 行） | `cluster_router.rs:319` + `cognitive_router.rs:1506` | **0** |
| `RarRouter`（L3 能力） | `harness/src/rar_router.rs:327`（1342 行） | `rar_router.rs:498` + `cognitive_router.rs:1561` | **0** |
| `CapabilityRouter` | `harness/src/capability_router.rs:134` | `capability_router.rs:216` | **0** |
| `RouteEngine` | `harness/src/route_engine.rs:194`（282 行） | `NoopRouteEngine` `:247`（**名字自述为 noop**） | **0** |
| `ModelTierResolver` | `harness/src/route_bridge.rs:29` | `route_bridge.rs:148`（`:277` 那份是测试 MockResolver） | **0** |
| `RouterSelfUpdateManager` | `harness/src/dual_registry.rs:192` | 无实现 | **0** |
| `WorkflowGraphRouter` | `harness/src/workflow_graph.rs` | 无实现 | **0** |

79 个零实现者 trait 里，**被主壳以 `Arc<dyn T>` 全局持有的恰好是路由四个**：`CognitiveRouter` / `DomainRouter` / `ClusterRouter` / `RarRouter`
（`src-tauri/src/app_state.rs:436` + `src-tauri/src/init/state.rs:1456`）。⇒ 它们不是废弃接口，是**运行期中枢**。

### 0.9.3 路由今天已经「可插」，但插的不是编译产物

`src-tauri/src/init/cognitive_router_init.rs:11-14` 的自述与 `:31-37` 的常量说明：三层路由是 **`workflow_templates` 表里的 preset 工作流**
（`cognitive_router_main` ~20 节点 → `SubWorkflowNode` → `cognitive_l1_domain_router` / `l2_cluster_router` / `l3_capability_router`，标签 `cognitive_router` + `is_preset=true`），
执行交给 `WorkEngine`（`src-tauri/src/commands/cognitive.rs:1134`）。

⇒ 本项目实际有**三种**可插单元，不是两种：

| 单元 | 装的是什么 | 受口径 1（「插拔的都是已编译好的产品」）约束 | 今天的状态 |
|---|---|---|---|
| 提供方接缝（§0.7） | worker 可执行 + DTO | ✔ 受 | 24 条有实证 |
| 功能包（§0.8） | 页面 + 命令 + store + crate | ✔ 受，但六个前置未做 | **可拆**，前置未备（见 §0.8.5 改判） |
| **策略数据（本节）** | `workflow_templates` 行、tier 映射、规则表 | ✘ **不受**（它是内容，不是代码） | **已经在用，只是从没被这么称呼** |

### 0.9.4 我的三条判定（写成结论，不再挂「待你拍」）

1. **路由族判 L0 内核，不做 worker 插件。** 两条实证理由 + 一条作废（原第三条是引擎依赖，被我误当成否决判据）：① 0 外部实现者 + 主壳全局持有 ⇒ 它是「决定哪个插件被使用」的东西，放进可替换接缝就等于允许插件操纵全量流量，与 §0.6.2 权限模型正面冲突；
   ② 每轮对话热路径，出进程即每轮一次 IPC。③ ~~执行依赖进程内 `WorkEngine`~~ **作废（§0.8.5：依赖方向是「域 → 引擎」，引擎不认识域）** ⇒ 本判定只剩两条实证，但结论不变：路由是「决定哪个插件被使用」的**裁决位**，裁决位归主壳，与它是否耦合引擎无关。
   其中 `RouteEngine`（唯一实现名 `NoopRouteEngine`）、`RouterSelfUpdateManager`、`WorkflowGraphRouter`（后两个**无任何实现**）是**空接口**，属债不属能力。
2. **§0.7 的 C 组有 2 条证据不足，降级待证**：`context.engine`（`harness/src/context_engine.rs:24`）与 `rule.custom`（`trajectory/src/constitution.rs:102`）
   外部实现者 = **0** —— 我当初引的是「trait 自带 `name()` 方法」名单，不是实现者证据，那是证据等级不够。⇒ **有实证的提供方接缝 = 24 条（A 11 + B 10 + C 3）**，待建接缝 13 条（若两条待证成立则 15）。
   同批的 `DojoExtension` 也是 0 外部实现者 ⇒ 它「没人实现」不是因为不重要，是因为它是**第二协议**（§0.7.6）。
3. **候选集定义从此改为两层**，本 PLAN 的权威计数以本节为准：
   **提供方接缝 24（+2 待证）· 策略位 79 个 trait 待逐条判（路由族 9 个已判内核）· 功能包 20 个但需 P6.0 前置**。
   任何单层数字再被当成「能力全集」报出，都是错的——我此前 12 / 27 / 46 三次翻烧饼的共同根因就是这条。

### 0.9.5 功能点全集已一次扫全并独立成文（此后不再逐点补）

逐条清点落在 **`AUDIT-capability-inventory-2026-11-01.md`**，由 `node scripts/audit-capability-inventory.mjs` 直接生成（改判据即改脚本，文档可重生成，不手抄）。
骨架 = build.rs 生成的 Tauri 命令注册表，与前端 invoke 消费面、`commands/**` 代码行、后端 `axagent_*` 引用、全仓 `pub trait` 实现者分布、壳层 `Arc<dyn T>` 持有面**五路交叉**。闭合读数：

| 面 | 闭合读数 |
|---|---|
| Tauri 命令 / 命令模块 | **1542 条 / 164 个模块**，100% 归入功能域，无「其它」残桶 |
| 功能域 | **19 个**：4 个判 **L0 公共底座**（智能体引擎与编排 15 · 对话与消息 11 · 设置/安全/外壳 10 · 工作流引擎与编辑器 10；括号为跨面共享消费方数），15 个判 L1 功能包 |
| `pub trait` | **316**：L2 已建接缝 **14** · L2 待建接缝 **19** · **有外部实现者但无键控分发点 198** · L3 策略位 **56**（壳层全局持有 4 + 仅内置实现 52） · **全仓零实现的空接口 29** |

⇒ 「项目里已有功能点非常多」现在有数：**19 域 / 164 模块 / 1542 命令 / 315 trait**（命令数 2026-11-01 增 1 = 新增 `plugin_resolve_page_asset`；trait 数减 1 = 已删的 `DojoExtension`；两个旧读数按门禁规矩只写在 §0.12.4 的漂移列举段，不在这里复述；口径同 §0.7.2）。
⚠ **上表「L0 公共底座」是按跨面共享消费方数得出的，它只度量拆分的顺序成本，不度量可行性**（`AUDIT-capability-inventory-2026-11-01.md` §5 第 0 条）：
 「工作流引擎与编辑器」判 L0 ≠ 股票分析拆不成插件 —— 依赖方向是「域 → 引擎」，真因与改判见 §0.8.5。
而真正符合可插拔判据的只有 **14 + 19 = 33** 条接缝面；**198 个 trait 是「多实现者但编译期选死」的分层解耦**，29 个是零实现的空接口债。
**这个差值（33 对 231）就是本方案的真实工作量与真实边界**，不是再往表上补几行。
两处待办：① 198 个分层解耦 trait 要不要升级为接缝 ⇒ 必须逐条按 K2 检验（不能整批提），② 29 个空接口属清理项，不进能力清单。

---

## 0.10 股票与 OPC **必须**成为 L1 插件 —— 阻塞面实测与五条贡献通道（2026-11-01 逐文件扫描）

> 口径修正：**「六项前置待做」不是交付**。股票与 OPC 是插件化的刚需面，这两块拆不成插件，本方案就没有价值。
> 所以本节把它们当**必须落地的目标**：逐条给阻塞、解法、可打回的验收判据。
> 数字唯一来源：**`node scripts/audit-l1-blockers.mjs` → `AUDIT-l1-blockers-2026-11-01.md`**（156 行，逐文件逐行号可查）；改判据改脚本，不手抄。

### 0.10.1 体量与三类引用桶（只决定工量与顺序，不决定可行性）

| 度量 | 股票业务 | OPC 业务 |
|---|---|---|
| Tauri 命令条数 / 命令模块数（**插件边界口径**） | **304 条 / 20 模块**（`stock_analysis` 172、`stock_workflow` 24、`paper` 20） | **138 条 / 13 模块**（`opc` 57、`opc_capability_pack_actions` 20、`_runtime` 14） |
| — 同一注册表在**审计 19 域口径**下的读数 | 306 条 / 19 模块（域「股票分析与管线」） | 117 条 / 10 模块（域「OPC 公司运行时」） |
| **① 主壳反向阻塞**（主壳非域文件引用域 crate ⇒ **必须逐条改写**） | **152 处 / 28 文件** | **0 处** |
| **② 跨包口**（一个功能包的文件调另一个功能包的 crate ⇒ 走 `call_seam`） | **29 处 / 9 文件**，且**全部是 OPC 的文件在调股票 crate** | 0 处 |
| ③ 域自有文件内引用（随插件整体迁走，**不是障碍**） | 444 处 / 37 文件 | 16 处 / 2 文件 |
| Cargo.toml 构建边 | 8 条（`src-tauri` 4 + `tools` 2 + 域内 2） | 1 条（`src-tauri → axagent-company-runtime`） |
| 前端反向 import（非域 import 域文件） | 54 处 / 39 文件 | 28 处 / 11 文件（15 处集中在 `ContentArea.tsx`） |
| 前端体量（**目录口径**：`components/stock-analysis` / `pages/opc`） | `components/stock-analysis` 211 文件 / 60 757 行 + store 6 文件 / 4 514 行 | `pages/opc` 20 文件 / 8 859 行 + `OpcPage.tsx` 134 行 |
| — 同一注册表在**脚本文件名口径**（`feRe` 匹配 `src/**` 全部 .ts/.tsx）下的读数 | 293 文件 / 84 873 行 | 40 文件 / 12 797 行 |
| entities 表文件 | 20 个 | 28 个 |
| zh-CN i18n key | 3 527（`stockAnalysis`/`invest`/`quant`/`trade` 是**整段**） | 1 897（`opc` 整段） |

① 的 152 处里，前 7 个文件吃掉 **106** 处：`tools/src/tools/opc.rs` **41**、`init/services.rs` **28**、`init/state.rs` **10**、`bin/axagent-validate-backfill.rs` **8**、`commands/evolution_proposal.rs` **7**、`tools/src/tools/finance.rs` **6**、`app_state.rs` **6**（41+28+10+8+7+6+6=106，旧记 103 是加错）；其余 21 个文件各 ≤4 处。
⚠ **为什么同一张表里有两个命令数 / 两个前端体量**：两套**归属规则**不同 —— 「插件边界口径」按迁出后归哪个包，「审计 19 域口径」按模块名正则；
差集 11 个模块逐条列在 `AUDIT-l1-blockers-2026-11-01.md` 判读口径第 6 条（例：`demand_discovery` 审计归股票、边界归 OPC；`mover_recall`/`narrative` 反之）。
**两个数都是真的，引用时必须带口径**；本节与 P6.1/P6.2 的判据一律用**边界口径**。
⚠ **②的 29 处是一条硬顺序约束**：`opc_capability_pack_actions.rs`(10)、`opc.rs`(4)、`opc_capability_pack_runtime.rs`(4)、`opc_workflows/mod.rs`(3)、`opc_workflow_kpi_hook.rs`(3)… 全在调 `astock_data` / `analysis_engine` ⇒ **OPC 依赖股票**。
所以顺序只能是 **先股票、后 OPC**（股票先成为插件后，OPC 侧这 29 处才能改成 `call_seam`）；我上一稿说「OPC 更容易先做」只看了 Rust 桶 ①，**是错的**，撤回。
`tools/src/tools/opc.rs` 的 41 处按桶 ① 记在股票名下（它 import 的是股票 crate），但迁 OPC 时必须连带处理这一文件。

### 0.10.2 一个 L1 功能包要贡献的五类资产 —— 每类都**已有承接机制**，缺口都不是新协议

| 通道 | 插件贡献什么 | 仓内已有的承接点 | 真缺口 |
|---|---|---|---|
| **T1 工具集** | 工具声明 + 执行 | `astock-data/src/mcp_tools.rs:76` `pub fn stock_mcp_tools() -> Vec<serde_json::Value>`（57 个工具，声明是**纯 JSON** ⇒ K4 天然过帧）；`harness/src/registry.rs:130 ToolSetProvider` 是**已建接缝 `tool.set`**；`tools/src/tools/astock_data.rs:12-15` 自述「注册后 `state.rs::register_all_capabilities` 的**通用逻辑自动派生 `tool:{name}` 护照，无需额外接线」 | 只剩 **D4**（`ToolContext` 被丢弃 ⇒ 白名单投影）+ 把静态槽位 `tools/src/global_state.rs:63 GLOBAL_ASTOCK_CLIENT` 从宿主收回给插件私有 |
| **T2 命令** | 命令清单 + 处理 | `harness/src/dojo_extension.rs:215` 已定义 `command_specs()`（`:245`）与 `execute_command(command, args: &Value)`（`:250`）——**形状正好是「运行期贡献命令」** | ⚠ **这一条推翻 §0.8.3-3 的重量级判断**：缺的不是「给 `build.rs` 加运行期入口」，而是这条通道**没登记、没权限面、没与清单对账**。成本从「改构建系统」降为「升格 + 对账」 |
| **T3 页面** | 入口 + UI | `DynamicPageViewer` + `dynamicUIStore` 已在（`ContentArea.tsx:40` 就 lazy 了它）；硬编码点在 `workspaceTabs.tsx:28/70`、`WorkspaceHub.tsx:166-173`、`ContentArea.tsx:40-100` lazy 表 | tab 表改数据驱动 + 清单声明 `pages[]`；**重交互页**（股票 6 万行自绘图表）不套 DynamicPage，走「打包 JS 产物 + 宿主 asset/webview 装载」 |
| **T4 表与迁移** | schema 声明 | `dao/src/reconcile/` **声明式 schema 同步引擎已接启动路径**：`apply.rs:900` 钉死 `additive_only = true`（只加不减，DROP/RENAME 永不在启动期执行），期望侧 `expected.rs:86 build(dialect)`，L2 声明早已有 DTO 形态（`extras.rs:81 IndexDecl` / `:95 CheckDecl` / `:131 VirtualTableDecl` / `:139 TriggerDecl`）；全仓**无 SeaORM Migrator**，建表就是声明驱动 | 给 `expected::build` 加一路**外部声明输入**（插件产物里的 `schema.json`）⇒ 装插件即建表、卸载**不删表**，与数据主权口径天然对齐 |
| **T5 文案** | locale 片段 | i18n 分段组织，股票/OPC 有整段可搬（`stockAnalysis`/`invest`/`quant`/`trade`/`opc`） | 装载时合并插件 locale（11 语言各一份），并入 §7 孤儿 key 门禁 |

⇒ **P6 六项前置里只有 ①（`WorkflowNode` 封闭 enum）是真机制缺口**；②③④ 分别落在 T2/T3/T4 的既有承接点上，⑤⑥ 是码问题。**没有一项需要新发明协议**。

### 0.10.3 反向阻塞逐条与解法（这才是「拆得动」的证据）

| 阻塞 | 实测 | 解法 |
|---|---|---|
| 装配层持有域类型 | `init/services.rs` 28、`init/state.rs` 10、`app_state.rs` 6 | `AppState` 摘掉 `AStockClient` / 分析引擎字段，改由插件装载流程写入 contribution registry（T1 已证通用逻辑能自动派生护照） |
| **工具 crate 反向依赖域**（最硬的一条，也是分层债） | `tools/src/tools/opc.rs` 41、`finance.rs` 6、`audit.rs` 4、`global_state.rs` 3、`astock_data.rs` 3 | 工具清单与执行整体走 `tool.set` 接缝；删 `GLOBAL_ASTOCK_CLIENT` 静态槽位；`audit.rs:167/545` 那种「硬编码清单校验」改为「清单 ↔ describe 对账」（正是 §2 的 D3） |
| 别的功能叶子调域函数 | `backtest_validation.rs` 12、`analyst_feedback.rs` 8、`time_travel_batch.rs` 8、`market_close_sweep.rs` 7、`universe_scan.rs` 6、`conditional_order.rs` 5（**桶③，随迁**） | **它们本来就是股票业务的命令面** ⇒ 按「粒度 = 能力」与 `stock_analysis` 一起进同一个插件，不是障碍 |
| 真跨面调用（非股票域调股票） | `conversations/compress.rs` 4、`fleet/mod.rs` 4、`background_tasks.rs` 3、`price_alert_upsert.rs` 3 | 走 `call_seam` 或事件，并在 §0.6.2 权限面登记 `declared_calls` —— 数量小，是**唯一需要逐条改写语义**的一类 |
| 前端反向 | `pages/TasksCenterPage` 8、`src/lib` 6、`components/chat` 5、`stores` 3、`components/time-travel` 3 | 股票结果卡片/面板改**插件面板贡献**（`dashboard_panels` 已有先例），tab/路由走 T3 |
| 表 | entities 股票 20 / OPC 28 文件 | T4：表声明随插件产物走，宿主只保留 `stock_*` / `opc_*` 命名空间约定 |
| 引擎认识股票 | `agent_executor.rs:4385` / `:1232-1246` / `:854` | §0.8.5-⑤ 去域化 |

### 0.10.4 落地顺序与验收判据

**P6.0 五通道机制**（T1 的 D4 投影 + T2 命令派发口 + T3 tab 数据驱动 + T4 外部 schema 声明 + ⑤⑥ 引擎侧）
→ **P6.1 股票迁出**（改写 152 处 + 前端 54 处、304 条命令、6 万行页面）
→ **P6.2 OPC 迁出**（改写 0 处 Rust + 前端 28 处，但要把 §0.10.1 的 **29 处跨包口改成 `call_seam`**、138 条命令、8 859 行页面）
→ **P6.3** 其余功能包按同一模板批量。
**顺序不能反**：OPC 的 29 处引用指向股票 crate ⇒ 股票先成为插件，OPC 才有接缝可调。

判据（每条都可打回，全部机械可检，口径 = `scripts/audit-l1-blockers.mjs`）：

1. **主壳残留文件里 `grep axagent_(astock_data|analysis_engine|company_runtime|quant|market_sim)::` 命中 0。**
 今天该作用域（`src-tauri/src` + `axagent-runtime/crates`，去注释、去 `examples/`、去 `.workbuddy`）**总命中 625 处 / 74 文件**，
 其中 **444 处随域自有文件（`stock_analysis.rs` 209、`stock_workflow/*` 69、`opc.rs` 18…）一起搬走**、**29 处是跨包口**（改 `call_seam`），
 **真正要改写的只有 152 处 / 28 个主壳文件**，且前 7 个文件吃掉 **106** 处（2026-11-01 订正：旧记 103 是列项加错，见 §0.10.1）。
2. `src-tauri/Cargo.toml` 不再声明 4 个股票 crate + `company-runtime`；`axagent-runtime/crates/tools/Cargo.toml` 两条边归零（今天 8 + 1 条）
3. 主壳 `register_commands.rs` 里股票 / OPC 模块 **0**（今天 304 + 138 条，注册表总 1 542 条 ⇒ 迁完剩 1 100 条）
4. 前端 `src/**` import 股票 / OPC 文件命中 **0**（今天 54 + 28），tab 表由插件清单生成
5. **卸载该插件后**：应用启动不红、其余 1 099 条命令可用、`stock_*` / `opc_*` 表与数据保留（`apply.rs:900` 的 `additive_only = true` 已保证启动期永不 DROP）
6. 新增一个同族业务包（期货 / 基金）不改主壳一行代码 —— 终局判据（§0.5.2）

### 0.10.5 路线已定：**OPC 走迁出（即插件化），「删除」口径作废**（2026-11-01）

用户口径已给：**股票与 OPC 都是插件化刚需** ⇒ `#32` 原先的「OPC 收口**删除** 9 887 + 2 064 行」**作废，改为迁出为 L1 插件**。
**「迁出」不是第三种动作，它就是插件化的执行本身**：把 `commands/opc_*` + `company-runtime` + `pages/opc` 从主壳搬进
`plugin-opc` 独立仓，产出 worker 可执行 + 前端 bundle + `plugin.json`，主壳从此不再静态引用它。
顺序：股票先做（P6.1）、OPC 紧随（P6.2）—— 只因为那 29 处跨包口指向股票 crate；**OPC 的主壳 Rust 反向阻塞是 0 处**，是全套里最干净的样板。
迁出前先用 `grep` 把**前端消费方确实为零**的那批 `opc_*` 命令单独列清单，那部分可以真删；剩下的都是产品功能，走迁出。
⚠ 边界要说清：与 OPC 无关的**域包 / 行业包**（`opc_industries` 一族）仍是「已从产品移除」口径，不参与本表计数（§0.7.8），别混为一谈。



---

## 0.11 终局形态总图：仓库形态 + 应用形态（2026-11-01 定稿，§0.5 族的汇总）

### 0.11.1 仓库面：4 类仓 + 1 份 SDK 发布物

| 仓 | 数量 | 装什么 | 产物 | 构建图 |
|---|---|---|---|---|
| **axagent 主仓** | 1 | 主壳 `src-tauri`（应用 + commands + 装配 + mobile）+ **19 个内核 crate**；今天散在 7 个子根（实现层）的成员**全部收回同一张图**（文件不搬，只撤 `[workspace]`）+ `docs/` | **主壳安装包**（Tauri bundle，**不含任何插件**） | 1 张 |
| **SDK**（发布物，不独立建仓） | 3 crate | `axagent-harness`（15 条接缝 + DTO + 错误码）· `axagent-plugin-proto`（帧协议 + 版本协商）· `axagent-plugin-testkit`（契约快照与夹具） | 发布到 **私有 registry**（推荐）。备选 vendored 目录源：零运维但最容易漂出两份 SDK；不推荐公开 crates.io（AGPL 边界 + 内部 API 面） | 与主壳同图 |
| **L2 接缝插件仓** | N₁，**一个能力族一仓** | 例 `plugin-notify-channel`、`plugin-provider-deepseek`、`plugin-market-vendor-tushare` | 各平台 **worker 可执行** + `plugin.json` | 每仓一张图、自己的 `Cargo.lock` 与 CI，**只依赖 SDK + 第三方** |
| **L1 功能包仓** | N₂，**一个用户可见业务域一仓** | 首批 3 个：`plugin-stock`（P6.1）· `plugin-opc`（P6.2）· `plugin-workflow-editor`（P6.3） | **两个产物**：worker 可执行 + 前端 bundle，外加 `plugin.json` | 同上 |
| `plugin-template` | 1 | 参考模板：CI、profile、三份工具配置、独立 `--target-dir`、契约测试 | 模板，不发布 | — |

⇒ 主仓里**不存放任何插件源码**；终局判据之一：主仓 `grep axagent-plugin-` 命中为 0（§6 P5.11）。

### 0.11.2 一个 L1 功能包仓长什么样（以股票为例，逐面对上 §0.10.2 的 T1–T5）

```text
plugin-stock/
├─ Cargo.toml            # 依赖只有 SDK + 随包的 astock-data / analysis-engine + 第三方
├─ src/main.rs           # worker 入口：describe → 五面贡献 → op 路由
├─ src/commands/         # T2：304 条股票命令的实现（不再进主壳 build.rs 的注册表）
├─ src/tools/            # T1：57 个 stock_mcp_tools（清单是 Vec<serde_json::Value>，纯 JSON 过帧）
├─ crates/               # astock-data · analysis-engine · quant · market-sim 随包出仓
├─ web/                  # T3：293 文件 / 84 873 行前端源码 → 打包 bundle
├─ schema/tables.json    # T4：20 张表的声明，宿主按 additive-only 建表
├─ i18n/*.json           # T5：11 语言 / 3 527 key
├─ plugin.json           # 清单：id · 协议兼容窗口 · requires 权限 · pages · commands · tools · seams
└─ dist/                 # {win|mac|linux}/stock-worker(.exe) + web.zip + sha256
```

`plugin-opc` 同形，差别是 138 条命令 / 40 个前端文件 / 28 张表，且它的 `plugin.json` 必须显式声明那 **29 处跨包口**
（调股票插件的接缝，进 `declared_calls`）—— 这也是 P6.1 必须先于 P6.2 的唯一原因。

### 0.11.3 应用形态（用户视角，七步）

1. **装主壳**：一个安装包。装完只有内核面 —— 会话与对话、设置与安全、知识库/RAG 存储、MCP、**工作流引擎**（编辑器与业务包都不在）、插件管理器。
   未安装业务域的入口显示**引导位**，不许报错、不许空白。
2. **获得插件**：设置 → 插件 → 安装，走四条既有通道（本地目录 / npm / git / 下载 URL）。安装时展示 **来源 + sha256 + 逐条权限申请**
   （`requires`：文件路径模式 / 出站域名 / 宿主接缝子集 / 凭证 scope），用户显式同意才落盘。
   **不编译、不下源码、不需要 Rust 工具链**（口径 1）。
3. **启用后**：业务 tab 出现（入口由清单生成，不改主壳代码）；304 条命令挂到派发通道；57 个工具进统一工具注册表并自动派生 `tool:{name}` 护照；
   20 张表被建出来；11 语言文案合并。插件列表是**三态可见**的：清单资产可用 / worker 可用 / worker 启动失败（失败必须有状态，不许只写日志）。
4. **运行期跨包**：插件之间不直连，一律经宿主**星形转发** `call_seam`（四道闸：未声明即拒 / 自调环 / 深度 ≤8 / 表外即拒）。
   ⇒ **D1（宿主调插件无超时）必须在 P6.2 之前修**，否则股票 worker 僵死会让 OPC 整批失联并级联到宿主调用线程（§11 第 4 条）。
5. **停用 / 卸载**：tab 与命令消失；下一次调用**明确失败**（`PLUGIN_NOT_INSTALLED` / `SEAM_PROVIDER_UNAVAILABLE` / `SEAM_CALL_TIMEOUT`），
   不挂起、不静默降级；**表与数据一律保留**（启动路径钉死 `additive_only = true`，永不 DROP / RENAME）。
6. **主壳升级**：握手由「严格相等」改为 `(min_compat, current)` 区间 + 逐能力 `cap.version` 协商；窗口外的 worker **拒载并可见提示「该插件需要 vX」**，不是崩、不是悄悄消失。
7. **发版解耦**：第 15 家 LLM、新行情源、股票包改版，都不进主壳发版节奏；主仓 CI 不需要任何插件源码，插件仓 CI 不需要主仓源码。

### 0.11.4 终局明确**不是**什么（补 §0.5.6，防止被读成别的形态）

- **不是微服务**：没有网络 RPC、没有服务发现，worker 是本机子进程，帧走 stdin/stdout。
- **不是运行期编译平台**：`PLAN-everything-is-plugin.md` §12 那套生成→审计→编译管线整节出范围。
- **不是 OS 级沙箱**：权限模型 = 声明 + 宿主执法 + 人工同意，别把它称作沙箱。
- **不是前端插件市场**：L1 自带打包 JS，但不开放「任意第三方贡献 UI」的生态（另案）。
- **不是把内核也拆掉**：19 个内核 crate 永久在壳 —— 会话存储、事件总线、DAO/迁移、凭证、加密、插件载入器本身。


---

---

## 0.12 五项待拍的裁定（2026-11-01，判据能裁的我自己裁，不把取舍推给用户）

### 0.12.1 DojoExtension → **废弃 trait 本体，把它的契约形状吸收进 L1 命令通道 T2**

**证据**：全仓 `impl DojoExtension` 命中 3 处，其中 2 处是同文件的 `impl DojoExtensionHealth` / `impl DojoExtensionRegistry`，
唯一真实实现是 `harness/src/dojo_extension.rs:414` 的 `TestExt`（**测试替身**）⇒ **生产实现者 0**，`registry.rs` 的注册 API 无生产注册方。
**顺带查清的两条既有通道（此前没分清）**：
1. `commands/plugin.rs:485 plugin_execute_command` → `plugins/src/manager.rs:551 execute_plugin_command` 是 **CLI 型**命名命令分发
 （拿 `entry.command` 起外部进程、`input_schema: Object(default)` 即**没有 schema**、权限**写死 `ReadOnly`**）；
2. 前端已有**受限桥**：`components/plugin/PluginPanelHost.tsx:30` `PANEL_ALLOWED_COMMANDS = ["plugin_ui_action","plugin_execute_command"]`，
 且有测试锁「不在白名单的命令一律拒绝且**不发 IPC**」。

**裁定与做法**：不把零实现者的第二协议"合流"进接缝体系（那等于给它上权限模型、对账与兼容窗口三套成本）；
**废弃 trait**，把 `command_specs / execute_command` 的形状写进 SDK 的 L1 命令贡献面（T2 = 一条静态注册、运行期路由到 worker 的派发口）。
CLI 通道保留给既有"外部 CLI 插件"品类，但必须补 `input_schema` 与权限声明，**去掉写死的 `ReadOnly`**（它今天是假的宽松：声明只读，执行却是任意子进程）。
删除前按禁区 12 用 grep 确认引用归零，并同步收窄前端白名单。
**翻案条件**：若在仓外（第三方仓 / 其它分支）发现真实 `DojoExtension` 实现者 ⇒ 改为升格，不废弃。

### 0.12.2 市场仿真 → **维持内核；可插的部分下放到「策略数据」层**

**证据**：`market-sim/src/agent/*.rs` 共 27 个 `SimAgent` / `Oracle` 实现（Background / EventDriven / Exchange / Institutional / MarketMaker …）全部在 crate 内；
`kernel.rs:703` 是 `for entry in self.agents.values()` ⇒ **每个 tick 遍历全部 agent**。
若 agent 出进程，IPC 次数 = tick 数 × agent 数，比任何接缝预算高几个数量级 ⇒ **K4 不过**（同 §0.7.7 原判定）。

**裁定**：DES 内核与内置 agent 变体留 L0；**真正可插的是场景配置、参数表与策略脚本**（§0.9.3 的第三类「策略数据」，仓内已有 rhai/JSON 载体在用）。
这与"拍板措辞含市场仿真"**不冲突**：市场仿真继续是产品功能，只是它的实现单元层级是「内核 + 策略数据」，不是 worker 插件。
**翻案条件**：出现「每 tick 只调 1 个远端 agent」的真实需求（例如外部撮合源）⇒ 重判为会话式 op。

### 0.12.3 RAG 切法 → **拍解 1**（接缝切算法面，存储面留内核）

**判据链**：B 存储面不能进插件（口径 2 + `vector_store.rs:160/:173` 直接吃宿主 `DatabaseConnection` + sqlite-vec 是进程内扩展、唯一注册点 `init/database.rs:300` + 向量必须与 chunk 同库同事务）；
解 2 要求产品接受外部服务依赖，与桌面单机定位相悖 ⇒ 在**已拍口径**之下，解 1 是唯一自洽解，不是偏好。
**但不锁死后路**：`vector_store.rs:149-152` 的双后端运行期分叉（SQLite `vec0`+FTS5 / PostgreSQL `VECTOR(n)`+`tsvector` GIN）**已经在代码里**，还有 `search/tests/pg_integration.rs`
 ⇒ 将来若要引入服务端形态，解 2 的架构成本≈0，无需为它提前改接缝。
**随附前置**（本轮实测新发现）：抽象契约 `pub trait VectorSearch` 目前定义在 **consumer crate** `agent/src/query_engine.rs:46`，
而插件 SDK 不含 agent ⇒ 第三方够不着它。要贡献向量后端，**必须先把它下沉到 `harness`**（任务 #37）。
⇒ §0.6.3 末尾「需要单独拍一次」这句话现在可以关掉。

**① 预留接口（2026-11-01 追加口径：暂不上服务端，但把口子留好，后续有需求再兑现）**

实测 `VectorStore` 有 **27 个公开方法**（`vector_store.rs:475` → `:1833`），而 agent 侧现有的 `trait VectorSearch`（`agent/src/query_engine.rs:46`）**只有 1 个 `search`** ⇒
**拿它当预留口子是假的** —— 写侧与集合生命周期完全没盖住，将来换后端时调用方照样得改。

| 预留 trait | 覆盖的现有方法 | 职责 |
|---|---|---|
| `VectorCollectionAdmin` | **7 个**：`ensure_collection:475` · `get_collection_dimensions:522` · `prepare_collection_for_indexing:538` · `ensure_collection_hnsw:615` · `delete_collection:1160` · `clear_embeddings:1177` · `set_collection_embedding_model:1371` | 集合与索引生命周期 |
| `VectorWriter` | **9 个**：`upsert_embeddings:664` · `add_single_chunk:811` · `delete_document_embeddings:1027` · `insert_metadata_only_chunk:1055` · `list_all_metadata_rows` 之外全部写入类 · `reinsert_embeddings:1288` · `upsert_document_embeddings:1298` · `delete_chunk:1617` · `update_chunk_content:1677` · `update_chunk_embedding:1704` | 写入与删除 |
| `VectorReader` | **11 个**：`search:916` · `search_with_filter:930` · `list_all_chunks:1229` · `list_document_chunks_raw:1235` · `ensure_fts5_index:1399` · `rebuild_fts_index:1481` · `fts_search:1516` · `list_document_chunks:1833` 等 | 查询与列举 |

⚠ **命名必须去方言**：`ensure_fts5_index` / `ensure_collection_hnsw` 带着 SQLite 专属特性 ⇒ 接口里要改成
 `ensureKeywordIndex` / `ensureVectorIndex`。否则「预留」会把后端绑死在 vec0/FTS5 上，等于没留。
⚠ **K4 预算**：向量是 ② 类大对象（`&[f32]`，1536 维 ≈ 6 KB/条）⇒ DTO 要么分块，要么**只传宿主侧 chunk 引用**；
 后者更省、也更贴口径 2。

⇒ **本轮只做「接口化 + 占接缝位」，不实现任何外部后端**（接缝 id 预留为 `rag.vector.store`，进 §0.7 登记表但不进 P5 批次）。
 纯重构、零行为变更：3 个 trait + `VectorStore` 的 2 个构造点 + 注入改造（`VectorStore` 引用共 70 处，其中 search crate 内约 55 处属同 crate，
 外部只有 `tools/src/capability_indexer_impl.rs:22`、`agent/src/query_engine.rs:42` 等少数）⇒ 估 **30–40 个改动点**。
⇒ **必须与任务 #37（`VectorSearch` 下沉 `harness`）同批做**，否则 SDK 公开面会被定型两次（禁区 12 的重复定义风险）。

**② asset 协议 / CSP**：**已选 A**（开 `assetProtocol`、CSP 加 `asset:`；`'unsafe-eval'` 单独立项）—— 见 §0.12.5。

**③ P6.1/P6.2 排期**：不是待定项，估工、依赖与推荐序列见 **§6 表下方的「P6 估工与排期」**。

### 0.12.4 第五道门 `check-capability-register.mjs` → **建**

**判据**：今天这轮自查就抓出 **15 处数字漂移 + 2 套并行口径**（1534/1541、工作流 124/83、股票 304/306、前端 60 757/84 873），**2026-11-01 本批再记三对漂移（旧/新）：命令 1541/1542、`.rs` 文件 2005/2004、`pub trait` 316/315**——本段是这些旧读数的唯一合法出现处，写在别处由第五道门判红。
全靠人工复扫才发现 —— 没有机器对账，这些数字**一定会再漂**，而它们是 P5/P6 全部判据的分母。
**成本（可复用件已在）**：`scripts/audit-capability-inventory.mjs` + `scripts/audit-l1-blockers.mjs` + 共享规则 `scripts/lib/command-domains.mjs` 都已落地，
第五道门只需「复跑两份 audit → 与 §0.7 登记表逐行 diff → 条数变化未显式改表即红」。
接线按 AGENTS.md：**`scripts/ci-check.mjs` 一个 step + `.github/workflows/ci.yml` 一步，先 `--selftest` 再判定，命令行逐字同源**。估算 1 人日。

### 0.12.5 L1 前端装载形态 → **两条都要，按页面复杂度分流（这不是二选一）**

**实测：既有能力比 §0.10.2 上一版估计的多**
- 动态 UI 通道已成型：`DynamicPageViewer.tsx:29` 的 ready 状态**已经带 `pluginId`**，配 `DynamicUIRenderer` + `dynamicUIStore`（schema / 表单）⇒ 适合**面板与表单类**贡献。
- **插件前端产物装载 + 受限 IPC 桥已经在仓里**：`PluginPanelHost.tsx:6` 自述「面板内容来自插件安装目录内的文本资产 `manifest.dashboardPanels[].frontendEntry`」，`:30` 是命令白名单。
- iframe 隔离渲染有先例（`ArtifactPreview/ChartPreview.tsx:33` 用 `srcdoc`），但 `tauri.conf.json` **没有 `assetProtocol` 配置**，
 CSP 是 `default-src 'self' tauri: ipc: data:` ⇒ 加载插件自带 bundle 需要**开 asset 协议 + CSP 加一条源**。

**裁定**：
1. **业务域主页面**（股票 293 文件 / 84 873 行的自绘图表与多面板交互）→ 走「插件 bundle + 宿主通用容器页」，
 **复用 `PluginPanelHost` 的桥与白名单扩到 page 粒度**，不新造一套 IPC 桥。
2. **表单 / 看板类**贡献（OPC 的 12 797 行里多数是列表与表单）→ 继续走 DynamicPage 的 UISchema 通道。
3. 每个 L1 包在 `plugin.json` 里逐页声明用哪条，**不许一个包混用两条却不声明**。
**代价与风险**：`frontendEntry` 是插件提供的 HTML/JS ⇒ 开 asset 协议与 CSP 属于**安全面改动**，
必须与 §0.6.2 的权限模型**同批评审**（页面产物的 origin、可调命令集、是否允许 eval —— 现 CSP 里有 `'unsafe-eval'`，这条要在评审时一起处理）。
**翻案条件**：若 DynamicPage 的 UISchema 被扩展到能表达股票图表（成本高于本方案），第 1 条可以取消。

**② asset 协议 / CSP 的三选一（带推荐）**

| 选项 | 做法 | 安全面代价 | 工程量 | 我的评价 |
|---|---|---|---|---|
| **A ✅ 已选（2026-11-01）** | 开 `assetProtocol`，只把**插件安装目录**列进允许根；CSP 的 `default-src` 加一条 `asset:`；`'unsafe-eval'` **本批不动**，单独立项收 | 中：新增一个可被插件写入的静态源；但 origin 仍是 asset 协议、不获得宿主 invoke（桥仍走 `PluginPanelHost.tsx:30` 白名单） | 小：`tauri.conf.json` 一条配置 + CSP 一段 + 白名单测试扩到 page | 一次到位，股票 bundle 有唯一现实载体；风险由已有白名单兜住 |
| **B（零安全变更）** | 不开 asset 协议，插件页面继续用现有 `manifest.dashboardPanels[].frontendEntry` **文本资产 + iframe `srcdoc`**（`ArtifactPreview/ChartPreview.tsx:33` 已有同款用法） | **零**：不新增源，CSP 不动 | 中：bundle 必须内联成单文件 HTML | 适合 OPC 的表单类页面；**股票 84 873 行内联不现实**（首屏解析与图表库体积会炸），只能当兜底 |
| **C（最隔离）** | 插件页面开**独立 WebviewWindow**，origin 完全隔离，宿主只发事件 | 低（隔离最强），但**多窗口生命周期/主题同步/i18n 全要新写** | 大：新增一条 UI 体系 | 过度。与"业务包看起来像主壳功能"这个产品目标相反 |

⇒ **裁定（2026-11-01 选定 A）**：开 `assetProtocol` + CSP 加 `asset:` 一条 + 桥扩到 page 粒度（任务 #39，3–5 人日）。
   B 降级为**兜底通道**：OPC 的表单 / 列表类页面可继续走 `frontendEntry` + `srcdoc`，不浪费既有的 `PluginPanelHost.tsx:30` 命令白名单。
   `'unsafe-eval'` 的收口**从本批剥出、单独立项** —— 它是全局 CSP 问题，不该被插件化顺带改动，两件事评审口径不同。

---

## 1. 现在能做到 / 做不到（核对事实，全部指到行号）

### 1.1 已具备（机制层，可直接复用）

| 资产 | 位置 | 为什么关键 |
|---|---|---|
| 15 条接缝 + `provider_as` 平权存储 | `axagent-core/crates/harness/src/capability_registry.rs:602`–`:914` | 内置与外部走同一 `register_*`，无需为插件开第二条路 |
| 13 条接缝已声明可出进程 | `axagent-plugins/crates/plugins/src/worker.rs:110`（`SUPPORTED_REMOTE_SEAMS`） | 试点不必新造协议 |
| 帧协议 v1 + 严格相等握手 | `axagent-core/crates/plugin-proto/src/lib.rs:61`、`:337-353` | 版本不匹配 = 载入失败，不猜不兜底 |
| 双向调用（宿主→插件门面 / 插件→宿主 `call_seam`） | `worker.rs:2371-2450`、`worker.rs:352-391` | 插件之间可组合，不是只出不进 |
| 环检测 + 深度上限 8 + 未声明即拒 | `worker.rs:362-386`、`:1200-1208`、`plugin-proto/src/lib.rs:282` | 防跨进程死锁的三件套已在 |
| **投放已编译可执行文件今天已成立** | `plugins/src/manager.rs:618`（LocalPath 装目录）+ `plugins/src/manager.rs:2407-2427`（`resolve_worker_program`：绝对路径拒、含 `..` 拒、必须是已存在文件） | 口径 1 不需要新机制，只需要产物 |
| 优雅卸载（逆序 `undo` → `shutdown`） | `worker.rs:1125-1141`、`:2326-2333` | 「拔下来不崩」的前半条已成立 |
| provider 接缝**真接上了** | `axagent-ai/crates/providers/src/registry.rs:65`（读接缝）、`:108`（写接缝） | 内置 14 家与外部插件同入口（缺口 G4 对 provider 已闭合） |

### 1.2 不具备（分解层，本方案的工作项）

| 空白 | 证据 | 后果 |
|---|---|---|
| 没有任何能力以「独立可执行产物」存在 | 43 个成员分布：core 7 / kit 1 / ai 6 / infra 12 / quant 5 / plugins 2 / runtime 8 / src-tauri 2 | 「可插拔」今天只在**声明层**成立 |
| 没有可发布的 SDK（插件仓在仓外拿不到依赖） | 全仓**零** registry / 发布配置；`e2e_business_rule_worker.rs`（84 行）以 `version + path` 依赖树内 `axagent-harness`，path 出不了仓 | **终局的硬阻塞**（§0.5.7 第 1 条）：独立仓 ⇒ SDK 必须可被仓外解析 |
| 没有脚手架 / 模板 / 示例插件 | 无 `plugin-template`、无 `examples/`、`crates/plugins/Cargo.toml` 无 `[[bin]]`（靠自动发现） | 每个插件都要重写 describe + 手工解参 + 手工错误码 |
| **15 类能力有键控分发点但无接缝**（§0.7.4 的 10 + §0.7.5 的 5） | 逐条行号见 §0.7；其中 5 条（notify / doc-parser / market / strategy / RAG）的 DTO 分级已在 §3 表内，余 **10 条未做 K4 分级** | 它们无论如何拔不下来；且 10 条没分级 ⇒ **协议版本号现在还不能签** |
| 有一整套**平行的第二插件协议**（`DojoExtension`，`harness/src/dojo_extension.rs:215`） | 自带 11 个方法 + `HashMap<String, Arc<dyn DojoExtension>>` 键控注册表（`:282`），但不在 15 条接缝与 `SUPPORTED_REMOTE_SEAMS` 内 | 不处理它（§0.7.6），「全覆盖」的范围与 §7-1 对账门永远对不平 |
| 主壳依赖面零门禁 | `src-tauri/Cargo.toml:118-156` **38 个 `axagent-*` path 依赖**；`check-layer-discipline.mjs:15-24` 只管 commands 内部互调，`check-contracts.mjs:193-215` 段 D 只约束 agent/gateway/orchestrator/runtime-core | 「主壳只许依赖 SDK」这条规则今天**无处可挂** |
| 装配是 1.25 万行手工 new | `init/services.rs` 5756、`init/state.rs` 3246、`workflow_injections.rs` 1573、`cognitive_router_init.rs` 1198、`database.rs` 809 | 搬走任何能力都要在这些文件里动刀 |
| 宿主把 DB 连接反向灌进实现层全局 | `init/database.rs:397` `axagent_tools::global_state::set_sea_db(...)`、`:401` `axagent_dao::...::register_repositories(&db.conn)`、`:300` `register_sqlite_vec_extension()`、`state.rs:79` `set_knowledge_search_callback` | 与口径 2 直接冲突，是数据主权的主病灶 |

---

## 2. 六个必须先修的既有缺陷（不修则「插拔＝随机崩」）

| ID | 现象 | 证据 | 影响面 | 最小修法 |
|---|---|---|---|---|
| **D1** | 宿主调插件**无调用超时**：对端僵死（进程活着但不回帧）时 `receiver.recv()` **无限阻塞**；只有对端关闭/崩溃才 `Err` | `worker.rs:1085-1091`；`PluginWorkerConfig` 只有 `shutdown_timeout`（`worker.rs:192-218`），**无 per-call 超时、无重启策略** | 任一插件卡住 ⇒ 宿主调用方卡住 ⇒ UI 卡死，且「拔不掉」 | `invoke` 外包 `tokio::time::timeout`，超时返回 `SEAM_CALL_TIMEOUT`（新错误码，需同步 11 语言 `error` 段）；worker 崩溃后的重启策略显式化（不重启也要写明） |
| **D2** | **无流式**：远程 provider 的 `chat_stream` 第一帧即错误 | `worker.rs:2074-2090`（注释自述「帧协议为一请求一响应，无流式分片复用」）；`reflector.aggregate_patterns/get_history`、`evolver.*`、`optimizer.*` 返回 `remote_method_not_supported`（`worker.rs:1527`、`:1551-1714`） | **provider 试点的硬前置**：聊天应用不能失去逐 token 输出 | 协议加流式分片（同一 `request_id` 多帧 + `final` 哨兵），是 `PLAN-everything-is-plugin.md` §15.4 那套扩展的自然延伸；**不做则 provider 不许插件化** |
| **D3** | 两套注册路径且无一致性校验：`manifest.capabilities` 只写 `declarations` 表**不挂 provider**；可调用门面只来自 describe 自述 | `plugins/src/manager.rs:705-713`→`:783-821`；`capability_registry.rs:484-510`；测试名自证 `records_declaration_only_and_rolls_back`（`:1370`）；真正挂门面在 `worker.rs:2262-2279` | 清单声明 ≠ 运行时能力 ⇒ UI 显示「插件提供 X」而运行时拿不到（既有方案缺陷 1 的变体，仍在） | 握手后做 `manifest.capabilities` ↔ `declaration.capabilities` 双向对账：声明未实现 ⇒ 拒绝注册并计入可见失败（不许静默） |
| **D4** | **远程工具丢弃宿主 `ToolContext`** | `worker.rs:624` 构造 `ToolContext::new(".")` 最小上下文；`worker.rs:1881` `RemoteTool::call` 直接忽略 `_ctx` | 插件工具的**宿主权限校验、输出脱敏、审批规则、Guardian、ask-user、回滚栈全部失效**（安全相关，非风格问题） | ctx 走「可序列化白名单投影」出境（策略/开关/IDs），宿主侧保留句柄类对象（`Arc<Notify>`、审批栈）不下发；需复用反向 `call_seam` / `UI_ACTION` 通道做 ask-user 与审批回环 |
| **D5** | worker 启动失败**只 warn**，插件仍显示「已启用」 | `plugins/src/manager.rs:722-730` | 「启用了却一个远程能力都没有」只能靠翻日志 | 启用结果区分三态：A 层资产可用 / B 层 worker 可用 / B 层失败（可见状态 + 前端提示） |
| **D6** | 信任链只有 sha256 完整性，**无签名** | `plugins/src/types.rs:880-887`；`PLAN-plugin-gap-closure.md` 与 gap-closure 调研一致结论「签名机制：未找到」 | 独立交付产物变成供应链目标；`source_audit.rs:5-8` 已自述「形式合规检查，非安全边界」 | 短期：产物来源与 hash 钉死 + 安装时人工确认；长期：manifest 签名（口径待定，不阻塞 P5.2/P5.3） |

**优先级判断**：D4 是安全缺陷（现在就该修，与插件化进度无关）；D1/D5 是「拔插体验」的地基；
D2 只在 provider 试点前才需要；D3 在任何一个插件真正投放前必须修，否则声明与实现会分叉。

---

## 3. 逐能力接缝补齐清单（量化「新增一条 seam 的代价」）

> ⚠ **本表只覆盖 §0.7 的 26 条能力里的 7 条**（`model.provider` / `tool.set` 两条已有接缝，加 notify / doc-parser / market / strategy / RAG 五条待建）。
> **15 条待建新接缝（主口径 13 条，此为两条待证成立时的上限）里还有 10 条没做 K4 分级**：§0.7.4 余下 5 条（`search.source` / `image.gen` / `mcp.server` / `workflow.node` / `memory.provider`）
> + §0.7.5 全部 5 条钩子族。分级没做完就不许签协议版本号 ⇒ 它们是 P5.0a 的**输入缺口**（任务 #29）。

DTO 障碍分级：**①** 纯 serde DTO 可直接过帧；**②** 可序列化但含大对象/字节流；**③** 不可序列化（借用 / `Arc` / 闭包 / DB 类型 / `&mut`）。

| 能力 | 现有 trait | 接缝 | 实现者 | 主要障碍 | 建议接缝 ID | 难度 |
|---|---|---|---|---|---|---|
| 通知渠道 | `harness/src/notification_channel.rs:195` | ✘ 无 | 6（`LogChannel` / ServerChan / PushPlus / Ntfy / Gotify / 宿主包装的 `PlatformNotificationChannel`） | `name()/display_name()` 返回借用 ⇒ 照 `tool.set` 补一个 `list` 发现面 | `notification.channel.{渠道名}` | **最低**（DTO 全 ①，去重/冷却属宿主分发器，渠道自身只读 config） |
| 文档解析 | `search_sources.rs:62` | ✘ 无 | 1（`DefaultDocumentParser`，零字段单元结构） | 按绝对路径读盘 ⇒ worker 须与宿主同文件视图，否则要走字节流分块（PDF/EML 属 ②）；另有两套同名 `OnceLock` 全局（`search/src/sources.rs:22`、`tools/src/parser.rs:13`）和一处硬编码绕过（`commands/knowledge.rs:1169` 直调具体函数） | `document.parser.{格式族}` | 低（唯一无状态、纯进出的能力） |
| LLM provider | `ProviderAdapter` | ✅ `model.provider.{类型}` | 内置 14 家 | **D2 流式**（`chat_stream` 首帧即错）；`speech_stream` 同类 | 已有 | 中（机制全在，卡在协议） |
| 行情数据源 | `market_data.rs:192` | ✘ 无 | 1 生产（`AStockClient`，`astock-data/src/lib.rs:6667`）+ 2 测试替身 | `astock-data/src/as_of.rs:36` 的 `tokio::task_local! AS_OF` 是**隐式上下文**，跨进程必丢 ⇒ 必须改成显式入参 `asOfDate`；客户端自带 Moka L1 / `DiskCache` L2 / health tracker / token 热更新 ⇒ 缓存与凭证主权要选边 | `market.provider.{源名}` | 中高（签名可原样走，状态主权难） |
| 量化策略 | `strategy_contract.rs:440` | ✘ 无 | 7（MA/ MACD/RSI/BOLL/Turtle + `RhaiStrategy` + 仿真替身） | `&mut self` + `&mut StrategyCtx` 的 **bar 级热循环**不适合 request/response；`StrategyCtx` 含 `bar_history`/`trades`/`equity_curve` ⇒ 每 bar 全量传输直接爆帧；消费者用工厂闭包（`param_optimizer.rs:72`，③）；主壳 `commands/quant_backtest.rs:363-430` 用 `match strategy_type` + `name.contains("rsi")` 硬构造 | `strategy.{策略名}` | **最高**（需会话式批量 op：spawn / setParam / onBarBatch / drop + ctx 差分） |
| RAG 向量库 | `rag_provider.rs:21` | ✘ 无 | **生产实现未找到**（仅 `NoopVectorStoreProvider`，`test_support.rs:1079`） | `VectorQueryResult:13` **无 serde**；`&[f32]` 大向量；真实后端 `search/src/vector_store.rs:160 VectorStore{db: DatabaseConnection}` 绑 DB 且以 `Arc<VectorStore>` 直注（`state.rs:72`、`capability_retriever_impl.rs:57`、`knowledge_integration.rs:168`），sqlite-vec 是**进程内扩展** ⇒ 出进程即失效 | `rag.vector_store.{后端名}` | 高（trait 目前是空壳；先补 DTO derive 再谈远程化） |
| 工具集 | `harness/src/registry.rs:29` / `ToolSetProvider:130` | ✅ `tool.set` | 多 | **D4**：`ToolContext:142-197` 大面积 ③（abort/permissions/sanitizer/ask_user/rollback/sandbox/approval/guardian），代码两处注释自述「不可跨进程传输」 | 已有 | 中（协议已通，缺投影层，缺的是安全正确性而非可行性） |

**新增一条接缝的固定成本**（从 `tool.set` / `platform.adapter` 反推，共 6 处，缺一处即 `SEAM_CALL_UNSUPPORTED`）：
`ServiceDefinition::{ctor}`（`capability_registry.rs:48-180`）+ `register_*` / `get_*`（`:599-915`）+
`SUPPORTED_REMOTE_SEAMS` 一行（`worker.rs:110`）+ `register_remote_seam` 一个分支（`:2371`）+
一个 `Remote*` 门面 + worker 侧 op 路由分支（`:396-820` 形态）+ **`PLAN-everything-is-plugin.md` §5.4 表加一行**（表是唯一权威，`worker.rs:815`「表外即拒，不猜不兜底」）。

> 调研明确发现：既有两份文档**没有成文的「新接缝怎么加」流程**，只有实践痕迹。
> 本方案把它升级为门禁（§7 的 `check-plugin-seam-table.mjs`），这是既有规划的空白 Y 之一。

---

## 4. 目标布局

### 4.1 三层独立要先分清（本节有两次修订，原因写在这里防止第三次翻烧饼）

| 层次 | 含义 | 结论 | 依据 |
|---|---|---|---|
| **交付产物独立** | 拔插单位 = worker 可执行 + `plugin.json` | 必须，且今天已成立 | `plugins/src/manager.rs:618` + `:2407-2427` |
| **构建图独立** | 插件是否为独立 workspace 根 | **应该**（第一次修订曾误判为「不需要」，理由见下） | 插件改动不该 relink 主壳；插件作者不该拉整仓 |
| **源码仓库独立** | 同一 git 仓 / 不同仓 | **终局要独立**（§0.5.1b 四条判据没有一条能靠同仓成立）；同仓不同根只是脚手架期临时工位 | §0.5.2 退出判据三条 |

> 修订史：初稿把插件划成「三个新根 + 8 根照旧」；第一次修订误读 D7，把「不新增根」当成了结论；
> 第二次修订（当前）把 D7 的**成因**定位清楚后恢复「插件独立根」。
>
> **D7 不是「分根」的反证，是「分错了对象」的反证。** 那 8 根劈的是互相咬合、且全部喂给 src-tauri 的
> 实现层（dao / entities / storage / migration / tools / runtime / analysis-engine…）—— feature 组合不同、
> 版本各自解析 ⇒ 同一份 target 里并存 5 个 `librmcp-*.rmeta` ⇒ E0460。
> 实测规模：`src-tauri` 根一次 `cargo check -p axagent --all-targets` **19m19s**、编译目标 **709 个**。

**候选插件的仓内依赖面实测（这就是为什么插件根不会重演 D7）**：

```
notification/Cargo.toml:9      → 只有 axagent-harness（version + path）
document-parser/Cargo.toml:9   → 只有 axagent-harness
astock-data/Cargo.toml:10      → 只有 axagent-harness
quant/Cargo.toml:13            → 只有 axagent-harness
market-sim/Cargo.toml:10       → 只有 axagent-harness
providers/Cargo.toml:10-11     → harness + kit（第 2 个依赖必须先解掉或下沉进 harness）
```

一个仓内依赖 ⇒ 该根的锁漂移面比实现层根小一个数量级；功能再多也是**复制模板**，不是每次重造病灶。

### 4.2 目标形状：主壳图收拢 + N 个「只依赖 SDK」的插件根

```
主壳图（1–2 根）      src-tauri（应用 + commands + init 装配 + mobile）
                      └─ 被劈开的实现层根收回：kit / ai / infra / quant / plugins / runtime 恢复为成员
SDK                  harness + plugin-proto + plugin-testkit（脚手架 / 契约快照 / 锁定测试夹具）
插件根（N 个）        axagent-plugin-notify · -doc-parser · -market-data · -provider-* …
                      每个根 = 若干只依赖 SDK 的 crate + 一个 [[bin]] worker 产物 + plugin.json
内核（留在主壳，不做插件）：entities · dao · storage · migration · crypto · credential · cache ·
                            telemetry · rt-messaging · runtime · runtime-core · gateway · agent ·
                            orchestrator · kit · crdt · rt-theme
```

**分根的三条前提，缺一条就不许分**（逐条可检，检查方式见 §7）：

1. 插件根**只许**依赖 `harness` + `plugin-proto` + 第三方 crate；一旦出现 `entities` / `dao` / `kit` / 任何实现层，
   当场判违规（providers 的 `kit` 依赖是现存第一例）。
2. 插件根**各自独立 `--target-dir`**（或全树共用一把 `Cargo.lock`）。不做这条，D7 的 E0460 必然复发。
3. 每个根逐字复制 `[profile.dev]` + `[profile.dev.build-override]` 两段与
   `clippy.toml` / `rustfmt.toml` / `rust-toolchain.toml`（`[profile.*]` 只在发起构建的那个根生效，
   写成员里 cargo 会忽略；8 根已踩过：缺它即退回 `incremental = true`，实测多编出 879 个僵尸会话）。

**为什么内核不做插件**：口径 2 已把数据主权留在宿主；`agent.loop` 等虽可出进程，但它们是宿主
装配路径本身 —— 与既有结论一致（`PLAN-everything-is-plugin.md:523-525`：基础设施接缝「A 层专属」）。

**源码仓库的终局是独立**（§0.5.1b）。同仓不同根只允许作为**脚手架期的临时工位**，退出判据三条在 §0.5.2 末尾；
没达成退出判据之前，不许把「同仓」当成终局写进任何阶段表。

### 4.3 现 8 根 → 新布局映射

> 「插件」列 = **独立 workspace 根内的一个 `[[bin]]` 目标 + 一份 `plugin.json`**，其根只依赖 SDK（§4.2 三条前提）。
> 目录分组（`axagent-core/` 等物理位置）保留；被收回主壳图的实现层根**只收回构建图，不搬文件**。

| 现根（成员数） | 去向 | 说明 |
|---|---|---|
| axagent-core（7） | harness / plugin-proto → **SDK**；entities → 内核；agent-command-types / agent-macro / disk-cache / rt-theme → 内核 | SDK 是插件根唯一允许的编译期依赖 |
| axagent-plugins（2） | plugins → **壳内核**（宿主侧载入器）；schema-gen → 内核 | 载入器属宿主，不是插件 |
| axagent-ai（6） | providers → **插件根**（需先补 D2，且先解掉 `kit` 依赖）；agent / orchestrator / mcp / prompt-guard / trajectory → 内核 | |
| axagent-quant（5） | astock-data → 插件根；quant + market-sim → **同一个插件根**（Rhai 沙箱与仿真同进程更自然）；analysis-engine / company-runtime → 内核（依赖 DB） | |
| axagent-infra（12） | document-parser → 插件根；其余 11 个全部内核（数据/凭证/缓存主权） | |
| axagent-runtime（8） | notification → **插件根（第一个试点）**；tools → 插件根（需先补 D4）；gateway / runtime / runtime-core / rt-messaging / rt-workflow / crdt → 内核 | |
| axagent-kit（1） | 内核（收回主壳图） | providers 对它的那条依赖是唯一例外，须在 §4.2 前提 1 里处理 |
| src-tauri（2） | **主壳**（就是它本身，不改名） | 其余 7 个子根（实现层）收回这张图 |

**首期插件根数：4**（notify · doc-parser · market-data · provider-*）—— **这只是草案**，
终局 N 由 §0.5.8 的 (a)(c) 决定；上限来自 §3「一条接缝动 6 处」的固定成本，一次铺 20 条必然失控。

### 4.4 SDK 发布：终局的硬前置（不是「生态需求，可推迟」）

> 本小节曾写过「同仓阶段代价近零 / 出仓本轮不做」。**那是错的**：§0.5.1b 已把插件独立仓定为终局，
> 而独立仓的第一块石头就是「SDK 在仓外可解析」。全仓今天**零** registry / 发布配置，
> `version + path` 出不了仓 ⇒ 这一条不解决，终局形态根本立不住。

三选一（P5.0 之前必须定，因为它决定 §0.5.7 第 2/3 条的治理强度）：

1. **vendored 目录源**（`cargo vendor` + `.cargo/config.toml` 的目录 registry）：改动最小、无外部服务，
   适合「我方写插件」阶段；代价是每个插件仓要同步一份 vendor。
2. **私有 registry**（企业内）或**公开 crates.io**：真发布、真版本矩阵，适合开放第三方；代价是发布权限与不可撤销性。
3. **git 依赖**（插件仓直接 `git = ` 指向主仓 tag）：最轻，但把「主仓的 tag 节奏」变成插件仓的依赖 ⇒
   只适合过渡，且过渡期也要有 §0.5.2 的退出判据守着。

**内核层的依赖怎么办**：`providers` 现在多依赖一个 `kit`（`axagent-ai/crates/providers/Cargo.toml:11`）。
两条路：把 providers 用到的那部分下沉进 `harness`，或把 `kit` 一并发布为 SDK 的一部分。
**不许**把 `kit` 变成插件可依赖的实现层——那会让 §4.2 前提 1 的依赖面门当场破。

---

## 5. 数据主权的落地形状

1. 新接缝 `host.query`：**受约束的读/写投影 API**，入参是「命名查询 + 结构化过滤」，不是 SQL；
   出参是 serde DTO（camelCase，遵禁区 13）。写侧按白名单逐 op 开，未列入即拒（沿用「表外即拒，不猜不兜底」）。
2. 必须拆掉的全局注水（否则投影层形同虚设）：`init/database.rs:397`（tools 持 `sea_db` 全局）、
   `init/database.rs:401`（DAO 仓库全局注册）、`init/database.rs:300`（sqlite-vec 进程内扩展）、
   `state.rs:79`（knowledge 回调直注）。这四处都要改成「宿主持有句柄 + 插件经接缝请求」。
3. 迁移次序：**先只读投影**（低风险，能立刻验证跨进程 DTO 往返），**再受约束写**，
   最后才讨论「插件自带缓存/凭证」的主权选择（行情源那格）。

---

## 6. 分阶段（2026-11-01 定案后重排；每阶段自带验收判据，不跑全量除非阶段收口）

> **重排的唯一依据是依赖关系，不是价值大小。**
> 关键依赖链：`协议批次（一次定版）` → `权限模型` → `SDK 出仓` → `能力逐条插件化`。
> ⚠ **协议改动必须打包成一次版本**：D2 流式（provider）、D4 ctx 投影、§0.6.1 兼容窗口、§3 策略会话式 op
> 四者都要动 `AXAGENT_PLUGIN_PROTO_VERSION`。(b)=第三方之后，**分四次改协议 = 四次让全部第三方 worker 集体失效**，
> 所以 P5.0 先定版号、后逐项实现，不允许「先上一个洞再补下一个」。

| 阶段 | 内容 | 验收判据 |
|---|---|---|
| **P5.0a** | **协议批次定版**：一版之内同时设计 ①流式分片（同 `request_id` 多帧 + `final` 哨兵）②`ToolContext` 可序列化白名单投影 ③`proto_version` 由相等改 `(min_compat, current)` 区间 + 逐能力 `cap.version` 协商 ④策略会话式 op（spawn / setParam / onBarBatch / drop + ctx 差分） | `PLAN-everything-is-plugin.md` §5.4 op 映射表新增行的草稿评审通过；版本矩阵方案落文；**这一版定完之前不写任何 worker 侧实现** |
| **P5.0b** | 实现 P5.0a 的四项 + 修 D3（manifest ↔ declaration 对账）+ 新错误码同步 11 语言 | 锁定测试：远程工具**仍走**宿主审批/脱敏/回滚栈；远程 provider 逐 token 输出与内置一致；声明未实现 ⇒ 注册被拒且可见失败；低一版协议 worker 在兼容窗口内**能装载**（负控：窗口外必须被拒） |
| **P5.0c** | **权限模型**（§0.6.2）：manifest `requires`（路径模式 / 出站域名 / 接缝子集 / 凭证 scope），未声明即拒；安装时逐条展示并显式同意 | 未声明任何权限的 worker 尝试读盘 / 出网 ⇒ 被宿主拒且返回可见错误码；权限声明进契约快照测试，改一条必须双仓红 |
| **P5.0d** | **签名与发布者身份**（D6 转正）+ 分发通道（产物来源、sha256 钉死、签名校验、撤销通道） | 篡改一个字节 ⇒ 拒绝装载；未签名产物在「第三方插件」开关下 ⇒ 拒绝装载；撤销某发布者的密钥后其插件全部降级为不可用 |
| **P5.1** | ① 主壳构建图收拢（**7 个子根并回 src-tauri 一张图**，文件不动）—— 实测改写面：**138 行 axagent-* 依赖声明带 `path=`（50 个 `Cargo.toml`，其中 126 行还钉着 `version=`）** + 8 份 `Cargo.lock` 合 1 + 7 份冗余 `[profile.dev*]` 退役（今天共 24 段）；**收益有实测锚点**：`check-workspace-config-parity.mjs` 头部记着拆分让全量 clippy 从单根 **9m24s** 变成 8 根串行 **67m53s**、僵尸增量会话 **879 个 / 68G** ⇒ 收拢是**止损，不是优化**；随它一起退役的是 `ci-split.yml` 与 `check-workspace-config-parity.mjs`（它们存在的唯一理由就是补偿拆分的重复配置）；② **SDK 可被仓外解析**（§4.4 定案：公开 crates.io 或私有 registry，(b) 已排除纯 vendored）；③ `plugin-testkit` 脚手架 + 参考插件仓模板（含 CI/profile/工具配置/权限声明样例）；④ §7 门禁上线 | 在**不含主仓源码**的目录里 `cargo build` 出 worker、被宿主装载、CI 独立跑绿（同仓装载不算通过）；跨根重复编译归零（用 §8 的 869/533 口径复测） |
| **P5.2** | **试点 A：notify-channel**（最便宜，纯出、无状态、现成只依赖 harness） | 6 个渠道至少 2 个进独立插件仓；卸载后 `SEAM_PROVIDER_UNAVAILABLE` 明确失败不挂起；主壳 `init/notification_adapters.rs` + `services.rs:5053` 具体类型 new 清零 |
| **P5.3** | **试点 B：doc-parser**（验证文件视图/字节流 + 大 payload 分块） | 知识库导入链路端到端跑通；两套 `OnceLock` 与 `commands/knowledge.rs:1169` 硬编码绕过收敛到接缝 |
| **P5.4** | D1（per-call 超时）+ D5（三态启用）+ `host.query` 只读投影（§5） | 插件僵死 ⇒ 超时后返回 `SEAM_CALL_TIMEOUT`、UI 不卡、可卸载；设置页三态可见；插件侧出现 `DatabaseConnection` 即门禁红 |
| **P5.5** | **provider**（前置 = P5.0a/b 的流式与兼容窗口） | 内置 14 家中至少 1 家以插件形态上线且逐 token 行为一致；`providers` 对 `kit` 的依赖已下沉进 SDK（§4.4） |
| **P5.6** | market-data（前置 = `astock-data/src/as_of.rs:36` 的 task-local `AS_OF` 改显式入参 + 缓存/凭证主权决策 + `market.provider.{源名}` 新接缝） | 回测与推荐链路在插件形态下读数一致；`app_state.rs:244` 的 `Arc<AStockClient>` 具体类型字段消失 |
| **P5.7** | strategy（前置 = P5.0a 的会话式 op）—— **(a) 已定全覆盖，不再复核「值不值得做」** | 单回测不爆帧（分批 onBarBatch）、与进程内策略逐 bar 结果等价；`commands/quant_backtest.rs:363-430` 的 `match strategy_type` 硬构造消失 |
| **P5.8** | RAG 后端接缝 —— **切法待拍（§0.6.3：解 1「嵌入+重排」/ 解 2「服务端向量库」）** | 按所选解：解 1 ⇒ 插件提供嵌入源与重排、向量存储留内核；解 2 ⇒ 插件提供向量库客户端、内核存储路径重写。两者共同判据：`rag_provider.rs:13` 的 `VectorQueryResult` 补上 serde 且过帧 |
| **P5.9** | **其余 15 条接缝收尾**（§0.7.4 未进 P5.2–P5.8 的：`search.source` / `image.gen` / `workflow.node` / `memory.provider` / `mcp.server` + §0.7.5 的 5 条钩子族），另含已建接缝但卡 D2 的反思/进化/优化器（`worker.rs:1527`、`:1551-1714` 那批 `remote_method_not_supported` 逐条补）、工具集投影层（D4）、平台适配器、业务规则、webhook / message.callback；**以及 §0.7.6 的 `DojoExtension` 第二协议合流（合流或废弃，二选一）** | 26 条逐条对照 §0.5.3 三条判据（宿主侧无具体类型 / 新增同族不改主壳 / 卸载明确失败），**逐条打勾表进 `docs/audits/`**；接缝四方对账门（§7-1）在 15 条新接缝全绿后才允许摘 `--ratchet` |
| **P5.10** | 装配收敛：把 1.25 万行手工 new 中的「可插拔能力」部分改为注册表驱动 | 新增一个同类能力只需：插件仓产物 + manifest 声明 + 权限申请；**不改主壳代码**（最终判据） |
| **P5.11** | **全面出仓**：满足 §0.5.2 退出判据的插件逐个挪出主仓（主仓删除源码，只留主壳 + 内核 + SDK） | 主仓 `grep axagent-plugin-` 命中为 0；每仓 CI 不依赖主仓源码；兼容矩阵记录老壳 × 新插件的行为 |
| **P6.0** | **功能包层前置**（§0.8.4/§0.8.5 六项，排在 P5.11 之后、不与 L2 的 26 条混排）：① `WorkflowNode` 封闭 enum 开放化（`harness/src/workflow_types.rs:1366`）② `WorkEngine` 角色定性（底座 or 功能包，`app_state.rs:345` 证明它今天当底座用；依赖方向已核实为「域 → 引擎」，故换轨可行）③ 插件命令贡献通道（升格 `DojoExtension`，或给 `build.rs` 的注册面加运行期入口）④ 前端 tab/route/menu 数据驱动（复用已有 DynamicPageViewer 机制，改 `WorkspaceHub.tsx:166-173` 与 `workspaceTabs.tsx:28/70`）⑤ **引擎去域化**（`agent_executor.rs:4385` `PREFETCHABLE_TOOLS`、`:1232-1246` `stock_code` 预取、`:854` prompt 文本 —— 一律改为节点配置或接缝贡献）⑥ **节点执行器与接缝单轨化**（executor 只能是接缝薄壳、禁止自带实现；`document_parser_executor.rs` 空壳接真接缝，`notification_executor.rs` 的四平台 webhook 并入 `notify.channel`） | 判据：**新增一个工作流节点类型不改主壳代码**（今天需 8 处，见 §0.8.3-1）；**引擎内 `grep -i stock` 生产区命中为 0**（今天 32 行）；**每个节点执行器都能指到一条接缝**（今天 32 个执行器 0 个走能力注册表）；把一个功能包的页面 + 命令 + store 移出主仓后，其余功能编译与运行均不红 |

### 6.1 P6 估工与排期（2026-11-01；先说清这不是待定项，是我给的数与可打回的清单）

**我上一版把它写成「要你拍排期」是推卸** —— 排期依赖的是人力与窗口，那两个数只有你有；
但**估工、依赖顺序、里程碑切法**是判据能裁的，下面全部给死，你只需要填人力。

| 单元 | 改动面（实测） | 估工（人日，含改写+自测，不含评审） | 前置 |
|---|---|---|---|
| **P6.0-① `WorkflowNode` 开放化** | 今天新增一个节点类型要改 8 处（§0.8.3-1）；改造面 = enum + `node_executor_trait.rs:233` 穷举 match + deserializer + dispatcher 注册 + schema_validator + 前端 nodeTypes 表 + PropertyPanels + i18n | **12–18** | 无（可与 ③④ 并行） |
| **P6.0-② `WorkEngine` 角色定性** | 纯文档+一处 `app_state.rs:345` 字段说明，无代码分叉 | **1** | 无 |
| **P6.0-③ 命令贡献通道 T2** | 一条静态派发命令 + worker op 路由（形态照 `plugins/src/manager.rs:551`，但要补 `input_schema` 与权限声明、去掉写死的 `ReadOnly`）+ 废弃 DojoExtension（#30） | **8–12** | #30 先裁完（已裁） |
| **P6.0-④ 页面贡献通道 T3** | 见 §0.12.5 三选一：选 A 是**配置 + CSP + 桥扩到 page**（3–5）；选 C 另加 10+ | **3–5**（选 A） | ②的选择由你定，不影响其它项 |
| **P6.0-⑤ 引擎去域化** | `agent_executor.rs:4385` `PREFETCHABLE_TOOLS`、`:1232-1246` `stock_code` 预取、`:854` prompt 文本 ⇒ 改节点配置/接缝贡献；生产区股票词 32 行 | **5–8** | ③④ 之后（要有贡献通道才能把知识挪出去） |
| **P6.0-⑥ 执行器与接缝单轨化** | 32 个执行器 0 个走注册表；`document_parser_executor.rs`(59 行空壳) 接真接缝、`notification_executor.rs`(165 行自带四平台 webhook) 并入 `notify.channel` | **10–15** | ⑤ 同批（都是引擎侧） |
| **P6.1 股票迁出** | 改写 152 处 / 28 文件 + 54 处前端反向 + 304 条命令挂 T2 + 20 表转 `schema.json` + 84 873 行前端出包 + 3 527 key | **45–60** | P6.0 全部（③④⑤⑥ 是关键路径） |
| **P6.2 OPC 迁出** | 0 处主壳改写 + 28 处前端 + 138 条命令 + **29 处跨包口改 `call_seam`** + 28 表 + 12 797 行前端 | **20–28** | P6.1（跨包口指向股票 crate）+ D1 超时先修 |

**合计：P6.0 ≈ 39–59 人日，P6.1 ≈ 45–60，P6.2 ≈ 20–28 ⇒ 全段 ≈ 105–147 人日**（单人 5–7 个月，3 人并行受限于关键路径约 2.5–3.5 个月）。
误差来源先说清：① 这些是**改动点数 × 单点成本**的外推，不是实测 —— 仓内唯一的实测锚点是 §3 的「新增一条接缝 6 处固定成本 ⇒ 13 条约 78–90 处改动点」；
② 前端 84 873 行的出包成本没有先例，估工里 **±30% 的不确定性最大**；
③ 未含 D1/D2/D4 三个协议缺口的修复工时（在 P5.0a，另算）。

**推荐排期（按依赖，不按难度）**：
1. **P6.0-③ + ④ 先做，且合起来做一条垂直切片** —— 拿 OPC 的**一个**页面 + 10 条命令走通「清单声明 → 派发 → 渲染」，
   这一片打通才能把 ①⑤⑥ 的估工从外推变成实测（这是我建议的第一个真实里程碑，约 8–12 人日）。
2. P6.0-⑤⑥（引擎侧，可与 1 并行，不同人）。
3. P6.0-①（节点类型开放化，工作量最大但可独立验收）。
4. **P6.1 股票**（必须整块投人，中途切手成本高于收益）。
5. **P6.2 OPC**（含 D1 超时修复 —— 它是硬前置，不是优化）。

**为什么不让 OPC 先做**（我上一版说过，撤回）：它主壳阻塞确实是 0，但那 29 处引用指向股票 crate ⇒
股票还在主壳里时，OPC 迁出后只能反向依赖主壳 = 不是插件。顺序由这条决定，不由工作量决定。
阶段收口才跑全量（CI 口径三条，见 AGENTS.md）；阶段内用 `cargo check` + 定点 clippy。

| **P6.1** | **股票业务迁出为 L1 插件**（任务 #35）：改写 §0.10.1 桶① 的 152 处 / 28 文件（前 7 个文件占 **106** 处），304 条命令走 T2 通道，54 处前端反向 import 清 0，20 张表改由包内 `schema.json` 声明 | 主壳 `grep` 域 crate = 0；`src-tauri/Cargo.toml` 无股票 crate；卸载股票插件后其余 1 099 条命令可用、`stock_*` 表与数据保留 |
| **P6.2** | **OPC 业务迁出为 L1 插件**（任务 #36，**不得先于 P6.1**：OPC 的 29 处引用指向股票 crate）：138 条命令走 T2，28 处前端 import 清 0，29 处跨包口改 `call_seam` 并在 §0.6.2 权限面登记 `declared_calls` | 同上判据；且 OPC 停用不影响股票包 |
| **P6.3** | 其余功能包按 P6.1/P6.2 模板批量迁出：工作流编辑器（83 条命令）、**知识库/Wiki 业务功能面**（131 条命令 / 11 827 行，阻塞面 72 处 / 25 文件，见 §0.6.3 C 层；它与 RAG 解 1/解 2 的取舍无关，可独立排期）等 | 每包一条 `grep 域 crate = 0` 的门禁记录，进 `docs/audits/` |
---

## 7. 新增门禁（两处同源接线，命令行逐字一致）

1. `scripts/check-plugin-seam-table.mjs`（`--selftest`）：`PLAN-everything-is-plugin.md` §5.4 op 映射表 ↔ `SUPPORTED_REMOTE_SEAMS`
   ↔ `register_remote_seam` match 分支 ↔ `ServiceDefinition` 构造函数 **四方对账**。
   任一单边改动即红 ⇒ 把「新接缝怎么加」从实践痕迹变成机械判据。
2. `scripts/check-shell-dependency-surface.mjs`（`--selftest`，棘轮基线只减不增）：
   `src-tauri/Cargo.toml` 的 `axagent-*` path 依赖面 + `init/**` 里直接 `axagent_<impl>::` 具体类型装配点数。
3. `scripts/check-plugin-root-discipline.mjs`（`--selftest`）：把 §4.2 三条前提钉成机械判据 ——
   ① `axagent-plugin-*/Cargo.toml` 的 `axagent-*` 依赖**只许** `axagent-harness` / `axagent-plugin-proto`
   / `axagent-plugin-testkit`，出现实现层（`entities` / `dao` / `kit` / `storage`…）即红（providers 的 `kit`
   是现存唯一违例，进基线并只减不增）；② 每个插件根必须有独立 `--target-dir` 声明或 `build.target-dir`，
   且与主壳不同；③ 每个插件根的 `[profile.dev]` 两段与 `clippy.toml` / `rustfmt.toml` /
   `rust-toolchain.toml` **逐字等于**主壳那份（这条是 8 根踩过 879 个僵尸会话的那个坑）。
4. `scripts/check-plugin-artifact-contract.mjs`（可选，P5.5 后）：产物 `plugin.json.worker.program`
   与 describe 自述的一致性 + sha256 钉死（在 D6 签名方案定了之前只做完整性）。

三条都按既有规矩**两处注册**（`scripts/ci-check.mjs` + `.github/workflows/ci.yml`，先 `--selftest` 再判定），
由 `src/__tests__/ci.workflow.test.ts` 的对称差集护栏自动校验；**不往 ALLOWLIST 塞**。
ci.yml 侧一律**追加在文件末尾**（本文件被 18+ 处 `ci.yml:<行号>` 引用，中段插入即制造一整桶 BROKEN）。

### 7.1 本批已接线（2026-11-02，两处同源 + ci.yml 末尾）

5. `scripts/typecheck-unc.mjs`（`--selftest`，任务 #45）：**UNC 安全的全量 typecheck**。
   `tsc --noEmit` 在 VMware 共享目录上走 `include: ["src"]` 目录 glob ⇒ **枚举不到输入**，
   实测退 0 而 `--listFiles` 列 **0 个文件**（塞一处故意类型错误照样退 0）⇒ 本仓历史上所有
   「typecheck 通过」都不算证据。本门用 fs 遍历把 `src/**` 的 1433 个 `.ts/.tsx/.d.ts` **逐条**写进
   临时 tsconfig 的 `files` 再跑 `tsc -p`，三档 fail-closed：枚举 0 输入即红 · 输出命中 tsc 帮助横幅即红 ·
   生成配置混回目录 glob 即红。单次耗时实测 **67.6s**（已进 pre-push 快速档，代价见 §8）。
6. `scripts/check-capability-register.mjs --repin-plan`（任务 #48，第五道门的**维护模式**，不进 CI）：
   把镜像表里的 PLAN 行号引用按定位键重锚到当前文本。判据：钉句按 `mustContain` 全部 token 同行命中、
   登记项按 `planAnchorText` 整行逐字命中 ⇒ **命中 1 行才回写，零命中或多命中一律拒绝写盘**（不许猜行、
   不许插值）；写盘前逐字节备份到 `.tmp/`，回写后从盘上复检（`checkPlan` 归零）不过即回滚。
   ⚠ **红线：判定档仍按「第 N 行含该句」逐行核**，重锚只改表里的数字，绝不用全文模糊匹配替代行号锚点
   （真漂移必须还能被抓住）；`--selftest` 的负控㉑g 机械锁住这条。本模式**不碰 `evidence` 的源码行号**
   （代码侧漂了是该抓的缺陷，自动改写等于洗白）。

### 7.2 本文（活文档）的编辑约束 —— 行号被机器钉住，但「行数中性」不是要求

本文 §0.7 的行号被 `scripts/capability-register.mjs` 用**绝对行号**钉住（`planPinned[].line` +
`mustContain` 11 处、`entries[].planLine` / `planLineRange` 38 处）。已实测的旧形态：§0.7.6 从 5 行改成
4 行 ⇒ 全文少 1 行 ⇒ 8 处钉句**同时假红**，补回同数行才转绿 —— 那等于给活文档施加一条没写在任何地方的
约束「编辑必须行数中性」。自 #48 起这条约束改成一条**工序**：

1. 随便改本文（不必凑行数）；
2. `node scripts/check-capability-register.mjs` ⇒ 会红，且逐条点名受影响的锚点/登记项；
3. `node scripts/check-capability-register.mjs --repin-plan` ⇒ 唯一定位的行号自动回写（有歧义则拒绝写盘并
   逐条报告，那种情况要人工把 `mustContain` 收紧到只命中一行）；
4. 再跑第 2 步的判定 ⇒ 应当 EXIT=0。
   ⚠ 顺序约定：**改了本文就先把第 3 步跑掉再去做别的事**，不要把假红留在树里等人处理（假红的代价是
   训练人去忽略红）。`caliber.basisLines` / `pipeline.lines` 这两组是纯文档指针、无定位键，
   `--repin-plan` 不重锚它们，行数变动后须人工核对。

---

## 8. 代价（诚实口径，用实测外推，不报好消息）

- 实测基线（`PLAN-repo-split.md` §20.9）：8 根全量 clippy **冷 41m36s / 暖 1m37s**；僵尸增量会话 879 → 0。
- **「根数」不是同一个量**：实现层根（互相咬合、都喂 src-tauri）加一个就多一份锁漂移面 ⇒ 一个都不该再加；
  插件根（只依赖 SDK）加一个是「多一把小锁 + 一个小 target」，且**它才是「只重建改动的插件」这条收益的来源**。
  每根仍须逐字复制 `[profile.dev*]` 与 `clippy.toml` / `rustfmt.toml` / `rust-toolchain.toml`
  （`[profile.*]` 只在发起构建的那个根生效，否则退回 `incremental = true`）。
- **⚠ 前置缺陷 D7：8 根共用一份 `target`，但各持独立 `Cargo.lock`**（2026-11-01 实测）。
  环境变量 `CARGO_TARGET_DIR=C:\Users\刘小平\.cargo\target-axagent` ⇒ 8 根全部写同一目录；
  而 8 根锁已漂出 277 包 ⇒ 同一 crate 在不同根解析出不同版本/feature ⇒ 元数据 hash 不同 ⇒
  **共享目录给不出跨根复用**。一次串行全量测试的计数：869 个 crate 中 **533 个在 >1 个根里被重新编译**
  （`axagent-harness` ×8、`sqlx` 全家 ×8、`tracing`/`chrono` ×7）。
  直接死因形态：`src-tauri` 根 `cargo test --workspace` `exit=101`，报
  `E0460: found possibly newer version of crate 'rmcp'`（同一 target 里躺着 5 份 `librmcp-*.rmeta`）
  与 `E0463: can't find crate for 'axagent_lib'` —— **失败在 bin / test target，lib 本身编过了**。
  ⇒ 对 P5 的后果：**「CI 按根并行」这条收益在当前形态下不成立**（并行只会加剧同一类元数据竞态）。
  **复现取证（同日）**：不清理、原样复用同一共享 target 跑 `cargo check -p axagent --all-targets`
  ⇒ `exit=0`、**19m19s**、0 error，E0460/E0463 **未复现** ⇒ 判定为**产物陈旧/竞态**而非源码缺陷。
  ⚠ 该绿有边界：`check` 只产 `.rmeta`，不产 `.rlib` 也不链接，而原报错含
  `crate 'rmcp' required to be available in rlib format` ⇒ **「src-tauri 根的 test target 能构建」尚未验证**，
  判据是 `cargo test -p axagent --no-run`。
  **P5 的插件根只依赖 SDK（§4.2 前提 1）**，与实现层根的漂移面不同量级，不加重 D7；
  但 D7 必须在 P5.1 的「主壳图收拢」里一并解决 —— 它同时是收拢的理由，也是插件根 `--target-dir` 规则的来源。
- 本机磁盘：`target` 实际落在 **C:**（`~/.cargo/target-axagent`），跑 8 根全量测试期间 C: 从 146G free
  掉到 **121G free**。⇒ P5.1 之后每个插件根自带独立 `--target-dir`（前提 2），**新增的是 N 份「只含 SDK + 第三方」的小 target**
  （量级参考：`axagent-core` 根本次只编 25 个目标），不是 N 份 709 目标的大 target；
  但仍建议先把主 target 迁 D:（任务 #21），再开插件根。
- D2（流式分片）与 D4（ctx 投影）是**协议改动**，会动 `AXAGENT_PLUGIN_PROTO_VERSION` ⇒ 所有现存 worker 需重编，
  这是本轮唯一「会打破兼容」的一类工作，必须排在试点之前讲清楚。

---

## 9. 不做清单（2026-11-01 定案后重编：三条拍板删掉了两条原有豁免）

- 不做 WASM / cdylib（口径 1 已定 worker 可执行；`PLAN-everything-is-plugin.md` §1.1 的主备倒置结论沿用）。
- 不做运行期编译管线（`PLAN-everything-is-plugin.md` §12 整节出范围）。
- 不做 `session.store` / `event.dispatch` 的**提供面**远程化（既有判据不变，插件仍是订阅方）。
- 不做**前端通用贡献点市场**（第三方任意贡献 UI 的开放生态，`PLAN-plugin-gap-closure.md:26` 另案）。
  ⚠ 但 **L1 功能包所需的 tab/route/menu 数据驱动 + 页面产物装载（P6.0 ④ + §0.10.2 T3）不在此列** ——
  它是股票/OPC 迁出的硬前置（§0.10.3 的 54 + 28 处前端反向阻塞只能靠它消化），**不属于「不做」**。
- ~~不在本轮做插件签名~~ —— **作废**。(b)=第三方 ⇒ 签名与发布者身份是终局要件（§0.6.2 第 3 条）。
- ~~能力清单里的策略 / RAG 可以缓做~~ —— **作废**（(a) 全覆盖，范围以 **§0.7 登记表**为准（登记 26 / 有实证 24），不再是手工的 12 条）。RAG 的切法**已裁定解 1**（§0.12.3）。
- ~~「市场仿真」是能力面，要出仓~~ —— **我判为内核**（§0.7.7：DES 每 tick 的 agent 变体，出进程即每步一次 IPC）。这条与拍板措辞有出入，需你确认或拉回。
- **新增一条明确不做**：不做 **OS 级强隔离沙箱**（容器 / Job Object / seccomp 一类）。
  终局给到的是「声明式权限 + 宿主侧执法 + 用户显式同意」，**不许把它称作沙箱**；
  `source_audit.rs:5-7` 那句「形式合规检查，不是安全边界」在第三方时代依然成立，这是能力的上限而不是遗漏。

---

## 10. 一句话总计

**机制已建，分解未做**：接缝注册表、帧协议、投放已编译产物、环检测与优雅卸载都在，
缺的是 ①「新接缝」（**13 条，上限 15** = §0.7.4 的 10 + §0.7.5 的 5 减去两条待证；按 §3 的 6 处固定成本外推约 **78–90** 个改动点）
②**「SDK 可被仓外解析」**（§4.4，独立仓的第一块石头）
③「脚手架与参考插件仓模板」④「D1/D2/D4 三个协议与语义缺口」⑤「依赖面与主壳图门禁」
⑥「主壳构建图收拢」（把今天 7 个子根（实现层）收回成员）⑦「Dojo 第二插件协议合流」（§0.7.6，不定它范围就永远不平）
⑧「功能包层 L1 的六项前置」（§0.8.4 + §0.8.5 = P6.0，排在 P5.11 之后）—— 不做它，「一切皆插件」只覆盖 L2 提供方接缝一层，
 工作流编辑器 / 股票分析 / 认知路由这些**用户可见的功能包**会整层留在主壳里。
**其中 ②⑥ 是终局的立身条件**：SDK 发不出去，插件就出不了仓；出不了仓，它就不是插件（§0.5.1b）。
**第一个试点选 notify-channel，而不是 provider** ——
不是因为它最有价值，而是它同时满足「K2 键控站点已存在」（`notification/src/dispatcher.rs:114`）与「K4 无流式无句柄」，
能在不签协议版本的前提下把整链走通（独立仓 → 产物 → 装载 → 卸载 → 降级）拿到真实代价数字。
⚠ (a) 已拍**全覆盖**，所以 provider 与 strategy 的协议改造**不在「值不值得」的讨论范围内**，只在「第几批做」的范围内。

### 待你拍的决定 —— **2026-11-01 第二轮：能裁的我已自行裁完，见 §0.12**

**已裁完（裁定与证据在 §0.12，此处只留结论一行）：**

- ~~**§0.6.3** RAG 切法~~ → **裁定：解 1**（接缝切算法面，存储面留内核）；pgvector 分支已在代码里，将来要上服务端形态解 2 成本≈0。
- ~~**§0.7.6** DojoExtension~~ → **裁定：废弃 trait，形状吸收进 L1 命令通道 T2**（生产实现者 0，唯一实现是 `dojo_extension.rs:414` 的测试替身）；顺带修掉 CLI 通道里写死的 `ReadOnly`。
- ~~**§0.7.7** 市场仿真~~ → **裁定：维持内核**，可插部分下放到「策略数据」层（场景/参数/脚本）；`kernel.rs:703` 每 tick 遍历全部 agent，出进程即 tick × agent 次 IPC。
- ~~**§0.7.8** 第五道门~~ → **裁定：建**（本轮自查就抓出 15 处数字漂移，分母不可信则全部判据失效）；可复用件已在，接线约 1 人日。
- ~~**§0.10.2 T3** 前端装载形态~~ → **裁定：两条都要，按页面复杂度分流**（`PluginPanelHost.tsx:30` 的受限 IPC 桥已存在，扩到 page 粒度即可；股票 84 873 行走 bundle，OPC 表单类走 DynamicPage）。

**真正还开着的（都不是架构问题，我裁不了）：**

1. **是否引入服务端部署形态**（解 2 的产品前提）——与桌面单机定位相关，只影响将来，**不阻塞任何阶段**。
2. ~~CSP / asset 协议的安全评审口径~~ → **已定 A**（§0.12.5）：开 `assetProtocol`、CSP 加 `asset:`、允许根只限插件安装目录。
   仍开着的一小条：现 CSP 里的 `'unsafe-eval'` 何时收 —— 已剥出插件化批次单独排期（与插件装载无关，不该混评）。
3. **P6 的排期与人力**：P6.1 股票（152 处改写 + 304 条命令 + 84 873 行前端）与 P6.2 OPC 谁先投入、投多少人 —— 这是项目计划，不是架构判据能裁的。

---

## 11. 未达成与未验证（截至 2026-11-01；本节是自查产物，不是待办清单的重复）

**本方案没有做到的部分，逐条列清，防止「门禁全绿＝方向成功」的误读：**

1. **没有任何一条链路被原型验证过。** 尤其 P6.0 的 T2（命令派发口）与 T3（前端装载）只有设计，**没写过一行可运行的东西**；
   「股票能拆成插件」目前是**机制层面成立**，不是跑通层面成立。
2. **10 条待建接缝的 K4 DTO 分级未做**（任务 #29）⇒ P5.0a 不许签协议版本号。
3. **OPC 仍是待迁出、域包口径未收口**（任务 #32 已改为迁出口径，9 887 + 2 064 行仍在盘上）⇒ 本文所有计数（24 / 13 / 304 / 138）都基于**混合作业树**，
   `git status` 显示 `stock_analysis.rs`、`recommender/mod.rs`、`stock_workflow/serenity.rs` 等还是其它会话的未提交改动。
   收口后**必须重跑两份 audit 脚本**，否则数字不作数。
4. **29 处跨包口是一条未消化的风险**：P6.2 之后 OPC 要在运行期调股票插件，而 D1（宿主调插件无超时）+ 
   「每个 worker 只有一条读线程」的既有事实 ⇒ **股票 worker 僵死会让 OPC 插件整批失联并级联到宿主调用线程**。
   环检测防得住环，防不住僵死。**D1 必须在 P6.2 之前修**，否则「插件可组合」是反向的卖点。
5. **`scripts/audit-*.mjs` 两份清点脚本未接任何门禁**（它们不是 `check-*` 门禁，按 AGENTS.md 不强制两处同源，
   但也意味着数字会随代码漂移而无人报警）。⇒ 第五道门 `check-capability-register.mjs` **已裁定要建**（§0.12.4 / 任务 #38），建好之前本文数字仍属人工核对。
6. **src-tauri 根的 test target 可构建性仍未验证**（任务 #19 收尾项，判据是 `cargo test -p axagent --no-run`，见 §8 的 D7 条）。
7. 两份文档 + 两份 AUDIT **原本不在版本控制里**：真凶是 `.gitignore` 的 `docs/` 一行（`git check-ignore -v` 指向它），
   而非同文件另一处冗余的 `Docs/`——后者在 `core.ignorecase=true` 下也命中 `docs/`，属叠加而非独立成因。
   2026-11-01 已移除 `docs/` 排除项：`docs/` 下 176 份文件（175 `.md` + 1 `.json`，共 7 MB）此前**零入库**，现全部转为
   untracked 可提交状态。剩余未达成：批量导入需单独一次提交，本文与两份 AUDIT 随本轮提交。
8. **本批已落地并让第五道门转绿（2026-11-01 收口读数）**：`#30` 删 `dojo_extension.rs`（−460）+ 命令通道改真授权；`#39` 交付配置面 + 共享桥（`pluginSandboxBridge.ts`，page/panel 同一份白名单）；主代理开 `protocol-asset` feature 解除主 crate 编译阻塞。⇒ 权威读数 **2004 个 `.rs` / 315 个 `pub trait`**（两档各降 1，旧读数按门禁规矩只记在 §0.12.4）。`node scripts/check-capability-register.mjs` **EXIT=0**。
9. **tool 侧权限档位仍是装饰**：`#30` 只把**命令通道**改成真授权。`PluginTool::check_permission`（`plugins/src/types.rs:645-684`）与唯一引用它的 `execute_with_permission_check` **至今零调用方** ⇒ 「插件工具能碰什么」这条线仍未执法，是 §2 缺陷表里独立的一条，不是本批已完成项。
10. **相对路径声明的插件命令今天根本跑不通**（`#30` 端到端实测）：命令串整团交 `Command::new`、可执行文件查找按宿主 cwd（子进程 `current_dir` 不参与）⇒ Windows `OS error 3`、POSIX 同理。修法二选一未定（走 tool 侧同款 `resolve_hook_entry` vs 真按空白切分），任务 #47。
11. **A 通道端到端仍未通**，四份额外输入：`#43` 资产绝对路径解析命令、`#44` `PluginManifest.pages[]` 逐页声明、`#46` 插件侧桥客户端运行时（`asset` 装载的第三方 bundle 够得着桥），以及一次 `npm run tauri dev` 的运行期实测（Windows 上确认 origin 是 `https://asset.localhost`）。§0.12.5 那句「工程量小：一条配置 + CSP 一段 + 白名单测试」**已被实测推翻**，且它当时的判断「`enable: true` 只是惰性生效」也错——缺 feature 时 tauri `build.rs` 直接**硬失败**，不是静默不生效。
12. **本地 `npm run typecheck` / `tsc --noEmit` 在 UNC 共享目录静默空跑**（退 0、`--listFiles` 列 0 个文件；主代理与 `#39` 各独立复现一次）⇒ **本文与历史会话里所有「typecheck 通过」的记录都不算证据**。CI 在本地盘跑，仍有效。固化绕过 = 任务 #45。
13. **`docs/` 仍有 173 份文档未入库**（`.gitignore` 已解除排除，全部转 untracked 可提交）；批量导入前需处置 `docs/plans/PLAN-asof-gaps.md:558` 的一条 `Acs-Token: …` 签名串（全仓密钥扫描唯一命中）。
14. **本文的行号已被机器钉死**：镜像表用 `planLine` / `planPinned.mustContain` 锚 PLAN 具体行（实测本轮我少写 1 行 ⇒ 8 处钉句假红，补回同数行才转绿）。⇒ **今后编辑本文必须「行数中性」**，或给第五道门加 `--repin-plan` 重锚模式（否则正常的文档演进会持续制造假红，而假红的代价是训练人去忽略红）。
15. **`WorkflowNode` 封闭 enum 不是 P6.1 的阻塞**（2026-11-01 调研订正）：编译强制穷举位点实数 **13 处 / 9 文件**（32 臂的 base_id/base/base_mut/base_output_var、assembly_builder 两处、node_type_name/node_kind、get_node_type、workflow_template 两处、fanout、seed_stock_analysis:8140）+ enum + deserializer 双处 ⇒ 贡献新节点类型 Rust 强制 **16 位点 ≈55–75 行**，而前端 4 张表全带 default 兜底 ⇒ **静默降级不报错**（这是更危险的那一半）。但**股票只使用现有变体、harness 不迁 ⇒ 对 P6.1 改动面为 0**，它属于 P6.3（工作流编辑器迁出）。⇒ P6.0-① 从 P6.1 的关键路径上摘除。
16. **P6.1 真正的先决裁定是「命令执行形态」，不是 enum**：实测 **280/304 条股票命令签名带 `State<'_, AppState>`**（无状态仅 24 条），而今天的 T2 现形是 `plugins/src/manager.rs:583` **一次性 spawn 子进程、stdout 原样 String**，13 条远程接缝也不含命令通道 ⇒ **spawn 形状承载不了主量命令面**。§0.12.1 承诺的「把 dojo 的 `command_specs/execute_command` 形状吸收进 T2」只落了授权那一半，派发口形态**没有落地**（dojo 本体已删，缺口必须补）。⇒ 裁定（主代理）：**长驻 worker + 命令帧**，命令内对宿主状态的访问一律走查询接缝（唯一与「插拔的是已编译好的 worker 产物」「数据主权在宿主」两条已拍口径同时自洽的解）；进程内贡献与 worker 产物口径相悖，不取。
17. **152 处的分桶（可直接排期）**：D4/`tool.set` 门 **61**（tools crate 9 文件，随 T1 整体进插件）、装配层 **44**（`services`/`state`/`app_state`，`MarketDataProvider`/`StockStore` 已在 harness ⇒ 只需改 trait 对象注入）、跨面 `call_seam` **15**、开发二进制 **11**、进化族 **7**（A 组接缝已建）、长尾 **14**（其中 `market_sim_service.rs` 2 处建议按 §0.12.2 从股票桶摘除）。**152 处中没有任何一处需要等 #43/#44**（那是 T3 前端账）。
18. **前端收口点已定位**：全站唯一 `invoke` 收口在 `src/lib/invoke.ts:462` ⇒ 加一层转发即可让 **240 个字面调用点零改动**；304 条命令中 **173 条被 76 个文件消费、131 条零前端消费**（可作删/缓迁弹药）；76 个文件里 **63 随 bundle 迁、13 留守主壳（37 点）**。⇒ 建议页面首批留主壳、Rust 先行，避免与 T3 未完成的三件互相阻塞。
19. **本批修掉的管线缺陷（我自己提交时带入的）**：`scripts/audit-capability-inventory.mjs` 模板字符串里一处**未转义反引号**导致整文件 `node --check` 失败（AUDIT 无法复现）。已修，现 `node --check` EXIT=0 且脚本可重跑（本次重跑把该 AUDIT 按当前盘面更新了 3 行）。⇒ 教训：`audit-*.mjs` 不属 `check-*` 门禁、无 selftest 覆盖，改它的模板串后必须手动 `node --check`。
