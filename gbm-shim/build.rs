use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let linker_script = PathBuf::from(manifest_dir).join("linker.lds");
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!(
        "cargo:rustc-cdylib-link-arg=-Wl,--version-script={}",
        linker_script.display()
    );
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libgbm.so.1");
}
