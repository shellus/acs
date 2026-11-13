//! 安装脚本只使用临时离线发布目录；不写真实安装位置或用户 PATH。
//! 安装脚本只支持 Linux 与 Windows；macOS 自带 bash 3.2 且缺 sha256sum，不冒充 Linux 运行。
#![cfg(not(target_os = "macos"))]
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn binary(windows: bool) -> Vec<u8> {
    let mut b = vec![0; 128];
    if windows {
        b[..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&64u32.to_le_bytes());
        b[64..68].copy_from_slice(b"PE\0\0");
        b[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
    } else {
        b[..6].copy_from_slice(b"\x7fELF\x02\x01");
        b[18..20].copy_from_slice(&62u16.to_le_bytes());
    }
    b
}
fn setup(root: &Path, windows: bool, mode: usize) -> (PathBuf, PathBuf, Vec<u8>) {
    let release = root.join("release");
    let target = root.join("target");
    fs::create_dir_all(&release).unwrap();
    fs::create_dir_all(&target).unwrap();
    let file = if windows { "acs.exe" } else { "acs" };
    fs::write(target.join(file), b"example old").unwrap();
    let name = if windows {
        "acs-cli-windows-x64.exe"
    } else {
        "acs-cli-linux-x64-musl"
    };
    let bytes = binary(if mode == 3 { !windows } else { windows });
    if mode != 2 {
        fs::write(release.join(name), &bytes).unwrap();
    }
    let sum = if mode == 1 {
        "0".repeat(64)
    } else {
        format!("{:x}", Sha256::digest(&bytes))
    };
    fs::write(
        release.join(format!("{name}.sha256")),
        format!("{sum}  {name}\n"),
    )
    .unwrap();
    (release, target, bytes)
}
#[cfg(windows)]
#[test]
fn powershell_installer_offline_success_hash_interruption_and_platform() {
    for mode in 0..4 {
        let d = tempfile::tempdir().unwrap();
        let (release, target, bytes) = setup(d.path(), true, mode);
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install.ps1");
        // 与 README 的 irm | iex 用法一致：以文本经 Invoke-Expression 执行，选项走环境变量。
        let out = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "Get-Content -Raw -LiteralPath $env:ACS_EXAMPLE_SCRIPT | Invoke-Expression",
            ])
            .env("ACS_EXAMPLE_SCRIPT", script)
            .env("ACS_RELEASE_DIR", &release)
            .env("ACS_INSTALL_DIR", &target)
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            mode == 0,
            "mode={mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            fs::read(target.join("acs.exe")).unwrap(),
            if mode == 0 {
                bytes
            } else {
                b"example old".to_vec()
            }
        );
    }
}
#[test]
fn bash_installer_offline_success_hash_interruption_and_platform() {
    #[cfg(windows)]
    let bash = PathBuf::from(std::env::var_os("ProgramFiles").unwrap()).join("Git/bin/bash.exe");
    #[cfg(not(windows))]
    let bash = PathBuf::from("bash");
    if Command::new(&bash).arg("--version").output().is_err() {
        eprintln!("bash unavailable; installer shell probe skipped");
        return;
    }
    for mode in 0..4 {
        let d = tempfile::tempdir().unwrap();
        let (release, target, bytes) = setup(d.path(), false, mode);
        let bin = d.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let uname = bin.join("uname");
        fs::write(
            &uname,
            "#!/usr/bin/env bash\nif [[ $1 == -s ]]; then echo Linux; else echo x86_64; fi\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&uname, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let shell_path = |p: &Path| {
            let s = p.to_string_lossy().replace('\\', "/");
            if cfg!(windows) && s.as_bytes().get(1) == Some(&b':') {
                format!("/{}{}", s[..1].to_ascii_lowercase(), &s[2..])
            } else {
                s
            }
        };
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install.sh");
        let out = Command::new(&bash)
            .args([
                "-c",
                "export PATH=\"$ACS_EXAMPLE_SHIM:$PATH\"; source \"$ACS_EXAMPLE_SCRIPT\"",
            ])
            .env("ACS_EXAMPLE_SHIM", shell_path(&bin))
            .env("ACS_EXAMPLE_SCRIPT", shell_path(&script))
            .env("ACS_RELEASE_DIR", shell_path(&release))
            .env("ACS_INSTALL_DIR", shell_path(&target))
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            mode == 0,
            "mode={mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            fs::read(target.join("acs")).unwrap(),
            if mode == 0 {
                bytes
            } else {
                b"example old".to_vec()
            }
        );
    }
}
