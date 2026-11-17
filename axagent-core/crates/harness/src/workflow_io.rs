// SPDX-License-Identifier: AGPL-3.0-only

//! 工作流节点的外部 IO 契约（P6.0-⑥ 批 E）。
//!
//! 五个曾自带实现的执行器（`http_request` / `webhook_send` / `file_operation` /
//! `storage` 的 fileSystem 分支 / `email`）在此收口为三条接缝：
//!
//! - [`WorkflowHttpClient`]：HTTP 出站（方法/URL/头/体/超时/重定向策略）；
//! - [`WorkflowFileIo`]：本地文件读写删判（路径语义由调用方给定）；
//! - [`WorkflowSmtpSender`]：SMTP 发信（凭据解析留在执行器，传输归实现层）。
//!
//! **执行器保留的是策略，不是实现**：URL 空校验、dry-run 短路、SSRF 黑名单、
//! 凭据注入、`output_var` 包装都在执行器；协议细节（reqwest / std::fs / lettre）
//! 全部在实现层（wiring 注入）。

use std::collections::HashMap;

/// HTTP 出站请求（与具体 HTTP 客户端库无关的最小形状）。
#[derive(Debug, Clone)]
pub struct WorkflowHttpRequest {
    /// 大写方法名：GET / POST / PUT / PATCH / DELETE / HEAD / OPTIONS
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    /// 请求体原文（`None` = 无体）
    pub body: Option<String>,
    /// `true` 时实现层按 JSON 发送（等价 reqwest `.json()`），否则按原文发送
    pub body_is_json: bool,
    pub timeout_secs: u64,
    /// `false` ⇒ 实现层**禁止跟随重定向**（SSRF 防绕过，见 http_request 执行器）
    pub follow_redirects: bool,
}

/// HTTP 出站响应。
#[derive(Debug, Clone)]
pub struct WorkflowHttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// HTTP 出站接缝。
#[async_trait::async_trait]
pub trait WorkflowHttpClient: Send + Sync {
    /// 发送一次 HTTP 请求。
    /// 网络层失败（DNS/连接/超时）返回 `Err`；非 2xx 是**成功响应**（`Ok`，由调用方判 status）。
    async fn send(&self, request: WorkflowHttpRequest) -> Result<WorkflowHttpResponse, String>;
}

/// 本地文件 IO 接缝。
///
/// 同步语义与文件系统一致（与 `DocumentParser` 同型：调用点本就在同步临界区外，
/// 由执行器直接调用；不引入异步包装的假象）。
pub trait WorkflowFileIo: Send + Sync {
    fn read_to_string(&self, path: &str) -> Result<String, String>;
    /// `create_parent_dirs = true` 时实现层负责建父目录（storage 的 insert/upsert 语义）。
    fn write(&self, path: &str, content: &str, create_parent_dirs: bool) -> Result<(), String>;
    fn delete(&self, path: &str) -> Result<(), String>;
    fn exists(&self, path: &str) -> bool;
}

/// SMTP 发信请求（凭据已由执行器解析完毕）。
#[derive(Debug, Clone)]
pub struct WorkflowSmtpRequest {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    /// `true` ⇒ STARTTLS；`false` ⇒ 明文（与 email 执行器原语义一致）
    pub tls: bool,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// SMTP 发信接缝。
#[async_trait::async_trait]
pub trait WorkflowSmtpSender: Send + Sync {
    /// 发送一封纯文本邮件。构建/传输/投递失败均返回 `Err`。
    async fn send(&self, request: WorkflowSmtpRequest) -> Result<(), String>;
}
