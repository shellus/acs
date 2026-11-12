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

修改命令会保存，但不自动启动；只有显式菜单启动操作或 `run` 会启动客户端。`--agent` 是全局参数，`ls` 和配置选择器用它过滤，`run` 和 `proxy` 用它选客户端（默认 Codex）。

### 交互菜单

stdin 和 stdout 都是终端时，直接运行 `acs` 进入提示式菜单；任一被管道或重定向时，无参仍输出 `ls`。列表按终端字符宽度对齐，每行显示客户端、当前标记、完整 ID、名称、类型、地址或账号摘要、模型和代理。终端 `ls` 将当前项着色，设置 `NO_COLOR` 禁用颜色。

主菜单的配置列表兼作状态视图，下面提供新增、启动 Codex、启动 Claude、官方代理、修复历史和退出。选择配置后可设为当前、设为当前并启动、编辑、改名、复制、删除；Codex 官方账号另有登录。官方账号复制只新建未登录条目，不复制令牌。Esc 返回上一级，主菜单 Esc 退出。输入名称或摘要可筛选列表。

新增向导依次选择客户端、类型、名称，自定义 API 进入字段菜单；Codex 官方账号保存后可选择立即登录。自定义 API 逐项修改后显式保存或放弃；保存失败展示核心错误并保留已填内容。API Key 的表单摘要与 GUI 一样显示原值，重新输入使用隐藏提示，Esc 保留原值。菜单操作与子命令共用保存操作，菜单间不保留草稿。

`use`、`edit`、`rm` 缺少目标及 `add` 缺少参数时，终端下进入选择器或向导；非终端下报用法错误。终端中 `edit <配置>` 不带字段进入编辑表单。启动菜单选择 CLI 或桌面版；Linux 只显示 CLI。启动 CLI 后 acs 使用客户端的退出码退出。

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

`--key` 会把 Key 写进 shell 历史与进程列表；日常可在交互菜单中隐藏输入，或在 GUI 里填写。

```sh
acs add codex official "example account"
acs login "example account"
```

`login` 只支持 Codex 官方账号。它打印本次 PKCE 授权链接；在任意能访问 OpenAI 的浏览器打开并完成登录，跳转到 `http://localhost:1455/auth/callback` 后，即使页面打不开，也可复制地址栏的完整 URL 回终端。acs 不监听本地端口、不需要 SSH 转发。终端粘贴隐藏输入；非交互调用从 stdin 读取一行，便于外部程序通过管道传入。不要把实际回调 URL 放入命令参数、shell 历史或日志。

每次登录的 state 和 PKCE verifier 独立生成，只驻留内存。acs 校验回调地址、唯一参数及 state 后换取 token，使用该客户端保存的官方 HTTP 代理（未设置则遵循进程网络代理）。成功后保存 `auth.json` 和配置库，并将该配置设为 Codex 当前官方账号；不启动客户端。地址错误、state 不匹配、授权拒绝、网络失败、无效 token 或配置外部变化均报错，失败不保存半份凭据。回调 code 和 token 不写常规日志。后续 token 刷新仍由 Codex 负责。

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

## 从 2.x 升级

首次以 `~/.acs` 打开时，如果旧的 `~/.codex/launcher-profiles/modes.json` 存在且 `profiles.json` 尚未创建，会把它作为 Codex 配置库初值读取一次，保存后落到 `~/.acs/profiles.json`。旧文件不改写、不删除，Claude 侧从空配置开始。旧的 `# BEGIN CHATGPT API ONLY PROXY` 标记区块在下一次保存时被清理。

## 分发与平台启动

Windows 便携 ZIP 只包含 `acs-gui.exe`，需要 WebView2；NSIS 安装包可处理缺失运行时。CLI 为独立的 `acs.exe`。macOS 使用 `.app` 压缩包，未配置 Apple 发布证书时未经 Developer ID 签名和公证，构建成功不代表无提示分发。

Windows 自动发现当前用户最高版本 `OpenAI.Codex` 包；Claude 查找 `Claude` MSIX 包，其次是旧安装器的 `%LOCALAPPDATA%\AnthropicClaude\claude.exe`。macOS 在 `/Applications` 和用户 Applications 目录查找 `Codex.app` 或 `Claude.app`。客户端缺失或启动失败时留在设置页；Codex 的安装入口在 Windows 打开 Microsoft Store，Claude 打开对应平台的官方安装包下载地址，“检查更新”打开 claude.com/download。macOS“检查更新”是下载入口，不声称执行了更新检查。
