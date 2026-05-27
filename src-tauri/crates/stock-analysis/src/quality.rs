//! 数据质量门控模块。
//!
//! 在 phase_2 分析师报告生成完成后，对所有报告执行质量检查，
//! 生成数据质量摘要，注入到后续辩论/风控/决策阶段。

use std::collections::HashMap;

/// 数据质量等级
#[derive(Debug, Clone, PartialEq)]
pub enum QualityGrade {
    /// 全部通过
    A,
    /// 轻微问题
    B,
    /// 部分数据缺失
    C,
    /// 显著缺口
    D,
    /// 严重失败
    F,
}

/// 数据质量检查结果
#[derive(Debug, Clone)]
pub struct QualityCheck {
    pub grade: QualityGrade,
    pub summary: String,
    pub warnings: Vec<String>,
}

/// 检查单个分析师报告的质量
pub fn check_report_quality(
    _expert_id: &str,
    report_text: &str,
    required_items: &[Vec<&str>],
) -> QualityGrade {
    let text = report_text.to_lowercase();

    // 占位报告检测 (LLM 未连接时生成的假报告)
    let is_placeholder = text.contains("\"summary\":\"占位报告")
        || text.contains("agentrunner 未注入")
        || text.contains("placeholder");
    if is_placeholder {
        return QualityGrade::F;
    }

    // 硬检查 1: 报告是否为空或过短
    if report_text.trim().is_empty() {
        return QualityGrade::F;
    }
    if report_text.len() < 100 {
        return QualityGrade::D;
    }

    // 硬检查 2: 是否包含失败标记
    let failure_markers = [
        "无法获取",
        "数据不足",
        "无数据",
        "error",
        "failed",
        "抱歉",
        "无法分析",
    ];
    let has_failure = failure_markers.iter().any(|m| text.contains(m));
    if has_failure {
        return QualityGrade::D;
    }

    // 硬检查 3: 必采清单覆盖率
    let covered = required_items
        .iter()
        .filter(|group| {
            group
                .iter()
                .any(|keyword| text.contains(&keyword.to_lowercase()))
        })
        .count();
    let total = required_items.len();
    if total == 0 {
        return QualityGrade::B;
    }

    let ratio = covered as f64 / total as f64;
    if ratio >= 0.8 {
        QualityGrade::A
    } else if ratio >= 0.6 {
        QualityGrade::B
    } else if ratio >= 0.4 {
        QualityGrade::C
    } else {
        QualityGrade::D
    }
}

/// 必采清单 — 每个分析师报告必须包含的关键数据项
pub fn get_required_items(expert_id: &str) -> Vec<Vec<&'static str>> {
    match expert_id {
        "market-analyst" => vec![
            vec!["趋势", "走势", "方向"],
            vec!["形态", "图形", "模式"],
            vec!["指标", "MACD", "RSI", "KDJ"],
            vec!["支撑", "支撑位", "底部"],
            vec!["压力", "压力位", "阻力", "阻力位"],
        ],
        "sentiment-analyst" => vec![
            vec!["情绪", "市场情绪", "人气"],
            vec!["乐观", "看多", "积极"],
            vec!["悲观", "看空", "消极"],
            vec!["舆情", "舆论", "社交媒体"],
            vec!["散户", "个人投资者", "零售"],
        ],
        "news-analyst" => vec![
            vec!["公告", "披露", "通告"],
            vec!["新闻", "资讯", "消息"],
            vec!["行业", "产业", "板块"],
            vec!["宏观", "经济", "GDP"],
            vec!["影响", "冲击", "效应"],
        ],
        "fundamentals-analyst" => vec![
            vec!["盈利", "利润", "收益"],
            vec!["营收", "收入", "营业额"],
            vec!["ROE", "净资产收益率"],
            vec!["PE", "市盈率", "估值"],
            vec!["估值", "价值", "定价"],
        ],
        "policy-analyst" => vec![
            vec!["政策", "方针", "规划"],
            vec!["监管", "合规", "审查"],
            vec!["产业", "行业政策"],
            vec!["补贴", "扶持", "优惠"],
            vec!["窗口指导", "约谈", "警示"],
        ],
        "hot-money-tracker" => vec![
            vec!["资金", "成交", "流入"],
            vec!["龙虎榜", "席位", "营业部"],
            vec!["主力", "机构", "大单"],
            vec!["北向", "外资", "沪港通", "深港通"],
            vec!["流入", "净流入", "增仓"],
            vec!["流出", "净流出", "减仓"],
        ],
        "lockup-watcher" => vec![
            vec!["解禁", "限售股", "锁定期"],
            vec!["减持", "套现", "抛售"],
            vec!["质押", "抵押", "担保"],
            vec!["增持", "回购", "护盘"],
            vec!["限售", "锁股", "禁售"],
        ],
        _ => vec![],
    }
}

/// 对所有分析师报告执行质量检查，生成数据质量摘要
pub fn run_quality_gate(reports: &HashMap<String, String>) -> QualityCheck {
    let mut grades: Vec<(String, QualityGrade)> = Vec::new();
    let mut warnings = Vec::new();

    // 排除非分析师角色的 ID（辩论员、经理、交易员等）
    let non_analyst_keywords = ["debator", "researcher", "manager", "trader"];

    for (id, report) in reports.iter() {
        if non_analyst_keywords.iter().any(|kw| id.contains(kw)) {
            continue;
        }
        let required = get_required_items(id);
        let grade = check_report_quality(id, report, &required);

        if grade == QualityGrade::F || grade == QualityGrade::D {
            warnings.push(format!(
                "⚠️ {} 报告质量{}，已标记为低置信度",
                id,
                match grade {
                    QualityGrade::D => "D",
                    _ => "F",
                }
            ));
        }
        grades.push((id.clone(), grade));
    }

    let total_count = grades.len();
    let fail_count = grades
        .iter()
        .filter(|(_, g)| *g == QualityGrade::F || *g == QualityGrade::D)
        .count();

    let overall = if total_count == 0 {
        QualityGrade::C
    } else if fail_count == 0 {
        QualityGrade::A
    } else if fail_count <= (total_count as f64 * 0.2).ceil() as usize && fail_count <= 1 {
        QualityGrade::B
    } else if fail_count <= (total_count as f64 * 0.5).ceil() as usize {
        QualityGrade::C
    } else if fail_count <= (total_count as f64 * 0.8).ceil() as usize {
        QualityGrade::D
    } else {
        QualityGrade::F
    };

    let grade_str = match overall {
        QualityGrade::A => "A",
        QualityGrade::B => "B",
        QualityGrade::C => "C",
        QualityGrade::D => "D",
        QualityGrade::F => "F",
    };

    let summary = format!(
        "数据质量: {}级 | {}位分析师中{}位报告存在质量问题 | {}",
        grade_str,
        total_count,
        fail_count,
        if warnings.is_empty() {
            "所有报告通过质量检查".to_string()
        } else {
            warnings.join("; ")
        }
    );

    QualityCheck {
        grade: overall,
        summary,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_empty_report_gets_f() {
        let grade = check_report_quality("market-analyst", "", &[vec!["趋势"]]);
        assert_eq!(grade, QualityGrade::F);
    }

    #[test]
    fn test_short_report_gets_d() {
        let grade = check_report_quality("market-analyst", "短", &[vec!["趋势"]]);
        assert_eq!(grade, QualityGrade::D);
    }

    #[test]
    fn test_failure_marker_gets_d() {
        let report = "无法获取数据，分析失败。".repeat(10);
        let grade = check_report_quality("market-analyst", &report, &[vec!["趋势"]]);
        assert_eq!(grade, QualityGrade::D);
    }

    #[test]
    fn test_full_coverage_gets_a() {
        let report = "趋势向上，形态良好，指标多头，支撑强劲，压力位突破。".repeat(10);
        let grade = check_report_quality(
            "market-analyst",
            &report,
            &[
                vec!["趋势"],
                vec!["形态"],
                vec!["指标"],
                vec!["支撑"],
                vec!["压力"],
            ],
        );
        assert_eq!(grade, QualityGrade::A);
    }

    #[test]
    fn test_partial_coverage_gets_b_or_c() {
        let report = "趋势向上，形态良好。".repeat(20);
        let grade = check_report_quality(
            "market-analyst",
            &report,
            &[
                vec!["趋势"],
                vec!["形态"],
                vec!["指标"],
                vec!["支撑"],
                vec!["压力"],
            ],
        );
        assert!(grade == QualityGrade::C || grade == QualityGrade::B);
    }

    #[test]
    fn test_run_quality_gate_with_mixed_reports() {
        let mut reports = HashMap::new();
        reports.insert(
            "market-analyst".to_string(),
            "趋势向上，形态良好，MACD金叉，支撑位有效，压力位需观察。".repeat(10),
        );
        reports.insert(
            "sentiment-analyst".to_string(),
            "".to_string(), // Empty = F
        );
        reports.insert(
            "news-analyst".to_string(),
            "短".to_string(), // Short = D
        );
        reports.insert(
            "fundamentals-analyst".to_string(),
            "营收增长，ROE稳健，PE合理，估值偏低。".repeat(10),
        );
        reports.insert("policy-analyst".to_string(), "政策利好，产业扶持，监管放松。".repeat(10));
        reports.insert(
            "hot-money-tracker".to_string(),
            "主力流入，龙虎榜机构买入，北向加仓。".repeat(10),
        );
        reports.insert(
            "lockup-watcher".to_string(),
            "近期无解禁，大股东增持，质押比例低。".repeat(10),
        );

        let result = run_quality_gate(&reports);
        // 2 failures out of 7 = grade C
        assert_eq!(result.grade, QualityGrade::C);
        assert_eq!(result.warnings.len(), 2);
    }
}
