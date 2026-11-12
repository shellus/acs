# Agent 环境基础

## 模块和命令

`crates/core/src/environment` 管理环境路径、本机配置、仓库声明、Profile 选择及只读校验。`model.rs` 定义 YAML 模型，`published.rs` 校验已发布 Skills 与快照，`state.rs` 定义后续 apply 使用的 JSON 状态。CLI 只解析参数和输出，不打开账号配置 Session。

```sh
acs env source set https://example.com/agent-env.git --branch main
acs env source show
acs env configure --profile default --agents codex,claude
acs env validate --repo ./example-env
```

configure 可只传一个选项以保留另一个已保存值；最终必须存在有效 Profile 和至少一个 Agent。当前入口要求显式参数；终端环境页由后续 TUI 实现。缺少命令或参数返回用法错误（退出码 2）。声明校验失败返回 1，成功返回 0。

source set 校验本地仓库声明，或通过系统 git 拉取来源后再保存；不会 apply。远端工作副本位于 cache/environment 下，缓存键是来源与分支连接后 SHA-256 的前 16 位。clone/fetch 仅发生在显式设置来源和 configure 时。validate 仅访问指定仓库，不读取本机环境配置，也不执行 git、npm、build 或 patch。

## 本机路径与格式

| 数据 | 路径与格式 |
| --- | --- |
| 配置 | ACS_HOME/environment.yaml；YAML，version: 1，environment 包含 source、branch、profile、agents |
| 缓存 | ACS_HOME/cache/environment、cache/sources、cache/builds（后两项供后续链路使用） |
| 应用状态 | ACS_HOME/state/environment.json；JSON version: 1，source、branch、commit、profile、agents |
| Agent 状态 | agents 以 codex/claude 为键，值包含 version、instruction_hash、skills（名称到哈希）、skills_root |
| Skills 标记 | Skills 根下 .acs-managed.json；JSON version: 1，skills（名称到哈希） |
| 排他锁 | ACS_HOME/operation.lock；与账号保存/历史修复共用 Store::lock |

ACS_HOME 默认用户目录下 .acs。CODEX_HOME 与 CLAUDE_CONFIG_DIR 控制对应 Agent 的配置与指令根。Codex Skills 固定为用户目录下 .agents/skills，Claude Skills 位于 CLAUDE_CONFIG_DIR/skills。CLI --root 优先于所有环境变量，采用 Roots::under，并将 Codex Skills 放到该隔离根下 .agents/skills。

本机配置缺失时返回默认值但不创建文件：version 为 1、branch 为 main，其余为空。保存时裁剪 source/branch/profile，两种 Agent 按名称排序，重复 Agent 报错。版本不支持、未知字段及未知 Agent 均拒绝读取。

## 仓库校验和选择

environment.yaml 与 environment.lock 保持 version 2 及既有结构；发布快照继续读取 skills.vendor 下的 .ags-snapshot.yaml，该名称属于仓库数据格式，不属于本机受管标记。所有模型拒绝未知字段。仅接受 codex、claude；其他 Agent 在解析时报告字段路径和 YAML 行列，不静默跳过。

Profile 与 Agent include/exclude 分别拼接；disabled_skills 仅允许 Codex，名称必须安全且不可重复。include 支持 *、Skill 名、单 Skill 来源名、来源:Skill 名和来源:glob；exclude 仅精确匹配名称或来源:名称，不作为 glob。重复选择同一 Skill 去重，不同来源的同名 Skill 报错，include 无匹配时报错。

validate 检查全局指令可读、local/vendor Skills 唯一、快照版本、锁、vendor_include、数量和逐项哈希，并验证各 Profile 的最终选择。文本哈希将 CRLF 规范为 LF，包含 NUL 的文件按原字节计算，与 AGS 相同。它不会检查上游 build/patch 的执行结果。

声明路径拒绝绝对路径、盘符及词法越界；local source、discover、build template/output、patch 文件及 patch 目标也适用。build template/output 必须成对；命令平台仅接受 all/windows/linux/darwin。patch 文件严格解析，检查 source、操作和正则。这些声明错误提前报告，不等到 vendor 执行时才失败。

## 事务与迁移边界

`transaction::apply` 对任意文件集合提供写入、删除、幂等及错误回滚，复用 storage::read/write 的原子替换与权限保持。提交前完整读取基线，拒绝空路径、目录和重复的规范化目标；已有文件链接解析到真实目标，链接保留，悬空链接拒绝写入。调用方持有 Store 锁。每次写入前重新校验字节；失败时逆序恢复，继续尝试所有回滚项，报告回滚失败，并保留回滚期间发现的外部改动。

事务只覆盖普通文件，不提供目录树替换或掉电恢复日志；apply 的 Skills 目录暂存、备份和恢复需在下一段建立。失败可能留下空父目录。新文件在 Unix 使用 tempfile 的私有权限，替换已有文件保持权限，回滚删除项也恢复其权限。

第二段迁移入口应显式读取旧配置、旧环境状态和旧受管标记，转换成 LocalConfig、State、ManagedMarker；不在普通 load 中保留兼容分支。旧本机配置路径由 AGS 的用户目录约定给出，缓存和状态可能来自系统目录或旧环境变量，不能仅从旧配置路径推断。缺失 skills_root 的旧状态需要迁移器按旧目标根补齐；不得将它默认映射到当前 Codex Skills 根。

只有 Codex 使用 .agents/skills，不需要多 Agent 共用根的选择一致性与引用计数逻辑。迁移时仍需核对历史根、旧标记与非受管内容，不能将整个目录视为 acs 所有。Codex 禁用块预留标记为 `# BEGIN ACS MANAGED CODEX DISABLED SKILLS` 和 `# END ACS MANAGED CODEX DISABLED SKILLS`，旧块替换及其幂等/异常标记测试属于下一段。
