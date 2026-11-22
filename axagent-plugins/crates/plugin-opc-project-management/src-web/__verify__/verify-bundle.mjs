// SPDX-License-Identifier: AGPL-3.0-only
/** project_management 页面 bundle 的最小静态校验：页面命令只能走 executeCommand。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = [
  "opc_project_management_opc_create_project",
  "opc_project_management_opc_add_milestone",
  "opc_project_management_opc_list_projects",
  "opc_project_management_run_automation_rules",
];
for (const command of commands) {
  if (!bundle.includes(`"${command}"`)) { throw new Error(`bundle 未包含命令 ${command}`); }
}
if (/client\.invoke\(\s*["']opc_/.test(bundle)) {
  throw new Error('project_management 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: project_management bundle 页面命令均走 executeCommand");
