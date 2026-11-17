// SPDX-License-Identifier: AGPL-3.0-only

//! 工作流节点的动态外壳与类型注册模型（P6.0-① 阶段1）。
//!
//! 与封闭 enum `WorkflowNode` **共存**：本模块不替换 enum，而是提供一条并行的
//! 「类型字符串 + 契约声明 + 运行期注册表」通道。阶段 2–6 再逐步把各消费点
//! 从 enum 迁到本模型，最终删除 enum。
//!
//! 设计约束（见 `PROPOSAL-workflow-node-open-registry.md` §3）：
//! - 序列化形状必须与 enum 逐字一致：`{"type":"<tag>", ...base, "config":{...}}`；
//! - 强类型 config 在 descriptor 边界解析（`parse_config`），执行器内部仍是强类型；
//! - 新增节点类型 = 实现 descriptor + 调用 `register`，主壳零改。

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workflow_types::{NodeKind, WorkflowNode, WorkflowNodeBase};

/// 工作流节点的运行期统一表示（动态外壳）。
///
/// `type` 取原 enum 的 serde tag（camelCase，如 `agent` / `httpRequest`）；
/// `base` 以 `#[serde(flatten)]` 展开，保持与 envelope 同层；
/// `config` 保留类型专属 JSON，布局/持久化时无损透传。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNodeDyn {
    /// 节点类型字符串；对应原 enum 的 serde tag。
    /// 必须显式 rename 为 `type`（而非 camelCase 默认的 `nodeType`），
    /// 才能与既有 32 变体的 `{"type":"..."}` 序列化形状逐字一致。
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub node_type: String,
    /// 节点通用字段（id / title / position / retry / enabled …）。
    #[serde(flatten)]
    pub base: WorkflowNodeBase,
    /// 类型专属配置原始 JSON（无损保留原 config 结构体的 JSON 形状）。
    pub config: Value,
}

/// 一个节点类型的契约声明：类型标识 + 分类 + config 解析 + 输出变量定位。
///
/// 新增节点类型时实现本 trait 并注册即可，无需改 `WorkflowNode` enum
/// 或任何穷举 match（阶段 2–6 完成后成立）。
pub trait WorkflowNodeDescriptor: Send + Sync {
    /// 类型字符串，与序列化 tag 一致（如 `agent` / `httpRequest`）。
    fn type_name(&self) -> &'static str;

    /// 节点高级分类（供引擎调度与前端配色）。
    fn kind(&self) -> NodeKind;

    /// 校验/解析 config：成功返回 `()`，失败返回可读错误。
    /// 执行器在 `execute()` 入口用同一 config 类型 `from_value` 还原强类型。
    fn parse_config(&self, raw: &Value) -> Result<(), String>;

    /// config 的 JSON Schema（供 schema_validator 与前端表单生成）。
    /// 默认空对象，由具体 descriptor 覆盖。
    fn config_schema(&self) -> Value {
        serde_json::json!({})
    }

    /// 从 config 中提取 output_var（统一返回 owned String，规避生命周期差异）。
    /// 无 output_var 字段的节点类型返回 `None`。
    fn output_var(&self, raw: &Value) -> Option<String>;

    /// 展示配色分类（P6.0-① 阶段5）。
    ///
    /// 与 `kind()` 是**两个维度**：`kind()` 是引擎调度分类，`display_kind()`
    /// 是编辑器配色 / 容器判定分类。二者在 32 个内置类型中 11 个不同
    /// （如 `aggregator` 引擎=Tool、展示=Container）。默认等于 `kind()`，
    /// 内置类型由 `BUILTIN_DISPLAY_KIND` 映射表覆盖，外部类型可自行实现。
    fn display_kind(&self) -> NodeKind {
        self.kind()
    }

    /// 左侧节点面板分组（P6.0-① 阶段5）。默认 `"flow"`。
    fn display_category(&self) -> &'static str {
        "flow"
    }

    /// 供前端按清单渲染的展示元数据（P6.0-① 阶段5）。
    ///
    /// 默认从 `type_name` / `display_kind` / `display_category` 公式派生，
    /// 与前端原 `NODE_KIND_HUE` / `workflow.nodeTypes.${type}` 逐字对齐，
    /// 插件类型只需保证 i18n key 存在即可零改主壳。
    fn display_meta(&self) -> WorkflowNodeDescriptorDto {
        let display_kind = self.display_kind();
        let hue = match display_kind {
            NodeKind::Input => "yellow",
            NodeKind::Output => "red",
            NodeKind::Tool => "green",
            NodeKind::Agent => "blue",
            NodeKind::Condition => "orange",
            NodeKind::Loop => "purple",
            NodeKind::Container => "cyan",
            NodeKind::Storage => "pink",
        };
        let is_container =
            matches!(display_kind, NodeKind::Container) || self.type_name() == "loop";
        WorkflowNodeDescriptorDto {
            type_name: self.type_name().to_string(),
            kind: self.kind(),
            display_kind,
            label_key: format!("workflow.nodeTypes.{}", self.type_name()),
            category: self.display_category().to_string(),
            hue: hue.to_string(),
            is_container,
            config_schema: self.config_schema(),
        }
    }
}

/// 节点类型的展示元数据 DTO（P6.0-① 阶段5，供清单命令下发前端）。
///
/// 字段 snake_case，经 `#[serde(rename_all = "camelCase")]` 输出 camelCase，
/// 与前端 `src/types/workflow.ts` 的 `WorkflowNodeDescriptorDto` 对齐（禁区13）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNodeDescriptorDto {
    /// 类型字符串，与序列化 tag 一致。
    pub type_name: String,
    /// 引擎调度分类（保留给引擎侧；前端配色不读此字段）。
    pub kind: NodeKind,
    /// 展示配色分类（与前端原 NODE_KIND_MAP 逐字等价）。
    pub display_kind: NodeKind,
    /// i18n key，形如 `workflow.nodeTypes.<type>`。
    pub label_key: String,
    /// 左侧面板分组 id。
    pub category: String,
    /// 类目色相（与前端 CategoricalHue 对齐）。
    pub hue: String,
    /// 是否容器节点（决定自动挂入 / 包围盒行为）。
    pub is_container: bool,
    /// config 参数 schema（当前内置类型为空 `{}`）。
    pub config_schema: Value,
}

/// 节点类型契约注册表（foundation 层，零 axagent 依赖）。
///
/// 用 `std::sync::RwLock` 持有 map：注册发生在启动期，读取是高频但只读，
/// 且临界区不跨 await（ trait 方法均同步）⇒ 同步锁语义正确。
// SAFETY: 本注册表的 std::sync::RwLock 仅在同步临界区使用，guard 不跨 await
// （trait 方法均同步，注册发生在启动期、读取为只读），无死锁 / 毒化风险。
#[allow(clippy::disallowed_types)]
pub struct NodeDescriptorRegistry {
    entries: std::sync::RwLock<HashMap<String, Arc<dyn WorkflowNodeDescriptor>>>,
}

impl Default for NodeDescriptorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeDescriptorRegistry {
    // SAFETY: 同上，构造时的 RwLock 不跨 await。
    #[allow(clippy::disallowed_types)]
    pub fn new() -> Self {
        Self { entries: std::sync::RwLock::new(HashMap::new()) }
    }

    /// 注册一个节点类型契约；同名重复注册会被忽略（保留先注册者）。
    pub fn register<D: WorkflowNodeDescriptor + 'static>(&self, descriptor: D) {
        let mut map = self.entries.write().expect("NodeDescriptorRegistry 写锁被毒化");
        map.entry(descriptor.type_name().to_string()).or_insert_with(|| Arc::new(descriptor));
    }

    /// 注册一个共享 Arc（供框架侧预置 descriptor）。
    pub fn register_arc(&self, descriptor: Arc<dyn WorkflowNodeDescriptor>) {
        let mut map = self.entries.write().expect("NodeDescriptorRegistry 写锁被毒化");
        map.entry(descriptor.type_name().to_string()).or_insert(descriptor);
    }

    /// 按类型名查询契约。
    pub fn get(&self, type_name: &str) -> Option<Arc<dyn WorkflowNodeDescriptor>> {
        self.entries.read().expect("NodeDescriptorRegistry 读锁被毒化").get(type_name).cloned()
    }

    /// 已注册类型名列表（顺序不保证，供 dispatcher 枚举）。
    pub fn type_names(&self) -> Vec<String> {
        self.entries.read().expect("NodeDescriptorRegistry 读锁被毒化").keys().cloned().collect()
    }

    /// 已注册 descriptor 列表（供遍历自动注册 executor）。
    pub fn descriptors(&self) -> Vec<Arc<dyn WorkflowNodeDescriptor>> {
        self.entries.read().expect("NodeDescriptorRegistry 读锁被毒化").values().cloned().collect()
    }
}

impl WorkflowNodeDyn {
    /// 从动态外壳的 config 还原成强类型 config（执行器入口用）。
    pub fn parse_config<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_value(self.config.clone())
            .map_err(|e| format!("节点类型 `{}` 的 config 解析失败: {e}", self.node_type))
    }
}

// ---------------------------------------------------------------------------
// 与封闭 enum 的互转（共存期桥接，阶段 6 删除 enum 时一并移除）
// ---------------------------------------------------------------------------

impl TryFrom<&WorkflowNode> for WorkflowNodeDyn {
    type Error = String;

    fn try_from(node: &WorkflowNode) -> Result<Self, Self::Error> {
        let value = serde_json::to_value(node).map_err(|e| e.to_string())?;
        let node_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "WorkflowNode 缺少 type 字段".to_string())?
            .to_string();
        let config = value.get("config").cloned().unwrap_or_else(|| serde_json::json!({}));
        // base = envelope 去掉 type/config 后的剩余字段
        let mut base_value = value.clone();
        if let Some(obj) = base_value.as_object_mut() {
            obj.remove("type");
            obj.remove("config");
        }
        let base: WorkflowNodeBase = serde_json::from_value(base_value)
            .map_err(|e| format!("WorkflowNodeBase 还原失败: {e}"))?;
        Ok(Self { node_type, base, config })
    }
}

/// 将动态外壳转回封闭 enum（共存期用；未知类型如实报错）。
impl TryFrom<&WorkflowNodeDyn> for WorkflowNode {
    type Error = String;

    fn try_from(dyn_node: &WorkflowNodeDyn) -> Result<Self, Self::Error> {
        let mut envelope = serde_json::to_value(&dyn_node.base).map_err(|e| e.to_string())?;
        let obj = envelope
            .as_object_mut()
            .ok_or_else(|| "WorkflowNodeBase 序列化结果不是对象".to_string())?;
        obj.insert("type".to_string(), Value::String(dyn_node.node_type.clone()));
        obj.insert("config".to_string(), dyn_node.config.clone());
        serde_json::from_value(envelope)
            .map_err(|e| format!("未知或非法节点类型 `{}`: {e}", dyn_node.node_type))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_types::*;

    fn sample_base() -> WorkflowNodeBase {
        serde_json::from_value(serde_json::json!({
            "id": "n1", "title": "节点", "position": {"x": 0, "y": 0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true
        }))
        .unwrap()
    }

    /// 共存桥接：enum → 动态外壳 → enum 往返后，序列化字节必须逐字不变。
    #[test]
    fn enum_dyn_roundtrip_preserves_json() {
        let node: WorkflowNode = serde_json::from_value(serde_json::json!({
            "type": "storage", "id": "s1", "title": "存储",
            "position": {"x": 0, "y": 0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {"backend": "sqlite", "operation": "upsert", "input_var": "v", "collection": "c"}
        }))
        .unwrap();
        let original = serde_json::to_value(&node).unwrap();
        let dyn_node = WorkflowNodeDyn::try_from(&node).unwrap();
        let back = WorkflowNode::try_from(&dyn_node).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), original);
    }

    #[test]
    fn dyn_parse_config_restores_strong_type() {
        let dyn_node = WorkflowNodeDyn {
            node_type: "storage".to_string(),
            base: sample_base(),
            config: serde_json::json!({
                "backend": "sqlite", "operation": "upsert", "input_var": "v", "collection": "c"
            }),
        };
        let cfg: StorageNodeConfig = dyn_node.parse_config().unwrap();
        assert_eq!(cfg.backend, "sqlite");
        assert_eq!(cfg.collection, "c");
    }

    #[test]
    fn unknown_type_roundtrip_error_is_explicit() {
        let dyn_node = WorkflowNodeDyn {
            node_type: "brandNewThirdPartyType".to_string(),
            base: sample_base(),
            config: serde_json::json!({}),
        };
        assert!(WorkflowNode::try_from(&dyn_node).is_err());
    }

    /// 阶段2：32 个内置 type 标签逐个验证「enum JSON → 动态外壳」无损。
    ///
    /// 这里只断言外壳这一步：config 原样保留、base 字段集合不变、type 正确。
    /// 「外壳 → enum」的完整往返要求每个 config 的最小合法实例，阶段0 审计
    /// §4 已把该项后移到阶段6（届时 enum 删除，Value 直接成为权威，比对更有意义）。
    #[test]
    fn all_32_builtin_types_wrap_losslessly() {
        let registry = crate::workflow_node_dyn::NodeDescriptorRegistry::new();
        crate::workflow_node_descriptors::register_builtin_descriptors(&registry);
        let names = registry.type_names();
        assert_eq!(names.len(), 32);
        for name in names {
            // 动态外壳的 Deserialize 不解析 config 内容，故可用同一最小 payload
            // 覆盖全部 32 个 type 标签，逐个验证平坦化后字段无损。
            let mut envelope = serde_json::to_value(sample_base()).unwrap();
            envelope
                .as_object_mut()
                .unwrap()
                .insert("type".into(), Value::String(name.clone()));
            envelope
                .as_object_mut()
                .unwrap()
                .insert("config".into(), serde_json::json!({"probe": 1}));
            let dyn_node: WorkflowNodeDyn = serde_json::from_value(envelope).unwrap();
            assert_eq!(dyn_node.node_type, name);
            assert_eq!(dyn_node.config, serde_json::json!({"probe": 1}));
            assert_eq!(dyn_node.base.id, "n1");
            // 锁住序列化形状：动态外壳自身输出必须是 `type`（不是 nodeType），
            // 且与 enum 的 tagged 布局同层。阶段6 删除 enum 后此断言即契约本身。
            let out = serde_json::to_value(&dyn_node).unwrap();
            assert_eq!(out.get("type").and_then(|v| v.as_str()), Some(name.as_str()));
            assert!(out.get("nodeType").is_none(), "不得输出 nodeType");
            assert_eq!(out.get("config"), Some(&serde_json::json!({"probe": 1})));
        }
    }
}
