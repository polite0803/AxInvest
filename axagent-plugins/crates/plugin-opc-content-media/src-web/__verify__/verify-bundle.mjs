// SPDX-License-Identifier: AGPL-3.0-only
/** content_media 页面 bundle 的最小静态校验：页面命令只能走 executeCommand。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = [
  "opc_list_blog_posts",
  "opc_create_blog_post",
  "opc_list_content_assets",
  "opc_create_content_asset",
  "opc_delete_content_asset",
  "opc_list_publish_schedules",
  "opc_create_publish_schedule",
  "opc_cancel_publish_schedule",
];
for (const command of commands) {
  if (!bundle.includes(`"${command}"`)) { throw new Error(`bundle 未包含命令 ${command}`); }
}
if (/client\.invoke\(\s*["']opc_/.test(bundle)) {
  throw new Error('content_media 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: content_media bundle 页面命令均走 executeCommand");
