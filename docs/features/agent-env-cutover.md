# 从 AGS 切换到 acs

以下步骤分别在 Windows 工作站和两台 Linux 上执行。先分发已构建的 acs CLI；本流程不安装、不卸载或删除 AGS。

## 先决条件

环境数据仓库必须可由 acs 严格校验。原仓库仍有不支持的 Agent 声明时，应先使用已移除该声明的独立副本验收；正式切换需要相应来源已完成清理。acs 不会修改或静默过滤环境仓库声明。

在切换前记录本机原来源、分支、Profile、启用 Agent，以及 AGS_STATE_DIR 等旧目录覆盖。备份 AGS 本机配置/状态、Codex config.toml 和所有受管 Skills 根，供需要整体退回时恢复。导入会改写 Skills 标记和禁用块，保留的旧 AGS 状态不代表可以直接交替运行两套工具。

## Windows 工作站

在 PowerShell 中执行；将示例路径替换为本机已清理的环境仓库副本。路径用变量承接，避免真实运行路径写入协作文档。

```powershell
$repo = 'C:\example\agent-env'
ags env status
acs env validate --repo $repo
acs env import-ags
acs env source show
acs env status
acs env diff
acs env apply --yes
acs env status
```

## 两台 Linux

在每台机器的常规 shell 中分别执行，保持原有 HOME、CODEX_HOME、CLAUDE_CONFIG_DIR 和代理环境。未设置 ACS_HOME 时使用 ~/.acs。

```sh
repo=/example/agent-env
ags env status
acs env validate --repo "$repo"
acs env import-ags
acs env source show
acs env status
acs env diff
acs env apply --yes
acs env status
```

## 每步预期

1. `ags env status`：无安装、指令、配置或 Skills 漂移；有漂移先用旧流程处理。
2. `acs env validate`：环境仓库校验通过。未知 Agent 报错时先处理仓库声明，不继续切换。
3. `acs env import-ags`：提示导入完成，若旧配置启用了不再支持项则提示丢弃；不应出现覆盖已有 acs 环境的提示。需要指定旧文件时使用下方参数。
4. `acs env source show`：中文显示旧来源、分支、Profile，以及保留的 Codex/Claude 选择。
5. `acs env status` 与 `diff`：显示“环境无漂移，无需写入”。若导入的来源尚未清理，可在此之前显式 `acs env source set <已验收副本路径> --branch <原分支>`，再核对来源；不要向未清理的远端直接 apply。
6. `acs env apply --yes`：仍显示无需写入，目标文件、标记和本机状态的字节及修改时间不变。
7. 最后的 status 仍无漂移，再启动 Codex/Claude 验证指令与 Skills。确认前保留 AGS 和切换前备份，不交替 apply。

自定义旧布局：

```sh
acs env import-ags --ags-config /example/old-config.yaml --ags-state /example/old-environment.json
```

Windows 同样接受这两个文件参数。默认发现遵守 AGS_STATE_DIR（其下仍有 ags 子目录）、XDG_STATE_HOME；Windows 回退到 APPDATA，Linux 回退到 ~/.local/state。缓存不会迁入，不影响已应用文件的接管。只有被丢弃项、没有 Codex/Claude 选择时，导入后需显式 configure。

重复 import 同一旧状态应提示“已导入，未改写任何文件”。已有 acs 环境且没有匹配迁移收据时会拒绝覆盖；先核对现场和备份，不删除 acs 文件来强行重试。
