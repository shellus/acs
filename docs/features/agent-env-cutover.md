# 从 AGS 切换到 acs

AGS 的环境管理已并入 acs，独立执行器退役。`acs env import-ags` 显式接管本机选择、状态、受管标记和 Codex 禁用块；不导入 Provider，不删除旧程序、配置、缓存或状态。迁移入口之外不读取旧格式。

环境仓库 main 已使用 `.vendor-snapshot.yaml`，AGS 无法再读取，切换时不需要过渡分支或新旧工具并行。

## 已有机器

安装方法见 [CLI 分发](cli-maintenance.md#安装-cli)。安装后在 Windows PowerShell 或 Linux shell 运行同一组命令：

```sh
acs env import-ags
acs env diff
acs env apply --yes
acs env status
acs doctor
```

导入保留原来源、Profile 和 Codex/Claude 选择；旧配置中不再支持的项会提示丢弃。首次导入前不要先 source set/configure 创建新的 acs 环境，否则会拒绝覆盖。第一次 apply 可能安装锁要求的 Agent 版本并更新全局指令与 Skills；之后 status 应显示无漂移，doctor 返回 0。

没有旧环境的新机器跳过导入，直接 `acs env source set <仓库地址>` 和 `acs env configure --profile <名称> --agents codex,claude`。

## 旧路径与幂等

旧配置默认 `~/.ags/config.yaml`。Windows 默认状态为 `%APPDATA%/ags/environment.json`，缓存 `%LOCALAPPDATA%/ags`；Linux 默认状态为 `~/.local/state/ags/environment.json`，缓存 `~/.cache/ags`。状态遵守 `AGS_STATE_DIR`、`XDG_STATE_HOME`，Unix 缓存遵守 `XDG_CACHE_HOME`；覆盖值指向的实际目录需另行清理。

```sh
acs env import-ags --ags-config /example/old-config.yaml --ags-state /example/old-environment.json
```

收据为 `ACS_HOME/state/environment-import.json`，只记录导入来源和哈希；重复导入同一旧状态不写文件。已有 acs 环境且无匹配收据时拒绝覆盖，不应删除 acs 配置强行重试。

## 清理

确认 acs 环境无漂移后，删除旧可执行文件及上面列出的旧配置、状态和缓存目录。不删除 `.codex`、`.claude`、`.agents` 或环境数据仓库。导入和文件应用有进程内事务恢复，但不支持掉电恢复；全局 npm 安装不属于文件事务。
