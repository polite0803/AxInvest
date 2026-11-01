# goink 对比审计：小说书写工作流可借鉴什么

> **审计对象**：`github.com/sigpanic/goink`（Go + Wails 桌面 AI 小说创作助手，346 star）
> **对照对象**：本仓 `workflow-cm-literary-creation`（内容媒体行业「文字创作」工作流模板）及其叙事结构子系统
> **审计日期**：2026-09-15
> **取证方式**：GitHub API tree + raw 并发快照（616 文件 / 3.93 MB），5 组并行侦察兵 + 主审复核
> **git 状态**：全程零 git 命令，纯 HTTP 快照
> **配套计划**：`PLAN-goink-borrowings.md`
> **侦察原始报告**（5 份，含全部文件:行证据）：`output/goink-audit-2026-09-15/recon-*.md`

---

## §0 边界声明

### 许可证（决定借鉴边界）

| 项 | 值 |
|---|---|
| License | **AGPL-3.0**（`repo.json` → `license.spdx_id`） |
| 本轮定位 | **用自己的栈重新实现设计意图，不是移植** |

**AGPL-3.0 是传染性许可。本审计的产出边界**：

- ✅ **可以**借鉴：机制设计意图、数据结构思路、领域方法论（写作规则）、工程判据
- ❌ **不可以**：复制源码到本仓、逐句照搬其 prompt 文本 / `skill/builtin/*.md` 的原文措辞
- ⚠️ 本报告 §5 引用的「写作方法论 87 条」是**从原文提炼的规则要点**，落地到本仓时必须**自己重写措辞**，不得粘原文段落

### 栈与范式差异（决定「代码级移植是否成立」）

| 维度 | goink | AxInvest |
|---|---|---|
| 语言/框架 | Go 1.25 + Wails v2 + React 19 | Rust + Tauri v2 + React 19 |
| ORM / DB | GORM + SQLite | sea-orm + SQLite/PG |
| **编排范式** | **ReAct agent loop**（31 个工具，LLM 自主决策调用） | **节点图工作流**（预定义 DAG + Loop + 条件路由） |
| 状态维护触发者 | LLM 自己（提示词强制 + 工具返回注入提醒） | 工作流节点（编排层强制） |
| 正文存储 | 文件 + 内置 git | 工作流变量 + Word 装配输出 |

> **范式差异的推论**：两者是**互补而非同类**。goink 没有编排层，AxInvest 没有「结构化创作状态 + 自主工具维护」。
> ⇒ **代码级移植不成立，只有机制级借鉴成立。** 本报告所有条目均按「机制」而非「代码」表述。

### 规模实测（先量代码再下判断）

```
goink 源码快照：616 文件 / 3.93 MB（已剔除图片、tiktoken 词表、二进制）
Go      225 文件 /  41,418 行
TS+TSX  244 文件 /  35,966 行
Markdown 93 文件 /  18,241 行
────────────────────────────────
合计              约 9.6 万行
```

结构指标（`analyze.py` 实测）：**40 个 Go 包 / 157 条跨包边**。

| 指标 | 值 | 读法 |
|---|---|---|
| fan-in 最高 | `internal/storage` 18（fan-out **0**） | 真契约层候选：领域名词、零扇出 |
| fan-in 次高 | `internal/git` 13、`internal/config` 11 | 基础设施层 |
| fan-out 最高 | `app` 34、`internal/mcp_tools` 18、`internal/migrate` 15 | `app` = Wails 绑定层（预期）；`mcp_tools` = 工具层（预期） |
| 跨包环 | **不适用** | Go 编译器禁止循环导入 ⇒ 包级图天然无环，「找环」恒零命中，**不能当「它很干净」的证据** |
| 分层护栏 | `.golangci.yml` **无 depguard** | ⇒ **分层是「约定」不是「机制」**（四链只成立第一链） |

---

## §1 总判断：两家各属哪一型

### goink = **出口不严型**（伴少量入边缺失）

| 证据 | 事实 |
|---|---|
| 审批覆盖 | 破坏性工具 **19 个，仅 2 个走审批（10.5%）**：`edit`（rw_tools.go:102）、`delete_record`（delete_tools.go:93）。17 个写 DB 的 create/update/upsert **零审批** |
| fail-open 分支 | `if tc.Approver != nil` ⇒ **Approver 为 nil 时静默跳过审批直接落盘**（rw_tools.go:102、delete_tools.go:93） |
| 回滚承诺 | `delete_record` 描述告诉 LLM「误删可通过操作日志回滚」，但 `handler.go:180-182` 注册 oplog 钩子失败时**仅记 Error 后继续** ⇒ 回滚静默不可用 |
| 回滚执行体 | `RollbackBeforeTurn` / `RollbackTo` **全项目零调用者、无 Wails 绑定、无测试** ⇒ 引擎写了没接线 |
| 技能安全 | `skill/sanitize.go:7-20` **只清洗文件名非法字符**，对技能正文**零清洗、零签名**；远程技能 `index.json` 由远端完全控制、**原样落盘并进入 LLM 上下文** |
| 治理断链 | 7 条 CI 链真跑，但 `check_omitempty` 是「文档声称 CI 自动检查、实际 CI/Makefile/hook 均未调用」 |

### AxInvest 文学创作工作流 = **入边缺失型**（为主）

| 证据 | 事实 |
|---|---|
| 774 行叙事引擎 | `harness/src/narrative.rs` 的**全部类型在模块外零使用**（§2.A 铁证） |
| 叙事结构表三方不通 | 表 / CRUD 命令 / 前端面板是一条链，工作流运行时是另一条链，**两者无交集** |
| KPI 无数据源 | `config/opc/industries/content_media/runtime.yaml` 的 **7 个 KPI 全部 `type: constant` / `value: 0`** |
| 悬空变量 | `lc-chapter-structure.current_chapter` **全仓零产出点**（§2.D） |
| 前端类型孤儿 | `NarrativeKpiMetrics`（arcCompletion/foreshadowRecoveryRate/structureCompliance）**仅有定义，零组件消费** |

### 推论：**检法不可互套**

- 对 goink，要问「**每个 gate 异常分支是放行还是拦截？默认值开没开？**」
- 对 AxInvest，要问「**每个声明必须有读取点；每个状态值必须有写入点**」
- ⚠️ **把 goink 当范本去抄它会用错药**：它的「状态维护三重保障」是可抄的，但它的审批 / 回滚落地面**恰恰是不可抄的反面教材**。

### 关键一问：对方有哪几条是「既设计了又真的实现了」的？

这一问决定借鉴清单长度。逐条核实后，**真实现且值得借鉴的有 9 条**（其余为占位 / 未接线 / 默认关闭）：

| # | 机制 | 档位 | 出处 |
|---|---|---|---|
| 1 | **工具注册表统一校验 + `DisallowUnknownFields()`** | 真实现 | `base.go:236-247` |
| 2 | **工具白名单双重校验**（OpenAI + Execute 各一次） | 真实现 | `base.go:210` / `base.go:232` |
| 3 | **写前重读对比（乐观锁）** | 真实现 | `rw_tools.go:179-181` |
| 4 | **`create_*` 循环 + 事务 + `{"ids":[],"count":N}`** | 真实现 | `character_tools.go:188,381` 等 10 处 |
| 5 | **搜索替换四层容错** | 部分实现 | `rw_tools.go:332-378` |
| 6 | **技能三态触发 + 目录只给 name+description** | 真实现 | `skill_catalog.go:26-34`、`store.go:104-112` |
| 7 | **Append-only + `is_current` 关系演化史** | 真实现 | `character/types.go:34-44` |
| 8 | **时间线「软锚点只排不滤」上下文构造** | 真实现 | `timeline/types.go:40-60` |
| 9 | **操作日志（GORM callback）+ turn 级整体回滚** | 真实现（但无执行入口） | `storage/operation_log.go:408` |

---

## §2 我方缺陷实证（本轮新发现，全部带文件:行）

### A. 🔴 P0：774 行叙事结构引擎零消费端

`src-tauri/crates/harness/src/narrative.rs`（774 行）定义了完整的强类型叙事模型 + 合规检查 + 调整建议：

| 类型 / API | 在 narrative.rs 之外的使用点 |
|---|---|
| `NarrativeStructure` | **0**（`dao/entities/repo/commands` 的 `NarrativeStructure*` 是**另起一套同名定义**） |
| `NarrativeArc` / `ConfluencePoint` / `Foreshadow` | **0** |
| `ChapterStructureInstruction` | **0** |
| `StructureComplianceReport` | **0** |
| `StructureAdjustmentSuggestion` | **0** |
| `ArcInstruction` / `ForeshadowInstruction` | **0** |
| `get_chapter_instructions()`（给某章生成约束） | **0** |
| `to_prompt_constraints()`（结构 → prompt 约束） | **0** |
| `should_setup_in_chapter()` / `should_payoff_in_chapter()` | **0** |
| `mark_payoff()` / `mark_abandoned()` / `advance_progress()` | **0** |

验证命令（可复现）：

```bash
grep -rn "\bChapterStructureInstruction\b" --include="*.rs" src-tauri/src src-tauri/crates \
  | grep -v "target/" | grep -v "harness/src/narrative.rs"   # → 零命中
```

**这正是「一名多义 / 同名多套值域」的应用实例**：`harness::narrative::NarrativeStructure`（强类型、无序列化落库）
与 `entities::narrative_structures::Model`（`structure` 单列 JSON）是**两套零交集的词汇表**。

**根因**：`PLAN-narrative-structure-enhancement.md`（2026-08-10）的 Phase 1 明确要求
「`rt-workflow` 仅依赖 `harness` 读取这些类型，不得绕过」—— 类型建成了，**但没有任何消费方被接上**。

### B. 🔴 P0：叙事结构表 / 命令 / 面板 / 工作流 —— 四方不通

```
路径 ①（手动 CRUD，通）
  前端 NarrativeStructurePanel.tsx
    → lib/narrativeStructure.ts（5 个 invoke）
      → commands/narrative.rs（201 行，5 个 #[tauri::command]）
        → dao/repo/narrative.rs
          → 表 narrative_structures（v126 真建；v220 已自曝为 no-op）

路径 ②（工作流运行时，通但另一套）
  lc-conceive 节点 LLM 输出 narrative_structure（JSON 文本）
    → 作为 prompt 变量 {narrative_structure} 注入下游
      → lc-structure-injector / checker / adapter 节点（全 LLM）

① 与 ② 之间：零交点
```

- `narrative_structures` 表的写入方只有 `commands/narrative.rs`；**工作流运行从不写表**
- 工作流运行从不读表（叙事结构来自 `lc-conceive` 的 LLM 输出文本）
- 后果：用户在面板里精心设计的弧线/伏笔，**对工作流执行无任何影响**；反之工作流产出的结构也不落到面板

**根因**：`PLAN-narrative-structure-enhancement.md` 的 Phase 4「新增 `narrative_structure` 数据表 + 实现结构的跨会话保存与恢复」
被标为 🟢 低优先级且最后实施 —— 表建了，**两端都没接**。

### C. 🔴 P0：content_media 的 7 个 KPI 全部无数据源

`config/opc/industries/content_media/runtime.yaml:64-85`：

```yaml
kpi_sources:
  content_count:    { type: constant, value: 0 }
  page_views:       { type: constant, value: 0 }
  conversion_rate:  { type: constant, value: 0 }
  content_engagement:{ type: constant, value: 0 }
  word_count:       { type: constant, value: 0 }   # ← 创作类
  completion_rate:  { type: constant, value: 0 }   # ← 创作类
  revision_rounds:  { type: constant, value: 0 }   # ← 创作类
```

注释明写「创作类 KPI（文学创作工作流专属）」，但**三个创作 KPI 的 source 也是 `constant: 0`**
⇒ 面板恒显示 0，与「已跑完 N 章」无关。

**对照**：goink 有 `internal/writing/store.go` 的 `WritingLog`（date / chapter_number / word_delta）
+ `DailyActivity`（按 date `GROUP BY SUM(word_delta)`）+ `WritingStats`（total_words / current_streak / longest_streak）
—— **`word_count`、`revision_rounds` 在 goink 是可直接算出来的量**，我方是零。

### D. 🟠 P1：`lc-chapter-structure.current_chapter` 是悬空变量路径

```
seed_content_media.rs:1097  ("chapter_structure", "lc-chapter-structure.current_chapter")
seed_content_media.rs:1141  ("chapter_structure", "lc-chapter-structure.current_chapter")
```

全仓（`src-tauri` + `src`，排除 target/.worktrees/backup）**仅这两处引用，零处产出**。

解析链（`condition_executor.rs:393-410`）：

```rust
if let Some(root) = context.variables.get(parts[0]) {   // "lc-chapter-structure" 命中
    let mut current = root.clone();
    for part in &parts[1..] {
        current = current.get(part)?.clone();           // get("current_chapter") → None ⇒ 整体提前 return None
    }
    return Some(current);
}
// fallback（把整串当变量名直查）—— 因 root 已命中，永远走不到
```

失败处理（`agent_executor.rs:766-772`）：**`tracing::debug!` 打日志后跳过该输入项** —— 「解析不到」不进 prompt。

`lc-chapter-structure` 节点的 prompt 要求输出 `{"1": {...}, "2": {...}}`（键为章节号，`seed_content_media.rs:950-963`），
**不含 `current_chapter` 字段**。

> **判定**：这是**悬空契约**。严格结论 = 「该 input_mapping 项在运行时恒不注入」；
> 「`{chapter_structure}` 因此为空」属**强推论**，需实测 dump prompt 确认（列入 §7 不确定项）。
> **危害等级不因不确定而降**：`lc-draft-chapter` 的 prompt 自带兜底
> 「**若约束为空，可自由推进剧情**」（`seed_content_media.rs:790`）⇒ 一旦失效是 **fail-open 静默退化**，
> 表现为「工作流照常跑完、照常出稿，但结构约束从未生效」，**无任何报错**。

**同型前科**：同引擎内已有两处注释记录同类故障 —— `agent_executor.rs:1772`
「下游 resolve_var_path 无法解析（表现为 c-scorer 的 chain_analysis 取不到 chain_nodes → no_data）」、
`agent_executor.rs:2184`「input_mapping 变量为 null / Rhai Variable not found」。

### E. 🟠 P1：前端 `NarrativeKpiMetrics` 是孤儿类型

`src/types/narrative.ts:176-185` 定义 `arcCompletion` / `foreshadowRecoveryRate` / `structureCompliance`，
**全前端零组件消费**（grep 仅命中定义处）。

而 `PLAN-narrative-structure-enhancement.md` §3.3.B 要求的正是这三个 KPI；
后端 `content_media/runtime.yaml` 的 `kpi_definitions` 里**也没有**这三个 ——
只有 `config/opc/industries/content_media/learning.yaml:22,80,114` 把 `structure_compliance` 用作**反思维度**（非 KPI）。

⇒ **三层不一致**：前端类型有 / 后端 KPI 无 / 反思维度有。

### F. 🟡 P2：prompt 装配无「稳定前缀 / 动态后缀」分层，KV cache 命中率天然低

我方每个 Agent 节点的 prompt 都是**完整独立字符串**（`seed_content_media.rs:755-1000` 十余段），
注入内容（persona / world_schema / narrative_structure / chapter）直接拼进正文。
节点之间**不共享稳定前缀**。

**对照**：goink 的 `SystemMessages` 显式分 5 段并标注稳定性
（`internal/agentcfg/system_messages.go:22-28`）：

```go
Identity string // AgentIdentity（system1 提示词）
Always   string // 常驻技能正文
Catalog  string // auto 模式技能目录
Profile  string // NovelProfile（设定+偏好，稳定）
State    string // NovelState（基础信息+goink.md，动态）
```

`novel_state.go:15-16` 的注释写明设计动机：「**稳定内容（设定/偏好）由 NovelProfile 注入，与 NovelState 分离以优化 KV cache**」。

### G. 🟡 P2：`v220_narrative_structure.rs` 是自曝的冗余 no-op

`src-tauri/crates/dao/src/migrations/v220_narrative_structure.rs:5-17` 的文档注释已自行澄清：
「本 migration 自称建表，实为冗余 no-op……DLL 是旧三列 schema 的残留草稿……勿在此追加任何 DDL」。
**文件保留有价值（migration 序列不能留空洞）**，但名字会持续误导后来者 —— 建议改名或在注册表加注。

---

## §3 逐维度对照表（十类骨架，未覆盖项也列出）

| 大类 | 维度 | goink | AxInvest（文学创作） | 判定 |
|---|---|---|---|---|
| ① 数据层 | 数据来源/fallback | SQLite 单库 + git 文件 + 本地 ONNX 嵌入 | 工作流变量 + DB | 平 |
| ① 数据层 | 持久化介质 | **10 张领域表**（角色/关系/地点/地点边/弧线/弧节点/时间线/章节计划/读者认知/偏好/设定/写作日志） | `narrative_structures` 1 张表（JSON 单列） | **goink 领先** |
| ① 数据层 | 一致性 | **无任何外键**；删除影响面检查**仅 3/10 表**（character/location/story_arc）；关系演进靠 append-only 手工保证 | 无级联（表本身弱） | 双方均弱 |
| ① 数据层 | 迁移/版本 | GORM AutoMigrate；**vec0 表迁移即 DROP 重建**（无 ALTER） | sea-orm migration（v126 真建 / v220 no-op） | 平 |
| ② 契约层 | DTO / schema | 结构体 tag 驱动，**JSON Schema 反射生成 + 内联 `$defs`**，`schema_test.go` 全量断言无 `$ref` | harness DTO 强类型但**零消费**；DB 层 JSON 单列无约束 | **goink 领先** |
| ② 契约层 | 字段命名/序列化 | `_chapter` / `_chapter_number` 后缀约定 + 业务章号 ≠ DB id，**显式写入系统提示** | 无同类约定 | **goink 领先** |
| ③ 分层与依赖 ★ | 依赖方向 | 40 包 / 157 边；fan-in 最高 `storage`(18, fan-out 0)；**无 depguard ⇒ 约定非机制** | cargo workspace 36 crate + L1 门禁脚本（`check-layer-discipline.mjs`） | **AxInvest 领先**（我方有真门禁） |
| ③ 分层与依赖 | 跨包环 | 不适用（Go 语言保证） | — | 不适用 |
| ④ 接口层 | IPC/命令 | Wails 绑定 + 32 个 MCP 工具 | Tauri 命令 + 工作流节点 | 平 |
| ④ 接口层 | **出口严不严** | **fail-open**（审批覆盖 2/19 + `Approver==nil` 放行 + 回滚静默不可用） | 工作流节点失败有 `continueOnFail`；**但结构约束失效是 fail-open**（§2.D） | **双方均病，类型不同** |
| ⑤ 业务/决策层 | 对象模型 | 角色关系**有向图 + 演化史**、地点**双图**（包含树 + 空间无向边）、**读者认知**（known/suspense/misconception）、时间线三槽位 | 弧线/交汇点/伏笔（**有交汇点概念，goink 无**）+ 合规评分 | **各有所长** |
| ⑤ 业务/决策层 | 公式链与量纲 | 无量化公式（LLM 主观） | 合规分 0-100（LLM 主观，无锚定） | 均弱 |
| ⑤ 业务/决策层 | 可解释性/溯源 | 关系留 `chapter_number`、时间线留 `source_chapter` / `source`(ai/user)、写作日志 | 无溯源字段 | **goink 领先** |
| ⑥ 时间语义 | 双时态 | **无**（全是当前态；关系史用 append-only 近似） | 无 | 均缺 |
| ⑥ 时间语义 | as-of / 时间旅行 | 有「按 turn 回滚」（引擎存在、入口缺失） | 无 | goink 设计领先、实现不通 |
| ⑦ 治理与门禁 | 四链 | ①规则文件 ✅ ②执行者 ✅ ③CI 真跑 ✅（7 条）④代码遵守 ⚠️（rules.md 4 条被违反；`check_omitempty` 链断） | ①规则文件 ✅(`AGENTS.md`) ②clippy/tsc/dprint ✅ ③CI ✅ | **平** |
| ⑧ 工程质量 | 测试质量 | e2e 12 文件；**但 `style` 包 33 个 Test 无一覆盖 `Extract`**（LLM 路径零测试），`pattern` 包有 httptest mock SSE 集成测试 | vitest + playwright | 平（goink 内部密度不均） |
| ⑨ 可观测性 | 执行轨迹/自检 | 操作日志表 + `turn_commits` + per-turn git commit | 工作流执行器日志 | **goink 设计领先** |
| ⑩ 安全 | 越权/密钥/失败模式 | `DisallowUnknownFields()` 防 LLM 写 PK ✅；正则白名单 + SafePath 双层沙箱 ✅；**技能正文零清洗 ❌** | 工作流变量隔离 | **各有所长** |

---

## §4 对方病历表（`机制 | 声称 | 代码事实 | 判定`）

| 机制 | 声称 | 代码事实 | 判定 |
|---|---|---|---|
| 审批覆盖范围 | `edit` 描述「**所有**修改会先提交用户审批」 | 仅 `edit` / `delete_record` 两处调用 `Approver`；17 个 create/update/upsert 零审批 | 部分实现 |
| 审批异常分支 | — | `Approver == nil` ⇒ **跳过审批直接执行**（rw_tools.go:102） | **fail-open** |
| 审批默认档位 | — | `config/store.go:18 default:manual`；空值→manual；`config_test.go:21` 断言 | 真实现（默认拦）✅ |
| 回滚能力 | `delete_record` 描述「误删可通过操作日志回滚」 | `RollbackBeforeTurn`/`RollbackTo` **零调用者、零测试**；钩子注册失败仅 `logger.Error` | 定义但无人调用 |
| 硬删 vs 软删 | — | 10 张表全部 `tc.DB.Delete(&rec)` **硬删**，无 `DeletedAt`/`Unscoped` | 未实现软删 |
| 删除影响面检查 | 描述称「删除前自动检查关联数据，存在关联时拒绝」 | **仅 3/10 表有检查**；7 张表零检查 | 部分实现 |
| 技能 sanitize | 名字暗示防 prompt 注入 | `sanitize.go:7-20` **只清洗文件名非法字符**，正文零清洗 | 空壳（安全语义不成立） |
| 远程技能信任链 | — | 无签名/无哈希/无 allowlist；`index.json` 远端可控，内容原样落盘并进 LLM 上下文 | 未实现 |
| 压缩失败处理 | — | `agent.go:214-216` 仅 `Warn`，**继续用超预算上下文调 LLM** | fail-open 降级 |
| `NovelProfile/State`「每轮注入」 | docstring「每轮对话开头注入」 | 构建点仅 3 处：`chat.go:104`（**仅 isNew 时**）、`compress.go:84`、`agent.go:101`；**turn 级从不重建** ⇒ 会话中途改的设定本会话不生效 | 部分实现（docstring 与代码冲突） |
| MaxTurns 上限 | `DESIGN.md:49`「默认 50」 | 实际 **100**，三处定义（`agent.go:119`/`:154`/`chat.go:170`）；文档过时 | 真实现（文档腐烂） |
| `Session.Summary` / `Message.TokenCount` | DESIGN 声称是摘要/token 列 | 定义存在，**全项目零写入** ⇒ 恒为空 / 恒 0 | 定义但无人调用（死字段） |
| `llm.CountMessagesTokens` | 文档多次引用为「token 计数入口」 | 零代码调用（实际用 `CountMessageTokens`） | 定义但无人调用 |
| `RunOptions.SubAgentVersion` | 设计文档称用于子代理压缩隔离 | `compress.go:182` 自增后仅写日志，**无任何读取方** | 空壳 |
| rules.md 自身合规 | 11 节设计准则 | **4 条被代码违反/部分违反**：`utils.go` 不存在（实则 `base.go`）、`AllowSet` 未被使用（自写 `toSet()`）、`delete_` 前缀被禁用却存在 `delete_record`、`ExposeToLLM` 无「默认」机制 | 文档漂移 |
| 工具数量 | README「**31 个**结构化工具」 | 实测 **32 个** | 文档与代码差 1 |
| `get_characters` 描述 | 「冷门角色用 search 按名搜索」 | **注册表中不存在 `search` 工具** | 死引用 |
| `edit` 描述 | 「先生成 **git diff** 提交用户审批」 | 后端只传 `original`/`modified` 字符串，**不产 git diff**，diff 由前端渲染 | 部分实现 |
| 子代理白名单 | review / memory 各有职责 | 两组白名单**逐字完全相同**（9 个只读工具），无差异化 | 部分实现 |

---

## §5 我方强于对方的地方（不妄自菲薄）

| # | 维度 | 我方事实 | 对方事实 |
|---|---|---|---|
| 1 | **分层护栏是真机制** | L1 门禁脚本 `scripts/check-layer-discipline.mjs` + 棘轮 + CI | `.golangci.yml` **无 depguard** ⇒ 分层纯约定 |
| 2 | **编排能力** | 13+ 节点 DAG：Loop（50 章上限）+ 条件路由 + 结构注入/校验/适配三节点 + Word 装配 | **无编排层**，纯对话式 ReAct |
| 3 | **叙事模型表达力** | 有**交汇点（ConfluencePoint）**概念 —— 多线索汇聚/冲突/转折；含 `structure_compliance` / `confluence_compliance` 维度评分 | 弧线 = 节点链；**无交汇点概念**，无合规评分 |
| 4 | **可视化** | `ArcTimeline.tsx`(242) + `ForeshadowGraph.tsx`(190) + `NarrativeStructurePanel.tsx`(458) 三组件 | 有前端图（G6 力导向），但**弧线关系靠自然语言关联、不需结构化边表** |
| 5 | **模板化与版本治理** | `TEMPLATE_VERSION` 递增 + 变量参数化（output_dir/document_title/file_format/chapter_separator） | 无模板版本概念 |
| 6 | **工程门禁密度** | `cargo clippy --workspace --all-targets -D warnings` + 全量 `cargo test` + tsc + dprint + oxlint + 11 语言 i18n 全 key 覆盖 | `golangci-lint` + ESLint + i18n 检查等 7 条（其中 1 条断链） |
| 7 | **多语言** | 11 语言全 key 覆盖（CI 强制） | 中文为主（README 双语） |

> ⚠️ **禁止用「我方也有 X」做伪对冲**（如用「我方有编排层」去抵消「我方无结构化创作状态」）。
> 上表的每一行都是**独立维度事实**，不构成对 §2 缺陷的抵消。

---

## §6 借鉴清单速览（详细版见 `PLAN-goink-borrowings.md`）

**性质图例**：🔴 我方自有缺陷修复 ｜ 🟡 借鉴对方机制 ｜ 🟢 元机制（工程纪律） ｜ ⛔ 反面教材

| ID | 性质 | 内容 | 对方是否真做到 |
|---|---|---|---|
| A1 | 🔴 | 774 行叙事引擎接进工作流（或明确标注为待接线） | — |
| A2 | 🔴 | 叙事结构落库：`lc-conceive` 输出经 harness 类型校验后写入 `narrative_structures` | — |
| A3 | 🔴 | 7 个 constant KPI 接真实数据源（可直算：work_count / revision_rounds） | 对方有 `writing_log` 可直算 ✅ |
| A4 | 🔴 | 修 `lc-chapter-structure.current_chapter` 悬空路径 | — |
| A5 | 🔴 | 前端 `NarrativeKpiMetrics` 三层对齐（类型 / KPI 定义 / 反思维度） | — |
| B1 | 🟡 | **结构化创作状态 10 表**（角色/关系有向图/地点双图/弧线/伏笔/读者认知/偏好/设定） | ✅ 真实现 |
| B2 | 🟡 | **关系 append-only + `is_current`** 演化史（替代自引用指针链） | ✅ 真实现 |
| B3 | 🟡 | **读者认知（known/suspense/misconception）** —— 控制信息释放与反转时机 | ✅ 真实现 |
| B4 | 🟡 | **时间线「软锚点只排不滤」+ 四段窗口组装** | ✅ 真实现 |
| B5 | 🟡 | **写完自动注入维护提醒（注入在工具返回值里）** | ✅ 真实现（`rw_tools.go:233`） |
| B6 | 🟡 | **自由文本 vs 枚举的判据**：是否驱动行为决定字段形态 | ✅ 真实现（含设计决策记录） |
| B7 | 🟡 | **Skill 三层覆盖 × 三态触发 + 热重载** | ✅ 真实现 |
| B8 | 🟡 | **文风蒸馏**：统计先行 → LLM 归纳（含六维拆解） | ✅ 真实现 |
| B9 | 🟡 | **写作方法论规则清单 87 条**（去 AI 味 27 / 潜台词 12 / 节奏 12 / 悬念 11 / 节拍 9 / 修改 14 等） | ✅ 领域知识资产 |
| B10 | 🟡 | **写前重读对比（乐观锁）** 防覆盖 | ✅ 真实现 |
| B11 | 🟡 | **prompt 分层（稳定前缀 / 动态后缀）** 提 KV cache 命中 | ✅ 真实现（注释显式说明动机） |
| B12 | 🟡 | **ErrKind 三级工具失败中断**（system 5 / args 5 / global 10）+ 死循环检测 | ✅ 真实现 |
| B13 | 🟡 | **版本化摘要压缩**（压缩不删旧消息、可回滚） | ✅ 真实现 |
| B14 | 🟡 | **`DisallowUnknownFields()` + Schema 反射生成 + 全量 schema 测试** | ✅ 真实现 |
| B15 | 🟡 | **本地语义检索链**（ONNX 中文嵌入 + sqlite-vec + MMR + 三层记忆块分块） | ✅ 真实现 |
| C1 | 🟢 | **「统计先行、LLM 后归纳」prompt 范式**（零 token 确定性特征 + 明示「此数值由代码算出」） | ✅ 真实现 |
| C2 | 🟢 | **递归压缩到预算 + 收敛停滞检测**长文本流水线 | ✅ 真实现 |
| C3 | 🟢 | **新节点上线必须带 mock LLM 集成测试**（对方 `style` 包零覆盖 vs `pattern` 包有集成测试） | 部分（反面驱动） |
| ⛔ D1 | ⛔ | **不要抄**：审批只在 2/19 工具、`Approver==nil` 即放行 | ❌ fail-open |
| ⛔ D2 | ⛔ | **不要抄**：技能正文零清洗、远程技能无签名 | ❌ 未实现 |
| ⛔ D3 | ⛔ | **不要抄**：压缩失败继续用超预算上下文 | ❌ fail-open |
| ⛔ D4 | ⛔ | **不要抄**：回滚引擎不接执行入口 | ❌ 写而不接线 |

> ⚠️ **「参考设计意图、不要参考实现」标记**：D1–D4 在源项目中为 **fail-open / 默认关闭 / 从未接线**。
> 但它们的价值**不在「抄」而在「镜子」** —— 它们精确映射出我方 §2.D 的同型风险：
> **约束失效不报错、静默退化成无约束运行**。据此反推的护栏规则见 PLAN §C4。

---

## §7 不确定项（未验证，不要当结论用）

1. **`{chapter_structure}` 的实际渲染结果**：已确认 `lc-chapter-structure.current_chapter` 解析恒为 `None`
   且该项不进 prompt；但 `prompt 正文里的 {chapter_structure}` 是**渲染成空串还是保留字面量**，
   取决于 `render_prompt` 对缺失键的处理，**本轮未读该实现**。⇒ 需实测 dump 一次 `lc-draft-chapter` 的最终 system_prompt。
2. **`lc-outline.remaining` / `lc-conceive.persona` 等其余 var_path 是否也失效**：
   取决于节点输出是否被包装（如 `{content: {...}}`）。`agent_executor.rs:1772/2184` 的注释提示这是**已知高发区**，
   但本轮未逐个核对 `lc-*` 全部 var_path —— 建议做一次**全工作流 var_path 产出-消费对照扫描**。
3. **`lc-chapter-structure` 节点输出的真实结构**：prompt 要求 `{"1": {...}}`，但 `outputMode: json` 的实际包装层未核实。
4. **goink 的 `search_story_memory` 在非 cgo 构建下的可见性**：白名单含该工具，非 cgo 构建时工具未注册 ⇒ `OpenAI` 侧静默跳过（`base.go:228-231`），行为未实测。
5. **goink 前端 diff/审批 UX**：`rw_tools.go:103-109` 的 payload 如何被前端消费（是否真展示 diff）本轮未读前端审批组件。
6. **goink 的 `operation_log` 是否覆盖 MCP 工具路径**：hook 是否挂在同一 `*gorm.DB` 实例上未确认。
7. **87 条写作方法论的「可直接落地性」**：本轮为文档级提炼，**未在任何模型上实测其效果**。落地前建议先做小样本 A/B。

---

## §8 一句话总结

> **goink 值得借鉴的不是它的代码，而是它把「小说创作状态」当第一类公民的那套数据结构与注入纪律；
> 它的审批、回滚、技能安全三块则是彻头彻尾的反面教材 —— 而这三块的病灶（约束失效不报错）恰好是我方
> 774 行叙事引擎与 `current_chapter` 悬空路径的同型病。本轮最大价值不是「抄到了什么」，而是
> 用对方的失败当镜子，照出我方「设计完好、接线断裂、失效静默」这条同一根因链。**

---

*报告完。全程零 git 命令；所有「对方声称」均引自其仓库源码注释 / DESIGN.md，冲突处已显式标注；
所有「我方事实」均带文件:行且可 grep 复现。*
