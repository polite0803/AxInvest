# AxAgent — CLAUDE.md

> ⚠ **本文件已收敛为「薄壳」（thin shell），不再承载任何规范内容。**
>
> **项目规范的唯一权威是 `AGENTS.md`。** 下方导入语句会在会话启动时把该文件**整篇展开**进上下文，
> 因此这里既不需要、也不允许再复制任何规则。
>
> **禁止在本文件新增 / 复制 / 节选规则** —— 双副本必然漂移，其中滞后的那一份会向 AI 撒谎。
> 本项目确有先例：本文件曾整体滞后于 `AGENTS.md`（命令数 / crate 数 / 权威层级全错），
> 且有一条**教人去手工编辑构建生成物** —— 照做就是必被覆盖的假修复。
>
> 需要改规范 → **只改 `AGENTS.md`**；本文件不需要任何同步动作。

@AGENTS.md

---

## 插件独立仓的位置（仅指路，不是规则）

17 个业务插件的源码权威已迁到 `/home/polit/projects/AxInvest-plugin-repos/<plugin>/`，
每个目录都是独立 Git 仓（`github.com/polite0803/axinvest-plugin-<plugin>`）。

- 改插件代码 → 在对应插件仓里开会话，规则读 `/home/polit/projects/AxInvest-plugin-repos/AGENTS.md`。
- 本仓 `axagent-plugins/` 只保留宿主 loader、`plugin-opc-core`、`plugin-testkit`、`schema-gen`；
  `crates/` 下残留的 17 个业务插件目录是待清理副本，不要在那边改。
- 本文件不导入插件仓规范（避免把 17 仓规则整篇灌进主仓会话），需要时按上面的路径去读。
