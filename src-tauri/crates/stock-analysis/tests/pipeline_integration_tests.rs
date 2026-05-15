use axagent_astock_data::indicators::*;
use axagent_stock_analysis::decision::*;
use axagent_stock_analysis::quality::*;
use axagent_stock_analysis::rules::RuleEngine;
use axagent_stock_analysis::scoring::ScoringEngine;

fn generate_test_klines(
    count: usize,
    start: f64,
    end: f64,
) -> Vec<axagent_astock_data::KLine> {
    let step = (end - start) / count as f64;
    (0..count)
        .map(|i| {
            let price = start + step * i as f64;
            axagent_astock_data::KLine {
                date: format!("2026-01-{:02}", i + 1),
                open: price - 0.1,
                high: price + 0.3,
                low: price - 0.3,
                close: price,
                volume: 1000.0 + i as f64 * 10.0,
                amount: price * 1100.0,
                turnover_rate: None,
            }
        })
        .collect()
}

#[test]
fn test_scoring_to_signal_pipeline() {
    let klines = generate_test_klines(60, 10.0, 12.0);
    let indicators = compute_indicators("TEST", &klines);
    let score = ScoringEngine::score(&indicators, 11.0, None);
    assert!(score.total <= 100);
    assert!(!score.signal.is_empty());
    assert!(
        ["strong_buy", "buy", "hold", "watch", "sell", "strong_sell"]
            .contains(&score.signal_code.as_str())
    );
    assert!(score.trend_score <= 30);
    assert!(score.rsi_score <= 10);
}

#[test]
fn test_rules_override_strong_buy() {
    let klines = generate_test_klines(60, 10.0, 28.0);
    let indicators = compute_indicators("TEST", &klines);
    let score = ScoringEngine::score(&indicators, 27.0, None);
    let result = RuleEngine::check(
        &indicators,
        &score,
        "买入",
        Some(25.0),
        Some(27.0),
    );
    if indicators.rsi6 > 80.0 || indicators.bias_ma5 > 5.0 {
        assert!(!result.passed);
        assert!(result.force_signal.is_some());
    }
}

#[test]
fn test_quality_gate_mixed_reports() {
    let mut reports = std::collections::HashMap::new();
    reports.insert(
        "market-analyst".into(),
        "趋势向上，形态良好，MACD金叉，支撑有效，压力突破。".repeat(10),
    );
    reports.insert("news-analyst".into(), "".to_string());
    reports.insert("fundamentals-analyst".into(), "short".to_string());
    let result = run_quality_gate(&reports);
    assert!(result.warnings.len() >= 2);
}

#[test]
fn test_config_validation() {
    assert!(AnalysisConfig::default().validate().is_ok());
    assert!(
        AnalysisConfig {
            max_debate_rounds: 0,
            ..AnalysisConfig::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        AnalysisConfig {
            kline_limit: 0,
            ..AnalysisConfig::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        AnalysisConfig {
            kline_limit: 501,
            ..AnalysisConfig::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        AnalysisConfig {
            news_limit: 0,
            ..AnalysisConfig::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        AnalysisConfig {
            kline_period: "yearly".into(),
            ..AnalysisConfig::default()
        }
        .validate()
        .is_err()
    );
}
