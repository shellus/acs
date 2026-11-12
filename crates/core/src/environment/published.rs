use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillUnit {
    pub source: String,
    pub name: String,
    pub path: PathBuf,
}
/// include 支持 *、名称、单 Skill 来源、来源:glob；exclude 与 AGS 一致，仅精确匹配。
pub fn select_units(
    include: &[String],
    exclude: &[String],
    units: &[SkillUnit],
) -> Result<Vec<SkillUnit>> {
    let mut all = units.to_vec();
    all.sort_by(|a, b| (&a.source, &a.name).cmp(&(&b.source, &b.name)));
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    let mut names = BTreeMap::new();
    for pattern in include {
        let mut matched = false;
        for unit in &all {
            let key = format!("{}:{}", unit.source, unit.name);
            let matches = pattern == "*"
                || pattern == &unit.name
                || pattern == &key
                || (pattern == &unit.source
                    && all.iter().filter(|u| u.source == unit.source).count() == 1)
                || pattern
                    .strip_prefix(&format!("{}:", unit.source))
                    .is_some_and(|p| glob::Pattern::new(p).is_ok_and(|g| g.matches(&unit.name)));
            if !matches {
                continue;
            }
            matched = true;
            if exclude.contains(&unit.name) || exclude.contains(&key) || !seen.insert(key) {
                continue;
            }
            if let Some(previous) = names.insert(unit.name.clone(), unit.source.clone()) {
                if previous != unit.source {
                    bail!(
                        "Skill {} 同时来自 {} 和 {}",
                        unit.name,
                        previous,
                        unit.source
                    );
                }
            }
            selected.push(unit.clone());
        }
        if !matched {
            bail!("include {pattern:?} 没有匹配 Skill");
        }
    }
    Ok(selected)
}
pub fn tree_hash(root: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    // Go WalkDir 按每层文件名排序，不能用所有相对路径的扁平排序替代。
    for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        hash.update(relative.as_bytes());
        hash.update([0]);
        let bytes = std::fs::read(entry.path())?;
        if bytes.contains(&0) {
            hash.update(bytes);
        } else {
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                hash.update([bytes[i]]);
                i += 1;
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn discover(source: &str, root: &Path) -> Result<Vec<SkillUnit>> {
    let mut units = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let skill = entry.path().join("SKILL.md");
        if !skill.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(skill)?;
        let mut front = false;
        let mut name = entry.file_name().to_string_lossy().into_owned();
        for line in content.lines().map(str::trim) {
            if line == "---" {
                if front {
                    break;
                }
                front = true;
                continue;
            }
            if front {
                if let Some(n) = line.strip_prefix("name:") {
                    let n = n.trim().trim_matches(['\"', '\'']);
                    if !n.is_empty() {
                        name = n.into();
                    }
                    break;
                }
            }
        }
        if !safe_name(&name) {
            bail!("不安全 Skill 名 {name:?}");
        }
        units.push(SkillUnit {
            source: source.into(),
            name,
            path: entry.path(),
        });
    }
    Ok(units)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    #[serde(default)]
    sources: BTreeMap<String, SourceLock>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    skills: BTreeMap<String, SnapshotSkill>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotSkill {
    source: String,
    hash: String,
}
pub(super) fn validate(repo: &Repository) -> Result<()> {
    let mut units = discover("local", &repo.root.join(&repo.manifest.skills.local))?;
    let vendor = discover("vendor", &repo.root.join(&repo.manifest.skills.vendor))?;
    let mut seen = BTreeSet::new();
    for u in units.iter().chain(&vendor) {
        if !seen.insert(&u.name) {
            bail!("重复发布 Skill {}", u.name);
        }
    }
    // 发布快照是环境仓库数据格式的一部分，继续读取原有文件名。
    let snapshot: Snapshot = load_yaml(
        &repo
            .root
            .join(&repo.manifest.skills.vendor)
            .join(".ags-snapshot.yaml"),
    )?;
    if snapshot.version != 1
        || snapshot.sources != repo.lock.sources
        || snapshot.include != repo.manifest.skills.vendor_include
        || snapshot.skills.len() != vendor.len()
    {
        bail!("发布快照与环境声明不一致");
    }
    for u in &vendor {
        let record = snapshot.skills.get(&u.name).context("发布快照缺少 Skill")?;
        if record.source.is_empty() || record.hash != tree_hash(&u.path)? {
            bail!("Skill {} 与发布快照哈希不一致", u.name);
        }
    }
    units.extend(vendor);
    for (name, p) in &repo.manifest.profiles {
        for agent in p.agents.keys() {
            let s = repo.selection(name, *agent)?;
            select_units(&s.include, &s.exclude, &units)
                .with_context(|| format!("Profile {name} {}", agent.label()))?;
        }
    }
    Ok(())
}
