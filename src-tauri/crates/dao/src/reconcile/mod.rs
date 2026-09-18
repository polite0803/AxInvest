// SPDX-License-Identifier: AGPL-3.0-only

//! `reconcile` — 声明式 schema 同步引擎（PLAN-declarative-schema-sync）。
//!
//! ## 当前阶段：P5 接线中（引擎**已接入启动路径**，只做纯新增）
//!
//! 已有能力：
//!
//! | 阶段 | 产出 |
//! |---|---|
//! | P0–P3 | 读实况（[`introspect`]）· 算期望（[`expected`]）· 求差排序（[`plan`]）· 摘要指纹（[`fingerprint`]） |
//! | P4 | [`render`] 把 [`plan::Change`] 渲染成 DDL 文本 · [`safety`] 七道闸 + 元表簿记 · [`apply`] 执行编排 |
//! | P5（进行中） | [`apply::bootstrap_schema`] 接入 `db::create_pool`：启动期把库收敛到声明形态 |
//!
//! ⚠⚠ **本模块已具备改变库结构的能力** —— [`apply::cycle`] / [`apply::run`] 会真的执行
//! DDL。它「安全」的理由**不再是**「它没接线」（2026-09-16 P5 接线起这句话已不成立），
//! 而是四条：
//!
//! 1. **接线点唯一，且该点的配置被钉死**：启动路径只能经 [`apply::bootstrap_schema`]
//!    接触引擎，而它内部固定 `additive_only = true` + `dry_run = false` +
//!    `max_destructive_per_run = 0`（配置提到 [`apply::bootstrap_options`] 供测试直接
//!    断言）。任何人想绕过它直接构造 [`ApplyOptions`] 或调 `apply::cycle(` /
//!    `apply::run(`，会被守卫 `only_bootstrap_may_reach_apply_from_startup` 拦下。
//! 2. **收缩类变更永不在启动期执行**：判据 [`plan::ChangeKind::is_purely_additive`]
//!    的白名单 —— DROP / RENAME / `SET NOT NULL` 一律留给离线探针。
//! 3. **逃生阀默认开**：`AX_SCHEMA_DRY_RUN` 未设、为空、写错（`treu`）一律判为「开」
//!    ⇒ 只渲染不执行（仅约束探针路径；bootstrap 显式关掉它，理由见该函数文档）；
//! 4. **七道闸**：配额 · 基数熔断 · 墓碑证据 · 延迟一周期 · 白名单 · 审计 —— 见 [`safety`]。
//!
//! ⚠ 上面这段自我描述是**承重的**，且已经**栽过两次**：P4 之前这里写「全模块未执行过
//! 任何 DDL」，P4-4 把它变成假话；P5 又让「没有接线」这一条变成假话。
//! `examples/p3_plan_probe.rs` 的模块文档记着同一次教训 —— 把「安全」的证明写成
//! **「某个能力不存在」**，会随阶段推进**静默失真**；写成**「某个约束被机器检查」**
//! 才能活到下一阶段。故本次改写改用后者。
//!
//! ⚠ **`migrations/` 仍在，两套并存**，分工见 `db::create_pool` 的注释：
//! 迁移负责存量库的一次性**数据搬迁**（L3，实体声明表达不了），引擎负责 **schema 形态**
//! 收敛。迁移清单清空后，引擎成为唯一建表来源。
//!
//! ## L1 / L2 / L3 三层真相源
//!
//! | 层 | 载体 | 覆盖 |
//! |---|---|---|
//! | L1 | SeaORM 实体（`crates/entities/src/*.rs`） | 表 / 列 / 单列索引 / 单列 UNIQUE / FK |
//! | L2 | [`extras`]（本模块） | 多列索引 / partial / GIN / CHECK / 生成列 / FTS5 / 触发器 / 函数 |
//! | L3 | 程序式规则（P4 待实现） | 值域改名 / 数据搬迁 / 去重 |

pub mod apply;
pub mod evidence;
pub mod expected;
pub mod extras;
pub mod fingerprint;
pub mod introspect;
pub mod model;
pub mod plan;
pub mod render;
pub mod safety;
pub mod status;

pub use apply::{ApplyOptions, ApplyOutcome, ApplyRefusal, SkipReason};
pub use evidence::{ExportError, ExportOptions, ExportReport, ExportTarget};
pub use extras::Dialect;
// 只重导出**类型**，不重导出 `render::render` 函数 —— 那会让 `reconcile::render`
// 同时是模块名（类型命名空间）与函数名（值命名空间）：合法，但读起来像笔误。
pub use model::{
    CheckModel, ColumnModel, ENGINE_REV, FkModel, IndexModel, SchemaModel, TableModel,
};
pub use render::{RenderError, Rendered};
// 同 `render` 那条：只重导出**类型**。调用方写 `reconcile::status::probe(db)` ——
// 连 `probe` 一起重导出会让 `reconcile::status` 在模块命名空间与值命名空间各指一处。
pub use status::EngineStatus;

#[cfg(test)]
mod tests {
    /// 剥掉 Rust 源码里的**注释**（行注释 + 可嵌套的块注释），保留换行以便定位。
    ///
    /// 为什么必须剥：守卫要扫的是**代码**里出现的调用名，而注释里**经常**提到这些
    /// 名字（`examples/p3_plan_probe.rs` 的模块文档就写了 `reconcile::render`）。
    /// 不剥注释 ⇒ 守卫永远为红；若为迁就它把禁词写得含糊 ⇒ 守卫失去意义。
    /// 这是判据 K 组「自证注释污染」的可执行版本：**先剥注释，再下判据**。
    ///
    /// 只处理 `"` 字符串（不处理 `'`）：字符字面量与生命周期用 `'` 引入，把它们当
    /// 引号开始会把后续代码整段吞掉，反而制造假阴性。禁词都是标识符，`'x'` 之间
    /// 不可能夹带它们。
    fn strip_rust_comments(src: &str) -> String {
        let b = src.as_bytes();
        let mut out = String::with_capacity(src.len());
        let mut i = 0usize;
        let mut line_comment = false;
        let mut block_depth = 0usize;
        let mut in_str = false;

        while i < b.len() {
            let c = b[i];
            if line_comment {
                if c == b'\n' {
                    line_comment = false;
                    out.push('\n');
                }
                i += 1;
                continue;
            }
            if block_depth > 0 {
                if c == b'/' && b.get(i + 1) == Some(&b'*') {
                    block_depth += 1;
                    i += 2;
                    continue;
                }
                if c == b'*' && b.get(i + 1) == Some(&b'/') {
                    block_depth -= 1;
                    i += 2;
                    continue;
                }
                if c == b'\n' {
                    out.push('\n');
                }
                i += 1;
                continue;
            }
            if in_str {
                out.push(c as char);
                if c == b'\\' {
                    if let Some(n) = b.get(i + 1) {
                        out.push(*n as char);
                        i += 2;
                        continue;
                    }
                } else if c == b'"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            if c == b'/' && b.get(i + 1) == Some(&b'/') {
                line_comment = true;
                i += 2;
                continue;
            }
            if c == b'/' && b.get(i + 1) == Some(&b'*') {
                block_depth = 1;
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = true;
            }
            out.push(c as char);
            i += 1;
        }
        out
    }

    #[test]
    fn comment_stripper_handles_line_and_nested_block_comments() {
        assert_eq!(strip_rust_comments("a // b\nc"), "a \nc");
        assert_eq!(strip_rust_comments("a /* b */ c"), "a  c");
        assert_eq!(strip_rust_comments("a /* x /* y */ z */ c"), "a  c", "块注释可嵌套");
        assert_eq!(
            strip_rust_comments("let s = \"// not a comment\";"),
            "let s = \"// not a comment\";",
            "字符串里的 `//` 不是注释"
        );
        assert_eq!(strip_rust_comments("a // x\n/* y\nz */b"), "a \n\nb");
        assert_eq!(strip_rust_comments("let s = \"a\\\"b\";"), "let s = \"a\\\"b\";");
    }

    /// ⚠ **只读探针的常驻守卫**（P4-2 新增，替代原文那段「没有 render 模块」的自证）。
    ///
    /// 为什么不能再用「模块不存在」来证明只读：P4 给 `reconcile` 加了 `render`，
    /// 那句注释当场失真。**只依赖本文件自身的判据**才不会随相邻阶段腐烂。
    #[test]
    fn read_only_probe_cannot_execute_or_render() {
        const SRC: &str = include_str!("../../examples/p3_plan_probe.rs");
        const FORBIDDEN: &[&str] = &[
            "execute_unprepared(", // 唯一能执行任意 SQL 的入口
            "reconcile::render",   // 渲染入口
            "render::render",
            "safety::", // 写审计 / 墓碑的模块
            "apply(",   // 执行编排
        ];
        let code = strip_rust_comments(SRC);
        for f in FORBIDDEN {
            assert!(
                !code.contains(f),
                "只读探针的**代码**里出现了 `{f}` —— 它不再是只读的。\
                 （若只是文档里提到，检查 `strip_rust_comments` 是否失效）"
            );
        }
        // 反向断言：剥注释后必须仍看得见「读库」的证据 —— 否则说明扫描对象整体为空，
        // 上面那圈 `assert!(!contains)` 就是「对着空字符串下判据」（纪律 #8 的同族）。
        assert!(
            code.contains("introspect::read"),
            "扫描对象异常：剥注释后连 `introspect::read` 都找不到，说明剥过头了"
        );
    }

    /// 证明「剥注释」这一步是**承重的**，不是装饰：不剥就必然假红。
    #[test]
    fn comment_stripping_is_load_bearing_for_the_guard() {
        const SRC: &str = include_str!("../../examples/p3_plan_probe.rs");
        assert!(
            SRC.contains("reconcile::render"),
            "本测试的前提是探针的**注释**里确实提到过 `reconcile::render`；\
             若探针改得不再提到它，这条测试应当被删掉（而不是放宽）"
        );
        assert!(!strip_rust_comments(SRC).contains("reconcile::render"));
    }

    /// ⚠「**只有唯一闸门能接线**」—— 由「一律禁止」升级为「只许一道闸门，且禁止绕过」。
    ///
    /// 本守卫原为 `apply_is_not_wired_into_any_startup_path`（判据：启动路径**任何**
    /// 地方都不得出现 `reconcile::apply` 等 4 个名字）。2026-09-16 P5 接线时它按设计
    /// 变红 —— 它的旧注释预告过这一刻：「这条守卫将来会**阻碍** P5 的接线，那是**正确
    /// 的**：接线时必须同时改这里，而那正好是『有人认真决定把它接上启动路径』的
    /// 那一刻」。
    ///
    /// ## 改造成什么（**更强**，不是放宽）
    ///
    /// 旧判据是黑名单（4 个名字一个都不许出现），黑名单的固有缺陷是「新起一个入口名
    /// 就绕过去了」。新判据 = **白名单 + 反绕过**：
    ///
    /// * **白名单**：启动路径只允许经 `bootstrap_schema` 这一个出口接触引擎；
    /// * **反绕过**：`apply::cycle(` / `apply::run(` / 就地构造 `ApplyOptions` **一律
    ///   禁止** —— 想拿引擎的写能力，只能走那道闸门。
    ///
    /// 闸门自身的**配置**（`additive_only` / `dry_run` / 配额）不归本守卫管，由
    /// `apply::tests::bootstrap_options_are_pinned_to_the_safe_side` 直接断言。
    /// 两条测试分工明确：**本守卫管「谁在调」，那条管「调的时候是什么配置」。**
    ///
    /// 扫描面：`crates/dao/src`（除掉 `reconcile/` 自身）与 Tauri 应用的 `src/`
    /// —— 即「最可能接上启动路径」的两处。
    #[test]
    fn only_bootstrap_may_reach_apply_from_startup() {
        /// 允许的唯一出口。
        const ALLOWED: &str = "bootstrap_schema";
        /// 绕过闸门的写法：直接调编排、或在启动路径就地拼一个配置对象。
        ///
        /// ⚠ `ApplyOptions::default()` **不在**禁列 —— 它拿不到写能力（`dry_run` 默认
        /// 为 `true`），且 `bootstrap_options()` 自身就要用它。禁的是「就地拼配置」。
        const FORBIDDEN: &[&str] =
            &["apply::cycle(", "apply::run(", "ApplyOptions {", "ApplyOptions{"];

        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let roots = [here.join("src"), here.join("../../src")];

        let mut offenders: Vec<String> = Vec::new();
        let mut callers: Vec<String> = Vec::new();
        let mut scanned = 0usize;
        for root in &roots {
            if !root.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                let is_rs = path.is_file() && path.extension().is_some_and(|e| e == "rs");
                if !is_rs {
                    continue;
                }
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                // `reconcile/` 自身不算「接线」—— 定义端当然会出现这些名字。
                if rel.starts_with("reconcile/") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(path) else { continue };
                scanned += 1;
                let code = strip_rust_comments(&text); // 剥一次，全部禁词共用
                if let Some(hit) = FORBIDDEN.iter().find(|n| code.contains(*n)) {
                    offenders.push(format!("{rel}（命中禁词 `{hit}`）"));
                }
                if code.contains(ALLOWED) {
                    callers.push(rel);
                }
            }
        }
        // 反向断言 1：扫描面必须真的扫到东西 —— 否则「零命中」也可能只是根路径写错了
        // （纪律 #8：统计量恰为 0 / 恰 100% 时先怀疑测量工具，再怀疑被测对象）。
        assert!(scanned > 100, "扫描面异常：只扫到 {scanned} 个 .rs 文件，检查根路径");
        assert!(
            offenders.is_empty(),
            "启动路径绕过了唯一闸门 `{ALLOWED}`，直接拿引擎写能力：{offenders:?}\n\
             若确有新入口的需要，请先改 `bootstrap_options` 的固定配置并在此显式登记，\
             而不是在调用点就地拼一个配置对象（那会让「配置被钉死」这条保证失效）。"
        );
        // 反向断言 2：**白名单必须真的有人用**。若某次重构把接线撤掉却没同步文档，
        // 「已接入启动路径」就成了假话 —— 而这正是本模块文档栽过两次的形态
        // （P4 让「未执行过任何 DDL」失真，P5 让「没有接线」失真）。
        assert!(
            !callers.is_empty(),
            "没有任何文件调用 `{ALLOWED}`：模块文档声称「已接入启动路径」，\
             而代码里找不到接线点 —— 二者必有一个是假话。"
        );
    }
}
