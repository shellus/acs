# AGENTS.md

## 项目边界

- 项目采用 Rust 共享核心（`crates/core`）、独立 CLI（`crates/cli`）和 Tauri GUI（`src-tauri`、`ui`）。业务校验只在核心实现，不得在前端复制。
- Windows/macOS GUI 使用系统 WebView；Windows 便携 EXE 依赖 WebView2，安装包可处理缺失运行时。Linux CLI 不得依赖 GUI、Node.js 或外部 SQLite DLL。
- 不添加启动器级单实例锁。重复启动行为应继续交给官方桌面应用处理。
- 打开程序始终显示配置窗口，只有“启动”按钮触发客户端启动；不添加自动启动、启动倒计时、全局空格入口或 Enter 默认启动按钮。
- 保存按钮放在各自 Tab 内，保存成功后留在设置页；底部“启动”只读取已保存配置，有未保存修改时提示并阻止启动。启动器不得因打开设置或保存配置而停止已运行的 ChatGPT，仅在启动请求成功后退出启动器，失败留在设置页。
- “关闭”、标题栏关闭和 Esc 共用未保存修改检查；关闭不撤销已保存配置。保存与启动前校验目标文件读取基线，冲突时保留草稿并阻止覆盖；启动不得使用配置库 Key 补足活动认证。
- Electron 外壳的 OpenAI 云端域名阻断不能影响内置 Codex app-server 对自定义 API 的访问；阻断只用于直连的自定义 API，配置了代理时以代理代替阻断。

## 配置与 Provider

- `model_provider` 是历史对话使用的 provider ID；自定义 API 模式固定为 `custom`，官方账号模式使用内置 `openai`。
- 表单中的“提供者名称”只对应 `[model_providers.custom].name`，不得作为 provider ID 写入历史数据。
- 模式切换更新 `config.toml`、`auth.json` 与本机 `~/.acs/profiles.json`；切换 Tab 不落盘，保存或切换模式不得隐式修复历史对话。
- Codex 官方模式支持 acs 核心的 PKCE 授权链接与粘贴完整回调 URL 登录，也可由官方客户端登录；后续 OAuth 刷新仍由官方客户端负责。Claude 登录与刷新由官方客户端负责。回调须严格校验地址及 state，换 token 与保存失败不得留下半写凭据，不记录回调 code 或 token。官方启动不得携带域名阻断或自定义 API 环境变量；仅凭本地凭证存在不得声称账号有效。
- 官方代理按客户端保存在 `official_proxy_url`，自定义 API 代理按配置保存在 `custom_proxy_url`；当前模式生效的代理同步到 Codex `.env` 的 `ACS PROXY` 标记区块和 Claude `settings.json` 的 env，并注入桌面启动参数和子进程环境。不得修改 Windows 系统代理或用户环境变量；生效代理为空时只移除管理区块或键，保留原有内容。回归覆盖地址校验、父进程环境不变、幂等、禁用、模式保留、异常标记预检及包含 `.env` 的目标文件保存失败回滚。
- 切换前保留当前凭证及各模式模型设置，官方退出登录后不得恢复旧令牌。多文件保存失败必须尝试恢复原状态并阻止启动。
- 认证类型由 `auth.json` 决定，不写入 `forced_login_method`；存在显式限制时提示冲突。`cli_auth_credentials_store` 默认不写，已有 `file` 保留，其他显式存储方式提示冲突。
- acs 只管理 `model_providers.custom`；两种模式下均原样保留目标配置中的其他 provider，不将其写入配置库。`~/.acs/profiles.json` 的 `model_providers_toml` 只保存 custom 的完整 TOML 片段，保留其未知字段和嵌套表。官方模式将活动 custom 替换为名称、`wire_api = "responses"` 与 `requires_openai_auth = true` 的精简定义，供历史对话解析，不携带自定义地址、Key、认证覆盖或请求头；自定义模式恢复 custom 快照后更新表单字段。重复保存不得用官方模式的精简定义覆盖快照。
- 数据格式变更只做一次性迁移，不在程序中保留旧版本兼容分支或升级测试。
- Claude 桌面版的自定义 API 只通过 `Claude-3p` 的 `configLibrary` acs 条目与 `deploymentMode` 切换，不写 `disableDeploymentModeChooser` 或 `HKCU\SOFTWARE\Policies\Claude`；切回官方只移除 acs 条目及其连接字段，保留用户条目。
- 桌面窗口几何写入 `~/.acs/window.json`，不进入目标文件基线，也不因记住尺寸触发外部变化校验；只在退出时落盘，失败仅记日志，不阻断退出或启动。记住的位置不在任何显示器上时放弃该位置。
- 不静默删除 profile 或路由覆盖配置；发现与目标模式冲突时，在落盘前提示并阻止切换。
- 历史修复只能由 GUI“修复对话”按钮或 CLI `repair` 命令显式触发。进度必须来自真实工作量，GUI 以临时 `n/total` 进度区展示。
- 修改 rollout 或 SQLite 前必须创建备份；SQLite 更新使用事务，失败时恢复已改写的 rollout 并阻止误报成功。

## 安全与测试

- 不得把真实 API Key、API 地址、账号、本机路径或 Codex 配置提交到仓库。
- README 截图只能捕获应用窗口，并使用隔离 fixture 与明显的 `example` 占位值；不得包含桌面背景、终端、用户名或真实配置。`docs/images/*.png` 取自 `npm test` 输出的 `.impeccable/review/<默认窗口宽>-<模式>.png`，界面布局变更后必须同步替换，不保留与当前界面不符的图片。
- Rust 测试显式传入临时 Store；正式 GUI 自动化必须设置 `ACS_HOME`、`CODEX_HOME`、`CLAUDE_CONFIG_DIR`、`ACS_CLAUDE_DESKTOP_DIR` 并指向隔离 fixture，禁止测试读取真实用户目录。
- 测试入口放在 Rust tests 或 `#[cfg(test)]` 中，不得进入正式 EXE，不得添加运行时测试后门。
- 修改同步逻辑时至少验证：成功更新、无变更幂等、数据库失败回滚、备份存在，以及保存配置不会触发同步。

## 构建与发布产物

- 构建命令以 `README.md` 为准，Windows GUI 产物为 `acs-gui.exe`，CLI 为 `acs`（Windows 带 `.exe`），macOS GUI 为 `.app`。
- 源码行为改变后必须重新构建正式 EXE，并确认测试入口未进入正式二进制。
- 正式 EXE 与发布 ZIP 不进入 Git 跟踪，只能作为 GitHub Release 资产分发。
- Windows 便携 ZIP 只包含正式 `acs-gui.exe`；发布前确认 ZIP 内 EXE 与对应正式 EXE 的 SHA-256 一致。CLI 与其他平台产物独立打包。
- Release tag 使用语义化版本；同一版本的 tag、标题、ZIP 文件名与发布说明必须一致。
- 提交前检查 `git status`、staged diff、敏感信息扫描结果和构建验证结果。
