//! 分析师 id → 显示名映射。
//!
//! ## 本模块的来源与现状（2026-10-16 收窄）
//!
//! 本模块原先实现「证据引用审计」的全套判据：把 `decision_reasoning` 按句拆成 claim，
//! 再与各分析师报告正文做字符集 Jaccard 匹配。现网实测（156 条 `completed` 行）该判据
//! 对 **129 条（83%）** 只反映出**文案长度**：这些行的理由是 `portfolio-mgr` 的计算轨迹串，
//! 不含 `。！？\n` ⇒ 恒拆出 1 条 claim；而未命中组与命中组的「claim 字符被报告覆盖率」
//! 同为 0.6–0.78，命中与否只由 claim 的唯一字符数决定。
//!
//! ⇒ 文本匹配器已整体退役，换成 [`crate::evidence_audit`] 的**决策因子对账**
//! （动机、实证与四态判据见 `PLAN-evidence-factor-audit.md`）。
//!
//! 本模块只保留仍在服役的一件东西：显示名映射 —— [`crate::evidence_audit`] 逐腿调用它。
//! 删掉本文件会让审计面板的「属主分析师」退回原始节点 id，所以它连同判据一起留着。

/// 分析师 ID → 中文名的映射
///
/// 先经 `analyst_base_of` 剥档位后缀再匹配（与 `evidence_weight::classify_role` 同一条接线）：
/// B2-2 之后黑板键会是 `report.a-sentiment--mid`，不归一就整片落到 `_ => id`，
/// 面板上「来源分析师」那一格直接显示原始节点 id —— 而今天全仓还没有带档 id，
/// **这条缺陷当场测不出来**，所以配了 `tiered_analyst_ids_still_resolve_to_display_name` 提前上电。
///
/// 显示名刻意**不带档位**：Rust 侧没有档名的权威中文名（11 语言的档名键在前端），
/// 在这里再写一份就是第二个权威源；要区分哪一档读 `source_analyst_id` 本身。
pub(crate) fn analyst_display_name(id: &str) -> String {
    let base = axagent_harness::holding_period::analyst_base_of(id).unwrap_or(id);
    match base {
        "a-fundamentals" | "fundamentals-analyst" => "基本面分析师".into(),
        "a-technical" | "market-analyst" | "a-market-analyst" => "技术面分析师".into(),
        // 权威清单（`evidence_weight::EVIDENCE_ANALYST_IDS`）里的 id 必须有显示名 ——
        // 由 `every_authority_analyst_has_a_display_name` 逐条锁。补这三条之前，
        // `a-lockup` / `a-catalyst` / `a-research` / `a-policy` / `a-market-analyst`
        // 五个**现役** id 走进 `_ => id` 分支，面板「来源分析师」直接显示原始节点 id。
        "a-macro" | "policy-analyst" | "a-policy" => "宏观分析师".into(),
        "a-sector" | "sector-analyst" => "行业分析师".into(),
        "a-sentiment" | "sentiment-analyst" => "情绪分析师".into(),
        "a-news" | "news-analyst" => "新闻分析师".into(),
        "a-hot-money" | "hot-money-tracker" => "热钱追踪".into(),
        "a-lockup" => "解禁观察".into(),
        "a-catalyst" => "催化剂分析师".into(),
        "a-research" => "研报分析师".into(),
        "value-investor" => "价值投资者".into(),
        "research-analyst" => "研报分析师".into(),
        "bull-researcher" | "bull-r2" | "bull-r3" => "多头研究员".into(),
        "bear-researcher" | "bear-r2" | "bear-r3" => "空头研究员".into(),
        "research-manager" | "research-mgr" => "研究经理".into(),
        "trader" => "交易员".into(),
        "catalyst-analyst" => "催化剂分析师".into(),
        "lockup-watcher" => "解禁观察".into(),
        "rule-checker" => "规则检查".into(),
        _ => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B2-2 上电：带档分析师 id 必须仍解析到显示名。
    /// 今天全仓还没有 `a-xxx--档` 的 id ⇒ 这条在**未改产端**时是红的（先取红读数再修），
    /// 修完它锁住的是「分析师按档后面板不许退回原始节点 id」。
    #[test]
    fn tiered_analyst_ids_still_resolve_to_display_name() {
        assert_eq!(analyst_display_name("a-sentiment"), "情绪分析师");
        // 带档 id 用**产端 helper** 生成，不手抄字面量（`check-analyst-id-shape.mjs` 的 R2
        // 就是为此而红）：这样一条测试同时锁「产端拼出的形状 == 读端能剥的形状」。
        let mid = axagent_harness::holding_period::analyst_node_id("a-sentiment", "mid");
        assert_eq!(analyst_display_name(&mid), "情绪分析师");
        let lk = axagent_harness::holding_period::analyst_node_id("a-lockup", "ultra_short");
        assert_eq!(analyst_display_name(&lk), "解禁观察");
        // 未登记的 id 保留原串（含档位后缀）——不许伪装成「已收编」。
        // 样例刻意不用 `a-` 前缀：`check-analyst-id-shape.mjs` 的 R1 会把它当成
        // 「表外 base」而要走豁免 —— 为一个测试夹具开豁免，等于给门打了个洞。
        let ghost = axagent_harness::holding_period::analyst_node_id("ghost-analyst", "long");
        assert_eq!(analyst_display_name(&ghost), ghost, "未登记的 id 必须原样保留");
    }

    /// 覆盖判据：权威清单（`evidence_weight::EVIDENCE_ANALYST_IDS`）里每个 id 都必须有显示名，
    /// 不许落进 `_ => id`。存在理由：`check-analyst-id-shape.mjs` 的带日期豁免只盯【表外名字】，
    /// 而补之前 `a-lockup` / `a-catalyst` / `a-research` / `a-policy` / `a-market-analyst`
    /// 五个【表内现役】 id 一个都没登记 ⇒ 面板「来源分析师」显示原始节点 id，豁免门却一片绿。
    #[test]
    fn every_authority_analyst_has_a_display_name() {
        for id in crate::evidence_weight::EVIDENCE_ANALYST_IDS {
            let name = analyst_display_name(id);
            assert!(name.as_str() != *id, "权威分析师 {id} 没登记显示名（面板会显示原始 id）");
            let tiered = axagent_harness::holding_period::analyst_node_id(id, "mid");
            assert_eq!(
                analyst_display_name(&tiered),
                name,
                "带档 id {tiered} 与裸 id 的显示名不一致 ⇒ 按档后引用归属会分裂"
            );
        }
    }
}
