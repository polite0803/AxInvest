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

按这三条判，终局的**能力全集**以 **§0.7 的登记表**为准（**登记 23 条** = A 11 + B 8 + C 4，逐条带行号证据；另有 3 条移入 §0.7.9 债档、只登记不计数；其中 **22 条有实证**，`rule.custom` 经核为零外部实现者 ⇒ 降级待证，权威读数见 §0.9.4）。
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
  机器核准后的范围见 **§0.7 登记表（登记 23 条 / 有实证 22 条）**，本节只锁「不许有豁免」这一意志。原「策略与 RAG 代价高于收益，可只覆盖前 8 类」的建议**未被采纳**，
  因此 §9 里对应的两条豁免作废，P5.7 / P5.8 变成必做项（RAG 的**切法**仍待单独拍，§0.6.3）。
- **(b) 谁能写插件 → 「第三方」**。签名、兼容窗口、权限模型、审核与分发通道全部升级为硬前置；
  并且它顶穿了「握手严格相等」与「清单无权限字段」两处现存设计 —— 逐条见 **§0.6.1 / §0.6.2**。
- **(c) provider → 「进」**。D2 流式协议改动成为必做，且必须与 (b) 的兼容窗口**同一次定版**（P5.0a）。

> 这三条定完之后，剩下的唯一形态级未决项是 **§0.6.3 RAG 向量的切法**（其余都是工序）。

---

## 0.6 三条拍板（2026-11-01 **定案**）与由此顶穿的现存设计

| 决定 | 拍板 | 立刻变成终局要件的东西 |
|---|---|---|
| (a) 能力覆盖 | **全覆盖（范围 = §0.7 登记的 23 条、其中有实证 22 条；拍板时写作「12 类」，那是我手工挑的口径，已由机械登记表替代）** | 需新建接缝 **11 条**（§0.7.4 的 8 + §0.7.5 有实证 3；一条待证成立则 12）+ 策略会话式批量 op + RAG 后端接缝 + MCP/工具集投影层 + Dojo 第二协议合流（§0.7.6）。**§9 里任何「某类不做」的豁免作废** |
| (b) 谁能写插件 | **第三方** | 签名（D6）、SDK 兼容窗口、审核与分发通道、权限模型 —— 全部从「登记/可推迟」升级为**硬前置**。§0.5.7 六件事一件不能省 |
| (c) provider | **进** | D2 流式分片协议改动，且因为它打破兼容 ⇒ 必须与 (b) 的兼容窗口**同时**落地，不能先改协议再补窗口 |

### 0.6.1 顶穿之一：握手是**严格相等**，第三方不可能被强制同步升级

`axagent-core/crates/plugin-proto/src/lib.rs:338`：

```rust
if self.proto_version != AXAGENT_PLUGIN_PROTO_VERSION {
    return Err(DeclarationError::VersionMismatch { declared, expected });
}
```

`AXAGENT_PLUGIN_PROTO_VERSION: u32 = 1`（`:61`）⇒ 今天**任何**协议改动都会让所有第三方 worker 当场被拒载。**例外已于 2026-11-02 由 `#49` 实证成立：「加法式新 op」不触 bump** —— 新增 `ops::COMMAND` 未 bump 版本，旧 worker 收到未知 op 回 `PLUGIN_UNKNOWN_OP` 错误帧、新宿主把它解码成 Fault（**降级为失败而非误成功**）。⚠ **这不是「有测试锁住」**（独立审查 `#49` 的 A2 判我超额表述，已订正）：`plugin-proto/src/lib.rs` 的测试 `command_frame_is_additive_no_proto_bump_required` 锁的是**宿主侧对合成错误帧的解码**，`worker.rs` 的三条用 `Peer` 测试替身回构造帧 ⇒ **没有任何测试能锁住「真实旧 worker 二进制会回该帧」**，那只是 handler 模板惯例（`plugin-proto/src/lib.rs` 头部模块文档的 handler 模板段）。⇒ 准确措辞 = **「约定 + 夹具示证」**。且该降级今天带来一条必修回归：见 §16.1。⇒ bump 判据本身仍收窄为**「改动会让既有帧的语义发生歧义」**（加字段、加 op、加错误码不必 bump；改既有 op 的入参形状或回帧语义才必须）。
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
| **B 存储面** | `vec0` 虚拟表 + FTS5 + `vec_collections` 注册表 + chunk 元数据 | ✘ **不能**（口径 2 的正面撞墙处） | `vector_store.rs:114` `VectorStore { db: DatabaseConnection }`；`:127 new(db)` 直接吃宿主连接；sqlite-vec 是**进程内扩展**，唯一注册点 `init/database.rs:300` ⇒ worker 里那份 sqlite 与宿主库**不是同一连接**，`vec0` 虚拟表无法跨进程共享；向量必须与 chunk 同库同事务（删 source 要级联清向量） |
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
| K1：`pub trait` 总数（8 根 2009 个 `.rs`） | **317** | 含引擎内部变化点、测试替身、serde 派生面。**2026-11-01 两档各降 1**：`#30` 删除 `harness/src/dojo_extension.rs` 整文件（−460 行），同时少 1 个 `.rs` 与 1 个 `pub trait DojoExtension`。**2026-11-02 文件数升 1（2004→2005）**：`#49` 新增命令帧 e2e 桁块 `axagent-plugins/crates/plugins/src/bin/e2e_command_frame_worker.rs`；同批另一份 `crates/plugins/tests/e2e_command_frame_test.rs` 落在 `tests/`，被扫描面的 `K1_IGNORE` 排除 ⇒ **加 2 个 `.rs` 只升 1 个读数**（这一条是口径事实，不是数据漂移）。权威读数 = `node scripts/check-capability-register.mjs --dump-readings` 的 `k1.rsFiles / k1.pubTraits`。**2026-11-02 收口重测：2009 / 317**（对 HEAD 2004 / 315 的净增溯源：`.rs` = +7 新增〔harness `vector_search.rs`、`vector_store.rs` + 5 个 `plugins/src/bin/e2e_*_worker.rs`〕−2 删除〔两份 `ir_renderer.rs`〕；`pub trait` 唯一名 = +3〔`VectorCollectionAdmin` / `VectorWriter` / `VectorReader`〕−2〔`IrRenderer` / `KitSlashCommandProcessor` 随文件移除〕⇒ 净 +1；**`#62` 再净 +1（316→317）**：ToT 侧 `LlmReasoningProvider` 改名 `TotReasoningProvider`（+1 唯一名），`ProviderRepository` / `SettingsRepository` 两处重复定义并回 1 处（原始命中 −2，唯一名数不变））。 |
| K1 + 有非 harness、非测试实现者 | **233** | ⚠ 这一档**不能**当能力数。~~232 出自一次性 census 工件（`.tmp/traits-full.json`，未入库 ⇒ 谓词无法从仓内代码复现）~~ ⇒ **该说法已作废（`#61` + `#66` 落地后）**：谓词已入库为 `k1DeclaredExternalImplTraitCount`，`--dump-readings` 在现树复现 = **233**（`#62` 后：ToT 拆成两名、两端都仍有生产 impl ⇒ +1），且与本门 audit 侧输出**排序名单逐字相等**（`#66` 的 4.2 档，差分探针 + 两条命令背靠背 + 扫描面常量逐项对照，三条同源性证据齐）⇒ **233 现在是可复算值**，锚点**具备升 `hard:true` 的条件**（升不升、用哪个数由我在收口时一次定）。另两个历史数**只留作解释、不再与 233 并列**：**210** = 单趟扫描缺陷（当时同谓词**两趟交并** = 232，差值恰 **22**；`#62` 后未重测单趟读数）；**269** = 门里最严格的读法（不要求 trait 声明面、不排 std/派生噪声名），**不作文档该锚点值**。⚠ 处置次序仍不能反：**先让谓词可复算，再升 hard** —— 反向必先假红 |
| K1 + K2（键控站点，四次扫描合并去漏报） | **51** 个 trait 面 | 含 15 条已建接缝的 trait。⚠ **2026-11-02 `#33` 实测：这一档不可作论据** —— 谓词真身是**未入库的一次性工件** `.tmp/capability-register3.cjs:44-82`（仓内无处复跑，与 §0.7.2 的 232 同病根），且它要求签名/块内出现 `dyn T` ⇒ 198 条里 **21 条假阴性**，把 PLAN B/C 组落入该桶的 **13 条契约全部吞掉** ⇒ **不许**用「无键控点」论证「这些 trait 只是分层解耦」。谓词入库 = 任务 **#57**；逐条分桶结论见 **§14** |
| 51 → 同族合并 + K3/K4 判定 | **23 条能力**（另有 3 条移入 §0.7.9 债档，只登记不计数） | ⇒ 本节表格 A+B+C 三组 |
| （对照）今天就能远程跑的接缝 | **13** | `axagent-plugins/crates/plugins/src/worker.rs:133` `SUPPORTED_REMOTE_SEAMS` |

> 报数纪律：以后凡是「能力数」，必须连带给出它用的是哪一档口径。单说「26」「27」「12」而不说口径，三者同样不可核对。

### 0.7.3 A 组：接缝已建，机制可用 —— 11 条

| 能力 ID | 契约（定义行） | 键控分发点（证据行） | 内置实现者 | 拆分阻塞 |
|---|---|---|---|---|
| `model.provider` | `axagent-core/crates/harness/src/provider.rs:25` | `axagent-ai/crates/providers/src/registry.rs:24`（`HashMap<String, Arc<dyn ProviderAdapter>>`）+ `:33 register` + `:44 get` | 14 | **D2** 流式：`chat_stream`/`speech_stream` 首帧即错 |
| `tool.set` | `axagent-core/crates/harness/src/registry.rs:130`（契约面 `harness/src/tool.rs:424 Tool`） | `axagent-runtime/crates/tools/src/registry.rs:183` | Tool impl **199** | **D4** `ToolContext` 被丢弃（安全，必须先修） |
| `platform.adapter` | `axagent-core/crates/harness/src/message_adapter.rs:83` + `channel_adapter.rs:25` | `axagent-runtime/crates/rt-messaging/src/message_gateway/platform_manager.rs:16` + `:104 get_adapter(&str)` | 10 | — |
| `message.callback` | `axagent-core/crates/harness/src/platform_callback.rs:21` | 接缝 `message.callback`（内置与插件同入口） | 2 | — |
| `webhook.dispatch` | `axagent-core/crates/harness/src/webhook_subscription.rs:154` | 接缝 + `axagent-infra/crates/rt-webhook/src/webhook_dispatcher.rs:9` | 4 | — |
| `system.prompt` | `axagent-core/crates/harness/src/context_contributor.rs:22` | 接缝 `system.prompt.{段名}` | 3 | — |
| `workflow.reflector` | `axagent-core/crates/harness/src/workflow_reflection.rs:190` | 接缝 | 3 | **D2**：`axagent-plugins/crates/plugins/src/worker.rs:2382` 起 `remote_method_not_supported` |
| `workflow.evolver` | `axagent-core/crates/harness/src/workflow_evolution.rs:172` | 接缝 | 3 | 同上（`axagent-plugins/crates/plugins/src/worker.rs:2474`–`:2522` 一批 `remote_method_not_supported`） |
| `workflow.optimizer` | `axagent-core/crates/harness/src/workflow_optimization.rs:80` | 接缝 | 2 | 同上 |
| `workflow.sandbox` | `axagent-core/crates/harness/src/workflow_evolution.rs:263` | 接缝 | 4 | — |
| `workflow.business_rule` | `axagent-core/crates/harness/src/business_rules.rs:71` | 接缝（已有 e2e worker 产物） | 3 | — |

### 0.7.4 B 组：K1+K2 成立但**没有接缝** —— 8 条，P5.0a 必须新建（另 2 条 `workflow.node` / `memory.provider` 已按 #54 移入 §0.7.9 债档；本表仍保留原行作判据与历史，不计数）

| 建议能力 ID | 契约 | 键控分发点 | 内置实现者 | K4 代价（帧协议/主权） |
|---|---|---|---|---|
| `notify.channel` | `axagent-core/crates/harness/src/notification_channel.rs:195` | `axagent-runtime/crates/notification/src/dispatcher.rs:114` + `:143 register_channel` + `:149 unregister(&str)` | 6 | **最低**：DTO 全可序列化；去重/冷却留宿主 |
| `market.vendor` | `axagent-quant/crates/astock-data/src/vendors/mod.rs:7 StockVendor` | `astock-data/src/lib.rs:863 register_vendor(&str)` + `:1672` 键控取回 | 16 | `AS_OF` task-local 必须改显式入参；门面 `MarketDataProvider`（`harness/src/market_data.rs:192`）不单列 |
| `search.source` | `axagent-ai/crates/agent/src/search_provider.rs:95` | `axagent-ai/crates/agent/src/search_orchestrator.rs:28`（`HashMap<SourceType, Arc<dyn SearchProvider>>`） | 4 | `rate_limit()`/计费属宿主。**2026-11-02 实测分档**：`search()` 侧结果 400–600 B × 默认 `max_results` 10 ⇒ **①，可直接走接缝**；`extract_content(url)` 返回大文本（今天 50 KB 截断）⇒ **②**，需**分页/游标**而非分块。DTO（`search_provider.rs:38-57`、`research_state.rs:416,76`）全 serde✓。前置 **D1** |
| `image.gen` | `axagent-ai/crates/providers/src/image_gen.rs:64` | `name()` 自述键 + providers 注册面 | 2 | **2026-11-02 实测**：`url` 形态 ≈200 B ⇒ **①**；`base64` 形态 **1.3–4 MB × n** ⇒ ②（接近 `MAX_FRAME_LEN = 64 MiB` 的 `plugin-proto/src/lib.rs` 的 `pub const MAX_FRAME_LEN`，但 n 张累加会顶穿）⇒ 裁定取「只回 url / 宿主侧落盘引用」形态，**保 b64 才需要 D2 分块**。DTO（`image_gen.rs:24-36,45-54`）全 serde✓；provider 凭证 scope 留宿主 |
| `document.parser` | `axagent-core/crates/harness/src/search_sources.rs:62` | **证据行改定（§14.4 + §24.2 两次裁定；本行此前仍是旧行）**：`axagent-infra/crates/document-parser/src/lib.rs:533` 只是 ext→MIME 的**字符串映射表**、块内**不构造实现类型** ⇒ 真正的行为选择点是同文件 `:61` 的 `match mime_type`，而该块**同样无构造分支** ⇒ **K2 的 ⑧（factory-match）在结构上永不命中 = 已知漏检族第 4 例**（该形态「按字符串 `match` 选行为、但不构造实现类型」已补进镜像表 `countRule.k2Caveat`）。⇒ **本行准入判据改写为：K1 成立（外部实现者 1）+ K3 成立；不以 K2 命中为准**（`#54` 已裁「检测器跟着登记表走」是禁止方向，见 §24.2 第 1 条） | 1 | 37 分支是**编译期穷举** ⇒ K3 今天要改主壳，正是本条要拆的理由；另有两套 `OnceLock` 全局（`search/src/sources.rs:22`、`tools/src/parser.rs:13`）和一处硬编码绕过（`commands/knowledge.rs:1169`） |
| `rag.source` | `axagent-core/crates/harness/src/rag_provider.rs:7/40`（Embedding/Rerank）+ `axagent-infra/crates/search/src/rag.rs:45 RAGSource` | `search/src/rag.rs:941`（3 分支工厂） | 4 | **§0.6.3 未拍**；`VectorStore` 是持 `DatabaseConnection` 的 struct（`search/src/vector_store.rs:114`），sqlite-vec 进程内扩展 ⇒ 出进程即失效 |
| `mcp.server` | `axagent-core/crates/harness/src/mcp_service.rs:54 McpClientService` + `:48 McpServerStore` | `axagent-ai/crates/mcp/src/client_service_impl.rs:155 build_*` | 1 | **订正（2026-11-02 实测）**：句柄**不过帧** ⇒ 门面 `McpClientService` 是零字段单元结构（`client_service_impl.rs:21-22`），连接池在实现侧进程内全局（`mcp_client.rs:661` / `:434` / `:2407`）；桥今天即 one-shot（`mcp_tool_bridge.rs:197-202`）⇒ **可直接走请求-响应接缝**，「会话式 op」是**延迟优化而非 K4 前置**。需另补两条生命周期 op（`evict_by_server_id` `mcp_client.rs:547`、`probe_and_evict` `mcp_health.rs:100`）⇒ 可与 `notify.channel` 同批。入参 300–800 B、每工具 500 B–2 KB ⇒ ①；`callTool`/`readResource` 返回值**无上界** ⇒ ②。拟接缝 id = `mcp.client`（`McpServerStore` 留内核：配置面不是能力面）。前置 **D1** |
| `strategy.quant` | `axagent-core/crates/harness/src/strategy_contract.rs:440` | `src-tauri/src/commands/quant_backtest.rs:363-430`（`match strategy_type` 硬构造） | 7 | **最高**：`&mut self` + `&mut StrategyCtx` 的 bar 级热循环 ⇒ 会话式批量 op（spawn/setParam/onBarBatch/drop） |
| `workflow.node` | `axagent-runtime/crates/rt-workflow/src/work_engine/node_executor_trait.rs:224` | `work_engine/dispatcher.rs:31`（`HashMap<&'static str, Arc<dyn NodeExecutorTrait>>`）+ `:397 get_executor(&str)` | 33 | 键是 `'static` 字面量 ⇒ K3 今天不成立；**粒度必须是「节点包」而非单节点**，否则每个节点一次 IPC。**2026-11-02 K4 实测判不过**：`ExecutionState`（`execution_state.rs:213`）含 **≥8 个 skip 句柄位** ⇒ ③；node 入参 0.5–3 KB 但 context 10 KB–数 MB ⇒ 需 snapshot 化才勉强到 ②（snapshot 形态 `:491` **已存在**）⇒ 移入内核桶属 #54，本条真正的归属是 **P6.0-①（节点包协议批次④，与 `strategy.quant` 共用 spawn/feed/close）**，**不入 P5.0a**；**2026-11-02 `#54` 已移入 §0.7.9 债档（不计数）** |
| `memory.provider` | `axagent-ai/crates/trajectory/src/memory_provider.rs:63` | **自述键 ≠ 分发点（本行的原病，§14.6 + §24.1 已裁）**：`provider_name()/provider_version()` 只是实现者的自述字段，仓内**没有**按它取实现的容器 —— `MemoryProviderRegistry` **不存在**（全树 2 处命中都是「它不存在」的注释），`Arc<dyn MemoryProvider` 的 4 处位点**全是单字段 / 函数参数**、无键控集合 ⇒ **「建接缝」= 建接缝 + 建键表**（成本 6+1）。它**留 B** 的依据是 **K1 + 第三方贡献故事**（§14.6），不是「键控位点已存在」；**2026-11-02 `#54` 已移入 §0.7.9 债档（不计数）** | 2 | `sync_turn(entries)` 可序列化；预取语义要与会话生命周期对齐 |

### 0.7.5 C 组：钩子/会话族，判可拆但先验 K4 —— 4 条（⚠ K4 实测订正 2026-11-02：`transport.kind` 体量**全 ①**，原判「必须留内核」在盘上**不成立**；`rule.custom` **K4 通过**但全仓无生产注册点 ⇒ 待第一个实现者；`context.engine` **判债**（零实现者 + 零派发点）⇒ 已按 #54 移入 §0.7.9 债档，本表保留原行但不计数）

| 能力 ID | 契约 | 证据 | 判定理由 / 待验 |
|---|---|---|---|
| `lifecycle.hook` | `axagent-core/crates/harness/src/workflow_lifecycle.rs:33` | 4 实现者；文档自述「业务侧实现 + 注册，WorkEngine 按模板声明调用」 | 天然第三方面。实测（2026-11-02）：`HookExecContext`（`:51`）与 `HookOutcome`（`:64`）**无 serde** ⇒ 今天属 **③**，补 derive 后即 ②/①；`postExec.results` 是全 DAG 输出 ⇒ 需引用化（ctx 20 KB–400 KB 为推断）。频率每次工作流执行 1+1 次（低频）⇒ 可走接缝，前置 **D1** |
| `plugin.hook` | `axagent-core/crates/harness/src/plugin_hook.rs:159` | 14 方法（`pre_tool_call`/`post_tool_call`/`on_session_*`） | **与 D4 无关（2026-11-02 订正）**：`ToolCallContext` 的真实位置是 `plugin_hook.rs:14-19`，**本就是 4 字段纯 serde DTO**；D4 丢弃的是宿主可执行上下文 `ToolContext`（`harness/src/tool.rs:142`；**tools 侧只是 re-export 同一份，不是第二份定义** ⇒ 它与 `ToolCallContext` 是**近名不同物**，不构成铁律 4 违规，`#50` 实测订正）⇒ **不需要等白名单投影改造**。真阻塞是**频率**（每工具调用 + 每出站请求 ⇒ 一轮数十~数百次）与守卫语义外置 ⇒ 一格拆三：`on_session_*` 今天即可直连、`pre_*` 留内核待 **D1 + 权限模型**、`post_*`/`transform_*`（体量 = 工具输出/HTTP body，②）需**批量帧**（「一次帧多条事件」即可，不强制绑 D2 的 token 流语义）。DTO 面 `:14-156` 已全 serde✓ |
| `context.engine` | `axagent-core/crates/harness/src/context_engine.rs:24` | `compact()/name()/supports(&Session)` | 压缩策略可拆（LLM 摘要 / 记忆 / 混合）；`supports(&Session)` 是借用入参 ⇒ 需投影成 DTO。**2026-11-02 判债**：0 实现者 + **0 派发点**（全仓扫无 `dyn ContextEngine`，生产压缩走 `compact_session` 自由函数 `harness/compact_session.rs:67`，被 `agent/session_manager.rs:781` 直调）+ 签名与在用路径不符（`Session` 仅 `Serialize`、无 `Deserialize` `session.rs:51`）⇒ 连 K2 都不成立 ⇒ **不进待建数**，P5.0a **不许为它预留协议面**。体量 ≈0.4–0.7 MB/次（80 000 token×4 B 反推）、每会话 1 次；**2026-11-02 `#54` 已移入 §0.7.9 债档（不计数）** |
| `rule.custom` | `axagent-ai/crates/trajectory/src/constitution.rs:102` | `:118 HashMap<String, Arc<dyn CustomRuleChecker>>` | 自我改进宪法的自定义校验；与 `workflow.business_rule` 不同域，不并条。**2026-11-02 实测：K4 完全成立**（① + 低频 + 纯 DTO，**无 D1/D2 依赖**，2–80 KB 一次），但缺口不在帧而在「没人实现、没人注册」：门口径实现者 0，内核消费逻辑是活的（`ImmutableConstitution.custom_registry` `:158`、`with_custom_registry` `:170`、三个调用点）但外部调用 0 ⇒ 登记为「**K4 通过 · 待第一个实现者再建接缝**」，与 `context.engine`（K4 也不过）**分档处置** |
| `transport.kind` | `axagent-runtime/crates/rt-messaging/src/message_gateway.rs:168` | `:164 HashMap<TransportType, Arc<dyn TransportHandler>>`，4 实现者 | ✅ **原判据在盘上不成立（2026-11-02 实测）**：体量**全 ①** ⇒ `AgentEndpoint`（`:143-152`）只缺 serde derive（字段类型全 serde-able，补 derive 即 ①）、`MediaAttachment`（`harness/src/message_adapter.rs:67-73`）**只带 `path`**（传宿主侧文件引用，不是字节流）⇒ 无 4× 膨胀、无分块需求。连接生命周期归实现侧、出进程自洽（同 `mcp.server`），`get_state()` 同步方法远程化即请求-响应。代价换成另一类约束：**worker 须与宿主同文件视图**（与 `document.parser` 同一病灶，不是新病灶）。⚠ 真缺口是**今天无入站消费路径**：`transport_handlers.rs:155-171` 的 `Message::Text` 落 `_ => {}` 丢弃 ⇒ 将来补入站才需要 `ops::EMIT`（`plugin-proto/src/lib.rs` 的 `pub const EMIT`）扩展。前置 **D1**；优先级低于 `notify.channel` |

### 0.7.6 机制合流，不是能力 —— 1 条（必须处理，但不计入 23）

**已执行（2026-11-01，`#30`）**：`harness/src/dojo_extension.rs` 整文件删除（**−460 行**），`harness/src/lib.rs:332-337` 换成 4 行墓碑注释（337 是 `pub use` 收尾 `};`，按 332–336 删必编译不过）。删除门禁：8 个 `DojoExtension*` 符号在两文件之外命中 0（`.tmp/t30-dojo-gate.mjs`，20441 文件面；排除 `.tmp/`、`output/`、`docs/`、`.workbuddy/`）；翻案检索 `git log --all -S"impl DojoExtension for"` = 4 提交，`git grep` 到 `origin/master`/`upstream/master` 的 18 处命中全落在被删文件自身 ⇒ **历史与其它分支均无生产实现者，翻案条件未触发**。
它原来「是一套与 capability 接缝并行的第二插件协议」这一判据不变：自带键控注册表、自己声明工具与命令、自己带健康探针，却不在 15 条接缝里，也不在 `SUPPORTED_REMOTE_SEAMS`。
⇒ **已裁定并执行（§0.12.1）：废弃 trait 本体，把 `command_specs/execute_command` 的形状吸收进 L1 命令通道 T2**（生产实现者 0，唯一实现是测试替身）——不给零实现者的第二协议上「权限模型 + 对账 + 兼容窗口」三套成本。
   原记「CLI 派发口无 `input_schema` 且权限写死 `ReadOnly`」**判据低估了实情**：`PluginTool::check_permission`（`plugins/src/types.rs:787-826`）全仓零调用方，唯一引用它的 `execute_with_permission_check` 也零调用方 ⇒ 档位在 tools 与 commands **两条路上都是纯装饰**；本批只把命令通道改成真授权（按具体权限名判定，`implies_*` 一字未改），**tool 侧依旧是装饰**，另立批次。
   同批已修的假话：`manager.rs` 原注释自述「按空白切分为程序 + 参数」是假的（命令串整团交 `Command::new`、`args` 恒空，可执行文件查找按宿主 cwd ⇒ **相对路径声明的 command 今天根本跑不通**，见任务 #47）。CLI 派发口现位于 **`plugins/src/manager.rs:738`**（原 `:708`；#75 加 `list_plugin_pages` 后再下移 30 行；镜像表同批改）。

### 0.7.7 判为内核，明确不拆 —— 10 条（每条都给不过哪道判据）

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
| `rag.vector.store`（预留接缝位，`axagent-core/crates/harness/src/vector_store.rs:12` 模块头） | K3 / K4 待兑现 | 本轮只占位不实现：方言无关三 trait（`VectorCollectionAdmin` / `VectorWriter` / `VectorReader`）已就位，但宿主注入点仍是 `Arc<VectorStore>` 具体类型 ⇒ 不计能力面、不进 P5 批次 |

### 0.7.8 这版登记表推翻 / 锁死了什么

- **锁死（后被 §0.9.4 修正）**：能力数曾锁为 **26**（A 11 + B 10 + C 5；**2026-11-02 `#54` 级联后 = 23（A 11 + B 8 + C 4）**，三条移入 §0.7.9 债档）。C 组里 `context.engine` 已移债档、`rule.custom` 经核为**零外部实现者**（我引错了证据等级）⇒ 有实证的提供方接缝为 **22**，权威口径见 §0.9.4。
- **`domain.pack`（域包 / OPC 能力包）不登记为 L2 接缝，理由 = 层级归属（L1 功能包 ≠ L2 提供方接缝），不是「已从产品移除」**：本行原写「本轮剔除 `domain.pack`……用户口径为『域包已从产品移除』」——**该措辞由 §24.2 第 2 条推翻并就地改掉**，理由必须是可复述的层级口径而不是产品状态，否则它会成为下一个人手里的假证据。**盘上反证在案**：仍有 14 个顶层 `opc_*` / `capability_pack_*` 命令文件共 **9887 行**（最大 `opc_capability_pack_actions.rs` 3486 行）、`analysis-engine/src/opc/` **26 个模块**、`company-runtime` **2064 行**，且 `harness/src/capability_pack_orchestration/mod.rs:38` 的 `CapabilityPackAdapter` **仍被注册**（3 个实现者 + `mod.rs:97` 按 `domain_pack_id` **键控 get** ⇒ 它活着，只是不归 L2）。⇒ 它现在不是插件，是**一条待迁出的 L1 功能包**（P6.2 / §0.10.5：原「删除」口径已作废，OPC 与股票同属插件化刚需）。
- **推翻**：§0.5.3 末尾那句按名字列举的 12 条是手工挑选，已作废（正文保留划线注，防止又有人引用它）。
- **新增工作量**：需新建接缝 **11 条**（B 8 + C 3 有实证，另有 1 条待证 ⇒ 上限 12），按 §3 反推的 6 处固定成本 ⇒ **约 78–90 处改动点**；
  另加 §0.7.6 的 Dojo 协议合流一项。**§3 的 DTO 分级只覆盖了 7 条能力，12 条待建接缝（一条待证成立的上限；主口径 11 条）里还有 7 条未做 K4 分级**（§0.7.4 余 3 条 + §0.7.5 的 4 条）⇒ P5.0a 的输入补齐项（任务 #29）。
- **登记但不实现**：第五道门 `scripts/check-capability-register.mjs`（复跑本节扫描、与本表逐行对账，能力数变化必须显式改表）—— **已裁定要建（§0.12.4 / 任务 #38）**。
  同批登记一条**预留接缝位** `rag.vector.store`（§0.12.3 的 3 个方言无关 trait 就是它的 op 面）：本轮**只占位不实现**，不进 P5 批次、不计入 11/12 的待建数。
  接线代价 = 两处同源注册 + 一次 `--selftest`；**这条要点头才做**，不擅自开工。

### 0.7.9 债档：只登记不计数（3 条，2026-11-02 `#54`）

| 能力 ID | 原档 | 裁决 | 盘上判据（摘要；全文见 §24.1） |
|---|---|---|---|
| `workflow.node` | B | 移出计数，留内核 | K3 不成立（`&'static str` 键 + `WorkflowNode` enum 穷举 + `init_builtin` 编译期写死）；实现者 33 中 1 个是 test 块内 `TestExecutor` ⇒ **生产 32**；归属 P6.0-①，不入 P5.0a |
| `memory.provider` | B | 移出计数（建接缝 = 建接缝 + 建键表，但今天无生产消费） | 2 个「外部实现者」均不可达：`CountingProvider` 在 test mod 内、`SkillSummaryProvider` 零构造且自述未接线；`MemoryProviderRegistry` 不存在；`MemoryHookProvider` 5 个构造点全在测试 ⇒ **生产 1**（且零构造） |
| `context.engine` | C | 移出计数（trait 本体判债） | 0 实现者 + 0 派发点；同文件活的工具函数（`generate_skill_injection` / `extract_injected_skills` / `should_reinject` / `CompactionRecord`）**不是这条 trait** ⇒ 判债 ≠ 该文件可删 |

⇒ 三条仍计入 `totalRows`（可盘点），但**不进 `capacityRegister`、不进「待建接缝」数、不进 P5 批次**。能力面 = A 11 + B 8 + C 4 = **23**（`totalRows` = 23 + H 1 + K 10 + 债档 3 = **37**）。

---

## 0.8 功能包层（L1）：工作流编辑器、股票分析今天为什么拆不干净（2026-11-01 端到端实读）

### 0.8.1 先立层级，并撤回我造出来的第二个数

本方案里「能力」只有**一个**可插拔单元定义（§0.5.3：一条接缝族 + DTO + worker 产物）。为避免再翻烧饼，层级钉死如下：

| 层 | 是什么 | 判据 | 与插件的关系 |
|---|---|---|---|
| **L0 内核** | 装载器、帧协议、会话/DB/迁移、凭证、权限模型 | 拆了就没有「宿主」 | ✖ 永不拆 |
| **L1 功能包** | 一个用户可见入口 + 它的页面/store/命令面/crate（工作流编辑器、股票分析在这一层） | 能不能整体关掉而产品不崩 | ⚠ **今天拆不干净**：六项前置未做（§0.8.3 + §0.8.5），不是原理上不可拆 |
| **L2 提供方接缝** | 同一功能的多实现可替换槽 | K1–K4 ⇒ §0.7 的 **23 条**（另有 §0.7.9 债档 3 条不计入） | ✔ 唯一已成立的插件单元 |

**撤回声明**：我曾在本轮给出「L1 20 个功能包 + L2 26 = 46 个可拆单元」。**46 作废**——它把两层并列计数，等于凭空造第二种插件单元，与 §0.5.3 冲突。
L1 的正确身份是**接缝的消费者**，不是「27 里漏掉的一格」。本会话三次改数（12 → 27 → 46）的根因是同一个：
**我拿「我先扫到什么」当「架构是什么」**。此后本 PLAN 内任何计数必须绑死口径档位（§0.7.2 的敏感性表）。

### 0.8.2 实读对象：工作流编辑器（132 个前端文件 / 83 条命令（审计 19 域口径）/ 引擎 7366 行）

| 面 | 事实（全部指到行号） |
|---|---|
| 前端 | `src/pages/WorkflowPage.tsx:23`、`WorkflowMarketplace.tsx:186`；`src/components/workflow/` 132 文件（`WorkflowEditor.tsx` 2660 行、`Nodes/` 39 个）；store `workflowStore.ts` 1202 / `workflowEditorStore.ts` 3127 / `workEngineStore.ts` 542 |
| 命令 | 前端 57 个 `invoke` 名（约 128 处）；Rust 侧 `register_commands.rs` 共 **1543 条 / 164 模块**（口径 = `scripts/lib/command-domains.mjs` 的 `parseRegistry`，与两份 AUDIT 同源），工作流域（审计 19 域口径）**83 条 / 11 模块**（`workflows`=10、`work_engine`=21、`workflow_template`=16、`workflow_reflection`=8、`workflow_ai`=8…；`stock_workflow` 28 条归股票域，不计入本域） |
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
 `src-tauri/src/register_commands.rs:1` 自述「Auto-generated by build.rs — DO NOT EDIT」，1543 条全部静态来自 `commands/mod.rs`。
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
| 提供方接缝（§0.7） | worker 可执行 + DTO | ✔ 受 | 22 条有实证（另 3 条债档） |
| 功能包（§0.8） | 页面 + 命令 + store + crate | ✔ 受，但六个前置未做 | **可拆**，前置未备（见 §0.8.5 改判） |
| **策略数据（本节）** | `workflow_templates` 行、tier 映射、规则表 | ✘ **不受**（它是内容，不是代码） | **已经在用，只是从没被这么称呼** |

### 0.9.4 我的三条判定（写成结论，不再挂「待你拍」）

1. **路由族判 L0 内核，不做 worker 插件。** 两条实证理由 + 一条作废（原第三条是引擎依赖，被我误当成否决判据）：① 0 外部实现者 + 主壳全局持有 ⇒ 它是「决定哪个插件被使用」的东西，放进可替换接缝就等于允许插件操纵全量流量，与 §0.6.2 权限模型正面冲突；
   ② 每轮对话热路径，出进程即每轮一次 IPC。③ ~~执行依赖进程内 `WorkEngine`~~ **作废（§0.8.5：依赖方向是「域 → 引擎」，引擎不认识域）** ⇒ 本判定只剩两条实证，但结论不变：路由是「决定哪个插件被使用」的**裁决位**，裁决位归主壳，与它是否耦合引擎无关。
   其中 `RouteEngine`（唯一实现名 `NoopRouteEngine`）、`RouterSelfUpdateManager`、`WorkflowGraphRouter`（后两个**无任何实现**）是**空接口**，属债不属能力。
2. **§0.7 的 C 组有 2 条证据不足，降级待证**：`context.engine`（`harness/src/context_engine.rs:24`）与 `rule.custom`（`trajectory/src/constitution.rs:102`）
   外部实现者 = **0** —— 我当初引的是「trait 自带 `name()` 方法」名单，不是实现者证据，那是证据等级不够。⇒ **有实证的提供方接缝 = 22 条（A 11 + B 8 + C 3）**，待建接缝 11 条（若一条待证成立则 12）。
   同批的 `DojoExtension` 也是 0 外部实现者 ⇒ 它「没人实现」不是因为不重要，是因为它是**第二协议**（§0.7.6）。
3. **候选集定义从此改为两层**，本 PLAN 的权威计数以本节为准：
   **提供方接缝 24（+2 待证）· 策略位 79 个 trait 待逐条判（路由族 9 个已判内核）· 功能包 20 个但需 P6.0 前置**。
   任何单层数字再被当成「能力全集」报出，都是错的——我此前 12 / 27 / 46 三次翻烧饼的共同根因就是这条。

### 0.9.5 功能点全集已一次扫全并独立成文（此后不再逐点补）

逐条清点落在 **`AUDIT-capability-inventory-2026-11-01.md`**，由 `node scripts/audit-capability-inventory.mjs` 直接生成（改判据即改脚本，文档可重生成，不手抄）。
骨架 = build.rs 生成的 Tauri 命令注册表，与前端 invoke 消费面、`commands/**` 代码行、后端 `axagent_*` 引用、全仓 `pub trait` 实现者分布、壳层 `Arc<dyn T>` 持有面**五路交叉**。闭合读数：

| 面 | 闭合读数 |
|---|---|
| Tauri 命令 / 命令模块 | **1543 条 / 164 个模块**，100% 归入功能域，无「其它」残桶 |
| 功能域 | **19 个**：4 个判 **L0 公共底座**（智能体引擎与编排 15 · 对话与消息 11 · 设置/安全/外壳 10 · 工作流引擎与编辑器 10；括号为跨面共享消费方数），15 个判 L1 功能包 |
| `pub trait` | **316**：L2 已建接缝 **14** · L2 待建接缝 **19** · **有外部实现者但无键控分发点 198** · L3 策略位 **56**（壳层全局持有 4 + 仅内置实现 52） · **全仓零实现的空接口 29** |

⇒ 「项目里已有功能点非常多」现在有数：**19 域 / 164 模块 / 1543 命令 / 315 trait**（命令数 2026-11-01 增 1 = 新增 `plugin_resolve_page_asset`；2026-11-02 再增 1 = 新增 `plugin_list_pages`（#75）；trait 数减 1 = 已删的 `DojoExtension`；两个旧读数按门禁规矩只写在 §0.12.4 的漂移列举段，不在这里复述；口径同 §0.7.2）。
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

**判据链**：B 存储面不能进插件（口径 2 + `vector_store.rs:114/:127` 直接吃宿主 `DatabaseConnection` + sqlite-vec 是进程内扩展、唯一注册点 `init/database.rs:300` + 向量必须与 chunk 同库同事务）；
解 2 要求产品接受外部服务依赖，与桌面单机定位相悖 ⇒ 在**已拍口径**之下，解 1 是唯一自洽解，不是偏好。
**但不锁死后路**：`vector_store.rs:149-152` 的双后端运行期分叉（SQLite `vec0`+FTS5 / PostgreSQL `VECTOR(n)`+`tsvector` GIN）**已经在代码里**，还有 `search/tests/pg_integration.rs`
 ⇒ 将来若要引入服务端形态，解 2 的架构成本≈0，无需为它提前改接缝。
**随附前置**（本轮实测新发现）：抽象契约 `pub trait VectorSearch` 原定义在 **consumer crate** `agent/src/query_engine.rs:46`，
而插件 SDK 不含 agent ⇒ 第三方够不着它。要贡献向量后端，**必须先把它下沉到 `harness`**（任务 #37；**已交付 2026-11-02**，全过程与出入见 §22）。
⇒ §0.6.3 末尾「需要单独拍一次」这句话现在可以关掉。

**① 预留接口（2026-11-01 追加口径：暂不上服务端，但把口子留好，后续有需求再兑现）**

实测 `VectorStore` 有 ~~**27 个公开方法**~~ ⇒ **交付时复核为 25 个**（`#37/#40` 报告 §2 逐条点名；差 2 = 构造子 `new(db: DatabaseConnection)` + 自由函数 `register_sqlite_vec_extension()`，**两者都不属 `VectorStore` 的方法面**，且 `new` 吃 sea-orm 类型**按分层铁律不得进 harness**），而 agent 侧原有的 `trait VectorSearch`（`agent/src/query_engine.rs:46`）**只有 1 个 `search`** ⇒
**拿它当预留口子是假的** —— 写侧与集合生命周期完全没盖住，将来换后端时调用方照样得改（这句仍然成立，现已被 3 个 trait 盖住）。

| 预留 trait（权威定义已落 `axagent-core/crates/harness/src/vector_store.rs`） | 覆盖的宿主方法（**行号 = 交付后的 `axagent-infra/crates/search/src/vector_store.rs`**，宿主 inherent 定义位） | 职责 |
|---|---|---|
| `VectorCollectionAdmin` | **7 个（已落，与下表逐条一致）**：`ensure_collection:438` · `get_collection_dimensions:485` · `prepare_collection_for_indexing:501` · `ensure_collection_hnsw:578` ⇒ trait 面 `ensure_vector_index` · `delete_collection:1123` · `clear_embeddings:1140` · `set_collection_embedding_model:1334` | 集合与索引生命周期 |
| `VectorWriter` | **9 个（已落）**：`upsert_embeddings:627` · `add_single_chunk:774` · `delete_document_embeddings:990` · `insert_metadata_only_chunk:1018` · `reinsert_embeddings:1251` · `upsert_document_embeddings:1261` · `delete_chunk:1580` · `update_chunk_content:1640` · `update_chunk_embedding:1667`。⚠ 原表第 5 项「`list_all_metadata_rows` **之外**全部写入类」是**排除条款、不是第 10 条** ⇒ 该方法落 Reader | 写入与删除 |
| `VectorReader` | ~~**11 个**~~ ⇒ **9 个（已落；原表的 11 是 27 那个错数的下游，盘上不存在另外 2 条实体）**：`search:879` · `search_with_filter:893` · `list_all_chunks:1192` · `list_document_chunks_raw:1198` · `ensure_fts5_index:1362` ⇒ trait 面 `ensure_keyword_index` · `rebuild_fts_index:1444` · `fts_search:1479` · `list_document_chunks:1796` · `list_all_metadata_rows:1089` | 查询与列举 |

⚠ **命名已去方言（交付时落定，订正本表原写法）**：trait 面 = `ensure_keyword_index` / `ensure_vector_index`（snake_case）——原表写的 `ensureKeywordIndex` / `ensureVectorIndex` 是 **camelCase，不合本仓 Rust 惯例**（rustfmt/clippy 会报 `non_snake_case`），交付按仓内惯例落。
 ⚠ 且撞名比原表预想的多一处：`search` 侧原有一个**私有** PG 专用 helper 就叫 `ensure_vector_index` ⇒ 为避让改叫 `ensure_pg_vector_index`（1 定义 + 4 调用点，纯标识符）。**宿主 inherent 名 `ensure_fts5_index` / `ensure_collection_hnsw` 逐字保留**，改名只发生在 trait 面与 impl 桥接 ⇒ 既有调用方零改动；否则「预留」会把后端绑死在 vec0/FTS5 上，等于没留。
⚠ **K4 预算**：向量是 ② 类大对象（`&[f32]`，1536 维 ≈ 6 KB/条）⇒ DTO 要么分块，要么**只传宿主侧 chunk 引用**；
 后者更省、也更贴口径 2。

⇒ **本轮只做「接口化 + 占接缝位」，不实现任何外部后端**（接缝 id 预留为 `rag.vector.store`，**已写进 `harness/src/vector_store.rs:12` 模块头文档**；实测 `capability_registry.rs` 命中 **0** ⇒ 第五道门的 `seamDeclared` 判据今天仍判它「未建」，这是对的）。⚠ **但本行原要求「进 §0.7 登记表」至今未执行**（#37/#40 只落了代码注释，我改写本行时一度把这个待办写没了 ⇒ 记回这里）：它应作为**新增一行进 §0.7.7 的 K 组**（占位、留内核）⇒ `totalRows` 36→37、`groupCounts.K` 9→10，而 `capacityRegister` **仍 26**（K 组只登记不计数）；**归 #54 级联一起做**，不单独开表改动。
 **实交付面（订正本表原估）**：3 个 trait + 3 个 DTO 搬家（`EmbeddingRecord`/`HnswConfig` 补 `Serialize/Deserialize` 是 harness「只允许标准库 + serde 可序列化类型」约束所需，additive）+ 5 处 `pub use` 断链防护；
 **刻意未做**：`VectorStore` 的 2 个构造点与全部持有位（tools 的 `Arc<VectorStore>`、wiring 的 AppState）**仍是具体类型** ⇒ 注入 trait 化留给兑现 `rag.vector.store` 的那批（它要动 src-tauri/wiring，与本批「不触主 crate」的窗口约束冲突）。原估 30–40 个改动点 ⇒ 实际集中在 4 个文件，`VectorStore` 的 70 处引用**一处未动**。
⇒ **必须与任务 #37（`VectorSearch` 下沉 `harness`）同批做**，否则 SDK 公开面会被定型两次（禁区 12 的重复定义风险）。

**② asset 协议 / CSP**：**已选 A**（开 `assetProtocol`、CSP 加 `asset:`；`'unsafe-eval'` 单独立项）—— 见 §0.12.5。

**③ P6.1/P6.2 排期**：不是待定项，估工、依赖与推荐序列见 **§6 表下方的「P6 估工与排期」**。

### 0.12.4 第五道门 `check-capability-register.mjs` → **建**

**判据**：今天这轮自查就抓出 **15 处数字漂移 + 2 套并行口径**（1534/1541、工作流 124/83、股票 304/306、前端 60 757/84 873），**2026-11-01 本批再记三对漂移（旧/新）：命令 1541/1542、`.rs` 文件 2005/2004、`pub trait` 316/315**；**2026-11-02 `#75` 再记一对：命令 1542/1543（新增 `plugin_list_pages`）**——本段是这些旧读数的唯一合法出现处，写在别处由第五道门判红。
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
| **D2** | **无流式**：远程 provider 的 `chat_stream` 第一帧即错误 | `worker.rs:2625-2626`（注释自述「帧协议为一请求一响应，无流式分片复用」）、实现 `:2656-2670`；`reflector.aggregate_patterns/get_history`、`evolver.*`、`optimizer.*` 返回 `remote_method_not_supported`（`worker.rs:2382`、`:2474-2522`） | **provider 试点的硬前置**：聊天应用不能失去逐 token 输出 | 协议加流式分片（同一 `request_id` 多帧 + `final` 哨兵），是 `PLAN-everything-is-plugin.md` §15.4 那套扩展的自然延伸；**不做则 provider 不许插件化** |
| **D3** | 两套注册路径且无一致性校验：`manifest.capabilities` 只写 `declarations` 表**不挂 provider**；可调用门面只来自 describe 自述 | `plugins/src/manager.rs:705-713`→`:783-821`；`capability_registry.rs:484-510`；测试名自证 `records_declaration_only_and_rolls_back`（`:1370`）；真正挂门面在 `worker.rs:2262-2279` | 清单声明 ≠ 运行时能力 ⇒ UI 显示「插件提供 X」而运行时拿不到（既有方案缺陷 1 的变体，仍在） | 握手后做 `manifest.capabilities` ↔ `declaration.capabilities` 双向对账：声明未实现 ⇒ 拒绝注册并计入可见失败（不许静默） |
| **D4** | **远程工具丢弃宿主 `ToolContext`** | `worker.rs:624` 构造 `ToolContext::new(".")` 最小上下文；`worker.rs:1881` `RemoteTool::call` 直接忽略 `_ctx` | 插件工具的**宿主权限校验、输出脱敏、审批规则、Guardian、ask-user、回滚栈全部失效**（安全相关，非风格问题） | ctx 走「可序列化白名单投影」出境（策略/开关/IDs），宿主侧保留句柄类对象（`Arc<Notify>`、审批栈）不下发；需复用反向 `call_seam` / `UI_ACTION` 通道做 ask-user 与审批回环 |
| **D5** | worker 启动失败**只 warn**，插件仍显示「已启用」 | `plugins/src/manager.rs:722-730` | 「启用了却一个远程能力都没有」只能靠翻日志 | 启用结果区分三态：A 层资产可用 / B 层 worker 可用 / B 层失败（可见状态 + 前端提示） |
| **D6** | 信任链只有 sha256 完整性，**无签名** | `plugins/src/types.rs:880-887`；`PLAN-plugin-gap-closure.md` 与 gap-closure 调研一致结论「签名机制：未找到」 | 独立交付产物变成供应链目标；`source_audit.rs:5-8` 已自述「形式合规检查，非安全边界」 | 短期：产物来源与 hash 钉死 + 安装时人工确认；长期：manifest 签名（口径待定，不阻塞 P5.2/P5.3） |

**优先级判断**：D4 是安全缺陷（现在就该修，与插件化进度无关）；D1/D5 是「拔插体验」的地基；
D2 只在 provider 试点前才需要；D3 在任何一个插件真正投放前必须修，否则声明与实现会分叉。

---

## 3. 逐能力接缝补齐清单（量化「新增一条 seam 的代价」）

> ⚠ **本表只覆盖 §0.7 的 23 条能力里的 7 条**（`model.provider` / `tool.set` 两条已有接缝，加 notify / doc-parser / market / strategy / RAG 五条待建）。
> **12 条待建新接缝（主口径 11 条，此为一条待证成立时的上限）里还有 8 条没做 K4 分级**：§0.7.4 余下 3 条（`search.source` / `image.gen` / `mcp.server`）
> + §0.7.5 全部 5 条钩子族。分级没做完就不许签协议版本号 ⇒ 它们是 P5.0a 的**输入缺口**（任务 #29）。

DTO 障碍分级：**①** 纯 serde DTO 可直接过帧；**②** 可序列化但含大对象/字节流；**③** 不可序列化（借用 / `Arc` / 闭包 / DB 类型 / `&mut`）。

| 能力 | 现有 trait | 接缝 | 实现者 | 主要障碍 | 建议接缝 ID | 难度 |
|---|---|---|---|---|---|---|
| 通知渠道 | `harness/src/notification_channel.rs:195` | ✘ 无 | 6（`LogChannel` / ServerChan / PushPlus / Ntfy / Gotify / 宿主包装的 `PlatformNotificationChannel`） | `name()/display_name()` 返回借用 ⇒ 照 `tool.set` 补一个 `list` 发现面 | `notification.channel.{渠道名}` | **最低**（DTO 全 ①，去重/冷却属宿主分发器，渠道自身只读 config） |
| 文档解析 | `search_sources.rs:62` | ✘ 无 | 1（`DefaultDocumentParser`，零字段单元结构） | 按绝对路径读盘 ⇒ worker 须与宿主同文件视图，否则要走字节流分块（PDF/EML 属 ②）；另有两套同名 `OnceLock` 全局（`search/src/sources.rs:22`、`tools/src/parser.rs:13`）和一处硬编码绕过（`commands/knowledge.rs:1169` 直调具体函数） | `document.parser.{格式族}` | 低（唯一无状态、纯进出的能力） |
| LLM provider | `ProviderAdapter` | ✅ `model.provider.{类型}` | 内置 14 家 | **D2 流式**（`chat_stream` 首帧即错）；`speech_stream` 同类 | 已有 | 中（机制全在，卡在协议） |
| 行情数据源 | `market_data.rs:192` | ✘ 无 | 1 生产（`AStockClient`，`astock-data/src/lib.rs:6667`）+ 2 测试替身 | `astock-data/src/as_of.rs:36` 的 `tokio::task_local! AS_OF` 是**隐式上下文**，跨进程必丢 ⇒ 必须改成显式入参 `asOfDate`；客户端自带 Moka L1 / `DiskCache` L2 / health tracker / token 热更新 ⇒ 缓存与凭证主权要选边 | `market.provider.{源名}` | 中高（签名可原样走，状态主权难） |
| 量化策略 | `strategy_contract.rs:440` | ✘ 无 | 7（MA/ MACD/RSI/BOLL/Turtle + `RhaiStrategy` + 仿真替身） | `&mut self` + `&mut StrategyCtx` 的 **bar 级热循环**不适合 request/response；`StrategyCtx` 含 `bar_history`/`trades`/`equity_curve` ⇒ 每 bar 全量传输直接爆帧；消费者用工厂闭包（`param_optimizer.rs:72`，③）；主壳 `commands/quant_backtest.rs:363-430` 用 `match strategy_type` + `name.contains("rsi")` 硬构造 | `strategy.{策略名}` | **最高**（需会话式批量 op：spawn / setParam / onBarBatch / drop + ctx 差分） |
| RAG 向量库 | `rag_provider.rs:21` | ✘ 无 | **生产实现未找到**（仅 `NoopVectorStoreProvider`，`test_support.rs:1079`） | `VectorQueryResult:13` **无 serde**；`&[f32]` 大向量；真实后端 `search/src/vector_store.rs:114 VectorStore{db: DatabaseConnection}` 绑 DB 且以 `Arc<VectorStore>` 直注（`state.rs:72`、`capability_retriever_impl.rs:57`、`knowledge_integration.rs:168`），sqlite-vec 是**进程内扩展** ⇒ 出进程即失效 | `rag.vector_store.{后端名}` | 高（trait 目前是空壳；先补 DTO derive 再谈远程化） |
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
| **P5.9** | **其余 11 条接缝收尾**（§0.7.4 未进 P5.2–P5.8 的：`search.source` / `image.gen` / `mcp.server` + §0.7.5 的 4 条钩子族），另含已建接缝但卡 D2 的反思/进化/优化器（`worker.rs:2382`、`:2474-2522` 那批 `remote_method_not_supported` 逐条补）、工具集投影层（D4）、平台适配器、业务规则、webhook / message.callback；**以及 §0.7.6 的 `DojoExtension` 第二协议合流（合流或废弃，二选一）** | 23 条逐条对照 §0.5.3 三条判据（宿主侧无具体类型 / 新增同族不改主壳 / 卸载明确失败），**逐条打勾表进 `docs/audits/`**；接缝四方对账门（§7-1）在 11 条新接缝全绿后才允许摘 `--ratchet` |
| **P5.10** | 装配收敛：把 1.25 万行手工 new 中的「可插拔能力」部分改为注册表驱动 | 新增一个同类能力只需：插件仓产物 + manifest 声明 + 权限申请；**不改主壳代码**（最终判据） |
| **P5.11** | **全面出仓**：满足 §0.5.2 退出判据的插件逐个挪出主仓（主仓删除源码，只留主壳 + 内核 + SDK） | 主仓 `grep axagent-plugin-` 命中为 0；每仓 CI 不依赖主仓源码；兼容矩阵记录老壳 × 新插件的行为 |
| **P6.0** | **功能包层前置**（§0.8.4/§0.8.5 六项，排在 P5.11 之后、不与 L2 的 23 条混排）：① `WorkflowNode` 封闭 enum 开放化（`harness/src/workflow_types.rs:1366`）② `WorkEngine` 角色定性（底座 or 功能包，`app_state.rs:345` 证明它今天当底座用；依赖方向已核实为「域 → 引擎」，故换轨可行）③ 插件命令贡献通道（升格 `DojoExtension`，或给 `build.rs` 的注册面加运行期入口）④ 前端 tab/route/menu 数据驱动（复用已有 DynamicPageViewer 机制，改 `WorkspaceHub.tsx:166-173` 与 `workspaceTabs.tsx:28/70`）⑤ **引擎去域化**（`agent_executor.rs:4385` `PREFETCHABLE_TOOLS`、`:1232-1246` `stock_code` 预取、`:854` prompt 文本 —— 一律改为节点配置或接缝贡献）⑥ **节点执行器与接缝单轨化**（executor 只能是接缝薄壳、禁止自带实现；`document_parser_executor.rs` 空壳接真接缝，`notification_executor.rs` 的四平台 webhook 并入 `notify.channel`） | 判据：**新增一个工作流节点类型不改主壳代码**（今天需 8 处，见 §0.8.3-1）；**引擎内 `grep -i stock` 生产区命中为 0**（今天 32 行）；**每个节点执行器都能指到一条接缝**（今天 32 个执行器 0 个走能力注册表）；把一个功能包的页面 + 命令 + store 移出主仓后，其余功能编译与运行均不红 |

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
- ~~能力清单里的策略 / RAG 可以缓做~~ —— **作废**（(a) 全覆盖，范围以 **§0.7 登记表**为准（登记 23 / 有实证 22），不再是手工的 12 条）。RAG 的切法**已裁定解 1**（§0.12.3）。
- ~~「市场仿真」是能力面，要出仓~~ —— **我判为内核**（§0.7.7：DES 每 tick 的 agent 变体，出进程即每步一次 IPC）。这条与拍板措辞有出入，需你确认或拉回。
- **新增一条明确不做**：不做 **OS 级强隔离沙箱**（容器 / Job Object / seccomp 一类）。
  终局给到的是「声明式权限 + 宿主侧执法 + 用户显式同意」，**不许把它称作沙箱**；
  `source_audit.rs:5-7` 那句「形式合规检查，不是安全边界」在第三方时代依然成立，这是能力的上限而不是遗漏。

---

## 10. 一句话总计

**机制已建，分解未做**：接缝注册表、帧协议、投放已编译产物、环检测与优雅卸载都在，
缺的是 ①「新接缝」（**11 条，上限 12** = §0.7.4 的 8 + §0.7.5 的 5 减去一条待证；按 §3 的 6 处固定成本外推约 **78–90** 个改动点）
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
9. **tool 侧权限档位仍是装饰**：`#30` 只把**命令通道**改成真授权。`PluginTool::check_permission`（`plugins/src/types.rs:787-826`）与唯一引用它的 `execute_with_permission_check` **至今零调用方** ⇒ 「插件工具能碰什么」这条线仍未执法，是 §2 缺陷表里独立的一条，不是本批已完成项。
10. **相对路径声明的插件命令今天根本跑不通**（`#30` 端到端实测）：命令串整团交 `Command::new`、可执行文件查找按宿主 cwd（子进程 `current_dir` 不参与）⇒ Windows `OS error 3`、POSIX 同理。修法二选一未定（走 tool 侧同款 `resolve_hook_entry` vs 真按空白切分），任务 #47。
11. **A 通道端到端仍未通**，四份额外输入：`#43` 资产绝对路径解析命令、`#44` `PluginManifest.pages[]` 逐页声明、`#46` 插件侧桥客户端运行时（`asset` 装载的第三方 bundle 够得着桥），以及一次 `npm run tauri dev` 的运行期实测（Windows 上确认 origin 是 `https://asset.localhost`）。§0.12.5 那句「工程量小：一条配置 + CSP 一段 + 白名单测试」**已被实测推翻**，且它当时的判断「`enable: true` 只是惰性生效」也错——缺 feature 时 tauri `build.rs` 直接**硬失败**，不是静默不生效。
12. **本地 `npm run typecheck` / `tsc --noEmit` 在 UNC 共享目录静默空跑**（退 0、`--listFiles` 列 0 个文件；主代理与 `#39` 各独立复现一次）⇒ **本文与历史会话里所有「typecheck 通过」的记录都不算证据**。CI 在本地盘跑，仍有效。固化绕过 = 任务 #45。
13. **`docs/` 仍有 173 份文档未入库**（`.gitignore` 已解除排除，全部转 untracked 可提交）；批量导入前需处置 `docs/plans/PLAN-asof-gaps.md:558` 的一条 `Acs-Token: …` 签名串（全仓密钥扫描唯一命中）。
14. **本文的行号已被机器钉死**：镜像表用 `planLine` / `planPinned.mustContain` 锚 PLAN 具体行（实测本轮我少写 1 行 ⇒ 8 处钉句假红，补回同数行才转绿）。⇒ **今后编辑本文必须「行数中性」**，或给第五道门加 `--repin-plan` 重锚模式（否则正常的文档演进会持续制造假红，而假红的代价是训练人去忽略红）。
15. **`WorkflowNode` 封闭 enum 不是 P6.1 的阻塞**（2026-11-01 调研订正）：编译强制穷举位点实数 **13 处 / 9 文件**（32 臂的 base_id/base/base_mut/base_output_var、assembly_builder 两处、node_type_name/node_kind、get_node_type、workflow_template 两处、fanout、seed_stock_analysis:8140）+ enum + deserializer 双处 ⇒ 贡献新节点类型 Rust 强制 **16 位点 ≈55–75 行**，而前端 4 张表全带 default 兜底 ⇒ **静默降级不报错**（这是更危险的那一半）。但**股票只使用现有变体、harness 不迁 ⇒ 对 P6.1 改动面为 0**，它属于 P6.3（工作流编辑器迁出）。⇒ P6.0-① 从 P6.1 的关键路径上摘除。
16. **P6.1 真正的先决裁定是「命令执行形态」，不是 enum**：实测 **280/304 条股票命令签名带 `State<'_, AppState>`**（无状态仅 24 条），而今天的 T2 现形是 `plugins/src/manager.rs:583` **一次性 spawn 子进程、stdout 原样 String**，13 条远程接缝也不含命令通道 ⇒ **spawn 形状承载不了主量命令面**。§0.12.1 承诺的「把 dojo 的 `command_specs/execute_command` 形状吸收进 T2」只落了授权那一半，派发口形态**当时没有落地**（dojo 本体已删，缺口必须补）。⇒ 裁定（主代理）：**长驻 worker + 命令帧**，命令内对宿主状态的访问一律走查询接缝（唯一与「插拔的是已编译好的 worker 产物」「数据主权在宿主」两条已拍口径同时自洽的解）；进程内贡献与 worker 产物口径相悖，不取。**已执行（2026-11-02，`#49`）**：`ops::COMMAND` 命令帧 + 宿主绑定 `plugin_id`（帧内自述不符 ⇒ `PLUGIN_COMMAND_ID_MISMATCH`，**写帧前**就拒）+ 预算 30s（`DEFAULT_COMMAND_TIMEOUT_MS`）+ 宽限 1s 收帧 + 迟到帧按「无等待者」丢弃；`execute_plugin_command` 双通道（有长驻 worker ⇒ 帧；否则 spawn-on-call 逐字保留）。**唯一行为翻转**：同时声明 `worker` + `commands` 的插件，启用后命令改走帧、`command` 程序串不再执行（仓内 bundled 无此类 ⇒ 风险面在第三方）。
17. **152 处的分桶（可直接排期）**：D4/`tool.set` 门 **61**（tools crate 9 文件，随 T1 整体进插件）、装配层 **44**（`services`/`state`/`app_state`，`MarketDataProvider`/`StockStore` 已在 harness ⇒ 只需改 trait 对象注入）、跨面 `call_seam` **15**、开发二进制 **11**、进化族 **7**（A 组接缝已建）、长尾 **14**（其中 `market_sim_service.rs` 2 处建议按 §0.12.2 从股票桶摘除）。**152 处中没有任何一处需要等 #43/#44**（那是 T3 前端账）。
18. **前端收口点已定位**：全站唯一 `invoke` 收口在 `src/lib/invoke.ts:462` ⇒ 加一层转发即可让 **240 个字面调用点零改动**；304 条命令中 **173 条被 76 个文件消费、131 条零前端消费**（可作删/缓迁弹药）；76 个文件里 **63 随 bundle 迁、13 留守主壳（37 点）**。⇒ 建议页面首批留主壳、Rust 先行，避免与 T3 未完成的三件互相阻塞。
19. **本批修掉的管线缺陷（我自己提交时带入的）**：`scripts/audit-capability-inventory.mjs` 模板字符串里一处**未转义反引号**导致整文件 `node --check` 失败（AUDIT 无法复现）。已修，现 `node --check` EXIT=0 且脚本可重跑（本次重跑把该 AUDIT 按当前盘面更新了 3 行）。⇒ 教训：`audit-*.mjs` 不属 `check-*` 门禁、无 selftest 覆盖，改它的模板串后必须手动 `node --check`。
20. **#29 K4 分级已完成（10 条全判），产物 `.tmp/res-k4-dto-grading.md`（45 569 B / 7 节，增量落盘）**：**现协议 v1（一请求一响应 + 长度前缀 + `MAX_FRAME_LEN = 64 MiB`，`plugin-proto/src/lib.rs` 的 `pub const MAX_FRAME_LEN`）内即可表达 7 条** = `search.source`、`image.gen`（取「只回 url / 宿主侧落盘引用」形态）、`mcp.client`、`memory.provider`、`lifecycle.hook`、`transport.kind`、`rule.custom` ⇒ **协议版本号不必等全 10 条就覆盖这 7 条**；其中只有 **2 条需要协议外的 DTO 改造**（`lifecycle.hook` 给 `HookExecContext`/`HookOutcome` 补 serde derive、`transport.kind` 给 `AgentEndpoint` 补 derive），**其余 5 条的 DTO 今天已全 serde✓**。**D1（宿主调插件无超时）是这 7 条的公共前置**、不是某条的局部问题；**无一依赖 D2 流式**（只有 `image.gen` 坚持 b64 形态、与 `plugin.hook` 的 `post_*`/`transform_*` 需要批量语义，后者用「一次帧多条事件」即可，不强制绑 D2 的 token 流）。⇒ 两条排序变化：**D1 提到 P5.0a 之前**（任务 #53），「先做流式协议面」降级。
21. **K4 判出的「不进 P5.0a」四条（2026-11-02 订正本 —— 此前一条按二手小结转录，把 `transport.kind` 的方向判反了，已按产物原文改回）**：① `workflow.node` —— `ExecutionState`（`execution_state.rs:213`）含 ≥8 个 skip 句柄位 + context 10 KB–数 MB ⇒ **③、K4 不过**，归 **P6.0-①**（节点包协议批次④，与 `strategy.quant` 共用 spawn/feed/close），不入 P5.0a；② `context.engine` —— **判债**（0 实现者 + 0 派发点 + 与在用 `compact_session` 签名不符）⇒ 不进待建数；③ `rule.custom` —— **K4 通过**（①+低频+纯 DTO、无 D1/D2 依赖）但全仓无生产注册点 ⇒ 登记「待第一个实现者再建接缝」，与 ② **分档**；④ `plugin.hook` —— **一格拆三**（`on_session_*` 可直连 / `pre_*` 留内核待 D1+权限模型 / `post_*`+`transform_*` 需批量帧）。⇒ **`transport.kind` 实测全 ①**：原判据错在把「实现侧进程内自持的连接态」当成过帧句柄（4 个实现者全在 `runtime/src/transport_handlers.rs`：`:229 WS`/`:380 HTTP`/`:504 SSE`/`:588 Stdio`；`connect_internal` `:100` split 后 `tokio::spawn` 长驻循环 `:135`，持 `connections: RwLock<HashMap<agent_id, mpsc::Sender>>>` 与 ping/pong 超时 `:175` —— 出进程后依然自洽，同 `mcp.server` 的结论），且 `MediaAttachment`（`harness/src/message_adapter.rs:67-73`）**只带 `path`** ⇒ 无 4× 膨胀、无分块需求；真缺口是今天**无入站消费路径**（`transport_handlers.rs:155-171` 的 `Message::Text` 落 `_ => {}`）。⇒ 移桶只涉及 `workflow.node`（B 10→9）与 `context.engine`（C 5→4）⇒ 能力面 **26 → 24**；分组是策略数、一改就级联（`groupCounts` 变、且与 §0.9.4 的「有实证 24 条」撞数）⇒ **单独一次动作**（任务 #54），并与 #33 的分桶结论同批收口，避免两次级联。
22. **K4 分级与 PLAN 原文的三处分歧（以盘上为准）**：① `plugin.hook` 的 `ToolCallContext` 本就是 4 字段纯 DTO，**不需要等 D4 投影改造**（D4 丢弃的是宿主可执行上下文 `ToolContext` = `harness/src/tool.rs:142`，**tools 侧只是 re-export 同一份 ⇒ 近名不同物，不是重复定义**，#50 实测订正）；② `mcp.server` 的「会话式 op」是**延迟优化**而非 K4 前置（桥今天即 one-shot，`mcp_tool_bridge.rs:197-202`）；③ `context.engine` 在零实现者之外新增**零派发点**事实（全扫无 `dyn ContextEngine`，生产压缩走 `compact_session` 自由函数 `harness/compact_session.rs:67` + `agent/session_manager.rs:781` 直调）⇒ 连 K2 都不成立。
23. **本批另立案三条（都源自实测，不是推测）**：**#50** `pub trait` 三读数 318/316/315 的差值查清 = `LlmReasoningProvider`、`ProviderRepository`、`SettingsRepository` 各定义两次，其中后两个在 **harness 同一 crate 内重复定义**（`platform_adapter.rs` vs `repositories.rs`）= 违反铁律 4；**#51** `../` 前缀命令修复后由「必然失败」变「真解析到安装目录外执行」，收紧必须 tool 与 command 一起收，属安全面单独立项；**#52** 本批故意未提交 `register_commands.rs`（它同时含另一会话未落地命令的注册行，单提会让 HEAD 编译不过），代价是干净检出上的产物比 PLAN 登记值**少 1 条命令**（旧读数 vs 新读数之别，两数并列只允许写在 §0.12.4）⇒ 第五道门在 CI 上可能假红，**不许为此把 PLAN 退回旧数**。

## 12. P5.0a 定版（2026-11-02，#29 K4 转录完成后；判据全部来自 `.tmp/res-k4-dto-grading.md` §2–§3）

**硬前置 = D1**（宿主经 `call_seam` 调 worker 无超时，任务 #53）。**状态更新（2026-11-02）：D1 收侧已闭合 ⇒ 本节的开工闸门已打开**（形态见 §18）。写侧与握手 `describe` 仍开放（#56 / #63），但**不再阻塞 P5.0a 开建**——理由：收侧超时已兜住「远端不可达即永久挂住」这条主风险，而写侧只在**对端僵死且管道满**时才卡，是另一类故障面。⇒ 每条接缝的验收判据必须自带超时语义测试（不许依赖 `AXAGENT_TEST_PLUGIN_SUBPROCESS` 的 SKIP 蒙过），并把 §18 的「7 个 op 预算待实证」回收合耗时分布。

### 12.1 现协议 v1 内即可表达、且现在就该建 —— 5 条

| 接缝 id | op 面 | 建之前必须做的协议外改造 | 宿主改动面 | 验收判据 |
|---|---|---|---|---|
| `search.source` | `search`（+ `extract_content` 走分页/游标） | 无（`search_provider.rs:38-57`、`research_state.rs:416,76` 全 serde✓）；`extract_content` 返回大文本 ⇒ **分页/游标**而非一次大帧 | `axagent-ai/crates/agent/src/search_orchestrator.rs:28` 的 `HashMap<SourceType, Arc<dyn SearchProvider>>` 加远程分支 | `rate_limit()`/计费仍在宿主；一个真 worker 产物跑通 `search` 往返 |
| `image.gen` | `generate`（`image_gen.rs:64`） | 无（`:24-36,45-54` 全 serde✓）；**只回 url 或宿主侧落盘引用**形态 ⇒ ①；坚持 b64（1.3–4 MB×n）才需要 D2 分块 | providers 注册面按 `name()` 自述键加远程分支 | provider 凭证 scope 不出进程；b64 形态被显式拒绝且有测试 |
| `mcp.client` | 请求-响应 + `evict_by_server_id`（`mcp_client.rs:547`）+ `probe_and_evict`（`mcp_health.rs:100`） | 无（`mcp_service.rs` DTO 全 serde✓）；`McpServerStore` **留内核**（配置面不是能力面） | `client_service_impl.rs:155 build_*`；桥 `mcp_tool_bridge.rs:197-202` 今天即 one-shot | 连接池仍进程内全局（`mcp_client.rs:661/:434/:2407`）；**句柄不过帧**有测试锁住 |
| `memory.provider` | `sync_turn` / `prefetch` / `provider_name` | **复核订正（2026-11-02）：不是「无」** —— 体量侧 DTO 全 serde✓（`memory_provider.rs:8-60`、tier/nature `service.rs:19,95`），但**键控位点不存在**：`Arc<dyn>` 只在仅测试构造的 `MemoryHookProvider`，模块自述无注册中心（`skill_summary_provider.rs:24/31`）⇒ **建之前必须先建 provider 键表**（`provider_name()` 只是自述键、不是分发点） | 同上：触发 = 会话首尾各 1 次（非每轮） | 键表建好并有一条真 worker 产物注册进去，才算这条接缝成立；预取语义与会话生命周期对齐 |
| `lifecycle.hook` | `preExec` / `postExec` | **补 serde derive**：`workflow_lifecycle.rs:51 HookExecContext` / `:64 HookOutcome` 今天无 derive（③）；`postExec.results` 是全 DAG 输出 ⇒ **引用化** | `workflow_lifecycle.rs:33` 4 实现者 | 每次工作流执行 1+1 次往返；ctx 20 KB–400 KB 不整团过帧 |

### 12.2 可表达但本批不建（各自都有判据，不是拖延）

- `transport.kind`：体量**全 ①**、补 `AgentEndpoint`（`message_gateway.rs:143-152`）derive 即可走接缝 ⇒ 但 4 个实现者全在 runtime 内核 crate，第三方收益只是「新增一种 transport 不改主壳」（K1 面窄），**优先级低于 `notify.channel`**；且要求 **worker 与宿主同文件视图**（`MediaAttachment` 只带 `path`）。⇒ 排 P5.2。
- `rule.custom`：**K4 通过**（①+低频+纯 DTO、无 D1/D2 依赖）⇒ 但门口径实现者 0、`with_custom_registry` 外部调用 0（内核消费逻辑是活的：`constitution.rs:158/:170` + 三个调用点）⇒ 建了没有实现者。登记「**待第一个实现者再建接缝**」。
- `plugin.hook`：**一格拆三** —— `on_session_*` 今天即可直连（低频，可并入 12.1 任一批）；`pre_*` 留内核待 **D1 + 权限模型**（tool 侧 `check_permission` 至今是装饰，见 §0.7.6）；`post_*`/`transform_*`（= 工具输出/HTTP body，②）待**批量帧**语义（「一次帧多条事件」即可，不强制绑 D2 token 流）。
- `workflow.node`：K4 **不过**（`ExecutionState` `execution_state.rs:213` 含 ≥8 skip 句柄位 ⇒ ③；每节点一次往返）⇒ 归 **P6.0-①** 节点包协议批次④，与 `strategy.quant` 共用 spawn/feed/close。
- `context.engine`：**判债**（0 实现者 + 0 派发点 + `Session` 无 `Deserialize` 与在用 `compact_session` 签名不符）⇒ 不建、**不预留协议面**。

### 12.3 与镜像表的关系（#54 的输入）

本节「建 5 / 缓 1 / 待实现者 1 / 拆三 1 / 不过 1 / 判债 1」是 **#54 移桶的直接输入**：B 少 `workflow.node`（10→9）、C 少 `context.engine`（5→4）⇒ 能力面 Σ **26 → 24**（与 §0.9.4「有实证 24 条」撞数但**不同义**，改表同批必须写明措辞）。⇒ 分组改动的执行**等 #33 的 198 条分桶结论一并收口**，避免两次级联；在此之前 §0.7.4/§0.7.5 保原分组，只在标题与本节标注判定。⚠ **已执行（§24.1 + §0.7.9，`#54`）：最终为 23（B 8 / C 4；`memory.provider` 也移债档）——本节 24 是中间态，勿再引用为现值。**

## 13. `#49` 交付的两张下游弹药（2026-11-02 转录，报告全文在其会话）

### 13.1 D1 的真实形状是**两半**，本批只可能闭合收侧（⚠ 本节是 #49 交报告时的时点快照：**收侧已由 #53 闭合，见 §18**；下文所有 `worker.rs:NNNN` 已整体后移约 60 行，**认符号不认号**——文档补录会话拿盘上核我给的数字，实测 6 处过时）

- **收侧（`#53` 在修）**：seam 唯一入口 `SeamInvoker::invoke_with_chain` → `worker.rs:1387 self.exchange(...)` → `worker.rs:1071` 以 `wait=None` 委托 → `:1121–1125` `receiver.recv()` **无限阻塞**；全仓 `exchange_with_timeout` 只有 `:1213`（命令帧）一处传 `Some`。`plugin.rs:403` 的 `OP_UI_ACTION` 是同病灶 ⇒ **在 worker.rs 内部上预算即可，不需要动主 crate**。
- **写侧（新立 `#56`，本批明确不做）**：`worker.rs:1100–1108` 的 `stdin.lock()` + 阻塞 `write_json_frame` ⇒ 对端僵死不读 stdin 且管道满时**写方持锁卡死，`recv_timeout` 覆盖不到**；修它要 writer 线程 / 异步写 = 接缝栈重构。⇒ **只修收侧不等于 D1 闭合**，这条要写进代码注释，否则下一个读者会误判。
- **预算裁定（主代理）**：不给 `FrameRequest` 加预算字段 —— 预算是**宿主策略**、不是插件可协商的东西，协议面零变化优先；逐接缝预算表放 plugin-proto，与 `DEFAULT_COMMAND_TIMEOUT_MS` 同族，默认 `DEFAULT_SEAM_TIMEOUT_MS = 15_000`，合法长跑型（典型 `agent.loop.run_turn`，分钟级）显式 180_000，**其余按 `SUPPORTED_REMOTE_SEAMS` 逐条定，定不出来的用默认并注释「预算待实证」**。超时后的连接处置沿用 `#49` 既有语义（撤配对槽位 + 迟到帧按「无等待者」丢弃，`:1311–1319`）。
- **超时无取消协议**（既存缺口）：僵死 worker 的 unload 走 `drain → shutdown` 裸 `recv` 要等它自己醒来，兜底靠 `kill_child`（Drop 路径，e2e 实测有效）。

### 13.2 股票域 280 条命令的**查询接缝缺口**（`#49` 实测 24 文件字段直方图）

`harness` 181 · `astock_client` 89 · `cron_job_store` 22 · `trading_engine` 10 · `work_engine` 8 · `stock_monitor` 6 · `quote_watcher` 4 · `vector_store` 3 · `cross_stock_aggregator` 3 · `trajectory_storage` 2（`status`/`inner`/`input_params`/`variables` 是局部 `state` 噪音，迁移时剔除）。

对照 `SUPPORTED_REMOTE_SEAMS`（13 条）⇒ **以下全部需新增查询接缝**：行情 `market.quote.*`（**89 处，最高优先**）、调度 `jobs.store.*`、交易引擎 `trading.engine.*`、监控 `stock.monitor.*` / `quote.watcher.*`、向量 `knowledge.vector.*`、轨迹 `trajectory.store.*`；`harness` 那 181 处大头可走既有 `event.dispatch` + `workflow.*`。
⚠ **该直方图只支持接缝优先级排序，不构成 280 条的逐条归类**。单命令迁移五步模板：输入 DTO 钉死 → manifest 双声明 → 缺哪条数据补哪条接缝 → worker 实现 → 摘 `State`；**24 条无状态命令只需 ①②④**。

### 13.3 对 `#35` 排期的直接影响

P6.1 的先决（T2 命令帧）**已落地** ⇒ `#35` 的阻塞从「等派发口形态」改为「**等 D1 收侧（#53）+ 13.2 的接缝面（至少 `market.quote.*`）**」。另：主 crate 的编译窗口实测 **16m47s**（proto 变更触发全图重编），排期按这个数，**不要按 10m33s 的基线估**（`#49` 报告 §②#7）。

## 14. `#33` 的 198 条分桶闭合账（2026-11-02 转录；产物 `.tmp/res33-trait-adjudication.md`，330 行 / 56 471 B）

### 14.1 总和闭合
`198 = 桶1 真接缝候选 13 + 桶2 应为策略数据 3 + 桶3 必须留内核 175（3a K4 不过 13 / 3b 只有 DI·构造期选择 134 / 3c 既定口径·主权·机制 28）+ 桶4 零派发点 7 + 待人工 0`。自证 = `node .tmp/res33-verify.mjs` EXIT=0（198 条在全文恰出现一次，零缺失零重复）。**⚠ 对抗复核后的改判（2026-11-02，`.tmp/rev33-*`）：桶1 13 → 8（5 条改桶：`MarketplaceScanner`/`UnifiedKnowledgeSource`/`PluginHook`/`MemoryProvider` → 桶3，`Strategy` → 桶2）⇒ 新账 = 8 / 4 / 179 / 7，总和仍闭合。**

### 14.2 桶1（13 条）= P5.0a 的候选面
11 条其实就是 §0.7.4/§0.7.5 已登记的契约（`DocumentParser` · `EmbeddingProvider` · `ImageGenProvider` · `McpClientService` · `MemoryProvider` · `NotificationChannel` · `PluginHook` · `RerankBackend` · `SearchProvider` · `Strategy` · `WorkflowLifecycleHook`），被旧 K2 谓词的假阴性从「键控点」证据里**吞掉了**；~~2 条是本轮新证 ⇒ 要进登记表~~ ⇒ **两条新证均被对抗复核推翻，登记表不新增任何条目**：
- `MarketplaceScanner`（**复核改判 → 桶3**）—— 21 个 impl **全在 `axagent-runtime/crates/tools` 内部**（17 文件 + 4 同文件），**仓外实现者 0**；`platform()` 有 3 个 wrapper 取调用方自填（`:1209/:1427/:1505`）⇒ id 唯一无强制；工厂 `:387` 是硬编码 18 臂 ⇒ 属 `tool.set` 内部，不是独立能力面。
- `UnifiedKnowledgeSource`（**复核改判 → 桶3，且与 B 组 `rag.source` 同族**）—— 4 个 impl 全在 `dao/src/search_sources_impl.rs:107/180/289/407`；`RAGSource`（`search/src/rag.rs:45`）是单后端、本 trait 是多源门面，`source_type` 只分 KnowledgeBase/Wiki/Memory/Obsidian ⇒ **按禁区 12 并入 `rag.source`，不新增条目**。

另三条同批改桶：`PluginHook` → 桶3（`hook_chain.rs:20` 是 **Vec 广播、无键**，`pre_*` 每工具调用一次）· `Strategy` → **桶2**（差异载体是 rhai 脚本 + 参数 JSON：`quant/src/script.rs`、`param_optimizer.rs:72` ⇒ 不开接缝；但 K4 侧仍判 bar 级热循环 ⇒ 它走 §12.2 的「节点包/批量 op」那条路）· `MemoryProvider` → 桶3（`Arc<dyn>` 只出现在**仅测试构造**的 `MemoryHookProvider`，模块自述无注册中心 `skill_summary_provider.rs:24/31` ⇒ **键控位点不存在**，与 §12.1 的冲突处理见该表末列订正）。**维持 8 条**：`DocumentParser`（但证据行要改成 `lib.rs:61` 的 `match mime_type`——`:533` 只是 ext→MIME 映射、不构造实现）· `EmbeddingProvider` · `ImageGenProvider` · `McpClientService` · `NotificationChannel` · `RerankBackend` · `SearchProvider` · `WorkflowLifecycleHook`。两处实测降本/降信：`WorkflowLifecycleHook` 的键位**已是** `RwLock<HashMap<String, Arc<dyn …>>>`（`engine/mod.rs:665`）真注册表 ⇒ 升格成本降到 ≈6+0.5；但 #29 订正它的 4 个实现者**全在主壳业务命令文件**（`opc_workflow_kpi_hook.rs:442`、`stock_workflow/hooks.rs:1036/1105/1159`）⇒ **「天然第三方面」证据不足，K3 待第一个仓外实现者验证**。

### 14.3 桶2（3 条）= 不开接缝，开参数/声明载体
`CapabilityPassport`（`harness/src/capability.rs:815`，方法面纯数据声明，无「按护照选实现」的运行期站点）· `HealthCheck`（`runtime/src/health_check.rs:70`，键位在 `:76`，但 §0.7.7 已裁「并入插件清单自检面」）· `RecommendStrategy`（`analysis-engine/src/recommender/strategy.rs:58`，`mod.rs:1115/1119/1613/1614` 是 Vec 全链遍历、**无键选择** ⇒ 载体应是参数表）。

### 14.4 桶4（7 条）= #34 的最终候选（旧「29 个零实现」口径作废待核）
判据（02:59 修订版）：剔除 trait 定义块与 `impl … for T` 块自身后，**dyn / 泛型 bound / 关联路径 / 实现类型在非自身文件被构造或提及（wiring）四类位点全 0 ⇒ ZERO**；`pub use` 导入行**归为非取用**（这正是 ZERO 从 4→6 的原因）。**⚠ 对抗复核逐条复扫后：7 条里只有 2 条可删。**不可删 4 条（都有生产取用点）：`LlmContentGenerator`（`research_agent.rs:356/403` 直调 `generator.generate_outline/generate_content`）· `TaskExecutorImpl`（`task_executor.rs:314` `executor.execute_task(&context)`）· `SignalService`（`device_signal.rs:55/62/91/175` 调 `self.mark_online/broadcast_signal`；实现零构造 = 未接线，但删 trait 必改方法体）· `FeatureFlagProvider`（`tools/agent.rs:21` `OnceLock<SharedFeatureFlagProvider>` + `:244/:253` 的 `f.is_enabled(...)` 是生产读点）。**可删 2 条**：`IrRenderer`（8 处全在 `ir_renderer.rs:113` 起的 `#[cfg(test)]`）、`KitSlashCommandProcessor`（全仓仅 3 处：def / impl / `pub use` 续行）。`OpcAutomationService` 零取用但属 OPC 迁出族 ⇒ **随 #32 处置，不单独删**。
硬 ZERO 6（⚠ **本行前两个号是 `#34` 删除前的时点号，故意保留不改**：`harness/src/ir_renderer.rs` 已**整文件删除**、`harness/src/kit_bridge.rs` 现 **119 行** ⇒ `:121` 落在 EOF 外，`--dump-doc-refs` 因此把这条报成「越界 1」（属既存 7 条可疑之一，非新坏；本行是 §14.4 判定「可删 2 条」的依据本身，改号就等于抹掉判据））：`IrRenderer`（`harness/src/ir_renderer.rs:18`）· `KitSlashCommandProcessor`（`harness/src/kit_bridge.rs:121`）· `LlmContentGenerator`（`agent/src/research_agent.rs:51`）· `OpcAutomationService`（`analysis-engine/src/opc/automation.rs:298`，随 #32 处置）· `SignalService`（`harness/src/device_sync.rs:647`）· `TaskExecutorImpl`（`agent/src/task_executor.rs:92`）；B 级 1：`FeatureFlagProvider`（唯一 dyn 位点是别名定义 `feature_flag_provider.rs:17`，生产实现 0）。
外延观察 6 条**不判删**（有真实 wiring 兜底构造位点，只有 Noop/Mock/Echo/Fail 空壳）：`DecomposerLlmClient`、`LlmSampler`、`LlmEvaluator`、`PluginLifecycle`、`SubTaskHandler`、`TokenUsageSink` ⇒ 属 #34「接口活着、实现空心化」同族。
⚠ #34 原口径「**29** 个零实现空接口」与本轮 7 条**不是同一谓词**，出处已定位 = `docs/audits/AUDIT-capability-inventory-2026-11-01.md:21`「空接口（全仓无实现）29」，谓词 = `audit-capability-inventory.mjs:106` 的 `implFiles[t].length === 0`（**含 harness 与测试实现**）⇒ 它数的是「无实现」，本轮数的是「有实现但无取用」，**两个数不能互换**。⇒ #34 的真实候选面从「29」收敛为**「2 可删 + 1 随 #32 + 4 不可删」**。

### 14.5 K2 盲点的四类漏法（#57 的输入，逐条已实测）
⑤ **容器字段声明**（命中 15 条；映射型 `HashMap/BTreeMap/DashMap/IndexMap` = 强键控，顺序型 `Vec/VecDeque` = 弱，须与自述键或按 key 筛选同现才升强）· ⑥ **push 注册口动词面**（`push|add_*|set_*|insert`，本轮证据 `add_stage`、`set_unified_sources`、`scanners.push`）· ⑦ **自述键 + 集合筛选** ⇒ 判「弱键控」，**不直接进 51** · ⑧ **factory-match 构造具体类型**（`DocumentParser` 的 37 分支，需先建 impl→trait 反查表）。
净结论：**21/198 假阴性** = 15 容器 + DocumentParser + 3 条 keyed-get/桥形态 + 2 条自述键。**容器命中 ≠ 能力**（15 条里只有 8 条进桶1）⇒ 登记表要加「裁决列」，谓词只产候选、不产结论。
次序硬约束：新谓词先以 `--dump-readings` 的 `k2.strongHits / k2.weakHits`（`hard:false`）出数并与本表 198 逐条对账跑顺，**才**谈升 `hard:true` 与改 51/26 锚点；**严禁静默重算 51 ⇒ 26**。

### 14.6 对 #54 的裁定输入（结论：能力面 Σ 不变，成员换进换出各 2）
~~B 组 −1 +2（两条新证）= 11；C 组 −1 = 4 ⇒ Σ 仍 26~~ ⇒ **复核推翻两条新证后：B 组 −1（`workflow.node` 移内核，§12.2）= 9；C 组 −1（`context.engine` 判债）= 4；A 组 11 不变 ⇒ Σ 26 → 24**，且 `MemoryProvider` 虽改判桶3 但它是 **B 组已登记条目**（`memory.provider` 留在 B，只是「建之前必须先建注册表」）⇒ 不动分组。**为什么它留 B、`MarketplaceScanner` 却出表（判据必须可复述，不能像临时豁免）**：B 组的准入条件是 **K1（有非 harness、非测试实现者）+ 有第三方贡献故事**，键控位点**缺失是「待建」的内容之一**，不是出局理由 —— `memory.provider` 有 2 个真外部实现者（`CountingProvider`/`SkillSummaryProvider`）与会话首尾的触发面 ⇒ 故事成立，只是`provider_name()` 自述键**不等于**分发点、注册中心不存在 ⇒ 建接缝=建接缝+建键表（成本 6+1）。而 `MarketplaceScanner` 的 21 个实现者**全在 tools crate 内部**、`platform()` 由调用方自填 ⇒ **K1 与故事双双不成立**，与「键表待建」是两种缺陷。⇒ #54 落地时 `memory.provider` 的 §0.7.4「键控分发点」列必须照此改写（现写「`provider_name()/provider_version()` 自述键」，是把自述键当分发点的原病）。⇒ **与 §0.9.4 的「有实证 24 条」重新撞数**：改表同批必须写明两者措辞不同义（本文的 24 = 能力面登记数，§0.9.4 的 24 = 有实证的可插条目数），否则读者会当成同一个数被重复声明。⚠ **已被 §24.1 推翻（`#54`）：`memory.provider` 移入 §0.7.9 债档 ⇒ 最终 23；「留 B」这段保留作判据史，勿再引用为现值。**
一处必须裁的冲突：`TransportHandler` 在 #33 按任务既定样本留 3a（「必须留内核」），而 #29 实测判「全 ①、可走接缝、原判据不成立」⇒ **以盘上实测为准（跟 #29）**，但它不进 P5.0a、排 P5.2（§12.2）。⇒ 两判不矛盾的根源是**「能不能走接缝」与「现在该不该建」被混成一列**，登记表与 §0.7 表都要拆成两列。

### 14.7 ⚠ 污染事件（读数可信度的前提，必须留档）
本会话期间 `.tmp` 探针被**上一轮「已死」会话的子进程迟到重写三次**：`scan3.mjs` 02:34→02:59（ZERO 名单 5→4→**6**）、`regen.mjs` 02:52（容器命中 12→**15**）、`use-sites.tsv` 03:00 被改写。处置 = 对最新盘上版本逐一复跑（各 EXIT=0、产物逐字一致）后采信，并冻结为 `.tmp/res33-frozen-*`（md5 记入产物 §6.3）。⇒ **今后对账数字与本表不符时，先查 frozen md5，再读数**；「污染是否仍在发生」由对抗复核会话回答（未决）。

## 15. `#52` 的实测与**读数入库纪律**（2026-11-02）

### 15.1 结论：干净检出**必红**，但败因不是「产物滞后」

| # | 实测结论 | 证据 |
|---|---|---|
| 1 | 工作区产物不滞后（1542 条 / 1608 个 LF），与逐字复刻 build.rs 算法的输出**逐字节相同**；HEAD 产物 = **1540 条**、HEAD 源码按算法应生成 **1541** ⇒ 当时登记值 1542 **比 HEAD 源码多 1**（该数为 #75 前的登记态）（含另一会话未落地的 `get_reco_pool_batch`） | `.tmp/buildrs-replica.mjs`；`git show HEAD:src-tauri/src/register_commands.rs` |
| 2 | 第五道门在 job `workspace-config-parity`（`ci.yml:1026`，无 `needs`/无 `if` ⇒ push+PR 必跑），该 job **连 Rust 工具链都没装**、前面不存在任何重生成产物的步骤；把 HEAD 树 `git archive` 出来复跑本门 ⇒ **5 条硬失败，首条 `commandTotal 登记 1542 / 实测 1540 / 差 −2`**。同棵树上门禁契约 A（`ci.yml:562`）也红 ⇒ **假红不止一处** | `.github/workflows/ci.yml:1026-1062`、`scripts/check-contracts.mjs:95-121` |
| 3 | `--quick`（pre-push 路径）**必跑**此门且前面同样没有重生成步骤；本机绿/CI 红的差别只在「本机产物被 cargo 刷新过」 | `scripts/ci-check.mjs:474-477、516` |
| 5 | `registryFileLines` 的 ±1 **是口径错配不是滞后**：脚本用 `split("\n").length`（含末行空元素，`check-capability-register.mjs:1169`），label 却自陈「纯 LF 行数」= `wc -l`；HEAD 报 −1、工作区报 +1 ⇒ 它随产物长度走，**产物补齐后仍差 1**。同脚本对 `frontendFileCaliber` 明说用 split 口径 ⇒ **一个脚本两套行数口径**（禁区 12 候选违规） | 锚点 `registryFileLines`；`.tmp/gate_head.txt:7` |

⇒ **5 条红里 4 条**（`commandTotal`、股票命令 304/303、306/305、前端体量 −355/−326）根因都是「**登记值取工作区瞬时读数，代码未同批入库**」——修法 (a) 改读源码 / (b) CI 前置构建 / (c) 滞后分流**一条都救不了**，能救的只有工序。

### 15.2 纪律（本仓今后登记任何 `hard:true` 读数的三条硬规矩）
1. **登记值必须能由「同批提交的那棵树」复现**；取自工作区瞬时读数的，必须在提交说明里点名它依赖谁的未提交代码（本轮 = `commandTotal=1542`（#75 前的登记态）依赖另一会话的 `get_reco_pool_batch`）。
2. 提交前先跑 `--repin-plan` / `--repin-code` 与判定档，**不许把「待回写」当「已一致」**。
3. 一套脚本只许一套行数口径；`registryFileLines` 已按 split 口径重新登记（1609），label 与判据的分歧在 §15.1 第 5 行留档，**不许再用它当滞后的证据**。

### 15.3 `ci.yml` 行号引用的真实规模（注释本身是过期读数）
全仓 `ci.yml:<行号>` 形态引用实测 **278 处 / 涉及 101 个文件**（`.md`/`.rs`/`.ts`/`.tsx`/`.mjs`/`.json`；排除 `node_modules`/`target`/`.git`/`dist`/`.tmp`/`coverage`），而 `ci.yml:1021-1022 / 1047-1048 / 1072-1073` 三处注释自陈「被 **18+** 处引用」⇒ **低估一个量级**。⇒ 「新步骤一律追加 job 末尾、禁止中段插入」的真实代价：中段插 N 行会让其后**全部**引用整体移位，落进 `check-single-source-facts.mjs` 的 BROKEN 桶是**几十到上百条**。
另：`src/__tests__/**` 对 `check-capability-register` / `capability_register` **零直接引用** ⇒ 第五道门在测试面只被对称差集护栏间接覆盖（`ci.workflow.test.ts:298-503`），而该护栏的 `ALLOWLIST` 已 **8/8 满**（`MAX_ALLOWLIST=8`，`:387-425`）⇒ **单边接线必红且没有空位登记**。

### 15.4 落地分层（主力是工序，不是扫描口径）
1. **本批**：镜像表 + 本文 §0.12.4 同批改；`register_commands.rs` **明确不提**（它含另一会话的注册行，单提会让 HEAD 编不过）。
2. **`#58`**：第五门在比 `commandTotal` 之前先跑**已接线的** `check-contracts.mjs:95-121` 名字集合谓词（`defined ⊆ registered`，零新解析器、成本实测 5.5s）⇒ 判出「滞后」就走专码并点名「跑 `cargo check` 重生成后复跑本门」，一致才按登记值判**真漂移**。⚠ 专码只买到**诊断正确性**，在 GitHub Actions 里仍是红（`ci-check.mjs:114-137` 的 `step()` 非零即中断）。
3. **可选最小版**：把 `commandTotal` 口径改成「契约 A 名集合」，并**新增不变量「名集合数 == 产物条目数」**，用来兜「同名跨模块只注册一个 ⇒ 名集合静默低估」的风险（今天 0 例，但**无编译期保证**）。
4. **否掉 (b)**：为该 job 装 Rust 工具链 + 全量编译只为拿一个文件；且重生成后是 1541 仍 ≠ 1542，**对本批红无效**，副作用最差（CI 现场把产物写回工作树，让「产物必须入库」的纪律更隐身）。

## 16. `#49` 的独立审查结论与裁定（2026-11-02；审查稿 `.tmp/rev49-findings.md`，基线 = 未提交工作区）

### 16.1 一条必修回归（M1 ⇒ 任务 **#59**，已派；行号同上「认符号不认号」，且 `manager.rs` 的前置链盘上实为**五道**：存在→启用→**命令声明**→授权→形状，#49 报告写的「四关」少一道）
「旧 worker 优雅降级」这句自述**在盘上站不住**：`manager.rs:732-743` 只要 `loaded_workers` 命中就走命令帧，未实现 `ops::COMMAND` 的旧 worker 按 handler 模板回 `PLUGIN_UNKNOWN_OP`，宿主只把它归成 `CommandFailed(COMMON_INTERNAL)` ⇒ **对「已声明 `worker` + `commands`、但 worker 二进制未升级」的存量插件是功能回归**，升级主壳后首次执行命令即失败。而 `manager.rs` 自己的文档写「宁可用旧语义，不给命令凭空消失」——**回退只覆盖「worker 没在跑」，没覆盖「worker 在跑但不认识 command」**。⇒ 裁定：按最小修复落地 = 通道一失败含 `PLUGIN_UNKNOWN_OP` ⇒ warn 后落通道二（该码是**派发前的能力拒绝、无副作用** ⇒ 回退安全；这条前提必须写进注释并锁测试，否则后人不敢加回退）。

### 16.2 A2 判我超额表述，已当场订正 §0.6.1
`command_frame_is_additive_no_proto_bump_required` 锁的是**宿主侧对合成错误帧的解码**，`worker.rs` 的三条用 `Peer` 测试替身回构造帧 ⇒ **没有任何测试能锁住「真实旧 worker 二进制会回该帧」**。⇒ 准确措辞 = **「约定 + 夹具示证」**。
顺带立一条**引用写法纪律**：正在被并发编辑的文件，`文件:行` 一律改写成**符号引用**。本轮实测漂移：`plugin-proto/src/lib.rs` 从 `+404` 长到 `+514`，其间 `MAX_FRAME_LEN` `:64→:80`、`ops::EMIT` `:242→:258`、上面那条测试 `:938→:1481`。⚠ **单源事实门禁对这类腐烂是放行的**：本轮 `check-single-source-facts.mjs` = `BROKEN/越界/空行/控制流 0`、EXIT=0，因为它只验「该行存在且非空/非控制流」，**不验该行还是不是那个符号**。⇒ 别指望门抓，靠写法。

### 16.3 A3 ⇒ 任务 **#60**（裁定：命令超时不得默认引导重试）
`commands/plugin.rs:675-677` 把 `CommandTimeout` 映射成 `COMMON_TIMEOUT` / **Retryable**，注释只写「调用方可原样重试」；但 `worker.rs:1083-1087` 自己声明「**超时 ≠ 取消**，worker 可能仍会跑完（副作用已发生）」，而 serve 循环逐帧处理 ⇒ 第一条仍在跑时用户按重试再发同一命令 = **二次/并发执行**。⇒ 短期：分支注释与前端引导文案注明幂等前提；根治：命令帧加**去重键**或补**取消协议**（与 #56 写侧同族）。需主 crate 窗口（实测 16m47s），排下一批。

### 16.4 A4 = `#51` 的独立判读（裁定：**该收紧，模板就在同文件**）
- 越界面**不是本批引入**：由已提交的 #47 引入；本批命令帧通道根本不经过 `resolve_hook_entry`（`manager.rs:749-757` 仅通道二）。
- **绝对路径改动前后一直能越界**（`root.join(绝对)` = 绝对，`manager.rs:2841`）。
- 门槛拦不住：`check_command_gate`（`:789`）+ `subprocess_execution` 都是「按插件**自述**权限名放行」⇒ 是 **authorization 不是 containment**（`manager.rs:676` 文档自己承认）。恶意插件声明 `execute` + `subprocess_execution` 即合法过关。
- ⇒ 收紧判据：`./`、`../` 声明 canonicalize 后必须 `starts_with(安装根)`，否则 `InvalidManifest`；绝对路径声明纳入**同一门槛**（带迁移豁免期）。**模板已存在于同文件** = `resolve_worker_program`（`:2856-2865`：拒绝对路径 + 拒绝任何 `ParentDir` 分量）⇒ 照抄形态，不另造。⚠ **必须 tool 侧与 command 侧一起收**（两者共用 `resolve_hook_entry`）——单边收紧会让两条通道的能力面分叉，这正是禁区 12 要避免的形态。

### 16.5 审查放行的部分（记下来，免得下轮重复怀疑）
三道防线本体成立：`pluginId` 是**宿主绑定**（比对源 = spawn 时注入的注册表权威键，`worker.rs:941/979/1001`、`manager.rs:732-734`），**不是帧内自述字段 ⇒ 不是假防线**；drain 闸门与接缝同口径；迟到帧按**单调 `request_id`** 丢弃 ⇒ **不串答**、流不污染。seam 的 `None` 分支逐字等价、三个 seam 调用点确无预算。spawn 主体是 diff 上下文行、逐字未动。禁区 8/9 零违例（新代码只用 `parking_lot` / atomics / mpsc，Tauri 侧 `spawn_blocking`）。`CommandFrame` 族全仓唯一（铁律 4 过）。`MAX_FRAME_LEN` **先校验长度再分配**，新代码无 panic 面。
⚠ 但这份审查是**纯静态**的：未跑 fmt/clippy/test、e2e 未实跑 ⇒ 「可接受」≠「已验证无风险」。A1（两处超 `max_width=100`）正是它标出来、但只能靠 cargo 定论的那一类。

## 17. `#50` 的三读数归因与 harness 重复 trait 定案（2026-11-02；产物 `.tmp/res50-trait-caliber.md`，152 行）

### 17.1 「三个口径」是伪命题：318 / 316 / 315 = 同一口径的三种计数姿势 + **一份过期工件**
同一工作树实测（EXIT=0）：「8 根 vs 全仓」「行首 vs 允许缩进」「排除 vs 不排除 `tests/`」**三处分叉的实际贡献全是 0**（另测：`tests/` 与示例桁命中 0、非行首 `pub trait` 0、`pub unsafe/async trait` 0、8 根之外 0、`#[cfg(test)]` 块内 0）。
- **315** = 按 trait 名去重的唯一名数（第五道门现跑的正是它，可复算）。
- **318** = 原始命中行数（`318 − 315 = 3` 条重名溢出）。
- **316 不是「活数」**：`scripts/audit-capability-inventory.mjs:83` 读 `.tmp/traits-full.json`（mtime **2026-11-01T04:41:54Z**，**里面仍含已删的 `DojoExtension`**，盘上该文件已不存在）再取 `Object.keys(cen.def).length` ⇒ 同谓词在当前树重跑 = **315**。**`316 − 315 = 1 = DojoExtension`，是漂移不是口径差**。
- 连带订正：`docs/audits/AUDIT-capability-inventory-2026-11-01.md:15` 的「316（harness 233）」是**同一工件的快照**，harness 侧今天复算 = **232**（差 1 也是 `DojoExtension`）。
⇒ 定案：唯一口径 = **`K1-trait`**（伪码在产物 §2，落地 = 任务 **#61**）；**316 退役、318 降级为同口径内的诊断字段 `rawHits`**，两个「总数」身份取消。
⚠ **同一份未入库工件还是 `k1ExternalImplTraitCount = 232`「口径不可核对」的病根**（两个锚点共用它）⇒ #61 要拔的是这个根，**不是给 316 找一个新别名**。

### 17.2 真重复只有 2 组，都在 harness 内部；跨 crate 重复 = 0
全仓同名 `pub trait` 重复 = **3 组**：`ProviderRepository`、`SettingsRepository`（**同 crate 两份定义 = 铁律 4 违规**）、`LlmReasoningProvider`（**同名不同物**，方法交集 0 ⇒ 不可 `pub use` 合并，但必须改名）。放宽到「跨 crate、不同名、方法名交集 ≥2」扫得 17 对候选，**最高 Jaccard 0.43，全是 `get/set/delete` 型 KV 命名的巧合** ⇒ 违规不外溢。

### 17.3 合并方向（定论）
- 权威 = `repositories::ProviderRepository` / `repositories::SettingsRepository`（消费面最大：9 文件 17 点 / 9 文件 21 点，vs facade 侧 3 个 impl 站点），**但错误通道取 typed 的 `core_error::Result`**（AGENTS.md：库 crate 用 thiserror，`String` 是遗留），方法面取**并集**（`ProviderRepository` 5 方法含 `report_key_failure`；`SettingsRepository` 取 3 方法超集）。
- `platform_adapter.rs` 删本地定义、改 `pub use crate::repositories::{…}` ⇒ **`harness::ProviderRepository`（`lib.rs:435` 根 re-export）与 `harness::platform_adapter::…` 两条旧路径不断链**，typed 侧 3 个 impl 站点只改签名不改导入。
- **整文件删除 2 个**：`dao/src/provider_repository.rs`（56 行）、`dao/src/settings_repository.rs`（36 行），逻辑并进 `dao/src/platform_adapter_impl.rs:28 / :49`，注册表改注册它。
- `LlmReasoningProvider`：**ToT 侧改名 `ToTReasoningProvider`**（`agent/src/lib.rs:207-209` 的 re-export 抢占了 `axagent_agent::LlmReasoningProvider`，逼消费方写全限定 `src-tauri/src/init/llm_providers.rs:31` 或起别名），ReAct 侧保原名（5 impl + 2 测试文件不动）。
- 合计 **26 文件 / 305–385 行 / 触 6 根**。

### 17.4 cargo 窗口：最小批数 = 2（按共享 target 锁的串行代价定）
批 1 = 改名（只碰 `axagent-ai` + `src-tauri`；定点 `cargo check -p axagent-agent` + 主 crate check ≈10m 一次）。批 2 = ①+② 合并 ⇒ **必须同批**（两者共用 6 个文件与同一 impl 站点，拆开必现「trait 改了 impl 没改」的中间红态），10m check + 定点 clippy（harness / dao / gateway / rt-workflow，`--all-targets`）。全量 `clippy --workspace --all-targets --all-features`（≈18m）**只在两批都收口后跑一次**。

### 17.5 顺带纠出的既存漂移（不归本批）
同一谓词今天算出 `.rs` = **2006**，而 PLAN:247 与镜像表都钉 **2005** ⇒ `k1RsFileCount` 已漂 +1（= `#53` 新增的 `axagent-plugins/crates/plugins/src/bin/e2e_seam_timeout_worker.rs`）。**trait 数 315 未漂移**。⇒ 等 #53/#59 交锁后与 4 条代码锚点一次 `--repin-code` 收（工序记在 `.tmp/repin-note.md`）。

### 17.6 一条被证伪的旧断言（我已当场改回）
「`ToolContext` 是 tools 侧同名不同物的第二份定义」——**错**。全仓 `pub struct ToolContext` **只有 1 份** = `harness/src/tool.rs:142`；tools 侧走 `axagent-runtime/crates/tools/src/lib.rs:78` 的 `pub use axagent_harness::{… ToolContext …}`，其余（`generated_tool.rs:15/116/127/134`、`mcp_tool_wrapper.rs:9/45`、`orchestration.rs:6`、`registry.rs:2006`、`runtime_mutation.rs:25/107/164/221`；tools + src-tauri 面 **286 处提及 / 482 文件**）**全是消费点**。⇒ 它与 `ToolCallContext`（`plugin_hook.rs:14`）是**近名不同物、两份都在 harness**，**不构成铁律 4 违规、不进合并清单**；§0.7.5 与 §11 第 22 条的措辞已按这一版改。
另点名 4 对「看着像重复但不可合」：`PluginManifest`×2（`plugins/src/types.rs:293` vs `tools/src/plugin_sdk.rs:10` ⇒ 合并会改**已装插件清单的序列化形状**，正确处置是 SDK 侧「消费权威 + 本地投影」，独立任务）· `MarketDataProvider` vs `StockVendor`（J=0.06，是分层）· `Plugin` vs `AxAgentPlugin`（J=0.15）· `PluginLifecycle` trait vs struct（属 #34 零派发族）。

## 18. D1 **收侧已闭合**（2026-11-02，`#53`）⇒ §12 的开工闸门打开

| 项 | 现值 | 位置（行号会漂，认符号） |
|---|---|---|
| 默认 seam 预算 | `DEFAULT_SEAM_TIMEOUT_MS = 15_000` | `plugin-proto/src/lib.rs` 的 `pub const DEFAULT_SEAM_TIMEOUT_MS` |
| 长跑例外 | `AGENT_LOOP_RUN_TURN_TIMEOUT_MS = 180_000`（**是裁定值、非实测分布**，注释已标） | 同文件 |
| 逐 op 预算表 | `SEAM_OP_TIMEOUT_OVERRIDES` + `seam_timeout_ms_for_op()`；**未进表 ⇒ 回落连接 `seam_wait_timeout`（缺省 = 协议常量）**，声明漂移也回落默认（失败方向安全） | 同文件 |
| 取预算位点 | `invoke_with_chain` ⇒ `seam_wait_for_op(op)`，**表值优先** | `plugins/src/worker.rs` |
| UI-action | 同入口 ⇒ **主 crate 零改动**（实测 `commands/plugin.rs:407-409` 确为 `invoker.invoke(OP_UI_ACTION, …)`） | — |
| 超时后处置 | **标脏拒后续** = 宿主本地一个 `AtomicBool` + 两处闸；沿用 #49 语义（撤配对槽位 + 迟到帧按「无等待者」丢弃）⇒ **不引入取消帧/重启/健康探测等新协议机制**。裁定：**接受，不拆** | — |

- ⚠ **这只闭合 D1 的一半**：写侧（`stdin.lock()` + 阻塞写 ⇒ 对端僵死且管道满时**写方持锁卡死，连读线程一起堵**）留给 **#56**；触发条件原文已写进 `worker.rs` 模块头、`exchange_with_timeout` 语义边界、`seam_wait_for_op` 文档**三处**，防止读者误当已闭合。
- ⚠ 本轮新暴露的边角**已由 `#63` 闭合（2026-11-02，交付账见 §27）**：握手 `describe` **不在 seam 公共入口** ⇒ 原为无限等，现走**独立预算** `DEFAULT_HANDSHAKE_TIMEOUT_MS = 10 s`（不进接缝表、不改线格式、版本仍为 1）；超时**只认 `Timeout` 即置脏**，否则装载会从「挂握手」平移成「挂停机」。**写侧仍归 #56**；⚠ 同族残余未闭：`shutdown()` 在连接**未标脏**时仍无限等回执（回了 `describe` 却装死不回 `shutdown` 的 worker 会挂）⇒ 排期见 §27.3。
- ⚠ **本句口径已被 `#59/#53` 对抗复核推翻，订正如下（详证见 §26.2）**：不是「其余 **7 个** op 走 15 s 默认」，而是「**除 `run_turn` 以外的全部 op** 走 15 s」（覆盖表今天只有 1 行，`plugin-proto` 自己的用例就列了 19 个未进表项）；且触发形态比原句**严重一档**——不是「真接入 LLM 型 provider 插件时*可能*误杀」，而是「**只要嵌套一次耗时 ≥15 s 的宿主侧调用就必然误杀，并把健康连接永久标脏**」（SDK 文档亲自示范的 `peer.call_seam` 形态即中招，`chat`/`embed`/`reflect`/`suggest`/`evolve`/`on_message`/`exec` 只是举例）。表已就位、加一行即扩、不必改传输层；但 `run_turn` 那 180 s 本身偏小（宿主 LLM 单请求上限 300 s、工作流 Agent 节点预算 900 s）⇒ 处置与排期见 **#73**。
- 实跑（真实 EXIT）：clippy（proto / plugins）= 0；`cargo test -p axagent-plugin-proto` = 31 passed + doctest 1；`axagent-plugins` lib **142 passed**，且**不带子进程门控也全绿**；带 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 时 seam e2e 真子进程 2.62s passed、#49 的 4 条命令帧 1.67s passed。⚠ **原句推论「⇒ 超时/标脏/预算表用例是 CI 恒跑的护栏，不靠 SKIP 蒙过」是自述过度，已由 `#59/#53` 对抗复核推翻（S-2，详 §26.2 第 2 条）**：无门控恒跑的只是**单元层**那几条；#59 的**端到端形态锁 + 三标计数**全在 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 门控内，而 **CI 从不设该变量 ⇒ 那两条证据在 CI 恒为 SKIP，SKIP ≠ 通过**。处置：要么把该变量接进 CI 对应 job，要么明写「本机证据、CI 不背书」——**不许继续称它们为 CI 护栏**。
- ⚠ 记一笔环境缺陷：本机 UNC 下 `cargo test` 的**运行阶段**复现「exit 0 且零输出吞日志」⇒ 取证一律改成**测试 exe 直跑**（EXIT 为 libtest 真实返回码）。这与 §11 记的「tsc / dprint 在 UNC 静默空跑」同族，但**这次发生在 `cargo test` 上**，是新例。
5. 复刻 build.rs 解析器（(a) 全量版）否掉的实证理由：**「数对了」不等于「口径对了」**——复刻首次漏实现 `pending_cfg.take()` 时条目数照样 1542 对、而 cfg 行数 790 vs 真实 56；第二份解析器会在 build.rs 一改之后静默漂移。

## 19. `#57`/`#58`：K2 键控判据**入库** + 「产物滞后 / 真漂移」分流（2026-11-02；报告全文在派单会话 `475ac4e4`，改动面唯一文件 `scripts/check-capability-register.mjs`，1926 → 2367 行 / +441 −0）

### 19.1 入库了什么

- **八形态 K2 谓词**（①–④ 逐字沿用一次性工件、⑤–⑧ 为 #57 新写）进主脚本，全 export 纯函数、输入为注入的「路径 → 内容」映射。判据原文不再散落在 `.tmp`：强弱档口径登记进镜像表 `countRule.k2Forms`，三处已知偏差登记进 `countRule.k2Caveat`。
- 新锚点 **`k2KeyedFaces` = 强 92 / 弱 7**（`hard:false`，扫描面 318 个 `pub trait`、⑧ 的 impl→trait 反查表 744 个实现类型）。⚠ **刻意不给 `planNumber`**：PLAN 原文无此数，给了就是编造锚点。它与软锚点 `k2TraitFaces`（51 / `check:manual`）**不是同一口径**（样本面从 198 扩到全树、且多两个形态），所以 51 的现值、`check`、`hard` 一字未动 —— **不许借新读数把旧读数洗成绿**。
- 新锚点 **`commandNameSetParity` = 1542**（`hard:true`；#75 后三方同为 **1543**）：登记值 == 产物条目数 == 契约 A 名集合数三方对齐。存在理由见 §15.4 第 3 条的「名集合静默低估」风险（今天 0 例、且**无编译期保证**）。
- **#58 分流**：`commandTotal` 在比登记值**之前**先跑仓内已接线的 `check-contracts.mjs:95-121` 名字集合谓词 ⇒ 判出「滞后」走专码 `ARTIFACT-STALE` 并点名「跑 `cargo check` 重生成后复跑本门」，名集合一致才按**真漂移**判。⚠ **两档都仍判红**，分流只买到诊断正确性，不买到绿（GitHub Actions 里 `ci-check.mjs:114-137` 的 `step()` 非零即中断）。
- 自检：㉑㉒ 存量负控 26 条**一条未删**，新增 ㉓ 19 断言（八形态正负对照）+ ㉔ 13 断言（分流），check 种类覆盖度 16/16 无空洞；单趟读数 21.2 s → 23.0 s。㉔ 里锁死一条「**分流判据不得引用产物行数**」——防的就是拿 `registryFileLines` 的 ±1 口径错配当滞后证据（§15.1 第 5 行那个坑）。

### 19.2 与 `#33` 的 198 条逐条对账 ⇒ 三条结论

1. **⑤ 复现冻结读数成功**：198 内命中 15 条、逐条一致（「冻结有 / 入库版无」= 0）⇒ 老工件的容器形态口径没有失真。失真在别处：**15 条落在桶1 8 / 桶2 2 / 桶3 5** ⇒ 「容器命中 ≠ 能力」在盘上坐实，K2 是**找漏器**不是**判据**。
2. **⑥ 是入库版自己的主噪声源**：新谓词在 198 内命中 66（强 61 / 弱 5），差集 A（命中却被 #33 判「留内核」）= 54 条，其中 **48 条只靠 ⑥**（`set_xxx(Arc<dyn T>)` 形态的全局 DI 注入函数：各 `*Repository`、`SettingsRepository`、`ProviderRegistry` 一族）⇒ ⑥ 按现文本口径把**构造期注入**当运行期注册口。**收窄判据现在就定死：⑥ 只有当接收者是集合类型才算强档，写进单个结构体字段的 setter 不算。** 但本批不动它 —— 改它 = 改口径，与「入库」同批会让读数失去可复现基线，归 #54。
3. **弱档 ≠ 非能力**：弱档 5 条里 `PluginHook` 恰是桶1 的**已建接缝**（宿主容器是 `Vec<Arc<dyn PluginHook>>`）⇒ #54 不许拿「落弱档」当排除判据，人工双问面必须保留。

### 19.3 订正三处旧归因（盘上原文优先于我的转录）

| 条目 | 此前写法 | 盘上实测 | 后果 |
|---|---|---|---|
| `document.parser` | §0.7.4 把 `document-parser/src/lib.rs:533` 的「37 分支 match ext」当键控证据（⑧ factory-match），`res33` §5.1 同此归因 | 那个 match 块是 **ext → MIME 字符串表**，块内**不构造任何实现类型** ⇒ ⑧ 不命中。它进强档靠 `tools/src/parser.rs:16 set_document_parser(Arc<dyn DocumentParser>)`（⑥） | 「37 分支是编译期穷举 ⇒ 加扩展名要改主壳」这条**拆分理由不变**（§0.7.4 该行的结论不动），要改的是**键控证据的位置** |
| 全树容器命中 | 把 198 当「全部候选面」 | 另有 **13 条**容器命中落在 198 样本之外：`ProviderAdapter` / `Tool` / `Agent` / `CommandHandler` / `NodeExecutorTrait` / `AnyNodeState` / `MessagePlatformAdapter` / `VaultSource` / `ContextContributor` / `ReversibleEffect` / `CapabilityPackAdapter` / `CustomRuleChecker` / `CompletionProvider` | ⚠ **不预设「新增 13 条能力」**：其中至少 6 条**已在本表登记**（`tool.set`、`command.handler`、`workflow.node`、`platform.adapter`、`vault.source`、`rule.custom`）⇒ 198 是 315 的子集、不是全集。#54 的第一步是拿这 13 条与本表逐条对账，**只登记真差额**。**⚠ `#54` 对账后本行不完整（2026-11-02 补）**：198 之外的命中总数是 **33 条**，其中含容器形态的恰是这 13 条（本行没错），**另有 20 条非容器形态命中从未进过对账** ⇒ 那 20 条也已判完，净新增同样 **0**（见 **§24.3**）。本行此后以「样本外 33（其中容器 13）」为准。 |
| `k1ExternalImplTraitCount`（232） | 「口径不可核对，只报告」 | 本批未动 | 仍只报告；拔根属 **#61**（census 谓词入库） |

### 19.4 keyed-get 变体漏检：入库版会漏**真**能力

桶1 里 3 条**已建接缝**在 ⑤–⑧ 全不命中：`EmbeddingProvider`、`McpClientService`（keyed-get 的 getter 返具体 DTO、块内不含 `dyn`）、`MemoryProvider`（有 `provider_name()` 自述键，但窗口内无 dyn 集合筛选）。⇒ 新形态**不是**「只漏假阳性」的保守判据，它同样漏真能力 ⇒ 这是「K2 读数必须与 §0.7 登记表**双向**对账、不能单向当权威」的实证，已写进 `countRule.k2Caveat`。

### 19.5 `#54` 的输入齐了 ⇒ 范围现在就钉（不是待议项）

⑥ 收窄 → 重跑入库版读数 → 与 23 条双向对账（含 13 条外溢、3 条 keyed-get 漏检）→ 按 `#33` 复核后的净结论做**分组级联 26 → 23（换成员，不是减成员）**→ 同批改 PLAN §0.7.2–§0.7.7 + 镜像表 `groupCounts` / `capacityRegister` / `capacityPlanPinned` + `memory.provider` 键控列改写 + 新增「裁决列」+ **补登 `rag.vector.store` 一行进 §0.7.7 的 K 组**（§0.12.1 第 693 行的既有义务，至今未执行；`totalRows` 36→37、`groupCounts.K` 9→10、`capacityRegister` **仍 26**）。
**在 #54 出数之前，§0.7 的现分组与 `capacityRegister=26` 一律不动** —— 现盘那三项仍判绿，提前改会造出第二个假红，且会让「改表」与「改判据」两件事在同一批里互相掩护。

## 20. `#46` 的独立审查裁定 + 交 `#64` 的接口需求（2026-11-02；审查稿 `.tmp/rev46-findings.md`，工程师 §6–§7 交回全文在会话 `ff4a6678`）

### 20.1 四条应修：全部接受，已派单（会话 `e8a000ae` / 任务 #67）

| # | 缺陷（审查员实测，非推断） | 裁定 |
|---|---|---|
| 1 | `rpcBridge.ts:58-63 defaultAllowedOrigin()` 在 `tauri:` 下返回宿主 origin，而发送侧（`:134/:188/:284`）把它当 targetOrigin 用 ⇒ 目标文档是 opaque ⇒ **回包被丢弃**。macOS/Linux（`tauri://localhost`）命中；Windows 与浏览器模式落 `"*"` 分支幸免。Chromium 实测：传宿主 origin 沙箱收不到、传 `"*"` 收到 | **改**：把「出向投递 targetOrigin」与「入向来源校验 allowedOrigin」拆成两个概念，出向恒 `"*"`，来源正确性交给应修 4 的 `event.source` 比对。**唯一定义处写一处判据注释**，不许到处复制理由 |
| 2 | 空串错误的成功/失败判定有**两份**：模板运行时真值判定 `if (msg[F.error])`，客户端 `pluginClientBridge.ts:221-226` 与 `callSkillMethod` 按「键在不在」判 ⇒ 同一次失败，bundle 客户端 reject、srcdoc 面板 `resolve(undefined)`。门 D 只测 builder/reader，**测不到模板侧** | **改**：失败判定收成协议层的**唯一谓词**，三处消费全用；并补两条用例（模板产物文本只引用共享谓词 + 空串错误两通道都判失败） |
| 3 | 可携带性门只匹配行首 `import … from` ⇒ 实测植入 `export { invoke } from "@/lib/invoke"`、`await import("@/tauri-apps/api/core")`、`require(…)` **命中全 0**，而 `src/sdk/README.md` 承诺「一添加内部依赖就报红」⇒ **承诺当前为假** | **改**：门补 re-export / 动态 `import()` / `require()` 三形态，再加一条更粗的兜底（禁说明符的**字符串字面量**整体禁止，注释豁免必须逐字可复核）；README 那句话与门的面**逐字对齐**，不留超话。门 A 同步纳入帧字段名、方法名、targetOrigin 常量 |
| 4 | 宿主接收侧**零身份校验**（`SkillSandboxContainer.tsx:265-311`、`rpcBridge.ts:110-118` 只判帧类型；全仓 `event.source` / `event.origin` 校验零命中）⇒ 应用内任一可脚本化 iframe（HtmlPreview / ChartPreview / 远端 iframe）能伪造 `{type:"rpc:request", method:"api.invoke", args:{command:…}}`，借**当前挂载 skill 的权限面**（store/ui 比插件白名单宽得多）执行。既存缺陷，**但本批把它写成了设计依据** | **改**：宿主接收侧补 `event.source === 该沙箱 iframe.contentWindow`，不匹配**直接丢弃且不回包**；协议注释改成如实描述（三层里必须含 source 比对，并点名校验在哪一行、由谁保证）；补一条「伪造 source ⇒ 宿主不调 `invoke`、不回包」的用例，断言打在宿主拒绝上（照 `pluginPageBridge.test.ts:149-164` 的 `sendRawFrame` 形态），不自证 |

### 20.2 出向 `"*"` 的终局裁定：维持，但把口子开在正确的一侧

工程师 §7「风险 2」给了三个窄化选项。裁定：**①② 不采纳，采 ③ 的精神但落点不同。**
- ①② 否掉的实证理由（不是风格偏好）：opaque `srcdoc` 文档的 `location.origin === "null"`，传 `"null"` 是**同步抛 SyntaxError**（`Invalid target origin 'null'`），不是静默丢；旧代码里 `skill:ready` 那处发送被空 `catch` 吞掉 ⇒ 旧症状是「容器收不到 ready、30 s 超时」。⇒ **「page 用宽值 / panel 留窄值」在引擎层面不可行**（审查员 Chromium 实测，与 WebView2 同引擎族）。可行的更窄选项是「生成模板时注入宿主 origin」，但 asset 页拿不到那个值 ⇒ 仍需 `"*"`，代价是**两条通道两套词汇 —— 正是本批要消灭的形态**。
- 出向 `"*"` 的泄露面近零：接收方恒是宿主父窗口，出境字段为 method / command / 业务 args，**不含 token / 凭证字段**。真正会带业务数据出境的是「宿主 → 已导航到远端的 iframe」那一路（= 应修 1 的入向回包方向），与本条不同向。
- **真正的口子在接收侧**：`sandboxFrameProtocol.ts:144-146` 那句「宿主侧守卫不依赖 origin，依赖三层」的前提是「帧确实来自那个沙箱」，而这条前提**今天没有任何一处在校验** ⇒ 收口靠 `event.source`，不靠给 targetOrigin 换个更窄的值。

### 20.3 审查员判「可接受」的，照单采信，其中三条要记账

- **命令面零增长**（可接受 7）：HEAD 的 14 个协议字面量（6 帧类型 + 8 方法名）逐字 ⊆ 新协议模块（探针输出「HEAD 有 / 协议模块无：(空)」）；两条命令名与 HEAD 数组内容一致；`PLUGIN_BRIDGE_COMMANDS` 是**同一对象的再导出**（`toBe` 断言两处）；新客户端只引用 `api.invoke`。
- **「行为不变」的自述不通过两处**（可接受 11）：出向 targetOrigin（工程师已自述）+ 应修 2 的空串判定反转（**未申报**）。⇒ 交付口径从「行为不变」改成 **「协议词汇逐字不变、判定语义收口」**，今后这类「值相同但分支不同」的改动一律按后者申报。
- **不需要另接静态门**（可接受 10）：四道门活在 vitest，随 `npm run test:run` 在 CI 执行（`ci.yml:888`、include 见 `vite.config.ts:294`）；本仓「两处同源注册」规则的对象是 `scripts/check-*.mjs`，测试内守卫的同类先例即 `ci.workflow.test.ts`。⇒ **但反馈环必须记账**：`scripts/ci-check.mjs:510` 把 Vitest 关在 `!quick && !frontendOnly` ⇒ **预推送快速档不跑这几例**，`--frontend-only` 连全量 vitest 都不跑（既存）。这是「本地快绿 ≠ CI 绿」的第三条实例（前两条见 AGENTS.md 的 `fmt --all` / `clippy --all-features`）。

### 20.4 交 `#64`（页面容器本体）的接口需求 —— 工程师 §6 原文登记进 PLAN，不必再翻会话记录

**A. 客户端接入签名**：`createAxAgentPluginClient(options?)`，生产用法**不传参**（缺省投递面 = `window.parent` / `window`），返回 `{ invoke<C extends PluginBridgeCommand>, uiAction(action), executeCommand(args), dispose(), announceReady() }`；`options` 的 `postTarget/messageSource/timeoutMs/targetOrigin` 是**测试注入点，容器不要传**。初始化时机 = bundle 入口模块顶层求值即创建、随即调一次 `announceReady()`；宿主既有生命周期是「收到 `skill:ready` 才建 host 桥 + 发 `mount` + 撤 30 s 超时」⇒ page 若复用这套门，**ready 是唯一开闸信号**。`dispose()` 接在容器卸载路径（与 `bridgeRef.destroy()` / `apiBridge.destroy()` 同一处）。
**B. 为什么这两个调用是硬要求**（不是「建议」）：不 `announceReady()` ⇒ 容器停在 loading、30 s 后落 `skill.timeoutError` + `SkillErrorFallback`，症状是「页面白一块并报错」而不是「桥不通」，双方都会误判；不 `dispose()` ⇒ 沙箱文档内的 `message` 监听器与在途 `setTimeout` 不回收，泄漏按重试次数累计。**两者都不做 ≠ 功能降级，而是静默不可用。**
**C. 容器必须显式做的事**：入向（宿主 → 沙箱）的 `allowedOrigin` 在 `tauri:` 下会丢 opaque 文档的回包 ⇒ **~~容器显式 `createHostApiBridge({ allowedOrigin: "*", … })`~~ ⇒ **本句作废（`#67` 应修 1 已把该 origin 形参整段删除：`grep -n allowedOrigin src/sdk/rpcBridge.ts` 零命中 ⇒ 容器根本没有参数可传）**；`#64` 按盘上口径落成「**入向不传 origin**，来源正确性交给 `event.source === 沙箱 iframe.contentWindow` 比对」（见 §33.3）**（工程师本批未做，理由：属宿主侧且会改 panel/skill 行为，判据不足 ⇒ 现由应修 1/4 收口，#64 按 §20.1 的落点接）。出向恒 `"*"`，**容器不要再改**（`SANDBOX_MESSAGE_TARGET_ORIGIN` 是全仓唯一定义处，改它同时打断 panel 与 skill）。事件面：`emitEvent("theme-change")` 对 bundle 侧无效（无 `window.ctx`、宿主 `permissions.events` 仍为空）⇒ 主题跟随需第三方自接 `message` 帧。
**D. 加第三条命令只改两处**：`src/sdk/pluginBridgeProtocol.ts`（命令名字面量 + 派生清单）+ 后端 `commands/plugin.rs` 的命令面；清单内容**不许写在任何其它文件**（宿主 `pluginSandboxBridge.ts`、`PluginPanelHost` 别名、客户端导出都是同一对象的再导出）。会跟着红：`pluginSandboxBridge.test.ts`、`pluginPageBridge.test.ts`、`PluginPanelHost.bridge.test.ts`、`pluginClientBridge.test.ts`，若有人在消费方手写命令名则门 A 额外红。
**E. 本批未做、移交 #64 的六条**：
1. `pages[]` 列表投影：照 `PluginManager::list_dashboard_plugins()`（逐插件实时 `load_plugin_from_directory`、单包读失败只 warn 跳过、**无第二套 registry**）扩同形 `list_plugin_pages()` + 命令 `plugin_list_pages`（`build.rs` 自动注册；`commands/mod.rs` 只允许 `pub use sub::*`）；前端 `src/types/` 加 `PluginPagesInfoDto`。**禁止**另建路由表 / 平行 manifest / registry 缓存（禁区 4、§0.7.6 的教训）。
2. 容器本体：`plugin_resolve_page_asset(pluginId, pageId)` → `absolutePath` → asset URL → iframe `src`；`sandbox="allow-scripts"` 且**不加** `allow-same-origin`；分支判据**只取 `loading`**（`schema` 走 `DynamicPageViewer` / `useDynamicUIStore`，**绝不调解析命令** —— `manager.rs` 那条路必报错）；`entry` 只用于展示/排错，**禁止参与路径拼接**。
3. `convertFileSrc`：本仓禁业务侧直接 `import "@tauri-apps/api/core"` ⇒ 在 `@/lib/invoke` 增设一个 `assetUrl()` 出口，**不要新开 import 面**（禁区 1）。
4. `src/lib/browserMock.ts`：`plugin_resolve_page_asset` 与将来的 `plugin_list_pages` **都无 case** ⇒ `npm run dev` 浏览器模式下页面通道不可用，需补桩。
5. 运行期人工验证：装一个含 bundle 页的真实插件，逐条确认 ready → mount → 两条命令往返、opaque origin 下回包可达、`assetProtocol.scope` 与 CSP 实际放行、以及**忘记 `announceReady()` 时的报错形态**。
6. i18n：**0 新增**。四类失败按错误码分支，`PLUGIN_PANEL_ASSET_{UNAVAILABLE,BLOCKED,UNPINNED,TAMPERED}` 在 11 语言已全有译文（措辞偏面板）；换 `PLUGIN_PAGE_ASSET_*` 必须先补 11 语言（禁区 2）。

### 20.5 `#64` 的开工闸门

**应修 1–4 落地并交回用例报告，#64 才动容器。** 理由不是排期习惯：#64 在容器里要做的头两件事（`allowedOrigin` 传值、接收侧 `event.source` 校验）**正是应修 1/4 的宿主侧改动** ⇒ 同批开工必然互相覆盖，且会把「谁修好了这条」变成事后考古。
本批仍未覆盖（工程师自报 + 审查员补）：全量 vitest（只跑过定向 557 例 + 桥 5 文件 72 例）、Playwright e2e、真机装载、`oxlint`/`dprint` 仅覆盖改动目录；门 C 对 srcdoc 的提取依赖生成代码里的 `var M = AX_PROTOCOL.m` 别名写法（已加「提取失效不得当通过」的分母断言，但仍是文本耦合，改别名要同步改测试）；客户端 `dispose()` 后不可复用（后续调用一律拒绝）；注入表里的 `timeoutMs` 是无人读的装饰值，本批按「删字段或让运行时真取表值」二选一收口。

## 21. `#51` 的裁定：插件**命令/工具**声明可越出安装根 —— 判「不是漏洞、是未写明的口径」，不收紧（2026-11-02，盘上原文复核）

### 21.1 事实（同一文件里已经有两套相反口径，且都是**有意**的）

| 面 | 解析函数 | 绝对路径 | `../` 越界 | 字面命令（`node` / `python3`） |
|---|---|---|---|---|
| **worker 程序** | `resolve_worker_program`（`manager.rs:2912`） | **拒**（`:2914-2918` `InvalidManifest`） | **拒**（`:2919-2923` 逐 `Component::ParentDir` 判） | 不适用（必须是安装目录内的文件，`:2925` 还验存在性） |
| **命令 / 工具 / hook** | `resolve_hook_entry`（`manager.rs:2895`）+ `is_literal_command`（`:2903`） | **原样保留** | **按安装根 join ⇒ 结果可越出安装根** | **原样交给 PATH 查找** |

⇒ 两套口径的取舍在 `:2907-2911` 的注释里已自述（「worker 是插件自己的二进制，只应来自安装目录 —— 否则声明里一个绝对路径就能让宿主去跑任意可执行文件；hook/tool 是『命令字符串』，可以是系统程序」），**不是遗漏**。#47 修相对路径时明确复用了 `resolve_hook_entry`（`:780-787` 的注释逐字写了三条语义），因此越界形态是**修复时就存在的既定行为**。

### 21.2 为什么「收紧路径」不产生安全增益（这是裁定的实质，不是偏好）

1. **门槛在「能不能起子进程」，不在「起的是哪个路径」**：命令派发最终走 `tool.execute_sandboxed(input, &sandbox)`（`manager.rs:806`），第一行就是 `check_subprocess_permission(sandbox)`（`types.rs:924`；`sandbox.rs:213-217` 的拒绝文案「插件未声明 `subprocess_execution` 权限，禁止调用 shell 执行 hook 脚本」）。⇒ **未声明 `subprocess_execution` 的插件，三种形态一个都跑不起来**；越界路径只在已获得该权限的插件手里可达。
2. **同一权限下，PATH 字面命令这一支本来就允许任意系统可执行文件**：`is_literal_command` 把不带 `./`/`../` 且非绝对的串原样交 `Command::new` ⇒ 声明 `command: "cmd"` / `"sh"` / `"python"` 就已经是「跑系统程序」。⇒ 把绝对路径与 `../` 收进安装根，**拦住的只是「懒得用 PATH 查找的攻击者」**，对一个已握有子进程权限的插件是零增益。
3. **真要收，收的必须是权限面而不是路径**：而权限面刚被 #30 改成真授权（按具体权限名判，`:809-818` 明确否掉了会恒过的 `implies_*`）。路径限制叠上去只会被「改用字面命令」绕过 ⇒ 是**装饰性控制**，与 #30 纠出的「权限档位两条路上都是纯装饰」同类，本 PLAN 已立过判据：**不给装饰性控制付兼容成本**。

### 21.3 裁定与后续动作（三条，全部是「写明」而不是「改语义」）

- **判：不收紧、不单列安全批次。** 越界可达性是 `subprocess_execution` 权限的**直接后果**，不是路径解析的缺陷 ⇒ 风险登记在权限面上（`#51` 关闭）。
- **动作 A（待 manager.rs 交锁）**：在 `resolve_hook_entry` 的文档注释里补上 §21.2 的三条推理（现注释只说了「宽松口径有意不同」，没说「收紧无增益」）⇒ 否则下一个读代码的人会把它当漏洞再修一遍，而那次修改会**静默打断所有以解释器为命令的既存插件**。⚠ 本轮不写：`axagent-plugins/crates/plugins/src/manager.rs` 正被 #59 改（M1 回退），并发编辑同一文件必然互相覆盖。
- **动作 B（真风险另立）**：命令/工具越界**不是**风险，但「子进程以宿主用户身份、无文件系统读写约束地运行」**是** —— `build_sandbox_from_permissions` 只管起不起得来，不管起来之后能碰哪些路径。⇒ 该面归 **#35/#36（股票与 OPC 业务迁出为 L1 插件）**的验收判据：插件一旦能起子进程，它读到的就是宿主进程可见的全部数据（含行情库与凭证落盘位置），**这才是需要立项的边界**，不是 `../`。编号从 #51 移交，不新开号（避免同一件事两个号，见 §20.3 低优先那条的教训）。
- **不动的东西**：`resolve_worker_program` 的严格口径保持（worker 是「宿主主动 spawn 的常驻进程」，与「插件声明一条命令」的信任来源不同，严是应该的）；`assetProtocol.scope` 只放行安装根（`:516-518`）是**页面资产**的面，与本条的**进程面**无关，不要因为本条结论去放松它。

## 22. `#37` + `#40` 交付账（2026-11-02；报告全文 `.tmp/res3740-final.md`，向量三 trait 落 `harness`）

### 22.1 交付面（**同批做**是对的：拆开会把插件 SDK 的公开面定型两次）

| 文件 | +/− | 内容 |
|---|---|---|
| `axagent-core/crates/harness/src/vector_search.rs` | 新增 21 行 | `pub trait VectorSearch` 权威定义，签名与 agent 原定义**逐字一致** |
| `axagent-core/crates/harness/src/vector_store.rs` | 新增 224 行 | 3 trait（Admin 7 / Writer 9 / Reader 9）+ 3 DTO（`EmbeddingRecord` / `VectorSearchResult` / `HnswConfig`）权威定义；模块头登记占位接缝 `rag.vector.store` |
| `axagent-core/crates/harness/src/lib.rs` | +13 | 两模块登记 + 根 re-export |
| `axagent-ai/crates/agent/src/query_engine.rs` | +4/−9 | 原 trait 定义 ⇒ `pub use axagent_harness::VectorSearch;`（照抄本 crate `retry_policy.rs:22` 的先例形态，不断链） |
| `axagent-infra/crates/search/src/vector_store.rs` | +220/−58 | DTO 改 `pub use`；3 个 `impl … for VectorStore` 纯转接块（置于 `#[cfg(test)]` 之前，避开 `items_after_test_module`）；私有 helper 避让改名 |

**「零行为变更」不是自我声明，是可分类的 diff**：纯导入/再导出 · 定义逐字平移 · 纯标识符改名 · 纯新增且不在任何既有调用路径上。`git diff` 里 **0 处 SQL 字符串、0 处 `tokio::spawn`/mutex/`map_err` 路径变化** ⇒ 等价性由「宿主 inherent 未动 + 新 trait 无既有调用者」两条结构性事实保证，不靠测试兜。

### 22.2 分层自检（这是本批唯一有返工风险的判据，逐条给了 grep 原文）

- `harness/src/vector_store.rs`：`axagent|sea_orm|sqlx|DatabaseConnection` 命中 **2 行，全是 `//!` 模块头注释文字**（转述 PLAN 口径用）；代码面只引 `use crate::core_error::Result;` + `async_trait` + serde derive。`vector_search.rs` 同理命中 1 行、也是注释。⇒ **foundation 零依赖这条守住了**。
- 新 trait 全部签名只用 `&str / &[f32] / Vec<f32> / Vec<String> / i64 / Option<&str> / 元组 / harness 内 DTO` ⇒ **一条都不需要「引用化入参」改造**（PLAN K4 里设想的宿主侧 chunk 引用 `rowid: i64` 本来就是原签名）。
- `agent`（consumer）侧：`Cargo.toml` grep `axagent-search` **EXIT=1（零命中）** ⇒ 没有反向依赖实现层，铁律 2 守住。`tools`（hybrid）与 `src-tauri` 的 17 处 `vector_store::` 引用经 search 侧 `pub use` 保活，**断链为零**。

### 22.3 门禁实测（全定点，一条全量都没跑）

`cargo check -p …--all-targets`：harness **0**/54s · search **0**/133s · agent **0**/141s · tools **0**/224s
`cargo clippy -p …--all-targets -- -D warnings`：harness **0**/1m53s（含 1 次 `Blocking waiting for file lock`，对端会话占锁，**未 kill 未 clean**）· search **0**/24.8s · agent **0**/36.4s · tools **0**/36.3s
`__TAURI_WORKSPACE__=true cargo test -p axagent-search`：**0**/200s（lib 186 passed / ngram 3 / pg_integration 1 / doctest 2，0 failed）

### 22.4 本批订正掉的三条 PLAN 错账

1. **`VectorStore` 公开方法 27 → 25、`VectorReader` 11 → 9**（差值 = 构造子 `new` + 自由函数 `register_sqlite_vec_extension`，两者都不在方法面上；`new` 吃 sea-orm 类型**按铁律根本不能进 harness**）。⇒ 已就地改 §0.12.1 那张表，7+9+9=25 自洽。⚠ **这是「PLAN 表被当权威抄了两周」的又一例**：派单时我要求「逐条照 PLAN 那张表划分，不要自己重划」，执行者照做了、并且**报出了对不上**——判据层该被相信的是盘，不是我的表。
2. **trait 方法名的 casing**：原表写 `ensureKeywordIndex` / `ensureVectorIndex`（camelCase）⇒ 本仓 Rust 惯例是 snake_case，clippy `non_snake_case` 会直接红。交付按 `ensure_keyword_index` / `ensure_vector_index` 落。
3. **撞名比原表预想多一处**：`search` 侧本来就有**私有** PG helper 叫 `ensure_vector_index` ⇒ 为避让改名 `ensure_pg_vector_index`（1 定义 + 4 调用点，纯标识符）。**「预留接口」的改名成本要算上同名避让，这条记进 §0.12.1。**
   **未彻底去 fts 字样是有界决定**：`rebuild_fts_index` / `fts_search` 保留 —— **`fts5` 才是 SQLite 方言名，`fts` 是全文检索泛称** ⇒ 我裁定**不另开批次**再洗一遍，要洗就连同调用方一次做完。

### 22.5 registry 影响（两个读数的漂移**全额归因**，不留「无法归因」）

工程师报告 §7 说 2005→2009 里有 **1 个来源它无法归因** ⇒ 我核对完：**没有无法归因的那一个**，是它不知道 2005 这个数的构成。逐笔：
`2005`（已含 `#49` 的 `bin/e2e_command_frame_worker.rs`；同批的 `tests/e2e_command_frame_test.rs` 被扫描面的 `tests` 排除）
**+1** `#53` 的 `bin/e2e_seam_timeout_worker.rs` → 2006（§18 已记）
**+1** `#59` 的 `bin/e2e_legacy_worker.rs` → 2007
**+2** 本批 `harness/vector_search.rs` + `harness/vector_store.rs` → **2009** ✔
`k1PubTraitCount` **315 → 318**：净 +3 = `VectorCollectionAdmin` / `VectorWriter` / `VectorReader`；`VectorSearch` 从 agent 搬到 harness 属**定义搬家、按名去重 ⇒ 净 0**。
⇒ **两个读数的重钉等 `#59` 交锁**（它还在加文件），与 4 处代码锚点位移（`worker.rs` ×3、`manager.rs` ×1、`search/vector_store.rs` ×1）一次 `--repin-code --apply` 收口；工序在 `.tmp/repin-note.md`。

### 22.6 未覆盖（明说，不含糊）

- **主 crate `axagent`（src-tauri）本批未编**（命令约束）。风险点已按 `pub use` 链逐个判过（`llm_wiki.rs:796 impl query_engine::VectorSearch for WikiVectorSearchAdapter`、`indexing.rs` / `commands/*` 的 `axagent_search::vector_store::*`），判定「不破坏」——**但这是推理，不是编译证据**。⇒ 任务 **#68**：一次 `cargo check -p axagent --all-targets` 定点窗口（实测 16m47s 量级），**在 #59 交锁后排**，不与它抢共享 target。
- **注入点未 trait 化**：2 个构造点与全部持有位仍是 `Arc<VectorStore>` / AppState 具体类型 ⇒ 留给兑现 `rag.vector.store` 的批次（要动 wiring）。
- trait 面本身**没加专门单测**（转发体无分支，可测性为零），由 `--all-targets` 的编译性 + `cargo test -p axagent-search` 186 例既有行为锁共同兜。

## 23. `#59` 的 M1 回退交付 + **CI 的 fmt 门禁在 master 上本来就红**（2026-11-02；报告全文在会话 `a451607d`）

### 23.1 交付：回退是**结构性一次性**，不是「重试碰运气」

改动面：`manager.rs`（+118，回退臂在 `:736-776`，判据函数 `command_error_is_unknown_op()` 在 `:2514-2536`）· `worker.rs`（+3，A1 超宽拆行）· `plugin-proto/src/lib.rs`（+2，同上）· 新夹具 `bin/e2e_legacy_worker.rs`（65 行，只实现 `describe`，其余一律回 `PLUGIN_UNKNOWN_OP` 并落 `op=` 标记）· 新 `tests/e2e_legacy_worker_fallback_test.rs`（364 行）。

**「只回退一次」有三类互证，缺一类都可能是自证**：
1. **代码结构**：回退臂是通道一的**唯一出口**，走出 `if let` 后直落通道二并 `return` ⇒ 无环回路径；超时/断裂/绑定不符/命令级失败在上一行直接上抛（**不回退**）。
2. **正向用例**：命令成功 + worker 侧 `op=command` 标记**恰好 1 行** + 脚本标记**恰好 1 行** + warn **恰好 1 条** ⇒ 三者互证没有双跑。
3. **反向锁**：`已进命令分发但命令级失败` ⇒ 脚本标记 **0 行**（已执行的命令**不被重发**，这是副作用保护，不是洁癖）；`错误码位不是 UNKNOWN_OP 但说明文本里含它` ⇒ **不回退**；`超时与非传输错误` ⇒ **不回退**。
4. **错误串不靠推测**（⚠ 本条原句已被对抗复核判为措辞不实，按 §26.2 第 3 条订正）：真旧 worker 子进程经宿主 Display 成形为 `与插件 \`legacy-fb-demo@external\` 的通道错误：PLUGIN_UNKNOWN_OP: 未知操作：command`，但**判据吃的不是这个整串**——它读的是 `WorkerError::Transport { reason }` 的 **`reason` 字段**并按首个 `:` 切段取码（`manager.rs:2528-2536`）；**整串里首个 `:` 前段是中文句子「与插件 … 的通道错误」，不是码** ⇒ 照原句去动文案会**静默关掉回退**（S-1；审查员点名的「最可能被下一个改它的人弄坏的一行」= `worker.rs:1285` 那句 `format!("{code}: {message}")`）。实测串已写死进断言（`e2e_legacy_worker_fallback_test.rs:295-298`），形态漂移时**先红**——⚠ 但该 e2e 在子进程门控内、**CI 恒 SKIP**（见上一条），所以这道防线今天只在本机有效。结构化处置（把 code 提成字段）提进 **#73**。

### 23.2 遗留两条（都属「最小修复」的边界，不是遗漏）

- **回退后的 spawn 通道仍无超时**（#49 已登记的既存遗留）⇒ 旧 worker 插件的命令卡死仍会挂住调用线程。**这是 M1 修完之后仍然存在的真风险**，且它的存在理由是「通道二今天就是阻塞 spawn」，不在本批改动面内。⇒ 归 **#56**（写侧/阻塞面同一病灶），不另立号。
- **判据依赖 `Transport.reason` 的 `{code}: {message}` 文本形态**，没把 code 提成结构化字段 ⇒ 提字段要改协议面，超出「最小修复」。⚠ **这是有意的技术债，且已用「写死实测串」把它变成会先红的债**，不是静默脆弱。

### 23.3 实测发现：**`cargo fmt --all --check` 在 master 上就红**（本仓第一次量化它）

#59 报「21 处 fmt diff，其中 9 处在 `manager.rs` 且与 HEAD 逐字相同」。我独立复跑探针坐实了这 9 处：

```
git show HEAD:axagent-plugins/crates/plugins/src/manager.rs > $TEMP/manager_head.rs
rustfmt --edition 2024 --config-path axagent-plugins/rustfmt.toml --check $TEMP/manager_head.rs
⇒ EXIT=1，`^Diff in` 命中 9（首两例：`:3902` 的 `if cfg!(windows) { "greet.cmd" } else { … }` 单行体必须拆块；`:3957` 的多行调用缩进）
```

三条结论：
1. **既存债，不是本批引入** ⇒ 本批**不顺手修**（#59 的命令里我明令 `manager.rs` 那 9 处不许动）。理由：混进本批会让「哪几行是谁改的」归不了属，而这正是本仓反复出事的形态。**单独一次提交 = 任务 #69**。
2. ⚠ **探针第一次跑错了，结论差点反过来**：不带 `--config-path` 直接把文件丢到 `%TEMP%` 下跑 `rustfmt` ⇒ 报 **139 处** diff，全是 `},` → `}` 这类**配置差异**（本仓 `rustfmt.toml` 逐根存在且有 `match_block_trailing_comma = true` / `fn_params_layout = "Tall"` / `use_small_heuristics = "Max"` 等 8 条非缺省项，且**仓库根没有统一的 `rustfmt.toml`**，8 根各一份）。⇒ **判据：验 fmt 必须带 `--config-path` 指向发起构建那根的 `rustfmt.toml`**，否则量出来的是「配置文件没被读到」，不是代码问题。这条与 AGENTS.md「`[profile.*]` 只在发起构建的那个 workspace 根生效」是同一类「按根分散的配置」陷阱。
3. **CI 为什么一直没报这个**：AGENTS.md 已记「`cargo fmt --check` 不带 `--all` 只检查根 package，`crates/*` 完全不被检查」。CI 补 `--all` 之后这条存量才具备暴露条件 ⇒ 与 §17 的「三读数」和 §21 的「装饰性控制」同族：**门禁的覆盖面本身就是门禁的一部分**，覆盖面变了，既存债会一次性浮出来，必须单独记账而不是塞进功能提交。

### 23.4 本批的 fmt 账（与既存债分开结）

本批（#49/#53/#59/#37/#40 的未提交行）引入 **12 处** 需修：`plugin-proto/src/lib.rs` 9 处（`:422/:463/:474/:1417/:1426/:1462/:1555/:1563/:1570`）+ `worker.rs` 4 处（`:1252/:1479/:3714/:3895`），另有 `bin/e2e_command_frame_worker.rs` 2 / `tests/e2e_command_frame_test.rs` 5 / `e2e_seam_timeout_test.rs` 1（同属本批未提交产物）。⇒ 已派回 #59 会话单独收，验收口径不是「退 0」（`-p axagent-plugins` 里必然还含那 9 处既存债），**而是「本批引入的几处从红项列表消失、剩下的一条不少全是既存债」的差集证明**。

### 23.5 环境缺陷记账（新增一条，与 tsc/dprint 同族）

`__TAURI_WORKSPACE__=true cargo test -p axagent-plugins` 在 UNC 下 **EXIT=0 且输出 0 字节** ⇒ 执行者自己判定「**此绿不作数**」，改用 `cargo test --no-run` 编出 12 个测试 exe 后逐个直跑取 libtest 真实返回码（带门控轮 168 passed / 0 failed；无门控轮 e2e `--nocapture` 复核到 `SKIP:`×2 且 0.00s ⇒ SKIP 没被当成通过）。⇒ **判据升级：本仓所有「跑测试拿证据」的命令，EXIT=0 且日志字节数为 0 的一律判未执行**，不是判通过。这条写进 §15 的读数入库纪律。

### 23.6 `#59` 第二轮（fmt 收口）复核通过 + **我自己当场踩到的第三种假绿**

执行者交回的差集证明，我独立复跑**采信**：`cargo fmt --check -p axagent-plugins` ⇒ 剩余 **9 条** `Diff in`，行号与修前基线里 `manager.rs` 那 9 条逐字相同（`4052/4107/4145/4225/4305/4312/4320/4339/4381`）⇒ 本批 21 条（proto 9 + worker 4 + bin 2 + 两个 test 6）**清零**，既存债**一条未动**。`manager.rs` 未被顺手改的独立证据：`git diff --numstat` 仍是 `220/10`。
**纯格式改动仍做了编译复核**：`cargo check -p axagent-plugin-proto --all-targets` = 0/8s、`-p axagent-plugins --all-targets` = 0/21s（含全部 bin/test target）。

⚠ **但要记一条新的失效形态，它和 §23.5 那条正好相反**：我第一次复验时把命令跑在 `axagent-plugins/` 根下，而 `axagent-plugin-proto` 属 `axagent-core/` 根 ⇒

```
cargo fmt --check -p axagent-plugin-proto        # 在 axagent-plugins/ 根下跑
⇒ EXIT=1，827 字节，内容是「package ... is not a member of the workspace」+ usage
⇒ 而我用的判据是 `grep -c "^Diff in"` ⇒ 输出 0 —— 看起来「零 fmt 债」，实际「根本没检查」
```

⇒ **判据升级（与 §15 的入库纪律同族）**：静态门的读数**不能只看「匹配行数为 0」**，必须同时看 **EXIT + 日志里有没有「工具自述没干活」的字样**（`is not a member of the workspace` / `Skipping` / `no files` / 0 字节的 `cargo test`）。三种假绿现在齐了：
① 退 0 且 0 字节（`cargo test` 运行阶段，UNC 特有）；② 退 0 但列出 0 个文件（`tsc --noEmit` / `dprint`，见 §11）；③ **退非 0 但错误文本不含判据关键词**（本次：根目录选错，包不属于该 workspace）。
⇒ **跨根跑定点命令前先确认包归属哪个根**（8 根各一份 `Cargo.toml`/`rustfmt.toml`，AGENTS.md 已记 `[profile.*]` 与 lint 继承也是按根分散的）。

⚠ 执行者还补了一条我原先没有的线索：**把测试 exe 的 cwd 从 UNC 换到 `%TEMP%` 后直跑，同样「0 字节 + 退 0」** ⇒ 所以 §23.5 推荐的「exe 直跑」取证法本身也有前置条件——**cwd 必须与 cargo 发起构建时一致**，否则换了目录反而触发同一伪装。

## 24. `#54` 的裁决（2026-11-02；对账产物 `.tmp/res54-reconcile.md`，203 行，探针在仓外 `res54\probe{13,2-outside,3-evidence,4,5,6,7}.mjs` 可复跑）

### 24.1 能力面 **26 → 23**，分组 `A 11 / B 8 / C 4`，出组三条

| 出组条目 | 原档 | 盘上判据（对账产物 §3–§4，逐条复核成立） | 去向 |
|---|---|---|---|
| `workflow.node` | B | **K3 不过**，但理由**要改写**（原写「键是 `'static` 字面量 ⇒ 没有注册口」是**错的**）：盘上有公开运行期注册口 `dispatcher.rs:145-148 register_external` + `engine/mod.rs:1235 register_executor`，键在注册时才产生 ⇒ **同二进制内的 crate 今天就能注册**，不可插的是**出进程的插件**。真理由 = `&'static str` 键类型 + `node_type()` 返 `&'static str` + 派发侧对 `WorkflowNode` enum（`harness/src/workflow_types.rs:1366`）穷举 31+ 臂 + `init_builtin` 编译期写死 ⇒ 加第 N+1 种节点必须改 enum + 改 match + 改 init_builtin。⚠ 实现者 **33 里含 1 个 test mod 内的 `TestExecutor`** ⇒ 生产 **32** | K（留内核） |
| `context.engine` | C | 三条全部今天仍成立且**比原判更强**：`impl … ContextEngine for` = **0**、`\bdyn ContextEngine` = **0**、标识符 `ContextEngine` 全树仅 **2** 处（自身声明 + 一句文档注释，剥注释 = 1）⇒ **全仓除自己没有任何引用**；签名与在用 `compact_session.rs:67` 第 3 参换了物（`&CompactionContext` ↔ `&dyn PromptProvider`）。⚠ **判债只针对 trait 本体**：同文件的 `generate_skill_injection:217` / `extract_injected_skills:238` / `should_reinject:257` / `CompactionRecord` 系列**是活的** ⇒ 「判债」≠「该文件可删」 | 债档 |
| `memory.provider` | B | **留 B 的依据在盘上不成立**（这是执行者点名「唯一可能再减 1」的一条，我采纳）：所谓「2 个真外部实现者」= ① `CountingProvider` 在 `#[cfg(test)] mod`（起 `memory_hook_provider.rs:239`）内；② `SkillSummaryProvider` 自述「尚未接线 / 零构造 / 对外不可达」+ `#![allow(dead_code)]`，其 6 个构造点全在 test mod 内。`MemoryProviderRegistry` **不存在**（全树 2 处命中都是「它不存在」的注释）；无键控容器（`Arc<dyn MemoryProvider` 4 处全是单字段/参数）；消费面 `MemoryHookProvider` 的 5 个构造点**全在测试里** ⇒ 生产零构造 | 债档 |

⇒ 组合：`B 10 → 8`（出 `workflow.node`、`memory.provider`）、`C 5 → 4`（出 `context.engine`）、`A 11` 不动 ⇒ **23**。登记表 `totalRows` 36 → **38**（三条移出改为「只登记不计数」并新增 §0.7.9 **债档**，另加 `rag.vector.store` 占位一行进 K，见 §22.5/#54 范围）。

### 24.2 我**推翻**执行者的两条推荐（理由必须同批写下来，否则下一轮还会吵）

1. **`document.parser` 不降 B、能力面不因此 23→22。**
 执行者裁定「⑥ 收窄后它的 K2 证据 = 0，且结构上无法补救（全树只有 1 个 `impl DocumentParser`，⑧ 要求块内构造 ≥2 个实现类型 ⇒ 永远不可能命中）⇒ 移桶」。盘上事实我全采（`lib.rs:533` 是 ext→MIME 字符串表、`lib.rs:61` 的 `match mime_type` 块内无构造分支，两点独立复跑一致）。**但我判它留 B，改的是这一行的措辞而不是桶。**
 **理由**：§19.4 已经立过结论——入库版 K2 **会漏真能力**（`EmbeddingProvider` / `McpClientService` 都是已建接缝却不命中），所以「检测器不命中」从来不是「不是能力」的证据；如果今天因为 ⑧ 结构上不命中就把它踢出 B，等于**让登记表跟着检测器走**，正是 §19.4 明令禁止的方向。它真正成立的判据是 **K3**：加一个新扩展名今天必须改主壳的 `mime_from_extension` + `extract_text`，这一点与检测器命中与否无关，且是 §0.7.4 该行从来的拆分理由。
 ⇒ 动作：该行「准入前提」由「K1+K2 成立」改写为「**K1 成立（外部实现者 1）+ K3 成立；K2 检测器不命中，属已知漏检族第 4 例**」，并把这个形态（**按字符串 `match` 选行为、但块内不构造实现类型**）补进镜像表 `countRule.k2Caveat` 的漏检清单。**执行者那句「采纳收窄却不处理这条 = 留一行假话」是对的，我按它的方式解决假话——改判据位置，不是删条目。**
2. **`AnyNodeState` / `CapabilityPackAdapter` 不翻案，但 §0.7.8 的一句原话必须改。**
 `CapabilityPackAdapter` 盘上活着（3 个实现者 + `capability_pack_orchestration/mod.rs:97` 按 `domain_pack_id` 键控 get），这与 §0.7.8 自述「域包已从产品移除」**冲突**。裁定：**层级口径不变**（L1 功能包 ≠ L2 提供方接缝，域包归 P6.2/#36 的 OPC 迁出），**但剔除理由要改写成「层级归属」而不是「已从产品移除」**——后者是一句会被下一个人拿去当证据的假话，而 14 个 `opc_*`/`capability_pack_*` 命令文件、9887 行就在盘上。
 `AnyNodeState` 判**不是差额**（唯一实现者就是主壳引擎自己的 Typestate 节点状态表示，键 = 节点 ID 的运行期数据、类型集 = 编译期 enum），与 `workflow.node` 同族，按禁区 12 并入不独立成条。

### 24.3 净新增 = 0，且「13 条」不是样本外全集

- **Q1 十三条容器命中的净新增 = 0 条**：10 条「换名未换物」（本表已有行，逐条指到 `model.provider` / `tool.set` / `agent.def` / `command.handler` / `workflow.node` / `platform.adapter` / `vault.source` / `system.prompt` / `reversible.effect` / `rule.custom`）、2 条同族并入、2 条不过 K1。⇒ **26→23 的方向不会被「补漏」推翻。**
- ⚠ **PLAN §19.3 那句「另有 13 条容器命中落在 198 之外」不完整**：198 之外命中总数是 **33 条**，其中含容器形态的恰是 13 条（逐字一致，19.3 没错），**另有 20 条非容器形态从未进过对账**。⇒ 本批补判完，净新增同样 **0**（9 条已有行 / 4 条已裁不单列 / 5 条不过 K1 / 2 条 K2 实质不过：`KitSkillDirs` 的命中点是**借用入参**、`PathEncoder` 是 `get_*` 签名窗口**恰好含 `dyn`** 的巧合）。⇒ §19.3 该行改写为「样本外 33（其中容器 13）」。
- **K2 不是 A 组的证据来源**（§2.2 的新证）：A 组已建接缝 `workflow.sandbox` 的唯一 K2 证据也是 ⑥ 单槽注入，收窄即归零，但它今天**确实是已建接缝**（`"workflow.sandbox"` 字面量在 `capability_registry.rs` 实测存在）。⇒ 单向拿 K2 读数去证实/证伪 A、B 行是方法错误，本 PLAN 已两次抓到（§19.4 keyed-get、本条 ⑥）。

### 24.4 ⑥ 收窄的落法与读数（**推算值，改完必须重跑取真值**）

- 统一误判原因在盘上：`K2_PUSH_RE`（`check-capability-register.mjs:786`）只看「动词 + 3 行窗口内出现 `dyn T`」，**不看赋值目标** ⇒ 本仓全局 DI 的写法恰好是 `set_xxx(Arc<dyn T>)`，`push-register` 命中 **104 次**里绝大多数是构造期注入口。十条样本的接收者原文全部是**单槽**（`OnceLock<RwLock<Option<…>>>::replace`、`*guard = Some(x)`、`static PARSER.set()`）或**trait 方法声明本身**（无函数体）。
- 收窄口径（我定死）：**⑥ 只有当接收者是集合类型才算强档**，机械判据 = 站点及其后 5 行内出现 `.insert(` / `.push(` / `.push_back(` / `.push_front(` / `.extend(` / `.entry(` / `vec![` 之一；写进单个结构体字段/`Option`/`OnceLock` 一律**不计**。
- **推算 vs 真值（`#66` 06:42 交回：门内 `--dump-readings` EXIT=0、200.2 s、`walkFiles=10292`）**：我原推算「54 条整条掉出 + 2 条降弱档 ⇒ 强 92 → 36、弱 7 → 9」**差 1**，实测 **强 37 / 弱 8** ⇒ 36/9 作废、**表里写 37/8**。差值出在同一条 `CompletionProvider` 上：推算把它归入「只剩 ⑥ ⇒ 降弱」，但**它那条 ⑥ 站点的接收者确实是集合** —— `runtime/src/shell_completer.rs:144 add_provider(Box<dyn CompletionProvider + Send + Sync>)` 内 `:146 providers.push(provider)`，而 `providers` 是 `custom_providers: Arc<RwLock<Vec<_>>>` 的写守卫（守卫取自 `:145 self.custom_providers.write().await`；该字段是 `Vec`，两处构造行 `:130` 与 `:139` 均为 `custom_providers: Arc::new(RwLock::new(Vec::new()))`，行号取自 `grep -n`）⇒ **⑥ 判「强」不变**；真正降弱的只有 `UnifiedKnowledgeSource` 一条 ⇒ 掉出 **55** 条、降级 **1** 条。⚠ 这条差值不是精度问题，它同时裁定了「集合接收者」的口径边界：**顺序型容器（`Vec`）算集合、单槽（`Option`/`OnceLock`/结构体字段）不算**，与我 §24.4 定的判据一致 ⇒ `k2KeyedFaces` 现登记的 92 属既存**软锚点**（`hard:false` ⇒ 只报告，差 −55 不叫停），**由我在 #73/#74 交锁后随一次干净 `--dump-readings` 统一重钉为 37/8**（那两批若在测试里新增 `Vec<dyn …>` 的 push 站点，这两个数还会动）。

### 24.5 顺带抓到的一处口径缺口（影响多行实现者读数，本批一起处理）

`countRule.implementors` 的测试替身剔除**只看文件路径**（`test_support|mock_|/bin/|e2e_|_fixture`），**看不见内联 `#[cfg(test)] mod`** ⇒ 已确证受影响的登记值：`workflow.node` 33 → 生产 **32**、`memory.provider` 2 → 生产 **1**（且那 1 个还是零构造，见 §24.1）、`EmbeddingProvider` 值对但路径不同（机械口径剔 harness 时顺带剔对了）。
⇒ 动作：给实现者读数**加一列「其中在 `#[cfg(test)]` 块内」**（判据可机械检：同文件内找最近一个未闭合的 `#[cfg(test)] mod`），不改 `implTraitOf` 本体。
⚠ 另一条**本批不修**：`platform.adapter` 的 10 里含 `MockPlatformAdapter@src-tauri/src/init/notification_adapters.rs:150` —— 它在**生产路径文件**里，只是类型名带 Mock ⇒ 路径口径对它无感。我没判它「登记错」（10 逐字复现了口径），但这是一条既存口径瑕疵，修它 = 改实现者定义 = 又要动全表 ⇒ 另立 **#71**。⇒ **⚠ 本句的「路径口径对它无感」已被 `#66` 实跑推翻，订正见 §29.3**：它**同时就在该文件的 `#[cfg(test)] mod`（区间 `[{127,308}]`）里面** ⇒ 新加的 test 列**能机械指认它**（`platform.adapter` 的 10 里唯一那 1 个 test 块内站点就是它，其余 9 个 = 8 个真平台适配器 + `RemotePlatformAdapter@axagent-plugins/crates/plugins/src/worker.rs:2420`）⇒ **#71 不必再造类型名定位器，直接接这一列**；但「本批不修」的结论**不变**（剔它会重算全表实现者数，属 #54 结构级联）——**错的是我给的理据，不是裁决**。

## 25. `#67`（= §20.1 四条应修）交付与我采纳的四项自裁（2026-11-02；报告 `.tmp/fix46-report.md`，304 行；基线冻结 `.tmp/fix46-baseline/*.before.*` + md5）

### 25.1 四条都改了，且「帧词汇零变更」是**取值集合对账**不是全文 diff

探针 `.tmp/fix46-vocab.mjs`：基线 40 项取值 / 4 标量 ⇒ 当前 **40 项 / 5 标量**，「仅基线有」「仅当前有」两侧均为**空**；唯一新增标量 = `SANDBOX_RPC_FAILURE_ERROR_TYPE = "string"`（应修 2 要求的单点判据，**不是帧词汇**）。六张表逐字未动。
⇒ **我独立在盘上复核了两处「应该消失的东西确实消失了」**：`defaultAllowedOrigin` 全 `src/**` 只剩测试里的**反断言**（`sandboxFrameProtocol.test.ts:623 expect(code).not.toContain(...)`）；裸真值判定 `if (msg[F.error])` 只剩注释与门的反断言（`:279`）。生产代码零残留。

| 应修 | 落点 | 关键判据 |
|---|---|---|
| 1 出向 targetOrigin | `rpcBridge.ts` 的 `defaultAllowedOrigin()` 与 origin 形参**整段删除**，三个投递点恒取 `SANDBOX_MESSAGE_TARGET_ORIGIN`；判据只写在协议表旁一处（`:151-175`），其它文件只引用 | 新增**机械护栏**门 F：`.postMessage(` 调用点数 == 取协议常量点数（且 ≥3），且文件内不出现 `location.origin` |
| 2 空串错误判定两份 | 协议层新增 `isFailureFrame()`（类型守卫）+ 常量值；`readRpcResponseError` 改为**委托**它 ⇒ 两侧长不出第二判据；srcdoc 侧（不能 import 的那一侧）吃注入表的**值**自写 `axIsFailure(msg)` | 审查员点名的「门 D 测不到模板侧」补上了：**产物真跑**（门 E，假 `window`/`document` + `new Function`）断空串错误 ⇒ reject 且 `message === ""`；文本门 B 反断言模板里既无裸真值、也无把 `"string"` 写死的第二份 |
| 3 可携带性门漏检 | 检测面从 2 种扩到 **5 种形态**（静态 import 含 type / 副作用 / re-export 三形态 / 动态 `import()` / `require()`）+ **被禁说明符字面串兜底** | 加了**检测面自证**用例：审查员实测漏检的三种形态逐个点名必须命中、合法形态不误伤 ⇒ 「正则写偏 + 消费方恰好没写」不再能静默长绿；`README:123-131` 的承诺与门的实际覆盖面逐字对齐 |
| 4 入向零身份校验 | 协议层新增 `isFrameFromSandboxSource(event, sandboxWindow)`；三个接收点 + 容器两处补校验；**`handleRpcRequest` 签名改成收事件** | 签名改形态是刻意的：将来 page 容器接同一条桥时**没法忘记校验**（把判据长在接口上，不是长在文档上）。用例打在宿主拒绝上：帧内容完全合法、只把 source 换成 `foreignWindow` ⇒ 不调 IPC、不回包；再加一条「同一帧换回本沙箱 source 照常执行」防「一律拒绝」假绿 |

### 25.2 执行者自裁的四项，我逐条表态（都**接受**）

1. **被禁说明符门的面 = 三件套本身**（不是「SDK 目录下除测试外所有文件」）⇒ **接受**。`rpcBridge.ts` 合法 `import { logIpcError } from "@/lib/invoke"`（禁区 1 要求的唯一出口），把它扫进去就是让门守错东西：这条门守的是「第三方装不装得出来」。
2. **`isFrameFromSandboxSource` 在期望来源为 `null/undefined` 时一帧都不认** ⇒ **接受，且这是本批最容易漏的一条**。不写它，`null === null` 判真 ⇒ **沙箱还没挂上反而放行外来帧**，正是要防的形态反过来被防住了。
3. **注入表新增 `failureErrorType`** ⇒ **接受**。替代写法（模板侧判 `F.error in msg`）会把**成功帧判成失败**——srcdoc 的 `sendResponse` 恒带该键、成功时值为 `undefined`；这条边界已被门 D 单独锁住（`error` 键存在但值 `undefined` ⇒ 不算失败）。
4. **容器级行为用例没补**（要渲染整组件 + jsdom 事件派发）⇒ **接受**。安全边界已长在桥自己身上并被用例打在宿主拒绝上，容器那两层是 defense-in-depth；但**这条是未覆盖项，不许当已完成**（见 25.3）。

### 25.3 本批留下的三条真缺口（各归各位，不混进本批）

- **沙箱收宿主帧那一侧仍是 origin 判定**（`sandboxTemplate.ts:175-183` 的 `isValidOrigin`）~~，而 opaque 文档判不了身份 ⇒ 与宿主侧的 `source` 强判据**不对等**~~ ⇒ **本句已被 `#72` 推翻（2026-11-02）**：现判 **source**（origin 只在非 opaque 文档时作为附加一层），且上述行号已漂；`#64` 已按此落地并补用例。⚠ **残余面**（`pluginClientBridge.ts` 的 `handleMessage` 仍只按 `callId` 认领、不判 `event.source` ⇒ 沙箱内嵌文档可用猜到的 `callId` 提前 settle）⇒ 并入 **#78**，与「`dispose()` 后不可复用」「门 C 的 `var M = AX_PROTOCOL.m` 文本耦合」同批做（三者都要动 vendored bundle 客户端或其契约）。
- **执行者未跑的，我已补跑其中一条**：`oxlint` ⇒ 仓根确实**没有任何 oxlint 配置**（`oxlintrc.json`/`.oxlintrc.json` 零命中，`package.json:85` 的命令是逐字 `oxlint src --max-warnings=0` ⇒ 跑的是默认 96 条规则），所以我绕开 UNC 上的 `npm` 直接调 `node_modules/.bin/oxlint` ⇒ **EXIT=0，Found 0 warnings and 0 errors，1432 files / 17.0s**。**仍**未跑：Playwright e2e、全量 `npm run test:run`（按硬约束不跑）——这两条在收口时按 CI 口径补一次，不是本批义务。
- **未提交**：本批 10 个文件未 stage。⚠ 执行者报了一条并发风险：它会话内多次出现「Read 返回的内容 ≠ 磁盘内容」（同一文件两次读出行号/正文不一致），其处置是全部改走**磁盘直读核验**（`git diff --no-index` + node 字节级 + 测试/tsc 跑磁盘文件）。⇒ 这条环境缺陷与 §23.5/§23.6 同族，**记为读数取证的一等判据**：本仓的证据必须来自字节级磁盘读，工具视图不可单独采信。

## 26. `#59` + `#53` 的独立对抗复核结果与我的处置（2026-11-02；审查稿 `.tmp/rev59-findings.md`，252 行，只读、未跑 cargo）

**复核总判**：`回退只发生一次` 在**宿主侧结构上成立**（`manager.rs` 三臂穷举：`Ok` 出函数、`非 UNKNOWN_OP` 出函数不回退、`UNKNOWN_OP` 无 `return/?/break` 直落通道二，回路不可达）；**但主张的另一半不成立**，且查出 **4 条必修 + 6 条应修**，其中 **M-3 是我自己派单时假设错了方向**（见 26.2）。

### 26.1 必修四条，我逐条给处置（不留「二选一都不选」）

| 号 | 缺陷（盘上原文撑着） | 我的处置 |
|---|---|---|
| **M-1** | `manager.rs:644-645` 契约文档承诺「worker 中途异常退出等『声明了 worker 但当前没在跑』的状态也退回原通道 —— 宁可用旧语义，不给命令凭空消失」。**盘上没有这条路**：`loaded_workers` 只在 enable 成功时 insert、`unload_plugin` 时 remove ⇒ **子进程自己死了，表项还在**，命令走通道一拿到 transport 错误、不命中 UNKNOWN_OP ⇒ **凭空失败**。⇒ 这就是 **M1 的失败形态换了个触发条件**，#59 只修了 UNKNOWN_OP 那一支 | **判 #59 未收口，重开**：实现「worker 已死 ⇒ 回退」（连接关闭/子进程退出这一类错误进回退判据），而不是改文档说假话。⚠ 审查员提的唯一残余双跑入口（A1.3：worker **执行完**命令却把 `PLUGIN_UNKNOWN_OP` 当 Fault.code 回出来）与此同判据面 ⇒ 加回退必须同时锁「已执行不回退」的反向用例 |
| **M-2** | 「标脏」是**永不复位**的连接级终局标记：全文件只有 `store(true)`、两处，无任何 `store(false)`；注释自述「无复位口（有意），随 `WorkerInner` 生灭，下一次 `load` 是新实例」——**但盘上没有任何调用方会重启**（上层门面拿到的是 `Result<Value, String>`，一律原样上抛）；拒次文案**不含任何处置指引**，且该串在**三张错误码表里查无此码**（`commands/error_code.rs`、`harness/error_codes.rs`、前端 11 语言） | 必修：**加复位口 + 加错误码 + 文案给指引**。复位口径我定：闸跟着 `LoadedPlugin` 的生命周期走，**显式 `unload`/`disable` 即清**（新 `load` 本就是新实例），同时把「禁用再启用该插件」写进拒绝文案并落一个既有错误码（**不新增码** ⇒ 免 11 语言成本，与 #63 同一条约束） |
| **M-3** | **嵌套调用会确定性误杀**（我派单时的假设是错的，见 26.2）：SDK 文档亲自示范 `peer.call_seam(...)` 的形态（`plugin-proto/src/lib.rs:31-44`），而**入站请求在读线程上同步 `block_on` 处理并就地回写** ⇒ 宿主侧那一跳（如 LLM 调用，宿主 HTTP 上限 300 s）占住读线程 ⇒ 外层出向 15 s 先超时 ⇒ 撤配对槽位 + **置终局闸** ⇒ 内层答案回来后无槽位被丢弃 ⇒ **此后该插件全部新接缝调用永久被拒**。任何声明 `calls: [...]` 的插件都中招，不是 provider 专属 | 必修，且这是 **#73 的主项**。两个动作：① **读线程不得在自身上 `block_on` 入站请求**（把入站处理交给独立任务，保帧顺序的做法只在同插件同 peer 内串行，不跨请求全局串行）；② 嵌套形态必须有一条**用例**覆盖 —— 审查员确认**现有测试面对这条链完全没覆盖**（静态可判），这必须是 #73 新增的第一条用例 |
| **M-4** | `run_turn = 180_000` 是「偏小到会误杀的错值」，不是「安全的拍脑袋」：宿主自己的 LLM 客户端单请求上限 **300 s**（`providers/src/lib.rs:278`）、生产工作流给一个 Agent 节点的预算 **900 s**（附实证「TTFB 偶发 >120 s」），而 `run_turn` 的真实消费方就在生产（⚠ **`#74` 纠出我此处引用不成立**：该文件真实路径是 `axagent-runtime/crates/runtime/src/work_engine/executors/agent_executor.rs`，`:270 runner.run_turn(request)`、`:433` 注册表取 runner，**但文件里既无 `NODE_BUDGET` 也无 900 字面量** ⇒ **900 s 的出处不在这里**，在 `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:4515` + `:4509`（附 TTFB 偶发 >120 s 实证）、`harness/src/capability_pack_orchestration/subgraph.rs:145`（900）与 `:94`（600）、`commands/stock_analysis_setup/seed_variables.rs:202`（缺省 600））且 `max_tool_rounds: None` ⇒ 回合时长**没有秒级上界**（⚠ **`#74` 复核订正了我这句**：宿主适配器 `src-tauri/src/init/agent_turn_adapter.rs:199-207` **不把** `request.max_tool_rounds` 透传给 runtime，真正的兜底来自 `runtime-core` 的 `max_iterations` 缺省 **50**（`axagent-runtime/crates/runtime-core/src/conversation.rs:262`）⇒ 严格说是「50 轮 × 单请求 300 s ⇒ 无秒级上界」，**不是「无迭代上界」**） | **✔ 已做（`#74`）**：`run_turn` 由 `180_000` 改 **`900_000`**，注释按「取 `max(LLM 单请求上限 300 s, 工作流 Agent 节点预算 900 s)`」逐字点名两个来源 ⇒ **「无实测的裁定值」这个标签已摘**（现在是「有出处的 max」，交付账见 §30）。⚠ 今天出向 `run_turn` 流量仍为 0 ⇒ 尚未造成实际损害，但这条不再是「拍脑袋」。`exec` 的 60 s 对齐**本批未做**（撞既有测试口径，见 §30.2）⇒ 另立 **#76** |

### 26.2 **我写进 PLAN 的两句话被复核推翻，就地订正**

1. **§18 的「其余 7 个 op 现走 15 s 默认」口径错**：盘上是「**除 `run_turn` 以外的全部 op** 走 15 s」（`SEAM_OP_TIMEOUT_OVERRIDES` 今天只有 1 行，plugin-proto 自己的用例就列了 19 个未进表项）。⇒ 措辞改为「1 个进表 / 其余全部走默认」，并把 §18 那句「真接入 LLM 型 provider 插件时**可能**误杀」升级为 M-3 的「**只要嵌套一次**耗时 ≥15 s 的宿主侧调用就必然误杀，且把健康连接判死」。
2. **§18 的「不带子进程门控也全绿 ⇒ 超时/标脏/预算表用例是 CI 恒跑的护栏，不靠 SKIP 蒙过」是自述过度**（审查员必答③：推理方向反了一半）：无门控跑的是**单元层**那几条；而 #59 的**形态锁 + 三标计数**全在 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 门控内，**CI 从不设该变量 ⇒ 这两条证据在 CI 恒为 SKIP**。⇒ 订正为分层表述：**单元层恒跑 / 端到端形态锁门控内、CI 恒 SKIP**；后者要么把变量接进 CI 那一步，要么明写「本机证据，CI 不背书」（不许继续写成「护栏」）。
3. **§23.1 第 4 点也错**（S-5）：我把** Display 整串**当成判据输入。实测成形串是 `与插件 \`x\` 的通道错误：PLUGIN_UNKNOWN_OP: 未知操作：command`，**首个 `:` 前段是中文句子、不是码**；判据吃的是 `Transport.reason` 字段（不是 Display 整串）。⇒ 那句「首个 `:` 前段正是 `PLUGIN_UNKNOWN_OP`」必须改，否则下一个人照这句话去改文案就会**静默关掉回退**（S-1：在 `worker.rs:1285` 的 `format!("{code}: {message}")` 前加任何前缀，首段就不再是码 ⇒ **M1 复发**，且唯一防线是一条门控内 e2e 的写死串）。
   ⇒ **审查员对「最可能被下一个人弄坏的是哪一行」的回答我照抄进 PLAN**：`worker.rs:1285`。处置（S-1）：命令码通道的错误**必须带结构化 code 字段**（`WorkerError` 上加 `code`，判据读字段不读文本）——这是 #49/#53 早就该收的债，本批把它提进 #73。

### 26.3 应修里另三条的处置

- **S-3**（`execute_command` 不读 `seam_suspect`）⇒ **随 M-3 一起修**：已实证装死的连接上，每条命名命令仍各自等满 `timeout_ms + 1 s`（默认 31 s，**不是 15 s**）且可无限重复 ⇒ #53 的止血对命令通道完全不生效。⚠ 注意这与 M-1 方向相反（一个要「脏了别再发」，一个要「死了要回退」）⇒ **判据必须分开写**：「worker 进程不在」= 回退；「worker 在但曾超时」= 拒绝并复位。这两件事混在一个 bool 里就是 M-2/M-3 的根。
- **S-4**（回退的**原则化措辞宽于实现**：注释说回退只发生在「派发前被拒、无副作用」的错误上，但同为「派发前 + 零副作用」的 drain 拒绝与 pluginId 绑定不符**不回退**）⇒ 改注释为穷举式（列明哪三类回退、哪三类不回退），**不要为了符合原则去放宽回退面**。
- **S-6**（同一「插件工具」两条通道预算差 4 倍：spawn/沙箱 60 s、远程 `exec` 15 s；`send`/`dispatch`/`start`/`stop` 被归入「毫秒~秒级 ⇒ 余量充足」**而无实测**）⇒ 随 M-4 一起做**同源**：`exec` 与 spawn 通道的 60 s 对齐，其余进表的判据必须是**盘上实测或明确来源**，「看起来快」不算。
- **S-2**（门控 e2e 在 CI 恒 SKIP）见 26.2 第 2 条，与 §18 订正同批。

### 26.4 「可接受」档我全部照单采信（记下来免得下轮重复怀疑）

回退臂结构一次性 · 超时/断裂/形状非法/绑定不符/命令级失败一律不回退 · 判据 fail-closed（缺省填 `SEAM_EXECUTE_FAILED`、空串不命中、中文句子首段不命中 ⇒ **形态漂移一律退成「报错」而不是「重跑」**）· 回退不绕过授权（`check_command_gate` 与 `inputSchema` 都在选通道之前）· 旧 worker 装死等 31 s 是 **#49 既存语义**、不归 #59（归 #56）· 占位串命令回退后劣化成「找不到程序」的 OS 错误（warn 已留痕，记一笔）。

### 26.5 静态不可判的四条（不许当已定论）

`N-1` 真实存量旧 worker 是否**一律**按 SDK 模板对未知 op 回 UNKNOWN_OP（决定 A1.3 那条残余双跑入口的现实概率）· `N-2` 用户机上是否已有插件声明这些接缝（决定「今天零流量」在真实装机上是否成立，要读 config home，不在仓内）· `N-3` 门控 e2e 与 M-3 嵌套链**审查员没实跑**（派单禁 cargo）· `N-4` 15 s / 31 s 的真实收口时刻。

⇒ 处置：`#73` 开工时 **N-3 的嵌套链必须先补用例再谈修复**（先让红出现）；`N-1`/`N-2` 属盘外信息，登记为**已知不确定**，不冒充结论。

## 27. `#63` 交付账：握手 `describe` 的有界等待（2026-11-02；报告 `.tmp/res63-report.md`，195 行；会话 `a111dd5c`；§18 的边角句已就地订正）

### 27.1 改动面与逐条硬要求的落点

| 硬要求 | 落点（原文级） | 实测证据 |
|---|---|---|
| **独立预算常量**（不拿接缝表凑数） | `plugin-proto`：`pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 10_000;`，插在 `seam_timeout_ms_for_op()` 之后、`CommandFrame` 之前；文档点名「接缝 = 用户发起的调用 / 握手 = 装载前置」两条通道**不许互借** | `seam_timeout_ms_for_op("describe")` **仍返回 `None`**；既有测试 `seam_timeout_override_table_is_evidence_bounded` **一字未动**、复跑绿 |
| **不改线格式** | `AXAGENT_PLUGIN_PROTO_VERSION` 仍为 **1**，`FrameRequest` 零新字段（预算是宿主侧等待策略，不进帧） | plugin-proto 用例基线 31 → **32 passed**（新增 `handshake_budget_is_a_separate_constant_and_not_a_seam_table_entry`） |
| **失败可定位 + 不新增码** | `WorkerError::Handshake{reason}` 内嵌 `Timeout` 的 Display ⇒ 串含「握手失败 + `describe` + 上限毫秒数」三要素，不是笼统 `Transport`；「为什么不新增码」写进 `Handshake` 文档（禁区 2 ⇒ 免 11 语言同步义务） | 形如 `插件 <id> 握手失败：插件 <id> 的 \`describe\` 超过宿主等待上限 300 ms，调用已放弃` |
| **测试必须证「有界」** | 装死夹具 `plugins/src/bin/e2e_no_describe_worker.rs`（48 行，装死 20 000 ms）+ `plugins/tests/e2e_handshake_timeout_test.rs`（199 行）；预算由测试用 `with_handshake_timeout` 调到 **300 ms**，**产品常量未为测试缩小** | 带门控逐条取证：`e2e_load_fails_within_handshake_budget_on_silent_worker` **1.16 s** 收口 EXIT=0（夹具装死 20 s ⇒ 「有界」是实测数不是推理）· `e2e_handshake_timeout_does_not_block_the_next_load` 1.12 s；**不带门控 e2e = `2 passed` 但判 SKIP 不判通过**；lib 单元层不带门控 `146 passed`（恒跑护栏含新握手用例） |
| **新字段的波及面** | `PluginWorkerConfig::handshake_timeout` + builder `with_handshake_timeout` → `WorkerInner::handshake_timeout` → `spawn` 装配；**接缝与握手是两个独立旋钮**，互不串写（有用例钉） | 实测非推理：`grep -rl PluginWorkerConfig --include=*.rs` 在 `src-tauri`/`axagent-ai`/`axagent-kit`/`axagent-infra`/`axagent-quant`/`axagent-core` **六根全部零命中** ⇒ 构造面只在 `axagent-plugins` 内，唯一全字段结构体字面量 `plugins/src/manager.rs:1055` 用 `..PluginWorkerConfig::new(...)` 兜底 ⇒ 加字段**不破任何调用方** |

行数账：`plugin-proto/src/lib.rs` 1596 → **1643**（+47）· `plugins/src/worker.rs` 3939 → **4096**（+157）· 新文件 2 个（48 行 / 199 行）。13 条命令**全 EXIT=0 且逐条配日志字节数**，无一条「0 字节 + 退 0」；`cargo check`/`clippy` 日志内均含 `Checking <目标包>` 原文；带门控那轮走**测试 exe 直跑**且 cwd 与 cargo 发起处一致（`axagent-plugins/`）。fmt 只用只读探针（`rustfmt --check --config-path`）⇒ 4 文件 `^Diff in` 命中 **0**，本批零新增 fmt 债。

### 27.2 本批唯一的处置面改动：握手超时**也置脏**，以及为什么必须
`LoadedPlugin::load` 的失败路径调 `invoker.shutdown()`，而 `shutdown()` **只在未标脏时**才发停机帧并**无限等**它的回执（`exchange(..., None)`）⇒ 若只把握手改成有界而不置脏，症状从「挂握手」平移成「挂停机」，同一病灶换个位置、D1 没被补上。**置脏在这里买的不是「拒后续接缝调用」**（那条连接根本没交付出去），**买的是急停路径**。判据与接缝完全同源：**只认 `WorkerError::Timeout`**；`Transport`（对端明确回了错）与 `Declaration`（声明非法）**不置脏** —— 「装死」与「回了错」是两种可观测性。
⇒ 盘上后果（**我今日 grep 原文**）：`seam_suspect.store(true)` = **2 处**（`axagent-plugins/crates/plugins/src/worker.rs:1178` = `handshake` 的 `Timeout` 分支、本批新增；`:1559` = `invoke_with_chain`）· `load` = **2 处**（`:1383` 停机闸、`:1538` 接缝闸）· 构造期初始化 `false` = 2 处（`:1074`/`:1119`）。⚠ **计数口径冲突登记**：§26.1 的 M-2 行写「全文件只有 `store(true)`、**两处**」是**在 #63 之前**写的 —— 若当时确为两处，则握手写点是**第三处**、本批使总数变 3；若当时实为一处（#63 自报「现在仍是 2 写」倾向此说，但「仍是」二字是引用 §26 的旧措辞、不是它的实测），则现状 2 才自洽。**两种解释我都无法从当前工作树证实**（`git diff` 相对 HEAD 把 #49/#53/#59/#63 的四批未提交行混成一坨，切不开）。⇒ 不由 PLAN 裁定，**交 #73 一并澄清**：它要做复位口，本来就得逐写点列表；届时以盘上写点清单为准回写 §26.1 与 §27.2 两处。
⇒ **标脏不会永久拒后续调用**（正面回答，非推理）：装载失败 ⇒ `load` 返 `Err` ⇒ 局部 `Arc<WorkerInvoker>` drop ⇒ `Drop` 再兜一层 `kill_child` + 关 `stdin` + `join_reader`，带脏闸的连接连同进程一起消失；`start_plugin_worker` 只在 `Ok` 后 `loaded_workers.insert`，失败 invoker 不进任何表 ⇒ 下次启用拿到**全新进程 + 全新 `WorkerInner`（`seam_suspect=false`）**，由用例 11 实测「先装死失败、紧接着装另一个真 worker 成功并干净卸载」。无半挂状态：握手失败发生在任何能力注册**之前**（顺序 = spawn → handshake → 逐条 `register_remote_seam`）。
⚠ §18 表格里「超时后处置 = 标脏拒后续 | 一个 `AtomicBool` + 两处闸」那行的**适用范围现在多了握手一栏**，但**本 PLAN 暂不改那行** —— #73 的 M-2 要给这个 bool 加复位口并按「进程不在 = 回退 / 曾在超时 = 拒绝」拆判据，现在改会被下一批覆盖。

### 27.3 执行者交回的两个判断题，我裁定（不留「二选一都不选」）
- **握手预算要不要进 registry / 做用户可见配置** ⇒ **不进**。今天唯一覆盖口是 `PluginWorkerConfig`，调用方 `plugins/src/manager.rs:1055` 一律用缺省 10 s；慢启动是**插件作者自己的属性**（他能在 worker 侧控），把它做成用户可见项会诱导用户去调一个**无实测分布**的数。评审判点 = P5.0a 首个真插件冷启动数据到手时（与下面 27.4 的标注同一时刻回收）。
- **`shutdown()` 的无限等**（回了 `describe` 却装死不回 `shutdown` 的 worker 仍会挂）⇒ **并进 #56，不单立案**。理由：#56 的病灶原文就是「写帧持 `stdin` 锁 + 对端僵死不读且管道满 ⇒ 写方阻塞」，停机帧同属「宿主侧对端僵死时的无界等待」这一族；拆开会让同一处代码被两个任务先后改。

### 27.4 登记为债（不冒充已定论）
`DEFAULT_HANDSHAKE_TIMEOUT_MS = 10 s` 与 #74 正在处理的接缝常量同一口径：**判断值、非实测分布**（仓内没有任何真插件的握手耗时样本），风险方向是**误杀**（解释器型插件冷启动 import 慢于 10 s ⇒ 装载失败，不是挂死），已在常量文档按 `AGENT_LOOP_RUN_TURN_TIMEOUT_MS` 的同口径标「真插件接入后应复核」。带门控的 e2e 只证「宿主侧有界 + 标脏不外泄」，**不证「误杀概率可接受」**。按派单约束我没让 #74 碰它（本批刚立）⇒ 由我单点记名，与 M-4 的「无实测的裁定值」标签一起收。

## 28. `#34` 交付账：删 §14.4 判「可删」的两条 trait（2026-11-02；报告 `.tmp/res34-report.md`，149 行；**§③ 验证表执行者留空，由我代跑补齐**）

### 28.1 改动面（**实测订正：`#34` 自身 = +2 / −262 ⇒ 净 −260**；原报「−258」是执行者 §① 在**我补删之前**测的，两条差异都归我）

`git diff --numstat HEAD` 现值（06:2x 我实跑，非任何一侧的算术）：`agent/src/ir_renderer.rs` 0/197 · `agent/src/lib.rs` 0/1 · `harness/src/ir_renderer.rs` **0/21** · `harness/src/kit_bridge.rs` 0/6 · `harness/src/lib.rs` **13/1** · `kit/src/bridge_impls.rs` 2/36 ⇒ 六文件合计 **+15 / −262**。⚠ **其中 `harness/src/lib.rs` 的 +13 不属于 #34**，是 #37/#40 在该文件加的 `pub mod vector_search;` / `pub mod vector_store;` 及两组 `pub use`（`git diff` 原文可逐行核），#34 在该文件的贡献只有那 **−1** 行 mod 声明。⇒ **本批自身 = +2 / −262 = 净 −260 行**，比执行者报的 −258 少 2：① 我 `rm` 掉 SPDX 占位行使 `harness/src/ir_renderer.rs` 由 −20 变 **−21**；② 我删的 `harness/src/lib.rs` mod 行 **−1**。

| 文件 | `+/−` | 内容 |
|---|---|---|
| `harness/src/ir_renderer.rs` | 0/**21** | trait 本体 + `use crate::types::ContentBlock` + 模块文档；文件只剩 1 行 SPDX 占位 ⇒ **随后由我 `rm`** |
| `agent/src/ir_renderer.rs` | 0/197 | **整文件删除**（`DefaultIrRenderer` + `translate_tool_result` + `json_to_natural` + `clean_output` + **7 条 `#[tokio::test]`**） |
| `agent/src/lib.rs` | 0/1 | 删 `pub mod ir_renderer;` |
| `harness/src/kit_bridge.rs` | 0/6 | 删 `pub trait KitSlashCommandProcessor`；**`SlashCommandAction` / `SlashCommandPreprocessed` DTO 原样保留** |
| `kit/src/bridge_impls.rs` | 2/36 | `pub use` 列表摘掉 `KitSlashCommandProcessor` + `SlashCommandAction` 两个名 + 删 `KitSlashCommandProcessorImpl` 与其 `impl`；同列 `KitHtmlCleaner`/`KitMarkdownParser`/`KitSkillDirs`/`KitTokenBudgetTracker` **一字未动** |
| **我补的两处** | **−2** | `harness/src/lib.rs` 的 `pub mod ir_renderer;`（−1 行，在执行者禁改名单内）+ SPDX 占位文件本体 `rm`（−1 行 ⇒ 该文件由 −20 变 −21） |

禁区 12「删前确认引用归零」我独立复扫一遍：`IrRenderer` 族改前 **12 处**（1 trait 定义 + 1 doc + 1 `use` + 1 struct + 1 impl + **7 处仅测试构造**）· `KitSlashCommandProcessor` 族 **4 处**（定义 / 导入 / struct / impl）⇒ 改后 8 根 grep **EXIT=1 零命中**；今日盘上复验 `grep -n ir_renderer axagent-core/crates/harness/src/lib.rs` = **EXIT=1**，两个文件在 `git status --porcelain` 均为 **`D`**。近名复扫唯一残留即那条禁改的 mod 行（已删）。非 `.rs` 侧两处历史快照（`AUDIT-capability-inventory-2026-11-01.md:228`、本文 §14.4）**按历史时点保留不改**。

### 28.2 验证表：执行者首轮留空，**两轮补齐后比我那一版更硬**（它抓出自己三次假绿）
我先行补跑的那版：定点六步 `cargo check -p <crate> --all-targets` + `cargo clippy -p <crate> --all-targets -- -D warnings` × `harness` / `agent` / `kit` = 全 EXIT=0；`__TAURI_WORKSPACE__=true cargo test -p axagent-agent`（`--no-run` + 测试 exe 直跑）= `NORUN_EXIT=0`、`TEST_EXIT=0`，**2052 passed / 0 failed**（lib 1936 + 8 个集成 target 共 116）+ doctest `0 passed / 5 ignored`，日志 **155 821 B** 非零；日志内 `ir_renderer|IrRenderer` **0 命中**。
执行者随后交回的 `res34-report.md` §③（17 651 B，06:17）把同一条链**按「自证真跑」的口径重做了一遍**，并**自我推翻三轮**（这三条都是本仓假绿谱的新形态，逐条记进 §28.7）：
| 轮次 | 它原本打算采信的东西 | 它自己的判读 |
|---|---|---|
| 第一轮 clippy ×3 | 三条 `EXIT=0`（25/18/22 s，日志 127/72/72 B） | **判为不成立**：「只吐 `Finished`、无 `Checking` 行 = **缓存命中，不是这次跑的**」 |
| 补验 `-v` | `DIRTY=0 / FRESH=259~448` | **判为间接**：「`-v` 只说明产物比源新 ⇒ **不采信**」 |
| 强制跑 | `touch` 本批 3 个 `.rs` **只改 mtime、内容零变化**（`git diff --numstat` 复核一致）后重跑 clippy | **成立**：三包各自打出 `Checking axagent-{harness,agent,kit}`，`-D warnings` 下 **EXIT=0**、耗时 **115/135/88 s**，`grep '^warning|^error'` 三份日志**无输出** |
| 测试构建 | 经 `env` 前缀的 `cargo test -p axagent-agent --no-run`：EXIT=0 **且日志 0 字节** | **判为未执行**，并核对盘上**无** `axagent_agent-*.exe` ⇒ 换 cwd=`axagent-ai` 直调后列出 8 个 exe、目标 exe **45 695 488 B / mtime 05:53:50**（晚于最后源码改动 05:32）|
| 直跑 | 该 libtest exe，cwd 保持 UNC 的 `axagent-ai` | `running 1936 tests` … `1936 passed; 0 failed`，EXIT=0，日志 **138 094 B** |
⚠ 等锁实况：链内多次 `Blocking waiting for file lock on build directory`（#63 在占）⇒ 按约束等、未 kill 未 clean，上表耗时**含等锁**。
⇒ **结论不变但证据升级**：「agent crate 其余用例未被这次删除打破」现在有**断言面**保证（2052 例全绿），不再只是 `--all-targets` 的编译面。执行者 §⑦.1 原先那句「要不要补跑，请裁」已作废 —— **已补，且两轮独立取数同值**。

### 28.3 覆盖面账（降覆盖必须留名）
**净失 5 条断言**：`skips_tool_use`（锁的是「**工具调用不外泄**」这一**产品语义** ⇒ 语义随**实现**一起消失，不是随用例消失）· `translates_tool_result_json` · `handles_error_tool_result`（ToolResult 文案）· `renders_plain_text` · `handles_empty_blocks`（空输入直通）。**未丢 2 条**：占位符清理与连续空行合并在 `harness/src/text_clean.rs:42/48` **原文还在**（逐字相同断言）。
⇒ 今后若要恢复「不外泄工具调用」语义，需要**重建实现 + 用例**，**不是去 git 里捞旧测试** —— 这句必须留在 PLAN，否则下一个人会恢复一个没有调用方的实现。

### 28.4 执行者标记的「唯一可能被盘上推翻的点」，我据盘裁死：**删除成立**
`harness/src/ir_renderer.rs` 原注释自称「实现方（agent）通过 `LlmCallConfig.ir_renderer` 注入」。我读了 `harness/src/llm_execution.rs:19-28`：`LlmCallConfig` 只有 `strict_mode` / `max_context_tokens` / `reserved_output_tokens` / `session_id` **四个字段，没有 `ir_renderer`**；且改前全仓 `ir_renderer` 命中只有 4 处（两 `mod` + 一 `use` + **该注释本身**）。⇒ 判**过期陈述**，不是「本该存在但没落地」的接线缺口：盘上没有承接物（无字段、无调用方、§0.7 登记表无条目）。IR 渲染若要复活属产品决策，与本次删除无因果。**不留活口。**

### 28.5 顺带发现（免下轮重复清理）
被删的 `KitSlashCommandProcessorImpl` 曾转发到 `kit::slash_command::{load_bundle_content, load_skill_content, process_slash_command}` —— 这三个函数**仍有生产取用**（`kit/src/slash_command.rs:279/289/305` 的 `apply_slash_command_to_input`）⇒ **未因本批成为孤儿，无需后续清理**。另 `agent/src/lib.rs:220 pub fn clean_output` 是**函数体自实现的 `content.trim_end()`**，与被删的 `ir_renderer::clean_output` 无继承关系，复扫确认不受影响。

### 28.6 读数贡献（只记贡献，绝对值等收口统一重钉）
本批对 `k1` 的贡献 = **`pubTraits` −2**（执行者编辑窗口内外两跑同值 **316**，即 §17 的 #50 定案登记值 318 − 本批 2）· **`rsFiles` −2**（两个 `.rs` 在我补删 mod 行后彻底消失）。
⚠ 「绝对值不可采信」的两条实证理由仍成立，登记以免下轮有人直接抄：① **扫描器自己在两次取数之间被改**（`check-capability-register.mjs` mtime 05:29 落在中间，`k1` 段第一跑没有 `declaredExternalTraits` 字段、第二跑有 `232` ⇒ 同一命令输出**结构都不同**，差 1 完全可能是口径被改而非文件数变了）；② **树在并发改**（10 个未跟踪新 `.rs`，都不是 #34 的：`harness/src/vector_{store,search}.rs` + `plugins/src/bin/e2e_*_worker.rs` 4 个 + `plugins/tests/e2e_*.rs` 4 个，#63 又加 2 个）。⇒ 绝对值**由我在收口时用一次干净的 `--dump-readings` 统一重钉**，与 #63/#73/#74 的贡献同批归因（**但当前已可对账到个位**：`pubTraits` = **316**，我与执行者各自独立跑、同值 ⇒ 可信；`rsFiles` = **2008** = 2007（本批补删 mod 行与占位文件后）+ **1**（`#63` 的 `plugins/src/bin/e2e_no_describe_worker.rs`）—— 同批另一个新文件 `plugins/tests/e2e_handshake_timeout_test.rs` **不计**，它落 `tests/` 被扫描面 `K1_IGNORE` 排除 ⇒ **加 2 个 `.rs` 只升 1 个读数**，与 §0.12.1 记的 `#49` 先例逐字同口径，**无未知漂移**）；任何会话**不许自写第二套扫描器来仲裁**（禁区 12，第五道门已在扫同一棵树）。

### 28.7 本轮新增的三类「假绿 / 假读」形态（今后派单的规避表要带上）
1. **`cargo clippy` 只吐 `Finished`、无 `Checking` 行 ⇒ 缓存命中，不是这次跑的**（执行者自判第一轮三条 `EXIT=0` 全部作废）。`-v` 给出的 `DIRTY=0 / FRESH=259~448` **仍只是间接**（它只证明产物比源新）。⇒ **强制手段 = `touch` 目标 `.rs` 只改 mtime、内容零变化**（改完必须 `git diff --numstat` 复核内容未变），重跑才出 `Checking <包>`。
2. **`cargo test --no-run` 经 `env` 前缀 ⇒ EXIT=0 + 日志 0 字节 + 盘上没有目标 exe**：已立的「退 0 且 0 字节 = 未执行」判据**在构建阶段同样成立**，不止运行阶段。⇒ 判 `--no-run` 成功要看 **exe 清单 + 文件 mtime 晚于最后一次源码改动**（本次实测 `axagent_agent-2637d30aa871c532.exe` = 45 695 488 B / mtime 05:53:50 > 源码 05:32）。
3. **UNC / SMB 客户端缓存会把我自己的读数退回旧值**：同一条 `wc -l docs/plans/PLAN-shell-plus-plugins.md` 在写入后第一次读到 **1652**、隔数秒复跑读到 **1711**（后者为真 —— `grep -n` 同一轮就命中了第 1654 / 1681 行，与 1652 自相矛盾）。⚠ **这条对收口工序是实质风险**：`--repin-plan` 与判定档都要**读 PLAN**，读到缓存里的旧 PLAN ⇒ 写进镜像表的行号会按旧树锚。⇒ 新增硬规矩：**跑任何读 PLAN 的门模式前，把行数连读两次取一致值**，两次不一致就等一轮再读；凡「门报红、但我刚改过被扫文件」的组合，**先怀疑缓存，别去改判据**。
4. **门禁脚本自己会有一版「语法不过」**（执行者 §⑤ 实测，本仓第一次记）：`scripts/check-capability-register.mjs` 在取数窗口内 mtime 连变三次 **05:29:11 → 05:57:35 → 06:01:09**（**32 秒内改两次**），其中 **05:57:35 那一版 `node --check` 直接 `SyntaxError: missing ) after argument list`（该版 :2399 的一条 `正控㉗a` 自检断言）**，`node` 秒退 EXIT=1、产物 1380 B **全是报错文本**。⇒ 判据补两条：① **取读数前先 `node --check <门脚本>`**，不过就等交锁（假绿型 ⑤「脚本被并发改」的升级形态）；② **EXIT 之外必须看产物内容**——「EXIT 正常 + 产物是一段 SyntaxError」同样不是读数。⇒ 也正是这条实证，坐实了 §28.6 的结论：**门被并发编辑期间取到的绝对值一律作废**，重锚与重钉只能在全部并发会话交锁后做一次。

## 29. `#66` 交付账：⑥ 收窄真值 + `#[cfg(test)]` 实现者列（2026-11-02；报告 `.tmp/res54-gate-side.md`，218 行 / 22 444 B；会话 `150a04c6`；改动面唯一文件 `scripts/check-capability-register.mjs`，2869 → 2870 行）

### 29.1 收窄后的**门内真值 = 强 37 / 弱 8**，我的推算 36/9 作废（订正已就地在 §24.4）
`--dump-readings` EXIT=0、200.2 s、`walkFiles=10292`。差 1 出在 `CompletionProvider`：它唯一那条 ⑥ 站点的**接收者确实是集合** ⇒ 留强（`runtime/src/shell_completer.rs`：`:139` 字段是 `Vec` → `:144 add_provider(Box<dyn CompletionProvider + Send + Sync>)` → `:145 custom_providers.write().await` → **`:146 providers.push(provider)` 落在「站点行 + 其后 5 行」窗内**；四行号均取自 `grep -n`）。⇒ 真正降弱的只有 `UnifiedKnowledgeSource` ⇒ **掉出 55、降级 1**。
⇒ **这条差值反过来把口径边界钉死了**：顺序型容器（`Vec`）算集合、单槽（`Option` / `OnceLock` / 结构体字段）不算 —— 与我 §24.4 定的判据一致，**不是执行者自行放宽**。⚠ 表内 `k2KeyedFaces` 现登记 **92/7**、与新口径差 **−55**，因 `hard:false` ⇒ 只报告不叫停；**重钉 37/8 由我在 #73/#74 交锁后随一次干净 `--dump-readings` 统一做**（那两批若在测试里新增 `Vec<dyn …>` 的 push 站点，这两个数还会动），`k2TraitFaces` 的 51 同批。

### 29.2 test 列：`implementorsTestMod` = **48 个 trait / 84 个 impl 站点**（06:42 首测）⚠ **06:59 同树复测已变 `49 / 86`** —— 并发会话在 `worker.rs` 的 `#[cfg(test)] mod` 里新增了 `NativeWebhookDispatch`（`:4028`）与 `SlowWebhookDispatch`（`:4580`）两个替身 ⇒ **这两个数是「随时会动」的，收口必须用一次干净 `--dump-readings` 重取**，本节数值只作形态与自证用。其自证比数值更有价值：**每个 trait 都同时出现在 `implementors` 里，且每个 `testMod ≤ implementors`** ⇒ 无双计、无凭空多出 ⇒ 「生产 = 机械 − test」**恒非负**。判据 = 同文件内最近一个未闭合的 `#[cfg(test)] mod`，区间 = mod 头行到闭合行**两端都含**。
**落在登记行的 12 行**（三档实测；`PLAN 行` 由它**逐字取自 `capability-register.json` 的 `planLine`**、现已随我改 PLAN 漂移 ⇒ 重钉归我）：`model.provider` 14/2⇒12 · `tool.set` 199/1⇒198 · `platform.adapter` 10/1⇒**9** · `webhook.dispatch` 3/1⇒2 · `workflow.business_rule` 3/1⇒2 · `market.vendor` 16/2⇒14 · `search.source` 4/2⇒2 · `strategy.quant` 7/1⇒6 · **`workflow.node` 33/1⇒32** · **`memory.provider` 2/1⇒1** · `plugin.hook` 3/1⇒2 · market-sim DES `SimAgent` 12/1⇒11。
⇒ `workflow.node → 32`、`memory.provider → 1` **与 §24.1 / §24.5 的人工核数逐字一致**；且 `dispatcher.rs` 一次吃掉 4 个站点（`TestExecutor` + 3 个 `Always{Allow,Deny,Audit}Checker`）而区间没串到块外（`cfgTestRanges` 实跑 = `[{501,730}]`；`memory_hook_provider.rs` = `[{240,394}]`）—— **同文件多 test mod 边界各自闭合**是本列最硬的一条实证。
⚠ 第 13 行 `SeamInvoker`（K 组）**机械映射落不进去**：3 个 test 块内站点（4 ⇒ 生产 1），但登记行**无 `trait` 字段**（既存「未机器复检」行）⇒ 只在报告点名、不进表。
⇒ **未落登记行的 36 个 trait / 69 个站点**里有一批「实现者几乎全是测试替身」的极端值，今后判 K1 时应看一眼：`ApiClient 17/18`（生产仅 1）· `RepoProviderRepo 1/1`、`PermissionChecker 3/3`、`Greeter 2/2`、`FeatureFlagProvider 1/1`、`DecomposerLlmClient 1/1`（**生产全为 0**）· `MarketplaceScanner 1/21` · `Drop 3/29` · `Default 1/507`。

### 29.3 ⚠ 执行者推翻我 §24.5 的一句话，我采纳 —— **#71 的范围随之缩小**
我原写：`MockPlatformAdapter@src-tauri/src/init/notification_adapters.rs:150`「在**生产路径文件**里 ⇒ 路径口径对它无感」。执行者用 `.tmp/res54-t71-probe.mjs` **实跑**（非目测）纠正：「路径」那句成立，但它**同时就在该文件的 `#[cfg(test)] mod`（区间 `[{127,308}]`）里面** ⇒ **本批新列能机械指认它**；`platform.adapter` 的 10 个站点里唯一落在 test 块内的那个正是它，其余 9 个 = 8 个真平台适配器（`Dingtalk/Discord/Feishu/QQ/Slack/Telegram/WeChat/WhatsApp`）+ `RemotePlatformAdapter@axagent-plugins/crates/plugins/src/worker.rs:2420`。
⇒ **三条裁定**：① **#71 不必再造「类型名定位器」，直接接这一列**（任务描述已同步改）；② 但**按 §24.5 原裁决本批不剔它、不改 `implTraitOf`** —— 剔除属 #54 结构级联，一动就要重算全表实现者数（当初拆两批正是为此）；③ 我的**顾虑仍成立、理由错了**：不是「口径看不见」，是**我把它的位臵记错了**。⇒ 已就地订正 §24.5。

### 29.4 判据不变性的实证（我要的正是这条，它给对了）
`--selftest` **EXIT=0**、覆盖度 **17 / 16（按镜像表实际使用计）—— 无空洞**；新增断言含「集合接收者进强 / 单槽 setter 与 `Option`·字段写入**不计且不留形态** / 集合标记在窗内计、出窗不计 / `vec![` 与 `.insert(` 也算集合接收者 / **动词表与 3 行参数窗口未动**」及 test 列 8 条边界断言（块内外两本账不串 · 属性后跟 `fn` 不开区间 · 花括号配平 · 无属性零区间）。
**判定档改前 → 改后同一棵树**（`walkFiles=10292` 两次相同，K1 读数 `2008/316/270/232` 两次相同），`diff <(grep ^FAIL 改前) <(grep ^FAIL 改后)` **输出为空** ⇒ **6 条硬失败逐字不变、EXIT 均 1**；非 FAIL 面只变 3 行（只报告条目 +1 · `k2KeyedFaces` 由 `PASS` 转 `REPORT` 差 −55 · `push-register` 站点 **104 → 6**）。⇒ **「收窄判据只改读数、不动判定档」被实测钉住，不是声称**。
⚠ 头注另钉一条：**窗口行数与动词表不变正是 §24.4 推算可反判的前提**。执行者守住「不许为让推算对上而动窗口/词表」，才使「推算 vs 真值」的差**能**用来反推实现与口径是否一致 —— 本次就是这么发现 `CompletionProvider` 的。⇒ **今后凡我给推算值让执行者实现判据，必须把「不许改窗口/词表」写进派单**，否则这道反证作废。
⇒ test 列**只进 `--dump-readings`、不参与退出码**（按我原令），执行者按最小面落 ⇒ 我确认这个落法即可，升 `hard:true` 归 #54。

### 29.5 `#66` 交回四条待裁，我逐条裁（不留二选一）
1. **92 → 37 与 51 何时重钉** ⇒ 见 29.1：**统一等 #73/#74 交锁**，一次 `--dump-readings` 重钉，不分批。
2. **`planLine` 已随我改 PLAN 漂移，它未跑 `--repin-plan`** ⇒ ✔ 正是我给它的禁令（leader 改 PLAN 期间执行者禁跑 repin），保持。
3. **新列要不要打进判定档表行** ⇒ **不要**：判定档是发布门，把尚未参与裁定的读数打进去，会让下一次 `--repin-*` 的行位变化被误读成能力面变化。#54 落表时一起进。
4. **`CompletionProvider` 留这条 push 站点是否合产品直觉** ⇒ **合，且它是收窄的正面样本**：`add_provider(Box<dyn …>)` + `Vec` 写守卫 = **运行期可增长的注册口**，与构造期单槽 DI 注入是两种东西；**但不因它命中就把 `CompletionProvider` 升成接缝候选** —— 那属 K1/K3 裁定，检测器只产候选不产结论（§14.5、§24.3 已两次立此条）。

## 30. `#74` 交付账：M-4 已做、S-6 卡住并牵出一条**既存冻结**（2026-11-02；报告 `.tmp/res74-report.md`，23 511 B / 229 行；会话 `c1a44390`；改动面唯一文件 `plugin-proto/src/lib.rs`，1643 → **1686** 行）

### 30.1 M-4 落地：`run_turn` 180 000 → **900 000**，且它的「有出处」是真的
逐 op 判据表把每个数的**证据类型**分三档写死（实测 / 有出处 / 无）：`run_turn` 取 `max(providers/src/lib.rs:278 的 300 s, seed_stock_analysis.rs:4515 的 900 s)`，旁证 `:4509`（TTFB 偶发 >120 s）、`harness/src/capability_pack_orchestration/subgraph.rs:145`（900）/ `:94`（600）、`commands/stock_analysis_setup/seed_variables.rs:202`（缺省 600）。⇒ §26.1 的 M-4 行已就地标 ✔ 已做（含两处对我原句的订正，见 30.3）。
⚠ **同批产出一条不该被忘的负面结论**：**18 个 op 至今没有任何盘上耗时实测或宿主预算出处**（`reflect` `suggest` `evolve` `on_message` `execute` `record` `assert` `start` `stop` `send` `dispatch` `is_enabled` `is_connected` `list` `ui.action` `call_seam` `event` `shutdown`）⇒ 全部留 15 s 默认；另有 `chat` / `embed` 属「有文档口径（0.5–30 s）、无 op 级实测」⇒ 同样不进表，只在注释标「高端会误杀、预算待实证」。⇒ **§26.1 那句「除 `run_turn` 外全部 op 走 15 s」的根因不是「没来得及加表项」，是「没有证据可加」** —— 今后要给任何 op 定预算，先给实测，再进表。

### 30.2 ⚠ 它发现「这张表今天事实上被冻结」，我采纳并派 **#76**
`seam_timeout_override_table_is_evidence_bounded` 把 §5.4 全部 **23 个 op 逐个钉成「必须 `None`」** ⇒ **任何新表项，无论证据多强，都必须先改这个测试**。这与 M-4/S-6「按证据逐条扩表」的路线**互相咬**：`exec` 的 60 s 明明有盘上出处（`plugins/src/types.rs:837 DEFAULT_TOOL_TIMEOUT_SECS = 60`，用于 `:867` / `:955`），却因名单锁进不了表 ⇒ 执行者按我原令「不许为适配表项而放宽测试」**停手交回**（正确）。
⇒ **我的裁定（三选一，理由同批写死）**：采 **(a) `("exec", 60_000)`**，但**不是删名单一项了事，而是把该测试升级成双向锁** —— **(i) 表内每一项必须携带盘上出处且断言返 `Some(_)`**（今天两项 = `run_turn` 900 s / `exec` 60 s），**(ii) 名单外的 22 项继续逐个断言 `None`**。⇒ **总覆盖面不降**：23 个 op 仍逐个被断言，只是判据从「恒 None」变成「按有无证据分两档」，**这是把「有证据才进表」这条不变量变成可执行的**，不是放宽。
- **(b) 驳回**（由 #73 在 `types.rs` 侧把 spawn 通道降到 15 s）：那是**改宿主自己的工具执行预算**，且与 §26.3 S-6 的对齐方向**相反** —— S-6 说的是「远程 `exec` 对齐 spawn 的 60 s」，不是反过来削 spawn。
- **(c) 驳回**（维持现状）：留着 4 倍差 = §26.3 已判过的缺陷不修。
- 顺带授权它改 `DEFAULT_HANDSHAKE_TIMEOUT_MS` 注释的一句措辞：`run_turn` 本批后不再是判断值 ⇒ 原句「与 `AGENT_LOOP_RUN_TURN_TIMEOUT_MS` 同口径标如实」会变成**假话** ⇒ 改成「一样**缺实测分布**（那边已有宿主预算出处，本数仍是判断值）」。⚠ 不动常量值、不动语义、`describe` 仍不进表。
⇒ 另要求它**不许顺手把 `chat` / `embed` / `send` / `dispatch` 加进表**（无实测），保持 30.1 的负面结论不被绕过。

### 30.3 它推翻了我 §26.1 M-4 行的两处引用（我采纳，已就地订正）
1. **`agent_executor.rs` 里没有 900**：我原句把 900 s 挂在「`executors/agent_executor.rs:433/270`」旁边。它**拒绝采信一条混入后台任务输出的假行**（`rt-workflow/src/executors/` 注：agent_executor.rs:432` `NODE_BUDGET = Duration::from_secs(900)`），改用 `git ls-files` + `git grep` 独立复核 ⇒ 真实路径是 `axagent-runtime/crates/runtime/src/work_engine/executors/agent_executor.rs`，**该文件既无 `NODE_BUDGET` 也无 900 字面量**；`:270` / `:433` 只是 `run_turn` 的**消费点**。⇒ 已按盘上实况订正 §26.1。⚠ **这条的价值不在「行号错了」，而在「后台任务的输出也可能是别的命令的行」** —— 与本仓 §28.7 的假读族同族，判据：**引用别人贴的行前先 `git grep` 自己复现一次**。
2. **「`max_tool_rounds: None` ⇒ 回合时长无上界」措辞过强**：宿主适配器 `src-tauri/src/init/agent_turn_adapter.rs:199-207` **不透传** `request.max_tool_rounds` 给 runtime，兜底来自 `runtime-core` 的 `max_iterations` 缺省 **50**（`axagent-runtime/crates/runtime-core/src/conversation.rs:262`）⇒ 严格说是「50 轮 × 单请求 300 s ⇒ **没有秒级上界**」，不是「无迭代上界」。已就地订正。

### 30.4 它的取证质量（值得记，因为它自己报了三条瑕疵）
10 条命令全带 **EXIT + 起止时刻 + 日志字节**，基线与改后各跑一遍（plugin-proto 用例 **32 → 32**，`--no-run` 列出 exe 名，check/clippy 日志含 `Checking axagent-plugin-proto` 原文，fmt 只读探针带 `--config-path` 且 `Diff in` = 0）。自报瑕疵三条：① `--exact` 漏 `tests::` 前缀 ⇒ libtest `running 0 tests` 且**退 0**（本仓第三种假绿形态，它主动登记并用完整名重跑取到 `1 passed / 31 filtered out`）；② 一次把 `RUN_EXIT` 取自 `| tail` 的退出码 ⇒ **自己判定该行不作数**，由无管道那次覆盖；③ 上一条的假行号采信风险（见 30.3.1）。⇒ **这三条正是我派单里「EXIT 之外要看产物内容」被执行者内化的证据**。

## 31. `#70` 已做：232 从「不可复现」翻成**可复算硬锚点**（2026-11-02，我本人改，无委派）

**触发**：`#61` 落地谓词、`#66` 验收之后，PLAN:248 与镜像表里「232 出自一次性 census 工件、未入库 ⇒ 无法复现」**已成假话**。按「留假话先改措辞」的规矩就地订正，不删条目：
1. **PLAN:248 就地改写**：232 改为「**可复算值**」（谓词已入库为 `k1DeclaredExternalImplTraitCount`，现树复现 232，且与 audit 侧**排序名单逐字相等**）；`210` 与 `269` **降级为解释、不再与 232 并列**（210 = 单趟缺陷，两趟差恰 22；269 = 最严读法，永不作本锚点值）；保留「**先让谓词可复算、再升 hard**，反向必先假红」这条处置次序。
2. **镜像表锚点 `k1ExternalImplTraitCount` 升级**（`scripts/capability-register.json`）：`check: manual → k1DeclaredExternalImplTraitCount`、`hard: false → true`、**删 `implementorsMechanical: 210`**（单趟缺陷读数，并列会让人拿 210 当基线）；`value` / `planNumber` / PLAN:248 三者同为 **232**（满足门内「纯数字 token 必须同值」的硬判据）。
3. **未做（留给收口）**：完整判定档复跑（此刻 `#73` 正在挪 `worker.rs`/`manager.rs`，evidence 锚点必然漂，跑了也只会看到预期内的红）。⇒ 收口时若 232 与现树不符，**先查是不是 `#62`（合并两组重复 trait + ToT 改名）改了计数**，再决定重钉还是回退 `hard`：本锚点已不是「人工例外」。
4. **顺带更正一条我口头说过多次的错**：硬失败是 **7 条**不是 6 条 —— 漏的是 `webhook.dispatch`（`implementorCount` 类，不在 evidence 行位里）。逐条清单与处置见 `.tmp/repin-note.md` 的 07:20 节。

## 32. `#73` 交付账：M-3 嵌套误杀根除 + M-1/M-2/S-1/S-3/S-4（2026-11-02；报告 `.tmp/res73-report.md`，38 277 B / 445 行；会话 `de135eba`）

### 32.1 改动面与「先红后绿」
`worker.rs` 4096 → **4704**（+608）· `manager.rs` 4871 → **5006**（+135）· 新文件 `src/bin/e2e_exit_after_handshake_worker.rs`（79 行）+ `tests/e2e_legacy_worker_fallback_test.rs` 扩到 442 行。
**M-3 的红是真红**：`nested_call_seam_beyond_outer_budget_does_not_poison_the_connection` 在「只加用例」阶段 `EXIT=101`，panic 原文同时坐实 §26.1 的两条断言 —— 外层被 150 ms **误杀**（`nest` 超过上限 150 ms）+ 闸落下后**永久被拒**（`拒绝新接缝调用`）。夹具走**回环 socket + 真实 `Peer`、不起子进程** ⇒ 不受 `AXAGENT_TEST_PLUGIN_SUBPROCESS` 门控 ⇒ **CI 恒跑**（这是本批最重要的一条取证设计）。改实现后同一 exe 同命令 `EXIT=0`、0.63 s ≈ 内层 600 ms ⇒ 外层确实**熬过**了嵌套。clippy 在实现阶段抓到 `collapsible_if`（`worker.rs:1531`）并修，终轮 `EXIT=0` 且日志含 `Checking axagent-plugins`。
**判据分离落地成表**（`#59` 复核要求的「进程不在 = 回退 / 曾在超时 = 拒绝」）：worker 退出或管道断 ⇒ `NotDelivered` **回退**；worker 已执行完（Fault 文本里装了 `UNKNOWN_OP`）⇒ **不回退**；脏闸落下 ⇒ 接缝与命令**都拒绝**并给复位口；**嵌套**（内层 > 外层预算）⇒ 不置脏、无错误。⇒ `#59` 的「回退只发生一次」从此有逐情形用例锁。

### 32.2 它偏离了我派单的一处，我**采纳**（理由比我的口径更对）
我派单写「保序口径：只在同插件 + 同 peer 内串行」。它**没照做**，理由是：一条连接的两个端点就是「本插件 + 该 worker」，按连接串行 = **保留 head-of-line 阻塞 = M-3 的误杀链照旧**（嵌套用例仍会红）。它改成「**同连接内并发处理入站请求，仅回写经 `stdin` 锁互斥**，跨请求顺序由 `request_id` 配对保证（`Peer` ID 不匹配立即 `Err`，不猜）」。
⇒ **我判它对、我错**：我那句「同 peer 串行」在连接粒度上等于什么都没改；它的形态同时满足我的硬约束（不许跨请求全局串行）且余量更大。⚠ 但要**留一句代价**：入站处理从串行变并发后，**协议不再保证同连接内的处理顺序**（今天没有插件依赖它，但这是语义变化，不是实现细节）⇒ 若要恢复串行，正确的切分键是**目标接缝 / peer**，不是连接。

### 32.3 它交回的 9 条，我逐条裁（不留二选一）
1. **嵌套期间外层预算挂起 ⇒ 外层跟着没有上界** ⇒ **不加入站总闸**。理由两条：① 入站服务侧**今天本就没有等待上限**（审查稿 A4.2 原文），本批没有新增无界；② 内层那一跳是宿主侧调用，它自己有预算（LLM 300 s / 工作流节点 900 s，§30）⇒ 实际有界，只是「界」在下层。⇒ 登记进 **#56** 的栈（与写侧/阻塞面同一病灶），本批不动。
2. **`loaded_workers` 陈旧表项不驱逐** ⇒ **维持现状**：每次命名命令多一次 `try_wait`（微秒级），驱逐需要 `&mut self` 或内部锁，收益不抵复杂度；登记进 #56 栈。
3. **M-1 的端到端证据在门控内（CI 恒 SKIP）** ⇒ 与 S-2 同一笔债，**本批不改 CI**；PLAN 侧明写「**本机证据、CI 不背书**」，无门控侧由两条 `NotDelivered` 用例 + 分类用例兜住「回退发生」。接进 CI 是独立任务（涉及 `.github/**` + `scripts/**`）。
4. **`SEAM_*` 码在 11 语言里零翻译** ⇒ **本次不补**，但理由必须写死：这些码是 **Transport 的原因串**，不是 `ErrorResponse.code`；前端今天**没有**按它们做分支，因此不存在「显示原始码」的新缺陷。⇒ 判「不是本批引入的缺口，也不在本批补」；若将来要把它们当 UI 错误码用，才按错误码规范补 11 语言。⚠ 这条与 #63 的「不新增码」选择一致，**不构成同路径双码**。
5. **它改了 `WorkerError::Handshake` 的 Display**（尾部加「请在设置中禁用再启用该插件以重试装载」）⇒ **接受**：这是我在 §27.3 给 M-2 定的「两条置脏路径的用户可见出口都要说到」的落地；它已 grep 确认仓内只有 `contains` 断言、无精确匹配。**未新增码**。
6. **`fmt` 未验证（608 行手工排版）** ⇒ 收口时**我**跑只读探针 `rustfmt --check --config-path` 复核 `worker.rs`/`manager.rs`，与 HEAD 既存 9 处债（#69）分开计；本批若引入新 diff，退回给它修。
7. **N-1/N-2/N-4 仍是盘外/未统计** ⇒ 保持「已知不确定」，不冒充结论（与 §26.5 一致）。

### 32.4 ⚠ 它的「`capability-register.json` 本批无读数变化」这句**不准确**，必须更正
它新增的 `impl WebhookDispatch for RemoteWebhookDispatch`（`worker.rs:1996`，生产）**正是第 7 条硬失败的成因**：`webhook.dispatch` 的 `implementorCount` 3 → **4**，并连带 `externalImplTraits 270→271`、`implTypesKnown 742→743`、遍历 `10292→10295`、`implementorsTestMod 48/84→49/86`。它之所以说「无变化」，是因为**只看了 trait 计数**（确实没变）而没看**实现者计数**。
⇒ 判据（写给下一批）：**「读数变化」= 本门全部读数，不只是 trait 数**；新增一个 `impl … for …` 就是 A 档计数变化，哪怕它只是接缝桥。
⇒ 处理（见 `.tmp/repin-note.md` 07:20 第 3 条）：机械 4 = 生产 2（`webhook_dispatcher.rs:97` + `worker.rs:1996`）+ test 2（`worker.rs:4028` / `:4580`）⇒ `expect.implementors` 3 → **4**，note 写「生产 2」，理由留字。

## 33. `#64`（含 `#72`）交付账：页面容器本体 + 一个**今天就打不开**的既存缺陷（2026-11-02；报告 `.tmp/res64-report.md`，29 828 B；会话 `2a846c32`）

### 33.1 落地面（新增 23 条用例：容器 19 + 门 E/F 行为 3 + 门 A 消费方 1 行；全部随 `npm run test:run` 进 CI）
按 §20.4 的 A–F 逐条对账（**E1 的 Rust 侧按派单未做，已拆 #75**）：容器本体（`plugin_resolve_page_asset` → asset URL → iframe；`sandbox="allow-scripts"` 且**未加** `allow-same-origin`；分支判据只取 `loading`，`schema` 不调解析命令；`entry` 只用于展示）；`announceReady()` / `dispose()` 接线；`assetUrl()` 出口进 `src/lib/invoke.ts`（未新开 import 面）；`browserMock.ts` 补桩；**`#72`：沙箱侧入向判据与宿主侧对齐**（复用同一个 `isFrameFromSandboxSource`，期望来源为 `null`/`undefined` 时一帧都不认）。
⇒ 关键取证形态：容器那条「iframe 在 `loading` 期就在 DOM 里」的用例是本批最有价值的一条（见 33.2）。

### 33.2 ⚠ 本批最重要的一条产出：**既存缺陷 —— `SkillSandboxContainer` 首挂载必然落 `skill.iframeNotReady`，srcdoc 面板 / Skill UI 今天打不开**
机理（它 jsdom 实测取证，非推理）：`loading` 初值 `true` ⇒ `if (loading) return <SkillLoadingSkeleton/>` ⇒ **iframe 元素只在 `!loading && !error` 时进 DOM**，而 `loadSandbox()` 在 `await` 之后读 `iframeRef.current` ⇒ 恒为 `null` ⇒ 抛 `skill.iframeNotReady`。取证：`iframeCount=0` + 界面文本「技能 "probe-skill" 加载失败 / Error: iframe ref 未就绪 / 重试」；并由此让门 E 的两条超时用例成了**闸生效的反证**。
⇒ **我的裁定：立即单开 #77 修它，不随本批**。理由：这是**功能不可用**（不是降级），且与「插件页面容器」这条主线直接冲突 —— 新容器做对了，旧 srcdoc 容器仍打不开，等于主壳只支持一半页型。它的处置口径（iframe 常驻 DOM + 骨架/错误做覆盖层 + 重试换 `key` 强制重导航 + 卸载时回收文档）**我整条采纳**，并要补一组 srcdoc 容器级用例（它已写了一条「loading 期 iframe 就在 DOM 里」，不够）。
⚠ 它不顺手改的理由我认可：`SkillSandboxContainer.tsx` 是 `#67` 刚交付、未提交的文件，且同批还压着门 E/F 的改动 ⇒ 混进去会让人分不清哪个改动由哪个判据背书。

### 33.3 §20.4 C 与 §25.3 的两处**口径腐烂**（盘上已被 `#67`/`#72` 推翻，我本节就地订正）
- **§20.4 C「容器显式 `createHostApiBridge({ allowedOrigin: "*" })`」已无法执行**：`#67` 的应修 1 把 `HostApiBridgeOptions` 的 **origin 形参整段删除**（`grep -n allowedOrigin src/sdk/rpcBridge.ts` 零命中）⇒ **容器本来就没有那个参数可传**。它按盘上口径做成「容器的入向不传 origin，来源正确性交给 `event.source` 比对」⇒ **正确**。⇒ §20.4 C 该句作废，改读作「入向不传 origin；来源校验靠 `event.source === 沙箱 iframe.contentWindow`」。
- **§25.3「沙箱收宿主帧那一侧仍是 origin 判定（`sandboxTemplate.ts:175-183`）」被 `#72` 推翻**：现在判 **source**（origin 只在非 opaque 文档时作为附加一层），且那三个行号已漂。⇒ 该句作废。

### 33.4 它交回的 4 条，我逐条裁
1. **`pluginClientBridge.ts` 的 `handleMessage` 仍只按 `callId` 认领、不判 `event.source`**（沙箱内若嵌了插件自己的文档，可用猜到的 `callId` 提前 settle 在途 Promise）⇒ **接受延期，但不许丢**：并入新任务 **#78**，与「`dispose()` 后不可复用」「门 C 依赖 `var M = AX_PROTOCOL.m` 的文本耦合」**同批**做（三者都要动 vendored bundle 客户端或其契约测试，拆开会两次改已分发 bundle 的接口面）。⚠ 风险等级我判为**中**（需要插件自己在沙箱内再嵌一块文档 + 猜 `callId`，且只能污染自己那一份在途调用），故不阻断 #64。
2. **`check-i18n-orphan-diff.mjs` 在本机是「跳过」而非「通过」**：其 `execSync("git …")` 在 UNC 工作树下取不到 git，脚本自己打印「跳过 ≠ 通过」但**退出码仍 0** ⇒ **又一条「退 0 ≠ 检查跑了」**（记进 §28.7 族）。它的结构性替代取证成立：`git diff -U0 HEAD -- <4 文件> | grep "^-" | grep -E "\bt\(|i18n\.t\("` = **`GREP_EXIT=1` 零命中** ⇒ 本批**没删任何 `t()` 调用点**。⇒ 正式通过仍需在 CI 或 git 可用的工作树跑一次（与 S-2 同笔债）。
3. **`list_plugin_pages`（E1 的 Rust 侧）未做** ⇒ 已拆 **#75**（要改 `plugins/src/manager.rs`，等 `#73` 交锁；现 `#73` 已交锁 ⇒ 可派）。
4. **没有改 `#73`/`#74`/`#76` 的文件**（`plugins/**`、`plugin-proto/**` 零改动）⇒ 与三批无覆盖冲突，这一点它主动核过。

## 34. `#76`（#74 续批）交付账：`exec` 进表 + 超时表测试升级为双向锁（2026-11-02；报告 `.tmp/res74-report.md` §9，37 612 B）

改动面唯一文件 `axagent-core/crates/plugin-proto/src/lib.rs`（1 686 → **1 745** 行，净 +59）。
- §5.4 超时表新增 `("exec", TOOL_SET_EXEC_TIMEOUT_MS = 60_000)`，出处写死 = `plugins/src/types.rs:837 DEFAULT_TOOL_TIMEOUT_SECS = 60`（用于 `:867` / `:955`），与 spawn/沙箱通道同源；`run_turn` 保持 900_000。
- 测试 `seam_timeout_override_table_is_evidence_bounded` 改为**双向锁**：表内每一项必须指到盘上出处且返 `Some`；名单外 22 项仍逐字断言 `None` ⇒ 总覆盖面不降（23 个 op 仍全部被断言，只是判据从「恒 None」变成「按有无证据分两档」）。
- 握手注释按裁定二改写（「与 `AGENT_LOOP_RUN_TURN_TIMEOUT_MS` 一样缺实测分布」），常量值 / 语义 / describe 不进表均未动。
- 未做：`chat` / `embed` / `send` / `dispatch` 仍无 op 级实测 ⇒ 留在表外（注释保留「15 s 在高端会误杀、预算待实证」）。

## 35. `#77` 交付账：srcdoc 容器首挂载必挂（功能不可用）已修（2026-11-02；报告 `.tmp/res77-report.md`，11 272 B）

根因（#64 取证）：`loading` 初值 true 时早退 `<SkillLoadingSkeleton/>` ⇒ iframe 不在 DOM ⇒ `loadSandbox()` 读 `iframeRef.current` 恒 null ⇒ `skill.iframeNotReady`。
- `SkillSandboxContainer.tsx` 408 → **442** 行：iframe **无条件渲染**；`loading` / `error` 改 `absolute; inset:0` 覆盖层；新增 `loadNonce` 作 `key` 强制重导航；cleanup 清 `srcdoc` + 摘监听 + 拆桥 + 清定时器。
- 保留语义：`skill.timeoutError` 30 s 闸、`skill.iframeNotReady` 文案/错误码、`SkillErrorFallback` 展示条件均未改；i18n 0 新增。
- 测试：新增 5 条（首挂载 iframe 在 DOM、骨架覆盖层、错误+重试换 key、30 s 闸、卸载回收）。**先红后绿**：修前 EXIT=1 / `Tests 5 failed (5)`（首条 `expected length 1 got 0`）；修后 EXIT=0 / `Tests 5 passed (5)`；#64/#67 相关 7 文件回归 116 passed。
- 未做：真机 WebView2 像素复核（jsdom 证据）；build / e2e / 全量 vitest 按约定未跑。

## 36. `#54` 交付账：能力面 26 → 23 + 债档 + 预留行（2026-11-02）

- 出组三条：`workflow.node` / `memory.provider`（B 10→8）、`context.engine`（C 5→4）⇒ **能力面 23 = A 11 + B 8 + C 4**；三条移入 **§0.7.9 债档**（只登记不计数，裁决列逐条写明；生产实现者 32 / 1 / 0）。
- `rag.vector.store` 占位行进 §0.7.7 K 组（K 9→10），`capacityRegister` 不变。
- 登记表 `totalRows` 36 → **37**（36 + 1 条占位）；镜像表 `caliber.groupCounts` 增 `D: 3`，门的分组表同步增 D 档（自检 17/17 仍通过）。
- 原 §24.1 写的 `totalRows` 38 是算术笔误（移出不增减总行数：36 + 1 = 37），本处就地订正；其余仍写 26/24/13/15 的段落分两类：① 本批已改的当前态（§0.7.2 / §0.7.4 / §0.7.5 / §0.7.8 / §0.9 / §11 / §13.1）；② **历史快照**（§12.x / §13.x / §24.1 的推理过程），保留原数并就地标注「已被 §24.1/§0.7.9 取代」，不重写历史。
- 判据：`node scripts/check-capability-register.mjs` = EXIT 0 · `hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`；`--selftest` EXIT 0。
- **`#71` 一并结案**：`MockPlatformAdapter@src-tauri/src/init/notification_adapters.rs:150` 已被新 test 列机械指认（`platform.adapter` 的 10 里唯一落在 `#[cfg(test)] mod` 的站点），**不剔除**（剔除会重算全表实现者数，收益未被证明）；「改判直接接 test 列」的原意已落地。

## 37. `#78` 交付账：bundle 客户端三件同批（2026-11-02；报告 `.tmp/res78-report.md`，10 902 B）

- **来源校验**：`pluginClientBridge.handleMessage` 第一层改为身份判据（复用 `sandboxFrameProtocol.ts` 的 `isFrameFromSandboxSource`，与宿主接收侧/srcdoc 运行时**同一条谓词**）；期望来源为空 ⇒ 一帧都不认。对抗用例：外来来源用猜到的 `callId` 伪造响应 ⇒ 不 settle（修前红：`expected 'resolve:stolen' to be ''`）。
- **dispose 不可复用**：新调用立即拒绝且不发帧、在途 Promise 立即以「已释放」拒绝、`handleMessage` 首行 `if (disposed) return;` 兜住被捕获 listener 的迟到帧。三态各一条用例（(a)(b) 修前已绿，(c) 为新增兜底）。
- **门 C 去文本耦合**：`sandboxFrameProtocol.ts` 新增 `SANDBOX_SRCDOC_RPC_OPS`（方法/参数只存键，值仍单点定义）；`sandboxTemplate.ts` 产物注入 `/* AX_RPC_OPS_BEGIN */ var AX_RPC_OPS = <JSON>; /* AX_RPC_OPS_END */`，`window.ctx` 由该表生成（表即行为，不是旁挂说明）；测试改为解析数据，保留分母断言「op 数 == 协议方法数」且标记缺失/数据清空即红。
- **偏离派单并报备**：表放在 `sandboxFrameProtocol.ts` 而非 `pluginBridgeProtocol.ts` —— 后者已 import 前者，反向放会成环；单一定义点要求已满足。
- 验证：修前红 1 例（客户端）+ 4 例（门 C/E）；修后 `3 files / 76 passed`，相关组件回归 `4 files / 46 passed`；`tsc --extendedDiagnostics` EXIT=0（7479 files，error TS 0）、`oxlint` 0 警告（1442 files）、`dprint check` 7 文件 EXIT=0。
- 未做：真机 WebView2 验证（假 window 证据）；生成式 `ctx` 的 `Function.length` 由 2 变 0（仓内无消费点，未加兼容垫片）；~~门 C 的 bundle 侧仍是源码文本提取（另立任务）~~ ⇒ **已由 `#79` 收口（2026-11-02，见 §40）**：bundle 侧改成实例化真客户端的行为探测，不再扫源码正则。

## 38. `#62` 交付账：两组重复 trait 合流 + ToT 改名，K1 读数 317 / 233 已重钉（2026-11-02；报告 `.tmp/res62-report.md`，18 453 B / 204 行）

### 38.1 改动面（8 文件，全部在职责内）与「每名 1 份定义」

| # | 文件 | 改动 |
|---|---|---|
| 1 | `axagent-core/crates/harness/src/platform_adapter.rs` | `ProviderRepository` / `SettingsRepository` 保留为**唯一权威**；后者补并集方法 `get_setting` / `set_setting`（默认实现返 `NotFound`） |
| 2 | `axagent-core/crates/harness/src/repositories.rs` | 删两份内联定义，改 `pub use crate::platform_adapter::{ProviderRepository, SettingsRepository};` |
| 3-4 | `axagent-infra/crates/dao/src/{provider_repository,settings_repository}.rs` | 实现方错误类型 `String` → `AxAgentError` |
| 5 | `axagent-runtime/crates/rt-workflow/src/work_engine/executors/llm_classifier_executor.rs` | 测试替身错误类型适配 |
| 6 | `axagent-ai/crates/agent/src/tree_of_thoughts.rs` | ToT trait 改名 `LlmReasoningProvider` → `TotReasoningProvider`（本文件 12 处） |
| 7 | `axagent-ai/crates/agent/src/lib.rs` | 根导出改名 + **兼容别名** `TotReasoningProvider as LlmReasoningProvider`（见 38.3） |
| 8 | `axagent-ai/crates/agent/tests/agent_integration_tests.rs` | 导入与 10 处使用点改名 |

**取证（全仓 8 根 `^pub trait` 四名逐个跑）**：`ProviderRepository`=1（platform_adapter.rs:29）、`SettingsRepository`=1（:61）、`LlmReasoningProvider`=1（react_engine.rs:88，ReAct 侧，语义不动）、`TotReasoningProvider`=1（tree_of_thoughts.rs:626）⇒ 原始命中 **6 → 4**。其余 6 根零命中。**选 platform 侧保留的依据是方法集超集 + 实现方 6:3 + 契约面已是根导出**，报告 §2 逐条列出（不是「先到先得」）。

### 38.2 我采纳它的两处裁定（含一处任务文本自相矛盾）

1. **`resolve_model_for_node` 的 Display 前缀变化（`xxx` → `Internal error: xxx`）⇒ 接受，不追改**。依据是报告 §3.3 的**零文本匹配**核查：调用点全是 `.map_err(|e| anyhow!("{}", e))` / `.ok()?` / `format!("{e}")`，`is_non_retryable_error` 走错误码不走文本。⚠ 但**保留一条代价**：这是**用户可见错误串的变化**，若日后有 UI 直接展示该路径的错误，会多出一个英文前缀 ⇒ 归错误码 i18n 规范处理（`AGENTS.md`「后端错误码 i18n 规范」），不属本批。
2. **任务文本里「原始命中 319 → 317」与「ToT 改名…仍 318」自相矛盾 ⇒ 按实测 317 记**。319 − 2（删两份重复定义）= 317，改名不增删定义行；`--dump-readings` 一次运行复现 317，故 318 那条口径**作废**（不是「两个都对」，是派单写错了一位）。

### 38.3 兼容别名：接受，且**必须**在同批写下来龙去脉

`agent/src/lib.rs:212` 的 `pub use tree_of_thoughts::TotReasoningProvider as LlmReasoningProvider;` 是**别名不是第二份定义**（`^pub trait` 仍 1 处），存在理由 = 禁改面 `src-tauri/src/commands/agent/mod.rs:2246` 仍写 `Arc<dyn axagent_agent::LlmReasoningProvider>` 且语义是 ToT。⇒ **不改禁改面**是铁律优先；别名保证该路径解析到 ToT trait，与改前同义。
⚠ **删除窗口已钉死**：#68 把该行改为 `TotReasoningProvider` 后**同一批**删别名（已在 #68 派单清单里写明）。**不许**在没有改 src-tauri 消费点的情况下单独删 —— 那会直接击穿主 crate 编译。

### 38.4 读数重钉：317 / 233，且 §31 第 3 条的「先查是不是 #62」已应验

`--dump-readings` 实测 `{"rsFiles":2009,"pubTraits":317,"externalImplTraits":272,"declaredExternalTraits":233}`，于是同批改三处：

| 位置 | 旧 | 新 | 归因 |
|---|---|---|---|
| PLAN:247 / 镜像 `k1RsFileCount.planPinned` | 2009 | **2009**（不动） | 无文件增删 |
| PLAN:247 / 镜像 `k1PubTraitCount` | 316 | **317** | −2（删两份重复定义）+1（ToT 产生新唯一名）= 净 +1 |
| PLAN:248 / 镜像 `k1ExternalImplTraitCount` | 232 | **233** | ToT 一分为二，两端仍各有非 harness 生产实现（`tree_of_thoughts.rs:771/827`、`react_engine.rs:135/392`）⇒ 交并集 +1 |
| 镜像 `k2KeyedFaces` / `k2TraitFaces` / `pipeline` 的扫描面 | 316 / 232 | **317 / 233** | 同上，仅换分母 |
| 镜像 `k2TraitFaces.note` 流水线 | `316 → 232 → 51 → 23` | **`317 → 233 → 51 → 23`** | 51 与 23 不动（K2 强 37 / 弱 8 实测不变） |

⇒ **§31 第 3 条「收口时若 232 与现树不符，先查是不是 #62」应验**：差值正是 #62 的 +1，不是并发会话或谓词漂移。**重钉后判定档 `node scripts/check-capability-register.mjs` EXIT=0**（`登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项`）—— 232 的 `hard:true` 没被回退，也没被洗绿。

### 38.5 验证与未做

- 定点：harness 831 passed · agent 1936 passed · `agent_integration_tests test_tot` 11 passed · rt-workflow `llm_classifier` 18 passed；check 含 `--all-targets` 与 **src-tauri 主 crate 18m28s 实编**（37 个 axagent crate，warning/error 计数 0）。日志均在 `.tmp/res62-check-*.log` / `.tmp/res62-test-*.log`，编译前 `touch` 过目标文件（排除「只吐 Finished」假绿）。
- **未做**：`cargo fmt` / 全量 clippy / 全量 test（分层门禁约定，留给阶段末）；删兼容别名（#68）；把 `repo::provider::resolve_model_for_node` 本身改成 `AxAgentError`（Display 前缀要逐字不变时的备选，改动面更大，不取）。

## 39. `#75` 交付账：`plugin_list_pages` 列表投影（2026-11-02；报告 `.tmp/res75-report.md`，9 117 B）

### 39.1 改动面（6 文件，逐形对标 dashboard 投影）

| 文件 | 改动 |
|---|---|
| `axagent-plugins/crates/plugins/src/types.rs:181` | 新增 `PluginPagesInfo`（`rename_all = "camelCase"`，`pages: Vec<PluginPage>`） |
| `axagent-plugins/crates/plugins/src/manager.rs:628` | `list_plugin_pages()`：遍历 `list_plugins()` → 单包 `load_plugin_from_directory` 失败只 `warn!` 跳过 → `pages.is_empty()` 跳过 ⇒ **无第二套 registry / 缓存 / 路由表** |
| `src-tauri/src/commands/plugin.rs:509` | `plugin_list_pages`（`#[agent_command(domain = plugin, safety = Safe, call_mode = StateOnly)]` + `spawn_blocking` + `blocking_read`）；`register_commands.rs` 由 `build.rs` 自动生成，未手改 |
| `src/types/index.ts:2123` | `PluginPagesInfoDto`，`pages` 复用既有 `PluginPageDto` |
| `src/lib/browserMock.ts` | `plugin_list_pages` ⇒ `[]`（与 `dashboard_list_plugins` 同口径，不抛错） |
| `src/components/plugin/PluginPageHost.tsx` | 只订正「列表侧无枚举面」那段过期注释；渲染逻辑未动 |

### 39.2 验证与三点要注意的

- `cargo check -p axagent-plugins` EXIT=0（1m22s）；`list_plugin_pages` 用例 `1 passed`；`cargo check -p axagent` **15m48s** EXIT=0；定点 clippy（含 test target）EXIT=0；tsc（`subst` 映射盘后）`Files: 7480` / error 0；oxlint / dprint EXIT=0。
- **测试③用「registry 有记录但安装目录不存在」覆盖**，不是「manifest 损坏」：后者会让 `list_plugins()` 整体报错，`warn` 分支只在 TOCTOU 窗口可达（报告 §⑥-1 给了源码判据）⇒ 这个用例证明的是「陈旧记录被发现期剔除」，不是「坏 manifest 被跳过」，两条不能混读。
- 首轮 clippy 红两次是**并发编辑的瞬时态**（`worker.rs` 正被 `#56` 改写），`#56` 停笔后重跑全绿 —— 这不是「clippy 不稳定」，是**共享工作树上「读数必须晚于写者停笔」**的又一实例。
- `cargo fmt -p axagent-plugins --check` EXIT=1，10 处 diff 与 HEAD 同集（存量债），归 `#69`。

## 40. `#79` 交付账：门 C 的 bundle 侧改行为探测（2026-11-02；报告 `.tmp/res79-report.md`；提交 52d894504）

### 40.1 为什么必须改（旧门是「看着在、实际会漏」）

旧 `methodsUsedByBundleClient()` 用 `stripComments(sourceOf("src/sdk/pluginClientBridge.ts"))` + 正则 `/SANDBOX_RPC_METHODS\.([A-Za-z][A-Za-z0-9]*)/g` 从**源码文本**提取方法面。分母断言只能挡「零个」，**挡不住「少一个」**：客户端一旦把方法名解构、起别名、提到常量，提取集合就静默变小，而门 C 的对称差集照旧为空的形态正是「两边都少同一个」⇒ 静默通过。

### 40.2 新形态与三层抛错

实例化 `createAxAgentPluginClient({ postTarget, messageSource, hostWindow })`（测试注入点，`#78` 已留），逐个调用公开方法 `invoke` / `uiAction` / `executeCommand`（+ `announceReady` 只用于负例），从**投递帧**的 `method` 读方法面；帧是同步投递，调用返回即边界确定。三条机械判据：

1. 一帧都没收到 ⇒ `throw`（不返回空集）；
2. 帧里都没有 `method`（ready 帧必须被滤掉）⇒ `throw`；
3. 某个**被启用**的 RPC 调用自己没发帧 ⇒ `throw` —— 这一条堵的是「漏调被另外两个凑齐」。

保守留 `api.invoke` 的收窄断言（`PLUGIN_BRIDGE_CLIENT_METHODS`），现在它**同时**证明三个公开方法共用同一个 RPC 方法。

### 40.3 取证与验收

- **新旧逐字对照**：删旧提取器之前，在同一棵树上跑两边并断言集合相等（`["api.invoke"]`，37 passed）⇒ 证明「今天旧门还没漏」，改动是防将来而不是修既有假绿。
- 负例自证：白名单外命令（投递前拒绝）⇒ 抛「一个请求帧都没收到」；只发 ready ⇒ 抛「都没有 method 字段」。复用同一个探测器，未写平行实现。
- 最终：`sandboxFrameProtocol` 36 passed / EXIT=0、`pluginClientBridge` 28 passed / EXIT=0、tsc `Files: 7479` / error 0、dprint / oxlint EXIT=0。**我独立在 `C:\Users\Public\axinvest-vitest` 镜像复跑**：md5 与仓库逐字一致，`36 passed / EXIT=0`。
- `stripComments` / `sourceOf` 仍被门 A/E/F 使用 ⇒ 保留；生产代码零改动。
- 执行者报的偏离（记下来免得下一个人踩）：`node_modules/.bin/*` 是 bash shim，`node` 直接执行会 `SyntaxError` ⇒ 前端工具要用包真实入口（`dprint/bin.cjs`、`oxlint/bin/oxlint`）。

## 41. `#56` 交付账：写 worker stdin 不再持锁阻塞（2026-11-02；报告 `.tmp/res56-report.md`；提交 3b4c5c58c）

### 41.1 改动面（唯一文件 `worker.rs`，+741 / −162）

| 落点 | 内容 |
|---|---|
| `WRITER_EXIT_GRACE = 200ms` | 杀进程后写线程的退出宽限；超时**detach**并 warn，绝不为了 join 把停机拖成无界 |
| `OutboundFrame` + `run_writer` | 专用写线程**独占** stdin，请求帧与入站回写共用一条 FIFO 串行写者 ⇒ 帧完整性由「单写者」保证，不再由调用方持锁保证 |
| `WriteAck { Written, Failed{wrote_any}, NoAck }` | 三态写回执；`WorkerInner.stdin: Mutex<...>` 删除，换成 `writer: Mutex<Option<WriterThread>>` |
| `WorkerInner::submit_frame` | 调用方只克隆投递口（持锁极短），随后**不持任何锁**做带预算的等待 |
| `poison()` / `close_writer(deadline)` | 超时或部分写出 ⇒ 置可疑闸 + `kill_child()`（关管道读端是解阻写者的唯一手段）；停机先 Close、宽限内不退则杀进程再收 |
| `exchange_with_timeout(wait: Duration)` | `wait: Option<Duration>` 直接改 `Duration`，**删除无限 `exchange()`** —— 把「无限等」从类型里去掉，而不是留一个分支等人复用 |
| `shutdown()` / `Drop` | 礼貌帧预算 = 剩余预算的一半；整体落在 `shutdown_timeout` 量级；Drop 顺序 kill → close_writer → join_reader |

### 41.2 判据表（这是本任务的核心，别的都可以重构，这张表不能变）

| 情形 | 报什么 | 为什么 |
|---|---|---|
| 可疑闸拒投（**未尝试写**） | `Transport` | 毒化连接的口径是「拒绝」，与「进程不在 ⇒ 回退」方向相反 |
| 写回执 `Failed { wrote_any: false }` | **`NotDelivered`** | 零字节进管道 ⇒ 帧不可能被执行 ⇒ 零副作用 ⇒ 命名命令可回退（#73 M-1） |
| 写回执 `Failed { wrote_any: true }` | `Transport` + `poison()` | 已有字节被接受，对端可能读到半帧，流边界不再可信 |
| 写回执 `NoAck`（预算耗尽） | `Transport`（`submit_frame` 已毒化+杀进程） | **「未送达」不可证明**：写者还阻塞在 `write` 里，写出进度不可追溯 |
| `Written` 后收侧到点 | `Timeout` | 「超时不等于取消」原样保留 |

⇒ `NotDelivered` 从此**只出现一格**。这条比「修好卡死」更重要：把回退判据从「写失败」收紧成「确定零字节」，否则一次半帧写出就会让上层误以为可以重跑命令（二次副作用）。

### 41.3 用例（3 条新增，`worker::tests` 42 例全绿）

1. `write_ack_timeout_is_bounded_and_never_reports_not_delivered`：阻塞写夹具下调用方在预算内返回、错误非 `NotDelivered`、`seam_suspect == true`、第二次调用毫秒级拒绝且**写计数不增**、`shutdown()` 在写者仍卡死时 < 1s。
2. `healthy_writer_delivers_the_exact_frame_and_closed_stdin_reports_not_delivered`：健康往返帧**逐字节**等于 `serde_json::to_vec(&request)`；对端关闭后投递 ⇒ `NotDelivered` 且文本含「stdin 已释放」；配对表无残留。
3. `inbound_writeback_is_bounded_while_an_outbound_write_is_stuck`：出向写卡住期间读线程照常派发入站任务（读侧不被写侧阻塞），入站回写在自己的预算内收口，出向调用同样在预算内失败且非 `NotDelivered`。

### 41.4 一次测试挂死：根因是夹具，不是生产死锁（必须记，因为它是**误判高发形态**）

首轮 `cargo test` 在 `healthy_writer_...` 上**挂死 5 分钟**（日志停在 `test ... ---`，无 ok/FAILED）。根因：夹具把**对端 socket** 当宿主写端 ⇒ 帧回到宿主自己、假 worker 收不到 ⇒ 5 s `Timeout` panic；而 panic 展开时 `Drop for WorkerInvoker` 的 `join_reader()` 对端尚未关闭 ⇒ 读线程永不 EOF ⇒ **进程挂死而不是 FAILED**。
处置：写端改回 `Box::new(host_side)`（与 `command_transport` 同形）、用例体包 `with_watchdog(10s)`（未来回归表现为明确失败而不是挂死），保留「先关对端再 shutdown」。**生产逻辑在编译/测试期零修正**：首轮 `check` / `clippy --all-targets -D warnings` 即 EXIT=0。
⇒ 判据：**测试挂死先查夹具的方向与收尾，再怀疑生产死锁**；同时「panic 展开路径里有无界 join」本身是既存风险（纯流式夹具没有子进程可杀）—— 本次未改 `join_reader`，由每个用例的 watchdog 兜住，登记为残余。

### 41.5 两条残余（登记，不假装已解决）

1. **写回执预算按「投递→回执」计，含排队时间**：若写者前面压着一帧接近 64 MiB 上限的大帧，后面预算最小的命令帧可能在健康连接上提前 `NoAck` ⇒ 毒化+杀进程（假阳性）。触发条件苛刻，本仓现有载荷远小于此；缓解选项（写预算下限 / 写者回报「开始写」）都会改「预算只归 worker」的口径，留裁。
2. **`join_reader()` 仍无界**：其有界性依赖「进程已死 ⇒ stdout EOF」；纯流式夹具必须由用例自行关对端。若要连读线程 join 也带 deadline，需给读线程加退出信号通道，未做。
3. **入站总闸未加**（按 §32.3-1 裁定）：入站服务侧本就没有等待上限，本批没有新增无界；互饿评估结论是「单写者 FIFO，不互等；唯一延迟是 head-of-line」，加闸会重新引入 M-3 误杀。

## 42. `#60` + `#68` 交付账：超时不再引导重试、ToT 别名已删、主 crate `--all-targets` 窗口补齐（2026-11-02；报告 `.tmp/res60-68-report.md`；提交 77af74ed8）

### 42.1 `#60`：`CommandTimeout` 从 `Retryable` 改 `General`

| 面 | 改前 | 改后 |
|---|---|---|
| 分类 | `ErrorCategory::Retryable` | **`General`**（默认档：仍按 error 展示，但不进前端自动重试分支） |
| `detail` | 原样错误串 | 追加「；超时不等于取消：插件可能仍在执行，重试可能导致副作用重复」 |
| 文档 | 「调用方可原样重试」 | 「重试前必须确认命令幂等」——**旧句是假话**，因为 `worker.rs` 同文件就写着「超时 ≠ 取消」 |
| 用例 | `command_timeout_maps_to_retryable_common_timeout` | `command_timeout_is_not_marked_retryable`：code 仍 `COMMON_TIMEOUT`、category=`general`、detail 含「超时不等于取消」 |

⇒ 判据：**「没等到结果」不等于「没发生副作用」**。凡是可能已执行的操作，分类只能表达「可诊断」而不能表达「可自动重试」；`Retryable` 的语义是「重试是安全的」，这里不成立。

### 42.2 `#68`：兼容别名删除 + 主 crate 定点窗口

- `src-tauri/src/commands/agent/mod.rs:2246` 改用 `axagent_agent::TotReasoningProvider`；`axagent-ai/crates/agent/src/lib.rs` 删除 `#62` 留的 `TotReasoningProvider as LlmReasoningProvider` 别名。
- **全仓命中分类**：活源码里 ToT 旧名残留 **0**；其余 `LlmReasoningProvider` 全部属于 ReAct 侧自有 trait（`react_engine.rs` 及其测试、`init/llm_providers.rs` 的显式 `react_engine::` 导入）⇒ 保留。两个同名 trait 从「靠别名共存」变成「各在各的模块里」，这正是 `#62` 想要但当时被禁改面挡住的状态。
- **窗口结果**：`cargo check -p axagent --all-targets` **EXIT=0 / 23m08s**（此前 #37/#40 之后主 crate 只编过默认 target，测试 target 从未编过）；`cargo check -p axagent-agent` EXIT=0；`cargo test -p axagent --lib commands::plugin` **EXIT=0 / 27m52s / 5 passed**（测试 exe mtime 晚于源码，非旧产物）。
- 口径备注：任务原文写「四条命令」，实际是 **3 条 cargo 命令 + 1 条全仓 grep 取证**（UNC 下 `grep -rn` 超时，改 Grep 工具分根复查，结论一致）——**不要**把「条目数」当成「命令数」，这类计数错误在本 PLAN 已出现过（§31.4 的 7≠6）。
- 编译耗时新基线：主 crate **单编 23–28 min**（check `--all-targets` 23m08s / test 27m52s）⇒ 后续涉及 `src-tauri` 的任务排期按 25 min 量级估，不要再按 16m47s 估。
