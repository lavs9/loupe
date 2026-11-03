fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        println!("cargo:rerun-if-changed=src/photokit_bridge.m");
        println!("cargo:rerun-if-changed=scripts/Info.plist");

        cc::Build::new()
            .file("src/photokit_bridge.m")
            .compile("photokit_bridge");

        println!("cargo:rustc-link-lib=framework=Photos");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=AppKit");

        let info_plist = std::path::Path::new("scripts/Info.plist");
        if info_plist.exists() {
            if let Ok(abs) = info_plist.canonicalize() {
                println!("cargo:rustc-link-arg=-sectcreate");
                println!("cargo:rustc-link-arg=__TEXT");
                println!("cargo:rustc-link-arg=__info_plist");
                println!("cargo:rustc-link-arg={}", abs.display());
            }
        }
    }
}
