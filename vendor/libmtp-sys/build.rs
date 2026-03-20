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
        build_autotools(
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
        )?;
        fs::write(&stamp, b"ok").map_err(|err| format!("write {}: {err}", stamp.display()))?;
    }

    let pkg_config_path = format!(
        "{}:{}",
        prefix.join("lib/pkgconfig").display(),
        prefix.join("lib64/pkgconfig").display()
    );
    env::set_var("PKG_CONFIG_PATH", prepend_env("PKG_CONFIG_PATH", &pkg_config_path));
    if common.is_cross_target {
        env::set_var("PKG_CONFIG_ALLOW_CROSS", "1");
    }

    pkg_config::Config::new()
        .atleast_version(MIN_LIBMTP_VERSION)
        .statik(true)
        .cargo_metadata(true)
        .probe("libmtp")
        .map_err(|err| format!("pkg-config probe for vendored libmtp failed: {err}"))?;

    Ok(())
}

struct CommonEnv {
    is_cross_target: bool,
    path: String,
    pkg_config_path: String,
    cppflags: String,
    cflags: String,
    cxxflags: String,
    ldflags: String,
    cc: Option<String>,
    cxx: Option<String>,
    ar: Option<String>,
    ranlib: Option<String>,
    strip: Option<String>,
    host_arg: Option<String>,
    build_arg: Option<String>,
}

impl CommonEnv {
    fn new(prefix: &Path) -> Result<Self, String> {
        let host = env::var("HOST").map_err(|err| err.to_string())?;
        let target = env::var("TARGET").map_err(|err| err.to_string())?;
        let pkg_config_path = format!(
            "{}:{}",
            prefix.join("lib/pkgconfig").display(),
            prefix.join("lib64/pkgconfig").display()
        );
        let bin_dir = prefix.join("bin");
        let path = prepend_env("PATH", &bin_dir.to_string_lossy());
        let mut cppflags = format!("-I{}", prefix.join("include").display());
        if target.ends_with("-linux-musl") {
            cppflags.push_str(" -idirafter /usr/include");
        }
        let cppflags = prepend_space_env("CPPFLAGS", &cppflags);
        let ldflags = prepend_space_env(
            "LDFLAGS",
            &format!("-L{} -L{}", prefix.join("lib").display(), prefix.join("lib64").display()),
        );
        let cflags = env::var("CFLAGS").unwrap_or_else(|_| "-O2 -fPIC".to_string());
        let cxxflags = env::var("CXXFLAGS").unwrap_or_else(|_| "-O2 -fPIC".to_string());
        let is_cross_target = host != target;
        let target_env_name = target.replace('-', "_");
        let target_prefix = musl_tool_prefix(&target);
        let cc = env::var(format!("CC_{target_env_name}"))
            .ok()
            .or_else(|| target_prefix.as_ref().map(|prefix| format!("{prefix}gcc")))
            .filter(|tool| command_exists(tool));
        let cxx = env::var(format!("CXX_{target_env_name}"))
            .ok()
            .or_else(|| target_prefix.as_ref().map(|prefix| format!("{prefix}g++")))
            .filter(|tool| command_exists(tool));
        let ar = env::var(format!("AR_{target_env_name}"))
            .ok()
            .or_else(|| target_prefix.as_ref().map(|prefix| format!("{prefix}ar")))
            .filter(|tool| command_exists(tool));
        let ranlib = env::var(format!("RANLIB_{target_env_name}"))
            .ok()
            .or_else(|| target_prefix.as_ref().map(|prefix| format!("{prefix}ranlib")))
            .filter(|tool| command_exists(tool));
        let strip = env::var(format!("STRIP_{target_env_name}"))
            .ok()
            .or_else(|| target_prefix.as_ref().map(|prefix| format!("{prefix}strip")))
            .filter(|tool| command_exists(tool));
        let host_arg = is_cross_target.then(|| autotools_host(&target));
        let build_arg = is_cross_target.then(|| host.clone());

        Ok(Self {
            is_cross_target,
            path,
            pkg_config_path,
            cppflags,
            cflags,
            cxxflags,
            ldflags,
            cc,
            cxx,
            ar,
            ranlib,
            strip,
            host_arg,
            build_arg,
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
    let mut configure_args = configure_args.to_vec();
    if let Some(build_arg) = &common.build_arg {
        configure_args.push(format!("--build={build_arg}"));
    }
    if let Some(host_arg) = &common.host_arg {
        configure_args.push(format!("--host={host_arg}"));
    }

    run(
        apply_common_env(
            Command::new("./configure")
                .current_dir(src_dir)
                .args(&configure_args),
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

    if let Some(cc) = &common.cc {
        command.env("CC", cc);
    }
    if let Some(cxx) = &common.cxx {
        command.env("CXX", cxx);
    }
    if let Some(ar) = &common.ar {
        command.env("AR", ar);
    }
    if let Some(ranlib) = &common.ranlib {
        command.env("RANLIB", ranlib);
    }
    if let Some(strip) = &common.strip {
        command.env("STRIP", strip);
    }
    if common.is_cross_target {
        command.env("PKG_CONFIG_ALLOW_CROSS", "1");
    }

    command
}

fn autotools_host(target: &str) -> String {
    target.replacen("-unknown-", "-", 1)
}

fn musl_tool_prefix(target: &str) -> Option<String> {
    target
        .ends_with("-linux-musl")
        .then(|| format!("{}-", autotools_host(target)))
}

fn command_exists(program: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|path| path.join(program).exists()))
        .unwrap_or(false)
}
