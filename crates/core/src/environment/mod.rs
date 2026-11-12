//! Agent 环境基础；不会在非 env 命令路径探测或调用 git/npm。
mod agent;
pub mod command;
pub use agent::{compare_versions, AgentPackage};
mod disabled;
pub use disabled::{split_block, update_disabled};
mod doctor;
mod service;
pub use doctor::{doctor, Check, DoctorReport};
mod tree;
pub use service::{Plan, Service};
mod migration;
pub use migration::{import_ags, LegacyPaths, LegacyPlatform};
mod state;
pub use state::{AgentState, ManagedMarker, State};
mod maintenance;
mod model;
mod published;
use crate::{
    storage,
    transaction::{self, Change},
    Agent, Roots, Store,
};
use anyhow::{bail, Context, Result};
use command::Runner;
pub use maintenance::{update_lock, vendor, Progress};
pub use model::*;
pub use published::{select_units, tree_hash, SkillUnit};
pub use published::{SkillSnapshot, SnapshotSkill};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub const MANAGED_MARKER: &str = ".acs-managed.json";
pub const DISABLED_BEGIN: &str = "# BEGIN ACS MANAGED CODEX DISABLED SKILLS";
pub const DISABLED_END: &str = "# END ACS MANAGED CODEX DISABLED SKILLS";
#[derive(Clone, Debug)]
pub struct Paths {
    pub roots: Roots,
    pub codex_skills: PathBuf,
}
impl Paths {
    pub fn under(root: &Path) -> Self {
        Self {
            roots: Roots::under(root),
            codex_skills: root.join(".agents/skills"),
        }
    }
    pub fn discover() -> Result<Self> {
        let home = dirs::home_dir().context("无法确定用户主目录")?;
        Ok(Self {
            roots: Roots::discover()?,
            codex_skills: home.join(".agents/skills"),
        })
    }
    pub fn resolve(home: &Path, env: impl Fn(&str) -> Option<PathBuf>) -> Result<Self> {
        #[cfg(windows)]
        let parent = env("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"));
        #[cfg(target_os = "macos")]
        let parent = home.join("Library/Application Support");
        #[cfg(not(any(windows, target_os = "macos")))]
        let parent = env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        Ok(Self {
            roots: Roots::resolve(home, &parent, env)?,
            codex_skills: home.join(".agents/skills"),
        })
    }
    pub fn config(&self) -> PathBuf {
        self.roots.acs.join("environment.yaml")
    }
    pub fn cache(&self) -> PathBuf {
        self.roots.acs.join("cache")
    }
    pub fn state(&self) -> PathBuf {
        self.roots.acs.join("state/environment.json")
    }
    pub fn destinations(&self, agent: Agent) -> (PathBuf, PathBuf) {
        match agent {
            Agent::Codex => (
                self.roots.codex.join("AGENTS.md"),
                self.codex_skills.clone(),
            ),
            Agent::Claude => (
                self.roots.claude.join("CLAUDE.md"),
                self.roots.claude.join("skills"),
            ),
        }
    }
    pub fn store(&self) -> Store {
        Store::new(self.roots.clone())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalConfig {
    pub version: u32,
    pub environment: EnvironmentConfig,
}
impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            version: 1,
            environment: EnvironmentConfig::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentConfig {
    pub source: String,
    pub branch: String,
    pub profile: String,
    pub agents: Vec<Agent>,
}
impl Default for EnvironmentConfig {
    fn default() -> Self {
        Self {
            source: String::new(),
            branch: "main".into(),
            profile: String::new(),
            agents: vec![],
        }
    }
}
impl LocalConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let Some(data) = storage::read(path)? else {
            return Ok(Self::default());
        };
        let c: Self = decode_yaml(&data).with_context(|| format!("解析 {}", path.display()))?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("不支持本机环境配置版本 {}", self.version);
        }
        if self.environment.branch.trim().is_empty() {
            bail!("environment.branch 不能为空");
        }
        let mut seen = BTreeSet::new();
        for a in &self.environment.agents {
            if !seen.insert(a) {
                bail!("重复 environment agent {}", a.label());
            }
        }
        Ok(())
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let mut c = self.clone();
        let e = &mut c.environment;
        e.source = e.source.trim().into();
        e.branch = e.branch.trim().into();
        e.profile = e.profile.trim().into();
        e.agents.sort_by_key(|a| a.label());
        transaction::apply(&[Change {
            path: path.into(),
            content: Some(serde_yaml::to_string(&c)?.into_bytes()),
        }])
    }
}
/// 来源缓存只在显式 env 操作时访问。工作副本为本机路径时不调用 git。
pub fn source_repository(paths: &Paths, source: &str, branch: &str) -> Result<Repository> {
    use sha2::{Digest, Sha256};
    let source = source.trim();
    let branch = branch.trim();
    if source.is_empty() || branch.is_empty() {
        bail!("来源与分支不能为空");
    }
    if Path::new(source).is_dir() {
        return Repository::load(Path::new(source));
    }
    if source.starts_with('-') || branch.starts_with('-') {
        bail!("来源或分支不能以 - 开头");
    }
    let digest = format!("{:x}", Sha256::digest(format!("{source}\0{branch}")));
    let checkout = paths.cache().join("environment").join(&digest[..16]);
    let git = |dir: Option<&Path>, args: &[&std::ffi::OsStr]| {
        command::SystemRunner.run(
            "git",
            &args.iter().map(|v| v.to_os_string()).collect::<Vec<_>>(),
            dir,
        )
    };
    if !checkout.join(".git").exists() {
        std::fs::create_dir_all(checkout.parent().unwrap())?;
        if checkout.exists() {
            std::fs::remove_dir_all(&checkout)?;
        }
        let stage = tempfile::tempdir_in(checkout.parent().unwrap())?;
        let staged = stage.path().join("checkout");
        git(
            None,
            &[
                "clone".as_ref(),
                "--no-checkout".as_ref(),
                "--".as_ref(),
                source.as_ref(),
                staged.as_os_str(),
            ],
        )?;
        std::fs::rename(staged, &checkout)?;
    }
    let remote = git(
        Some(&checkout),
        &["remote".as_ref(), "get-url".as_ref(), "origin".as_ref()],
    )?;
    if String::from_utf8_lossy(&remote).trim() != source {
        bail!("环境缓存 origin 与来源不符");
    }
    git(
        Some(&checkout),
        &[
            "fetch".as_ref(),
            "--prune".as_ref(),
            "origin".as_ref(),
            branch.as_ref(),
        ],
    )?;
    git(
        Some(&checkout),
        &[
            "checkout".as_ref(),
            "--detach".as_ref(),
            "--force".as_ref(),
            "FETCH_HEAD".as_ref(),
        ],
    )?;
    git(Some(&checkout), &["clean".as_ref(), "-fdx".as_ref()])?;
    Repository::load(&checkout)
}
pub fn set_source(paths: &Paths, source: &str, branch: Option<&str>) -> Result<LocalConfig> {
    let _lock = paths.store().lock()?;
    let mut c = LocalConfig::load(&paths.config())?;
    let repo = source_repository(paths, source, branch.unwrap_or(&c.environment.branch))?;
    c.environment.source = source.into();
    if let Some(b) = branch {
        c.environment.branch = b.into();
    }
    if c.environment.profile.is_empty() && repo.manifest.profiles.contains_key("default") {
        c.environment.profile = "default".into();
    }
    c.save(&paths.config())?;
    LocalConfig::load(&paths.config())
}
pub fn configure(
    paths: &Paths,
    profile: Option<&str>,
    agents: Option<Vec<Agent>>,
) -> Result<LocalConfig> {
    let _lock = paths.store().lock()?;
    let mut c = LocalConfig::load(&paths.config())?;
    let repo = source_repository(paths, &c.environment.source, &c.environment.branch)?;
    if let Some(p) = profile {
        c.environment.profile = p.into();
    }
    if let Some(a) = agents {
        c.environment.agents = a;
    }
    if c.environment.agents.is_empty() {
        bail!("至少启用一个 Agent");
    }
    for a in &c.environment.agents {
        repo.selection(&c.environment.profile, *a)?;
    }
    c.save(&paths.config())?;
    LocalConfig::load(&paths.config())
}
pub fn validate(root: &Path) -> Result<()> {
    let repo = Repository::load(root)?;
    std::fs::read(repo.root.join(&repo.manifest.instructions.global)).context("读取全局指令")?;
    published::validate(&repo)
}
