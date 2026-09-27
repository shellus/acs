# 使用与 CLI 命令

## 配置目录

本程序自己的目录由 `ACS_HOME` 决定，默认 `~/.acs`；被管理的客户端目录由 `CODEX_HOME`（默认 `~/.codex`）、`CLAUDE_CONFIG_DIR`（默认 `~/.claude` 与 `~/.claude.json`）决定。CLI 的 `--root` 把三者一起放到指定目录下，用于隔离测试。

| 文件 | 归属 | 用途 |
| --- | --- | --- |
| `~/.acs/profiles.json` | 本程序 | 两个客户端的账号、API 配置、提供者快照与官方代理偏好 |
| `~/.acs/window.json` | 本程序 | 桌面窗口的尺寸、位置与最大化状态，仅本机有效 |
| `~/.acs/logs/acs.log` | 本程序 | 操作失败诊断 |
| `~/.acs/backups_state/provider-sync` | 本程序 | 历史修复前的 rollout 与 SQLite 一致性备份 |
| `~/.codex/config.toml` | Codex | 当前 provider、模型、路由与其他设置 |
| `~/.codex/auth.json` | Codex | 当前官方凭据或 API Key |
| `~/.codex/.env` | Codex | 仅管理 `ACS PROXY` 标记区块 |
| `~/.claude/settings.json` | Claude | 自定义 API 的 `ANTHROPIC_*` 环境变量 |
| `~/.claude/.credentials.json` | Claude | 官方登录凭据 |
| `~/.claude.json` 的 `oauthAccount` | Claude | 官方账号身份；其余字段不参与基线 |

配置和备份包含私人凭据，不进入源码仓库。

## 桌面操作

添加、复制 API、删除、改名、切换客户端或 Tab、选择配置只改变草稿。修改“配置名称”即可改名。“保存配置”保存当前客户端和整个配置库草稿，成功后留在设置页；关闭不撤销已保存配置。GUI 不删除当前模式最后一项配置，需先添加其他配置。

API 地址使用 `https://主机/路径/v1`（Claude 填 `https://主机` 即可），提供者显示名称与历史 provider ID 分离。API 模式固定使用 `custom` ID；官方模式使用内置 `openai`。官方模式保留不带自定义地址、Key、请求头的 `custom` 定义，使旧对话仍能解析。

![官方账号设置](../images/official-mode.png)

Codex 的登录、刷新和联网验证由 Codex 完成，Claude 由 Claude Code 完成。新增账号不携带其他账号旧令牌；客户端明确退出登录后，不再恢复该账号旧凭据。部分损坏的令牌阻止读取，避免误判为退出登录。

官方 HTTP 代理按客户端独立设置，只影响新启动客户端、子进程和读取同一配置的 CLI，不修改系统代理或父终端环境。Codex 的代理写在 `.env` 的托管区块，Claude 的代理写在 `settings.json` 的 `ANTHROPIC_*` 同级环境变量。清空代理或切到 API 模式时，仅移除管理的键，原有内容保留；原有代理变量仍可能生效。

窗口尺寸、位置和最大化状态在退出时记住，下次打开按记住的几何还原；记住的位置不在任何显示器上时改由系统摆放。最大化或最小化期间的尺寸不覆盖手动调整过的还原尺寸。

保存或启动遇到外部变化时保留草稿。“重新读取”可读取最新配置，存在草稿时先确认放弃。修复历史期间应关闭正在写入历史的 Codex 客户端。

## CLI

修改命令会保存，但不自动启动；`run` 是唯一启动入口。`--agent` 是全局参数，`ls` 用它过滤，`run` 和 `proxy` 用它选客户端（默认 Codex）。

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

`--key` 会把 Key 写进 shell 历史与进程列表；只在本机排障时这样用，日常请在 GUI 里填写。

```sh
acs edit example-api --model example-other-model
acs edit example-api --key example-new-key
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

不带地址时 `proxy` 只显示当前客户端的代理偏好。地址必须是 `http://主机:端口`，不含账号密码、路径或查询参数。

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

Windows 自动发现当前用户最高版本 `OpenAI.Codex` 包，Claude 查找 `AnthropicClaude` 包。macOS 查找 `/Applications/Codex.app` 和用户 Applications 目录。客户端缺失或启动失败时留在设置页；安装入口在 Windows 打开 Microsoft Store，其他平台打开对应客户端下载页。macOS“检查更新”是下载入口，不声称执行了更新检查。
