//! 显式一次性迁移；普通读取路径不识别旧命名。
use super::*;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs};
#[derive(Clone, Copy)]
pub enum LegacyPlatform {
    Windows,
    Unix,
}
#[derive(Clone, Debug)]
pub struct LegacyPaths {
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}
impl LegacyPaths {
    pub fn resolve(
        home: &Path,
        platform: LegacyPlatform,
        env: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let expand = |s: String| {
            let s = s.trim();
            if s == "~" {
                home.into()
            } else if s.starts_with("~/") || s.starts_with("~\\") {
                home.join(&s[2..])
            } else {
                PathBuf::from(s)
            }
        };
        let get = |k: &str| env(k).filter(|s| !s.trim().is_empty()).map(expand);
        let state = get("AGS_STATE_DIR")
            .or_else(|| get("XDG_STATE_HOME"))
            .unwrap_or_else(|| match platform {
                LegacyPlatform::Windows => {
                    get("APPDATA").unwrap_or_else(|| home.join("AppData/Roaming"))
                }
                LegacyPlatform::Unix => home.join(".local/state"),
            });
        let cache = match platform {
            LegacyPlatform::Windows => {
                get("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"))
            }
            LegacyPlatform::Unix => get("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")),
        };
        Self {
            config: home.join(".ags/config.yaml"),
            state: state.join("ags/environment.json"),
            cache: cache.join("ags"),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    config: PathBuf,
    state: PathBuf,
    source_hash: String,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn import_ags(paths: &Paths, legacy: &LegacyPaths) -> Result<Vec<String>> {
    import_with(paths, legacy, transaction::apply)
}
fn import_with(
    paths: &Paths,
    legacy: &LegacyPaths,
    writer: impl FnOnce(&[Change]) -> Result<()>,
) -> Result<Vec<String>> {
    let _lock = paths.store().lock()?;
    let old_config = fs::read(&legacy.config).context("读取 AGS 本机配置")?;
    let old_state = fs::read(&legacy.state).context("读取 AGS 应用状态")?;
    let source_hash = digest(&[old_config.as_slice(), old_state.as_slice()].concat());
    let receipt_path = paths.roots.acs.join("state/environment-import.json");
    if let Some(bytes) = storage::read(&receipt_path)? {
        let receipt: Receipt = serde_json::from_slice(&bytes)?;
        if receipt.version == 1
            && receipt.config == legacy.config
            && receipt.state == legacy.state
            && receipt.source_hash == source_hash
            && paths.config().exists()
        {
            return Ok(vec!["该 AGS 状态已导入，未改写任何文件".into()]);
        }
        bail!("迁移记录与旧状态不一致，保留现有 acs 环境，不覆盖");
    }
    if paths.config().exists() || paths.state().exists() {
        bail!("已存在 acs 环境配置或状态，拒绝覆盖");
    }
    let mut yaml: serde_yaml::Value = decode_yaml(&old_config)?;
    let mut messages = vec![];
    if let Some(agents) = yaml
        .get_mut("environment")
        .and_then(|e| e.get_mut("agents"))
        .and_then(serde_yaml::Value::as_sequence_mut)
    {
        let count = agents.len();
        agents.retain(|v| v.as_str() != Some("happy"));
        if agents.len() != count {
            messages.push("已丢弃旧配置中不再支持的 happy 项".into());
        }
    }
    let mut config: LocalConfig = serde_yaml::from_value(yaml)?;
    config.validate()?;
    config.environment.agents.sort_by_key(|a| a.label());
    let mut json: serde_json::Value = serde_json::from_slice(&old_state)?;
    let agents = json["agents"]
        .as_object_mut()
        .context("AGS 状态缺少 agents 对象")?;
    agents.remove("happy");
    for (key, value) in agents.iter_mut() {
        let agent: Agent = key.parse()?;
        let root = match agent {
            Agent::Codex => paths.roots.codex.join("skills"),
            Agent::Claude => paths.roots.claude.join("skills"),
        };
        if value
            .get("skills_root")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
        {
            value["skills_root"] = serde_json::to_value(root)?;
        }
    }
    let state: State = serde_json::from_value(json)?;
    state.validate()?;
    let mut roots = BTreeSet::new();
    for agent in Agent::ALL {
        roots.insert(paths.destinations(agent).1);
    }
    roots.insert(paths.roots.codex.join("skills"));
    for s in state.agents.values() {
        roots.insert(s.skills_root.clone());
    }
    let mut changes = vec![];
    for root in roots {
        let old = root.join(".ags-managed.json");
        if let Some(bytes) = storage::read(&old)? {
            let marker: ManagedMarker = serde_json::from_slice(&bytes)?;
            let encoded = marker.encode()?;
            let new = root.join(MANAGED_MARKER);
            if let Some(current) = storage::read(&new)? {
                let current: ManagedMarker = serde_json::from_slice(&current)?;
                if current.skills != marker.skills || current.version != marker.version {
                    bail!("目标 Skills 根已有不同的 acs 标记：{}", root.display());
                }
            }
            changes.push(Change {
                path: new,
                content: Some(encoded),
            });
            changes.push(Change {
                path: old,
                content: None,
            });
        }
    }
    let config_path = paths.roots.codex.join("config.toml");
    if let Some(bytes) = storage::read(&config_path)? {
        let text = std::str::from_utf8(&bytes)?;
        let (_, acs) = split_block(text, DISABLED_BEGIN, DISABLED_END)?;
        let (base, old) = split_block(
            text,
            "# BEGIN AGS MANAGED CODEX DISABLED SKILLS",
            "# END AGS MANAGED CODEX DISABLED SKILLS",
        )?;
        if old.is_some() && acs.is_some() {
            bail!("Codex 同时存在两种受管禁用块，拒绝猜测覆盖");
        }
        if let Some(block) = old {
            let converted = block
                .replace("# BEGIN AGS MANAGED CODEX DISABLED SKILLS", DISABLED_BEGIN)
                .replace("# END AGS MANAGED CODEX DISABLED SKILLS", DISABLED_END);
            let start = text.find(block).unwrap();
            let mut converted_text = base;
            converted_text.insert_str(start, &converted);
            changes.push(Change {
                path: config_path,
                content: Some(converted_text.into_bytes()),
            });
        }
    }
    changes.push(Change {
        path: paths.config(),
        content: Some(serde_yaml::to_string(&config)?.into_bytes()),
    });
    changes.push(Change {
        path: paths.state(),
        content: Some(state.encode()?),
    });
    changes.push(Change {
        path: receipt_path,
        content: Some(serde_json::to_vec_pretty(&Receipt {
            version: 1,
            config: legacy.config.clone(),
            state: legacy.state.clone(),
            source_hash,
        })?),
    });
    writer(&changes)?;
    messages.push("AGS 环境已导入；旧配置、状态目录和二进制均保留".into());
    Ok(messages)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_migration_write_failure_rolls_back_converted_markers_block_and_config() {
        for fail in 0..6 {
            let d = tempfile::tempdir().unwrap();
            let paths = Paths::under(d.path());
            let old = LegacyPaths::resolve(d.path(), LegacyPlatform::Unix, |_| None);
            storage::write(&old.config,Some(b"version: 1\nenvironment: {source: example, branch: main, profile: default, agents: [codex]}\n")).unwrap();
            storage::write(&old.state,Some(br#"{"version":1,"source":"example","branch":"main","commit":"example","profile":"default","agents":{}}"#)).unwrap();
            let marker = paths.codex_skills.join(".ags-managed.json");
            storage::write(&marker, Some(br#"{"version":1,"skills":{}}"#)).unwrap();
            let config = paths.roots.codex.join("config.toml");
            let block=b"# BEGIN AGS MANAGED CODEX DISABLED SKILLS\n[[skills.config]]\nname = 'example'\nenabled = false\n# END AGS MANAGED CODEX DISABLED SKILLS\n";
            storage::write(&config, Some(block)).unwrap();
            assert!(import_with(&paths, &old, |c| transaction::apply_failing_at(c, fail)).is_err());
            assert_eq!(fs::read(&config).unwrap(), block);
            assert!(marker.exists());
            assert!(!paths.codex_skills.join(MANAGED_MARKER).exists());
            assert!(!paths.config().exists());
            assert!(!paths.state().exists());
            assert!(!paths
                .roots
                .acs
                .join("state/environment-import.json")
                .exists());
        }
    }
}
