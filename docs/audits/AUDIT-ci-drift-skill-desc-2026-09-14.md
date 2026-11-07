# 元设施维护：CI 门禁清单漂移修复 + 技能 description 拆分（2026-09-14）

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


> 两项任务同属「让工具说真话」这件事的两面：
> **门禁清单漂移** = 本地与 CI 各查一半（假安全感）；**description 膨胀** = 召回索引被无关内容稀释。
> 本轮附带查出并修复 4 个技能的 frontmatter 语法缺陷，以及我自己造出的 2 个工具缺陷。

## 0. 结论

| # | 任务 | 结果 |
|---|---|---|
| 1 | 修 CI 门禁清单漂移 | 本地镜像补 **3 条**；CI 补 **1 条**（含脚本改造 + 前置守卫）。双向差集归零 |
| 2 | 拆 `rust-consistency-audit` 的 description | **1625 → 923** 字符（-43%），7 个路由技能名 **0 丢失** |
| 3 | 附带：4 个技能 frontmatter YAML 解析失败 | 已修（根因：值区含半角「冒号 + 空格」） |
| 4 | 附带：`check-ngram-consistency.mjs` 的 `exit 2` 缺陷 | 已修（「配置缺失」改为诚实 SKIP） |

改动文件：

| 文件 | 改动 |
|---|---|
| `scripts/ci-check.mjs` | +3 条 step（前端段 2 条 / Rust 段 1 条） |
| `.github/workflows/ci.yml` | `test-unit` job +1 步（含前置守卫） |
| `scripts/check-ngram-consistency.mjs` | env 通道 + 配置缺失诚实 SKIP + 头部文档 |
| `~/.workbuddy/skills/rust-consistency-audit/SKILL.md` | description 拆分 + 维护规则 |
| `~/.workbuddy/skills/rust-consistency-audit/references/method-24-*.md` | 追 §8「两套门禁清单的漂移审计」 |
| 4 个技能 `SKILL.md` | frontmatter 值区 `: ` → ` - `，合并为单行 |

---

## 1. 任务一：CI 门禁清单漂移

### 1.1 实测（改动前，双向差集）

判据：抽出两侧的 `node scripts/*.mjs` 做**双向**差集（单向只能看到一半）。

| 方向 | 内容 | 后果 |
|---|---|---|
| **只在 CI 有** | `check-id-validation.mjs`（`ci.yml:164`）、`check-errorcode-alignment.mjs`（`:167`）、`check-rust-raw-map-err.mjs`（`:230`） | 本地 `ci-check` 一路绿 ⇒ **推送后才发现** |
| **只在本地有** | `check-ngram-consistency.mjs` | CI **从不校验**中文 ngram 两侧一致性 ⇒ 真盲区 |
| 假阳性 | `i18n-scan.mjs` | `check-hardcoded-i18n.sh:11` 是 `exec node scripts/i18n-scan.mjs "$@"` —— 同一门禁的两种调用方式 |

三条「只在 CI 有」的门禁**单独实跑全部 `EXIT=0`**（`check-id-validation` 报 38 条 WARN，**警告级不阻提交**；另两条各自 OK）。

### 1.2 修法 A：本地镜像补 3 条

放在「纯 Node 静态门禁」组（`ci-check.mjs` 的设计意图：秒级、不依赖 `npm ci`/构建产物 ⇒ 最快失败），
顺序与 `ci.yml` 不同但**命令行逐字同源**。`check-rust-raw-map-err.mjs` 放 Rust 段最前。

### 1.3 修法 B：CI 补 ngram —— 三件事，缺一不可

| 问题 | 若不处理的后果 | 修法 |
|---|---|---|
| `frontend-check` job 无 PG | 挂那儿只能**永久 SKIP** = 空转守卫 | 改挂 `test-unit`（唯一有 `pgvector` service 的 job） |
| CI 无 `~/.axagent/db_config.json` | 脚本连配置来源都没有 | 加 `AXAGENT_NGRAM_PG_URL` env 通道，保留本机回落 |
| 配置缺失时 `exit 2` | CI 里**恒红的噪音门** | 改为**诚实 SKIP**（与「无驱动」同类，都不是校验失败） |
| `pg` 装不上时脚本 SKIP + `exit 0` | 「装包失败」被静默转成「通过」= **用新盲区换旧盲区** | 加前置守卫：`[ ! -d …/pg ] && exit 1` |

CI 的库是**干净的**（从未跑 v227 迁移）⇒ 生成列 / GIN 索引不存在，那半检查恒红 ⇒ 用 `--only=consistency` 只验两侧函数一致。

### 1.4 负对照（缺一个都不算验证）

| 构造 | 结果 | 证明了什么 |
|---|---|---|
| env 传**错误口令** | `SKIP：…"postgres" Password 认证失败` | env **被采纳**（未采纳会走 `~/.axagent` 而 PASS） |
| 清空 `USERPROFILE`（无配置） | `SKIP` + **`exit 0`** | 「配置缺失」走 SKIP 而非 `exit 2` |
| env 传正确连接 → `axinvest` | **`PASS 34/34`** | env 成功分支成立 |
| env 传正确连接 → **`axagent`（0 表库）** | **`PASS 34/34`** | **CI 的干净库场景在本机可行** |

最后一条是关键：用一个 **0 表的库**精确模拟 CI 的干净库，是「在本机证明 CI 上会跑」的唯一办法。

### 1.5 ⚠ 验证边界（如实登记）

**本地 `ci-check` 没能端到端跑完** —— 卡在第 4 步（分层护栏），是**并发会话**的
`sim_hook.rs` ×3 / `rhai_pm.rs` ×1 超出棘轮基线所致，与本次改动无关。新步骤排在其后 ⇒ 跑不到。

替代证据（两件静态可判的事）：
- **执行序列**：`ci-check.mjs` 共 **22** 个 step；新增的是 **#9 `check-id-validation`**、**#10 `check-errorcode-alignment`**、**#17 `check-rust-raw-map-err`**
- **逐条实跑**：三条均 `EXIT=0`
- **YAML 结构**（真 `yaml` 解析）：`test-unit` 有 15 步，新步序 **11 > 10**（在 `npm ci` 之后）；`env` 连接串、`--only=consistency`、守卫子串、`NODE_PATH` 均在；`frontend-check` **未**挂 ngram
- **接线校验脚本** `output/verify-drift-fix.mjs` → `EXIT=0`

我没有为了让验证通过去改那些并发会话的文件，也没有下调棘轮基线。

---

## 2. 任务二：拆 `rust-consistency-audit` 的 description

### 2.1 实测：它是 41 个技能里最长的

| 指标 | 值 |
|---|---|
| `rust-consistency-audit` | **1625** 字符（全目录之最） |
| 中位数 | 251 |
| 第二长 | 661（`imagegen-frontend-web`） |

膨胀原因：**约 700 字符与正文 §①（26 行分流表）/ §②（专门技能分工表）逐条重复** ——
方法二十到二十六的完整触发词在 frontmatter 里抄了一份，正文里又有一份。

### 2.2 拆分方案与**守恒校验**

`description` 只留「召回必需」四件：主触发词 / 一句症状簇 / 写法 / 专门技能名。长尾交给 §①。

脚本内置五道自证（缺一不写盘）：

| 自证 | 结果 |
|---|---|
| 目标行确是那条 1625 字符的长行 | L3 ✓ |
| 新值不含半角「冒号 + 空格」（否则 frontmatter 解析失败） | ✓ |
| 新值不含换行（否则破坏单行 frontmatter） | ✓ |
| **7 个路由技能名 0 丢失** | ✓（拆 description 不能丢路由能力） |
| §① 分流表行数 > 0（长尾有归宿） | **26** ✓ |
| 写盘后用真 YAML 解析器回读 | `name` / `description` 长度 923 ✓ |

**结果 1625 → 923（-43%）**，且它现在排第 2（第一是修完语法后恢复完整长度的 `axinvest-db-type-audit` 1051）。

并补了**维护规则**（防重新膨胀）：新症状加进 §① 表 + 在 description 症状簇补一个短语，
**不要把整段判据搬回 frontmatter**。

---

## 3. 附带发现：4 个技能 frontmatter 有 YAML 语法缺陷（已修）

### 3.1 症状与根因

`yaml.parse` 抛 `Nested mappings are not allowed in compact mappings at line 2, column 14`。
根因：**description 值区含半角「冒号 + 空格」**（plain scalar 里不允许 `: `）。

在 WorkBuddy 里的**可见后果**（决定性证据）：`env-gated-test-rot-audit` 的 description
在技能列表里**被截断在第一行** —— 只到 `…because they are`，续行里的 `Triggers:` 全部丢失。

| 技能 | 受影响位置 |
|---|---|
| `antd-v6-message-migration` | `[antd: message]` |
| `axinvest-db-type-audit` | `Also encodes: the…`（description 跨 14 行） |
| `env-gated-test-rot-audit` | `chain: …`（注意不是 `Triggers:`） |
| `vitest-drive-letter-dual-instance` | 中文值区里的半角 `: ` |

### 3.2 修法选型（为什么不用块标量 / 引号）

块标量 `>-` 与引号都能让 `: ` 合法，但它们**依赖解析器支持对应特性** ——
而当前解析器**连标准缩进续行都取不全**，再引入更高级特性风险更大。
故采用**零特性**写法：合并成真单行 + 值区 `: ` → ` - `（无引号、无转义、无块标量）。

| 技能 | 值长度 | 解析 | 其余字段 |
|---|---|---|---|
| `antd-v6-message-migration` | 291 → 292 | FAIL → **OK** | 不变 |
| `axinvest-db-type-audit` | 1050 → 1051 | FAIL → **OK** | 不变 |
| `env-gated-test-rot-audit` | 690 → 692 | FAIL → **OK** | 不变 |
| `vitest-drive-letter-dual-instance` | 390 → 391 | FAIL → **OK** | 不变 |

脚本对每个目标先断言「改动前**确实**失败」（否则跳过 ⇒ 不修没坏的），改动后回读解析 + 校验其余字段不变。

---

## 4. 本轮我自造的两个工具缺陷（都是「审计工具会撒谎」）

| # | 形态 | 真相 | 修法 |
|---|---|---|---|
| 1 | `skill-fm-diagnose.mjs` 报「5 个技能全部 OK」 | `catch` 里**忘了把 `ok` 置 false** ⇒ 抛错也打印 OK。若不经交叉验证，我会得出「那 4 个技能没坏」的**反向结论** | 由 `skill-fm-conflict.mjs` 用**逐字相同的提取逻辑**复现 FAIL，矛盾才暴露 |
| 2 | `skill-desc-audit.mjs` 把 `grill-me-ytang__skillhub` 判 FAIL | 该技能用 `slug` / `displayName`，**没有 `name` 字段** ⇒ 判据 `!!parsed.name` 过宽，把**格式变体**当缺陷 | 登记为判据缺陷：须把「解析失败」与「解析成功但字段名不同」**分开报** |

另有一个差集判据的假阳性：`ci-check.mjs` **文件头用法注释**里写着 `node scripts/ci-check.mjs`，
被抽取器当成「只在本地存在的命令」⇒ 改为**按行剥注释**（`//` `*` `#` `/*` 开头）后再抽。

---

## 5. 记忆与技能沉淀

新增判据（`mem.mjs add`，只写判据层、**不占注入层预算**）：

| 编号 | 组 | 判据 |
|---|---|---|
| **#148** | K | frontmatter / 单行配置的**值区禁含半角「冒号 + 空格」**（YAML plain scalar 不允许）；症状是「**值被截断**」而非报错；审计须把「解析失败」与「解析成功但字段名不同」分开报 |
| **#149** | A | 往 CI 补门禁步骤**三问**：① 挂到**有真依赖**的 job（需要 PG 就挂有 PG service 的）② 脚本有 SKIP 分支 ⇒ **必须补前置守卫**（否则装包失败被静默转成通过）③ 脚本读开发机专有配置 ⇒ 必须加 env 通道，且「配置缺失」要**诚实 SKIP** 而非非 0 退出 |
| **#150** | K | 门禁清单漂移判据必须**双向**差集（本地缺 CI 有 / CI 有本地缺 —— 单向只看得到一半）；三个假阳性须先剥：`.sh` 包装调用、文件头用法注释自引用、聚合脚本的 `--only=` 过滤 |

技能沉淀：`rust-consistency-audit` 追 **§8「两套门禁清单的漂移审计」**（含双向判据代码、三个假阳性表、
补 CI 两条硬约束、env 通道改造、**负对照三连**、以及「端到端跑不通时用执行序列 + 逐条实跑」）。

---

## 6. 备份与回滚

| 备份 | 内容 |
|---|---|
| `output/backup-2026-09-14d/ci-check.mjs.pre-drift` | 本地镜像（改动前） |
| `output/backup-2026-09-14d/ci.yml.pre-drift` | CI 工作流（改动前） |
| `output/backup-2026-09-14d/check-ngram-consistency.mjs.pre-drift` | ngram 脚本（改动前） |
| `output/backup-2026-09-14d/rust-consistency-audit.SKILL.md.pre-desc` | 技能（拆 description 前） |
| `output/backup-2026-09-14d/skills-frontmatter/*.before` | 4 个技能（修 frontmatter 前） |

**备份自证**（`#` 形态：备份必须证明是「改造前」）：三份 `.pre-drift` 对本次所有新增符号
（`AXAGENT_NGRAM_PG_URL` / `resolvePgConfig` / `check-id-validation` / `check-errorcode-alignment` /
`check-rust-raw-map-err` / `ngram`）命中数**均为 0**。

证据脚本（可复跑）：

```
node output/verify-drift-fix.mjs        # 备份自证 + 双向差集 + YAML 结构 + 语法/行尾 + 执行序列 + 逐条实跑
node output/drift-verify.mjs            # 改动前的差异清单（对照用）
node output/ngram-env-probe.mjs axagent # 用 0 表库验证 env 通道成功分支
node output/skill-desc-audit.mjs        # 41 个技能 description 长度 + frontmatter 解析
```
