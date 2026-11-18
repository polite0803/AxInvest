// SPDX-License-Identifier: AGPL-3.0-only
/** 最小静态校验：bundle 页不出现被 SDK 白名单拒绝的 client.invoke("opc_*")。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = ["opc_list_invoices", "opc_create_invoice", "opc_transition_invoice"];
for (const command of commands) {
  if (!bundle.includes(`"${command}"`)) { throw new Error(`bundle 未包含命令 ${command}`); }
}
if (/client\.invoke\(\s*["']opc_/.test(bundle)) {
  throw new Error('accounting 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: accounting bundle 三条命令均走 executeCommand");
