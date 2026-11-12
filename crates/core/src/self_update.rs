//! CLI Release 校验与原位替换，不处理 GUI。
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path, time::Duration};
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub assets: Vec<Asset>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}
pub trait Downloader {
    fn get(&self, url: &str) -> Result<Vec<u8>>;
}
pub struct HttpDownloader(pub reqwest::blocking::Client);
impl HttpDownloader {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                .user_agent("acs-cli-update")
                .build()?,
        ))
    }
}
impl Downloader for HttpDownloader {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.0.get(url).send()?.error_for_status()?;
        Ok(response.bytes()?.to_vec())
    }
}
pub fn asset_name(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Ok("acs-cli-windows-x64.exe"),
        ("linux", "x86_64") => Ok("acs-cli-linux-x64-musl"),
        ("macos", "aarch64") => Ok("acs-cli-macos-arm64"),
        ("macos", "x86_64") => Ok("acs-cli-macos-x86_64"),
        _ => bail!("不支持的平台 {os}/{arch}"),
    }
}
pub fn checksum(text: &str, name: &str) -> Result<String> {
    let mut matches = text.lines().filter_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() == 2 && fields[1].trim_start_matches('*') == name {
            Some(fields[0])
        } else {
            None
        }
    });
    let sum = matches.next().context("校验文件缺少对应 CLI 产物")?;
    if matches.next().is_some() || sum.len() != 64 || !sum.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("SHA-256 校验条目无效或重复");
    }
    Ok(sum.to_ascii_lowercase())
}
fn u16_at(b: &[u8], n: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(n..n + 2).context("二进制头截断")?.try_into()?,
    ))
}
fn u32_at(b: &[u8], n: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(n..n + 4).context("二进制头截断")?.try_into()?,
    ))
}
pub fn validate_binary(b: &[u8], os: &str, arch: &str) -> Result<()> {
    asset_name(os, arch)?;
    match os {
        "windows" => {
            if !b.starts_with(b"MZ") {
                bail!("产物不是 Windows PE");
            }
            let offset = u32_at(b, 60)? as usize;
            if b.get(offset..offset + 4) != Some(b"PE\0\0") || u16_at(b, offset + 4)? != 0x8664 {
                bail!("PE 平台不匹配");
            }
        }
        "linux" => {
            if !b.starts_with(b"\x7fELF\x02\x01") || u16_at(b, 18)? != 62 {
                bail!("产物不是 Linux x64 ELF");
            }
            let offset =
                u64::from_le_bytes(b.get(32..40).context("ELF 头截断")?.try_into()?) as usize;
            let size = u16_at(b, 54)? as usize;
            let count = u16_at(b, 56)? as usize;
            if count > 0 && size < 56 {
                bail!("ELF 程序头无效");
            }
            for i in 0..count {
                let at = offset
                    .checked_add(i.checked_mul(size).context("ELF 偏移溢出")?)
                    .context("ELF 偏移溢出")?;
                if u32_at(b, at)? == 3 {
                    bail!("Linux CLI 带动态解释器，拒绝替换静态 musl CLI");
                }
            }
        }
        "macos" => {
            let cpu = if arch == "aarch64" {
                0x0100000c
            } else {
                0x01000007
            };
            if b.get(..4) != Some(&[0xcf, 0xfa, 0xed, 0xfe]) || u32_at(b, 4)? != cpu {
                bail!("Mach-O 平台不匹配");
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
fn url<'a>(release: &'a Release, name: &str) -> Result<&'a str> {
    let mut assets = release.assets.iter().filter(|a| a.name == name);
    let a = assets
        .next()
        .with_context(|| format!("Release 缺少 {name}"))?;
    if assets.next().is_some() {
        bail!("Release 含重复产物 {name}");
    }
    Ok(&a.browser_download_url)
}
pub fn update(
    downloader: &dyn Downloader,
    api: &str,
    target: &Path,
    os: &str,
    arch: &str,
) -> Result<String> {
    let name = asset_name(os, arch)?;
    let release: Release = serde_json::from_slice(&downloader.get(api)?)?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    if crate::environment::compare_versions(version, version).is_none()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        bail!("Release tag 不是有效版本");
    }
    let bytes = downloader.get(url(&release, name)?)?;
    let sums = downloader.get(url(&release, &format!("{name}.sha256"))?)?;
    let expected = checksum(std::str::from_utf8(&sums)?, name)?;
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        bail!("CLI SHA-256 不匹配，保留当前程序");
    }
    validate_binary(&bytes, os, arch)?;
    replace(target, &bytes, os == "windows")?;
    Ok(release.tag_name)
}
fn replace(target: &Path, bytes: &[u8], windows: bool) -> Result<()> {
    replace_with(target, bytes, windows, |a, b| {
        fs::rename(a, b).map_err(Into::into)
    })
}
fn replace_with(
    target: &Path,
    bytes: &[u8],
    windows: bool,
    mut rename: impl FnMut(&Path, &Path) -> Result<()>,
) -> Result<()> {
    let target = std::path::absolute(target)?;
    let mut temp = tempfile::NamedTempFile::new_in(target.parent().context("可执行文件无父目录")?)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    if windows {
        let backup = target.with_file_name(format!(
            "{}.old",
            target.file_name().unwrap().to_string_lossy()
        ));
        if backup.exists() {
            fs::remove_file(&backup).context("清理上次更新备份，请关闭仍运行的旧版本后重试")?;
        }
        let existed = target.exists();
        if existed {
            rename(&target, &backup)?;
        }
        if let Err(error) = rename(temp.path(), &target) {
            if existed {
                if let Err(restore) = fs::rename(&backup, &target) {
                    bail!(
                        "替换失败：{error:#}；恢复失败：{restore}；旧文件保留在 {}",
                        backup.display()
                    );
                }
            }
            return Err(error);
        }
    } else {
        rename(temp.path(), &target)?;
    }
    Ok(())
}
pub const RELEASE_API: &str = "https://api.github.com/repos/shellus/acs/releases/latest";
#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        map: std::collections::BTreeMap<String, Vec<u8>>,
    }
    impl Downloader for Fake {
        fn get(&self, u: &str) -> Result<Vec<u8>> {
            self.map
                .get(u)
                .cloned()
                .context("example interrupted download")
        }
    }
    fn pe() -> Vec<u8> {
        let mut b = vec![0; 128];
        b[..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&64u32.to_le_bytes());
        b[64..68].copy_from_slice(b"PE\0\0");
        b[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        b
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "subprocess-only running image probe"]
    fn running_image_probe() {
        use std::io::{self, Write};
        println!("example-update-ready");
        io::stdout().flush().unwrap();
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn running_windows_image_can_be_renamed_and_replaced() {
        use std::{
            io::{BufRead, BufReader},
            process::{Command, Stdio},
        };
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("acs.exe");
        fs::copy(std::env::current_exe().unwrap(), &target).unwrap();
        let mut child = Child(
            Command::new(&target)
                .args([
                    "--exact",
                    "self_update::tests::running_image_probe",
                    "--ignored",
                    "--nocapture",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut reader = BufReader::new(child.0.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line.contains("example-update-ready") {
                break;
            }
            line.clear();
        }
        let bytes = pe();
        replace(&target, &bytes, true).unwrap();
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert!(dir.path().join("acs.exe.old").exists());
        drop(child);
    }
    #[test]
    fn update_success_bad_hash_interrupted_and_wrong_platform_preserve_old() {
        for mode in 0..4 {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("acs.exe");
            fs::write(&path, b"old").unwrap();
            let name = asset_name("windows", "x86_64").unwrap();
            let bytes = if mode == 3 { vec![0; 128] } else { pe() };
            let sum = format!(
                "{}  {name}\n",
                if mode == 1 {
                    "0".repeat(64)
                } else {
                    format!("{:x}", Sha256::digest(&bytes))
                }
            );
            let release = serde_json::json!({"tag_name":"v1.2.3","assets":[{"name":name,"browser_download_url":"binary"},{"name":format!("{name}.sha256"),"browser_download_url":"checksum"}]});
            let mut map = std::collections::BTreeMap::from([
                ("api".into(), serde_json::to_vec(&release).unwrap()),
                ("binary".into(), bytes.clone()),
                ("checksum".into(), sum.into_bytes()),
            ]);
            if mode == 2 {
                map.remove("binary");
            }
            let result = update(&Fake { map }, "api", &path, "windows", "x86_64");
            assert_eq!(result.is_ok(), mode == 0);
            assert_eq!(
                fs::read(path).unwrap(),
                if mode == 0 { bytes } else { b"old".to_vec() }
            );
        }
    }
    #[test]
    fn failed_second_rename_restores_old() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("acs.exe");
        fs::write(&p, b"old").unwrap();
        let mut n = 0;
        assert!(replace_with(&p, b"new", true, |a, b| {
            n += 1;
            if n == 2 {
                bail!("example replacement failure");
            }
            fs::rename(a, b)?;
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(p).unwrap(), b"old");
    }
    #[test]
    fn platform_and_checksum_validation() {
        assert!(asset_name("linux", "aarch64").is_err());
        assert!(asset_name("windows", "x86").is_err());
        let n = asset_name("linux", "x86_64").unwrap();
        assert_eq!(
            checksum(&format!("{}  {n}\n", "a".repeat(64)), n).unwrap(),
            "a".repeat(64)
        );
        assert!(checksum(&format!("{}  other\n", "a".repeat(64)), n).is_err());
        assert!(checksum(&format!("{}  {n}\n", "z".repeat(64)), n).is_err());
        let mut b = vec![0; 128];
        b[..6].copy_from_slice(b"\x7fELF\x02\x01");
        b[18..20].copy_from_slice(&62u16.to_le_bytes());
        validate_binary(&b, "linux", "x86_64").unwrap();
        b[32..40].copy_from_slice(&64u64.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&3u32.to_le_bytes());
        assert!(validate_binary(&b, "linux", "x86_64").is_err());
    }
}
