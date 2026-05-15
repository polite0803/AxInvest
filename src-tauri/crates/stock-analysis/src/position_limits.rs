/// 全局仓位限制配置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionLimits {
    pub max_single_stock_pct: f64,
    pub max_total_positions: u32,
    pub max_sector_exposure_pct: f64,
}

impl Default for PositionLimits {
    fn default() -> Self {
        Self {
            max_single_stock_pct: 20.0,
            max_total_positions: 10,
            max_sector_exposure_pct: 40.0,
        }
    }
}

impl PositionLimits {
    /// 检查新增仓位是否合规
    pub fn check_new_position(
        &self,
        new_position_value: f64,
        total_portfolio_value: f64,
        current_positions: usize,
    ) -> Result<(), String> {
        if current_positions >= self.max_total_positions as usize {
            return Err(format!(
                "持仓数量已达上限 ({}只)，请先减仓再新增",
                self.max_total_positions
            ));
        }

        let new_pct = if total_portfolio_value > 0.0 {
            (new_position_value / total_portfolio_value) * 100.0
        } else {
            0.0
        };

        if new_pct > self.max_single_stock_pct {
            return Err(format!(
                "单股仓位 {:.1}% 超过上限 {:.0}%，请减少买入数量",
                new_pct, self.max_single_stock_pct
            ));
        }

        Ok(())
    }
}
