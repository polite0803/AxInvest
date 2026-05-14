use serde::{Deserialize, Serialize};

/// 回测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResult {
    pub stock_code: String,
    pub analysis_date: String,
    pub decision_action: String,
    pub decision_confidence: f64,
    /// 分析日的收盘价（入场价）
    pub entry_price: Option<f64>,
    /// 持有N日后的收盘价（出场价）
    pub exit_price: f64,
    pub holding_days: u32,
    /// 收益率（%）
    pub return_pct: f64,
    /// 决策是否正确（买入/增持应涨，减持/卖出应跌）
    pub was_correct: bool,
    /// 持有期间最大回撤（%）
    pub max_drawdown: f64,
}

/// 回测统计数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestStats {
    pub total_analyses: u32,
    /// 准确率（%）
    pub accuracy_pct: f64,
    /// 平均收益率（%）
    pub avg_return_pct: f64,
    /// 平均最大回撤（%）
    pub avg_max_drawdown_pct: f64,
    /// 平均置信度
    pub avg_confidence: f64,
}

/// 历史分析记录（用于回测输入）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalAnalysis {
    pub stock_code: String,
    pub analysis_date: String,
    pub decision_action: String,
    pub decision_confidence: f64,
}

/// 回测引擎
pub struct BacktestEngine;

impl BacktestEngine {
    /// 回测单个分析决策。
    ///
    /// 策略：假设在 `analysis_date` 以收盘价入场，持有 `holding_days` 后以收盘价出场。
    /// 从 API 获取 `holding_days + 10` 根日 K 线以覆盖分析日之后的数据。
    pub async fn backtest_decision(
        client: &axagent_astock_data::AStockClient,
        stock_code: &str,
        analysis_date: &str,
        decision_action: &str,
        decision_confidence: f64,
        holding_days: u32,
    ) -> Result<BacktestResult, String> {
        // 获取分析日后 holding_days+10 天的K线数据
        let klines = client
            .get_klines(stock_code, "daily", holding_days + 10)
            .await
            .map_err(|e| format!("获取K线失败: {}", e))?;

        if klines.is_empty() {
            return Err(format!("{} 无K线数据", stock_code));
        }

        // 找到分析日对应的 K 线（或最近的一根）
        let entry_kline = klines.iter().find(|k| k.date.as_str() >= analysis_date);
        let entry_idx = entry_kline.map(|_| {
            klines
                .iter()
                .position(|k| k.date.as_str() >= analysis_date)
                .unwrap_or(0)
        });
        let entry_price = entry_kline.map(|k| k.close);

        // 取第 holding_days 根K线（或最后可用的）
        let exit_idx = entry_idx
            .map(|i| (i + holding_days as usize).min(klines.len() - 1))
            .unwrap_or(klines.len() - 1);
        let exit_price = klines[exit_idx].close;

        // 计算持有期间最大回撤
        let relevant: Vec<_> = klines
            .iter()
            .skip_while(|k| k.date.as_str() < analysis_date)
            .take(holding_days as usize + 1)
            .collect();

        let mut peak = 0.0;
        let mut max_dd = 0.0;
        for k in &relevant {
            if k.close > peak {
                peak = k.close;
            }
            let dd = if peak > 0.0 {
                (peak - k.close) / peak
            } else {
                0.0
            };
            if dd > max_dd {
                max_dd = dd;
            }
        }

        // 计算收益率
        let return_pct = match entry_price {
            Some(entry) if entry > 0.0 => ((exit_price - entry) / entry) * 100.0,
            _ => 0.0,
        };

        // 判断决策是否正确：买入/增持应涨，减持/卖出应跌
        let was_correct = match decision_action {
            "买入" | "增持" => return_pct > 0.0,
            "减持" | "卖出" => return_pct < 0.0,
            _ => true, // 持有/观望不算错
        };

        Ok(BacktestResult {
            stock_code: stock_code.to_string(),
            analysis_date: analysis_date.to_string(),
            decision_action: decision_action.to_string(),
            decision_confidence,
            entry_price,
            exit_price,
            holding_days,
            return_pct,
            was_correct,
            max_drawdown: max_dd * 100.0,
        })
    }

    /// 批量回测历史分析记录
    pub async fn backtest_history(
        client: &axagent_astock_data::AStockClient,
        analyses: Vec<HistoricalAnalysis>,
        holding_days: u32,
    ) -> Result<Vec<BacktestResult>, String> {
        let mut results = Vec::new();
        for analysis in analyses {
            match Self::backtest_decision(
                client,
                &analysis.stock_code,
                &analysis.analysis_date,
                &analysis.decision_action,
                analysis.decision_confidence,
                holding_days,
            )
            .await
            {
                Ok(r) => results.push(r),
                Err(e) => {
                    tracing::warn!(
                        "回测失败 {} {}: {}",
                        analysis.stock_code,
                        analysis.analysis_date,
                        e
                    );
                },
            }
        }
        Ok(results)
    }

    /// 计算回测统计指标
    pub fn compute_stats(results: &[BacktestResult]) -> BacktestStats {
        let total = results.len() as f64;
        let correct = results.iter().filter(|r| r.was_correct).count() as f64;

        let accuracy = if total > 0.0 {
            (correct / total) * 100.0
        } else {
            0.0
        };
        let avg_return: f64 = if total > 0.0 {
            results.iter().map(|r| r.return_pct).sum::<f64>() / total
        } else {
            0.0
        };
        let avg_max_dd: f64 = if total > 0.0 {
            results.iter().map(|r| r.max_drawdown).sum::<f64>() / total
        } else {
            0.0
        };
        let avg_confidence = if total > 0.0 {
            results.iter().map(|r| r.decision_confidence).sum::<f64>() / total
        } else {
            0.0
        };

        BacktestStats {
            total_analyses: results.len() as u32,
            accuracy_pct: accuracy,
            avg_return_pct: avg_return,
            avg_max_drawdown_pct: avg_max_dd,
            avg_confidence,
        }
    }
}
