-- SPDX-License-Identifier: AGPL-3.0-only
--
-- 决策 alpha 离线报告（只读，不调任何 LLM）
--
-- ## 用途
--
-- 回答一个问题：**这套分析链的决策，相对「同一天随便买」有没有超额？**
-- 用「同日固定效应」当市场基准 —— 同一天、同一周期的全部验证样本的平均收益。
-- 相比外部指数，这个基准与样本同源、无需 K 线、无复权口径问题；代价是它衡量的是
-- **横截面选股能力**，不是绝对择时能力（同涨同跌会被基准吸收）。
--
-- ## 为什么必须看 span_days
--
-- 2,000+ 条样本可能只覆盖十几个交易日。横截面再厚，时间维度薄到一定程度，
-- 任何「规律」都可能是某几天的产物 ⇒ 报告第一段就把 days / first_day / last_day
-- 打出来，低于 60 个交易日时任何结论都只当假设。
--
-- ## 运行
--
--   node scripts/decision-alpha-report.mjs            # 打印 JSON
--   node scripts/decision-alpha-report.mjs --md       # 打印 Markdown 表
--
-- 底层走 `scripts/pg-connect.mjs`（口令从 ~/.axagent 解密，脚本里不含密钥）。
--
-- ## 数据前提
--
-- `decision_validations.final_return_pct` 非空才算已结算；`generated_at` 前 10 位当决策日。
-- 同一天同一周期不足 8 条时，该日不进基准（样本太少的日子会把基准拖偏）。
WITH v AS (
  SELECT period,
         t_plus_n,
         confidence,
         inferred_action,
         left(generated_at, 10) AS d,
         final_return_pct       AS ret,
         CASE WHEN jsonb_typeof(factor_snapshot::jsonb -> 'chg1') = 'number'
              THEN (factor_snapshot::jsonb ->> 'chg1')::float8 END       AS chg1,
         CASE WHEN jsonb_typeof(factor_snapshot::jsonb -> 'dist_lo20') = 'number'
              THEN (factor_snapshot::jsonb ->> 'dist_lo20')::float8 END  AS dist_lo20
  FROM decision_validations
  WHERE final_return_pct IS NOT NULL
), dm AS (
  SELECT d, t_plus_n, avg(ret) AS mkt, count(*) AS n_day
  FROM v
  GROUP BY d, t_plus_n
  HAVING count(*) >= 8
), e AS (
  SELECT v.*, v.ret - dm.mkt AS ex
  FROM v
  JOIN dm ON v.d = dm.d AND v.t_plus_n = dm.t_plus_n
), q AS (
  SELECT e.*,
         ntile(4) OVER (PARTITION BY period, t_plus_n ORDER BY chg1)      AS q_chg1,
         ntile(4) OVER (PARTITION BY period, t_plus_n ORDER BY dist_lo20) AS q_dist
  FROM e
), samples AS (
  SELECT 'mid + T+2 + 置信≥70'                     AS rule, d, ret, ex
    FROM e WHERE period = 'mid' AND t_plus_n = 2 AND confidence >= 70
  UNION ALL
  SELECT 'mid + T+2', d, ret, ex
    FROM e WHERE period = 'mid' AND t_plus_n = 2
  UNION ALL
  SELECT 'chg1 最高四分位（强动量）', d, ret, ex
    FROM q WHERE q_chg1 = 4
  UNION ALL
  SELECT 'dist_lo20 最低四分位（贴近 20 日低）', d, ret, ex
    FROM q WHERE q_dist = 1
  UNION ALL
  SELECT 'T+2 + 避开 dist_lo20 最高四分位', d, ret, ex
    FROM q WHERE t_plus_n = 2 AND (q_dist IS NULL OR q_dist <> 4)
  UNION ALL
  SELECT '仅 T+2（全样本）', d, ret, ex
    FROM e WHERE t_plus_n = 2
  UNION ALL
  SELECT 'chg1 最低四分位（弱动量）', d, ret, ex
    FROM q WHERE q_chg1 = 1
  UNION ALL
  SELECT 'dist_lo20 最高四分位（远离 20 日低）', d, ret, ex
    FROM q WHERE q_dist = 4
), by_rule AS (
  SELECT rule,
         count(*)                                          AS n,
         avg(ret)                                          AS avg_ret,
         avg(ex)                                           AS avg_excess,
         avg(CASE WHEN ret > 0 THEN 1.0 ELSE 0 END) * 100  AS win_abs
  FROM samples GROUP BY rule
), rule_day AS (
  SELECT rule, d, avg(ret) AS day_ret FROM samples GROUP BY rule, d
), rule_stability AS (
  SELECT rule,
         count(*)                                     AS days,
         count(*) FILTER (WHERE day_ret > 0)          AS days_positive
  FROM rule_day GROUP BY rule
)
SELECT jsonb_pretty(jsonb_build_object(
  'span', (SELECT jsonb_build_object(
      'rows_settled',     (SELECT count(*) FROM v),
      'rows_with_bench',  (SELECT count(*) FROM e),
      'days',             (SELECT count(DISTINCT d) FROM dm),
      'first_day',        (SELECT min(d) FROM dm),
      'last_day',         (SELECT max(d) FROM dm),
      'trade_days',       (SELECT min(t_plus_n) || '..' || max(t_plus_n) FROM dm))),
  'baseline', (SELECT jsonb_build_object(
      'n',          count(*),
      'avg_ret',    round(avg(ret)::numeric, 2),
      'avg_excess', round(avg(ex)::numeric, 2),
      'win_abs',    round((avg(CASE WHEN ret > 0 THEN 1.0 ELSE 0 END) * 100)::numeric, 1),
      'win_vs_day', round((avg(CASE WHEN ex > 0 THEN 1.0 ELSE 0 END) * 100)::numeric, 1),
      't_stat',     round((avg(ex) / nullif(stddev_samp(ex) / sqrt(count(*)), 0))::numeric, 2))
    FROM e),
  'by_horizon', (SELECT jsonb_agg(jsonb_build_object(
      't', t_plus_n, 'n', n, 'avg_ret', ar, 'avg_excess', ae, 'win_abs', wa, 't_stat', ts)
      ORDER BY t_plus_n) FROM (
      SELECT t_plus_n, count(*) AS n,
             round(avg(ret)::numeric, 2) AS ar,
             round(avg(ex)::numeric, 2)  AS ae,
             round((avg(CASE WHEN ret > 0 THEN 1.0 ELSE 0 END) * 100)::numeric, 1) AS wa,
             round((avg(ex) / nullif(stddev_samp(ex) / sqrt(count(*)), 0))::numeric, 2) AS ts
      FROM e GROUP BY t_plus_n HAVING count(*) >= 30) s),
  'by_period', (SELECT jsonb_agg(jsonb_build_object(
      'period', period, 't', t_plus_n, 'n', n,
      'avg_ret', ar, 'avg_excess', ae, 't_stat', ts) ORDER BY period, t_plus_n) FROM (
      SELECT period, t_plus_n, count(*) AS n,
             round(avg(ret)::numeric, 2) AS ar,
             round(avg(ex)::numeric, 2)  AS ae,
             round((avg(ex) / nullif(stddev_samp(ex) / sqrt(count(*)), 0))::numeric, 2) AS ts
      FROM e GROUP BY period, t_plus_n HAVING count(*) >= 40) s),
  'by_confidence', (SELECT jsonb_agg(jsonb_build_object(
      'band', band, 'n', n, 'avg_excess', ae, 't_stat', ts) ORDER BY band) FROM (
      SELECT band, count(*) AS n,
             round(avg(ex)::numeric, 2) AS ae,
             round((avg(ex) / nullif(stddev_samp(ex) / sqrt(count(*)), 0))::numeric, 2) AS ts
      FROM (SELECT *, CASE WHEN confidence < 50 THEN '<50'
                           WHEN confidence < 60 THEN '50-59'
                           WHEN confidence < 70 THEN '60-69'
                           ELSE '70+' END AS band FROM e) b
      GROUP BY band) s),
  'rules', (SELECT jsonb_agg(jsonb_build_object(
      'rule', r.rule, 'n', r.n,
      'avg_ret', round(r.avg_ret::numeric, 2),
      'avg_excess', round(r.avg_excess::numeric, 2),
      'win_abs', round(r.win_abs::numeric, 1),
      'days', st.days, 'days_positive', st.days_positive)
      ORDER BY r.avg_excess DESC) FROM by_rule r
    JOIN rule_stability st ON st.rule = r.rule)
))::text;
