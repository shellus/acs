# Agent 环境

acs 在各机器拉取独立环境仓库，按 Profile 和启用的 Codex/Claude 同步安装版本、全局指令、Skills 和 Codex 禁用项。账号、自定义 API、登录凭据和代理继续由账号功能管理，不进入环境仓库。AGS 的环境管理能力已并入 acs，旧机器使用[显式导入](agent-env-cutover.md)切换。

## 配置与日常同步

```sh
acs env source set https://example.com/agent-env.git --branch main
acs env source show
acs env configure --profile default --agents codex,claude
acs env status
acs env diff
acs env apply --yes
acs env status
acs agent status all
acs doctor
```

来源可为本地目录或 Git URL；本地目录使用当前内容，不 checkout 分支，远端来源按配置分支更新本机缓存。configure 可只修改 Profile 或 Agent；这些选择保存在 `ACS_HOME/environment.yaml`，默认 `~/.acs`。status/diff 展示具体变化但不写目标内容；没有漂移时 apply 不改写文件或时间。非终端 apply 必须显式 `--yes`。

Codex Skills 使用 `~/.agents/skills`，Claude 使用其配置目录下的 `skills`。`CODEX_HOME` 与 `CLAUDE_CONFIG_DIR` 控制配置根，`ACS_HOME` 控制 acs 本机数据。`acs --root <目录> ...` 将账号配置、环境缓存/状态与两个 Agent 目标一起隔离；不读取真实用户环境。

apply 只使用已发布 local/vendor 内容，不执行上游 build/patch。Node Skill 新增或更新时可能按 package-lock 执行 npm ci；需要 git、Node.js/npm 和相应网络。Agent 缺失或版本不足时，确认应用后可能运行 npm 全局安装。pin 表示精确版本，否则锁定版本是最低要求；无关非受管内容保留，同名接管与用户修改的受管内容会在计划中列出。

## TUI

终端运行 `acs`，按 `v` / F3 或 Tab 切到环境页。页面展示来源、Profile、启用 Agent、安装版本和逐项漂移；`R` 刷新、`a` 查看确认、`y` 应用、`n` / Esc 取消，`d` 诊断。↑↓/j k、PgUp/PgDn、Home/End 滚动完整长列表，Esc/q 返回账号页。

应用复用确认时的计划，基线变化时拒绝旧计划并重新读取，必须再次确认。后台阶段计数和等待时间持续更新；Ctrl+C 请求任务完成后退出，避免中断事务。来源/Profile 未配置时页面提供 CLI 配置命令。完整按键见 [CLI 用法](usage.md#全屏-tui)。

## 仓库维护与开发检出

环境仓库包含严格 version 2 的 `environment.yaml`、`environment.lock`，以及 instructions、local、vendor、sources/build/patch 声明。vendor 元数据为 `skills/vendor/.vendor-snapshot.yaml`；只支持当前文件名。环境仓库的内容与快照必须一起更新。

在 acs 根目录开发时，约定单独 clone 到固定的 `agent-env/`；acs 已忽略该路径，不能添加为子模块或提交其中内容。现有独立工作副本/worktree 不自动搬动，运行时代码不假定此目录存在：

```sh
git clone https://example.com/agent-env.git agent-env
acs env validate --repo ./agent-env
acs env vendor --repo ./agent-env
acs env source set <agent-env的绝对路径>
```

本地来源建议使用绝对路径，相对路径按运行命令时的工作目录解释；Windows 可用 `(Resolve-Path ./agent-env).Path` 得到绝对路径。常规客户端从远端配置来源，不要求检出到 acs 源码目录。`env lock/vendor`、SHA-256 安装脚本、`doctor` 和 CLI 自更新见 [CLI 维护与分发](cli-maintenance.md)，数据与核心入口见 [开发规范](../dev-spec/agent-environment.md)。
