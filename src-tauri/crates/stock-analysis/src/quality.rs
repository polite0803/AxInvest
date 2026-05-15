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
    required_items: &[&str],
) -> QualityGrade {
    let text = report_text.to_lowercase();

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
        .filter(|item| text.contains(&item.to_lowercase()))
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
pub fn get_required_items(expert_id: &str) -> Vec<&'static str> {
    match expert_id {
        "market-analyst" => vec!["趋势", "形态", "指标", "支撑", "压力"],
        "sentiment-analyst" => vec!["情绪", "乐观", "悲观", "舆情", "散户"],
        "news-analyst" => vec!["公告", "新闻", "行业", "宏观", "影响"],
        "fundamentals-analyst" => vec!["盈利", "营收", "ROE", "PE", "估值"],
        "policy-analyst" => vec!["政策", "监管", "产业", "补贴", "窗口指导"],
        "hot-money-tracker" => vec!["资金", "龙虎榜", "主力", "北向", "流入", "流出"],
        "lockup-watcher" => vec!["解禁", "减持", "质押", "增持", "限售"],
        _ => vec![],
    }
}

/// 对所有分析师报告执行质量检查，生成数据质量摘要
pub fn run_quality_gate(reports: &HashMap<String, String>) -> QualityCheck {
    let mut grades: Vec<(String, QualityGrade)> = Vec::new();
    let mut warnings = Vec::new();

    let analyst_ids = [
        "market-analyst",
        "sentiment-analyst",
        "news-analyst",
        "fundamentals-analyst",
        "policy-analyst",
        "hot-money-tracker",
        "lockup-watcher",
    ];

    for id in &analyst_ids {
        let report = reports.get(*id).cloned().unwrap_or_default();
        let required = get_required_items(id);
        let grade = check_report_quality(id, &report, &required);

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
        grades.push((id.to_string(), grade));
    }

    let fail_count = grades
        .iter()
        .filter(|(_, g)| *g == QualityGrade::F || *g == QualityGrade::D)
        .count();

    let overall = if fail_count == 0 {
        QualityGrade::A
    } else if fail_count <= 1 {
        QualityGrade::B
    } else if fail_count <= 3 {
        QualityGrade::C
    } else if fail_count <= 5 {
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
        "数据质量: {}级 | 7位分析师中{}位报告存在质量问题 | {}",
        grade_str,
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
        let grade = check_report_quality("market-analyst", "", &["趋势"]);
        assert_eq!(grade, QualityGrade::F);
    }

    #[test]
    fn test_short_report_gets_d() {
        let grade = check_report_quality("market-analyst", "短", &["趋势"]);
        assert_eq!(grade, QualityGrade::D);
    }

    #[test]
    fn test_failure_marker_gets_d() {
        let report = "无法获取数据，分析失败。".repeat(10);
        let grade = check_report_quality("market-analyst", &report, &["趋势"]);
        assert_eq!(grade, QualityGrade::D);
    }

    #[test]
    fn test_full_coverage_gets_a() {
        let report = "趋势向上，形态良好，指标多头，支撑强劲，压力位突破。".repeat(10);
        let grade = check_report_quality(
            "market-analyst",
            &report,
            &["趋势", "形态", "指标", "支撑", "压力"],
        );
        assert_eq!(grade, QualityGrade::A);
    }

    #[test]
    fn test_partial_coverage_gets_b_or_c() {
        let report = "趋势向上，形态良好。".repeat(20);
        let grade = check_report_quality(
            "market-analyst",
            &report,
            &["趋势", "形态", "指标", "支撑", "压力"],
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
