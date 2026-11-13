# 从 AGS 切换到 acs

AGS 的环境管理已并入 acs，独立执行器退役。`acs env import-ags` 显式接管本机选择、状态、受管标记和 Codex 禁用块；不导入 Provider，不删除旧程序、配置、缓存或状态。迁移入口之外不读取旧格式。

## 发布与切换顺序

1. 先合并并发布 acs：CLI 必须支持 `.vendor-snapshot.yaml`，Release 同时包含平台产物与 SHA-256，安装脚本随源码发布。
2. 发布已验证的环境仓库切换分支，暂不合入 main。该分支使用新快照名，且只声明 Codex 与 Claude。
3. 各机器安装新 CLI、导入旧状态，再将来源设为切换分支。查看差异、确认应用，直到 status/diff 无漂移并完成客户端检查。
4. 所有机器切换后再合并环境仓库 main，逐台将本机来源分支改回 main 并验证。之后才清理旧程序与目录、归档旧执行器仓库。

新 CLI 不能消费未切换的旧快照，旧执行器也不能消费新仓库。导入后不能交替运行两套 apply。切换分支有全局指令和 Skills 的正常更新，首次 status/diff 允许有这些明确变化，应用后才应无漂移。

## 已有机器

保留原 HOME、Agent 路径和代理环境。先记录原来源、Profile、启用 Agent 及旧路径覆盖，备份旧配置/状态、Codex config.toml、全局指令和所有受管 Skills 根；原状态应无漂移。安装方法见 [CLI 分发](cli-maintenance.md#安装-cli)。

安装后分别在 Windows PowerShell 或 Linux shell 运行同一组 CLI 命令，来源替换为已发布且已校验的环境仓库：

```sh
acs self version
acs env import-ags
acs env source set https://example.com/agent-env.git --branch feat/acs-cutover
acs env source show
acs env status
acs env diff
acs env apply --yes
acs env status
acs env diff
acs doctor
```

导入保留原 Profile 和 Codex/Claude 选择；旧配置中不再支持的项会提示丢弃。只有未保留任何 Agent 或要改变选择时，才显式 `acs env configure --profile <名称> --agents codex,claude`。首次导入前不要先 source set/configure 创建新的 acs 环境，否则会拒绝覆盖。

若没有旧环境，直接 source set/configure，跳过导入。导入本身不要求访问来源；新来源应在导入后、首次 status/diff 之前设置。第一次 apply 可能安装锁要求的 Agent 版本、准备 Node 依赖和更新正常漂移；确认后新的 status/diff 应显示无漂移，doctor 返回 0。

## 旧路径与幂等

旧配置默认 `~/.ags/config.yaml`。Windows 默认状态为 `%APPDATA%/ags/environment.json`，缓存 `%LOCALAPPDATA%/ags`；Linux 默认状态为 `~/.local/state/ags/environment.json`，缓存 `~/.cache/ags`。状态遵守 `AGS_STATE_DIR`、`XDG_STATE_HOME`，Unix 缓存遵守 `XDG_CACHE_HOME`；覆盖值指向的实际目录必须另行记录和清理。

```sh
acs env import-ags --ags-config /example/old-config.yaml --ags-state /example/old-environment.json
```

Windows 同样接受这两个参数。收据为 `ACS_HOME/state/environment-import.json`，只记录导入来源和哈希；重复导入同一旧状态不写文件。已有 acs 环境且无匹配收据时拒绝覆盖，不应删除 acs 配置强行重试。早期构建的导入收据如需保留幂等，应在备份后一次性改到当前规范文件名；普通读取不兼容旧文件名。

## 清理与恢复边界

清理旧执行器前，确认 acs 命令确实指向新安装位置，两个 Agent 的指令、Skills、禁用块均可使用，保留切换前备份。仅删除旧可执行文件及旧工具专属目录，不删除 `.codex`、`.claude`、`.agents` 或环境数据仓库。

导入和文件应用有进程内事务恢复，但不支持掉电恢复。全局 npm 安装不属于文件事务；清理旧目录不可由 import 撤销。若需退回旧执行器，必须恢复成套旧配置/状态、标记、指令、Skills 和旧仓库提交，不能仅复制旧状态文件或重新运行旧 apply。
