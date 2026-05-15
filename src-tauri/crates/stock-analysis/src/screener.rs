use axagent_astock_data::AStockClient;

/// 筛选条件
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCriteria {
    /// 最小涨跌幅（默认None=不限）
    pub min_change_pct: Option<f64>,
    /// 最大涨跌幅
    pub max_change_pct: Option<f64>,
    /// 龙虎榜净买入>N（万元）
    pub dragon_tiger_net_min: Option<f64>,
    /// 主力净流入>N（万元）
    pub main_inflow_min: Option<f64>,
    /// 北向持仓占比>N%
    pub northbound_ratio_min: Option<f64>,
    /// 换手率>N%
    pub turnover_rate_min: Option<f64>,
    /// RSI 超卖（<30）
    pub rsi_oversold: bool,
    /// RSI 超买（>70）
    pub rsi_overbought: bool,
}

/// 筛选结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenResult {
    pub stock_code: String,
    pub stock_name: String,
    pub price: f64,
    pub change_pct: f64,
    /// 匹配原因
    pub reasons: Vec<String>,
    /// 综合得分
    pub score: u32,
}

/// 股票筛选器
pub struct StockScreener;

impl StockScreener {
    /// 从自选股中筛选符合条件的标的
    pub async fn screen_watchlist(
        client: &AStockClient,
        watchlist: &[(String, String)],
        criteria: &ScreenCriteria,
    ) -> Result<Vec<ScreenResult>, String> {
        let mut results = Vec::new();

        for (code, name) in watchlist {
            let mut reasons = Vec::new();
            let mut score = 0u32;

            // 获取行情
            let quote = match client.get_quote(code).await {
                Ok(q) => q,
                Err(_) => continue,
            };

            // 涨跌幅筛选
            if let Some(min) = criteria.min_change_pct {
                if quote.change_pct < min {
                    continue;
                }
                reasons.push(format!("涨幅{:.2}%", quote.change_pct));
                score += 2;
            }
            if let Some(max) = criteria.max_change_pct {
                if quote.change_pct > max {
                    continue;
                }
            }

            // 换手率筛选
            if let Some(tr) = criteria.turnover_rate_min {
                if quote.turnover_rate < tr {
                    continue;
                }
                reasons.push(format!("换手率{:.2}%", quote.turnover_rate));
                score += 2;
            }

            // 龙虎榜
            if criteria.dragon_tiger_net_min.is_some() {
                if let Ok(dt) = client.get_dragon_tiger(code).await {
                    let net: f64 = dt.iter().map(|d| d.net_amount).sum();
                    if let Some(min_net) = criteria.dragon_tiger_net_min {
                        if net >= min_net {
                            reasons.push(format!("龙虎榜净买入{:.0}万", net / 10000.0));
                            score += 3;
                        } else {
                            continue;
                        }
                    }
                }
            }

            // 主力资金
            if criteria.main_inflow_min.is_some() {
                if let Ok(Some(mf)) = client.get_money_flow(code).await {
                    if let Some(min_inflow) = criteria.main_inflow_min {
                        if mf.main_net_inflow >= min_inflow {
                            reasons
                                .push(format!("主力净流入{:.0}万", mf.main_net_inflow / 10000.0));
                            score += 3;
                        } else {
                            continue;
                        }
                    }
                }
            }

            results.push(ScreenResult {
                stock_code: code.clone(),
                stock_name: name.clone(),
                price: quote.price,
                change_pct: quote.change_pct,
                reasons,
                score,
            });
        }

        // 按得分降序
        results.sort_by(|a, b| b.score.cmp(&a.score));
        Ok(results)
    }
}
