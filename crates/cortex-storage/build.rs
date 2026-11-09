fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
