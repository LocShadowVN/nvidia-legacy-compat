fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libgbm.so.1");
}
