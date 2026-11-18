# AUDIT — P6.3 其余功能包启动基线（2026-11-18）

> 状态：active ｜ 日期：2026-11-18 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md` §0.6.3、§0.8、§6.1、§1003
> 性质：时点快照。只记录盘上机械取证，不把尚未迁移的业务面写成已完成。

## 一、结论

P6.3 尚未开始真实功能包迁出。当前只确认 `WorkflowNode` 开放化前置已闭合：节点 descriptor 注册表既有生产消费方，也有前端清单命令；工作流编辑器与知识库/Wiki 仍是宿主业务面，不能靠新增插件骨架宣告完成。

## 二、已闭合前置的取证

- `axagent-core/crates/harness/src/workflow_node_dyn.rs`：`WorkflowNodeDescriptor` + `NodeDescriptorRegistry`。
- `axagent-core/crates/harness/src/workflow_node_descriptors.rs`：32 个内置 descriptor，`register_node_descriptor` 提供外部注册口。
- `axagent-runtime/crates/rt-workflow/src/work_engine/node_executor_trait.rs:239` 与 `engine/schema_validator.rs:106`：生产消费 descriptor。
- `src-tauri/src/commands/workflow_node_descriptors.rs:19`：`list_workflow_node_descriptors` 暴露清单；`src/stores/feature/workflowEditorStore.ts:815` 真调用。

⇒ §0.8.3-1 的“封闭 enum 阻塞新增节点”在消费侧已有动态注册表通道；后续新增功能包可以贡献 descriptor，不必先改主壳 8 处。

## 三、仍未迁出的业务面

1. **工作流编辑器**：`src/components/workflow/` 132 个前端文件、`WorkflowEditor.tsx` 2660 行、三个大 store；83 条审计域命令仍由宿主命令面承接。
2. **知识库 / Wiki**：`src-tauri/src/commands/{knowledge,knowledge_graph,knowledge_source,llm_wiki,sources,unified_knowledge,wiki}.rs` 仍在宿主；`WikiDetailPanel.tsx:178` 仍有裸 `invoke("sync_note_to_knowledge_base")`。
3. 计划 §0.6.3 的 C 层口径仍是 131 条命令 / 11,827 行、阻塞面 72 处 / 25 文件；本次没有重跑该历史口径，故不宣称新读数。

## 四、为什么本轮不新建功能包骨架

- 新建空 worker / 空 manifest 只能降低 grep 或制造“有插件”的假象，不能把用户可见页面、命令、数据面迁出，违反 P6.3 的 L1 五面判据。
- `sync_note_to_knowledge_base` 目前直接读写宿主 DB 与文件，现有 8 条远程接缝没有对应的“note / knowledge document”写入口；必须先新增宿主接缝并落实权限，再让插件命令承接。
- 工作流编辑器依赖 `WorkEngine` 与大量宿主 store；P6.0-① 只解决了节点类型开放化，未解决编辑器整体装载与命令面迁移。

## 五、下一批的可执行入口

优先为 Wiki 同步链新增一条宿主写接缝（note → markdown 临时文件 → knowledge document），把 `sync_note_to_knowledge_base` 的命令体下沉为 provider；前端改走插件命名命令并补 browserMock。该切片有明确单一调用点，可在不复制实现的前提下验证 P6.3 的接缝模板。
