# 审计：为什么「公司 / 个人 / 行业之间没有任何关联关系」（2026-09-17）

> 状态：done ｜ 日期：2026-09-17 ｜ 对应版本：v2.9.7


> 结论先行：**用户观察成立**。库里这些关系**存在且完好**（跨类型边 83,131 条），
> 但它们在进入 Wiki 图谱时被**整条切断** —— 因为实体被一次「跨知识库合并」搬走了，而**边没跟着搬**。
> 另有第二个独立成因：唯一还在画的 `link` 边，颜色 token 与画布底色差 ≈ 1 个色阶，**落屏几乎不可见**。
>
> **全程只读**（6 个 PG 探针 + 1 个像素探针）；未改任何代码与数据；修复方案需产品语义裁决。

---

## 1. 数据事实（DB 真值，非推测）

`wikis` 绑定关系：

| wiki | knowledge_base_id |
|---|---|
| `dc8efbd8-…`「A股知识」 | **`f9b2b050-…`**（KB 名「A股知识图谱」） |
| `935fabab-…`「OPC 业务知识库」 | `null` |

跨类型关系（`knowledge_relations` JOIN `knowledge_entities`，端点全 active）：

| 源类型 → 目标类型 | 行数 |
|---|---|
| person → company | 74,705 |
| company → concept | 28,270 |
| company → industry | 9,929 |
| 其它 | 33 |
| **合计** | **112,937** |

⇒ **关系一条都不缺**。「公司—个人（任职）」「公司—行业（所属）」在库里都是足量存在的。

## 2. 断裂点：实体与边的**域列（`knowledge_base_id`）不再一致**

| 量 | `f9b2b050`（wiki 绑定的域） | `lemonhu_knowledge_graph`（对照域） |
|---|---|---|
| 实体行（全量） | 24,274 | 46,137（部分为软删） |
| 实体行（**可见**，`lifecycle IS NULL`） | **4** | **22,608** |
| 关系边（本域） | **38,171** | 74,766 |
| **两端都可见的边** | **0** | **74,490（99.6%）** |
| 边端点的实际归属 | 38,171 条 **全部**属 lemonhu | 自洽 |

⇒ Wiki 融合（`commands/wiki.rs:1038-1173`）的四步在 `f9b2b050` 域下产出：

```
entity_nodes_visible         = 4
reference_edges              = 38171
edges_src_visible            = 0
edges_both_endpoints_visible = 0
```

**前端在 `GraphView.tsx:3278` 对端点缺失的边 `continue`（静默丢弃）** ⇒ 38,171 条关系**一条都不画**。
这 38,171 条**恰恰就是**「公司/个人/行业」之间的关系 —— 所以用户看到「没有任何关联」。

## 3. 根因链（代码级，含行号）

| # | 位置 | 缺陷 |
|---|---|---|
| 1 | `dao/repo/knowledge_graph.rs:1267-1273` | `merge_duplicate_entities_across_all` 分组键 = `(name, entity_type)`，**不含 `knowledge_base_id`** ⇒ 跨知识库合并（kb 被当标签用，实为命名空间） |
| 2 | 同函数 `:1287` | 主实体取 `created_at` 最早者 ⇒ lemonhu 的实体胜出，`f9b2b050` 的 24,270 行被写 `lifecycle`（软删） |
| 3 | `update_relation_references` `:1386-1393` | 只 `UPDATE … source_entity_id / target_entity_id`，**不改边的 `knowledge_base_id`** ⇒ 边留在原域、端点去了新域 |
| 4 | `commands/wiki.rs:1108-1127` | 融合时节点与边**用同一个 kb 过滤** ⇒ 域不一致即全悬空 |
| 5 | `GraphView.tsx:3278` | 悬空边**静默丢弃**（无日志、无计数、无提示）⇒ 用户侧无任何线索 |
| 6 | 触发源 `init/services.rs:167-198` | 定时任务 `knowledge_consolidation`，**每 6 小时**执行，第一步就是它；另有手动命令 `commands/knowledge_graph.rs:455` |

**「为什么是现在」**：合并写 `updated_at = now` ⇒ 实测合并时刻 **`2026-09-15T19:59:35Z`**
（= 2026-09-16 03:59:35 +08），两侧共 **50,204 行同时**写入（`count(DISTINCT updated_at) = 1`，单次事件）。

**可逆性（先量再谈修）**：`lifecycle->>'merged_into'` 的反向映射**不是 1:1** ——
fanout 分布最大到 **11**，另有 772 组是 2:1 ⇒ 简单「把 lifecycle 清空」恢复不了，反向重指有歧义。

**现有「修复图谱」按钮无效**：`repair_wiki_graph`（`commands/wiki.rs` 中 `pub async fn repair_wiki_graph` 起至文件末尾）只重建 `note_links`
+ 按 `{wiki.name}图谱` 名字自动绑 KB，对本缺陷**零覆盖**。

## 4. 第二个独立成因：`link` 边落屏对比度 ≈ 1 个色阶

笔记层（现在唯一还在画的层）数据是**健康的**：24,288 笔记 / **74,791** 条 `wikilink`，两端全有效（dangling = 0）。

把 `note.title` join 到 `knowledge_entities.name`（active）后算交叉类型矩阵：

| 源 → 目标 | 行数 |
|---|---|
| company ↔ person | 49,532 |
| company ↔ concept | 18,747 |
| company ↔ industry | 6,225 |
| company ↔ company / person ↔ person | 255 |
| 无同名实体 | 26 |

（有效样本率 24,279/24,288 = **99.96%**，矩阵可信。）

**但**：`getEdgeTypeStylesMap`（`graphViewUtils.ts:422`）把 `link` 边色定为
**`token.colorBorderSecondary`**（antd 里**最浅的分隔线** token，深色主题 ≈ `#303030`），
绘制时再乘 `globalAlpha = zoom<0.3 ? 0.12 : zoom<0.5 ? 0.2 : 0.3`（`GraphView.tsx:3323-3324`）。

像素实测（解已有截图，非目测）：

| 量 | 值 |
|---|---|
| 画布底色主色 | `rgb(31,26,23)`，亮度 26.85 |
| 与底色**完全相同**的像素 | 48.6% |
| 亮度差 **0~3%** 的像素 | **0** |
| 最弱的非背景内容 | 落在 **3~4%** 区间（≈ 1 个色阶） |
| fit 态边采样率 `edgeRate` | **0.1604**（只画 16%） |

⇒ 落在深色背景上的边与背景差约 1 个色阶，**肉眼不可分**；
而 zoom=5 的截图里边清晰可见（因为它们叠在**较亮的气泡填充**上）。
⇒ fit 态整屏呈「互相孤立的彩色气泡、一条线都没有」——这与用户的描述完全一致。

## 5. 修复选项（**未动手，须裁决**）

| 方案 | 做法 | 代价 / 风险 |
|---|---|---|
| **A. 只改数据** | 把 wiki 的 `knowledge_base_id` 改指 `lemonhu_knowledge_graph`（那侧 99.6% 边可解析） | 弃用 wiki 自己的 KB；`f9b2b050` 的 38,171 条边成孤儿；语义上是「绕过」不是「修好」 |
| **B. 根治代码（推荐）** | ① 合并分组键加 `knowledge_base_id`（kb 是命名空间）；② 若坚持跨库合并，则**边的域列必须与端点一起迁**；③ 补验收：合并后每个域 `edges_both_visible / edges` **不得下降**（只看 `entities_merged` 计数会全绿） | 改动面 = DAO + 定时任务；存量 24,270 行的恢复需单独脚本（且因 fanout 非 1:1，需人工确认策略） |
| **C. 读端放宽** | 融合时端点不按 kb 过滤（纯按 id 解析） | 破坏 KB 隔离语义；跨库因果边（`kb = 空串`）会漏进检索（该风险在 `graph_enhanced_search` 注释里已有前科） |
| **D. 视觉侧（独立于 A~C，建议同时做）** | `link` 边不要用 `colorBorderSecondary`；给悬空边加**计数日志**（现在静默丢弃） | 低风险；但「边色」属观感决策，需你定 |

## 6. 复现资产（全只读）

`output/verify-graph-2026-09-15/`：

| 文件 | 作用 |
|---|---|
| `kg-relation-probe-0917.{mjs,json,txt}` | 规模 + 类型矩阵 + **第一版 dangling（口径错，假绿灯 0）** |
| `kg-relation-probe2-0917.*` | 口径修正：JOIN 带 `lifecycle IS NULL`，按 kb 拆 |
| `kg-relation-probe3-0917.*` | ★ 复现后端返回集：`4 节点 / 38171 边 / 0 条两端可见` |
| `kg-relation-probe4-0917.*` | 笔记层：`wiki_graph_cache` 内容、`note_links` 端点健康度 |
| `kg-relation-probe5-0917.*` | wikilink 交叉类型矩阵（笔记 title → 实体类型） |
| `kg-relation-probe6-0917.*` | ★ 合并时间戳 `2026-09-15T19:59:35Z`、可逆性、对照域健康度 |
| `analyze-edge-visibility-0917.{mjs,json,txt}` | 截图像素：底色主色 / 亮度差分桶 / 灰vs有色像素 |
| `probe-before-fit.png` | 视觉证据：24288N 74791E 但整屏只有孤立气泡 |

## 7. 我自己的错误（留痕）

**第一版 dangling 判据给出假绿灯**：`LEFT JOIN knowledge_entities`（**全表**）⇒
`relations_with_dangling_endpoint = 0`，差点把根因判成「端点都好、问题在前端」。
读端真实口径带 `lifecycle IS NULL` ⇒ **「端点行存在」≠「端点在读端可见」**。
修正方式：把读端的**每一个**过滤条件搬进 JOIN。

---

## 8. ⚠ 归因更正（2026-09-17 晚，第二轮实测 —— **推翻 §3 的第 1、2 条归因**）

§3 把根因判成「`merge_duplicate_entities_across_all` 跨库合并**把实体搬走**」。
第二轮实测推翻：**这次跨域合并是一次已验收数据修复的产物，不是定时任务跑出来的 bug**。

### 8.1 决定性证据（全只读）

| 判据 | 实测 | 读法 |
|---|---|---|
| `lifecycle` 的**字段形状** | `{merged_into, merged_at_ms, group_mention_sum}` | **`group_mention_sum` 在全仓 Rust 里 0 命中** ⇒ 不是本仓合并函数写的 |
| `merged_at_ms` 精度 | `1789502375024`（真毫秒），而本仓代码写 `now * 1000`（末尾恒为 `000`） | 同上 ⇒ 另一个写者 |
| 副本行 vs 主实体 | 24,270 行 **100%** 找得到主实体，且 **100% 同名同类型** | 它们是**真重复副本** |
| 主实体所属库 | **100% 在 `lemonhu_knowledge_graph`**（`count(DISTINCT kb) = 1`） | 合并是「去重」，不是「搬家」 |
| `f9b2b050` 的 38,171 条边 | **37,375 条（97.9%）** 与 lemonhu 域三元组**完全相同** | 边也是同一批导入的副本 |
| 施工记录 | `docs/plans/PLAN-declarative-schema-sync.md:682` | 「实体 UPDATE（kb 还原 + 写 `lifecycle={merged_into, merged_at_ms, group_mention_sum}` + 复活 1073 幸存者）**51277**」 |
| 同文档 `:672` | 「关系表 kb 列……**40% 与源端实体 kb 不一致，属独立既存问题**」 | **本缺陷早被登记**，只是一直没修 |

⇒ **正确因果链**：

1. 2026-09-16 的一次数据修复（`output/tmp-p1-merge-repair-apply.mjs`，验收全绿）把 `f9b2b050` 的
   24,270 个**副本行**标 `lifecycle`（去重），边端点重定向到 lemonhu 的主实体；
2. 它**没有**同步迁移 `knowledge_relations.knowledge_base_id`（该文件自己登记为「独立既存问题」）；
3. ⇒ 边留在 `f9b2b050` 域、端点去了 lemonhu ⇒ 在那个域里**一条也画不出**；
4. 合并函数的「跨库分组键」是**第二个独立缺陷**（与本次数据无关，但会持续制造同型状态）—— 已修（§9.1）。

⇒ §3 表第 1、2 条归因**不成立**（那次合并不是定时任务跑出来的）；第 3~6 条仍然成立。

### 8.2 因此「复活 24,270 行」被**否决**

第一版 dry-run（`kg-domain-restore-dryrun-0917.mjs`）主张的「复活 + 按域还原」会：

* 把 24,270 个**重复副本**重新变成活跃实体 ⇒ 破坏 09-16 修复的验收判据「每组活跃行恒为 1」；
* 它在**宽松口径**下预测自洽率 **100.00%** —— 这个「漂亮数字」恰恰是**判据问错了**的症状：
  该口径默认「每个副本都该是独立实体」，而事实是「它们本来就是一回事」。

该脚本**保留作反例**；它的 Q5b/Q5c 采样（`lifecycle` 形状与时间戳精度）正是本次更正的关键证据。

---

## 9. 本轮落地（用户 2026-09-17 裁决：**B + D**）

### 9.1 B —— 代码根治

| 文件 | 改动 |
|---|---|
| `crates/harness/src/graph_dtos.rs` | 新增 `DanglingEdgeSummary` + `collect_dangling_edge_stats`；`GraphData` 增 `dangling_edges`（**总是序列化**，与 `unresolved_*` 的 skip 约定刻意相反）；`refresh_unresolved` 同时刷三类统计；`UNRESOLVED_SAMPLE_LIMIT` → `SAMPLE_ID_LIMIT` |
| `crates/dao/src/repo/knowledge_graph.rs` | ① 合并分组键 → `(knowledge_base_id, name, entity_type)`；② 跨库同名组**登记不合并**（`cross_kb_groups_skipped` + `warn`）；③ 新增 `KbDomainRow` / `audit_kb_domain_consistency` / `dangling_regressions` / `sum_dangling`；④ 合并前后**逐域**比对，任一域悬空上升即 `rollback` + `Err`；⑤ `update_relation_references` 改参数化 builder + **真实 `rows_affected`**（原返回值恒为伪造的 1/0） |
| `src-tauri/src/init/services.rs` | 定时任务新增「步骤 0：域一致性体检」（每轮全库审计 + 逐域告警）；合并失败日志 `warn` → `error` |
| `src-tauri/src/commands/wiki.rs` | 融合返回前对悬空边 `warn`（`should_warn_dangling` 按 `(边数, 悬空数)` 去重，**不按时间节流** —— 时间节流会在数据变化时闭嘴） |

### 9.2 D —— 视觉 + 可观测

| 文件 | 改动 |
|---|---|
| `graphViewUtils.ts` | `link` 边色 `colorBorderSecondary` → `colorTextTertiary`；`backlink` `colorBorder` → `colorTextSecondary`（**不动 `link.width`**：`GraphView.tsx` 的 `em.width / 0.4` 把它当全体边的相对线宽基准） |
| `GraphView.tsx` | `GraphData` 加 `danglingEdges` + `DanglingEdgeSummary` TS 类型；`idSet` 丢弃点加注释说明**为何不在此处计数**（视口级口径 ≠ 后端悬空口径，两个判据混用必然报假不一致） |
| `WikiGraphPage.tsx` | 新增悬空边提示条（`missingBoth > 0` 时用 `error` 级） |
| `i18n/locales/*.json` | 11 语言 × 4 key（`wiki.graph.danglingEdges.*`）；插入后 key 覆盖自检 `missing = 0` |

---

## 10. 数据侧：**仍未动手**，等一句裁决

`kg-edge-ownership-repair-dryrun-0917.mjs` 三档（全部只读预测）：

| 档 | 动作 | 影响 |
|---|---|---|
| **(i) 归域** | `UPDATE knowledge_relations SET knowledge_base_id = <端点的库>` | 37,889 条归 lemonhu；**282 条两端跨库**无法归属（需裁决） |
| **(ii) 归域 + 去重** | 再删掉与 lemonhu 完全相同的三元组 | 删 37,383 条，保留 788 条（其中 **767 条为 `f9b2b050` 独有**，关系类型全是 `mentions`） |
| **(iii) 整批删除** | 删 `f9b2b050` 的 38,171 条 | 会丢那 767 条独有关系 |

**无论选哪档，wiki 都必须改绑 lemonhu** —— 否则 `f9b2b050` 域只有 4 个节点、0 条可画边：

| 绑定 | 节点 | 边 | **画得出** |
|---|---|---|---|
| 当前 `f9b2b050`「A股知识图谱」 | 4 | 38,171 | **0** |
| `lemonhu_knowledge_graph`「开源股票知识库」 | 22,608 | 74,766 | **74,490** |

---

## 11. 第二轮我自己的错误（留痕）

1. **§3 的归因错了**（把一次已验收的数据修复读成定时任务 bug）—— 因为**只读当前代码、没读 `docs/plans/` 的施工记录**，
   也没查 `lifecycle` 的**字段形状**（`group_mention_sum` 这个「作者指纹」一秒就能证伪）。
   ⇒ 教训：**动一处数据形态之前，先把「谁写的、什么时候写的」查清**；字段形状与时间戳精度都是指纹。
2. **第一版 dry-run 的分桶不互斥**（`B ⊆ C` ⇒ 三类之和 72,572 ≠ 38,171），给出「看着合理但读不出结论」的数。
3. **第一版歧义判据恒假**：`s_n` 取自「只含 n=1 组」的子集 ⇒ `s_n > 1` 永远 false，报「歧义 0 条」，
   而同脚本 Q4 正列着 1,074 个 n>1 的组 —— **同一份报告自相矛盾**，是判据错了的典型信号。
4. **Q5 无损判据问错了**：`updated_at == created_at` 报 0 —— 合并**必然**改 `updated_at`，该判据恒假。
   正确判据是「末次写入是否就是合并那一次」（`updated_at*1000` vs `merged_at_ms`），
   而它恰好暴露了真写者（**差 24 毫秒** ⇒ 非本仓代码）。
5. 另一处判据口径错：`Expr` 不在 `sea_orm::*` 的再导出里 ⇒ 编译期报错（已按本仓惯例显式 `use`）。

---

## 12. ⚠ 备份真相：`output/backup-2026-09-17-graph10/` **不是回滚点**（第三处自曝）

收尾核对 sha256 时发现：**备份副本的内容是「改动之后」的文件**，不是改前的。

**两条独立判据**：

1. **哈希**：18 个文件里 **16 个**与当前工作区**逐字节相同**；仅 `graph_dtos.rs` / `knowledge_graph.rs` 不同
   （原因是其后跑了 `cargo fmt --all`，恰好只格式化这两个 Rust 文件）。
   若备份取样于改动之前，不可能 16 个全部相等。
2. **内容标记**：备份副本里**已含本轮新增物** —— `DanglingEdgeSummary`、`colorTextTertiary`、
   `danglingStats`、`danglingEdges`、`audit_kb_domain_consistency`、`should_warn_dangling` 全部 HIT。
   「改前版本」里不可能存在这些字符串。

⇒ 执行顺序实为 **先 Edit、后 cp**（我把顺序记反了，并在其后的记录里按「已备份」陈述）。
⇒ 备份副本自身完整性 OK（18/18 与 `CHECKSUMS-graph10-0917.txt` 吻合），但**职能是「改动后快照」而非回滚点**。

**已做的补救**：在备份目录写入 `README-NOT-A-ROLLBACK-POINT.md`，内含
① 本条结论与两条判据；② **逐文件反向还原清单**（18 文件 × 改动点 + 改后行号 + 反向操作）。
本轮改动**全部为增量**（唯一的值变更 = `graphviewUtils.ts` 两个颜色 token），因此该清单可完整反向。

**教训（可复用）**：「先备份再改」这句话本身需要判据 —— 备份完成后必须**立刻验证
「备份副本 ≠ 工作区」**（有差异才证明取样在改动之前）；若全部相等，说明取样发生在改动之后，须重做。
仅凭「我执行了 cp」不能断言「回滚点已备」。
另：备份目录**不要**用 `src-tauri-crates\a.rs` 这类路径压平命名而不留映射表（本次靠 CHECKSUMS 反推才对齐）。

---

## 13. 数据侧执行：用户裁决 **(ii) 归域 + 去重**（2026-09-18 凌晨，**已落库**）

### 13.1 精确分桶（`kg-repair-plan2-0918.mjs`，只读）

| 桶 | 定义 | 数量 |
|---|---|---|
| **A 删除** | 三元组在 lemonhu 域已存在 ⇒ 是副本 | **37,383** |
| **B 归域** | 两端同属 lemonhu 且两端都可见 ⇒ `kb` 改为 lemonhu | **782**（去重后 **761**） |
| **B′ 删除** | 桶 B 内部的同三元组多余行（保留 `(created_at,id)` 最小） | **21** |
| **C 不归** | 两端同 lemonhu 但含**非活跃**端点（归了会让 lemonhu 域 `dangling` 上升） | **0** |
| **D 留** | 两端跨库（`lemonhu → f9b2b050`） | **6** |
| **E/F 留** | 两端同属 f9b2b050 / 端点属第三库 | **0 / 0** |
| | 合计 | **38,171** ✓ |

> 桶 D/F 的另一条事实：跨库 282 条里 **276 条属桶 A**（被删），只有 6 条留下。
> 留下的这 6 条端点跨库、无法归属，**保持原 kb 不动**，登记为已知悬空（f9b2b050 域最终 `edges=6 / dangling=6`）。

### 13.2 执行与结果（`kg-repair-apply-0918.mjs --apply`）

三条语句的真实 `rowCount` 与预测**逐条吻合**：

| 步骤 | 语句 | 期望 | 真实 affected |
|---|---|---|---|
| 1 | DELETE 桶 A | 37,383 | **37,383** |
| 2 | DELETE 同域重复多余行 | 21 | **21** |
| 3 | UPDATE 归域（`kb` → lemonhu，同时更新 `updated_at`） | 761 | **761** |

逐域审计 before → after：

| 域 | 可见节点 | 边 | 画得出 | 悬空 |
|---|---|---|---|---|
| f9b2b050 | 4 → 4 | 38,171 → **6** | 0 → 0 | 38,171 → **6** |
| lemonhu | 22,608 → 22,608 | 74,766 → **75,527** | 74,490 → **75,251** | 276 → **276** |

全库总边数 112,937 → **75,533**（= 删 37,404 + 保留 75,533）。

**六项后置断言全绿**（提交前跑）：① f9b2b050 余 6 条；② ★ lemonhu 域 `dangling` **未上升**（276→276）；
③ 可画边 75,251；④ ★ **归域未给 lemonhu 引入任何新重复三元组**（既存 12,342 条不变）；
⑤ 总边数恒等；⑥ 外键完整性（无指向不存在实体的边）。
提交后复核：**三条语句重跑命中 0 行 ⇒ 幂等**。

### 13.3 回滚能力**实测**（不是声称）

`kg-repair-rollback-0918.mjs` 不靠「反向 UPDATE」推导（apply 后「两端同属 lemonhu 且可见」对新旧行同样成立，
反推会误伤原有 74,766 条），而是**逐 id 对齐备份**：库里缺 → INSERT；kb 与原值不同 → UPDATE 回原值。

| 时点 | 库中缺失 | kb 不同 | 一致 | 结论 |
|---|---|---|---|---|
| apply **前**（基线自证） | 0 | 0 | 38,171 | 备份 = **改前快照**（与 §12 的教训形成对照） |
| apply **后** | **37,404** | **761** | 6 | 与操作量精确吻合 |

事务内真跑还原后：f9b2b050 回到 38,171 条 / 画得出 0，lemonhu 回到 74,766 条 ⇒ **逐域均回到 apply 前原状**。
备份：`output/backup-2026-09-18-kg/kg-f9b2b050-edges-0918.json`（38,171 行 × 13 列全字段）。

### 13.4 ⚠ 缓存真相：两处假设被实测推翻（`kg-cache-dangling-probe-0918.mjs`）

解析了 `wiki_graph_cache` 里 `dc8efbd8`（wiki「A股知识」）那份**用户当时看到的图**
（`computed_at` = 2026-09-17T05:17:09Z，即用户报告那次访问时写入）：

| 判据 | 实测 |
|---|---|
| 自报 vs 实测 | `node_count` 24,288/24,288 ✓ · `edge_count` 74,791/74,791 ✓ |
| 节点类型 | **只有 `note` 一种**（24,288 个） |
| 边类型 | **只有 `link` 一种**（74,791 条） |
| 悬空边 | **0 条**（可画率 100%） |

⇒ **推翻假设一**：「缓存里混着悬空边」错 —— 这份图自身完全自洽。
⇒ **推翻假设二**：「修完必须失效图谱缓存」错 —— 缓存只存**笔记部分**，实体是
`get_wiki_graph_cached` 第 3 步**每次实时融合**的，内存缓存以 `kb_id` 为 key（`ENTITY_CACHE_TTL=30s`）；
改绑 ⇒ key 变 ⇒ 必 miss ⇒ 下次请求即用新数据。**清缓存与本题无关**。
⇒ **真实机制**：融合层**一条知识图谱边都没进来**。判据极简 —— **边类型只有 `link` 一种**；
若实体的边（`mentions` 等）进来了，边类型集合不可能只有一种。节点类型同理只有 `note`。
⇒ 用户观感「公司/个人/行业之间没有任何关联」由此完整解释：图上只有**笔记↔笔记**的连线，
没有任何**实体↔实体**的连线。

### 13.5 待裁决：③ wiki 改绑（**未执行**）

绑定在 `wikis` 表：`dc8efbd8-3a28-4fdf-9c96-9d26e2b2dac9`（name「A股知识」）→
`knowledge_base_id = f9b2b050-…`。改绑语句只有一条：

```sql
UPDATE wikis SET knowledge_base_id = 'lemonhu_knowledge_graph',
                 updated_at = (floor(extract(epoch from clock_timestamp())*1000))::bigint
 WHERE id = 'dc8efbd8-3a28-4fdf-9c96-9d26e2b2dac9';
```

⚠ 改绑后图规模翻倍，需复核渲染：24,288 + 22,608 = **46,896 节点**、
74,791 + 75,251 = **150,042 条可画边**（现为 24,288 / 74,791）。本仓有过
「大图首屏永久空白」「聚合布局环状」类历史问题（见 `PLAN`/任务 #28~#34），届时需复测。

### 13.6 本轮我自己的错误（第四批，留痕）

1. **桶 B 查询漏 `NOT EXISTS` 桶 A** ⇒ 预测 782 而集合实际 37,889 行。
   被**前置断言**当场抓住（这才让断言有了价值 —— 若没有这条断言，`UPDATE ... WHERE` 会命中 37,889 次，
   把 37,107 条**本应删除**的行也搬进 lemonhu，制造出 3.7 万条重复）。
2. **审计 SQL 的 `vis` 没按 `kb` 关联** ⇒ 「端点可见」退化成「端点在**任意库**里活跃」，
   每个域的 `dangling` 恒为 0，审计报「全绿」。这与 Rust 侧口径不一致。**同一个坑第三次踩**
   （2026-09-17 只读探针第一版、本轮脚本第一版），已把口径写进脚本注释。
3. **写结论时没先看「缓存里存的到底是什么」**就断言「必须失效缓存」——
   实测节点类型只有 `note`、边类型只有 `link`，缓存压根不含实体。**先看构成，再下结论**。
4. 分桶判据重叠（桶 D 与 F 都把「跨库」算进去）⇒ 加总 38,177 ≠ 38,171。
   **加总不闭合就是要找重叠的信号**，修判据后闭合。

## 14. 执行 ③ wiki 改绑（2026-09-18 凌晨，**已落库**）

### 14.1 执行器与前置断言

`output/verify-graph-2026-09-15/kg-rebind-apply-0918.mjs`（dry-run 默认 / `--apply` 提交）。
5 条前置断言任一失败即中止、不进入写路径：A1 当前绑定仍是旧值 · A2 目标库存在 ·
A3 既有 `updated_at` 全是 10 位 · **A4 操作有意义**（旧绑定可画边 == 0 且新绑定可画边 > 0）·
A5 图谱缓存只含 `note`/`link` ⇒ 无需清缓存。

读端唯一，已核实：`src-tauri/src/commands/wiki.rs:1102-1107`
（`get_wiki(db, wiki_id)` → `w.knowledge_base_id`，`.unwrap_or_else(|| wiki_id.clone())` 兜底）。
`wikis.knowledge_base_id` 上**无外键**（自由文本），改它不触发级联。

⚠ **执行前抓到我自己 §13.5 那条 SQL 的单位错**：`wikis.created_at/updated_at` 存的是**秒**
（10 位；写者 `crates/dao/src/repo/wiki.rs:48` 与 `:119` 都是 `Utc::now().timestamp()`），
而 §13.5 备的是 `floor(extract(epoch from clock_timestamp())*1000)` = **毫秒**（13 位）
⇒ 照抄执行会把该行写成 ≈ 公元 58600 年。已改为秒，并加「写入值必须是 10 位 + 单调不减」两条断言。

### 14.2 结果

| 项 | 改前 | 改后 |
|---|---|---|
| `wikis.id=dc8efbd8` 的 `knowledge_base_id` | `f9b2b050-…` | **`lemonhu_knowledge_graph`** |
| `updated_at` | `1786463658` | `1789661740`（10 位，秒） |
| wiki 图谱节点 | 24,292 | **46,896** |
| wiki 图谱边 | 74,798 | **172,926** |
| 可画边 | 74,792 | **172,648** |
| 实体侧可画边 | **0** / 6 | **75,251** / 75,527 |
| 悬空边 | 6 | 276（lemonhu 既存，非本轮引入） |

真实 `rowCount` = 1；幂等重跑命中 0 行；实体总数 72,816 与关系总数 75,533 **两侧均未变**；
逐域审计**逐域比对**后 0 个域发生变化（本操作不碰知识图谱）；`wiki_graph_cache` 的
`computed_at/node_count/edge_count` 三者均未变 —— 自证「缓存只存笔记部分、实体每次实时融合」。

### 14.3 回滚能力**实测**（不是声称）

`kg-rebind-rollback-0918.mjs`（默认只比对，`--apply` 全列还原）：

| 时点 | 结果 |
|---|---|
| apply **前** | **0 差异** ⇒ 自证该快照确实是**改前**的（这才是回滚点的判据） |
| apply **后** | 精确报出 2 列变化（`knowledge_base_id` + `updated_at`），其余 12 列全一致 |

### 14.4 ⚠ 渲染侧新发现：聚合图密度 9.4% → **90.5%**

用**真实融合产物**（`kg-fused-payload-dump-0918.mjs` 导出 46,896 节点 / 172,926 边 +
缓存里的真实社区 24,288 条）灌进**真函数**（`mergeCommunitiesTopologically` +
`buildAggregateGraph`，非重抄）跑三种归桶策略（`__tests__/kgFusedScale.test.ts`）：

| 策略 | 桶数 | 聚合边 | **密度** |
|---|---|---|---|
| **A 改绑前**（仅笔记，= 当时融合图） | 200 | 1,873 | **9.4%** |
| **B 改绑后（现状）** | 200 | **18,011** | **90.5%** |
| **C 实体继承其 mapping 笔记的桶** | 200 | 1,947 | **9.8%** |

C(200,2) = 19,900 ⇒ 现状 18,011 条聚合边 = **91% 的桶对之间都有边**。

**根因**：Louvain 只跑**笔记图**（`wiki_graph_communities_cached` → `note::get_vault_graph`），
**不含实体**；而融合发生在社区计算**之后**。改绑前融合图 ≡ 笔记图 ⇒ 社区 100% 覆盖，
「近半数节点无社区」是改绑才第一次出现的形态。无社区节点被
`GraphView.tsx:1234-1241` 按 `hash(id) % len` **并入已有桶** ⇒ 22,608 个实体被**随机**撒进 200 桶，
其 97,135 条实体侧边（含 22,608 条 mapping）随之变成**近乎随机的长程桶对**。

**为什么这是缺陷而不是观感问题**：`GraphView.tsx:1203-1206` 已把这条判据写进源码注释 ——
「聚合图边密度逼近完全图（旧哈希分桶实测 82.69%）⇒ 聚合节点受力趋同 ⇒ 力导向退化为均匀铺开，
宏观上就是一片互不相连、均匀分布的点」。现状 **90.5% 比那次的 82.69% 更差**。

**建议（策略 C，未实施，待裁决）**：在补全那一步，无社区节点**优先并入它 `mapping` 到的笔记所在的桶**，
只有无 mapping 的才 hash 兜底。收益已量化：聚合边 18,011 → 1,947（密度回到 9.8%），
且实体天然落在自己的笔记群内（mapping 边的本意就是「消除孤岛」）。
规模不变量不受影响：三种策略下 `真实节点混入聚合层` 均为 **0**、桶数均为 200 ≤ `MAX_AGG_PHYS_NODES(800)`
⇒ **不会**退化成静态显示/空白；问题只在布局可读性。

### 14.5 本轮我自己的错误（第五批，留痕）

1. **§13.5 备好的 SQL 单位错**（毫秒 vs 表里实际是秒）—— 幸在执行前的「A3 位数断言」这一步核出来。
   教训复用：**改一列之前先看这一列的写者用什么单位**，别照抄自己上次写的 SQL。
2. **apply 脚本第一版有一条断言是假红**：`cacheRow` 的 SELECT 漏了 `computed_at`，比较的是
   `undefined` ⇒ 报「缓存被改动」。**断言读不到基准值 ⇒ 必假红（或必假绿）**，两种都危险。
3. 脚本初稿里留了一行**恒真断言**（`readWiki().then === undefined || true`）和一行无用占位变量 ——
   自查时删掉。恒真断言比没有断言更坏：它占着「已检查」的位置。
4. **mapping 边计数两套口径**：本轮的 dump 按**每个实体节点**算 = 22,608
   （对齐 `wiki.rs:1169` 的 `for entity_node in &entity_nodes`），而上一轮的预览脚本按
   **去重后的名字集合**算 = 22,606（差 2 = 2 个同名但 `entity_type` 不同的实体节点）。
   已在 dump 脚本里把两套口径都打出来并注明对齐的是哪一套。
5. **`graphAggregate.test.ts:26` 手抄常量已腐烂**：注释写「与生产 `AGG_PHYSICS_CONFIG` 同值」，
   而生产 `repulsion` 已在 2026-09-17 标定为 `5400`，测试仍写 `600` ⇒ 该文件的形状判据
   一直跑在**生产不存在的参数**上。顺手修为直接引用生产常量（断言本就以 `TEST_PHYSICS`/`rStar`
   相对推导，换值不改变判据语义），并把写死数值的注释改成由函数现算。

### 14.6 复现资产（全在 `output/verify-graph-2026-09-15/`）

| 文件 | 作用 | 是否只读 |
|---|---|---|
| `kg-rebind-apply-0918.mjs` / `.txt` | 改绑执行器 + 断言报告 | 默认只读（dry-run 走 ROLLBACK） |
| `kg-rebind-rollback-0918.mjs` / `.txt` | 改绑回滚器 + 差异报告 | 默认只读 |
| `kg-fused-payload-dump-0918.mjs` / `.txt` / `.json` | 融合产物导出（22 MB，供渲染判据） | 只读 |
| `kg-rebind-preview-0918.mjs` / `.txt` | 改绑前后规模量化 | 只读 |
| `kg-render-scale-probe-0918.txt` | 三种归桶策略的密度对照（vitest 输出） | 只读 |
| `src/components/wiki/__tests__/kgFusedScale.test.ts` | 规模不变量 + 密度对照**回归测试**（真实 fixture 在则用真实数据，不在则等规模合成，**两条路都断言、不跳过**） | 测试 |
| `output/backup-2026-09-18-rebind/wikis-snapshot-0918.json` | 改前快照（含改前 `knowledge_base_id`），回滚器读它 | — |

---

## 15. 修复 §14.4 的渲染侧缺陷：锚点优先归桶（2026-09-18 凌晨，**已落码**）

用户裁决「要」⇒ 按 §14.4 的策略 C 改。

### 15.1 改动（3 文件 + 1 取证脚本）

| 文件 | 改动 |
|---|---|
| `src/components/wiki/communityMerge.ts` | 新增 `assignMissingCommunities`（+ `CommunityAnchor` / `AssignMissingOptions` / `AssignMissingOutcome`）。规则：只在**真的有缺桶节点**时补；锚点对端有桶 ⇒ 继承；只跟**一跳**（不做传递闭包）；多锚点取**桶 id 最小**（与 `anchors` 顺序无关 ⇒ 确定性）；对端也没桶 ⇒ hash 兜底并**计数**（`viaHash`）；只并入已有桶、绝不生成新 id；不修改入参；幂等。⚠ 本文件此前**没有任何 import** —— 补了 `hashStringToInt`（见 §15.5-1） |
| `src/components/wiki/GraphView.tsx` | 补全块 `:1219-1297` 改为：从 `data.edges` 取 `type === "mapping"` 的边作锚点 → 调真函数（不再内联 hash 循环）；新增 `warnOnHashFallback`（模块级按**值变化**节流，`viaHash > 0` 才报） |
| `src/components/wiki/__tests__/kgFusedScale.test.ts` | 复刻点 ③ 改为**调用真函数**（少一处复刻）；新增纯函数单测 7 例；把「生产路径 == 策略 C」钉成断言；`type` 契约断言 |
| `output/verify-graph-2026-09-15/kg-fused-payload-dump-0918.mjs` | 修 fixture 的 `type` 失真（见 §15.5-2），并把它变成一条**会失败**的自证（9 项） |

### 15.2 判据结果（真实载荷 46,896 节点 / 172,926 边，跑真函数）

| 策略 | 桶数 | 聚合边 | **密度** | 桶内被丢弃边 |
|---|---|---|---|---|
| A 改绑前（仅笔记） | 200 | 1,873 | 9.4% | 51,125 |
| B 旧规则（hash 兜底） | 200 | 18,011 | **90.5%** | 51,638（link 51,125 / reference 396 / mapping 117）|
| **C 生产规则（锚点优先）** | 200 | **1,947** | **9.8%** | 123,792（link 51,125 / reference 50,059 / mapping 22,608）|

- 生产路径与策略 C **逐位相同**（`aggEdges` / `buckets` / `density` 三条断言）⇒ 谁把补全改回 hash，测试会红。
- `viaAnchor = 22,608 / viaHash = 0`：锚点覆盖 **100%**，hash 兜底一次都没走；这条是**可失败**的（锚点链断则变正数）。
- 规模不变量不变：三策略真实节点混入均 **0**、桶数 200 ≤ `MAX_AGG_PHYS_NODES(800)`、聚合边 > 0 ⇒ **不会退化成空白/静态显示**。

### 15.3 保真度代价（必须说清，不是免费的）

C 把 **71.6%** 的边留在桶内（聚合层不画），比 B 的 29.9% 高得多。逐类看：

- `link` 51,125 条桶内 —— 与 **A 逐位相同**（笔记的社区没动）⇒ 这是社区聚合的**正常**形态；
- `mapping` 22,608 条**必然**全在桶内（实体就住在它笔记的桶里）⇒ 聚合层看不到它，但它在 `data.edges` 里仍在、**全细节层级照画**，它的作用（把实体挂到笔记上、消除孤岛）已由「同桶」实现；
- `reference` 50,059 条桶内（占 75,527 的 66%）⇒ **实体侧拓扑在桶级不可见**。

这不是丢数据：聚合层只产出**布局与聚合 LOD**，成员节点的真实边由渲染路径另算（`layoutUnits` 两种模式：隐式聚合时成员仍是真实节点）。
但它是一条**语义选择**：桶级视图现在等于「笔记社区视图 + 实体挂靠」，实体侧结构（公司/董事/行业）不再影响布局。
要让实体侧在桶级也可见，正确做法是**给实体单独算社区**（后端对实体图跑 Louvain，或按 `entity_type`/行业分桶），而不是退回随机撒。

### 15.4 门禁

`vitest run src/components/wiki` **92 passed (8 files)**（原 83 + 新增 9）· `tsc --noEmit` **0 字节** · `dprint check` **0 字节**（fmt 后复跑 vitest + tsc 仍全绿）。本轮**无 Rust 改动**。

### 15.5 第六批我自己的错误（留痕）

1. **★ `communityMerge.ts` 里 `hashStringToInt` 从未 import** —— 我只在**文档注释**里见过这个名字（同族：`graphAggregate.ts` 有该 import，我记混了）。
   后果之所以没在第一时间暴露：真实数据下锚点覆盖 100%、**兜底分支一次不执行** ⇒ 主路径与「锚点优先」用例全绿，只有我专为兜底写的 5 个用例才炸出 `ReferenceError`。
   三条教训：① **vitest 不做类型检查**，漏 import 只在运行时暴露 ⇒ 改完必须跑 `tsc`；② **「分支不被执行」会让缺失依赖隐形** ⇒ 纯函数也要有强制走兜底分支的用例；③ 判据不达标时先怀疑测量工具这条**反向也成立**：这里测试全绿恰恰因为被测分支没跑。
2. **fixture 的 `type` 失真**：`kg-fused-payload-dump-0918.mjs` 初版把 DB 的 `relation_type` 直接写进了 `type`，而 DTO 契约是 `type = "reference"`（**渲染类别**）+ `relationType = <关系名>`（`graph_dtos.rs:34-63`，「决定画成什么样，不代表这条边是什么关系」）。
   失真不影响聚合/社区计算（两者只看端点），但会让 fixture 在「前端按 type 取样式/着色/粒子」这条路径上与真实载荷分叉 —— 而 fixture 存在的全部意义就是不分叉。已修正，并加了两条会失败的自证（类别集合恰为 `{link, reference, mapping}`；`relationType` 只出现在 reference 边上 ⇒ 实测 75,527 条 / 56 个关系名）。
3. 收尾时手滑删掉 `communityMerge.ts` 头部注释里的一个换行（把 ` * ────` 拼成了 ` * * ────`），当场看文件发现并修回。
4. `mergeCommunitiesTopologically` 多传了一个不存在的参数、策略 C 的输入构造方式（伪造 `__bucket__*` 键冒充桶集合）不干净 —— 自查时改掉（前者会让 tsc 报 excess property）。

**可复用判据**：既然「未执行的分支」会掩盖缺陷，那 fixture 与用例的价值就不只在于「断言什么」，还在于**覆盖哪些分支** —— 主路径全绿的测试集，可能一个分支都没进过。

### 15.6 回滚点实况（这次先说结论，不再事后自曝）

| 文件 | 改前副本 | 回滚操作 |
|---|---|---|
| `GraphView.tsx` | **有**：`output/backup-2026-09-17-graph10/src-components-wiki/GraphView.tsx`（已核：三个新标记全无；行集合差 = 仅旧有 8 行 / 仅新有 49 行） | 直接 `cp` 覆盖 |
| `communityMerge.ts` | **无**（本轮新增依赖，此前从未改动过） | 删两段：`:35` 的 import（含其上 3 行注释）、`:295-EOF` 的「缺失节点的桶补全」整节（`:434` 行止） |
| `kgFusedScale.test.ts` | **无**（同日新建，`Write` 全量重写） | 无改前副本；该文件是回归测试，删掉不影响生产路径 |
| fixture `.json` | **无** | 重新跑 dump 脚本即可再生成（只读 DB） |

`GraphView.tsx` 的「行集合差」是本次可用的**回滚判据**：仅旧有 8 行恰为旧 import + 旧 hash 补全块 ⇒ 差异 = 且仅为本次改动。

---

## 16. 连带修复：两次「文件顶部插入代码」把下游 21 处行号引用搞漂（2026-09-18）

### 16.1 怎么发现的

本轮改完前端后顺手跑仓库门禁（此前几轮只跑了 `vitest/tsc/dprint`），`check:facts` **硬失败**：

```
✖ 新增客观错 1 处（修法唯一，**硬拦**）：
   ✖ src-tauri/crates/harness/src/knowledge_graph.rs 引用指向空行：dao/src/repo/knowledge_graph.rs:811
~ 疑似腐烂 1 处（软判据）：page_type.rs → src/commands/wiki.rs:1415 指向收尾括号行
```

### 16.2 归属判定（不靠猜，靠算术 + mtime）

| 引用目标 | 引用写的 | 真实当前行 | 判定 |
|---|---|---|---|
| `wiki.rs` 的 `synced` 写入点 | 1415 | **1469** | **我的**（1415+54）—— 我 2026-09-17 在 `wiki.rs:44` 插了 `should_warn_dangling`，整文件下方 +54 |
| `services.rs` 的 `knowledge_document` / `set_llm_judge` / 内层 `content` | 357 / 922 / 1913-1916 | **410 / 975 / 1972-1976** | **我的**（+53；我 2026-09-17 在 `services.rs:179` 插了「步骤 0 域一致性体检」） |
| `wiki.rs` 的 `mapping` 合成边 / `daily` / `knowledge` | 1151 / 1669 / 1915 | **1173 / 1723 / 1969** | **非我**（改动前即偏 32；插入点在 :44，算术对不上就是别人的漂移） |
| `dao/repo/knowledge_graph.rs` 的 `causes` 排除 / `reference` 边 | 811 / 1349 | **901 / 1794** | **非我**（我的新增全在 :1261 起，811 上方只有 +1 行 import ⇒ 89 行的偏差不是我造成的） |

**可复用的归属判据**：`引用偏移量 == 我的插入行数` ⇒ 是我搞漂的；对不上就要去找别的插入点（或本来就是既存腐烂）。
另一条旁证是 **mtime**：`harness/src/*.rs` 是 10:43/10:47（非我），`commands/wiki.rs`/`init/services.rs` 是 15:40/15:58（我）。

### 16.3 处置

写一次性脚本 `output/verify-graph-2026-09-15/fix-stale-line-refs-0918.mjs` 校正 **6 个文件 / 11 组 / 21 处**引用：
每条替换**断言命中数**（不符即抛错且不写盘 —— 避免静默的部分替换），口径按 `check:facts` 的原话「真源是代码，不是引用」。

| 文件 | 处数 | 变化 |
|---|---|---|
| `crates/harness/src/knowledge_graph.rs` | 5 | `dao:811→901`、`dao:1349→1794`×2、`wiki.rs:1151→1173`×2 |
| `crates/harness/src/page_type.rs` | 10 | `wiki.rs:1669→1723`×3 / `1915→1969`×3 / `1415→1469`×3、`services.rs:357→410`×3 |
| `crates/dao/examples/wiki_edge_relation_probe.rs` | 1 | `dao:1349→1794` |
| `crates/tools/src/tools/narrative_structure.rs` | 1 | `services.rs:1913-1916→1972-1976` |
| `src/commands/opc_workflows/seed_content_media.rs` | 1 | 同上 |
| `src/init/llm_providers.rs` | 1 | `services.rs:922→975` |

**自证「零语义改动」**：与备份逐行比对 ⇒ 19 处改动行**全部**是注释或字符串字面量，且**行数不变**（每个文件行数一致）⇒ 无需 `cargo check`。

**回滚点（这次是改动前建的）**：`output/backup-2026-09-18-linerefs/`（6 个文件的改前副本 + `CHECKSUMS-linerefs-0918.txt`），并已自证「备份不含任何新行号」。

### 16.4 门禁前后

| 检查 | 前 | 后 |
|---|---|---|
| `check:facts` | ✖ 硬失败 1（引用指向空行） | **✔ 无新增客观错；软判据 0 命中**（`BROKEN/越界/空行/控制流 0`） |
| `check:layers` | 0 | 0 |
| `check:types` | 0 | 0 |
| `check:contracts` | ✖ `[D] runtime-core 越界依赖 axagent-entities` | ✖ **同一条，未处置** |

`check:contracts` 那条是 **Rust crate 依赖越界**（`runtime-core` 只允许依赖 `axagent-harness`，却依赖了 `axagent-entities`），与本轮前端改动无关，属需架构决策的既存问题 ⇒ **只登记不擅改**。
它另有一条**棘轮提醒**值得单列：`[H] 静默丢弃 Result 已降至 273（基线 278），请把 SILENT_RESULT_BASELINE 下调到 273` —— 基线虚高会让棘轮变松、后续回归检不出（同族：`gate-baseline-ratchet`）。

### 16.5 教训（写成纪律，因为它一定会再犯）

**往文件任意位置插入代码 ⇒ 必须扫「谁引用这个文件:行」，并跑 `check:facts`。** 本轮两次插入都在**文件顶部**，按定义会把该文件**下方所有**引用整体推移；而这类腐坏**不会**被 `tsc`/`vitest`/`dprint`/`cargo check` 中的任何一个发现 —— 它们只看代码形状，不看注释里的字符串。

两个具体坑：
1. `check:facts` 只在 `.rs` 上硬拦，且**自己就声明 `.md` 一条都没查**（`--dump-doc-refs` 只取证、恒退出 0）⇒ 我在审计文档/记忆里写的 `xxx.rs:NNN` 属于**未覆盖面**，同样会腐烂（本文档内的行号引用请一律按符号名复核）。
2. 引用**指向非空行也不代表是对的**：硬判据只拒绝空行/越界/裸控制流，所以一条偏了 244 行的引用可以长期「LOCATED」而无人发现 —— 本轮 `dao:1349 → 1794`（偏 445 行）、`wiki.rs:1151 → 1173`（偏 22 行）就是这种。**软判据只抓「附近已无同句引号值」的那部分。**

---

## 17. 回滚点实况复核（收尾时发现两处与记忆不符）

§15 落码后我按 §16 的教训逐条复核回滚点，用递归扫描 + sha256 + 内容标记独立取证，**推翻了自己两处口头结论**。

### 17.1 实况表

本轮共改 3 个前端文件：

| 文件 | 改前副本 | 位置 / 自证 |
|---|---|---|
| `src/components/wiki/GraphView.tsx` | ✅ **有**（先前我不知道） | `output/backup-2026-09-18-anchor-before/GraphView.tsx.before-anchor-0918`（6031 行 / `c2413da15de9`） |
| `src/components/wiki/communityMerge.ts` | ❌ **无**（全 `output/` 递归扫描 0 份） | 只能按反向清单还原 |
| `src/components/wiki/__tests__/kgFusedScale.test.ts` | ❌ 无 | 同上（本轮为整文件重写，只能重写回去） |

后端 6 个 `.rs` 的改前副本不受影响：`output/backup-2026-09-18-linerefs/`（§16.3，改动前建的，`CHECKSUMS-linerefs-0918.txt` 自证）。

### 17.2 关键更正：`backup-2026-09-17-graph10` 的合法性是**相对**的

该目录自带 `README-NOT-A-ROLLBACK-POINT.md`，自曝它对**上一批改动（B+D）**不是回滚点（副本里已含 `DanglingEdgeSummary` / `colorTextTertiary` / `should_warn_dangling` 等那批新增物）。我据此在收尾前一直记着「本轮没有 `GraphView.tsx` 改前副本」—— **这个推论是错的**。

正确判据是**相对哪一批改动**，不是「这个目录是不是回滚点」：

| 判据（内容标记） | graph10 副本 | 含锚点归桶后应当为 |
|---|---|---|
| `assignMissingCommunities` | `false` | `true` |
| `warnOnHashFallback` | `false` | `true` |
| 被替换掉的旧实现 `% FORCE_CLUSTER_COUNT` 内联循环 | `true` | `false` |
| 行数 | 6031 | 6084 |

三条同向成立 ⇒ 它就是**本轮**的改前版本（那批 B+D 改动完成于 16:00 前，锚点归桶在 16:38）。已复制登记并写 `README-rollback-anchor-0918.md`。

### 17.3 新纪律

1. **「有备份」是逐文件的，不是整体件数** —— 本轮改 3 个文件只备了 1 个（后端那批另算），漏了 2/3；
2. **一个备份目录的「是不是回滚点」必须带限定语（相对哪批改动）**，否则会出现「因为对 A 批不合法，就误判对 B 批也不合法」的**假缺口**（本轮实际发生）；
3. **判据用内容标记，不用时间**：`含本轮新增标识符 ⇒ 不是改前版本`（沿用 `backup-2026-09-17-graph10/README-NOT-A-ROLLBACK-POINT.md` §5）。

---

## 18. 「端点缺失的边（未绘制）276 条」归因（用户界面实测，2026-09-18）

D-2 加的悬空边面板上线后，界面报：**「172926 条边中有 276 条的端点不在当前节点集里」**，样例形如
`entity:603377 -> entity:f9b2b050-e336-4de8-b305-0b40cbf5740f_50371a2c5078b757a8f8c75b8877e815`。
本节把 276 归因到底，结论：**是跨库 id 引用留下的重复行，真实关系一条没丢。**

### 18.1 复现与第一性归因（先数 distinct，别看 276 条）

探针 `output/verify-graph-2026-09-15/dangling-edge-probe-0918.mjs`（本地复算，不连库）
+ `dangling-edge-probe2-0918.mjs`（形态与对应关系）：

| 判据 | 实测 | 说明 |
|---|---|---|
| dangling / 总边 | **276 / 172,926** | 与界面**逐位相同** ⇒ 复现成功 |
| `bothMissing` | **0** | 全是单端缺失（告警文案的另一条分支本次不成立）|
| 缺哪一端 | 缺 source **0** / 缺 target **276** | — |
| **distinct 缺失端点** | **1**（！） | 276 条边指向**同一个**实体 |
| 边类型 | `reference` 276（`link` / `mapping` 各 0）| — |
| relationType | `has_concept` 224 + `in_industry` 52 | — |
| 276 个存活 source | 全是 `entity:<股票代码>` 公司节点 | 如 603377 东方时尚 |

> **★ 方法：第一件事是数 `distinct`，不是逐条看 276 行样例。** 276 条边 ↔ **1 个实体**，
> 问题立刻从「276 条边有问题」塌缩成「1 个实体的 id 拼法不对」。

### 18.2 那一个实体是什么

`f9b2b050-…_50371a2c5078b757a8f8c75b8877e815` 与节点集里的 `entity:50371a2c5078b757a8f8c75b8877e815`
是**同一实体**：`md5("次新股") = 50371a2c5078b757a8f8c75b8877e815`（实测逐字符吻合）。

DB 里「次新股」有**三条**同名实体记录，用**三套 id 规范**：

| id | 域 | entity_type | lifecycle | 进图？ |
|---|---|---|---|---|
| `50371a2c…`（裸 md5）| `lemonhu_knowledge_graph` | concept | NULL | ✅ 节点集里的那个 |
| `f9b2b050-…_50371a2c…`（**uuid 前缀**）| `f9b2b050-…`（A股知识图谱）| industry | NULL | ❌ 域不是 lemonhu |
| `lemonhu_knowledge_graph_50371a2c…`（**库名前缀**）| `lemonhu_knowledge_graph` | industry | **非 NULL** | ❌ 被 lifecycle 过滤 |

**两套 id 规范的来源**（`knowledge_entities` 全表统计）：

| kb | lifecycle IS NULL | 实体数 | 含 `_` | uuid 前缀 |
|---|---|---|---|---|
| `f9b2b050-…` | false | 24,270 | 24,270 | 24,270 |
| `f9b2b050-…` | **true** | **4** | 1 | 1 ← 就是它 |
| `lemonhu_knowledge_graph` | false | 25,934 | 24,271 | 0 |
| `lemonhu_knowledge_graph` | **true** | **22,608** | **0** | 0 ← **恰好 = 图上实体节点数** |

⇒ A 库用 `<kb_uuid>_<md5>`，lemonhu 用 `<库名>_<md5>`，**而图上用的实体 id 全是裸 md5**。

### 18.3 根因（两条独立判据）

**判据一 · 跨域引用**：lemonhu 域里 `target_entity_id` 指向**别的域的实体**的关系数 = **276**（SQL `EXISTS` 子查询），
与 dangling **逐位相同** ⇒ 悬空边 100% 由「边的域 ≠ 端点的域」构成。

**判据二 · 孪生行**：把 `target LIKE '%50371a2c…'` 全部取出 ⇒ `rows=557`、`distinct(source|relationType)=281`，
拆开看 `has_concept` 224 + 224、`in_industry` 52 + 52（**各有一份裸 md5 版本**），
且**裸 md5 版本正常绘制**（在节点集里）⇒ **真实关系 0 丢失，276 条全是冗余行**。

**它怎么产生的**：那 276 条的 `created_at` **只有一个值** `1786277831750` = **2026-08-09T12:17:11**（`distinct_ts=1`）
⇒ 一次批量写入；而对应的裸 md5 版本是 **2026-07-24**。即 **8/9 那次写入往 lemonhu 域塞了 target 写成 A 库 id 形态的边，且没删旧行**。

**是否复发**：全库跨域 target 共 276 条、全部同一时间戳 ⇒ **一次性历史事件，此后未新增**。

### 18.4 顺带发现（本次不处理，登记）

| # | 发现 | 影响 |
|---|---|---|
| 1 | node 侧过滤 `lifecycle IS NULL`（`dao/src/repo/knowledge_graph.rs:263`）而 relation 侧**无对应约束** | 只要关系表里有一个「不在节点集」的 target 就 dangling ⇒ 结构性缺口 |
| 2 | 两个库的 `created_at` **单位不一致**：`f9b2b050` 存**秒**（`1785473244`）、lemonhu 存**毫秒**（`1786277831750`）| 跨库时间比较/排序会错 |
| 3 | 部分记录 `updated_at` = `1970-01-21`（≈0/负值）| 脏值 |
| 4 | 「次新股」等**同名多 id** 实体（图上另有 2 组：金宇 / 太阳能）| 图上出现重复点 |

### 18.5 处置选项（等裁决；**A+B 已于 §19 按裁决全量执行**）

| 方案 | 动作 | 效果 | 风险 |
|---|---|---|---|
| **A 数据层** | 删 276 条 uuid 前缀重复行（有孪生行）| dangling → 0，边数 172,926 → 172,650 | 低（建议先 dry-run + 备份 + 比对孪生完整性）|
| **B 融合层**（建议叠加）| 融合图**剔除端点不在节点集的边**，但**保留 dangling 计数用于告警** | 「边数」不再虚高，异常仍可见 | 低 |
| C 实体层 | 去重三条「次新股」记录 | 消除隐患 | 中（需产品语义决策：concept 还是 industry）|
| D 写入端 | 定位 8/9 写入来源 | 防复发 | 判据二显示**已不复发**，优先级低 |

**A+B 的理由**：A 治当前脏数据；B 让「边数」这个数字无论未来还有什么脏数据都不再撒谎 ——
**B 不可用 A 替代**，因为 A 只是清了这一批。

## 19. A+B 落地（用户裁决「A+B都做」，2026-09-18）

### 19.1 A —— 数据层：删 276 条悬空重复边（**已落库**）

**范围先精确化再动手**（§18.5 表里写的是「删 276 条 uuid 前缀重复行」，但库里 uuid 前缀 target 一共 **281** 条）：

| `rel_kb` | `target_kb` | 条数 | 判定 |
|---|---|---|---|
| `lemonhu_knowledge_graph` | `f9b2b050-…`（**别域**）| **276** | 悬空 ⇒ **删** |
| `f9b2b050-…` | `f9b2b050-…`（**本域自洽**）| 5 | 合法 ⇒ **不动** |

⚠ 判据必须是「**target 属别域**」而不是「target 带 uuid 前缀」—— 那 5 条形态相同但在自己域内自洽。

⚠⚠ 更不能用 `id LIKE 'lemonhu_knowledge_graph_%'`：实测这种 id 形态有 **37,383** 条（是本库的**合法**形态），按形态删会误杀 37,107 条真实关系。

**安全性前提（逐行验证，不是总数）**：`with_twin = 276` —— 每一条都有孪生行（同 `source_entity_id` + 同 `relation_type`，target 指向**本域可见**实体）⇒ 删除不丢任何真实关系。
**为什么必须逐行**：`COUNT(*) == 276` 的写法在「275 行安全、1 行无孪生」时同样通过 —— 统计量相等是最容易骗人的判据。

**删前外键/触发器体检**：`knowledge_relations` **不被任何表以外键引用**、**无触发器** ⇒ 删行无级联、无副作用。

**备份（`output/backup-2026-09-18-dangling-276/`，全列 13 列）**：

| 文件 | 大小 | 用途 |
|---|---|---|
| `dangling-276-rows.json` | 145,771 B | 备份本体（全列）|
| `dangling-276-twins.json` | 58,437 B | 每行对应的孪生行 id（安全性证据）|
| `dangling-276-ids.txt` | 20,858 B | 精确 id 清单（幂等重放）|
| `rollback-insert-276.sql` | 133,902 B | 回滚（逐行 INSERT，可直接 `psql -f`）|
| `CHECKSUMS-dangling-276.txt` | 831 B | sha256 清单 |
| `backup-and-plan-0918.mjs` | 7,819 B | 备份 + 8 项自证脚本 |
| `delete-dangling-276-0918.mjs` | — | 删除器（支持 `--dry-run`）|

备份脚本 **8 项自证全绿**（行数=276 / 同域 / target 形态 / **逐行有孪生** / 时间戳唯一 / 映射行数一致 / id 无重复 / 13 列齐全）。

**执行（单事务 + 6 批 × 50 + 逐批断言 + 提交后独立连接复查）**：

| 阶段 | 判据 | 结果 |
|---|---|---|
| 删前 | 清单全部命中 | 276/276 ✔ |
| 删前 | **指纹与备份逐位相同** | 库 `6233a832…` == 备份 `6233a832…` ✔ |
| 删前 | 全表基线 | 75,533 ✔ |
| 删除 | 6 批（50×5 + 26）| 逐批 rowCount 相等 ✔ |
| 删后 | 全表 = 75,533 − 276 | **75,257** ✔ |
| 删后 | **悬空边归零** | **0** ✔ |
| 删后 | **孪生行 276 条全部健在** | 276 ✔ |
| 提交后 | 独立连接复查（同上 4 项 + 其它域未动）| 全部 ✔ |

**端到端复算**（同一 dump 脚本，删前/删后两份载荷并存）：

| 指标 | 删前 | 删后 | 差 |
|---|---|---|---|
| 融合边 | 172,926 | **172,650** | **−276** |
| `reference` 边 | 75,527 | **75,251** | **−276** |
| `link` / `mapping` / 笔记节点 / 实体节点 | 74,791 / 22,608 / 24,288 / 22,608 | 同左 | **0** |
| dangling（探针）| **276**（缺 target 276）| **0** | −276 |
| distinct 缺失端点 | **1**（次新股）| **0** | −1 |

⇒ **只有 reference 边少了 276，其它一切逐位相同** —— 这正是「删的是重复行」的形态证据。

dump 脚本里写死的期望基数**已同步更新**（`75,527 → 75,251`、`172,926 → 172,650`），并留注释说明「这两个数变了本身就是判据；若哪天涨回去，别急着改这里的数，先查为什么」。

### 19.2 B —— 融合层：剔除画不出来的边，但**保留**淘汰规模

**为什么不能只做 A**：A 只清了这一批。B 让「边数」这个数**无论未来还有什么脏数据都不再撒谎**。

改动（4 文件）：

| 文件 | 改动 |
|---|---|
| `crates/harness/src/graph_dtos.rs` | 新增 `GraphData::retain_resolved_edges()`；`DanglingEdgeSummary::total_edges` 口径文档重写；新增 **4 条单测** |
| `src/commands/wiki.rs` | 融合出口由「只报」改成「先统计 → 再剔除 → 后告警」（新步骤 7 + 原步骤改 8）|
| `src/components/wiki/GraphView.tsx` | `DanglingEdgeSummary` 文档与 `totalEdges` 字段注释（3 处）|
| `src/pages/WikiGraphPage.tsx` | 悬空提示节的口径注释 |
| `src/i18n/locales/*.json` ×11 | `wiki.graph.danglingEdges.desc`「仍计入边数」→「已不计入边数」|

**口径变化（这是全节最关键的一处）**：

```
未剔除：total_edges ==  edges.len()          （GraphData::new 之后）
已剔除：total_edges >   edges.len()          （差额 == dangling）
两者都满足：edges.len() + dangling == total_edges
```

⇒ `total_edges` 从「参与统计的边总数（= edges.length）」变成「**淘汰前**的统计基准」。
读它的人**不得**再假设它等于 `edges.len()` —— 那正是本节要修的病。

**幂等是硬要求**：`retain_resolved_edges()` 二次调用时已无悬空边 ⇒ 统计为 0 ⇒ **提前返回，不覆盖**上一次的记录。
去掉这个短路就会出现：任何一次多余的 `refresh_unresolved` 把那 276 抹成 0 ⇒ **告警消失而脏数据仍在**。单测 `test_retain_resolved_edges_is_idempotent` 就是钉这一条的回归锁。

**顺序**：必须先 `refresh_unresolved()`（算统计）再 `retain_resolved_edges()`（剔除）。反过来 = 先剔边后刷新 ⇒ 刷新按剔后边集重算 ⇒ dangling 归 0，一次静默的「修好了」。

**为什么不改 `GraphData::new()`**（即不把「剔除」做成构造期不变量）：会让所有构造路径都剔边，而审计/诊断方可能依赖完整边集；且改动面从 1 个调用点扩到全仓。本方法只在**融合出口**（唯一会新增悬空边的步骤）调用一次。

### 19.3 B 的判据（单测 + 门禁）

新增 4 条单测（`graph_dtos::tests`，全部通过）：

| 测试 | 钉住什么 |
|---|---|
| `test_retain_resolved_edges_drops_them_but_keeps_the_count` | 边数下降 + **统计保留** + 恒等式；缺任一条都算失败（只删边=静默；只报不改=边数撒谎）|
| `test_retain_resolved_edges_is_idempotent` | ★ 二次调用**不得**抹掉记录（去掉短路即变红）|
| `test_retain_resolved_edges_noop_on_clean_graph` | 干净图一条不丢；未剔除形态下恒等式也成立 |
| `test_retain_resolved_edges_on_empty_graph` | 空边集短路不写出「干净」 |

**真实界面判据仍待验**：B 的效果要在**重新编译后的 dev** 上打开图谱才能看到（工具栏边数应从 172,926 变成 172,650、悬空提示条消失）。本节交付的是代码级判据。

### 19.4 连带修复：本轮插入把下游行号引用搞漂 —— 而 `check:facts` **只报了 2 处，实际漂了 9 处**

**发现方式**：改完立刻跑 `check:facts` ⇒ **硬失败**（退出码 1）。

```
✖ src-tauri/crates/harness/src/page_type.rs 引用指向空行：src/commands/wiki.rs:1723
✖ src-tauri/crates/harness/src/page_type.rs 引用指向裸控制流语句：src/commands/wiki.rs:1969（实为 continue;）
```

**归属判定（算术）**：我在 `wiki.rs` 的融合出口（原 `:1196` 附近）把「只报」改成「统计→剔除→告警」，插入 **+23 行**。
`1723 → +23 = 1746`、`1969 → +23 = 1992` ⇒ **是我的**。

**按符号名找真源（不靠算术猜）**，实测三处写入点：

| 引用语义 | 旧引用 | 现真源（实测行内容）| 偏移 |
|---|---|---|---|
| `Daily` | `wiki.rs:1723` | **`1746`** = `page_type: Some("daily".to_string())` | +23 |
| `Knowledge` | `wiki.rs:1969` | **`1992`** = `page_type: Some("knowledge".to_string())` | +23 |
| `Synced` | `wiki.rs:1469` | **`1492`** = `page_type: Some("synced".to_string())` | +23 |
| `mapping` 边合成 | `wiki.rs:1173` | `1173` = `mapping_edges.push(GraphEdge::structural(` | **0**（在插入点之前）|

★ **`Synced` 那一组（3 处）`check:facts` 没报** —— 因为它指向的是非空行、非控制流，落进 `LOCATED`。
这正是 §16.5 记录过、本轮再次实证的盲区：**「指向非空行」≠「引用正确」**。
所以处置不是「修 facts 报的 2 处就完事」，而是**主动扫全部引用该文件的行号**（`grep 'wiki\.rs:\d+'` 全仓）⇒ 共 **9 处**（`page_type.rs` 的 `:42/:43/:45/:69/:71/:75/:420/:421`），已全部校正。

**`.md` 侧不动**：本轮审计文档前文（§3/§18/§19 等）与 `docs/plans/**`、`output/**` 里的 `wiki.rs:NNNN` 是**当时那一刻的实测值**（很多本就陈旧），按「不改历史记录」处置；引用时按**符号名**定位。本节以下所有行号均为 2026-09-18 实测值。

### 19.5 门禁（本轮全部复跑）

| 门禁 | 结果 |
|---|---|
| `tsc --noEmit` | **0 字节** ✔ |
| `dprint check`（全仓，含 11 个 locale JSON）| **0 字节** ✔ |
| `check:facts` | **✔ 无新增客观错 / 软判据 0 命中**（LOCATED 398 / BROKEN·越界·空行·控制流 **0**）|
| `check:layers` | ✅ 棘轮无超基线 |
| `check:types` | EXIT=0 ✔ |
| `check:i18n --strict` | ✅ 11 个 locale 语法正常、code key 全在 zh-CN |
| `check:i18n:untranslated` | ✅ 0 新增未翻译 |
| `vitest src/components/wiki` | **92 passed / 8 files** ✔ |
| 全量 `vitest` | **1897 passed / 1 failed**（`DemandDiscoveryPage`，日志有 PG `deadlock detected`；**单独重跑 passed ⇒ flaky**）|
| `cargo fmt --all -- --check` | **0 字节** ✔（★ 首跑报我 2 处 —— `tsc`/`dprint`/`vitest`/`cargo check` 全都发现不了；修时只 `cargo fmt -p axagent-harness`，**不用 `--all`**）|
| `check:contracts` | 与上轮**完全相同**两条（`[D]` `runtime-core` 越界依赖、`[H]` 静默丢弃 Result 273 < 基线 278）⇒ **无新增** |
| `cargo test -p axagent-harness --lib graph_dtos` | **15 passed / 1 failed**（失败项见 19.6，**既有红**）|

### 19.6 既有红（**非本轮引入**，登记不代改）

| 项 | 现象 | 归属判据 |
|---|---|---|
| `graph_dtos::tests::test_dangling_edges_always_serialized_and_legacy_parseable` | 断言「不含 `unresolvedTypes`」，实测含 `[{rawType:"wiki_page",…}]` | 该测试用 `node("a","wiki_page")`，而 **`"wiki_page"` 从来不在 `PageType` 词汇表**（`page_type.rs:122-140` 的合法值是 `note`/`entity`/`doc`/…）⇒ 该断言**必然失败**。本轮**未改** `PageType` 词汇表 / `parse_strict` / `node()` helper / `collect_unresolved_type_stats` ⇒ 行为输入逐位不变 ⇒ **改动前也红**。倾向成因：D3（2026-09-14）补词汇表时，这条 fixture 没跟着改 |
| `ci:check --quick` → 「ID 数据边界（undefined/null 字符串腐化）」 | 命中 `src/stores/feature/ttsChannelStore.ts:128`、`src/stores/shared/animationStore.ts:40,77` | 命中点全在 `src/stores/**`，与本轮（wiki 图谱）零交集 |
| `check:contracts` → `[D] runtime-core 越界依赖 axagent-entities` | 上轮已登记 | 需架构裁决 |

**建议**：第一条只需把 fixture 的 `"wiki_page"` 改成 `"note"`（该测试测的是 **serde 契约**，不是 type 词汇表），但**改别人的测试需要确认它是不是故意留的绊线** ⇒ 只登记，等裁决。

### 19.7 第七批我自己的错误（留痕）

1. **★ 又一次「插入代码搞漂下游行号」** —— 与 §16 同一形态，且**上一轮刚把它写成纪律**。这次是插入在文件中段（`:1196`）而不是文件顶部，说明「只防文件顶部插入」是不够的：**只要插入点在引用目标之前，下游全漂**。
   已修 9 处；`check:facts` 只报 2 处 ⇒ **不能只修它报的**。
2. **期望基数写死在 dump 脚本里** ⇒ A 执行后脚本报红（reference 75,527 vs 实测 75,251）。这不是脚本的错（它**故意**把基线钉死当判据），但**必须同步更新**，否则它永远红、下次真回归就被淹没。
3. **`id LIKE 'lemonhu_knowledge_graph_%'` 差点被当删除谓词用** —— 实测有 37,383 条合法命中。若按形态删会误杀 37,107 条真实关系。最终用「target 跨域」谓词 + **精确 id 清单** + 逐批 `rowCount` 断言。
4. **删除前差点只对总数（`COUNT(*)==276`）不对逐行孪生** —— 那在「275 行安全、1 行无孪生」时同样通过。

### 19.8 回滚点（A 与 B 各自独立）

| 层 | 回滚方式 | 实测 |
|---|---|---|
| **A** | `psql -f output/backup-2026-09-18-dangling-276/rollback-insert-276.sql`（276 条逐行 INSERT）| 备份与库**指纹对拍相同**（`6233a832…`）；回滚脚本未实测执行过（**如实登记**）|
| **B** | 反向还原：删 `graph_dtos.rs` 的 `retain_resolved_edges` + 4 测试、`wiki.rs` 的步骤 7、前端注释与 i18n 文案改回 | 均为**纯增量**，可精确反向；`GraphView.tsx`/`communityMerge.ts` 的改前副本见 §17.1 |

**A 的回滚风险自陈**：回滚脚本会把这 276 条**重新插回**（含 `created_at`），但若届时那些孪生行已被别的流程改动，回滚得到的是「删除前那一刻的快照」而不是「当前逻辑的等价态」—— 这是快照式回滚的固有性质，不是缺陷。

---

## §20 追问归因：A 股知识库「被导入了两次（名称不同）」—— 实测是**三次导入 / 两个库记录 / 一次跨库合并**

**用户问题原文**：`目前这个A股知识库是不是被导入了两次（两次名称并不相同）`

**结论（先给答案）**

> **是。而且不止两次 —— 同一批内容进了 3 次，落在 2 条 `knowledge_bases` 记录里**（`f9b2b050…`「A股知识图谱」与 `lemonhu_knowledge_graph`「开源股票知识库(lemonhu)」，**两条都 `enabled=1`**，所以列表里就是两份）。
> 其中 2026-09-16 的一次**跨库合并把 `f9b2b050` 那一份整体标记为「已并入」lemonhu**；这次合并里**唯一 1 行选错了主实体**，正是 §18 那 276 条悬空边的**唯一源头**。

### 20.1 两条库记录（都启用）

| id | name | kind | enabled | embedding_provider | description |
|---|---|---|---|---|---|
| `f9b2b050-e336-4de8-b305-0b40cbf5740f` | **A股知识图谱** | indexed | **1** | `6f67c842-…::bge-m3.Q5_K_M.gguf` | `lemonhu A 股知识图谱（实体 + 关系）` |
| `lemonhu_knowledge_graph` | **开源股票知识库(lemonhu)** | indexed | **1** | NULL | `由开源项目 lemonhu 构建的 A 股知识图谱，含概念/行业/公司/高管关系` |

另有 `__sys_memory_reflow__` / `__sys_trajectory__` 两条 `enabled=0` 的系统库，与本问题无关。
⚠ 第一条的 `description` 里**自己写着「lemonhu」** —— 两库同源，从描述就能看出。

### 20.2 三次导入（按实体写入时间戳，各批 `distinct_ts = 1`）

| 波次 | 时间（+08） | 目标库 | 实体 id 规范 | 实体行 |
|---|---|---|---|---|
| ① 首次 | **2026-07-24 20:52:44** | `lemonhu` | 裸 md5 21,083 ＋ 裸数字（股票代码）3,188 | 24,271 |
| ② | **2026-07-28 19:27:54** | `f9b2b050` | `<kb_uuid>_<md5>` 21,083 ＋ `<kb_uuid>_<数字>` 3,188 | 24,271 |
| ③ 再导入 | **2026-08-09 20:17:11** | `lemonhu` | `<库名>_<md5>` 21,083 ＋ `<库名>_<数字>` 3,188 | 24,271 |

文档侧：

| 库 | 文档行 | 去重标题 | 时间戳 |
|---|---|---|---|
| `f9b2b050` | 24,271 | 22,606 | 2026-07-29 01:56:18（波次②实体导入后约 6.5 h）|
| `lemonhu` | 24,271 | 22,606 | 2026-08-09 20:17:11（与波次③实体**同一时刻**）|

**旁证**：wiki `dc8efbd8…`（name「A股知识」）的 `created_at = 1785238074`（秒）= **2026-07-28 19:27:54**
—— 与**波次②实体导入同一秒** ⇒ 这个 wiki 是跟着 `f9b2b050` 那次导入建的。
⚠ `knowledge_bases` **没有 `created_at`/`updated_at` 列**（information_schema 已核）⇒ 建库时刻不可查，只能用下属实体时间戳反推。

### 20.3 指纹集合：四套 id 规范的实体**完全重合**

判据 = 实体 id 的**末尾 32 位 hex**（正则 `([0-9a-f]{32})$`）。

| 集合 | 去重 tail |
|---|---|
| `f9b2b050`（`<uuid>_<md5>`）| 21,083 |
| `lemonhu`（`<库名>_<md5>`）| 21,083 |
| `lemonhu`（裸 md5）| 21,083 |
| **交集**（`f9b2b050` ∩ `lemonhu`） | **21,083** |
| 差集（只在一边） | **0 / 0** |

文档标题：两侧各 **22,606** 去重标题，**交集 22,606**（完全重合）。
⇒ 内容同一批，**没有哪一侧「多出」任何实体或文档**。

### 20.4 2026-09-16 的跨库合并：把 `f9b2b050` 整体并入 lemonhu

`lifecycle` 的实际内容是 `{"merged_into": <主实体 id>, "merged_at_ms": …, "group_mention_sum": n}`，
**所有已并入行的 `merged_at_ms` 都是 `1789502375024` = 2026-09-16 03:59:35（+08）** ⇒ 一次批量合并。

`f9b2b050`（24,274 行）：

| id 形态 | 行 | 已并入 | **存活** |
|---|---|---|---|
| `<uuid>_<md5>` | 21,083 | 21,082 | **1** |
| `<uuid>_<数字>` | 3,188 | 3,188 | 0 |
| 裸 uuid（3 条，名字含 U+FFFD 替换符乱码）| 3 | 0 | **3** |
| **存活合计** | | | **4** ← 与「`f9b2b050` 域 `lifecycle IS NULL` = 4」逐位相同 |

`lemonhu`（48,542 行）：

| id 形态 | 行 | 已并入 | **存活** |
|---|---|---|---|
| 裸 md5（波次①）| 21,083 | 1,661 | **19,422** |
| 裸数字（波次①）| 3,188 | 2 | **3,186** |
| `<库名>_<md5>`（波次③）| 21,083 | 21,083 | 0 |
| `<库名>_<数字>`（波次③）| 3,188 | 3,188 | 0 |
| **存活合计** | | | **22,608** ← **与图谱实体节点数 22,608 逐位相同** |

★★ **两条独立推论在此会合**：`lifecycle IS NULL` 的语义就是**「没被合并掉」**，
而图谱的实体节点数正是 `lemonhu` 域存活行数。§14 记的「节点集 = lemonhu 域 AND `lifecycle IS NULL`」得到了机制层解释。

**主实体归属**（被别的行 `merged_into` 指向的行）：

| 库 | 主实体行数 |
|---|---|
| `lemonhu` | 22,607 |
| `f9b2b050` | **1** |

**跨库指向统计**：

| 库 | 已并入行 | canonical 同库 | **canonical 跨库** | canonical 找不到 |
|---|---|---|---|---|
| `f9b2b050` | 24,270 | **0** | **24,270（100%）** | 0 |
| `lemonhu` | 25,934 | 25,933 | **1** | 0 |

两表可交叉验算：`lemonhu` 的 `1,661+2+21,083+3,188 = 25,934` ✓、`f9b2b050` 的 `21,082+3,188 = 24,270` ✓。

### 20.5 这一行就是 §18 那 276 条悬空边的唯一源头

★ `lemonhu` 里那**唯一 1 条**跨库 canonical：

```
主实体（canonical） = f9b2b050-e336-4de8-b305-0b40cbf5740f_50371a2c5078b757a8f8c75b8877e815
被并入的行（lemonhu）= lemonhu_knowledge_graph_50371a2c5078b757a8f8c75b8877e815   ← 名字「次新股」
```

与 §18 实测的「276 条边的 **distinct 缺失端点 = 1 个** = `entity:f9b2b050-…_50371a2c…`（**次新股**）」
**逐位相同**。机制链：

1. 合并的分组键曾是 `(name, entity_type)`、**不含 `knowledge_base_id`** ⇒ 同名同类型实体被**跨库**合并；
2. 主实体选择 = 「同组内保留最早创建的」 ⇒ 绝大多数落到 lemonhu（7-24 最早）；
3. `update_relation_references` 只改边两端的 id、**不改 `knowledge_relations.knowledge_base_id`** ⇒
   **边留在 lemonhu 域、端点却指向 `f9b2b050` 的 id** ⇒ 该域可见实体里没有它 ⇒ 276 条画不出来的边。

这条链与 `src-tauri/crates/dao/src/repo/knowledge_graph.rs:1409-1430` 的文档段**完全吻合**
（那段标题就是「⚠ 2026-09-17：分组键为什么必须带 `knowledge_base_id`」）。
⇒ **§18 不是新缺陷，是这次合并的遗留；本轮 A 已把 276 条删净，B 已让今后的同类脏数据不再虚高边数、但仍然报警。**

**A 执行后的现存健康度**（独立复查，`target_*` 三个维度全 0）：

| 域 | 关系行 | target 不存在 | target 属别库 | target 已被合并 |
|---|---|---|---|---|
| `lemonhu` | 75,251 | **0** | **0** | **0** |
| `f9b2b050` | 6 | **0** | **0** | **0** |

### 20.6 第八批：我自己的口径错误（**全部是「测量工具撒谎」型**，记忆 #8 的又一实例）

| # | 错误 | 症状 | 正解 |
|---|---|---|---|
| 1 | **★ `SELECT DISTINCT tail` 把 NULL 当成一个值** | 集合差凭空多出 `+1`：早前 `q-setdiff.txt` 报 `only_uuid_prefixed = 1`、`q-align.txt` 报 `a_tails = 21084` 而 `a_n = 24274`，我都差点读成「有 1 个独有实体」 | `AND tail IS NOT NULL` 后两侧差集 **0 / 0**；脚本里加了**口径自证**（同时打印含 NULL 与不含 NULL 两个计数并说明差 1 的来源）|
| 2 | **用 `position('_' in id)+1` 取后缀** | `lemonhu_knowledge_graph` **自身含下划线** ⇒ 切出 `knowledge_graph_…` ⇒ 得出 `md5_overlap = 0` 的**假结论**（`q-kb-overlap.txt` 仍留着这个假 0）| 正则 `([0-9a-f]{32})$` 取**末尾** 32 位 |
| 3 | **把两个不同口径的数并列当同一量** | `q-forms` 报 `f9b2b050.uuid_prefixed n = 24,271`、`q-duprows` 报 `n = 3191`，我把后者读成「库内重复行」 | 真值：`f9b2b050` 库内 **tail 级重复 = 0 行**；3,191 = 3,188 条 `<uuid>_<数字>` ＋ 3 条裸 uuid（**「尾巴不含 32 位 hex 的行数」**，不是重复）|
| 4 | `string_agg(lifecycle)` | DB 报「函数不存在」 | `lifecycle` 是 **json**，须 `::text`。**DB 自己挡住的错误属良性** —— 值得注意的反而是前三条 DB 挡不住的 |
| 5 | 照抄别表的列名 | `knowledge_bases.created_at` 不存在 | 先查 `information_schema.columns` 再写查询（本轮已照做，5 秒成本）|

**★ 可复用判据**：**「集合差恰好 ±1」先查 NULL 桶，不要先查被测对象。** 同理任何「恰好差的很小」的集合运算结果，都优先怀疑口径（`NULL` / 大小写 / 单位 / 去重键），而不是数据本身。

### 20.7 处置选项（**未改任何东西**，等裁决）

| 选项 | 动作 | 风险 / 前置 |
|---|---|---|
| **(a) 只留 lemonhu** | 删 `f9b2b050` 那条 `knowledge_bases` 记录（或先 `enabled=0`）| ⚠ 必须先**穷举引用**：已知以它为域的行有 实体 24,274（存活 4）、关系 6、文档 24,271；`knowledge_attributes` / `knowledge_flows` / `knowledge_interfaces` / `paper_overviews` / `retrieval_hits` / `conversations.enabled_knowledge_base_ids` **尚未查** |
| **(b) 改名 + 停用（最小动作）** | `enabled = 0`，名称标注「（已废弃副本）」| 保留全部数据可回滚；UI 上不再出现第二份 |
| **(c) 不动 DB、只在 UI 过滤** | — | 不推荐：库真实存在且 `enabled=1`，属掩盖而非修复 |

**代码侧**：`f9b2b050` 在 **Rust 源码里只出现在注释/文档与 1 条测试夹具字符串**
（`dao/src/repo/knowledge_graph.rs:1255`、`:1418`、`dao/src/reconcile/extras.rs:1511` 的索引名派生用例），
**不参与任何生产逻辑** ⇒ 删库不会断代码；但那 3 处引用会变成**文档漂移**，删库时须一并更新。

### 20.8 复现资产

| 文件 | 说明 |
|---|---|
| `output/verify-graph-2026-09-15/q-dupimport-final-0918.mjs` / `.txt` | 终局探针：**9 节自述式输出**（每节标清数是「行数」还是「去重 tail 数」），含 20.3 的口径自证 |
| `q-kb-overlap.txt` / `q-setdiff.txt` / `q-align.txt` / `q-forms.txt` / `q-duprows.txt` | 早前脚本输出，**口径有坑**（`position` 假 0 / NULL 桶 +1 / 两口径并列）—— 留作对照，勿当判据 |
| `knowledge_entities_20260916` / `knowledge_relations_20260916` | **合并前备份表**（⚠ **不是我建的**，是 2026-09-16 那次合并自带的）|

---

## §21 「二选一」执行：删掉 `f9b2b050`（用户裁决 2026-09-18）

### 21.0 三句话结论

1. **方向由两条不对称硬证据钉死**，不是偏好问题：`lemonhu` 持有**全部 22,608 个存活实体**（= 全图 canonical 集）与 **75,251** 条关系；而 `f9b2b050` 存活实体只有 **4** 条、关系 **6** 条，且那 4 条里 3 条是 mojibake 乱码名。⇒ 删 A，留 lemonhu。
2. **删库前必须做 4 件「反悬空」事**，否则删完立刻长出新的悬空引用：改 1 行 `merged_into`、改 7 处模板 `nodes`、**刻意保留** `vec_collections` 登记行、**不删** `vec_kb_f9b2b050*` 三张表。
3. 终态：`knowledge_bases` 4 → **3**，A 全域归零（`{ent:0,rel:0,doc:0,kb:0,wt:0}`），lemonhu 四项**逐位不变**，悬空 **0 / 0**。

### 21.1 决策依据（不对称证据，逐条可复算）

| # | 证据 | `f9b2b050`「A股知识图谱」 | `lemonhu_knowledge_graph` |
|---|---|---|---|
| 1 | 存活实体（`lifecycle IS NULL`） | **4**（其中 **3** 条乱码名） | **22,608** |
| 2 | 关系行 | 6 | **75,251** |
| 3 | wiki `dc8efbd8`「A股知识」的 `knowledge_base_id` | — | **就是它**（说明真正在用哪个） |
| 4 | 向量集合 | `vec_kb_f9b2b050…` **主表 0 行**（死集合，只剩 meta 24,516 + `_meta_dup_backup` 24,518） | `vec_wiki_dc8efbd8…` **49,072 行**（在用） |

### 21.2 删除前的 14 项前置自证（全绿才允许 `--apply`）

脚本 `output/backup-2026-09-18-retire-kb/retire-kb-f9b2b050-0918.mjs`（**默认只读 = dry-run**，写库必须显式 `--apply`；`pg` 直连 + 参数化绑定）。
自证覆盖：实体/关系/库行/文档行数、A 存活恰 4、`merged_into` 命中 A 的**恰 1** 行、改指目标行**确实存活**、lemonhu 无关系边指向 A、文档标题**宽口径 + `(title, source_path, size_bytes)` 严口径**、A 实体指纹**全有孪生**、vec 主表 0 行、模板里 A 恰好出现 **7** 次、全库仅 1 行 `merged_into` 指进 A。
`dryrun.log` 实测 **14/14 ✔**。

> ★ 其中第 2 项一开始写成 `knowledge_base_id <> A AND merged_into IS NOT NULL` ⇒ 数出 **25,934**（lemonhu 全部已合并行），我把差点读成「有 25,934 处引用」。
> 真值是「**merged_into 指进 A** 的条数 = 1」，口径应为 `lifecycle->>'merged_into' LIKE '$A%'`。**这是测量口径错，不是数据脏**（与 §20.6 同族）。

### 21.3 执行（单事务 + 事务内对拍 + 提交后独立连接复查）

| 步骤 | 动作 | 实测 |
|---|---|---|
| 事务内指纹对拍 | 三份 md5 与备份逐位相同 | `86851c02…` / `65c24636…` / `52159128…` ✔ |
| STEP1 | `UPDATE knowledge_entities SET lifecycle = replace(lifecycle::text,$A,$L)::json WHERE … merged_into = $A` | rowCount = **1**，新值指向 `50371a2c…`（lemonhu 里存活的那条），另两键 `merged_at_ms=1789502375024`、`group_mention_sum=4` **逐字保留** |
| STEP2 | `UPDATE workflow_templates SET nodes = replace(nodes,$A,$L) WHERE nodes LIKE %A%` | rowCount = **1**，A 归零、lemonhu 恰 7 次 |
| STEP3–6 | 分批 `DELETE`（BATCH=500，逐批 `rowCount` 断言） | 实体 **24,274** / 关系 **6** / 文档 **24,271** / 库行 **1**，全部与计划相等 |
| 终态 | — | `knowledge_bases` 4 → **3**；实体 72,816 → **48,542**；关系 75,257 → **75,251**；文档 48,542 → **24,271** |
| lemonhu 自证 | 四项逐位不变 | 实体 48,542 / 存活 **22,608** / 关系 **75,251** / 文档 **24,271** ✔ |
| 悬空 | — | **0 / 0** ✔；`merged_into` 悬空 **0**（`merged_rows` 50,204 → 25,934）|

**剩余三个库**：`__sys_memory_reflow__`（0 行）/ `__sys_trajectory__`（0 行）/ **`lemonhu_knowledge_graph`（`enabled=1`）**。

### 21.4 刻意**不动**的东西（每条都有理由，不是遗漏）

| 对象 | 处置 | 理由 |
|---|---|---|
| `workflow_templates.nodes` 里 7 处 `knowledge:f9b2b050…` | **改指 lemonhu** | `RagSource::from_knowledge_base` 由 `knowledge_bases` **行**构造 ⇒ 库行一删，这个源**连构造都做不了**，不能只登记 |
| lemonhu 那 **1** 行 `lifecycle.merged_into` | **改指** lemonhu 自己存活的同指纹行 | 原值指向 A ⇒ 删 A 后它是**悬空 canonical** |
| `vec_collections` 登记行 | **保留** | 它是「24,516 块切片在哪」的**唯一线索**，且不是外键 |
| `vec_kb_f9b2b050*` 三张表 | **不删** | 含**手工备份**（`_meta_dup_backup`），删表不可逆 |
| `index_jobs` 49,762 行（`container_id` = A） | **保留** | 历史作业流水 / 审计轨迹，**不是引用** |
| `workflow_template_versions` 21 行 | **保留** | 模板**历史快照**，快照就该保持当时原样 |
| `lifecycle` 用 `replace(lifecycle::text,…)::json` 而不是 `jsonb_set` | — | `lifecycle` 列是 **`json`**（非 jsonb）；走 jsonb 会**按 key 重排**⇒ 除目标串外逐字节不动这条不成立 |

### 21.5 复现资产

`output/backup-2026-09-18-retire-kb/`：`rows-*.json` 8 份全列备份 + 三份 md5 指纹 · `dryrun.log`（14/14）· `apply.log`（闭环）· `plan-0918.txt` · `CHECKSUMS-0918.txt` · 执行器 `retire-kb-f9b2b050-0918.mjs`。

**遗留登记（文档漂移，未改）**：`f9b2b050` 仍出现在 3 处注释/测试夹具（`dao/src/repo/knowledge_graph.rs:1255`、`:1418`、`dao/src/reconcile/extras.rs:1511`）。

---

## §22 `check:contracts` 两条既有红的修复（用户裁决 2026-09-18）

### 22.0 三句话结论

1. `[D]` 的根因是 `runtime-core/src/cron_job.rs` **自己拿着 sea-orm + 实体建表并 upsert**，而该门禁规则是「consumer crate（agent / gateway / orchestrator / **runtime-core**）只允许依赖 `axagent-harness`」。
2. 修法**不是放宽门禁**，而是补上本 crate 在 `cron_delivery.rs` 已经写清楚的同一套 **port/adapter**（「DTO + Trait 在 harness，具体实现在 wiring 层」）——`cron_job.rs` 只是漏了这一层。
3. `[H]` 的棘轮基线 **278 → 273**（实测 `共 273 处，已排除 47 处豁免调用`）；重跑后 `errors=0 warnings=0`，**PASS**。

### 22.1 `[D]` 下沉清单

| 层 | 文件 | 内容 |
|---|---|---|
| port | `crates/harness/src/cron_persistence.rs`（新） | `CronHistoryRecord`（纯数据，**不引用**表实体）+ `#[async_trait] trait CronJobPersistence`（`ensure_tables` / `load_jobs` / `upsert_job` / `delete_job` / `insert_history` / `load_history`）|
| adapter | `crates/dao/src/repo/cron_job_persistence.rs`（新） | `PgCronJobPersistence`，逐字保留原实现的两条既有修复（建表语句由实体生成、`cron_job_history` 三个时间戳列必须 BIGINT 的 PG-only 迁移）|
| consumer | `crates/runtime-core/src/cron_job.rs` | 删 `use axagent_entities::…` 与全部 `sea_orm` 导入、删 3 个建表函数；`db: Arc<DatabaseConnection> + persist: bool` → **`persistence: Option<Arc<dyn CronJobPersistence>>`**（用 `Option` 让「没有持久化」在类型上不可绕过）；新增 `fn port()` |
| wiring | `src/init/state.rs:520` | 构造点从 `CronJobStore::new(Arc::new(sea_db.clone()))` 改为注入 `Arc<dyn CronJobPersistence>` |
| manifest | `crates/runtime-core/Cargo.toml` | 删 `axagent-entities` / `sea-orm` 两行（`[dependencies]` 段内 `axagent-*` 只剩 `axagent-harness`）|

**接线核对（两跳 grep，不是只看一行）**：`CronJobStore::new(` 全仓只有 `src/init/state.rs:520` 一处（其余 7 处都是 `new_ephemeral()`）⇒ 没有漏改的构造点；`cron_job.rs` 里 `self.db` / `cron_job::Entity` / `Set(` / `DbBackend` / `Schema::` 的**残留引用为 0**（只剩注释）。

### 22.2 `[H]` 棘轮

```js
const SILENT_RESULT_BASELINE = 273;   // 原 278
```
附 6 行注释：实测依据、归因（此前几轮清理后基线没跟着走 ⇒ 棘轮松了 5 格）、回滚方式。降级依据是**实测计数**（`共 273 处`），不是为了让门禁变绿。

### 22.3 验证

| 判据 | 命令 | 结果 |
|---|---|---|
| `[D]` + `[H]` | `node scripts/check-contracts.mjs --only=d,h` | `[H] 273 处`、**无 `[D]` 输出**、`汇总 errors=0 warnings=0`、**PASS** |
| 编译 | `cargo check -p axagent-runtime-core -p axagent-dao -p axagent-harness` | `Finished dev profile in 41.12s`，EXIT=0 |
| 格式 | `cargo fmt -p axagent-runtime-core -p axagent-dao -p axagent-harness -p axagent` | EXIT=0（未连带改动任何其它文件，写集实测恰 7 个文件） |

### 22.4 ★ 第九批我自己的错误（两条，都是「工具/流程」型）

| # | 错误 | 症状 | 正解 / 纪律 |
|---|---|---|---|
| 1 | **★ 补丁脚本的锚点只校验「被替换区域」，校验不到「替换文本与原上下文重复」** | block 1 把 `:8-14` 换成新文本，末尾又写了一遍 `use serde::{Deserialize, Serialize};`，而 `:15` 本来就是它 ⇒ **重复导入 `E0252`**。三重锚点（首行 + 中段 + 末行）**全部通过**，脚本报「✔ 已写入 5 个文件」 | **行区间补丁必须紧跟一次编译**——锚点只能证明「我在改我以为的那段」，证明不了「改完还自洽」。另：`assertTo` 取 `};` / `    }` 这类**通用行**等于没校验，中段锚点要取区间内**唯一**代码 |
| 2 | **★ 改源码前没查进程表 ⇒ 与在飞的 `cargo run` 竞态** | 写盘那一刻用户 PID 36088 正在编译 `axagent-harness`（`Get-CimInstance` 实测）。若它已读完旧 `lib.rs`，紧接着编 `runtime-core` 就会报模块不存在 —— 用户看到的是「我的 dev 构建被 AI 搞坏了」 | **改 Rust 源码前先查进程表**：有别人的 `cargo run` / `check` 在飞 ⇒ 要么等它结束，要么明确承认会与之竞态并在汇报里说清。本次侥幸（用户构建在我最后一次写盘后 1 秒重启，看到的是完整一致的树），但**侥幸不是流程** |

---

## §23 实体侧单独算社区（用户裁决 2026-09-18）

### 23.0 三句话结论

1. 融合图里 **22,608 个实体节点此前没有任何社区**（Louvain 只跑笔记图），只能靠 `mapping` 锚点**继承同名笔记的桶** —— 那是补丁，不是结论：实体的 75,251 条 `reference` 边**从未参与过分区**。
2. 实测代价（真实载荷 46,896 节点 / 172,650 边）：锚点路径下 **全部 22,608 条 `mapping` 边 + 50,059 条 `reference` 边落在桶内、被聚合层直接丢弃** —— 桶级密度 9.8% 这个好看的数字正是这么换来的。
3. 改为**后端在实体子图上单独跑一次 Louvain**（`LouvainResult.entityCommunities`），前端把两份社区**错开命名空间**后一起送进拓扑归并；锚点路径降级为兜底。

### 23.1 为什么必须错开命名空间（本改动的核心判据）

两次 Louvain 是**独立运行**，各自 cid 都从 0 起算 ⇒ 值域重合。直接 `new Map([...notes, ...entities])` 会让两侧的 `cid = 7` 在数值上撞上，后果是：
- `countDistinctCommunities` **低估**社区数；
- 归并阶段把两个拓扑无关的社区当成**同一个簇**求 ΔQ（边权与度数直接相加）⇒ 被强制并进一个桶 ⇒ 桶内边被 `buildAggregateGraph` 的 `sIdx === tIdx` 分支丢弃 ⇒ 结构被抹掉。

**对拍实测（合成等规模数据）**：错开 **3,358** vs 朴素合并 **2,704** ⇒ 被折叠 **654** 个社区（= 两侧 cid 值域交集 `[246, 899]` 的大小，可算术复算）。

### 23.2 改动清单

| 层 | 文件 | 内容 |
|---|---|---|
| DTO | `harness/src/louvain_dtos.rs` | `LouvainResult` 新增 `entity_communities: Option<HashMap<String,i32>>`（`#[serde(default)]` ⇒ 旧缓存 JSON 仍可解析）|
| DTO 构造 | `dao/src/repo/louvain.rs` ×2、`dao/tests/pg_upsert_bare_column.rs` ×1 | 补字段（后者刻意给**非 None** 值，让新字段真的进一次序列化 → 落库 → 读回）|
| 后端 | `commands/wiki.rs` | ① 提取 `load_fused_entities(db, kb_id)`（原第 3 步，30s TTL 缓存），**融合图谱与实体侧社区检测共用同一实体集**；② 原 `wiki_graph_communities_cached` 正文改名为 `note_communities_cached(&State, String)`（三处 `return` **原样不动**）；③ 新命令入口在**所有早返回之后**统一附加实体侧结果并回写缓存 |
| 前端纯函数 | `components/wiki/communityMerge.ts` | 新增 `mergeEntityCommunities(notes, entities)`：实体 cid 偏移 `max(笔记 cid) + 1`（**派生值**，常量会在值域增长时静默失效）|
| 前端消费 | `GraphView.tsx` | 新 prop `entityCommunities`；合成放在 forceCluster 判定**之前**（小图分支同样需要）；`rawCommunitiesRef` 与 `colorCommunities` 同步用合成映射（否则主题切换会退回「只有笔记侧」⇒ N5 那类错位复发）；deps 补 `entityCommunities` |
| 前端契约 | `WikiGraphPage.tsx` / `lib/browserMock.ts` | `entityCommunities?` 逐字段独立判定（笔记侧有、实体侧无时**不能**一起清空）|

**设计上刻意不做的事**：没跑「笔记 + 实体」的**单次** Louvain —— 那会重排笔记侧 cid（社区面板语义跟着变）且把 24k 笔记混进来稀释实体分区；也没给「主路径桶数超限」加任何前端兜底 —— 社区粒度与桶数的分工维持原样（后端出细粒度，前端归并到 ≤200 桶）。

### 23.3 验证（真实载荷 46,896 节点 / 172,650 边）

| 判据 | 结果 |
|---|---|
| 命名空间互斥 | 实体 cid 全部 > 最大笔记 cid，两侧交集 **∅**；合成后社区数 = 笔记 2,458 + 实体 900（**可加**）|
| 覆盖 / 兜底 | 合成数据下「无桶节点」= **0**，`assignMissingCommunities` 三路计数全 **0**（锚点一次都不走）|
| 主路径 vs 锚点兜底 | **46,855 / 46,896** 个节点给出不同桶 ⇒ 实体侧社区**真的被消费**（不是摆设）|
| 规模不变量 | 聚合节点 ≤ 800、真实节点混入 **0**、聚合边 > 0（真实 fixture：桶 200 / 聚合边 1,947 / 密度 9.8%）|
| 门禁 | tsc EXIT=0 · dprint EXIT=0 · vitest `kgFusedScale` + `communityMerge` **32 passed** |

### 23.4 ★ 顺带修掉一处「测试静默降级」（与本轮改动无关，但正是本轮才暴露）

`__tests__/kgFusedScale.test.ts` 写死的真实 payload 路径 `kg-fused-payload-0918.json` 在 §21 那轮**被改名**成了 `-before-delete-`、另出了 `-after-delete-` 版本 ⇒ `realFixturePresent` 静默变 `false`，**所有「真实 payload 下的口径自证」一次都没跑过**，界面上却仍显示「不跳过」（确实不跳过，只是跑的是合成数据）。

⇒ 改为**候选路径列表**（优先现状 `after-delete`）+ **打印实际用了哪一份** + 缺失时 `console.warn` 列出全部候选。修复后首次实测：`REAL(kg-fused-payload-after-delete-0918.json) 节点 46896 … 边 172650 … 聚合边 1947 密度 9.8%`。
**可复用判据**：任何「找不到就用替代数据继续跑」的判据，都必须把「用了替代数据」这件事**打印出来**；否则它的腐烂形态与「跑过了」完全一样。

### 23.5 诚实的边界（未验证的部分）

- 真实 fixture **没有** `entityCommunities`（它由迁移前的后端产出）⇒ 「主路径在真实拓扑上的桶级密度」**尚未量化**。要量它需要在跑过新版后端后重新 dump：
  `node output/verify-graph-2026-09-15/kg-fused-payload-dump-0918.mjs output/verify-graph-2026-09-15/kg-fused-payload-with-entity-comms.json`
  （dump 脚本读 `wiki_graph_cache.communities_json`，故需先让应用跑一次以写缓存）。
- 实体图缓存是 **30 秒 TTL**、社区缓存随 notes 写入失效 ⇒ 两者**可能不同源**，新出现的实体会暂时没有社区；那批节点走锚点兜底且**可观测**（`warnOnHashFallback`），不是静默丢弃。
- 实体子图 Louvain 的耗时（22,608 节点 / 75,251 边）与笔记图同量级，实测**未单独计时**；结果随 `LouvainResult` 写进 `wiki_graph_cache`，只在「未算过」时执行一次。

### 23.6 复现资产

`output/verify-graph-2026-09-15/`：`patch-D-sink-0918.mjs`（§22 的原子补丁）· `contracts-DH-after-0918.txt` · `check-D-after-0918[b].log` · `check-entity-communities-0918.log` · `vitest-entity-communities-0918.log` · `tsc-entity-communities-0918.log` · `dprint-*-entity-communities-0918.log`。

---

## §24 行号引用漂移校正（第三轮，2026-09-18 晚）

### 24.0 三句话

1. §23 在 `src/commands/wiki.rs` 的三处编辑（顶部 +1 行 use、中段提取函数、尾部拆函数）让全仓 **28 处**「`wiki.rs:NNNN`」引用失效 —— 我**自己造的**，不是存量。
2. 校正用脚本 + **双重断言**（旧引用命中次数 + **新引用回读目标行内容**），共 **38 组替换 / 28 处引用 / 7 个文件**，回读自证 **24/24 通过**。
3. ★ **不能按偏移量推算**：这两个文件本轮各自的偏移量都是**分段常量**（`wiki.rs` 实测 **+1 / +13 / +84**，`GraphView.tsx` 实测 **+23 / +33 / +41**）⇒ 推算必错 —— 每个新值都用 `grep` 在**当前**源码里按内容定位。

### 24.1 漂移量为什么是「分段常量」而不是一个数

#### `wiki.rs`（§23 的三处编辑叠加，同一文件内分段）

| 编辑 | 位置 | 对该点之后的影响 |
|---|---|---|
| 新增 `use sea_orm::{ConnectionTrait, DatabaseConnection};` | `:17` | **+1**（全文件） |
| 融合第 3 步提取为 `load_fused_entities` | `:1054-1108` | 累计 **+13** |
| 原命令拆成 `note_communities_cached` + 新入口 | `:1266-` | 累计 **+84** |

#### `GraphView.tsx`（§23 的前端编辑，同一文件内分段）

| 编辑 | 位置 | 对该点之后的影响 |
|---|---|---|
| 新增 import `mergeEntityCommunities` | `:44` | **+1** |
| `GraphViewProps` 新增 `entityCommunities` + 说明 | `:328-336` | 累计 **+23** |
| `effectiveCommunities` 段开头插入合成逻辑 | `:1245-1250` | 累计 **+33** |
| `assignMissingCommunities` 段新增说明 | `:1305-1322` | 累计 **+41** |

⇒ 同样是「文件里的一行」，在 `wiki.rs:500` 处偏 1、`:1100` 处偏 13、`:1500` 处偏 84；在 `GraphView.tsx:743` 处偏 23、`1275` 处偏 33、`1891` 处偏 41。
**判据：改了行数之后，任何「旧值 + 某常量」的批量推算都是错的；必须逐条按内容重新定位。**
（★ 首版本节把两个文件的偏移量混成一句「同一次编辑 +1/+13/+23/+33/+41」——**归属错误**：+23/+33/+41 来自另一个文件。已当场更正。）

### 24.2 校正清单

| 消费方 | 旧引用 | 新引用 | 依据（新值的验证锚点） |
|---|---|---|---|
| `crates/harness/src/knowledge_graph.rs`（×2） | `wiki.rs:1173` | **`:1186`** | `mapping_edges.push(GraphEdge::structural(` |
| `crates/harness/src/page_type.rs`（×3） | `wiki.rs:1746` | **`:1830`** | `page_type: Some("daily".to_string())` |
| `crates/harness/src/page_type.rs`（×3） | `wiki.rs:1992` | **`:2076`** | `page_type: Some("knowledge".to_string())` |
| `crates/harness/src/page_type.rs`（×3） | `wiki.rs:1492` | **`:1576`** | `page_type: Some("synced".to_string())` |
| `__tests__/kgFusedScale.test.ts`（×2） | `GraphView.tsx:1850` | **`:1891`** | `const aggOver` |
| `__tests__/kgFusedScale.test.ts`（×1） | `GraphView.tsx:1240-1243` | **`:1273-1276`** | `聚合图边密度 82.69%` |
| `__tests__/kgFusedScale.test.ts`（×1） | `GraphView.tsx:703 / :706 / :720` | **`:726 / :729 / :743`** | 三个规模常量逐行命中 |
| `__tests__/kgFusedScale.test.ts`（×1） | `GraphView.tsx:1218-1304` | **`:1242-1344`** | 段首/段尾双锚点 |
| `__tests__/kgFusedScale.test.ts`（×1） | `wiki.rs:1160-1179` | **`:1173-1192`** | 映射边段首/段尾 |
| `GraphView.tsx`（自引用 ×1） | `本文件 :1240-1243` | **`:1273-1276`** | 同上 |
| `GraphView.tsx`（×1） | `wiki.rs:1160-1179` | **`:1173-1192`** | 同上 |
| `communityMerge.ts`（×2） | `GraphView.tsx:1240-1243` · `wiki.rs:1160-1179` | **`:1273-1276`** · **`:1173-1192`** | 同上 |
| `stores/shared/backendStatusStore.ts`（×1） | `wiki.rs:495` | **`:529`** | `wiki-rebuild-complete` 发射点（★ **既有漂移**，见 24.4） |
| `kg-fused-payload-dump-0918.mjs`（×6） | `:1065-1228` · `:1138`×2 · `:1148-1152` · `:1160-1179` · `:1169` | **`:1119-1264`** · **`:1089`** · **`:1100-1101`** · **`:1173-1192`** · **`:1182`** | 逐个回读目标行 |

所有新值与旧值**长度完全相同** ⇒ 不触发 rustfmt / dprint 重排（已由 `dprint check` + `cargo fmt --check` 复核）。

### 24.3 双重断言机制（以及它当场拦下的一次错）

脚本 `fix-linerefs-round3-0918.mjs` 的写盘前断言：

1. **旧引用在同一消费方文件里的出现次数 == 期望值**；不等 ⇒ **一个文件都不写**。
2. 写盘后**逐条回读目标文件的那一行**，断言含所述内容；不含 ⇒ 退出码 2 并要求再修。

★ 第一次执行被拦下：`kg-fused-payload-dump-0918.mjs` 里 `wiki.rs:1138` 我写 `n=1`，实测 **2** 处（`:10` 头部 + `:64`「实体节点（id 加前缀，与 wiki.rs:1138 一致）」）。**脚本拒绝写盘**。
若只做「有就替换」，`:64` 那处会被**静默漏掉** —— 而它恰好是**紧贴代码**的那条引用，比头部那条更容易被下一个读者当真。

> 可复用判据：**「替换成功」只证明匹配上了，不证明匹配全了。** 批量替换必须带命中次数断言，且期望值要能被独立来源复核。

### 24.4 归属：哪些是我造的，哪些本来就在

| 类别 | 处数 | 说明 |
|---|---|---|
| **本轮自引入** | 27 | §23 对 `wiki.rs` / `GraphView.tsx` 的编辑所致 |
| **既有漂移** | 1 | `backendStatusStore.ts:130` 的 `wiki.rs:495`：真值 529，**偏 33 行**；我这轮只贡献其中 +1 ⇒ 改动前就已偏 32 |

既有那处一并修正，但**不冒充「我引入的」** —— 否则下一次有人查「这漂移哪来的」会查错轮次。

### 24.5 ★ 门禁为什么没拦住（两个盲区叠加）

| 盲区 | 实测 |
|---|---|
| `check:facts` **只扫 `.rs`** | 门禁自陈：「本门禁只扫 `.rs` ⇒ `.md` 里的引用**一条都没查过**」⇒ 前端 `.ts/.tsx` 与 `.md` 全在覆盖面外 |
| 硬判据**只拒空行/越界/裸控制流** | 偏 84 行的引用仍指向**非空行** ⇒ 落 `LOCATED`、不报（§19 已记过同型：`dao:1349→1794` 偏 445 行长期无人发现） |

实测反证：改完这 28 处**之前**先跑了一次 `check:facts` ⇒ `LOCATED 398 / BROKEN·越界·空行·控制流 0`、`✔ 无新增客观错`、**EXIT=0** —— 门禁对这批漂移**完全不敏感**。
⇒ **判据：门禁全绿 ≠ 行号引用是活的。改过行数的文件，必须自己 `grep '文件名:\d+'` 全仓主动扫。**

### 24.6 诚实的边界

- **`docs/**` 里的引用这次没动**：审计/计划文档按「时间快照」使用，正文保留写下时的行号才有考古价值；本轮只登记换算关系（§24.1 的分段偏移量足以换算）。`docs/plans/PLAN-weknora-borrowings.md`、`PLAN-declarative-schema-sync.md` 里另有**更早的**既有漂移（如 `cron_job.rs:38/441`、`wiki.rs:1660`），**非本轮引入、本轮也未处置**。
- `output/backup-*/**` 下的历史副本里同样有旧引用 —— **刻意不动**（它们是备份，改了就不是备份了）。

### 24.7 复现资产

`output/verify-graph-2026-09-15/`：`fix-linerefs-round3-0918.mjs`（带双重断言，可重跑自证）· `fix-linerefs-round3-0918.txt`（38 组替换 + 24/24 回读自证）· `facts-after-entity-communities-0918.log`（改前跑门禁 ⇒ 全绿，证明盲区）· `tsc-linerefs-0918.log` · `dprint-linerefs-0918.log` · `vitest-linerefs-0918.log`。

---

## 25. 全量 harness 测试里那两条红的归属（2026-09-18）

### 25.0 结论先行

| 失败用例 | 归属 | 处置 |
|---|---|---|
| `domain_router::tests::test_stock_analysis_input_routes_to_finance` | **既有竞态**（随机红），非本轮改动引入 | **顺手修**（纯测试基建缺陷 = 缺一把锁，零产品语义） |
| `graph_dtos::tests::test_dangling_edges_always_serialized_and_legacy_parseable` | **既有红**，fixture 与同 crate 词汇表自相矛盾 | **只登记，不代改**（修法含产品语义二选一 + 该文件有非我轮次的写入，见 §25.4） |

原始观测：`test-harness-0918.log` ⇒ `running 718 tests` / `716 passed; 2 failed` / `TEST_EXIT=101`。我本轮新增的 4 条 `louvain_dtos::tests::*` **全过**（`log:340-345`）。

### 25.1 归属一：`domain_router` 是竞态，不是我的改动

| 证据 | 实测 |
|---|---|
| 该文件 mtime | **`2026-09-15 15:28:54Z`** ⇒ 本轮（首份落盘产物 18:08Z 起）**从未写过它** |
| **反证（决定性）** | `b-harness-test-0918.log` 是**更早一轮**的运行，`16 tests` + `698 filtered out` ⇒ 当时总数 **714 = 718 − 我新增的 4 条**；其中**同一条用例、逐字相同的 panic 文本**同样 FAILED ⇒ 早于我的改动 |
| 机制 | `domain_router.rs` 的 3 条 `route` 用例里**只有 1 条持锁**；`test_route_skips_disabled_domain` 会在其临界区内**短暂停用 finance** ⇒ 未持锁的读者读到「停用态」 |
| 为什么失败信息自相矛盾 | 规则优先级 `finance 88 > data_analysis 80`，红时却报 `left: DataAnalysis` ⇒ 只可能是 finance 被**临时停用**（停用后的兜底域正是 data_analysis） |

**复现矩阵**（`repro-domain-flake-0918.log`）：

| 运行方式 | 结果 |
|---|---|
| `--exact` 单跑该用例 | **1 passed**，`EXIT=0` |
| 并行子集 `domain_router::tests`（3 条）8 轮 | **1 红 7 绿**（第 1 轮红，`EXIT=1` 的 `left: DataAnalysis`；其余 7 轮全绿）|

⇒ 判定：**既有、随机、与源码逻辑无关**。`domain_registry.rs:553-558` 的模块文档其实**早就写下了这个症状**（「若两个模块各持一把锁，就等于没锁 —— 症状是随机红的、与本机改动无关的失败」），只是**遗漏了这两条读者**。

### 25.2 处置：给读者补锁（含同型穷举）

**穷举结论**（全 crate 扫「读覆盖层」的用例，按量纲而非按文件）：

| 位置 | 是否持锁 | 结论 |
|---|---|---|
| `domain_router.rs:935`（finance 路由；guard 在 `:942`） | 补锁**前**：否 ← 本次红的那条 | 补 |
| `domain_router.rs:958`（data_analysis 路由；guard 在 `:962`） | 补锁**前**：否 | 补（同型：当前无「停用 data_analysis」的用例故暂不会红，但读的是同一份全局） |
| `domain_router.rs:984`（停用域跳过；guard 在 `:989`） | 一直是 | — |
| `domain_registry.rs` 8 条覆盖层用例（`:856/873/888/913/942/975/1008/1038`） | **全部是** | — |
| `cognitive_router.rs:1764/:1808` 调 `route(...)` | 不适用 | 走 `MockDomainRouter` ⇒ **不读**真覆盖层 |
| `dao/src/repo/capability_domain_override.rs` 测试读同一份 `resolve_enabled_domain` | 无法持锁 | **登记**：属**另一个测试二进制**（进程隔离），且 harness 的锁是 `#[cfg(test)] pub(crate)`、跨 crate 不可见 ⇒ 它需要自己的串行化机制（本轮不处置） |

**修复 diff**（2 行 + 注释，`crate::domain_registry::test_support::overlay_case()` 即 `domain_registry.rs:588` 那个**全 crate 唯一**的锁，且带前置清空）：

```rust
// test_stock_analysis_input_routes_to_finance
let _guard = crate::domain_registry::test_support::overlay_case().await;
// test_generic_analysis_still_routes_to_data_analysis
let _guard = crate::domain_registry::test_support::overlay_case().await;
```

**验证矩阵**：

| 判据 | 结果 |
|---|---|
| `cargo fmt -p axagent-harness` 后 `-- --check` | **EXIT=0**（此前 `fmtcheck-harness2-0918.log` 因 `r#"` 语法错 EXIT=1，已随该修正一并转绿） |
| fmt **只动了自己的文件** | fmt 前后对 `harness/src/*.rs` 逐个 SHA256 对拍 ⇒ **`FMT_CHANGED_COUNT=1`**，且唯一变化是 `louvain_dtos.rs`（我的文件）；`graph_dtos.rs`/`page_type.rs` 等**逐位未动** |
| 子集 12 轮 + 30 轮 | **42/42 全绿**（`fix-flake-0918.log` / `fix-flake-extended-0918.log`） |
| **全量 `--lib`** | **`717 passed; 1 failed`** ⇒ 失败集合从 `{domain_router, graph_dtos}` **收敛为 `{graph_dtos}`**（既有红不变，`FULL_EXIT=101`） |
| 可回滚 | `output/backup-2026-09-18-flake/domain_router.rs.bak`（改前 SHA256 `18B23E44…`，与源文件同哈希） |

⚠ **判据诚实性**：42 轮全绿**本身不作为充分证据** —— 原概率 12.5%，42 轮全绿的偶然概率仍有 `0.875^42 ≈ 0.4%`（弱）。充分的是**结构论证**：读者现在持有的正是写者持有的那**同一个静态 `tokio::sync::Mutex`**，而写者的全部改写都在锁内 ⇒ 交叠在构造上不可能。统计只作辅助。

### 25.3 一个观测手段污染证据的实例（记进环境纪律）

复现日志里那条中文 panic 变成 `杈撳叆銆屽垎鏋愯偂绁?00519` —— PowerShell 5.1 会把 cargo 的 **UTF-8 字节流按 ANSI 解码**。判据（`left:` / `right:` / `EXIT=`）全是 ASCII 故未受影响；但凡**断言文本本身承载信息**（如中文校验名）的场景，落盘日志必须改用 `-Encoding utf8` 或直接重定向 `> file 2>&1`，不要走 `Select-String | Out-File`。

### 25.4 归属二：`graph_dtos` 既有红 —— 为什么只登记不改

| 项 | 内容 |
|---|---|
| 失败断言 | `graph_dtos.rs:1144` 对照组：`!s.contains(r#""unresolvedTypes""#)` |
| 实际输出 | 写入了 `"unresolvedTypes":[{"rawType":"wiki_page",…}]` |
| **根因** | fixture 用 `node("a","wiki_page")`，而 `page_type.rs:400-405` 有一条**既有测试**明确断言 `"wiki_page"` **不在词汇表内**（`parse_strict → None`，与 `""`/`"invalid"`/`"entities"` 并列）；合法值是 `doc/daily/knowledge/knowledge_document/synced/log/note`（`page_type.rs:422`）|
| 为什么矛盾 | 拿一个「**刻意未登记**的类型」去构造「预期**无**未识别类型」的对照组 ⇒ fixture 自相矛盾，断言必红 |
| 为什么不擅自修 | 修法二选一：① 换 fixture 类型为词汇表成员（纯测试修复）；② 把 `wiki_page` 升格进词汇表（**产品语义** —— 而 `page_type.rs:400` 的注释恰恰说明它是**被刻意排除**的）。选②越界；选①虽属纯 bug，但该文件 `mtime = 2026-09-17 17:23:49Z` **与本轮我的动作序列（18:08Z 起才有落盘产物、更早的日志全在 10:00Z 之前）对不上**，有并发写入嫌疑 ⇒ 按「报告不擅改 + 并发文件不代改」**只登记 + 单列告知** |

### 25.5 顺带取证：CI 的 PG 覆盖缺口（带行号）

| 项 | 实测 |
|---|---|
| CI 注释自陈 | `.github/workflows/ci.yml:302` 「仓库里有 **4** 个 env-gated 的 PG 集成测试」 |
| CI 实际点名 | `ci.yml:419` `cargo test -p axagent-dao --test pg_cjk_fts` + `:421` `cargo test -p axagent-search --test pg_integration` ⇒ **2 个** |
| 全仓实际 | **3 个**：`dao/tests/pg_cjk_fts.rs` · `dao/tests/pg_upsert_bare_column.rs` · `search/tests/pg_integration.rs`（注释里的 `pg_migrations` / `pg_runtime_test` 已随 §十一·十一⑨ 删除） |
| **缺口** | **`pg_upsert_bare_column` 在任何地方都没有被执行过** —— 而它正是 §23 `entity_communities` **落库往返的唯一观测点**（`dao/tests/pg_upsert_bare_column.rs:129` 刻意给非 `None` 值）。即：我 ② 的后端字段在真 PG 上的往返**至今零执行** |

⇒ 补法有二：CI 加一行（治标，且该测试只动 `wiki_graph_cache`、与既有两步无库级冲突），或把「跳过」升级为失败（治本）。**需裁决，本轮只登记。**

### 25.6 复现资产

`output/verify-graph-2026-09-15/`：`repro-domain-flake-0918.log`（单跑绿 / 8 轮 1 红）· `fix-flake-0918.log`（含 fmt 前后 SHA256 对拍 + 12 轮）· `fix-flake-extended-0918.log`（30 轮 + 全量 717/1）· `mtimes-harness-src-0918.txt`（mtime 归属）· `b-harness-test-0918.log`（更早一轮的同一条红，用于证明「非本轮引入」）。回滚点：`output/backup-2026-09-18-flake/`。

---

## §26 新登记项处置（2026-09-18 续）

> 承接 §25 末尾那批「已登记、未处置」。本轮把其中 4 项收口，另有 2 项**推翻了我上轮自己的登记**（都留痕）。

### 26.0 三句话

1. **项 A（CI 漏点名）已修**：`pg_upsert_bare_column` 补进 CI 点名清单，并给它单独加了「**SKIP 即失败**」守卫 —— 因为它的跳过是 `return` 而非 panic，harness 照样报 `1 passed` ⇒ 只有判输出文本才拦得住。
2. **项 B 我上轮登记错了**：`dao` 侧不是「未处置的同族缺陷」—— 它有**自己的** `TEST_LOCK`，且 6/6 用例全部走统一入口 `case()`（含前置清空）。**登记更正为「已合格、无需处置」**。
3. **项 D 的两处描述要更正**：红的**位置**不是 `src/stores/**` 而是 `src/types/office.ts:156`；但「属于 `--quick`」**是对的**（我一度以为它错，随即被代码推翻 —— 见 §26.5.3）。

### 26.1 项 A：把 `pg_upsert_bare_column` 接进 CI（治标 + 治本一起做）

§25.5 写「补法有二…**需裁决**」。本轮两条**都**做了，因为它们是互补而非替代：

| # | 动作 | 为什么不是二选一 |
|---|---|---|
| 1 | `ci.yml` 的 PG 步骤补第三个 target | 「点名」解决**没被调度**；不解决「被调度了但自己跳过」 |
| 2 | 该 target 后加「SKIP 即失败」守卫 | 「守卫」解决**静默跳过**；不解决「压根没被调度」 |

**顺序是硬依赖**（不是审美）：该测试**不自己建表**，它断言的 `wiki_graph_cache` 由**前一个** target 的 `pg_cjk_fts` 第 1 步 `reconcile::apply::bootstrap_schema` 建出（`pg_cjk_fts.rs:111`）。⇒ 排在前面**必然 SKIP**。

**取证陷阱（我踩过）**：在 `crates/dao/src/reconcile/` 里 grep `wiki_graph_cache` 是**零命中**，差点据此判定「引擎不建这张表 ⇒ 加了也永远 SKIP」。实际表集合由 `dao/build.rs` 在**编译期**读 `entities/src/lib.rs` 的 `pub mod` 生成（`reconcile/expected.rs:67-69` 的 `include!`），而 `entities/src/lib.rs:228` 正是 `pub mod wiki_graph_cache;`；它也不在 `extras::NON_MAIN_DB` 的排除名单里（那份只有 `ast_*` / `file_index` / `l2_*` 六张侧车库表）。
⇒ **判据是「注册表里有没有」，不是「reconcile/ 里有没有字面量」**。又一例「0 命中 ≠ 没问题」。

### 26.2 项 A-续：真 PG 首跑通过 —— ② 的落库往返**首次被执行**

| 判据 | 读数 |
|---|---|
| 命令 | `AXAGENT_TEST_PG_URL=<由 scripts/pg-connect.mjs 解密> cargo test -p axagent-dao --test pg_upsert_bare_column -- --test-threads=1 --nocapture` |
| 结果 | `test upsert_set_must_not_use_bare_column_reference ... ok` · `test result: ok. 1 passed; 0 failed` · **`TEST_EXIT=0`** |
| 编译 | `Finished \`test\` profile in 2m 20s` |
| **是否跳过** | **否** —— 输出里无 `SKIP:` ⇒ 真连库、真跑了 `DO UPDATE SET` 冲突分支 |
| 意味着 | §23 新增的 `LouvainResult::entity_communities` **真的走了一趟「序列化 → 落库 → 读回」**（该测试的 `sample_louvain()` 刻意给它非 `None` 值）。这是该字段在真 PG 上的**第一次**往返 |
| 凭据纪律 | 连接串由 `scripts/pg-connect.mjs url` 解密后**只进环境变量、不进 argv**（避免进程表泄漏），日志里只记 `URL_CAPTURED=True LEN=57`，**值不落盘** |

### 26.3 项 B 更正：`dao` 侧**本来就有锁**，我上轮登记有误

| 项 | 上轮登记 | 实测（本轮逐行核） |
|---|---|---|
| `dao/src/repo/capability_domain_override.rs` 的覆盖层读者 | 「读同一份全局，但属另一个测试二进制，**无法复用 harness 的** `#[cfg(test)] pub(crate)` 锁」——语气像待处置缺陷 | **它有自己的 `TEST_LOCK`**（`:215` `tokio::sync::Mutex::const_new(())`）+ 统一入口 `case()`（`:218-222`，含 `clear_domain_overrides()` 前置清空）；**6/6** 个 `#[tokio::test]` 全部 `let _g = case().await`（`:227/:239/:292/:320/:342/:369`）|
| 结论 | —— | **已合格，无需处置**。而且它的「统一入口 + 前置清空」比 harness 侧的形态更整齐，是**同族的正向样本** |

⇒ **我把「跨 crate 无法复用锁」这个事实，误当成了「缺陷未处置」**。事实是它根本不需要复用 —— 不同测试二进制不共享进程级 static，各持一把自己的即可。**登记口径的错**（把「机制差异」写成「待办」）。

### 26.4 项 C：`f9b2b050` 退役后的文档同步（**零行号漂移**方案）

`f9b2b050` 已按 §21 退役，但它在 Rust 侧还有 4 处字面量。逐处判定后**只改 2 处、且不增减任何行**：

| 位置 | 类型 | 处置 | 理由 |
|---|---|---|---|
| `dao/src/repo/knowledge_graph.rs:1255` | 注释（事故**反例**：「那样 f9b2b050 的 38,171 条边一条都不悬空」） | **行内**加「（已于 2026-09-18 退役，见 `merge_entities_by_name` 的文档）」 | 它讲的是**道理**，道理仍成立；缺的只是一个状态标注，否则读者会去库里找这个已不存在的库 |
| 同上 `:1416` | `///` 表格标题行（「后果（2026-09-17 实测，生产 PG）：」） | **行内**改成含「下表是**退役前**值 + 退役的三个动作（1 行 `merged_into` 改指 lemonhu 同指纹存活行；24,271 文档 / 24,274 实体 / 6 关系在对拍确认后删除）」 | 同上。**修法刻意写成单行** —— 见下 |
| `dao/src/reconcile/extras.rs:1511-1512` | **测试夹具**入参+期望（`vec_meta_index_name("vec_kb_f9b2b050_…_meta")` → `"idx_vec_kb_f9b2b050_…_meta_tsv"`） | **不改** | 它锁的是**命名规则**（`vec_X_meta` → `idx_vec_X_meta_tsv`），与库是否存在**无关**：输入串是任意串，换成 `foo` 也照样测同一件事。改了反而制造无意义的行号漂移 |

**为什么刻意用「单行行内」而不是「加几行说明」**：我先写了 5 行的版本，立刻发现它会让该文件 `+4` 行 ⇒ 连带 7 处引用需校正（`AUDIT:1014` 的 `:1409-1430`、`:1046`/`:1115` 的 `:1418`、`PLAN-memory-kb-reflow:627` 的 `:1736:9`、`harness/src/knowledge_graph.rs:733/749` 的 `:1794`、`dao/examples/wiki_edge_relation_probe.rs:126` 的 `:1794`），其中 2 处还在 harness crate ⇒ 要付一次重编译。改成单行后**净变化 0 行**，上述 7 处**全部仍然有效**（已用内容锚定验证：`:1418` 仍是表格首行、`TOTAL_LINES 2224` 未变）。
⇒ 一般化的判据：**「加行做说明」的成本不是那几行，而是它牵动的全部行号引用**；当说明可以压缩进一行时，优先单行。

### 26.5 项 D：`check-id-validation` 的红 —— 取证 + **两处描述更正** + 不代改

| 项 | 上轮登记 | 实测 | 判定 |
|---|---|---|---|
| 红点位置 | 「命中全在 `src/stores/**`」 | **1 个 ERROR：`src/types/office.ts:156`**（`conversationKey` 的 `` `${fleetId}::${conversationId}` ``）；38 个 WARN 分布在 `components/**` / `lib/**` / `sdk/**` | **上轮位置描述错误**，更正 |
| 归属 | 「`ci:check --quick` 的红」 | ✔ **正确** —— `ci-check.mjs:191` 那条 step **不在任何 `!quick` 条件里**（只有 ESLint`:219`/Vitest`:225`/Rust`:231` 受 `!quick` 影响） | 保留 |
| 新旧 | —— | `src/types/office.ts` mtime = **2026/9/15 10:33:07** ⇒ 自 9/15 起存在 | **既有红，非本轮引入** |

**26.5.3 我一度推翻自己又翻回来（留痕）**：读到 `ci-check.mjs:9` 的注释写「`--quick` 快速检查 (dprint + rustfmt + tsc)」时，我判定上轮「属于 `--quick`」这句**错了**，并已在对话里宣布「更正」。随后读 `:104-231` 的实际结构才发现：那段注释**只列举了 quick 保留的重点**，`check-id-validation` 的 step 并不在条件块内 ⇒ **上轮的表述原本就是对的，我的「更正」才是错的**。同日第二例：注释与代码不符时，**以代码为准**（本仓 `ci-check.mjs:9` 与 `:190` 两处注释都已过时：`:190` 还写着「38 条 WARN，**退出 0**」，而现在有 1 个 ERROR ⇒ 实测 `EXIT=1`）。

**为什么不代改**（`conversationKey` 的修法是「用 `safeJoinIds` 替代模板字符串」—— 看似机械，实则含语义决策）：

| 事实 | 影响 |
|---|---|
| `safeJoinIds` 是 `.filter(isValidId).join(sep)` ⇒ **跳过**无效段，而非「整体返回空串」 | 键的**段数**从「恒 2」变成「可能 1」：`conversationKey(f, "")` 由 `"f::"` 变 `"f"` |
| 而 `safeJoinIds` 自己的文档**自相矛盾**：`:86` 写「如果任一参数无效，**返回空字符串**」，`:90` 写「无效部分会被**跳过**」 | 改之前得先裁定它到底该是哪种语义 ⇒ 这是**契约决策**，不是 bug 修复 |
| 消费面已穷举：`DirectMessagePanel:53` / `ChatPanel:40` / `officeStore:92` 全是**不透明 Map 键**；`src` 内 5 处 `split("::")` **没有一处**解析 `conversationKey` | 短期无害，但 `officeStore:32` 的注释把「`fleetId::conversationId`」写成了键的**形态契约** ⇒ 改实现必须同步改契约描述 |
| 类型签名已是 `fleetId: string, conversationId: string`（非可选） | 在**类型契约**意义上该 ERROR 接近误报（检查器是纯文本匹配，不看类型）；但 `DirectMessagePanel:49` 确实会传 `""`（falsy，被 `:52` 的短路挡掉） |

⇒ 按「报告不擅改：需产品语义决策的缺陷只登记 + 单列告知」**只登记**。

### 26.6 复现资产（本轮新增）

`output/verify-graph-2026-09-15/`：`pg-upsert-run-0918.log`（真 PG 首跑，`1 passed` / `TEST_EXIT=0`）· `skip-guard-selftest-0918.mjs` + `.log`（SKIP 守卫判据的正负对照，9 条用例）· `id-validation-0918.log`（ID 边界检查全量输出）· `mtimes-idcheck-0918.txt`（既有红的归属）· `fmt-check-dao-0918.log`（`FMT_CHECK_EXIT=0`）· `facts-after-f9b2b050-0918.log`（`LOCATED 398 · NONLOC 0 · 空行 0`）· `proctable-0918c/d.txt`（改 Rust 前的进程表，两次均为 `COUNT=0`）。

### 26.7 附：一个门禁**漏报**的实证（`evidence` 指向空行，门禁却报 0）

这是本轮顺带发现的、与上面 4 项并列的**第 5 个新登记项**。它不属「未处置的旧登记」，而是**新缺陷**。

| 项 | 事实 |
|---|---|
| 引用 | `crates/harness/src/knowledge_graph.rs:749` 的 `evidence: "dao/src/repo/knowledge_graph.rs:1794"`（另一处是 `:733` 的 rustdoc 同串） |
| 目标行 | `dao/src/repo/knowledge_graph.rs:1794` = **空行**（逐行读过；与 §26.4 的改动无关 —— 我只动过 `:1255` / `:1416`，`:1794` 从未被我的编辑触及） |
| 门禁规则 | `check-single-source-facts.mjs:1055-1058`：`cited === "" && !blankOk` ⇒ 报「引用指向空行」**客观错**。而单点引用（无 `-`）恒为 `blankOk: false`（`:247`）；只有区间**终点**才 `blankOk: true`（`:257`） |
| 门禁实测 | `LOCATED 398 · NONLOC 0 · BROKEN/越界/空行/控制流 0` · `✔ 无新增客观错` · `EXIT=0`（**跑了两次**，改前改后各一次） |
| **三者矛盾** | ① 引用**进扫描面**（已证，见下） ② 该行**是空行**（已证） ③ 门禁报 **0** ⇒ **漏报** |

**怎么把「未覆盖」这个善意解释排除掉的**：用门禁**自己的** `extractRefs`（`check-single-source-facts.mjs:209` 有 `export`）喂**原样**串，而不是我手抄一份正则 —— 手抄会造出第二份可能漂移的同义实现，「我抄的能匹配」证明不了「门禁会抽到它」：

```
--- evidence 字段（harness/src/knowledge_graph.rs:749 的原样）
    输入: evidence: "dao/src/repo/knowledge_graph.rs:1794",
    抽出 1 条:
      file=dao/src/repo/knowledge_graph.rs  line=1794  blankOk=false
```

⇒ **它抽得到**（`blankOk=false` 也正确）⇒ 门禁看得见这条引用，问题出在**定位或判定**环节，不是覆盖缺口。
探针：`output/verify-graph-2026-09-15/probe-extractrefs-0918.mjs`。它靠 **stub `process.exit`** 绕开「该模块顶层无条件跑主流程并退出」这一点（零文件改动 ⇒ 无 mtime 变化、不触发重编译）。

**剩余疑点（本轮到此为止，登记给下一轮）**：判定段（`:1046-1058`）是 `const target = loc.abs; const tl = linesOf(target); … cited = tl[r.line-1].trim()`。既然 `line=1794 ≤ tl.length`，且该行确实是空行，那唯一能让 `cited !== ""` 的解释是 **`loc.abs` 不是 `dao/src/repo/knowledge_graph.rs`** —— 即 `locate()` 的 `resolveRef`（`:635`，含 `/` 时优先）或 basename 消歧（`:638-646`）把它**指到了别的同名文件**（`knowledge_graph.rs` 全仓有 ≥3 份：`dao/src/repo/`、`harness/src/`、`src/commands/`）。**验证它需要拿到 `locate()` 的返回值（`resolveRef`/basename 索引均未 export）**，本轮不做 —— 避免无限递归；机制一句「待查定位结果」即可交付给修的人。

**为什么这个登记重要**：它是「审计脚本自身会撒谎 / **0 命中 ≠ 没问题**」（判据 #7）的又一例，且这次能**排除**「它没覆盖到」这个善意解释 ⇒ 这一条上的绿是**假绿**。若把「空行 0」当作「引用面干净」的证据，就会**借假绿给别处背书**。
> **⚠ 2026-09-18 更正（详见 §27.1）：本节上述结论已被推翻。**
> 
> `probe-evidence-1794-0918.mjs` 跑完整判定链后的实测：该 `evidence`（`harness/src/knowledge_graph.rs:749`）
> → `resolveRef` 第三基准命中 ⇒ `LOCATED`（目标 2,224 行、行号未越界）⇒ 引用的那一行**非空**，且正是
> `Some(rt) => GraphEdge::relation(rel.source_entity_id, rel.target_entity_id, rt),` —— **reference 边的主构造点**。
> 因此 `classifyDocRef ⇒ null` 是**正确判定**，「空行 0」**不是假绿**。
> 
> 上一轮错在：链条第 1 环（`extractRefs` 抽得到）之后的推断 —— 尤其「1794 是空行」—— **从未被独立断言过**。
> 紧随其后的「第二重问题」同样不成立：`:1805` 是 `GraphEdge::structural(…, "reference")` 的**兜底**分支，
> 不是 `reference` 的主构造点。


**同串的第二重问题（不代改）**：这条 `evidence` 想说「知识库实体关系 `reference`」，而 `reference` 的真实位置是 `:1787`（注释「渲染类别仍是 `reference`」）/ `:1805`（`GraphEdge::structural(…, "reference")`）—— 指向空行属**修法唯一**的客观错。不代改的理由：它落在 `check-decl-evidence.mjs` 的面上（那条链另有 `--ci` 硬/软分流），且要与 §26.4 的行号纪律**一起**处置，不宜单点插队。

**复现资产（本节）**：`probe-extractrefs-0918.mjs`（同源函数探针）。相关读数：`facts-after-f9b2b050-0918.log`（第一次）、`facts-after-entity-communities-0918.log`（更早一次，同为 `空行 0`）。
---

## §27 未处置项续处置（2026-09-18 · 第二轮）

> 承接 §26.6「诚实的边界（未处置）」与新登记项。本轮处置三块：① **撤销 §26 的「门禁漏报」结论**；② 活文档（`.md`）行号引用的规模量测 + **形态分类** + 首批订正；③ 三项 DB 登记的**只读取证**（一项未复现、一项比登记严重）。

### 27.0 结论表

| 块 | 结论 | 关键读数 |
|---|---|---|
| ① 项 E（门禁漏报） | **撤销 —— 是我的假结论** | 门禁判定**正确**；`classifyDocRef` 经 **6/6** 正负对照证明真会告警 |
| ② 活文档行号引用 | 规模量测 + 首批订正 | A 栏 **1,810** 条引用 ⇒ 粗筛可疑 **112 → 108**（Δ=−4，恰等于订正数，**无新增**）|
| ③ DB 单位不一致 | **确证，且比登记严重** | 不是「跨库」而是**同一列内混用**：`s(10位)` **31** 列 / `ms(13位)` **24** 列 |
| ③ DB `updated_at`=1970 | **未复现** | `axinvest` 库落在 1970-01-01 的行 = **0** |
| ③ DB 同名多 id | **确证恰好 2 组** | 「金宇」「太阳能」；且暴露 **id 双形态** |

### 27.1 撤销 §26 的「假绿」结论（我上一轮判错了）

**事实**：`harness/src/knowledge_graph.rs:749` 的 `evidence: "dao/src/repo/knowledge_graph.rs:1794"` 与同文件 `:733` 的 rustdoc，两条都 → `LOCATED`（`resolveRef` 的第三基准 `src-tauri/crates` 命中，**唯一**命中）→ 目标文件共 2,224 行、行号未越界 → 该行实测为

```
Some(rt) => GraphEdge::relation(rel.source_entity_id, rel.target_entity_id, rt),
```

⇒ `classifyDocRef ⇒ null`（**正确判定**：非空、非控制流、非收尾行）。

**我错在哪（两层）**：

1. **上一层错在证据链只走了一环**：上一轮只证明了链条第一环「`extractRefs` 抽得到它 ⇒ 进扫描面」，剩下两环（① 该行是空行 ② 所以该报「空行」）**全是推断**。而「1794 是空行」这个前提**来自一次人工读文件的印象，从未被任何断言独立验证**——上一轮的探针头部注释里甚至把它当既定事实写死了（「该行实测是空行」）。
2. **同串的第二重问题也不成立**：上一轮另写「`reference` 的真实位置是 `:1787` / `:1805`」。实测 `:1794` **就是** `GraphEdge::relation(...)` ⇒ reference 边的主构造点；`:1805` 是 `GraphEdge::structural(…, "reference")` 的**兜底**分支。⇒ 该 `evidence` **当前是准确的**，无需订正。

**本节真正该记的是「判据的判据」这一步**（上一轮缺了它，本轮补上）：单看「空行 0」这个读数，无法区分「引用面干净」与「判据已死」。所以先跑正负对照：

| 输入 | 期望 | 实测 |
|---|---|---|
| 空行 + 单点引用（`blankOk=false`）| 空行 | **空行** ✔ |
| 空行 + 区间终点（`blankOk=true`）| 通过 | 通过 ✔ |
| 越界（`cited=undefined`）| 越界 | **越界** ✔ |
| 裸控制流 `continue;` | 控制流行 | **控制流行** ✔ |
| 收尾行 `}` | 收尾行 | **收尾行** ✔ |
| 普通代码行 `let a = 1;` | 通过 | 通过 ✔ |

6/6 符合 ⇒ 判据活着 ⇒ 「空行 0」这一次是**可采信的读数**。

**顺带排除掉一个我自己造的假发现**：本节探针首版还断言「`blocks.ids[srcLine]` 有 off-by-one」——**纯属我传错参数**：`srcLine` 由 `text.split(/\r?\n/).entries()` 产出 ⇒ **0-based**（脚本里 5 处打印才 `+1`，见 `:1243/:1346/:1383/:1387/:1475`），而 `commentBlocks.ids` 同样 0-based ⇒ 两者口径**一致**。我第一版把 1-based 行号直传进去，「查到了下一行的块」，再把账记在被测对象头上。探针里已加**口径自查**（「该行是不是注释行」必须与「`ids >= 0`」一致，2/2 通过）来永久钉住这一点。

### 27.2 活文档行号引用：规模、形态分类、首批订正

**规模**（`scripts/check-single-source-facts.mjs --dump-doc-refs`，A 栏 = `docs/plans/**` + 根级 `*.md`）：

| 读数 | 值 |
|---|---|
| 文件数 | 38 份 `.md` |
| 引用总数 | **1,810**（去重后 1,572）|
| 第三方源码（免检）| 39 |
| 可定位 | 1,168 |
| **无法定位（= 未检查）** | **365** |
| **粗筛可疑** | **112** |

> 注意 `365` 这个数：它是「未检查」，在输出里与「检查且通过」长得一样（判据 #7）。其中 135 条是 BROKEN、其余是「同名 N 份且路径后缀不能消歧」（`mod.rs`/`lib.rs` 一类）。

**三类形态（本轮的判据，脚本自己声明「分不出」，所以必须人判）**：

| 形态 | 特征 | 修法 |
|---|---|---|
| **① 叙述型** | 正文/草稿里**声称现状**（「关键事实：X（`f:line`）只过滤…」）| **订正**行号 |
| **② 核查快照表** | 表格形态「\| 位置 \| 当时的结论 \|」，结论是**某次核查时**下的 | **不订正行号** —— 只改行号会让「当时的结论」冒充「基于当前代码的结论」，是制造新错配，不是修复 ⇒ 需裁决（订正 vs 加日期戳）|
| **③ 论断依据失效型** | 引用对象**不存在**，或论断方向**与代码相反** | **绝不订正** —— 改行号等于给一个错误论断挂上看似有效的出处，**比不改更糟** ⇒ 只登记 |

**本批订正 4 条**（全部落在 `docs/plans/PLAN-cognitive-orchestration-fixes.md`，均属①）：

| 文档位置 | 原值 | 新值 | 依据（改动后自证）|
|---|---|---|---|
| `:31` | `crates/tools/src/registry.rs:815` | **`:918`** | `pub fn get_chat_tools_by_names` |
| `:191` | `cognitive_router.rs:842` | **`:857`** | `fn clamp_mode_for_kind` |
| `:445` | `cognitive.rs:758-777` | **`:1008-1032`** | `:1008` `let execution = match forced_kind`；`:1032` `_ => {` |
| `:634` | `cognitive.rs:799` | **`:1051-1052`** | `:1051` `forced_tools, forced_skills`；`:1052` `resolve_exposure_injection`（紧接 `:1053` 的 `AgentQueryRequest {`）|

**③ 型的本批实例（登记不代改）**：文档 `:23` 写「同一代码库里 `agent_def_types.rs:156` `is_tool_allowed` 的顺序是对的（**先判 disallowed 再判白名单**）」。实测：该文件全仓**唯一**，只有 **120 行**（156 必然越界），且内容是**纯类型定义**（`AgentDefinition` / `AgentDefSource` / `IsolationMode` / `MemoryScope` / `AgentMcpServerSpec`）—— **根本没有 `is_tool_allowed`**；全仓唯一实现在 `harness/src/delegation.rs:68`，而它的顺序是**先白名单、再黑名单**（`:70` 判 `allowed_tools` → `:75` 判 `blocked_tools`），**与论断相反**。⇒ 这条**不能靠改行号修**。

**验证（判据的判据）**：`verify-docrefs-delta-0918.mjs` 现场重跑 dump 并逐条对比：

| 断言 | 结果 |
|---|---|
| 无新增可疑条目 | ✔（新增 = ∅）|
| 消失集合 == 我改的那 4 条 | ✔（4/4，且 `gone.length === 4`）|
| 逐文件可疑数：仅目标文档 −4，其余**一位不动** | ✔（意外变化 0 处）|
| 目标文档可疑数 | 13 → **9**（−4）✔ |
| A 栏汇总 | 1,809 → 1,810 条引用（区间写法多出的 1 条）；1,167 → 1,168 可定位；**112 → 108** 可疑 |

### 27.3 DB 三项只读取证（`axinvest` 库，`public` schema）

脚本 `probe-db-3items-0918.mjs`（**只读**：全部 `SELECT` / `information_schema`；连接串从 `pg-connect.mjs url` 取得后**只进环境变量**，不落盘、不进 `argv`）。

**① 单位不一致 —— 确证，且比登记严重得多**

全库 227 个 `*_at` 列（151 张表）：`bigint` **198** / `text` **28** / `timestamptz` **1**。有数据的列里：

| 单位 | 列数 |
|---|---|
| `s`（10 位 epoch 秒）| **31** |
| `ms`（13 位 epoch 毫秒）| **24** |
| 空表（`NULL`）| 143 |

**关键升级**：登记写的是「跨库 `created_at` 单位不一致」，实测是**同一列内混用**——`min` 与 `max` 落在不同量级：

| 表.列 | min | max | 说明 |
|---|---|---|---|
| `agent_profiles.created_at` | `1786769161`（s）| `1789668382341`（ms）| 同列混用 |
| `knowledge_entities.updated_at` | `1789421929`（s）| `1784897564447`（ms）| 同列混用，且 **max < min** |
| `knowledge_relations.created_at` | `1785473031`（s）| `1786277831750`（ms）| 同列混用 |
| `knowledge_relations.updated_at` | `1787611005`（s）| `1789661287419`（ms）| 同列混用 |
| `workflow_templates.created_at` | `1785972207`（s）| `1789475474927`（ms）| 同列混用 |
| `workflow_templates.updated_at` | `1789354672`（s）| `1789667852488`（ms）| 同列混用 |
| `knowledge_entities.created_at` | `1784897564447`（ms）| `1786277831750`（ms）| 一致（对照）|

⇒ 后果**不是**「显示差 1000 倍」这么轻：同一列内两种单位并存 ⇒ **`ORDER BY` 与任何范围查询（`WHERE updated_at > ?`）都失效**，且 `max()` 会返回一个**比 `min()` 更早**的值（见表中第 2 行）。这是「看起来正常的坏数据」，比缺列更难发现。

**② `updated_at = 1970` —— 未复现**

判据（先说清口径，免得把「读到 0 行」当「没问题」）：数值型取 `0 ≤ v < 86400`（秒级）或 `0 ≤ v < 86_400_000`（毫秒级）；时间型取 `date_trunc('day', v) = 1970-01-01`。**命中 0 行**。

⇒ 该登记**本轮无法复现**（不是「已修复」，是**没有观测到**）。可能的来源：前端把 `0` / 极小值格式化成 `1970-01-01` 的**显示**，或来自 `.worktrees/` 下的另一棵树。**保留登记，但注明「不得再当作已知事实引用」**。

**③ 同名多 id —— 确证恰好 2 组，且暴露 id 双形态**

`knowledge_entities` 列：`id, knowledge_base_id, name, entity_type, description, source_path, source_language, properties, lifecycle, behaviors, metadata, created_at, updated_at, aliases, mention_count, confidence, first_seen_at, last_seen_at, source_type, source_id, node_type, external_id`。

查询（限 `lemonhu_knowledge_graph`、排除 `lifecycle->>'merged_into'` 非空的已合并行）：

| 名称 | 行数 | 不同 id 数 | ids |
|---|---|---|---|
| 金宇 | 2 | 2 | `803` ／ `dac59c4c8e5c17eb6e42eb783dd53a7e` |
| 太阳能 | 2 | 2 | `2cc60d3b30e5ff15cc795f0b922c2bb2` ／ `591` |

**总数恰 2 组**，与登记一致。**新发现**：2 组里 **id 形态都是「纯数字短 id ↔ 32 位 hex」两种**⇒ 这不是 2 个巧合，而是 **id 生成方案分叉**的可见症状（同族：`kb-dup-import-attribution` 技能里「实体的 id 有好几种写法」）。

### 27.4 我本轮的错误（4 个，全部当场清掉并留痕）

| # | 错误 | 性质 | 处理 |
|---|---|---|---|
| 1 | 上一轮的「门禁漏报」结论 | **假结论**（前提未验证）| 本节撤销 + §26 原位加更正标记 |
| 2 | 探针首版断言 `bid < 0` 才安全 | 口径错 —— 把「不进判定」当成「不被吞掉」的**必要条件** | 改为 `!externalMarkFor(...)`，并把两次口径错都写进注释 |
| 3 | 断言「`blocks.ids[srcLine]` 有 off-by-one」| **假发现**（我传错参数）| 加**口径自查**对拍，2/2 通过 |
| 4 | `EXPECT_GONE` 手写四条时漏 `docs/plans/` 前缀 ⇒ 断言假红 | 判据与被测对象**字符串不同源** | 改用同一构造（`${P}:…`），重跑 ALL_OK |
| 5 | 本文追加脚本的「章节结构未被打断」断言阈值凭印象写成 `>= 8`（实测不足）⇒ **假红** | 阈值未经实测 | 改为按实测值下阈值 + **打印实际数量**（否则假红与真损坏在日志里同形）|

另有一条**纪律违反**（不是结论错）：本轮**同一条消息里对同一文件发了两个 `Edit`** ⇒ 后发者基于旧快照写回，**静默**覆盖了先发者（工具两次都报 success）。诊断依据是「先发的改动消失、后发的保留」。已改为串行重做 —— 注入层通用纪律 #2 早已写明「同文件 `Edit` 串行」，但那里记的信号只有 `TS6133`；本轮的信号是「**Edit 报成功但内容回退**」，已补入 DETAIL §90。

### 27.5 复现资产（本节）

`output/verify-graph-2026-09-15/`：

- `probe-evidence-1794-0918.mjs` / `.log`（完整判定链 + 判据的判据 6/6 + 口径自查 2/2）· `mtimes-kg1794-0918.txt`（mtime 排除并发写入）
- `doc-refs-0918.log`（改前 dump）· `doc-refs-0918.json`（112 条结构化）· `triage-doc-refs-0918.mjs` / `.txt`（分流）· `probe-doc-13-0918.mjs` / `.txt`（逐条取证）· `fix-doc-refs-0918.mjs` / `.log`（**10/10 前置断言 + 4/4 后置回读**）· `doc-refs-after-0918.log` · `verify-docrefs-delta-0918.mjs` / `.log` / `delta-ascii-0918.txt`
- `probe-db-3items-0918.mjs` / `probe-db-3items-0918.txt`（三项只读取证全文）
- 回滚点：`output/backup-2026-09-18-docrefs/`（含改前 SHA256 与行数）

**未处置（继续登记）**：`docs/plans/**` 其余 **108** 条可疑（10 份文档）；B 栏（`.md`）里 365 条「未检查」的 BROKEN/同名不可消歧；`graph_dtos` 既有红；`src/types/office.ts:156` 的 ID 边界红；DB 单位混用（**影响面已量化，但修法涉及数据迁移，需裁决**）。

## §28 未处置项续处置（2026-09-18 · 第三轮）：项 C 复检到根因 + 记忆层收口

### 28.0 结论表

| 项 | 上一轮读数 | 本轮处置 | 判定 |
|---|---|---|---|
| C `updated_at` = 1970 | 0 行 | **扩口径重跑 + 判据的判据**：198 张表（`public` + `ax_backup`）/ 362 个「时间类」列 / 正负对照**全过** | **`updated_at` 确实无此问题** ⇒ 登记里的说法不成立 |
| C 真身（新） | — | `trajectory_rewards.created_at`（text/ISO）**13 行 = `1970-01-01T00:00:xx+00:00`** | **确证缺陷**，根因定位到源码（见 28.2） |
| 上游同族（新） | — | `trajectory_steps.timestamp_ms` **同一列混用绝对 epoch-ms 与相对偏移-ms**（11 行 < 1e9） | **确证**（项 B 的同族，量纲是 abs/rel 而非 s/ms） |
| 我的假发现 | — | `trajectory_workflow_reflections.{created_at,timestamp}` 的 25 / 58 行「含 1970」 | **口径噪声**：`^1970-` 命中 **0**，`1970` 只是 9 位纳秒小数秒里的子串 |
| 记忆层 | 6 条判据 + DETAIL §91 已写入 | 归位 2 条 + 更正 `mem.mjs` 腐烂注释 | 注入层**已饱和**（7161 / 7168 B）⇒ 见 28.4 |

### 28.1 项 C 真身：机制（逐位吻合，可复现）

`trajectory_rewards.created_at` 存的是 **`trajectory_steps.timestamp_ms ÷ 1000` 当成 epoch 秒**：

| `trajectory_steps.timestamp_ms` | ÷ 1000 | `trajectory_rewards.created_at` 实测值 |
|---|---|---|
| 23089 | 23.089 | `1970-01-01T00:00:23.089+00:00` ✔ |
| 27351 | 27.351 | `1970-01-01T00:00:27.351+00:00` ✔ |
| 27823 | 27.823 | `1970-01-01T00:00:27.823+00:00` ✔ |
| 34730 | 34.730 | `1970-01-01T00:00:34.730+00:00` ✔ |
| 43082 | 43.082 | `1970-01-01T00:00:43.082+00:00` ✔ |
| 44657 | 44.657 | `1970-01-01T00:00:44.657+00:00` ✔ |
| 56867 | 56.867 | `1970-01-01T00:00:56.867+00:00` **×3** ✔ |
| 65230 | 65.230 | `1970-01-01T00:01:05.230+00:00` ✔ |

同列正常值对照：`2026-09-06T08:49:50.755+00:00`。
`trajectory_rewards` 共 **63** 行，其中 **13** 行落在 1970；该列 `^1970-` 命中 **13** 与 `LIKE '%1970%'` 命中 **13** **相等** ⇒ 不存在「部分命中是子串噪声」的余地。

### 28.2 根因：契约两端不一致（不是数据脏）

| 位置 | 事实 |
|---|---|
| `crates/trajectory/src/causal.rs:19` | `//! - 延迟只取 step 之间的差值，不假设 timestamp_ms 的基准（相对/绝对皆可）。` |
| `crates/trajectory/src/causal.rs:524` | `/// 延迟一律取 step 之间的差值，不假设 timestamp_ms 是相对还是绝对基准。` |
| **`crates/trajectory/src/storage.rs:229`** | `created_at: Set(chrono::DateTime::from_timestamp_millis(r.timestamp_ms as i64)` —— **全仓唯一**把该字段当**绝对 epoch-ms** 用的地方 |

⇒ `timestamp_ms` 的基准被契约**明确声明为「未定义」**，各消费端一律只取**差分**；`storage.rs:229` 越界当绝对时间 ⇒ 相对偏移（如 `56867`，单位 ms）被解释成 `1970-01-01T00:00:56.867Z`。
⇒ 这是**契约两端不一致**，不是数据被写脏：修一边即可，但**修哪一边属产品语义**（见 28.3）。

**附带发现：兜底档恒不可达。** `from_timestamp_millis(v).unwrap_or_else(Utc::now)` —— `from_timestamp_millis` 对**任何** i64 都返回 `Some` ⇒ `.unwrap_or_else` 永不触发，是**装饰性保护**。
**反向印证**：DB 里落的是 `1970` 而**不是** `now` —— 兜底若真触发过，就不会出现 1970。这正是「兜底档须按声明值比对」的可判形态。

### 28.3 需裁决（不代改）

| 方向 | 内容 | 代价 |
|---|---|---|
| **(a) 尊重既有契约** | `storage.rs:229` 改为写「真实写入时刻」（`Utc::now()`），或把该列语义改名为相对偏移并同步注释 | 1 处写入 + 下游读 `created_at` 的处所 |
| (b) 收紧契约 | 要求全部生成端写绝对 epoch-ms | **大**：`process_reward.rs:430`、`pattern.rs:563`、`skill_learning.rs:1120` 等生成端都在用小值（`(i+1)*1000`），且**与 `causal.rs` 已声明的契约冲突** |

**建议 (a)**（改动面窄、不动既有契约），但属产品语义 ⇒ **登记待裁决**。

**存量数据**：`trajectory_rewards` 按 `trajectory_id` **先删后插**（`storage.rs:218-221`）⇒ 该轨迹**重存一次即自动刷新**，无需手工 `UPDATE`；`trajectory_steps` 的 11 行小值属上游真实形态，须随 (a)/(b) 一并定。

### 28.4 记忆层：注入层已饱和（承接 2026-09-18 第七节的遗留提醒）

| 量 | 读数 |
|---|---|
| 注入层 `MEMORY.md` | **7161 / 7168 B**（余量 **7 B ≈ 0 条**）|
| 判据层 / 证据层 | 297 585 B（528 条 / 16 组 / 22 节 / 最大编号 531）· 647 901 B |
| `mem.mjs` 头部注释自陈 | 「当前占用约 **75%**，余量足够再加 9 条通用纪律」⇒ **读数已腐烂**（同 K 组 #524：别拿注释当判据），已当场更正 |

本轮做的**零字节成本**修正（不动注入层）：

| 动作 | 依据 |
|---|---|
| `#530`（epoch 秒/毫秒同列混用）E 组 → **D 组** | D 组已有触发词「时间戳读了不比对」⇒ 才召回得到；留在 E 组则永不命中（= 通用纪律 #10「写进产物 ≠ 有人消费」）|
| `#529`（观测手段撒谎两式）E 组 → **A 组** | 与 A 组 `#518`（shell 管道转码）是**同一失效模式的两例**；分居两组时，读一组只能命中一例 |
| `mem.mjs` 注释更正 | 见上 |

**仍未解决（需裁决）**：新增判据的**触发词**无处可加 —— 注入层余量 7 B，而 `mem.mjs` **没有**管理触发词的命令 ⇒ 触发词只能手工 `Edit MEMORY.md`，且必须先外迁等量内容。建议二选一：

1. 外迁 `通用纪律 #6` 的五型枚举（详 `#72`/§37）只留指针，可省 ~150 B；
2. 给 `mem.mjs` 加 `trigger` 子命令（含预算校验），把触发词从「手改」变成「受检事务」。

### 28.5 本轮的错误（2 个，全部当场剔除）

| # | 错误 | 性质 | 处理 |
|---|---|---|---|
| 6 | 把 `trajectory_workflow_reflections.{created_at,timestamp}` 的 25 / 58 行「含 1970」当真 | **假发现**（判据被别的东西满足）| 加**判别性正对照**：该列 `~ '^\d{4}-\d{2}-\d{2}'` 命中 = 总行数（说明确为日期列），但 `^1970-` 命中 **0** ⇒ 噪音列剔除 |
| 7 | 宽口径首跑报 11 列「命中」，其中 **7 列是我自己正则捞的噪声**（`timeout_seconds`、`mcp_servers.*_timeout_secs`、`senti**ment**_score`、`invali**date**d`、`total_time_ms`×2 等）| 口径过宽（子串命中）| 逐列读**实际取值**复核后剔除；噪声列不计入结论 |

**教训**：判据**放得越宽，假发现越多**，而「宽口径 ⇒ 命中 N」与「真有 N 个缺陷」在计数上**同形** ⇒ 放宽口径之后**必须逐列读值**（已入 DETAIL §92.4）。

### 28.6 复现资产（本轮）

`output/verify-graph-2026-09-15/`：

- `probe-db-1970-0918.mjs` / `.txt` —— 扩口径分段计数（198 表 / 362 列）+ 正负对照（`pos == nn`、`neg == 0` 全过）
- `probe-db-1970-samples-0918.mjs` / `.txt` —— 取值形态验证（14 个真 ISO 日期列 / 3 个含 `1970` 列的去重取值 / `trajectory_steps` 11 行**同行语境**）
- 记忆层：`mem.mjs move 530 D`、`mem.mjs move 529 A`、`mem.mjs check` 读数；备份 `output/backup-memory/MEMORY-RULES.md.bak`

## §29 第四轮：T1–T4 修复落地 + DB 时间单位混用的「后果取证」（2026-09-18）

> 入口 = 用户三分裁决：① `trajectory_rewards.created_at` 修法选 **(a)**（改存储层写真实时刻）；
> ② 注入层触发词 **外迁**；③ 其余全部「登记待裁决」项 **授权修复**。

### 29.1 执行总览

| # | 项 | 结论 | 判据读数 |
|---|---|---|---|
| T1 | `trajectory_rewards.created_at` 1970 | ✅ 已修 + 已验证 | `FMT_EXIT=0` · `cargo test -p axagent-trajectory` **1 passed / TEST_EXIT=0** · `cargo clippy -p axagent-trajectory --all-targets -- -D warnings` **CLIPPY_EXIT=0 / ERROR_OR_WARN_LINES=0** |
| T2 | 注入层外迁 + 补触发词 | ✅ 已完成 | **7161 → 7086 B**（Δ−75，余量 **82 B**）· 逐行 diff 恰 6 行 · 回读一致 |
| T3 | `graph_dtos` 悬空边既有红 | ✅ 已修 + 全量验证 | `cargo test -p axagent-harness --lib` **718 passed / 0 failed / TEST_EXIT=0** |
| T4 | `src/types/office.ts:156` ID 边界红 | ✅ 已修 + 双面验证 | `check-id-validation` ERROR **1 → 0**（`IDV_EXIT=0`，WARN 38 未增未掩）· `officeStore.conversation.test.ts` **5 passed / VITEST_EXIT=0** |
| T5a | DB 时间单位混用 **形态** | ✅ 已复核（登记数**低估**） | 活表同列混用 **7 处**（登记称 6 处）；另 4 处在备份表 |
| T5b | DB 时间单位混用 **后果** | ✅ 已取证（危害**远小于预期**） | 受影响分组 **0 / 0**；`list_workflow_templates` 顺序**确实错**（2 条秒值沉底） |
| T5c | DB 数据订正 | ⏸ **未动数据**（破坏性动作，待裁决） | 方案见 §29.6.4 |
| T5d | 项A 剩余 108 条文档引用 / B栏 365 条 | ⏸ 未批量订正（理由见 §29.7） | 现状见 §29.7 |

### 29.2 T1：`trajectory_rewards.created_at` 的 1970（方案 a）

根因链三方印证，**生产端与存储端对同一字段的基准理解相反**：

| 环节 | 位置 | 事实 |
|---|---|---|
| 生产端 | `crates/agent/src/trajectory_recorder.rs:142` | `timestamp_ms: (Utc::now() - state.start_time).num_milliseconds() as u64` ⇒ **相对轨迹起点的毫秒偏移** |
| 契约端 | `crates/trajectory/src/causal.rs:19` / `:524` | 明文「**不假设** `timestamp_ms` 的基准（相对/绝对皆可）」，消费端一律只取**差分** |
| 越界端 | `crates/trajectory/src/storage.rs:229`（全仓唯一） | `from_timestamp_millis(r.timestamp_ms)` ⇒ 相对偏移 `56867`(ms) 被落成 `1970-01-01T00:00:56.867Z` |
| 回读端 | `crates/trajectory/src/storage.rs:458-465` | `parse_from_rfc3339 → timestamp_millis()` —— **与写入互为逆变换**（lossless 往返） |

**关键约束**：因为写入/回读互为逆变换，直接把 `:229` 改成 `Utc::now()` 会**打破往返**，等于换一个缺陷。

**修法**：

- 写端 `:229` ⇒ `created_at: Set(r.timestamp_ms.to_string())`（存**原样十进制毫秒**，不伪装成 1970）
- 读端 ⇒ **双向前兼容**（数字优先，RFC3339 回退），且**不用 `Utc::now()` 兜底**（改为 `tracing::warn!` + 取 `0`）；兜底分支是**恒不可达**的，写现在是刻意让它退化得可见
- 新增模块级纯函数 `parse_reward_timestamp_ms`（放在 `fn model_to_skill` 之前）
- 同文件先例：`memory_items` 时间列 = `timestamp_millis().to_string()`（`:1207`）

**存量零迁移**：毫秒精度在 RFC3339 里由**小数秒**承载 ⇒ 旧形态 `1970-01-01T00:00:56.867Z` 反解回来**逐位仍是 56867**（已用 node 自证）。且该表按 `trajectory_id` **先删后插**（`:218-221`）⇒ 重存即自动刷新。

**★ 本仓 `use sea_orm::ExprTrait` 会让 `x.max(0)` 二义**：

```rust
// ⚠ 必须写 `Ord::max` 而非 `ms.max(0)`：本文件 `use sea_orm::ExprTrait` 也在作用域里，
// 该 trait 为同型提供了 `max` ⇒ 点号写法报 `E0034 multiple applicable items in scope`
return Some(Ord::max(ms, 0) as u64);
```

**★ 由此得到一条判据**：**`cargo fmt -p X -- --check` 报 EXIT=0 不能当编译证据** —— 实测同一份代码 fmt 判绿而 `cargo test` 报 2 个 `E0034`。

**测试**：`reward_timestamp_ms_roundtrip_both_encodings`，断言 `"56867"→Some(56867)`、`"1789355135987"→Some(1789355135987)`、`"1970-01-01T00:00:56.867+00:00"→Some(56867)`、`"2026-09-06T08:49:50.755+00:00"→Some(1788684590755)`、`"not-a-timestamp"/""→None`（**强制走兜底分支**）、`"  56867 "→Some(56867)`。

### 29.3 T2：注入层外迁（字节账）

| 判据 | 期望 | 实测 |
|---|---|---|
| 字节 | ≤ 7168 且净省 | **7161 → 7086（Δ−75）· 余量 82 B** ✔ |
| 逐行 diff | 只允许 6 行变化 | **6 行 / 非预期 0** ✔ |
| 回读 | 与期望一致 | ✔ + 副本已留 |

**★ 首跑凭感觉写的触发词比外迁省下的还多 130 B（总量 7291 B）⇒ 是脚本的「写盘前断言」拒绝落盘才没顶穿。** 字节只认 `Buffer.byteLength` 实测值，且断言必须在**写盘之前**。

外迁内容 = 纯去重：第 6 条与判据 **#72** 是同一份内容（⇒ 压成指针）；第 12 条删举例句（正文在 DETAIL §75）。补 4 处触发词：A 组 `追加脚本幂等`；D 组 `时间戳基准（相对/绝对）· 秒/毫秒同列混用`；I 组 `0命中先证会告警`；K 组 `行号引用分形态 · 期望值同源构造`。

### 29.4 T3：`graph_dtos` 悬空边既有红

**红点定位**（实测，非登记）：`panicked at graph_dtos.rs:1144`，测试输出含

```json
"unresolvedTypes":[{"rawType":"wiki_page",...}]
```

⇒ `node("a","wiki_page")` 被判「未识别类型」，而 `:1144` 的断言是 `assert!(!s.contains(r#""unresolvedTypes""#))` ⇒ 直接撞红。

**哪一侧错**：`crates/harness/src/page_type.rs:403-408` **自己**把 `"wiki_page"` 写进**非法清单**并断言 `parse::<PageType>() == PageType::Unknown`；DB 实测 9 个类型列 / 198 张表 **命中 0 行**（真值只有 `doc`/`entity`/`person`/`company`/`concept`/`industry`）⇒ **是用例把 `wiki_page` 当成了「干净图」，判据是对的**。

**修法判据（本轮定下）**：

> **「用来污染的类型」不动，「用来当干净基线的类型」换成词汇表成员。**
> 同一个字面量 `wiki_page` 在文件里承担两种角色，**必须逐处判角色**，不能全量替换、也不能全量保留。

19 处（实为 18 行）分三类：

| 角色 | 位置 | 处置 |
|---|---|---|
| 污染点（兜底路径入口） | `:880` `types.push("wiki_page"); // 越界值（兜底入口）` | **不动** |
| 故意非法输入（判定用例） | `:869`（注释）`:901` `:911` `:926` `:931` `:955` `:958` | **不动** |
| 故意的污染源（需清单非空） | `:1050` `assert!(!types_before.is_empty())` 依赖它 | **不动 + 补注释**（防下一个人当干净值删掉） |
| 历史缓存 JSON（用例真实性） | `:1148` `"type":"wiki_page"` | **不动** |
| **当干净基线** | `:1097` `:1124` `:1141` `:1163` `:1192` `:1235` `:1256` `:1277` | **改**（`:1163` 改 `note` —— 语义本就是「笔记节点」；其余改 `entity`） |

**验证**（只读校验脚本，8 条判据全过）：

```
LINES_OK orig=1283 now=1285 inserted=2
WIKI_PAGE_HITS orig=21 now=12 delta=-9        ← 自洽：删掉 10 次（:1097/:1192 各含 2 次）+ 新注释 1 次
NORMALIZED_CHANGED_LINES=8
CHANGED_SET_EXACT_OK=8 / CONTENT_MATCH_OK=8 / MUST_KEEP_INTACT=8
KEEP_BY_DESIGN_OK=3
RESULT: ALL_OK
```

**★ 修我自己的两个假判据（当场更正并留痕）**：

1. 修脚本里 `countHits(lines)` 是在**已修改**的 `lines` 上算「before」⇒ `delta` **恒为 0**。真值必须用「备份文件 vs 现文件」**两个不同对象**来算（= before 21 / after 12）。
2. 修脚本报 `CHANGED_LINES=235` —— 其中 **233 行是插入 2 行造成的行位移噪声**。正确判据 = 先剔除新增注释行再做逐行比较，且变化行集合必须**恰好等于**期望集合。
3. 校验脚本 v2 又把 `EXPECT` 里多写了 `1050`（它的**内容一字未动**，只是上方插了注释）⇒ 归一化 diff 里它**不该出现**。**是我的期望值错，不是被测对象错**（判据口径问题）。
4. 校验脚本 v3 把 `now[INSERT_AT - 1]` 当成了「第 2 条注释」（其实是第 1 条）⇒ 三处索引写错。

### 29.5 T4：`src/types/office.ts` ID 边界红

`check-id-validation.mjs` 的规则是**纯正则扫文件原始文本**：`/`[^`]*\$\{[^}]+\}[^`]*::[^`]*\$\{[^}]+\}[^`]*`/g` ⇒ **连注释里写的示例也算命中**（该脚本自带 `FALSE_POSITIVE_FILES` 白名单，正因如此）。

**★ 门禁给的建议修法在这个位置是错的**：`safeJoinIds` = `filter(isValidId).join(sep)` ⇒ 缺一段时**静默**产出**一段**的键。而 `conversationKey` 是**持久化**键（`messagesByConversation`，`fleetId::conversationId`，只作对象键、**无人拆开读**）⇒ 退化的键既读不回原消息、也不抛错，**比 `undefined::xxx` 更难发现**。

`safeJoinIds` 自己的文档**自相矛盾**（`:86` 写「返回空字符串」／`:90` 写「无效部分会被跳过」），而门禁 suggestion 引的正是那条错描述 ⇒ §29.7 已更正。

**★ 我自己在注释里制造了一条新门禁违规**：第一版 `conversationKey` 注释里写了模板串示例 ⇒ 当场被该门禁命中，已改写。

### 29.6 DB 时间单位混用（原登记：「31 列秒 / 24 列毫秒 / 6 处」）

#### 29.6.1 形态（逐列分桶，不是 min/max 目视）

`probe-db-unit-mix-0918.mjs`（只读）对**全部数值型时间列**做分段计数（`sec = 1e9..1e12-1` / `ms = 1e12..1e15-1`），读数：

```
非空列: 87  ⇒  纯秒=40  纯毫秒=30  ★同列混用=11  纯越界=6
空列(NULL): 207
```

> **登记被低估**：原登记「秒 31 / 毫秒 24」是按 **min/max 的位数**分类得来的 ⇒ 只看极值必然把「混用列」归进其中一类。逐列分桶才看得见 11 处混用。

11 处里 **4 处在备份表**（`knowledge_entities_20260916.*` / `knowledge_relations_20260916.*`）—— **备份表是历史快照，改它等于伪造历史，不应动**。活表 **7 处**：

| 列 | n | 秒 | 毫秒 |
|---|---|---|---|
| `agent_profiles.created_at` | 576 | 24 (4.2%) | 552 (95.8%) |
| `agent_profiles.updated_at` | 576 | 24 (4.2%) | 552 (95.8%) |
| `knowledge_entities.updated_at` | 48542 | 48541 (100.0%) | 1 (0.0%) |
| `knowledge_relations.created_at` | 75251 | 761 (1.0%) | 74490 (99.0%) |
| `knowledge_relations.updated_at` | 75251 | 40541 (53.9%) | 34710 (46.1%) |
| `workflow_templates.created_at` | 111 | 92 (82.9%) | 19 (17.1%) |
| `workflow_templates.updated_at` | 111 | 2 (1.8%) | 109 (98.2%) |

**★「纯越界 6」全部是我自己的列名正则误捞**：`agent_roles.timeout_seconds`（超时秒数）、`mcp_servers.discover/execute_timeout_secs`、`workflow_execution_stats.total_time_ms` / `workflow_executions.total_time_ms`（**耗时**）、`trajectory_trajectories.is_invalidated`（名字含 `date`）—— 这些列**不是 epoch 语义**，被判「越界」纯属口径问题。**⇒「宽口径命中 N」与「真有 N 个缺陷」计数同形，放宽后必须逐列读值**（同 DETAIL §92.4）。

`trajectory_steps.timestamp_ms`（53 行：毫秒 42 / <1e9 的 11）是**已知项**，来源 = `storage.rs:200` ← `recorder.rs:142` 的相对偏移，而 `causal.rs:19` 明文声明**不假设基准** ⇒ **契约允许的自由度，无需修**。

#### 29.6.2 根因：两套约定并存

`crates/harness/src/util_fns.rs:19-20` **自己就写着**：

```rust
/// 毫秒是运行时侧（TTL 过期、能力统计、会话状态）的统一时间单位；
/// 秒级 `now_ts` 只用于 DB TEXT 时间列与跨天计算，两者不要混用。
```

而 `workflow_templates.created_at` 是 **bigint**（探针类型分布：`{"bigint":198,"text":28,"timestamp with time zone":1}`）却同列混用 ⇒ **违反了自己的约定**。

活跃写入端**已统一毫秒**：

- `crates/dao/src/workflow_template_repository.rs:50-51` ⇒ `Set(axagent_harness::util_fns::now_ms())`
- `crates/dao/src/repo/workflow_template.rs:260` / `:283` ⇒ `Set(chrono::Utc::now().timestamp_millis())`

而秒级写入端仍在多处存在（`company-runtime/src/{work_item_service,org,experience}.rs`、`dao/src/marketplace_service.rs`、`dao/src/knowledge_crud_repository.rs:22-24` 的 `fn now_ts()`、`agent/src/{wiki_compiler,ingest_queue}.rs`）。

⇒ **结论：这是「存量数据 + 多约定并存」问题，不是单一活跃 bug；工作流模板这条链现在已经只写毫秒，不再新增。**

#### 29.6.3 后果（本轮新增：从「形态」推到「所以呢」）

形态是「数据长这样」，后果才是「谁受害」。已知消费端（源码事实）：

```rust
// crates/dao/src/repo/workflow_template.rs:151   list_workflow_templates
query.order_by(workflow_template::Column::UpdatedAt, Order::Desc)
// crates/dao/src/repo/workflow_template.rs:399   find_latest_by_mission_hash（注释：仅返回最新一条）
.order_by(workflow_template::Column::UpdatedAt, Order::Desc).one(db)
```

判据：`归一化挂钟 = CASE WHEN v < 1e12 THEN v*1000 ELSE v END`；`排序错位 ⟺ 原值 DESC 第 1 名 ≠ 归一化 DESC 第 1 名`。

`probe-db-unit-mix-impact-0918.mjs` 读数：

| 检查 | 读数 |
|---|---|
| 正对照（归一化后范围） | `[1785048214002, 1789667852488]`，越界 **0** 行 ✔ |
| **全局 top1 排序错位** | **否**（两形态的最高值恰好都是毫秒值） |
| `find_latest_by_mission_hash` 受影响分组 | **0 / 0** —— `mission_hash` 重复分组**为 0**，无组可选错 |
| `agent_profiles`：秒值行挂钟晚于所有毫秒行的行数 | **0** |
| `knowledge_relations`：同上 | **0** |
| **`list_workflow_templates` 顺序** | **★ 确实不同** —— 真实时间序里 9-14 的两条（`prod-landing-page` / `prod-startup-mvp`，秒值）应排在 9-13 / 9-10 之前，按原值排序却**沉到第 4/5 名之后** |

**⇒ 危害被实证为「远小于预期」**：唯一可见后果是**模板列表顺序错**（111 行里 2 行），且这是**展示层**影响，不改变任何选择结果 —— 因为会「选错」的那些消费端（`find_latest_by_mission_hash`）当前**没有任何重复分组**。

**这也是「形态取证 ≠ 后果取证」的现例**：单看「11 列混用、7 万行受污染」会以为很严重；量化消费端后才知真实影响面是 2 行排序。**先量化后果，再决定要不要动数据。**

#### 29.6.4 方案（**未动数据**；迁移属破坏性动作，须先备份 + 先出方案）

| 方案 | 做法 | 代价 | 风险 |
|---|---|---|---|
| **A（推荐）存量订正** | 把活表 7 处里的**秒值行** `×1000`；逐列限定 `WHERE v < 1e12`，改后断言「归一化挂钟落在 2020..2100 且列内不再混用」 | 一次性 UPDATE，7 列、约 9 万行（含 `knowledge_relations` 40541 行） | 中：需 `SELECT` 计数**先出 dry-run 报告**；`knowledge_relations` 量大，须单列事务 + 改前整表备份 |
| **B 消费端归一化** | 在 `order_by` / 比较处统一包 `CASE WHEN v < 1e12 THEN v*1000 ELSE v END` | 每个消费端都要改，容易漏 | 低（不改数据）但**治标**，且新增写入端无人约束 |
| **C 改列类型** | 时间列统一改 `timestamptz` | 198 个 bigint 列涉及时间语义的一批 + 全部写入/读取端 | 极高，超出本轮范围 |

**建议 A + 一条写入端门禁**（`now_ms()` 是唯一入口，新增列禁止直接 `Utc::now().timestamp()` 写数值列）。**A 的执行须经你确认**（本仓无法回滚 DB）。

### 29.7 项A 剩余 108 条 / B栏 365 条：本轮未批量订正的理由

不是遗漏，是**判据不足**：

1. 上一轮已修 4 条并验证 delta（`before=112 → after=108`，`Δ=-4`，逐文件核对「仅目标文档下降 4、其余不变」）⇒ **现状 108 条是已收窄后的量**。
2. 分流产物 `triage-doc-refs-0918.txt` 显示这 108 条**不是同一类**：`[控制流行]2` / `[越界]20` / `[空行]30` / `[收尾行]` 等。其中 `[越界]20` 条的「语义重定位候选」**全是 `—`**（无可定位标识符，多为 `sea-orm/src/schema/entity.rs:279` 这类**外部 crate 路径**）。
3. ⇒ **修法必须按类分**：行号漂移型可机械订正（有判据）；核查快照表型应**加时间戳而不订正行号**；论断依据失效型要**改论断本身**（这是产品语义判断，按 `AGENTS.md` 属「报告不擅改」）。
4. **待你裁决**：是否授权我按第 3 点的三分法**分批**推进（第一批建议先做「外部 crate 路径去行号」那 20 条 —— 外部依赖行号必然随版本漂移，**钉行号本身就是错的设计**，可机械执行且有判据）。

### 29.8 本轮新增判据与教训

| # | 判据 / 教训 | 归属 |
|---|---|---|
| 1 | **`cargo fmt --check` 报 EXIT=0 不能当编译证据** —— 同一份代码 fmt 判绿而 `cargo test` 报 `E0034` | 并入「编译通过五型假阳性」族 |
| 2 | 同一字面量在文件内承担**多种角色**时（污染值 vs 干净基线），**必须逐处判角色**，不得全量替换/全量保留 | E 组（量纲穷举的反面） |
| 3 | **行位移型 diff 的 `CHANGED_LINES` 不是判据** —— 插入 n 行会让后续全部「变化」；正确判据 = 剔除插入行后比对 + 变化集合恰等于期望集合 | L 组 |
| 4 | **统计量的 before/after 必须取两个不同对象** —— 在已改对象上算 before ⇒ delta 恒 0（恒等式型假判据） | L 组 |
| 5 | **形态取证 ≠ 后果取证** —— 先量化消费端影响，再决定要不要动数据（本例：11 列混用/7 万行 → 真实影响 2 行排序） | 新增 |
| 6 | **备份表不得订正** —— `_YYYYMMDD` 后缀表是历史快照，改它 = 伪造历史 | 新增 |
| 7 | 列名正则捞时间列会**误捞非 epoch 语义列**（`timeout_seconds` / `total_time_ms` / `is_invalidated`）⇒ 逐列读值 | 同 DETAIL §92.4 |
| 8 | 门禁给的「建议修法」可能比原缺陷更糟 —— 采纳前**必须先读被建议函数的实现** | K 组 |
| 9 | 门禁若为**纯正则扫原始文本**，则「在注释里写示例」= 当场制造违规 | K 组 |

### 29.9 复现资产

`output/verify-graph-2026-09-15/`：

| 文件 | 用途 |
|---|---|
| `verify-fix-graphdtos-0918.mjs` / `.log` | graph_dtos 夹具替换的**只读校验**（8 条判据，含归一化 diff） |
| `fix-graphdtos-wiki-page-0918.mjs` / `.log` | 替换执行（含**改前逐行全等校验** + 自动备份） |
| `wiki-page-sites-0918.log` | 19 处 `wiki_page` 的原始清单（改前） |
| `probe-db-unit-mix-0918.mjs` / `.txt` | 全库数值型时间列**逐列分桶** + 正负对照 |
| `probe-db-unit-mix-impact-0918.mjs` / `.txt` | **后果**取证（消费端排序错位量化） |
| `clippy-traj-0918.log` | T1 的 `-D warnings` 读数 |
| `test-harness-0918.log` | T3 的 harness 全量（718 passed） |

回滚点：`output/backup-2026-09-18-round3/`（含 `graph_dtos.rs.bak`、`MEMORY.md.bak` 等）。
