use crate::Agent;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub instructions: Instructions,
    pub skills: Skills,
    pub agents: BTreeMap<Agent, Package>,
    pub sources: BTreeMap<String, Source>,
    pub profiles: BTreeMap<String, Profile>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Instructions {
    pub global: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Skills {
    pub local: String,
    pub vendor: String,
    pub vendor_include: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Package {
    pub package: String,
    pub pin: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Source {
    pub r#type: String,
    pub path: String,
    pub url: String,
    pub lock: String,
    pub discover: Discover,
    pub build: Option<Build>,
    pub patches: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Discover {
    pub mode: String,
    pub path: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Build {
    pub template: String,
    pub output: String,
    pub replacements: BTreeMap<String, String>,
    pub commands: BTreeMap<String, Vec<String>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub agents: BTreeMap<Agent, AgentProfile>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentProfile {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub disabled_skills: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lock {
    pub version: u32,
    pub agents: BTreeMap<Agent, AgentLock>,
    pub sources: BTreeMap<String, SourceLock>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentLock {
    pub version: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourceLock {
    pub commit: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PatchDocument {
    pub source: String,
    pub patches: Vec<Patch>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Patch {
    pub id: String,
    pub file: String,
    pub op: String,
    pub start: String,
    pub end: String,
    pub r#match: String,
    pub replace: String,
}

pub fn load_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let data = std::fs::read(path).with_context(|| format!("读取 {}", path.display()))?;
    decode_yaml(&data).with_context(|| format!("解析 {}", path.display()))
}
pub fn decode_yaml<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T> {
    // 通用映射解析拒绝重复键，类型解析使用原文以保留行列。
    let _: serde_yaml::Value = serde_yaml::from_slice(data)?;
    Ok(serde_yaml::from_slice(data)?)
}
/// 所有声明的工作路径均须在所属根内。兼顾两种分隔符，拒绝 Windows 盘符/UNC。
pub fn relative_path(value: &str, empty: bool) -> Result<()> {
    if value.trim().is_empty() && !empty {
        bail!("路径不能为空");
    }
    let value = value.replace('\\', "/");
    if value.starts_with('/') || value.contains(':') {
        bail!("路径必须相对根目录：{value}");
    }
    let mut depth = 0;
    for part in value.split('/') {
        match part {
            ".." => {
                if depth == 0 {
                    bail!("路径越出根目录：{value}");
                }
                depth -= 1;
            }
            "" | "." => {}
            _ => depth += 1,
        }
    }
    Ok(())
}
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name == name.trim()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
}
#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub lock: Lock,
}
impl Repository {
    pub fn load(root: &Path) -> Result<Self> {
        let root = std::path::absolute(root)?;
        let repo = Self {
            manifest: load_yaml(&root.join("environment.yaml"))?,
            lock: load_yaml(&root.join("environment.lock"))?,
            root,
        };
        repo.validate()?;
        Ok(repo)
    }
    pub fn selection(&self, profile: &str, agent: Agent) -> Result<AgentProfile> {
        let p = self
            .manifest
            .profiles
            .get(profile)
            .with_context(|| format!("未知 Profile {profile}"))?;
        let a = p
            .agents
            .get(&agent)
            .with_context(|| format!("Profile {profile} 未配置 {}", agent.label()))?;
        Ok(AgentProfile {
            include: p.include.iter().chain(&a.include).cloned().collect(),
            exclude: p.exclude.iter().chain(&a.exclude).cloned().collect(),
            disabled_skills: a.disabled_skills.clone(),
        })
    }
    pub fn validate(&self) -> Result<()> {
        let m = &self.manifest;
        if m.version != 2 || self.lock.version != 2 {
            bail!("environment 与 lock version 必须均为 2");
        }
        for (label, p) in [
            ("instructions.global", &m.instructions.global),
            ("skills.local", &m.skills.local),
            ("skills.vendor", &m.skills.vendor),
        ] {
            relative_path(p, false).with_context(|| label)?;
        }
        for a in Agent::ALL {
            if m.agents.get(&a).is_none_or(|p| p.package.trim().is_empty()) {
                bail!("{} package 未配置", a.label());
            }
            if self
                .lock
                .agents
                .get(&a)
                .is_none_or(|p| p.version.trim().is_empty())
            {
                bail!("{} version 未锁定", a.label());
            }
        }
        if m.profiles.is_empty() {
            bail!("profiles 至少需要一项");
        }
        if !m.sources.is_empty() && m.skills.vendor_include.is_empty() {
            bail!("skills.vendor_include 必须选择发布的上游 Skills");
        }
        for (name, s) in &m.sources {
            if !safe_name(name) {
                bail!("无效 source 名 {name:?}");
            }
            match s.r#type.as_str() {
                "local" => {
                    relative_path(&s.path, false).with_context(|| format!("sources.{name}.path"))?
                }
                "git" => {
                    if s.url.trim().is_empty() {
                        bail!("git source {name} 缺少 url");
                    }
                    if self
                        .lock
                        .sources
                        .get(s.lock_name(name))
                        .is_none_or(|l| l.commit.trim().is_empty())
                    {
                        bail!("git source {name} 缺少 lock {}", s.lock_name(name));
                    }
                }
                _ => bail!("source {name} 不支持 type {:?}", s.r#type),
            }
            if !matches!(s.discover.mode.as_str(), "single" | "flat" | "collection") {
                bail!("source {name} 无效 discover.mode");
            }
            relative_path(&s.discover.path, true)?;
            if let Some(b) = &s.build {
                if b.template.is_empty() != b.output.is_empty() {
                    bail!("source {name} template/output 必须成对声明");
                }
                relative_path(&b.template, true)?;
                relative_path(&b.output, true)?;
                for platform in b.commands.keys() {
                    if !matches!(platform.as_str(), "all" | "windows" | "linux" | "darwin") {
                        bail!("source {name} 未知 build 平台 {platform}");
                    }
                }
            }
            for patch in &s.patches {
                relative_path(patch, false)?;
                let doc: PatchDocument = load_yaml(&self.root.join(patch))?;
                doc.validate(name)?;
            }
        }
        for (name, p) in &m.profiles {
            if name.trim().is_empty() || p.agents.is_empty() || p.include.is_empty() {
                bail!("Profile {name:?} 需要名称、Agent 和 include");
            }
            for (a, ap) in &p.agents {
                if *a != Agent::Codex && !ap.disabled_skills.is_empty() {
                    bail!("Profile {name} {} 不支持 disabled_skills", a.label());
                }
                let mut seen = BTreeSet::new();
                for s in &ap.disabled_skills {
                    if !safe_name(s) || !seen.insert(s) {
                        bail!("Profile {name} disabled_skills 名称不安全或重复：{s:?}");
                    }
                }
            }
        }
        Ok(())
    }
}
impl Source {
    pub fn lock_name<'a>(&'a self, name: &'a str) -> &'a str {
        if self.lock.trim().is_empty() {
            name
        } else {
            &self.lock
        }
    }
}
impl PatchDocument {
    pub fn validate(&self, source: &str) -> Result<()> {
        if self.source != source {
            bail!("patch source {} 与 {source} 不符", self.source);
        }
        for p in &self.patches {
            relative_path(&p.file, false)?;
            match p.op.as_str() {
                "delete_line" | "replace_line" => {
                    regex::Regex::new(&p.r#match).context("无效 patch match")?;
                }
                "delete_block" => {
                    regex::Regex::new(&p.start).context("无效 patch start")?;
                    regex::Regex::new(&p.end).context("无效 patch end")?;
                }
                _ => bail!("不支持 patch op {:?}", p.op),
            }
        }
        Ok(())
    }
}
