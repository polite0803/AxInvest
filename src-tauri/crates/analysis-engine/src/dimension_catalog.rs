// SPDX-License-Identifier: AGPL-3.0-only

//! 股票分析「维度定义」目录（批2）
//!
//! 把 `stock_orchestration.rs` 里硬编码的 `SubTask::new` 清单抽成配置：
//! `dimension_catalog.yaml` 编译期内嵌（`include_str!`），无运行时 IO。
//!
//! 每个维度是一个 `StockDimensionDef`：`{id, name, description, data_source, role, weight, enabled}`。
//! - `role` 即该维度映射到的 harness 标准节点（Agent agent_profile_id）。
//! - `enabled=false` 的维度不进入子图（启停）。
//! - `weight`、`data_source` 作为元数据保留，供进化/反思侧后续消费。
//!
//! 新增分析维度 = 在 YAML 加一条 + 提供对应 role 节点/工具，不动 adapter 核心；
//! 编排仍由 `DynamicSubGraph` 完成。

use serde::Deserialize;

use axagent_harness::SubTask;

/// 单一分析维度定义
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StockDimensionDef {
    /// 唯一标识（step_id / 节点 id）
    pub id: String,
    /// 可读维度名
    pub name: String,
    /// 维度说明
    pub description: String,
    /// 依赖的数据源（元数据）
    #[serde(default)]
    pub data_source: String,
    /// 映射的标准节点角色（harness Agent agent_profile_id）
    pub role: String,
    /// 权重（元数据，用于进化/反思优先级）
    #[serde(default = "default_weight")]
    pub weight: f32,
    /// 启停：false 时该维度不进入子图
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 前置依赖维度 id（可选，缺省由策略顺序推导）
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// 是否支持并行执行
    #[serde(default)]
    pub parallel: bool,
}

impl StockDimensionDef {
    /// 映射为 harness 标准 SubTask 节点
    pub fn to_subtask(&self) -> SubTask {
        let mut st = SubTask::new(
            self.id.clone(),
            self.name.clone(),
            self.description.clone(),
            self.role.clone(),
        )
        .with_dependencies(self.dependencies.clone());
        if self.parallel {
            st = st.with_parallel();
        }
        st
    }
}

/// 某策略（Pipeline / Debate）下的维度集合
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StrategyDimensionSet {
    /// 最大并行 worker 数
    #[serde(default = "default_max_parallel")]
    pub max_parallel: u32,
    /// 最大重规划轮数
    #[serde(default = "default_max_replans")]
    pub max_replans: u32,
    /// 维度清单（按配置顺序执行）
    pub dimensions: Vec<StockDimensionDef>,
}

/// 维度目录：Pipeline 与 Debate 两套策略的维度清单
#[derive(Debug, Clone, Deserialize)]
pub struct DimensionCatalog {
    pub pipeline: StrategyDimensionSet,
    pub debate: StrategyDimensionSet,
}

impl DimensionCatalog {
    /// 加载编译期内嵌的 YAML 维度目录
    pub fn embedded() -> Self {
        let yaml = include_str!("dimension_catalog.yaml");
        serde_yaml::from_str(yaml)
            .expect("dimension_catalog.yaml 维度目录配置非法，请检查 YAML 语法与字段")
    }

    /// 读取指定策略集（用于 `decompose_mission`）
    pub fn strategy_set(&self, is_debate: bool) -> &StrategyDimensionSet {
        if is_debate {
            &self.debate
        } else {
            &self.pipeline
        }
    }

    /// 该策略集下所有启用维度映射的 SubTask 列表
    pub fn enabled_sub_tasks(&self, is_debate: bool) -> Vec<SubTask> {
        self.strategy_set(is_debate)
            .dimensions
            .iter()
            .filter(|d| d.enabled)
            .map(StockDimensionDef::to_subtask)
            .collect()
    }
}

fn default_weight() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_max_parallel() -> u32 {
    2
}

fn default_max_replans() -> u32 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses() {
        let catalog = DimensionCatalog::embedded();
        // Pipeline 含 7 个维度，Debate 含 3 个维度
        assert_eq!(catalog.pipeline.dimensions.len(), 7);
        assert_eq!(catalog.debate.dimensions.len(), 3);
        // 关键分析维度存在
        assert!(
            catalog.pipeline.dimensions.iter().any(|d| d.id == "data_fetch" && d.enabled),
            "data_fetch 应存在且启用"
        );
        assert!(
            catalog.pipeline.dimensions.iter().any(|d| d.id == "decision_generation"),
            "decision_generation 应存在"
        );
    }

    #[test]
    fn enabled_sub_tasks_respects_weight_metadata() {
        let catalog = DimensionCatalog::embedded();
        let sub_tasks = catalog.enabled_sub_tasks(false);
        // 所有启用的 pipeline 维度都映射成 SubTask，顺序等于 YAML 顺序
        assert_eq!(sub_tasks.len(), catalog.pipeline.dimensions.len());
        assert_eq!(sub_tasks[0].id, "data_fetch");
        assert_eq!(sub_tasks[0].role, "data_agent");
        assert_eq!(sub_tasks[6].id, "decision_generation");
    }

    #[test]
    fn disabled_dimension_is_excluded() {
        let def = StockDimensionDef {
            id: "dummy".to_string(),
            name: "占位".to_string(),
            description: "测试".to_string(),
            data_source: "无".to_string(),
            role: "dummy_agent".to_string(),
            weight: 1.0,
            enabled: false, // 停用
            dependencies: vec![],
            parallel: false,
        };
        let set =
            StrategyDimensionSet { max_parallel: 1, max_replans: 1, dimensions: vec![def.clone()] };
        let catalog = DimensionCatalog {
            pipeline: set.clone(),
            debate: StrategyDimensionSet { max_parallel: 1, max_replans: 1, dimensions: vec![] },
        };
        // 全部维度停用 ⇒ 不产出任何 SubTask
        assert!(catalog.enabled_sub_tasks(false).is_empty());
        // 但底料仍保留在配置里
        assert_eq!(set.dimensions.len(), 1);
    }

    #[test]
    fn debate_sub_tasks_built() {
        let catalog = DimensionCatalog::embedded();
        let sub_tasks = catalog.enabled_sub_tasks(true);
        assert_eq!(sub_tasks.len(), 3);
        assert_eq!(sub_tasks[0].id, "bull_analyst");
        assert_eq!(sub_tasks[2].id, "arbitrator");
    }
}
