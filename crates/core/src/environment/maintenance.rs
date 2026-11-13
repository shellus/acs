use super::{
    command::{run, Runner},
    published, tree, *,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};
#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: &'static str,
    pub completed: usize,
    pub total: usize,
}
fn scalar(s: &str) -> Result<String> {
    Ok(serde_yaml::to_string(s)?.trim_end().into())
}
fn locks_text(locks: &BTreeMap<String, SourceLock>, out: &mut String) -> Result<()> {
    if locks.is_empty() {
        out.push_str("sources: {}\n");
    } else {
        out.push_str("sources:\n");
        for (name, l) in locks {
            out.push_str(&format!(
                "    {}:\n        commit: {}\n",
                scalar(name)?,
                scalar(&l.commit)?
            ));
        }
    }
    Ok(())
}
fn lock_text(lock: &Lock) -> Result<String> {
    let mut text = format!("version: {}\nagents:\n", lock.version);
    for agent in [Agent::Claude, Agent::Codex] {
        text.push_str(&format!(
            "    {}:\n        version: {}\n",
            agent.label().to_lowercase(),
            scalar(&lock.agents[&agent].version)?
        ));
    }
    locks_text(&lock.sources, &mut text)?;
    Ok(text)
}
fn snapshot_text(s: &SkillSnapshot) -> Result<String> {
    let mut text = format!("version: {}\n", s.version);
    locks_text(&s.sources, &mut text)?;
    if s.include.is_empty() {
        text.push_str("include: []\n");
    } else {
        text.push_str("include:\n");
        for p in &s.include {
            text.push_str(&format!("    - {}\n", scalar(p)?));
        }
    }
    if s.skills.is_empty() {
        text.push_str("skills: {}\n");
    } else {
        text.push_str("skills:\n");
        for (name, skill) in &s.skills {
            text.push_str(&format!(
                "    {}:\n        source: {}\n        hash: {}\n",
                scalar(name)?,
                scalar(&skill.source)?,
                scalar(&skill.hash)?
            ));
        }
    }
    Ok(text)
}
fn verify_baselines(entries: &[(PathBuf, Option<Vec<u8>>)]) -> Result<()> {
    for (p, b) in entries {
        if &storage::read(p)? != b {
            bail!("维护期间文件已变化：{}", p.display());
        }
    }
    Ok(())
}
pub fn update_lock(root: &Path, runner: &dyn Runner) -> Result<Lock> {
    let repo = Repository::load(root)?;
    let baseline = vec![
        (
            repo.root.join("environment.yaml"),
            storage::read(&repo.root.join("environment.yaml"))?,
        ),
        (
            repo.root.join("environment.lock"),
            storage::read(&repo.root.join("environment.lock"))?,
        ),
    ];
    let mut updated = Lock {
        version: 2,
        ..Default::default()
    };
    for agent in Agent::ALL {
        let p = &repo.manifest.agents[&agent];
        let version =
            String::from_utf8(run(runner, "npm", &["view", &p.package, "version"], None)?)?
                .trim()
                .to_owned();
        if version.is_empty() {
            bail!("npm 未返回 {} 版本", agent.label());
        }
        updated.agents.insert(agent, AgentLock { version });
    }
    let mut urls = BTreeMap::new();
    for (name, source) in &repo.manifest.sources {
        if source.r#type != "git" {
            continue;
        }
        let key = source.lock_name(name);
        if let Some(url) = urls.insert(key, &source.url) {
            if url != &source.url {
                bail!("共享 lock {key} 指向不同 URL");
            }
            continue;
        }
        let result = String::from_utf8(run(
            runner,
            "git",
            &["ls-remote", &source.url, "HEAD"],
            None,
        )?)?;
        let commit = result.split_whitespace().next().context("git 未返回提交")?;
        if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("git 返回无效 commit");
        }
        updated.sources.insert(
            key.into(),
            SourceLock {
                commit: commit.into(),
            },
        );
    }
    verify_baselines(&baseline)?;
    transaction::apply(&[Change {
        path: repo.root.join("environment.lock"),
        content: Some(lock_text(&updated)?.into_bytes()),
    }])?;
    Ok(updated)
}
fn copy_checkout(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for e in fs::read_dir(source)? {
        let e = e?;
        if e.file_name() == ".git" {
            continue;
        }
        tree::copy(&e.path(), &target.join(e.file_name()))?;
    }
    Ok(())
}
fn checkout(
    paths: &Paths,
    name: &str,
    url: &str,
    commit: &str,
    runner: &dyn Runner,
) -> Result<PathBuf> {
    if url.starts_with('-') || commit.starts_with('-') {
        bail!("URL/commit 不能以 - 开头");
    }
    let hash = format!("{:x}", Sha256::digest(format!("{name}\0{url}")));
    let root = paths.cache().join("sources").join(&hash[..16]);
    fs::create_dir_all(root.parent().unwrap())?;
    if !root.join(".git").exists() {
        if root.exists() {
            tree::remove(&root)?;
        }
        let temp = tempfile::tempdir_in(root.parent().unwrap())?;
        let stage = temp.path().join("repo");
        fs::create_dir(&stage)?;
        run(runner, "git", &["init"], Some(&stage))?;
        run(
            runner,
            "git",
            &["remote", "add", "origin", url],
            Some(&stage),
        )?;
        run(
            runner,
            "git",
            &["fetch", "--depth=1", "origin", commit],
            Some(&stage),
        )?;
        fs::rename(stage, &root)?;
    }
    let remote = String::from_utf8(run(
        runner,
        "git",
        &["remote", "get-url", "origin"],
        Some(&root),
    )?)?;
    if remote.trim() != url {
        bail!("上游缓存 origin 不匹配");
    }
    if run(
        runner,
        "git",
        &["cat-file", "-e", &format!("{commit}^{{commit}}")],
        Some(&root),
    )
    .is_err()
    {
        run(
            runner,
            "git",
            &["fetch", "--force", "origin", commit],
            Some(&root),
        )?;
    }
    command::checkout_files(runner, &root, commit)?;
    run(runner, "git", &["clean", "-fdx"], Some(&root))?;
    Ok(root)
}
fn build(source: &Source, root: &Path, runner: &dyn Runner) -> Result<()> {
    let Some(b) = &source.build else {
        return Ok(());
    };
    if !b.template.is_empty() {
        let mut text = fs::read_to_string(root.join(&b.template))?;
        for (key, value) in &b.replacements {
            text = text.replace(&format!("{{{{{key}}}}}"), value);
        }
        let target = root.join(&b.output);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::write(target, text)?;
    }
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    for command in b
        .commands
        .get("all")
        .into_iter()
        .flatten()
        .chain(b.commands.get(os).into_iter().flatten())
    {
        if cfg!(windows) {
            run(runner, "cmd", &["/C", command], Some(root))?;
        } else {
            run(runner, "sh", &["-c", command], Some(root))?;
        }
    }
    Ok(())
}
pub(super) fn patch_text(text: &str, p: &Patch) -> Result<String> {
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    match p.op.as_str() {
        "delete_line" => {
            let regex = regex::Regex::new(&p.r#match)?;
            Ok(lines.into_iter().filter(|l| !regex.is_match(l)).collect())
        }
        "replace_line" => {
            let regex = regex::Regex::new(&p.r#match)?;
            let mut matched = false;
            let mut out = String::new();
            for line in lines {
                if !matched && regex.is_match(line) {
                    out.push_str(&p.replace);
                    if line.ends_with('\n') {
                        out.push('\n');
                    }
                    matched = true;
                } else {
                    out.push_str(line);
                }
            }
            if !matched {
                bail!("replace_line 未匹配内容");
            }
            Ok(out)
        }
        "delete_block" => {
            let start = regex::Regex::new(&p.start)?;
            let end = regex::Regex::new(&p.end)?;
            let a = lines.iter().position(|l| start.is_match(l));
            if let Some(a) = a {
                let b = lines
                    .iter()
                    .enumerate()
                    .skip(a + 1)
                    .find(|(_, l)| end.is_match(l))
                    .map(|(i, _)| i + 1)
                    .unwrap_or(lines.len());
                Ok(lines[..a].iter().chain(&lines[b..]).copied().collect())
            } else {
                Ok(text.into())
            }
        }
        _ => bail!("未知 patch 操作 {}", p.op),
    }
}
pub fn vendor(
    paths: &Paths,
    root: &Path,
    runner: &dyn Runner,
    mut notify: impl FnMut(Progress),
) -> Result<SkillSnapshot> {
    let _lock = paths.store().lock()?;
    let repo = Repository::load(root)?;
    let target = repo.root.join(&repo.manifest.skills.vendor);
    let baseline = tree::fingerprint(&target)?;
    let mut baseline_files = vec![
        (
            repo.root.join("environment.yaml"),
            storage::read(&repo.root.join("environment.yaml"))?,
        ),
        (
            repo.root.join("environment.lock"),
            storage::read(&repo.root.join("environment.lock"))?,
        ),
    ];
    for source in repo.manifest.sources.values() {
        for p in &source.patches {
            let path = repo.root.join(p);
            baseline_files.push((path.clone(), storage::read(&path)?));
        }
    }
    fs::create_dir_all(paths.cache().join("builds"))?;
    let work = tempfile::tempdir_in(paths.cache().join("builds"))?;
    let mut checkouts = BTreeMap::<String, (String, PathBuf)>::new();
    let mut units = vec![];
    notify(Progress {
        phase: "准备上游来源",
        completed: 0,
        total: repo.manifest.sources.len(),
    });
    for (index, (name, source)) in repo.manifest.sources.iter().enumerate() {
        let src = if source.r#type == "local" {
            repo.root.join(&source.path)
        } else {
            let key = source.lock_name(name);
            if let Some((url, path)) = checkouts.get(key) {
                if url != &source.url {
                    bail!("共享 lock {key} 指向不同 URL");
                }
                path.clone()
            } else {
                let path = checkout(
                    paths,
                    key,
                    &source.url,
                    &repo.lock.sources[key].commit,
                    runner,
                )?;
                checkouts.insert(key.into(), (source.url.clone(), path.clone()));
                path
            }
        };
        let stage = work.path().join(name);
        copy_checkout(&src, &stage)?;
        build(source, &stage, runner)?;
        for file in &source.patches {
            let doc: PatchDocument = load_yaml(&repo.root.join(file))?;
            doc.validate(name)?;
            for patch in doc.patches {
                let path = stage.join(&patch.file);
                if !path.exists() {
                    continue;
                }
                let updated = patch_text(&fs::read_to_string(&path)?, &patch)
                    .with_context(|| format!("patch {file}/{}", patch.id))?;
                fs::write(path, updated)?;
            }
        }
        units.extend(published::discover_source(name, &stage, &source.discover)?);
        notify(Progress {
            phase: "准备上游来源",
            completed: index + 1,
            total: repo.manifest.sources.len(),
        });
    }
    let selected = select_units(&repo.manifest.skills.vendor_include, &[], &units)?;
    let mut seen: BTreeSet<_> =
        published::discover("local", &repo.root.join(&repo.manifest.skills.local))?
            .into_iter()
            .map(|u| u.name)
            .collect();
    fs::create_dir_all(target.parent().unwrap())?;
    let guard = tempfile::tempdir_in(target.parent().unwrap())?;
    let stage = guard.path().join("vendor");
    fs::create_dir(&stage)?;
    let mut snapshot = SkillSnapshot {
        version: 1,
        sources: repo.lock.sources.clone(),
        include: repo.manifest.skills.vendor_include.clone(),
        skills: BTreeMap::new(),
    };
    let total = selected.len();
    notify(Progress {
        phase: "生成发布快照",
        completed: 0,
        total,
    });
    for (index, unit) in selected.into_iter().enumerate() {
        if !seen.insert(unit.name.clone()) {
            bail!("重复发布 Skill {}", unit.name);
        }
        let dest = stage.join(&unit.name);
        tree::copy(&unit.path, &dest)?;
        for entry in walkdir::WalkDir::new(&dest) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            if let std::borrow::Cow::Owned(normalized) = published::portable_bytes(&bytes) {
                fs::write(entry.path(), normalized)?;
            }
        }

        snapshot.skills.insert(
            unit.name,
            SnapshotSkill {
                source: unit.source,
                hash: tree_hash(&dest)?,
            },
        );
        notify(Progress {
            phase: "生成发布快照",
            completed: index + 1,
            total,
        });
    }
    fs::write(stage.join(VENDOR_SNAPSHOT), snapshot_text(&snapshot)?)?;
    verify_baselines(&baseline_files)?;
    tree::commit(
        &[tree::DirectoryChange {
            path: target,
            stage: Some(stage),
            baseline,
        }],
        &[],
    )?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_boundaries_match_existing_operations() {
        let mut p = Patch {
            op: "replace_line".into(),
            r#match: "^old".into(),
            replace: "new".into(),
            ..Default::default()
        };
        assert_eq!(
            patch_text("old one\r\nold two\r\n", &p).unwrap(),
            "new\nold two\r\n"
        );
        assert!(patch_text("missing\n", &p).is_err());
        p.op = "delete_line".into();
        assert_eq!(
            patch_text("old one\nkeep\nold two\n", &p).unwrap(),
            "keep\n"
        );
        p.op = "delete_block".into();
        p.start = "^begin".into();
        p.end = "^end".into();
        assert_eq!(patch_text("keep\nbegin\ninside\n", &p).unwrap(), "keep\n");
        assert_eq!(patch_text("keep\n", &p).unwrap(), "keep\n");
    }
    #[test]
    fn publication_normalizes_text_but_preserves_binary() {
        assert_eq!(published::portable_bytes(b"a\r\nb\r\n").as_ref(), b"a\nb\n");
        assert_eq!(published::portable_bytes(b"a\0\r\n").as_ref(), b"a\0\r\n");
    }
}
