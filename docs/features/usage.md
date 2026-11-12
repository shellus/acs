# 使用与 CLI 命令

## 配置目录

本程序自己的目录由 `ACS_HOME` 决定，默认 `~/.acs`；被管理的客户端目录由 `CODEX_HOME`（默认 `~/.codex`）、`CLAUDE_CONFIG_DIR`（默认 `~/.claude` 与 `~/.claude.json`）、`ACS_CLAUDE_DESKTOP_DIR`（默认为 Claude 桌面版的 `Claude-3p` 数据目录：Windows `%LOCALAPPDATA%\Claude-3p`，macOS `~/Library/Application Support/Claude-3p`，Linux `~/.config/Claude-3p`）决定。CLI 的 `--root` 把全部目录一起放到指定目录下，用于隔离测试。

| 文件 | 归属 | 用途 |
| --- | --- | --- |
| `~/.acs/profiles.json` | 本程序 | 两个客户端的账号、API 配置、提供者快照与代理偏好 |
| `~/.acs/window.json` | 本程序 | 桌面窗口的尺寸、位置与最大化状态，仅本机有效 |
| `~/.acs/logs/acs.log` | 本程序 | 操作失败诊断 |
| `~/.acs/backups_state/provider-sync` | 本程序 | 历史修复前的 rollout 与 SQLite 一致性备份 |
| `~/.codex/config.toml` | Codex | 当前 provider、模型、路由与其他设置 |
| `~/.codex/auth.json` | Codex | 当前官方凭据或 API Key |
| `~/.codex/.env` | Codex | 仅管理 `ACS PROXY` 标记区块 |
| `~/.claude/settings.json` | Claude | 自定义 API 的 `ANTHROPIC_*` 环境变量 |
| `~/.claude/.credentials.json` | Claude | 官方登录凭据 |
| `~/.claude.json` 的 `oauthAccount` | Claude | 官方账号身份；其余字段不参与基线 |
| `Claude-3p/claude_desktop_config.json` 的 `deploymentMode` | Claude 桌面版 | `3p` 使用自定义 API，`1p` 使用官方账号；其余字段不参与基线 |
| `Claude-3p/configLibrary/_meta.json` | Claude 桌面版 | 3P 配置条目列表与 `appliedId` |
| `Claude-3p/configLibrary/00000000-0000-4000-8000-000000000ac5.json` | Claude 桌面版 | acs 管理的网关地址、Key 与模型 |

配置和备份包含私人凭据，不进入源码仓库。

## 桌面操作

添加、复制 API、删除、改名、切换客户端或 Tab、选择配置只改变草稿。修改“配置名称”即可改名。“保存配置”保存当前客户端和整个配置库草稿，成功后留在设置页；关闭不撤销已保存配置。GUI 不删除当前模式最后一项配置，需先添加其他配置。

API 地址使用 `https://主机/路径/v1`（Claude 填 `https://主机` 即可），提供者显示名称与历史 provider ID 分离。API 模式固定使用 `custom` ID；官方模式使用内置 `openai`。官方模式保留不带自定义地址、Key、请求头的 `custom` 定义，使旧对话仍能解析。

![官方账号设置](../images/official-mode.png)

Codex 可通过官方客户端或 acs CLI 的回调 URL 流程登录，刷新和联网验证仍由 Codex 完成，Claude 由 Claude Code 完成。新增账号不携带其他账号旧令牌；客户端明确退出登录后，不再恢复该账号旧凭据。部分损坏的令牌阻止读取，避免误判为退出登录。

官方账号的 HTTP 代理按客户端设置，同一客户端的官方账号共用；自定义 API 的代理按配置单独设置，部署在海外的中转站等需要代理时填写，留空直连。生效的代理只影响新启动客户端、子进程和读取同一配置的 CLI，不修改系统代理或父终端环境。Codex 的代理写在 `.env` 的托管区块，Claude 的代理写在 `settings.json` 的 `ANTHROPIC_*` 同级环境变量。当前模式的代理为空时，仅移除管理的键，原有内容保留；原有代理变量仍可能生效。

Claude 桌面版不读取 `settings.json` 的 `ANTHROPIC_*`，自定义 API 通过它的第三方推理（3P）配置生效：保存时写入 acs 自己的 `configLibrary` 条目，设为 `appliedId`，并把 `deploymentMode` 设为 `3p`。切回官方账号时 `deploymentMode` 改为 `1p`，移除 acs 条目及其连接字段，保留用户在桌面版里添加的条目和其他键；从未使用过 3P 时不创建桌面版文件。桌面版只接受 `claude-*` 形式的模型名：配置了这类模型名时只向桌面版提供该模型；留空时由中转站的 `/v1/models` 提供列表（桌面版会滤掉非 Claude 名称）；填了其他模型名（如 gemini）时启动桌面版报错并阻止启动，Claude Code CLI 不受影响。官方模式由桌面版自己登录 claude.ai，与 `.credentials.json` 无关，所在地区受限时需要设置官方代理。桌面版只在启动时读取配置，已在运行时需完全退出后再点“启动”。

Codex 桌面版在自定义 API 模式直连时阻断 Electron 外壳的 OpenAI 云端域名，避免请求长时间卡住；该配置设置了代理时改为通过代理访问，不再阻断。

窗口尺寸、位置和最大化状态在退出时记住，下次打开按记住的几何还原；记住的位置不在任何显示器上时改由系统摆放。最大化或最小化期间的尺寸不覆盖手动调整过的还原尺寸。

保存或启动遇到外部变化时保留草稿。“重新读取”可读取最新配置，存在草稿时先确认放弃。修复历史期间应关闭正在写入历史的 Codex 客户端。

## CLI

修改命令会保存，但不自动启动；只有全屏界面的 `s` 或 `run` 会启动客户端。`--agent` 是全局参数，`ls` 用它过滤，TUI 用它选择初始客户端，`run` 和 `proxy` 用它选客户端（默认 Codex）。

### 全屏 TUI

stdin 和 stdout 都是终端时，直接运行 `acs` 进入备用屏幕的全屏 TUI；任一被管道或重定向时，无参仍输出 `ls`。顶栏显示版本及 Codex、Claude 各自当前模式与配置名；左栏按官方账号 / 自定义 API 分组，`*` 标记活动配置，`>` 和高亮标记仅供查看的选择。右栏即时显示全部字段、登录状态、代理与核心校验提示；`PgUp/PgDn` 可查看完整长值与 custom 快照。低于 80 列时列表与详情上下排列，表单占满正文，不横向折行列表。窗口尺寸变化即时重排。`NO_COLOR` 禁用颜色，文本标记与反显仍能区分状态。

| 按键 | 操作 |
| --- | --- |
| ↑↓ / j k | 移动高亮配置，不落盘 |
| ←→ / Tab | 切换客户端 |
| Enter | 设为当前并立即保存，不启动 |
| e / n / c / r | 编辑 / 新增 / 复制 / 改名 |
| d / l / s / p | 删除 / Codex 官方登录 / 显式启动 / 官方代理 |
| / / ? / R | 过滤 / 帮助 / 重新读取 |
| h | 确认后备份并修复 Codex 历史，显示真实进度 |
| q / Esc / Ctrl+C | 退出并恢复终端；表单中 Esc 取消编辑 |

按 `n` 首先显示“官方账号 / 自定义 API”二选一控件，←→ / ↑↓ / Tab 或 1 / 2 选择，Enter 继续，Esc 取消。随后在详情区就地填写表单；官方账号只需名称，自定义 API 填写地址、Key 等字段。↑↓ / Tab 移动字段，直接输入，←→ 移动光标，Home/End 到首尾，Ctrl+U 清空字段。F2 或 Ctrl+S 保存，Esc 取消。仅确实改过的表单取消时询问放弃，确认框 Esc 返回继续编辑，再按 Esc、y 可放弃；未修改则直接关闭表单。校验失败显示核心错误并保留内容。API Key 与 GUI 一样明文显示。动作成功即保存，无跨动作草稿；官方账号复制只创建未登录条目，不复制令牌。删除与历史修复均需在界面内按 y 明确确认。

左栏宽度允许时，名称后补充地址主机名，官方账号显示邮箱或“未登录”；空间不足就省略或截断次要信息，不折行。高亮未登录的 Codex 官方账号时，右栏和底栏明确提示 `l` 登录；Claude 官方账号提示由 Claude 客户端登录，不显示 `l` 的登录入口。

新增 Codex 官方账号保存后直接询问是否立即登录。确认框显示换令牌使用的实际代理，以及空官方代理的地区限制提示；按 `p` 就地编辑官方代理，F2 保存并留在确认框，Enter 保存代理并继续登录，编辑时 Esc 放弃该次代理修改。确认框中 y / Enter 登录，n / Esc 稍后。取消登录不撤销已保存的账号或代理，稍后可高亮账号按 `l` 继续。Claude 官方账号保存后只提示由 Claude 客户端完成登录。

`use`、`edit`、`rm` 缺少目标及 `add` 缺少参数时，终端下统一打开全屏界面并提示相应按键；`add` 未指定类型时先显示类型选择，已指定类型则直接打开对应表单，`edit <配置>` 不带字段时定位该配置并打开表单。非终端下缺参数仍报用法错误。启动按 1 / Enter 选择 CLI，Windows/macOS 可按 2 启动桌面版，Linux 不显示桌面版。客户端启动成功后退出 TUI，CLI 使用子进程退出码；失败回到界面显示错误。

### 子命令与远程登录

```sh
acs --help
acs ls
acs ls --agent claude
```

配置 ID 是实际存储的 8 位随机十六进制字符串，在两个客户端、两种模式间全局唯一。`ls` 显示完整 ID，名称、地址与登录摘要只用于识别；后续命令接受 ID、唯一的 ID 前缀或名称，不必再写 `--kind`。名称可以重复，匹配多项时改用报错中列出的 ID。

```sh
acs add codex official "example account"
acs add codex custom "example API" --provider "example provider" --url https://api.example.com/v1 --key example-key --model example-model
acs add claude custom "example Claude" --url https://claude.example.com --key example-claude-key --model example-claude-model
acs add codex custom "example copy" --copy example-api
```

`--key` 会把 Key 写进 shell 历史与进程列表；日常可在 TUI 表单或 GUI 中填写。

```sh
acs add codex official "example account"
acs login "example account"
```

`login` 只支持 Codex 官方账号。它打印本次 PKCE 授权链接；在任意能访问 OpenAI 的浏览器打开并完成登录，跳转到 `http://localhost:1455/auth/callback` 后，即使页面打不开，也可复制地址栏的完整 URL 回终端。acs 不监听本地端口、不需要 SSH 转发。终端粘贴隐藏输入；非交互调用从 stdin 读取一行，便于外部程序通过管道传入。不要把实际回调 URL 放入命令参数、shell 历史或日志。

TUI 登录时会临时离开全屏，授权链接以独占一行的纯文本输出，完成、失败或空行取消后回到界面。所有入口（含 `acs login`）先显示实际换令牌代理。优先使用已保存的 Codex 官方 HTTP 代理；为空时保留普通 Codex HTTP 传输的环境 / 系统代理发现行为：识别 `HTTPS_PROXY` / `https_proxy`、`ALL_PROXY` / `all_proxy`，遵守 `NO_PROXY` / `no_proxy`；没有环境代理时可采用平台静态系统代理，无可用代理才直连。`HTTP_PROXY` 单独只用于 HTTP 目标，不代替 HTTPS 代理。NO_PROXY 可令环境路线直连，但显式官方代理优先生效。不读取其他模式的托管代理，也不修改父进程环境或系统代理；需要固定出口时应明确保存官方代理。Codex 专有的 PAC/WPAD 或管理员网络策略不由 acs 导入。

每次登录的 state 和 PKCE verifier 独立生成，只驻留内存；代理路线同时冻结，显示与实际请求使用同一条路线，换 token 不自动重放。acs 校验回调地址、唯一参数及 state 后换取 token。成功后保存 `auth.json` 和配置库，并将该配置设为 Codex 当前官方账号；不启动客户端。地址错误、state 不匹配、授权拒绝、网络失败、无效 token 或配置外部变化均报错，失败不保存半份凭据。地区不支持错误会明确提示在登录确认按 `p` 或运行 `acs proxy --agent codex http://主机:端口` 设置代理，再重新登录；其他错误保留原有说明。回调 code、token 和服务端错误正文不写常规日志。后续 token 刷新仍由 Codex 负责。

隔离验证可运行 `acs --root /tmp/example-acs login <配置>`，然后使用 `CODEX_HOME=/tmp/example-acs/codex codex login status` 检查官方客户端是否识别凭据；该命令只检查本地登录状态，不代表已完成联网请求验证。

```sh
acs edit example-api --model example-other-model
acs edit example-api --key example-new-key
acs edit example-api --proxy http://127.0.0.1:7890
acs use example-api
acs mv example-api "example renamed"
acs rm example-api
```

`use` 按配置所属客户端切换该客户端的当前模式与选中项。最后一项活动配置不能被删除后直接应用；应先切换到另一模式的可用配置。删除其他模式的最后一项不影响活动模式。

```sh
acs proxy http://127.0.0.1:7890
acs proxy --agent claude http://127.0.0.1:7890
acs proxy ""
```

`proxy` 只管理官方账号共用的代理，不带地址时显示当前值；自定义 API 的代理用 `add`/`edit` 的 `--proxy` 设置，空字符串表示直连。地址必须是 `http://主机:端口`，不含账号密码、路径或查询参数。

```sh
acs run
acs run --desktop
acs run --agent claude --desktop
acs run --dry-run
acs run --executable /path/to/codex -- --help
```

`run --dry-run` 只输出计划，不启动。Linux CLI 不添加 Electron 域名参数；`run --desktop` 只在 GUI 平台有意义。

完整编辑支持导出和应用：

```sh
acs export > example-draft.json
acs apply example-draft.json
```

导出包含凭据和读取版本。外部修改后旧草稿不能覆盖新配置，需重新导出并核对差异。

```sh
acs repair
acs repair --yes
```

修复仅将 JSONL `session_meta.payload.model_provider`、SQLite `threads.model_provider` 和存在时的 `local_thread_catalog.model_provider` 改为 `custom`，不修改显示名称或云端对话。不带 `--yes` 时默认取消；确认后先备份，进度来自实际数量。

### 跨机器复制自定义配置

```sh
acs export-custom /private/example-custom.json
# 通过受信任文件传输把配置包复制到目标机器
acs import-custom /private/example-custom.json
```

配置包包含两个客户端的自定义 API 完整字段和 custom TOML 未知字段，含 Key，不包含官方账号与令牌。导出拒绝覆盖已有文件。导入只写 acs 配置库，不改动 `config.toml`、`auth.json`、`.env`、Claude 设置或桌面配置；既有模式保持。若当前活动配置与迁入项完整匹配，选中该条；否则另存“原有生效配置”并选中。相同内容重复导入不增加条目，ID 冲突分配新 ID；导入不按名称覆盖条目。代理值保留，是否在目标机器可用需自行核对。实际迁移前备份目标文件，迁移后核对活动文件字节及客户端请求；含 Key 的传输临时文件用完删除。

## 从 2.x 升级

首次以 `~/.acs` 打开时，如果旧的 `~/.codex/launcher-profiles/modes.json` 存在且 `profiles.json` 尚未创建，会把它作为 Codex 配置库初值读取一次，保存后落到 `~/.acs/profiles.json`。旧文件不改写、不删除，Claude 侧从空配置开始。旧的 `# BEGIN CHATGPT API ONLY PROXY` 标记区块在下一次保存时被清理。

## 分发与平台启动

Windows 便携 ZIP 只包含 `acs-gui.exe`，需要 WebView2；NSIS 安装包可处理缺失运行时。CLI 为独立的 `acs.exe`。macOS 使用 `.app` 压缩包，未配置 Apple 发布证书时未经 Developer ID 签名和公证，构建成功不代表无提示分发。

Windows 自动发现当前用户最高版本 `OpenAI.Codex` 包；Claude 查找 `Claude` MSIX 包，其次是旧安装器的 `%LOCALAPPDATA%\AnthropicClaude\claude.exe`。macOS 在 `/Applications` 和用户 Applications 目录查找 `Codex.app` 或 `Claude.app`。客户端缺失或启动失败时留在设置页；Codex 的安装入口在 Windows 打开 Microsoft Store，Claude 打开对应平台的官方安装包下载地址，“检查更新”打开 claude.com/download。macOS“检查更新”是下载入口，不声称执行了更新检查。
