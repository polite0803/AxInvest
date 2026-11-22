#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 17 个独立插件仓的远端发布编排。
 *
 * 默认只做只读检查（dry-run）；显式传 `--apply` 才会：
 *   1. 在指定 owner 下创建缺失的 GitHub 仓库；
 *   2. 把本地 `master` 推送到 `main` 并建立 tracking branch；
 *   3. 写入与 `plugin.json` 版本一致的 `v<version>` tag；
 *   4. 推送 tag，由仓内 release-plugin.yml 构建并发布六平台 Release。
 * Git HTTPS 不可用时，可加 `--api` 改走 GitHub Git Data API；该模式同样创建
 * 真实 commit 和 annotated tag，不会伪造 Release 资产。
 *
 * 本脚本不伪造远端结果。`--verify` 会核对仓内 metadata、remote、tag 与 GitHub
 * Release 的 `release.json` 下载地址；远端尚未完成时明确失败。
 */
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  PLUGIN_CONTRACT,
  PLUGIN_NAMES,
  pluginReposRoot,
  readPluginRepoMetadata,
} from "./lib/plugin-repos.mjs";

const ROOT = resolve(import.meta.dirname, "..");
const argv = process.argv.slice(2);
const APPLY = argv.includes("--apply");
const VERIFY = argv.includes("--verify");
const API_TRANSPORT = argv.includes("--api");
const OWNER = arg("--owner") ?? "polite0803";
const ROOT_DIR = resolve(arg("--root") ?? pluginReposRoot(ROOT));
const REMOTE = arg("--remote") ?? "origin";
const REMOTE_BASE = arg("--remote-base") ?? `https://github.com/${OWNER}`;
const ONLY = (arg("--only") ?? "")
  .split(",")
  .map((value) => value.trim())
  .filter(Boolean);
const DEFAULT_BRANCH = "main";
const RELEASE_TAG_PREFIX = "v";

function arg(name) {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : null;
}

function fail(message) {
  console.error(`[plugin-publish] FAIL: ${message}`);
  process.exit(1);
}

function run(command, args, cwd, options = {}) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
    ...options,
  });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  if (result.error || result.status !== 0) {
    fail(`${command} ${args.join(" ")} 失败：${result.error?.message ?? `exit ${result.status}`}\n${output}`);
  }
  return output;
}

function tryRun(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", windowsHide: true });
  return {
    ok: !result.error && result.status === 0,
    output: `${result.stdout ?? ""}${result.stderr ?? ""}`.trim(),
  };
}

function ghApi(method, endpoint, cwd, body) {
  const args = ["api", endpoint, "--method", method];
  if (body !== undefined) args.push("--input", "-");
  const output = run(
    "gh",
    args,
    cwd,
    body === undefined ? {} : { input: JSON.stringify(body) },
  );
  return output ? JSON.parse(output) : null;
}

function tryGhApi(method, endpoint, cwd) {
  const result = tryRun("gh", ["api", endpoint, "--method", method], cwd);
  return result.ok && result.output ? JSON.parse(result.output) : null;
}

function repoPath(plugin) {
  return join(ROOT_DIR, plugin);
}

function remoteUrl(metadata) {
  return metadata.repoUrl ?? `${REMOTE_BASE}/${metadata.repo}.git`;
}

function manifestUrl(metadata) {
  return `https://github.com/${metadata.owner}/${metadata.repo}/releases/latest/download/release.json`;
}

function verifyMetadata(plugin, owner = OWNER) {
  const repo = repoPath(plugin);
  if (!existsSync(join(repo, ".git"))) {
    fail(`${plugin} 不是独立 Git 仓：${repo}`);
  }
  const metadata = readPluginRepoMetadata(ROOT, plugin, undefined, ROOT_DIR);
  if (!metadata) {
    fail(`${plugin} 缺少发布元数据；先运行 generate-plugin-independent-repos.mjs --bootstrap-from-main --all`);
  }
  if (owner && metadata.owner !== owner) {
    fail(`${plugin} metadata.owner=${metadata.owner}，本次 --owner=${owner}`);
  }
  if (metadata.releaseManifestUrl !== manifestUrl(metadata)) {
    fail(`${plugin} metadata.releaseManifestUrl 与 owner/repo 不一致`);
  }
  return { plugin, repo, metadata };
}

function inspectRepo({ plugin, repo, metadata }) {
  const dirty = run("git", ["status", "--porcelain"], repo);
  if (dirty) fail(`${plugin} 工作树不干净，拒绝发布：\n${dirty}`);
  const branch = run("git", ["branch", "--show-current"], repo);
  if (!branch) fail(`${plugin} 处于 detached HEAD，拒绝发布`);
  const head = run("git", ["rev-parse", "--short", "HEAD"], repo);
  const remote = tryRun("git", ["remote", "get-url", REMOTE], repo);
  const tags = run("git", ["tag", "--list", `${RELEASE_TAG_PREFIX}*`], repo)
    .split(/\r?\n/)
    .filter(Boolean);
  return {
    plugin,
    repo,
    metadata,
    branch,
    head,
    remote: remote.ok ? remote.output : null,
    expectedRemote: remoteUrl(metadata),
    tags,
  };
}

function ensureRepo(repo, metadata, plugin) {
  const full = `${metadata.owner}/${metadata.repo}`;
  const present = tryRun("gh", ["repo", "view", full, "--json", "nameWithOwner"], repo);
  if (present.ok) {
    const visibility = tryRun("gh", ["repo", "view", full, "--json", "visibility"], repo);
    if (!visibility.ok || JSON.parse(visibility.output).visibility !== "PUBLIC") {
      fail(`${plugin} 远端 ${full} 不是公开仓，发布地址无法供未登录的主应用安装`);
    }
    return false;
  }
  run("gh", ["repo", "create", full, "--public"], repo);
  console.log(`[plugin-publish] ${plugin}: 已创建公开远端 ${full}`);
  return true;
}

function configureRemote(repo, metadata, plugin) {
  const expected = remoteUrl(metadata);
  const actual = tryRun("git", ["remote", "get-url", REMOTE], repo);
  if (actual.ok) {
    if (actual.output !== expected) {
      fail(`${plugin} remote ${REMOTE}=${actual.output}，期望 ${expected}`);
    }
    return;
  }
  run("git", ["remote", "add", REMOTE, expected], repo);
  console.log(`[plugin-publish] ${plugin}: 已绑定 ${expected}`);
}

function pushBranch(repo, plugin) {
  run("git", ["push", "-u", REMOTE, `HEAD:refs/heads/${DEFAULT_BRANCH}`], repo);
  console.log(`[plugin-publish] ${plugin}: 已推送 ${DEFAULT_BRANCH}`);
}

function createTag(repo, version, plugin) {
  const tag = `${RELEASE_TAG_PREFIX}${version}`;
  const exists = tryRun("git", ["rev-parse", "--verify", `refs/tags/${tag}`], repo);
  if (exists.ok) {
    const localCommit = run("git", ["rev-list", "-n", "1", tag], repo);
    const head = run("git", ["rev-parse", "HEAD"], repo);
    if (localCommit !== head) {
      fail(`${plugin} 已存在 ${tag}，但它不指向当前 HEAD；拒绝覆盖既有发布历史`);
    }
    return tag;
  }
  run(
    "git",
    ["-c", "user.name=Codex", "-c", "user.email=codex@example.com", "tag", "-a", tag, "-m", `${plugin} ${tag}`],
    repo,
  );
  return tag;
}

function pushTag(repo, tag, plugin) {
  run("git", ["push", REMOTE, `refs/tags/${tag}`], repo);
  console.log(`[plugin-publish] ${plugin}: 已推送 ${tag}`);
}

function listTrackedFiles(repo, plugin) {
  const result = spawnSync("git", ["ls-files", "--stage", "-z"], {
    cwd: repo,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    fail(`${plugin} 读取 tracked 文件失败：${result.error?.message ?? result.stderr}`);
  }
  return result.stdout.split("\0").filter(Boolean).map((record) => {
    const tab = record.indexOf("\t");
    const [mode, , stage] = record.slice(0, tab).split(" ");
    const path = record.slice(tab + 1);
    if (stage !== "0") fail(`${plugin} 含未解决索引冲突：${path}`);
    if (mode === "160000") fail(`${plugin} 含 submodule，Git Data API 发布暂不支持：${path}`);
    if (!["100644", "100755", "120000"].includes(mode)) {
      fail(`${plugin} 含不支持的 Git mode ${mode}：${path}`);
    }
    const [, , , sha] = record.slice(0, tab).split(" ");
    return { mode, path, sha };
  });
}

function ensureApiMainRef(repo, full, trackedFiles, plugin) {
  const main = tryGhApi("GET", `repos/${full}/git/ref/heads/${DEFAULT_BRANCH}`, repo);
  if (main) return main;
  const first = trackedFiles.find((file) => file.path === ".axinvest-plugin-repos.json")
    ?? trackedFiles[0];
  if (!first) fail(`${plugin} 没有可引导空仓的 tracked 文件`);
  ghApi("PUT", `repos/${full}/contents/${encodeURIComponent(first.path)}`, repo, {
    message: `chore: initialize ${plugin}`,
    content: readFileSync(join(repo, first.path)).toString("base64"),
  });
  console.log(`[plugin-publish] ${plugin}: 已 API 引导空仓 ${DEFAULT_BRANCH}`);
  const created = tryGhApi("GET", `repos/${full}/git/ref/heads/${DEFAULT_BRANCH}`, repo);
  if (!created) fail(`${plugin} API 引导后仍无 ${DEFAULT_BRANCH} 分支`);
  return created;
}

function remoteTreeMatches(repo, full, trackedFiles) {
  const tree = tryGhApi(
    "GET",
    `repos/${full}/git/trees/${DEFAULT_BRANCH}?recursive=1`,
    repo,
  );
  if (!tree || tree.truncated) return false;
  const remote = new Map(
    (tree.tree ?? [])
      .filter((entry) => entry.type === "blob")
      .map((entry) => [entry.path, `${entry.mode}:${entry.sha}`]),
  );
  if (remote.size !== trackedFiles.length) return false;
  return trackedFiles.every((file) => remote.get(file.path) === `${file.mode}:${file.sha}`);
}

function ensureApiTag(repo, full, plugin, version, mainSha, identity) {
  const tag = `${RELEASE_TAG_PREFIX}${version}`;
  const tagRefEndpoint = `repos/${full}/git/ref/tags/${encodeURIComponent(tag)}`;
  const existing = tryGhApi("GET", tagRefEndpoint, repo);
  if (existing) {
    console.log(`[plugin-publish] ${plugin}: 远端已存在 ${tag}，保留既有发布历史`);
    return;
  }
  const tagObject = ghApi("POST", `repos/${full}/git/tags`, repo, {
    tag,
    message: `${plugin} ${tag}`,
    object: mainSha,
    type: "commit",
    tagger: identity,
  });
  ghApi("POST", `repos/${full}/git/refs`, repo, {
    ref: `refs/tags/${tag}`,
    sha: tagObject.sha,
  });
  console.log(`[plugin-publish] ${plugin}: 已通过 API 创建 ${tag}`);
}

/**
 * 本机 Git HTTPS 不可用但 GitHub API 可用时，直接按 Git Data API 写入同一份
 * tracked 文件树。远端仍得到真实 Git commit/tag，不伪造 Release 或下载资产。
 */
function publishViaGitDataApi(repo, metadata, plugin, version) {
  const full = `${metadata.owner}/${metadata.repo}`;
  const files = listTrackedFiles(repo, plugin);
  const main = ensureApiMainRef(repo, full, files, plugin);
  const identity = {
    name: "Codex",
    email: "codex@example.com",
    date: new Date().toISOString(),
  };
  if (remoteTreeMatches(repo, full, files)) {
    console.log(`[plugin-publish] ${plugin}: 远端 ${DEFAULT_BRANCH} 内容一致，跳过提交`);
    ensureApiTag(repo, full, plugin, version, main.object.sha, identity);
    return;
  }
  console.log(`[plugin-publish] ${plugin}: API 上传 ${files.length} 个 tracked 文件`);
  const treeEntries = files.map((file, index) => {
    const blob = ghApi("POST", `repos/${full}/git/blobs`, repo, {
      content: readFileSync(join(repo, file.path)).toString("base64"),
      encoding: "base64",
    });
    if ((index + 1) % 50 === 0 || index + 1 === files.length) {
      console.log(`[plugin-publish] ${plugin}: 已上传 ${index + 1}/${files.length}`);
    }
    return {
      path: file.path,
      mode: file.mode,
      type: "blob",
      sha: blob.sha,
    };
  });
  const tree = ghApi("POST", `repos/${full}/git/trees`, repo, { tree: treeEntries });
  const commit = ghApi("POST", `repos/${full}/git/commits`, repo, {
    message: `chore: publish ${plugin} v${version}`,
    tree: tree.sha,
    parents: [main.object.sha],
    author: identity,
    committer: identity,
  });
  ghApi("PATCH", `repos/${full}/git/refs/heads/${DEFAULT_BRANCH}`, repo, {
    sha: commit.sha,
    force: false,
  });
  console.log(`[plugin-publish] ${plugin}: 已通过 API 更新 ${DEFAULT_BRANCH}=${commit.sha.slice(0, 7)}`);
  ensureApiTag(repo, full, plugin, version, commit.sha, identity);
}

function verifyRemote(repo, metadata, plugin, tag) {
  const full = `${metadata.owner}/${metadata.repo}`;
  const view = tryRun("gh", ["repo", "view", full, "--json", "defaultBranchRef,url,visibility"], repo);
  if (!view.ok) fail(`${plugin} 远端不存在：${full}`);
  const repoInfo = JSON.parse(view.output);
  if (repoInfo.visibility !== "PUBLIC") fail(`${plugin} 远端必须公开，实得 ${repoInfo.visibility}`);
  if (repoInfo.defaultBranchRef?.name !== DEFAULT_BRANCH) {
    fail(`${plugin} 远端默认分支应为 ${DEFAULT_BRANCH}，实得 ${repoInfo.defaultBranchRef?.name ?? "null"}`);
  }
  const release = tryRun("gh", ["release", "view", tag, "--repo", full, "--json", "assets,url"], repo);
  if (!release.ok) fail(`${plugin} 尚未发布 ${tag} Release`);
  const releaseInfo = JSON.parse(release.output);
  const assets = new Set((releaseInfo.assets ?? []).map((asset) => asset.name));
  if (!assets.has("release.json")) fail(`${plugin} Release ${tag} 缺少 release.json`);
  const archivePattern = new RegExp(
    `^${metadata.repo.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}-?${tag.slice(1)}?(?:-|_).*\\.tar\\.gz$`,
  );
  if (![...assets].some((name) => archivePattern.test(name) || name.endsWith(".tar.gz"))) {
    fail(`${plugin} Release ${tag} 缺少平台 .tar.gz 包`);
  }
  return {
    plugin,
    repoUrl: repoInfo.url,
    visibility: repoInfo.visibility,
    releaseUrl: releaseInfo.url,
    releaseJsonUrl: manifestUrl(metadata),
    assetCount: assets.size,
  };
}

function plan(rows) {
  for (const row of rows) {
    console.log(
      `[plugin-publish] ${row.plugin}: branch=${row.branch} head=${row.head} remote=${row.remote ?? "(missing)"} tags=${row.tags.length}`,
    );
  }
  console.log(
    `[plugin-publish] 计划：${rows.length} 个独立仓，owner=${OWNER}，branch=${DEFAULT_BRANCH}，transport=${API_TRANSPORT ? "api" : "git"}`,
  );
}

/** 主仓验收只核对元数据形状；网络/Release 验收由显式 `--verify` 负责。 */
export function verifyPluginReleaseRepos({ root = ROOT_DIR, owner = OWNER } = {}) {
  const results = [];
  for (const plugin of PLUGIN_NAMES) {
    const repo = join(root, plugin);
    if (!existsSync(join(repo, ".git"))) fail(`${plugin} 不是独立 Git 仓：${repo}`);
    const metadata = readPluginRepoMetadata(ROOT, plugin, undefined, root);
    if (!metadata) fail(`${plugin} 缺少发布元数据`);
    if (owner && metadata.owner !== owner) {
      fail(`${plugin} metadata.owner=${metadata.owner}，本次 --owner=${owner}`);
    }
    const expectedRepoUrl = metadata.repoUrl ?? `https://github.com/${metadata.owner}/${metadata.repo}.git`;
    const expectedReleaseUrl = `https://github.com/${metadata.owner}/${metadata.repo}/releases/latest/download/release.json`;
    if (metadata.releaseManifestUrl !== expectedReleaseUrl) {
      fail(`${plugin} metadata.releaseManifestUrl 与 owner/repo 不一致`);
    }
    const readme = readFileSync(join(repo, "README.md"), "utf8");
    if (!readme.includes(metadata.releaseManifestUrl)) {
      fail(`${plugin} README 未登记 ${metadata.releaseManifestUrl}`);
    }
    results.push({ plugin, repoUrl: expectedRepoUrl, releaseManifestUrl: metadata.releaseManifestUrl });
  }
  return results;
}

function main() {
  if (!existsSync(ROOT_DIR)) fail(`独立仓根不存在：${ROOT_DIR}`);
  const unknown = ONLY.filter((plugin) => !PLUGIN_NAMES.includes(plugin));
  if (unknown.length) fail(`--only 含未知插件：${unknown.join("、")}`);
  const rows = (ONLY.length ? ONLY : PLUGIN_NAMES)
    .map((plugin) => inspectRepo(verifyMetadata(plugin)));

  // fail-closed：待打 tag 的版本来自仓内 metadata，它必须与统一契约逐项相等。
  // 之前没有这道闸，契约翻牌后仍可能拿旧版本 tag 发布，远端 CI 全绿而版本语义已经错了。
  const contractByPlugin = new Map(PLUGIN_CONTRACT.plugins.map((p) => [p.name, p]));
  for (const row of rows) {
    const expected = contractByPlugin.get(row.plugin)?.version;
    if (!expected) fail(`契约缺少插件 ${row.plugin}`);
    if (row.metadata.pluginVersion !== expected) {
      fail(
        `${row.plugin}: metadata.pluginVersion=${row.metadata.pluginVersion} 与契约版本 ${expected} 不一致；` +
          "请先跑生成器同步并确认 plugin.json 版本，再发布",
      );
    }
  }

  if (VERIFY) {
    const results = rows.map((row) => {
      const tag = `${RELEASE_TAG_PREFIX}${row.metadata.pluginVersion}`;
      return verifyRemote(row.repo, row.metadata, row.plugin, tag);
    });
    console.log(JSON.stringify({ ok: true, owner: OWNER, root: ROOT_DIR, results }, null, 2));
    console.log("PLUGIN_RELEASE_REPOS_VERIFY_OK");
    return;
  }

  plan(rows);
  if (!APPLY) {
    console.log("PLUGIN_RELEASE_REPOS_DRY_RUN_OK");
    return;
  }

  if (!tryRun("gh", ["auth", "status"], ROOT).ok) {
    fail("gh 未登录或凭据无效");
  }

  for (const row of rows) {
    ensureRepo(row.repo, row.metadata, row.plugin);
    configureRemote(row.repo, row.metadata, row.plugin);
    if (API_TRANSPORT) {
      publishViaGitDataApi(row.repo, row.metadata, row.plugin, row.metadata.pluginVersion);
      continue;
    }
    pushBranch(row.repo, row.plugin);
    const version = row.metadata.pluginVersion;
    const tag = createTag(row.repo, version, row.plugin);
    pushTag(row.repo, tag, row.plugin);
  }
  console.log("PLUGIN_RELEASE_REPOS_PUSHED");
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(import.meta.filename)) {
  main();
}
