# CLI 环境维护与分发

## 仓库维护

```sh
acs env lock --repo ./example-env
acs env vendor --repo ./example-env
acs doctor
acs self version
acs self update --yes
```

lock 查询每个 Agent 的 npm 最新版本和 Git 来源的 HEAD，更新 environment.lock；共享 lock 只查询一次，相同 lock 指向不同 URL 会报错。失败或维护期间文件被外部修改时不覆盖原锁。pin 不阻止 lock 更新；pin 控制应用时是否要求精确安装。

vendor 只处理显式指定的环境仓库，以锁定提交浅拉取上游，复用同一 lock 的 checkout；在临时工作副本执行模板替换、all 与当前平台命令、补丁和 discover，然后原子发布整个 skills/vendor。sources 的 build 命令属于仓库维护声明，会在临时副本执行，apply/status/diff 不调用它们。平台键为 windows/linux/darwin。flat、collection 扫描目录下一层，single 使用指定目录；名称读取 SKILL.md frontmatter，缺省使用目录名。

补丁支持 delete_line、replace_line（首次匹配）、delete_block（首个块，结束未匹配时删除至文件末尾）；目标文件不存在时跳过，replace_line 无匹配时报错。模板替换按键排序，保证多重替换顺序稳定。所有模型严格拒绝未知字段。

发布快照名为 `.vendor-snapshot.yaml`，YAML 使用四空格格式；不读取旧文件名。vendor 文本统一 LF，含 NUL 的二进制原样保留，和可移植哈希规则一致；遵循环境仓库的 vendor eol=lf 约定，避免上游自带 CRLF 或机器 Git 配置造成字节差异。快照及所有 payload 同步替换，失败恢复旧 vendor。

维护命令会联网。下载、构建或补丁失败时保留现有 vendor，不修改本机已应用的 Agent 文件。Node Skill 的运行依赖仍由环境应用链路在其暂存目录按 package-lock 准备，不混入 vendor 发布快照。

来源与上游检出共用固定的 `core.autocrlf=false`、`core.symlinks=true`，不修改用户 Git 配置。含符号链接的仓库需要操作系统允许创建链接；Windows 维护环境应具备该权限，不能将链接悄悄发布成普通文本。

## 诊断

doctor 逐项检查 git/node/npm、本机配置、来源仓库、两个 Agent 的安装及版本、受管标记与状态、指令哈希及实际文件漂移。失败项目附带可执行修复命令，全部正常返回 0，有问题返回 1。只诊断，不自动修复或卸载；来源同步和必要的 Node 暂存可能访问网络并写缓存，不写目标 Agent 配置与 Skills。诊断不声明账号或令牌在线有效，也不修改账号逻辑。

## 安装 CLI

发布资产就绪后，Linux x86_64：

```sh
curl -fsSL https://raw.githubusercontent.com/shellus/acs/main/scripts/install.sh | bash
```

Windows x64 PowerShell：

```powershell
irm https://raw.githubusercontent.com/shellus/acs/main/scripts/install.ps1 | iex
```

默认安装到 Linux 的 ~/.local/bin/acs、Windows 的 ~/bin/acs.exe。ACS_INSTALL_DIR 可显式指定目录；脚本不写用户 PATH，完成后提示将目录加入 PATH。不需要 GUI、Node 或 npm 来运行安装脚本或 CLI 的既有账号功能。Linux 安装需要 Bash、curl、sha256sum、od 等基础命令。

离线安装使用相同脚本与校验逻辑：将对应 CLI 和 .sha256 放在一个目录，设置 ACS_RELEASE_DIR。PowerShell 也接受 -ReleaseDir 与 -InstallDir。ACS_RELEASE_BASE（Bash）或 -ReleaseBase（PowerShell）可指定已审核的发布镜像，资产名与校验格式必须一致。

| 平台 | 原始 CLI 资产 | 校验文件 |
| --- | --- | --- |
| Windows x64 | acs-cli-windows-x64.exe | 同名追加 .sha256 |
| Linux x64 静态 musl | acs-cli-linux-x64-musl | 同名追加 .sha256 |
| macOS Apple Silicon | acs-cli-macos-arm64 | 同名追加 .sha256 |
| macOS Intel（构建时） | acs-cli-macos-x86_64 | 同名追加 .sha256 |

工作流保留版本化归档，并为每个 CLI 资产生成标准 SHA-256 条目。Linux 仍使用 x86_64-unknown-linux-musl 构建，额外检查不存在 ELF 动态解释器。校验文件与 payload 必须一起发布；仅修改工作流不会补齐既有 Release 的资产。

## 自更新

self update 查询 acs 的最新 GitHub Release，只选本平台的原始 CLI 与同名 .sha256。SHA-256 必须恰好一条且匹配；同时检查 PE/ELF/Mach-O 的平台头，Linux 拒绝带动态解释器的 ELF。不支持的平台、缺失资产、重复校验、下载中断或校验失败均不替换现有程序。

非终端自更新必须 --yes；交互终端先确认。下载通过可替换 Downloader，生产默认 HTTPS GitHub API。验证完成后在可执行文件同级写临时文件并同步，再原子替换。Windows 将运行中的 CLI 移到 .old，再放入新文件，失败恢复原名；.old 可能仍由旧进程占用，需要关闭旧进程后再执行下一次更新。此链路不更新 GUI。
