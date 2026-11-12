//! Skills 目录替换与文件事务组合；备份始终在目标同级，失败恢复目录实体/链接。
use super::*;
use sha2::{Digest, Sha256};
use std::fs;
#[derive(Debug)]
pub struct DirectoryChange {
    pub path: PathBuf,
    pub stage: PathBuf,
    pub baseline: String,
}
/// 原字节与链接目标参与并发基线，不跟随目录链接。
pub fn fingerprint(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    if !path.try_exists()? && fs::symlink_metadata(path).is_err() {
        return Ok("absent".into());
    }
    for e in walkdir::WalkDir::new(path)
        .follow_links(false)
        .sort_by_file_name()
    {
        let e = e?;
        hash.update(e.path().strip_prefix(path)?.to_string_lossy().as_bytes());
        let ty = e.file_type();
        if ty.is_symlink() {
            hash.update(b"link");
            hash.update(fs::read_link(e.path())?.to_string_lossy().as_bytes());
        } else if ty.is_dir() {
            hash.update(b"dir");
        } else {
            hash.update(b"file");
            hash.update(fs::read(e.path())?);
        }
        hash.update([0]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn remove(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            #[cfg(windows)]
            {
                if m.is_dir() {
                    fs::remove_dir(path)?;
                } else {
                    fs::remove_file(path)?;
                }
            }
            #[cfg(not(windows))]
            fs::remove_file(path)?;
        }
        Ok(m) if m.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
pub fn copy(source: &Path, target: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(source)?;
    if meta.file_type().is_symlink() {
        let link = fs::read_link(source)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(link, target)?;
        #[cfg(windows)]
        {
            if source.is_dir() {
                std::os::windows::fs::symlink_dir(link, target)?;
            } else {
                std::os::windows::fs::symlink_file(link, target)?;
            }
        }
    } else if meta.is_dir() {
        fs::create_dir_all(target)?;
        for e in fs::read_dir(source)? {
            let e = e?;
            copy(&e.path(), &target.join(e.file_name()))?;
        }
        fs::set_permissions(target, meta.permissions())?;
    } else {
        fs::copy(source, target)?;
    }
    Ok(())
}
pub fn commit(dirs: &[DirectoryChange], files: &[Change]) -> Result<()> {
    commit_with(
        dirs,
        files,
        |a, b| fs::rename(a, b).map_err(Into::into),
        transaction::apply,
    )
}
fn commit_with(
    dirs: &[DirectoryChange],
    files: &[Change],
    mut rename: impl FnMut(&Path, &Path) -> Result<()>,
    mut file_writer: impl FnMut(&[Change]) -> Result<()>,
) -> Result<()> {
    transaction::preflight(files)?;
    for d in dirs {
        if fingerprint(&d.path)? != d.baseline {
            bail!("Skills 在计划后变化：{}", d.path.display());
        }
    }
    let mut applied: Vec<(&DirectoryChange, tempfile::TempDir, bool, bool, String)> = vec![];
    let result = (|| -> Result<()> {
        for d in dirs {
            let parent = d.path.parent().context("Skills 目标无父目录")?;
            fs::create_dir_all(parent)?;
            let backup = tempfile::tempdir_in(parent)?;
            let existed = fs::symlink_metadata(&d.path).is_ok();
            if existed {
                rename(&d.path, &backup.path().join("original"))?;
            }
            let expected = fingerprint(&d.stage)?;
            applied.push((d, backup, existed, false, expected));
            rename(&d.stage, &d.path)?;
            applied.last_mut().unwrap().3 = true;
        }
        file_writer(files)
    })();
    if let Err(error) = result {
        let mut failures = vec![];
        for (d, backup, existed, installed, expected) in applied.iter().rev() {
            let restore = (|| -> Result<()> {
                let current = fingerprint(&d.path)?;
                if (*installed && &current != expected) || (!installed && current != "absent") {
                    bail!("Skills 再次变化，保留外部修改");
                }
                remove(&d.path)?;
                if *existed {
                    fs::rename(backup.path().join("original"), &d.path)?;
                }
                Ok(())
            })();
            if let Err(e) = restore {
                failures.push(format!("{}: {e:#}", d.path.display()));
            }
        }
        // 回滚不完整时留下备份路径，避免 TempDir 析构误删唯一旧数据。
        if !failures.is_empty() {
            let kept: Vec<_> = applied
                .into_iter()
                .map(|(_, b, _, _, _)| b.keep().display().to_string())
                .collect();
            bail!(
                "应用失败：{error:#}；回滚失败：{}；保留备份：{}",
                failures.join("；"),
                kept.join("，")
            );
        }
        return Err(error.context("应用失败，已恢复文件和 Skills 目录"));
    }
    for (_, backup, _, _, _) in applied {
        let path = backup.keep();
        if let Err(error) = remove(&path) {
            bail!(
                "环境已应用，旧目录备份清理失败：{}：{error:#}",
                path.display()
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_file_failure_restores_files_and_skills_together() {
        for fail in 0..3 {
            let d = tempfile::tempdir().unwrap();
            let root = d.path().join("skills");
            fs::create_dir(&root).unwrap();
            storage::write(&root.join("example"), Some(b"old skill")).unwrap();
            let stage = d.path().join("stage");
            fs::create_dir(&stage).unwrap();
            storage::write(&stage.join("example"), Some(b"new skill")).unwrap();
            let dirs = vec![DirectoryChange {
                baseline: fingerprint(&root).unwrap(),
                path: root.clone(),
                stage,
            }];
            let files: Vec<_> = (0..3)
                .map(|i| Change {
                    path: d.path().join(format!("file{i}")),
                    content: Some(b"new".to_vec()),
                })
                .collect();
            for c in &files {
                storage::write(&c.path, Some(b"old")).unwrap();
            }
            assert!(commit_with(
                &dirs,
                &files,
                |a, b| fs::rename(a, b).map_err(Into::into),
                |c| transaction::apply_failing_at(c, fail)
            )
            .is_err());
            for c in &files {
                assert_eq!(fs::read(&c.path).unwrap(), b"old");
            }
            assert_eq!(fs::read(root.join("example")).unwrap(), b"old skill");
        }
    }
    #[cfg(unix)]
    #[test]
    fn failed_file_save_restores_linked_root_without_touching_link_target() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        fs::create_dir(&target).unwrap();
        storage::write(&target.join("example"), Some(b"original")).unwrap();
        let root = d.path().join("skills");
        std::os::unix::fs::symlink(&target, &root).unwrap();
        let stage = d.path().join("stage");
        fs::create_dir(&stage).unwrap();
        let change = DirectoryChange {
            baseline: fingerprint(&root).unwrap(),
            path: root.clone(),
            stage,
        };
        assert!(commit_with(
            &[change],
            &[],
            |a, b| fs::rename(a, b).map_err(Into::into),
            |_| bail!("example file failure")
        )
        .is_err());
        assert!(root.is_symlink());
        assert_eq!(fs::read(target.join("example")).unwrap(), b"original");
    }
    #[test]
    fn every_directory_rename_failure_restores_all_roots() {
        for step in 0..4 {
            let d = tempfile::tempdir().unwrap();
            let mut changes = vec![];
            for i in 0..2 {
                let p = d.path().join(format!("root{i}"));
                fs::create_dir(&p).unwrap();
                storage::write(&p.join("example"), Some(b"old")).unwrap();
                let stage = d.path().join(format!("stage{i}"));
                fs::create_dir(&stage).unwrap();
                storage::write(&stage.join("example"), Some(b"new")).unwrap();
                changes.push(DirectoryChange {
                    baseline: fingerprint(&p).unwrap(),
                    path: p,
                    stage,
                });
            }
            let mut n = 0;
            assert!(commit_with(
                &changes,
                &[],
                |a, b| {
                    let fail = n == step;
                    n += 1;
                    if fail {
                        bail!("example rename failure");
                    }
                    fs::rename(a, b)?;
                    Ok(())
                },
                transaction::apply
            )
            .is_err());
            for c in changes {
                assert_eq!(fs::read(c.path.join("example")).unwrap(), b"old");
            }
        }
    }
    #[test]
    fn file_preflight_failure_restores_replaced_directory() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        fs::create_dir(&root).unwrap();
        storage::write(&root.join("example"), Some(b"old")).unwrap();
        let stage = d.path().join("stage");
        fs::create_dir(&stage).unwrap();
        let change = DirectoryChange {
            baseline: fingerprint(&root).unwrap(),
            path: root.clone(),
            stage,
        };
        assert!(commit(
            &[change],
            &[Change {
                path: d.path().into(),
                content: Some(vec![])
            }]
        )
        .is_err());
        assert_eq!(fs::read(root.join("example")).unwrap(), b"old");
    }
}
