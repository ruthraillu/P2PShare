use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    record_source_revision();
    println!("cargo:rerun-if-changed=../../assets/app-icon.ico");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest
        .join("../../assets/app-icon.ico")
        .canonicalize()
        .unwrap();
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let script = out.join("app-icon.rc");
    let resource = out.join("app-icon.res");
    let filename = icon
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    fs::write(&script, format!("1 ICON \"{filename}\"\n")).unwrap();
    let rc = env::var_os("RC").map(PathBuf::from).unwrap_or_else(|| {
        let kits =
            PathBuf::from(env::var_os("ProgramFiles(x86)").unwrap()).join("Windows Kits/10/bin");
        let mut candidates: Vec<_> = fs::read_dir(&kits)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("x64/rc.exe"))
            .filter(|p| p.is_file())
            .collect();
        candidates.sort();
        candidates
            .pop()
            .expect("Windows SDK rc.exe is required to embed the application icon")
    });
    let status = Command::new(rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(&script)
        .status()
        .expect("Could not run the Windows resource compiler");
    assert!(
        status.success(),
        "Application icon resource compilation failed"
    );
    println!("cargo:rustc-link-arg-bin=P2PShare={}", resource.display());
}

// Read Git metadata without requiring Git to be installed when building a source ZIP.
fn record_source_revision() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let git = root.join(".git");
    let head = git.join("HEAD");
    let mut revision = String::new();
    if let Ok(text) = fs::read_to_string(&head) {
        println!("cargo:rerun-if-changed={}", head.display());
        let value = text.trim();
        if let Some(reference) = value.strip_prefix("ref: ").filter(|r| {
            r.starts_with("refs/")
                && std::path::Path::new(r)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
        }) {
            let file = git.join(reference);
            if file.is_file() {
                println!("cargo:rerun-if-changed={}", file.display());
                revision = fs::read_to_string(file).unwrap_or_default();
            }
            let packed = git.join("packed-refs");
            if packed.is_file() {
                println!("cargo:rerun-if-changed={}", packed.display());
                if revision.is_empty() {
                    if let Ok(text) = fs::read_to_string(packed) {
                        revision = text
                            .lines()
                            .filter_map(|line| line.split_once(' '))
                            .find(|(_, name)| *name == reference)
                            .map(|(hash, _)| hash.to_owned())
                            .unwrap_or_default();
                    }
                }
            }
        } else {
            revision = value.to_owned();
        }
    }
    let value = revision.trim();
    let label = if matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        &value[..12]
    } else {
        "source-archive"
    };
    println!("cargo:rustc-env=P2PSHARE_BUILD_COMMIT={label}");
}
