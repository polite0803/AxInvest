// SPDX-License-Identifier: AGPL-3.0-only
//! OPC 域插件 worker。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! #36 第 1 批只有骨架（describe + 帧循环 + shutdown，零业务命令）。**第 3–4 步**接进第一条
//! 真实链路：命名命令 `opc_list_blog_posts` → 出站只读接缝 `opc.store` 的 `list_blog_posts`
//! op → 宿主 provider → dao。**写面第一批**加 `opc_publish_blog_post` → 同一接缝的
//! `publish_blog_post` op（两态枚举逐字透传）。**本批（#180-①②）**再加两条写命令：
//! `opc_delete_blog_post` → `delete_blog_post`（**三态**：`removed` / `not_found` /
//! `protected`）与 `opc_create_blog_post` → `create_blog_post`（入参是既有
//! `CreateBlogPostInput` 的五个字段，正文的**字节预算**在宿主判、本进程一个数字都不写）。
//! 四条命令的数据主权（连接池、列裁剪、分页闸、写列清单、字节闸、保护 id 名单）全在宿主侧，
//! 本进程只做宿主做不了的一件事：「整表语义」的分页取尽（见 [`blog_posts`]）。出参线格式
//! 逐字透传接缝视图 / 枚举串的 camelCase 与 snake_case，**不再有任何键名适配层**
//! （禁区 13 无例外，判据见该文「出参线格式」一节）。
//!
//! **本批（#36 批 1）**再加 9 条只读命令：8 条 `opc_list_*` 各对一条 `opc.store` 只读 op
//! （出参 = 接缝回帧的裸数组逐字透传），`opc_get_dashboard` 由本进程组合三次列表查询现算
//! （宿主不新增聚合 op）。
//!
//! 为什么用 `Peer` 而不是自写读循环：插件一旦能主动调宿主，同一条管道上就同时挂着
//! 「我方请求的响应」与「对端发来的新请求」，朴素读一帧写一帧必然错配。`Peer` 把对端请求
//! 进 `deferred` 队列，等响应期间不丢帧。
//!
//! 为什么是纯可执行包（只有 `src/main.rs`、没有 `src/lib.rs`）：与 `plugin-stock`、
//! `schema-gen` 同一形态 —— worker 由宿主 spawn，没有 Cargo 消费方，不需要库入口。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, Peer, PluginDeclaration, ResponseKind, error_codes, ops, serve,
};
use serde_json::{Value, json};

/// worker 侧的 Peer 具体类型：`serve` 固定用加锁的 stdin / stdout（三处签名共用一份）。
type WorkerPeer = Peer<StdinLock<'static>, StdoutLock<'static>>;

/// 本二进制归属的插件**名**（清单 `manifest.name` 逐字一致）。
///
/// ⚠ 这里存的**不是**完整插件 id：宿主 spawn worker 时经 `AXAGENT_PLUGIN_ID` 注入的是
/// **注册表键**，形态为 `{小写 name}@{marketplace}`（`plugins/src/manager.rs` 的
/// `plugin_id()` + `worker.rs` 的 `command.env("AXAGENT_PLUGIN_ID", …)`）—— 内置包是
/// `opc@bundled`，外部安装是 `opc@external`。⇒ 写死一份完整 id 的字面量在两条路上必然
/// 有一条对不上，会把每一条真实命令都拒成 `PLUGIN_COMMAND_ID_MISMATCH`。
/// 归属判据因此只看**名**：见 [`own_id_belongs_to_this_plugin`]。
const PLUGIN_NAME: &str = "opc";

/// 注入身份是否属于本二进制声明的那个插件（`{name}` 或 `{name}@{任意市场}`）。
fn own_id_belongs_to_this_plugin(own_id: &str) -> bool {
    own_id == PLUGIN_NAME || own_id.starts_with(&format!("{PLUGIN_NAME}@"))
}

/// 出站宿主接缝：OPC 站点内容的数据面（契约见 harness `src/opc_store.rs`）。
/// 本插件不持 `DatabaseConnection`，一切表读表写都走它。
const OPC_STORE_SEAM: &str = "opc.store";
/// 读 op（表外 op 在宿主分派层落 `SEAM_CALL_UNSUPPORTED`）。
const OPC_STORE_LIST_BLOG_POSTS: &str = "list_blog_posts";
/// 写 op：把一行博客置为已发布。宿主侧要 `write` 权限（`worker.rs` 的
/// `required_permission_for`），故**本插件清单必须声明 `write`** —— `subprocess_execution`
/// 补不齐它（`has_permission` 是精确 `contains`），少声明一条就是 `PLUGIN_PERMISSION_DENIED`。
const OPC_STORE_PUBLISH_BLOG_POST: &str = "publish_blog_post";
/// 写 op：删除一行博客（同样要 `write`）。出参是**三态**枚举串（`removed` / `not_found` /
/// `protected`），与 `publish_blog_post` 的两态是同一套「正常值走 payload」口径。
const OPC_STORE_DELETE_BLOG_POST: &str = "delete_blog_post";
/// 写 op：新建一行博客（同样要 `write`）。正文的字节预算由**宿主**按契约常量判，
/// 本进程既不截断也不预算（`MAX_FRAME_LEN` 那 64 MiB 是帧层上限，不是本 op 的预算）。
const OPC_STORE_CREATE_BLOG_POST: &str = "create_blog_post";

/// 八个只读查询 op（#36 批 1，均为读面 ⇒ 宿主侧**无权限闸**）：入参是嵌一层的
/// `request` DTO（契约 `*ListQuery`），出参是 `*SummaryView` 的**裸数组**。
const OPC_STORE_LIST_INVOICES: &str = "list_invoices";
const OPC_STORE_LIST_CUSTOMERS: &str = "list_customers";
const OPC_STORE_LIST_PROJECTS: &str = "list_projects";
const OPC_STORE_LIST_LANDING_PAGES: &str = "list_landing_pages";
const OPC_STORE_LIST_CONTACTS: &str = "list_contacts";
const OPC_STORE_LIST_KPIS: &str = "list_kpis";
const OPC_STORE_LIST_CONTENT_ASSETS: &str = "list_content_assets";
const OPC_STORE_LIST_PUBLISH_SCHEDULES: &str = "list_publish_schedules";
/// 看板总览（零入参，宿主整表聚合；与 `opc_get_dashboard` 的「首页现算」不同口径）。
const OPC_STORE_DASHBOARD_SUMMARY: &str = "dashboard_summary";

/// #36 批 2 的四条写 op（同样要 `write`）：发票状态变更 / 内容资产更新 / 内容资产删除 /
/// 发布计划取消。出参一律是契约的外部标记枚举（`{"transitioned": …}` / `{"updated": …}` /
/// `"removed"` / `"not_found"`），本进程逐字透传。
const OPC_STORE_TRANSITION_INVOICE: &str = "transition_invoice";
const OPC_STORE_UPDATE_CONTENT_ASSET: &str = "update_content_asset";
const OPC_STORE_DELETE_CONTENT_ASSET: &str = "delete_content_asset";
const OPC_STORE_CANCEL_PUBLISH_SCHEDULE: &str = "cancel_publish_schedule";

/// #36 批 3 的九条 op：八个写 op（要 `write`）+ 一条读 op（财务报表，宿主侧无权限闸）。
/// 出参一律是契约的列裁剪视图 / 视图数组，逐字透传（同 `create_blog_post` 的判据）。
const OPC_STORE_CREATE_INVOICE: &str = "create_invoice";
const OPC_STORE_CREATE_CUSTOMER: &str = "create_customer";
const OPC_STORE_CREATE_PROJECT: &str = "create_project";
const OPC_STORE_ADD_MILESTONE: &str = "add_milestone";
const OPC_STORE_CREATE_LANDING_PAGE: &str = "create_landing_page";
const OPC_STORE_GET_FINANCIAL_REPORT: &str = "get_financial_report";
const OPC_STORE_CREATE_CONTENT_ASSET: &str = "create_content_asset";
const OPC_STORE_CREATE_PUBLISH_SCHEDULE: &str = "create_publish_schedule";
const OPC_STORE_PROCESS_DUE_SCHEDULES: &str = "process_due_schedules";

/// #32 批 A 的三条 op（都要 `write`）：客户更新 / 客户删除 / 发票删除。前两者同属
/// 客户面，`delete_invoice` 删的是**域包发票**——宿主同名 Tauri 命令删的是交付域发票
/// （两套体系同名不同义，见 `worker.rs` 分派臂的警示）。`update_customer` 出参是
/// [`CustomerSummaryView`] 逐字透传；两条删除 op 成功无载荷（接缝出 `Value::Null`）。
const OPC_STORE_UPDATE_CUSTOMER: &str = "update_customer";
const OPC_STORE_DELETE_CUSTOMER: &str = "delete_customer";
const OPC_STORE_DELETE_INVOICE: &str = "delete_invoice";

/// #32 批 B 的四条项目 op（读 1 / 写 3）：详情读 + 更新 + 删除 + 里程碑完成。
/// `get_project` 是读 op（宿主 `required_permission_for` 刻意不登记 ⇒ 不要 `write`）；
/// 其余三条改库，其中删除**不可逆** ⇒ 都要 `write`。
const OPC_STORE_GET_PROJECT: &str = "get_project";
const OPC_STORE_UPDATE_PROJECT: &str = "update_project";
const OPC_STORE_DELETE_PROJECT: &str = "delete_project";
const OPC_STORE_COMPLETE_MILESTONE: &str = "complete_milestone";

/// #36 D-4 刀的三条 op（都要 `write`）：发布 / 删除落地页 + 联系标记已读。
/// 后两条成功无载荷（接缝出 `Value::Null`）；发布出参是摘要视图（`published` 已翻转）。
const OPC_STORE_PUBLISH_LANDING_PAGE: &str = "publish_landing_page";
const OPC_STORE_DELETE_LANDING_PAGE: &str = "delete_landing_page";
const OPC_STORE_MARK_CONTACT_READ: &str = "mark_contact_read";

/// #264 批 1：交付域五 op（三写两读）。
const OPC_STORE_LIST_DELIVERY_INVOICES: &str = "list_delivery_invoices";
const OPC_STORE_CREATE_INVOICE_FROM_LEAD: &str = "create_invoice_from_lead";
const OPC_STORE_UPDATE_DELIVERY_INVOICE_STATUS: &str = "update_delivery_invoice_status";
const OPC_STORE_DELETE_DELIVERY_INVOICE: &str = "delete_delivery_invoice";
const OPC_STORE_DELIVERY_SUMMARY: &str = "delivery_summary";

/// #264 批 2：需求发现八 op（五写三读）。
const OPC_STORE_LIST_PLATFORMS: &str = "list_platforms";
const OPC_STORE_SAVE_PLATFORM: &str = "save_platform";
const OPC_STORE_DELETE_PLATFORM: &str = "delete_platform";
const OPC_STORE_LIST_LEADS: &str = "list_leads";
const OPC_STORE_GET_SCAN_POLICY: &str = "get_scan_policy";
const OPC_STORE_SAVE_SCAN_POLICY: &str = "save_scan_policy";
const OPC_STORE_DISCOVER_AND_EVALUATE_LEADS: &str = "discover_and_evaluate_leads";
const OPC_STORE_CREATE_LEAD: &str = "create_lead";

/// #264 批 3：需求实现三 op（都要 `write`）—— 状态机 / 转模板 / 启动执行。
/// 三条都改库（转模板还会回灌能力索引），无读 op 随迁。
const OPC_STORE_UPDATE_LEAD_STATUS: &str = "update_lead_status";
const OPC_STORE_CONVERT_LEAD_TO_WORKFLOW: &str = "convert_lead_to_workflow";
const OPC_STORE_RUN_LEAD_WORKFLOW: &str = "run_lead_workflow";

/// #264 批 4：领域工作流三只读 op（都不登记权限闸）。
const OPC_STORE_LIST_DOMAIN_WORKFLOWS: &str = "list_domain_workflows";
const OPC_STORE_GET_DOMAIN_WORKFLOW: &str = "get_domain_workflow";
const OPC_STORE_LIST_DOMAINS: &str = "list_domains";

/// #264 批 5：交付面六 op —— execute / update / retry / cancel 四写（都要 `write`），
/// list / get 两读（不登记权限闸）。
const OPC_STORE_EXECUTE_DEMAND_WORKFLOW: &str = "execute_demand_workflow";
const OPC_STORE_LIST_DELIVERIES: &str = "list_deliveries";
const OPC_STORE_GET_DELIVERY: &str = "get_delivery";
const OPC_STORE_UPDATE_DELIVERY: &str = "update_delivery";
const OPC_STORE_RETRY_DELIVERY: &str = "retry_delivery";
const OPC_STORE_CANCEL_DELIVERY: &str = "cancel_delivery";

/// #264 批 6：能力发现面五 op —— proactive / close / analyze 三写（都要 `write`），
/// scan_capabilities / list_capability_gaps 两读（不登记权限闸）。
const OPC_STORE_SCAN_CAPABILITIES: &str = "scan_capabilities";
const OPC_STORE_PROACTIVE_EVALUATE_LEADS: &str = "proactive_evaluate_leads";
const OPC_STORE_LIST_CAPABILITY_GAPS: &str = "list_capability_gaps";
const OPC_STORE_CLOSE_CAPABILITY_GAP: &str = "close_capability_gap";
const OPC_STORE_ANALYZE_CAPABILITY_GAPS: &str = "analyze_capability_gaps";

/// #264 批 7 刀 A：需求订阅面三 op（`save_subscription` / `delete_subscription` 要
/// `write`，`list_subscriptions` 不登记权限闸）。
const OPC_STORE_LIST_SUBSCRIPTIONS: &str = "list_subscriptions";
const OPC_STORE_SAVE_SUBSCRIPTION: &str = "save_subscription";
const OPC_STORE_DELETE_SUBSCRIPTION: &str = "delete_subscription";

/// #264 批 7 刀 A：域包配置面八 op，全只读 ⇒ 宿主权限表一条都不登记。
///
/// ⚠ 两组同名不同源，不可合并：`list_industries` 读宿主代码内的 14 份注册表，
/// `list_runtime_industries` 读磁盘上的域包目录（迁移前就是两条不同的命令）。
const OPC_STORE_LIST_INDUSTRIES: &str = "list_industries";
const OPC_STORE_GET_CAPABILITY_PACK_CONFIG: &str = "get_capability_pack_config";
const OPC_STORE_GET_ACTION_CONFIG: &str = "get_action_config";
const OPC_STORE_LIST_CAPABILITY_PACK_ACTIONS: &str = "list_capability_pack_actions";
const OPC_STORE_LIST_CAPABILITY_PACK_WORKFLOWS: &str = "list_capability_pack_workflows";
const OPC_STORE_LIST_RUNTIME_INDUSTRIES: &str = "list_runtime_industries";
const OPC_STORE_GET_KPI_DEFINITIONS: &str = "get_kpi_definitions";
const OPC_STORE_HAS_CAPABILITY_PACK: &str = "has_capability_pack";

/// #264 批 7 刀 B1：域包全读七 op（薄层透传），全只读 ⇒ 宿主权限表一条都不登记。
///
/// 出参除 `get_workflow_config`（复用刀 A 的 `CapabilityPackWorkflowConfig`）外，
/// 其余六条是宿主拼好的 `serde_json::Value`（外层 camelCase 键与迁移前命令体逐字同形），
/// worker 与本插件都不做二次映射。
const OPC_STORE_GET_WORKFLOW_CONFIG: &str = "get_workflow_config";
const OPC_STORE_VALIDATE_ENTITY: &str = "validate_entity";
const OPC_STORE_BATCH_VALIDATE_ENTITIES: &str = "batch_validate_entities";
const OPC_STORE_COMPUTE_KPIS: &str = "compute_kpis";
const OPC_STORE_GET_CAPABILITY_PACK_DATA: &str = "get_capability_pack_data";
const OPC_STORE_GET_CAPABILITY_PACK_PRECHECK: &str = "get_capability_pack_precheck";
const OPC_STORE_GET_CAPABILITY_PACK_HEALTH: &str = "get_capability_pack_health";

/// #264 批 7 刀 B2：域包单据面十二 op（薄层透传）。
///
/// 六条写 op（`update_invoice` / `record_kpi` / `import_talent_library` / `sync_fleet` /
/// `update_publish_schedule` / `execute_dynamic_workflow`）在宿主权限表登记 `Write`；
/// 六条读 op（`get_invoice` / `get_customer` / `find_customer_by_email` / `list_revenue` /
/// `get_investment_advice` / `get_publish_schedule`）一条闸都不登记 —— 两侧同批改，判据在
/// `worker.rs` 的 `required_permission_for` 与 `READ_OPS` / `WRITE_OPS` 用例。
const OPC_STORE_GET_INVOICE: &str = "get_invoice";
const OPC_STORE_UPDATE_INVOICE: &str = "update_invoice";
const OPC_STORE_GET_CUSTOMER: &str = "get_customer";
const OPC_STORE_FIND_CUSTOMER_BY_EMAIL: &str = "find_customer_by_email";
const OPC_STORE_RECORD_KPI: &str = "record_kpi";
const OPC_STORE_LIST_REVENUE: &str = "list_revenue";
const OPC_STORE_GET_INVESTMENT_ADVICE: &str = "get_investment_advice";
const OPC_STORE_IMPORT_TALENT_LIBRARY: &str = "import_talent_library";
const OPC_STORE_SYNC_FLEET: &str = "sync_fleet";
const OPC_STORE_GET_PUBLISH_SCHEDULE: &str = "get_publish_schedule";
const OPC_STORE_UPDATE_PUBLISH_SCHEDULE: &str = "update_publish_schedule";
const OPC_STORE_EXECUTE_DYNAMIC_WORKFLOW: &str = "execute_dynamic_workflow";

/// #264 刀 C 批 2：域包分析执行（读 op）。风控阈值取域包 `runtime.yaml` 声明、
/// 数据服务与时间窗装配都在宿主 provider，本层零权限闸。
const OPC_STORE_EXECUTE_ANALYSIS: &str = "execute_analysis";

/// #264 批 7 刀 B3：域包自动规则 / 仪表盘四 op（三读一写）。
///
/// `run_automation_rules` 在宿主权限表登记 `Write`（规则动作会改库）；另三条
/// （仪表盘 / 工作流步骤 / 启用规则清单）是纯读，宿主侧无权限闸 —— 两侧同批改，
/// 判据在 `worker.rs` 的 `required_permission_for` 与 `READ_OPS` / `WRITE_OPS` 用例。
const OPC_STORE_CAPABILITY_PACK_DASHBOARD: &str = "capability_pack_dashboard";
const OPC_STORE_CAPABILITY_PACK_WORKFLOW_STEPS: &str = "capability_pack_workflow_steps";
const OPC_STORE_CAPABILITY_PACK_AUTOMATION_RULES: &str = "capability_pack_automation_rules";
const OPC_STORE_RUN_AUTOMATION_RULES: &str = "run_automation_rules";

/// #264 批 7 刀 B3（后三条）：订阅扫描 / 扫描任务装配（两写）+ 能力匹配（一读）。
///
/// 前两条在宿主权限表登记 `Write`（扫描落库并推进到期时间、幂等建改调度任务）；
/// `match_lead_capabilities` 是纯读 ⇒ 无闸。两侧同批改，判据在 `worker.rs` 的
/// `required_permission_for` 与 `READ_OPS` / `WRITE_OPS` 用例。
const OPC_STORE_RUN_SUBSCRIPTION_SCAN: &str = "run_subscription_scan";
const OPC_STORE_ENSURE_DEMAND_SCAN_JOB: &str = "ensure_demand_scan_job";
const OPC_STORE_MATCH_LEAD_CAPABILITIES: &str = "match_lead_capabilities";

/// 刀 B3 收尾：自动学习（写 op —— 落 RL 经验并触发反思/进化/策略优化）。
///
/// 学习引擎与域包适配器注册表只在宿主装配，插件进程够不着 ⇒ 必须走接缝回宿主；
/// 宿主权限表同批登记 `Write`，判据在 `worker.rs` 的 `required_permission_for` 与
/// `WRITE_OPS` 用例。
const OPC_STORE_AUTO_LEARN_WORKFLOW: &str = "auto_learn_workflow";

/// #264 刀 C 批 1：域包 manifest / prompt 装配 + 学习面五条读 op。
///
/// 全部纯读 ⇒ 宿主权限表一条闸都不加（`worker.rs` 的 `READ_OPS` 用例反向钉住「只读插件
/// 不得被拒」）。出参形状与信封键名钉在宿主 provider（迁移前命令体逐字同形），本插件只做
/// 入参解包与 camelCase→snake_case 的接缝键名转写。
const OPC_STORE_CAPABILITY_PACK_MANIFEST: &str = "capability_pack_manifest";
const OPC_STORE_BUILD_CAPABILITY_PACK_PROMPT: &str = "build_capability_pack_prompt";
const OPC_STORE_GET_LEARNING_METRICS: &str = "get_learning_metrics";
const OPC_STORE_GET_LEARNING_CONFIG: &str = "get_learning_config";
const OPC_STORE_LIST_LEARNING_CONFIGS: &str = "list_learning_configs";

/// #264 刀 C 批 3：学习面七 op（六写一读）。
///
/// 学习引擎与域包适配器注册表只在宿主装配（`AutoLearnHandles`），插件进程够不着 ⇒
/// 全部走接缝回宿主。六条写 op 在宿主权限表登记 `Write`（判据在 `worker.rs` 的
/// `required_permission_for` 与 `WRITE_OPS` 用例）；`get_rl_stats` 是纯读 ⇒ 无闸。
const OPC_STORE_REFLECT_ON_WORKFLOW: &str = "reflect_on_workflow";
const OPC_STORE_EVOLVE_WORKFLOW: &str = "evolve_workflow";
const OPC_STORE_RUN_SELF_IMPROVEMENT: &str = "run_self_improvement";
const OPC_STORE_TRIGGER_CAPABILITY_PACK_LEARNING: &str = "trigger_capability_pack_learning";
const OPC_STORE_GET_RL_STATS: &str = "get_rl_stats";
const OPC_STORE_TRIGGER_RL_OPTIMIZATION: &str = "trigger_rl_optimization";
const OPC_STORE_RECORD_RL_EXPERIENCE: &str = "record_rl_experience";

/// #264 批 4a：工作项六写一读（七 op）。
///
/// 工作项状态机与质量门都在宿主（`company-runtime` + 自改进执行器），插件进程够不着 ⇒
/// 全部走接缝回宿主。六条写 op 在宿主权限表登记 `Write`；`kanban_board` 是纯读 ⇒ 无闸。
const OPC_STORE_CREATE_WORK_ITEM: &str = "create_work_item";
const OPC_STORE_KANBAN_BOARD: &str = "kanban_board";
const OPC_STORE_WORK_ITEM_START: &str = "work_item_start";
const OPC_STORE_WORK_ITEM_REVIEW: &str = "work_item_review";
const OPC_STORE_ESCALATE_WORK_ITEM: &str = "escalate_work_item";
const OPC_STORE_WORK_ITEM_UNBLOCK: &str = "work_item_unblock";
const OPC_STORE_RUN_SELF_IMPROVING_WORK_ITEM: &str = "run_self_improving_work_item";

/// 本插件承接的命名命令（manifest `commands[].name` 必须与它逐字一致：宿主
/// `execute_plugin_command` 先按清单查命令，未声明的那条直接 `NotFound`）。
const COMMAND_LIST_BLOG_POSTS: &str = "opc_list_blog_posts";
/// 发布命令。宿主同名 Tauri 命令已在 #36 blog 刀删除（先例 #144 的双声明收口：
/// 前端已切完通道）——本条是发布博客的唯一通道，回宿主的两态枚举串。
const COMMAND_PUBLISH_BLOG_POST: &str = "opc_publish_blog_post";
/// 删除命令。宿主同名 Tauri 命令已随 #36 blog 刀删除 —— 本条是删除博客的唯一通道，
/// 出参是三态枚举串逐字透传。
const COMMAND_DELETE_BLOG_POST: &str = "opc_delete_blog_post";
/// 新建命令。宿主同名 Tauri 命令已随 #36 blog 刀删除 —— 本条是新建博客的唯一通道。
///
/// ⚠ 入参形状与已删的宿主命令**不同名不同形**：宿主命令曾是 `{ input: CreateBlogPostInput }`
/// （Tauri 具名参数），本命令按清单 `inputSchema` 收**平铺**的五个字段（与 `opc_publish_blog_post`
/// 的平铺 `id` 同风格）。
const COMMAND_CREATE_BLOG_POST: &str = "opc_create_blog_post";

/// #36 批 1 的 9 条只读命令：前 8 条各对一条只读接缝 op，第 9 条由本进程组合三次列表调用
/// 现算看板读数（宿主**不**新增聚合 op）。
const COMMAND_LIST_INVOICES: &str = "opc_list_invoices";
const COMMAND_LIST_CUSTOMERS: &str = "opc_list_customers";
const COMMAND_LIST_PROJECTS: &str = "opc_list_projects";
const COMMAND_LIST_LANDING_PAGES: &str = "opc_list_landing_pages";
const COMMAND_LIST_CONTACTS: &str = "opc_list_contacts";
const COMMAND_LIST_KPIS: &str = "opc_list_kpis";
const COMMAND_LIST_CONTENT_ASSETS: &str = "opc_list_content_assets";
const COMMAND_LIST_PUBLISH_SCHEDULES: &str = "opc_list_publish_schedules";
const COMMAND_GET_DASHBOARD: &str = "opc_get_dashboard";
/// 看板总览命令。宿主同名 Tauri 命令已随 #267 删除 —— 本条是看板唯一通道，
/// 出参与迁移前逐字段同形（camelCase）。
const COMMAND_GET_DASHBOARD_SUMMARY: &str = "opc_get_dashboard_summary";

/// #36 批 2 的四条写命令（清单 `commands[].name` 必须逐字一致）。
const COMMAND_TRANSITION_INVOICE: &str = "opc_transition_invoice";
const COMMAND_UPDATE_CONTENT_ASSET: &str = "opc_update_content_asset";
const COMMAND_DELETE_CONTENT_ASSET: &str = "opc_delete_content_asset";
const COMMAND_CANCEL_PUBLISH_SCHEDULE: &str = "opc_cancel_publish_schedule";

/// #36 批 3 的九条命令（清单 `commands[].name` 必须逐字一致）：八条写命令 + 一条只读的
/// 财务报表（宿主侧无权限闸，与八个 list op 同一档）。
const COMMAND_CREATE_INVOICE: &str = "opc_create_invoice";
const COMMAND_CREATE_CUSTOMER: &str = "opc_create_customer";
const COMMAND_CREATE_PROJECT: &str = "opc_create_project";
const COMMAND_ADD_MILESTONE: &str = "opc_add_milestone";
const COMMAND_CREATE_LANDING_PAGE: &str = "opc_create_landing_page";
const COMMAND_GET_FINANCIAL_REPORT: &str = "opc_get_financial_report";
const COMMAND_CREATE_CONTENT_ASSET: &str = "opc_create_content_asset";
const COMMAND_CREATE_PUBLISH_SCHEDULE: &str = "opc_create_publish_schedule";
const COMMAND_PROCESS_DUE_SCHEDULES: &str = "opc_process_due_schedules";

/// #32 批 A 的三条写命令（清单 `commands[].name` 必须逐字一致）。`opc_update_customer` /
/// `opc_delete_customer` 与宿主同名 Tauri 命令同义（批 C 删宿主侧后本条是唯一通道）；
/// `opc_delete_invoice` 与宿主同名 Tauri 命令**同名不同义**（域包发票 vs 交付域发票）。
const COMMAND_UPDATE_CUSTOMER: &str = "opc_update_customer";
const COMMAND_DELETE_CUSTOMER: &str = "opc_delete_customer";
const COMMAND_DELETE_INVOICE: &str = "opc_delete_invoice";

/// #32 批 B 的四条项目命令（清单 `commands[].name` 必须逐字一致）：一条只读详情
/// （`opc_get_project`，与宿主同名命令同义，批 C 删宿主侧后本条是唯一通道）+ 三条写命令。
const COMMAND_GET_PROJECT: &str = "opc_get_project";
const COMMAND_UPDATE_PROJECT: &str = "opc_update_project";
const COMMAND_DELETE_PROJECT: &str = "opc_delete_project";
const COMMAND_COMPLETE_MILESTONE: &str = "opc_complete_milestone";

/// #36 D-4 刀的三条落地页 / 联系命令（清单 `commands[].name` 必须逐字一致）：
/// 发布 / 删除落地页 + 联系标记已读，宿主同名 Tauri 命令随本刀删除
/// （先例 #144 的双声明收口）——本组是这三个写面的唯一通道。
const COMMAND_PUBLISH_LANDING_PAGE: &str = "opc_publish_landing_page";
const COMMAND_DELETE_LANDING_PAGE: &str = "opc_delete_landing_page";
const COMMAND_MARK_CONTACT_READ: &str = "opc_mark_contact_read";

/// #264 批 1：交付域五命令（清单 `commands[].name` 必须逐字一致）。宿主同名 Tauri 命令
/// 随本批删除 —— 本组是这五个交付面的唯一通道。
///
/// ⚠ 与域包发票命令（`opc_list_invoices` / `opc_delete_invoice`）**同名不同义**：
/// 交付账本只列 `lead_id IS NOT NULL` 的交付来源行（迁移前的宿主模块头对此有说明，
/// 该模块已随本批删除）。
const COMMAND_LIST_DELIVERY_INVOICES: &str = "opc_list_delivery_invoices";
const COMMAND_CREATE_INVOICE_FROM_LEAD: &str = "opc_create_invoice_from_lead";
const COMMAND_UPDATE_INVOICE_STATUS: &str = "opc_update_invoice_status";
const COMMAND_DELETE_DELIVERY_INVOICE: &str = "opc_delete_delivery_invoice";
const COMMAND_GET_DELIVERY_SUMMARY: &str = "opc_get_delivery_summary";

/// #264 批 2：需求发现八命令（清单 `commands[].name` 必须逐字一致）。宿主同名 Tauri 命令
/// 随本批删除（模块 `commands::opc_demand_discovery` 整文件移除）。
const COMMAND_LIST_PLATFORMS: &str = "opc_list_platforms";
const COMMAND_SAVE_PLATFORM: &str = "opc_save_platform";
const COMMAND_DELETE_PLATFORM: &str = "opc_delete_platform";
const COMMAND_LIST_LEADS: &str = "opc_list_leads";
const COMMAND_GET_SCAN_POLICY: &str = "opc_get_scan_policy";
const COMMAND_SAVE_SCAN_POLICY: &str = "opc_save_scan_policy";
const COMMAND_DISCOVER_AND_EVALUATE_LEADS: &str = "opc_discover_and_evaluate_leads";
const COMMAND_CREATE_LEAD: &str = "opc_create_lead";

/// #264 批 3：需求实现三命令（清单 `commands[].name` 必须逐字一致）。宿主同名 Tauri 命令
/// 随本批删除（模块 `commands::opc_demand_implement` 整文件移除）。
const COMMAND_UPDATE_LEAD_STATUS: &str = "opc_update_lead_status";
const COMMAND_CONVERT_LEAD_TO_WORKFLOW: &str = "opc_convert_lead_to_workflow";
const COMMAND_RUN_LEAD_WORKFLOW: &str = "opc_run_lead_workflow";

/// #264 批 4：领域工作流三只读命令（清单 `commands[].name` 必须逐字一致）。宿主同名 Tauri
/// 命令随本批删除（模块 `commands::opc_workflows::domain_workflows` 整文件移除）。
/// 前端零 `invoke`：三条原本是 `#[agent_command]` 工具，迁后由插件命名命令继续出现在
/// 智能体命令面（#146 的合并通道），工具名与出参语义不变。
const COMMAND_LIST_DOMAIN_WORKFLOWS: &str = "opc_list_domain_workflows";
const COMMAND_GET_DOMAIN_WORKFLOW: &str = "opc_get_domain_workflow";
const COMMAND_LIST_DOMAINS: &str = "opc_list_domains";

/// #264 批 5：交付面六命令（清单 `commands[].name` 必须逐字一致）。宿主同名 Tauri 命令
/// 随本批从 `commands::demand_discovery` 删除（该文件余发现面五条）；前端四处裸 `invoke`
/// 同批切 `invokePluginCommand`。
const COMMAND_EXECUTE_DEMAND_WORKFLOW: &str = "opc_execute_demand_workflow";
const COMMAND_LIST_DELIVERIES: &str = "opc_list_deliveries";
const COMMAND_GET_DELIVERY: &str = "opc_get_delivery";
const COMMAND_UPDATE_DELIVERY: &str = "opc_update_delivery";
const COMMAND_RETRY_DELIVERY: &str = "opc_retry_delivery";
const COMMAND_CANCEL_DELIVERY: &str = "opc_cancel_delivery";

/// #264 批 6：能力发现面五命令（清单 `commands[].name` 必须逐字一致）。宿主同名 Tauri
/// 命令随本批删除（`commands::demand_discovery` 整文件退役；需求发现 cron 兜底轮留在
/// 宿主 `init::services`，与插件命令面无关）。
const COMMAND_SCAN_CAPABILITIES: &str = "opc_scan_capabilities";
const COMMAND_PROACTIVE_EVALUATE_LEADS: &str = "opc_proactive_evaluate_leads";
const COMMAND_LIST_CAPABILITY_GAPS: &str = "opc_list_capability_gaps";
const COMMAND_CLOSE_CAPABILITY_GAP: &str = "opc_close_capability_gap";
const COMMAND_ANALYZE_CAPABILITY_GAPS: &str = "opc_analyze_capability_gaps";

/// #264 批 7 刀 A：订阅面三命令 + 域包配置面八命令（清单 `commands[].name` 必须逐字一致）。
/// 宿主同名的十一条 Tauri 命令随本批删除 —— #273 已裁定这 28 条零前端调用命令**全部保留**
/// （`#[agent_command]` 让它们在智能体工具面是活命令），故唯一收口路径就是迁 T2，
/// 且插件命令必须保持 `opc_*` 原名（改名等于换掉智能体侧的工具名）。
const COMMAND_LIST_SUBSCRIPTIONS: &str = "opc_list_subscriptions";
const COMMAND_SAVE_SUBSCRIPTION: &str = "opc_save_subscription";
const COMMAND_DELETE_SUBSCRIPTION: &str = "opc_delete_subscription";
const COMMAND_LIST_INDUSTRIES: &str = "opc_list_industries";
const COMMAND_GET_CAPABILITY_PACK_CONFIG: &str = "opc_get_capability_pack_config";
const COMMAND_GET_ACTION_CONFIG: &str = "opc_get_action_config";
const COMMAND_LIST_CAPABILITY_PACK_ACTIONS: &str = "opc_list_capability_pack_actions";
const COMMAND_LIST_CAPABILITY_PACK_WORKFLOWS: &str = "opc_list_capability_pack_workflows";
const COMMAND_LIST_RUNTIME_INDUSTRIES: &str = "opc_list_runtime_industries";
const COMMAND_GET_KPI_DEFINITIONS: &str = "opc_get_kpi_definitions";
const COMMAND_HAS_CAPABILITY_PACK: &str = "opc_has_capability_pack";

/// #264 批 7 刀 B1：域包全读七命令（清单 `commands[].name` 必须逐字一致）。宿主同名七条
/// Tauri 命令随本批删除 —— 七条全读，仍按 #273 裁定走迁 T2 收口，插件命令保持 `opc_*`
/// 原名（智能体工具名与出参形状不变）。
const COMMAND_GET_WORKFLOW_CONFIG: &str = "opc_get_workflow_config";
const COMMAND_VALIDATE_ENTITY: &str = "opc_validate_entity";
const COMMAND_BATCH_VALIDATE_ENTITIES: &str = "opc_batch_validate_entities";
const COMMAND_COMPUTE_KPIS: &str = "opc_compute_kpis";
const COMMAND_GET_CAPABILITY_PACK_DATA: &str = "opc_get_capability_pack_data";
const COMMAND_GET_CAPABILITY_PACK_PRECHECK: &str = "opc_get_capability_pack_precheck";
const COMMAND_GET_CAPABILITY_PACK_HEALTH: &str = "opc_get_capability_pack_health";

/// #264 批 7 刀 B2：域包单据面十二命令（清单 `commands[].name` 必须逐字一致）。宿主同名
/// 十二条 Tauri 命令随本批删除 —— 六读六写都按 #273 裁定迁 T2 收口，插件命令保持 `opc_*`
/// 原名（智能体工具名与出参形状不变）。六条写命令还需同批进 `scripts/build-plugin-bundle.mjs`
/// 的 `OPC_WRITE_COMMANDS`（否则智能体面按未声明 = 只读档暴露）。
const COMMAND_GET_INVOICE: &str = "opc_get_invoice";
const COMMAND_UPDATE_INVOICE: &str = "opc_update_invoice";
const COMMAND_GET_CUSTOMER: &str = "opc_get_customer";
const COMMAND_FIND_CUSTOMER_BY_EMAIL: &str = "opc_find_customer_by_email";
const COMMAND_RECORD_KPI: &str = "opc_record_kpi";
const COMMAND_LIST_REVENUE: &str = "opc_list_revenue";
const COMMAND_GET_INVESTMENT_ADVICE: &str = "opc_get_investment_advice";
const COMMAND_IMPORT_TALENT_LIBRARY: &str = "opc_import_talent_library";
const COMMAND_SYNC_FLEET: &str = "opc_sync_fleet";
const COMMAND_GET_PUBLISH_SCHEDULE: &str = "opc_get_publish_schedule";
const COMMAND_UPDATE_PUBLISH_SCHEDULE: &str = "opc_update_publish_schedule";
const COMMAND_EXECUTE_DYNAMIC_WORKFLOW: &str = "opc_execute_dynamic_workflow";

/// #264 刀 C 批 2：域包分析执行（读命令，宿主同名 `#[agent_command]` 随本批删除 ⇒
/// 命令名续用原名，智能体工具面与前端调用名不变）。读命令 ⇒ 不进 `OPC_WRITE_COMMANDS`。
const COMMAND_EXECUTE_ANALYSIS: &str = "opc_execute_analysis";

/// #264 批 7 刀 B3：域包自动规则 / 仪表盘四命令（三读一写，宿主同名 Tauri 命令随本批删除）。
/// `opc_run_automation_rules` 是写命令，同批进 `scripts/build-plugin-bundle.mjs` 的
/// `OPC_WRITE_COMMANDS`（否则智能体面按未声明 = 只读档暴露）。
const COMMAND_GET_CAPABILITY_PACK_DASHBOARD: &str = "opc_get_capability_pack_dashboard";
const COMMAND_GET_CAPABILITY_PACK_WORKFLOW_STEPS: &str = "opc_get_capability_pack_workflow_steps";
const COMMAND_GET_CAPABILITY_PACK_AUTOMATION_RULES: &str =
    "opc_get_capability_pack_automation_rules";
const COMMAND_RUN_AUTOMATION_RULES: &str = "opc_run_automation_rules";

/// #264 批 7 刀 B3（后三条）：订阅扫描 / 扫描任务装配（两写）+ 能力匹配（一读）。
///
/// 宿主同名三条 Tauri 命令随本批删除（`opc_demand_subscription.rs` /
/// `opc_demand_capability.rs` 只留自由函数，供 `HostOpcStore` 转调）。两条写命令还需
/// 同批进 `scripts/build-plugin-bundle.mjs` 的 `OPC_WRITE_COMMANDS`。
const COMMAND_RUN_SUBSCRIPTION_SCAN: &str = "opc_run_subscription_scan";
const COMMAND_ENSURE_DEMAND_SCAN_JOB: &str = "opc_ensure_demand_scan_job";
const COMMAND_MATCH_LEAD_CAPABILITIES: &str = "opc_match_lead_capabilities";

/// 刀 B3 收尾：自动学习命令（宿主同名 `#[agent_command]` 已随本批改走接缝；
/// 命令名续用原名，智能体工具面按名单续用）。写命令 ⇒ 同批进 `OPC_WRITE_COMMANDS`。
const COMMAND_OPC_AUTO_LEARN_WORKFLOW: &str = "opc_auto_learn_workflow";

/// #264 刀 C 批 1：域包 manifest / prompt 装配 + 学习面五条命令（全读，宿主同名
/// `#[agent_command]` 已随本批删除，命令名续用原名 ⇒ 智能体工具面与前端调用名不变）。
/// 读命令 ⇒ **不进** `scripts/build-plugin-bundle.mjs` 的 `OPC_WRITE_COMMANDS`。
const COMMAND_GET_CAPABILITY_PACK: &str = "opc_get_capability_pack";
const COMMAND_BUILD_CAPABILITY_PACK_PROMPT: &str = "opc_build_capability_pack_prompt";
const COMMAND_GET_LEARNING_METRICS: &str = "opc_get_learning_metrics";
const COMMAND_GET_LEARNING_CONFIG: &str = "opc_get_learning_config";
const COMMAND_LIST_LEARNING_CONFIGS: &str = "opc_list_learning_configs";

/// #264 刀 C 批 3：学习面七条命令（六写一读，宿主同名 `#[tauri::command]` 随本批删除，
/// 命令名续用原名 ⇒ 智能体工具面与前端调用名不变）。
///
/// 六条写命令同批进 `scripts/build-plugin-bundle.mjs` 的 `OPC_WRITE_COMMANDS`；
/// `opc_get_rl_stats` 是读命令 ⇒ 不进。
const COMMAND_REFLECT_ON_WORKFLOW: &str = "opc_reflect_on_workflow";
const COMMAND_EVOLVE_WORKFLOW: &str = "opc_evolve_workflow";
const COMMAND_RUN_SELF_IMPROVEMENT: &str = "opc_run_self_improvement";
const COMMAND_TRIGGER_CAPABILITY_PACK_LEARNING: &str = "opc_trigger_capability_pack_learning";
const COMMAND_GET_RL_STATS: &str = "opc_get_rl_stats";
const COMMAND_TRIGGER_RL_OPTIMIZATION: &str = "opc_trigger_rl_optimization";
const COMMAND_RECORD_RL_EXPERIENCE: &str = "opc_record_rl_experience";

/// #264 批 4a：工作项七条命令（六写一读）。命令名续用宿主原名 ⇒ 前端调用名与智能体
/// 工具面不变；宿主同名 `#[tauri::command]` 处于双声明期，待前端半迁完再删。
///
/// 六条写命令同批进 `scripts/build-plugin-bundle.mjs` 的 `OPC_WRITE_COMMANDS`；
/// `opc_kanban_board` 是读命令 ⇒ 不进。
const COMMAND_CREATE_WORK_ITEM: &str = "opc_create_work_item";
const COMMAND_KANBAN_BOARD: &str = "opc_kanban_board";
const COMMAND_WORK_ITEM_START: &str = "opc_work_item_start";
const COMMAND_WORK_ITEM_REVIEW: &str = "opc_work_item_review";
const COMMAND_ESCALATE_WORK_ITEM: &str = "opc_escalate_work_item";
const COMMAND_WORK_ITEM_UNBLOCK: &str = "opc_work_item_unblock";
const COMMAND_RUN_SELF_IMPROVING_WORK_ITEM: &str = "run_self_improving_opc_work_item";

fn main() -> ExitCode {
    match serve(handle) {
        // 对端关管道 = 宿主停机，正常退出（`Peer::run` 里 EOF 不是错误）。
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[axagent-plugin-opc] 帧通道失败：{e}");
            ExitCode::FAILURE
        },
    }
}

fn handle(request: &FrameRequest, peer: &mut WorkerPeer) -> FrameResponse {
    match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        // T2 命令面：宿主发 `CommandFrame`（pluginId 绑定 + 命令名 + 入参 + 预算）。
        ops::COMMAND => handle_command(request, peer),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("OPC 插件未实现该操作：{other}"),
        ),
    }
}

/// 处理一条命令帧：身份自检 → 按命令名分发。
///
/// 身份自检是**第二道防线**（第一道在宿主 `WorkerInvoker::execute_command` 写帧前）：
/// 帧上的 `plugin_id` 必须等于 spawn 时注入的 `AXAGENT_PLUGIN_ID`，防宿主侧错配。
fn handle_command(request: &FrameRequest, peer: &mut WorkerPeer) -> FrameResponse {
    let frame = match CommandFrame::from_frame_request(request) {
        Ok(frame) => frame,
        Err(e) => {
            return FrameResponse::error(
                error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                format!("命令帧解码失败：{e}"),
            );
        },
    };
    let own_id = std::env::var("AXAGENT_PLUGIN_ID").unwrap_or_default();
    if frame.plugin_id != own_id {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧绑定 `{}` 与本 worker 身份 `{own_id}` 不符，拒绝执行", frame.plugin_id),
        )
        .into_frame_response();
    }
    // 注入身份与二进制自述身份也要一致：上面那条只在「宿主注入值 = 帧绑定值」时成立，
    // 宿主把 OPC 的帧错配到别的 worker 时两边是**一起错**的 ⇒ 那条拦不住，这里按名再钉一次
    // 二进制归属。判据只看名前缀，不比完整 id（完整 id 有 `@bundled` / `@external` 两种真形态）。
    if !own_id_belongs_to_this_plugin(&own_id) {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("worker 身份 `{own_id}` 不属于本二进制声明的插件 `{PLUGIN_NAME}`，拒绝执行"),
        )
        .into_frame_response();
    }
    let outcome = match frame.command.as_str() {
        // 博客列表取数：行数据来自宿主 `opc.store` 的 `list_blog_posts`，本命令只做整表
        // 分页（形状与判据见 [`blog_posts`]）。
        COMMAND_LIST_BLOG_POSTS => match blog_posts(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 发布一篇博客：唯一写面，本命令只做入参形状 + 逐字透出宿主的两态结果。
        COMMAND_PUBLISH_BLOG_POST => {
            match publish_blog_post(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 删除一篇博客：入参形状同 publish（只有 `id`），出参是**三态**枚举的逐字透传
        // （判据见 [`delete_blog_post`]）。
        COMMAND_DELETE_BLOG_POST => {
            match delete_blog_post(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 新建一篇博客：入参是平铺的五个 DTO 字段（形状判在本层），出参是列裁剪视图
        // （判据见 [`create_blog_post`]）。
        COMMAND_CREATE_BLOG_POST => {
            match create_blog_post(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #36 批 1：八条只读查询命令（出参 = 接缝回帧的裸数组，逐字透传）──
        COMMAND_LIST_INVOICES => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_INVOICES,
            QUERY_KEYS_INVOICES,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_CUSTOMERS => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_CUSTOMERS,
            QUERY_KEYS_CUSTOMERS,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_PROJECTS => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_PROJECTS,
            QUERY_KEYS_PROJECTS,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 以下四条无入参（契约 op 也不收 query）。
        COMMAND_LIST_LANDING_PAGES => {
            match list_rows(
                &frame.input,
                &request.call_chain,
                peer,
                OPC_STORE_LIST_LANDING_PAGES,
                &[],
            ) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_CONTACTS => {
            match list_rows(&frame.input, &request.call_chain, peer, OPC_STORE_LIST_CONTACTS, &[]) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_KPIS => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_KPIS,
            QUERY_KEYS_KPIS,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_CONTENT_ASSETS => {
            match list_rows(
                &frame.input,
                &request.call_chain,
                peer,
                OPC_STORE_LIST_CONTENT_ASSETS,
                &[],
            ) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_PUBLISH_SCHEDULES => {
            match list_rows(
                &frame.input,
                &request.call_chain,
                peer,
                OPC_STORE_LIST_PUBLISH_SCHEDULES,
                &[],
            ) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #36 批 2：四条写命令（出参 = 接缝外部标记枚举，逐字透传）──
        COMMAND_TRANSITION_INVOICE => {
            match transition_invoice(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_UPDATE_CONTENT_ASSET => {
            match update_content_asset(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_DELETE_CONTENT_ASSET => {
            match delete_content_asset(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CANCEL_PUBLISH_SCHEDULE => {
            match cancel_publish_schedule(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #36 批 3：八条写命令 + 一条只读报表（出参 = 契约列裁剪视图，逐字透传）──
        COMMAND_CREATE_INVOICE => match create_invoice(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_CREATE_CUSTOMER => match create_customer(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── #32 批 A：三条写命令（更新出参 = 列裁剪视图，删除出参 = 无载荷）──
        COMMAND_UPDATE_CUSTOMER => match update_customer(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_DELETE_CUSTOMER => match delete_customer(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_DELETE_INVOICE => match delete_invoice(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── #32 批 B：四条项目命令（详情读出参 = 详情视图；删除出参 = 无载荷）──
        COMMAND_GET_PROJECT => match get_project(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_UPDATE_PROJECT => match update_project(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_DELETE_PROJECT => match delete_project(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_COMPLETE_MILESTONE => {
            match complete_milestone(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #36 D-4 刀：落地页发布 / 删除 + 联系标记已读（成功无载荷的两条出 Null）──
        COMMAND_PUBLISH_LANDING_PAGE => {
            match publish_landing_page(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_DELETE_LANDING_PAGE => {
            match delete_landing_page(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_MARK_CONTACT_READ => {
            match mark_contact_read(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #264 批 1：交付域五命令（三写两读；删除成功无载荷出 Null）──
        COMMAND_LIST_DELIVERY_INVOICES => {
            match list_delivery_invoices(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CREATE_INVOICE_FROM_LEAD => {
            match create_invoice_from_lead(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_UPDATE_INVOICE_STATUS => {
            match update_invoice_status(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_DELETE_DELIVERY_INVOICE => {
            match delete_delivery_invoice(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_DELIVERY_SUMMARY => match get_delivery_summary(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── #264 批 2：需求发现八命令（五写三读；删除平台成功无载荷出 Null）──
        COMMAND_LIST_PLATFORMS => match list_platforms(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_SAVE_PLATFORM => match save_platform(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_DELETE_PLATFORM => match delete_platform(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_LEADS => match list_leads(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GET_SCAN_POLICY => match get_scan_policy(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_SAVE_SCAN_POLICY => {
            match save_scan_policy(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_DISCOVER_AND_EVALUATE_LEADS => {
            match discover_and_evaluate_leads(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CREATE_LEAD => match create_lead(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── #264 批 3：需求实现三命令（状态机 / 转模板 / 启动执行）──
        COMMAND_UPDATE_LEAD_STATUS => {
            match update_lead_status(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CONVERT_LEAD_TO_WORKFLOW => {
            match convert_lead_to_workflow(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_RUN_LEAD_WORKFLOW => {
            match run_lead_workflow(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #264 批 4：领域工作流三只读命令（列表 / 详情 / 领域元数据）──
        COMMAND_LIST_DOMAIN_WORKFLOWS => {
            match list_domain_workflows(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_DOMAIN_WORKFLOW => {
            match get_domain_workflow(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_DOMAINS => match list_domains(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── #264 批 5：交付面六命令（执行 / 列表 / 详情 / 更新 / 重试 / 取消）──
        COMMAND_EXECUTE_DEMAND_WORKFLOW => {
            match execute_demand_workflow(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_DELIVERIES => match list_deliveries(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GET_DELIVERY => match get_delivery(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_UPDATE_DELIVERY => match update_delivery(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_RETRY_DELIVERY => match retry_delivery(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_CANCEL_DELIVERY => match cancel_delivery(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #264 批 6：能力发现面五命令。scan / proactive / analyze 零入参；list 的
        // `status` 可选；close 只收 `gapId`。
        COMMAND_SCAN_CAPABILITIES => match scan_capabilities(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_PROACTIVE_EVALUATE_LEADS => {
            match proactive_evaluate_leads(&request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_CAPABILITY_GAPS => {
            match list_capability_gaps(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CLOSE_CAPABILITY_GAP => {
            match close_capability_gap(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_ANALYZE_CAPABILITY_GAPS => {
            match analyze_capability_gaps(&request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 批 7 刀 A：订阅面三命令 + 域包配置面八命令（十一条全部一条接缝调用直投影，
        // 命令层只做形状判；订阅的业务校验在 dao，域包的 NotFound 在宿主实现体）。
        COMMAND_LIST_SUBSCRIPTIONS => match list_subscriptions(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_SAVE_SUBSCRIPTION => {
            match save_subscription(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_DELETE_SUBSCRIPTION => {
            match delete_subscription(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_INDUSTRIES => match list_industries(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GET_CAPABILITY_PACK_CONFIG => {
            match get_capability_pack_config(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_ACTION_CONFIG => {
            match get_action_config(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_CAPABILITY_PACK_ACTIONS => {
            match list_capability_pack_actions(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_CAPABILITY_PACK_WORKFLOWS => {
            match list_capability_pack_workflows(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_RUNTIME_INDUSTRIES => {
            match list_runtime_industries(&request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_KPI_DEFINITIONS => {
            match get_kpi_definitions(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_HAS_CAPABILITY_PACK => {
            match has_capability_pack(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 批 7 刀 B1：域包全读七命令（每条一次接缝调用直投影；命令层只做形状判，
        // 查无域包/工作流的 NotFound 在宿主实现体，校验语义在 company-runtime）。
        COMMAND_GET_WORKFLOW_CONFIG => {
            match get_workflow_config(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_VALIDATE_ENTITY => match validate_entity(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_BATCH_VALIDATE_ENTITIES => {
            match batch_validate_entities(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_COMPUTE_KPIS => match compute_kpis(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GET_CAPABILITY_PACK_DATA => {
            match get_capability_pack_data(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_CAPABILITY_PACK_PRECHECK => {
            match get_capability_pack_precheck(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_CAPABILITY_PACK_HEALTH => {
            match get_capability_pack_health(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 批 7 刀 B2：域包单据面十二命令（六读六写，每条一次接缝调用直投影；命令层只做
        // 形状判，非空 / 存在性 / 状态机合法性 / 目录可读全在宿主 provider）。
        COMMAND_GET_INVOICE => match get_invoice(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_UPDATE_INVOICE => match update_invoice(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GET_CUSTOMER => match get_customer(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_FIND_CUSTOMER_BY_EMAIL => {
            match find_customer_by_email(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_RECORD_KPI => match record_kpi(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_REVENUE => match list_revenue(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GET_INVESTMENT_ADVICE => {
            match get_investment_advice(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_IMPORT_TALENT_LIBRARY => {
            match import_talent_library(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_SYNC_FLEET => match sync_fleet(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GET_PUBLISH_SCHEDULE => {
            match get_publish_schedule(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_UPDATE_PUBLISH_SCHEDULE => {
            match update_publish_schedule(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_EXECUTE_DYNAMIC_WORKFLOW => {
            match execute_dynamic_workflow(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 刀 C 批 2：域包分析执行（读命令；风控阈值与数据服务装配在宿主 provider，
        // 命令层只做形状判与接缝调用）。
        COMMAND_EXECUTE_ANALYSIS => {
            match execute_analysis(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 刀 C 批 3：学习面七命令（六写一读，每条一次接缝调用直投影；学习引擎与
        // 域包适配器注册表的装配全在宿主 provider）。
        COMMAND_REFLECT_ON_WORKFLOW => {
            match reflect_on_workflow(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_EVOLVE_WORKFLOW => match evolve_workflow(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_RUN_SELF_IMPROVEMENT => {
            match run_self_improvement(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_TRIGGER_CAPABILITY_PACK_LEARNING => {
            match trigger_capability_pack_learning(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_RL_STATS => match get_rl_stats(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_TRIGGER_RL_OPTIMIZATION => {
            match trigger_rl_optimization(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_RECORD_RL_EXPERIENCE => {
            match record_rl_experience(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 批 4a：工作项七命令（六写一读，每条一次接缝调用直投影；状态机、质量门与
        // 自改进执行器全在宿主）。
        COMMAND_CREATE_WORK_ITEM => {
            match create_work_item(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_KANBAN_BOARD => match kanban_board(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_WORK_ITEM_START => match work_item_start(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_WORK_ITEM_REVIEW => {
            match work_item_review(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_ESCALATE_WORK_ITEM => {
            match escalate_work_item(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_WORK_ITEM_UNBLOCK => {
            match work_item_unblock(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_RUN_SELF_IMPROVING_WORK_ITEM => {
            match run_self_improving_work_item(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 批 7 刀 B3：域包自动规则 / 仪表盘四命令（三读一写，每条一次接缝调用直投影；
        // 命令层只做形状判，规则装配与仪表盘聚合全在宿主 provider）。
        COMMAND_GET_CAPABILITY_PACK_DASHBOARD => {
            match get_capability_pack_dashboard(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_CAPABILITY_PACK_WORKFLOW_STEPS => {
            match get_capability_pack_workflow_steps(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_CAPABILITY_PACK_AUTOMATION_RULES => {
            match get_capability_pack_automation_rules(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_RUN_AUTOMATION_RULES => {
            match run_automation_rules(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 批 7 刀 B3（后三条）：订阅扫描 / 扫描任务装配 / 能力匹配（每条一次接缝
        // 调用直投影；命令层只做形状判，业务语义全在宿主 provider）。
        COMMAND_RUN_SUBSCRIPTION_SCAN => {
            match run_subscription_scan(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_ENSURE_DEMAND_SCAN_JOB => {
            match ensure_demand_scan_job(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_MATCH_LEAD_CAPABILITIES => {
            match match_lead_capabilities(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 刀 B3 收尾：自动学习（一次接缝调用直投影；学习语义全在宿主 provider）。
        COMMAND_OPC_AUTO_LEARN_WORKFLOW => {
            match auto_learn_workflow(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // #264 刀 C 批 1：域包 manifest / prompt 装配 + 学习面五条读命令（每条一次接缝调用
        // 直投影；配置读取与 `{{input}}` 装配全在宿主 provider，命令层只做形状判与键名转写）。
        COMMAND_GET_CAPABILITY_PACK => {
            match get_capability_pack(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_BUILD_CAPABILITY_PACK_PROMPT => {
            match build_capability_pack_prompt(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_LEARNING_METRICS => {
            match get_learning_metrics(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_LEARNING_CONFIG => {
            match get_learning_config(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_LEARNING_CONFIGS => {
            match list_learning_configs(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CREATE_PROJECT => match create_project(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_ADD_MILESTONE => match add_milestone(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_CREATE_LANDING_PAGE => {
            match create_landing_page(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_FINANCIAL_REPORT => {
            match get_financial_report(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CREATE_CONTENT_ASSET => {
            match create_content_asset(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CREATE_PUBLISH_SCHEDULE => {
            match create_publish_schedule(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_PROCESS_DUE_SCHEDULES => match process_due_schedules(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 看板读数：宿主没有聚合 op，由本进程组合三次列表查询现算。
        COMMAND_GET_DASHBOARD => match dashboard(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 看板总览：走宿主整表聚合 op（与上面「首页现算」不同口径，两者不能互换）。
        COMMAND_GET_DASHBOARD_SUMMARY => match dashboard_summary(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        other => CommandOutcome::Fault(CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_UNKNOWN,
            format!("OPC 插件没有命令 `{other}` 的实现"),
        )),
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

/// 握手声明：`calls` 是**出站**准入（宿主 `handle_call_seam` 会校验本插件是否声明过目标接缝），
/// 少写一条 = 运行时被拒。`capabilities` 仍为空：本插件不向宿主提供任何接缝 —— `opc.store`
/// 是「宿主提供给插件」的查询接缝，方向相反，不进 `SUPPORTED_REMOTE_SEAMS`。
fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec![OPC_STORE_SEAM.to_string()],
            capabilities: Vec::new(),
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// `opc_list_blog_posts`：博客文章列表，出参是行对象数组。
///
/// ## 为什么要在这里分页取尽
///
/// 它替代的宿主命令一次返回**整表**（同进程 IPC、不过帧、无上限）；接缝侧要过帧，故契约给
/// `limit` 上了区间闸（`OPC_BLOG_POST_LIST_LIMIT_RANGE`）。于是「调用方没指定分页」= 宿主语义
/// 的整表 ⇒ 按接缝回带的 `total` 翻页直到取尽。不这么做的后果不是「少一帧数据」而是**静默少
/// 计数**：`DashboardTab` 的 `blog_posts_count` 取的就是本数组的 `length`，一页封顶会让它把
/// 137 篇显示成 50 篇。
///
/// 给了 `limit` / `offset` 则**原样透传、只取一页**（插件不钳位：越界由宿主按契约区间拒收，
/// 缺省档也交给宿主 ⇒ 本进程里没有第二个 50 / 200，同 `plugin-stock` 的
/// `stock_analysis_records` 判据）。
///
/// ## 出参线格式 = 接缝视图逐字 camelCase（禁区 13，无例外）
///
/// 出参就是接缝视图 `BlogPostView`（契约：harness `src/opc_store.rs`，该结构体带
/// `#[serde(rename_all = "camelCase")]`）的回帧键名，本进程**不做任何键名改写**：
/// `publishedAt` / `viewCount` / `createdAt` / `updatedAt` / `excerpt` / `slug` / `tags` /
/// `title` / `id` / `published`。前端消费方（`src/pages/opc/components/SitesTab.tsx` 的
/// `interface BlogPost` 与表格列 `dataIndex: "viewCount"`）与这套键名逐字对齐 —— 键名在
/// 整条链上只有一套。
///
/// 这里曾有的一层 camelCase → snake_case 逐字段映射（复刻被删宿主 DTO
/// `company-runtime::BlogPost` 的蛇形线格式）已删除 —— 那是禁区 13 全站 camelCase 约定的一处
/// 刻意偏离，迁移初期为了「本批只换通道、不动前端」而临时保留，#167 连同那两个前端消费面
/// 一起收口。新增命令**不许**再建这种出口适配层：接缝给什么就交什么。
///
/// 唯一仍在出口做的形状约束是**字段集合不扩容**：出参逐字 = 接缝回帧里的行对象，接缝哪天
/// 多给一个字段，它会直接出现在出参里 —— 由 `tests/protocol.rs` 与本文件的键集合用例逐字
/// 钉住，扩容属于契约变更，要走评审而不是静默发生。
///
/// ## 少掉的字段
///
/// `content`（博客正文）不过线 —— 裁剪做在 SQL 侧（判据与两个消费方的实证见契约模块头）。
/// 现网无人读列表里的正文；真要读正文走宿主详情面，不属本命令。
fn blog_posts(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let explicit_limit = input.get("limit").filter(|v| !v.is_null()).cloned();
    let explicit_offset = input.get("offset").filter(|v| !v.is_null()).cloned();

    if explicit_limit.is_some() || explicit_offset.is_some() {
        let mut args = serde_json::Map::new();
        if let Some(limit) = explicit_limit {
            args.insert("limit".to_string(), limit);
        }
        if let Some(offset) = explicit_offset {
            args.insert("offset".to_string(), offset);
        }
        let page =
            call_opc_store(peer, OPC_STORE_LIST_BLOG_POSTS, Value::Object(args), call_chain)?;
        // 逐字透传接缝视图（键名 camelCase，禁区 13）：出口不再有第二套键名。
        return Ok(Value::Array(post_rows_of(&page)));
    }

    // 整表语义：offset 按**实际取回的条数**推进（不是按请求的 limit 推进）——最后一页本来就短。
    let mut rows: Vec<Value> = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let page = call_opc_store(
            peer,
            OPC_STORE_LIST_BLOG_POSTS,
            json!({ "offset": offset }),
            call_chain,
        )?;
        let posts = post_rows_of(&page);
        // 两个终止条件都要：`total` 只在该次调用内自洽（并发写入下可能小于已取行数），
        // 而空页是越界 offset 的合法回帧（契约：空数组不是错误）—— 只看 `total` 会吊死。
        let total = page.get("total").and_then(Value::as_u64).unwrap_or(0);
        let fetched = posts.len();
        rows.extend(posts);
        offset += fetched as u64;
        if fetched == 0 || rows.len() as u64 >= total {
            break;
        }
    }
    Ok(Value::Array(rows))
}

/// 发一次 `opc.store` 的帧（op 由调用方给），把宿主侧的失败落成命令失败。
///
/// 接缝报错 ⇒ **原样透传错误码**，不退化成空列表 / 假成功：空数组在页面上长得和
/// 「一篇都没有」一样，而「发布失败」被吞掉更直接 —— 用户看见「已发布」但库里没动。
///
/// 读面与写面共用这一条（只差 `op` 与 `args` 两个参数）：两份「解 `ResponseKind`、
/// 缺码兜底、缺文案兜底」的实现对齐在同一处才不会漂移（禁区 12）。
fn call_opc_store(
    peer: &mut WorkerPeer,
    op: &str,
    args: Value,
    call_chain: &[String],
) -> Result<Value, (String, String)> {
    let reply = peer.call_seam(OPC_STORE_SEAM, op, args, call_chain.to_vec()).map_err(|e| {
        (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("OPC 站点接缝调用失败：{e}"))
    })?;
    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "OPC 站点接缝返回错误但未说明".to_string()),
        )),
    }
}

/// `opc_publish_blog_post`：发布一篇博客（本插件的写面之一）。
///
/// ## 入参只有 `id`，且只判形状
///
/// 命令层只看 `id` 在不在、是不是字符串（不是 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，**不发帧**）；
/// 「非空」这条语义闸留在宿主 provider（`opc_store_service.rs` 的 `require_publish_id`）。
/// 分两层各判一半会漂移，故刻意分工成「形状 / 语义」而不是两处都判空白。
/// 正文 `content` 从来不是本命令的入参 —— 接缝那侧也没读它。
///
/// ## 出参 = 接缝的两态枚举串，逐字透传
///
/// `"published"` / `"not_found"`（契约：`PublishBlogPostOutcome`，`#[serde(rename_all =
/// "snake_case")]`）。这里**不建包装对象、不改取值、不建适配层**（禁区 13 那条判据同样
/// 管枚举）：「这一篇不在表里」是**正常结果**，插件侧不许把它翻成错误码 —— 翻了就等于让
/// 前端分不出「发布失败」与「本来就没有这篇」。
fn publish_blog_post(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            "opc_publish_blog_post 需要字符串 id（形状判在命令层，非空判在宿主 provider）"
                .to_string(),
        ));
    };
    // 出站帧的 args 只有一个键 `id` —— 逐字与接缝 op 的入参同名，不套 `request` 外层
    // （那是 `knowledge.vector` / `stock.monitor` 的嵌套 DTO 形态；这里没有 DTO）。
    call_opc_store(peer, OPC_STORE_PUBLISH_BLOG_POST, json!({ "id": id }), call_chain)
}

/// 接缝回帧里的 `posts` 数组（缺键 / 非数组 ⇒ 空页，**不** panic）。
///
/// 它交出的行**原样**成为命令出参：键名 = 接缝视图的 camelCase，出口不做任何改写
/// （判据见 [`blog_posts`] 的「出参线格式」一节）。
fn post_rows_of(page: &Value) -> Vec<Value> {
    page.get("posts").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `opc_delete_blog_post`：删除一篇博客（#180-① 的写 op）。
///
/// ## 入参只有 `id`，且只判形状
///
/// 与 [`publish_blog_post`] 逐条同构：命令层只看 `id` 在不在、是不是字符串
/// （不是 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，**不发帧**）；「非空」这条语义闸留在宿主
/// provider（`init/opc_store_service.rs` 的 `require_blog_post_id`）。正文不是入参。
///
/// ## 出参 = 接缝的三态枚举串，逐字透传
///
/// `"removed"` / `"not_found"` / `"protected"`（契约：`RemoveBlogPostOutcome`，
/// `#[serde(rename_all = "snake_case")]`，与 `jobs.store` 的 `remove` 同一套取值）。
/// **不建包装对象、不改取值、不建适配层**（禁区 13 那条判据同样管枚举）：
/// 「本来就没有这篇」与「这篇受保护、不许删」都是**正常结果**，插件侧不许把它们翻成错误码 ——
/// 翻了就等于让前端分不出「删失败」「本来没有」「不许删」三件事。
fn delete_blog_post(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            "opc_delete_blog_post 需要字符串 id（形状判在命令层，非空判在宿主 provider）"
                .to_string(),
        ));
    };
    // 出站帧的 args 只有一个键 `id` —— 与接缝 op 的入参同名，不套 `request` 外层。
    call_opc_store(peer, OPC_STORE_DELETE_BLOG_POST, json!({ "id": id }), call_chain)
}

/// `opc_create_blog_post`：新建一篇博客（#180-② 的写 op）。
///
/// ## 入参形状：平铺的五个 DTO 字段（判形状，不判语义）
///
/// 清单 `inputSchema` 声明 `title` / `slug` / `excerpt` / `content` / `tags` 五个属性，
/// 本层只验**形状**（前四个是字符串、`tags` 是字符串数组）。语义闸（`slug` 空、正文超
/// 字节预算）全在宿主 provider —— 两处各判一半必然漂移，且字节预算的数字是对接缝的约定，
/// 不该在进程侧再抄一份（`MAX_FRAME_LEN` 的 64 MiB 是帧层上限，**不是**本 op 的预算）。
///
/// ## 出站形状：嵌一层的 `{"request": {…}}`
///
/// 这是一个 DTO（不是散标量），照 `knowledge.vector` 的 `request` 形态；宿主侧
/// `deserialize_arg(&args, "request", seam)` 只认这一种，平铺会在宿主落
/// `SEAM_CALL_INVALID_ARGS`。多余键不转发（宿主 serde 本来也会忽略未知字段，本层给它同一条口径）。
///
/// ## 出参 = 创建的**列裁剪视图**（逐字）
///
/// 与列表行同一套 10 个 camelCase 键、同样不含正文。带 `id` 回来是关键：调用方拿它做后续
/// 发布 / 删除的句柄（#180-④ 的教训 —— 让前端自己造 id 会落下悬空引用）。
fn create_blog_post(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let request = blog_post_create_request(input)?;
    call_opc_store(peer, OPC_STORE_CREATE_BLOG_POST, json!({ "request": request }), call_chain)
}

/// 命令层形状闸：从平铺入参里挑出 `CreateBlogPostInput` 的五个字段，重建一个**只含这五个键**
/// 的对象交给接缝。
///
/// 判据分层与 `publish` / `delete` 同一条：这里只判「键在不在、类型对不对」，不判语义
/// （空串 / 超预算都放行到宿主 —— 那是 provider 的闸）。
fn blog_post_create_request(input: &Value) -> Result<Value, (String, String)> {
    for key in ["title", "slug", "excerpt", "content"] {
        if input.get(key).and_then(Value::as_str).is_none() {
            return Err(invalid_shape(format!(
                "opc_create_blog_post 需要平铺的字符串字段 `{key}`\
                 （形状判在命令层，非空 / 字节预算判在宿主 provider）"
            )));
        }
    }
    let tags = match input.get("tags") {
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            Value::Array(items.clone())
        },
        _ => {
            return Err(invalid_shape(
                "opc_create_blog_post 的 `tags` 必须是字符串数组（形状判在命令层）".to_string(),
            ));
        },
    };
    Ok(json!({
        "title": input["title"],
        "slug": input["slug"],
        "excerpt": input["excerpt"],
        "content": input["content"],
        "tags": tags,
    }))
}

/// 命令层形状错误（`PLUGIN_COMMAND_INVALID_ARGS`）：**不发帧**，也就不存在「注定失败的帧」。
fn invalid_shape(message: String) -> (String, String) {
    (error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(), message)
}

// ── #36 批 1：只读查询命令 ──────────────────────────────────────────────

/// 四条带过滤条件的命令各自认的入参键（camelCase，与契约 `*ListQuery` 的 serde 键逐字一致）。
/// 只挑这些键转发：多余键不进门（同 [`blog_post_create_request`] 的口径）。`list_customers`
/// 刻意不含 `tags` —— 域服务不消费该过滤位，契约也不收（见 `opc_store.rs` 的 `CustomerListQuery`）。
const QUERY_KEYS_INVOICES: &[&str] = &["status", "customerId", "dateFrom", "dateTo", "limit"];
const QUERY_KEYS_CUSTOMERS: &[&str] = &["status", "customerType", "search", "limit"];
const QUERY_KEYS_PROJECTS: &[&str] = &["status", "customerId", "search", "limit"];
const QUERY_KEYS_KPIS: &[&str] = &["period", "limit"];

/// 只读列表命令的公共出口：把 agent 侧传来的 camelCase 键**原样**装进 `request`，出参逐字
/// 透传接缝回帧（裸数组，camelCase 键，零键名改写 —— 同 [`blog_posts`] 的「出参线格式」）。
///
/// - `keys` 为空 ⇒ 该 op 无入参，直接发空入参（不塞 `request` 空壳）；
/// - 缺失的键**不塞默认值、不塞 null**：契约 DTO 是 `default`，缺键即 `None`，塞 null 会
///   多造一个「调用方显式传了 null」的假信号。
fn list_rows(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
    op: &str,
    keys: &[&str],
) -> Result<Value, (String, String)> {
    if keys.is_empty() {
        return call_opc_store(peer, op, json!({}), call_chain);
    }
    let mut request = serde_json::Map::new();
    for key in keys {
        // 缺键与 null 都不转发（同宿主工具侧 `filter_request` 的口径）。
        if let Some(value) = input.get(*key).filter(|value| !value.is_null()) {
            request.insert((*key).to_string(), value.clone());
        }
    }
    call_opc_store(peer, op, json!({ "request": request }), call_chain)
}

/// 回帧里的行数组（缺键 / 非数组 ⇒ 空数组，**不** panic）：空表 = 空数组不是错误。
fn rows_of(value: &Value) -> &[Value] {
    match value.as_array() {
        Some(rows) => rows,
        None => &[],
    }
}

/// 行对象里某个字符串字段（缺键 / 非字符串 ⇒ `None`，调用方按「不匹配」处理）。
fn row_str<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

/// `opc_get_dashboard`：仪表盘六项读数，由本进程组合三次只读列表查询现算。
///
/// 宿主**不**新增聚合 op（同一条 `opc.store` 数据面只开列表读面）：三张表各取一页
/// `limit: 100` 后按视图字段现算 —— **超出这一页的行不计入聚合**（这是本命令的取值边界，
/// 不是宿主 provider 的缺省）。空表 ⇒ 各项为 0。
fn dashboard(call_chain: &[String], peer: &mut WorkerPeer) -> Result<Value, (String, String)> {
    let invoices = call_opc_store(
        peer,
        OPC_STORE_LIST_INVOICES,
        json!({ "request": { "limit": 100 } }),
        call_chain,
    )?;
    let customers = call_opc_store(
        peer,
        OPC_STORE_LIST_CUSTOMERS,
        json!({ "request": { "limit": 100 } }),
        call_chain,
    )?;
    let projects = call_opc_store(
        peer,
        OPC_STORE_LIST_PROJECTS,
        json!({ "request": { "limit": 100 } }),
        call_chain,
    )?;

    let invoice_rows = rows_of(&invoices);
    let customer_rows = rows_of(&customers);
    let project_rows = rows_of(&projects);

    let total_revenue: f64 = invoice_rows
        .iter()
        .filter(|row| row_str(row, "status") == Some("paid"))
        .filter_map(|row| row.get("total").and_then(Value::as_f64))
        .sum();
    let pending_invoices = invoice_rows
        .iter()
        .filter(|row| matches!(row_str(row, "status"), Some("draft" | "sent" | "overdue")))
        .count();
    let active_projects = project_rows
        .iter()
        .filter(|row| matches!(row_str(row, "status"), Some("active" | "planning")))
        .count();
    let active_customers =
        customer_rows.iter().filter(|row| row_str(row, "status") == Some("active")).count();

    // 键名 camelCase：宿主工具按这套键格式化，改一个字母就会静默读空。
    Ok(json!({
        "totalRevenue": total_revenue,
        "pendingInvoices": pending_invoices,
        "activeProjects": active_projects,
        "activeCustomers": active_customers,
        "totalCustomers": customer_rows.len(),
        "totalProjects": project_rows.len(),
    }))
}

/// `opc_get_dashboard_summary`：看板总览（零入参）。
///
/// 与 [`dashboard`] 的区别：`dashboard` 由本进程组合三次 `limit: 100` 列表查询现算
/// （智能体工具 `opc_dashboard` 消费它的六键），超出首页的行不计入；本命令走宿主
/// `dashboard_summary` op，宿主按**整表**聚合，出参与迁移前宿主命令逐字段同形。
fn dashboard_summary(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_DASHBOARD_SUMMARY, json!({}), call_chain)
}

// ── #36 批 2：四条写命令 ────────────────────────────────────────────────

/// `opc_transition_invoice`：变更发票状态（写 op）。
///
/// ## 入参只判形状
///
/// `id` / `status` 都必须在且是字符串（不是 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，**不发帧**）。
/// 语义闸全在宿主 provider：`status` 的枚举合法性（非法串 ⇒ `SEAM_CALL_INVALID_ARGS`）与
/// 状态转换合法性（非法转换 ⇒ `PLUGIN_CALL_FAILED`）。
///
/// ## 出站形状：`{"id", "target"}`
///
/// 契约 op 的第二个键叫 `target`（与域服务 `transition_status(id, target)` 同名）；命令面
/// 对外用 `status`（与清单 inputSchema / 智能体工具入参一致）⇒ 改名只发生在**本函数**。
///
/// ## 出参 = `{"transitioned": {…}}` / `"not_found"`，逐字透传
fn transition_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_transition_invoice 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    let Some(status) = input.get("status").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_transition_invoice 需要字符串 status（合法性判在宿主 provider）".to_string(),
        ));
    };
    call_opc_store(
        peer,
        OPC_STORE_TRANSITION_INVOICE,
        json!({ "id": id, "target": status }),
        call_chain,
    )
}

/// `opc_update_content_asset`：更新内容资产（写 op）。
///
/// 入参是平铺的 `id` + 五个可选字段；出站是 `{"id", "request": {…}}`（`request` 是一个
/// DTO，必须嵌层 —— 与 `create_blog_post` 同一条判据）。缺键 = 该列不动。
fn update_content_asset(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_update_content_asset 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    let request = content_asset_update_request(input)?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_CONTENT_ASSET,
        json!({ "id": id, "request": request }),
        call_chain,
    )
}

/// 命令层形状闸：从平铺入参里挑出 `UpdateContentAssetInput` 的五个字段。
///
/// 判据同 [`blog_post_create_request`]：只判「键在不在、类型对不对」，不判语义（空串 /
/// 非法状态串都放行到宿主 —— 域服务与契约各自收口）。`contentType` 是 camelCase 线格式
/// （禁区 13），本层不做 snake_case 兼容翻译。
fn content_asset_update_request(input: &Value) -> Result<Value, (String, String)> {
    let mut request = serde_json::Map::new();
    for key in ["title", "contentType", "body", "status"] {
        match input.get(key) {
            None | Some(Value::Null) => {},
            Some(Value::String(value)) => {
                request.insert(key.to_string(), Value::String(value.clone()));
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "opc_update_content_asset 的 `{key}` 必须是字符串（形状判在命令层）"
                )));
            },
        }
    }
    match input.get("tags") {
        None | Some(Value::Null) => {},
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            request.insert("tags".to_string(), Value::Array(items.clone()));
        },
        _ => {
            return Err(invalid_shape(
                "opc_update_content_asset 的 `tags` 必须是字符串数组（形状判在命令层）".to_string(),
            ));
        },
    }
    Ok(Value::Object(request))
}

/// `opc_delete_content_asset`：删除内容资产（写 op，两态出参逐字透传）。
fn delete_content_asset(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_delete_content_asset 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    call_opc_store(peer, OPC_STORE_DELETE_CONTENT_ASSET, json!({ "id": id }), call_chain)
}

/// `opc_cancel_publish_schedule`：取消发布计划（写 op，**硬删**；两态出参逐字透传）。
fn cancel_publish_schedule(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_cancel_publish_schedule 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    call_opc_store(peer, OPC_STORE_CANCEL_PUBLISH_SCHEDULE, json!({ "id": id }), call_chain)
}

// ── #36 批 3：八条写命令 + 一条只读报表 ────────────────────────────────
//
// 判据与批 2 同一条：**命令层只判形状**（缺必需键 / 类型不对 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，
// **不发帧**），语义与闸全在宿主 provider（空串 / 非法枚举 / 缺行由它各自收口）。出参是契约
// 的列裁剪视图（或视图数组），**逐字透传** —— 本层不做任何键名改写或业务判断。
//
// 入参键一律 camelCase（禁区 13，与契约 DTO 的 serde 线格式同源）。数组字段（`lineItems` /
// `tags`）只判「是不是对象数组 / 字符串数组」，元素内部字段类型的错由宿主 serde 拒收
// （`SEAM_CALL_INVALID_ARGS`）—— 逐元素逐字段再判一遍只会把同一件事判两次。

/// `opc_create_invoice`：创建发票（写 op）。
///
/// 行项目的 `total` **不在这里算**：由宿主 provider 按 `quantity × unitPrice × (1 + taxRate)`
/// 现算（迁移前工具在本进程算，上收后单价/税率的缺省口径只有一个落点）。
fn create_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_invoice";
    let customer_id = require_str(input, OP, "customerId")?;
    let line_items = object_array(input, OP, "lineItems")?;
    let mut request = serde_json::Map::new();
    request.insert("customerId".to_string(), Value::String(customer_id.to_string()));
    request.insert("lineItems".to_string(), line_items);
    copy_optional_strings(input, OP, &mut request, &["currency", "notes"])?;
    copy_optional_i64(input, OP, &mut request, &["dueAt"])?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_INVOICE,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_create_customer`：创建客户（写 op）。
///
/// `customerType` / `source` 收**串**：未知值分别由宿主落 `Unknown` / `Other(原串)`，
/// 不在此处把枚举抄第二遍（迁移前工具的映射原样下沉到 provider）。
fn create_customer(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_customer";
    let name = require_str(input, OP, "name")?;
    let email = require_str(input, OP, "email")?;
    let mut request = serde_json::Map::new();
    request.insert("name".to_string(), Value::String(name.to_string()));
    request.insert("email".to_string(), Value::String(email.to_string()));
    copy_optional_strings(
        input,
        OP,
        &mut request,
        &[
            "phone",
            "company",
            "customerType",
            "country",
            "region",
            "city",
            "address",
            "source",
            "notes",
        ],
    )?;
    copy_optional_f64(input, OP, &mut request, &["latitude", "longitude"])?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_CUSTOMER,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_update_customer`：更新客户（#32 批 A 的写 op）。
///
/// `id` 必填（散标量）；正文平铺可选字段组装 `{"request": {…}}`。可选字段缺键 /
/// `null` 都不转发（helper 口径）⇒ 接缝 DTO 外层 `None` = 不动；给了值 = 写入。与
/// 接缝注释一致：**清空（域包 `Some(None)`）在本命令的 JSON 线格式上不可表达**，与
/// 既存宿主 Tauri 命令行为完全一致。出参 = 更新后的 [`CustomerSummaryView`]。
fn update_customer(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_customer";
    let id = require_str(input, OP, "id")?;
    let mut request = serde_json::Map::new();
    copy_optional_strings(
        input,
        OP,
        &mut request,
        &[
            "name",
            "email",
            "phone",
            "company",
            "customerType",
            "country",
            "region",
            "city",
            "address",
            "notes",
        ],
    )?;
    copy_optional_f64(input, OP, &mut request, &["latitude", "longitude"])?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_CUSTOMER,
        json!({ "id": id, "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_delete_customer`：删除一个客户（#32 批 A 的写 op）。
///
/// 入参只有 `id`，且只判形状（非空判在宿主 provider，同 [`publish_blog_post`] 的分工）。
/// 成功无载荷（接缝出 `Value::Null`，逐字透传）。
fn delete_customer(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_customer";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_CUSTOMER, json!({ "id": id }), call_chain)
}

/// `opc_delete_invoice`：删除一张**域包发票**（#32 批 A 的写 op）。
///
/// ⚠ 与宿主同名 Tauri 命令**同名不同义**：宿主命令删的是交付域发票
/// （`opc_delivery` repo），本命令删的是 `opc_invoices` 表的域包发票 ——
/// 前端发票页两套体系并存时靠命令通道区分。入参只有 `id`（只判形状）。
fn delete_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_invoice";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_INVOICE, json!({ "id": id }), call_chain)
}

/// `opc_get_project`：取项目详情（#32 批 B 的读 op，宿主侧无写权限闸）。
///
/// 入参只有 `id`（散标量，不套 `request` 壳）；出参 = 接缝 `ProjectDetailView`
/// 逐字透传（摘要列 + 描述 / 备注 / 里程碑明细，全部 camelCase，出口零改写）。
fn get_project(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_project";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_GET_PROJECT, json!({ "id": id }), call_chain)
}

/// `opc_update_project`：更新项目（#32 批 B 的写 op）。
///
/// `id` 必填（散标量）；可选字段平铺组装 `{"request": {…}}`，缺键 / `null` 都不转发
/// ⇒ 接缝 DTO 外层 `None` = 该列不动。⚠ `customerId` / `currency` 刻意不转发：域包
/// `UpdateProjectInput` 没有这两个更新位（转发等于宣称「改了」，实际静默不生效）。
fn update_project(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_project";
    let id = require_str(input, OP, "id")?;
    let mut request = serde_json::Map::new();
    copy_optional_strings(input, OP, &mut request, &["title", "description", "notes"])?;
    copy_optional_f64(input, OP, &mut request, &["budget"])?;
    copy_optional_i64(input, OP, &mut request, &["deadline"])?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_PROJECT,
        json!({ "id": id, "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_delete_project`：删除一个项目（#32 批 B 的写 op）。
///
/// 入参只有 `id`（只判形状，非空判在宿主 provider，同 [`delete_customer`] 的分工）；
/// 成功无载荷（接缝出 `Value::Null`，逐字透传）。
fn delete_project(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_project";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_PROJECT, json!({ "id": id }), call_chain)
}

/// `opc_complete_milestone`：把项目里的一个里程碑标成完成（#32 批 B 的写 op）。
///
/// 两个散标量（`projectId` / `milestoneId`）；出参 = 完成后的详情视图（前端据此原地
/// 刷新时间线）。`milestoneId` 必须在项目明细里命中（宿主域服务否则回 `NotFound`）。
fn complete_milestone(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_complete_milestone";
    let project_id = require_str(input, OP, "projectId")?;
    let milestone_id = require_str(input, OP, "milestoneId")?;
    call_opc_store(
        peer,
        OPC_STORE_COMPLETE_MILESTONE,
        json!({ "projectId": project_id, "milestoneId": milestone_id }),
        call_chain,
    )
}

/// `opc_publish_landing_page`：发布落地页（#36 D-4 刀的写 op）。
///
/// 入参只有 `id`（只判形状，非空判在宿主 provider，同 [`delete_project`] 的分工）；
/// 出参 = 发布后的摘要视图逐字透传（`published` 已翻转，全部 camelCase，出口零改写）。
fn publish_landing_page(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_publish_landing_page";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_PUBLISH_LANDING_PAGE, json!({ "id": id }), call_chain)
}

/// `opc_delete_landing_page`：删除落地页（#36 D-4 刀的写 op）。
///
/// 入参只有 `id`；成功无载荷（接缝出 `Value::Null`，逐字透传）。
fn delete_landing_page(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_landing_page";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_LANDING_PAGE, json!({ "id": id }), call_chain)
}

/// `opc_mark_contact_read`：把联系提交标成已读（#36 D-4 刀的写 op）。
///
/// 入参只有 `id`；成功无载荷（接缝出 `Value::Null`，逐字透传）。
fn mark_contact_read(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_mark_contact_read";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_MARK_CONTACT_READ, json!({ "id": id }), call_chain)
}

// ── #264 批 1：交付域五命令 ────────────────────────────────────────────────
//
// 与域包发票命令**同名不同义**：这里只碰 `lead_id IS NOT NULL` 的交付来源行。
// 命令面入参 camelCase、接缝面 snake_case（三面键名分档）；出参一律宿主原样回帧
// （视图字段由契约 DTO 的 `serde(rename_all = "camelCase")` 决定，本层零改写）。

/// `opc_list_delivery_invoices`：交付发票列表（只读 op）。
///
/// `status` 可选；缺键 / `null` 不转发（宿主按「不过滤」处理）。
fn list_delivery_invoices(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_delivery_invoices";
    let mut args = serde_json::Map::new();
    match input.get("status") {
        None | Some(Value::Null) => {},
        Some(Value::String(status)) => {
            args.insert("status".to_string(), Value::String(status.clone()));
        },
        Some(_) => return Err(invalid_shape(format!("{OP} 的 `status` 必须是字符串"))),
    }
    call_opc_store(peer, OPC_STORE_LIST_DELIVERY_INVOICES, Value::Object(args), call_chain)
}

/// `opc_create_invoice_from_lead`：won 线索开票（写 op，宿主侧幂等 + 状态前置校验）。
fn create_invoice_from_lead(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_invoice_from_lead";
    let lead_id = require_str(input, OP, "leadId")?;
    let mut args = serde_json::Map::new();
    args.insert("lead_id".to_string(), Value::String(lead_id.to_string()));
    // 可选 `input` 对象原样透传（`CreateInvoiceFromLeadInput` 是 camelCase 契约 DTO，
    // 缺省字段由宿主实现体从线索元数据补齐）。
    match input.get("input") {
        None | Some(Value::Null) => {},
        Some(value @ Value::Object(_)) => {
            args.insert("input".to_string(), value.clone());
        },
        Some(_) => return Err(invalid_shape(format!("{OP} 的 `input` 必须是对象"))),
    }
    call_opc_store(peer, OPC_STORE_CREATE_INVOICE_FROM_LEAD, Value::Object(args), call_chain)
}

/// `opc_update_invoice_status`：推进交付发票状态机（写 op）。
fn update_invoice_status(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_invoice_status";
    let invoice_id = require_str(input, OP, "invoiceId")?;
    let status = require_str(input, OP, "status")?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_DELIVERY_INVOICE_STATUS,
        json!({ "invoice_id": invoice_id, "status": status }),
        call_chain,
    )
}

/// `opc_delete_delivery_invoice`：删除交付发票（写 op，硬删）。
fn delete_delivery_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_delivery_invoice";
    let id = require_str(input, OP, "invoiceId")?;
    call_opc_store(peer, OPC_STORE_DELETE_DELIVERY_INVOICE, json!({ "id": id }), call_chain)
}

/// `opc_get_delivery_summary`：交付汇总（只读 op，零入参）。
fn get_delivery_summary(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_DELIVERY_SUMMARY, json!({}), call_chain)
}

// ── #264 批 2：需求发现八命令 ───────────────────────────────────────────
//
// 三面键名分档同既有批：命令面 camelCase（`minScore` / `input`），接缝面 snake_case
// （`min_score`）。命令层只做形状判（缺键/类型不对即拒），业务校验（空串、非法枚举）
// 一律在宿主实现体 —— 与迁移前的 Tauri 命令分层一致。

/// `opc_list_platforms`：列出需求平台配置（只读 op，零入参）。
fn list_platforms(call_chain: &[String], peer: &mut WorkerPeer) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_LIST_PLATFORMS, json!({}), call_chain)
}

/// `opc_save_platform`：保存平台配置（写 op，`input` 必填对象）。
fn save_platform(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_save_platform";
    let payload = require_object(input, OP, "input")?;
    call_opc_store(peer, OPC_STORE_SAVE_PLATFORM, json!({ "input": payload }), call_chain)
}

/// `opc_delete_platform`：删除平台配置（写 op，硬删，成功无载荷）。
fn delete_platform(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_platform";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_PLATFORM, json!({ "id": id }), call_chain)
}

/// `opc_list_leads`：线索列表（只读 op，三个可选过滤项）。
fn list_leads(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_leads";
    let mut args = serde_json::Map::new();
    match input.get("limit") {
        None | Some(Value::Null) => {},
        Some(value) if value.as_u64().is_some() => {
            args.insert("limit".to_string(), value.clone());
        },
        Some(_) => {
            return Err(invalid_shape(format!("{OP} 的 `limit` 必须是非负整数（形状判在命令层）")));
        },
    }
    // 命令面 `minScore` ⇒ 接缝面 `min_score`（三面键名分档）。
    match input.get("minScore") {
        None | Some(Value::Null) => {},
        Some(value) if value.as_f64().is_some() => {
            args.insert("min_score".to_string(), value.clone());
        },
        Some(_) => {
            return Err(invalid_shape(format!("{OP} 的 `minScore` 必须是数值（形状判在命令层）")));
        },
    }
    copy_optional_strings(input, OP, &mut args, &["status"])?;
    call_opc_store(peer, OPC_STORE_LIST_LEADS, Value::Object(args), call_chain)
}

/// `opc_get_scan_policy`：读扫描策略（只读 op，零入参）。
fn get_scan_policy(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_GET_SCAN_POLICY, json!({}), call_chain)
}

/// `opc_save_scan_policy`：存扫描策略（写 op，`policy` 必填对象；范围钳制在宿主侧）。
fn save_scan_policy(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_save_scan_policy";
    let policy = require_object(input, OP, "policy")?;
    call_opc_store(peer, OPC_STORE_SAVE_SCAN_POLICY, json!({ "policy": policy }), call_chain)
}

/// `opc_discover_and_evaluate_leads`：扫描并评估需求线索（写 op，落库）。
fn discover_and_evaluate_leads(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_discover_and_evaluate_leads";
    let query = require_str(input, OP, "query")?;
    call_opc_store(
        peer,
        OPC_STORE_DISCOVER_AND_EVALUATE_LEADS,
        json!({ "query": query }),
        call_chain,
    )
}

/// `opc_create_lead`：手动补录线索（写 op，`input` 必填对象）。
fn create_lead(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_lead";
    let payload = require_object(input, OP, "input")?;
    call_opc_store(peer, OPC_STORE_CREATE_LEAD, json!({ "input": payload }), call_chain)
}

/// `opc_update_lead_status`：线索生命周期状态机（写 op，`leadId` + `status` 必填）。
///
/// 迁移判定（合法迁移 / 终态 / 幂等）全留在宿主 DAO 层，命令层只判形状 ——
/// 与 `publish_blog_post` 的「形状 / 语义」分工同口径。
fn update_lead_status(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_lead_status";
    let lead_id = require_str(input, OP, "leadId")?;
    let status = require_str(input, OP, "status")?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_LEAD_STATUS,
        json!({ "lead_id": lead_id, "status": status }),
        call_chain,
    )
}

/// `opc_convert_lead_to_workflow`：把线索转成实现工作流模板（写 op，`leadId` 必填）。
///
/// 出参是新建的 `WorkflowTemplateData`（含 `id`）；「已转化」由宿主 provider 判并报错。
fn convert_lead_to_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_convert_lead_to_workflow";
    let lead_id = require_str(input, OP, "leadId")?;
    call_opc_store(
        peer,
        OPC_STORE_CONVERT_LEAD_TO_WORKFLOW,
        json!({ "lead_id": lead_id }),
        call_chain,
    )
}

/// `opc_run_lead_workflow`：启动线索实现工作流的执行（写 op，`leadId` 必填）。
///
/// 出参是执行 id 字符串（接缝出 JSON string，逐字透传）；「尚未转化」由宿主 provider 报错。
fn run_lead_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_run_lead_workflow";
    let lead_id = require_str(input, OP, "leadId")?;
    call_opc_store(peer, OPC_STORE_RUN_LEAD_WORKFLOW, json!({ "lead_id": lead_id }), call_chain)
}

/// `opc_list_domain_workflows`：领域工作流摘要列表（只读 op，两个可选过滤项）。
///
/// 命令面 `domainId` ⇒ 接缝面 `domain_id`（三面键名分档）；`tag` 两面同形。
fn list_domain_workflows(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_domain_workflows";
    let mut args = serde_json::Map::new();
    match input.get("domainId") {
        None | Some(Value::Null) => {},
        Some(Value::String(value)) => {
            args.insert("domain_id".to_string(), Value::String(value.clone()));
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `domainId` 必须是字符串（形状判在命令层）"
            )));
        },
    }
    copy_optional_strings(input, OP, &mut args, &["tag"])?;
    call_opc_store(peer, OPC_STORE_LIST_DOMAIN_WORKFLOWS, Value::Object(args), call_chain)
}

/// `opc_get_domain_workflow`：单个领域工作流的完整定义（只读 op，`workflowId` 必填）。
fn get_domain_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_domain_workflow";
    let workflow_id = require_str(input, OP, "workflowId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_DOMAIN_WORKFLOW,
        json!({ "workflow_id": workflow_id }),
        call_chain,
    )
}

/// `opc_list_domains`：领域 id 与中文名（只读 op，零入参）。
fn list_domains(call_chain: &[String], peer: &mut WorkerPeer) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_LIST_DOMAINS, json!({}), call_chain)
}

/// `opc_execute_demand_workflow`：为线索创建交付记录并后台触发工作流（写 op）。
///
/// `leadId` 必填；`workflowTemplateId` 可选（缺省链 `入参 → lead.linkedWorkflowId →
/// "default_demand_delivery"` 由宿主实现）。出参是刚落库的交付记录（camelCase DTO）。
fn execute_demand_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_execute_demand_workflow";
    let lead_id = require_str(input, OP, "leadId")?;
    let mut args = serde_json::Map::new();
    args.insert("lead_id".to_string(), Value::String(lead_id.to_string()));
    match input.get("workflowTemplateId") {
        None | Some(Value::Null) => {},
        Some(Value::String(value)) => {
            args.insert("workflow_template_id".to_string(), Value::String(value.clone()));
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `workflowTemplateId` 必须是字符串（形状判在命令层）"
            )));
        },
    }
    call_opc_store(peer, OPC_STORE_EXECUTE_DEMAND_WORKFLOW, Value::Object(args), call_chain)
}

/// `opc_list_deliveries`：交付记录列表（只读 op，两个可选过滤项）。
///
/// 命令面 `leadId` ⇒ 接缝面 `lead_id`（三面键名分档）；`status` 两面同形。
fn list_deliveries(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_deliveries";
    let mut args = serde_json::Map::new();
    copy_optional_strings(input, OP, &mut args, &["status"])?;
    match input.get("leadId") {
        None | Some(Value::Null) => {},
        Some(Value::String(value)) => {
            args.insert("lead_id".to_string(), Value::String(value.clone()));
        },
        Some(_) => {
            return Err(invalid_shape(format!("{OP} 的 `leadId` 必须是字符串（形状判在命令层）")));
        },
    }
    call_opc_store(peer, OPC_STORE_LIST_DELIVERIES, Value::Object(args), call_chain)
}

/// `opc_get_delivery`：单个交付详情（只读 op，`deliveryId` 必填）。
fn get_delivery(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_delivery";
    let delivery_id = require_str(input, OP, "deliveryId")?;
    call_opc_store(peer, OPC_STORE_GET_DELIVERY, json!({ "delivery_id": delivery_id }), call_chain)
}

/// `opc_update_delivery`：更新交付执行面字段（写 op；`deliveryId` + `status` 必填）。
///
/// `progress`（数值）/ `resultSummary`（字符串）可选；`deliverables` / `errors` 是任意
/// JSON 值（形状由宿主 serde 判），缺键 = 不动。
fn update_delivery(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_delivery";
    let delivery_id = require_str(input, OP, "deliveryId")?;
    let status = require_str(input, OP, "status")?;
    let mut args = serde_json::Map::new();
    args.insert("delivery_id".to_string(), Value::String(delivery_id.to_string()));
    args.insert("status".to_string(), Value::String(status.to_string()));
    copy_optional_f64(input, OP, &mut args, &["progress"])?;
    match input.get("resultSummary") {
        None | Some(Value::Null) => {},
        Some(Value::String(value)) => {
            args.insert("result_summary".to_string(), Value::String(value.clone()));
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `resultSummary` 必须是字符串（形状判在命令层）"
            )));
        },
    }
    for key in ["deliverables", "errors"] {
        match input.get(key) {
            None | Some(Value::Null) => {},
            Some(value) => {
                args.insert(key.to_string(), value.clone());
            },
        }
    }
    call_opc_store(peer, OPC_STORE_UPDATE_DELIVERY, Value::Object(args), call_chain)
}

/// `opc_retry_delivery`：重置为待执行（写 op，`deliveryId` 必填）。
fn retry_delivery(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_retry_delivery";
    let delivery_id = require_str(input, OP, "deliveryId")?;
    call_opc_store(
        peer,
        OPC_STORE_RETRY_DELIVERY,
        json!({ "delivery_id": delivery_id }),
        call_chain,
    )
}

/// `opc_cancel_delivery`：取消交付（写 op，`deliveryId` 必填）。
fn cancel_delivery(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_cancel_delivery";
    let delivery_id = require_str(input, OP, "deliveryId")?;
    call_opc_store(
        peer,
        OPC_STORE_CANCEL_DELIVERY,
        json!({ "delivery_id": delivery_id }),
        call_chain,
    )
}

/// `opc_scan_capabilities`：扫描系统能力清单（读 op，零入参）。
fn scan_capabilities(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_SCAN_CAPABILITIES, json!({}), call_chain)
}

/// `opc_proactive_evaluate_leads`：主动评估入库（写 op，零入参 —— 关键词由宿主从
/// 配置提取，前端不再传 minScore 历史遗留参数）。
fn proactive_evaluate_leads(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_PROACTIVE_EVALUATE_LEADS, json!({}), call_chain)
}

/// `opc_list_capability_gaps`：列出能力缺口（读 op，`status` 可选）。
fn list_capability_gaps(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_capability_gaps";
    let mut args = serde_json::Map::new();
    copy_optional_strings(input, OP, &mut args, &["status"])?;
    call_opc_store(peer, OPC_STORE_LIST_CAPABILITY_GAPS, Value::Object(args), call_chain)
}

/// `opc_close_capability_gap`：关闭能力缺口（写 op，`gapId` 必填）。
fn close_capability_gap(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_close_capability_gap";
    let gap_id = require_str(input, OP, "gapId")?;
    call_opc_store(peer, OPC_STORE_CLOSE_CAPABILITY_GAP, json!({ "gap_id": gap_id }), call_chain)
}

/// `opc_analyze_capability_gaps`：主动分析能力缺口（写 op，零入参 —— Top 5 高频缺失
/// 自动建缺口行）。
fn analyze_capability_gaps(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_ANALYZE_CAPABILITY_GAPS, json!({}), call_chain)
}

// ── #264 批 7 刀 A：订阅面三命令 + 域包配置面八命令 ──────────────────────────
//
// 键名分档同既有批（PLAN §144.2）：命令面 camelCase（`capabilityPackId` / `actionKey` /
// `input`），接缝面 snake_case（`domain_pack_id` / `action_key`）。域包侧的入参键沿用
// 迁移前那十一条 Tauri 命令的参数名（`capability_pack_id` ⇒ `capabilityPackId`）；
// 接缝 op 的键则是 harness trait 的形参名（`domain_pack_id`），两处不许互换。
// （刀 B1 的七条域包全读命令已同风格落地，见下方「域包全读七命令」段；
// 旧注里的 `opc_get_workflow_config` 已随刀 B1 迁入本文件。）

/// `opc_list_subscriptions`：列出全部需求订阅（读 op，零入参；创建时间升序整表下发）。
fn list_subscriptions(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_LIST_SUBSCRIPTIONS, json!({}), call_chain)
}

/// `opc_save_subscription`：新增或更新订阅（写 op，`input` 必填对象）。
///
/// 命令层只判 `input` 是不是对象；关键词必填、`intervalHours` / `minScore` 钳位、
/// 缺行 `NotFound` 全在 dao（迁移前的 Tauri 命令就是 dao 的直包装，接缝不加新闸）。
fn save_subscription(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_save_subscription";
    let payload = require_object(input, OP, "input")?;
    call_opc_store(peer, OPC_STORE_SAVE_SUBSCRIPTION, json!({ "input": payload }), call_chain)
}

/// `opc_delete_subscription`：删除订阅（写 op，`id` 必填；空串与查无行同回 `NotFound`）。
fn delete_subscription(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_subscription";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_SUBSCRIPTION, json!({ "id": id }), call_chain)
}

/// `opc_list_industries`：列出内建域包摘要（读 op，零入参 —— 数据源是代码内注册表）。
fn list_industries(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_LIST_INDUSTRIES, json!({}), call_chain)
}

/// `opc_get_capability_pack_config`：取一个内建域包的完整配置（读 op，含 prompt 全文）。
fn get_capability_pack_config(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack_config";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_CAPABILITY_PACK_CONFIG,
        json!({ "domain_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_get_action_config`：取单个域包操作的配置（读 op，两个必填键）。
fn get_action_config(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_action_config";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let action_key = require_str(input, OP, "actionKey")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_ACTION_CONFIG,
        json!({ "domain_pack_id": pack_id, "action_key": action_key }),
        call_chain,
    )
}

/// `opc_list_capability_pack_actions`：列出一个域包的全部操作摘要（读 op，裁掉两条 prompt 字段）。
fn list_capability_pack_actions(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_capability_pack_actions";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_LIST_CAPABILITY_PACK_ACTIONS,
        json!({ "domain_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_list_capability_pack_workflows`：列出一个域包的全部工作流摘要（读 op）。
fn list_capability_pack_workflows(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_capability_pack_workflows";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_LIST_CAPABILITY_PACK_WORKFLOWS,
        json!({ "domain_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_list_runtime_industries`：列出磁盘上的运行时域包（读 op，零入参，出参带 `count`）。
///
/// 与 `opc_list_industries` 同名不同源（那边是代码内注册表），两条命令都保留、不许合并。
fn list_runtime_industries(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_LIST_RUNTIME_INDUSTRIES, json!({}), call_chain)
}

/// `opc_get_kpi_definitions`：取一个域包的 KPI 定义（读 op）。
///
/// 出参内层键是 snake_case（`metric_type` / `calculation_rule`），与迁移前逐字一致 ——
/// 那条线格式由 harness 那份视图的文档钉住，本命令不改写、不重命名。
fn get_kpi_definitions(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_kpi_definitions";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_KPI_DEFINITIONS,
        json!({ "domain_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_has_capability_pack`：判断运行时域包是否存在（读 op；不存在回 `exists: false`，不报错）。
fn has_capability_pack(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_has_capability_pack";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_HAS_CAPABILITY_PACK,
        json!({ "domain_pack_id": pack_id }),
        call_chain,
    )
}

// ── #264 批 7 刀 B1：域包全读七命令 ──────────────────────────────────────────
//
// 键名分档同刀 A：命令面 camelCase（`capabilityPackId` / `workflowId` / `entityType` /
// `entityData` / `sourceId` / `dataDomain`，沿用迁移前七条 Tauri 命令的参数名），
// 接缝面 snake_case（`domain_pack_id` 与刀 A 域包 op 同键），两处不许互换。

/// `opc_get_workflow_config`：取域包单个工作流配置（读 op，两个必填键；查无回未找到）。
fn get_workflow_config(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_workflow_config";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let workflow_id = require_str(input, OP, "workflowId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_WORKFLOW_CONFIG,
        json!({ "domain_pack_id": pack_id, "workflow_id": workflow_id }),
        call_chain,
    )
}

/// `opc_validate_entity`：验证域包实体（读 op —— 只校验不落库；`entityData` 形状判在命令层）。
fn validate_entity(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_validate_entity";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let entity_type = require_str(input, OP, "entityType")?;
    let entity_data = require_object(input, OP, "entityData")?;
    call_opc_store(
        peer,
        OPC_STORE_VALIDATE_ENTITY,
        json!({
            "domain_pack_id": pack_id,
            "entity_type": entity_type,
            "entity_data": entity_data,
        }),
        call_chain,
    )
}

/// `opc_batch_validate_entities`：批量验证域包实体（读 op；元素内 `entityType` 为空的
/// 条目由宿主跳过 —— 裁剪语义在宿主实现体，命令层只判「是数组」）。
fn batch_validate_entities(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_batch_validate_entities";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let entities = input
        .get("entities")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_shape(format!("{OP} 的 `entities` 必须是数组（形状判在命令层）")))?;
    call_opc_store(
        peer,
        OPC_STORE_BATCH_VALIDATE_ENTITIES,
        json!({ "domain_pack_id": pack_id, "entities": entities }),
        call_chain,
    )
}

/// `opc_compute_kpis`：计算域包 KPI（读 op；`days` 可选，缺省由宿主按 30 处理）。
fn compute_kpis(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_compute_kpis";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let mut request = serde_json::Map::new();
    request.insert("domain_pack_id".to_string(), json!(pack_id));
    copy_optional_i64(input, OP, &mut request, &["days"])?;
    call_opc_store(peer, OPC_STORE_COMPUTE_KPIS, Value::Object(request), call_chain)
}

/// `opc_get_capability_pack_data`：按 analysis.yaml 数据源路由取域包数据（读 op；
/// `query` 可选，缺省由宿主按空对象处理）。
fn get_capability_pack_data(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack_data";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let source_id = require_str(input, OP, "sourceId")?;
    let data_domain = require_str(input, OP, "dataDomain")?;
    let mut request = serde_json::Map::new();
    request.insert("domain_pack_id".to_string(), json!(pack_id));
    request.insert("source_id".to_string(), json!(source_id));
    request.insert("data_domain".to_string(), json!(data_domain));
    if let Some(query) = input.get("query").filter(|value| !value.is_null()) {
        request.insert("query".to_string(), query.clone());
    }
    call_opc_store(peer, OPC_STORE_GET_CAPABILITY_PACK_DATA, Value::Object(request), call_chain)
}

/// `opc_get_capability_pack_precheck`：域包数据源质量预检（读 op，`quality_precheck`
/// 源清单探测）。
fn get_capability_pack_precheck(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack_precheck";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_CAPABILITY_PACK_PRECHECK,
        json!({ "domain_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_get_capability_pack_health`：域包数据源健康快照（读 op，vendor 降级可观测）。
fn get_capability_pack_health(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack_health";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_CAPABILITY_PACK_HEALTH,
        json!({ "domain_pack_id": pack_id }),
        call_chain,
    )
}

// ── #264 批 7 刀 B2：域包单据面十二命令（六读六写）─────────────────────────────
//
// 迁移前是宿主 `commands::opc` 的同名 `#[agent_command]`（`opc_execute_dynamic_workflow` 在
// `commands::opc_capability_pack_runtime`）；本命令只做入参形状 + 透传，语义闸（非空 / 存在性 /
// 目录可读 / 状态机合法性）全在宿主 provider。键名分档同前批：命令面 camelCase，接缝面
// snake_case；`input` 是域模型 DTO，内部键沿用迁移前的 snake_case，**不转换**。

/// `opc_get_invoice`：取一张域包发票（读 op，`id` 必填；查无回未找到）。
fn get_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_invoice";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_GET_INVOICE, json!({ "id": id }), call_chain)
}

/// `opc_update_invoice`：更新一张域包发票（写 op，`id` 必填 + `input` 必填对象）。
///
/// 命令层只判 `input` 是不是对象：`line_items` / `notes` / `due_at` 的逐字段类型由宿主
/// `from_value` 反序列化 fail-closed（缺键按默认值，类型不符整条拒）。
fn update_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_invoice";
    let id = require_str(input, OP, "id")?;
    let payload = require_object(input, OP, "input")?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_INVOICE,
        json!({ "id": id, "input": payload }),
        call_chain,
    )
}

/// `opc_get_customer`：取一个客户（读 op，`id` 必填；查无回未找到）。
fn get_customer(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_customer";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_GET_CUSTOMER, json!({ "id": id }), call_chain)
}

/// `opc_find_customer_by_email`：按邮箱查客户（读 op，`email` 必填；查无回 `null`，
/// 与「查库失败」分属两种结果，插件侧不许把 `null` 翻成错误）。
fn find_customer_by_email(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_find_customer_by_email";
    let email = require_str(input, OP, "email")?;
    call_opc_store(peer, OPC_STORE_FIND_CUSTOMER_BY_EMAIL, json!({ "email": email }), call_chain)
}

/// `opc_record_kpi`：记录一条 KPI（写 op，`input` 必填对象）。
///
/// `domain_pack_id` 的必填与「空串 / 空白即拒」都在宿主 provider（迁移前就是
/// `CreateKpiInput` 的 serde 约束 + `canonical_domain_pack_id` 的语义闸），命令层不越界。
fn record_kpi(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_record_kpi";
    let payload = require_object(input, OP, "input")?;
    call_opc_store(peer, OPC_STORE_RECORD_KPI, json!({ "input": payload }), call_chain)
}

/// `opc_list_revenue`：列出收入记录（读 op，`category` / `limit` 可选）。
///
/// 两个可选键缺省都**不转发**（不塞 null）：宿主侧是 `Option`，缺键即 `None`，塞 null 会
/// 多造一个「调用方显式传了 null」的假信号。`limit` 给了必须是数值（非整数由宿主 `u32`
/// 反序列化 fail-closed 拒）。
fn list_revenue(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_list_revenue";
    let mut args = serde_json::Map::new();
    copy_optional_strings(input, OP, &mut args, &["category"])?;
    copy_optional_f64(input, OP, &mut args, &["limit"])?;
    call_opc_store(peer, OPC_STORE_LIST_REVENUE, Value::Object(args), call_chain)
}

/// `opc_get_investment_advice`：按周期给投资建议（读 op，`period` 必填；纯计算不落库）。
fn get_investment_advice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_investment_advice";
    let period = require_str(input, OP, "period")?;
    call_opc_store(peer, OPC_STORE_GET_INVESTMENT_ADVICE, json!({ "period": period }), call_chain)
}

/// `opc_import_talent_library`：导入人才库（写 op，`path` 必填 —— 扫描目录并落模板表）。
///
/// 「目录不存在 / 不可读」的判定留在宿主 provider（迁移前命令体里就有一条 `is_dir` 前置闸）：
/// 插件进程不该拿到一条可绕过的路径判据副本（两侧各判一半必然漂移）。
fn import_talent_library(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_import_talent_library";
    let path = require_str(input, OP, "path")?;
    call_opc_store(peer, OPC_STORE_IMPORT_TALENT_LIBRARY, json!({ "path": path }), call_chain)
}

/// `opc_sync_fleet`：把 OPC 员工同步为舰队成员（写 op，零入参，幂等）。
fn sync_fleet(call_chain: &[String], peer: &mut WorkerPeer) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_SYNC_FLEET, json!({}), call_chain)
}

/// `opc_get_publish_schedule`：取单个发布计划（读 op，`id` 必填；查无回未找到）。
fn get_publish_schedule(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_publish_schedule";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_GET_PUBLISH_SCHEDULE, json!({ "id": id }), call_chain)
}

/// `opc_update_publish_schedule`：更新发布计划（写 op，`id` 必填 + `input` 必填对象）。
///
/// `scheduled_at` / `status` 的取值域（状态机合法值、时间戳量纲）全在宿主 provider；命令层
/// 只判 `input` 是对象。
fn update_publish_schedule(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_publish_schedule";
    let id = require_str(input, OP, "id")?;
    let payload = require_object(input, OP, "input")?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_PUBLISH_SCHEDULE,
        json!({ "id": id, "input": payload }),
        call_chain,
    )
}

/// `opc_execute_dynamic_workflow`：动态工作流执行（写 op，兼容旧接口）。
///
/// 命令面 `capabilityPackId` 必填，`workflowId` / `days` / `userInput` 可选；接缝键改为
/// snake_case（`capability_pack_id` / `workflow_id` / `days` / `user_input`），可选键缺省
/// **不传**（不塞 null —— 宿主侧是 `Option`，缺键即 `None`）。三级模板查找与引擎装配在宿主
/// provider，命令层不复制。
fn execute_dynamic_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_execute_dynamic_workflow";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let mut args = serde_json::Map::new();
    args.insert("capability_pack_id".to_string(), json!(pack_id));
    match input.get("workflowId") {
        None | Some(Value::Null) => {},
        Some(Value::String(workflow_id)) => {
            args.insert("workflow_id".to_string(), Value::String(workflow_id.clone()));
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `workflowId` 必须是字符串（形状判在命令层）"
            )));
        },
    }
    match input.get("days") {
        None | Some(Value::Null) => {},
        Some(value) if value.is_number() => {
            args.insert("days".to_string(), value.clone());
        },
        Some(_) => {
            return Err(invalid_shape(format!("{OP} 的 `days` 必须是数值（形状判在命令层）")));
        },
    }
    if let Some(user_input) = input.get("userInput").filter(|value| !value.is_null()) {
        args.insert("user_input".to_string(), user_input.clone());
    }
    call_opc_store(peer, OPC_STORE_EXECUTE_DYNAMIC_WORKFLOW, Value::Object(args), call_chain)
}

/// `opc_execute_analysis`：执行域包分析（读 op）。
///
/// 命令面 `capabilityPackId` 必填、`days` 可选（缺省 30 天由宿主补）；接缝键用 snake_case，
/// `days` 缺省**不传**（宿主侧是 `Option`，缺键即 `None`）。风控阈值取域包 `runtime.yaml`
/// 的声明、数据服务与时间窗装配全在宿主 provider，命令层不复制。
fn execute_analysis(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_execute_analysis";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let mut args = serde_json::Map::new();
    args.insert("capability_pack_id".to_string(), json!(pack_id));
    copy_optional_i64(input, OP, &mut args, &["days"])?;
    call_opc_store(peer, OPC_STORE_EXECUTE_ANALYSIS, Value::Object(args), call_chain)
}

/// `opc_reflect_on_workflow`：触发工作流反思（写 op；反思模板由宿主从域包适配器取）。
fn reflect_on_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_reflect_on_workflow";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let workflow_id = require_str(input, OP, "workflowId")?;
    // `workflowResult` 形状不限（迁移前命令参数就是 `serde_json::Value`，可为 `null`）。
    let Some(workflow_result) = input.get("workflowResult") else {
        return Err(invalid_shape(format!("{OP} 需要 `workflowResult` 参数（形状判在命令层）")));
    };
    call_opc_store(
        peer,
        OPC_STORE_REFLECT_ON_WORKFLOW,
        json!({
            "capability_pack_id": pack_id,
            "workflow_id": workflow_id,
            "workflow_result": workflow_result,
        }),
        call_chain,
    )
}

/// `opc_evolve_workflow`：触发工作流进化（写 op；进化约束由宿主从域包适配器取）。
fn evolve_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_evolve_workflow";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let workflow_id = require_str(input, OP, "workflowId")?;
    let reason = require_str(input, OP, "reason")?;
    call_opc_store(
        peer,
        OPC_STORE_EVOLVE_WORKFLOW,
        json!({
            "capability_pack_id": pack_id,
            "workflow_id": workflow_id,
            "reason": reason,
        }),
        call_chain,
    )
}

/// `opc_run_self_improvement`：执行自我改进（写 op）。
fn run_self_improvement(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_run_self_improvement";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let target = require_str(input, OP, "target")?;
    call_opc_store(
        peer,
        OPC_STORE_RUN_SELF_IMPROVEMENT,
        json!({ "capability_pack_id": pack_id, "target": target }),
        call_chain,
    )
}

/// `opc_trigger_capability_pack_learning`：触发域包学习闭环（写 op；四段编排在宿主）。
fn trigger_capability_pack_learning(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_trigger_capability_pack_learning";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let workflow_id = require_str(input, OP, "workflowId")?;
    let Some(workflow_result) = input.get("workflowResult") else {
        return Err(invalid_shape(format!("{OP} 需要 `workflowResult` 参数（形状判在命令层）")));
    };
    call_opc_store(
        peer,
        OPC_STORE_TRIGGER_CAPABILITY_PACK_LEARNING,
        json!({
            "capability_pack_id": pack_id,
            "workflow_id": workflow_id,
            "workflow_result": workflow_result,
        }),
        call_chain,
    )
}

/// `opc_get_rl_stats`：经验池统计（读 op；`capabilityPackId` 可选，缺省取全局池）。
fn get_rl_stats(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_rl_stats";
    let mut args = serde_json::Map::new();
    match input.get("capabilityPackId") {
        None | Some(Value::Null) => {},
        Some(Value::String(value)) => {
            args.insert("capability_pack_id".to_string(), json!(value));
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `capabilityPackId` 必须是字符串（形状判在命令层）"
            )));
        },
    }
    call_opc_store(peer, OPC_STORE_GET_RL_STATS, Value::Object(args), call_chain)
}

/// `opc_trigger_rl_optimization`：手动触发 RL 策略优化（写 op；RL 配置由宿主读）。
fn trigger_rl_optimization(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_trigger_rl_optimization";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_TRIGGER_RL_OPTIMIZATION,
        json!({ "capability_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_record_rl_experience`：手动记录一条工作流执行经验（写 op）。
fn record_rl_experience(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_record_rl_experience";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let workflow_id = require_str(input, OP, "workflowId")?;
    let Some(quality_score) = input.get("qualityScore").and_then(Value::as_f64) else {
        return Err(invalid_shape(format!("{OP} 的 `qualityScore` 必须是数值（形状判在命令层）")));
    };
    let Some(workflow_result) = input.get("workflowResult") else {
        return Err(invalid_shape(format!("{OP} 需要 `workflowResult` 参数（形状判在命令层）")));
    };
    call_opc_store(
        peer,
        OPC_STORE_RECORD_RL_EXPERIENCE,
        json!({
            "capability_pack_id": pack_id,
            "workflow_id": workflow_id,
            "quality_score": quality_score,
            "workflow_result": workflow_result,
        }),
        call_chain,
    )
}

/// `opc_get_capability_pack_dashboard`：域包仪表盘（读 op；`days` 缺省由宿主按 30 天取）。
fn get_capability_pack_dashboard(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack_dashboard";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let mut args = serde_json::Map::new();
    args.insert("capability_pack_id".to_string(), json!(pack_id));
    copy_optional_i64(input, OP, &mut args, &["days"])?;
    call_opc_store(peer, OPC_STORE_CAPABILITY_PACK_DASHBOARD, Value::Object(args), call_chain)
}

/// `opc_get_capability_pack_workflow_steps`：域包工作流步骤（读 op）。
fn get_capability_pack_workflow_steps(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack_workflow_steps";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_CAPABILITY_PACK_WORKFLOW_STEPS,
        json!({ "capability_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_get_capability_pack_automation_rules`：域包启用的自动化规则（读 op）。
fn get_capability_pack_automation_rules(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack_automation_rules";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_CAPABILITY_PACK_AUTOMATION_RULES,
        json!({ "capability_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_run_automation_rules`：触发域包自动化规则（写 op）。
///
/// 命令面平铺 camelCase（`capabilityPackId` / `entityType` / `entityId` / `overdueDays` /
/// `createdDays`），接缝键改 snake_case；可选键缺省**不传**（宿主侧是 `Option`，缺键即
/// `None`）。`fields` 是任意对象，原样透传。
fn run_automation_rules(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_run_automation_rules";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let entity_type = require_str(input, OP, "entityType")?;
    let entity_id = require_str(input, OP, "entityId")?;
    let mut args = serde_json::Map::new();
    args.insert("capability_pack_id".to_string(), json!(pack_id));
    args.insert("entity_type".to_string(), json!(entity_type));
    args.insert("entity_id".to_string(), json!(entity_id));
    copy_optional_strings(input, OP, &mut args, &["status"])?;
    // 命令面键是 camelCase、接缝键是 snake_case，故逐键改名而不是批量复制。
    for (from, to) in [("overdueDays", "overdue_days"), ("createdDays", "created_days")] {
        match input.get(from) {
            None | Some(Value::Null) => {},
            Some(value) if value.as_u64().is_some_and(|n| n <= u64::from(u32::MAX)) => {
                args.insert(to.to_string(), value.clone());
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "{OP} 的 `{from}` 必须是非负整数（形状判在命令层）"
                )));
            },
        }
    }
    match input.get("fields") {
        None | Some(Value::Null) => {},
        Some(value) if value.is_object() => {
            args.insert("fields".to_string(), value.clone());
        },
        Some(_) => {
            return Err(invalid_shape(format!("{OP} 的 `fields` 必须是对象（形状判在命令层）")));
        },
    }
    call_opc_store(peer, OPC_STORE_RUN_AUTOMATION_RULES, Value::Object(args), call_chain)
}

/// `opc_run_subscription_scan`：批量扫描需求订阅（写 op）。
///
/// 唯一的可选键 `onlyDue` 缺省不传（宿主侧 `Option<bool>`，缺键即「只扫到期订阅」）；
/// 到期判定、命中过滤、到期时间推进全在宿主 provider。
fn run_subscription_scan(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_run_subscription_scan";
    let mut args = serde_json::Map::new();
    match input.get("onlyDue") {
        None | Some(Value::Null) => {},
        Some(value) if value.is_boolean() => {
            args.insert("only_due".to_string(), value.clone());
        },
        Some(_) => {
            return Err(invalid_shape(format!("{OP} 的 `onlyDue` 必须是布尔（形状判在命令层）")));
        },
    }
    call_opc_store(peer, OPC_STORE_RUN_SUBSCRIPTION_SCAN, Value::Object(args), call_chain)
}

/// `opc_ensure_demand_scan_job`：幂等装配需求扫描定时任务（写 op）。
///
/// 三个可选键都是字符串，缺省**不传**（宿主侧 `Option<String>`）；cron 合法性由宿主
/// 校验，投递渠道两个都不给 ⇒ 只写执行历史不推送。
fn ensure_demand_scan_job(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_ensure_demand_scan_job";
    let mut args = serde_json::Map::new();
    // 命令面键是 camelCase、接缝键是 snake_case，故逐键改名而不是批量复制。
    for (from, to) in [
        ("cronExpression", "cron_expression"),
        ("webhookUrl", "webhook_url"),
        ("filePath", "file_path"),
    ] {
        match input.get(from) {
            None | Some(Value::Null) => {},
            Some(Value::String(value)) => {
                args.insert(to.to_string(), Value::String(value.clone()));
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "{OP} 的 `{from}` 必须是字符串（形状判在命令层）"
                )));
            },
        }
    }
    call_opc_store(peer, OPC_STORE_ENSURE_DEMAND_SCAN_JOB, Value::Object(args), call_chain)
}

/// `opc_match_lead_capabilities`：单条线索的能力匹配（读 op）。
///
/// `leadId` 必填；`topK` 缺省不传（宿主按实现方缺省，越界由宿主夹取）。
fn match_lead_capabilities(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_match_lead_capabilities";
    let lead_id = require_str(input, OP, "leadId")?;
    let mut args = serde_json::Map::new();
    args.insert("lead_id".to_string(), json!(lead_id));
    match input.get("topK") {
        None | Some(Value::Null) => {},
        Some(value) if value.as_u64().is_some_and(|n| n <= u64::from(u32::MAX)) => {
            args.insert("top_k".to_string(), value.clone());
        },
        Some(_) => {
            return Err(invalid_shape(format!("{OP} 的 `topK` 必须是非负整数（形状判在命令层）")));
        },
    }
    call_opc_store(peer, OPC_STORE_MATCH_LEAD_CAPABILITIES, Value::Object(args), call_chain)
}

/// `opc_get_capability_pack`：域包 manifest 摘要（读 op，`{ manifest }` 信封由宿主组）。
fn get_capability_pack(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_capability_pack";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_CAPABILITY_PACK_MANIFEST,
        json!({ "capability_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_build_capability_pack_prompt`：域包操作的 prompt 装配（读 op）。
///
/// `capabilityPackId` / `actionKey` 必填，`userInput` 可选字符串（缺省或空白 ⇒ 宿主不替换
/// 模板里的 `{{input}}`，与迁移前命令体同一判据）。出参五键信封由宿主 provider 组。
fn build_capability_pack_prompt(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_build_capability_pack_prompt";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    let action_key = require_str(input, OP, "actionKey")?;
    let mut args = serde_json::Map::new();
    args.insert("capability_pack_id".to_string(), json!(pack_id));
    args.insert("action_key".to_string(), json!(action_key));
    match input.get("userInput") {
        None | Some(Value::Null) => {},
        Some(Value::String(value)) => {
            args.insert("user_input".to_string(), Value::String(value.clone()));
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `userInput` 必须是字符串（形状判在命令层）"
            )));
        },
    }
    call_opc_store(peer, OPC_STORE_BUILD_CAPABILITY_PACK_PROMPT, Value::Object(args), call_chain)
}

/// `opc_get_learning_metrics`：域包学习指标（读 op）。
fn get_learning_metrics(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_learning_metrics";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_LEARNING_METRICS,
        json!({ "capability_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_get_learning_config`：单个域包的学习配置（读 op；缺 learning.yaml 由宿主退回默认配置）。
fn get_learning_config(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_learning_config";
    let pack_id = require_str(input, OP, "capabilityPackId")?;
    call_opc_store(
        peer,
        OPC_STORE_GET_LEARNING_CONFIG,
        json!({ "capability_pack_id": pack_id }),
        call_chain,
    )
}

/// `opc_list_learning_configs`：全部域包的学习配置（读 op，无入参）。
fn list_learning_configs(
    _input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_LIST_LEARNING_CONFIGS, json!({}), call_chain)
}

/// `opc_auto_learn_workflow`：触发 OPC 域包工作流的自动学习管线（写 op）。
///
/// `templateId` 必填字符串；`workflowResult` 必填但**形状不限**（迁移前命令参数就是
/// `serde_json::Value`，可以是任何 JSON 含 `null`）—— 识别域包与算质量分的判据在宿主。
fn auto_learn_workflow(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_auto_learn_workflow";
    let template_id = require_str(input, OP, "templateId")?;
    let Some(workflow_result) = input.get("workflowResult") else {
        return Err(invalid_shape(format!("{OP} 需要 `workflowResult` 参数（形状判在命令层）")));
    };
    let mut args = serde_json::Map::new();
    args.insert("template_id".to_string(), json!(template_id));
    args.insert("workflow_result".to_string(), workflow_result.clone());
    call_opc_store(peer, OPC_STORE_AUTO_LEARN_WORKFLOW, Value::Object(args), call_chain)
}

/// `opc_create_project`：创建项目（写 op）。
fn create_project(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_project";
    let title = require_str(input, OP, "title")?;
    let mut request = serde_json::Map::new();
    request.insert("title".to_string(), Value::String(title.to_string()));
    copy_optional_strings(
        input,
        OP,
        &mut request,
        &["customerId", "description", "currency", "notes"],
    )?;
    copy_optional_f64(input, OP, &mut request, &["budget"])?;
    copy_optional_i64(input, OP, &mut request, &["deadline"])?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_PROJECT,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_add_milestone`：给项目加里程碑（写 op）。
///
/// 里程碑 id 与初始状态由宿主 provider 生成（`Pending`）—— 命令面不收 `id` / `status`。
fn add_milestone(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_add_milestone";
    let project_id = require_str(input, OP, "projectId")?;
    let title = require_str(input, OP, "title")?;
    let mut request = serde_json::Map::new();
    request.insert("projectId".to_string(), Value::String(project_id.to_string()));
    request.insert("title".to_string(), Value::String(title.to_string()));
    copy_optional_strings(input, OP, &mut request, &["description"])?;
    copy_optional_i64(input, OP, &mut request, &["dueAt"])?;
    call_opc_store(
        peer,
        OPC_STORE_ADD_MILESTONE,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_create_landing_page`：创建落地页（写 op，创建后为草稿态）。
fn create_landing_page(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_landing_page";
    let title = require_str(input, OP, "title")?;
    let slug = require_str(input, OP, "slug")?;
    let content = require_str(input, OP, "content")?;
    let mut request = serde_json::Map::new();
    request.insert("title".to_string(), Value::String(title.to_string()));
    request.insert("slug".to_string(), Value::String(slug.to_string()));
    request.insert("content".to_string(), Value::String(content.to_string()));
    copy_optional_strings(input, OP, &mut request, &["description"])?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_LANDING_PAGE,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_get_financial_report`：取财务报表（只读 op，宿主侧无写权限闸）。
///
/// 入参是**散标量** `{"period": …}`（不是 DTO，不套 `request` 壳）；出参是
/// [`FinancialReportView`] 的扁平对象 —— 报表与投资建议成对返回，只多一次帧往返。
fn get_financial_report(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_financial_report";
    let period = require_str(input, OP, "period")?;
    call_opc_store(peer, OPC_STORE_GET_FINANCIAL_REPORT, json!({ "period": period }), call_chain)
}

/// `opc_create_content_asset`：创建内容资产（写 op，创建后为草稿态）。
fn create_content_asset(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_content_asset";
    let title = require_str(input, OP, "title")?;
    let content_type = require_str(input, OP, "contentType")?;
    let body = require_str(input, OP, "body")?;
    let mut request = serde_json::Map::new();
    request.insert("title".to_string(), Value::String(title.to_string()));
    request.insert("contentType".to_string(), Value::String(content_type.to_string()));
    request.insert("body".to_string(), Value::String(body.to_string()));
    if let Some(tags) = input.get("tags").filter(|value| !value.is_null()) {
        let items = tags.as_array().filter(|items| items.iter().all(Value::is_string)).ok_or_else(
            || invalid_shape(format!("{OP} 的 `tags` 必须是字符串数组（形状判在命令层）")),
        )?;
        request.insert("tags".to_string(), Value::Array(items.clone()));
    }
    call_opc_store(
        peer,
        OPC_STORE_CREATE_CONTENT_ASSET,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_create_publish_schedule`：创建发布计划（写 op）。
///
/// `contentRefId` 是**内容实体的主键**（博客 id / 内容资产 id），不是发布计划自己的 id。
fn create_publish_schedule(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_publish_schedule";
    let content_ref_type = require_str(input, OP, "contentRefType")?;
    let content_ref_id = require_str(input, OP, "contentRefId")?;
    let scheduled_at = input
        .get("scheduledAt")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid_shape(format!("{OP} 需要整数时间戳字段 `scheduledAt`")))?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_PUBLISH_SCHEDULE,
        json!({
            "request": {
                "contentRefType": content_ref_type,
                "contentRefId": content_ref_id,
                "scheduledAt": scheduled_at,
            }
        }),
        call_chain,
    )
}

/// `opc_process_due_schedules`：处理全部到期的发布计划（写 op，无入参）。
///
/// 出参是**本次处理过的行**（视图数组）：空数组 = 「没有到期计划」，不是错误。
fn process_due_schedules(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_PROCESS_DUE_SCHEDULES, json!({}), call_chain)
}

/// 必需的对象字段（对象内部形状由宿主 serde 判）。
fn require_object<'a>(
    input: &'a Value,
    op: &str,
    key: &str,
) -> Result<&'a Value, (String, String)> {
    match input.get(key) {
        Some(value @ Value::Object(_)) => Ok(value),
        _ => Err(invalid_shape(format!("{op} 需要对象字段 `{key}`（形状判在命令层）"))),
    }
}

/// `opc_create_work_item`：创建工作项（写 op；id 由宿主生成 `wi-<12 位 hex>`）。
fn create_work_item(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_work_item";
    let title = require_str(input, OP, "title")?;
    let mut args = serde_json::Map::new();
    args.insert("title".to_string(), json!(title));
    match input.get("ownerRoleId") {
        None | Some(Value::Null) => {},
        Some(Value::String(value)) => {
            args.insert("owner_role_id".to_string(), json!(value));
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `ownerRoleId` 必须是字符串（形状判在命令层）"
            )));
        },
    }
    call_opc_store(peer, OPC_STORE_CREATE_WORK_ITEM, Value::Object(args), call_chain)
}

/// `opc_kanban_board`：看板投影（读 op，无入参）。
fn kanban_board(
    _input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_KANBAN_BOARD, Value::Object(serde_json::Map::new()), call_chain)
}

/// `opc_work_item_start`：认领工作项（写 op）。
fn work_item_start(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_work_item_start";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_WORK_ITEM_START, json!({ "id": id }), call_chain)
}

/// `opc_work_item_review`：提交评审（写 op；质量门前置在宿主）。
fn work_item_review(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_work_item_review";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_WORK_ITEM_REVIEW, json!({ "id": id }), call_chain)
}

/// `opc_escalate_work_item`：升级阻塞（写 op）。
fn escalate_work_item(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_escalate_work_item";
    let id = require_str(input, OP, "id")?;
    let reason = require_str(input, OP, "reason")?;
    call_opc_store(
        peer,
        OPC_STORE_ESCALATE_WORK_ITEM,
        json!({ "id": id, "reason": reason }),
        call_chain,
    )
}

/// `opc_work_item_unblock`：解除阻塞（写 op）。
fn work_item_unblock(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_work_item_unblock";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_WORK_ITEM_UNBLOCK, json!({ "id": id }), call_chain)
}

/// `run_self_improving_opc_work_item`：自改进回合制闭环（写 op；规则化评估，不调 LLM）。
fn run_self_improving_work_item(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "run_self_improving_opc_work_item";
    let task = require_str(input, OP, "task")?;
    let mut args = serde_json::Map::new();
    args.insert("task".to_string(), json!(task));
    match input.get("maxRounds") {
        None | Some(Value::Null) => {},
        Some(value) if value.as_u64().is_some_and(|n| u32::try_from(n).is_ok()) => {
            args.insert("max_rounds".to_string(), value.clone());
        },
        Some(_) => {
            return Err(invalid_shape(format!(
                "{OP} 的 `maxRounds` 必须是 u32 范围内的整数（形状判在命令层）"
            )));
        },
    }
    call_opc_store(peer, OPC_STORE_RUN_SELF_IMPROVING_WORK_ITEM, Value::Object(args), call_chain)
}

fn require_str<'a>(input: &'a Value, op: &str, key: &str) -> Result<&'a str, (String, String)> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_shape(format!("{op} 需要字符串字段 `{key}`（形状判在命令层）")))
}

/// 可选字符串字段：缺键 / `null` 不转发；给了非字符串 ⇒ 拒收。
fn copy_optional_strings(
    input: &Value,
    op: &str,
    request: &mut serde_json::Map<String, Value>,
    keys: &[&str],
) -> Result<(), (String, String)> {
    for key in keys {
        match input.get(*key) {
            None | Some(Value::Null) => {},
            Some(Value::String(value)) => {
                request.insert((*key).to_string(), Value::String(value.clone()));
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "{op} 的 `{key}` 必须是字符串（形状判在命令层）"
                )));
            },
        }
    }
    Ok(())
}

/// 可选浮点字段（JSON 只有一种 number 类型，整数也算命中）。
fn copy_optional_f64(
    input: &Value,
    op: &str,
    request: &mut serde_json::Map<String, Value>,
    keys: &[&str],
) -> Result<(), (String, String)> {
    for key in keys {
        match input.get(*key) {
            None | Some(Value::Null) => {},
            Some(value) if value.as_f64().is_some() => {
                request.insert((*key).to_string(), value.clone());
            },
            Some(_) => {
                return Err(invalid_shape(format!("{op} 的 `{key}` 必须是数值（形状判在命令层）")));
            },
        }
    }
    Ok(())
}

/// 可选时间戳字段（整数；秒级 epoch，与库里的时间列同量纲）。
fn copy_optional_i64(
    input: &Value,
    op: &str,
    request: &mut serde_json::Map<String, Value>,
    keys: &[&str],
) -> Result<(), (String, String)> {
    for key in keys {
        match input.get(*key) {
            None | Some(Value::Null) => {},
            Some(value) if value.as_i64().is_some() => {
                request.insert((*key).to_string(), value.clone());
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "{op} 的 `{key}` 必须是整数时间戳（形状判在命令层）"
                )));
            },
        }
    }
    Ok(())
}

/// 必需的对象数组字段（元素内部字段类型由宿主 serde 判）。
fn object_array(input: &Value, op: &str, key: &str) -> Result<Value, (String, String)> {
    let items = input
        .get(key)
        .and_then(Value::as_array)
        .filter(|items| items.iter().all(Value::is_object))
        .ok_or_else(|| invalid_shape(format!("{op} 需要对象数组字段 `{key}`（形状判在命令层）")))?;
    Ok(Value::Array(items.clone()))
}

// ── 测试 ────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 只测三件纯函数事：回帧取列的容错、「出口零键名改写 + 键集合逐字 camelCase」的形态锁，
// 以及**创建命令的形状闸**（`blog_post_create_request`：五个字段齐 + 类型对才放行、
// 多余键不转发、语义闸不越界）。「真管道上发帧 / 分页取尽 / 三条写命令的出参两态与三态」
// 归 `tests/protocol.rs`（需要真进程与真 stdin/stdout）。
//
// ⚠ 三条写命令（[`publish_blog_post`] / [`delete_blog_post`] / [`create_blog_post`]）在这里
// **没有**端到端用例，不是漏测而是够不着：它们的签名要 `&mut WorkerPeer`（真管道才构造得出来），
// 入参形状与两态 / 三态透传全部归 `tests/protocol.rs` 的 `command_frame_*` 三条用例钉住 ——
// 那里同时锁出站帧的 seam / op 逐字、`args` 的键集合、以及 `not_found` / `protected`
// 走 payload 不走错误码。

#[cfg(test)]
mod tests {
    use super::*;

    fn seam_row() -> Value {
        json!({
            "id": "post-1",
            "title": "第一篇",
            "slug": "first-post",
            "excerpt": "摘要",
            "tags": ["opc", "blog"],
            "published": true,
            "publishedAt": 1_700_000_000_000_i64,
            "viewCount": 42,
            "createdAt": 1_700_000_000_000_i64,
            "updatedAt": 1_700_000_060_000_i64,
        })
    }

    /// 出口的「零键名改写」形态锁：命令出参逐字 = 接缝回帧里的行，键集合是那 10 个 camelCase
    /// 键（禁区 13）。前端表格列按 `dataIndex` 取值，键名漂一个字母不会报错，只会让那一列
    /// 静默变空 ⇒ 这是本命令唯一的形态锁。
    #[test]
    fn rows_pass_through_verbatim_with_exactly_the_camel_case_keys() {
        let rows = post_rows_of(&json!({ "posts": [seam_row()] }));
        assert_eq!(rows.len(), 1);
        let row = &rows[0];

        // ① 出口没有适配层：交出的行与接缝回帧逐字节相同。
        assert_eq!(*row, seam_row(), "出参必须逐字透传接缝视图，不得改写键名");

        // ② 键集合逐字钉住：接缝扩容新字段要连同前端一起走评审，不许静默出现在出参里。
        let mut keys: Vec<String> = row.as_object().expect("行应是对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "createdAt",
                "excerpt",
                "id",
                "published",
                "publishedAt",
                "slug",
                "tags",
                "title",
                "updatedAt",
                "viewCount"
            ]
        );

        // ③ snake_case 改写与正文列一律不许出现。
        for bad in ["published_at", "view_count", "created_at", "updated_at", "content"] {
            assert!(row.get(bad).is_none(), "出口不得再造 snake_case / 正文列键：{bad}");
        }
        assert_eq!(row["viewCount"], json!(42));
        assert_eq!(row["publishedAt"], json!(1_700_000_000_000_i64));
        assert_eq!(row["tags"], json!(["opc", "blog"]));
    }

    /// 草稿（`publishedAt = null`）必须透出 `null` 且**保留键**：契约里它是 `Option<i64>`，
    /// 序列化出的就是 `null`；前端把「值为 null」和「键不存在」当成两种东西（后者读出来是
    /// `undefined`，会落进缺省分支）。
    #[test]
    fn null_stays_null_and_key_still_present() {
        let mut row = seam_row();
        row["publishedAt"] = json!(null);
        row["published"] = json!(false);
        let rows = post_rows_of(&json!({ "posts": [row] }));
        assert_eq!(rows[0]["publishedAt"], json!(null));
        assert!(rows[0].get("publishedAt").is_some(), "键必须存在");
        assert_eq!(rows[0]["published"], json!(false));
    }

    #[test]
    fn posts_absent_or_wrong_type_yields_empty_page() {
        for page in [json!({}), json!({ "posts": null }), json!({ "posts": {} })] {
            assert!(post_rows_of(&page).is_empty(), "缺键 / 畸形 posts 都当空页：{page}");
        }
        let page = json!({ "posts": [seam_row()] });
        assert_eq!(post_rows_of(&page).len(), 1);
    }

    /// 创建命令的**形状闸**：五个字段齐 / 类型对才放行，且只转发这五个键。
    ///
    /// 另两档同样要钉：① 空串与超预算正文**照常放行**（语义闸在宿主 provider，本层越界就会
    /// 变成「两处各判一半」）；② 缺字段 / 类型不符**一律拒**（不发帧）。
    #[test]
    fn create_request_shape_gate_keeps_exactly_five_keys() {
        let ok = blog_post_create_request(&json!({
            "title": "标题",
            "slug": "first-post",
            "excerpt": "摘要",
            "content": "正文",
            "tags": ["opc", "blog"],
            // 多余键：不转发（宿主 serde 本来也会忽略未知字段，本层给它同一条口径）。
            "extra": 1,
        }))
        .expect("五个字段齐应放行");
        let mut keys: Vec<String> = ok.as_object().expect("应是对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["content", "excerpt", "slug", "tags", "title"],
            "只转发这五个键（接缝 args 的形状是契约）：{ok}"
        );
        assert_eq!(ok["tags"], json!(["opc", "blog"]));

        // 语义闸不在本层：空 slug / 超预算正文都放行（宿主 provider 会拒）。
        let semantic = blog_post_create_request(&json!({
            "title": "",
            "slug": "   ",
            "excerpt": "",
            "content": "x".repeat(70 * 1024),
            "tags": [],
        }));
        assert!(semantic.is_ok(), "空 slug / 超预算必须留给宿主判（两处各判一半必然漂移）");

        // 形状不符：缺字段、类型不符、tags 非字符串数组 —— 一律拒。
        for bad in [
            json!({}),
            json!({ "title": 1, "slug": "s", "excerpt": "e", "content": "c", "tags": [] }),
            json!({ "title": "t", "slug": "s", "content": "c", "tags": [] }),
            json!({ "title": "t", "slug": "s", "excerpt": "e", "content": "c", "tags": "opc" }),
            json!({ "title": "t", "slug": "s", "excerpt": "e", "content": "c", "tags": [1] }),
        ] {
            let (code, message) = blog_post_create_request(&bad).expect_err("形状不符必须拒");
            assert_eq!(code, error_codes::PLUGIN_COMMAND_INVALID_ARGS, "错误码必须逐字：{message}");
            assert!(!message.is_empty(), "错误文案必须说明缺什么");
        }
    }
}
