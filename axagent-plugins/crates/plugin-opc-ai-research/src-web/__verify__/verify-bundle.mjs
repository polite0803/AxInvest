// SPDX-License-Identifier: AGPL-3.0-only
/** ai_research 页面 bundle 的最小静态校验：页面命令只能走 executeCommand。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = [
  "opc_ai_research_opc_list_projects",
  "opc_ai_research_opc_search_wiki",
  "opc_ai_research_web_search",
  "opc_ai_research_file_read",
  "opc_ai_research_bash",
  "opc_ai_research_file_write",
  "opc_ai_research_run_automation_rules",
];
for (const command of commands) {
  if (!bundle.includes(`"${command}"`)) { throw new Error(`bundle 未包含命令 ${command}`); }
}
if (/client\.invoke\(\s*["']opc_/.test(bundle)) {
  throw new Error('ai_research 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: ai_research bundle 页面命令均走 executeCommand");
