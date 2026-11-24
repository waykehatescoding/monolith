use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn payload(var: &str, name: &str, out: &Path) {
    println!("cargo:rerun-if-env-changed={var}");
    let target = out.join(name);
    match env::var(var) {
        Ok(path) if !path.is_empty() => {
            println!("cargo:rerun-if-changed={path}");
            if fs::copy(&path, &target).is_err() {
                let _ = fs::write(&target, b"");
            }
        }
        _ => {
            let _ = fs::write(&target, b"");
        }
    }
}

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    payload("MONOLITH_PAYLOAD_EXE", "payload_exe.bin", &out);
    payload("MONOLITH_PAYLOAD_PLUGIN", "payload_plugin.bin", &out);

    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let icon = Path::new(&manifest).join("../../assets/brand/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=setup.rc");
    println!("cargo:rerun-if-changed=setup.manifest");
    if icon.exists() {
        if let Err(error) =
            embed_resource::compile("setup.rc", embed_resource::NONE).manifest_required()
        {
            panic!("failed to embed setup resources: {error}");
        }
    }
}
