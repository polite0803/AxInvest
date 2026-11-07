# CRDT 接入与缺陷修复 — 2026-09-13

> 状态：active ｜ 日期：2026-09-13 ｜ 对应版本：v2.9.7


> **触发**：用户指令「接入并修复缺陷」（承接四轮 CRDT 专项审计的处置建议）
> **范围**：`src-tauri/crates/crdt` 接入至 `axagent-device` + 四处缺陷修复
> **纪律**：git 零接触；所有改动可回滚（备份见 `output/backup-2026-09-13/crdt-fix/`）

---

## 0. 结论摘要

**这次接入是必要的 —— 一接入就暴露 `crates/crdt` 根本无法编译（D1），另有三个语义缺陷（D2–D4）。**

此前这些问题被「零引用」完美掩盖：它不在任何依赖链上，`cargo check`（不带 `--workspace`）从不编译它，
于是「从未被编译」=「从未被验证」，而项目里没有任何机制会去碰它。

| 编号 | 缺陷 | 严重度 | 状态 |
|---|---|---|---|
| **D1** | `lww_register.rs:69` **编译错误** —— `&str > &String` 在项目 toolchain（1.97.0）下非法 | **P0** | ✅ 已修 |
| **D2** | `ORSet::remove` 删除信息**无法通过 merge 传播**（空集与「从未见过」不可区分） | P1 | ✅ 已修 |
| **D3** | `RGA::merge` 墓碑锚点失效 ⇒ 元素**静默丢失**（存在于 `entries` 却从可见链表断裂） | P1 | ✅ 已修 |
| **D4** | `GCounter` 的 `PartialEq` 比较**总值**而非状态 ⇒ 收敛判断假阳性 | P2 | ✅ 已修 |

另更正上一轮的两处错误结论（见 §4）。

**另有一条会卡 CI 的发现（已修）**：`src/commands/error_code.rs` 有 **8 条 `unused_imports`**
（权威清单见 §5.1），根因是 `src/lib.rs:14` 的 `mod commands;` 为**私有** ⇒ 内部 `pub use` 退化为
crate 私有导入 ⇒ 未被引用的重导出触发该 lint，而 CI 用 `-D warnings`。
**该缺陷已在第七轮按方案 A 清除**（见 §8.1），`cargo check -p axagent --all-targets` 已验证 **0 warning**（§8.4）。

**第七轮（用户指令「继续」）另完成两件待裁决**：删除 `runtime/src/collaboration/` 633 行死分叉（§8.2）、
把名实不符的 `GCounter::apply_remote_increment` 改名 `observe_remote` 并锁语义（§8.3）；
过程中**自伤一次**（E0585，删 item 后遗留悬空 `///`），已修并沉淀为铁律 63（§8.5）。

---

## 1. 机制：为什么一个编译不过的 crate 能活 11 天

这不是「疏忽」，而是项目验证方式的一个**结构性盲区**：

| 环节 | 事实 | 后果 |
|---|---|---|
| `crates/crdt` 是否在 workspace | ✅ 在（`Cargo.toml:12`） | 名义上属于项目 |
| 是否有人依赖它 | ❌ **入度 = 0**（`use axagent_crdt` 全仓 0 命中） | **不在任何依赖链上** |
| `cargo check`（不带 `--workspace`） | 从主 crate 出发做可达性编译 | **永远不编译它** |
| 本地校验脚本 `scripts/ci-check.mjs:119` | `cargo clippy --all-targets --all-features -- -D warnings` —— **不带 `--workspace`** | 本地恒绿 |
| CI `ci.yml:145/150/236` | `cargo clippy/check/nextest **--workspace**` | **CI 应当早就红了** |

### 1.1 决定性证据（最小复现，非推测）

```rust
// 与 crates/crdt/src/lww_register.rs:69 完全同型
let a: &str = "x";
let b: String = "y".to_string();
println!("{}", a > &b);        // &str vs &String
```

| toolchain | 结果 |
|---|---|
| 默认 toolchain（1.96.1） | ✅ 编译通过 |
| **项目 toolchain 1.97.0**（`rust-toolchain.toml` 固定） | ❌ **编译失败，退出码 1** |

⇒ `impl PartialOrd<String> for str` 的可用性在两个版本间发生了变化。
**项目固定 1.97.0 ⇒ 该行必然编译不过**，而 `lww_register.rs` 的 mtime 是 `2026-09-02`，即自建库起从未改动。

### 1.2 同类风险：入度 = 0 的 crate 一共有三个

由 `cargo metadata` 拓扑分析（`output/tmp-semantica-src/analyze_crates.py`）得出：

| crate | 入度 | 说明 |
|---|---|---|
| `axagent-crdt` | 0 | ✅ 本次已修复并**接入**（入度变为 1） |
| `axagent-mobile` | 0 | `crate-type=["staticlib"]`，仅 3 行 `lib.rs`，依赖主 crate |
| `schema-gen` | 0 | `src-tauri/schema-gen`，构建期工具 |

**尚未验证 `axagent-mobile` / `schema-gen` 是否有同类未暴露错误** —— 它们是同一盲区的剩余覆盖对象。

> **根因判定（铁律 11「规则文件 ≠ 检查脚本 ≠ CI 真跑 ≠ 代码真遵守」的又一形态）**：
> 本地检查脚本与 CI 的**检查范围不一致**（本地少 `--workspace`）。
> 这导致同一个 crate 在本地恒绿、在 CI 恒红 —— 而只要没人看 CI，就没人知道。

---

## 2. 接入：`axagent-crdt` → `axagent-device`

### 2.1 接入点的选择依据

不是随便挑一个地方挂上，而是选**已在真实活跃链路、且存在同一能力的重复实现**的位置：

```
device/src/sync_engine.rs  ──(已在跑)──▶  commands/device_sync.rs (9 处 use)
        │                                        │
        │                                        ▼
        │                           register_commands.rs:297-322（26 条命令）
        │                                        │
        ▼                                        ▼
   SyncEngineImpl { crdt_engine }          前端 227 处 deviceSync UI
```

`SyncEngineImpl` 里本就有 `Arc<RwLock<CrdtEngine>>`（OT 引擎）与**一整套手写版本向量**——
后者与 `axagent_crdt::VersionVector` 是同一能力的双实现。接入即 = 让这份手写实现退场。

### 2.2 五项改动

| # | 改动 | 位置 |
|---|---|---|
| 1 | **消除重复类型**：`crdt::VVEntry{site_id}` 改为复用 `harness::VersionVectorEntry{device_id}`（harness 是共享数据模型的权威来源，AGENTS.md 第 12 条） | `crdt/src/version_vector.rs` |
| 2 | **新增 O(1) 的 `observe(device_id, counter)`**（取 max，替代逐次递增） | `crdt/src/version_vector.rs` |
| 3 | **删除 `device` 的自建版本向量**（178 行，含 `CausalOrder` 枚举） | ~~`device/src/version_vector.rs`~~ → 已删 |
| 4 | **热路径循环修复**（见 §2.4） | `device/src/sync_engine.rs:302-313` |
| 5 | 并发判定改用 `is_concurrent_with` | `device/src/conflict_resolver.rs:59` |

对外路径保持不变：`device/src/lib.rs` 用 `pub use axagent_crdt::VersionVector;` 做 re-export，
`axagent_device::VersionVector` 引用依旧成立（crate 外零消费，但保持口径一致）。

### 2.3 效果：同一能力从 3 份定义收敛为 1 份

| 修复前 | 修复后 |
|---|---|
| `crdt::VVEntry{site_id, counter}`（重复类型） | 复用 `harness::VersionVectorEntry` |
| `crdt::VersionVector`（1,111 行 crate 内，零引用） | **唯一实现**（✅ 已被 device 消费） |
| `device::VersionVector`（178 行手写，活跃） | 已删除 |

### 2.4 顺手修掉的热路径 O(n) 循环

```rust
// 修复前（device/src/sync_engine.rs，注释还自陈「修复原有的 while 循环逻辑」）
for entry in &entries {
    for vv_entry in &entry.version_vector {
        let current = vv.get(&vv_entry.device_id);
        if vv_entry.counter > current {
            for _ in current..vv_entry.counter {   // ← O(计数器值)
                vv.increment(&vv_entry.device_id);
            }
        }
    }
}

// 修复后
for entry in &entries {
    for vv_entry in &entry.version_vector {
        vv.observe(&vv_entry.device_id, vv_entry.counter);   // O(1)
    }
}
```

**为什么这是真问题**：本函数在**每次 `record_change` 都被调用**，而它要遍历**全部变更日志**。
多设备长期同步后单站点计数器可达数千，复杂度是 `O(日志条数 × 计数器值)` —— 量级会迅速失控。
语义上「推进到水位」本就等价于取 max，逐格爬毫无必要。

`observe` 的等价性已用测试锁死（`test_observe_equals_repeated_increment`）。

---

## 3. 四个缺陷的修复

### D1【P0】`lww_register.rs:69` 编译错误

```rust
// 修复前 —— 项目 toolchain 1.97.0 下非法
&& site_id > &self.site_id

// 修复后
&& site_id > self.site_id.as_str()
```

语义完全不变（都是「新站点 ID 字典序更大则替换」）。同时补了 doc 说明该三级打破存在的理由
（时间戳与逻辑时钟都相同时，必须保证各副本选出同一赢家，否则永久分叉）。

### D2【P1】`ORSet::remove` 删除信息无法传播

**修复前**：
```rust
pub fn remove(&mut self, element: &str) {
    if let Some(tags) = self.elements.get_mut(element) { tags.clear(); }   // ← 清空
}
```

**为什么在多副本下静默失效**：清空后 `elements["x"] = {}`，而一个**空集**与
「从未听说过 x」在结构上完全无法区分；`merge` 只能取并集，于是承载删除意图的空集
**传递不出任何信息**：

```
A: add("x") → tagA          B: add("x") → tagB
A.merge(B) → A={tagA,tagB}  B.merge(A) → B={tagA,tagB}
A.remove("x") → A={"x":{}}       ← 删除只体现在本地
B.merge(A) → B={"x":{tagA,tagB}} ← 空集并进来什么都没改变
                  ⇒ B.contains("x") = true   ❌ 删除永不收敛
```

**修复**：引入墓碑集合 `R(e)`，`remove` 只**记墓碑**不动 `A(e)`；存活性判定改为 `A(e) \ R(e) ≠ ∅`；
`merge` 时 `A` 与 `R` **都取并集**。这正是 observed-remove 的标准形态
（remove 只能移除**自己已观察到**的标签），且保留 add-wins 语义。

```rust
let observed: Vec<String> = tags.iter().cloned().collect();
let removed = self.removed.entry(element.to_string()).or_default();
for tag in observed { removed.insert(tag); }
```

**代价（已在模块文档写明）**：墓碑只增不减 ⇒ 内存随冲突历史单调增长，生产环境需要按
「最慢副本已同步水位」做 GC。**本 crate 目前没有 GC**，接入前必须评估这个前提。

### D3【P1】`RGA::merge` 墓碑锚点失效

**修复前**：`mark_deleted` 把已删条目的 `left_id` / `right_id` 置 `None` 并**重连邻居**。

```rust
entry.deleted = true;
entry.left_id = None;      // ← 根因
entry.right_id = None;     // ← 根因
// ...然后把左邻居的 right 指向右邻居（跳过墓碑）
```

**为什么静默丢数据**：`left_id`/`right_id` 是远端并发插入时**定位锚点的唯一依据**
（`find_position_for_merge` 直接读它）。墓碑指针被清空后，远端「在已删条目旁插入」
仍锚回那个**已失去位置的墓碑**：

```
本地: a → b → c        远端在 b 与 c 之间插入 x（x.left=b, x.right=c）
删除 b（修复前）: a ──────▶ c     b 的指针被清空、邻居被重连
merge x: x.left=b → b.right=x;  x.right=c → c.left=x
         但 a.right 仍指向 c ❌
遍历: a → c ⇒ ["a","c"]    ← x 存在于 entries，却不可达（静默丢失）
```

**修复**：墓碑**留在链上**，指针不动，只置 `deleted = true`；由 `to_vec` 遍历时跳过。

```rust
let Some(entry) = self.entries.iter_mut().find(|e| e.id == entry_id) else { return false; };
if entry.deleted { return false; }
entry.deleted = true;
true
```

修复后 `b.right = x` 保持不变 ⇒ `a → b(跳过) → x → c` ⇒ `["a","x","c"]` ✅

### D4【P2】`GCounter` 的 `PartialEq` 比较总值

```rust
// 修复前
impl PartialEq for GCounter {
    fn eq(&self, other: &Self) -> bool { self.value() == other.value() }
}
```

`{siteA: 5}` 与 `{siteB: 5}` 总值都是 5 ⇒ **被判等**，但两者状态完全不同、并未收敛。
任何用 `==` 判断「本地与远端是否一致」的调用方都会拿到**假阳性**。

修复为逐站点向量比较（`self.counters == other.counters`）。
⚠️ 这是**行为变更**（相等性语义变了），已在测试中锁死两个契约：状态比较 + 自反/对称。

---

## 4. 更正上一轮的两处错误结论

| # | 上一轮结论 | 事实 | 根因 |
|---|---|---|---|
| 1 | 「`cargo test -p axagent-crdt` = **18 passed / 0 failed**，代码健康非残骸」 | **不成立** —— 该 crate 在此 toolchain 下**根本编译不过**，不可能跑出 18 passed | 记录与事实不符；未校验 toolchain（`rust-toolchain.toml` 固定 1.97.0） |
| 2 | 「`ORSet::remove` 清空全部 tag ⇒ 并发 add 被误删，探针**期望 true**」 | **期望值本身错了** —— 按 observed-remove 语义，`remove` 移除**调用方已观察到的全部**标签，返回 `false` 才是正确行为。真正的缺陷是**删除信息无法传播**（D2），而非「并发 add 被误删」 | 用「直觉」而非「规范」设定期望值；未做正算复现（铁律 5） |

> **教训**：对 CRDT 这类有**明确规范**的数据结构，判定缺陷前必须先写清「规范语义是什么」，
> 而不是凭「两个 tag 应该都活着」的直觉。上一轮因此把**正确行为**报成缺陷，
> 同时**漏掉了真正致命的缺陷**（删除不传播）。

---

## 5. 验证

| 项 | 命令 | 结果 |
|---|---|---|
| crdt 编译（含测试目标） | `cargo check -p axagent-crdt --all-targets` | ✅ 35s |
| crdt 单测 | `cargo test -p axagent-crdt` | ✅ **33 passed / 0 failed**（新增 14 个回归测试） |
| 语义探针（临时） | `cargo test -p axagent-crdt --test probe_semantics -- --nocapture` | ✅ 5/5，两处 ★ 输出直接证实修复生效 |
| device 编译（接入验证） | `cargo check -p axagent-device --all-targets` | ✅ 2m17s |
| **clippy 门禁**（对齐 CI 口径） | `cargo clippy -p axagent-crdt -p axagent-device --all-targets --all-features -- -D warnings` | ✅ 9m08s，**0 警告** |
| **主 crate 编译**（crdt 现已在主构建链上） | `cargo check -p axagent` | ✅ 10m56s，**0 error** |
| **收尾复检**（并发会话已多次改动本仓，故取**当前状态**权威值） | `cargo test -p axagent-crdt` | ✅ **33 passed / 0 failed**（2m18s） |
| 收尾复检：D1–D4 + 接入点是否仍在位 | 逐条 grep 复检 | ✅ 全部在位（见 §5.2） |
| 收尾复检：`pub struct VersionVector` 收敛度 | `grep -rnE "pub struct VersionVector\b"` | ✅ **仅 1 处** |

crdt 侧的 33 个单测明细：gcounter 6 / lww_register 3 / or_set 10 / rga 7 / version_vector 6。

### 5.1 主 crate 的 8 个 warning —— 权威清单已取回；**归属待确认，非本轮引入**

`cargo check -p axagent` 报 `axagent (lib) generated 8 warnings`，**全部落在 `src/commands/error_code.rs`**，
形态统一为 `unused_imports` —— `pub use axagent_harness::error_codes::*` 的重导出无人消费。

**取回方式（零成本，未重编）**：cargo 把 rustc stderr 缓存在
`target/debug/.fingerprint/<crate>-<hash>/output-<target>`，直接读最新那份即可。
本轮读取结果为 `lib` 与 `test-lib` 两份**告警集合完全一致（各 8 条）**。

**全部 8 条**（行号按**当前**文件；括号内为缓存诊断里的旧行号，因并发会话又插入了 9 行）：

| # | 当前行 | 语句 / 符号 | 形态 |
|---|---|---|---|
| 1 | `:38` (旧 29) | `conversation::{ALREADY_ARCHIVED, DELETE_FAILED, NOT_FOUND, NOT_WORKFLOW}` | 4 个逐符号 unused |
| 2 | `:71` (旧 62) | `browser::ACTION_FAILED`（同行的 `NOT_INITIALIZED` 有 21 处消费，故只报它） | 逐符号 |
| 3 | `:89` (旧 80) | `skill::HOME_DIR_FAILED` | 逐符号 |
| 4 | `:343` (旧 334) | `pub use ...::voice;` | 整模块 |
| 5 | `:373` (旧 364) | `pub use ...::paper;` | 整模块 |
| 6 | `:377` (旧 368) | `pub use ...::reading_list;` | 整模块 |
| 7 | `:539` (旧 530) | `capability::EMBEDDING_FAILED`（`NOT_FOUND` 有 42 处消费） | 逐符号 |
| 8 | `:663` (旧 654) | `pub use ...::unity;` | 整模块 |

#### 根因（已定位，不是拼写问题，是**可见性结构**问题）

| 层 | 事实 |
|---|---|
| `src/lib.rs:14` | **`mod commands;`** —— **私有** |
| `src/commands/mod.rs:53` | `pub mod error_code;` |
| 结论 | 祖先模块 `commands` 私有 ⇒ `error_code` 对外**不可达** ⇒ 里面所有 `pub use` 退化为**crate 内私有导入** |
| 后果 | rustc 的 `unused_imports` 对「不可达的 `pub use`」照常生效 ⇒ 未被 crate 内引用的重导出全部报警 |

也就是说：**这些重导出在文件注释里被写成「权威源在 harness、此处仅 re-export」的门面，
但该门面事实上只对内开放**。删掉它们不会破坏任何外部消费方（已核：其他 crate 里
`axagent::commands` 命中 **0** 次），但也**不等于**它们「没用」——若哪天把 `commands` 放开成
`pub mod`，它们立刻变成有效公开 API。**因此这是产品/API 决策，本轮不擅改**（铁律 7）。

> ⚠️ **三点必须说明**：
> 1. 该文件**不在本轮改动范围内**（本轮只改 `crates/crdt/*` 与 `crates/device/*`）
> 2. 它的 mtime 在本轮开始后**被改过两次**（`11:25:31` → **`12:14:56`**，且文件长了 9 行），
>    而**本会话从未触碰它** ⇒ **确认有并发会话正在编辑同一仓库**
>    （旁证：本轮多次遇到 `Blocking waiting for file lock on build directory`，并有多个 `cargo` / `cargo-clippy` 进程）
> 3. 因此本轮**未修这 8 条** —— 归属不明，且看起来是他人**正在进行的在建工作**的中间状态
>    （形态即「错误码从主 crate 迁往 harness」的迁移残渣：重导出先行、消费点已改直连 harness）

**但请注意**：CI 的 `cargo clippy --workspace --all-targets --all-features -- -D warnings` 会因这 8 条而**失败**
（`-D warnings` 把警告升级为错误）。这是与 crdt 编译错误**并列**的另一个 CI 红因，
建议你确认归属后再清理。**修法**：删除上表 8 条死重导出（或把 `commands` 放开为 `pub mod`，二选一，属行为变更）。


### 5.2 本轮改动未被并发会话覆盖（已核查）

| 检查项 | 结果 |
|---|---|
| 所有改动文件 mtime | 与本会话写入时间一致（11:31–11:36） |
| D1 `site_id > self.site_id.as_str()` | ✅ 在位 |
| D2 `ORSet.removed` 墓碑字段 | ✅ 在位（3 处） |
| D3 墓碑保留指针（无 `entry.left_id = None`） | ✅ 未回退 |
| D4 `self.counters == other.counters` | ✅ 在位 |
| 接入 `device/Cargo.toml` 的 `axagent-crdt` 依赖 | ✅ 在位 |
| `device/src/version_vector.rs` 是否复活 | ✅ 未复活 |
| 收尾重跑 `cargo test -p axagent-crdt` | ✅ 33 passed / 0 failed |
| 收尾复检 `pub struct VersionVector`（严格词边界） | ✅ 仅 1 处（`crates/crdt/src/version_vector.rs:25`） |

> 注：收尾复检时曾对 `rga.rs` 里 `left_id = None` 报警 —— 复查后**是误报**：
> 那两处在 `find_position_for_merge` 内部，是**局部变量**的兜底赋值（邻居找不到时回退），
> `mark_deleted` 本身只做 `entry.deleted = true`，**未清任何指针**。D3 修复完好。
> （再次印证：`grep` 字面量命中必须回看上下文，否则会把局部变量当成结构破坏。）

---

## 6. 改动清单（git 零接触）

| 文件 | 动作 |
|---|---|
| `crates/crdt/src/lww_register.rs` | **修 D1**：`&self.site_id` → `self.site_id.as_str()` + 补 doc |
| `crates/crdt/src/or_set.rs` | **重写**：引入墓碑集合 `R(e)`，修 D2；新增 7 个回归测试 |
| `crates/crdt/src/rga.rs` | **修 D3**：墓碑保留邻接指针；新增 3 个回归测试 |
| `crates/crdt/src/gcounter.rs` | **修 D4**：`PartialEq` 改逐站点比较；新增 2 个回归测试 |
| `crates/crdt/src/version_vector.rs` | **重写**：复用 harness 类型 + 新增 `observe`；新增 2 个回归测试 |
| `crates/device/src/version_vector.rs` | **删除**（178 行，重复实现） |
| `crates/device/src/lib.rs` | 删 `pub mod version_vector`，改 re-export `axagent_crdt::VersionVector` |
| `crates/device/src/sync_engine.rs` | 换 import + `observe` 替代 O(n) 循环 |
| `crates/device/src/conflict_resolver.rs` | 换 import + `is_concurrent_with` |
| `crates/device/Cargo.toml` | 加 `axagent-crdt` 依赖 |
| `crates/crdt/tests/probe_semantics.rs` | **临时诊断探针，跑完已删**（5 个场景已由更强的正式断言覆盖） |

**备份**：`output/backup-2026-09-13/crdt-fix/*.before-fix`

---

## 7. 未做的事（待裁决）

### 7.1 建议 CI 增加「孤立 crate 编译」覆盖

本地 `scripts/ci-check.mjs:119` 的 clippy 缺 `--workspace`，而 CI 有 —— 两者不一致。
**建议**：本地脚本对齐 CI，或新增一条 `scripts/check-orphan-crates.mjs`
（列出入度为 0 的 crate 并强制 `cargo check -p <it>`）。
否则下一个「写了但没人用」的 crate 会重演同样的 11 天潜伏。

### 7.2 `runtime/src/collaboration/` 633 行死分叉（~~本轮未删~~ → **已删，见 §8.2**）

| 文件 | 行数 | 状态 |
|---|---|---|
| `crdt.rs` | 174 | 与 `device/src/crdt.rs`（活的 OT 引擎）**类型逐字相同**的缩水复制品 |
| `global_event_bus.rs` | 206 | 零引用 |
| `session_share.rs` | 248 | 零引用（注意：与活跃的 `commands/session_share.rs` **同名但无关**） |
| `mod.rs` | 5 | 仅 `pub mod` 声明；`runtime/src/lib.rs:30` 是唯一引用 |

**零引用已严格验证**（精确 grep 命中 0；正对照 `device_sync` 103 命中；
前端零调用；CLI/CI/脚本零引用）。删除它属于纯冗余清理、零行为影响，
但**不在「接入并修复缺陷」的授权范围内**，故留待你一句话确认。

> 注：`scripts/type-consistency.mjs:828` 的 allowlist 里有 `collaboration_list_sessions`
> （注解「not yet implemented in backend」）—— 但前端**已无任何 collaboration 调用**，
> 该条为**过期 allowlist 项**，可一并清理。

### 7.3 `GCounter::apply_remote_increment` 名实不符（~~报告不擅改~~ → **已改名，见 §8.3**）

```rust
pub fn apply_remote_increment(&mut self, site_id: &str, amount: u64) {
    let counter = self.counters.entry(site_id.to_string()).or_insert(0);
    *counter = (*counter).max(amount);   // 名字说「增量」，实现是「取绝对值的 max」
}
```

- 若语义是 **state-based 合并**（把远端某站点水位并进来）⇒ 取 max **正确**（幂等）
- 若语义是 **op-based 增量应用**（`+= delta`）⇒ 取 max **错误**（会吞掉增量）

两者收敛性质不同（取 max 幂等、`+=` 不幂等需 op 去重），属**产品语义决策**，
按铁律 7 仅报告不擅改。建议二选一后重命名（`apply_remote_value` / 改用 `+=` 并补 op 去重）。

### 7.4 剩余两个孤立 crate 未验证

`axagent-mobile` / `schema-gen` 是否也藏着未暴露的编译错误，**本轮未验证**。
（见 §1.2）

### 7.5 `error_code.rs` 的 8 条死重导出（~~未修，待确认归属~~ → **已删，见 §8.1**）

权威清单与根因见 §5.1。二选一：

| 方案 | 动作 | 性质 |
|---|---|---|
| A（推荐，最小） | 删掉那 8 条无人消费的 `pub use` | 纯清理，零外部影响（其他 crate 引用 `axagent::commands` = 0） |
| B | 把 `src/lib.rs:14` 的 `mod commands;` 改为 `pub mod commands;` | **行为变更**：命令层整棵子树变成公开 API |

**本轮不动的理由**：① 不在「接入并修复缺陷」授权范围；② 该文件 mtime 在本轮内被改动两次
（`11:25:31` → `12:14:56`，且长了 9 行），**确认有并发会话正在编辑该文件**，
此刻插手指令会与在建工作冲突（且对方可能整文件回写覆盖）。

---

## 8. 执行结果（第七轮：三项待裁决已按推荐方案落地）

> 触发：用户指令「继续」。执行前已确认两个目标文件**均已停止变动**
> （`error_code.rs` 停在 `12:14:56`，`collaboration/` 停在 09-05）⇒ 并发冲突风险解除。
> **备份**：`output/backup-2026-09-13/dead-code-cleanup/`（含 4 个文件 + 整个 `collaboration/` 目录，85K）
> **回退**：把 `*.before` 覆盖回原路径、`collaboration/` 拷回 `crates/runtime/src/`、恢复 `runtime/src/lib.rs` 第 30 行。

### 8.1 §7.5 / §5.1 的 8 条死重导出 → **方案 A（删除）** ✅

逐一处理（**注意第 1 条与直觉相反**）：

| # | 原语句 | 处理 | 依据 |
|---|---|---|---|
| 1 | `conversation::{ALREADY_ARCHIVED, DELETE_FAILED, NOT_FOUND, NOT_WORKFLOW}` | **整条 `pub use` 移除** | ⚠ 诊断原文是「unused imports: A, DELETE_FAILED, **NOT_FOUND**, and NOT_WORKFLOW」——**4 个全未使用**，含 `NOT_FOUND`；只删 3 个会漏 |
| 2 | `browser::{ACTION_FAILED, NOT_INITIALIZED}` | 只删 `ACTION_FAILED` | 诊断只点名它；`NOT_INITIALIZED` 有 21 处消费 |
| 3 | `skill::{…, HOME_DIR_FAILED, …}`（6 符号） | 只删 `HOME_DIR_FAILED` | 诊断只点名它；其余 5 个有消费 |
| 4 | `pub use …::voice;` | 整条移除 | 零消费者 |
| 5 | `pub use …::paper;` | 整条移除 | 零消费者 |
| 6 | `pub use …::reading_list;` | 整条移除 | 零消费者 |
| 7 | `capability::{EMBEDDING_FAILED, NOT_FOUND}` | 只删 `EMBEDDING_FAILED` | 诊断只点名它；`NOT_FOUND` 有 42 处消费 |
| 8 | `pub use …::unity;` | 整条移除 | 零消费者 |

**处理方式**：不粗暴删注释，而是把「`// ── 权威源：…（此处仅 re-export）──` + `pub use`」
改写为**指路注释** —— 说明权威源在 harness、**为何不再 re-export**（祖先 `mod commands;` 私有
⇒ 内部 `pub use` 退化为私有导入 ⇒ 零消费者触发 `unused_imports` ⇒ CI `-D warnings` 红）、
需要时直接从 harness 导入。原始文档注释（如 voice 的前端翻译约定、unity 的镜像约定）**全部保留**。

> **教训（本轮我自己先犯了一次）**：读到「unused imports: A, B, C, and D」时，
> 容易只处理「看起来像业务码」的那几个而**留下同名通用码**（我第一版就误留了 `NOT_FOUND`）。
> **诊断给的是全量列表，必须逐符号核对，不能按名字的「重要性」直觉取舍。**

### 8.2 §7.2 的 633 行死分叉 → **删除** ✅

| 动作 | 内容 |
|---|---|
| 删除目录 | `crates/runtime/src/collaboration/`（`crdt.rs` 5521B / `global_event_bus.rs` 6767B / `session_share.rs` 7090B / `mod.rs` 106B） |
| 删除声明 | `crates/runtime/src/lib.rs:30` 的 `pub mod collaboration;` |

**执行前复验（当前源码，非沿用旧结论）**：
- `collaboration::` 路径引用：runtime 内 **0**、其他 crate **0**
- 全仓 `collaboration` 字样 17 命中，**逐条核对均为无关项**：英文注释散文
  （"multi-document collaboration"）、其他模块自有函数
  （`multi_agent_executor.rs` 的 `simulate_swarm_collaboration` 等）、
  `harness/domain_router.rs:654` 与 `xianyu_scanner.rs:108` 的关键字字符串
- 删除量：目录 4 文件

### 8.3 §7.3 的 `GCounter::apply_remote_increment` → **改名 `observe_remote` + 锁语义** ✅

**新证据推翻了我上轮「需产品语义决策」的判断**：该方法**全仓 0 调用点**（grep 只有定义自身）。
且其实现 `counters[site] = max(cur, value)` 与 `merge` 的**单站点特化逐字等价**
⇒ 取 `max` 是对的，**错的只是名字**（`increment` 暗示可累加）。

| 动作 | 内容 |
|---|---|
| 改名 | `apply_remote_increment` → **`observe_remote`**（与 `VersionVector::observe` 对齐语义） |
| 补 doc | 明确「取水位 / 幂等 / **不是** op-based 增量累加」，并指向 `increment` + op 去重 |
| 补 2 个回归测试 | ① `test_observe_remote_is_idempotent_and_never_regresses`（重复并入不翻倍、旧水位不回退、新水位可推进）② `test_observe_remote_equivalent_to_single_site_merge`（与单站点 `merge` 结果一致） |

零调用 ⇒ 改名为**零行为变更**；但两个新测试把语义钉死，防后续误改回 `+=`。

### 8.4 验证

| 项 | 命令 | 结果 |
|---|---|---|
| crdt 单测（含 2 个新测试） | `cargo test -p axagent-crdt` | ✅ **35 passed / 0 failed**（33 → 35） |
| runtime（验证 collaboration 删除） | `cargo check -p axagent-runtime --all-targets` | ✅ 1m26s，**0 error / 0 warning** |
| **主 crate（验证 8 条告警消失）** | `cargo check -p axagent --all-targets` | ✅ **退出码 0，11m14s，0 error / 0 warning**；**无 `generated N warnings` 行** ⇒ 8 条 `unused_imports` 全部消除 |
| 结构完整性 | 花括号 54/54 平衡；行数 663 → 666 | ✅（−2 −1 +1 +1 净 −1，再 +3 为 `voice` 处的防回归说明，最后 +1 为 E0585 相关说明） |
| 悬空文档注释静态扫描 | `.workbuddy/tmp/check_doc_orphans.py` | ✅ 0 处悬空 |

> `--all-targets` = CI 的口径（`ci.yml` 用 `--workspace --all-targets --all-features`），故此处 0 告警
> 意味着**CI 不再因这 8 条而红**（`-D warnings` 会把它们升级为错误）。

### 8.5 ⚠ 中途自伤一次：删除 item 后遗留悬空 `///` 文档注释（E0585）

第一次改完跑主 crate 编译，**报错**：

```
error: expected item after doc comment
   --> src\commands\error_code.rs:660:1
660 |   /// 镜像 `crates/harness/src/error_codes.rs::unity`，两处定义值必须一致。
    |   ^^^^^^^^^^^ this doc comment doesn't document anything
```

**根因**：`voice` / `paper` / `reading_list` / `unity` 四处的 `pub use` 是那一段**唯一的 item**
（原文件里它们各自只有「文档注释 + 一条 `pub use`」）。把 `pub use` 删掉后，前面的 `///`
就**没有可挂载的 item** ⇒ E0585。`unity` 恰好是文件最后一项，于是**硬报错**。

**更隐蔽的是另外三处**（voice / paper / reading_list 不报错）：`///` 会**静默串到下一个 item 的文档上**
—— 例如「论文/文献相关错误码」「阅读列表相关错误码」会被挂到后面的 `pub mod knowledge_source`。
**编译通过、文档错位**，属于「能过编译的错」。

**修法**：四处带头 `///` 一律降级为 `//`（块后已无 item），并在 `voice` 处补一条显式说明，
防止后人「好心」把它们改回 `///` 再次踩雷。

> **教训（新增）**：删除「带文档注释的 item」时**必须同步处置注释本身**。
> `///` 是**属性**（`#[doc]`）而**不是注释** —— 它必须有宿主 item，否则 E0585；
> 若后面还有别的 item，它会**静默劫持**该 item 的文档。
> **检法**：静态扫描「`///` 块之后的**下一有效行** = 文件尾 或 `}`」即命中
> （脚本 → `.workbuddy/tmp/check_doc_orphans.py`，本轮已跑：0 处悬空）。

