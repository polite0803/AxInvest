// SPDX-License-Identifier: AGPL-3.0-only

//! 记忆向量索引「孤儿条目」兜底推进的回归测试。
//!
//! 背景（生产实证 2026-09-12，PG）：`index_jobs` 中 `container_type='mem'` 与
//! `job_type='index_memory'` 的记录数**均为 0** —— 记忆向量化从未入队过一次；
//! 同时 `memory_items` 有 2 条 `source='reflector'` 的条目自 09-06 起持续
//! `pending`（`index_error` 为空），`vec_collections` 里也不存在任何 `mem_*` 集合。
//!
//! 根因不是「入队代码写错了」，而是**结构性的**：`index_status` 由 DAO 层在插入时
//! 写成 `pending`，而「入队」是命令层（唯一拿得到 AppHandle 的层）的独立动作。
//! `trajectory::storage::save_memory`、`agent::reflector::persist_insight`、
//! `agent::project_memory`、`tools::agent_memory` 这些写入路径都在命令层之下，
//! **结构上不可能**自行入队 —— 于是每新增一个写入点就多一批永久 pending。
//!
//! 因此兜底做在队列侧：`sweep_pending_memory_items` 周期扫描 `index_status='pending'`
//! 且无活跃作业的条目，按「provider 是否可用」分流为入队 / 跳过 / 终态失败。
//! 本文件锁住这套分流、它的幂等性，以及它对确定性配置错误的处置。
//!
//! 基于 `create_test_pool()` 的内存 SQLite（自动跑迁移 + 外键开启），
//! 普通 `cargo test -p axagent-dao` 即可运行。

use axagent_dao::db::create_test_pool;
use axagent_dao::repo::index_jobs::{
    self as jobs, CONTAINER_TYPE_MEM, CONTAINER_TYPE_MEM_ALIAS, CreateIndexJobInput,
    JOB_TYPE_INDEX_MEMORY, PendingMemoryRepair,
};
use axagent_dao::repo::memory::{add_item, create_namespace, get_item, update_item_index_status};
use axagent_dao::repo::provider::create_provider;
use axagent_harness::constants::status;
use axagent_harness::types::{
    CreateMemoryItemInput, CreateMemoryNamespaceInput, CreateProviderInput, ProviderType,
};
use sea_orm::DatabaseConnection;

/// 建一个可被解析的 provider，返回其 id。
///
/// 必须真的落库：扫描器会校验命名空间引用的 provider id 是否还存在，
/// 用假 id 会让「应当入队」的用例变成「悬空引用 ⇒ 终态失败」。
async fn make_provider(db: &DatabaseConnection, name: &str) -> String {
    create_provider(
        db,
        CreateProviderInput {
            name: name.to_string(),
            provider_type: ProviderType::OpenAI,
            api_host: "http://127.0.0.1:9".to_string(),
            api_path: None,
            enabled: true,
            builtin_id: None,
        },
    )
    .await
    .expect("测试：创建 provider 应成功")
    .id
}

/// 建一个命名空间。`embedding_provider` 传 `None` 表示未配置。
async fn make_namespace(
    db: &DatabaseConnection,
    name: &str,
    embedding_provider: Option<String>,
) -> String {
    create_namespace(
        db,
        CreateMemoryNamespaceInput {
            name: name.to_string(),
            scope: "global".to_string(),
            embedding_provider,
            embedding_dimensions: None,
            retrieval_threshold: None,
            retrieval_top_k: None,
            icon_type: None,
            icon_value: None,
        },
    )
    .await
    .expect("测试：创建命名空间应成功")
    .id
}

/// 建一个命名空间，其 embedding_provider 指向一个真实存在的 provider。
async fn make_namespace_with_live_provider(db: &DatabaseConnection, name: &str) -> String {
    let provider_id = make_provider(db, "测试 embedding provider").await;
    make_namespace(db, name, Some(format!("{provider_id}::bge-m3.gguf"))).await
}

/// 建一条记忆条目。`add_item` 落库时 `index_status` 固定为 `pending`
/// —— 这正是「状态说有活要干、但没人排活」的初始形态。
async fn make_pending_item(db: &DatabaseConnection, namespace_id: &str, title: &str) -> String {
    let item = add_item(
        db,
        CreateMemoryItemInput {
            namespace_id: namespace_id.to_string(),
            title: title.to_string(),
            content: format!("{} 的正文内容", title),
            source: Some("reflector".to_string()),
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
    .expect("测试：创建记忆条目应成功");
    assert_eq!(
        item.index_status,
        status::PENDING,
        "前置条件：新条目必须落在 pending —— 否则本文件的被测场景不成立",
    );
    item.id
}

async fn all_jobs(db: &DatabaseConnection) -> Vec<jobs::IndexJob> {
    jobs::list_all_jobs(db, 100, 0).await.expect("测试：列举作业应成功").0
}

/// 场景 1：provider 可用、无活跃作业的 pending 条目 ⇒ 补入队。
///
/// 这是生产里那 2 条 reflector 记忆的形态（差别只在它们的 provider 已失效，
/// 见场景 3b）。
#[tokio::test]
async fn sweep_enqueues_orphan_pending_item() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let ns = make_namespace_with_live_provider(db, "Reflector Insights").await;
    let item_id = make_pending_item(db, &ns, "瓶颈掘金").await;

    // 前置：队列里确实一条作业都没有（孤儿成立）
    assert!(all_jobs(db).await.is_empty(), "前置条件：队列应为空");

    let report = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：孤儿扫描应成功");

    assert_eq!(report.scanned, 1, "应扫到 1 条 pending 条目");
    assert_eq!(report.enqueued.len(), 1, "应补入 1 个索引作业");
    assert_eq!(report.already_queued, 0);
    assert_eq!(report.marked_skipped, 0);
    assert_eq!(report.marked_failed, 0);

    let (reported_item, reported_ns, job_id) = &report.enqueued[0];
    assert_eq!(reported_item, &item_id);
    assert_eq!(reported_ns, &ns);

    // 作业本身必须可被队列消费：job_type / container_type / 两个 id 缺一不可。
    // container_type 必须是权威写法，否则 run_indexing 的 match 会落到 Err 分支。
    let job = jobs::get_job(db, job_id).await.expect("测试：应能取回新建作业");
    assert_eq!(job.job_type, JOB_TYPE_INDEX_MEMORY);
    assert_eq!(job.container_type, CONTAINER_TYPE_MEM);
    assert_eq!(job.container_id, ns, "container_id 必须是命名空间 id（load_container 用它取 ns）");
    assert_eq!(job.item_id, item_id);
    assert_eq!(job.status, jobs::INDEX_JOB_STATUS_PENDING);

    // 条目状态保持 pending：它确实在等队列干活，此处不该被改写。
    let item = get_item(db, &item_id).await.expect("测试：应能取回条目");
    assert_eq!(item.index_status, status::PENDING);
    assert!(item.index_error.is_none());
}

/// 场景 2：同一批数据反复扫描不得重复入队。
#[tokio::test]
async fn sweep_is_idempotent() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let ns = make_namespace_with_live_provider(db, "Reflector Insights").await;
    let item_id = make_pending_item(db, &ns, "瓶颈掘金").await;

    let first = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：首轮扫描应成功");
    assert_eq!(first.enqueued.len(), 1);

    let second = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：次轮扫描应成功");
    assert_eq!(second.enqueued.len(), 0, "第二轮不得再补入队（否则会重复付 embedding 成本）");
    assert_eq!(second.already_queued, 1);

    assert_eq!(all_jobs(db).await.len(), 1, "队列里应恰好只有 1 个作业");
    assert_eq!(
        jobs::get_active_job_for_item(db, CONTAINER_TYPE_MEM, &item_id)
            .await
            .expect("测试：查询活跃作业应成功")
            .map(|j| j.id),
        Some(first.enqueued[0].2.clone()),
        "重复扫描后活跃作业应仍是同一个",
    );
}

/// 场景 3：命名空间未配置 embedding provider ⇒ 按「未配置」处理，且不入队。
///
/// 判定的意义是「停止说谎」：条目不能永远挂着 `pending` 让扫描器每轮白跑。
/// 同时**不越界断言**「永远不可能被向量化」—— rag 层在 provider 为空时会回退到
/// `settings.defaultProviderId`，那句话可能为假。
#[tokio::test]
async fn sweep_marks_skipped_when_namespace_has_no_provider() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let ns = make_namespace(db, "无 provider 的命名空间", None).await;
    let item_id = make_pending_item(db, &ns, "不会被向量化的记忆").await;

    let report = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：孤儿扫描应成功");

    assert_eq!(report.marked_skipped, 1);
    assert_eq!(report.marked_failed, 0);
    assert_eq!(report.enqueued.len(), 0, "未配置 provider 时不该入队");
    assert!(all_jobs(db).await.is_empty(), "不该产生任何作业");

    let item = get_item(db, &item_id).await.expect("测试：应能取回条目");
    assert_eq!(item.index_status, status::SKIPPED);
    let err = item.index_error.expect("处理必须写明原因，否则前端只能看到状态突变");
    assert!(
        err.contains("embedding provider"),
        "原因应点明缺的是 embedding provider，实际为：{err}",
    );
    assert!(
        !err.contains("不参与向量索引") && !err.contains("不可能"),
        "不得断言「不可能被向量化」—— rag 层在 provider 为空时会回退默认 provider，\
         该断言可能为假。实际文案：{err}",
    );
}

/// 场景 3b：命名空间绑定的 provider **已不存在**（悬空引用）⇒ 终态 failed，且不入队。
///
/// 这是生产实证形态（2026-09-12）：`memory_namespaces` 的两个命名空间都绑着同一个
/// 已被删除的 provider `af052547-…`（实际存在的是 `llama.cpp` = `6f67c842-…`）。
///
/// 若放它入队，作业会一路走到 embedding 调用才失败（`Not found: Provider <id>`）。
///
/// 该错误现在**拦得住**：`indexing::build_embed_context` 会把它改写成带
/// `ERR_EMBEDDING_PROVIDER_GONE` 标记的消息，index_queue 的 R9 通道据此直接判终态
/// （2026-09-12 补的第二条出口）。所以「入队前判」的价值不再是「避免无意义重试」，
/// 而是**更早更省**：不产生作业（零队列占用）、不经过 worker（零 embed 尝试），
/// 且条目状态与原因在扫描当轮就可见。
#[tokio::test]
async fn sweep_marks_failed_when_provider_reference_is_dangling() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    // 刻意用一个绝不存在的 provider id
    const DANGLING: &str = "c0ffee00-dead-beef-0000-00000000dead";
    let ns =
        make_namespace(db, "悬空 provider 的命名空间", Some(format!("{DANGLING}::m.gguf"))).await;
    let item_id = make_pending_item(db, &ns, "绑定已失效的记忆").await;

    let report = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：孤儿扫描应成功");

    assert_eq!(report.marked_failed, 1, "悬空引用应判为确定性配置错误");
    assert_eq!(report.enqueued.len(), 0, "不得入队（必然失败且会引发无意义重试）");
    assert!(all_jobs(db).await.is_empty(), "不得产生任何作业（连失败的作业都不该有）");

    let item = get_item(db, &item_id).await.expect("测试：应能取回条目");
    assert_eq!(item.index_status, status::FAILED);
    let err = item.index_error.expect("终态失败必须写明原因");
    assert!(
        err.contains(DANGLING),
        "原因必须点出悬空的 provider id，用户才能据此重绑。实际：{err}",
    );
    assert!(
        err.contains("重建索引"),
        "原因必须给出恢复路径（重绑后重建索引），否则用户只知道失败了。实际：{err}",
    );

    // 已落终态 ⇒ 下一轮扫描不再重复处理（保证不会退化成每轮重试）
    let again = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：次轮扫描应成功");
    assert_eq!(again.scanned, 0, "终态条目不再属于 pending，不应被重复处理");
}

/// 场景 3c：旧格式（不含 `::`，只有 provider_id）不做悬空预判。
///
/// 原因：`indexing::resolve_embedding_provider` 会对旧格式自动补全 model_id，
/// 并在目标 provider 下无 embedding 模型时**跨 provider 兜底**，因此在扫描器里
/// 预判「不存在」会误伤一条本可成功的路径。
#[tokio::test]
async fn sweep_does_not_prejudge_legacy_provider_format() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let ns =
        make_namespace(db, "旧格式 provider 的命名空间", Some("legacy-provider-id".to_string()))
            .await;
    let item_id = make_pending_item(db, &ns, "旧格式的记忆").await;

    let report = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：孤儿扫描应成功");

    assert_eq!(report.enqueued.len(), 1, "旧格式应交由下游补全逻辑处理");
    assert_eq!(report.marked_failed, 0, "不得对旧格式预判悬空");
    let item = get_item(db, &item_id).await.expect("测试：应能取回条目");
    assert_eq!(item.index_status, status::PENDING, "入队后应保持 pending 等待队列");
}

/// 场景 4：非 `pending` 的条目一律不碰。
///
/// 反向锁：扫描器若把 `ready` / `failed` 也当成孤儿，会产生大量重复 embedding，
/// 且会把真失败（已写明 index_error）洗成 pending，掩盖问题。
#[tokio::test]
async fn sweep_ignores_non_pending_items() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let ns = make_namespace_with_live_provider(db, "Reflector Insights").await;
    let ready_id = make_pending_item(db, &ns, "已完成向量化的").await;
    let failed_id = make_pending_item(db, &ns, "向量化失败的").await;
    let skipped_id = make_pending_item(db, &ns, "明确跳过的").await;

    update_item_index_status(db, &ready_id, status::READY, None)
        .await
        .expect("测试：置为 ready 应成功");
    update_item_index_status(db, &failed_id, status::FAILED, Some("provider 返回 429"))
        .await
        .expect("测试：置为 failed 应成功");
    update_item_index_status(db, &skipped_id, status::SKIPPED, None)
        .await
        .expect("测试：置为 skipped 应成功");

    let report = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：孤儿扫描应成功");

    assert_eq!(report.scanned, 0, "无 pending 条目时扫描结果应为空");
    assert!(all_jobs(db).await.is_empty(), "不得为任何非 pending 条目入队");

    assert_eq!(
        get_item(db, &failed_id).await.expect("测试：应能取回条目").index_status,
        status::FAILED,
        "真失败必须保持 failed，不能被扫描器洗回 pending",
    );
}

/// 场景 5：活跃作业用了历史等价写法 `"memory"` 时，也必须判定为「已在队列中」。
///
/// 为什么需要这条：`enqueue_job` 的去重是**按 `container_type` 精确匹配**的，
/// 若扫描器只查 `"mem"`，就会在已存在 `"memory"` 作业的情况下再插一个 ——
/// 同一条记忆被做两遍 embedding。
#[tokio::test]
async fn sweep_respects_alias_container_type() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    let ns = make_namespace_with_live_provider(db, "Reflector Insights").await;
    let item_id = make_pending_item(db, &ns, "已有别名作业的").await;

    // 用别名写法手工塞一个活跃作业，模拟历史写入点
    jobs::enqueue_job(
        db,
        CreateIndexJobInput {
            job_type: JOB_TYPE_INDEX_MEMORY.to_string(),
            container_type: CONTAINER_TYPE_MEM_ALIAS.to_string(),
            container_id: ns.clone(),
            item_id: item_id.clone(),
            max_retries: None,
            priority: None,
            metadata: None,
        },
    )
    .await
    .expect("测试：手工入队应成功");

    let report = jobs::sweep_pending_memory_items(db, 200).await.expect("测试：孤儿扫描应成功");

    assert_eq!(report.already_queued, 1, "别名写法下的活跃作业也必须被认出来");
    assert_eq!(report.enqueued.len(), 0, "不得产出第二个作业");
    assert_eq!(all_jobs(db).await.len(), 1);

    // 直接调用单条判定，锁住枚举形态
    let item = get_item(db, &item_id).await.expect("测试：应能取回条目");
    let repair = jobs::repair_pending_memory_item(db, &item).await.expect("测试：单条判定应成功");
    assert_eq!(repair, PendingMemoryRepair::AlreadyQueued);
}
