// SPDX-License-Identifier: AGPL-3.0-only

//! 工作流节点外部 IO 接缝的宿主实现（P6.0-⑥ 批 E）。
//!
//! 三件事各自只有一份实现，全部落在 wiring 层：
//! - `HostHttpClient`：reqwest（超时 / 重定向策略由请求参数决定）；
//! - `HostFileIo`：std::fs（建父目录语义由参数决定）；
//! - `HostSmtpSender`：lettre（STARTTLS / 明文两种模式，与 email 执行器原语义一致）。
//!
//! 执行器侧保留的只有**策略**（URL 空校验 / dry-run 短路 / SSRF 黑名单 / 凭据注入 /
//! output 包装），协议细节一律经由本模块。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axagent_harness::workflow_io::{
    WorkflowFileIo, WorkflowHttpClient, WorkflowHttpRequest, WorkflowHttpResponse, WorkflowKvStore,
    WorkflowSmtpRequest, WorkflowSmtpSender, WorkflowVectorWriter,
};
use sea_orm::DatabaseConnection;

/// reqwest 实现（按请求参数构建客户端，与 http_request 执行器原有语义一致）。
pub struct HostHttpClient;

#[async_trait]
impl WorkflowHttpClient for HostHttpClient {
    async fn send(&self, request: WorkflowHttpRequest) -> Result<WorkflowHttpResponse, String> {
        let policy = if request.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(request.timeout_secs.clamp(5, 300)))
            .redirect(policy)
            .build()
            .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;

        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|e| format!("非法 HTTP 方法: {e}"))?;
        let mut req = client.request(method, &request.url);
        for (k, v) in &request.headers {
            req = req.header(k, v);
        }
        if let Some(body) = &request.body {
            req = if request.body_is_json {
                match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(v) => req.json(&v),
                    Err(_) => req.body(body.clone()),
                }
            } else {
                req.body(body.clone())
            };
        }

        let response = req.send().await.map_err(|e| format!("HTTP 请求失败: {e}"))?;
        let status = response.status().as_u16();
        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = response.text().await.map_err(|e| format!("读取响应体失败: {e}"))?;
        Ok(WorkflowHttpResponse { status, headers, body })
    }
}

/// std::fs 实现。
pub struct HostFileIo;

impl WorkflowFileIo for HostFileIo {
    fn read_to_string(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| format!("读取文件失败 ({path}): {e}"))
    }

    fn write(&self, path: &str, content: &str, create_parent_dirs: bool) -> Result<(), String> {
        if create_parent_dirs && let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, content).map_err(|e| format!("写入文件失败 ({path}): {e}"))
    }

    fn delete(&self, path: &str) -> Result<(), String> {
        std::fs::remove_file(path).map_err(|e| format!("删除文件失败 ({path}): {e}"))
    }

    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
}

/// lettre 实现（语义与 email 执行器原实现一致：587 走 STARTTLS，其余明文）。
pub struct HostSmtpSender;

#[async_trait]
impl WorkflowSmtpSender for HostSmtpSender {
    async fn send(&self, request: WorkflowSmtpRequest) -> Result<(), String> {
        use lettre::message::Mailbox;
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

        let from: Mailbox = request.from.parse().map_err(|e| format!("非法发件地址: {e}"))?;
        let mut builder = Message::builder().from(from).subject(&request.subject);
        for addr in &request.to {
            let mbox: Mailbox = addr.parse().map_err(|e| format!("非法收件地址 {addr}: {e}"))?;
            builder = builder.to(mbox);
        }
        let msg = builder
            .header(lettre::message::header::ContentType::TEXT_PLAIN)
            .body(request.body.clone())
            .map_err(|e| format!("构建邮件失败: {e}"))?;

        let creds = Credentials::new(request.user.clone(), request.pass.clone());
        let mailer = if request.tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&request.host)
        } else {
            Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&request.host))
        }
        .map_err(|e| format!("构建 SMTP 传输失败: {e}"))?
        .port(request.port)
        .credentials(creds)
        .build();

        mailer.send(msg).await.map_err(|e| format!("发送邮件失败: {e}"))?;
        Ok(())
    }
}

/// 通用 KV 写口实现（P6.0-⑥ storage(sqlite) 接真）：复用宿主 settings 表。
///
/// `collection` 作为命名空间前缀拼进 key（`wfkv:{collection}:{key}`），避免与宿主自身
/// settings key 冲突。`upsert` / `insert` 都走 `set_setting`（on conflict update 幂等）；
/// `append` 先读旧值再拼接。DB 失败如实返回 `Err`，不再假成功。
pub struct HostKvStore {
    db: DatabaseConnection,
}

impl HostKvStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl WorkflowKvStore for HostKvStore {
    async fn write(
        &self,
        collection: &str,
        key: &str,
        value: &str,
        mode: &str,
    ) -> Result<(), String> {
        let full_key = format!("wfkv:{collection}:{key}");
        let final_value = if mode == "append" {
            let old = axagent_dao::repo::settings::get_setting(&self.db, &full_key)
                .await
                .map_err(|e| format!("读取 KV 旧值失败: {e}"))?
                .unwrap_or_default();
            format!("{old}{value}")
        } else {
            value.to_string()
        };
        axagent_dao::repo::settings::set_setting(&self.db, &full_key, &final_value)
            .await
            .map_err(|e| format!("写入 KV 失败: {e}"))
    }
}

/// 向量库文本写口实现（P6.0-⑥ storage(vectorDb) 接真）：复用宿主 `VectorStore` 的
/// `insert_metadata_only_chunk`（无 embedding 的元数据文本写入，幂等 upsert）。
pub struct HostVectorWriter {
    store: Arc<axagent_search::vector_store::VectorStore>,
}

impl HostVectorWriter {
    pub fn new(store: Arc<axagent_search::vector_store::VectorStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl WorkflowVectorWriter for HostVectorWriter {
    async fn insert_text(
        &self,
        collection: &str,
        key: &str,
        content: &str,
        _mode: &str,
    ) -> Result<(), String> {
        // insert_metadata_only_chunk 自身即幂等 upsert（先删同名 id 再插），
        // 故 insert / upsert 两种 mode 语义一致。
        self.store
            .insert_metadata_only_chunk(collection, key, key, content)
            .await
            .map_err(|e| format!("写入向量库文本失败: {e}"))
    }
}
