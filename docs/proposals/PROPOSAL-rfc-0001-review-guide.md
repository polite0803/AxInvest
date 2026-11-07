> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# RFC-0001 评审指南

**适用范围**：[RFC-0001-stream-runtime.md](./rfc-0001-stream-runtime.md)
**设计文档**：[stream-architecture.md](./stream-architecture.md)（已 APPROVED）
**目标读者**：AxAgent 核心开发团队（Rust + TS 双向栈）
**预计评审时长**：1.5–2 小时（异步）/ 1 次 60 分钟会议

---

## 1. 评审目标

让团队在动手写代码前达成 3 个共识：

1. **方向对不对** —— "流式执行模型"是否真的是当前 28 节点底座的最佳增量路径
2. **细节够不够** —— RFC 是否消除了所有模糊点，能直接进入 Sprint 1
3. **风险看清了没** —— 引擎双写 / 锁顺序 / 命名冲突 / YAGNI 边界是否经得起追问

---

## 2. 评审前必读（建议 30 分钟）

| 阅读顺序 | 文档 | 重点 |
|---------|------|------|
| 1 | [stream-architecture.md §3.1 核心抽象 + §3.4 5 锚点](./stream-architecture.md) | 理解"流式执行模型"的心智模型 |
| 2 | [rfc-0001-stream-runtime.md §3 详细设计](./rfc-0001-stream-runtime.md) | 看完整 trait 签名 + 迁移方案 |
| 3 | [评审材料 §2 现有节点元数据改造示例](./rfc-0001-review-materials.md) | 看到 RFC 的代码长什么样 |
| 4 | `src-tauri/crates/rt-workflow/src/work_engine/node_executor_trait.rs` | 理解现有 28 节点的实际载体 |
| 5 | `src/components/workflow/Nodes/BaseNode.tsx` | **重要：这是 React 函数组件，不是类** |

---

## 3. 评审会议程（60 分钟）

| 时段 | 议题 | 主持人 | 输出 |
|------|------|--------|------|
| 0-5 min | 背景回顾：28 节点现状 + 痛点 | 作者 | 共识"现状" |
| 5-15 min | 方案 A/B/C 走查（重点 C 的 4 个原语） | 作者 | 共识"方向" |
| 15-35 min | RFC 逐节评审（§3-§6） | 全员 | 决策点决议 |
| 35-50 min | 风险/边界讨论（双写、锁、命名冲突、YAGNI） | 全员 | 风险清单 |
| 50-60 min | 决议 + 责任人 + 时间线 | 作者 | 行动项 |

---

## 4. 关键决策点（必须达成明确结论）

### 决策 1：方向是否锁定"流式执行模型"？

- **选择 A**：通过 RFC-0001，立即启动 Sprint 1
- **选择 B**：保留方向，但 RFC 细节需修订（列出具体修订点）
- **选择 C**：拒绝 RFC，回退到 Approach B（事件总线方案）

**决策依据**：
- 流式执行模型是否能服务 28 节点底座的全部 5 类覆盖探针（反应式实时 / 异步长时 / 并行分布 / 决策驱动 / 数据流协同）？
- 双写迁移的成本（2x IO）是否可接受？
- "节点输出即流"的心智模型切换对用户的认知负担？

### 决策 2：引擎双写迁移方案 A（推荐）vs B

- **方案 A**：`results` 保留 + `stream_log` 新增，双写（推荐，Sprint 4 后评估 GC）
- **方案 B**：直接 `results` → `Vec<StreamEvent>` 迁移

**决策依据**：
- 双写 IO 开销测量结果（**前置条件**：RFC §5.3 性能基线未做，需要 Sprint 1.8 跑出）
- `results` 字段的所有现有依赖方（搜索 `workflow.results` 看有多少处读）
- 故障模式可接受度（双写一致性 vs 一次性迁移风险）

### 决策 3：订阅协议边界（进程内 vs 跨进程）

- **选择 A**：进程内 `InProcessBus` + 跨进程 Tauri 事件（RFC 当前方案）
- **选择 B**：抽象异构后端（Kafka/SSE/NATS）—— 已被 RFC 拒绝，但需确认无反对意见

**决策依据**：
- Tauri 内置事件系统是否能覆盖所有跨进程场景？
- 是否有"未来需要接入 Kafka/NATS"的具体场景？

### 决策 4：Sprint 1 验收标准是否可执行？

- 单元测试 ≥ 50 个是否够？
- 性能基线 P99 < 10% 是否可达（需 Sprint 1.8 验证）？
- i18n 11 语言、Tauri 命令两步注册是否齐备？

### 决策 5：Stream 元数据载体 —— TS 还是 Rust？

**重要纠偏**：RFC §3.4 当前描述把 `produces` / `consumes` 放在 TS 的 `BaseNode.tsx`，但 **BaseNode.tsx 是 React 函数组件，不是类**。实际元数据载体应是 Rust `NodeExecutorTrait`（每个 impl 加 `stream_meta()` 方法）。

- **选择 A**（推荐）：Rust `NodeExecutorTrait::stream_meta()` + TS 侧通过 IPC 拉取元数据
- **选择 B**：TS 侧建独立的 `NODE_STREAM_META_MAP`（与 `NODE_TYPE_MAP` 平行），TS 与 Rust 双重维护
- **选择 C**：codegen（Rust trait 单一源 + ts-rs 生成 TS 类型）

---

## 5. 必须追问的 7 个问题

评审中**每个人**至少要追问以下问题之一：

1. **双写一致性**：节点执行成功但 `stream_log` 写入失败时，状态是什么？重试语义？
2. **锁顺序**：`workflows → executions → node_breakers → cancel_tokens → stream_subs` 这个顺序是**所有**新代码必须遵守的吗？如何静态保证？
3. **跨工作流订阅生命周期**：被订阅方未启动时，订阅进入 Pending，但 Pending 状态持续多久算"超时"？
4. **运行期 schema 校验开销**：丢弃策略 + metric 是否真的不影响主路径？测试覆盖？
5. **命名冲突**：`WorkflowStream` vs `ChatStream` 在 i18n 文本中如何区分（`workflow.stream` vs `chat.stream`）？
6. **决策点 vs 副作用节点的语义边界**：一个 LLM 节点既调外部 API（副作用）又有 prompt 模板（决策），应该归类哪个？还是多角色并存？
7. **Sprint 4 GC `results` 的回退路径**：如果 GC 后发现某个隐式依赖 `results` 的旧逻辑出问题，回滚成本多大？

---

## 6. 评审产出物

会后 24 小时内产出：

1. **决议纪要**（RFC-0001 文件末追加 §11 评审记录）
2. **修订点清单**（如有）：具体到行号 / 节号
3. **责任分工表**：
   - 谁负责 Sprint 1.1 stream_types crate
   - 谁负责节点元数据改造
   - 谁负责 Tauri 命令注册
   - 谁负责 i18n 11 语言
4. **风险登记**：每个风险 1 行 + 缓解 owner

---

## 7. 评审通过标准

满足以下**全部**才视为通过：

- [ ] 决策 1-5 全部有结论（A/B/C 之一）
- [ ] 7 个追问问题**至少 5 个**有书面答复
- [ ] Sprint 1 责任分工到人
- [ ] 无未解决的高风险项（高风险 = 阻塞 Sprint 1 启动）

**未达标准** → 重新评审或 RFC 回炉。

---

## 8. 评审中常见反对意见预案

### 反对意见 1："流式执行模型是过度抽象，28 节点用不上"

**回应**：Sprint 1 不会强制所有 28 节点都用流，**默认零行为变化**。`stream_log` 与 `results` 双写，前端默认读 `results`（兼容旧视图），流能力**按需启用**（Sprint 2 才加 StreamInspector UI）。

### 反对意见 2："双写 IO 开销 2x 不可接受"

**回应**：Sprint 1.8 性能基线**前**不做判断。`stream_log` 写是 best-effort（不阻塞主路径），且 ring buffer 默认 1000 条。如果基线 P99 > 10%，立刻回滚到方案 B（一次性迁移）—— 这是 RFC §5.3 已写明的退出条件。

### 反对意见 3："前端 100+ 行元数据改不动"

**回应**：实际改的是 **Rust 28 个 executor**，每个加一个 `stream_meta()` 方法（默认实现为空，impl 需要时 override）。**预计 28 个文件 × 3-5 行 = 100 行左右**。TS 侧 0 改动（Stream 元数据从 Rust 通过 IPC 拉取）。

### 反对意见 4："命名会一直冲突"

**回应**：`WorkflowStream` / `ChatStream` 前缀 + `stores/domain/workflowStreamStore.ts` vs `stores/domain/streamStore.ts` 目录隔离。RFC §3.6 已有完整方案。

### 反对意见 5："RFC 太长/太短/不够详细"

**回应**：RFC 是"工程规范级详细"，目的是让工程师**不查代码也能动手**。如果评审认为太短，说明 RFC 没尽到职责；太长，说明抽象层级没拿捏好——根据反馈调整。

---

## 9. 评审会议召集建议

- **预审材料**：本文档 + 评审材料（task 3 产物）+ RFC 全文
- **会前阅读**：30 分钟
- **会议时长**：60 分钟
- **纪要 owner**：RFC 作者本人
- **后续动作**：会后 24h 内发出纪要 + 修订 RFC（如需）

---

## 10. 关键文件路径速查

| 文件 | 用途 |
|------|------|
| [stream-architecture.md](./stream-architecture.md) | 设计文档（已 APPROVED） |
| [rfc-0001-stream-runtime.md](./rfc-0001-stream-runtime.md) | 待评审 RFC |
| [rfc-0001-review-materials.md](./rfc-0001-review-materials.md) | 评审材料（含节点改造示例 + FAQ） |
| [sprint-1-plan.md](../plans/2026-06-06-stream-runtime-sprint-1.md) | Sprint 1 TDD 任务（RFC 通过后启动） |
| `src-tauri/crates/rt-workflow/src/work_engine/node_executor_trait.rs` | 28 节点元数据载体（重要纠偏） |
| `src/components/workflow/Nodes/BaseNode.tsx` | **React 函数组件**（非类，RFC §3.4 需修正） |

---

## 11. 评审记录（会后填）

```
日期: YYYY-MM-DD
参会人: [list]
决议: [通过 / 修订后通过 / 拒绝]
修订点: [编号列表]
责任分工: [table]
下次评审日期: [date]
```

---

> **核心原则**：评审不是"挑刺"，是"降低 Sprint 1 走偏的概率"。每个追问都是礼物。
