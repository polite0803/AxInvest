#!/usr/bin/env node
// scripts/check-capability-register.mjs
// 能力接缝登记表对账门禁（第五道门，2026-11-01，PLAN §0.12.4 裁定「建」/ 任务 #38）。
//
// 建门理由：`docs/plans/PLAN-shell-plus-plugins.md` §0.7「提供方接缝（L2）登记表」是 P5/P6
//   全部判据的分母，而它此前**只靠人工复扫** —— §0.12.4 那轮自查实测抓出
//   **15 处数字漂移 + 2 套并行口径**（1534/1541、工作流 124/83、股票 304/306、前端 60 757/84 873），
//   证明「数字漂了不会有人报警」不是假设。本门复跑两份 audit 的口径与 §0.7.1 声明的扫描面，
//   对机器镜像 `scripts/capability-register.json`（PLAN §0.7 的逐字镜像）做**逐行对账**；
//   读数变化必须显式改表（同批改 PLAN），否则红。
//
// 判据分档：
//   · hard:true 锚点/登记项不一致 ⇒ 红（退出码 1）；
//   · hard:false（register_commands.rs 行数、K1 外部实现者 232、K2 trait 面 51）⇒ 印出但不决定退出码；
//   · entries[].check="manual" ⇒ 不判红，**也不算通过** —— 计入「未机器复检」摘要并逐条印出。
//   ⚠ 例外（2026-11-02 收紧）：**锚点自相矛盾一律硬失败，不看 anchor.hard** —— 那是「表自身散架」，
//     与分组计数不符同档；否则 hard:false 的行可以自说自话（假绿洞①，见 checkAnchorConsistency）。
//   镜像表本身是唯一权威：**无增量基线、无 --update-* 洗白路径**（存在洗白路径 = 漂移可以再被
//   「合法化」，本门失去意义）；--dump-readings 只读调试，不落任何盘。
//
// 数据文件格式（capability-register.json）：note / caliber / sources / countRule / anchors / pipeline / entries。
//   anchors[].planOld[] = 旧读数白名单 `{ number, note, alsoAt? }`；`alsoAt[]` = 该旧读数**另外**允许出现的
//   PLAN 区段 `{ startLine, endLine, mustContain }`（可选，缺省=零放宽；语义与腐烂判据见下方「planOld[].alsoAt」段）。
//   entries[].planAnchorText / planAnchorTextRange = 该登记项 planLine（含区间端点）**登记当时的整行逐字**，
//   只作 --repin-plan 的定位键，**不参与判定档**（判据仍然只认行号 + token ⇒ 加它不会放松任何红）。
//   entries[].evidence[].anchorText = 该代码证据锚点**登记当时的整行逐字**（口径同上，仅折叠行内连续空白），
//   只作 --repin-code 的定位键，同样**不参与判定档**（判据仍然只认「第 line 行含 token」）。
//   anchors[].check ∈ registryCommandCount registryModuleCount registryUnassigned registryFileLineCount
//     registryBoundaryCount registryDomainCount frontendVolume frontendDirVolume l1ForeignBlockers
//     k1RsFileCount k1PubTraitCount k1DeclaredExternalImplTraitCount remoteSeamRegistryCount registerGroupSum manual
//     k2KeyedHits commandNameSetCount agentCommandFaceCount      ← #57 / #58 / #306（声明清单见 ANCHOR_CHECK_KINDS_DECLARED）
//   entries[].check ∈ implementorCount seamExists evidenceOnly manual。
//   ⚠ 为什么登记项的**身份键是 seamId / trait 名而不含行号**，行号只出现在 evidence 锚点里：
//   `.i18n-allowlist.json` 拿行号当键、随编辑整体腐烂是前车之鉴（见 check-i18n-exemption-budget.mjs 头注）。
//   证据锚点的行号**漂移本身就是要抓的缺陷**（登记与盘上脱节），所以它必须判红，而不是被当键使用。
//
// 强制复用：`scripts/lib/command-domains.mjs` 的 REG_PATH / parseRegistry / domainOf / unassigned
//   （AGENTS.md 禁区 12 —— 注册表解析规则全仓唯一，禁止另写第二套正则）。
//   边界口径的 DOMAINS（cmdRe/feRe/useRe/ownedRe）**逐字抄自** `scripts/audit-l1-blockers.mjs:64-79`
//   —— 该审计脚本不导出符号且禁改，抄录是本门与其唯一合法的同步点；改那边正则必须同改这里。
//
// 扫描面（按 sources.scanSurface 显式选边；**一次目录遍历 + 内容按需缓存**，否则共享目录上不可用）：
//   · K1/实现者计数：8 个 workspace 根的全部 .rs，忽略路径段 target / node_modules / gen / tests；
//   · 命令/域/前端/L1 读数：audit-l1-blockers.mjs 的忽略集（target node_modules output .git .tmp
//     dist gen build coverage .workbuddy examples）。两份忽略集**不同**（tests 只属 K1 侧，
//     examples/build 等只属 audit 侧），故目录树只走一遍（仅剪体积目录），按路径段分类选边。
//
// K1 侧的三档读数（#61 定案，全部出自 `collectReadings` 的**同一趟**扫描，禁另起一遍遍历）：
//   · `k1.pubTraits` = 扫描面内 `pub trait X` 的**唯一名**数（锚点 `k1PubTraitCount`）。
//   · `k1.externalImplTraits` = A 档：非 harness、非测试替身文件里的 `impl … for …` 目标名集合，
//     **不要求**该名在扫描面有声明面 ⇒ 逐条实现者数（§0.7.3–0.7.7）用它，改这一档会打穿实现者对账。
//   · `k1.declaredExternalTraits` = 定案档 `k1DeclaredExternalImplTraitCount` = A 档站点面 ∩ 声明面，
//     **两趟**语义（交并在遍历收齐之后做，见 `declaredExternalTraitNames`）⇒ 与
//     `scripts/audit-capability-inventory.mjs` 的 `scanK1Traits().declaredExternalCount` 同一集合。
//   ⚠ 两档**不可互换**：差集就是「impl 了但扫描面内没有 `pub trait` 声明」的名字（std/派生宏/外部 crate 名）。
//   ⚠ 单趟写法（就地查声明集）不是第三档口径，是 #61 归因的**缺陷**（当前树恒少 22 条），自检 ㉖ 钉住。
//
// 旧读数（planOld：1534、124）的合法出现位置 —— **显式判定规则（非隐性约定）**：
//   定位 §0.12.4 标题行（/^#{1,4}…0\.12\.4/）到下一个同级或更高一级标题之间的**区段**，
//   且该区段文本含「漂移」字样；旧读数的每次数字命中必须落在该区段内（数字按「非行号引用」
//   边界匹配：前后不接数字，左侧不接 `: . / #`，以免 `PLAN:124` 这类行号引用误判）。
//   区段外的任何出现 ⇒ 红。这就是「漂移列举段」的机械定义，勿在别处再发明第二套。
//
// planOld[].alsoAt（任务 #65，2026-11-02）：上一条规则的**已知缺陷是「只能登记一个区段」**，不是 PLAN 写错。
//   PLAN 里对 **HEAD 提交态**的事实陈述（如 §15.1「HEAD 产物 = 1540 条、HEAD 源码按算法应生成 1541」）
//   必然重述旧读数，而它是「干净检出必红」结论的**可复算证据**，删掉等于毁证据 ⇒ 给这类条目开一个
//   **白名单区段数组** `alsoAt: [{ startLine, endLine, mustContain }]`：
//     · 判据只加不改：出现行落在 §0.12.4 漂移段 **或** 任一**健康** alsoAt 段中**被自证锚定的那条出现行** ⇒ 合法；
//       段外照旧红。⚠ `#138` 起放行按**出现行**，不按整个区间 —— 见下一条。
//     · **健康 = 三条同时成立**：① 区间合法且带 key；② **段内仍有旧读数**（数字边界形态）——
//       否则窗口已漂到与断言无关处，仅凭「token 还在段里」判健康就是一个永远放行的空洞；
//       ③ 至少一个出现行的 **±`ALSOAT_ANCHOR_RADIUS`(=3) 行**内命中 mustContain token。
//       放行集 = 这些 `anchored` 出现行；落在同区间但离 token 超过半径的行 ⇒ 判红（窗口不得吞掉
//       无关同数出现）。任一 token 不命中 / 出现归不了因 ⇒ 报 `[区段腐烂 ALSOAT-STALE]`
//       （与「旧读数出段」是两回事：前者说行号锚已不可信，后者说旧数用错了地方），
//       **且该段内的出现仍判红** —— 腐烂的重锚不得默默放行旧数。
//     · 不登记 alsoAt ⇒ 行为与旧版**逐字一致**（零放宽）；空数组同样零放宽。
//     · `alsoAt[].mustContain` 里的**纯数字 token** 一并纳入锚点内部自洽检查（见 checkAnchorConsistency
//       ⑥ 档）：alsoAt 不是藏数口子 —— 允许的数字只有「本锚点 value/value2」「hard:true 且机器复检的
//       兄弟锚点数」「**本条 planOld 自己声明的旧读数**」三档；非纯数字 token（`"HEAD 产物"`、`"1540 条"`）
//       按既有 ③ 档惯例不做数字比对。**重锚**由 `--repin-plan` 负责（同口径：唯一定位才回写，
//       零命中 / 歧义 / 缺 token 一律拒写），不许留一个不能重锚的钉（#48 修的就是这个病）。
//
// planMustAppear 的命中口径（2026-11-02 收紧，见 planNumberHits）：**数字边界匹配**，不是裸子串 ——
//   千位空格写法算命中（`84 873`），日期/时刻串整串涂白，前后紧邻数字/小数点不算，左侧紧邻
//   `: / #` 的「行号引用 / 斜杠配对旧数」不算。现盘 15 个 planMustAppear 锚点在新口径下仍全命中。
//
// 锚点内部自洽（2026-11-02 新增硬判据，见 checkAnchorConsistency）：同一锚点的 `value` /
//   `planNumber` / `planPinned[].mustContain`（纯数字 token）必须指同一个数，否则**一律硬失败**
//   （不看 anchor.hard）。补的是旧门的假绿洞：只改 `value` 迁就盘上、PLAN 原文与 planNumber/钉句
//   留旧数，旧门三档判据彼此不比，能一路判绿。
//   ⚠ ②档「跨锚点宽容」的来源**收窄为且仅为** `hard:true` 且被机器复检（check ≠ "manual"）的锚点
//   登记数（value/value2）。理由见 checkAnchorConsistency 头注：软锚点改值永不判红，收它们的数字
//   等于给「把旧数藏进软锚点」留了洗白钉句的后门（假绿洞②，2026-11-11 审查员裁定阻断 1）。
//
// 自检覆盖度（2026-11-02 新增，见 ANCHOR_CHECK_NEGATIVE_CASES）：镜像表用到的**每一类** anchor
//   check 都必须有「登记≠实测 ⇒ 必被抓出」的负控 + 「登记=实测 ⇒ 不假阳性」的正控（全走合成读数，
//   零盘扫描）。--selftest 每次印「已覆盖 check 种类 N / 表内实际使用 M」，出现空洞即自检红。
//
// 前端反向阻塞复算（任务 #207，2026-11-06）：本门**不重写第二套扫描器** —— PLAN §125.7 加锚次序
//   ①「先让复算器可机械复现」的落地形态 = 判定跑里子进程调 `scripts/audit-l1-blockers.mjs --fe-json`
//   （复算器 = 权威脚本 §3 自身，其 feRe 判据由该脚本 --selftest 锁定），结果只进「只报告」档。
//   锚点 `feForeignReverse` 本轮按 **check:"manual" / hard:false** 的软锚点登记（先例 `k2TraitFaces`），
//   复算不符只印 ⚠、不决定退出码；升 hard 的前置条件逐条写在该锚点 note 里（次序③，且反向必先假红）。
//   子进程失败/输出不可解析 ⇒ 印「复算未出数 —— 跳过 ≠ 通过」，同样不判红、更不许借缺数改登记值。
//
// 用法：
//   node scripts/check-capability-register.mjs                  # 门禁：复算全部实测读数并与镜像表逐行对账
//   node scripts/check-capability-register.mjs --selftest       # 内建正负对照（纯函数、零盘扫描、先于任何 IO 短路）
//   node scripts/check-capability-register.mjs --dump-readings  # 只读调试：打印实测读数（不写盘）
//   node scripts/check-capability-register.mjs --repin-plan     # 维护模式：按定位键把 PLAN 行号引用重锚到当前文本（写盘前先备份）
//   node scripts/check-capability-register.mjs --repin-code     # 维护模式：按定位键把代码证据锚点 evidence[].line 重锚到当前文本（默认 dry-run，--apply 才写盘）
//   node scripts/check-capability-register.mjs --repin-code --apply  # 同上，真正回写（备份 → 定点补丁 → 从盘上复检 → 不过即回滚）
//
// ── --repin-plan 的由来与判据（任务 #48，2026-11-02）─────────────────────
// 本门用**绝对行号**钉 PLAN（`anchors[].planPinned[].line` + `mustContain`、`entries[].planLine` /
// `planLineRange`、`anchors[].planOld[].alsoAt[].startLine/endLine` —— #65 起也算 PLAN 行号引用）。
// 已实测的代价：主代理把 §0.7.6 从 5 行改成 4 行 ⇒ 全文少 1 行 ⇒ 8 处钉句**同时假红**
// （352/515/500/939/508/507/502/320），补回同数行才转绿 ⇒ 门禁事实上给一个**活文档**施加了一条
// 没写在任何地方的约束「编辑必须行数中性」。假红的代价是训练人去忽略红，所以把它变成一条**可执行的工序**：
// 改 PLAN ⇒ 普通运行红 ⇒ 跑 `--repin-plan` ⇒ 表回写 ⇒ 再跑普通运行。
//
// 判据（三档，全部 fail-closed）：
//   ① **定位键唯一才回写**：钉句按 `mustContain` 全部 token 同行命中；指针按 `planAnchorText`
//      （登记时的整行逐字，仅折叠行内连续空白）全等。命中 1 行 ⇒ 回写该行号；命中 0 或多行 ⇒
//      **整次拒绝写盘**并逐条报告（不许猜行、不许插值、不许「取最近的一个」）。
//      ⚠ **任务 #65 补：`planOld[].alsoAt[]` 区段同样在本模式覆盖范围内**（否则又造一个不能重锚的钉，
//      正是上面这段讲的病）。区段的定位键 = `mustContain` 的**最小包络窗口**（窗口内含全部 token，
//      且首行、末行各携带一个「窗口内别处没有」的 token ⇒ 按包含关系不可再缩）：命中 1 个 ⇒ 成对回写
//      startLine/endLine；0 个 ⇒ 零命中拒写；≥2 个 ⇒ 歧义拒写并列出全部候选（修法同 planPinned：
//      收紧 mustContain，补一条只有该段才有的句子）。
//      ⚠ 为什么**不**用「保持段高」：位移之后段高是**不可知**的冗余（登记的那几行此刻坐着上一段的
//      尾巴），而 token 仍在盘上。实测现盘 §15.1 把 `1540 / 1541 / HEAD` 挤在**同一行** ⇒ 「同高且含全部
//      token」的候选恒有 段高+1 = 8 个（探针 `.tmp/res65-alsoAt-probe.mjs`，+5/+9/+40/−9/−40 五个位移
//      全部歧义）⇒ 每次位移都拒写，区段就成了不能重锚的钉。改成按包络重划边界后，重锚只会把段**收紧**
//      到 token 所在的最小范围（判据只朝更严的方向动，`mustContain` 一字未动），并逐条打印段高变化。
//   ② **红线：绝不用模糊匹配代替行号锚点**。判定跑（`checkPlan`）仍按「第 N 行是否含该句」逐行核 ——
//      句子漂到别的行、或整句被改写，仍是红。重锚只改**表里的数字**，不改判据形态。
//      这条由负控㉑g 机械锁定：把钉句指到「同一句话出现的另一行」必须照样判红。
//   ③ **缺定位键即拒绝**：钉句无 token、登记项无 `planAnchorText` ⇒ 判 blocker（逼着「新增行号引用
//      必须同批带定位键」，否则重锚能力会随表增长而静默腐烂）。
//
// 写盘工序：先把整份表**逐字节备份**到 `.tmp/capability-register.<UTC 时间戳>.bak.json`，再按定点
// 文本补丁回写（不整体重新序列化 ⇒ 未涉及的字节逐字不动，与并行编辑者的表不互相打散格式），
// 回写后**从盘上重读**并复跑 `checkSelfConsistency` + `checkAnchorConsistency` + `checkPlan`
// 作为复检；复检不过 ⇒ 用备份**回滚**并以非零退出。全程逐条打印「哪个引用 · 旧行号 → 新行号」，
// 未变动的引用给计数、覆盖不到的引用（`caliber.basisLines` / `pipeline.lines` 是纯文档指针，
// 无定位键 ⇒ 本模式不重锚）单独点名，不装作覆盖到了。
//
// ⚠ 本模式**不接进 CI**（`ci-check.mjs` / `ci.yml` 都只有 `--selftest` + 判定两步）：
//   门禁的意义是「漂了就红、由人确认后再改表」，把自动回写挂进 CI 等于让漂移自我合法化。
//
// ── --repin-code 的由来与判据（任务 #55，2026-11-02）─────────────────────
// 本门还有一族**代码**行号锚点：`entries[].evidence[]` = `{ ref, file, line, token, endLine? }`，
// 判定档的判据是「该文件的第 `line` 行必须含 `token`」（见 adjudicateEvidence）。它和 PLAN 行号锚
// 同一种病，而且更常犯：**多会话并行改同一个 Rust 文件**时，函数只要整体往下位移，这条门就成批假红。
// 实测触发样本（2026-11-02）：并发编辑 `axagent-plugins/crates/plugins/src/worker.rs` 与 `manager.rs`
// ⇒ `workflow.reflector`（worker.rs:1527）、`workflow.evolver`（worker.rs:1551 与 :1714）、
// `dojo.extension`（manager.rs:654）三条同时红，红的却是「行号不在了」而不是「语义变了」。
//
// 与 --repin-plan 的分工（勿混）：--repin-plan 只重锚 PLAN 行号引用（`planPinned` / `planLine` /
//   `planLineRange` / `planOld[].alsoAt[]` 区段），--repin-code 只重锚 evidence 行号；两者都不碰判据形态。runRepin 结尾那句
//   「本模式只重锚 PLAN 行号，不碰 evidence」依然成立 —— evidence 的位移由**本模式**处理，
//   语义变化（零命中）仍旧两边都不许改写、判定档照旧红。
//
// 判据（三档，全部 fail-closed；与 --repin-plan 同形，但**定位键换成 evidence 自己的键**）：
//   ① **定位键唯一才回写**：新增可选字段 `entries[].evidence[].anchorText` = 该 evidence 登记当时的
//      **整行逐字**（仅折叠行内连续空白，口径同 `normPlanLine`）。重锚时在该 evidence 的**原文件**内
//      按「整行归一后全等」检索：命中 1 行 ⇒ 把 `line` 改成新行号（并同步 `ref` 末尾的 `:行号`）；
//      命中 0 行 ⇒ 该行已被改写/删除 = **语义变化不是位移** ⇒ 拒写并逐条点名；
//      命中多行 ⇒ 歧义 ⇒ **整次运行拒绝写盘**并逐条打印全部候选行号（不许挑一个、不许插值）。
//      ⚠ 为什么不能只拿 `token` 当键：现盘 `workflow.evolver` 两条 evidence 用**同一个 token**
//      （`remote_method_not_supported`）指向同文件的 :1551 与 :1714，按 token 全文检索**必然多命中** ⇒
//      它是判据（"第 N 行含该串"），不是身份键。token 命中情况在报告里**只作诊断信息打印**，不参与放行。
//   ② **红线：绝不放宽判定档**。`adjudicateEvidence` 一行没改 —— 本模式只改**表里的数字**，
//      「第 N 行含 token」的逐行核法照旧；行号再指错一行仍判红（负控㉒b2 / ㉒h 锁这条）。
//   ③ **缺定位键即拒绝**：evidence 需要回写（判定档已红）但没有 `anchorText` ⇒ 落「缺定位键」档，
//      **不退回用 token 猜**，拒写并点名（报告里给出该 token 当前在文件内的命中行号，便于人工补键）。
//      行号仍与盘上相符的 evidence 不需要回写 ⇒ 不计缺键（否则现盘 66 条全无 anchorText，本模式永远
//      不可用）；缺键只在「真要改这一行」时才咬人。
//   ⚠ 不重锚的 evidence（逐条点名，不装作覆盖到了）：**文件级**（无 `line`）、**区间锚点**（带 `endLine`，
//   两端位移量要人判）、**无 token**（判定档本就不检查该行 ⇒ 无键可定位）。
//
// 写盘工序（与 --repin-plan 同，但**默认 dry-run**）：不带 `--apply` 只打印「旧→新」计划与统计，
//   一个字都不写；带 `--apply` 才逐字节备份到 `.tmp/capability-register.<UTC 时间戳>.bak.json`，
//   再走**定点文本补丁**（不整体 `JSON.stringify` 重写 ⇒ 未涉及的字节逐字不动，另有会话正在编辑同一张表，
//   整体重排会互相打散），补丁完成后先比对**内存版 vs 文本版**（不一致即拒写），落盘后**从盘上重读**并复检
//   （除行号外一字未动 + 复跑 computeCodeRepin 必须归零），不过 ⇒ 用备份回滚并以非零退出。
//
// ⚠ 本模式同样**不接进 CI**：理由与上面那条逐字相同 —— 自动回写挂进 CI = 让漂移自我合法化。
//   本模式的存在也不改判定档：普通运行仍照旧红（验收时须同时看到「--repin-code 默认档不写盘」
//   与「判定档仍 EXIT=1」两件事）。
//
// 退出码：0 = hard:true 判据全部一致（或 selftest 全过 / 重锚写盘并复检通过 / 无需重锚 / --repin-code 默认 dry-run 无阻断）；
//   1 = 任一 hard:true 不一致 / 镜像表缺失 / selftest 失败 / 重锚被歧义或零命中或缺定位键阻断（未写盘）/ 复检不过已回滚。
//   hard:false 与 manual 一律不影响退出码（但必须印出来 —— 「未机器复检」不是「通过」）。
//
// main guard（2026-11-11 阻断 2）：本模块的判据**全是 export 的纯函数**，主流程收在 `main()` 里，
//   只在「本文件被直接执行」时运行（`import.meta.url === pathToFileURL(process.argv[1]).href`）。
//   此前顶层即扫描 + `process.exit` ⇒ 任何 `import` 本模块复算纯函数的行为（审查 / 自检 / 纸面实证脚本）
//   都会被劫持跑一次全仓遍历并强制结束进程（实测被劫持跑过 33.3s 全门）。现被 import 时**零副作用、
//   绝不调用 process.exit**；三条命令行（判定 / --selftest / --dump-readings）行为与退出码逐字不变。
//
// 遍历抖动（2026-11-11，同阻断 2 一并落实）：`k1RsFileCount` 是 hard:true，而并发会话正在增删 `.rs`
//   ⇒ 同一趟遍历的头尾可能数到不同文件清单，门禁会因**与登记无关的盘上并发**随机判红/随机判绿。
//   规则：判定跑完后按**同一枚举口径**重新统计一次扫描面文件清单数，与本趟开始时不一致 ⇒ 印
//   「⚠ 遍历期间盘上文件数变化（X→Y），本轮不判 —— 跳过 ≠ 通过」并**重试一次**；重试后仍不收敛
//   ⇒ 按本仓既有门的 skip 语义处理（印警告 + 计入 skipped 摘要），**该轮不因此判红**，exit 码由
//   其余硬失败决定。理由：抖动不是「登记与盘上不符」，判它红是噪声；但也不许静默吞掉 ——
//   「本轮不判」必须显式可见（跳过 ≠ 通过）。代价上界：稳定路径只多一次**纯目录枚举**（不读文件内容），
//   抖动路径最多再跑一轮复算；`--selftest` 零盘扫描，不受影响。

import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { REG_PATH, parseRegistry, domainOf, unassigned, scanAgentCommandFace, AC_NO_DOMAIN_BUCKET } from "./lib/command-domains.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const DUMP = args.includes("--dump-readings");
const REPIN = args.includes("--repin-plan");
const REPIN_CODE = args.includes("--repin-code");
const APPLY = args.includes("--apply");

const REGISTER_PATH = join(ROOT, "scripts", "capability-register.json");

// ── 口径常量（与数据文件/审计脚本的同步点都在注释里钉死）────────────────
const K1_ROOTS = ["axagent-ai", "axagent-core", "axagent-infra", "axagent-kit", "axagent-plugins", "axagent-quant", "axagent-runtime", "src-tauri"];
const K1_IGNORE = new Set(["target", "node_modules", "gen", "tests"]); // PLAN §0.7.1 扫描面（= .tmp/capability-census.cjs）
const AUDIT_SKIP = new Set(["target", "node_modules", "output", ".git", ".tmp", "dist", "gen", "build", "coverage", ".workbuddy", "examples"]); // audit-l1-blockers.mjs:29
const WALK_PRUNE = new Set(["target", "node_modules", "gen", ".git"]); // 一次遍历：只剪「体积/必然无料」目录，其余按路径段分类
const HARNESS_PREFIX = "axagent-core/crates/harness/";
// `pub trait X` 的声明面正则。**逐字等于** `scripts/audit-capability-inventory.mjs:60` 的 `PUB_TRAIT_RE`
// （#61 定案档 `k1DeclaredExternalImplTraitCount` = 本正则的声明面 ∩ `implTraitOf` 的站点面，两边必须
// 同一份判据 ⇒ 改这里必须同改那边，与 DOMAINS 的抄录同步点同一处置）。
const PUB_TRAIT_RE = /^pub trait\s+([A-Za-z_][A-Za-z0-9_]*)/;

// 锚点 check 种类的**声明清单**（与文件头 data-format 行同源，逐字一致）。
// --selftest 用它兜底并反向校验镜像表：表里出现本清单之外的新 check 种类 ⇒ 自检红，
// 逼着「新增一类判据」必须同批补一对负控+正控（第 2 条修复：覆盖度不许再靠惯性维持）。
const ANCHOR_CHECK_KINDS_DECLARED = [
  "registryCommandCount", "registryModuleCount", "registryUnassigned", "registryFileLineCount",
  "registryBoundaryCount", "registryDomainCount", "frontendVolume", "frontendDirVolume",
  "l1ForeignBlockers", "k1RsFileCount", "k1PubTraitCount", "remoteSeamRegistryCount",
  "registerGroupSum", "manual",
  "k2KeyedHits", // #57：K2 键控面读数（value=强档 trait 数 / value2=弱档）——登记必须 hard:false
  "commandNameSetCount", // #58：契约 A 名集合数 == 产物条目数 == 登记值（三方对齐）
  "k1DeclaredExternalImplTraitCount", // #61：K1 + **有声明面**的外部实现者 trait 数（两趟法，与 audit 脚本 scanK1Traits 同源）
  "agentCommandFaceCount", // #306：智能体工具面 = `#[agent_command]` 标注计数（value=全扫描面标注条数 / value2=非宿主根条数）
];

// DOMAINS 逐字抄自 scripts/audit-l1-blockers.mjs:64-79（152/28 与 293/84 873 的基线出处）
// ⚠ B9-1 起 STOCK.ownedRe 增加插件侧路径（`plugin-stock` / `stock-core` = 迁出目的地，
//   不是主壳）；两份必须同改，否则本门与审计脚本的 foreign 读数会分叉。
// ⚠ 2026-11-08：`stock-core` 同时进 `crates` / `useRe` —— 它是插件内 foundation，不是宿主
//   共享契约层；宿主直连它仍是「编译期链接股票业务代码」的阻塞。漏登记会让搬家凭空降数。
const DOMAINS = {
  STOCK: {
    crates: ["axagent-astock-data", "axagent-analysis-engine", "axagent-quant", "axagent-market-sim", "axagent-stock-core"],
    useRe: /axagent_(astock_data|analysis_engine|quant|market_sim|stock_core)::/,
    ownedRe: /^src-tauri\/src\/(commands|init|bin)\/(stock|market_|backtest|quant|pool_scan|universe_scan|time_travel|paper|trade|conditional_order|price_alert|analyst|recommendation_cron|mover_recall|limit_up|signal|reco|wf_des)|^axagent-plugins\/crates\/(plugin-stock|stock-core)\//,
    feRe: /stock|quote|kline|technical|portfolio|trading|advis|invest|quant|market/i,
    cmdRe: /^(stock|quote|kline|quant|backtest|portfolio|market_|pool|universe|reco|signal|north|margin|dragon|limit_up|time_travel|advis|paper|trade|conditional_order|price_alert|analyst|fundamental|techn|wf_des)/,
  },
  OPC: {
    crates: ["axagent-company-runtime"],
    useRe: /axagent_company_runtime::/,
    // agency_expert 已改判平台资产（2026-11-13 #299，与 audit-l1-blockers.mjs 的 DOMAINS 同批逐字对齐）：
    // Rust 侧零 OPC 域包依赖、前端消费面横跨 5 个 feature 域 ⇒ 不随 plugin-opc 迁出，不计入 OPC 边界。
    ownedRe: /^src-tauri\/src\/(commands|init)\/(opc|capability_pack|domain_pack|horizon|dojo|demand_discovery)/,
    feRe: /opc|capability-?pack|horizon|domain-?pack|dojo/i,
    cmdRe: /^(opc|capability_pack|domain|horizon|dojo|demand_)/,
  },
};

// ── 纯函数判据（--selftest 与正式跑共用同一实现，禁止两套）────────────────

/** 表自身一致性：分组计数=caliber.groupCounts、entries.length=totalRows、Σ 能力面=capacityRegister。 */
export function checkSelfConsistency(register) {
  const problems = [];
  const { caliber } = register;
  const groups = { A: 0, B: 0, C: 0, H: 0, K: 0, D: 0 };
  for (const e of register.entries) {
    if (!(e.group in groups)) { problems.push(`未知分组「${e.group}」（seamId=${e.seamId}）`); continue; }
    groups[e.group]++;
  }
  for (const g of Object.keys(groups)) {
    if (groups[g] !== caliber.groupCounts[g]) {
      problems.push(`分组 ${g}：登记 ${caliber.groupCounts[g]} / 实测表内 ${groups[g]} / 差 ${groups[g] - caliber.groupCounts[g]}`);
    }
  }
  if (register.entries.length !== caliber.totalRows) {
    problems.push(`表总行数：登记 ${caliber.totalRows} / 实测 ${register.entries.length} / 差 ${register.entries.length - caliber.totalRows}`);
  }
  const capSum = caliber.groupCounts.A + caliber.groupCounts.B + caliber.groupCounts.C;
  if (capSum !== caliber.capacityRegister) {
    problems.push(`能力面 Σ(A+B+C)：caliber.capacityRegister ${caliber.capacityRegister} / 分组和 ${capSum} / 差 ${capSum - caliber.capacityRegister}`);
  }
  const anchor = register.anchors.find((a) => a.check === "registerGroupSum");
  if (anchor && anchor.value !== capSum) {
    problems.push(`锚点 ${anchor.id}：登记 ${anchor.value} / 分组和 ${capSum} / 差 ${capSum - anchor.value}`);
  }
  return problems;
}

/** 锚点内部自洽（第 1 条修复 2026-11-02，堵假绿洞）：同一锚点的 `value` / `planNumber` /
 *  `planPinned[].mustContain` 必须指**同一个数**。旧门只比「登记值 vs 盘上实测」，从不比锚点自己 ⇒
 *  可以只把 `value` 改成盘上实测值迁就，而 PLAN 原文与 `planNumber`/`mustContain` 还留着旧数，
 *  一路合法判绿（读数漂移被「半改」洗白）。机械判据：
 *   ① 有 `planNumber` 且它是纯数字串（可含千位空格）者：剥掉空格后必须逐字等于 `String(value)`；
 *   ② `planPinned[].mustContain` 里凡是**纯数字串**（含 `84 873` 这类千位空格写法）的 token，
 *      规范化后必须等于**本锚点**的 `value`/`value2`，或等于**跨锚点宽容集合**里的数 —— 后一档宽容
 *      是必需的：现表 PLAN:352 的钉句同时含兄弟锚点读数（「1541 条 / 164 模块」）、PLAN:247 同时钉
 *      2005 与 316、PLAN:508 钉 293（= 本锚点 value2），只按「必须等于 String(value)」死判会把既存
 *      正确写法一并判红（本轮约束：不许新增红）。两侧都对不上 ⇒ 要么 pin 留旧数、要么 value 被单方面
 *      改走 ⇒ 红。
 *      ⚠ **宽容集合的口径（2026-11-11 阻断 1 收紧）**：跨锚点宽容只认 `hard:true` 且机器复检的锚点数；
 *      软锚点的数字不得作为宽容来源，否则改软锚点即可洗白钉句。旧实现把 `hard:false` / `check:"manual"`
 *      锚点的 `value`/`value2`/`planNumber` 一并收进集合，而那些锚点**改值永不判红** ⇒ 攻击链：把
 *      `k1RsFileCount` 的 `value` 与 `planNumber` 同改成盘上实测值（①被同时满足，所以①抓不到），
 *      再把旧数 2005 塞进 manual 锚点 `k1ExternalImplTraitCount`「藏」起来 ⇒ ②条形同虚设，整条钉句
 *      假绿。现口径：只有「登记与盘上实测会真的判红」的锚点才有资格替别人的钉句背书；
 *      `hard` 缺省（未声明 true）同样不给资格 —— fail-closed，宁可少宽容，不可留洗白路径。
 *      现盘仅两条 token 依赖这档宽容，且 donor 都是 hard:true 机器复检锚点 ⇒ 收窄后不新增红：
 *      `commandTotal` 钉句里的 `164` ← `commandModules`（check=registryModuleCount）、
 *      `k1RsFileCount` 钉句里的 `316` ← `k1PubTraitCount`（check=k1PubTraitCount）。
 *   ③ 非纯数字 token（`"28 文件"`、`"A 11 + B 10 + C 5"`）不做数字比对，仍只由 checkPlan 的存在性把关。
 *   ④（任务 #65）`planOld[].alsoAt[].mustContain` 的纯数字 token **同样**受 ② 管辖，另给一档窄宽容：
 *      允许等于「本条 planOld 自己声明的旧读数」（alsoAt 的用途就是给这个旧读数开合法出现处，
 *      要求它反过来等于登记值会自相矛盾）。除此之外不得放宽 —— 这是「把脱节数字从钉句挪进区段」
 *      这条洗白路径的封堵点。
 *  分档：本判据是「表自身散架」，与 checkSelfConsistency 同档 ⇒ **一律计入硬失败，不看 anchor.hard**
 *  （否则 hard:false 的 232/51 可以自说自话，判据分档只适用于「登记 vs 实测」那一类差异）。 */
export function checkAnchorConsistency(register) {
  const problems = [];
  const norm = (s) => String(s).replace(/[\s\u00A0\u2009]/g, "");
  const pureNum = (t) => /^\d+(?: \d{3})*$/.test(norm(t));
  const tableNumbers = new Set();
  for (const a of register.anchors) {
    if (a.hard !== true || a.check === "manual") { continue; } // 软锚点/manual 锚点的数不得作宽容来源（见上②）
    for (const v of [a.value, a.value2]) { if (typeof v === "number") { tableNumbers.add(norm(v)); } }
  }
  for (const a of register.anchors) {
    const own = new Set();
    for (const v of [a.value, a.value2]) { if (typeof v === "number") { own.add(norm(v)); } }
    const pins = (a.planPinned ?? []).map((p) => (p.mustContain ?? []).join(" + ")).join(" | ") || "—";
    if (a.planNumber && pureNum(a.planNumber) && norm(a.planNumber) !== norm(a.value)) {
      problems.push(`锚点 ${a.id} 自相矛盾：value=${a.value} / planNumber=${a.planNumber} / mustContain=${pins} —— planNumber 与 value 不是同一个数（①）`);
    }
    for (const pin of a.planPinned ?? []) {
      for (const t of pin.mustContain ?? []) {
        if (!pureNum(t)) { continue; }
        const d = norm(t);
        if (own.has(d) || tableNumbers.has(d)) { continue; }
        problems.push(`锚点 ${a.id} 自相矛盾：value=${a.value} / planNumber=${a.planNumber ?? "—"} / mustContain=${t} —— PLAN:${pin.line} 钉句里的这个数字既非本锚点登记数、也不属于「hard:true 且机器复检」锚点的登记数（②：一侧被改、一侧留旧数；藏进软锚点不算宽容）`);
      }
    }
    // ⑥ 档（任务 #65）：`planOld[].alsoAt[].mustContain` 的纯数字 token 同受本判据管辖 —— alsoAt 是新开的
    // 口子，若不纳入，「把与登记脱节的数字从 planPinned 挪进 alsoAt」就是现成的洗白路径。
    // 第四档宽容是**且仅是**「本条 planOld 自己声明的那个旧读数」：alsoAt 的用途正是给旧读数的合法出现处
    // 开门，这个数字必然不是登记值，要求它等于登记值会自相矛盾；除此之外一律按②/⑮ 的原口径判。
    for (const old of a.planOld ?? []) {
      const declaredOld = old.number === undefined || old.number === null ? null : norm(old.number);
      for (const rg of old.alsoAt ?? []) {
        for (const t of rg.mustContain ?? []) {
          if (!pureNum(t)) { continue; } // ③ 档惯例：非纯数字 token（`"HEAD 产物"`、`"1540 条"`）不做数字比对
          const d = norm(t);
          if (own.has(d) || tableNumbers.has(d) || (declaredOld !== null && d === declaredOld)) { continue; }
          problems.push(`锚点 ${a.id} 自相矛盾：value=${a.value} / planNumber=${a.planNumber ?? "—"} / alsoAt.mustContain=${t} —— planOld「${old.number}」的重锚区段 PLAN:${rg.startLine ?? "?"}–${rg.endLine ?? "?"} 钉了这个数字，它既非本锚点登记数、不是该条 planOld 声明的旧读数、也不属于「hard:true 且机器复检」锚点的登记数（⑥：alsoAt 不是藏数口子；要把段内其它数字当自证键，请写成非纯数字 token，如「${t} 条」）`);
        }
      }
    }
  }
  return problems;
}

/** 测试替身/产物排除（countRule.implementors：test_support|mock_|/bin/|e2e_|_fixture）。 */
export function isTestDoublePath(rel) {
  const parts = rel.split("/");
  const base = parts[parts.length - 1];
  return base.includes("test_support") || base.includes("mock_") || base.includes("e2e_") || base.includes("_fixture") || parts.includes("bin");
}

/** `^\s*(?:unsafe\s+)?impl … for <trait>` 的机械口径：取 impl 与 for 之间最后一个路径段（剥泛型）。
 *  被实现类型允许以 `$` 开头 —— 宏臂内的 `impl ProviderAdapter for $t`（providers/src/compat.rs:112/198）与
 *  `impl Tool for $name`（tools/src/tools/{browser,finance,obsidian}.rs 共 6 处）**计入站点**：
 *  这正是镜像表 note「实现者 14/199 由宏展开的 impl 站点构成」所声明的同一口径（14=12+2、199=193+6）。 */
const IMPL_LINE_RE = /^\s*(?:unsafe\s+)?impl(?:\s*<[^;{]*?>)?\s+([A-Za-z_][\w:]*(?:<[^>]*>)?)\s+for\s+([A-Za-z_$(][\w:]*)/;
export function implTraitOf(line) {
  const m = IMPL_LINE_RE.exec(line);
  if (!m) { return null; }
  return m[1].replace(/<.*$/, "").split("::").pop();
}

/** 同一行的**被实现类型**侧（group 2）——K2 形态⑧ 的 impl→trait 反查表用（#57）。
 *  与 implTraitOf 共用 IMPL_LINE_RE（禁区 12：一条 impl 形态只许有一套正则）；
 *  group 2 只是把已有匹配的右半边取出来，**不改变命中集合**。宏臂 `for $t` 无具体类型名 ⇒ 返回 null。 */
export function implTypeOf(line) {
  const m = IMPL_LINE_RE.exec(line);
  if (!m || m[2].startsWith("$")) { return null; }
  return m[2].replace(/<.*$/, "").split("::").pop();
}

/** 内联 `#[cfg(test)] mod` 块的行区间（PLAN §24.5 / 任务 #54 的「其中在 test 块内」列）。
 *  判据（机械、可复核）：属性行 `^\s*#\[cfg\(test\)\]`（同一行或**紧邻下一个非空行**必须跟着
 *  `mod 名 … {`），从该 `mod` 行起按**花括号配平**到块闭合为止 ⇒ 「最近一个未闭合的 `#[cfg(test)] mod`」。
 *  ⚠ 三条刻意的边界：
 *   ① `#[cfg(test)]` 后跟 `fn` / `use` / `const` **不开区间** —— 那是被门控的单个条目，误开会把后续
 *      生产站点整段吞进 test 列（方向上比「少计」更危险）；
 *   ② 嵌套在已识别块里的属性不重复开区间（外层区间已覆盖）；
 *   ③ 花括号按字符计数，不剥字符串/注释 —— 与本文件既有的块体定位（`k2TraitDefs`、④/⑧ 的 match 块）
 *      同一套粗粒度做法，**不许**在这里另发明第二套「块边界」。
 *  纯函数（输入整份文件文本）⇒ collectReadings 与 selftest 共用同一实现。 */
export function cfgTestRanges(text) {
  const ranges = [];
  if (!text || !text.includes("#[cfg(test)]")) { return ranges; }
  const lines = text.split("\n");
  const ATTR_RE = /^\s*#\[cfg\(test\)\]/;
  const MOD_RE = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\b/;
  for (let i = 0; i < lines.length; i++) {
    if (!ATTR_RE.test(lines[i])) { continue; }
    let head = -1;
    if (MOD_RE.test(lines[i].replace(ATTR_RE, "").trim())) { head = i; } else {
      for (let j = i + 1; j <= Math.min(i + 2, lines.length - 1); j++) {
        if (!lines[j].trim()) { continue; }
        if (MOD_RE.test(lines[j])) { head = j; }
        break; // 紧邻的非空行不是 `mod` ⇒ 这个属性不属于测试块
      }
    }
    if (head < 0) { continue; }
    let depth = 0, opened = false, end = lines.length;
    for (let k = head; k < lines.length; k++) {
      for (const ch of lines[k]) {
        if (ch === "{") { depth++; opened = true; } else if (ch === "}") { depth--; }
      }
      if (opened && depth <= 0) { end = k + 1; break; }
    }
    ranges.push({ start: head + 1, end });
    i = end - 1; // 块内不重复开区间（边界 ②）
  }
  return ranges;
}

/** 行号（1 基）是否落在某个 test 块区间内（**两端闭合**：块头行与块尾行都算）。 */
export function lineInRanges(ranges, n) {
  return ranges.some((r) => n >= r.start && n <= r.end);
}

/** seamDeclared（countRule.seamDeclared）：seamId 字面量相等或以 `seamId.` 为前缀。text=null 表示文件缺失。 */
export function seamDeclared(seamId, text) {
  if (text === null || text === undefined) { return null; }
  return text.includes(`"${seamId}"`) || text.includes(`"${seamId}.`);
}

/** `k1DeclaredExternalImplTraitCount`（#61 定案档）的交并步，**纯函数**：
 *  「扫描面内有 `pub trait X` 声明」∩「X 有非 harness、非测试替身的 impl 站点」的 trait 名。
 *  = `scripts/audit-capability-inventory.mjs` 的 `scanK1Traits().declaredExternalTraits` 同一定义。
 *  ⚠ 本函数**只能在两批名字都收齐之后调用**（两趟法）。在遍历中途逐行调用，等价于把
 *  「impl 站点早于其声明文件出现」的 trait 静默丢掉 ⇒ 顺序相关读数（audit 脚本里命名为 `singlePass`，
 *  当前树恒少 22 条）—— 那是 #61 复现并归因的**缺陷**，不是另一档口径，禁止用它出数。
 *  返回排序后的名单（自检与 dump 可逐条点名）。 */
export function declaredExternalTraitNames(siteNames, declaredNames) {
  const declared = new Set(declaredNames);
  return [...new Set(siteNames)].filter((t) => declared.has(t)).sort();
}

/** 证据锚点判据：文件存在 + 行号存在 + 有 token 时该行（或 [line,endLine] 区间）含该 token。 */
export function adjudicateEvidence(ev, contentFor) {
  const content = contentFor(ev.file);
  if (content === null) { return `登记 ${ev.ref} / 实测 文件不存在`; }
  if (!ev.line) { // fileLevel：只判存在 + token 在文件任一处
    if (ev.token && !content.includes(ev.token)) { return `登记 ${ev.ref}（文件级 token「${ev.token}」）/ 实测 全文无此 token`; }
    return null;
  }
  const lines = content.split("\n");
  if (ev.line > lines.length) { return `登记 ${ev.ref} / 实测 文件仅 ${lines.length} 行（行号越界）`; }
  if (ev.endLine) {
    if (ev.endLine > lines.length) { return `登记 ${ev.ref} 区间止点 ${ev.endLine} / 实测 文件仅 ${lines.length} 行`; }
    const seg = lines.slice(ev.line - 1, ev.endLine);
    if (ev.token && !seg.some((l) => l.includes(ev.token))) { return `登记 token「${ev.token}」应落在 ${ev.line}–${ev.endLine} / 实测 区间内无此 token`; }
    return null;
  }
  const l = lines[ev.line - 1];
  if (ev.token && !l.includes(ev.token)) {
    return `登记 token「${ev.token}」/ 实测 第 ${ev.line} 行「${l.trim().slice(0, 80)}」`;
  }
  // 有 anchorText 时必须整行相符：只看 token 会让「同一 token 的另一处调用」冒充原锚点。
  // 实测样本（#131）：workflow.evolver 区间止点原指 `is_running`，行位移后该行变成
  // `set_llm_provider` —— token 仍在，判据却绿，端点已指向别的语义。
  if (ev.anchorText && normPlanLine(l) !== normPlanLine(ev.anchorText)) {
    return `登记 anchorText「${normPlanLine(ev.anchorText).slice(0, 80)}」/ 实测 第 ${ev.line} 行「${normPlanLine(l).slice(0, 80)}」`;
  }
  return null;
}

/** 数字在文本中的「非行号引用」出现（前后不接数字；左不接 `: . / # §`，右不接 `.数字`，
 *  防 `PLAN:124`、章节号 `§124`、**小节号 `124.1`** 被算成读数）。返回行号列表。 */
export function numberOccurrences(text, num) {
  const hits = [];
  const re = new RegExp(`(^|[^\\d:.#§])${num}(?!\\d|\\.\\d)`, "g");
  text.split("\n").forEach((l, i) => {
    re.lastIndex = 0;
    if (re.test(l)) { hits.push(i + 1); }
  });
  return hits;
}

/** planNumber 的「数字边界」命中（第 4 条修复 2026-11-02，替代旧的裸子串 `planText.includes(num)`）。
 *  旧实现近乎恒真：短数 `"26"` 在 PLAN 里裸子串命中 52 行（绝大多数是 `2026-11-xx` 日期里的 `026`）、
 *  `"51"` `"83"` 各 11 行 ⇒ 登记值改错了也照样「出现在 PLAN 里」，这一档实际不判任何东西。
 *  新规则（机械，四条，写死在此处）：
 *   ① 取 planNumber 的数字位，**位与位之间允许 0 或 1 个空格**（半角空格 / NBSP / 窄空格），
 *      兼容 PLAN 的千位空格写法 `84 873`、`60 757`；不允许插入任何其它字符；
 *   ② 命中处右侧不得紧邻数字或小数点，左侧不得紧邻数字/小数点/冒号/斜杠/井号/**节号 `§`**
 *      （沿用 numberOccurrences 的「非行号引用」约定：防 `84873.5`、`1.26`、`PLAN:124`、
 *      以及漂移段 `1534/1541`、`124/83` 这类「斜杠后的配对旧数」被当成正读数控数。
 *      ⚠ `§` 与 `.数字` 这两条例外三处都要有：`numberOccurrences` / `numberHitLines` /
 *      `planNumberHits` 必须逐字同步，否则「段内是否仍有旧读数」与「出段列举」会对同一个数
 *      给出相反答案。三者的**右边界并不完全相同**（既存差异，不是本次引入）：
 *      `planNumberHits` 用 `(?![\d.])`，句末 `124.` 也不计；另两者只排除 `124.1` 这种
 *      「点后紧跟数字」的小节号，句末 `124.` 仍计 —— 判 planNumber「登记值有没有写进正文」
 *      时取严是安全的，判「旧读数出没出段」时取严会把真出段放过，故那里只能到 `.\d` 为止）。
 *   ③ 逐行匹配前先把**日期/时刻上下文整串涂白**（`YYYY-MM-DD`、`YYYY/M/D`、`D.M.YYYY`、`HH:MM(:SS)`
 *      替换为等长空格）⇒ 纯日期行不再贡献命中，日期里的 26/51/83 之类两位数不再假绿；
 *   ④ planNumber 不是纯数字串（若将来出现措辞型锚点）时退回裸子串语义，不放松 ①–③。
 *  返回命中行号列表（空列表 = 未在 PLAN 正文以合法形态出现）。
 *  2026-11-02 现盘核对：15 个 planMustAppear 锚点在 ①–③ 下仍**全部命中**（`26` 由 52 行收敛到 18 行、
 *  `51` 11→2、`83` 11→4、`306` 2→1、`1541` 5、`60 757` 2）⇒ 只削假绿，不新增红。 */
export function planNumberHits(planText, num) {
  const lines = planText.split("\n");
  const digits = String(num).replace(/[\s\u00A0\u2009]/g, "");
  if (!/^\d+$/.test(digits)) {
    return lines.map((l, i) => (l.includes(String(num)) ? i + 1 : -1)).filter((i) => i > 0);
  }
  const dateRes = [
    /\b(?:19|20)\d{2}[-/.]\d{1,2}(?:[-/.]\d{1,2})?\b/g,
    /\b\d{1,2}[-/.](?:19|20)\d{2}\b/g,
    /\b\d{1,2}:\d{2}(?::\d{2})?\b/g,
  ];
  const re = new RegExp(`(?:^|[^\\d.:#/§])${digits.split("").join("[ \\u00A0\\u2009]?")}(?![\\d.])`, "g");
  const hits = [];
  lines.forEach((raw, i) => {
    let l = raw;
    for (const dr of dateRes) { l = l.replace(dr, (m) => " ".repeat(m.length)); }
    re.lastIndex = 0;
    if (re.test(l)) { hits.push(i + 1); }
  });
  return hits;
}

/** §0.12.4 漂移列举段区段（判定规则见文件头注释 —— 区段含「漂移」字样才作数）。 */
export function driftRegionLines(planText) {
  const lines = planText.split("\n");
  const start = lines.findIndex((l) => /^#{1,4}\s.*0\.12\.4/.test(l));
  if (start < 0) { return null; }
  const level = (lines[start].match(/^#+/) || [""])[0].length;
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    const m = /^(#+)\s/.exec(lines[i]);
    if (m && m[1].length <= level) { end = i; break; }
  }
  const region = lines.slice(start, end).join("\n");
  if (!region.includes("漂移")) { return null; }
  return { start: start + 1, end };
}

/** 旧读数与自证 token 的允许距离（行）。见 [`alsoAtRegionStatus`] 的三条判据。 */
export const ALSOAT_ANCHOR_RADIUS = 3;

/** `planOld[].alsoAt[]` 区段是否**健康**（任务 #65 立，任务 `#138` 加严）。
 *
 *  ## 三条判据（缺一即腐烂）
 *
 *  1. **区间合法且带键**：1 ≤ startLine ≤ endLine ≤ 文末，`mustContain` 非空；
 *  2. **段内仍有旧读数**：`number` 以数字边界形态出现在区段内 —— 否则窗口已经漂到与断言无关的位置，
 *     仅凭「token 还在段里」判健康会让这个区段变成一个永远放行的空洞（`#138` 的假健康形态）；
 *  3. **出现处有自证**：至少一个出现行的 **±`ALSOAT_ANCHOR_RADIUS` 行**内命中某个 mustContain token。
 *     放行按**出现行**算（返回 `anchored`），不再按整个区段放行 —— 否则一个 60 行的窗口会把
 *     漂进来的无关同数出现一起吞掉。
 *
 *  token 比对口径逐字复用 planPinned 的 `String.includes`；差别只在 planPinned 要求全部 token
 *  **同一行**、alsoAt 允许 token 在出现行附近跨行（表格行 / 紧邻说明），这是定位形态的差异。
 *
 *  `number` 缺省（旧调用方）时跳过 2/3 两条 —— 只有 `--repin-plan` 的「原样是否仍可用」自检走这一档，
 *  正式判红一律传 number。
 *
 *  返回 `{ ok, start, end, missing, reason, occurrences, anchored }`。 */
export function alsoAtRegionStatus(planLines, region, number = null) {
  const start = Number(region?.startLine);
  const end = Number(region?.endLine);
  const raw = region?.mustContain ?? [];
  if (!Number.isInteger(start) || !Number.isInteger(end) || start < 1 || end < start) {
    return { ok: false, start, end, missing: [], reason: `区段行号缺失或非法（startLine=${region?.startLine ?? "缺"} / endLine=${region?.endLine ?? "缺"}，须 1 ≤ start ≤ end）`, occurrences: [], anchored: [] };
  }
  if (!raw.some((t) => String(t).length > 0)) {
    return { ok: false, start, end, missing: [], reason: "段内无 mustContain 自证 token ⇒ 只剩裸行号，无据放行（缺定位键即拒，与 planPinned 同口径）", occurrences: [], anchored: [] };
  }
  const missing = raw.filter((t) => String(t).length > 0 && !planLines.slice(start - 1, end).some((l) => l.includes(t)));
  if (missing.length) {
    return { ok: false, start, end, missing, reason: "", occurrences: [], anchored: [] };
  }
  if (number === null || number === undefined) {
    return { ok: true, start, end, missing: [], reason: "", occurrences: [], anchored: [] };
  }
  const occurrences = String(number).split("").every((c) => /\d/.test(c))
    ? numberHitLines(planLines, String(number)).filter((ln) => ln >= start && ln <= end)
    : planLines.slice(start - 1, end).map((l, i) => (l.includes(String(number)) ? start + i : 0)).filter(Boolean);
  if (!occurrences.length) {
    return { ok: false, start, end, missing: [], reason: `段内已找不到旧读数 ${number} 的合法出现 ⇒ 窗口漂到了与断言无关的位置（token 还在段内也不作数）`, occurrences: [], anchored: [] };
  }
  const anchors = raw.filter((t) => String(t).length > 0);
  const anchored = occurrences.filter((ln) => {
    const lo = Math.max(start, ln - ALSOAT_ANCHOR_RADIUS);
    const hi = Math.min(end, ln + ALSOAT_ANCHOR_RADIUS);
    for (let i = lo; i <= hi; i++) {
      if (anchors.some((t) => (planLines[i - 1] ?? "").includes(t))) return true;
    }
    return false;
  });
  if (!anchored.length) {
    return { ok: false, start, end, missing: [], reason: `旧读数 ${number} 的出现行 ±${ALSOAT_ANCHOR_RADIUS} 行内没有任何自证 token ⇒ 段内的出现无法归因到本断言`, occurrences, anchored: [] };
  }
  return { ok: true, start, end, missing: [], reason: "", occurrences, anchored };
}

/** 按数字边界找旧读数的出现行（与 [`numberOccurrences`] **同一正则口径**，只是输入换成分行数组；
 *  ⚠ 两处必须逐字同步，否则「段内是否仍有旧读数」与「出段列举」会对同一个数给出相反答案）。 */
export function numberHitLines(planLines, number) {
  const re = new RegExp(`(^|[^\\d:.#§])${number}(?!\\d|\\.\\d)`);
  const hits = [];
  planLines.forEach((l, i) => {
    re.lastIndex = 0;
    if (re.test(l)) hits.push(i + 1);
  });
  return hits;
}

/** PLAN 正文对账（只读 docs/**，绝不写）：planMustAppear / planPinned / planOld / bannedPlanPhrases。
 *  分级与锚点自身一致：hard:false 锚点（如 manual 档 232/51）的 planNumber/钉句缺失进 soft（只报告），
 *  与「manual ⇒ 不判红」的总分档一致；禁语属 caliber 级判据，一律 hard。 */
export function checkPlan(register, planText) {
  const fails = [];
  const soft = [];
  const passed = [];
  if (planText === null) {
    fails.push(`PLAN 文件缺失（${register.sources.plan}）—— 跳过 ≠ 通过：planMustAppear / planPinned / planOld / bannedPhrases 全部无法核对`);
    return { fails, soft, passed };
  }
  const lines = planText.split("\n");
  let appears = 0;
  let healthyRegions = 0;
  let rottenRegions = 0;
  for (const a of register.anchors) {
    const sink = a.hard === false ? soft : fails;
    if (a.planMustAppear && a.planNumber) {
      // 第 4 条修复：数字边界命中（旧 `planText.includes` 裸子串对短数近乎恒真，规则见 planNumberHits 注释）
      const hits = planNumberHits(planText, a.planNumber);
      if (!hits.length) {
        sink.push(`锚点 ${a.id}：planNumber「${a.planNumber}」未在 PLAN 正文以数字边界形态命中（裸子串旧口径已废 —— 只允许日期涂白后、前后不接数字/小数点的出现算数）`);
      } else { appears++; }
    }
    for (const pin of a.planPinned ?? []) {
      const l = lines[pin.line - 1];
      const missing = pin.mustContain.filter((t) => !(l ?? "").includes(t));
      if (l === undefined || missing.length) {
        sink.push(`锚点 ${a.id} 钉句 PLAN:${pin.line} 不符 —— 缺「${missing.join(" / ") || (l === undefined ? "该行不存在" : "")}」`);
      }
    }
    for (const old of a.planOld ?? []) {
      const region = driftRegionLines(planText);
      const hits = numberOccurrences(planText, old.number);
      const alsoList = old.alsoAt ?? [];
      const healthy = [];
      const rotten = [];
      // ① 先验段自身（与出现行无关：段腐烂是**独立**一条缺陷，段内一次旧读数没有也要报）
      alsoList.forEach((rg, j) => {
        const st = alsoAtRegionStatus(lines, rg, old.number);
        if (st.ok) { healthyRegions++; healthy.push(st); return; }
        rottenRegions++;
        rotten.push(st);
        sink.push(`[区段腐烂 ALSOAT-STALE] 锚点 ${a.id} 旧读数 ${old.number} 的第 ${j + 1} 个 alsoAt 区段 PLAN:${rg?.startLine ?? "?"}–${rg?.endLine ?? "?"} 已不可信 —— ${st.missing.length ? `段内缺自证 token [${st.missing.join(" / ")}]` : st.reason} ⇒ 缺陷类型是**行号锚腐烂**（段被改写/搬走/没带键），与「旧读数出段」不是同一件事；该段内的旧读数出现一律照旧判红 —— 腐烂的重锚不得默默放行旧数。修法：跑 \`--repin-plan\` 重锚或直接收紧 mustContain${old.note ? ` —— ${old.note}` : ""}`);
      });
      const inSeg = (ln, s) => s !== null && ln >= s.start && ln <= s.end;
      const segLabel = (s) => `${s.start}–${s.end}`;
      for (const ln of hits) {
        if (inSeg(ln, region) || healthy.some((s) => s.anchored.includes(ln))) { continue; }
        const parts = region ? [region, ...healthy].map(segLabel) : healthy.map(segLabel);
        const where = region
          ? `，合法区段仅 ${parts.join(" / ")}`
          : (healthy.length ? `，且找不到含「漂移」的 §0.12.4 区段（合法区段仅剩 alsoAt ${parts.join(" / ")}）` : "，且找不到含「漂移」的 §0.12.4 区段");
        const inside = rotten.filter((s) => inSeg(ln, s));
        // `#138`：落在**健康**区段范围内、但该行离自证 token 超过 ±半径 ⇒ 不是本断言的合法出现。
        // 放行必须按出现行（`anchored`），不能按窗口整段 —— 否则位移后漂进来的无关同数被一起吞掉。
        const swallowed = healthy.filter((s) => inSeg(ln, s));
        sink.push(`旧读数 ${old.number} 出现在漂移列举段之外（PLAN:${ln}${where}）—— ${old.note}${inside.length ? `；⚠ 该行落在**已腐烂**的 alsoAt 区段 ${inside.map(segLabel).join(" / ")} 内 ⇒ 腐烂区段不作放行，缺陷定性见同锚点的 ALSOAT-STALE 专码行` : ""}${swallowed.length ? `；⚠ 该行虽在健康 alsoAt 区段 ${swallowed.map(segLabel).join(" / ")} 的范围内，但 ±${ALSOAT_ANCHOR_RADIUS} 行内无自证 token ⇒ 窗口不得吞掉无关同数出现（#138），请收窄区段或把该处的自证 token 补进 mustContain` : ""}`);
      }
    }
  }
  for (const ph of register.caliber.bannedPlanPhrases ?? []) {
    if (planText.includes(ph)) { fails.push(`并行口径禁语「${ph}」出现在 PLAN 正文（定案口径见 caliber.statement）`); }
  }
  const alsoAtCount = register.anchors.reduce((n, a) => n + (a.planOld ?? []).reduce((m, o) => m + (o.alsoAt?.length ?? 0), 0), 0);
  passed.push(`PLAN 对账：planMustAppear ${appears} 项 · planPinned ${register.anchors.reduce((n, a) => n + (a.planPinned?.length ?? 0), 0)} 处 · planOld ${register.anchors.reduce((n, a) => n + (a.planOld?.length ?? 0), 0)} 项${alsoAtCount ? `（另有 ${alsoAtCount} 个 alsoAt 重锚区段，健康 ${healthyRegions} / 腐烂 ${rottenRegions}）` : ""} · 禁语 ${(register.caliber.bannedPlanPhrases ?? []).length} 条`);
  return { fails, soft, passed };
}

/** 总对账：读表 + 读数 + PLAN 文本 ⇒ 硬失败/报告/未机器复检三本账。纯函数（内容经 readings.contentFor 注入）。 */
export function reconcile(register, readings, planText) {
  const hardFails = [];
  const reports = [];
  const manualLines = [];
  const anchorResults = [];
  const entryResults = [];

  // ① 表自身一致性（镜像表是判据 ⇒ 它自己散架必红）
  for (const p of checkSelfConsistency(register)) { hardFails.push(`表一致性：${p}`); }
  // ①b 锚点内部自洽（第 1 条修复：value / planNumber / planPinned.mustContain 三对齐；不看 anchor.hard）
  const anchorConsistency = checkAnchorConsistency(register);
  for (const p of anchorConsistency) { hardFails.push(p); }

  const remoteSet = new Set(readings.remoteSeams ?? []);
  // 封闭集合对账（countRule.closure）：13 = A 11 + K 组 agent.loop / session.log.invariant，多一条少一条都红
  const expectRemote = register.entries.filter((e) => e.expect && e.expect.remote === true).map((e) => e.seamId).sort();
  const seamAnchor = register.anchors.find((a) => a.check === "remoteSeamRegistryCount");
  // 计数/封闭两本账先算好（消息里的 seamId 由锚点行统一印出，见下方 switch 的 remoteSeamRegistryCount 分支）
  let remoteCountIssue = null;
  let remoteClosureIssue = null;
  if (readings.remoteSeams !== null) {
    const actual = [...remoteSet].sort();
    const expectSet = new Set(expectRemote);
    const missing = expectRemote.filter((s) => !remoteSet.has(s));
    const extra = actual.filter((s) => !expectSet.has(s));
    if (seamAnchor && readings.remoteSeams.length !== seamAnchor.value) {
      remoteCountIssue = `登记 ${seamAnchor.value} 条 / 实测 ${readings.remoteSeams.length} 条 / 差 ${readings.remoteSeams.length - seamAnchor.value}`;
    }
    if (missing.length || extra.length) {
      remoteClosureIssue = `封闭集合破裂：缺 [${missing.join(", ")}] / 多 [${extra.join(", ")}]（登记面=表中 remote:true 的 ${expectRemote.length} 条）`;
    }
  }

  // ② 锚点
  for (const a of register.anchors) {
    let mismatch = null;
    const cmpNum = (actual, exp, unit) => {
      if (actual === undefined || actual === null) { return "实测读数缺失（源文件未见）"; }
      if (actual !== exp) { return `登记 ${exp}${unit ?? ""} / 实测 ${actual}${unit ?? ""} / 差 ${actual - exp}`; }
      return null;
    };
    switch (a.check) {
      case "registryCommandCount": {
        if (!readings.registry) { mismatch = "注册表缺失 —— 跳过 ≠ 通过"; break; }
        const base = cmpNum(readings.registry.commandTotal, a.value, " 条");
        if (!base) { mismatch = null; break; }
        // #58：比登记值之前先跑**仓内已有**的契约 A 名字集合谓词，分流「产物滞后」与「真漂移」。
        // 两档**都判红**（专码只买到诊断正确性，CI 里非零即中断）—— 不许借分流把红洗成绿或降成只报告。
        const cls = classifyCommandCountDrift(readings.commandNames);
        mismatch = `${cls.code}${base} —— ${cls.note}`;
        break;
      }
      case "commandNameSetCount": {
        // #58 加强档：三方对齐（登记值 == 产物条目数 == 契约 A 名集合数）。
        // 存在理由：`defined ⊆ registered` 用的是**名字集合**，同名跨模块只注册一个时名集合会**静默低估**
        // （今天 0 例，且无编译期保证）⇒ 只有把「名集合数 == 产物条目数」也钉住，A 档的绿才等于「产物与源码一致」。
        const n = readings.commandNames;
        if (!n) { mismatch = "契约 A 名字集合读数缺失（扫描面未产出 src-tauri/src）"; break; }
        const base = cmpNum(n.entriesCount, a.value, " 条（产物条目数）") ?? cmpNum(n.definedCount, a.value, " 个（契约 A 名集合数）");
        if (base) {
          const cls = classifyCommandCountDrift(n);
          mismatch = `${base}${n.definedCount !== n.entriesCount ? ` / **名集合与产物条目数不等（${n.definedCount} ≠ ${n.entriesCount}）⇒ 名集合静默低估或有同名跨模块未注册**` : ""} —— ${cls.code}${cls.note}`;
        }
        break;
      }
      case "registryModuleCount":
        mismatch = !readings.registry ? "注册表缺失 —— 跳过 ≠ 通过" : cmpNum(readings.registry.commandModules, a.value, " 模块");
        break;
      case "registryUnassigned":
        mismatch = !readings.registry ? "注册表缺失 —— 跳过 ≠ 通过"
          : readings.registry.unassigned.length ? `未归属模块非空（domainOf 兜底「其它」）：${readings.registry.unassigned.join(", ")} —— 非空即漂移，PLAN:226 候选集前提失效` : null;
        break;
      case "registryFileLineCount":
        mismatch = !readings.registry ? null : cmpNum(readings.registry.registryFileLines, a.value, " 行");
        break;
      case "registryBoundaryCount": {
        const r = readings.boundary?.[a.domainKey];
        mismatch = !r ? "读数缺失" : (cmpNum(r.total, a.value, " 条") ?? cmpNum(r.modules, a.value2, ` ${a.value2Label}`));
        break;
      }
      case "registryDomainCount": {
        const r = readings.auditDomain?.[a.domainName];
        if (!r) {
          // 整域合法归零（2026-11-13 #264 批 4 收口：`opc` 宿主模块删尽 ⇒ 「OPC 公司运行时」域从
          // domainOf 读数中消失）：锚点登记 0/0 ⇒ 域缺失就是 0 条命令的读数形态，放行；
          // 登记非 0 ⇒ 域缺失仍是「域名/DOMAIN_RULES 变了或漂移未回写」，照旧必红（负控⑭-opcZero 锁）。
          mismatch = (a.value === 0 && (a.value2 ?? 0) === 0) ? null
            : `域「${a.domainName}」不存在于 domainOf 读数（域名或 DOMAIN_RULES 变了 ⇒ 同批改表）`;
          break;
        }
        mismatch = cmpNum(r.total, a.value, " 条") ?? cmpNum(r.modules, a.value2, ` ${a.value2Label}`);
        break;
      }
      case "frontendVolume":
        mismatch = !readings.frontend ? "前端读数缺失" : (cmpNum(readings.frontend.lines, a.value, " 行") ?? cmpNum(readings.frontend.files, a.value2, " 文件"));
        break;
      case "frontendDirVolume": {
        const r = readings.frontendDir?.[a.dir];
        mismatch = !r ? `目录 ${a.dir} 读数缺失` : (cmpNum(r.lines, a.value, " 行") ?? cmpNum(r.files, a.value2, " 文件"));
        break;
      }
      case "l1ForeignBlockers": {
        const r = readings.l1?.[a.domainKey];
        mismatch = !r ? "L1 读数缺失" : (cmpNum(r.hits, a.value, " 处") ?? cmpNum(r.files, a.value2, " 文件"));
        break;
      }
      case "k1RsFileCount":
        mismatch = !readings.k1 ? "K1 扫描面缺失" : cmpNum(readings.k1.rsFiles, a.value, " 文件");
        break;
      case "k1PubTraitCount":
        mismatch = !readings.k1 ? "K1 扫描面缺失" : cmpNum(readings.k1.pubTraits, a.value, " trait");
        break;
      case "k1DeclaredExternalImplTraitCount":
        // #61：定案档 = 「扫描面内有 `pub trait X` 声明」∩「X 有非 harness、非测试替身的 impl 站点」。
        // 读数由 collectReadings 的两趟交并给出，口径与 `scripts/audit-capability-inventory.mjs`
        // 的 `scanK1Traits().declaredExternalCount` **逐字同源**（谓词 implTraitOf / isTestDoublePath
        // 本就被该脚本 import 本门，见禁区 12）。它与 k1.externalImplTraits（不要求声明面，A 档）
        // 是两档不同的数，**不许互换**：实现者逐条对账用 A 档，锚点用本档。
        mismatch = !readings.k1 ? "K1 扫描面缺失" : (readings.k1.declaredExternalTraits === undefined
          ? "读数缺字段 declaredExternalTraits（本门 K1 块未产出 ⇒ 不是登记错，是判据没落地）"
          : cmpNum(readings.k1.declaredExternalTraits, a.value, " trait"));
        break;
      case "k2KeyedHits": {
        // #57：**只出数不判红** —— 本锚点在镜像表必须以 `hard:false` 登记（下方 reports 档），
        // 它不改 `k2TraitFaces`（51 / manual / hard:false）的现值与口径，升 hard 属 #54。
        const k = readings.k2;
        mismatch = !k ? "K2 读数缺失（扫描面未产出）" : (cmpNum(k.strongHits, a.value, " 强") ?? cmpNum(k.weakHits, a.value2, " 弱"));
        break;
      }
      case "agentCommandFaceCount": {
        // #306：智能体工具面 = `#[agent_command]` 属性标注的条数。存在理由——本门原读数全部钉在
        //   **命令注册面**（parseRegistry 逐行）上；而 LLM 实际可调用的工具面由宏的 `inventory::submit!`
        //   在编译期收集，源码里没有命令名字符串 ⇒ 「grep 不到消费方」恒真、不证明无人使用。删一批标注
        //   时 commandTotal 会红，但「这些命令原本是不是工具」门此前一个数都不出。
        //   value=扫描面标注总数 · value2=**非宿主根**条数（当前恒 0：插件命名命令经
        //   `build_plugin_command_index` 并入索引、不走 inventory ⇒ 这个 0 是要钉住的锚，插件侧真接入
        //   标注时必须显式判红并登记，不许静默变化）。
        const ac = readings.agentCommand;
        mismatch = !ac ? "智能体工具面读数缺失（本门未出数 ⇒ 不是登记错，是判据没落地）"
          : (cmpNum(ac.total, a.value, " 条标注") ?? cmpNum(ac.nonHost, a.value2, ` ${a.value2Label ?? "条非宿主根标注"}`));
        break;
      }
      case "remoteSeamRegistryCount":
        mismatch = readings.remoteSeams === null ? "worker.rs 缺失 —— 跳过 ≠ 通过（数量与封闭集合均无法核对）"
          : (remoteCountIssue ?? remoteClosureIssue);
        break;
      case "registerGroupSum":
        mismatch = null; // 已在表一致性处判
        break;
      case "manual":
        reports.push(`[只报告] 锚点 ${a.id}（${a.label}）：登记 ${a.value}${a.implementorsMechanical !== undefined && readings.k1 ? ` / 本门机械复算 ${readings.k1.externalImplTraits}（表内自记 ${a.implementorsMechanical}）` : ""} —— ${a.reason ?? "不可机械复算"}`);
        break;
      default:
        mismatch = `未知 check「${a.check}」`;
    }
    if (a.check !== "manual") {
      anchorResults.push({
        status: !mismatch ? "PASS" : a.hard === false ? "REPORT" : "FAIL",
        txt: `${a.id}（${a.label}）= ${a.value}${mismatch ? ` —— ${mismatch}` : ""}`,
      });
    }
    if (mismatch && a.check !== "manual") {
      const line = `锚点 ${a.id}（${a.label}）：${mismatch}`;
      if (a.hard === false) { reports.push(`[只报告] ${line}`); } else { hardFails.push(`FAIL: ${line}`); }
    }
  }
  // 锚点自洽档的通过面也印出来（否则「绿」只显示逐锚点读数，看不出这条新判据到底跑没跑）
  anchorResults.push({
    status: anchorConsistency.length ? "FAIL" : "PASS",
    txt: `内部自洽（value/planNumber/mustContain 三对齐）：${register.anchors.length} 项${anchorConsistency.length ? ` —— ${anchorConsistency.length} 项矛盾` : ""}`,
  });

  // 出数不判红（#57 / #58）：即使镜像表还没登记这两类锚点，读数与分流诊断也必须每轮印出来 ——
  // 「口径工件不落库 / 数字不印出来」正是 #50 三读数案的病根。这两行进 reports 档，**不参与退出码**。
  if (readings.k2) {
    reports.push(`[只报告] K2 键控面（#57 新入库谓词，不决定退出码）：强 ${readings.k2.strongHits} / 弱 ${readings.k2.weakHits} —— 扫描面 ${readings.k2.scannedTraits} 个 pub trait、⑧ impl→trait 反查表 ${readings.k2.implTypesKnown} 个实现类型 · 形态命中 ${JSON.stringify(readings.k2.formCounts)}。⚠ 与软锚点 k2TraitFaces（51 / check:manual）不是同一口径：本读数**不判红、不改 51**（升 hard 与重算 51 属 #54 同批）。`);
  }
  if (readings.commandNames) {
    const n = readings.commandNames;
    reports.push(`[只报告] 契约 A 名字集合（#58 分流依据，复用 check-contracts 判据 ⇒ 零新解析器）：defined ${n.definedCount} / 已注册名 ${n.registeredNameCount} / 产物条目 ${n.entriesCount} · 已定义未注册 ${n.unregistered.length}${n.unregistered.length ? `（首条 ${n.unregistered[0]}）⇒ commandTotal 不符时判**产物滞后**，跑 \`cargo check\` 重生成后复跑本门` : "（⇒ 不符时按**真漂移**判）"} · 孤儿注册 ${n.orphanRegistered.length}${n.orphanRegistered.length ? `（首条 ${n.orphanRegistered[0]}；check-contracts 判 warning，不改分流）` : ""}`);
  }
  // 前端反向（任务 #207）：**无条件印**这一行 —— 连 feForeign/feForeignError 双双 undefined 也要印，
  // 否则将来谁把 collectReadings 里的子进程删掉，这行会静默消失（判据死亡的形态恰是「少印一行」）。
  reports.push(feForeignRecomputeReport(register, readings));

  // ③ 逐条登记项
  let machineRows = 0;
  let manualRows = 0;
  for (const e of register.entries) {
    const probs = [];
    const loc = `${e.seamId}（组 ${e.group} · planLine ${e.planLine}）`;
    for (const ev of e.evidence ?? []) {
      const p = adjudicateEvidence(ev, readings.contentFor);
      if (p) { probs.push(`证据 ${ev.ref}：${p}`); }
    }
    if (e.check === "manual") {
      manualRows++;
      manualLines.push(`${loc} —— 未机器复检：${e.reason ?? "原文无可机械复检判据"}${e.expectMissingFields?.length ? `（缺省字段：${e.expectMissingFields.join(" / ")}）` : ""}`);
      continue;
    }
    machineRows++;
    const faces = e.faces ?? [e.seamId];
    if (e.check === "implementorCount" && e.trait) {
      const expected = e.expect.implementors ?? (e.expectImplementorZero ? 0 : null);
      if (expected !== null && e.implementorsHard !== false) {
        if (!readings.implementors) { probs.push(`实现者读数缺失（扫描面未产出，trait=${e.trait}）`); }
        // 站点计数里「键不存在」= 全扫描面零命中 = 实测 0（context.engine / rule.custom / dojo 的 0 值判据正是此形态）
        else {
          const actual = readings.implementors[e.trait] ?? 0;
          if (actual !== expected) { probs.push(`实现者「${e.trait}」：登记 ${expected} / 实测 ${actual} / 差 ${actual - expected}`); }
        }
      }
    }
    const state = e.expect.seam ?? "";
    if (/^(已建|待建|有接缝)/.test(state)) {
      for (const f of faces) {
        const declared = seamDeclared(f, readings.seamRegistryText);
        if (declared === null) { probs.push(`capability_registry.rs 缺失 —— 接缝「${f}」无法核对（跳过 ≠ 通过）`); break; }
        if (state.startsWith("待建") && declared) { probs.push(`接缝「${f}」登记「${state}」/ 实测已在 capability_registry.rs 登记`); }
        if ((state.startsWith("已建") || state.startsWith("有接缝")) && !declared) { probs.push(`接缝「${f}」登记「${state}」/ 实测 capability_registry.rs 无字面量`); }
      }
    }
    if (e.expect.remote === true && !remoteSet.has(e.seamId)) { probs.push("期望可远程 / 实测不在 SUPPORTED_REMOTE_SEAMS"); }
    if (e.expect.remote === false && remoteSet.has(e.seamId)) { probs.push("期望不可远程 / 实测在 SUPPORTED_REMOTE_SEAMS"); }
    if (e.implementorsMechanical !== undefined) {
      reports.push(`[只报告] ${loc}：实现者机械读数 ${e.implementorsMechanical}${e.expect.implementors === null ? "（行内登记值缺省，无登记值可比）" : ` / 登记 ${e.expect.implementors}`} —— ${e.reason ?? "口径不可比"}`);
    }
    if (e.expectMissing) {
      reports.push(`[字段缺省] ${loc}：${(e.expectMissingFields ?? []).join(" / ")} —— 缺省字段不算通过（note：字段缺省=PLAN 原文就缺）`);
    }
    if (probs.length) { hardFails.push(`FAIL: 登记项 ${loc} —— ${probs.join("；")}`); }
    entryResults.push({ ok: !probs.length, txt: loc });
  }

  // ④ PLAN 正文对账
  const plan = checkPlan(register, planText);
  for (const f of plan.fails) { hardFails.push(`FAIL: ${f}`); }
  for (const s of plan.soft) { reports.push(`[只报告] ${s}`); }

  return { hardFails, reports, manualLines, anchorResults, entryResults, planPassed: plan.passed, counts: { total: register.entries.length, machine: machineRows, report: reports.length, manual: manualRows } };
}

/** 前端反向复算对照（任务 #207）。纯函数：读数由 collectReadings 的子进程注入，--selftest 用合成读数
 *  走**同一实现**（禁止两套）。只出报告档文本，永不返回硬失败 —— 锚点 feForeignReverse 本轮是
 *  check:"manual" / hard:false 的软锚点，升 hard 的前置条件在其 note（次序：复现稳定 → 反向假红 → 同批 PLAN）。 */
export function feForeignRecomputeReport(register, readings) {
  const f = readings.feForeign;
  const a = register.anchors.find((x) => x.id === "feForeignReverse") ?? null;
  if (!f) {
    return `[只报告] 前端反向复算未出数（子进程 \`scripts/audit-l1-blockers.mjs --fe-json\`：${readings.feForeignError ?? "读数缺失 —— collectReadings 未产出 feForeign/feForeignError"}）—— 软锚点 feForeignReverse 本轮无对照；跳过 ≠ 通过，也不许借缺数改登记值。`;
  }
  const act = { STOCK: f.stock, OPC: f.opc };
  const fmt = (o) => `${o.points} 处 / ${o.files} 文件`;
  const head = `[只报告] 前端反向（复算器 = audit-l1-blockers.mjs --fe-json 权威 §3 自身，basis=${f.basis ?? "?"}${f.commit ? ` commit=${String(f.commit).slice(0, 7)}` : ""}，不决定退出码）：实测 STOCK ${fmt(f.stock)} · OPC ${fmt(f.opc)}`;
  if (!a) {
    return `${head} —— ⚠ 表内无软锚点 feForeignReverse ⇒ 复算无对照对象（读数已印出；补登记是升 hard 的前置步骤，不许反过来删读数）。`;
  }
  const reg = a.feReadings ?? {};
  const diffs = [];
  for (const dk of ["STOCK", "OPC"]) {
    const r = reg[dk];
    if (!r) { diffs.push(`${dk} 锚点缺登记值（feReadings.${dk}）`); continue; }
    if (r.points !== act[dk].points || r.files !== act[dk].files) {
      diffs.push(`${dk} 登记 ${r.points} 处 / ${r.files} 文件 / 实测 ${act[dk].points} 处 / ${act[dk].files} 文件（差 ${act[dk].points - r.points >= 0 ? "+" : ""}${act[dk].points - r.points} 处 / ${act[dk].files - r.files >= 0 ? "+" : ""}${act[dk].files - r.files} 文件）`);
    }
  }
  const tail = diffs.length
    ? ` ⇒ ⚠ 复算不符：${diffs.join("；")}。本轮是 check:manual / hard:false 的**软锚点 ⇒ 不判红**（先例 k2TraitFaces）；改 PLAN §0.10 镜像行或改登记值前，必须先逐条归因差额（PLAN §125.7 的教训：47/34 腐烂时第五道门全程绿，就是因为这个口径一个锚都没有）。`
    : " ⇒ 复算相符（软锚点登记值与权威复算逐字相等）。";
  return `${head} —— 对照软锚点 feForeignReverse 登记 STOCK ${reg.STOCK ? fmt(reg.STOCK) : "?"} · OPC ${reg.OPC ? fmt(reg.OPC) : "?"}${tail}`;
}

// ── K2 键控分发谓词（任务 #57，2026-11-02）─────────────────────────────────
// 存在理由：PLAN §0.7.2 派生流水里「K1 + K2 = 51 个 trait 面」用的谓词真身在
//   `.tmp/capability-register3.cjs:44-82`（一次性工件，**仓内无处可复跑**）⇒ 数字不可核对，
//   与 #50「三读数无法复现」同病根。本节把它的 4 形态（①keyed-register ②self-id-register
//   ③keyed-get ④factory-match-dyn）逐字入库，并按 `.tmp/res33-trait-adjudication.md` §5.2
//   补 ⑤–⑧ 四类新形态。**①–④ 的正则逐字抄自该工件 ⇒ 与 res33 冻结读数（md5 见其 §6.3）同口径**；
//   未剥注释（与冻结读数一致），这是**已知的高估面**，不在本批改口径（改口径=重算 51 的前置，属 #54）。
// ⚠ 本节的产物**只出数不判红**：读数进 `--dump-readings` 的 `k2.strongHits / k2.weakHits`，
//   对应锚点以 `hard:false` 登记；`k2TraitFaces`（51 / check:manual / hard:false）在本门**一字未动**
//   —— 严禁用新谓词静默重算 51 ⇒ 26（PLAN §0.7.2 明令，升 hard 与改 51 是 #54 与镜像表同批的活）。
// 判据全是**纯函数**：输入 =「相对路径 → 文件内容」的注入映射，零盘 IO ⇒ selftest（合成源码）
//   与 198 对账探针（.tmp/res57-k2-vs-198.mjs，import 本节）都能原地复跑同一套实现。
//
// 形态与档位（res33 §5.2 的四条建议，强弱分档不得合并）：
//   ① keyed-register   `fn register|install|mount|attach*` 的参数窗口含 `dyn T` 且含键参数（KEYARG）⇒ **强**
//   ② self-id-register 同上但无键参数、trait 自带 v1 identity 方法 ⇒ **强**
//   ③ keyed-get        `fn get|lookup|resolve|find*` 签名窗口含 `dyn T` 且含 `&str/String` 形参 ⇒ **强**
//   ④ factory-match-dyn match 块 ≥3 分支且 ≥3 构造点，块内/回溯 14 行出现 `dyn T` ⇒ **强**
//                      ⚠ 与工件的**唯一有意差异**：原 ④ 把 `-> Box<T>` 里的 T 不加校验就记成证据（T 可以不是
//                      trait）；本节按 traitDefs 过滤 ⇒ 非 trait 名不再冒充强档命中。51 的现值本来就不是本节的
//                      产物（本节的强/弱两档只出数），这一刀只会让新读数**更保守**，不改判据形态。
//   ⑤ container-*      容器字段声明 `C<…(Arc|Box|Rc)<dyn T>>`：映射型（HashMap/BTreeMap/DashMap/IndexMap）⇒ **强**；
//                      顺序型（Vec/VecDeque）⇒ **弱**，须与「自述键方法或按 key 迭代筛选」同现才升强
//   ⑥ push-register    `push|add_[a-z_]+|set_[a-z_]+|insert` 动词面的参数窗口（**3 行**）含 `dyn T`，
//                      **且接收者是集合类型** ⇒ **强**（#54 收窄，PLAN §24.4：站点行及其后 5 行内必须出现
//                      `.insert(` / `.push(` / `.push_back(` / `.push_front(` / `.extend(` / `.entry(` /
//                      `vec![` 之一；写进单个结构体字段 / `Option` / `OnceLock` 一律**不计** —— 本仓全局 DI
//                      恰好写成 `set_xxx(Arc<dyn T>)`，动词表命中它但那是**构造期注入口**，不是键控位点）
//   ⑦ self-key-filter  trait 自带 identity 方法（name/id/platform/source_type/transport_type/provider_name）
//                      且存在对 dyn 集合按该键 `.iter()/.find()/.any()` 筛选 ⇒ **弱键控**（可插位点在、键未外提），
//                      **不直接进 51**
//   ⑧ factory-match-impl match 块 ≥3 分支且块内构造 **≥2 个该 trait 的已知实现类型**（impl→trait 反查表），
//                      不再要求块内出现 `dyn` ⇒ **强**
const K2_IDENTITY_V1_METHODS = ["name", "id", "kind", "provider_type", "channel_type", "platform"]; // 逐字 = capability-register3.cjs:29
const K2_SELFKEY_METHODS = ["name", "id", "platform", "source_type", "transport_type", "provider_name"]; // #57 ⑦ 口径
/** identity 方法检测器（v1 与 ⑦ 共用同一构造器 ⇒ 只有一套 `fn X(&self) -> &str/String` 形态，禁区 12）。 */
const k2IdentityRe = (names) => new RegExp(`fn\\s+(${names.join("|")})\\s*\\(\\s*&self\\s*\\)\\s*->\\s*(?:&'?\\s*(?:\\[?]str|\\?)|String)`, "g");
const K2_REGISTER_FN_RE = /\bfn\s+(?:register|install|mount|attach)[a-z0-9_]*\s*[<(]/;
const K2_KEYARG_RE = /(?:name|key|id|provider_type|platform|channel|vendor|engine|kind|backend|r#type|domain_pack_id|kb_id)\s*:\s*(&str|String|&\s*str)/; // 逐字 = :45
const K2_GETFN_RE = /\bfn\s+(?:get|lookup|resolve|find)[a-z0-9_]*\s*[<(]/;
const K2_GETKEY_RE = /:\s*(&str|&\s*str|String)/;
const K2_CONTAINER_RE = /(HashMap|BTreeMap|DashMap|Vec|VecDeque|IndexMap)<[^;]{0,80}?(?:Arc|Box|Rc)<\s*dyn\s+(?:(?:[A-Za-z_][A-Za-z0-9_]*|crate|self|super)::)*([A-Za-z_][A-Za-z0-9_]*)/; // 逐字 = res33-regen.mjs:73 ⇒ 复现冻结的 15 条
const K2_MAP_CONTAINERS = new Set(["HashMap", "BTreeMap", "DashMap", "IndexMap"]);
const K2_PUSH_RE = /\b(?:push|push_back|push_front|add_[a-z_]+|set_[a-z_]+|insert)\s*\(/;
/** ⑥ 的**接收者**判据（PLAN §24.4 定死，token 表勿增删）：集合类型才会同时出现这些标记；
 *  写进 `OnceLock` / `Option` / 单个结构体字段 ⇒ 无此标记 ⇒ 不算键控位点。
 *  ⚠ 它**不改** `K2_PUSH_RE`（动词表）也不改参数窗口行数（仍 3 行）—— #54 的「92 → 36」推算正是
 *  以这两者不变为前提；动了就没法用「推算 vs 真值」的差反判实现与口径是否一致。 */
const K2_COLLECTION_RECV_RE = /\.(?:insert|push|push_back|push_front|extend|entry)\s*\(|\bvec!\[/;
const K2_FILTER_RE = /\.(?:iter|iter_mut|into_iter|find|any)\s*\(/;
const K2_MATCH_HEAD_RE = /^\s*(?:let\s+[a-z_]+\s*:\s*[^=]{0,60}=\s*)?match\s+[^{]{0,80}\{\s*$/;
const K2_ARM_RE = /^\s*(?:"[^"]*"|'[A-Za-z]'|[A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*(?:\([^\)]*\))?)(\s*,)?\s*=>/; // 与工件 :68 同一条，**唯一修正**：把 `(\s*,)?\s*` 从「只作用于第三个候选」提到候选之外 ⇒ `"pdf" =>` 这类字符串臂（工厂分派最常见写法）不再漏计；命中的是**更多**分支，不是更松的档位
const K2_BUILT_RE = /(Box|Arc)::new\(|create_|build_|new\(/;
const K2_CONSTRUCT_RE = /(?:\b(?:Box|Arc|Rc)::new\(\s*|\b)([A-Z][A-Za-z0-9_]*)\s*(?:::\s*new\s*\(|\s*\{)/g;
/** 判为「强键控」的形态集（⑦ 与顺序型 ⑤ 一律不在其中）。 */
const K2_STRONG_FORMS = new Set(["keyed-register", "self-id-register", "keyed-get", "factory-match-dyn", "container-map", "push-register", "factory-match-impl"]);

/** `dyn T` 的机械口径（逐字 = capability-register3.cjs:44）：只认**本表已定义的 trait**。 */
export function k2DynIn(txt, traitDefs) {
  return [...new Set([...txt.matchAll(/\bdyn\s+([A-Za-z_][A-Za-z0-9_]*)/g)].map((m) => m[1]).filter((t) => traitDefs[t]))];
}

/** trait 定义面：`pub trait` + 块体 ⇒ identity 方法（v1 与 ⑦ 两套名单）与方法名。纯函数。 */
export function k2TraitDefs(sources) {
  const defs = {};
  for (const [rel, s] of Object.entries(sources)) {
    for (const m of s.matchAll(/^pub trait ([A-Za-z0-9_]+)([^{]*)\{/gm)) {
      if (defs[m[1]]) { continue; }
      let depth = 0, end = m.index + m[0].length - 1;
      for (; end < s.length; end++) { if (s[end] === "{") { depth++; } else if (s[end] === "}") { depth--; if (!depth) { break; } } }
      const body = s.slice(m.index + m[0].length, end);
      const idV1 = [...body.matchAll(k2IdentityRe(K2_IDENTITY_V1_METHODS))].map((x) => x[1]);
      const selfKeys = [...body.matchAll(k2IdentityRe(K2_SELFKEY_METHODS))].map((x) => x[1]);
      defs[m[1]] = {
        def: `${rel}:${s.slice(0, m.index).split("\n").length}`,
        hasIdentityV1: idV1.length > 0, identityV1: [...new Set(idV1)],
        hasSelfKey: selfKeys.length > 0, selfKeys: [...new Set(selfKeys)],
      };
    }
  }
  return defs;
}

/** impl→trait 反查表（⑧ 的前提）。测试替身/产物路径按 `countRule.implementors` 口径剔除
 *  （复用本门已有的 isTestDoublePath，不另发明第二套「非生产实现」判据）。 */
export function k2ImplPairs(sources, traitDefs) {
  const traitOfImplType = {};
  const implsByTrait = {};
  for (const [rel, s] of Object.entries(sources)) {
    if (isTestDoublePath(rel)) { continue; }
    s.split("\n").forEach((l, i) => {
      const t = implTraitOf(l);
      if (!t || !traitDefs[t]) { return; }
      const ty = implTypeOf(l);
      if (!ty) { return; }
      traitOfImplType[ty] = t;
      (implsByTrait[t] = implsByTrait[t] || []).push({ type: ty, at: `${rel}:${i + 1}` });
    });
  }
  return { traitOfImplType, implsByTrait };
}

/** 逐行证据扫描（①–⑧ 一趟）。返回 `{ trait: [{form, at}] }`。纯函数，输入注入。 */
export function k2Evidence(sources, traitDefs, traitOfImplType) {
  const ev = {};
  const add = (t, form, at) => { (ev[t] = ev[t] || []).push({ form, at }); };
  for (const [rel, s] of Object.entries(sources)) {
    const lines = s.split("\n");
    const win = (from, n) => lines.slice(from, from + n).join(" ").replace(/\s+/g, " ");
    for (let i = 0; i < lines.length; i++) {
      const l = lines[i];
      if (K2_REGISTER_FN_RE.test(l)) {
        const arg = win(i, 6);
        for (const t of k2DynIn(arg, traitDefs)) {
          if (K2_KEYARG_RE.test(arg)) { add(t, "keyed-register", `${rel}:${i + 1}`); } else if (traitDefs[t].hasIdentityV1) { add(t, "self-id-register", `${rel}:${i + 1}`); }
        }
      }
      if (K2_GETFN_RE.test(l)) {
        const sig = win(i, 5);
        for (const t of k2DynIn(sig, traitDefs)) { if (K2_GETKEY_RE.test(sig)) { add(t, "keyed-get", `${rel}:${i + 1}`); } }
      }
      const cm = l.match(K2_CONTAINER_RE);
      if (cm && traitDefs[cm[2]]) { add(cm[2], K2_MAP_CONTAINERS.has(cm[1]) ? "container-map" : "container-seq", `${rel}:${i + 1}`); }
      if (K2_PUSH_RE.test(l)) {
        const arg = win(i, 3); // 参数窗口**保持 3 行**（收窄只加接收者判据，不动窗口与动词表，理由见 K2_COLLECTION_RECV_RE 头注）
        if (K2_COLLECTION_RECV_RE.test(win(i, 6))) { // 接收者判据：站点行 + 其后 5 行
          for (const t of k2DynIn(arg, traitDefs)) { add(t, "push-register", `${rel}:${i + 1}`); }
        }
      }
      if (K2_FILTER_RE.test(l)) {
        // ⑦ 的窗口**向前也向后**（前 6 后 3）：真实的「按自述键筛选」常写成
        // 「上面几行声明 `Vec<Arc<dyn T>>` / 参数 `&[Box<dyn T>]`，下面 `.iter().find(|x| x.platform()==k)`」，
        // 只看当前行会把 ⑦ 判成 0 命中（res33 §5.1 的 MarketplaceScanner 形态正是这种跨行写法）。
        const w = lines.slice(Math.max(0, i - 6), i + 4).join(" ").replace(/\s+/g, " ");
        for (const t of k2DynIn(w, traitDefs)) {
          const d = traitDefs[t];
          if (d.hasSelfKey && d.selfKeys.some((k) => new RegExp(`\\.\\s*${k}\\s*\\(`).test(w))) { add(t, "self-key-filter", `${rel}:${i + 1}`); }
        }
      }
      if (K2_MATCH_HEAD_RE.test(l)) {
        let depth = 0, j = i, arms = 0, built = 0;
        const constructed = new Set();
        for (; j < Math.min(i + 130, lines.length); j++) {
          depth += (lines[j].match(/[{(]/g) || []).length - (lines[j].match(/[})]/g) || []).length;
          if (K2_ARM_RE.test(lines[j])) { arms++; }
          if (K2_BUILT_RE.test(lines[j])) { built++; }
          for (const c of lines[j].matchAll(K2_CONSTRUCT_RE)) { if (traitOfImplType[c[1]]) { constructed.add(c[1]); } }
          if (j > i && depth <= 0) { break; }
        }
        if (arms >= 3 && built >= 3) { // ④ 原文形态（含 `-> Box<T>` 具体返回型这一支），逐字保留
          const block = lines.slice(i, j + 1).join(" ");
          const head = lines.slice(Math.max(0, i - 14), i + 1).join(" ");
          const cands = [...new Set([...k2DynIn(block, traitDefs), ...k2DynIn(head, traitDefs),
            ...[...(block + " " + head).matchAll(/->\s*(?:anyhow::Result<\s*)?(?:Box|Arc|Rc)<\s*([A-Z][A-Za-z0-9_]*)\s*>/g)].map((x) => x[1]).filter((t) => traitDefs[t])])];
          for (const t of cands) { add(t, "factory-match-dyn", `${rel}:${i + 1}`); }
        }
        if (arms >= 3 && constructed.size >= 2) { // ⑧ 新形态：块内构造 ≥2 个同一 trait 的已知实现类型
          const byTrait = {};
          for (const ty of constructed) { (byTrait[traitOfImplType[ty]] = byTrait[traitOfImplType[ty]] || new Set()).add(ty); }
          for (const [t, set] of Object.entries(byTrait)) { if (set.size >= 2) { add(t, "factory-match-impl", `${rel}:${i + 1}(构造 ${set.size} 个实现类型/${arms} 分支)`); } }
        }
        i = Math.max(i, j - 1);
      }
    }
  }
  return ev;
}

/** 分档裁决：强弱两档互斥（同一 trait 有强证据即只计强档），⑦ 永不单独进强档。纯函数。 */
export function adjudicateK2(evByTrait, traitDefs) {
  const strong = [];
  const weak = [];
  const formsOf = {};
  for (const [t, list] of Object.entries(evByTrait)) {
    if (!traitDefs[t]) { continue; } // 证据表的键必须是 trait（非 trait 名不配占档位，见上方 ④ 的 traitDefs 过滤）
    const forms = new Set(list.map((e) => e.form));
    formsOf[t] = [...forms].sort().join("+");
    const selfKeyFilter = forms.has("self-key-filter");
    let isStrong = [...forms].some((f) => K2_STRONG_FORMS.has(f));
    // ⑤ 顺序型容器：单飞算弱；与「自述键方法」或「按 key 迭代筛选」同现才升强
    if (!isStrong && forms.has("container-seq") && (selfKeyFilter || (traitDefs[t] && traitDefs[t].hasSelfKey))) { isStrong = true; }
    if (isStrong) { strong.push(t); } else if (forms.has("container-seq") || selfKeyFilter) { weak.push(t); }
  }
  return { strongTraits: strong.sort(), weakTraits: weak.sort(), formsOf };
}

/** K2 读数总入口（collectReadings / 对账探针 / selftest 共用同一实现）。纯函数，零盘 IO。 */
export function computeK2Readings(sources) {
  const traitDefs = k2TraitDefs(sources);
  const { traitOfImplType, implsByTrait } = k2ImplPairs(sources, traitDefs);
  const evByTrait = k2Evidence(sources, traitDefs, traitOfImplType);
  const { strongTraits, weakTraits, formsOf } = adjudicateK2(evByTrait, traitDefs);
  const formCounts = {};
  for (const list of Object.values(evByTrait)) { for (const e of list) { formCounts[e.form] = (formCounts[e.form] || 0) + 1; } }
  return {
    strongHits: strongTraits.length, weakHits: weakTraits.length,
    strongTraits, weakTraits, formCounts,
    scannedTraits: Object.keys(traitDefs).length,
    implTypesKnown: Object.keys(traitOfImplType).length,
    detail: { traitDefs, traitOfImplType, implsByTrait, evByTrait, formsOf },
  };
}

// ── 产物滞后 vs 真漂移（任务 #58，2026-11-02）─────────────────────────────
// 背景：本门跑在 CI job `workspace-config-parity`（`.github/workflows/ci.yml:1026`），**该 job 不装
//   Rust 工具链、前面没有任何重生成步骤** ⇒ 干净检出读到的是**提交态构建产物** `register_commands.rs`。
//   HEAD 树 `git archive` 复跑实测 = 5 条硬失败，首条 `commandTotal 登记 1542 / 实测 1540 / 差 −2`，
//   而工作区产物是 1542 ⇒ 同一份表「本机绿 / CI 红」。根因不是登记错，是**产物没跟着源码重生成**。
// 判据（零新解析器）：比 commandTotal 之前，先跑**仓内已有、已接线的**契约 A 名字集合谓词
//   （`scripts/check-contracts.mjs:95-121` 的 `defined ⊆ registered`）——
//   · defined 侧正则逐字抄自该处（禁区 12：与 DOMAINS 抄 audit-l1-blockers 同一形态的**唯一合法同步点**，
//     改那边必须同改这里；不 import 该脚本是因为它顶层即扫描 + `process.exit`，import 会破坏本模块
//     「被 import 时零副作用」的规矩）；
//   · registered 侧**强制复用** `lib/command-domains.parseRegistry`（本门已有的唯一注册表解析口径）。
// 分档（诚实边界）：专码只买到**诊断正确性**，**仍是红** —— `ci-check.mjs:114-137` 的 `step()`
//   非零即中断，GitHub Actions 里一样红。不伪装成绿、不放宽 commandTotal 的现值/现口径。
const CONTRACT_A_DEF_RE = /#\[(?:tauri::)?command\][\s\S]*?(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)/g; // 逐字 = check-contracts.mjs:106

/** 契约 A 名字集合（纯函数）：`regEntries` 来自 parseRegistry，`sourceTexts` = [[rel, 原文], …]。
 *  与 check-contracts 的 A 档同口径：先剥行注释再找 `#[tauri::command] … fn 名`。 */
export function commandNameSets(regEntries, sourceTexts) {
  const registered = new Set(regEntries.map((e) => e.cmd));
  const defined = new Set();
  for (const [, raw] of sourceTexts) {
    const srcClean = raw.replace(/\/\/.*$/gm, "");
    for (const m of srcClean.matchAll(CONTRACT_A_DEF_RE)) { defined.add(m[1]); }
  }
  const unregistered = [...defined].filter((n) => !registered.has(n)).sort();
  const orphanRegistered = [...registered].filter((n) => !defined.has(n)).sort();
  return { definedCount: defined.size, registeredNameCount: registered.size, entriesCount: regEntries.length, unregistered, orphanRegistered };
}

/** 命令数不符时的分流（纯函数）：**只有**「已定义但未注册」非空才判产物滞后（fail-closed）；
 *  孤儿注册（注册了但源码无定义）按 check-contracts 的既有分档只算 warning ⇒ 不足以推翻「真漂移」。 */
export function classifyCommandCountDrift(names) {
  if (!names) { return { kind: "unknown", code: "", note: "契约 A 名字集合读数缺失 ⇒ 无法分流，按原口径判" }; }
  if (names.unregistered.length) {
    return {
      kind: "artifact-stale",
      code: "[产物滞后 ARTIFACT-STALE] ",
      note: `契约 A 名集合谓词（与 check-contracts.mjs:105-116 同口径）测出 ${names.unregistered.length} 条 \`#[tauri::command]\` 已在源码定义、却不在 register_commands.rs 里（未注册首条：${names.unregistered[0]}）⇒ 判为**构建产物滞后**，不是登记值漂移。修法：跑 \`cargo check\`（build.rs 重生成 src/register_commands.rs）后复跑本门。`,
      hint: "cargo check",
    };
  }
  return {
    kind: "true-drift",
    code: "[真漂移 TRUE-DRIFT] ",
    note: `契约 A 名集合与产物一致（defined ${names.definedCount} / 已注册名 ${names.registeredNameCount} / 未注册 0${names.orphanRegistered.length ? ` / 孤儿注册 ${names.orphanRegistered.length}（check-contracts 判 warning，不改变本分流）` : ""}）⇒ 差额按**登记值漂移**处理：改代码或同批改表 + PLAN §0.7。`,
    hint: null,
  };
}

// ── --repin-plan（任务 #48）：按定位键把「PLAN 行号引用」重锚到当前文本 ──────────
// 判据与红线见文件头「--repin-plan 的由来与判据」段。以下四个是**纯函数**（selftest 与正式跑共用同一实现）：
// 定位（pinHitLines / textHitLines）⇒ 汇总（collectPlanLineRefs）⇒ 计算（computeRepin，只给方案不写盘）
// ⇒ 回写（applyRepinToRegister 内存版 + planRepinPatches 文本版）。

/** 整行归一：去首尾空白、行内连续空白折叠成一个空格 —— 只容排版差异，不容内容差异。
 *  （--repin-plan 的 planAnchorText 与 --repin-code 的 evidence.anchorText **共用这一口径**）*/
export function normPlanLine(s) {
  return String(s ?? "").trim().replace(/[\t ]+/g, " ");
}

/** 钉句候选行：mustContain 的**全部** token 落在同一行才算命中（与 checkPlan 同一判据形态）。 */
export function pinHitLines(planLines, mustContain) {
  const toks = (mustContain ?? []).filter((t) => String(t).length > 0);
  if (!toks.length) { return []; }
  return planLines.map((l, i) => (toks.every((t) => l.includes(t)) ? i + 1 : 0)).filter(Boolean);
}

/** 整行逐字（归一后）全等才算命中 —— 名字里的 planLines 指「被检索那份文本的行数组」，PLAN 与源码文件同用。 */
export function textHitLines(planLines, anchorText) {
  const t = normPlanLine(anchorText);
  if (!t) { return []; }
  return planLines.map((l, i) => (normPlanLine(l) === t ? i + 1 : 0)).filter(Boolean);
}

/** 全部「受行号约束」的 PLAN 引用（三类），各带自己的定位键与写回坐标。 */
export function collectPlanLineRefs(register) {
  const refs = [];
  for (const a of register.anchors ?? []) {
    (a.planPinned ?? []).forEach((p, i) => {
      refs.push({ kind: "planPinned", owner: a.id, idx: i, line: p.line, mustContain: p.mustContain ?? [], anchorText: null, path: `anchors[${a.id}].planPinned[${i}].line` });
    });
    // alsoAt 区段是**成对**的两个行号（一个区间），随文档整体位移一起走 ⇒ 一个引用一次回写，
    // 不拆成两条独立的 planLine（拆开会允许「只挪对一半」，那是新造的假绿面）。
    (a.planOld ?? []).forEach((o, i) => {
      (o.alsoAt ?? []).forEach((rg, j) => {
        refs.push({ kind: "alsoAtRegion", owner: a.id, oldIdx: i, idx: j, number: o.number, line: rg.startLine, endLine: rg.endLine, mustContain: rg.mustContain ?? [], anchorText: null, path: `anchors[${a.id}].planOld[${i}].alsoAt[${j}].startLine/endLine` });
      });
    });
  }
  for (const e of register.entries ?? []) {
    if (e.planLine !== undefined && e.planLine !== null) {
      refs.push({ kind: "planLine", owner: e.seamId, idx: null, line: e.planLine, mustContain: null, anchorText: e.planAnchorText ?? null, path: `entries[${e.seamId}].planLine` });
    }
    (e.planLineRange ?? []).forEach((n, i) => {
      refs.push({ kind: "planLineRange", owner: e.seamId, idx: i, line: n, mustContain: null, anchorText: (e.planAnchorTextRange ?? [])[i] ?? null, path: `entries[${e.seamId}].planLineRange[${i}]` });
    });
  }
  return refs;
}

/** 计算重锚方案（零写盘）。changes=可回写项 · blockers=歧义/零命中（整次拒绝）· noKey=缺定位键（同拒）。 */
export function computeRepin(register, planText) {
  const planLines = String(planText ?? "").split("\n");
  const changes = [];
  const blockers = [];
  const noKey = [];
  let kept = 0;
  const refs = collectPlanLineRefs(register);
  for (const r of refs) {
    if (r.kind === "alsoAtRegion") {
      // 区段重锚（任务 #65）：与 planPinned 同口径的三档 fail-closed（唯一定位才回写 · 零命中与歧义拒写 ·
      // 缺定位键拒写），差别只在定位键是**整段**的 token 集合而不是单行 —— 判据形态（写哪些数字）见下面
      // 「最小包络」注释与文件头 --repin-plan 段。
      if (!r.mustContain.length) { noKey.push(`${r.path} = ${r.line}–${r.endLine}：alsoAt 区段无 mustContain 自证 token ⇒ 无定位键（新增区段必须同批带键，否则重锚能力随表腐烂）`); continue; }
      const st = alsoAtRegionStatus(planLines, { startLine: r.line, endLine: r.endLine, mustContain: r.mustContain }, r.number);
      if (st.ok) { kept++; continue; }
      const hOK = Number.isInteger(r.line) && Number.isInteger(r.endLine) && r.endLine >= r.line;
      if (!hOK) { blockers.push(`${r.path} = ${r.line}–${r.endLine}：${st.reason} ⇒ 原区段连行号都不成立 ⇒ 不许猜边界，请人工改表`); continue; }
      // 定位键 = mustContain 的**最小包络窗口**（含全部 token，且首行/末行各携带一个「窗口内别处没有」的
      // token ⇒ 按包含关系不可再缩）。为什么不是「保持段高」：位移之后，段高是**不可知**的冗余信息
      // （登记行号上此刻坐着上一段的尾巴），而 token 集合仍在盘上；实测现盘 §15.1 把 `1540/1541/HEAD`
      // 挤在同一行 ⇒ 「同高且含全部 token」的窗口恒有 段高+1 个（贴着该行的每一个窗口都算），
      // 必然歧义、必然拒写 ⇒ 区段成了一个不能重锚的钉（#48 修的正是这个病）。
      // 代价：重锚会把段**收紧到 token 的最小包络**（只朝更严的方向动，且 mustContain 一字未动），
      // 回写时逐条打印「旧段→新段 + 段高变化」，由人过目。
      const hitLines = r.mustContain.map((t) => planLines.map((l, i) => (l.includes(t) ? i + 1 : 0)).filter(Boolean));
      const dead = r.mustContain.filter((_, k) => !hitLines[k].length);
      if (dead.length) { blockers.push(`${r.path} = ${r.line}–${r.endLine}：自证 token [${dead.join(" / ")}] 在 PLAN 全文**零命中**（该句已被改写或删除 ⇒ 语义变化不是位移）⇒ 不许猜行、不许插值，请先人工核对该项`); continue; }
      const inRange = (arr, lo, hi) => arr.some((n) => n >= lo && n <= hi);
      const firstAtLeast = (arr, x) => arr.find((n) => n >= x);
      const starts = [...new Set(hitLines.flat())].sort((a, b) => a - b);
      const cands = [];
      const seen = new Set();
      for (const i of starts) {
        const js = hitLines.map((arr) => firstAtLeast(arr, i));
        if (js.some((n) => n === undefined)) { continue; }
        const j = Math.max(...js);
        const necStart = r.mustContain.some((_, k) => inRange(hitLines[k], i, i) && !inRange(hitLines[k], i + 1, j));
        const necEnd = r.mustContain.some((_, k) => inRange(hitLines[k], j, j) && !inRange(hitLines[k], i, j - 1));
        if (!necStart || !necEnd) { continue; }
        const key = `${i}–${j}`;
        if (seen.has(key)) { continue; }
        seen.add(key);
        cands.push({ start: i, end: j });
      }
      if (cands.length === 1) { changes.push({ ...r, to: cands[0].start, toEnd: cands[0].end }); continue; }
      if (cands.length === 0) {
        blockers.push(`${r.path} = ${r.line}–${r.endLine}：PLAN 内**零命中**能同时含自证 token [${r.mustContain.join(" / ")}] 的最小包络（段被改写、或 token 被拆散到多处 ⇒ 无法确定边界）⇒ 不许猜行、不许插值，请先人工核对该项`);
        continue;
      }
      blockers.push(`${r.path} = ${r.line}–${r.endLine}：区段定位键**非唯一命中** ${cands.length} 个最小包络 [${cands.map((c) => `${c.start}–${c.end}`).join(", ")}] ⇒ 请先收紧 mustContain（补一条只有该段才有的句子），不许从里面挑一个`);
      continue;
    }
    const cur = planLines[r.line - 1] ?? "";
    let curOk;
    let hits;
    if (r.kind === "planPinned") {
      if (!r.mustContain.length) { noKey.push(`${r.path} = ${r.line}：无 mustContain token ⇒ 无定位键`); continue; }
      curOk = r.mustContain.every((t) => cur.includes(t));
      hits = curOk ? [r.line] : pinHitLines(planLines, r.mustContain);
    } else {
      if (!r.anchorText) { noKey.push(`${r.path} = ${r.line}：无 planAnchorText 定位键（新增行号引用必须同批带键，否则重锚能力随表腐烂）`); continue; }
      curOk = normPlanLine(cur) === normPlanLine(r.anchorText);
      hits = curOk ? [r.line] : textHitLines(planLines, r.anchorText);
    }
    if (curOk) { kept++; continue; }
    if (hits.length === 1) { changes.push({ ...r, to: hits[0] }); continue; }
    if (hits.length === 0) {
      blockers.push(`${r.path} = ${r.line}：定位键在 PLAN 内**零命中**（该句已被改写或删除）⇒ 不许猜行、不许插值，请先人工核对该项`);
      continue;
    }
    blockers.push(`${r.path} = ${r.line}：定位键**非唯一命中** ${hits.length} 行 [${hits.join(", ")}] ⇒ 请先收紧 mustContain / 用更长的整行文本，不许从里面挑一个`);
  }
  return { changes, blockers, noKey, kept, total: refs.length };
}

/** 内存版回写（也是文本版补丁的对照物：两者必须逐字段一致才允许落盘）。 */
export function applyRepinToRegister(register, changes) {
  const next = JSON.parse(JSON.stringify(register));
  for (const c of changes) {
    if (c.kind === "planPinned") {
      const a = next.anchors.find((x) => x.id === c.owner);
      a.planPinned[c.idx].line = c.to;
    } else if (c.kind === "alsoAtRegion") {
      const a = next.anchors.find((x) => x.id === c.owner);
      const rg = a.planOld[c.oldIdx].alsoAt[c.idx];
      rg.startLine = c.to;
      rg.endLine = c.toEnd;
    } else {
      const e = next.entries.find((x) => x.seamId === c.owner);
      if (c.kind === "planLine") { e.planLine = c.to; } else { e.planLineRange[c.idx] = c.to; }
    }
  }
  return next;
}

function countOcc(hay, needle) {
  let n = 0;
  let i = 0;
  while ((i = hay.indexOf(needle, i)) >= 0) { n++; i += needle.length; }
  return n;
}

const rangeLiteral = (arr) => `[${arr.join(", ")}]`;
const rangeLiteralPretty = (arr) => `[\n        ${arr.join(",\n        ")}\n      ]`;

/**
 * 文本版回写：对每条 change 用「候选 needle 里第一个在全文**恰好出现一次**的」做定点替换
 * （不整体重新序列化 ⇒ 未涉及的字节逐字不动，与并行编辑者互不打散格式）。
 * 任一 change 找不到唯一 needle ⇒ {ok:false, problems}，调用方**不得写盘**。
 * `planLineRange` 的两个端点合并按整个数组字面量替换（否则改完端点 0 后端点 1 的原文就变了）。
 */
export function planRepinPatches(srcText, register, changes) {
  const problems = [];
  const patches = [];
  const groups = new Map();
  for (const c of changes) {
    const key = c.kind === "planLineRange" ? `planLineRange|${c.owner}`
      : c.kind === "alsoAtRegion" ? `alsoAtRegion|${c.owner}|${c.oldIdx}|${c.idx}`
        : `${c.kind}|${c.owner}|${c.idx}`;
    if (!groups.has(key)) { groups.set(key, []); }
    groups.get(key).push(c);
  }
  const lineNumRe = (n) => new RegExp(`"line": ${n}(?!\\d)`, "g");
  const planLineRe = (n) => new RegExp(`"planLine": ${n}(?!\\d)`, "g");
  for (const [key, list] of groups) {
    const kind = key.split("|")[0];
    const owner = list[0].owner;
    let needles;
    let sub;
    if (kind === "alsoAtRegion") {
      // 成对回写：`"startLine": X, "endLine": Y`（`\s` 覆盖换行与缩进 ⇒ 排版怎么排都能定点）。
      // 找不到**恰好一处**就整次拒写 —— 与其它两类行号锚同一口径，宁可人工改表。
      const c = list[0];
      const pairRe = new RegExp(`"startLine":\\s*${c.line}(?!\\d)(\\s*,\\s*)"endLine":\\s*${c.endLine}(?!\\d)`, "g");
      const pair = [...srcText.matchAll(pairRe)];
      if (pair.length === 1) {
        patches.push({ needle: pair[0][0], replacement: `"startLine": ${c.to}${pair[0][1]}"endLine": ${c.toEnd}`, paths: [c.path] });
        continue;
      }
      const sOnly = [...srcText.matchAll(new RegExp(`"startLine":\\s*${c.line}(?!\\d)`, "g"))];
      const eOnly = [...srcText.matchAll(new RegExp(`"endLine":\\s*${c.endLine}(?!\\d)`, "g"))];
      if (sOnly.length === 1 && eOnly.length === 1) {
        patches.push({ needle: sOnly[0][0], replacement: `"startLine": ${c.to}`, paths: [`${c.path}（startLine）`] });
        patches.push({ needle: eOnly[0][0], replacement: `"endLine": ${c.toEnd}`, paths: [`${c.path}（endLine）`] });
        continue;
      }
      problems.push(`${c.path} ${c.line}–${c.endLine}→${c.to}–${c.toEnd}：镜像表原文里找不到唯一的替换锚（排版被重排过、或该引用不止一处）⇒ 拒绝定点写盘，请人工改表`);
      continue;
    }
    if (kind === "planPinned") {
      const c = list[0];
      const items = c.mustContain.map((t) => JSON.stringify(t)).join(", ");
      needles = [`{ "line": ${c.line}, "mustContain": [${items}] }`, `"line": ${c.line}, "mustContain": [${items}]`];
      sub = { re: lineNumRe(c.line), to: `"line": ${c.to}` };
    } else if (kind === "planLine") {
      const c = list[0];
      const at = JSON.stringify(c.anchorText);
      needles = [`"planAnchorText": ${at}, "planLine": ${c.line}`, `"planAnchorText": ${at},\n      "planLine": ${c.line}`, `"planLine": ${c.line}`];
      sub = { re: planLineRe(c.line), to: `"planLine": ${c.to}` };
    } else {
      const e = (register.entries ?? []).find((x) => x.seamId === owner);
      const oldArr = [...(e?.planLineRange ?? [])];
      const newArr = oldArr.map((n, i) => list.find((c) => c.idx === i)?.to ?? n);
      if (!list.length || !oldArr.length) { problems.push(`${owner} planLineRange：登记表里没有该区间，无法定点替换`); continue; }
      if (rangeLiteral(oldArr) === rangeLiteral(newArr)) { continue; }
      needles = [`"planLineRange": ${rangeLiteral(oldArr)}`, `"planLineRange": ${rangeLiteralPretty(oldArr)}`];
      sub = { literal: rangeLiteral(oldArr), to: rangeLiteral(newArr), literalPretty: rangeLiteralPretty(oldArr), toPretty: rangeLiteralPretty(newArr) };
    }
    const hit = needles.find((n) => countOcc(srcText, n) === 1);
    if (!hit) {
      problems.push(`${list.map((c) => `${c.path} ${c.line}→${c.to}`).join(" / ")}：镜像表原文里找不到唯一的替换锚（排版被重排过、或该引用不止一处）⇒ 拒绝定点写盘，请人工改表`);
      continue;
    }
    let replacement;
    if (sub.literal !== undefined) { replacement = hit.includes(sub.literal) ? hit.replace(sub.literal, sub.to) : hit.replace(sub.literalPretty, sub.toPretty); } else { replacement = hit.replace(sub.re, sub.to); }
    if (replacement === hit) { problems.push(`${list.map((c) => c.path).join(" / ")}：替换锚命中但行号没被改动（判据落空）⇒ 拒绝写盘`); continue; }
    patches.push({ needle: hit, replacement, paths: list.map((c) => c.path) });
  }
  if (problems.length) { return { ok: false, problems, patches: [], text: null }; }
  let out = srcText;
  for (const p of patches) {
    const at = out.indexOf(p.needle);
    out = `${out.slice(0, at)}${p.replacement}${out.slice(at + p.needle.length)}`;
  }
  let parsed;
  try {
    parsed = JSON.parse(out);
  } catch (e) {
    return { ok: false, problems: [`定点替换后 JSON 解析失败（说明替换落到了字符串字面量里）：${e.message}`], patches, text: null };
  }
  const expect = applyRepinToRegister(register, changes);
  if (JSON.stringify(parsed) !== JSON.stringify(expect)) {
    return { ok: false, problems: ["文本版回写与内存版回写结果不一致 ⇒ 定点替换的把握不足，拒绝写盘（宁可人工改表）"], patches, text: null };
  }
  return { ok: true, text: out, problems: [], patches };
}

/** 只保留「非行号」字段 —— 用于断言重锚**只改行号**，判据（token/读数/分组）一个字都没松。 */
export function stripRepinEditableFields(register) {
  const clone = JSON.parse(JSON.stringify(register));
  for (const a of clone.anchors ?? []) {
    for (const p of a.planPinned ?? []) { p.line = null; }
    for (const o of a.planOld ?? []) { for (const rg of o.alsoAt ?? []) { rg.startLine = null; rg.endLine = null; } }
  }
  for (const e of clone.entries ?? []) {
    if (e.planLine !== undefined) { e.planLine = null; }
    if (e.planLineRange) { e.planLineRange = e.planLineRange.map(() => null); }
  }
  return clone;
}

// ── --repin-code（任务 #55）：按整行定位键把「代码证据锚点 evidence[].line」重锚到当前文本 ──────
// 判据与红线见文件头「--repin-code 的由来与判据」段。以下全是**纯函数**（--selftest 与正式跑共用同一实现，
// 禁止两套）：收集（collectCodeLineRefs）⇒ 计算（computeCodeRepin，只给方案不写盘）⇒ 回写
// （applyCodeRepinToRegister 内存版 + codeRepinPatches 文本版）⇒ 落盘工序（executeCodeRepin，IO 注入 ⇒
// 自检在内存假盘上跑完整的「备份 / 写盘 / 复检 / 回滚」，零真盘）。
// ⚠ 定位键是 `evidence[].anchorText`（登记当时的整行逐字，仅折叠行内连续空白，归一口径同 normPlanLine），
//   **不是 token**：现盘 `workflow.evolver` 两条 evidence 用同一个 token 指向同文件的 :1551 与 :1714 ⇒
//   按 token 全文检索必然多命中；token 是**判据**（「第 N 行含该串」），不是身份键。
//   token 的命中行号在报告里只作**诊断信息**打印（告诉人该补哪一行的整行文本），永不参与放行。
// ⚠ 本模式**只重锚 line 与 ref 末尾的 :行号**，其它字段（token / file / endLine / role / anchorText）
//   一个字都不动 —— 由 stripCodeRepinEditableFields + 写盘后从盘上复检双重锁定。

/** 受行号约束的 evidence 清单 + 本模式覆盖不到的条目（逐条给理由，不装作覆盖到了）。 */
export function collectCodeLineRefs(register) {
  const refs = [];
  const skipped = [];
  for (const e of register.entries ?? []) {
    (e.evidence ?? []).forEach((ev, i) => {
      const path = `entries[${e.seamId}].evidence[${i}].line`;
      if (!ev || !ev.line) { skipped.push({ path, reason: "文件级证据（无 line，走 fileLevel 判据）⇒ 不受行号约束，本模式不动" }); return; }
      if (ev.endLine) { skipped.push({ path, reason: `区间锚点 ${ev.line}–${ev.endLine}：两端位移量要人工判定（区间内任一行含 token 即算绿）⇒ 本模式不重锚` }); return; }
      if (!ev.token) { skipped.push({ path, reason: "无 token ⇒ 判定档本就不检查该行内容，没有任何键可定位" }); return; }
      refs.push({ kind: "evidence", owner: e.seamId, idx: i, path, file: ev.file, line: ev.line, token: ev.token, ref: ev.ref ?? null, anchorText: ev.anchorText ?? null });
    });
  }
  return { refs, skipped };
}

/** token 在该文件内的命中行号（子串口径，与 adjudicateEvidence 逐字一致；只用于诊断打印，不参与放行）。 */
export function tokenHitLines(fileLines, token) {
  const t = String(token ?? "");
  if (!t) { return []; }
  return fileLines.map((l, i) => (l.includes(t) ? i + 1 : 0)).filter(Boolean);
}

/** 计算代码锚点重锚方案（零写盘）。changes=可回写 · blockers=零命中/歧义/文件缺失（整次拒绝）· noKey=缺定位键（同拒）。 */
export function computeCodeRepin(register, fileTextFor) {
  const { refs, skipped } = collectCodeLineRefs(register);
  const linesCache = new Map();
  const linesOf = (rel) => {
    if (!linesCache.has(rel)) {
      const txt = fileTextFor(rel);
      linesCache.set(rel, txt === null || txt === undefined ? null : String(txt).split("\n"));
    }
    return linesCache.get(rel);
  };
  const changes = [];
  const blockers = [];
  const noKey = [];
  let kept = 0;
  for (const r of refs) {
    const lines = linesOf(r.file);
    if (lines === null) { blockers.push(`${r.path} = ${r.line}：文件 ${r.file} 读不出（缺失或不可读）⇒ 无文本可定位，拒绝重锚 —— 先核对文件是否被移动/删除`); continue; }
    const cur = lines[r.line - 1] ?? "";
    // 「相符」的口径与判定档逐字一致：行含 token，且**有 anchorText 时整行相符**。
    // 只判 token 会把「同一 token 的另一处调用」当成原锚点（#131 的端点静默错指）。
    if (
      r.line <= lines.length && cur.includes(r.token) &&
      (!r.anchorText || normPlanLine(cur) === normPlanLine(r.anchorText))
    ) { kept++; continue; }
    const hits = tokenHitLines(lines, r.token);
    const diag = `token「${r.token}」在 ${r.file} 内命中 ${hits.length} 行${hits.length <= 12 ? ` [${hits.join(", ")}]` : "（过多，未列出）"}`;
    if (!r.anchorText) {
      noKey.push(`${r.path} = ${r.line}（${r.file}）：判定档已红（第 ${r.line} 行「${normPlanLine(cur).slice(0, 60) || "<空行>"}」不含 token）+ **无 anchorText 定位键** ⇒ 拒写。诊断：${diag}`);
      continue;
    }
    const found = textHitLines(lines, r.anchorText);
    if (found.length === 1) { changes.push({ ...r, to: found[0] }); continue; }
    if (found.length === 0) { blockers.push(`${r.path} = ${r.line}（${r.file}）：anchorText 整行定位键在文件内**零命中** ⇒ 该行已被改写或删除（语义变化，不是位移），不许猜行、不许插值。诊断：${diag}`); continue; }
    blockers.push(`${r.path} = ${r.line}（${r.file}）：anchorText **非唯一命中** ${found.length} 行 [${found.join(", ")}] ⇒ 请用更长的整行文本作键，不许从里面挑一个。诊断：${diag}`);
  }
  return { changes, blockers, noKey, skipped, kept, total: refs.length };
}

/** evidence.ref 末尾的 `:行号` 尾巴跟着 line 一起走；尾部数字与旧行号不符（区间写法 / 无行号）时**原样不动**。 */
export function evidenceRefAfterRepin(ref, from, to) {
  const s = String(ref ?? "");
  const m = /(\d+)$/.exec(s);
  return m && m[1] === String(from) ? `${s.slice(0, m.index)}${to}` : s;
}

/** 内存版回写（也是文本版补丁的对照物：两者必须逐字段一致才允许落盘）。 */
export function applyCodeRepinToRegister(register, changes) {
  const next = JSON.parse(JSON.stringify(register));
  for (const c of changes) {
    const e = (next.entries ?? []).find((x) => x.seamId === c.owner);
    const ev = e?.evidence?.[c.idx];
    if (!ev) { continue; } // 表里没有该坐标 ⇒ 不改；文本版若真改了什么，下面的逐字段比对会拦住
    ev.ref = evidenceRefAfterRepin(ev.ref, c.line, c.to);
    ev.line = c.to;
  }
  return next;
}

/** 只把「本模式有权改动的东西」抹平 —— 用于断言重锚**只改行号**（含 ref 的 :行号 尾巴），判据一字未松。 */
export function stripCodeRepinEditableFields(register) {
  const clone = JSON.parse(JSON.stringify(register));
  for (const e of clone.entries ?? []) {
    for (const ev of e.evidence ?? []) {
      if (ev.line !== undefined) { ev.line = null; }
      if (typeof ev.ref === "string") { ev.ref = ev.ref.replace(/:\d+$/, ":<行号>"); }
    }
  }
  return clone;
}

/** 替换锚的定位：命中且**其后不接数字**（防 `"line": 5` 落进 `"line": 55` 里）。≥0 = 唯一位置，-1 = 多处，-2 = 没有。 */
function findUniqueNeedleAt(src, needle) {
  const at = (from) => {
    let i = src.indexOf(needle, from);
    while (i >= 0) {
      const after = src[i + needle.length];
      if (after === undefined || !/\d/.test(after)) { return i; }
      i = src.indexOf(needle, i + needle.length);
    }
    return -1;
  };
  const first = at(0);
  if (first < 0) { return -2; }
  return at(first + needle.length) >= 0 ? -1 : first;
}

/**
 * 文本版回写：对每条 change 取「候选替换锚里第一个在全文**恰好出现一次**的」做定点补丁
 * （不整体 `JSON.stringify` 重写 ⇒ 未涉及的字节逐字不动；另有会话正在编辑同一张表，整体重排会互相打散）。
 * 任一 change 找不到唯一锚 ⇒ {ok:false}，调用方**不得写盘**（不许只改能定位的那一半）。
 * 候选按精确度降序：ref+file+line 三段相邻（同一行 / 换行 + 8·6·10 空格缩进）⇒ 兜底 `"line": N`；
 * 兜底档不动 ref，会让文本版与内存版不一致 ⇒ 由最后的逐字段比对拦下（负控㉒f 锁这条）。
 */
export function codeRepinPatches(srcText, register, changes) {
  const problems = [];
  const patches = [];
  const groups = new Map();
  for (const c of changes) {
    const key = c.path ?? `entries[${c.owner}].evidence[${c.idx}].line`;
    if (!groups.has(key)) { groups.set(key, []); }
    groups.get(key).push(c);
  }
  for (const [path, list] of groups) {
    if (list.length > 1) { problems.push(`${path}：同一坐标出现 ${list.length} 条回写（${list.map((c) => `${c.line}→${c.to}`).join(" / ")}）⇒ 方案自身不自洽，拒绝写盘`); continue; }
    const c = list[0];
    const refJson = JSON.stringify(String(c.ref ?? ""));
    const refToJson = JSON.stringify(evidenceRefAfterRepin(String(c.ref ?? ""), c.line, c.to));
    const fileJson = JSON.stringify(c.file);
    const cand = [];
    for (const sep of [" ", "\n        ", "\n      ", "\n          "]) {
      cand.push({ needle: `"ref": ${refJson},${sep}"file": ${fileJson},${sep}"line": ${c.line}`, replacement: `"ref": ${refToJson},${sep}"file": ${fileJson},${sep}"line": ${c.to}` });
    }
    cand.push({ needle: `"line": ${c.line}`, replacement: `"line": ${c.to}` });
    const scored = cand.map((x) => ({ ...x, at: findUniqueNeedleAt(srcText, x.needle) }));
    const hit = scored.find((x) => x.at >= 0);
    if (!hit) { problems.push(`${path} ${c.line}→${c.to}：镜像表原文里找不到唯一的替换锚（排版被重排过、或该引用不止一处）⇒ 拒绝定点写盘，请人工改表`); continue; }
    if (hit.replacement === hit.needle) { problems.push(`${path}：替换锚命中但行号没被改动（判据落空）⇒ 拒绝写盘`); continue; }
    patches.push({ ...hit, paths: [path] });
  }
  if (problems.length) { return { ok: false, problems, patches: [], text: null }; }
  const sorted = [...patches].sort((a, b) => a.at - b.at);
  for (let i = 1; i < sorted.length; i++) {
    if (sorted[i].at < sorted[i - 1].at + sorted[i - 1].needle.length) { problems.push(`${sorted[i].paths.join(" / ")}：两个替换锚在原文里重叠 ⇒ 定点补丁无法互不干扰落位，拒绝写盘`); }
  }
  if (problems.length) { return { ok: false, problems, patches: [], text: null }; }
  let out = srcText;
  for (const p of [...sorted].reverse()) { out = `${out.slice(0, p.at)}${p.replacement}${out.slice(p.at + p.needle.length)}`; }
  let parsed;
  try {
    parsed = JSON.parse(out);
  } catch (e) {
    return { ok: false, problems: [`定点替换后 JSON 解析失败（说明替换落到了字符串字面量里）：${e.message}`], patches, text: null };
  }
  const expect = applyCodeRepinToRegister(register, changes);
  if (JSON.stringify(parsed) !== JSON.stringify(expect)) {
    return { ok: false, problems: ["文本版回写与内存版回写结果不一致（含 ref 末尾的 :行号）⇒ 定点替换把握不足，拒绝写盘（宁可人工改表）"], patches, text: null };
  }
  return { ok: true, text: out, problems: [], patches };
}

/**
 * 落盘工序（IO 注入 ⇒ --selftest 用内存假盘跑完整流程，零真盘；正式跑与自检共用同一实现）。
 * rp = computeCodeRepin 的结果：**阻断或缺定位键任一条 ⇒ 整次运行拒绝写盘**（规则只写在这里一份）。
 * io = { backup(text)→标签, write(text), reread()→表|"BAD"|null, restore(text), fileTextFor(rel) }
 * status：refused-locate（定位不唯一/缺键，未写盘）· refused-patch（定点补丁把握不足，未写盘）
 *   · dry-run（未写盘）· rolled-back（写过但复检不过，已回滚）· written。
 */
export function executeCodeRepin({ register, srcText, rp, apply, io, emit = (s) => console.log(s) }) {
  const changes = rp.changes;
  if (rp.blockers.length || rp.noKey.length) {
    return { status: "refused-locate", problems: [...rp.blockers, ...rp.noKey], wrote: false, backup: null, patches: [], changes };
  }
  if (!changes.length) { return { status: "nothing-to-write", problems: [], wrote: false, backup: null, patches: [], changes }; }
  const patched = codeRepinPatches(srcText, register, changes);
  if (!patched.ok) { return { status: "refused-patch", problems: patched.problems, wrote: false, backup: null, patches: patched.patches, changes }; }
  if (!apply) { return { status: "dry-run", problems: [], wrote: false, backup: null, patches: patched.patches, changes }; }
  const backup = io.backup(srcText);
  io.write(patched.text);
  const reread = io.reread();
  const problems = [];
  if (reread === null || reread === undefined || reread === "BAD") {
    problems.push("写盘后的表无法解析（JSON 坏在盘上）");
  } else {
    if (JSON.stringify(stripCodeRepinEditableFields(reread)) !== JSON.stringify(stripCodeRepinEditableFields(register))) {
      problems.push("除行号（evidence 的 line 与 ref 末尾 :行号）外有字段被改动 ⇒ 违反本模式前提，判据不许顺手放宽");
    }
    const after = computeCodeRepin(reread, io.fileTextFor);
    if (after.changes.length) { problems.push(`回写后仍有 ${after.changes.length} 条待重锚：${after.changes.map((x) => `${x.path} ${x.line}→${x.to}`).join(" / ")}`); }
    if (after.blockers.length) { problems.push(`回写后仍有 ${after.blockers.length} 条阻断：${after.blockers.join(" | ")}`); }
    if (after.noKey.length) { problems.push(`回写后仍有 ${after.noKey.length} 条缺定位键：${after.noKey.join(" | ")}`); }
  }
  if (problems.length) {
    emit(`复检失败：${problems.join("；")} —— 用备份回滚。`);
    io.restore(srcText);
    return { status: "rolled-back", problems, wrote: true, backup, patches: patched.patches, changes };
  }
  return { status: "written", problems: [], wrote: true, backup, patches: patched.patches, changes };
}

/** 表内正文（note 等）仍引用旧行号的地方 —— 本模式**不改正文**，只点名，交人工同批核对。 */
function codeRepinProseMentions(srcText, c) {
  const base = String(c.file).split("/").pop();
  const out = [];
  srcText.split("\n").forEach((l, i) => {
    if (l.includes(`"line": ${c.line}`)) { return; } // 该 evidence 对象自身（ref + line 同一行），不是正文引用
    if (l.includes(`${base}:${c.line}`) || l.includes(`${c.file}:${c.line}`)) { out.push(i + 1); }
  });
  return out;
}

// ── selftest（纯函数、零盘；负控+正控成对）──────────────────────────────
function fixture() {
  const register = {
    note: "fixture",
    caliber: { capacityRegister: 2, groupCounts: { A: 1, B: 1, C: 0, H: 0, K: 1, D: 0 }, totalRows: 3, bannedPlanPhrases: ["B 11", "登记 27"] },
    sources: { plan: "docs/PLAN-x.md" },
    countRule: {},
    // 每种 anchor check 各配一个锚点（与真实表同档：registryFileLineCount 在真实表是 hard:false）⇒
    // 干净读数（正控）与逐类改读数（负控）都能在同一张 fixture 上成对跑，见 ANCHOR_CHECK_NEGATIVE_CASES
    anchors: [
      { id: "commandTotal", label: "命令总数", value: 10, check: "registryCommandCount", hard: true, planNumber: "10", planMustAppear: true, planPinned: [{ line: 1, mustContain: ["10", "3"] }], planOld: [{ number: "9", note: "旧读数，只许在漂移段" }] },
      { id: "commandModules", label: "命令模块数", value: 3, check: "registryModuleCount", hard: true },
      { id: "unassignedModules", label: "未归属", value: 0, check: "registryUnassigned", hard: true },
      { id: "registryFileLines", label: "注册表行数", value: 5, check: "registryFileLineCount", hard: false },
      { id: "stockBoundary", label: "股票边界命令", value: 4, value2: 2, value2Label: "模块", check: "registryBoundaryCount", domainKey: "STOCK", hard: true },
      { id: "stockDomain", label: "股票域命令", value: 4, value2: 2, value2Label: "模块", check: "registryDomainCount", domainName: "股票域", hard: true },
      // 整域归零形态（2026-11-13 #264 批 4 收口新增）：域在本 fixture 的 auditDomain 里**不存在**，
      // 登记 0/0 ⇒ 正控必须 PASS（域缺失 = 0 条命令）；负控侧把登记改成非 0 ⇒ 域缺失必须红。
      { id: "opcDomainZero", label: "OPC 域命令（整域归零形态）", value: 0, value2: 0, value2Label: "模块", check: "registryDomainCount", domainName: "OPC 域", hard: true },
      { id: "frontendVolume", label: "前端体量", value: 100, value2: 4, value2Label: "文件", check: "frontendVolume", hard: true },
      { id: "frontendDirVolume", label: "前端目录体量", value: 40, value2: 2, value2Label: "文件", check: "frontendDirVolume", dir: "src/x", hard: true },
      { id: "l1Stock", label: "L1 股票阻塞", value: 6, value2: 2, value2Label: "文件", check: "l1ForeignBlockers", domainKey: "STOCK", hard: true },
      { id: "k1Files", label: "K1 .rs 文件数", value: 12, check: "k1RsFileCount", hard: true },
      { id: "k1Traits", label: "K1 pub trait 总数", value: 4, check: "k1PubTraitCount", hard: true },
      // #61 定案档（fixture 侧刻意取 3 ≤ pubTraits 4 ≤ 外部站点面 7，让三条闭合恒等式在合成读数上也成立）
      { id: "k1DeclaredExt", label: "K1 + 有声明面的外部实现者", value: 3, check: "k1DeclaredExternalImplTraitCount", hard: true },
      { id: "remoteSeamCount", label: "远程清单", value: 1, check: "remoteSeamRegistryCount", hard: true },
      { id: "capacityRegister", label: "能力面", value: 2, check: "registerGroupSum", hard: true },
      { id: "k1Ext", label: "外部实现者", value: 99, check: "manual", hard: false, implementorsMechanical: 7 },
      // #57 / #58 新增的两类 check（fixture 侧必须与真实表同档：k2KeyedHits=hard:false、commandNameSetCount=hard:true）
      { id: "k2Keyed", label: "K2 键控面（强/弱）", value: 3, value2: 1, check: "k2KeyedHits", hard: false },
      { id: "commandNameSet", label: "契约 A 名集合=产物条目数", value: 10, check: "commandNameSetCount", hard: true },
      // #306 智能体工具面：fixture 取「宿主 7 条 / 非宿主根 0 条」，与真实表同档 hard:true
      { id: "agentCommandFace", label: "智能体工具面（标注总数 / 非宿主根）", value: 7, value2: 0, value2Label: "条非宿主根", check: "agentCommandFaceCount", hard: true },
      // ⚠ #207 的前端反向软锚点 feForeignReverse **刻意不进通用 fixture**：它 check:"manual"，进 fixture
      //   会被「覆盖度反向校验」认账、逼着新增一类 check ⇒ 与本设计相反（它不占机器复检名额，对照走
      //   feForeignRecomputeReport 的「只报告」档）。它的用例在 ㉙ 系列自配表与读数。
    ],
    entries: [
      { seamId: "a.one", group: "A", trait: "TraitOne", expect: { implementors: 2, seam: "已建", remote: true }, check: "implementorCount", planAnchorText: "登记项一 a.one 已建。", planLine: 5, evidence: [{ ref: "f.rs:3", file: "f.rs", line: 3, token: "pub trait TraitOne" }] },
      { seamId: "b.two", group: "B", trait: null, expect: { implementors: null, seam: "待建", remote: false }, expectMissing: true, expectMissingFields: ["implementors"], check: "seamExists", faces: ["b.two"], planAnchorText: "登记项二 b.two 待建。", planLine: 6, evidence: [], planLineRange: [5, 6], planAnchorTextRange: ["登记项一 a.one 已建。", "登记项二 b.two 待建。"] },
      { seamId: "k.three", group: "K", expect: { implementors: null, seam: null, remote: null }, check: "manual", reason: "无可机械复检判据", planAnchorText: "登记项三 k.three 人工。", planLine: 7, evidence: [] },
    ],
    pipeline: {},
  };
  const readings = {
    registry: { commandTotal: 10, commandModules: 3, unassigned: [], registryFileLines: 5 },
    boundary: { STOCK: { total: 4, modules: 2 } },
    auditDomain: { 股票域: { total: 4, modules: 2 } },
    frontend: { lines: 100, files: 4 },
    frontendDir: { "src/x": { lines: 40, files: 2 } },
    l1: { STOCK: { hits: 6, files: 2 } },
    k1: { rsFiles: 12, pubTraits: 4, externalImplTraits: 7, declaredExternalTraits: 3 },
    k2: { strongHits: 3, weakHits: 1, strongTraits: ["TraitOne", "TraitTwo", "TraitThree"], weakTraits: ["TraitFour"], formCounts: { "container-map": 2, "container-seq": 1, "self-key-filter": 1 }, scannedTraits: 4, implTypesKnown: 2 },
    commandNames: { definedCount: 10, registeredNameCount: 10, entriesCount: 10, unregistered: [], orphanRegistered: [], scannedFiles: 3 },
    agentCommand: { total: 7, host: 7, nonHost: 0, domainCount: 2, byDomain: { finance: 5, general: 2 }, byRoot: { "src-tauri": 7 }, roots: { "src-tauri": { total: 7, byDomain: { finance: 5, general: 2 } } } },
    implementors: { TraitOne: 2 },
    remoteSeams: ["a.one"],
    seamRegistryText: 'const X = "a.one";',
    contentFor: (p) => (p === "f.rs" ? "ln1\nln2\npub trait TraitOne {" : null),
  };
  // 5/6/7 三行是「登记项 planLine 的落点」，供 --repin-plan 的正负对照使用（见 ㉑ 系列）。
  const plan = "前言：能力面 10 条 / 3 模块。\n\n### 0.12.4 漂移列举\n本轮抓出 9 → 10 的数字漂移。\n登记项一 a.one 已建。\n登记项二 b.two 待建。\n登记项三 k.three 人工。\n\n### 0.13 下一节\n别的事。\n";
  return { register, readings, plan };
}

// 逐类 anchor check 的负控清单（第 2 条修复）：每一类都配「登记 ≠ 实测 ⇒ 必须被抓出」的合成读数改造。
// 正控 = 同一 fixture 的干净读数下该锚点必须 PASS/REPORT 且不进 hardFails（见下方正向断言）。
// tier: "hard" ⇒ 差异进 hardFails；"report" ⇒ 该锚点在真实表是 hard:false，差异只许进「只报告」（不许静默绿）。
const ANCHOR_CHECK_NEGATIVE_CASES = [
  { check: "registryCommandCount", id: "commandTotal", tier: "hard", mutate: (f) => { f.readings.registry.commandTotal = 11; } },
  { check: "registryModuleCount", id: "commandModules", tier: "hard", mutate: (f) => { f.readings.registry.commandModules = 4; } },
  { check: "registryUnassigned", id: "unassignedModules", tier: "hard", mutate: (f) => { f.readings.registry.unassigned = ["zz_orphan"]; } },
  { check: "registryFileLineCount", id: "registryFileLines", tier: "report", mutate: (f) => { f.readings.registry.registryFileLines = 6; } },
  { check: "registryBoundaryCount", id: "stockBoundary", tier: "hard", mutate: (f) => { f.readings.boundary.STOCK.total = 5; } },
  { check: "registryBoundaryCount", id: "stockBoundary", tier: "hard", mutate: (f) => { f.readings.boundary.STOCK.modules = 3; } },
  { check: "registryDomainCount", id: "stockDomain", tier: "hard", mutate: (f) => { delete f.readings.auditDomain["股票域"]; } },
  { check: "registryDomainCount", id: "stockDomain", tier: "hard", mutate: (f) => { f.readings.auditDomain["股票域"].modules = 5; } },
  // 整域归零的两面：域消失但登记还留非 0 旧值 ⇒ 必须红（漂移未回写形态，2026-11-13 #264 批 4 收口锁）；
  // 域消失且登记 0/0 ⇒ 必须绿（负控表只收「必须红」的 mutate；绿面由正控⑭的干净读数断言覆盖 ——
  // fixture 的 auditDomain 里本就没有「OPC 域」键，cleanResults 对该锚走「0/0 放行」分支必须 PASS）。
  { check: "registryDomainCount", id: "opcDomainZero", tier: "hard", mutate: (f) => { const a2 = f.register.anchors.find((x) => x.id === "opcDomainZero"); a2.value = 2; a2.planNumber = "2"; } },
  { check: "frontendVolume", id: "frontendVolume", tier: "hard", mutate: (f) => { f.readings.frontend.lines = 101; } },
  { check: "frontendVolume", id: "frontendVolume", tier: "hard", mutate: (f) => { f.readings.frontend.files = 5; } },
  { check: "frontendDirVolume", id: "frontendDirVolume", tier: "hard", mutate: (f) => { f.readings.frontendDir["src/x"].files = 3; } },
  { check: "frontendDirVolume", id: "frontendDirVolume", tier: "hard", mutate: (f) => { delete f.readings.frontendDir["src/x"]; } },
  { check: "l1ForeignBlockers", id: "l1Stock", tier: "hard", mutate: (f) => { f.readings.l1.STOCK.hits = 7; } },
  { check: "l1ForeignBlockers", id: "l1Stock", tier: "hard", mutate: (f) => { f.readings.l1.STOCK.files = 3; } },
  { check: "k1RsFileCount", id: "k1Files", tier: "hard", mutate: (f) => { f.readings.k1.rsFiles = 13; } },
  { check: "k1PubTraitCount", id: "k1Traits", tier: "hard", mutate: (f) => { f.readings.k1.pubTraits = 5; } },
  // #61 定案档：登记≠实测必红（hard:true 机器复检锚点）· 整份 K1 读数缺失必红 · 读数**缺新字段**也必红
  //（后者锁的是「门内没出数却把 check 改成机器种类」这种半落地状态 ⇒ 不许静默按 undefined 比出绿）
  { check: "k1DeclaredExternalImplTraitCount", id: "k1DeclaredExt", tier: "hard", mutate: (f) => { f.readings.k1.declaredExternalTraits = 4; } },
  { check: "k1DeclaredExternalImplTraitCount", id: "k1DeclaredExt", tier: "hard", mutate: (f) => { delete f.readings.k1.declaredExternalTraits; } },
  { check: "k1DeclaredExternalImplTraitCount", id: "k1DeclaredExt", tier: "hard", mutate: (f) => { f.readings.k1 = null; } },
  { check: "k1RsFileCount", id: "k1Files", tier: "hard", mutate: (f) => { f.readings.k1 = null; } },
  { check: "remoteSeamRegistryCount", id: "remoteSeamCount", tier: "hard", mutate: (f) => { f.readings.remoteSeams = ["a.one", "x.extra"]; } },
  { check: "registerGroupSum", id: "capacityRegister", tier: "hard", mutate: (f) => { f.register.anchors.find((a) => a.id === "capacityRegister").value = 3; } },
  { check: "manual", id: "k1Ext", tier: "report", mutate: (f) => { f.readings.k1.externalImplTraits = 99; } },
  // #57 新增 check 种类必须同批带负控+正控（覆盖度反向校验镜像表）；k2KeyedHits 在真实表是 hard:false
  // ⇒ 两档差异只许进「只报告」，**不得**升成硬失败（这正是「只出数不判红」的机械锁）。
  { check: "k2KeyedHits", id: "k2Keyed", tier: "report", mutate: (f) => { f.readings.k2.strongHits = 4; } },
  { check: "k2KeyedHits", id: "k2Keyed", tier: "report", mutate: (f) => { f.readings.k2.weakHits = 2; } },
  { check: "k2KeyedHits", id: "k2Keyed", tier: "report", mutate: (f) => { f.readings.k2 = null; } },
  // #58 新增 check 种类：名集合数 ≠ 产物条目数 ⇒ 硬失败（这条钉的是「A 档的绿等于产物与源码一致」）
  { check: "commandNameSetCount", id: "commandNameSet", tier: "hard", mutate: (f) => { f.readings.commandNames.definedCount = 9; } },
  { check: "commandNameSetCount", id: "commandNameSet", tier: "hard", mutate: (f) => { f.readings.commandNames.entriesCount = 11; } },
  { check: "commandNameSetCount", id: "commandNameSet", tier: "hard", mutate: (f) => { f.readings.commandNames = null; } },
  // #306 智能体工具面：标注数掉一条（= 少一条 LLM 可调用工具）必红 · 非宿主根从 0 变 1 必红
  //   （插件侧接入 inventory 标注是要显式登记的架构变化，不许静默）· 整份读数缺失必红。
  { check: "agentCommandFaceCount", id: "agentCommandFace", tier: "hard", mutate: (f) => { f.readings.agentCommand.total = 6; } },
  { check: "agentCommandFaceCount", id: "agentCommandFace", tier: "hard", mutate: (f) => { f.readings.agentCommand.nonHost = 1; } },
  { check: "agentCommandFaceCount", id: "agentCommandFace", tier: "hard", mutate: (f) => { delete f.readings.agentCommand; } },
];

function runSelftest() {
  let fail = 0;
  const check = (cond, msg) => {
    if (!cond) { console.error(`SELFTEST FAIL：${msg}`); fail++; }
  };
  const clone = (x) => JSON.parse(JSON.stringify(x));
  const has = (r, re) => r.hardFails.some((f) => re.test(f));
  const anchor = (f, id) => f.register.anchors.find((a) => a.id === id);

  // 正控：干净读数不得假阳性
  const f0 = fixture();
  let r = reconcile(f0.register, f0.readings, f0.plan);
  const cleanResults = r.anchorResults;
  check(r.hardFails.length === 0, `正控：干净读数应零硬失败，实得 ${r.hardFails.join(" | ")}`);
  check(r.counts.machine === 2 && r.counts.manual === 1, "正控：机器复检 2 / 未机器复检 1");

  // 负控①：命令数被改
  const f1 = fixture(); f1.readings.registry.commandTotal = 1534;
  r = reconcile(f1.register, f1.readings, f1.plan);
  check(has(r, /commandTotal/), "负控：命令总数被改必须红");

  // 负控②：分组计数漂移（B 10→B 11 的 fixture 版：B 1→2 ⇒ Σ=3≠2 且行数不符）
  const f2 = fixture(); f2.register.caliber.groupCounts.B = 2;
  const probs = checkSelfConsistency(f2.register);
  check(probs.some((p) => /分组 B/.test(p)) && probs.some((p) => /Σ\(A\+B\+C\)/.test(p)), "负控：分组计数漂移必须报分组与 Σ 两处");
  check(checkSelfConsistency(f0.register).length === 0, "正控：干净表无一致性问题");

  // 负控③：evidence 行号漂移
  const f3 = fixture(); f3.register.entries[0].evidence[0].line = 2;
  r = reconcile(f3.register, f3.readings, f3.plan);
  check(has(r, /a\.one.*实测 第 2 行/), "负控：证据行号漂移必须红并点名 seamId");

  // 负控④：未归属模块非空
  const f4 = fixture(); f4.readings.registry.unassigned = ["zz_orphan"];
  r = reconcile(f4.register, f4.readings, f4.plan);
  check(has(r, /unassignedModules/), "负控：未归属非空必须红");

  // 负控⑤：旧读数出现在漂移段之外
  const f5 = fixture();
  r = reconcile(f5.register, f5.readings, "正文里出现旧读数 9 条（无任何漂移字样）。\n");
  check(has(r, /旧读数 9/), "负控：旧读数在漂移段外必须红");
  // 正控：旧读数在含「漂移」的 §0.12.4 区段内 ⇒ 允许（f0 已含）
  const f5b = fixture();
  r = reconcile(f5b.register, f5b.readings, f5b.plan.replace("### 0.13 下一节", "### 0.13 下一节\n这里再写一次 9 条就违规了"));
  check(has(r, /旧读数 9 出现在漂移列举段之外/), "负控⑤b：漂移段之后的同节外出现也要红");

  // 正控⑤c / 负控⑤d（2026-11-06，章节号与读数撞号）：`§124` 这类**章节号**不是读数。
  // 触发实参是本仓真实形态：给 PLAN 追加 `## §124 …` 一节，让 11 行含 `§124` 的正文被
  // 判成「旧读数 124 出现在漂移列举段之外」⇒ 门红 11 条，而 124 一次都没作为读数出现。
  // 三个入口（numberOccurrences / numberHitLines / planNumberHits）必须同一口径，逐个数过去：
  check(numberOccurrences("见 §124 一节，另见 §0.7。", 124).length === 0, "正控⑤c：numberOccurrences 不得把章节号 `§124` 算成读数出现");
  check(numberOccurrences("该读数现为 124 条。", 124).length === 1, "负控⑤d：裸 `124` 仍必须算命中（加了 § 例外不许把真读数一起放过）");
  check(numberHitLines(["引用 §124 的结论"], 124).length === 0, "正控⑤e：numberHitLines 与 numberOccurrences 同口径（§ 放过）");
  check(numberHitLines(["该值 124 已过期"], 124).length === 1, "负控⑤f：numberHitLines 的裸数命中不许一起放过");
  check(planNumberHits("详见 §9 一节。", "9").length === 0, "正控⑤g：planNumberHits 同样放过章节号");
  check(planNumberHits("登记值 9 条。", "9").length === 1, "负控⑤h：planNumberHits 的裸数命中仍算（否则 § 例外会顺带放宽 planNumber 判据）");
  const f5c = fixture();
  r = reconcile(f5c.register, f5c.readings, "正文里出现旧读数 9 条（无任何漂移字样）。\n");
  check(has(r, /旧读数 9/), "负控⑤i：reconcile 档的裸读数出段照旧红（§ 例外没有把这一族整体关掉）");
  const f5d = fixture();
  r = reconcile(f5d.register, f5d.readings, "本结论见 §9 一节（无任何漂移字样）。\n");
  check(!has(r, /旧读数 9/), "正控⑤j：reconcile 档不得因 `§9` 判红（本仓追加 §124 一节的真实形态）");
  // 正控⑤k / 负控⑤l：**小节号** `124.1`（`### 124.1 标题`）同样不是读数 —— 只放过「`.` 后紧跟数字」这一种
  // 形态，句末的 `124.`（后面不是数字）必须照旧算命中，否则这条例外会顺手放宽真读数的判据。
  check(numberOccurrences("### 124.1 纠正口径", 124).length === 0, "正控⑤k：小节号 `124.1` 不得算成读数出现");
  check(numberHitLines(["### 124.1 纠正口径"], 124).length === 0, "正控⑤k2：numberHitLines 与 numberOccurrences 对 `124.1` 判定一致");
  check(numberOccurrences("该读数现为 124. 句末点号", 124).length === 1, "负控⑤l：句末 `124.`（点后不是数字）仍必须算命中，例外不许顺手放宽真读数");
  check(numberHitLines(["该读数现为 124. 句末点号"], 124).length === 1, "负控⑤l2：numberHitLines 同样保留句末点号形态的命中");

  // 负控⑥：bannedPlanPhrases 命中
  const f6 = fixture();
  r = reconcile(f6.register, f6.readings, f6.plan + "\n某段写了 B 11。\n");
  check(has(r, /并行口径禁语/), "负控：禁语命中必须红");

  // 负控⑦：manual 行不得被当成通过
  r = reconcile(f0.register, f0.readings, f0.plan);
  check(r.manualLines.some((m) => m.includes("k.three") && m.includes("未机器复检")), "负控：manual 行必须进「未机器复检」摘要");
  const f7 = fixture(); f7.register.entries[2].expect.implementors = 777; // manual 行登记值再离谱也不许进 hardFails
  r = reconcile(f7.register, f7.readings, f7.plan);
  // 第 3 条修复：counts.machine 也必须钉死 —— 若 `continue` 分支被删，manual 行会被误计进机器复检，
  // 单看 hardFails===0 && manual===1 测不出来（3 行表里 manual 既是 manual 又是 machine 才叫假绿）。
  check(r.hardFails.length === 0 && r.counts.manual === 1 && r.counts.machine === 2, `负控⑦b：manual 行不判红、只计 1 条未机器复检且**不占机器复检名额**（实得 manual=${r.counts.manual} machine=${r.counts.machine}）`);

  // 负控⑧：接缝状态（待建却在 registry 出现 / 已建却缺失）与远程清单封闭集合
  const f8 = fixture(); f8.readings.seamRegistryText = '"b.two"';
  r = reconcile(f8.register, f8.readings, f8.plan);
  check(has(r, /接缝「b\.two」登记「待建」/), "负控：待建接缝已被登记必须红");
  const f8b = fixture(); f8b.readings.seamRegistryText = "";
  r = reconcile(f8b.register, f8b.readings, f8b.plan);
  check(has(r, /接缝「a\.one」登记「已建」/), "负控：已建接缝缺字面量必须红");
  const f8c = fixture(); f8c.readings.remoteSeams = ["x.extra"]; // 等长替换：数量档绿、封闭集合档必须红
  r = reconcile(f8c.register, f8c.readings, f8c.plan);
  check(has(r, /封闭集合破裂/), "负控：远程清单多一条（等长替换）必须红");
  const f8d = fixture(); f8d.readings.remoteSeams = ["a.one", "x.extra"];
  r = reconcile(f8d.register, f8d.readings, f8d.plan);
  check(has(r, /remoteSeamCount.*登记 1 条 \/ 实测 2 条/), "负控：远程清单数量不符必须红并报差值");

  // 负控⑨：实现者计数
  const f9 = fixture(); f9.readings.implementors.TraitOne = 3;
  r = reconcile(f9.register, f9.readings, f9.plan);
  check(has(r, /实现者「TraitOne」：登记 2 \/ 实测 3 \/ 差 1/), "负控：实现者数差 1 必须红并报「登记/实测/差」");

  // 负控⑩：planNumber 缺失 —— hard 锚点红；manual/hard:false 锚点只报告不红
  const f10 = fixture(); anchor(f10, "commandTotal").planNumber = "4242";
  r = reconcile(f10.register, f10.readings, f10.plan);
  check(has(r, /planNumber「4242」/), "负控：hard 锚点 planNumber 缺失必须红（本例同时踩中⑪「自相矛盾」，两条都是真红）");
  const f10b = fixture(); anchor(f10b, "k1Ext").planNumber = "99"; anchor(f10b, "k1Ext").planMustAppear = true;
  r = reconcile(f10b.register, f10b.readings, f10b.plan);
  check(r.hardFails.length === 0 && r.reports.some((x) => x.includes("planNumber「99」未在 PLAN 正文以数字边界形态命中")), "正控⑩b：manual 锚点 planNumber 缺失只报告（planNumber 与 value 保持同数，不触发⑪）");

  // 负控⑪（第 1 条修复）：锚点内部自洽 —— 只改 value 迁就盘上、planNumber 留旧数必须被抓出
  const f11 = fixture(); anchor(f11, "commandTotal").value = 11; f11.readings.registry.commandTotal = 11;
  r = reconcile(f11.register, f11.readings, f11.plan.replace("能力面 10 条 / 3 模块", "能力面 11 条 / 3 模块"));
  check(has(r, /锚点 commandTotal 自相矛盾：value=11 \/ planNumber=10/), "负控⑪a：value 单方面迁就盘上（planNumber 留旧数）必须硬失败并点名锚点");
  // 负控⑫：钉句里留旧数（value/planNumber 同批改了，PLAN 原文没改）
  const f12 = fixture(); anchor(f12, "commandTotal").planPinned = [{ line: 1, mustContain: ["5555", "3"] }];
  r = reconcile(f12.register, f12.readings, f12.plan);
  check(has(r, /锚点 commandTotal 自相矛盾：.*mustContain=5555/), "负控⑫b：planPinned 纯数字 token 与表内登记数脱节必须硬失败");
  // 正控：跨锚点宽容不得放松「兄弟读数」这一档 —— 用直调把「撞别锚点的数」与「谁都不撞」分开验
  const consReg = (anchors) => ({ caliber: { groupCounts: { A: 1, B: 0, C: 0, H: 0, K: 0, D: 0 }, totalRows: 1, capacityRegister: 1 }, anchors, entries: [] });
  check(checkAnchorConsistency(consReg([
    { id: "x", value: 84873, planNumber: "84 873", planPinned: [{ line: 1, mustContain: ["84 873", "293"] }], value2: 293 },
    { id: "y", value: 3, planNumber: "3" },
  ])).length === 0, "正控⑪c：千位空格 planNumber + 兄弟锚点读数（293=value2）不得误判为矛盾");
  check(checkAnchorConsistency(consReg([{ id: "x", value: 10, planNumber: "10", planPinned: [{ line: 1, mustContain: ["10", "28 文件", "A 11 + B 10"] }] }])).length === 0, "正控⑪d：非纯数字 token（「28 文件」「A 11 + …」）不做数字比对");
  check(checkAnchorConsistency(consReg([{ id: "x", value: 10, planNumber: "12" }])).length === 1, "负控⑪e：planNumber≠value 直调必计 1 条矛盾");
  check(checkAnchorConsistency(consReg([{ id: "x", value: 10, planPinned: [{ line: 1, mustContain: ["77"] }] }])).length === 1, "负控⑪f：钉句数字谁都不撞 ⇒ 矛盾");

  // 负控⑮（2026-11-11 阻断 1）：跨锚点宽容的来源必须只有「hard:true 且机器复检」的锚点。
  // 攻击构造（纸面复演 .tmp/cr-review-sim.mjs 的攻击 1）：hard 锚点把 value 与 planNumber **同改**成
  // 盘上实测 2004 ⇒ ①档自洽、抓不到；钉句里的旧数 2005 靠另一条锚点「藏」进表 ⇒ 旧实现收 soft/manual
  // 锚点的数当宽容，于是②档整条形同虚设、返回空。合成表逐条断言「藏数进软锚点必红」。
  const launder = (donor) => checkAnchorConsistency(consReg([
    { id: "hard", hard: true, check: "k1RsFileCount", value: 2004, planNumber: "2004", planPinned: [{ line: 1, mustContain: ["2004", "2005"] }] },
    donor,
  ]));
  check(launder({ id: "soft", hard: false, check: "registryFileLineCount", value: 2005, planNumber: "2005" }).length === 1, "负控⑮a：把旧数 2005 藏进 hard:false 锚点 ⇒ 钉句 2005 必须仍判红（软锚点不得作跨锚点宽容来源，否则改软锚点即可洗白钉句）");
  check(launder({ id: "manual", hard: false, check: "manual", value: 2005, planNumber: "2005" }).length === 1, "负控⑮b：把旧数藏进 check=manual 锚点（永不判红那类）⇒ 同样必须判红");
  check(launder({ id: "hardButManual", hard: true, check: "manual", value: 2005, planNumber: "2005" }).length === 1, "负控⑮c：hard:true 但 check=manual（未被机器复检）⇒ 其数字仍不得作宽容来源");
  check(launder({ id: "undeclared", check: "k1PubTraitCount", value: 2005, planNumber: "2005" }).length === 1, "负控⑮d：donor 未声明 hard:true ⇒ fail-closed，不给宽容资格");
  check(launder({ id: "donor", hard: true, check: "k1PubTraitCount", value: 2005 }).length === 0, "正控⑮e：合法跨锚点引用（donor 为 hard:true 且机器复检的锚点，value 即被钉的数）不得误判红");
  check(checkAnchorConsistency(consReg([
    { id: "a", hard: true, check: "registryCommandCount", value: 1541, planNumber: "1541", planPinned: [{ line: 1, mustContain: ["1541", "164"] }] },
    { id: "b", hard: true, check: "registryModuleCount", value: 164, planNumber: "164" },
    { id: "c", hard: false, check: "registryFileLineCount", value: 1608 },
  ])).length === 0, "正控⑮f：现盘两条合法宽容（164←commandModules、316←k1PubTraitCount 同型）在收窄后仍不红；hard:false 的 1608 在场也不参与");

  // 负控⑬（第 4 条修复）：planNumber 的数字边界 —— 被更长数字包住 / 日期串里的两位数不再算命中
  const f13 = fixture();
  r = reconcile(f13.register, f13.readings, "登记 110 条 / 30 模块（都被更长数字包住，独立成词的那个数一个也没有）。\n\n### 0.12.4 漂移列举\n旧读数 9 只在本段出现。\n\n### 0.13 下一节\n无。\n");
  check(has(r, /planNumber「10」未在 PLAN 正文以数字边界形态命中/), "负控⑬a：`10` 只在 `110` 这类更长数字里出现 ⇒ 裸子串假绿必须消失");
  check(planNumberHits("发布 2026-11-26，另有 1.26 与 PLAN:26 与 1534/26。", "26").length === 0, "负控⑬b：日期/小数点/行号引用/斜杠配对里的 26 一律不算命中");
  check(planNumberHits("股票 84 873 行前端出包。", "84 873").length === 1, "正控⑬c：千位空格写法仍算命中（不许误伤现盘 15 个锚点）");
  check(planNumberHits("终局能力全集登记 26 条。", "26").length === 1, "正控⑬d：独立成词的 26 算命中");

  // 负控⑯（`#138`）：alsoAt 的「假健康」—— 只有 token 在段里不算健康，必须旧读数也在段里，
  // 且出现行 ±3 行内能归因到某个 token；放行按出现行（anchored），不按整个窗口。
  {
    const far = ["### 段头（自证 token）", "填充 1", "填充 2", "填充 3", "填充 4", "填充 5", "无关段落里也写了 124 这个数"];
    const st = alsoAtRegionStatus(far, { startLine: 1, endLine: 7, mustContain: ["段头（自证 token）"] }, "124");
    check(!st.ok && /±3 行内没有任何自证 token/.test(st.reason), "负控⑯a：旧读数在段内但离 token 超过半径 ⇒ 不得判健康（窗口吞数的假健康）");
    check(st.occurrences.includes(7) && st.anchored.length === 0, "负控⑯a2：出现行必须被识别、但不得进 anchored");
    const near = ["# 事实：HEAD 产物 = 1540 条、按算法应生成 1541。", "填充"];
    const st2 = alsoAtRegionStatus(near, { startLine: 1, endLine: 2, mustContain: ["HEAD 产物", "按算法应生成"] }, "1541");
    check(st2.ok && st2.anchored.includes(1), "正控⑯b：出现行与 token 同行 ⇒ 健康且只放行该行");
    const empty = alsoAtRegionStatus(["只有 token 没有数", "填充"], { startLine: 1, endLine: 2, mustContain: ["只有 token 没有数"] }, "1541");
    check(!empty.ok && /段内已找不到旧读数/.test(empty.reason), "负控⑯c：段内没有旧读数 ⇒ 即使 token 齐也不得判健康");
  }

  // 正控⑰ / 负控⑰（#306）：智能体工具面**判据本体**的用例（合成源码、零盘 IO）。断言的是
  //   「什么算一条标注」；锚点数值档由 ANCHOR_CHECK_NEGATIVE_CASES 的 agentCommandFaceCount 负责，两档分工不同。
  //   ⚠ 标注名一律用字符串拼接构造（理由见 lib/command-domains.mjs 同段注释）：判据扫的是源码文本，
  //     完整字面量留在 `.mjs` 里，将来扫描面扩到脚本自身就会自命中。
  {
    const AC = "#" + "[agent" + "_command";
    const L = (...ls) => ls.join("\n");
    const src = L(
      "use axagent_agent_macro::agent_command;",
      `/// 文档示例 ${AC}(domain = "docfake") —— 注释不算`,
      `//${AC}(domain = "linefake") —— 紧贴的行注释不算`,
      `${AC}(domain = "finance", safety = Safe)`,
      `    ${AC}(domain = market_sim)`,
      `${AC}`,
      `#[tauri::command]`,
      `pub async fn demo() {}`,
      `${AC}(`,
      `  safety = Safe,`,
      `)`,
      `${AC}(domain = "general",`,
      `  safety = Safe)`,
    );
    const ac = scanAgentCommandFace([["src-tauri/src/commands/demo.rs", src]]);
    check(ac.total === 4, `正控⑰a：真实标注必须计满 4（引号 domain / 裸值 domain / 无 domain 跨行 / 有 domain 跨行各一条），实得 ${ac.total}`);
    check(ac.byDomain.finance === 1 && ac.byDomain.market_sim === 1 && ac.byDomain.general === 1, `正控⑰b：三种 domain 写法都要归位，实得 ${JSON.stringify(ac.byDomain)}`);
    check(ac.byDomain[AC_NO_DOMAIN_BUCKET] === 1, "正控⑰c：未声明 domain 必须单列一桶，不许静默并进别的域");
    check(ac.domainCount === 4, `正控⑰d：domainCount 数的是桶数 4，实得 ${ac.domainCount}`);
    check(ac.host === 4 && ac.nonHost === 0, "正控⑰e：宿主根分段（src-tauri 归 host）");
    // 负控⑰a：裸标注（行内无开括号）不得吞掉**下一行**的左括号。旧写法只判 indexOf >= 0，
    //   这组两行会被配平成属性体 `foo` 并计入未声明桶 ⇒ 读数虚高 1；行内限定后必须 0。
    check(scanAgentCommandFace([["src-tauri/a.rs", L(AC, "fn foo() {}")]]).total === 0, "负控⑰a：裸标注 + 后续无关左括号 ⇒ 必须 0 条（跨行吞括号即静默虚高）");
    // 负控⑰b：三种注释形态一律不计（`//!` 模块文档、缩进的 `//`、块注释）。
    const commentOnly = L(`//! ${AC}(domain = "mod_doc")`, `  // ${AC}(domain = "block")`, `  /* ${AC}(domain = "x") */`);
    check(scanAgentCommandFace([["src-tauri/b.rs", commentOnly]]).total === 0, "负控⑰b：`//!` / `//` / `/*` 注释形态的标注名一律不计");
    // 负控⑰c：非宿主根必须单列。真实树当前为 0（插件命名命令走 `build_plugin_command_index`，
    //   不经 inventory），而锚点 value2 钉的就是这个 0 ⇒ 插件侧真接入标注时必须显式判红，不许静默变化。
    const plug = scanAgentCommandFace([["AxInvest-plugin-repos/stock/plugin-stock/src/main.rs", L(`${AC}(domain = "finance")`)]]);
    check(plug.total === 1 && plug.host === 0 && plug.nonHost === 1, "负控⑰c：插件根标注计入 nonHost，不得并进 host");
  }

  // 负控⑭（第 2 条修复）：逐类 anchor check 的「登记≠实测 必被抓出」+ 正控「登记=实测 不假阳性」。
  // 全部走合成读数（fixture），零盘扫描；覆盖度按**镜像表实际用到的 check 种类**核算并印出。
  const covered = new Set();
  for (const c of ANCHOR_CHECK_NEGATIVE_CASES) {
    const bad = fixture(); c.mutate(bad);
    const rb = reconcile(bad.register, bad.readings, bad.plan);
    const inHard = rb.hardFails.some((x) => x.includes(`锚点 ${c.id}`) || x.includes(` ${c.id}：`));
    const inReport = rb.reports.some((x) => x.includes(`锚点 ${c.id}`));
    check(c.tier === "hard" ? inHard : inReport, `负控⑭：check「${c.check}」登记≠实测（锚点 ${c.id}，应落 ${c.tier} 档）必须被抓出并点名，实得 hard=[${rb.hardFails.join(" | ")}] report=[${rb.reports.join(" | ")}]`);
    if (c.tier === "report") { check(!inHard, `负控⑭：check「${c.check}」的 hard:false 锚点 ${c.id} 差异只许「只报告」，不得升成硬失败`); }
    covered.add(c.check);
    // 正控：同一 check 在干净读数下必须存在且不假阳性（hard:false 档干净时同样 PASS，差异时才降为 REPORT）
    if (c.check === "manual") {
      check(rb.reports.some((x) => x.includes("本门机械复算 99")), "负控⑭m：manual 锚点的机械复算读数必须印进报告（登记 99 与实测 99 可对照，且不进 hardFails）");
    } else {
      const cleanRow = cleanResults.find((a) => a.txt.startsWith(`${c.id}（`));
      check(cleanRow && cleanRow.status === "PASS", `正控⑭：check「${c.check}」在干净读数下锚点 ${c.id} 必须 PASS（实得 ${cleanRow ? cleanRow.status : "fixture 无此锚点，覆盖清单与 fixture 不同步"}）`);
    }
  }
  // 覆盖度的分母 = **镜像表实际用到的 check 种类**（只读表本体一次，不扫源码）；表缺失时退回脚本声明清单
  let tableChecks = null;
  let coverageNote = "（按脚本声明清单计）";
  if (existsSync(REGISTER_PATH)) {
    try { tableChecks = new Set(JSON.parse(readFileSync(REGISTER_PATH, "utf8")).anchors.map((a) => a.check)); coverageNote = "（按镜像表实际使用计）"; }
    catch { coverageNote = "（镜像表 JSON 解析失败，退回脚本声明清单）"; }
  } else { coverageNote = "（镜像表缺失，退回脚本声明清单）"; }
  const usedChecks = tableChecks ?? new Set(ANCHOR_CHECK_KINDS_DECLARED);
  for (const k of usedChecks) { check(covered.has(k), `自检覆盖空洞：check 种类「${k}」没有负控+正控 ⇒ 必须补进 ANCHOR_CHECK_NEGATIVE_CASES 与 fixture`); }
  for (const k of covered) { check(ANCHOR_CHECK_KINDS_DECLARED.includes(k), `覆盖清单里的 check「${k}」不在脚本声明清单（头注 data-format 行）内`); }
  if (tableChecks) {
    for (const k of tableChecks) { check(ANCHOR_CHECK_KINDS_DECLARED.includes(k), `镜像表用了声明清单之外的 check 种类「${k}」⇒ 头注 data-format 行与本门 switch 必须同批改（禁止只改表）`); }
  }
  const missing = [...usedChecks].filter((k) => !covered.has(k));
  console.log(`SELFTEST 覆盖度：已覆盖 check 种类 ${covered.size} / 表内实际使用 ${usedChecks.size}${coverageNote}${missing.length ? ` —— 缺 [${missing.join(", ")}]` : " —— 无空洞"}`);

  // ㉑系列（任务 #48，--repin-plan）：定位键唯一才回写 · 歧义 / 零命中 / 缺键一律拒绝写盘 ·
  //   回写后 PLAN 对账必归零 · 只改行号不动判据 · 行号锚点**不得**被全文模糊匹配替代（主代理红线）。
  {
    // 正控㉑d：PLAN 未变 ⇒ 无需重锚（幂等，绝不无谓写盘）
    const f = fixture();
    const rp = computeRepin(f.register, f.plan);
    check(rp.changes.length === 0 && rp.blockers.length === 0 && rp.noKey.length === 0, `正控㉑d：干净读数不得产生任何回写（实得 changes=${rp.changes.length} blockers=[${rp.blockers.join(" | ")}] noKey=[${rp.noKey.join(" | ")}]`);
    check(rp.total === 6 && rp.kept === 6, `正控㉑d2：受行号约束引用应 6 条全相符（1 钉句 + 3 planLine + 2 区间端点；实得 total=${rp.total} kept=${rp.kept}）`);
  }
  {
    // 负控㉑a：删 PLAN 一行（实测过的真实形态）⇒ 其后的引用各 -1，钉句在第 1 行不受影响
    const f = fixture();
    const arr = f.plan.split("\n");
    arr.splice(1, 1);
    const shifted = arr.join("\n");
    const rp = computeRepin(f.register, shifted);
    check(rp.blockers.length === 0 && rp.noKey.length === 0, `负控㉑a：删一行后每条引用都应唯一定位（实得 blockers=[${rp.blockers.join(" | ")}]）`);
    check(rp.changes.length === 5 && rp.kept === 1, `负控㉑a2：应恰好 5 条待回写 + 1 条不动（实得 changes=${rp.changes.length} kept=${rp.kept}）`);
    const to = Object.fromEntries(rp.changes.map((c) => [c.path, c.to]));
    check(to["entries[a.one].planLine"] === 4 && to["entries[k.three].planLine"] === 6, `负控㉑a3：planLine 必须按定位键逐条重算（实得 ${JSON.stringify(to)}）`);
    check(to["entries[b.two].planLineRange[0]"] === 4 && to["entries[b.two].planLineRange[1]"] === 5, "负控㉑a4：planLineRange 两端点各自按定位键回写（不许整段 -1 猜）");
    // 正控㉑e：回写后 PLAN 对账归零 —— 这就是验收条「跑完 --repin-plan，普通运行不再报 PLAN 红」的等价断言
    const next = applyRepinToRegister(f.register, rp.changes);
    const after = checkPlan(next, shifted);
    check(after.fails.length === 0 && after.soft.length === 0, `正控㉑e：回写后 checkPlan 必须零差异（实得 hard=[${after.fails.join(" | ")}] soft=[${after.soft.join(" | ")}]）`);
    // 正控㉑h：重锚**只改行号** —— token / 读数 / 分组等判据字段必须逐字未动
    check(JSON.stringify(stripRepinEditableFields(next)) === JSON.stringify(stripRepinEditableFields(f.register)), "正控㉑h：除行号外的任何字段被改动即违反本模式的前提（判据不许顺手放宽）");
    // 负控㉑i：回写后的表若再被指错行，判定照样红（与 ㉑g 同族，防「重锚把锚点洗成摆设」）
    const mis = JSON.parse(JSON.stringify(next));
    mis.anchors[0].planPinned[0].line = mis.anchors[0].planPinned[0].line + 1;
    const misRes = checkPlan(mis, shifted);
    check(misRes.fails.some((x) => /钉句 PLAN:\d+ 不符/.test(x)), "负控㉑i：行号再指错一行必须照样红（重锚不改变判据形态）");
  }
  {
    // 负控㉑b：定位键**非唯一命中** ⇒ 阻断该项、绝不进 changes（不许从多命中里挑一个）
    const f = fixture();
    const dup = `${f.plan}\n另一处也写 10 条 / 3 模块。\n`; // 第 11 行与第 1 行同命中同一 token 组
    const g = fixture();
    g.register.anchors[0].planPinned[0].line = 4; // 表里的行号漂到别处 ⇒ 需要重锚
    const rp = computeRepin(g.register, dup);
    check(rp.blockers.some((b) => /anchors\[commandTotal\]\.planPinned\[0\]\.line = 4.*非唯一命中.*\[1, 12\]/.test(b)), `负控㉑b：钉句两处命中必须报阻断并列出候选行（实得 blockers=[${rp.blockers.join(" | ")}]）`);
    check(!rp.changes.some((c) => c.path === "anchors[commandTotal].planPinned[0].line"), "负控㉑b2：歧义项不得进 changes");
  }
  {
    // 负控㉑c：**零命中**（句子被改写）⇒ 同样阻断，报「不许猜行、不许插值」
    const f = fixture();
    const g = fixture();
    g.register.anchors[0].planPinned[0].mustContain = ["这句盘上不存在的话"];
    const rp = computeRepin(g.register, f.plan);
    check(rp.blockers.some((b) => /零命中/.test(b)), `负控㉑c：改写后的钉句必须报零命中并阻断（实得 blockers=[${rp.blockers.join(" | ")}]）`);
    const h = fixture();
    h.register.entries[2].planAnchorText = "整行已被改写";
    const rp2 = computeRepin(h.register, h.plan);
    check(rp2.blockers.some((b) => /entries\[k\.three\]\.planLine = 7.*零命中/.test(b)), `负控㉑c2：整行文本键失配必须阻断（实得 blockers=[${rp2.blockers.join(" | ")}]）`);
  }
  {
    // 负控㉑f：**缺定位键** ⇒ 阻断（新增行号引用必须同批带键，否则重锚能力随表增长静默腐烂）
    const f = fixture();
    const g = JSON.parse(JSON.stringify(f.register));
    delete g.entries[1].planAnchorText;
    g.entries[0].planPinned = [{ line: 9, mustContain: [] }];
    g.anchors.push({ id: "noTok", label: "无 token 钉句", value: 1, check: "manual", hard: false, planPinned: [{ line: 3, mustContain: [] }] });
    const rp = computeRepin(g, f.plan);
    check(rp.noKey.some((x) => /entries\[b\.two\]\.planLine/.test(x)), `负控㉑f：登记项缺 planAnchorText 必须记「缺定位键」（实得 noKey=[${rp.noKey.join(" | ")}]）`);
    check(rp.noKey.some((x) => /anchors\[noTok\]\.planPinned\[0\]/.test(x)), "负控㉑f2：钉句无 mustContain token ⇒ 同样缺键（b.two 的 planLineRange[0] 也因缺 planAnchorTextRange 落同一档）");
    check(!rp.changes.some((c) => c.path.startsWith("entries[b.two]")), "负控㉑f3：缺键项绝不进 changes");
  }
  {
    // 负控㉑g（**红线**）：判据仍是「第 N 行含该句」，不是「该句在 PLAN 某处出现」——
    //   把锚点指到同一文档的另一行、句子仍在别处存在时，checkPlan 必须照样红。
    //   这条锁的是「不许改成用模糊匹配代替行号锚点」：一旦有人把 checkPlan 换成全文 includes，
    //   本断言立刻红（真漂移就放掉了）。
    const g = fixture();
    g.register.anchors[0].planPinned[0].line = 6; // 「10 条 / 3 模块」实际在第 1 行
    const rp = checkPlan(g.register, g.plan);
    check(rp.fails.some((x) => /钉句 PLAN:6 不符/.test(x)), "负控㉑g：行号错而句子仍在文档别处 ⇒ 必须判红（行号锚点没被降成全文模糊匹配）");
    const rpRepin = computeRepin(g.register, g.plan);
    check(rpRepin.changes.some((c) => c.path === "anchors[commandTotal].planPinned[0].line" && c.to === 1), "正控㉑g2：同一形态下 --repin-plan 能按 token 唯一定位回第 1 行（重锚治的是行号漂，不是判据）");
  }
  {
    // 负控㉑j：文本版定点回写（真正落盘的那条路径）——needle 唯一才改，不唯一就整次拒绝
    const entry = (seamId, anchorText, planLine, extra = "") => `    { "seamId": "${seamId}", "group": "A", "planAnchorText": ${JSON.stringify(anchorText)}, "planLine": ${planLine}${extra} },`;
    const pinObj = (line) => `    { "id": "commandTotal", "planPinned": [ { "line": ${line}, "mustContain": ["10", "3"] } ] }`;
    const mkTable = (pinLine, rows) => `{\n  "anchors": [\n${pinObj(pinLine)}\n  ],\n  "entries": [\n${rows.join("\n")}\n  ]\n}\n`;
    const arr0 = fixture().plan.split("\n");
    arr0.splice(1, 1);
    const shifted = arr0.join("\n");
    const inline = mkTable(1, [
      entry("a.one", "登记项一 a.one 已建。", 5),
      entry("b.two", "登记项二 b.two 待建。", 6, `, "planLineRange": [5, 6], "planAnchorTextRange": ${JSON.stringify(["登记项一 a.one 已建。", "登记项二 b.two 待建。"])}`),
      entry("k.three", "登记项三 k.three 人工。", 7).replace(/,$/, ""),
    ]);
    const regInline = JSON.parse(inline);
    const rp = computeRepin(regInline, shifted);
    check(rp.blockers.length === 0 && rp.noKey.length === 0, `正控㉑j0：内联表应无阻断（实得 blockers=[${rp.blockers.join(" | ")}] noKey=[${rp.noKey.join(" | ")}]）`);
    const patched = planRepinPatches(inline, regInline, rp.changes);
    check(patched.ok, `正控㉑j：内联排版的表必须能定点替换（实得 problems=[${patched.problems.join(" | ")}]）`);
    const after = patched.text ? JSON.parse(patched.text) : { entries: [] };
    const aOne = after.entries?.find((e) => e.seamId === "a.one");
    const bTwo = after.entries?.find((e) => e.seamId === "b.two");
    check(aOne?.planLine === 4 && bTwo?.planLine === 5 && (bTwo?.planLineRange ?? []).join(",") === "4,5", `正控㉑j2：定点替换后的行号必须与内存版一致（含区间两端点；实得 ${JSON.stringify((after.entries ?? []).map((e) => [e.planLine, e.planLineRange]))}）`);
    check((patched.text ?? "").includes('"mustContain": ["10", "3"]') && (patched.text ?? "").includes('"group": "A"'), "正控㉑j3：判据与其它字段逐字未动（只动行号）");
    // 负控㉑j4：兜底 needle `"planLine": 5` 在表里出现两次（另一条登记项同行号、且它的整行键被拆行排版
    //   ⇒ 带 planAnchorText 的精确 needle 也失效）⇒ **整次拒绝写盘**，不许只改能定位的那一半。
    const twinRow = (seamId, anchorText) => `    {\n    "seamId": "${seamId}",\n    "group": "A",\n    "planAnchorText": ${JSON.stringify(anchorText)},\n    "planLine": 5\n    }`;
    const twin = mkTable(1, [`${twinRow("a.one", "登记项一 a.one 已建。")},`, twinRow("k.three", "登记项三 k.three 人工。")]);
    const regTwin = JSON.parse(twin);
    const rpTwin = computeRepin(regTwin, shifted);
    check(rpTwin.changes.length > 0, `负控㉑j4 前提：夹具必须真的产生待回写项（实得 ${rpTwin.changes.length}）`);
    const bad = planRepinPatches(twin, regTwin, rpTwin.changes);
    check(!bad.ok && bad.problems.some((p) => /找不到唯一的替换锚/.test(p)), `负控㉑j4：替换锚不唯一时必须拒绝定点写盘并给原因（实得 ok=${bad.ok} problems=[${bad.problems.join(" | ")}]）`);
    check(bad.text === null, "负控㉑j5：拒绝写盘时不得返回任何可落盘文本");
  }

  // ㉒系列（任务 #55，--repin-code）：代码证据锚点按**整行定位键**重锚 —— 唯一命中才回写 ·
  //   零命中 / 歧义 / 缺键 / 补丁不唯一 / 内存版与文本版不一致 ⇒ **整次拒绝写盘** ·
  //   默认 dry-run 不落盘 · 写盘后从盘上复检不过 ⇒ 回滚。IO 注入内存假盘 ⇒ 与正式跑共用
  //   executeCodeRepin 同一实现，但零真盘（不碰 scripts/capability-register.json）。
  {
    const A_SRC = "impl Worker {\n  fn one() { remote_method_not_supported(); }\n  fn two() { remote_method_not_supported(); }\nfn remote_method_not_supported() -> String {\n    String::new()\n}\n";
    const DUP_SRC = `${A_SRC}  fn one() { remote_method_not_supported(); }\n`; // 第 7 行与第 2 行整行逐字相同 ⇒ anchorText 歧义
    const srcs = { "a.rs": A_SRC, "d.rs": DUP_SRC };
    const ftf = (rel) => (Object.prototype.hasOwnProperty.call(srcs, rel) ? srcs[rel] : null);
    const evLit = (o) => `      { ${["ref", "file", "line", "endLine", "token", "anchorText", "role"].filter((k) => o[k] !== undefined).map((k) => `${JSON.stringify(k)}: ${JSON.stringify(o[k])}`).join(", ")} }`;
    const mkTable = (rows) => {
      const text = `{\n  "entries": [\n${rows.map((r) => `    { "seamId": "${r.seamId}", "group": "A", "evidence": [\n${r.evs.map(evLit).join(",\n")}\n      ] }`).join(",\n")}\n  ]\n}\n`;
      return { text, register: JSON.parse(text) };
    };
    const mkIo = (text, opts = {}) => {
      const st = { register: text, backups: [], writes: 0 };
      const io = {
        backup: (t) => { st.backups.push(t); return ".tmp/fake.bak.json"; },
        write: (t) => { st.writes++; st.register = t; },
        reread: () => { try { const parsed = JSON.parse(st.register); return opts.tamper ? opts.tamper(parsed) : parsed; } catch { return "BAD"; } },
        restore: (t) => { st.register = t; },
        fileTextFor: ftf,
      };
      return { st, io };
    };
    const emitOff = () => {};
    const brief = (rp) => `changes=${rp.changes.map((c) => `${c.path} ${c.line}→${c.to}`).join(",") || "-"} blockers=[${rp.blockers.join(" | ") || "-"}] noKey=[${rp.noKey.join(" | ") || "-"}]`;

    // ㉒a 正控：anchorText 整行唯一命中 ⇒ 可回写，且回写后判定档转绿、只动行号
    const t1 = mkTable([{ seamId: "s.one", evs: [{ ref: "a.rs:1", file: "a.rs", line: 1, token: "fn remote_method_not_supported", anchorText: "fn remote_method_not_supported() -> String {", role: "实现" }] }]);
    const rp1 = computeCodeRepin(t1.register, ftf);
    check(rp1.total === 1 && rp1.kept === 0 && rp1.changes.length === 1 && rp1.changes[0].to === 4 && !rp1.blockers.length && !rp1.noKey.length, `正控㉒a：整行键唯一命中第 4 行应给出一条回写（实得 ${brief(rp1)}）`);
    const n1 = applyCodeRepinToRegister(t1.register, rp1.changes);
    check(adjudicateEvidence(n1.entries[0].evidence[0], ftf) === null, "正控㉒a2：唯一命中回写后 evidence 判据必须转绿（= 验收条「跑完 --repin-code 代码锚点不再假红」的等价断言）");
    check(n1.entries[0].evidence[0].ref === "a.rs:4", `正控㉒a3：ref 末尾的 :行号 与 line 同步回写，表内不许自相矛盾（实得 ${n1.entries[0].evidence[0].ref}）`);
    check(JSON.stringify(stripCodeRepinEditableFields(n1)) === JSON.stringify(stripCodeRepinEditableFields(t1.register)), "正控㉒a4：除行号外任何字段被改动即违反本模式前提（token/file/role/anchorText 一字未松）");
    const mis1 = JSON.parse(JSON.stringify(n1));
    mis1.entries[0].evidence[0].line = 5;
    check(adjudicateEvidence(mis1.entries[0].evidence[0], ftf) !== null, "负控㉒a5：**红线** —— 回写后行号再指错一行，判定档照样红（重锚治行号漂，不改判据形态）");

    // ㉒e 正控：默认 dry-run 一个字都不写、连备份都不做
    const d1 = mkIo(t1.text);
    const dry = executeCodeRepin({ register: t1.register, srcText: t1.text, rp: rp1, apply: false, io: d1.io, emit: emitOff });
    check(dry.status === "dry-run" && dry.wrote === false && d1.st.writes === 0 && d1.st.backups.length === 0 && d1.st.register === t1.text, `正控㉒e：不带 --apply 必须零写盘（实得 status=${dry.status} writes=${d1.st.writes} backups=${d1.st.backups.length}）`);

    // ㉒h 正控：--apply 全流程（备份 → 定点补丁 → 写盘 → 从盘上复检通过）
    const w1 = mkIo(t1.text);
    const wr = executeCodeRepin({ register: t1.register, srcText: t1.text, rp: rp1, apply: true, io: w1.io, emit: emitOff });
    check(wr.status === "written" && w1.st.writes === 1 && w1.st.backups.length === 1 && w1.st.backups[0] === t1.text, `正控㉒h：--apply 应写一次盘且写前先留逐字节备份（实得 status=${wr.status} writes=${w1.st.writes}）`);
    check(w1.st.register.includes('"ref": "a.rs:4"') && w1.st.register.includes('"line": 4') && !w1.st.register.includes('"ref": "a.rs:1"'), `正控㉒h2：落盘的必须是定点补丁后的原文（未涉及字节逐字不动；实得 ${JSON.stringify(w1.st.register)}）`);
    check(JSON.parse(w1.st.register).entries[0].evidence[0].anchorText === "fn remote_method_not_supported() -> String {", "正控㉒h3：定位键本身逐字保留（下次重锚还要用）");
    const afterH = computeCodeRepin(JSON.parse(w1.st.register), ftf);
    check(afterH.changes.length === 0 && afterH.blockers.length === 0 && afterH.noKey.length === 0 && afterH.kept === 1, `正控㉒h4：从盘上的表复算 ⇒ 待回写/阻断/缺键全归零（实得 ${brief(afterH)}）`);

    // ㉒b 负控：anchorText 在文件内**零命中**（语义变了，不是位移）⇒ 拒写并点名
    const t2 = mkTable([{ seamId: "s.one", evs: [{ ref: "a.rs:1", file: "a.rs", line: 1, token: "fn remote_method_not_supported", anchorText: "fn remote_method_not_supported() -> Result<String> {", role: "实现" }] }]);
    const rp2 = computeCodeRepin(t2.register, ftf);
    check(rp2.changes.length === 0 && rp2.blockers.length === 1 && /零命中/.test(rp2.blockers[0]), `负控㉒b：整行键零命中必须报阻断、绝不猜行（实得 ${brief(rp2)}）`);
    check(/命中 1 行 \[4\]/.test(rp2.blockers[0]), `负控㉒b2：阻断里要带 token 命中行号诊断（只诊断，不参与放行；实得 ${rp2.blockers[0]}）`);
    const b1 = mkIo(t2.text);
    const rb = executeCodeRepin({ register: t2.register, srcText: t2.text, rp: rp2, apply: true, io: b1.io, emit: emitOff });
    check(rb.status === "refused-locate" && b1.st.writes === 0 && b1.st.register === t2.text, `负控㉒b3：零命中 ⇒ 即使带 --apply 也整次拒绝写盘（实得 status=${rb.status}）`);
    check(adjudicateEvidence(t2.register.entries[0].evidence[0], ftf) !== null, "负控㉒b4：同一条 evidence 在判定档照样红 —— 新增模式没给判定档开后门");

    // ㉒c 负控：anchorText **非唯一命中** ⇒ 列出全部候选行 + 整次拒绝（另一条能定位的不许被单独回写）
    const t3 = mkTable([
      { seamId: "s.amb", evs: [{ ref: "d.rs:1", file: "d.rs", line: 1, token: "fn one() { remote_method_not_supported(); }", anchorText: "fn one() { remote_method_not_supported(); }", role: "歧义键" }] },
      { seamId: "s.ok", evs: [{ ref: "d.rs:6", file: "d.rs", line: 6, token: "fn two() { remote_method_not_supported(); }", anchorText: "fn two() { remote_method_not_supported(); }", role: "可定位" }] },
    ]);
    const rp3 = computeCodeRepin(t3.register, ftf);
    check(rp3.changes.length === 1 && rp3.blockers.length === 1 && /非唯一命中/.test(rp3.blockers[0]) && rp3.blockers[0].includes("2 行 [2, 7]"), `负控㉒c：两处逐字相同的行 ⇒ 报「非唯一命中」并**逐条打印所有命中行号**（实得 ${brief(rp3)}）`);
    const c1 = mkIo(t3.text);
    const rc = executeCodeRepin({ register: t3.register, srcText: t3.text, rp: rp3, apply: true, io: c1.io, emit: emitOff });
    check(rc.status === "refused-locate" && c1.st.writes === 0 && c1.st.register === t3.text, "负控㉒c2：一条歧义 ⇒ 整次运行拒绝写盘，不许只回写能定位的那一半");

    // ㉒c3 正控：现盘 workflow.evolver 的真实形态 —— **同一个 token** 的两条 evidence 各带整行键 ⇒ 各自唯一定位
    const t4 = mkTable([{ seamId: "s.twin", evs: [{ ref: "a.rs:11", file: "a.rs", line: 11, token: "remote_method_not_supported", anchorText: "fn one() { remote_method_not_supported(); }", role: "区间起点" }, { ref: "a.rs:22", file: "a.rs", line: 22, token: "remote_method_not_supported", anchorText: "fn two() { remote_method_not_supported(); }", role: "区间止点" }] }]);
    const rp4 = computeCodeRepin(t4.register, ftf);
    check(rp4.changes.length === 2 && !rp4.blockers.length && !rp4.noKey.length, `正控㉒c3：同 token 双条目必须靠 anchorText 各自解歧（实得 ${brief(rp4)}）`);
    const twinTo = rp4.changes.map((c) => `${c.idx}:${c.line}→${c.to}`).join(",");
    check(twinTo === "0:11→2,1:22→3", `正控㉒c4：两条各回自己那一行，不许合并成一个数（实得 ${twinTo}）`);
    // 同形态但**没有** anchorText ⇒ 落缺定位键档（证明「按 token 检索」正是本模式拒绝采用的键）
    const t4b = mkTable([{ seamId: "s.twin", evs: [{ ref: "a.rs:11", file: "a.rs", line: 11, token: "remote_method_not_supported", role: "区间起点" }, { ref: "a.rs:22", file: "a.rs", line: 22, token: "remote_method_not_supported", role: "区间止点" }] }]);
    const rp4b = computeCodeRepin(t4b.register, ftf);
    check(rp4b.changes.length === 0 && rp4b.noKey.length === 2 && /命中 3 行 \[2, 3, 4\]/.test(rp4b.noKey[0]), `负控㉒c5：无整行键的同 token 双条目 ⇒ 两条都拒写，诊断给出 token 命中的 3 行（实得 ${brief(rp4b)}）`);

    // ㉒d 负控：缺定位键 ⇒ 拒写并点名，**不退回**用 token 猜行
    const t5 = mkTable([{ seamId: "s.nokey", evs: [{ ref: "a.rs:1", file: "a.rs", line: 1, token: "fn remote_method_not_supported", role: "没带键" }] }]);
    const rp5 = computeCodeRepin(t5.register, ftf);
    check(rp5.changes.length === 0 && rp5.noKey.length === 1 && /无 anchorText 定位键/.test(rp5.noKey[0]), `负控㉒d：已红但缺整行键 ⇒ 落「缺定位键」档（实得 ${brief(rp5)}）`);
    const d2 = mkIo(t5.text);
    const rk = executeCodeRepin({ register: t5.register, srcText: t5.text, rp: rp5, apply: true, io: d2.io, emit: emitOff });
    check(rk.status === "refused-locate" && d2.st.writes === 0, `负控㉒d2：缺键条目即使诊断里 token 唯一命中第 4 行也不许自动回写（实得 status=${rk.status}）`);
    // 正控㉒d3：行号仍相符的条目**不**算缺键（否则现盘 66 条全无 anchorText ⇒ 本模式永远不可用）
    const t6 = mkTable([{ seamId: "s.ok", evs: [{ ref: "a.rs:4", file: "a.rs", line: 4, token: "fn remote_method_not_supported", role: "未漂" }] }]);
    const rp6 = computeCodeRepin(t6.register, ftf);
    check(rp6.kept === 1 && rp6.changes.length === 0 && rp6.noKey.length === 0 && rp6.blockers.length === 0, `正控㉒d3：未漂移的 evidence 无键也不报错、不回写（实得 ${brief(rp6)}）`);
    const n6 = mkIo(t6.text);
    const rn = executeCodeRepin({ register: t6.register, srcText: t6.text, rp: rp6, apply: true, io: n6.io, emit: emitOff });
    check(rn.status === "nothing-to-write" && n6.st.writes === 0 && n6.st.backups.length === 0, "正控㉒d4：无需重锚 ⇒ 不写盘也不备份（幂等）");

    // ㉒d5 正控：本模式覆盖不到的形态 ⇒ 逐条点名「不重锚」，不当缺键、也不当已处理
    const t7 = mkTable([{ seamId: "s.skip", evs: [{ ref: "a.rs", file: "a.rs", line: 0, token: "fn remote_method_not_supported", role: "文件级" }, { ref: "a.rs:1", file: "a.rs", line: 1, endLine: 3, token: "impl Worker", role: "区间锚点" }, { ref: "a.rs:2", file: "a.rs", line: 2, role: "无 token" }] }]);
    const rp7 = computeCodeRepin(t7.register, ftf);
    check(rp7.total === 0 && rp7.skipped.length === 3 && rp7.skipped.some((x) => /文件级/.test(x.reason)) && rp7.skipped.some((x) => /区间锚点/.test(x.reason)) && rp7.skipped.some((x) => /无 token/.test(x.reason)), `正控㉒d5：文件级 / 区间 / 无 token 三类一律「不重锚」并给出理由（实得 ${JSON.stringify(rp7.skipped.map((x) => x.reason))}）`);

    // ㉒i 负控：登记的文件读不出（被移动/删除）⇒ 无文本可定位 ⇒ 拒写点名
    const t8 = mkTable([{ seamId: "s.gone", evs: [{ ref: "zz.rs:3", file: "zz.rs", line: 3, token: "whatever", anchorText: "whatever", role: "文件已搬走" }] }]);
    const rp8 = computeCodeRepin(t8.register, ftf);
    check(rp8.changes.length === 0 && rp8.blockers.length === 1 && /读不出/.test(rp8.blockers[0]), `负控㉒i：文件缺失 ⇒ 拒绝重锚（跳过 ≠ 通过；实得 ${brief(rp8)}）`);

    // ㉒f 负控：兜底替换锚只动 line、内存版还要同步 ref ⇒ **文本版与内存版不一致 ⇒ 拒写**
    const prettyText = `{\n  "entries": [\n    {\n      "seamId": "s.pretty",\n      "group": "A",\n      "evidence": [\n        {\n          "ref": "a.rs:1",\n          "note": "ref 与 file 之间隔了一个键，三段相邻的替换锚都够不着，只能落到兜底的 line 数字，于是 ref 不会同步",\n          "file": "a.rs",\n          "line": 1,\n          "token": "fn remote_method_not_supported",\n          "anchorText": "fn remote_method_not_supported() -> String {"\n        }\n      ]\n    }\n  ]\n}\n`;
    const t9 = { text: prettyText, register: JSON.parse(prettyText) };
    const rp9 = computeCodeRepin(t9.register, ftf);
    check(rp9.changes.length === 1 && rp9.changes[0].to === 4, `负控㉒f 前提：换排版的表仍要能算出 1→4（实得 ${brief(rp9)}）`);
    const p9 = codeRepinPatches(t9.text, t9.register, rp9.changes);
    check(!p9.ok && p9.problems.some((x) => /文本版回写与内存版回写结果不一致/.test(x)), `负控㉒f：内存版与文本版任一处不一致就必须拒写（实得 ok=${p9.ok} problems=${JSON.stringify(p9.problems)}）`);
    check(p9.text === null, "负控㉒f2：拒写时不得返回任何可落盘文本");
    const f1 = mkIo(t9.text);
    const rf = executeCodeRepin({ register: t9.register, srcText: t9.text, rp: rp9, apply: true, io: f1.io, emit: emitOff });
    check(rf.status === "refused-patch" && f1.st.writes === 0 && f1.st.backups.length === 0 && f1.st.register === t9.text, `负控㉒f3：补丁把握不足 ⇒ 不写盘、不备份（实得 status=${rf.status}）`);

    // ㉒j 负控：同一字面在表里出现两次 ⇒ 定点替换锚不唯一 ⇒ 整次拒绝（不许只改第一处）
    const dupEv = { ref: "a.rs:1", file: "a.rs", line: 1, token: "fn remote_method_not_supported", anchorText: "fn remote_method_not_supported() -> String {", role: "同字面" };
    const t10 = mkTable([{ seamId: "s.dup1", evs: [dupEv] }, { seamId: "s.dup2", evs: [dupEv] }]);
    const rp10 = computeCodeRepin(t10.register, ftf);
    check(rp10.changes.length === 2, `负控㉒j 前提：两处同字面各自都要回写（实得 ${rp10.changes.length}）`);
    const p10 = codeRepinPatches(t10.text, t10.register, rp10.changes);
    check(!p10.ok && p10.problems.some((x) => /找不到唯一的替换锚/.test(x)), `负控㉒j：替换锚在原文里不唯一 ⇒ 拒绝定点写盘并给原因（实得 ok=${p10.ok} problems=${JSON.stringify(p10.problems)}）`);
    const j1 = mkIo(t10.text);
    const rj = executeCodeRepin({ register: t10.register, srcText: t10.text, rp: rp10, apply: true, io: j1.io, emit: emitOff });
    check(rj.status === "refused-patch" && j1.st.writes === 0 && j1.st.register === t10.text, "负控㉒j2：不唯一时一律不落盘");

    // ㉒g 负控：写盘后从盘上复检发现「除行号外还有字段变了」⇒ 用备份回滚到原字节
    const g1 = mkIo(t1.text, { tamper: (reg) => { reg.entries[0].evidence[0].token = "并发写把判据改了"; return reg; } });
    const rg = executeCodeRepin({ register: t1.register, srcText: t1.text, rp: rp1, apply: true, io: g1.io, emit: emitOff });
    check(rg.status === "rolled-back" && rg.wrote === true && rg.problems.some((x) => /除行号/.test(x)), `负控㉒g：复检不过必须回滚并给原因（实得 status=${rg.status} problems=${JSON.stringify(rg.problems)}）`);
    check(g1.st.register === t1.text && g1.st.backups.length === 1 && g1.st.backups[0] === t1.text && g1.st.writes === 1, "负控㉒g2：回滚后表逐字节回到本次读取的原样（备份先于写盘存在）");
    // 同一形态但判据字段没动、只是行号还没落对 ⇒ 也回滚（复检复算归零这一条真在起作用）
    const g2 = mkIo(t1.text, { tamper: (reg) => { reg.entries[0].evidence[0].line = 1; return reg; } });
    const rg2 = executeCodeRepin({ register: t1.register, srcText: t1.text, rp: rp1, apply: true, io: g2.io, emit: emitOff });
    check(rg2.status === "rolled-back" && g2.st.register === t1.text, `负控㉒g3：盘上行号仍是旧值（= 从盘上复检不复算归零）⇒ 照样回滚（实得 status=${rg2.status}）`);
  }

  // ㉓系列（任务 #57，K2 键控谓词入库）：八形态各配**合成源码**的正负对照，零盘扫描。
  //   锁三件事：① 强弱分档不许合并（顺序型容器单飞算弱、⑦ 永不单独进强档）；
  //   ② ⑧ 没有 impl→trait 反查表时**不得**命中（不许把「match 里出现大写字母开头的标识符」当工厂）；
  //   ③ 本节只出数 —— k2KeyedHits 的差异一律进「只报告」，绝不进 hardFails（⑭ 系列已锁，这里再锁一次分档形态）。
  {
    const k2 = (src) => { const r = computeK2Readings(src); return { s: r.strongTraits, w: r.weakTraits, f: r.detail.formsOf }; };
    const inList = (arr, t) => arr.includes(t);
    const form = (f, t) => (f[t] ?? "").split("+");

    // ① keyed-register：动词 + 键参数 + dyn ⇒ 强
    const reg = { "r.rs": "pub trait Reg1 { fn go(&self) -> String; }\nimpl Hub { fn register_handler(&mut self, name: &str, h: Arc<dyn Reg1>) { self.m.insert(name, h); } }\n" };
    check(inList(k2(reg).s, "Reg1"), `负控㉓a：①（register 动词 + name: &str 键参数 + dyn）必须判强档（实得 ${JSON.stringify(k2(reg))}）`);
    check(form(k2(reg).f, "Reg1").includes("keyed-register"), "正控㉓a2：① 的证据形态必须点名 keyed-register（分档可追溯，不许只给结论）");
    // ①的负控：有 `register + dyn` 但**既无键参数、trait 也无 v1 identity** ⇒ 一档都不许命中
    const regNeg = { "r.rs": "pub trait Reg2 { fn go(&self) -> String; }\nimpl Hub { fn register_handler(&mut self, h: Arc<dyn Reg2>) { self.count += 1; } }\n" };
    const rn = k2(regNeg);
    check(!inList(rn.s, "Reg2") && !inList(rn.w, "Reg2"), `负控㉓a3：①缺键参数 ⇒ 不得凭「register 动词 + dyn」升强（实得 强=${JSON.stringify(rn.s)} 弱=${JSON.stringify(rn.w)}）`);
    // ② self-id-register：无键参数但 trait 自带 **v1** identity 方法（kind）⇒ 强（v1 名单与 ⑦ 名单不同，这里锁 v1 分支）
    const sid = { "r.rs": "pub trait Sid1 { fn kind(&self) -> String; fn go(&self) -> String; }\nimpl Hub { fn register_thing(&mut self, h: Arc<dyn Sid1>) { self.v.push(h); } }\n" };
    check(form(k2(sid).f, "Sid1").includes("self-id-register"), `正控㉓b：②（v1 identity 方法 kind + register 动词）必须记 self-id-register（实得 ${JSON.stringify(k2(sid).f.Sid1)}）`);
    // ③ keyed-get：get/lookup/resolve/find + dyn + &str/String 形参 ⇒ 强
    const get = { "r.rs": "pub trait Get3 { fn go(&self) -> String; }\nimpl Hub { fn lookup_item(&self, id: &str) -> Arc<dyn Get3> { self.m.get(id).unwrap().clone() } }\n" };
    check(form(k2(get).f, "Get3").includes("keyed-get"), `正控㉓c：③（lookup + id: &str + dyn 返回）必须记 keyed-get（实得 ${JSON.stringify(k2(get).f.Get3)}）`);
    const getNeg = { "r.rs": "pub trait Get4 { fn go(&self) -> String; }\nimpl Hub { fn lookup_item(&self) -> Arc<dyn Get4> { self.first.clone() } }\n" };
    check(!inList(k2(getNeg).s, "Get4"), "负控㉓c2：③无键形参 ⇒ 不得命中（否则任何返回 dyn 的 getter 都算键控）");
    // ④ factory-match（dyn 形态）：≥3 分支 + ≥3 构造点 + 块内 dyn ⇒ 强
    const fac = { "r.rs": "pub trait Fac1 { fn go(&self) -> String; }\nfn build(k: &str) -> Box<dyn Fac1> {\n    match k {\n        \"a\" => Box::new(Arc::new(Alpha::new())),\n        \"b\" => Box::new(Arc::new(Beta::new())),\n        _ => Box::new(Arc::new(Gamma::new())),\n    }\n}\n" };
    check(form(k2(fac).f, "Fac1").includes("factory-match-dyn"), `正控㉓d：④（match ≥3 分支 + 块内 dyn）必须记 factory-match-dyn（实得 ${JSON.stringify(k2(fac).f.Fac1)}）`);
    const facNeg = { "r.rs": "pub trait Fac2 { fn go(&self) -> String; }\nfn build(k: &str) -> Box<dyn Fac2> {\n    match k {\n        \"a\" => Box::new(Arc::new(Alpha::new())),\n        _ => Box::new(Arc::new(Beta::new())),\n    }\n}\n" };
    check(!inList(k2(facNeg).s, "Fac2"), "负控㉓d2：④只有 2 个分支 ⇒ 不算工厂分派（阈值 arms≥3 是形态定义的一部分）");
    // ⑤ 容器：映射型=强
    const cmap = { "r.rs": "pub trait MapC { fn go(&self) -> String; }\nstruct Reg { m: HashMap<String, Arc<dyn MapC>> }\n" };
    check(inList(k2(cmap).s, "MapC") && form(k2(cmap).f, "MapC").includes("container-map"), `正控㉓e：⑤映射型容器（HashMap<String, Arc<dyn T>>）判**强**（实得 ${JSON.stringify(k2(cmap))}）`);
    // ⑤ 容器：顺序型单飞=弱，**不得**升强
    const cseq = { "r.rs": "pub trait SeqC { fn go(&self) -> String; }\nstruct Reg { v: Vec<Arc<dyn SeqC>> }\n" };
    const cs = k2(cseq);
    check(!inList(cs.s, "SeqC") && inList(cs.w, "SeqC"), `负控㉓e2：⑤顺序型容器（Vec<Arc<dyn T>>）无自述键/无按键筛选时**不得**升强（实得 强=${JSON.stringify(cs.s)} 弱=${JSON.stringify(cs.w)}）`);
    // ⑤ 升强：顺序型 + trait 自带自述键 ⇒ 强（res33 §5.2.1 的「同现才升强」）
    const cup = { "r.rs": "pub trait SeqU { fn name(&self) -> String; fn go(&self) -> String; }\nstruct Reg { v: Vec<Arc<dyn SeqU>> }\n" };
    check(inList(k2(cup).s, "SeqU"), "正控㉓e3：顺序型容器与自述键方法同现 ⇒ 升强（两档分界必须可双向移动，不许只紧不松）");
    // ⑥ push 注册口：add_* 动词 + 参数含 dyn ⇒ 强
    const push = { "r.rs": "pub trait Stg6 { fn go(&self) -> String; }\nimpl Hub { fn add_stage(&mut self, s: Box<dyn Stg6>) { self.stages.push(s); } }\n" };
    check(form(k2(push).f, "Stg6").includes("push-register"), `正控㉓f：⑥（add_stage 参数含 dyn）必须记 push-register（实得 ${JSON.stringify(k2(push).f.Stg6)}）`);
    const pushNeg = { "r.rs": "pub trait Stg7 { fn go(&self) -> String; }\nimpl Hub { fn add_stage(&mut self, s: Box<RuleStage>) { self.stages.push(s); } }\n" };
    check(!inList(k2(pushNeg).s, "Stg7") && !inList(k2(pushNeg).w, "Stg7"), "负控㉓f2：⑥参数不含 dyn ⇒ 不得命中（否则任何 push 都算注册口）");
    // ⑦ 自述键 + 集合筛选：**弱键控**，单独出现时绝不进强档（= 不直接进 51）
    const skey = { "r.rs": "pub trait Src7 { fn platform(&self) -> String; fn fetch(&self) -> String; }\nfn pick(list: &[Box<dyn Src7>], key: &str) -> bool {\n    list.iter().find(|x| x.platform() == key).is_some()\n}\n" };
    const sk = k2(skey);
    check(!inList(sk.s, "Src7") && inList(sk.w, "Src7") && form(sk.f, "Src7").includes("self-key-filter"), `正控㉓g：⑦（自述键 + 按 key 迭代筛选）只判**弱键控**，不进强档（实得 强=${JSON.stringify(sk.s)} 弱=${JSON.stringify(sk.w)} 形态=${JSON.stringify(sk.f.Src7)}）`);
    const skeyNeg = { "r.rs": "pub trait Src8 { fn fetch(&self) -> String; }\nfn pick(list: &[Box<dyn Src8>], key: &str) -> bool {\n    list.iter().find(|x| x.tag() == key).is_some()\n}\n" };
    check(!inList(k2(skeyNeg).w, "Src8"), "负控㉓g2：⑦trait 无自述键方法 ⇒ 不得命中（筛选的键必须来自 trait 自己）");
    // ⑧ factory-match 构造**已知实现类型**：需要 impl→trait 反查表
    const ctor = { "r.rs": "pub trait Ctor9 { fn go(&self) -> String; }\nimpl Ctor9 for Gamma { fn go(&self) -> String { \"g\".into() } }\nimpl Ctor9 for Delta { fn go(&self) -> String { \"d\".into() } }\nfn build9(k: &str) -> String {\n    match k {\n        \"a\" => { let _ = Gamma::new(); \"x\".into() }\n        \"b\" => { let _ = Delta::new(); \"y\".into() }\n        _ => \"z\".into(),\n    }\n}\n" };
    const ck = k2(ctor);
    check(inList(ck.s, "Ctor9") && form(ck.f, "Ctor9").includes("factory-match-impl"), `正控㉓h：⑧（match 块内构造同一 trait 的 ≥2 个已知实现类型、块内无 dyn）必须判强（DocumentParser 型盲点的修法；实得 ${JSON.stringify(ck)}）`);
    const ctorNeg = { "r.rs": "pub trait CtorA { fn go(&self) -> String; }\nfn build9(k: &str) -> String {\n    match k {\n        \"a\" => { let _ = Gamma::new(); \"x\".into() }\n        \"b\" => { let _ = Delta::new(); \"y\".into() }\n        _ => \"z\".into(),\n    }\n}\n" };
    const cn = k2(ctorNeg);
    check(!inList(cn.s, "CtorA") && !inList(cn.w, "CtorA") && cn.f.CtorA === undefined, `负控㉓h2：**无 impl→trait 反查表时 ⑧ 不得命中**（块内出现大写标识符 ≠ 构造该 trait 的实现类型；实得 ${JSON.stringify(cn)}）`);
    // 分档互斥 + 只出数：强档 trait 不重复计入弱档；k2 读数差异永远不进 hardFails
    const both = { "r.rs": "pub trait B1 { fn name(&self) -> String; fn go(&self) -> String; }\nstruct R { m: HashMap<String, Arc<dyn B1>>, v: Vec<Arc<dyn B1>> }\n" };
    const bk = k2(both);
    check(inList(bk.s, "B1") && !inList(bk.w, "B1"), `正控㉓i：同 trait 同时有强/弱证据 ⇒ 只计强档一次（两档互斥，否则 strongHits+weakHits 会重复计数）`);
    const fk = fixture(); fk.readings.k2 = null;
    const rk2 = reconcile(fk.register, fk.readings, fk.plan);
    check(!rk2.hardFails.some((x) => /锚点 k2Keyed/.test(x)) && rk2.reports.some((x) => x.includes("锚点 k2Keyed")), "正控㉓j：K2 读数缺失/不符时 k2KeyedHits（hard:false）**只报告**，绝不升成硬失败（「只出数不判红」的机械锁）");
  }

  // ㉔系列（任务 #58，产物滞后 vs 真漂移）：分流**只买到诊断正确性**，两档都照旧判红。
  //   锁四件事：① 未注册非空 ⇒ 专码 ARTIFACT-STALE + 点名 cargo check；② 名集合一致 ⇒ TRUE-DRIFT 且文案不变；
  //   ③ 专码**不得**把红洗成绿/降成只报告（CI 里 step() 非零即中断，伪装绿是本任务明令禁止的）；
  //   ④ 名集合数 ≠ 产物条目数 ⇒ 新不变量点名「名集合静默低估」。全部走合成输入，零盘扫描。
  {
    const mkSrc = (cmds, extra = "") => ({ "m.rs": cmds.map((c) => `#[tauri::command]\npub async fn ${c}() -> String { String::new() }`).join("\n") + extra });
    const mkReg = (cmds) => cmds.map((c) => ({ mod: "m", cmd: c }));
    // 正控㉔a：源码与产物逐条对上 ⇒ 未注册 0 / 孤儿 0 / 名集合数 == 条目数
    const ok = commandNameSets(mkReg(["a1", "a2", "a3"]), Object.entries(mkSrc(["a1", "a2", "a3"])));
    check(ok.unregistered.length === 0 && ok.orphanRegistered.length === 0 && ok.definedCount === 3 && ok.entriesCount === 3, `正控㉔a：一致时三个读数必须全等（实得 ${JSON.stringify(ok)}）`);
    check(classifyCommandCountDrift(ok).kind === "true-drift" && classifyCommandCountDrift(ok).code.includes("真漂移"), "正控㉔b：名字集合一致 ⇒ 分流为真漂移（按登记值判，口径不变）");
    // 负控㉔c：定义了两条、产物只有一条 ⇒ 未注册非空 ⇒ 判产物滞后 + 点名 cargo check
    const stale = commandNameSets(mkReg(["b1"]), Object.entries(mkSrc(["b1", "b2"])));
    const cls = classifyCommandCountDrift(stale);
    check(stale.unregistered.join(",") === "b2", `负控㉔c：已定义未注册必须逐条点名（实得 ${JSON.stringify(stale.unregistered)}）`);
    check(cls.kind === "artifact-stale" && cls.code === "[产物滞后 ARTIFACT-STALE] " && /cargo check/.test(cls.note) && cls.note.includes("b2"), `负控㉔c2：产物滞后档必须给专码 + 首条未注册名 + 直指「跑 cargo check 重生成后复跑本门」（实得 ${cls.code}${cls.note.slice(0, 60)}…）`);
    // 负控㉔d：注释里的 #[tauri::command] 不算定义（沿用 check-contracts 的剥注释口径）
    const cmt = commandNameSets(mkReg(["c1"]), Object.entries({ "m.rs": "// #[tauri::command]\n// pub fn c1() {}\npub fn helper() {}\n" }));
    check(cmt.definedCount === 0 && cmt.unregistered.length === 0, `负控㉔d：注释掉的命令不得进名集合（否则分流会把「没写」判成「产物滞后」；实得 ${JSON.stringify(cmt)}）`);
    // 负控㉔e：专码不得洗白 —— 产物滞后档**仍是一条硬失败**，且不进「只报告」
    const f1 = fixture();
    f1.readings.registry.commandTotal = 9; // 登记 10 / 实测 9
    f1.readings.commandNames.entriesCount = 9;
    f1.readings.commandNames.definedCount = 10;
    f1.readings.commandNames.unregistered = ["zz_new_command"];
    const r1 = reconcile(f1.register, f1.readings, f1.plan);
    const staleHard = r1.hardFails.filter((x) => /产物滞后 ARTIFACT-STALE/.test(x));
    check(staleHard.length >= 1 && staleHard.every((x) => x.startsWith("FAIL:")), `负控㉔e：产物滞后分流**必须仍判红**（本仓 CI 的 step() 非零即中断，专码只改诊断不改档位；不许伪装成绿）`);
    check(staleHard.some((x) => /cargo check/.test(x) && x.includes("登记 10 条 / 实测 9 条")), `负控㉔e2：红行里同时给出差值与修法（实得 ${staleHard[0] ?? "无"}）`);
    check(!r1.reports.some((x) => /锚点 commandTotal/.test(x)), "负控㉔e3：分流后的 commandTotal 差异不得同时出现在「只报告」档（同一差异不许既红又报告，档位必须唯一）");
    // 负控㉔f：名字集合一致时的差额 ⇒ 文案保持真漂移口径，且照样红
    const f2 = fixture();
    f2.readings.registry.commandTotal = 9;
    f2.readings.commandNames.entriesCount = 9;
    f2.readings.commandNames.definedCount = 9;
    const r2 = reconcile(f2.register, f2.readings, f2.plan);
    check(r2.hardFails.some((x) => /真漂移 TRUE-DRIFT/.test(x) && /锚点 commandTotal/.test(x)), `负控㉔f：名字集合一致 ⇒ [真漂移] 专码 + 原口径文案（登记/实测/差值一个不少；实得 ${JSON.stringify(r2.hardFails.filter((x) => /commandTotal/.test(x)))}）`);
    // 负控㉔g：新不变量 —— 同名跨模块只注册一个 ⇒ 名集合静默低估必须被点名
    const f3 = fixture();
    f3.readings.commandNames.definedCount = 9; // 名集合 9 / 产物条目 10（锚点登记 10）
    const r3 = reconcile(f3.register, f3.readings, f3.plan);
    check(r3.hardFails.some((x) => /锚点 commandNameSet/.test(x) && /名集合与产物条目数不等/.test(x)), `负控㉔g：名集合数 ≠ 产物条目数 ⇒ commandNameSetCount 硬失败并说明「静默低估/同名未注册」（实得 ${JSON.stringify(r3.hardFails.filter((x) => /commandNameSet/.test(x)))}）`);
    const f4 = fixture();
    check(reconcile(f4.register, f4.readings, f4.plan).hardFails.every((x) => !/commandNameSet|commandTotal/.test(x)), "正控㉔h：干净读数下两个新锚点均不假阳性（专码不得变成新的常驻红）");
    // 正控㉔i：commandTotal 的现值/现口径未动 —— 分流不引入对 registryFileLines 的依赖（±1 是口径错配，PLAN §15.1）
    check(/split\("\\n"\)\.length/.test(String(collectReadings.toString())), "正控㉔i1：registryFileLines 仍用 split 口径（未把它的 ±1 拉进滞后证据）");
    check(!/registryFileLines/.test(String(classifyCommandCountDrift)), "正控㉔i2：分流判据只看契约 A 名字集合，不引用产物行数读数");
  }

  // ㉕系列（任务 #65，`planOld[].alsoAt` 重锚区段）：区段内放行 · 段外仍红 · 段腐烂双报 ·
  //   不填 alsoAt 与旧行为逐字一致 · --repin-plan 能重锚区段（唯一才回写 / 歧义与零命中与缺键拒写）·
  //   alsoAt 的纯数字 token 仍受锚点内部自洽管辖（不得成为藏数口子）。全部零盘扫描。
  {
    const P15 = `${fixture().plan}### 15.1 HEAD 事实\nHEAD 产物 = 1540 条、按算法应生成 9 条 ⇒ 登记值 10 比 HEAD 源码多 1。\n### 15.9 段外\n这里在两个合法区段之外又提了一次 9 条。\n`;
    const PL = P15.split("\n");
    // 夹具行号自证：漂移段=3–8，段内出现=4，alsoAt 目标段=11–12（旧读数在 12），段外出现=14
    check(PL[2].startsWith("### 0.12.4") && PL[8].startsWith("### 0.13") && PL[10] === "### 15.1 HEAD 事实" && PL[11].includes("HEAD 产物") && PL[13].includes("9 条"), `㉕ 前提：P15 行号与断言一致（实得 11=${PL[10]} / 12=${PL[11].slice(0, 20)} / 14=${PL[13].slice(0, 20)}）`);
    const oldRow = (reg) => reg.anchors.find((a) => a.id === "commandTotal").planOld[0];
    const outLines = (res) => res.fails.filter((x) => /旧读数 9 出现在漂移列举段之外/.test(x));
    const rotLines = (res) => res.fails.filter((x) => x.startsWith("[区段腐烂 ALSOAT-STALE]"));

    // ㉕a 正控：出现落在登记过的健康区段内 ⇒ 不红；同一次运行里段外的出现仍红（一条都不许漏）
    const g1 = fixture();
    oldRow(g1.register).alsoAt = [{ startLine: 11, endLine: 12, mustContain: ["HEAD 产物", "9 条"] }];
    const r1 = checkPlan(g1.register, P15);
    check(rotLines(r1).length === 0, `㉕a1 正控：mustContain 全命中 ⇒ 不得报区段腐烂（实得 ${JSON.stringify(rotLines(r1))}）`);
    check(outLines(r1).length === 1 && /PLAN:14/.test(outLines(r1)[0]), `㉕a2 正控+负控：段内 PLAN:12 不红、段外 PLAN:14 仍红（实得 ${JSON.stringify(outLines(r1))}）`);
    check(/合法区段仅 3–8 \/ 11–12/.test(outLines(r1)[0] ?? ""), `㉕a3：红行的合法区段清单必须同时列出漂移段与 alsoAt 段（实得 ${outLines(r1)[0] ?? "无"}）`);
    check(r1.fails.length === 1 && r1.soft.length === 0, `㉕a4：本例 PLAN 侧只该有这一条红（实得 hard=${JSON.stringify(r1.fails)} soft=${JSON.stringify(r1.soft)}）`);
    const rr1 = reconcile(g1.register, g1.readings, P15);
    check(rr1.hardFails.length === 1 && /PLAN:14/.test(rr1.hardFails[0]), `㉕a5：经 reconcile 也是 1 条硬失败（commandTotal 是 hard:true ⇒ 段外出现不得只报告）（实得 ${JSON.stringify(rr1.hardFails)}）`);

    // ㉕b 负控：区段 mustContain 不满足 ⇒ ①单独一条「重锚腐烂」专码 ②段内出现**仍**判红 ③两类措辞可区分
    const g2 = fixture();
    oldRow(g2.register).alsoAt = [{ startLine: 11, endLine: 12, mustContain: ["HEAD 产物", "9 条", "这句盘上不存在"] }];
    const r2 = checkPlan(g2.register, P15);
    check(rotLines(r2).length === 1 && /这句盘上不存在/.test(rotLines(r2)[0]) && /PLAN:11[–-]12 已不可信/.test(rotLines(r2)[0]), `㉕b1：腐烂专码必须点名区段行号与缺失 token（实得 ${JSON.stringify(rotLines(r2))}）`);
    check(outLines(r2).length === 2 && outLines(r2).some((x) => /PLAN:12/.test(x)), `㉕b2：腐烂段内的 PLAN:12 出现**仍判红**，不得默默放行（实得 ${JSON.stringify(outLines(r2))}）`);
    check(/已腐烂.*11–12/.test(outLines(r2).find((x) => /PLAN:12/.test(x)) ?? ""), `㉕b3：段内那条红必须写明它落在腐烂段内（实得 ${outLines(r2).find((x) => /PLAN:12/.test(x)) ?? "无"}）`);
    check(!rotLines(r2).some((x) => /出现在漂移列举段之外/.test(x)) && !outLines(r2).some((x) => /^\[区段腐烂/.test(x)), "㉕b4：两类措辞互斥 ⇒ 「区段重锚腐烂」与「旧读数出段」在报告里分得开（腐烂专码只出现在它自己那一行）");
    check(!outLines(r2).some((x) => /PLAN:4/.test(x)), "㉕b5：腐烂的是 alsoAt 段，漂移段内的出现照常放行（一条腐烂不牵连另一条）");

    // ㉕c 负控：不填 alsoAt ⇒ 与改造前**逐字**一致（零放宽）
    const LEGACY_ONE = "旧读数 9 出现在漂移列举段之外（PLAN:1，且找不到含「漂移」的 §0.12.4 区段）—— 旧读数，只许在漂移段";
    const legacyFails = checkPlan(fixture().register, "正文里出现旧读数 9 条（无任何漂移字样）。\n").fails.filter((x) => /旧读数/.test(x));
    check(JSON.stringify(legacyFails) === JSON.stringify([LEGACY_ONE]), `㉕c1：未登记 alsoAt 的红行文案必须逐字等于改造前（实得 ${JSON.stringify(legacyFails)}）`);
    const ge = fixture(); oldRow(ge.register).alsoAt = [];
    const gn = fixture();
    check(JSON.stringify(checkPlan(ge.register, P15)) === JSON.stringify(checkPlan(gn.register, P15)), "㉕c2：alsoAt: [] 与不填 alsoAt 的 PLAN 对账结果逐字相同（空数组零放宽）");
    check(outLines(checkPlan(gn.register, P15)).length === 2, "㉕c3：不填 alsoAt ⇒ P15 里段外两次出现（12/14）全红（放宽前后差 2 条，正是本字段买到的唯一自由度）");

    // ㉕d 负控：段腐烂但**结构**失配（行号缺失 / 无 token）⇒ 同样给腐烂码，不放行
    const g4 = fixture();
    oldRow(g4.register).alsoAt = [{ startLine: 11 }];
    check(/\[区段腐烂 ALSOAT-STALE\]/.test(checkPlan(g4.register, P15).fails.join(" | ")), "㉕d1：区段只有行号、没有 mustContain ⇒ 无据放行，判腐烂");
    const g4b = fixture();
    oldRow(g4b.register).alsoAt = [{ startLine: 99999, endLine: 99999, mustContain: ["HEAD 产物"] }];
    const r4b = checkPlan(g4b.register, P15);
    check(rotLines(r4b).length === 1 && outLines(r4b).some((x) => /PLAN:12/.test(x)), "㉕d2：区段行号越界（PLAN 里没有那段）⇒ 报腐烂，段内出现的判定退回「段外」口径仍红");

    // ㉕e 正控：--repin-plan 能按 mustContain 把区段重锚到新行号（文档删一行 ⇒ 段跟着走）
    const arr = P15.split("\n"); arr.splice(1, 1); const shifted = arr.join("\n");
    const g5 = fixture();
    oldRow(g5.register).alsoAt = [{ startLine: 13, endLine: 14, mustContain: ["HEAD 产物", "9 条"] }]; // 表里的段指错了位置（等价 ㉑b 的漂移动手形态）
    const rp5 = computeRepin(g5.register, shifted);
    const ch5 = rp5.changes.find((x) => x.kind === "alsoAtRegion");
    check(!!ch5 && ch5.to === 11 && ch5.toEnd === 11, `㉕e1 正控：唯一最小包络 ⇒ 回写 13–14 → 11–11（实得 ${JSON.stringify(ch5 ?? null)} blockers=${JSON.stringify(rp5.blockers)}）`);
    check(!rp5.blockers.some((x) => /alsoAt/.test(x)) && !rp5.noKey.some((x) => /alsoAt/.test(x)), `㉕e2：可定位的区段不得落阻断/缺键档（实得 blockers=${JSON.stringify(rp5.blockers)} noKey=${JSON.stringify(rp5.noKey)}）`);
    const n5 = applyRepinToRegister(g5.register, rp5.changes);
    const reg5 = n5.anchors.find((a) => a.id === "commandTotal").planOld[0].alsoAt[0];
    check(reg5.startLine === 11 && reg5.endLine === 11, `㉕e3：内存版成对回写 startLine/endLine（实得 ${JSON.stringify(reg5)}）`);
    check(JSON.stringify(stripRepinEditableFields(n5)) === JSON.stringify(stripRepinEditableFields(g5.register)), "㉕e4：重锚只改行号，mustContain / note / 判据字段一字未动");
    const after5 = checkPlan(n5, shifted);
    check(after5.fails.length === 1 && /PLAN:13/.test(after5.fails[0]) && rotLines(after5).length === 0, `㉕e5：回写后腐烂归零，只剩「段外出现」这一条真红（实得 ${JSON.stringify(after5.fails)}）`);
    const mis5 = JSON.parse(JSON.stringify(n5));
    mis5.anchors.find((a) => a.id === "commandTotal").planOld[0].alsoAt[0] = { startLine: 5, endLine: 6, mustContain: ["HEAD 产物", "9 条"] };
    check(/\[区段腐烂 ALSOAT-STALE\]/.test(checkPlan(mis5, shifted).fails.join(" | ")), "㉕e6：**红线** —— 回写后再指错一行 ⇒ 判定照样红（重锚治行号漂，没把区段洗成摆设）");

    // ㉕f 负控：歧义 / 零命中 / 缺键 ⇒ 一律拒写（与 planPinned 同口径）
    const dupP15 = `${P15}### 15.8 重复\nHEAD 产物 = 1540 条、按算法应生成 9 条 ⇒ 登记值 10 比 HEAD 源码多 1。\n`;
    const g6 = fixture();
    oldRow(g6.register).alsoAt = [{ startLine: 8, endLine: 9, mustContain: ["HEAD 产物", "9 条"] }];
    const rp6 = computeRepin(g6.register, dupP15);
    check(!rp6.changes.some((x) => x.kind === "alsoAtRegion") && rp6.blockers.some((x) => /alsoAt.*非唯一命中.*\[12–12, 16–16\]/.test(x)), `㉕f1：两处同形事实句 ⇒ 两个最小包络 ⇒ 报「非唯一命中」并列出候选，绝不进 changes（实得 ${JSON.stringify(rp6.blockers)}）`);
    const g7 = fixture();
    oldRow(g7.register).alsoAt = [{ startLine: 8, endLine: 9, mustContain: ["这句盘上不存在", "9 条"] }];
    const rp7 = computeRepin(g7.register, P15);
    check(!rp7.changes.some((x) => x.kind === "alsoAtRegion") && rp7.blockers.some((x) => /alsoAt.*全文\*\*零命中\*\*/.test(x) && x.includes("这句盘上不存在")), `㉕f2：token 在 PLAN 全文零命中（句子被改写=语义变化）⇒ 拒写并点名失效 token（实得 ${JSON.stringify(rp7.blockers)}）`);
    const g8 = fixture();
    oldRow(g8.register).alsoAt = [{ startLine: 11, endLine: 12, mustContain: [] }];
    const rp8 = computeRepin(g8.register, P15);
    check(rp8.noKey.some((x) => /alsoAt.*无 mustContain 自证 token/.test(x)) && !rp8.changes.some((x) => x.kind === "alsoAtRegion"), `㉕f3：区段不带定位键 ⇒ 落「缺定位键」档并拒写（实得 ${JSON.stringify(rp8.noKey)}）`);

    // ㉕g 正控：文本版定点回写（真正落盘那条路径）——needle 唯一才改，成对改，其它字节不动
    const tableText = `{\n  "anchors": [\n    { "id": "commandTotal", "planOld": [{ "number": "9", "note": "n", "alsoAt": [{ "startLine": 13, "endLine": 14, "mustContain": ["HEAD 产物", "9 条"] }] }] }\n  ]\n}\n`;
    const regG = JSON.parse(tableText);
    const rpG = computeRepin({ ...regG, caliber: { groupCounts: { A: 0, B: 0, C: 0, H: 0, K: 0, D: 0 }, totalRows: 0, capacityRegister: 0 }, entries: [] }, shifted);
    const chG = rpG.changes.filter((x) => x.kind === "alsoAtRegion");
    check(chG.length === 1 && chG[0].to === 11 && chG[0].toEnd === 11, `㉕g1 前提：文本版同样应算出 13–14→11–11（实得 ${JSON.stringify(chG)}）`);
    const gp = planRepinPatches(tableText, regG, chG);
    check(gp.ok && gp.text?.includes('"startLine": 11, "endLine": 11') && gp.text?.includes('"mustContain": ["HEAD 产物", "9 条"]'), `㉕g2：定点替换成对回写行号、token 原样保留（实得 ok=${gp.ok} problems=${JSON.stringify(gp.problems)}）`);
    check(gp.ok && gp.text.includes('"note": "n"'), "㉕g3：未涉及的字节逐字不动（note / 结构 / 缩进没被整体重排）");
    const twinTable = `{\n  "anchors": [\n    { "id": "commandTotal", "planOld": [{ "number": "9", "note": "n", "alsoAt": [{ "startLine": 13, "endLine": 14, "mustContain": ["HEAD 产物", "9 条"] }] }, { "number": "8", "note": "n2", "alsoAt": [{ "startLine": 13, "endLine": 14, "mustContain": ["HEAD 产物", "8 条"] }] }] }\n  ]\n}\n`;
    const regT = JSON.parse(twinTable);
    const rpT = computeRepin({ ...regT, caliber: { groupCounts: { A: 0, B: 0, C: 0, H: 0, K: 0, D: 0 }, totalRows: 0, capacityRegister: 0 }, entries: [] }, shifted);
    const bad = planRepinPatches(twinTable, regT, rpT.changes.filter((x) => x.kind === "alsoAtRegion"));
    check(!bad.ok && bad.problems.some((x) => /找不到唯一的替换锚/.test(x)) && bad.text === null, `㉕g4：两段行号字面相同 ⇒ 替换锚不唯一 ⇒ 整次拒绝写盘（实得 ok=${bad.ok} problems=${JSON.stringify(bad.problems)}）`);

    // ㉕h 判据 6：alsoAt 的纯数字 token 同受锚点内部自洽管辖（挪数字不等于拆判据）
    const consReg2 = (anchors) => ({ caliber: { groupCounts: { A: 1, B: 0, C: 0, H: 0, K: 0, D: 0 }, totalRows: 1, capacityRegister: 1 }, anchors, entries: [] });
    const alsoAnchor = (mustContain) => consReg2([{ id: "x", hard: true, check: "registryCommandCount", value: 1542, planNumber: "1542", planOld: [{ number: "1541", alsoAt: [{ startLine: 1, endLine: 3, mustContain }] }] }]);
    check(checkAnchorConsistency(alsoAnchor(["HEAD 产物", "1541"])).length === 0, "㉕h1 正控：alsoAt 钉本条 planOld 自己声明的旧读数（1541）⇒ 不算矛盾（这就是该字段的用途）");
    check(checkAnchorConsistency(alsoAnchor(["HEAD 产物", "7777"])).length === 1, `㉕h2 负控：alsoAt 钉谁都不撞的数字 ⇒ 必须硬失败（实得 ${JSON.stringify(checkAnchorConsistency(alsoAnchor(["HEAD 产物", "7777"])))}）`);
    check(checkAnchorConsistency(alsoAnchor(["HEAD 产物", "7777"])).join(" | ").includes("alsoAt"), "㉕h3：矛盾行的定位必须指到 alsoAt（不能只报 planPinned）");
    check(checkAnchorConsistency(alsoAnchor(["HEAD 产物", "1540 条"])).length === 0, "㉕h4：非纯数字 token（「1540 条」）按既有 ③ 档惯例不做数字比对 ⇒ 登记方用句子片段自证即可");
    const launder1541 = consReg2([
      { id: "hard", hard: true, check: "registryCommandCount", value: 1542, planNumber: "1542", planPinned: [{ line: 1, mustContain: ["1542", "164"] }] },
      { id: "mod", hard: true, check: "registryModuleCount", value: 164 },
    ]);
    check(checkAnchorConsistency(launder1541).length === 0, "㉕h5：现盘形态（钉句只含 1542/164）在纳入 alsoAt 检查后仍不红（零新增红）");
    const moved = JSON.parse(JSON.stringify(launder1541));
    moved.anchors[0].planOld = [{ number: "1534", alsoAt: [{ startLine: 2, endLine: 4, mustContain: ["9999"] }] }];
    check(checkAnchorConsistency(moved).length === 1, "㉕h6：**藏数路径封堵** —— 把与登记脱节的数字从钉句挪进 alsoAt ⇒ 照样硬失败（alsoAt 不是判据 ②/⑮ 的新豁口）");
  }

  // ㉖系列（任务 #61，`k1DeclaredExternalImplTraitCount` 的两趟语义）：锁「定案档 = 站点面 ∩ 声明面，
  //   且交并必须在收齐之后做」。单趟那种「看到 impl 时就地查声明集」的写法会随遍历次序丢条目，
  //   它是 #61 复现并归因的**缺陷**（当前树恒少 22 条），本系列把「不许退回单趟」钉成断言。
  {
    const singlePassLike = (files) => { // 缺陷复现器：def 随遍历就地累积 ⇒ 顺序相关
      const def = new Set(); const got = new Set();
      for (const f of files) { for (const l of f) { if (l.kind === "def") { def.add(l.t); } else if (def.has(l.t)) { got.add(l.t); } } }
      return got.size;
    };
    const EARLY = [{ kind: "impl", t: "Widget" }]; // 站点文件先被扫到
    const LATE = [{ kind: "def", t: "Widget" }]; // 声明文件后被扫到
    check(singlePassLike([EARLY, LATE]) === 0 && singlePassLike([LATE, EARLY]) === 1, `负控㉖a：就地查声明集 ⇒ 同一棵树换次序就少 1 条（实得 ${singlePassLike([EARLY, LATE])}/${singlePassLike([LATE, EARLY])}）= audit 的 singlePass 缺陷，**禁止当口径**`);
    check(declaredExternalTraitNames(["Widget"], ["Widget"]).length === 1 && declaredExternalTraitNames(["Widget"], ["Widget", "Other"]).length === 1, "正控㉖b：两趟交并与遍历次序无关（站点面/声明面谁先收齐都一样）");
    const s6 = declaredExternalTraitNames(["C", "A", "B", "Std", "A"], ["A", "C", "D"]);
    check(JSON.stringify(s6) === JSON.stringify(["A", "C"]), `㉖c：定案档 = 站点面 ∩ 声明面，**去重 + 排序**逐条可点名；未声明名（Std/B）不得计入 ⇒ 与 A 档 externalImplTraits 的差就是它（实得 ${JSON.stringify(s6)}）`);
    const d6 = ["X", "Y", "Z", "W"];
    const g6 = declaredExternalTraitNames(["X", "Y", "Z"], d6);
    check(g6.length === 3 && g6.every((t) => d6.includes(t)), `㉖d：闭合恒等式 declaredExternal ⊆ pubTraitNames（#61 的 234 ≤ 318 那一档在纯函数侧同形；实得 ${JSON.stringify(g6)}）`);
    // 门内谓词侧的两趟锁（与 audit 的 `scanK1Traits` fixture 同形）：站点文件**先**被扫到仍必须计入
    const EARLY_SRC = "impl Widget for A { fn go(&self) -> String { String::new() } }\n";
    const LATE_SRC = "pub trait Widget { fn go(&self) -> String; }\n";
    const scanGateStyle = (srcs) => { // 与 collectReadings 的 K1 块同形：先收齐，遍历结束后再交并
      const def = new Set(); const site = new Set();
      for (const s of srcs) {
        for (const l of s.split("\n")) {
          const m = PUB_TRAIT_RE.exec(l); if (m) { def.add(m[1]); }
          const t = implTraitOf(l); if (t) { site.add(t); }
        }
      }
      return declaredExternalTraitNames(site, def).length;
    };
    const scanInPlace = (srcs) => { // 就地判定（单趟缺陷形态）：看到 impl 时声明集还没收齐 ⇒ 静默丢
      const def = new Set(); const got = new Set();
      for (const s of srcs) {
        for (const l of s.split("\n")) {
          const m = PUB_TRAIT_RE.exec(l); if (m) { def.add(m[1]); }
          const t = implTraitOf(l); if (t && def.has(t)) { got.add(t); }
        }
      }
      return got.size;
    };
    check(scanGateStyle([EARLY_SRC, LATE_SRC]) === 1 && scanGateStyle([LATE_SRC, EARLY_SRC]) === 1, `㉖e：门内谓词侧两趟 ⇒ impl 站点早于声明文件仍计入、换序不变（与 audit fixture 同形；实得 ${scanGateStyle([EARLY_SRC, LATE_SRC])}/${scanGateStyle([LATE_SRC, EARLY_SRC])}）`);
    check(scanInPlace([EARLY_SRC, LATE_SRC]) === 0 && scanInPlace([LATE_SRC, EARLY_SRC]) === 1, `㉖f：同一对源文件改成就地判定 ⇒ 次序一换就少 1 条（实得 ${scanInPlace([EARLY_SRC, LATE_SRC])}/${scanInPlace([LATE_SRC, EARLY_SRC])}）⇒ 两趟不可省，红线：不许把 K1 块的交并挪回循环内`);
    // 门内接线正向锁：干净读数下新锚点必须 PASS 且读数被对账（负控由 ⑭ 系列的三条 mutate 承担）
    const f6 = fixture();
    const r6 = reconcile(f6.register, f6.readings, f6.plan);
    check(r6.hardFails.length === 0 && r6.anchorResults.some((x) => x.txt.startsWith("k1DeclaredExt（") && x.status === "PASS"), `正控㉖g：fixture 干净读数下 k1DeclaredExternalImplTraitCount 必须 PASS（实得 hard=[${r6.hardFails.join(" | ")}]）`);
    const f6b = fixture(); delete f6b.readings.k1.declaredExternalTraits;
    const r6b = reconcile(f6b.register, f6b.readings, f6b.plan);
    check(r6b.hardFails.some((x) => /k1DeclaredExt.*读数缺字段 declaredExternalTraits/.test(x)), `负控㉖h：门内没出数却登记成机器种类 ⇒ 报「读数缺字段」并硬失败，不许按 undefined 静默比绿（实得 ${JSON.stringify(r6b.hardFails)}）`);
  }

  // ㉗系列（任务 #54，两处门内改动）：⑥ 收窄到「接收者是集合类型」（PLAN §24.4）+
  //   实现者读数加一列「其中在 `#[cfg(test)]` 块内」（PLAN §24.5，**只分列不改判**）。
  {
    const forms = (src) => computeK2Readings(src).detail.formsOf;
    const strong = (src) => computeK2Readings(src).strongTraits;
    const weak = (src) => computeK2Readings(src).weakTraits;
    const COLL = "pub trait StgW { fn go(&self) -> String; }\nimpl Hub { fn add_stage(&mut self, s: Box<dyn StgW>) { self.stages.push(s); } }\n";
    const SLOT = "pub trait ParsW { fn go(&self) -> String; }\nimpl Hub { fn set_parser(&mut self, p: Arc<dyn ParsW>) { SELF_PARSER.set(p).ok(); } }\n";
    const OPT = "pub trait GdW { fn go(&self) -> String; }\nimpl Hub { fn set_gd(&mut self, p: Arc<dyn GdW>) { *self.guard.write().unwrap() = Some(p); } }\n";
    const NEAR = "pub trait LtW { fn go(&self) -> String; }\nimpl Hub { fn add_thing(&mut self, s: Box<dyn LtW>) {\n  let n = self.count + 1;\n  self.items.push(s);\n  self.count = n;\n} }\n";
    const FAR = "pub trait FtW { fn go(&self) -> String; }\nimpl Hub { fn add_thing(&mut self, s: Box<dyn FtW>) {\n  let a = 1;\n  let b = 2;\n  let c = 3;\n  let d = 4;\n  let e = 5;\n  self.items.push(s);\n} }\n";
    check(strong({ "c.rs": COLL }).includes("StgW") && forms({ "c.rs": COLL }).StgW.includes("push-register"), `正控㉗a：集合接收者（同行「.push(」）⇒ ⑥ 仍进强档（动词表与 3 行参数窗口一字未动；实得 ${JSON.stringify(forms({ "c.rs": COLL }).StgW)}）`);
    check(!strong({ "s.rs": SLOT }).includes("ParsW") && !weak({ "s.rs": SLOT }).includes("ParsW") && forms({ "s.rs": SLOT }).ParsW === undefined, `负控㉗b：单槽 DI 注入口（OnceLock::set）⇒ ⑥ 不计，且不留任何形态（收窄前它会带 push-register ⇒ 这条是收窄本身的机制证据；实得 ${JSON.stringify(forms({ "s.rs": SLOT }))}）`);
    check(!strong({ "g.rs": OPT }).includes("GdW"), `负控㉗c：写进 Option / 结构体字段（「*guard = Some(x)」）⇒ 不计（实得 强=${JSON.stringify(strong({ "g.rs": OPT }))}）`);
    check(strong({ "n.rs": NEAR }).includes("LtW"), `正控㉗d：集合接收者在站点行**之后第 3 行** ⇒ 仍在「站点行 + 其后 5 行」窗口内 ⇒ 计（口径的字面边界，不外扩也不收窄）`);
    check(!strong({ "f.rs": FAR }).includes("FtW") && !weak({ "f.rs": FAR }).includes("FtW"), `负控㉗e：集合接收者在站点行之后第 7 行 ⇒ 出窗 ⇒ 不计（不许悄悄把 5 行改成 7 行；实得 强=${JSON.stringify(strong({ "f.rs": FAR }))} 弱=${JSON.stringify(weak({ "f.rs": FAR }))}）`);
    // vec![ 与 .insert(/.entry( 三个接收者标记各自可用（token 表是判据的一部分，少一个就会误降档位）
    const VEC = "pub trait VcW { fn go(&self) -> String; }\nimpl Hub { fn add_one(&mut self, s: Box<dyn VcW>) { let v = vec![s]; } }\n";
    check(strong({ "v.rs": VEC }).includes("VcW"), `正控㉗f：「vec![」算集合接收者 ⇒ 计（实得 ${JSON.stringify(forms({ "v.rs": VEC }).VcW)}）`);
    const MAP = "pub trait MpW { fn go(&self) -> String; }\nimpl Hub { fn set_entry(&mut self, k: &str, s: Arc<dyn MpW>) { self.m.insert(k.to_string(), s); } }\n";
    check(strong({ "m.rs": MAP }).includes("MpW") && forms({ "m.rs": MAP }).MpW.includes("push-register"), `正控㉗g：「.insert(」算集合接收者 ⇒ 计（单槽措辞的 setter 名不改判据，看的是接收者）`);

    // —— #54 §24.5 的 test mod 列（纯函数侧）——
    const T1 = "pub trait SvcW {}\nstruct A {}\nimpl SvcW for A {}\n#[cfg(test)]\nmod tests {\n  struct B {}\n  impl SvcW for B {}\n}\nstruct C {}\nimpl SvcW for C {}\n";
    const r1 = cfgTestRanges(T1);
    check(r1.length === 1 && r1[0].start === 5 && r1[0].end === 8, `㉗h：test 块区间 = mod 头行到闭合行（两端都含；属性行本身不算）（实得 ${JSON.stringify(r1)}）`);
    check(lineInRanges(r1, 7) && !lineInRanges(r1, 3) && !lineInRanges(r1, 10), "正控㉗i：块内站点计入、块外（前与后）计入生产账 ⇒ 两本账不串");
    // 同文件多个 test mod ⇒ 各自独立区间，中间的生产行不算 test
    const T2 = "#[cfg(test)]\nmod a {\n  impl X for A {}\n}\nimpl X for PROD {}\n#[cfg(test)]\nmod b {\n  impl X for B {}\n}\n";
    const r2 = cfgTestRanges(T2);
    check(r2.length === 2 && r2[0].start === 2 && r2[0].end === 4 && r2[1].start === 7 && r2[1].end === 9, `负控㉗j：同文件两个 test mod 的边界必须各自闭合（实得 ${JSON.stringify(r2)}）`);
    check(!lineInRanges(r2, 5) && lineInRanges(r2, 3) && lineInRanges(r2, 8), "负控㉗j2：夹在两个 test mod 之间的生产行**不得**被判进 test 列（边界不串）");
    // `#[cfg(test)]` 后跟 fn（不是 mod）⇒ 不开区间，否则后续生产站点整段被吞
    const T3 = "#[cfg(test)]\npub fn test_only_helper() {}\nimpl X for PROD {}\n#[cfg(test)]\nmod real { impl X for T {}\n}\n";
    const r3 = cfgTestRanges(T3);
    check(r3.length === 1 && r3[0].start === 5 && r3[0].end === 6 && lineInRanges(r3, 3) === false, `负控㉗k：属性后跟「fn」不开区间（那是门控的单条目，误开会把生产站点整段吞进 test 列）；紧随其后的真 test mod 仍被识别（实得 ${JSON.stringify(r3)}）`);
    // 嵌套花括号：块内的内层括号不影响闭合判定
    const T4 = "mod prod { fn f() { let _ = 1; } }\n#[cfg(test)]\nmod t {\n  fn g() { if true { let _ = 1; } }\n  impl X for T {}\n}\nimpl X for PROD {}\n";
    const r4 = cfgTestRanges(T4);
    check(r4.length === 1 && r4[0].start === 3 && r4[0].end === 6 && lineInRanges(r4, 5) && !lineInRanges(r4, 7), `㉗l：块体按花括号配平闭合（嵌套内层括号不提前截断、也不越界吞后续生产行）（实得 ${JSON.stringify(r4)}）`);
    check(cfgTestRanges("").length === 0 && cfgTestRanges("fn a() {}\n").length === 0, "正控㉗m：无 `#[cfg(test)]` 的文件 ⇒ 零区间（空文件也算），且 collectReadings 侧有 includes 短路 ⇒ 不给全树加无谓扫描");
  }

  // ㉙系列（任务 #207，前端反向复算对照）：软锚点只报告不判红 · 复算不符必印 · 复算失败必印 · 删锚/判据死亡必点名
  {
    const mkFe = () => {
      const fx = fixture();
      fx.register.anchors.push({ id: "feForeignReverse", label: "前端反向阻塞（软锚点）", value: 4, value2: 2, value2Label: "文件", check: "manual", hard: false, planMustAppear: false, feReadings: { STOCK: { points: 4, files: 2 }, OPC: { points: 1, files: 1 } } });
      fx.readings.feForeign = { basis: "worktree", commit: null, stock: { points: 4, files: 2 }, opc: { points: 1, files: 1 }, perFile: [] };
      return fx;
    };
    const fe0 = mkFe();
    const r0 = reconcile(fe0.register, fe0.readings, fe0.plan);
    check(r0.hardFails.length === 0 && r0.reports.some((x) => x.includes("前端反向") && x.includes("复算相符")), `正控㉙a：复算=登记 ⇒ 只报告档印「复算相符」且零硬失败（实得 hard=${r0.hardFails.length}）`);
    const feBad = mkFe();
    feBad.readings.feForeign.stock.points = 5;
    const rBad = reconcile(feBad.register, feBad.readings, feBad.plan);
    check(rBad.hardFails.length === 0 && rBad.reports.some((x) => x.includes("⚠ 复算不符") && x.includes("STOCK 登记 4 处 / 2 文件 / 实测 5 处")), "负控㉙b：复算≠登记 ⇒ 必印逐域差额，但软锚点不判红（本轮未升 hard；升 hard 后此档必须改判红，前置条件见锚点 note）");
    const feBad2 = mkFe();
    feBad2.readings.feForeign.opc.files = 2;
    const rBad2 = reconcile(feBad2.register, feBad2.readings, feBad2.plan);
    check(rBad2.hardFails.length === 0 && rBad2.reports.some((x) => x.includes("OPC 登记 1 处 / 1 文件 / 实测 1 处 / 2 文件")), "负控㉙c：只改**文件数**也必须被抓出（处数与文件数是两本账，少一本就是半个锚）");
    const feNil = mkFe();
    delete feNil.readings.feForeign;
    feNil.readings.feForeignError = "退出码 2（演示）";
    const rNil = reconcile(feNil.register, feNil.readings, feNil.plan);
    check(rNil.hardFails.length === 0 && rNil.reports.some((x) => x.includes("前端反向复算未出数") && x.includes("跳过 ≠ 通过")), "负控㉙d：子进程失败 ⇒ 印「未出数 —— 跳过 ≠ 通过」，不判红也不静默");
    const feNoAnchor = fixture();
    feNoAnchor.readings.feForeign = { basis: "worktree", commit: null, stock: { points: 4, files: 2 }, opc: { points: 1, files: 1 }, perFile: [] };
    const rNA = reconcile(feNoAnchor.register, feNoAnchor.readings, feNoAnchor.plan);
    check(rNA.hardFails.length === 0 && rNA.reports.some((x) => x.includes("表内无软锚点 feForeignReverse")), "负控㉙e：锚点被删而复算照跑 ⇒ 必须点名「无对照对象」（不许让删锚变成静默绿）");
    const feDead = fixture();
    const rDead = reconcile(feDead.register, feDead.readings, feDead.plan);
    check(rDead.hardFails.length === 0 && rDead.reports.some((x) => x.includes("前端反向复算未出数")), "负控㉙f：feForeign/feForeignError 双缺（collectReadings 侧 spawn 被删的死亡形态）⇒ 对照行仍必须印出 —— 打印是无条件的");
    const feSkew = mkFe();
    feSkew.register.anchors.find((a) => a.id === "feForeignReverse").value = 9;
    const rSkew = reconcile(feSkew.register, feSkew.readings, feSkew.plan);
    check(rSkew.hardFails.length === 0 && rSkew.reports.some((x) => x.includes("复算相符")), "正控㉙g：对照判据看的是 feReadings（四数一体），不是 value/value2 —— 软锚点 self-skew 不判红也不改变对照口径");
  }

  if (fail > 0) { return false; }
  console.log("SELFTEST OK：命令数改红 · 分组漂移红 · 证据行号漂移红 · 未归属非空红 · 旧读数出段红 · 禁语红 · manual 不算通过 · 接缝/远程/实现者负控 · 逐类 anchor check 负控+正控 · 锚点内部自洽（value/planNumber/mustContain）· 藏数进 hard:false/manual 锚点必红（跨锚点宽容只认 hard:true 且机器复检的锚点数）· planNumber 数字边界 · --repin-plan 重锚（唯一命中才回写 / 歧义与零命中与缺键一律拒写 / 回写后 PLAN 对账归零 / 只改行号不动判据 / 行号锚点没被降成全文模糊匹配 / 定点替换不唯一就整次拒绝）· --repin-code 代码锚点重锚（整行定位键唯一才回写 / 零命中与歧义与缺键一律拒写 / 同 token 双条目靠整行键各自解歧 / 默认 dry-run 零写盘 / 内存版与文本版不一致拒写 / 从盘上复检不过回滚 / 判定档照样红）· K2 八形态正负对照（映射型容器强 / 顺序型容器无自述键不升强 / ⑦ 只判弱键控 / ⑧ 无 impl→trait 反查不命中 / k2KeyedHits 永不升硬）· 契约 A 分流（产物滞后专码仍判红且点名 cargo check / 名集合一致才按真漂移 / 名集合低估点名 / 判据不引用产物行数）· planOld.alsoAt（段内放行 / 段外仍红 / 腐烂双报且专码可区分 / 不填该字段逐字等价旧行为 / 越界与无键判腐烂 / --repin-plan 按最小包络重锚且歧义零命中缺键拒写 / 文本版成对定点替换不唯一即整次拒写 / alsoAt 数字 token 同受锚点自洽管辖、挪数字不能拆判据）· k1DeclaredExternalImplTraitCount 两趟语义（站点面 ∩ 声明面、与遍历次序无关、未声明名不计、单趟就地判定必丢条目、读数缺字段不许按 undefined 静默比绿）· K2 ⑥ 接收者收窄（集合接收者进强 / 单槽 setter 与 Option·字段写入不计且不留形态 / 集合标记在窗口内计、出窗不计 / vec![ 与 .insert( 也算集合接收者 / 动词表与 3 行参数窗口未动）· test 块内实现单列（区间 = mod 头行到闭合行两端都含 / 块内外两本账不串 / 同文件多 test mod 边界各自闭合 / 属性后跟 fn 不开区间 / 花括号配平闭合 / 无属性零区间）· 前端反向复算对照 #207（复算相符正控 ㉙a/㉙g · 逐域差额/只改文件数/子进程失败/删锚/判据死亡 五负控 ㉙b–㉙f，全部只报告不判红）· 智能体工具面判据本体 ⑰（引号 / 裸值 / 无 domain 跨行 / 有 domain 跨行四条都计 · `//!`·`//`·`/*` 三档注释不计 · 裸标注不许吞下一行的左括号 · 插件根计入 nonHost 不并进 host）· 干净读数零假阳性 全部通过");
  return true;
}

// ── 正式跑：读数复算（一次遍历 + 内容缓存）───────────────────────────────
const loadRegister = () => {
  if (!existsSync(REGISTER_PATH)) { return null; }
  try { return JSON.parse(readFileSync(REGISTER_PATH, "utf8")); } catch { return "BAD"; }
};

/** 扫描面枚举的**唯一实现**（makeWalker 与抖动复点共用，禁另写第二套口径 —— 否则「数到的文件数」
 *  与「判定用的文件清单」不是同一件事，抖动检测本身就失去意义）。 */
function enumerateScanSurface(onFile) {
  (function walk(d) {
    let es;
    try { es = readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of es) {
      if (WALK_PRUNE.has(e.name)) { continue; }
      const p = join(d, e.name);
      if (e.isDirectory()) { walk(p); }
      else if (/\.(rs|ts|tsx|toml)$/.test(e.name)) { onFile(p.slice(ROOT.length + 1).split(sep).join("/")); }
    }
  })(ROOT);
}

function makeWalker() {
  const cache = new Map();
  const contentFor = (rel) => {
    if (!cache.has(rel)) {
      try { cache.set(rel, readFileSync(join(ROOT, rel), "utf8")); } catch { cache.set(rel, null); }
    }
    return cache.get(rel);
  };
  const files = [];
  enumerateScanSurface((rel) => files.push(rel));
  return { files, contentFor };
}

/** 抖动复点：判定跑完后重新统计一次扫描面文件清单数（只走目录树、不读内容 ⇒ 远廉于一次复算）。 */
function countScanSurfaceFiles() {
  let n = 0;
  enumerateScanSurface(() => { n++; });
  return n;
}

function collectReadings(register) {
  const { files, contentFor } = makeWalker();
  const rd = { files, contentFor, walkFiles: files.length };

  // 注册表（lib/command-domains 强制复用，禁止另写正则）
  const regText = contentFor(REG_PATH);
  let regEntries = null;
  if (regText !== null) {
    const entries = parseRegistry(regText);
    regEntries = entries;
    rd.registry = { commandTotal: entries.length, commandModules: new Set(entries.map((e) => e.mod)).size, unassigned: unassigned(entries), registryFileLines: regText.split("\n").length };
    const modNames = entries.map((e) => e.mod);
    rd.boundary = {};
    for (const [dk, d] of Object.entries(DOMAINS)) {
      const c = {};
      modNames.forEach((m) => { if (d.cmdRe.test(m)) { c[m] = (c[m] || 0) + 1; } });
      rd.boundary[dk] = { total: Object.values(c).reduce((a, b) => a + b, 0), modules: Object.keys(c).length };
    }
    rd.auditDomain = {};
    for (const m of modNames) {
      const dn = domainOf(m);
      rd.auditDomain[dn] ??= { total: 0, modules: new Set() };
      rd.auditDomain[dn].total++;
      rd.auditDomain[dn].modules.add(m);
    }
    for (const v of Object.values(rd.auditDomain)) { v.modules = v.modules.size; }
  } else {
    rd.registry = null; rd.boundary = null; rd.auditDomain = null;
  }

  const auditIn = (rel) => !rel.split("/").some((s) => AUDIT_SKIP.has(s));
  const auditFiles = files.filter(auditIn);

  // 前端体量（脚本文件名口径 + 目录口径）
  {
    let f = 0, ln = 0;
    for (const x of auditFiles) {
      if (/^src\/.*\.(ts|tsx)$/.test(x) && DOMAINS.STOCK.feRe.test(x)) { f++; ln += (contentFor(x) ?? "").split("\n").length; }
    }
    rd.frontend = { files: f, lines: ln };
    rd.frontendDir = {};
    for (const a of register.anchors) {
      if (a.check !== "frontendDirVolume") { continue; }
      let df = 0, dl = 0;
      for (const x of auditFiles) {
        if (x.startsWith(a.dir + "/") && /\.(ts|tsx)$/.test(x)) { df++; dl += (contentFor(x) ?? "").split("\n").length; }
      }
      rd.frontendDir[a.dir] = { files: df, lines: dl };
    }
  }

  // L1 反向阻塞（audit-l1-blockers.mjs:65-126 原口径：foreign = 无任何域 owner 的文件）
  {
    const ownedByAny = (x) => Object.entries(DOMAINS).find(([, d]) => d.ownedRe.test(x))?.[0] || null;
    rd.l1 = {};
    for (const [dk, d] of Object.entries(DOMAINS)) {
      const hitFiles = new Set(); let hits = 0;
      for (const x of auditFiles) {
        if (!x.endsWith(".rs") || ownedByAny(x)) { continue; }
        if (d.crates.some((c) => x.includes("crates/" + c.split("axagent-")[1] + "/"))) { continue; }
        const src = contentFor(x) ?? "";
        for (const l of src.split("\n")) {
          const t = l.trim();
          if (t.startsWith("//") || t.startsWith("///")) { continue; }
          if (d.useRe.test(l)) { hits++; hitFiles.add(x); }
        }
      }
      rd.l1[dk] = { hits, files: hitFiles.size };
    }
  }

  // 前端反向阻塞（任务 #207）：**门内不重写第二套扫描器** ⇒ 子进程调权威脚本自身的机器可读档
  // `scripts/audit-l1-blockers.mjs --fe-json`（其 §3 判据由该脚本 --selftest 锁定；工作树口径，约 3~10 s）。
  // 失败只落 feForeignError ⇒ 由 feForeignRecomputeReport 印进「只报告」档，**不进 hardFails**
  //（本轮锚点 feForeignReverse 是 check:manual / hard:false 的软锚点，先例 k2TraitFaces）。
  {
    const r = spawnSync(process.execPath, [join(ROOT, "scripts", "audit-l1-blockers.mjs"), "--fe-json"], { encoding: "utf8", maxBuffer: 1 << 26 });
    if (r.error) {
      rd.feForeignError = `子进程无法启动：${r.error.message}`;
    } else if (r.status !== 0) {
      rd.feForeignError = `退出码 ${r.status}（stderr 首行：${(r.stderr || "").split("\n")[0] || "空"}）`;
    } else {
      try {
        rd.feForeign = JSON.parse(r.stdout);
      } catch (e) {
        rd.feForeignError = `stdout 不是合法 JSON（${e.message}；前 120 字符：${String(r.stdout).slice(0, 120)}）`;
      }
    }
  }

  // K1 扫描面：8 根全部 .rs（忽略 target/node_modules/gen/tests）；实现者站点 + pub trait 一次遍历。
  // ⚠ #61 的定案档必须**两趟**（先收齐、后交并）⇒ 见下方 `declaredExternalTraits`：集合的交并放在
  //   遍历**之后**做，就是两趟语义；若在循环里就地判 `pubTraits.has(t)`，会退化成「impl 早于其声明
  //   文件出现的站点被静默丢弃」的顺序相关读数（= audit 脚本里的 `singlePass`，当前树恒少 22 条），
  //   那是 #61 明确复现并归因的**缺陷**，不是口径。
  const k1Files = files.filter((p) => p.endsWith(".rs") && K1_ROOTS.includes(p.split("/")[0]) && !p.split("/").some((s) => K1_IGNORE.has(s)));
  const pubTraits = new Set();
  const externalTraits = new Set();
  const implementors = {};
  const implementorsTestMod = {}; // #54 §24.5 新列：**只分列**「其中在 `#[cfg(test)]` 块内」，一不并入
  //   `implementors`、二不参与任何判据与退出码（升判据是另一批）；路径口径（`isTestDoublePath`）照旧剔文件，
  //   本列管的是**内联** test mod ⇒ 两本账各记各的，不许互相折算。
  const k2Sources = {}; // #57：K2 谓词与 K1 共用**同一份扫描面 + 同一份内容缓存**（禁另起一遍遍历）
  for (const x of k1Files) {
    const s = contentFor(x) ?? "";
    k2Sources[x] = s;
    const harness = x.startsWith(HARNESS_PREFIX);
    const dbl = isTestDoublePath(x);
    const testMod = !harness && !dbl && s.includes("#[cfg(test)]") ? cfgTestRanges(s) : null;
    const k1Lines = s.split("\n");
    for (let li = 0; li < k1Lines.length; li++) {
      const l = k1Lines[li];
      const pt = PUB_TRAIT_RE.exec(l);
      if (pt) { pubTraits.add(pt[1]); }
      const t = implTraitOf(l);
      if (t && !harness && !dbl) {
        externalTraits.add(t);
        implementors[t] = (implementors[t] || 0) + 1; // 既有读数与形状一字未动
        if (testMod && lineInRanges(testMod, li + 1)) { implementorsTestMod[t] = (implementorsTestMod[t] || 0) + 1; }
      }
    }
  }
  // 声明面 ∩ 站点面（遍历收齐之后）⇒ 与 `scanK1Traits().declaredExternalCount` 同一集合（#61 定案档）。
  const declaredExternal = declaredExternalTraitNames(externalTraits, pubTraits);
  rd.k1 = { rsFiles: k1Files.length, pubTraits: pubTraits.size, externalImplTraits: externalTraits.size, declaredExternalTraits: declaredExternal.length };
  // 智能体工具面（#306）：**与 K1/K2 同一份扫描面、同一份内容缓存**（`k2Sources` 由上面同一个循环填），
  //   不另起遍历、不自写第二套标注正则（判据本体在 lib，`--selftest` 与本门共用同一实现）。
  rd.agentCommand = scanAgentCommandFace(Object.entries(k2Sources));
  rd.implementors = implementors;
  rd.implementorsTestMod = implementorsTestMod; // #54 §24.5：进 `--dump-readings`，**不进**任何判据分支

  // K2 键控面读数（#57）：只出数。`computeK2Readings` 的 `detail`（trait 定义面 / impl 反查表 /
  // 逐条证据）刻意**不进 readings/dump**（否则 dump 从 9 KB 涨到数百 KB，噪声淹没判据行）；
  // 需要逐条证据的对账探针自行 import 该纯函数复算（.tmp/res57-k2-vs-198.mjs 就是这么跑的）。
  const { detail: _k2Detail, ...k2Plain } = computeK2Readings(k2Sources);
  rd.k2 = k2Plain;

  // 契约 A 名字集合（#58）：defined 侧口径逐字抄 check-contracts.mjs:105-116，registered 侧复用
  // 已有 parseRegistry ⇒ 分流「产物滞后 / 真漂移」不引入第二套解析器。文件面 = check-contracts 的
  // `walk(src-tauri/src, ".rs")`，本门已有遍历直接选边（内容走同一缓存，不额外开 IO）。
  if (regEntries) {
    const cmdSources = files.filter((p) => p.startsWith("src-tauri/src/") && p.endsWith(".rs")).map((p) => [p, contentFor(p) ?? ""]);
    rd.commandNames = commandNameSets(regEntries, cmdSources);
    rd.commandNames.scannedFiles = cmdSources.length;
  } else {
    rd.commandNames = null;
  }

  // 远程接缝清单（worker.rs:110 SUPPORTED_REMOTE_SEAMS）
  const wk = contentFor(register.sources.remoteSeamFile);
  if (wk === null) {
    rd.remoteSeams = null;
  } else {
    const lines = wk.split("\n");
    const st = lines.findIndex((l) => l.includes("SUPPORTED_REMOTE_SEAMS") && l.includes("["));
    const seams = [];
    for (let i = st; i >= 0 && i < lines.length; i++) {
      for (const m of lines[i].matchAll(/"([^"]+)"/g)) { seams.push(m[1]); }
      if (i > st && lines[i].includes("];")) { break; }
    }
    rd.remoteSeams = seams;
  }
  rd.seamRegistryText = contentFor(register.sources.seamRegistryFile);
  return rd;
}

// ── --repin-plan 的正式跑（本门**唯一**会写盘的分支：写盘前逐字节备份，写完从盘上复检，复检不过即回滚）──
function runRepin() {
  const register = loadRegister();
  if (register === null || register === "BAD") {
    console.error(`FAIL: 镜像表 ${REGISTER_PATH} ${register === null ? "缺失" : "JSON 解析失败"} —— 没有表就没有可重锚的东西。`);
    return 1;
  }
  const planRel = register.sources?.plan;
  const planAbs = planRel ? join(ROOT, planRel) : null;
  if (!planAbs || !existsSync(planAbs)) {
    console.error(`FAIL: PLAN 缺失（${planRel ?? "sources.plan 未声明"}）—— 无文本可定位 ⇒ 不重锚、不写盘。`);
    return 1;
  }
  const planText = readFileSync(planAbs, "utf8");
  const srcText = readFileSync(REGISTER_PATH, "utf8");
  const { changes, blockers, noKey, kept, total } = computeRepin(register, planText);
  const planLineCount = planText.split("\n").length;

  console.log("\n=== --repin-plan：按定位键把 PLAN 行号引用重锚到当前文本 ===");
  console.log(`受行号约束的引用 ${total} 条 · 已与 PLAN 现状逐行相符 ${kept} · 待回写 ${changes.length} · 阻断 ${blockers.length} · 缺定位键 ${noKey.length}（PLAN 现 ${planLineCount} 行）`);
  for (const c of changes) {
    const d = c.to - c.line;
    if (c.kind === "alsoAtRegion") {
      console.log(`  重锚 ${c.path}：${c.line}–${c.endLine} → ${c.to}–${c.toEnd}（起点 ${d >= 0 ? "+" : ""}${d} 行 · 段高 ${c.endLine - c.line + 1} → ${c.toEnd - c.to + 1}${c.endLine - c.line === c.toEnd - c.to ? "（未变）" : "（按 mustContain 最小包络重划边界，token 一字未动；只会朝更严的方向收紧，请过目）"}）`);
      continue;
    }
    console.log(`  重锚 ${c.path}：${c.line} → ${c.to}（${d >= 0 ? "+" : ""}${d} 行）`);
  }
  for (const b of blockers) { console.log(`  阻断 ${b}`); }
  for (const k of noKey) { console.log(`  缺键 ${k}`); }
  const uncovered = [];
  if (register.caliber?.basisLines?.length) { uncovered.push(`caliber.basisLines = [${register.caliber.basisLines.join(", ")}]`); }
  if (register.pipeline?.lines?.length) { uncovered.push(`pipeline.lines = [${register.pipeline.lines.join(", ")}]`); }
  if (uncovered.length) {
    console.log(`  ⚠ 本模式**不重锚**（无定位键的纯文档指针，行号漂了不会红也不会被改写）：${uncovered.join(" · ")} —— PLAN 行数变动后请人工核对这些数字。`);
  }

  if (blockers.length || noKey.length) {
    console.error(`\n拒绝写盘：${blockers.length + noKey.length} 条引用无法**唯一**定位 —— 不许猜行、不许插值、不许从多命中里挑一个。`);
    console.error("修法：把该项的 mustContain 补到只命中一行（或给登记项补 planAnchorText 整行文本键），再重跑本模式。");
    return 1;
  }
  if (!changes.length) {
    console.log("\nPASS: 无需重锚 —— 全部行号引用与 PLAN 现状逐行相符（未写盘、未备份）。");
    return 0;
  }

  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..+$/, "");
  const bakPath = join(ROOT, ".tmp", `capability-register.${stamp}.bak.json`);
  if (!existsSync(dirname(bakPath))) { mkdirSync(dirname(bakPath), { recursive: true }); }
  writeFileSync(bakPath, srcText, "utf8");
  console.log(`\n备份：${bakPath.split(sep).join("/")}（${srcText.length} 字符，逐字节）`);

  const patched = planRepinPatches(srcText, register, changes);
  if (!patched.ok) {
    console.error(`\n拒绝写盘（定点替换把握不足）：\n  - ${patched.problems.join("\n  - ")}`);
    console.error(`备份仍在 ${bakPath}，原表未动。`);
    return 1;
  }
  writeFileSync(REGISTER_PATH, patched.text, "utf8");
  console.log(`已回写 ${REGISTER_PATH.split(sep).join("/")}：${changes.length} 处行号（只改行号，判据一字未动）`);

  // 从盘上重读复检：表自身一致性 + 锚点自洽 + PLAN 对账（hard 与 soft 都必须为空才算「重锚成功」）
  const reread = loadRegister();
  if (reread === null || reread === "BAD") {
    writeFileSync(REGISTER_PATH, srcText, "utf8");
    console.error(`复检失败：写盘后的表无法解析 ⇒ 已从备份回滚。`);
    return 1;
  }
  if (JSON.stringify(stripRepinEditableFields(reread)) !== JSON.stringify(stripRepinEditableFields(register))) {
    writeFileSync(REGISTER_PATH, srcText, "utf8");
    console.error("复检失败：除行号外的任何字段被改动（判据被动过）⇒ 已从备份回滚。");
    return 1;
  }
  const cons = [...checkSelfConsistency(reread), ...checkAnchorConsistency(reread)];
  const plan = checkPlan(reread, planText);
  if (cons.length || plan.fails.length || plan.soft.length) {
    writeFileSync(REGISTER_PATH, srcText, "utf8");
    console.error(`复检失败：回写后 PLAN 对账仍有差异 ⇒ 已从备份回滚。\n  - ${[...cons, ...plan.fails, ...plan.soft].join("\n  - ")}`);
    return 1;
  }
  console.log(`\nPASS: 回写后复检通过 —— 钉句 ${reread.anchors.reduce((n, a) => n + (a.planPinned?.length ?? 0), 0)} 处、登记项 planLine ${reread.entries.filter((e) => e.planLine != null).length} 条全部逐行命中、planOld.alsoAt 区段 ${reread.anchors.reduce((n, a) => n + (a.planOld ?? []).reduce((m, o) => m + (o.alsoAt?.length ?? 0), 0), 0)} 段全部段内自证命中；`);
  console.log("      PLAN 侧判据已归零 ⇒ 再跑 `node scripts/check-capability-register.mjs` 时，planPinned / planLine / alsoAt 一族不应再报红。");
  console.log("      ⚠ 本模式只重锚 PLAN 行号，**不碰 evidence（源码 文件:行）** —— 代码侧漂移是该抓的缺陷，自动改写等于洗白。");
  return 0;
}

function runRepinCode() {
  const register = loadRegister();
  if (register === null || register === "BAD") {
    console.error(`FAIL: 镜像表 ${REGISTER_PATH} ${register === null ? "缺失" : "JSON 解析失败"} —— 没有表就没有可重锚的东西。`);
    return 1;
  }
  const srcText = readFileSync(REGISTER_PATH, "utf8");
  const cache = new Map();
  const fileTextFor = (rel) => { // 只读 evidence 涉及的源文件（不做全仓遍历 ⇒ 与 --repin-plan 一样廉价）
    if (!cache.has(rel)) {
      try { cache.set(rel, readFileSync(join(ROOT, rel), "utf8")); } catch { cache.set(rel, null); }
    }
    return cache.get(rel);
  };
  const rp = computeCodeRepin(register, fileTextFor);

  console.log("\n=== --repin-code：按整行定位键（evidence.anchorText）把代码证据锚点 evidence[].line 重锚到当前文本 ===");
  console.log(`受行号约束的 evidence ${rp.total} 条 · 与盘上逐行相符 ${rp.kept} · 待回写 ${rp.changes.length} · 阻断 ${rp.blockers.length} · 缺定位键 ${rp.noKey.length} · 本模式不重锚 ${rp.skipped.length}（默认 dry-run，--apply 才写盘）`);
  for (const c of rp.changes) {
    const d = c.to - c.line;
    console.log(`  重锚 ${c.path}（${c.file}）：${c.line} → ${c.to}（${d >= 0 ? "+" : ""}${d} 行）· token「${c.token}」整行键唯一命中`);
    const prose = codeRepinProseMentions(srcText, c);
    if (prose.length) {
      console.log(`    ⚠ 表内正文（note 等）另有 ${prose.length} 处仍写旧行号 ${String(c.file).split("/").pop()}:${c.line}（镜像表第 ${prose.join(", ")} 行）⇒ 本模式**不改正文**，请人工同批核对`);
    }
  }
  for (const b of rp.blockers) { console.log(`  阻断 ${b}`); }
  for (const k of rp.noKey) { console.log(`  缺键 ${k}`); }
  for (const s of rp.skipped) { console.log(`  不重锚 ${s.path}：${s.reason}`); }

  if (rp.blockers.length || rp.noKey.length) {
    console.error(`\n${APPLY ? "拒绝写盘" : "以下条目即使 --apply 也不会写盘"}：${rp.blockers.length + rp.noKey.length} 条 evidence 无法**唯一**定位 —— 不许猜行、不许插值、不许从多命中里挑一个。`);
    console.error("修法：给该条 evidence 补 anchorText = 登记当时那一行的**整行逐字**（仅折叠行内连续空白）；");
    console.error("     上面报「零命中」的那些是代码语义真的改了（不是位移）⇒ 只能人工重写 token，本模式永不代劳。");
    return 1;
  }
  if (!rp.changes.length) {
    console.log("\nPASS: 无需重锚 —— 全部代码证据锚点与盘上逐行相符（未写盘、未备份）。");
    return 0;
  }
  if (!APPLY) {
    console.log(`\nDRY-RUN：以上 ${rp.changes.length} 条就是「旧→新」计划，一个字都没写。`);
    console.log("确认无误后再跑 `node scripts/check-capability-register.mjs --repin-code --apply`（备份 → 定点补丁 → 写盘 → 从盘上复检 → 不过即回滚）。");
    return 0;
  }
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..+$/, "");
  const bakPath = join(ROOT, ".tmp", `capability-register.${stamp}.bak.json`);
  console.log(`\n⚠ --apply 覆盖的是**本次读到的原文**：若另一会话在「读取 → 写盘」之间改过这张表，复检会拦下并回滚到我读到的版本 ⇒ 对方的改动会丢。`);
  console.log("   因此 --apply 只应在**没有其它会话正在写这张表**时使用；本模式不接 CI。");
  const res = executeCodeRepin({
    register,
    srcText,
    rp,
    apply: true,
    io: {
      backup: (t) => { if (!existsSync(dirname(bakPath))) { mkdirSync(dirname(bakPath), { recursive: true }); } writeFileSync(bakPath, t, "utf8"); return bakPath; },
      write: (t) => { writeFileSync(REGISTER_PATH, t, "utf8"); },
      reread: () => loadRegister(),
      restore: (t) => { writeFileSync(REGISTER_PATH, t, "utf8"); },
      fileTextFor,
    },
    emit: (s) => console.log(s),
  });
  if (res.status === "refused-patch") {
    console.error(`\n拒绝写盘（定点替换把握不足）：\n  - ${res.problems.join("\n  - ")}`);
    console.error("原表未动、未备份（补丁先在内存里算，算不拢就不碰盘）。");
    return 1;
  }
  if (res.status === "rolled-back") {
    console.error(`\n写盘后复检不过 ⇒ 已从备份回滚（备份仍在 ${bakPath.split(sep).join("/")}）：`);
    console.error(`  - ${res.problems.join("\n  - ")}`);
    return 1;
  }
  console.log(`\n备份：${bakPath.split(sep).join("/")}（${srcText.length} 字符，逐字节）`);
  console.log(`已回写 ${REGISTER_PATH.split(sep).join("/")}：${res.changes.length} 处 evidence 行号（含 ref 末尾 :行号 同步；token / file / anchorText 一字未动）`);
  console.log("PASS: 从盘上重读复检通过 —— 复算 computeCodeRepin 已无待回写、无阻断、无缺键。");
  console.log("      再跑 `node scripts/check-capability-register.mjs` 时，evidence 一族不应再因**位移**报红；");
  console.log("      仍报红的是语义变化，那是该抓的缺陷，不许用本模式洗白。");
  return 0;
}

function main() {
  if (SELFTEST) { process.exit(runSelftest() ? 0 : 1); }
  const T0 = Date.now();
  if (REPIN && REPIN_CODE) {
    console.error("FAIL: --repin-plan（PLAN 行号）与 --repin-code（代码证据行号）互斥，请分开跑 —— 两条判据链各有自己的定位键，合跑无法逐条归因。");
    process.exit(1);
  }
  if (REPIN) { process.exit(runRepin()); }
  if (REPIN_CODE) { process.exit(runRepinCode()); }
  const register = loadRegister();
  if (register === null || register === "BAD") {
    console.error(`FAIL: 镜像表 ${REGISTER_PATH} ${register === null ? "缺失" : "JSON 解析失败"} —— 本门唯一权威就是它，跳过 ≠ 通过。`);
    process.exit(1);
  }
  const planText = existsSync(join(ROOT, register.sources.plan)) ? readFileSync(join(ROOT, register.sources.plan), "utf8") : null;

  // 只读调试档：行为与退出码逐字不变（它不做判定，故也不做抖动复点）
  if (DUMP) {
    const readings = collectReadings(register);
    const { files, contentFor, ...plain } = readings;
    console.log(JSON.stringify({ ...plain, seamRegistryText: plain.seamRegistryText ? `<${plain.seamRegistryText.length} chars>` : null, walkFiles: readings.walkFiles }, null, 1));
    console.log(`（--dump-readings 只读不落盘；耗时 ${((Date.now() - T0) / 1000).toFixed(1)}s）`);
    process.exit(0);
  }

  // 判定 + 遍历抖动守卫（口径与理由见头注「遍历抖动」段）：最多两轮；
  // 首轮头尾不一致 ⇒ 本轮不判、重试一次；重试轮仍不收敛 ⇒ 走本仓既有 skip 语义
  // （印警告 + 计入 skipped 摘要 + **不因抖动判红**，exit 码由其余硬失败决定）。
  let judged = null;
  let skipped = null;
  for (let attempt = 1; attempt <= 2; attempt++) {
    const readings = collectReadings(register);
    const res = reconcile(register, readings, planText);
    const before = readings.walkFiles; // 本趟开始时的扫描面文件清单数（makeWalker 先枚举、后才读内容）
    const after = countScanSurfaceFiles(); // 判定跑完后按同一口径复点一次
    judged = { readings, res, before, after, attempt };
    if (before === after) { break; }
    console.log(`⚠ 遍历期间盘上文件数变化（${before}→${after}），本轮不判 —— 跳过 ≠ 通过${attempt === 1 ? "；重试一次。" : "；重试后仍不收敛，按 skip 语义处理。"}`);
    if (attempt === 2) { skipped = `${before}→${after}`; }
  }
  const { res, before, after } = judged;
  const secs = ((Date.now() - T0) / 1000).toFixed(1);
  const volatility = skipped ? ` / ⚠ 抖动跳过 1 项（${skipped} 未收敛，本轮不判）` : judged.attempt > 1 ? ` / 抖动重试 1 次后收敛（${before}=${after}）` : "";

  console.log("\n=== 能力接缝登记表对账 ===");
  console.log(`登记 ${res.counts.total} 行 / 机器复检 ${res.counts.machine} / 只报告 ${res.counts.report} / 未机器复检 ${res.counts.manual}（耗时 ${secs}s，遍历 ${before} 文件 / 复点 ${after} 文件${volatility}）`);
  if (skipped) {
    console.log("── 抖动跳过 1 项（不算通过，也不因抖动判红）──");
    console.log(`⚠ 本趟判定不可信：遍历期间盘上文件数变化（${skipped}），重试一次后仍不收敛 ⇒ 跳过 ≠ 通过。`);
    console.log("   下方判据按重试轮读数逐条印出；硬失败只在「登记与盘上实测真的不符」时成立，抖动本身不新增红，exit 码由其余硬失败决定。");
  }
  // 逐条列表（第 5 条修复）：锚点 / 登记项 / PLAN / 未机器复检 / 硬失败 **一律全量** ——
  // 旧 cap=30 对 36 行登记项每次吞掉 4–6 条 PASS，被省略的恰是最需要看见的那几行；
  // 只有「只报告」这类噪声摘要仍限量 30 条并显式印剩余数（不灌水，也不假绿）。
  const list = (rows, cap = Number.POSITIVE_INFINITY) => {
    rows.slice(0, cap).forEach((x) => console.log(x));
    if (rows.length > cap) { console.log(`… 其余 ${rows.length - cap} 条省略（此档为噪声摘要，不含判据行）`); }
  };
  list(res.anchorResults.map((a) => `${a.status}: 锚点 ${a.txt}`));
  list(res.entryResults.map((e) => `${e.ok ? "PASS" : "FAIL"}: 登记项 ${e.txt}`));
  list(res.planPassed.map((p) => `PASS: ${p}`));
  if (res.reports.length) { console.log("── 只报告（不决定退出码）──"); list(res.reports, 30); }
  if (res.manualLines.length) {
    console.log(`── 未机器复检 ${res.manualLines.length} 行（不算通过，逐条印出）──`);
    list(res.manualLines);
  }
  if (res.hardFails.length) {
    console.log("");
    list(res.hardFails.map((f) => (f.startsWith("FAIL:") ? f : `FAIL: ${f}`)));
    console.error(`\n共 ${res.hardFails.length} 条硬失败 —— 镜像表与盘上实测不一致：改代码或同批改表+PLAN §0.7，不许放宽判据。`);
    process.exit(1);
  }
  console.log(`\nPASS: hard:true 判据全部一致（登记 ${register.entries.length} = ${Object.entries(register.caliber.groupCounts).map(([g, n]) => `${g}${n}`).join("/")}；能力面 Σ=${register.caliber.capacityRegister}；锚点 ${register.anchors.length} 项）。`);
  process.exit(0);
}

// main guard（阻断 2）：只有「本文件被直接执行」才跑主流程。被 `import` 时纯函数照常可用，
// 零盘扫描、零副作用、绝不 process.exit（旧版顶层即扫描 + exit，任何复算纯函数的 import 都会被
// 劫持跑一次全门，实测 33.3s）。argv[1] 缺省（管道 / -e / 被其它解释器载入）⇒ 不匹配 ⇒ 不跑。
const entryPath = process.argv[1];
if (entryPath && import.meta.url === pathToFileURL(entryPath).href) { main(); }
