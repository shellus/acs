//! 环境管理的文件事务；调用方持有 Store::lock，事务不获取第二把锁。
use crate::storage;
use anyhow::{bail, Context, Result};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Change {
    pub path: PathBuf,
    /// None 删除文件，Some（包括空字节）写文件。
    pub content: Option<Vec<u8>>,
}
struct Entry {
    change: Change,
    before: Option<Vec<u8>>,
    permissions: Option<std::fs::Permissions>,
}

/// 解析已存在的祖先和文件链接，保留链接本身；悬空文件链接拒绝写入。
fn resolve(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("事务目标路径为空");
    }
    let path = std::path::absolute(path)?;
    if std::fs::symlink_metadata(&path).is_ok() {
        return Ok(path.canonicalize()?);
    }
    let parent = path.parent().context("事务目标无父目录")?;
    Ok(resolve(parent)?.join(path.file_name().context("事务目标无文件名")?))
}

pub fn apply(changes: &[Change]) -> Result<()> {
    apply_with(changes, storage::write)
}

/// 只读预检，供 npm 等不可回滚操作前排除目标格式及路径错误。
pub fn preflight(changes: &[Change]) -> Result<()> {
    prepare(changes).map(|_| ())
}
fn prepare(changes: &[Change]) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for change in changes {
        let path = resolve(&change.path)?;
        #[cfg(not(windows))]
        let identity = path.clone();
        #[cfg(windows)]
        let identity = path.to_string_lossy().to_lowercase();
        if !seen.insert(identity) {
            bail!("重复事务目标：{}", path.display());
        }
        let before = storage::read(&path)?;
        let permissions = std::fs::metadata(&path).ok().map(|m| m.permissions());
        entries.push(Entry {
            change: Change {
                path,
                content: change.content.clone(),
            },
            before,
            permissions,
        });
    }
    Ok(entries)
}

fn apply_with(
    changes: &[Change],
    mut writer: impl FnMut(&Path, Option<&[u8]>) -> Result<()>,
) -> Result<()> {
    let entries = prepare(changes)?;
    let mut written: Vec<usize> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if entry.before == entry.change.content {
            continue;
        }
        let result = (|| {
            if storage::read(&entry.change.path)? != entry.before {
                bail!("事务目标已被外部修改：{}", entry.change.path.display());
            }
            writer(&entry.change.path, entry.change.content.as_deref())
        })();
        if let Err(error) = result {
            let mut failures = Vec::new();
            for j in written.into_iter().rev() {
                let old = &entries[j];
                let rollback = (|| -> Result<()> {
                    if storage::read(&old.change.path)? != old.change.content {
                        bail!("文件再次变化，保留外部修改：{}", old.change.path.display());
                    }
                    storage::write(&old.change.path, old.before.as_deref())?;
                    if let Some(mode) = &old.permissions {
                        std::fs::set_permissions(&old.change.path, mode.clone())?;
                    }
                    Ok(())
                })();
                if let Err(e) = rollback {
                    failures.push(format!("{e:#}"));
                }
            }
            bail!(
                "事务失败：{error:#}；回滚错误：{}",
                if failures.is_empty() {
                    "无".into()
                } else {
                    failures.join("；")
                }
            );
        }
        written.push(i);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn apply_failing_at(changes: &[Change], fail: usize) -> Result<()> {
    let mut index = 0;
    apply_with(changes, |p, b| {
        let current = index;
        index += 1;
        if current == fail {
            bail!("example injected file failure");
        }
        storage::write(p, b)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn symbolic_link_is_preserved_and_alias_targets_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("example-target");
        let link = d.path().join("example-link");
        storage::write(&target, Some(b"example old")).unwrap();
        if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
            eprintln!("当前 Windows 不允许创建测试链接：{error}");
            return;
        }
        let change = Change {
            path: link.clone(),
            content: Some(b"example new".to_vec()),
        };
        apply(std::slice::from_ref(&change)).unwrap();
        assert!(link.is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"example new");
        assert!(apply(&[
            change,
            Change {
                path: target.clone(),
                content: None
            }
        ])
        .is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"example new");
    }
    #[test]
    fn unchanged_bytes_never_call_writer() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("example");
        storage::write(&p, Some(b"example")).unwrap();
        apply_with(
            &[Change {
                path: p,
                content: Some(b"example".to_vec()),
            }],
            |_, _| panic!("unchanged transaction attempted a write"),
        )
        .unwrap();
    }
    #[test]
    fn rollback_keeps_external_changes_and_continues_restoring_other_files() {
        let d = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..3).map(|i| d.path().join(i.to_string())).collect();
        for p in &paths {
            storage::write(p, Some(b"example original")).unwrap();
        }
        let changes: Vec<_> = paths
            .iter()
            .map(|p| Change {
                path: p.clone(),
                content: Some(b"example new".to_vec()),
            })
            .collect();
        let mut n = 0;
        let error = apply_with(&changes, |p, b| {
            n += 1;
            if n == 3 {
                storage::write(&paths[1], Some(b"example external"))?;
                bail!("example write failure");
            }
            storage::write(p, b)
        })
        .unwrap_err();
        assert!(error.to_string().contains("保留外部修改"));
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"example original");
        assert_eq!(std::fs::read(&paths[1]).unwrap(), b"example external");
        assert_eq!(std::fs::read(&paths[2]).unwrap(), b"example original");
    }
    #[test]
    fn write_delete_idempotence_and_rollback_at_every_step() {
        for fail in 0..3 {
            let d = tempfile::tempdir().unwrap();
            let paths: Vec<_> = (0..3).map(|i| d.path().join(i.to_string())).collect();
            storage::write(&paths[0], Some(b"example")).unwrap();
            storage::write(&paths[1], Some(b"example delete")).unwrap();
            let changes = vec![
                Change {
                    path: paths[0].clone(),
                    content: Some(b"new".to_vec()),
                },
                Change {
                    path: paths[1].clone(),
                    content: None,
                },
                Change {
                    path: paths[2].clone(),
                    content: Some(vec![]),
                },
            ];
            let mut n = 0;
            assert!(apply_with(&changes, |p, b| {
                let step = n;
                n += 1;
                if step == fail {
                    bail!("example failure");
                }
                storage::write(p, b)
            })
            .is_err());
            assert_eq!(storage::read(&paths[0]).unwrap(), Some(b"example".to_vec()));
            assert_eq!(
                storage::read(&paths[1]).unwrap(),
                Some(b"example delete".to_vec())
            );
            assert!(!paths[2].exists());
            apply(&changes).unwrap();
            apply(&changes).unwrap();
            assert!(!paths[1].exists());
            assert_eq!(std::fs::read(&paths[2]).unwrap(), b"");
        }
    }
    #[test]
    fn preflight_rejects_duplicate_directory_and_empty_targets() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("example");
        let c = Change {
            path: p.clone(),
            content: Some(vec![1]),
        };
        assert!(apply(&[c.clone(), c.clone()]).is_err());
        assert!(!p.exists());
        assert!(apply(&[
            c,
            Change {
                path: d.path().into(),
                content: None
            }
        ])
        .is_err());
        assert!(!p.exists());
        assert!(apply(&[Change {
            path: PathBuf::new(),
            content: None
        }])
        .is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        let link = d.path().join("link");
        storage::write(&target, Some(b"example")).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        apply(&[Change {
            path: link.clone(),
            content: Some(b"new".to_vec()),
        }])
        .unwrap();
        assert!(link.is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let private = d.path().join("private");
        apply(&[Change {
            path: private.clone(),
            content: Some(vec![]),
        }])
        .unwrap();
        assert_eq!(
            std::fs::metadata(private).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
