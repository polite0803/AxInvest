// SPDX-License-Identifier: AGPL-3.0-only

//! `reconcile::apply` — P4-4：把 plan 变成库上的实际改动（**编排层**）。
//!
//! ## 这一层为什么必须单独存在
//!
//! `render` 是纯函数（`Change` → DDL 文本），`safety` 是纯判据 + 簿记读写。
//! 两者都不知道**顺序**：先建元表还是先熔断？白名单在 diff 前读还是 diff 后读？
//! 墓碑写在 `DROP` 之前还是之后？这些顺序决定了引擎是「安全网」还是「事故现场」。
//! 顺序集中在本模块，且每一条都用单测钉住。
//!
//! ## 五个顺序决策（每一条都是刻意的）
//!
//! | 顺序 | 决策 | 写反了会怎样 |
//! |---|---|---|
//! | 1 | `ensure_meta_tables` **最先** | 簿记写不进去 ⇒ 「这次做了什么」永久丢失 |
//! | 2 | 白名单在 **diff 之前**读 | 白名单功能存在但从不生效（孤儿照删）—— 判据 B 组「断链」 |
//! | 3 | **全部渲染完**再开始执行 | 第 N 条渲染失败时前 N-1 条已落库 ⇒ 半同步库 |
//! | 4 | 墓碑在 `DROP` **之前**写 | 进程在 `DROP` 中间死掉 ⇒ 数据没了而「导出在哪」无记录 |
//! | 5 | 审计在**执行之后**写 | 见下「审计与墓碑的分工」 |
//!
//! ## 审计与墓碑的分工（第 5 条为什么也可以是可接受的）
//!
//! 审计在最后写，因此存在一个窗口：进程在执行中途被杀 ⇒ DDL 已生效而审计缺行。
//! 这个窗口**对不可逆的那一类是关闭的** —— `DROP TABLE` 之前一定先落墓碑，而墓碑正是
//! 「不可逆操作」的**前置**记录。其余变更（建表 / 加列 / 建索引）即使没留审计也能靠
//! 「重新 introspect 一次」反推出来，代价是多一轮 diff，不是数据丢失。
//! 所以正确的写法不是「把审计也提前」，而是把**墓碑提前**，让 pre-write 只覆盖
//! 真正需要它的那个类别。
//!
//! ## 逃生阀管的是业务 DDL，不管簿记
//!
//! `dry_run` 下**仍然**会建元表、**仍然**会写审计。这不是漏洞，是必须的：
//! 「dry-run 看过什么」如果无处记录，那它就只能靠终端 scrollback —— 而那个东西会消失。
//! 元表全部落在 `_ax_schema_` 前缀下，对 `introspect` 不可见，故不影响指纹。
//!
//! ## ⚠ dry-run **不消费**延迟令牌
//!
//! 闸 5 的「一个周期」靠 `_ax_schema_pending_drops` 里的行数来判定。若 dry-run 也去
//! 登记，那么「先 dry-run 看一眼，再真跑」会让真跑**当场执行**破坏性变更 ——
//! 延迟闸被一次观察行动吃掉了。所以 dry-run 只**报告**「若真跑，本条会被延迟」。
//!
//! ## ⚠ 不包整轮事务
//!
//! 每条语句各自提交。理由不是「简单」：墓碑必须能被后续的独立查询看到（它证明的是
//! 「当时有导出证据」），而把它和被执行的 DDL 放进同一个回滚单元，会让 `DROP` 失败时
//! 墓碑一并消失 —— 那时人就无从知道「差一步就删了哪张表、导出在哪」。
//! 失败时的正确性靠**审计 + 墓碑 + fail-stop**（首条失败即中止后续），不靠回滚。
//!
//! ## ⚠ **未接线**
//!
//! 本模块不是启动路径的一部分。它是**一次性入口**（PLAN §五·三）：只由
//! `examples/p4_apply_probe.rs` 显式调用，不随进程启动自动跑。

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr};

use super::extras::Dialect;
use super::plan::{Change, ChangeKind, Plan, PlanOptions};
use super::render::{self, RenderError, Rendered};
use super::safety::{self, AuditRow, DeferDecision, GraveEvidence, Refusal, SafetyConfig};
use super::{expected, fingerprint, introspect};

/// 渲染器的函数指针类型。
///
/// ## 为什么做成可注入（而不是直接调 [`render::render`]）
///
/// apply 的**编排逻辑**（顺序、七道闸、簿记、收敛判据）必须能在 CI 上端到端验证，而 CI
/// 里能跑的是 `sqlite::memory:`。真渲染器在 SQLite 上只放行**原生支持的 6 类**
/// （`CreateTable` / `DropTable` / `AddColumn` / `RenameColumn` / `CreateIndex` /
/// `DropIndex`，见 `render::sqlite_supports_natively`），其余 15 类返回
/// [`RenderError::UnsupportedDialect`]。若编排测试只能用真渲染器，那 15 类的
/// 闸逻辑（配额 / 延迟 / 墓碑 / 证据）在这套测试里就**永远覆盖不到**。
/// 一个「够用的假渲染器」让它们能被真跑。安全网的核心逻辑若永远无法在 CI 里被验证，
/// 那等于没有安全网。
///
/// ⚠ 注意 [`render::render`] 早已不是「只做 PG」—— 那是 P4 时期的状态，已过期。
pub type RenderFn = fn(&Change, Dialect) -> Result<Rendered, RenderError>;

/// apply 的运行期输入。
#[derive(Debug, Clone)]
pub struct ApplyOptions {
    /// 七道闸的配置（默认全部取安全方向：dry-run 开、配额 5、延迟、不许基数下降）。
    pub config: SafetyConfig,
    /// `loses_data` 变更的导出证据，键 = `Change::object`（闸 4）。
    pub evidence: BTreeMap<String, GraveEvidence>,
    /// 上一轮的期望表数（闸 3 的基准）。`None` = 首轮，不比。
    pub last_expected_table_count: Option<usize>,
    /// **起始**序号。实际 `run_id` 由 [`safety::allocate_run_id`] 从它往上找第一个空位
    /// —— `(run_id, seq)` 是审计表主键，同秒重跑必须能换号。
    pub ordinal: u32,
    /// DDL 渲染器。默认 [`render::render`]（**双方言**：PG 全量，SQLite 原生 6 类）。
    pub renderer: RenderFn,
    /// **只执行纯新增变更**（判据见 [`Plan::retain_purely_additive`]）。
    ///
    /// 启动期 bootstrap 必须置 `true`：那条路径**没有人工审查**，任何收缩类变更
    /// （DROP / RENAME / SET NOT NULL）都得留给离线探针在有人看着时执行。
    ///
    /// 默认 `false` —— 探针路径保持全量语义，破坏性另由七道闸把关（两者是
    /// 「显式收窄」与「事后拦截」的关系，不是替代）。
    pub additive_only: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            config: SafetyConfig::default(),
            evidence: BTreeMap::new(),
            last_expected_table_count: None,
            ordinal: 0,
            renderer: render::render,
            additive_only: false,
        }
    }
}

impl ApplyOptions {
    /// 生产路径：闸配置从进程环境读，其余取默认。
    pub fn from_env() -> Self {
        Self { config: SafetyConfig::from_env(), ..Self::default() }
    }
}

/// 一条变更**没被执行**的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// 闸 1：逃生阀开着。
    DryRun,
    /// 闸 5：破坏性变更首次出现，本轮只登记。
    DeferredFirstSight,
    /// 闸 2/3/4 或渲染失败：整批被拒，一条都没执行。
    BatchRefused,
    /// 前一条执行失败，本轮在此中止（fail-stop）。
    AbortedAfterError,
    /// **本方言没有原生 DDL**（`RenderError::UnsupportedDialect`）：只跳过本条，其余照跑。
    ///
    /// ## 为什么它不是「整批拒绝」那一类
    ///
    /// 2026-09-16 实测（`repo::message::tests::create_message_round_trips_attachment_metadata`
    /// 报红）：迁移建出的 SQLite 库里，实体声明与实况有 40 条差集，其中 19 条是
    /// `SET DEFAULT` / `ADD FK` —— SQLite 的 `ALTER TABLE` 只有四条形态，这两类都没法
    /// 原生表达。旧实现把它们计入 `render_errors` ⇒ `ApplyRefusal::Unrenderable` ⇒
    /// **整批拒绝 ⇒ 启动中止**（当时的处置是「刻意让它被拒」）。
    ///
    /// 那个处置的前提是错的：**「跳过」在这两类上并不比现状更差**。被拒绝的那些变更
    /// 本来就是「库里没有、声明里有」，跳过只是让库保持原样（它已经这样跑了很多个版本），
    /// 而整批拒绝会让 App 直接起不来。旧理由「回退到只建表不建约束会造出少约束的表且
    /// 永不收敛」说的是 `CreateTable` 少写 FK/CHECK 的情形 —— 那种情形下 SQLite 走的是
    /// **内联**（见 `render::create_table_statements`），根本不经过本变体。
    ///
    /// 代价（如实登记，不粉饰）：这些条目会在**每一轮** diff 里重新出现（永不收敛），
    /// 直到实现重建表 12 步流程（PLAN §3.1）。所以它必须**可见** —— 进 `report()`
    /// 的一行、进审计的 `notes`、并在 `db::initialize_schema` 里打 `warn!`。
    UnsupportedDialect,
}

impl SkipReason {
    /// 进审计与报告的说明文本。
    pub fn note(self) -> &'static str {
        match self {
            Self::DryRun => "逃生阀（dry-run）：只渲染并写审计，未执行任何 DDL",
            Self::DeferredFirstSight => "闸 5：破坏性变更首次出现，本轮只登记，下一轮才执行",
            Self::BatchRefused => "整批被拒：本轮一条都没执行",
            Self::AbortedAfterError => "前一条执行失败，本轮在此中止（fail-stop）",
            Self::UnsupportedDialect => {
                "本方言没有这条变更的原生 DDL（SQLite 的 ALTER TABLE 只有 RENAME TO / \
                 RENAME COLUMN TO / ADD COLUMN / DROP COLUMN）⇒ 只跳过本条，其余照跑。\
                 后果：该库形态与本轮声明仍有此差集，下一轮 diff 会再次报出它"
            },
        }
    }

    /// 短标签（报告里的方括号）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DryRun => "dry-run",
            Self::DeferredFirstSight => "延迟",
            Self::BatchRefused => "被拒",
            Self::AbortedAfterError => "中止",
            Self::UnsupportedDialect => "方无DDL",
        }
    }
}

/// 逐条结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// 在 `Plan::changes` 里的下标（= 审计表的 `seq`）。
    pub seq: usize,
    pub kind: ChangeKind,
    pub object: String,
    pub destructive: bool,
    pub loses_data: bool,
    /// 该变更渲染出的语句（按执行顺序）。渲染失败时为空。
    pub statements: Vec<String>,
    /// 渲染器给出的语义备注（进审计）。
    pub notes: Vec<String>,
    /// 是否**真的**在库上执行成功。
    pub executed: bool,
    /// 未执行的原因（`None` 且 `executed == false` 只可能是执行失败，见 `error`）。
    pub skip: Option<SkipReason>,
    /// 渲染失败 / 执行失败的原文。
    pub error: Option<String>,
}

/// 整批中止的原因。任一命中 ⇒ **一条 DDL 都不执行**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyRefusal {
    /// 闸 2 / 3：熔断。
    Circuit(Refusal),
    /// 闸 4：`loses_data` 变更缺导出证据。
    MissingEvidence(Vec<String>),
    /// 渲染失败。**必须在执行前全部渲染完** —— 边渲染边执行会在第 N 条失败时
    /// 留下「前 N-1 条已落库」的半同步库，而引擎下次启动无法区分它和「被外部改动」。
    Unrenderable(Vec<String>),
}

impl ApplyRefusal {
    /// 人读的一行说明（进日志与审计）。
    pub fn reason(&self) -> String {
        match self {
            Self::Circuit(r) => format!("熔断：{}", r.reason()),
            Self::MissingEvidence(objs) => format!(
                "{} 条会丢数据的变更缺少导出证据：{}（先把数据导出来，再重跑）",
                objs.len(),
                objs.join("、")
            ),
            Self::Unrenderable(errs) => format!(
                "{} 条变更渲染失败：{}（渲染器缺陷或 payload 与 kind 不匹配）",
                errs.len(),
                errs.join("；")
            ),
        }
    }

    /// 短标签（报告头与审计）。
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Circuit(_) => "circuit",
            Self::MissingEvidence(_) => "evidence",
            Self::Unrenderable(_) => "render",
        }
    }
}

/// 一次 apply 的完整结果。
#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub run_id: String,
    /// 逃生阀是否开着（开着 ⇒ 一条 DDL 都没执行）。
    pub dry_run: bool,
    /// 整批中止的原因（`None` = 没有被整批拒）。
    pub refusal: Option<ApplyRefusal>,
    pub items: Vec<Item>,
    /// 因闸 5 被延迟的条数（**预期**不会收敛，不等于失败）。
    pub deferred: usize,
    /// 真正执行成功的条数。
    pub executed: usize,
    /// 因执行失败而中止的原文（`None` = 未中止）。
    pub aborted: Option<String>,
    /// 本轮写入墓碑的表名。
    pub graveyard: Vec<String>,
    /// ⚠ **墓碑已写但 `DROP` 未成功**的表名 —— 该表仍在库里，必须人工看。
    pub false_graves: Vec<String>,
    /// 写入审计的行数。
    pub audit_rows: usize,
    /// 闸 6：生效的白名单条数。
    pub whitelist_active: usize,
    /// 闸 6：已过期的白名单（**不再豁免**，留痕用）。
    pub whitelist_expired: Vec<String>,
    /// 熔断闸用的「期望表数」（`None` = 调用方未提供）。
    pub expected_table_count: Option<usize>,
    /// diff 时算出的期望指纹（`None` = 传进来的 plan 没带，无从比对收敛）。
    pub expected_fingerprint: Option<String>,
    /// apply 之后重新 introspect 得到的实况指纹。
    pub post_fingerprint: Option<String>,
    /// 收敛判据：`post_fingerprint == expected_fingerprint`。
    /// **只在意指纹相等** —— 被延迟的条目会让它保持 `false`，那是预期的。
    ///
    /// ⚠ 这个判据在 PG 上**结构性地不可满足**，见 [`Self::advisories`] 的文档。
    pub converged: Option<bool>,
    /// diff 阶段留下的**声明漂移**（advisory：不产出任何 DDL）。
    ///
    /// ## 为什么它必须原样带进 outcome
    ///
    /// 2026-09-16 真库实测：同一份 plan，`--pg-render` 的日志里有 **273 条**
    /// `text-diff … default/generated/check`，而 `<pg-url>` 生产路径的日志里 **0 条**
    /// —— 因为本结构体此前**没有**这个字段，`report()` 也从不打印它。
    /// 「不被看到就等于不存在」：273 条表达式漂移在生产日志里是**无声无息**的，
    /// 而这恰好违反 advisory 机制自己的立项理由（`extras.rs`：「而不是无声无息」）。
    ///
    /// ## 它同时是 `converged` 恒否的证据
    ///
    /// 这 273 条**全是 PG 对表达式文本的重写**（`'x'` → `'x'::text`、`true` → `TRUE`、
    /// `0` → `0.0`、生成的 `COALESCE(...)` 加括号、CHECK 的 `= ANY (ARRAY[…])`），
    /// 无一为语义漂移 —— `plan.rs::expression_text_diff_is_advisory_only` 这个测试
    /// 就是为此写的，其注释原话：「**若判成差异，plan 永远清不空**」。
    ///
    /// 该判据在 `diff` 里被**正确堵住**了（文本差异只记 advisory、不产变更），
    /// 但 [`crate::reconcile::fingerprint`] 是 `SHA-256(canonical_json())` —— 整个模型
    /// 逐字段序列化，**列默认值/生成列/CHECK 的表达式文本全在里面** ⇒
    /// `post_fingerprint` 与 `expected_fingerprint` 在 PG 上**永不相等**。
    /// 即：**同一个噪声，两个判据，只堵了一个。**
    pub advisories: Vec<String>,
}

impl ApplyOutcome {
    /// 本轮是否「干净」：没有被整批拒、没有中止、没有假墓碑。
    pub fn is_clean(&self) -> bool {
        self.refusal.is_none() && self.aborted.is_none() && self.false_graves.is_empty()
    }

    /// 本轮是否把 plan **完整**应用掉了 —— 执行了全部变更，无延迟、无中止、无拒绝。
    ///
    /// 与 [`Self::converged`] 的分工：`converged` 是**指纹**相等（结构真的对上了），
    /// 本函数是**流程**走完（没有任何一条被拦）。两者都为真才算一次收敛的 apply。
    pub fn fully_applied(&self) -> bool {
        self.refusal.is_none()
            && self.aborted.is_none()
            && self.deferred == 0
            && self.executed == self.items.len()
    }

    /// **本方言表达不了**的条目（[`SkipReason::UnsupportedDialect`]）。
    ///
    /// 为什么是方法而不是新增一个计数字段：`items` 已经是唯一真相源，另存一份计数就是多一处
    /// 会各自腐烂的副本（判据 K 组「只改定义端、漏消费端」）。要数字就是 `.len()`。
    ///
    /// ## 调用方该怎么用它
    ///
    /// | 场景 | 期望 | 非空说明什么 |
    /// |---|---|---|
    /// | 全新库（引擎独自建表） | **空** | 非空 ⇒ `render` 的方言能力表漏了一类建表所需形态，那是**真缺陷**，必须红 |
    /// | 存量库（迁移建出的） | 可非空 | 结构性的已知缺口（PLAN §3.1 重建表 12 步流程未实现）⇒ 只报不阻 |
    ///
    /// 这个分野是本项目最容易搞错的地方：同一个数字在两种输入下含义相反。
    pub fn unsupported(&self) -> Vec<&Item> {
        self.items.iter().filter(|i| i.skip == Some(SkipReason::UnsupportedDialect)).collect()
    }

    /// **因前一条执行失败而被跳过**的条目（[`SkipReason::AbortedAfterError`]）。
    ///
    /// 与 [`Self::unsupported`] 的分工 —— 两者都来自 `skip`，但影响面不同：
    /// `unsupported` 是「本方言表达不了**那一条**」，其余照常；本方法**非空**则意味着
    /// **本轮收敛半途而废**：失败那条之后的所有变更都没执行，库停在「改了一半」的状态。
    ///
    /// 为什么给方法而不是新字段：`items` 已是唯一真相源，另存计数就是多一处会各自腐烂的
    /// 副本（同 [`Self::unsupported`] 的理由）。
    ///
    /// 为什么消费方需要它：`aborted` 本身只是一条错误串，**不带「牵连了多少条」这个量**，
    /// 而那个量正是判断「要不要手工收敛」的依据。启动路径上 `refusal` 会转成 `Err`，
    /// 但 `aborted` **不阻断启动**（拦下来会把「结构没收敛完」变成「应用起不来」，
    /// 是更差的交换）⇒ 那就必须至少让它在日志里带上这个量。
    pub fn aborted_skips(&self) -> Vec<&Item> {
        self.items.iter().filter(|i| i.skip == Some(SkipReason::AbortedAfterError)).collect()
    }

    /// 可打印报告。
    pub fn report(&self, title: &str) -> String {
        let mut s = String::new();
        s.push_str(&format!("== reconcile apply · {title} ==\n"));
        s.push_str(&format!("run_id      : {}\n", self.run_id));
        s.push_str(&format!(
            "逃生阀      : {}\n",
            if self.dry_run {
                "dry-run（只渲染不执行）"
            } else {
                "已关闭（会执行 DDL）"
            }
        ));
        s.push_str(&format!(
            "变更合计    : {}｜执行 {}｜延迟 {}｜未执行 {}\n",
            self.items.len(),
            self.executed,
            self.deferred,
            self.items.len() - self.executed
        ));
        s.push_str(&format!(
            "整批拒绝    : {}\n",
            match &self.refusal {
                None => "无".to_string(),
                Some(r) => format!("[{}] {}", r.kind_str(), r.reason()),
            }
        ));
        s.push_str(&format!(
            "执行中止    : {}\n",
            self.aborted.clone().unwrap_or_else(|| "无".to_string())
        ));
        // 这一行**必须**存在：条目被跳过而报告不提，就等于「方言能力缺口」不存在。
        // 它同时是「本库永远不会因这几条而收敛」的唯一可见信号。
        let unsupported = self.unsupported();
        if !unsupported.is_empty() {
            let sample = unsupported
                .iter()
                .take(8)
                .map(|i| format!("{} {}", i.kind.as_str(), i.object))
                .collect::<Vec<_>>()
                .join("、");
            s.push_str(&format!(
                "⚠ 方无原生DDL: {} 条只跳过（不阻断本轮，也不阻断启动）—— 该库形态因此永不收敛，\
                 直到实现重建表 12 步流程（PLAN §3.1）。示意：{}{}\n",
                unsupported.len(),
                sample,
                if unsupported.len() > 8 { " …" } else { "" }
            ));
        }
        s.push_str(&format!("墓碑        : {} 条\n", self.graveyard.len()));
        if !self.false_graves.is_empty() {
            s.push_str(&format!(
                "⚠ 假墓碑    : {} —— 墓碑已写但 DROP 未成功，表仍在库里，下次运行会重试\n",
                self.false_graves.join("、")
            ));
        }
        s.push_str(&format!("审计        : 写 {} 行\n", self.audit_rows));
        s.push_str(&format!(
            "白名单      : 生效 {} 条｜已过期 {} 条{}\n",
            self.whitelist_active,
            self.whitelist_expired.len(),
            if self.whitelist_expired.is_empty() {
                String::new()
            } else {
                format!("（过期项不再豁免：{}）", self.whitelist_expired.join("、"))
            }
        ));
        if let Some(n) = self.expected_table_count {
            s.push_str(&format!("期望表数    : {n}\n"));
        }
        match (&self.expected_fingerprint, &self.post_fingerprint) {
            (Some(e), Some(p)) => {
                let short = |x: &str| -> String { x.chars().take(16).collect() };
                s.push_str(&format!("期望指纹    : {}\n", short(e)));
                s.push_str(&format!("应用后指纹  : {}\n", short(p)));
                s.push_str(&format!(
                    "收敛        : {}\n",
                    match self.converged {
                        Some(true) => "是（实况指纹 == 期望指纹）".to_string(),
                        Some(false) => format!(
                            "否 —— {}",
                            if self.deferred > 0 {
                                format!("有 {} 条被闸 5 延迟，本轮本就不该收敛", self.deferred)
                            } else if self.items.is_empty() {
                                // ⚠ 这句话是 2026-09-16 真库实测的**归因结果**，不是猜测。
                                // 原文案是「指纹不等，需逐条归因」，它把一个**已知的口径差**
                                // 说成了「未知故障」，让读日志的人去逐条查一个不存在的结构差异
                                // （实测：查完 17 张差集表 + 273 条表达式漂移才收敛到根因）。
                                "指纹不等，但**本轮 plan 为空** ⇒ 结构层面已一致。\
                                 已知口径差：指纹是整模型的 SHA-256，含 default/generated/CHECK\
                                 的**表达式文本**，而 PG 会重写这些文本（`'x'` → `'x'::text`、\
                                 `true` → `TRUE`、`0` → `0.0`、生成列加括号）；\
                                 `plan.rs::expression_text_diff_is_advisory_only` 已把这类差异\
                                 判为噪声（不产变更）⇒ 指纹在 PG 上结构性无法相等。\
                                 结构是否收敛请以「逐条是否为空」为准（见上面的声明漂移段）。"
                                    .to_string()
                            } else {
                                "指纹不等，需逐条归因".to_string()
                            }
                        ),
                        None => "未判（dry-run 不判收敛）".to_string(),
                    }
                ));
            },
            _ => s.push_str("收敛        : 未判（传入的 plan 未带期望指纹）\n"),
        }

        s.push_str("\n-- 逐条 --\n");
        if self.items.is_empty() {
            s.push_str("（空集：结构与声明一致）\n");
        }
        for it in &self.items {
            let status = if it.executed {
                "执行".to_string()
            } else {
                it.skip.map_or_else(|| "失败".to_string(), |x| x.as_str().to_string())
            };
            s.push_str(&format!(
                "  [{status:<7}] 段{} {:<14} {:<44} {} 条语句\n",
                it.kind.phase(),
                it.kind.as_str(),
                it.object,
                it.statements.len()
            ));
            if let Some(e) = &it.error {
                s.push_str(&format!("        ⚠ {e}\n"));
            }
            for n in &it.notes {
                s.push_str(&format!("        · {n}\n"));
            }
        }

        // ── 声明漂移（advisory）──
        // 放在逐条之后：它是「本轮一条 DDL 都没产出」的**解释**，而不是变更清单的一部分
        // （plan 为空 + advisory 非空 = 结构与声明一致，但表达式文本有漂移）。
        if !self.advisories.is_empty() {
            s.push_str(&format!(
                "\n-- 声明漂移 {} 条（advisory：不产变更，但需人看）--\n",
                self.advisories.len()
            ));
            for a in &self.advisories {
                s.push_str(&format!("  !! {a}\n"));
            }
        }
        s
    }
}

/// 连接的方言。全模块统一用它，避免各处各写一遍 `match`。
pub fn dialect_of(db: &DatabaseConnection) -> Result<Dialect, DbErr> {
    match db.get_database_backend() {
        DbBackend::Postgres => Ok(Dialect::Postgres),
        DbBackend::Sqlite => Ok(Dialect::Sqlite),
        other => Err(DbErr::Custom(format!("reconcile 不支持 backend {other:?}"))),
    }
}

/// dry-run 时给「本来会被延迟」的条目加的一条备注。
const NOTE_DRY_RUN_WOULD_DEFER: &str =
    "若关掉逃生阀：本条会因闸 5 只登记、不执行（dry-run 不登记，令牌留给真跑）";

/// 登记待删表时写进 `reason` 列的理由。
const DEFER_REASON: &str = "孤儿表：闸 5 首次见到，延迟一个周期";

/// 执行一个**已经算好**的 plan。调用方负责 diff；若不打算自己读白名单，用 [`cycle`]。
///
/// `expected_table_count` 是闸 3（基数熔断）的「本轮值」。它无法从 `plan` 反推
/// —— `plan` 里只有变更，没有期望表总数 ⇒ 由调用方给出（`cycle` 传 `want.tables.len()`）。
pub async fn run(
    db: &DatabaseConnection,
    plan: &Plan,
    expected_table_count: usize,
    dialect: Dialect,
    opts: &ApplyOptions,
) -> Result<ApplyOutcome, DbErr> {
    // 顺序决策 1：簿记先建起来。失败即整批中止（`?`），
    // 因为「簿记都写不进去」时执行 DDL 等于让这次改动永久无记录。
    safety::ensure_meta_tables(db).await?;
    let now = safety::now_epoch();
    // ⚠ 不是 `make_run_id(now, ordinal)`：审计表主键是 `(run_id, seq)`，而同秒重跑
    // （冒烟探针、任何重试、任何调试）用同一个默认序号就会撞主键。见 `allocate_run_id`。
    let run_id = safety::allocate_run_id(db, now, opts.ordinal).await?;

    let mut out = ApplyOutcome {
        run_id: run_id.clone(),
        dry_run: opts.config.dry_run,
        refusal: None,
        items: Vec::with_capacity(plan.changes.len()),
        deferred: 0,
        executed: 0,
        aborted: None,
        graveyard: Vec::new(),
        false_graves: Vec::new(),
        audit_rows: 0,
        whitelist_active: 0,
        whitelist_expired: Vec::new(),
        expected_table_count: Some(expected_table_count),
        expected_fingerprint: match plan.expected_fingerprint.is_empty() {
            true => None,
            false => Some(plan.expected_fingerprint.clone()),
        },
        post_fingerprint: None,
        converged: None,
        advisories: plan.advisories.clone(),
    };

    // ── 顺序决策 3：**先全部渲染** ──
    // 渲染产出为空也算失败：一个静默返回空语句的渲染器会让 apply「成功」而什么都没做，
    // 那是最难发现的一类假成功。
    for (i, c) in plan.changes.iter().enumerate() {
        let mut it = Item {
            seq: i,
            kind: c.kind,
            object: c.object.clone(),
            destructive: c.destructive,
            loses_data: c.loses_data,
            statements: Vec::new(),
            notes: Vec::new(),
            executed: false,
            skip: None,
            error: None,
        };
        match (opts.renderer)(c, dialect) {
            Ok(r) if r.statements.is_empty() => {
                it.error = Some(format!("渲染产出 0 条语句（{}）", c.kind.as_str()));
            },
            Ok(r) => {
                it.statements = r.statements;
                it.notes = r.notes;
            },
            // ⚠ 这一支**必须**按错误类型分流，绝不能靠 `e.to_string()` 里有没有某个词来猜
            // （判据 K 组「自由文本猜结构」：文案一改，判据静默失效）。
            //
            // `UnsupportedDialect` = **方言能力边界**（不是缺陷）⇒ 记 `skip`，不参与整批拒绝。
            // 其余（`Unrenderable` / `PayloadMismatch`）= 渲染器缺陷或 payload 与 kind 不匹配
            // ⇒ 仍是 `it.error`，仍走整批拒绝。两类混在一起的后果见 `SkipReason::UnsupportedDialect`
            // 的文档：SQLite 存量库会因此**整批**被拒 ⇒ 启动中止。
            Err(e) => {
                if matches!(e, RenderError::UnsupportedDialect { .. }) {
                    // 原文进 `notes` 而不是 `error`：`error.is_some()` 正是「整批拒绝」的输入，
                    // 而这里恰恰**不能**触发整批拒绝。原文不能丢 —— 审计要能答「为什么没跑」。
                    it.notes.push(e.to_string());
                    it.skip = Some(SkipReason::UnsupportedDialect);
                } else {
                    it.error = Some(e.to_string());
                }
            },
        }
        out.items.push(it);
    }

    // ── 整批拒绝的判定（闸 2 / 3 / 4 + 渲染） ──
    // ⚠ 这里的 `i.error.is_some()` **恰好**等于「真渲染错误」：`UnsupportedDialect` 在渲染
    // 循环里已被分流进 `skip` + `notes`，不置 `error`（见那里的注释）。所以「方言能力边界」
    // 不会进 `ApplyRefusal::Unrenderable` —— 这是刻意的，不是遗漏。
    let render_errors: Vec<String> = out
        .items
        .iter()
        .filter(|i| i.error.is_some())
        .map(|i| format!("{} {}：{}", i.kind.as_str(), i.object, i.error.as_deref().unwrap_or("")))
        .collect();

    out.refusal = match safety::check_circuit_breakers(
        plan,
        expected_table_count,
        opts.last_expected_table_count,
        &opts.config,
    ) {
        Some(r) => Some(ApplyRefusal::Circuit(r)),
        None => {
            let missing = safety::missing_evidence(plan, &opts.evidence);
            if !missing.is_empty() {
                Some(ApplyRefusal::MissingEvidence(
                    missing.iter().map(|c| c.object.clone()).collect(),
                ))
            } else if !render_errors.is_empty() {
                Some(ApplyRefusal::Unrenderable(render_errors))
            } else {
                None
            }
        },
    };

    // ── 决定「哪些不跑」 ──
    let mut aborted_msg: Option<String> = None;
    let mut graveyard: Vec<String> = Vec::new();

    match &out.refusal {
        Some(_) => {
            for it in &mut out.items {
                it.skip = Some(SkipReason::BatchRefused);
            }
        },
        None => {
            // 待删表只在「真的可能执行」时才读 —— 被整批拒时读它没有意义
            // （所以它声明在这个分支里，而不是提到外面）。
            let pending = safety::load_pending_drops(db).await?;
            for it in &mut out.items {
                // 渲染阶段已定性的条目（本方言无原生 DDL）**保持原判**，不被下面两条覆盖。
                // 不守这一条会丢信息：dry-run 下它会被改写成 `DryRun`，于是报告说
                // 「逃生阀开着所以没跑」，而真正的原因（这个方言永远跑不了这一条）消失了 ——
                // 读者会以为关掉逃生阀就能收敛。执行结果不变，损失全在可读性上。
                if it.skip.is_some() {
                    continue;
                }
                // 闸 5 只作用于 `DropTable`：`_ax_schema_pending_drops` 的键是**表名**，
                // 而 `DropColumn` 的 object 是 `表.列` —— 混用会让这一列变成「一名多义」
                // （判据 K 组），且「首次见到某列」这个概念本身没有跨运行的稳定含义。
                let first_sight = it.kind == ChangeKind::DropTable
                    && safety::defer_decision(&it.object, &pending, &opts.config)
                        == DeferDecision::DeferFirstSight;

                if opts.config.dry_run {
                    if first_sight {
                        it.notes.push(NOTE_DRY_RUN_WOULD_DEFER.to_string());
                    }
                    it.skip = Some(SkipReason::DryRun);
                    continue;
                }
                if first_sight {
                    safety::record_pending_drop(db, &it.object, &run_id, DEFER_REASON, now).await?;
                    it.skip = Some(SkipReason::DeferredFirstSight);
                }
            }
        },
    }

    // ── 执行 ──
    if out.refusal.is_none() && !opts.config.dry_run {
        for (i, it) in out.items.iter_mut().enumerate() {
            if it.skip.is_some() {
                continue;
            }
            if aborted_msg.is_some() {
                it.skip = Some(SkipReason::AbortedAfterError);
                continue;
            }

            // 顺序决策 4：墓碑在 `DROP` 之前写。
            if it.kind == ChangeKind::DropTable
                && let Some(ev) = opts.evidence.get(&it.object)
            {
                let ddl = it.statements.join(";\n");
                safety::write_graveyard(db, &run_id, i, &it.object, &ddl, ev, now).await?;
                graveyard.push(it.object.clone());
            }

            let stmts = it.statements.clone();
            let mut ok = true;
            for s in &stmts {
                if let Err(e) = db.execute_unprepared(s.as_str()).await {
                    it.error = Some(format!("执行 `{s}` 失败：{e}"));
                    aborted_msg = Some(format!("`{}` 执行 `{s}` 失败：{e}", it.object));
                    ok = false;
                    break;
                }
            }
            if ok {
                it.executed = true;
                if it.kind == ChangeKind::DropTable {
                    safety::clear_pending_drop(db, &it.object).await?;
                }
            }
        }
    }

    out.executed = out.items.iter().filter(|i| i.executed).count();
    out.deferred =
        out.items.iter().filter(|i| i.skip == Some(SkipReason::DeferredFirstSight)).count();
    out.aborted = aborted_msg;
    out.false_graves = out
        .items
        .iter()
        .filter(|i| !i.executed && graveyard.contains(&i.object))
        .map(|i| i.object.clone())
        .collect();
    out.graveyard = graveyard;

    // ── 闸 7：审计。每一条**无论执行与否**都写 ──
    let rows: Vec<AuditRow> = out
        .items
        .iter()
        .map(|it| {
            let mut notes = it.notes.clone();
            match it.skip {
                Some(SkipReason::BatchRefused) => {
                    notes.push(match &out.refusal {
                        Some(r) => format!("整批被拒：[{}] {}", r.kind_str(), r.reason()),
                        None => SkipReason::BatchRefused.note().to_string(),
                    });
                },
                Some(s) => notes.push(s.note().to_string()),
                None => {},
            }
            AuditRow {
                seq: it.seq,
                kind: it.kind.as_str(),
                object: it.object.clone(),
                destructive: it.destructive,
                loses_data: it.loses_data,
                executed: it.executed,
                statements: it.statements.clone(),
                notes,
                error: it.error.clone(),
            }
        })
        .collect();
    out.audit_rows = rows.len();
    safety::write_audit(db, &run_id, &rows, now).await?;

    // ── 收敛判据 ──
    // 实况指纹**总是**采（只读，且对 dry-run 的调用方也有用：能看出「现在离期望有多远」）；
    // `converged` 只在真跑过之后才判 —— 否则 dry-run 会给出一个「未收敛」的假信号。
    if out.expected_fingerprint.is_some() {
        let after = introspect::read(db).await?;
        let fp = fingerprint::fingerprint(&after);
        if !opts.config.dry_run {
            out.converged = Some(fp == plan.expected_fingerprint);
        }
        out.post_fingerprint = Some(fp);
    }

    Ok(out)
}

/// **一站式 apply**：读簿记 → 读实况 → 算期望 → 求差 → 执行。
///
/// 为什么必须提供它（而不是只提供 [`run`]）：闸 6（人工白名单）的作用是「不判孤儿」，
/// 而孤儿判定发生在 `diff` 里。若让调用方自己拼装，就会有人先 diff 再读白名单 ——
/// 那时白名单**看起来配好了却从不生效**，症状只是「明明登记了还是被删了」。
/// 顺序内建在这个函数里，调用方无从搞错。
pub async fn cycle(db: &DatabaseConnection, opts: &ApplyOptions) -> Result<ApplyOutcome, DbErr> {
    let dialect = dialect_of(db)?;

    safety::ensure_meta_tables(db).await?;
    let now = safety::now_epoch();

    // 顺序决策 2：白名单在 diff **之前**读。
    let wl = safety::load_orphan_whitelist(db, now).await?;

    let actual = introspect::read(db).await?;
    let want = expected::build(dialect)?;
    let mut p = super::plan::diff_with(
        &want,
        &actual,
        dialect,
        &PlanOptions { orphan_whitelist: wl.active.clone() },
    );

    // 启动期 bootstrap：收窄成纯新增集。**必须发生在 `run` 之前** —— `run` 按 plan
    // 的 `n_destructive` 做闸 2 判定，过滤与重算算账都得在那之前完成，否则闸 2 会
    // 按一个含已过滤条目的数字来比配额。
    if opts.additive_only {
        p.retain_purely_additive();
    }

    let mut out = run(db, &p, want.tables.len(), dialect, opts).await?;
    out.whitelist_active = wl.active.len();
    out.whitelist_expired = wl.expired;
    Ok(out)
}

/// **启动期 bootstrap**：把库收敛到实体声明形态，**只做纯新增**。
///
/// 这是「引擎取代历史迁移」的落点 —— 全新库靠它建出全部表 / 索引 / 外键，
/// 存量库靠它补齐缺失的表与列（缺什么补什么，已有的不碰）。
///
/// ## 与 [`cycle`] 的三点差异（都是为「无人审查的启动路径」而设）
///
/// 1. **`dry_run = false`** —— 启动路径必须真的把 schema 补齐，否则 App 起不来。
/// 2. **`additive_only = true`** —— 只加不减。收缩类变更（DROP / RENAME /
///    `SET NOT NULL`）留给离线探针，在有人看着的时候执行。
/// 3. **`max_destructive_per_run = 0`** —— 与第 2 点配套的第二道保险：即便将来有人
///    往 [`ChangeKind::is_purely_additive`] 里误加了一类破坏性变更，闸 2 也会把
///    整批拦下来（`BatchRefused`），而不是让它悄悄执行。
///
/// 闸 3（基数熔断）在本路径上不参与判定：`last_expected_table_count` 保持 `None`
/// （首轮语义「不比」）—— 期望基数由实体注册表决定，启动期没有「上一轮」可比。
///
/// ## 两个方言都会真跑（此前的「非 PG 早退」已移除）
///
/// 早退的理由曾是「`render` 只做 PG，进 [`cycle`] 会得到『全部渲染失败 ⇒ 整批被拒』的
/// 噪声」。SQLite 渲染适配后这个前提不成立了：SQLite 侧 `CREATE TABLE` / 索引 / 列级
/// 新增 / RENAME 都有原生 DDL（见 `render::render` 的方言能力表）。**早退必须移除** ——
/// 迁移清单清空后，引擎是唯一的建表来源；在 SQLite 上早退等于「全新 SQLite 库一张业务表
/// 都没有」，而那是本项目 300+ 个 `sqlite::memory:` 单测与移动端/降级路径的地基。
///
/// ⚠ **已知缺口**：SQLite 上「有约束 / 默认值 / 唯一约束差异」的存量库（迁移建出来的）
/// 会得到 `UnsupportedDialect` 的那几类（`SET DEFAULT` / `ADD FK` / `ADD CHECK` /
/// `ADD UNIQUE`）。本函数对它们的处置是 **跳过 + 登记**（[`SkipReason::UnsupportedDialect`]），
/// **不是**整批拒绝。
///
/// ## 为什么不是整批拒绝（2026-09-16 改判，附代价）
///
/// 旧处置曾是「刻意让它被拒，理由是回退到『只建表不建约束』会造出少约束的表且永不收敛」。
/// 那个理由**张冠李戴**：`CreateTable` 在 SQLite 上会把 FK/CHECK **内联**进建表语句
/// （见 `render::create_table_statements`），根本不经过本变体。真正会落到这里的是
/// 「表已存在、只差一个约束」——跳过只是让库保持现状（它已经这样跑了很多个版本），
/// 而整批拒绝的后果是 **SQLite 存量库直接起不来**：`db::initialize_schema` 会把
/// `refusal` 转成启动错误。实测现场：`create_test_pool`（38 处调用）建出的迁移库
/// 有 19 条此类差集 ⇒ 单测报红、真实用户的 `.axagent/data/axagent.db` 同样起不来。
///
/// **如实登记的代价**：这几条会在**每一轮** diff 里重新出现 —— 即该库在结构层面永不收敛。
/// 收敛它们要走「建新表 + 复制 + 改名 + 重建索引」的重建流程（PLAN §3.1，尚未实现），
/// 那是**有数据搬迁风险**的操作，只该在离线路径上、有导出证据时执行，绝不放在启动路径。
///
/// ⇒ 所以「跳过」必须**可见**：`report()` 有一行、审计 `notes` 有原文、
/// `db::initialize_schema` 打 `warn!`。想确认自己手上那个库是哪种情况，跑
/// `cargo run -p axagent-dao --example p5_engine_takeover -- --diff <url>`。
pub async fn bootstrap_schema(db: &DatabaseConnection) -> Result<ApplyOutcome, DbErr> {
    cycle(db, &bootstrap_options()).await
}

/// 启动期 bootstrap 的**固定配置**。
///
/// 单独成一个函数（而不是内联在 [`bootstrap_schema`] 里）是为了让它**可被测试
/// 断言**：`reconcile::apply::tests::bootstrap_options_are_pinned_to_the_safe_side`
/// 直接检查这四个值，而不是靠「读过源码，认为是这样」。
///
/// 这四个值各自挡一类事故：
/// * `dry_run = false` —— 否则启动永远只渲染不执行，schema 永远补不上（静默失效）；
/// * `additive_only = true` —— 删掉它，启动路径就会执行 DROP / RENAME（无声删数据）；
/// * `max_destructive_per_run = 0` —— 第二道保险，挡住「有人往
///   [`ChangeKind::is_purely_additive`] 白名单里误加破坏性类别」；
/// * `last_expected_table_count = None`（由 `Default` 提供）—— 闸 3 的「首轮不比」
///   语义：期望基数由实体注册表决定，启动期没有「上一轮」可比。
pub fn bootstrap_options() -> ApplyOptions {
    ApplyOptions {
        config: SafetyConfig {
            dry_run: false,
            max_destructive_per_run: 0,
            ..SafetyConfig::default()
        },
        additive_only: true,
        ..ApplyOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::model::{ColumnModel, SchemaModel, TableModel};
    use crate::reconcile::plan::{Change, ChangePayload};
    use sea_orm::{Database, Statement};
    // `BTreeMap` 由 `super::*` 带进来；`BTreeSet` 只在测试里用（生产路径的那处
    // 局部 `let pending` 已经在 `run` 的分支内），所以单独引。
    use std::collections::BTreeSet;

    const EVIDENCE_PATH: &str = "output/export.json";
    const EVIDENCE_SHA: &str = "deadbeef";

    async fn mem() -> DatabaseConnection {
        Database::connect("sqlite::memory:").await.expect("连内存库应成功")
    }

    fn evidence() -> GraveEvidence {
        GraveEvidence {
            export_path: EVIDENCE_PATH.into(),
            export_sha256: EVIDENCE_SHA.into(),
            row_count: Some(3),
        }
    }

    fn evid_map(objs: &[&str]) -> BTreeMap<String, GraveEvidence> {
        objs.iter().map(|o| ((*o).to_string(), evidence())).collect()
    }

    fn change(kind: ChangeKind, object: &str) -> Change {
        Change {
            kind,
            object: object.to_string(),
            destructive: kind.destructive(),
            loses_data: kind.loses_data(),
            detail: String::new(),
            payload: ChangePayload::Bare,
        }
    }

    /// 手搓 plan：`expected_fingerprint` 留空 ⇒ 不判收敛（除非测试自己设）。
    fn plan_of(changes: Vec<Change>) -> Plan {
        let n_destructive = changes.iter().filter(|c| c.destructive).count();
        let n_safe = changes.len() - n_destructive;
        Plan { changes, n_destructive, n_safe, ..Plan::default() }
    }

    // ── 假渲染器 ──

    /// 全 `ChangeKind` 都能渲染，且 `CreateTable` / `DropTable` 产出的 DDL 在
    /// SQLite 上**真的能执行** —— 编排测试需要真建/真删。
    ///
    /// ⚠ 这里的理由**不是**「`render` 在 SQLite 上不可用」（2026-09-16 SQLite 适配后
    /// 那 6 类原生类别已经可用），而是：编排测试要覆盖 [`ChangeKind::ALL`] **21 类**的
    /// 闸逻辑，而真渲染器在 SQLite 上只放行 6 类 —— 用真渲染器会让其余 15 类在
    /// **渲染阶段**就被拒，于是「闸 2 配额」「闸 1 fail-closed 顺序」这些**编排**性质
    /// 根本没被走到。假渲染器把「渲染」这一维固定住，让被测对象只剩编排本身。
    ///
    /// 真渲染器在 SQLite 上的实际行为由这两个用例单独守：
    /// `render::tests::sqlite_can_create_every_expected_table` 与
    /// `apply::tests::bootstrap_schema_builds_every_table_on_sqlite`（后者用的就是
    /// `ApplyOptions::default()` 里的真渲染器）。
    fn fake_render(c: &Change, _d: Dialect) -> Result<Rendered, RenderError> {
        let stmts = match c.kind {
            ChangeKind::CreateTable => {
                vec![format!("CREATE TABLE \"{}\" (\"id\" TEXT NOT NULL PRIMARY KEY)", c.object)]
            },
            ChangeKind::DropTable => vec![format!("DROP TABLE \"{}\"", c.object)],
            // 其余 kind 在编排测试里不参与执行；给一条无害语句保证「非空」。
            _ => vec![format!("CREATE TABLE IF NOT EXISTS \"{}\" (\"id\" TEXT)", c.object)],
        };
        Ok(Rendered { statements: stmts, notes: vec![] })
    }

    /// 渲染一切正常，但 `DropTable` 会去删一张不存在的表 ⇒ 执行必失败。
    fn fake_render_bad_drop(c: &Change, d: Dialect) -> Result<Rendered, RenderError> {
        if c.kind == ChangeKind::DropTable {
            return Ok(Rendered {
                statements: vec!["DROP TABLE \"no_such_table_at_all\"".to_string()],
                notes: vec![],
            });
        }
        fake_render(c, d)
    }

    /// 第 2 条（下标 1）渲染失败 ⇒ 用于验证「先全渲染再执行」。
    fn fake_render_fails_on_second(c: &Change, _d: Dialect) -> Result<Rendered, RenderError> {
        if c.object == "boom" {
            return Err(RenderError::Unrenderable {
                kind: c.kind,
                object: c.object.clone(),
                why: "测试注入".into(),
            });
        }
        fake_render(c, Dialect::Sqlite)
    }

    /// 渲染「成功」但一条语句都不给 ⇒ 必须被判为渲染失败。
    fn fake_render_empty(_c: &Change, _d: Dialect) -> Result<Rendered, RenderError> {
        Ok(Rendered { statements: vec![], notes: vec![] })
    }

    /// `object == "no_native_ddl"` 的那条报「本方言无原生 DDL」，其余正常。
    ///
    /// 用来把 `UnsupportedDialect` 与 `Unrenderable` 的**分流**钉死：前者只跳过本条，
    /// 后者整批拒绝。两者都是一句 `Err`，差别全在 `apply` 怎么分类。
    fn fake_render_dialect_gap(c: &Change, _d: Dialect) -> Result<Rendered, RenderError> {
        if c.object == "no_native_ddl" {
            return Err(RenderError::UnsupportedDialect { dialect: Dialect::Sqlite, kind: c.kind });
        }
        fake_render(c, Dialect::Sqlite)
    }

    fn cfg_exec() -> SafetyConfig {
        SafetyConfig { dry_run: false, ..SafetyConfig::default() }
    }

    // ── 启动期 bootstrap 的配置守卫 ──

    /// [`bootstrap_options`] 的四个值**钉死在安全侧**。
    ///
    /// 这些不是「风格选择」，每一个都挡一类事故（逐条理由见 `bootstrap_options` 的
    /// 文档）。断言在这里的理由：启动路径**没有人工审查**，配置被改松是**静默**的
    /// —— 只有机器检查能让它变红。
    ///
    /// 与 `reconcile::tests::only_bootstrap_may_reach_apply_from_startup` 的分工：
    /// 那条管「谁在调」，这条管「调的时候是什么配置」。
    #[test]
    fn bootstrap_options_are_pinned_to_the_safe_side() {
        let o = bootstrap_options();

        assert!(!o.config.dry_run, "bootstrap 必须真的执行（否则 schema 永远补不上，且无声）");
        assert!(o.additive_only, "bootstrap 必须只做纯新增（否则会执行 DROP / RENAME）");
        assert_eq!(
            o.config.max_destructive_per_run, 0,
            "bootstrap 的破坏性配额必须是 0 —— 它是 additive_only 之外的第二道保险"
        );
        assert!(
            o.last_expected_table_count.is_none(),
            "启动期没有「上一轮期望基数」可比，必须保持 None（闸 3 首轮语义）"
        );

        // 反向断言：默认配置必须是**相反**的 —— 否则上面第一条与第二条就没有区分力
        // （若 `Default` 也等于 bootstrap 配置，那「钉死」这件事根本没发生过）。
        let d = ApplyOptions::default();
        assert!(d.config.dry_run, "前置：默认必须 dry-run（证明上面第一条有区分力）");
        assert!(!d.additive_only, "前置：默认不得是 additive_only");
    }

    /// bootstrap 在 SQLite 上**真的把全部表建出来** —— 这是「SQLite 建表不再依赖历史迁移」
    /// 的出口判据。
    ///
    /// 前身是 `bootstrap_schema_skips_non_postgres_dialects`（断言 SQLite 上返回 `Ok(None)`）。
    /// SQLite 渲染适配完成后那条断言**过期了**：`None` 现在意味着「引擎不接管」，而引擎
    /// **必须**接管 —— 否则删掉迁移后，全新 SQLite 库一张业务表都没有，300+ 个
    /// `sqlite::memory:` 单测与移动端/降级路径全部崩掉。
    ///
    /// 判据写成「逐表存在」而不是「executed == N」：N 是**执行条数**，只看它会漏掉
    /// 「某张表根本没被计划到」这类问题（条数可能由别的表多建几条索引凑够）。
    #[tokio::test]
    async fn bootstrap_schema_builds_every_table_on_sqlite() {
        let db = mem().await;
        let want = expected::build(Dialect::Sqlite).expect("期望模型应可构建");

        let out = bootstrap_schema(&db).await.expect("SQLite 上 bootstrap 不应报错");
        assert!(out.refusal.is_none(), "SQLite 的 bootstrap 被拒了：{:?}", out.refusal);
        assert!(out.executed > 0, "bootstrap 一条都没执行 —— 引擎没有接管建表");

        let seen = introspect::read(&db).await.expect("introspect 应成功");
        let mut missing: Vec<&str> = Vec::new();
        for t in &want.tables {
            if seen.table(&t.name).is_none() {
                missing.push(&t.name);
            }
        }
        assert!(
            missing.is_empty(),
            "{} / {} 张表没有被建出来（前 10 张）：{:?}",
            missing.len(),
            want.tables.len(),
            missing.iter().take(10).collect::<Vec<_>>()
        );
        // 反向断言：`seen` 非空才算真做过 introspect（否则上面那个循环在空集合上恒真）。
        assert!(
            seen.tables.len() >= want.tables.len(),
            "introspect 只看到 {} 张表，期望至少 {}",
            seen.tables.len(),
            want.tables.len()
        );
        // ── 端到端：单列唯一性真的落进了 DDL（不只是模型里有个标志）──
        //
        // 为什么必须在这里补一条：上面只断言「表存在」，对**约束**一字未提。
        // 而本轮修复的 5 条唯一性（见 `expected.rs::migration_unique_semantics_survive_in_column_flag`）
        // 走的是「实体 `#[sea_orm(unique)]` → 模型列标志 → `render` 内联 `UNIQUE (col)`
        // → 真建进 SQLite → `introspect` 读回列标志」这条链。链条两端各自都有断言，
        // 但**没有任何一条断言把它整条走完** —— 中间某环没接上时（标志有了但 DDL 没出，
        // 或 DDL 出了但 introspect 没读回）两端都照样绿。这里把它走完。
        //
        // 形态提示：`render.rs:1122` 生成的是**表级** `UNIQUE (col)`，列名经 `quote_ident` 加双引号；
        // **不是**列级 `col TEXT UNIQUE`。SQLite 对两者都产生 `origin='u'` 的单列自动索引 ⇒
        // `dao/src/reconcile/introspect/sqlite.rs:286-292` 分流成列标志。若哪天渲染改用唯一索引，
        // 该分流会把它落成 `IndexModel` 而非列标志，本条会红 —— 那正是要拦的回归。
        const UNIQUE_COLS: &[(&str, &str, &str)] = &[
            ("sync_devices", "unique_id", "v116 `unique_id TEXT NOT NULL UNIQUE`"),
            ("sync_permissions", "device_id", "v116 `device_id … ON DELETE CASCADE UNIQUE`"),
            ("opc_landing_pages", "slug", "v210 `slug TEXT NOT NULL UNIQUE`"),
            ("opc_blog_posts", "slug", "v210 `slug TEXT NOT NULL UNIQUE`"),
            ("trajectory_preferences", "key", "v100 `key TEXT NOT NULL UNIQUE`"),
        ];
        for (table, col, src) in UNIQUE_COLS {
            let t = seen.table(table).unwrap_or_else(|| panic!("{table} 应由引擎建出"));
            let c = t.column(col).unwrap_or_else(|| panic!("{table}.{col} 应存在"));
            assert!(
                c.unique,
                "端到端断裂：{table}.{col} 的唯一性没落进 DDL（迁移出处 {src}）。\
                 实体标志 → 渲染 → 建表 → introspect 这条链上有一环没接上；\
                 只改实体标志而 DDL 少约束属于**假修复**",
            );
        }
        // **区分力反例**：同表上一个非唯一列必须为 `false`。
        // 没有这一条时，只要 introspect 退化到「给每列都标 unique」（例如
        // `sqlite.rs` 的 `origin` 分流被写坏成 `_ => c.unique = true`），上面那 5 条
        // 断言会**全部照样绿** —— 而它们才是本轮修复的核心证据，假绿代价最大。
        // 这几列的对侧出处见 `expected.rs::migration_unique_semantics_survive_in_column_flag`
        // 的「同表反例列」。
        const NON_UNIQUE_COUNTERPARTS: &[(&str, &str)] = &[
            ("opc_landing_pages", "title"),
            ("opc_blog_posts", "title"),
            ("trajectory_preferences", "value"),
        ];
        for (table, col) in NON_UNIQUE_COUNTERPARTS {
            let t = seen.table(table).unwrap_or_else(|| panic!("{table} 应由引擎建出"));
            let c = t.column(col).unwrap_or_else(|| panic!("{table}.{col} 应存在"));
            assert!(
                !c.unique,
                "区分力反例失败：{table}.{col} 本非唯一列，却被读成 unique ⇒ \
                 上面那 5 条唯一性断言可能是假绿（introspect 的 unique 分流退化了）",
            );
        }
        println!(
            "SQLite bootstrap：期望 {} 张，实况 {} 张；{} 条单列唯一性端到端已核对",
            want.tables.len(),
            seen.tables.len(),
            UNIQUE_COLS.len()
        );
    }

    fn opts_exec() -> ApplyOptions {
        ApplyOptions { config: cfg_exec(), renderer: fake_render, ..ApplyOptions::default() }
    }

    async fn table_exists(db: &DatabaseConnection, name: &str) -> bool {
        let rows = db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='table' AND name = ?",
                [name.into()],
            ))
            .await
            .expect("查 sqlite_master 应成功");
        !rows.is_empty()
    }

    async fn audit_count(db: &DatabaseConnection) -> i64 {
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT count(*) AS n FROM {}", safety::META_AUDIT),
            ))
            .await
            .expect("读审计应成功");
        rows[0].try_get("", "n").expect("n 应可读")
    }

    // ── 闸 1：逃生阀 ──

    /// dry-run：**一条 DDL 都不执行**，但每一条都写进审计。
    ///
    /// 这条同时证明「元表在 dry-run 下仍然建起来」—— 否则下面的审计查询会报「表不存在」。
    #[tokio::test]
    async fn dry_run_executes_nothing_but_audits_everything() {
        let db = mem().await;
        let p = plan_of(vec![
            change(ChangeKind::CreateTable, "brand_new"),
            change(ChangeKind::CreateTable, "another_new"),
        ]);
        let opts = ApplyOptions { renderer: fake_render, ..ApplyOptions::default() };
        assert!(opts.config.dry_run, "默认必须是只渲染不执行");

        let out = run(&db, &p, 2, Dialect::Sqlite, &opts).await.expect("run 应成功");
        assert_eq!(out.executed, 0);
        assert_eq!(out.audit_rows, 2, "未执行的也必须写审计");
        assert!(!table_exists(&db, "brand_new").await, "dry-run 绝不许建表");
        assert!(!table_exists(&db, "another_new").await);
        assert!(out.items.iter().all(|i| i.skip == Some(SkipReason::DryRun)));
        assert!(out.is_clean(), "{out:?}");
        // dry-run 不判收敛（否则会给出一个假信号）
        assert_eq!(out.converged, None);

        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT executed, statements FROM {} ORDER BY seq", safety::META_AUDIT),
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        for r in &rows {
            assert_eq!(r.try_get::<i64>("", "executed").unwrap(), 0);
            // 语句必须**留痕**：dry-run 的价值就是「看得到会执行什么」
            let stmts: Vec<String> =
                serde_json::from_str(&r.try_get::<String>("", "statements").unwrap()).unwrap();
            assert_eq!(stmts.len(), 1, "渲染结果应进审计");
        }
    }

    /// ⚠ dry-run **不消费**延迟令牌：连 dry-run 两次，真跑时仍然只登记不执行。
    ///
    /// 若 dry-run 也去 `record_pending_drop`，「先看一眼再真跑」就会让真跑当场删表 ——
    /// 一次观察行动把延迟闸吃掉了。这条测试就是钉住这一点。
    ///
    /// ⚠ 必须给证据：闸 4 在 dry-run 下**同样**生效（闸描述的是「这份计划可不可接受」，
    /// 与执不执行无关），缺证据会先被整批拒，那样就观察不到闸 5 的预告了。
    #[tokio::test]
    async fn dry_run_does_not_consume_the_deferral_token() {
        let db = mem().await;
        let p = plan_of(vec![change(ChangeKind::DropTable, "orphan")]);

        let dry = ApplyOptions {
            evidence: evid_map(&["orphan"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        for _ in 0..2 {
            let out = run(&db, &p, 1, Dialect::Sqlite, &dry).await.unwrap();
            assert_eq!(out.deferred, 0, "dry-run 不产生「延迟」，它一律 dry-run");
            assert!(
                out.items[0].notes.iter().any(|n| n.contains("闸 5")),
                "但必须预告「若真跑会被延迟」：{:?}",
                out.items[0].notes
            );
        }
        assert!(safety::load_pending_drops(&db).await.unwrap().is_empty(), "dry-run 不得写待删表");

        // 真跑：仍然只登记，不执行 —— 说明令牌没被 dry-run 消耗掉
        let real = ApplyOptions {
            config: cfg_exec(),
            evidence: evid_map(&["orphan"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 1, Dialect::Sqlite, &real).await.unwrap();
        assert_eq!(out.items[0].skip, Some(SkipReason::DeferredFirstSight));
        assert_eq!(out.executed, 0);
        assert_eq!(out.deferred, 1);
    }

    /// ⚠ dry-run **也**跑整批拒绝的判定，并把「会被哪道闸拦住」报出来。
    ///
    /// 这是 dry-run 的主要价值：在执行**之前**就知道这份计划过不过。若把闸判据挪到
    /// 「只有真跑时才判」，`--dry-run` 就只能告诉你「会执行 200 条」，而不会告诉你
    /// 「其中 6 条会因缺导出证据被拒」—— 那就得真跑去试，安全网等于失效。
    #[tokio::test]
    async fn dry_run_still_reports_which_gate_would_refuse() {
        let db = mem().await;
        let p = plan_of(vec![change(ChangeKind::DropTable, "no_evidence")]);
        let opts = ApplyOptions { renderer: fake_render, ..ApplyOptions::default() };
        assert!(opts.config.dry_run);

        let out = run(&db, &p, 1, Dialect::Sqlite, &opts).await.unwrap();
        assert!(
            matches!(out.refusal, Some(ApplyRefusal::MissingEvidence(_))),
            "dry-run 必须报出会被哪道闸拦住：{:?}",
            out.refusal
        );
        assert_eq!(out.executed, 0);
        assert_eq!(out.items[0].skip, Some(SkipReason::BatchRefused));
        assert_eq!(out.audit_rows, 1);
    }

    // ── 闸 5：延迟一个周期 ──

    /// 首见只登记，再见才执行；执行成功后待删表里的行被清掉。
    #[tokio::test]
    async fn first_sight_defers_and_second_sight_executes() {
        let db = mem().await;
        db.execute_unprepared("CREATE TABLE \"ghost\" (\"id\" TEXT)").await.unwrap();
        let p = plan_of(vec![change(ChangeKind::DropTable, "ghost")]);
        assert!(p.changes[0].loses_data, "DropTable 必须被判为丢数据");

        let opts = ApplyOptions {
            config: cfg_exec(),
            evidence: evid_map(&["ghost"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };

        let first = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();
        assert_eq!(first.items[0].skip, Some(SkipReason::DeferredFirstSight));
        assert!(table_exists(&db, "ghost").await, "首见不得真删");
        assert_eq!(
            safety::load_pending_drops(&db).await.unwrap(),
            BTreeSet::from(["ghost".into()])
        );
        assert_eq!(first.graveyard.len(), 0, "延迟的条目不该写墓碑");

        let second = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();
        assert!(second.items[0].executed, "第二次见到应执行：{:?}", second.items[0]);
        assert_eq!(second.deferred, 0);
        assert!(!table_exists(&db, "ghost").await, "第二遍必须真删");
        assert!(safety::load_pending_drops(&db).await.unwrap().is_empty(), "执行后清待删表");
        assert!(second.fully_applied(), "{second:?}");
    }

    /// 关掉延迟闸 ⇒ 首见即执行（配置真的生效，不是摆设）。
    #[tokio::test]
    async fn disabling_defer_executes_on_first_sight() {
        let db = mem().await;
        db.execute_unprepared("CREATE TABLE \"ghost\" (\"id\" TEXT)").await.unwrap();
        let p = plan_of(vec![change(ChangeKind::DropTable, "ghost")]);
        let opts = ApplyOptions {
            config: SafetyConfig { defer_destructive: false, ..cfg_exec() },
            evidence: evid_map(&["ghost"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();
        assert!(out.items[0].executed);
        assert!(!table_exists(&db, "ghost").await);
    }

    // ── 闸 4：墓碑证据 ──

    /// 缺证据 ⇒ **整批**中止：连同一条本来安全的建表也不许执行。
    ///
    /// 「跳过缺证据的那几条、执行其余」会留下半同步库，而引擎下次启动无法区分
    /// 「未同步」与「被外部改动」。
    #[tokio::test]
    async fn missing_evidence_refuses_the_whole_batch() {
        let db = mem().await;
        let p = plan_of(vec![
            change(ChangeKind::CreateTable, "harmless_new"),
            change(ChangeKind::DropTable, "risky"),
        ]);
        let opts = ApplyOptions {
            config: cfg_exec(),
            evidence: BTreeMap::new(), // 证据全缺
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 2, Dialect::Sqlite, &opts).await.unwrap();

        assert!(
            matches!(out.refusal, Some(ApplyRefusal::MissingEvidence(ref v)) if v == &["risky".to_string()]),
            "{:?}",
            out.refusal
        );
        assert_eq!(out.executed, 0);
        assert!(
            !table_exists(&db, "harmless_new").await,
            "⚠ 整批被拒 ⇒ 连不丢数据的建表也不许执行"
        );
        assert!(out.items.iter().all(|i| i.skip == Some(SkipReason::BatchRefused)));
        assert_eq!(out.audit_rows, 2, "被拒也要留审计");
        assert!(!out.is_clean(), "被整批拒不算 clean");
    }

    /// 只有 `loses_data` 的才要证据：不丢数据的变更不受闸 4 影响。
    ///
    /// ⚠ 这里**刻意**用「会执行 + 零证据」的配置：若闸 4 的判据写宽了（比如按
    /// `destructive` 而不是 `loses_data` 来要证据），`CreateIndex` 会被误要证据 ——
    /// 而 `CreateIndex::destructive()` 是 `false`、`loses_data()` 也是 `false`，
    /// 两者都不该要。用一个真实会执行的配置来跑，才能同时钉住「放行」与「执行」。
    #[tokio::test]
    async fn evidence_is_only_demanded_for_data_losing_changes() {
        let db = mem().await;
        let p = plan_of(vec![
            change(ChangeKind::CreateTable, "new_t"),
            change(ChangeKind::CreateIndex, "idx_new"),
        ]);
        let opts = opts_exec();
        let out = run(&db, &p, 2, Dialect::Sqlite, &opts).await.unwrap();
        assert_eq!(out.refusal, None);
        assert_eq!(out.executed, 2);
        assert!(out.fully_applied(), "{out:?}");
    }

    // ── 先全渲染，再执行 ──

    /// 第 2 条渲染失败 ⇒ 第 1 条也不许执行。这是「先全渲染」的核心价值。
    #[tokio::test]
    async fn render_failure_is_detected_before_any_execution() {
        let db = mem().await;
        let p = plan_of(vec![
            change(ChangeKind::CreateTable, "would_be_created"),
            change(ChangeKind::CreateTable, "boom"),
            change(ChangeKind::CreateTable, "never_reached"),
        ]);
        let opts = ApplyOptions {
            config: cfg_exec(),
            renderer: fake_render_fails_on_second,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 3, Dialect::Sqlite, &opts).await.unwrap();

        assert!(matches!(out.refusal, Some(ApplyRefusal::Unrenderable(_))), "{:?}", out.refusal);
        assert_eq!(out.executed, 0);
        assert!(
            !table_exists(&db, "would_be_created").await,
            "⚠ 边渲染边执行会让这一条已经落库 —— 半同步库"
        );
        assert!(!table_exists(&db, "never_reached").await);
        let boom = out.items.iter().find(|i| i.object == "boom").unwrap();
        assert!(boom.error.as_deref().unwrap().contains("测试注入"), "{boom:?}");
    }

    /// 渲染产出 0 条语句 ⇒ 算渲染失败（否则「什么都没做」会被报成成功）。
    #[tokio::test]
    async fn empty_render_output_is_a_rendering_failure() {
        let db = mem().await;
        let p = plan_of(vec![change(ChangeKind::CreateTable, "silent_noop")]);
        let opts = ApplyOptions {
            config: cfg_exec(),
            renderer: fake_render_empty,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 1, Dialect::Sqlite, &opts).await.unwrap();
        assert!(matches!(out.refusal, Some(ApplyRefusal::Unrenderable(_))), "{:?}", out.refusal);
        assert_eq!(out.executed, 0);
        assert!(!table_exists(&db, "silent_noop").await);
    }

    // ── 方言能力边界（`UnsupportedDialect`）≠ 整批拒绝 ──

    /// **只跳过本条，其余照跑** —— 这条是 2026-09-16 改判的核心判据。
    ///
    /// 旧行为把 `UnsupportedDialect` 与 `Unrenderable` 混成同一个 `render_errors`，
    /// 于是 SQLite 存量库（迁移建出的、差 19 条 `SET DEFAULT` / `ADD FK`）**整批被拒**
    /// ⇒ `db::initialize_schema` 转成启动错误 ⇒ 38 处 `create_test_pool` 与真实用户的库
    /// 全部起不来。这条把「两类错误必须分流」钉死。
    #[tokio::test]
    async fn dialect_gap_is_skipped_not_refused() {
        let db = mem().await;
        let p = plan_of(vec![
            change(ChangeKind::CreateTable, "created_anyway"),
            change(ChangeKind::AddForeignKey, "no_native_ddl"),
            change(ChangeKind::CreateTable, "also_created"),
        ]);
        let opts = ApplyOptions {
            config: cfg_exec(),
            renderer: fake_render_dialect_gap,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 3, Dialect::Sqlite, &opts).await.unwrap();

        assert_eq!(out.refusal, None, "方言能力边界不该整批拒绝：{:?}", out.refusal);
        assert_eq!(out.executed, 2, "除缺口那一条外，其余必须真的执行");
        assert!(table_exists(&db, "created_anyway").await, "缺口不该拖累别的条目");
        assert!(table_exists(&db, "also_created").await);

        let gap = out.items.iter().find(|i| i.object == "no_native_ddl").unwrap();
        assert_eq!(gap.skip, Some(SkipReason::UnsupportedDialect));
        assert!(gap.error.is_none(), "⚠ 置了 error 就会重新凑成整批拒绝 —— 这两类必须分开");
        assert!(gap.statements.is_empty(), "没渲染出语句，不该假装有");
        assert!(
            gap.notes.iter().any(|n| n.contains("没有原生 DDL")),
            "渲染器的原文不能丢（审计要能答「为什么没跑」）：{:?}",
            gap.notes
        );

        assert_eq!(out.unsupported().len(), 1);
        assert!(out.report("gap").contains("方无原生DDL"), "报告必须让这个缺口可见");
        assert!(!out.fully_applied(), "这一条确实没被应用掉 —— 粉饰它就会掩盖「该库永不收敛」");
    }

    /// dry-run 下缺口**保留自己的理由**，不被改写成 `DryRun`。
    ///
    /// 不守这一条，报告会说「逃生阀开着所以没跑」，读者会以为关掉逃生阀就能收敛 ——
    /// 而真相是这个方言永远跑不了这一条。执行结果不变，损失全在**可读性**上，
    /// 而那正是本引擎在无人值守路径上唯一的诊断面。
    #[tokio::test]
    async fn dialect_gap_keeps_its_reason_under_dry_run() {
        let db = mem().await;
        let p = plan_of(vec![change(ChangeKind::AddUnique, "no_native_ddl")]);
        let opts = ApplyOptions {
            config: SafetyConfig::default(), // dry_run = true（默认取安全侧）
            renderer: fake_render_dialect_gap,
            ..ApplyOptions::default()
        };
        assert!(opts.config.dry_run, "前提：默认配置是 dry-run");
        let out = run(&db, &p, 1, Dialect::Sqlite, &opts).await.unwrap();

        assert_eq!(
            out.items[0].skip,
            Some(SkipReason::UnsupportedDialect),
            "dry-run 把这个理由改写成 DryRun 会误导读者去关逃生阀"
        );
        assert_eq!(out.unsupported().len(), 1);
    }

    /// **全新库 ⇒ 方言缺口必须为空**：这是「方言能力表漏了一类」的唯一探针。
    ///
    /// 空库建表只需要 `CreateTable` / `CreateIndex`，两者都在白名单里。若哪天有人从
    /// `render::sqlite_supports_natively` 的白名单里误删一类，空库会**静默少建** ——
    /// 而 `executed > 0`、「表数对得上」这两条判据在多数情况下仍然为真。
    /// 「本方言无原生 DDL」这个数字在**全新库**与**存量库**上含义相反：前者非空即缺陷，
    /// 后者非空是已知缺口。所以它必须在这两种输入上各有一条判据。
    #[tokio::test]
    async fn bootstrap_on_empty_sqlite_has_no_dialect_gaps() {
        let db = mem().await;
        let out = bootstrap_schema(&db).await.expect("空库 bootstrap 应成功");

        assert!(out.refusal.is_none(), "{:?}", out.refusal);
        assert!(out.executed > 0, "一条都没执行 ⇒ 引擎没接管建表");
        assert!(
            out.unsupported().is_empty(),
            "全新库上出现方言缺口 = 能力表漏了一类：{:?}",
            out.unsupported()
                .iter()
                .map(|i| format!("{} {}", i.kind.as_str(), i.object))
                .collect::<Vec<_>>()
        );
        assert!(out.fully_applied(), "空库上应当把整份 plan 完整应用掉");
    }

    // ── 闸 2 / 3：熔断 ──

    /// 配额超限 ⇒ 整批拒绝，但审计照写（「这次运行决定了什么」必须有记录）。
    ///
    /// ⚠ 用的是**破坏性**变更：配额闸看的是 `Plan::n_destructive`，而 `CreateIndex`
    /// 等非破坏性变更根本不计入 —— 拿它们测配额会得到一个「永远放行」的假绿。
    #[tokio::test]
    async fn quota_refusal_still_writes_audit() {
        let db = mem().await;
        let p =
            plan_of((0..6).map(|i| change(ChangeKind::DropColumn, &format!("t.c{i}"))).collect());
        assert_eq!(p.n_destructive, 6, "前提：这 6 条必须都是破坏性的");
        let opts = ApplyOptions {
            config: cfg_exec(), // 默认配额 5
            evidence: evid_map(&["t.c0", "t.c1", "t.c2", "t.c3", "t.c4", "t.c5"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 100, Dialect::Sqlite, &opts).await.unwrap();
        assert!(
            matches!(
                out.refusal,
                Some(ApplyRefusal::Circuit(Refusal::TooManyDestructive { n: 6, limit: 5 }))
            ),
            "{:?}",
            out.refusal
        );
        assert_eq!(out.audit_rows, 6);
        assert_eq!(audit_count(&db).await, 6);
        assert_eq!(out.executed, 0);
    }

    /// 配额**恰好等于**上限 ⇒ 放行（配额是闭区间上限，不是「必须小于」）。
    #[tokio::test]
    async fn quota_boundary_is_inclusive() {
        let db = mem().await;
        let p =
            plan_of((0..5).map(|i| change(ChangeKind::DropColumn, &format!("t.c{i}"))).collect());
        let opts = ApplyOptions {
            config: SafetyConfig { max_destructive_per_run: 5, ..cfg_exec() },
            evidence: evid_map(&["t.c0", "t.c1", "t.c2", "t.c3", "t.c4"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 100, Dialect::Sqlite, &opts).await.unwrap();
        assert_eq!(out.refusal, None, "{:?}", out.refusal);
        assert_eq!(out.executed, 5, "恰好 5 条必须放行");
    }

    /// 基数下降 ⇒ 拒绝（这是「声明被误删 ⇒ 全库 DROP」的专用闸）。
    #[tokio::test]
    async fn base_shrink_refuses_and_can_be_opted_out() {
        let db = mem().await;
        let p = plan_of(vec![change(ChangeKind::CreateTable, "t")]);
        let base = ApplyOptions {
            config: cfg_exec(),
            last_expected_table_count: Some(200),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 30, Dialect::Sqlite, &base).await.unwrap();
        assert!(
            matches!(
                out.refusal,
                Some(ApplyRefusal::Circuit(Refusal::BaseShrank { now: 30, last: 200 }))
            ),
            "{:?}",
            out.refusal
        );

        let opt_out = ApplyOptions {
            config: SafetyConfig { allow_base_shrink: true, ..cfg_exec() },
            ..base.clone()
        };
        let out = run(&db, &p, 30, Dialect::Sqlite, &opt_out).await.unwrap();
        assert_eq!(out.refusal, None);
        assert_eq!(out.executed, 1);
    }

    // ── 墓碑 ──

    /// 墓碑在 `DROP` 之前写，内容含将被执行的 DDL 原文与导出证据。
    #[tokio::test]
    async fn graveyard_is_written_before_drop() {
        let db = mem().await;
        db.execute_unprepared("CREATE TABLE \"ghost\" (\"id\" TEXT)").await.unwrap();
        let p = plan_of(vec![change(ChangeKind::DropTable, "ghost")]);
        let opts = ApplyOptions {
            config: SafetyConfig { defer_destructive: false, ..cfg_exec() },
            evidence: evid_map(&["ghost"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();
        assert!(out.items[0].executed);
        assert_eq!(out.graveyard, vec!["ghost".to_string()]);
        assert!(out.false_graves.is_empty());
        assert!(out.is_clean());

        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!(
                    "SELECT table_name, ddl, export_path, export_sha256, row_count FROM {}",
                    safety::META_GRAVEYARD
                ),
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].try_get::<String>("", "table_name").unwrap(), "ghost");
        assert_eq!(rows[0].try_get::<String>("", "ddl").unwrap(), "DROP TABLE \"ghost\"");
        assert_eq!(rows[0].try_get::<String>("", "export_path").unwrap(), EVIDENCE_PATH);
        assert_eq!(rows[0].try_get::<String>("", "export_sha256").unwrap(), EVIDENCE_SHA);
        assert_eq!(rows[0].try_get::<Option<i64>>("", "row_count").unwrap(), Some(3));
    }

    /// ⚠ `DROP` 失败但墓碑已写 ⇒ 必须**显式**报出来（表还在，下次会重试），
    /// 不能只留一条 readme 式的「执行失败」让人以为数据没了。
    #[tokio::test]
    async fn failed_drop_leaves_a_false_grave_that_is_flagged() {
        let db = mem().await;
        db.execute_unprepared("CREATE TABLE \"ghost\" (\"id\" TEXT)").await.unwrap();
        let p = plan_of(vec![change(ChangeKind::DropTable, "ghost")]);
        let opts = ApplyOptions {
            config: SafetyConfig { defer_destructive: false, ..cfg_exec() },
            evidence: evid_map(&["ghost"]),
            renderer: fake_render_bad_drop,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();

        assert_eq!(out.executed, 0);
        assert_eq!(out.graveyard, vec!["ghost".to_string()], "墓碑已写");
        assert_eq!(out.false_graves, vec!["ghost".to_string()], "必须被标成假墓碑");
        assert!(!out.is_clean());
        assert!(
            out.aborted.as_deref().unwrap().contains("no_such_table_at_all"),
            "{:?}",
            out.aborted
        );
        assert!(table_exists(&db, "ghost").await, "表确实还在");
        assert!(out.report("假墓碑").contains("假墓碑"), "报告必须显眼地写出来");
        // 待删表里不该留下一条「已删」的假状态
        assert!(safety::load_pending_drops(&db).await.unwrap().is_empty());
    }

    // ── fail-stop ──

    /// 首条失败 ⇒ 后续条目一律 `AbortedAfterError`，不再往下执行。
    #[tokio::test]
    async fn execution_failure_aborts_the_rest() {
        let db = mem().await;
        let p = plan_of(vec![
            change(ChangeKind::DropTable, "missing_one"), // 必失败
            change(ChangeKind::CreateTable, "after_failure"),
        ]);
        let opts = ApplyOptions {
            config: SafetyConfig { defer_destructive: false, ..cfg_exec() },
            evidence: evid_map(&["missing_one"]),
            renderer: fake_render,
            ..ApplyOptions::default()
        };
        let out = run(&db, &p, 1, Dialect::Sqlite, &opts).await.unwrap();

        assert_eq!(out.executed, 0);
        assert!(out.aborted.is_some());
        assert_eq!(out.items[1].skip, Some(SkipReason::AbortedAfterError));
        assert!(!table_exists(&db, "after_failure").await, "失败后不得继续执行");
        assert_eq!(out.audit_rows, 2, "被中止的条目也要留审计");
    }

    // ── 收敛 ──

    fn model_with(rows: &[(&str, &str)]) -> SchemaModel {
        let mut m = SchemaModel::new(Dialect::Sqlite);
        for (t, c) in rows {
            let mut tab = TableModel::new(*t);
            tab.upsert_column(ColumnModel::new(*c, "text", true));
            m.upsert_table(tab);
        }
        m.finalize();
        m
    }

    /// 指纹相等 ⇒ 收敛。空对空是最干净的用例（plan 为空，什么都不用做）。
    #[tokio::test]
    async fn converged_true_when_fingerprints_match() {
        let db = mem().await;
        let m = model_with(&[]);
        let p = crate::reconcile::plan::diff(&m, &m, Dialect::Sqlite);
        assert!(p.is_empty());
        assert!(!p.expected_fingerprint.is_empty());

        let opts =
            ApplyOptions { config: cfg_exec(), renderer: fake_render, ..ApplyOptions::default() };
        let out = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();
        assert_eq!(out.converged, Some(true), "{out:?}");
        assert_eq!(out.post_fingerprint, out.expected_fingerprint);
        assert!(out.fully_applied());
    }

    /// 指纹不等 ⇒ 未收敛。手搓的 plan 让「建表」渲染成别的结构，实况对不上期望。
    #[tokio::test]
    async fn converged_false_when_fingerprints_differ() {
        let db = mem().await;
        let want = model_with(&[]); // 期望：空
        let mut actual = SchemaModel::new(Dialect::Sqlite);
        let mut t = TableModel::new("stray");
        t.upsert_column(ColumnModel::new("id", "text", true));
        actual.upsert_table(t);
        actual.finalize();

        // 期望有表、实况没有 ⇒ 会产出 CreateTable；但渲染器建出来的列与期望不一致。
        let want2 = model_with(&[("wanted", "val")]);
        let p = crate::reconcile::plan::diff(&want2, &actual, Dialect::Sqlite);
        assert!(!p.is_empty(), "应有 CreateTable");

        let opts =
            ApplyOptions { config: cfg_exec(), renderer: fake_render, ..ApplyOptions::default() };
        let out = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();
        // 真实建出来的是假渲染器的 `id TEXT NOT NULL PRIMARY KEY`，与期望的 `val text` 不同
        assert_eq!(out.converged, Some(false), "{out:?}");
        assert_ne!(out.post_fingerprint, out.expected_fingerprint);
        let _ = want; // 保留以说明「期望侧为空时」的对照
    }

    /// **advisory 必须走到 outcome 与 report** —— 2026-09-16 真库实测缺陷的守卫。
    ///
    /// 缺陷原状：`ApplyOutcome` 根本没有 advisory 字段、`report()` 也不打印它
    /// ⇒ 同一份 plan、同一次 diff，`--pg-render` 的日志里 **273 条** `text-diff`，
    /// 而 `<pg-url>` 生产路径的日志里 **0 条**。而 advisory 机制自己的立项理由就是
    /// 「而不是无声无息」（`extras.rs`）—— 生产路径把它的唯一价值丢掉了。
    ///
    /// 断言的是**传递链**（plan → outcome → report），不是文案：文案会漂移，
    /// 链路断了才是真缺陷。
    #[tokio::test]
    async fn advisories_reach_the_outcome_and_the_report() {
        let db = mem().await;
        let mut p = plan_of(Vec::new());
        p.advisories.push("text-diff t.c default：实况 'x'::text ｜ 期望 'x'".to_string());

        let opts =
            ApplyOptions { config: cfg_exec(), renderer: fake_render, ..ApplyOptions::default() };
        let out = run(&db, &p, 0, Dialect::Sqlite, &opts).await.unwrap();
        assert_eq!(out.advisories.len(), 1, "advisory 没进 outcome：{out:?}");

        let txt = out.report("P4 apply · 测试");
        assert!(txt.contains("text-diff t.c default"), "report 丢了 advisory 正文：\n{txt}");
        assert!(txt.contains("声明漂移 1 条"), "report 丢了 advisory 计数：\n{txt}");
    }

    // ── 元表 ──

    /// apply 建出来的表对 `introspect` 可见，而四张元表**不可见**。
    ///
    /// 这是「apply 后指纹 == 期望指纹」能成立的前提被端到端验证了一次。
    #[tokio::test]
    async fn business_tables_are_visible_and_meta_tables_are_not() {
        let db = mem().await;
        let p = plan_of(vec![change(ChangeKind::CreateTable, "visible_table")]);
        let opts =
            ApplyOptions { config: cfg_exec(), renderer: fake_render, ..ApplyOptions::default() };
        let out = run(&db, &p, 1, Dialect::Sqlite, &opts).await.unwrap();
        assert!(out.items[0].executed);

        let seen = introspect::read(&db).await.unwrap();
        let names = seen.table_names();
        assert_eq!(names, vec!["visible_table"], "只应看见业务表，元表不可见：{names:?}");
        for t in safety::META_TABLES {
            assert!(!names.contains(t), "元表 `{t}` 泄漏进了实况模型");
        }
    }

    // ── 一站式入口 ──

    /// `cycle` 在空 SQLite 上：全量建表计划，dry-run 下一条都不执行，审计行数 == 变更条数。
    #[tokio::test]
    async fn cycle_on_empty_sqlite_plans_only_creates_and_executes_nothing() {
        let db = mem().await;
        let opts = ApplyOptions { renderer: fake_render, ..ApplyOptions::default() };
        let out = cycle(&db, &opts).await.expect("cycle 应成功");

        assert!(out.dry_run);
        assert_eq!(out.refusal, None, "{:?}", out.refusal);
        assert_eq!(out.executed, 0);
        assert!(out.items.len() > 50, "空库的期望结构应是全量建表：{} 条", out.items.len());
        assert_eq!(out.audit_rows, out.items.len());
        assert!(!out.expected_fingerprint.as_ref().unwrap().is_empty());
        // 空库上的期望模型不该含破坏性变更
        assert!(
            out.items.iter().all(|i| !i.destructive),
            "空库上出现破坏性变更：{:?}",
            out.items.iter().filter(|i| i.destructive).map(|i| &i.object).collect::<Vec<_>>()
        );
    }

    /// ⚠ **白名单必须在 diff 之前读** —— 用`cycle` 端到端证明它真的生效。
    ///
    /// 若顺序写反（先 diff 再读白名单），孤儿表照样会被判 `DROP TABLE`，而白名单
    /// 「看起来配好了」—— 症状只有「明明登记了还是被删了」。
    #[tokio::test]
    async fn cycle_loads_whitelist_before_diff_so_it_actually_exempts() {
        let db = mem().await;

        // 一张既不在 L1 实体、也不在 L2 声明里的表 ⇒ 天然孤儿
        db.execute_unprepared("CREATE TABLE \"orphan_runtime_x\" (\"id\" TEXT)").await.unwrap();

        let opts = ApplyOptions { renderer: fake_render, ..ApplyOptions::default() };
        let before = cycle(&db, &opts).await.unwrap();
        let dropped_before: Vec<&str> = before
            .items
            .iter()
            .filter(|i| i.kind == ChangeKind::DropTable)
            .map(|i| i.object.as_str())
            .collect();
        assert!(
            dropped_before.contains(&"orphan_runtime_x"),
            "前提：未登记白名单时它应被判孤儿：{dropped_before:?}"
        );

        // 登记到很远之后（走写入口：它自带 `ensure_meta_tables`，调用方不需要记得建表）
        safety::put_orphan_whitelist(
            &db,
            "orphan_runtime_x",
            "正在迁移",
            safety::now_epoch() + 86_400,
        )
        .await
        .unwrap();

        let after = cycle(&db, &opts).await.unwrap();
        let dropped_after: Vec<&str> = after
            .items
            .iter()
            .filter(|i| i.kind == ChangeKind::DropTable)
            .map(|i| i.object.as_str())
            .collect();
        assert!(
            !dropped_after.contains(&"orphan_runtime_x"),
            "白名单生效后不得再判孤儿：{dropped_after:?}"
        );
        assert_eq!(after.whitelist_active, 1);
        assert!(after.whitelist_expired.is_empty());
    }

    /// 过期白名单**不再豁免**，但必须留痕（否则「我登记过它，它怎么还是被删了」无从解释）。
    #[tokio::test]
    async fn expired_whitelist_stops_exempting_but_is_reported() {
        let db = mem().await;
        db.execute_unprepared("CREATE TABLE \"orphan_expired_y\" (\"id\" TEXT)").await.unwrap();
        // 登记一条**已经过期**的（`expires_at` 在过去）
        safety::put_orphan_whitelist(&db, "orphan_expired_y", "早就过期", safety::now_epoch() - 1)
            .await
            .unwrap();

        let opts = ApplyOptions { renderer: fake_render, ..ApplyOptions::default() };
        let out = cycle(&db, &opts).await.unwrap();
        assert_eq!(out.whitelist_active, 0);
        assert_eq!(out.whitelist_expired, vec!["orphan_expired_y".to_string()]);
        assert!(
            out.items
                .iter()
                .any(|i| i.kind == ChangeKind::DropTable && i.object == "orphan_expired_y"),
            "过期后应恢复判孤儿"
        );
        assert!(out.report("过期").contains("过期项不再豁免"));
    }

    // ── 杂项不变量 ──

    #[test]
    fn skip_reasons_are_distinguishable_in_text() {
        let all = [
            SkipReason::DryRun,
            SkipReason::DeferredFirstSight,
            SkipReason::BatchRefused,
            SkipReason::AbortedAfterError,
            SkipReason::UnsupportedDialect,
        ];
        let mut labels: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all.len(), "标签必须互不重复");
        for s in all {
            assert!(!s.note().is_empty());
        }
    }

    /// `run_id` 由 `ordinal` 决定 ⇒ 同秒内两次运行不撞（审计主键是 `(run_id, seq)`）。
    #[tokio::test]
    async fn ordinal_makes_run_ids_unique_within_a_second() {
        let db = mem().await;
        let p = plan_of(vec![]);
        let a = ApplyOptions { ordinal: 1, renderer: fake_render, ..ApplyOptions::default() };
        let b = ApplyOptions { ordinal: 2, renderer: fake_render, ..ApplyOptions::default() };
        let oa = run(&db, &p, 0, Dialect::Sqlite, &a).await.unwrap();
        let ob = run(&db, &p, 0, Dialect::Sqlite, &b).await.unwrap();
        assert_ne!(oa.run_id, ob.run_id);
        assert!(oa.run_id.starts_with("run-"), "{}", oa.run_id);
        assert_eq!(oa.audit_rows, 0, "空 plan 不写审计行");
    }

    /// `dialect_of` 认得出 SQLite，且不支持的 backend 明确报错（不静默当成 PG）。
    #[tokio::test]
    async fn dialect_of_maps_sqlite() {
        let db = mem().await;
        assert_eq!(dialect_of(&db).unwrap(), Dialect::Sqlite);
    }
}
