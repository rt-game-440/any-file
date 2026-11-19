# Issue 追踪器：GitHub

本仓库的 Issue 与规格说明（spec）托管在 GitHub Issues 中，所有操作统一使用 `gh` CLI。

## 常用约定

- **创建 Issue**：`gh issue create --title "..." --body "..."`。多行正文请使用 heredoc 或文件传入。
- **读取 Issue**：`gh issue view <number> --json number,title,body,labels,comments`。
- **列出 Issue**：`gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`，按需配合 `--label` 与 `--state` 过滤。
- **将 Issue 关联为父 Issue 的子 Issue**：使用 `gh issue create --parent <parent> ...`，或创建后执行 `gh issue edit <parent> --add-sub-issue <child>`（需要 `gh` 2.94+）。旧版本 `gh` 可用：`gh api --method POST repos/<owner>/<repo>/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>`（这里的 id 为数据库数字 ID，见下方 **阻塞关系**）。若未启用 sub-issues，则在子 Issue 正文顶部写 `Part of #<parent>`。
- **阻塞关系（Blocking）**：优先使用 GitHub 原生 Issue 依赖（UI 可见的标准方式）。通过 `gh issue create --blocked-by <n>,<n> ...` 或 `gh issue edit <child> --add-blocked-by <blocker>` 添加依赖（需要 `gh` 2.94+）。旧版本 `gh` 可用：`gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`，其中 `<blocker-db-id>` 是阻塞方的数字**数据库 ID**（通过 `gh api repos/<owner>/<repo>/issues/<n> --jq .id` 获取，**不是** `#number` 或 `node_id`）。若原生依赖不可用，则退化为在子 Issue 正文顶部写 `Blocked by: #<n>, #<n>`。当所有阻塞方均已关闭时，该工单即解除阻塞。
- **统计未关闭的阻塞项**：`gh api repos/<owner>/<repo>/issues/<n> --jq .issue_dependencies_summary.blocked_by`（仅统计未关闭的阻塞项；为 0 表示可开工）。`gh issue view <n> --json blockedBy` 也会包含已关闭的阻塞项，需配合过滤：`--jq '[.blockedBy.nodes[] | select(.state == "OPEN")] | length'`。
- **评论 Issue**：`gh issue comment <number> --body "..."`
- **添加 / 移除标签**：`gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **关闭 Issue**：先用 `gh issue comment <number> --body "..."` 说明原因，再执行 `gh issue close <number>`。对于已关闭的 Issue，`gh issue close --comment` 会丢弃评论且依然返回退出码 0，因此需分两步执行。

仓库信息从 `git remote -v` 自动推断；在本地克隆目录内执行 `gh` 时会自动识别当前仓库。

只允许向用户自己的仓库提交 Issue，严禁向 `mattpocock/skills`（技能源仓库）提交 Issue（除非用户是其所有者）。在 fork 仓库中 `gh` 可能默认指向上游父仓库，因此在会话首次执行 `gh issue create` 前，请先运行 `gh repo view --json nameWithOwner` 检查目标仓库；如不是用户的仓库，须先向用户确认。

## 将 Pull Request 作为分诊入口（Pull requests as a triage surface）

**PRs as a request surface: no.** _（若本仓库将外部 PR 视为功能请求纳入分诊队列，请将其改为 `yes`；`/triage` 会读取此开关。）_

当设置为 `yes` 时，PR 与 Issue 共用同一套标签和状态，对应使用 `gh pr` 命令：

- **读取 PR**：先执行 `gh pr view <number>`，再执行 `gh pr view <number> --comments`（非 TTY 环境下 `--comments` 仅输出评论而不含正文），查看代码变更用 `gh pr diff <number>`。
- **列出待分诊的外部 PR**：`gh api --paginate 'repos/{owner}/{repo}/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`。
- **评论 / 打标 / 关闭**：`gh pr comment`、`gh pr edit --add-label` / `--remove-label`、`gh pr close`。

GitHub 的 Issue 与 PR 共用编号空间，因此裸编号 `#42` 可能是其中任意一种：优先用 `gh pr view 42` 解析，失败再回退到 `gh issue view 42`。

## 当技能要求“publish to the issue tracker”时

创建一个 GitHub Issue。

## 当技能要求“fetch the relevant ticket”时

按上方 **读取 Issue** 的方式读取对应 Issue。

## Wayfinding 操作

供 `/wayfinder` 使用。**map** 是单个主 Issue，**child** 是其下的子工单 Issue。

- **Map**：带有 `wayfinder:map` 标签的单个 Issue，正文记录 Notes / Decisions-so-far / Fog。使用 `gh issue create --label wayfinder:map` 创建。
- **子工单（Child ticket）**：作为 GitHub sub-issue 关联到 map（见上方 **将 Issue 关联为父 Issue 的子 Issue**）。若未启用 sub-issues，则在 map 正文的任务列表中加入该子 Issue，并在子 Issue 正文顶部写 `Part of #<map>`。标签使用 `wayfinder:<type>`（`research` / `prototype` / `grilling` / `task`）。认领后将工单指派给当前执行者。
- **阻塞关系**：见上方常用约定中的 **阻塞关系（Blocking）**。
- **前沿查询（Frontier query）**：列出 map 下所有未关闭的子 Issue（`gh issue list --state open`，限定在 map 的子 Issue 或任务列表范围内），排除存在未关闭阻塞项（见 **统计未关闭的阻塞项** 或 `Blocked by` 行中仍有未关闭 Issue）以及已被指派的工单；按 map 中的先后顺序取第一个。
- **认领（Claim）**：`gh issue edit <n> --add-assignee @me`，作为会话中的首次写操作。
- **解决（Resolve）**：先执行 `gh issue comment <n> --body "<answer>"`，再执行 `gh issue close <n>`，最后将结论摘要与链接追加到 map 的 Decisions-so-far 中。
