// SPDX-License-Identifier: AGPL-3.0-only
/** software_dev 页面 bundle 的最小静态校验：页面命令只能走 executeCommand。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = [
  "opc_software_dev_opc_list_projects",
  "opc_software_dev_opc_create_project",
  "opc_software_dev_file_write",
  "opc_software_dev_web_search",
  "opc_software_dev_opc_add_milestone",
  "opc_software_dev_batch_validate_entities",
  "opc_software_dev_run_automation_rules",
];
for (const command of commands) {
  if (!bundle.includes(`"${command}"`)) { throw new Error(`bundle 未包含命令 ${command}`); }
}
if (/client\.invoke\(\s*["']opc_/.test(bundle)) {
  throw new Error('software_dev 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: software_dev bundle 页面命令均走 executeCommand");
