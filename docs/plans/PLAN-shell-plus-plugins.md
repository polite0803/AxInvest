# PLAN · 主壳 + 按能力插件（= `PLAN-everything-is-plugin.md` 的 P5 阶段）

> 状态：active ｜ 日期：2026-11-03 ｜ 对应版本：v3.0.0+（未定版，待确认）


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
判不住股票包（304 条命令 + 57 个工具 + 294 个前端文件 + 20 张表）。终局有**两类可插单元**，判据不同：

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
| K1：`pub trait` 总数（8 根 2045 个 `.rs`） | **327** | 含引擎内部变化点、测试替身、serde 派生面。**2026-11-01 两档各降 1**：`#30` 删除 `harness/src/dojo_extension.rs` 整文件（−460 行），同时少 1 个 `.rs` 与 1 个 `pub trait DojoExtension`。**2026-11-02 文件数升 1（2004→2005）**：`#49` 新增命令帧 e2e 桁块 `axagent-plugins/crates/plugins/src/bin/e2e_command_frame_worker.rs`；同批另一份 `crates/plugins/tests/e2e_command_frame_test.rs` 落在 `tests/`，被扫描面的 `K1_IGNORE` 排除 ⇒ **加 2 个 `.rs` 只升 1 个读数**（这一条是口径事实，不是数据漂移）。权威读数 = `node scripts/check-capability-register.mjs --dump-readings` 的 `k1.rsFiles / k1.pubTraits`。**2026-11-02 收口重测：2009 / 317**（对 HEAD 2004 / 315 的净增溯源：`.rs` = +7 新增〔harness `vector_search.rs`、`vector_store.rs` + 5 个 `plugins/src/bin/e2e_*_worker.rs`〕−2 删除〔两份 `ir_renderer.rs`〕；`pub trait` 唯一名 = +3〔`VectorCollectionAdmin` / `VectorWriter` / `VectorReader`〕−2〔`IrRenderer` / `KitSlashCommandProcessor` 随文件移除〕⇒ 净 +1；**`#62` 再净 +1（316→317）**：ToT 侧 `LlmReasoningProvider` 改名 `TotReasoningProvider`（+1 唯一名），`ProviderRepository` / `SettingsRepository` 两处重复定义并回 1 处（原始命中 −2，唯一名数不变））。**2026-11-02 `#81` 再升 1（2009→2010）**：行情 DTO 上收新增 `harness/src/stock_market_types.rs`；`astock-data/src/types.rs` 保留为垫片（未删文件），故只 +1。**同日 C 批再升 1（2010→2011）**：`DataOutcome` 上收新增 `harness/src/data_outcome.rs`；`astock-data/src/outcome.rs` 保留为垫片。**同日 `jobs.store` 接缝再升 2 项**：新增 `harness/src/jobs_store.rs`（文件 +1）与 `JobStoreProvider`（trait +1）⇒ **2012 / 318**。**同日 `workflow.run` 再升 2 项**：新增 `harness/src/workflow_run.rs`（文件 +1）与 `WorkflowRunProvider`（trait +1）⇒ **2013 / 319**。**同日宿主服务再升 1（2013→2014）**：新增 `src-tauri/src/commands/stock_workflow/workflow_run_service.rs`。**同日 `trading.engine` 再升 2 项**：新增 `harness/src/trading_engine.rs`（文件 +1）与 `TradingEngineProvider`（trait +1）⇒ **2015 / 320**。**同日宿主服务再升 1（2015→2016）**：新增 `src-tauri/src/commands/stock_workflow/trading_engine_service.rs`。**2026-11-03 `stock.monitor` + `quote.watcher` 契约再升 4 项**：新增 `harness/src/stock_monitor.rs`、`harness/src/quote_watcher.rs`（文件 +2）与 `StockMonitorProvider`、`QuoteWatcherProvider`（trait +2）⇒ **2018 / 322**；同批从 `analysis-engine/src/monitor.rs`、`cross_stock_aggregator.rs`、`astock-data/src/realtime_quote.rs` 删定义改 `pub use`，垫片文件保留不删 ⇒ 文件数不降。**同日 `#107` 契约 + `#103` 宿主再升 4 项文件 / 2 项 trait**：`harness/src/knowledge_vector.rs`、`harness/src/trajectory_store.rs`（+2 文件 +2 trait ⇒ 324）与 `commands/stock_workflow/monitor_seam_service.rs`、`quote_watcher_service.rs`（+2 文件）⇒ **2022 / 324**。**同日 `#114` 再升 1（2024→2025）**：新增 `axagent-infra/crates/dao/src/repo/trade.rs`（`exists_same_key` 下沉），同批 `market_data.rs` 只做格式、`workflow_run_service.rs` 只换 `DatabaseConnection` 导入源 ⇒ 其余读数不动。**2026-11-03 B1（`#100`）再升 5（2025→2030）**：`harness` 新增 `task_cancel.rs` / `trading_calendar.rs` / `candlestick_pattern.rs` / `divergence.rs` / `news_archive.rs` 五个模块，`astock-data` 原文件全部保留为 `pub use` 垫片（未删文件）⇒ 只 +5；`pub trait` 唯一名不动（324，`NewsArchiveSink` 只是换文件）。**2026-11-04 `#134` 第二片再升 2（2030→2032）**：新增 `axagent-infra/crates/dao/src/reconcile/external.rs`（插件 `schema.json` 解析）与 `src-tauri/src/init/plugin_schema.rs`（宿主挂载点）；两个文件都没有新增 `pub trait` ⇒ 唯一名仍 324。**2026-11-04 B4-a 骨架再升 1（2032→2033）**：新增 `axagent-plugins/crates/plugin-stock/src/main.rs`（插件侧 worker；纯可执行包、无 `lib.rs`，分层门禁规则 4 对纯可执行包有 `schema-gen` 先例）；同 crate 的 `tests/protocol.rs` 落在 `tests/`，被 `K1_IGNORE` 排除 ⇒ 加 2 个 `.rs` 只升 1；没有新增 `pub trait` ⇒ 唯一名仍 324。**2026-11-04 B4-g 迁出 `intent_parser`（2033→2034）**：新增 foundation crate `axagent-plugins/crates/stock-core`（`src/lib.rs` + `src/intent_parser.rs` = +2），删除 `axagent-quant/crates/analysis-engine/src/intent_parser.rs`（−1）⇒ 净 +1；`analysis-engine` 改路径级 re-export，宿主 `stock_analysis.rs` 一行未改；无新增 `pub trait` ⇒ 仍 324。**2026-11-04 B5 净 +2 文件 / +1 trait（2034→2036 / 324→325）**：`tools/src/tools/astock_data.rs` 换成通用包装 `tools/src/tools/domain_mcp.rs`（改名不增减），新增 `tools/src/domain_tools.rs`（槽位，含 `pub trait DomainToolExecutor` ⇒ +1 唯一名）与 `src-tauri/src/init/stock_domain_tools.rs`（域侧注册 + 执行器实现 ⇒ `k1ExternalImplTraitCount` +1，同一 trait 名只计一次）⇒ 文件 +3 −1 = +2。**2026-11-04 B6 再升 1（2036→2037）**：新增 `src-tauri/src/init/stock_domain_state.rs`（六字段持有 + `AppState::stock()`）；无新增 `pub trait` ⇒ 唯一名仍 325。**2026-11-05 `analysis.record` 接缝再升 2（2037→2039 / 325→326）**：新增 `axagent-core/crates/harness/src/analysis_record.rs`（契约，含 `pub trait AnalysisRecordProvider`）与 `src-tauri/src/init/analysis_record_service.rs`（宿主 provider）；同批 `harness/src/lib.rs` / `capability_registry.rs` / `plugins/src/worker.rs` / `plugin-stock/src/main.rs` 只改既存文件 ⇒ 文件数只 +2。**2026-11-05 `#36` 第 1 批骨架再升 1（2041→2042，trait 两读数不动）**：新增 `axagent-plugins/crates/plugin-opc/src/main.rs`（OPC worker 骨架，纯可执行包、无 `lib.rs`）；同 crate 的 `tests/protocol.rs` 落在 `tests/`，被 `K1_IGNORE` 排除 ⇒ 加 2 个 `.rs` 只升 1。声明是空表（零 `capabilities` / 零 `calls`）⇒ 零新增 `pub trait`，唯一名仍 326、外部实现者仍 242。**2026-11-06 对账批重放（`3fe49c950`，#36 第 3–4 批）**：文件数再升 3（2042→**2045**）= 新增 `harness/src/opc_store.rs` + `dao/src/repo/opc_blog_post.rs` + `src-tauri/src/init/opc_store_service.rs`；`pub trait` 唯一名再升 1（326→**327**）= `OpcStoreProvider`（`opc_store.rs:141`）；外部实现者再升 1（242→**243**）= `HostOpcStore` 在 `src-tauri/src/init/opc_store_service.rs:93` 有生产 impl。权威读数 = `node scripts/check-capability-register.mjs --dump-readings` 的 `k1.rsFiles / k1.pubTraits / k1.declaredExternalTraits`（⚠ 不是同段的 `externalImplTraits`）。 |
| K1 + 有非 harness、非测试实现者 | **243** | ⚠ 这一档**不能**当能力数。~~232 出自一次性 census 工件（`.tmp/traits-full.json`，未入库 ⇒ 谓词无法从仓内代码复现）~~ ⇒ **该说法已作废（`#61` + `#66` 落地后）**：谓词已入库为 `k1DeclaredExternalImplTraitCount`，`--dump-readings` 在现树复现 = **233**（`#62` 后：ToT 拆成两名、两端都仍有生产 impl ⇒ +1），且与本门 audit 侧输出**排序名单逐字相等**（`#66` 的 4.2 档，差分探针 + 两条命令背靠背 + 扫描面常量逐项对照，三条同源性证据齐）⇒ **233 现在是可复算值**，锚点**具备升 `hard:true` 的条件**（升不升、用哪个数由我在收口时一次定）。另两个历史数**只留作解释、不再与 233 并列**：**210** = 单趟扫描缺陷（当时同谓词**两趟交并** = 232，差值恰 **22**；`#62` 后未重测单趟读数）；**269** = 门里最严格的读法（不要求 trait 声明面、不排 std/派生噪声名），**不作文档该锚点值**。⚠ 处置次序仍不能反：**先让谓词可复算，再升 hard** —— 反向必先假红。**同日 `jobs.store` 再 +1（233→234）**：`JobStoreProvider` 在 `runtime-core` 有生产 impl。**同日 `workflow.run` 再 +1（234→235）**：`WorkflowRunProvider` 在 `src-tauri/src/init/workflow_run.rs` 有生产 impl（后迁 `commands/stock_workflow/workflow_run_service.rs`，读数不变）；**同日 `trading.engine` 再 +1（235→236）**：`TradingEngineProvider` 在 `src-tauri/src/commands/stock_workflow/trading_engine_service.rs` 有生产 impl；**同日 `#103` 宿主批再 +2（236→238）**：`StockMonitorProvider` 在 `commands/stock_workflow/monitor_seam_service.rs`、`QuoteWatcherProvider` 在 `quote_watcher_service.rs` 各有生产 impl（`KnowledgeVectorProvider` / `TrajectoryStoreProvider` 仍只有声明 ⇒ 不计，等 `#108`）。**2026-11-04 B5 再 +1（240→241）**：`DomainToolExecutor`（`tools/src/domain_tools.rs`）在 `src-tauri/src/init/stock_domain_tools.rs` 有生产 impl。**2026-11-06 对账批**：再 +1（242→**243**）= `OpcStoreProvider` 有宿主生产 impl `HostOpcStore`（`src-tauri/src/init/opc_store_service.rs:93`，`#36` 第 4 批）。 |
| K1 + K2（键控站点，四次扫描合并去漏报） | **51** 个 trait 面 | 含 15 条已建接缝的 trait。⚠ **2026-11-02 `#33` 实测：这一档不可作论据** —— 谓词真身是**未入库的一次性工件** `.tmp/capability-register3.cjs:44-82`（仓内无处复跑，与 §0.7.2 的 232 同病根），且它要求签名/块内出现 `dyn T` ⇒ 198 条里 **21 条假阴性**，把 PLAN B/C 组落入该桶的 **13 条契约全部吞掉** ⇒ **不许**用「无键控点」论证「这些 trait 只是分层解耦」。谓词入库 = 任务 **#57**；逐条分桶结论见 **§14** |
| 51 → 同族合并 + K3/K4 判定 | **23 条能力**（另有 3 条移入 §0.7.9 债档，只登记不计数） | ⇒ 本节表格 A+B+C 三组 |
| （对照）今天就能远程跑的接缝 | **13** | `axagent-plugins/crates/plugins/src/worker.rs:133` `SUPPORTED_REMOTE_SEAMS` |

> 报数纪律：以后凡是「能力数」，必须连带给出它用的是哪一档口径。单说「26」「27」「12」而不说口径，三者同样不可核对。

### 0.7.3 A 组：接缝已建，机制可用 —— 11 条

| 能力 ID | 契约（定义行） | 键控分发点（证据行） | 内置实现者 | 拆分阻塞 |
|---|---|---|---|---|
| `model.provider` | `axagent-core/crates/harness/src/provider.rs:25` | `axagent-ai/crates/providers/src/registry.rs:24`（`HashMap<String, Arc<dyn ProviderAdapter>>`）+ `:33 register` + `:44 get` | 14 | **D2** 流式：`chat_stream`/`speech_stream` 首帧即错 |
| `tool.set` | `axagent-core/crates/harness/src/registry.rs:130`（契约面 `harness/src/tool.rs:424 Tool`） | `axagent-runtime/crates/tools/src/registry.rs:183` | Tool impl **198** | **D4** `ToolContext` 被丢弃（安全，必须先修） |
| `platform.adapter` | `axagent-core/crates/harness/src/message_adapter.rs:83` + `channel_adapter.rs:25` | `axagent-runtime/crates/rt-messaging/src/message_gateway/platform_manager.rs:16` + `:104 get_adapter(&str)` | 10 | — |
| `message.callback` | `axagent-core/crates/harness/src/platform_callback.rs:21` | 接缝 `message.callback`（内置与插件同入口） | 2 | — |
| `webhook.dispatch` | `axagent-core/crates/harness/src/webhook_subscription.rs:154` | 接缝 + `axagent-infra/crates/rt-webhook/src/webhook_dispatcher.rs:9` | 4 | — |
| `system.prompt` | `axagent-core/crates/harness/src/context_contributor.rs:22` | 接缝 `system.prompt.{段名}` | 3 | — |
| `workflow.reflector` | `axagent-core/crates/harness/src/workflow_reflection.rs:190` | 接缝 | 3 | **D2**：`axagent-plugins/crates/plugins/src/worker.rs:4386` 起 `remote_method_not_supported` |
| `workflow.evolver` | `axagent-core/crates/harness/src/workflow_evolution.rs:172` | 接缝 | 3 | 同上（`axagent-plugins/crates/plugins/src/worker.rs:4478`–`:4526` 一批 `remote_method_not_supported`） |
| `workflow.optimizer` | `axagent-core/crates/harness/src/workflow_optimization.rs:80` | 接缝 | 2 | 同上 |
| `workflow.sandbox` | `axagent-core/crates/harness/src/workflow_evolution.rs:263` | 接缝 | 4 | — |
| `workflow.business_rule` | `axagent-core/crates/harness/src/business_rules.rs:71` | 接缝（已有 e2e worker 产物） | 3 | — |

### 0.7.4 B 组：K1+K2 成立但**没有接缝** —— 8 条，P5.0a 必须新建（另 2 条 `workflow.node` / `memory.provider` 已按 #54 移入 §0.7.9 债档；本表仍保留原行作判据与历史，不计数）

| 建议能力 ID | 契约 | 键控分发点 | 内置实现者 | K4 代价（帧协议/主权） |
|---|---|---|---|---|
| `notify.channel` | `axagent-core/crates/harness/src/notification_channel.rs:195` | `axagent-runtime/crates/notification/src/dispatcher.rs:114` + `:143 register_channel` + `:149 unregister(&str)` | 6 | **最低**：DTO 全可序列化；去重/冷却留宿主 |
| `market.vendor` | `axagent-quant/crates/astock-data/src/vendors/mod.rs:7 StockVendor` | `astock-data/src/lib.rs:837 register_vendor(&str)` + `:1646` 键控取回 | 16 | `AS_OF` task-local 必须改显式入参；门面 `MarketDataProvider`（`harness/src/market_data.rs:205`）不单列 |
| `search.source` | `axagent-ai/crates/agent/src/search_provider.rs:95` | `axagent-ai/crates/agent/src/search_orchestrator.rs:28`（`HashMap<SourceType, Arc<dyn SearchProvider>>`） | 4 | `rate_limit()`/计费属宿主。**2026-11-02 实测分档**：`search()` 侧结果 400–600 B × 默认 `max_results` 10 ⇒ **①，可直接走接缝**；`extract_content(url)` 返回大文本（今天 50 KB 截断）⇒ **②**，需**分页/游标**而非分块。DTO（`search_provider.rs:38-57`、`research_state.rs:416,76`）全 serde✓。前置 **D1** |
| `image.gen` | `axagent-ai/crates/providers/src/image_gen.rs:64` | `name()` 自述键 + providers 注册面 | 2 | **2026-11-02 实测**：`url` 形态 ≈200 B ⇒ **①**；`base64` 形态 **1.3–4 MB × n** ⇒ ②（接近 `MAX_FRAME_LEN = 64 MiB` 的 `plugin-proto/src/lib.rs` 的 `pub const MAX_FRAME_LEN`，但 n 张累加会顶穿）⇒ 裁定取「只回 url / 宿主侧落盘引用」形态，**保 b64 才需要 D2 分块**。DTO（`image_gen.rs:24-36,45-54`）全 serde✓；provider 凭证 scope 留宿主 |
| `document.parser` | `axagent-core/crates/harness/src/search_sources.rs:62` | **证据行改定（§14.4 + §24.2 两次裁定；本行此前仍是旧行）**：`axagent-infra/crates/document-parser/src/lib.rs:533` 只是 ext→MIME 的**字符串映射表**、块内**不构造实现类型** ⇒ 真正的行为选择点是同文件 `:61` 的 `match mime_type`，而该块**同样无构造分支** ⇒ **K2 的 ⑧（factory-match）在结构上永不命中 = 已知漏检族第 4 例**（该形态「按字符串 `match` 选行为、但不构造实现类型」已补进镜像表 `countRule.k2Caveat`）。⇒ **本行准入判据改写为：K1 成立（外部实现者 1）+ K3 成立；不以 K2 命中为准**（`#54` 已裁「检测器跟着登记表走」是禁止方向，见 §24.2 第 1 条） | 1 | 37 分支是**编译期穷举** ⇒ K3 今天要改主壳，正是本条要拆的理由；另有两套 `OnceLock` 全局（`search/src/sources.rs:22`、`tools/src/parser.rs:13`）和一处硬编码绕过（`commands/knowledge.rs:1169`） |
| `rag.source` | `axagent-core/crates/harness/src/rag_provider.rs:7/40`（Embedding/Rerank）+ `axagent-infra/crates/search/src/rag.rs:45 RAGSource` | `search/src/rag.rs:941`（3 分支工厂） | 5（4 生产 + 1 测试替身） | **§0.6.3 未拍**；`VectorStore` 是持 `DatabaseConnection` 的 struct（`search/src/vector_store.rs:127`），sqlite-vec 进程内扩展 ⇒ 出进程即失效 |
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
   原记「CLI 派发口无 `input_schema` 且权限写死 `ReadOnly`」**判据低估了实情**：`PluginTool::check_permission`（`plugins/src/types.rs:892`，fn 起点；原写 `:787-826` 在 HEAD 上就已错位 —— 787 是 `pub fn new(`、822 才是该 fn，区间尾也切在函数体里 ⇒ 改钉 fn 起点）全仓零调用方，唯一引用它的 `execute_with_permission_check` 也零调用方 ⇒ 档位在 tools 与 commands **两条路上都是纯装饰**；本批只把命令通道改成真授权（按具体权限名判定，`implies_*` 一字未改），**tool 侧依旧是装饰**，另立批次。
   同批已修的假话：`manager.rs` 原注释自述「按空白切分为程序 + 参数」是假的（命令串整团交 `Command::new`、`args` 恒空，可执行文件查找按宿主 cwd ⇒ **相对路径声明的 command 今天根本跑不通**，见任务 #47）。CLI 派发口现位于 **`plugins/src/manager.rs:875`**（`pub fn execute_plugin_command(`，`--repin-code --apply` 定锚）。行号链：`:708` → `:738`（#75 加 `list_plugin_pages`）→ `:755` → `:787`（B10-1 前插 `agent_command_specs`）→ `:838`（B8-1b 在其前插 `list_plugin_task_sources` 一族）→ `:875`（#149 第 2 步在 `build_plugin_manifest` 一处加 `load_declared_ui_keys` 与两条 membership 校验，`manager.rs` 净增 37 行）。⚠ 本句原先停在 `:738`，而镜像表登记已随 B10-1 重锚到 `:787` ⇒ **散文引用与镜像表各说一套、两边都不红**（散文不受 evidence 判据管辖），这一条是本批撞出来的既存缺陷，不是新引入的漂移。

### 0.7.7 判为内核，明确不拆 —— 10 条（每条都给不过哪道判据）

| 面 | 不过的判据 | 理由 |
|---|---|---|
| `agent.loop`（`AgentTurnRunner`，已建接缝） | 语义 | 引擎本体。拆掉它就没有主壳，与 §0.5.6「主壳不是插件」直接冲突 ⇒ **接缝留着，实现永远内置** |
| `session.store` / `event.dispatch` | 既定口径 | §9 已定：提供面不远程化（插件是订阅方） |
| `session.log.invariant`（有接缝且可远程） | 语义 | 「模型所见可重建」是宿主对插件的**审计约束**；让被审计者自己出证不成立 ⇒ 留内核 |
| DES 仿真（`market-sim` `SimAgent`/`Oracle`，**27 个实现**，全在 crate 内） | K4 + 语义 | 实现者是同一内核内的 agent 变体；`kernel.rs:703` 每 tick 遍历全部 agents ⇒ 出进程即 tick × agent 次 IPC。**已裁定维持内核**，可插部分下放到「策略数据」层（场景 / 参数 / 脚本）—— 见 §0.12.2 |
| `command.handler`（`src-tauri/src/commands/agent/command_bridge.rs:1292`，15 实现者） | K3 | 主壳 IPC 路由机制本身；命令注册已由 `build.rs` 自动生成（禁区 7） |
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
| 命令 | 前端 57 个 `invoke` 名（约 128 处）；Rust 侧 `register_commands.rs` 共 **1541 条 / 164 模块**（口径 = `scripts/lib/command-domains.mjs` 的 `parseRegistry`，与两份 AUDIT 同源），工作流域（审计 19 域口径）**83 条 / 11 模块**（`workflows`=10、`work_engine`=21、`workflow_template`=16、`workflow_reflection`=8、`workflow_ai`=8…；`stock_workflow` 28 条归股票域，不计入本域） |
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
| Tauri 命令条数 / 命令模块数（**插件边界口径**） | **302 条 / 20 模块**（`stock_analysis` 171、`stock_workflow` 24、`paper` 20） | **137 条 / 13 模块**（`opc` 56、`opc_capability_pack_actions` 20、`_runtime` 14；2026-11-06 对账批 −1 = `3fe49c950` 删宿主 `opc_list_blog_posts`，`opc` 模块 57→56） |
| — 同一注册表在**审计 19 域口径**下的读数 | 304 条 / 19 模块（域「股票分析与管线」） | 116 条 / 10 模块（域「OPC 公司运行时」；2026-11-06 对账批 −1 = `3fe49c950` 删宿主 `opc_list_blog_posts`） |
| **① 主壳反向阻塞**（主壳非域文件引用域 crate ⇒ **必须逐条改写**） | **54 处 / 16 文件**（2026-11-06 对账批：`84d8c357b` 把 48 处股票 crate 直连改指 `axagent_company_runtime::` ⇒ 这 48 处的账整体从股票侧迁到 OPC 侧，股票剩 54） | **48 处 / 3 文件**（`tools/src/tools/opc.rs` 41 + `commands/fleet/mod.rs` 4 + `init/state.rs` 3；原记 0 处是因为它们当时记在股票名下） |
| **② 跨包口**（一个功能包的文件调另一个功能包的 crate ⇒ 走 `call_seam`） | **29 处 / 9 文件**，且**全部是 OPC 的文件在调股票 crate** | 0 处 |
| ③ 域自有文件内引用（随插件整体迁走，**不是障碍**） | 456 处 / 41 文件（2026-11-04 复测：原 444 / 37 的增量来自并行批在域自有文件里加代码，**与 B1/B2 两刀无关** —— 那两刀只动主壳） | 16 处 / 2 文件 |
| Cargo.toml 构建边 | 7 条（`src-tauri` 4 + `tools` 1 + 域内 2；B7b 前 `tools` 2） | 1 条（`src-tauri → axagent-company-runtime`） |
| 前端反向 import（非域 import 域文件） | 47 处 / 34 文件（2026-11-05 #149 第 3 步：删宿主股票源 adapter（`adapters/registry.ts` 与 `__tests__/adapters.test.ts` 各 −1 处）⇒ 49 / 36 → **47 / 34**；49 / 36 出自 B8-2 第一刀 #151，再上一读数 51 / 37 出自 B8-1a） | 28 处 / 11 文件（15 处集中在 `ContentArea.tsx`） |
| 前端体量（**目录口径**：`components/stock-analysis` / `pages/opc`） | `components/stock-analysis` 212 文件 / 60 938 行（2026-11-06 对账批：−1 文件 = `a13bff03d` B8-4a 把 `_shared/PageHeader.tsx` 下沉出目录，其后 `d5572fb65` + `4224098b5` 两批在目录内净 +10 行）+ store 6 文件 / 4 514 行 | `pages/opc` 20 文件 / 8 859 行 + `OpcPage.tsx` 134 行 |
| — 同一注册表在**脚本文件名口径**（`feRe` 匹配 `src/**` 全部 .ts/.tsx）下的读数 | 292 文件 / 84 815 行（2026-11-05 #149 第 3 步：删宿主股票源 adapter（95 行）与详情抽屉（94 行）⇒ −189 行 / −2 文件；同日工作区另有 −209 行 / −2 文件的并行改动，故绝对读数与上一档再差 209 / 2。**2026-11-06 对账批**：再 −44 = B8-5 第二批摘股票域类型门面（`src/types/stock-analysis.ts` 按 split 口径 2 337→2 293，口径内文件数不变）；同日第二轮对账再 +2 = `975652a17` 的 8 个前端文件里只有 `src/lib/seed-stock-schemas.ts`（路径含 `stock`）命中 `feRe`，+4/−2 ⇒ 现值 292 / 84 815；⚠ 上一登记值 84 850 比 `c6c4ce198` 那棵树的实际读数 84 857 少 7 行（该批取的是工作区瞬时态），所以表上差 −37 ≠ 本批代码差 −44） | 40 文件 / 12 797 行 |
| entities 表文件 | 20 个 | 28 个 |
| zh-CN i18n key | 3 527（`stockAnalysis`/`invest`/`quant`/`trade` 是**整段**） | 1 897（`opc` 整段） |

① 的 54 处（含 H 桶 3 处 / 3 文件：`commands/gateway.rs` + `init/browser_fetcher.rs` + `init/monitor_emitter.rs`）里，待改写 51 处的前 5 个文件吃掉 **43** 处：`init/services.rs` **24**、`bin/axagent-validate-backfill.rs` **7**、`commands/evolution_proposal.rs` **7**、`commands/screenshot_diagnosis.rs` **3**、`market_sim_service.rs` **2**（24+7+7+3+2=43）；其余 8 个待改写文件各 1 处（`tools/src/tools/{finance,industry_constituents,market_mainline,stock_factor}.rs` 各 1、`bin/axagent-batch-rerun.rs` 1、`commands/decision_gates.rs` 1、`commands/_shared_reco_pool.rs` 1、`init/daily_snapshot_store.rs` 1）。**2026-11-06 对账批补记**：上一登记 102 处里的 41 + 4 + 3 = 48 处（`tools/src/tools/opc.rs` + `commands/fleet/mod.rs` + `init/state.rs`）已由 `84d8c357b` 改指 `axagent_company_runtime::` ⇒ 两处读数**总量守恒**（54 + 48 = 102），账目从股票侧整体迁到 OPC 侧（`l1ForeignOpc` 0 → 48 处 / 3 文件）。（B1 首刀前口径为 152 / 28，前 7 文件 106 处，见 §63；B1 首刀后为 134 / 待改写 124，2026-11-04 B2 那刀再降 1 到 133 / 123，见 §64.14；B5 那刀再降 8 到 125 / 待改写 115、文件 26→23，见 §87；B6 那刀再降 14 到 111 / 待改写 107、文件 23→20，见 §88；B7 那刀再降 5 到 106 / 待改写 102、文件 20 不变（`state.rs` 仍有 3 处 OPC 面引用），见 §89；B7b 那刀再降 3 到 102 / 待改写 99 处、文件 19 —— `tools/global_state.rs` 的 AStockClient 槽位退役（3 处）+ gateway 的显式类型注解消失（H 桶 4→3），见 §90。）
⚠ **为什么同一张表里有两个命令数 / 两个前端体量**：两套**归属规则**不同 —— 「插件边界口径」按迁出后归哪个包，「审计 19 域口径」按模块名正则；
差集 11 个模块逐条列在 `AUDIT-l1-blockers-2026-11-01.md` 判读口径第 6 条（例：`demand_discovery` 审计归股票、边界归 OPC；`mover_recall`/`narrative` 反之）。
**两个数都是真的，引用时必须带口径**；本节与 P6.1/P6.2 的判据一律用**边界口径**。
⚠ **②的 29 处是一条硬顺序约束**：`opc_capability_pack_actions.rs`(10)、`opc.rs`(4)、`opc_capability_pack_runtime.rs`(4)、`opc_workflows/mod.rs`(3)、`opc_workflow_kpi_hook.rs`(3)… 全在调 `astock_data` / `analysis_engine` ⇒ **OPC 依赖股票**。
所以顺序只能是 **先股票、后 OPC**（股票先成为插件后，OPC 侧这 29 处才能改成 `call_seam`）；我上一稿说「OPC 更容易先做」只看了 Rust 桶 ①，**是错的**，撤回。
`tools/src/tools/opc.rs` 的 41 处原按桶 ① 记在股票名下（它 import 的是股票 crate），`84d8c357b` 起改指 `axagent_company_runtime::` ⇒ 连同 `commands/fleet/mod.rs` 4 处、`init/state.rs` 3 处一起迁到 **OPC** 名下（合计 48 处 / 3 文件，见上表与 §116），但迁 OPC 时仍必须连带处理这一文件。

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
→ **P6.1 股票迁出**（改写 133 处 + 前端 54 处、304 条命令、6 万行页面）
→ **P6.2 OPC 迁出**（改写 0 处 Rust + 前端 28 处，但要把 §0.10.1 的 **29 处跨包口改成 `call_seam`**、138 条命令、8 859 行页面）
→ **P6.3** 其余功能包按同一模板批量。
**顺序不能反**：OPC 的 29 处引用指向股票 crate ⇒ 股票先成为插件，OPC 才有接缝可调。

判据（每条都可打回，全部机械可检，口径 = `scripts/audit-l1-blockers.mjs`）：

1. **主壳残留文件里 `grep axagent_(astock_data|analysis_engine|company_runtime|quant|market_sim)::` 命中 0。**
 今天该作用域（`src-tauri/src` + `axagent-runtime/crates`，去注释、去 `examples/`、去 `.workbuddy`）**总命中 625 处 / 74 文件**，
 其中 **456 处随域自有文件（`stock_analysis.rs` 209、`stock_workflow/*` 69、`opc.rs` 18…）一起搬走**、**29 处是跨包口**（改 `call_seam`），
 **真正要改写的只有 133 处 / 26 个主壳文件**（含 H 桶 10 处 / 4 文件），且前 5 个文件吃掉 **87** 处（2026-11-01 订正：旧记 103 是列项加错；2026-11-03 B1 首刀再降 18 处，2026-11-04 B2 再降 1，见 §63 与 §64.14）。
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
├─ web/                  # T3：294 文件 / 85 009 行前端源码 → 打包 bundle
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

**判据**：今天这轮自查就抓出 **15 处数字漂移 + 2 套并行口径**（1534/1541、工作流 124/83、股票 304/306、前端 60 757/85 009），**2026-11-01 本批再记三对漂移（旧/新）：命令 1541/1542、`.rs` 文件 2005/2004、`pub trait` 316/315**；**2026-11-02 `#75` 再记一对：命令 1542/1543（新增 `plugin_list_pages`）**——本段是这些旧读数的唯一合法出现处，写在别处由第五道门判红。
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
1. **业务域主页面**（股票 294 文件 / 85 009 行的自绘图表与多面板交互）→ 走「插件 bundle + 宿主通用容器页」，
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
| **B（零安全变更）** | 不开 asset 协议，插件页面继续用现有 `manifest.dashboardPanels[].frontendEntry` **文本资产 + iframe `srcdoc`**（`ArtifactPreview/ChartPreview.tsx:33` 已有同款用法） | **零**：不新增源，CSP 不动 | 中：bundle 必须内联成单文件 HTML | 适合 OPC 的表单类页面；**股票 85 009 行内联不现实**（首屏解析与图表库体积会炸），只能当兜底 |
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
| **D2** | **无流式**：远程 provider 的 `chat_stream` 第一帧即错误 | `worker.rs:2966-2967`（注释自述「帧协议一请求一响应，无流式分片复用」）、实现 `:2997-3010`；`reflector.aggregate_patterns/get_history`、`evolver.*`、`optimizer.*` 返回 `remote_method_not_supported`（`worker.rs:2450`、`:2542-2590`） | **provider 试点的硬前置**：聊天应用不能失去逐 token 输出 | 协议加流式分片（同一 `request_id` 多帧 + `final` 哨兵），是 `PLAN-everything-is-plugin.md` §15.4 那套扩展的自然延伸；**不做则 provider 不许插件化** |
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
| 行情数据源 | `market_data.rs:192` | ✘ 无 | 1 生产（`AStockClient`，`astock-data/src/lib.rs:6641`）+ 2 测试替身 | `astock-data/src/as_of.rs:36` 的 `tokio::task_local! AS_OF` 是**隐式上下文**，跨进程必丢 ⇒ 必须改成显式入参 `asOfDate`；客户端自带 Moka L1 / `DiskCache` L2 / health tracker / token 热更新 ⇒ 缓存与凭证主权要选边 | `market.provider.{源名}` | 中高（签名可原样走，状态主权难） |
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
| **P5.9** | **其余 11 条接缝收尾**（§0.7.4 未进 P5.2–P5.8 的：`search.source` / `image.gen` / `mcp.server` + §0.7.5 的 4 条钩子族），另含已建接缝但卡 D2 的反思/进化/优化器（`worker.rs:2450`、`:2542-2590` 那批 `remote_method_not_supported` 逐条补）、工具集投影层（D4）、平台适配器、业务规则、webhook / message.callback；**以及 §0.7.6 的 `DojoExtension` 第二协议合流（合流或废弃，二选一）** | 23 条逐条对照 §0.5.3 三条判据（宿主侧无具体类型 / 新增同族不改主壳 / 卸载明确失败），**逐条打勾表进 `docs/audits/`**；接缝四方对账门（§7-1）在 11 条新接缝全绿后才允许摘 `--ratchet` |
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
| **P6.1 股票迁出** | 改写 133 处 / 26 文件 + 54 处前端反向 + 304 条命令挂 T2 + 20 表转 `schema.json` + 85 009 行前端出包 + 3 527 key | **45–60** | P6.0 全部（③④⑤⑥ 是关键路径） |
| **P6.2 OPC 迁出** | 0 处主壳改写（**现况已变 48 处 / 3 文件**：`84d8c357b` 起 `tools/opc.rs` 41 + `commands/fleet/mod.rs` 4 + `init/state.rs` 3 指 `axagent_company_runtime::`，见 §0.10.1 与 §116；估工随之上浮，另批核）+ 28 处前端 + 137 条命令 + **29 处跨包口改 `call_seam`** + 28 表 + 12 797 行前端 | **20–28** | P6.1（跨包口指向股票 crate）+ D1 超时先修 |

**合计：P6.0 ≈ 39–59 人日，P6.1 ≈ 45–60，P6.2 ≈ 20–28 ⇒ 全段 ≈ 105–147 人日**（单人 5–7 个月，3 人并行受限于关键路径约 2.5–3.5 个月）。
误差来源先说清：① 这些是**改动点数 × 单点成本**的外推，不是实测 —— 仓内唯一的实测锚点是 §3 的「新增一条接缝 6 处固定成本 ⇒ 13 条约 78–90 处改动点」；
② 前端 85 009 行的出包成本没有先例，估工里 **±30% 的不确定性最大**；
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

| **P6.1** | **股票业务迁出为 L1 插件**（任务 #35）：改写 §0.10.1 桶① 的 133 处 / 26 文件（前 5 个文件占 **87** 处），304 条命令走 T2 通道，54 处前端反向 import 清 0，20 张表改由包内 `schema.json` 声明 | 主壳 `grep` 域 crate = 0；`src-tauri/Cargo.toml` 无股票 crate；卸载股票插件后其余 1 099 条命令可用、`stock_*` 表与数据保留 |
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
- ~~**§0.10.2 T3** 前端装载形态~~ → **裁定：两条都要，按页面复杂度分流**（`PluginPanelHost.tsx:30` 的受限 IPC 桥已存在，扩到 page 粒度即可；股票 85 009 行走 bundle，OPC 表单类走 DynamicPage）。

**真正还开着的（都不是架构问题，我裁不了）：**

1. **是否引入服务端部署形态**（解 2 的产品前提）——与桌面单机定位相关，只影响将来，**不阻塞任何阶段**。
2. ~~CSP / asset 协议的安全评审口径~~ → **已定 A**（§0.12.5）：开 `assetProtocol`、CSP 加 `asset:`、允许根只限插件安装目录。
   仍开着的一小条：现 CSP 里的 `'unsafe-eval'` 何时收 —— 已剥出插件化批次单独排期（与插件装载无关，不该混评）。
3. **P6 的排期与人力**：P6.1 股票（133 处改写 + 304 条命令 + 85 009 行前端）与 P6.2 OPC 谁先投入、投多少人 —— 这是项目计划，不是架构判据能裁的。

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
9. **tool 侧权限档位仍是装饰**：`#30` 只把**命令通道**改成真授权。`PluginTool::check_permission`（`plugins/src/types.rs:892`）与唯一引用它的 `execute_with_permission_check` **至今零调用方** ⇒ 「插件工具能碰什么」这条线仍未执法，是 §2 缺陷表里独立的一条，不是本批已完成项。
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
硬 ZERO 6（⚠ **本行前两个号是 `#34` 删除前的时点号，故意保留不改**：`harness/src/ir_renderer.rs` 已**整文件删除**、`harness/src/kit_bridge.rs` 现 **119 行** ⇒ `:121` 落在 EOF 外，`--dump-doc-refs` 因此把这条报成「越界 1」（属既存 7 条可疑之一，非新坏；本行是 §14.4 判定「可删 2 条」的依据本身，改号就等于抹掉判据））：`IrRenderer`（`harness/src/ir_renderer.rs:18`）· `KitSlashCommandProcessor`（`harness/src/kit_bridge.rs:121`）· `LlmContentGenerator`（`agent/src/research_agent.rs:51`）· `OpcAutomationService`（`company-runtime/src/opc/automation.rs:298`，随 #32 处置）· `SignalService`（`harness/src/device_sync.rs:647`）· `TaskExecutorImpl`（`agent/src/task_executor.rs:92`）；B 级 1：`FeatureFlagProvider`（唯一 dyn 位点是别名定义 `feature_flag_provider.rs:17`，生产实现 0）。
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

## 43. `#80` 交付账：`market.quote` 宿主→插件接缝（2026-11-02；报告 `.tmp/res80-report.md`；提交 4bbbeabbe）

### 43.1 方向必须写清楚（这是最容易混的一条）

本仓有两类接缝，**方向相反、挂载点不同**：

| 类别 | 清单 | 含义 | 本次 |
|---|---|---|---|
| 插件**提供**给宿主 | `worker.rs:138 SUPPORTED_REMOTE_SEAMS`（13 条） | 插件实现、宿主调用；进门面注册 | **未动**（`diff` 284 行全新增、0 删除） |
| 宿主**提供**给插件 | `capability_registry` 的注册项 + `dispatch_seam` 的入站分支 | 宿主实现、插件经 `call_seam` 调用 | **本次新增 `market.quote`** |

⇒ `market.quote` **不进** `SUPPORTED_REMOTE_SEAMS`。把它加进那份清单会翻转语义：那会声称「插件可以提供行情数据给宿主」，而本次要的是相反方向。

### 43.2 落点与 op 形状（下一阶段写插件 SDK 时按这张表）

| 位置 | 内容 |
|---|---|
| `harness/src/capability_registry.rs:102` | `ServiceDefinition::market_quote()`：ID `market.quote` / v1.0 / contract `axagent_harness::MarketDataProvider` |
| 同文件 `:888` | `register_market_data` / `get_market_data`（逐形对标 `register_agent_loop`） |
| `worker.rs:506` | `dispatch_seam` 本体；`:744` 起 `market.quote` 三个分支 |
| `src-tauri/src/init/state.rs:1715` | `let market_data: Arc<dyn MarketDataProvider> = astock_client.clone();` 注册（失败只 warn，不阻断启动） |

| op | args | 返回 |
|---|---|---|
| `get_quote` | `{ "stock_code": string }` | `StockQuote`（字段是 `code`，**不是** `stockCode`——任务书写错过一次，执行者按真实 DTO 断言） |
| `get_klines` | `{ "stock_code": string, "period": string, "limit"?: u32=100, "adj_type"?: "forward"\|"backward"\|null }` | `Vec<KLine>` |
| `search_stock` | `{ "keyword": string }` | `Vec<StockSearchResult>` |

**契约复用而非新造**：`MarketDataProvider` 早在 harness（`market_data.rs:192`），本次只把它接上注册表与入站分派。`astock-data` 的实现（`lib.rs:6667`）零改动 —— 这正是「扩展而非重定义」（禁区 12）的形态。

### 43.3 验证与一条**新的假绿形态**

- `axagent-harness` check 43.95s / `axagent-plugins` check 36.67s / `worker::tests` **46 passed**（基线 42，+4）/ `clippy --all-targets -D warnings` 17.25s —— 全 EXIT=0。
- 主 crate 首次 `cargo check -p axagent` **退 0 但日志里没有 `Checking axagent` 行**（cargo 判 Fresh 只打印 `Compiling axagent`，那是 build script）。执行者没把它当绿，补跑 `--lib` 强制重查，出现真实 `Checking axagent v3.0.0`，10m55s EXIT=0。
⇒ **新假绿形态（并入本 PLAN 的假绿族）**：**`cargo check -p <pkg>` 退出 0 + 只有 build script 的 `Compiling <pkg>`、没有 `Checking <pkg>`** = 目标包可能根本没被检查。判据：日志里必须出现 `Checking <目标包>`；否则补 `--lib` / `touch` 主源文件强制重查。⚠ 与既有「只吐 Finished」「退 0 + 0 字节日志」是同族，但**日志非空、有 Compiling、却仍不代表目标包被检查**是本族第一次被点名。

### 43.4 未做与本 PLAN 的记账口径

1. **trait 方法集未扩**：行情/K线/搜索 3 条只是 `MarketDataProvider` 的现有面；调研实测股票域实际用到 **16 类操作**（基本面、指数、涨停/热股/行业/龙虎榜/北向、概念板块、研报/一致预期、公告/解禁/分红/财报日历、资讯与情绪、同业/期权PCR/行业归属、跨市场、批量、MCP 工具直通、数据源运维、凭证注入）⇒ 第二阶段扩 trait + DTO。
2. **插件侧 SDK 封装未做**：插件今天要手写 `call_seam("market.quote", op, args)`；是否在 `plugin-proto` / SDK 提供类型化助手，等第二阶段定。
3. **记账口径：能力面保持 23 / totalRows 37 不变**。理由：§0.7 登记表统计的是「L2 提供方接缝」（插件 → 宿主）。`market.quote` 是宿主 → 插件的**查询面**，与 `event.dispatch`（订阅面）同族，不在这 23 条里。⇒ 第五道门判定档未因本次改动变红；**不新增登记行**，但本节的 op 表就是它的权威文档。

## 44. `#81` + `#82` 交付账与 `market.quote` 扩面终局裁定（2026-11-02）

### 44.1 `#81`：行情 DTO 上收 harness（提交 `e45649bd3`）

| 项 | 事实 |
|---|---|
| 新文件 | `axagent-core/crates/harness/src/stock_market_types.rs`（740 行） |
| 迁入 | **40 个 `pub struct` + 1 个 `pub enum`（`KLinePeriod`）** + 2 个 inherent impl（`KLinePeriod::to_em_code`、`LimitUpPool::is_for_requested_date`） |
| 原文件 | `axagent-quant/crates/astock-data/src/types.rs` **895 → 192 行**，变 `pub use axagent_harness::stock_market_types::*;` 垫片；`estimated_financial_report` 留原处（依赖 `crate::as_of`） |
| 导出 | 仅在 **模块路径** 提供，harness 根**不做** glob —— 根已有 `dashboard_report::IndexQuote`（字段 name/price/change_pct），与迁入的 `IndexQuote`（code/name/price/pre_close/...）同名异义 |
| 逐字保真 | 三段区间 sha1 与原文件一致；零语义变化 |
| 验证 | harness check/clippy、astock-data check、analysis-engine check、双包 clippy 全 EXIT=0（39s / 57s / 13s / 31s / 40s），日志均含 `Checking <目标包>`；类型身份用例 `test_dto_type_identity_with_harness` 实跑 1 passed |
| 未验证 | `src-tauri/**`（34 个文件引用 astock-data）、`tools` / `rt-workflow` 未实编 —— 由 #83 的窗口顺带覆盖 |

**口径纠正（执行者推翻我）**：我派单写「41 struct + 2 enum」，盘上实为 **40 struct + 1 enum**；`AdjType` 原本就已在 harness `market_data` 并被 astock-data re-export，不是本文件定义。

### 44.2 `#82`：op 映射全表（报告 `.tmp/res82-op-map.md`，317 行）

**计数闭合**：并集 **54** = 数据面 **42** + 运维/凭证面 **12** + 不可走接缝 **0**。数据面 42 行合并两组「薄壳/委托」后 = **40 个 op**：`get_klines_with_adj` 并入既有 `get_klines`（trait impl 内部已委托）、`get_index_quotes` 并入 `get_index_quotes_outcome`（`into_data()` 薄壳）。扣除已上线 3 个 ⇒ **需新增 37 个数据 op**。

**最大的语义缺口（本批最重要的发现）**：42 个数据方法**全部**读进程内 `AS_OF`（`task_local` + 进程级 `GLOBAL_AS_OF` 双存）。插件进程读不到宿主 task_local；宿主代执行时若不在 `with_optional_asof` 作用域内，会经全局栈**读到别的请求残留的 as-of（串味）**。⇒ 不是「退化成 live」这么简单，而是**同一请求的语义取决于宿主并发时序**。

**三类专项判定**：
- 凭证 4 字段（`neodata_token` / `xq_token` / `iwencai_key` / `health_tracker`）是 **pub 字段**不是方法，**永不跨进程**；
- 运维面 8 个方法（`check_vendor_health` / `set_enabled_vendors` / `set_eastmoney_proxy` / `interface_health_history` / `has_daily_snapshot` / `daily_snapshot_enabled` / `set_daily_snapshot` / `set_stock_daily_snapshot`）**不混入 `market.quote`**；
- `get_index_quotes_outcome` 返回非 `Result` 的 `DataOutcome`（在 **astock-data** `outcome.rs`），`market_list_page` 返回 tuple ⇒ 推迟到第二小批（先上收 `DataOutcome` / 定义命名对象返回）。

**执行者对我种子数据的纠错（同批采信）**：① 主壳直调名单是 35 项（31 方法 + 4 字段），不是 33；② 4 个「方法」实为字段；③ 我只统计字面量 `astock_client.`，漏掉 `let client = ...clone()` 后的 `client.` 与跨行链式（实测主壳 `get_quote` 16 次而非 5、`get_klines` 10 次而非 2）；④ 漏方法 11 个（含 `get_klines_with_adj`、`get_margin_data`、`get_pledge_data`、`get_sector_info`、`get_news` 等）；⑤ 「16 类」括号里实际只有 14 个标签；⑥ **op/args 线上是 snake_case**（我派单写成 camelCase 会破坏 #80 已交付协议）；⑦ PLAN §13.2 只有 89 处的直方图，不构成逐类清单。

### 44.3 六项裁定（leader 定，不再留选择题）

1. **命名**：op 名与 args 键**沿用 snake_case**（`stock_code` / `period` / `limit` / `adj_type` / `keyword`），DTO 字段保持 camelCase。改 camelCase 属破坏性，不做。
2. **as-of**：新增数据 op 一律带 `as_of?: string | object` —— 字符串按 `YYYY-MM-DD` 解析（`AsOfSource::UserReplay` + `AsOfDataScope::All`），对象按完整 `AsOfContext` 反序列化（保住 `data_scope` / `source`）。宿主分派用 `with_optional_asof` 包住本次调用；**禁止**依赖全局栈兜底。trait 侧参数类型 `Option<AsOfContext>`。
3. **`get_index_quotes`**：不单列，只保留 outcome 形态；等 `DataOutcome` 上收 harness 后随第二小批交付。
4. **`market_list_page`**：**新 op 从一开始就返回命名对象** `{"items":[{"code","name","industry"}],"total":n}`，不照抄内部 tuple（无既有插件客户端，零兼容成本）。
5. **运维/凭证面**：另立接缝（暂名 `market.ops`，只读诊断 + 显式写操作分开），凭证 4 字段**永不暴露**；本批不做。
6. **MCP 直通**：单独立项 —— 先数 `stock_mcp_tools()` 工具数与 `execute_mcp_tool` 分支，再决定「逐工具」还是「单条 `mcp_tool_call{tool_name,arguments}`」。

### 44.4 `#83` 分批（本段执行中）

| 批次 | 范围 | 文件 |
|---|---|---|
| **A** | harness `MarketDataProvider` 扩 **12 个引擎强需求方法**（默认「不支持」实现）+ `AStockClient` 逐条委托（`with_optional_asof` 包裹） | `harness/src/market_data.rs`、`astock-data/src/lib.rs` |
| **B** | `worker.rs` 12 个入站分支 + `as_of` 双形态解析 + 用例 | `plugins/src/worker.rs` |
| **第二小批（未开）** | 余 **25 个数据 op**（含 `get_index_quotes_outcome`、`market_list_page`）+ `DataOutcome` 上收 | harness / astock-data / worker |
| **另立（未开）** | `market.ops` 运维接缝、MCP 直通、插件侧 SDK 类型化助手 | 待定 |

12 个 A 批方法：`get_financials` · `get_money_flow` · `get_north_bound_holding` · `get_dragon_tiger` · `get_margin_balance_trend` · `get_margin_data` · `get_holder_count` · `get_block_trades` · `get_pledge_data` · `get_sector_info` · `get_news` · `market_snapshot_batch`。

**既有 3 个 op 的 as-of 债（登记，不在本批改）**：`get_quote` / `get_klines` / `search_stock` 的 trait 签名不带 `as_of`；改它们会牵动 analysis-engine 中大量 `dyn MarketDataProvider` 调用点。等插件侧 SDK 适配器落地时一并收口（届时 analysis-engine 在插件进程内通过 SDK 实现同一 trait）。本批新增方法**全部带 as_of**，不扩大该债。

## 45. `#83` A/B 批交付账：`market.quote` 首批 12 个个股数据 op（2026-11-02；报告 `.tmp/res83a-report.md` / `.tmp/res83b-report.md`）

### 45.1 两批落点与提交

| 批 | 提交 | 文件 | 内容 |
|---|---|---|---|
| A | `0e55104b8` | `harness/src/market_data.rs`（+169/−1）、`astock-data/src/lib.rs`（+196） | trait 追加 **12 个方法**（带默认「不支持」实现，末参 `as_of: Option<AsOfContext>`）；`AStockClient` 逐条用 `with_optional_asof` 包裹固有方法（全限定调用防同名递归） |
| B | `f38e295ac` | `plugins/src/worker.rs`（+540/−2） | `dispatch_seam` 的 `market.quote` 追加 **12 个分支** + `deserialize_as_of_arg` + 7 条新用例 |

12 个 op：`get_financials` · `get_money_flow` · `get_north_bound_holding` · `get_dragon_tiger` · `get_margin_balance_trend` · `get_margin_data` · `get_holder_count` · `get_block_trades` · `get_pledge_data` · `get_sector_info` · `get_news` · `market_snapshot_batch`。args 键沿用 **snake_case**；`days` / `limit` 必填（不设缺省）。

### 45.2 `as_of` 双形态（跨进程显式透传）

| 输入 | 行为 |
|---|---|
| 键缺失 / `null` | `None`（实盘） |
| `"YYYY-MM-DD"` | `AsOfContext::parse`（拒未来日期） |
| 对象 | 反序列化 `AsOfContext`，再用 `AsOfContext::new(date, source)` **补回未来日期校验**，最后 `.with_data_scope(scope)` 保留范围 |
| 其它类型 | `SEAM_CALL_INVALID_ARGS`，说清只接受字符串或对象 |

**未来日期补校验是 leader 补的**（B 批执行者按「对象走 `from_value`」实现，绕过了 `AsOfContext::new` 的契约校验）：新增用例 `market_quote_object_as_of_rejects_future_date`，断言 `2099-01-01` 对象被 `SEAM_CALL_INVALID_ARGS` 拒绝且**不触达 provider**。补丁后 `cargo clippy -p axagent-plugins --all-targets -D warnings` EXIT=0，`worker::tests` **53 passed**（46 基线 + 6 新增 + 1 leader 补例）。

### 45.3 验证

| 命令 | EXIT | 结果 |
|---|---|---|
| `cargo check -p axagent-harness` / `clippy --all-targets` | 0 / 0 | `Checking axagent-harness` 命中 |
| `cargo check -p axagent-astock-data` / `-p axagent-analysis-engine` | 0 / 0 | 均命中 `Checking`；analysis-engine 110s |
| `clippy -p axagent-astock-data -p axagent-analysis-engine --all-targets` | 0 | 42s |
| `cargo check -p axagent-plugins` / `clippy --all-targets` | 0 / 0 | 3s / 5s |
| `cargo test -p axagent-plugins --lib worker::tests` | 0 | **53 passed**；`cargo test` 用 `Compiling` 而非 `Checking` 证明真实重编 |

A 批首轮 clippy 曾 EXIT=101（默认实现参数未使用，`-D warnings` 升 error）⇒ 参数改 `_` 前缀后全绿；B 批首轮 clippy 曾报 2 处 `clone_on_copy` ⇒ 改 `(*guard)` 后全绿。两次都不是签名的语义变化。

### 45.4 B 批提出的六项，leader 裁定

1. **对象形态未来日期**：已修（见 45.2）。
2. **`to_value` 的 DTO 名风格不统一**（元素名 vs `Vec<X>`）：只出现在「序列化失败」的不可达文案里，**不统一**，避免无收益 churn。
3. **`limit` / `days` 必填**：**保持必填**。缺省值会把调用方漏参静默成「默认窗口」，与「缺失不得兜底」的口径冲突。
4. **新 op 的接缝预算**：暂落 `DEFAULT_SEAM_TIMEOUT_MS = 15s`；实跑股票插件后再按实测补 `SEAM_OP_TIMEOUT_OVERRIDES`（不在本批猜）。
5. **`codes` 无上限/去重**：属接缝契约层，随 P6.1 迁移批量裁定。
6. **`get_news` 在 `data_scope=structured` 下是否截断**：由实现方（`AStockClient`）决定；分派层只透传，不二次判断。

### 45.5 重锚与遗留

- `worker.rs` 的 evidence 整段后移 **+296 行**（`remote_method_not_supported` 2450→2746、evolver 区间 2542/2590→2838/2886）；`harness/src/market_data.rs` 的 `pub trait MarketDataProvider` **192→197**（A 批在 trait 前加了 import/文档行）。已 `--repin-code --apply` 并同批改正文与 PLAN:265/266/276。
- **未验证**：主 crate（`src-tauri`）未实编。本批只改 harness/astock-data/plugins，`MarketDataProvider` 是既有 trait 的增量默认方法，主壳不引用新方法；风险面在「trait 对象体积/编译」，留给下一次主 crate 窗口。
- **剩余 25 个数据 op**（含 `get_index_quotes_outcome` 需先上收 `DataOutcome`、`market_list_page` 改命名对象返回）、`market.ops` 运维接缝、插件侧 SDK 类型化助手 —— 均未开工。

## 46. `#84` C/D 批交付账：`market.quote` 补齐到 40 个数据 op（2026-11-02；报告 `.tmp/res83c-report.md` / `.tmp/res83d-report.md`）

### 46.1 落点与提交

| 批 | 提交 | 文件 | 内容 |
|---|---|---|---|
| C | `dd96731d6` | 新建 `harness/src/data_outcome.rs`（+111）；`harness/src/lib.rs`（+2）；`harness/src/stock_market_types.rs`（+22）；`harness/src/market_data.rs`（+279/−2）；`astock-data/src/lib.rs`（+417）；`astock-data/src/outcome.rs`（+3/−82） | `DataStatus` / `DataOutcome<T>` 上收 harness；`MarketListItem` / `MarketListPage` 新增；trait 追加 **25 个方法**；`AStockClient` 全部 `with_optional_asof` 委托 |
| D | `d8a270508` | `plugins/src/worker.rs`（+798/−1） | `dispatch_seam` 追加 **25 个分支** + 6 条用例（含无参 op / 可选参数 / Outcome 信封 / 命名对象返回） |

**合计：`market.quote` 数据面 = 40 个 op**（3 原有 + A/B 12 + C/D 25）；trait 内 `async fn` 41 个（40 数据方法 + `subscribe`），复核时勿把 41 当成「多了一个 op」。

### 46.2 特殊形态

| 形态 | 处理 |
|---|---|
| `get_index_quotes_outcome`（固有非 `Result`） | trait 包成 `Result<DataOutcome<Vec<IndexQuote>>>`（`Ok(固有值)`）；信封的 `status` / `detail` 原样过帧（用例锁 `schema_changed` + `detail`） |
| `market_list_page`（固有 tuple） | trait 返回 `MarketListPage { items: [{code,name,industry}], total }`，保持原顺序与 `total`；D 批用例锁字段名与条数 |
| 无参 op（6 个） | 不读任何业务键，**仍**调用 `deserialize_as_of_arg`；`args: {}` 合法 |
| `requested_date` / `adj_type` 可选 | `deserialize_optional_arg`：键缺失 / `null` ⇒ `None`；不把缺键折成空串 / 不复权以外的值 |
| `DataOutcome` 路径 | `astock_data::outcome::DataOutcome` 是 `pub use axagent_harness::data_outcome::…` 的同类型再导出（非别名、非副本）；`outcome` 模块 4 条既有自测实跑通过 |

### 46.3 验证

- C：harness check 33s / clippy 44s；astock-data check 43s / analysis-engine check 105s / 双包 clippy 41s；`cargo test -p axagent-astock-data --lib outcome` **4 passed**。全 EXIT=0 且出现 `Checking <目标包>`。
- D：`cargo check -p axagent-plugins` 33s / clippy 6s / `cargo test --lib worker::tests` 49s ⇒ **59 passed**（53 基线 + 6 新增）。`cargo test` 用 `Compiling` 证明真实重编。
- **未验证**：主 crate（`src-tauri`）仍未实编 —— 本段四批都只动 harness / astock-data / plugins，主壳不引用新方法；下一次主 crate 窗口顺带覆盖。

### 46.4 D 批提出的三项，leader 裁定

1. **剩余 19 个分支不补逐 op 运行时用例**：类型层（`check` + `clippy --all-targets`）已保证签名与序列化可编译；6 条代表性用例锁住五类形态（无参 / 可选 / 信封 / 命名对象 / 非法入参）。逐 op 断言 19 条属重复覆盖，等真实插件 e2e 一次验完更值。
2. **不修 `worker.rs` 既有 6 处 rustfmt 漂移**：是本批之前的存量债，修它会混进本批 diff；留到下一次「rustfmt 单独批」。
3. **`clippy --all-features` 暂不补**：`axagent-plugins` 当前默认 feature 即 CI 主路径；全量 `--all-features` 属「所有阶段完成后」的收口命令，届时一并跑。

### 46.5 记账与重锚

- K1 扫描面 **2010 → 2011**（`DataOutcome` 上收新增 `harness/src/data_outcome.rs`；`astock-data/src/outcome.rs` 保留为垫片）。
- `harness/src/market_data.rs` 的 `pub trait MarketDataProvider` **197 → 201**（C 批 import 扩列）。
- `worker.rs` 的 evidence 再后移 **+545 行**：`remote_method_not_supported` 2746→**3291**，evolver 区间 2838/2886→**3383/3431**。已 `--repin-code --apply` 并同批改正文与 PLAN:265/266/276。

### 46.6 仍未做

- 既有 3 个 op（`get_quote` / `get_klines` / `search_stock`）的 as-of 债未收口（签名为兼容 analysis-engine 保持不带 `as_of`）。
- `market.ops` 运维/凭证接缝、MCP 直通、插件侧 SDK 类型化助手、剩余查询接缝（`jobs.store` 等，调研中）均未开工。

## 47. `#85` 其余查询面调研与五项裁定（2026-11-02；报告 `.tmp/res85-seams.md`，488 行）

### 47.1 计数与结论（以报告原文为准）

- 8 个字段全部存在；种子 22/10/8/6/4/3/3/2 可逐项复现（同日 `state.<field>` 口径，合计 58）。
- **真实访问点 ≠ 同行出现**：24 文件内 **66**（+7 多行链、+1 `s.stock_monitor` 别名）；业务域补扫再加 **12**（`pool_scan.rs` cron +4、`backtest_validation.rs` cron +4、`time_travel_batch.rs` work/vector/traj 各 +1、`workflow_template.rs` monitor +1）⇒ **同口径 68 / 真实访问点 78**。
- **K4 直接成立 3 条**：`jobs.store`（`update` 的闭包改 patch DTO 后）、`knowledge.vector`、`trajectory.store`；`trading.engine` / `stock.monitor` / `quote.watcher` / `stock.aggregator` 改造后可成立但本体留宿主；**`work_engine` 直接暴露不成立**（`RunOptions` 含 7 类闭包/句柄）。
- **最大风险不是字段，是执行权**：`CronExecutor` 在宿主按 `task_type` 执行且启动时播种 11+ 条内置股票任务；`WorkEngine` 是宿主唯一实例。只建 CRUD 接缝 = 半条链。

### 47.2 五项裁定

1. **cron 执行主权：宿主保留执行，插件只拿 CRUD。** 理由：把 `stock-*` task_type 路由到 worker 需要重写 `CronExecutor` 的 11+ 个 handler（它们捕获 `state`/`db`/`work_engine`/`trading_engine`），且 cron 触发时插件不一定已启动。内置播种任务（`init/stock_automation_cron_seed.rs` 的固定 id）**归宿主所有**：`jobs.store.remove` 对保留 id 一律拒绝；`set_status` 允许（用户可暂停）。插件新建任务走同一 `jobs.store`，宿主继续执行。**双向路由留到插件稳定后另立**，不在 P6.1 首批。
2. **workflow 执行主权：批准 `workflow.run` 异步 job 形态**（`start` 立即返回 `execution_id`；`status` / `cancel` / `list_active`；进度走 `event.dispatch`）。宿主独占 `WorkEngine`；插件只持 `execution_id`。**`RunOptions` 永不进 harness**。实现排在 `jobs.store` 之后。
3. **写面权限：必须 op 级校验，不许「声明 write 就能调全部写 op」。** 每条 seam 的每个 op 标注 `read` / `write`；`dispatch_seam` 在调 provider 前按插件 `permissions` 校验，写 op 要求 `write`。**在该校验落地前，不发布任何写 op**（jobs 首批先只上 read + 受校验的写）。
4. **DTO 落点**：新建 `harness/src/stock_seam_types.rs` 集中放跨进程视图（`CronJobView` / `TradeView` / `PositionView` / `MonitorConfigView` / `AggregatorConfigView` 等）；`Trajectory` / `QueryTrajectory` / `Workflow` / `VectorSearchResult` 已有 harness 定义，**直接复用**；ORM `Model` 与 `RunOptions` 一律不过线。
5. **`as_of` 约定**：凡会触发行情取数的 op（`trading.get_positions` / `validate_trade` / `execute_trade`）显式收 `as_of`，宿主包 `with_optional_asof`；纯 DB/内存 op（jobs / trajectory / vector 索引）不收。与 `market.quote` 口径一致。

### 47.3 实施顺序（按阻塞 #35 的强度）

| 顺序 | seam | 理由 | 依赖 |
|---|---|---|---|
| 1 | `jobs.store` | 股票自动化入口几乎全是 cron；纯数据最易过线 | 无（contract + impl + dispatch） |
| 2 | `workflow.run` | 分析/反思/Serenity/管道都跑在 WorkEngine；没有它股票业务只是壳 | 需要异步 job + 事件 |
| 3 | `trading.engine` | 交易面板/组合风险/仓位限制；读面大、写面最高风险 | 需 `as_of` + op 级权限 |
| 4 | `stock.monitor`（并入 aggregator） | 告警闭环；必须 DB+内存复合写 | 需复合 op |
| 5 | `quote.watcher` | 仅管理推送集合；缺了宿主仍推送 | 需补 serde |
| 6 | `knowledge.vector` | 只影响决策/反思入 RAG | 只传文本，宿主管 embedding |
| 7 | `trajectory.store` | 成本最低、DTO 现成，可随手带上 | 无 |

### 47.4 报告自报的未决/未做（登记）

- 24 文件清单是从 8 个总数反推的，不能证明「恰好这 24 个」；其余候选文件命中为 0，不影响总数。
- `PoolScan` / `BacktestValidation` / `TimeTravel` 是否属本次迁移范围无明文；本 PLAN 按**业务域**纳入。
- `trading.get_positions` 内部 N+1 次取数、可能超 15s 默认预算：实测前不设 override。
- 报告未审计 `stock_analysis_setup/**` 的 DTO 前端消费与 `market.quote` 40 op 的完整实现（只抽样）。

## 48. `#86` 交付账：`jobs.store` 只读接缝（2026-11-02；报告 `.tmp/res86-report.md` / `.tmp/res86b-report.md`）

### 48.1 落点与提交

| 批 | 提交 | 文件 | 内容 |
|---|---|---|---|
| E1a | `f6ed72853` | 新建 `harness/src/jobs_store.rs`（+94）；`harness/src/lib.rs`（+3）；`harness/src/capability_registry.rs`（+33）；`runtime-core/src/cron_job.rs`（+93）；`src-tauri/src/init/state.rs`（+12） | `CronJobView` / `CronJobRunView` + `JobStoreProvider`（3 方法默认「不支持」）；`CronJobStore` 实现；`jobs.store` 注册/取回；宿主注册（失败只 warn） |
| E1b | `30765577d` | `plugins/src/worker.rs`（+238） | 三个**只读**分支 `list` / `get` / `count` + 5 条用例 |

### 48.2 契约形状（线上 snake_case）

| op | args | 返回 |
|---|---|---|
| `list` | `task_type?`(string)、`active_only?`(bool，缺省 false) | `Vec<CronJobView>` |
| `get` | `id`(必填) | `CronJobView \| null`（**查无即 `null`，不是错误**） |
| `count` | `task_type?` | `{ "count": n }`（具名对象） |

`CronJobView` 刻意**不暴露**：`config` / `delivery` / `epoch_cost_estimate` / `params` / `last_result.output` / 进程内 `running` 登记。`last_result` 保留 `success/skipped/partial/error/durationMs/executedAt`（`partial` 由 leader 本批补入，避免 DTO 二次改）。`count_jobs` 不提供 `active_only`（插件可用 `list(task_type, true)`）。

### 48.3 验证与状态

- harness check 101s / clippy 45s；runtime-core check 87s / clippy 53s；全 EXIT=0 且出现 `Checking <目标包>`。
- leader 补 `partial` 后复跑 harness / runtime-core check：EXIT=0（各命中 1 次 `Checking`）。
- plugins 三条命令（执行者先 `cargo clean -p axagent-plugins` 清 2.1 GiB 旧产物）：check 3s / clippy 5s / test ⇒ **64 passed**（59 基线 + 5 新增）。
- **未验证**：`src-tauri` 主 crate 仍未实编；`init/state.rs` 的注册块按「`Arc<CronJobStore>` → `Arc<dyn JobStoreProvider>` 标准 unsized coercion + clone 后按值 move」静态核对，留给下一次主 crate 窗口。

### 48.4 记账

- K1：`rsFiles 2011→2012`（新增 `jobs_store.rs`）、`pubTraits 317→318`（`JobStoreProvider`）、`declaredExternalTraits 233→234`（runtime-core 有生产 impl）。已随提交 `6083a1fc1` 同步镜像表与 PLAN:247/248。
- `worker.rs` evidence 再后移 **+62 行**：`remote_method_not_supported` 3291→**3353**，evolver 区间 3383/3431→**3445/3493**。

### 48.5 本批未做（写面按裁定暂缓）

- 未发布 `add` / `remove` / `set_status` / `update` 任何写 op。
- op 级权限校验未落地：`dispatch_seam` 目前**完全没有权限面**，`PluginWorkerConfig` 也不携带 `PluginPermission`。
- 写 op 及权限门作为下一批（E2）：先做 `PluginWorkerConfig.permissions` + `dispatch_seam` 的 op→permission 表 + `write` 门，再上 `jobs.store` 写面（含保留 id 防误删、`update` 用 patch DTO 不转发闭包）。

## 49. `#87`/`#88` 交付账：op 级写权限闸 + `jobs.store` 写面（2026-11-02；报告 `.tmp/res87-report.md` / `.tmp/res88-report.md` / `.tmp/res89-report.md`）

### 49.1 落点与提交

| 批 | 提交 | 文件与增删 | 内容 |
|---|---|---|---|
| #87 权限闸 | `ac89e1606` | `plugin-proto/src/lib.rs` +5；`plugins/src/worker.rs` +190；`plugins/src/manager.rs` +3 | `PLUGIN_PERMISSION_DENIED` 内部诊断码；`PluginWorkerConfig.permissions` / `with_permissions`；`HostPeer.permissions` + `has_permission`；纯函数 `required_permission_for(seam, op)`；`dispatch_seam` **match 之前**的统一准入闸；`start_plugin_worker` 从 manifest 注入 |
| #88 E2b1 契约+宿主 | `25fbcf9df` | `harness/src/jobs_store.rs` +87/−5；`harness/src/lib.rs` +2/−2；`runtime-core/src/cron_job.rs` +193/−2；`src-tauri/src/init/state.rs` +26/−1（合计 +308/−10） | `NewCronJob`、`RemoveJobOutcome`；`JobStoreProvider` 增 `add_job` / `remove_job` / `set_job_status`（trait 默认「未实现」）；`CronJobStore` 三方法实现 + 14 个种子 id 保护；`state.rs` 注册块内 `protect_ids(...).await` |
| #88 E2b2 分派 | `2916a16fb` | `plugins/src/worker.rs` +282/−12 | `add` / `remove` / `set_status` 三个写分支；`FixedJobStoreProvider` 扩展；5 条新用例；旧「未知 op」用例改锚 `update` |

### 49.2 op 级权限闸（#87）

| seam | op | 需要权限 |
|---|---|---|
| `jobs.store` | `add` / `remove` / `set_status` / `update` | `write` |
| `jobs.store` | `list` / `get` / `count` | 无 |
| 其它任何接缝（含 `market.quote` 全部 40 op） | 任意 | 无 |

- 闸在 `dispatch_seam` 第一段、任何 `match seam` arm / `registry.get_*()` / 参数反序列化之前；命中即回 `PLUGIN_PERMISSION_DENIED`，**代码路径上不可能触达 provider**（用例用空注册表验证：仍回权限码而非 `SEAM_PROVIDER_UNAVAILABLE`）。
- **精确匹配**，不套 `PluginPermission::implies_*`：声明 `execute` / `subprocess_execution` 不会自动获得 `write`。口径与 #30「必须声明对应权限」一致；若产品口径改为「宽权限可顶」，只需改 `HostPeer::has_permission` 并补用例。
- 权限是**连接级不可变快照**：spawn 时从 `WorkerInner.permissions` 克隆给 `HostPeer`；清单变更需重新启用插件生效（运行期不协商）。

### 49.3 写面契约（#88，线上 snake_case 的 op args）

| op | args | 成功返回 | 关键失败 |
|---|---|---|---|
| `add` | `job`（`NewCronJob` 对象） | `{"id": "<新 id>"}` | 缺 `job` / 类型不符 ⇒ `SEAM_CALL_INVALID_ARGS`；id 为空 / schedule 为空 / id 已存在 / 非法 priority ⇒ `PLUGIN_CALL_FAILED`（宿主校验，原任务不被覆盖） |
| `remove` | `id` | **裸字符串** `"removed"` / `"not_found"` / `"protected"` | 缺 `id` ⇒ `SEAM_CALL_INVALID_ARGS` |
| `set_status` | `id`、`status` | `{"found": bool}`（`false` = 不存在，不是错误） | 非 `active` / `paused` / `disabled` ⇒ `SEAM_CALL_INVALID_ARGS`，且**先验字面量再取 provider**，非法值零触达 |

- `NewCronJob` 缺省：`recurring=true`、`priority="medium"` 由 harness DTO 的 serde 提供，分派层**不另填一份**；`id/name/description/schedule/prompt` 必填。宿主 `add_job` 另校验 `priority ∈ {low, medium, high, batch}`（`cron_job.rs:1008`），非法值拒绝且不落库。
- **入参键名口径**：op 级参数（`task_type` / `active_only` / `as_of`）是 snake_case；`job` 内层是 DTO，按 AGENTS 规则 13 线上为 **camelCase**（`workflowId` / `taskType` / `enabledToolsets`）。写成蛇形**不会报错**，会被 serde 静默丢弃为 `None` —— 本 PLAN 以此为准；若将来插件 SDK 文档需要蛇形，只能在 harness 加 `serde(alias)`，不得在分派层做键名映射。
- **受保护 id 语义**：14 个内置种子 id 只对 `remove_job` 生效（返回 `protected`，不删除）；`set_status` **允许**（用户可暂停/恢复）；`seed_stock_automation_crons(..., overwrite=true)` 的宿主内部重置路径不受保护限制（保护只作用于接缝）。名单是进程内 `HashSet`，依赖 `init/state.rs` 每次启动登记；当前全仓只有一处生产构造（`state.rs:579`）。

### 49.4 验证

| 批 | 命令 | EXIT | 证据 |
|---|---|---|---|
| #87 | plugins check / clippy / `cargo test --lib worker::tests` | 0 / 0 / 0 | check、clippy 各命中 1 次 `Checking axagent-plugins`；`67 passed` |
| #88 E2b1 | harness check+clippy；runtime-core check+clippy；`cargo test -p axagent-runtime-core job_store_write` | 全 0 | 四个包均命中 `Checking`；`4 passed; 272 filtered out` |
| #88 E2b2 | plugins check 59s / clippy 7s / test | 0 / 0 / 0 | `.tmp/res89-check.log` 命中 `Checking axagent-plugins`；`72 passed; 113 filtered out`（67 基线 + 5） |

### 49.5 记账

- **K1 不变**：`2012 / 318 / 234`（本批无新增 harness 文件、无新增 pub trait —— `JobStoreProvider` 与 `NewCronJob` 已在 #86 计入）。
- `worker.rs` evidence 因写分支前移 **+136 行**：`remote_method_not_supported` 3353→**3489**；evolver 区间 3445/3493→**3581/3629**。已 `--repin-code --apply` 并人工同步 JSON note / planAnchorText 与 PLAN:265/266（repin 模式不改正文）。

### 49.6 本批未做与已登记风险

1. **`update` 未实现**：仍只登记 `write` 权限；有权限时落 `SEAM_CALL_UNSUPPORTED`。需先裁定 patch DTO 的「null = 清空 vs 缺省 = 不改」语义。
2. **`add_job` 查重→写非原子**：并发同名 add 理论可同时通过 `get()` 检查。契约未要求严格唯一；要严格需在 `CronJobStore` 内加写锁包住「查 + 加」。
3. **受保护 id 可被「先删后建」绕过**：种子任务被删后，同 id 新建会被接受；下次宿主播种因「已存在即跳过」不再覆盖它。该路径是否允许待裁定。
4. **主 crate 已实编（2026-11-02 19:53）**：`cargo check -p axagent` **EXIT=0**，16m13s，日志 `.tmp/res91-maincheck.log`（4535 B）命中 `Checking axagent v3.0.0` ⇒ `state.rs` 三处改动（market.quote 注册、jobs.store 注册、`protect_ids(...).await`）类型检查通过。注：该次编译包含工作树中他人未提交的 stock-analysis 改动；若有错需按归属分流。
5. **`worker.rs` 既有 rustfmt 漂移 9 处**（1691 / 2068 / 6658 / 6680 / 6711 / 6898 / 7274 / 7303 / 7317），非本批引入，本批新增代码 0 差异；未修（属其它批次改动面）。
6. 未跑全量 workspace 门禁；未推送。

## 50. `workflow.run` 接缝交付账（2026-11-02；报告 `.tmp/res90-workflow-design.md` / `.tmp/res93-report.md` / `.tmp/res94-report.md` / `.tmp/res95-report.md`）

### 50.1 落点与提交

| 批 | 提交 | 文件与增删 | 内容 |
|---|---|---|---|
| E3a 契约 | `6a79ecea9` | 新建 `harness/src/workflow_run.rs`（332 行）；`lib.rs` +6；`capability_registry.rs` +33 | 6 个 camelCase DTO + `WorkflowRunProvider`（4 方法全默认「未实现」）；`workflow_run()` / `register_workflow_run` / `get_workflow_run` |
| E3b 宿主 | `35db96406` | `rt-workflow/engine/mod.rs` +58；新建 `src-tauri/src/init/workflow_run.rs`（813 行）；`init/mod.rs` +1；`init/state.rs` +30 | 引擎 `get_execution_output`；`WorkflowRunService`（受理/spawn/owner/准入/总超时/进度桥）；`state.rs` 注册 |
| E3c 分派 | `b62d5ddcf` | `plugins/src/worker.rs` +574/−2；`plugin-proto/src/lib.rs` +2 | 四 op 分派；`start`/`cancel` 需 `write`；新码 `WORKFLOW_RUN_REJECTED`；7 条用例（72→79） |

决策依据：`.tmp/res90-workflow-design.md`（397 行，只读调研）。**真实执行点 5 处**（不是 9 处）：`stock_workflow/core.rs:1036`、`core.rs:2269`、`stock_pipeline/core.rs:353`、`reflection.rs:1112`、`serenity.rs:1288`；其余 `state.work_engine` 访问是透传 / `trigger_manager` / `cancel_workflow`。

### 50.2 契约形状（op args snake_case；DTO 响应 camelCase）

| op | args | 成功返回 |
|---|---|---|
| `start` | `template_id`(必填)、`input`(JSON)、`variables`(数组)、`model_id` / `provider_id`、`max_concurrent`(u32)、`step_timeout_secs` / `tool_timeout_secs` / `total_timeout_secs`(u64)、`dry_run`(bool)、`as_of_date`("YYYY-MM-DD") | `{"executionId","workflowId","acceptedAt"}` |
| `status` | `execution_id`(必填)、`include_output`(bool，缺省 **false**) | `ExecutionView \| null`（`executionId/workflowId/status/currentNodeId/totalTimeMs/createdAt/updatedAt`，`output` 仅 `include_output=true`） |
| `cancel` | `execution_id` | `{"cancelled": bool, "status": <ExecutionStatus\|null>}` |
| `list_active` | `workflow_id?`、`limit?`(缺省 50，clamp 1..=200) | `{"executions":[...],"total":n}`（项含 `ownerPluginId`） |

- **owner 隔离**：owner 取自分派层连接身份（插件不可伪造）。`status` 跨 owner / 查无 ⇒ `null`；`cancel` 跨 owner / 查无 ⇒ `{false,null}`；`list_active` 只列本插件。
- **取消幂等**：仅 `running|paused` 才调引擎 `cancel(execution_id)`；终态一律 `{cancelled:false,status:<现值>}`，**绝不调引擎**（引擎 `cancel` 会无条件把终态覆写成 `cancelled`）。
- **模板白名单**：仅 `stock-*` 前缀（常量 `ALLOWED_TEMPLATE_PREFIXES`）；不在白名单 ⇒ `WORKFLOW_RUN_REJECTED`。
- **准入**：per-owner 2 / 全局 8（`RunLimits`，同一把写锁内判上限+计数+登记）；超限 ⇒ `WORKFLOW_RUN_REJECTED`。
- **总超时**：`total_timeout_secs` 缺省 **1800s**，clamp 1..=7200；到点调引擎 cancel，宿主视图/事件标 failed。
- **进度（不走契约）**：宿主自带 `ProgressCallback` → `DomainEvent`：`workflow/step`（`executionId/workflowId/seq/event`，**丢弃 streaming**）、`workflow/end`（`executionId/workflowId/seq/status/error`）；fire-and-forget，先 `would_dispatch` 短路。外层键 camelCase，内嵌 `event` 是引擎原生 DTO（snake_case）。
- **as_of 双包裹**：`RunOptions.node_task_scope`（逐节点 spawn 兜底）+ 执行体 `with_optional_asof`；`as_of_date` 同时补进 `input` 供 enhance 钩子读取。

### 50.3 宿主实现要点

- `start` 12 步：白名单 → as_of 解析 → `input.stock_code` → mint `execution_id` → 准入登记 → 载图（`stock-*` 复用 `load_and_inject_template` + `inject_vendor_state`；通用走 `load_workflow_template`）→ `create_workflow_with_hooks`（**必须带 hooks**）→ 回填 workflow_id → 变量合并 → 组装 `RunOptions` → `tokio::spawn` → 立即返回。
- **变量合并口径（执行者推翻「整表替换」并已采纳）**：模板 variables 为底 + 请求按 `name` 覆盖 + 新增追加。理由：模板 variables 承载 `max_concurrent` / `agent_timeout_secs` / `tool_timeout_secs` 与各 `input_mapping` 兜底值，整表替换会让未传参数静默退回模块常量。
- `get_execution_output` 只 clone 单个 `Value`，不 clone 整个 `Workflow`（20+ 节点可达 MB 级）。

### 50.4 验证

| 批 | 命令 | EXIT | 证据 |
|---|---|---|---|
| E3a | harness check / clippy / `test workflow_run` | 0 / 0 / 0 | 均命中 `Checking axagent-harness`；`4 passed; 832 filtered` |
| E3b | `check -p axagent-rt-workflow` 3m55s / clippy 46.9s / `test get_execution_output` / `check -p axagent` 13m05s | 全 0 | 前三条命中 `Checking axagent-rt-workflow`；主 crate 命中 `Checking axagent v3.0.0`；`1 passed; 343 filtered` |
| E3c | plugins check 49.9s / clippy 6.4s / `test --lib worker::tests` | 全 0 | 均命中 `Checking axagent-plugins`；`79 passed; 113 filtered`（72 基线 + 7） |

### 50.5 记账

- K1：`rsFiles 2012→2013`（`workflow_run.rs`）、`pubTraits 318→319`（`WorkflowRunProvider`）、`declaredExternalTraits 234→235`（`src-tauri/src/init/workflow_run.rs` 生产 impl）。
- `worker.rs` evidence 再后移 **+179 行**：`remote_method_not_supported` 3489→**3668**；evolver 区间 3581/3629→**3760/3808**（已 repin + 人工同步 note / planAnchorText / PLAN:265/266）。

### 50.6 未做与已登记风险

1. **`ExecutionView` 无节点计数/失败详情**：进度只能靠 `workflow/step` 事件；要 status 里也有 `nodeTotal/nodeDone/failedNode` 须扩契约。
2. **总超时后 DB 终态可能是 `cancelled`**（引擎写），宿主视图与事件为 `failed`；不额外写 DB 以免与引擎收尾竞写。是否必须一致待裁定。
3. **stock-* 生命周期钩子副作用**：插件发起执行若不传 `analysis_id`，会走 precheck/persist 分支并写 `stock_analyses`。插件迁移（#35）前需裁定：注入 `analysis_id` 跳过，或契约增业务标记。
4. **未实现**：卸载即取消、跨重启查询（`NotFound` ⇒ `null`）、`plan_callbacks` / `heartbeat` / `timeout_warning` / `system_capability_callback`（契约无字段）、`stock-*` 之外模板（白名单当前只开 `stock-`）。
5. ~~`src-tauri` 单测未编译~~ ⇒ **已确认（2026-11-03）**：`cargo test -p axagent --lib stock_workflow::workflow_run_service` ⇒ EXIT=0、**5 passed; 833 filtered**（`.tmp/res110-wfsvc-test.log`）；同法 `trading_engine_service` ⇒ **1 passed**（`.tmp/res111-tdesvc-test.log`）。测试目标已在真机上编译并跑通。
6. **双拼写**：`variables` 元素是 harness `Variable`（snake_case 字段），外层请求 DTO 是 camelCase；`input: null` 与缺失在插件侧不可区分。均已在契约注释写明，插件 SDK 文档需同步。

## 51. `trading.engine` 接缝交付账（2026-11-03；报告 `.tmp/res92-trading-design.md` / `.tmp/res98-report.md` / `.tmp/res99-report.md` / `.tmp/res102-report.md`）

### 51.1 落点与提交

| 批 | 提交 | 文件与增删 | 内容 |
|---|---|---|---|
| E4a 契约 | `2ca8730dc` | 新建 `harness/src/trading_engine.rs`（407 行）；`lib.rs` +6；`capability_registry.rs` +38；`analysis-engine/src/trading.rs` +5/−34 | `PositionSummary` / `TradeValidation` / `TradePredictionComparison` **上收 harness**（analysis-engine 改 `pub use`，调用点零改动）；新增 `TradeView` / `TradeValidationRequest` / `TradeExecutionRequest` / `TradeExecutionOutcome`；`TradingEngineProvider` 六方法 |
| E4b 宿主 | `8eeaedb90` | `analysis-engine/src/trading.rs` +308/−30；新建 `commands/stock_workflow/trading_engine_service.rs`（256 行）；`stock_workflow/mod.rs` +3；`init/state.rs` +31/−6 | 引擎新增 `execute_trade_outcome`（开关闸 + **只校验一次** + 结构化拒绝）与 `compare_trade_vs_prediction_opt`（查无分析 ⇒ `Ok(None)`），旧方法签名与文案不变；`HostTradingEngine` 六 op；`state.rs` 引擎构造上提 + 注册 |
| E4c 分派 | `26823e2d5` | `plugins/src/worker.rs` +625/−77 | 六 op 分支；`execute_trade` 需 `write`；as_of 复用 `deserialize_as_of_arg`；7 条用例（79→86） |

### 51.2 op 形状（args snake_case；嵌套 DTO 原样 camelCase）

| op | args | 成功返回 | 权限 |
|---|---|---|---|
| `get_positions` | `as_of?` | `[PositionSummary]`（camelCase） | 无 |
| `get_trades` | `stock_code?`、`limit?`（缺省 50，宿主再 clamp ≤500） | `[TradeView]` | 无 |
| `validate_trade` | `request`（对象）、`as_of?` | `TradeValidation` | 无 |
| `execute_trade` | `request`（对象）、`as_of?` | `TradeExecutionOutcome` | **`write`** |
| `compare_trade` | `trade_id` | `TradePredictionComparison \| null` | 无 |
| `trading_enabled` | 无 | `{"enabled": bool}` | 无 |

**语义要点**：
- `execute_trade` 的 `accepted=false` 是**正常业务结果**（`reason ∈ trading_disabled / validation_failed / duplicate`），不借用错误码；**DB 写失败**（`write_failed`）由宿主映射成 `Err(Provider)`，不混进 outcome（leader 在 E4b 复核时改判并复编验证）。
- **同键查重**在 provider 的 per-`stock_code` 串行锁**内**执行（否则两次并发同单同时判「不重复」）；键口径照 CSV 导入的 6 字段。
- 开关唯一权威是 settings 的 `trading_enabled`；引擎的 `enabled` 是死字段，**禁读禁写**。
- `get_positions` 内埋 `tracing::info!` 计时（持仓数 / 总耗 / 单只均耗）。**未加** `SEAM_OP_TIMEOUT_OVERRIDES` 条目（表要求「文件:行」实测出处，且 22 个未进表 op 有断言 ⇒ 无实测不得进表）。

### 51.3 验证

| 批 | 命令 | EXIT | 证据 |
|---|---|---|---|
| E4a | harness check / clippy / `test trading_engine`；analysis-engine check / clippy | 全 0 | 均命中 `Checking <目标包>`；`4 passed; 836 filtered` |
| E4b | quant check 34.31s / clippy 37.42s；主 crate check 16m27s；**改动后复编 14m27s** | 全 0 | `Checking axagent-analysis-engine` / `Checking axagent v3.0.0`；引擎 2 条新测试 passed |
| E4c | plugins check 1m02s / clippy 7.87s / `test --lib worker::tests` | 全 0 | `Checking axagent-plugins` ×2；**`86 passed; 113 filtered`**（79 基线 + 7） |

⚠ E4c 顺带把 `worker.rs` 既存的 14 处 rustfmt 漂移归位（≈64 行格式 churn，随单文件 `rustfmt` 发生，非 `cargo fmt --all`）。

### 51.4 记账

- K1：`rsFiles 2014→2016`（`harness/src/trading_engine.rs` + `src-tauri/src/commands/stock_workflow/trading_engine_service.rs`）、`pubTraits 319→320`（`TradingEngineProvider`）、`declaredExternalTraits 235→236`（宿主生产 impl）。
- `worker.rs` evidence 再后移 **+130 行**：reflector 3668→**3798**；evolver 区间 3760/3808→**3890/3938**。
- **L1 反向阻塞保持 152 处 / 28 文件**：两个宿主服务都落在 `commands/stock_workflow/` 域内路径（`workflow_run_service.rs` 从 `init/` 迁过来就是为了这条，见 §50.6 补记）。

## 52. 构建环境实测与清理（任务 #27 / #21；报告 `.tmp/res97-target-research.md`，490 行）

| 事项 | 实测（2026-11-03） |
|---|---|
| 生效 target | `CARGO_TARGET_DIR=C:\Users\刘小平\.cargo\target-axagent`（User + Process 两级），**覆盖**仓内 7 个子根 `.cargo/config.toml` 的 `../src-tauri/target` |
| `src-tauri/target` | 101.27 GiB / 98,447 文件，最后写入 2026-11-01 18:34 ⇒ **死重**。经确认后已删除（`.tmp/res101-cleanup.log`：`RM_EXIT=0`、目录不存在）；删除后 `cargo check -p axagent` 复编 EXIT=0（14m27s）⇒ 真实 target 未受影响 |
| D 盘 | `DriveType=4 / HGFS`（VMware 共享），元数据操作比 C 慢 160–370 倍 ⇒ **#21「target 迁 D」否决**；C 盘删后剩 214 GB |
| lock 漂移 | 8 份 lock 并集 1,061 包，303 包多版本；公共 360 包中 **159 个版本不一致**（旧簇 `{src-tauri, axagent-core}` vs 新簇其余 6 根） |
| 重复编译 | live target 内 831 crate / 5,639 编译单元（均 6.8 份）；`axagent_harness` 仍 36 份；fingerprint 里 `tokio` hash 24 种、`sea_orm` 28 种 ⇒ 根因是**依赖图哈希沿根分裂**，不是 profile 未对齐 |
| 额外分裂源 | 两套 rustc（1.97.0 / 1.97.1）：`rust-toolchain.toml` 只在 8 个根内，仓库根没有 ⇒ 从仓库根起 cargo 用默认工具链，同 target 写两套产物 |
| CI | `Swatinem/rust-cache` 只缓存 `src-tauri -> target`，而 CI 用 `--manifest-path` 从仓库根跑 ⇒ **7/8 根零缓存** |

**后续动作（未做，按风险排序）**：① 仓库根补 `rust-toolchain.toml`；② 统一访问视图（全树固定 `D:\` 本地路径，不再混用 UNC）；③ lock 收敛门（不是合并 lock）；④ CI 缓存覆盖 8 根；⑤ target 内 ≈16.6 GiB 冗余产物的定向清理。⚠ 硬约束：共享 target 下不带 `-p` 的 `cargo clean` 会清掉 **8 根共用整份 target**。

## 53. `trading.engine` × 股票模板的并发交叉，与 #25 门红结案（2026-11-03；报告 `.tmp/res106-dataverifier.md`）

### 53.1 #25「data-verifier.rhai 阶段3 数值判据门」结案：归因纠正

- 门 **`scripts/check-rhai-numeric-typing.mjs`**：`--selftest` 与判定实跑均 **EXIT=0**（扫描面 86/21/2，非零排除假绿）。
- **红因不是「他人未提交改动」**：未桥守卫由 v65 阶段3 写进 HEAD，修复同样在 HEAD（`2f9bfec23`，`data-verifier.rhai:474` 收编 `num_of` 桥，`seed_serenity.rs:114` 的 `TEMPLATE_VERSION` 65→66 并在注释里点名本次事件）。当前 24 项未提交改动**没有一个**落在该门扫描面或其输入里。
- `NaN` / `±INF` 已按铁律逐情形推演：旧 `type_of` 守卫与新 `num_of(x, 50.0)` **完全等价**（+INF→封顶 54、-INF 原样、NaN 两侧同）。
- 已接 CI 两处同源：`scripts/ci-check.mjs:344-345` ↔ `.github/workflows/ci.yml:259-262`（自检先于判定）。
- **cargo 侧已确认（2026-11-03）**：`__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_workflow::rhai_registry::tests::data_verifier` ⇒ **EXIT=0，4 passed; 834 filtered**（`.tmp/res107-dv-test.log`）。D 类残余消除，#25 彻底结案。判据本身仍**没有 NaN/嵌层整数用例**（防回归只靠静态扫描），列为可选加固项。
- ⚠ 方法论教训：**pending 登记必须留当时的门禁输出留档**。本轮多条「卡他人改动」的 pending 需要按同样方式复验（#25 复验结果是误归因）。

### 53.2 接缝 × 股票模板：写面无交集，但读面单向交叉

- **写面交集：0**。serenity 模板（`stock_analysis_setup/seed_serenity.rs`）全文无 `execute_trade` / `get_trades`，落库走 `reco_pool`，不写 `trades` / `portfolio_holdings`。
- **共享读面**：接缝写 `portfolio_holdings`（`analysis-engine/src/trading.rs:437-475`、`:694-731`），而模板在 `stock_workflow/hooks.rs:860-862`（`portfolio-risk-gate` 注入全表持仓）与 `stock_workflow/core.rs:1575-1590`（退出判定）读它。
- 四条并发风险（按严重度）：
  1. **锁不跨面**：`trading_engine_service.rs:173-196` 的 per-`stock_code` 串行锁只约束接缝自身；模板两处读**不经该锁** ⇒ run 中点读到旧持仓/旧市值。
  2. **无事务多表写**（`trading.rs:372` 注释自证「先 trades 后 holdings」）⇒ 半态窗口内并发读会拿到不一致组合。
  3. **as_of 与实时写交叉**：接缝只对引擎调用套 `with_optional_asof`（`:199-202`），但同函数 `:176` 的查重读的是当前表 ⇒ 回放 run 叠加 live 下单时，查重与行情读数分属两个时间态。
  4. **锁是进程本地**：不约束 gateway 等其它写入口，跨入口并发写同股时「锁内查重」失效（设计边界，非新缺陷）。
- **当前评级：中低**（前提：不在 serenity run 进行中触发 `execute_trade`）。若要放开并发，需先做「risk-gate 持仓快照一致性」测试或文档化互斥约束。

### 53.3 能力面进度（截至本轮）

| 接缝 | op 数 | 写 op 权限闸 | 状态 |
|---|---|---|---|
| `market.quote` | 40 | 无（全只读） | 已交付 |
| `jobs.store` | 6（3 读 + 3 写） | `add`/`remove`/`set_status` 需 `write` | 已交付 |
| `workflow.run` | 4 | `start`/`cancel` 需 `write` | 已交付 |
| `trading.engine` | 6 | `execute_trade` 需 `write` | 已交付 |
| `stock.monitor`（并入 aggregator） | 9（5 读 + 4 写） | `set_t0_config`/`upsert_alert`/`remove_alert`/`set_aggregator_config` 需 `write` | **契约已交付**（§55），宿主 provider 与分派待做 |
| `quote.watcher` | 3（1 读 + 2 写） | `watch`/`unwatch` 需 `write` | **契约已交付**（§55），宿主 provider 与分派待做 |
| `knowledge.vector` / `trajectory.store` | 待设计 | 待定 | 调研中 |

### 53.4 主 crate 测试目标实跑（2026-11-03）

| 命令（`src-tauri` 根，`__TAURI_WORKSPACE__=true`） | EXIT | 结果 |
|---|---|---|
| `cargo test -p axagent --lib stock_workflow::rhai_registry::tests::data_verifier` | 0 | `4 passed; 0 failed; 834 filtered out`（`.tmp/res107-dv-test.log`） |
| `cargo test -p axagent --lib stock_workflow::workflow_run_service` | 0 | `5 passed; 0 failed; 833 filtered out`（`.tmp/res110-wfsvc-test.log`） |
| `cargo test -p axagent --lib stock_workflow::trading_engine_service` | 0 | `1 passed; 0 failed; 837 filtered out`（`.tmp/res111-tdesvc-test.log`） |

⇒ 两条接缝的宿主服务与 `data-verifier` 判据都已在**真测试二进制**上跑通（不是 `--no-run`）。⚠ 遗留可选加固：`data_verifier_factor_scores_override_and_rank` 无「嵌层整数 + NaN」用例（res106 §5 处置 3）。

## 54. #35 股票迁出的可执行分解与八条裁定（2026-11-03；报告 `.tmp/res109-migration-plan.md`，454 行）

### 54.1 判据纠偏（最重要）

- **152 不是「152 个待补接缝」**。按被引用模块符号归桶（加总闭合）：**O=45**（`tools/opc.rs` 41 + `fleet/mod.rs` 4，import 的是 `analysis_engine::opc::`，因审计按 crate 名归到股票名下）⇒ **股票迁移一条不许动**；**H=35**（宿主自有：`services.rs` 装配与 cron callback 15 + `state.rs` 6 + `app_state.rs` 6 + `gateway` 2 + 其余各 1）⇒ 按现 `ownedRe` 口径**结构性不可消减**；**M=34**；**U=21**；**X=8**；**P=9**。
- ⇒ **迁移可消减上限 = 107 处**；B1–B7 全做完的现实终态 **≈96 处 / ≈18 文件**；**下界锁死 ≈83**。
- ⚠ 原来写的「改写 152 处主壳引用」当验收，**永远不可能通过**。判据改为：**U/M/X/P 四桶按名单归零；H 桶 = 一份逐条点名的白名单；O 桶随 #32 另收**。
- 两条审计脚本瑕疵（登记，暂不改判据口径）：① **（2026-11-03 作废）** 原登记「只剔行首注释 ⇒ `background_tasks.rs:266` 的 `///` 被计入、152 里含 1 处假阳性（真实 151）」——今盘上不成立：`audit-l1-blockers.mjs` 的判据已是 `t.startsWith('//') || t.startsWith('///')`，该文件报 3 处 @8/350/916，`:266` 不在列 ⇒ **152 是干净的 152**（详见 §61.1）；② 28 文件按行号回读只有 **78 处**能在该行匹配到 `axagent_<域crate>::<模块>`，余 74 处是多行 `use {…}` 续行 / 裸前缀 / `AppState` 字段类型行 ⇒ **桶边界有 ±5 不确定**。因此**逐行点名只用于 B1 的 24 处（实数，见 §61）**，其余批次一律按**文件级**承诺。

### 54.2 八条裁定

| # | 事项 | 裁定 |
|---|---|---|
| R1 | 域数据落库通道（`state.harness.db()` 在 `stock_analysis.rs` 单文件 **100 处**，涉及 ≥14 张表） | **方向采纳但分两批，不许一步做成 KV 大桶**：第一批先处置既存半截工程 `session.store`（注册口 `capability_registry.rs:1036`，但 `dispatch_seam` **0 处**分支 ⇒ 插件根本调不到）——要么补分派支、要么显式摘除；第二批再设计 `domain.store`：**命名空间白名单 + 每命名空间声明 DTO 类型名与 `schema_version` + 宿主按注册类型校验后才落库**（守住「ORM 不过线」与表级类型约束）。未裁定完之前 **#35 视为不可开工** |
| R2 | 凭证 4 字段跨进程 | **不过线**（沿用 §44.2「pub 字段不是方法，永不跨进程」）。走宿主 secret 注入 + 清单 `config`；`market.ops` 只暴露「已注入与否」的只读诊断。理由：op 参数会落进 `trajectory` 记录 |
| R3 | 插件任务路由（`jobs.execution`） | **开**，但只对 `stock-` 前缀 `task_type` 生效，14 个种子 id 仍走宿主 handler；**排在 B2 之后**（不开 ⇒ 插件建的任务能列能删不能跑，是第二条「半条链」）。执行**触发权**仍在宿主，不与 §47.2 裁定 1 冲突 |
| R4 | `workflow.hook`（宿主→插件回调） | 排 **B4 之后、B6 之前**；首批只 `pre_run` + `on_end`；`post_step` 在热路径（每步一次）默认关，等 B3 的每订阅者超时/丢弃计数落地再谈 |
| R5 | 审计口径 | **不扩 `ownedRe`**，改为 `audit-l1-blockers.mjs` 加 `hostOwnedAllowlist`（键 = file + 锚文本，**不含行号**，与 `component-neutrality-allowlist.json` 同族）。白名单**可审**，正则扩面**不可审** |
| R6 | 前端 barrel | **拆，不留垫片**；宿主侧域类型一律走 `src/types/generated/` 生成物。留 `export *` = 宿主仍编译期依赖域文件 ⇒ 54 处只降不零 |
| R7 | B5 的 41 处 `tools/opc.rs` | **归 #32（OPC）**。B5 消减承诺从 −49 改为 **−8**（`audit` 4 + `astock_data` 3 + `capability_pack_defs` 1），不许虚报终态 |
| R8 | 审计产物日期命名 | 现状用 UTC ⇒ 同日重跑覆盖既有快照，且 git 里表现为 modify 而非 add，与「审计报告 = 时点快照」语义冲突。改为本地日（`Intl.DateTimeFormat('sv')`）+ 同日二次跑加 `-HHMM` |

### 54.3 批次表与首刀（B1 不是接缝批）

| 批次 | 名称 | 前置 | 消减反向阻塞 |
|---|---|---|---|
| **B1** | **公用件上收（首刀）**：`trading_calendar` / `task_cancel` / K 线形态与背离 / `NewsArchiveSink` / `LIVE_GENERATION` → harness，原处变 `pub use` 垫片（`as_of` 运行层与 `risk` **不在首刀**，理由见 §61.2 J2/J3） | 无 | **−18 处 / −2 文件**（实数 24 处 / 7 文件逐行见 §61.1；原写「−13 处 / −4 文件」两个数都不成立） |
| B2 | `market.ops` + `stock.monitor` + `quote.watcher` 三接缝 | B1 | −6～−9 |
| B3 | 事件面补强（第四跳 + 每订阅者超时 + 丢弃计数） | B2 | 0（但它是 B6 的安全前提） |
| B4 | L1 插件打包（本体搬迁，`plugins/stock/`） | B1+B2 | 0（走 ownedRe 桶） |
| B5 | MCP 直通 + 运维面收口 | B4 | **−8**（R7） |
| B6 | `AppState` 摘 6 个域字段 + 测试替身同步 | B1–B5 | −14 |
| B7 | 算法接缝 + P 桶随本体 | B4 + **R1 已裁定** | −9 |
| B8 | 前端收口（54 处） | B4 | 前端桶（与 152 不同轴） |

**为什么首刀是 B1 而不是接缝批**：B1 零接缝依赖、零语义变更、能**当场把 152 降到 ≈139** 且 13 处点得到行号（逐字保真法沿用 §44.1 的三段区间 sha1 对照）；而 `stock.monitor` / `quote.watcher` 落在域内路径，**按设计就不降读数**（只保证不抬高）——把它们当首刀会造成「交付了但读数没动、判据看着像失败」的假进展。B1 同时是 B2 的前置（否则 B2 会在 `commands/stock_workflow/` 里再造一份日历，违反禁区 12）。

### 54.4 其它实跑事实

- 主 crate 测试目标已实跑：`data_verifier` 4 passed、`workflow_run_service` 5 passed、`trading_engine_service` 1 passed（§53.4）。
- 注册表命令总数 **1543**；股票迁完后 **1543 − 304 = 1239**（⚠ 其中已含 `screenshot_diagnosis` 8 + `decision_gates` 3 + `workflow_template` 16 三类跨域命令，**不得二次扣**）。
- 本体体量：域自有 Rust 引用 **450 处 / 39 文件**；前端域自有 **293 文件 / 84,873 行**。
- res104 的 `monitor.rs` 行号有漂移（`apply_panel_runtime_vars` 记 `:432`，盘上实测 `:348` 起）⇒ 实现批以盘上为准。

---

## 55. `stock.monitor` + `quote.watcher` 契约批交付账（2026-11-03；设计 `.tmp/res104-monitor-design.md` 423 行，交付报告 `.tmp/res108-report.md` 259 行，提交 `366ff9c04`）

### 55.1 交付面（L1 契约层，本批**不含**宿主 provider 与 worker 分派）

| 文件 | 行 | 内容 |
|---|---|---|
| **新建** `axagent-core/crates/harness/src/stock_monitor.rs` | **855** | `StockMonitorProvider`（9 op，`:469`，全默认 `Err(Provider)`）+ 新类型 `MonitorStatusView :367` / `AlertUpsertRequest :383` / `PriceAlertView :408`（**12 列**镜像 `price_alerts`，`isTriggered` 保持 `i32`）/ `AlertUpsertOutcome :435` / `RemoveAlertOutcome :451` + clamp 常量 `:342-348` |
| **新建** `axagent-core/crates/harness/src/quote_watcher.rs` | **243** | `QuoteWatcherProvider`（3 op，`:104`）+ `WatchOutcome{requested,applied}` + `WATCH_BATCH_LIMIT=200 :50` / `WATCH_SET_LIMIT=1000 :52`，配 `const _: () = assert!(BATCH <= SET)` 编译期不变式 |
| **上收 + `pub use` 垫片** | — | `MonitorConfig` / `TZeroConfig :90` / `MonitorAlert` / `PollRoundStats` ← `analysis-engine/src/monitor.rs`；`SignalType`/`SignalDirection`/`StockSignal`/`PortfolioSignal`/`PortfolioSnapshot`/`AggregatorConfig` ← `cross_stock_aggregator.rs`；`WatchPriority :64` ← `astock-data/src/realtime_quote.rs` |
| `harness/src/event_bus.rs` | `:103` | **`EventCategory::Stock`**（+ `as_str()`），kind 前缀 `stock/` |
| `harness/src/capability_registry.rs` | `:164` / `:178` / `:1044-1070` | `ServiceDefinition::stock_monitor()` / `quote_watcher()` + 四个 register/get（走既有 `register_seam` / `provider_as`） |
| `scripts/check-monitor-event-authority.mjs` | +168 −6 | 新判据 **R18**：`TZeroConfig` 的 Rust serde 注解（`rustStructSerdeAttrs :189`）与前端 `invoke` 读侧形状（`tsInvokeShapeKeys :208`）逐键对 `TZERO_CAMEL_KEYS :103` 相等；`ruleTZeroConfigCamel :513`。自检 71 → **79** 条 |

⚠ 上收时**随体 inherent impl 必须一起搬**（Rust 不允许跨 crate 给 foreign 类型写 inherent impl）：实删 **9 处**定义而非 res104 记的 6 处；`WatchPriority::interval_ms` 原**私有**、被宿主轮询循环调用 ⇒ 升 `pub`，数值逐字不变。

### 55.2 本批修掉的两条既存缺陷

1. **`TZeroConfig` 的 camelCase 漂移**（res104 §0-3，即「T+0 面板写了不生效」的静态判据）：Rust 侧补 `#[serde(rename_all = "camelCase")]`，**故意不加 `serde(default)`** 以保留「缺字段即报错」的防线。两处前端单测的 mock 原本用 camelCase（`PriceAlertPanel.t0.test.tsx:35-37` / `.alertRow.test.tsx:98-100`）而真后端吐 snake_case ⇒ mock 比被测代码更早假设了修复后的形状，这正是「单测全绿却看不见缺陷」的成因；修后 mock 变成真话。
2. **告警类型白名单**：`alert_mapping.rs:96-104` 实际接受 **5 个** legacy 写法（`above`/`below`/`change_up`/`change_down`/`volume_spike`，归一后落 4 类），res104 记的「4 个」不准。契约文档按 5 个写，provider 白名单照抄不许少一个（`normalize_alert_type` 对认不出的串静默回落 `take_profit`，插件侧一条拼错的串就会落成反方向的单）。

### 55.3 验收（工作根 × 命令 × EXIT × 日志，全部为独占写入的终态）

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| A1/A2 | `cargo check` / `clippy -p axagent-harness --all-targets -- -D warnings`（axagent-core） | 0 / 0 | `Checking axagent-harness` |
| A3/A4 | `__TAURI_WORKSPACE__=true cargo test -p axagent-harness --lib stock_monitor` / `--lib quote_watcher` | 0 / 0 | **10 passed** / **6 passed** |
| A5/A6 | `cargo check` / `clippy -p axagent-analysis-engine`（axagent-quant） | 0 / 0 | 含 harness 全链 |
| A7/A8 | `cargo check` / `clippy -p axagent-astock-data` | 0 / 0 | — |
| B1/B2 | `node scripts/check-monitor-event-authority.mjs --selftest` / 判定 | 0 / 0 | **79/79**；R1–R18 全绿，含 `✓ R18 …（4 键逐项相等且带 camelCase 注解）` |
| B3 | `cargo fmt -p axagent-harness -- --check` | **1** | 仅剩**非本批**文件 `market_data.rs` 4 处 diff（本机 rustfmt 1.9.0 与仓内基线的工具链差）；本批 4 个 Rust 文件零 diff ⇒ 仓级 fmt 既不由本批引入也不由本批修复 |
| B4 | `cargo check -p axagent-plugins --all-targets` | 0 | 契约面对下游零破坏 |

### 55.4 我对 res104 §J 八条的裁定

| # | 事项 | 裁定 |
|---|---|---|
| J1 | DTO 落点 | **上收 harness + `pub use` 垫片**（纯数据类型不建镜像）；只对「需拼多源」的建 `MonitorStatusView`。✅ 本批已执行 |
| J2 | camelCase 漂移是否本批修 | **是，且作为接缝前置**（否则两个 op 上线即继承同一个 bug）+ 补 R18 防漂。✅ 已执行 |
| J3 | `price_alert` 用哪个 `EventCategory` | **新增 `EventCategory::Stock`**，kind = `stock/price_alert` / `stock/portfolio_signal` / `stock/quote_change`。✅ 枚举已加；⚠ kind 常量尚未落地（见 §55.5-N6） |
| J4 | `set_t0_config` 给不给插件 | **给**。要求 `write` 权限 + provider 侧 clamp（`minIntervalMinutes ≥ 1`、两阈值 > 0），文档注释点名「开启后影响所有告警的宿主行为」。trait 已这样写，闸在分派批 |
| J5 | `watch` 数量闸 | **单次 ≤200、集合 ≤1000，超限截断不报错**，响应给 `requested`/`applied`。常量已在 `:50/:52`，行为在宿主批 |
| J6 | `list_watched` 带不带 priority | **第一批不带**（零引擎改动），但 provider 侧**必须 `sort`**（`HashMap` 键无序，不排序则断言不可写） |
| J7 | `aggregator_stats` 进不进首批 | **不进**（需新引擎方法，`last_triggered` 无公开读口）；插件要「窗口内几只同向」用 `aggregator_buffer` 自数 |
| J8 | 注册时机 | **`init/services.rs` 装配成功后就地注册**（`stock_monitor.set` 成功后、`quote_watcher.set` 成功后两处），不在 `state.rs` 靠 `AppHandle` 每次取 ⇒ 少一个 None 分支 |

### 55.5 我对 res108 §6 七条口子的裁定（宿主批 / 分派批必须照此执行）

| # | 事项 | 裁定 |
|---|---|---|
| N1 | J2 缺陷只有静态判据 | **接受**。宿主批收口前须在真机走一次 T+0 开关并回填结论；静态门绿 ≠ 现象消失 |
| N2 | `upsert_alert` 入参形态（平铺 vs 嵌一层） | **嵌一层**：`{"request": {...}}`，照 `worker.rs:2031-2035` 的 `deserialize_arg(&args, "request", seam)` 先例。分派批只能按这一种写，否则报 `missing field` |
| N3 | `PriceAlertView` 是否第二套真相 | **算必要的镜像**：`entities` 依赖 `harness`，ORM `Model` 反向过线即成环 ⇒ 只能建视图，先例 `trading_engine.rs:57-87` 的 `TradeView`。按 **12 列**逐字镜像 |
| N4 | 两组 clamp 常量可能漂移 | **不新增形态锁**（门禁钉形态、钉不了数值）。改为：宿主 provider **只准引用 harness 常量**，`analysis-engine` 里那组 `POLL_INTERVAL_RANGE`（现 `monitor.rs:20`）与冷却守卫改引同源常量；宿主批 provider 单测加一条 `assert_eq!` 锁区间，比再加一条 R19 更省 |
| N5 | `aggregator_buffer` 无序 | **宿主 provider 侧 `sort`**（零引擎改动，一行），与 J6 同口径 |
| N6 | stock 域 kind 的权威落点 | **进 harness 常量表**（三条 `pub const`，紧邻 `EventCategory`），发射点**只准引用常量、禁止字面量**；**不复用 `IpcEventName`**（那是 Tauri 事件名，与 `DomainEvent.kind` 不同域）。否则复刻 R7/R14 管的「两头各抄一遍」 |
| N7 | 存量 JSON 是否 snake_case | **删死代码消掉这个问题**：`RealtimeMonitor::subscribe`、`serialize_configs`、`deserialize_configs` 三处全仓零引用（grep 已确认），宿主批一并删除 ⇒ 不再有「按 camelCase 重序列化历史串」的可能 |
| N8 | `MonitorStatusDto` 与 `MonitorStatusView` 两份定义 | 宿主批把 `commands/stock_analysis.rs:2334-2347` 改为 harness 类型的 `pub use`（线上形状不变），并**同批**把门禁 R12 的 Rust 读源从 `stock_analysis.rs` 改指 harness（做法同本批对 R4/R16 的改线）——否则 R12 会因抽不到类型判「判据没电」而红 |

⚠ 另两条留给分派批的陷阱：① **op 名 ≠ 方法名** —— trait 方法叫 `t0_config`，发布的 op 叫 `get_t0_config`（其余 8 个一一对应）；② `WatchPriority` 命令层历史上还认 `bg` 别名（`stock_analysis.rs:2400-2403`/`:2462-2465`），serde 只产出 `background` ⇒ **接缝不继承这个宽容**，`bg` 一律 `SEAM_CALL_INVALID_ARGS`，要给别名必须在 provider 里显式映射。

### 55.6 K1 与剩余批次

- K1：**2016/320 → 2018/322**（+2 文件 = 两个 harness 新契约文件；垫片未删 ⇒ 不降）。`declaredExternalTraits` **保持 236** —— 两个新 trait 此刻**还没有生产实现者**，宿主批注册后应为 **238**。
- 下一批（宿主）：新建 `commands/stock_workflow/monitor_seam_service.rs` + `quote_watcher_service.rs`，`upsert_alert` **直接调** `price_alert_upsert::upsert_price_alert_and_wire`（不抄第二遍），`monitor_emitter.rs` 加第四跳 `dispatch_event` 并多带一个 `Arc<EventDispatchBus>`。⚠ 新文件必须落在 `commands/stock_workflow/`（ownedRe 桶），落 `init/` 会把反向阻塞读数从 152 抬到 154（§53 的同一课）。
- 再下一批（分派）：`worker.rs` 新增 12 个分支（`stock.monitor` 9 + `quote.watcher` 3，形态照 `trading.engine` 分支 `:1990-2106`）+ 6 条 `required_permission_for` 登记 + 单测（⚠ **基线订正**：此处原写 86，实测链条是 88（#104 开工基线，`.tmp/res114-00-baseline-test.log`）→ 96（#104 终态）→ 108（#109 终态，`.tmp/res109-A2-test-worker.log`）；86 是 `res102` 时代的数）；**不进** `SUPPORTED_REMOTE_SEAMS`（方向相反）。

---

## 56. `knowledge.vector` + `trajectory.store` 的裁定（2026-11-03；调研报告 `.tmp/res105-vector-trajectory-design.md`，390 行）

### 56.1 六条待裁定的裁决

| # | 事项 | 裁决 |
|---|---|---|
| Q1 | 接缝 ID 取 `knowledge.vector` 还是 `rag.vector.store` | **`knowledge.vector`**。§47.3 顺序表（`PLAN:2245`）用的就是它，且首批 op 面（检索 / 关键词检索 / 入库 / 集合信息）**不止 RAG 存储**。代价我认：交付批必须**同批**改 `harness/src/vector_store.rs:12` 模块头、`PLAN:317` 组 K 预留行、`scripts/capability-register.json:956` 行的 `seamId`/`faces`/`evidence`/`planAnchorText`（锚点文本要与 PLAN:317 逐字一致，否则 repin 判错）。**历史章节里的 `rag.vector.store` 提法一律不改**（§704/1460/1499/1571/1864 是时点记录）⇒ 两名的关系只在**本节**说明一次：预留名 → 交付名。**两名并存地写进新代码是最坏解**，判据只认 `capability_registry.rs` 里那一个字面量。 |
| Q2 | 首批开几个写 op | **只开 `index_text` 一个**（`write` 闸 + 硬上限）。**`delete_document` 不开** —— 比 res105 的理由更硬的一条：`delete_document_embeddings`（search `:990`）**只清向量、不删文档行**，宿主侧删文档走 `knowledge.rs:982` 的「先清旧向量再删行」同事务路径 ⇒ 插件拿到这个 op 只能造成**半截删除**（库里留孤儿行）。删除权留在宿主命令面。 |
| Q3 | 为 `list_collections` 扩 #40 的 trait 面？ | **不扩**。#40 三 trait 共 25 方法、宿主 `VectorStore` 28 个 pub 方法**零列举能力**，扩面属改冻结契约。首批以 `get_collection_info`（trait 已有 `get_collection_dimensions`，harness `:87`）替代。 |
| Q4 | `trajectory.store` 开不开 `append` | **首批只读**（`list`/`get`/`query`/`count` 四个 op）。res105 B9 的五条判据全部成立，尤其：`outcome`/`quality`/`value_score` 是**可写字段**（`trajectory_types.rs:105-108`）而进化门槛读它们（`reflection.rs:4515` 取 30 条、<10 条拒进化）⇒ 插件伪造一条 `Success` 就能抬分母。**若将来必开**，走「最小写子集」（单 op + 插件前缀 `agentName` + 入参禁带 `steps`/`rewards`/`quality`/`valueScore` + 速率闸 + 先补 `TRAJECTORY_*` 码族与 11 语言译键）。 |
| Q5 | 业务错误码透出到帧 `code`？ | **不改协议**。分派层继续 `PLUGIN_CALL_FAILED` 兜底，`KNOWLEDGE_*`/`VECTOR_*` 放进 payload 的 `code` 字段（与 `trading.engine` 的「业务结果走 payload `reason`、不借错误码」同源）。反向选择会让每个新码都欠 11 语言译键（AGENTS「后端错误码 i18n 规范」第 7 条）⇒ **首批零新增码**。 |
| Q6 | `TrajectoryQuery` 加 `rename_all` 还是新建视图 | **新建 `TrajectoryQueryView`**（camelCase + `sinceMs`/`untilMs: i64` epoch），**不动** `harness/src/trajectory_types.rs:288` 原形：它被宿主 `storage.rs:409` 与 proactive/dream/closed-loop 消费，改注解会同时改落库与既有 IPC 的键形态。 |

### 56.2 我在裁决之外补的四条（res105 没问，但会决定形状）

1. **`index_text` 的 args 只收 `{collection_id, document_id, content}`** —— 删掉 res105 op 表里的 `embedding_provider` 与 `dimensions` 两个入参。理由：**插件不得指定 provider**（那是宿主配置与计费主权，`indexing.rs:618 build_embed_context(db, master_key, &provider_id)` 需要 `master_key` 解密），维度必须由宿主按所选 embedding 模型决定；让外部传 provider 名等于把「任意触发模型计费」的口交给插件。
2. **`index_text` 不许隐式建集合**：写前必须先读 `get_collection_dimensions`，集合不存在 ⇒ 落 `Err`（分派层映射 `SEAM_CALL_INVALID_ARGS`），**禁止**顺手 `ensure_collection`。否则插件可无限造命名空间，而删除权在上一行已被收归宿主 ⇒ 造完没人能清。这条是 Q2「不开 delete」的**配套前提**，两条必须同时成立。
3. **硬上限写进 provider 而不是文档**：`content ≤ 64 KB`、`search.top_k ≤ 50`、`list.limit ≤ 200`，超限即截断或 `Err`（照 `jobs_store.rs` 把校验放在实现方的做法）。**速率闸首批不做**（协议层只有 `write` 一档权限，`types.rs:474-484` 无粒度；挪用 `SECURITY_RATE_LIMIT_EXCEEDED` 属跨域借码，不做）—— 代价如实登记：**一个乱调 `index_text` 的插件能把宿主 embedding 花费推高**，这是已知未修风险，不是已解决。
4. **provider 落 `src-tauri/src/commands/` 根**（res105 §4 批 2 的推荐，我采纳）：`audit-l1-blockers.mjs:38-40` 的计数只看 `useRe` 是否 import STOCK 那 4 个 crate，本两条 provider 只 import `axagent_search` / `axagent_trajectory` ⇒ 落哪个目录都不抬 152/28；能力本身跨域（知识库 + 轨迹服务多条业务线），**不随股票插件迁出**。⚠ 这条与 §55.6 给 `stock.monitor` 的口径相反且都正确 —— 差别就在 import 面：`monitor_seam_service.rs` import `axagent_analysis_engine`，必须在 ownedRe 覆盖的 `commands/stock_workflow/` 下。

### 56.3 基线与记账（res105 §5 的预告表按新基线重算）

- 现值 **2018 / 322 / 236**（§55.6，res105 报的 2016/320 已被契约批 `366ff9c04` 抬过）。
- **契约批**：+2 文件（`harness/src/knowledge_vector.rs`、`harness/src/trajectory_store.rs`）、+2 trait（`KnowledgeVectorProvider`、`TrajectoryStoreProvider`）⇒ **2020 / 324**；`declaredExternalTraits` **必须仍 236**（声明有、非 harness 实现无 ⇒ 交并不进）。
- **宿主批**：+2 文件（`commands/knowledge_vector_service.rs`、`commands/trajectory_store_service.rs`）⇒ **2022 / 324**，`declaredExternalTraits` **236→238**。⚠ 分两批改镜像表，批 1 就改 238 会判红。
- 不受影响、**别顺手改**：`capacityRegister 23`、`groupCounts`（`totalRows 37`）、`remoteSeamRegistryCount 13`（两条新接缝都**不进** `SUPPORTED_REMOTE_SEAMS`，方向相反）。唯一例外是 §0.7.7 的 `rag.vector.store` 预留行（`json:956-973`，`planLine 317`）——按 Q1 改名并改写 `reason`（「capability_registry 命中 0 ⇒ 判未建」交付后会变成**过期的真话反面**）。
- res105 §5 风险 2（「复用 #40 三 trait 当契约会不会让 236 意外跳变」）**随 Q1–Q3 的裁决自动消解**：不复用、新建 provider trait ⇒ 交并集只在宿主批才成立。

---

## 57. `#101` 落地：H 桶白名单入盘 + 审计快照改用本地日（2026-11-03；承 §54 R5/R8）

### 57.1 判据改动（`scripts/audit-l1-blockers.mjs`）

1. **H 桶从「桶计数」变成「名单」**：新增 `scripts/l1-host-owned-allowlist.json`，条目 = `{file, anchorText, reason}` —— **不存行号**（行号随编辑漂移，本仓已反复为此重锚）。审计时逐行命中锚点即计入 H 桶，其余仍是「待改写」。
2. 反向阻塞读数拆成三行，**合计与旧口径逐字可比**：

| 口径 | 股票 STOCK | OPC |
|---|---|---|
| 待改写（H 桶除外） | **142 处 / 24 文件** | 0 |
| H 桶（本批点名豁免） | 10 处 / 4 文件 | 0 |
| **合计（= 旧 152 口径，镜像表锚点仍指这一行）** | **152 处 / 28 文件** | **0 处 / 0 文件** |

3. **腐烂复扫三判据**（`--check-allowlist` 模式，任一即 EXIT=1）：① 锚点在文件原文里找不到；② 同 `(file, anchorText)` 重复；③ 锚点还在但那一行今天不再计入 H 桶（文件被划成域自有 / 行成注释 / 不再 import 域 crate）= **豁免空转**。⇒ 白名单不会在迁完后继续「豁免」已经不存在的行。本模式**只读**，不写快照。
4. **R8 已修**：快照文件名从 `toISOString()`（UTC 日）改为**本地日**，同日二次跑追加 `-HHMM` 不覆盖既有快照。⚠ 实证代价：本轮第一次跑（04:54 本地 = UTC 前一日 20:54）把 `AUDIT-l1-blockers-2026-11-02.md` 覆盖了 —— 该文件未入库（`?? ` 状态），读数与 res109 完全一致（152/28、304/20）⇒ **无数据损失，但归属日确实错了**，正是 R5/R8 要点名的形态。改后当日重跑正确落 `AUDIT-l1-blockers-2026-11-03.md`。
5. **CI 接线（两处同源）**：`scripts/ci-check.mjs` 的 `step("L1 反向阻塞 H 桶白名单逐条锚点复扫", …)` 与 `.github/workflows/ci.yml` 的 `Check L1 blocker host-owned allowlist anchors` 命令行逐字同源；不加 `if (!quick)` 门控（AGENTS「CI 接线」：纯 Node 静态门必须走 pre-push 档）。

### 57.2 首批名单只点 10 处，**故意不点满 35 处**

res109 §A2 的机械归桶读数是 **H=35 处 / 9 文件**，但其中 `services.rs`（H 15）与 `state.rs`（H 6）**只给了桶计数、没给逐行归属**，`app_state.rs` 标 H→X、`daily_snapshot_store`/`decision_gates`/`screenshot_diagnosis` 是 M 或 P/H 混合。⇒ 本批只收**整文件判 H 且行号已点名**的四处文件共 **10 处**（gateway 2 + monitor_emitter 1 + browser_fetcher 1 + app_state 6）。**没有逐行证据就不写进豁免名单** —— 名单宁可短，短了只是读数不好看，写长了就是假话。余 **25 处**（含 `services.rs`/`state.rs` 的装配段）等 B2/B6 按行落名，同批必须补 `anchorText`。

### 57.3 顺带纠出的既存引用腐烂（同批改指）

审计脚本头部注释加了 11 行 ⇒ 内部段落整体下移，`check-capability-register.mjs` 与镜像表里**四处按行号抄的引用**当场失效，已重指：`DOMAINS 36-51 → 48-63`（两处）、`SKIP 17 → 29`、`注释剥离 66-69 → 87-90`、`foreign 判据 55-77 → 65-126`。⇒ 又一次实证「按行号的同源引用必须与判据同批改」，与 §54 R5「白名单可审、正则不可审」是同一件事的两面。

## 58. `stock.monitor` + `quote.watcher` 宿主批（#103）交付与对执行者 17 条「判据不足」的逐条裁定

> 执行：软件开发专家团 · 宿主批（res115）｜日期：2026-11-03｜范围：`src-tauri` 宿主 provider + 注册 + 第四跳，harness 只追加 kind 常量，dao 加两个函数，引擎删死代码
> 上位：§55（契约与分派）· 调研 `.tmp/res104-monitor-design.md` §H/§I · 交付报告 `.tmp/res115-host-report.md`

### 58.1 交付面（落盘行号一律 `grep -n` 现取，非报告自陈）

| 文件 | 形态 | 关键锚点（现号） |
|---|---|---|
| `src-tauri/src/commands/stock_workflow/monitor_seam_service.rs` | 新建 715 行（res115 自陈 718，差 3 行是收尾格式收敛；本表一律取盘上号） | `HostStockMonitor :72`；纯函数 `is_allowed_alert_type :98` / `status_view :106` / `validate_t0_config :130` / `validate_aggregator_config :156` / `price_alert_view :190`；`impl StockMonitorProvider` 九方法 `:212` 起 |
| `src-tauri/src/commands/stock_workflow/quote_watcher_service.rs` | 新建 336 行 | `HostQuoteWatcher :51`；`dedup_preserve_order :65` / `plan_watch_batch :82` / `sorted_codes :94`；第四跳 `dispatch_quote_change :105`；`impl QuoteWatcherProvider` `:126` 起 |
| `src-tauri/src/init/services.rs` | +69/−5 | 装配点取回 `event.dispatch` **同一个** `Arc` 给 emitter（`get_event_dispatcher()` 在 `:5052`）；`set_aggregator` `:5071`；`stock.monitor` 就地注册（`HostStockMonitor::new :5181`、`register_stock_monitor :5187`）；行情回调第四跳（取总线 `:5228`、`dispatch_quote_change` 调用点 `:5244`）；watcher 改 `Arc` 构造 `:5273` + `quote.watcher` 注册（`HostQuoteWatcher::new :5283`、`register_quote_watcher :5287`） |
| `src-tauri/src/init/monitor_emitter.rs` | +89/−4 | `plugin_event_bus :59`、登记表 `plugin_kind_of :81`、发射体 `dispatch_domain_event :96`、`emit` 内第四跳分支 `:122`、新增单测 `only_alert_and_portfolio_events_cross_into_the_plugin_bus :387` |
| `src-tauri/src/app_state.rs` | +8/−1 | `quote_watcher: OnceLock<Arc<RealTimeQuoteWatcher>> :263`（越界批准，见 58.2-③） |
| `src-tauri/src/commands/stock_workflow/mod.rs` | +7 | 两条 `pub(crate) mod`（零 `#[tauri::command]` ⇒ `register_commands.rs` 生成物不受本批影响） |
| `axagent-core/crates/harness/src/stock_monitor.rs` | +35 | `STOCK_PRICE_ALERT_KIND :578`、`STOCK_PORTFOLIO_SIGNAL_KIND :583`、前缀形态锁单测 `:879` |
| `axagent-core/crates/harness/src/quote_watcher.rs` | +32 | `STOCK_QUOTE_CHANGE_KIND :160`、前缀/互异单测 `:260` |
| `axagent-quant/crates/analysis-engine/src/monitor.rs` | +30/−13 | 删 `RealtimeMonitor::subscribe` / `serialize_configs` / `deserialize_configs`（三处零引用，删前检索留证在 res115 §1.4）；`POLL_INTERVAL_RANGE :20`、新增 `ALERT_COOLDOWN_MIN_SECS :29`、守卫 `:366`、区间形态锁单测 `runtime_var_ranges_are_pinned :1230` |
| `axagent-infra/crates/dao/src/repo/price_alert.rs` | +21 | `find_by_id :88`、`delete_by_id :99`（返 `rows_affected`，因为 `RemoveAlertOutcome.db_deleted` 要区分「删成」与「查无此行」，而「查无」按 `jobs.store get` 口径是**正常结果**） |
| `src-tauri/src/commands/stock_workflow/workflow_run_service.rs` | 1 行 | `:720` `sort_by(|a,b| b.cmp(a))` → `sort_by_key(Reverse)`：这是 #90 落地时我造成的既存 clippy 红（`clippy::unnecessary_sort_by`），本批主 crate `-D warnings` 的唯一阻塞项，同批修掉并在本条留账 |

注册口现号（§55.5-N8 报的 `:1044/:1052/:1062/:1070` 已被 #107 再推）：`register_stock_monitor :1078` · `get_stock_monitor :1086` · `register_quote_watcher :1096` · `get_quote_watcher :1104`；`ServiceDefinition::stock_monitor() :164` / `quote_watcher() :178` 未漂。

### 58.2 对 res115 §④ 的逐条裁定（17 条，一条不并）

**① 契约文档「非法值 ⇒ `SEAM_CALL_INVALID_ARGS`」与分派层现状不符（res115 §4.1-1）——判执行者对，收口点是我欠的。**
`worker.rs` 那 12 个分支把 provider 的一切 `Err` 一律落成 `PLUGIN_CALL_FAILED`，`SEAM_CALL_INVALID_ARGS` 只由 `deserialize_arg` 在反序列化失败时产出。执行者把越界建成 `AxAgentError::Validation`（与 `workflow.run` 同族）是**当前唯一不越权的做法**。裁定：**不改契约文档的措辞**（那是承诺，不是笔误），由 **#112** 在分派层补 `Validation ⇒ SEAM_CALL_INVALID_ARGS` 的四行映射兑现它；映射加在分派层而**不是** provider 侧，因为 provider 回 `AxAgentError`、专用码只有分派层能发（先例 `workflow_run_error`）。**在 #112 落地前，插件侧文档不得写「越界能拿到 INVALID_ARGS」。
   ⚠ **#112 的覆盖面不止 stock.monitor**：契约里同形的承诺还有 `knowledge_vector.rs` 中 `index_text` 的文档（现 `:280`）那句「集合不存在 ⇒ 分派层映射 `SEAM_CALL_INVALID_ARGS`」（§56.2-2 的配套口径）。⇒ 那条映射是**两条接缝共用**的 `AxAgentError::Validation ⇒ 专用码` 函数，一次落地；#108 的 provider 必须同样用 `Validation` 抛「集合不存在」，否则映射无从命中。**

**② `status` 的 `assembled` 恒 `true`（§4.1-2）——接受纯函数解法。**
裁定 J8「装配成功后就地注册」与「provider 能返回 `assembled=false`」结构上互斥：本 provider 持非 `Option` 的 `Arc`，「未装配」在接缝上的**真实表现是注册表取不到 provider ⇒ `SEAM_PROVIDER_UNAVAILABLE`**。执行者没有为了凑那一格把字段改回 `Option`（正是 J8 否掉的形态），而是把九键映射抽成 `status_view(assembled, …)` 并给 `assembled=false` 写单测 —— 契约形态完整、运行语义诚实。**不许**后续批次以「让 `assembled` 能用」为由再引 `Option`。

**③ 两处越出任务书文件清单（§1.3 / §4.5-18）——两处都批准。**
- `app_state.rs` 那一行：`RealTimeQuoteWatcher` 不是 `Clone`，`OnceLock<T>` 拿不出第二个 owning 句柄 ⇒ 不改这行就只有「provider 另造一份集合」（两份「谁被监听」的真相，违反单源）或「provider 持 `AppHandle` 每次 `try_state`」（provider 单测全部写不出来）。消费方四处调用点按方法名调用，经 deref 自动落内层 ⇒ 一字未改，符合「禁改 `stock_analysis.rs`」。
- dao 两个函数：命令层不许直连 SQL（`check-layer-discipline.mjs` 规则 1 对新文件基线 0），而 dao 原先没有按主键的查/删。这正是 #114 想要的**正确方向**（下沉 dao），不是越界。

**④ clamp 单一源：我撤回对裁定 N4 的过度要求（§4.2-6）。**
执行者拒绝「借 `AGGREGATOR_WINDOW_SECS (1,3600)` 当轮询间隔权威」是对的：harness 那组管「跨股聚合窗口 / 组合信号去重」，引擎那组管「轮询节拍 / 单条告警冷却」，**四件不同的事，数值相同纯属巧合**，合并会把两处判定假性绑死（调聚合窗口即误伤轮询）。落地形态就此定案：聚合器四区间 provider 只引用 harness `AGGREGATOR_*`（真同源）；引擎两个数字用具名常量 + 单测 `runtime_var_ranges_are_pinned` 钉住。**不加** `MONITOR_POLL_INTERVAL_SECS` / `MONITOR_ALERT_COOLDOWN_SECS` 到 harness（那会超出「只追加 kind 常量」，且造出第三份名义同源、实则不同义的常量）。

**⑤ 行数与锚点订正照单收下（§4.1-3/4/5）：`trading_engine_service.rs` 268 行（非 256）；`capability_registry` 四锚现号见 58.1 末；res104 §G15/§I2 的 `:5154` / `:5224` 复核为真。**

**⑥ `alert_tx` 现在是没有接收端的 `Sender`（§4.3-8）——本批不退役，已登记 #115。**
删 `subscribe` 后 `:627` 的 `send` 恒 `Err`、被 `let _ =` 吞；留着不影响正确性（告警真相走 `event_emitter` + 前端 IPC + DB + 通知四跳），但它是「写进一条没人听的频道」，与阶段 D/G 退役的那批同形。属**行为面**改动，超出「删三处零引用死代码」的授权 ⇒ 执行者不动是对的。

**⑦ K1 三档读数已收口（§4.4-9）：`2022 / 324 / 238` 已在镜像表与 `PLAN:247` 落账（提交 `a0521d583`），`--repin-code` 顺带把 `worker.rs` 三个锚点 +237 重锚。**

**⑧ 分层门禁既存红（§4.4-10）——判为我的债，登记在 #114，不折进本批。**
`trading_engine_service.rs` 2 处、`workflow_run_service.rs` 1 处 `use sea_orm::{…}` 在 HEAD 就红（两文件 `git status` 当时干净）。本批两个新文件 0 命中。**修法**：把 `trades` / `price_alerts` 的查询下沉 dao，而不是把违规加进基线（基线是棘轮，只减不增）。

**⑨ harness 测试目标被 `knowledge_vector.rs` 挡住（§4.4-11）——已解，且欠的取证要补。**
那条 `error: invalid format string`（`{hits[0]:?}`）是 #107 交付里的真错，我已修并随 `71e6f88a4` 提交。⇒ 执行者因它没跑成的 A3/A4（`cargo test -p axagent-harness --lib stock_monitor` / `quote_watcher`，钉住三条 kind 常量与前缀形态锁）**现在必须复跑**，否则 58.1 表里那两条单测只是「写过」而不是「跑过」。

**⑩ `memory_removed` 口径采纳执行者的判法（§4.5-13），并作为插件侧文档的**唯一**措辞**：语义是「引擎内存里确有这一条、且被摘掉」（先 `list_configs` 判存在，再 `remove_config_by_id`（引擎 `monitor.rs:193`）），**不是**「调用没报错」。⇒ 删一条已触发 / 落不成配置的行会是 `dbDeleted=true, memoryRemoved=false`，与 `wired=false` 同族，都是合法读数。命令层 `delete_price_alert` 返 `()`，没有对应读数 ⇒ 这条口径**只在接缝上成立**。

**⑪ 空 `stock_code` / `stock_name`（§4.5-14）——裁定加严，且这是一处尚未落地的代码改动。**
执行者与命令层 `create_price_alert` 同口径「都不拒」，本批我没拦。改判理由：命令层是**人在面板上点**（空股票名当场看得见），接缝是**插件自动化写**（无人复核，会往告警表灌空名行，而面板按股票名展示 ⇒ 删都找不到）。代价是「面板能建、插件不能建」的分叉，**接受** —— 接缝是收紧面，不是放宽面。落点：`monitor_seam_service.rs` 的 `upsert_alert` 校验段 + 一条反面单测。

**⑫ `is_finite()` 拒 NaN / ±INF 保留（§4.5-15）**：契约「阈值越界 ⇒ 拒」不区分是哪种越界；`RangeInclusive::contains` 对 NaN 恒 `false` 是**fail-closed**，但阈值判的是 `<= 0`（对 NaN 恒 `false` = fail-open），所以这层显式判不可省。与 AGENTS「浮点判据套用 lint 建议不是纯等价改写」那条实证同源。

**⑬ `watch` 的 `applied` = 截断后、去重后「本次真的提交给引擎的数量」（§4.5-16）——采纳可复算的那种读法**，不含「集合因此涨了几只」（已在集合里的代码 `watch` 是覆盖优先级，仍计入 `applied`）。契约与插件文档都按这句写，别用「实际生效的数量」这种两可措辞。

**⑭ 第四跳发射点留在 `init/services.rs` 的行情回调（§4.5-17）——接受这处越界**：不改 `RealTimeQuoteWatcher`（不在可改面）就没有别的发射位置；执行者已把逻辑封成 `quote_watcher_service.rs::dispatch_quote_change`，services.rs 只留一行调用 `:5244` ⇒ 主权与形态都在 provider 侧，注册段之外只多一个调用点，可审。

**⑮ 未做项里三条是**流程纪律**，本批背书，不改判**：① 阶段边界不跑全量门禁（AGENTS 口径）；② 只对新改文件逐个 `rustfmt --edition 2024`，**不**对 `mod.rs` 跑（实测 rustfmt 顺 `mod` 声明会把子模块整文件重写，包括他人正在改的 `serenity.rs` —— 这是「单文件 rustfmt 顺 mod 树」的已知坑）；③ 前端 / i18n / `src/types` 零改动（接缝消费方是插件，不是前端）。

**⑯ `register_commands.rs` 此刻的 diff（`plugin_list_pages` 一行）是并行批次产物，本批提交不含它。**

**⑰ `TRAJECTORY_/KNOWLEDGE_` 与 #107 相关的四条待裁（res117 §④ 6/7/9/11）已在 §58 之外单独裁定，见 58.3。**

### 58.3 #107 契约批四条待裁的收口（一并落账，避免 #108/#109 各猜一次）

1. **补 `KNOWLEDGE_SNIPPET_MAX_CHARS = 200`**（res117 §④-6 的缺口）：与 trajectory 同值、理由同源（帧预算 = `top_k × snippet 上限`）。已落 `knowledge_vector.rs` + `const _: () = assert!(… > 0)` + 形态锁，随 `71e6f88a4` 之后的提交入库 ⇒ #108 provider 侧裁剪必须引用它，不许随手取 500/2000。
2. `chunk_count` / `dimensions` 保持 **`u32`**（不收宽成 `usize`）：>4G 块数 / >4G 维度不成立，收窄安全；#108 provider 里那一次 `as u32` 是预期的，不是缺陷。
3. **args 键形态就此钉死**：#109 分派层用 `deserialize_arg::<VectorSearchRequest>(&args, "request", seam)`（嵌一层，照 §55.5-N2）；`index_text` 用 `VectorIndexRequest` 整体反序列化。插件侧文档不得写 snake_case 入参。
4. `required_permission_for` 在 #109 **只加一条**：`("knowledge.vector", "index_text") => Some(PluginPermission::Write)`；`trajectory.store` 首批只读 ⇒ 不加条目（落 `_ => None`）。res105 批 3 表里那行 `delete_document` **不存在**，别照抄（§56.1-Q2 已砍）。
5. `score` 方向（L2 距离，越小越相似）契约层钉不住 ⇒ **#108 宿主 provider 的单测必须锁比较方向**，与 `indexing.rs` 的「过滤 → 排序 → 截断」口径对齐。
6. **op 名 ≠ 方法名就此定案**（res117 §④-12 待裁）：knowledge.vector 的第四个 op 用 **`get_collection_info`**，方法名保持 `collection_info()` 不动。理由：本仓入站面的既有形态就是「op 名带 `get_`/`list_` 前缀、trait 方法名是名词」（`get_t0_config`→`t0_config`、`list_watched`→`watched_codes`），改方法名要重启 K1 记账与已交付的契约文档，收益只有「少一条映射」；而映射成本已在 §55.5 的两条先例里被吸收。⇒ #109 按 op 名匹配分支，`#108` 不动 trait。
7. **入站 args 是双形态，不是笔误**：外层键 **snake_case**（盘上先例 `"request"` / `"alert_id"` / `"codes"` / `"priority"` / `"replace"`），嵌进去的 DTO 字段 **camelCase**（禁区 13 管的是 DTO 与 Tauri invoke 参数，帧协议外层键另有这套先例）。⇒ #108/#109 与插件侧文档一律照此写，不许「统一成 camelCase」把既有 12 条分支的入参键形改一遍。
8. **方法签名以盘上为准**（#109 执行者纠正我任务书里的写错处）：knowledge 四个方法**按值**收 Request（`search(VectorSearchRequest)` 等，`:239/:249/:266`），`collection_info(&str) :276` 是唯一带引用的；trajectory 四个是 `list(Option<usize>) :212` / `get(&str) :221` / `query(TrajectoryQueryView) :231` / `count() :240`。⇒ 我在派单里写的 `&VectorSearchRequest` 与「`query` 参数名 request」都按盘上改；分派层因此是 `deserialize_arg::<DTO>(&args, "request", seam)` 后**移动**进去，不存在借用问题。

### 58.4 本批验收状态（诚实口径：绿的是**跑过的**，不是写过的）

| 项 | 状态 |
|---|---|
| `cargo clippy -p axagent --all-targets -- -D warnings`（主 crate，含 58.1 全部改动 + `:720` 那处修复） | **EXIT=0**，`.tmp/res123-E1-axagent-clippy.log`（322 B / `Compiling axagent v3.0.0` + `Checking axagent-runtime` + `Finished dev … in 16m 14s` + `E1_EXIT=0`） |
| 被测版本归因 | 该轮起点 07:52:00 > 末次源写入 07:51:51 ⇒ 读到的是修好的版本 |
| 同命令复跑 | `.tmp/res103-E1b-axagent-clippy-recheck.log`（327 B / `Finished dev … in 17m 33s` / `RECHECK_EXIT=0`）。⚠ 这一轮**不是 no-op**：我在起跑后约 36 s（08:11:19）改了 `monitor.rs:22` 的一句**假注释**（把单测名写成 `runtime_var_ranges_are_the_single_source_numbers`，盘上真名 `runtime_var_ranges_are_pinned`）⇒ 级联重编。那一处另起定点取证：`.tmp/res126-F1-engine-clippy.log` 首行 `Checking axagent-analysis-engine` + `Finished … in 40.61s` + `F1_EXIT=0` ⇒ 现号内容确有 lint 级证，不是「改完就说绿」 |
| harness `--all-targets` clippy | 因 58.2-⑨ 那条真错曾被挡；`71e6f88a4` 修后需复跑（执行者只交回 `--lib` 口径绿证） |
| `cargo test -p axagent-harness --lib stock_monitor` / `quote_watcher` | **未跑**（同上原因）⇒ #103 合入后补跑，三条 kind 常量目前只有 lib 级 lint 证、没有测试级证 |
| 全量三件套（`fmt --all --check` / `clippy --workspace --all-targets --all-features` / `test --workspace`） | 未跑（AGENTS：全部阶段完成后一次）；已知两处会红：`market_data.rs` 4 处 rustfmt 漂移（#113）、分层门禁 3 处既存（#114） |
| 前端 / Playwright | 未跑（本批零 TS/JSON 改动） |
### 58.5 #108 宿主批执行侧推翻我的两条（记入账，别在下一批重犯）

1. **`od -c | grep -c '\r'` 在这台机器上是假判据**（实测：对已提交的干净文件 `monitor_seam_service.rs` 报 1106 个「CR」）。真因不是 SMB 也不是 CRLF，是 **grep 的 BRE 把 `\r` 当字母 `r`** —— `od -c` 的输出里满是 `r` 这个词字母（`Arc` / `axagent` / 每个路径）。⇒ 数 CR 的唯一可信口径是 `tr -cd '\r' | wc -c`（先把非 CR 字节全删掉再数）。这条订正**覆盖** §58.1 与既往派单里「行尾用 `od -c` 判」那句的读法：`od -c` 只能**肉眼看**某一行的字节，不能拿去 `grep` 计数。
2. **`search` 的 `min_similarity` 不在 provider 复筛**（判据 4 的「过滤 → 排序 → 截断」措辞是我写的，Literal 执行会造出第二把尺子）：宿主检索层已经在 `combined_score >= min_score` 上筛过、并产出 `score = 1 - combined_score`，provider 再按距离筛一道 = 复刻 `rag.rs` 里那段「两把尺子」的原案缺陷。⇒ 改判为**透传 + 方向锁**：单测钉「距离小的排前」+「分值逐位不变（`to_bits` 比，避开浮点 `==`）」。契约侧「L2 越小越相似」那句仍然成立（`1 - combined_score` 就是越小越相似），但**方向由宿主单点决定**，接缝不重算。
3. 装配点选择：注册段落在 `start_background_services`（`:97`/`:99`）而非我给的 `start_realtime_monitor` 先例段 —— 判据是「必须早于 `start_plugins`，否则插件初始化时接缝还没注册」。这条比先例更重要，照它。待 #108 报告交回时验：`grep -n "start_plugins\|register_knowledge_vector\|register_trajectory_store" src-tauri/src/init/*.rs` 的先后关系。
4. 本批**没有独立复核环节**（专家团在本启动路径下两条委派通道都不可用：CLI delegate 被 hook 改写零返回、内置建会话被宿主以递归限制拒），执行者如实上报而非静默自审 ⇒ 复核由我做；下一派单要么写明「leader 复核」，要么先修委派通道（并入 #106 同族的本机缺陷账）。

## 59. `knowledge.vector` + `trajectory.store` 分派批与宿主批交付（#109 / #108）＋ 我对两批「判据不足」的逐条裁定

> 日期：2026-11-03｜提交：`b63193674`（#109，`worker.rs` 单文件 +1132/−0）· `5ad4158ba`（#108，4 文件 +1301/−0）｜报告：`.tmp/res109-dispatch-report.md`（24 KB）· `.tmp/res108-host-report.md`（32 KB）· 团内自审 `.tmp/res109-review.md`
> 下表所有行号是我**自己** `grep -n` 复扫出来的（不是抄执行者自陈）；验收行是我逐条打开日志读到的原文。

### 59.1 我独立复扫的取证（执行者的交付表要能被这叫停）

| 判据 | 我复扫的原文 |
|---|---|
| #109 改动面 | `git diff --numstat` = `1132  0  worker.rs`（纯增量）；`grep -c '"knowledge.vector" if op =='` = **4**、`"trajectory.store" if op ==` = **4**；`required_permission_for` 新增条目在 **`:459`**（`("knowledge.vector", "index_text") => Some(PluginPermission::Write)`），`_ => None` 兜底未动；`SUPPORTED_REMOTE_SEAMS`（`:139`）不在任何 hunk 内（首个 hunk 从 `:438` 起） |
| #108 改动面 | `commands/mod.rs +8/−0`、`init/services.rs +35/−0`；`HostTrajectoryStore::new :4801` + `register_trajectory_store :4805`、`HostKnowledgeVector::new :4917` + `register_knowledge_vector :4924` |
| 注册时机（判据 9 的前提） | 同一调用序列里 `start_trajectory_cleanup(state) :97`、`start_index_job_service(app, state) :99` ⇒ **早于** `start_plugins(state) :100`。⇒ 「插件初始化时接缝已在注册表里」成立 |
| 分层纪律 | `grep -nE "^(use\|pub use) (sea_orm\|axagent_entities)"` 对两个新 provider = **零命中**；`AxAgentError::Validation` 出现 21 / 14 次；`grep -cE "std::sync::(Mutex\|RwLock)"` = **0 / 0**（两文件零锁） |
| 行尾 | `tr -cd '\r' \| wc -c` 对 `knowledge_vector_service.rs` / `trajectory_store_service.rs` / `worker.rs` 均 **CR=0**（这是 §58.5-1 换的可信口径，我自己复跑了一遍） |
| #109 验收 | `.tmp/res109-A1-clippy.log`：`Checking axagent-plugins` + `Finished … in 1m 11s` + `CLIPPY_EXIT=0`；`.tmp/res109-A2-test-worker.log`：`running 108 tests` + `test result: ok. 108 passed; 0 failed; … 111 filtered out` |
| #108 验收 | A2b：全文 `grep -cE "^error"` = **0**、`Checking axagent-harness` + `Compiling axagent v3.0.0` + `Finished … in 20m 51s`；A3b `test result: ok. 15 passed`；A4b `test result: ok. 10 passed` |
| **#103 欠账收口** | B1b `Checking axagent-harness v3.0.0 …` + `Finished … in 1m 15s`（`touch` 逼出的真跑，md5 前后同值）；B2 `11 passed` 含 `stock_monitor::tests::stock_event_kind_prefix_matches_category ... ok`；B3 `7 passed` 含 `quote_watcher::tests::stock_quote_change_kind_prefix_matches_category ... ok` ⇒ §58.4 那两行「未跑」就地作废，三条 kind 常量现在是 **lint 级 + 测试级**双证 |

### 59.2 逐条裁定（两批共 20 条 + 必答 2 条）

**A. #109 的七条**

1. **任务书写的取 provider 形态是错的，执行者纠正为 `self.registry.get_*()`**（盘上入站分派面零处用全局注册表；`HostPeer` 持 `Arc<CapabilityRegistry>` 就是为了替身注入）。⇒ **接受**，且我这份任务书如果照盘上写会直接废掉「空注册表 ⇒ `SEAM_PROVIDER_UNAVAILABLE`」和全部替身用例。订正入 59.3-①。
2. 任务书给的「最近三次提交」过期（HEAD 被推进）。⇒ 接受；它同时证明 `worker.rs` 自 `47955bb19` 起未被任何后续提交碰过 ⇒ 基线归因不成立即不成立，没被它拿去充数。
3. **契约承诺 `SEAM_CALL_INVALID_ARGS` 的位置是两处不是一处**（`knowledge_vector.rs:85` 正文超 64 KB、`index_text` 文档现 `:275` 集合不存在）。⇒ 裁定 **(a)**：由 **#112** 一次性收口，映射函数是 `AxAgentError::Validation ⇒ SEAM_CALL_INVALID_ARGS` 单点，覆盖面 = `stock.monitor`（白名单/阈值 clamp）+ `knowledge.vector`（这两格）。**不改契约文档措辞**（那是承诺不是笔误），#112 落地前插件侧文档不得写「越界拿得到 INVALID_ARGS」。
4. 任务书引用的锚点逐条复核为真。⇒ 收下，不重述。
5. **「先跑改前基线」与「只允许一个 cargo 会话」在本机不可同时满足**，它选了「写完再排队」，并给了四段间接取证（改后 108 实测 + 本批 12 实测 + 上一批终态日志 + `git log -- worker.rs` 证明版本同一）。⇒ **接受**（换我也不会为了一个数字去做 UNC 上的 A/B 摆渡）；同时把它点出的 PLAN §55.6「基线 86 passed」**就地订正**为实测链条 88 → 96 → 108。
6. **团内 res 编号与任务号不同源，且已经撞名**（`.tmp/res109-migration-plan.md` 是 #35 的调研产物，同前缀不同归属）。⇒ 接受批评，59.4 挂对照表；后续派单一律「任务号前缀 + 用途后缀」（如 `res108-host-*`、`res117-host-*`）。
7. 判据不足项（生产装配未验、帧体积无实测、「provider 部分实现」的可用性）⇒ 逐条确认成立，其中**帧体积**登记为 #124（跨批，需要真机数据）。

**B. #108 的十三条 + 必答两条**

8. **必答 A：宿主没有「按集合 + 文档」幂等重建入口**（四个候选逐条否掉：整集合 `clear_embeddings` 越权、`delete_document_embeddings` 是 §56.1-Q2 点名的半截删除原语、`index_knowledge_document` 收不了裸文本、`upsert_document_embeddings` 要调用方持 rowid 与向量本体 ⇒ 违反 K4）。⇒ 幂等语义定为**按块覆盖**（块 id `{document_id}_{chunk_index}` + `upsert_embeddings`），已知不完全 = 新正文变短时尾部旧块残留 ⇒ 登记 **#125**（需要「删块 + 删向量」同成的宿主原语），本批在模块头声明、不藏。
9. **它给 `index_text` 加了一条任务书没有的前置闸**：文档行必须已存在且 `knowledge_base_id` 与请求的 `collection_id` 一致。⇒ **接受**。这正是 §56.1-Q2「不开 delete」的同一判据的正面用法：放陌生 id 进去只会造出「检索得到、面板删不掉」的孤儿块。**代价如实说**：插件不能凭接缝建文档行 ⇒ 「先建文档再入库」的两步得走宿主命令面，这条要写进插件侧文档，不许让插件以为 `index_text` 能凭空造文档。
10. **必答 B：`query` 八个过滤字段宿主支持七个，不支持「半边时间窗」**（原形 `time_range` 是闭区间元组）。它落成 `Err(Validation)`，而契约 `trajectory_store.rs` 的 `since_ms`/`until_ms` 文档（原 `:145-146`，现 `:159-160`）承诺「None 端点按开区间处理」——该承诺已由 #118 改为「只给一端 ⇒ `Err(Validation)`」，故本句今后只成立为裁定记录。⇒ 裁定 **改契约措辞**（与原形落不了地的承诺比，诚实的契约更有价值），「钳位」旧措辞实际只有 `:119` 一处（`:127`/`:147` 写的是「合法区间」）⇒ 一并收进 #118 扩展后的 harness 契约批，两处「合法区间」也显式改写成拒收。同一批它还拦了 `untilMs < sinceMs`（恒零命中，属「阈值尺度写反」同型）⇒ **接受**。
11. **判据 4 的字面写法它偏离了**（详见 59.3-②）。⇒ **接受替代方案**（阈值原样透传 + 不重排 + `to_bits` 逐位比），并登记 **#120**：在 `axagent-search` 侧给 `min_score` 补**带数据**的检索级单测，那才是「筛得动」的证。
12. **`[0,1]` 量纲数字（两处生产点：`knowledge_vector_service.rs:168`（原 `:167`，#118 r2 漂 +1）、`trajectory_store_service.rs:138` 零漂移）算不算第二套权威**？⇒ 我的判法：**现在不算缺陷、但算债务** —— 它是量纲定义（相关度/评分的定义域）而非可调区间，且宿主侧无可引用的常量；但两处两个文件已经是事实上的重复。⇒ 随 **#118** 上收 harness（`KNOWLEDGE_SIMILARITY_RANGE` / `TRAJECTORY_RATIO_RANGE`），provider 改引；越界的后果是**静默空集**而不是静默夹取，所以「拒收」的判法保留。
13. **`collection_info` 的两种 `dimensions: None` 宿主不可区分**（`get_collection_dimensions` 只返 `Option<usize>`，`table_exists` 私有）。它按「有维度即存在」判 ⇒ 「存在但未绑模型」被读成 `exists=false`（保守假阴，不会诱导读，但不诚实）。⇒ **接受现状 + 登记 #122**（search crate 出一个集合存在性读口）。不许为凑这一格在接缝侧造猜测。
14. **`list`/`query` 的读放大**：`≤200` 限的是**返回**条数，宿主是「全表取出后内存截断」且每条轨迹再做两次子查询取 steps/rewards ⇒ 十万行库上一次 `list(Some(200))` 就是十万行扫描 + 200 次额外查询。⇒ **登记 #121**，优先级高于一般重构：这条是**接缝开放给插件后才会被踩到**的成本面，判据 1 的上界挡不住宿主自己的读放大。
15. **A2 的第一条红是它自己写的单测**（`assert!(!(nan < 0.0 \|\| nan > 1.0))` 触 `manual_range_contains`），而且它识别出「按 clippy 建议机械改成 `contains` 会让 NaN 那一支失去区分力」⇒ 改成断言 `Validation` 文案含 "NaN"。⇒ **接受**，这是「测试数据须自证区分力」的正确形态，与我此前记的「lint 的机械建议会改语义」同族，只是这次发生在测试里。
16. **A1（`cargo check --all-targets`）没在修好的那一版上重跑**，它如实标成「第一轮基线」并交我定。⇒ **裁定不补**：A2b（clippy `--all-targets -D warnings`，退 0）与 A3b/A4b（test profile 全量 codegen + 真跑）在覆盖面与严格度上都**包含** `check --all-targets`，再占一次 15–21 分钟主 crate 锁只换一个更弱的读数。
17. **`touch` 逼真跑那次动了禁区文件的 mtime**（`stock_monitor.rs`，md5 前后同值、`git status` 该文件不出现）。⇒ 接受（这是任务书「强制真跑要 touch」那条纪律的执行），且我复扫时该文件确实未出现在工作树 diff 里。
18. **它没跑 `--repin-code`（要写 `docs/**`/`scripts/**`，在禁改面）也没落 K1 账**，并明确「这一格是 leader 的活，不是我漏了」。⇒ 已做：K1 三档按实测重钉（扫描面文件数与「有外部实现者的 trait 数」各 +2，`--dump-readings` 复现），`--repin-code --apply` 回写 3 处 evidence 行号，第五道门 **GATE_EXIT=0**。
19. **`od -c` 验 LF 是假判据**（§58.5-1）与 **res 编号撞车**（§59.2-6）⇒ 两条都已在 59.4 落表；`res108-*` 前缀已被 §55 那批用过，它改用自己的产物前缀 `res108-host-*` / `res117-host-*`，未覆盖任何既有文件 ⇒ 接受。
20. **`commands/mod.rs` 的并发风险**（两批都要加 `pub mod` 会同文件打架）：实测本批只有 #108 动过它（+8），#109 零接触 ⇒ 风险没兑现，但派单口径保留（同一文件只派一路）。

### 59.3 我推翻自己任务书的三条（写在这里，不许下批照抄我的错）

1. **取 provider 的形态**：我写「`axagent_harness::get_capability_registry().get_*()`」，盘上入站分派面**一律** `self.registry.get_*()`（改前实测 `let Some(provider) = self.registry.get_` 命中 69 处，全局表只出现在 `use` 清单与门面注册侧）。⇒ 照我写的做会让替身注入与空注册表用例全部失效。
2. **判据 4 的「provider 里再按距离筛一道」**：宿主已把过滤上移到检索层（`rag.rs` 在 `truncate` 之前按 `combined_score >= min_score` 筛，视图里的 `score` 是它的补数）。⇒ 复筛 = 拿相关度阈值去比距离，正是 `rag.rs` 两处注释记录的「两把尺子」已修缺陷。**单源判据优先级高于我的措辞**。
3. **「先跑改前基线」**：与本机单 cargo 会话锁冲突。⇒ 派单以后写成「基线用上一批终态日志 + 本批增量实测」的间接四段式，别要求 A/B 摆渡。

### 59.4 任务号 ↔ `.tmp` 前缀 ↔ 提交号 对照（本会话，执行者建议的表，就地挂上）

| 任务 | 报告 | 日志前缀 | 提交 |
|---|---|---|---|
| #104 分派 | `res114-dispatch-report.md` | `res114-*` | `47955bb19` |
| #103 宿主 | `res115-host-report.md` | `res115-*` / `res123-*` / `res103-E1b-*` | `1741aa2f4` |
| #107 契约 | `res117-contract-report.md` | `res117-*` | `71e6f88a4` + 格式收尾 `8124df14b` |
| #109 分派 | `res109-dispatch-report.md`、团内自审 `res109-review.md` | `res109-A*/fmt-*`（⚠ `res109-migration-plan.md` 是 **#35** 的，同前缀不同归属） | `b63193674` |
| #108 宿主 | `res108-host-report.md`、同步件 `res108-leader-brief.md` | `res108-host-*` / `res117-host-*`（⚠ `res108-*.log` 20 多个是 §55 那批的） | `5ad4158ba` |

⇒ 口径：`.tmp` 里的历史前缀不改名（改了断门禁与文档引用），**新派单一律用「任务号 + 用途后缀」**，并在报告第一节写清自己占用了哪些前缀。

### 59.5 本轮新登记的跨批项

**#118**（harness 契约批合并口径，四条一起收 + `[0,1]` 量纲上收，两 crate 定点复验）· **#119**（团内委派通道在本机不可用：CLI delegate 被 hook 改写零返回、内置建会话被递归限制拒 ⇒ 要么修通道要么在派单里明写「leader 复核」）· **#120**（`axagent-search` 侧给 `min_score` 补带数据的检索级单测）· **#121**（`get_trajectories` / `query_trajectories` 补 SQL 侧 `LIMIT` + 消 N+1 子查询，**读放大**）· **#122**（集合存在性独立读口，让 `collection_info` 区分两种 `None`）· **#123**（`index_text` 变短留尾块，需「删块 + 删向量」同成原语）· **#124**（帧体积最坏值实测，本批无真机数据）。

### 59.6 交付状态（诚实口径）

- 本轮把最后两条（`knowledge.vector` / `trajectory.store`）补齐三段 ⇒ 这八条入站查询接缝（`market.quote` / `jobs.store` / `workflow.run` / `trading.engine` / `stock.monitor` / `quote.watcher` / `knowledge.vector` / `trajectory.store`）的「契约 / 宿主 / 分派」**全部到位**，插件侧可调用面从六条涨到八条。
- 已实测：#109 的 108 passed、#108 的 15 + 10 passed、#103 欠的 harness 双证。**未实测**：真实 embedding 链、FTS5 检索链、真库有数据时的读回形状、端到端「插件调接缝拿数据」（要跑起来的宿主 + 凭证）。⇒ 「编译过 + 单测绿」不等于「端到端通」，这四格在 #120/#121/#124 之前不许对外说成已验证。
### 59.7 我自己造的一处险情（如实记账，别学）

落 K1 账时用 `JSON.stringify(j, null, 2)` 重写镜像表 ⇒ 文件从 **976 行涨到 1926 行**：原表把 `planPinned` / `alsoAt` 这类对象**内联**写在一行，`stringify` 把它们全展开成多行。后果不是「难看」而是**按行引用当场腐烂** —— `PLAN:2626` 就明写「必须同批改 `scripts/capability-register.json:956` 行的 `seamId`/`faces`/`evidence`/`planAnchorText`」，展开后那一行不再在 `:956`。⇒ 已回退到旧格式（`git show HEAD~1:` 取原文覆盖）再改用**逐字符串替换**，改动全部「一行换一行」⇒ 净 diff `11/11`，`:956` 仍在原位（已复扫），锚点回写改用 `--repin-code --apply`（它本就是外科式改法）。判据与 §58.5-1 同族：**重写整份序列化 ≠ 最小改动**，凡有人按行引用这张表，就只能做等行数替换。

## 60. #112 / #116 / #118(r2+r3) 交付账与我对执行者的裁定（含「按行引用」政策的收口）

> 日期：2026-11-03｜提交链：`7d48768c0`（#112 映射）→ `60d4638ba`（#118 r2 + #116）→ `efe7c399e`（#118 r3 追加）｜报告：`.tmp/res112-mapping-report.md`、`.tmp/res118-116-harness-report.md`（§⑥ 为追加轮）

### 60.1 三条接缝的错误码语义就此定形

`worker.rs:2854` 新增 `seam_provider_error`（结构逐字照 `workflow_run_error`）：

| provider 回的变体 | 帧码 | 覆盖面 |
|---|---|---|
| `AxAgentError::Validation` | **`SEAM_CALL_INVALID_ARGS`** | `stock.monitor` 9 + `quote.watcher` 3 + `knowledge.vector` 4 + `trajectory.store` 4 = **20 处**分支 |
| 其余（含 trait 默认 `Provider("…未实现…")`） | `PLUGIN_CALL_FAILED` + `to_string()` | 同一批 20 处的另一半 |
| 入参反序列化失败 | `SEAM_CALL_INVALID_ARGS`（`deserialize_arg` 自己产出，与本函数无关） | 全部入站接缝 |

数量对账我自己复扫过：改前 `Err(e) => …PLUGIN_CALL_FAILED` 形态 **79** 处 ⇒ 改后 **59** 处（79 − 20 = 59，零溢出），`=> seam_provider_error(e),` **20** 处。`trading.engine` / `jobs.store` / `market.quote` **不改**（三份契约里 `SEAM_CALL_INVALID_ARGS` 零命中 ⇒ 无同形承诺）。

### 60.2 我对四条「契约 vs 实现」互斥的裁定（#118 r3 的 E1–E5 已落）

1. `stock_monitor.rs:56`「provider 返回 `Err` 一律 `PLUGIN_CALL_FAILED`」与 `:54`「白名单/越界 ⇒ `SEAM_CALL_INVALID_ARGS`」**覆盖同一事件给两个码** ⇒ `:56` 改「**非 `Validation` 变体**」。执行者**只改这一句**，`:54`/`:55` 与类型、默认实现一字未动。
2. `trajectory_store.rs:69-70` 与 `knowledge_vector.rs:61-62` 同族反话 ⇒ 都改成「只有 `Validation` 升专用码」。**E4 那处（`knowledge_vector.rs:61-62`）不在我的清单里**，是执行者用 `grep -rnE "兜底成|一律 .PLUGIN_CALL_FAILED." harness/src/*.rs` 扫全族后自加的，并在报告里写明「若判越界，回退 = 恢复这 2 行原文，零风险」⇒ **我接受**：同一文件内两条契约陈述互斥时，留旧那句就是留假话；而且它把回退路径一起交了，这才是「越界自纠」的正确形态。
3. `workflow_run.rs:21`「分派层**将来**映射为 `WORKFLOW_RUN_REJECTED`」⇒ 现在时（早已落地，#112 一字未动那条接缝）。
4. **`quote_watcher.rs` 不补承诺**（我已裁两次）：宿主 provider 今天零处抛 `Validation`（`grep -c` = 0）。为了让契约「有承诺」去造一个校验是反向动机 —— 契约的每句承诺都必须有一个产生它的代码路径。

### 60.3 我推翻自己的三条 + 执行者推翻我的一次性纠正

| 我的原写法 | 盘上事实 | 出处 |
|---|---|---|
| 「`knowledge_vector.rs:85` 与 `:127` 都写着钳位」 | `grep -n "钳位"` 改前**只命中一行**（`:119`，现 `:133`）；`:85` 讲的是正文超限（本来就拒收），`:141`/`:161` 写的是「合法区间」 | #118 ④-1 |
| 派单没给工作根 | 本仓**无根 `Cargo.toml`**（八个一级 workspace），从仓根跑三条 ⇒ 三条全 `EXIT=101`、日志各 87 字节 | #112 ④-1 |
| 「市场报价契约 `market_quote.rs`」 | 实名为 **`market_data.rs`**（执行者另确认这个错名没进过它任何注释） | #112 ④-9 |
| 「NaN 阈值越界 ⇒ 由映射收口」 | `serde_json` **不承载 NaN**（`json!` 把 `f64::NAN` 写成 `null`）⇒ NaN 在反序列化阶段就落 `invalid_args`；结论仍是 `SEAM_CALL_INVALID_ARGS`，但**不是**由 `seam_provider_error` 收口，宿主侧 `is_finite()` 在帧协议上不可达 | #112 ④-7，已从我给 #112 的 §1 表格里摘掉那一格 |

### 60.4 「按行引用」政策收口（本会话内第三次腐烂，改判我自己的写法）

实测三轮漂移：#109 让 `worker.rs` 漂 +1132、#112 再漂 +682（镜像表 3 条 evidence 各 +21，已 `--repin-code --apply` 回写，`GATE_EXIT=0`）；#118 r2 让两个契约文件漂 +14。⇒ **PLAN 与镜像表今后对我的正文引用一律改用符号形态**（「`index_text` 的文档」「`seam_provider_error`」），行号只保留给**编译期可复算**的锚点（`anchorText` 整行键）。本轮我自己在 PLAN 里引用的 harness 行号已就地改指现号，并在 §60.5 记下原号 ⇒ 不再制造「读到一个错号就去改代码」的诱导。

### 60.5 我改掉的 PLAN 内部引用（原号 → 现号，同批改指）

| PLAN 位置 | 原引 | 现引 |
|---|---|---|
| §58.2-① | `knowledge_vector.rs:266` | 该文件 `index_text` 的文档，现 `:280` |
| §59.2-3 / §59.2-10 | `knowledge_vector.rs:85` / `:261` | `:85`（未漂）/ `:275` |
| §59.2-10 | `trajectory_store.rs:145-146`（承诺 `None` 端点按开区间） | `:159-160`，**且该承诺已被 #118 改掉**（⇒ 本句今后只成立为裁定记录） |
| §59.2-12 | `knowledge_vector_service.rs:167` / `trajectory_store_service.rs:138` | `:168` / `:138`（后者零漂移） |

### 60.6 验收（逐条我自己开日志读过，不是转述）

| 批 | 取证 |
|---|---|
| #112 | `res112-clippy-r4.log`：`Blocking waiting for file lock`（真排队）+ `Checking axagent-plugins` + `Finished … 16m43s`，`EXIT=0`；`res112-test-r4.log`：**`test result: ok. 112 passed`**（基线 108 + 4，`111 filtered out` 与上批逐字同）；`res112-check-r4.log` `EXIT=0`；被测 sha `6d7d15884e8fd28c` 与四个依赖契约文件的**起/止哈希逐字相同** |
| #118 r2 | harness clippy `Checking axagent-harness` + 1m22s；harness `11 / 12 passed`；主 crate `Compiling axagent v3.0.0` + clippy `EXIT=0`（23m04s，与墙钟吻合）；`monitor_seam_service` **18 passed**（含两条新格）、`knowledge_vector` 15、`trajectory_store` 10；五个文件 md5 在四个检查点相同 ⇒ 交付版本 == 被测版本 |
| #118 r3 | 四条全 `EXIT=0`（harness clippy 1m11s 含 `Checking`、11/12 passed、主 crate `Compiling` + 15 passed）；**零行漂移**由 `wc -l` 七文件前后逐字相同自证 ⇒ 我不用再重锚一次 |
| **我补的那格** | r3 未跑 `--lib trajectory_store`（它改了那处用户可见文案）。我判定无覆盖缺口后又**亲自跑了一遍**：`.tmp/res118pm-r4-leader-trajectory.log` ⇒ **`test result: ok. 10 passed; 0 failed` + `EXIT=0`** |

### 60.7 一条必须留在账上的操作纪律（本轮新踩）

#112 的执行者为终止自己被作废的轮次跑了 **`taskkill //F //IM cargo.exe`（按镜像名杀，自报终止 3 个）**，连带杀掉并行批正在排队的编译。⇒ 派单模板加一条硬规：**本机八个根共用一把 target 锁，禁止按镜像名杀进程**；要终止只按自己记录的 PID 树。受影响轮次的处理已当场做完（#118 判掉三条 0 字节假绿并整轮重跑，结果见 60.6）。

### 60.8 未覆盖项（不许读成「已验证」）

- 真实 embedding 链、FTS5 检索链、真库有数据时的读回形状、端到端「插件调接缝拿到数据」——四格仍未走（依赖凭证与真机，见 #120/#121/#124）。
- 主 crate 全量 `clippy --all-targets` 未在 r3 之后重跑（r3 只动注释与两处文案，且 `cargo test -p axagent --lib knowledge_vector` 连带做了全 crate `Compiling`）。⇒ 这条并入 **#19** 的终验一次收。

## 61. #100（B1 首刀）实数订正与 J1–J10 裁定（2026-11-03，报告 `.tmp/res100-b1-cut.md`）

### 61.1 实数：「−13 处 / −4 文件」两个数都不成立

- **盘上实测 24 处 / 7 文件**（快照 `docs/audits/AUDIT-l1-blockers-2026-11-03-1421.md`；当日读数：待改写 142 / 24 文件 + H 桶 10 / 4 文件 = 合计 152 / 28）。
- 13 处的来源：PLAN 只点了 5 个文件。漏点 **11 处** = `finance.rs:1178/1189`（`KLine` 2）+ `validate-backfill.rs:533`（`as_of`）+ `services.rs` 6 + `state.rs` 2。
- 「−4 文件」与「13 处」不配对：13 处落在 5 个文件里，而 `finance.rs`（改后仍剩 `KLine` 2 行）、`validate-backfill.rs`（剩 7 行）都清不掉；且 `batch-rerun.rs:44` 依赖 `as_of` 运行层，严格说连它也清不掉 ⇒ 13 处最多清 3 个文件。
- 逐族加总（桶内）：`task_cancel` 3 + `calendar` 6 + `as_of` 7 + `indicators` 1 + `KLine` 2 + `candlestick_pattern` 1 + `divergence` 1 + `types::NewsItem` 1 + `NewsArchiveSink` 1 + `analysis_engine::risk` 1 = **24** ✓。
- 同批订正 `PLAN:2513` 的「真实 151」：审计脚本现在同时剔 `//` 与 `///`，`background_tasks.rs` 报 3 处 @8/350/916，`:266` 不在列 ⇒ **152 是干净的 152**。

### 61.2 J1–J10 逐条裁定

| # | 事项 | 裁定 | 理由 |
|---|---|---|---|
| J1 | `services.rs` 6 + `state.rs` 2 这 8 处归 B1 还是留给 B2/B6 | **B1 收其中 5 处**（`services.rs` 的 calendar 1 + `as_of` 常量 2、`state.rs` calendar 2）；其余 3 处（`services.rs` 的 `as_of` 运行层）随 T2 留档 | §57.2 许诺给 B2/B6 的是「逐行落名」的 **H 桶**锚点，而这些是 **foreign** 行；且 B2/B6 尚未写任何锚点 ⇒ 不存在腐烂。若留给 B2/B6，垫片会成为该模块唯一路径，等于把「已上收」变成两套写法 |
| J2 | 推翻 `harness/src/as_of.rs:7`「harness 不含运行时状态」，把 `as_of` 运行层（5 处）也上收 | **否（T2 延后）** | 需 harness 引入 `task_local`/全局 `Mutex` 运行态、`serial_test` dev-dep，且 `as_of` 的跨 `spawn` 传播语义属运行时策略；收益 5 行、K1 零增量。要做得单独立项并先写清「为什么当初这样切」 |
| J3 | `risk` 的 6 个 snake_case 结果结构体进 harness | **否（首刀不收）** | 补 `rename_all = "camelCase"` = 改工具出参 JSON key = **语义变更**，与 B1「零语义变更」冲突；原样进则往 foundation 塞 6 个 snake_case 反例（禁区 13）。只换 1 行读数，性价比最差。另立「工具出参 key 政策」批 |
| J4 | `costs.rs`（回测成本）/ `factors.rs`（因子版本）算公用件吗 | **否，判股票域本体** | `DEFAULT_ROUND_TRIP_COST_PCT` 是 A 股印花税/佣金默认值，`FACTOR_VERSION` 是因子定义版本 —— 随 #35 外迁，harness 不携带域默认参数 |
| J5 | `hit_rate_backtest` 为 `RecoPick` 拆层 | **否，归 B7** | 需再钻 `recommender/types` 依赖链，首刀不扩面 |
| J6 | 其余 `axagent_analysis_engine::*` 哪些该随 #35 外迁 | **B1 不逐模块裁，只记机械判据**：能进 harness = 零 `axagent_*` 依赖 **且** 签名无 `&DatabaseConnection`/entity；不满足者一律随 #35 | 域切分是产品决定，不是分层决定 |
| J7 | `market_sim_service.rs` 2 处 | **否，不属 B1** | `market-sim` 按 `AGENTS.md` 是 consumer，wiring 引用合规；归市场仿真批（未排期） |
| J8 | harness 新模块命名 | **`trading_calendar.rs`**（PLAN 既定）+ `task_cancel.rs` / `candlestick_pattern.rs` / `divergence.rs` / `news_archive.rs`；七个候选名实测零撞名 | 与既有 `holding_period.rs` / `indicators.rs` 区分；垫片保留原路径，消费方零改动 |
| J9 | `NewsArchiveSink` 上收是否冲撞数据主权 | **合规，可收** | 端口只含 DTO、不碰 DB；实现仍在 `src-tauri/src/init/news_archive_sink.rs`；`astock-data` 的持有关系字段不动 |
| J10 | `tools/opc.rs` 41 + `audit.rs` 4 + `capability_pack_defs.rs` 1 + `astock_data.rs` 3 = 49 处 | **确认归 #32，B1 一行不碰** | PLAN §54.1 R7 已裁定；B1 的 24 行里没有一行落在这 4 个文件 |

### 61.3 首刀范围：T0 + T1 + T1a = **18 处**

| 档 | 内容 | 处数 |
|---|---|---|
| T0（路径改指，零新文件） | `finance.rs` 的 `indicators::sma` 1 + `KLine` 2；`news_archive_sink.rs` 的 `types::NewsItem` 1 | 4 |
| T1（新建 5 个 harness 模块 + 垫片） | `task_cancel` 3；`calendar` 6；`candlestick_pattern` 1；`divergence` 1；`NewsArchiveSink` 1 | 12 |
| T1a（`LIVE_GENERATION` 并入 `harness/src/as_of.rs`） | `services.rs` 2 处 | 2 |
| **合计** | | **18** |

新建：`harness/src/task_cancel.rs` / `trading_calendar.rs` / `candlestick_pattern.rs` / `divergence.rs` / `news_archive.rs`（5 个）+ `harness/src/lib.rs` 注册 5 行（与 #113/#114 无同文件冲突，实测）。
垫片：`astock-data/src/{task_cancel,calendar,candlestick_pattern,divergence,as_of}.rs` + `astock-data/src/lib.rs`（`NewsArchiveSink` 改 `pub use`）。
消费方（7 文件 / 18 行，快照号）：`finance.rs` 5（`:9` 除外）、`background_tasks.rs` 3、`news_archive_sink.rs` 2、`batch-rerun.rs` 2、`validate-backfill.rs` 1、`services.rs` 3、`state.rs` 2。

### 61.4 预期读数（每个数跟算式）

| 指标 | 现读数 | 首刀后 | 算式 |
|---|---|---|---|
| 待改写（H 除外） | 142 处 / 24 文件 | **124 处 / 22 文件** | 142 − 18 = 124；清空 `background_tasks.rs` 与 `news_archive_sink.rs` ⇒ 24 − 2 = 22（`finance.rs` 因 `risk` 留下 1 行，不清零） |
| H 桶 | 10 / 4 | 10 / 4 | 名单零动 |
| **合计** | **152 / 28** | **134 / 26** | 124 + 10；22 + 4 |
| K1 `.rs` 文件数 | 2025（登记 2024，已红） | **2030** | +5 个新 harness 模块；垫片保留 ⇒ 不降 |
| K1 `pub trait` 唯一名 | 324 | **324** | `NewsArchiveSink` 换文件不换名 |
| K1 + 外部实现者 | 240 | **240** | 该 trait 原本已在扫描面（`astock-data` 是 `.rs`）且实现者在 `src-tauri` ⇒ 声明位置移动不改变交集 |

⚠ **代价必须写进交付说明**：`.rs` 净升 5，读数净降 18 —— 「拆了就变少」是假的。

⚠ **本表首行那个预测值与工作流旧读数同形**：`workflowAuditDomainCommands` 的 `planOld` 就是同一个数（旧工作流命令数，见 §0.12.4 的漂移列举）。本表里那个数是 B1 落地后的**预测值**，与它无关。第五道门靠该锚点的 `alsoAt` 区段（`scripts/capability-register.json` 的 `planOld[0].alsoAt`，覆盖本节表格）放行，**不是改数过门**；该区段有 `mustContain` 三键，行号漂了就红。

### 61.5 一条已证伪的顾虑：harness **不需要**改 tokio feature

`task_local!` 在 tokio 1.53 里确实位于 `cfg_rt!` 块内（本机 `tokio-1.53.1/src/task/mod.rs:277-331`，`mod task_local` 在 `:307`），但 **harness 已经拿得到 `rt`**：`axagent-core/Cargo.toml` 的 workspace 声明是 `tokio = { version = "1", features = ["full"] }`，而成员里的 `features = ["sync", "time"]` 是**追加**、不是覆盖。

实测（`cargo tree -p axagent-harness -e features -i tokio`，只读解析，未编译）启用集含 `full` / `rt` / `rt-multi-thread` / `macros` / `tokio-macros` ⇒ `tokio::task_local!` 与 `#[tokio::test]` 都直接可用（harness 现有 35 处 `#[tokio::test]` 即旁证）。

**裁定：本批禁止改 `axagent-core/crates/harness/Cargo.toml`。** 若实现时编不过，先按「盘上与 `cargo tree` 矛盾」上报，不许自行加 feature 掩盖。

### 61.6 实现批硬约束（与 §60.7 同源）

1. **R1**：harness 内零 `axagent_*::` 字面量（`check-layer-discipline.mjs` 规则 3 的正则不排除 `axagent_harness::`）。搬 `candlestick_pattern.rs` 时 `use crate::KLine` 必须改 `crate::market_data::KLine`；搬 `divergence.rs` 的 `use crate::indicators::rsi` 必须改 `crate::indicators::rsi_wilder as rsi`。验收 = `--list` 里 harness 段零命中，不只看退出码。
2. **R2**：`trading_calendar.rs` 模块头必须点名两套 `is_trading_day` 语义 —— `&str` 薄封装（本地节假日表，`analysis-engine/src/trading.rs`）vs `async fn(&self, &str)` 网络查询（`stock_data_service.rs` 的 trait），**禁止合并**。
3. **R4**：不得在 #113/#114 入库前跑 cargo；不得改 `harness/src/market_data.rs`（#113 正在改）。cargo 一律后台、日志独占 `.tmp/res100b1-*`；**禁止按镜像名杀进程**。
4. 垫片一律 `pub use axagent_harness::<mod>::*;`（原文件保留不删，沿 #81 先例）；`astock-data/src/lib.rs` 只删 `NewsArchiveSink` 定义、`:430` 字段与 `with_news_archive_sink` 不动。
5. 账（`scripts/capability-register.json` / `PLAN:513` / `PLAN:247`）由**我**在实现落地后同批重锚，执行者不许改 `scripts/**` 与 `docs/**`。

## 62. #113 + #114 交付账（2026-11-03，报告 `.tmp/res113-114-debt-report.md`）

> 执行：软件开发专家团 · 项目经理会话（委派通道三次被 hook 改写 ⇒ 自做自证，**无成员复核**）。我随后独立复扫了下表全部数字。

### 62.1 交付面（`git diff --numstat` 由我复取）

| 文件 | +/− | 关键锚点（`grep -n`） |
|---|---|---|
| `axagent-core/crates/harness/src/market_data.rs` | 4 / 7 | 4 个 fmt hunk（`:15` 导入字母序 + 三条「未实现」文案合一）；**文案逐字未动** |
| `axagent-infra/crates/dao/src/repo.rs` | 1 / 0 | `pub mod trade;`（字母序位） |
| `axagent-infra/crates/dao/src/repo/trade.rs` | 新增 55 行 / 2799 B | `exists_same_key :35`（六字段等值 + `.one()`，无 `order_by`/`limit`） |
| `src-tauri/.../trading_engine_service.rs` | 16 / 13 | 调用点 `:247`；错误映射文案逐字不变 |
| `src-tauri/.../workflow_run_service.rs` | 5 / 1 | 只换 `DatabaseConnection` 导入源（`axagent_dao` 根 `pub use sea_orm::DatabaseConnection`，类型不变） |

提交：`f89b8994e`（代码）· `76bf55cfd`（PLAN §61 + K1 重锚）· `1dafdb354`（#127 引用修复）。

### 62.2 我独立复扫的结果（不转述执行者）

| 判据 | 命令 | 结果 |
|---|---|---|
| #113 格式 | `rustfmt --edition 2024 --check crates/harness/src/market_data.rs` | **EXIT=0 / 日志 0 字节** |
| 其余四文件格式 | 逐 workspace `rustfmt --check` | 两个双文件调用均 **EXIT=0 / 0 字节** |
| 分层门禁 | `node scripts/check-layer-discipline.mjs` | **EXIT=0**；`commands-no-direct-db 625（基线 628）`、`harness-fanout-zero 0` |
| 语义等价 | 读 `trade.rs` + 两处 diff | 六列、六值、`.one()`、错误文案均与改前逐字一致；`TradeExecutionRequest` 六字段类型为 `String`/`f64`/`i32` ⇒ `&String→&str` 无隐式转换风险 |
| 行尾 | `tr -cd '\r' \| wc -c` | 五文件全 0 |
| 第五道门 | `check-capability-register.mjs` | **EXIT=0**（`k1RsFileCount = 2025` 已重锚） |
| 单源事实门 | `check-single-source-facts.mjs` | **BROKEN/越界/空行/控制流 0**（原唯一一条 = `quote_watcher_service.rs` 引用 `workflow_run_service.rs:249` 空行，已按符号改指） |

### 62.3 执行者纠正我的四条（全部成立）

1. `dao/src/repo/mod.rs` **不存在**：dao 是 edition 2024 的 `repo.rs` + 同名目录；注册只能改 `repo.rs` 一行。**我追加授权**（原派单「`repo/` 下对应文件」写窄了）。
2. `workflow_run_service.rs` 的 1 处命中是 `use sea_orm::DatabaseConnection;`，**不是查询**，无处可沉。执行者改从 `axagent_dao` 再导出点导入 —— 我复核 `dao/src/lib.rs:12` 确为 `pub use sea_orm::DatabaseConnection;` ⇒ **接受**，但登记为「规则 1 的字面量口径 ≠ 真实直连」的样本：该文件本就不发 SQL。
3. `market_data.rs:12` 的字母序方向：rustfmt 要 `ConceptBlocks` 在前（`l` < `o`），我派单说反了。**接受**。
4. `--update-baseline` 不影响退出码（`gateExit(over, …)`，`below` 只打印）⇒ 不改基线也能拿到 EXIT=0。**接受**，但见 62.4。

### 62.4 我撤回了执行者没做、我自己试过的基线下调

我跑了一次 `--update-baseline`，diff 除了 `commands-no-direct-db 628→625`，还把**并行批未提交**的 `serenity.rs 4→3`、`reflection.rs 35→33` 一起吃进基线 ⇒ 若那批回退就会造成假红。**已用 `git show HEAD:…` 还原**（未用 checkout），基线维持 628，下调债登记为 **#128**，等所有在飞批次提交后重做。

### 62.5 未覆盖项（不许读成「已验证」）

- `exists_same_key` **没有单测**：行为等价只有编译期与逐字对照证据，无运行时证据。
- 未跑 `clippy -p axagent-dao`（它只作为依赖被 `Checking`，自身 lint 未过 `-D warnings`）。
- 未跑全量 `cargo fmt --all --check` ⇒ **#19 是否整体解锁无结论**，只证明 `market_data.rs` 一个文件。
- 主 crate clippy 那 18m58s 是**联合状态**的绿：同批并行修改的 `stock_analysis.rs` / `reco_pool.rs` / `recommender/mod.rs` / `register_commands.rs` 同在 `-p axagent` 编译面，不能归因为本批隔离绿。

## 63. B1 首刀交付账（2026-11-03；`#100`；提交 `8d802bc6f`；执行者报告 `.tmp/res100-b1-impl-report.md` 17 676 B）

### 63.1 交付面（leader 逐项独立复核，非转述）

- **新增 5 个 harness 模块**（untracked → 本批提交）：`task_cancel.rs` 103 行、`trading_calendar.rs` 969 行、`candlestick_pattern.rs` 558 行、`divergence.rs` 494 行、`news_archive.rs` 39 行；`harness/src/lib.rs:148-152` 五条 `pub mod` 注册。
- **常量并入**：`harness/src/as_of.rs:195` `pub const LIVE_GENERATION: Option<&str> = None;`；`astock-data/src/as_of.rs:513` 改 `pub use`（运行层原样留在本文件）。
- **5 个垫片**：`astock-data/src/{task_cancel,calendar,candlestick_pattern,divergence}.rs` 变为 3–5 行 `pub use axagent_harness::<mod>::*;`；`astock-data/src/lib.rs:85` 改 `pub use axagent_harness::news_archive::NewsArchiveSink;`（`:430` 字段与 `with_news_archive_sink` 未动）。
- **18 处消费方改指 harness**：`finance.rs:13/1178/1180/1189/1192`、`background_tasks.rs:11/350/916`、`news_archive_sink.rs:13/14`、`axagent-batch-rerun.rs:418/718`、`axagent-validate-backfill.rs:1301`、`services.rs:2368/2632/2682`、`state.rs:1828/1830`（逐条改前/改后见执行者报告 §2，行号 `grep -n` 现取）。
- **零扩面**：`git status` 中本批 15 个修改文件 + 5 个新文件，与报告 numstat 逐字一致；**`Cargo.toml` 零改动**（R3 已作废）；`as_of` 运行层 5 处与 `analysis_engine::risk` 均未动（§61.2 J2/J3）；`src/**` 前端与 i18n 未动；全部改动文件 CR 计数 0。

### 63.2 读数兑现 §61.4 的预测（逐数复核）

| 度量 | B1 前 | B1 后 | 预测（§61.4） | 对账 |
|---|---|---|---|---|
| 待改写（H 桶除外） | 142 处 / 24 文件 | **124 处 / 22 文件** | 124 / 22 | 兑现 |
| H 桶 | 10 处 / 4 文件 | **10 处 / 4 文件** | 不变 | 兑现 |
| 合计（与旧口径可比） | 152 处 / 28 文件 | **134 处 / 26 文件** | 134 / 26 | 兑现 |
| K1 `.rs` 文件数 | 2025 | **2030** | 2030 | 兑现 |
| K1 `pub trait` 唯一名 | 324 | **324** | 324 | 兑现（`NewsArchiveSink` 只换文件） |

- 逐文件 delta（B1 前后两份审计快照对比，`AUDIT-l1-blockers-2026-11-03.md` → `…-1557.md`）：`finance.rs` 6→1（−5）、`services.rs` 28→25（−3）、`state.rs` 10→8（−2）、`axagent-batch-rerun.rs` 3→1（−2）、`axagent-validate-backfill.rs` 8→7（−1）、**`background_tasks.rs` 3→0**、**`init/news_archive_sink.rs` 2→0**；合计 **−18 处 / −2 文件** ✓。
- 审计快照 `…-1557.md` 与 `…-1632.md` 字节完全相同（14 396 B），两次重跑读数一致。

### 63.3 验证门（全部实跑，日志在仓根 `.tmp/res100b1-*`）

| 门 | EXIT | 关键原文 |
|---|---:|---|
| `cargo check -p axagent-harness --all-targets` | 0 | `Checking axagent-harness v3.0.0` · `Finished … 1m 21s` |
| `cargo test -p axagent-harness --lib` | 0 | `test result: ok. 927 passed; 0 failed; 1 ignored`（搬迁测试确在 harness 内跑：`trading_calendar::tests` 20 / `task_cancel::tests` 3 / `candlestick_pattern::tests` 16 / `divergence::tests` 8） |
| `cargo clippy -p axagent-harness --all-targets -- -D warnings` | 0 | `Finished … 1m 12s` |
| `cargo check -p axagent-astock-data -p axagent-analysis-engine --all-targets` | 0 | 两 crate 均 `Checking` · `Finished … 3m 00s` |
| `cargo check -p axagent-tools --all-targets` | 0 | `Checking axagent-tools v3.0.0` · `Finished … 3m 14s` |
| `cargo check -p axagent --all-targets` | 0 | `Compiling axagent v3.0.0 (src-tauri)` · `Finished … 18m 15s` |
| `audit-l1-blockers` / `--check-allowlist` | 0 / 0 | 124/22 + 10/4 = 134/26；白名单 PASS |
| `check-layer-discipline` | 0 | `harness-fanout-zero 0 处（基线 0）`；无超基线 |
| `check-capability-register` | **0**（重锚后） | 三条红全部消解，见 63.4 |
| `check-single-source-facts`（`.rs` 面） | 3（软判据） | BROKEN/越界/空行/控制流 **0**；12 条「疑似腐烂」全为既存整段引用，无一条由本批引入 |

⚠ 主 crate 那 18m15s 仍是**联合状态**的绿：并行批的 `stock_analysis.rs` / `reco_pool.rs` / `recommender/mod.rs` / `register_commands.rs` 同在 `-p axagent` 编译面 ⇒ 不能归因为 B1 隔离绿。

### 63.4 本批重锚（leader 做，执行者按纪律未碰）

1. `l1ForeignStock`：value 152→**134**、value2 28→**26**、planNumber `"152"`→`"134"`、`planPinned` PLAN:513 mustContain 同步；PLAN:513/523/558/568/976/996/1134 的 live 陈述同步（523 的重算：前 5 文件 88 处 = 41+25+8+7+7，其后 4 文件各 4，其余 13 文件各 ≤3）。
2. `k1RsFileCount`：2025→**2030**，PLAN:247 括号数字与 `planPinned` 同步。
3. **执行者 §6.3 指出的第三条漂移已确认并重锚**：删 `NewsArchiveSink` 定义使 `astock-data/src/lib.rs` 后移 −26 行 ⇒ `market.vendor` 证据 `register_vendor` **863→837**、`find_vendor` **1672→1646**；两条 evidence 补 `anchorText`（整行逐字），`planAnchorText`（PLAN:276）同步。
4. `workflowAuditDomainCommands` 的 `planOld` 124 新增第 2 个 `alsoAt`（PLAN:523–523），理由：该行的 124 是 B1 后的反向阻塞待改写数，与工作流命令数的历史读数**同形不同物**；自证 token = `134 处（含 H 桶 10 处 / 4 文件）` + `待改写 124 处` + `B1 首刀前口径为 152 / 28`。
5. 门禁结果：`check-capability-register` **EXIT=0**（21 项 hard 判据全一致，能力面 Σ=23）。

### 63.5 附带修掉的 docs 引用（B1 直接造成）

- `docs/plans/PLAN-four-horizon-workflow-alignment.md:6876-6878`：原文断言「`NewsArchiveSink` 由 astock-data 自己声明」已被 B1 推翻 ⇒ 就地改指 `axagent-core/crates/harness/src/news_archive.rs:19`，并注明 `lib.rs:85` 现为 `pub use` 垫片、注入点现号 `lib.rs:922`（原记 `:914` 在 HEAD 已是 `:948`，属**既存**漂移，一并纠正）；「端口不进实现层、实现留 wiring」的原则不变。
- **未改**：`docs/audits/AUDIT-capability-inventory-2026-11-01.md:228` 仍写 `NewsArchiveSink`（`crates/astock-data/src/lib.rs:91`）—— 审计是**时点快照**，按纪律不改历史报告；现指映射为 `harness/src/news_archive.rs:19`。

### 63.6 未做 / 未覆盖（不许读成「已验证」）

- **无成员独立复核、无第二人复扫**：委派通道当日被 hook 改写（`team-task delegate` 零返回），执行者按 §119 提示停止重试并自做自证；本报告 §63.1–63.3 的数字是 leader 本人在盘上重取，但**评审视角仍只有一人**。
- 未跑 `check-single-source-facts --dump-doc-refs` 的完整人判（扫描仍在后台跑；本批只确认无 BROKEN/ABORT）。
- 未跑主 crate 测试（派单只要求 `cargo check`）；未跑全量 `cargo fmt --all --check` / 全量 clippy（#19 属终验批）。
- `#128` 的棘轮下调（`commands-no-direct-db 628→625`）仍未做，等全部在飞批次提交后一次性重算；本批未跑 `--update-baseline`。
- §0.10 的历史分解段（PLAN:1167 的 152 分桶、§50.6 的 2446 行、§54.1 的 2512 行、§57 的 2661 行三行表）保留 B1 前口径原样，属**历史交付账**，不再回写。

## 64. 两条延期裁定收口（2026-11-03；`#125` 工具出参 key 政策 / `#126` as_of 运行层）

### 64.1 `#125`：工具出参 key 政策 = **逐工具冻结，不按分层改名**

**裁定**：工具出参 JSON 是**LLM 可见契约**，不是 IPC/frontend DTO。政策三条：

1. **按工具现状冻结**：既有工具的 key 原样保留（`snake_case`、短名、中文值都算契约的一部分），不许以「统一命名」「分层上收」为由批量改名。
2. **新增 key 跟随该工具既有风格**，不强制 camelCase；跨进程（Tauri 命令 / 接缝 op）的 DTO 才受禁区 13 的 camelCase 约束。
3. **上收的前置判据 = JSON key 零变化**：领域结果结构体只有在「逐字段同名、同型、同序」时才可整删本地类型；字段超集必须留适配层（本文件 `finance.rs:140-149` 的 `PegR` 就是先例，`VarResult` 整删是另一侧的实证）。

**对 J3 的处置**：`analysis_engine::risk` 的 6 个结果结构体**不因分层而上收 harness**。理由：它们的字段名（`var_pct` / `mean_return` / `cvar_pct` …）直接就是 `calc_var` / `calc_sharpe` 等工具的输出 key（`finance.rs:117-118`、`:1443`）；上收时若补 `rename_all = "camelCase"` 即改 LLM 可见 JSON，属语义变更；原样上收则把 6 个 snake_case 结构体塞进 foundation，与禁区 13 的示范相反。它们留在 `analysis-engine`（implementor），由 `tools`（hybrid）按既有依赖方向使用 —— 分层上没有违规，只有「读数不动」这一条代价，接受。

### 64.2 `#126`：as_of 运行层**不进 harness**，维持 `harness/src/as_of.rs:8` 的边界声明

**裁定**：不上收，`#126` 关闭（不做）。三条依据：

1. **定位**：harness 是「纯契约 + 纯 DTO」的 foundation；`harness/src/as_of.rs:8` 明写「不含运行时状态（task_local、全局 Mutex 等实现在 astock-data 中）」。这条边界是当前 foundation 零 `axagent-*` 依赖之外的第二条硬约束。
2. **形态**：运行层不是几个函数，而是一套**进程级状态机** —— `tokio::task_local! AS_OF`（`astock-data/src/as_of.rs:36`）+ `static GLOBAL_AS_OF: OnceLock<Mutex<AsOfScopeStack>>` 作用域栈（`:82`，含 `next_token`）+ 降级日志双缓冲（`:280-291`）。把它搬进 foundation 会让 7 个 workspace 共享一个隐式可变单例，测试隔离、插件沙箱与「数据主权在宿主」三条都变难。
3. **归属**：它服务的对象是 vendor 调用（数据源实现层），正确归属是 implementor（`astock-data`）；#35 股票迁出时随域 crate 一起走，不由 harness 承接。

**残留的 5 处读数**（`batch-rerun.rs:44`、`validate-backfill.rs:533`、`services.rs:2283/5764/5787`）是宿主装配对域运行层的合法引用；要彻底消掉，方向是 `market.vendor` 接缝已声明的「`AS_OF` task-local 必须改显式入参」，不是搬进 harness。该改造归 B2/B7，不在本裁定内。

### 64.3 `#102`：`session.store` 是**宿主内部注入通道**，不对插件开放（半截状态改为显式契约）

**盘上事实**（本轮复核，纠正 R1 的行号）：

- 注册口现为 `capability_registry.rs:1186-1200`（R1 写的 `:1036` 已漂移），预置声明在 `:248-260`；注释自陈用途是「工具侧原先经 `OnceLock` + setter 各自注入同一实例，现统一从本接缝取回（缺口 G4）」。
- `get_session_state_store()` 的真实调用方是两个**宿主内**工具：`capability_load.rs:54`、`save_as_workflow.rs:53` —— 走的都是注册表直取，不是 worker 帧。
- `dispatch_seam`（`axagent-plugins/crates/plugins/src/worker.rs:586`）确实 **0 处** `session.store` 分支 ⇒ 插件发起 `call_seam` 会落到末尾 `SEAM_CALL_UNSUPPORTED`。

**裁定：不补分派支（不向插件暴露）**。依据：

1. **数据主权判据未定**，PLAN 既有口径是「`session.store` 提供面不远程化」（本文件 :309、:1091）；`PLAN-plugin-gap-closure.md` 明写「本期不做（数据主权判据未定）」。
2. 它的现存价值是 **G4 宿主内注入**（替换 OnceLock 双注入），已在生产使用；「插件调不到」不是缺陷，是边界。
3. 会话状态是跨会话 / 跨 agent 的宿主自有 KV（含 `NS_SKILL_LOADED` 等命名空间键），一旦对插件开放 `list_by_prefix` 就等于开放全量读取；要开放必须先有 §54.2 R1 第二段设计的「命名空间白名单 + DTO/`schema_version` 校验」，那是 `domain.store` 批的事。

**落地（2026-11-03 已完成，`#102`）**：① `worker.rs:162` 新增 `HOST_INTERNAL_SEAMS = ["session.store"]`，`:611` 在权限闸之后、取 provider 之前显式拒绝（`SEAM_CALL_UNSUPPORTED`，文案点名「宿主内部注入通道」）；② 单测 `worker.rs:11397` 锁住「即使插件声明 `write`，被拒的 `set` 也不落库」；③ 预置声明注释 `capability_registry.rs:254` 写明宿主内部。测试 `cargo test -p axagent-plugins session_store_is_host_internal` = 1 passed（223 filtered out）；`rustfmt --edition 2024` 两文件均 0。

### 64.4 `#120` / `#122` / `#124` 交付账（2026-11-03；提交 `979c77a66`）

**`#120` min_score 检索级单测**：`rag.rs:3249` 的 `min_similarity_really_filters_results_on_the_retrieval_path` 走宿主同一条 `search_with_filter` 链（真实 sqlite-vec L2 + 真实数据），固定 query embedding=`[0,0]`，四条向量距离 2/4/6/8 ⇒ 相关度 0.9/0.8/0.7/0.6；`bm25_weight=0` + `weighted` 融合让断言落在绝对标尺。四格：`None` 不过滤、0.55 全留、0.95 返回空（非错误）、0.0 不过滤、0.75 只留前两条。leader 独立复跑 **1 passed**（`.tmp/v120-min-score.log`）。

**`#122` 集合存在性与维度解耦**：根因不是契约缺字段 —— `CollectionInfoView` 本来就有 `exists` 与 `dimensions`，是宿主把 `exists` 算成了 `dimensions.is_some()`。最小改法（已落）：search 层新增 `VectorStore::collection_exists`（`vector_store.rs:534`，判据=注册表行 / vec0 表 / `_meta` 表任一存在），宿主 `collection_info`（`knowledge_vector_service.rs:419`）两把尺子分开读；**契约零改动**。测试覆盖：查无 / 正常集合 / 只留 `_meta` 的「存在但维度未知」/ 只有 vec0 表的残缺态 / 非法集合名返 `Validation`；独立复跑 **1 passed**（`.tmp/v122-exists.log`）。相邻既有缺陷另立 `#130`：`vector_store.rs:435` 的 `SELECT dimensions FROM pragma_table_info(...)` 列名不存在（`pragma_table_info` 只有 cid/name/type/notnull/dflt_value/pk）；**实测更糟** —— vec0 虚拟表连 `type` 也报空串（`name=embedding type=""`）⇒ 该回退路径**恒 `None`**，不是「改个列名」能修。`#130` 已改为读 `sqlite_master.sql` 里的 `float[N]`（`vector_store.rs:425` `registry_get_dimensions_from_schema` / `:453` `parse_vec_dimensions`），单测 `dimensions_fall_back_to_schema_when_registry_row_missing`（`vector_store.rs:2262`）**1 passed**（`.tmp/v130-schema-test.log`）。

**`#124` 读 op 最坏帧体积（实测，不再是乘法推演）**：口径 = `FrameResponse` serde 形态 + 4B 长度前缀，`MAX_FRAME_LEN = 64 MiB`。契约内最坏：knowledge `search` 50 条 × snippet 200 控制字符（JSON 转义最坏）≈ **69.5 KB**（0.10%）；trajectory `list` 200 条同口径 ≈ **477.5 KB**（0.71%）。模块头推演「200×200 中文 ≈120 KB」只算了 snippet，实测同口径 357.5 KB（≈2.98×），差额是每行固定字段（quality 五元组 / uuid / 时间戳 ≈1.1 KB/条），仍远低于上限。

**唯一可越限的是无上限字段**：`TrajectorySummaryView::topic` 无截断（`trajectory_store_service.rs:219`），按 200 条 + snippet 200 emoji + uuid 口径换算，**单条 topic 平均 > 334,156 B（≈326 KiB）即越过 64 MiB**。越限时 `write_frame` 在写任何字节前返回 `InvalidData`，写线程记 `Failed { wrote_any: false }` ⇒ 调用失败但**不会产生半帧**。

**裁定：不加截断**。理由：① topic 由模型生成，正常长度 <200 字符，334 KiB/条没有真实来源；② 截断会**静默改 LLM 可见内容**，违反「工具出参 key/内容按现状冻结」（§64.1）的同族原则；③ 越限路径本身就是 fail-closed（无半帧）。记为**已知边界**并入契约注释；若未来真出现越限，再按 §56 的契约演进流程加 `MAX_TOPIC_CHARS` 与兼容测试，不现在预防性加。

**未覆盖（如实）**：宿主测试 `collection_info_reports_present_collection_with_dimensions` 已随 `--all-targets` 编译通过，但本机测试 exe 启动报 `STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)`（需 `__TAURI_WORKSPACE__=true` 重建），运行时未验证 ⇒ 并入 `#130` 的验收。

### 64.5 `#121` 轨迹读口读放大（2026-11-03）

**病灶**：`get_session_trajectories` / `query_trajectories` 原实现先 `.all()` 拉回全部命中行（`query_trajectories` 的 `limit` 是内存 `take`），再对每条轨迹各发 2 条子查询（`get_trajectory_steps` / `get_trajectory_rewards`）⇒ 一次读 = **1 + 2N** 条 SQL，且 `LIMIT` 根本没下推。

**改法（`axagent-ai/crates/trajectory/src/storage.rs`）**：① `query_trajectories`（:402）把 `query.limit` 直接交给 SQL `.limit()`（`ORDER BY created_at DESC` 已定序，前 N 行与原内存截取同义；`Some(0)` 两边都为空）；② 新增 `attach_steps_and_rewards`（:446）/ `load_steps_and_rewards`（:477）：steps 与 rewards 各一条 `IN (...)` 批查，按 `trajectory_id` 分组后回填，整条读路径固定为 **3 条** SQL，步骤顺序仍由全局 `ORDER BY step_index` 保证；③ 读端映射抽成 `model_to_step` / `model_to_reward`，单查与批查共用同一份。

**验证**：`cargo test -p axagent-trajectory --lib storage::tests` = **11 passed**（`.tmp/v121-recheck.log`，leader 独立复跑）；新增三测 `query_limit_returns_newest_n_in_desc_order`（:2112，含 limit > 总数 / limit=0 边界）、`query_limit_applies_after_filters`（:2154，LIMIT 在过滤之后）、`batched_steps_and_rewards_match_single_row_queries`（:2188，逐项与单查结果比对）；`cargo clippy -p axagent-trajectory --all-targets -- -D warnings` EXIT=0（`.tmp/v121-clippy.log`）；`rustfmt --edition 2024 --check` EXIT=0。

**未做 / 新发现**：① 批量装配的 rewards 沿袭原查询「无 ORDER BY」的物理行序，若将来给 rewards 引入显式顺序需同改两处；② 发现既有读写不对称 —— 写端 `storage.rs:227` 用 `format!("{:?}", …)` 存 `TaskCompletion`，读端只认 `task_completion` ⇒ 所有 reward_type 读回都落到 `UserFeedback`（两条读路径彼此一致，测试掩盖了它），另立 `#132`（§64.6 已修）。

### 64.6 `#123` / `#133` / `#132` 交付账（2026-11-03）

**`#123`「变短留尾块」结论修正**：`rag::index` → `upsert_embeddings` 在**同一事务**内先按 `document_id` 删旧行（meta + vec0）再插入 ⇒ 同一文档重复入库是**整文档替换**，不存在尾块残留；`knowledge_vector_service.rs:65` 模块头原「已知不完全」的陈述与实现不符。已补回归测试 `reindexing_shorter_document_drops_tail_chunks`（`vector_store.rs:2269`，5 块 → 2 块后 meta COUNT=2）并改写模块头。接缝仍**不提供**独立 delete op（理由不变：插件 supplied 的 `document_id` 可指向宿主真实文档）。

**`#133` SQLite 写路径被 `md5()` 打死**：上述测试第一次运行即暴露 `no such function: md5` —— 内容级去重 SQL 无条件用 PG 内置函数，SQLite 上 `upsert_embeddings` 首块即失败。已按方言分流（`vector_store.rs:759`）：PG 保留 `md5(content)=md5($1)`（规避历史乱码行的 UTF8 校验），SQLite 改 `content = $1`（SQLite 不做 UTF-8 校验）。验证：`vector_store::tests` **6 passed**、`cargo clippy -p axagent-search --all-targets -- -D warnings` EXIT=0（`.tmp/v123-verify.log`）。

**`#132` reward_type 读写口径**：写端 `format!("{:?}")` 落 `TaskCompletion`，旧读端只匹配 3 个 snake_case 变体、其余 4 个全塌成 `UserFeedback`。改为读端按大写字母边界归一化后交给权威 `RewardType::try_from_str`（`storage.rs:1680` `reward_type_from_db`，不再在 storage 抄变体表）；新增 `reward_type_roundtrips_for_all_variants`（`storage.rs:2333`，8 变体逐一往返）。写端暂保留 Debug 形态以兼容存量行，统一格式留给迁移批。验证：`storage::tests` **12 passed**、clippy EXIT=0、rustfmt EXIT=0（`.tmp/v132-verify.log`）。

### 64.7 `#131` 第五道门 evidence 判据收紧（2026-11-03）

**缺陷**：`adjudicateEvidence` 与 `computeCodeRepin` 的「相符」只看「该行含 token」。`worker.rs` 位移后 `workflow.evolver` 区间止点（anchorText=`is_running`）落到同 token 的另一处调用（`set_llm_provider`），判据仍绿 ⇒ 端点静默指向错误语义。**改法**（`check-capability-register.mjs`）：有 `anchorText` 时要求整行（空白归一）相等；重锚模式的「相符」同口径 —— 不符则按 anchorText 重新定位，零 / 多命中照旧拒写。**验证**：全量 8 条 anchorText 锚点整行相符 0 例外（`.tmp/ev-anchor-audit.mjs`）；`--selftest` 全绿；判定跑 EXIT=0（`.tmp/gate-20261103-2115.log`）。

### 64.8 `#129` pre-push 的 UNC 失效（2026-11-03）

**根因比 `npx` 更宽**：`ci-check.mjs` 的 `step()` 用 `execSync(cmd)`，Windows 下经 cmd.exe 拉起子进程；cmd.exe 拒绝 UNC cwd ⇒ 退回 `C:\Windows`，所有相对路径失效（改前 `npm run ci:check:quick` 实测报 `Cannot find module 'C:\Windows\scripts\ci-check.mjs'`）。**改法**：① `package.json` 与 `.git/hooks/pre-push` 改直调 `node scripts/ci-check.mjs --quick`；② UNC 下 `step()` 改 `spawnSync` 直拉解释器（不经 shell；带引号 / shell 运算符直接 fail-closed），子进程 cwd 统一换成**盘符镜像**（`findDriveMirror`，与 `check-tsc.mjs` 同判据）—— 因为 tsgo 在 UNC cwd 下 0 文件、门禁脚本内部 `execSync("git …")` 也不支持 UNC；③ `npx dprint / oxlint / tsc / vitest` 全部改 `node node_modules/...` 直调。

**Vitest 处置**：UNC 下 root 被规范成 `Z: Folders/D/...` ⇒ `No test files found`（换映射盘 / `--root` / 绝对 include 均无效）⇒ **显式失败**（`failStep`），指向 `#111`；`--quick` 不含该门。

**验证**：`node scripts/ci-check.mjs --quick` 从 UNC **52 门全绿、EXIT=0、1486.3s**（`.tmp/129-quick-recheck.log`）；`ci.workflow.test.ts` 25/25；hook 与 package.json 命令 MATCH。**顺带**：`i18n-scan` 报的 8 条既有硬编码（插件桥 / 插件页的诊断错误）登记进 `.i18n-allowlist.json`（reason 写明「开发者诊断、不进入宿主 UI」），8 根 fmt 债已在 `093915bfb` 清空 —— 这两条原本会让「真正跑起来的 pre-push」仍然红。

### 64.9 `#19` L2-g 全量门禁终验（2026-11-03）

**判据与 CI 逐字同源**（`.github/workflows/ci.yml:616/619/738` 的三条 cargo 命令），逐根跑、遇锁排队、日志逐根独立：

| 门 | 命令 | 结果 | 日志 |
|---|---|---|---|
| fmt | `cargo fmt --all --check` ×8 根 | **8/8 EXIT=0**（既有债已在 `093915bfb` 清空） | `.tmp/fmt-*.log` |
| clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` ×8 根 | **8/8 EXIT=0**（src-tauri 单根 21m03s） | `.tmp/full-clippy-<root>.log` / `full-clippy-summary.log` |
| test | `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` ×8 根 | **8/8 EXIT=0**：**9447 passed / 0 failed / 57 ignored** | `.tmp/full-test-<root>.log` / `full-test-summary.log` |

逐根用例数（binaries · passed/failed/ignored）：core 15·1036/0/7、kit 2·112/0/3、ai 24·2713/0/9、infra 35·816/0/1、quant 14·1720/0/24、runtime 43·1879/0/8、plugins 17·250/0/0、src-tauri 11·921/0/5。

**口径说明（不许当全绿洗白）**：
- 本轮是在**含并行批未提交工作区**的状态下跑的（`stock_analysis.rs` 等 21 个文件）。它们既参与编译也被测到 ⇒ 「红/绿」是**联合态**，不能单独归因给本批已提交的改动；反过来也说明这些未提交文件在门禁口径下可编译、可测。
- CI 的 `pg_migrations` / `pg_integration` / `pg_upsert_bare_column` 是**独立 Postgres job**，本机无 PG 实例 ⇒ 不在本次终验范围（沿用 CI 跑）。
- `nextest` 本机未安装，用 CI 同义命令 `cargo test --workspace --no-fail-fast` 跑同一测试面；`--no-fail-fast` 保证一次暴露全部失败而非首错短路。

### 64.10 §54.2 R1 第二批裁定：不做通用 `domain.store` KV（2026-11-03，解除 #35 的「不可开工」闸门）

**背景**：§54.2 R1 把域数据落库通道拆成两批，第一批（`session.store` 半截接缝）已由 #102 收口为 **host-internal**；第二批 `domain.store` 一直没有裁定，而 R1 原文写明「未裁定完之前 #35 视为不可开工」。

**机械读数（`commands/stock_analysis.rs`，2026-11-03）**：
- `state.harness.db()` **100 处**；写操作（`insert` / `update` / `delete_by_id`）**42 处**、读（`::Entity::find`）**43 处**，其余为传 `&db` 给 repo / 引擎函数。
- 直连实体 **13 张**：`stock_analyses` · `watchlist_items` · `portfolio_holdings` · `trades` · `price_alerts` · `stock_reflections` · `stock_evolution_history` · `strategy_performance` · `financial_snapshots` · `reco_picks` · `decision_validations` · `settings` · `workflow_template`。
- **仅经 `axagent_dao::repo` 访问另有 7 张**（3 张与上面重合）：`mover_recall_runs` · `mover_recall_events` · `reco_pool` · `reco_seed` · `stock_execution_reflections` · `stock_lesson_queries` · `workflow_param_changes`。
- ⇒ 并集**实测 20 张表**（§54.2 R1 原文写「≥14 张」偏保守，按上表更正）；台面比原估更宽，正是不该做通用 KV 的直接理由。

**裁定**：
1. **不建通用 `domain.store` KV 桶，也不做「按表名 + 任意 JSON」的 CRUD 通道。** 20 张表里有外键/同成约束（`portfolio_holdings` → `trades`、`stock_analyses` → `stock_reflections`/`strategy_performance`）与增量语义（`reco_pool` 批次、`workflow_param_changes` 状态机），通用桶只能表达「按主键存取一行」，既守不住同成性，也会把表结构变成插件可猜的隐式协议。
2. **域数据通道 = 按域命名的显式类型化 op 接缝**（与既有 6 条同形：`market.quote` 37 op / `jobs.store` / `trading.engine` 6 op / `stock.monitor` 9 op / `quote.watcher` 3 op / `knowledge.vector` / `trajectory.store`）。每个 op 满足三条硬约束：① 入参/出参 DTO 定义在 `axagent-harness`（ORM 不过线）；② 装在每个 op 的分派支上带 **op 级权限**（#87 的机制，不新造）；③ 只暴露 B4 清点出的**必需 op**，不按表做 CRUD 全展开 —— 需要哪几个动作就声明哪几个（`upsert_alert` 先例）。
3. **数据仍留在宿主那一个 DB 文件里，插件卸载不删域数据。** 理由：用户的分析/自选/持仓历史不能随插件卸载蒸发；单文件仍是备份与迁移的单位；拆成「插件自有 DB」会引出跨库 join 与存量数据搬迁，收益不抵成本。`schema_version` 校验收敛为**每个域接缝一个版本常量**（随 DTO 演进递增，宿主拒收未知版本），不做「每命名空间一张版本表」。
4. **B4 必须先产出「op 清单」再动手搬本体**：以本节的 20 张表为底，逐条标「宿主继续持有（`settings` / `workflow_template` 这类宿主也要读的）/ 走域接缝 / 随插件整体迁出后不再被宿主读」三档；清单未出之前，B5/B6/B7 不得开工（避免又出现「接缝建了但没人调」的半条链）。

**与 §54.2 R1 原文的关系**：R1 原文的方向是「命名空间白名单 + 每命名空间声明 DTO 类型名与 schema_version + 宿主按注册类型校验后才落库」。本节保留「白名单 / DTO 类型 / 版本校验」三项实质，但把载体从**一张通用 KV**改成**显式类型化 op**：白名单退化为「接缝 op 表」（本来就必须登记，见 §38 第五道门），DTO 类型即 op 的入参/出参，版本校验落在接缝常量上。R1 原文担心的「ORM 不过线」与「表级类型约束」因此由编译器与门禁两侧保证，而不是靠运行时嗅探 JSON。

**状态**：R1 第一、二批均已裁定 ⇒ **#35 的「不可开工」闸门解除**。⚠ 但 #35 的 B2 实际执行仍卡在 `commands/stock_analysis.rs`（并行批未提交的 W4-3 改动占用该文件，逐行改写会与他人未提交工作混提交）——**闸门解除 ≠ 现在可改那个文件**。

### 64.11 §54.3 B4 开工前置：股票域 17 张表的归属与 op 清单（2026-11-03）

本节是 §64.10 第 4 条要求的「B4 先出 op 清单」的产物。扫描面 `src-tauri/src`，读= `Entity::find*` / dao 读函数 / 读 SQL，写= `insert` / `update` / `delete*` / `ActiveModel` / 写 SQL；已剔除注释与 `#[cfg(test)]` 段。

**先纠三处清单错误**（§64.10 的 20 项里有 3 项不是表）：`reco_seed` 无建表（`dao/src/repo/reco_seed.rs:14` 只读 `watchlist_items` + `market_*`，全仓零 `"reco_seed"` 建表）⇒ 按 `watchlist_items` 读口处理；`stock_lesson_queries` 是查询组（`dao/src/repo/stock_lesson_queries.rs:8`，落 `stock_reflections`/`reflection_lessons`/`stock_analyses`，`:659/672` 写 `strategy_performance`）⇒ op 直接镜像其函数名；`reco_pool` 是 dao 模块，真表 `reco_pool_batches`。另 `workflow_template` 真表名 `workflow_templates`。⇒ **实际 17 张表**。

| 表 | 档 | 读/写 | 需要的 op（S 档） |
|---|---|---|---|
| `stock_analyses` | **H** | 30 / 20 | 宿主 gateway(`gateway_stock_store.rs:34/47`)、`scheduler/restore.rs:179`、两个离线 bin 都在读/写 ⇒ 宿主继续持有 |
| `watchlist_items` | **H** | 8 / 7 | 宿主 gateway 直写（`:64/77`）、`init/services.rs:2775` 读；⚠ 增删现有 **三份实现**（命令 / agent 桥 / gateway）应收口到 dao 一处 |
| `settings` | **H** | 8 / 8 | 全仓 61 文件共用；域内只碰 6 个具名键 ⇒ 即便 H 也按具名设置 op 暴露，不做通用 KV |
| `workflow_templates` | **H** | 44 / 22（含内联测试与 seed，只作量级） | 非域 49 文件也在写；域内 seed 建/删模板，插件起来前模板必须在库 ⇒ 种子改宿主具名 op |
| `price_alerts` | **S** | 9 / 6 | `upsert_by_key`（必须一个 op：`price_alert_upsert.rs:52→61/80` 是查未触发同键行再复用/新建）/ `list_all` / `get_by_id` / `delete_by_id` / `list_triggered_in_range` / `mark_triggered_by_id` / `mark_triggered_by_stock_type` |
| `stock_reflections` | **S** | 19 / 14 | `list_page` / `get_by_id` / `insert` / `delete_by_id` / `update_status_batch` / `list_pending` / `delete_completed_before` / `requeue_failed` / `retire_orphan_pending` |
| `stock_evolution_history` | **S** | 11 / 6 | `insert_pending` / `count_pending_since` / `list_pending` / `get_by_id` / `approve_pending`(CAS) / `reject_pending` / `insert_accepted` / `reject_accepted` / `latest_accepted` |
| `strategy_performance` | **S** | 3 / 3 | `insert_sample` / `has_sample` / `delete_by_code_strategy` / `list_by_agreement` |
| `financial_snapshots` | **S** | 3 / 2 | `list_by_code_range` / `replace_code_range`（删窗+逐条插必须同 op，`stock_analysis.rs:3868/3899` 无事务）/ `codes_with_snapshot_since` |
| `reco_picks` | **S** | 18 / 8 | `upsert_pick` / `insert_batch` / `list_latest_by_style` / `list_by_generated_at` / `delete_by_generated_at` / `get_by_id` / `history_aggregate`（`reco_history.rs:57/63` 裸 SQL） |
| `decision_validations` | **S** | 8 / 2 | `insert_if_absent`（`backtest_validation.rs:764→783/814` 判重后跳写）/ `list` / `list_shadow_replay_samples` / `update_factor_snapshot` |
| `mover_recall_runs` | **S** | 4 / 2 | `insert_run_snapshot`（**与 events 同事务**，`dao/src/repo/mover_recall_runs.rs:91`）/ `list_recent` / `fetch_run` / `update_asof_summary` / `latest_reco_run_asof` / `list_runs_since` |
| `reco_pool_batches` | **S** | 2 / 5 | `insert_pool_batch` / `load_pool_batches` / `latest_live_pool_batch` |
| `portfolio_holdings` | **M** | 3 / 5 | 宿主零引用；写点在 `analysis-engine/src/trading.rs`、`trade_import.rs`、`exit_recommend.rs` |
| `trades` | **M** | 2 / 0 | `src-tauri` 内零写；真实写在 `analysis-engine/src/trading.rs:403`、`trade_import.rs:394` |
| `mover_recall_events` | **M** | 14 / 9 | 宿主零引用（`init/services.rs` 仅注释命中） |
| `workflow_param_changes` | **M** | 3 / 3 | 唯一文件就是 `stock_analysis.rs` |

**五条配套裁定**（与 §64.10 同批，B4 直接照此实施）：
1. **`astock-data` 留宿主**：它自己实现 `MarketDataProvider`（`axagent-quant/crates/astock-data/src/lib.rs:6641`）并由宿主注册为 `market.quote`；凭证不过线（§54.2 R2）⇒ 插件内的 `analysis-engine` 一律经 `MarketDataProvider` trait 取数，不得直接构造 `AStockClient`（B7 的活）。
2. **`analysis-engine` 随股票插件**，其 `db()` 写口按 1:1 改接缝：`trades` 已有 `trading.engine` 六 op 承接；`portfolio_holdings` 的增删改需在 `trading.engine` 下补 `portfolio_*` op（不新开接缝名）。
3. **`commands/evolution_proposal.rs` 判域内**（证据：`use axagent_analysis_engine::…` ×3 + `stock_evolution_history` / `stock_lesson_queries`，纯股票进化闭环）。⚠ **但不改 `audit-l1-blockers.mjs` 的 `ownedRe`** —— §54.2 R5 明令「不扩 ownedRe」；该文件随 B4 迁出后，指向它的反向阻塞引用会自然消失，届时读数再降。
4. **宿主 cron → 域函数的两处反向调用**（`init/services.rs:2622` 荐股批次、`:5715` 财务快照回填）改为**插件自持调度**：manifest 声明 schedule，宿主定时器只发 tick 到 worker，不在宿主保留对域函数的调用。
5. **六处跨表同成性必须合成单个 op**（拆开就会丢原子性）：`mover_recall_runs+events`（已有事务）、`financial_snapshots` 删窗+插、`price_alert` 查改写、`stock_evolution_approval` 取 latest→CAS→插 accepted→驳旧 accepted、`reco_picks+reco_scan_audit+reco_pool_batches`（现状已非原子，op 化至少别再加宽窗口）、`reflection` 抢占 running + 建行。

**未做**：未改任何代码（B2 仍卡 `commands/stock_analysis.rs` 的并行批未提交改动）；未跑 `audit-l1-blockers.mjs`（本节只做归属与 op 清单，不重算桶读数）；`stock_management`/`reco_scan_audit` 等未被 §64.10 列出的表未纳入（待 B4 全量扫描再补）。

### 64.12 `#105` / `#110`（半程）/ `#111` 与 W4-3 批次入库（2026-11-04）

**`#105` 监控读数收口 harness**：删掉命令层本地 `MonitorStatusDto`，改 `use axagent_harness::stock_monitor::MonitorStatusView as MonitorStatusDto;` —— 九个键与 camelCase 序列化逐字不变，前端零改动（类型注释同步改指 harness）。**R12 同时收紧**：原判据只比「当前那一份」的键，命令层若再抄一份结构体会静默通过；新判据三条一起判：① harness `MonitorStatusView` 键 = 前端 `MonitorStatus` 键；② 命令层不得再出现 `pub struct MonitorStatusDto`；③ 命令层必须出现 `MonitorStatusView as MonitorStatusDto` 别名引用。**验证**：`check-monitor-event-authority.mjs --selftest` **81/81**（新增两条负控：重抄结构体 / 未切别名）；判定 **R12（9）** 绿；`cargo clippy -p axagent --all-targets --all-features -- -D warnings` **EXIT=0（13m48s）**；`rustfmt --check` 与 `dprint check` 均 0。提交 `e4fa74fa3`。

**`#111` 本机 Vitest（根因与修复，不是「换本地副本」）**：两处叠加 ——
① **vite 8 默认 `bundle`（rolldown）配置加载器**在 VMware 共享目录上把配置里的 `root` / `import.meta.dirname` 解析成畸形串 `Z: Folders/D/OneManager/AxInvest`。二分判据：0–1 个真实插件正常、**≥2 个真实插件必现**；`--root` 显式指定也压不住；`--configLoader native` 正常。
② `scripts/vitest-run.mjs` 的 `realpathSync.native` 把网络映射盘 D: 解析成 UNC 再 `chdir`，root 变 `//vmware-host/...`，同样收集不到用例。
**修复**：vitest-run 只在盘符大小写上规范化（不跟随 UNC），并强制注入 `--configLoader native`；`ci-check.mjs` 的 Vitest 门从「UNC 显式失败」改为走同一入口（UNC 与本地盘同一条路）。**验证（全部从 D: 共享盘实跑）**：`ci.workflow.test.ts` 25/25；W4-3 两条面板用例 16/16；`ScreenerToWorkspace.integration` 5/5；全量套件跑通第一部分后仍受本机 I/O 影响出现集成用例超时（同一用例单独复跑 5/5 通过）⇒ **路径问题已解，剩余是并行负载下的超时 flake**，不作为门禁结论。提交 `31e60d21d`。

**`#110`（半程）**：`git rm --cached` 移除 3 份已入库的 `adapter_model.json`，**工作区文件保留**，`.gitignore:175` 的目录规则自此生效（`git check-ignore` 已验证）。另一半（把 `output_dir` 从相对路径改成宿主 app-data 固定路径）是**写盘行为变更**，本轮未做，仍待微调面确认。提交 `e729d1455`。

**W4-3 荐股池批次留痕（并行批，用户授权单独入库）**：`f57c76c22`（21 文件）。要点：池快照改为四档并集、按代码排序去重后在准备阶段算一次；`dao::reco_pool::latest_live_pool_batch` 只取实盘代且不投影池原文、保留 `has_snapshot`；宿主新增 `get_reco_pool_batch`；面板区分「无批次 / 无快照 / 池为空 / 有只数」四态；i18n 11 语言同步。**验证**：该批 clippy/test 已随 §64.9 全量面全绿；前端两条用例 16/16。另 `register_commands.rs` 的 `plugin_list_pages` 滞后单独提交 `710694efa`（生成物重新生成，与 W4-3 无关）。

### 64.13 B2 开工前的现况读数（2026-11-04，`docs/audits/AUDIT-l1-blockers-2026-11-04.md`）

- 股票反向阻塞 **124 处 / 22 文件**（H 桶 10 / 4 文件，合计 134 / 26 与旧 152 口径可比）；OPC 直接 Rust 阻塞 **0**；跨包口 **29 / 9**（全在 `opc_*`，归 #32/#36）。
- **124 里有 13 处落在 `#[cfg(test)]` 块内**（审计脚本只剔行首注释、不剥测试块 —— 与 `check-*` 门的口径不同）：`conversations/compress.rs` 4、`conversations/mod.rs` 4、`tools/audit.rs` 2、`init/services.rs` 1、`fleet/mod.rs` 1、`tools/astock_data.rs` 1。这些是 **`AppState` 测试构造器**，要等 B6 摘六字段时随之消失，**不属于 B2 的改写面**。⇒ B2 的生产改写面 = **111 处**（不是 124）。
- 审计仍按「124」出数（不剥测试块），本节只登记差集，**不改脚本口径**；B6 完成后测试构造器会自然消失，届时读数一次性下降，不需要「洗绿」动作。
- B2 可直接换 harness 的候选（B1 已 `pub use` 垫片）：`init/services.rs:5047` 的 `MonitorConfig`、`:5059` 的 `AggregatorConfig`；其余同文件引用是 `RealtimeMonitor` / `TZeroCallback` / `TradingEngine` / `StockAdaptiveEngine` / `AStockClient` 等**实现类型**，换接缝属于 B4/B6（持有关系先改），不是单纯改 import。

### 64.14 B2 交了一刀，同时把「−6～−9」这个数推翻（2026-11-04）

**已交付**：`init/services.rs` 装配段两处切到 harness 权威 —— `MonitorConfig`（`use axagent_harness::stock_monitor::MonitorConfig`）与 `AggregatorConfig::default()`。反向阻塞读数 **124 → 123**（`AUDIT-l1-blockers-2026-11-04-0322.md`，与 `-0000` 那份同日两份快照是 §54.2 R8 的预期行为）。`cargo check -p axagent --lib` **Finished 14m17s / 0 error**、`rustfmt --check` 0、`cargo clippy -p axagent-agent --all-targets -D warnings` 0（同批的 #110 也在此窗口验）。

**⚠ §54.3 的「B2 消减 −6～−9」不成立，按证据改判为「B2 实际只有 −1；其余分属 B4/B6/B7 或随本体迁出」**。逐桶拆 124 处（`scripts/audit-l1-blockers.mjs` 的 foreign 桶 + 盘上 `grep -n` 复核）：

| 桶 | 处数 | 为什么不是 B2 的活 |
|---|---|---|
| O 桶（`tools/opc.rs` 41 + `fleet/mod.rs` 4） | 45 | §54.1 已裁「股票迁移一条不许动」，归 #32 |
| `#[cfg(test)]` 内的 `AppState` 测试构造器 | 12（`conversations/{compress,mod}.rs` 各 4、`tools/audit.rs` 2、`tools/astock_data.rs` 1、`init/services.rs` 1；`fleet/mod.rs` 那 1 处与 O 桶重叠，不重复计） | 消失条件是 **B6 摘 AppState 六个域字段**，改 import 改不掉 |
| 路径判 foreign、**功能属股票**的文件 | 12（`evolution_proposal.rs` 7 + `screenshot_diagnosis.rs` 3 + `decision_gates.rs` 1 + `_shared_reco_pool.rs` 1） | 随 B4 本体一起迁出；⚠ 同 §64.11 第 3 条，**不改 `ownedRe`**（R5 禁扩面） |
| 真·宿主待处置面 | **55**（`services.rs` 23 + `state.rs` 8 + 两个离线 bin 8 + `market_sim_service.rs` 2 + `tools/*` 9 + `daily_snapshot_store.rs` 1 + 其余 4） | 里面绝大多数是**实现类型**（`TradingEngine` / `RealtimeMonitor` / `AStockClient` / `CrossStockSignalAggregator`）与引擎函数调用 ⇒ 要等 B4 打包 + B7 算法接缝；单纯改 import 无处可改（本批能改的两处已改完） |

**结论写死，不给后人留乐观数**：#35 的读数从「124 待改写」修正为「**45（O）+ 12（测试）+ 12（随本体迁出）+ 55（宿主真面）**」，而 55 里今天可机械换成 harness 垫片的**只有 1 处，已交付**。后续批次的验收应按这个拆法记，不再引用「B2 −6～−9」。

**未做**：`plugins/stock/` 打包骨架（B4）未建；`screenshot_diagnosis` / `decision_gates` / `_shared_reco_pool` 的域归属只在文档裁定，`audit-l1-blockers.mjs` 与 `l1-host-owned-allowlist.json` 均未改（改了会让 124 这个可比读数失去连续性）。

## 65. B3 交完剩余半：每订阅者超时兜底 + 两级丢弃计数（2026-11-04）

§54.3 的 B3 = 「第四跳 + 每订阅者超时 + 丢弃计数」。第四跳已随宿主批交付（§55 / §57），本批补后两件。**先记一条被单测抓出来的缺陷**：第一轮 harness 单测 13 passed / 1 failed —— `waterfall_timeout_keeps_the_chain_running` 红，实测 `outcome.timed_out = 0`，根因是我在 Waterfall 分支只给订阅者计数（`entered/completed/timed_out`）加了账，**漏加派发汇总的 `timed_out`**；这一支漏了，调用方只看 `DispatchOutcome` 时「插件装死」与「插件表态继续」就不可区分。修在 `axagent-core/crates/harness/src/typed_event.rs:360`（`None => outcome.timed_out += 1`），复跑 14 passed / 0 failed。

**交付面**（行号取盘上 `grep -n`，只认这一个给号引擎）：

| 文件 | 行 | 内容 |
|---|---|---|
| `harness/src/typed_event.rs` | `:188` | `DEFAULT_SUBSCRIBER_TIMEOUT_MS = 60_000`（定位=**兜底**，见下「为什么不是 15 s」） |
| 同上 | `:214` | `with_subscriber_timeout(Duration)`：测试与「宿主知道某条链更紧」的构造口 |
| 同上 | `:237` | `subscribe_named(label, …)`：归因标签（插件事件桥用 plugin_id）；`subscribe()` 委派它，标签退化为 `sub-{id}` |
| 同上 | `:280` | `subscriber_stats() -> Vec<SubscriberStats>`：`entered / completed / timed_out` + `label`，读口按当前订阅列 |
| 同上 | `:307` | `invoke_one()`：进入前加 `entered`，返回后二选一加 `completed` / `timed_out` ⇒ `entered == completed + timed_out` 可对账 |
| 同上 | `:346–383` | 四模式全部套超时；Emit/Serial 计 `timed_out`，Waterfall 超时按 `Continue` 继续问下一条，Parallel 逐条独立界定 |
| `plugins/src/worker.rs` | `:4911` | `EventDeliveryCounters`：`delivered / dropped_serialize / dropped_invoke / dropped_unavailable / dropped_wait` |
| 同上 | `:4942` / `:4958` | `EventDeliveryStats`（快照）+ `dropped_total()` |
| 同上 | `:4967` / `:4983` | `WorkerEventSubscriber` 持计数句柄；投递失败**仍返回 `Continue`**（沿用「不因插件异常阻断宿主」的既有口径），但账记在桥侧 |
| 同上 | `:5124` / `:5163` | `LoadedPlugin::event_stats()` 读口 + 订阅改走 `subscribe_named(plugin_id,…)`；计数由持有方留活口（总线快照随撤销句柄消失） |
| `plugins/src/manager.rs` | `:1166` | `plugin_event_stats(plugin_id)`：与 `plugin_worker_is_running` 同一条取数链（都从 `loaded_workers` 现取） |
| `src-tauri/src/commands/plugin.rs` | `:619` / `:629` / `:639` | `PluginSummaryDto.eventStats`（camelCase）+ `PluginEventStatsDto` + `From<EventDeliveryStats>` |
| `src/types/index.ts` | `:2005` / `:2012` | `eventStats: PluginEventStatsDto | null` 与接口；`pluginStore` 乐观插入处补 `eventStats: null` |

**两级计数为什么必须都有（本批的核心判据）**：总线那一层的 `timed_out` 只回答「订阅者连降级返回都没给」；插件投递失败时桥**照旧返回 `Continue`**，于是总线看到 `completed +1` 而事件其实没送达插件 ⇒ 只看总线会**高估投递成功**。所以按原因分桶的账记在桥侧，`bus_sees_completed_while_bridge_records_a_drop` 这条单测把两个数的差（completed=1 而 delivered=0）钉成断言。

**为什么兜底取 60 s 而不是 15 s**：总线预算若**早于**内层预算到点，内层还在等帧、外层已判死 ⇒ `timed_out` 把「正常返回」与「仍在跑」混成一坨，两级计数从此不可对账。盘上内层事件帧的预算是 `DEFAULT_SEAM_TIMEOUT_MS = 15 s`，且任务 #73 M-3 之后「嵌套 `call_seam` 期间计时挂起」⇒ 一条事件合法地可能跑过 15 s。取 60 s 与宿主既有 `TOOL_SET_EXEC_TIMEOUT_MS` 同量级，并由单测 `subscriber_budget_is_a_backstop_above_the_inner_frame_budget` 锁住「必须 > 15 000」。

**⚠ 刻意没做的事：不给 `ops::EVENT` 进 `SEAM_OP_TIMEOUT_OVERRIDES` 覆盖表。** 该表的收录判据写在 `axagent-core/crates/plugin-proto/src/lib.rs:424` 上方：「证据 = 盘上实测或有出处的宿主预算，『看起来快』不算」，且 `describe` 那条注释点名了不许为凑数进表。我手上**没有**事件投递的 op 级耗时实测，硬进表就是往判据里塞假证据 ⇒ EVENT 继续走 15 s 默认，状态与 `chat`/`embed`/`reflect` 那批一样标「预算待实证」。

**验收（工作根 × 命令 × EXIT × 日志）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| A1 | `cargo check -p axagent-harness --all-targets`（axagent-core） | 0 | `Finished` 1m14s |
| A2 | `cargo clippy -p axagent-harness --all-targets -- -D warnings` | 0 | 首轮 1m43s；Waterfall 那行修完后复跑 **21m08s 仍 0**（`.tmp/b3-harness-clippy2.log`） |
| A3 | `__TAURI_WORKSPACE__=true cargo test -p axagent-harness --lib typed_event` | 0 | 修前 **13 passed / 1 failed**，修后 **14 passed / 0 failed**（`.tmp/b3-harness-test2.log`） |
| B1 | `cargo check -p axagent-plugins --all-targets`（axagent-plugins） | 0 | 1m27s |
| B2 | `cargo clippy -p axagent-plugins --all-targets -- -D warnings` | 0 | Finished 9.79s |
| B3 | `__TAURI_WORKSPACE__=true cargo test -p axagent-plugins --lib worker::` | 0 | **113 passed / 0 failed**，含新增 `event_bridge_counts_drops_by_reason`、`bus_sees_completed_while_bridge_records_a_drop` |
| C1 | `cargo check -p axagent --lib`（src-tauri） | 0 | Finished **20m42s**（含 harness→plugins→主壳全链） |
| C2 | `cargo clippy -p axagent --lib -- -D warnings` | **101（首轮红）** | `src\commands\plugin.rs:618` `doc_lazy_continuation`：`/// -` 列表项后的收束句没缩进 ⇒ 空一行成独立段落修掉；AGENTS.md 早就写过「改注释也会触发 lint」，这次是我自己撞的 |
| D1 | `node scripts/check-monitor-event-authority.mjs --selftest` / 判定 | 0 / 0 | **81/81**，R1–R18 全绿（本批动的是派发总线，不在该门扫描面上，跑它是为了确认没顺手改坏事件权威） |
| D2 | `rustfmt --edition 2024 --check`（4 个 Rust 文件） | 0 | 零 diff |
| E0 | `node node_modules/oxlint/bin/oxlint src/types/index.ts src/stores/feature/pluginStore.ts --max-warnings=0` | 0 | 0 warnings / 0 errors（96 规则） |
| E1 | `node scripts/check-contracts.mjs` | 0 | errors=0 warnings=0 ⇒ `plugin_list` 的 DTO 扩字段未破命令契约 |
| E2 | `node scripts/vitest-run.mjs run src/components/plugin`（**cwd 必须是盘符路径** `/d/OneManager/AxInvest`） | 0 | **4 文件 / 54 passed**，含 `PluginPageHost` 19 条 |
| E3 | 股票反向阻塞读数 | — | `node scripts/audit-l1-blockers.mjs` ⇒ **123 处 / 22 文件**，与 §64.14 交付后逐字相同 ⇒ 本批不触域 crate；快照 `AUDIT-l1-blockers-2026-11-04-0422.md` |

⚠ **`#111` 的修复还差一角，本批实测到**：从 UNC 目录（`//vmware-host/…`）起 `node scripts/vitest-run.mjs run <某文件>` 会报 `No test files found` —— root 落在 UNC 坐标、filter 按相对路径匹配不上；把 cwd 换成盘符镜像（`/d/OneManager/AxInvest`）后同一命令立刻命中。**全量跑（不带过滤器）两种 cwd 都能过**，所以这条只在「定点跑某个测试文件」时咬人 ⇒ 记 **#137**。

**#35 的批次状态**：B1 ✅ / B2 ✅（三接缝 + 权威收口）/ **B3 ✅（本批，含第四跳）** ⇒ 下一个可开工的是 **B4 打包骨架**；B3 是 §54.3 明写的 B6 安全前提，现已解除。

**一条与本批无关但挡住提交的环境缺陷（已修）**：pre-commit 的 `check-line-endings.mjs` 把 **gitignore 的 `.tmp/` 也扫了**，并行会话刚在 `.tmp/` 落了 `refs-dump.txt`（纯 CRLF）与 `ingest-upstream.ps1`（LF）⇒ 一笔与它们毫无关系的提交被拒（`git add` 的九个文件逐字节数 CR 全为 0，实测见 `ALL_LF=0`）。修法不是绕钩子，是把 `.tmp` 加进脚本的 `EXCLUDED_DIRS`（`.gitignore:141` 已声明 `/.tmp/` 不入库 ⇒ 它的行尾不可能污染仓库；且原状下 `--fix` 会去改写**别人正在用**的脚本）。顺手记一条纪律：**数 CR 用 `node` 读字节，不要用 `grep -c $'\r'`**（SMB/UNC 下会给假值，本仓已栽过）。

**未做（逐条点名，不写「后续再议」）**：
1. **UI 未渲染 `eventStats`**：数据链已接到宿主命令与 TS 类型，插件列表页看不见这六个数 ⇒ 需要 11 语言文案 + 面板改动，记 **B3-c**。
2. **脏闸后的历史计数不保留**：`subscriber_stats()` 只列当前订阅；插件停用后「生前吞了多少事件」要靠 `plugin_event_stats` 在卸载前取走，今天没有做「卸载时落一份历史」的动作。
3. **panic 不在管辖内**：`tokio::time::timeout` 不接 panic；订阅者 panic 会击穿派发任务（本批既不修也不假装修，判据写在 `SubscriptionCounters` 注释里）。
4. **EVENT 的 op 级耗时实测**：没有实测 ⇒ 覆盖表不进（见上），需要一次真帧往返测量再裁。
5. ~~**第五道门的两条读数漂移未登记**~~ → **已随 #136 一并登记**：`l1ForeignStock` 现值 **133 处 / 26 文件**（−1 = §64.14 那刀 `services.rs`，§0.10.1 桶① 与 §54 三处估计句同步）、`frontendFileCaliber` 现值 **84 874 行 / 293 文件**（+1 = #105 在 `src/types/stock-analysis.ts` 加的注释行，全篇 10 处同步）、`③ 域自有文件内引用` 444→**456 处 / 41 文件**（W4-3 批扩面所致，非门管）；§64.13 起的尾段作为「同形不同物」的历史读数登记进 `planOld[].alsoAt`（自证 token 见登记表）。⚠ **本条原先把 9 条 `钉句 PLAN:n 不符` 归因给「并行文档打标批插了 3 行」，是错的**：#136 落地时逐条复跑，14 条红**全部**是现值漂移与 alsoAt 段腐烂（缺自证 token），与行号位移无关。但 +3 行这件事本身**确实**会把门打红，实测：把那 3 行插回工作区文件 ⇒ 同一条门 **27 条硬失败**（不止本锚点：`commandTotal`、`stockBoundaryCommands`、`opcBoundaryCommands`、`frontendDirCaliber` 的钉句与区段一起错位），撤掉即回到 EXIT=0 —— 所以登记表钉的是 **HEAD 坐标（3337 行）**，重锚归那批落地时做。回写细节与「为什么那条旧读数要八段 alsoAt」见 **§66**。

---

## 66. 第五道门回写：两条现值重锚 + `planOld` 区段从 5 段扩到 15 段（2026-11-04；`#136`）

**起因**：§65 的未做 5 记了两条读数漂移没回写登记表 —— B2 那刀（§64.14）把股票反向阻塞降了 1 处，#105 的一行注释把前端 `src/**` 行数顶了 1 行。本批只做这一件事：**改表 + 同步 PLAN 正文**，判据脚本 `scripts/check-capability-register.mjs` 一个字没动。

**三条锚点**（改动面 = `scripts/capability-register.json`）：

| 锚点 | 改了什么 | 现值的出处 |
|---|---|---|
| `l1ForeignStock` | `value` 与 `planNumber` 同步到 **133**、`planPinned` PLAN:513 的 token 同步；**新增 `planOld`**（B1 后、B2 前那条旧读数，八段 alsoAt） | `node scripts/audit-l1-blockers.mjs` ⇒ 待改写 123 处 / 22 文件 + H 桶 10 处 / 4 文件 = **合计 133 / 26 文件**（26 未变：B2 那两处同在 `services.rs`） |
| `frontendFileCaliber` | `value` 84873→**84874**、`planNumber` →`84 874`、`planPinned` PLAN:519 的 token 同步；另把 `frontendDirCaliber` 注释里「与 84 873 是两套并行口径」那句改指现值 | 门自己的 `frontendVolume` 实测（+1 行 = #105 在 `src/types/stock-analysis.ts` 加的注释）；293 文件未变 |
| `workflowAuditDomainCommands` | `planOld` 124 的第 2 段（PLAN:523）**换句重锚**（523 被本批改写，旧 token 整句没了 ⇒ `[ALSOAT-STALE]`）；**新增第 5 段 PLAN:3255–3278** | §64.13 / §64.14 是这个数最新的历史出现处；段外一次出现即判红 |

**PLAN 正文同步**（同一笔里做完，不然 `planMustAppear` 与 `planPinned` 立刻红）：§0.10.1 的 8 行表（513 反向阻塞、515 域自有 444→**456 处 / 41 文件**、519 前端行数）、523 桶① 分解句、558 / 568 / 976 / 996 / 1134 的估计句、§0.12.4 漂移列举段（715）。行数类 10 处、反向阻塞现值 5 处、前 5 文件吃掉处数 3 处（88→87）。

**为什么旧读数那档要八段而另一档只要五段** —— 本批唯一需要判据的地方，也是门的一条**既存口径不对称**：

- `planNumber` 的命中走 `planNumberHits`（`check-capability-register.mjs:505`）：左界剔 `\d : . # /`，且**先把日期 / 时刻整串涂白**。
- `planOld` 的「出现在段外即红」走 `numberOccurrences`（同文件 `:480`）：左界**只剔 `\d : . #`，不剔 `/`**，也不涂日期。

⇒ `generated_tool.rs:15/116/127/134` 这种**行号引用清单**在 `planOld` 档会命中，而 `planNumber` 档不会。我没改检测器（放宽判据凑绿是禁手，AGENTS.md 与既有纪律同族），而是按 §61.4 / §63 的先例把三条「同形不同物」显式登记成带自证 token 的区段：PLAN:518 是 `OpcPage.tsx` 的行数、PLAN:1224 是 198 拆分里「3b 只有 DI·构造期选择」的桶内计数、PLAN:1332 是上面那串行号引用。**这条不对称只登记、不在本批统一** —— 统一两档口径要连 `commandTotal` 名下那两条旧读数的既有区段一起复算，另立任务。

**⚠ 登记区段的 token 必须钉住那个数自己**（本批踩到的第二条判据级缺陷）：首版七段里 `518` / `1224` / `3255` 三段只写了「同段里另有的句子」（`零派发点`、`不改脚本口径` 之类），门照样判**健康** ⇒ 段是健康的，却**没有真的为这个数作保**；直到我自己在本文件文末写下那条旧读数的复盘（一行出段红）才暴露。修法是 token 里带上那个数所在的算式（`124 处 / 22 文件`、`构造期选择` + 同段的数），已逐段落表；据此**本 §66 自己也占一段**（本节复述了两条旧读数，落在任何既有区段之外）。

**验收（命令 × EXIT × 取证）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| F1 | `node scripts/check-capability-register.mjs`（回写前） | 101 | **14 条硬失败**：现值差 2（`frontendFileCaliber` 登记 vs 实测、`l1ForeignStock` 登记 vs 实测）+ `planNumber` 数字边界未命中 1 + 钉句不符 2 + `[ALSOAT-STALE]` 1 + 旧读数出段 8 |
| F2 | 同上（只改现值与 523 段之后） | 101 | 3 条 ⇒ 全是那条旧读数的「同形不同物」出现（PLAN:518 / 1224 / 1332） |
| F3 | 同上（回写完） | **0** | `hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`；`planOld 4 项 · 13 个 alsoAt 区段 健康 13 / 腐烂 0` |
| F4 | 同上 `--selftest` | 0 | 覆盖度 17/17 无空洞；含 `planOld.alsoAt`（段内放行 / 段外仍红 / 腐烂双报且专码可区分 / alsoAt 数字 token 同受锚点自洽管辖）与 `--repin-plan`（唯一命中才回写 / 歧义与零命中与缺键拒写）两档 |
| F5 | `node scripts/audit-l1-blockers.mjs` | 0 | 133 / 26、123 / 22、跨包口 29 / 9、域自有 456 / 41、命令 304 / 20 与 138 / 13；输出与已入库的 `AUDIT-l1-blockers-2026-11-04-0422.md` **除时间戳外逐字相同**（`diff` 退 0）⇒ 没多留第 4 份同日快照 |
| F6 | 插回他人那 +3 行文档头再跑 F3 | 101 | **26 条硬失败**（`commandTotal`、`stockBoundaryCommands`、`opcBoundaryCommands`、`frontendDirCaliber` 的钉句与区段一起错位）；撤掉即回 EXIT=0 ⇒ 登记表钉的是 **HEAD 坐标（3378 行）**。⚠ 同一条探针在 §66 写下来**之前**测到 27 条，差的正好 1 条：+3 位移把 §66 那段的**行号窗口**整体搬走，窗口仍判「健康」（token 都还在 38 行里），却顺手**吞掉一条本来出段的出现** ⇒ 区间的「健康」不等于「还在为那个数作保」，位移越大越容易假绿 |

**未做（逐条点名，不写「后续再议」）**：
1. **`③ 域自有 456 处 / 41 文件` 没有锚点** ⇒ 门不检它，本批只同步 PLAN:515 那一格。要不要建硬锚点属 #54 那批「先出数再升 hard」的节奏，不借本批偷偷升。
2. **625 与 456 + 29 + 133 = 618 差 7 处未对账**：前者是分层棘轮 `scripts/layer-discipline-allowlist.json:6` 的 `total`，后者是审计脚本按「域自有 / 跨包口 / 反向阻塞」三分法的合计 —— **两套扫描面不同源**（棘轮含 `bin/` 与离线工具）。不许把它当「有 7 处漏计」来算，也不许顺手抹平；归 **#128**（下调棘轮基线那批）一并裁。
3. **未跑 `--repin-plan`**：本批之后登记表与 HEAD 自洽（CI 与干净检出绿），而工作区里他人未提交的 +3 行文档头已在批末**原样交回**；那批落地时由它自己重锚，本批不替它把坐标钉到一个尚未存在的状态上。

---

## 67. 本机前端验证两条腿：`#137` vitest 坐标与过滤器 + B3-c 面板渲染（2026-11-04）

### 67.1 `#137`：UNC cwd 下 vitest 的定点过滤器恒 0 命中（已修）

**两层现象，都是「坐标不对」而不是「没有测试」**：

- 从 `\\vmware-host\...` 调 `run <文件或目录>`，位置过滤按**相对 root** 匹配；UNC root 与映射盘 root 不是同一串 ⇒ 恒 0 命中，vitest 只报 `No test files found`（退 1，但理由看不出来）。
- 坐标修好后还有第二层：网络盘上 fork worker 启动就慢到撞 vitest 的 60 s `Timeout waiting for worker to respond`（并行时 4 个 worker 全灭；串行时 4 个里能起 2 个）。

**改法**（`scripts/vitest-run.mjs` —— 只动这个本地入口，不动 vitest 配置、不动 CI）：

1. UNC → 扫 A–Z 用 `realpathSync.native("X:\\")` 找映射，取**最长**匹配的盘符根拼回盘符路径；**跳过「项目目录就是盘根」的塌缩别名**（本机 `X:` 恰指向本仓目录 ⇒ 别名塌成 `X:\`，vitest root 变 `X:/`，四个 worker 全 60 s 超时）。
2. 映射不到又带位置过滤 ⇒ **当场退 2 并说明怎么调**，不再让它沉成一句 `No test files found`。
3. root 落在网络共享 ⇒ 默认追加 `--no-file-parallelism`（调用方显式给并行开关时尊重它），实测依据写进注释。

**验收（cwd 一律 = UNC 根）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| G1 | `node scripts/vitest-run.mjs run src/components/plugin`（修前） | 1 | `No test files found, exiting with code 1` |
| G2 | 同上（修后，串行） | 1 | 两次样本：**4 文件 / 54 passed**（628 s）与 **2 文件 / 23 passed + 2 个 worker 启动超时**（505 s）⇒ 过滤器已修；worker 启动超时是**另一层**（未做 1） |
| G3 | 同上加 `--no-file-parallelism` 之前（并行对照） | 1 | 并行时 **4 个 worker 全部启动超时**，两次样本一致 |
| G4 | `node scripts/vitest-run.mjs run src/components/chat/__tests__/PluginMarketplace.eventStats.test.tsx` | 0 | **3 passed**（即 §67.2 的用例，同时验证「新文件路径也能被过滤命中」） |

⚠ **这条腿顺带抓出一条门禁假绿**：`npm run typecheck`（= `tsc --noEmit`）从 UNC cwd 调时，npm 用 cmd.exe 起进程，而 **cmd.exe 不支持 UNC 作 cwd** ⇒ cwd 回退 `C:\Windows`、tsc 找不到 tsconfig、**打印帮助并退 1**（列 0 个文件）。本批改用手工等价命令取证：`cd D:\... && node node_modules/typescript/bin/tsc --noEmit`（同一条 tsc，只是不经 cmd.exe），并加 `--listFiles` **自证真扫了 1455 个项目文件**、两份改动都在列。与 #45 同族但不同因（#45 修的是直接调用；这次是 `npm run` 的 cmd.exe 中转）⇒ 见未做 2。

### 67.2 B3-c：插件面板渲染 `eventStats`（含 11 语言文案）

**改动面**：`src/components/chat/PluginMarketplace.tsx` 每张卡片底部加一行「事件投递 / 丢弃」；`eventStats === null` 显示「未订阅事件」而不是 0 —— 两种状态处置不同，合并会把「没订阅」显示成「零丢弃」。悬停列出三桶明细，并写明 `unavailable` 是 `invoke` 的子集、不重复计入。文案 3 条 × **11 个 locale**（块内按行插入、保序、LF；写盘后逐文件 `JSON.parse` 复核 + CR 字节 0）。

**新增测试**（`src/components/chat/__tests__/PluginMarketplace.eventStats.test.tsx`，3 条）：null ⇒ 未订阅；有值 ⇒ `delivered / droppedTotal` 原样透出；`unavailable` **不加进总数**（若重复相加，`0,2` 会变 `0,4`，用例转红）。

**验收**：dprint 0（两条新/改文件）；oxlint `0 warnings / 0 errors`；tsc 0；`check-jsx-text-i18n` 基线外 0；`check-i18n-exemption-budget` 基线外 0；vitest 见 G4。⚠ `check-i18n-orphan-diff` 本轮**自报跳过**（「取不到 git」）⇒ 增量孤儿 key 未验证，不当通过。

**⚠ 同批回写第五道门**：本批给被 `feRe` 命中的前端文件净加 **135 行 / 1 个文件**（`PluginMarketplace.tsx` +29 行、新测试 106 行）⇒ `frontendFileCaliber` 从 84 874 / 293 涨到 **85 009 / 294**。按判据「改代码必须同批改表」，登记表 `value` / `value2` / `planNumber` / `planPinned[519]` 与 PLAN 的 9 处「85 009 行」、4 处「294 文件」全部同步；§65 未做 5 与 §66 表格里那两个 84 874 **刻意保留**（它们记的是 #136 当天的现值，不是今天的）。⚠ **+1 文件是 `feRe` 的既有假阳性，不是股票域扩面**：正则的 `market` 分支命中文件名里的 `Marketplace`（`src/components/chat/PluginMarketplace.tsx` 在 #139 之前就已落在该口径里）⇒ 本批动的是插件市场面板。不改正则：`feRe` 与 `audit-l1-blockers.mjs:48-63` 逐字同源，改它必须两处同批，属另案。这一条也说明该锚点每碰一次前端就要重锚一次：它钉的是**按路径正则筛出的全仓体量**，不是某个模块的行数。

**未做（逐条点名）**：
1. **worker 启动超时未解**：本机 SMB 上单文件 worker 启动可 >60 s，该超时硬编码在 vitest 的 pool-runner 里、CLI 无开关；`--poolOptions.forks.singleFork` 是**配置项**不是 CLI 参数（实测 `CACError: Unknown option --poolOptions`）。解法要么本地盘检出、要么给本机加一份不被仓库接管的本地 vitest 配置 —— 另立任务，本批不擅自改仓库测试配置（会改 CI 语义）。
2. **`npm run typecheck` 在 UNC 下退 1 且一个文件都不查**（见 §67.1）；本批只用手工等价命令取证，没改 `package.json`。
3. **dprint 在 UNC 下空跑**（`No files found ... at \\?\UNC\...`，退 14；加 `--allow-no-files` 反而退 0，更危险）：本批用「本地盘临时目录 + 指向仓内 wasm 的探针 dprint.json + 逐文件 copy」取证。要不要做成 `scripts/dprint-check.mjs` 另裁。
4. **UI 未在浏览器里目视**：类型 / 格式化 / 单测三关已过，但没起 dev server 看实际排版（本轮无可驱动的 Tauri 实例）。

---

## 68. `#134` 第一片：外部表声明入口 + 四条硬判据（2026-11-04）

**交付**：`axagent-infra/crates/dao/src/reconcile/expected.rs` 新增 `build_with_external(dialect, &[TableModel])`；`build(dialect)` 原地委派（签名与行为不变）。

**四条判据（全是硬失败，不是 warn）**：

1. 外部表与实体/L2 已声明的表**重名 ⇒ 报错**（两份真源，拒绝静默覆盖 —— 与 `overlay_l2` 的「L2 认得、实体不认得 ⇒ 报错」互为反向）；
2. 外部声明**之间**重名 ⇒ 报错；
3. 外部表落到 `non_main_db` 表名上 ⇒ 报错（该方言下这张表不在主库）；
4. **空切片与 `build` 逐位等价**（`canonical_json` 相等，PG / SQLite 各测一次）。

**为什么先交入口、不一次交整条通道**（边界写死，不许把本片当「T4 已完成」）：

- PLAN:541 已定挂载点；本片把它落成代码 + 判据，`plan` / `apply` **一行未改**。
- 还缺两段，且都是**形状决策**而不是机械改写：
  - ① **插件 `schema.json` 的最小形状**：`ColumnModel.sql_type` 存的是**归一后的方言类型名**（`canonical_type` 的产物），不是 `sea_query::ColumnType`；要从 JSON 反序列化，得先定「类型枚举 ↔ `canonical_type` 的双向映射」，否则插件写的 `TEXT` 与实体产出的 `text` 在 diff 里永远不相等。
  - ② **宿主 hook**：`apply::cycle` 里 `expected::build(dialect)` 是硬编码（`apply.rs:808`），且闸 3 用 `want.tables.len()` 做基数 —— 装插件后再跑一轮会把基线抬高，必须先裁定「插件表如何计入基数」，否则一次装载就能让下一轮把**未声明**的插件表判成孤儿。
- 结论：第二片开工前，先把 `schema.json` 形状与「闸 3 基数口径」写进本文件，**不在代码里先射箭**。

**验收（cwd = `axagent-infra/`）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| H1 | `cargo check -p axagent-dao --all-targets` | 0 | `Finished` 3m26s |
| H2 | `cargo test -p axagent-dao --lib reconcile::expected` | 1 → **0** | 首轮 **18 passed / 1 failed**：内部重名用例先撞上「与实体重名」分支 —— 我把两个检查的顺序写反了，错误信息会把「两份插件声明打架」误报成「插件声明与实体打架」；调换顺序后 **19 passed / 0 failed** |
| H3 | `cargo clippy -p axagent-dao --all-targets -- -D warnings` | 1 → **0** | 首轮 **2 条** `clippy::doc_lazy_continuation`（`expected.rs:96/97` —— 有序列表项的续行我只缩进 3 格，clippy 要 4 格）；纯注释改动就把门打红，正是 AGENTS「改注释也会触发 lint」那条的复现。改成 4 格后 **Finished 1m57s / 0 error** |
| H4 | `rustfmt --edition 2024 --check`（单文件） | 1 → **0** | 一处超宽断言自动折行后通过 |

**本片踩到的两个坑（都记下来，别再踩）**：

1. **判据顺序会改错误语义**：内部重名用例首轮失败——因为我在 `upsert_table` 之后才查内部重名，第二份同名表先命中了「与实体重名」分支 ⇒ 报错文案把「两份插件声明打架」误报成「插件声明与实体打架」。查重必须**先查同批、再查存量**。
2. **注释同样过 clippy**：`doc_lazy_continuation` 的有序列表续行缩进是 4 格（不是 3），写 3 格会红两条；`cargo check` / `cargo test` 都不跑这条（AGENTS 的「三条硬约束」第 1 条）。

**未做（第二片，逐条点名）**：① `schema.json` 形状 + JSON→`TableModel` 反序列化；② `apply::cycle` 的外部声明注入口与闸 3 基数口径；③ 端到端「装插件即建表、卸载不删表」的真库测试（要等 ①② 落地）。

## 69. `#134` 第二片：`schema.json` 形状 + 注入口 + 装/卸真库测试（2026-11-04）

**边界**：本片只把「插件声明 → 宿主建表」这条通道打通，**不做 B4 全量**（20 张股票表转 `schema.json` 属于 `#35`）。

### 69.1 `schema.json` v1 最小形状

位置：**插件安装根**（`PluginSummary.metadata.root`）下的 `schema.json`，可选。缺失 = 该插件不声明表，**不是错误**（大量既有插件只贡献 hooks / 页面 / 命令）。

```json
{
  "version": 1,
  "tables": [
    {
      "name": "plugin_stock_watch",
      "columns": [
        { "name": "id", "type": "bigint", "primaryKey": true, "autoIncrement": true },
        { "name": "code", "type": "text" },
        { "name": "score", "type": "double", "nullable": true },
        { "name": "created_at", "type": "timestamp", "default": "CURRENT_TIMESTAMP" }
      ],
      "indexes": [
        { "name": "idx_plugin_stock_watch_code", "cols": ["code"], "unique": false }
      ],
      "checks": [
        { "name": "ck_plugin_stock_watch_score", "expr": "score >= 0" }
      ],
      "foreignKeys": []
    }
  ]
}
```

字段全集（**刻意只到这里**）：

| 层 | 字段 | 语义 |
|---|---|---|
| 根 | `version` | 必须**恰好**为 `1`；未知版本直接报错，不降级、不猜测 |
| 表 | `name` / `columns` | 必填 |
| 表 | `primaryKey` | 复合主键列名数组；缺省 = 用列级 `primaryKey` 表达单列主键 |
| 表 | `indexes` / `checks` / `foreignKeys` | 选填，缺省为空 |
| 列 | `name` / `type` | 必填 |
| 列 | `nullable` | 缺省 **false（NOT NULL）**，与实体侧「默认非空」一致 |
| 列 | `primaryKey` / `unique` / `autoIncrement` | 布尔，缺省 false |
| 列 | `default` | **原样 SQL 表达式**（渲染直接拼进 DDL） |
| 索引 | `name` / `cols` / `unique` | 不支持 `method` / `where`（v1 边界） |
| CHECK | `name` / `expr` | |
| 外键 | `name?` / `cols` / `refTable` / `refCols` / `onDelete?` / `onUpdate?` | 动作词表 `Cascade` / `SetNull` / `Restrict` / `NoAction` / `SetDefault` |

**不能声明的**（不是遗漏，是越权）：`extras`（宿主 L2 认领标记）、`renamedFrom`（插件没有实体迁移史）、列 `generated`（v1 边界）。所有结构体 `deny_unknown_fields` ⇒ 拼错一个键就报错，而不是静默忽略。

**类型词表**（JSON token → 期望侧归一串）：

| token | PG | SQLite |
|---|---|---|
| `text` | `text` | `text` |
| `varchar(n)` / `char(n)` | `varchar(n)` / `char(n)` | `text` |
| `smallint` / `integer` / `bigint` | 同名 | `integer` |
| `real` / `double` | `real` / `double precision` | `real` |
| `numeric` / `numeric(p,s)` | 同名 | `real`（亲和性把 decimal 折进 REAL） |
| `boolean` | `boolean` | `integer`（`BOOL` 折叠） |
| `blob` | `bytea` | `blob` |
| `timestamp` / `timestamptz` / `date` / `time` | 同名 | `text` |
| `json` / `jsonb` / `uuid` | 同名 | `text` |
| `vector(n)` | `vector(n)` | **拒绝**（SQLite 渲染器无此类型） |

实现**不建第二张映射表**：token → `sea_query::ColumnType` → 复用 `canonical_type_expected(dialect)`。上表是**测试预言**，由回归用例逐行钉住；「双向」指 `canonical_type_actual(canonical_type_expected(ct))` 必须回到同一个串（否则插件写 `TEXT` 与实体产出的 `text` 永远不等）。

**硬判据**（全部是错误，不是 warn）：

1. `version != 1`、任何未知键 ⇒ 报错；
2. 表名非空、**不以 `_ax_` 开头**（那是宿主簿记命名空间，`safety` 的审计 / 待删表都在这里）、文件内不重名；与实体 / L2 重名由 `build_with_external` 拒；
3. 主键只能二选一：列级 `primaryKey` **或**表级 `primaryKey` 数组，同时出现 ⇒ 报错；主键列必须存在且 `nullable=false`；
4. 索引 / 外键引用的**本表列**必须存在；索引 `cols` 非空；索引 / CHECK 名非空；
5. `autoIncrement` 只允许整数族（`smallint` / `integer` / `bigint`），且与 `default` 互斥；
6. `default` / `expr` 做**结构校验**：非空、无 `;`、无 `--` / `/*`、引号与括号配平。**它不是 SQL 防火墙** —— 插件本来就是可执行包，这里只挡「语法噪声把整批 DDL 带歪」；
7. SQLite 上 `vector(n)` ⇒ 报错（该方言渲染会 panic，必须在声明期拒）。

### 69.2 闸 3 基数口径（插件表如何计入）

`cycle_with_external(db, opts, &[TableModel])` 与 `cycle` 的唯一差异是期望侧输入；**基数仍是 `want.tables.len()`，现在天然包含外部表**。

- 闸 3（`safety.rs` 的 `BaseShrank`）只在 `now < last` 时拒绝 ⇒ **装插件抬高基线是允许方向**；
- 卸载后声明消失、基线回落 ⇒ 非启动路径会被闸 3 拒绝整批（`BaseShrank`）；启动路径 `last_expected_table_count = None` 且 `additive_only = true` ⇒ 不判、不删；
- ⇒ 「卸载不删表」由**两道既有机制**保证：卸载路径零 DDL + 启动路径 additive-only / 闸 2 零破坏配额。**不引入新的表级白名单**。

**如实登记的边界**：离线探针（非 additive-only）如果**不传**外部声明，插件表会被判孤儿；此时仍受闸 4（导出证据）与闸 5（首见延迟）约束，但「永不 DROP」要靠调用方传齐声明或后续的「插件表归属登记表」——记在 69.5 未做。

### 69.3 挂载点（全部是 wiring，`dao` 不认识插件）

| 时机 | 做什么 |
|---|---|
| `plugin_install` 成功后 | 读 `<install_path>/schema.json` → `cycle_with_external(bootstrap_options())` |
| `plugin_enable` 成功后 | 同上（覆盖「装的时候建表失败」与「库被换掉」） |
| `plugin_update` 成功后 | 同上（新版本可能新增表；旧表不删） |
| 启动 `start_plugins` 里 `start_enabled_plugins()` 之后 | 对**全部已安装**插件（含 disabled）各跑一次 —— 表的归属是「安装」，不是「启用」 |
| `plugin_uninstall` | **零 DDL**（这是本片要测的行为，不是漏写） |

统一用 `bootstrap_options()`：`dry_run=false` + `additive_only=true` + `max_destructive_per_run=0` + `last=None`。
`schema.json` 缺失 ⇒ no-op；存在但非法 ⇒ 安装 / 启用命令返回错误（带文件路径与字段），启动路径只 `error!` 并继续（绝不让一个坏声明挡住 App 启动）。

### 69.4 验收（2026-11-04 实测）

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| J1 | `cargo check -p axagent-dao --all-targets` | 0 | `Finished` 1m18s；首轮 1 条 `unused variable: expect_args`（`vector` 改成可选参数后该闭包失去调用方）⇒ 已删，后续 J2–J4 在修后源码上复跑 |
| J2 | `cargo test -p axagent-dao --lib reconcile::external` | 0 | **15 passed / 0 failed**；覆盖类型词表双方言回读、未知版本 / 未知键、`_ax_` 命名空间、主键二选一、列引用、自增约束、默认值结构校验、FK 动作词表、SQLite `vector` 拒绝、重名 |
| J3 | `cargo test -p axagent-dao --lib reconcile::apply::tests::cycle_with_external` | 0 | **1 passed / 0 failed**（3.04s）：建表 + 闸 3 基数含外部表 + 第二轮 `executed == 0` + 声明消失后表仍在 |
| J4 | `cargo clippy -p axagent-dao --all-targets -- -D warnings` | 0 | `Finished` 1m26s |
| J5 | `cargo check -p axagent --all-targets` | 1 → **0** | 首轮 **EXIT 101 / E0282**：安装 / 更新命令的闭包由「函数尾表达式」改成「先绑定再继续用」后，`Ok(...)` 的 `E` 无法推断 ⇒ 给两个闭包补 `-> Result<…, String>`；修后 `Finished` **16m56s** |
| J6 | `cargo clippy -p axagent --all-targets -- -D warnings` | 0 | `Finished` 15m46s；0 error（首轮 J5 的 E0282 同批修复后一次过） |
| J7 | `node scripts/check-capability-register.mjs` | 0 | 登记 37 行；`k1RsFileCount` **2030→2032**（本片新增 2 个 `.rs`）同批回写登记表 + PLAN:247；全量 37 项一致 |

### 69.5 未做（逐条点名）

1. 20 张股票表转 `schema.json` —— `#35` 的范围，本片只提供通道；
2. 离线探针的外部声明输入（`examples/p5_engine_takeover` 目前不认识插件）；
3. 「插件表归属登记表」—— 让卸载后永不 DROP 不再依赖调用方每次传齐声明；
4. 索引 `method` / partial `where`、列 `generated` —— v1 显式边界；
5. 插件 schema 变更（加列 / 加索引）的审计报告入口 —— 当前走既有 `ApplyOutcome`，未投影到前端；
6. 跨插件 / 插件→宿主的外键**建表顺序**未特判 —— 同一批次里的 `CREATE TABLE` 顺序由 `plan` 决定，外键指向尚未建出的表时由数据库报错（不静默）；B4 若需要跨表外键，先补拓扑排序或拆两轮。

### 69.6 本片踩到的三个坑

1. **新增 `.rs` 文件会抬 K1 文件数**：2030→2032（`reconcile/external.rs` + `init/plugin_schema.rs`）。第五道门因此硬红，必须**同批**改登记表 `value` / `planNumber` / `planPinned` 与 PLAN:247 —— 这不是「放宽判据」，是镜像表跟上盘上事实。
2. **他人未提交的文档头会让 `planPinned` 集体假红**：PLAN 头部多出 3 行未提交内容时，511–526 段整体下移 3 行 ⇒ 第五道门一次报出 **26 条**「钉句不符」。验证/提交前必须把那 3 行 park 回 HEAD 形态，门绿后再原样交回；**不能**跑 `--repin-plan` 去迁就别人的未提交改动（那会把行号永久钉到未提交状态）。
3. **`spawn_blocking` 闭包改成「先绑定再继续用」会丢类型推断**：原先 `Ok(Dto{..})` 是函数尾表达式，返回类型由签名决定；绑定成 `let outcome = …??;` 后，闭包内部的 `E` 不再有上下文 ⇒ `E0282`。给闭包显式补 `-> Result<Dto, String>`，不要用 `Ok::<_, String>` 逐点标注（两处形态不一致时会再漂）。

## 70. `#138`：alsoAt 区间的「假健康」判据（2026-11-04）

**缺陷**：旧健康判据只要求 `mustContain` 的每个 token 出现在区段内。行号位移后可能出现两种假健康：① 旧读数已经移出区段，只剩 token 在段里 —— 区段变成一个永远放行的空洞；② 一个无关的同数出现漂进宽窗口，被整段放行（窗口「吞掉」了出段读数）。

**新判据**（`scripts/check-capability-register.mjs` 的 `alsoAtRegionStatus`，三条同时成立才算健康）：

1. 区间合法且带自证 token（原判据不变）；
2. **段内仍有旧读数**（`planOld[].number` 的数字边界形态）—— 没有出现 ⇒ 段已漂离断言，报 `[区段腐烂 ALSOAT-STALE]`；
3. **出现行 ±3 行内命中某个自证 token**（`ALSOAT_ANCHOR_RADIUS = 3`）。

**放行口径同时收窄**：合法出现按 **`anchored` 出现行**判定，不再按整个区间判定。落在同区间但离 token 超过半径的旧读数，报「窗口不得吞掉无关同数出现」，并提示「收窄区段或补自证 token」。

**登记表同批补键**（3 个区段，共 7 个 token；具体字面量见 `scripts/capability-register.json`，本段不复述数字以免自造旧读数出现）：

- `workflowAuditDomainCommands` 的 §63 交付账区段：补「算式行」与「同一 `alsoAt` 段」两个 token；
- 同锚点的 §64 区段：补「反向下修结论行」与「逐桶拆行」两个 token；
- `l1ForeignStock` 的 §63 区段：补「算式行」「value 下修行」「同一 `alsoAt` 段」三个 token。

**`--repin-plan` 同步**：重锚 ref 携带 `number`，因此「旧读数已移出区段」的段会判为不健康并被重锚（而不是原样保留）；token 最小包络的重锚语义未变。

**验收**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| L1 | `node scripts/check-capability-register.mjs --selftest` | 0 | 新增负控 ⑯a/⑯a2/⑯c（远处同数、出现行识别、段内无出现）与正控 ⑯b（同行 token 放行）；覆盖度 **17 / 17**，无空洞 |
| L2 | `node scripts/check-capability-register.mjs` | 0 | 15 个 alsoAt 区段 **健康 15 / 腐烂 0**；37 项 hard 判据全一致 |
| L3 | 位移模拟（在文档头插 3 行后调 `checkPlan`） | 红 | **26 条硬失败 / 7 条 ALSOAT-STALE / 15 条旧读数出段**，无一条静默通过 |

**未做**：锚定半径是固定 3 行；若某处合法旧读数天然离任何 token 都超过 3 行，必须为该处补一个自证 token（不接受把半径调大来迁就）。

## 71. `#140`：EVENT op 帧往返实测与超时表裁定（2026-11-04）

**测法**（`axagent-plugins/crates/plugins/src/worker.rs::tests`）：回环 socket 起一个假 worker（读一帧、原样回 `{"verdict":"continue"}`），宿主侧用**与生产同源**的 `WorkerInvoker::from_streams`，经 `WorkerEventSubscriber::handle` 走完整路径（序列化事件 → `spawn_blocking` → `invoke_with_chain` → 写帧 → 配对回帧 → 裁决）。预热 1 次后采样 200 次。

**实测**（2026-11-04，本机，两轮各 200 次）：`p50 0.39–0.50 ms`、`p99 0.87–1.74 ms`、`max 1.5–2.1 ms`。

**裁定**：**`event` 不进 `SEAM_OP_TIMEOUT_OVERRIDES`**。相对默认接缝预算（15 s）有四个数量级余量，调大只会掩盖真正的僵死；证据已回写 `plugin-proto` 的收录判据注释。

**测试自带的两条防回归**：
1. 上界固定为 **2 s**（≈2000× 余量）—— CI 抖动下稳定，同时能抓「帧没走通、一路等到默认预算」这类回归；
2. 收尾必须先 `shutdown` 对端 socket 再 `invoker.shutdown()` —— 写端/读端各持一个 `try_clone` handle，只 drop 写端不会让读线程收到 EOF，否则测试会挂到进程超时（本片第一次跑就踩到，按 PID 终止自己的测试进程后修）。

**本片未做**：`send` / `dispatch` / `chat` / `embed` 等触外网 op 仍无 op 级实测；它们的预算裁定要各自补实测，不借本条结论。

## 72. `#142`：本机 typecheck / dprint 的可用入口（2026-11-04）

**结论先说**：`npm run *` 在 UNC cwd 下**修不了** —— npm 经 cmd.exe 拉起脚本，cmd.exe 拒绝 UNC cwd、退回 `C:\Windows`，**命令根本没进仓库**（不是 tsc / dprint 自身的问题）。可用入口 = 同一批 node 命令直调；npm scripts 已改成 node 入口，**盘符路径下**（`D:\OneManager\AxInvest`）可正常用，且与 CI / `ci-check.mjs` 的调用形态同源。

**实测（2026-11-04，本机）**：

| 入口 | cwd | 结果 |
|---|---|---|
| `npm run typecheck`（改前 = `tsc --noEmit`） | UNC | **EXIT=1** —— cmd.exe 打印「UNC 路径不支持…默认值设为 Windows 目录」，tsc 在 `C:\Windows` 下找不到项目 ⇒ 吐帮助横幅 |
| `npm run format:check`（改前 = `dprint check`） | UNC | **EXIT=11** —— `No config file found at C:\Windows\dprint.json` |
| `npm run check:routes:selftest`（任意 `node` 脚本） | UNC | **EXIT=1 / MODULE_NOT_FOUND** ⇒ 证明问题不在 tsc / dprint，而是**所有** npm script 都起不来 |
| `node scripts/check-tsc.mjs` | UNC | EXIT=0，先报「发现 src 源文件 1460 个」再跑完 `--noEmit` |
| `node node_modules/dprint/bin.cjs check --incremental=false` | UNC | EXIT=0；`output-file-paths` 列出 **1781** 个文件（绝对路径） |
| `npm run format:check`（改后） | `D:\` | EXIT=0 |
| `npm run lint:eslint`（改后） | `D:\` | EXIT=0（1436 文件 / 96 规则 / 3.9 s） |
| `npm run typecheck`（改后） | `D:\` | EXIT=0（1460 文件） |

**dprint 不是空跑的判据**：往 `src/__tests__/` 塞一个故意未格式化的探针（`export const    t142Probe=1`）⇒ `dprint check --incremental=false` **EXIT=20** 并点名该文件；探针当场删除、`src/__tests__` 恢复原样。空扫描面本来也会被 dprint 自己判红（无 `--allow-no-files`），本条只是把「它真在比内容」钉住。

**改动**（`package.json`）：`typecheck` / `lint` → `node scripts/check-tsc.mjs`（带盘符镜像换算 + 文件发现数下限，比裸 `tsc --noEmit` 多两道 fail-closed）；`format` / `format:check` → `node node_modules/dprint/bin.cjs …`（check 带 `--incremental=false`，与 `ci-check.mjs` 的步骤同参）；`lint:eslint` → `node node_modules/oxlint/bin/oxlint src --max-warnings=0`（同属 `.bin` shim 一族，同一个坑）。`scripts/check-tsc.mjs` 头部同步改为「唯一入口」，npm scripts / pre-commit 三处指向对齐。

**本片未做**：`build`（`tsc && vite build`）、`test:e2e`、`dev` / `preview` / `tauri*` 仍是 `.bin` shim 或复合命令，UNC cwd 下同样起不来 —— 但它们不在这条日常路径上（构建走 `node scripts/tauri-build.mjs`、E2E 在 CI 跑），未一并改，避免把「入口可用」扩成「重写全部脚本」。另外 `npm run` 在 UNC 下仍不可用，这是 npm + cmd.exe 的固有限制，本仓不做 `.npmrc` 级改法（`script-shell` 指向 Git Bash 会连带影响 Linux CI）。

## 73. `#141`：SMB 上 vitest 的 fork worker 启动超时 → 网络 root 默认换 threads 池（2026-11-04）

**根因不是配置**：`Timeout waiting for worker to respond` 的时限是 vitest 内部常量 `START_TIMEOUT = 6e4`（`node_modules/vitest/dist/chunks/cli-api.*.js`，本机 4.1.12），CLI 没有对应开关、`poolOptions.*` 也够不着；而 forks 池在 SMB 上**每个测试文件都要重起一次进程**，握手本身就能吃掉 60 s。

**四条对照**（本机 SMB，4 个 plugin 测试文件 = 54 tests，统一走 `node scripts/vitest-run.mjs run …`）：

| 变体 | 结果 |
|---|---|
| forks + 串行（改前默认） | 两次样本：628 s / 54 passed；505 s / 23 passed + **2 个 worker 启动超时** |
| forks + `--no-isolate` | 48.6 s 跑完 4 文件，但 **3 文件 / 17 条断言转红** —— 单 worker 复用模块表，`vi.mocked(invoke)` 的 mock 跨文件污染 ⇒ **排除**（这是语义破坏，不是性能方案） |
| threads + 串行 | **4 文件 / 54 passed，两次 EXIT=0**（181.5 s、199.1 s） |
| threads + 串行（重型单文件 `WorkspaceHub.test.tsx` —— 配置注释里 4.1.4 时代 threads 池崩过的那个） | 14 passed / 76.6 s |

**改法**（`scripts/vitest-run.mjs`，只动本地入口）：网络 root 且调用方没给 `--pool` ⇒ 追加 `--pool=threads`；原有 `--no-file-parallelism` 保留（SMB 上并行仍会撞同一个 60 s 握手常量）。显式 `--pool=…` 一律尊重（实测 `--pool=forks` 不再叠加 threads）。另加一行**自证输出**（仅网络 root）：`[vitest-run] 网络 root ⇒ --pool=threads --no-file-parallelism`，排障时不用读源码猜这次用的是哪个池。

**未做（逐条点名）**：
1. **没改 `vite.config.ts` 的 `pool: "forks"`**：CI 与本地盘仍走配置原值，本机差异只落在包装器。`threads` 在本机 4.1.12 上只复测了 5 个文件（4 plugin + WorkspaceHub），**没跑全量 280 个测试文件**；把全量默认池换掉要另立任务，且必须在 CI 之外单独验证。
2. **没解决 SMB 本身慢**：单文件 import 28–37 s、environment 12–16 s 仍照旧。本片只让 worker 别撞 60 s 握手，不把网络盘变快。

## 74. `#35` B4-a：股票插件 worker 骨架（2026-11-04）

**交付**：新 crate `axagent-plugins/crates/plugin-stock`（`src/main.rs` + `tests/protocol.rs`），登记进 `axagent-plugins` workspace；角色写进新建的 `axagent-plugins/crates/README.md`（「插件侧可执行，不入 harness 分层，只允许依赖 foundation」）。

- **帧协议零复制**：宿主与 worker 用同一份 `axagent-plugin-proto` 类型（`FrameRequest` / `FrameResponse` / `PluginDeclaration`），不另造 JSON 形状；
- 只实现 `describe`（回合法声明：`proto_version` = 1，subscribe / calls / capabilities 全空）与 `shutdown`（回执后以 0 退出）；**EOF 也是干净退出**（宿主停插件靠关 stdin）；
- 其余 op 一律回 `PLUGIN_UNKNOWN_OP` 错误帧，且 `request_id` 原样回填 —— 不静默成功；
- **不声明任何能力**：域逻辑未搬入前写空能力，等于在宿主侧留一条调不通的接缝（半条链），本片明确不做。

**为什么是纯可执行包而不是 lib + bin**：分层门禁规则 4 要求「每个库 crate 至少被一份 `Cargo.toml` 引用」，而 worker 由宿主 spawn、没有 Cargo 消费方 —— 首版 lib + bin 实测被该规则判红（`crate-has-consumer 2 > 基线 1`）。门禁对「纯可执行包」已有 `crates/schema-gen` 先例，故照同一形态（删 lib.rs、逻辑落 main.rs），**不改门禁判据**。

**验收**：

| 标签 | 命令（cwd = `axagent-plugins`，除注明外） | EXIT | 取证 |
|---|---|---|---|
| A1 | `cargo check -p axagent-plugin-stock --all-targets` | 0 | Finished 20.73s |
| A2 | `cargo clippy -p axagent-plugin-stock --all-targets -- -D warnings` | 0 | Finished 2.09s（重构后复跑） |
| A3 | `cargo test -p axagent-plugin-stock` | 0 | **3 passed**：①真进程 + 真管道 describe，回帧再过宿主同一套 `validate()`；②未知 op 回错误码且不静默成功；③关 stdin 干净退出 |
| A4 | `rustfmt --edition 2024 --check`（2 个文件） | 0 | 零 diff |
| A5 | `node scripts/check-layer-discipline.mjs`（仓根） | 0 | 首版 lib + bin ⇒ 规则 4 红；改纯可执行包后 ✅ 通过（`crate-has-consumer 1` = `schema-gen` 的既有基线） |
| A6 | `node scripts/check-capability-register.mjs`（仓根） | 0 | K1 **2032 → 2033**：加 2 个 `.rs` 而 `tests/protocol.rs` 被 `K1_IGNORE` 的 `tests` 排除 ⇒ 只升 1；`pub trait` 仍是 324 |

**未做（逐条点名，不写「后续再议」）**：
1. **`bundled/stock/plugin.json` 清单与产物分发未做**：`PluginWorkerDecl.program` 是「相对插件安装目录」的路径，而仓内**没有**「编译产物随 bundle 分发」的通道（现有 `bundled/` 两个样例都是 hooks/ui 资产）。这条要先定「worker 可执行怎么进包」（构建脚本拷贝 / 制品仓库 / 首次启用时现编），属 B4 下一批。
2. **T2 命令声明与分派未做**：manifest `commands[]` + `ops::COMMAND` 分支一并留到域逻辑搬入时做，现在写空命令列表会制造「声明了但没实现」。
3. **不降反向阻塞读数**（计划内）：B4 走 ownedRe 桶，读数 123 处 / 22 文件本片不动；下一次下降要等本体搬迁。

## 75. `#35` B4-b：插件目录打包脚本 + 成品自检（2026-11-04）

**接的是 §74 未做 1**：`PluginWorkerDecl.program` 只接受**相对插件安装目录**的路径（`resolve_worker_program` 拒绝对路径与 `..`），而 `cargo build` 的产物在 target 目录 —— 中间缺「把产物摆进插件目录」这一步，本片补的就是它。

**交付**：`scripts/build-plugin-bundle.mjs`（npm script `plugin:bundle`），四步：

1. `cargo build -p axagent-plugin-stock`（`--release` 可选）；
2. 用 `cargo metadata` 取 `target_directory` 与包版本 —— **不猜 `target/`**（本机 `CARGO_TARGET_DIR` 指向 `~/.cargo/target-axagent`，写死 `target/` 会打出空包）；
3. 拷到 `<out>/bin/<exe>`，写 `<out>/plugin.json`：`worker.program` = `bin/<exe>`（相对路径）、`permissions` / `mcp_servers` / `skills` / `agents` 按 `PluginManifest` 的**无 default 必填字段**给空数组、`defaultEnabled: false`、`integrity` 用现算的 sha256；
4. `--check`：spawn **打包出来的那个文件**（不是 target 里的），按帧协议 `describe` → `shutdown`，核对 `request_id` 回填、`proto_version` = 1、退出码 0，并核对清单 `integrity.hash` 与盘上一致。

**实测**（`node scripts/build-plugin-bundle.mjs --check`，EXIT=0）：产物 `dist/plugins/stock/{plugin.json, bin/axagent-plugin-stock.exe}`；sha256 `97ac4c4e…c6cd`；自检行 `describe 握手 + shutdown 回执 + 退出码 0 + sha256 一致`。`dist/` 已被 `.gitignore` 忽略 ⇒ 产物不入库。

**未做（逐条点名）**：
1. **宿主侧清单解析未在本片验证**：脚本按 `minimal_plugin_json`（`crates/plugins/tests/common/mod.rs`）的同形状给字段，但真跑一遍 `load_plugin_from_directory` 要等「从 bundle 目录走真实安装 + 启用」的批次；现在不造假门（没有 skip-if-absent 的伪测试）。
2. **产物没接发布链路**：未进 `tauri.conf.json` 的 resources，装好的 App 里怎么带插件包仍空白 —— 属发布链路，与插件独立仓（#32 同族）一起裁。
3. **`--check` 不验 `resolve_worker_program`**：它只在 **enable** 时调用（`manager.rs:1114`），本片产物 `defaultEnabled: false`，不会走到那里；启用路径的端到端验证留到 B4-c。

## 76. `#35` B4-c：首个真实能力搬进 worker（`tool.set` + `get_stock_quote`）（2026-11-04）

**首刀选择判据**（§64.11 op 清单里筛「宿主只读、不持 AppState、不碰凭证」那一档）：

| 候选 | 裁定 |
|---|---|
| `compute_scoring` 等打分/分析 op | ✖ 域逻辑在 `astock-data`，而 §64.11 裁定 1 明确 `astock-data` **留宿主**；搬它等于先推翻该裁定 |
| 新建一条只给插件用的只读域接缝 | ✖ 没有真实调用方 ⇒ 造出「接缝建了但没人调」的半条链 |
| **`tool.set` + `get_stock_quote`** | ✔ 宿主侧 `RemoteToolSet::load` / `RemoteTool::call` 就是真实消费者；取数复用**既有** `market.quote.get_quote`；不改前端 |

**线上格式（两侧均由既有代码锁定，本片不自造）**：
- 宿主 → worker：`op="list"` 取 `RemoteToolDescriptor` 数组；`op="exec"` 入参 `{name, args}`，回参 `{content: String, metadata}`（`crates/plugins/src/worker.rs` 的 `RemoteToolSet` / `RemoteTool`）；
- worker → 宿主：`op=call_seam`，`args={seam:"market.quote", op:"get_quote", args:{stock_code}}`，`call_chain` 原样带上入站链（否则宿主环检测收不到这条边）；
- 权限闸：`required_permission_for("tool.set","exec")` 落在 `_ => None`（`worker.rs:458-471`）⇒ **无需 `permissions`**，§75 生成的空 `permissions` 数组是对的口径，不改 manifest。

**交付**（只改既有 2 文件，无新增 `.rs`）：

- `crates/plugin-stock/src/main.rs`：改用 `axagent_plugin_proto::serve(handle)` —— 插件一旦能主动调宿主，同一条管道上就同时挂着「我方请求的响应」与「对端新请求」，自写「读一帧写一帧」必然错配；`Peer` 把对端请求进 `deferred` 队列。`handle` 新增 `list` / `exec` 两支：`list` 回单工具描述（camelCase `inputSchema` / `category`），`exec` 校验工具名与 `stock_code`（空白按 `SEAM_CALL_INVALID_ARGS`），再 `peer.call_seam(MARKET_SEAM, "get_quote", …)`，成功把行情序列化进 `content` 字符串并带 `metadata.stock_code`，失败**原样透传**提供方的 `code` / `message`（不包装成成功）。`describe` 声明 `calls:["market.quote"]` + `capabilities:[{seam:"tool.set", op:"exec", version:"0.1.0"}]`。
- `crates/plugin-stock/tests/protocol.rs`：真进程 + 真管道，测试自己扮演宿主。

**验收（2026-11-04 实测，日志 `.tmp/t35b4c-{check,clippy,test}.log`）**：

| 标签 | 命令（cwd = `axagent-plugins`） | EXIT | 取证 |
|---|---|---|---|
| C1 | `cargo check -p axagent-plugin-stock --all-targets` | 0 | Finished 3.55s |
| C2 | `cargo clippy -p axagent-plugin-stock --all-targets -- -D warnings` | 0 | 零告警 |
| C3 | `cargo test -p axagent-plugin-stock` | 0 | **6 passed / 0 failed**（0.10s）：①describe 声明 `tool.set/exec` + `calls=["market.quote"]` 且过宿主同一套 `validate()`；②`list` 露 `get_stock_quote` 且 `inputSchema.required` 正确；③`exec` 全链：worker 先发 `call_seam` → 测试回 `{price:1500.25}` → 工具回 `content` 为 JSON 字符串且 `metadata.stock_code` 一致；④提供方错误原样透传（`MARKET_PROVIDER_UNAVAILABLE` 不被吞）；⑤未知 op / 未知工具 / 空白 `stock_code` 三种显式错误码；⑥关 stdin 干净退出 |
| C4 | `rustfmt --edition 2024 --check`（2 个文件） | 0 | 零 diff |

K1 不变（**2033**）：本片只改既有 `main.rs` 与 `tests/protocol.rs`，无新增 `.rs`。

**未做（逐条点名）**：
1. **未用真实 Rust 宿主跑 `RemoteToolSet::load` → `exec`**：本片测试里「宿主」是测试进程自写的最小应答器，只锁定**帧形状**；真实 `HostPeer` 侧的 `declared_calls` 准入、`RemoteTool` 命名前缀与 `ToolExecReply` 组装未在本片执行。⇒ §75 未做 3 的「启用路径端到端验证」**仍未关闭**，不得由本片推定已通。
2. **产物未重打**：`dist/plugins/stock/` 里仍是 §75 的骨架二进制（sha256 `97ac4c4e…c6cd`），重跑 `plugin:bundle` 后哈希必变；本片未重打也未入库（`dist/` 已忽略）。
3. **未接 T2 命令面**：`commands[]` 仍为空数组，插件目前只贡献工具，不贡献命令。
4. **读数不降**（计划内）：ownedRe 桶仍 123 处 / 22 文件，本片不动。

## 77. `#35` B4-d：首刀的真宿主端到端（关闭 §76 未做 1）（2026-11-04）

**接的是 §76 未做 1**：§76 的 6 条测试里「宿主」是测试进程自写的最小应答器，只锁帧形状；本片补上**宿主侧真实实现**跑一遍。

**交付**：新测试 `axagent-plugins/crates/plugins/tests/e2e_stock_tool_test.rs`（真 `LoadedPlugin` + 真 worker 进程 + 假行情 provider），一条用例覆盖全链：

`LoadedPlugin::load`（spawn → describe → 校验 `calls` → 注册 `tool.set`）→ `registry.get_tool_set()` 取远程工具集 → 断言 `get_stock_quote` 的 `category` / `inputSchema.required` → `Tool::call({"stock_code":"600519"})` → worker `call_seam("market.quote","get_quote")` → 宿主 `HostPeer::dispatch_seam` → `FixedQuoteProvider.get_quote` → `content` / `metadata.stock_code` 回到宿主 → `drop(loaded)` 回滚 `tool.set`、`market.undo()` 回滚行情替身。

**为什么行情用替身**：真实 provider 要联网取数，不能进单测；本片验的是**接线**（谁的 provider 被调到、数据是否原样回到宿主），取数正确性属 provider 自身测试。

**门控与取路径**：`AXAGENT_STOCK_WORKER_BIN=<worker 可执行文件>`，未设则打印 SKIP 并说明「未通过、未被验证」（沿用 `AXAGENT_TEST_PLUGIN_SUBPROCESS` 口径）。不用 `CARGO_BIN_EXE_*` 的原因：`plugin-stock` 是另一个 package，该变量只对本 package 的 bin 生效；本机 `CARGO_TARGET_DIR` 又指向 `~/.cargo/target-axagent`，写死 `target/` 找不到 ⇒ 由调用方给路径（推荐指向 `dist/plugins/stock/bin/`）。

**验收（2026-11-04 实测，日志 `.tmp/t35b4d-{e2e,clippy}.log`）**：

| 标签 | 命令（cwd = `axagent-plugins`） | EXIT | 取证 |
|---|---|---|---|
| D1 | `AXAGENT_STOCK_WORKER_BIN=<dist 绝对路径> cargo test -p axagent-plugins --test e2e_stock_tool_test -- --nocapture` | 0 | **1 passed / 0 failed**（0.13–0.20s）：真实 `LoadedPlugin` + 真实子进程，`content.price=1500.25`、`code=600519`、`metadata.stock_code=600519`；卸载后两条接缝均回滚 |
| D2 | 同上但不设环境变量 | 0 | 打印 SKIP 三行（明示未验证），不算通过 |
| D3 | `cargo clippy -p axagent-plugins --all-targets -- -D warnings` | 0 | Finished 16.43s，零告警 |
| D4 | `rustfmt --edition 2024 --check`（1 个文件） | 0 | 首轮 1 处 diff（`assert_eq!` 参数换行），`rustfmt` 修正后零 diff |

K1 不变（**2033**）：新文件落在 `tests/`，被 `K1_IGNORE` 排除。

**本片踩到的两个坑（写下来省下一次）**：
1. **`EffectHandle` 没有 `Drop`**：`drop(handle)` 不会回滚，必须显式 `.undo()`。首版断言「drop 后应回滚」当场红，真因在测试写法而非产品代码。
2. **装载必须在 tokio 运行时上下文内**：`set_declaration` 用 `Handle::try_current()` 取运行时，拿不到 ⇒ 装载照样成功，但 worker 回调 `market.quote` 会以「无运行时」失败（错误出现在调用期，不在装载期，容易误判成协议问题）。
3. 顺带一条：bash 里 `\\\\vmware-host\\...` 传给环境变量会塌成单反斜杠 `\vmware-host\...`，Windows 上按 UNC 解析不了；用正斜杠 `//vmware-host/...` 或单引号原样传。

**未做（逐条点名）**：
1. **未走 `PluginManager` 的 enable 路径**：本片用 `PluginWorkerConfig` + **绝对** program 路径，没验「清单 `worker.program` 相对路径 → `resolve_worker_program` → 权限从 manifest 注入」这一段；那是 §75 未做 3，仍开着。
2. **CI 跑不到**：CI 没有先跑 `plugin:bundle` 的步骤 ⇒ 该用例在 CI 上走 SKIP，**不是回归护栏**；要变成护栏得把「建产物 + 设环境变量」接进 CI，属发布链路（与 §75 未做 2 同族）。
3. **产物已重打**：`node scripts/build-plugin-bundle.mjs` 重新打包（`dist/` 忽略不入库），sha256 由 `97ac4c4e…c6cd` 变为 `fcf24fe7…6111` —— 反映 B4-c 的代码变化，非新问题。
4. **读数不降**（计划内）：ownedRe 桶仍 123 处 / 22 文件。

## 78. `#35` B4-e：股票插件的 T2 命令面（`stock_get_quote`）（2026-11-04）

**接的是 §76 未做 3**：插件此前只贡献工具，`commands[]` 为空 ⇒ 「命令能不能经 worker 跑」在真实插件上没验过。本片补上命令通道。

**交付**（3 处改动，无新增 `.rs`）：
1. `crates/plugin-stock/src/main.rs`：新增 `ops::COMMAND` 分支 —— 用 `CommandFrame::from_frame_request` 解码；**身份自检**（帧 `plugin_id` 必须等于 spawn 注入的 `AXAGENT_PLUGIN_ID`，不符回 `PLUGIN_COMMAND_ID_MISMATCH`）；按命令名分发，未声明名回 `PLUGIN_COMMAND_UNKNOWN`；成功回 `CommandReply`、失败回 `CommandFault`（都用 proto 的现成构造器，不自造 JSON 形状）。
2. **取数逻辑不写第二遍**：抽出 `fetch_quote(input, call_chain, peer) -> (stock_code, quote)`，工具 `exec` 与命令面共用同一条 `market.quote.get_quote` 出站路径（入站 `call_chain` 原样透传，环检测才收得到这条边）。
3. `scripts/build-plugin-bundle.mjs` 的清单补 `commands[]`：`name` / `description` / `inputSchema.required=["stock_code"]`，`command` 填 `worker://stock_get_quote` 占位（worker 通道按 `name` 发帧；占位串保证一旦意外退回 spawn 通道会立刻失败而不是跑别的东西）。

**验收（2026-11-04 实测）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| E1 | `cargo test -p axagent-plugin-stock`（cwd=`axagent-plugins`） | 0 | **8 passed / 0 failed**（0.03s）：原 6 条 + ①命令帧全链（先出 `call_seam` 再回 `CommandReply`，`plugin_id`/`command` 回填、行情原样透出）；②身份不符 → `PLUGIN_COMMAND_ID_MISMATCH` 且**未发起**行情调用、未知命令 → `PLUGIN_COMMAND_UNKNOWN` |
| E2 | `AXAGENT_STOCK_WORKER_BIN=<dist 路径> cargo test -p axagent-plugins --test e2e_stock_tool_test` | 0 | **1 passed**（0.14s）：同一条 worker 连接上 `LoadedPlugin::execute_command` 跑通命令面，`reply.value.price=1500.25` |
| E3 | `node scripts/build-plugin-bundle.mjs` | 0 | 清单含 1 条命令；sha256 `d43e6379…bdd8`（随代码变化，`dist/` 忽略不入库） |
| E4 | `cargo clippy -p axagent-plugin-stock --all-targets -- -D warnings` / `-p axagent-plugins …` | 0 / 0 | 两个 crate 各自零告警 |
| E5 | `rustfmt --edition 2024 --check`（3 个 `.rs`） | 0 | 零 diff（`scripts/**` 在 dprint 排除面内，本批 JS 不受 format:check 管辖 —— 这是既有配置，不是本片放宽） |

K1 不变（**2033**）：改的都是既有文件。

**踩坑（写下来省下一次）**：`CommandFrame` / `CommandReply` **没有 `rename_all`**，线格式是 **snake_case**（`plugin_id`），不是 camelCase。首版按 `pluginId` 断言，回读为 `Null` 当场红 —— 与 `ToolExecReply` / `RemoteToolDescriptor`（那两个是 camelCase）**不是同一套**，记混就会写出假断言。

**未做（逐条点名）**：
1. **未走 `PluginManager::execute_plugin_command`**：本片用 `LoadedPlugin::execute_command` 直连 worker，绕过了「manifest 授权门槛（`requiredPermission`）+ `inputSchema` 最小形状校验 + 命令超时预算」这一层；那层的端到端用例在 `e2e_command_frame_test.rs`（夹具 worker），**没用在真实股票插件上**。
2. **没有真实股票命令被迁移**：`stock_get_quote` 只是把已有 `market.quote` 能力以命令形态再暴露一次，不含任何域逻辑；304 条宿主股票命令一条未动。
3. **未接前端**：没有页面/调用方使用这条命令 ⇒ 属于「通道可用」而非「功能上线」。
4. **读数不降**（计划内）：ownedRe 桶仍 123 处 / 22 文件。

## 79. `#35` B4-f：真安装链路（install → enable → 命令）+ 一个真缺陷（2026-11-04）

**接的是 §75 未做 3 / §77 未做 1 / §78 未做 1**：此前所有验证都绕开了 `PluginManager` —— 用的是 `PluginWorkerConfig` + **绝对**路径，清单的 `worker.program` 相对路径、`permissions` 推沙箱、`commands[]` 授权门槛这三段没被走过。

**本片查出的真缺陷**：§75 生成的清单写的是 `permissions: []`，而宿主 `build_sandbox_from_permissions` 从 **manifest** 推沙箱（不是 `PluginWorkerConfig::new` 的 permissive 默认值）⇒ 产物装得上、`enable` 也返回成功，**一执行就被子进程权限闸拒**：`PermissionDenied("插件未声明 subprocess_execution 权限…")`。已修：清单改为 `permissions: ["subprocess_execution"]`（worker 由宿主 spawn，这是必需项；网络仍不声明，取数走宿主接缝）。

> 附带一条判据：**`enable` 绿 ≠ 能跑**。A/B 反证里 enable 是成功的，错误直到 `execute_plugin_command` 才出现 —— 装机后自检必须打一条命令，不能只看「已启用」。

**交付**：
1. `scripts/build-plugin-bundle.mjs`：清单 `permissions` 补 `subprocess_execution`（含注释说明为什么不能省）。
2. 新测试 `axagent-plugins/crates/plugins/tests/e2e_stock_bundle_install_test.rs`：`PluginManager::install(<产物目录>)` → 断言安装目录里 `bin/` 被一起拷走 → `enable` → `execute_plugin_command("stock_get_quote")` → 断言 `price` / `code`。
3. 行情替身下沉 `tests/common/mod.rs` 的 `FixedQuoteProvider`，两个 e2e 用例共用（不再各写一份）。

**验收（2026-11-04 实测）**：

| 标签 | 命令（cwd = `axagent-plugins`，除注明外） | EXIT | 取证 |
|---|---|---|---|
| F1 | `AXAGENT_STOCK_BUNDLE_DIR=<dist/plugins/stock> cargo test -p axagent-plugins --test e2e_stock_bundle_install_test` | 0 | **1 passed**（0.27s）：install（整目录拷贝含 `bin/`）→ enable（相对 `worker.program` 解析成功）→ 命令执行拿到 `price=1500.25` / `code=600519` |
| F2 | 同上，但把产物复制到 `.tmp/stock-noperm` 并清空 `permissions` | 1（**预期红**） | 失败点 `execute_plugin_command`：`PermissionDenied("插件未声明 subprocess_execution 权限…")` ⇒ 证明该权限是硬前提，不是装饰 |
| F3 | `AXAGENT_STOCK_WORKER_BIN=<dist 的 worker> cargo test -p axagent-plugins --test e2e_stock_tool_test` | 0 | **1 passed**（0.15s）：改用共享替身后原用例仍绿 |
| F4 | `cargo clippy -p axagent-plugins --all-targets -- -D warnings` | 0 | Finished 1.52s，零告警 |
| F5 | `rustfmt --edition 2024 --check`（3 个 `.rs`） | 0 | 零 diff |

K1 不变（**2033**）：新文件在 `tests/`（被 `K1_IGNORE` 排除），其余为既有文件改动。

**未做（逐条点名）**：
1. **CI 仍跑不到**：F1/F3 依赖先打包的产物，CI 没有「先 `plugin:bundle` 再设环境变量」的步骤 ⇒ 这两个用例在 CI 走 SKIP（同 §77 未做 2）。
2. **未验真实用户目录升级路径**：config home 用临时目录，没跑「旧版本已装 → 更新 → registry 迁移」。
3. **清单 `integrity` 与宿主 trust-on-install 哈希不是同一值**：前者是打包脚本对 worker 单文件的 sha256，后者是宿主对**整个安装目录**算的（`hash_plugin_directory`），两者各自独立，本片未尝试对齐。
4. **前端仍未接**（同 §78 未做 3）。
5. **读数不降**（计划内）：ownedRe 桶仍 123 处 / 22 文件。

## 80. `#35` B4-g 开工决策：纯域逻辑的家（2026-11-04，只决策不搬代码）

**为什么先停下来定这件事**：B4-a ～ B4-f 交付的都是「通道 + 装机链路」，一条域逻辑未搬。往下每一步都要回答同一个问题：**插件只能依赖 foundation（`plugin-proto` 级别），而现有股票域逻辑在 implementor（`analysis-engine` / `astock-data`）里，两边够不着彼此**。不先钉死落位，搬一条就要改一次口径（§54.3 的「−6～−9」就是这么推翻的）。

**三个候选与裁定**：

| 候选 | 裁定 | 理由 |
|---|---|---|
| 插件直接依赖 `analysis-engine` | ✖ | implementor 会把 `dao` / `entities` / `astock-data` 拖进插件进程（铁律 2 + 插件不得依赖实现层）；且 `analysis-engine` 是 `astock-data` 的上游，搬它等于先推翻 §64.11 裁定 1（`astock-data` 留宿主） |
| 把纯逻辑塞进 `harness` | ✖ | `harness` 的定位是「trait 契约 + 纯 DTO」，塞业务解析器会让所有 consumer 共享一份业务代码，且与「股票迁出后主壳不再链接股票逻辑」的终局相反 |
| **新建插件侧 foundation crate `axagent-stock-core`**（`axagent-plugins/crates/stock-core`） | ✔ | 零 `axagent-*` 依赖 ⇒ 插件可直接链接；宿主过渡期也可链接；终局随插件走（宿主断开依赖即完成迁出）。依赖方向是 implementor → foundation，向下，不构成环 |

**登记要求**（搬之前必须同时做，缺一条就是半成品）：
1. `axagent-plugins/crates/README.md` 与 AGENTS.md 附区登记角色 = **foundation**（零 `axagent-*` 依赖），并写明「插件与宿主过渡期共用」；
2. 分层门禁规则 4：由 `plugin-stock`（bin）与 `analysis-engine` 各引用一次 ⇒ 有消费者；
3. K1 会变：新增 `stock-core/src/lib.rs` + 迁入模块（+2），`analysis-engine/src/intent_parser.rs` 删除（−1）⇒ **净 +1（2033 → 2034）**；同批改 `scripts/capability-register.json` 的 `k1RsFileCount`（`value` / `planNumber` / `planPinned.mustContain`）与 PLAN §0.7.1 的读数说明；
4. `analysis-engine` 侧留**路径级 re-export**（`pub use axagent_stock_core::intent_parser;`）⇒ 宿主 `stock_analysis.rs` 的 `axagent_analysis_engine::intent_parser::ParsedIntent` 一行不用改，主 crate 无需重编即可保持兼容（迁移期不制造第二套类型）。

**首片选 `intent_parser`（已核过的画像）**：441 行、**只依赖 `serde`**、单一宿主消费方（`commands/stock_analysis.rs:7696` 的 `parse_analysis_intent` 命令入口）、自带 9 条单测、无 DB / 无时钟 / 无 AppState —— 正是 §76 选首刀时用的同一档判据，也是 §49 夹具 worker 注释里点名的「真迁移时直接依赖本体」那个模块。
**首片验收口径**（照 §76 的 A1–A4 形态）：`cargo test -p axagent-stock-core`（迁入的 9 条原样通过）+ `cargo test -p axagent-analysis-engine`（re-export 后原测试仍指向同一实现）+ 插件新增命令 `parse_analysis_intent` 的真进程用例（用**本体**而非夹具子集）+ `cargo check -p axagent --lib`（证明主 crate 不用改）。

**本片明确不做**：不搬 `astock-data`（§64.11 裁定 1 保留）、不删宿主 `parse_analysis_intent` 命令（commandTotal 不变）、不动 ownedRe 读数（123 / 22）。

## 81. `#35` B4-g 落地：`intent_parser` 迁入 `axagent-stock-core`（2026-11-04）

**按 §80 的裁定执行**：纯域逻辑放插件侧 foundation crate，宿主与插件链接**同一份实现**。

**交付**：
1. 新 crate `axagent-plugins/crates/stock-core`（`axagent-stock-core`，foundation，仅依赖 `serde`）：`src/lib.rs` + `src/intent_parser.rs`（441 行原样迁入，含原有 9 条单测；模块头补归属说明）。
2. `axagent-quant/crates/analysis-engine`：删 `src/intent_parser.rs`，`lib.rs` 改为**路径级 re-export**（`pub use axagent_stock_core::intent_parser;`）⇒ 宿主 `commands/stock_analysis.rs` 的 `axagent_analysis_engine::intent_parser::…` **一行未改**，类型仍是同一个（不造第二套 DTO）。
3. `plugin-stock`：依赖 `axagent-stock-core`，新增命令 `parse_analysis_intent`（`input` 必须是字符串，成功回 `ParsedIntent` 的 camelCase JSON，失败回 `PLUGIN_COMMAND_INVALID_ARGS`）；打包清单 `commands[]` 补该条（`required: ["input"]`）。
4. 角色登记：`axagent-plugins/crates/README.md` 增 `stock-core` 行（foundation，零 `axagent-*` 依赖）。

**验收（2026-11-04 实测）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| G1 | `cargo test -p axagent-stock-core`（cwd=`axagent-plugins`） | 0 | **9 passed**：迁入的 9 条原单测原样通过（同一批词表/判据） |
| G2 | `cargo check -p axagent-analysis-engine --all-targets`（cwd=`axagent-quant`） | 0 | Finished 2m39s；re-export 后原路径可用 |
| G3 | `cargo test -p axagent-plugin-stock`（cwd=`axagent-plugins`） | 0 | **9 passed**（原 8 + 新命令用例：`分析宁德时代中线` → `stockCode=300750` / `timeHorizon=mid` / `actionType=research`，且**不向宿主发任何帧**） |
| G4 | `cargo check -p axagent --lib`（cwd=`src-tauri`） | 0 | Finished **13m29s**：主 crate 无需改动（证明 re-export 兼容） |
| G5 | `cargo clippy -p axagent-stock-core -p axagent-plugin-stock --all-targets -- -D warnings` / `-p axagent-analysis-engine …` | 0 / 0 | 前者零告警；后者 Finished 41.65s |
| G6 | `node scripts/check-capability-register.mjs`（仓根） | 0 | K1 **2033 → 2034**（+2 新增库文件、−1 删除）；`pub trait` 仍 324；登记表 `value` / `planNumber` / `planPinned` 与 PLAN §0.7.1 同批更新 |
| G7 | `rustfmt --edition 2024 --check`（5 个 `.rs`） | 0 | 零 diff |

**踩坑**：给新 crate 的 `Cargo.toml` 首行写了 `// SPDX…`（照 `.rs` 的习惯）⇒ cargo 直接报 `key with no value, expected '='`，整个 workspace 成员加载失败。TOML 注释是 `#`。

**未做（逐条点名）**：
1. **宿主命令未删**：`commands/stock_analysis.rs:7696` 的 `parse_analysis_intent` 仍在，`commandTotal` 1543 不变 —— 本片是「两份调用方、一份实现」，真正的「迁出」还差「前端改走插件命令 + 删宿主命令」那一步（会动 commandTotal 与前端，需单独立项）。
2. **AGENTS.md 角色未登记**：该文件当前有**他人未提交改动**，本批不碰；已在 `axagent-plugins/crates/README.md` 登记（AGENTS 允许二者之一）。
3. **夹具 worker 的子集词表未删**：`crates/plugins/src/bin/e2e_command_frame_worker.rs` 的移植副本仍在，属 #49 夹具；删它要同步改 `e2e_command_frame_test`，另批。
4. **读数不降**（计划内）：ownedRe 桶仍 123 处 / 22 文件；宿主仍链接 `analysis-engine`。

## 82. `#35` B4-h：内置插件交付的运行时一半（资源目录 + 暂存入口）（2026-11-04）

**这一批为什么先做交付而不是切调用方**：要删宿主命令，就得先保证「用户机器上有插件可用」。而查下来发现一个**静默失效**：

> `PluginManager::bundled_root()` 的默认值是 `env!("CARGO_MANIFEST_DIR")/bundled` —— 那是**编译机**的源码路径，被打进二进制后在用户机器上不存在；`discover_plugin_dirs` 对 `NotFound` 返回**空表**（不报错）⇒ **发布版的内置插件恒为零，且没有任何日志**。§75/§77 说的「产物没接发布链路」根因就在这里。

**交付**（3 处）：
1. `PluginManagerConfig::resolve_bundled_root(Option<&Path>)`：优先 `<resource_dir>/plugins/bundled`，不存在则退回源码树（开发期语义不变）；带 3 条单测（命中 / 资源目录无 plugins / 无资源目录）。
2. 新入口 `create_app_state_with_resource_dir(db_result, Option<PathBuf>)`；原 `create_app_state` 改为**委托 None** —— 两个离线 bin 与 `server_main` 三个非 Tauri 调用方一行不改。`lib.rs` 的 `setup` 传 `app.path().resource_dir().ok()`。
3. `npm run plugin:bundle:stage`：把 worker 产物摆到 `src-tauri/resources/plugins/bundled/stock`（`plugin.json` + `bin/<exe>`），该路径进 `.gitignore`（二进制不入库）。

**验收（2026-11-04 实测）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| H1 | `cargo test -p axagent-plugins --lib bundled_root` | 0 | **4 passed**（含既存同名用例），226 filtered out |
| H2 | `cargo check -p axagent --lib`（cwd=`src-tauri`） | 0 | Finished **17m05s**（lib.rs + state.rs 改动） |
| H3 | `cargo clippy -p axagent-plugins --all-targets -- -D warnings` | 0 | Finished 25.41s |
| H4 | `rustfmt --edition 2024 --check`（3 个 `.rs`） | 0 | 首轮 state.rs 1 处换行 diff，修正后零 diff |
| H5 | `node scripts/build-plugin-bundle.mjs --out src-tauri/resources/plugins/bundled/stock` | 0 | 产物落位；`git status` 命中 **0** 条（确认被忽略） |

**未做（逐条点名）**：
1. **`tauri.conf.json` 未接**：`bundle.resources` 没加 `resources/plugins`、`beforeBuildCommand` 也没跑暂存 ⇒ **发布版此刻仍不带插件**。为什么不顺手接：`beforeBuildCommand` 同时服务 `tauri android/ios build`，在桌面机上跑的 worker 产物对移动端是**错架构**，需要 target 守卫后再接 —— 属发布链路单独立项。
2. **资源目录分辨率未在真打包产物里验**：只在单测里覆盖（临时目录造 `plugins/bundled`），真安装包验证要等第 1 条接上。
3. **前端未切插件命令、宿主命令未删**：`StockSearchBar.tsx:118` 仍调 `parse_analysis_intent`，`commandTotal` 仍 1543。
4. **读数不降**（计划内）：ownedRe 桶仍 123 处 / 22 文件。

## 83. `#35` B4-i：发布链路接通（暂存落点 + resources 声明 + 移动端守卫）（2026-11-04）

**这一批关掉 §82 未做 1/2**：`bundle.resources` 与 `beforeBuildCommand` 接上，并证明落点与宿主解析路径逐字对齐。

**落点为什么是 `src-tauri/plugins` 而不是 `src-tauri/resources/plugins`**（§82 H5 用的是后者，本批修正）：Tauri 打包 resources **保留相对 `tauri.conf.json` 的路径**。依据不是文档而是本机源码 `tauri-utils 2.9.3/src/resources.rs:17`（`resource_relpath` 逐段拼）与 `:228-231`（数组形态分支直接 `resource_relpath(&path)`）—— 声明 `"plugins"` ⇒ `$RESOURCE/plugins/...`；声明 `"resources/plugins"` ⇒ `$RESOURCE/resources/plugins/...`，比 `resolve_bundled_root` 期望的 `<resource_dir>/plugins/bundled` 多一层，宿主必然找不到。

**交付（5 处）**：
1. 新 `scripts/stage-plugin-bundles.mjs`：读 `TAURI_ENV_PLATFORM` / `TAURI_ENV_TARGET_TRIPLE`，命中 android/ios 打印跳过并退 0（桌面 `cargo build` 编出的是宿主架构，打进移动包无用且白增体积）；无 `TAURI_ENV_DEBUG` ⇒ `--release`（手工直跑按「想产出产物」处理）；落点 `src-tauri/plugins/bundled/stock`，转调 `build-plugin-bundle.mjs`。
2. `tauri.conf.json`：`beforeBuildCommand` = `npm run build && node scripts/stage-plugin-bundles.mjs`；`bundle.resources` = `["pricing.toml", "plugins"]`。
3. `.gitignore` 改 `src-tauri/plugins/`（二进制不入库，构建前生成）；`package.json` 的 `plugin:bundle:stage` 落点同步改。
4. `scripts/build-plugin-bundle.mjs`：修 `--check` 自检的挂死（见踩坑）。
5. 删掉本批早前按旧落点生成的 `src-tauri/resources/plugins/`（含一个 exe），旧目录不再有任何引用。

**验收（2026-11-04 实测）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| I1 | `node scripts/stage-plugin-bundles.mjs`（无 `TAURI_ENV_*`，按桌面处理） | 0 | release 构建 13.69s；产物 `src-tauri/plugins/bundled/stock/{plugin.json,bin/axagent-plugin-stock.exe}` |
| I2 | `git status --porcelain src-tauri/plugins` + `git check-ignore -v …/plugin.json` | — | **0 条**；命中 `.gitignore:44:src-tauri/plugins/` |
| I3 | `TAURI_ENV_PLATFORM=android node …` / `TAURI_ENV_TARGET_TRIPLE=aarch64-apple-ios node …` | 0 / 0 | 两条都打印移动端跳过，**未触发 cargo** |
| I4 | `node scripts/build-plugin-bundle.mjs --out src-tauri/plugins/bundled/stock --release --check` | 0 | describe 握手 + shutdown 回执 + 退出码 0 + sha256 `e00d8e28…` 一致 |
| I5 | `dprint.exe check --config <ABS> tauri.conf.json package.json scripts/stage-plugin-bundles.mjs` | 0 | 零 diff（UNC 下必须显式 `--config`，见 §72） |
| I6 | `node -e` 读 `tauri.conf.json` | 0 | `resources=["pricing.toml","plugins"]`、`beforeBuildCommand` 含暂存脚本 |

**踩坑（自检脚本自己挂死）**：`--check` 首跑永不返回 —— `shutdown` 在帧协议里只是**回执**，worker 的 `serve` 循环靠 **stdin EOF** 退出（`plugin-stock/src/main.rs:52` 注释逐字这么说），而脚本发完 `shutdown` 就 `await child.on("exit")`、不关 stdin ⇒ 双方互等（实测 worker 进程常驻、日志停在 sha256 那一行）。修法：收到回执后 `child.stdin.end()`，退出等待加 10 s 上限，并 `process.on("exit", () => child.kill())` 防 `fail()` 泄漏子进程。**判据：「自检通过」先得证明自检自己能退出**，否则它测的只是挂起。

**未做（逐条点名）**：
1. **未跑真 `tauri build`**：资源落位只在源码层对齐（I6 + 落盘），安装包里 `$RESOURCE/plugins/bundled` 尚未解包验证（Windows 上整包 >30 min，且需签名环境）。
2. **移动端真机未验**：只验了守卫分支不跑 cargo，未验 `tauri android/ios build` 全链路。
3. **前端未切插件命令、宿主命令未删**（B4-j / B4-k）：`StockSearchBar.tsx:118` 仍调宿主 `parse_analysis_intent`，`commandTotal` 仍 1543。

## 84. `#35` B4-j：前端 NL 意图解析改走插件命令（2026-11-04）

**为什么与 B4-k 同交付**：`StockSearchBar` 是宿主 `parse_analysis_intent` 的**唯一**调用方（全仓 grep 确认）。只切前端不删宿主 = 两份实现并存；只删宿主不切前端 = 功能直接消失。两批同提交，中间态不存在（关闭 §83 未做 3 的前半）。

**交付（5 处）**：
1. `src/lib/pluginCommandResolve.ts`：按命令名从 `plugin_list` 反查**启用中**插件的 id（不写死 `stock@bundled` —— 安装来源/改名都会让它静默失效）；结果按命令名缓存 —— `plugin_list` 每次都跑一轮内置插件落盘同步（`sync_bundled_plugins`），而这里在搜索框每次回车都会命中。枚举失败**不缓存**（宿主未就绪时下次重试）。
2. `StockSearchBar.tsx`：`invoke("parse_analysis_intent")` → `plugin_execute_command{pluginId,name,input}`；插件未装 / 未启用 / 调用失败一律退回普通搜索（与「解析不出代码」同一处置，不新增提示文案）。
3. 打包清单 `defaultEnabled: false → true`：切换后内置包不默认启用，发布版里这条功能就是**静默失效**（与 §79 的 `subprocess_execution` 同类：清单缺一项，enable 绿 ≠ 能用）。
4. 开发期落点 `npm run plugin:bundle:dev` → `axagent-plugins/crates/plugins/bundled/stock`（`PluginManager::bundled_root()` 的编译期默认值）。`tauri dev` 的 resource 目录里没有插件，这是开发态唯一能加载到 worker 的路径；该目录进 `.gitignore`。
5. 回归 6 条：`src/lib/__tests__/pluginCommandResolve.test.ts`（4 条：启用过滤 / 缓存 / 无命中 / 失败不缓存）+ `StockSearchBar.pluginIntent.test.tsx`（2 条：命中插件→填 store + 取行情；命令失败→退回普通搜索）。

**验收（2026-11-04 实测）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| J1 | `node scripts/vitest-run.mjs run src/lib/__tests__/pluginCommandResolve.test.ts` | 0 | **4 passed**（243 ms） |
| J2 | `node scripts/vitest-run.mjs run src/components/stock-analysis/__tests__/StockSearchBar.pluginIntent.test.tsx` | 0 | **2 passed**：`plugin_execute_command` 收到 `{pluginId:"stock@bundled", name:"parse_analysis_intent", input:{input:…}}`，store 得到 `300750` 并取行情；失败分支退回 `searchStock("茅台", true)` |
| J3 | `node scripts/check-tsc.mjs` | 0 | 文件发现数 **1463**（含新增 2 个） |
| J4 | `node node_modules/dprint/bin.cjs check --incremental=false`（本批 8 个文件） | 0 | 零 diff |

**踩坑（都是测试替身的问题，不是产品缺陷）**：
- `userEvent.type(input, "…{enter}")` 在这个受控输入上**不触发** `onSearch`（实测两次调用数都是 0）⇒ 改 `fireEvent.change` + `fireEvent.keyDown(Enter)`（antd 的 `onPressEnter` 挂在 keyDown 上）。
- `onChange` 本身会调 `searchStock(value)`（**单参**），所以「没走回退分支」只能用**双参**形态断言；按「完全没调用」断言必假红。
- 空闲期之外跑 vitest 会撞 #141 的 SMB worker 启动超时（本轮首跑即 `Timeout waiting for worker to respond`，与代码无关；cargo 空闲后重跑即绿）。

**未做（逐条点名）**：
1. **桌面真机未验**：没有带插件的安装包，也没在 `tauri dev` 里跑过「启用插件 → 回车 → 解析」全链路。浏览器 mock 的 `plugin_execute_command` 返回空串 ⇒ 该模式下 NL 解析与改造前一样不可用（改造前它调的宿主命令在 mock 里也没有实现）。
2. **反查缓存不感知运行期禁用**：会话内首次命中后，用户在插件页禁用该插件，下一次解析仍会拿旧 id → 命令报错 → 退回普通搜索（不崩、不误导），但不会自动重查。是否加失效广播，等真实反馈再定。
3. **开发期需手工暂存**：`plugin:bundle:dev` 不进 `beforeDevCommand`（那会把 cargo 构建塞进每次 dev 启动）。

## 85. `#35` B4-k：删宿主同名命令 + 断开 `analysis-engine → stock-core`（2026-11-04）

**这一批把 B4-j 的切换收尾**：宿主 `parse_analysis_intent` 已无调用方（全仓 grep：只剩它自己的定义与 `register_commands.rs` 的产物行），删除后主壳不再借道 `analysis-engine` 链回股票域逻辑。

**交付（4 处）**：
1. 删 `src-tauri/src/commands/stock_analysis.rs` 的 `parse_analysis_intent`（含 `#[agent_command]` 段与 `── P2-1: NLU 意图解析 ──` 小标题，共 −13 行）。`register_commands.rs` 由 `build.rs` 重生成（**不手改**，禁区 7）：−1 条目、行数 1610 → 1609。
2. `axagent-quant/crates/analysis-engine`：删 `pub use axagent_stock_core::intent_parser;` 与 `Cargo.toml` 的 `axagent-stock-core` 依赖。理由不是洁癖 —— 留 re-export 等于「主壳仍经 analysis-engine 链接股票域 foundation」，与迁出方向相反；消费方现在只剩 `plugin-stock`（同一 workspace，路径依赖照旧）。
3. `axagent-plugins/crates/plugins` 的夹具 worker（#49 的 `e2e_command_frame_worker.rs`）：把移植的 `HOT_STOCKS` / `HORIZONS` / `extract_*` / `parse_intent`（**约 70 行重复词表**，禁区 12）整段删除，命令改为 `echo_input`（回显 `text` 与字符数）。夹具的职责是证明**帧通道**（pluginId 绑定、形状校验先于派发、超时、错误码透传），股票解析的端到端已由 §77/§79 的真插件用例覆盖 —— 两件事不该用一份复制词表粘在一起。同步改 `e2e_command_frame_test.rs` 的 manifest 与 4 条断言。
4. 读数同批改：`commandTotal` 1543→**1542**、`commandNameSetParity` 1543→**1542**、`stockBoundaryCommands` 304→**303**、`stockAuditDomainCommands` 306→**305**、`registryFileLines` 1610→**1609**、`frontendFileCaliber` 85 009→**85 164 / 295 文件**、`frontendDirCaliber` 60 757→**60 912 / 212 文件**（后两条来自 B4-j 的前端改动；首轮估值 85 152 / 60 900 是拿改测试**之前**的 130 行算的，实测 142 行 ⇒ 门禁差 12 行当场抓出，见 §86）；PLAN §0.8.2 / §0.10.1 的现值句同批改。其余「304 条」是**包级口径**（宿主 303 + 插件 1 条已迁走的 `parse_analysis_intent`），不逐处改写。

**验收（2026-11-04 实测）**：

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| K1 | `cargo check -p axagent --lib`（cwd=`src-tauri`） | 0 | Finished **21m30s**；`register_commands.rs` 已重生成：`split('\n')` 行数 **1609**、`parse_analysis_intent` **零命中** |
| K2 | `AXAGENT_TEST_PLUGIN_SUBPROCESS=1 cargo test -p axagent-plugins`（cwd=`axagent-plugins`） | 0 | **230 passed**（lib）+ 夹具 e2e **4 passed**（`echo_input` 往返 / 缺 `text` 派发前被拒 / 超时 / 错误码透传） |
| K3 | `cargo test -p axagent-plugin-stock` | 0 | **9 passed**（真插件命令面不受影响） |
| K4 | `cargo check -p axagent-analysis-engine --all-targets`（cwd=`axagent-quant`） | 0 | Finished 1m42s（re-export 与依赖都删净） |
| K5 | `rustfmt --edition 2024 --check`（3 个 `.rs`）+ `cargo fmt -p axagent-analysis-engine -- --check` | 0 | 零 diff |
| K6 | `node scripts/check-tsc.mjs` / `dprint check --incremental=false` | 0 / 0 | 1463 文件；8 个本批文件零 diff |

**踩坑**：`cargo check` 是**唯一**会重生成 `register_commands.rs` 的动作 —— 删命令后先跑门禁会得到「产物滞后」（`commandTotal` 与 `commandNameSetParity` 同时红、专码 `ARTIFACT-STALE`，见 §58），那不是读数写错。必须先 `cargo check` 再对账。

**未做（逐条点名）**：
1. **全量门禁未跑**：按 AGENTS「阶段内用 `cargo check` + 定点 clippy，全部阶段完成后跑一次全量」，本批只跑了定点（`cargo check` 主 crate、`analysis-engine`、插件两个 crate 的测试）；`clippy --workspace --all-targets --all-features -- -D warnings` 与 `test --workspace` 留到 #35 收尾。
2. **主 crate clippy 未单跑**：本批对主 crate 只有「删一段」的改动，用 `cargo check` 兜底；真要 clippy 全绿需 ~20 min 独占构建窗口。
3. **桌面真机 / 安装包未验**：与 §84 未做 1 同因（没有带插件的安装包）。
4. **`AGENTS.md` 角色登记未做**：`stock-core` 的 foundation 登记已进 `axagent-plugins/crates/README.md`；`AGENTS.md` 当前有**他人未提交改动**，本批不碰。
5. **ownedRe 读数不降**：123 处 / 22 文件是「主壳非域文件引用域 crate」，本批删的是域自有文件里的两行（不计入该读数）；真正的下降要等股票域文件整体搬迁（#35 后续 B 阶段）。

## 86. `#35` B4-i/j/k 的交付门禁（2026-11-04）

**第一轮：2 条硬失败，全部是「前端行数」这一族**：

```
FAIL: 锚点 frontendFileCaliber：登记 85152 行 / 实测 85164 行 / 差 12
FAIL: 锚点 frontendDirCaliber ：登记 60900 行 / 实测 60912 行 / 差 12
```

**真因**：两条读数的增量是本批自己算的（`60757 + StockSearchBar 增量 13 + 新测试行数`），而新测试在「算完行数」之后又被改过一次（`userEvent.type` → `fireEvent.change + keyDown`，见 §84 踩坑），从 130 行涨到 **142** 行 ⇒ 少算 12 行。**不是判据错、不是盘上漂移**：其余锚点（commandTotal 1542 / stockBoundary 303 / stockAudit 305 / registryFileLines 1609 / 契约 A 三方 1542/1542/1542）第一轮就全绿。

**修法（不放宽判据）**：只把两个登记值与 PLAN §0.10.1 的现值句改成实测值（85 164 / 60 912），`feRe` 与行数口径一字未动。

**第二轮（修正后）**：`node scripts/check-capability-register.mjs` **EXIT=0** —— `PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`；`FAIL:` 行 **0** 条；PLAN 对账 `planMustAppear 15 项 · planPinned 11 处 · planOld 4 项（alsoAt 健康 15 / 腐烂 0）`。

**判据（写下来防复发）**：前端体量这类「自己算出来的读数」，**必须在本批最后一次改文件之后再算**；算完到跑门禁之间若又碰了 `src/**`，重算而不是让门禁来纠。行数口径 = `split('\n').length`（含末行空元素），与 `audit-l1-blockers.mjs` 同源。

## 87. `#35` B5：MCP 直通 + 运维面收口（−8，2026-11-04）

**承诺口径（§54.3 R7）**：B5 只兑 `tools` crate 内 8 处 —— `audit.rs` 4 + `tools/astock_data.rs` 3 + `capability_pack_defs.rs` 1；`tools/opc.rs` 的 41 处归 #32（OPC），不在本批。

### 87.1 交付

1. **新增通用槽位** `axagent-runtime/crates/tools/src/domain_tools.rs`：`pub trait DomainToolExecutor`（`domain()` + `async execute(name, input) -> Result<String, String>`）+ 两段注册（`register_tool_definitions` / `register_tool_executor`）。两段分开是因为时机不同：定义必须在**工具注册表构建之前**（`UnifiedToolRegistry::new` → `init_all` 按定义建实例），执行器在**域客户端就绪后**（只在 `Tool::call` 时查）。
2. **`tools/src/tools/astock_data.rs` → `tools/src/tools/domain_mcp.rs`**：`StockMcpTool` 改为通用 `DomainMcpTool`（域标识 + schema + 按域查执行器）；`stock_mcp_tool_instances()` → `domain_mcp_tool_instances()`。重名跳过名单（`get_north_bound_flow` / `get_market_dragon_tiger` / `get_cls_flash`）保留在工具层 —— 它是「与既有内置实现重名」的兼容事实，不是域知识。
3. **`audit.rs` 豁免集合改为从槽位派生**：`read_only_data_tool_names()` 读 `domain_tools::all_definitions()`；派生逻辑抽成纯函数 `data_tool_names()` 供测试注入夹具；缓存从 `OnceLock` 改为**按 `domain_tools::revision()` 记忆化**（注册晚于首次读取也不会永远读到空集）。`MUTATING_DATA_TOOL_NAMES` 与 `is_min_interval_exempt` 转 `pub`，供域侧接线测试。
4. **`capability_pack_defs.rs`：`stock_tool_defs(names)` → `domain_tool_defs(names)`**（按已注册定义做「名字 → ToolDef」通用映射）。消费方 `opc_workflows/mod.rs` 与 `opc_capability_pack_actions.rs` 同批改名。
5. **域侧注册**（新文件 `src-tauri/src/init/stock_domain_tools.rs`，`ownedRe` 命中 ⇒ 不计 foreign）：`stock_tool_definitions()` = `stock_mcp_tools()` ∪ `industry_chain_mcp_tools()`（与拆分前逐字同源）；`register_tool_definitions()` 在 `tool.set` 接缝注册前调用（`init/state.rs`）；`register_tool_executor(AStockClient)` 在 `set_astock_client` 之后调用。执行仍走 `mcp_tools::execute_mcp_tool`（与工作流通道同一实现，禁区 12）。

### 87.2 验收读数

| 项 | 前 | 后 |
|---|---|---|
| `l1ForeignStock.hits` | 133 | **125**（−8，与承诺逐条对齐：audit 4 + astock_data 3 + capability_pack_defs 1） |
| `l1ForeignStock.files` | 26 | **23**（audit.rs / capability_pack_defs.rs 归零；astock_data.rs 整文件删除） |
| 待改写（H 桶 10/4 除外） | 123 / 22 | **115 / 19** |
| `k1RsFileCount` | 2034 | **2036**（+3 新增 −1 删除，改名不计） |
| `k1PubTraitCount` | 324 | **325**（`DomainToolExecutor`） |
| `k1ExternalImplTraitCount` | 240 | **241**（同一 trait 名只计一次） |

- `cargo check -p axagent-tools --all-targets`：**Finished，0 error**（5m18s）。
- `cargo clippy -p axagent-tools --all-targets --all-features -- -D warnings`：**绿**（首轮 `type_complexity` 1 条，加类型别名后复跑绿）。
- `cargo test -p axagent-tools --lib`：**469 passed / 1 failed**；唯一失败 `hacker_news_scanner::tests::test_fetch_latest_ids` 是**联网用例**（抓 HN API），与本次改动无交集（改动面 = `domain_tools` / `domain_mcp` / `audit` 三处；该用例只依赖网络）。
- 定向复跑：`audit` 7 passed、`domain_tools` 3 passed、`tools::domain_mcp` 4 passed。
- `cargo check -p axagent --lib`：**Finished，0 error**（20m13s）。
- `rustfmt --edition 2024 --check` 本批 6 个 Rust 文件：**EXIT=0**；`cargo fmt -p axagent-tools -- --check`：**EXIT=0**。

### 87.3 踩坑（写下来防复发）

1. **全局槽位不能直接在 `tools` crate 内做断言**：进程级槽位与并行用例串扰（一个用例注册夹具，另一个用例读缓存）。修法 = 派生逻辑抽纯函数、测试注入本地数据；真实数据源接线的断言下沉到域侧（`stock_domain_tools` 的测试：≥50 项、事故工具在豁免集合、写类名单无幽灵条目）。
2. **`OnceLock` 一次定终身是错的选择**：豁免集合第一次读发生在运行期限流检查，但「读早于注册」的代价是**永远空集且无报错**。改成按注册代号记忆化后，注册前后都正确。
3. **第五道门必须先 park PLAN 头部再跑**：直接裸跑会对 `planPinned` / `alsoAt` 报 30 条「钉子错行」（内容整体 +3 行），全是假红。按既有流程 park 掉头部 3 行后，只剩本轮真红的 4 条读数 + `l1ForeignStock` 的现值句。

### 87.4 未做（如实列）

1. **`tools/src/global_state.rs` 的 3 处 `AStockClient` 直连未动**（`GLOBAL_ASTOCK_CLIENT` 槽位 + `set/get_astock_client`）。它仍让 `tools` 认识具体客户端类型 ⇒ 属 B6/B7（客户端槽位改为 trait 对象或随接缝迁移），本批不承诺。
2. **`tools/src/tools/finance.rs:1236` 的 1 处 `get_astock_client()` 未动**：同因（api_tool 宏需要具体客户端）。
3. **`init/services.rs` 24 处、`tools/opc.rs` 41 处、`AppState` 8 处等大头未动**：按 §54.3 归后续 B 阶段（`tools/opc.rs` 明确归 #32）。
4. **全量 `clippy --workspace --all-targets --all-features` / `test --workspace` 未跑**：按 AGENTS 分层门禁策略留到 #35 收尾一次跑。
5. **未做真实 `tauri build` / 安装包行为回归**：与 §84/§85 同因（无带插件的安装包）。
6. **非 `create_app_state` 路径构造的注册表依赖注册顺序**：域定义只在 `state.rs` 装配时注册；任何**绕过 AppState** 直接 `UnifiedToolRegistry::new()` 的入口（当前仓内 11 处都在启动后运行，非启动前）若出现在启动前，股票工具会**静默缺席**（语义等同「域未安装」）。域侧测试（`init::stock_domain_tools::real_definitions_reach_the_auditor_through_the_slot`）只能守住「注册 → 派生」这一段，守不住「未注册」。若后续出现新入口，需在入口处显式调用 `register_tool_definitions()`。

## 88. `#35` B6：`AppState` 摘六个域字段 + 测试替身同步（−14，2026-11-04）

**承诺口径（§54.3）**：`AppState` 摘 6 个域字段 + 测试替身同步 = **−14**。

### 88.1 交付

1. **新增域自有持有模块** `src-tauri/src/init/stock_domain_state.rs`（`ownedRe` 命中 `init/stock*` ⇒ 不计 foreign）：`StockDomainState` 持有原 `AppState` 的六字段（`astock_client` / `stock_monitor` / `quote_watcher` / `trading_engine` / `cross_stock_aggregator` / `stock_adaptive_engine`），进程级 `OnceLock` 单例 + `init_stock_domain_state()` 注入；`impl AppState { pub fn stock(&self) -> &'static StockDomainState }` 提供零成本访问（返回 `&'static`，不引入临时 Arc 跨 `.await` 的借用问题）。
2. **`app_state.rs` 删六个字段声明**（6 处域 crate 路径随之消失）；`stock_workflow_t0_*` 两个宿主并发闸**保留**（不引用域 crate，不属本批）。
3. **`state.rs` 装配点改为一处注入**：`init_stock_domain_state(StockDomainState { … })` 放在 `Ok(AppState { … })` 之前（构造之后 `services.rs` 会立刻读这些槽位）。
4. **消费面 149 处改写**（`state.` / `app_state.` / `s.` 三种接收者）：140 处单行 + 9 处跨行链式，统一为 `state.stock().<字段>`。
5. **测试替身同步**：`conversations/compress.rs` 与 `conversations/mod.rs` 两个 `AppState` 测试构造器删除八行域字段赋值（原先各构造一份 `AStockClient` / `TradingEngine`，纯浪费）。
6. **H 桶白名单收口**：`scripts/l1-host-owned-allowlist.json` 删 `app_state.rs` 六条（字段类型行已不存在 ⇒ 保留即「锚点腐烂」），10 → 4 条 / 3 文件。

### 88.2 验收读数

| 项 | 前 | 后 |
|---|---|---|
| `l1ForeignStock.hits` | 125 | **111**（−14，与承诺逐条对齐：app_state 6 + 两个测试构造器 8） |
| `l1ForeignStock.files` | 23 | **20**（`app_state.rs` / `conversations/compress.rs` / `conversations/mod.rs` 归零） |
| H 桶 | 10 / 4 文件 | **4 / 3 文件**（仅剩 gateway 2 + browser_fetcher 1 + monitor_emitter 1） |
| 待改写（H 桶除外） | 115 / 19 | **107 / 17** |
| `k1RsFileCount` | 2036 | **2037**（+`init/stock_domain_state.rs`） |
| `k1PubTraitCount` / `k1ExternalImplTraitCount` | 325 / 241 | **325 / 241**（无新增 trait） |

- `cargo check -p axagent --lib --bins`：**Finished 18m54s / 0 error**。
- `cargo fmt -p axagent -- --check`：**EXIT=0**（首轮 5 文件 17 处 diff，全在本批改写面）。
- `node scripts/audit-l1-blockers.mjs --check-allowlist`：下面「验收」一并跑；app_state 六条同批删除后应 PASS。
- 第五道门：park 头部后跑，读数见 88.3。

### 88.3 踩坑

1. **跨行链式访问会漏改**：`state\n    .astock_client` 这类把接收者与字段拆两行的写法，单行正则一条也匹配不到（首轮漏 9 处，全在 `stock_analysis.rs` / `stock_evolution_approval.rs` / `stock_workflow/core.rs`）。验收判据要按行做：「以 `.字段` 开头的行，上一行必须是 `.stock()`」——用这条查出来的是 0 处才算干净；只看 `grep -c "\.field"` 会把带 `.stock().` 的单行与未改的跨行混在一起。
2. **H 桶白名单是「锚点」不是「常量」**：字段一摘，`app_state.rs` 六条 `anchorText` 在盘上原文里全部消失 ⇒ `--check-allowlist` 判「锚点腐烂／豁免空转」。B6 与白名单必须同批改，不能留到「下次顺手删」（§54 R5 的原话就是「名单宁可短，写长了就是假话」）。
3. **单例语义要写死**：`AppState` 原先随构造/析构自然管理这六个值的生命周期；改成进程级单例后，同进程第二次 `create_app_state` 保留首份。生产只调一次，测试里这些字段无人访问（两个测试构造器已删除对应赋值）⇒ 不影响；但必须在模块文档里写明，否则下一次「为什么测试里换了 client 不生效」会重新踩。

### 88.4 未做（如实列）

1. **`state.rs` 剩余 8 处装配未搬**：`AStockClient::new()`、L2 磁盘缓存 flush、`TradingEngine`、`StockAdaptiveEngine`（前 4 处属客户端构造，计划随域本体搬迁）、`decision_gates::refresh_from_db` 与 3 处 `opc::capability_pack::*`（属 B7 / OPC 面）。
2. **`init/services.rs` 24 处**（实时监控启动、T+0、聚合器等）：属 B7 算法接缝 / 服务装配，未动。
3. **`tools/opc.rs` 41 处**：按 §54.3 R7 归 #32（OPC），未动。
4. **全量 clippy / test --workspace 未跑**：留到 #35 收尾。
5. **真实 `tauri build` / 安装包行为回归未做**：与 §84/§85/§87 同因。
6. **`AppState::stock()` 的 `panic` 分支无单测**：注入需要真实客户端，在单测里构造会污染进程级单例（同进程其它用例看到假 client）；该分支由编译期接线 + 启动路径保证，故障形态是「启动即 panic 并点名 init 调用」，不是静默。

## 89. `#35` B7：算法引擎构造下沉 + 启动期域装配收口（−5，2026-11-04）

**承诺口径（§54.3）**：算法接缝 + P 桶随本体 = **−9**。本批**兑现 −5**，差额与原因见 89.4（不虚报终态）。

### 89.1 交付

1. **`StockDomainState::new(client, db)`**（`src-tauri/src/init/stock_domain_state.rs`）：`TradingEngine::new` 与 `StockAdaptiveEngine::new().with_db` 两个**引擎构造**下沉到域自有模块。`state.rs` 只递「客户端 + DB」，接缝注册所需同一个 `Arc` 从 `stock_domain_state.trading_engine.clone()` 取（与 jobs.store / workflow.run 同款「clone 注册、按值持有」）。
2. **`create_astock_client(l2_path)` + `DiskCacheFlush`**（同文件）：`AStockClient::new()` 与 `disk_cache::spawn_flush_loop` 收进域模块。flush 句柄包一层 newtype（只暴露 `shutdown_and_join`），主壳不再命名 `astock_data::disk_cache::FlushLoopHandle`；其余注入（browser fetcher / 快照归档 / 新闻归档）仍在装配层按原顺序链式完成。
3. **`refresh_decision_gates(db)`**（同文件）：决策否决门开关的「读 settings + 四项日志口径」整体下沉；`state.rs` 只留一行调用。失败仍只 warn、不阻断启动（与原语义逐字一致）。
4. **`state.rs` 装配段收口**：`let stock_domain_state = StockDomainState::new(...)` 上提到 `trading.engine` 注册块之前；注入点改为 `init_stock_domain_state(stock_domain_state)`，原字面量构造（含 `stock_adaptive_engine` 的 `with_db`）删除。

### 89.2 验收读数

| 项 | 前 | 后 |
|---|---|---|
| `l1ForeignStock.hits`（门禁口径 = 待改写 + H 桶） | 111 | **106**（−5：state.rs 8→3） |
| `l1ForeignStock.files` | 20 | **20**（`state.rs` 仍有 3 处 OPC 面引用，文件不归零） |
| 待改写（H 桶除外） | 107 / 17 | **102 / 17** |
| H 桶 | 4 / 3 文件 | **4 / 3 文件**（未动） |
| `state.rs` 域引用 | 8 | **3**（仅剩 `opc::capability_pack::` 三处，归 #36） |

- `cargo check -p axagent --lib`：见 89.3 的运行记录。
- `cargo fmt --all -- --check`（主 crate）：**EXIT=0**。
- 第五道门（`check-capability-register.mjs`）：park 头部三行后跑，读数见 89.3。

### 89.3 运行记录

| 命令 | EXIT | 结果 |
|---|---|---|
| `cargo check -p axagent --lib`（`src-tauri` 根） | 0 | `Finished dev profile … in 10m 31s`，0 error（`.tmp/b7-check.log`） |
| `cargo fmt -p axagent -- --check` | 0 | 无 diff |
| `node scripts/audit-l1-blockers.mjs --check-allowlist` | 0 | 登记 4 条 / 实际豁免 4 处（STOCK 4 · OPC 0），锚点逐条复核通过 |
| `node scripts/check-capability-register.mjs`（plan 头部三行 park 后） | 0 | `PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`；跑完 `cmp` 校验 `RESTORED_IDENTICAL` |

读数复算用一次性脚本（`.tmp/b7-foreign.mjs`，判据与 `audit-l1-blockers.mjs` 逐字对齐）：`foreign total: 102 files: 17` + `hostOwned 4 / 3` ⇒ 门禁口径 106 / 20。⚠ 复算脚本首版漏了「OPC-owned 文件对股票 crate 的引用应进 cross 桶」⇒ 少算 4 处，与门禁 111 对不上；修正后才与盘上一致（同 §54.1 记录的桶边界陷阱）。

### 89.4 未做（如实列，且解释 −9 → −5 的差额）

1. **`_shared_reco_pool.rs` 1 处未搬**：该文件是「手动刷新 / 荐股 cron / 时间旅行批次」三入口共用的落库入口，文档已自述「被多个命令模块共用的基础设施」并登记在 `check-layer-discipline.mjs` 的 `SHARED_INFRA_MODULES`。把它改到 `commands/reco/` 下可以让它命中 `ownedRe`、当场 −1，但那是**改路径让判据变绿**（§54 R5 禁「扩面洗绿」的同族），真正的处置是随域本体整体迁出（B4 本体面），不在本批。
2. **P 桶在 res109 时记 9 处的另外 3 处已被 B6 吸收**：res109 的 P 桶 = `stock_adaptive_engine` 4 + `trading` 4 + `_shared_reco_pool` 1，其中 `app_state.rs` 的两个字段类型行（adaptive 1 + trading 1）与 `state.rs` 里的一处（trading 1）已随 B6 的 −14 消失。⇒ B7 开工时的**可消减余额只有 6 处**（state.rs 5 + `_shared_reco_pool` 1），本批实拿 5。这不是「少做了」，是承诺数与实际余额在批次间重叠。
3. **`state.rs` 剩余 3 处 `analysis_engine::opc::capability_pack::`**：属 **OPC 面**（O 桶同族：OPC 代码物理住在股票 crate 里），按 §54 R7「股票迁移一条不许动、归 #32/#36」未动。
4. **`init/services.rs` 24 处**：实时监控启动、T+0、聚合器、荐股 cron 回调等**服务装配**，是 `services.rs` 的宿主主权段（M/H 桶），不是「引擎构造」；随 #36 的运维面接缝一并处置。
5. **`tests`/真实启动路径未跑**：本批只做构造位置迁移，无行为改动；真实 `tauri build` 与移动端真机回归仍与 §84/§85/§87/§88 同因未做。
6. **全量 `clippy --workspace --all-targets --all-features -- -D warnings` 与 `test --workspace`** 留到 #35 收尾（阶段内只跑定点 `cargo check` / `cargo fmt`）。

## 90. `#35` B7b：tools crate 最后三处域直连退役（−3 + H 桶 −1，2026-11-04）

**背景**：§87 遗留第 4 条「`tools/global_state.rs` 的 3 处 `AStockClient` 直连未动」；§88 遗留第 1 条把 `finance.rs` 的 1 处列为「随域本体搬迁」。本批把这条尾巴收掉：`tools` crate **不再认识任何股票域类型**。

### 90.1 交付

1. **五个数据 API 工具改为域工具**：`finance.rs` 的 `api_tool!` 宏与实例（`get_research_reports` / `get_concept_blocks` / `get_north_bound_flow` / `get_market_dragon_tiger` / `get_cls_flash`）删除。其中 3 个与 MCP 工具同名（原先靠 `REGISTRY_DUPLICATE_TOOLS` 跳过域侧注册），2 个是旧名。
2. **`astock-data/src/mcp_tools.rs`**：`stock_mcp_tools()` 新增 2 个旧名定义（带 `stock_code` schema，LLM 可见契约从「空 schema」升级为真 schema）；`execute_mcp_tool` 的执行分支对两对名字做别名（`get_stock_research_reports | get_research_reports`、`get_stock_concept_blocks | get_concept_blocks`）。旧名保留的硬理由：`data-quality.rhai` 判据表与历史工作流仍按旧名调用。
3. **`domain_mcp.rs`**：删除 `REGISTRY_DUPLICATE_TOOLS` 与过滤分支 —— 那三个工具现在必须由域侧注册（跳过即「工具消失」）；单测改为 `malformed_definitions_are_skipped`（只跳畸形定义，并断言同名工具**必须**在）。
4. **`global_state.rs`**：`GLOBAL_ASTOCK_CLIENT` / `set_astock_client` / `get_astock_client` 整段退役。
5. **宿主消费方改走域状态**：`gateway.rs` → `state.stock().astock_client.clone()`；`workflow_run_service.rs` → 新增的非 panic 访问口 `stock_domain_state::try_stock_domain()`（单测不跑 `create_app_state` 时返回 `None`，与旧槽位未注入同义）；`init/state.rs` 的注入调用删除。
6. **`tools/Cargo.toml` 摘掉 `axagent-astock-data` 依赖**：Cargo 构建边 8 → 7（`tools` 仅剩 `analysis-engine`，用于 `risk` 数学）。

### 90.2 验收读数

| 项 | 前 | 后 |
|---|---|---|
| `l1ForeignStock.hits`（门禁口径 = 待改写 + H 桶） | 106 | **102** |
| `l1ForeignStock.files` | 20 | **19**（`tools/src/global_state.rs` 归零；gateway 仍在 H 桶） |
| 待改写（H 桶除外） | 102 / 17 | **99 / 16** |
| H 桶 | 4 / 3 文件 | **3 / 3 文件**（仅剩 gateway 的 `HttpPollingStreamer` + monitor_emitter + browser_fetcher） |
| Cargo 构建边 | 8 | **7**（`tools → astock-data` 消失） |
| 工具面（名称集合） | 5 个由 `tools` 注册 | 同 5 个由域侧注册（3 个解除跳过 + 2 个旧名新定义） |
| `tool.set` Tool impl 站点数 | 199 | **198**（删掉 `api_tool!` 宏体内那一处文本 impl；5 个实例由 `DomainMcpTool` 承载，不新增站点） |

### 90.3 运行记录

| 命令 | EXIT | 结果 |
|---|---|---|
| `cargo check -p axagent-astock-data`（`axagent-quant` 根） | 0 | `Finished dev profile … in 1m 29s` |
| `cargo check -p axagent-tools --all-targets`（`axagent-runtime` 根） | 0 | `Finished … in 1m 45s` |
| `cargo test -p axagent-tools --lib` | 101（1 失败，既有联网用例） | `469 passed; 1 failed; 2 ignored`；唯一失败 `hacker_news_scanner::tests::test_fetch_latest_ids`（抓 HN API，与本批无交集，§87 同款） |
| `cargo clippy -p axagent-tools --all-targets --all-features -- -D warnings` | 0 | `Finished … in 39.27s` |
| `cargo check -p axagent --lib`（`src-tauri` 根） | 0 | `Finished dev profile … in 13m 18s`，0 error（`.tmp/b7b-main.log`） |
| `node scripts/check-capability-register.mjs`（plan 头部三行 park 后） | 0 | `PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`；`cmp` 校验 `RESTORED_IDENTICAL` |
| `node scripts/audit-l1-blockers.mjs --check-allowlist` | 0 | 登记 3 条 / 实际豁免 3 处（STOCK 3 · OPC 0） |

### 90.4 未做 / 风险

1. **域名与执行面的「双名」是刻意的**：`get_research_reports` / `get_concept_blocks` 与 `get_stock_*` 并存，且执行分支做别名。彻底收敛成一个名字要改 `data-quality.rhai` 判据与历史工作流模板，属行为面迁移，不在本批。
2. **非 `create_app_state` 入口构造注册表时，5 个工具与其余域工具一并不在**（§87 遗留第 6 条同源）。它们本来就需要客户端，缺席只改变错误文本（从「AStockClient 未初始化」变成「工具不存在」），但这是**已知缺口**，不是本批修好的。
3. **`finance.rs` 仍留 1 处 `analysis_engine::risk` 引用**（VaR/PEG/KELLY 的权威数学源）。把风险数学上收 harness 属 §125 裁定的 J3 延期项，不在 B7b。
4. **`tools` crate 的 `domain_mcp.rs` 测试只覆盖「畸形跳过 + 同名保留」**：没有端到端验证「注册表里 5 个名字都能被 LLM 调用」——那需要真实客户端，留到集成测试。
5. **`axagent-runtime/Cargo.lock` / `axagent-infra/Cargo.lock` 未随本批提交**：两个文件在本批开工前已被并发会话改动（同一份 lock 里有一行与本批无关的 `thiserror` 依赖增补），无法按行切分。本批的 lock 变化只有 `axagent-tools → axagent-astock-data` 一条，已落在 `src-tauri/Cargo.lock`（开工前干净）；其余两个 lock 待并发变更落地时一并提交。CI 未使用 `--locked`（已 grep 确认），不会因这一条产生红灯。

## 91. `#35` B8：前端收口（54 处 / 39 文件）可执行分解 —— 先定终局，再分批（2026-11-04）

**本批只做分解，不动代码。** 原因：B8 的 54 处不是同一类问题，其中 6 处是判据误报/跨域，48 处是**真实耦合**且分属四种不同终局；按 B1–B7 的经验，先混在一起改会把「改路径洗绿」当成进展。

### 91.1 四条终局（R6 已定框架）

| 类 | 终局 | 处数 | 前置 |
|---|---|---|---|
| ① 判据误报 | **不改代码、不改判据**（§64.13 先例：中途改 `feRe` 会让 54 失去连续性）；在审计判读口径里逐条登记 | **6** | 无 |
| ② 共享基元下沉 | 搬到 `src/components/<neutral>/` 或 `src/lib/` **且目标本身中性**（组件中立性门禁要求搬入中性目录后不得再指向 feature 目录） | 20 | 需先证明目标无域语义 |
| ③ 宿主生成 DTO | 域 DTO 进 `schema-gen` → `src/types/generated/`（R6 原文）；宿主只认生成物 | 6 | schema-gen 扩面（当前只生成 18 个 IPC DTO + event names） |
| ④ 随插件迁出 | 页面 / 任务中心 / 聊天渲染器改走插件 `pages[]` 或**渲染器注册表** | 22 | 前端 bundle 通道（股票包目前只有 Rust worker + manifest，**没有前端资产**） |

### 91.2 逐类拆解（盘上逐条解析，`.tmp/b8-classify.mjs` 可复算）

**① 判据误报 / 跨域（6 处，不改代码）**
- `chat/ChatViewMessages.tsx:87 → ./QuoteBlock`、`chat/InputArea.tsx:73 → ./QuotePreviewBar`：聊天**消息引用块**，与行情无关（`quote` 命中 feRe）。
- `components/settings/index.ts:30`、`pages/SettingsPage.tsx:71 → chat/PluginMarketplace`：**插件市场**，与股市无关（`market` 命中 feRe）。
- `pages/OpcPage.tsx:22,25 → ./opc/components/{MarketPackTab,TalentMarketTab}`：导入方与目标**都是 OPC 自有文件**；Rust 侧有 `cross` 桶处理这种情形，前端侧没有 —— 属**两域判据不同源**。

**② 共享基元下沉（20 处）**
- `@/lib/stock-analysis-utils` **13 处**（agentOutput / decision-agreement / dataQualityDiagnosis / decisionInputDiagnosis / ~~MultiLlmVotePanel~~（2026-11-05 §97 随死代码删除）/ time-travel ×3 / theme 与 utils 测试 ×2 等）。**2026-11-05 实测余 12 处**。
- `@/lib/investRoutes` **3 处**（office StrategyRoomMiniBar / time-travel ReplayWorkbench ×1 / DailyReviewPanel）→ 宿主需要「跳到投资页」的通用导航口。
- `stock-analysis/_shared/PageHeader` 1、`stock-analysis/utils` 1、`stock-analysis/vendorCheck` 1、`office/phaser/investSceneTemplates` 2。
- ⚠ 其中 `stock-analysis-utils` 的函数（`parseAction` / `getActionColor` / `analystBaseOf` / `horizonLabelKey`）是**股票语义**，单纯改名是洗绿 ⇒ 终局是「这些宿主 lib 本身属于域」或「把纯函数上收成参数化工具」，必须先逐函数判中性，不能整文件搬。

**③ 宿主生成 DTO（6 处）**
- `agentOutput.ts` / `TrajectoryPanel.tsx` / `agentOutput.test.ts` → `@/types/stock-analysis` 的 `StockDecision` / `TimelineNode` / `TimelinePhase`；`screenshotDiagnosisStore.ts` → `@/types/paper-portfolio` 的 `PaperPortfolio`。
- 判据：这些类型若在 Rust 侧有 DTO（`PaperPortfolio` 有），走 schema-gen；若是**纯前端视图类型**（`TimelineNode` 的 `tabKey: "market"|"analyze"|"execute"`），则只能留在域文件里 ⇒ 宿主消费方必须改走渲染器或删除该依赖。

**④ 随插件迁出（22 处）**
- 页面：`ContentArea → InvestPage`（lazy 路由）、`ScreenerPage`、`ScreenshotDiagnosisPage`。
- 任务中心：`adapters/registry.ts`、`TaskCreateModal`(2)、`OverviewTab`(3)、`RunDetailDrawer`、两个测试(4) —— 任务中心的 adapter 形态已是「按 run 类型分派」，终局是把股票 adapter 注册进插件贡献面。
- 聊天渲染器：`WorkflowAgentCard`(2)、`MultiLlmVotePanel`(1) —— 宿主聊天渲染股票输出，需要**渲染器注册表**（宿主提供插槽，域提供实现）。
- barrels：`stores/index.ts`(5)、`types/index.ts`(3) —— R6 已裁「拆，不留垫片」，但**必须先完成上面 ④ 的消费方改道**，否则拆 barrel 只会把隐藏依赖变成显式计数（读数会上升）。

### 91.3 批次顺序（依赖决定）

| 批次 | 内容 | 前置 | 预期读数 |
|---|---|---|---|
| **B8-0** | **前端 bundle 通道**：把股票前端（页面 + 渲染器）打进插件产物，宿主按 `pages[]` 装载 | 无（#44/#46/#64 已建，缺 vite 产物与 manifest 资产段） | 0（前置）；**B8-0a 通道已打通，见 §92** |
| **B8-1** | **渲染器注册表**：宿主聊天/任务中心只认「按类型注册的渲染器」，股票实现由插件注入（**终局形态已裁：见 §93 —— schema 卡片 / iframe 富卡片，不取主 origin React**） | B8-0 | −6～−9（先建注册表 + 1 个试点卡片） |
| **B8-2** | 页面与任务中心迁移（InvestPage / Screener / ScreenshotDiagnosis / TasksCenter adapter） | B8-0 | −10～−12 |
| **B8-3** | 生成 DTO（schema-gen 扩面）或删除前端视图类型依赖 | 无（可与 B8-2 并行） | −4～−6 |
| **B8-4** | 共享基元下沉（逐函数判中性后） | B8-1/B8-2 之后（避免先搬又改） | −8～−12 |
| **B8-5** | barrel 拆除（`stores/index.ts` / `types/index.ts`） | B8-1～B8-4 全部完成 | 收尾，读数不变或略升后归零 |

**终局判据**：B8 完成后，前端反向阻塞 = **6 处判据误报（登记不改）** + 0 真实耦合；若判据同源化（前端补 `cross` 桶）则读数为 **52 → 0**。**不得**为了数字去改 `feRe`。

### 91.4 本批未做（如实列）

1. B8-0 的前端打包方案未定稿：需要决定「页面产物用 iframe 容器（T3 现有形态）还是 host 内 module federation」——前者复用 #64 的 page 容器，后者要新机制；先用现有 `pages[]`。
2. 20 处共享基元的「逐函数中性判定」未做（`stock-analysis-utils` 的 8 个导出需要一条条判）。
3. `schema-gen` 扩面未做（当前只出 18 个 IPC DTO）。

## §92 B8-0a 前端 bundle 通道打通（#35，2026-11-04）

**目标**：把 `pages[]` 这条「插件自带前端资产 → 宿主解析 → 页面容器装载」的链从**只有宿主侧实现**补到**产物侧真的带资产**。§91 判定的缺口是「股票包只有 Rust worker + manifest，没有前端资产」；本批只打通通道，不搬 UI。

### 92.1 改动（3 个文件，+114 行 / 新增 1 个资产文件）

| 文件 | 改动 |
|---|---|
| `axagent-plugins/crates/plugin-stock/web/pages/index.html`（新增，31 行） | 通道探针页：静态 HTML，**刻意不含脚本** —— 桥客户端是 TS 源码模块（`src/sdk/pluginClientBridge.ts` 由插件作者 vendored 进构建图），静态页没有构建步骤，写内联 `postMessage` 只会成为协议的第二份副本（违反「协议值只住两个文件」的既有约束）。 |
| `scripts/build-plugin-bundle.mjs`（+89 行） | ① `webRoot = axagent-plugins/crates/plugin-stock/web` 整树拷进产物，相对路径原样保留；拷贝前按**源目录顶层子目录**清同名目录（源里删掉的页面不会作为孤儿留在产物里）；② 扩展名按 `PANEL_ASSET_ALLOWED_EXTS` 的镜像白名单在**打包期**拒绝（不等安装期清单校验才失败）；③ 清单新增 `pages: [{ id: "stock-home", title: "股票", entry: "pages/index.html", loading: "bundle" }]`；④ `--check` 新增 `checkDeclaredPages()`：`entry` 必须是相对路径、不含 `..`、扩展名合规、解析后仍在产物根内且文件存在。 |
| `axagent-plugins/crates/plugins/tests/e2e_stock_bundle_install_test.rs`（+25 行） | 安装链路末尾补断言：`list_plugin_pages()` 能看到 `stock-home`；`resolve_page_asset()` 返回的绝对路径**以 `outcome.install_path` 为前缀**、是既有文件、内容确为探针页。 |

### 92.2 验证（全部实跑，读数可复算）

| 命令 | 结果 |
|---|---|
| `node scripts/build-plugin-bundle.mjs --check --out .tmp/plugin-stock-check` | `EXIT=0`；产物 1 个前端文件；`--check ✅ 1 个 bundle 页的 entry 均存在且在白名单内`；`--check ✅ describe 握手 + shutdown 回执 + 退出码 0 + sha256 一致` |
| 负例 A/B：把 `web/pages/index.html` 移走后重跑打包 | `EXIT=1`，报 `前端源目录为空` —— **缺资产不会产出只有清单没有页面的包**（跑完已原样移回） |
| `AXAGENT_STOCK_BUNDLE_DIR='D:\OneManager\AxInvest\.tmp\plugin-stock-check' cargo test -p axagent-plugins --test e2e_stock_bundle_install_test` | `1 passed; 0 failed`（含新增的页面解析断言，0.49 s） |
| `rustfmt --edition 2024 --check e2e_stock_bundle_install_test.rs` | `EXIT=0` |
| `node --check scripts/build-plugin-bundle.mjs` | `SYNTAX_OK` |
| `node scripts/check-capability-register.mjs`（按既有规程驻留 PLAN 头部 3 行后跑，`cmp` 校验 `RESTORED_IDENTICAL`） | `PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）` |

**读数不变**：`l1ForeignStock` 102 处 / 19 文件、`tool.set` 198、H 桶 3 条 / 3 文件、Cargo 边 7 条 —— 本批**不碰任何域 crate 引用**。前端体量口径（`feRe` 只扫 `src/**` 的 .ts/.tsx）同样不变：新增的 `.html` 在 `axagent-plugins/` 下，不入该口径。

### 92.3 本批未做（如实列）

1. **真实股票 UI 未迁**：探针页只证明通道；页面/任务中心/渲染器迁移仍按 §91.3 的 B8-1..B8-5 排队。插件前端仍**没有 vite 构建**（当前直拷静态资产；等 B8-1 落地后再决定是否引入构建步骤）。
2. **整栈 UI 未跑**：没有启动 Tauri 应用做「安装包 → 页面容器渲染」的目视验收；本批验到的是宿主 `resolve_page_asset` 边界（e2e 安装测试）与产物自检，容器渲染归 #64 的既有测试覆盖。
3. **`integrity` 语义未动**：`plugin.json` 里的 `integrity.hash` 仍是**worker 二进制**的哈希（打包脚本自检用）；宿主安装锚点是 `hash_plugin_directory()` 的**整目录**哈希，二者不同源。这是 B4 打包骨架遗留的措辞问题，不在本批扩大；页面资产受宿主目录哈希保护，不受此影响。
4. 第五道门裸跑仍会因 PLAN 头部 3 行与钉死行号差 3 而报 26 条假红（**既有状态**，非本批引入）；本批按既定驻留规程取证。

## §93 B8-1 终局裁定：插件卡片**不取主 origin React**（2026-11-05）

**结论先行**：`pages[]` 的 `loading: "bundle"` 与 `loading: "schema"` 两条通道**都是沙箱/宿主渲染**；聊天与任务中心的插件卡片**不得**以「第三方编译好的 React 组件注入主 origin」实现。插件贡献卡片只有两条合法形态：

| 形态 | 装载 | 适用 | 现状 |
|---|---|---|---|
| **schema 卡片**（首选） | 插件给 `schemaId` + 数据投影，宿主 `DynamicUIRenderer` 渲染（Card/Table/Chart 等容器已具备，见 `src/components/dynamicUI/`）；文案随插件产物走，不占宿主 11 语言 key | 决策卡、分析进度、投票结果这类**结构化展示** | 页面侧已通（`PluginPageHost.tsx:108` 的 schema 分支），**消息侧插槽未建** |
| **iframe 富卡片** | 复用 `pages[]` 的 bundle 容器（sandbox iframe + 桥），宿主只给一块固定尺寸的容器 | 需要第三方 JS 交互的富卡片（图表联动、拖拽） | 容器已建，**消息侧容器未建** |

**为什么不能走 module federation / 动态 `import()`（推翻 §91.3 原措辞里的「插件注入 React 实现」）**：

1. **安全边界**：面板与页面今天都跑在 `sandbox="allow-scripts"` 的 iframe 里；第三方 React 进了主 origin，就能直接摸 `window`、Tauri IPC 桥与 `@/lib/invoke` —— 等于把已建立的「插件不碰宿主状态」口径**从根上作废**。隔离不是性能选项，是权限模型的前提。
2. **技术面未验证且成本高**：asset 协议跨源 `import()` 需要放开 CSP `script-src`、共享 React 19 / antd 6 运行时（import map 或 federation），而插件产物口径是「自包含 worker + 静态资产」；引入共享运行时会让「插件自己构建」变成「必须与宿主版本严格对齐」，与已拍的独立性冲突。
3. **不需要新协议**：schema 通道是现成的（#39/#44/#64），只是还没接到消息渲染上。发明第二套装载机制的收益是「能写任意 React」，代价是上面两条。

**对批次的影响**：

- **B8-1 重新定义**为「**宿主卡片注册表 + 数据投影契约**」：注册表键 = `(消息类型 / 任务 run 类型)`，值 = `schemaId` 或 `bundle pageId`；宿主只认这两类值，**不认 React 组件**。首个试点选 `WorkflowAgentCard` 的「分析进度」子树（结构简单、无第三方 JS），验收点是「宿主不再 import 任何 `stock-analysis` 文件即可画出该卡」。
- **B8-2..B8-5 顺序不变**；B8-4（共享基元逐函数判中性）与 B8-5（barrel 拆除）仍需做。原目标「−6～−9」拆成「注册表落地 −2～−3 + 各卡片逐张迁 −4～−6」，不许一次性宣称全部达成。
- **irreversible 的一条**：一旦有人把第三方 React 拉进主 origin，再想收回就得改插件产物格式 ⇒ 本裁定先于任何 B8-1 代码写入。

**复算口径修正（同批如实记）**：`.tmp/b8-classify.mjs` 2026-11-04 复算为 **52 处 / 39 文件**（`genuine(目标属域)` 桶），与 §91 记的 54 处差 2 —— 差额来自并行批的前端改动，§91 的分类结论（6 误报 / 20 基元 / 6 DTO / 22 随包迁出）与批次顺序不受影响；B8-1 开工时以**当日复算**为准，不把 54 当现值引用。

**本批未做**：没有改任何前端代码；消息侧 schema/iframe 插槽、注册表本体、试点卡片均未实现（这是 B8-1 的实现批）。

## §94 B9-1a：`compute_evidence_weights` 迁入股票插件（双声明期，2026-11-05）

**批次定位**：P6.1 命令面迁出的**首个用例**，走 §13.2 的五步模板 ①输入 DTO（复用 analysis-engine 既有结构）②manifest 双声明 ④worker 实现；**不做 ⑤（删宿主命令）** —— 删命令要重生成 `register_commands.rs`（实测主 crate `cargo check` 21m30s）并同批改 5 个登记读数，与「改前端路由」放在同一批不可回滚。宿主同名 Tauri 命令**保留但已无调用方**，退役走 B9-2。

### 94.1 改动（8 个文件 + 1 个锁文件）

| 文件 | 改动 |
|---|---|
| `axagent-plugins/crates/plugin-stock/Cargo.toml` | 新增 `axagent-analysis-engine` 依赖（**域自有实现**，终局随 #32 进插件仓）；注释从「只允许 foundation」改为「可依赖域自有实现，不得反向依赖主壳/宿主插件运行时」。 |
| `axagent-plugins/crates/plugin-stock/src/main.rs` | `COMMAND_EVIDENCE_WEIGHTS` 常量 + 命令分发 + `evidence_weights()`：`from_value::<EvidenceWeightRequest>` 严格解码（缺字段报 `PLUGIN_COMMAND_INVALID_ARGS`，不默认值兜底）、`to_value` 回执。 |
| `axagent-plugins/crates/plugin-stock/tests/protocol.rs` | 新增 `compute_evidence_weights_runs_without_any_seam_call`：真进程帧往返，断言**不发任何接缝帧**、权重/建议动作非空、缺字段显式报错。 |
| `scripts/build-plugin-bundle.mjs` | manifest `commands[]` 第 3 条：`compute_evidence_weights` + 完整 camelCase `inputSchema`（marketRegime / timeHorizon / analysts / historicalWeights）。 |
| `src/lib/stock-analysis-utils.ts` | `computeEvidenceWeights()` 由宿主 `invoke("compute_evidence_weights")` 改为 `resolvePluginForCommand` → `invoke("plugin_execute_command", { pluginId, name, input })`；未装/未启用抛错，调用方走既有离线兜底（`ANALYST_TIME_HORIZON_WEIGHT`）。 |
| `src/lib/__tests__/evidenceWeights.pluginRoute.test.ts`（新增） | 3 条：命令名经 resolver、入参原样透传、无插件/命令失败不吞错。 |
| `axagent-plugins/crates/plugins/tests/e2e_stock_bundle_install_test.rs` | 安装 → 启用后追加执行权重命令，断言输出 `timeHorizon` 与 1 条分析师权重。 |
| `scripts/audit-l1-blockers.mjs` + `scripts/check-capability-register.mjs` | `STOCK.ownedRe` 增加 `^axagent-plugins/crates/(plugin-stock\|stock-core)/`（**两份逐字同改**）：插件侧是迁出目的地、不是主壳，其链接域 crate 属预期方向；宿主侧运行时 `axagent-plugins/crates/plugins` 仍计入 foreign。**这不是为数字放宽** —— 原判据把「插件本体」算成「主壳反向阻塞」是归属错误；若不改，本批会凭空 +1。 |
| `scripts/capability-register.json` | `frontendFileCaliber` 85 164→**85 181**（`stock-analysis-utils.ts` +17 行；文件数 295 不变，新测试不命中 `feRe`）。`l1ForeignStock` 保持 **102 / 19**。 |
| `axagent-plugins/Cargo.lock` | 新增 analysis-engine 传递图（+1841 / −3 行），只增本工作区依赖。 |

### 94.2 验收（全部实跑）

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| K1 | `cargo test -p axagent-plugin-stock`（cwd=`axagent-plugins`） | 0 | 首编 4m19s；**10 passed**（含新增权重用例） |
| K2 | `node scripts/build-plugin-bundle.mjs --check --out .tmp/plugin-stock-check` | 0 | 前端页 1 个；清单 3 条命令；握手/回执/sha256/页面四项断言全过 |
| K3 | `AXAGENT_STOCK_BUNDLE_DIR='…\.tmp\plugin-stock-check' cargo test -p axagent-plugins --test e2e_stock_bundle_install_test` | 0 | **1 passed**（安装→启用→行情命令→权重命令→页面解析） |
| K4 | `cargo clippy -p axagent-plugin-stock --all-targets -- -D warnings` | 0 | Finished 3m07s |
| K5 | `node scripts/check-tsc.mjs` | 0 | 1464 个源文件（+1 新测试） |
| K6 | `vitest-run.mjs run src/lib/__tests__/evidenceWeights.pluginRoute.test.ts` | 0 | **3 passed** |
| K7 | `npx dprint check`（2 个 TS 文件）+ `rustfmt --edition 2024 --check`（3 个 `.rs`） | 0 / 0 | 零 diff |
| K8 | `node scripts/check-capability-register.mjs`（驻留 PLAN 头部 3 行后）+ `audit-l1-blockers.mjs --check-allowlist` | 0 / 0 | 门 PASS（37 = A11/B8/C4/H1/K10/D3）；H 桶 3 条锚点全在 |

### 94.3 已量化的一项代价

worker 二进制 **827,392 → 5,703,680 字节（debug）**：analysis-engine 的传递图（dao / entities / trajectory / astock-data，含 image/keyring 一族）随依赖整体链接。原因是本仓 `analysis-engine` 是**未拆分的整体 crate**，而这个命令只需要其中纯函数模块。终局随 #32 把股票域实现搬进插件仓后会重新评估；**release 体积未测**（不拿 debug 数当发布结论）。**不得**为了这个数在插件里复制一份权重算法（禁区 12）。

### 94.4 本批未做（如实列）

1. **宿主命令未删**（B9-2）：`src-tauri/src/commands/stock_analysis.rs` 的 `compute_evidence_weights` 与 `#[agent_command]` 段仍在，只是无人调用。删除需 `cargo check -p axagent --lib` 重生成 `register_commands.rs`，同批改 `commandTotal / commandNameSetParity / stockBoundaryCommands / stockAuditDomainCommands / registryFileLines` 五处 + PLAN 钉句。
2. **智能体命令面未接**：插件命令不进 `#[agent_command]` 的 inventory（`command_bridge.rs` 只读宏注册表）⇒ 删除宿主命令后，LLM 侧会少一条命令。与 B4-k 同口径，等「插件命令并入智能体命令面」立批解决。
3. **桌面真机未验**：没有带插件的安装包，也没在 `tauri dev` 里跑「共识计算 → 权重报告」全链路；浏览器 mock 的 `plugin_execute_command` 返回空串 ⇒ 该模式下与改造前一样走离线兜底（改造前宿主命令在 mock 里也无实现）。
4. **开发期需手工暂存**：`plugin:bundle:dev` 不进 `beforeDevCommand`（与 §84 未做 3 同因），本地不跑它就没有可解析的插件。

## §95 B9-2：删宿主 `compute_evidence_weights` 并收口注册表（#35，2026-11-05）

**这一批把 B9-1a 的切换收尾**：宿主 `stock_analysis.rs` 的 `compute_evidence_weights`（含 `#[agent_command]` 段、文档注释与专用 import）已删除，前端仍走插件通道 ⇒ 宿主不再提供第二条同名路径。

### 95.1 改动（4 个文件）

| 文件 | 改动 |
|---|---|
| `src-tauri/src/commands/stock_analysis.rs` | 删 `compute_evidence_weights` 命令块（`// ── P0-1 …` 小标题 + 文档注释 + `#[agent_command]` + `#[tauri::command]` + 函数），删第 23 行 `use axagent_analysis_engine::evidence_weight::{self, EvidenceWeightReport, EvidenceWeightRequest};`（全文件仅该命令使用）。 |
| `src-tauri/src/register_commands.rs` | **build.rs 产物**，由 `cargo check -p axagent --lib` 重生成（禁区 7，不手改）：`split('\n')` **1609 → 1608**、`compute_evidence_weights` **零命中**。 |
| `scripts/capability-register.json` | `commandTotal` 1542→**1541**、`commandNameSetParity` 1542→**1541**、`stockBoundaryCommands` 303→**302**、`stockAuditDomainCommands` 305→**304**、`registryFileLines` 1609→**1608**；`commandTotal.planPinned` 钉句同步；**移除** `planOld` 里 1541 那一条（它不再是旧读数；1542 转历史、不设 planOld，理由写进 note）。 |
| `docs/plans/PLAN-shell-plus-plugins.md` | §0.8.2 的 `register_commands.rs` 现值、§0.10.1 两行命令读数（302/304、`stock_analysis` 171）同批改。 |

### 95.2 验收（实跑）

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| K1 | `cargo check -p axagent --lib`（cwd=`src-tauri`） | 0 | Finished **15m59s**；产物行数 1608、`compute_evidence_weights` 零命中 |
| K2 | `rustfmt --edition 2024 --check src-tauri/src/commands/stock_analysis.rs` | 0 | 零 diff |
| K3 | `node scripts/check-capability-register.mjs`（驻留 PLAN 头部 3 行） | 0 | 硬判据全绿（37 = A11/B8/C4/H1/K10/D3） |
| K4 | `node scripts/audit-l1-blockers.mjs` / `--check-allowlist` | 0 / 0 | 边界口径 **302 / 20**、审计域口径 **304 / 19**；H 桶 3 条锚点全在 |
| K5 | 前端 | — | 本批零前端改动：`computeEvidenceWeights` 在 B9-1a 已走插件，宿主命令本无调用方 |

### 95.3 已知行为变化（不是副作用，是裁定后的代价）

**`#[agent_command]` 面少了 `compute_evidence_weights`**：`command_bridge.rs` 只读宏注册表（inventory），插件命令不在其中 ⇒ 删除宿主命令后，LLM 的可用命令清单里不再有它。这与 B4-k 删 `parse_analysis_intent` 同口径；**插件命令并入智能体命令面**仍未立项，是 #35 收尾前必须补的一条（否则每迁一条命令，智能体面就少一条）。前端不受影响。

### 95.4 本批未做

1. **主 crate clippy 未单跑**：本批对主 crate 只有「删一段」的改动，用 `cargo check` 兜底；真跑 `clippy -p axagent --all-targets` 需约 20 分钟独占构建窗口，留到 #35 收尾的全量门禁。
2. **桌面真机 / 安装包未验**：与 §84/§94 同因（没有带插件的安装包）。
3. **local dprint 在 UNC 下仍不可用**（#142 既有环境问题）：本批改了 `capability-register.json`，用 JSON 解析 + 第五道门（会完整解析该表）兜底；CI 在 Windows 本地磁盘上会正常跑 dprint。
4. **智能体命令面并轨未做**：见 95.3，未在本批扩大。

## §96 B10-1：插件命令并入智能体命令面（#35，2026-11-05）

**兑现 §95.3 的欠账**：`#[agent_command]` 宏 inventory 里只有编译期宿主命令，插件贡献的命令不在其中 —— 每迁走一条宿主命令，智能体可用命令清单就静默少一条。本批把「已启用插件 manifest 的 `commands[]`」并入命令桥的**索引**与**执行**两个面，删宿主命令不再以智能体能力为代价。

### 96.1 改动（5 个文件 + 登记表）

| 文件 | 改动 |
|---|---|
| `axagent-plugins/crates/plugins/src/types.rs` | 新增 `PluginAgentCommand{plugin_id,name,description,required_permission}`：运行期数据，故用拥有所有权的 `String`，**不复用**编译期的 `CommandMetadata`（后者是 `&'static str` + 宏域）。 |
| `axagent-plugins/crates/plugins/src/manager.rs` | 新增 `PluginManager::agent_command_specs()`：只收 `enabled` 插件；单插件 manifest 读失败按跳过（与 `list_plugin_pages` 同口径）；按 `name`/`plugin_id` 排序（目录遍历顺序不可当稳定序）。 |
| `src-tauri/src/commands/agent/command_bridge.rs` | 新增 `PluginCommandSpec` + `plugin_command_safety`（None/`read-only`→Safe，`workspace-write`→Caution，`danger-full-access`→Dangerous，**未知档→Dangerous fail-closed**）、`merge_plugin_command_specs`（**宿主重名优先**）、`collect_plugin_command_specs`（`spawn_blocking` + `blocking_read`）、`render_plugin_command_index` / `build_plugin_command_index`、`execute_plugin_agent_command`；`execute_tauri_command` 改为「宏命令 + 插件命令」合并查表后走同一安全门（合并查表后 `CommandRegistry::contains` 失去唯一调用方 ⇒ 同批删除，编译警告 `dead_code` 归零）；`list_available_commands` 在**无 domain 过滤**时追加插件命令（`source:"plugin"` / `pluginId` / `domain:"plugin"`），宿主命令补 `source:"host"`。 |
| `src-tauri/src/commands/agent/mod.rs` | 系统提示注入处把 `command_index` 改为可变，并在其后追加 `build_plugin_command_index(&app).await`。 |
| `scripts/capability-register.json` | 代码 evidence 重锚：`dojo.extension` `manager.rs` **755→787**（`--repin-code --apply`，插 32 行所致，表内正文同步）；`command.handler` `command_bridge.rs` **1125→1297** 并**补 `anchorText`**（该条原无定位键，`--repin-code` 拒写 ⇒ 人工按唯一命中行补键）。 |
| `docs/plans/PLAN-shell-plus-plugins.md` | §0.7 的 `command.handler` 行号 1125→1297（与登记表 `planAnchorText` 逐字同改）。 |

### 96.2 判据（为什么是这几条）

1. **安全档位由宿主推导，不由插件自封**：插件命令只能声明 `requiredPermission`，`Safe/Caution/Dangerous` 一律由 `plugin_command_safety` 换算；未知值走 Dangerous（清单校验会拒未知值，但该函数是 `pub` 的，不能依赖上游一定校验过）。
2. **重名宿主优先**：两边都暴露同名命令时，LLM 看到两条、实际命中哪条取决于分支顺序 —— 合并时直接剔除插件侧并告警。
3. **`list_available_commands` 与索引同口径**：索引里出现的插件命令，用本工具二次核对时必须查得到；带 domain 过滤时不返回（插件命令不属于任何宿主 `CapabilityDomain`，按域语义就该空）。
4. **执行路径与 `plugin_execute_command` 同形**：`spawn_blocking` + `blocking_read`，帧往返最长 30 s，不得在异步上下文里同步阻塞。

### 96.3 验收（实跑）

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| K1 | `cargo check -p axagent --lib`（cwd=`src-tauri`，首轮） | 0 | Finished **26m41s**；**1 条** `dead_code` 告警（`CommandRegistry::contains`）⇒ 当批删除该失效方法 |
| K1b | 同上（删方法后增量复跑） | 0 | Finished **11m57s**；`grep -c warning` = **0** |
| K2 | `cargo check -p axagent-plugins --tests` / `cargo test -p axagent-plugins --lib agent_command_specs` | 0 / 0 | Finished 1m02s；`agent_command_specs_projects_only_enabled_plugins` ok |
| K3 | `cargo clippy -p axagent-plugins --all-targets -- -D warnings` | 0 | Finished 18.11s，零告警 |
| K4 | `rustfmt --edition 2024 --check`（3 个改动文件） | 0 | 零 diff（改前有 3 处 diff，已 `rustfmt` 落盘）；CR 字节 = 0 |
| K5 | `node scripts/check-capability-register.mjs`（驻留 PLAN 头部 3 行） | 0 | 37 项硬判据全 PASS；`cmp` 校验 `RESTORED_IDENTICAL`；`--repin-code --apply` 后复检无待回写 |

### 96.4 已知代价 / 本批未做

1. **每条命令执行都重扫一遍已启用插件的 manifest**（`list_plugins` + 逐插件 `load_plugin_from_directory`）：插件数少时可接受；不能用 `CommandCache` 缓存，因为清单会随安装/卸载/改文件变化。首个性能信号出现时再考虑「按目录 mtime 失效」。
2. **`requiredPermission` 仍是插件自述**：`check_command_gate` 拿它与已授权权限比对，但声明为 `read-only` 的命令不会触发权限检查 —— 这是既有语义（§0.7.6 的「权限不执法」同族），本批不扩大。
3. **主 crate clippy 未单跑**：与 §95.4 同因（20 分钟独占构建窗口），留 #35 收尾全量门禁。
4. **桌面真机 / 安装包未验**：注册命令清单不变（本批零命令增删），`register_commands.rs` 未重生成。

## §97 B8-1a：聊天侧股票卡片**复扫推翻试点**，死代码清零（#35，2026-11-05）

**动手前先把 §93 的试点验了一遍，结论是试点对象不存在** —— §91.2 ④ 记的「聊天渲染器 `WorkflowAgentCard`(2)、`MultiLlmVotePanel`(1) 需要渲染器注册表」，实测是**死代码**，不是耦合：

| 判据 | 实测 |
|---|---|
| 谁渲染 `WorkflowAgentCard` 组件 | **零渲染点**。聊天分派在 `ChatViewMessages.tsx` 的 `contentRender`（按 `msg.blocks[].type` 与 `CHAT_CUSTOM_HTML_TAGS` 的自定义 HTML 标签），该组件不在任何分派分支里，只有文件内 `:731` 自递归。 |
| `parseWorkflowCard` 调用方 | **0**（全仓仅定义处）。 |
| 生产端 | `stores/feature/stockWorkflowChatBridge.ts:85` 仍往消息正文写 `<!-- workflow-<type>:JSON -->` ⇒ **写了没人渲染**（HTML 注释在 Markdown 里不可见）。 |
| `MultiLlmVotePanel` 外部引用 | **0**（组件、类型、测试、i18n 全无；`chat.multiModelVote.*` 15 个 key 只有它自己用）。 |

**判据裁定**：不能为「注册表」新建一个今天没有任何渲染点的插槽 —— 那是给假想需求做设计。本批因此改成**死代码清零 + 试点改判**，注册表本体留到 §98 的真实试点上再建。

### 97.1 改动（4 类）

| 面 | 改动 |
|---|---|
| 域内新落点 | 新增 `src/components/stock-analysis/workflowChatCard.ts`（61 行）：只搬宿主那份文件里**真正被消费的股票语义** —— `WorkflowCardData` 类型 + `makeWorkflowContent`（拼 `<!-- workflow-*:JSON -->` 标记）。React 卡片实现与 `parseWorkflowCard` 随死代码删除（§93 已裁：卡片渲染只能走 schema 或 iframe，不再留第三方 React 骨架）。 |
| 删除 | `src/components/chat/WorkflowAgentCard.tsx`（792 行，组件零渲染点）、`src/components/chat/MultiLlmVotePanel.tsx`（零外部引用）。 |
| 改 import | `src/stores/feature/stockWorkflowChatBridge.ts:1` 改指 `@/components/stock-analysis/workflowChatCard`。 |
| 11 语言清键 | 删 `chat.multiModelVote` 整块（每语言 15 键 / 17 行，共 11×17 行），逐语言 `JSON.parse` 校验后落盘，其余字节不动。 |

### 97.2 顺带修的门禁缺陷（不是为数字放宽）

`check-i18n-orphan-diff.mjs` 的候选式是 `U − K − 动态前缀 − error.*`，**没有「键还在不在 locale」这一维** ⇒ 「删码同时把 11 语言键删掉」这一**正确做法**必然红：本批删完 `MultiLlmVotePanel` + 11 语言 15 键后，门仍报 12 条「须同步删除 11 语言 locale」，而那 12 条已经不在 locale 里。修法 = 候选再减 `{k | k ∉ localeKeys}`，`localeKeys` 取 **11 语言现存键的并集**（任一语言还留着就照报，不做 zh-CN 单侧豁免）；读不到/解析失败某个 locale ⇒ 显式打印并**退回不做该维过滤**（宁可多报，不许静默放行）。selftest 同步补 3 条：「已同步删键」正控、`localeKeys` 缺省不放行负控、`flattenLocaleKeys` 数组不产键。

### 97.3 验收（实跑）

| 标签 | 命令 | EXIT | 取证 |
|---|---|---|---|
| K1 | `npm run typecheck`（`scripts/check-tsc.mjs`） | 0 | 扫到 **1463** 个 src 文件（非空跑），`--noEmit` 无错 |
| K2 | `npm run test:run -- stockWorkflowChatBridge` | 0 | **2 passed**；⚠ 必须走包装器 `scripts/vitest-run.mjs` —— 直接 `npx vitest run <filter>` 在本机 UNC 根下把 root 解析成 `Z: Folders/D/...`，报「No test files found」（#111/#137 同族，本次实测复现） |
| K3 | `node scripts/check-i18n-orphan-diff.mjs --selftest` / 正式跑 | 0 / 0 | 自检含 3 条新用例全过；正式跑 PASS |
| K4 | 负控（往 zh-CN 塞回 `chat.multiModelVote.title` 再跑孤儿门） | 1 | 恰好报 **1** 条且就是那条键 ⇒ 新维度只豁免「已删干净」，没削弱原判据；随后 `cmp` 校验还原 `RESTORED_IDENTICAL` |
| K5 | `node scripts/check-i18n-key-parity.mjs` | 0 | 11 语言各 **16 282** 条，路径集合一致 |
| K6 | 中立性 / JSX 文案 / 豁免预算 三道门 | 0/0/0 | 中立性 225 条、基线外 0（并提示「本轮已修复 1 条」= 删掉的 `chat → stock-analysis` 那条，**未跑 `--update-baseline`**，等并行批落地再收紧）；JSX 基线外 0；豁免预算无基线外问题 |
| K7 | `node scripts/audit-l1-blockers.mjs` + 第五道门（驻留 3 行） | 0 / 0 | 前端反向阻塞 **55 → 52 处 / 40 → 38 文件**；`frontendFileCaliber` **85 242 / 296**、`frontendDirCaliber` **60 973 / 213**（+61 行 / +1 文件，同批改表与 PLAN §0.10.1）；`zh-CN i18n key` 仍 3 527（删的是 `chat.*` 命名空间）；37 项硬判据全 PASS |

### 97.4 试点改判（下一批 B8-1 的真实对象）

聊天侧「今天真的在渲染」的股票耦合 = **0 处**，注册表没有试点。逐条扫剩余 52 处后，唯一同时满足「真实渲染 + 已有按类型分派的注册表 + 可数据化」的落点是**任务中心**：

- `src/pages/TasksCenterPage/adapters/registry.ts` 已有 `REGISTRY` 数组 + `registerAdapter()`（运行时可追加），键 = `SourceId`；
- 股票两个源 `stockAnalysis.adapter.ts` / `pipeline.adapter.ts` 由宿主静态注册，`SourceId` 是**闭合联合** ⇒ 插件源进不来；
- 终局形态（待 §98 裁定后实现）：插件清单新增**任务源声明面**（`taskSources[]`：id / 标题 / 取数命令 / deepLink / 颜色），宿主用**泛型 adapter 工厂**按声明实例化，数据走 T2 命名命令返回 `UnifiedRun` 形态的行 ⇒ 不往主 origin 注入任何代码，与 §93 一致。
- ⚠ 代价先摆明：现有股票 adapter 含映射逻辑（状态映射、根记录过滤、deepLink 拼装、名称拼接）⇒ 要么把逻辑上移到插件 worker（Rust，返回已映射的行），要么给声明面加一套映射 DSL。**前者是终局，后者会造出第二套声明语言**；`analysisRecordPath` 依赖前端路由表，须一并裁定宿主路由口。

### 97.5 本批未做

1. **`<!-- workflow-* -->` 标记仍写了没人渲染**：生产端照常写，本批不改行为（改就是「新增一张卡」，属新需求，不是迁出）。要恢复展示须走 §93 的 schema 卡片，另批裁定。
2. **未跑 `--update-baseline`**（中立性少 1 条存量）：并行批仍在改前端，按既定纪律等全批落地再收紧棘轮。
3. **全量 Vitest 未跑**：只定点跑了受影响的 1 个测试文件；`chat` 目录删掉两个文件后没有任何测试引用它们（已 grep 确认），全量留 #35 收尾。
4. **注册表本体未建**：见 97.4，试点改判后下一批做。

---

## §98 B8-1b：插件贡献任务源的**整条通道**（#35，2026-11-05）

任务 #148。§97.4 把试点改判到任务中心后，本批把那条通道一次建穿：**声明面 → 清单校验 → 宿主投影命令 → 插件取数命令 → 行契约 → 前端泛型适配器 → 注册表 → 渲染口径**，七段全在盘上，端到端由真安装测试取证。

### 98.1 终局口径（先钉死形态，再谈工序）

| 段 | 落点 | 为什么在这 |
|---|---|---|
| 行契约 | `axagent-core/crates/plugin-proto/src/lib.rs` 的 `PluginTaskRunRow` + `PLUGIN_TASK_RUN_STATES`（7 态） | 插件 worker 唯一够得着的共享 crate；宿主 `UnifiedRun` 是前端视图类型（含 `capabilities`/`badges`），不该也不能过线 ⇒ 行形状只此一份（禁区 12） |
| 声明面 | `plugins::types::PluginTaskSource` / `RawPluginTaskSource` / `PluginManifest.task_sources[]` | 与 `pages[]` 同族：**只声明数据面，不声明渲染**（§93 裁定：第三方 React 不得进主 origin） |
| 清单校验 | `manager.rs` 的 `build_manifest_task_sources` + `is_task_source_id_shape` + 两条新错误码 | 三条硬判据全在**装载期**拦：id 形状（进 `SourceId` 与 `runIdOf` 拼接，脏 id 会破坏「全局唯一 run id」）、`command` 必须指向**同一份清单**已声明的命令、`title` 非空 |
| 投影 | `PluginManager::list_plugin_task_sources()` + 宿主命令 `plugin_list_task_sources`（`src-tauri/src/commands/plugin.rs`，`#[agent_command(domain=plugin, safety=Safe, call_mode=StateOnly)]`） | 与 `plugin_list_pages` 逐字同形：实时扫描、不缓存、不建第二份源清单；`enabled` 位原样透出，启停过滤留给消费侧 |
| 取数 | plugin-stock 的命名命令 `list_active_workflow_runs` → 出站接缝 `workflow.run` 的 `list_active` | 执行态在宿主引擎里，插件不复制登记表；插件只干两件宿主干不了的事：**状态词表翻译** + **形状收敛** |
| 前端 | `adapters/pluginTaskSource.adapter.ts`（工厂）+ `registry.ts` 的 `registerPluginTaskSources` / `useTaskSourceRegistry` | 泛型工厂 ⇒ 新增一个插件任务源 = 改插件清单 + 实现一条命令，宿主前端零改动 |
| 渲染口径 | `TaskSourceAdapter.name?` ⇒ 消费点统一 `adapter.name ?? t(adapter.nameKey)`（`SourceChip` / `RunsTab` 两处筛选与错误条） | 插件文案归产物自己负责（与 `PluginPage.title` 同口径），不占宿主 11 语言 key |

### 98.2 四条裁定与它们的代价

1. **不给 `deepLink`**：工作流执行今天没有独立站内路由（2026-10-28 已删 `/workflow` 一类旧链）⇒ 造一个指向 404 的深链比没有深链更坏。代价：这类 run 的 `openSource` 恒 false，「打开详情」按钮不出现。
2. **`partially_completed` → `succeeded`**：它是终态且确有产出，判成 `failed` 会把「跑完但部分节点没成」的批次误标成失败；细分差异留在 `raw.status`，宿主不改口径。
3. **表外状态一律落 `waiting`**（不猜 running/failed）：`waiting` 在中心的语义是「还没轮到」，既不会虚增「进行中」，也不会污染健康度。代价：一条状态不明的执行看起来像「排队中」，真值只在 `raw` 里。⚠ 前端镜像表用 `Record<RunState, RunState>` 写死七态：`model.ts` 加第八态会**编译不过**，不会静默落 `waiting`；Rust 侧那张表仍靠人工与 `PLUGIN_TASK_RUN_STATES` 对齐（B8-3 生成 DTO 才收口）。
4. **`list_active` 只含 running / paused**（宿主侧过滤，`workflow_run_service.rs:697`）⇒ 本源**今天只会显示在跑与暂停的执行**；`completed`/`failed`/`cancelled` 三条翻译分支是防御性的，**当前不可达**，不许在记账里当成实测到达过的路径。同理 **不给 `finishedAt`**：`updatedAt` 是「最近一次推进」不是结束时刻，拿它当结束时间就是编读数 ⇒ 插件源的耗时列恒空。

### 98.3 本批撞出并修掉的四个缺陷（前三条都不是「代码写错」这么简单）

- **`RawPluginManifest` 没有 `rename_all` ⇒ `taskSources` 被 serde 静默丢成空表**。表现是「清单里有、任务中心里没有」，零报错、零日志。根因不是本批写的字段，是这张结构体的既有约定：多词字段必须逐条 `alias`（`mcpServers` 就是这么写的），而 `pages` 这类单词字段掩盖了这条规则。修法：`#[serde(default, alias = "taskSources")]`（两种写法都吃）。取证链：e2e 真安装测试 `install_enable_and_execute_a_command_from_the_staged_bundle` 先红（断言「股票包应出现在任务源列表里」），加 alias 后同一测试转绿。**教训写进口径**：清单级（走 serde）单测必须和纯函数级单测同批跑，否则「校验逻辑正确、装载链路断裂」这种形态只由 e2e 抓得到。
- **散文引用与镜像表各说一套，两边都不红**：`--repin-code` 把 `dojo.extension` 的 evidence 从 `manager.rs:787` 重锚到 `:838`，同时点出表内正文还写着 `:787`；顺手核对发现 **PLAN §0.7.6 早就停在 `:738`**，与表内登记的 `:787` 已经差了两批。第五道门只管 `evidence[]`，散文里的 `文件:行` 不受管辖 ⇒ 这类腐烂是静默累积的。本批把三处（PLAN 两处 + 表内两处镜像文本）改成同一句话，并把行号链 `:708 → :738 → :755 → :787 → :838` 写进正文。**待立项**：给第五道门加一条「散文引用可定位」巡检（`check-single-source-facts.mjs` 的 `locate()` 已有这套判据，复用即可），否则下一次还是靠人撞。
- **注册时序：源挂上了但列表是空的**。插件清单是页面挂载后异步取回并注册的，而 `RunsTab` 的首次聚合成效只依赖 `[loadRuns, locale]` ⇒ 新源进得了筛选器、进不了表格，且**关掉自动刷新后永不自愈**（轮询只在已有 `hasActive` 时起）。修法：把 `registryVersion` 加进该成效的依赖（不是加轮询、不是在建页时同步取清单）。这条是本批自己写的实现里的缺陷，由「先摆形态再写码」的复核抓出，不是测试红出来的。
- **vitest 在本机会把「mock 直接 reject」的用例判成用例级未处理异常**（假红）。探针取证：`it("PROBE-A")` 里只 `invokeMock.mockImplementation(async () => { throw … })`、**用例内没有任何调用**，该用例仍失败并打印实现里新建的 Error ⇒ 实现被当场以空参试调了一次，那一次没人 await。绕法（已落测试）：实现按「有没有实参」分流，试调那次回 resolved，rejection 只在真实调用时产生。根因（vitest 4 + threads pool + SMB 的组合还是 mock 框架本身）**未查明**，记进 98.5。

### 98.4 验证台账（本机，全部真跑）

| # | 命令 | 退码 | 读数 |
|---|---|---|---|
| V1 | `cargo test -p axagent-plugin-proto`（axagent-core 根） | 0 | 本批新增 2 条（camelCase + 缺省项不进帧 / 七态词表精确小写）随全量绿 |
| V2 | `cargo clippy -p axagent-plugins -p axagent-plugin-stock --all-targets -- -D warnings` | 0 | 首轮红在 `PluginManifest` 两处构造缺 `task_sources` 字段（编译器抓到，非 clippy 语义）⇒ 补齐后绿 |
| V3 | `cargo test -p axagent-plugin-stock` | 0 | bin 单测 4 passed（行映射 / camelCase / 缺省端点不出现 / 状态词表）；`tests/protocol.rs` **11 passed**（新增帧级 `command_frame_lists_active_workflow_runs_as_task_center_rows`：camelCase 入参→snake_case 接缝参数翻译、缺省档 50、错误码原样透传不退化成空数组） |
| V4 | `cargo fmt --all --check`（axagent-plugins 根） | 0 | 单文件 `rustfmt --check src/commands/plugin.rs` 亦 0 |
| V5 | `npm run plugin:bundle -- --check` | 0 | 清单 4 条命令 + 1 个 taskSource + 1 个 bundle 页；`sha256 a908b260be5a…`；describe 握手 + shutdown 回执 + 退出码 0 |
| V6 | `AXAGENT_STOCK_BUNDLE_DIR=… cargo test -p axagent-plugins --test e2e_stock_bundle_install_test` | 0 | 装真产物 ⇒ 投影出 `stock-workflow-runs` ⇒ 执行该命令 ⇒ 回传一行 `nativeId=exec-1` / `state=running` / 无 `finishedAt`/`deepLink` |
| V7 | `cargo test -p axagent-plugins --lib` | 0 | **236 passed / 0 failed**（含本批 5 条清单级 + 反向「装不上」测试）。⚠ 这条是第一轮验证链**漏跑**的，正因漏跑 serde alias 缺陷才只由 V6 抓到 ⇒ 已把「库单测」写进链 |
| V8 | `cargo check -p axagent --lib`（src-tauri 根） | 0 | 首轮 15m38s（serde alias 修正前），终态复跑 **12m48s** ⇒ 两轮都 0 告警 0 error；重生成 `src-tauri/src/register_commands.rs`（新命令落在第 954 行，产物 diff 恰 1 行）|
| V9 | `npm run typecheck`（`scripts/check-tsc.mjs`） | 0 | 扫 1465 个 src 源文件，无诊断（98.3 第 4 条落地后又跑一次 = V13c） |
| V10 | `npm run test:run -- pluginTaskSource` | 0 | 11 passed（映射七态/表外落 waiting、`durationMs` 端点相减与倒挂、`openSource` 仅在有 deepLink 时开、非数组抛错、命令失败上抛、只挂 enabled、宿主优先忽略同名、空清单不 bump、单源失败隔离成一条 `SourceFetchError`） |
| V11 | `npm run test:run -- TasksCenterPage` | 1（本批已修） | 首轮 133 passed / 1 failed（即 98.3 的 vitest 假红）；修复后定点 V10 绿。**全目录复跑未做**（见 98.5），其余 9 个文件在该轮里全绿且此后未再改动 |
| V12 | `node node_modules/oxlint/bin/oxlint src/pages/TasksCenterPage src/types/index.ts src/lib/browserMock.ts --max-warnings=0` | 0 | 42 文件 0 warning 0 error（与 CI 同源命令） |
| V13 | `node node_modules/dprint/bin.cjs fmt <本批 9 个 TS 文件>` | 0 | 3 文件被重排；`scripts/**` 在 dprint excludes 里 ⇒ 打包脚本不受管辖 |
| V14 | 第五道门：`check-capability-register.mjs --selftest` + 判定（按驻留规程摘 PLAN 头部 3 行跑，`cmp` 校验 `RESTORED_IDENTICAL`） | 0 | `PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`；`--repin-code --apply` 回写 1 处（787→838）后复检无待回写 |
| V15 | `node scripts/audit-l1-blockers.mjs` + `--check-allowlist` | 0 / 0 | Rust 待改写 **99 处 / 16 文件**、+H 桶 3 处 = **102 处 / 19 文件**；前端反向阻塞 **52 处 / 38 文件** ⇒ **本批未动这两个数**（新增前端文件不 import 股票域）；H 桶白名单 3 条锚点全在 |
| V13b | `node node_modules/dprint/bin.cjs check`（RunsTab / adapter / 新测试）+ 复跑 V12 oxlint | 0 / 0 | 98.3 第 4 条改完后复跑：40 文件 0 warning、格式无 diff |
| V13c | `npm run typecheck`（改 RunsTab 依赖后复跑） | 0 | 1465 个源文件，无诊断 |
| V16 | `node scripts/check-i18n-orphan-diff.mjs --selftest` + 判定 / `check-jsx-text-i18n.mjs` / `check-component-neutrality.mjs` | 0 / 0 / 0 | 变更条目 207（src/** 内 8）、K 10434 / U 69 / 动态前缀 111 ⇒ 无孤儿候选；JSX 基线外 0；中立性原始 228、豁免后 225、基线 212 ⇒ 无新增（存量少 1 条未收紧棘轮） |

**镜像表与 PLAN 读数同批改**：`commandTotal` **1541→1542**、`commandNameSetParity` 1541→1542（三方 1542/1542/1542 全等）、`registryFileLines` 1608→1609（`split("\n")` 口径）、PLAN:366 的「`register_commands.rs` 共 1542 条 / 164 模块」、`commandModules.note` 的 PLAN 行引用 352→363。⚠ 只报告项 `k2KeyedFaces` 登记 37 强 / 实测 **38 强**：A/B 复扫（临时摘掉本批新增的 `tests/common` 替身 impl 后重跑 `--dump-readings`）读数**仍是 38** ⇒ 这 1 条不是本批引入，按纪律**不改登记值、不洗白**，等并行批落地后统一重算（该锚点 `hard:false`，不参与退出码）。

### 98.5 本批未做

1. **宿主股票两个源未删**（`stockAnalysis.adapter.ts` / `pipeline.adapter.ts` 仍是宿主实现）⇒ 本批只建通道、不做切换；切换批 = B8-1c，删完才是「插件源顶上了宿主源」。今天中心里同时有宿主 `stock-analysis` 与插件 `stock-workflow-runs`（后者只在装了插件并启用后出现）。
2. **真机未验**：`npm run tauri dev` 里打开任务中心、看插件源出现在筛选器与源 chip 上这条**没在桌面上点过**。已验的是：浏览器 mock 走空清单桩（不报错）、Vitest 单元/注册表口径、Rust 侧端到端 V6。⚠ `RunsTab.tsx` / `SourceChip.tsx` **没有组件级测试**（仓内本来也没有），所以 98.3 第 4 条那类「钩子依赖漏项」缺陷**测试拦不住**，全目录 Vitest 复跑也未做（改动文件不在任何测试的渲染树里）。
3. **插件源只有 `list`**：详情抽屉、步骤、日志、IO、历史、取消/重试全部未接（`RunCapabilities` 只给 `openSource`，且因 98.2-1 恒 false）⇒ 点开一条插件 run 没有抽屉。
4. **无事件推送**：新执行开始不会自己刷新列表，要重进页面或手动刷新（B3 事件面尚未接到任务中心）。
5. **卸载/禁用插件后**，已注册的源要等重进任务中心才消失（清单实时扫描，前端只在页面挂载期取一次）。本批刻意不做轮询，也不建第二份源清单。
6. **`list_active` 之外的取数面未接**：终态批次/历史执行需要宿主新增 op（或在 `workflow.run` 上开「近期已结束」），属 #35 整域搬迁批。**〔已由 §99 收口（2026-11-05，`workflow.run` 的 `list_recent`）〕**
7. **`PLUGIN_TASK_RUN_STATES` 与前端七态仍是人工两份**（前端已用 `Record<RunState, RunState>` 锁住自己那一侧）；跨语言同源要等 B8-3 的生成 DTO。
8. **vitest 假红的根因未查**：只在测试里绕开（98.3 第三条），没读 vitest mock 源码定论。若后续别的用例再撞，先查这条。
9. **`--update-baseline` 未跑**（中立性少 1 条存量）、**全量 `clippy --workspace` / `test --workspace` 未跑**：并行批仍在改前端与 Rust，按纪律留到 #35 全部阶段完成后一次收。

## §99 `workflow.run` 的 `list_recent`：让插件任务源看见**已结束**的执行（#35，2026-11-05）

任务 #150。§98.5-6 留的那个口子在本批收掉：`list_active` 只吐 running / paused，所以 §98 建的插件任务源「跑完即消失」，且 98.2-4 判掉的 `finishedAt` 恒缺 ⇒ 耗时列永远空。本批一次做穿 **契约（harness trait + DTO）→ 宿主 provider → worker 分派 → 插件取数命令 → 清单产物 → 前端消费**，六段同批落地。

### 99.1 形态裁定（四条，都是「先判据后动手」的结果）

| # | 裁定 | 判据 |
|---|---|---|
| 1 | **新增 op，而不是给 `list_active` 加 `include_finished` 参数** | 加参数会让同一个 op 有两种语义，而两者的排序键、`total` 口径、可达状态集都不同 ⇒ 分页契约说不清。两个 op 各自单义，夹档与缺省逐字同形（分派层两条分支的测试互为镜像） |
| 2 | **返回类型复用 `WorkflowRunActivePage` / `WorkflowRunActiveView`**，只给行加一个可选 `finished_at` | 两个 op 的行形状只差「是否含终态」⇒ 新造一份 DTO 是重复定义（禁区 12）。`#[serde(default, skip_serializing_if)]` 保证**既有 `list_active` 的线格式一字节没变**（键不出现，不是 `null`），所以 §98 的 e2e 断言「无 `finishedAt`」在同批里仍然是正确的、且改成了正向断言 |
| 3 | **`finished_at` 只认宿主登记的时刻，不由 `updated_at` 倒推** | 引擎终态行的 `updated_at` 是「最后一次状态写入」；总超时 / 执行体异常终止那两种终态是**宿主**判定的（`terminal_override`），宿主判定的那一刻引擎那边并没有写。⇒ 在 `release_slot` 与 `active = false` 同一次写锁内盖 `finished_at`，`recent_row` 只在该行**已退出额度**时透出。仍在占额度而引擎已写终态的过渡行 ⇒ `None`（宁缺毋造） |
| 4 | **候选窗口按登记表的 `created_at` 降序取 `limit` 条，再逐条问引擎** | 终态行只在登记表里（`engine.list_active_executions` 过滤掉它们，`engine/mod.rs:5576`），而「谁属于本插件」这条唯一权威也在登记表。反过来按 `updated_at` 选窗口要先给每条登记行问一次引擎 ⇒ 把 N 次查询花在选窗口上。代价：窗口内若有引擎查不到的行会被跳过，本页可能少于 `limit`（`total` 不回退，契约里明写「插件不得据 `total` 推页数」） |

⚠ **`list_recent` 不进 `SEAM_OP_TIMEOUT_OVERRIDES`**：那张表按**实测帧耗时**准入（任务 #140 立的纪律），而本 op 是「内存遍历 + 至多 `limit` 次内存查状态」，没有新 I/O ⇒ 走默认 15 s 预算。登记表随会话内执行数增长，每次调用是 O(n) 遍历 + O(limit) 查询 —— 今天 n 是个位数，未做裁剪（记进 99.5）。

### 99.2 命令改名的连带面（一次改六处，漏一处就是「装得上、取不到数」）

`list_active_workflow_runs` ⇒ **`list_recent_workflow_runs`**。任务源的语义从「在跑的」变成「近期的」，名字不跟着改就是留一句假话（§93 的命名纪律）。六处同批：

1. `plugin-stock/src/main.rs` 的常量与分发（`COMMAND_RECENT_RUNS`，函数名 `recent_workflow_runs`）；
2. 出站调用的 op 字面量 `list_active` ⇒ `list_recent`；
3. `scripts/build-plugin-bundle.mjs` 的 `commands[].name` / `command: "worker://…"` / 描述；
4. 同文件 `taskSources[].command`（宿主清单校验会硬拦「指向不存在的命令」⇒ 3、4 不同批改就装不上）；
5. `crates/plugin-stock/tests/protocol.rs` 的帧级用例（含 `finishedAt` 正向断言）；
6. `crates/plugins/tests/{common/mod.rs,e2e_stock_bundle_install_test.rs}` 的替身与端到端断言。

**§98.1 表里「取数」那一行写的旧名不改**：那是 B8-1b 那一批的时点快照，本批用本节记录改名，并把 §98.5-6 标成「已由 §99 收口」——快照不重写，只在原处指路。

### 99.3 可达性变化（把 §98 的三条「防御性」转正）

| 分支 | §98 的说法 | 本批之后 |
|---|---|---|
| `completed` / `partially_completed` / `failed` / `cancelled` 四条状态翻译 | 「当前不可达，不许当成实测到达过的路径」 | **可达**：`list_recent` 一页里就能出现终态行，e2e 用 `partially_completed` 走通「终态且有产出 ⇒ `succeeded`」 |
| `finishedAt` 不交 | 「`list_active` 只含在跑 ⇒ 恒不交」 | **交**，且值只来自宿主登记；`durationMs` 仍不在插件算 —— 两端点相减只在宿主 `resolveDurationMs` 一处 |
| `deepLink` 不交 | 站内无路由（2026-10-28 删） | **不变**，本批没造深链（代价照旧：这类 run 的 `openSource` 恒 false） |
| 前端 | —— | **零改动**：`PluginTaskRunRowDto.finishedAt` 与 `resolveDurationMs` 在 §98 就按「有就算、没就算不出」写好，本批一条 TS 都没动 ⇒ 耗时列自动出现 |

### 99.4 验证台账（本机，全部真跑）

| # | 命令 | 退码 | 读数 |
|---|---|---|---|
| V1 | `cargo check -p axagent-harness --all-targets`（axagent-core 根） | 0 | 2m41s，日志首行即 `Checking axagent-harness`（不是只吐 Compiling 的假绿形态），0 warning |
| V2 | `cargo test -p axagent-harness --lib workflow_run` | 0 | **5 passed / 0 failed**（930 被过滤）= 既有 4 条 + 本批 `active_view_finished_at_camel_case_and_absent_when_none`；`default_trait_methods_are_unimplemented` 现含 `list_recent` 一档 ⇒ 默认实现五个方法全是错误 |
| V3 | `cargo check -p axagent-plugins -p axagent-plugin-stock --all-targets` | 0 | 4m23s（含 `tests/` 与 bin 目标 ⇒ 新分派分支、两处替身、e2e 断言全部编过） |
| V4 | `cargo clippy -p axagent-plugins -p axagent-plugin-stock --all-targets -- -D warnings` | 0 | 与 CI 同参（`--all-targets` 才 lint 测试目标） |
| V5 | `cargo clippy -p axagent-harness --all-targets -- -D warnings` | 0 | 契约批的 lint 面（含新测试模块） |
| V6 | `cargo test -p axagent-plugin-stock` | 0 | bin 单测 **5 passed**（新增「终态行透传 `finishedAt` + 在跑行不出现该键」）；`tests/protocol.rs` **11 passed**（帧级 `command_frame_lists_recent_workflow_runs_as_task_center_rows`：op 是 `list_recent`、`workflowId`→`workflow_id` 翻译、夹档 20 / 缺省 50、错误码原样透传不退化成空数组） |
| V7 | `cargo test -p axagent-plugins --lib` | 0 | **238 passed / 0 failed**（§98 是 236 ⇒ 本批 +2：`workflow_run_list_recent_passes_terminal_rows`、`workflow_run_list_recent_defaults_match_list_active`） |
| V8 | `npm run plugin:bundle -- --check` | 0 | 产物清单命令改名生效 + 1 个 taskSource 指向新命令；`sha256 3c620a5251bee7c3…`（§98 是 `a908b260…`）；describe 握手 + shutdown 回执 + 退出码 0 |
| V9 | `AXAGENT_STOCK_BUNDLE_DIR=dist/plugins/stock cargo test -p axagent-plugins --test e2e_stock_bundle_install_test` | 0 | **1 passed**：装真产物 ⇒ 投影出 `stock-workflow-runs` / `command=list_recent_workflow_runs` ⇒ 回传**两行** —— `exec-1` `running` 无 `finishedAt`、`exec-0` `partially_completed`→`succeeded` **带** `finishedAt=1699999999000`，两行都不带 `durationMs`/`deepLink` |
| V10 | `cargo check -p axagent --lib`（src-tauri 根） | 0 | **23m06s**，0 warning（`list_recent` 宿主实现 + `RunMeta.finished_at` + `release_slot` 同锁盖戳） |
| V11 | `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::stock_workflow::workflow_run_service` | 0 | **6 passed / 0 failed**（894 过滤），含本批 `recent_row_overrides_terminal_and_gates_finished_at` 的四条判据。⚠ 这条是「`clippy --all-targets` 只 lint 测试、不跑测试」纪律的兑现，不是可省的尾巴 |
| V12 | `cargo fmt --all --check`（axagent-core / axagent-plugins / src-tauri 三根） | 0 / 0 / 0 | 先用单文件 `rustfmt --check` 定位：三处 diff 全是我自己写的行 ⇒ 手工改成 rustfmt 形态。**不许对他人文件跑写模式** —— `rustfmt file.rs` 会顺 mod 树重写整个模块 |
| V13 | 第五道门：`--selftest` + 判定（摘 PLAN 头 3 行的副本跑，`cp` 还原后 `cmp` = `RESTORED_IDENTICAL`） | 0 / 0 | 首轮 **2 条硬失败**（`workflow.reflector` / `workflow.evolver` 的三个 `worker.rs` 证据锚点因本批 +29 行位移）⇒ `--repin-code --apply` 回写 4266→4295、4358→4387、4406→4435（token / file / anchorText 一字未动，自动备份 `.tmp/capability-register.*.bak.json`）；复检 `PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）` |
| V14 | `node scripts/audit-l1-blockers.mjs` | 0 | Rust 待改写 **99 处 / 16 文件**、H 桶 3 处 ⇒ 合计 **102 / 19**，与登记值逐字相同 ⇒ **本批未动这两个数**；命令读数仍 1542 / 164 模块（本批没加 Tauri 命令，`register_commands.rs` 零 diff） |

**顺手抓出的既存腐烂（V13 同批改）**：`--repin-code` 只管 `evidence[]` 的 `line`，不管**同一张表 `note` 里**与 **PLAN §0.7 表格散文里**的 `worker.rs:3798` / `:3890`–`:3938`。用 `git show HEAD:…worker.rs` 复算：HEAD 上真实位置是 `:4290` / `:4358`–`:4406` —— **本批之前就已差 492 / 468 行**，不是本批改出来的。已把 PLAN:268–269 两行、镜像表两处 `note` 与两处 `planAnchorText` 一起重抄成盘上现值 `:4319` / `:4387`–`:4435`（`grep -n` 给号）。§98.3 第二条立的「散文可定位巡检」**仍未立项**，这次靠人撞 + `git show` 复算才定性 ⇒ 判据缺口继续挂着。

**未跑的 CI 口径全量**：`clippy --workspace --all-targets --all-features -- -D warnings` / `test --workspace` / 全目录 `npm run test:run` 本批**未跑**（理由同 §98.5-9：并行批仍在改同一编译面）。主 crate 只跑了**定点** `--lib commands::stock_workflow::workflow_run_service`，`clippy -p axagent --all-targets`（会编全部 target）也未跑 ⇒ 记进 99.5，**不许当成「已按 CI 口径验过」**。

### 99.5 本批未做

1. **没裁登记表的内存增长**：`RunRegistry.runs` 只增不减（`release_slot` 只翻 `active`、从不删行），`list_recent` 每次调用是 O(n) 遍历 + 至多 O(limit) 次内存查状态。今天 n = 「本次会话该插件发起过的执行数」，量级个位 ⇒ 无感；但任务中心自动刷新会按 `ACTIVE_POLL_MS` 轮询，长期挂着线性变慢。**待办**：给登记加保留窗口（如按 owner 留最近 N 行）或加缓存 —— 两者都会引入第二份状态，需先定口径再动。
2. **`total` 与本页可能不一致这条没有测试**：契约写明「`total` 取登记匹配数，引擎查不到的行只从本页剔除」，但要造「登记有、引擎无」得 mock `WorkEngine`，而本文件的测试全是纯函数级。今天按「登记表与引擎同生命周期、重启双双清空 ⇒ 实际不出现」把它判成防御性分支，**没有实测到达过**，不许在后续记账里当实测路径引用。
3. **宿主 `list_recent` 本体未跑过真引擎**：V11 验的是纯函数 `recent_row` 的四条判据；遍历 / 窗口截断 / 页内排序要真 `WorkEngine` 才能测（同 `list_active` 自 §50 以来的既有覆盖面）。已验的是分派层（V7）、插件侧（V6）、真产物端到端（V9）与编译（V10）。
4. **没有按状态过滤**：任务中心要「只看失败」时只能全量拉回再前端筛；`list_active` / `list_recent` 都没有 status 参数。
5. **翻页未接**：有 `limit`、有 `total`，没有 offset/cursor ⇒ 一次只拿最近 50 条，历史翻不出来（§98.5-3「插件源只有 list」在这里同样成立）。
6. **B8-1c 仍挂起**（任务 #149）：宿主 `stockAnalysis.adapter.ts` / `pipeline.adapter.ts` 未删 —— 详情抽屉按 `source` 字面量分派（`RunDetailDrawer.tsx:36`、`:45`），删了会退化 ⇒ 须先做 B8-2 页面/抽屉迁移。本批补齐的是插件源的**数据面**（在跑 + 终态 + 耗时），**详情面**依然缺。
   ⇒ **其中「详情面按 source 字面量分派」这条前置已由 §100（任务 #151）收口**，上面两个行号是 §99 时点快照，勿按现盘行号找。#149 剩余的真实阻塞是**取数接缝**（该源 `list()` 打的是股票域宿主命令），不是分派形态。
7. **真机未验**：桌面里打开任务中心、看一条已结束的工作流批次带着耗时出现在插件源下 —— 这条**没在 `npm run tauri dev` 里点过**；已验的是 Rust 侧端到端与单元/分派口径。
8. **`PLUGIN_TASK_RUN_STATES` 与前端七态仍是两份人工镜像**（同 §98.5-7，等 B8-3 生成 DTO）；`--update-baseline`（中立性棘轮）与全量门禁同 §98.5-9 继续挂。


## §100 B8-2 第一刀：任务中心**详情面**改由源声明（#35，2026-11-05）

任务 #151。§99.5-6 判掉的「详情面」前置在本批收掉。

**为什么先切这一刀而不是直接删宿主股票源**（#149）：`audit-l1-blockers` 把任务中心的股票反向阻塞记成 10 处 / 6 文件，但只有 `RunDetailDrawer.tsx` 那 1 处是**纯前端就能解**的 —— 其余 9 处要么需要取数接缝（registry 里那两个股票适配器打的仍是股票域宿主命令），要么是产品决策（把审批健康卡从概览 Tab 移走会改 UX）。先做结构上零风险的一处，顺带把 #149 的阻塞理由从「分派形态」纠正为「取数接缝」——后者才是真原因。

### 100.1 形态裁定（四条）

| # | 裁定 | 判据 |
|---|---|---|
| 1 | **详情形态由适配器声明（`renderDetail?(ctx): ReactNode`），中心不再认识任何源** | 改造前中心有五条 `selected.source === "…"` 字面量判断，其中一条 import 股票域抽屉组件 ⇒ 中性分发器「知道每个源怎么渲染」。后果是双份的：每加一个源要回来改这个文件，且这个文件永远是域耦合的落脚处。改成注册表取回后，中心只剩「取适配器 → 调它 → 没有就兜底」 |
| 2 | **`ctx` 只给 `run + close` 两样，不给源特定 props** | 五个详情组件的入参本来就不同（pipeline 要整条 `run.raw`、两个批次只要 `nativeId`、cron 要 `taskId + title`）。中心若替它们拆参数，就等于把五个源的 props 形状抄进中性文件 ⇒ 让各源自己在 `renderDetail` 里从 `ctx` 取所需 |
| 3 | **`renderDetail` 是**可选**方法，未声明走空抽屉兜底** | §93 已裁「插件只交数据不交渲染」（第三方 React 不得进主 origin）⇒ 插件贡献的源永远不声明它。兜底分支与改造前**逐字一致**（同一个 `Drawer` + `tasksCenter.runs.detailUnavailable`），插件源今天点开的表现没变 |
| 4 | **适配器用 `createElement` 而不是 JSX** | 五个适配器都是 `.ts`；为一个表达式把它们全改名成 `.tsx` 会把 diff 面扩大到文件重命名。`createElement(Comp, props)` 的 props 仍受 TS 检查 ⇒ 组件改签名照样红 |

**cron 的执行历史抽屉**（`CronHistoryDrawer`）原是 `RunDetailDrawer.tsx` 里的**内部函数**，只有 cron 源用。既然详情形态归源自述，它就不是「中心的一部分」⇒ 原样移成 `jobs/CronHistoryDrawer.tsx`，与本源其它详情组件同层。搬移零逻辑改写。

### 100.2 改动面（10 文件，全在 `src/pages/TasksCenterPage/`）

| 文件 | 改动 |
|---|---|
| `adapters/types.ts` | 加 `RunDetailContext`（`:12`）与可选 `renderDetail?(ctx): ReactNode`（`:65`） |
| `RunDetailDrawer.tsx` | 211 行 → **45 行**；删五条 source 字面量分支与四个抽屉 import，改 `getAdapter(selected.source)?.renderDetail?.({ run, close })`（`:28`） |
| `jobs/CronHistoryDrawer.tsx` | **新建**（145 行），自原内部函数原样移出，加 `CronHistoryDrawerProps` 命名导出 |
| `adapters/{pipeline,stockAnalysis,timeTravel,moverRecallAnalysis,cron}.adapter.ts` | 各加一条 `renderDetail` 声明 + 对应抽屉 import |
| `__tests__/RunDetailDrawer.test.tsx` | **新建** 3 条用例 |
| `jobs/PipelineRunDetailDrawer.tsx` | `:4` 的 `usePipelineStore` 由 `@/stores` barrel 改为直连 `@/stores/feature/pipelineStore` —— **100.3 那条模块环的断点** |

### 100.3 本批撞出的缺陷：详情面上移把**模块环**接上了

全目录那一跑（100.5 W5）没有白跑 —— 它抓到的不是「某个用例红」，而是**两只测试文件根本没被收集**。链路：

```
adapters.test.ts 先 import ../adapters/cron.adapter
  → cron.adapter → jobs/CronHistoryDrawer → @/stores（barrel）
    → stores/index.ts → feature/taskCenterStore.ts:3 → adapters/registry.ts
      → registry 求值期就执行 REGISTRY.map(a => a.id)（:34）
        → 此时 pipeline/cron 适配器还在求值中 ⇒ 数组元素 undefined ⇒ TypeError
```

**为什么改造前没有这个环**：`@/stores` barrel 原先只被**中心**（`RunDetailDrawer.tsx`）与页面引用，适配器模块一个都不碰它 ⇒ 从适配器一侧进入时不会回到 registry。本批把抽屉 import 搬进适配器，等于把「组件图 ⇄ store 图 ⇄ 注册表图」三者首尾相接。**类型检查抓不到它**（W3 全绿）：环是**求值顺序**问题，不是类型问题 —— 只有「从哪一侧先进入模块图」才决定炸不炸，而页面入口恰好是安全的那一侧，所以**真机上今天也不会红**。这种「测试入口红、应用入口绿」的形态最阴，必须当场断掉而不是靠入口运气。

**断点选在哪**：`PipelineRunDetailDrawer.tsx:4` 是这条环上唯一一处 `@/stores` barrel 引用（其余四个抽屉都不碰 store）。改成直连具体 store 模块（`pipelineStore.ts` 自身只依赖 `invoke` / `@/types` / `zustand`，是叶子），环即断开。

- ⚠ **先试过、被推翻的修法**：给两只测试的 `vi.mock("@/lib/invoke")` 补 `isTauri` 等导出（仓内 `OverviewTab.evolutionApproval.test.tsx` 的 `importOriginal` 展开形态）。它能让 `pluginTaskSource.test.ts` 过，但 `adapters.test.ts` 随即换成 `adapter.id` 的 TypeError —— **因为缺导出只是环的第一个症状，不是根因**。改完根因后这两处 mock 已**还原成零 diff**，不留「放宽替身凑绿」的后门（那条展开形态本身是合法的，但用它盖住环会让下一个 barrel 泄漏继续静默）。
- 保留 `renderDetail` 的**双向** import 环（`pipeline.adapter ⇄ PipelineRunDetailDrawer`）：两处引用都只发生在函数体/闭包内（`createElement` 在 `renderDetail` 箭头函数里，`mapPipelineState` 在组件 render 里），模块求值期互不取值 ⇒ 安全。断的只是**经过 registry 求值期**那一圈。


### 100.4 本批**不做**的 9 处（逐条写原因，不许当「顺手能清」）

| 站点 | 处数 | 为什么本批解不了 |
|---|---|---|
| `adapters/registry.ts:12` | 1 | 它 import `stockAnalysis.adapter` 是为了注册。注册表要中性，前提是**那两个股票适配器不再是宿主代码**（= #149，被取数接缝阻塞）或注册面本身插件化（= B8-3） |
| `OverviewTab.tsx:3,6,7` | 3 | 进化健康卡直接打股票域 store / 命令。移走它会改概览 Tab 的信息架构 ⇒ 属产品决策，按「先说副作用再实现」另批 |
| `__tests__/OverviewTab.evolutionApproval.test.tsx:2,3` | 2 | 上一条的测试镜像，随它一起走 |
| `forms/TaskCreateModal.tsx:25,26` | 2 | 创建表单内联股票任务的字段与模板取数；需要 T2 命令通道上的插件字段声明（B8-4） |
| `__tests__/adapters.test.ts:17` | 1 | 它 import 股票适配器做断言，随 #149 收口 |

PLAN §0.10 的前端读数因此从 **52 处 / 38 文件 → 51 处 / 37 文件**（任务中心 10 处 / 6 文件 → 9 处 / 5 文件），**本批只清掉 1 处** —— 这就是这一刀的体量，别把它读成「任务中心已解耦」。

### 100.5 验证台账（本机，全部真跑）

| # | 命令 | 退码 | 读数 |
|---|---|---|---|
| W1 | `dprint fmt` → `dprint check`（本批全部改动文件） | 0 | fmt 首轮改写 3 个文件；断环后对 `jobs/PipelineRunDetailDrawer.tsx` 复查仍 check 全绿 |
| W2 | `oxlint src/pages/TasksCenterPage --max-warnings=0` | 0 | **Found 0 warnings and 0 errors，42 files**（与 CI 同源；本仓走 oxlint，不是 ESLint） |
| W3 | `npm run typecheck`（`scripts/check-tsc.mjs`） | 0 | 日志首行「tsc 发现 src 源文件 **1467 个**」⇒ 扫描面非空，不是 UNC 下的静默空跑；`createElement` 的 props 由 tsc 实检。⚠ **它对 100.3 那个模块环完全无感** —— 环是求值顺序问题，不是类型问题。断环后复跑同退 0 |
| W4 | `npm run test:run -- RunDetailDrawer` | 0 | **1 file / 3 tests passed**（声明源渲染其 MARKER 且带上 `nativeId` / 未声明源落 `detailUnavailable` 兜底 / 源不在注册表也不崩） |
| W5 | `npm run test:run -- TasksCenterPage`（全目录） | **报 0，实为红** | ⚠ **假绿实例**：后台任务报 exit 0，但日志里 `adapters.test.ts (0 test)` 与 `pluginTaskSource.test.ts (0 test)` 两条带 `❯`，且**没有结尾汇总行** ⇒ 「退 0 ≠ 检查跑了」在本仓再次应验。定点复跑退 **1**，首条错误 `No "isTauri" export is defined on the "@/lib/invoke" mock` ← `cacheStore.ts:40` ← `stores/index.ts:45` ⇒ 顺藤摸到 100.3 的模块环。**这一跑赚回了它的 16 分钟**，判据见本节末「关于验证面」 |
| W6 | `node scripts/audit-l1-blockers.mjs` | 0 | 前端 **51 处 / 37 文件**（改前 52/38，见 `AUDIT-l1-blockers-2026-11-05-0705.md` 与 `-0944.md` 对照）；`RunDetailDrawer.tsx @15` 条目**消失**；Rust 侧 99/16 + H 桶 3 未动 |
| W7 | `check-component-neutrality.mjs --selftest` + 判定 | 0 / 0 | 「PASS: 无新增违规（基线 212 条，**本轮已修复 1 条可择机 `--update-baseline` 收紧**）」——并行期**不跑** `--update-baseline`（会把别人未提交的减少钉进棘轮），留 100.6。断环后复跑同读数 |
| W8 | `check-i18n-orphan-diff.mjs --selftest` + 判定 | 0 / 0 | 「PASS: 无孤儿候选」。⚠ 覆盖面只到**已跟踪**的改动文件（新建两只是 untracked，不进 `git diff HEAD`）⇒ 这条门对本批不覆盖新文件。断环后复跑同退 0，`K` 仍 10 434、判定不变（该门报的是「孤儿候选」，本批一个都没造出来） |
| W9 | `check-jsx-text-i18n.mjs --selftest` + 判定 | 0 / 0 | 扫 781 个 `.tsx`，命中 51 全在基线，**基线外 0** |
| W10 | `check-i18n-exemption-budget.mjs --selftest` + 判定 | 0 / 0 | 存量 142 条冻结，无基线外豁免问题 |
| W11 | 行尾自证：`node` 读 latin1 数 `\r` | 0 | **ALL_ZERO_CR（12 文件）** = 10 个源码文件 + PLAN + `capability-register.json`（本机 `grep -c $'\r'` 是假值，判据只用 latin1） |
| W12 | 断环后定点复跑 `adapters.test` + `pluginTaskSource` | 0 | `pluginTaskSource.test.ts` **1 file passed**、`adapters.test.ts` **1 file / 40 tests passed**。⚠ 中间一次 `adapters.test` 单跑撞上 `[vitest-pool]: Timeout waiting for worker to respond`（Duration 60.09 s、`transform 0ms` ⇒ worker 没起来，不是本批代码）——即 #141 的 60 s 启动抖动，**重试即绿**，不许把它读成用例红。两只测试的 `vi.mock` 已还原成零 diff（`git diff --stat` 空） |
| W13 | 第五道门 `--selftest` + 判定（摘 PLAN 头 3 行的副本跑；`cp` 还原后 `cmp` = `RESTORED_IDENTICAL`，共 3 次驻留） | 首轮 0 / **1** ⇒ 复检 0 / 0 | 判定首轮 **1 条硬失败**：`frontendFileCaliber` 登记 85 242 / 实测 **85 248**（差 6）。归因已复算：本批 10 个文件里**只有 `adapters/stockAnalysis.adapter.ts` 命中 `feRe`**，`git diff --numstat` = `6 0`、`split('\n')` 90→96 ⇒ +6 全部由本批造成，且它是**涨**不是降（详情声明从中心搬回域内适配器的净结果）。同批改登记表 `value` / `planNumber` / `planPinned.mustContain` / `note` 四处 + PLAN §0.10.1 表那一行（盘上 `:522`）⇒ 复检 `PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`，该锚点转 `PASS: … = 85248`。`frontendDirCaliber`（`components/stock-analysis`）与其余 20 项锚点本批未触及。⚠ 顺带复核 `k2KeyedFaces` 登记 37 / 实测 38 仍是**只报告**档（等并行批落地重算，处置同 §99.4 V13） |

**Rust 侧零改动**：本批一行 `cargo` 都没跑 —— 纯前端 10 文件，`cargo check` / clippy / fmt 与批无关。

**关于验证面**（用户 2026-11-05 纠偏，已入用户级记忆 `feedback-verify-scope-by-risk`）：按新口径，W5 那种「顺手把整目录再扫一遍」默认不该做 —— 本批风险面是「分派形态换了 + 一个内部函数搬家」，`typecheck` + 单文件用例 + grep 引用面按理够。**但 W5 抓到的模块环是 typecheck 结构上抓不到的那类东西**（它只看类型，不看「从哪一侧先进入模块图」）。所以正确的结论不是「全目录不该跑」，而是：**当改动会把组件图接进 store 图/注册表图时，必须跑一次「从被引用方一侧进入」的加载**，而这件事 `adapters.test.ts` 一只文件（约 30 s）就能做到 ⇒ 便宜的定点动作若能命中判据，就不该换贵的全量动作。下次照此收窄。


### 100.6 本批未做

1. **中立性棘轮没收紧**：W7 报「已修复 1 条」⇒ `--update-baseline` 应把基线 212→211，但并行批仍在改前端，按纪律留到 #35 阶段末一次收（同 §98.5-9）。
2. **`renderDetail` 没有真抽屉的渲染测试**：W4 用的是声明式替身（MARKER 组件），验的是**分派机制**；五个真详情组件各自的渲染用例仍住在它们自己的测试里。⇒ 「cron 源点开能看到执行历史表格」这条**没在本批被测过**，只是逐字搬移 + 类型检查过。
3. **真机未点**：`npm run tauri dev` 里从 Runs 表逐源点开抽屉没走过（同 §99.5-7 口径）。
4. **`registry.ts` 那 1 处未清**：见 100.4，被 #149 的取数接缝挡住。
5. **B8-2 的「页面」半边未动**：本批只切详情面，任务中心的**创建表单 / 概览卡 / 注册表**三处站点仍待 B8-4 与 #149。
6. **模块环只验了一条入口**：100.3 的断环由 `adapters.test.ts`（从适配器一侧进入）与 `pluginTaskSource.test.ts`（从 registry 一侧进入）两条路径证实。其它入口顺序（页面直入、`registerPluginTaskSources` 运行期追加后再进入）**没有穷举**。⚠ 更根本的一条没做：**`registry.ts:34` 仍在模块求值期解引用适配器的 `.id`** —— 把它改成惰性建 `Map`（首次 `getAdapter` 时构建）能从根上免疫这一类环，但会把「加载即崩」换成「静默 undefined 源」，判据方向需要先定，本批不动。
7. **同形态的隐患留了一处**：`jobs/PipelineJobCard.tsx:6` 仍 import `@/stores` barrel。它今天不在适配器图里（只有页面引用）⇒ 不成环；但一旦将来有适配器引用它，100.3 的环原样复发。**没有加防御**（门禁层面「适配器不得 import 经 barrel 拉 store 的组件」是可以写成一条静态检查的，属新判据，未立项）。
8. **`--update-baseline`（中立性 212→211）与全量门禁同 §98.5-9 继续挂**；`frontendFileCaliber` 已按实测回写（W13），**不代表** #35 阶段终口径收口。


## §101 B8-2 第二刀：`deepLink` 的**宿主侧消费者**（#35，2026-11-05）

任务 #152。本批不是从计划表里挑的，是**查 #149 前置时撞出来的缺陷**：为判断「删掉宿主 `stockAnalysis` / `pipeline` 适配器后详情面还剩什么」，逐个去查 `capabilities.openSource` 的消费点，结果是**零** —— `cronAdapter` 每一行都写 `deepLink: "/tasks?tab=schedules"` 且置 `openSource: true`，而全仓没有任何组件读这两个字段（唯一的 `deepLink` 读取者在 `StockAnalysisRunDrawer` 里，它读的是抽屉自己的「打开」按钮）。

### 101.1 三条裁定

| # | 裁定 | 判据 |
|---|---|---|
| 1 | 按钮放 **Runs 表操作列**，不放行点击 | 行点击已绑详情抽屉（`selectRun`）。「打开」与「详情」是两种意图（一个跳原页面、一个看摘要），塞进同一个手势会互相顶掉。操作列已是「详情」的位置 ⇒ 并列最自然 |
| 2 | 只认 `run.deepLink`，**中心不重拼路由** | 路由构造归各源：cron 给 `/tasks?tab=schedules`，stock-analysis 给 `analysisRecordPath(...)`。中心若自己拼，就又把「谁该跳到哪」收回来了 —— 与 §100 同一条纪律 |
| 3 | 判据用 **`deepLink` 是否存在**，不用 `capabilities.openSource` | 两者今天恒等（`pluginTaskSource.adapter.ts:98` 就是 `Boolean(deepLink)`），但 `openSource` 是**能力矩阵**用的布尔，地址在 `deepLink`。用布尔判存在会让两者有机会分叉（布尔 false 但有地址 ⇒ 静默不出按钮，且查不出为什么）。`openSource` **未删** —— 它是插件行 DTO 的字段，删它要动 `plugin-proto`；记为冗余，见 101.5 |

### 101.2 改动面（13 文件）

| 文件 | 改动 |
|---|---|
| `src/pages/TasksCenterPage/RunsTab.tsx` | 操作列 80 → 130 宽；`详情` + 条件渲染的 `打开`；`useNavigate`；`e.stopPropagation()`（否则点「打开」会连带开详情抽屉） |
| `src/i18n/locales/*.json`（11 个） | 新增 `tasksCenter.action.open`：打开 / 開啟 / Open / 開く / 열기 / Öffnen / Ouvrir / Abrir / Открыть / فتح / खोलें |
| `src/pages/TasksCenterPage/__tests__/RunsTab.openSource.test.tsx` | **新建** 3 条：带 `deepLink` ⇒ 出按钮且跳到该地址、不打开抽屉；不带 ⇒ 只有「详情」；多行时只有带链接的那行出按钮 |

### 101.3 行为变化（如实列，含新出现的可点路径）

- **cron 源真的能跳了**：改造前那 100% 的 cron 行都声明了 `deepLink` 却点不到；现在每行多一个「打开」，跳到 `/tasks?tab=schedules`。这是**新增的可用路径**，不是纯重构。
- **插件源获得唯一的返程**：插件贡献的任务源没有详情抽屉（§93 裁「第三方 React 不进主 origin」），抽屉兜底只有一句「暂未接入」；若插件行带 `deepLink`，用户至少能从列表回到原页面。**这是 #149 删宿主适配器的部分替代品，不是全部** —— 见 101.5-1。
- **i18n 保序 + LF**：11 个 locale 由脚本按「插入到 `action.detail` 之后」重写，写前先验证 `JSON.stringify(parse(x), null, 2)` 与原文件**字节等价**（11/11 `ROUNDTRIP_IDENTICAL`），避免全量重写引入无关 diff；写后逐个抽查键序 `…detail,open,retry…` 与 CR=0。

### 101.4 验证台账（本机，全部真跑）

| # | 命令 | 退码 | 读数 |
|---|---|---|---|
| X1 | `dprint fmt`（`RunsTab.tsx` + 新测试） | 0 | 无改写（首跑即合格式） |
| X2 | `oxlint src/pages/TasksCenterPage --max-warnings=0` | 0 | **0 warnings / 0 errors，43 files** |
| X3 | `npm run typecheck` | 0 | **1468 个源文件**（+1 = 新测试），退 0 |
| X4 | `npm run test:run -- RunsTab.openSource` | 0 | **1 file / 3 tests passed** |
| X5 | `check-i18n-orphan-diff` `--selftest` + 判定 | 0 / 0 | **K 10 435**（+1 = 新键静态引用），U 21，无孤儿 |
| X6 | `check-jsx-text-i18n` `--selftest` + 判定 | 0 / 0 | 781 个 `.tsx`，命中 51 全在基线，基线外 **0**（新按钮走 `t()`） |
| X7 | `check-i18n-exemption-budget` `--selftest` + 判定 | 0 / 0 | 存量 142 冻结，无基线外问题 |
| X8 | `check-component-neutrality` `--selftest` + 判定 | 0 / 0 | 922 文件，基线 212，无新增违规 |
| X9 | `node scripts/audit-l1-blockers.mjs` | 0 | 前端 **51 处 / 37 文件**（本批不动耦合面：`RunsTab.tsx` 只 import `./adapters/registry`，非域→非域） |

**Rust 侧零改动**：本批纯前端 + locale，一行 `cargo` 未跑。

### 101.5 本批未做

1. **#149 仍不能做**：删宿主 `stockAnalysis` 适配器的后果已查清 —— 那个抽屉是**通往单股分析记录的唯一路径**（`StockAnalysisRunDrawer` 里的「打开」读的正是 `run.deepLink`），而列表级的「打开」只给跳转、不给摘要。⇒ 替换宿主适配器要求 §93 的富卡片通道先落地。`stock-pipeline` 同理（它的抽屉读 `get_pipeline_run_detail` 的逐股明细）。本批的按钮**降低了**这个损失，但没有消除。
2. **`capabilities.openSource` 成了纯冗余字段**：宿主侧已改判 `deepLink`，插件行仍产出它。收掉要改 `plugin-proto` 的 DTO 与两端校验，属跨语言契约批，不夹带。
3. **OverviewTab 的 run 卡片未加「打开」**：那里是「最近运行」摘要卡，点击也是开抽屉；要不要加同一个按钮属产品取舍，未动。
4. **真机未点**：`npm run tauri dev` 里点一次 cron 行的「打开」没走过（同 §99.5-7 / §100.6-3 口径）。
5. **`--update-baseline` 与全量门禁**同 §98.5-9 继续挂。

## §102 任务源详情槽：把 §93 的 schema 卡片接到任务中心（设计，待实现）

**为什么需要它**：#149 删宿主 `stockAnalysis` / `pipeline` 适配器的最后一颗钉子，是**详情面没有替代品**（§101.5-1 已查实：`StockAnalysisRunDrawer` 是通往分析记录的唯一路径）。§93 已裁定插件卡片只许两种形态，「schema 卡片」是首选，但当时只接到 `pages[]`（`PluginPageHost.tsx:108`）与消息侧未建的插槽 —— **任务中心的详情槽不在其中**。

**本设计只钉形态与判据，不含实现**（实现要改 `src/types/**`，该目录正被 B8-3 批占用；避免同批互编）。

### 102.1 三条落点（都已核实存在，不是设想）

| 环 | 现状 | 本设计怎么用 |
|---|---|---|
| 清单声明 | `manifest.taskSources[]` 由 `build_manifest_task_sources`（`axagent-plugins/crates/plugins/src/manager.rs:2965`）校验并转写；`plugin_list_task_sources`（同文件 `:695`）只读投影 | 新增可选 `detailSchema: string`（schema id），校验规则同 `pages[].entry` 的 schema 支：**必须是本插件 `schema.json` 里声明的 id**，否则安装期报错（不留到运行期白屏） |
| schema 注册 | 插件 `schema.json` 在安装/启用/更新时收敛进 dynamic UI 库，`owner_id = pluginId`；卸载时 `revoke_schemas_owned_by` 撤销（`src-tauri/src/commands/plugin.rs:398`） | 宿主**不再新造注册通道**，直接复用：详情槽按 `(pluginId, detailSchema)` 取 |
| 渲染 | `DynamicUIStandalone`（`src/components/dynamicUI/DynamicUIStandalone.tsx:19`）接受 `schemaId` + `dataContext`（`:169` 把合并后的 context 传给渲染器） | 详情槽 = `<DynamicUIStandalone schemaId={detailSchema} dataContext={run.raw} />`。插件行的 `raw` 已经是插件自带投影（`PluginTaskRunRow.raw`），**不新增取数命令** |

### 102.2 判据（先写死，实现时照做）

1. **插件不交渲染代码**：只交 `(schemaId, 数据)`；宿主侧不出现任何 `import` 插件产物的路径（§93 的安全边界）。
2. **`renderDetail` 仍是宿主适配器的可选方法**：插件源由 `createPluginTaskSourceAdapter` 在运行时**补上**一个 `renderDetail`，其内容就是上面那一行 —— 于是 §100 的分派机制零改动，中心仍不认识任何源。
3. **schema 取不到就落既有空抽屉兜底**，不白屏、不抛错（与 §100 未声明分支同形）。
4. **`deepLink` 与 `detailSchema` 可以同时存在**：详情看结构化数据，`打开` 跳原页面；两者不互斥（列表级「打开」已由 §101 提供）。
5. **安全面必须先验一条**：schema 的 `onMount` / 事件动作能否触达宿主能力（navigate / IPC）。**结论已出（2026-11-05 实测）：能，且无闸** —— `actions[].type === "invoke"` 在无 `permissions` 上下文时直接调 `invoke()`（`src/lib/actionRouter.ts:325`、`:339`）。细节与影响面见 102.4-1（任务 #155）。**这条不解决，本设计不得实现。**

### 102.3 实现面（预估 6 文件，全部在既有接缝上）

| 文件 | 改动 |
|---|---|
| `axagent-plugins/crates/plugin-proto`（`PluginTaskSource` 形状） | 加 `detail_schema: Option<String>`（wire 名 `detailSchema`） |
| `axagent-plugins/crates/plugins/src/{manager.rs,types.rs}` | Raw 清单解析（多词字段必须逐条 `#[serde(alias)]` —— `RawPluginManifest` 无 `rename_all`，漏 alias 是**静默空值**）、校验「schema 属于本插件」、投影透出 |
| `src/types/**` | `PluginTaskSourceDto.detailSchema?: string`（**需与 B8-3 批串行**） |
| `src/pages/TasksCenterPage/adapters/pluginTaskSource.adapter.ts` | 声明 `renderDetail`：有 `detailSchema` ⇒ `createElement(DynamicUIStandalone, …)`；没有 ⇒ 不声明（走兜底） |
| `__tests__/pluginTaskSource.test.ts` + 一条渲染用例 | 声明透传 / 无声明走兜底 / schema 缺失不崩 |

### 102.4 本设计未定的两件事

1. **⚠ 已实测：声明式 `invoke` 动作没有命令闸 —— 第三方 schema 今天就能调任意 Tauri 命令。** 链路：`SchemaValidator` 只用 `ALLOWED_INVOKE_PREFIXES = ["dynamic_ui_"]` 校验 **`dataSource.config.endpoint`**（`src/lib/dynamicUI/SchemaValidator.ts:225`），而 **`actions[].type === "invoke"` 走另一条路**：`executeActions` 构造 `ActionContext` 时**不传 `permissions`**（`src/lib/dynamicUI/EventHandlerEngine.ts:39-44`），`ActionRouter` 的判据是 `if (ctx.permissions) { …白名单校验… }`（`src/lib/actionRouter.ts:325`）⇒ 无权限对象时**直接 `invoke(action.command, action.args)`**（同文件 `:339`）。
   - **影响面不止本设计**：`PluginPageHost` 的 schema 分支（`page.loading === "schema"`）今天就用同一条渲染链 ⇒ **插件页 schema 已经具备调用任意已注册 Tauri 命令的能力**。§93 立「第三方不许进主 origin，因为能摸 IPC」这条理由，在**声明式动作**这一侧事实上没有兑现。
   - **为什么本批不改**：收紧会改变现有 schema 能力的边界（Agent 生成 / 用户导入 / 插件导入三类来源的信任级别不同），必须先定「谁能拿 permissions、白名单给到什么粒度」，属独立安全批（任务 #155）。§102 的详情槽**必须等它落地**，否则等于把这条口子再扩大一处。
   - **判据方向（供 #155）**：给 schema 记录带来源标记（`owner_id` 已能区分插件），插件来源的 schema 在渲染时注入一份受限 `permissions`（至少 `commands: ["dynamic_ui_*"]` + 显式声明的读命令），用户/Agent 来源维持现状或同样收紧 —— 二者选一都必须先回答「业务命令谁允许」。
2. **`raw` 的数据形状契约**：`dataContext={run.raw}` 意味着 schema 直接绑定插件给的字段名。要不要在 `plugin-proto` 里给 `raw` 一个「建议形状」（如 `{ summary: [...], rows: [...] }`）以便宿主侧校验 —— 目前 `raw` 是 `unknown`，宿主不校验。倾向**不校验**（校验等于第二套 DTO），但要在文档里写明「schema 与 raw 由插件自己对齐」。

## §104 #36 OPC 迁出：29 处跨包口**全部同源**，路线改为整棵子树搬迁（2026-11-05）

并行批（任务 #158）实测结论，**推翻了 §0.10.1 桶② 的「走 call_seam」默认路线**：

1. **29 处全部是 `axagent_analysis_engine::opc::*`**，没有一处指向真正的股票符号 ⇒ 「改导入源」（①类）与「搬几处」（②类）都不成立，只剩**整棵子树原子搬迁**。
2. **根因是位置而非依赖**：OPC 业务代码物理寄居在股票 crate `analysis-engine/src/opc/`（28 文件 / 12,899 行，`lib.rs:79 pub mod opc;`），而该子树对 `analysis-engine` 的股票模块**零引用**（全部 `super::` 内部 + harness/entities）⇒ 搬到 `company-runtime/src/opc/` 后**不需要**加回股票依赖，也不产生反向边。
3. **6 条接缝方案降为备选**：`domain_pack.read/write`、`opc.kpi_data`、`opc.learning`、`opc.business_crud`、`capability.inventory`。其中 `opc.business_crud` 等于把 45 个 service/DTO × 57 条命令全量 RPC 化；`Arc<dyn Trait>` 无法跨进程（`opc.kpi_data`）；`opc_workflows/mod.rs:52,59` 是 `pub use`，**接缝根本转发不了**。⇒ 接缝是在给「位置放错」付网络开销，不是解耦。

### 104.1 本批实际改动（1 处，语义零变化）

| 文件 | 改动 | 依据 |
|---|---|---|
| `axagent-quant/crates/company-runtime/Cargo.toml:15` | 删除 `axagent-analysis-engine` 依赖边（留 4 行注释说明） | 全 crate 对它**零代码引用**（`grep -rn "axagent_analysis_engine" …/company-runtime/src` 空；仅 `self_improving.rs` 注释提到 `StockAnalysisRound` 样板）。`axagent-quant/Cargo.lock` 同批少一行 |

**读数**：Cargo 构建边 **8 → 7**（`AUDIT-l1-blockers-2026-11-05-1131.md` → `…-1202.md` 两份快照可对照）。OPC 跨包口**仍 29 / 9 文件**（未动一处，别把构建边 −1 读成跨包口 −1）。
**验证**（agent 实跑，日志 `.tmp/opc-batch1-*.log`）：`cargo check -p axagent-company-runtime` 退 0；`__TAURI_WORKSPACE__=true cargo test -p axagent-company-runtime --lib` 退 0（**23 passed**）；`cargo clippy -p axagent-company-runtime --all-targets -- -D warnings` 退 0。

### 104.2 搬迁批（下一刀）的范围与授权需求

- **搬什么**：`axagent-quant/crates/analysis-engine/src/opc/` 整棵 → `company-runtime/src/opc/`（或直接新建 `plugin-opc` crate，终局口径见 #32）。`company-runtime` 需补 7 个依赖：`anyhow / parking_lot / regex / serde_yaml / uuid / zip / tokio`。
- **外部引用 77 行 / 6 文件**（搬迁时逐条改指）：`axagent-runtime/crates/tools/src/tools/opc.rs` 41、`src-tauri/src/commands/opc_*` 29、`src-tauri/src/commands/fleet/mod.rs` 4、`src-tauri/src/init/state.rs` 3。
- ⚠ 后两者不在本批 agent 的授权面内，且 `tools/opc.rs` 是股票迁移的高频修改点 ⇒ **搬迁必须由 leader 扩权后单独立项**，不要拆给多个 agent 并发改同一批文件。
- **预期读数**：跨包口 29 → 0；同时 `tools/opc.rs`(41) + `fleet`(4) + `state.rs`(3) = 48 处会从「股票阻塞」正确归到 OPC 面（总数下降 29、归属转移 48，两者不是同一件事）。

### 104.3 本批未做

1. **一处没搬**（改动只有那条无用依赖边）。
2. 未在 harness 定任何接缝（原路线作废，改搬迁；避免先建再拆）。
3. 未跑 `--workspace` 全量（并行批仍在跑）。

## §103 B8-3 首刀：schema-gen 扩面出「前端消费型 DTO」（2026-11-05）

任务 #152。**先如实说结论：本批把生成器打通了，但没兑现 §91.3 给 B8-3 的「−4~−6」** —— 前端反向阻塞仍是 **51 处 / 37 文件**（`audit-l1-blockers` 复测）。原因是首刀挑的 6 个类型属于 **Provider / Workflow 域**（`ProviderType` / `ModelType` / `ModelCapability` / `ProviderKey` / `ProviderProxyConfig` / `SubWorkflowOrigin`），不属于股票视图。选它们的理由是「唯一对应 + 已 camelCase + 消费面明确 + 改动可逆」，但代价是**对目标读数零贡献**。

### 103.1 形态

| 环 | 做法 |
|---|---|
| 生成模式 | `schema-gen` 新增 `frontend-dto` 子命令（`cargo run -p schema-gen -- frontend-dto`），与既有 `ipc`（只做一致性校验、无人消费）分工：本模式产**前端真正 import 的类型** |
| 推导 | 用 `ts-rs` 按 serde 属性推导：`rename_all = "camelCase"` ⇒ camelCase 字段；`Option<T>` ⇒ `T \| null`；带 `skip_serializing_if` ⇒ 可选字段 `?:`；`i64/u64` ⇒ `number`（ts-rs 默认 `bigint`，与 JSON 线上不符，生成器显式改掉） |
| 落点 | `src/types/generated/dto.ts`（生成物，头注释写明勿手改）；`src/types/index.ts` 只 re-export，**删除 6 处手写第二份定义** |
| 守卫 | `foreign_type_refs` 扫描生成体里的外部类型引用，要求「注册闭包自包含」——漏登记一个类型就报错，而不是留一个悬空引用 |

### 103.2 验证（本机实跑）

| # | 命令 | 退码 | 读数 |
|---|---|---|---|
| Y1 | `cargo run -p schema-gen -- frontend-dto` | 0 | 生成 42 行 `dto.ts`；**连跑两次 `cmp` 一致** ⇒ 生成确定性 |
| Y2 | `__TAURI_WORKSPACE__=true cargo test -p schema-gen` | 0 | **7 passed**：`generated_text_matches_golden_snapshot` / `camel_case_field_names_come_from_serde` / `option_maps_to_nullable_number` / `option_with_skip_serializing_if_maps_to_optional_field` / `enum_variants_follow_serde_spelling` / `foreign_ref_guard_has_teeth_and_no_false_positives` / `registry_closure_is_self_contained` |
| Y3 | `cargo clippy -p schema-gen --all-targets -- -D warnings` | 0 | 定点 lint（含测试目标） |
| Y4 | `rustfmt --edition 2024 --check crates/schema-gen/src/main.rs` | 0 | 首轮 4 处 diff 由 leader 用**写模式**修（该 crate 是单文件 bin，无 mod 树，不会误伤） |
| Y5 | `npm run typecheck` | 0 | **1476 个源文件**（含并行批新增），非空跑 |
| Y6 | 语义等价复核 | —— | 逐字比对 HEAD 的手写定义：`ProviderType` 9 个变体、`ModelType` / `ModelCapability` 的 PascalCase 拼写、`ProviderProxyConfig` 三字段形状**全部一致**；`interface` → `type` 属结构等价 |

### 103.3 过程事故（如实记）

- 执行 agent 跑了 **39 分钟 / 104 步后异常退出**，留下：生成器与 7 条测试**已写好**、`tests/golden/` **空目录**（`include_str!` 会让测试目标编不过）、`main.rs` 未过 rustfmt。leader 补了 golden（从生成物复制）+ 跑了 fmt 写模式，其余原样保留。
- 教训：派单里应要求「每完成一个可编译子步就落一次验证」，否则 agent 崩了无法判断完成度；这次靠 `include_str!` 的编译失败点反推出缺口。

### 103.4 未做

1. **股票视图类型一个没生成**（`AnalysisSummary` / `PipelineRun` / `ScheduledTask` 等仍手写）⇒ B8-3 的 −4~−6 完全未兑现，下一刀必须直接对着 `audit-l1-blockers` 的股票清单挑类型。
2. `frontend-dto --check` 未接 CI：现在只在本地手动跑，漂移靠单测 golden 兜（若 Rust 改了而没人重生成，golden 与生成物会一起过期——**这是本批留下的真缺口**）。
3. 未跑 `--workspace` 全量 clippy/test（并行批）。

## §105 第六道门：文档 `文件:行号` 引用的段对齐巡检（#153，2026-11-05）

`AGENTS.md` 立了「引用必须段对齐、禁止文件名中段截断」的规矩，但**此前没有任何自动检查**；源码可编译、门禁可全绿，而文档里的行号会持续腐烂。本批把它变成第六道静态门禁 `scripts/check-doc-refs.mjs`（抽取/定位原语与 `check-single-source-facts.mjs` 共用 `scripts/lib/doc-refs.mjs`，判据不分叉）。

### 105.1 分桶与判定

| 桶 | 含义 | 是否判红 |
|---|---|---|
| OK | 完整路径 / 唯一后缀 / 唯一 basename 可定位，且行号在文件行数内 | 否 |
| NONLOC | 同名多份且路径后缀不能消歧（如裸 `mod.rs`） | 否 —— 但这是「**未检查**」，不是通过 |
| STALE | 文件存在但行号越界（含区间终点） | **红** |
| BROKEN | 仓内不存在该文件，且找不到截断/换词候选 | **红** |
| ABBREV | 不存在，但自证到它缩写的真实文件（段内截断 / 同义词） | 只报告（首轮未升红） |
| EXTERNAL / EPHEMERAL | 跨仓 / 本机绝对路径；`.workbuddy/`、`output/` 等 CI 检出里不存在的目录 | 只报告（环境差异不是文档缺陷） |
| MALFORMED | 抽出来根本不是路径（表格残片） | 丢弃、只计数（灌进 BROKEN 会让该桶指不回真实风险） |

棘轮基线 `scripts/doc-refs-baseline.json`（342 条，**只减不增**）：键 = `<文档>::<引用串>:<起始行>[-<结束行>]`，**不含文档行号**（文档增删会让行号整体漂移，拿它当键会自己先烂）。`--init-baseline` 仅在文件不存在时可用；`--update-baseline` 先要求本轮无新增。

### 105.2 验证（全部实跑）

| # | 命令 | 读数 |
|---|---|---|
| D1 | `node scripts/check-doc-refs.mjs --selftest` | `SELFTEST OK`：三档合法、截断/同义词自证候选、单行与区间终点越界、同名不可消歧、跨仓/环境免检、棘轮正负控 |
| D2 | `node scripts/check-doc-refs.mjs` | **279 篇 / 7920 条**：OK 6348 ｜ NONLOC 1145 ｜ BROKEN 271 ｜ ABBREV 23 ｜ STALE 71 ｜ EXTERNAL 50 ｜ EPHEMERAL 12 ｜ MALFORMED 0；**基线外新增 0**，退 0 |
| D3 | CI 接线 | `scripts/ci-check.mjs:543-544`（自检 + 判定两步）与 `.github/workflows/ci.yml:573/576` 逐字同源；由 `src/__tests__/ci.workflow.test.ts` 的对称差集护栏校验 |

### 105.3 上线即抓到的两件事（如实记）

1. **门禁自身会被「已删除未提交」的文件打崩**：文件索引来自 `git ls-files --cached --others`，OPC 子树搬迁期间文档仍指向旧路径 ⇒ `locate()` 返回一个盘上不存在的绝对路径 ⇒ `readFileSync` 抛 `ENOENT`，整道门退非 0 但**不是判红，是崩溃**。已修为「盘上不存在 ⇒ BROKEN」（`check-doc-refs.mjs` 的 `lineCount` 返回 `null`），并加了一条回归钉（`classifyRef` 对索引内有、盘上已删的引用必须判 BROKEN）。
2. **OPC 搬迁带来 47 条基线外新增**（全部指向 `analysis-engine/src/opc/*`）。处置：**只改目标确实存在的新路径**（`analysis-engine/src/opc/` → `company-runtime/src/opc/`，40 + 8 处，含 4 篇审计快照与 4 篇 PLAN），改完 BROKEN 318 → 271、基线外新增 0。判据写在一次性脚本里：新路径不存在则不替换 —— 否则会把基线内的存量 BROKEN 改成一个**新的引用串**（基线键含引用串），凭空制造新红。**纯移动不改行号**，故 4 篇审计快照的 `:行号` 全部仍有效。

### 105.4 未做（如实列）

1. **ABBREV 23 条未升红**：首轮只报告（存量未知量级），下一轮应先修文档或登记，再把它并入红面。
2. **NONLOC 1145 条是「未检查」**：同名多份（`mod.rs` / `index.ts`）靠路径后缀消歧不了，本门对它们**没有判断力**，别把「不是红」读成「引用正确」。
3. 基线 342 条只冻结未修（本轮 `已消失 0`），`--update-baseline` 未跑（并行批期间禁止）。
4. 未扫 `docs/` 以外的 `.md`（根目录白名单文档不在本门覆盖面内）。

## §106 B8-4 前置：所谓「20 处共享基元」实测只有 **2 处**（#154，2026-11-05）

逐符号判定全文在 `PLAN-shared-primitives-neutrality.md`（51 行判定表，行号可复算）。结论**推翻了 §91.4-2 的前提**：

| 类 | 处 / 文件 | 处置 |
|---|---|---|
| ① `feRe` 判据假阳性（quote=引用回复、market=插件/人才市场） | 7 / 6 | 不改代码，登记审计判读口径 |
| ② **中性可下沉** | **2 / 2**（`PageHeader`、`clearVendorCheckCache`） | B8-4a |
| ③ 域特定但导入方也是域自有（含 2 处死代码：`office/panels/{TrajectoryPanel,StrategyRoomMiniBar}`） | 20 / 17 | 随股票前端打包迁出，**换目录是洗绿** |
| ④ 域特定且宿主文件内联域逻辑 | 22 / 12 | 走插件贡献面 / barrel 拆除（B8-1/B8-2/B8-5） |

**对 B8-4 的影响**：§91.3 给 B8-4 的预期读数是 −8～−12，**不成立**。真实可下沉面只有 2 处（预期 51 → 49），其余 42 处属「域随插件走」或「判据误报」，不是共享基元问题。**不得**为了凑读数把 ③ 类符号搬进中性目录。§4.3 已明确列出「不要下沉的东西」，防后人重做一遍。

**未做**：两处下沉本身尚未实现（B8-4a 仍待做）；`feRe` 的盲区（只按**路径名**判域归属，域逻辑写在非域文件里时看不见）只登记未改判据 —— 改判据会让既有审计读数整体漂移，需单独立项。

## §107 插件 schema 的动作权限闸 + 消息侧卡片插槽（#155/#156/#157，2026-11-05）

### 107.1 安全闸（#155 的实体）

缺陷链路（§102.4-1 已实测）：`dataSource` 的 `config.endpoint` 有 `ALLOWED_INVOKE_PREFIXES=["dynamic_ui_"]`，但**声明式动作**走另一条路 —— `EventHandlerEngine.executeActions` 构造 `ActionContext` 时不传 `permissions`，`ActionRouter` 的判据是「有 `permissions` 才校验」，于是 `actions[].type === "invoke"` 直接 `invoke()`。插件 `schema.json` 收敛出的记录（`origin = "plugin"`）正是经这条链执行 onMount/onClick ⇒ **第三方 schema 可调任意已注册 Tauri 命令**。

| 环 | 做法 |
|---|---|
| 判据 | 只取 `DynamicUISchemaRecord.origin`：`builtin/user/ai` 可信（不注入，行为与改动前逐字一致）；`plugin` / 缺失 / 未知字面量一律**受限**（fail-closed，新增 origin 忘了登记得到的是「被收紧」） |
| 受限面 | `commands: ["dynamic_ui_*"]`；`navigate/events/storeRead/storeWrite` **显式空数组**（这些 executor 的判据也是「有 permissions 就走白名单」，省略等于放行） |
| 注入点 | `DynamicUIRenderer` 树根注入上下文；`SchemaNodeRenderer` 的 onMount/onUnmount/events、`FormRenderer` 的 onSubmit 全部透传 —— **任一漏传 = 漏一条 IPC** |
| 连带收口 | `handler` 执行器加 `sourceRestricted`，禁止借目标 skill manifest 放宽；受限来源的 `update-schema` 不得跨 renderer 广播（`__global__` 降级为当前 scope，无 scope 直接丢弃） |
| 非持久化 schema | 无记录的（Agent 实时生成 / 编辑器 / 预览）维持现状 —— 它们不是插件产物 |

**验证**：`npm run typecheck` 退 0（**1477** 文件）；`cargo check --lib` 退 0（18m01s，零 warning）；`cargo test --lib commands::dynamic_ui` **2 passed**（表驱动 7 拒 3 放行 + 副作用用例：`origin="plugin"` 返回 Err 且串内无 `LEAK_SECRET`，同数据 `origin="user"` 正控）。前端定点 vitest（`--pool=forks`）**28 passed**：`schemaActionPermissions` 10、`pluginSchemaActionPermissions` 10、`PluginSchemaCard` 5、`cacheInvalidation` 3。

**本机 vitest 的可用姿势（重要）**：threads 池的 worker 启动超时硬编码 60 s，而本机 jsdom 启动已达 **46 s**（`cacheInvalidation` 单文件实测 `environment 46.28s`），叠加负载后必超时；换 `--pool=forks` 可跑（同一文件 Duration 62.6 s）。**这不是代码问题**，用无关文件复现即可判环境。

**过程事故（如实记）**：这条 10 例文件先后暴露两个**用例自身**的错误，都不是被测逻辑的问题——
1. 首轮 4 失败：测试没包 `MemoryRouter`，`useNavigate()` 在渲染期抛错，整棵树没起来 ⇒ **「被拒」的三条是假绿**（树根本没渲染，自然没有 IPC）。
2. 第二轮 2 失败：`update-schema` 在 `EventHandlerEngine` 顶层读的是 `action.config`（内层 chain 才读顶层字段），我却写在顶层 ⇒ 两条静默不派发，另一条因抛错而「假绿」。
两轮都靠**同文件的正控用例**（「放行」那条必须真的有 IPC）才暴露出来 —— 判据：**「断言不发生」类用例必须与正控配对**。

### 107.2 消息侧卡片插槽（#157）

- 契约载体是既有的 `Message.meta: Record<string, unknown>`（**不新增 DTO**）：键 `pluginCards`，值 `PluginSchemaCardSpec[]`（`schemaId` + 可选 `dataContext` / `title`）。
- 渲染用 `DynamicUIRenderer`（`DynamicUIStandalone` 内部那棵树），不 import 任何插件产物；取不到 schema 落 `dynamicUI.schemaNotFound` 的 11 语言既有译文，不抛到消息渲染树。
- 解析对每条独立校验：坏条目只丢自己，不炸整条消息。`PluginSchemaCardSlot` 在无卡片时返回 `null`（不占位）。
- **生产者侧今天还没人写 `meta.pluginCards`**（插件产出入消息 meta 的通道不在本批）—— 插槽现网渲染为空，属「通道已通、生产未接」。

### 107.3 独立审查：闸门外仍有绕过路径（已实测）

只读审查（另一路）复核后列出 7 条，其中 **4 条可真利用**：

| # | 绕过 | 现状 | 处置 |
|---|---|---|---|
| A | **信任锚可伪造**：`create_dynamic_ui_schema` 直接采信请求里的 `origin` / `owner_id`（`src-tauri/src/commands/dynamic_ui.rs:337`），插件可造一条 `origin:"user"` 的记录把 `invoke` 写进 schemaJson ⇒ 前端按来源判定即放行 | 后端 | **已修**：该命令一律写 `origin="user"` / `owner_id=""`，请求字段标注废弃；插件来源只能由安装通道直接落库 |
| B | **写命令无归属校验**：update / delete / restore / save-get-delete-form-data / pin / unpin 只拦 `is_builtin`；`get_dynamic_ui_form_data` 连 schema 是否存在都不查（最大泄露面：用户填写值） | 后端 | **已修**：`plugin_owned_denied(origin)`（白名单 `builtin/user/ai`，`plugin`/空/未知一律拒）+ 统一闸门 `load_schema_for_command` 接在 8 个入口 |
| C | **`chain` 内嵌 `update-schema` 绕过 scope 收敛**：顶层被拦，内层经 `ActionRouter` 派发的 detail **无 scope** ⇒ 接收端当广播处理，可把恶意节点替换进**可信**渲染树并以不受限权限执行 | 前端 | **本批已修**：`ActionContext.scope` + 受限来源强制带 scope、无 scope 直接拒发/拒收（3 条回归用例） |
| D | **回查全局单槽 `currentSchema` 的时序洞**：同屏两张插件卡片时后加载者覆盖前者 ⇒ 先渲染的回查落空 ⇒ `undefined` ⇒ 不受限 | 前端 | **本批已修**：`PluginSchemaCard` / `DynamicPageViewer` 直接用自己那条记录算权限面 |
| E | `dataSource` 的 store 读取不受 `storeRead` 约束；`api` 分支允许任意 http(s) 出站 | 前端 | 未修（要改 `subscribeDataSource` 签名与数据源分派，单独立项） |
| F | `function` 执行器无权限判定 | 前端 | 当前 `registerCustomHandler` **零调用点**、不可达；规则缺位，待接线时补 |
| G | 受限 `update-schema` 仍采信 `config.scope` | 前端 | **本批已修**：受限来源忽略 `config.scope`，只认当前渲染树 scope |

**另一个事实（推翻 §102.1 的前提）**：插件的 `schema.json` **注册通道尚未实现** —— 全仓只有撤销面 `revoke_schemas_owned_by`，没有任何代码写入 `origin="plugin"`。§102.1 写的「安装/启用时收敛进库」目前不成立，§102 的详情槽因此还不能接线。

### 107.4 未做

1. 前端 E（dataSource 读面权限）未修：`subscribeDataSource` 没有 permissions 入参，属独立改造。
2. 其余未声明 `actionPermissions` 的渲染入口（`AgentUIRenderer` / `DynamicUIPreview` / `VisualEditor` / `SkillPageRenderer` 等 8 处）目前靠 `schemaId` 回查兜底；它们无记录时按「不受限」处理，是否收紧取决于那些入口的来源模型，未在本批裁。
3. 插件产出入消息 `meta` 的通道未建。
4. 未跑全量 vitest（定点 + typecheck）。

## §108 #36 搬迁落地：OPC 子树整棵迁入 company-runtime（2026-11-05）

§104 裁定的路线（不用接缝，整棵搬）已执行完毕。

### 108.1 改动

| 环 | 内容 |
|---|---|
| 搬迁 | `axagent-quant/crates/analysis-engine/src/opc/`（25 模块 + `opc_adapter/` 3 文件）→ `axagent-quant/crates/company-runtime/src/opc/`；`analysis-engine/src/lib.rs` 删 `pub mod opc;` 并留一行说明 |
| 依赖 | `company-runtime` 补 `parking_lot` / `regex` / `serde_yaml` / `uuid` / `zip`（各带用途注释）；**不加回** `axagent-analysis-engine` 边（该子树对股票模块零引用） |
| 宿主引用 | `src-tauri/src/commands/opc*.rs` / `fleet/mod.rs` / `init/state.rs` / `demand_discovery.rs` / `conversations_search.rs` / `tools/opc.rs` 共 48 处改指 `axagent_company_runtime::opc::*`；`src-tauri/Cargo.toml` 与 `tools/Cargo.toml` 加依赖边 |
| 文档 | 指向旧路径的 48 处引用同批改指（**纯移动、行号不变**，4 篇审计快照也逐条核对未越界） |

### 108.2 读数（`audit-l1-blockers` 复算，快照 `AUDIT-l1-blockers-2026-11-05-1356.md`）

| 度量 | 改前 | 改后 |
|---|---|---|
| Rust 跨包口（OPC） | 29 处 / 9 文件 | **0 处 / 0 文件** |
| Cargo 构建边（OPC） | 7 | **2** |
| Rust 待改写 · 股票桶 | 99 处 / 16 文件 | **51 处 / 13 文件** |
| Rust 待改写 · OPC 桶 | 0（全记在股票桶） | **48 处 / 3 文件**（归属转移，不是新增） |
| 前端反向阻塞 | 51 / 37（股票）、28 / 11（OPC） | **不变** |

> ⚠ 别看错：「待改写 99 → 51」不是修好了 48 处，而是**归属从股票桶转到 OPC 桶**（`tools/opc.rs` 41 + fleet 4 + state.rs 3）。总数只降了「跨包口」那 29 处。

### 108.3 验证（实跑）

| # | 命令 | 结果 |
|---|---|---|
| Z1 | `cargo check -p axagent-company-runtime` | 首轮 **E0433**（1 error，缺依赖）→ 补后 `Finished in 30.01s`，EXIT=0 |
| Z2 | `__TAURI_WORKSPACE__=true cargo test -p axagent-company-runtime --lib` | **104 passed; 0 failed**（33.08s），EXIT=0 |
| Z3 | `cd src-tauri && cargo check --lib` | **Finished in 15m21s**，EXIT=0，0 error |
| Z4 | `node scripts/check-doc-refs.mjs` | 基线外新增 0（搬迁带来的 47 条已改指；改指脚本的判据是「新路径确实存在才替换」） |

> **过程事故**：执行 agent 的主 crate 检查日志停在 `Checking axagent-runtime` 就再没有输出，且 cargo 进程已不在 —— 该轮**没有完成**。Z3 是 leader 重跑的，不要把它读成 agent 自报。

### 108.4 未做

1. **`company-runtime` 搬迁后未跑 clippy**（本批只跑 check + test；上一批的 clippy 是针对「删依赖边」那次，不能替这一批作证）。
2. 未跑 `--workspace` 全量 clippy / test。
3. `company-runtime` 现为 **implementor 依赖 consumer 面**的检查点：它被 `tools`（hybrid）与 `src-tauri`（wiring）依赖，方向合法；但 `company-runtime → entities` 的传递依赖是否符合 hybrid 的「禁止 entities」判据，未在本批裁（既有先例，未新增边）。
4. 距 #32（独立仓）还差：`plugin-opc` crate 拆分与插件化立项，不在本批。

## §109 B8-4a：两处真中性基元下沉完成（#154 落地，2026-11-05）

### 109.1 改动

| 符号 | 做法 |
|---|---|
| `PageHeader` | 移到 `src/components/common/PageHeader.tsx`（中性目录白名单内）；`showTimeAnchor?: boolean` → 中性的 `anchor?: ReactNode` 插槽，**删掉对 `time-travel/PageTimeAnchor` 的反向 import**（否则中立性门禁当场红）；`.sa-header*` 4 条样式（42 行）从中性组件自带 `pageHeader.css`，修掉「独立路由下头部无样式」的既有隐患（`stock-analysis.css` 留指针注释）；原 `stock-analysis/_shared/PageHeader.tsx` 删除（改后零引用）；6 个调用点改指新路径，`ReplayWorkbench` 改传 `anchor={<PageTimeAnchor />}` |
| `clearVendorCheckCache` | 改为中性失效广播 `src/lib/cacheInvalidation.ts`（`onInvalidate` / `invalidate(topic)`）：缓存持有方（股票侧 `vendorCheck.ts`）在模块加载时订阅 `vendor-config`，宿主 `DataVendorsTab` 只广播 —— **宿主不再 import 股票域文件**。懒加载时序安全：域模块未加载时本就没有缓存可失效 |

### 109.2 读数

| 度量 | 改前 | 改后 |
|---|---|---|
| 前端股票反向阻塞 | 51 处 / 37 文件 | **49 处 / 36 文件** |
| 组件中立性（原始 / 基线） | 原始 225（白名单后） | 原始 **222**，PASS；已修复 4 条 |

> 首轮曾读到 50：`cacheInvalidation.ts` 的注释里写了字面 `import("@/components/stock-analysis/vendorCheck")`，被 `feRe` 按**文本**误命中 —— 改措辞后归 49。这正是 §106 记的「`feRe` 按名字判域」盲区的又一次实证。

### 109.3 验证

| # | 命令 | 结果 |
|---|---|---|
| W1 | `npm run typecheck` | EXIT=0，**1477** 文件 |
| W2 | `node scripts/audit-l1-blockers.mjs` | EXIT=0，51/37 → **49/36**（快照 `AUDIT-l1-blockers-2026-11-05-1413.md` / `-1418.md`，未提交） |
| W3 | `node scripts/check-component-neutrality.mjs` | **PASS**（原始 225 → 222） |
| W4 | `node scripts/check-jsx-text-i18n.mjs` | **PASS**（基线外 0） |
| W5 | 定点 vitest | **未取得读数**：14:04 那轮 2 文件 / 16 tests 通过、其余 5 文件 worker 启动超时；此后 7 次单文件重试（threads / forks 两池）全部卡在 vitest 硬编码 60 s；用**无关文件** `browserEvents.test.ts` 复现同样失败 ⇒ 环境问题（SMB + 并发 cargo 编译），不是本批改动 |

### 109.4 未做

1. **`--update-baseline` 未跑**：已修复的 4 条中立性引用里有 1 条属 chat 批次在途改动，收紧会把别人的中间态烘进棘轮基线。
2. 剩余 49 处一律未动：③ 类（`horizonLabelKey` / `investPath` / `agentOutput` 整文件域）随股票前端迁出；④ 类（TasksCenterPage / barrel / `useRightPanel`）走贡献面或 B8-5。`SectionHeading` / `investRoutes` / `dualView` 按 §4.3 明确不下沉。
3. 本批的 5 条前端测试读数缺失（见 W5），需在编译窗口空闲后补跑。

## §110 B8-3 第二刀：股票 DTO 生成 + 前端手写类型收口（#160，2026-11-05）

**立项口径：这是「去重」（禁区 12），不是「降读数」。** `audit-l1-blockers` 的前端计数按 import 串判域，收口手写类型不会让它下降（实测确认 49/36 不变）。

### 110.1 Rust / 生成器

| 改动 | 内容 |
|---|---|
| `harness::market_data` | `StockQuote` / `KLine` / `StockSearchResult` 加 `ts_rs::TS` 派生 |
| `harness::stock_market_types` | `ConsensusEPS` / `IndexQuote` / `EarningsEvent` 加 `TS` |
| `harness::dashboard_report` | `IndexQuote` → **`DashboardIndexQuote`**（3 字段复盘行与 7 字段行情行撞名，生成名单只能留一个）；`lib.rs` 根导出同步 |
| `harness::market_data::StockQuote` | `limit_up` / `limit_down` 补 `#[serde(default)]` —— **实测 ts-rs 12 只在 `default` 存在时判可选**，单有 `skip_serializing_if` 仍生成必填，会让类型声称「键必在」而线上载荷缺键 |
| `schema-gen` | `frontend_dto_decls` 新增 `STOCK` 分节 7 条（6 股票 + `DashboardIndexQuote`），生成物 6 → **13** 个类型 |

**纠正一条既存假判据**：`schema-gen` 原有单测 `option_with_skip_serializing_if_maps_to_optional_field` 的判据模型 `Model.input_price_per_mtok` 实际带 `default`，测的并不是 `skip_serializing_if`。已改名为 `option_with_default_maps_to_optional_field`，并补反手抄 `AgentResult.session_id`（只有 `skip_serializing_if`）钉住「仍必填」。同时修正 `dto.ts` 头注释同一处错误表述（生成物注释必须与生成器行为一致）。

### 110.2 前端收口

| 改动 | 内容 |
|---|---|
| `src/types/stock-analysis.ts` | 删 6 个手写接口（`ConsensusEPS` / `StockQuote` / `KLine` / `EarningsEvent` / `StockSearchResult` / `IndexQuote`），保留纯前端的 `EarningsEventType` 联合字面量 |
| `src/types/index.ts` | 生成物 re-export 名单 6 → 13 |
| 6 处 import | `stockAnalysisStore.ts` / `CompareView.tsx` / `stock-quote-schema.ts` / `KLineChart.tsx` / `WfDesTab.tsx` 改走 `@/types` barrel（删手写版后深路径会 TS2305） |
| `MarketReviewReport.indices` | `IndexQuote[]` → `DashboardIndexQuote[]` —— 修掉一处**类型谎言**：后端灌的是 3 字段复盘行，前端却声称 7 字段行情行 |

### 110.3 验证

| # | 命令 | 结果 |
|---|---|---|
| V1 | `cd axagent-plugins && cargo run -p schema-gen -- frontend-dto` | EXIT=0，2m35s，生成 13 类型 |
| V2 | `cd axagent-plugins && cargo test -p schema-gen` | **7 passed**，含 golden 逐字比对 |
| V3 | `cd axagent-core && cargo check -p axagent-harness` | EXIT=0（agent 实测，2m34s） |
| V4 | `npm run typecheck` | EXIT=0，**1478** 文件 |
| V5 | `node scripts/audit-l1-blockers.mjs` | EXIT=0，前端 **49/36 不变**（快照 `AUDIT-l1-blockers-2026-11-05-1749.md`，未提交） |
| V6 | `dprint check`（7 个前端文件） | 全绿（`WfDesTab.tsx` import 排序修正 1 处） |
| V7 | `cargo fmt -p <crate> -- --check`（harness / schema-gen / analysis-engine） | 三根全绿 |

### 110.4 未做

1. `MarketReviewReport` 本身仍是前端手写类型（`generatedAt: string` vs 后端 `DateTime<Utc>` 等），本刀只修了 `indices` 一处的字段类型，整体收口留给后续。
2. 前端 49 处反向阻塞未动（与本次无关；它们多是 store/组件 import，不是类型深导入）。
3. `frontend-dto --check` 仍未接 CI（B8-3 首刀遗留缺口）。
4. `EarningsEventType` 目前零消费方（`EarningsEvent.eventType` 已是 `string`），保留作 UI 约束，未删。
5. 前端单元测试未跑：本机 vitest 在 SMB 上 60 s worker 启动超时（#141），本批改动是纯类型收口，typecheck 已覆盖。

## §111 analysis.record：单股分析记录只读接缝（#149 第 1 步，2026-11-05）

**为什么新建**：`workflow.run` 的 `list_recent` 按 owner 隔离，只看插件自己发起的**执行**；任务中心 `stock-analysis` 源读的是**记录表** `stock_analyses` —— 宿主工作台 / 定时任务发起的记录，插件一条都看不见。两者不同表、不同语义（「跑过什么」vs「得出过什么结论」），故不共用。

### 111.1 契约（`axagent-core/crates/harness/src/analysis_record.rs`）

| 项 | 值 |
|---|---|
| 键名 / op | `analysis.record` / `list`（唯一，只读；无写 op ⇒ 不设权限闸） |
| 入参 | `{ limit?: usize, offset?: usize }` |
| 出参 | `{ records: AnalysisRecordView[], total: u64 }`（具名对象，不返裸数组） |
| `limit` 闸 | 契约区间 `1..=200`，缺省 50；越界 `Validation` **拒收不夹取**（夹取会让插件分页 offset 算错） |
| `offset` 闸 | 缺省 0；> `u32::MAX` 拒收（`as u32` 会静默截断到另一页） |
| 行视图 | 19 字段全 camelCase（测试逐字钉住键集合），时间只有 epoch 毫秒 |
| 不过帧 | `decisionJson` / `blackboardSnapshot` / `decisionReasoning` / `llmDecisionJson` / `configId` / `source` / `synthetic` |
| `total` 口径 | `limit` 之前的整表行数（无过滤）；根记录收窄归消费方（replay 子行仍计入） |

### 111.2 四段接线

| 段 | 文件 | 要点 |
|---|---|---|
| 契约 + 注册表 | `harness/src/analysis_record.rs`、`capability_registry.rs`、`lib.rs` | `ServiceDefinition::analysis_record()` + `register_analysis_record` / `get_analysis_record` |
| 宿主 provider | `src-tauri/src/init/analysis_record_service.rs` | **复用** `query_stock_analysis_items`（投影唯一，禁区 12）+ 一次 `COUNT(*)`；落 `init/` 以避开 `commands-no-direct-db` |
| worker 分派 | `plugins/src/worker.rs` | `"analysis.record" if op == "list"`；两个可选标量走 `deserialize_optional_arg`；`required_permission_for` **不加条目** |
| 插件命令 | `plugin-stock/src/main.rs` | `list_stock_analysis_records`：状态词表 `mapAnalysisState` 迁入插件、根记录收窄、行由 `PluginTaskRunRow` 序列化；不交 `deepLink`（宿主路由契约） |

**如实口径**：复用的宿主查询当前仍 SELECT `decision_json`（`StockAnalysisListItem` 既有形状），本接缝在映射时丢弃 ⇒ 不过帧但会进宿主内存；要连读都省需改既有 IPC 查询，不在本批改动面。

### 111.3 读数

| 度量 | 改前 | 改后 |
|---|---|---|
| K1 `.rs` 文件数 | 2037 | **2039**（契约 + 宿主 provider 两个新文件） |
| K1 `pub trait` 唯一名 | 325 | **326**（`AnalysisRecordProvider`） |
| K1 外部实现者 | 241 | **242** |

### 111.4 验证

| # | 命令 | 结果 |
|---|---|---|
| V1 | `cargo test -p axagent-harness --lib analysis_record` | **9 passed** |
| V2 | `cargo check -p axagent-plugins -p axagent-plugin-stock` | EXIT=0，3m52s |
| V3 | `cargo test -p axagent-plugin-stock --test protocol` | **12 passed**（含 `command_frame_lists_stock_analysis_records_via_host_seam`） |
| V4 | `cargo test -p axagent-plugins --lib analysis_record` | **2 passed**（含「只读 op 无权限闸」的取证） |
| V5 | `cd src-tauri && cargo check --lib` | EXIT=0，**19m43s**，零 error |
| V6 | `node scripts/check-capability-register.mjs` | 29 → **10 条**：本批 `k1RsFileCount` 已修绿（`planPinned.line` 247→250 是既存行号漂移，顺手同批修）；余 10 条为**既存红**（`l1ForeignStock` 的 planOld 历史账 + `capacityRegister` 的 23 钉句），非本批文件面，未擅自重锚 |

### 111.5 未做

1. **#149 第 2/3 步**：插件 schema 注册通道（`origin="plugin"` 写入 + `detail_schema` 归属校验）与详情槽；源 id 切换 + 删宿主 `stockAnalysis.adapter.ts`。设计结论：写入用安装/启用期的宿主 reconcile（插件声明 `ui.json`，origin/owner 由宿主戳，插件不自报），详情槽只在 `detail_schema` 存在时挂 `PluginSchemaCard`。
2. 插件清单 `taskSources[]` 未声明 `list_stock_analysis_records`（与第 2 步同批做）。
3. `deepLink` 未交（宿主路由契约）；插件命令未透出 `total`（分页仍由宿主泛型 adapter 按数组长度处理）。
4. 真库端到端未测（需预置 `stock_analyses` 行）；前端零改动。
5. 第五道门 10 条既存红未修（属他人 PLAN 历史账，重锚会把中间态烘进基线）。

## §112 插件 schema 注册通道 + `detailSchema` 归属校验（#149 第 2 步 Rust 侧，2026-11-05）

### 112.1 结论

插件不再只能「渲染宿主给的 schema」，而是可以**自带** schema：安装根下一份 `ui.json`，
宿主在安装 / 启用 / 更新 / 启动四个点把它 reconcile 进 `dynamic_ui_schemas`，
落库时**由宿主盖章** `origin = "plugin"` + `owner_id = pluginId`。同时把
`taskSources[].detailSchema` 与 `pages[]`（`loading = "schema"`）的 `entry` 都收成
「**本插件 `ui.json` 的 key**」，安装期 membership 校验 + 运行期按库复核。

### 112.2 `ui.json` 形状

```json
{
  "version": 1,
  "schemas": [
    {
      "key": "order-form",
      "title": "Order Form",
      "category": "custom",
      "tags": ["order"],
      "version": "1.0.0",
      "schema": { "type": "object", "fields": [] }
    }
  ]
}
```

| 字段 | 必填 | 校验 |
|---|---|---|
| `version` | 是 | 只能是 **1**（不支持即整份拒绝，不按缺省字段猜） |
| `schemas[].key` | 是 | 形状 = `is_declared_key_shape`（小写字母/数字/`-`，首字符字母或数字，≤ 64）；文件内唯一；**不是行 id** |
| `schemas[].title` | 是 | 非空 |
| `schemas[].category` | 否 | 缺省/空白 ⇒ `custom`（与列默认值同口径） |
| `schemas[].tags` | 否 | 逐项 trim、空串丢弃 |
| `schemas[].version` | 是 | `major.minor.patch` 形态的数字版本（点数 1–3 段）—— 非法值会让宿主 `semver_gt` 退化成 0.0.0，「内容变了但版本没升」的判据静默失效 |
| `schemas[].schema` | 是 | 必须是 JSON **object**（序列化进 `schema_json` 列；键序稳定 ⇒ 内容比较可逐字做） |

行 id 恒为 `plugin.{pluginId}.{key}`（`ui_schema::row_id`），前缀 `plugin.{pluginId}.`
（`owner_prefix`）用于剪枝差集。`ui.json` 里**没有任何字段**能声明 origin / owner / 行 id。

### 112.3 幂等 / 版本 / 剪枝口径

| 情形 | 处置 |
|---|---|
| 同 owner + 同 id，内容与 version 都没变 | **no-op**（安装/启用/启动反复跑零写） |
| 内容变、`version` 升高 | 先写旧版本快照（复用 `create_version_snapshot`）再更新 |
| 内容变、`version` 未升 | **安装失败**（对齐 `update_dynamic_ui_schema` 的「版本号冲突」口径） |
| id 已存在但 `origin`/`owner_id` 不符 | **安装失败并报出实际归属，绝不覆盖** |
| 同 owner 的其他行（本次未声明）且 id 在前缀内 | 剪枝：级联删表数据 + 版本历史 + 本体 |
| `ui.json` 缺失 / 读失败 / 解析失败 | 本轮跳过，**禁止剪枝**（防瞬时 IO 错误清库） |
| 插件禁用 | schema **保留**（归属是「安装」不是「启用」）；卸载仍走 `revoke_schemas_owned_by` |
| 并发 reconcile 撞 PK | 重读该行，按「已存在」分支处置（幂等 / 版本冲突 / 拒绝覆盖），不当作写失败 |

### 112.4 四段接线

| 段 | 文件 | 要点 |
|---|---|---|
| 纯解析 + key 校验 | `axagent-plugins/crates/plugins/src/ui_schema.rs`（新） | 零 I/O、零 DB；`parse` / `is_declared_key_shape` / `row_id` / `owner_prefix` |
| 清单 membership | `plugins/src/manager.rs` | `load_declared_ui_keys` 读 `<root>/ui.json`；`PageSchemaKeyUnknown` / `TaskSourceSchemaKeyUnknown` / `InvalidUiSchemaDeclaration` 三个变体；`pages[]` 与 `taskSources[]` 共用同一 key 集合 |
| 宿主 reconcile | `src-tauri/src/init/plugin_ui_schema.rs`（新） | `reconcile_ui_schemas` / `retain_owned_detail_schemas`；签名**不收** origin/owner；pluginId 取自 `plugin_registry_report()` |
| 挂载 + 运行期复核 | `commands/plugin.rs`、`init/services.rs` | install / enable / update / 启动四处；`plugin_list_task_sources` 用 db 复核 `detailSchema`，查无此行即置 `None` |

`detail_schema` 只能是 key（不是行 id）：拼全 id 的活儿在宿主侧 ⇒ **跨插件引用在声明面上写不出来**；
`pages[].entry`（`loading = "schema"`）同理，顺手补上了这条路径此前**零归属校验**的缺口
（前端曾直接把 `page.entry` 当 schemaId 用）。

### 112.5 读数

| 度量 | 改前 | 改后 |
|---|---|---|
| K1 `.rs` 文件数 | 2039 | **2041**（`plugins/src/ui_schema.rs` + `src-tauri/src/init/plugin_ui_schema.rs`） |
| K1 `pub trait` 唯一名 | 326 | **326**（本批零新增 trait） |
| K1 外部实现者 | 242 | **242** |

### 112.6 验证

| # | 命令 | 结果 |
|---|---|---|
| V1 | `cd axagent-plugins && cargo test -p axagent-plugins` | EXIT=0，**251 passed / 0 failed**（含 `ui.json` 解析 8 条 + membership 4 条新测） |
| V2 | `cd src-tauri && cargo check --lib` | EXIT=0，**17m56s**，零 warning |
| V3 | `cargo fmt -p axagent -- --check` / `cargo fmt -p axagent-plugins -- --check` | 两侧 **EXIT=0** |
| V4 | `node scripts/check-capability-register.mjs` | 见 §112.7-5：K1 三读数 PASS（2041 / 326 / 242）；总数维持 29 条**既存**硬失败（本批先把自己造成的 2 条漂移修绿：`dojo.extension` 证据行 838→875、PLAN:250 的 2041） |

### 112.7 未做

1. **前端已接（同批完成）**：`detailSchema` → `schemaId` 与 `pages[].entry` → 行 id 两处共用 `src/lib/pluginUiSchema.ts` 的 `pluginSchemaRowId`（后端 `ui_schema::row_id` 的同源镜像，不再各拼一串）；`pluginTaskSource.adapter` 仅在声明了 schema 时挂 `renderDetail`，未声明仍走 `RunDetailDrawer` 的空抽屉兜底。前端验证：`npm run typecheck` EXIT=0（**1479** 文件）、`dprint check` 绿；定点 vitest 因 #141（SMB worker 启动 60 s 超时）未取得读数。
2. 插件清单实样未改：`plugin-stock` 的 `taskSources[]` 尚未声明 `detailSchema`（第 3 步随源 id 切换一起做）。
3. `ui.json` 的 schema 体未做结构校验（只验 JSON object）——动态 UI 的字段语法仍归前端加载期判。
4. 并发 reconcile 的「剪枝 vs 插入」不做事务隔离：极端交错下可能删掉另一路刚写的行，下一轮 reconcile 自愈（幂等重建）。
5. 第五道门在本工作树已有 **29 条既存红**（PLAN 行号漂移，非本批文件面；任务书里的 10 条是上一批的读数）。本批只同步 `k1RsFileCount` 与 PLAN:250，未重锚/放宽任何判据。







## §113 插件来源 Schema 的第二条 IPC 出口收口（#165，2026-11-06）

§107 那批把**动作面**（`actions[].type === "invoke"`）按 origin 收成 fail-closed，但留了两处没闭合，本批一起收。

### 113.1 两条绕过面（都是可复现的提权/泄露链，不是假想风险）

1. **动作面给了 `dynamic_ui_*` 通配 ⇒ 通配里含三条写命令**。判据链：
   - `create_dynamic_ui_schema` 硬写 `origin = "user"`、`owner_id` 恒空（`src-tauri/src/commands/dynamic_ui.rs:362` 起，注释在 `:102`）；
   - 而 `"user"` 正是 `TRUSTED_SCHEMA_ORIGINS` 的成员 ⇒ 插件只要声明一个 invoke 该命令的 onMount 动作，就**自造出一行「可信」Schema**，那一行渲染时权限面是 `undefined` = 完全不受限。
   - `update_dynamic_ui_schema` / `delete_dynamic_ui_schema` 只挡 `builtin`，**不按 `owner_id` 限定目标** ⇒ 插件来源的动作能改写／删除用户或别的插件的 Schema；改写等于把恶意节点塞进可信树。
2. **`dataSource` 是第二条 IPC 出口，完全没接权限面**。`SchemaValidator` 与 `DataBindingEngine` 只过 `ALLOWED_INVOKE_PREFIXES = ["dynamic_ui_"]`（按前缀、对所有来源同视），且 `type: "store"` 的数据源直读任意已注册 store —— 含 `providerStore`（存放提供商配置／代理）。
   - 更阴的一条：`subscribeDataSource` 里 `type: "store"` 的响应式订阅**绕过 `resolveDataSource`**，而初始加载失败只走 `onError`、流程继续往下 ⇒ 就算在 `resolveDataSource` 里拦，订阅照样起来。
   - `DynamicUIRenderer.tsx:181-182` 的注释本来就写着「树内**所有** `executeActions` 调用点都必须带上它 —— 任何一处漏传都等于对不可信 schema 放行了一条 IPC」。`:220` 那个 `subscribeDataSource` 调用点就是漏传的那一处。

### 113.2 改动

| 文件 | 改了什么 |
|---|---|
| `src/lib/dynamicUI/schemaActionPermissions.ts` | 通配 ⇒ **四条逐条列出**：`get_dynamic_ui_schema` / `get_dynamic_ui_form_data` / `save_dynamic_ui_form_data` / `delete_dynamic_ui_form_data`（不给通配 = 新增命令默认拒绝）；文档块重写，记下 113.1-1 那条链 |
| `src/lib/dynamicUI/DataBindingEngine.ts` | 新增 `assertDataSourceCoveredByPermissions(config, permissions)`；`resolveDataSource` / `subscribeDataSource` 各接一个可选 `permissions` 入参；订阅路径的判定放在**三条出口之前**且走 `onError` 不抛出（调用方只挂 `.then` 没 `.catch`，抛出即未处理拒绝） |
| `src/components/dynamicUI/DynamicUIRenderer.tsx` | `:220` 的 `subscribeDataSource` 补传 `actionPermissions`，并加进该 effect 的依赖数组 |
| `src/lib/dynamicUI/useDataSource.ts` | 同样把面转发给两个引擎入口（当前无生产调用方，转发是为了将来接上时不天然绕过） |
| `src/lib/dynamicUI/dataSourceSecurity.ts` | 注释点明「这条只是第一道，按前缀放行 ⇒ 含三条写命令；改这层不构成对插件的收口」 |
| 三个测试文件 | 见 113.3 |

### 113.3 取证

新增判据测试都配了同型正控（先证明「带上权限面没把路整体打死」，才谈得上「拒绝是判据命中，不是链路没起来」）：

- `src/lib/__tests__/schemaActionPermissions.test.ts`：面内四条逐条命中 + 九条面外（含 `create/update/delete/restore/*_nl/pin/unpin/list_dynamic_ui_schemas`）逐条拒；旧通配形态下这九条会**全部**放行，所以这组断言本身就是回归锁。
- `src/lib/__tests__/dataBindingEngine.test.ts` 新增「来源受限权限面」7 例：invoke 面内放行／面外写命令拒且**不发 IPC**、`storeRead: []` 拒读且**判定发生在查 registry 之前**、`storeName:fieldPath` 覆盖语法（分隔符是**冒号**，见 `src/lib/skillPermissions.ts:86-98` 的 `parseStorePerm`）、覆盖某 store ≠ 覆盖其全部字段、不传面时行为逐字不变、`subscribeDataSource` 在直订阅分支前就拦。
- `src/components/dynamicUI/__tests__/pluginSchemaActionPermissions.test.tsx`：原先两处**用假命令名**（`dynamic_ui_get_dynamic_ui_form_data` / `dynamic_ui_list_pins`）—— 真实命令名没有 `dynamic_ui_` 前缀，旧通配把它们一并盖过所以没暴露；已换成真名，并补 `create_dynamic_ui_schema` 被拒的提权链用例。

**验证读数**：`npm run typecheck` EXIT=0（**1476** 文件）；`npx dprint check` 8 文件 EXIT=0；`npx oxlint src/lib/dynamicUI src/components/dynamicUI src/lib/__tests__` 0 warnings 0 errors（113 文件）；`node scripts/check-component-neutrality.mjs` PASS 无新增违规；8 文件 `perl -ne '$c++ if /\r$/'` 全 CR=0。
⚠ **vitest 读数未取得**：`npm run test:run -- <两文件>` 报 `[vitest-pool]: Failed to start threads worker`（120 s，#141 的 SMB worker 启动超时，非本批代码问题）。⇒ 上面 16 条新增/改写断言**在本机没有执行级取证**，判据正确性靠读 `parseStorePerm` / `isWildcardMatch` 源码推演 + 类型面。CI 亦无 vitest 步骤（既存缺口）。

### 113.4 行为面变化（需要知道的代价）

- **只影响 `origin = "plugin"` 的 Schema**。`builtin` / `user` / `ai` 与内存态 Schema 一律维持原行为（面为 `undefined`，两条闸都不介入）。
- 仓内实样**零影响**：`axagent-plugins/crates/plugin-stock/ui.json` 没有任何 invoke 动作（`grep -n "invoke\|command"` 零命中）。
- 对**第三方插件**是破坏性收窄：原先能用 `list_dynamic_ui_pins` / `pin_*` / `*_nl` 这类 `dynamic_ui_*` 命令的声明式动作，现在必须走宿主命令面。这是本条改动的目的，不是副作用。

### 113.5 未做

1. **已闭合（`43803e5ac`；本会话此前误登记为「在做」，2026-11-06 读源码纠）**：`*_form_data` 三点与 update / delete / 回滚 / 钉入共 7 个调用点全部过统一闸门 —— `src-tauri/src/commands/dynamic_ui.rs:132`（`load_schema_for_command`）被 `:411` / `:484` / `:624` / `:689` / `:739` / `:759` / `:1070` 调用，判据是 `TRUSTED_SCHEMA_ORIGINS`（`src-tauri/src/commands/dynamic_ui.rs:104`）白名单取反 fail-closed（`:112`），单测 `:1195` 钉住空串 / `bogus` / `Plugin` / 带空格四种全拒。**`owner_id` 刻意不进授权判定**（非插件行它恒等于空串 ⇒ 加进去零区分度），这是终局形态，不要再「补归属校验」。
2. 同批堵死 `origin` 自报：`create_dynamic_ui_schema` 在 `src-tauri/src/commands/dynamic_ui.rs:375` 硬写 `origin="user"`、`owner_id=""`，入参 `req.origin` / `req.owner_id` 被静默丢弃 ⇒ 本条原句「插件仍可读写他人 Schema」在生产路径不成立。**残余在两处**：`src/lib/browserMock.ts`（dev 模式）仍采信 `req.origin` / `req.ownerId`，且它的 update / delete 没有任何闸 ⇒ 按 mock 开发会得出「后端也不拦」的错误结论；`schema_json` 字节上限仍未加（见下条）。两者分别转 #186 / #185。
3. `schema_json` 体积上限仍未加（既存延期项）。
4. `useDataSource` 无生产调用方，本批只做「接上时不天然绕过」的转发；真要启用需带来源解析。
5. 渲染链级的 dataSource 执法（`DynamicUIRenderer:220` 那一行接线）只在引擎层有单测，没有加组件树测试 —— 本机 vitest 跑不动（#141），加了也无法取证。

## §114 B8-5 第二批：股票域类型门面摘除 + 拆 `./pipeline`（#162，2026-11-06）

### 114.1 先纠一处我自己的口径

上一批收尾时说「31 个符号变成零消费，为下一批铺路」—— **这个数不成立**，执行者按实测推翻：`src/types/stock-analysis.ts` 里传递性零消费的**手写**类型只有 2 个，其余要靠文件头那块**向后兼容 re-export 门面**（29 名，其中 23 名零消费）才凑得出，且仍凑不到 31。⇒ 本批按实测做，**删 25 项**。判据层的教训：**「零消费」的判据是「没有文件把这个模块当导入源要这个名字」，不是「全文搜不到这个串」** —— `Consensus` / `Sentiment` 这类符号的权威定义在 `src/lib/stock-analysis-utils.ts`，61 个文件从那里直引，全文命中会给出一堆假「仍在用」。

### 114.2 已删（25 项，`src/types/stock-analysis.ts` 2336 → 2292 行）

| 档 | 数量 | 内容 |
|---|---|---|
| 门面 `export { … } from "@/lib/stock-analysis-utils"` | 13 | `computeEvidenceDrivenConsensus` · `computeEvidenceWeights` · `computeStockConsensus` · `Consensus` · `getActionTagStyle` · `getActionTKey` · `getRiskColor` · `getRiskTKey` · `parseRiskLevel` · `Sentiment` · `STOCK_ACTION_LABELS` · `STOCK_RISK_LABELS` · `StockRiskLevel` |
| 门面 `export type { … }`（证据权重类型） | 7 | 整块摘除：`AnalystInput` · `AnalystWeight` · `EvidenceConsensus` · `EvidenceWeightReport` · `EvidenceWeightRequest` · `HoldGateResult` · `MarketRegimeInfo` |
| 门面 `export type`（只摘对外，`import type` 留） | 3 | `PositionStateType` · `StockActionType` · `StockRiskLevelType` —— 本文件内部仍消费它们（`StockDecision.action` / `positionState` / `riskLevel`），**摘 import 会红** |
| 本文件手写类型 | 2 | `AnalysisConfig` · `HorizonHitrateGroup` |

保留 6 个门面名（`classifySentiment` / `getActionColor` / `getSignalColor` / `parseAction` / `StockAction` / `StockConsensus`）—— 有真实消费方，其中两条来自 `src/types/__tests__/stock-analysis.test.ts` 与 `src/stores/feature/__tests__/stockAnalysisStore.test.ts` 的**相对路径** `from "../stock-analysis"`，**纯文本搜 `@/types/stock-analysis` 搜不到**。

⚠ 顺带纠出一个**扫描器自身的坑**（首版误判 4 个 `HorizonResult*` 为死）：单行 `export type X = …` 的引用 span 若从声明**下一行**起算，就漏掉同行的内部引用 ⇒ 判据必须让 span 起点对齐声明行。

**`EarningsEventType` 我判删**（执行者列为裁定项，理由是我上一批称它「仍在用」）：`grep` 实测全仓唯一命中 = 它自己的定义行，且 `EarningsEvent.eventType` 已是 `string`，删它不动生成物契约。它上面那段「纯前端 UI 约束」的注释同批删 —— **留着就是一段没人兑现的假话**。

### 114.3 任务 B：拆掉 `src/types/index.ts` 的 `export * from "./pipeline"`

判据：`src/types/pipeline.ts` 5 个导出落在 `// === AxInvest 投资域（stock/quant/pipeline） ===` 段内 ⇒ 明确股票域；与 barrel 自身 278 个名字零重名、`src/types/` 内部零交叉、外部消费方仅 5 个文件（对比 `./quant` 11 文件且 `BacktestResult` 与 stock-analysis 同名，`./opc` 属 OPC 域）。改法与第一批逐字同形态（删行 + 原地留说明），5 个消费方改深路径，混合导入块拆两条（`adapters.test.ts`）。

### 114.4 `src/stores/index.ts` 拆不动 —— 这是门禁与 barrel 互相锁死，不是偷懒

股票 store 在 `src/stores/index.ts:102-111`，**68 个消费方**从 `@/stores` 取它们，其中 **36 个落在 `src/components/**` 扫描面内**。`scripts/check-component-neutrality.mjs:82-85` 的 `adjudicateImport` 只匹配 `^src/stores/feature/([^/]+)` ⇒ **`@/stores` barrel 当前完全不入判据**；一旦改深路径就立刻入判据，而 owner 比较是「目录名 vs 剥 `Store` 后缀的文件名」**逐字**比 ⇒ `src/components/stock-analysis/**`（26 文件）取 `stockAnalysisStore` 会因 `"stockAnalysis" ≠ "stock-analysis"` 判成**新增违规**，连目录同源的都不自认。
⇒ 需要的是**前置裁定**，不是改 import 串。裁定见 114.5-③。

### 114.5 四项裁定（执行者交上来判据不足，逐条裁）

1. **`EarningsEventType`** ⇒ 删（114.2 末段）。我的「仍在用」是错的，按判据走。
2. **向后兼容门面算不算本批清理对象** ⇒ **算，不回退**。插件化的目标形态就是主壳不再经 `types/` 门面把股票域的**实现函数**（`computeEvidenceWeights` 等）转发出去；这 20 行摘掉后，`src/lib/stock-analysis-utils.ts` 是该符号的唯一可见入口，正是想要的方向。
3. **`stock-analysis` ↔ `stockAnalysisStore` 命名不同源** ⇒ 判**在门禁侧做 camel↔kebab 归一**，不判重命名 store 文件（重命名要动 68 个消费方，且 `stores/feature/` 下已有同族命名）。⚠ 这条属于**改判据**，必须独立成批：归一后 `src/components/stock-analysis/**` 那批既有基线条目会**变成自认** ⇒ 棘轮应当**下降**，须先出「归一前/后基线条数 + 逐条名单」再动表，不许拿它当「顺手洗白」。未裁定前 `src/stores/index.ts` 保持现状，不是缺陷。
4. **`scripts/i18n-exemption-budget-baseline.json` 里 30+ 条 `src/components/stock-analysis/**` 豁免项**随股票域迁出怎么处理 ⇒ **迁出批落地后**在并行期之外跑一次 `--update-baseline`，与全仓棘轮收紧同批；本批不碰。

### 114.6 验证

`npm run typecheck` EXIT=0（**1476** 文件，改后复跑）· `npx dprint check` 7 文件 EXIT=0 · `npm run lint:eslint`（oxlint）0 warnings 0 errors / 1452 文件 · `node scripts/check-component-neutrality.mjs` PASS 无新增（基线 212，未跑 `--update-baseline`）· 7 文件 CR=0。
因三道股票门禁会读本文件的接口键集，**主动加验**：`check-monitor-event-authority.mjs` / `check-reco-loop-status-coverage.mjs` / `check-horizon-decision-keys.mjs` 各 EXIT=0（删 `EarningsEventType` 后复跑仍全 0）。
⚠ **vitest 读数未取得**（#141）。执行者另补一条本机证据：`npx vitest run <单文件>` 与 `--root .` 两种写法都被空格路径截断成 `Z: Folders/D/OneManager/AxInvest` ⇒ `No test files found`，EXIT=1 —— 与 #142 同一族的根路径症状，不是用例失败。`src/types/__tests__/stock-analysis.test.ts` 是本批的直接相关用例，**没有执行级取证**。

### 114.7 未做

1. `src/stores/index.ts` 股票 store 未拆（114.4，等 114.5-③ 的判据批先行）。
2. `export * from "./opc"`（`src/types/index.ts:4`，仅 2 个消费方、16 导出、零重名）未拆 —— 属 OPC 域，本批口径限死股票域。**建议下一批首刀就是它**（影响面最小）。
3. `export * from "./quant"`（`:2448`）未拆（11 消费方 + `BacktestResult` 同名需先定权威）。
4. `src/types/index.ts` 有**重复 barrel 行** `export * from "./wiki"`（`:1543` 与 `:1980`）—— 与本批两域无关，未动，登记备查。

## §115 #36 第 3–4 步：`opc.store` 只读接缝 + OPC 第一条端到端插件命令（#163，2026-11-06）

派给 cargo 写通道的执行者在 150 轮上限处被截断（截断时正打算「开始写契约层」，实际盘上已落 845 行改动）⇒ **自检由 leader 接手做完**，下列全部读数是 leader 重跑的。

### 115.1 接缝面（照 `analysis.record` 的形态逐段对齐，四段齐全）

| 段 | 文件 | 行数 | 干什么 |
|---|---|---|---|
| 契约 | `axagent-core/crates/harness/src/opc_store.rs` | 324 | `BlogPostView`（10 字段）+ `BlogPostListOutcome{posts,total}` + `trait OpcStoreProvider`，`OPC_BLOG_POST_LIST_LIMIT_RANGE=(1,200)` / `DEFAULT_LIMIT=50` 有编译期 `const _` 自洽断言；9 条单测 |
| 注册槽 | `capability_registry.rs` + `lib.rs` | +35 / +8 | `ServiceDefinition::opc_store()` + `register_opc_store` / `get_opc_store` |
| SQL | `axagent-infra/crates/dao/src/repo/opc_blog_post.rs` | 192 | `select_only()` 逐列点名 + `(created_at DESC, id ASC)` + `count_blog_posts`；3 条纯函数单测 |
| 宿主 provider | `src-tauri/src/init/opc_store_service.rs` | 189 | `HostOpcStore` 只调 dao（**不出现 SQL**，#114 那条），`limit` 越界**拒收不夹取**，闸在 IO 之前 |
| 分派 | `plugins/src/worker.rs` | +166 | `("opc.store","list_blog_posts")` 分支：两个可选标量走 `deserialize_optional_arg`，出参具名对象；测试替身用 `parking_lot::Mutex`（不是 `std::sync`） |
| 插件 | `plugin-opc/src/main.rs` | +278 | 首条命名命令 `opc_list_blog_posts`：`calls` 里声明 `opc.store`，不传分页则**按 `total` 翻页取尽** |

三处判断值得记下来（都是执行者自己想到、派单里没写的）：

1. **`content` 正文列连 SELECT 都没有** —— 裁剪做在 SQL 侧，不留「进了宿主内存但没进帧」的中间态（`analysis.record` 那批是复用了既有查询、内存里有正文，这里比它更严）。
2. **`defaultEnabled: true` 先补齐再删宿主命令** —— 这正是上一批 #149 学到的那道闸：内置包不默认启用 = 删完宿主命令该功能静默失效。
3. **分页终止看两个条件**（`total` 与空页）—— 只看 `total` 在并发写入下会吊死；`offset` 按**实际取回条数**推进而不是按请求的 `limit` 推进，最后一页本来就短。`DashboardTab` 的 `blog_posts_count` 取的就是这个数组的 `length`，不取尽会把 137 篇显示成 50 篇。

### 115.2 三个必修缺陷（由并行的只读核查抓出，判据 leader 逐条复核过）

删宿主命令这个动作本身没问题，**问题是删完之后插件这条路根本走不通** —— 三个都判在「命令在两条消费通道都拿不到数据」这一档：

- **M1｜worker 第二道身份自检恒失败。** 宿主注入的是**注册表键** `{小写 name}@{marketplace}`（`plugins/src/manager.rs:3636` 的 `plugin_id()`，marketplace 常量在 `plugins/src/core.rs:30`），内置包实为 `opc@bundled`；骨架里写死的却是 `"plugin.opc"` ⇒ 每条真实命令都落 `PLUGIN_COMMAND_ID_MISMATCH`，而那个码不在回退面。**这条是我上一批建骨架（`c6c4ce198`）时带进来的**，当时零命令所以惰性潜伏，接上真命令才炸。修法**不是**照抄 `plugin-stock` 删掉第二道（那道管的是宿主把帧错配到别的 worker，此时「帧绑定值 = 注入值」是一起错的，第一道完全看不见），而是改成只比**名前缀**：`main.rs:40` `const PLUGIN_NAME = "opc"` + `:43/:107` 的 `own_id_belongs_to_this_plugin`。
- **M2｜测试同源自证。** `tests/protocol.rs` 注入的就是那个错值，所以它绿而功能死。改成宿主真形态（`:24` `opc@bundled`），并补 `identity_check_accepts_any_marketplace_but_rejects_foreign_plugin` 两条：`opc@external` 必须**放行**（拿未知命令名当探针，`PLUGIN_COMMAND_UNKNOWN` 与 `PLUGIN_COMMAND_ID_MISMATCH` 可机械区分）、`stock@bundled` 一致错配必须**拒**。
- **M3｜本地根本产不出 opc 产物。** `package.json:68-69` 把 `--out` 写死成 `.../stock` —— 那是插件清单的**第二份**，加插件时没人想起改它。改成两条脚本一律转调 `scripts/stage-plugin-bundles.mjs`（清单只剩一份），并给它加 `--root <目录>`（开发期落点是编译期默认根 `axagent-plugins/crates/plugins/bundled/`，与发布落点不同）与 `--debug` 档。

### 115.3 已知偏离：出口手工改回 snake_case

接缝视图按全站约定出 camelCase，但 `plugin-opc` 的命令出口把它逐字段改回 snake_case，为的是与既存前端 `interface BlogPost` 及被替代的宿主 DTO（`company-runtime` 的 `BlogPost` **没有** `#[serde(rename_all)]`）逐字一致 —— 不动前端两个文件的类型与 `dataIndex`。代价：主壳的 snake_case 消费面被留在原处（禁区 13 的方向），且插件里多出一个第二映射点。**登记为 #167**，正解随 OPC 前端类型收口批一起做。

### 115.4 智能体（T1）侧的等价性 —— 结论：没有静默退化，但有三格不等价

只读核查把链路钉到底：`#[agent_command]` 宏**只产元数据**（`inventory::submit!`），交给 LLM 的 tools 是静态表；而 `call_mode = StateInput` 的执行终点 `invoke_state_input` 返回的是占位 JSON（`agent/command_bridge.rs` 的 `"status": "ready_for_invocation"`），全仓不存在「命令 → 工具」的桥。⇒ 宿主那条挂在智能体侧**从来就不可真执行**，是「索引里写着 ✓ 可用 + 调用回假成功」，删掉它降的是幻觉诱因。插件形态反而**真执行**。三格不等价登记为 #170（智能体通道错误无 code；`list_available_commands` 是死臂）与 #171（`inputSchema` 没投影进索引 ⇒ LLM 看不见 `limit/offset`）。

### 115.5 验证（leader 重跑，逐条给退出码）

- 定点 `cargo check --all-targets`：`axagent-harness` EXIT=0（1m59s）· `axagent-dao` EXIT=0（7m49s）· `axagent-plugins` + `axagent-plugin-opc` EXIT=0（3m18s）—— 四者日志里都有真的 `Checking axagent-*` 行，不是只吐 `Compiling`/`Finished`。
- 主 crate `cargo check --lib`：改前 EXIT=0（20m30s）、fmt 后复跑 EXIT=0（14m51s，尾部 `Checking axagent v3.0.0` ✓）。
- 定点 `cargo clippy -p … --all-targets -- -D warnings`：plugins+plugin-opc EXIT=0（fmt 后复跑 1m15s）· harness EXIT=0 · dao EXIT=0。
- `__TAURI_WORKSPACE__=true cargo test -p axagent-harness --lib opc_store` ⇒ **9 passed / 0 failed**（944 filtered）；`-p axagent-dao --lib opc_blog_post` ⇒ **3 passed**（390 filtered）；`-p axagent-plugin-opc --test protocol` ⇒ **5 passed**（含 M2 新用例，改后复跑仍 5/5）；`-p axagent-plugins --lib opc_store` ⇒ **2 passed**（251 filtered）。
- `cargo fmt --check`：`axagent-core` / `axagent-plugins` / `src-tauri` 三根改前各报 diff（共 8 处，**逐条归属后全部落在本批文件内**，无外人既存债混入），`cargo fmt` 后三根复跑全 EXIT=0；`axagent-infra` 本来即 0。
- ⚠ **未跑**：全量 `clippy --workspace` / `test --workspace` / `cargo fmt --all`（并行期铁律）。⇒ 本批的联合态门禁仍未验证。

### 115.6 读数变化（登记表与 PLAN 正文由下一批同步，本批未改表）

| 读数 | 改前 | 盘上实测 |
|---|---|---|
| `commandTotal` | 1542 | **1541**（`register_commands.rs` 由 build.rs 自动少一行，未手改） |
| `opcBoundaryCommands` | 138 / 13 模块 | **137 / 13** |
| K1 `.rs` 文件数 | 2042 | **2045**（+3：`opc_store.rs` / `opc_blog_post.rs` / `opc_store_service.rs`） |
| K1 `pub trait` | 326 | **327**（+`OpcStoreProvider`） |
| K1 有外部实现的 trait | 242 | **243**（+`HostOpcStore`） |

⚠ 抄读数时的坑：`--dump-readings` 里与登记表对齐的是 **`k1.declaredExternalTraits`（243）**，不是同段那个 `externalImplTraits: 282` —— 后者把 `Default` / `Clone` / `Debug` / `From` 这些 derive 全算进去了。⇒ 以门自己的 FAIL 文案为准，别按 dump 的字段名猜。

### 115.7 未做

1. `opc.store` 只有**读面一个 op**；写面（发布 / 建草稿 / 删）、落地页 / 联系提交 / 排期三张表都还没挂上来。
2. OPC 的 T1 工具迁移（41 站点）与 `OpcSendNotification` 的 `set_opc_notify_tx` 零调用方 panic **没碰** —— 那是 #36 后续步骤，且要先修那个既存 panic。
3. dao 侧**没有真库端到端测**（本批只测行收敛的纯函数）：`SELECT` 是否真的没带 `content`、`ORDER BY id` 是否落进 SQL，都要预置 `opc_blog_posts` 行才能钉，属跨批项。⇒ 「列裁剪做在 SQL 侧」目前只有源码判据，没有运行判据。
4. 前端两个调用点没有组件级测试（vitest 在 SMB 上跑不动，#141）。
5. 未跑 `--update-baseline`、未同步 `capability-register.json` 与本 PLAN 的 §0.7/§0.10 镜像数字（第五道门对账批统一做）。
6. `SiteService::list_blog_posts`（`company-runtime`）宿主侧调用方归零但 `pub trait` 方法不报 dead_code ⇒ 留着是**零调用实现**，待 OPC 整域迁出批一并收口。

## 116. 第五道门对账批 + `opc.store` 运行侧收口（2026-11-06）

> 本批**只动两张表**：`scripts/capability-register.json` 与本文档。零代码改动 ⇒ 不触发任何 cargo 命令。
> 存在的理由：§115.6 把「盘上实测」与「登记读数」的差列了出来、§115.7 第 5 条明写「待对账批统一做」。**没对账的登记表是假绿**
> —— 门禁此时仍在 PASS，但它 PASS 的是一个月前那份世界。

### 116.1 改了什么（行数逐字给）

| 文件 | +/− | 内容 |
|---|---|---|
| `scripts/capability-register.json` | 96 / 96 | 11 个锚点值 + 与之一致的 `planNumber` / `mustContain` token + 10 处 PLAN 行重钉 + 3 处代码证据行重钉 |
| `docs/plans/PLAN-shell-plus-plugins.md` | 15 / 15 | §0.7 / §0.10 等镜像小节里的 15 条读数改到盘上实测值，历史措辞保留（见 116.3） |

11 个锚点值逐条（改前 → 盘上实测）：`register_commands` 1542→**1541**（其奇偶镜像同值）· `opcBoundaryCommands` 138→**137**（模块数 13 不变）· `auditL1` 117→**116** · 前端 `.ts` 行数 84850→**84813** · 60973/213→**60938/212** · 102/19→**54/16** · `opc` 侧 0/0→**48/3** · K1 `.rs` 2042→**2045** · K1 `pub trait` 326→**327** · K1 有外部实现的 trait 242→**243**。

⚠ 抄读数只认 `--dump-readings` 的 **`k1.declaredExternalTraits`（243）**；同段那个 `externalImplTraits: 282` 把 `Default`/`Clone`/`Debug`/`From` 这些 derive 全算进去了，不是同一件事（§115.6 已记一次，本批按门禁 FAIL 原文复核确认）。

### 116.2 门禁（本批唯一的判据来源）

- `node scripts/check-capability-register.mjs` ⇒ **EXIT=0，FAIL 计数 0**，尾部 PASS 原文：
  `hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）。`
- `node scripts/check-doc-refs.mjs` ⇒ EXIT=0（本批新增的 `文件:行` 引用全部可定位）。
- `check-line-endings.mjs`：本批两文件 CR=0；唯一违规仍是 `docs/reference/skill-manifest-v2-example.json`（**他人在改的文件，不碰**）。
- ⚠ **PASS 不是自证的**：登记表 96 行改动做过程序化审计 —— 逐条比对「有无删除锚点 / 有无条目 / 有无 `hard` 由 true 翻 false」。结论：**三项均为零**。本批只把数改对，**没有为了让门绿而削任何判据**（未跑 `--update-baseline`、未弱化 `mustContain` token、未动扫描器）。

### 116.3 镜像小节的改法是「补注」不是「换数字」

PLAN 里那 15 行原本是**当时那一批的实证记录**（含「改前 → 实测」两栏）。本批只把**仍指向旧世界的当前值**改到实测值，并保留原句与归因；凡「某数字曾是 X」的史实陈述一律不删。
⇒ 也就是说：本批之后，PLAN 的历史段仍可读，但「现在的数」只有一个来源，且与登记表逐字相同 —— 这正是第五道门要钉的同一性。

### 116.4 打包证据（收口 §115 的 M3：脚本改了 ≠ 产物能用）

`npm run plugin:bundle:dev`（= `stage-plugin-bundles.mjs --debug --root axagent-plugins/crates/plugins/bundled`）
⇒ **STAGE_EXIT=0**（取的是 npm 包装器自己的退出码，不是管道尾端的 `tail`）。日志逐插件给出落点与产物 sha256：

- `stock ⇒ D:\OneManager\AxInvest\axagent-plugins\crates\plugins\bundled\stock` · sha256 `dd625ff6…3027bc`（worker + `pages/index.html` + `ui.json`）
- `opc   ⇒ D:\OneManager\AxInvest\axagent-plugins\crates\plugins\bundled\opc` · sha256 `fd66d9bc…360ad2`（**只有清单 + worker**，本批没有页面资产 —— 形状与 plugin-stock 不同是有意：OPC 前端贡献面还没动）

`axagent-plugins/crates/plugins/bundled/opc/plugin.json` 运行时字段（`node -e` 实读产物，非源码推断）：
`name="opc"` · `defaultEnabled=true` · `commands=["opc_list_blog_posts"]` · `worker.program="bin/axagent-plugin-opc.exe"`（产物 780800 B 已在同目录 `bin/` 下）。

> 这条证据的意义：M3 原来只有「`stage-plugin-bundles.mjs` 加了 `--root`/`--debug` 两个参数」这种源码判据。
> 现在有了运行判据 —— **宿主机上真的能发现一个带第一条命令的 OPC 插件产物**。清单里那个 `commands` 数组只有一个名字，
> 与 §115 的插件侧唯一命令逐字相同，两处不一致就会在 worker 首帧落 `SEAM_CALL_UNSUPPORTED` 之外的另一条错误码。

### 116.5 未做

1. 登记表的 `hard:false` 条目（软判据）本批**没逐条复核** —— 门禁只钉 `hard:true`。软判据漂移要靠人，目前无人。
2. `capability-register.json` 里 `DOMAINS.OPC.ownedRe` **仍缺 plugin-opc 那一支**（§115 归因过的四条判据缺陷之一）。本批**刻意没补**：补它要先决定「OPC 业务迁出后哪些文件算 OPC 域」，那是判据变更，不是对账 —— 判据变更必须与重算 `alsoAt`  occurrence 同批，不能夹在对账里。
3. 联合态门禁（全量 `clippy --workspace --all-targets --all-features` / `test --workspace` / `fmt --all`）仍未跑 —— 并行期铁律。
4. 没有把 §116 的 15 行镜像改动反映进 `docs/INDEX.md` 的该文档「对应版本」字段（本 PLAN 是活文档，索引里只登记一条入口，无数字镜像）。

## 117. 并行六路的一轮：三个「我的前提」被执行者推翻 + 一轮对账（2026-11-06）

本轮一次派六路（一路 cargo 写者 + 一路前端写者 + 四路只读取证），落六个提交：
`b169351a6`（§116 对账）· `fc577c513` #167 出口 camelCase · `2dfce7b51` #172 摘 `./opc` barrel ·
`ed75ffff7` 本节 113.5 纠偏 · `975652a17` #186 mock 对齐 + #178 首刀 · `a9e68621a` #179 接缝第一个写 op ·
`ccbd29f74` #189 前置（测试与编译期 bundled 根解耦）。

### 117.1 我写错、被执行者推翻的三条（照惯例逐条留名，不悄悄改）

1. **「`src/types/opc.ts` 的 16 个符号零引用」—— 错，实际 13 个有活消费方**，且正是经我打算摘掉的
   `index.ts:4` 那行 barrel 取得。若按我的结论落地，`DemandDiscoveryPage` 与 `browserMock` 当场 TS2305。
   - **根因不是搜索词不对，是判据形式**：我用一次性 node 脚本扫引用面，**它一行都没打印**（连
     「检查了几个文件」的计数都没输出），我把「探针静默」当成了「命中为零」。全文搜串固然会算多
     （注释假命中），但**算少的成因必须是「覆盖面没跑成」**，而没跑成一定是可自证的 —— 探针必须
     自己打印文件数。第二轮派单把这条写成了硬要求：双引擎（`git grep` + 显式遍历）交叉、两侧
     文件集合与命中行集合逐字一致才算结论，且「超时/空输出 ⇒ 记为探针失败，不许当零命中」。
     第三引擎（ripgrep 在 `src` 根跑 16 项大 alternation）实测**静默漏项 + 20 s 超时**，已被弃用为判据。
2. **「接缝时间口径是 epoch 毫秒」—— 错，是 epoch 秒**。判据：`now_ts()` 函数体是
   `chrono::Utc::now().timestamp()`，而它的文档自述「秒级 `now_ts` 只用于 DB 时间列与跨天计算，
   `now_ms()` 才是毫秒」。这条是我在上一批（§115）写进契约注释的，连带把 `SitesTab.tsx` 的注释也带歪。
   - 收口方式不是「改注释」了事：新写的 dao 真库用例用 `(1_000_000_000..100_000_000_000).contains(&published_at)`
     **机械区分秒与毫秒**（毫秒量级 ≥1e12 必然越界）⇒ 单位从此有运行判据，不再靠散文。
   - 抄错单位的真实后果也写进了契约注释：不是「数值难看」，是 **KPI 静默少一篇** ——
     `published_at BETWEEN from AND to` 的窗口两端都是秒，写毫秒会让该行永远落在窗外。
3. **「后端 `dynamic_ui` 的两条归属校验缺口仍在做」—— 错，早在 `43803e5ac` 就闭合了**。复核证据：
   统一闸门 `load_schema_for_command`（`src-tauri/src/commands/dynamic_ui.rs:132`）被 7 个调用点使用
   （`:411` 更新 / `:484` 删除 / `:624` 回滚 / `:689` `:739` `:759` 表单三点 / `:1070` 钉入），
   判据是 `TRUSTED_SCHEMA_ORIGINS`（`:104`）白名单取反 fail-closed，单测 `:1195` 钉住空串 / `bogus` /
   `Plugin` / 带空格四种全拒；`create` 在 `:375` 硬写 `origin="user"`、入参静默丢弃。
   - ⇒ **教训是「审查抓出的绕过要定期回读销账」**：我把它登记成「在做」整整一轮，期间还据此派过单。
   - 顺带一条判据形态结论：**`owner_id` 不进授权判定是终局形态而非缺口**（非插件行的 `owner_id`
     恒等于空串 ⇒ 加进判定零区分度），已写进 PLAN 与代码注释，防止下一批再去「补归属校验」。

### 117.2 执行者撞 150 轮上限的处置：leader 接完链，不重派

#179 那路在 150 轮时只绿到第一档（harness `cargo check --all-targets`），报告只剩一句「继续 dao 层」。
处置是**我自己把剩下的链跑完**（重派会把已落盘的 785 行改动再解释一遍，代价更高）：
`cargo test -p axagent-harness --lib opc_store` 13 passed · `axagent-dao --lib` 394 passed（含新真库用例）·
`axagent-plugin-opc --bins` 3 passed · `--test protocol` 6 passed · 四个包的
`clippy --all-targets -- -D warnings` 全 0 · 主 crate `cargo check -p axagent --lib` 0（13m06s）·
`npm run plugin:bundle:dev` 0（产物 sha256 `cc2585a9…`，实读新清单
`permissions=["subprocess_execution","write"]`、`commands=["opc_list_blog_posts","opc_publish_blog_post"]`）。
过程中两处是我自己的命令写错，都记下来：
- 链条第 4/5 档丢了 `cd`（仓根无 `Cargo.toml`，cargo 直接报错），而 `echo "S4_EXIT=$?"` 取的是
  **管道尾端 `tail` 的退出码** ⇒ 假报 EXIT=0。判据：管道后取码必须 `${PIPESTATUS[0]}`。
- `cargo clippy -p axagent-dao --all-targets` 报 `manual_range_contains`（在被判 -D warnings 提升的
  是 **test target**，正是 AGENTS.md「必须 `--all-targets`」那条要抓的位置）。按建议改写为
  `.contains(&now_seconds)` —— 被改写的是 `i64` 区间，**不涉及该 lint 对 NaN 的语义陷阱**
  （AGENTS.md 记的那次是 `f64`），判据逐情形等价。

### 117.3 三条测试红的归因：不是代码坏了，是测试读盘

`cargo test -p axagent-plugins --lib` 三条 `manager::tests` 红（`agent_command_specs` 断 1 得 7、
任务源投影断 1 得 2、`ui_json_keys_gate…`）。**A/B 实证**：把 `axagent-plugins/crates/plugins/bundled/{stock,opc}`
挪开 ⇒ 43 条 manager 用例全绿；按 sha256 复验还原后红回来。
根因是缺省 bundled 根 = `env!("CARGO_MANIFEST_DIR")/bundled`（`manager.rs:445`、`:1907` 的
`unwrap_or_else` 回落），而 §116 里我为 dev 加的 `--root` 恰好把产物暂存进同一个目录 ——
**宿主 dev 模式正是从那里装载，所以「别暂存」不是解法**。
解法是测试与盘上产物解耦：新增测试助手 `isolated_config()`（把 `bundled_root` 钉到 `config_home`
下的空目录，随既有 cleanup 一并删），测试 mod 内 **16 个** `PluginManagerConfig::new` 站点按行号逐条换掉
（不整体替换 —— 助手自身那一处必须保持原样，否则自我递归）。
复验取的是**破坏条件在位**的正控：暂存产物存在时 `256 passed / 0 failed`。
⚠ 同类站点在 `crates/plugins/src/lib.rs`（7 处）与 `hooks.rs`（1 处）仍在，已登记 #189。

### 117.4 本轮新抓出的缺陷（都立项了，不在本轮修）

| # | 缺陷 | 严重度判据 |
|---|---|---|
| #181 | `command_bridge.rs:1078` 的 `permission_mode` 没进 `execute_tauri_command` 的 `input_schema` ⇒ 恒取 `"default"` ⇒ **Caution 档插件命令一律落 `requires_confirmation` 占位、永不真执行**；`:1092` 的 `is_blocked()` 臂无条件不可达；`:1131` 的 `list_available_commands` 是死臂且 `:1073` 的报错文案让 LLM 去用一个不存在的工具 | **目标口径级别**：#146「插件命名命令并入智能体命令面」到今天只做到投影，而门禁全绿 |
| #187 | `create_dynamic_ui_schema` 硬写 `origin="user"` 之后，库里**不可能**再有 `origin="ai"` 的行 ⇒ `DynamicUIManagerPage.tsx:553` 的「AI 生成」标签是死 UI 面（`TRUSTED_SCHEMA_ORIGINS` 里的 `"ai"` 是纯装饰） | 静默退化，由 `43803e5ac` 的安全修复带出 |
| #188 | dev mock 的 `handleCommand` 对入参做**递归** snake_case ⇒ mock 读 `req.schemaJson` 恒 `undefined`，dev 模式下 schemaJson 从未真正落库；且 `*_form_data` / `pin` / `unpin` / `restore` 四条 mock 仍未过闸（后端已过） | 「按 mock 开发 ≠ 按后端行为开发」只收窄了一半 |
| #185 | `schema_json` 字节上限全仓零落点（列无约束、写一次同步落一份快照、每 schema 留 30 份、`list_versions` 一次拉 50 份全文）⇒ 一条 10 MB 行 ≈ 300 MB 版本表 | 既存延期项，数值定案受「本机 sqlite 该表 0 行、活动库是远端 PG」限制 |
| #183 | HEAD 上 `npx dprint check` 就报 2 个未格式化文件（`axagent-plugins/Cargo.toml` 的 members 序 + `crates/README.md` 的表格补宽）⇒ 并行期只能定点 fmt | 执行者一度把它归因成「别人在改」，`git status` 干净即证伪 |
| #175 | `plugin-stock/src/main.rs:727` 既存 rustfmt 债（不带 `-p` 的 `cargo fmt --check` 会把它算进任何一批） | 同上，口径陷阱 |

### 117.5 本轮对账（第五道门）

| 锚点 | 改前 | 盘上实测 | 归属 |
|---|---|---|---|
| `frontendFileCaliber`（hard） | 84 813 | **84 815** | `975652a17` 的 8 个前端文件里只有 `seed-stock-schemas.ts`（路径含 `stock`）命中 `feRe`，+4/−2 = 净 +2；其余 −31/−11/−6 全不命中 |
| `registryFileLines`（soft） | 1609 | **1608** | 产物与 HEAD 逐字相同（`git status` 干净）⇒ 1609 是上一批 ±1 抵掉后没回灌的滞后值 |
| `k2KeyedFaces` 强档（soft） | 37 | **38** | `a9e68621a` 新登记 (opc.store, publish_blog_post) 二元组键控面 ⇒ 方向正确的涨，弱档仍 8 |
| `workflow.reflector` / `evolver` 的 `worker.rs` 证据锚点 | :4356 / :4448 / :4496 | **:4386 / :4478 / :4526** | `a9e68621a` 给该文件净加 217 行 ⇒ 三端点各 +30；`--repin-code --apply` 回写（token / file / anchorText 一字未动，逐字节备份 `.tmp/capability-register.20261105T231325.bak.json`），PLAN `:268` / `:269` 两行的**文字**引用与两处 `planAnchorText` 同批改指（重锚模式刻意不改正文） |

复跑：`--selftest` 全绿（覆盖 17/17 类 check，无空洞）⇒ 判定档 `GATE_EXIT=0`、FAIL 0、
`PASS: hard:true 判据全部一致（登记 37 = A11/B8/C4/H1/K10/D3；能力面 Σ=23；锚点 21 项）`。
未删锚点、未增删条目、未翻 `hard`、未弱化 `mustContain`、未跑 `--update-baseline`。

### 117.6 未做

1. #179 只做了**一个**写 op。`delete_*`（要先配三态 + 保护 id 名单）、`create_*`（要先引入字节闸）、
   落地页 / 联系提交 / 排期三族都还没上 —— 见 #180。
2. 宿主命令 `opc_publish_blog_post` 与 service 写体**都还在**（双声明），前端调用点未切通道 ⇒
   插件写面今天没有真实消费方，只有帧级测试。`tools/opc.rs:960` 那条整表读也仍在绕过接缝。
3. #170 / #171 / #181 三条智能体侧缺陷一行未改（取证完整，改动落在主 crate，一次 15–21 分钟档）。
4. 全量 `cargo clippy --workspace` / `test --workspace` / `cargo fmt --all` 仍未跑（并行期铁律）⇒
   联合态门禁没验证过，这是本轮七个提交共同的未覆盖项。
5. 组件中立性棘轮的 4 条已修复 + 3 条腐烂条目未收紧（#177，`--update-baseline` 只能串行窗口做）。
6. `#32`（OPC 独立仓）照旧被 `#36` 挡着。

## 118. 组件中立性门禁批 A：owner 比较的 camel↔kebab 归一（#168，2026-11-06）

提交 `7594e32d1`，**一个文件** `scripts/check-component-neutrality.mjs`（`git diff --numstat` = +157 / −8，
281 → 430 行）。批 A / 批 B 的分界沿用立项时的裁定：**批 A = 判据变更**（本批），
**批 B = 收紧棘轮 212 → 186**（#177，仍待无写手窗口）。

### 118.1 改的是什么

旧判据把「组件一级目录名」与「`src/stores/feature/<X>Store` 剥掉 `Store` 后的 `<X>`」**逐字全等**比较，
而这两套名字天生是两种命名形：目录 kebab（`stock-analysis`）、store camel（`stockAnalysisStore`）
⇒ 同一个 owner 的两种写法判成跨域。本批只加一层**比较口径**：

- `normOwner(name) = name.toLowerCase().replace(/[-_]/g, "")`（`:127`）；
- 三支比较**同批**归一：owner 支、targetOwner 支、NEUTRAL 支（`:149-150` + `isNeutralOwner` `:130-132`）
  —— NEUTRAL 不归一会和 owner 支分裂成两套口径，故 `n3d` 专门钉它；
- **不作用于棘轮键**：键仍是 `file + " → " + import 字面量原文`，条目里 `target` / `targetOwner` 存原形
  （`k1` / `k2` 两条断言锁住）。若把归一并进键，212 条基线**全部失配** ⇒ 那是洗白而不是修判据。

### 118.2 零假阴是量出来的，不是推的

并集 = 41 个组件一级目录名 ∪ 78 个 `stores/feature` 名（剥后缀后参与；其中 73 个真带 `Store`、
5 个不带）= 去重 100 个名字 ⇒ 归一后 97 键，同键组（成员 >1）**仅 3 组**，且每组都「目录形与 store 形
同时命中」= 同一模块的两种写法（`llm-wiki`/`llmWiki`、`multi-agent`/`multiAgent`、
`stock-analysis`/`stockAnalysis`）⇒ 不存在「两个真不同 owner 被归一成同名」的组。
另有一条未合并组做负控：`gatewayLinkStore`（剥后缀 = `gatewayLink`）与 `link` / `gateway` 两个目录
归一后仍不相等（`n4`）。

### 118.3 取证

判定档实跑（当前树）：`扫描 927 文件 · 原始跨域引用 202 · 白名单豁免后 199` + `PASS（基线 212，本轮已修复 26）`，
`GATE_EXIT=0`；`--selftest` 全绿（原 12 条 + 本批 15 条）。

**两个变异体对照**（临时副本 `scripts/.tmp-*.mjs`，跑完即删，未入库 —— 入库的锁是 selftest 断言本身）：

| 变异体 | 改法 | selftest | 判定档读数 |
|---|---|---|---|
| A 归一退回全等 | `normOwner` 改回 `return name` | **6 条 FAIL**（norm 形态 2 + 同 owner 正控 4），退红 | — |
| C NEUTRAL 不归一 | `isNeutralOwner` 改回 `NEUTRAL_PREFIXES.includes` | **1 条 FAIL**（`③d`） | 逐字不变：202 / 199 / PASS |

C 那一行是本轮最有价值的一条证据：它证明 **NEUTRAL 支的归一是纯防御性改动**，今天的真实数据上
一分钱账面都没动 ⇒ 22 条消失必须全部来自 store 支。这一点由**键集差集**独立复算证实（把 HEAD 版与
本版各插一行 dump 出 `effective` 键集再比对）：

| 口径（都跑同一棵当前树） | 实例 | 唯一键 | 基线外新增 | 账面已修复 |
|---|---|---|---|---|
| HEAD 判据（全等） | 222 | 208 | 0 | 4 |
| 本批判据（归一） | 199 | 186 | 0 | 26 |
| 差 | −23 | −22 | **0** | +22 |

消失的 22 条键**逐条**判定：全部 `kind = store` 且 `norm(目录名) == norm(剥 Store 后缀名)`
（comp 支 0 条、非归一条目 0 条）；新增 0 条。算术自洽：199 实例 − 186 唯一键 = 13 条重复实例
（旧口径 14 条，差的那 1 条正是 `stock-analysis → stockAnalysisStore` 里被归一消掉的重复实例，
23 = 22 键 + 1 重复）。

⚠ **那 4 条「已修复」不是本批的**：旧判据在当前树上就已经少 4 条（文档记的 HEAD 时点值是「修复 1」），
差出来的 3 条来自并行批未提交的改动 —— 这就是记忆里「`--update-baseline` 并行期会把别人未提交的减少
钉进棘轮」的实例。**批 B 钉账前必须先逐条归属这 4 条**，否则会把别人的减少记成归一的战果。

锚点复核：文件 1–92 行一字未动 ⇒ `docs/plans/PLAN-shared-primitives-neutrality.md:147` 引用的
`check-component-neutrality.mjs:53`（`NEUTRAL_PREFIXES` 那行）与本文件 `:82-85` 引用的
`adjudicateImport` 两条正则（PLAN:5354）仍在原位、仍逐字可指。
`component-neutrality-baseline.json` 与 `component-neutrality-allowlist.json` 相对 HEAD **零 diff**。

### 118.4 未做

1. **批 B（212 → 186）没跑**：`--update-baseline` 一个字节都没写。归一后基线里仍躺着 186 条**真**跨域，
   外加 3 条腐烂条目（键指向的文件/target 已不存在）—— 都得等无写手窗口。
2. `findViolations:143` 的 `const selfOwner = owner ?? ownerOf(resolved)` 是 **fail-open** 既存缺陷：
   调用方传进 `owner` 就无条件采信，传错即少判。动它同样是判据变更（要配负控），本批刻意不碰。
3. `#190` 不在范围：归一只消 camel/kebab 的**假**阳性，不消拆 stores barrel 后那 243 条真跨域，
   也不解决「棘轮没有合法收紧通道」。
4. 旧读数的三处文档记录**未回写**（`PLAN-shared-primitives-neutrality.md:147`、
   `docs/plans/PLAN-shell-plus-plugins.md:4485` 的 K6、`docs/plans/PLAN-shell-plus-plugins.md:4557` 的 V16）
   —— 那是各批次当时的实跑快照，改成新值等于篡改取证；差异（228/225/修复 1 →
   旧判据 225/222/修复 4）已由本节的归属表说明。
5. 行尾门禁仍红在他人正在编辑的 `docs/reference/skill-manifest-v2-example.json`（纯 CRLF 139 行），
   与本批无关；本批文件实测 `CR_count=0` 后才用 `SKIP_SIMPLE_GIT_HOOKS=1` 提交。

## 119. dev mock 补齐 dynamic_ui 闸门 + `schemaJson` 落库缺陷（#188，2026-11-06）

提交 `7650ba903`：`src/lib/browserMock.ts`（+116 / −20）与 `src/lib/__tests__/browserMock.dynamicUI.test.ts`
（+480 / −4），合计 **+596 / −24**。

### 119.1 闸门面：上一批只补了三分之一

`975652a17` 只给 create / update / delete 三条接了归属闸；**存 / 读 / 删表单、钉入、回滚五条改前一行闸门都没有**
⇒ dev 模式下插件来源的行可被读写表单、可被钉进导航、可被回滚。本批六条全过同一个
`mockLoadSchemaForCommand`（`:1808`）+ 同一份 `TRUSTED_SCHEMA_ORIGINS`（`src/types/dynamicUI.ts`），**没有第二套白名单**。

`restore_dynamic_ui_schema_version` 按后端严格四步同序（`:624` 闸门 → `:626` 内置守卫 → `:630` 版本查找
→ `:637` 跨 Schema 校验），并把改前混成一条 `Version not found` 的两种情形换成与后端**逐字同文案**的两条串。
`unpin` 是唯一例外：判据是「宿主存在**且** `origin` 非可信 ⇒ 拒；宿主不存在 ⇒ 放行」，
照后端 `dynamic_ui.rs:1130-1132` 的**注释原文**实现（理由 = `delete` 不级联 pins，否则悬空钉入永清不掉），
执行者复核确认这条例外确实是「放行」而非「收紧」，未跟着转述走。

### 119.2 `schemaJson` 从未落库（真缺陷，且是闸门的前提）

`handleCommand`（调用点 `:2045`）对入参做**递归** snake_case（函数 `convertToSnakeCase` `:1195`）
⇒ 只有「嵌套 `req` 且字段名含大写」的读法恒 `undefined`：`req.schemaJson` / `req.schemaId` /
`req.formDataJson` / `req.instanceKey` / `req.changeLog` 全是空读，顶层参数各 handler 早就按 snake 形读，
所以没一起烂掉（这是对派单前提的一处**纠口径**：影响面不是「顶层也受影响」）。

改法用仓内既有先例（`browserMock.ts:2765` 的 `a ?? b` 双键形）收敛成 `readMockReqField`（`:1839`，
camel 优先、回落 `camelToSnake`），落点 create `:5383` / update `:5444`+`:5463` / save `:5502`+`:5510`+`:5514`。
**不动递归转换本身**（那是全 mock 面几十个命令共同的入参形状）。

⚠ 派单没预见的耦合：form_data 三条的宿主键 `req.schemaId` 改前也恒 `undefined` ⇒ 若只加闸门不做键形兼容，
闸门会把**每一次**保存判成「宿主不存在」（假收紧）。所以 119.2 是 119.1 在 form_data 上的前提，同批做。

### 119.3 取证（leader 独立复跑，不复用执行者的日志）

| 项 | 结果 |
|---|---|
| 目标测试文件 | `44 passed / 0 failed`，`VITEST=0`（HEAD 13 例 → 现 44 例，**+31**） |
| 同目录 4 个既存 browserMock 测试（回归面） | `19 passed`，4 files，`VITEST=0` |
| `npm run typecheck` | `EXIT=0`，`tsc 发现 src 源文件 **1477** 个`（改前基线同值） |
| 4 道静态门（jsx-text-i18n / i18n-orphan-diff / component-neutrality / i18n-exemption-budget） | 全 `EXIT=0`，未跑 `--update-baseline` |
| `npx dprint check <2 文件>` | `EXIT=0`（执行者首次 20 → 定点 fmt 后 0，leader 复跑仍 0） |
| 行尾 | 两文件 `CR=0`（perl 计数字节级，不用 `grep -c $'\r'`） |
| 执行者自证的 esbuild 探针 | 25 例（REPRO 3 + GATE 22）：改前 **18 FAIL** / 改后 **0 FAIL** —— 探针件在 `.tmp/`，leader 未复跑，行为级结论由上面 44 例 vitest 独立覆盖 |

⚠ 取数陷阱复现两次并已在记忆里：直接 `npx vitest run <文件>` 会把根路径截断成 `Z: Folders/...`
并报「No test files found, exiting with code 1」—— **这是探针失败，不是零命中**；必须走
`npm run test:run -- <文件>`（`scripts/vitest-run.mjs`）。执行者另记录 3 次 worker 60 s 启动超时
发生在并发 tsc + cargo 的窗口，且用同目录无关既存文件复现 ⇒ 判环境；tsc 退出后连续三次通过。

### 119.4 登记而非本批修

1. 新任务 **#191**：`schemaActionPermissions.ts:76-81` 给插件来源放行 `save/get/delete_*_form_data`，
   而后端与现在的 mock 按 origin 白名单**一律拒插件行** ⇒ 前端权限表给了后端不兑现的承诺。
   两条出路（前端收窄 / 后端按 `owner_id` 真放行，后者与 §113.5-1 同批）必须选一条，不许两边各让一步。
2. 新任务 **#192**：dev mock 与后端的 7 条 dynamic_ui 语义差（版本推进条件、`semver_gt` 冲突校验、
   restore 回滚前存档、delete 不级联 form_data、pin 的 group 内 position、默认 `instanceKey` 字面量、
   `revoke_schemas_owned_by` 无 dev 镜像）—— 逐条要么对齐要么在注释里写明「刻意分歧 + 理由」，不许留成语义差。
3. 执行者未普查 dynamic_ui 之外其它 handler 的嵌套 `req` camelCase 读点（全文件现在只剩 4 处
   `req.[a-z]*[A-Z]`，1 处是 `:2765` 先例、其余是注释文本）⇒ **其它命令域可能仍有同类恒 `undefined`，未验证**。

## 120. 本轮排期与第五道门读数（2026-11-06，含故意不重锚的两条）

第五道门判定档在 `7650ba903` 这棵树（PLAN 已含 §118 / §119）上跑了一次，`SELFTEST` 17/17 类 check 无空洞，
但 **`GATE5=1`：2 条硬失败**，且两条都是**在途写手**造成的锚点漂移，不是读数漂移：

| 锚点 | 登记 token | 盘上实测 | 归因 |
|---|---|---|---|
| `dojo.extension`（组 H · planLine 304） | `pub fn execute_plugin_command` @ `manager.rs:875` | 第 875 行是一行 `///` 文档注释 | `axagent-plugins/crates/plugins/src/manager.rs` 与 `types.rs` 正在被智能体侧那一批改（未提交） |
| `command.handler`（组 K · planLine 315） | `pub trait CommandHandler` @ `command_bridge.rs:1292` | 第 1292 行是 `\|spec\| {` | 同一批正在改 `src-tauri/src/commands/agent/command_bridge.rs` |

⇒ **刻意不跑 `--repin-code --apply`**：锚点在别人未提交的中间态上重锚，等那批落地又得再锚一次，
而且会把「中间态行号」钉成登记表基线。处理办法与 §117.5 相同：那批交回后一次性重锚 + 对账，
同时把 `frontendFileCaliber`（现值仍 84 815）与 §117.6-5 的棘轮收紧一起收掉。

⚠ 一个反直觉的好消息：`7650ba903` 那批 +596 行前端代码**没有**动 `frontendFileCaliber` —— 第五道门对本次
判定档只报上面两条硬失败 ⇒ `src/lib/browserMock.ts` 与 `src/lib/__tests__/browserMock.dynamicUI.test.ts`
都不命中 `feRe`（`/stock|quote|kline|technical|portfolio|trading|advis|invest|quant|market/i`），
口径读数不变。这与 §117.5 里「只有 `seed-stock-schemas.ts` 命中」是同一判据的两次独立验证。
