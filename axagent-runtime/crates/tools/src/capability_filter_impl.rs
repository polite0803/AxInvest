// SPDX-License-Identifier: AGPL-3.0-only
//! 能力过滤器实现 — 8 维硬性闸门 + 可注册策略裁剪（Phase 3 策略对象化）
//!
//! 复用 harness 层 CapabilityFilter trait 的默认实现，
//! 提供可注入的结构体实例。所有 8 个维度的检查逻辑在 harness 层定义。
//!
//! # 策略对象化（Phase 3）
//! 注入 DB 连接后，`check_all` 前置执行可注册策略（capability_policies 表）的
//! 排除规则裁剪（exclude_domains / exclude_tags / exclude_capability_ids），
//! 与 8 维硬编码闸门不冲突（策略是环境性裁剪，闸门是能力自身硬约束）。
//! 未注入 DB / 无启用策略时行为与既有版本完全一致。
//!
//! # 域启用闸门（P2，2026-09-15）
//! `check_all` 的**第一个**检查是 `check_domain_enabled`：读 harness 的**进程内**
//! 域覆盖层，把「被停用域」的能力全部裁掉（维度 `DomainEnabled`）。
//! 它与策略通道的区别：域停用是**硬闸门**（不可被另一条策略捞回），
//! 且**零 IO**（覆盖层在启动时加载、写命令后刷新），故放在策略查询之前。

use async_trait::async_trait;
use axagent_harness::{
    CapabilityFilter, CapabilityPassportDto, FilterContext, FilterDecision, FilterDimension,
};
use sea_orm::DatabaseConnection;

/// 能力过滤器实现
///
/// 所有 8 个维度的检查方法均来自 harness 层默认实现，
/// 可按需 override 特定维度以实现自定义逻辑。
#[derive(Debug, Default, Clone)]
pub struct CapabilityFilterImpl {
    /// 是否启用维度二（记忆/状态）检查
    pub enable_memory_state: bool,
    /// 可选 DB 连接：注入后启用可注册策略裁剪（Phase 3 策略对象化）
    pub db: Option<DatabaseConnection>,
}

impl CapabilityFilterImpl {
    pub fn new() -> Self {
        Self { enable_memory_state: true, db: None }
    }

    pub fn with_memory_state(mut self, enabled: bool) -> Self {
        self.enable_memory_state = enabled;
        self
    }

    /// 注入 DB 连接，启用可注册策略裁剪（capability_policies 表）。
    pub fn with_db(mut self, db: DatabaseConnection) -> Self {
        self.db = Some(db);
        self
    }

    /// 策略排除规则裁剪（Phase 3 策略对象化，前置执行）。
    ///
    /// 遍历启用策略，护照命中任一排除规则（域 / 标签 / 能力 ID）即拒绝。
    /// 加载失败（DB 错误 / 规则 JSON 损坏）仅记日志放行，保证不因策略层故障阻断检索。
    async fn check_policies(&self, passport: &CapabilityPassportDto) -> FilterDecision {
        let Some(db) = &self.db else { return FilterDecision::Pass };
        let policies = match axagent_dao::repo::capability_policy::list_enabled(db).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("[capability] 加载策略失败（放行）: {e}");
                return FilterDecision::Pass;
            },
        };
        if policies.is_empty() {
            return FilterDecision::Pass;
        }

        let domain_str = passport.domain.as_str();
        for policy in &policies {
            let rules = &policy.rules;
            if rules.exclude_domains.iter().any(|d| d.eq_ignore_ascii_case(domain_str)) {
                return FilterDecision::Reject {
                    reason: format!("策略「{}」排除域 {} ", policy.name, domain_str),
                    dimension: FilterDimension::Policy,
                };
            }
            if passport
                .tags
                .iter()
                .any(|t| rules.exclude_tags.iter().any(|rt| rt.eq_ignore_ascii_case(t)))
            {
                return FilterDecision::Reject {
                    reason: format!("策略「{}」排除标签", policy.name),
                    dimension: FilterDimension::Policy,
                };
            }
            if rules.exclude_capability_ids.iter().any(|id| id == &passport.capability_id) {
                return FilterDecision::Reject {
                    reason: format!("策略「{}」排除能力 {}", policy.name, passport.capability_id),
                    dimension: FilterDimension::Policy,
                };
            }
        }
        FilterDecision::Pass
    }
    /// 域启用闸门（P2 消费端③）：被停用的域，其能力**一律裁剪**。
    ///
    /// # 为什么这是硬闸门而不是策略
    ///
    /// 停用域是「这个域现在不该被用」，不是「按某条规则排除掉一些候选」：
    /// 若做成策略，用户可能再加一条策略把它捞回来 —— 而域已经被停用了，
    /// 那种组合没有语义。故它读的是域自身的启用位（`is_domain_enabled`），
    /// 与 L1 分类器 prompt / L1 路由**同一判据**。
    ///
    /// # 读的是**进程内**覆盖层，不是 DB
    ///
    /// 覆盖层在启动时加载、在 `update_capability_domain` 后刷新（见 DAO 的 `load_into_runtime`），
    /// 故本处是纯内存读、零 IO。刻意不在这里查库：本函数在每个候选上都会被调用，
    /// 而策略那条通道（`check_policies`）已经证明了「每候选一次 DB 往返」的代价。
    fn check_domain_enabled(&self, passport: &CapabilityPassportDto) -> FilterDecision {
        if axagent_harness::is_domain_enabled(passport.domain) {
            FilterDecision::Pass
        } else {
            FilterDecision::Reject {
                reason: format!("域 {} 已停用（能力域覆盖层）", passport.domain.as_str()),
                dimension: FilterDimension::DomainEnabled,
            }
        }
    }
}

#[async_trait]
impl CapabilityFilter for CapabilityFilterImpl {
    /// 维度一：置信度检查
    ///
    /// 此维度在 Ranker 阶段通过分差检测实现，
    /// 过滤阶段默认通过（仅做硬闸门检查）
    async fn check_all(
        &self,
        passport: &CapabilityPassportDto,
        ctx: &FilterContext,
    ) -> FilterDecision {
        // ── P2 域启用闸门（最先执行）──
        //
        // 放在最前的理由：它读的是**进程内**覆盖层（零 IO），而下面的策略裁剪要查库。
        // 停用域的能力占候选的一部分时，先拦掉它们能省掉那次查询；
        // 更重要的是**语义**：域都停用了，就没必要再问「策略是否也排除它」。
        if let FilterDecision::Reject { reason, dimension } = self.check_domain_enabled(passport) {
            return FilterDecision::Reject { reason, dimension };
        }

        // 策略前置裁剪（Phase 3 策略对象化）：可注册排除规则，优先于 8 维硬闸门
        if let FilterDecision::Reject { reason, dimension } = self.check_policies(passport).await {
            return FilterDecision::Reject { reason, dimension };
        }

        // 执行所有硬闸门维度检查
        // 维度三：模态
        if let FilterDecision::Reject { reason, dimension } =
            self.check_modality(passport, ctx).await
        {
            return FilterDecision::Reject { reason, dimension };
        }

        // 维度四：安全/合规
        if let FilterDecision::Reject { reason, dimension } =
            self.check_security(passport, ctx).await
        {
            return FilterDecision::Reject { reason, dimension };
        }

        // 维度五：资源/成本
        if let FilterDecision::Reject { reason, dimension } =
            self.check_resource_cost(passport, ctx).await
        {
            return FilterDecision::Reject { reason, dimension };
        }

        // 维度六：交互策略
        if let FilterDecision::Reject { reason, dimension } =
            self.check_interaction(passport, ctx).await
        {
            return FilterDecision::Reject { reason, dimension };
        }

        // 维度七：规划复杂度
        if let FilterDecision::Reject { reason, dimension } =
            self.check_planning_complexity(passport, ctx).await
        {
            return FilterDecision::Reject { reason, dimension };
        }

        // 维度八：实验/灰度
        if let FilterDecision::Reject { reason, dimension } =
            self.check_experiment_group(passport, ctx).await
        {
            return FilterDecision::Reject { reason, dimension };
        }

        // 维度十：前提条件匹配（P1：Skill preconditions）
        if let FilterDecision::Reject { reason, dimension } =
            self.check_preconditions(passport, ctx).await
        {
            return FilterDecision::Reject { reason, dimension };
        }

        FilterDecision::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::{CapabilityDomain, DomainOverride};

    /// 用例结束（**含 panic 展开**）时恢复覆盖层。
    ///
    /// ⚠ 覆盖层是**进程级全局**：若只在用例末尾手动 `clear_domain_overrides()`，
    /// 则断言失败 panic 后清理不执行，残留的「finance 已停用」会污染同一测试二进制内
    /// 的其它用例（症状是假红，且与本机改动无关）。`Drop` 在 unwind 时同样执行。
    struct OverlayReset;

    impl Drop for OverlayReset {
        fn drop(&mut self) {
            axagent_harness::clear_domain_overrides();
        }
    }

    /// 指定域的极简护照（其余字段取 `Default`，默认值均能让其它 8 个维度放行）。
    fn passport_in(domain: CapabilityDomain) -> CapabilityPassportDto {
        CapabilityPassportDto { domain, ..Default::default() }
    }

    /// **P2 消费端③ 的验收证据**：被停用域的能力不得通过能力过滤。
    ///
    /// # 为什么这样写才是证据
    ///
    /// 断言落在**公共入口 `check_all` 的返回决策**上（判据 #152：接线点须落在公共下游），
    /// 而不是「`check_domain_enabled` 被调用过」——后者在调用点被挪走/判据被换掉时照样绿。
    ///
    /// 用例含两处**对照**，缺任一都会让退化实现蒙混过关：
    /// 1. 未停用时同一护照必须 `Pass`（否则「恒 Reject」也能满足下面的断言）；
    /// 2. 停用 finance 后，**另一个域**的护照仍须 `Pass`（否则「一停全停」也能满足）。
    #[tokio::test]
    async fn test_check_all_rejects_disabled_domain() {
        let _reset = OverlayReset;
        axagent_harness::clear_domain_overrides();
        let filter = CapabilityFilterImpl::new();
        let ctx = FilterContext::default();

        // ── 改前（对照组 1）：未停用 ⇒ 该域护照通过 ──
        let finance = passport_in(CapabilityDomain::Finance);
        assert_eq!(
            filter.check_all(&finance, &ctx).await,
            FilterDecision::Pass,
            "对照组失败：未停用 finance 时该护照本应 Pass（若这里红了，说明默认 FilterContext \
             被别的维度拦下，本用例的对照组需要换一个域）"
        );

        // ── 改后：停用 finance ⇒ 必须被 DomainEnabled 维度拒绝 ──
        axagent_harness::apply_domain_overrides(vec![DomainOverride::with_enabled(
            CapabilityDomain::Finance,
            false,
        )]);
        match filter.check_all(&finance, &ctx).await {
            FilterDecision::Reject { dimension, .. } => assert_eq!(
                dimension,
                FilterDimension::DomainEnabled,
                "拒绝维度应为 domain_enabled —— 否则是被别的闸门顺手拦下的，不能证明本闸门接上了"
            ),
            other => panic!("停用 finance 后仍放行：{other:?} ⇒ 域启用闸门没接在 check_all 上"),
        }

        // ── 对照组 2：停用一个域不应影响别的域 ──
        let data = passport_in(CapabilityDomain::DataAnalysis);
        assert_eq!(
            filter.check_all(&data, &ctx).await,
            FilterDecision::Pass,
            "停用 finance 不应影响 data_analysis"
        );
    }
}
