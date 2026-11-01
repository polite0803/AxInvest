// SPDX-License-Identifier: AGPL-3.0-only

//! `save_settings` 写路径回归：单事务删除 + upsert。
//!
//! 修复前：删除在事务外、insert 为纯插入，与另一路 `save_settings` 并发交错时
//! 会对同一主键重复 insert，报 `settings_pkey` 唯一约束冲突。修复后删除与写入
//! 同事务、insert 带 `on_conflict` upsert，主键冲突在结构上不可能发生。

use axagent_dao::db::create_test_pool;
use axagent_dao::repo::settings::{get_setting, save_settings, set_setting};
use axagent_harness::types::AppSettings;

#[tokio::test]
async fn save_twice_is_idempotent_and_overwrites() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let s = AppSettings { theme_mode: "dark".to_string(), ..Default::default() };
    save_settings(db, &s).await.expect("第一次保存应成功");

    let s2 = AppSettings { theme_mode: "light".to_string(), ..Default::default() };
    save_settings(db, &s2).await.expect("第二次保存应成功（重复保存不报唯一键冲突）");

    assert_eq!(
        get_setting(db, "themeMode").await.expect("读取应成功"),
        Some("light".to_string()),
        "同键重存应覆盖为新值"
    );
    assert_eq!(
        get_setting(db, "language").await.expect("读取应成功"),
        Some("zh-CN".to_string()),
        "常驻键应保留"
    );
}

#[tokio::test]
async fn save_removes_keys_absent_from_new_set() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let s = AppSettings { default_provider_id: Some("prov-1".to_string()), ..Default::default() };
    save_settings(db, &s).await.expect("第一次保存应成功");
    assert_eq!(
        get_setting(db, "defaultProviderId").await.expect("读取应成功"),
        Some("prov-1".to_string())
    );

    // 同键置 None（序列化为 null → 被跳过）后重存 ⇒ 该行应被清掉
    let s2 = AppSettings::default();
    save_settings(db, &s2).await.expect("第二次保存应成功");
    assert_eq!(
        get_setting(db, "defaultProviderId").await.expect("读取应成功"),
        None,
        "新集合中被跳过的键应随全量替换语义被删除"
    );
    assert_eq!(
        get_setting(db, "language").await.expect("读取应成功"),
        Some("zh-CN".to_string()),
        "常驻键应保留"
    );
}

#[tokio::test]
async fn save_clears_legacy_snake_case_keys() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    set_setting(db, "default_provider_id", "legacy").await.expect("预置 snake_case 旧键应成功");

    let s = AppSettings::default();
    save_settings(db, &s).await.expect("保存应成功");
    assert_eq!(
        get_setting(db, "default_provider_id").await.expect("读取应成功"),
        None,
        "snake_case 旧格式键应被清理"
    );
}

#[tokio::test]
async fn concurrent_saves_all_ok_and_state_consistent() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = h.conn.clone();

    // 多轮并发交错：每轮两路并发保存（themeMode 取值相反），全部应 Ok，
    // 终态每键取其中一路的值（后写者胜），不得出现唯一键冲突或键丢失。
    for i in 0..8 {
        let a = AppSettings { theme_mode: format!("dark-{i}"), ..Default::default() };
        let b = AppSettings { theme_mode: format!("light-{i}"), ..Default::default() };

        let db_a = db.clone();
        let db_b = db.clone();
        let (ra, rb) = tokio::join!(save_settings(&db_a, &a), save_settings(&db_b, &b));
        ra.expect("并发保存 A 应成功");
        rb.expect("并发保存 B 应成功");

        let theme = get_setting(&db, "themeMode").await.expect("读取应成功");
        assert!(
            theme == Some(format!("dark-{i}")) || theme == Some(format!("light-{i}")),
            "第 {i} 轮终态应收敛到两路之一的值，实际 {theme:?}"
        );
    }
}
