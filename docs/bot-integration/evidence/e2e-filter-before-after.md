# e2e 过滤口径：修前（标题正则）vs 修后（显式 tag）

日期：2026-11-22
仓库：`/home/polit/projects/AxInvest`（WSL ext4）
基线 commit：`6b9e5ea85`（前端 Bot UI 已落地）

## 1. 分母

```console
$ npx playwright test --list --project=chromium
Total: 134 tests in 24 files
```

## 2. 修前：`--grep-invert "(Agent|Cache|Conversation|Gateway)"`

```console
$ npx playwright test --list --project=chromium --grep-invert "(Agent|Cache|Conversation|Gateway)"
Total: 110 tests in 19 files
```

→ **收集 110 / 跳过 24**。其中 18 条是真正想排除的重用例（5 个整文件打
`@heavy`），另外 **6 条是被标题正则误伤的正常用例**（Playwright 的 grep
大小写不敏感，`agent`/`cache`/`conversation`/`gateway` 都命中）：

| 文件 | 用例 | 误伤原因 |
| --- | --- | --- |
| `e2e/chat-navigation.spec.ts:13` | should display new conversation button | 标题含 `conversation` |
| `e2e/chat-navigation.spec.ts:21` | should display agent status indicator | 标题含 `agent` |
| `e2e/chat-navigation.spec.ts:45` | should display cache indicator | 标题含 `cache` |
| `e2e/chat-navigation.spec.ts:65` | should navigate to gateway from chat | 标题含 `gateway` |
| `e2e/cross-page-navigation.spec.ts:22` | should load Gateway page (/gateway) | 标题含 `Gateway` |
| `e2e/bots.spec.ts` | Bot 列表可见：标题含 Conversation 的用例… | 新增用例，刻意保留 `Conversation` 一词作活证据 |

**后果**：CI 报「e2e 全绿」，实际有 5 条既有用例从未执行；更严重的是
**任何新写的 Bot 用例只要标题带上 `Agent`/`Conversation`，就会被静默跳过**。

## 3. 修后：`--grep-invert "@heavy"`

```console
$ npx playwright test --list --project=chromium --grep-invert "@heavy"
Total: 116 tests in 19 files
```

→ **收集 116 / 跳过 18**。跳过的 18 条全部是显式标注的 `@heavy`
（`agent-execution` / `cache-config` / `conversation-lifecycle` / `gateway` /
`platform-gateway` 五个文件），误伤为 0。`e2e/bots.spec.ts` 的 6 条全部在
收集结果中。

## 4. 修后全量执行（CI 同等条件：`CI=1`，串行 + 2 次重试）

```console
$ CI=1 npx playwright test --project=chromium --grep-invert "@heavy" --timeout=120000
  2 skipped
  114 passed (7.9m)
```

- 114 = 116 收集 − 2 个既有的 `test.fixme`（`e2e/workflow-editor.spec.ts:92`、
  `:100`，与 Bot 无关，长期挂起）。
- 0 failed。
- 被旧口径误伤的 5 条既有用例本次全部执行并通过。

## 5. Bot 用例自身的稳定性

首次非 CI 并行跑（`workers` 默认 8+、`retries=0`）时，「刷新后仍归属」一条
偶发红：UI 已出现线程，但 `localStorage` 尚未写入就刷新了。已改为先等落库：

```ts
await page.waitForFunction(() => {
  const raw = localStorage.getItem("axagent_conversations");
  return JSON.parse(raw ?? "[]").length > 0;
});
```

复跑：

```console
$ npx playwright test e2e/bots.spec.ts --project=chromium --repeat-each=3 --workers=8
  18 passed (45.8s)
```
