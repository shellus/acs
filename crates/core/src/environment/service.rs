use super::{
    command::{run, Runner},
    tree, *,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs};
#[derive(Debug)]
struct RootEdit {
    path: PathBuf,
    stage: PathBuf,
    baseline: String,
}
pub struct Plan {
    pub changes: Vec<String>,
    pub packages: Vec<(AgentPackage, String)>,
    pub profile: String,
    files: Vec<Change>,
    baselines: Vec<(PathBuf, Option<Vec<u8>>)>,
    roots: Vec<RootEdit>,
    root_baselines: Vec<(PathBuf, String)>,
    state: State,
    workspace: tempfile::TempDir,
}
impl Plan {
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty() || self.packages.iter().any(|(p, v)| !p.satisfied(v))
    }
}
pub struct Service<'a> {
    pub paths: &'a Paths,
    pub runner: &'a dyn Runner,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}
fn file_change(
    plan: &mut Plan,
    path: PathBuf,
    content: Option<Vec<u8>>,
    label: &str,
) -> Result<()> {
    let before = storage::read(&path)?;
    plan.baselines.push((path.clone(), before.clone()));
    if before != content {
        plan.changes.push(label.into());
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            let stage = plan
                .workspace
                .path()
                .join(format!("linked-file-{}", plan.roots.len()));
            storage::write(&stage, content.as_deref())?;
            plan.roots.push(RootEdit {
                baseline: tree::fingerprint(&path)?,
                path,
                stage,
            });
            return Ok(());
        }
        plan.files.push(Change { path, content });
    }
    Ok(())
}
fn payload_hash(root: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    for e in walkdir::WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.file_name() != "node_modules")
    {
        let e = e?;
        if e.file_type().is_dir() {
            continue;
        }
        hash.update(e.path().strip_prefix(root)?.to_string_lossy().as_bytes());
        hash.update(fs::read(e.path())?);
    }
    Ok(format!("{:x}", hash.finalize()))
}
impl Service<'_> {
    pub fn prepare(&self) -> Result<Plan> {
        let _lock = self.paths.store().lock()?;
        let config = LocalConfig::load(&self.paths.config())?;
        let e = &config.environment;
        if e.profile.is_empty() || e.agents.is_empty() {
            bail!("请先配置环境 Profile 与 Agent");
        }
        let repo = source_repository(self.paths, &e.source, &e.branch)?;
        published::validate(&repo)?;
        let instruction = fs::read(repo.root.join(&repo.manifest.instructions.global))?;
        fs::create_dir_all(self.paths.cache().join("builds"))?;
        let workspace = tempfile::tempdir_in(self.paths.cache().join("builds"))?;
        let state = State::load(&self.paths.state())?;
        let mut plan = Plan {
            changes: vec![],
            packages: vec![],
            profile: e.profile.clone(),
            files: vec![],
            baselines: vec![],
            roots: vec![],
            root_baselines: vec![],
            state,
            workspace,
        };
        plan.baselines
            .push((self.paths.config(), storage::read(&self.paths.config())?));
        plan.baselines
            .push((self.paths.state(), storage::read(&self.paths.state())?));
        let mut units = published::discover("local", &repo.root.join(&repo.manifest.skills.local))?;
        units.extend(published::discover(
            "vendor",
            &repo.root.join(&repo.manifest.skills.vendor),
        )?);
        let active: Vec<_> = Agent::ALL
            .into_iter()
            .map(|a| self.paths.destinations(a).1)
            .collect();
        for agent in &e.agents {
            let selection = repo.selection(&e.profile, *agent)?;
            let selected = select_units(&selection.include, &selection.exclude, &units)?;
            let pkg = repo.package(*agent)?;
            let installed = pkg.installed(self.runner)?;
            let result_version = if pkg.satisfied(&installed) {
                installed.clone()
            } else {
                pkg.version.clone()
            };
            plan.packages.push((pkg, installed));
            let (guidance, root) = self.paths.destinations(*agent);
            file_change(
                &mut plan,
                guidance,
                Some(instruction.clone()),
                &format!("{} 全局指令：更新", agent.label()),
            )?;
            if *agent == Agent::Codex {
                let path = self.paths.roots.codex.join("config.toml");
                let old = storage::read(&path)?;
                let text = std::str::from_utf8(old.as_deref().unwrap_or_default())?;
                let updated = update_disabled(text, &selection.disabled_skills)?;
                let content = if old.is_none() && updated.is_empty() {
                    None
                } else {
                    Some(updated.into_bytes())
                };
                file_change(&mut plan, path, content, "Codex 禁用 Skills 配置：更新")?;
            }
            let root_baseline = tree::fingerprint(&root)?;
            plan.root_baselines
                .push((root.clone(), root_baseline.clone()));
            let takeover = fs::symlink_metadata(&root).is_ok() && !real_dir(&root);
            let mut managed = if takeover {
                BTreeMap::new()
            } else {
                ManagedMarker::load(&root)?.skills
            };
            if managed.is_empty() {
                if let Some(s) = plan
                    .state
                    .agents
                    .get(agent)
                    .filter(|s| s.skills_root == root)
                {
                    managed = s.skills.clone();
                }
            }
            let stage = plan
                .workspace
                .path()
                .join(format!("root-{}", agent.label()));
            if real_dir(&root) {
                tree::copy(&root, &stage)?;
            } else {
                fs::create_dir(&stage)?;
            }
            let mut desired = BTreeMap::new();
            let mut root_changed = takeover;
            for unit in selected {
                let target = root.join(&unit.name);
                let rendered =
                    plan.workspace
                        .path()
                        .join(format!("render-{}-{}", agent.label(), unit.name));
                tree::copy(&unit.path, &rendered)?;
                if rendered.join("package.json").is_file() {
                    if !rendered.join("package-lock.json").is_file() {
                        bail!("Node Skill {} 缺少 package-lock.json", unit.name);
                    }
                    let reuse = managed
                        .get(&unit.name)
                        .is_some_and(|h| tree_hash(&target).is_ok_and(|actual| &actual == h))
                        && payload_hash(&target).ok() == Some(payload_hash(&unit.path)?);
                    if reuse {
                        tree::remove(&rendered)?;
                        tree::copy(&target, &rendered)?;
                    } else {
                        run(
                            self.runner,
                            "npm",
                            &["ci", "--omit=dev", "--no-audit", "--no-fund"],
                            Some(&rendered),
                        )?;
                    }
                }
                let wanted = tree_hash(&rendered)?;
                desired.insert(unit.name.clone(), wanted.clone());
                let actual = if takeover {
                    None
                } else {
                    match tree_hash(&target) {
                        Ok(h) => Some(h),
                        Err(_) if fs::symlink_metadata(&target).is_err() => None,
                        Err(error) => return Err(error),
                    }
                };
                let kind = if takeover {
                    "接管根目录"
                } else if actual.is_none() {
                    "新增"
                } else if !managed.contains_key(&unit.name) {
                    "接管同名非受管内容"
                } else if actual.as_deref() != Some(&wanted) {
                    "更新"
                } else {
                    ""
                };
                if !kind.is_empty() {
                    root_changed = true;
                    plan.changes
                        .push(format!("{} Skill {}：{kind}", agent.label(), unit.name));
                    tree::remove(&stage.join(&unit.name))?;
                    tree::copy(&rendered, &stage.join(&unit.name))?;
                }
            }
            for name in managed.keys() {
                if !desired.contains_key(name) {
                    root_changed = true;
                    plan.changes
                        .push(format!("{} Skill {name}：移除受管项", agent.label()));
                    tree::remove(&stage.join(name))?;
                }
            }
            if root_changed {
                let marker = ManagedMarker {
                    version: 1,
                    skills: desired.clone(),
                };
                storage::write(&stage.join(MANAGED_MARKER), Some(&marker.encode()?))?;
                plan.roots.push(RootEdit {
                    baseline: root_baseline,
                    path: root.clone(),
                    stage,
                });
            }
            let mut retired = vec![];
            if *agent == Agent::Codex {
                retired.push(self.paths.roots.codex.join("skills"));
            }
            if let Some(s) = plan.state.agents.get(agent) {
                retired.push(s.skills_root.clone());
            }
            for old in retired {
                if old.as_os_str().is_empty()
                    || active.contains(&old)
                    || !real_dir(&old)
                    || plan.roots.iter().any(|r| r.path == old)
                {
                    continue;
                }
                let marker = ManagedMarker::load(&old)?;
                let mut recorded = marker.skills;
                if recorded.is_empty() {
                    if let Some(s) = plan
                        .state
                        .agents
                        .get(agent)
                        .filter(|s| s.skills_root == old)
                    {
                        recorded = s.skills.clone();
                    }
                }
                if recorded.is_empty() && !old.join(MANAGED_MARKER).exists() {
                    continue;
                }
                let retired_baseline = tree::fingerprint(&old)?;
                let retired_stage = plan
                    .workspace
                    .path()
                    .join(format!("retired-{}", plan.roots.len()));
                tree::copy(&old, &retired_stage)?;
                for name in recorded.keys() {
                    if !safe_name(name) {
                        bail!("历史根含不安全 Skill 名");
                    }
                    tree::remove(&retired_stage.join(name))?;
                }
                tree::remove(&retired_stage.join(MANAGED_MARKER))?;
                plan.changes
                    .push(format!("{} 历史 Skills 根：清理受管项", agent.label()));
                plan.roots.push(RootEdit {
                    baseline: retired_baseline,
                    path: old,
                    stage: retired_stage,
                });
            }
            plan.state.agents.insert(
                *agent,
                AgentState {
                    version: result_version,
                    instruction_hash: hash(&instruction),
                    skills: desired,
                    skills_root: root,
                },
            );
        }
        plan.state.source = e.source.clone();
        plan.state.branch = e.branch.clone();
        plan.state.profile = e.profile.clone();
        plan.state.commit = run(self.runner, "git", &["rev-parse", "HEAD"], Some(&repo.root))
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .map(|s| s.trim().into())
            .unwrap_or_else(|| "local".into());
        // 拒绝配置根重叠，避免一个目录替换覆盖另一客户端或配置文件。
        for (i, r) in plan.roots.iter().enumerate() {
            let a = std::path::absolute(&r.path)?;
            for other in plan.roots.iter().skip(i + 1) {
                let b = std::path::absolute(&other.path)?;
                if a.starts_with(&b) || b.starts_with(&a) {
                    bail!("Skills 根重叠，不能安全应用");
                }
            }
            for f in &plan.baselines {
                let file = std::path::absolute(&f.0)?;
                if file.starts_with(&a) && (file != a || r.stage.is_dir()) {
                    bail!("Skills 根覆盖配置文件，不能安全应用");
                }
            }
        }
        Ok(plan)
    }
    pub fn apply(&self, plan: &Plan) -> Result<()> {
        let _lock = self.paths.store().lock()?;
        for (p, b) in &plan.baselines {
            if &storage::read(p)? != b {
                bail!("配置在计划后发生变化：{}", p.display());
            }
        }
        for (path, baseline) in &plan.root_baselines {
            if &tree::fingerprint(path)? != baseline {
                bail!("Skills 在计划后发生变化：{}", path.display());
            }
        }
        if !plan.has_changes() {
            return Ok(());
        }
        for (pkg, installed) in &plan.packages {
            if !pkg.satisfied(installed) {
                pkg.install(self.runner)?;
            }
        }
        let mut guards = vec![];
        let mut dirs = vec![];
        for r in &plan.roots {
            let parent = r.path.parent().context("Skills 根无父目录")?;
            fs::create_dir_all(parent)?;
            let guard = tempfile::tempdir_in(parent)?;
            let stage = guard.path().join("replacement");
            tree::copy(&r.stage, &stage)?;
            dirs.push(tree::DirectoryChange {
                path: r.path.clone(),
                stage,
                baseline: r.baseline.clone(),
            });
            guards.push(guard);
        }
        let mut files = plan.files.clone();
        files.push(Change {
            path: self.paths.state(),
            content: Some(plan.state.encode()?),
        });
        tree::commit(&dirs, &files)
    }
    pub fn repository(&self) -> Result<Repository> {
        let c = LocalConfig::load(&self.paths.config())?;
        source_repository(self.paths, &c.environment.source, &c.environment.branch)
    }
    pub fn install(&self, agents: &[Agent]) -> Result<Vec<String>> {
        let _lock = self.paths.store().lock()?;
        let repo = self.repository()?;
        let mut report = vec![];
        for agent in agents {
            let p = repo.package(*agent)?;
            let v = p.installed(self.runner)?;
            if p.satisfied(&v) {
                report.push(format!(
                    "{} 已安装 {v}，满足 {}",
                    agent.label(),
                    p.requirement()
                ));
            } else {
                p.install(self.runner)?;
                report.push(format!("{} 已安装 {}", agent.label(), p.version));
            }
        }
        Ok(report)
    }
    pub fn agent_status(&self, agents: &[Agent]) -> Result<Vec<String>> {
        let repo = self.repository().ok();
        agents
            .iter()
            .map(|a| {
                let pkg = match &repo {
                    Some(r) => r.package(*a)?,
                    None => default_package(*a),
                };
                let v = pkg.installed(self.runner)?;
                Ok(format!(
                    "{}：已安装 {}；要求 {}",
                    a.label(),
                    if v.is_empty() { "无" } else { &v },
                    if repo.is_none() {
                        "未配置".into()
                    } else {
                        pkg.requirement()
                    }
                ))
            })
            .collect()
    }
    pub fn uninstall(&self, agents: &[Agent], purge: bool) -> Result<()> {
        let _lock = self.paths.store().lock()?;
        let mut state = State::load(&self.paths.state())?;
        let mut files = vec![];
        let mut dirs = vec![];
        let mut guards = vec![];
        let configured = LocalConfig::load(&self.paths.config())?;
        let repository = if configured.environment.source.is_empty() {
            None
        } else {
            Some(self.repository()?)
        };
        let mut packages = vec![];
        for agent in agents {
            let pkg = match &repository {
                Some(r) => r.package(*agent)?,
                None => default_package(*agent),
            };
            packages.push(pkg);
            let (guidance, root) = self.paths.destinations(*agent);
            let mut roots = vec![root];
            if *agent == Agent::Codex {
                roots.push(self.paths.roots.codex.join("skills"));
            }
            if let Some(s) = state.agents.get(agent) {
                if !roots.contains(&s.skills_root) && !s.skills_root.as_os_str().is_empty() {
                    roots.push(s.skills_root.clone());
                }
            }
            for r in roots {
                if !real_dir(&r) {
                    continue;
                }
                let marker = ManagedMarker::load(&r)?;
                if marker.skills.is_empty() && !r.join(MANAGED_MARKER).exists() {
                    continue;
                }
                let parent = r.parent().unwrap();
                let guard = tempfile::tempdir_in(parent)?;
                let stage = guard.path().join("replacement");
                tree::copy(&r, &stage)?;
                for name in marker.skills.keys() {
                    tree::remove(&stage.join(name))?;
                }
                tree::remove(&stage.join(MANAGED_MARKER))?;
                dirs.push(tree::DirectoryChange {
                    baseline: tree::fingerprint(&r)?,
                    path: r,
                    stage,
                });
                guards.push(guard);
            }
            if state.agents.remove(agent).is_some() {
                files.push(Change {
                    path: guidance,
                    content: None,
                });
            }
            if *agent == Agent::Codex {
                let path = self.paths.roots.codex.join("config.toml");
                if let Some(b) = storage::read(&path)? {
                    files.push(Change {
                        path,
                        content: Some(update_disabled(std::str::from_utf8(&b)?, &[])?.into_bytes()),
                    });
                }
            }
        }
        files.push(Change {
            path: self.paths.state(),
            content: Some(state.encode()?),
        });
        transaction::preflight(&files)?;
        for pkg in packages {
            pkg.uninstall(self.runner)?;
        }
        tree::commit(&dirs, &files)?;
        if purge {
            for agent in agents {
                let root = match agent {
                    Agent::Codex => &self.paths.roots.codex,
                    Agent::Claude => &self.paths.roots.claude,
                };
                tree::remove(root)?;
            }
        }
        Ok(())
    }
}
fn default_package(agent: Agent) -> AgentPackage {
    AgentPackage {
        agent,
        name: match agent {
            Agent::Codex => "@openai/codex",
            Agent::Claude => "@anthropic-ai/claude-code",
        }
        .into(),
        version: String::new(),
        pin: false,
    }
}
