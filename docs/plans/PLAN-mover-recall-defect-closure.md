# 妖股漏检链缺陷收口计划

> 状态：代码已执行，待重启验证（2026-10-22）
> 范围：`mover_recall` 漏检链（枚举 → 收盘 → 核查 → 事件 → 补分析 → 反思 → 回灌 → 降权）的**全部已知缺陷**
> 前置：`PLAN-mover-recall-attribution.md`、`PLAN-missed-mover-reflection-closure.md`（本轮不重开已闭合项）

## 一、现网基线（2026-10-22 取数，执行前重取）

| 读数 | 值 | 说明 |
|---|---|---|
| `market_stock_universe` | 2563 只 | main_sh 1737 / main_sz 620 / bj 181 / chinext 17 / star 8 |
| 其中有 `market_daily_close` 的票 | 684 只 | main_sz 620 / main_sh 39 / chinext 17 / star 8 |
| 每日收盘行数（10-16~10-21） | 684 行/日 | 与清单 2563 差 1879 只 |
| `market_universe_stats` 最新 | 10-21 23:29，universe 684，coverage 11.54% | 清单实际 2563/5925 = 43.3% |
| `mover_recall_runs` | reco 8 轮 + serenity 8 轮 + 旧混池 2 轮 | 10-21 同一收盘日跑了 5 轮 |
| `mover_recall_events` pending | 674 行 / 107 个唯一事件 | 同一事件跨轮重复 6.3 倍 |
| 补分析批次 | 10-21 total=0；10-22 total=6 | 467 条 reco 事件 0 处理 |
| `stock_analyses(source=mover_recall)` | 6 行 | 反思已到 4 行 |
| `stock_reflections.miss_layer` 非空 | 4 行 | 归因层样本几乎为零 |
| `mover_label` | mover 2 / normal 26 / window_incomplete 132 / no_market_data 26 / NULL 49 | 可判定样本 28 条 |
| 实盘 `reco_picks`（窗口内） | synthetic=1 3344 行 / synthetic=0 634 行 | 命中判定被降级占位行主导 |

## 二、修复原则

1. **先止血再扩面**：先修让补分析出不了货的单位错配，再动核查频率与事件域。
2. **口径单一**：一个量只有一个单位、一个分母、一个判据入口；新增判据必须同时写出消费点与负控。
3. **不破既有裁定**：Q1「只降不升」、禁 mover 提权、四档独立、回放代际隔离均不变。
4. **数据变更先出清单**：旧事件作废是可逆性差的操作，先只读预览，用户确认后执行。
5. **共享树**：当前 `services.rs`、`reflection.rs`、`axagent-batch-rerun.rs` 有他人未提交改动。本计划在 `services.rs` 只改 `start_market_close_sweep` 区域，不碰 `start_batch_reflection`；全部阶段完成后按文件归属切分提交。

## 三、拍板项（先定，再动手）

| 编号 | 问题 | 推荐 | 备选 | 影响 |
|---|---|---|---|---|
| 拍-1 | 核查频率 | **改为每日（周二~周六 09:00）**，每次只核最新收盘日 | 保留周频，改为扫全周 anchor 并按 `(code,period,window_start)` 去重 | 日频改动小、每天都落事件；周频需重做事件去重 |
| 拍-2 | 同一 `(chain,date_to)` 重跑 | **覆盖**：删旧 run 及其 events 后重插 | 跳过，返回「本轮已存在」 | 覆盖能刷新同日结论；跳过保留首次结果 |
| 拍-3 | 命中判据里的 synthetic | **只有 `synthetic=0` 算命中**；`synthetic=1` 只用于池覆盖 | 除 `watchlist` 外，`capital` 等代理模式也算命中 | 推荐值会让漏检数上升（预期，不是回归） |
| 拍-4 | `LateTiming` 名实相反 | **改名 `PriorCovered`** + 时间下界（窗口起点前 ≤ 该档窗口长度） | 只加下界、保留旧名（DB/i18n 不动） | 改名涉枚举、DB 旧值、i18n 11 语言、门 p |
| 拍-5 | clist 扩页 | **保留为确认源，但停用每小时 6 页，改每日一轮 + 轮转页落库** | 直接停用，只留 bootstrap + 段扫 | 保留会继续与封禁博弈；停用则 `confirmed` 不再增长 |
| 拍-6 | 空批次池留痕 | **新建 `reco_pool_batches`**（每批一行，含 0 票批次） | 不建表，只在面板声明「无批次留痕 ⇒ 不可判池」 | 建表是「留痕不足」的根治，但改荐股写侧 + 读侧 + 门 |
| 拍-7 | 停牌判据 | `change_pct==0 && amount==0 && volume==0` ⇒ 不落行并计入 `suspended` | 维持现状（0% 落行） | 推荐值会让部分低成交平盘票被排除，需在报告显式 |

> 拍-1/2/3/7 影响判据；拍-4/5/6 影响命名与结构。未拍前阶段 1 可先做（不依赖拍板）。

## 四、阶段与判据

### 阶段 1：补分析止血（最高优先，不依赖拍板）

**问题**：`rates_json.unexplainedShare` 是百分数（0-100），`max_unexplained_share` 是 0-1，`select_events` 直接比较 ⇒ 实际阈值 0.5%。

**修法**：
1. `dao/src/repo/mover_recall_events.rs::load_run_unexplained_shares`：读取后 `/100.0`，返回 0-1；函数注释写单位，改名/注释点明「归一化占比」。
2. `commands/mover_recall_analysis.rs::select_events`：比较点只认 0-1；断言 `request.max_unexplained_share` 已在 `launch` 校验 0-1。
3. `select_events` 返回选样报告（`selected / skipped_by_share / skipped_by_period / scanned`），空态消息按实际原因分桶，不再硬编码「全部因 ≥ X%」。
4. 加一条 `tracing::warn!`：`selected=0 && scanned>0` 时输出两种跳过计数。
5. 现有 10-21/10-22 两个错误批次保留为历史，不手工重跑；修复后由下一轮 cron 自动补。

**判据**：
- 单测：`rates_json` 里 `unexplainedShare=45.45` 的轮次通过 0.5 阈值；`50.5` 被拒；`null` 放行。
- 单测：空态消息按 `skipped_by_share` / `skipped_by_period` 分别成句，负控各改一个计数。
- 门：新增/扩 `check-mover-recall-units.mjs`（或 `check-reco-horizon-parity.mjs` 新段），锁「读取函数 `/100.0`」+「比较点不出现 `*100`」；`--selftest` 两负控；注册 `ci-check.mjs` + `ci.yml` 两处。
- 现网：下一轮 06:00 批次 `total` 应接近 `min(60, 唯一 pending)`；`stock_analyses(source=mover_recall)` 增量 > 0。

### 阶段 2：核查覆盖（依赖拍-1/拍-2）

**问题**：`build_chain_view` 只保留 `anchor_date == f.to`；cron 周频 ⇒ 一周只核一个交易日；同一收盘日重跑落多轮重复事件。

**修法**：
1. `stock_automation_cron_seed.rs`：`mover-recall-weekly` → `mover-recall-daily`，cron `0 9 * * 2-6`（周二~周六覆盖周一~周五收盘）；保留 ID 迁移（旧 ID 行改名或新建 + 禁用旧行，按种子「存在即跳过」语义处理）。
2. `persist_mover_recall_run` 前按 `(chain, date_to)` 判重：拍-2 选覆盖则删旧 run + 旧 events（同事务），再插入；选跳过则返回已有 run id 并写摘要。
3. 旧口径事件作废（拍-2 后执行）：把 `anchor_date` 早于「新口径生效日」的滚动窗口事件、以及 A-3 前 `serenity` 超短/短事件标 `excluded`，`error_detail` 写「旧口径/层已退役」。先出只读清单（按 chain/anchor/period 计数），用户确认后执行。
4. 选样去随机：`select_events` 先按三键分组，同一事件取「两链中最小 share」判过滤（任一链通过即入选）；`list_candidate_events` 保留按涨幅降序，但不再依赖 `dedupe_by_event_key` 保留哪条链。
5. `latest_reco_miss_codes`（快通路）与 D-4 回灌（慢通路）在面板/文档中各占一句，避免只看见慢通路。

**判据**：
- 单测：同一 `date_to` 连续 persist 两次 ⇒ run 1 轮、同三键 event 不翻倍（拍-2 覆盖）。
- 单测：两链同键、reco share=6.67 / serenity share=0.0 ⇒ 事件被选中且只认领一次。
- 单测：旧口径事件状态 `excluded` 后不在 `CANDIDATE_STATUSES` 结果里。
- 现网：连续 5 个交易日各有一轮 run，`date_to` 连续；事件表 pending 唯一数不再跨轮放大。
- 现网：作废清单执行后 `pending` 只剩新口径事件。

### 阶段 3：事件域与清单域对齐（依赖拍-5/拍-7）

**问题**：清单 2563、有收盘 684；`ensure_close_history` 用全局日期数直接 return；面板 `universe_size` 用清单数，事件域只用有收盘的票。

**修法**：
1. `MoverRecallView` 新增 `codes_with_close`、`judgeable_codes_by_tier: BTreeMap<String, usize>`（逐档 `series.len() >= window_days` 的票数）；`events_by_tier` 保留。面板增加「清单 / 有收盘 / 本档可判」三数，窗口未满按**逐档可判票数**判，不再只看全局 `collected_days`。
2. `ensure_close_history` 改逐票覆盖：读每票最近 90 个交易日行数，对不足的票补；每日最多补 `K` 只（建议 200），失败退避（记录 `last_attempt_at`，避免每周重试同一批）。不得再用 `close_dates.len() >= 90` 一刀切。
3. `run_close_sweep` 重跑阈值：`have * 2 < size` → `have < size`（或 `have < size * 0.98`）；`CloseSweepReport` 增加 `missing_codes` 计数；任务摘要显示「不完整」。
4. `close_row` 停牌：拍-7 判据；`CloseSweepReport.suspended` 计数；单测覆盖停牌/平盘/零价/缺量。
5. 段扫：`CODE_SEGMENTS` 补北交所 87x（870000-879999）与 92x（920000-920999），先确认 `detect_market_type` 对两段返回 `bj`；`scan_segment_shard` 的 `?` 改为记录失败批并继续；`ScanStats` 加 `failed_batches`；cron 摘要显示。
6. clist（拍-5）：改为每日一轮（或停用）；页轮转状态落 `settings`/`market_universe_stats`，不再用进程内 `tick`。
7. `market_universe_stats`：段扫后也写一行（或新增「清单 / 有收盘 / 确认」三数读数），并加新鲜度判据（`observed_at` 距今 ≤ 36h）。

**判据**：
- 单测：`ensure_close_history` 的选票判据（全局日期够但某票缺行 ⇒ 该票入选；该票够 90 行 ⇒ 不入选）。
- 单测：`close_row` 停牌样本不落行、平盘有量样本落行。
- 单测：`CODE_SEGMENTS` 与 `detect_market_type` 的 bj 段全集对表（新增段各一正控、430 段一负控）。
- 现网：面板三数出现；新票逐日进入有收盘域；`market_universe_stats` 新鲜度 ≤ 36h。
- 门：`check-universe-coverage.mjs` 的 C1/C3 改用三数口径（仍只在本地跑，`--selftest` 进 CI）。

### 阶段 4：归因口径（依赖拍-3/拍-4/拍-6）

**问题**：synthetic 占位行被当命中；`pool_degraded` 用 any；空批次不落行；三个率分母不一致；`LateTiming` 名实相反。

**修法**：
1. `build_chain_view` 拆 `hit_picks`（synthetic=0）与 `pool_picks`（含 synthetic）：
   - `picks_same_period` / `recommended` / `picked_before_window` / `picked_in_other_period` 用 `hit_picks`；
   - `in_pool` / `pool_degraded` 用 `pool_picks`。
2. 拍-6 建 `reco_pool_batches`：荐股扫描结束写一行（`batch_id, generated_at, as_of_date, seed_pool_json, stock_count, degraded`，0 票也写）；`in_pool` 优先按该表判，缺行回退到 pick 行；`pool_degraded` 改为「该票所在的全部批次都空/缺」。
3. 三率分母：
   - 新增 `judgeable_events`（排除 `ByDesign` / `LateTiming`）；`reachability = 入池或命中 / judgeable_events`（`PoolDegraded` 不计入分子）；
   - `coverage` 分母 = 入池事件（与 `in_pool` 变量逐项一致，含 `PoolDegraded`）；
   - `unexplained_share` 分母 = misses（不变）；
   - 面板 tooltip 写明三句分母；DTO 增加 `judgeableEvents`。
4. 拍-4：枚举改名 `PriorCovered`（DB 旧值 `late_timing` 读侧兼容映射，新写只写新值），加时间下界；`MissLayer::ALL`、`attribute`、门 p、11 语言同步。
5. 摘要：两链各一段（现状已分），末尾注明「同一事件在两条链各计一次，唯一事件数见面板」；`MoverCheckOutcome.misses` 保留两链和，但通知文案不写「N 条事件」而写「两条链合计 N 条（唯一事件见任务结果）」。
6. `load_mover_cell_signal` 与核查对齐：只统计 `anchor_date == to` 的事件（或按 `(code,period,window_start)` 去重），避免同一票多日达标重复计 `scored_out`/`hits`；`scored_out` 与 `hits` 的 synthetic 口径对称（拍-3 后都只认 synthetic=0，或都含，需在实现时写明）。

**判据**：
- 单测：synthetic=1 行覆盖的票不再判命中；池覆盖仍 true；负控各一。
- 单测：一个空池批次 + 一个正常批次 ⇒ `pool_degraded=false`；全空才 true。
- 单测：`judgeable_events` 排除 ByDesign/LateTiming；`coverage` 分母与 `in_pool` 变量同集；负控各一。
- 单测：`PriorCovered` 时间下界（一年前推荐不豁免、窗口前一个窗口内豁免）。
- 现网：reco 漏检数会上升（拍-3），记录新旧差值；面板三率分母文案与 DTO 一致。

### 阶段 5：呈现、门禁与版本

**问题**：区间文案只写 from~to；轮次无口径版本；覆盖率门不在 CI；单位 bug 无门。

**修法**：
1. 面板 `boundary` 改为「可核查窗口 {{from}}~{{to}}；实际只核查 {{to}} 当日达标事件」；11 语言。
2. `mover_recall_runs` 加 `gate_version`（声明式对账加列），写入当前层口径版本（如 `"mover-layers-v2"`）；面板轮次下拉显示版本，跨版本轮次标注「口径不同，不可直接比较」。
3. 覆盖率门：C1/C3 改三数口径；新鲜度判据；`--selftest` 进 CI。正式跑仍本地（要 PG）。
4. 单位门：阶段 1 的门保持注册；补一条「批次 total=0 且 scanned>0 时消息必须分桶」的 Rust 单测。
5. 面板测试：`MoverRecallPanel` 增加「清单/有收盘/本档可判」三数、「只核右端日」文案、三率分母 tooltip 的渲染断言；每个破坏样本只红对应一条。
6. 任务中心摘要测试：两链各一段 + 唯一事件说明。

**判据**：
- `npm run typecheck` / `dprint check` / `oxlint src` 0；vitest 新增/相关测试全绿。
- 11 语言 parity + `check:eol`。
- `cargo fmt --all --check` / `cargo clippy -p axagent -p axagent-analysis-engine -p axagent-dao --all-targets -- -D warnings` 0。
- 门脚本 `--selftest` + 正跑全绿；`ci-check.mjs` 与 `ci.yml` 两处注册逐字同源。

## 五、执行顺序与提交

1. 阶段 1（止血）→ 先跑一轮现网批次验证 → 再进入阶段 2。
2. 阶段 2/3 可并行（文件不重叠：阶段 2 在 `commands/mover_recall*.rs` + cron seed；阶段 3 在 `market_close_store.rs` + `services.rs` + `universe_scan.rs`）。`services.rs` 只改 `start_market_close_sweep`，不碰他人 `start_batch_reflection` 区域。
3. 阶段 4 依赖拍-3/4/6；若拍-6 不建表，则把 `in_pool` 改为「仅在有批次留痕时判定」，并在面板声明「无留痕不可判」，不伪造。
4. 阶段 5 收口门禁与呈现。
5. 共享树有他人未提交改动（`services.rs`、`reflection.rs`、`axagent-batch-rerun.rs`）。本批**只改不提交**；待他人改动落地后按文件归属切分：
   - 阶段 1/2/4 的 `mover_recall*` 文件可独立提交；
   - `services.rs` 的采集 hunk 与 `reflection.rs` 的文案/上下文 hunk 若与（他人在先的）改动同文件，提交前用 `git diff --stat` + 逐 hunk 归属确认。
6. 全量门禁只在全部阶段完成后跑一次（CI 口径）：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace`。

## 六、不做 / 延后

- 不改四档阈值、不改权重提权、不改 `reco_ic_gate` 出厂 shadow。
- 不给智选链补 `reco_scan_audit`（B-2 已判定「按设计不可判」）。
- 不做「跨档并集事件」；四档独立。
- 不在本轮做「全市场 90 天历史回填」的全量一次性任务；阶段 3 按每日限流补，避免打爆通道。
- 阶段 6（可选）：若拍-6 不建 `reco_pool_batches`，则「空批次留痕」只在文档/面板声明，不在本轮根治。

## 七、回退与风险

| 风险 | 处置 |
|---|---|
| 阶段 3 补历史打爆行情通道 | 每日限流 200 只 + 失败下轮再试；已有部分历史的优先，0 行新票随后轮转 |
| 阶段 2 日频核查让补分析排队变长 | 观察 2-3 天；`CRON_BATCH_LIMIT=60` 可调；补分析按最新 anchor 优先 |
| 拍-3 让漏检数上升被误读为回归 | 面板/任务摘要注明「口径变更：降级占位行不再算命中」；保留旧轮次快照对照 |
| 旧事件作废不可逆 | 先只读清单；执行前备份 `mover_recall_events` 状态计数；只改 status，不删行 |
| `PriorCovered` 改名影响旧数据/门 | 读侧兼容 `late_timing` 旧值；门 p 同时认两名一过渡期；11 语言键迁移 |
| 共享树冲突 | 本批只改不提交；`services.rs` 只碰 `start_market_close_sweep`；提交前逐 hunk 归属 |

## 八、执行状态（2026-10-22）

全部阶段代码已落地，**未提交**（共享树里有他人未提交的 `reflection.rs` / `axagent-batch-rerun.rs` 改动，本轮未碰其区域）。

- 已执行数据变更：旧口径事件作废 `541` 条（`pending` 674 → `133`），只改 `status='excluded'`，不删行。
- 编译与 lint：`cargo check`（dao/analysis-engine/entities/harness/axagent，`--all-targets`）0；小 crate `clippy --all-targets --all-features -D warnings` 0；主 crate 同口径 clippy 0。
- 测试：引擎 `mover_recall` 23/23；dao `reco_pool` 3/3；命令层 `commands::mover_recall` 21/21；种子 `init::stock_automation_cron_seed` 7/7。
- 前端：`tsc --noEmit` 0；`dprint check` 0；`oxlint` 0；`MoverRecallPanel` 4 文件 28 例 + 新增事件域 2 例全绿；i18n 键 parity 11/11、占位符门 0、行尾门 0。
- 脚本门：`check-reco-horizon-parity` 正跑 + selftest 全绿；`check-mover-recall-units` 正跑 + selftest 4/4；`check-universe-coverage --selftest` 5/5；`check-layer-discipline` / `check-reco-generation` / `check-task-type-whitelist` / `check-reflection-output-consumers` 全绿。
- 现网读取：`check-universe-coverage` 正跑仍红 C1（11.54% < 50%，读的是重启前旧 stats）与 C2（`main_sh` 67.8%），这两条是当前真实覆盖现状，不是本批回归；重启后段扫会更新 stats。

**偏差与待重启验证**：
1. cron ID 仍为 `mover-recall-weekly`（幂等键不改），靠一次性 `schedule == "0 9 * * 6"` 升级为日频；已加种子测试。
2. `reco_pool_batches` 已加实体、DAO 与三条荐股写侧（命令 / 定时 cron / 时间旅行），0 票批次也写；**需重启**由声明式对账建表后才会写行。旧轮与智选链仍回退 `reco_picks.seed_pool_json`（智选 batch 边界在 workflow，另案）。
3. 逐票历史补齐挂在收盘采集钩子（每天 200 只），不在核查前同步跑。
4. 停牌判据用「涨幅 0 且成交额/成交量均缺」；腾讯对停牌的实际返回形态未单独实测，若停牌票带正成交量会回落为落 0% 行。
5. `check-decl-evidence` 有 8 条与知识图谱无关的既有「内容与 id 不同源」红（退出码 3），不在本轮文件内。

**重启后要看的四个读数**：① `mover_recall_runs` 每个交易日各一轮；② 06:00 补分析 `total` 接近 60（不再被 0.5% 阈值拦）；③ `reco_pool_batches` 有行且 0 票批次 `degraded=true`；④ 面板「清单 / 有收盘 / 本档可判」三数与 `gateVersion=mover-layers-v2`。
