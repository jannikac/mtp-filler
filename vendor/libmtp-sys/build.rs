const MIN_LIBMTP_VERSION: &str = "1.1.15";

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    println!("cargo:rerun-if-env-changed=LIBMTP_SYS_USE_PKG_CONFIG");
    println!("cargo:rerun-if-env-changed=LIBMTP_SYS_FORCE_VENDORED");

    if target_family() == Some("unix") && env::var_os("LIBMTP_SYS_USE_PKG_CONFIG").is_none() {
        build_vendored().unwrap_or_else(|err| panic!("failed to build vendored libmtp stack: {}", err));
        return;
    }

    probe_system();
}

fn target_family() -> Option<&'static str> {
    if env::var_os("CARGO_CFG_UNIX").is_some() {
        Some("unix")
    } else if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        Some("windows")
    } else {
        None
    }
}

fn probe_system() {
    if let Err(err) = pkg_config::Config::new()
        .atleast_version(MIN_LIBMTP_VERSION)
        .cargo_metadata(true)
        .probe("libmtp")
    {
        eprintln!("Couldn't find libmtp on your system!  (minimum version: {MIN_LIBMTP_VERSION})");
        eprintln!("This crate requires that it's installed and its pkg-config is working correctly!");
        panic!(
            "Couldn't find libmtp via `pkg-config`: {:?}\nPKG_CONFIG_SYSROOT_DIR={}",
            err,
            env::var("PKG_CONFIG_SYSROOT_DIR").unwrap_or_default(),
        );
    }
}

fn build_vendored() -> Result<(), String> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|err| err.to_string())?);
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|err| err.to_string())?);
    let build_root = out_dir.join("vendored-build");
    let prefix = out_dir.join("vendored-prefix");
    let stamp = prefix.join(".built-stamp");
    let common = CommonEnv::new(&prefix)?;
    let tarballs = [
        manifest_dir.join("vendor-src/libusb-1.0.29.tar.bz2"),
        manifest_dir.join("vendor-src/libmtp-1.1.23.tar.gz"),
    ];

    for tarball in &tarballs {
        println!("cargo:rerun-if-changed={}", tarball.display());
        if !tarball.exists() {
            return Err(format!("missing vendored source archive {}", tarball.display()));
        }
    }

    if !stamp.exists() || env::var_os("LIBMTP_SYS_FORCE_VENDORED").is_some() {
        if prefix.exists() {
            fs::remove_dir_all(&prefix).map_err(|err| format!("remove {}: {err}", prefix.display()))?;
        }
        if build_root.exists() {
            fs::remove_dir_all(&build_root)
                .map_err(|err| format!("remove {}: {err}", build_root.display()))?;
        }
        fs::create_dir_all(&build_root).map_err(|err| format!("mkdir {}: {err}", build_root.display()))?;
        fs::create_dir_all(&prefix).map_err(|err| format!("mkdir {}: {err}", prefix.display()))?;

        build_autotools(
            "libusb",
            &tarballs[0],
            &build_root.join("libusb"),
            &[
                format!("--prefix={}", prefix.display()),
                "--enable-static".to_string(),
                "--disable-shared".to_string(),
                "--disable-udev".to_string(),
            ],
            &common,
        )?;
        build_libmtp(
            "libmtp",
            &tarballs[1],
            &build_root.join("libmtp"),
            &[
                format!("--prefix={}", prefix.display()),
                "--enable-static".to_string(),
                "--disable-shared".to_string(),
                "--disable-doxygen".to_string(),
                "--disable-mtpz".to_string(),
            ],
            &common,
            &prefix,
        )?;
        fs::write(&stamp, b"ok").map_err(|err| format!("write {}: {err}", stamp.display()))?;
    }

    let pkg_config_path = format!(
        "{}:{}",
        prefix.join("lib/pkgconfig").display(),
        prefix.join("lib64/pkgconfig").display()
    );
    env::set_var("PKG_CONFIG_PATH", prepend_env("PKG_CONFIG_PATH", &pkg_config_path));

    pkg_config::Config::new()
        .atleast_version(MIN_LIBMTP_VERSION)
        .statik(true)
        .cargo_metadata(true)
        .probe("libmtp")
        .map_err(|err| format!("pkg-config probe for vendored libmtp failed: {err}"))?;

    Ok(())
}

struct CommonEnv {
    path: String,
    pkg_config_path: String,
    cppflags: String,
    cflags: String,
    cxxflags: String,
    ldflags: String,
}

impl CommonEnv {
    fn new(prefix: &Path) -> Result<Self, String> {
        let pkg_config_path = format!(
            "{}:{}",
            prefix.join("lib/pkgconfig").display(),
            prefix.join("lib64/pkgconfig").display()
        );
        let bin_dir = prefix.join("bin");
        let path = prepend_env("PATH", &bin_dir.to_string_lossy());
        let cppflags = prepend_space_env("CPPFLAGS", &format!("-I{}", prefix.join("include").display()));
        let ldflags = prepend_space_env(
            "LDFLAGS",
            &format!("-L{} -L{}", prefix.join("lib").display(), prefix.join("lib64").display()),
        );
        let cflags = env::var("CFLAGS").unwrap_or_else(|_| "-O2 -fPIC".to_string());
        let cxxflags = env::var("CXXFLAGS").unwrap_or_else(|_| "-O2 -fPIC".to_string());

        Ok(Self {
            path,
            pkg_config_path,
            cppflags,
            cflags,
            cxxflags,
            ldflags,
        })
    }
}

fn build_autotools(
    name: &str,
    archive: &Path,
    src_dir: &Path,
    configure_args: &[String],
    common: &CommonEnv,
) -> Result<(), String> {
    extract_archive(archive, src_dir)?;

    run(
        apply_common_env(
            Command::new("./configure")
                .current_dir(src_dir)
                .args(configure_args),
            common,
        ),
        &format!("configure {name}"),
    )?;

    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "1".to_string());
    run(
        apply_common_env(
            Command::new("make")
                .current_dir(src_dir)
                .arg(format!("-j{jobs}")),
            common,
        ),
        &format!("make {name}"),
    )?;
    run(
        apply_common_env(Command::new("make").current_dir(src_dir).arg("install"), common),
        &format!("make install {name}"),
    )?;

    Ok(())
}

fn build_libmtp(
    name: &str,
    archive: &Path,
    src_dir: &Path,
    configure_args: &[String],
    common: &CommonEnv,
    prefix: &Path,
) -> Result<(), String> {
    extract_archive(archive, src_dir)?;

    run(
        apply_common_env(
            Command::new("./configure")
                .current_dir(src_dir)
                .args(configure_args),
            common,
        ),
        &format!("configure {name}"),
    )?;

    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "1".to_string());
    run(
        apply_common_env(
            Command::new("make")
                .current_dir(src_dir.join("src"))
                .arg(format!("-j{jobs}")),
            common,
        ),
        &format!("make {name} library"),
    )?;
    run(
        apply_common_env(Command::new("make").current_dir(src_dir.join("src")).arg("install"), common),
        &format!("make install {name} library"),
    )?;

    let pkgconfig_dir = prefix.join("lib/pkgconfig");
    fs::create_dir_all(&pkgconfig_dir).map_err(|err| format!("mkdir {}: {err}", pkgconfig_dir.display()))?;
    fs::copy(src_dir.join("libmtp.pc"), pkgconfig_dir.join("libmtp.pc"))
        .map_err(|err| format!("copy libmtp.pc: {err}"))?;

    Ok(())
}

fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|err| format!("remove {}: {err}", dest.display()))?;
    }
    fs::create_dir_all(dest).map_err(|err| format!("mkdir {}: {err}", dest.display()))?;

    run(
        Command::new("tar")
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(dest)
            .arg("--strip-components=1"),
        &format!("extract {}", archive.display()),
    )
}

fn run(command: &mut Command, label: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|err| format!("{label} failed to start: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} exited with status {status}"))
    }
}

fn prepend_env(name: &str, prefix: &str) -> String {
    match env::var_os(name) {
        Some(existing) if !existing.is_empty() => {
            let mut value = OsString::from(prefix);
            value.push(":");
            value.push(existing);
            value.to_string_lossy().into_owned()
        }
        _ => prefix.to_string(),
    }
}

fn prepend_space_env(name: &str, prefix: &str) -> String {
    match env::var(name) {
        Ok(existing) if !existing.trim().is_empty() => format!("{prefix} {existing}"),
        _ => prefix.to_string(),
    }
}

fn apply_common_env<'a>(command: &'a mut Command, common: &CommonEnv) -> &'a mut Command {
    command
        .env("PATH", &common.path)
        .env("PKG_CONFIG_PATH", &common.pkg_config_path)
        .env("CPPFLAGS", &common.cppflags)
        .env("CFLAGS", &common.cflags)
        .env("CXXFLAGS", &common.cxxflags)
        .env("LDFLAGS", &common.ldflags);

    command
}
