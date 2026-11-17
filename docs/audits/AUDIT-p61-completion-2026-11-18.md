# AUDIT — P6.1 股票迁出完成度实测（2026-11-18）

> 状态：active ｜ 日期：2026-11-18 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md` §6.1 判据行（:999）、§54（#35 八条裁定）、§64.10/64.11（表归属与 op 清单）、§204.4（可迁面清零）、§205–§208（B22/#349/A+B+C）、`AUDIT-t2-migration-axis-2026-11-16.md`
> 性质：**时点快照**——记录 2026-11-18 盘上机械实测；判据行的历史读数与当前实况的口径差如实对账，不替任何一侧辩护。
> 取证方式：`node scripts/audit-l1-blockers.mjs`（只读档）、`node scripts/check-capability-register.mjs --dump-readings`、`grep -rn`（指定路径）；所有数字均可用同命令复跑。

---

## 一、结论摘要

P6.1 的**已裁定工作全部完成并推送**（HEAD `00918c06a`，tag `v3.0.1`，与 `origin/master` 同步）：

- T2 命令迁移 47 批（#301–#349）+ B22（#348）、新架构三 seam 合流（#349）、B1–B8 插件打包链路、判据③ A+B+C 三链（#352）——均已落地。
- 阻塞面实测：Rust 反向阻塞 **0 处 / 0 文件**、跨包口 **0 处 / 0 文件**；前端反向 **12 处 / 10 文件**（与 §0.10.1 登记一致）。
- 判定档 `commandTotal=1171`、`commandModules=145`、`registryFileLines=1232`。

**但判据行 :999 的四项终局判据按字面口径均未达标**，原因是判据行写于 2026-11-04（立项时），而 PLAN 后续章节已把终局形态改为「过渡期宿主仍链接域 crate、通过 re-export 兼容」（§64.10 / §80 / §81）。**这是口径差，不是本轮未完成的工作**；判据行文字需要一次显式订正才能与终局自洽（见 §四）。

## 二、判据行 :999 四项逐条对账

| # | 判据行原文（2026-11-04 立项） | 2026-11-18 盘上实测 | 判定 |
|---|---|---|---|
| 1 | 主壳 `grep` 域 crate = 0 | `src-tauri/src` 命中 **500 处**（`axagent_analysis_engine::` / `axagent_astock_data::` / `axagent_quant::` / `axagent_stock_core::`；含 `lib.rs:143` 注释中的说明） | ✗ 字面未达标；按 §80/§81 过渡期口径为**预期态**，终局断开依赖才算完成 |
| 2 | `src-tauri/Cargo.toml` 无股票 crate | **6 条依赖**：`axagent-analysis-engine` / `axagent-astock-data` / `axagent-company-runtime` / `axagent-market-sim` / `axagent-quant`（均 :128–147）+ `axagent-stock-core`（:160） | ✗ 同上，过渡期预期态 |
| 3 | 卸载股票插件后其余 1 099 条命令可用 | 判定档 `commandTotal=1171`，插件边界口径股票命令 71 条 ⇒ 理论余量 1100 条；**未做真实卸载端到端测试** | △ 算术可达，**未实测** |
| 4 | `stock_*` 表与数据保留 | entities 侧股票/荐股/组合/反思表实体文件 **21 个**（`stock_analyses` / `stock_reflections` / `reco_picks` / `reco_pool_batches` / `portfolio_holdings` / `trades` 等）仍在；`MIGRATIONS = &[]`（声明式收敛引擎建表），历史手写 migration 已删（§8364） | ✓ 表实体保留；数据保留**未做升级前后数据对账实测** |

## 三、阻塞面与命令面实测（`audit-l1-blockers.mjs` 只读档）

| 度量 | STOCK | OPC | 对照登记 |
|---|---|---|---|
| 命令条数 / 模块数（插件边界口径 `cmdRe`） | 71 / 12 | 0 / 0 | 插件命名命令已在册 |
| 命令条数 / 模块数（审计 19 域口径） | 70 / 11 | 0 / 0 | — |
| Rust 反向阻塞（待改写，H 桶外） | **0 / 0** | 0 / 0 | 与 §0.10.1 现值一致 |
| Rust H 桶（host-owned 白名单） | 21 处 / 7 文件 | 3 处 / 3 文件 | 与 §54.1「B1–B7 终态 ≈96 处」不同：后续批次已进一步压到 21 |
| Rust 合计（待改写 + H） | 21 / 7 | 3 / 3 | 旧 152 口径不可比 |
| Rust 跨包口 | 0 / 0 | 0 / 0 | — |
| Rust 域自有文件内引用 | 478 / 39 | 50 / 8 | 随本体迁出，不算阻塞 |
| Cargo.toml 构建边 | 10 | 1 | 与 §二-2 的 6 条直接依赖不同口径（构建边含 workspace 成员关系） |
| 前端反向阻塞 | **12 / 10** | 0 / 0 | 与 §0.10.1 登记一致（软锚点，改锚前须逐条归因） |
| entities 表文件 | 20 | 27 | 与 §二-4 的 21 个文件差 1：脚本按域归属口径统计 |
| zh-CN i18n key | 3532 | 1894 | — |

**命令面**：`commandTotal=1171`、`commandModules=145`、`registryFileLines=1232`；`finance` 域智能体标注判定档读数 **53**，简单 grep `#[agent_command(...domain = "finance"...)]` 为 **45**，差 **8 条**属判定档脚本口径（跨行属性宏 / 多参数形态）与单行 grep 的差异，**未逐条归因，列为未完成项**（见 §四）。

## 四、未完成项与需裁定项（如实登记）

1. **判据行 :999 文字订正**（需裁定）：四项终局判据写于立项时，与 §64.10/§80/§81 已裁定的「过渡期宿主链接域 crate」形态冲突。建议改判据为两阶段：过渡期 = 阻塞面 0 / 前端反向清零 / 命令面迁移完成；终局 = 宿主断开域 crate 依赖（另立批次）。**未改文档**——等用户确认口径。
2. **判据③④未端到端实测**：卸载插件后命令可用数（算术 1100）与升级前后表数据保留，均只做了静态取证，未做真机/迁移实测。
3. **finance 53 vs 45 的 8 条口径差**：未逐条归因，不能把「53」当精确的命令清单引用。
4. **§204.4 D 组 `quant_backtest_run`**：样本/预算裁定未完成，按 §142 序列等待。
5. **F9 43 条**：§204.4 列为「刻意不迁」，是否永远留宿主未完成最终裁定。
6. **前端反向 12/10**：§0.10.1 软锚点，按 §125.7「改锚前先逐条归因差额」，本轮未逐条复核。

## 五、取证命令（可复跑）

```bash
node scripts/audit-l1-blockers.mjs                      # 只读档，输出上表
node scripts/check-capability-register.mjs --dump-readings
grep -rn 'axagent_\(analysis_engine\|astock_data\|stock_core\|quant\)::' src-tauri/src --include='*.rs'
grep -n 'axagent-\(analysis-engine\|astock-data\|stock-core\|quant\)' src-tauri/Cargo.toml
grep -rn '#\[agent_command' src-tauri/src --include='*.rs' | grep -i finance
```

## 六、性质声明

本文件是**时点快照**：上述读数对应 2026-11-18 HEAD `00918c06a`；后续代码/文档行号漂移不订正本文件，新读数请另出快照。文中「未完成项」是对当前工作状态的如实记录，不是对已裁定结论的推翻。
