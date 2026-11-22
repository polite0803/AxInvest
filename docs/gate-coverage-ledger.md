# 门禁覆盖台账（gate-coverage-ledger）

> 状态：active ｜ 日期：2026-10-31 ｜ 对应版本：v3.0.0


> 来源：qa-verifier 2026-10-31 静态核实报告（三类未验覆盖缺口）+ team-lead 裁决。
> **冻结语义**：18 条清单冻结——不再新增条目（「做完台账就停，不要自行扩展第四类」）；
> 修复一条划掉一条（状态列注明修复方式与交付物），**只减不增**。
> 实跑清单与台账物理分节：台账条目只登记事实，实跑清单条目必须有人跑过命令才能定论。

## 未验台账（18 条，冻结）

| # | 缺口 | 类 | 精确位置 | 会被什么假绿掩盖 | 建议补法 | 优先级 | 状态 |
|---|---|---|---|---|---|---|---|
| 1 | 真 PG 上**全量 schema**（表集合/列类型/约束）无任何断言 | 1 | `ci.yml:689-695`、`ci.yml:736-739`、`ci.yml:783-786` | PG 三目标跑绿 ⇒ 误判 DDL 已验；`cargo test` 全绿也验不到 | 新写 PG 侧引擎建库测试（`PLAN-declarative-schema-sync.md` §十一·十一⑨） | P0 | 待办 |
| 2 | **ci.yml ↔ ci-check 同步护栏只守 7 个门** | 1 | `src/__tests__/ci.workflow.test.ts:80-218`（清单：domain-semantics / layer-discipline / ontology-consistency / i18n-scan / i18n-untranslated / i18n-key-parity / i18n-placeholder-usage） | 其余约 53 个 ci.yml 门单边改动无人发现；i18n ④⑤ **已实证过一次**（`ci-check.mjs:347-351` 记录在案） | 护栏改为从 ci.yml 自动提取清单比对，弃手写白名单 | P0 | **已修 2026-10-31**：`ci.workflow.test.ts` 追加自动提取护栏（`extractCiYmlCommands`/`extractCiCheckCommands`/`canon`/对称差集 + allowlist≤8 带理由 + 分母>40 + 双向负控，25 用例全绿） |
| 3 | `check-single-source-facts` 四类引用点只有 LOCATED 被真查 | 1 | `ci.yml:192-194` | 「0 腐烂」被读成「全都查过了」，NONLOC/EXTERNAL/BROKEN 无人看 | 把 NONLOC/EXTERNAL/BROKEN 计数纳入通过判据 | P1 | 待办 |
| 4 | 本地 ci:check 自陈 8 类 CI 步骤未覆盖 | 1 | `scripts/ci-check.mjs:14-23` | `ci:check` 全绿当验收证据 | 已进 SOP：Rust 走 `npm run test:rust`；build/contracts/e2e/audit/deny 手动补 | P0（已在 SOP） | 持续生效 |
| 5 | [历史·已修] schema-gen `run_check` 从不非零退出 | 1 | `ci.yml:570`，现已在 `ci.yml:577` 修为字节级比对 | 当时任意生成物漂移全绿 | 已修；保留为同类审查锚点 | P2 | 已修（历史锚点） |
| 6 | check-accent-foreground（主题前景色禁写死白）**本地跑 CI 不跑** | 2 | 仅本地：`ci-check.mjs:334-335`；ci.yml 命中 0 | 本地绿 ⇒ 误以为 CI 也验；违规可直合 master | ci.yml 补一行即可 | P2 | 待办（不对称已登记进 #2 护栏 allowlist，失效即红） |
| 7 | check-decision-validation-integrity（pick_id 不得悬空）**本地跑 CI 不跑** | 2 | 仅本地：`ci-check.mjs:487`；ci.yml 命中 0 | 同上；且依赖本机 PG 读数 | 补 CI 时需同时指明 PG 依赖 | P1 | 待办（同上） |
| 8 | check-replay-live-consistency（回放↔实盘 style 集合）**本地跑 CI 不跑** | 2 | 仅本地：`ci-check.mjs:481`；ci.yml 命中 0 | 同上 | 同上 | P1 | 待办（同上） |
| 9 | check-route-reachability（每条路由至少一入口）**孤儿** | 2 | 仅 `package.json:108-109`，ci.yml 与 ci-check 均 0 | 死路由无任何机器拦 | 二选一：接 CI，或删脚本并在 AGENTS.md 注明「手动按需」 | P2 | 待办 |
| 10 | check-upgrade-compat（上游 workaround 漂移哨兵）**孤儿** | 2 | package.json 无 script，全仓零调用 | 上游修复后 workaround 长期滞留 | 手工哨兵，建议季度提醒或接 CI | P3 | 待办 |
| 11 | type-consistency（check:types）**孤儿** | 2 | `package.json:87` | 前后端类型约定无机器校验 | 先评估是否已被 check-contracts / tsc 覆盖，否则删除 | P3 | 待办 |
| 12 | check-paired-fields（check:paired）**孤儿** | 2 | `package.json:88`（.ts，需 loader） | 分离字段对失配无人拦 | 技术栈不同可能是未接 CI 真因 | P3 | 待办 |
| 13 | check-serde-annotations（check:serde）**孤儿** | 2 | `package.json:89`（.ts） | Rust DTO serde 注解缺失 | 先核对与 `crates/harness/tests/serialization_contracts.rs` 重叠再定 | P3 | 待办 |
| 14 | check-ngram-consistency 无 PG ⇒ exit 0 并印「跳过不代表通过」 | 3 | `ci.yml:826-831` + `ci-check.mjs:629-636` | 无 PG 环境全绿，CJK ngram 两侧失配永不发现 | CI 已加 exit 1 前置守卫；**本地镜像无此守卫** | P1 | 待办 |
| 15 | pg_upsert_bare_column 缺前置表时 `return` ⇒ EXIT=0 | 3 | `ci.yml:761-771`（守卫 `:789-793`） | 曾长期「装了门禁从没见它红」 | 已用 grep 输出文本作守卫，但判据脆弱 | P1 | 待办（守卫在，判据待加固） |
| 16 | nextest 主测试步刻意不设 `AXAGENT_TEST_PG_URL` | 3 | `ci.yml:697-705` | PG 三目标该步恒跳过，绿 ≠ 验 | 设计合理；任何结论须注明「PG 侧另计」 | P2 | 设计如此（记录在案） |
| 17 | ci-check 探测不到 Rust ⇒ 整组 Rust 检查跳过（只打一行黄字） | 3 | `scripts/ci-check.mjs:43`、`:113` | 缺工具链时「全绿」实为「什么都没跑」 | hasRust=false 视为失败，或显式 `--skip-rust` 并计入报告 | P1 | **已修 2026-10-31**：skipped 记账 + 三分支结论（黄字「跳过 ≠ 通过」逐条列出）+ `--strict-skip`（qa-verifier 规格，6 场景实测） |
| 18 | ci-check 无 python ⇒ `check_i18n.py --strict` 整项略过 | 3 | `scripts/ci-check.mjs:357-364` | i18n key 对齐 / zh-CN 空值维度静默缺失 | 本机锁定 python3，或移植为 node | P2 | **已修 2026-10-31**：同 #17（`skipStep` 记账，仅前端在跑时记入） |

### 本会话（2026-10-31）新落地的互补门禁（不属于 18 条，另行登记）

| 门禁 | 盲区 | 文件 |
|---|---|---|
| JSX children 文本 i18n 扫描（CJK/ASCII 双向，锚定式） | 五道既有 i18n 门只看属性字面量；`>文本<` 51 条存量进棘轮基线 | `scripts/check-jsx-text-i18n.mjs` |
| 增量孤儿 i18n key（对 HEAD 的 R/M/D 旧内容减现存引用与动态前缀） | 删页面后 locale key 遗留，`check_i18n.py` 孤儿只报告不判失败（41% 孤儿恒绿） | `scripts/check-i18n-orphan-diff.mjs` |
| 组件中立性（跨 feature import 图） | feature 组件互引耦合，212 条存量棘轮基线 | `scripts/check-component-neutrality.mjs` |
| i18n 豁免预算（reason+until 到期日 + 三本账棘轮） | 77 个 i18n-exempt 文件整文件免检无期限；allowlist 14 死文件已清洗（433→419） | `scripts/check-i18n-exemption-budget.mjs` |

## 实跑清单（需运行才能定论）

| 序号 | 待实跑项 | 证据现状 | 一句修复候选 + 谁/何时/何命令 | 阻塞点 |
|---|---|---|---|---|
| 1 | 36 个未继承 `[lints]` 的 crate 是否真有 `module_name_repetitions` 违规 | 继承事实已验（34 = 4 个 `[lints.rust]` 内联 + 3 个空表/注释壳 + 27 个完全无段；全景 36 = 34 + 根包 `axagent` + `schema-gen`）；违规与否**未验** | be-dev 用 `cargo clippy -p axagent-cache --all-targets --all-features -- -D warnings`（依赖链最短）先探一个；证实后再决定台账行 6 降级还是划掉 | 需真实 clippy 一次（min 最便宜入口） |
| 2 | `sqlite-vec 0.1.8-alpha.1` 是否被 deny 规则覆盖 | 依赖版本 + 注释自陈（RUSTSEC 不为 Alpha 发公告）+ `deny.toml`/`audit.toml` 存在，**均已验**；覆盖与否**未验** | be-dev 跑 `cargo deny check licenses bans sources` | 需 cargo deny 一次 |
| 3 | `check:routes` 默认形态是否恒绿 | 静态已可判定：`check-route-reachability.mjs:17-18` 自陈「总是 exit 0」+ `:26` 仅 `STRICT` 下设 1 + `package.json:108` 不带 `--strict`；**但接 CI 前需实跑确认 `--strict` 形态真会红** | 谁要把它接 CI，谁先跑 `node scripts/check-route-reachability.mjs --strict` 确认会红 | 无人接 CI 前无实际暴露面 |
