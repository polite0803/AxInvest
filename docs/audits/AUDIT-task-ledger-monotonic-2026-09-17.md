# AUDIT — task_ledger 时间戳全序化 与 disk-cache 缩进修复 的验收取证

> 状态：active ｜ 日期：2026-09-17 ｜ 对应版本：v2.9.7


> 日期：2026-09-17（本地 08:15–08:57，UTC 00:15–00:57）
> 范围：`crates/dao/src/task_ledger.rs`（`next_event_at` 全序化）、`crates/disk-cache/src/lib.rs:229`
> 方式：**全程只读取证**（冻结令期间未改任何 `src-tauri/**` 源码）；cargo 类命令由 worker 执行，产物落 `output/`
> 相关判据：`MEMORY-RULES.md` #457 / #458；技能 `concurrent-session-failure-attribution` §⑦ §⑧

## 冻结状态（收工实测，`NOW=00:57:52Z`）

| 文件 | size | mtime (UTC) | sha256 | 判定 |
|---|---|---|---|---|
| `crates/dao/src/task_ledger.rs` | 29122 B | `00:53:13.230Z` | `0CC9CE5A…AC9E2` | **= 基线** |
| `crates/disk-cache/src/lib.rs` | 29201 B | `21:47:01.195Z` | `2131D4FD…E7B5` | **= 基线** |

`task_ledger.rs` 的 mtime 前进到 `00:53:13` 已解释：worker 为逼 cargo 失效指纹而
`(Get-Item $src).LastWriteTime = Get-Date` —— **只改 mtime、内容未变**。

---

## 一、第 4 项：`next_event_at` 全序化 —— **验收通过**

### 1.1 方法（本环境唯一免疫并发的读法）

注入/变异 → 编译（独立 `CARGO_TARGET_DIR`）→ **拷出测试二进制** → 立刻还原源码 →
此后只反复直跑那个 exe。二进制是冻结产物，不受其后任何源码写入污染。

反例警示：在共享 target dir 上「用 cargo 反复跑」会被并发写者污染 —— 实测有一轮
「DET 3 红 + 2 绿」，绿的两次是 cargo 编到了**已被别人还原回基线的源码**。

### 1.2 读数（固定 exe × N 次重复）

| 探针 | 修复态 exe `F433EA8A…`（08:17:47 拷贝） | 变异态 exe `D9C9D7E4…` |
|---|---|---|
| `new_event_sorts_after_an_existing_future_timestamp` | **0 红 / 20** | **20 红 / 20**（另一独立批次 13/13 红） |
| `rapid_writes_keep_timeline_strictly_ascending` | **0 红 / 200** | **194 红 / 200**（另一批次 48/50 红） |
| `created_event_is_queryable_and_timeline_is_ascending` | **0 红 / 200** | 3 红 / 200（另一批次 0/200） |

- 变异态跑整份 `task_ledger` 套件 × 5 轮：每轮恒为 `10 passed; 2 failed`，红的固定是上表前两条。
- 基线隔离态 10× 连跑：全绿（`12 passed; 0 failed`），sha 全程 `0CC9CE5A…`。
- 失败是**由被测对象自身形态**产生的，非环境噪声：
  `assert_eq!` 报 `left: Some("n5") vs right: Some("n4")`（第 4 条事件顺序错位）、`left: None`。

### 1.3 编译/静态门禁

| 命令 | 退出码 | 日志含 `Checking` | 判定 |
|---|---|---|---|
| `cargo clippy -p axagent-dao --all-targets -- -D warnings`（隔离 target dir） | 0 | **是** `Checking axagent-dao` | **真绿**，且 `SRC_UNCHANGED_DURING_CLIPPY=True`、`HAS_ITEMS_AFTER_TEST_MODULE=False` |
| 同上（共享 dir，两次） | 0 | **否**（83s / 未完成） | **假绿，作废**（cargo 短路） |
| `cargo fmt -p axagent-dao -- --check` | 0 | — | 通过 |

### 1.4 ⚠ 残余不确定性（不得夸大为「已证明」）

1. **修复前源码已不在工作树上** ⇒ 原始 flaky **无法直接验证**。能验证的只有「修复态在同口径下 0 红」。
2. gp16 的变异体保留了那次 `SELECT`（`let _ = latest;` 丢弃结果）⇒ 比真正修复前代码**多一次 DB 往返**，
   拉大了两次写入的间隔 ⇒ 变异体下 `created_event…` 几乎不红。
   **故不能用「变异体下它不红」推断「原 flaky 已消失」。**
3. 被寄予期望的 `created_event_is_queryable…` 探针判别力很弱（合并 3 红 / 400 次 ≈ 0.75%）；
   真正扛判别的是 `new_event_sorts…`（确定性 100%）与 `rapid_writes…`（≈96%）。
4. 变异体有两种形态，不可互换：「只废守卫、保留 SELECT」（MUT1）与「整条查询链 + 守卫一并替换为桩」（MUT2）。
   对 `rapid_writes…` 两者相近（92/100 红 vs 194/200 红）；但对 `created_event…` 给出 **0/100 红 vs 3/200 红**
   —— 都极弱。**只有 MUT2 能等价于「修复前不做读」的那种形态，MUT1 不能。**

---

## 二、第 3 项：`disk-cache/src/lib.rs:229` 缩进 —— **验收通过，且为最小正确解**

| 命令 | 退出码 | 关键行 |
|---|---|---|
| `cargo clippy -p axagent-disk-cache --all-targets -- -D warnings`（还原后） | **0** | `Checking axagent-disk-cache` |
| 反证：`:229` 由 5 空格改 3 空格后同命令 | **101** | 只报 `crates\disk-cache\src\lib.rs:229:9` **`doc list item without indentation`**，`help: indent this line`（建议补 2 空格 ⇒ 5） |
| 逐字节还原后 | **0** | 日志含 `Checking axagent-disk-cache`；sha 回 `2131D4FD…` |
| `cargo fmt -p axagent-disk-cache -- --check` | 0 | 无 diff |

反证只报 `:229` 一处，其余 6 行（221/222/223/225/226、228）一处都没报
⇒ 判据是「缩进 ≥ 该列表项的内容列」，不是「越小越好」，也不是「越大越好」；原值即最小正确值。
（未实测「7 行统一改成 2 或 4 会新增 6 个 error」这一说法，既不证实也不证伪。）

---

## 三、事故与归属（本轮最贵的一节）

### 3.1 事实

- `output/` 下**同时存在两个 actor** 用同一 `gp16-` 前缀写日志：一方日志被另一方覆盖
  （`gp16-dao-clippy.txt` 被覆盖为 `SRC_AT_BASELINE_BEFORE_CLIPPY=True…`）。
- 两个 worker 被派了**同一个文件的变异验证**，各自「注入 → 编译 → 还原」，互相把对方的实验对象改回基线
  ⇒ 一方的 5 连跑被截断成「3 红 2 绿」，另一方撞上 `SOURCE_STABLE_DURING_BUILD=False`。
- **结论**：写者是**两个**，不是「某一个 worker」。我此前对用户说的「不是第三方会话，就是本队某 worker」
  **已收回**。

### 3.2 判据（已固化到 `MEMORY-RULES.md` #457 / #458）

1. **`EXIT=0` ≠ 该 crate 被检查过** —— 必须同时证明日志含 `Checking <crate>`
   （test 则 `Compiling <crate>`），否则是 cargo 短路。
2. **「日志 mtime 没被动过」≠「这次编译编的是我的形态」** ——
   判归属必须**编译前后各取源码 sha256 且相等**；mtime 只用于「有没有人在动」。
3. **mtime 判归属有四类反例**：故意只改 mtime 的脚本、`WriteAllBytes` 的「内容回退而 mtime 前进」、
   变异测试脚本共存、日志自身被污染 ⇒ **终裁用内容 hash**。
4. **多 worker 共用 `output/` 必须分配唯一文件名前缀**；**同一文件同一时刻只允许一个变异注入者**。

### 3.3 本条登记、冻结期内不擅改的缺陷（需产品语义决策）

`crates/dao/src/task_ledger.rs:225`（doc comment）：

```
/// **不开事务（有意）**：本函数只写一行事件，单条 `INSERT` 本身就是原子的，包一层事务
```

改动后 `record_task_created`（`:232-256`）实为 `:239 let at = next_event_at(db, …).await?`（一次 SELECT）
+ `:251 task_events::Entity::insert(event).exec(db)`（一次 INSERT）⇒
**「只写一行事件」的前提已不成立**（结论「不包事务」仍成立，`:229-231` 的论证也仍然有效，
它恰好已点明「同一任务被并发重复创建时，默认隔离级别下两个事务会读到同一个 max」）。

建议改写为「只写一行事件 **+ 一次读**」。**因涉及语义表述决策，只登记、未动手。**

---

## 四、边界与未闭环项

- 未改任何 `src-tauri/**` 源码；零 git 命令；零 kill；零 `--workspace` 级 cargo。
- 未验证：修复前源码的真实 flaky 行为（见 §1.4）；`disk-cache` 的「7 行统一缩进」反证。
- 换环境的坑：仓库根**不是** cargo workspace 根，`cargo` 必须带
  `--manifest-path src-tauri/Cargo.toml`；`axagent-disk-cache` 冷编译实测 **57m49s**
  （闭包内含 `aws-lc-sys`），不是「小目标很快」。
