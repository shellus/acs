# 架构、应用行为与验证范围

## 模块

| 模块 | 职责 |
| --- | --- |
| `crates/core`（`acs-core`） | 配置、凭据、Codex PKCE OAuth、快照、代理、目标文件保存、历史修复、启动计划 |
| `crates/cli`（`acs`） | clap 参数、inquire 菜单与字段提示、对齐状态输出和退出码；调用核心 |
| `src-tauri`（`acs-gui`） | IPC、窗口生命周期、后台修复和进度事件 |
| `ui/src` | React + Radix Themes 表单、内存草稿、显示与确认 |

核心不依赖 Tauri。默认 workspace 只构建核心和 CLI，Linux 无 GUI 依赖。SQLite 编译进二进制，不调用 Windows 专用 DLL。平台差异集中在 `launch.rs`。

## 目录

| 目录 | 决定方式 | 内容 |
| --- | --- | --- |
| `acs` | `ACS_HOME`，默认 `~/.acs` | 配置库、窗口状态、日志、修复备份、操作锁 |
| `codex` | `CODEX_HOME`，默认 `~/.codex` | `config.toml`、`auth.json`、`.env` 托管区块 |
| `claude` | `CLAUDE_CONFIG_DIR`，默认 `~/.claude` | `settings.json`、`.credentials.json` |
| `claude_json` | 随 `CLAUDE_CONFIG_DIR`，默认 `~/.claude.json` | 只管理 `oauthAccount` 字段 |
| `claude_desktop` | `ACS_CLAUDE_DESKTOP_DIR`，默认桌面版 `Claude-3p` 数据目录 | `claude_desktop_config.json` 的 `deploymentMode`、`configLibrary/_meta.json` 与 acs 条目 |

`Roots::under(dir)` 是测试与 GUI 自动化的隔离入口，禁止落到真实用户目录。

## 操作契约

| 操作 | 行为 |
| --- | --- |
| 打开 | 只读取到内存，不写配置，不启动；按记住的窗口几何还原 |
| 编辑 / 客户端 / Tab / 列表 | 只修改草稿；恢复原值不再视为修改 |
| 保存 | 校验目标文件基线、结构、认证和路由；成功留在窗口 |
| 启动 | 阻止草稿和外部变化；只读取活动认证；请求成功才退出 |
| 关闭 / 标题栏 / Esc | 共用确认，默认继续编辑，不撤销已保存配置 |
| 修复 | 明确确认后备份和修复；不保存表单，不启动 |

不使用自动启动、倒计时、全局空格或 Enter 默认启动，不停止既有客户端，也不添加启动器单实例锁。`operation.lock` 只串行化本工具写入，不阻止多个窗口打开。

窗口几何在移动和缩放时记入内存，退出时写 `acs/window.json`；最大化与最小化期间只更新最大化标记，保留手动调整过的还原尺寸。该文件不在目标文件基线内，不参与保存与启动校验，写失败只记日志。还原前核对可见显示器，窗口中心不落在任何显示器上时放弃记住的位置。

## 配置和认证

Codex 与 Claude 各自独立生效，不是二选一。GUI 草稿同时持有两份 `AgentDraft`，切换客户端只切显示，两个客户端的编辑一起参与“未保存修改”判断，一起提交。

GUI 保存请求显式携带当前客户端；校验和应用当前客户端，以及另一个客户端中实际修改过的草稿。未改动的另一客户端不重写、不因其地址或认证校验失败阻塞当前保存。Claude 表单只写入 `draft.claude`。

快照包含目标文件原始字节和缺失状态，revision 使用长度分隔的 SHA-256。保存前整体核对，并在逐文件替换前再次核对。同目录临时文件 flush/sync 后原子替换，失败逆序恢复。外部再次改变已写文件时不覆盖，报告回滚错误。

`~/.claude.json` 由 Claude Code 持续写入，因此对它只把 `oauthAccount` 一个顶层字段纳入基线并局部改写，其余字段原样保留。

`toml_edit` 只替换 `model_providers.custom`，两种模式下都保留目标配置中其他 provider 及其注释、字段和嵌套表。API 条目的 `model_providers_toml` 只保存 custom 完整片段，保留其未知字段和嵌套表；其他 provider 属于用户，不进入快照。官方模式将活动 custom 精简为名称、协议与官方认证标记，重复保存不以精简定义覆盖快照；自定义模式恢复 custom 快照并更新表单字段。显示名称不是 ID；名称精确为 `OpenAI` 可能触发上游特殊行为，本工具不将它解释为模式选择。

Codex 认证由 `auth.json` 决定。不写 forced_login_method；显式限制、非 file 存储、选中的 profile、冲突路由或 custom 认证覆盖阻止应用。官方启动不携带自定义 Key、地址或域名阻断；自定义 API 直连时桌面启动阻断 OpenAI 云端域名，配置了代理时改用 `--proxy-server` 与代理环境变量，不再阻断；选中的官方账号没有已保存凭据时删除 `auth.json`，由客户端显示登录页（仅含 `auth_mode` 的占位文件会被新版客户端当作令牌缺失，显示“无法加载你的账户”）；部分损坏令牌阻止操作。JWT 只用于身份提示。

Claude 认证由 `settings.json` 的 `ANTHROPIC_*` 与 `.credentials.json` 共同决定。API 模式写入 `ANTHROPIC_AUTH_TOKEN` 与 `ANTHROPIC_BASE_URL`，以及该配置的代理变量；切回官方账号时移除这两个键，`.credentials.json` 原样保留，因此退出登录后不会恢复旧令牌。空字符串视为未登录。

Claude 桌面版忽略 `settings.json` 的 `ANTHROPIC_BASE_URL`，同一次保存同步它的 3P 配置。acs 条目使用固定 UUID `00000000-0000-4000-8000-000000000ac5`，只管理 `inferenceProvider`、`inferenceGatewayBaseUrl`、`inferenceGatewayApiKey`、`inferenceGatewayAuthScheme`（`bearer`）和 `inferenceModels`，其他键原样保留；内容未变时保留原字节。不写 `disableDeploymentModeChooser`：桌面版把它当作 `disableClaudeAiSignIn`，会强制 3P 并忽略 `deploymentMode`。`claude_desktop_config.json` 由桌面版持续写入，只把 `deploymentMode` 纳入基线。启动校验要求自定义模式下 `deploymentMode` 为 `3p`、`appliedId` 指向 acs 条目且地址和 Key 与 `settings.json` 一致，官方模式下为 `1p` 或桌面版从未进入 3P；不一致时提示重新保存。自定义模式保存的模型名不是 `claude-*` 形式时，桌面启动计划直接报错，避免桌面版改用中转站列表里的其他模型；CLI 启动不做此限制。桌面启动不加 OpenAI 域名阻断，生效代理以 `--proxy-server` 与代理环境变量注入。

配置 ID 是 8 位随机小写十六进制字符串，在全部客户端和模式间全局唯一；读取配置库和保存草稿时由核心统一校验，重复或格式错误时阻止操作，不自动重编号。历史 ID、配置 ID 和显示名称独立，显示名称允许重复。

外部 API 只按地址和 Key 刷新已有条目；无法匹配时不创建配置。官方凭据只刷新已知账号，或绑定已选中的未登录配置，保留既有 ID；未知账号不自动添加。新增配置由 GUI“添加”或 CLI `add` 显式完成。

## CLI 与 OAuth

无参仅在 stdin/stdout 都是终端时进入菜单，其余情况保留列表输出。inquire 使用 crossterm 后端，支持 Windows 和 Linux musl，无全屏 UI。`NO_COLOR` 令提示与状态输出不使用颜色。子命令与菜单共用操作和保存入口；只有字段编辑期间保留局部表单，校验失败不丢失输入，其他动作完成即保存，无跨菜单草稿。业务校验及多文件基线检查仍由核心负责。历史修复先确认，终端 stderr 刷新同一行的真实 `n/total`，重定向时逐行输出。

`oauth::Login` 在开始时读取隔离 Store 和目标 Codex 官方账号，生成随机 state、PKCE verifier 及 S256 challenge。授权使用 Codex 公共 OAuth client、固定 localhost 回调及 OpenAI HTTPS token 端点，不启用监听服务。完成时严格校验 URL、state、唯一 code，再提交带 verifier 的一次性表单请求；禁用 HTTP 重定向，限定网络等待和响应大小，不自动重放 token POST。配置的官方代理优先生效，不修改父进程环境。

成功响应须包含非空 id/access/refresh token 及账号 ID。JWT 仅提取账号摘要，可信边界是 TLS token 端点；凭据写成 Codex 的 `auth_mode=chatgpt`、`tokens`、`last_refresh` 格式。更新目标账号快照、选中项和活动认证仍使用 Session 原基线及 Store 提交/回滚；浏览器流程期间的外部变化也会阻止覆盖。失败响应正文、回调 URL、code 和 token 不进入错误或常规日志，核心对象不实现 Debug/Serialize。端点替换仅为核心私有函数的单元测试调用，正式 CLI 无端点覆盖或测试入口。OAuth 后续刷新由官方客户端负责，GUI 可复用协议逻辑但目前仍由客户端登录。

## 数据迁移

配置库只读取 `~/.acs/profiles.json`，缺失时从空库开始，不扫描旧配置库或自动导入账号。既有数据的导入、ID 重编号在升级时独立执行一次：备份原文件，同步修改配置 ID 和选中引用，保留其他字段；不在程序中保留兼容分支或升级测试。

旧的 provider 快照在升级时独立清理一次：备份 `profiles.json`，将 Codex API 条目的 `model_providers_toml` 缩减为 custom 表及其嵌套内容，其他字段和目标 `config.toml` 不变。旧快照中的其他 provider 不自动恢复到目标配置，由用户自行决定保留内容。旧 `.env` 的 `# BEGIN CHATGPT API ONLY PROXY` 区块仍在保存时按标记清理，避免残留代理生效。

## 历史修复

逐行扫描 rollout，待修改内容暂存临时文件，保留其他行、换行及修改时间。备份先于任何修改。SQLite 在线备份包含已提交 WAL 内容，不直接复制运行中的 WAL/SHM。

只纳入含目标历史表及 provider 字段的数据库，无关数据库不进入写事务或备份。目标数据库作为 ATTACH 库加入同一事务，先执行全部 SQL，再替换 rollout，最后提交。SQL、文件冲突或替换失败时回滚数据库，并恢复已改写 rollout；恢复前核对内容以免覆盖外部写入。最多支持 10 个目标数据库，超限阻止。

## 验证范围

- 核心：Claude 桌面版 3P/1P 同步、用户条目保留、幂等与外部切换阻止启动、8 位 ID 与跨客户端/模式唯一性、保存/启动分离、目标文件外部变化、写入失败逐点回滚、快照往返、已有账号登录捕获与退出、未知账号不自动创建、部分凭据阻止、外部 API 归属、代理校验/幂等/父环境隔离、自定义 API 按配置代理与域名阻断取舍、路由冲突、备份/WAL、幂等、多数据库与文件失败回滚。
- CLI：实际二进制两个客户端的增改删、ID 前缀定位、歧义与未知目标报错、代理读写隔离、修复确认。
- OAuth：随机 PKCE/授权链接、回调地址和重复参数、state、授权拒绝、本地假 token 服务的表单交换、凭据格式与选中项、失败不落盘和授权期间外部变化；不请求真实 token 端点。提示层无需端到端测试。
- Web 界面：两份草稿独立、失败与重试、三个关闭入口、修复确认、浅深色、默认与最小窗口和大文本。
- Windows 正式 GUI：fixture、实际 IPC、保存与外部冲突、取消关闭和 Esc、退出记住窗口几何与再次打开按记住尺寸还原；脚本不进入正式二进制。

CI 在三个系统运行核心/CLI 回归，构建 Windows/macOS GUI。构建和 fixture 测试不等同于真实账号联网或真实客户端启动验证。

### Windows 商店包启动

优先直接启动已登记的 Codex EXE。仅当该 EXE 返回拒绝访问时，通过 Windows PowerShell 的 `Invoke-CommandInDesktopPackage` 在对应商店包上下文中创建进程；GUI 与 CLI `run --desktop` 共用实现。普通 CLI 进程不走此回退。

包激活不会继承调用者环境，因此辅助进程从临时请求文件恢复完整子进程环境，应用启动计划中的变量覆盖与删除，并原样传递参数和工作目录。只有目标进程创建成功并返回 PID 后才视为启动请求成功；失败或等待确认超时保留配置窗口。临时文件在请求结束后清理，不修改系统权限、用户环境变量或商店包文件，不停止已有客户端，也不使用 Explorer 激活回退。

Windows 常规测试验证参数转义、环境传递、父环境不变和目标进程创建失败。有 Codex 商店包的机器可运行 `cargo test -p acs-core --locked installed_package_preserves_launch_environment -- --ignored`，通过隔离探针验证真实包上下文中的两种模式与失败回传；探针不读取真实用户配置。该检查验证启动链路，不代表真实账号联网成功。

## 仍需关注的边界

- `run` / `proxy` 未给 `--agent` 时仍默认 Codex；后续评估是否要求显式客户端，本次保持脚本兼容。

- Claude 桌面版的 `deploymentMode` 与 `configLibrary` 格式未公开，依据桌面版 3.6556 的实际读取逻辑；升级后需复核。桌面版在 3P 登录页切回 Anthropic 登录会改写 `deploymentMode`，此后启动校验要求重新保存。
- Claude 桌面版的官方账号登录保存在桌面版自身，acs 的多官方账号切换只作用于 Claude Code。
- 外部 Codex 不参与操作锁；基线核对与替换间仍有竞争窗口，多文件保存也不是掉电原子事务。
- SQLite ATTACH 在 WAL、崩溃或掉电条件下不保证跨库提交原子性；备份提供恢复依据。修复应停止客户端写入后执行。
- rollout 扫描流式处理，但替换/冲突核对阶段仍读取完整文件；极大文件峰值内存待评估。
- 窗口打开后的 OAuth 刷新会触发外部变更阻止，需重新读取。
- Windows 商店包回退依赖系统 Windows PowerShell 与 Appx 模块；系统策略禁止辅助进程或包激活时明确报错。
- macOS 真实 Codex 启动、联网、菜单和多显示器仍需对应机器实测；无发布凭据时不声称完成 Apple 签名/公证。
