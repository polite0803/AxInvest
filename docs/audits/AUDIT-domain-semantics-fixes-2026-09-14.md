# AUDIT — 领域语义四项裁决执行（2026-09-14）

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


上游报告：`AUDIT-domain-semantics-o1-2026-09-14.md`（O1 注册表 + O2 扫描器）
配套计划：`docs/plans/PLAN-semantica-borrowings.md`（O1 / O2 节）
技能：`rust-consistency-audit` → **方法二十三 ·「一名多义」治理**

---

## §0 一句话

上一轮只做到「**把量纲歧义登记下来**」；这一轮把它推到「**让编译器与 CI 拦住它**」——
四项裁决全部执行：① 打分公式量纲修复 ② 校准哨兵不再冒充最优 ③ 量纲化 newtype 落地并接入首个消费端 ④ 语义扫描接进 CI。

> 附带收获：**自证测试当场抓到本轮新代码的一个真缺陷** —— `#[serde(transparent)]` 下派生的
> `Deserialize` 绕过构造函数，使「构造即钳制」在反序列化路径上完全失效（§4.5）。

---

## §1 执行清单

| # | 裁决项 | 结论 | 行为变更 | 证据 |
|---|---|---|---|---|
| 1 | `evidence_citation.rs:151` 量纲不匹配 → **方案 A** | 已修复 | **是**（匹配变严，见 §2.3） | `evidence_citation.rs:176` |
| 2 | `calibration.rs:280` 哨兵 9999 参与排序 | 已修复（含 §3.4 同族缺陷） | 否（`CalibrationRunner` 零生产调用方） | `calibration.rs:98` / `:356` |
| 3 | `confidence` / `score` newtype 化 | 已落地 v1（2 个类型 + 1 个消费端） | 否（serde 形状不变） | `domain_semantics.rs` |
| 4 | O2 接 CI（`--strict`） | 已接入 | 否（只拦未登记量纲，当前为 0） | `ci.yml` frontend-check |

---

## §2 #1 打分公式量纲修复（方案 A）

### 2.1 改动

```diff
- let score = jaccard * 0.7 + (number_overlap as f64 / claim_len.max(1.0)) * 30.0;
+ let score = jaccard * W_JACCARD + (number_overlap as f64 / claim_len.max(1.0)) * W_NUMBER;
```

其中 `W_JACCARD = 0.7` / `W_NUMBER = 0.3` / `MATCH_THRESHOLD = 0.15` 均为具名常量，
并写明「**两项权重之和必须为 1**，且两项输入都必须先落在 `[0,1]`」这一不变量。

### 2.2 为什么这是唯一自洽的解

| 项 | 值域 | 依据 |
|---|---|---|
| `jaccard` | `[0,1]` | 交集 / 并集 |
| `number_overlap / claim_len` | `[0,1]` | 数字个数 ≤ 去重字符数 |
| **权重和** | `0.7 + 0.3 = 1` | ⇒ `score ∈ [0,1]` |

修复前第二项权重为 `30.0`，于是 `score` 实际可达 **30.7**，而下游**全部**按 `0–1` 消费：
字段文档（`0.0-1.0`）、markdown 渲染色带、前端 `matchConfidence * 100`。

### 2.3 ⚠️ 行为变更（必须知情）

1. **匹配判定变严**。`score > 0.15` 在修复前近乎恒真（数字项一项就能给出 4.5）；
   修复后若数字不重合，需 `jaccard > 0.214` 才命中。
   ⇒ **纯靠数字重合、文本几乎不相似的引用会从「有来源」变为「未识别来源」** —— 这正是修复目标
   （数字重合不构成证据）。
2. **`support_rate` 会下降**。`supported` 只在 `if let Some((analyst_id, score, snippet))` 命中分支内自增，
   故命中数减少 ⇒ 分子减少。**分母（`total_claims`）不变**。
   - 判定为**合理**：没有来源就不该算「有数据支撑」。
   - 但这是**对外可见的 KPI**（报告里渲染成「数据支撑率 xx%」）⇒ 单列告知，不擅自解耦。
3. **排序结果可能变化**：`citations` 按 `match_confidence` 降序，权重变了序也可能变。

### 2.4 回归测试（**关键设计**）

新增 `test_match_confidence_is_ratio_not_saturated`。为什么不能只断言 `<= 1.0`：

> `Ratio01` **构造即钳制** ⇒ 越界会被压成 `1.0` ⇒ 「在范围内」**恒真**、断言等于没写。

判据改成「**命中但未饱和**」：`0.05 < max < 0.99`。
本用例（3 个数字 + 约 21 个去重字符）在旧公式下第二项 = `3/21 × 30 ≈ 4.29` ⇒ 会正好饱和到 `1.0`，
**该断言在修复前必红**。

---

## §3 #2 校准哨兵不再冒充最优

### 3.1 问题形态（比"哨兵进排序"更准的描述）

`run()` 升序排序，哨兵（`999` / `9999`）是大数 ⇒ **自然落在末尾**。
所以「哨兵排在前面」不成立；真正的缺陷是：

> **没有任何有效性判断** ⇒ **全部参数都失败**时 `results[0]` 仍是哨兵，
> 上层照样打印「--- 最佳参数 ---」（分数渲染成 `N/A`，但**结论成立**）。

### 3.2 改动

| 位置 | 改动 |
|---|---|
| 常量 | `999.0` / `9999.0` / `20` → 具名 `SCORE_NO_TRADES` / `SCORE_SIM_FAILED` / `MIN_TRADES_FOR_FACTS` + `SCORE_VALID_MAX` |
| 谓词 | 新增 `CalibrationResult::is_valid()`（`total_trades >= MIN_TRADES_FOR_FACTS && score < SCORE_VALID_MAX`） |
| 取优 | 新增 `best_valid() -> Option<&CalibrationResult>`；`best_n()` 改为**只取有效结果**；两者**不依赖入参有序**（见 §3.4） |
| 渲染 | `print_best` 用 `is_valid()` 取代魔数 `score >= 100.0`，无效项追加 `⚠无效` |
| 测试 | `test_full_calibration_scan` 的「最佳参数」改用 `best_valid`，全无效时 **panic 并说明先修仿真** |

`best_n` 的语义变更（过滤无效项）**零影响面**：全仓 grep 确认它**没有任何调用方**。

### 3.3 新增纯单元测试

`test_best_selection_ignores_sentinels` —— 用**合成数据**锁定三种形态（全失败 / 部分失败 / 全有效）。
为什么合成而非真跑：真实扫描是否产生哨兵取决于内核行为与成交数，
**无法稳定复现「全失败」这一关键形态**。

### 3.4 ⚠️ 顺手补出的**第二处同族缺陷**：`find` 隐含「入参已升序」

首版 `best_valid` 写成 `results.iter().find(|r| r.is_valid())`。上述新测试**故意传乱序切片**，
于是它当场红了 —— 注意：**是测试抓到了代码，不是测试写错了**：

```
test_best_selection_ignores_sentinels ... FAILED
  assertion `left == right` failed
    left: 12.5   ← find 返回的「第一个有效项」
   right: 3.0    ← 真正的最小有效分数（分数是拟合误差，越小越优）
```

**根因**：`find` 把「**入参已升序**」这一前置条件**藏进了实现**。`run()` 恰好排过序因而正确，
但 `best_valid` / `best_n` 是 **公开 API** ⇒ 任何调用方传入乱序或手工构造的切片，
都会**静默拿到错误答案**（不报错，且结果看着合理）。
这与「哨兵冒充最优」**同属一族**：都是把「排序 / 有效性」的先验寄托在调用方身上。

| 方法 | 修复前 | 修复后 | 对已排序入参（`run()` 输出） |
|---|---|---|---|
| `best_valid` | `find(is_valid)` | `filter(is_valid).min_by(score)` | **逐位等价** |
| `best_n` | `filter(is_valid).take(n)` | `filter(is_valid).sort_by(score).truncate(n)` | **逐位等价** |

⇒ **生产行为零变化**，只是移除了隐式前置条件（哨兵判据本就靠 `is_valid`，与排序无关）。

**为什么不改测试（把 `vec` 排序即可变绿）**：那等于**删掉守卫** —— 排序后的切片再也测不出这个隐式依赖。
故在切片上方写明「**不要为了让本测试变绿而排这个序**」，并新增两条断言锁死
「`best_n` 取的是**最优**的 n 个，而非**最先**的 n 个有效项」。

**影响面已核**：`best_valid` / `best_n` 全仓仅 2 处调用点，**都在 `calibration.rs` 自身**
（`:410` 真实扫描路径 + 本节单元测试）⇒ 无外部消费方，无回归风险。

---

## §4 #3 量纲化 newtype（声明之外的执行侧）

### 4.1 交付

`harness/src/domain_semantics.rs` 新增 `Ratio01`（`[0,1]`）与 `Percent100`（`[0,100]`）：

- 构造即钳制；`NaN → ZERO`；`±∞` 收敛到边界；
- `is_in_range(v)` 判**原始值**（钳制不掩盖越界事实，供审计）；
- `From<Ratio01> for Percent100`（×100）与反向（÷100）—— **跨单位只有这一条通道，且必须写明类型**；
- `#[serde(transparent)]` ⇒ **JSON 仍是裸数字**，前端 `matchConfidence * 100` 契约不变。

### 4.2 首个消费端

`EvidenceCitation.match_confidence: f64 → Ratio01`。
选它的理由：**它就是刚被证明写错量纲的那个字段** —— 拿真实事故当第一个接入点，比拿无关字段试水更有说服力。

配套：`match_confidence: 0.0` → `Ratio01::ZERO`；`(score * 100.0).round() / 100.0` → `Ratio01::new(score)`；
markdown 展示改走 `Percent100::from(...)`（**算术与修复前逐位一致**，只多了 `.get()`）。

### 4.3 设计取舍（四条，改动前先读）

| 取舍 | 理由 |
|---|---|
| 钳制而非 panic | 越界多来自上游数据；panic 把「量纲错」升级为「链路断」 |
| `serde(transparent)` | 否则 JSON 形状变成 `{"0":0.85}`，前端契约被无声改坏 |
| 只给 `From`，不给 `as` 捷径 | 隐式穿越正是事故成因 |
| 测试锚在边界与方向 | 越界 / `NaN` / `∞` / `0.85→85→0.85` 往返 / 「把 85 当比率会得到 1.0 而非 0.85」 |

### 4.4 边界（**newtype 保护不到的地方**）

跨语言边界（Rust → JSON → TS）**newtype 必然失效**：TS 侧只有 `number`。
所以本次前端同步补了两处：① 类型声明处写明契约 `[0,1]` 与来源类型；
② **删掉 `Math.min(x*100, 100)` 的静默压平** —— 越界改为红色告警（`#ef4444`）。

> 这一条很重要：那个 `Math.min` 正是本次缺陷**潜伏很久**的原因 ——
> 它把「恒满格」伪装成正常渲染，`> 0.5` 又让它恒绿。**越界必须看得见。**

### 4.5 ⚠️ 自证过程中抓到的**自身设计缺陷**（本轮最有价值的发现）

首版实现里两个 newtype 都写了 `#[derive(..., Deserialize)]` + `#[serde(transparent)]`，
并配了「反序列化越界同样被钳制」的断言。**测试当场失败**：

```
test_newtype_serde_shape_unchanged ... FAILED
  assertion `left == right` failed
    left: 450.0      ← from_str("450.0") 的返回值
   right: 100.0      ← new(450.0) 应有的返回值
```

**根因**：`#[serde(transparent)]` 会让**派生的 `Deserialize` 直连内层 `f64`**，
**绕过 `new()`** ⇒ 「构造即钳制」在**反序列化路径上完全失效**。

这正是本项目反复出现的形态 —— **校验写在构造函数里，却存在一条 ingress 绕过构造函数**
（同类：`register` 侧写了校验但 `seed` 直插 DB）。只不过这次它出现在「用来防这类问题」的工具里。

**修法**：不派生 `Deserialize`，手写实现显式走构造函数：

```rust
impl<'de> Deserialize<'de> for Ratio01 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        Ok(Self::new(f64::deserialize(deserializer)?))
    }
}
```

并把「越界输入反序列化」固化为回归断言（断言**故意保留**，它就是锁）。
⇒ 已登记为判据 **#82**、技能方法二十三 §3 步骤 6 的取舍表。

---

## §5 #4 接入 CI

### 5.1 门禁语义（**这一条是本轮的设计决策，不是顺手改的**）

```
--strict        : 只拦 A 段（未登记量纲 = 由代码推出的硬事实）
--strict-plan   : 上述 + PLAN 病点 > 0 也拦（人工巡检用，**不设为默认**）
```

**为什么不拦 PLAN 病点**：PLAN 是**待办清单**，按定义就记着未修项。
让「清单上有未修项」去挂 CI ⇒ 人们会**删条目**而不是修代码 —— 信号不是被解决，是被消灭。

### 5.2 门禁自证（**从不上红的门禁 ≈ 没装门禁**）

把判定抽成纯函数并在自检里覆盖四格：

```js
function gateExit(anomalousCount, planBadCount, strictPlan) {
  return anomalousCount > 0 || (strictPlan && planBadCount > 0) ? 1 : 0;
}
```

自检 5 格含**「默认放行」那一格** —— 否则别人把默认值改反，门禁静默失效而自检仍全绿。

### 5.3 结案项转正向守卫

`O-claim-3` 原文检查「PLAN 写了 `position ∈ [0,1]`」——
PLAN 已在上一轮更正（第 173 行 ⚠ 注），该检查会**永远报同一句「PLAN 写错」**（垃圾信号）。
本轮改写为正向守卫：**「`position_pct` 恒为 0–100，不得再出现 0–1 口径」**，
并新增 `已结案` verdict。

### 5.4 CI 位置

`.github/workflows/ci.yml` → `frontend-check`，紧跟「Check line endings (LF)」之后：

```yaml
      - name: Check domain semantics (unit registry / one-name-many-concepts)
        run: |
          node scripts/check-domain-semantics.mjs --selftest
          node scripts/check-domain-semantics.mjs --strict
```

放这么前的理由与行尾检查相同：**纯 Node、不依赖 `npm ci`、几秒内出结果**。

---

## §6 验证证据

| 项 | 命令 | 结果 |
|---|---|---|
| 脚本自检 | `node scripts/check-domain-semantics.mjs --selftest` | **25/25**（检测器 7 + 注册表 9 + raw-string 4 + **门禁判定 5**） |
| 语义扫描 | `node scripts/check-domain-semantics.mjs` | A 段 **0**；覆盖率 **100%（553/553）**；一名多义 4 / 量纲单义 6 |
| 门禁 | `… --strict` | **exit 0** ✅ |
| 门禁（含待办） | `… --strict-plan` | exit 0（PLAN 病点已归零：`已消失` 1 / `已结案` 1 / `查无实据` 2） |
| Rust 格式 | `cargo fmt -- --check` | 0 差异（首跑 5 处差异已整改） |
| 前端格式 | `dprint fmt` + `dprint check`（改动文件） | Formatted 1 file / **check 0** ✅ |
| 前端类型 | `npm run typecheck` | **exit 0** ✅ |
| 前端测试（新增） | `node scripts/vitest-run.mjs run evidenceConfidenceBar` | **5 passed / 1 file** ✅ |
| Rust 测试 | 见 §6.1 | — |

### 6.1 Rust 测试与 clippy

| 步骤 | 命令 | 结果 |
|---|---|---|
| 格式（**定点**） | `rustfmt --edition 2024 --check` × 本次 3 个文件 | **exit 0** ✅ |
| harness（`domain_semantics`） | `cargo test -p axagent-harness --lib domain_semantics` | **12 passed / 0 failed** ✅ |
| analysis-engine（`evidence_citation`） | `cargo test -p axagent-analysis-engine --lib evidence_citation` | **5 passed / 0 failed** ✅ |
| market-sim（`calibration`） | `cargo test -p axagent-market-sim --lib calibration` | **4 passed / 0 failed** ✅（含真实扫描 `test_full_calibration_scan`，89.4s） |
| clippy（3 crate） | `cargo clippy -p axagent-harness -p axagent-analysis-engine -p axagent-market-sim --all-targets -- -D warnings` | **exit 0** ✅ |

**✅ 并发锁已解除，上表两项于 02:56 补齐** —— 当时被并发会话的构建锁阻塞
（`Blocking waiting for file lock on build directory`，6 个 cargo/rustc 进程），
`MARKETSIM_EXIT=0` 与 `CLIPPY_EXIT=0` 均已取到。

**⚠️ 全仓 `cargo fmt -- --check` = `FMT_EXIT=1`，但差异文件不在本次改动清单内**：
`src-tauri/src/commands/stock_analysis.rs:4636`、`src-tauri/src/commands/stock_workflow/hooks.rs:131`
（均带 `P1-6(2026-09-14)` 标记）⇒ 属**并发会话中间态**，按纪律**未碰对方文件**。
故改用**定点** `rustfmt --check` 只验本次 3 个文件 ⇒ **exit 0**。

**并发构建锁（已解除）**：`cargo` 报 `Blocking waiting for file lock on build directory`
（并发会话有 **6 个** cargo/rustc 进程在跑），market-sim 与 clippy 两步一度被阻塞；
02:56 锁释放后两项均通过（见上表）。

**补偿验证（绕开 cargo 锁，保留作为方法论证据）**：为不干等，写了**自包含复刻**
`output/verify-best-selection.rs`（与 `calibration.rs` 新实现逐字同形、零依赖、`rustc` 直编），实测输出：

```
[反向对照] 旧 find 实现 => score = 12.5（非最优）        ← 坐实缺陷真实存在
[新实现]   best_valid  => score = 3（最优）
[新实现]   best_n(2)  => [3, 12.5]（已自行排序）
[反向对照] 全失败      => best_valid = None / best_n = []
[边界]     阈值语义    => trades>=MIN 有效 / score<哨兵 有效
✔ 全部断言通过
```

⇒ 除 crate 级 `cargo test`/clippy（现已补齐）外，**选择逻辑与新增断言另获独立证实**
（含 `>=` / `<` 两处边界语义）。

---

## §7 遗留与后续

| # | 项 | 性质 |
|---|---|---|
| 1 | `support_rate` 下降是否需要重新标定阈值 | **产品决策**（§2.3-2） |
| 2 | `Unit` 仍缺 `Count` 与「价格」变体 | 出现时再补（价格与 `Cny` 同量纲、语义不同） |
| 3 | newtype 只接入了 1 个消费端 | 其余 23 个概念仍是裸 `f64`/`u8`；建议按「曾被误用」优先级逐个接入 |
| 4 | 跨语言边界的量纲保护 | 只能靠命名（`xxxPct` / `xxxRatio`）+ 单侧守卫；TS 无类型级手段 |
| 5 | `L1` 分层护栏 | ✅ **已执行**（2026-09-14 03:05）→ `AUDIT-layer-discipline-l1-2026-09-14.md`。5 条规则 + 棘轮 + 自检 51 格 + 活体自证；`harness` 扇出锁 0 已升级为**硬拦** |
| 6 | **校验只挡上界 ⇒ 静默下溢** | 契约查实：`decision_json.confidence` 确为 **0–100**（prompt `agent_executor.rs:1431/1469/1548` 明写 `"confidence": 0-100`；守卫 `analyst_evolution.rs:122`「必须在 0-100 范围内」）⇒ `stock_analysis.rs:1499 → decision_confidence` **不是缺陷**（已核对无误）。但 `harness/dashboard_report.rs:260` 只判 `< 0.0 \|\| > 100.0` ⇒ LLM 若输出 `0.85`（意图 85%）会**静默通过**，被读成「0.85%」。**未擅自加下限** —— 0.5% 本身合法，判「0.85 是笔误还是真值」属产品口径。同类形态：`position_pct`（`:266`） |
| 7 | **`ci.yml` 本身无测试守卫** | ✅ **已执行**（2026-09-14 03:05）→ `src/__tests__/ci.workflow.test.ts`（8 用例）。**附带发现**：原 `pr-ci.workflow.test.ts` 断言的 `pr-ci.yml` **早已不存在** ⇒ 其 2 个用例**恒被 skip**（空转守卫）；新守卫**刻意不用 `skipIf`**，文件缺失即红。→ 该空转守卫**已于 03:20 删除**（用户裁决），备份 `output/backup-2026-09-14/del-pr-ci-guard/` |
| 8 | 门禁只认**字面量锚点** | 变量参与的比较（`x > threshold`）一律无法判定（§5.4 报告「局限 3」）。要覆盖需类型信息 ⇒ 属 `L1` 分层护栏的范畴 |

---

## §8 改动清单

| 文件 | 动作 | 备份（`output/backup-2026-09-14/fix4/`） |
|---|---|---|
| `src-tauri/crates/harness/src/domain_semantics.rs` | 新增 `Ratio01` / `Percent100` + 4 测试 + 2 处锚点更新 | `domain_semantics.rs.before` |
| `src-tauri/crates/analysis-engine/src/evidence_citation.rs` | 权重常量 + `Ratio01` 接入 + 1 回归测试 | `evidence_citation.rs.before` |
| `src-tauri/crates/market-sim/src/calibration.rs` | 哨兵具名化 + `is_valid` / `best_valid` + 1 单元测试 | `calibration.rs.before` |
| `src/components/stock-analysis/EvidenceCitationPanel.tsx` | 契约注记 + 越界可见（删静默压平） | 未单独备份（dprint 已格式化） |
| `scripts/check-domain-semantics.mjs` | strict 语义 + 门禁自检 + O-claim-3 转正向守卫 | `check-domain-semantics.mjs.before` |
| `.github/workflows/ci.yml` | 新增领域语义门禁步骤 | `ci.yml.before` |
| `docs/plans/PLAN-semantica-borrowings.md` | O1 / O2 执行状态 | `PLAN-semantica-borrowings.md.before` |
