# acs

管理 Codex 与 Claude 的官方账号、自定义 API 和独立代理。采用 Rust 共享核心、Tauri 桌面界面和独立命令行入口。

[下载发布版本](https://github.com/shellus/acs/releases/latest) · [使用与 CLI 命令](docs/features/usage.md) · [架构和行为边界](docs/dev-spec/application-behavior.md)

## 支持范围

| 平台 | 入口 | 分发依赖 |
| --- | --- | --- |
| Windows x64 | Tauri GUI（`acs-gui.exe`）、CLI（`acs.exe`） | GUI 使用 WebView2；安装包可在缺失时安装运行时，便携 EXE 要求已安装 WebView2 |
| macOS Apple Silicon | Tauri GUI（`.app`）、CLI | GUI 使用系统 WKWebView；桌面目标是 Electron 版 Codex.app |
| Linux x64 | 独立 CLI | musl 静态构建，不依赖 Tauri、WebView、Node.js 或系统 SQLite |

macOS 原生 ChatGPT.app 不等同于 Electron Codex.app，不能套用相同的域名阻断参数。

Codex 与 Claude 各自独立生效：可以 Codex 停在官方账号，同时 Claude 走自定义 API。两边的草稿在同一窗口编辑，切换客户端不丢失编辑。

## 桌面使用

1. 打开应用，选择 Codex 或 Claude，再选择“官方账号”或“自定义 API”，添加或选择配置。
2. 填写并点击当前页的“保存配置”。保存不会启动或停止客户端。
3. 点击底部“启动”。未保存修改或外部配置变化会阻止启动，并保留草稿。

官方账号通过官方客户端登录。新增账号保存后启动即可登录；本地存在凭据不代表账号有效。官方模式可设置客户端共用的 HTTP 代理，每条自定义 API 配置也可单独设置代理（海外中转站等）。Codex 自定义模式直连时只阻断 Electron 外壳的指定云端域名，不阻断内置 app-server 访问自定义 API；配置了代理时改走代理，不再阻断。

Codex 只托管 `model_providers.custom`；手工添加的其他 provider 在保存和模式切换时原样保留，不进入 acs 配置快照。

关闭按钮、标题栏和 Esc 共用草稿确认。只有显式“修复对话”才改写 Codex 历史 provider ID，修复前创建备份。

![官方账号设置](docs/images/official-mode.png)

![自定义 API 设置](docs/images/custom-mode.png)

## 开发与验证

需要 Rust stable、Node.js 24；Windows 构建需要 MSVC C++ 构建工具和 Windows SDK，macOS 构建需要 Xcode Command Line Tools。

```sh
npm ci
npm run build
cargo fmt --all -- --check
cargo test -p acs-core -p acs --locked
cargo clippy -p acs-core -p acs --all-targets -- -D warnings
cargo build -p acs --release --locked
```

Windows / macOS 桌面开发和打包：

```sh
npm run tauri -- dev
npm run tauri -- build
```

仅构建桌面可执行文件：`cargo build -p acs-gui --release --locked`，此前须完成 `npm run build`。Windows 产物为 `target/release/acs-gui.exe`，CLI 为 `target/release/acs`（Windows 带 `.exe`）。

```sh
npx playwright install chromium
npm test
```

Windows 可运行 `node scripts/desktop-smoke.mjs`，以临时 example 配置验证正式程序的实际 IPC、保存、冲突和关闭行为；可追加 EXE 路径以核对 CI 下载的发布文件。Rust 测试显式传入临时 Store；GUI 测试使用隔离 fixture，不修改真实用户认证。

CI 在 Windows、Linux、macOS 运行核心和 CLI 测试，构建 Windows/macOS GUI，并上传平台产物。行为验证与运行边界见[架构文档](docs/dev-spec/application-behavior.md)。

## 许可证

[GNU Affero General Public License v3.0](LICENSE)，SPDX：`AGPL-3.0-only`。历史 provider 修复的初始数据范围与备份思路参考 [Codex++](https://github.com/BigPizzaV3/CodexPlusPlus)。
