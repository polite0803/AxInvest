// SPDX-License-Identifier: AGPL-3.0-only

//! **差异 → 变更计划**。P3 只到「算出来并打印」，**不渲染、不执行**。
//!
//! 输入 = 实况模型（[`super::introspect::read`]）与期望模型（[`super::expected::build`]），
//! 输出 = [`Plan`]：可执行变更序列 + 孤儿候选 + 只报告不执行的诊断项。
//!
//! ## 身份判定（每类对象「什么算同一个」）
//!
//! | 对象 | 身份 | 为什么 |
//! |---|---|---|
//! | 表 | 表名 | —— |
//! | 列 | 列名（**改名通道除外**） | `renamed_from` 命中时按旧名匹配，见下 |
//! | 索引 | **索引名**，改名按「定义等价 + 新名在 A 缺席」识别 | 库有 264 索引 + 30 FK 需一次性改名（PLAN §五·二·补二），判成「删+建」会白做一遍 |
//! | FK | **定义**（cols + ref_table + ref_cols），名不同 ⇒ 改名 | 同上；且 PG 侧 FK 名不参与语义 |
//! | CHECK | **约束名** | PG 会重写表达式（`'general'` → `'general'::text`），文本比较不可靠 ⇒ 名字定身份、文本降为诊断 |
//!
//! ## ⚠ 表达式文本一律**不参与结构等价**
//!
//! `default` / `generated` / `where_clause` / CHECK 的 `expr` 在两侧**必然**文本不同：
//! 期望侧是 sea-query 渲染，实况侧是 `pg_get_expr` / `pg_get_constraintdef`，PG 会补
//! `::类型` 转换与冗余括号。实测（`output/tmp-p3-constraint-probe.log`）：
//!
//! ```text
//! 声明：period ~ '^\d{4}-\d{2}$'
//! pg 报回：CHECK ((period ~ '^\d{4}-\d{2}$'::text))
//! ```
//!
//! ⇒ 结构等价只看「**有没有**」（`is_some()` 是否一致），文本差异进 [`Plan::advisories`]。
//! 这条纪律是 P3 出口判据能被满足的前提：否则 1333 个 `text` 列那一类噪声会重演。
//!
//! ## 排序（PLAN §四·三）
//!
//! 固定顺序共 6 段，段内再按对象名升序（确定性输出，便于逐轮比对）：
//!
//! ```text
//! 0 改名      列 / 索引 / FK        ← 必须先于「增删」，否则改名退化成删+建
//! 1 撤依赖    CHECK / FK / 索引 / 表（逆拓扑）
//! 2 建表      （FK 拓扑序）
//! 3 列变更    增 / 改类型 / 可空 / 默认 / 生成列
//! 4 删列      ← 必须在撤掉引用它的索引与约束之后
//! 5 建依赖    UNIQUE / FK / CHECK / 索引
//! ```
//!
//! ⚠ **段 2 是唯一的例外**：建表**不加名称 tiebreaker**，保留产出顺序（= FK 拓扑序）。
//! 按名重排会把「被引用者先建」压成字典序，`z_parent` 排到 `a_child` 之后 ⇒ DDL 失败。
//! 其余段落加名称兜底，是因为它们的产出顺序依赖容器种类（`index_level` /
//! `check_level` 里有 HashMap 查找），只靠稳定排序不足以自证确定性。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::{
    expected, extras,
    extras::Dialect,
    model::{CheckModel, ColumnModel, FkModel, IndexModel, SchemaModel, TableModel},
};

/// 渲染 [`Change`] 所需的**结构化参数**。
///
/// ## 为什么 P4 必须补这一层
///
/// P3 的 `Change` 只有 `object`（`表.列` / 索引名 / 约束名）与人读的 `detail`。
/// 计划只被**打印**，所以够用 —— 这也正是 P3 看不出缺口的原因。
///
/// P4 要把每条变更渲染成 DDL，而 DDL 要的是**定义本身**：`ADD COLUMN` 需要类型 /
/// 可空 / 默认 / 生成表达式；`CREATE INDEX` 需要列集 / 访问方法 / partial 谓词。
/// 从 `object` 字符串反查有两条硬伤：
///
/// 1. **字符串有歧义** —— FK 的 object 是 `表.colA,colB`，自带逗号，与列级的
///    `表.列` 不是同一套切分规则；
/// 2. **渲染器不再纯** —— 反查要求同时持有期望模型与实况模型，`render` 就无法
///    只吃一个 `&[Change]`，单测也就无从下手。
///
/// ⇒ 载荷在**产出变更的那一刻**从模型 clone 出来（那时信息就在手边）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangePayload {
    /// 无附加参数：`object` 已含全部信息（`DROP TABLE`）。
    Bare,
    /// 改名：`ALTER … {from} RENAME TO {to}`。
    ///
    /// `from` 可能为空串（实况侧匿名对象）—— 渲染器须据此改走「按定义重建」而非
    /// `RENAME`，见 `render` 模块文档。
    Rename { table: String, from: String, to: String },
    /// 列级：`ADD COLUMN` / `ALTER COLUMN …` / `DROP COLUMN` 共用。
    /// 建列时取**期望侧**列定义（新定义），删列时取**实况侧**列定义。
    Column { table: String, col: ColumnModel },
    /// 索引：建索引取期望侧，删索引取实况侧。
    Index { table: String, def: IndexModel },
    /// 外键：同上。
    Fk { table: String, def: FkModel },
    /// CHECK 约束：同上。
    Check { table: String, def: CheckModel },
    /// 整表定义（`CREATE TABLE`）。
    Table(TableModel),
}

/// 一条变更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// 对象标识：`表` / `表.列` / `索引名` / `约束名`。
    pub object: String,
    /// 需要人工确认的破坏性操作（丢数据或丢结构）。
    pub destructive: bool,
    /// 真正可能丢**数据行/列值**（`DropTable` / `DropColumn` / `AlterColumnType`）。
    /// 这是 [`Change::destructive`] 的子集，单列出来是为了让「墓碑」只盖在最危险的一类上。
    pub loses_data: bool,
    /// 人读的一行说明（含差异前后值，供逐条归因）。
    pub detail: String,
    /// 渲染所需的结构化参数。P3 不读它；P4 的 `render` 只读它。
    pub payload: ChangePayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    // ── 段 0：改名 ──
    RenameColumn,
    RenameIndex,
    RenameForeignKey,
    // ── 段 1：撤依赖 ──
    DropCheck,
    DropForeignKey,
    DropIndex,
    DropTable,
    // ── 段 2：建表 ──
    CreateTable,
    // ── 段 3：列变更 ──
    AddColumn,
    AlterColumnType,
    SetNotNull,
    DropNotNull,
    SetDefault,
    DropDefault,
    AlterGenerated,
    // ── 段 4：删列 ──
    DropColumn,
    // ── 段 5：建依赖 ──
    AddUnique,
    DropUnique,
    AddForeignKey,
    AddCheck,
    CreateIndex,
}

/// 一条变更对**具名对象**的增删方向（见 [`ChangeKind::object_op`]）。
///
/// 存在的唯一理由是 [`Plan::retain_purely_additive`] 的第二刀：把「成对重建」的两半
/// **一起**收窄，否则只留一半会发出一条必然失败的语句。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjOp {
    /// 会 CREATE 一个具名对象（表 / 列 / 索引 / 约束）。
    Create,
    /// 会 DROP 一个具名对象。
    Drop,
}

impl ChangeKind {
    /// 执行段序号（越小越先）。
    pub fn phase(self) -> u8 {
        use ChangeKind::*;
        match self {
            RenameColumn | RenameIndex | RenameForeignKey => 0,
            DropCheck | DropForeignKey | DropIndex | DropTable => 1,
            CreateTable => 2,
            AddColumn | AlterColumnType | SetNotNull | DropNotNull | SetDefault | DropDefault
            | AlterGenerated => 3,
            DropColumn => 4,
            AddUnique | DropUnique | AddForeignKey | AddCheck | CreateIndex => 5,
        }
    }

    /// 破坏性（丢数据或丢结构）。P4 执行前须落墓碑 + 人工确认。
    pub fn destructive(self) -> bool {
        use ChangeKind::*;
        matches!(
            self,
            DropTable
                | DropColumn
                | DropIndex
                | DropForeignKey
                | DropCheck
                | AlterColumnType
                | DropDefault
                | AlterGenerated
        )
    }

    /// 真丢数据（墓碑必须覆盖的最小集合）。
    pub fn loses_data(self) -> bool {
        matches!(self, ChangeKind::DropTable | ChangeKind::DropColumn | ChangeKind::AlterColumnType)
    }

    /// **纯新增** —— 可以安全地用于**启动期 bootstrap** 的类别白名单。
    ///
    /// ## 为什么在 `plan` 层定义这个判据
    ///
    /// 启动路径 (`create_pool`) 要在**用户的生产库**上把 schema 补齐到实体声明的形态。
    /// 这条路径没有人工审查，所以它只允许做「加东西」：建表 / 加列 / 建索引 / 加约束。
    /// 任何会让已有对象消失或收紧的类别都必须在启动路径上被**静默跳过**，留给离线
    /// 探针（`p4_apply_probe`）在有人看着的时候执行。
    ///
    /// ## 为什么是白名单而不是 `!destructive()`
    ///
    /// `!destructive()` 会放进三类别名（`RenameColumn` / `RenameIndex` /
    /// `RenameForeignKey`）和 `SetNotNull`，它们**都不丢数据**，但都不适合自动执行：
    /// * 改名：`renamed_from` 声明若写错，启动时会把**真列**改掉。全新库不存在改名
    ///   场景（没有旧列），存量库的改名早已由历史迁移做过。
    /// * `SetNotNull`：表里存在 NULL 行时 DDL 必失败；启动路径是 fail-stop，
    ///   一条失败会**中止整批**，把「补 175 张表」变成「一张都没补」。
    ///
    /// ## 新增 `ChangeKind` 时的强制分类
    ///
    /// 白名单的代价是「新类别默认不进 bootstrap」—— 这是**安全方向**的默认。但必须
    /// 防止「加了新类别却忘了分类」。配套测试
    /// `every_change_kind_is_classified_for_bootstrap` 会遍历 [`ChangeKind::ALL`]
    /// 逐项比对判定表，漏一个就红。
    ///
    /// 另外这条不变量由测试断言：**additive 集合与 `destructive()` 集合不相交**。
    pub fn is_purely_additive(self) -> bool {
        use ChangeKind::*;
        matches!(
            self,
            CreateTable
                | AddColumn
                | CreateIndex
                | AddForeignKey
                | AddCheck
                | AddUnique
                | SetDefault
        )
    }

    /// 本类别对**具名对象**的增删方向；`None` = 不增删具名对象（只改属性）。
    ///
    /// ## 为什么必须是穷举 `match` 而不是 `matches!` + 兜底
    ///
    /// 它是 [`Plan::retain_purely_additive`] 第二刀（成对重建自洽化）的输入。用
    /// `_ => None` 兜底的话，将来新增一个「会 CREATE 某个具名对象」的 `ChangeKind` 时
    /// 没人会记得回来登记 —— 症状是那条变更**逃过自洽化**，于是重新出现
    /// 「`CREATE X` 时 X 已存在 ⇒ fail-stop ⇒ 启动中止」。
    /// 穷举 `match` 让编译器在新增变体时把作者拦在这里（判据 I 组：加变体靠编译器穷举）。
    pub fn object_op(self) -> Option<ObjOp> {
        use ChangeKind::*;
        match self {
            // ── 增 ──
            CreateTable | AddColumn | CreateIndex | AddForeignKey | AddCheck | AddUnique => {
                Some(ObjOp::Create)
            },
            // ── 删 ──
            DropTable | DropColumn | DropIndex | DropForeignKey | DropCheck | DropUnique => {
                Some(ObjOp::Drop)
            },
            // ── 只改属性，不增删具名对象 ──
            //
            // 逐条理由（为什么它们**不**参与「成对重建」判定）：
            // * `RenameX` —— 自身已是「改名」通道，不再产同名的 Create/Drop（见 §四·二）；
            // * `AlterColumnType` / `AlterGenerated` —— 原地改写，没有「名字已被占用」问题；
            // * `SetNotNull` / `DropNotNull` —— `ALTER COLUMN … SET/DROP NOT NULL` 是覆写语义；
            // * `SetDefault` / `DropDefault` —— `SET DEFAULT` 同样是覆写，不会撞名。
            //
            // ⚠ 判据是「这条语句会不会因为**同名的东西已存在**而失败」，不是「它危险不危险」。
            // 危险与否由 `destructive()` / `loses_data()` 那套独立判定，两件事别混。
            RenameColumn | RenameIndex | RenameForeignKey | AlterColumnType | AlterGenerated
            | SetNotNull | DropNotNull | SetDefault | DropDefault => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        use ChangeKind::*;
        match self {
            RenameColumn => "RENAME COLUMN",
            RenameIndex => "RENAME INDEX",
            RenameForeignKey => "RENAME FK",
            DropCheck => "DROP CHECK",
            DropForeignKey => "DROP FK",
            DropIndex => "DROP INDEX",
            DropTable => "DROP TABLE",
            CreateTable => "CREATE TABLE",
            AddColumn => "ADD COLUMN",
            AlterColumnType => "ALTER TYPE",
            SetNotNull => "SET NOT NULL",
            DropNotNull => "DROP NOT NULL",
            SetDefault => "SET DEFAULT",
            DropDefault => "DROP DEFAULT",
            AlterGenerated => "ALTER GENERATED",
            DropColumn => "DROP COLUMN",
            AddUnique => "ADD UNIQUE",
            DropUnique => "DROP UNIQUE",
            AddForeignKey => "ADD FK",
            AddCheck => "ADD CHECK",
            CreateIndex => "CREATE INDEX",
        }
    }

    /// **全部类别** —— 「21 类」这个数字的唯一来源。
    ///
    /// ## 为什么必须有它
    ///
    /// 「21 类全覆盖」这句话本仓说过很多次，而支撑它的判据原本是渲染器测试里的一句
    /// `assert_eq!(cases.len(), 21)`。那种断言只查**数量**：加一类的同时删一类，数量不变，
    /// 少测的那一类**无人发现**。集中到一处之后，测试可以比对**集合**而不是数量。
    ///
    /// ⚠ 这张表**不会被编译器强制同步**（`match` 才有穷举检查）。兜底是两件事一起：
    /// ① 渲染器的穷举测试断言「用例集合 == `ALL`」（漏加成员会让它红）；
    /// ② `ALL` 的长度是**数组类型的一部分**（`[Self; 21]`）—— 改长度必须改类型，
    /// 顺手就会看到这个数字。
    pub const ALL: [Self; 21] = [
        // 段 0
        Self::RenameColumn,
        Self::RenameIndex,
        Self::RenameForeignKey,
        // 段 1
        Self::DropCheck,
        Self::DropForeignKey,
        Self::DropIndex,
        Self::DropTable,
        // 段 2
        Self::CreateTable,
        // 段 3
        Self::AddColumn,
        Self::AlterColumnType,
        Self::SetNotNull,
        Self::DropNotNull,
        Self::SetDefault,
        Self::DropDefault,
        Self::AlterGenerated,
        // 段 4
        Self::DropColumn,
        // 段 5
        Self::AddUnique,
        Self::DropUnique,
        Self::AddForeignKey,
        Self::AddCheck,
        Self::CreateIndex,
    ];
}

/// 孤儿候选：实况有、期望无 ⇒ 会被判 DROP 的对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub kind: ChangeKind,
    pub object: String,
    pub detail: String,
}

/// 一次 diff 的完整结果。
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// 按 [`ChangeKind::phase`] 排序后的可执行变更。
    pub changes: Vec<Change>,
    /// 只报告不执行：表达式文本口径差异、结构性存疑项。
    pub advisories: Vec<String>,
    /// 非破坏性变更数（新建/改名/加约束这类可重建动作）。
    pub n_safe: usize,
    /// 破坏性变更数（丢数据或丢结构）。
    pub n_destructive: usize,
    /// 已 finalize 的两侧指纹（诊断用）。
    pub actual_fingerprint: String,
    pub expected_fingerprint: String,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// 只保留「纯新增」变更（判据见 [`ChangeKind::is_purely_additive`]）。
    ///
    /// 启动期 bootstrap 用它把 plan 收窄成可安全自动执行的子集。三处必须同步：
    ///
    /// * **重算 `n_safe` / `n_destructive`** —— 这两个计数是闸 2（破坏性配额）与
    ///   报告数字的输入。只 `retain` 不重算，被过滤掉的条目仍计入 `n_destructive`，
    ///   闸 2 就会报出一个与 plan 实际内容不符的配额占用。
    /// * **`advisories` 原样保留** —— 那是「只报告不执行」的诊断信息，与可执行集
    ///   无关；启动期日志恰恰需要它来解释「这个库为什么不等于声明」。
    /// * **成对重建必须**成对**收窄**（见下）。
    ///
    /// ## 为什么必须做第二刀：成对重建的自洽化
    ///
    /// 有些变更在 plan 里是**成对**出现的：「期望侧新建」+「实况侧删掉」，两者
    /// **object 相同** —— `index_level` 的 `Some(ai) if !same_index(...)` 分支就是
    /// 这么产的（`DropIndex(key)` + `CreateIndex(key)`，同一个 key）。
    ///
    /// 只留前一半，后果**不是**「少做一件事」，而是**发出一条必然失败的语句**：
    /// `CREATE INDEX x` 的时候库里的 `x` 还在。第一现场（2026-09-16，
    /// `p5_engine_takeover --legacy-sqlite`）：
    ///
    /// ```text
    /// 执行中止: `idx_index_jobs_status` 执行 `CREATE INDEX "idx_index_jobs_status" …` 失败：
    ///           Execution Error: … index idx_index_jobs_status already exists
    /// ```
    ///
    /// 那一次的直接根因（`ASC` 未归一，见 `same_index`）已单独修掉，但**这一刀必须独立
    /// 存在**：下次真有索引定义变化（改列顺序、改谓词、PG 的 `real`→`double precision`
    /// 类型自愈顺带改索引）时，启动路径的选择只有两个 —— **跳过**（= 保持原样，库与声明
    /// 差异延续，下一轮 diff 还会报）与**中止**（= App 起不来）。前者可解释、可离线修；
    /// 后者是一次无人值守的停服。选前者。
    ///
    /// ⚠ 代价如实登记：被这一刀切掉的条目**不会**自动消失，该库在它们上永不收敛，
    /// 必须走离线全量 apply（探针）或重建流程。所以 `apply` 侧要把「被切掉的条数」
    /// 报出来（`ApplyOutcome::unsupported()` 之外，见 `report()` 的「未执行」一行）。
    pub fn retain_purely_additive(&mut self) {
        // ── 第二刀的数据必须**在第一刀之前**收集 ──
        //
        // ⚠ 这里踩过一次（2026-09-16，本函数的第一版）：先 `retain(is_purely_additive)`
        // 再收集「被删对象名」，而第一刀恰好把 `Drop*` **全部**滤掉了（它们没有一个属纯新增）
        // ⇒ 收集到的永远是空集 ⇒ 第二刀是一条死代码。
        // 症状不是报错，而是**静默无效**：计划里只剩孤零零一条 `CreateIndex`，
        // 看上去「收窄成功」，实际一执行就撞「已存在」。判据见测试
        // `additive_only_drops_both_halves_of_a_paired_index_rebuild`（它就是这么抓到的）。
        let dropped: HashSet<String> = self
            .changes
            .iter()
            .filter(|c| c.kind.object_op() == Some(ObjOp::Drop))
            .map(|c| c.object.clone())
            .collect();

        // ── 第一刀：按类别 ──
        self.changes.retain(|c| c.kind.is_purely_additive());

        // ── 第二刀：成对重建的自洽化 ──
        self.changes.retain(|c| {
            !(c.kind.object_op() == Some(ObjOp::Create) && dropped.contains(&c.object))
        });

        self.n_destructive = self.changes.iter().filter(|c| c.destructive).count();
        self.n_safe = self.changes.len() - self.n_destructive;
    }

    /// 孤儿候选（会被 DROP 的全部对象）。
    pub fn orphans(&self) -> Vec<Orphan> {
        self.changes
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    ChangeKind::DropTable
                        | ChangeKind::DropColumn
                        | ChangeKind::DropIndex
                        | ChangeKind::DropForeignKey
                        | ChangeKind::DropCheck
                )
            })
            .map(|c| Orphan { kind: c.kind, object: c.object.clone(), detail: c.detail.clone() })
            .collect()
    }

    /// 逐段分组计数：`(phase, 条数)`。
    fn by_phase(&self) -> BTreeMap<u8, usize> {
        let mut m = BTreeMap::new();
        for c in &self.changes {
            *m.entry(c.kind.phase()).or_insert(0) += 1;
        }
        m
    }

    /// 可打印报告（P3 的交付物本体：人能逐条读、能逐条归因）。
    pub fn render(&self, title: &str) -> String {
        let mut s = String::new();
        s.push_str(&format!("== {title} ==\n"));
        s.push_str(&format!("实况指纹 : {}\n", short(&self.actual_fingerprint)));
        s.push_str(&format!("期望指纹 : {}\n", short(&self.expected_fingerprint)));
        s.push_str(&format!(
            "变更合计 : {}（破坏性 {} / 非破坏性 {}）\n",
            self.changes.len(),
            self.n_destructive,
            self.n_safe
        ));
        for (phase, n) in self.by_phase() {
            s.push_str(&format!("  段{phase} : {n}\n"));
        }

        s.push_str("\n-- 孤儿候选（将被 DROP） --\n");
        let orphans = self.orphans();
        if orphans.is_empty() {
            s.push_str("（空集）\n");
        } else {
            for o in &orphans {
                s.push_str(&format!("  {:<12} {}   {}\n", o.kind.as_str(), o.object, o.detail));
            }
        }

        s.push_str("\n-- 变更序列（按执行段） --\n");
        if self.changes.is_empty() {
            s.push_str("（空集：结构与声明一致）\n");
        } else {
            for c in &self.changes {
                let mark = if c.loses_data {
                    "!!"
                } else if c.destructive {
                    " ~"
                } else {
                    "  "
                };
                s.push_str(&format!(
                    "{mark} 段{} {:<14} {:<48} {}\n",
                    c.kind.phase(),
                    c.kind.as_str(),
                    c.object,
                    c.detail
                ));
            }
        }

        s.push_str("\n-- 诊断（只报告，不产生变更） --\n");
        if self.advisories.is_empty() {
            s.push_str("（无）\n");
        } else {
            for a in &self.advisories {
                s.push_str(&format!("  {a}\n"));
            }
        }
        s
    }
}

fn short(fp: &str) -> String {
    fp.chars().take(16).collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// 入口
// ═══════════════════════════════════════════════════════════════════════════

/// 求差的**运行期**选项。
///
/// 做成结构体而不是「往 `diff_with` 多塞一个 map 参数」，是为了让后续新增选项
/// 不再改签名。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanOptions {
    /// 动态孤儿豁免：表名 → 理由。
    ///
    /// ⚠ **理由必须自带来源与有效期**（形如
    /// `whitelist: 数据迁移中，至 2026-10-01`）。理由会原样进 `Plan::advisories`，
    /// 它是「这张表为什么没被 DROP」的唯一线索 —— 只写「白名单」等于没写。
    pub orphan_whitelist: BTreeMap<String, String>,
}

/// 求差。**无 IO、无副作用**：两个模型都是内存值。
///
/// `dialect` 只用于 `PATTERN_TSV` 的方言过滤。等价于
/// `diff_with(expected, actual, dialect, &PlanOptions::default())`。
pub fn diff(expected: &SchemaModel, actual: &SchemaModel, dialect: Dialect) -> Plan {
    diff_with(expected, actual, dialect, &PlanOptions::default())
}

/// 带运行期选项的求差入口。
///
/// ## 为什么必须有它（而不是把白名单也写成常量）
///
/// 孤儿豁免有两类来源：**编译期规则**（`extras::ORPHAN_EXEMPT`，描述「哪些
/// 名字形态永远不判孤儿」）与**运行期人工登记**（`_ax_schema_orphan_whitelist` 里
/// 「这张表暂时别删，到某日为止」）。后者带过期时间与人工理由，写不进常量。
///
/// ⇒ 选项在**入口**合并进 `Differ`。`Plan` 仍是纯值：同一份模型 + 同一份选项 ⇒
/// 同一份 plan（指纹与变更集不受任何隐藏状态影响）。
pub fn diff_with(
    expected: &SchemaModel,
    actual: &SchemaModel,
    dialect: Dialect,
    opts: &PlanOptions,
) -> Plan {
    // 静态规则与人工白名单合并到同一张表（`Differ.orphan_exempt`）。理由字符串里
    // 必须带来源与有效期 —— 否则日志里「这张表为什么没被删」无法追溯。
    // 合并在后 ⇒ 人工白名单覆盖同表的静态理由（人工更具体）。
    let mut orphan_exempt = orphan_exempt_tables_of(actual, dialect);
    // 「非主库实体」在**孤儿判定这一侧**的处置与静态豁免相同（都不判孤儿），但理由
    // 完全不同：前者是「这张表的家不在这里」（同时也已被排除出期望集），后者是
    // 「名字形态特殊」。合进同一张查找表、**理由分开写** —— 排障时唯一能看的就是理由串。
    orphan_exempt.extend(non_main_db_tables_of(actual, dialect));
    // L2 声明的 FTS5 虚表是**第三个**「不判孤儿」来源，成因与前两个都不同：
    // 期望侧**只进指纹不建表**（`expected::build` ④ 段的 `claim_extra`），实况侧却被
    // `introspect` 建成表 ⇒ 不加这一行就被判孤儿。详见 `l2_virtual_tables_of`。
    orphan_exempt.extend(l2_virtual_tables_of(actual, dialect));
    orphan_exempt.extend(opts.orphan_whitelist.iter().map(|(k, v)| (k.clone(), v.clone())));

    let mut d = Differ {
        expected,
        actual,
        plan: Plan {
            actual_fingerprint: super::fingerprint::fingerprint(actual),
            expected_fingerprint: super::fingerprint::fingerprint(expected),
            ..Plan::default()
        },
        pattern_tables: pattern_tables_of(actual, dialect),
        orphan_exempt,
    };

    d.run();
    // ⚠ `sort_by` 是**稳定**排序 ⇒ 相邻并列项保持产出顺序。`CreateTable` 刻意**不**加
    // 任何 tiebreaker：它的产出顺序是 FK 拓扑序（被引用者先建），按对象名重排会把
    // `z_parent` / `a_child` 排成「先建子表」⇒ DDL 直接失败。
    // 其余类别产出顺序本身依赖容器种类（有 HashMap 查找），故一律用对象名兜底确定性。
    d.plan.changes.sort_by(|a, b| {
        a.kind.phase().cmp(&b.kind.phase()).then_with(|| a.kind.cmp(&b.kind)).then_with(|| {
            if a.kind == ChangeKind::CreateTable {
                std::cmp::Ordering::Equal
            } else {
                a.object.cmp(&b.object)
            }
        })
    });
    d.plan
}

/// 实况中命中 `PATTERN_TSV` 的表名集合。
///
/// 这些表**不在期望模型里**（运行时按向量集合创建，见 `expected.rs` 模块文档），
/// 故必须在这里排除掉孤儿判定 —— 否则每建一个向量集合，引擎就 DROP 一次它的表。
fn pattern_tables_of(actual: &SchemaModel, dialect: Dialect) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for decl in expected::pattern_tsv_decls() {
        for t in &actual.tables {
            if expected::pattern_tsv_applies(decl, &t.name, dialect) {
                set.insert(t.name.clone());
            }
        }
    }
    set
}

/// 实况中命中 `ORPHAN_EXEMPT` 的表名集合 —— 这些表**不判孤儿**。
///
/// 覆盖向量集合的**两类**表：`vec_{collection}` 基表与 `{collection}_meta` 元数据表
/// （`vector_store.rs:200` / `:235`）。注意 `_meta` 一类同时命中 [`pattern_tables_of`]；
/// 两集合的**差集**就是「只靠豁免保命」的表（当前 = 3 张基表）。
fn orphan_exempt_tables_of(actual: &SchemaModel, dialect: Dialect) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for decl in expected::orphan_exempt_decls() {
        for t in &actual.tables {
            if expected::orphan_exempt_applies(decl, &t.name, dialect) {
                // 理由带来源前缀：本函数只产**编译期规则**那一半，另一半（人工白名单）
                // 由 `diff_with` 合并。两者最终在同一张表里，日志上必须能一眼分开。
                map.insert(t.name.clone(), format!("L2 规则：{}", decl.reason));
            }
        }
    }
    map
}

/// 实况中**命中「非主库实体」声明**的表名集合 —— 这些表**不判孤儿**。
///
/// ## 为什么它必须与 `orphan_exempt_tables_of` 分开列（虽然最后并进同一张表）
///
/// 两者在「不判孤儿」这一点上等价，但另一侧不同：非主库实体**同时被排除出期望集**
/// （`expected::build`），静态豁免的表仍在期望集里（例如向量基表由 L2 的
/// `PATTERN_TSV` 覆盖）。把两者合成一个常量会让「这张表到底是谁的」无法从声明上读出来。
///
/// ## 为什么命中的表必须**不判孤儿**（而不是交给孤儿判定去清）
///
/// 主库里那 8 张侧车库空壳（0 行，run3 误建）确实该清，但**不该由引擎每轮扫描自己决定**：
/// 引擎此时已经不认识这些名字（它们被排除出期望集），它能看到的只有「实况有、声明没有」
/// —— 这与「某个子系统的私有表」在数据上**完全同形**。要么显式声明所有权并放过，要么
/// 就是引擎按名字删除别人的对象。判据 **#381**：声称范围必须与落实范围逐对象对账。
fn non_main_db_tables_of(actual: &SchemaModel, dialect: Dialect) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for t in &actual.tables {
        if let Some(reason) = expected::non_main_db_reason(&t.name, dialect) {
            map.insert(t.name.clone(), format!("非主库实体：{reason}"));
        }
    }
    map
}

/// 实况中**命中 L2 FTS5 虚表声明**的表名集合 —— 这些表**不判孤儿**。
///
/// 与 [`non_main_db_tables_of`] 同构的第三个来源（静态规则 / 非主库实体 / **L2 虚表**），
/// 三者只在一点上相同：都回答「这张实况表不归孤儿判定管」；理由串必须能分开，因为
/// 排障时唯一能看的就是理由。
///
/// ## 它修的缺陷（2026-09-17 实测）
///
/// 期望侧与实况侧对「虚表」的处理**不对称**：
///
/// | 侧 | 行为 | 结果 |
/// |---|---|---|
/// | 期望（`expected::build` ④） | `claim_extra("l2.fts5:…")`，**不建 `TableDecl`** | 不在 `expected.tables` |
/// | 实况（`introspect/sqlite.rs`） | `table_or_insert(&raw.name)` **建表** | 在 `actual.tables` |
///
/// ⇒ 表级判定（[`Differ::table_level`] 的 ②）把它们判成孤儿。生产库副本实测：
/// `messages_fts` / `trajectories_fts` / `trajectory_skills_fts` /
/// `trajectory_messages_fts` 四张全部出现在「仅实况（将被 DROP）」里。
///
/// ## 为什么**不**往 `ORPHAN_EXEMPT` 里再加一条规则
///
/// 那会要求在本文件**手抄一份**虚表名单（`OrphanExemptScope` 只有 `Prefix` / `Exact`
/// 两种形态，没有「按后缀」，而虚表名是 `*_fts`），而真相源是 `extras::VIRTUAL_TABLES`
/// —— 两份名单会在下次增删虚表时各自腐烂（判据 K 组：手抄常数必须配防回流，更好的
/// 做法是根本不抄）。这里逐表回调 [`expected::l2_virtual_table_reason`] **现查**，
/// 声明改了豁免自动跟随；同理也**不**能用 `_fts` 后缀通配 —— 那会误豁免
/// `trajectory_memories_fts`（L2 刻意未收录、库里仍在的真残留，必须继续清理）。
fn l2_virtual_tables_of(actual: &SchemaModel, dialect: Dialect) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for t in &actual.tables {
        if let Some(reason) = expected::l2_virtual_table_reason(&t.name, dialect) {
            map.insert(t.name.clone(), format!("L2 虚表：{reason}"));
        }
    }
    map
}

struct Differ<'a> {
    expected: &'a SchemaModel,
    actual: &'a SchemaModel,
    plan: Plan,
    pattern_tables: BTreeSet<String>,
    /// 不参与孤儿判定的实况表名 → 理由（**不参与孤儿判定**）。
    ///
    /// 两个来源合并在同一张表里：`ORPHAN_EXEMPT`（编译期规则）与
    /// `PlanOptions::orphan_whitelist`（运行期人工登记）。值用 `String` 而非
    /// `&'static str` 就是为了容纳后者 —— 人工理由自带有效期，是运行期数据。
    ///
    /// 与 `pattern_tables` **不是**同一集合：后者还驱动 [`Differ::pattern_level`] 的
    /// tsv 判据，而本集合只用于「要不要 `DROP TABLE`」。分开持有是为了让两个范围各自
    /// 可对账 —— 过去向量族的豁免完全寄生在 `PATTERN_TSV` 的匹配条件里，基表因此
    /// 从未被豁免（判据 **#381**）。
    orphan_exempt: BTreeMap<String, String>,
}

impl Differ<'_> {
    fn run(&mut self) {
        self.table_level();
    }

    fn push(&mut self, kind: ChangeKind, object: String, detail: String, payload: ChangePayload) {
        let destructive = kind.destructive();
        let loses_data = kind.loses_data();
        if destructive {
            self.plan.n_destructive += 1;
        } else {
            self.plan.n_safe += 1;
        }
        self.plan.changes.push(Change { kind, object, destructive, loses_data, detail, payload });
    }

    fn advise(&mut self, msg: String) {
        self.plan.advisories.push(msg);
    }

    // ─────────────────────────── 表 ───────────────────────────

    fn table_level(&mut self) {
        // 先把两个模型的引用**复制**出来（`&'a` 是 Copy）：
        // 否则下面 `self.column_level(..)` 的 `&mut self` 会与外借冲突。
        let exp = self.expected;
        let act = self.actual;

        let expected_names: BTreeSet<&str> = exp.tables.iter().map(|t| t.name.as_str()).collect();
        let actual_names: BTreeSet<&str> = act.tables.iter().map(|t| t.name.as_str()).collect();

        // ① 模式规则先行：`vec_*_meta` 这类运行时表的 content_tsv + GIN
        //    只能靠模式保证（期望模型里没有它们的条目）。
        self.pattern_level();

        // ② 孤儿表：实况有、期望无。命中 PATTERN_TSV 的除外（运行时表）。
        for name in &actual_names {
            if expected_names.contains(name) {
                continue;
            }
            if self.pattern_tables.contains(*name) {
                self.advise(format!(
                    "pattern: 表 `{name}` 由 PATTERN_TSV 动态覆盖（运行时表），不参与孤儿判定"
                ));
                continue;
            }
            // ②' 孤儿豁免：向量族的**基表**不在 `PATTERN_TSV` 范围内，过去落到这里被判
            //     DROP（生产库 3 张基表共 49135 行）。`reason` 先取出来，避免
            //     `&self.orphan_exempt` 与 `&mut self` 的借用冲突。
            //
            //     值现在是 `String`（编译期规则 + 运行期人工白名单共用一张表），
            //     所以理由字符串自带来源前缀（`L2 规则：…` / `whitelist: …`）。
            let exempt_reason: Option<String> = self.orphan_exempt.get(*name).cloned();
            if let Some(reason) = exempt_reason {
                self.advise(format!("exempt: 表 `{name}` 不判孤儿（{reason}）"));
                continue;
            }
            self.push(
                ChangeKind::DropTable,
                (*name).to_string(),
                format!(
                    "实况有表、L1+L2 均无声明；列 {} 个",
                    act.table(name).map_or(0, |t| t.columns.len())
                ),
                ChangePayload::Bare,
            );
        }

        // ③ 新建表：期望有、实况无。按 FK 拓扑序（被引用的先建）。
        let to_create: BTreeSet<String> =
            expected_names.difference(&actual_names).map(|s| (*s).to_string()).collect();
        for name in topo_order(exp, &to_create) {
            let t = exp.table(&name).expect("刚取自期望模型");
            self.push(
                ChangeKind::CreateTable,
                name.clone(),
                format!(
                    "列 {} / 索引 {} / FK {} / CHECK {}",
                    t.columns.len(),
                    t.indexes.len(),
                    t.fks.len(),
                    t.checks.len()
                ),
                ChangePayload::Table(t.clone()),
            );
        }

        // ④ 共有表：逐对象对账
        for name in expected_names.intersection(&actual_names) {
            let b = exp.table(name).expect("交集");
            let a = act.table(name).expect("交集");
            self.column_level(name, a, b);
            self.index_level(a, b);
            self.fk_level(name, a, b);
            self.check_level(name, a, b);
        }
    }

    /// 模式规则：对每张命中 `PATTERN_TSV` 的**实况**表，保证声明的那一个生成列与
    /// 一条 GIN 索引存在。
    ///
    /// 为什么必须有这一段：这些表随向量集合创建而增多，期望模型**枚举不出它们**
    /// （见 `expected.rs` 模块文档）。若不作判据，每新建一个集合其 `content_tsv` 与
    /// GIN 都会缺失，混合检索的 BM25 分支静默退化为全表扫描。
    ///
    /// ⚠ 本段**只判声明的那两个对象**，不对该表其余列下任何判据 —— 运行时表的列集
    /// 不由 L1/L2 声明，拿声明去对账就会想改它。这是「模式表豁免」的精确边界。
    fn pattern_level(&mut self) {
        let dialect = self.expected.dialect;
        let names: Vec<String> = self.pattern_tables.iter().cloned().collect();
        for decl in expected::pattern_tsv_decls() {
            for name in &names {
                if !expected::pattern_tsv_applies(decl, name, dialect) {
                    continue;
                }
                let Some(a) = self.actual.table(name) else { continue };

                match a.column(decl.column) {
                    None => self.push(
                        ChangeKind::AddColumn,
                        format!("{name}.{}", decl.column),
                        format!(
                            "pattern 声明的生成列缺失（源列 {}）",
                            decl.source_columns.join("+")
                        ),
                        ChangePayload::Column {
                            table: name.clone(),
                            col: extras::pattern_tsv_column_model(decl),
                        },
                    ),
                    Some(c) if c.generated.is_none() => self.push(
                        ChangeKind::AlterGenerated,
                        format!("{name}.{}", decl.column),
                        "pattern 声明的列存在但不是生成列".to_string(),
                        ChangePayload::Column {
                            table: name.clone(),
                            col: extras::pattern_tsv_column_model(decl),
                        },
                    ),
                    Some(_) => {},
                }

                let idx_name = (decl.index_name)(name);
                match a.index(&idx_name) {
                    None => self.push(
                        ChangeKind::CreateIndex,
                        idx_name,
                        format!("USING gin ({})", decl.column),
                        ChangePayload::Index {
                            table: name.clone(),
                            def: extras::pattern_tsv_index_model(decl, name),
                        },
                    ),
                    Some(i) if i.method.as_deref() != Some("gin") => {
                        let old = i.clone();
                        self.push(
                            ChangeKind::DropIndex,
                            idx_name.clone(),
                            format!("访问方法 {} → gin", i.method.as_deref().unwrap_or("btree")),
                            ChangePayload::Index { table: name.clone(), def: old },
                        );
                        self.push(
                            ChangeKind::CreateIndex,
                            idx_name,
                            format!("USING gin ({})", decl.column),
                            ChangePayload::Index {
                                table: name.clone(),
                                def: extras::pattern_tsv_index_model(decl, name),
                            },
                        );
                    },
                    Some(_) => {},
                }
            }
        }
    }

    // ─────────────────────────── 列 ───────────────────────────

    fn column_level(&mut self, table: &str, a: &TableModel, b: &TableModel) {
        // 改名通道先行：`renamed_from` 命中「旧名在实况、新名不在实况」⇒ RENAME，
        // 而不是「删旧列 + 建新列」（后者丢数据，PLAN §四·二 第 1 条）。
        let mut renamed_old: HashSet<String> = HashSet::new();
        let mut renamed_new: HashSet<String> = HashSet::new();
        for bc in &b.columns {
            let Some(old) = &bc.renamed_from else { continue };
            if a.column(old).is_some() && a.column(&bc.name).is_none() {
                self.push(
                    ChangeKind::RenameColumn,
                    format!("{table}.{}", bc.name),
                    format!("{old} → {}（实体声明 renamed_from）", bc.name),
                    ChangePayload::Rename {
                        table: table.to_string(),
                        from: old.clone(),
                        to: bc.name.clone(),
                    },
                );
                renamed_old.insert(old.clone());
                renamed_new.insert(bc.name.clone());
            }
        }

        for bc in &b.columns {
            let Some(ac) = a.column(&bc.name) else {
                if renamed_new.contains(&bc.name) {
                    // 这一列是本轮 RENAME 的产物：`RENAME COLUMN` 已代表它的存在，
                    // 再补 `ADD COLUMN` 就是「改名 + 建同名新列」的自相矛盾
                    // （P3 实测缺陷）。类型/可空差异在改名落地后的下一轮按同名对账
                    // —— 引擎每次启动都跑，收敛不需要单轮完成。
                    continue;
                }
                self.push(
                    ChangeKind::AddColumn,
                    format!("{table}.{}", bc.name),
                    format!(
                        "{} null={} default={}",
                        bc.sql_type,
                        bc.nullable,
                        bc.default.as_deref().unwrap_or("-")
                    ),
                    ChangePayload::Column { table: table.to_string(), col: bc.clone() },
                );
                continue;
            };
            self.column_pair(table, ac, bc);
        }

        // 孤儿列
        for ac in &a.columns {
            if b.column(&ac.name).is_some() || renamed_old.contains(&ac.name) {
                continue;
            }
            if b.primary_key.contains(&ac.name) {
                // 主键列在期望侧必然也在列集里；走到这里说明两侧主键集合不一致，
                // 属于必须人看一眼的结构分歧，不静默 DROP。
                self.advise(format!(
                    "!! {table}.{} 是实况主键列但期望列集里没有 ⇒ 需人工确认（不自动 DROP）",
                    ac.name
                ));
                continue;
            }
            self.push(
                ChangeKind::DropColumn,
                format!("{table}.{}", ac.name),
                format!("实况有列、L1+L2 均无声明；类型 {}", ac.sql_type),
                ChangePayload::Column { table: table.to_string(), col: ac.clone() },
            );
        }
    }

    /// 同名列对账（类型 / 可空 / 唯一 / 生成列 / 默认）。
    fn column_pair(&mut self, table: &str, ac: &ColumnModel, bc: &ColumnModel) {
        let obj = format!("{table}.{}", bc.name);

        if ac.sql_type != bc.sql_type {
            self.push(
                ChangeKind::AlterColumnType,
                obj.clone(),
                format!("{} → {}", ac.sql_type, bc.sql_type),
                ChangePayload::Column { table: table.to_string(), col: bc.clone() },
            );
        }
        if ac.nullable != bc.nullable {
            self.push(
                if bc.nullable {
                    ChangeKind::DropNotNull
                } else {
                    ChangeKind::SetNotNull
                },
                obj.clone(),
                format!("nullable {} → {}", ac.nullable, bc.nullable),
                ChangePayload::Column { table: table.to_string(), col: bc.clone() },
            );
        }
        if ac.unique != bc.unique {
            self.push(
                if bc.unique {
                    ChangeKind::AddUnique
                } else {
                    ChangeKind::DropUnique
                },
                obj.clone(),
                format!("unique {} → {}", ac.unique, bc.unique),
                ChangePayload::Column { table: table.to_string(), col: bc.clone() },
            );
        }
        // 生成列：只比「是不是」，不比表达式文本（PG 会重写，见模块文档）
        if ac.generated.is_some() != bc.generated.is_some() {
            self.push(
                ChangeKind::AlterGenerated,
                obj.clone(),
                format!("generated {} → {}", ac.generated.is_some(), bc.generated.is_some()),
                ChangePayload::Column { table: table.to_string(), col: bc.clone() },
            );
        } else if ac.generated != bc.generated {
            self.advise(format!(
                "text-diff {obj} generated：实况 {} ｜ 期望 {}",
                ac.generated.as_deref().unwrap_or("-"),
                bc.generated.as_deref().unwrap_or("-")
            ));
        }

        // 默认值：同上，只比「有没有」
        //
        // ⚠ **自增列必须先排除**（2026-09-16 真库实测缺陷）。两侧对「自增」的表达方式
        // 不同：实况是「一个序列 + `DEFAULT nextval('…_seq')`」⇒ `default` 非空；声明是
        // `#[sea_orm(auto_increment = true)]` ⇒ sea-orm 写进 `auto_increment()` 而**不写
        // `default`** ⇒ `default` 为空。照「有没有」比就必然产出 `DROP DEFAULT`，而执行它
        // 等于**把自增列降级成普通列**：此后任何不给 id 的 INSERT 都因 NOT NULL 失败。
        // 实测生产库 10 个 `nextval` 列里，**6 个实体已正确声明却仍被计划成 DROP DEFAULT**。
        //
        // 处置口径：任一侧自增 ⇒ **不比对 default**（序列绑定不是「默认值」，是另一种东西），
        // 两侧自增属性不一致时**出 advisory 而非 DDL**（改写自增属性是单独一批工作，
        // 详见 `ColumnModel::auto_increment`）。宁可显式报出、也不静默当成一致。
        if ac.auto_increment || bc.auto_increment {
            if ac.auto_increment != bc.auto_increment {
                self.advise(format!(
                    "!! {obj} 自增属性两侧不一致：实况 {} ｜ 期望 {} —— \
                     引擎暂不改写自增属性（不比对 default，也不动序列），需人工对齐声明；\
                     实况 default {} ｜ 期望 default {}",
                    ac.auto_increment,
                    bc.auto_increment,
                    ac.default.as_deref().unwrap_or("-"),
                    bc.default.as_deref().unwrap_or("-")
                ));
            }
        } else if ac.default.is_some() != bc.default.is_some() {
            self.push(
                if bc.default.is_some() {
                    ChangeKind::SetDefault
                } else {
                    ChangeKind::DropDefault
                },
                obj.clone(),
                format!(
                    "default {} → {}",
                    ac.default.as_deref().unwrap_or("-"),
                    bc.default.as_deref().unwrap_or("-")
                ),
                ChangePayload::Column { table: table.to_string(), col: bc.clone() },
            );
        } else if ac.default != bc.default {
            self.advise(format!(
                "text-diff {obj} default：实况 {} ｜ 期望 {}",
                ac.default.as_deref().unwrap_or("-"),
                bc.default.as_deref().unwrap_or("-")
            ));
        }
    }

    // ─────────────────────────── 索引 ───────────────────────────

    fn index_level(&mut self, a: &TableModel, b: &TableModel) {
        let a_by_name: HashMap<String, &IndexModel> =
            a.indexes.iter().map(|i| (i.key(), i)).collect();
        let b_by_name: HashMap<String, &IndexModel> =
            b.indexes.iter().map(|i| (i.key(), i)).collect();

        let mut consumed_actual: HashSet<String> = HashSet::new();

        // B 侧逐个对账
        //
        // ⚠ 遍历 `b.indexes`（`finalize` 后已按名排序）而**不是** `b_by_name` 这个
        // HashMap：「哪条 A 侧索引被认领为某条 B 侧索引的改名前身」取决于先处理哪条
        // B 侧索引（`consumed_actual` 是贪心的），遍历 HashMap 会让同一份输入产出
        // 随哈希种子漂移的 plan。
        for bi in &b.indexes {
            let key = bi.key();
            match a_by_name.get(&key) {
                Some(ai) if same_index(ai, bi) => {},
                Some(ai) => {
                    self.push(
                        ChangeKind::DropIndex,
                        key.clone(),
                        format!(
                            "定义不一致，先删后建：实况 {} ｜ 期望 {}",
                            index_desc(ai),
                            index_desc(bi)
                        ),
                        // `ai` 来自 `HashMap<String, &IndexModel>` 的 `get` ⇒ 是 `&&IndexModel`，
                        // `.clone()` 只解一层（拿到的仍是引用）⇒ 必须显式解引用。
                        ChangePayload::Index { table: a.name.clone(), def: (*ai).clone() },
                    );
                    self.push(
                        ChangeKind::CreateIndex,
                        key.clone(),
                        index_desc(bi),
                        ChangePayload::Index { table: a.name.clone(), def: bi.clone() },
                    );
                },
                None => {
                    // 改名识别：实况里存在「定义等价、名字不在期望侧」的索引
                    let twin = a.indexes.iter().find(|ai| {
                        !b_by_name.contains_key(&ai.key())
                            && !consumed_actual.contains(&ai.key())
                            && same_index(ai, bi)
                    });
                    match twin {
                        Some(ai) => {
                            consumed_actual.insert(ai.key());
                            self.push(
                                ChangeKind::RenameIndex,
                                key.clone(),
                                format!("{} → {}（列集/unique/method 一致）", ai.key(), key),
                                ChangePayload::Rename {
                                    table: a.name.clone(),
                                    from: ai.key(),
                                    to: key.clone(),
                                },
                            );
                        },
                        None => self.push(
                            ChangeKind::CreateIndex,
                            key.clone(),
                            index_desc(bi),
                            ChangePayload::Index { table: a.name.clone(), def: bi.clone() },
                        ),
                    }
                },
            }
        }

        // A 侧孤儿索引
        for ai in &a.indexes {
            let key = ai.key();
            if b_by_name.contains_key(&key) || consumed_actual.contains(&key) {
                continue;
            }
            // 主键索引在两侧都被跳过（introspect / expected 分流），走到这里都是真实索引
            self.push(
                ChangeKind::DropIndex,
                key.clone(),
                format!("实况有索引、L1+L2 均无声明：{}", index_desc(ai)),
                ChangePayload::Index { table: a.name.clone(), def: ai.clone() },
            );
        }
    }

    // ─────────────────────────── 外键 ───────────────────────────

    fn fk_level(&mut self, table: &str, a: &TableModel, b: &TableModel) {
        let a_keys: Vec<String> = a.fks.iter().map(fk_key).collect();
        let b_keys: Vec<String> = b.fks.iter().map(fk_key).collect();
        let mut consumed: HashSet<usize> = HashSet::new();

        for (i, bf) in b.fks.iter().enumerate() {
            let k = &b_keys[i];
            if a_keys.contains(k) {
                continue;
            }
            // 改名识别：实况里定义等价但名字不同的 FK
            let twin = a.fks.iter().enumerate().find(|(j, af)| {
                !b_keys.contains(&a_keys[*j]) && !consumed.contains(j) && fk_def_eq(af, bf)
            });
            match twin {
                Some((j, af)) => {
                    consumed.insert(j);
                    self.push(
                        ChangeKind::RenameForeignKey,
                        format!("{table}.{}", bf.cols.join(",")),
                        format!(
                            "{} → {}（定义一致）",
                            af.name.as_deref().unwrap_or("-"),
                            bf.name.as_deref().unwrap_or("-")
                        ),
                        ChangePayload::Rename {
                            table: table.to_string(),
                            from: af.name.clone().unwrap_or_default(),
                            to: bf.name.clone().unwrap_or_default(),
                        },
                    );
                },
                None => self.push(
                    ChangeKind::AddForeignKey,
                    format!("{table}.{}", bf.cols.join(",")),
                    fk_desc(bf),
                    ChangePayload::Fk { table: table.to_string(), def: bf.clone() },
                ),
            }
        }

        for (j, af) in a.fks.iter().enumerate() {
            if consumed.contains(&j) || b_keys.contains(&a_keys[j]) {
                continue;
            }
            self.push(
                ChangeKind::DropForeignKey,
                format!("{table}.{}", af.cols.join(",")),
                format!("实况有 FK、L1 无声明：{}", fk_desc(af)),
                ChangePayload::Fk { table: table.to_string(), def: af.clone() },
            );
        }
    }

    // ─────────────────────────── CHECK ───────────────────────────

    fn check_level(&mut self, table: &str, a: &TableModel, b: &TableModel) {
        let a_by_key: HashMap<String, &CheckModel> =
            a.checks.iter().map(|c| (c.key(), c)).collect();
        let b_by_key: HashMap<String, &CheckModel> =
            b.checks.iter().map(|c| (c.key(), c)).collect();

        // 同上：遍历已排序的 `b.checks` 而非 HashMap，保证 advisory 顺序稳定
        for bc in &b.checks {
            let k = &bc.key();
            match a_by_key.get(k) {
                Some(ac) if ac.expr == bc.expr => {},
                Some(ac) => self.advise(format!(
                    "text-diff {table}.{k} check：实况 {} ｜ 期望 {}（按名判同一约束，不产生变更）",
                    ac.expr, bc.expr
                )),
                None => self.push(
                    ChangeKind::AddCheck,
                    format!("{table}.{k}"),
                    bc.expr.clone(),
                    ChangePayload::Check { table: table.to_string(), def: bc.clone() },
                ),
            }
        }
        for ac in &a.checks {
            let k = &ac.key();
            if b_by_key.contains_key(k) {
                continue;
            }
            self.push(
                ChangeKind::DropCheck,
                format!("{table}.{k}"),
                format!("实况有 CHECK、L2 无声明：{}", ac.expr),
                ChangePayload::Check { table: table.to_string(), def: ac.clone() },
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 等价判定与拓扑排序
// ═══════════════════════════════════════════════════════════════════════════

/// 两条索引是否**结构等价**。
///
/// ## 四项都要走同一把尺子（2026-09-16 修正）
///
/// `method` 与 `where_clause` 一直走 [`normalize_sql_expr`]，而 `cols` 是**裸逐字比较**
/// （`a.cols == b.cols`）—— 这个不对称漏了一整类等价写法，代价是一次启动中止：
///
/// | 侧 | `idx_index_jobs_status` 的列 |
/// |---|---|
/// | L2 声明（`extras.rs:903-904`） | `["status", "priority DESC", "created_at"]` |
/// | 实况（存量库由已删的 `v100` 合并基线建出） | `(status, priority DESC, created_at ASC)` |
///
/// `ASC` 是默认方向 ⇒ 两侧等价，但裸比较判「不等」⇒ **同名**索引被拆成
/// `DropIndex` + `CreateIndex`（见本文件 `index_level` 的 `Some(ai)` 分支）；
/// `additive_only` 滤掉前者 ⇒ 只剩「建已存在的索引」⇒ fail-stop（第一现场
/// `p5_engine_takeover --legacy-sqlite`）。根因修在 `model::strip_default_sort_dir`。
///
/// ⇒ 判据必须对**语义等价的写法差异**不敏感，而这类差异恰好集中在表达式文本上；
/// 每一项各自裸比一次，就是给同一类缺陷留 N 个入口。
///
/// ⚠ 归一化只会**减少**报告的差异，不会漏掉真实差异：`DESC`、`NULLS FIRST/LAST`、
/// 列名/列序、`unique`、`method` 全都有语义且都被保留（见 `strip_default_sort_dir` 的边界）。
fn same_index(a: &IndexModel, b: &IndexModel) -> bool {
    norm_cols(&a.cols) == norm_cols(&b.cols)
        && a.unique == b.unique
        && norm_opt(&a.method) == norm_opt(&b.method)
        && norm_opt(&a.where_clause) == norm_opt(&b.where_clause)
}

/// 索引列表达式逐个归一（理由见 [`same_index`] 的文档）。
fn norm_cols(cols: &[String]) -> Vec<String> {
    cols.iter().map(|c| super::model::normalize_sql_expr(c)).collect()
}

fn norm_opt(v: &Option<String>) -> Option<String> {
    // `as_deref()` 已经是 `Option<&str>` ⇒ 直接传函数，不必包一层闭包（clippy `redundant_closure`）。
    // 注意上面 `norm_cols` 里的 `|c| normalize_sql_expr(c)` **不能**这样改：那里 `c` 是 `&String`，
    // 需要闭包提供 `&String → &str` 的 deref coercion。
    v.as_deref().map(super::model::normalize_sql_expr)
}

fn index_desc(i: &IndexModel) -> String {
    format!(
        "cols[{}] unique={} method={} where={}",
        i.cols.join(","),
        i.unique,
        i.method.as_deref().unwrap_or("btree"),
        i.where_clause.as_deref().unwrap_or("-")
    )
}

/// FK 的身份键：**列集 + 引用表 + 引用列**（名字不参与 —— 名字差异走改名通道）。
fn fk_key(f: &FkModel) -> String {
    format!("{}→{}({})", f.cols.join(","), f.ref_table, f.ref_cols.join(","))
}

fn fk_def_eq(a: &FkModel, b: &FkModel) -> bool {
    fk_key(a) == fk_key(b) && a.on_delete == b.on_delete && a.on_update == b.on_update
}

fn fk_desc(f: &FkModel) -> String {
    format!(
        "→ {}({}) on_delete={} on_update={} name={}",
        f.ref_table,
        f.ref_cols.join(","),
        f.on_delete.as_deref().unwrap_or("-"),
        f.on_update.as_deref().unwrap_or("-"),
        f.name.as_deref().unwrap_or("-")
    )
}

/// 建表用的 FK 拓扑序（被引用的表先建）。
///
/// `subset` 里只含**本次要新建**的表 —— 只有它们之间的 FK 边才有意义（引用已存在
/// 的表不构成约束）。环（互相引用）按名字升序打破，并留下确定性输出；engine 在
/// P4 遇到环时需要 `ALTER TABLE ADD CONSTRAINT` 二次通过，这里先如实排序。
fn topo_order(model: &SchemaModel, subset: &BTreeSet<String>) -> Vec<String> {
    // edges: child -> 需要先建的父表（仅子集内）
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for name in subset {
        let mut set = BTreeSet::new();
        if let Some(t) = model.table(name) {
            for fk in &t.fks {
                if subset.contains(&fk.ref_table) {
                    set.insert(fk.ref_table.clone());
                }
            }
        }
        deps.insert(name.clone(), set);
    }

    let mut out: Vec<String> = Vec::new();
    while !deps.is_empty() {
        // 取出所有「依赖已全部输出」的表；按名升序保证确定性
        let ready: Vec<String> =
            deps.iter().filter(|(_, d)| d.is_empty()).map(|(n, _)| n.clone()).collect();
        let batch = if ready.is_empty() {
            // 环：按名取一个打破，并把它从别人的依赖里摘掉
            let first = deps.keys().next().cloned().expect("非空");
            vec![first]
        } else {
            ready
        };
        for n in batch {
            deps.remove(&n);
            for d in deps.values_mut() {
                d.remove(&n);
            }
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, nullable: bool) -> ColumnModel {
        ColumnModel::new(name, ty, nullable)
    }

    fn base(dialect: Dialect) -> SchemaModel {
        let mut m = SchemaModel::new(dialect);
        let mut t = TableModel::new("t1");
        t.upsert_column(col("id", "bigint", false));
        t.upsert_column(col("name", "text", true));
        t.primary_key = vec!["id".into()];
        m.upsert_table(t);
        m.finalize();
        m
    }

    /// 完全一致 ⇒ 零变更零孤儿（这是「短路」之外的第二道自证：diff 不会凭空造变更）。
    #[test]
    fn identical_models_produce_empty_plan() {
        let m = base(Dialect::Postgres);
        let p = diff(&m, &m, Dialect::Postgres);
        assert!(p.changes.is_empty(), "同模型不该有变更：{:?}", p.changes);
        assert!(p.orphans().is_empty());
    }

    /// 空库 vs 期望 ⇒ 全部表都在「建」侧，且 FK 拓扑序（被引用者先建）。
    #[test]
    fn empty_actual_creates_everything_in_fk_topology() {
        let mut e = SchemaModel::new(Dialect::Postgres);
        let mut child = TableModel::new("a_child");
        child.upsert_column(col("id", "bigint", false));
        child.fks.push(FkModel {
            name: Some("fk-a_child-parent_id".into()),
            cols: vec!["parent_id".into()],
            ref_table: "z_parent".into(),
            ref_cols: vec!["id".into()],
            on_delete: None,
            on_update: None,
        });
        let mut parent = TableModel::new("z_parent");
        parent.upsert_column(col("id", "bigint", false));
        e.upsert_table(child);
        e.upsert_table(parent);
        e.finalize();

        let p = diff(&e, &SchemaModel::new(Dialect::Postgres), Dialect::Postgres);

        let creates: Vec<&Change> =
            p.changes.iter().filter(|c| c.kind == ChangeKind::CreateTable).collect();
        assert_eq!(creates.len(), 2);
        // 拓扑序：z_parent 必须在 a_child 之前（名字恰好相反，故这是真判据）
        assert_eq!(
            creates[0].object,
            "z_parent",
            "被引用的表应先建：{:?}",
            creates.iter().map(|c| &c.object).collect::<Vec<_>>()
        );
        assert_eq!(creates[1].object, "a_child");
    }

    // ── 启动期 bootstrap 的判据（`is_purely_additive`） ──

    /// [`ChangeKind::ALL`] **每一类**都必须在 bootstrap 分类表里被显式判定。
    ///
    /// 为什么需要它：[`ChangeKind::is_purely_additive`] 是**白名单**，新加一类
    /// `ChangeKind` 时默认落在「不参与启动期 bootstrap」这一侧 —— 默认值本身是安全的
    /// 方向，但「加了新类别却忘了分类」不会被任何东西发现（白名单的 `matches!` 没有
    /// 穷举检查，编译器帮不上忙）。这条测试遍历 `ALL` 逐项比对人工判定表，漏一个就红。
    #[test]
    fn every_change_kind_is_classified_for_bootstrap() {
        // 人工判定的完整输出：可在**无人审查的启动路径**上自动执行的类别。
        const ADDITIVE: [ChangeKind; 7] = [
            ChangeKind::CreateTable,
            ChangeKind::AddColumn,
            ChangeKind::CreateIndex,
            ChangeKind::AddForeignKey,
            ChangeKind::AddCheck,
            ChangeKind::AddUnique,
            ChangeKind::SetDefault,
        ];

        for k in ChangeKind::ALL {
            assert_eq!(
                k.is_purely_additive(),
                ADDITIVE.contains(&k),
                "ChangeKind::{k:?} 的 bootstrap 分类与判定表不符 —— \
                 新增类别后必须在这里显式分类"
            );
        }

        // 不变量：additive 白名单与「丢数据」「丢结构」两个集合**不相交**。
        // 比上一段更强 —— 它挡住的是「把 DROP 误加进白名单」这类改动，
        // 而那类改动在启动路径上等于无声删用户数据。
        for k in ChangeKind::ALL {
            if k.is_purely_additive() {
                assert!(!k.destructive(), "additive 白名单不得含破坏性类别 {k:?}");
                assert!(!k.loses_data(), "additive 白名单不得含丢数据类别 {k:?}");
            }
        }
    }

    /// bootstrap 收窄后，`n_safe` / `n_destructive` 必须**重算**。
    ///
    /// 只看 `changes` 长度会漏掉这条：两个计数字段是闸 2（破坏性配额）与报告数字的
    /// 输入。若 `retain` 后不同步重算，闸 2 会拿一个「含已被过滤掉的 DROP」的数字去比
    /// 配额 —— 计划里明明一条 DROP 都没有，却报「破坏性超配额」。
    #[test]
    fn retain_purely_additive_recomputes_the_counts() {
        // 本模块的 `mod tests` 没有 `change()` / `plan_of()` —— 那两个是
        // `apply::tests` 的私有 helper。就地构造，让测试自证自足。
        let mk = |kind: ChangeKind, object: &str| Change {
            kind,
            object: object.to_string(),
            destructive: kind.destructive(),
            loses_data: kind.loses_data(),
            detail: String::new(),
            payload: ChangePayload::Bare,
        };
        let mut p = Plan {
            changes: vec![
                mk(ChangeKind::CreateTable, "t_new"),
                mk(ChangeKind::AddColumn, "t_new.c"),
                mk(ChangeKind::DropTable, "t_old"),
                mk(ChangeKind::AlterColumnType, "t_new.c"),
            ],
            ..Plan::default()
        };
        // `Plan::default()` 的两个计数是 0，必须显式填成与 `changes` 一致 ——
        // 否则「retain 后重算」这条判据没有区分力（0 → 0 恒成立，测试永远绿）。
        p.n_destructive = p.changes.iter().filter(|c| c.destructive).count();
        p.n_safe = p.changes.len() - p.n_destructive;

        // 前置断言：收窄前两类都在，且计数认得出它们
        assert_eq!(p.changes.len(), 4);
        assert_eq!(p.n_destructive, 2, "前置：DROP TABLE 与 ALTER TYPE 都属破坏性");

        p.retain_purely_additive();

        assert_eq!(
            p.changes.iter().map(|c| c.object.as_str()).collect::<Vec<_>>(),
            vec!["t_new", "t_new.c"],
            "只应留下纯新增的两条"
        );
        assert_eq!(p.n_destructive, 0, "被过滤掉的破坏性条目不得再计入 n_destructive");
        assert_eq!(p.n_safe, 2);
        assert_eq!(p.n_safe + p.n_destructive, p.changes.len(), "分账必须闭合");
    }

    /// **自增列不得被判 `DROP DEFAULT`** —— 两侧对「自增」的表达方式不同（真库实测缺陷）。
    ///
    /// 实况是「一个序列 + `DEFAULT nextval('…_seq')`」⇒ `default` 非空；声明是
    /// `auto_increment = true`，sea-orm 写进 `auto_increment()` 而**不写 `default`**。
    /// 照「有没有默认值」比就必然产出 `DROP DEFAULT`，而执行它等于把自增列降级成普通列。
    /// 生产库实测 10 个 `nextval` 列，其中 **6 个的声明本来是对的**却仍被计划成
    /// `DROP DEFAULT`（`output/tmp-p4-serial.log`）。
    #[test]
    fn sequence_bound_column_is_not_reported_as_drop_default() {
        let e = {
            let mut m = SchemaModel::new(Dialect::Postgres);
            let mut t = TableModel::new("notes");
            let mut id = col("id", "bigint", false);
            id.auto_increment = true;
            t.upsert_column(id);
            t.primary_key = vec!["id".into()];
            m.upsert_table(t);
            m.finalize();
            m
        };
        let a = {
            let mut m = SchemaModel::new(Dialect::Postgres);
            let mut t = TableModel::new("notes");
            let mut id = col("id", "bigint", false);
            id.auto_increment = true;
            id.default = Some("nextval('notes_id_seq'::regclass)".into());
            t.upsert_column(id);
            t.primary_key = vec!["id".into()];
            m.upsert_table(t);
            m.finalize();
            m
        };

        let p = diff(&e, &a, Dialect::Postgres);
        let defaults: Vec<&Change> = p
            .changes
            .iter()
            .filter(|c| matches!(c.kind, ChangeKind::DropDefault | ChangeKind::SetDefault))
            .collect();
        assert!(defaults.is_empty(), "自增列的序列绑定不是「默认值」：{defaults:?}");
        assert!(
            !p.advisories.iter().any(|s| s.contains("自增属性")),
            "两侧都是自增 ⇒ 不该有 advisory：{:?}",
            p.advisories
        );
    }

    /// 自增属性**两侧不一致**时出 advisory 而非 DDL。
    ///
    /// 这是「不静默」口径：引擎暂不改写自增属性（PG 侧要 `ADD GENERATED` / 建序列 /
    /// 改默认值三件事），但**必须报出来** —— 静默当成一致会让这个字段变成装饰。
    /// 实测生产库有 3 张表属于这一情形（实体没写 `auto_increment`，库里是 `SERIAL`）。
    #[test]
    fn auto_increment_mismatch_is_advisory_not_ddl() {
        let e = {
            let mut m = SchemaModel::new(Dialect::Postgres);
            let mut t = TableModel::new("gateway_usage");
            t.upsert_column(col("id", "bigint", false)); // 声明侧漏了 auto_increment
            t.primary_key = vec!["id".into()];
            m.upsert_table(t);
            m.finalize();
            m
        };
        let a = {
            let mut m = SchemaModel::new(Dialect::Postgres);
            let mut t = TableModel::new("gateway_usage");
            let mut id = col("id", "bigint", false);
            id.auto_increment = true;
            id.default = Some("nextval('gateway_usage_id_seq'::regclass)".into());
            t.upsert_column(id);
            t.primary_key = vec!["id".into()];
            m.upsert_table(t);
            m.finalize();
            m
        };

        let p = diff(&e, &a, Dialect::Postgres);
        assert!(
            !p.changes.iter().any(|c| c.kind == ChangeKind::DropDefault),
            "不一致也**不得**产出 DROP DEFAULT（那会打掉序列默认值）：{:?}",
            p.changes
        );
        assert!(
            p.advisories.iter().any(|s| s.contains("自增属性两侧不一致")),
            "必须显式报出不一致：{:?}",
            p.advisories
        );
    }

    /// 孤儿表被判 DROP，且标记为「丢数据」+ 破坏性。
    #[test]
    fn orphan_table_is_drop_and_loses_data() {
        let e = SchemaModel::new(Dialect::Postgres);
        let a = base(Dialect::Postgres);
        let p = diff(&e, &a, Dialect::Postgres);
        let d =
            p.changes.iter().find(|c| c.kind == ChangeKind::DropTable).expect("应有 DROP TABLE");
        assert_eq!(d.object, "t1");
        assert!(d.destructive && d.loses_data, "删表必须同时标记破坏性与丢数据");
        assert_eq!(p.orphans().len(), 1);
    }

    /// **向量族不判孤儿（基表 + 元数据表），备份残留继续判** —— 裁决 ①/② 的行为锁。
    ///
    /// 修前：`vec_capabilities` / `vec_wiki_…` 这些基表会出现在孤儿候选里（生产库实测
    /// 3 张共 49135 行）；元数据表则靠 `PATTERN_TSV` 的**副作用**保命。本测试同时锁住
    /// 两件事：豁免生效、**且**没有宽到把 `_dup_backup` 残留一起保下来。
    #[test]
    fn vector_family_is_orphan_exempt_but_backup_residue_is_not() {
        let e = SchemaModel::new(Dialect::Postgres);
        let mut a = SchemaModel::new(Dialect::Postgres);
        const REAL_BASE: &str = "vec_wiki_dc8efbd8_3a28_4fdf_9c96_9d26e2b2dac9";
        for name in [
            "vec_capabilities",           // 基表（生产库 63 行）
            REAL_BASE,                    // 基表（生产库 49072 行）
            "vec_kb_abc_meta",            // 元数据表（过去靠 PATTERN_TSV 顺带豁免）
            "vec_kb_abc_meta_dup_backup", // 备份残留（24518 行）
            "notes",                      // 非向量族（必须继续判孤儿）
        ] {
            let mut t = TableModel::new(name);
            t.upsert_column(col("id", "bigint", false));
            t.primary_key = vec!["id".into()];
            a.upsert_table(t);
        }
        a.finalize();

        let p = diff(&e, &a, Dialect::Postgres);
        let dropped: Vec<&str> = p
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::DropTable)
            .map(|c| c.object.as_str())
            .collect();

        for exempted in ["vec_capabilities", REAL_BASE, "vec_kb_abc_meta"] {
            assert!(!dropped.contains(&exempted), "{exempted} 已豁免，不该 DROP：{dropped:?}");
        }
        for residue in ["vec_kb_abc_meta_dup_backup", "notes"] {
            assert!(dropped.contains(&residue), "{residue} 必须继续判孤儿：{dropped:?}");
        }
        assert_eq!(dropped.len(), 2, "只该有残留 + 非向量族两个 DROP：{dropped:?}");

        // 豁免必须留痕 —— 否则「为什么少了一条 DROP」无法从产出本身复核。
        assert!(
            p.advisories.iter().any(|s| s.contains("exempt:") && s.contains("vec_capabilities")),
            "豁免要出现在 advisories 里便于逐对象复核：{:?}",
            p.advisories
        );
    }

    /// **L2 声明的 FTS5 虚表不判孤儿**；而 L2 **刻意未收录**的同类残留、以及普通孤儿表，
    /// 必须**继续**被判 DROP —— 这条测试的价值**全在下半段**。
    ///
    /// ## 修前行为（实测 2026-09-17，`p3_plan_probe` 对生产库副本）
    ///
    /// 4 张在 `extras::VIRTUAL_TABLES` 里声明过、且在库中存在的虚表全部落进
    /// 「仅实况（将被 DROP）」，计划原文形如：
    ///
    /// ```text
    /// !! 段1 DROP TABLE  messages_fts  实况有表、L1+L2 均无声明；列 0 个
    /// ```
    ///
    /// 其中 `messages_fts` 上挂着三条 FTS 同步触发器 ⇒ 执行即全文检索失效。
    ///
    /// ## 区分力：为什么不能用 `_fts` 后缀通配
    ///
    /// `extras.rs` 的 `VIRTUAL_TABLES` **刻意没有**收录 `trajectory_memories_fts`
    /// （`v101:372` 已随表 DROP 它），而它在存量库里**仍然存在** —— 真残留，必须继续清。
    /// 后缀通配是最省事的写法，也是**唯一会把这条残留一起放过**的写法，故必须锁住。
    ///
    /// 另锁两条边界：**不建**（豁免只改「不删」，没顺手让引擎去建一张同名普通表）、
    /// **不跨方言**（虚表声明是 `dialect: Some(Sqlite)`，PG 侧不得豁免）。
    #[test]
    fn l2_fts5_virtual_tables_are_orphan_exempt_but_residue_and_other_dialect_are_not() {
        let mut a = SchemaModel::new(Dialect::Sqlite);
        for name in [
            "messages_fts",            // L2 收录
            "trajectory_messages_fts", // L2 收录
            "trajectory_skills_fts",   // L2 收录
            "trajectory_memories_fts", // ⚠ L2 **刻意未收录** ⇒ 真残留，必须继续判孤儿
            "notes",                   // 普通表（L1 该声明而此处故意不声明）⇒ 孤儿
        ] {
            // 虚表在实况侧就是 0 列：`introspect` 只登记存在性，不读列
            // （`introspect/sqlite.rs` 的 `is_virtual` 分支）。
            a.upsert_table(TableModel::new(name));
        }
        a.finalize();

        let e = SchemaModel::new(Dialect::Sqlite);
        let p = diff(&e, &a, Dialect::Sqlite);
        let dropped: Vec<&str> = p
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::DropTable)
            .map(|c| c.object.as_str())
            .collect();

        // 上半段：L2 声明过的虚表必须从 DROP 候选里消失。
        for keep in ["messages_fts", "trajectory_messages_fts", "trajectory_skills_fts"] {
            assert!(!dropped.contains(&keep), "L2 声明过的虚表不该被 DROP：{dropped:?}");
        }
        // 下半段（区分力）：后缀通配的错法会把这两张一起放过。
        for must_drop in ["trajectory_memories_fts", "notes"] {
            assert!(dropped.contains(&must_drop), "{must_drop} 必须**继续**判孤儿：{dropped:?}");
        }
        assert_eq!(dropped.len(), 2, "只该有「真残留 + 普通孤儿」两项：{dropped:?}");

        // 豁免必须留痕，且理由要能读出**谁声明的** —— 否则「为什么少了一条 DROP」无从复核。
        assert!(
            p.advisories.iter().any(|s| s.contains("exempt:")
                && s.contains("messages_fts")
                && s.contains("VIRTUAL_TABLES")),
            "豁免理由必须写明来源（VIRTUAL_TABLES）：{:?}",
            p.advisories
        );

        // 边界一：**不建**。期望侧依旧不产生虚表的 `TableDecl` ⇒ 豁免只把「删」拿掉，
        // 没有顺手把它变成「引擎去建一张同名普通表」（那会毁掉 FTS5 虚表语义）。
        assert!(
            !p.changes
                .iter()
                .any(|c| c.kind == ChangeKind::CreateTable && c.object.ends_with("_fts")),
            "虚表不该进 CREATE TABLE（L2 的建表原文是 CREATE VIRTUAL TABLE）：{:?}",
            p.changes.iter().map(|c| (c.kind, c.object.as_str())).collect::<Vec<_>>()
        );

        // 边界二：**不跨方言**。`VIRTUAL_TABLES` 全部是 `dialect: Some(Sqlite)`。
        let pg = diff(&e, &a, Dialect::Postgres);
        let pg_dropped: Vec<&str> = pg
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::DropTable)
            .map(|c| c.object.as_str())
            .collect();
        assert!(
            pg_dropped.contains(&"messages_fts"),
            "PG 方言下 FTS5 虚表声明不适用，不得豁免：{pg_dropped:?}"
        );
    }

    /// **基础设施表不判孤儿**：`axagent_schema_version`（迁移子系统元表）必须从
    /// `DROP TABLE` 候选里消失，且**理由要能说明 owner**。
    ///
    /// 这条锁的是端到端行为（`diff` 的产出），不是规则本身 —— 规则单测在
    /// `expected::tests::exact_orphan_exempt_covers_only_listed_infrastructure_tables`。
    /// 两处都要有：规则单测防「写错范围」，这里防「规则写了但 plan 没接上」。
    #[test]
    fn schema_version_meta_table_is_orphan_exempt_with_owner_reason() {
        let e = SchemaModel::new(Dialect::Postgres);
        let mut a = SchemaModel::new(Dialect::Postgres);
        let mut t = TableModel::new("axagent_schema_version");
        t.upsert_column(col("id", "bigint", false));
        t.primary_key = vec!["id".into()];
        a.upsert_table(t);
        a.finalize();

        let p = diff(&e, &a, Dialect::Postgres);
        let dropped: Vec<&str> = p
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::DropTable)
            .map(|c| c.object.as_str())
            .collect();
        assert!(
            dropped.is_empty(),
            "迁移元表被判 DROP ⇒ 下一次 SeaORM 迁移会认为从未迁移过：{dropped:?}"
        );

        let trail = p
            .advisories
            .iter()
            .find(|s| s.contains("axagent_schema_version"))
            .unwrap_or_else(|| panic!("豁免必须在 advisories 里留痕：{:?}", p.advisories));
        assert!(trail.contains("owner"), "豁免理由必须写明 owner：{trail}");
    }

    /// **非主库实体不判孤儿，也不被建** —— plan 层的端到端守卫。
    ///
    /// 场景就是生产库现状：主库里真有 `ast_classes`（P4 首次执行误建的空壳，0 行），
    /// 而期望集里没有它（被 `expected::build` 按非主库声明排除）。
    ///
    /// 为什么这条必须存在（而不是只测 `expected` 那一侧）：`expected` 侧只保证「不建」，
    /// 而「实况有、声明无」在孤儿判定眼里与「某个子系统的私有表」**完全同形** ——
    /// 少接一根线，引擎就会按名字把别人家的表删掉。两处都要有：规则单测防「写错范围」，
    /// 这里防「规则写了但 plan 没接上」。
    ///
    /// 反向边界同样锁住：普通孤儿（`notes_orphan`）必须**继续**被删 —— 否则这条声明
    /// 一旦过宽，症状是整个孤儿判定静默失效。
    #[test]
    fn non_main_db_table_is_neither_created_nor_dropped() {
        // ⚠ 2026-09-17：**两个方言都要跑**。`NON_MAIN_DB` 的 `dialect` 已由
        // `Some(Dialect::Postgres)` 改为 `None`（双向退出），而**主库方言由用户在设置中
        // 设定** ⇒ 只看 PG 会漏掉「SQLite 主库那条路径上『既不建也不删』是否同样成立」，
        // 而漏掉的后果是引擎去 DROP 一批名字属于别的子系统的表。
        for dialect in [Dialect::Postgres, Dialect::Sqlite] {
            let e = SchemaModel::new(dialect);
            let mut a = SchemaModel::new(dialect);
            for name in ["ast_classes", "l2_search_results", "notes_orphan"] {
                let mut t = TableModel::new(name);
                t.upsert_column(col("id", "bigint", false));
                t.primary_key = vec!["id".into()];
                a.upsert_table(t);
            }
            a.finalize();

            let p = diff(&e, &a, dialect);
            let touched: Vec<&str> = p.changes.iter().map(|c| c.object.as_str()).collect();
            for sidecar in ["ast_classes", "l2_search_results"] {
                assert!(
                    !touched.contains(&sidecar),
                    "[{dialect:?}] {sidecar} 是侧车库表（家不在主库）⇒ 既不建也不删，\
                     实际被动了：{touched:?}"
                );
            }
            assert!(
                p.changes
                    .iter()
                    .any(|c| c.kind == ChangeKind::DropTable && c.object == "notes_orphan"),
                "[{dialect:?}] 普通孤儿必须继续判 DROP —— 否则说明这条声明过宽：{touched:?}"
            );

            // 留痕：排障时唯一能看的是理由串，且必须能分辨来源（`非主库实体：` ≠ `L2 规则：`）。
            let trail =
                p.advisories.iter().find(|s| s.contains("ast_classes")).unwrap_or_else(|| {
                    panic!("[{dialect:?}] 非主库实体必须在 advisories 里留痕：{:?}", p.advisories)
                });
            assert!(trail.contains("非主库实体"), "理由串必须标明来源：{trail}");
            assert!(trail.contains("owner"), "理由串必须写明 owner：{trail}");
        }
    }

    /// 运行期白名单（`PlanOptions::orphan_whitelist`）必须与静态规则同样生效，
    /// 且**理由要带来源** —— advisory 里能分辨「人工登记」与「L2 规则」。
    ///
    /// 这条是 `safety::_ax_schema_orphan_whitelist` 的功能前置：表建出来却不影响
    /// 孤儿判定，白名单就是个摆设（而症状是「登记了但表还是被删」）。
    #[test]
    fn runtime_whitelist_exempts_orphans_and_keeps_its_provenance() {
        let e = SchemaModel::new(Dialect::Postgres);
        let mut a = SchemaModel::new(Dialect::Postgres);
        for name in ["data_migration_temp", "truly_orphan", "vec_capabilities"] {
            let mut t = TableModel::new(name);
            t.upsert_column(col("id", "bigint", false));
            t.primary_key = vec!["id".into()];
            a.upsert_table(t);
        }
        a.finalize();

        // 基线：不带选项时**只有两张**判孤儿。第三张 `vec_capabilities` 被 L2 静态规则
        // 豁免 —— 若这里写成 3，说明基线本身对不上静态规则，那么「人工白名单消掉一条
        // 差异」这个结论就是拿错误基线比出来的。
        //
        // ⚠ 这里特意用**基表**（`vec_{集合}`）而不是 `{集合}_meta` 元表：两张表都能免于
        // 孤儿判定，但走的是**不同机制** —— 基表靠 `ORPHAN_EXEMPT`（advisory 前缀
        // `exempt:`），元表靠 `PATTERN_TSV` 动态覆盖（前缀 `pattern:`）。用元表做样本时
        // 下面那条「静态规则前缀也要在」断言会失败，而失败原因看着像「静态规则坏了」，
        // 其实只是样本选错了机制。
        assert_eq!(diff(&e, &a, Dialect::Postgres).orphans().len(), 2);

        let opts = PlanOptions {
            orphan_whitelist: BTreeMap::from([(
                "data_migration_temp".to_string(),
                "whitelist: 迁移中，至 2026-10-01".to_string(),
            )]),
        };
        let p = diff_with(&e, &a, Dialect::Postgres, &opts);
        let dropped: Vec<&str> = p
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::DropTable)
            .map(|c| c.object.as_str())
            .collect();

        assert!(!dropped.contains(&"data_migration_temp"), "白名单表不该被 DROP：{dropped:?}");
        assert!(dropped.contains(&"truly_orphan"), "白名单不该顺带豁免别的表：{dropped:?}");
        assert!(
            !dropped.contains(&"vec_capabilities"),
            "L2 静态豁免覆盖的表本来就不判孤儿：{dropped:?}"
        );
        assert_eq!(dropped.len(), 1, "{dropped:?}");

        // 留痕：两种来源必须可分辨，且人工理由自带有效期
        let wl = p
            .advisories
            .iter()
            .find(|s| s.contains("data_migration_temp"))
            .expect("白名单豁免必须留痕，否则「为什么少了一条 DROP」无从复核");
        assert!(wl.contains("whitelist:"), "{wl}");
        assert!(wl.contains("2026-10-01"), "理由要带有效期：{wl}");
        // 两种来源的前缀必须可分辨：「L2 规则：」（编译期规则）与「whitelist:」（人工登记）
        let static_rule = p
            .advisories
            .iter()
            .find(|s| s.contains("vec_capabilities"))
            .expect("静态规则的豁免也必须留痕");
        assert!(static_rule.contains("L2 规则："), "{static_rule}");
        assert!(
            static_rule.starts_with("exempt:"),
            "走的是 `exempt:`（ORPHAN_EXEMPT）而不是 `pattern:` 那条通道：{static_rule}"
        );

        // `Plan` 仍是纯值：同一模型 + 同一选项 ⇒ 同一结果（选项不引入隐藏状态）
        let again = diff_with(&e, &a, Dialect::Postgres, &opts);
        assert_eq!(p.changes, again.changes);
        assert_eq!(p.advisories, again.advisories);
    }

    /// 改名通道：`renamed_from` 命中 ⇒ RENAME 而不是「删 + 建」。
    #[test]
    fn renamed_from_produces_rename_not_drop_add() {
        let mut a = base(Dialect::Postgres);
        a.table_mut("t1").expect("表在").columns.clear();
        a.table_mut("t1").unwrap().upsert_column(col("id", "bigint", false));
        a.table_mut("t1").unwrap().upsert_column(col("old_name", "text", true));

        let mut e = base(Dialect::Postgres);
        let t = e.table_mut("t1").expect("表在");
        t.columns.clear();
        t.upsert_column(col("id", "bigint", false));
        let mut c = col("new_name", "text", true);
        c.renamed_from = Some("old_name".into());
        t.upsert_column(c);
        e.finalize();

        let p = diff(&e, &a, Dialect::Postgres);
        assert!(
            p.changes.iter().any(|c| c.kind == ChangeKind::RenameColumn),
            "应产出 RENAME COLUMN：{:?}",
            p.changes
        );
        assert!(
            !p.changes
                .iter()
                .any(|c| matches!(c.kind, ChangeKind::DropColumn | ChangeKind::AddColumn)),
            "有了改名就不该再删+建：{:?}",
            p.changes
        );
    }

    /// 索引改名按「定义等价 + 新名缺席实况」识别，不能判成删+建。
    #[test]
    fn index_rename_is_detected_by_definition() {
        let mut a = base(Dialect::Postgres);
        a.table_mut("t1").unwrap().upsert_index(IndexModel {
            name: "idx_t1_name_old".into(),
            cols: vec!["name".into()],
            unique: false,
            method: None,
            where_clause: None,
        });
        a.finalize();

        let mut e = base(Dialect::Postgres);
        e.table_mut("t1").unwrap().upsert_index(IndexModel {
            name: "idx-t1-name".into(),
            cols: vec!["name".into()],
            unique: false,
            method: None,
            where_clause: None,
        });
        e.finalize();

        let p = diff(&e, &a, Dialect::Postgres);
        assert!(
            p.changes.iter().any(|c| c.kind == ChangeKind::RenameIndex),
            "应识别为改名：{:?}",
            p.changes
        );
        assert!(!p.changes.iter().any(|c| c.kind == ChangeKind::DropIndex));
    }

    /// **表达式文本差异不得产出变更**（PG 会重写 default/generated 文本）。
    ///
    /// 这是 P3 出口判据可满足的前提：库侧 1333 列 `text` 与 PG 补的 `::text` 转换
    /// 属于同一类噪声，若判成差异，plan 永远清不空。
    #[test]
    fn expression_text_diff_is_advisory_only() {
        let mut a = base(Dialect::Postgres);
        a.table_mut("t1").unwrap().columns.clear();
        a.table_mut("t1").unwrap().upsert_column(col("id", "bigint", false));
        let mut c = col("k", "text", true);
        c.default = Some("'live'::text".into());
        c.generated = Some("to_tsvector('simple'::regconfig, coalesce(x, ''::text))".into());
        a.table_mut("t1").unwrap().upsert_column(c);
        a.finalize();

        let mut e = base(Dialect::Postgres);
        e.table_mut("t1").unwrap().columns.clear();
        e.table_mut("t1").unwrap().upsert_column(col("id", "bigint", false));
        let mut c = col("k", "text", true);
        c.default = Some("'live'".into());
        c.generated = Some("to_tsvector('simple', coalesce(x, ''))".into());
        e.table_mut("t1").unwrap().upsert_column(c);
        e.finalize();

        let p = diff(&e, &a, Dialect::Postgres);
        assert!(p.changes.is_empty(), "文本差异不该产出变更：{:?}", p.changes);
        assert_eq!(p.advisories.len(), 2, "应留下两条诊断：{:?}", p.advisories);
    }

    /// 「有没有默认值」不一致 ⇒ 产出变更（结构差异必须被抓住）。
    #[test]
    fn default_presence_diff_is_a_change() {
        let mut a = base(Dialect::Postgres);
        a.table_mut("t1").unwrap().columns.clear();
        a.table_mut("t1").unwrap().upsert_column(col("id", "bigint", false));
        a.table_mut("t1").unwrap().upsert_column(col("k", "text", true));
        a.finalize();

        let mut e = base(Dialect::Postgres);
        e.table_mut("t1").unwrap().columns.clear();
        e.table_mut("t1").unwrap().upsert_column(col("id", "bigint", false));
        let mut c = col("k", "text", true);
        c.default = Some("'live'".into());
        e.table_mut("t1").unwrap().upsert_column(c);
        e.finalize();

        let p = diff(&e, &a, Dialect::Postgres);
        assert!(
            p.changes.iter().any(|c| c.kind == ChangeKind::SetDefault),
            "应产出 SET DEFAULT：{:?}",
            p.changes
        );
    }

    /// **变更排序不变量**：段位单调不减；且 `DROP INDEX` 早于 `DROP COLUMN`。
    #[test]
    fn changes_are_phase_ordered() {
        let mut a = base(Dialect::Postgres);
        a.table_mut("t1").unwrap().upsert_index(IndexModel {
            name: "idx_orphan".into(),
            cols: vec!["name".into()],
            unique: false,
            method: None,
            where_clause: None,
        });
        a.finalize();
        // 期望侧：删掉 name 列，并新增一个索引
        let mut e = base(Dialect::Postgres);
        let t = e.table_mut("t1").unwrap();
        t.columns.retain(|c| c.name != "name");
        t.upsert_index(IndexModel {
            name: "idx-t1-id".into(),
            cols: vec!["id".into()],
            unique: false,
            method: None,
            where_clause: None,
        });
        e.finalize();

        let p = diff(&e, &a, Dialect::Postgres);
        let phases: Vec<u8> = p.changes.iter().map(|c| c.kind.phase()).collect();
        assert!(phases.windows(2).all(|w| w[0] <= w[1]), "段位应单调不减：{phases:?}");
        let drop_idx = p.changes.iter().position(|c| c.kind == ChangeKind::DropIndex);
        let drop_col = p.changes.iter().position(|c| c.kind == ChangeKind::DropColumn);
        assert!(drop_idx.is_some() && drop_col.is_some());
        assert!(drop_idx < drop_col, "DROP INDEX 必须在 DROP COLUMN 之前");
    }

    /// 拓扑排序：互为引用的环不许死循环，且输出确定性（两次结果一致）。
    #[test]
    fn topo_order_breaks_cycles_deterministically() {
        let mut m = SchemaModel::new(Dialect::Postgres);
        for (a, b) in [("alpha", "beta"), ("beta", "alpha")] {
            let mut t = TableModel::new(a);
            t.upsert_column(col("id", "bigint", false));
            t.fks.push(FkModel {
                name: None,
                cols: vec!["other_id".into()],
                ref_table: b.into(),
                ref_cols: vec!["id".into()],
                on_delete: None,
                on_update: None,
            });
            m.upsert_table(t);
        }
        m.finalize();
        let subset: BTreeSet<String> =
            ["alpha".to_string(), "beta".to_string()].into_iter().collect();
        let one = topo_order(&m, &subset);
        let two = topo_order(&m, &subset);
        assert_eq!(one.len(), 2);
        assert_eq!(one, two, "环上的打破必须确定性");
    }

    /// `ChangeKind::ALL` 是「21 类」的唯一来源 ⇒ 它自己必须无重复、无空标签、段号合法。
    #[test]
    fn change_kind_all_is_complete_and_unique() {
        assert_eq!(ChangeKind::ALL.len(), 21, "变体数变了：改 `ALL` 的数组类型长度并同步本断言");
        let mut labels: Vec<&str> = ChangeKind::ALL.iter().map(|k| k.as_str()).collect();
        labels.sort_unstable();
        let before = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), before, "`as_str` 有重复标签：{labels:?}");

        // 段划分必须落在已定义的 6 段内，否则排序会把它扔到未定义位置。
        // 这条能查出「新类别忘了归段」—— 那种情况下 `phase()` 必然要改，改错就露馅。
        for k in ChangeKind::ALL {
            assert!(k.phase() <= 5, "{} 的 phase 越界", k.as_str());
        }
        // 每段都非空（段号是密集的，没有空洞），否则排序表里会有一段永远不出现
        let phases: BTreeSet<u8> = ChangeKind::ALL.iter().map(|k| k.phase()).collect();
        assert_eq!(phases, BTreeSet::from([0u8, 1, 2, 3, 4, 5]), "段号有空洞：{phases:?}");
    }

    /// `object_op` 必须把 21 类**全部**归位，且增/删两侧与 `is_purely_additive` 不冲突。
    ///
    /// 这条守的是 `retain_purely_additive` 第二刀的输入完整性：漏登记一类 ⇒ 那条变更逃过
    /// 自洽化 ⇒ 「`CREATE X` 时 X 已存在 ⇒ fail-stop」。它同时是 `match` 穷举的运行时镜像
    /// （编译期靠穷举拦，运行期靠这条核「归位是否合理」）。
    #[test]
    fn object_op_covers_all_kinds_and_agrees_with_additivity() {
        let mut creates: Vec<&str> = ChangeKind::ALL
            .iter()
            .filter(|k| k.object_op() == Some(ObjOp::Create))
            .map(|k| k.as_str())
            .collect();
        let drops: Vec<&str> = ChangeKind::ALL
            .iter()
            .filter(|k| k.object_op() == Some(ObjOp::Drop))
            .map(|k| k.as_str())
            .collect();

        // 反向断言 1：两侧都必须非空 —— 否则「覆盖 21 类」这句话在 `_ => None` 的写法下
        // 也能成立（全归 None 时两个列表都是空的，而那条判据什么错都抓不到）。
        assert!(!creates.is_empty() && !drops.is_empty(), "{creates:?} / {drops:?}");
        assert_eq!(creates.len() + drops.len() + 9, ChangeKind::ALL.len(), "有类别未被归位");

        // 反向断言 2：`Create` 侧必须**恰好**是 additive 白名单里「会 CREATE 具名对象」的那 6 类。
        // `SetDefault` 是 additive 但**不** CREATE 具名对象（它是覆写属性），故不在 Create 侧 ——
        // 这正是「危险性与撞名风险是两件事」的落点。
        //
        // 排序后比较：`ChangeKind::ALL` 的顺序是**声明顺序**，与下面这份期望列表无关；
        // 按顺序比会变成一条「换个声明顺序就红」的假判据（它抓不到任何真缺陷）。
        creates.sort_unstable();
        assert_eq!(
            creates,
            vec!["ADD CHECK", "ADD COLUMN", "ADD FK", "ADD UNIQUE", "CREATE INDEX", "CREATE TABLE"],
            "Create 侧应为「会 CREATE 具名对象」的那 6 类"
        );
        for k in ChangeKind::ALL {
            if k.object_op() == Some(ObjOp::Create) {
                assert!(
                    k.is_purely_additive(),
                    "{} 会 CREATE 却不属纯新增 —— 两套判据打架",
                    k.as_str()
                );
            }
        }
    }

    /// ⚠ **实测缺陷的回归判据**：`ASC` 是默认排序方向 ⇒ 两侧等价 ⇒ **不许产变更**。
    ///
    /// 第一现场：`idx_index_jobs_status` 在声明侧写 `created_at`、在实况侧（v100 建的）
    /// 是 `created_at ASC`，裸 `cols` 比较判不等 ⇒ 同一条索引被拆成**同名**的
    /// `DropIndex` + `CreateIndex` ⇒ `additive_only` 只留后者 ⇒ 执行撞
    /// `index … already exists` ⇒ **启动中止**。
    ///
    /// 区分力（另一半必须红，否则这条判据可以被「一律判等」蒙混）：
    /// `DESC` ≠ 无修饰、列序不同、列名不同 —— 这三类都必须**仍然报差异**。
    #[test]
    fn asc_is_the_default_direction_and_must_not_be_a_difference() {
        let mk = |cols: Vec<&str>| -> SchemaModel {
            let mut m = base(Dialect::Sqlite);
            let t = m.table_mut("t1").unwrap();
            t.upsert_column(col("priority", "integer", false));
            t.upsert_column(col("created_at", "bigint", false));
            t.upsert_index(IndexModel {
                name: "idx_index_jobs_status".into(),
                cols: cols.into_iter().map(String::from).collect(),
                unique: false,
                method: None,
                where_clause: None,
            });
            m.finalize();
            m
        };

        // ── 主题：复刻第一现场的两侧原文 ──
        //   声明（`extras.rs:903-904`）：["status", "priority DESC", "created_at"]
        //   实况（`v100:1114` 建的）：(status, priority DESC, created_at ASC)
        let expected = mk(vec!["id", "priority DESC", "created_at"]);
        let actual = mk(vec!["id", "priority DESC", "created_at ASC"]);
        let p = diff(&expected, &actual, Dialect::Sqlite);
        assert!(
            p.changes.is_empty(),
            "`ASC` 是默认方向，两侧语义等价，不该产出任何变更：{:?}",
            p.changes
        );

        // 大小写不敏感：SQLite 会原样保留声明文本，作者写 `asc` 也一样
        assert!(
            diff(&expected, &mk(vec!["id", "priority DESC", "created_at asc"]), Dialect::Sqlite)
                .changes
                .is_empty(),
            "`asc` 与 `ASC` 都是默认方向"
        );

        // ── 区分力：这三类是真差异，必须报出来（否则「一律判等」也能骗过上面两条）──
        for (label, bad_cols) in [
            ("DESC 有语义，抹平它就会漏掉真差异", vec!["id", "priority", "created_at"]),
            ("列序不同是真差异", vec!["id", "created_at", "priority DESC"]),
            ("列名不同是真差异", vec!["id", "priority DESC", "other"]),
        ] {
            let pb = diff(&expected, &mk(bad_cols), Dialect::Sqlite);
            assert!(
                pb.changes.iter().any(|c| c.kind == ChangeKind::CreateIndex),
                "{label}：{:?}",
                pb.changes
            );
        }
    }

    /// ⚠ **防线判据**：`retain_purely_additive` 必须把「成对重建」两半一起收窄。
    ///
    /// 只留 `CreateIndex` 会发出一条**必然失败**的语句（同名对象还在）。这条判据不依赖
    /// 「`ASC` 归一化」那个具体根因 —— 它守的是**任何**索引定义变化下的启动路径行为。
    #[test]
    fn additive_only_drops_both_halves_of_a_paired_index_rebuild() {
        let mk = |cols: Vec<&str>| -> SchemaModel {
            let mut m = base(Dialect::Sqlite);
            m.table_mut("t1").unwrap().upsert_index(IndexModel {
                name: "idx_t1_name".into(),
                cols: cols.into_iter().map(String::from).collect(),
                unique: false,
                method: None,
                where_clause: None,
            });
            m.finalize();
            m
        };

        // 真差异（列序不同）⇒ plan 里必然是同名的 Drop + Create 一对
        let mut p = diff(&mk(vec!["name", "id"]), &mk(vec!["id", "name"]), Dialect::Sqlite);
        assert!(
            p.changes.iter().any(|c| c.kind == ChangeKind::DropIndex && c.object == "idx_t1_name"),
            "前提：应有一条同名的 DROP INDEX：{:?}",
            p.changes
        );
        assert!(
            p.changes
                .iter()
                .any(|c| c.kind == ChangeKind::CreateIndex && c.object == "idx_t1_name"),
            "前提：应有一条同名的 CREATE INDEX：{:?}",
            p.changes
        );

        p.retain_purely_additive();
        assert!(
            !p.changes.iter().any(|c| c.object == "idx_t1_name"),
            "成对重建必须**成对**收窄 —— 只留 CREATE 会撞「已存在」⇒ 启动中止：{:?}",
            p.changes
        );

        // ── 区分力：没有配对 DROP 的 CREATE 必须**留下**（否则就是把该做的事也砍了）──
        let mut orphan_create = mk(vec!["id", "name"]);
        orphan_create.table_mut("t1").unwrap().upsert_index(IndexModel {
            name: "idx_new_only".into(),
            cols: vec!["name".into()],
            unique: false,
            method: None,
            where_clause: None,
        });
        orphan_create.finalize();
        let mut p3 = diff(&orphan_create, &mk(vec!["id", "name"]), Dialect::Sqlite);
        assert!(
            p3.changes
                .iter()
                .any(|c| c.kind == ChangeKind::CreateIndex && c.object == "idx_new_only"),
            "前提：孤立新建应在 plan 里：{:?}",
            p3.changes
        );
        p3.retain_purely_additive();
        assert!(
            p3.changes
                .iter()
                .any(|c| c.kind == ChangeKind::CreateIndex && c.object == "idx_new_only"),
            "没有配对 DROP 的新建索引是**真纯新增**，不该被误砍：{:?}",
            p3.changes
        );
    }
}
