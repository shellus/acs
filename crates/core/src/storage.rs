use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// 目标文件的读写视角。整份文件参与基线时用 `Whole`；`Field` 只把 JSON 对象的
/// 一个顶层字段纳入基线并局部改写，供 `~/.claude.json` 这类由客户端持续写入的
/// 大文件使用——否则每次保存都会因为无关字段的变动报外部修改。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lens {
    Whole,
    Field(&'static str),
}
pub struct Target {
    pub label: &'static str,
    pub path: PathBuf,
    pub lens: Lens,
}

pub const LIBRARY: usize = 0;
pub const CODEX_CONFIG: usize = 1;
pub const CODEX_AUTH: usize = 2;
pub const CODEX_ENV: usize = 3;
pub const CLAUDE_SETTINGS: usize = 4;
pub const CLAUDE_CREDENTIALS: usize = 5;
pub const CLAUDE_ACCOUNT: usize = 6;
pub const DESKTOP_MODE: usize = 7;
pub const DESKTOP_META: usize = 8;
pub const DESKTOP_PROFILE: usize = 9;
pub const TARGETS: usize = 10;

/// acs 在 Claude 桌面版 `configLibrary` 里固定使用的条目 ID；桌面版只接受 UUID 形式。
pub const DESKTOP_PROFILE_ID: &str = "00000000-0000-4000-8000-000000000ac5";

const WINDOW_STATE: &str = "window.json";
pub type Snapshot = Vec<Option<Vec<u8>>>;

/// 各客户端的配置根目录。配置库、窗口状态和日志放在本程序自己的 `acs` 目录，
/// 其余目录属于被管理的客户端。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roots {
    pub acs: PathBuf,
    pub codex: PathBuf,
    pub claude: PathBuf,
    pub claude_json: PathBuf,
    /// Claude 桌面版第三方推理（3P）模式的数据目录，桌面版固定从这里读取 3P 配置。
    pub claude_desktop: PathBuf,
}
impl Roots {
    pub fn discover() -> Result<Self> {
        let home = dirs::home_dir();
        #[cfg(target_os = "linux")]
        let desktop = dirs::config_dir();
        #[cfg(not(target_os = "linux"))]
        let desktop = dirs::data_local_dir();
        Self::resolve_optional(home.as_deref(), desktop.as_deref(), |k| {
            std::env::var_os(k).map(PathBuf::from)
        })
    }
    /// 保持账号入口语义：变量原样使用，包括空值和字面 ~；未设置才回退。
    pub fn resolve(
        home: &Path,
        desktop: &Path,
        env: impl Fn(&str) -> Option<PathBuf>,
    ) -> Result<Self> {
        if home.as_os_str().is_empty() {
            bail!("用户主目录为空");
        }
        Self::resolve_optional(Some(home), Some(desktop), env)
    }
    fn resolve_optional(
        home: Option<&Path>,
        desktop: Option<&Path>,
        env: impl Fn(&str) -> Option<PathBuf>,
    ) -> Result<Self> {
        let directory = |key, default| {
            env(key)
                .or_else(|| home.map(|p| p.join(default)))
                .with_context(|| format!("无法确定 {key} 配置目录"))
        };
        let claude_home = env("CLAUDE_CONFIG_DIR");
        Ok(Self {
            acs: directory("ACS_HOME", ".acs")?,
            codex: directory("CODEX_HOME", ".codex")?,
            claude: directory("CLAUDE_CONFIG_DIR", ".claude")?,
            claude_json: match claude_home {
                Some(p) => p.join(".claude.json"),
                None => home.context("无法确定用户主目录")?.join(".claude.json"),
            },
            claude_desktop: env("ACS_CLAUDE_DESKTOP_DIR")
                .or_else(|| desktop.map(|p| p.join("Claude-3p")))
                .context("无法确定 Claude 桌面版配置目录")?,
        })
    }
    /// 测试与 GUI 自动化使用的隔离目录，禁止落到真实用户配置上。
    pub fn under(dir: &Path) -> Self {
        Self {
            acs: dir.join("acs"),
            codex: dir.join("codex"),
            claude: dir.join("claude"),
            claude_json: dir.join("claude.json"),
            claude_desktop: dir.join("claude-desktop"),
        }
    }
}
#[derive(Clone)]
pub struct Store {
    pub roots: Roots,
}
impl Store {
    pub fn new(roots: Roots) -> Self {
        Self { roots }
    }
    pub fn discover() -> Result<Self> {
        Ok(Self::new(Roots::discover()?))
    }
    pub fn targets(&self) -> Vec<Target> {
        let r = &self.roots;
        vec![
            Target {
                label: "profiles.json",
                path: r.acs.join("profiles.json"),
                lens: Lens::Whole,
            },
            Target {
                label: "config.toml",
                path: r.codex.join("config.toml"),
                lens: Lens::Whole,
            },
            Target {
                label: "auth.json",
                path: r.codex.join("auth.json"),
                lens: Lens::Whole,
            },
            Target {
                label: ".env",
                path: r.codex.join(".env"),
                lens: Lens::Whole,
            },
            Target {
                label: "settings.json",
                path: r.claude.join("settings.json"),
                lens: Lens::Whole,
            },
            Target {
                label: ".credentials.json",
                path: r.claude.join(".credentials.json"),
                lens: Lens::Whole,
            },
            Target {
                label: ".claude.json 的 oauthAccount",
                path: r.claude_json.clone(),
                lens: Lens::Field("oauthAccount"),
            },
            // 桌面版持续改写自己的偏好，只管理决定 1P/3P 的 deploymentMode。
            Target {
                label: "Claude 桌面版的 deploymentMode",
                path: r.claude_desktop.join("claude_desktop_config.json"),
                lens: Lens::Field("deploymentMode"),
            },
            Target {
                label: "Claude 桌面版 configLibrary/_meta.json",
                path: r.claude_desktop.join("configLibrary").join("_meta.json"),
                lens: Lens::Whole,
            },
            Target {
                label: "Claude 桌面版 acs 配置",
                path: r
                    .claude_desktop
                    .join("configLibrary")
                    .join(format!("{DESKTOP_PROFILE_ID}.json")),
                lens: Lens::Whole,
            },
        ]
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        self.targets().iter().map(read_target).collect()
    }
    pub fn lock(&self) -> Result<File> {
        let path = self.roots.acs.join("operation.lock");
        fs::create_dir_all(path.parent().unwrap())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive()
            .context("另一项保存或修复正在执行，请稍后重试")?;
        Ok(file)
    }
    pub fn check(&self, baseline: &Snapshot) -> Result<()> {
        if &self.snapshot()? != baseline {
            bail!("配置已被其他程序修改，请重新读取配置；当前草稿尚未保存")
        }
        Ok(())
    }
    pub fn commit(&self, before: &Snapshot, after: &Snapshot) -> Result<()> {
        self.commit_with(before, after, write_target)
    }
    fn commit_with(
        &self,
        before: &Snapshot,
        after: &Snapshot,
        mut writer: impl FnMut(&Target, Option<&[u8]>) -> Result<()>,
    ) -> Result<()> {
        self.check(before)?;
        let targets = self.targets();
        let mut written = Vec::new();
        let operation = (|| -> Result<()> {
            // 倒序写入，让配置库最后落盘：中途失败时配置库仍与客户端现状一致。
            for i in (0..targets.len()).rev() {
                if before[i] == after[i] {
                    continue;
                }
                if read_target(&targets[i])? != before[i] {
                    bail!("{} 在保存期间发生变化", targets[i].label);
                }
                writer(&targets[i], after[i].as_deref())?;
                written.push(i);
            }
            Ok(())
        })();
        if let Err(error) = operation {
            let mut failures = Vec::new();
            for i in written.into_iter().rev() {
                let result = (|| -> Result<()> {
                    if read_target(&targets[i])? != after[i] {
                        bail!("{} 再次变化，未覆盖外部修改", targets[i].label)
                    }
                    write_target(&targets[i], before[i].as_deref())
                })();
                if let Err(e) = result {
                    failures.push(e.to_string());
                }
            }
            bail!(
                "保存失败：{error:#}。回滚错误：{}",
                if failures.is_empty() {
                    "无".into()
                } else {
                    failures.join("；")
                }
            );
        }
        Ok(())
    }
    pub fn window_state(&self) -> Option<WindowState> {
        let bytes = read(&self.roots.acs.join(WINDOW_STATE)).ok().flatten()?;
        serde_json::from_slice(&bytes).ok()
    }
    /// 记不住窗口尺寸只影响下次打开时的观感，失败只记日志，不打断退出或启动。
    pub fn set_window_state(&self, state: &WindowState) {
        let result = serde_json::to_vec_pretty(state)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| write(&self.roots.acs.join(WINDOW_STATE), Some(&bytes)));
        if let Err(e) = result {
            self.log("保存窗口尺寸", &format!("{e:#}"));
        }
    }
    pub fn log(&self, stage: &str, error: &str) {
        let dir = self.roots.acs.join("logs");
        if fs::create_dir_all(&dir).is_ok() {
            if let Ok(mut f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("acs.log"))
            {
                let _ = writeln!(f, "{:?} [{stage}] {error}", std::time::SystemTime::now());
            }
        }
    }
}

/// 桌面窗口的逻辑尺寸与位置。只描述本机窗口，不属于目标文件基线，
/// 因此读写它不会触发保存与启动的外部变化校验。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub width: f64,
    pub height: f64,
    pub x: f64,
    pub y: f64,
    pub maximized: bool,
}

pub fn revision(snapshot: &Snapshot) -> String {
    let mut hash = Sha256::new();
    for file in snapshot {
        match file {
            None => hash.update([0]),
            Some(b) => {
                hash.update([1]);
                hash.update((b.len() as u64).to_le_bytes());
                hash.update(b);
            }
        }
    }
    format!("{:x}", hash.finalize())
}
fn object(path: &Path) -> Result<Value> {
    let Some(bytes) = read(path)? else {
        return Ok(Value::Object(Default::default()));
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(Default::default()));
    }
    let value: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("{} 格式无效", path.display()))?;
    if !value.is_object() {
        bail!("{} 必须是 JSON 对象", path.display())
    }
    Ok(value)
}
pub fn read_target(target: &Target) -> Result<Option<Vec<u8>>> {
    match target.lens {
        Lens::Whole => read(&target.path),
        Lens::Field(key) => Ok(object(&target.path)?
            .get(key)
            .filter(|v| !v.is_null())
            .map(serde_json::to_vec)
            .transpose()?),
    }
}
pub fn write_target(target: &Target, bytes: Option<&[u8]>) -> Result<()> {
    match target.lens {
        Lens::Whole => write(&target.path, bytes),
        Lens::Field(key) => {
            if bytes.is_none() && read(&target.path)?.is_none() {
                return Ok(());
            }
            let mut value = object(&target.path)?;
            let map = value.as_object_mut().unwrap();
            match bytes {
                Some(bytes) => {
                    map.insert(key.into(), serde_json::from_slice(bytes)?);
                }
                None => {
                    map.remove(key);
                }
            }
            write(&target.path, Some(&serde_json::to_vec_pretty(&value)?))
        }
    }
}
pub fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("读取 {}", path.display())),
    }
}
pub fn write(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    if let Some(bytes) = bytes {
        let parent = path.parent().context("无父目录")?;
        fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        if let Ok(meta) = fs::metadata(path) {
            tmp.as_file().set_permissions(meta.permissions())?;
        }
        tmp.write_all(bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(path)
            .map_err(|e| e.error)
            .with_context(|| format!("替换 {}", path.display()))?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_write_failure_restores_existing_and_absent_files() {
        for existing in [false, true] {
            for fail_at in 0..TARGETS {
                let dir = tempfile::tempdir().unwrap();
                let store = Store::new(Roots::under(dir.path()));
                if existing {
                    for target in store.targets() {
                        let original = match target.lens {
                            Lens::Whole => b"example original".to_vec(),
                            Lens::Field(key) => {
                                format!("{{\"keep\":1,\"{key}\":{{\"a\":\"original\"}}}}").into()
                            }
                        };
                        write(&target.path, Some(&original)).unwrap();
                    }
                }
                let before = store.snapshot().unwrap();
                let after = store
                    .targets()
                    .iter()
                    .map(|target| match target.lens {
                        Lens::Whole => Some(b"example changed".to_vec()),
                        Lens::Field(_) => Some(br#"{"a":"changed"}"#.to_vec()),
                    })
                    .collect::<Snapshot>();
                let mut count = 0;
                let result = store.commit_with(&before, &after, |target, bytes| {
                    let fail = count == fail_at;
                    count += 1;
                    if fail {
                        bail!("example disk failure");
                    }
                    write_target(target, bytes)
                });
                assert!(result.is_err());
                assert_eq!(store.snapshot().unwrap(), before);
            }
        }
    }
    #[test]
    fn field_lens_keeps_unrelated_content_and_reports_absence() {
        let dir = tempfile::tempdir().unwrap();
        let target = Target {
            label: "example",
            path: dir.path().join("example.json"),
            lens: Lens::Field("oauthAccount"),
        };
        assert_eq!(read_target(&target).unwrap(), None);
        write(&target.path, Some(br#"{"projects":{"a":1}}"#)).unwrap();
        assert_eq!(read_target(&target).unwrap(), None);
        write_target(&target, Some(br#"{"emailAddress":"example@example.com"}"#)).unwrap();
        let stored: Value = serde_json::from_slice(&read(&target.path).unwrap().unwrap()).unwrap();
        assert_eq!(stored["projects"]["a"], 1);
        assert_eq!(
            stored["oauthAccount"]["emailAddress"],
            "example@example.com"
        );
        write_target(&target, None).unwrap();
        assert_eq!(read_target(&target).unwrap(), None);
        assert!(read(&target.path).unwrap().is_some());
    }
}
