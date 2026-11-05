// SPDX-License-Identifier: AGPL-3.0-only

//! 插件声明文件 `ui.json` 的**纯解析 + key 校验**（任务 `#149` 第 2 步）。
//!
//! ## 为什么是 `ui.json` 而不是 `schema.json`
//!
//! `schema.json` 已被插件**数据表**声明占用（`axagent-infra/crates/dao/src/reconcile/external.rs`
//! 的 `SCHEMA_FILE_NAME`）。同一目录下两个同名文件只会让「装表」与「装 UI」互相覆盖，
//! 故 UI 贡献单独一份 `ui.json`。
//!
//! ## 本模块只做三件事
//!
//! 1. 把 `ui.json` 的**未校验形态**解析成强类型声明（`Vec<UiSchemaDeclaration>`）；
//! 2. 校验 key 形状与文件内唯一性（**纯函数、零 I/O、零 DB**，故可单测）；
//! 3. 给出**宿主侧拼行 id 的唯一实现**（[`row_id`] / [`owner_prefix`]）—— 插件只声明
//!    key，绝不能自己拼 `plugin.<id>.<key>`：拼得出来就能引用别的插件的 schema。
//!
//! ## 归属锚点
//!
//! 行 id 恒为 `plugin.{pluginId}.{key}`，`origin = "plugin"`、`owner_id = pluginId`
//! 由**宿主**在落库时加盖（见 `src-tauri/src/init/plugin_ui_schema.rs`）。`ui.json`
//! 里**没有**任何字段能声明 origin / owner / 行 id —— 这是防冒充的结构性保证
//! （同 `create_dynamic_ui_schema` 拒收外部 `origin` 的口径）。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// 插件 UI 贡献的声明文件名（插件安装根下）。
pub const UI_SCHEMA_FILE_NAME: &str = "ui.json";

/// 当前支持的 `ui.json` 格式版本。**只认这一个值**：格式演进必须显式升版本，
/// 让旧宿主报「不支持」而不是按缺省字段猜。
pub const UI_SCHEMA_FORMAT_VERSION: u32 = 1;

/// 插件 schema 行的 id 前缀（`plugin`）。
pub const PLUGIN_SCHEMA_ID_PREFIX: &str = "plugin";

/// 插件 schema 行的 `origin` 值（宿主加盖，插件无法自报）。
pub const PLUGIN_SCHEMA_ORIGIN: &str = "plugin";

/// `ui.json` 顶层形态（未校验）。
///
/// `schemas` 缺省 = 空表：**显式空声明合法**（表示「本插件不贡献/已撤掉全部 UI」），
/// 宿主据此剪枝；文件**不存在**才是「无声明、不剪枝」（见 `reconcile_ui_schemas`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawUiSchemaFile {
    /// 格式版本；必须等于 [`UI_SCHEMA_FORMAT_VERSION`]。
    pub version: u32,
    #[serde(default)]
    pub schemas: Vec<RawUiSchemaEntry>,
}

/// `ui.json` 的单条 schema 声明（未校验）。
///
/// `schema` 是动态 UI 的 schema JSON（宿主原样序列化进 `dynamic_ui_schemas.schema_json`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawUiSchemaEntry {
    /// 插件内唯一的 key（同 [`is_declared_key_shape`] 的形状）。
    pub key: String,
    pub title: String,
    /// 分类；缺省 / 空串 ⇒ `custom`（与 `dynamic_ui_schemas.category` 的列默认值同口径）。
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// schema 自身的版本号（**不是** `ui.json` 的格式版本）。
    pub version: String,
    /// schema JSON 体；必须是 JSON object。
    pub schema: serde_json::Value,
}

/// `ui.json` 里的一条**已校验**声明（宿主落库直接消费）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSchemaDeclaration {
    pub key: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub version: String,
    /// `schema` 的 JSON 文本（确定性序列化：`serde_json` 缺省 `Map` 是 `BTreeMap`，
    /// 键序稳定 ⇒ 「内容是否变过」可以逐字比较，不需要结构 diff）。
    pub schema_json: String,
}

/// 声明 key 的形状判据 —— **同一套判据同时用于任务源 id 与 `ui.json` key**。
///
/// 规则：长度 ≤ 64，首字符是小写字母或数字，其余只允许小写字母、数字与 `-`。
/// 为什么必须这么紧：
/// 1. key 会进宿主行 id `plugin.{pluginId}.{key}`，带 `.` 会让「按前缀剪枝」把
///    另一条 id 当成自己的子命名空间；带 `:` / 空格会破坏前端路由与 `runIdOf` 拼接；
/// 2. 大写会让「同一 key 两种写法」在文件内去重失效（大小写敏感的文件系统与
///    大小写不敏感的数据库排序规则会给出相反结论）。
pub fn is_declared_key_shape(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {},
        _ => return false,
    }
    key.len() <= 64 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 插件 schema 行的**权威 id**：`plugin.{plugin_id}.{key}`。
///
/// 宿主与插件都只能经本函数拼 id（插件侧只声明 key，见模块文档）。
pub fn row_id(plugin_id: &str, key: &str) -> String {
    format!("{PLUGIN_SCHEMA_ID_PREFIX}.{plugin_id}.{key}")
}

/// 某插件 schema 行的 id 前缀（用于剪枝时做差集；含尾部 `.`，避免 `plugin.abc`
/// 误伤 `plugin.abcd.*`）。
pub fn owner_prefix(plugin_id: &str) -> String {
    format!("{PLUGIN_SCHEMA_ID_PREFIX}.{plugin_id}.")
}

/// 解析并校验 `ui.json` 全文。
///
/// 返回**已归一化**的声明表（顺序与文件一致）。错误信息按「第几条 / 哪个 key」定位，
/// 一次列出全部缺陷（与清单校验的聚合口径一致），不让用户改一条试一次。
pub fn parse(source: &str) -> Result<Vec<UiSchemaDeclaration>, String> {
    let raw: RawUiSchemaFile =
        serde_json::from_str(source).map_err(|e| format!("ui.json 解析失败：{e}"))?;
    if raw.version != UI_SCHEMA_FORMAT_VERSION {
        return Err(format!(
            "ui.json 的版本 {} 不受支持（本宿主只支持 {}）",
            raw.version, UI_SCHEMA_FORMAT_VERSION
        ));
    }

    let mut errors: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<UiSchemaDeclaration> = Vec::with_capacity(raw.schemas.len());

    for (index, entry) in raw.schemas.into_iter().enumerate() {
        let key = entry.key.trim().to_string();
        let at = format!("schemas[{index}]");
        if key.is_empty() {
            errors.push(format!("{at} 的 key 不能为空"));
            continue;
        }
        if !is_declared_key_shape(&key) {
            errors.push(format!(
                "{at} 的 key `{key}` 形状非法：只允许小写字母、数字与 `-`，首字符须是字母或数字，长度 ≤ 64"
            ));
            continue;
        }
        if !seen.insert(key.clone()) {
            errors.push(format!("{at} 的 key `{key}` 重复声明"));
            continue;
        }
        let title = entry.title.trim().to_string();
        if title.is_empty() {
            errors.push(format!("{at}（key `{key}`）的 title 不能为空"));
            continue;
        }
        let version = entry.version.trim().to_string();
        if !is_version_shape(&version) {
            errors.push(format!(
                "{at}（key `{key}`）的 version `{}` 非法：须是 `major.minor.patch` 形态的数字版本",
                entry.version
            ));
            continue;
        }
        if !entry.schema.is_object() {
            errors.push(format!("{at}（key `{key}`）的 schema 必须是 JSON object"));
            continue;
        }
        let category = entry
            .category
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "custom".to_string());
        // 空串 tag 只会让标签区出现空胶囊，按「未声明」丢弃（与 color / loading 的空白同判）。
        let tags: Vec<String> = entry
            .tags
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let schema_json = serde_json::to_string(&entry.schema)
            .map_err(|e| format!("{at} 的 schema 序列化失败：{e}"))?;
        out.push(UiSchemaDeclaration { key, title, category, tags, version, schema_json });
    }

    if !errors.is_empty() {
        return Err(errors.join("；"));
    }
    Ok(out)
}

/// schema 自身版本号的形状：点数 1–3 段、每段纯数字。
///
/// 宿主 `semver_gt` 把非法版本按 `(0,0,0)` 处理 ⇒ 若这里放行 `"v1"` / `"1.0.0-beta"`，
/// 「内容变了但版本没升」的判据会静默失效（任何变更都判成冲突、或都判成升级）。
fn is_version_shape(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (1..=3).contains(&parts.len())
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"{
        "version": 1,
        "schemas": [
            {
                "key": "order-form",
                "title": "Order Form",
                "tags": ["order", " "],
                "version": "1.2.3",
                "schema": { "type": "object", "fields": [] }
            }
        ]
    }"#;

    #[test]
    fn parses_minimal_declaration_and_normalizes_defaults() {
        let parsed = parse(OK).expect("合法声明应解析成功");
        assert_eq!(parsed.len(), 1);
        let entry = &parsed[0];
        assert_eq!(entry.key, "order-form");
        assert_eq!(entry.title, "Order Form");
        // 未声明 category ⇒ 列默认值 custom
        assert_eq!(entry.category, "custom");
        // 空白 tag 被丢弃
        assert_eq!(entry.tags, vec!["order".to_string()]);
        assert_eq!(entry.version, "1.2.3");
        assert!(
            entry.schema_json.starts_with('{'),
            "schema 应序列化成 JSON object：{}",
            entry.schema_json
        );
    }

    #[test]
    fn empty_schemas_array_is_a_valid_declaration() {
        let parsed = parse(r#"{ "version": 1, "schemas": [] }"#).expect("显式空声明合法");
        assert!(parsed.is_empty());
        // 缺省 schemas 同判空声明
        assert!(parse(r#"{ "version": 1 }"#).expect("缺省 schemas 合法").is_empty());
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let err = parse(r#"{ "version": 2, "schemas": [] }"#).expect_err("格式版本必须显式匹配");
        assert!(err.contains("不受支持"), "实际 = {err}");
    }

    #[test]
    fn rejects_bad_key_shapes_and_duplicates() {
        for key in ["Order", "order.form", "order form", "-order", "", "order_form"] {
            let source = format!(
                r#"{{ "version": 1, "schemas": [ {{ "key": "{key}", "title": "T", "version": "1.0.0", "schema": {{}} }} ] }}"#
            );
            let err = parse(&source).expect_err(&format!("key `{key}` 应被拒"));
            assert!(err.contains("key"), "错误应点出 key，实际 = {err}");
        }

        let dup = r#"{ "version": 1, "schemas": [
            { "key": "a", "title": "A", "version": "1.0.0", "schema": {} },
            { "key": "a", "title": "B", "version": "1.0.0", "schema": {} }
        ] }"#;
        let err = parse(dup).expect_err("同文件内 key 必须唯一");
        assert!(err.contains("重复"), "实际 = {err}");
    }

    #[test]
    fn rejects_empty_title_and_bad_schema_shapes() {
        let empty_title = r#"{ "version": 1, "schemas": [
            { "key": "a", "title": "  ", "version": "1.0.0", "schema": {} }
        ] }"#;
        assert!(parse(empty_title).expect_err("空 title 应被拒").contains("title"));

        let bad_schema = r#"{ "version": 1, "schemas": [
            { "key": "a", "title": "A", "version": "1.0.0", "schema": [] }
        ] }"#;
        assert!(parse(bad_schema).expect_err("schema 必须是 object").contains("JSON object"));
    }

    #[test]
    fn rejects_versions_that_break_host_semver_comparison() {
        for version in ["", "v1", "1.0.0-beta", "1..0", "1.0.0.0"] {
            let source = format!(
                r#"{{ "version": 1, "schemas": [ {{ "key": "a", "title": "A", "version": "{version}", "schema": {{}} }} ] }}"#
            );
            let err = parse(&source).expect_err(&format!("version `{version}` 应被拒"));
            assert!(err.contains("version"), "实际 = {err}");
        }
    }

    /// id 拼法必须是**唯一实现**：`plugin.{pluginId}.{key}`，前缀带尾点。
    #[test]
    fn row_id_and_prefix_are_single_sourced() {
        assert_eq!(row_id("demo", "order-form"), "plugin.demo.order-form");
        assert_eq!(owner_prefix("demo"), "plugin.demo.");
        // 前缀带尾点 ⇒ `plugin.abcd.x` 不会被 `plugin.abc.` 误判
        assert!(!row_id("abcd", "x").starts_with(&owner_prefix("abc")));
    }

    #[test]
    fn key_shape_matches_task_source_id_rules() {
        let longest = "x".repeat(64);
        let too_long = "x".repeat(65);
        for ok in ["a", "a1", "1a", "a-b", longest.as_str()] {
            assert!(is_declared_key_shape(ok), "`{ok}` 应合法");
        }
        for bad in ["", "A", "a.b", "a b", "-a", too_long.as_str()] {
            assert!(!is_declared_key_shape(bad), "`{bad}` 应非法");
        }
    }
}
