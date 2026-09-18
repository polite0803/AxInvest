// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::sea_query::Expr;
use sea_orm::*;

use axagent_entities::index_jobs;
pub use axagent_entities::index_jobs::{Column, Entity};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::util_fns::gen_id;

pub const INDEX_JOB_STATUS_PENDING: &str = "pending";
pub const INDEX_JOB_STATUS_PROCESSING: &str = "processing";
pub const INDEX_JOB_STATUS_COMPLETED: &str = "completed";
pub const INDEX_JOB_STATUS_FAILED: &str = "failed";
pub const INDEX_JOB_STATUS_RETRYING: &str = "retrying";
pub const INDEX_JOB_STATUS_CANCELLED: &str = "cancelled";

pub const JOB_TYPE_INDEX_DOCUMENT: &str = "index_document";
pub const JOB_TYPE_INDEX_MEMORY: &str = "index_memory";
pub const JOB_TYPE_INDEX_WIKI_NOTE: &str = "index_wiki_note";
pub const JOB_TYPE_REBUILD_CONTAINER: &str = "rebuild_container";
pub const JOB_TYPE_REINDEX_DOCUMENT: &str = "reindex_document";
pub const JOB_TYPE_EXTRACT_ENTITIES: &str = "extract_entities";

pub const STAGE_PARSING: &str = "parsing";
pub const STAGE_CHUNKING: &str = "chunking";
pub const STAGE_EMBEDDING: &str = "embedding";
pub const STAGE_STORING: &str = "storing";
pub const STAGE_EXTRACTING: &str = "extracting";

pub fn model_to_job(m: index_jobs::Model) -> IndexJob {
    IndexJob {
        id: m.id,
        job_type: m.job_type,
        container_type: m.container_type,
        container_id: m.container_id,
        item_id: m.item_id,
        status: m.status,
        current_stage: m.current_stage,
        progress: m.progress,
        error_message: m.error_message,
        retry_count: m.retry_count,
        max_retries: m.max_retries,
        priority: m.priority,
        created_at: m.created_at,
        started_at: m.started_at,
        completed_at: m.completed_at,
        metadata: m.metadata,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexJob {
    pub id: String,
    pub job_type: String,
    pub container_type: String,
    pub container_id: String,
    pub item_id: String,
    pub status: String,
    pub current_stage: Option<String>,
    pub progress: i32,
    pub error_message: Option<String>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub priority: i32,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateIndexJobInput {
    pub job_type: String,
    pub container_type: String,
    pub container_id: String,
    pub item_id: String,
    pub max_retries: Option<i32>,
    pub priority: Option<i32>,
    pub metadata: Option<String>,
}

pub async fn enqueue_job(db: &DatabaseConnection, input: CreateIndexJobInput) -> Result<IndexJob> {
    // 去重检查：同一 container_type + item_id 已有活跃 job（pending / processing / retrying）则跳过
    if let Some(existing) =
        get_active_job_for_item(db, &input.container_type, &input.item_id).await?
    {
        tracing::debug!(
            container_type = %existing.container_type,
            item_id = %existing.item_id,
            existing_job_id = %existing.id,
            "[index_queue] 跳过重复入队，已有活跃 job"
        );
        return Ok(existing);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let id = gen_id();
    let am = index_jobs::ActiveModel {
        id: Set(id.clone()),
        job_type: Set(input.job_type),
        container_type: Set(input.container_type),
        container_id: Set(input.container_id),
        item_id: Set(input.item_id),
        status: Set(INDEX_JOB_STATUS_PENDING.to_string()),
        current_stage: Set(None),
        progress: Set(0),
        error_message: Set(None),
        retry_count: Set(0),
        max_retries: Set(input.max_retries.unwrap_or(3)),
        priority: Set(input.priority.unwrap_or(0)),
        created_at: Set(now),
        started_at: Set(None),
        completed_at: Set(None),
        metadata: Set(input.metadata),
    };

    am.insert(db).await?;
    get_job(db, &id).await
}

pub async fn get_job(db: &DatabaseConnection, id: &str) -> Result<IndexJob> {
    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;
    Ok(model_to_job(model))
}

pub async fn list_pending_jobs(db: &DatabaseConnection, limit: u64) -> Result<Vec<IndexJob>> {
    let models = index_jobs::Entity::find()
        .filter(index_jobs::Column::Status.eq(INDEX_JOB_STATUS_PENDING))
        .order_by_desc(index_jobs::Column::Priority)
        .order_by_asc(index_jobs::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;
    Ok(models.into_iter().map(model_to_job).collect())
}

pub async fn list_jobs_by_status(
    db: &DatabaseConnection,
    status: &str,
    limit: u64,
) -> Result<Vec<IndexJob>> {
    let models = index_jobs::Entity::find()
        .filter(index_jobs::Column::Status.eq(status))
        .order_by_desc(index_jobs::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;
    Ok(models.into_iter().map(model_to_job).collect())
}

pub async fn list_retryable_failed_jobs(db: &DatabaseConnection) -> Result<Vec<IndexJob>> {
    let retry_col = Expr::col(index_jobs::Column::RetryCount);
    let max_col = Expr::col(index_jobs::Column::MaxRetries);
    let models = index_jobs::Entity::find()
        .filter(index_jobs::Column::Status.eq(INDEX_JOB_STATUS_FAILED).and(retry_col.lt(max_col)))
        .order_by_asc(index_jobs::Column::CreatedAt)
        .all(db)
        .await?;
    Ok(models.into_iter().map(model_to_job).collect())
}

pub async fn list_jobs_by_container(
    db: &DatabaseConnection,
    container_type: &str,
    container_id: &str,
) -> Result<Vec<IndexJob>> {
    let models = index_jobs::Entity::find()
        .filter(
            index_jobs::Column::ContainerType
                .eq(container_type)
                .and(index_jobs::Column::ContainerId.eq(container_id)),
        )
        .order_by_desc(index_jobs::Column::CreatedAt)
        .all(db)
        .await?;
    Ok(models.into_iter().map(model_to_job).collect())
}

pub async fn list_jobs_by_item(
    db: &DatabaseConnection,
    container_type: &str,
    item_id: &str,
) -> Result<Vec<IndexJob>> {
    let models = index_jobs::Entity::find()
        .filter(
            index_jobs::Column::ContainerType
                .eq(container_type)
                .and(index_jobs::Column::ItemId.eq(item_id)),
        )
        .order_by_desc(index_jobs::Column::CreatedAt)
        .all(db)
        .await?;
    Ok(models.into_iter().map(model_to_job).collect())
}

pub async fn mark_job_processing(
    db: &DatabaseConnection,
    id: &str,
    stage: Option<&str>,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;

    let mut am: index_jobs::ActiveModel = model.into();
    am.status = Set(INDEX_JOB_STATUS_PROCESSING.to_string());
    am.current_stage = Set(stage.map(|s| s.to_string()));
    am.started_at = Set(Some(now));
    am.error_message = Set(None);
    am.update(db).await?;
    Ok(())
}

pub async fn update_job_progress(
    db: &DatabaseConnection,
    id: &str,
    stage: Option<&str>,
    progress: i32,
) -> Result<()> {
    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;

    let mut am: index_jobs::ActiveModel = model.into();
    if let Some(s) = stage {
        am.current_stage = Set(Some(s.to_string()));
    }
    am.progress = Set(progress.clamp(0, 100));
    am.update(db).await?;
    Ok(())
}

pub async fn mark_job_completed(db: &DatabaseConnection, id: &str) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;

    let mut am: index_jobs::ActiveModel = model.into();
    am.status = Set(INDEX_JOB_STATUS_COMPLETED.to_string());
    am.progress = Set(100);
    am.current_stage = Set(None);
    am.completed_at = Set(Some(now));
    am.error_message = Set(None);
    am.update(db).await?;
    Ok(())
}

pub async fn mark_job_failed(db: &DatabaseConnection, id: &str, error: &str) -> Result<IndexJob> {
    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;

    let mut am: index_jobs::ActiveModel = model.into();
    let retry_count = am.retry_count.take().expect("IndexJob 缺少 retry_count");
    let next_retry = retry_count + 1;

    let max_retries = am.max_retries.take().expect("IndexJob 缺少 max_retries");
    if next_retry < max_retries {
        am.status = Set(INDEX_JOB_STATUS_RETRYING.to_string());
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        am.status = Set(INDEX_JOB_STATUS_FAILED.to_string());
        am.completed_at = Set(Some(now));
    }
    am.retry_count = Set(next_retry);
    am.current_stage = Set(None);
    am.error_message = Set(Some(error.to_string()));
    let updated = am.update(db).await?;
    Ok(model_to_job(updated))
}

/// 标记任务为终态失败且不重试。
///
/// 用于确定性配置错误（如 embedding provider 未配置，R9）：这类错误重试
/// max_retries 次结果必然相同，直接进入 failed 终态避免指数退避空转。
pub async fn mark_job_failed_no_retry(
    db: &DatabaseConnection,
    id: &str,
    error: &str,
) -> Result<IndexJob> {
    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut am: index_jobs::ActiveModel = model.into();
    am.status = Set(INDEX_JOB_STATUS_FAILED.to_string());
    am.completed_at = Set(Some(now));
    am.current_stage = Set(None);
    am.error_message = Set(Some(error.to_string()));
    let updated = am.update(db).await?;
    Ok(model_to_job(updated))
}

pub async fn reset_job_for_retry(db: &DatabaseConnection, id: &str) -> Result<()> {
    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;

    let mut am: index_jobs::ActiveModel = model.into();
    am.status = Set(INDEX_JOB_STATUS_PENDING.to_string());
    am.progress = Set(0);
    am.current_stage = Set(None);
    am.started_at = Set(None);
    am.error_message = Set(None);
    am.update(db).await?;
    Ok(())
}

pub async fn cancel_job(db: &DatabaseConnection, id: &str) -> Result<()> {
    let model = index_jobs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("IndexJob {}", id)))?;

    let mut am: index_jobs::ActiveModel = model.into();
    let status = am.status.take().expect("IndexJob 缺少 status");
    if status != INDEX_JOB_STATUS_PROCESSING && status != INDEX_JOB_STATUS_PENDING {
        return Err(AxAgentError::Validation(
            "Can only cancel pending or processing jobs".to_string(),
        ));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    am.status = Set(INDEX_JOB_STATUS_CANCELLED.to_string());
    am.completed_at = Set(Some(now));
    am.current_stage = Set(None);
    am.update(db).await?;
    Ok(())
}

pub async fn cancel_pending_jobs_for_item(
    db: &DatabaseConnection,
    container_type: &str,
    item_id: &str,
) -> Result<u64> {
    let result = index_jobs::Entity::update_many()
        .col_expr(index_jobs::Column::Status, Expr::value(INDEX_JOB_STATUS_CANCELLED))
        .filter(
            index_jobs::Column::ContainerType
                .eq(container_type)
                .and(index_jobs::Column::ItemId.eq(item_id))
                .and(
                    index_jobs::Column::Status
                        .is_in([INDEX_JOB_STATUS_PENDING, INDEX_JOB_STATUS_RETRYING]),
                ),
        )
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 取消指定容器下所有未完成（pending / retrying / processing）的索引任务。
///
/// 用于删除知识容器（wiki / knowledge_base / memory namespace）时清理残留任务，
/// 避免队列继续轮询已删除容器导致 NotFound 错误刷屏。
pub async fn cancel_jobs_by_container(
    db: &DatabaseConnection,
    container_type: &str,
    container_id: &str,
) -> Result<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let result = index_jobs::Entity::update_many()
        .col_expr(index_jobs::Column::Status, Expr::value(INDEX_JOB_STATUS_CANCELLED))
        .col_expr(index_jobs::Column::CompletedAt, Expr::value(now))
        .filter(
            index_jobs::Column::ContainerType
                .eq(container_type)
                .and(index_jobs::Column::ContainerId.eq(container_id))
                .and(index_jobs::Column::Status.is_in([
                    INDEX_JOB_STATUS_PENDING,
                    INDEX_JOB_STATUS_RETRYING,
                    INDEX_JOB_STATUS_PROCESSING,
                ])),
        )
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 物理删除指定容器下的所有索引任务。
/// 用于删除知识容器（wiki / knowledge_base / memory namespace）时彻底清理残留 job 数据。
pub async fn delete_jobs_by_container(
    db: &DatabaseConnection,
    container_type: &str,
    container_id: &str,
) -> Result<u64> {
    let result = index_jobs::Entity::delete_many()
        .filter(
            index_jobs::Column::ContainerType
                .eq(container_type)
                .and(index_jobs::Column::ContainerId.eq(container_id)),
        )
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn count_jobs_by_status(db: &DatabaseConnection, status: &str) -> Result<u64> {
    let count =
        index_jobs::Entity::find().filter(index_jobs::Column::Status.eq(status)).count(db).await?;
    Ok(count)
}

pub async fn cleanup_completed_jobs(db: &DatabaseConnection, older_than_ms: i64) -> Result<u64> {
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
        - older_than_ms;

    let result = index_jobs::Entity::delete_many()
        .filter(
            index_jobs::Column::Status
                .is_in([INDEX_JOB_STATUS_COMPLETED, INDEX_JOB_STATUS_CANCELLED])
                .and(index_jobs::Column::CompletedAt.lte(cutoff)),
        )
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn get_active_job_for_item(
    db: &DatabaseConnection,
    container_type: &str,
    item_id: &str,
) -> Result<Option<IndexJob>> {
    let model = index_jobs::Entity::find()
        .filter(
            index_jobs::Column::ContainerType
                .eq(container_type)
                .and(index_jobs::Column::ItemId.eq(item_id))
                .and(index_jobs::Column::Status.is_in([
                    INDEX_JOB_STATUS_PENDING,
                    INDEX_JOB_STATUS_PROCESSING,
                    INDEX_JOB_STATUS_RETRYING,
                ])),
        )
        .order_by_desc(index_jobs::Column::CreatedAt)
        .one(db)
        .await?;
    Ok(model.map(model_to_job))
}

pub async fn list_all_jobs(
    db: &DatabaseConnection,
    limit: u64,
    offset: u64,
) -> Result<(Vec<IndexJob>, u64)> {
    let total = index_jobs::Entity::find().count(db).await?;
    let models = index_jobs::Entity::find()
        .order_by_desc(index_jobs::Column::CreatedAt)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await?;
    Ok((models.into_iter().map(model_to_job).collect(), total))
}

// ────────────────────────────────────────────────────────────────────────────
// 孤儿记忆索引条目的兜底推进
// ────────────────────────────────────────────────────────────────────────────

/// 记忆容器的标准 `container_type` 写法。
///
/// 队列消费侧（`index_queue::is_mem_container`）同时接受 `"mem"` 与 `"memory"`
/// 两种等价写法，但 `enqueue_job` 的去重是**按字符串精确匹配**的 —— 若两个写入点
/// 用了不同写法，同一条目会同时存在两个活跃作业（做两遍 embedding）。故本文件
/// 以本常量为准，且判定「是否已有活跃作业」时两种写法都查。
pub const CONTAINER_TYPE_MEM: &str = "mem";
/// `"mem"` 的历史等价写法，见 [`CONTAINER_TYPE_MEM`]。
pub const CONTAINER_TYPE_MEM_ALIAS: &str = "memory";

/// 单条 pending 记忆条目的兜底处置结论。四种结局互斥且穷尽。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingMemoryRepair {
    /// 队列里已有活跃作业（pending / processing / retrying），无需介入。
    /// 这也构成幂等保证：重复扫描不会重复入队。
    AlreadyQueued,
    /// 命名空间未配置 embedding provider，按「未配置」处理，不进入向量索引队列。
    /// 与新增记忆时的判定口径一致（`commands::memory::add_memory_item`）。
    MarkedSkipped,
    /// 命名空间绑定的 embedding provider **已不存在**（悬空引用）⇒ 确定性配置错误，
    /// 已把条目标为 `failed` 并写明可操作的恢复指引。**不入队**：这类错误重试多少次
    /// 结果都一样，入队只会产出「必然失败 + 指数退避空转」的作业与日志噪音。
    MarkedFailed,
    /// 已补入索引作业，载荷为新建的作业 id。
    Enqueued(String),
}

/// 单轮孤儿扫描的统计结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrphanSweepReport {
    /// 本轮检出的 pending 条目总数。
    pub scanned: usize,
    /// 补入索引作业的条目：`(item_id, namespace_id, job_id)`。
    pub enqueued: Vec<(String, String, String)>,
    /// 因命名空间无 embedding provider 而按「未配置」处理的条目数。
    pub marked_skipped: usize,
    /// 因绑定悬空 provider（确定性配置错误）而标记为 failed 的条目数。
    pub marked_failed: usize,
    /// 队列里已有活跃作业、无需介入的条目数。
    pub already_queued: usize,
}

/// 扫描「状态为 pending 但队列里没有任何活跃作业」的记忆条目，并为它们补入队。
///
/// # 为什么需要这个函数
///
/// `memory_items.index_status` 由本 crate 在插入时写成 `pending`，而「入队」是
/// 上层（命令层，因为只有它拿得到 `AppHandle`）的**独立动作**。任何不经命令层的
/// 写入路径都会留下永久 `pending`：
///
/// - `trajectory::storage::save_memory`（DAO/entity 层）
/// - `agent::reflector::persist_insight`（走 `MemoryRepository` trait）
/// - `agent::project_memory`、`tools::agent_memory`
///
/// 这些路径都拿不到 Tauri 上下文，**结构上不可能**自行入队。所以兜底必须做在
/// 队列侧 —— 逐点给每个写入路径补 enqueue 只会不断漏掉新出现的写入点。
///
/// 生产实证（2026-09-12，PG）：`index_jobs` 中 `container_type='mem'` 与
/// `job_type='index_memory'` 的记录数**均为 0** —— 记忆向量化从未入队过一次；
/// 同时 `memory_items` 有 2 条 `source='reflector'` 的条目自 09-06 起持续
/// `pending`（`index_error` 为空），`vec_collections` 里也没有任何 `mem_*` 集合。
///
/// # 幂等性
///
/// 每个条目都先查活跃作业，且 `enqueue_job` 内部另有同维度去重，
/// 因此可安全地按周期反复调用。
pub async fn sweep_pending_memory_items(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<OrphanSweepReport> {
    let items = crate::repo::memory::list_items_by_index_status(
        db,
        axagent_harness::constants::status::PENDING,
        limit,
    )
    .await?;

    let mut report = OrphanSweepReport { scanned: items.len(), ..Default::default() };
    for item in &items {
        match repair_pending_memory_item(db, item).await {
            Ok(PendingMemoryRepair::AlreadyQueued) => report.already_queued += 1,
            Ok(PendingMemoryRepair::MarkedSkipped) => report.marked_skipped += 1,
            Ok(PendingMemoryRepair::MarkedFailed) => report.marked_failed += 1,
            Ok(PendingMemoryRepair::Enqueued(job_id)) => {
                report.enqueued.push((item.id.clone(), item.namespace_id.clone(), job_id));
            },
            Err(e) => {
                // 单条失败不中断整轮：否则一条坏数据会永久阻塞它后面所有条目。
                tracing::warn!(
                    item_id = %item.id,
                    error = %e,
                    "[index_jobs] 修复孤儿记忆条目失败，跳过该条",
                );
            },
        }
    }
    Ok(report)
}

/// 处理单条 pending 记忆条目。
///
/// 判定顺序即优先级：**先看队列，再看 provider**。反过来会让「已有活跃作业但
/// 命名空间 provider 刚被清空」的条目被误降为 `skipped`，而它的作业马上就会
/// 把它写成 `ready`（两个写入者互相覆盖）。
pub async fn repair_pending_memory_item(
    db: &DatabaseConnection,
    item: &axagent_harness::types::MemoryItem,
) -> Result<PendingMemoryRepair> {
    // 1. 已有活跃作业 ⇒ 队列会处理，不介入。
    //    两种等价写法都查，避免漏判另一种写法下的活跃作业而产出重复作业。
    for container_type in [CONTAINER_TYPE_MEM, CONTAINER_TYPE_MEM_ALIAS] {
        if get_active_job_for_item(db, container_type, &item.id).await?.is_some() {
            return Ok(PendingMemoryRepair::AlreadyQueued);
        }
    }

    // 2. 命名空间未配置 embedding provider ⇒ 按「未配置」处理。
    //    刻意不断言「永远不可能被向量化」：rag 层的
    //    `ContainerSource::resolve_embedding_provider` 在 provider 为空时会回退到
    //    `settings.defaultProviderId`，那条路径是可能成功的。此处只是沿用
    //    「新增记忆时不配 provider 就不入队」这一既有口径
    //    （`commands::memory::add_memory_item`），保持两条链语义一致。
    let ns = crate::repo::memory::get_namespace(db, &item.namespace_id).await?;
    let provider_ref = match ns.embedding_provider.as_deref() {
        Some(p) => p,
        None => {
            let reason = format!("命名空间「{}」未配置 embedding provider", ns.name);
            crate::repo::memory::update_item_index_status(
                db,
                &item.id,
                axagent_harness::constants::status::SKIPPED,
                Some(&reason),
            )
            .await?;
            tracing::info!(
                item_id = %item.id,
                namespace = %ns.name,
                "[index_jobs] 命名空间未配置 embedding provider，条目按跳过处理",
            );
            return Ok(PendingMemoryRepair::MarkedSkipped);
        },
    };

    // 3. 绑定的 provider 已不存在（悬空引用）⇒ 确定性配置错误，判为终态失败。
    //
    //    为什么必须在入队**之前**判：`resolve_embedding_provider` 只检查
    //    `embedding_provider.is_some()`，并不校验被引用的 provider id 是否还活着，
    //    因此悬空引用会一路走到 embedding 调用才失败（`Not found: Provider <id>`）。
    //
    //    该错误现在**拦得住**：`indexing::build_embed_context` 会把它改写成带
    //    `ERR_EMBEDDING_PROVIDER_GONE` 标记的消息，index_queue 的 R9 通道据此直接判
    //    终态（2026-09-12 补的第二条出口）。因此「入队前判」的价值不再是「避免无意义
    //    重试」，而是**更早更省**：不产生作业（零队列占用）、不经过 worker（零 embed
    //    尝试），且条目状态与原因在扫描当轮就可见 —— 作业即便被 R9 判失败，也仍占用
    //    一轮调度。
    //
    //    生产实证（2026-09-12）：`memory_namespaces` 的两个命名空间都绑着同一个
    //    已被删除的 provider `af052547-…`（实际存在的是 `llama.cpp` = `6f67c842-…`），
    //    扫描器补入的 2 个作业因此停在 `retrying` 并刷出 4 条 WARN。
    //
    //    只对完整格式（`providerId::modelId`）校验：不含 `::` 的旧格式由
    //    `indexing::resolve_embedding_provider` 负责补全且会跨 provider 兜底，
    //    在此预判会误伤。
    //    条件是 let-chain（Edition 2024）而非嵌套 `if`：`clippy::collapsible_if`
    //    在 `-D warnings` 下会拒绝嵌套写法，而本项目 CI 强制 clippy 零警告。
    if let Some((provider_id, _model_id)) = provider_ref.split_once("::")
        && !provider_id.is_empty()
        && !crate::repo::provider::provider_exists(db, provider_id).await?
    {
        let reason = format!(
            "命名空间「{}」绑定的 embedding provider {} 已不存在（provider 被删除，\
             或重建后换了 id）。请在设置中重新为该命名空间绑定 embedding provider，\
             然后对该条目执行重建索引。",
            ns.name, provider_id
        );
        crate::repo::memory::update_item_index_status(
            db,
            &item.id,
            axagent_harness::constants::status::FAILED,
            Some(&reason),
        )
        .await?;
        tracing::warn!(
            item_id = %item.id,
            namespace = %ns.name,
            provider_id = %provider_id,
            "[index_jobs] 命名空间绑定的 embedding provider 不存在，判为终态失败（不入队）",
        );
        return Ok(PendingMemoryRepair::MarkedFailed);
    }

    // 4. 真的孤儿 ⇒ 补入队。
    let job = enqueue_job(
        db,
        CreateIndexJobInput {
            job_type: JOB_TYPE_INDEX_MEMORY.to_string(),
            container_type: CONTAINER_TYPE_MEM.to_string(),
            container_id: item.namespace_id.clone(),
            item_id: item.id.clone(),
            max_retries: None,
            priority: None,
            metadata: None,
        },
    )
    .await?;

    tracing::info!(
        job_id = %job.id,
        item_id = %item.id,
        namespace = %ns.name,
        updated_at = %item.updated_at,
        "[index_jobs] 检出孤儿记忆条目，补入索引作业",
    );
    Ok(PendingMemoryRepair::Enqueued(job.id))
}
