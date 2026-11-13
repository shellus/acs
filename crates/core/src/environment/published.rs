use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillUnit {
    pub source: String,
    pub name: String,
    pub path: PathBuf,
}
/// include 支持 *、名称、单 Skill 来源、来源:glob；exclude 仅精确匹配。
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
/// 与仓库的 vendor 文本及既有哈希约定一致；含 NUL 的二进制保持原字节。
pub(super) fn portable_bytes(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if bytes.contains(&0) || !bytes.windows(2).any(|w| w == b"\r\n") {
        return std::borrow::Cow::Borrowed(bytes);
    }
    std::borrow::Cow::Owned(
        bytes
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                if *b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    None
                } else {
                    Some(*b)
                }
            })
            .collect(),
    )
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
        hash.update(portable_bytes(&bytes));
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(super) fn discover_source(
    source: &str,
    work: &Path,
    discover: &Discover,
) -> Result<Vec<SkillUnit>> {
    let root = work.join(&discover.path);
    if discover.mode == "single" {
        if !root.join("SKILL.md").is_file() {
            bail!("single source {source} 缺少 SKILL.md");
        }
        return Ok(vec![make_unit(source, &root)?]);
    }
    discover_flat(source, &root)
}
pub(super) fn discover(source: &str, root: &Path) -> Result<Vec<SkillUnit>> {
    discover_flat(source, root)
}
fn discover_flat(source: &str, root: &Path) -> Result<Vec<SkillUnit>> {
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
        units.push(make_unit(source, &entry.path())?);
    }
    Ok(units)
}
fn make_unit(source: &str, path: &Path) -> Result<SkillUnit> {
    let content = std::fs::read_to_string(path.join("SKILL.md"))?;
    let mut front = false;
    let mut name = path
        .file_name()
        .context("Skill 无目录名")?
        .to_string_lossy()
        .into_owned();
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
    Ok(SkillUnit {
        source: source.into(),
        name,
        path: path.into(),
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSnapshot {
    pub version: u32,
    #[serde(default)]
    pub sources: BTreeMap<String, SourceLock>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub skills: BTreeMap<String, SnapshotSkill>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSkill {
    pub source: String,
    pub hash: String,
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
    // 只读取当前发布格式；数据仓库通过维护命令一次性切换文件名。
    let snapshot: SkillSnapshot = load_yaml(
        &repo
            .root
            .join(&repo.manifest.skills.vendor)
            .join(VENDOR_SNAPSHOT),
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
