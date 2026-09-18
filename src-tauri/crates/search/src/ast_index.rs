// SPDX-License-Identifier: AGPL-3.0-only

//! AST-based structured code index for semantic code search.
//!
//! Extracts function signatures, class definitions, interface declarations,
//! variable declarations, and call relationships from source code using
//! pattern-based parsing (upgradeable to tree-sitter).
//!
//! Stores extracted definitions in SQLite for fast semantic matching during
//! the L2 phase of the three-level recall pipeline.
//!
//! # 访问层（2026-09-16 改造：原生 SQL → SeaORM 实体）
//!
//! 原先持 `rusqlite::Connection` + 手写 DDL + 原生 SQL；现改持 `DatabaseConnection`
//! 并全部走 `axagent_entities::ast_*` 五个实体：
//! - 建表由实体派生（`Schema::create_table_from_entity`）；
//! - `unchecked_transaction()` → `TransactionTrait::transaction`（真事务，失败回滚）；
//! - 单文件删除 / 前缀删除改为实体 `delete_many`，**前缀比较在 Rust 侧做字面
//!   `starts_with`**（理由与 `file_index.rs` 相同：`LIKE` 的 `_` 是通配符，而扫描根
//!   必定含 `_`；`substr` 需写裸 SQL，会把方言引回来）；
//! - `remove_file` 原先是 `execute_batch(&format!("... WHERE file_path = '{0}'"))`
//!   —— **字符串拼 SQL**（只做了 `'` 转义）。现为参数化实体删除，该注入面消失。
//!
//! ⚠ `ast_call_edges` 的主键是实体侧**新增**的四列复合主键（原手写 DDL 无主键），
//! 因此插入配 `OnConflict::do_nothing()`：完全重复的边本身无意义。
//! 存量库不会因 `CREATE TABLE IF NOT EXISTS` 补上该约束（该库是可重建缓存，
//! 处置方式是删库重扫，见实体文件说明）。
//!
//! ## 同步 → async
//!
//! 方法全部改为 `async`。`DatabaseConnection` 是 `Send + Sync + Clone`，从而解除了
//! `rusqlite::Connection`（`Send` 但非 `Sync`）带来的「访问必须收进 `spawn_blocking`
//! 且不得跨 `.await` 共享」约束（原裁定见 `PLAN-weknora-borrowings.md §12.11.2`）。

use axagent_entities::{ast_call_edges, ast_classes, ast_functions, ast_interfaces, ast_variables};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QuerySelect, Schema, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub id: String,
    pub file_path: String,
    pub name: String,
    pub signature: String,
    pub line_start: usize,
    pub line_end: usize,
    pub visibility: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassDef {
    pub id: String,
    pub file_path: String,
    pub name: String,
    pub line_start: usize,
    pub line_end: usize,
    pub language: String,
    pub parent_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceDef {
    pub id: String,
    pub file_path: String,
    pub name: String,
    pub line_start: usize,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableDecl {
    pub id: String,
    pub file_path: String,
    pub name: String,
    pub type_annotation: Option<String>,
    pub line: usize,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller_file: String,
    pub caller_function: String,
    pub callee_name: String,
    pub line: usize,
}

pub struct AstIndex {
    pub(crate) db: DatabaseConnection,
}

impl std::fmt::Debug for AstIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AstIndex").finish_non_exhaustive()
    }
}

impl AstIndex {
    pub async fn new(db: DatabaseConnection) -> Result<Self, String> {
        let index = Self { db };
        index.ensure_tables().await?;
        Ok(index)
    }

    /// 5 张表的建表语句**由实体生成**（不再手写列清单）。
    async fn ensure_tables(&self) -> Result<(), String> {
        let backend = self.db.get_database_backend();
        for stmt in [
            Schema::new(backend).create_table_from_entity(ast_functions::Entity),
            Schema::new(backend).create_table_from_entity(ast_classes::Entity),
            Schema::new(backend).create_table_from_entity(ast_interfaces::Entity),
            Schema::new(backend).create_table_from_entity(ast_variables::Entity),
            Schema::new(backend).create_table_from_entity(ast_call_edges::Entity),
        ] {
            let mut stmt = stmt;
            stmt.if_not_exists();
            self.db
                .execute(&stmt)
                .await
                .map_err(|e| format!("Failed to create AST tables: {e}"))?;
        }
        Ok(())
    }

    /// Index a file's AST, replacing previous entries for this file.
    ///
    /// 全程在**一个事务**内：先按单文件删 5 张表，再批量插回。失败回滚 ⇒
    /// 不会出现「旧的删了、新的没插上」这种索引空洞。
    pub async fn index_file(&self, file_path: &str, content: &str) -> Result<usize, String> {
        let lang = detect_language(file_path);
        let functions = extract_functions(content, file_path, lang);
        let classes = extract_classes(content, file_path, lang);
        let interfaces = extract_interfaces(content, file_path, lang);
        let variables = extract_variables(content, file_path, lang);
        let call_edges = extract_call_edges(content, file_path, &functions);

        let total =
            functions.len() + classes.len() + interfaces.len() + variables.len() + call_edges.len();
        let fp = file_path.to_string();

        self.db
            .transaction::<_, (), sea_orm::DbErr>(move |txn| {
                Box::pin(async move {
                    ast_functions::Entity::delete_many()
                        .filter(ast_functions::Column::FilePath.eq(&fp))
                        .exec(txn)
                        .await?;
                    ast_classes::Entity::delete_many()
                        .filter(ast_classes::Column::FilePath.eq(&fp))
                        .exec(txn)
                        .await?;
                    ast_interfaces::Entity::delete_many()
                        .filter(ast_interfaces::Column::FilePath.eq(&fp))
                        .exec(txn)
                        .await?;
                    ast_variables::Entity::delete_many()
                        .filter(ast_variables::Column::FilePath.eq(&fp))
                        .exec(txn)
                        .await?;
                    ast_call_edges::Entity::delete_many()
                        .filter(ast_call_edges::Column::CallerFile.eq(&fp))
                        .exec(txn)
                        .await?;

                    if !functions.is_empty() {
                        let models: Vec<ast_functions::ActiveModel> = functions
                            .iter()
                            .map(|f| ast_functions::ActiveModel {
                                id: Set(f.id.clone()),
                                file_path: Set(f.file_path.clone()),
                                name: Set(f.name.clone()),
                                signature: Set(f.signature.clone()),
                                line_start: Set(f.line_start as i32),
                                line_end: Set(f.line_end as i32),
                                visibility: Set(f.visibility.clone()),
                                language: Set(f.language.clone()),
                            })
                            .collect();
                        ast_functions::Entity::insert_many(models)
                            .on_conflict(
                                OnConflict::column(ast_functions::Column::Id)
                                    .do_nothing()
                                    .to_owned(),
                            )
                            .exec(txn)
                            .await?;
                    }

                    if !classes.is_empty() {
                        let models: Vec<ast_classes::ActiveModel> = classes
                            .iter()
                            .map(|c| ast_classes::ActiveModel {
                                id: Set(c.id.clone()),
                                file_path: Set(c.file_path.clone()),
                                name: Set(c.name.clone()),
                                line_start: Set(c.line_start as i32),
                                line_end: Set(c.line_end as i32),
                                language: Set(c.language.clone()),
                                parent_class: Set(c.parent_class.clone()),
                            })
                            .collect();
                        ast_classes::Entity::insert_many(models)
                            .on_conflict(
                                OnConflict::column(ast_classes::Column::Id).do_nothing().to_owned(),
                            )
                            .exec(txn)
                            .await?;
                    }

                    if !interfaces.is_empty() {
                        let models: Vec<ast_interfaces::ActiveModel> = interfaces
                            .iter()
                            .map(|i| ast_interfaces::ActiveModel {
                                id: Set(i.id.clone()),
                                file_path: Set(i.file_path.clone()),
                                name: Set(i.name.clone()),
                                line_start: Set(i.line_start as i32),
                                language: Set(i.language.clone()),
                            })
                            .collect();
                        ast_interfaces::Entity::insert_many(models)
                            .on_conflict(
                                OnConflict::column(ast_interfaces::Column::Id)
                                    .do_nothing()
                                    .to_owned(),
                            )
                            .exec(txn)
                            .await?;
                    }

                    if !variables.is_empty() {
                        let models: Vec<ast_variables::ActiveModel> = variables
                            .iter()
                            .map(|v| ast_variables::ActiveModel {
                                id: Set(v.id.clone()),
                                file_path: Set(v.file_path.clone()),
                                name: Set(v.name.clone()),
                                type_annotation: Set(v.type_annotation.clone()),
                                line: Set(v.line as i32),
                                language: Set(v.language.clone()),
                            })
                            .collect();
                        ast_variables::Entity::insert_many(models)
                            .on_conflict(
                                OnConflict::column(ast_variables::Column::Id)
                                    .do_nothing()
                                    .to_owned(),
                            )
                            .exec(txn)
                            .await?;
                    }

                    if !call_edges.is_empty() {
                        let models: Vec<ast_call_edges::ActiveModel> = call_edges
                            .iter()
                            .map(|e| ast_call_edges::ActiveModel {
                                caller_file: Set(e.caller_file.clone()),
                                caller_function: Set(e.caller_function.clone()),
                                callee_name: Set(e.callee_name.clone()),
                                line: Set(e.line as i32),
                            })
                            .collect();
                        ast_call_edges::Entity::insert_many(models)
                            .on_conflict(
                                OnConflict::columns([
                                    ast_call_edges::Column::CallerFile,
                                    ast_call_edges::Column::CallerFunction,
                                    ast_call_edges::Column::CalleeName,
                                    ast_call_edges::Column::Line,
                                ])
                                .do_nothing()
                                .to_owned(),
                            )
                            .exec(txn)
                            .await?;
                    }

                    Ok(())
                })
            })
            .await
            .map_err(|e| format!("index_file: {e}"))?;

        Ok(total)
    }

    /// Remove all AST entries for a given file.
    ///
    /// 改造前是 `execute_batch(&format!(...))` 字符串拼 SQL；现为参数化实体删除。
    pub async fn remove_file(&self, file_path: &str) -> Result<(), String> {
        let fp = file_path.to_string();
        ast_functions::Entity::delete_many()
            .filter(ast_functions::Column::FilePath.eq(&fp))
            .exec(&self.db)
            .await
            .map_err(|e| format!("remove_file (ast_functions): {e}"))?;
        ast_classes::Entity::delete_many()
            .filter(ast_classes::Column::FilePath.eq(&fp))
            .exec(&self.db)
            .await
            .map_err(|e| format!("remove_file (ast_classes): {e}"))?;
        ast_interfaces::Entity::delete_many()
            .filter(ast_interfaces::Column::FilePath.eq(&fp))
            .exec(&self.db)
            .await
            .map_err(|e| format!("remove_file (ast_interfaces): {e}"))?;
        ast_variables::Entity::delete_many()
            .filter(ast_variables::Column::FilePath.eq(&fp))
            .exec(&self.db)
            .await
            .map_err(|e| format!("remove_file (ast_variables): {e}"))?;
        ast_call_edges::Entity::delete_many()
            .filter(ast_call_edges::Column::CallerFile.eq(&fp))
            .exec(&self.db)
            .await
            .map_err(|e| format!("remove_file (ast_call_edges): {e}"))?;
        Ok(())
    }

    /// Remove all AST entries whose `file_path` starts with the given prefix.
    ///
    /// 与 `FileIndex::remove_by_prefix` 对称（前缀在 Rust 侧做**字面**
    /// `starts_with` 比较，不用 `LIKE` —— 扫描根由 `WorkspaceUri::cache_path`
    /// 生成，必定含 `_`；也不用 `substr` 裸 SQL，避免方言回归）。
    ///
    /// **为什么持久化索引必须调它**：[`Self::index_file`] 只按**单文件**
    /// `DELETE`，对「文件已从磁盘删除或改名」无能为力；索引落盘后若不做
    /// 「root 切片全量替换」，磁盘上已不存在的文件会留下幽灵定义。
    pub async fn remove_by_prefix(&self, prefix: &str) -> Result<usize, String> {
        let mut total = 0usize;

        // 4 张表按 `file_path`
        let paths: Vec<String> = ast_functions::Entity::find()
            .select_only()
            .column(ast_functions::Column::FilePath)
            .distinct()
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("load ast_functions paths: {e}"))?;
        let hit: Vec<String> = paths.into_iter().filter(|p| p.starts_with(prefix)).collect();
        if !hit.is_empty() {
            let r = ast_functions::Entity::delete_many()
                .filter(ast_functions::Column::FilePath.is_in(hit))
                .exec(&self.db)
                .await
                .map_err(|e| format!("remove prefix {prefix} (ast_functions): {e}"))?;
            total += r.rows_affected as usize;
        }

        let paths: Vec<String> = ast_classes::Entity::find()
            .select_only()
            .column(ast_classes::Column::FilePath)
            .distinct()
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("load ast_classes paths: {e}"))?;
        let hit: Vec<String> = paths.into_iter().filter(|p| p.starts_with(prefix)).collect();
        if !hit.is_empty() {
            let r = ast_classes::Entity::delete_many()
                .filter(ast_classes::Column::FilePath.is_in(hit))
                .exec(&self.db)
                .await
                .map_err(|e| format!("remove prefix {prefix} (ast_classes): {e}"))?;
            total += r.rows_affected as usize;
        }

        let paths: Vec<String> = ast_interfaces::Entity::find()
            .select_only()
            .column(ast_interfaces::Column::FilePath)
            .distinct()
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("load ast_interfaces paths: {e}"))?;
        let hit: Vec<String> = paths.into_iter().filter(|p| p.starts_with(prefix)).collect();
        if !hit.is_empty() {
            let r = ast_interfaces::Entity::delete_many()
                .filter(ast_interfaces::Column::FilePath.is_in(hit))
                .exec(&self.db)
                .await
                .map_err(|e| format!("remove prefix {prefix} (ast_interfaces): {e}"))?;
            total += r.rows_affected as usize;
        }

        let paths: Vec<String> = ast_variables::Entity::find()
            .select_only()
            .column(ast_variables::Column::FilePath)
            .distinct()
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("load ast_variables paths: {e}"))?;
        let hit: Vec<String> = paths.into_iter().filter(|p| p.starts_with(prefix)).collect();
        if !hit.is_empty() {
            let r = ast_variables::Entity::delete_many()
                .filter(ast_variables::Column::FilePath.is_in(hit))
                .exec(&self.db)
                .await
                .map_err(|e| format!("remove prefix {prefix} (ast_variables): {e}"))?;
            total += r.rows_affected as usize;
        }

        // 边表按 `caller_file`
        let paths: Vec<String> = ast_call_edges::Entity::find()
            .select_only()
            .column(ast_call_edges::Column::CallerFile)
            .distinct()
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("load ast_call_edges paths: {e}"))?;
        let hit: Vec<String> = paths.into_iter().filter(|p| p.starts_with(prefix)).collect();
        if !hit.is_empty() {
            let r = ast_call_edges::Entity::delete_many()
                .filter(ast_call_edges::Column::CallerFile.is_in(hit))
                .exec(&self.db)
                .await
                .map_err(|e| format!("remove prefix {prefix} (ast_call_edges): {e}"))?;
            total += r.rows_affected as usize;
        }

        Ok(total)
    }

    /// Search functions by name (partial match).
    pub async fn search_functions(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FunctionDef>, String> {
        let pattern = format!("%{query}%");
        let rows = ast_functions::Entity::find()
            .filter(
                Condition::any()
                    .add(ast_functions::Column::Name.like(&pattern))
                    .add(ast_functions::Column::Signature.like(&pattern)),
            )
            .limit(limit as u64)
            .all(&self.db)
            .await
            .map_err(|e| format!("search_functions: {e}"))?;

        Ok(rows
            .into_iter()
            .map(|f| FunctionDef {
                id: f.id,
                file_path: f.file_path,
                name: f.name,
                signature: f.signature,
                line_start: f.line_start.max(0) as usize,
                line_end: f.line_end.max(0) as usize,
                visibility: f.visibility,
                language: f.language,
            })
            .collect())
    }

    /// Search classes by name.
    pub async fn search_classes(&self, query: &str, limit: usize) -> Result<Vec<ClassDef>, String> {
        let pattern = format!("%{query}%");
        let rows = ast_classes::Entity::find()
            .filter(ast_classes::Column::Name.like(&pattern))
            .limit(limit as u64)
            .all(&self.db)
            .await
            .map_err(|e| format!("search_classes: {e}"))?;

        Ok(rows
            .into_iter()
            .map(|c| ClassDef {
                id: c.id,
                file_path: c.file_path,
                name: c.name,
                line_start: c.line_start.max(0) as usize,
                line_end: c.line_end.max(0) as usize,
                language: c.language,
                parent_class: c.parent_class,
            })
            .collect())
    }

    /// Find callers of a function.
    pub async fn find_callers(&self, function_name: &str) -> Result<Vec<CallEdge>, String> {
        let rows = ast_call_edges::Entity::find()
            .filter(ast_call_edges::Column::CalleeName.eq(function_name))
            .all(&self.db)
            .await
            .map_err(|e| format!("find_callers: {e}"))?;

        Ok(rows
            .into_iter()
            .map(|e| CallEdge {
                caller_file: e.caller_file,
                caller_function: e.caller_function,
                callee_name: e.callee_name,
                line: e.line.max(0) as usize,
            })
            .collect())
    }

    /// Search for matching definitions across all types.
    ///
    /// Returns file paths that contain definitions matching the query.
    pub async fn search_all(&self, query: &str, limit: usize) -> Result<Vec<String>, String> {
        let pattern = format!("%{query}%");
        let mut results = std::collections::HashSet::new();

        let paths: Vec<String> = ast_functions::Entity::find()
            .select_only()
            .column(ast_functions::Column::FilePath)
            .filter(ast_functions::Column::Name.like(&pattern))
            .limit(limit as u64)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("search_all (ast_functions): {e}"))?;
        results.extend(paths);

        let paths: Vec<String> = ast_classes::Entity::find()
            .select_only()
            .column(ast_classes::Column::FilePath)
            .filter(ast_classes::Column::Name.like(&pattern))
            .limit(limit as u64)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("search_all (ast_classes): {e}"))?;
        results.extend(paths);

        let paths: Vec<String> = ast_interfaces::Entity::find()
            .select_only()
            .column(ast_interfaces::Column::FilePath)
            .filter(ast_interfaces::Column::Name.like(&pattern))
            .limit(limit as u64)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("search_all (ast_interfaces): {e}"))?;
        results.extend(paths);

        let paths: Vec<String> = ast_variables::Entity::find()
            .select_only()
            .column(ast_variables::Column::FilePath)
            .filter(ast_variables::Column::Name.like(&pattern))
            .limit(limit as u64)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("search_all (ast_variables): {e}"))?;
        results.extend(paths);

        let mut sorted: Vec<String> = results.into_iter().collect();
        sorted.sort();
        sorted.truncate(limit);
        Ok(sorted)
    }

    /// Get the total count of indexed definitions.
    ///
    /// ⚠ 改造前这里对两次 COUNT 都用了 `.unwrap_or(0)` —— 计数失败会被静默当成 0
    /// （「0 个定义」与「查不出来」不可区分）。现改为向上传递错误。
    pub async fn total_definitions(&self) -> Result<usize, String> {
        let fn_count = ast_functions::Entity::find()
            .count(&self.db)
            .await
            .map_err(|e| format!("count ast_functions: {e}"))?;
        let cls_count = ast_classes::Entity::find()
            .count(&self.db)
            .await
            .map_err(|e| format!("count ast_classes: {e}"))?;
        Ok((fn_count + cls_count) as usize)
    }
}

fn detect_language(file_path: &str) -> &str {
    let lower = file_path.to_lowercase();
    if lower.ends_with(".rs") {
        "rust"
    } else if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        "typescript"
    } else if lower.ends_with(".js") || lower.ends_with(".jsx") {
        "javascript"
    } else if lower.ends_with(".py") {
        "python"
    } else if lower.ends_with(".go") {
        "go"
    } else if lower.ends_with(".java") {
        "java"
    } else if lower.ends_with(".cpp") || lower.ends_with(".cc") || lower.ends_with(".cxx") {
        "cpp"
    } else if lower.ends_with(".c") || lower.ends_with(".h") {
        "c"
    } else {
        "unknown"
    }
}

fn extract_functions(content: &str, file_path: &str, lang: &str) -> Vec<FunctionDef> {
    let mut functions = Vec::new();
    let mut func_id = 0;

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let vis = detect_visibility(trimmed);

        let fn_name = match lang {
            "rust" => extract_rust_fn_name(trimmed),
            "python" => extract_python_fn_name(trimmed),
            "typescript" | "javascript" => extract_ts_fn_name(trimmed),
            "go" => extract_go_fn_name(trimmed),
            _ => None,
        };

        if let Some(name) = fn_name {
            if is_common_keyword(&name) {
                continue;
            }
            func_id += 1;
            let sig = trimmed.chars().take(200).collect::<String>();
            let end_line = find_block_end(content, line_num);
            functions.push(FunctionDef {
                id: format!("{}_{}", file_path.replace(['/', '\\'], "_"), func_id),
                file_path: file_path.to_string(),
                name,
                signature: sig,
                line_start: line_num + 1,
                line_end: end_line + 1,
                visibility: vis.to_string(),
                language: lang.to_string(),
            });
        }
    }
    functions
}

fn extract_classes(content: &str, file_path: &str, lang: &str) -> Vec<ClassDef> {
    let mut classes = Vec::new();
    let mut cls_id = 0;

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let cls_name = match lang {
            "rust" => extract_rust_struct_enum_name(trimmed),
            "python" => extract_python_class_name(trimmed),
            "typescript" | "javascript" => extract_ts_class_name(trimmed),
            "go" => extract_go_type_name(trimmed),
            _ => None,
        };

        if let Some(name) = cls_name {
            cls_id += 1;
            let end_line = find_block_end(content, line_num);
            classes.push(ClassDef {
                id: format!("{}_cls_{}", file_path.replace(['/', '\\'], "_"), cls_id),
                file_path: file_path.to_string(),
                name,
                line_start: line_num + 1,
                line_end: end_line + 1,
                language: lang.to_string(),
                parent_class: None,
            });
        }
    }
    classes
}

fn extract_interfaces(_content: &str, _file_path: &str, _lang: &str) -> Vec<InterfaceDef> {
    Vec::new()
}

fn extract_variables(content: &str, file_path: &str, lang: &str) -> Vec<VariableDecl> {
    let mut vars = Vec::new();
    let mut var_id = 0;

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let var_info = extract_variable_declaration(trimmed, lang);
        if let Some((name, type_ann)) = var_info {
            if is_common_keyword(&name) || name.len() < 2 {
                continue;
            }
            var_id += 1;
            vars.push(VariableDecl {
                id: format!("{}_var_{}", file_path.replace(['/', '\\'], "_"), var_id),
                file_path: file_path.to_string(),
                name,
                type_annotation: type_ann,
                line: line_num + 1,
                language: lang.to_string(),
            });
        }
    }
    vars
}

fn extract_call_edges(content: &str, file_path: &str, functions: &[FunctionDef]) -> Vec<CallEdge> {
    let mut edges = Vec::new();
    let fn_names: std::collections::HashSet<&str> =
        functions.iter().map(|f| f.name.as_str()).collect();

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("--") {
            continue;
        }

        for fn_name in &fn_names {
            if trimmed.contains(*fn_name)
                && !trimmed.contains(&format!("fn {}", fn_name))
                && !trimmed.contains(&format!("def {}", fn_name))
                && !trimmed.contains(&format!("function {}", fn_name))
            {
                let caller = functions
                    .iter()
                    .find(|f| f.line_start <= line_num + 1 && f.line_end > line_num);
                if let Some(caller_fn) = caller {
                    edges.push(CallEdge {
                        caller_file: file_path.to_string(),
                        caller_function: caller_fn.name.clone(),
                        callee_name: fn_name.to_string(),
                        line: line_num + 1,
                    });
                }
            }
        }
    }
    edges
}

// ── Language-specific extractors ──────────────────────────────────────────

fn detect_visibility(line: &str) -> &str {
    if line.starts_with("pub ") || line.starts_with("pub(") {
        "public"
    } else if line.starts_with("pub(crate) ") {
        "crate"
    } else {
        "private"
    }
}

fn extract_rust_fn_name(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("fn ")
        && !line.starts_with("pub fn ")
        && !line.starts_with("pub(crate) fn ")
        && !line.starts_with("async fn ")
        && !line.starts_with("pub async fn ")
    {
        return None;
    }
    let after_fn = line.split("fn ").nth(1)?;
    let name = after_fn.split(['(', '<']).next()?.trim();
    if name.is_empty() || name == "fn" {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_rust_struct_enum_name(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with("struct ") || line.starts_with("pub struct ") {
        let after = line.split("struct ").nth(1)?;
        let name = after.split(['<', '{', '(', ';']).next()?.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else if line.starts_with("enum ") || line.starts_with("pub enum ") {
        let after = line.split("enum ").nth(1)?;
        let name = after.split(['<', '{']).next()?.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else if line.starts_with("trait ") || line.starts_with("pub trait ") {
        let after = line.split("trait ").nth(1)?;
        let name = after.split(['<', '{']).next()?.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else if line.starts_with("impl ") {
        let after = line.trim_start_matches("impl ");
        let name = after.split(['<', ' ', '{']).next()?.trim();
        if name == "for" {
            after.split("for ").nth(1)?.split(['<', '{']).next().map(|n| n.trim().to_string())
        } else if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else {
        None
    }
}

fn extract_python_fn_name(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("def ") && !line.starts_with("async def ") {
        return None;
    }
    let after = if line.starts_with("async def ") {
        line.split("async def ").nth(1)?
    } else {
        line.split("def ").nth(1)?
    };
    let name = after.split(['(', ':']).next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_python_class_name(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("class ") {
        return None;
    }
    let after = line.strip_prefix("class ")?;
    let name = after.split(['(', ':']).next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_ts_fn_name(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.contains("function ") && !line.contains("=>") {
        return None;
    }
    if line.starts_with("function ") {
        let after = line.strip_prefix("function ")?;
        let name = after.split(['(', '<']).next()?.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else if line.starts_with("export function ") {
        let after = line.strip_prefix("export function ")?;
        let name = after.split(['(', '<']).next()?.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else if line.starts_with("export const ") || line.starts_with("const ") {
        let after = if line.starts_with("export const ") {
            line.strip_prefix("export const ")?
        } else {
            line.strip_prefix("const ")?
        };
        let name = after.split(['=', ':', '(']).next()?.trim();
        if name.contains("=>") || name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else {
        None
    }
}

fn extract_ts_class_name(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with("class ") || line.starts_with("export class ") {
        let after = if line.starts_with("export class ") {
            line.strip_prefix("export class ")?
        } else {
            line.strip_prefix("class ")?
        };
        let name = after.split(['<', '{', ' ', ':']).next()?.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else {
        None
    }
}

fn extract_go_fn_name(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("func ") {
        return None;
    }
    let after = line.strip_prefix("func ")?;
    let rest = if after.starts_with('(') {
        after.split(')').nth(1)?.trim_start()
    } else {
        after
    };
    let name = rest.split(['(', '<']).next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_go_type_name(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("type ") {
        return None;
    }
    let after = line.strip_prefix("type ")?;
    let name = after.split([' ', '[', '{']).next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_variable_declaration(line: &str, lang: &str) -> Option<(String, Option<String>)> {
    let line = line.trim();
    match lang {
        "rust" => {
            if line.starts_with("let ") && !line.contains('=') {
                return None;
            }
            if line.starts_with("let mut ") || line.starts_with("let ") {
                let after = line.strip_prefix("let mut ").or_else(|| line.strip_prefix("let "))?;
                let name = after.split(['=', ':', ' ']).next()?.trim();
                let type_ann = after
                    .split(':')
                    .nth(1)
                    .and_then(|t| t.split('=').next())
                    .map(|t| t.trim().to_string());
                Some((name.to_string(), type_ann))
            } else {
                None
            }
        },
        "typescript" | "javascript" => {
            if line.starts_with("let ") || line.starts_with("var ") || line.starts_with("const ") {
                let after = line
                    .trim_start_matches("export ")
                    .trim_start_matches("let ")
                    .trim_start_matches("var ")
                    .trim_start_matches("const ");
                let name = after.split(['=', ':', ' ']).next()?.trim();
                if name.is_empty() || name == "{" {
                    return None;
                }
                let type_ann = after
                    .split(':')
                    .nth(1)
                    .and_then(|t| t.split('=').next())
                    .map(|t| t.trim().to_string());
                Some((name.to_string(), type_ann))
            } else {
                None
            }
        },
        "python" => {
            if line.contains('=')
                && !line.starts_with("if ")
                && !line.starts_with("for ")
                && !line.starts_with("while ")
                && !line.starts_with("def ")
                && !line.starts_with("class ")
            {
                let name = line.split('=').next()?.trim();
                if name.is_empty() || name.contains(' ') {
                    return None;
                }
                Some((name.to_string(), None))
            } else {
                None
            }
        },
        _ => None,
    }
}

fn is_common_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "for"
            | "while"
            | "match"
            | "switch"
            | "case"
            | "return"
            | "break"
            | "continue"
            | "true"
            | "false"
            | "None"
            | "Some"
            | "Ok"
            | "Err"
            | "self"
            | "Self"
            | "super"
            | "this"
            | "new"
            | "use"
            | "mod"
            | "crate"
            | "pub"
            | "async"
            | "await"
            | "let"
            | "const"
            | "var"
            | "import"
            | "export"
            | "from"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "yield"
            | "with"
            | "type"
            | "interface"
            | "enum"
            | "struct"
            | "impl"
            | "trait"
            | "fn"
            | "def"
            | "class"
            | "function"
            | "static"
            | "public"
            | "private"
            | "protected"
            | "final"
            | "abstract"
            | "override"
    )
}

fn find_block_end(content: &str, start: usize) -> usize {
    let mut depth: i32 = 0;
    let mut started = false;
    for (i, line) in content.lines().enumerate() {
        if i < start {
            continue;
        }
        let trimmed = line.trim();
        let opens = trimmed.matches('{').count() as i32 + trimmed.matches("do").count() as i32;
        let closes = trimmed.matches('}').count() as i32;

        if i == start {
            if trimmed.ends_with('{') || trimmed.ends_with(':') {
                started = true;
            } else if !trimmed.contains('{') && !trimmed.ends_with(':') {
                return i; // Single-line definition
            }
        }

        if started {
            depth += opens - closes;
            if depth <= 0 && i > start {
                return i;
            }
        }
    }
    content.lines().count().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};

    /// ⚠ `sqlite::memory:` 必须 `max_connections(1)`：sqlx 每条池连接各持一份
    /// 独立内存库，多连接下建表与写入会落到不同库（表现为「表不存在」）。
    async fn test_index() -> AstIndex {
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1).sqlx_logging(false);
        let db = Database::connect(opt).await.expect("测试：打开内存数据库应成功");
        AstIndex::new(db).await.expect("测试应成功")
    }

    #[tokio::test]
    async fn test_extract_rust_functions() {
        let code = "fn main() {\n    println!(\"hello\");\n}\n\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let idx = test_index().await;
        idx.index_file("/test.rs", code).await.expect("测试：index_file 应成功");
        let results = idx.search_functions("add", 10).await.expect("测试：search_functions 应成功");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "add");
        assert!(results[0].signature.contains("pub fn add"));
    }

    #[tokio::test]
    async fn test_extract_rust_structs() {
        let code = "pub struct User {\n    name: String,\n}\n\nenum Color { Red, Blue }\n";
        let idx = test_index().await;
        idx.index_file("/test.rs", code).await.expect("测试：index_file 应成功");
        let results = idx.search_classes("User", 10).await.expect("测试：search_classes 应成功");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "User");

        let all = idx.search_classes("Color", 10).await.expect("测试：search_classes 应成功");
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_search_all() {
        let code = "fn calculate() -> u32 { 42 }\nfn render() {}\nstruct Widget {}\n";
        let idx = test_index().await;
        idx.index_file("/test.rs", code).await.expect("测试：index_file 应成功");
        let results = idx.search_all("calc", 10).await.expect("测试：search_all 应成功");
        assert!(results.contains(&"/test.rs".to_string()));
    }

    #[tokio::test]
    async fn test_extract_python() {
        let code = "def hello():\n    print('hi')\n\nclass MyClass:\n    pass\n";
        let idx = test_index().await;
        idx.index_file("/test.py", code).await.expect("测试：index_file 应成功");
        let fns = idx.search_functions("hello", 10).await.expect("测试：search_functions 应成功");
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "hello");
        let cls = idx.search_classes("MyClass", 10).await.expect("测试：search_classes 应成功");
        assert_eq!(cls[0].name, "MyClass");
    }

    /// 回归锁：重复索引同一文件**不得**累积行（`index_file` 先删后插）。
    #[tokio::test]
    async fn test_index_file_is_idempotent() {
        let code = "pub fn alpha() {}\npub fn beta() {}\n";
        let idx = test_index().await;
        for _ in 0..3 {
            idx.index_file("/t.rs", code).await.expect("测试：index_file 应成功");
        }
        assert_eq!(idx.total_definitions().await.expect("测试：计数应成功"), 2);
        let fns = idx.search_functions("a", 50).await.expect("测试：search 应成功");
        assert_eq!(fns.len(), 2, "重复索引后不应出现重复函数行");
    }

    #[tokio::test]
    async fn test_remove_file() {
        let idx = test_index().await;
        idx.index_file("/a.rs", "pub fn one() {}\n").await.expect("测试：index 应成功");
        idx.index_file("/b.rs", "pub fn two() {}\n").await.expect("测试：index 应成功");
        assert_eq!(idx.total_definitions().await.expect("测试：计数应成功"), 2);

        idx.remove_file("/a.rs").await.expect("测试：remove_file 应成功");
        assert_eq!(idx.total_definitions().await.expect("测试：计数应成功"), 1);
        let left = idx.search_all("one", 10).await.expect("测试：search_all 应成功");
        assert!(left.is_empty(), "被删文件的定义不应再被搜到");
    }

    /// 回归锁：前缀必须是**字面**比较（扫描根 `<authority>_<md5>` 必定含 `_`）。
    /// 若实现改用 `LIKE`，`_` 会匹配任意字符 ⇒ `/cacheXa1` 会被误删。
    #[tokio::test]
    async fn test_remove_by_prefix_is_literal_not_like() {
        let idx = test_index().await;
        idx.index_file("/cache_a1/keep.rs", "pub fn kept() {}\n")
            .await
            .expect("测试：index 应成功");
        idx.index_file("/cacheXa1/sibling.rs", "pub fn sib() {}\n")
            .await
            .expect("测试：index 应成功");

        let removed = idx.remove_by_prefix("/cache_a1").await.expect("测试：前缀删除应成功");
        assert_eq!(removed, 1, "只应删掉字面命中那条");

        let left = idx.search_all("sib", 10).await.expect("测试：search_all 应成功");
        assert_eq!(left, vec!["/cacheXa1/sibling.rs".to_string()], "兄弟目录不得被误删");
    }

    #[tokio::test]
    async fn test_find_callers() {
        let idx = test_index().await;
        let code = "fn helper() {}\nfn caller() {\n    helper();\n}\n";
        idx.index_file("/c.rs", code).await.expect("测试：index 应成功");
        let callers = idx.find_callers("helper").await.expect("测试：find_callers 应成功");
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].caller_function, "caller");
        assert_eq!(callers[0].caller_file, "/c.rs");
    }
}
