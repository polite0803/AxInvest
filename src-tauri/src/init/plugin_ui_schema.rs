// SPDX-License-Identifier: AGPL-3.0-only

//! 插件 `ui.json` → `dynamic_ui_schemas` 的**宿主侧**收敛（任务 `#149` 第 2 步）。
//!
//! 与 `init/plugin_schema.rs`（插件 `schema.json` → 数据表）同形、并列挂载：
//! 安装 / 启用 / 更新 / 启动各跑一次，**卸载不在这里**（卸载仍走
//! `commands/dynamic_ui.rs::revoke_schemas_owned_by`，按 `origin + owner_id` 撤销）。
//!
//! ## 防冒充的三条结构性保证
//!
//! 1. **签名里没有 origin / owner**：本模块的函数不接受任何「来源」入参，`origin = "plugin"`
//!    与 `owner_id = pluginId` 由宿主在落库时盖章；`pluginId` 只来自
//!    `PluginManager::plugin_registry_report()`（与 `init/plugin_schema.rs` 同法）。
//!    ⇒ 插件无法借 `ui.json` 自报归属，也无法给别人的 schema 改名。
//! 2. **key 只是 key**：行 id 由 [`axagent_plugins::ui_schema::row_id`] 拼成
//!    `plugin.{pluginId}.{key}`，插件写不出「别的插件的 id」。
//! 3. **同 id 冲突绝不覆盖**：已存在的行若 `origin != "plugin"` 或 `owner_id != pluginId`
//!    （例如用户恰好建了同 id 的行、或另一个插件的旧行），安装当场失败并报出实际归属。
//!
//! ## 幂等与版本口径
//!
//! - 同 owner + 同 id 且内容/版本都没变 ⇒ **no-op**（安装 / 启用 / 启动反复跑不产生写）；
//! - 内容变了但 `version` 没升 ⇒ 安装失败（对齐 `update_dynamic_ui_schema` 的
//!   「版本号冲突」口径，见 `commands/dynamic_ui.rs`）；
//! - `ui.json` **不存在**或**解析失败** ⇒ 本轮跳过，**禁止剪枝** —— 一次瞬时 IO 错误
//!   不该把用户库里该插件的 UI 全删掉（同 `init/plugin_schema.rs` 对 `failed` 插件的跳过纪律）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use axagent_entities::dynamic_ui_form_data::{Column as FormDataColumn, Entity as FormDataEntity};
use axagent_entities::dynamic_ui_schema_versions::{
    Column as VersionColumn, Entity as VersionEntity,
};
use axagent_entities::dynamic_ui_schemas::{
    ActiveModel as SchemaActiveModel, Column as SchemaColumn, Entity as SchemaEntity,
    Model as SchemaModel,
};
use axagent_plugins::PluginManager;
use axagent_plugins::ui_schema::{
    self, PLUGIN_SCHEMA_ORIGIN, UI_SCHEMA_FILE_NAME, UiSchemaDeclaration,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::sync::RwLock;

use crate::commands::dynamic_ui::{create_version_snapshot, semver_gt};

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 把**已安装**插件的 `ui.json` 收敛进 `dynamic_ui_schemas`；`only` 用于安装 / 启用 /
/// 更新后只跑该插件。
///
/// 返回实际写入（新增 + 更新）的行数；`0` 表示全部幂等命中。剪枝条数只记日志
/// （它是「升级后少了哪些 UI」的事实，不参与调用方的成功判定）。
///
/// 失败语义分两种：
/// - `only = Some(_)`（安装 / 启用 / 更新路径）⇒ 首个错误直接返回，调用方把它交给用户；
/// - `only = None`（启动路径）⇒ 逐插件记录 `error!` 后继续，一个坏声明不能挡住 App 启动
///   与其余插件的 UI（同 `reconcile_installed` 在启动点的处置，但更宽容：那里是整体失败）。
pub async fn reconcile_ui_schemas(
    db: &DatabaseConnection,
    plugin_manager: &RwLock<PluginManager>,
    only: Option<&str>,
) -> Result<usize, String> {
    let (roots, failed_plugins) = {
        let manager = plugin_manager.read().await;
        let report =
            manager.plugin_registry_report().map_err(|e| format!("读取插件清单失败：{e}"))?;
        let failures = report.failures().len();
        let roots: Vec<(String, PathBuf)> = report
            .summaries()
            .into_iter()
            .filter(|s| only.is_none_or(|id| s.metadata.id == id))
            .filter_map(|s| s.metadata.root.map(|root| (s.metadata.id, root)))
            .collect();
        (roots, failures)
    };
    if failed_plugins > 0 {
        tracing::warn!(
            failed = failed_plugins,
            "插件 ui.json 收敛：有 {failed_plugins} 个插件装载失败，已跳过（其声明不参与本轮）"
        );
    }

    let mut written = 0usize;
    for (plugin_id, root) in &roots {
        match reconcile_one(db, plugin_id, root).await {
            Ok(count) => written += count,
            Err(error) => {
                if only.is_some() {
                    return Err(error);
                }
                tracing::error!(plugin_id = %plugin_id, "插件 ui.json 收敛失败（不阻塞启动）：{error}");
            },
        }
    }
    Ok(written)
}

/// 单个插件的收敛：读 `ui.json` → 逐条 upsert → 剪枝差集。
async fn reconcile_one(
    db: &DatabaseConnection,
    plugin_id: &str,
    root: &Path,
) -> Result<usize, String> {
    let path = root.join(UI_SCHEMA_FILE_NAME);
    let source = match tokio::fs::read_to_string(&path).await {
        Ok(source) => source,
        // 没有声明文件 = 没有 UI 贡献。**不剪枝**：文件缺失可能是升级过程中的瞬态。
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(format!("插件 `{plugin_id}` 的 {} 读取失败：{error}", path.display()));
        },
    };
    // 解析失败同样不剪枝：坏声明是「这一轮不生效」，不是「这些 schema 都不要了」。
    let declarations = ui_schema::parse(&source)
        .map_err(|e| format!("插件 `{plugin_id}` 的 {} 非法：{e}", path.display()))?;

    let prefix = ui_schema::owner_prefix(plugin_id);
    // 只取「本插件拥有」的行：`origin` 与 `owner_id` 两个条件都要 —— 只有 id 前缀相同
    // 的行可能是历史遗留（例如手改过 origin），按 owner 过滤才不会误删。
    let mut owned: BTreeMap<String, SchemaModel> = SchemaEntity::find()
        .filter(SchemaColumn::Origin.eq(PLUGIN_SCHEMA_ORIGIN))
        .filter(SchemaColumn::OwnerId.eq(plugin_id))
        .all(db)
        .await
        .map_err(|e| format!("查询插件 `{plugin_id}` 的 UI Schema 失败：{e}"))?
        .into_iter()
        .map(|model| (model.id.clone(), model))
        .collect();

    let mut written = 0usize;
    for declaration in &declarations {
        let id = ui_schema::row_id(plugin_id, &declaration.key);
        // 已存在的行从 `owned` 里摘掉：剩下没被摘的就是「本次声明里已删除」的差集。
        owned.remove(&id);
        written += upsert_declaration(db, plugin_id, declaration).await?;
    }

    let mut pruned = 0usize;
    for (id, _) in owned {
        // 命名空间之外的行（owner 是它、但 id 不是 `plugin.{pluginId}.` 形态）不动：
        // 剪枝判据必须同时满足「owner + id 前缀」，否则会顺手删掉历史行。
        if !id.starts_with(&prefix) {
            tracing::warn!(plugin_id = %plugin_id, schema_id = %id, "UI Schema 归属该插件但不在命名空间内，跳过剪枝");
            continue;
        }
        delete_row_cascade(db, &id).await?;
        pruned += 1;
    }
    if pruned > 0 {
        tracing::info!(plugin_id = %plugin_id, pruned, "已剪除插件 ui.json 不再声明的 DynamicUI Schema");
    }
    Ok(written)
}

/// 单条声明的 upsert（幂等 + 版本冲突 + 防覆盖）。
///
/// 返回 `1`（新增或更新）或 `0`（内容与版本都没变）。
async fn upsert_declaration(
    db: &DatabaseConnection,
    plugin_id: &str,
    declaration: &UiSchemaDeclaration,
) -> Result<usize, String> {
    let id = ui_schema::row_id(plugin_id, &declaration.key);
    let tags_json = serde_json::to_string(&declaration.tags).unwrap_or_else(|_| "[]".to_string());

    let existing = SchemaEntity::find_by_id(id.clone())
        .one(db)
        .await
        .map_err(|e| format!("查询 DynamicUI Schema `{id}` 失败：{e}"))?;
    if let Some(model) = existing {
        return apply_existing(db, plugin_id, declaration, &tags_json, model).await;
    }

    let now = now_iso();
    let active = SchemaActiveModel {
        id: Set(id.clone()),
        title: Set(declaration.title.clone()),
        description: Set(String::new()),
        schema_json: Set(declaration.schema_json.clone()),
        category: Set(declaration.category.clone()),
        tags: Set(tags_json.clone()),
        version: Set(declaration.version.clone()),
        is_builtin: Set(0),
        origin: Set(PLUGIN_SCHEMA_ORIGIN.to_string()),
        owner_id: Set(plugin_id.to_string()),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    };
    match active.insert(db).await {
        Ok(model) => {
            create_version_snapshot(db, &model.id, &model.version, &model, "插件 ui.json 安装写入")
                .await?;
            Ok(1)
        },
        Err(error) => {
            // 并发 reconcile（安装与启动并发、或两次启用）会撞 PK：重读一次按「已存在」处置。
            // 重读不到 ⇒ 回到原始错误（不是冲突，是真实写失败）。
            match SchemaEntity::find_by_id(id.clone()).one(db).await {
                Ok(Some(model)) => {
                    tracing::debug!(schema_id = %id, "并发插入撞 PK，改按已存在处置");
                    apply_existing(db, plugin_id, declaration, &tags_json, model).await
                },
                _ => {
                    Err(format!("写入插件 `{plugin_id}` 的 DynamicUI Schema `{id}` 失败：{error}"))
                },
            }
        },
    }
}

/// 已存在行的处置：防覆盖 → 幂等判定 → 版本冲突判定 → 更新。
async fn apply_existing(
    db: &DatabaseConnection,
    plugin_id: &str,
    declaration: &UiSchemaDeclaration,
    tags_json: &str,
    model: SchemaModel,
) -> Result<usize, String> {
    let id = model.id.clone();
    // 防冒充的最后一跳：id 相同但归属不是本插件 ⇒ 装不上，**绝不覆盖**。
    if model.origin != PLUGIN_SCHEMA_ORIGIN || model.owner_id != plugin_id {
        return Err(format!(
            "拒绝覆盖：DynamicUI Schema `{id}` 已存在（origin = \"{}\"，owner_id = \"{}\"），不属于插件 `{plugin_id}`",
            model.origin, model.owner_id
        ));
    }

    let unchanged = model.version == declaration.version
        && model.schema_json == declaration.schema_json
        && model.title == declaration.title
        && model.category == declaration.category
        && model.tags == tags_json;
    if unchanged {
        return Ok(0);
    }

    let content_changed = model.schema_json != declaration.schema_json;
    if content_changed && !semver_gt(&declaration.version, &model.version) {
        return Err(format!(
            "插件 `{plugin_id}` 的 ui.json 里 `{}` 内容变了但版本号未升（库中 {}，声明 {}）；内容变更必须升版本",
            declaration.key, model.version, declaration.version
        ));
    }
    if content_changed {
        // 与用户侧更新路径同口径：改内容前先把旧版本落一份快照，版本历史才连得上。
        create_version_snapshot(db, &id, &model.version, &model, "插件 ui.json 更新").await?;
    }

    let mut active: SchemaActiveModel = model.into();
    active.title = Set(declaration.title.clone());
    active.category = Set(declaration.category.clone());
    active.tags = Set(tags_json.to_string());
    active.schema_json = Set(declaration.schema_json.clone());
    active.version = Set(declaration.version.clone());
    active.updated_at = Set(now_iso());
    active.update(db).await.map_err(|e| format!("更新 DynamicUI Schema `{id}` 失败：{e}"))?;
    Ok(1)
}

/// 级联删除一行 schema（表单数据 + 版本历史 + 本体）。
///
/// 与 `revoke_schemas_owned_by` 同口径：级联顺序固定，避免与用户侧删除路径分叉出
/// 第二套真相源（两侧都删同样的三张表，只是触发点不同）。
async fn delete_row_cascade(db: &DatabaseConnection, id: &str) -> Result<(), String> {
    FormDataEntity::delete_many()
        .filter(FormDataColumn::SchemaId.eq(id))
        .exec(db)
        .await
        .map_err(|e| format!("删除 DynamicUI Schema `{id}` 的表单数据失败：{e}"))?;
    VersionEntity::delete_many()
        .filter(VersionColumn::SchemaId.eq(id))
        .exec(db)
        .await
        .map_err(|e| format!("删除 DynamicUI Schema `{id}` 的版本历史失败：{e}"))?;
    SchemaEntity::delete_by_id(id.to_string())
        .exec(db)
        .await
        .map_err(|e| format!("删除 DynamicUI Schema `{id}` 失败：{e}"))?;
    Ok(())
}

/// 运行期复核：`plugin_list_task_sources` 的 `detailSchema` 必须指向**本插件在库里真实存在**
/// 的那一行（`plugin.{pluginId}.{key}`，`origin = "plugin"`）。
///
/// 为什么安装期校验还不够：`ui.json` 可能被删、库可能被换、行可能被后续的 reconcile 剪掉 ——
/// 安装期通过不代表今天还在。复核不过就**置 `None`**（详情槽退化成「无详情」），
/// 而不是把 key 原样交给前端去加载一张不存在/别人的表。
pub async fn retain_owned_detail_schemas(
    db: &DatabaseConnection,
    infos: &mut [axagent_plugins::PluginTaskSourcesInfo],
) -> Result<(), String> {
    let owned: BTreeSet<String> = SchemaEntity::find()
        .filter(SchemaColumn::Origin.eq(PLUGIN_SCHEMA_ORIGIN))
        .all(db)
        .await
        .map_err(|e| format!("查询插件 UI Schema 失败：{e}"))?
        .into_iter()
        .map(|model| model.id)
        .collect();

    for info in infos.iter_mut() {
        for source in info.task_sources.iter_mut() {
            let Some(key) = source.detail_schema.as_deref() else { continue };
            let expected = ui_schema::row_id(&info.id, key);
            if !owned.contains(&expected) {
                tracing::warn!(
                    plugin_id = %info.id,
                    source = %source.id,
                    key = %key,
                    "任务源 detailSchema 指向的插件 schema 不存在（库中查无此行），详情槽已关闭"
                );
                source.detail_schema = None;
            }
        }
    }
    Ok(())
}
