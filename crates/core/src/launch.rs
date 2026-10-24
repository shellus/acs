use crate::{
    config::{normalize_proxy, proxy_variables},
    Agent, Mode, Roots,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    path::PathBuf,
    process::{Child, Command},
};

#[cfg(windows)]
mod windows;

pub const RESOLVER_RULES: &str = "MAP chatgpt.com 0.0.0.0, MAP *.chatgpt.com 0.0.0.0, MAP chat.openai.com 0.0.0.0, MAP *.openai.com 0.0.0.0, MAP *.oaistatic.com 0.0.0.0";
const CODEX_KEYS: [&str; 2] = ["OPENAI_API_KEY", "OPENAI_BASE_URL"];
const CLAUDE_KEYS: [&str; 3] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
];

pub struct Settings {
    pub agent: Agent,
    pub mode: Mode,
    pub proxy: String,
    pub roots: Roots,
}
#[derive(Debug, Serialize)]
pub struct Plan {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub remove_env: Vec<String>,
}
impl Settings {
    pub fn plan(&self, executable: PathBuf, desktop: bool, args: Vec<String>) -> Result<Plan> {
        let mut plan = Plan {
            executable,
            args,
            env: Vec::new(),
            remove_env: Vec::new(),
        };
        // 活动认证只能来自配置文件；父进程里的同名变量一律清掉，避免越过所选模式。
        plan.remove_env.extend(match self.agent {
            Agent::Codex => CODEX_KEYS.map(String::from).to_vec(),
            Agent::Claude => CLAUDE_KEYS.map(String::from).to_vec(),
        });
        if self.agent == Agent::Codex {
            plan.env.push((
                "CODEX_HOME".into(),
                self.roots.codex.to_string_lossy().into(),
            ));
        }
        let proxy = normalize_proxy(&self.proxy)?;
        if proxy.is_empty() {
            // 直连时 OpenAI 云端域名会让 Codex 的 Electron 外壳长时间卡住，只能阻断；
            // 走代理时这些域名可正常访问，不再阻断。Claude 桌面版在 3P 模式下不访问 claude.ai。
            if desktop && self.agent == Agent::Codex && self.mode == Mode::Custom {
                plan.args
                    .push(format!("--host-resolver-rules={RESOLVER_RULES}"));
            }
        } else {
            if desktop {
                plan.args.push(format!("--proxy-server={proxy}"));
            }
            plan.env.extend(
                proxy_variables(&proxy)
                    .into_iter()
                    .map(|(k, v)| (k.into(), v)),
            );
        }
        Ok(plan)
    }
}
impl Plan {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.executable);
        cmd.args(&self.args);
        if let Some(home) = dirs::home_dir() {
            cmd.current_dir(home);
        }
        for key in &self.remove_env {
            cmd.env_remove(key);
        }
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        cmd
    }
    pub fn spawn(&self) -> Result<Child> {
        self.command()
            .spawn()
            .with_context(|| format!("无法启动 {}", self.executable.display()))
    }

    pub fn spawn_desktop(&self) -> Result<()> {
        match self.command().spawn() {
            Ok(_) => Ok(()),
            Err(error) => {
                #[cfg(windows)]
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    if let Some(package) = windows::package_for(&self.executable)? {
                        return windows::spawn_packaged(self, &package);
                    }
                }
                Err(error).with_context(|| format!("无法启动 {}", self.executable.display()))
            }
        }
    }
}

pub fn desktop_executable(agent: Agent) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let packages = windows::installed_packages(agent).unwrap_or_default();
        if let Some(package) = packages.into_iter().next() {
            return Ok(package.executable);
        }
        // 旧版 Claude 桌面安装器（Squirrel）装在用户目录，不是 MSIX 包。
        if agent == Agent::Claude {
            if let Some(path) = dirs::data_local_dir()
                .map(|dir| dir.join("AnthropicClaude").join("claude.exe"))
                .filter(|path| path.is_file())
            {
                return Ok(path);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let bundle = match agent {
            Agent::Codex => "Codex.app/Contents/MacOS/Codex",
            Agent::Claude => "Claude.app/Contents/MacOS/Claude",
        };
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Applications"));
        }
        for root in roots {
            let path = root.join(bundle);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    bail!(
        "未找到 {} 桌面客户端；请点击“安装客户端”，或在 CLI 使用 --exe 指定路径",
        agent.label()
    )
}
pub fn launch_desktop(settings: Settings) -> Result<()> {
    let executable = desktop_executable(settings.agent)?;
    settings
        .plan(executable, true, Vec::new())?
        .spawn_desktop()?;
    Ok(())
}
pub fn cli_executable(agent: Agent) -> PathBuf {
    let stem = match agent {
        Agent::Codex => "codex",
        Agent::Claude => "claude",
    };
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            for extension in ["exe", "cmd"] {
                let candidate = directory.join(format!("{stem}.{extension}"));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from(stem)
}
pub fn open_download(agent: Agent, updates: bool) -> Result<()> {
    if agent == Agent::Claude {
        // 桌面版自带更新；下载地址按平台跳转到最新安装包，需要浏览器请求。
        let url = if updates {
            "https://claude.com/download"
        } else if cfg!(target_os = "macos") {
            "https://claude.ai/api/desktop/darwin/universal/dmg/latest/redirect"
        } else if cfg!(all(windows, target_arch = "aarch64")) {
            "https://claude.ai/api/desktop/win32/arm64/setup/latest/redirect"
        } else if cfg!(windows) {
            "https://claude.ai/api/desktop/win32/x64/setup/latest/redirect"
        } else {
            "https://code.claude.com/docs/en/setup"
        };
        open::that(url)?;
        return Ok(());
    }
    #[cfg(windows)]
    {
        let uri = if updates {
            "ms-windows-store://downloadsandupdates"
        } else {
            "ms-windows-store://pdp/?ProductId=9PLM9XGG6VKS"
        };
        if open::that(uri).is_ok() {
            return Ok(());
        }
        open::that("https://apps.microsoft.com/detail/9PLM9XGG6VKS")?;
    }
    #[cfg(not(windows))]
    {
        let _ = updates;
        open::that("https://chatgpt.com/codex")?;
    }
    Ok(())
}
