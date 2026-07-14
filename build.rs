use std::{env, path::PathBuf};

fn main() -> Result<(), env::VarError> {
    println!("cargo:rerun-if-changed=Info.plist");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let plist_path = PathBuf::from(manifest_dir).join("Info.plist");
    println!(
        "cargo:rustc-link-arg-bin=taptext=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        plist_path.display()
    );
    Ok(())
}
