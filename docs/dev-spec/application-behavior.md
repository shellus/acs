# 架构、应用行为与验证范围

## 模块

| 模块 | 职责 |
| --- | --- |
| `crates/core`（`acs-core`） | 配置、凭据、Codex PKCE OAuth、快照、代理、目标文件保存、历史修复、启动计划 |
| `crates/cli`（`acs`） | clap 参数、ratatui/crossterm 全屏状态与就地表单、对齐列表输出和退出码；调用核心 |
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

无参仅在 stdin/stdout 都是终端时进入全屏 TUI，其余情况保留列表输出。ratatui 使用 crossterm 后端和备用屏幕，RAII guard 在正常、错误与退出路径恢复 raw mode、主屏幕和光标，panic hook 在输出错误前恢复；Ctrl+C 作为按键事件处理。OAuth 与客户端启动前先恢复主屏幕，OAuth 以无边框独占行输出完整链接，读回调后重新进入全屏。只保留一套 TUI，移除 inquire。Windows 和 Linux musl 均无需 GUI 或 Node。

顶栏保留两个客户端的当前模式 / 配置摘要；左侧分组列表，右侧跟随高亮的详情及核心 `profile_hints` 校验提示。80 列起左右分栏，更窄时上下单栏，表单使用完整正文。列表和状态行按终端显示宽度截断并加省略号，详情长值折行并可滚动，字段输入按 UTF-8 字符移动光标并水平滚动。NO_COLOR 保留标记与反显。编辑期间只有动作内表单，F2 保存、Esc 取消，未修改不确认；校验失败保留表单，其他动作成功即保存，无跨动作草稿。启动始终显式；危险动作需 y 确认。历史修复界面直接使用核心的真实进度，不改成估算进度。

新增类型使用独立二选一状态，只存 Mode，不伪装成 Input；Enter 后进入名称 / API 表单，官方账号只渲染名称。输入文本替换重建 Input，所有文本按键先修复过期光标到合法 UTF-8 边界，渲染也独立使用安全位置，防止陈旧长度或非字符边界导致切片 panic。Codex 新官方账号保存后创建登录确认状态，保存成功即清空过滤并聚焦新账号；p 编辑该客户端的官方代理，F2 仅保存、Enter 保存后开始登录。取消登录保留已保存配置。Claude 不创建 OAuth 状态，仅说明客户端登录。左栏按剩余显示宽度添加主机名、邮箱或未登录摘要，不挤占长名称造成折行。

`transfer` 核心负责版本化 custom-only 配置包。导出只读取本机库；导入预检类型、字段与代理、排除官方凭据，合并并处理全局 ID 冲突。活动配置由各客户端同一套 capture 代码提取，完整字段及 custom TOML 语义一致时复用迁入项，否则显式保存本机现场为单独当前配置；capture 优先匹配选中项，避免相同地址 / Key 但不同模型的条目被错误选中。提交的 Snapshot 只有 LIBRARY 改变，仍经过全量基线校验与原子写入，其他目标文件原字节保留。导入不触发历史同步，也不触发桌面配置同步。

`oauth::Login` 在开始时读取隔离 Store 和目标 Codex 官方账号，生成随机 state、PKCE verifier 及 S256 challenge。授权使用 Codex 公共 OAuth client、固定 localhost 回调及 OpenAI HTTPS token 端点，不启用监听服务。完成时严格校验 URL、state、唯一 code，再提交带 verifier 的一次性表单请求；禁用 HTTP 重定向，限定网络等待和响应大小，不自动重放 token POST。配置的官方代理优先生效，不修改父进程环境。

LoginRoute 使用 reqwest 同一 hyper-util Matcher 解析并冻结默认传输路线：显式官方代理优先，否则按环境 / 平台静态系统代理发现，并考虑 NO_PROXY；传输构造先关闭重复自动发现，再应用已冻结的 Proxy，保证确认、CLI 输出和真正 token 请求一致。无官方代理保留继承能力并提示地区限制，不假称总是直连。该选择依据 [Codex 普通传输与系统代理策略](https://github.com/openai/codex/blob/main/codex-rs/http-client/src/outbound_proxy.rs) 的默认 HTTP 行为；不复制其可选 PAC/WPAD 或受管理网络策略。地区错误只识别已知地区不支持标记并返回固定操作指引，不输出响应正文；其他 HTTP 错误说明保持原样。本地假代理断言请求的绝对目标 URI 和 PKCE 表单，测试不访问真实 token 端点。

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
- OAuth：随机 PKCE/授权链接、回调地址和重复参数、state、授权拒绝、本地假 token 服务的表单交换、凭据格式与选中项、失败不落盘和授权期间外部变化；不请求真实 token 端点。
- TUI：无真实终端的按键 / 状态与 TestBackend 字符缓冲区断言，覆盖导航不写入、未改取消、脏表单确认、失败保留字段、Unicode 光标和宽 / 80 列 / 窄屏渲染；另在隔离伪终端验收实际字符网格、终端退出恢复与子进程退出码。
- TUI 覆盖性回归：两个客户端、15 种状态、120 种支持按键逐一执行并渲染，另对每种状态执行 4 组固定种子的 96 步序列；每步检查所有 Input 的字符边界，并覆盖零尺寸及极小窗口。Unix 测试用独立测试二进制在真实 PTY 中 panic，断言终端属性恢复且离开备用屏幕 / 显示光标先于 panic 输出；探针仅存在于 cfg(test)。
- 自定义迁移：官方令牌不导出、未知字段保留、活动目标原字节不变、相同配置选中、不同配置独立保留、重复导入幂等、错误输入不落盘。
- Web 界面：两份草稿独立、失败与重试、三个关闭入口、修复确认、浅深色、默认与最小窗口和大文本。
- Windows 正式 GUI：fixture、实际 IPC、保存与外部冲突、取消关闭和 Esc、退出记住窗口几何与再次打开按记住尺寸还原；脚本不进入正式二进制。

CI 在三个系统运行核心/CLI 回归，构建 Windows/macOS GUI。构建和 fixture 测试不等同于真实账号联网或真实客户端启动验证。

### Windows 商店包启动

优先直接启动已登记的 Codex EXE。仅当该 EXE 返回拒绝访问时，通过 Windows PowerShell 的 `Invoke-CommandInDesktopPackage` 在对应商店包上下文中创建进程；GUI 与 CLI `run --desktop` 共用实现。普通 CLI 进程不走此回退。

包激活不会继承调用者环境，因此辅助进程从临时请求文件恢复完整子进程环境，应用启动计划中的变量覆盖与删除，并原样传递参数和工作目录。只有目标进程创建成功并返回 PID 后才视为启动请求成功；失败或等待确认超时保留配置窗口。临时文件在请求结束后清理，不修改系统权限、用户环境变量或商店包文件，不停止已有客户端，也不使用 Explorer 激活回退。

Windows 常规测试验证参数转义、环境传递、父环境不变和目标进程创建失败。有 Codex 商店包的机器可运行 `cargo test -p acs-core --locked installed_package_preserves_launch_environment -- --ignored`，通过隔离探针验证真实包上下文中的两种模式与失败回传；探针不读取真实用户配置。该检查验证启动链路，不代表真实账号联网成功。

## 仍需关注的边界

- `run` / `proxy` 未给 `--agent` 时仍默认 Codex；后续评估是否要求显式客户端，本次保持脚本兼容。
- Claude CLI 的启动计划尚未显式注入 `CLAUDE_CONFIG_DIR`；`--root` 隔离配置读写时，实际启动 Claude 仍需调用方设置该变量。后续补齐启动环境及回归验证。
- 使用文档“从 2.x 升级”仍描述自动读取旧配置库，与当前一次性迁移约束及核心实现不一致，待统一升级指引。

- Claude 桌面版的 `deploymentMode` 与 `configLibrary` 格式未公开，依据桌面版 3.6556 的实际读取逻辑；升级后需复核。桌面版在 3P 登录页切回 Anthropic 登录会改写 `deploymentMode`，此后启动校验要求重新保存。
- Claude 桌面版的官方账号登录保存在桌面版自身，acs 的多官方账号切换只作用于 Claude Code。
- 外部 Codex 不参与操作锁；基线核对与替换间仍有竞争窗口，多文件保存也不是掉电原子事务。
- SQLite ATTACH 在 WAL、崩溃或掉电条件下不保证跨库提交原子性；备份提供恢复依据。修复应停止客户端写入后执行。
- rollout 扫描流式处理，但替换/冲突核对阶段仍读取完整文件；极大文件峰值内存待评估。
- 窗口打开后的 OAuth 刷新会触发外部变更阻止，需重新读取。
- Windows 商店包回退依赖系统 Windows PowerShell 与 Appx 模块；系统策略禁止辅助进程或包激活时明确报错。
- macOS 真实 Codex 启动、联网、菜单和多显示器仍需对应机器实测；无发布凭据时不声称完成 Apple 签名/公证。
