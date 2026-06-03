use axagent_stock_analysis::decision::*;

#[test]
fn test_analysis_event_serialization() {
    let event = AnalysisEvent::Started {
        stock_code: "600519".into(),
        stock_name: "贵州茅台".into(),
        date: "2026-05-14".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"type\":\"started\""));
    assert!(json.contains("600519"));
    assert!(json.contains("茅台"));
}

#[test]
fn test_analysis_event_error_serialization() {
    let event = AnalysisEvent::Error {
        stage: "数据加载".into(),
        message: "数据加载失败".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"type\":\"error\""));
    assert!(json.contains("失败"));
}

#[test]
fn test_stock_decision_defaults() {
    let decision = StockDecision {
        action: "买入".into(),
        position_pct: 10.0,
        target_price: Some(1850.0),
        stop_loss: Some(1580.0),
        reasoning: "测试".into(),
        risk_level: "中".into(),
        confidence: 80,
    };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: StockDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.action, "买入");
    // camelCase after serde rename
    assert_eq!(parsed.position_pct, 10.0);
    assert_eq!(parsed.confidence, 80.0);
    assert_eq!(parsed.target_price, Some(1850.0));
    assert!(json.contains("positionPct"));
}

#[test]
fn test_analysis_config_validation() {
    let config = AnalysisConfig::default();
    assert!(config.validate().is_ok());

    let bad = AnalysisConfig {
        max_debate_rounds: 0,
        ..AnalysisConfig::default()
    };
    assert!(bad.validate().is_err());

    let bad2 = AnalysisConfig {
        max_debate_rounds: 11,
        ..AnalysisConfig::default()
    };
    assert!(bad2.validate().is_err());

    let bad3 = AnalysisConfig {
        kline_limit: 0,
        ..AnalysisConfig::default()
    };
    assert!(bad3.validate().is_err());

    let bad4 = AnalysisConfig {
        kline_limit: 501,
        ..AnalysisConfig::default()
    };
    assert!(bad4.validate().is_err());

    let bad5 = AnalysisConfig {
        news_limit: 0,
        ..AnalysisConfig::default()
    };
    assert!(bad5.validate().is_err());

    let bad6 = AnalysisConfig {
        kline_period: "minute".into(),
        ..AnalysisConfig::default()
    };
    assert!(bad6.validate().is_err());
}

#[test]
fn test_analysis_config_default() {
    let config = AnalysisConfig::default();
    assert_eq!(config.max_debate_rounds, 3);
    assert_eq!(config.kline_period, "daily");
    assert_eq!(config.kline_limit, 120);
    assert_eq!(config.news_limit, 30);
}

#[test]
fn test_stock_decision_roundtrip() {
    let decision = StockDecision {
        action: "持有".into(),
        position_pct: 30.0,
        target_price: None,
        stop_loss: None,
        reasoning: "基本面稳健，技术面震荡，建议持有观望".into(),
        risk_level: "低".into(),
        confidence: 65,
    };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: StockDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.action, "持有");
    assert_eq!(parsed.position_pct, 30.0);
    assert_eq!(parsed.target_price, None);
    assert_eq!(parsed.stop_loss, None);
    assert_eq!(parsed.confidence, 65.0);
}
