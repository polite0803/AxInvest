// SPDX-License-Identifier: AGPL-3.0-only
/**
 * bot-ipc-audit P3 段的合成源（供 selftest 正控 / 单点破坏用）。
 * 与真实仓库无关：这里只保证「完整 P3 契约 ⇒ 0 错」，破坏任一项 ⇒ 对应红。
 */
const P3_CMDS = [
  ["create_bot_workflow", "bot_workflow"],
  ["update_bot_workflow", "bot_workflow"],
  ["list_bot_workflows", "bot_workflow"],
  ["delete_bot_workflow", "bot_workflow"],
  ["run_bot_workflow", "bot_workflow"],
  ["list_bot_workflow_runs", "bot_workflow"],
  ["cancel_bot_workflow_run", "bot_workflow"],
  ["create_bot_workflow_schedule", "scheduled_task"],
];
const NL = String.fromCharCode(10);
const FIELDS = {
  BotWorkflow: ["id", "name", "description", "owner_scope", "department_id", "steps",
    "max_concurrency", "total_timeout_secs", "created_at", "updated_at"],
  BotWorkflowRun: ["id", "workflow_id", "workflow_name", "status", "origin",
    "requester_bot_id", "requester_conversation_id", "anchor_message_id", "context",
    "completed_steps", "total_steps", "error_code", "error_detail", "started_at",
    "finished_at", "created_at", "updated_at"],
};

export function decorateWithP3(sources) {
  sources.eventsEnum = sources.eventsEnum.replace("AgentDone,",
    ["AgentDone,", "    BotAssignmentUpdated,", "    BotWorkflowRunUpdated,"].join(NL));
  sources.eventsTs = 'export const IPC_EVENT_NAMES = ["agent-done", '
    + '"bot-assignment-updated", "bot-workflow-run-updated"];';
  const registrations = P3_CMDS
    .map(([name, mod]) => "        commands::" + mod + "::" + name + ",").join(NL);
  sources.registerRs = sources.registerRs.replace("    ]", registrations + NL + "    ]");
  sources.frontendFiles[0].src += NL
    + 'await invoke<unknown>("list_bot_assignments", { rootId: null });';
  for (const [name, fields] of Object.entries(FIELDS)) {
    sources.typesRs += NL
      + '#[derive(Serialize, Deserialize)]' + NL
      + '#[serde(rename_all = "camelCase")]' + NL
      + "pub struct " + name + " {" + NL
      + fields.map((f) => "    pub " + f + ": String,").join(NL) + NL + "}" + NL;
    const camel = fields.map((f) => f.replace(/_([a-z])/g, (_, c) => c.toUpperCase()));
    sources.typesTs += NL + "export interface " + name + " {" + NL
      + camel.map((f) => "  " + f + ": string;").join(NL) + NL + "}" + NL;
  }
  sources.p3 = {
    workflowRs: P3_CMDS.filter(([, mod]) => mod === "bot_workflow")
      .map(([name]) => "pub async fn " + name
        + "() { let _ = IpcEventName::BotWorkflowRunUpdated.as_str(); }").join(NL),
    scheduleRs: 'pub(crate) const BOT_WORKFLOW_TASK_TYPE: &str = "bot_workflow";' + NL
      + 'pub async fn create_bot_workflow_schedule() '
      + '{ let _ = serde_json::json!({ "workflowId": "wf-1" }); }',
    servicesRs: "fn run() { is_bot_workflow_job(&job); }",
    assignmentRs: "pub async fn list_bot_assignments(root_id: Option<String>) -> Result<Vec<BotAssignment>, String> { todo!() }",
    delegateRs: 'fn input_schema(&self) -> Value { serde_json::json!('
      + '{ "properties": { "mode": { "type": "string" } } }) }',
  };
  return sources;
}
