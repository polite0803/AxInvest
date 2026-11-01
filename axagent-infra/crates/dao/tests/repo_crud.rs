// SPDX-License-Identifier: AGPL-3.0-only

//! P0 数据安全底座仓库集成测试。
//!
//! 基于 `create_test_pool()` 的内存 SQLite 实例（自动建表 + 外键开启），
//! 覆盖 dao 高频仓库的核心 CRUD / 软删 / 外键 / 查询路径。
//! 不涉及任何外部资源，普通 `cargo test -p axagent-dao` 即可运行。

use axagent_dao::db::create_test_pool;
use axagent_dao::repo::conversation::create_conversation;
use axagent_dao::repo::credential_repo::{
    credential_exists, delete_credential, get_credential, insert_credential, list_credentials,
};
use axagent_dao::repo::memory::{
    add_item, create_namespace, delete_item, delete_namespace, get_item, get_namespace, list_items,
    list_namespaces,
};
use axagent_dao::repo::provider::{
    add_provider_key, create_provider, delete_provider_key, list_keys_for_provider,
};
use axagent_dao::repo::workflow_execution::{
    create_workflow_execution, get_workflow_execution, list_workflow_executions,
    update_workflow_execution_status,
};
use axagent_harness::types::{
    CreateMemoryItemInput, CreateMemoryNamespaceInput, CreateProviderInput, ProviderType,
};

#[tokio::test]
async fn conversation_crud_cycle() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    // 创建
    let conv = create_conversation(db, "测试会话", "model-1", "prov-1", Some("系统提示"))
        .await
        .expect("测试：数据库操作应成功");
    assert!(!conv.id.is_empty());
    assert_eq!(conv.title, "测试会话");
    assert_eq!(conv.model_id, "model-1");
    assert_eq!(conv.provider_id, "prov-1");
    assert_eq!(conv.system_prompt.as_deref(), Some("系统提示"));

    // 读取
    let fetched = axagent_dao::repo::conversation::get_conversation(db, &conv.id)
        .await
        .expect("测试：数据库操作应成功");
    assert_eq!(fetched.id, conv.id);

    // 列表非空
    let all = axagent_dao::repo::conversation::list_conversations(db)
        .await
        .expect("测试：数据库操作应成功");
    assert!(all.iter().any(|c| c.id == conv.id));

    // 删除后读取应失败
    axagent_dao::repo::conversation::delete_conversation(db, &conv.id)
        .await
        .expect("测试：数据库操作应成功");
    let res = axagent_dao::repo::conversation::get_conversation(db, &conv.id).await;
    assert!(res.is_err(), "删除后读取应返回错误");
}

#[tokio::test]
async fn credential_repo_roundtrip_and_soft_delete() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let row = insert_credential(db, "openai-key", "api_key", "enc:abcd1234")
        .await
        .expect("测试：数据库操作应成功");
    assert!(!row.id.is_empty());
    assert_eq!(row.name, "openai-key");
    assert_eq!(row.credential_type, "api_key");
    assert_eq!(row.data_encrypted, "enc:abcd1234");

    // 读取
    let got = get_credential(db, &row.id).await.expect("测试：数据库操作应成功");
    assert_eq!(got.data_encrypted, "enc:abcd1234");

    // 存在性
    assert!(credential_exists(db, &row.id).await.expect("测试：数据库操作应成功"));
    assert!(!credential_exists(db, "nope").await.expect("测试：数据库操作应成功"));

    // 列表包含该条
    let list = list_credentials(db).await.expect("测试：数据库操作应成功");
    assert!(list.iter().any(|c| c.id == row.id));

    // 删除后不存在
    delete_credential(db, &row.id).await.expect("测试：数据库操作应成功");
    assert!(!credential_exists(db, &row.id).await.expect("测试：数据库操作应成功"));
}

#[tokio::test]
async fn provider_key_rotation_and_delete() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    // provider_keys 有外键 REFERENCES providers(id)，需先建 provider
    let prov = create_provider(
        db,
        CreateProviderInput {
            name: "测试 Provider".into(),
            provider_type: ProviderType::OpenAI,
            api_host: "https://api.example.com".into(),
            api_path: None,
            enabled: true,
            builtin_id: None,
        },
    )
    .await
    .expect("测试应成功");

    let k1 =
        add_provider_key(db, &prov.id, "enc:key1", "sk-1").await.expect("测试：数据库操作应成功");
    let k2 =
        add_provider_key(db, &prov.id, "enc:key2", "sk-2").await.expect("测试：数据库操作应成功");

    // 轮换索引自增：第一条 0，第二条 1
    assert_eq!(k1.rotation_index, 0);
    assert_eq!(k2.rotation_index, 1);
    assert_eq!(k1.provider_id, prov.id);
    assert!(k1.enabled);

    // 按 provider 列出两条
    let keys = list_keys_for_provider(db, &prov.id).await.expect("测试：数据库操作应成功");
    assert_eq!(keys.len(), 2);

    // 删除一条后剩一条
    delete_provider_key(db, &k1.id).await.expect("测试：数据库操作应成功");
    let keys = list_keys_for_provider(db, &prov.id).await.expect("测试：数据库操作应成功");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].id, k2.id);
}

#[tokio::test]
async fn memory_namespace_and_item_lifecycle() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let ns = create_namespace(
        db,
        CreateMemoryNamespaceInput {
            name: "项目知识".into(),
            scope: "project".into(),
            embedding_provider: Some("emb-1".into()),
            embedding_dimensions: Some(1536),
            retrieval_threshold: Some(0.7),
            retrieval_top_k: Some(5),
            icon_type: None,
            icon_value: None,
        },
    )
    .await
    .expect("测试应成功");
    assert!(!ns.id.is_empty());
    assert_eq!(ns.name, "项目知识");
    assert_eq!(ns.scope, "project");

    // 命名空间可读 + 入列
    let fetched = get_namespace(db, &ns.id).await.expect("测试：数据库操作应成功");
    assert_eq!(fetched.id, ns.id);
    let list = list_namespaces(db).await.expect("测试：数据库操作应成功");
    assert!(list.iter().any(|n| n.id == ns.id));

    // 条目写入、读取、列表、删除
    let item = add_item(
        db,
        CreateMemoryItemInput {
            namespace_id: ns.id.clone(),
            title: "架构决策".into(),
            content: "采用 harness 解耦".into(),
            source: Some("manual".into()),
            tier: None,
            importance: None,
            memory_nature: None,
            tags: None,
            decay_rate: None,
            expires_at: None,
            applicability_tags: None,
            confirmed: None,
            source_conversation_id: None,
            source_message_id: None,
        },
    )
    .await
    .expect("测试应成功");
    assert_eq!(item.namespace_id, ns.id);
    assert_eq!(item.title, "架构决策");
    assert_eq!(item.content, "采用 harness 解耦");

    let items = list_items(db, &ns.id).await.expect("测试：数据库操作应成功");
    assert!(items.iter().any(|i| i.id == item.id));
    let got = get_item(db, &item.id).await.expect("测试：数据库操作应成功");
    assert_eq!(got.title, "架构决策");

    delete_item(db, &item.id).await.expect("测试：数据库操作应成功");
    let items = list_items(db, &ns.id).await.expect("测试：数据库操作应成功");
    assert!(!items.iter().any(|i| i.id == item.id));

    // 删除命名空间后列表不再包含
    delete_namespace(db, &ns.id).await.expect("测试：数据库操作应成功");
    let list = list_namespaces(db).await.expect("测试：数据库操作应成功");
    assert!(!list.iter().any(|n| n.id == ns.id));
}

#[tokio::test]
async fn workflow_execution_status_transition() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    create_workflow_execution(db, "exec-1", "wf-1", Some(r#"{"x":1}"#))
        .await
        .expect("测试：数据库操作应成功");

    // 初始为 running
    let exec = get_workflow_execution(db, "exec-1").await.expect("测试：数据库操作应成功");
    assert!(exec.is_some());
    assert_eq!(exec.expect("测试应成功").status, "running");

    // 更新为 success
    let updated = update_workflow_execution_status(
        db,
        "exec-1",
        "success",
        Some(r#"{"ok":true}"#),
        Some("[]"),
        Some(42),
        None,
    )
    .await
    .expect("测试应成功");
    assert!(updated);

    let exec = get_workflow_execution(db, "exec-1").await.expect("测试：数据库操作应成功").unwrap();
    assert_eq!(exec.status, "success");
    assert_eq!(exec.total_time_ms, Some(42));
    assert_eq!(exec.error_detail, None);

    // 失败原因要能落库读回：此前 failed 只留状态，细节只进日志，界面无从自查
    update_workflow_execution_status(
        db,
        "exec-1",
        "failed",
        None,
        None,
        None,
        Some("input_schema 校验失败：\"\" is required"),
    )
    .await
    .expect("测试应成功");
    let exec = get_workflow_execution(db, "exec-1").await.expect("测试：数据库操作应成功").unwrap();
    assert_eq!(exec.status, "failed");
    assert_eq!(exec.error_detail.as_deref(), Some("input_schema 校验失败：\"\" is required"));

    // 按 workflow 列出
    let list = list_workflow_executions(db, "wf-1").await.expect("测试：数据库操作应成功");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "exec-1");

    // 更新不存在的执行返回 false
    let none = update_workflow_execution_status(db, "exec-x", "failed", None, None, None, None)
        .await
        .expect("测试：数据库操作应成功");
    assert!(!none);
}

/// W3b（2026-11-01）：估值截面的读侧判据。
///
/// 「有行但 PE/PB 都是 NULL」不得进 map —— 覆盖率是价值桶唯一的可用性判据，把它算进分子
/// 会让 0 覆盖被判成「有估值」，于是桶内百分位是在一批 NULL 上排出来的。
/// 另一侧负控：跨日不得混入（同一轮的收盘历史与估值截面必须是同一个交易日）。
#[tokio::test]
async fn load_valuations_on_date_counts_only_rows_with_a_value() {
    use axagent_dao::repo::reco_seed::load_valuations_on_date;
    use axagent_entities::market_daily_close::{ActiveModel, Entity};
    use sea_orm::{ActiveValue::Set, EntityTrait};

    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;
    let row = |code: &str, date: &str, pe: Option<f64>, pb: Option<f64>| ActiveModel {
        id: Set(format!("{code}@{date}")),
        stock_code: Set(code.to_string()),
        stock_name: Set(format!("票{code}")),
        trade_date: Set(date.to_string()),
        close: Set(10.0),
        prev_close: Set(9.0),
        change_pct: Set(11.1),
        amount: Set(1.0e8),
        volume: Set(0.0),
        turnover_rate: Set(None),
        total_mv: Set(None),
        circulating_mv: Set(None),
        pe_ttm: Set(pe),
        pb: Set(pb),
        collected_at: Set(1),
        source: Set("tencent_batch".to_string()),
    };
    for am in [
        row("600001", "2026-11-02", Some(12.0), None),
        row("600002", "2026-11-02", None, Some(1.4)),
        row("600003", "2026-11-02", None, None),
        row("600004", "2026-11-03", Some(9.0), Some(9.0)),
    ] {
        Entity::insert(am).exec(db).await.expect("播种收盘行");
    }

    let got = load_valuations_on_date(db, "2026-11-02").await.expect("读估值截面");
    assert_eq!(got.len(), 2, "两列都 NULL 的行不算有估值: {got:?}");
    assert_eq!(got.get("600001"), Some(&(Some(12.0), None)), "只有一列也要算有估值");
    assert_eq!(got.get("600002"), Some(&(None, Some(1.4))));
    assert!(!got.contains_key("600003"), "全 NULL 要留在 map 外");
    assert!(!got.contains_key("600004"), "不得跨日取值");
}
