// SPDX-License-Identifier: AGPL-3.0-only
//! 行业成分股查询（确定性，阶段 2「候选池改趋势→全关联股票」的核心数据面）。
//!
//! # 解决什么
//!
//! 趋势智选两条链的候选来源此前都是 LLM 自由发挥：
//! - 快速链 `candidate-pool.rhai` 只收 search_stock 命中（≤20 只）+ 行业领涨股；
//! - 原链 `a-candidate-mapper` 靠模型多轮 search_stock「自己发现」（训练知识偏置 ⇒
//!   候选老是那几只）。
//!
//! 本模块把「行业 → 全部成分股」做成确定性查询：按行业名从 `market_stock_universe`
//! （东财 clist f100，与 `get_industry_ranking` 同源东财）取全量成分。
//!
//! # 词表对齐（三档回退，全部如实标注）
//!
//! 行业排名接口的板块名（f14）与宇宙表 industry（f100）**同出东财但不能拿「应该一致」
//! 当前提**：宇宙表实测 128 个行业值、带申万式「Ⅱ」后缀（银行Ⅱ / 证券Ⅱ / 白酒Ⅱ），
//! 而板块名可能是「银行」「证券」。三档回退：
//! ① 精确匹配 → ② 去「Ⅱ」归一匹配 → ③ 领涨股代码反查其 industry
//! （板块排名行自带 leaderCode，是同库词表的天然锚点）。
//! 归一化至多产生 2 个候选（X / XⅡ），且二者同库时精确匹配恒先命中 ⇒ 归一档恒唯一，
//! 无歧义分支。全部落空 → `matchMode: "missing"` 空组（缺席显式声明，不伪装成空池）。
//!
//! # 分层
//!
//! 本 crate 是 implementor（依赖 entities）⇒ DB 访问合法。工作流通达由 tools crate 的
//! 薄包装（`get_industry_constituents`，DB 经 `global_state::get_sea_db()`）+ `register_all`
//! 注册完成——同 `market_mainline` 先例。

use axagent_entities::market_stock_universe;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 单只成分股（输出 camelCase，对齐全站 DTO 规范）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Constituent {
    pub stock_code: String,
    pub stock_name: String,
}

/// 输入里的单个行业引用：带领涨股锚点或不带。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum IndustryRef {
    /// `{name, leaderCode?}`（快速链 hot_industries 元素带 leader_code/leaderCode 两种键）。
    Detailed {
        name: String,
        #[serde(alias = "leader_code")]
        leader_code: Option<String>,
    },
    /// 纯字符串（原链聚合脚本只产出行业名）。
    Plain(String),
}

impl IndustryRef {
    fn name(&self) -> &str {
        match self {
            IndustryRef::Detailed { name, .. } => name,
            IndustryRef::Plain(name) => name,
        }
    }

    fn leader_code(&self) -> Option<&str> {
        match self {
            IndustryRef::Detailed { leader_code, .. } => leader_code.as_deref(),
            IndustryRef::Plain(_) => None,
        }
    }
}

/// 批量查询输入。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndustryConstituentsInput {
    pub industries: Vec<IndustryRef>,
}

/// 匹配模式（对齐档位如实标注，消费端据此判断可信度）。
pub const MATCH_EXACT: &str = "exact";
pub const MATCH_NORMALIZED: &str = "normalized";
pub const MATCH_LEADER_CODE: &str = "leader_code";
pub const MATCH_MISSING: &str = "missing";

/// 单个行业的成分组。`matchMode: "missing"` 时 stocks 为空 —— 缺席显式声明。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndustryConstituentGroup {
    pub industry: String,
    pub matched_industry: Option<String>,
    pub match_mode: String,
    pub stocks: Vec<Constituent>,
}

/// 查询报告（宇宙表覆盖面一并带出：宇宙表 27% 行业为空属数据事实，不做静默补齐）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndustryConstituentsReport {
    pub groups: Vec<IndustryConstituentGroup>,
    pub universe_total: u64,
    pub universe_with_industry: u64,
}

/// 内存态宇宙索引（按行业分组 + 归一化倒排 + 代码反查）。
///
/// 纯数据结构 ⇒ 匹配逻辑的全部单测都不需要 DB。
#[derive(Debug, Clone, Default)]
pub struct UniverseByIndustry {
    by_industry: BTreeMap<String, Vec<Constituent>>,
    by_code: BTreeMap<String, String>,
    /// 去「Ⅱ」归一后的行业名 → 原始行业名列表（构造期确定性排序）。
    normalized: BTreeMap<String, Vec<String>>,
    total_rows: u64,
}

/// 去「Ⅱ」归一：只剥 U+2161（申万式二级后缀），不做其它改写。
pub fn normalize_industry(name: &str) -> String {
    name.strip_suffix('\u{2161}').unwrap_or(name).trim().to_string()
}

/// 领涨股代码归一：东财 f140 可能带交易所前缀（如 `SZ000001`）⇒ 取末 6 位纯数字。
fn normalize_code(code: &str) -> Option<String> {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 6 {
        return None;
    }
    Some(digits[digits.len() - 6..].to_string())
}

impl UniverseByIndustry {
    /// 从 (stock_code, stock_name, industry) 行构建；空行业行防御性跳过。
    pub fn from_rows(rows: impl IntoIterator<Item = (String, String, String)>) -> Self {
        let mut idx = Self::default();
        for (code, name, industry) in rows {
            let industry = industry.trim().to_string();
            if industry.is_empty() {
                continue;
            }
            idx.total_rows += 1;
            idx.by_code.insert(code.clone(), industry.clone());
            idx.by_industry
                .entry(industry.clone())
                .or_default()
                .push(Constituent { stock_code: code, stock_name: name });
        }
        for stocks in idx.by_industry.values_mut() {
            stocks.sort_by(|a, b| a.stock_code.cmp(&b.stock_code));
        }
        let names: Vec<String> = idx.by_industry.keys().cloned().collect();
        for name in names {
            let key = normalize_industry(&name);
            if key.is_empty() {
                continue;
            }
            let bucket = idx.normalized.entry(key).or_default();
            if !bucket.contains(&name) {
                bucket.push(name);
            }
        }
        idx
    }

    pub fn universe_with_industry(&self) -> u64 {
        self.total_rows
    }

    /// 领涨股代码 → 其所属行业（同库词表的天然锚点）。
    pub fn industry_of_code(&self, code: &str) -> Option<&String> {
        let norm = normalize_code(code)?;
        self.by_code.get(&norm)
    }

    /// 三档回退解析单个行业（见模块头注释）。组内按代码排序，确定性输出。
    pub fn resolve(&self, industry: &str, leader_code: Option<&str>) -> IndustryConstituentGroup {
        let target = industry.trim();
        if target.is_empty() {
            return IndustryConstituentGroup {
                industry: industry.to_string(),
                matched_industry: None,
                match_mode: MATCH_MISSING.to_string(),
                stocks: Vec::new(),
            };
        }

        // ① 精确
        if let Some(stocks) = self.by_industry.get(target) {
            return IndustryConstituentGroup {
                industry: industry.to_string(),
                matched_industry: Some(target.to_string()),
                match_mode: MATCH_EXACT.to_string(),
                stocks: stocks.clone(),
            };
        }

        // ② 归一（去「Ⅱ」）。候选至多 X / XⅡ 两个且同库时 ① 已命中 ⇒ 此处恒唯一。
        let key = normalize_industry(target);
        if let Some(candidates) = self.normalized.get(&key)
            && let Some(matched) = candidates.first()
        {
            return IndustryConstituentGroup {
                industry: industry.to_string(),
                matched_industry: Some(matched.clone()),
                match_mode: MATCH_NORMALIZED.to_string(),
                stocks: self.by_industry[matched].clone(),
            };
        }

        // ③ 领涨股反查
        if let Some(li) = leader_code.and_then(|c| self.industry_of_code(c))
            && let Some(stocks) = self.by_industry.get(li.as_str())
        {
            return IndustryConstituentGroup {
                industry: industry.to_string(),
                matched_industry: Some(li.clone()),
                match_mode: MATCH_LEADER_CODE.to_string(),
                stocks: stocks.clone(),
            };
        }

        IndustryConstituentGroup {
            industry: industry.to_string(),
            matched_industry: None,
            match_mode: MATCH_MISSING.to_string(),
            stocks: Vec::new(),
        }
    }

    /// 批量解析（保序，去重输入不影响输出组数——每输入一项对应一组）。
    pub fn resolve_all(&self, input: &IndustryConstituentsInput) -> Vec<IndustryConstituentGroup> {
        input.industries.iter().map(|r| self.resolve(r.name(), r.leader_code())).collect()
    }
}

/// 载入宇宙表（非空行业行）并统计总覆盖面。
pub async fn load_universe_by_industry(
    db: &DatabaseConnection,
) -> Result<(UniverseByIndustry, u64), sea_orm::DbErr> {
    let universe_total = market_stock_universe::Entity::find().count(db).await?;
    let rows = market_stock_universe::Entity::find()
        .filter(
            Condition::all()
                .add(market_stock_universe::Column::Industry.is_not_null())
                .add(market_stock_universe::Column::Industry.ne("")),
        )
        .all(db)
        .await?;
    let idx = UniverseByIndustry::from_rows(
        rows.into_iter().map(|r| (r.stock_code, r.stock_name, r.industry.unwrap_or_default())),
    );
    Ok((idx, universe_total))
}

/// 查询入口：行业引用列表 → 分组报告。
pub async fn fetch_industry_constituents(
    db: &DatabaseConnection,
    input: &IndustryConstituentsInput,
) -> Result<IndustryConstituentsReport, sea_orm::DbErr> {
    let (idx, universe_total) = load_universe_by_industry(db).await?;
    Ok(IndustryConstituentsReport {
        groups: idx.resolve_all(input),
        universe_total,
        universe_with_industry: idx.universe_with_industry(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_universe() -> UniverseByIndustry {
        UniverseByIndustry::from_rows([
            ("600036".into(), "招商银行".into(), "银行Ⅱ".into()),
            ("601166".into(), "兴业银行".into(), "银行Ⅱ".into()),
            ("300750".into(), "宁德时代".into(), "电池".into()),
            ("002049".into(), "紫光国微".into(), "半导体".into()),
            ("688981".into(), "中芯国际".into(), "半导体".into()),
            ("300059".into(), "东方财富".into(), "证券Ⅱ".into()),
        ])
    }

    #[test]
    fn exact_match() {
        let u = sample_universe();
        let g = u.resolve("电池", None);
        assert_eq!(g.match_mode, MATCH_EXACT);
        assert_eq!(g.matched_industry.as_deref(), Some("电池"));
        assert_eq!(g.stocks.len(), 1);
        assert_eq!(g.stocks[0].stock_code, "300750");
    }

    #[test]
    fn normalized_match_strips_ii_suffix() {
        let u = sample_universe();
        let g = u.resolve("银行", None);
        assert_eq!(g.match_mode, MATCH_NORMALIZED);
        assert_eq!(g.matched_industry.as_deref(), Some("银行Ⅱ"));
        assert_eq!(g.stocks.len(), 2);
    }

    #[test]
    fn both_x_and_xii_in_universe_exact_wins_for_either() {
        // X 与 XⅡ 同库时精确匹配恒先命中 —— 这也是归一档恒无歧义的原因。
        let u = UniverseByIndustry::from_rows([
            ("002049".into(), "紫光国微".into(), "半导体".into()),
            ("688981".into(), "中芯国际".into(), "半导体Ⅱ".into()),
        ]);
        let g1 = u.resolve("半导体", None);
        assert_eq!(g1.match_mode, MATCH_EXACT);
        assert_eq!(g1.stocks.len(), 1);
        let g2 = u.resolve("半导体Ⅱ", None);
        assert_eq!(g2.match_mode, MATCH_EXACT);
        assert_eq!(g2.stocks.len(), 1);
    }

    #[test]
    fn leader_code_fallback_when_name_unmatched() {
        let u = sample_universe();
        // 板块名两档都落空，但领涨股 600036 反查出 银行Ⅱ
        let g = u.resolve("国有大型银行", Some("SZ600036"));
        assert_eq!(g.match_mode, MATCH_LEADER_CODE);
        assert_eq!(g.matched_industry.as_deref(), Some("银行Ⅱ"));
        assert_eq!(g.stocks.len(), 2);
    }

    #[test]
    fn missing_is_explicit_not_silent() {
        let u = sample_universe();
        let g = u.resolve("不存在的行业", None);
        assert_eq!(g.match_mode, MATCH_MISSING);
        assert_eq!(g.matched_industry, None);
        assert!(g.stocks.is_empty());
        // 空行业名同样显式 missing
        let g2 = u.resolve("  ", None);
        assert_eq!(g2.match_mode, MATCH_MISSING);
    }

    #[test]
    fn leader_code_normalizes_exchange_prefix() {
        let u = sample_universe();
        assert_eq!(u.industry_of_code("SZ600036").map(String::as_str), Some("银行Ⅱ"));
        assert_eq!(u.industry_of_code("sh600036").map(String::as_str), Some("银行Ⅱ"));
        assert_eq!(u.industry_of_code("600036").map(String::as_str), Some("银行Ⅱ"));
        assert_eq!(u.industry_of_code("12"), None);
    }

    #[test]
    fn empty_industry_rows_are_skipped() {
        let u = UniverseByIndustry::from_rows([
            ("300750".into(), "宁德时代".into(), "电池".into()),
            ("999999".into(), "无名股".into(), "".into()),
            ("888888".into(), "空格行业".into(), "  ".into()),
        ]);
        assert_eq!(u.universe_with_industry(), 1);
        assert_eq!(u.by_industry.len(), 1);
        assert_eq!(u.by_code.get("300750").map(String::as_str), Some("电池"));
    }

    #[test]
    fn resolve_all_preserves_input_order_and_per_item_grouping() {
        let u = sample_universe();
        let input = IndustryConstituentsInput {
            industries: vec![
                IndustryRef::Detailed { name: "半导体".into(), leader_code: None },
                IndustryRef::Plain("银行".into()),
            ],
        };
        let groups = u.resolve_all(&input);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].industry, "半导体");
        assert_eq!(groups[1].industry, "银行");
    }

    /// serde 输出形态锁：camelCase 键名（stockCode/stockName/matchMode/matchedIndustry），
    /// 防止未来有人改回 snake_case 打断前端 / rhai 消费端。
    #[test]
    fn serde_output_is_camel_case() {
        let g = IndustryConstituentGroup {
            industry: "电池".into(),
            matched_industry: Some("电池".into()),
            match_mode: MATCH_EXACT.into(),
            stocks: vec![Constituent {
                stock_code: "300750".into(),
                stock_name: "宁德时代".into(),
            }],
        };
        let v = serde_json::to_value(&g).unwrap();
        let obj = v.as_object().unwrap();
        for key in ["industry", "matchedIndustry", "matchMode", "stocks"] {
            assert!(obj.contains_key(key), "缺少键 {key}，实际键集: {obj:?}");
        }
        assert_eq!(obj["stocks"][0]["stockCode"], "300750", "成分股键必须 camelCase");
    }
}
