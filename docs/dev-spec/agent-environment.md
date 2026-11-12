# Agent 环境管理

## 入口与分层

环境业务位于 `crates/core/src/environment`，CLI 只解析参数、展示计划和确认。`model` 处理严格 YAML 模型，`published` 校验快照与选择，`agent` 管理版本和 npm，`command::Runner` 提供可替换执行器，`disabled` 管理 Codex 文本块，`service` 生成与应用计划，`tree` 将目录替换和文件事务组合，`migration` 只负责显式一次性导入。

```sh
acs env source set https://example.com/agent-env.git --branch main
acs env source show
acs env configure --profile default --agents codex,claude
acs env validate --repo ./example-env
acs env diff
acs env apply --yes
acs env status
acs agent status all
acs agent install codex
acs agent uninstall codex --yes
acs env import-ags
```

新命令输出中文文本，当前不提供 JSON 输出。configure 支持部分修改；Agent 选择可用 codex、claude、all，all 展开后只保存具体 Agent，重复选择去重。全局 `--agent` 属于账号命令，与 env/agent 分组同时使用会报用法错误，避免静默选择不同目标。非终端 apply/uninstall 必须显式 `--yes`，缺参数退出 2；业务失败退出 1。status/diff 有漂移时展示计划并退出 0，不自动 apply。

## 路径和本机数据

四个配置环境变量统一由 `storage::Roots` 解析，账号与环境命令共用。保留历史账号入口语义：变量存在时按原路径使用，包括空字符串与字面 `~`；仅未设置才回退。第一段 env 特有的 `~` 展开、空值回退已取消。建议 shell 展开主目录或传绝对路径，避免把空值解释成当前目录。桌面配置的系统目录发现仍使用 dirs 提供的平台规则。

| 数据 | 路径与格式 |
| --- | --- |
| 本机选择 | ACS_HOME/environment.yaml，YAML v1；environment 含 source、branch、profile、agents |
| 来源缓存 | ACS_HOME/cache/environment；来源与分支组合的 SHA-256 前 16 位作为目录键 |
| 构建暂存 | ACS_HOME/cache/builds；只暂存已发布快照，计划销毁后清理 |
| 应用状态 | ACS_HOME/state/environment.json，JSON v1；source、branch、commit、profile、agents |
| Agent 状态 | version、instruction_hash、skills（名称到哈希）、skills_root |
| Skills 标记 | Skills 根下 .acs-managed.json，JSON v1，skills（名称到哈希） |
| 迁移收据 | ACS_HOME/state/ags-import.json，JSON v1；旧配置/状态路径及其联合内容哈希 |
| 排他锁 | ACS_HOME/operation.lock，与账号保存、历史修复共用 Store::lock |

ACS_HOME 默认用户目录下 .acs；CODEX_HOME 默认 .codex；CLAUDE_CONFIG_DIR 默认 .claude。Codex Skills 固定在用户目录下 .agents/skills，Claude Skills 位于其配置根下 skills。`--root` 优先于环境变量，采用 Roots::under，Codex Skills 放到该隔离根下 .agents/skills；导入默认也只从该隔离根寻找旧布局。

来源为本地目录时直接读取；远端仅同步环境数据仓库。clone 在缓存同级临时目录完成后再 rename，失败不留下可见半成品；既有缺少 .git 的缓存重建，已有不同 origin 拒绝复用。缓存源同步不修改原始本地来源。source set 和 configure 不写 Agent 目标文件。

## 快照、计划和应用

environment.yaml/environment.lock 使用 version 2，所有模型拒绝未知字段；只接受 codex/claude，其他 Agent 报字段路径和 YAML 行列。快照文件 .ags-snapshot.yaml 属于既有仓库数据格式，继续保留其名称。

Profile 与 Agent 的 include/exclude 分别拼接。include 支持 *、Skill 名、单 Skill 来源、来源:名称和来源:glob；exclude 精确匹配名称或来源:名称。重复选择去重，同名不同来源及无匹配 include 报错。disabled_skills 仅允许 Codex，名称须安全且不可重复。

validate 检查全局指令、local/vendor 唯一性、快照锁与选择、数量、哈希和每个 Profile 的选择；文本哈希将 CRLF 规范为 LF，含 NUL 的文件按原字节计算。声明路径不能绝对或越界；build template/output 成对，平台仅 all/windows/linux/darwin；patch 文件严格解析 source、操作和正则。validate 不运行 git/npm/build/patch。

prepare 同步环境仓库，校验后只消费 skills/local、skills/vendor；不会调用上游 sources 的 clone、build 或 patches。与 AGS 一致，Node Skill 要求 package-lock.json，并在暂存目录用 npm ci 准备运行依赖（禁用 Playwright 浏览器自动下载）。已安装内容匹配受管哈希且快照载荷未变化时复用完整运行目录，不重复 npm ci；变化或新增的 Node Skill 可能在 diff/status 准备阶段访问 npm。此过程不安装全局包。

计划明确列出新增、更新、同名非受管内容接管、受管项删除及历史根清理。用户修改过的受管内容按 AGS 语义计划更新，非受管同名内容按接管处理，必须随 apply 确认；无关非受管项、旧根中的个人内容和系统 Skills 保留。文件或链接形式的 Skills 根可以计划接管，目录链接本身被替换，其指向的数据不改动。

Codex 使用 `# BEGIN ACS MANAGED CODEX DISABLED SKILLS` / `# END ACS MANAGED CODEX DISABLED SKILLS`，生成排序后的 skills.config 数组，保留其余文本和换行风格；重复、缺失、逆序标记拒绝写入。账号保存先拆出此块，再编辑其余 TOML，最后原样接回；env 只更新块。两条路径共用操作锁和目标基线，旧账号草稿不会覆盖后来 apply 的禁用配置。

apply 校验计划时的配置与 Skills 基线，然后按 pin/minimum 要求安装 Agent；非 pin 不降级，无法解析的已安装版本保留。Skills 在目标同级暂存，通过 rename 保存旧目录实体/链接并替换，之后调用通用文件事务写指令、Codex 配置和状态。失败逆序恢复；外部再次修改的目录不覆盖，并保留备份、报告路径。没有变更时不写目标文件、标记或状态，也不改修改时间。npm 全局安装/卸载不是文件事务的一部分，文件应用失败不会尝试降级或重装 npm 包。

uninstall 默认只移除受管内容、对应受管指令和禁用块，保留非受管内容；优先使用已配置仓库的 npm 包名。`--purge` 另行删除选定客户端的整个配置根，属于用户显式选择的清理；npm 操作和 purge 无文件事务回滚保证。

## 显式导入

普通读取不兼容旧本机格式。`env import-ags` 单次转换旧本机配置、状态、活动/历史 Skills 根下标记以及禁用块；旧 AGS 目录、状态文件和二进制保留。旧配置与状态中的不再支持项仅在迁移器过滤，并显示提示。缺失 skills_root 的旧状态按原 Codex/Claude 根补齐，不猜测为新共享根。

旧配置默认用户目录下 .ags/config.yaml。Windows 状态默认 APPDATA/ags/environment.json，Linux 默认 XDG_STATE_HOME/ags/environment.json 或 .local/state/ags/environment.json，AGS_STATE_DIR 优先；旧缓存按 LOCALAPPDATA 或 XDG_CACHE_HOME/.cache 定位，但不复制缓存。可用 `--ags-config <文件>`、`--ags-state <文件>` 显式指定旧文件。

转换计划完整预检后作为文件事务一起提交，收据最后写入。重复执行相同旧状态只报告已导入；已有 acs 环境配置或状态且无匹配收据时拒绝覆盖。两个品牌的禁用块同时存在、ACS 标记冲突、损坏旧块或异常旧数据均阻止迁移，不留下部分接管。

## 后续链路接口

第三段 lock/vendor 可复用 Repository/SourceLock、compare_versions、command::Runner、published::tree_hash 和通用事务。来源缓存自愈集中在 source_repository；后续源缓存建议单独分配 cache/sources，不能将其当作运行时已发布快照。doctor 可以组合 LocalConfig/Repository/State 校验、Service::prepare 和执行器探测；自更新应独立于配置/Skills 事务。

Plan 持有临时渲染工作区，含人可读 changes、AgentPackage 与已安装版本；apply 必须消费同一计划并复核基线。目录事务当前提供进程内错误回滚，不提供掉电恢复日志。Linux musl 依赖仍由既有 CI 验证；新增能力仅在显式 env/agent 命令下调用系统 git/npm。
